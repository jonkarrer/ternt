//! A provider chosen while running: which vendor answers is the user's setting, not the
//! program's type.

use std::fmt;
use std::str::FromStr;

use anyhow::{Context as _, Result, bail};
use context::Context;
use serde_json::Value;

use super::{Anthropic, Api, Assembler, OpenAi, Provider, Request, Response};

/// The vendors a user can pick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vendor {
    Anthropic,
    Fireworks,
    Baseten,
}

impl Vendor {
    pub const ALL: [Vendor; 3] = [Vendor::Anthropic, Vendor::Fireworks, Vendor::Baseten];

    /// How it is written in settings: `anthropic`, `fireworks`, `baseten`.
    pub fn id(self) -> &'static str {
        match self {
            Vendor::Anthropic => "anthropic",
            Vendor::Fireworks => "fireworks",
            Vendor::Baseten => "baseten",
        }
    }

    /// The environment variable its key is read from.
    pub fn key_var(self) -> &'static str {
        match self {
            Vendor::Anthropic => "ANTHROPIC_API_KEY",
            Vendor::Fireworks => "FIREWORKS_API_KEY",
            Vendor::Baseten => "BASETEN_API_KEY",
        }
    }
}

impl fmt::Display for Vendor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

impl FromStr for Vendor {
    type Err = anyhow::Error;

    fn from_str(id: &str) -> Result<Self> {
        let id = id.trim().to_ascii_lowercase();
        match Vendor::ALL.into_iter().find(|vendor| vendor.id() == id) {
            Some(vendor) => Ok(vendor),
            None => bail!("no provider {id:?}: anthropic, fireworks or baseten"),
        }
    }
}

/// Either format, decided when the provider is made.
pub enum AnyApi {
    Anthropic(Anthropic),
    OpenAi(OpenAi),
}

impl Provider<AnyApi> {
    /// `vendor`'s provider, with the user's `key` for it.
    pub fn of(vendor: Vendor, key: &str) -> Self {
        match vendor {
            Vendor::Anthropic => Provider::anthropic(key).any(AnyApi::Anthropic),
            Vendor::Fireworks => Provider::fireworks(key).any(AnyApi::OpenAi),
            Vendor::Baseten => Provider::baseten(key).any(AnyApi::OpenAi),
        }
    }
}

impl<A> Provider<A> {
    /// The same provider, its format wrapped as one of `AnyApi`'s.
    pub fn any(self, wrap: impl FnOnce(A) -> AnyApi) -> Provider<AnyApi> {
        Provider {
            name: self.name,
            url: self.url,
            headers: self.headers,
            http: self.http,
            api: wrap(self.api),
            tape: self.tape,
        }
    }
}

impl Api for AnyApi {
    type Reply = Value;

    fn body(&self, req: &Request, ctx: &Context) -> Value {
        match self {
            AnyApi::Anthropic(api) => api.body(req, ctx),
            AnyApi::OpenAi(api) => api.body(req, ctx),
        }
    }

    fn response(&self, reply: Value) -> Option<Response> {
        self.read(&reply.to_string()).ok().flatten()
    }

    fn read(&self, raw: &str) -> Result<Option<Response>> {
        match self {
            AnyApi::Anthropic(api) => api.read(raw),
            AnyApi::OpenAi(api) => api.read(raw),
        }
        .context("reading the reply")
    }

    fn stream(&self) -> Option<Box<dyn Assembler>> {
        match self {
            AnyApi::Anthropic(api) => api.stream(),
            AnyApi::OpenAi(api) => api.stream(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_vendor_is_read_as_it_is_written() {
        for vendor in Vendor::ALL {
            assert_eq!(vendor.to_string().parse::<Vendor>().unwrap(), vendor);
        }
        assert_eq!(" Fireworks ".parse::<Vendor>().unwrap(), Vendor::Fireworks);
        assert!("openai".parse::<Vendor>().is_err());
    }

    /// Whichever format it wraps, it builds that format's request and reads its reply.
    #[test]
    fn it_speaks_the_format_of_its_vendor() {
        let mut ctx = Context::new("Be terse.");
        ctx.push(context::Message::user("hi"));
        let req = Request::new("m");

        let anthropic = AnyApi::Anthropic(Anthropic);
        assert_eq!(anthropic.body(&req, &ctx)["system"], "Be terse.");
        let openai = AnyApi::OpenAi(OpenAi);
        assert_eq!(openai.body(&req, &ctx)["messages"][0]["role"], "system");

        let reply = r#"{"id":"1","object":"chat.completion","created":1,"model":"m",
            "choices":[{"index":0,"message":{"role":"assistant","content":"pong"},
            "finish_reason":"stop"}],
            "usage":{"prompt_tokens":3,"completion_tokens":1,"total_tokens":4}}"#;
        let read = openai.read(reply).unwrap().unwrap();
        assert_eq!(read.message.text(), "pong");
        // The other format's reply is an error that says so, not "no reply".
        assert!(anthropic.read(reply).is_err());
    }
}
