//! `ternt-agent`: the stub agent sidecar, its own crate so it builds before
//! Ternt (Tauri's build script wants the sidecar binary in place). It speaks
//! the line protocol in docs/AGENT-SIDECAR.md (`../src/agent/wire.rs`) on
//! stdin/stdout, answering every input with a streamed canned reply
//! (`stub.rs`). tokidoki's `serve` binary replaces it with the same protocol;
//! Ternt doesn't change.
//!
//!     ternt-agent --data <dir>      # where agents would keep their files
//!
//! Two inputs test the supervisor in Ternt: `/crash` aborts the process,
//! `/exit` ends it cleanly.

#[path = "../../src/agent/protocol.rs"]
mod protocol;
mod stub;
#[path = "../../src/agent/wire.rs"]
mod wire;

use std::collections::HashMap;

use serde_json::{Value, json};
use tokio::{
    io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::mpsc,
};

use protocol::*;
use wire::{Frame, Ready, Request};

/// Events from every agent, on their way to stdout.
#[derive(Clone)]
pub struct Outbox(mpsc::UnboundedSender<Frame>);

impl Outbox {
    pub fn push(&self, space: &str, event: Event) {
        let _ = self.0.send(Frame::Event {
            space: space.into(),
            event,
        });
    }
}

#[tokio::main]
async fn main() {
    let (tx, mut rx) = mpsc::unbounded_channel::<Frame>();
    // One writer, so lines never interleave.
    let writer = tokio::spawn(async move {
        let mut out = io::stdout();
        while let Some(frame) = rx.recv().await {
            let mut line = serde_json::to_vec(&frame).unwrap_or_default();
            line.push(b'\n');
            if out.write_all(&line).await.is_err() || out.flush().await.is_err() {
                break;
            }
        }
    });
    let _ = tx.send(Frame::Ready {
        ready: Ready {
            backend: "stub".into(),
            version: env!("CARGO_PKG_VERSION").into(),
        },
    });

    let outbox = Outbox(tx.clone());
    let mut agents: HashMap<String, stub::Stub> = HashMap::new();
    let mut lines = BufReader::new(io::stdin()).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        let request: Request = match serde_json::from_str(&line) {
            Ok(request) => request,
            Err(err) => {
                eprintln!("ternt-agent: bad request: {err}");
                continue;
            }
        };
        let id = request.id;
        let shutdown = request.method == "shutdown";
        let frame = match handle(&mut agents, &outbox, request) {
            Ok(result) => Frame::Response {
                id,
                result: Some(result),
                error: None,
            },
            Err(error) => Frame::Response {
                id,
                result: None,
                error: Some(error),
            },
        };
        let _ = tx.send(frame);
        if shutdown {
            break;
        }
    }
    // Let the writer drain, then go.
    drop(agents);
    drop(outbox);
    drop(tx);
    let _ = tokio::time::timeout(std::time::Duration::from_millis(500), writer).await;
}

fn handle(
    agents: &mut HashMap<String, stub::Stub>,
    outbox: &Outbox,
    request: Request,
) -> Result<Value, String> {
    if request.method == "shutdown" {
        return Ok(Value::Null);
    }
    let space = request.space.ok_or("a space is required")?;
    // The stub keeps nothing on disk, so deleting a space is closing it.
    if request.method == "close" || request.method == "delete" {
        agents.remove(&space);
        return Ok(Value::Null);
    }
    let params = request.params;
    let agent = agents
        .entry(space.clone())
        .or_insert_with(|| stub::Stub::new(space.clone(), outbox.clone()));
    match request.method.as_str() {
        "send" => {
            let input: Input = serde_json::from_value(params["input"].clone())
                .map_err(|e| format!("bad input: {e}"))?;
            // Test hooks for the supervisor in Ternt.
            match input.text().trim() {
                "/crash" => {
                    eprintln!("ternt-agent: crashing on request");
                    std::process::abort()
                }
                "/exit" => std::process::exit(0),
                _ => {}
            }
            if let Some(label) = params["label"].as_str() {
                agent.set_label(label);
            }
            agent.send(input).map(|sent| json!(sent))
        }
        "threads" => Ok(json!(agent.threads())),
        "history" => {
            let thread = params["thread"].as_str().ok_or("a thread is required")?;
            let since = params["since"].as_u64().unwrap_or(0) as usize;
            Ok(json!(agent.history(thread, since)))
        }
        "abort" => {
            let thread = params["thread"].as_str().ok_or("a thread is required")?;
            Ok(json!(agent.abort(thread)))
        }
        other => Err(format!("unknown method {other}")),
    }
}
