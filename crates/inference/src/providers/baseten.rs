//! Response types for Baseten's Model APIs (OpenAI-compatible).

use serde::Deserialize;

/// `POST /v1/chat/completions`
#[derive(Debug, Deserialize)]
pub struct ChatCompletion {
    pub id: String,
    pub model: String,
    pub created: u64,
    pub choices: Vec<Choice>,
    pub usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]
pub struct Choice {
    pub index: u32,
    pub message: Message,
    /// "stop", "length", "tool_calls", ...
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: Option<String>,
    /// The model's thinking, for reasoning models.
    pub reasoning_content: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    pub prompt_tokens_details: Option<PromptTokensDetails>,
    pub completion_tokens_details: Option<CompletionTokensDetails>,
}

#[derive(Debug, Deserialize)]
pub struct PromptTokensDetails {
    pub audio_tokens: u32,
    pub image_tokens: u32,
    pub cached_tokens: u32,
}

#[derive(Debug, Deserialize)]
pub struct CompletionTokensDetails {
    pub audio_tokens: u32,
    pub reasoning_tokens: u32,
}

/// Body of a non-2xx response, e.g. `{"error": "...deprecated."}`.
#[derive(Debug, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}
