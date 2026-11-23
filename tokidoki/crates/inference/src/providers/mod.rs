//! `Provider<A>` does the HTTP for every vendor; `A` is the API format (`OpenAi`, `Anthropic`,
//! or `AnyApi` when the vendor is chosen while running).

pub mod anthropic;
mod any;
pub mod openai;

pub use anthropic::Anthropic;
pub use any::{AnyApi, Vendor};
pub use anyhow::Result;
pub use openai::OpenAi;

use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context as _, bail};
use context::{Context, Message};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

/// Per-call settings. What the model sees lives in `Context`.
/// `None` leaves a setting at the model's default.
#[derive(Debug, Clone)]
pub struct Request {
    pub model: String,
    pub max_tokens: Option<u32>,
    /// `true` turns thinking on and asks for its text back; `false` turns it off.
    /// Some models reject one or the other (Opus 5.5 always thinks).
    pub thinking: Option<bool>,
    /// How hard the model works: depth of thinking, length of answers, number of tool calls.
    pub effort: Option<Effort>,
}

impl Request {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            max_tokens: None,
            thinking: None,
            effort: None,
        }
    }

    pub fn thinking(mut self, on: bool) -> Self {
        self.thinking = Some(on);
        self
    }

    pub fn effort(mut self, effort: Effort) -> Self {
        self.effort = Some(effort);
        self
    }
}

/// Sent as-is; a model that lacks a level returns an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    Low,
    Medium,
    High,
    XHigh,
    Max,
}

/// The model's turn, ready to `ctx.push(res.message)`.
#[derive(Debug, Clone)]
pub struct Response {
    pub message: Message,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub stop: Stop,
}

/// Why the model stopped, the same for every API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    /// It finished its answer.
    Done,
    /// It wants tools run.
    ToolUse,
    /// It hit the output limit: the answer is cut off.
    Cutoff,
    /// It refused, or a content filter stopped it.
    Refused,
}

impl Stop {
    /// The reply is likely not what was wanted.
    pub fn is_suspect(self) -> bool {
        matches!(self, Stop::Cutoff | Stop::Refused)
    }
}

/// How a format builds the request body and reads the reply.
pub trait Api {
    type Reply: DeserializeOwned;
    fn body(&self, req: &Request, ctx: &Context) -> Value;
    fn response(&self, reply: Self::Reply) -> Option<Response>;

    /// Reads a raw reply: an error when it isn't this format's, `None` when it holds no reply.
    fn read(&self, raw: &str) -> Result<Option<Response>> {
        let reply: Self::Reply = serde_json::from_str(raw)?;
        Ok(self.response(reply))
    }

    /// How the format streams a reply; `None` for one that only answers whole.
    fn stream(&self) -> Option<Box<dyn Assembler>> {
        None
    }
}

/// A piece of a reply, as the model writes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delta {
    Text(String),
    Thinking(String),
}

/// Puts a streamed reply back together, one server-sent event at a time, into the reply the
/// format's non-streaming call returns: the rest of the code (and the journal) sees one shape.
pub trait Assembler: Send {
    /// Makes a request body ask for the stream this reads.
    fn ask(&self, body: &mut Value) {
        body["stream"] = true.into();
    }
    /// Takes an event's data, and returns what it adds to the reply's text, if anything.
    fn event(&mut self, event: &Value) -> Result<Option<Delta>>;
    /// The whole reply. An error if the stream stopped before the reply was complete.
    fn finish(self: Box<Self>) -> Result<Value>;
}

/// What `Provider::stream` hands each piece of the reply to.
pub type OnDelta<'a> = &'a (dyn Fn(Delta) + Send + Sync);

/// Recorded replies, for replaying: given a request body exactly as a provider would send it,
/// the raw reply it got then, if any. A body it doesn't know goes to the provider.
pub trait Tape: Send + Sync {
    fn reply(&self, provider: &str, body: &Value) -> Option<Value>;
}

pub struct Provider<A> {
    name: &'static str,
    url: &'static str,
    headers: Vec<(&'static str, String)>,
    http: reqwest::Client,
    api: A,
    tape: Option<Arc<dyn Tape>>,
}

impl<A: Api> Provider<A> {
    pub fn new(
        api: A,
        name: &'static str,
        url: &'static str,
        headers: Vec<(&'static str, String)>,
    ) -> Self {
        Self {
            name,
            url,
            headers,
            http: reqwest::Client::new(),
            api,
            tape: None,
        }
    }

    /// Answers from `tape` the requests it recorded; the rest go to the provider.
    pub fn with_tape(mut self, tape: Arc<dyn Tape>) -> Self {
        self.tape = Some(tape);
        self
    }

    /// Reads a raw reply (as `post` returns it) into a `Response`.
    pub fn read(&self, raw: &str) -> Result<Response> {
        self.api
            .read(raw)
            .context(self.name)?
            .with_context(|| format!("{} returned no reply", self.name))
    }

    /// Sends the request and reads the reply. Every call is traced (`model call`), with the
    /// body sent and the raw reply, so it can be inspected or replayed later.
    pub async fn chat(&self, req: &Request, ctx: &Context) -> Result<Response> {
        self.call(req, ctx, None).await
    }

    /// `chat`, handing `on_delta` the reply's text as the model writes it (nothing, for a
    /// format that doesn't stream or a reply read from the tape). The reply returned and
    /// traced is the same as `chat`'s: the stream is put back together into it, and the
    /// request traced is the one `chat` would send, so replaying it needs no streaming.
    pub async fn stream(
        &self,
        req: &Request,
        ctx: &Context,
        on_delta: OnDelta<'_>,
    ) -> Result<Response> {
        self.call(req, ctx, Some(on_delta)).await
    }

    async fn call(
        &self,
        req: &Request,
        ctx: &Context,
        on_delta: Option<OnDelta<'_>>,
    ) -> Result<Response> {
        let body = self.api.body(req, ctx);
        // Serialized once: what is sent is what is traced (and hashed, for replay).
        let request = serde_json::to_string(&body)?;
        let started = Instant::now();
        let taped = self.tape.as_ref().and_then(|t| t.reply(self.name, &body));
        let replayed = taped.is_some();
        let raw = match (taped, on_delta.zip(self.api.stream())) {
            (Some(raw), _) => Ok(raw.to_string()),
            (None, Some((on_delta, assembler))) => {
                self.post_stream(body, assembler, on_delta).await
            }
            (None, None) => self.post(request.clone()).await,
        };
        let ms = started.elapsed().as_millis() as u64;
        match raw.and_then(|raw| Ok((self.read(&raw)?, raw))) {
            Ok((res, raw)) => {
                let calls: Vec<&str> = res.message.tool_calls().map(|(_, name, _)| name).collect();
                let stop = res.stop;
                let text = res.message.text();
                // Cut off, refused, or nothing at all: still a reply, but likely a bad one.
                if stop.is_suspect() || (calls.is_empty() && text.trim().is_empty()) {
                    tracing::warn!(
                        provider = self.name,
                        model = %req.model,
                        stop = ?stop,
                        "model reply was cut off, refused or empty"
                    );
                }
                tracing::info!(
                    stop = ?stop,
                    provider = self.name,
                    model = %req.model,
                    ms,
                    replayed,
                    input_tokens = res.input_tokens,
                    output_tokens = res.output_tokens,
                    text = %text,
                    calls = ?calls,
                    request = request.as_str(),
                    response = raw.as_str(),
                    "model call"
                );
                Ok(res)
            }
            Err(error) => {
                // The error only: the request holds the user's words, and a host
                // (ternt) keeps this process's stderr in its own log.
                tracing::warn!(
                    provider = self.name,
                    model = %req.model,
                    ms,
                    replayed,
                    error = %format!("{error:#}"),
                    "model call failed"
                );
                tracing::debug!(request = request.as_str(), "the request that failed");
                Err(error)
            }
        }
    }

    /// Posts a serialized request body as-is and returns the reply's text, unparsed: what
    /// `chat` sends, and how a recorded call is replayed, edited or not.
    pub async fn post(&self, body: String) -> Result<String> {
        self.send(body).await?.text().await.context(self.name)
    }

    /// Posts `body` asking for a stream, and returns the reply `assembler` builds from it, as
    /// text: what `post` would have returned.
    async fn post_stream(
        &self,
        mut body: Value,
        mut assembler: Box<dyn Assembler>,
        on_delta: OnDelta<'_>,
    ) -> Result<String> {
        assembler.ask(&mut body);
        let mut res = self.send(serde_json::to_string(&body)?).await?;
        let mut events = Sse::default();
        while let Some(chunk) = res.chunk().await.context(self.name)? {
            for event in events.feed(&chunk) {
                if let Some(delta) = assembler.event(&event).context(self.name)? {
                    on_delta(delta);
                }
            }
        }
        let reply = assembler.finish().context(self.name)?;
        Ok(serde_json::to_string(&reply)?)
    }

    async fn send(&self, body: String) -> Result<reqwest::Response> {
        let mut builder = self
            .http
            .post(self.url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body);
        for (key, value) in &self.headers {
            builder = builder.header(*key, value);
        }
        let res = builder.send().await.context(self.name)?;

        let status = res.status();
        if !status.is_success() {
            let body = res.text().await.unwrap_or_default();
            bail!("{} returned {status}: {}", self.name, error_message(&body));
        }
        Ok(res)
    }
}

/// Server-sent events out of a response body that arrives in arbitrary chunks: each `data:`
/// line that holds JSON is one event. (Event names repeat the data's `type`, so they're
/// skipped, with comments and keep-alives.)
#[derive(Default)]
pub(crate) struct Sse {
    /// Bytes of a line still arriving: a chunk can end anywhere, inside a character too.
    partial: Vec<u8>,
}

impl Sse {
    pub(crate) fn feed(&mut self, chunk: &[u8]) -> Vec<Value> {
        self.partial.extend_from_slice(chunk);
        let mut events = Vec::new();
        while let Some(end) = self.partial.iter().position(|&byte| byte == b'\n') {
            let line: Vec<u8> = self.partial.drain(..=end).collect();
            let data = std::str::from_utf8(&line)
                .ok()
                .and_then(|line| line.trim_end().strip_prefix("data:"));
            if let Some(event) = data.and_then(|data| serde_json::from_str(data.trim()).ok()) {
                events.push(event);
            }
        }
        events
    }
}

/// The message from `{"error": "..."}` or `{"error": {"message": "..."}}`, else the raw body.
fn error_message(body: &str) -> String {
    let json: Value = serde_json::from_str(body).unwrap_or_default();
    let error = &json["error"];
    error
        .as_str()
        .or(error["message"].as_str())
        .unwrap_or(body)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_message_handles_each_shape() {
        assert_eq!(error_message(r#"{"error": "deprecated"}"#), "deprecated");
        assert_eq!(
            error_message(r#"{"error": {"message": "not found"}}"#),
            "not found"
        );
        assert_eq!(error_message("bad gateway"), "bad gateway");
    }

    #[test]
    fn events_survive_any_chunking() {
        let body =
            "event: ping\ndata: {\"type\":\"ping\"}\n\n: comment\ndata: {\"text\":\"héllo\"}\n\n";
        for size in 1..body.len() {
            let mut sse = Sse::default();
            let events: Vec<Value> = body
                .as_bytes()
                .chunks(size)
                .flat_map(|chunk| sse.feed(chunk))
                .collect();
            assert_eq!(events.len(), 2, "chunks of {size}");
            assert_eq!(events[1]["text"], "héllo");
        }
    }
}
