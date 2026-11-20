//! Experience: the layer between State and Memory. It takes Attention's signals about a
//! thread, decides what to do about them, and carries that out against memory. While the
//! thread is idle, it dreams: summarizes episodes, cleans up memory, compacts the context.
//!
//!   recall           decided in code: search memory with the signal's text. Fast, no model.
//!   retain / refine  decided by the model: one call with every such signal and the memories
//!                    related to them, so it can correct a memory instead of duplicating it.
//!
//! Nothing here may break a turn: a decision that fails becomes `Ignore` with the reason.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::{Context as _, Result, ensure};
use attention::{Attention, Intent, Signal};
use context::{Block, Context, Message, Role};
use inference::providers::Api;
use inference::{Provider, Request};
use memory::{Body, Card, Episode, Item, Memory, Scope, Source, Write};
use serde_json::{Value, json};
use state::{Event, Recalled, Thread};

pub mod settle;
pub use settle::RESUME_HANDOFF;

/// Recall brings at most this many memories per query.
const RECALL_LIMIT: usize = 3;
/// Related memories the model sees per retain/refine signal.
const RELATED_LIMIT: usize = 5;
/// Transcript characters a card update reads, newest kept.
const CARD_INPUT_CHARS: usize = 12_000;
/// Memories refined per dream.
const REFINE_LIMIT: usize = 200;

/// Where memory is read from and written to.
#[derive(Debug, Clone)]
pub struct Scopes {
    /// Searched on recall.
    pub read: Vec<Scope>,
    /// Where new memories go; also searched.
    pub write: Scope,
}

/// What Experience decided about a signal.
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    Recall {
        query: String,
    },
    Retain(Write),
    /// A new version of an existing memory.
    Refine {
        id: i64,
        write: Write,
    },
    Forget {
        id: i64,
    },
    /// Considered and chose not to act, or the action failed.
    Ignore {
        reason: String,
    },
}

#[derive(Debug, Default)]
pub struct Outcome {
    pub signals: Vec<Signal>,
    pub decisions: Vec<Decision>,
    /// Brought into the thread's context.
    pub recalled: Vec<Item>,
    pub written: Vec<Item>,
}

#[derive(Debug, Default)]
pub struct Dream {
    pub episode: Option<Episode>,
    /// The thread's new card, when turns finished since the last one.
    pub card: Option<Card>,
    /// Memories replaced or forgotten while cleaning up.
    pub refined: Vec<Item>,
    pub compacted: bool,
}

pub struct Experience<A> {
    pub attention: Attention,
    memory: Arc<Memory>,
    provider: Provider<A>,
    request: Request,
    scopes: Scopes,
    /// Dreaming summarizes once this many finished turns have no episode yet.
    pub summarize_after_turns: usize,
    /// Dreaming compacts once the context holds more messages than this.
    pub compact_after_messages: usize,
    /// Compaction keeps at least this many recent messages as they are.
    pub keep_recent_messages: usize,
}

impl<A: Api> Experience<A> {
    /// `provider` and `request` are the strong model Experience thinks with.
    pub fn new(
        provider: Provider<A>,
        request: Request,
        memory: Arc<Memory>,
        scopes: Scopes,
    ) -> Self {
        Self {
            attention: Attention::default(),
            memory,
            provider,
            request,
            scopes,
            summarize_after_turns: 3,
            compact_after_messages: 40,
            keep_recent_messages: 10,
        }
    }

    pub fn memory(&self) -> &Memory {
        &self.memory
    }

    /// Call before each model request (after `start_step`). Attends to the checkpoints since it
    /// last looked; when something calls for it, decides, acts on memory, and records what it
    /// recalled into the thread. Errors only when the thread can't record.
    pub async fn consider(&self, thread: &mut Thread) -> Result<Outcome> {
        let upto = thread.checkpoints().len();
        let signals = self.attention.attend(thread, thread.considered_upto());
        if signals.is_empty() {
            return Ok(Outcome::default());
        }

        let mut planned = Vec::new();
        let mut queries: Vec<&str> = Vec::new();
        for signal in signals.iter().filter(|s| s.intent == Intent::Recall) {
            if !queries.contains(&signal.text.as_str()) {
                queries.push(&signal.text);
                planned.push(Decision::Recall {
                    query: signal.text.clone(),
                });
            }
        }
        let judged: Vec<&Signal> = signals
            .iter()
            .filter(|s| s.intent != Intent::Recall)
            .collect();
        if !judged.is_empty() {
            match self.decide(&judged, &thread.id).await {
                Ok(decisions) => planned.extend(decisions),
                Err(error) => planned.push(Decision::Ignore {
                    reason: format!("deciding failed: {error:#}"),
                }),
            }
        }

        let mut outcome = Outcome::default();
        let mut in_context = recalled_in_context(thread);
        for decision in planned {
            match self.execute(&decision, &mut in_context) {
                Ok((recalled, written)) => {
                    outcome.recalled.extend(recalled);
                    outcome.written.extend(written);
                    outcome.decisions.push(decision);
                }
                Err(error) => outcome.decisions.push(Decision::Ignore {
                    reason: format!("{decision:?} failed: {error:#}"),
                }),
            }
        }
        let recalled = outcome
            .recalled
            .iter()
            .map(|item| Recalled {
                id: item.id,
                text: item.body.text(),
            })
            .collect();
        thread.considered(upto, recalled)?;
        outcome.signals = signals;
        Ok(outcome)
    }

    /// Idle work, between turns: summarize finished turns into an episode, update the thread's
    /// card, clean up memory, and compact the context if it has grown large. Each part commits
    /// on its own, so dropping the future (a new message arrived) stops it at a safe point.
    pub async fn dream(&self, thread: &mut Thread) -> Result<Dream> {
        ensure!(!thread.in_turn(), "can't dream during a turn");
        Ok(Dream {
            episode: self.summarize(thread).await?,
            card: self.card(thread).await?,
            refined: self.refine().await?,
            compacted: self.compact(thread).await?,
        })
    }

    /// Rewrites the thread's card when turns finished since the last one.
    async fn card(&self, thread: &Thread) -> Result<Option<Card>> {
        let previous = self.memory.card(&thread.id)?;
        let start = previous.as_ref().map_or(0, |c| c.to + 1);
        let checkpoints = thread.checkpoints();
        let Some(end) = checkpoints
            .iter()
            .skip(start)
            .rev()
            .find(|c| c.event == Event::TurnEnded)
            .map(|c| c.id)
        else {
            return Ok(None);
        };
        let messages = checkpoints[start..=end]
            .iter()
            .filter_map(|c| match &c.event {
                Event::Message(m) => Some(m),
                _ => None,
            });
        let since = transcript(messages);
        let since = match since.char_indices().nth_back(CARD_INPUT_CHARS) {
            Some((cut, _)) => &since[cut..],
            None => &since,
        };
        let input = match &previous {
            Some(card) => format!("Current card:\n{}\n\nSince then:\n{since}", card.text),
            None => since.to_string(),
        };
        let text = self.ask(CARD_PROMPT, input).await?;
        Ok(Some(self.memory.add_card(
            &thread.id,
            self.scopes.write.clone(),
            text.trim(),
            end,
        )?))
    }

    /// One model call for every retain/refine signal, with the memories related to them.
    async fn decide(&self, signals: &[&Signal], thread: &str) -> Result<Vec<Decision>> {
        let mut scopes = vec![self.scopes.write.clone()];
        scopes.extend(self.scopes.read.iter().cloned());
        let mut related: HashMap<i64, Item> = HashMap::new();
        for signal in signals {
            for hit in self.memory.search(&signal.text, &scopes, RELATED_LIMIT)? {
                related.insert(hit.item.id, hit.item);
            }
        }
        let payload = json!({
            "signals": signals.iter().enumerate().map(|(i, s)| json!({
                "i": i,
                "intent": format!("{:?}", s.intent).to_lowercase(),
                "salience": s.salience,
                "source": format!("{:?}", s.source).to_lowercase(),
                "text": s.text,
                "reason": s.reason,
            })).collect::<Vec<_>>(),
            "related": related.values().map(describe).collect::<Vec<_>>(),
        });
        let reply = self.ask(DECIDE_PROMPT, payload.to_string()).await?;
        Ok(json_array(&reply)?
            .iter()
            .filter_map(|raw| {
                let signal = raw["signal"].as_u64().and_then(|i| signals.get(i as usize));
                let source = signal.map(|s| Source {
                    thread: thread.to_string(),
                    checkpoint: s.checkpoint,
                });
                to_decision(raw, &self.scopes.write, source, &related)
            })
            .collect())
    }

    /// Returns (recalled, written).
    fn execute(
        &self,
        decision: &Decision,
        in_context: &mut HashSet<i64>,
    ) -> Result<(Vec<Item>, Vec<Item>)> {
        Ok(match decision {
            Decision::Recall { query } => {
                let recalled = self
                    .memory
                    .search(query, &self.scopes.read, RECALL_LIMIT)?
                    .into_iter()
                    .map(|hit| hit.item)
                    .filter(|item| in_context.insert(item.id))
                    .collect();
                (recalled, vec![])
            }
            Decision::Retain(write) => (vec![], vec![self.memory.write(write.clone())?]),
            Decision::Refine { id, write } => {
                (vec![], vec![self.memory.replace(*id, write.clone())?])
            }
            Decision::Forget { id } => {
                self.memory.forget(*id)?;
                (vec![], vec![])
            }
            Decision::Ignore { .. } => (vec![], vec![]),
        })
    }

    /// An episode for the finished turns since the last one, once there are enough.
    async fn summarize(&self, thread: &Thread) -> Result<Option<Episode>> {
        let start = self
            .memory
            .episodes(&thread.id)?
            .last()
            .map_or(0, |e| e.to + 1);
        let ends: Vec<usize> = thread.checkpoints()[start.min(thread.checkpoints().len())..]
            .iter()
            .filter(|c| c.event == Event::TurnEnded)
            .map(|c| c.id)
            .collect();
        let Some(&end) = ends.last() else {
            return Ok(None);
        };
        if ends.len() < self.summarize_after_turns {
            return Ok(None);
        }
        let messages = thread.checkpoints()[start..=end]
            .iter()
            .filter_map(|c| match &c.event {
                Event::Message(m) => Some(m),
                _ => None,
            });
        let summary = self.ask(SUMMARIZE_PROMPT, transcript(messages)).await?;
        Ok(Some(self.memory.add_episode(
            &thread.id,
            self.scopes.write.clone(),
            start,
            end,
            summary.trim(),
        )?))
    }

    /// Drops exact duplicates in code, then lets the model merge or correct the rest.
    async fn refine(&self) -> Result<Vec<Item>> {
        let mut refined = Vec::new();
        let mut seen = HashSet::new();
        let mut remaining = Vec::new();
        // Newest first, so the newest copy of a duplicate is the one kept.
        for item in self
            .memory
            .list(std::slice::from_ref(&self.scopes.write), REFINE_LIMIT)?
        {
            let normal = format!("{}:{}", item.body.kind(), normalize(&item.body.text()));
            if seen.insert(normal) {
                remaining.push(item);
            } else {
                self.memory.forget(item.id)?;
                refined.push(item);
            }
        }
        if remaining.len() < 2 {
            return Ok(refined);
        }

        let payload: Vec<Value> = remaining.iter().map(describe).collect();
        let reply = self
            .ask(REFINE_PROMPT, serde_json::to_string(&payload)?)
            .await?;
        let by_id: HashMap<i64, Item> = remaining.into_iter().map(|i| (i.id, i)).collect();
        for raw in json_array(&reply)? {
            match to_decision(&raw, &self.scopes.write, None, &by_id) {
                Some(Decision::Forget { id }) => {
                    self.memory.forget(id)?;
                    refined.push(by_id[&id].clone());
                }
                // Skipped when an earlier decision already replaced or forgot it.
                Some(Decision::Refine { id, write })
                    if self.memory.get(id).is_ok_and(|item| item.is_some()) =>
                {
                    refined.push(self.memory.replace(id, write)?);
                }
                _ => {}
            }
        }
        Ok(refined)
    }

    /// Replaces older messages with a summary, keeping recent ones and whole turns intact.
    async fn compact(&self, thread: &mut Thread) -> Result<bool> {
        let messages = &thread.context().messages;
        if messages.len() <= self.compact_after_messages {
            return Ok(false);
        }
        let Some(cut) = compaction_cut(messages, self.keep_recent_messages) else {
            return Ok(false);
        };
        let summary = self
            .ask(COMPACT_PROMPT, transcript(messages[..cut].iter()))
            .await?;
        let summary = Message::user(format!("<summary>\n{}\n</summary>", summary.trim()));
        thread.replace(0..cut, vec![summary])?;
        Ok(true)
    }

    /// One stateless call to the strong model; never touches a thread.
    async fn ask(&self, system: &str, input: String) -> Result<String> {
        let mut ctx = Context::new(system);
        ctx.push(Message::user(input));
        Ok(self
            .provider
            .chat(&self.request, &ctx)
            .await?
            .message
            .text())
    }
}

/// Memories recalled since the context was last compacted, so they aren't recalled twice.
fn recalled_in_context(thread: &Thread) -> HashSet<i64> {
    let checkpoints = thread.checkpoints();
    let start = checkpoints
        .iter()
        .rposition(|c| matches!(c.event, Event::Replaced { .. }))
        .map_or(0, |i| i + 1);
    checkpoints[start..]
        .iter()
        .flat_map(|c| match &c.event {
            Event::Considered { recalled, .. } => recalled.iter().map(|r| r.id).collect(),
            _ => Vec::new(),
        })
        .collect()
}

/// Where to cut so the kept part starts at a turn's opening message and holds at least
/// `keep` messages. `None` when there is no such point past the start.
fn compaction_cut(messages: &[Message], keep: usize) -> Option<usize> {
    let opens_turn = |m: &Message| {
        m.role == Role::User
            && !m
                .content
                .iter()
                .any(|b| matches!(b, Block::ToolResult { .. }))
    };
    (1..=messages.len().saturating_sub(keep))
        .rev()
        .find(|&i| opens_turn(&messages[i]))
}

/// Messages as plain text, for summaries.
fn transcript<'a>(messages: impl Iterator<Item = &'a Message>) -> String {
    let mut out = Vec::new();
    for message in messages {
        let who = match message.role {
            Role::User => "User",
            Role::Assistant => "Assistant",
        };
        for block in &message.content {
            match block {
                Block::Text { text } => out.push(format!("{who}: {text}")),
                Block::ToolCall { name, input, .. } => {
                    out.push(format!("{who} called {name} {input}"))
                }
                Block::ToolResult {
                    content, is_error, ..
                } => out.push(format!(
                    "Tool {}: {content}",
                    if *is_error { "error" } else { "result" }
                )),
                Block::Thinking { .. } | Block::RedactedThinking { .. } => {}
            }
        }
    }
    out.join("\n")
}

/// A memory as the model sees it.
fn describe(item: &Item) -> Value {
    json!({ "id": item.id, "kind": item.body.kind(), "key": item.key, "text": item.body.text() })
}

fn normalize(text: &str) -> String {
    text.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The JSON array in a model's reply, ignoring any text around it.
fn json_array(reply: &str) -> Result<Vec<Value>> {
    let start = reply.find('[').context("no JSON array in reply")?;
    let end = reply.rfind(']').context("no JSON array in reply")?;
    serde_json::from_str(&reply[start..=end]).context("reply isn't a JSON array")
}

/// One raw decision from the model. Refine and forget only work on ids it was shown.
fn to_decision(
    raw: &Value,
    scope: &Scope,
    source: Option<Source>,
    shown: &HashMap<i64, Item>,
) -> Option<Decision> {
    let str_of = |k: &str| {
        raw[k]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .map(String::from)
    };
    let shown_item = || raw["id"].as_i64().and_then(|id| shown.get(&id));
    match raw["action"].as_str()? {
        "retain" => {
            let kind = raw["kind"].as_str().unwrap_or("semantic");
            let mut write = Write::new(scope.clone(), body(raw, kind, None)?);
            write.key = str_of("key");
            write.source = source;
            Some(Decision::Retain(write))
        }
        "refine" => {
            let old = shown_item()?;
            let mut write = Write::new(
                old.scope.clone(),
                body(raw, old.body.kind(), Some(&old.body))?,
            );
            write.key = old.key.clone();
            write.source = source.or_else(|| old.source.clone());
            Some(Decision::Refine { id: old.id, write })
        }
        "forget" => Some(Decision::Forget {
            id: shown_item()?.id,
        }),
        "ignore" => Some(Decision::Ignore {
            reason: str_of("reason").unwrap_or_default(),
        }),
        _ => None,
    }
}

/// A memory body from raw fields; a refine keeps old procedural fields it doesn't restate.
fn body(raw: &Value, kind: &str, old: Option<&Body>) -> Option<Body> {
    let str_of = |k: &str| {
        raw[k]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .map(String::from)
    };
    match kind {
        "semantic" => Some(Body::Semantic {
            text: str_of("text")?,
        }),
        "procedural" => {
            let (old_name, old_trigger, old_steps) = match old {
                Some(Body::Procedural {
                    name,
                    trigger,
                    steps,
                }) => (
                    Some(name.clone()),
                    Some(trigger.clone()),
                    Some(steps.clone()),
                ),
                _ => (None, None, None),
            };
            let steps = raw["steps"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str().map(String::from))
                        .collect()
                })
                .or(old_steps)?;
            Some(Body::Procedural {
                name: str_of("name").or(old_name)?,
                trigger: str_of("trigger").or(old_trigger)?,
                steps,
            })
        }
        _ => None,
    }
}

const DECIDE_PROMPT: &str = r#"You are the Experience layer of an AI agent: you decide what goes into long-term memory.
You receive "signals" (moments that may matter, found by pattern detectors) and "related" memories already stored.
Answer with ONLY a JSON array of decisions, one per action, each optionally with "signal": <index>:
  {"action":"retain","kind":"semantic","text":"<standalone fact>","key":"<optional stable key>","signal":0}
  {"action":"retain","kind":"procedural","name":"...","trigger":"<when it applies>","steps":["..."],"signal":0}
  {"action":"refine","id":<related id>,"text":"<corrected fact>","signal":0}
  {"action":"forget","id":<related id>,"signal":0}
  {"action":"ignore","reason":"...","signal":0}
Rules: retain only lasting facts, preferences or procedures, not one-off requests. Write facts as standalone
third-person statements ("The user prefers pnpm"), in the language the user used. When a signal corrects a
related memory, refine it instead of retaining a duplicate. Use a short key (e.g. "name", "package-manager")
for facts that are settings. When unsure, ignore."#;

const SUMMARIZE_PROMPT: &str = "Summarize these conversation turns as an episodic memory: what was asked, what was done, \
and how it ended, in 2-5 sentences, in the language of the conversation. Plain text only.";

const REFINE_PROMPT: &str = r#"You maintain an agent's long-term memory. Given these memories as JSON, find duplicates,
contradictions, or facts that should be merged. Answer with ONLY a JSON array:
  {"action":"refine","id":<id to keep>,"text":"<merged or corrected fact>"}
  {"action":"forget","id":<redundant id>}
Return [] when nothing needs changing."#;

const CARD_PROMPT: &str = "Write a card for this conversation: what it is about, so it can be found again later. \
One or two short lines naming the subject and the specific things in it (places, names, tools, decisions). \
When given the current card, update it with what happened since. Under 40 words. Plain text only.";

const COMPACT_PROMPT: &str = "Summarize the start of this conversation so it can continue without it: the user's goals, \
decisions made, facts and results that later messages may rely on, and anything still open. Be concrete and brief. \
Plain text only.";

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use inference::Effort;
    use inference::providers::Anthropic;

    const MODEL: &str = "claude-opus-5-5";

    pub fn me() -> Scope {
        Scope::user("jon")
    }

    pub fn experience(key: &str) -> Experience<Anthropic> {
        let scopes = Scopes {
            read: vec![me(), Scope::global()],
            write: me(),
        };
        Experience::new(
            Provider::anthropic(key),
            Request::new(MODEL).effort(Effort::Low),
            Arc::new(Memory::in_memory().unwrap()),
            scopes,
        )
    }

    pub fn turn(thread: &mut Thread, user: &str, reply: &str) {
        thread.start_turn(Message::user(user)).unwrap();
        thread.start_step().unwrap();
        thread.push(Message::assistant(reply)).unwrap();
        thread.end_turn().unwrap();
    }

    /// Recall is decided in code, so this makes no model call.
    #[tokio::test]
    async fn recalls_once_per_context() {
        let exp = experience("unused");
        let fact = exp
            .memory()
            .write(Write::fact(me(), "The user's name is Jon."))
            .unwrap();
        let mut thread = Thread::new("t1");
        thread.start_turn(Message::user("What's my name?")).unwrap();
        thread.start_step().unwrap();

        let outcome = exp.consider(&mut thread).await.unwrap();
        assert_eq!(outcome.recalled, [fact]);
        assert!(
            thread.context().messages[0]
                .text()
                .contains("<recalled>\n- The user's name is Jon.")
        );

        // Nothing new to attend to.
        assert!(exp.consider(&mut thread).await.unwrap().signals.is_empty());
        // Asked again: already in context, so not recalled twice.
        thread.push(Message::assistant("Jon.")).unwrap();
        thread.end_turn().unwrap();
        thread
            .start_turn(Message::user("Remind me, what's my name?"))
            .unwrap();
        let outcome = exp.consider(&mut thread).await.unwrap();
        assert!(!outcome.signals.is_empty());
        assert!(outcome.recalled.is_empty());
    }

    #[test]
    fn decisions_only_touch_shown_memories() {
        let memory = Memory::in_memory().unwrap();
        let old = memory
            .write(Write::fact(me(), "Deploys are daily.").key("deploys"))
            .unwrap();
        let shown = HashMap::from([(old.id, old.clone())]);
        let source = Source {
            thread: "t1".into(),
            checkpoint: 3,
        };

        let raw = json!({ "action": "refine", "id": old.id, "text": "Deploys are hourly." });
        let Some(Decision::Refine { id, write }) =
            to_decision(&raw, &me(), Some(source.clone()), &shown)
        else {
            panic!("expected a refine");
        };
        assert_eq!((id, write.key.as_deref()), (old.id, Some("deploys")));
        assert_eq!(write.body.text(), "Deploys are hourly.");

        let unknown = json!({ "action": "forget", "id": old.id + 1 });
        assert_eq!(to_decision(&unknown, &me(), None, &shown), None);

        let procedure = json!({ "action": "retain", "kind": "procedural", "name": "Ship",
            "trigger": "shipping", "steps": ["test", "tag"], "key": "ship" });
        let Some(Decision::Retain(write)) =
            to_decision(&procedure, &me(), Some(source.clone()), &shown)
        else {
            panic!("expected a retain");
        };
        assert_eq!(write.body.text(), "Ship (when shipping): test → tag");
        assert_eq!(write.source, Some(source));

        let reply = "Sure:\n```json\n[{\"action\":\"ignore\",\"reason\":\"one-off\"}]\n```";
        assert_eq!(json_array(reply).unwrap().len(), 1);
    }

    #[test]
    fn compaction_keeps_whole_turns() {
        let call = Message {
            role: Role::Assistant,
            content: vec![Block::ToolCall {
                id: "c".into(),
                name: "t".into(),
                input: json!({}),
            }],
        };
        let messages = vec![
            Message::user("a"),
            Message::assistant("b"),
            Message::user("c"),
            call,
            Message::tool_result("c", "ok"),
            Message::assistant("d"),
        ];
        // Keep 3: index 3 is a tool call, 4 a tool result; the last turn opens at 2.
        assert_eq!(compaction_cut(&messages, 3), Some(2));
        assert_eq!(compaction_cut(&messages, 5), None);
    }

    pub fn key() -> String {
        std::env::var("ANTHROPIC_API_KEY").expect("set ANTHROPIC_API_KEY")
    }

    /// Told in one thread, remembered in another.
    #[tokio::test]
    #[ignore = "calls Anthropic"]
    async fn remembers_across_threads() {
        let exp = experience(&key());

        let mut first = Thread::new("t1");
        first
            .start_turn(Message::user(
                "Hi! My name is Jon, and I prefer Rust for everything.",
            ))
            .unwrap();
        first.start_step().unwrap();
        let outcome = exp.consider(&mut first).await.unwrap();
        println!("decisions: {:#?}", outcome.decisions);
        let written: Vec<_> = outcome.written.iter().map(|i| i.body.text()).collect();
        println!("written: {written:?}");
        assert!(written.iter().any(|t| t.contains("Jon")));

        let mut second = Thread::new("t2");
        second.set_system("Be terse.").unwrap();
        second.start_turn(Message::user("What's my name?")).unwrap();
        second.start_step().unwrap();
        let outcome = exp.consider(&mut second).await.unwrap();
        assert!(
            outcome
                .recalled
                .iter()
                .any(|i| i.body.text().contains("Jon"))
        );

        let res = Provider::anthropic(&key())
            .chat(&Request::new(MODEL).effort(Effort::Low), second.context())
            .await
            .unwrap();
        println!("reply: {}", res.message.text());
        assert!(res.message.text().contains("Jon"));
    }

    #[tokio::test]
    #[ignore = "calls Anthropic"]
    async fn dreams() {
        let mut exp = experience(&key());
        exp.compact_after_messages = 4;
        exp.keep_recent_messages = 2;
        let memory = exp.memory();
        memory
            .write(Write::fact(me(), "The user's name is Jon."))
            .unwrap();
        memory
            .write(Write::fact(me(), "The user's  name is jon."))
            .unwrap();
        memory
            .write(Write::fact(me(), "The user uses npm."))
            .unwrap();
        memory
            .write(Write::fact(me(), "The user switched from npm to pnpm."))
            .unwrap();

        let mut thread = Thread::new("t1");
        turn(
            &mut thread,
            "Plan a trip to Kyoto in April.",
            "Cherry blossom season; book early.",
        );
        turn(
            &mut thread,
            "Find a ryokan near Gion.",
            "Try Gion Hatanaka, about ¥60,000 a night.",
        );
        turn(
            &mut thread,
            "Book it for April 3-5.",
            "Booked: Gion Hatanaka, April 3-5.",
        );

        let last_turn_end = thread.checkpoints().len() - 1;
        let dream = exp.dream(&mut thread).await.unwrap();
        let episode = dream.episode.expect("an episode");
        let card = dream.card.expect("a card");
        println!("card: {}", card.text);
        assert!(card.text.split_whitespace().count() < 60);
        assert!(card.text.contains("Kyoto"));
        assert_eq!(exp.memory().card("t1").unwrap(), Some(card));
        println!("episode: {}", episode.text);
        println!("refined: {:#?}", dream.refined);
        let left: Vec<_> = exp
            .memory()
            .list(&[me()], 10)
            .unwrap()
            .into_iter()
            .map(|i| i.body.text())
            .collect();
        println!("memory now: {left:?}");
        println!("context: {:#?}", thread.context().messages);

        assert_eq!((episode.from, episode.to), (0, last_turn_end));
        assert!(episode.text.contains("Kyoto") || episode.text.contains("Gion"));
        assert_eq!(
            left.iter()
                .filter(|t| t.to_lowercase().contains("jon"))
                .count(),
            1
        );
        assert!(!left.contains(&"The user uses npm.".to_string()));
        assert!(dream.compacted);
        assert!(thread.context().messages[0].text().starts_with("<summary>"));
    }
}
