//! State is the source of truth for a session. A `Thread` is an append-only log of
//! checkpoints; every change (a message, a tool, a compaction, a turn boundary) is one.
//! `Context`, what the model sees, is never stored: it's rebuilt from the log, so dropping
//! something from the context never loses it from history.
//!
//! A turn runs from the user's message to the model's final reply. Each `chat` call inside
//! it, plus the tools it asks for, is one step.
//!
//! With a `Checkpointer`, every checkpoint is saved before it takes effect; if the save
//! fails, the thread is unchanged and the call returns the error.

pub mod sqlite;

pub use sqlite::Sqlite;

use std::ops::Range;
use std::sync::Arc;
use std::time::SystemTime;

use anyhow::{Result, ensure};
use context::{Block, Context, Message, Role, Tool};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum Event {
    SystemSet(Option<String>),
    ToolRegistered(Tool),
    ToolRemoved(String),
    TurnStarted,
    StepStarted,
    /// User input, model replies (including tool calls), tool results.
    Message(Message),
    /// Swap `range` of the context's messages for `with`: compaction, or removal when
    /// `with` is empty. History keeps the originals.
    Replaced {
        range: Range<usize>,
        with: Vec<Message>,
    },
    TurnEnded,
    /// Experience looked at every checkpoint before `upto` and brought `recalled` into the
    /// context, shown as a text block where it happened (never by editing earlier messages).
    Considered {
        upto: usize,
        recalled: Vec<Recalled>,
    },
    /// An incoming message was placed in this thread: by what (rule, search, model) and why.
    /// Kept as training data for a learned router; the model never sees it.
    Routed {
        by: String,
        reason: String,
    },
}

/// What a thread list shows, without loading every checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct ThreadInfo {
    pub id: String,
    /// The start of the first message.
    pub title: String,
    pub turns: u32,
    pub checkpoints: usize,
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
    /// The last message, from anyone. Unlike `updated_at`, not moved by compaction or routing.
    pub active_at: SystemTime,
}

/// A memory brought into the context.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recalled {
    pub id: i64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Checkpoint {
    /// Position in the thread, from 0.
    pub id: usize,
    /// Turn and step this happened in; 0 means outside any.
    pub turn: u32,
    pub step: u32,
    pub at: SystemTime,
    pub event: Event,
}

/// Where checkpoints are saved. Shared between threads, so methods take `&self`.
pub trait Checkpointer: Send + Sync {
    /// Append checkpoints to a thread, all or none.
    fn save(&self, thread_id: &str, checkpoints: &[Checkpoint]) -> Result<()>;
    /// A thread's checkpoints in order; empty if it doesn't exist.
    fn load(&self, thread_id: &str) -> Result<Vec<Checkpoint>>;
    /// Every thread, most recently active first.
    fn threads(&self) -> Result<Vec<ThreadInfo>>;
    /// A thread's last `n` messages, oldest first, without loading the rest of it.
    fn recent_messages(&self, thread_id: &str, n: usize) -> Result<Vec<Message>>;
}

/// Runs after every recorded event and may record more. Events recorded from inside a hook
/// are checkpointed but don't trigger hooks again, so hooks can't loop on each other.
/// Hooks are code, not state: re-register them after `Thread::open`.
pub type Hook = Box<dyn FnMut(&mut Thread, &Event) -> Result<()> + Send>;

pub struct Thread {
    pub id: String,
    checkpoints: Vec<Checkpoint>,
    context: Context,
    position: Position,
    store: Option<Arc<dyn Checkpointer>>,
    hooks: Vec<Hook>,
}

/// Where the thread is: turn and step numbers, and whether a turn is open.
#[derive(Debug, Clone, Copy, Default)]
struct Position {
    turn: u32,
    step: u32,
    in_turn: bool,
}

impl Position {
    fn after(mut self, event: &Event) -> Self {
        match event {
            Event::TurnStarted => {
                self.turn += 1;
                self.step = 0;
                self.in_turn = true;
            }
            Event::StepStarted => self.step += 1,
            Event::TurnEnded => self.in_turn = false,
            _ => {}
        }
        self
    }
}

impl Thread {
    /// An in-memory thread.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            checkpoints: Vec::new(),
            context: Context::default(),
            position: Position::default(),
            store: None,
            hooks: Vec::new(),
        }
    }

    /// Resume a saved thread, or start it if the store has none by this id.
    pub fn open(id: impl Into<String>, store: Arc<dyn Checkpointer>) -> Result<Self> {
        let mut thread = Self::new(id);
        for (i, checkpoint) in store.load(&thread.id)?.into_iter().enumerate() {
            ensure!(
                checkpoint.id == i,
                "thread {} has a gap at checkpoint {i}",
                thread.id
            );
            thread.replay(checkpoint);
        }
        thread.store = Some(store);
        Ok(thread)
    }

    pub fn on(&mut self, hook: impl FnMut(&mut Thread, &Event) -> Result<()> + Send + 'static) {
        self.hooks.push(Box::new(hook));
    }

    pub fn set_system(&mut self, system: impl Into<String>) -> Result<()> {
        self.record(Event::SystemSet(Some(system.into())))
    }

    pub fn register(&mut self, tool: Tool) -> Result<()> {
        self.record(Event::ToolRegistered(tool))
    }

    pub fn remove_tool(&mut self, name: impl Into<String>) -> Result<()> {
        self.record(Event::ToolRemoved(name.into()))
    }

    /// Opens a turn with the user's message. A turn still open is ended first.
    pub fn start_turn(&mut self, message: Message) -> Result<()> {
        self.end_turn()?;
        self.record(Event::TurnStarted)?;
        self.record(Event::Message(message))
    }

    /// Call before each `chat`: hooks on `StepStarted` shape the context it will see.
    pub fn start_step(&mut self) -> Result<()> {
        ensure!(self.position.in_turn, "start_step outside a turn");
        self.record(Event::StepStarted)
    }

    /// A model reply or tool results.
    pub fn push(&mut self, message: Message) -> Result<()> {
        self.record(Event::Message(message))
    }

    /// Replace a run of context messages, e.g. old turns with a summary. Changes that keep the
    /// start of the context intact keep Anthropic's prompt cache and signed thinking valid.
    pub fn replace(&mut self, range: Range<usize>, with: Vec<Message>) -> Result<()> {
        let len = self.context.messages.len();
        ensure!(
            range.start <= range.end && range.end <= len,
            "replace {range:?} out of bounds for {len} messages"
        );
        self.record(Event::Replaced { range, with })
    }

    /// Records how far Experience has looked, and what it recalled.
    pub fn considered(&mut self, upto: usize, recalled: Vec<Recalled>) -> Result<()> {
        self.record(Event::Considered { upto, recalled })
    }

    /// Where Experience should look from next: just past the last `Considered`.
    pub fn considered_upto(&self) -> usize {
        self.checkpoints
            .iter()
            .rev()
            .find_map(|c| match c.event {
                Event::Considered { upto, .. } => Some(upto),
                _ => None,
            })
            .unwrap_or(0)
    }

    /// Notes where an incoming message was placed, and why.
    pub fn routed(&mut self, by: impl Into<String>, reason: impl Into<String>) -> Result<()> {
        self.record(Event::Routed {
            by: by.into(),
            reason: reason.into(),
        })
    }

    /// Does nothing outside a turn.
    pub fn end_turn(&mut self) -> Result<()> {
        if self.position.in_turn {
            self.record(Event::TurnEnded)?;
        }
        Ok(())
    }

    /// What the model sees now.
    pub fn context(&self) -> &Context {
        &self.context
    }

    /// What the model saw right after checkpoint `id`.
    pub fn context_at(&self, id: usize) -> Context {
        let mut ctx = Context::default();
        for checkpoint in &self.checkpoints[..=id] {
            apply(&mut ctx, &checkpoint.event);
        }
        ctx
    }

    pub fn checkpoints(&self) -> &[Checkpoint] {
        &self.checkpoints
    }

    pub fn turn(&self) -> u32 {
        self.position.turn
    }

    pub fn step(&self) -> u32 {
        self.position.step
    }

    pub fn in_turn(&self) -> bool {
        self.position.in_turn
    }

    /// A new thread holding this one's history up to and including checkpoint `id`, saved to
    /// the same store. Hooks aren't copied.
    pub fn fork(&self, id: usize, new_id: impl Into<String>) -> Result<Thread> {
        let mut thread = Thread::new(new_id);
        let history = &self.checkpoints[..=id];
        if let Some(store) = &self.store {
            store.save(&thread.id, history)?;
            thread.store = Some(store.clone());
        }
        for checkpoint in history {
            thread.replay(checkpoint.clone());
        }
        Ok(thread)
    }

    /// Apply an already-saved checkpoint, without saving or hooks.
    fn replay(&mut self, checkpoint: Checkpoint) {
        self.position = self.position.after(&checkpoint.event);
        apply(&mut self.context, &checkpoint.event);
        self.checkpoints.push(checkpoint);
    }

    fn record(&mut self, event: Event) -> Result<()> {
        let position = self.position.after(&event);
        let checkpoint = Checkpoint {
            id: self.checkpoints.len(),
            turn: position.turn,
            step: position.step,
            at: SystemTime::now(),
            event,
        };
        if let Some(store) = &self.store {
            store.save(&self.id, std::slice::from_ref(&checkpoint))?;
        }
        let event = checkpoint.event.clone();
        self.replay(checkpoint);

        // Taken out while running so hooks can record; nested records find none to fire.
        let mut hooks = std::mem::take(&mut self.hooks);
        let result = hooks.iter_mut().try_for_each(|hook| hook(self, &event));
        hooks.append(&mut self.hooks);
        self.hooks = hooks;
        result
    }
}

fn apply(ctx: &mut Context, event: &Event) {
    match event {
        Event::SystemSet(system) => ctx.system = system.clone(),
        Event::ToolRegistered(tool) => {
            ctx.tools.retain(|t| t.name != tool.name);
            ctx.tools.push(tool.clone());
        }
        Event::ToolRemoved(name) => ctx.tools.retain(|t| t.name != *name),
        Event::Message(message) => ctx.messages.push(message.clone()),
        Event::Replaced { range, with } => {
            ctx.messages.splice(range.clone(), with.iter().cloned());
        }
        Event::Considered { recalled, .. } if !recalled.is_empty() => {
            let lines: Vec<String> = recalled.iter().map(|r| format!("- {}", r.text)).collect();
            let block = Block::Text {
                text: format!("<recalled>\n{}\n</recalled>", lines.join("\n")),
            };
            // Joins the user's message (or tool results) it answers; adds one if there is none.
            match ctx.messages.last_mut() {
                Some(last) if last.role == Role::User => last.content.push(block),
                _ => ctx.messages.push(Message {
                    role: Role::User,
                    content: vec![block],
                }),
            }
        }
        Event::TurnStarted
        | Event::StepStarted
        | Event::TurnEnded
        | Event::Considered { .. }
        | Event::Routed { .. } => {}
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use context::Block;
    use serde_json::json;

    pub fn weather() -> Tool {
        Tool {
            name: "weather".into(),
            description: "Current weather for a city".into(),
            input_schema: json!({ "type": "object" }),
        }
    }

    fn call(id: &str) -> Message {
        Message {
            role: context::Role::Assistant,
            content: vec![Block::ToolCall {
                id: id.into(),
                name: "weather".into(),
                input: json!({ "city": "Paris" }),
            }],
        }
    }

    /// One turn, two steps: tool call then answer.
    pub fn tool_turn(thread: &mut Thread, question: &str) {
        thread.start_turn(Message::user(question)).unwrap();
        thread.start_step().unwrap();
        thread.push(call("c1")).unwrap();
        thread.push(Message::tool_result("c1", "18C")).unwrap();
        thread.start_step().unwrap();
        thread.push(Message::assistant("18C and sunny.")).unwrap();
        thread.end_turn().unwrap();
    }

    #[test]
    fn turns_and_steps_are_tracked() {
        let mut thread = Thread::new("t1");
        thread.set_system("Be terse.").unwrap();
        thread.register(weather()).unwrap();
        tool_turn(&mut thread, "Weather in Paris?");

        let ctx = thread.context();
        assert_eq!(ctx.system.as_deref(), Some("Be terse."));
        assert_eq!(ctx.tools.len(), 1);
        assert_eq!(ctx.messages.len(), 4);

        let at: Vec<_> = thread
            .checkpoints()
            .iter()
            .map(|c| (c.turn, c.step))
            .collect();
        assert_eq!(
            at,
            [
                (0, 0),
                (0, 0),
                (1, 0),
                (1, 0),
                (1, 1),
                (1, 1),
                (1, 1),
                (1, 2),
                (1, 2),
                (1, 2)
            ]
        );
        assert!(!thread.in_turn());
        assert!(thread.start_step().is_err());
    }

    #[test]
    fn replace_keeps_history() {
        let mut thread = Thread::new("t1");
        tool_turn(&mut thread, "Weather in Paris?");
        let before = thread.checkpoints().len() - 1;

        thread
            .replace(0..4, vec![Message::user("Summary: Paris is 18C.")])
            .unwrap();
        assert_eq!(
            thread.context().messages,
            [Message::user("Summary: Paris is 18C.")]
        );
        assert_eq!(thread.context_at(before).messages.len(), 4);
        assert!(thread.replace(0..5, vec![]).is_err());
    }

    #[test]
    fn tools_can_be_swapped() {
        let mut thread = Thread::new("t1");
        thread.register(weather()).unwrap();
        thread.register(weather()).unwrap();
        assert_eq!(thread.context().tools.len(), 1);
        thread.remove_tool("weather").unwrap();
        assert!(thread.context().tools.is_empty());
    }

    #[test]
    fn fork_branches_from_a_checkpoint() {
        let mut thread = Thread::new("t1");
        tool_turn(&mut thread, "Weather in Paris?");
        let after_first = thread.checkpoints().len() - 1;
        tool_turn(&mut thread, "And tomorrow?");

        let mut branch = thread.fork(after_first, "t2").unwrap();
        assert_eq!(branch.context(), &thread.context_at(after_first));
        assert_eq!(branch.turn(), 1);
        branch.start_turn(Message::user("And in Rome?")).unwrap();
        assert_eq!(branch.turn(), 2);
        assert_eq!(thread.context().messages.len(), 8);
        assert_eq!(branch.context().messages.len(), 5);
    }

    #[test]
    fn recalled_joins_the_users_message() {
        let mut thread = Thread::new("t1");
        thread.start_turn(Message::user("What's my name?")).unwrap();
        assert_eq!(thread.considered_upto(), 0);
        let upto = thread.checkpoints().len();
        let recalled = vec![Recalled {
            id: 7,
            text: "The user's name is Jon.".into(),
        }];
        thread.considered(upto, recalled).unwrap();
        assert_eq!(thread.considered_upto(), upto);

        let messages = &thread.context().messages;
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].text(),
            "What's my name?<recalled>\n- The user's name is Jon.\n</recalled>"
        );
    }

    /// A hook that compacts before each step once the context passes 4 messages.
    #[test]
    fn hooks_shape_context_before_a_step() {
        let mut thread = Thread::new("t1");
        thread.on(|thread, event| {
            let len = thread.context().messages.len();
            if *event == Event::StepStarted && len > 4 {
                // Keep the current turn's opening message; summarize everything before it.
                let summary = Message::user(format!("[{} earlier messages summarized]", len - 1));
                thread.replace(0..len - 1, vec![summary])?;
            }
            Ok(())
        });
        tool_turn(&mut thread, "Weather in Paris?");
        thread.start_turn(Message::user("And tomorrow?")).unwrap();
        thread.start_step().unwrap();

        assert_eq!(
            thread.context().messages,
            [
                Message::user("[4 earlier messages summarized]"),
                Message::user("And tomorrow?"),
            ]
        );
        let last = thread.checkpoints().last().unwrap();
        assert!(matches!(last.event, Event::Replaced { .. }));
        assert_eq!((last.turn, last.step), (2, 1));
    }
}
