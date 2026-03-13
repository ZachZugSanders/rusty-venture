use async_trait::async_trait;
use tracing::{debug, instrument};

use crate::{
    connector::LlmConnector,
    types::{LlmError, LlmRequest, LlmResponse},
};

/// Calls the Anthropic Messages API using an API key.
pub struct ClaudeConnector {
    client: reqwest::Client,
    api_key: String,
    model: String,
    base_url: String,
}

impl ClaudeConnector {
    /// Create a connector using the given API key.
    /// Defaults to `claude-sonnet-4-6` model.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(180))
                .build()
                .expect("failed to build reqwest client"),
            api_key: api_key.into(),
            model: "claude-sonnet-4-6".to_string(),
            base_url: "https://api.anthropic.com".to_string(),
        }
    }

    /// Override the model (e.g. `"claude-opus-4-6"`).
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// Override the base URL (useful for testing with a mock server).
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }
}

#[async_trait]
impl LlmConnector for ClaudeConnector {
    #[instrument(skip(self, request), fields(model = %self.model))]
    async fn complete(&self, mut request: LlmRequest) -> Result<LlmResponse, LlmError> {
        request.model = self.model.clone();

        debug!(
            model = %request.model,
            max_tokens = request.max_tokens,
            num_messages = request.messages.len(),
            "Sending request to Claude API"
        );

        let resp = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await?;

        let status = resp.status();

        if !status.is_success() {
            let body: serde_json::Value = resp.json().await.unwrap_or_default();
            let message = body["error"]["message"]
                .as_str()
                .unwrap_or("unknown error")
                .to_string();
            return Err(LlmError::Api {
                status: status.as_u16(),
                message,
            });
        }

        let response: LlmResponse = resp
            .json()
            .await
            .map_err(|e| LlmError::Deserialize(e.to_string()))?;

        debug!(
            input_tokens = response.usage.input_tokens,
            output_tokens = response.usage.output_tokens,
            stop_reason = %response.stop_reason,
            "Received response from Claude API"
        );

        Ok(response)
    }

    fn model_id(&self) -> &str {
        &self.model
    }
}
