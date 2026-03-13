use std::sync::Arc;

use async_trait::async_trait;
use bollard::{container::RemoveContainerOptions, Docker};
use rusty_venture_core::{action::Action, context::ExecutionContext, CoreError};
use tracing::info;

use super::spawn::CTX_CONTAINER_ID;
use super::spawn::CTX_DOCKER_CLIENT;

/// Stops and removes the container identified by `CTX_CONTAINER_ID` in context.
pub struct CleanupContainerAction;

impl Default for CleanupContainerAction {
    fn default() -> Self {
        Self
    }
}

#[async_trait]
impl Action for CleanupContainerAction {
    type Input = ();
    type Output = ();

    fn name(&self) -> &str {
        "cleanup-container"
    }

    async fn execute(&self, ctx: &ExecutionContext, _input: ()) -> Result<(), CoreError> {
        let container_id: String = ctx.require::<String>(CTX_CONTAINER_ID).await?;
        let docker: Arc<Docker> = ctx.require::<Arc<Docker>>(CTX_DOCKER_CLIENT).await?;

        info!(container_id = %container_id, "Stopping container");
        let _ = docker.stop_container(&container_id, None).await;

        info!(container_id = %container_id, "Removing container");
        docker
            .remove_container(
                &container_id,
                Some(RemoveContainerOptions {
                    force: true,
                    v: true,
                    ..Default::default()
                }),
            )
            .await
            .map_err(|e| CoreError::Docker(e.to_string()))?;

        info!(container_id = %container_id, "Container cleaned up");
        Ok(())
    }
}
