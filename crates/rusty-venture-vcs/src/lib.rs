pub mod azuredevops;
pub mod github;
pub mod gitlab;
pub mod provider;

pub use azuredevops::AzureDevOpsProvider;
pub use github::GithubProvider;
pub use gitlab::GitlabProvider;
pub use provider::{CodeSearchHit, CommitSha, FileCommit, RemoteRepo, RepoProvider, RepoRef};

// ── VcsError ──────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum VcsError {
    #[error("authentication error: {0}")]
    Auth(String),

    #[error("HTTP error: {0}")]
    Http(String),

    #[error("API error: {0}")]
    Api(String),

    #[error("parse error: {0}")]
    Parse(String),

    #[error("not implemented: {0}")]
    NotImplemented(String),

    #[error("rate limited — retry after {retry_after_secs}s")]
    RateLimited { retry_after_secs: u64 },
}
