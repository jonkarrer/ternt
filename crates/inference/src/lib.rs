pub mod providers;

use providers::baseten::ChatCompletion;
use serde_json::json;

/// Sends one user prompt to a Baseten Model API and returns the reply text.
pub async fn chat(api_key: &str, model: &str, prompt: &str) -> reqwest::Result<String> {
    let res: ChatCompletion = reqwest::Client::new()
        .post("https://inference.baseten.co/v1/chat/completions")
        .header("Authorization", format!("Api-Key {api_key}"))
        .json(&json!({
            "model": model,
            "messages": [{ "role": "user", "content": prompt }],
        }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    Ok(res
        .choices
        .into_iter()
        .next()
        .and_then(|c| c.message.content)
        .unwrap_or_default())
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
