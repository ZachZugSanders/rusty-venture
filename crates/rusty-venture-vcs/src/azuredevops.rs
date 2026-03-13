use async_trait::async_trait;

use crate::{
    provider::{CodeSearchHit, CommitSha, FileCommit, RemoteRepo, RepoRef, RepoProvider},
    VcsError,
};

/// VCS provider backed by the Azure DevOps REST API 7.1.
///
/// Authenticate with a Personal Access Token (PAT) scoped to `Code (Read & Write)`.
/// Set `organization` to your Azure DevOps org name (the part after
/// `dev.azure.com/`).
pub struct AzureDevOpsProvider {
    #[allow(dead_code)]
    token: String,
    #[allow(dead_code)]
    organization: String,
}

impl AzureDevOpsProvider {
    pub fn new(token: impl Into<String>, organization: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            organization: organization.into(),
        }
    }
}

#[async_trait]
impl RepoProvider for AzureDevOpsProvider {
    fn provider_name(&self) -> &str {
        "azuredevops"
    }

    async fn list_org_repos(&self, _project: &str) -> Result<Vec<RemoteRepo>, VcsError> {
        Err(VcsError::NotImplemented("Azure DevOps list_org_repos".to_string()))
    }

    async fn search_code(
        &self,
        _query: &str,
        _filename_glob: &str,
    ) -> Result<Vec<CodeSearchHit>, VcsError> {
        Err(VcsError::NotImplemented("Azure DevOps search_code".to_string()))
    }

    async fn create_branch(
        &self,
        _repo: &RepoRef,
        _branch: &str,
        _from_ref: &str,
    ) -> Result<(), VcsError> {
        Err(VcsError::NotImplemented("Azure DevOps create_branch".to_string()))
    }

    async fn get_file(
        &self,
        _repo: &RepoRef,
        _path: &str,
        _git_ref: &str,
    ) -> Result<Option<(String, String)>, VcsError> {
        Err(VcsError::NotImplemented("Azure DevOps get_file".to_string()))
    }

    async fn commit_files(
        &self,
        _repo: &RepoRef,
        _branch: &str,
        _files: Vec<FileCommit>,
        _message: &str,
    ) -> Result<CommitSha, VcsError> {
        Err(VcsError::NotImplemented("Azure DevOps commit_files".to_string()))
    }
}
