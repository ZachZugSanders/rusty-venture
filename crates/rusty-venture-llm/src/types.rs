use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Role of a message in a conversation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

/// A single message in a conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: Role::User, content: content.into() }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self { role: Role::Assistant, content: content.into() }
    }
}

/// Maps directly to the Anthropic Messages API request body.
#[derive(Debug, Clone, Serialize)]
pub struct LlmRequest {
    pub model: String,
    pub max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    pub messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
}

/// Top-level response from the Anthropic Messages API.
#[derive(Debug, Deserialize)]
pub struct LlmResponse {
    pub id: String,
    pub model: String,
    pub content: Vec<ContentBlock>,
    pub stop_reason: String,
    pub usage: Usage,
}

impl LlmResponse {
    /// Extract the text content from the first text block.
    pub fn text(&self) -> Option<&str> {
        self.content.iter().find_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
        })
    }

    /// Like `text()` but returns an empty string if no text block exists.
    pub fn text_or_empty(&self) -> &str {
        self.text().unwrap_or("")
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
}

#[derive(Debug, Deserialize)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// Errors that can occur in the LLM layer.
#[derive(Debug, Error)]
pub enum LlmError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("API error {status}: {message}")]
    Api { status: u16, message: String },

    #[error("Deserialization failed: {0}")]
    Deserialize(String),

    #[error("Response contained no text content")]
    EmptyResponse,
}

/// Builder for constructing `LlmRequest` values ergonomically.
#[derive(Default)]
pub struct LlmRequestBuilder {
    system: Option<String>,
    messages: Vec<Message>,
    max_tokens: u32,
    temperature: Option<f32>,
}

impl LlmRequestBuilder {
    pub fn new() -> Self {
        Self {
            max_tokens: 4096,
            ..Default::default()
        }
    }

    pub fn system(mut self, prompt: impl Into<String>) -> Self {
        self.system = Some(prompt.into());
        self
    }

    pub fn user(mut self, content: impl Into<String>) -> Self {
        self.messages.push(Message::user(content));
        self
    }

    pub fn assistant(mut self, content: impl Into<String>) -> Self {
        self.messages.push(Message::assistant(content));
        self
    }

    pub fn max_tokens(mut self, n: u32) -> Self {
        self.max_tokens = n;
        self
    }

    pub fn temperature(mut self, t: f32) -> Self {
        self.temperature = Some(t);
        self
    }

    /// Build the request. `model` is set by `ClaudeConnector` at send time.
    pub fn build(self) -> LlmRequest {
        LlmRequest {
            model: String::new(), // filled in by connector
            max_tokens: self.max_tokens,
            system: self.system,
            messages: self.messages,
            temperature: self.temperature,
        }
    }
}
