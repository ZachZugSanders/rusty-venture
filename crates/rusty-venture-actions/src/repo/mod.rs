pub mod active_validation;
pub mod analyze_deps;
pub mod audit_files;
pub mod clone;
pub mod content_quality;
pub mod detect_language;
pub mod detect_llm;
pub mod find_dockerfiles;
pub mod governance;
pub mod maturity;
pub mod report;
pub mod scaffold;

pub use active_validation::{
    ActiveValidationAction, ActiveValidationReport, CTX_ACTIVE_VALIDATION_REPORT,
};
pub use analyze_deps::{AnalyzeDepsAction, DependencyReport, CTX_DEPENDENCY_REPORT};
pub use audit_files::{AuditCommittedFilesAction, AuditReport, AuditViolation, CTX_AUDIT_REPORT};
pub use clone::{CloneRepoAction, CloneResult, CTX_REPO_LOCAL_PATH, CTX_REPO_URL};
pub use content_quality::{
    detect_content_quality_local, ContentQualityCheckAction, ContentQualityReport,
    CTX_CONTENT_QUALITY_REPORT,
};
pub use detect_language::{
    detect_language_local, DetectLanguageAction, DetectedLanguages, Language,
    CTX_DETECTED_LANGUAGES,
};
pub use detect_llm::{detect_llm_config_local, DetectLlmConfigAction, LlmConfig, LlmProvider, CTX_LLM_CONFIG};
pub use find_dockerfiles::{DockerfileReport, FindDockerfilesAction, CTX_DOCKERFILE_REPORT};
pub use governance::{
    detect_governance_local, GovernanceCheckAction, GovernanceReport, CTX_GOVERNANCE_REPORT,
};
pub use maturity::{
    compute_maturity, MaturityDimension, MaturityGrade, MaturityScore, CTX_MATURITY_SCORE,
};
pub use report::{FinalReport, GenerateReportAction, CTX_FINAL_REPORT};
pub use scaffold::{
    DirectoryNode, FieldSpec, FilePurpose, FileSpec, GenerateScaffoldSpecAction, NodeKind,
    PlannedAction, ScaffoldSpec, TypeKind, TypeSpec, VariableScope, VariableSpec,
    CTX_IMPROVEMENT_INTENT, CTX_SCAFFOLD_SPEC,
};

use std::sync::Arc;

use anyhow::Context;
use bollard::Docker;
use rusty_venture_core::{
    workflow::{OnFailure, StepBuilder, WorkflowBuilder, WorkflowEngine},
    DagEngine, DagNode, DagWorkflowBuilder, ExecutionContext, RetryStrategy,
};
use rusty_venture_llm::LlmConnector;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::container::{
    CacheRepoImageAction, CleanupContainerAction, ContainerConfig, SpawnContainerAction,
    CTX_DOCKER_CLIENT,
};

/// Pre-built runner image for the clone phase (has git + ca-certs, network enabled).
/// Built via: `docker compose build --profile runners`
pub const RUNNER_CLONE_IMAGE: &str = "rusty-venture-runner-clone:latest";

/// Pre-built runner image for the analysis phase (POSIX tools only, no network at runtime).
/// Built via: `docker compose build --profile runners`
pub const RUNNER_ANALYSIS_IMAGE: &str = "rusty-venture-runner-analysis:latest";

/// Pre-built runner image for Tier 3 active validation (language toolchains,
/// network enabled so package managers can fetch dependencies).
/// Built via: `docker compose build --profile runners`
pub const RUNNER_EXECUTION_IMAGE: &str = "rusty-venture-runner-execution:latest";

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
    /// Optional channel for streaming structured log lines to the caller.
    /// The server injects this to forward events over SSE.
    pub log_tx: Option<rusty_venture_core::LogSink>,
    /// Git commit SHA pre-fetched by the caller (e.g. via GitHub API) before
    /// the workflow starts. Stored in the scan row for deduplication.
    pub commit_hash: Option<String>,
    /// When `true`, commit the clone container (with checked-out repo) as a
    /// named local Docker image (`rv-cache-{owner}-{repo}:latest`) immediately
    /// after cloning. The image is stored only in the local daemon and is never
    /// pushed to a registry. Ignored when `skip_container` is `true`.
    pub cache_repo_image: bool,
    /// Which maturity tier to execute (1 = Static Discovery, 2 = Content
    /// Quality, 3 = Active Functional Validation). Defaults to `1`.
    /// The caller is responsible for validating this against `max_unlocked_tier`
    /// before constructing the request.
    pub scan_tier: u8,
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
    /// Git commit SHA that was scanned, if known.
    pub commit_hash: Option<String>,
    /// Which maturity tier was executed for this scan.
    pub scan_tier: u8,
}

/// Build the two-phase repository analysis workflow using the programmatic DSL.
pub fn repo_analysis_workflow<C: LlmConnector + 'static>(
    repo_url: &str,
    branch: Option<String>,
    connector: Arc<C>,
    volume_name: &str,
    cache_repo_image: bool,
    scan_tier: u8,
) -> rusty_venture_core::workflow::Workflow {

    let mut clone_action = CloneRepoAction::new(repo_url);
    if let Some(b) = branch {
        clone_action = clone_action.branch(b);
    }

    // Derive a short ID from the volume name (format: "rusty-venture-{run_id}").
    // First 8 chars of the run_id keep names readable in `docker ps`.
    let short_id: String = volume_name
        .strip_prefix("rusty-venture-")
        .unwrap_or(volume_name)
        .chars()
        .take(8)
        .collect();

    let clone_container = ContainerConfig::new(RUNNER_CLONE_IMAGE)
        .name(format!("rv-clone-{short_id}"))
        .memory_mb(256)
        .bind(format!("{volume_name}:/workspace:rw"));

    let analysis_container = ContainerConfig::new(RUNNER_ANALYSIS_IMAGE)
        .name(format!("rv-analysis-{short_id}"))
        .memory_mb(512)
        .network_disabled()
        .bind(format!("{volume_name}:/workspace:ro"));

    let mut builder = WorkflowBuilder::new("repo-analysis")
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
        );

    // ── Optional: snapshot the clone container as a cached image ────────────
    // Runs immediately after clone while the clone container is still active.
    // Uses OnFailure::Continue so a caching failure never aborts the scan.
    if cache_repo_image {
        builder = builder.step(
            StepBuilder::<(), _>::new("cache-repo-image")
                .action(CacheRepoImageAction::new(repo_url))
                .on_failure(OnFailure::Continue)
                .build(),
        );
    }

    // ── Phase 2: analyse ─────────────────────────────────────────────────────
    builder = builder
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
        );

    // ── Tier 2: content-quality check ────────────────────────────────────────
    if scan_tier >= 2 {
        builder = builder.step(
            StepBuilder::<(), _>::new("content-quality-check")
                .action(ContentQualityCheckAction)
                .on_failure(OnFailure::Continue)
                .build(),
        );
    }

    // ── Tier 3: active functional validation ─────────────────────────────────
    // Spawn a fresh execution container (network-enabled, read-write volume
    // mount, language toolchains). Inserting into CTX_CONTAINER_ID causes the
    // RAII guard to clean up the analysis container automatically.
    if scan_tier >= 3 {
        let execution_container = ContainerConfig::new(RUNNER_EXECUTION_IMAGE)
            .name(format!("rv-exec-{short_id}"))
            .memory_mb(2048)
            // No .network_disabled() — network is ON so deps can be fetched.
            .bind(format!("{volume_name}:/workspace:rw"));

        builder = builder
            .step(
                StepBuilder::<(), _>::new("spawn-execution-container")
                    .action(SpawnContainerAction::with_config(execution_container))
                    .retry(RetryStrategy::Fixed {
                        max_attempts: 2,
                        delay: Duration::from_secs(2),
                    })
                    .on_failure(OnFailure::Abort)
                    .build(),
            )
            .step(
                StepBuilder::<(), _>::new("active-validation")
                    .action(ActiveValidationAction)
                    .on_failure(OnFailure::Continue)
                    .build(),
            );
    }

    builder
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
    let mut ctx = ExecutionContext::new("repo-analysis-no-container");
    if let Some(tx) = request.log_tx {
        ctx.log_sink = Some(tx);
    }
    ctx.emit_log(
        "info",
        Some("clone-repo"),
        "Cloning repository (no-container mode)",
    );
    ctx.insert(CTX_DETECTED_LANGUAGES, languages.clone()).await;
    ctx.insert(CTX_GOVERNANCE_REPORT, governance.clone()).await;
    ctx.insert(clone::CTX_REPO_URL, request.repo_url.clone())
        .await;
    ctx.emit_log("info", Some("detect-language"), "Detecting language");

    let connector = Arc::new(rusty_venture_llm::ClaudeConnector::new(
        &request.claude_api_key,
    ));
    let report_action = report::GenerateReportAction::new(connector);

    ctx.emit_log("info", Some("generate-report"), "Generating LLM report");
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

    // Tier 2 content quality is skipped in no-container mode by default, but
    // we can still run a local version if scan_tier >= 2.
    let content_quality = if request.scan_tier >= 2 {
        Some(detect_content_quality_local(&repo_path))
    } else {
        None
    };

    // Tier 3 active validation is not supported in no-container mode
    // (running arbitrary test suites on the host would be unsafe).
    let llm_config = detect_llm_config_local(&repo_path);
    let maturity = compute_maturity(
        &report,
        &deps,
        &dockerfiles,
        &audit,
        &languages.primary,
        &governance,
        content_quality.as_ref(),
        None, // no active validation in no-container mode
        Some(&llm_config),
    );

    ctx.emit_log("info", None, "Analysis complete");
    ctx.insert(CTX_MATURITY_SCORE, maturity.clone()).await;

    let duration_ms = start.elapsed().as_millis() as u64;

    Ok(RepoAnalysisResult {
        run_id: ctx.run_id.to_string(),
        repo_url: request.repo_url,
        report,
        maturity,
        duration_ms,
        commit_hash: request.commit_hash,
        scan_tier: request.scan_tier,
    })
}

/// Run the full repository analysis workflow end-to-end.
/// This is the shared entry point used by both the CLI and HTTP server.
pub async fn run_repo_analysis(request: RepoAnalysisRequest) -> anyhow::Result<RepoAnalysisResult> {
    if request.skip_container {
        return run_repo_analysis_no_container(request).await;
    }

    let start = std::time::Instant::now();

    let docker = Arc::new(
        Docker::connect_with_local_defaults()
            .context("Failed to connect to Docker daemon. Is Docker running?")?,
    );

    let connector = Arc::new(rusty_venture_llm::ClaudeConnector::new(
        &request.claude_api_key,
    ));

    let mut ctx = ExecutionContext::new("repo-analysis");
    if let Some(tx) = request.log_tx {
        ctx.log_sink = Some(tx);
    }

    let volume_name = format!("rusty-venture-{}", ctx.run_id);
    docker
        .create_volume(bollard::volume::CreateVolumeOptions {
            name: volume_name.as_str(),
            ..Default::default()
        })
        .await
        .context("Failed to create Docker volume for repository workspace")?;

    ctx.insert(CTX_DOCKER_CLIENT, Arc::clone(&docker)).await;

    // Derive a short ID for readable container names (first 8 chars of run_id).
    let short_id: String = volume_name
        .strip_prefix("rusty-venture-")
        .unwrap_or(&volume_name)
        .chars()
        .take(8)
        .collect();

    let mut clone_action = CloneRepoAction::new(&request.repo_url);
    if let Some(b) = request.branch.clone() {
        clone_action = clone_action.branch(b);
    }

    let clone_container = ContainerConfig::new(RUNNER_CLONE_IMAGE)
        .name(format!("rv-clone-{short_id}"))
        .memory_mb(256)
        .bind(format!("{volume_name}:/workspace:rw"));

    let analysis_container = ContainerConfig::new(RUNNER_ANALYSIS_IMAGE)
        .name(format!("rv-analysis-{short_id}"))
        .memory_mb(512)
        .network_disabled()
        .bind(format!("{volume_name}:/workspace:ro"));

    let engine = WorkflowEngine::new();

    // ── Phase 1: clone + spawn analysis container (sequential) ───────────────
    let mut phase1_builder = WorkflowBuilder::new("repo-analysis-phase1")
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
        );

    if request.cache_repo_image {
        phase1_builder = phase1_builder.step(
            StepBuilder::<(), _>::new("cache-repo-image")
                .action(CacheRepoImageAction::new(&request.repo_url))
                .on_failure(OnFailure::Continue)
                .build(),
        );
    }

    let phase1 = phase1_builder
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
        .build();

    let mut run_result: anyhow::Result<()> =
        engine.run(&phase1, &ctx).await.context("Phase 1 (clone) failed");

    // ── Phase 2: five analysis steps running in parallel (DAG) ───────────────
    //
    // detect-language, analyze-deps, find-dockerfiles, audit-files, and
    // governance-check all read from the shared workspace written by the clone
    // phase and write to disjoint context keys — they have no inter-dependencies
    // so they run concurrently, cutting analysis time roughly 5×.
    if run_result.is_ok() {
        let analysis_dag = DagWorkflowBuilder::new("repo-analysis-dag")
            .node(DagNode::new(
                StepBuilder::<(), _>::new("detect-language")
                    .action(DetectLanguageAction)
                    .on_failure(OnFailure::Continue)
                    .build(),
            ))
            .node(DagNode::new(
                StepBuilder::<(), _>::new("analyze-deps")
                    .action(AnalyzeDepsAction)
                    .on_failure(OnFailure::LlmRemediate {
                        context_prompt: "Dependency analysis failed. Describe what additional steps might be needed to analyze dependencies for this repository.".to_string(),
                    })
                    .build(),
            ))
            .node(DagNode::new(
                StepBuilder::<(), _>::new("find-dockerfiles")
                    .action(FindDockerfilesAction)
                    .on_failure(OnFailure::Continue)
                    .build(),
            ))
            .node(DagNode::new(
                StepBuilder::<(), _>::new("audit-files")
                    .action(AuditCommittedFilesAction)
                    .on_failure(OnFailure::Continue)
                    .build(),
            ))
            .node(DagNode::new(
                StepBuilder::<(), _>::new("governance-check")
                    .action(GovernanceCheckAction)
                    .on_failure(OnFailure::Continue)
                    .build(),
            ))
            .node(DagNode::new(
                StepBuilder::<(), _>::new("detect-llm-config")
                    .action(DetectLlmConfigAction)
                    .on_failure(OnFailure::Continue)
                    .build(),
            ))
            .build();

        run_result = DagEngine::run(analysis_dag, &ctx)
            .await
            .context("Analysis DAG failed");
    }

    // ── Phase 3: post-analysis steps (sequential) ────────────────────────────
    if run_result.is_ok() {
        let mut phase3_builder = WorkflowBuilder::new("repo-analysis-phase3");

        if request.scan_tier >= 2 {
            phase3_builder = phase3_builder.step(
                StepBuilder::<(), _>::new("content-quality-check")
                    .action(ContentQualityCheckAction)
                    .on_failure(OnFailure::Continue)
                    .build(),
            );
        }

        if request.scan_tier >= 3 {
            let execution_container = ContainerConfig::new(RUNNER_EXECUTION_IMAGE)
                .name(format!("rv-exec-{short_id}"))
                .memory_mb(2048)
                .bind(format!("{volume_name}:/workspace:rw"));

            phase3_builder = phase3_builder
                .step(
                    StepBuilder::<(), _>::new("spawn-execution-container")
                        .action(SpawnContainerAction::with_config(execution_container))
                        .retry(RetryStrategy::Fixed {
                            max_attempts: 2,
                            delay: Duration::from_secs(2),
                        })
                        .on_failure(OnFailure::Abort)
                        .build(),
                )
                .step(
                    StepBuilder::<(), _>::new("active-validation")
                        .action(ActiveValidationAction)
                        .on_failure(OnFailure::Continue)
                        .build(),
                );
        }

        let phase3 = phase3_builder
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
            .build();

        run_result = engine
            .run(&phase3, &ctx)
            .await
            .context("Phase 3 (report) failed");
    }

    // Explicitly stop and remove the analysis container before removing the volume.
    if let Ok(container_id) = ctx
        .require::<String>(crate::container::CTX_CONTAINER_ID)
        .await
    {
        use bollard::container::RemoveContainerOptions;
        let _ = docker.stop_container(&container_id, None).await;
        let _ = docker
            .remove_container(
                &container_id,
                Some(RemoveContainerOptions {
                    force: true,
                    v: true,
                    ..Default::default()
                }),
            )
            .await;
    }

    // Always attempt to remove the shared volume.
    if let Err(e) = docker.remove_volume(&volume_name, None).await {
        tracing::warn!(volume = %volume_name, error = %e, "Failed to remove Docker volume");
    }

    run_result?;

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

    let content_quality = ctx
        .get::<ContentQualityReport>(CTX_CONTENT_QUALITY_REPORT)
        .await;

    let active_validation = ctx
        .get::<ActiveValidationReport>(CTX_ACTIVE_VALIDATION_REPORT)
        .await;

    let llm_config = ctx
        .get::<LlmConfig>(CTX_LLM_CONFIG)
        .await;

    let maturity = compute_maturity(
        &report,
        &deps,
        &dockerfiles,
        &audit,
        &languages.primary,
        &governance,
        content_quality.as_ref(),
        active_validation.as_ref(),
        llm_config.as_ref(),
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
        commit_hash: request.commit_hash,
        scan_tier: request.scan_tier,
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

impl Default for DetectedLanguages {
    fn default() -> Self {
        Self {
            primary: Language::Unknown,
            secondary: vec![],
            scores: vec![],
        }
    }
}
