//! What is happening right now that no commit of the engine's carries, for whoever is
//! watching: a reply as it is written, and (shown by whoever owns the memory) what memory
//! just learned.
//!
//! Nothing here is stored by being shown. A reply is committed whole once the model has
//! finished it, and a memory is in its own store before it is announced, so a crash, a retry
//! or a slow watcher loses nothing.

use inference::Delta;
use interface::Event;
use tokio::sync::broadcast;

/// Something shown, and where it belongs among the engine's commits.
#[derive(Debug, Clone)]
pub struct Shown {
    pub event: Event,
    /// For a reply's delta: the checkpoint of its thread the reply will be, at the earliest.
    /// The delta belongs before the commit that brings that checkpoint, and after any
    /// earlier one.
    pub ahead_of: Option<usize>,
}

/// Cheap to clone; every clone shows on the same channel.
#[derive(Clone)]
pub struct Live {
    events: broadcast::Sender<Shown>,
}

impl Default for Live {
    fn default() -> Self {
        Self {
            events: broadcast::channel(1024).0,
        }
    }
}

impl Live {
    /// What is shown from now on. One that falls 1024 behind skips ahead: the reply still
    /// arrives whole with its commit.
    pub fn subscribe(&self) -> broadcast::Receiver<Shown> {
        self.events.subscribe()
    }

    /// Shows `event` to everyone watching. Nobody watching is fine.
    pub fn show(&self, event: Event) {
        let _ = self.events.send(Shown {
            event,
            ahead_of: None,
        });
    }

    /// Shows a piece of the reply that will be `thread`'s checkpoint `seq` (or a later one,
    /// if something else is recorded there first).
    pub(crate) fn delta(&self, thread: &str, seq: usize, delta: Delta) {
        let delta = match delta {
            Delta::Text(text) => interface::Delta::Text { text },
            Delta::Thinking(text) => interface::Delta::Thinking { text },
        };
        let event = Event::Delta {
            thread: thread.to_string(),
            delta,
        };
        let _ = self.events.send(Shown {
            event,
            ahead_of: Some(seq),
        });
    }
}
