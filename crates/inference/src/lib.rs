pub mod providers;

use anyhow::{Context, bail};
use providers::baseten::{ChatCompletion, ErrorResponse};
use serde_json::json;

/// Sends one user prompt to a Baseten Model API and returns the reply text.
pub async fn chat(api_key: &str, model: &str, prompt: &str) -> anyhow::Result<String> {
    let res = reqwest::Client::new()
        .post("https://inference.baseten.co/v1/chat/completions")
        .header("Authorization", format!("Api-Key {api_key}"))
        .json(&json!({
            "model": model,
            "messages": [{ "role": "user", "content": prompt }],
        }))
        .send()
        .await
        .context("sending request to Baseten")?;

    let status = res.status();
    if !status.is_success() {
        let body = res.text().await.unwrap_or_default();
        let message = serde_json::from_str::<ErrorResponse>(&body).map_or(body, |e| e.error);
        bail!("Baseten returned {status}: {message}");
    }

    let completion: ChatCompletion = res.json().await.context("parsing Baseten response")?;
    completion
        .choices
        .into_iter()
        .next()
        .and_then(|c| c.message.content)
        .context("Baseten returned no reply text")
}
#[cfg(test)]
mod tests {
    use super::*;

    // BASETEN_API_KEY=... cargo test -p inference -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "calls Baseten; needs BASETEN_API_KEY"]
    async fn chat_returns_text() {
        let key = std::env::var("BASETEN_API_KEY").expect("set BASETEN_API_KEY");
        let reply = chat(&key, "deepseek-ai/DeepSeek-V4.1-Flash", "Reply with: pong")
            .await
            .unwrap();
        println!("{reply}");
        assert!(!reply.is_empty());
    }
}
