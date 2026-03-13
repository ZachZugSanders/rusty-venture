use std::sync::Arc;

use async_trait::async_trait;
use bollard::Docker;
use rusty_venture_core::{action::Action, context::ExecutionContext, CoreError};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::container::{exec_in_container, ExecCommand, CTX_CONTAINER_ID, CTX_DOCKER_CLIENT};

pub const CTX_REPO_LOCAL_PATH: &str = "repo.local_path";
pub const CTX_REPO_URL: &str = "repo.url";

/// Result of cloning a repository into the container.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloneResult {
    pub path: String,
    pub repo_url: String,
    pub branch: Option<String>,
}

/// Clones a git repository into `/workspace/repo` inside the container.
/// The container must have `git` available (use `alpine/git` or `ubuntu:22.04`
/// with git pre-installed).
pub struct CloneRepoAction {
    repo_url: String,
    branch: Option<String>,
    dest_path: String,
}

impl CloneRepoAction {
    pub fn new(repo_url: impl Into<String>) -> Self {
        Self {
            repo_url: repo_url.into(),
            branch: None,
            dest_path: "/workspace/repo".to_string(),
        }
    }

    pub fn branch(mut self, branch: impl Into<String>) -> Self {
        self.branch = Some(branch.into());
        self
    }

    pub fn dest_path(mut self, path: impl Into<String>) -> Self {
        self.dest_path = path.into();
        self
    }
}

#[async_trait]
impl Action for CloneRepoAction {
    type Input = ();
    type Output = CloneResult;

    fn name(&self) -> &str {
        "clone-repo"
    }

    async fn execute(&self, ctx: &ExecutionContext, _input: ()) -> Result<CloneResult, CoreError> {
        let container_id: String = ctx.require::<String>(CTX_CONTAINER_ID).await?;
        let docker: Arc<Docker> = ctx.require::<Arc<Docker>>(CTX_DOCKER_CLIENT).await?;

        // Ensure /workspace exists
        exec_in_container(
            &docker,
            &container_id,
            ExecCommand::new(["mkdir", "-p", "/workspace"]),
        )
        .await?;

        // Install git if not present (for ubuntu:22.04 base images)
        let git_check = exec_in_container(
            &docker,
            &container_id,
            ExecCommand::new(["which", "git"]),
        )
        .await?;

        if !git_check.is_success() {
            info!("Installing git in container");
            let install = exec_in_container(
                &docker,
                &container_id,
                ExecCommand::new(["apt-get", "update", "-qq"])
                    .timeout(120),
            )
            .await?;
            if !install.is_success() {
                return Err(CoreError::Git("Failed to run apt-get update".into()));
            }
            let install = exec_in_container(
                &docker,
                &container_id,
                ExecCommand::new(["apt-get", "install", "-y", "-qq", "git"])
                    .timeout(120),
            )
            .await?;
            if !install.is_success() {
                return Err(CoreError::Git("Failed to install git".into()));
            }
        }

        // Build the clone command
        let mut clone_cmd = vec!["git".to_string(), "clone".to_string(), "--depth".to_string(), "1".to_string()];

        if let Some(ref branch) = self.branch {
            clone_cmd.push("--branch".to_string());
            clone_cmd.push(branch.clone());
        }

        clone_cmd.push(self.repo_url.clone());
        clone_cmd.push(self.dest_path.clone());

        info!(repo_url = %self.repo_url, dest = %self.dest_path, "Cloning repository");

        let result = exec_in_container(
            &docker,
            &container_id,
            ExecCommand::new(clone_cmd).timeout(300),
        )
        .await?;

        if !result.is_success() {
            return Err(CoreError::Git(format!(
                "git clone failed (exit {}): {}",
                result.exit_code, result.stderr
            )));
        }

        info!(path = %self.dest_path, "Repository cloned successfully");

        let clone_result = CloneResult {
            path: self.dest_path.clone(),
            repo_url: self.repo_url.clone(),
            branch: self.branch.clone(),
        };

        ctx.insert(CTX_REPO_LOCAL_PATH, self.dest_path.clone()).await;
        ctx.insert(CTX_REPO_URL, self.repo_url.clone()).await;

        Ok(clone_result)
    }
}
