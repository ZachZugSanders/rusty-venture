use std::sync::Arc;

use async_trait::async_trait;
use bollard::{image::CommitContainerOptions, Docker};
use rusty_venture_core::{action::Action, context::ExecutionContext, CoreError};
use tracing::info;

use super::spawn::{CTX_CONTAINER_ID, CTX_DOCKER_CLIENT};

/// Context key storing the name of the cached image produced by this action.
pub const CTX_CACHE_IMAGE_NAME: &str = "container.cache_image_name";

/// Image name prefix that distinguishes cached repo images from runner images.
/// Convention: `rv-cache-{owner}-{repo}:latest`
///
/// Examples:
///   `rv-cache-zachzugsanders-rusty-venture:latest`
///   `rv-cache-torvalds-linux:latest`
const CACHE_IMAGE_PREFIX: &str = "rv-cache";

/// Derive a stable Docker image name from a GitHub repository URL.
///
/// `https://github.com/Owner/Repo` → `("rv-cache-owner-repo", "latest")`
fn repo_url_to_image_parts(repo_url: &str) -> (String, String) {
    let slug = repo_url
        .trim_start_matches("https://github.com/")
        .trim_end_matches(".git")
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>();

    (format!("{CACHE_IMAGE_PREFIX}-{slug}"), "latest".to_string())
}

/// Commits the currently-active clone container (which holds the checked-out
/// repository) as a named local Docker image.
///
/// This creates a reusable snapshot under `rv-cache-{owner}-{repo}:latest` so
/// future actions can start from a pre-cloned state without repeating the
/// network clone. The image is stored only in the local Docker daemon — it is
/// never pushed to a registry.
///
/// **When to run:** insert this step immediately after `clone-repo` and before
/// `spawn-analysis-container` so the clone container is still alive and its ID
/// is still the active `CTX_CONTAINER_ID`.
pub struct CacheRepoImageAction {
    repo_url: String,
}

impl CacheRepoImageAction {
    pub fn new(repo_url: impl Into<String>) -> Self {
        Self {
            repo_url: repo_url.into(),
        }
    }

    /// Return the image name that *would* be created for the given URL.
    /// Useful for callers that want to reference the image before running.
    pub fn image_name_for(repo_url: &str) -> String {
        let (repo, tag) = repo_url_to_image_parts(repo_url);
        format!("{repo}:{tag}")
    }
}

#[async_trait]
impl Action for CacheRepoImageAction {
    type Input = ();
    type Output = String; // the resulting image name

    fn name(&self) -> &str {
        "cache-repo-image"
    }

    async fn execute(&self, ctx: &ExecutionContext, _input: ()) -> Result<String, CoreError> {
        let docker: Arc<Docker> = ctx.require::<Arc<Docker>>(CTX_DOCKER_CLIENT).await?;
        let container_id: String = ctx.require::<String>(CTX_CONTAINER_ID).await?;

        let (repo, tag) = repo_url_to_image_parts(&self.repo_url);
        let image_name = format!("{repo}:{tag}");

        info!(
            container_id = %container_id,
            image = %image_name,
            "Committing clone container as cached repository image"
        );
        ctx.emit_log(
            "info",
            Some("cache-repo-image"),
            format!("Caching repository snapshot → {image_name}"),
        );

        docker
            .commit_container(
                CommitContainerOptions {
                    container: container_id.clone(),
                    repo: repo.clone(),
                    tag: tag.clone(),
                    pause: true,
                    ..Default::default()
                },
                bollard::container::Config::<String>::default(),
            )
            .await
            .map_err(|e| CoreError::Docker(format!("docker commit failed: {e}")))?;

        info!(image = %image_name, "Repository snapshot cached as local Docker image");
        ctx.emit_log(
            "info",
            Some("cache-repo-image"),
            format!("Snapshot cached as '{image_name}' — ready for future actions"),
        );

        ctx.insert(CTX_CACHE_IMAGE_NAME, image_name.clone()).await;

        Ok(image_name)
    }
}
