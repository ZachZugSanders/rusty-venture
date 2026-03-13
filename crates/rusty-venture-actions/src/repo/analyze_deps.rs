use std::sync::Arc;

use async_trait::async_trait;
use bollard::Docker;
use rusty_venture_core::{action::Action, context::ExecutionContext, CoreError};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::container::{exec_in_container, ExecCommand, CTX_CONTAINER_ID, CTX_DOCKER_CLIENT};
use super::clone::CTX_REPO_LOCAL_PATH;
use super::detect_language::{CTX_DETECTED_LANGUAGES, DetectedLanguages, Language};

pub const CTX_DEPENDENCY_REPORT: &str = "repo.dependency_report";

/// A single dependency entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dependency {
    pub name: String,
    pub version: Option<String>,
    pub kind: DependencyKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum DependencyKind {
    /// Regular/runtime dependency.
    Runtime,
    /// Development-only dependency.
    Dev,
    /// Build-time dependency.
    Build,
}

/// The result of dependency analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyReport {
    pub language: Language,
    pub dependencies: Vec<Dependency>,
    pub lock_file_present: bool,
    pub manifest_file: String,
    /// Raw output for LLM analysis.
    pub raw_output: String,
}

/// Analyzes project dependencies by dispatching to language-specific parsers.
#[derive(Default)]
pub struct AnalyzeDepsAction;

#[async_trait]
impl Action for AnalyzeDepsAction {
    type Input = ();
    type Output = DependencyReport;

    fn name(&self) -> &str {
        "analyze-deps"
    }

    async fn execute(&self, ctx: &ExecutionContext, _input: ()) -> Result<DependencyReport, CoreError> {
        let container_id: String = ctx.require::<String>(CTX_CONTAINER_ID).await?;
        let docker: Arc<Docker> = ctx.require::<Arc<Docker>>(CTX_DOCKER_CLIENT).await?;
        let repo_path: String = ctx.require::<String>(CTX_REPO_LOCAL_PATH).await?;
        let detected: DetectedLanguages = ctx.require::<DetectedLanguages>(CTX_DETECTED_LANGUAGES).await?;

        info!(language = %detected.primary, "Analyzing dependencies");

        let report = match &detected.primary {
            Language::Rust => analyze_rust(&docker, &container_id, &repo_path).await,
            Language::Node => analyze_node(&docker, &container_id, &repo_path).await,
            Language::Python => analyze_python(&docker, &container_id, &repo_path).await,
            Language::Go => analyze_go(&docker, &container_id, &repo_path).await,
            Language::Java => analyze_java(&docker, &container_id, &repo_path).await,
            Language::Ruby => analyze_ruby(&docker, &container_id, &repo_path).await,
            Language::PHP => analyze_php(&docker, &container_id, &repo_path).await,
            _ => Ok(DependencyReport {
                language: detected.primary.clone(),
                dependencies: vec![],
                lock_file_present: false,
                manifest_file: String::new(),
                raw_output: "No dependency parser available for this language.".to_string(),
            }),
        }?;

        ctx.insert(CTX_DEPENDENCY_REPORT, report.clone()).await;
        Ok(report)
    }
}

async fn analyze_rust(docker: &Docker, container_id: &str, repo_path: &str) -> Result<DependencyReport, CoreError> {
    // Check for Cargo.lock
    let lock_check = exec_in_container(
        docker,
        container_id,
        ExecCommand::new(["test", "-f", &format!("{repo_path}/Cargo.lock")]),
    ).await?;

    // Read Cargo.toml for a quick dep list
    let toml_read = exec_in_container(
        docker,
        container_id,
        ExecCommand::new(["cat", &format!("{repo_path}/Cargo.toml")]),
    ).await?;

    Ok(DependencyReport {
        language: Language::Rust,
        dependencies: parse_cargo_toml_deps(&toml_read.stdout),
        lock_file_present: lock_check.is_success(),
        manifest_file: "Cargo.toml".to_string(),
        raw_output: toml_read.stdout,
    })
}

fn parse_cargo_toml_deps(toml: &str) -> Vec<Dependency> {
    let mut deps = vec![];
    let mut in_deps_section = false;

    for line in toml.lines() {
        let trimmed = line.trim();
        if trimmed == "[dependencies]" || trimmed == "[dev-dependencies]" || trimmed == "[build-dependencies]" {
            in_deps_section = true;
            continue;
        }
        if trimmed.starts_with('[') {
            in_deps_section = false;
            continue;
        }
        if in_deps_section && !trimmed.is_empty() && !trimmed.starts_with('#') {
            if let Some(eq_pos) = trimmed.find('=') {
                let name = trimmed[..eq_pos].trim().to_string();
                let version_part = trimmed[eq_pos + 1..].trim();
                let version = if version_part.starts_with('"') {
                    Some(version_part.trim_matches('"').to_string())
                } else {
                    None
                };
                deps.push(Dependency {
                    name,
                    version,
                    kind: DependencyKind::Runtime,
                });
            }
        }
    }
    deps
}

async fn analyze_node(docker: &Docker, container_id: &str, repo_path: &str) -> Result<DependencyReport, CoreError> {
    let lock_check = exec_in_container(
        docker,
        container_id,
        ExecCommand::new(["sh", "-c", &format!(
            "test -f {repo_path}/package-lock.json || test -f {repo_path}/yarn.lock || test -f {repo_path}/pnpm-lock.yaml"
        )]),
    ).await?;

    let pkg_read = exec_in_container(
        docker,
        container_id,
        ExecCommand::new(["cat", &format!("{repo_path}/package.json")]),
    ).await?;

    let deps = parse_package_json_deps(&pkg_read.stdout);

    Ok(DependencyReport {
        language: Language::Node,
        dependencies: deps,
        lock_file_present: lock_check.is_success(),
        manifest_file: "package.json".to_string(),
        raw_output: pkg_read.stdout,
    })
}

fn parse_package_json_deps(json: &str) -> Vec<Dependency> {
    let mut deps = vec![];
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(json) {
        for (section, kind) in &[
            ("dependencies", DependencyKind::Runtime),
            ("devDependencies", DependencyKind::Dev),
        ] {
            if let Some(obj) = val[section].as_object() {
                for (name, ver) in obj {
                    deps.push(Dependency {
                        name: name.clone(),
                        version: ver.as_str().map(String::from),
                        kind: kind.clone(),
                    });
                }
            }
        }
    }
    deps
}

async fn analyze_python(docker: &Docker, container_id: &str, repo_path: &str) -> Result<DependencyReport, CoreError> {
    // Try pyproject.toml first, then requirements.txt
    let pyproject = exec_in_container(
        docker,
        container_id,
        ExecCommand::new(["cat", &format!("{repo_path}/pyproject.toml")]),
    ).await?;

    let (manifest_file, raw_output) = if pyproject.is_success() {
        ("pyproject.toml".to_string(), pyproject.stdout)
    } else {
        let req = exec_in_container(
            docker,
            container_id,
            ExecCommand::new(["cat", &format!("{repo_path}/requirements.txt")]),
        ).await?;
        ("requirements.txt".to_string(), req.stdout)
    };

    let lock_check = exec_in_container(
        docker,
        container_id,
        ExecCommand::new(["test", "-f", &format!("{repo_path}/poetry.lock")]),
    ).await?;

    let deps = raw_output.lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(|l| {
            let (name, version) = if let Some(pos) = l.find("==") {
                (l[..pos].trim().to_string(), Some(l[pos + 2..].trim().to_string()))
            } else if let Some(pos) = l.find(">=") {
                (l[..pos].trim().to_string(), Some(format!(">={}", &l[pos + 2..])))
            } else {
                (l.trim().to_string(), None)
            };
            Dependency { name, version, kind: DependencyKind::Runtime }
        })
        .collect();

    Ok(DependencyReport {
        language: Language::Python,
        dependencies: deps,
        lock_file_present: lock_check.is_success(),
        manifest_file,
        raw_output,
    })
}

async fn analyze_go(docker: &Docker, container_id: &str, repo_path: &str) -> Result<DependencyReport, CoreError> {
    let go_mod = exec_in_container(
        docker,
        container_id,
        ExecCommand::new(["cat", &format!("{repo_path}/go.mod")]),
    ).await?;

    let lock_check = exec_in_container(
        docker,
        container_id,
        ExecCommand::new(["test", "-f", &format!("{repo_path}/go.sum")]),
    ).await?;

    let deps = go_mod.stdout.lines()
        .skip_while(|l| !l.trim().starts_with("require"))
        .skip(1)
        .filter(|l| !l.trim().is_empty() && !l.trim().starts_with(')'))
        .map(|l| {
            let parts: Vec<&str> = l.trim().splitn(2, ' ').collect();
            Dependency {
                name: parts.first().unwrap_or(&"").to_string(),
                version: parts.get(1).map(|v| v.trim().to_string()),
                kind: DependencyKind::Runtime,
            }
        })
        .collect();

    Ok(DependencyReport {
        language: Language::Go,
        dependencies: deps,
        lock_file_present: lock_check.is_success(),
        manifest_file: "go.mod".to_string(),
        raw_output: go_mod.stdout,
    })
}

async fn analyze_java(docker: &Docker, container_id: &str, repo_path: &str) -> Result<DependencyReport, CoreError> {
    let pom = exec_in_container(
        docker,
        container_id,
        ExecCommand::new(["cat", &format!("{repo_path}/pom.xml")]),
    ).await?;

    Ok(DependencyReport {
        language: Language::Java,
        dependencies: vec![],
        lock_file_present: false,
        manifest_file: "pom.xml".to_string(),
        raw_output: pom.stdout,
    })
}

async fn analyze_ruby(docker: &Docker, container_id: &str, repo_path: &str) -> Result<DependencyReport, CoreError> {
    let gemfile = exec_in_container(
        docker,
        container_id,
        ExecCommand::new(["cat", &format!("{repo_path}/Gemfile")]),
    ).await?;

    let lock_check = exec_in_container(
        docker,
        container_id,
        ExecCommand::new(["test", "-f", &format!("{repo_path}/Gemfile.lock")]),
    ).await?;

    let deps = gemfile.stdout.lines()
        .filter(|l| l.trim().starts_with("gem "))
        .map(|l| {
            let parts: Vec<&str> = l.trim().splitn(3, ',').collect();
            let name = parts.first().unwrap_or(&"").trim_start_matches("gem ").trim_matches('\'').trim_matches('"').to_string();
            let version = parts.get(1).map(|v| v.trim().trim_matches('\'').trim_matches('"').to_string());
            Dependency { name, version, kind: DependencyKind::Runtime }
        })
        .collect();

    Ok(DependencyReport {
        language: Language::Ruby,
        dependencies: deps,
        lock_file_present: lock_check.is_success(),
        manifest_file: "Gemfile".to_string(),
        raw_output: gemfile.stdout,
    })
}

async fn analyze_php(docker: &Docker, container_id: &str, repo_path: &str) -> Result<DependencyReport, CoreError> {
    let composer = exec_in_container(
        docker,
        container_id,
        ExecCommand::new(["cat", &format!("{repo_path}/composer.json")]),
    ).await?;

    let lock_check = exec_in_container(
        docker,
        container_id,
        ExecCommand::new(["test", "-f", &format!("{repo_path}/composer.lock")]),
    ).await?;

    let deps = if let Ok(val) = serde_json::from_str::<serde_json::Value>(&composer.stdout) {
        let mut d = vec![];
        if let Some(obj) = val["require"].as_object() {
            for (name, ver) in obj {
                d.push(Dependency {
                    name: name.clone(),
                    version: ver.as_str().map(String::from),
                    kind: DependencyKind::Runtime,
                });
            }
        }
        d
    } else {
        vec![]
    };

    Ok(DependencyReport {
        language: Language::PHP,
        dependencies: deps,
        lock_file_present: lock_check.is_success(),
        manifest_file: "composer.json".to_string(),
        raw_output: composer.stdout,
    })
}
