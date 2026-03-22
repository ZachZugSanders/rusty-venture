use crate::types::{LlmError, LlmRequest, LlmResponse};
use async_trait::async_trait;

/// Abstraction over any LLM backend.
/// Implementations provide message-based request/response.
#[async_trait]
pub trait LlmConnector: Send + Sync {
    /// Send a request and receive a complete response.
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse, LlmError>;

    /// Return the model identifier string (e.g. `"claude-sonnet-4-6"`).
    fn model_id(&self) -> &str;
}
