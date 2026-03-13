pub mod commit;
pub mod containerize;
pub mod dependents;
pub mod gate;
pub mod governance;
pub mod workflow;

pub use commit::{CommittedBranch, CreateBranchAction, CTX_COMMITTED_BRANCHES};
pub use containerize::{ContainerizeAction, ContainerizeResult, CTX_CONTAINERIZE_RESULT};
pub use dependents::{DependentRepos, DiscoverDependentsAction, CTX_DEPENDENT_REPOS};
pub use gate::{
    DependencyDecision, DependencyDecisionGate, DependencyStrategy, CTX_DEPENDENCY_DECISION,
    CTX_DECISION_PROMPT,
};
pub use governance::{GenerateGovernanceFilesAction, GovernanceFile, CTX_GOVERNANCE_FILES};
pub use workflow::ImprovementWorkflowBuilder;

// ── Crate-level error type ────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum ImprovError {
    #[error("VCS error: {0}")]
    Vcs(#[from] rusty_venture_vcs::VcsError),

    #[error("LLM error: {0}")]
    Llm(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("core error: {0}")]
    Core(#[from] rusty_venture_core::CoreError),
}
