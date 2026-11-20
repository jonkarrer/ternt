//! Threads from Experience's side. The user doesn't pick threads: each incoming message is
//! routed to an existing thread or a new one. A thread is hot while active, cool once idle past
//! `dream_after`, and settled once idle past `settle_after`, when its context is reset to a
//! resume card that holds just enough to pick it up again.
//!
//! Routing reads cards, not threads. Dreaming keeps a short card per thread on what it's about;
//! routing looks at the cards (and titles) of the 10 most recently active threads, in three
//! tiers: rules on Attention's route cues, then a keyword search over the cards, then the model
//! when neither is sure. When the user names a thread ("back to the Metabase thing"), every
//! card is searched too, so older threads can be found. Only the chosen thread is then loaded
//! in full (`Thread::open`). The one thread read directly is the hot one the user is in, which
//! hasn't dreamed a card yet:
//! its last few messages, to tell whether the user is still on its subject.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, SystemTime};

use anyhow::{Result, ensure};
use attention::{RouteCue, RouteSignal};
use context::Message;
use inference::providers::Api;
use memory::{Card, Resume};
use serde_json::Value;
use state::{Checkpointer, Event, Thread, ThreadInfo};

use crate::{Experience, transcript};

/// Routing considers this many of the most recently active threads.
const ROUTE_THREADS: usize = 10;
/// Older threads a named reference can bring in, from a search of every card.
const NAMED_THREADS: usize = 3;

/// "yes, do that" continues the last thread if it was active within this window.
const CONTINUATION_WINDOW: Duration = Duration::from_secs(24 * 3600);
/// Search picks a thread when its match covers at least this share of the query…
const SEARCH_MIN: f64 = 0.5;
/// …and beats the runner-up by this factor.
const SEARCH_MARGIN: f64 = 1.5;
/// Threads the model chooses among when rules and search aren't sure.
const ROUTE_CANDIDATES: usize = 5;
/// Messages read from the hot thread to tell whether the user is still on its subject.
const RECENT_MESSAGES: usize = 10;
/// A message to a hot thread with at least this many words, none shared with it, may be a new
/// subject.
const DRIFT_MIN_TERMS: usize = 3;
const RESUME_INPUT_CHARS: usize = 24_000;
/// Heads the context of a settled thread.
pub const RESUME_HANDOFF: &str =
    "Where this conversation left off (it went quiet for a while; pick it up from here):";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Hot,
    Cool,
    Settled,
}

#[derive(Debug, Clone)]
pub struct ThreadView {
    pub info: ThreadInfo,
    pub phase: Phase,
    /// When the user last talked in it; a settled thread keeps the time from its resume card.
    pub active_at: SystemTime,
    /// What it's about, from its last dream.
    pub card: Option<Card>,
    /// Where it left off, from when it last settled.
    pub resume: Option<Resume>,
}

/// A user message not yet placed in a thread.
#[derive(Debug, Clone)]
pub struct Incoming {
    pub text: String,
    pub now: SystemTime,
    /// The thread the user talked in last.
    pub last: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum By {
    Rule,
    Search,
    Model,
}

impl By {
    pub fn as_str(self) -> &'static str {
        match self {
            By::Rule => "rule",
            By::Search => "search",
            By::Model => "model",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Route {
    Continue {
        thread: String,
        by: By,
        reason: String,
    },
    New {
        by: By,
        reason: String,
    },
}

impl Route {
    fn cont(thread: &str, by: By, reason: impl Into<String>) -> Self {
        Route::Continue {
            thread: thread.to_string(),
            by,
            reason: reason.into(),
        }
    }

    fn new(by: By, reason: impl Into<String>) -> Self {
        Route::New {
            by,
            reason: reason.into(),
        }
    }

    pub fn by(&self) -> By {
        match self {
            Route::Continue { by, .. } | Route::New { by, .. } => *by,
        }
    }

    pub fn reason(&self) -> &str {
        match self {
            Route::Continue { reason, .. } | Route::New { reason, .. } => reason,
        }
    }
}

impl<A: Api> Experience<A> {
    /// Every thread with its phase and cards, most recently active first.
    pub fn threads(&self, store: &dyn Checkpointer, now: SystemTime) -> Result<Vec<ThreadView>> {
        store
            .threads()?
            .into_iter()
            .map(|info| {
                let card = self.memory.card(&info.id)?;
                let resume = self.memory.resume(&info.id)?;
                let settled = resume.as_ref().is_some_and(|r| r.turns == info.turns);
                let active_at = match &resume {
                    Some(resume) if settled => resume.active_at,
                    _ => info.active_at,
                };
                let phase = if settled {
                    Phase::Settled
                } else if elapsed(now, active_at) < self.dream_after {
                    Phase::Hot
                } else {
                    Phase::Cool
                };
                Ok(ThreadView {
                    info,
                    phase,
                    active_at,
                    card,
                    resume,
                })
            })
            .collect()
    }

    /// Where a message goes, from the threads' cards. Load the chosen thread with
    /// `Thread::open` and note the route in it with `thread.routed(...)`.
    pub async fn route(
        &self,
        store: &dyn Checkpointer,
        incoming: &Incoming,
        threads: &[ThreadView],
    ) -> Result<Route> {
        let signals = self.attention.notice(&incoming.text);
        let cue =
            |cue: RouteCue, min: f32| signals.iter().find(|s| s.cue == cue && s.salience >= min);
        let strong_new = cue(RouteCue::NewTopic, 0.8);
        let any_new = cue(RouteCue::NewTopic, 0.0).is_some();
        let reference = cue(RouteCue::ThreadReference, 0.0);
        let goes_on = cue(RouteCue::Continuation, 0.0);
        let last = incoming
            .last
            .as_ref()
            .and_then(|id| threads.iter().find(|t| t.info.id == *id));
        let recent_last = last.filter(|t| elapsed(incoming.now, t.active_at) < CONTINUATION_WINDOW);

        // 1. Rules.
        if let (Some(new), None) = (strong_new, reference) {
            return Ok(Route::new(By::Rule, &new.reason));
        }
        if let (Some(cont), false, None, Some(last)) = (goes_on, any_new, reference, recent_last) {
            return Ok(Route::cont(&last.info.id, By::Rule, &cont.reason));
        }
        if threads.is_empty() {
            return Ok(Route::new(By::Rule, "no earlier threads"));
        }
        // The most recent threads, and the last one even if it's older. When the user names a
        // thread, every card is searched for it, so an old thread can still be found.
        let mut pool: Vec<&ThreadView> = threads.iter().take(ROUTE_THREADS).collect();
        let mut named = Vec::new();
        if let Some(reference) = reference {
            for (id, _) in self
                .memory
                .search_cards(&reference.spanned(), NAMED_THREADS)?
            {
                named.extend(threads.iter().find(|t| t.info.id == id));
            }
        }
        for view in last.into_iter().chain(named) {
            if !pool.iter().any(|t| t.info.id == view.info.id) {
                pool.push(view);
            }
        }
        let docs: HashMap<String, String> = pool
            .iter()
            .map(|t| (t.info.id.clone(), card_doc(t)))
            .collect();

        let last_hot = last.filter(|t| t.phase == Phase::Hot && !any_new && reference.is_none());
        if let Some(last) = last_hot {
            // Stay, unless the message is self-contained and shares nothing with the talk so
            // far (a silent change of subject): then search and the model decide.
            let words = terms_of(&incoming.text);
            let mut talk = docs[&last.info.id].clone();
            for message in store.recent_messages(&last.info.id, RECENT_MESSAGES)? {
                talk.push('\n');
                talk.push_str(&message.text());
            }
            let talk = terms_of(&talk);
            if words.len() < DRIFT_MIN_TERMS || words.iter().any(|w| talk.contains(w)) {
                return Ok(Route::cont(
                    &last.info.id,
                    By::Rule,
                    "the conversation is still going",
                ));
            }
        }

        // 2. Search: the words naming a thread, else the whole message.
        let query = reference.map_or(incoming.text.clone(), RouteSignal::spanned);
        let ranked = rank_threads(&query, &pool, &docs);
        if let Some(&(best, score)) = ranked.first() {
            let second = ranked.get(1).map_or(0.0, |r| r.1);
            if score >= SEARCH_MIN && score >= SEARCH_MARGIN * second {
                let quoted: String = query.chars().take(60).collect();
                return Ok(Route::cont(
                    best,
                    By::Search,
                    format!("matches “{quoted}” ({score:.2})"),
                ));
            }
        }

        // 3. The model, with search hits first, then the last thread, then the most recent.
        let fallback = match last.filter(|t| t.phase == Phase::Hot && !any_new) {
            Some(last) => Route::cont(&last.info.id, By::Rule, "no clear match; staying"),
            None => Route::new(By::Rule, "no thread matches"),
        };
        let mut ids: Vec<&str> = ranked.iter().filter(|r| r.1 > 0.0).map(|r| r.0).collect();
        ids.extend(last.map(|t| t.info.id.as_str()));
        ids.extend(pool.iter().map(|t| t.info.id.as_str()));
        let mut seen = HashSet::new();
        let candidates: Vec<&ThreadView> = ids
            .into_iter()
            .filter(|id| seen.insert(*id))
            .filter_map(|id| pool.iter().find(|t| t.info.id == id).copied())
            .take(ROUTE_CANDIDATES)
            .collect();
        let request = route_request(incoming, last, &candidates, &docs);
        let answer = match self.ask(ROUTE_PROMPT, request).await {
            Ok(reply) => json_object(&reply),
            Err(_) => return Ok(fallback),
        };
        let Some(answer) = answer else {
            return Ok(fallback);
        };
        let reason = answer["reason"]
            .as_str()
            .unwrap_or("chosen by the model")
            .to_string();
        let chosen = match &answer["thread"] {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            _ => return Ok(fallback),
        };
        if chosen == "new" {
            return Ok(Route::new(By::Model, reason));
        }
        Ok(candidates
            .iter()
            .find(|t| t.info.id == chosen)
            .map_or(fallback, |t| Route::cont(&t.info.id, By::Model, reason)))
    }

    /// Idle work, after `settle_after`: writes the thread's resume card and resets its context
    /// to it. History keeps everything; `None` when nothing finished since the last card.
    pub async fn settle(&self, thread: &mut Thread) -> Result<Option<Resume>> {
        ensure!(!thread.in_turn(), "can't settle during a turn");
        let previous = self.memory.resume(&thread.id)?;
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
        let messages: Vec<&Message> = checkpoints[start..=end]
            .iter()
            .filter_map(|c| match &c.event {
                Event::Message(m) => Some(m),
                _ => None,
            })
            .collect();
        let active_at = checkpoints[..=end]
            .iter()
            .rev()
            .find(|c| matches!(c.event, Event::Message(_)))
            .map_or(checkpoints[end].at, |c| c.at);

        let since = last_chars(&transcript(messages.into_iter()), RESUME_INPUT_CHARS);
        let input = match &previous {
            Some(card) => format!("Previous card:\n{}\n\nSince then:\n{since}", card.text),
            None => since,
        };
        let text = self.ask(RESUME_PROMPT, input).await?;
        let resume = self.memory.add_resume(
            &thread.id,
            self.scopes.write.clone(),
            text.trim(),
            end,
            thread.turn(),
            active_at,
        )?;
        let len = thread.context().messages.len();
        let handoff = Message::user(format!("{RESUME_HANDOFF}\n{}", resume.text));
        thread.replace(0..len, vec![handoff])?;
        Ok(Some(resume))
    }
}

/// What routing knows about a thread: its title and card.
fn card_doc(view: &ThreadView) -> String {
    match &view.card {
        Some(card) => format!("{}\n{}", view.info.title, card.text),
        None => view.info.title.clone(),
    }
}

fn elapsed(now: SystemTime, at: SystemTime) -> Duration {
    now.duration_since(at).unwrap_or_default()
}

fn ago(d: Duration) -> String {
    let minutes = d.as_secs() / 60;
    match minutes {
        m if m < 60 => format!("{m} min ago"),
        m if m < 48 * 60 => format!("{} h ago", m / 60),
        m => format!("{} days ago", m / (24 * 60)),
    }
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

fn route_request(
    incoming: &Incoming,
    last: Option<&ThreadView>,
    candidates: &[&ThreadView],
    docs: &HashMap<String, String>,
) -> String {
    let threads: Vec<String> = candidates
        .iter()
        .map(|t| {
            format!(
                "[{}] {} (last active {})\n{}",
                t.info.id,
                t.info.title,
                ago(elapsed(incoming.now, t.active_at)),
                last_chars(&docs[&t.info.id], 1200)
            )
        })
        .collect();
    let last_line = last.map_or(String::new(), |t| {
        format!(
            "The user last talked in thread [{}], {}.\n",
            t.info.id,
            ago(elapsed(incoming.now, t.active_at))
        )
    });
    format!(
        "Message: {}\n{last_line}\nThreads:\n{}",
        incoming.text,
        threads.join("\n\n")
    )
}

/// The JSON object in a model's reply, ignoring any text around it.
fn json_object(reply: &str) -> Option<Value> {
    let start = reply.find('{')?;
    let end = reply.rfind('}')?;
    serde_json::from_str(&reply[start..=end]).ok()
}

/// Too common to tell threads apart: stopwords, and words people use to come back to one.
const STOP: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "but", "by", "can", "could", "did", "do", "does",
    "for", "from", "had", "has", "have", "how", "i", "if", "in", "into", "is", "it", "its", "me",
    "my", "no", "not", "of", "on", "or", "our", "please", "so", "that", "the", "their", "them",
    "then", "there", "these", "they", "this", "to", "us", "was", "we", "were", "what", "when",
    "where", "which", "who", "why", "will", "with", "would", "you", "your", "about", "back",
    "thing", "stuff",
];

/// Search terms: words, lightly stemmed, without stopwords.
fn terms_of(text: &str) -> HashSet<String> {
    let lower = text.to_lowercase();
    lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| word.chars().count() >= 2 && !STOP.contains(word))
        .map(|word| {
            ["ing", "ed", "es", "s"]
                .iter()
                .find_map(|suffix| word.strip_suffix(suffix).filter(|s| !s.is_empty()))
                .unwrap_or(word)
                .to_string()
        })
        .collect()
}

/// Threads by how much of the query they cover, rare terms weighted higher (IDF over the
/// threads). Scores 0–1, best first; ties keep the given order (most recent first).
fn rank_threads<'a>(
    query: &str,
    threads: &[&'a ThreadView],
    docs: &HashMap<String, String>,
) -> Vec<(&'a str, f64)> {
    let query = terms_of(query);
    if query.is_empty() || threads.is_empty() {
        return Vec::new();
    }
    let terms: Vec<(&str, HashSet<String>)> = threads
        .iter()
        .map(|t| (t.info.id.as_str(), terms_of(&docs[&t.info.id])))
        .collect();
    let n = terms.len() as f64;
    let idf: HashMap<&String, f64> = query
        .iter()
        .map(|t| {
            let df = terms.iter().filter(|(_, set)| set.contains(t)).count() as f64;
            (t, (1.0 + n / if df == 0.0 { 0.5 } else { df }).ln())
        })
        .collect();
    let total: f64 = idf.values().sum();
    let mut ranked: Vec<(&str, f64)> = terms
        .iter()
        .map(|(id, set)| {
            let covered: f64 = query
                .iter()
                .filter(|t| set.contains(*t))
                .map(|t| idf[t])
                .sum();
            (*id, covered / total)
        })
        .collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
    ranked
}

const ROUTE_PROMPT: &str = r#"You route a user's message to the conversation it belongs to. The user talks to one assistant;
their earlier conversations are listed as threads. Pick the thread this message continues, or "new" when it starts
something none of them covers. Choose a thread only when the message clearly continues it (same subject, a
follow-up, or it refers to it). Answer with ONLY JSON: {"thread":"<thread id>" or "new","reason":"<a few words>"}"#;

const RESUME_PROMPT: &str =
    "Write a resume card for this conversation, so it can be picked up later from the card alone.
Use these headings, in the language of the conversation:
About: <one line>
Where we left off: <the last thing being worked on, and its state>
Open loops: <questions, next steps or promises still pending; \"none\" if none>
Key details: <names, numbers, decisions, links and preferences needed to continue>
Be concrete and brief (under 200 words). Plain text only.";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{experience, key, turn};
    use context::Role;
    use state::Sqlite;
    use std::sync::Arc;

    fn store_with(threads: &[(&str, &[(&str, &str)])]) -> Arc<Sqlite> {
        let store = Arc::new(Sqlite::memory().unwrap());
        for (id, turns) in threads {
            let mut thread = Thread::open(*id, store.clone()).unwrap();
            for (user, reply) in *turns {
                turn(&mut thread, user, reply);
            }
        }
        store
    }

    fn incoming(text: &str, last: Option<&str>, later: Duration) -> Incoming {
        Incoming {
            text: text.into(),
            now: SystemTime::now() + later,
            last: last.map(String::from),
        }
    }

    const KYOTO: (&str, &[(&str, &str)]) = (
        "kyoto",
        &[
            (
                "Plan a trip to Kyoto in April.",
                "Cherry blossom season; book early.",
            ),
            ("Find a ryokan near Gion.", "Try Gion Hatanaka."),
        ],
    );
    const METABASE: (&str, &[(&str, &str)]) = (
        "metabase",
        &[(
            "Set up Metabase with Postgres.",
            "Use the Postgres connector and a read-only user.",
        )],
    );

    /// Rules and search: no model calls.
    #[tokio::test]
    async fn rules_and_search() {
        let exp = experience("unused");
        let store = store_with(&[KYOTO, METABASE]);
        let route = |text: &str, last: Option<&str>, later: Duration| {
            let threads = exp
                .threads(store.as_ref(), SystemTime::now() + later)
                .unwrap();
            let incoming = incoming(text, last, later);
            let store = store.clone();
            let exp = &exp;
            async move {
                exp.route(store.as_ref(), &incoming, &threads)
                    .await
                    .unwrap()
            }
        };
        let now = Duration::ZERO;
        let hours = |h: u64| Duration::from_secs(h * 3600);

        assert_eq!(
            route("New topic: what's a good pasta recipe?", Some("kyoto"), now).await,
            Route::new(By::Rule, "starts a new topic")
        );
        assert_eq!(
            route("yes, do it", Some("kyoto"), now).await,
            Route::cont(
                "kyoto",
                By::Rule,
                "only makes sense in the last conversation"
            )
        );
        // Hot and on topic: stays without searching.
        assert_eq!(
            route("Is Gion walkable from the station?", Some("kyoto"), now).await,
            Route::cont("kyoto", By::Rule, "the conversation is still going")
        );
        // Cool, and names another thread: search finds it.
        let back = route("Back to the Metabase thing.", Some("kyoto"), hours(3)).await;
        assert_eq!(
            (back.by(), back.reason()),
            (By::Search, "matches “Metabase” (1.00)")
        );
        assert!(matches!(back, Route::Continue { thread, .. } if thread == "metabase"));

        let empty = Arc::new(Sqlite::memory().unwrap());
        let incoming = incoming("Hello", None, now);
        assert_eq!(
            exp.route(empty.as_ref(), &incoming, &[]).await.unwrap(),
            Route::new(By::Rule, "no earlier threads")
        );
    }

    #[test]
    fn phases() {
        let exp = experience("unused");
        let store = store_with(&[KYOTO]);
        let phase = |later| {
            exp.threads(store.as_ref(), SystemTime::now() + later)
                .unwrap()[0]
                .phase
        };
        assert_eq!(phase(Duration::ZERO), Phase::Hot);
        assert_eq!(phase(Duration::from_secs(31 * 60)), Phase::Cool);

        let now = SystemTime::now();
        exp.memory()
            .add_resume("kyoto", crate::tests::me(), "About: Kyoto.", 9, 2, now)
            .unwrap();
        assert_eq!(phase(Duration::ZERO), Phase::Settled);
        // A new turn makes it active again.
        let mut thread = Thread::open("kyoto", store.clone()).unwrap();
        turn(&mut thread, "Actually, make it May.", "Updated to May.");
        assert_eq!(phase(Duration::ZERO), Phase::Hot);
    }

    /// Search reads cards, and only the 10 most recently active threads'.
    #[tokio::test]
    async fn routes_on_cards_of_recent_threads() {
        let exp = experience("unused");
        let store = Arc::new(Sqlite::memory().unwrap());
        // Oldest, so outside the 10. Its title mentions a ryokan, but only its card is
        // searched when named, and the card is about Metabase.
        let mut old = Thread::open("old", store.clone()).unwrap();
        turn(&mut old, "Find a ryokan near Gion.", "Try Gion Hatanaka.");
        turn(&mut old, "Now set up Metabase.", "Done, on Postgres.");
        exp.memory()
            .add_card(
                "old",
                crate::tests::me(),
                "Metabase dashboards on Postgres.",
                7,
            )
            .unwrap();
        for i in 0..10 {
            let mut filler = Thread::open(format!("filler-{i}"), store.clone()).unwrap();
            turn(&mut filler, &format!("Question {i}"), "Answer.");
        }
        // Its title doesn't mention the ryokan; its card does.
        let mut trip = Thread::open("trip", store.clone()).unwrap();
        turn(&mut trip, "Help me plan April.", "Sure.");
        exp.memory()
            .add_card(
                "trip",
                crate::tests::me(),
                "Kyoto in April: picking a ryokan near Gion.",
                3,
            )
            .unwrap();

        let later = Duration::from_secs(3 * 3600);
        let threads = exp
            .threads(store.as_ref(), SystemTime::now() + later)
            .unwrap();
        let incoming = incoming("Back to the ryokan thing.", None, later);
        let route = exp
            .route(store.as_ref(), &incoming, &threads)
            .await
            .unwrap();
        assert_eq!(
            route,
            Route::cont("trip", By::Search, "matches “ryokan” (1.00)")
        );

        // Named, so every card is searched: the old thread is found.
        let incoming = self::incoming("Back to the Metabase thing.", None, later);
        let route = exp
            .route(store.as_ref(), &incoming, &threads)
            .await
            .unwrap();
        assert_eq!(
            route,
            Route::cont("old", By::Search, "matches “Metabase” (1.00)")
        );
    }

    #[test]
    fn terms_match_across_forms() {
        let terms = terms_of("Booking the trips to Kyoto");
        for t in ["book", "trip", "kyoto"] {
            assert!(terms.contains(t), "{t} in {terms:?}");
        }
        assert!(!terms.contains("the"));
    }

    /// A paraphrase shares no words with the thread, so search misses; the model doesn't.
    #[tokio::test]
    #[ignore = "calls Anthropic"]
    async fn model_routes_paraphrases() {
        let exp = experience(&key());
        let store = store_with(&[KYOTO, METABASE]);
        let later = Duration::from_secs(3 * 3600);
        let threads = exp
            .threads(store.as_ref(), SystemTime::now() + later)
            .unwrap();
        let incoming = incoming(
            "Which hotel did we pick for Japan?",
            Some("metabase"),
            later,
        );
        let route = exp
            .route(store.as_ref(), &incoming, &threads)
            .await
            .unwrap();
        println!("{route:?}");
        assert!(
            matches!(&route, Route::Continue { thread, by: By::Model, .. } if thread == "kyoto")
        );
    }

    #[tokio::test]
    #[ignore = "calls Anthropic"]
    async fn settles_into_a_card() {
        let exp = experience(&key());
        let store = store_with(&[KYOTO]);
        let mut thread = Thread::open("kyoto", store.clone()).unwrap();
        let card = exp
            .settle(&mut thread)
            .await
            .unwrap()
            .expect("a resume card");
        println!("{}", card.text);

        assert_eq!((card.to, card.turns), (thread.checkpoints().len() - 2, 2));
        assert!(card.text.contains("Gion"));
        let messages = &thread.context().messages;
        assert_eq!(messages.len(), 1);
        assert!(messages[0].text().starts_with(RESUME_HANDOFF));
        assert_eq!(messages[0].role, Role::User);
        let view = &exp.threads(store.as_ref(), SystemTime::now()).unwrap()[0];
        assert_eq!(view.phase, Phase::Settled);
        // Nothing new since: no second card.
        assert!(exp.settle(&mut thread).await.unwrap().is_none());
    }
}
