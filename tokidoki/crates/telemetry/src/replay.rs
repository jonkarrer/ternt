//! Re-sending a recorded model call, as it was or edited, to see whether the model answers
//! differently: a changed prompt, another model, or just a second sample.

use std::time::Instant;

use anyhow::{Context as _, Result, bail, ensure};
use inference::providers::Api;
use inference::{Provider, Response};
use serde_json::{Value, json};

use crate::inspect::Output;
use crate::journal::Entry;

/// Sends `entry`'s request again (or `body` in its place) to the same provider, with the API
/// key from the environment, and shows the reply next to the recorded one.
pub async fn resend(entry: &Entry, body: Option<Value>) -> Result<Output> {
    ensure!(
        entry.message.starts_with("model call"),
        "entry {} is not a model call",
        entry.id
    );
    let provider = entry.fields["provider"]
        .as_str()
        .context("the entry names no provider")?;
    let body = body
        .or_else(|| entry.fields.get("request").cloned())
        .context("the entry has no request")?;
    let key = |var: &str| std::env::var(var).with_context(|| format!("{var} is not set"));
    let started = Instant::now();
    let (raw, response) = match provider {
        "Anthropic" => send(Provider::anthropic(&key("ANTHROPIC_API_KEY")?), &body).await?,
        "Baseten" => send(Provider::baseten(&key("BASETEN_API_KEY")?), &body).await?,
        "OpenAI" => send(Provider::openai(&key("OPENAI_API_KEY")?), &body).await?,
        "Fireworks" => send(Provider::fireworks(&key("FIREWORKS_API_KEY")?), &body).await?,
        other => bail!("can't replay calls to {other}"),
    };
    let ms = started.elapsed().as_millis() as u64;

    let field = |name: &str| entry.fields.get(name).cloned().unwrap_or(Value::Null);
    let before = match entry.fields.get("error") {
        Some(error) => format!("failed: {error}"),
        None => crate::journal::as_text(&field("text")),
    };
    let after = response.as_ref().map(|r| r.message.text());
    let calls: Vec<String> = response
        .iter()
        .flat_map(|r| r.message.tool_calls())
        .map(|(_, name, input)| format!("{name} {input}"))
        .collect();
    let text = format!(
        "before (entry {}, {}ms, {}→{} tokens):\n{before}\n  calls: {}\n\nnow ({ms}ms, {}→{} tokens):\n{}\n  calls: {}\n",
        entry.id,
        field("ms"),
        field("input_tokens"),
        field("output_tokens"),
        field("calls"),
        response.as_ref().map_or(0, |r| r.input_tokens),
        response.as_ref().map_or(0, |r| r.output_tokens),
        after.as_deref().unwrap_or("(no reply)"),
        calls.join("; "),
    );
    Ok(Output {
        text,
        json: json!({
            "entry": entry.id,
            "before": { "text": field("text"), "calls": field("calls"), "error": field("error") },
            "now": { "ms": ms, "text": after, "calls": calls, "response": raw },
        }),
    })
}

async fn send<A: Api>(provider: Provider<A>, body: &Value) -> Result<(Value, Option<Response>)> {
    let text = provider.post(serde_json::to_string(body)?).await?;
    let response = provider.read(&text).ok();
    let raw = serde_json::from_str(&text).unwrap_or(Value::String(text));
    Ok((raw, response))
}
