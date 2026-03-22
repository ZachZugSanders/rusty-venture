use std::sync::Arc;

use async_trait::async_trait;
use bollard::{
    container::{Config, CreateContainerOptions},
    models::HostConfig,
    Docker,
};
use rusty_venture_core::{action::Action, context::ExecutionContext, CoreError};
use tracing::info;

use super::guard::ContainerGuard;

pub const CTX_CONTAINER_ID: &str = "container.id";
pub const CTX_DOCKER_CLIENT: &str = "docker.client";

/// Configuration for spawning a container.
#[derive(Debug, Clone)]
pub struct ContainerConfig {
    pub image: String,
    /// Explicit container name. Visible in `docker ps` and Docker Desktop.
    /// Convention: `rv-<role>-<short_run_id>` e.g. `rv-clone-a1b2c3d4`.
    pub container_name: Option<String>,
    /// Memory limit in MB. Default: 512.
    pub memory_limit_mb: u64,
    /// Whether to disable network. Default: false (network enabled for clone phase).
    pub network_disabled: bool,
    /// Working directory inside the container.
    pub working_dir: String,
    /// Volume bind mounts in Docker format: `"source:/dest:options"`.
    /// E.g. `"my-volume:/workspace:rw"` or `"my-volume:/workspace:ro"`.
    pub binds: Vec<String>,
}

impl ContainerConfig {
    pub fn new(image: impl Into<String>) -> Self {
        Self {
            image: image.into(),
            container_name: None,
            memory_limit_mb: 512,
            network_disabled: false,
            working_dir: "/workspace".to_string(),
            binds: vec![],
        }
    }

    /// Set the container name shown in `docker ps`.
    /// Convention: `rv-<role>-<short_run_id>` e.g. `rv-clone-a1b2c3d4`.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.container_name = Some(name.into());
        self
    }

    pub fn network_disabled(mut self) -> Self {
        self.network_disabled = true;
        self
    }

    pub fn memory_mb(mut self, mb: u64) -> Self {
        self.memory_limit_mb = mb;
        self
    }

    /// Add a bind mount. Format: `"volume_or_path:/dest:options"`.
    /// Options are typically `rw` (read-write) or `ro` (read-only).
    pub fn bind(mut self, spec: impl Into<String>) -> Self {
        self.binds.push(spec.into());
        self
    }
}

/// Spawns a Docker container and stores its ID and the Docker client in context.
pub struct SpawnContainerAction {
    config: ContainerConfig,
}

impl SpawnContainerAction {
    pub fn new(image: impl Into<String>) -> Self {
        Self {
            config: ContainerConfig::new(image),
        }
    }

    pub fn with_config(config: ContainerConfig) -> Self {
        Self { config }
    }
}

/// Output of the spawn action: the container ID.
#[derive(Debug, Clone)]
pub struct ContainerId(pub String);

#[async_trait]
impl Action for SpawnContainerAction {
    type Input = ();
    type Output = ContainerId;

    fn name(&self) -> &str {
        "spawn-container"
    }

    async fn execute(&self, ctx: &ExecutionContext, _input: ()) -> Result<ContainerId, CoreError> {
        let docker: Arc<Docker> = ctx.require::<Arc<Docker>>(CTX_DOCKER_CLIENT).await?;

        // Only pull from a registry if the image is not already present locally.
        // Locally-built runner images (e.g. rusty-venture-runner-*) are never
        // pushed to a registry, so attempting to pull them would always 404.
        let image_present = docker.inspect_image(&self.config.image).await.is_ok();

        if image_present {
            info!(image = %self.config.image, "Image already present locally, skipping pull");
        } else {
            info!(image = %self.config.image, "Pulling image from registry");

            use bollard::image::CreateImageOptions;
            use futures::StreamExt;

            let mut pull_stream = docker.create_image(
                Some(CreateImageOptions {
                    from_image: self.config.image.as_str(),
                    ..Default::default()
                }),
                None,
                None,
            );

            while let Some(result) = pull_stream.next().await {
                result.map_err(|e| CoreError::Docker(e.to_string()))?;
            }
        }

        info!(image = %self.config.image, "Creating container");

        let binds = if self.config.binds.is_empty() {
            None
        } else {
            Some(self.config.binds.clone())
        };

        let create_opts = self
            .config
            .container_name
            .as_deref()
            .map(|n| CreateContainerOptions {
                name: n,
                ..Default::default()
            });

        let container = docker
            .create_container(
                create_opts,
                Config {
                    image: Some(self.config.image.as_str()),
                    working_dir: Some(self.config.working_dir.as_str()),
                    network_disabled: Some(self.config.network_disabled),
                    tty: Some(true),
                    // Override the image's default entrypoint so the container
                    // stays alive and we can exec commands into it.
                    entrypoint: Some(vec!["/bin/sh"]),
                    cmd: Some(vec!["-c", "sleep infinity"]),
                    host_config: Some(HostConfig {
                        memory: Some((self.config.memory_limit_mb * 1024 * 1024) as i64),
                        auto_remove: Some(false),
                        binds,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| CoreError::Docker(e.to_string()))?;

        docker
            .start_container::<String>(&container.id, None)
            .await
            .map_err(|e| CoreError::Docker(e.to_string()))?;

        info!(container_id = %container.id, "Container started");

        // Register a RAII guard in context so cleanup is automatic.
        // Inserting at the same key drops the previous guard, which triggers
        // background cleanup of the prior container (used during phase swap).
        let guard = ContainerGuard::new(container.id.clone(), Arc::clone(&docker));
        ctx.insert(format!("{CTX_CONTAINER_ID}_guard"), guard).await;
        ctx.insert(CTX_CONTAINER_ID, container.id.clone()).await;

        Ok(ContainerId(container.id))
    }
}
