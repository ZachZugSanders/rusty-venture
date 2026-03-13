use std::sync::Arc;

use rusty_venture_actions::repo::scaffold::{PlannedAction, ScaffoldSpec};
use rusty_venture_core::{
    dag::{DagNode, DagWorkflow, DagWorkflowBuilder},
    workflow::{OnFailure, StepBuilder},
};
use rusty_venture_llm::LlmConnector;
use rusty_venture_vcs::RepoProvider;
use tokio::sync::oneshot;

use crate::{
    commit::CreateBranchAction,
    containerize::ContainerizeAction,
    dependents::DiscoverDependentsAction,
    gate::DependencyDecisionGate,
    governance::GenerateGovernanceFilesAction,
};

/// Builds a `DagWorkflow` from a `ScaffoldSpec`, wiring dependencies between
/// nodes exactly as specified in `spec.actions[].depends_on`.
///
/// The final `create-branch` node is automatically added as a sink that waits
/// for all improvement actions to complete.
pub struct ImprovementWorkflowBuilder<C> {
    connector: Arc<C>,
    provider: Arc<dyn RepoProvider>,
    repo_owner: String,
    repo_name: String,
    base_branch: String,
    /// Oneshot receiver for the dependency decision gate.
    /// Only needed when the spec includes "discover-dependents".
    decision_rx: Option<oneshot::Receiver<String>>,
}

impl<C: LlmConnector + 'static> ImprovementWorkflowBuilder<C> {
    pub fn new(
        connector: Arc<C>,
        provider: Arc<dyn RepoProvider>,
        repo_owner: impl Into<String>,
        repo_name: impl Into<String>,
        base_branch: impl Into<String>,
    ) -> Self {
        Self {
            connector,
            provider,
            repo_owner: repo_owner.into(),
            repo_name: repo_name.into(),
            base_branch: base_branch.into(),
            decision_rx: None,
        }
    }

    /// Provide the oneshot channel that will receive the user's dependency
    /// strategy decision. Required when the spec contains "discover-dependents".
    pub fn decision_receiver(mut self, rx: oneshot::Receiver<String>) -> Self {
        self.decision_rx = Some(rx);
        self
    }

    /// Build a `DagWorkflow` from the planned actions in `spec`.
    /// Unknown action IDs are skipped with a warning.
    pub fn build(mut self, spec: &ScaffoldSpec) -> DagWorkflow {
        let mut builder = DagWorkflowBuilder::new("improvement");
        let mut commit_deps: Vec<String> = vec![];

        for planned in &spec.actions {
            let node_opt = self.make_node(planned);
            match node_opt {
                Some(node) => {
                    commit_deps.push(planned.id.clone());
                    builder = builder.node(node);
                }
                None => {
                    tracing::warn!(action = %planned.id, "Unknown action ID — skipped");
                }
            }
        }

        // create-branch is always the final sink.
        let commit_step = StepBuilder::<(), _>::new("create-branch")
            .action(CreateBranchAction::new(
                Arc::clone(&self.provider),
                &self.repo_owner,
                &self.repo_name,
                &self.base_branch,
            ))
            .on_failure(OnFailure::Continue)
            .build();

        builder = builder.node(DagNode::new(commit_step).depends_on(commit_deps));
        builder.build()
    }

    fn make_node(&mut self, planned: &PlannedAction) -> Option<DagNode> {
        let deps = planned.depends_on.clone();

        let step = match planned.id.as_str() {
            "containerize" => StepBuilder::<(), _>::new("containerize")
                .action(ContainerizeAction::new(Arc::clone(&self.connector)))
                .on_failure(OnFailure::Continue)
                .build(),

            "generate-governance-files" | "add-license" | "add-security-policy"
            | "add-contributing" | "add-changelog" | "add-dependabot" | "add-lint-config"
            | "add-pre-commit" | "add-safety-config" => {
                StepBuilder::<(), _>::new("generate-governance-files")
                    .action(GenerateGovernanceFilesAction)
                    .on_failure(OnFailure::Continue)
                    .build()
            }

            "discover-dependents" => StepBuilder::<(), _>::new("discover-dependents")
                .action(DiscoverDependentsAction::new(Arc::clone(&self.provider)))
                .on_failure(OnFailure::Continue)
                .build(),

            "dependency-decision-gate" => {
                let rx = self.decision_rx.take()?; // consumed — gate is single-use
                StepBuilder::<(), _>::new("dependency-decision-gate")
                    .action(DependencyDecisionGate::new(
                        Arc::clone(&self.connector),
                        rx,
                    ))
                    .on_failure(OnFailure::Continue)
                    .build()
            }

            _ => return None,
        };

        Some(DagNode::new(step).depends_on(deps))
    }
}
