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

    /// `protocol.ts` is these types, as this crate has them. Regenerate it with `just types`.
    #[cfg(feature = "ts")]
    #[test]
    fn the_typescript_protocol_is_current() {
        use ts_rs::TS;
        fn decl<T: TS>() -> String {
            let docs = T::docs().unwrap_or_default();
            // Times in ms and row IDs: far below what a JSON number holds exactly.
            let config = ts_rs::Config::default().with_large_int("number");
            format!("{docs}export {}", T::decl(&config))
        }
        let types = [
            decl::<Speaker>(),
            decl::<Media>(),
            decl::<Part>(),
            decl::<WhenBusy>(),
            decl::<Input>(),
            decl::<Route>(),
            decl::<Sent>(),
            decl::<Status>(),
            decl::<Submission>(),
            decl::<ThreadSummary>(),
            decl::<Role>(),
            decl::<Content>(),
            decl::<Change>(),
            decl::<Event>(),
            decl::<Memory>(),
            decl::<MemoryKind>(),
            decl::<MemorySource>(),
            decl::<MemoryChange>(),
            decl::<Delta>(),
            decl::<Doing>(),
            decl::<Feedback>(),
        ];
        let header = "// The tokidoki protocol (crates/interface/src/protocol.rs), as TypeScript.\n\
            // Generated: change the Rust types, then run `just types`.\n\n\
            export type JsonValue = number | string | boolean | JsonValue[] | { [key: string]: JsonValue } | null;\n";
        let generated = format!("{header}\n{}\n", types.join("\n\n"));
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/protocol.ts");
        if std::env::var_os("TOKIDOKI_WRITE_TYPES").is_some() {
            std::fs::write(path, &generated).unwrap();
        }
        let current = std::fs::read_to_string(path).unwrap_or_default();
        assert!(
            current == generated,
            "protocol.ts is out of date: run `just types`"
        );
    }
}
