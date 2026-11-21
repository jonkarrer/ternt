//! The sidecar protocol: JSON, one object per line, over the agent process's
//! stdin and stdout. Ternt sends `Request`s; the process answers each with a
//! `Frame::Response` carrying the same `id`, and sends `Frame::Event` for
//! everything its agents do. Its first line is `Frame::Ready`.
//!
//! Methods and their `params` (all take `space`, the agent's folder name,
//! except `shutdown`):
//!
//! | method     | params                              | result                |
//! |------------|-------------------------------------|-----------------------|
//! | `send`     | `{ "input": Input }`                | `Sent`                |
//! | `threads`  |                                     | `[ThreadSummary]`     |
//! | `history`  | `{ "thread": id, "since": n }`      | `[Event]` checkpoints |
//! | `abort`    | `{ "thread": id }`                  | `bool`                |
//! | `close`    |                                     | `null`                |
//! | `shutdown` |                                     | `null`, then exit     |
//!
//! Agents open on first use; tokidoki's `Spaces::open` is the model. The
//! same file is compiled into the stub sidecar (`src/bin/ternt-agent`), and
//! docs/AGENT-SIDECAR.md describes it for tokidoki's `serve`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::protocol::Event;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Request {
    pub id: u64,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space: Option<String>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub params: Value,
}

/// What the sidecar says about itself when it starts.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Ready {
    /// `stub` or `tokidoki`.
    pub backend: String,
    pub version: String,
}

/// A line from the sidecar.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Frame {
    Ready {
        ready: Ready,
    },
    Event {
        space: String,
        event: Event,
    },
    Response {
        id: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_parse_by_shape() {
        let ready: Frame =
            serde_json::from_str(r#"{"ready":{"backend":"stub","version":"1"}}"#).unwrap();
        assert!(
            matches!(ready, Frame::Ready { ready: Ready { ref backend, .. } } if backend == "stub")
        );
        let event: Frame =
            serde_json::from_str(r#"{"space":"space-7","event":{"type":"lagged"}}"#).unwrap();
        assert!(
            matches!(event, Frame::Event { ref space, event: Event::Lagged } if space == "space-7")
        );
        let ok: Frame = serde_json::from_str(r#"{"id":3,"result":true}"#).unwrap();
        assert_eq!(
            ok,
            Frame::Response {
                id: 3,
                result: Some(Value::Bool(true)),
                error: None
            }
        );
        let err: Frame = serde_json::from_str(r#"{"id":4,"error":"no thread t9"}"#).unwrap();
        assert_eq!(
            err,
            Frame::Response {
                id: 4,
                result: None,
                error: Some("no thread t9".into())
            }
        );
    }

    #[test]
    fn requests_leave_out_what_is_empty() {
        let request = Request {
            id: 1,
            method: "shutdown".into(),
            space: None,
            params: Value::Null,
        };
        assert_eq!(
            serde_json::to_string(&request).unwrap(),
            r#"{"id":1,"method":"shutdown"}"#
        );
        let request = Request {
            id: 2,
            method: "history".into(),
            space: Some("space-1".into()),
            params: serde_json::json!({ "thread": "t1", "since": 4 }),
        };
        let back: Request =
            serde_json::from_str(&serde_json::to_string(&request).unwrap()).unwrap();
        assert_eq!(back, request);
    }
}
