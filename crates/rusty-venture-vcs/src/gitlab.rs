use async_trait::async_trait;

use crate::{
    provider::{CodeSearchHit, CommitSha, FileCommit, RemoteRepo, RepoProvider, RepoRef},
    VcsError,
};

/// VCS provider backed by the GitLab REST API v4.
///
/// Supports both gitlab.com and self-hosted GitLab instances.
/// Configure `base_url` to point to `https://gitlab.com/api/v4` or your
/// self-managed instance.
pub struct GitlabProvider {
    #[allow(dead_code)]
    token: String,
    #[allow(dead_code)]
    base_url: String,
}

impl GitlabProvider {
    pub fn new(token: impl Into<String>) -> Self {
        Self::with_base_url(token, "https://gitlab.com/api/v4")
    }

    pub fn with_base_url(token: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            base_url: base_url.into(),
        }
    }
}

#[async_trait]
impl RepoProvider for GitlabProvider {
    fn provider_name(&self) -> &str {
        "gitlab"
    }

    async fn list_org_repos(&self, _org: &str) -> Result<Vec<RemoteRepo>, VcsError> {
        Err(VcsError::NotImplemented(
            "GitLab list_org_repos".to_string(),
        ))
    }

    async fn search_code(
        &self,
        _query: &str,
        _filename_glob: &str,
    ) -> Result<Vec<CodeSearchHit>, VcsError> {
        Err(VcsError::NotImplemented("GitLab search_code".to_string()))
    }

    async fn create_branch(
        &self,
        _repo: &RepoRef,
        _branch: &str,
        _from_ref: &str,
    ) -> Result<(), VcsError> {
        Err(VcsError::NotImplemented("GitLab create_branch".to_string()))
    }

    async fn get_file(
        &self,
        _repo: &RepoRef,
        _path: &str,
        _git_ref: &str,
    ) -> Result<Option<(String, String)>, VcsError> {
        Err(VcsError::NotImplemented("GitLab get_file".to_string()))
    }

    async fn commit_files(
        &self,
        _repo: &RepoRef,
        _branch: &str,
        _files: Vec<FileCommit>,
        _message: &str,
    ) -> Result<CommitSha, VcsError> {
        Err(VcsError::NotImplemented("GitLab commit_files".to_string()))
    }
}
