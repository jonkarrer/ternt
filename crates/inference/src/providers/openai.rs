//! OpenAI-style chat completions: OpenAI, Baseten, Fireworks.

use super::{Api, Provider, Request, Response};
use serde::Deserialize;
use serde_json::{Value, json};

pub struct OpenAi;

impl Provider<OpenAi> {
    pub fn openai(key: &str) -> Self {
        Self::new(
            OpenAi,
            "OpenAI",
            "https://api.openai.com/v1/chat/completions",
            bearer(key),
        )
    }

    pub fn baseten(key: &str) -> Self {
        let auth = vec![("Authorization", format!("Api-Key {key}"))];
        Self::new(
            OpenAi,
            "Baseten",
            "https://inference.baseten.co/v1/chat/completions",
            auth,
        )
    }

    pub fn fireworks(key: &str) -> Self {
        Self::new(
            OpenAi,
            "Fireworks",
            "https://api.fireworks.ai/inference/v1/chat/completions",
            bearer(key),
        )
    }
}

fn bearer(key: &str) -> Vec<(&'static str, String)> {
    vec![("Authorization", format!("Bearer {key}"))]
}

impl Api for OpenAi {
    type Reply = ChatCompletion;

    fn body(&self, req: &Request) -> Value {
        json!({ "model": req.model, "messages": req.messages, "max_tokens": req.max_tokens })
    }

    fn response(&self, reply: ChatCompletion) -> Option<Response> {
        Some(Response {
            text: reply.choices.into_iter().next()?.message.content?,
            input_tokens: reply.usage.prompt_tokens,
            output_tokens: reply.usage.completion_tokens,
        })
    }
}

/// Full chat completion. Fields some vendors leave out are `Option`.
#[derive(Debug, Deserialize)]
pub struct ChatCompletion {
    pub id: String,
    /// "chat.completion"
    pub object: String,
    pub created: u64,
    pub model: String,
    pub choices: Vec<Choice>,
    pub usage: Usage,
    pub service_tier: Option<String>,
    pub system_fingerprint: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Choice {
    pub index: u32,
    pub message: ChoiceMessage,
    /// "stop", "length", "tool_calls", "content_filter"
    pub finish_reason: Option<String>,
    pub logprobs: Option<Value>,
    /// Fireworks only.
    pub raw_output: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub struct ChoiceMessage {
    pub role: String,
    pub content: Option<String>,
    /// The model's thinking, for reasoning models (Baseten, Fireworks).
    pub reasoning_content: Option<String>,
    pub refusal: Option<String>,
    pub tool_calls: Option<Vec<ToolCall>>,
}

#[derive(Debug, Deserialize)]
pub struct ToolCall {
    pub id: String,
    /// "function"
    #[serde(rename = "type")]
    pub kind: String,
    pub function: FunctionCall,
}

#[derive(Debug, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    /// JSON-encoded arguments.
    pub arguments: String,
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
    pub audio_tokens: Option<u32>,
    pub image_tokens: Option<u32>,
    pub cached_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct CompletionTokensDetails {
    pub accepted_prediction_tokens: Option<u32>,
    pub audio_tokens: Option<u32>,
    pub reasoning_tokens: Option<u32>,
    pub rejected_prediction_tokens: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::{Message, Role};

    async fn pong(provider: Provider<OpenAi>, model: &str) {
        let req = Request {
            model: model.into(),
            messages: vec![Message::new(Role::User, "Reply with: pong")],
            max_tokens: None,
        };
        let res = provider.chat(&req).await.unwrap();
        println!("{res:?}");
        assert!(!res.text.is_empty());
    }

    fn key(name: &str) -> String {
        std::env::var(name).unwrap_or_else(|_| panic!("set {name}"))
    }

    #[tokio::test]
    #[ignore = "calls Baseten"]
    async fn baseten() {
        pong(
            Provider::baseten(&key("BASETEN_API_KEY")),
            "deepseek-ai/DeepSeek-V4.1-Flash",
        )
        .await;
    }

    #[tokio::test]
    #[ignore = "calls Fireworks"]
    async fn fireworks() {
        pong(
            Provider::fireworks(&key("FIREWORKS_API_KEY")),
            "accounts/fireworks/models/deepseek-v4p1-flash",
        )
        .await;
    }
}
