//! The protocol: what goes into tokidoki and what comes out, as plain serde types.
//!
//! `tokidoki::Tokidoki` takes and returns these. Whatever sits upstream or downstream of it
//! depends on this crate alone to read and write them; how they travel is up to it.

mod protocol;

pub use protocol::*;

/// An input the core won't take, because of who sent it or what it holds: the sender's
/// mistake, with a message meant for them. Any other error is tokidoki's own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejected(pub String);

impl std::fmt::Display for Rejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Rejected {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_protocol_reads_as_documented() {
        let input: Input = serde_json::from_str(
            r#"{"from": {"id": "discord:42"}, "parts": [{"type": "text", "text": "hi"}]}"#,
        )
        .unwrap();
        assert_eq!(input.when_busy, WhenBusy::FollowUp);
        assert_eq!(input.from, Speaker::new("discord:42"));
        let event = Event::Checkpoint {
            thread: "t".into(),
            seq: 0,
            at: 0,
            turn: 1,
            change: Change::TurnStarted,
        };
        assert_eq!(
            serde_json::to_value(&event).unwrap(),
            serde_json::json!({"type": "checkpoint", "thread": "t", "seq": 0, "at": 0, "turn": 1,
                               "change": {"kind": "turn_started"}})
        );
    }
}
