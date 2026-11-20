//! `Provider<A>` does the HTTP for every vendor; `A` is the API format (`OpenAi`, `Anthropic`).

pub mod anthropic;
pub mod openai;

pub use anthropic::Anthropic;
pub use anyhow::Result;
pub use openai::OpenAi;

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
}

/// How a format builds the request body and reads the reply.
pub trait Api {
    type Reply: DeserializeOwned;
    fn body(&self, req: &Request, ctx: &Context) -> Value;
    fn response(&self, reply: Self::Reply) -> Option<Response>;
}

pub struct Provider<A> {
    name: &'static str,
    url: &'static str,
    headers: Vec<(&'static str, String)>,
    http: reqwest::Client,
    api: A,
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
        }
    }

    pub async fn chat(&self, req: &Request, ctx: &Context) -> Result<Response> {
        let mut builder = self.http.post(self.url).json(&self.api.body(req, ctx));
        for (key, value) in &self.headers {
            builder = builder.header(*key, value);
        }
        let res = builder.send().await.context(self.name)?;

        let status = res.status();
        if !status.is_success() {
            let body = res.text().await.unwrap_or_default();
            bail!("{} returned {status}: {}", self.name, error_message(&body));
        }

        self.api
            .response(res.json().await.context(self.name)?)
            .with_context(|| format!("{} returned no reply", self.name))
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
}
