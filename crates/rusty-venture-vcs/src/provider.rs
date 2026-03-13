use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::VcsError;

// ── Shared types ──────────────────────────────────────────────────────────────

/// A repository on a remote VCS host.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteRepo {
    pub owner: String,
    pub name: String,
    /// Convenience: `"owner/name"`.
    pub full_name: String,
    pub default_branch: String,
    pub clone_url: String,
    pub description: Option<String>,
    /// Primary language as reported by the host (e.g. `"Rust"`, `"TypeScript"`).
    pub language: Option<String>,
}

/// Identifies a repository on a remote host (owner + name pair).
#[derive(Debug, Clone)]
pub struct RepoRef {
    pub owner: String,
    pub name: String,
}

impl RepoRef {
    pub fn new(owner: impl Into<String>, name: impl Into<String>) -> Self {
        Self { owner: owner.into(), name: name.into() }
    }

    pub fn full_name(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}

impl std::fmt::Display for RepoRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.full_name())
    }
}

/// A single file to create or update in a commit.
#[derive(Debug, Clone)]
pub struct FileCommit {
    /// Repository-relative path, e.g. `"Dockerfile"` or `"src/main.rs"`.
    pub path: String,
    /// Full UTF-8 content of the file.
    pub content: String,
    /// The blob SHA of the existing file, required when **updating** a file.
    /// Leave `None` when creating a new file.
    pub existing_sha: Option<String>,
}

impl FileCommit {
    pub fn create(path: impl Into<String>, content: impl Into<String>) -> Self {
        Self { path: path.into(), content: content.into(), existing_sha: None }
    }

    pub fn update(
        path: impl Into<String>,
        content: impl Into<String>,
        sha: impl Into<String>,
    ) -> Self {
        Self {
            path: path.into(),
            content: content.into(),
            existing_sha: Some(sha.into()),
        }
    }
}

/// SHA of the commit created by `commit_files`.
#[derive(Debug, Clone)]
pub struct CommitSha(pub String);

impl std::fmt::Display for CommitSha {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A single code-search hit.
#[derive(Debug, Clone)]
pub struct CodeSearchHit {
    pub repo: RemoteRepo,
    pub file_path: String,
}

// ── RepoProvider trait ────────────────────────────────────────────────────────

/// Abstraction over VCS hosting providers (GitHub, GitLab, Azure DevOps).
///
/// Implementations communicate with their respective REST APIs using a
/// pre-configured access token.  All methods are async and return `VcsError`
/// on failure.
#[async_trait]
pub trait RepoProvider: Send + Sync {
    /// Provider identifier used in logs and error messages.
    fn provider_name(&self) -> &str;

    /// List all repositories visible to the authenticated user in `org`.
    /// Results are paginated internally; the full list is returned.
    async fn list_org_repos(&self, org: &str) -> Result<Vec<RemoteRepo>, VcsError>;

    /// Search for repositories that contain `query` in files matching
    /// `filename_glob` (e.g. `"Cargo.toml"`, `"package.json"`).
    ///
    /// This is the primary mechanism for discovering repos that depend on a
    /// package: search for its name inside manifest files.
    async fn search_code(
        &self,
        query: &str,
        filename_glob: &str,
    ) -> Result<Vec<CodeSearchHit>, VcsError>;

    /// Create a new branch named `branch` from `from_ref` (branch name or SHA).
    async fn create_branch(
        &self,
        repo: &RepoRef,
        branch: &str,
        from_ref: &str,
    ) -> Result<(), VcsError>;

    /// Read a file's content and blob SHA. Returns `None` if the path does not
    /// exist on the given ref.
    async fn get_file(
        &self,
        repo: &RepoRef,
        path: &str,
        git_ref: &str,
    ) -> Result<Option<(String, String)>, VcsError>; // (content, blob_sha)

    /// Commit one or more file changes to `branch` in a single operation.
    /// The provider handles the underlying tree/commit creation as needed.
    async fn commit_files(
        &self,
        repo: &RepoRef,
        branch: &str,
        files: Vec<FileCommit>,
        message: &str,
    ) -> Result<CommitSha, VcsError>;
}
