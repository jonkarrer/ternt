//! The core's state, as `interface` events: thread checkpoints, submissions, task activity,
//! built from what each engine commit stored; and, between commits, replies as they are
//! written.

use context::Block;
use interface::{Change, Content, Event, Role};

use crate::Events;
use state::Checkpoint;
use tokio::sync::{broadcast, mpsc};

/// Events from every commit from now on, and what is shown in between (replies as they are
/// written, memory changes), until the receiver is dropped. A reply's deltas come after the
/// commit that started its request and before the one that holds it, however far behind
/// this falls.
pub(crate) fn follow(engine: &engine::Engine, live: &agent::Live) -> Events {
    use broadcast::error::{RecvError, TryRecvError};
    let mut changes = engine.subscribe();
    let mut shown = live.subscribe();
    let (tx, rx) = mpsc::channel(256);
    tokio::spawn(async move {
        loop {
            // Stops as soon as nobody listens, rather than after reading one more commit.
            let change = tokio::select! {
                next = shown.recv() => {
                    let event = match next {
                        Ok(shown) => shown.event,
                        // What was skipped may include a memory change: the client reads again.
                        Err(RecvError::Lagged(_)) => Event::Lagged,
                        Err(RecvError::Closed) => return,
                    };
                    if tx.send(event).await.is_err() {
                        return;
                    }
                    continue;
                }
                change = changes.recv() => change,
                _ = tx.closed() => return,
            };
            let committed = match change {
                Ok(committed) => Some(committed),
                Err(RecvError::Lagged(_)) => None,
                Err(RecvError::Closed) => return,
            };
            // What was shown and not read yet: a delta of a reply this commit holds was shown
            // before it; the rest (a later request's deltas) after.
            let mut events = Vec::new();
            let mut later = Vec::new();
            loop {
                match shown.try_recv() {
                    Ok(shown) if comes_after(&shown, committed.as_deref()) => {
                        later.push(shown.event)
                    }
                    Ok(shown) => events.push(shown.event),
                    Err(TryRecvError::Lagged(_)) => events.push(Event::Lagged),
                    Err(TryRecvError::Empty | TryRecvError::Closed) => break,
                }
            }
            match &committed {
                Some(committed) => events.extend(read(committed)),
                None => events.push(Event::Lagged),
            }
            events.append(&mut later);
            for event in events {
                if tx.send(event).await.is_err() {
                    return;
                }
            }
        }
    });
    rx
}

/// Whether something shown belongs after `committed`: a delta whose reply this commit doesn't
/// hold yet. (With the commit unknown, everything goes first: the client reads again anyway.)
fn comes_after(shown: &agent::Shown, committed: Option<&engine::Committed>) -> bool {
    let (Some(seq), Some(committed), Event::Delta { thread, .. }) =
        (shown.ahead_of, committed, &shown.event)
    else {
        return false;
    };
    let reached = |(id, checkpoints): &(String, Vec<Checkpoint>)| {
        id == thread && checkpoints.iter().any(|c| c.id >= seq)
    };
    !committed.threads.iter().any(reached)
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

/// A memory item, as the protocol shows it.
pub(crate) fn memory(item: &memory::Item) -> interface::Memory {
    interface::Memory {
        id: item.id,
        kind: match item.body {
            memory::Body::Semantic { .. } => interface::MemoryKind::Fact,
            memory::Body::Procedural { .. } => interface::MemoryKind::Procedure,
        },
        text: item.body.text(),
        version: item.version,
        at: telemetry::time::ms(item.created_at),
        source: item.source.as_ref().map(|source| interface::MemorySource {
            thread: source.thread.clone(),
            seq: source.checkpoint,
        }),
    }
}

/// What the memory store just did, as an event.
pub(crate) fn memory_change(change: memory::Change) -> Event {
    use interface::MemoryChange as M;
    let change = match change {
        memory::Change::Written { item, was: None } => M::Learned {
            memory: memory(&item),
        },
        memory::Change::Written {
            item,
            was: Some(was),
        } => M::Revised {
            memory: memory(&item),
            replaces: was.id,
            was: was.body.text(),
        },
        memory::Change::Forgotten { ids } => M::Forgotten { ids },
        memory::Change::Carded(card) => M::Described {
            thread: card.thread,
            about: card.text,
        },
    };
    Event::Memory { change }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A delta read late still lands on the right side of each commit: after the one that
    /// started its request, before the one that holds its reply.
    #[test]
    fn a_late_delta_keeps_its_place_among_commits() {
        let delta = |thread: &str, seq| agent::Shown {
            event: Event::Delta {
                thread: thread.into(),
                delta: interface::Delta::Text { text: "hi".into() },
            },
            ahead_of: Some(seq),
        };
        let commit = |thread: &str, id| engine::Committed {
            threads: vec![(
                thread.to_string(),
                vec![Checkpoint {
                    id,
                    turn: 1,
                    step: 1,
                    at: std::time::SystemTime::now(),
                    event: state::Event::StepStarted,
                }],
            )],
            ..Default::default()
        };
        // The request started at checkpoint 4; its reply will be 5.
        assert!(comes_after(&delta("t", 5), Some(&commit("t", 4))));
        assert!(!comes_after(&delta("t", 5), Some(&commit("t", 5))));
        // Something else took 5 first: the reply is later, the delta still before it.
        assert!(!comes_after(&delta("t", 5), Some(&commit("t", 6))));
        assert!(comes_after(&delta("t", 5), Some(&commit("other", 9))));
        // With the commit unknown, and for what isn't a delta, order doesn't matter.
        assert!(!comes_after(&delta("t", 5), None));
        let memory = agent::Shown {
            event: Event::Lagged,
            ahead_of: None,
        };
        assert!(!comes_after(&memory, Some(&commit("t", 4))));
    }
}
