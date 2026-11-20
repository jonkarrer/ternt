//! The router decides where each input goes before anything else sees it: which thread of
//! work (continue one, start one) and, later, what kind of work it is.
//!
//! Inputs of any kind reach it as text: an input step in front of it turns audio into a
//! transcript and an image into a description, and the original goes into the thread's context.
//!
//! It reads thread cards, not threads: the cards and titles of the 10 most recently active
//! threads, plus any older thread whose card matches a thread the user names. Only the chosen
//! thread is then loaded in full (`Thread::load`); the one thread read directly is the hot one
//! the user is in, which hasn't dreamed a card yet: its last few messages.
//!
//! A model call on every input would be expensive, so it decides in code first, in tiers, and
//! stops at the first that is sure:
//!
//! 1. Cues: "new topic", "yes, do it". Pattern rules.
//! 2. Continuity: the user is in a hot thread and the input shares words with it.
//! 3. Search: keyword coverage of the cards (IDF), decisive by a margin; or two searches
//!    (coverage, and BM25 over every card) agreeing on one thread.
//! 4. Nothing in common: the input shares no word with any card and doesn't look back at
//!    anything, so it's a new thread.
//! 5. Classifier: a trained model run locally (e.g. a fine-tuned BERT), when one is set.
//! 6. Model: a fast model, only for what's left: inputs with some evidence for a thread, or
//!    that look back at something no card matches.

pub mod cues;
pub mod search;

pub use cues::{RouteCue, RouteDetector, RouteSignal};

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use anyhow::Result;
use context::{Context, Message};
use inference::providers::Api;
use inference::{Provider, Request};
use memory::{Card, Memory, Resume, Scope};
use serde_json::Value;
use state::{Checkpointer, ThreadInfo};

use search::{rank_threads, terms_of};

/// Routing considers this many of the most recently active threads.
const ROUTE_THREADS: usize = 10;
/// Older threads a named reference, or the all-cards search, can bring in.
const OLDER_THREADS: usize = 3;
/// "yes, do that" continues the last thread if it was active within this window.
const CONTINUATION_WINDOW: Duration = Duration::from_secs(24 * 3600);
/// Search picks a thread when its match covers at least this share of the input…
const SEARCH_MIN: f64 = 0.5;
/// …and beats the runner-up by this factor.
const SEARCH_MARGIN: f64 = 1.5;
/// Coverage this high is enough when the all-cards search agrees on the same thread.
const AGREE_MIN: f64 = 0.25;
/// Threads the model chooses among.
const ROUTE_CANDIDATES: usize = 5;
/// Messages read from the hot thread to tell whether the user is still on its subject.
const RECENT_MESSAGES: usize = 10;
/// An input to a hot thread with at least this many words, none shared with it, may be a new
/// subject.
const DRIFT_MIN_TERMS: usize = 3;

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

/// An input not yet placed in a thread, as text.
#[derive(Debug, Clone)]
pub struct Input {
    pub text: String,
    pub now: SystemTime,
    /// The thread the user was in last.
    pub last: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Continue(String),
    New,
}

/// What kind of work an input is. Only chat for now; browsing, research, coding, scheduling
/// come with the tools and workers that handle them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Work {
    Chat,
}

/// Which tier decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum By {
    Rule,
    Search,
    Classifier,
    Model,
}

impl By {
    pub fn as_str(self) -> &'static str {
        match self {
            By::Rule => "rule",
            By::Search => "search",
            By::Classifier => "classifier",
            By::Model => "model",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    pub target: Target,
    pub work: Work,
    pub by: By,
    pub reason: String,
}

impl Route {
    pub fn cont(thread: &str, by: By, reason: impl Into<String>) -> Self {
        Self {
            target: Target::Continue(thread.to_string()),
            work: Work::Chat,
            by,
            reason: reason.into(),
        }
    }

    pub fn new(by: By, reason: impl Into<String>) -> Self {
        Self {
            target: Target::New,
            work: Work::Chat,
            by,
            reason: reason.into(),
        }
    }
}

/// A trained classifier run locally, between search and the model: cheap per input once
/// trained. The router's logged decisions (`Event::Routed` in each thread) are its training data.
pub trait Classifier: Send + Sync {
    /// The candidate this input continues (`None` for a new thread) and a confidence 0–1;
    /// `None` when it can't tell.
    fn classify(&self, input: &Input, candidates: &[&ThreadView]) -> Option<(Option<String>, f32)>;
}

pub struct Router<A> {
    pub cues: Vec<Box<dyn RouteDetector>>,
    pub classifier: Option<Box<dyn Classifier>>,
    /// A classifier answer at least this confident is taken.
    pub classifier_min: f32,
    memory: Arc<Memory>,
    /// Whose thread cards it searches: the same scopes Experience writes cards to.
    scopes: Vec<Scope>,
    provider: Provider<A>,
    request: Request,
    /// Idle time after which a thread is cool: keep it equal to the agent's `dream_after_ms`,
    /// when the thread dreams. (Settled comes from the resume card, not from a timer.)
    pub dream_after: Duration,
}

impl<A: Api> Router<A> {
    /// `provider` and `request` are the fast model for the last tier; `scopes` are whose
    /// thread cards it searches.
    pub fn new(
        provider: Provider<A>,
        request: Request,
        memory: Arc<Memory>,
        scopes: Vec<Scope>,
    ) -> Self {
        Self {
            cues: cues::defaults(),
            classifier: None,
            classifier_min: 0.8,
            memory,
            scopes,
            provider,
            request,
            dream_after: Duration::from_secs(30 * 60),
        }
    }

    /// Every thread with its phase and cards, most recently active first.
    pub fn threads(&self, store: &dyn Checkpointer, now: SystemTime) -> Result<Vec<ThreadView>> {
        store
            .threads()?
            .into_iter()
            .map(|info| {
                let card = self.memory.card(&info.id)?;
                let resume = self.memory.resume(&info.id)?;
                // Settled until a turn after the resume card.
                let settled = resume.as_ref().filter(|r| r.turns == info.turns);
                let active_at = settled.map_or(info.active_at, |r| r.active_at);
                let phase = if settled.is_some() {
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

    /// Where an input goes. Load the chosen thread with `Thread::load` and note the route in it
    /// with `thread.routed(route.by.as_str(), &route.reason)`.
    pub async fn route(
        &self,
        store: &dyn Checkpointer,
        input: &Input,
        threads: &[ThreadView],
    ) -> Result<Route> {
        let signals = cues::notice(&self.cues, &input.text);
        let cue =
            |cue: RouteCue, min: f32| signals.iter().find(|s| s.cue == cue && s.salience >= min);
        let strong_new = cue(RouteCue::NewTopic, 0.8);
        let any_new = cue(RouteCue::NewTopic, 0.0).is_some();
        let reference = cue(RouteCue::ThreadReference, 0.0);
        let goes_on = cue(RouteCue::Continuation, 0.0);
        let looks_back = reference.is_some() || cue(RouteCue::PastReference, 0.0).is_some();
        let last = input
            .last
            .as_ref()
            .and_then(|id| threads.iter().find(|t| t.info.id == *id));

        // 1. Cues.
        if let Some(new) = strong_new
            && reference.is_none()
        {
            return Ok(Route::new(By::Rule, &new.reason));
        }
        if let Some(cont) = goes_on
            && !any_new
            && reference.is_none()
            && let Some(last) =
                last.filter(|t| elapsed(input.now, t.active_at) < CONTINUATION_WINDOW)
        {
            return Ok(Route::cont(&last.info.id, By::Rule, &cont.reason));
        }
        if threads.is_empty() {
            return Ok(Route::new(By::Rule, "no earlier threads"));
        }

        // 2. Continuity: stay in the hot thread unless the input is self-contained and shares
        // nothing with it (a silent change of subject).
        let last_hot = last.filter(|t| t.phase == Phase::Hot && !any_new);
        if let Some(last) = last_hot.filter(|_| reference.is_none())
            && still_on_subject(store, last, &input.text)?
        {
            return Ok(Route::cont(
                &last.info.id,
                By::Rule,
                "the conversation is still going",
            ));
        }

        // The recent threads, the last one, and older ones whose cards match: a named thread,
        // or else the whole input.
        let query = reference.map_or(input.text.clone(), RouteSignal::spanned);
        let all_cards = self
            .memory
            .search_cards(&query, &self.scopes, OLDER_THREADS)?;
        let older = all_cards
            .iter()
            .filter_map(|(id, _)| threads.iter().find(|t| t.info.id == *id));
        let mut pool: Vec<&ThreadView> = threads.iter().take(ROUTE_THREADS).collect();
        for view in last.into_iter().chain(older) {
            if !pool.iter().any(|t| t.info.id == view.info.id) {
                pool.push(view);
            }
        }

        // 3. Search: decisive coverage, or coverage and BM25 agreeing.
        let ranked = rank_threads(&query, &pool);
        let quoted: String = query.chars().take(60).collect();
        if let Some(&(best, score)) = ranked.first() {
            let second = ranked.get(1).map_or(0.0, |r| r.1);
            if score >= SEARCH_MIN && score >= SEARCH_MARGIN * second {
                return Ok(Route::cont(
                    best,
                    By::Search,
                    format!("matches “{quoted}” ({score:.2})"),
                ));
            }
            let bm25_best = all_cards.first().map(|(id, _)| id.as_str());
            if score >= AGREE_MIN && score > second && bm25_best == Some(best) {
                return Ok(Route::cont(
                    best,
                    By::Search,
                    format!("both searches match “{quoted}” ({score:.2})"),
                ));
            }
        }

        // 4. Nothing in common, and not looking back at anything: new.
        let evidence: Vec<&str> = ranked
            .iter()
            .filter(|r| r.1 > 0.0)
            .map(|r| r.0)
            .chain(all_cards.iter().map(|(id, _)| id.as_str()))
            .collect();
        if evidence.is_empty() && !looks_back {
            return Ok(Route::new(By::Rule, "shares nothing with any thread"));
        }

        // Candidates for the last tiers: evidence first, then the last thread, then the most
        // recent.
        let fallback = match last_hot {
            Some(last) => Route::cont(&last.info.id, By::Rule, "no clear match; staying"),
            None => Route::new(By::Rule, "no thread matches"),
        };
        let mut seen = HashSet::new();
        let candidates: Vec<&ThreadView> = evidence
            .into_iter()
            .chain(last.map(|t| t.info.id.as_str()))
            .chain(pool.iter().map(|t| t.info.id.as_str()))
            .filter(|id| seen.insert(*id))
            .filter_map(|id| pool.iter().find(|t| t.info.id == id).copied())
            .take(ROUTE_CANDIDATES)
            .collect();

        // 5. Classifier.
        if let Some((choice, confidence)) = self
            .classifier
            .as_ref()
            .and_then(|c| c.classify(input, &candidates))
            .filter(|(_, confidence)| *confidence >= self.classifier_min)
        {
            let reason = format!("classified ({confidence:.2})");
            return Ok(match choice {
                Some(id) if candidates.iter().any(|t| t.info.id == id) => {
                    Route::cont(&id, By::Classifier, reason)
                }
                Some(_) => fallback,
                None => Route::new(By::Classifier, reason),
            });
        }

        // 6. The model.
        Ok(self.ask_model(input, last, &candidates, fallback).await)
    }

    /// The model's pick among `candidates`; `fallback` when it fails or names no candidate.
    async fn ask_model(
        &self,
        input: &Input,
        last: Option<&ThreadView>,
        candidates: &[&ThreadView],
        fallback: Route,
    ) -> Route {
        let mut ctx = Context::new(ROUTE_PROMPT);
        ctx.push(Message::user(route_request(input, last, candidates)));
        let Some(answer) = (self.provider.chat(&self.request, &ctx).await)
            .ok()
            .and_then(|res| json_object(&res.message.text()))
        else {
            return fallback;
        };
        let reason = answer["reason"]
            .as_str()
            .unwrap_or("chosen by the model")
            .to_string();
        let chosen = match &answer["thread"] {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            _ => return fallback,
        };
        if chosen == "new" {
            return Route::new(By::Model, reason);
        }
        candidates
            .iter()
            .find(|t| t.info.id == chosen)
            .map_or(fallback, |t| Route::cont(&t.info.id, By::Model, reason))
    }
}

/// Whether `text` stays on the subject of `thread` (its card and last messages): it shares a
/// word with them, or is too short to tell.
fn still_on_subject(store: &dyn Checkpointer, thread: &ThreadView, text: &str) -> Result<bool> {
    let words = terms_of(text);
    if words.len() < DRIFT_MIN_TERMS {
        return Ok(true);
    }
    let mut talk = card_doc(thread);
    for message in store.recent_messages(&thread.info.id, RECENT_MESSAGES)? {
        talk.push('\n');
        talk.push_str(&message.text());
    }
    let talk = terms_of(&talk);
    Ok(words.iter().any(|w| talk.contains(w)))
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

fn route_request(input: &Input, last: Option<&ThreadView>, candidates: &[&ThreadView]) -> String {
    let threads: Vec<String> = candidates
        .iter()
        .map(|t| {
            format!(
                "[{}] (last active {})\n{}",
                t.info.id,
                ago(elapsed(input.now, t.active_at)),
                card_doc(t)
            )
        })
        .collect();
    let last_line = last.map_or(String::new(), |t| {
        format!(
            "The user was last in thread [{}], {}.\n",
            t.info.id,
            ago(elapsed(input.now, t.active_at))
        )
    });
    format!(
        "Message: {}\n{last_line}\nThreads:\n{}",
        input.text,
        threads.join("\n\n")
    )
}

/// The JSON object in a model's reply, ignoring any text around it.
fn json_object(reply: &str) -> Option<Value> {
    let start = reply.find('{')?;
    let end = start + reply[start..].rfind('}')?;
    serde_json::from_str(&reply[start..=end]).ok()
}

const ROUTE_PROMPT: &str = r#"You route a user's message to the conversation it belongs to. Their earlier conversations
are listed as threads, each with a short card. Pick the thread this message continues, or "new" when it starts
something none of them covers. Choose a thread only when the message clearly continues it (same subject, a
follow-up, or it refers to it). Answer with ONLY JSON: {"thread":"<thread id>" or "new","reason":"<a few words>"}"#;

#[cfg(test)]
mod tests {
    use super::*;
    use inference::providers::OpenAi;
    use memory::Scope;
    use state::{Sqlite, Thread};

    fn turn(thread: &mut Thread, user: &str, reply: &str) {
        thread.start_turn(Message::user(user)).unwrap();
        thread.start_step().unwrap();
        thread.push(Message::assistant(reply)).unwrap();
        thread.end_turn().unwrap();
    }

    const BASETEN_MODEL: &str = "deepseek-ai/DeepSeek-V4.1-Flash";

    fn router(key: &str) -> Router<OpenAi> {
        Router::new(
            Provider::baseten(key),
            Request::new(BASETEN_MODEL).thinking(false),
            Arc::new(Memory::in_memory().unwrap()),
            vec![me()],
        )
    }

    fn me() -> Scope {
        Scope::user("jon")
    }

    fn store_with(threads: &[(&str, &[(&str, &str)])]) -> Arc<Sqlite> {
        let store = Arc::new(Sqlite::memory().unwrap());
        for (id, turns) in threads {
            let mut thread = Thread::new(*id);
            for (user, reply) in *turns {
                turn(&mut thread, user, reply);
            }
            store.save_thread(&mut thread).unwrap();
        }
        store
    }

    fn input(text: &str, last: Option<&str>, later: Duration) -> Input {
        Input {
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

    async fn route(router: &Router<OpenAi>, store: &Sqlite, input: Input) -> Route {
        let threads = router.threads(store, input.now).unwrap();
        router.route(store, &input, &threads).await.unwrap()
    }

    /// No tier here reaches the model: the router's provider is never called.
    #[tokio::test]
    async fn decides_in_code() {
        let router = router("unused");
        let store = store_with(&[KYOTO, METABASE]);
        let now = Duration::ZERO;
        let later = Duration::from_secs(3 * 3600);

        assert_eq!(
            route(
                &router,
                &store,
                input("New topic: what's a good pasta recipe?", Some("kyoto"), now)
            )
            .await,
            Route::new(By::Rule, "starts a new topic")
        );
        assert_eq!(
            route(&router, &store, input("yes, do it", Some("kyoto"), now)).await,
            Route::cont(
                "kyoto",
                By::Rule,
                "only makes sense in the last conversation"
            )
        );
        assert_eq!(
            route(
                &router,
                &store,
                input("Is Gion walkable from the station?", Some("kyoto"), now)
            )
            .await,
            Route::cont("kyoto", By::Rule, "the conversation is still going")
        );
        assert_eq!(
            route(
                &router,
                &store,
                input("Back to the Metabase thing.", Some("kyoto"), later)
            )
            .await,
            Route::cont("metabase", By::Search, "matches “Metabase” (1.00)")
        );
        // Shares nothing with any thread and doesn't look back: new, without the model.
        assert_eq!(
            route(
                &router,
                &store,
                input(
                    "Write a cron job that rotates logs nightly.",
                    Some("kyoto"),
                    later
                )
            )
            .await,
            Route::new(By::Rule, "shares nothing with any thread")
        );
        let empty = Sqlite::memory().unwrap();
        assert_eq!(
            route(&router, &empty, input("Hello", None, now)).await,
            Route::new(By::Rule, "no earlier threads")
        );
    }

    #[test]
    fn phases() {
        let router = router("unused");
        let store = store_with(&[KYOTO]);
        let phase = |later| {
            router
                .threads(store.as_ref(), SystemTime::now() + later)
                .unwrap()[0]
                .phase
        };
        assert_eq!(phase(Duration::ZERO), Phase::Hot);
        assert_eq!(phase(Duration::from_secs(31 * 60)), Phase::Cool);

        router
            .memory
            .add_resume("kyoto", me(), "About: Kyoto.", 9, 2, SystemTime::now())
            .unwrap();
        assert_eq!(phase(Duration::ZERO), Phase::Settled);
        // A new turn makes it active again.
        let mut thread = Thread::load("kyoto", store.as_ref()).unwrap();
        turn(&mut thread, "Actually, make it May.", "Updated to May.");
        store.save_thread(&mut thread).unwrap();
        assert_eq!(phase(Duration::ZERO), Phase::Hot);
    }

    /// Cards of the 10 most recent threads; older ones only when their card matches.
    #[tokio::test]
    async fn routes_on_cards() {
        let router = router("unused");
        let store = Arc::new(Sqlite::memory().unwrap());
        // Oldest, outside the 10. Its title mentions a ryokan, but its card is about Metabase.
        let mut old = Thread::new("old");
        turn(&mut old, "Find a ryokan near Gion.", "Try Gion Hatanaka.");
        turn(&mut old, "Now set up Metabase.", "Done, on Postgres.");
        store.save_thread(&mut old).unwrap();
        router
            .memory
            .add_card("old", me(), "Metabase dashboards on Postgres.", 7)
            .unwrap();
        for i in 0..10 {
            let mut filler = Thread::new(format!("filler-{i}"));
            turn(&mut filler, &format!("Question {i}"), "Answer.");
            store.save_thread(&mut filler).unwrap();
        }
        // Its title doesn't mention the ryokan; its card does.
        let mut trip = Thread::new("trip");
        turn(&mut trip, "Help me plan April.", "Sure.");
        store.save_thread(&mut trip).unwrap();
        router
            .memory
            .add_card(
                "trip",
                me(),
                "Kyoto in April: picking a ryokan near Gion.",
                3,
            )
            .unwrap();

        let later = Duration::from_secs(3 * 3600);
        assert_eq!(
            route(
                &router,
                &store,
                input("Back to the ryokan thing.", None, later)
            )
            .await,
            Route::cont("trip", By::Search, "matches “ryokan” (1.00)")
        );
        assert_eq!(
            route(
                &router,
                &store,
                input("Back to the Metabase thing.", None, later)
            )
            .await,
            Route::cont("old", By::Search, "matches “Metabase” (1.00)")
        );
        // Not named, but the all-cards search finds the old thread's card too.
        assert_eq!(
            route(
                &router,
                &store,
                input("The Metabase dashboards are slow on Postgres.", None, later)
            )
            .await,
            Route::cont(
                "old",
                By::Search,
                "matches “The Metabase dashboards are slow on Postgres.” (0.70)"
            )
        );
    }

    #[test]
    fn json_object_survives_a_brace_before_the_object() {
        assert_eq!(
            json_object(r#"Answer} {"thread":"new"}"#),
            Some(serde_json::json!({ "thread": "new" }))
        );
        // A truncated reply whose only `}` comes before its `{`.
        assert_eq!(json_object(r#"} {"thread":"kyo"#), None);
    }

    struct Always(Option<&'static str>);

    impl Classifier for Always {
        fn classify(&self, _: &Input, _: &[&ThreadView]) -> Option<(Option<String>, f32)> {
            Some((self.0.map(String::from), 0.9))
        }
    }

    /// Looks back at something no card matches: ambiguous, so the classifier decides before
    /// any model would.
    #[tokio::test]
    async fn classifier_before_model() {
        let mut router = router("unused");
        router.classifier = Some(Box::new(Always(Some("kyoto"))));
        let store = store_with(&[KYOTO, METABASE]);
        let later = Duration::from_secs(3 * 3600);
        assert_eq!(
            route(
                &router,
                &store,
                input(
                    "Which hotel did we pick for Japan?",
                    Some("metabase"),
                    later
                )
            )
            .await,
            Route::cont("kyoto", By::Classifier, "classified (0.90)")
        );
    }

    fn key() -> String {
        std::env::var("BASETEN_API_KEY").expect("set BASETEN_API_KEY")
    }

    /// The same ambiguous input with no classifier: the fast model decides.
    #[tokio::test]
    #[ignore = "calls Baseten"]
    async fn model_last() {
        let router = router(&key());
        let store = store_with(&[KYOTO, METABASE]);
        let later = Duration::from_secs(3 * 3600);
        let started = std::time::Instant::now();
        let route = route(
            &router,
            &store,
            input(
                "Which hotel did we pick for Japan?",
                Some("metabase"),
                later,
            ),
        )
        .await;
        println!("{route:?} in {:?}", started.elapsed());
        assert_eq!(
            (route.target, route.by),
            (Target::Continue("kyoto".into()), By::Model)
        );
    }
}
