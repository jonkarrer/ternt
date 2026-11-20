//! The whole system as one library: input in, output out.
//!
//! `Tokidoki::open` takes a `Config` (usually `Config::from_env`) and wires the state
//! database, memory, the router, Experience, the engine and its agent tasks, and the journal.
//! It resumes whatever a previous process left unfinished. Whatever embeds it calls its
//! methods with `interface` types and gets `interface` types back: `send` an input, follow
//! `events` (or `wait`), read `threads` and `history`, `abort`, give `feedback`. Where inputs
//! come from and where outputs go is the embedding code's business, not tokidoki's.
//!
//! Who may speak is decided here: every input names its `Speaker`, and only the configured
//! speakers are let in, all as the one user whose memory scope this instance reads and
//! writes. (Several users, each with their own scope, is a later step.)

mod events;

use std::path::PathBuf;
use std::sync::Arc;

use agent::{Agent, Mind, Model, NewThreads, ProviderModel, Tools};
use anyhow::{Context as _, Result};
use engine::{Engine, Registry};
use experience::{Experience, Scopes};
use inference::providers::{Anthropic, OpenAi};
use inference::{Effort, Provider, Request};
use interface::{Feedback, Input, Rejected, Sent, Submission, ThreadSummary};
use memory::{Memory, Scope};
use router::Router;
use state::{Checkpointer, Sqlite};
use tokio::sync::mpsc;
use tracing::Instrument;

pub use interface;

/// Events as they happen, until dropped.
pub type Events = mpsc::Receiver<interface::Event>;

pub const DEFAULT_SYSTEM: &str = "You are tokidoki, a personal assistant. Memory is automatic: \
what matters is remembered across conversations without any tool, and memories relevant to \
the conversation are added to it for you. Be direct and concise.";

#[derive(Debug, Clone)]
pub struct Config {
    /// Holds `tokidoki.db` (state), `tokidoki.journal.db` and `memory.db`.
    pub data: PathBuf,
    /// Whose memory this is: the scope every speaker reads and writes.
    pub user: String,
    /// The `Speaker` IDs let in.
    pub speakers: Vec<String>,
    /// What every new thread starts with.
    pub system: Option<String>,
    pub anthropic_key: String,
    pub baseten_key: String,
    /// Answers, and Experience's memory work.
    pub model: String,
    pub effort: Effort,
    /// The router's fast model, its last resort.
    pub router_model: String,
    pub agent: agent::Settings,
    /// Record to the journal (`telemetry`), for `just inspect`.
    pub journal: bool,
}

impl Config {
    /// From the environment: `ANTHROPIC_API_KEY`, `BASETEN_API_KEY` (required);
    /// `TOKIDOKI_DATA` (default `data`), `TOKIDOKI_USER` (default `owner`),
    /// `TOKIDOKI_SPEAKERS` (comma-separated IDs, default `owner`),
    /// `TOKIDOKI_SYSTEM`.
    pub fn from_env() -> Result<Self> {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
        let required = |name: &str| var(name).with_context(|| format!("{name} is not set"));
        Ok(Self {
            data: telemetry::data_dir(),
            user: var("TOKIDOKI_USER").unwrap_or_else(|| "owner".into()),
            speakers: var("TOKIDOKI_SPEAKERS")
                .unwrap_or_else(|| "owner".into())
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            system: Some(var("TOKIDOKI_SYSTEM").unwrap_or_else(|| DEFAULT_SYSTEM.into())),
            anthropic_key: required("ANTHROPIC_API_KEY")?,
            baseten_key: required("BASETEN_API_KEY")?,
            model: "claude-opus-5-5".into(),
            effort: Effort::Low,
            router_model: "deepseek-ai/DeepSeek-V4.1-Flash".into(),
            agent: agent::Settings::default(),
            journal: true,
        })
    }

    pub fn state_path(&self) -> PathBuf {
        self.data.join(telemetry::STATE_FILE)
    }

    pub fn memory_path(&self) -> PathBuf {
        self.data.join(telemetry::MEMORY_FILE)
    }
}

/// The pieces `Tokidoki::build` puts together; `open` makes them from a `Config`.
pub struct Parts<M> {
    pub store: Arc<Sqlite>,
    pub model: Arc<M>,
    pub mind: Option<Arc<dyn Mind>>,
    pub router: Router<OpenAi>,
    pub tools: Tools,
    /// What every new thread starts with (with `tools` registered).
    pub system: Option<String>,
    pub settings: agent::Settings,
    pub speakers: Vec<String>,
    /// The journal `telemetry::init` opened, flushed on `close`.
    pub journal: Option<telemetry::Journal>,
}

/// The running system; cheap to clone.
#[derive(Clone)]
pub struct Tokidoki {
    inner: Arc<Inner>,
}

struct Inner {
    agent: Agent,
    router: Router<OpenAi>,
    store: Arc<Sqlite>,
    speakers: Vec<String>,
    journal: Option<telemetry::Journal>,
}

impl Tokidoki {
    /// Opens the system from `config`, with `tools` for the model, and resumes unfinished
    /// work. Call inside a Tokio runtime.
    pub fn open(config: Config, tools: Tools) -> Result<Self> {
        std::fs::create_dir_all(&config.data)
            .with_context(|| format!("data directory {}", config.data.display()))?;
        let journal = match config.journal {
            // Fails if the program installed its own subscriber, which then gets the events.
            true => telemetry::init(&telemetry::Journal::beside(config.state_path()))
                .map(Some)
                .unwrap_or_else(|error| {
                    eprintln!("tokidoki: not recording the journal: {error:#}");
                    None
                }),
            false => None,
        };
        let store = Arc::new(Sqlite::open(config.state_path())?);
        let memory = Arc::new(Memory::open(config.memory_path())?);
        let scope = Scope::user(&config.user);
        let claude = || Request::new(config.model.clone()).effort(config.effort);
        let anthropic = || Provider::<Anthropic>::anthropic(&config.anthropic_key);
        let router = Router::new(
            Provider::<OpenAi>::baseten(&config.baseten_key),
            Request::new(config.router_model.clone()).thinking(false),
            memory.clone(),
            vec![scope.clone()],
        );
        let experience = Experience::new(
            anthropic(),
            claude(),
            memory.clone(),
            Scopes {
                read: vec![scope.clone()],
                write: scope,
            },
        );
        let model = Arc::new(ProviderModel {
            provider: anthropic(),
            request: claude(),
        });
        Self::build(Parts {
            store,
            model,
            mind: Some(Arc::new(experience)),
            router,
            tools,
            system: config.system,
            settings: config.agent,
            speakers: config.speakers,
            journal,
        })
    }

    /// Puts given parts together (another model, a test double), and resumes.
    pub fn build<M: Model>(parts: Parts<M>) -> Result<Self> {
        let new_threads = NewThreads {
            system: parts.system,
            tools: parts.tools.specs(),
        };
        let mut registry = Registry::new();
        agent::register(
            &mut registry,
            parts.model,
            parts.tools,
            parts.mind,
            parts.settings,
        );
        let engine = Engine::open(parts.store.clone(), registry, engine::Settings::default())?;
        engine.resume();
        Ok(Self {
            inner: Arc::new(Inner {
                agent: Agent::new(engine, new_threads),
                router: parts.router,
                store: parts.store,
                speakers: parts.speakers,
                journal: parts.journal,
            }),
        })
    }

    /// Stops running work (it resumes when the system is opened again) and finishes writing
    /// the journal.
    pub fn close(&self) -> Result<()> {
        let closed = self.inner.agent.engine().close();
        if let Some(journal) = &self.inner.journal {
            journal.flush();
        }
        closed
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
    /// Admits an input, to its thread or wherever the router sends it, and returns at once:
    /// follow it with `events` or `wait`.
    pub async fn send(&self, input: Input) -> Result<Sent> {
        self.admit(&input.from)?;
        let speaker = input.from.id.clone();
        self.inner
            .agent
            .send(&self.inner.router, input)
            .instrument(tracing::info_span!("input", speaker = %speaker))
            .await
    }

    pub fn submission(&self, id: &str) -> Result<Option<Submission>> {
        self.inner.agent.submission(id)
    }

    /// Every thread, most recently active first.
    pub fn threads(&self) -> Result<Vec<ThreadSummary>> {
        Ok(self
            .inner
            .store
            .threads()?
            .into_iter()
            .map(|info| ThreadSummary {
                id: info.id,
                title: info.title,
                turns: info.turns,
                created_at: telemetry::time::ms(info.created_at),
                active_at: telemetry::time::ms(info.active_at),
            })
            .collect())
    }

    /// A thread's checkpoints from `since` on, as events: to load a thread, or catch up after
    /// missing some (`Event::Checkpoint::seq` has no gaps).
    pub fn history(&self, thread: &str, since: usize) -> Result<Vec<interface::Event>> {
        events::checkpoints(self.inner.agent.engine(), thread, since..usize::MAX)
    }

    /// What happens from now on, across threads, until the receiver is dropped. Subscribe
    /// before `send` to miss nothing about the input. Call inside a Tokio runtime.
    pub fn events(&self) -> Events {
        events::follow(self.inner.agent.engine())
    }

    /// Resolves once the submission is answered or can't be.
    pub async fn wait(&self, id: &str) -> Result<Submission> {
        self.inner.agent.wait(id).await
    }

    /// Stops the thread's run, if one is running. Returns whether there was one.
    pub fn abort(&self, thread: &str) -> Result<bool> {
        self.inner.agent.abort(thread)
    }

    /// "That was wrong", about a turn of a thread: kept with the thread in the state database,
    /// and traced as a warning so `just inspect recent` and `problems` show it on that turn.
    pub fn feedback(&self, feedback: Feedback) -> Result<()> {
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

/// Where a thread's feedback is kept, in the state database.
fn feedback_key(thread: &str) -> String {
    format!("feedback/{thread}")
}

#[cfg(test)]
mod tests;
