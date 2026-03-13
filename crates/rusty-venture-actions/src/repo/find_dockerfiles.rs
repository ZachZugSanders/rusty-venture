use std::sync::Arc;

use async_trait::async_trait;
use bollard::Docker;
use rusty_venture_core::{action::Action, context::ExecutionContext, CoreError};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::container::{exec_in_container, ExecCommand, CTX_CONTAINER_ID, CTX_DOCKER_CLIENT};
use super::clone::CTX_REPO_LOCAL_PATH;

pub const CTX_DOCKERFILE_REPORT: &str = "repo.dockerfile_report";

/// An entry for a discovered Docker-related file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerfileEntry {
    /// Path relative to the repo root.
    pub path: String,
    pub kind: DockerfileKind,
    /// Whether this file is in a subdirectory (not the repo root).
    pub is_nested: bool,
    /// Any potential issues detected (e.g. hardcoded secrets, FROM latest).
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum DockerfileKind {
    Dockerfile,
    DockerCompose,
    DockerIgnore,
    DockerEnv,
}

impl std::fmt::Display for DockerfileKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            DockerfileKind::Dockerfile => "Dockerfile",
            DockerfileKind::DockerCompose => "docker-compose",
            DockerfileKind::DockerIgnore => ".dockerignore",
            DockerfileKind::DockerEnv => ".env (docker)",
        };
        write!(f, "{s}")
    }
}

/// The aggregated result of scanning for Docker-related files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerfileReport {
    pub entries: Vec<DockerfileEntry>,
    pub has_root_dockerfile: bool,
    pub has_root_compose: bool,
    pub has_dockerignore: bool,
    pub nested_count: usize,
}

/// Scans the entire repository tree for Dockerfile and docker-compose files.
#[derive(Default)]
pub struct FindDockerfilesAction;

#[async_trait]
impl Action for FindDockerfilesAction {
    type Input = ();
    type Output = DockerfileReport;

    fn name(&self) -> &str {
        "find-dockerfiles"
    }

    async fn execute(&self, ctx: &ExecutionContext, _input: ()) -> Result<DockerfileReport, CoreError> {
        let container_id: String = ctx.require::<String>(CTX_CONTAINER_ID).await?;
        let docker: Arc<Docker> = ctx.require::<Arc<Docker>>(CTX_DOCKER_CLIENT).await?;
        let repo_path: String = ctx.require::<String>(CTX_REPO_LOCAL_PATH).await?;

        // Find all Dockerfile variants and docker-compose files
        let find_result = exec_in_container(
            &docker,
            &container_id,
            ExecCommand::new([
                "find", &repo_path,
                "-type", "f",
                "(", "-name", "Dockerfile", "-o",
                      "-name", "Dockerfile.*",
                      "-o", "-name", "*.dockerfile",
                      "-o", "-name", "docker-compose.yml",
                      "-o", "-name", "docker-compose.yaml",
                      "-o", "-name", "docker-compose.*.yml",
                      "-o", "-name", "docker-compose.*.yaml",
                      "-o", "-name", ".dockerignore",
                ")",
                "-not", "-path", "*/\\.git/*",
            ]),
        ).await?;

        let mut entries = vec![];

        for line in find_result.stdout.lines() {
            let path = line.trim();
            if path.is_empty() {
                continue;
            }

            // Make path relative to repo root
            let relative = path
                .strip_prefix(&format!("{repo_path}/"))
                .unwrap_or(path)
                .to_string();

            let is_nested = relative.contains('/');

            let kind = if relative.contains("docker-compose") {
                DockerfileKind::DockerCompose
            } else if relative.ends_with(".dockerignore") || relative == ".dockerignore" {
                DockerfileKind::DockerIgnore
            } else {
                DockerfileKind::Dockerfile
            };

            // Read the file and check for common warnings
            let file_content = exec_in_container(
                &docker,
                &container_id,
                ExecCommand::new(["cat", path]),
            ).await?;

            let warnings = check_dockerfile_warnings(&file_content.stdout, &kind);

            entries.push(DockerfileEntry {
                path: relative,
                kind,
                is_nested,
                warnings,
            });
        }

        let has_root_dockerfile = entries.iter().any(|e| e.kind == DockerfileKind::Dockerfile && !e.is_nested);
        let has_root_compose = entries.iter().any(|e| e.kind == DockerfileKind::DockerCompose && !e.is_nested);
        let has_dockerignore = entries.iter().any(|e| e.kind == DockerfileKind::DockerIgnore);
        let nested_count = entries.iter().filter(|e| e.is_nested).count();

        info!(
            total = entries.len(),
            has_root_dockerfile,
            has_root_compose,
            nested = nested_count,
            "Dockerfile scan complete"
        );

        let report = DockerfileReport {
            entries,
            has_root_dockerfile,
            has_root_compose,
            has_dockerignore,
            nested_count,
        };

        ctx.insert(CTX_DOCKERFILE_REPORT, report.clone()).await;
        Ok(report)
    }
}

fn check_dockerfile_warnings(content: &str, kind: &DockerfileKind) -> Vec<String> {
    let mut warnings = vec![];

    match kind {
        DockerfileKind::Dockerfile => {
            for line in content.lines() {
                let upper = line.trim().to_uppercase();
                if upper.starts_with("FROM") && upper.contains(":LATEST") {
                    warnings.push("Uses ':latest' tag — unpredictable builds. Pin to a specific version.".to_string());
                }
                if upper.starts_with("FROM") && upper == "FROM SCRATCH" {
                    // Fine, no warning
                }
                if upper.contains("PASSWORD") || upper.contains("SECRET") || upper.contains("API_KEY") {
                    warnings.push("Possible hardcoded secret in ENV or ARG instruction.".to_string());
                }
                if upper.starts_with("USER ROOT") || (upper.starts_with("USER") && upper.ends_with("ROOT")) {
                    warnings.push("Container runs as root. Consider adding a non-root USER.".to_string());
                }
            }
            if !content.to_uppercase().contains("HEALTHCHECK") {
                warnings.push("No HEALTHCHECK instruction found.".to_string());
            }
        }
        DockerfileKind::DockerCompose => {
            if content.contains("privileged: true") {
                warnings.push("Service runs with 'privileged: true' — security risk.".to_string());
            }
            if content.contains(":latest") {
                warnings.push("Service image uses ':latest' tag — unpredictable deployments.".to_string());
            }
        }
        _ => {}
    }

    warnings
}
