//! A stand-in agent until tokidoki is wired in: every input gets a streamed
//! canned reply with the same shape of events a real turn has (submission,
//! checkpoints, activity, deltas), so Ternt's panel can be built, tested and
//! profiled without keys or a model. Nothing is persisted: a restart of the
//! sidecar forgets every conversation (tokidoki resumes from its files).
//!
//! Inputs starting with `/tool` also run a pretend tool; `/error` goes
//! unanswered. Every answered question leaves one pretend memory behind, so
//! Settings → Agent → Memory has something to list, edit, forget and wipe.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use tokio::task::JoinHandle;

use crate::{Outbox, protocol::*};

/// Pause before the first word, as a model would.
const THINK: Duration = Duration::from_millis(350);
/// Pause while the pretend tool runs.
const TOOL: Duration = Duration::from_millis(400);
/// Between words of the reply.
const WORD: Duration = Duration::from_millis(18);

pub struct Stub {
    /// The agent's folder name (`space-7`): how events are tagged.
    space: String,
    /// What the space is called, for the canned text (`label` on `send`).
    label: Mutex<String>,
    /// What it was told to run as (`configure`): provider and tools, echoed
    /// back so Settings shows them. The stub runs on nothing.
    setup: Mutex<(String, Vec<String>)>,
    outbox: Outbox,
    inner: Arc<Mutex<Inner>>,
}

#[derive(Default)]
struct Inner {
    threads: Vec<Thread>,
    next: u64,
    /// The turn under way on a thread.
    running: HashMap<String, JoinHandle<()>>,
    /// What the stub "learned", newest last; ids come from `next` too.
    memories: Vec<Memory>,
}

struct Thread {
    id: String,
    title: String,
    created_at: u64,
    active_at: u64,
    turns: u32,
    /// Checkpoints, `seq` = index.
    log: Vec<Event>,
}

/// What the stub answers.
pub struct Reply {
    pub tool: Option<ToolUse>,
    /// `Err`: the input goes unanswered, with this reason.
    pub text: Result<String, String>,
}

pub struct ToolUse {
    pub name: String,
    pub input: serde_json::Value,
    pub result: String,
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// The canned reply for an input in a space called `space`.
pub fn script(space: &str, text: &str) -> Reply {
    let text = text.trim();
    if let Some(rest) = text.strip_prefix("/error") {
        let reason = rest.trim();
        return Reply {
            tool: None,
            text: Err(if reason.is_empty() {
                "The stub was asked to fail.".into()
            } else {
                reason.into()
            }),
        };
    }
    let (tool, asked) = match text.strip_prefix("/tool") {
        Some(rest) => (
            Some(ToolUse {
                name: "space_tabs".into(),
                input: serde_json::json!({ "space": space }),
                result: format!(
                    "The {space} space has 3 tabs open: Fixture Home, Fixture Page 2, a terminal."
                ),
            }),
            rest.trim(),
        ),
        None => (None, text),
    };
    let about = if asked.is_empty() {
        String::new()
    } else {
        format!(" You asked: “{asked}”.")
    };
    let tool_line = if tool.is_some() {
        " I looked at the space's tabs with a pretend tool first, the way the real agent will once it has tools."
    } else {
        ""
    };
    Reply {
        tool,
        text: Ok(format!(
            "I'm the {space} space's agent, standing in until tokidoki is connected.{about}{tool_line} \
             Each space gets its own agent with its own memory, so nothing said here reaches your other spaces. \
             Try /tool to see a tool call, /error to see a failed turn, or /crash to watch Ternt restart me."
        )),
    }
}

/// `text` split into words that stream one at a time, whitespace attached.
pub fn chunks(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        current.push(ch);
        if ch.is_whitespace() {
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

impl Thread {
    fn new(id: String, text: &str) -> Self {
        let now = now_ms();
        Self {
            id,
            title: text.chars().take(60).collect(),
            created_at: now,
            active_at: now,
            turns: 0,
            log: Vec::new(),
        }
    }

    /// Record a change and return its event.
    fn push(&mut self, change: Change) -> Event {
        let event = Event::Checkpoint {
            thread: self.id.clone(),
            seq: self.log.len(),
            at: now_ms(),
            turn: self.turns,
            change,
        };
        self.log.push(event.clone());
        self.active_at = now_ms();
        event
    }

    fn summary(&self) -> ThreadSummary {
        ThreadSummary {
            id: self.id.clone(),
            title: self.title.clone(),
            turns: self.turns,
            about: None,
            created_at: self.created_at,
            active_at: self.active_at,
        }
    }
}

impl Inner {
    fn thread(&mut self, id: &str) -> Option<&mut Thread> {
        self.threads.iter_mut().find(|t| t.id == id)
    }

    fn id(&mut self, prefix: &str) -> String {
        self.next += 1;
        format!("{prefix}{}", self.next)
    }
}

/// Record and send a checkpoint.
fn checkpoint(inner: &Mutex<Inner>, outbox: &Outbox, space: &str, thread: &str, change: Change) {
    let event = inner.lock().unwrap().thread(thread).map(|t| t.push(change));
    if let Some(event) = event {
        outbox.push(space, event);
    }
}

/// The stub learns one fact from every question it answers, and says so.
fn learn(
    inner: &Mutex<Inner>,
    outbox: &Outbox,
    space: &str,
    label: &str,
    thread: &str,
    asked: &str,
) {
    let asked: String = asked.trim().chars().take(80).collect();
    if asked.is_empty() {
        return;
    }
    let memory = {
        let mut inner = inner.lock().unwrap();
        let id = inner.next as i64 + 1;
        inner.next += 1;
        let seq = inner.thread(thread).map_or(0, |t| t.log.len());
        let memory = Memory {
            id,
            kind: MemoryKind::Fact,
            text: format!("In the {label} space, you asked: “{asked}”"),
            version: 1,
            at: now_ms(),
            source: Some(MemorySource {
                thread: thread.to_string(),
                seq,
            }),
        };
        inner.memories.push(memory.clone());
        memory
    };
    outbox.push(
        space,
        Event::Memory {
            change: MemoryChange::Learned { memory },
        },
    );
}

/// Send `base` (its id, thread and text) with a new status.
fn send_status(
    outbox: &Outbox,
    space: &str,
    base: &Submission,
    status: Status,
    answer: Option<String>,
    reason: Option<String>,
) {
    outbox.push(
        space,
        Event::Submission {
            submission: Submission {
                status,
                answer,
                reason,
                ..base.clone()
            },
        },
    );
}

impl Stub {
    pub fn new(space: String, outbox: Outbox) -> Self {
        Self {
            label: Mutex::new(space.clone()),
            setup: Mutex::new(("fireworks".into(), Vec::new())),
            space,
            outbox,
            inner: Arc::default(),
        }
    }

    /// Takes the setup's parts that are given, keeps the rest, and says how
    /// it "runs": the stub needs no key, so it always has one.
    pub fn configure(&self, params: &serde_json::Value) -> serde_json::Value {
        let mut setup = self.setup.lock().unwrap();
        if let Some(provider) = params["provider"].as_str() {
            setup.0 = provider.into();
        }
        if let Some(tools) = params["tools"].as_array() {
            setup.1 = tools
                .iter()
                .filter_map(|t| t.as_str().map(String::from))
                .collect();
        }
        serde_json::json!({
            "provider": setup.0,
            "model": null,
            "tools": setup.1,
            "has_key": true,
            "asleep": null,
        })
    }

    pub fn set_label(&self, label: &str) {
        *self.label.lock().unwrap() = label.into();
    }

    pub fn send(&self, input: Input) -> Result<Sent, String> {
        let text = text_of(&input);
        if text.trim().is_empty() {
            return Err("Nothing to send.".into());
        }
        let (thread, submission, route) = {
            let mut inner = self.inner.lock().unwrap();
            let (thread, route) = match input.thread {
                Some(id) => {
                    inner.thread(&id).ok_or_else(|| format!("no thread {id}"))?;
                    (id, None)
                }
                None => {
                    let id = inner.id("thread-");
                    inner.threads.push(Thread::new(id.clone(), &text));
                    (
                        id,
                        Some(Route {
                            by: "rule".into(),
                            reason: "a new conversation".into(),
                            new: true,
                        }),
                    )
                }
            };
            // Busy answering: the new input takes over (the stub doesn't queue).
            if let Some(running) = inner.running.remove(&thread) {
                running.abort();
                let ended = inner.thread(&thread).unwrap().push(Change::TurnEnded);
                self.outbox.push(&self.space, ended);
            }
            let submission = inner.id("sub-");
            inner.thread(&thread).unwrap().turns += 1;
            (thread, submission, route)
        };

        let space = self.space.clone();
        let outbox = self.outbox.clone();
        let inner = self.inner.clone();
        let base = Submission {
            id: submission,
            thread: thread.clone(),
            text: text.clone(),
            status: Status::Queued,
            answer: None,
            reason: None,
        };
        send_status(&outbox, &space, &base, Status::Queued, None, None);
        checkpoint(&inner, &outbox, &space, &thread, Change::TurnStarted);
        checkpoint(
            &inner,
            &outbox,
            &space,
            &thread,
            Change::Message {
                role: Role::User,
                content: vec![Content::Text { text: text.clone() }],
            },
        );
        send_status(&outbox, &space, &base, Status::Placed, None, None);

        let label = self.label.lock().unwrap().clone();
        let reply = script(&label, &text);
        let task_thread = thread.clone();
        let task_base = base.clone();
        let task = tokio::spawn(async move {
            let thread = task_thread;
            let base = task_base;
            outbox.push(
                &space,
                Event::Activity {
                    thread: thread.clone(),
                    doing: Doing::Thinking,
                },
            );
            tokio::time::sleep(THINK).await;
            if let Some(tool) = &reply.tool {
                outbox.push(
                    &space,
                    Event::Activity {
                        thread: thread.clone(),
                        doing: Doing::Tool {
                            name: tool.name.clone(),
                        },
                    },
                );
                checkpoint(
                    &inner,
                    &outbox,
                    &space,
                    &thread,
                    Change::Message {
                        role: Role::Assistant,
                        content: vec![Content::ToolCall {
                            id: "call-1".into(),
                            name: tool.name.clone(),
                            input: tool.input.clone(),
                        }],
                    },
                );
                tokio::time::sleep(TOOL).await;
                checkpoint(
                    &inner,
                    &outbox,
                    &space,
                    &thread,
                    Change::Message {
                        role: Role::User,
                        content: vec![Content::ToolResult {
                            call_id: "call-1".into(),
                            content: tool.result.clone(),
                            is_error: false,
                        }],
                    },
                );
                outbox.push(
                    &space,
                    Event::Activity {
                        thread: thread.clone(),
                        doing: Doing::Thinking,
                    },
                );
                tokio::time::sleep(THINK).await;
            }
            match &reply.text {
                Ok(answer) => {
                    for word in chunks(answer) {
                        outbox.push(
                            &space,
                            Event::Delta {
                                thread: thread.clone(),
                                delta: Delta::Text { text: word },
                            },
                        );
                        tokio::time::sleep(WORD).await;
                    }
                    checkpoint(
                        &inner,
                        &outbox,
                        &space,
                        &thread,
                        Change::Message {
                            role: Role::Assistant,
                            content: vec![Content::Text {
                                text: answer.clone(),
                            }],
                        },
                    );
                    checkpoint(&inner, &outbox, &space, &thread, Change::TurnEnded);
                    send_status(
                        &outbox,
                        &space,
                        &base,
                        Status::Done,
                        Some(answer.clone()),
                        None,
                    );
                    learn(&inner, &outbox, &space, &label, &thread, &base.text);
                }
                Err(reason) => {
                    checkpoint(&inner, &outbox, &space, &thread, Change::TurnEnded);
                    send_status(
                        &outbox,
                        &space,
                        &base,
                        Status::Unanswered,
                        None,
                        Some(reason.clone()),
                    );
                }
            }
            inner.lock().unwrap().running.remove(&thread);
        });
        self.inner
            .lock()
            .unwrap()
            .running
            .insert(thread.clone(), task);
        Ok(Sent {
            thread,
            submission: base.id,
            route,
        })
    }

    pub fn threads(&self) -> Vec<ThreadSummary> {
        let inner = self.inner.lock().unwrap();
        let mut threads: Vec<_> = inner.threads.iter().map(Thread::summary).collect();
        threads.sort_by_key(|t| std::cmp::Reverse(t.active_at));
        threads
    }

    pub fn history(&self, thread: &str, since: usize) -> Vec<Event> {
        let mut inner = self.inner.lock().unwrap();
        inner
            .thread(thread)
            .map(|t| t.log.iter().skip(since).cloned().collect())
            .unwrap_or_default()
    }

    pub fn abort(&self, thread: &str) -> bool {
        let mut inner = self.inner.lock().unwrap();
        let Some(running) = inner.running.remove(thread) else {
            return false;
        };
        running.abort();
        if let Some(t) = inner.thread(thread) {
            let ended = t.push(Change::TurnEnded);
            drop(inner);
            self.outbox.push(&self.space, ended);
        }
        true
    }
}

impl Stub {
    /// Newest first, as tokidoki lists them.
    pub fn memories(&self) -> Vec<Memory> {
        let inner = self.inner.lock().unwrap();
        inner.memories.iter().rev().cloned().collect()
    }

    /// A new version of the memory `id`, which must be current, replaces it.
    pub fn revise(&self, id: i64, text: &str) -> Result<Memory, String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("a memory needs some text".into());
        }
        let (memory, was) = {
            let mut inner = self.inner.lock().unwrap();
            let next = inner.next as i64 + 1;
            inner.next += 1;
            let old = inner
                .memories
                .iter_mut()
                .find(|m| m.id == id)
                .ok_or_else(|| format!("memory {id} has changed or is gone"))?;
            let was = old.text.clone();
            old.id = next;
            old.text = text.to_string();
            old.version += 1;
            old.at = now_ms();
            (old.clone(), was)
        };
        self.outbox.push(
            &self.space,
            Event::Memory {
                change: MemoryChange::Revised {
                    memory: memory.clone(),
                    replaces: id,
                    was,
                },
            },
        );
        Ok(memory)
    }

    pub fn forget(&self, id: i64) -> Result<(), String> {
        let mut inner = self.inner.lock().unwrap();
        let before = inner.memories.len();
        inner.memories.retain(|m| m.id != id);
        if inner.memories.len() == before {
            return Err(format!("memory {id} has changed or is gone"));
        }
        drop(inner);
        self.outbox.push(
            &self.space,
            Event::Memory {
                change: MemoryChange::Forgotten { ids: vec![id] },
            },
        );
        Ok(())
    }

    /// Everything learned, gone; the conversations stay.
    pub fn wipe(&self) {
        let ids: Vec<i64> = {
            let mut inner = self.inner.lock().unwrap();
            inner.memories.drain(..).map(|m| m.id).collect()
        };
        if !ids.is_empty() {
            self.outbox.push(
                &self.space,
                Event::Memory {
                    change: MemoryChange::Forgotten { ids },
                },
            );
        }
    }
}

/// The input's text, joined: what the stub reads to pick a reply.
pub fn text_of(input: &Input) -> String {
    input
        .parts
        .iter()
        .filter_map(|p| match p {
            Part::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_answers_in_the_space_s_name() {
        let reply = script("dev", "what tabs are open?");
        assert!(reply.tool.is_none());
        let text = reply.text.unwrap();
        assert!(text.starts_with("I'm the dev space's agent"));
        assert!(text.contains("“what tabs are open?”"));
    }

    #[test]
    fn script_tool_and_error() {
        let tool = script("dev", "/tool anything").tool.unwrap();
        assert_eq!(tool.name, "space_tabs");
        assert_eq!(tool.input["space"], "dev");
        assert_eq!(
            script("dev", "/error").text.unwrap_err(),
            "The stub was asked to fail."
        );
        assert_eq!(
            script("dev", "/error rate limited").text.unwrap_err(),
            "rate limited"
        );
    }

    #[test]
    fn chunks_rejoin_to_the_text() {
        let text = "one two\nthree  four.";
        let parts = chunks(text);
        assert_eq!(parts, ["one ", "two\n", "three ", " ", "four."]);
        assert_eq!(parts.concat(), text);
        assert!(chunks("").is_empty());
    }

    #[test]
    fn checkpoints_number_without_gaps() {
        let mut thread = Thread::new(
            "t".into(),
            "hello there, this is the first message of the thread",
        );
        assert_eq!(thread.title.chars().count(), 52);
        for change in [Change::TurnStarted, Change::TurnEnded, Change::StepStarted] {
            thread.push(change);
        }
        let seqs: Vec<_> = thread
            .log
            .iter()
            .map(|e| match e {
                Event::Checkpoint { seq, .. } => *seq,
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(seqs, [0, 1, 2]);
    }

    #[tokio::test]
    async fn a_turn_streams_then_lands() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let stub = Stub::new("space-1".into(), Outbox(tx));
        stub.set_label("dev");
        let input = Input {
            from: Speaker {
                id: "owner".into(),
                name: None,
            },
            parts: vec![Part::Text { text: "hi".into() }],
            thread: None,
            request_id: None,
            when_busy: WhenBusy::Steer,
        };
        let sent = stub.send(input).unwrap();
        assert!(sent.route.unwrap().new);
        let mut kinds = Vec::new();
        let mut text = String::new();
        while let Some(frame) = rx.recv().await {
            let crate::wire::Frame::Event { space, event } = frame else {
                panic!("not an event")
            };
            assert_eq!(space, "space-1");
            match &event {
                Event::Delta {
                    delta: Delta::Text { text: t },
                    ..
                } => text.push_str(t),
                Event::Submission { submission } if submission.status == Status::Done => {
                    kinds.push("done");
                    break;
                }
                Event::Checkpoint {
                    change:
                        Change::Message {
                            role: Role::Assistant,
                            ..
                        },
                    ..
                } => kinds.push("assistant"),
                Event::Checkpoint {
                    change:
                        Change::Message {
                            role: Role::User, ..
                        },
                    ..
                } => kinds.push("user"),
                Event::Activity { .. } => kinds.push("activity"),
                _ => {}
            }
        }
        assert_eq!(kinds, ["user", "activity", "assistant", "done"]);
        assert!(text.starts_with("I'm the dev space's agent"));
        let history = stub.history(&sent.thread, 0);
        assert_eq!(
            history.len(),
            4,
            "turn_started, user, assistant, turn_ended"
        );
        assert_eq!(stub.threads().len(), 1);
    }
}
