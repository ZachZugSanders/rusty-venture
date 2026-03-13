use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("Action '{action}' expected input type '{expected}' but received a different type")]
    TypeMismatch { action: String, expected: String },

    #[error("Context key '{key}' not found")]
    ContextKeyNotFound { key: String },

    #[error("Step '{step}' failed after {attempts} attempt(s): {message}")]
    StepFailed {
        step: String,
        attempts: u32,
        message: String,
    },

    #[error("Container exec returned non-zero exit code {code}: {stderr}")]
    ContainerExecFailed { code: i64, stderr: String },

    #[error("Container exec ran in detached mode unexpectedly")]
    ContainerExecDetached,

    #[error("Docker API error: {0}")]
    Docker(String),

    #[error("Git operation failed: {0}")]
    Git(String),

    #[error("LLM error: {0}")]
    Llm(String),

    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("{0}")]
    Other(String),
}

impl CoreError {
    pub fn other(msg: impl Into<String>) -> Self {
        CoreError::Other(msg.into())
    }
}
