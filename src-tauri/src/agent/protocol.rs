//! The agent protocol: tokidoki's `interface` types, as serde values.
//!
//! These mirror `crates/interface/src/protocol.rs` in the tokidoki repo (and
//! its generated `protocol.ts`, copied to `src/agent/protocol.ts`) so the
//! panel is written against the real wire format from day one. When tokidoki
//! is embedded this module becomes `pub use tokidoki::interface::*;` and the
//! stub goes away; nothing in the page changes.

use serde::{Deserialize, Serialize};

/// Who an input came from.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Speaker {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Bytes, inline or where to fetch them.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Media {
    Base64 { base64: String },
    Url { url: String },
}

/// One piece of an input. Only text is used today.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
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

/// What to do with an input to a thread that is busy answering another.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WhenBusy {
    FollowUp,
    Steer,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Input {
    pub from: Speaker,
    pub parts: Vec<Part>,
    /// The thread to send it to; `None` lets the router choose.
    #[serde(default)]
    pub thread: Option<String>,
    /// The same ID finds the same submission instead of submitting twice.
    #[serde(default)]
    pub request_id: Option<String>,
    pub when_busy: WhenBusy,
}

impl Input {
    /// The input's text, joined.
    pub fn text(&self) -> String {
        self.parts
            .iter()
            .filter_map(|p| match p {
                Part::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Why an input went where it did.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Route {
    /// `rule`, `search`, `classifier` or `model`.
    pub by: String,
    pub reason: String,
    /// A new thread was started for it.
    pub new: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Sent {
    pub thread: String,
    pub submission: String,
    /// `None` when the input named its thread.
    pub route: Option<Route>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Queued,
    Placed,
    Done,
    Unanswered,
}

/// An input and what became of it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Submission {
    pub id: String,
    pub thread: String,
    pub text: String,
    pub status: Status,
    pub answer: Option<String>,
    /// Why it went unanswered.
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ThreadSummary {
    pub id: String,
    /// The start of its first message.
    pub title: String,
    pub turns: u32,
    /// What it is about, in the agent's words (none for a new thread).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about: Option<String>,
    /// Ms since the epoch.
    pub created_at: u64,
    /// The last message, ms since the epoch.
    pub active_at: u64,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Content {
    Text {
        text: String,
    },
    ToolCall {
        id: String,
        name: String,
        input: serde_json::Value,
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
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Change {
    TurnStarted,
    StepStarted,
    Message { role: Role, content: Vec<Content> },
    TurnEnded,
    Routed { by: String, reason: String },
    Recalled { memories: Vec<String> },
    Compacted { replaced: u32, with: u32 },
    System { text: Option<String> },
    ToolAdded { name: String },
    ToolRemoved { name: String },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    Fact,
    Procedure,
}

/// The checkpoint a memory came from.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MemorySource {
    pub thread: String,
    pub seq: u64,
}

/// Something the agent remembers, across every thread.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Memory {
    pub id: i64,
    pub kind: MemoryKind,
    pub text: String,
    pub version: u32,
    pub at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<MemorySource>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MemoryChange {
    Learned {
        memory: Memory,
    },
    Revised {
        memory: Memory,
        replaces: i64,
        was: String,
    },
    Forgotten {
        ids: Vec<i64>,
    },
    Described {
        thread: String,
        about: String,
    },
}

/// What a reply grew by.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Delta {
    Text { text: String },
    Thinking { text: String },
}

/// What the agent started doing on a thread.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Doing {
    Remembering,
    Thinking,
    Tool { name: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Checkpoint {
        thread: String,
        seq: u64,
        /// Ms since the epoch.
        at: u64,
        turn: u32,
        change: Change,
    },
    Submission {
        submission: Submission,
    },
    Activity {
        thread: String,
        doing: Doing,
    },
    Delta {
        thread: String,
        delta: Delta,
    },
    Memory {
        change: MemoryChange,
    },
    /// Events were dropped: read `history` from where the page is.
    Lagged,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_format_matches_tokidoki() {
        // The shapes in crates/interface/protocol.ts: tags and snake_case.
        let event = Event::Checkpoint {
            thread: "t1".into(),
            seq: 3,
            at: 1,
            turn: 1,
            change: Change::Message {
                role: Role::Assistant,
                content: vec![Content::ToolCall {
                    id: "c".into(),
                    name: "tabs".into(),
                    input: serde_json::json!({ "space": 1 }),
                }],
            },
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "checkpoint");
        assert_eq!(json["change"]["kind"], "message");
        assert_eq!(json["change"]["role"], "assistant");
        assert_eq!(json["change"]["content"][0]["type"], "tool_call");
        assert_eq!(
            serde_json::to_value(Event::Lagged).unwrap(),
            serde_json::json!({ "type": "lagged" })
        );
        assert_eq!(
            serde_json::to_value(Doing::Tool { name: "x".into() }).unwrap(),
            serde_json::json!({ "kind": "tool", "name": "x" })
        );
        assert_eq!(
            serde_json::to_value(WhenBusy::FollowUp).unwrap(),
            "follow_up"
        );
        let input: Input = serde_json::from_value(serde_json::json!({
            "from": { "id": "owner" },
            "parts": [{ "type": "text", "text": "hi" }],
            "when_busy": "steer"
        }))
        .unwrap();
        assert_eq!(input.text(), "hi");
        assert_eq!(input.thread, None);
    }
}
