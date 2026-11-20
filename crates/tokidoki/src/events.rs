//! The core's state, as `interface` events: thread checkpoints, submissions, task activity,
//! read from what each engine commit changed.

use std::time::UNIX_EPOCH;

use agent::Agent;
use context::Block;
use interface::{Change, Content, Event, Role};

use crate::Events;
use state::Checkpoint;
use tokio::sync::{broadcast, mpsc};

/// Events from every commit from now on, until the receiver is dropped.
pub(crate) fn follow(agent: Agent) -> Events {
    let mut changes = agent.engine().subscribe();
    let (tx, rx) = mpsc::channel(256);
    tokio::spawn(async move {
        loop {
            let events = match changes.recv().await {
                Ok(committed) => read(&agent, &committed),
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
fn read(agent: &Agent, committed: &engine::Committed) -> Vec<Event> {
    let mut events = Vec::new();
    for (thread, range) in &committed.threads {
        let checkpoints = agent
            .engine()
            .thread(thread, |t| t.checkpoints()[range.clone()].to_vec())
            .unwrap_or_default();
        events.extend(checkpoints.iter().map(|c| checkpoint(thread, c)));
    }
    events.extend(committed.tasks.iter().map(|task| Event::Activity {
        thread: task.thread.clone(),
        task: task.id,
        kind: task.kind.clone(),
        phase: task.phase.clone(),
        state: task.state.to_string(),
    }));
    for key in &committed.docs {
        if let Some(id) = agent::submission_in(key)
            && let Ok(Some(found)) = agent.submission(id)
        {
            events.push(Event::Submission { submission: found });
        }
    }
    events
}

pub(crate) fn checkpoint(thread: &str, checkpoint: &Checkpoint) -> Event {
    Event::Checkpoint {
        thread: thread.to_string(),
        seq: checkpoint.id,
        at: checkpoint
            .at
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64),
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
