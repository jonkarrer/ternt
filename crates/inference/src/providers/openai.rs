//! OpenAI-style chat completions: OpenAI, Baseten, Fireworks.

use super::{Api, Assembler, Delta, Provider, Request, Response, Stop};
use anyhow::{Result, bail, ensure};
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
            "messages": messages,
        });
        // Only when set: newer OpenAI reasoning models reject the key (they take
        // `max_completion_tokens`), and leaving it out means the model's own limit.
        if let Some(max_tokens) = req.max_tokens {
            body["max_tokens"] = max_tokens.into();
        }
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
        let choice = reply.choices.into_iter().next()?;
        let stop = match choice.finish_reason.as_deref() {
            Some("tool_calls") => Stop::ToolUse,
            Some("length") => Stop::Cutoff,
            Some("content_filter") => Stop::Refused,
            _ if choice.message.refusal.is_some() => Stop::Refused,
            _ => Stop::Done,
        };
        let choice = choice.message;
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
            stop,
        })
    }

    fn stream(&self) -> Option<Box<dyn Assembler>> {
        Some(Box::<Stream>::default())
    }
}

/// A streamed completion, put back together: every chunk repeats the completion's ID and
/// model and brings a piece of the one choice (text, thinking, or tool calls by index); the
/// chunk that ends it has the `finish_reason`, and that one or a last one with no choices
/// the usage.
#[derive(Default)]
struct Stream {
    /// The first chunk: the completion's ID, creation time and model.
    first: Option<Value>,
    content: String,
    reasoning: String,
    refusal: String,
    calls: Vec<Value>,
    finish: Option<String>,
    usage: Value,
}

impl Assembler for Stream {
    fn ask(&self, body: &mut Value) {
        body["stream"] = true.into();
        // Without it OpenAI and Baseten leave the token counts out of a stream.
        body["stream_options"] = json!({ "include_usage": true });
    }

    fn event(&mut self, chunk: &Value) -> Result<Option<Delta>> {
        // A failure after the stream began comes as an event, the status long sent.
        let error = &chunk["error"];
        if !error.is_null() {
            bail!(
                "{}",
                error["message"].as_str().unwrap_or(&error.to_string())
            );
        }
        if self.first.is_none() {
            self.first = Some(chunk.clone());
        }
        if !chunk["usage"].is_null() {
            self.usage = chunk["usage"].clone();
        }
        let choice = &chunk["choices"][0];
        if let Some(finish) = choice["finish_reason"].as_str() {
            self.finish = Some(finish.to_string());
        }
        let delta = &choice["delta"];
        let piece = |field: &str| delta[field].as_str().unwrap_or_default();
        for call in delta["tool_calls"].as_array().into_iter().flatten() {
            let index = call["index"].as_u64().unwrap_or(self.calls.len() as u64) as usize;
            ensure!(index <= self.calls.len(), "tool calls out of order");
            if index == self.calls.len() {
                self.calls.push(json!({
                    "id": "", "type": "function", "function": { "name": "", "arguments": "" },
                }));
            }
            // The ID and name come once, the arguments as pieces of JSON text.
            let (so_far, function) = (&mut self.calls[index], &call["function"]);
            if let Some(id) = call["id"].as_str().filter(|id| !id.is_empty()) {
                so_far["id"] = id.into();
            }
            if let Some(name) = function["name"].as_str().filter(|name| !name.is_empty()) {
                so_far["function"]["name"] = name.into();
            }
            let arguments = so_far["function"]["arguments"].as_str().unwrap_or_default();
            let more = function["arguments"].as_str().unwrap_or_default();
            so_far["function"]["arguments"] = format!("{arguments}{more}").into();
        }
        self.refusal.push_str(piece("refusal"));
        // Thinking is `reasoning_content` on Baseten and Fireworks, `reasoning` on some others.
        let thinking = match piece("reasoning_content") {
            "" => piece("reasoning"),
            thinking => thinking,
        };
        self.reasoning.push_str(thinking);
        self.content.push_str(piece("content"));
        Ok(match (piece("content"), thinking) {
            ("", "") => None,
            ("", thinking) => Some(Delta::Thinking(thinking.into())),
            (text, _) => Some(Delta::Text(text.into())),
        })
    }

    fn finish(self: Box<Self>) -> Result<Value> {
        ensure!(
            self.finish.is_some(),
            "the stream ended before the reply did"
        );
        let first = self.first.unwrap_or_default();
        let filled = |text: String| (!text.is_empty()).then_some(text);
        let usage = match self.usage {
            Value::Null => json!({ "prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0 }),
            usage => usage,
        };
        Ok(json!({
            "id": first["id"].as_str().unwrap_or_default(),
            "object": "chat.completion",
            "created": first["created"].as_u64().unwrap_or_default(),
            "model": first["model"].as_str().unwrap_or_default(),
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": filled(self.content),
                    "reasoning_content": filled(self.reasoning),
                    "refusal": filled(self.refusal),
                    "tool_calls": (!self.calls.is_empty()).then_some(self.calls),
                },
                "finish_reason": self.finish,
            }],
            "usage": usage,
        }))
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
                "function": { "name": name, "arguments": arguments(input) },
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
    if text.is_empty() && tool_calls.is_empty() {
        return;
    }
    let mut msg = match message.role {
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

/// Tool-call arguments as a JSON string. Arguments that weren't valid JSON were kept as the
/// raw string (see `response`) and go back unchanged.
fn arguments(input: &Value) -> String {
    match input {
        Value::String(raw) => raw.clone(),
        input => input.to_string(),
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

    /// What `Provider::stream` does with a body of server-sent events.
    fn streamed(body: &str) -> (Result<Response>, Vec<Delta>) {
        let mut stream = OpenAi.stream().unwrap();
        let mut deltas = Vec::new();
        for event in crate::providers::Sse::default().feed(body.as_bytes()) {
            match stream.event(&event) {
                Ok(delta) => deltas.extend(delta),
                Err(error) => return (Err(error), deltas),
            }
        }
        let read = stream.finish().and_then(|reply| {
            let reply: ChatCompletion = serde_json::from_value(reply)?;
            Ok(OpenAi.response(reply).unwrap())
        });
        (read, deltas)
    }

    #[test]
    fn a_stream_asks_for_its_usage() {
        let mut body = json!({ "model": "m" });
        OpenAi.stream().unwrap().ask(&mut body);
        assert_eq!(body["stream"], true);
        assert_eq!(body["stream_options"]["include_usage"], true);
    }

    #[test]
    fn a_streamed_reply_is_the_whole_reply() {
        let chunk = |rest: &str| {
            format!(
                "data: {{\"id\":\"c1\",\"object\":\"chat.completion.chunk\",\"created\":7,\
                 \"model\":\"m\",{rest}}}\n\n"
            )
        };
        let choice = |delta: &str, finish: &str| {
            chunk(&format!(
                "\"choices\":[{{\"index\":0,\"delta\":{delta},\"finish_reason\":{finish}}}],\
                 \"usage\":null"
            ))
        };
        let body = [
            choice(r#"{"role":"assistant","content":""}"#, "null"),
            choice(r#"{"reasoning_content":"Let me "}"#, "null"),
            choice(r#"{"reasoning_content":"think."}"#, "null"),
            choice(r#"{"content":"It is "}"#, "null"),
            choice(r#"{"content":"sunny."}"#, "null"),
            choice(
                r#"{"tool_calls":[{"index":0,"id":"c9","type":"function","function":{"name":"weather","arguments":""}}]}"#,
                "null",
            ),
            choice(
                r#"{"tool_calls":[{"index":0,"function":{"arguments":"{\"city\":"}}]}"#,
                "null",
            ),
            choice(
                r#"{"tool_calls":[{"index":0,"function":{"arguments":"\"Paris\"}"}}]}"#,
                "null",
            ),
            choice("{}", r#""tool_calls""#),
            chunk(
                r#""choices":[],"usage":{"prompt_tokens":11,"completion_tokens":5,"total_tokens":16}"#,
            ),
            "data: [DONE]\n\n".to_string(),
        ]
        .concat();

        let (read, deltas) = streamed(&body);
        let read = read.unwrap();
        assert_eq!(
            deltas,
            [
                Delta::Thinking("Let me ".into()),
                Delta::Thinking("think.".into()),
                Delta::Text("It is ".into()),
                Delta::Text("sunny.".into()),
            ]
        );
        assert_eq!(read.stop, Stop::ToolUse);
        assert_eq!((read.input_tokens, read.output_tokens), (11, 5));
        assert_eq!(read.message.text(), "It is sunny.");
        let calls: Vec<_> = read.message.tool_calls().collect();
        assert_eq!(calls, [("c9", "weather", &json!({ "city": "Paris" }))]);
        assert!(
            matches!(&read.message.content[0], Block::Thinking { text, .. } if text == "Let me think.")
        );
    }

    #[test]
    fn a_stream_cut_short_or_failing_is_an_error() {
        let cut = "data: {\"id\":\"c1\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"It\"},\"finish_reason\":null}]}\n\n";
        let (read, deltas) = streamed(cut);
        assert_eq!(deltas, [Delta::Text("It".into())]);
        assert!(read.unwrap_err().to_string().contains("ended before"));

        let failed = "data: {\"error\":{\"message\":\"overloaded\"}}\n\n";
        assert_eq!(streamed(failed).0.unwrap_err().to_string(), "overloaded");
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

    /// An assistant turn with neither text nor tool calls (e.g. only thinking from another
    /// vendor) would be `{"content": null}` with no `tool_calls`, which OpenAI rejects.
    #[test]
    fn skips_empty_assistant_turns() {
        let mut ctx = Context::default();
        ctx.push(Message::user("Hi"));
        ctx.push(Message {
            role: Role::Assistant,
            content: vec![Block::Thinking {
                text: "signed".into(),
                signature: Some("sig".into()),
            }],
        });
        ctx.push(Message::user("Still there?"));
        let body = OpenAi.body(&Request::new("m"), &ctx);
        assert_eq!(
            body["messages"],
            json!([
                { "role": "user", "content": "Hi" },
                { "role": "user", "content": "Still there?" },
            ])
        );
    }

    /// Arguments that aren't valid JSON are kept as the raw string, and go back verbatim.
    #[test]
    fn invalid_tool_arguments_round_trip_verbatim() {
        let reply: ChatCompletion = serde_json::from_value(json!({
            "id": "c", "object": "chat.completion", "created": 0, "model": "m",
            "choices": [{ "index": 0, "finish_reason": "length", "message": {
                "role": "assistant", "content": null,
                "tool_calls": [{ "id": "c1", "type": "function",
                    "function": { "name": "weather", "arguments": "{\"city\": \"Par" } }],
            }}],
            "usage": { "prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2 }
        }))
        .unwrap();
        let mut ctx = Context::default();
        ctx.push(OpenAi.response(reply).unwrap().message);
        let body = OpenAi.body(&Request::new("m"), &ctx);
        assert_eq!(
            body["messages"][0]["tool_calls"][0]["function"]["arguments"],
            "{\"city\": \"Par"
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
    fn max_tokens_only_when_set() {
        let ctx = pong_context();
        assert!(
            OpenAi
                .body(&Request::new("m"), &ctx)
                .get("max_tokens")
                .is_none()
        );
        let mut req = Request::new("m");
        req.max_tokens = Some(256);
        assert_eq!(OpenAi.body(&req, &ctx)["max_tokens"], 256);
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

    /// The model tokidoki answers with on Fireworks (`tokidoki::Config::models`): vendors
    /// retire names, and a wrong one is a 404 on every call.
    const FIREWORKS_MODEL: &str = "accounts/fireworks/models/deepseek-v4p1-flash";

    #[tokio::test]
    #[ignore = "calls Fireworks"]
    async fn fireworks() {
        pong(
            Provider::fireworks(&key("FIREWORKS_API_KEY")),
            FIREWORKS_MODEL,
        )
        .await;
    }

    /// The cheap default must still call tools: the agent is nothing without them.
    #[tokio::test]
    #[ignore = "calls Fireworks"]
    async fn fireworks_tool_loop() {
        let provider = Provider::fireworks(&key("FIREWORKS_API_KEY"));
        let req = Request::new(FIREWORKS_MODEL).thinking(false);
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
        assert!(res.message.text().to_lowercase().contains("sunny"));
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
