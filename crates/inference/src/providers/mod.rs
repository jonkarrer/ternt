//! Provider-neutral chat types. Each provider maps these to and from its own API.

pub mod baseten;

use std::future::Future;

pub use anyhow::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct Request {
    pub model: String,
    pub messages: Vec<Message>,
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct Response {
    pub text: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
}

pub trait Provider {
    fn chat(&self, req: &Request) -> impl Future<Output = Result<Response>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Echo;

    impl Provider for Echo {
        async fn chat(&self, req: &Request) -> Result<Response> {
            Ok(Response {
                text: req.messages.last().unwrap().content.clone(),
                ..Default::default()
            })
        }
    }

    #[tokio::test]
    async fn provider_works() {
        let req = Request {
            model: "any".into(),
            messages: vec![Message {
                role: Role::User,
                content: "ping".into(),
            }],
            max_tokens: None,
        };
        assert_eq!(Echo.chat(&req).await.unwrap().text, "ping");
    }
}
