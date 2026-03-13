use bollard::exec::{CreateExecOptions, StartExecResults};
use bollard::Docker;
use futures::StreamExt;
use bollard::container::LogOutput;
use rusty_venture_core::CoreError;

/// A command to execute inside a container.
#[derive(Debug, Clone)]
pub struct ExecCommand {
    /// The command and its arguments as a Vec (never a shell string — no injection risk).
    pub cmd: Vec<String>,
    pub working_dir: Option<String>,
    pub env: Vec<String>,
    pub timeout_secs: Option<u64>,
}

impl ExecCommand {
    pub fn new(cmd: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            cmd: cmd.into_iter().map(Into::into).collect(),
            working_dir: None,
            env: vec![],
            timeout_secs: Some(60),
        }
    }

    pub fn working_dir(mut self, dir: impl Into<String>) -> Self {
        self.working_dir = Some(dir.into());
        self
    }

    pub fn env(mut self, key: &str, value: &str) -> Self {
        self.env.push(format!("{key}={value}"));
        self
    }

    pub fn timeout(mut self, secs: u64) -> Self {
        self.timeout_secs = Some(secs);
        self
    }
}

/// The result of executing a command inside a container.
#[derive(Debug, Clone)]
pub struct ExecResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i64,
}

impl ExecResult {
    pub fn is_success(&self) -> bool {
        self.exit_code == 0
    }

    pub fn into_core_error(self) -> CoreError {
        CoreError::ContainerExecFailed {
            code: self.exit_code,
            stderr: self.stderr,
        }
    }
}

/// Execute a command inside a running Docker container and return its output.
/// Commands are passed as `Vec<String>` (bollard exec takes an argv array,
/// never a shell string), preventing shell injection.
pub async fn exec_in_container(
    docker: &Docker,
    container_id: &str,
    cmd: ExecCommand,
) -> Result<ExecResult, CoreError> {
    let env_refs: Vec<&str> = cmd.env.iter().map(String::as_str).collect();

    let exec = docker
        .create_exec(
            container_id,
            CreateExecOptions {
                cmd: Some(cmd.cmd.iter().map(String::as_str).collect()),
                working_dir: cmd.working_dir.as_deref(),
                env: if env_refs.is_empty() { None } else { Some(env_refs) },
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                ..Default::default()
            },
        )
        .await
        .map_err(|e| CoreError::Docker(e.to_string()))?;

    let start_result = docker
        .start_exec(&exec.id, None)
        .await
        .map_err(|e| CoreError::Docker(e.to_string()))?;

    let mut output = match start_result {
        StartExecResults::Attached { output, .. } => output,
        StartExecResults::Detached => return Err(CoreError::ContainerExecDetached),
    };

    let mut stdout_buf: Vec<u8> = Vec::new();
    let mut stderr_buf: Vec<u8> = Vec::new();

    while let Some(msg) = output.next().await {
        match msg.map_err(|e| CoreError::Docker(e.to_string()))? {
            LogOutput::StdOut { message } => stdout_buf.extend_from_slice(&message),
            LogOutput::StdErr { message } => stderr_buf.extend_from_slice(&message),
            _ => {}
        }
    }

    let inspect = docker
        .inspect_exec(&exec.id)
        .await
        .map_err(|e| CoreError::Docker(e.to_string()))?;

    Ok(ExecResult {
        stdout: String::from_utf8_lossy(&stdout_buf).into_owned(),
        stderr: String::from_utf8_lossy(&stderr_buf).into_owned(),
        exit_code: inspect.exit_code.unwrap_or(-1),
    })
}
