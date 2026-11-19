//! `Provider<A>` does the HTTP for every vendor; `A` is the API format (`OpenAi`, `Anthropic`).

pub mod anthropic;
pub mod openai;

pub use anthropic::Anthropic;
pub use anyhow::Result;
pub use openai::OpenAi;

use anyhow::{Context, bail};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn new(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Request {
    pub model: String,
    pub messages: Vec<Message>,
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct Response {
    pub text: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// How a format builds the request body and reads the reply.
pub trait Api {
    type Reply: DeserializeOwned;
    fn body(&self, req: &Request) -> Value;
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

    pub async fn chat(&self, req: &Request) -> Result<Response> {
        let mut builder = self.http.post(self.url).json(&self.api.body(req));
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
            .with_context(|| format!("{} returned no reply text", self.name))
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
