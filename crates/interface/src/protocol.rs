//! What goes in and comes out. How it travels is not tokidoki's concern: whatever carries it
//! (an app, a bot, a terminal) passes these plain serde values on as it sees fit.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Bumped when a change would break code built against an older one.
pub const VERSION: u32 = 1;

/// Who an input came from. Their memory scope follows from it, so the core decides who may
/// speak. The ID is whatever the code passing inputs in uses to tell people apart.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Speaker {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl Speaker {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: None,
        }
    }

    /// The one whose tokidoki it is.
    pub fn owner() -> Self {
        Self::new("owner")
    }
}

/// Bytes, inline or where to fetch them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Media {
    Base64(String),
    Url(String),
}

/// One piece of an input. The core takes text today; the other kinds are in the protocol so
/// they can be passed in, and are refused until the core can use them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Part {
    Text {
        text: String,
    },
    Image {
        media_type: String,
        data: Media,
    },
    Audio {
        media_type: String,
        data: Media,
    },
    File {
        name: String,
        media_type: String,
        data: Media,
    },
}

impl Part {
    pub fn kind(&self) -> &'static str {
        match self {
            Part::Text { .. } => "text",
            Part::Image { .. } => "image",
            Part::Audio { .. } => "audio",
            Part::File { .. } => "file",
        }
    }
}

/// What to do with an input to a thread that is busy answering another.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WhenBusy {
    /// Wait for the current answer, then answer this.
    #[default]
    FollowUp,
    /// Join the current work after its tool round.
    Steer,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Input {
    pub from: Speaker,
    pub parts: Vec<Part>,
    /// The thread to send it to; `None` lets the router choose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
    /// The same ID finds the same submission instead of submitting twice (a retried send).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default)]
    pub when_busy: WhenBusy,
}

impl Input {
    pub fn text(from: Speaker, text: impl Into<String>) -> Self {
        Self {
            from,
            parts: vec![Part::Text { text: text.into() }],
            thread: None,
            request_id: None,
            when_busy: WhenBusy::default(),
        }
    }

    pub fn to(mut self, thread: impl Into<String>) -> Self {
        self.thread = Some(thread.into());
        self
    }
}

/// Why an input went where it did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Route {
    /// `rule`, `search`, `classifier` or `model`.
    pub by: String,
    pub reason: String,
    /// A new thread was started for it.
    pub new: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sent {
    pub thread: String,
    pub submission: String,
    /// `None` when the input named its thread, or its request ID was already submitted.
    pub route: Option<Route>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Queued,
    Placed,
    Done,
    Unanswered,
}

/// An input and what became of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Submission {
    pub id: String,
    pub thread: String,
    pub text: String,
    pub status: Status,
    pub answer: Option<String>,
    /// Why it went unanswered.
    pub reason: Option<String>,
}

impl Submission {
    pub fn is_settled(&self) -> bool {
        matches!(self.status, Status::Done | Status::Unanswered)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThreadSummary {
    pub id: String,
    /// The start of its first message.
    pub title: String,
    pub turns: u32,
    /// Ms since the epoch.
    pub created_at: u64,
    /// The last message, ms since the epoch.
    pub active_at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Content {
    Text {
        text: String,
    },
    ToolCall {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        call_id: String,
        content: String,
        is_error: bool,
    },
    Thinking {
        text: String,
    },
}

/// A change to a thread, as recorded: every checkpoint is one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Change {
    TurnStarted,
    StepStarted,
    Message {
        role: Role,
        content: Vec<Content>,
    },
    TurnEnded,
    Routed {
        by: String,
        reason: String,
    },
    /// Memories brought into the conversation.
    Recalled {
        memories: Vec<String>,
    },
    /// Messages replaced: compaction, or a long-idle thread settled into its resume card.
    Compacted {
        replaced: usize,
        with: usize,
    },
    System {
        text: Option<String>,
    },
    ToolAdded {
        name: String,
    },
    ToolRemoved {
        name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// A thread changed. `seq` counts a thread's checkpoints from 0 with no gaps: a client
    /// that sees one out of order, or reconnects, catches up with `Tokidoki::history(thread,
    /// seq)`.
    Checkpoint {
        thread: String,
        seq: usize,
        /// Ms since the epoch.
        at: u64,
        turn: u32,
        change: Change,
    },
    /// An input's status changed.
    Submission { submission: Submission },
    /// Work on a thread moved on: `kind` is what works (`agent.generation`, `agent.tool`,
    /// `agent.idle`), `phase` what it is doing (`request`, `consider`, `call`, `dream`, …),
    /// `state` whether it runs, waits, sleeps or has ended (`terminal`).
    Activity {
        thread: String,
        task: i64,
        kind: String,
        phase: String,
        state: String,
    },
    /// Events were missed (the client fell behind): re-read what it shows.
    Lagged,
}

/// "That was wrong": recorded with the thread (and turn), for investigating later.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Feedback {
    pub from: Speaker,
    pub thread: String,
    /// The turn it is about; `None` for the latest.
    #[serde(default)]
    pub turn: Option<u32>,
    pub text: String,
}
