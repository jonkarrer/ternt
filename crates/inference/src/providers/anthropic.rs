//! Anthropic Messages API: system prompt is its own field, `max_tokens` is required,
//! and the reply is a list of content blocks.

use super::{Api, Provider, Request, Response, Stop};
use context::{Block, Context, Message, Role};
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

    fn body(&self, req: &Request, ctx: &Context) -> Value {
        let mut body = json!({
            "model": req.model,
            "max_tokens": req.max_tokens.unwrap_or(16_000),
            "messages": ctx.messages.iter().filter_map(message).collect::<Vec<_>>(),
        });
        match req.thinking {
            Some(true) => body["thinking"] = json!({ "type": "adaptive", "display": "summarized" }),
            Some(false) => body["thinking"] = json!({ "type": "disabled" }),
            None => {}
        }
        if let Some(effort) = req.effort {
            body["output_config"] = json!({ "effort": effort });
        }
        if let Some(system) = &ctx.system {
            body["system"] = system.as_str().into();
        }
        if !ctx.tools.is_empty() {
            // Anthropic's tool shape is exactly `Tool`: name, description, input_schema.
            body["tools"] = json!(ctx.tools);
        }
        body
    }

    fn response(&self, reply: MessageReply) -> Option<Response> {
        let stop = match reply.stop_reason.as_deref() {
            Some("tool_use") => Stop::ToolUse,
            Some("max_tokens") => Stop::Cutoff,
            Some("refusal") => Stop::Refused,
            _ => Stop::Done,
        };
        let content = reply
            .content
            .into_iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text, .. } => Some(Block::Text { text }),
                ContentBlock::Thinking {
                    thinking,
                    signature,
                } => Some(Block::Thinking {
                    text: thinking,
                    signature: Some(signature),
                }),
                ContentBlock::RedactedThinking { data } => Some(Block::RedactedThinking { data }),
                ContentBlock::ToolUse {
                    id, name, input, ..
                } => Some(Block::ToolCall { id, name, input }),
                // Server-tool blocks have no neutral form yet.
                ContentBlock::Other(_) => None,
            })
            .collect();
        Some(Response {
            message: Message {
                role: Role::Assistant,
                content,
            },
            input_tokens: reply.usage.input_tokens,
            output_tokens: reply.usage.output_tokens,
            stop,
        })
    }
}

/// Anthropic rejects unsigned thinking (from another vendor), blank text blocks, and messages
/// left with no content, so those are dropped.
fn message(message: &Message) -> Option<Value> {
    let content: Vec<Value> = message
        .content
        .iter()
        .filter_map(|block| match block {
            Block::Text { text } if text.trim().is_empty() => None,
            Block::Text { text } => Some(json!({ "type": "text", "text": text })),
            Block::ToolCall { id, name, input } => {
                // Anthropic takes only an object. Arguments another vendor sent as invalid JSON
                // are kept as a raw string; wrap them rather than lose them.
                let input = match input {
                    Value::Object(_) => input.clone(),
                    other => json!({ "_raw": other }),
                };
                Some(json!({ "type": "tool_use", "id": id, "name": name, "input": input }))
            }
            Block::ToolResult {
                call_id,
                content,
                is_error,
            } => Some(json!({
                "type": "tool_result",
                "tool_use_id": call_id,
                "content": content,
                "is_error": is_error,
            })),
            Block::Thinking {
                text,
                signature: Some(signature),
            } => Some(json!({ "type": "thinking", "thinking": text, "signature": signature })),
            Block::Thinking {
                signature: None, ..
            } => None,
            Block::RedactedThinking { data } => {
                Some(json!({ "type": "redacted_thinking", "data": data }))
            }
        })
        .collect();
    (!content.is_empty()).then(|| json!({ "role": message.role, "content": content }))
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
    /// Code-execution container, when one was used.
    pub container: Option<Value>,
    pub diagnostics: Option<Value>,
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
        /// Who invoked the tool, e.g. `{"type": "direct"}`.
        caller: Option<Value>,
    },
    /// Server tools, web search results, etc. Kept as raw JSON.
    #[serde(untagged)]
    Other(Value),
}

#[derive(Debug, Deserialize)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub output_tokens_details: Option<OutputTokensDetails>,
    pub cache_creation_input_tokens: Option<u32>,
    pub cache_read_input_tokens: Option<u32>,
    pub cache_creation: Option<Value>,
    pub server_tool_use: Option<Value>,
    pub service_tier: Option<String>,
    pub inference_geo: Option<String>,
    pub speed: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OutputTokensDetails {
    pub thinking_tokens: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::Effort;
    use context::Tool;

    const MODEL: &str = "claude-opus-5-5";

    fn pong_context() -> Context {
        let mut ctx = Context::new("Be terse.");
        ctx.push(Message::user("Reply with: pong"));
        ctx
    }

    #[test]
    fn system_and_tools_are_top_level() {
        let mut ctx = pong_context();
        ctx.register(Tool {
            name: "weather".into(),
            description: "Current weather".into(),
            input_schema: json!({ "type": "object" }),
        });
        let body = Anthropic.body(&Request::new(MODEL), &ctx);
        assert_eq!(body["system"], "Be terse.");
        assert_eq!(
            body["messages"],
            json!([{ "role": "user", "content": [{ "type": "text", "text": "Reply with: pong" }] }])
        );
        assert_eq!(
            body["tools"],
            json!([{ "name": "weather", "description": "Current weather", "input_schema": { "type": "object" } }])
        );
    }

    #[test]
    fn thinking_and_effort() {
        let ctx = pong_context();
        let body = Anthropic.body(
            &Request::new(MODEL).thinking(true).effort(Effort::XHigh),
            &ctx,
        );
        assert_eq!(
            body["thinking"],
            json!({ "type": "adaptive", "display": "summarized" })
        );
        assert_eq!(body["output_config"], json!({ "effort": "xhigh" }));

        let body = Anthropic.body(&Request::new(MODEL).thinking(false), &ctx);
        assert_eq!(body["thinking"], json!({ "type": "disabled" }));
        assert!(body.get("output_config").is_none());

        let body = Anthropic.body(&Request::new(MODEL), &ctx);
        assert!(body.get("thinking").is_none());
    }

    #[test]
    fn tool_round_trip_shapes() {
        let mut ctx = Context::default();
        ctx.push(Message {
            role: Role::Assistant,
            content: vec![
                Block::Thinking {
                    text: "signed".into(),
                    signature: Some("sig".into()),
                },
                Block::Thinking {
                    text: "from another vendor".into(),
                    signature: None,
                },
                Block::ToolCall {
                    id: "c1".into(),
                    name: "weather".into(),
                    input: json!({ "city": "Paris" }),
                },
            ],
        });
        ctx.push(Message::tool_result("c1", "18C"));
        let body = Anthropic.body(&Request::new(MODEL), &ctx);
        assert_eq!(
            body["messages"],
            json!([
                { "role": "assistant", "content": [
                    { "type": "thinking", "thinking": "signed", "signature": "sig" },
                    { "type": "tool_use", "id": "c1", "name": "weather", "input": { "city": "Paris" } },
                ]},
                { "role": "user", "content": [
                    { "type": "tool_result", "tool_use_id": "c1", "content": "18C", "is_error": false },
                ]},
            ])
        );
    }

    /// Tool arguments another vendor sent as invalid JSON are kept as a raw string; Anthropic
    /// takes only an object, so they go wrapped.
    #[test]
    fn non_object_tool_input_is_wrapped() {
        let mut ctx = Context::default();
        ctx.push(Message {
            role: Role::Assistant,
            content: vec![Block::ToolCall {
                id: "c1".into(),
                name: "weather".into(),
                input: Value::String("{\"city\": \"Par".into()),
            }],
        });
        let body = Anthropic.body(&Request::new(MODEL), &ctx);
        assert_eq!(
            body["messages"][0]["content"][0]["input"],
            json!({ "_raw": "{\"city\": \"Par" })
        );
    }

    /// Anthropic rejects empty text blocks and messages with no content: a turn from another
    /// vendor that held only unsigned thinking, or an empty text, is left out.
    #[test]
    fn skips_empty_blocks_and_messages() {
        let mut ctx = Context::default();
        ctx.push(Message::user("Hi"));
        ctx.push(Message {
            role: Role::Assistant,
            content: vec![Block::Thinking {
                text: "unsigned".into(),
                signature: None,
            }],
        });
        ctx.push(Message::user(""));
        ctx.push(Message {
            role: Role::User,
            content: vec![
                Block::Text { text: " ".into() },
                Block::Text {
                    text: "Still there?".into(),
                },
            ],
        });
        let body = Anthropic.body(&Request::new(MODEL), &ctx);
        assert_eq!(
            body["messages"],
            json!([
                { "role": "user", "content": [{ "type": "text", "text": "Hi" }] },
                { "role": "user", "content": [{ "type": "text", "text": "Still there?" }] },
            ])
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
        assert_eq!(Anthropic.response(reply).unwrap().message.text(), "pong");
    }

    fn provider() -> Provider<Anthropic> {
        Provider::anthropic(&std::env::var("ANTHROPIC_API_KEY").expect("set ANTHROPIC_API_KEY"))
    }

    #[tokio::test]
    #[ignore = "calls Anthropic"]
    async fn anthropic() {
        let res = provider()
            .chat(&Request::new(MODEL), &pong_context())
            .await
            .unwrap();
        println!("{res:?}");
        assert!(!res.message.text().is_empty());
    }

    /// Model calls the tool, we answer, model replies with text: the whole loop through `Context`.
    #[tokio::test]
    #[ignore = "calls Anthropic"]
    async fn anthropic_tool_loop() {
        let provider = provider();
        // Thinking shown, so the second call sends signed thinking with text back.
        let req = Request::new(MODEL).thinking(true).effort(Effort::High);
        let mut ctx = Context::new("Use the weather tool to answer.");
        ctx.register(Tool {
            name: "weather".into(),
            description: "Current weather for a city".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "city": { "type": "string" } },
                "required": ["city"],
            }),
        });
        ctx.push(Message::user(
            "What is the weather in the capital of the country that won the 1998 FIFA World Cup? Work out which city first.",
        ));

        let res = provider.chat(&req, &ctx).await.unwrap();
        assert!(res.message.content.iter().any(|b| matches!(
            b,
            Block::Thinking {
                signature: Some(_),
                ..
            }
        )));
        let (id, name, input) = res.message.tool_calls().next().expect("a tool call");
        assert_eq!(name, "weather");
        println!("call: {input}");
        let id = id.to_string();
        ctx.push(res.message);
        ctx.push(Message::tool_result(id, "18C and sunny"));

        let res = provider.chat(&req, &ctx).await.unwrap();
        println!("reply: {}", res.message.text());
        assert!(res.message.text().contains("18"));
    }
}
