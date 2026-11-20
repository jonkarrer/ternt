//! The core's state, as `interface` events: thread checkpoints, submissions, task activity,
//! built from what each engine commit stored.

use context::Block;
use interface::{Change, Content, Event, Role};

use crate::Events;
use state::Checkpoint;
use tokio::sync::{broadcast, mpsc};

/// Events from every commit from now on, until the receiver is dropped.
pub(crate) fn follow(engine: &engine::Engine) -> Events {
    let mut changes = engine.subscribe();
    let (tx, rx) = mpsc::channel(256);
    tokio::spawn(async move {
        loop {
            // Stops as soon as nobody listens, rather than after reading one more commit.
            let change = tokio::select! {
                change = changes.recv() => change,
                _ = tx.closed() => return,
            };
            let events = match change {
                Ok(committed) => read(&committed),
                Err(broadcast::error::RecvError::Lagged(_)) => vec![Event::Lagged],
                Err(broadcast::error::RecvError::Closed) => return,
            };
            for event in events {
                if tx.send(event).await.is_err() {
                    return;
                }
            }
        }
    });
    rx
}

/// A commit's events: new checkpoints first, then task activity, then submissions (an answer
/// is reported after the message that holds it).
fn read(committed: &engine::Committed) -> Vec<Event> {
    let mut events = Vec::new();
    for (thread, checkpoints) in &committed.threads {
        events.extend(checkpoints.iter().map(|c| checkpoint(thread, c)));
    }
    events.extend(committed.tasks.iter().filter_map(|task| {
        Some(Event::Activity {
            thread: task.thread.clone(),
            doing: agent::doing(task)?,
        })
    }));
    for (key, value) in &committed.docs {
        if let Some(submission) = agent::submission_in(key, value.as_ref()) {
            events.push(Event::Submission { submission });
        }
    }
    events
}

/// A thread's checkpoints in `range` (clamped to what it has), as events.
pub(crate) fn checkpoints(
    engine: &engine::Engine,
    thread: &str,
    range: std::ops::Range<usize>,
) -> anyhow::Result<Vec<Event>> {
    engine.thread(thread, |t| {
        let all = t.checkpoints();
        let range = range.start.min(all.len())..range.end.min(all.len());
        all[range].iter().map(|c| checkpoint(thread, c)).collect()
    })
}

pub(crate) fn checkpoint(thread: &str, checkpoint: &Checkpoint) -> Event {
    Event::Checkpoint {
        thread: thread.to_string(),
        seq: checkpoint.id,
        at: telemetry::time::ms(checkpoint.at),
        turn: checkpoint.turn,
        change: change(&checkpoint.event),
    }
}

fn change(event: &state::Event) -> Change {
    use state::Event as E;
    match event {
        E::SystemSet(text) => Change::System { text: text.clone() },
        E::ToolRegistered(tool) => Change::ToolAdded {
            name: tool.name.clone(),
        },
        E::ToolRemoved(name) => Change::ToolRemoved { name: name.clone() },
        E::TurnStarted => Change::TurnStarted,
        E::StepStarted => Change::StepStarted,
        E::Message(message) => Change::Message {
            role: match message.role {
                context::Role::User => Role::User,
                context::Role::Assistant => Role::Assistant,
            },
            content: message.content.iter().filter_map(content).collect(),
        },
        E::Replaced { range, with } => Change::Compacted {
            replaced: range.len(),
            with: with.len(),
        },
        E::TurnEnded => Change::TurnEnded,
        E::Considered { recalled, .. } => Change::Recalled {
            memories: recalled.iter().map(|r| r.text.clone()).collect(),
        },
        E::Routed { by, reason } => Change::Routed {
            by: by.clone(),
            reason: reason.clone(),
        },
    }
}

fn content(block: &Block) -> Option<Content> {
    Some(match block {
        Block::Text { text } => Content::Text { text: text.clone() },
        Block::ToolCall { id, name, input } => Content::ToolCall {
            id: id.clone(),
            name: name.clone(),
            input: input.clone(),
        },
        Block::ToolResult {
            call_id,
            content,
            is_error,
        } => Content::ToolResult {
            call_id: call_id.clone(),
            content: content.clone(),
            is_error: *is_error,
        },
        Block::Thinking { text, .. } => Content::Thinking { text: text.clone() },
        Block::RedactedThinking { .. } => return None,
    })
}
