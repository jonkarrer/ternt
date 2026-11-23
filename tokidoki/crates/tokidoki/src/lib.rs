//! The whole system as one library: input in, output out.
//!
//! `Tokidoki::open` takes a `Config` (`Config::new`; `Config::from_env` for `chat`) and wires the state
//! database, memory, the router, Experience, the engine and its agent tasks, and the journal.
//! It resumes whatever a previous process left unfinished. Whatever embeds it calls its
//! methods with `interface` types and gets `interface` types back: `send` an input, follow
//! `events` (or `wait`), read `threads` and `history`, `abort`, give `feedback`; and see what
//! it remembers (`memories`), tell it something (`remember`), correct it (`revise`), have it
//! `forget` one thing or `wipe` all of it. Where inputs come from and where outputs go is the embedding code's business,
//! not tokidoki's.
//!
//! Who may speak is decided here: every input names its `Speaker`, and only the configured
//! speakers are let in, all as the one user whose memory scope this instance reads and
//! writes. (Several users, each with their own scope, is a later step.)

mod events;
pub mod serve;
mod spaces;

pub use spaces::{Setup, Spaces};

use std::path::PathBuf;
use std::sync::Arc;

use agent::{Agent, Live, Mind, Model, NewThreads, ProviderModel};
use anyhow::{Context as _, Result};
use engine::{Engine, Registry};
use experience::{Experience, Scopes};
use inference::providers::AnyApi;
use inference::{Provider, Request};
use interface::{Feedback, Input, Rejected, Sent, Speaker, Submission, ThreadSummary};
use memory::{Body, Memory, Scope, Write};
use router::Router;
use state::{Checkpointer, Sqlite};
use tokio::sync::mpsc;
use tracing::Instrument;

// Everything an embedding program names, so this crate is the only one it depends on.
pub use agent::{Settings, Tool, ToolFuture, Tools};
pub use inference::{Effort, Vendor};
pub use interface;
pub use tools;

/// Events as they happen, until dropped.
pub type Events = mpsc::Receiver<interface::Event>;

pub const DEFAULT_SYSTEM: &str = "You are tokidoki, a personal assistant. Memory is automatic: \
what matters is remembered across conversations without any tool, and memories relevant to \
the conversation are added to it for you. Be direct and concise.";

#[derive(Clone)]
pub struct Config {
    /// Holds `tokidoki.db` (state), `tokidoki.journal.db` and `memory.db`.
    pub data: PathBuf,
    /// Whose memory this is: the scope every speaker reads and writes.
    pub user: String,
    /// The `Speaker` IDs let in.
    pub speakers: Vec<String>,
    /// What every new thread starts with.
    pub system: Option<String>,
    /// Whose models it runs on, all of them, and the user's key for it.
    pub provider: Vendor,
    pub key: String,
    /// Answers, and Experience's memory work.
    pub model: String,
    pub effort: Effort,
    /// The router's fast model, its last resort.
    pub router_model: String,
    pub agent: agent::Settings,
    /// Record to the journal (`telemetry`), for `just inspect`. Each open system records to
    /// the journal in its own `data`, however many run in the process.
    pub journal: bool,
}

/// Never the key: a `{config:?}` anywhere would otherwise put it in the journal and the host's log.
impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("data", &self.data)
            .field("user", &self.user)
            .field("provider", &self.provider)
            .field(
                "key",
                &if self.key.is_empty() {
                    "<empty>"
                } else {
                    "<set>"
                },
            )
            .field("model", &self.model)
            .field("router_model", &self.router_model)
            .finish_non_exhaustive()
    }
}

impl Config {
    /// From the environment: `TOKIDOKI_PROVIDER` (`anthropic`, the default, `fireworks` or
    /// `baseten`) and that provider's key (`ANTHROPIC_API_KEY`, `FIREWORKS_API_KEY`,
    /// `BASETEN_API_KEY`: required); `TOKIDOKI_MODEL` and `TOKIDOKI_ROUTER_MODEL` (default:
    /// the provider's, `Config::models`); `TOKIDOKI_DATA` (default `data`), `TOKIDOKI_USER`
    /// (default `owner`), `TOKIDOKI_SPEAKERS` (comma-separated IDs, default `owner`),
    /// `TOKIDOKI_SYSTEM`.
    pub fn from_env() -> Result<Self> {
        let provider = Self::provider_from_env()?;
        let key = var(provider.key_var())
            .with_context(|| format!("{} is not set", provider.key_var()))?;
        Ok(Self::new(telemetry::data_dir(), provider, key).with_env())
    }

    /// The provider `TOKIDOKI_PROVIDER` names: Anthropic when it names none.
    pub fn provider_from_env() -> Result<Vendor> {
        match var("TOKIDOKI_PROVIDER") {
            Some(provider) => provider.parse().context("TOKIDOKI_PROVIDER"),
            None => Ok(Vendor::Anthropic),
        }
    }

    /// With what the environment says of the models, the user, the speakers and the system
    /// prompt (see `from_env`).
    pub fn with_env(mut self) -> Self {
        self = self.with_models_from_env();
        let config = &mut self;
        if let Some(user) = var("TOKIDOKI_USER") {
            config.user = user;
        }
        if let Some(speakers) = var("TOKIDOKI_SPEAKERS") {
            config.speakers = speakers
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
        if let Some(system) = var("TOKIDOKI_SYSTEM") {
            config.system = Some(system);
        }
        self
    }

    /// With the models the environment names: `TOKIDOKI_MODELS=cheap` for the provider's
    /// cheapest (tests, always), then `TOKIDOKI_MODEL` and `TOKIDOKI_ROUTER_MODEL` by name.
    /// The part of `with_env` a host's environment may set.
    pub fn with_models_from_env(mut self) -> Self {
        if var("TOKIDOKI_MODELS").as_deref() == Some("cheap") {
            let (model, router_model) = Self::cheap_models(self.provider);
            self.model = model.into();
            self.router_model = router_model.into();
        }
        if let Some(model) = var("TOKIDOKI_MODEL") {
            self.model = model;
        }
        if let Some(model) = var("TOKIDOKI_ROUTER_MODEL") {
            self.router_model = model;
        }
        self
    }

    /// The models a provider runs unless told otherwise: the one that answers, and the
    /// router's fast one.
    pub fn models(provider: Vendor) -> (&'static str, &'static str) {
        match provider {
            Vendor::Anthropic => ("claude-opus-5-5", "claude-haiku-5-5"),
            // Cheap by choice (the owner's): the flash model answers and routes. Checked
            // against the account's /v1/models on 2026-11-21; deepseek-v4-pro isn't served
            // there, kimi-k3p1-pro is, if something stronger is ever wanted.
            Vendor::Fireworks => (
                "accounts/fireworks/models/deepseek-v4p1-flash",
                "accounts/fireworks/models/deepseek-v4p1-flash",
            ),
            Vendor::Baseten => (
                "deepseek-ai/DeepSeek-V4-Pro",
                "deepseek-ai/DeepSeek-V4.1-Flash",
            ),
        }
    }

    /// The provider's cheapest models that still do the job: what every test that calls a
    /// model runs on (`TOKIDOKI_MODELS=cheap`), never the defaults above.
    pub fn cheap_models(provider: Vendor) -> (&'static str, &'static str) {
        match provider {
            Vendor::Anthropic => ("claude-haiku-5-5", "claude-haiku-5-5"),
            Vendor::Fireworks => (
                "accounts/fireworks/models/deepseek-v4p1-flash",
                "accounts/fireworks/models/deepseek-v4p1-flash",
            ),
            Vendor::Baseten => (
                "deepseek-ai/DeepSeek-V4.1-Flash",
                "deepseek-ai/DeepSeek-V4.1-Flash",
            ),
        }
    }

    /// The owner's own tokidoki, with its data in `data`, running on `provider` with the
    /// user's `key` for it: what a program that holds the key itself (an app) starts from.
    pub fn new(data: impl Into<PathBuf>, provider: Vendor, key: String) -> Self {
        let (model, router_model) = Self::models(provider);
        Self {
            data: data.into(),
            user: "owner".into(),
            speakers: vec!["owner".into()],
            system: Some(DEFAULT_SYSTEM.into()),
            provider,
            key,
            model: model.into(),
            effort: Effort::Low,
            router_model: router_model.into(),
            agent: agent::Settings::default(),
            journal: true,
        }
    }

    pub fn state_path(&self) -> PathBuf {
        self.data.join(telemetry::STATE_FILE)
    }

    pub fn memory_path(&self) -> PathBuf {
        self.data.join(telemetry::MEMORY_FILE)
    }
}

/// An environment variable that says something.
fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

/// The pieces `Tokidoki::build` puts together; `open` makes them from a `Config`.
pub struct Parts<M> {
    pub store: Arc<Sqlite>,
    pub model: Arc<M>,
    pub mind: Option<Arc<dyn Mind>>,
    pub router: Router<AnyApi>,
    /// What it remembers, and whose memories those are.
    pub memory: Arc<Memory>,
    pub scope: Scope,
    pub tools: Tools,
    /// What every new thread starts with (with `tools` registered).
    pub system: Option<String>,
    pub settings: agent::Settings,
    pub speakers: Vec<String>,
    /// The journal being recorded to (`telemetry::record`), flushed on `close`. Everything
    /// the system does happens in its span.
    pub journal: Option<telemetry::Recording>,
}

/// What a tokidoki runs on, for whoever embeds it to show (`Tokidoki::about`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct About {
    pub provider: Vendor,
    /// The model that answers.
    pub model: String,
    /// The tools its new threads get, by name.
    pub tools: Vec<String>,
}

/// The running system; cheap to clone.
#[derive(Clone)]
pub struct Tokidoki {
    inner: Arc<Inner>,
}

struct Inner {
    agent: Agent,
    live: Live,
    router: Router<AnyApi>,
    memory: Arc<Memory>,
    scope: Scope,
    store: Arc<Sqlite>,
    speakers: Vec<String>,
    journal: Option<telemetry::Recording>,
    about: About,
    /// Why it runs nothing, when it doesn't (`open_asleep`).
    asleep: Option<String>,
}

impl Tokidoki {
    /// Opens the system from `config`, with `tools` for the model, and resumes unfinished
    /// work. Call inside a Tokio runtime.
    pub fn open(config: Config, tools: Tools) -> Result<Self> {
        Self::open_as(config, tools, None)
    }

    /// Opens what `config`'s folder holds without running any of it, for when it can't be
    /// run (no key for the model, say): its threads, history and memory can be read and its
    /// memory changed, unfinished work stays unfinished for the next `open`, and an input is
    /// rejected with `why`. `config`'s keys are not used.
    pub fn open_asleep(config: Config, why: impl Into<String>) -> Result<Self> {
        Self::open_as(config, Tools::new(), Some(why.into()))
    }

    fn open_as(config: Config, tools: Tools, asleep: Option<String>) -> Result<Self> {
        std::fs::create_dir_all(&config.data)
            .with_context(|| format!("data directory {}", config.data.display()))?;
        let journal = match config.journal {
            // Fails if the program installed its own subscriber, which then gets the events.
            true => telemetry::record(&telemetry::Journal::beside(config.state_path()))
                .map(Some)
                .unwrap_or_else(|error| {
                    eprintln!("tokidoki: not recording the journal: {error:#}");
                    None
                }),
            false => None,
        };
        if cfg!(panic = "abort") {
            static WARNED: std::sync::Once = std::sync::Once::new();
            WARNED.call_once(|| {
                eprintln!(
                    "tokidoki: built with panic = \"abort\": a task that panics ends the \
                     process instead of being faulted"
                )
            });
        }
        // Opening is traced in the system's own span too.
        let _recorded = span_of(&journal).entered();
        let store = Arc::new(Sqlite::open(config.state_path())?);
        let memory = Arc::new(Memory::open(config.memory_path())?);
        let scope = Scope::user(&config.user);
        let answers = || Request::new(config.model.clone()).effort(config.effort);
        let provider = || Provider::of(config.provider, &config.key);
        let router = Router::new(
            provider(),
            Request::new(config.router_model.clone()).thinking(false),
            memory.clone(),
            vec![scope.clone()],
        );
        let experience = Experience::new(
            provider(),
            answers(),
            memory.clone(),
            Scopes {
                read: vec![scope.clone()],
                write: scope.clone(),
            },
        );
        let model = Arc::new(ProviderModel {
            provider: provider(),
            request: answers(),
        });
        let about = About {
            provider: config.provider,
            model: config.model.clone(),
            tools: tools.specs().into_iter().map(|t| t.name).collect(),
        };
        let parts = Parts {
            store,
            model,
            mind: Some(Arc::new(experience)),
            router,
            memory,
            scope,
            tools,
            system: config.system,
            settings: config.agent,
            speakers: config.speakers,
            journal,
        };
        Self::assemble(parts, asleep, about)
    }

    /// Puts given parts together (another model, a test double), and resumes.
    pub fn build<M: Model>(parts: Parts<M>) -> Result<Self> {
        let about = About {
            provider: Vendor::Anthropic,
            model: "built".into(),
            tools: parts.tools.specs().into_iter().map(|t| t.name).collect(),
        };
        Self::assemble(parts, None, about)
    }

    fn assemble<M: Model>(parts: Parts<M>, asleep: Option<String>, about: About) -> Result<Self> {
        // The engine keeps the span it is opened in, for everything it runs.
        let _recorded = span_of(&parts.journal).entered();
        let new_threads = NewThreads {
            system: parts.system,
            tools: parts.tools.specs(),
            place: Default::default(),
        };
        let live = Live::default();
        // Whatever changes memory (answering, dreaming, the user) is shown as it happens.
        let shown = live.clone();
        parts
            .memory
            .watch(move |change| shown.show(events::memory_change(change)));
        let mut registry = Registry::new();
        agent::register(
            &mut registry,
            parts.model,
            parts.tools,
            parts.mind,
            parts.settings,
            live.clone(),
        );
        let engine = Engine::open(parts.store.clone(), registry, engine::Settings::default())?;
        if asleep.is_none() {
            engine.resume();
        }
        Ok(Self {
            inner: Arc::new(Inner {
                agent: Agent::new(engine, new_threads),
                live,
                router: parts.router,
                memory: parts.memory,
                scope: parts.scope,
                store: parts.store,
                speakers: parts.speakers,
                journal: parts.journal,
                about,
                asleep,
            }),
        })
    }

    /// What it runs on.
    pub fn about(&self) -> &About {
        &self.inner.about
    }

    /// Why it takes no input, if it doesn't (`open_asleep`: no key for its provider).
    pub fn asleep(&self) -> Option<&str> {
        self.inner.asleep.as_deref()
    }

    /// Stops running work (it resumes when the system is opened again), waits until it has
    /// stopped, and finishes writing the journal: nothing of this system touches its files
    /// afterwards, so they can be opened again, or deleted.
    pub async fn close(&self) -> Result<()> {
        let engine = self.inner.agent.engine();
        let closed = self.span().in_scope(|| engine.close());
        // Work that never yields can't be stopped: go on without it rather than hang.
        if tokio::time::timeout(STOP_WITHIN, engine.stopped())
            .await
            .is_err()
        {
            self.span()
                .in_scope(|| tracing::warn!("closed while some work was still running"));
        }
        if let Some(recording) = &self.inner.journal {
            recording.journal.flush();
        }
        closed
    }

    /// Whether `close` was called: it takes nothing more, and `open` gives a new one.
    pub fn is_closed(&self) -> bool {
        self.inner.agent.engine().is_closed()
    }

    /// The span this system's own calls are traced in (see `span_of`).
    fn span(&self) -> tracing::Span {
        span_of(&self.inner.journal)
    }

    fn admit(&self, speaker: &interface::Speaker) -> Result<()> {
        if !self.inner.speakers.contains(&speaker.id) {
            let id = &speaker.id;
            return Err(Rejected(format!("{id} may not speak to this tokidoki")).into());
        }
        Ok(())
    }
}

/// What an embedding app calls. Each takes and returns `interface` types, ready to pass on as
/// JSON; an input it won't take fails with an `interface::Rejected`.
impl Tokidoki {
    /// Tell the agent what it is the agent of: the name of the space it lives in, as the host
    /// calls it. Threads started from now on are told; a rename reaches the next new thread.
    pub fn set_label(&self, label: &str) {
        self.inner.agent.set_place(Some(label.to_string()));
    }

    /// Admits an input, to its thread or wherever the router sends it, and returns at once:
    /// follow it with `events` or `wait`.
    pub async fn send(&self, input: Input) -> Result<Sent> {
        self.admit(&input.from)?;
        if let Some(why) = &self.inner.asleep {
            return Err(Rejected(why.clone()).into());
        }
        let speaker = input.from.id.clone();
        self.inner
            .agent
            .send(&self.inner.router, input)
            .instrument(tracing::info_span!(parent: self.span(), "input", speaker = %speaker))
            .await
    }

    pub fn submission(&self, id: &str) -> Result<Option<Submission>> {
        self.inner.agent.submission(id)
    }

    /// Every thread, most recently active first.
    pub fn threads(&self) -> Result<Vec<ThreadSummary>> {
        let threads = self.inner.store.threads()?.into_iter();
        threads
            .map(|info| {
                Ok(ThreadSummary {
                    about: self.inner.memory.card(&info.id)?.map(|card| card.text),
                    id: info.id,
                    title: info.title,
                    turns: info.turns,
                    created_at: telemetry::time::ms(info.created_at),
                    active_at: telemetry::time::ms(info.active_at),
                })
            })
            .collect()
    }

    /// What it remembers, newest first: the current version of each memory.
    pub fn memories(&self, from: &Speaker) -> Result<Vec<interface::Memory>> {
        self.admit(from)?;
        let scope = std::slice::from_ref(&self.inner.scope);
        let items = self.inner.memory.list(scope, MEMORIES_SHOWN)?;
        Ok(items.iter().map(events::memory).collect())
    }

    /// Something the user tells it outright, kept as a fact.
    pub fn remember(&self, from: &Speaker, text: &str) -> Result<interface::Memory> {
        let _recorded = self.span().entered();
        self.admit(from)?;
        let write = Write::fact(self.inner.scope.clone(), stated(text)?);
        let item = self.inner.memory.write(write)?;
        tracing::info!(id = item.id, speaker = %from.id, "memory added by the user");
        Ok(events::memory(&item))
    }

    /// The user's correction of a fact: a new version of the memory `id`, which keeps where
    /// it was learned. Rejected for an ID that is no longer current (it was revised or
    /// forgotten meanwhile: read `memories` again).
    pub fn revise(&self, from: &Speaker, id: i64, text: &str) -> Result<interface::Memory> {
        let _recorded = self.span().entered();
        self.admit(from)?;
        let old = self.current(id)?;
        if !matches!(old.body, Body::Semantic { .. }) {
            return Err(
                Rejected("only facts can be edited; forget this one instead".into()).into(),
            );
        }
        let mut write = Write::fact(self.inner.scope.clone(), stated(text)?);
        write.source = old.source;
        let item = self.inner.memory.replace(id, write)?;
        tracing::info!(id = item.id, replaces = id, speaker = %from.id, "memory revised by the user");
        Ok(events::memory(&item))
    }

    /// Removes the memory `id` and every earlier version of it, for good.
    pub fn forget(&self, from: &Speaker, id: i64) -> Result<()> {
        let _recorded = self.span().entered();
        self.admit(from)?;
        self.current(id)?;
        self.inner.memory.forget(id)?;
        tracing::info!(id, speaker = %from.id, "memory forgotten by the user");
        Ok(())
    }

    /// Forgets everything it has learned: every fact and procedure, and its summaries of
    /// past conversations. The conversations themselves stay. There is no undoing it.
    pub fn wipe(&self, from: &Speaker) -> Result<()> {
        let _recorded = self.span().entered();
        self.admit(from)?;
        self.inner.memory.wipe(&self.inner.scope)?;
        tracing::info!(speaker = %from.id, "memory wiped by the user");
        Ok(())
    }

    /// The memory `id`, if it is this user's and still current.
    fn current(&self, id: i64) -> Result<memory::Item> {
        match self.inner.memory.get(id)? {
            Some(item) if item.scope == self.inner.scope => Ok(item),
            _ => Err(Rejected(format!("memory {id} has changed or is gone")).into()),
        }
    }

    /// A thread's checkpoints from `since` on, as events: to load a thread, or catch up after
    /// missing some (`Event::Checkpoint::seq` has no gaps).
    pub fn history(&self, thread: &str, since: usize) -> Result<Vec<interface::Event>> {
        events::checkpoints(self.inner.agent.engine(), thread, since..usize::MAX)
    }

    /// What happens from now on, across threads, until the receiver is dropped: what is
    /// committed, and replies as they are written (`Event::Delta`). Subscribe before `send`
    /// to miss nothing about the input. Call inside a Tokio runtime.
    pub fn events(&self) -> Events {
        events::follow(self.inner.agent.engine(), &self.inner.live)
    }

    /// Resolves once the submission is answered or can't be.
    pub async fn wait(&self, id: &str) -> Result<Submission> {
        self.inner.agent.wait(id).instrument(self.span()).await
    }

    /// Stops the thread's run, if one is running. Returns whether there was one.
    pub fn abort(&self, thread: &str) -> Result<bool> {
        let _recorded = self.span().entered();
        self.inner.agent.abort(thread)
    }

    /// "That was wrong", about a turn of a thread: kept with the thread in the state database,
    /// and traced as a warning so `just inspect recent` and `problems` show it on that turn.
    pub fn feedback(&self, feedback: Feedback) -> Result<()> {
        let _recorded = self.span().entered();
        self.admit(&feedback.from)?;
        let thread = &feedback.thread;
        let turn = self.inner.agent.engine().commit(|tx| {
            let turn = match feedback.turn {
                Some(turn) => turn,
                None => tx.thread(thread)?.turn(),
            };
            let key = feedback_key(thread);
            let mut all: Vec<Feedback> = tx.doc(&key)?.unwrap_or_default();
            all.push(Feedback {
                turn: Some(turn),
                ..feedback.clone()
            });
            tx.put_doc(&key, &all)?;
            Ok(turn)
        })?;
        tracing::warn!(
            thread = %thread,
            turn,
            speaker = %feedback.from.id,
            text = %feedback.text,
            "feedback"
        );
        Ok(())
    }
}

/// The span a system's work is traced in: its journal's, so what it does is recorded there
/// and not in another system's. With no journal, the caller's own span.
fn span_of(journal: &Option<telemetry::Recording>) -> tracing::Span {
    journal
        .as_ref()
        .map_or_else(tracing::Span::current, |recording| recording.span.clone())
}

/// How long `close` waits for running work to stop.
const STOP_WITHIN: std::time::Duration = std::time::Duration::from_secs(10);

/// How many memories `memories` returns at most.
const MEMORIES_SHOWN: usize = 2_000;

/// A memory's text as the user gave it, trimmed; rejected when empty.
fn stated(text: &str) -> Result<String> {
    match text.trim() {
        "" => Err(Rejected("a memory needs some text".into()).into()),
        text => Ok(text.to_string()),
    }
}

/// Where a thread's feedback is kept, in the state database.
fn feedback_key(thread: &str) -> String {
    format!("feedback/{thread}")
}

#[cfg(test)]
mod tests;
