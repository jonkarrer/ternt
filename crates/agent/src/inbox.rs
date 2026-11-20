//! A thread's inbox and its submissions, as engine documents changed inside commits.

use anyhow::{Context as _, Result};
use context::Message;
use engine::{TaskId, TaskOptions, Tx};
use serde::{Deserialize, Serialize};

use crate::generation::{NAME as GENERATION, close_calls};
use crate::idle::NAME as IDLE;
use crate::{Draft, Record, Settled, Status, Submission, WhenBusy};

/// What a thread is working on, and what waits for it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct Inbox {
    /// The run in progress.
    pub run: Option<TaskId>,
    /// Submissions the run answers.
    pub placed: Vec<String>,
    /// Submissions waiting, oldest first.
    pub queue: Vec<String>,
    /// The idle task counting down to dreaming and settling, between runs.
    #[serde(default)]
    pub idle: Option<TaskId>,
}

pub(crate) fn submission_key(id: &str) -> String {
    format!("agent/submission/{id}")
}

pub(crate) fn inbox_key(thread: &str) -> String {
    format!("agent/inbox/{thread}")
}

pub(crate) fn request_key(request_id: &str) -> String {
    format!("agent/request/{request_id}")
}

fn load(tx: &mut Tx, thread: &str) -> Result<Inbox> {
    Ok(tx.doc(&inbox_key(thread))?.unwrap_or_default())
}

fn save(tx: &mut Tx, thread: &str, inbox: &Inbox) -> Result<()> {
    tx.put_doc(&inbox_key(thread), inbox)
}

pub(crate) fn submission(tx: &mut Tx, id: &str) -> Result<Record> {
    tx.doc(&submission_key(id))?
        .with_context(|| format!("no submission {id}"))
}

fn update(tx: &mut Tx, id: &str, change: impl FnOnce(&mut Record)) -> Result<()> {
    let mut submission = submission(tx, id)?;
    change(&mut submission);
    tx.put_doc(&submission_key(id), &submission)
}

/// The submission the input's request ID already made, if any.
pub(crate) fn existing(tx: &mut Tx, input: &Draft) -> Result<Option<String>> {
    match &input.request_id {
        Some(request_id) => tx.doc(&request_key(request_id)),
        None => Ok(None),
    }
}

pub(crate) fn submit(tx: &mut Tx, thread: &str, input: Draft) -> Result<String> {
    if let Some(id) = existing(tx, &input)? {
        return Ok(id);
    }
    let n: u64 = tx.doc("agent/next-submission")?.unwrap_or(1);
    tx.put_doc("agent/next-submission", &(n + 1))?;
    let id = format!("s{n}");
    if let Some(request_id) = &input.request_id {
        tx.put_doc(&request_key(request_id), &id)?;
    }
    tx.put_doc(
        &submission_key(&id),
        &Record {
            shown: Submission {
                id: id.clone(),
                thread: thread.to_string(),
                text: input.text,
                status: Status::Queued,
                answer: None,
                reason: None,
            },
            request_id: input.request_id,
            when_busy: input.when_busy,
            run: None,
        },
    )?;

    let mut inbox = load(tx, thread)?;
    inbox.queue.push(id.clone());
    match inbox.run {
        None => start_next(tx, thread, &mut inbox)?,
        Some(_) => recover_in(tx, thread, &mut inbox, "the run ended without an answer")?,
    }
    save(tx, thread, &inbox)?;
    Ok(id)
}

/// See `recover_in`.
pub(crate) fn recover(tx: &mut Tx, thread: &str, reason: &str) -> Result<()> {
    let mut inbox = load(tx, thread)?;
    recover_in(tx, thread, &mut inbox, reason)?;
    save(tx, thread, &inbox)
}

/// A run that ended without settling its inputs (it faulted, or was orphaned) never will:
/// they're settled unanswered with `reason`, and the queue moves on. Nothing while it's live.
fn recover_in(tx: &mut Tx, thread: &str, inbox: &mut Inbox, reason: &str) -> Result<()> {
    match inbox.run {
        Some(run) if tx.task(run).is_none() => {
            finish(tx, thread, inbox, Settled::unanswered(reason), false)
        }
        _ => Ok(()),
    }
}

/// Starts a run for the oldest queued submission, or, with none queued, the idle countdown.
fn start_next(tx: &mut Tx, thread: &str, inbox: &mut Inbox) -> Result<()> {
    if inbox.queue.is_empty() {
        let since = serde_json::json!({ "since": tx.now() });
        inbox.idle = Some(tx.create_task_named(IDLE, thread, since, TaskOptions::default())?);
        return Ok(());
    }
    if let Some(idle) = inbox.idle.take() {
        tx.abort_task(idle, "a new run started")?;
    }
    let id = inbox.queue.remove(0);
    let text = submission(tx, &id)?.shown.text;
    tx.thread(thread)?.start_turn(Message::user(text))?;
    let run = tx.create_task_named(
        GENERATION,
        thread,
        serde_json::Value::Null,
        TaskOptions::default(),
    )?;
    update(tx, &id, |s| {
        s.shown.status = Status::Placed;
        s.run = Some(run);
    })?;
    inbox.run = Some(run);
    inbox.placed = vec![id];
    Ok(())
}

/// Places queued steers into the running turn, after its tool round.
pub(crate) fn place_steers(tx: &mut Tx, thread: &str) -> Result<()> {
    let mut inbox = load(tx, thread)?;
    let mut kept = Vec::new();
    for id in std::mem::take(&mut inbox.queue) {
        let queued = submission(tx, &id)?;
        if queued.when_busy != WhenBusy::Steer {
            kept.push(id);
            continue;
        }
        tracing::info!(thread, submission = %id, text = %queued.shown.text, "steer placed");
        tx.thread(thread)?.push(Message::user(queued.shown.text))?;
        update(tx, &id, |s| {
            s.shown.status = Status::Placed;
            s.run = inbox.run;
        })?;
        inbox.placed.push(id);
    }
    inbox.queue = kept;
    save(tx, thread, &inbox)
}

/// Ends the run: settles its inputs and starts the next queued one. With `withdraw`, the queue
/// is settled the same way, and the thread goes idle.
pub(crate) fn end_run(tx: &mut Tx, thread: &str, settled: Settled, withdraw: bool) -> Result<()> {
    let mut inbox = load(tx, thread)?;
    finish(tx, thread, &mut inbox, settled, withdraw)?;
    save(tx, thread, &inbox)
}

/// Leaves the thread valid (no tool call without a result, no open turn), settles the run's
/// inputs, and moves on.
fn finish(
    tx: &mut Tx,
    thread: &str,
    inbox: &mut Inbox,
    settled: Settled,
    withdraw: bool,
) -> Result<()> {
    let open = tx.thread(thread)?;
    close_calls(open, "the run ended before this call returned")?;
    open.end_turn()?;
    inbox.run = None;
    let mut ended = std::mem::take(&mut inbox.placed);
    if withdraw {
        ended.append(&mut inbox.queue);
    }
    for id in ended {
        settle(tx, &id, settled.clone())?;
    }
    start_next(tx, thread, inbox)
}

fn settle(tx: &mut Tx, id: &str, settled: Settled) -> Result<()> {
    let thread = submission(tx, id)?.shown.thread;
    match &settled {
        Settled::Done { answer } => {
            tracing::info!(thread, submission = id, answer, "input answered")
        }
        Settled::Unanswered { reason } => {
            tracing::warn!(thread, submission = id, reason, "input unanswered")
        }
    }
    update(tx, id, |s| match settled {
        Settled::Done { answer } => {
            s.shown.status = Status::Done;
            s.shown.answer = Some(answer);
        }
        Settled::Unanswered { reason } => {
            s.shown.status = Status::Unanswered;
            s.shown.reason = Some(reason);
        }
    })
}
