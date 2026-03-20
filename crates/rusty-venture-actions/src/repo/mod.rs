pub mod analyze_deps;
pub mod audit_files;
pub mod clone;
pub mod detect_language;
pub mod find_dockerfiles;
pub mod governance;
pub mod maturity;
pub mod report;
pub mod scaffold;

pub use analyze_deps::{AnalyzeDepsAction, DependencyReport, CTX_DEPENDENCY_REPORT};
pub use audit_files::{AuditCommittedFilesAction, AuditReport, AuditViolation, CTX_AUDIT_REPORT};
pub use clone::{CloneRepoAction, CloneResult, CTX_REPO_LOCAL_PATH, CTX_REPO_URL};
pub use detect_language::{detect_language_local, DetectLanguageAction, DetectedLanguages, Language, CTX_DETECTED_LANGUAGES};
pub use find_dockerfiles::{DockerfileReport, FindDockerfilesAction, CTX_DOCKERFILE_REPORT};
pub use governance::{detect_governance_local, GovernanceCheckAction, GovernanceReport, CTX_GOVERNANCE_REPORT};
pub use maturity::{compute_maturity, MaturityDimension, MaturityGrade, MaturityScore, CTX_MATURITY_SCORE};
pub use report::{FinalReport, GenerateReportAction, CTX_FINAL_REPORT};
pub use scaffold::{
    DirectoryNode, FieldSpec, FilePurpose, FileSpec, GenerateScaffoldSpecAction,
    NodeKind, PlannedAction, ScaffoldSpec, TypeKind, TypeSpec, VariableScope,
    VariableSpec, CTX_IMPROVEMENT_INTENT, CTX_SCAFFOLD_SPEC,
};

use std::sync::Arc;

use anyhow::Context;
use bollard::Docker;
use rusty_venture_core::{
    workflow::{OnFailure, WorkflowBuilder, WorkflowEngine},
    ExecutionContext, RetryStrategy,
};
use rusty_venture_llm::LlmConnector;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::container::{
    CleanupContainerAction, ContainerConfig, SpawnContainerAction, CTX_DOCKER_CLIENT,
};

/// Request to run the full repository analysis workflow.
#[derive(Debug, Clone)]
pub struct RepoAnalysisRequest {
    pub repo_url: String,
    pub branch: Option<String>,
    pub claude_api_key: String,
    /// Docker socket path. `None` uses the platform default.
    pub docker_socket: Option<String>,
    /// When `true`, skip container spin-up and analyse using only local
    /// filesystem reads and the LLM. Requires `git` to be installed on the
    /// host. Dependency/audit analysis is skipped in this mode.
    pub skip_container: bool,
}

/// The result of a complete repository analysis run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoAnalysisResult {
    pub run_id: String,
    pub repo_url: String,
    pub report: FinalReport,
    /// Scientific maturity score computed from all analysis signals.
    pub maturity: MaturityScore,
    pub duration_ms: u64,
}

/// Build the two-phase repository analysis workflow using the programmatic DSL.
pub fn repo_analysis_workflow<C: LlmConnector + 'static>(
    repo_url: &str,
    branch: Option<String>,
    connector: Arc<C>,
    volume_name: &str,
) -> rusty_venture_core::workflow::Workflow {
    use rusty_venture_core::workflow::StepBuilder;

    let mut clone_action = CloneRepoAction::new(repo_url);
    if let Some(b) = branch {
        clone_action = clone_action.branch(b);
    }

    let clone_container = ContainerConfig::new("alpine/git")
        .memory_mb(256)
        .bind(format!("{volume_name}:/workspace:rw"));

    let analysis_container = ContainerConfig::new("ubuntu:22.04")
        .memory_mb(512)
        .network_disabled()
        .bind(format!("{volume_name}:/workspace:ro"));

    WorkflowBuilder::new("repo-analysis")
        // ── Phase 1: clone ──────────────────────────────────────────────────
        .step(
            StepBuilder::<(), _>::new("spawn-clone-container")
                .action(SpawnContainerAction::with_config(clone_container))
                .retry(RetryStrategy::Fixed {
                    max_attempts: 3,
                    delay: Duration::from_secs(3),
                })
                .on_failure(OnFailure::Abort)
                .build(),
        )
        .step(
            StepBuilder::<(), _>::new("clone-repo")
                .action(clone_action)
                .on_failure(OnFailure::Abort)
                .build(),
        )
        // ── Phase 2: analyse ────────────────────────────────────────────────
        .step(
            StepBuilder::<(), _>::new("spawn-analysis-container")
                .action(SpawnContainerAction::with_config(analysis_container))
                .retry(RetryStrategy::Fixed {
                    max_attempts: 2,
                    delay: Duration::from_secs(2),
                })
                .on_failure(OnFailure::Abort)
                .build(),
        )
        .step(
            StepBuilder::<(), _>::new("detect-language")
                .action(DetectLanguageAction)
                .on_failure(OnFailure::Continue)
                .build(),
        )
        .step(
            StepBuilder::<(), _>::new("analyze-deps")
                .action(AnalyzeDepsAction)
                .on_failure(OnFailure::LlmRemediate {
                    context_prompt: "Dependency analysis failed. Describe what additional steps might be needed to analyze dependencies for this repository.".to_string(),
                })
                .build(),
        )
        .step(
            StepBuilder::<(), _>::new("find-dockerfiles")
                .action(FindDockerfilesAction)
                .on_failure(OnFailure::Continue)
                .build(),
        )
        .step(
            StepBuilder::<(), _>::new("audit-files")
                .action(AuditCommittedFilesAction)
                .on_failure(OnFailure::Continue)
                .build(),
        )
        // Governance check runs in the analysis container after all other checks
        // so it has access to the same file tree.
        .step(
            StepBuilder::<(), _>::new("governance-check")
                .action(GovernanceCheckAction)
                .on_failure(OnFailure::Continue)
                .build(),
        )
        .step(
            StepBuilder::<(), _>::new("generate-report")
                .action(GenerateReportAction::new(connector))
                .on_failure(OnFailure::Abort)
                .build(),
        )
        .step(
            StepBuilder::<(), _>::new("cleanup-container")
                .action(CleanupContainerAction)
                .on_failure(OnFailure::Continue)
                .build(),
        )
        .build()
}

/// The no-container analysis path.
///
/// Clones the repository using the host's `git` binary, performs all
/// analysis purely via filesystem reads, and uses the LLM for the final
/// report. Dependency scanning and file-audit checks (which require an
/// isolated container) are skipped.
async fn run_repo_analysis_no_container(
    request: RepoAnalysisRequest,
) -> anyhow::Result<RepoAnalysisResult> {
    let start = std::time::Instant::now();

    // ── 1. Clone into a temp directory ───────────────────────────────────────
    let tmp_dir = tempfile::tempdir().context("create temp dir for no-container clone")?;
    let repo_path = tmp_dir.path().to_path_buf();

    let mut cmd = std::process::Command::new("git");
    cmd.args(["clone", "--depth", "1"]);
    if let Some(ref branch) = request.branch {
        cmd.args(["-b", branch.as_str()]);
    }
    cmd.arg(&request.repo_url).arg(&repo_path);

    let status = cmd.status().context("git clone failed")?;
    anyhow::ensure!(status.success(), "git clone exited with status {status}");

    // ── 2. Local analysis ────────────────────────────────────────────────────
    let lang = detect_language_local(&repo_path);
    let governance = detect_governance_local(&repo_path);

    let languages = DetectedLanguages {
        secondary: vec![],
        scores: vec![(lang.clone(), 10)],
        primary: lang,
    };

    // ── 3. Build execution context and run the LLM report step ──────────────
    let ctx = ExecutionContext::new("repo-analysis-no-container");
    ctx.insert(CTX_DETECTED_LANGUAGES, languages.clone()).await;
    ctx.insert(CTX_GOVERNANCE_REPORT, governance.clone()).await;
    ctx.insert(clone::CTX_REPO_URL, request.repo_url.clone()).await;

    let connector = Arc::new(rusty_venture_llm::ClaudeConnector::new(&request.claude_api_key));
    let report_action = report::GenerateReportAction::new(connector);

    rusty_venture_core::action::Action::execute(&report_action, &ctx, ())
        .await
        .map_err(|e| anyhow::anyhow!("Report generation failed: {e}"))?;

    let report: FinalReport = ctx
        .require::<FinalReport>(CTX_FINAL_REPORT)
        .await
        .context("Final report was not generated")?;

    // ── 4. Compute maturity from available signals ───────────────────────────
    let deps = DependencyReport::default();
    let dockerfiles = DockerfileReport::default();
    let audit = AuditReport::default();

    let maturity = compute_maturity(
        &report,
        &deps,
        &dockerfiles,
        &audit,
        &languages.primary,
        &governance,
    );

    ctx.insert(CTX_MATURITY_SCORE, maturity.clone()).await;

    let duration_ms = start.elapsed().as_millis() as u64;

    Ok(RepoAnalysisResult {
        run_id: ctx.run_id.to_string(),
        repo_url: request.repo_url,
        report,
        maturity,
        duration_ms,
    })
}

/// Run the full repository analysis workflow end-to-end.
/// This is the shared entry point used by both the CLI and HTTP server.
pub async fn run_repo_analysis(
    request: RepoAnalysisRequest,
) -> anyhow::Result<RepoAnalysisResult> {
    if request.skip_container {
        return run_repo_analysis_no_container(request).await;
    }

    let start = std::time::Instant::now();

    let docker = Arc::new(
        Docker::connect_with_local_defaults()
            .context("Failed to connect to Docker daemon. Is Docker running?")?,
    );

    let connector = Arc::new(rusty_venture_llm::ClaudeConnector::new(&request.claude_api_key));

    let ctx = ExecutionContext::new("repo-analysis");

    let volume_name = format!("rusty-venture-{}", ctx.run_id);
    docker
        .create_volume(bollard::volume::CreateVolumeOptions {
            name: volume_name.as_str(),
            ..Default::default()
        })
        .await
        .context("Failed to create Docker volume for repository workspace")?;

    ctx.insert(CTX_DOCKER_CLIENT, Arc::clone(&docker)).await;

    let workflow = repo_analysis_workflow(
        &request.repo_url,
        request.branch.clone(),
        connector,
        &volume_name,
    );

    let engine = WorkflowEngine::new();
    let run_result = engine.run(&workflow, &ctx).await;

    // Explicitly stop and remove the analysis container before removing the volume.
    if let Ok(container_id) = ctx.require::<String>(crate::container::CTX_CONTAINER_ID).await {
        use bollard::container::RemoveContainerOptions;
        let _ = docker.stop_container(&container_id, None).await;
        let _ = docker
            .remove_container(
                &container_id,
                Some(RemoveContainerOptions { force: true, v: true, ..Default::default() }),
            )
            .await;
    }

    // Always attempt to remove the shared volume.
    if let Err(e) = docker.remove_volume(&volume_name, None).await {
        tracing::warn!(volume = %volume_name, error = %e, "Failed to remove Docker volume");
    }

    run_result.context("Workflow failed")?;

    let report: FinalReport = ctx
        .require::<FinalReport>(CTX_FINAL_REPORT)
        .await
        .context("Final report was not generated")?;

    // ── Compute maturity score from all collected reports ───────────────────
    let deps = ctx
        .get::<DependencyReport>(CTX_DEPENDENCY_REPORT)
        .await
        .unwrap_or_default();
    let dockerfiles = ctx
        .get::<DockerfileReport>(CTX_DOCKERFILE_REPORT)
        .await
        .unwrap_or_default();
    let audit = ctx
        .get::<AuditReport>(CTX_AUDIT_REPORT)
        .await
        .unwrap_or_default();
    let languages = ctx
        .get::<DetectedLanguages>(CTX_DETECTED_LANGUAGES)
        .await
        .unwrap_or_default();
    let governance = ctx
        .get::<GovernanceReport>(CTX_GOVERNANCE_REPORT)
        .await
        .unwrap_or_default();

    let maturity = compute_maturity(
        &report,
        &deps,
        &dockerfiles,
        &audit,
        &languages.primary,
        &governance,
    );

    tracing::info!(
        composite_maturity = maturity.composite,
        grade = %maturity.grade.label(),
        "Maturity score computed"
    );

    // Store maturity in context so downstream actions (e.g. ScaffoldSpec) can access it.
    ctx.insert(CTX_MATURITY_SCORE, maturity.clone()).await;

    let duration_ms = start.elapsed().as_millis() as u64;

    Ok(RepoAnalysisResult {
        run_id: ctx.run_id.to_string(),
        repo_url: request.repo_url,
        report,
        maturity,
        duration_ms,
    })
}

// ── Default impls for graceful degradation when steps are skipped ───────────

impl Default for DependencyReport {
    fn default() -> Self {
        Self {
            language: Language::Unknown,
            dependencies: vec![],
            lock_file_present: false,
            manifest_file: String::new(),
            raw_output: String::new(),
        }
    }
}

impl Default for DockerfileReport {
    fn default() -> Self {
        Self {
            entries: vec![],
            has_root_dockerfile: false,
            has_root_compose: false,
            has_dockerignore: false,
            nested_count: 0,
        }
    }
}

impl Default for AuditReport {
    fn default() -> Self {
        Self {
            violations: vec![],
            total_tracked_files: 0,
            critical_count: 0,
            warning_count: 0,
        }
    }
}

impl Default for DetectedLanguages {
    fn default() -> Self {
        Self {
            primary: Language::Unknown,
            secondary: vec![],
            scores: vec![],
        }
    }
}
