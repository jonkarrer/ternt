//! OpenAI-style chat completions: OpenAI, Baseten, Fireworks.

use super::{Api, Provider, Request, Response};
use context::{Block, Context, Message, Role};
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

    fn body(&self, req: &Request, ctx: &Context) -> Value {
        let mut messages: Vec<Value> = ctx
            .system
            .iter()
            .map(|s| json!({ "role": "system", "content": s }))
            .collect();
        for message in &ctx.messages {
            push_message(&mut messages, message);
        }
        let mut body = json!({
            "model": req.model,
            "max_tokens": req.max_tokens,
            "messages": messages,
        });
        // One knob here: thinking off is effort "none"; thinking on just uses the effort.
        if req.thinking == Some(false) {
            body["reasoning_effort"] = "none".into();
        } else if let Some(effort) = req.effort {
            body["reasoning_effort"] = json!(effort);
        }
        if !ctx.tools.is_empty() {
            body["tools"] = ctx
                .tools
                .iter()
                .map(|t| {
                    json!({ "type": "function", "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.input_schema,
                    }})
                })
                .collect();
        }
        body
    }

    fn response(&self, reply: ChatCompletion) -> Option<Response> {
        let choice = reply.choices.into_iter().next()?.message;
        let mut content = Vec::new();
        if let Some(text) = choice.reasoning_content {
            content.push(Block::Thinking {
                text,
                signature: None,
            });
        }
        if let Some(text) = choice.content.filter(|t| !t.is_empty()) {
            content.push(Block::Text { text });
        }
        for call in choice.tool_calls.unwrap_or_default() {
            content.push(Block::ToolCall {
                id: call.id,
                name: call.function.name,
                // Arguments arrive as a JSON string; keep the raw string if it isn't valid JSON.
                input: serde_json::from_str(&call.function.arguments)
                    .unwrap_or(Value::String(call.function.arguments)),
            });
        }
        Some(Response {
            message: Message {
                role: Role::Assistant,
                content,
            },
            input_tokens: reply.usage.prompt_tokens,
            output_tokens: reply.usage.completion_tokens,
        })
    }
}

/// One `Message` can become several OpenAI messages: each tool result is its own
/// `role: "tool"` message. Thinking is dropped; these APIs don't take it back.
fn push_message(out: &mut Vec<Value>, message: &Message) {
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    for block in &message.content {
        match block {
            Block::Text { text: t } => text.push_str(t),
            Block::ToolCall { id, name, input } => tool_calls.push(json!({
                "id": id,
                "type": "function",
                "function": { "name": name, "arguments": input.to_string() },
            })),
            Block::ToolResult {
                call_id, content, ..
            } => out.push(json!({
                "role": "tool",
                "tool_call_id": call_id,
                "content": content,
            })),
            Block::Thinking { .. } | Block::RedactedThinking { .. } => {}
        }
    }
    let mut msg = match message.role {
        Role::User if text.is_empty() => return,
        Role::User => json!({ "role": "user", "content": text }),
        Role::Assistant => {
            json!({ "role": "assistant", "content": (!text.is_empty()).then_some(text) })
        }
    };
    if !tool_calls.is_empty() {
        msg["tool_calls"] = tool_calls.into();
    }
    out.push(msg);
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
    use crate::providers::Effort;
    use context::Tool;

    const BASETEN_MODEL: &str = "deepseek-ai/DeepSeek-V4.1-Flash";

    fn pong_context() -> Context {
        let mut ctx = Context::new("Be terse.");
        ctx.push(Message::user("Reply with: pong"));
        ctx
    }

    fn weather_tool() -> Tool {
        Tool {
            name: "weather".into(),
            description: "Current weather for a city".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "city": { "type": "string" } },
                "required": ["city"],
            }),
        }
    }

    async fn pong(provider: Provider<OpenAi>, model: &str) {
        let res = provider
            .chat(&Request::new(model), &pong_context())
            .await
            .unwrap();
        println!("{res:?}");
        assert!(!res.message.text().is_empty());
    }

    #[test]
    fn tool_round_trip_shapes() {
        let mut ctx = Context::default();
        ctx.push(Message {
            role: Role::Assistant,
            content: vec![
                Block::Thinking {
                    text: "hmm".into(),
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
        let body = OpenAi.body(&Request::new("m"), &ctx);
        assert_eq!(
            body["messages"],
            json!([
                { "role": "assistant", "content": null, "tool_calls": [{
                    "id": "c1", "type": "function",
                    "function": { "name": "weather", "arguments": "{\"city\":\"Paris\"}" },
                }]},
                { "role": "tool", "tool_call_id": "c1", "content": "18C" },
            ])
        );
    }

    #[test]
    fn reasoning_effort() {
        let ctx = pong_context();
        let body = OpenAi.body(&Request::new("m").effort(Effort::Low), &ctx);
        assert_eq!(body["reasoning_effort"], "low");
        let body = OpenAi.body(
            &Request::new("m").thinking(false).effort(Effort::High),
            &ctx,
        );
        assert_eq!(body["reasoning_effort"], "none");
        assert!(
            OpenAi
                .body(&Request::new("m"), &ctx)
                .get("reasoning_effort")
                .is_none()
        );
    }

    #[test]
    fn system_leads_messages_and_tools_are_functions() {
        let mut ctx = pong_context();
        ctx.register(weather_tool());
        let body = OpenAi.body(&Request::new("m"), &ctx);
        assert_eq!(
            body["messages"],
            json!([
                { "role": "system", "content": "Be terse." },
                { "role": "user", "content": "Reply with: pong" }
            ])
        );
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(
            body["tools"][0]["function"]["parameters"],
            weather_tool().input_schema
        );
    }

    fn key(name: &str) -> String {
        std::env::var(name).unwrap_or_else(|_| panic!("set {name}"))
    }

    #[tokio::test]
    #[ignore = "calls Baseten"]
    async fn baseten() {
        pong(Provider::baseten(&key("BASETEN_API_KEY")), BASETEN_MODEL).await;
    }

    fn has_thinking(message: &Message) -> bool {
        message
            .content
            .iter()
            .any(|b| matches!(b, Block::Thinking { .. }))
    }

    #[tokio::test]
    #[ignore = "calls Baseten"]
    async fn baseten_thinking() {
        let provider = Provider::baseten(&key("BASETEN_API_KEY"));
        let mut ctx = Context::default();
        ctx.push(Message::user("Which is larger, 9.11 or 9.9?"));

        let on = Request::new(BASETEN_MODEL)
            .thinking(true)
            .effort(Effort::Low);
        let res = provider.chat(&on, &ctx).await.unwrap();
        println!("on: {res:?}");
        assert!(has_thinking(&res.message));

        let off = Request::new(BASETEN_MODEL).thinking(false);
        let res = provider.chat(&off, &ctx).await.unwrap();
        println!("off: {res:?}");
        assert!(!has_thinking(&res.message));
    }

    /// Model calls the tool, we answer, model replies with text: the whole loop through `Context`.
    /// Thinking comes back unsigned, so the second call drops it.
    #[tokio::test]
    #[ignore = "calls Baseten"]
    async fn baseten_tool_loop() {
        let provider = Provider::baseten(&key("BASETEN_API_KEY"));
        let req = Request::new(BASETEN_MODEL).effort(Effort::Low);
        let mut ctx = Context::new("Use the weather tool to answer.");
        ctx.register(weather_tool());
        ctx.push(Message::user("What's the weather in Paris?"));

        let res = provider.chat(&req, &ctx).await.unwrap();
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
