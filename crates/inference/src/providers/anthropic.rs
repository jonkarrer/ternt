//! Anthropic Messages API: system prompt is its own field, `max_tokens` is required,
//! and the reply is a list of content blocks.

use super::{Api, Provider, Request, Response, Role};
use serde::Deserialize;
use serde_json::{Value, json};

pub struct Anthropic;

impl Provider<Anthropic> {
    pub fn anthropic(key: &str) -> Self {
        let headers = vec![
            ("x-api-key", key.into()),
            ("anthropic-version", "2023-06-01".into()),
        ];
        Self::new(
            Anthropic,
            "Anthropic",
            "https://api.anthropic.com/v1/messages",
            headers,
        )
    }
}

impl Api for Anthropic {
    type Reply = MessageReply;

    fn body(&self, req: &Request) -> Value {
        let (system, messages): (Vec<_>, Vec<_>) =
            req.messages.iter().partition(|m| m.role == Role::System);
        let mut body = json!({
            "model": req.model,
            "max_tokens": req.max_tokens.unwrap_or(16_000),
            "messages": messages,
        });
        if !system.is_empty() {
            let system: Vec<&str> = system.iter().map(|m| m.content.as_str()).collect();
            body["system"] = system.join("\n\n").into();
        }
        body
    }

    fn response(&self, reply: MessageReply) -> Option<Response> {
        Some(Response {
            text: reply
                .content
                .into_iter()
                .filter_map(ContentBlock::text)
                .collect(),
            input_tokens: reply.usage.input_tokens,
            output_tokens: reply.usage.output_tokens,
        })
    }
}

/// Full Messages API reply.
#[derive(Debug, Deserialize)]
pub struct MessageReply {
    pub id: String,
    /// "message"
    #[serde(rename = "type")]
    pub kind: String,
    /// "assistant"
    pub role: String,
    pub model: String,
    pub content: Vec<ContentBlock>,
    /// "end_turn", "max_tokens", "stop_sequence", "tool_use", "pause_turn", "refusal"
    pub stop_reason: Option<String>,
    pub stop_sequence: Option<String>,
    /// Set only when `stop_reason` is "refusal".
    pub stop_details: Option<Value>,
    pub usage: Usage,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
        citations: Option<Vec<Value>>,
    },
    Thinking {
        thinking: String,
        signature: String,
    },
    RedactedThinking {
        data: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    /// Server tools, web search results, etc. Kept as raw JSON.
    #[serde(untagged)]
    Other(Value),
}

impl ContentBlock {
    pub fn text(self) -> Option<String> {
        match self {
            ContentBlock::Text { text, .. } => Some(text),
            _ => None,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_creation_input_tokens: Option<u32>,
    pub cache_read_input_tokens: Option<u32>,
    pub cache_creation: Option<Value>,
    pub server_tool_use: Option<Value>,
    pub service_tier: Option<String>,
    pub inference_geo: Option<String>,
    pub speed: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::Message;

    fn pong() -> Request {
        Request {
            model: "claude-opus-5-5".into(),
            messages: vec![
                Message::new(Role::System, "Be terse."),
                Message::new(Role::User, "Reply with: pong"),
            ],
            max_tokens: None,
        }
    }

    #[test]
    fn system_moves_out_of_messages() {
        let body = Anthropic.body(&pong());
        assert_eq!(body["system"], "Be terse.");
        assert_eq!(
            body["messages"],
            json!([{ "role": "user", "content": "Reply with: pong" }])
        );
    }

    #[test]
    fn parses_full_reply() {
        let reply: MessageReply = serde_json::from_value(json!({
            "id": "msg_1", "type": "message", "role": "assistant", "model": "claude-opus-5-5",
            "content": [
                { "type": "thinking", "thinking": "", "signature": "sig" },
                { "type": "text", "text": "po" },
                { "type": "web_search_tool_result", "tool_use_id": "t1", "content": [] },
                { "type": "text", "text": "ng" }
            ],
            "stop_reason": "end_turn", "stop_sequence": null,
            "usage": { "input_tokens": 10, "output_tokens": 2, "cache_read_input_tokens": 0 }
        }))
        .unwrap();
        assert!(matches!(reply.content[2], ContentBlock::Other(_)));
        assert_eq!(Anthropic.response(reply).unwrap().text, "pong");
    }

    #[tokio::test]
    #[ignore = "calls Anthropic"]
    async fn anthropic() {
        let Ok(key) = std::env::var("ANTHROPIC_API_KEY") else {
            return eprintln!("skipping: ANTHROPIC_API_KEY not set");
        };
        let res = Provider::anthropic(&key).chat(&pong()).await.unwrap();
        println!("{res:?}");
        assert!(!res.text.is_empty());
    }
}
