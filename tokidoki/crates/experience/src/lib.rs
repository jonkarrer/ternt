//! Experience: the layer between State and Memory. It takes Attention's signals about a
//! thread, decides what to do about them, and carries that out against memory. While the
//! thread is idle, it dreams: summarizes episodes, cleans up memory, compacts the context.
//!
//!   recall           decided in code: search memory with the signal's text. Fast, no model.
//!   retain / refine  decided by the model: one call with every such signal and the memories
//!                    related to them, so it can correct a memory instead of duplicating it.
//!
//! With Jev (`sift`), a turn's opening message is judged first: which memories would help
//! answer it, whatever words they use, and whether it says something worth keeping that no
//! pattern caught. Jev's recall replaces the search's for that message; its retain is one
//! more signal for the model. A dream (`screen`) likewise asks the model for an episode only
//! when Jev finds something happened in the turns, and shows it only the memories Jev
//! suspects of repeating or contradicting each other. And what a signal means for memory
//! (`place`) is Jev's choice when it is sure, leaving the model only the wording.
//!
//! Nothing here may break a turn: a decision that fails becomes `Ignore` with the reason.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::{Context as _, Result, ensure};
use attention::{Attention, Intent, Signal};
use context::{Block, Context, Message, Role};
use inference::providers::Api;
use inference::{Jev, Provider, Request};
use memory::{Body, Card, Episode, Item, Memory, Scope, Source, Write};
use serde_json::{Value, json};
use state::{Checkpoint, Event, Recalled, Thread};

mod place;
mod screen;
pub mod settle;
mod sift;
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

impl Scopes {
    /// Every scope a search looks in.
    fn searched(&self) -> Vec<Scope> {
        let mut scopes = self.read.clone();
        if !scopes.contains(&self.write) {
            scopes.push(self.write.clone());
        }
        scopes
    }
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
    /// Considered and chose not to act.
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
    /// What went wrong: deciding (the model call), or carrying out a decision.
    pub failed: Vec<String>,
}

#[derive(Debug, Default)]
pub struct Dream {
    pub episode: Option<Episode>,
    /// The thread's new card, when turns finished since the last one.
    pub card: Option<Card>,
    /// Memories replaced or forgotten while cleaning up.
    pub refined: Vec<Item>,
    pub compacted: bool,
    /// Parts that failed, as (part, error). Each part runs on its own, so one failing (a model
    /// reply that won't parse, say) doesn't stop the others; it's tried again next dream.
    pub failed: Vec<(&'static str, String)>,
}

impl Dream {
    /// A part's result, or its default with the failure noted.
    fn keep<T: Default>(&mut self, part: &'static str, result: Result<T>) -> T {
        result.unwrap_or_else(|error| {
            self.failed.push((part, format!("{error:#}")));
            T::default()
        })
    }
}

pub struct Experience<A> {
    pub attention: Attention,
    memory: Arc<Memory>,
    provider: Provider<A>,
    request: Request,
    scopes: Scopes,
    /// Makes the small judgements, when the user has a TypeSafe key (`with_jev`): each turn's
    /// opening message (`sift`), and which memories a dream should look at (`screen`).
    jev: Option<Jev>,
    /// The newest memory a dream has screened: pairs of older ones aren't screened again.
    screened: std::sync::Mutex<i64>,
    /// Per thread, the turn end up to which Jev found nothing worth an episode: not asked
    /// again until more turns finish.
    uneventful: std::sync::Mutex<HashMap<String, usize>>,
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
            jev: None,
            screened: std::sync::Mutex::new(0),
            uneventful: Default::default(),
            summarize_after_turns: 3,
            compact_after_messages: 40,
            keep_recent_messages: 10,
        }
    }

    /// With Jev sifting each turn's opening message (`sift`) and screening memories for
    /// each dream's cleanup (`screen`).
    pub fn with_jev(mut self, jev: Jev) -> Self {
        self.jev = Some(jev);
        self
    }

    pub fn memory(&self) -> &Memory {
        &self.memory
    }

    /// Call before each model request (after `start_step`). Attends to the checkpoints since it
    /// last looked; when something calls for it, decides, acts on memory, and records what it
    /// recalled into the thread. Errors only when the thread can't record.
    #[tracing::instrument(skip_all)]
    pub async fn consider(&self, thread: &mut Thread) -> Result<Outcome> {
        let upto = thread.checkpoints().len();
        let mut signals = self.attention.attend(thread, thread.considered_upto());
        if signals.is_empty() {
            return Ok(Outcome::default());
        }
        let mut outcome = Outcome::default();
        let mut in_context = recalled_in_context(thread);
        // Jev's view of the message that opened the turn. Failing, it is as if there were
        // no Jev: the search recalls, the patterns alone say what to keep.
        let sifted = match &self.jev {
            Some(jev) => (self.sift(jev, &signals, &in_context).await).unwrap_or_else(|error| {
                outcome.failed.push(format!("sifting: {error:#}"));
                None
            }),
            None => None,
        };
        signals.extend(sifted.as_ref().and_then(|s| s.retain(&signals)));
        let sifted_text = sifted.as_ref().map(|s| s.text.as_str());

        let (recalls, judged): (Vec<&Signal>, Vec<&Signal>) =
            signals.iter().partition(|s| s.intent == Intent::Recall);
        let mut planned = Vec::new();
        // What Jev judged isn't searched for as well.
        for signal in recalls
            .into_iter()
            .filter(|s| Some(s.text.as_str()) != sifted_text)
        {
            let recall = Decision::Recall {
                query: signal.text.clone(),
            };
            if !planned.contains(&recall) {
                planned.push(recall);
            }
        }
        if !judged.is_empty() {
            match self.decide(&judged, &thread.id).await {
                Ok(decisions) => planned.extend(decisions),
                Err(error) => outcome.failed.push(format!("deciding: {error:#}")),
            }
        }

        if let Some(sifted) = sifted {
            outcome
                .decisions
                .push(Decision::Recall { query: sifted.text });
            let fresh = |item: &Item| in_context.insert(item.id);
            outcome
                .recalled
                .extend(sifted.relevant.into_iter().filter(fresh));
        }
        for decision in planned {
            match self.execute(&decision, &mut in_context, &mut outcome) {
                Ok(()) => outcome.decisions.push(decision),
                Err(error) => outcome.failed.push(format!("{decision:?}: {error:#}")),
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
        trace_outcome(&outcome);
        Ok(outcome)
    }

    /// Idle work, between turns: summarize finished turns into an episode, update the thread's
    /// card, clean up memory, and compact the context if it has grown large. Each part commits
    /// on its own, so dropping the future (a new message arrived) stops it at a safe point.
    #[tracing::instrument(skip_all)]
    pub async fn dream(&self, thread: &mut Thread) -> Result<Dream> {
        ensure!(!thread.in_turn(), "can't dream during a turn");
        let mut dream = Dream::default();
        // Each reads the thread and asks the model on its own: together.
        let (episode, card) = tokio::join!(self.summarize(thread), self.card(thread));
        dream.episode = dream.keep("summarize", episode);
        dream.card = dream.keep("card", card);
        dream.refined = dream.keep("refine", self.refine().await);
        dream.compacted = dream.keep("compact", self.compact(thread).await);
        for (part, error) in &dream.failed {
            tracing::warn!(part, error, "dream part failed");
        }
        tracing::info!(
            episode = ?dream.episode.as_ref().map(|e| &e.text),
            card = ?dream.card.as_ref().map(|c| &c.text),
            refined = ?dream.refined.iter().map(|i| (i.id, i.body.text())).collect::<Vec<_>>(),
            compacted = dream.compacted,
            failed = dream.failed.len(),
            "dreamed"
        );
        Ok(dream)
    }

    /// Rewrites the thread's card when turns finished since the last one.
    #[tracing::instrument(skip_all)]
    async fn card(&self, thread: &Thread) -> Result<Option<Card>> {
        let previous = self.memory.card(&thread.id)?;
        let previous = previous.as_ref().map(|c| (c.text.as_str(), c.to));
        let Some((end, input)) = card_input(thread, previous, "Current card", CARD_INPUT_CHARS)
        else {
            return Ok(None);
        };
        let text = self.ask(CARD_PROMPT, input).await?;
        Ok(Some(self.memory.add_card(
            &thread.id,
            self.scopes.write.clone(),
            text.trim(),
            end,
        )?))
    }

    /// What to do about the retain/refine signals: placed by Jev when there is one
    /// (`place`), else decided by the model.
    #[tracing::instrument(skip_all)]
    async fn decide(&self, signals: &[&Signal], thread: &str) -> Result<Vec<Decision>> {
        match &self.jev {
            Some(jev) => self.place(jev, signals, thread).await,
            None => self.decide_by_model(signals, thread).await,
        }
    }

    /// One model call for every retain/refine signal, with the memories related to them.
    async fn decide_by_model(&self, signals: &[&Signal], thread: &str) -> Result<Vec<Decision>> {
        let scopes = self.scopes.searched();
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

    /// Adds what it recalled or wrote to `outcome`.
    fn execute(
        &self,
        decision: &Decision,
        in_context: &mut HashSet<i64>,
        outcome: &mut Outcome,
    ) -> Result<()> {
        match decision {
            Decision::Recall { query } => outcome.recalled.extend(
                self.memory
                    .search(query, &self.scopes.searched(), RECALL_LIMIT)?
                    .into_iter()
                    .map(|hit| hit.item)
                    .filter(|item| in_context.insert(item.id)),
            ),
            Decision::Retain(write) => outcome.written.push(self.memory.write(write.clone())?),
            Decision::Refine { id, write } => outcome
                .written
                .push(self.memory.replace(*id, write.clone())?),
            Decision::Forget { id } => self.memory.forget(*id)?,
            Decision::Ignore { .. } => {}
        }
        Ok(())
    }

    /// An episode for the finished turns since the last one, once there are enough.
    #[tracing::instrument(skip_all)]
    async fn summarize(&self, thread: &Thread) -> Result<Option<Episode>> {
        let start = self
            .memory
            .episodes(&thread.id)?
            .last()
            .map_or(0, |e| e.to + 1);
        let ends = turn_ends(thread, start);
        let Some(&end) = ends.last() else {
            return Ok(None);
        };
        if ends.len() < self.summarize_after_turns {
            return Ok(None);
        }
        let turns = transcript(messages(&thread.checkpoints()[start..=end]));
        // Small talk gets no episode: with Jev, the model is asked only when something
        // happened. The turns stay unsummarized, and are judged again with the next ones.
        if let Some(jev) = &self.jev
            && !self.eventful(jev, &thread.id, end, &turns).await
        {
            return Ok(None);
        }
        let summary = self.ask(SUMMARIZE_PROMPT, turns).await?;
        Ok(Some(self.memory.add_episode(
            &thread.id,
            self.scopes.write.clone(),
            start,
            end,
            summary.trim(),
        )?))
    }

    /// Drops exact duplicates in code, then lets the model merge or correct the rest.
    #[tracing::instrument(skip_all)]
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
        // With Jev, the model sees only the memories Jev suspects of saying the same thing or
        // contradicting each other: usually none, and then it isn't asked at all. Failing,
        // it sees them all, as without Jev.
        if let Some(jev) = &self.jev {
            match self.suspects(jev, &remaining).await {
                Ok(suspects) => remaining = suspects,
                Err(error) => {
                    tracing::warn!(%error, "Jev couldn't screen memories; the model reads them all")
                }
            }
            if remaining.is_empty() {
                return Ok(refined);
            }
        }

        let payload: Vec<Value> = remaining.iter().map(describe).collect();
        let reply = self
            .ask(REFINE_PROMPT, serde_json::to_string(&payload)?)
            .await?;
        let by_id: HashMap<i64, Item> = remaining.into_iter().map(|i| (i.id, i)).collect();
        refined.extend(self.apply_refinements(&reply, &by_id)?);
        Ok(refined)
    }

    /// Carries out the refine and forget decisions in `reply` about the memories `shown`.
    fn apply_refinements(&self, reply: &str, shown: &HashMap<i64, Item>) -> Result<Vec<Item>> {
        // Not current once an earlier decision replaced or forgot it: skipped.
        let current = |id| self.memory.get(id).map(|item| item.is_some());
        let mut refined = Vec::new();
        for raw in json_array(reply)? {
            match to_decision(&raw, &self.scopes.write, None, shown) {
                Some(Decision::Forget { id }) if current(id)? => {
                    self.memory.forget(id)?;
                    refined.push(shown[&id].clone());
                }
                Some(Decision::Refine { id, write }) if current(id)? => {
                    refined.push(self.memory.replace(id, write)?);
                }
                _ => {}
            }
        }
        Ok(refined)
    }

    /// Replaces older messages with a summary, keeping recent ones and whole turns intact.
    #[tracing::instrument(skip_all)]
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

/// What attention noticed and what came of it; each failure is a warning.
fn trace_outcome(outcome: &Outcome) {
    let signal = |s: &Signal| {
        let intent = format!("{:?}", s.intent).to_lowercase();
        format!(
            "{intent} {:.2} #{} {}: {}",
            s.salience, s.checkpoint, s.detector, s.text
        )
    };
    for failure in &outcome.failed {
        tracing::warn!(failure, "consider failed");
    }
    tracing::info!(
        signals = ?outcome.signals.iter().map(signal).collect::<Vec<_>>(),
        decisions = ?outcome.decisions,
        recalled = ?outcome.recalled.iter().map(|i| (i.id, i.body.text())).collect::<Vec<_>>(),
        written = ?outcome.written.iter().map(|i| (i.id, i.body.text())).collect::<Vec<_>>(),
        "considered"
    );
}

/// Memories recalled into messages still in the context, so they aren't recalled twice. Follows
/// the context as State builds it: per message, its role and the memories recalled into it.
fn recalled_in_context(thread: &Thread) -> HashSet<i64> {
    let mut messages: Vec<(Role, Vec<i64>)> = Vec::new();
    for checkpoint in thread.checkpoints() {
        match &checkpoint.event {
            Event::Message(m) => messages.push((m.role, Vec::new())),
            Event::Replaced { range, with } => {
                messages.splice(range.clone(), with.iter().map(|m| (m.role, Vec::new())));
            }
            Event::Considered { recalled, .. } if !recalled.is_empty() => {
                let ids = recalled.iter().map(|r| r.id);
                match messages.last_mut() {
                    Some((Role::User, recalled_here)) => recalled_here.extend(ids),
                    _ => messages.push((Role::User, ids.collect())),
                }
            }
            _ => {}
        }
    }
    messages.into_iter().flat_map(|(_, ids)| ids).collect()
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
    (1..=messages.len().saturating_sub(keep.max(1)))
        .rev()
        .find(|&i| opens_turn(&messages[i]))
}

/// The checkpoints at which a turn ended, from `start` on.
fn turn_ends(thread: &Thread, start: usize) -> Vec<usize> {
    thread
        .checkpoints()
        .iter()
        .skip(start)
        .filter(|c| c.event == Event::TurnEnded)
        .map(|c| c.id)
        .collect()
}

fn messages(checkpoints: &[Checkpoint]) -> impl Iterator<Item = &Message> {
    checkpoints.iter().filter_map(|c| match &c.event {
        Event::Message(m) => Some(m),
        _ => None,
    })
}

/// For rewriting a card that covers checkpoints up to `previous`'s `to`: the last turn end since,
/// and the input: the card (headed `label`), then the newest `max` characters of what happened
/// since. `None` when no turn finished since.
fn card_input(
    thread: &Thread,
    previous: Option<(&str, usize)>,
    label: &str,
    max: usize,
) -> Option<(usize, String)> {
    let start = previous.map_or(0, |(_, to)| to + 1);
    let end = *turn_ends(thread, start).last()?;
    let since = last_chars(
        &transcript(messages(&thread.checkpoints()[start..=end])),
        max,
    );
    let input = match previous {
        Some((text, _)) => format!("{label}:\n{text}\n\nSince then:\n{since}"),
        None => since,
    };
    Some((end, input))
}

/// The last `max` characters, marked "…" when cut.
fn last_chars(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let tail: String = text.chars().skip(count - max).collect();
    format!("…{tail}")
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
    reply
        .match_indices('[')
        .find_map(|(start, _)| {
            serde_json::Deserializer::from_str(&reply[start..])
                .into_iter::<Vec<Value>>()
                .next()?
                .ok()
        })
        .context("no JSON array in reply")
}

/// One raw decision from the model. Refine and forget only work on ids it was shown.
fn to_decision(
    raw: &Value,
    scope: &Scope,
    source: Option<Source>,
    shown: &HashMap<i64, Item>,
) -> Option<Decision> {
    let shown_item = || raw["id"].as_i64().and_then(|id| shown.get(&id));
    match raw["action"].as_str()? {
        "retain" => {
            let kind = raw["kind"].as_str().unwrap_or("semantic");
            let mut write = Write::new(scope.clone(), body(raw, kind, None)?);
            write.key = field(raw, "key");
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
            reason: field(raw, "reason").unwrap_or_default(),
        }),
        _ => None,
    }
}

/// A non-blank string field.
fn field(raw: &Value, key: &str) -> Option<String> {
    raw[key]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .map(String::from)
}

/// A memory body from raw fields; a refine keeps old procedural fields it doesn't restate.
fn body(raw: &Value, kind: &str, old: Option<&Body>) -> Option<Body> {
    match kind {
        "semantic" => Some(Body::Semantic {
            text: field(raw, "text")?,
        }),
        "procedural" => {
            let (name, trigger, steps) = match old {
                Some(Body::Procedural {
                    name,
                    trigger,
                    steps,
                }) => (Some(name), Some(trigger), Some(steps)),
                _ => (None, None, None),
            };
            let new_steps = raw["steps"].as_array().map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str().map(String::from))
                    .collect()
            });
            Some(Body::Procedural {
                name: field(raw, "name").or_else(|| name.cloned())?,
                trigger: field(raw, "trigger").or_else(|| trigger.cloned())?,
                steps: new_steps.or_else(|| steps.cloned())?,
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

    // Tests that call a model run on a cheap one, always.
    const MODEL: &str = "claude-haiku-5-5";

    pub fn me() -> Scope {
        Scope::user("jon")
    }

    pub fn experience(key: &str) -> Experience<Anthropic> {
        experience_with(Provider::anthropic(key))
    }

    pub fn experience_with(provider: Provider<Anthropic>) -> Experience<Anthropic> {
        let scopes = Scopes {
            read: vec![me(), Scope::global()],
            write: me(),
        };
        let request = Request::new(MODEL).effort(Effort::Low);
        Experience::new(
            provider,
            request,
            Arc::new(Memory::in_memory().unwrap()),
            scopes,
        )
    }

    /// Experience over a model nobody answers.
    pub fn unreachable() -> Experience<Anthropic> {
        experience_with(Provider::new(
            Anthropic,
            "Nowhere",
            "http://127.0.0.1:9/",
            vec![],
        ))
    }

    pub fn turn(thread: &mut Thread, user: &str, reply: &str) {
        thread.start_turn(Message::user(user)).unwrap();
        thread.start_step().unwrap();
        thread.push(Message::assistant(reply)).unwrap();
        thread.end_turn().unwrap();
    }

    /// The model is unreachable: the parts that need it fail, the code-only cleanup still runs.
    #[tokio::test]
    async fn a_failing_dream_part_doesnt_stop_the_others() {
        let exp = unreachable();
        exp.memory()
            .write(Write::fact(me(), "The user's name is Jon."))
            .unwrap();
        exp.memory()
            .write(Write::fact(me(), "The user's name is Jon."))
            .unwrap();
        let mut thread = Thread::new("t1");
        for n in 0..3 {
            turn(&mut thread, &format!("question {n}"), "answer");
        }

        let dream = exp.dream(&mut thread).await.unwrap();
        let failed: Vec<&str> = dream.failed.iter().map(|(part, _)| *part).collect();
        assert_eq!(failed, ["summarize", "card"]);
        // The duplicate was dropped in code, without the model.
        assert_eq!(dream.refined.len(), 1);
        assert_eq!(exp.memory().list(&[me()], 10).unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_decision_that_cant_be_made_is_a_failure_not_a_choice() {
        let exp = unreachable();
        let mut thread = Thread::new("t1");
        thread
            .start_turn(Message::user("Remember that I always use pnpm, not npm."))
            .unwrap();
        thread.start_step().unwrap();

        let outcome = exp.consider(&mut thread).await.unwrap();
        assert!(!outcome.signals.is_empty());
        assert_eq!(outcome.failed.len(), 1, "{outcome:#?}");
        assert!(outcome.failed[0].starts_with("deciding:"), "{outcome:#?}");
        assert!(outcome.written.is_empty());
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

    /// `Scopes::write` is also searched: what was retained there can be recalled.
    #[tokio::test]
    async fn recalls_from_the_write_scope() {
        let mut exp = experience("unused");
        exp.scopes.read = vec![Scope::global()];
        let fact = exp
            .memory()
            .write(Write::fact(me(), "The user's name is Jon."))
            .unwrap();
        let mut thread = Thread::new("t1");
        thread.start_turn(Message::user("What's my name?")).unwrap();
        let outcome = exp.consider(&mut thread).await.unwrap();
        assert_eq!(outcome.recalled, [fact]);
    }

    /// Compaction that keeps the message a memory was recalled into doesn't make it recallable
    /// again; one that drops it does.
    #[tokio::test]
    async fn recalls_again_only_once_compacted_away() {
        let exp = experience("unused");
        let fact = exp
            .memory()
            .write(Write::fact(me(), "The user's name is Jon."))
            .unwrap();
        let mut thread = Thread::new("t1");
        turn(&mut thread, "Plan a trip to Kyoto.", "Sure.");
        thread.start_turn(Message::user("What's my name?")).unwrap();
        assert_eq!(exp.consider(&mut thread).await.unwrap().recalled, [fact]);
        thread.push(Message::assistant("Jon.")).unwrap();
        thread.end_turn().unwrap();

        // Drops the Kyoto turn, keeps the one holding the recalled memory.
        thread
            .replace(0..2, vec![Message::user("<summary>Kyoto</summary>")])
            .unwrap();
        thread
            .start_turn(Message::user("Remind me, what's my name?"))
            .unwrap();
        assert!(exp.consider(&mut thread).await.unwrap().recalled.is_empty());
        thread.end_turn().unwrap();

        let len = thread.context().messages.len();
        thread
            .replace(0..len, vec![Message::user("<summary>All</summary>")])
            .unwrap();
        thread
            .start_turn(Message::user("What's my name again?"))
            .unwrap();
        assert_eq!(exp.consider(&mut thread).await.unwrap().recalled.len(), 1);
    }

    /// Later decisions about a memory an earlier one already replaced or forgot are skipped.
    #[test]
    fn refinements_skip_what_was_already_handled() {
        let exp = experience("unused");
        let memory = exp.memory();
        let a = memory
            .write(Write::fact(me(), "Deploys are daily."))
            .unwrap();
        let b = memory
            .write(Write::fact(me(), "Deploys happen daily."))
            .unwrap();
        let shown = HashMap::from([(a.id, a.clone()), (b.id, b.clone())]);
        let reply = format!(
            r#"[{{"action":"refine","id":{a},"text":"Deploys are hourly."}},
                {{"action":"forget","id":{a}}},
                {{"action":"forget","id":{b}}},
                {{"action":"forget","id":{b}}}]"#,
            a = a.id,
            b = b.id
        );
        let refined = exp.apply_refinements(&reply, &shown).unwrap();
        let texts: Vec<_> = refined.iter().map(|i| i.body.text()).collect();
        assert_eq!(texts, ["Deploys are hourly.", "Deploys happen daily."]);
        let left: Vec<_> = memory.list(&[me()], 10).unwrap();
        assert_eq!(left, [refined[0].clone()]);
    }

    #[test]
    fn json_array_ignores_brackets_around_it() {
        let reply = "Decisions [2 of them]:\n[{\"action\":\"forget\",\"id\":1}, {\"action\":\"forget\",\"id\":2}]\nSee [1].";
        assert_eq!(json_array(reply).unwrap().len(), 2);
        assert!(json_array("no array [here]").is_err());
    }

    #[test]
    fn last_chars_keeps_exactly_max() {
        assert_eq!(last_chars("héllo wörld", 5), "…wörld");
        assert_eq!(last_chars("héllo", 5), "héllo");
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

    /// Keeping nothing still cuts at a turn's opening message, not past the end.
    #[test]
    fn compaction_keeping_nothing() {
        let messages = vec![
            Message::user("a"),
            Message::assistant("b"),
            Message::user("c"),
            Message::assistant("d"),
        ];
        assert_eq!(compaction_cut(&messages, 0), Some(2));
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
