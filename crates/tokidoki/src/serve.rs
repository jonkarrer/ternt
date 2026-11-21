//! Tokidoki as a process of its own, for a program that would rather not have its agents in
//! its own (ternt: a crash in an agent must never end the browser). The program starts
//! `tokidoki-serve`, writes requests to its stdin and reads its stdout: JSON, one object per
//! line. docs/SIDECAR.md is the contract, which is ternt's (its docs/AGENT-SIDECAR.md).
//!
//!   {"ready":{"backend":"tokidoki","version":"0.1.0",...}}                 first line out
//!   {"id":12,"method":"send","space":"space-7","params":{"input":{...}}}   a request, in
//!   {"id":12,"result":{...}}  or  {"id":12,"error":"..."}                  its answer, out
//!   {"space":"space-7","event":{...}}                                      an event, out
//!
//! One agent per space (`Spaces`), opened by the first request that names it. Requests are
//! answered as each finishes, in any order; a space's events are in the order they happened.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use interface::Speaker;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;

use crate::{Spaces, Tokidoki};

/// How long agents get to stop once the program says `shutdown` (or sends SIGTERM): it
/// waits 300 ms, then kills. What is cut off resumes at the next start.
pub const STOP_BUDGET: Duration = Duration::from_millis(150);

/// Opens the agent whose files are in a folder.
pub type Open = Arc<dyn Fn(&Path) -> Result<Tokidoki> + Send + Sync>;

#[derive(Debug, Deserialize)]
struct Request {
    id: u64,
    method: String,
    /// The agent's folder name. Every method but `shutdown` has one.
    #[serde(default)]
    space: Option<String>,
    #[serde(default)]
    params: Value,
}

/// Serves `spaces` (opened with `open`) to whoever writes `input` and reads `output`, until
/// `input` ends or a `shutdown` request, then stops every agent. `about` is what the ready
/// line says besides the backend and its version (an object: the provider in use, say).
pub async fn run(
    spaces: Arc<Spaces>,
    open: Open,
    about: Value,
    input: impl AsyncBufRead + Unpin,
    output: impl AsyncWrite + Unpin + Send + 'static,
) -> Result<()> {
    // One writer, so lines never interleave.
    let (out, lines) = mpsc::unbounded_channel();
    let writer = tokio::spawn(write(lines, output));
    let mut ready = json!({ "backend": "tokidoki", "version": env!("CARGO_PKG_VERSION") });
    if let (Some(ready), Value::Object(about)) = (ready.as_object_mut(), about) {
        ready.extend(about);
    }
    let _ = out.send(json!({ "ready": ready }));

    let server = Arc::new(Server {
        spaces: spaces.clone(),
        open,
        out,
    });
    let mut requests = input.lines();
    let mut told = false;
    loop {
        let line = match requests.next_line().await {
            Ok(Some(line)) => line,
            Ok(None) => break,
            // Bad bytes from the host (not UTF-8): the same as the host going away.
            Err(error) => {
                eprintln!("tokidoki: reading requests: {error}");
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let request: Request = match serde_json::from_str(&line) {
            Ok(request) => request,
            Err(error) => {
                eprintln!("tokidoki: a request that can't be read: {error}");
                continue;
            }
        };
        if request.method == "shutdown" {
            let _ = server.out.send(json!({ "id": request.id, "result": null }));
            told = true;
            break;
        }
        tokio::spawn(server.clone().answer(request));
    }

    let closed = match told {
        true => (tokio::time::timeout(STOP_BUDGET, spaces.close_all()).await).unwrap_or(Ok(())),
        false => spaces.close_all().await,
    };
    // What is still to be written is; an agent cut off above may hold the line open.
    drop(server);
    let _ = tokio::time::timeout(Duration::from_millis(100), writer).await;
    closed
}

async fn write(mut lines: mpsc::UnboundedReceiver<Value>, mut output: impl AsyncWrite + Unpin) {
    while let Some(line) = lines.recv().await {
        // JSON text has no raw newline: one value, one line.
        let mut line = line.to_string().into_bytes();
        line.push(b'\n');
        if output.write_all(&line).await.is_err() || output.flush().await.is_err() {
            return;
        }
    }
}

struct Server {
    spaces: Arc<Spaces>,
    open: Open,
    out: mpsc::UnboundedSender<Value>,
}

impl Server {
    /// Every request gets exactly one answer, a panic in its handler included: otherwise the
    /// host waits out its timeout and learns nothing.
    async fn answer(self: Arc<Self>, request: Request) {
        let id = request.id;
        let out = self.out.clone();
        let handled = tokio::spawn(async move { self.handle(request).await }).await;
        let answer = match handled {
            Ok(Ok(result)) => json!({ "id": id, "result": result }),
            Ok(Err(error)) => json!({ "id": id, "error": format!("{error:#}") }),
            Err(panicked) => {
                json!({ "id": id, "error": format!("the request panicked: {panicked}") })
            }
        };
        let _ = out.send(answer);
    }

    async fn handle(&self, request: Request) -> Result<Value> {
        let space = request.space.context("a space is required")?;
        let params = request.params;
        match request.method.as_str() {
            "close" => return self.spaces.close(&space).await.map(|()| Value::Null),
            "delete" => return self.spaces.delete(&space).await.map(|()| Value::Null),
            _ => {}
        }
        let agent = self.agent(&space).await?;
        let owner = Speaker::owner();
        Ok(match request.method.as_str() {
            "send" => {
                if let Some(label) = params["label"].as_str() {
                    agent.set_label(label);
                }
                json!(agent.send(param(&params, "input")?).await?)
            }
            "threads" => json!(agent.threads()?),
            "history" => {
                let since = params["since"].as_u64().unwrap_or(0) as usize;
                json!(agent.history(&param::<String>(&params, "thread")?, since)?)
            }
            "abort" => json!(agent.abort(&param::<String>(&params, "thread")?)?),
            "memories" => json!(agent.memories(&owner)?),
            "remember" => json!(agent.remember(&owner, &param::<String>(&params, "text")?)?),
            "revise" => {
                let text: String = param(&params, "text")?;
                json!(agent.revise(&owner, param(&params, "id")?, &text)?)
            }
            "forget" => json!(agent.forget(&owner, param(&params, "id")?)?),
            "feedback" => json!(agent.feedback(param(&params, "feedback")?)?),
            other => bail!("unknown method {other}"),
        })
    }

    /// The space's agent, opened if it isn't, its events on their way out from the start.
    async fn agent(&self, space: &str) -> Result<Tokidoki> {
        let open = |data: &Path| {
            let agent = (self.open)(data)?;
            let (mut events, out, space) = (agent.events(), self.out.clone(), space.to_string());
            tokio::spawn(async move {
                while let Some(event) = events.recv().await {
                    if out.send(json!({ "space": space, "event": event })).is_err() {
                        return;
                    }
                }
            });
            Ok(agent)
        };
        self.spaces.open_with(space, open).await
    }
}

/// One of a request's `params`.
fn param<T: DeserializeOwned>(params: &Value, name: &str) -> Result<T> {
    serde_json::from_value(params[name].clone()).with_context(|| format!("params.{name}"))
}
