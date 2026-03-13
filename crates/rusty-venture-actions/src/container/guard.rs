use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use bollard::Docker;
use tracing::warn;

/// RAII guard for a running Docker container. On `Drop`, spawns a background
/// task to stop and remove the container, preventing zombie containers even
/// on panic or early return.
pub struct ContainerGuard {
    pub id: String,
    docker: Arc<Docker>,
    cleaned_up: Arc<AtomicBool>,
}

impl ContainerGuard {
    pub fn new(id: String, docker: Arc<Docker>) -> Self {
        Self {
            id,
            docker,
            cleaned_up: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Explicitly clean up the container. Idempotent — safe to call multiple times.
    pub async fn cleanup(&self) {
        if self.cleaned_up.swap(true, Ordering::SeqCst) {
            return; // already cleaned up
        }
        self.do_cleanup().await;
    }

    async fn do_cleanup(&self) {
        use bollard::container::RemoveContainerOptions;

        let _ = self
            .docker
            .stop_container(&self.id, None)
            .await;

        if let Err(e) = self
            .docker
            .remove_container(
                &self.id,
                Some(RemoveContainerOptions {
                    force: true,
                    v: true,
                    ..Default::default()
                }),
            )
            .await
        {
            warn!(container_id = %self.id, error = %e, "Failed to remove container during cleanup");
        }
    }
}

impl Drop for ContainerGuard {
    fn drop(&mut self) {
        if self.cleaned_up.swap(true, Ordering::SeqCst) {
            return;
        }

        let docker = Arc::clone(&self.docker);
        let id = self.id.clone();

        tokio::spawn(async move {
            use bollard::container::RemoveContainerOptions;

            let _ = docker.stop_container(&id, None).await;
            let _ = docker
                .remove_container(
                    &id,
                    Some(RemoveContainerOptions {
                        force: true,
                        v: true,
                        ..Default::default()
                    }),
                )
                .await;
        });
    }
}
