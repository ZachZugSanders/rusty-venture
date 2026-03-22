/// Tier 3 — Active Functional Validation signals.
///
/// These checks actually *execute* commands inside the execution container
/// (build tools, test runners, linters). They run only when `scan_tier >= 3`
/// and require the `rusty-venture-runner-execution` image which ships
/// Rust, Node, Python, and Go toolchains.
///
/// Each check is independent — a timeout or failure in one does not prevent
/// the others from running. Results use `Option<bool>`:
///   `None`        → check was skipped (tool not applicable for this repo)
///   `Some(true)`  → check ran and passed (exit code 0)
///   `Some(false)` → check ran and failed (non-zero exit code)
use std::sync::Arc;

use async_trait::async_trait;
use bollard::Docker;
use rusty_venture_core::{action::Action, context::ExecutionContext, CoreError};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use super::detect_language::{DetectedLanguages, Language, CTX_DETECTED_LANGUAGES};
use crate::container::{exec_in_container, ExecCommand, CTX_CONTAINER_ID, CTX_DOCKER_CLIENT};

pub const CTX_ACTIVE_VALIDATION_REPORT: &str = "active_validation.report";

/// Results from Tier 3 active execution checks.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ActiveValidationReport {
    // ── Build ─────────────────────────────────────────────────────────────
    /// Command that was attempted (e.g. "cargo build --quiet").
    pub build_command: String,
    /// `None` = skipped, `Some(true)` = success, `Some(false)` = failed.
    pub build_result: Option<bool>,
    /// First 512 chars of combined stdout+stderr for the build.
    pub build_output_snippet: String,

    // ── Tests ─────────────────────────────────────────────────────────────
    pub test_command: String,
    pub test_result: Option<bool>,
    pub test_output_snippet: String,

    // ── Linter ────────────────────────────────────────────────────────────
    pub lint_command: String,
    pub lint_result: Option<bool>,
    pub lint_output_snippet: String,

    // ── Docker Compose ────────────────────────────────────────────────────
    /// `None` = no compose file found. `Some(true)` = YAML is valid.
    pub compose_result: Option<bool>,

    /// `true` when the action actually ran (scan_tier >= 3, container mode).
    pub was_checked: bool,
}

/// Runs Tier 3 active-validation checks in the execution container.
///
/// Uses the primary language detected by `DetectLanguageAction` to select
/// the appropriate build / test / lint commands.
pub struct ActiveValidationAction;

#[async_trait]
impl Action for ActiveValidationAction {
    type Input = ();
    type Output = ActiveValidationReport;

    fn name(&self) -> &str {
        "active-validation"
    }

    async fn execute(
        &self,
        ctx: &ExecutionContext,
        _input: (),
    ) -> Result<ActiveValidationReport, CoreError> {
        let docker: Arc<Docker> = ctx.require::<Arc<Docker>>(CTX_DOCKER_CLIENT).await?;
        let container_id: String = ctx.require::<String>(CTX_CONTAINER_ID).await?;

        let languages = ctx
            .get::<DetectedLanguages>(CTX_DETECTED_LANGUAGES)
            .await
            .unwrap_or_default();

        let report = run_active_validation(&docker, &container_id, &languages.primary).await;

        info!(
            build = ?report.build_result,
            tests = ?report.test_result,
            lint  = ?report.lint_result,
            compose = ?report.compose_result,
            "Active validation complete"
        );

        ctx.insert(CTX_ACTIVE_VALIDATION_REPORT, report.clone())
            .await;
        Ok(report)
    }
}

// ── Command selection ─────────────────────────────────────────────────────────

/// Returns `(build_cmd, test_cmd, lint_cmd)` for the given primary language.
/// Each is a `Vec<&str>` ready for `exec_in_container`.
fn commands_for_language(lang: &Language) -> Commands {
    match lang {
        Language::Rust => Commands {
            build: Some(vec!["cargo", "build", "--quiet"]),
            test: Some(vec![
                "cargo",
                "test",
                "--quiet",
                "--",
                "--test-output=immediate",
            ]),
            lint: Some(vec!["cargo", "clippy", "--quiet", "--", "-D", "warnings"]),
        },
        Language::Node => Commands {
            // npm run build may not exist — guarded at runtime
            build: Some(vec!["npm", "run", "build", "--if-present"]),
            test: Some(vec!["npm", "test", "--if-present"]),
            lint: Some(vec!["npm", "run", "lint", "--if-present"]),
        },
        Language::Python => Commands {
            build: None, // Python projects don't typically have a build step for tests
            test: Some(vec!["python3", "-m", "pytest", "-q", "--tb=short"]),
            lint: Some(vec!["python3", "-m", "ruff", "check", "."]),
        },
        Language::Go => Commands {
            build: Some(vec!["go", "build", "./..."]),
            test: Some(vec!["go", "test", "./...", "-v", "-count=1"]),
            lint: None, // golangci-lint not installed in base image
        },
        _ => Commands {
            build: None,
            test: None,
            lint: None,
        },
    }
}

struct Commands {
    build: Option<Vec<&'static str>>,
    test: Option<Vec<&'static str>>,
    lint: Option<Vec<&'static str>>,
}

// ── Core implementation ───────────────────────────────────────────────────────

const REPO_DIR: &str = "/workspace/repo";
const BUILD_TIMEOUT: u64 = 600; // 10 min — deps may need downloading
const TEST_TIMEOUT: u64 = 600; // 10 min
const LINT_TIMEOUT: u64 = 120; // 2 min

async fn run_active_validation(
    docker: &Arc<Docker>,
    container_id: &str,
    lang: &Language,
) -> ActiveValidationReport {
    let cmds = commands_for_language(lang);

    // ── Build ─────────────────────────────────────────────────────────────
    let (build_command, build_result, build_output_snippet) = match cmds.build {
        None => (String::new(), None, String::new()),
        Some(cmd) => {
            let label = cmd.join(" ");
            info!("Running build: {label}");
            let r = exec_checked(docker, container_id, cmd, BUILD_TIMEOUT).await;
            let snippet = r.as_ref().map(|(o, _)| o.clone()).unwrap_or_default();
            let passed = r.map(|(_, ok)| ok).ok();
            if let Some(false) = passed {
                warn!(cmd = %label, "Build failed");
            }
            (label, passed, snippet)
        }
    };

    // ── Tests ─────────────────────────────────────────────────────────────
    let (test_command, test_result, test_output_snippet) = match cmds.test {
        None => (String::new(), None, String::new()),
        Some(cmd) => {
            let label = cmd.join(" ");
            info!("Running tests: {label}");
            let r = exec_checked(docker, container_id, cmd, TEST_TIMEOUT).await;
            let snippet = r.as_ref().map(|(o, _)| o.clone()).unwrap_or_default();
            let passed = r.map(|(_, ok)| ok).ok();
            if let Some(false) = passed {
                warn!(cmd = %label, "Tests failed");
            }
            (label, passed, snippet)
        }
    };

    // ── Linter ────────────────────────────────────────────────────────────
    let (lint_command, lint_result, lint_output_snippet) = match cmds.lint {
        None => (String::new(), None, String::new()),
        Some(cmd) => {
            let label = cmd.join(" ");
            info!("Running linter: {label}");
            let r = exec_checked(docker, container_id, cmd, LINT_TIMEOUT).await;
            let snippet = r.as_ref().map(|(o, _)| o.clone()).unwrap_or_default();
            let passed = r.map(|(_, ok)| ok).ok();
            if let Some(false) = passed {
                warn!(cmd = %label, "Linter reported issues");
            }
            (label, passed, snippet)
        }
    };

    // ── Docker Compose validation ─────────────────────────────────────────
    // Use Python to validate YAML syntax — avoids requiring a running Docker daemon.
    let compose_result = check_compose_yaml(docker, container_id).await;

    ActiveValidationReport {
        build_command,
        build_result,
        build_output_snippet,
        test_command,
        test_result,
        test_output_snippet,
        lint_command,
        lint_result,
        lint_output_snippet,
        compose_result,
        was_checked: true,
    }
}

/// Execute a command and return `Ok((output_snippet, passed))`.
/// Returns `Err` only on Docker communication failure; command failures
/// produce `Ok((.., false))`.
async fn exec_checked(
    docker: &Arc<Docker>,
    container_id: &str,
    cmd: Vec<&'static str>,
    timeout_secs: u64,
) -> Result<(String, bool), ()> {
    let result = exec_in_container(
        docker,
        container_id,
        ExecCommand {
            cmd: cmd.iter().map(|s| s.to_string()).collect(),
            working_dir: Some(REPO_DIR.to_string()),
            env: vec![],
            timeout_secs: Some(timeout_secs),
        },
    )
    .await;

    match result {
        Err(e) => {
            warn!(error = %e, "Docker exec error");
            Err(())
        }
        Ok(r) => {
            let combined = format!("{}{}", r.stdout, r.stderr);
            let snippet = combined.chars().take(512).collect();
            Ok((snippet, r.exit_code == 0))
        }
    }
}

/// Validate `docker-compose.yml` / `compose.yml` YAML syntax using Python's
/// yaml module (present in the execution image). Returns `None` if no compose
/// file is found.
async fn check_compose_yaml(docker: &Arc<Docker>, container_id: &str) -> Option<bool> {
    // First check if a compose file exists at all.
    let find = exec_in_container(
        docker,
        container_id,
        ExecCommand {
            cmd: vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                "[ -f docker-compose.yml ] || [ -f docker-compose.yaml ] || [ -f compose.yml ] || [ -f compose.yaml ] && echo FOUND || echo NOTFOUND".to_string(),
            ],
            working_dir: Some(REPO_DIR.to_string()),
            env: vec![],
            timeout_secs: Some(10),
        },
    )
    .await
    .ok()?;

    if !find.stdout.trim().contains("FOUND") {
        return None; // no compose file — signal is not applicable
    }

    // Validate YAML syntax
    let validate = exec_in_container(
        docker,
        container_id,
        ExecCommand {
            cmd: vec![
                "python3".to_string(),
                "-c".to_string(),
                r#"
import yaml, sys, os
for name in ['docker-compose.yml','docker-compose.yaml','compose.yml','compose.yaml']:
    if os.path.exists(name):
        try:
            yaml.safe_load(open(name))
            sys.exit(0)
        except yaml.YAMLError as e:
            print(f"YAML error: {e}", file=sys.stderr)
            sys.exit(1)
sys.exit(0)
"#
                .to_string(),
            ],
            working_dir: Some(REPO_DIR.to_string()),
            env: vec![],
            timeout_secs: Some(30),
        },
    )
    .await
    .ok()?;

    Some(validate.exit_code == 0)
}
