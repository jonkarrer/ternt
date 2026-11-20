//! A thread's inbox and its submissions, as engine documents changed inside commits.

use anyhow::{Context as _, Result};
use context::Message;
use engine::{TaskId, TaskOptions, Tx};
use serde::{Deserialize, Serialize};

use crate::generation::NAME as GENERATION;
use crate::idle::NAME as IDLE;
use crate::{Input, Settled, Status, Submission, WhenBusy};

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

fn inbox_key(thread: &str) -> String {
    format!("agent/inbox/{thread}")
}

pub(crate) fn request_key(request_id: &str) -> String {
    format!("agent/request/{request_id}")
}

pub(crate) fn load(tx: &mut Tx, thread: &str) -> Result<Inbox> {
    Ok(tx.doc(&inbox_key(thread))?.unwrap_or_default())
}

pub(crate) fn save(tx: &mut Tx, thread: &str, inbox: &Inbox) -> Result<()> {
    tx.put_doc(&inbox_key(thread), inbox)
}

fn submission(tx: &mut Tx, id: &str) -> Result<Submission> {
    tx.doc(&submission_key(id))?
        .with_context(|| format!("no submission {id}"))
}

fn update(tx: &mut Tx, id: &str, change: impl FnOnce(&mut Submission)) -> Result<()> {
    let mut submission = submission(tx, id)?;
    change(&mut submission);
    tx.put_doc(&submission_key(id), &submission)
}

pub(crate) fn submit(tx: &mut Tx, thread: &str, input: Input) -> Result<String> {
    if let Some(request_id) = &input.request_id
        && let Some(existing) = tx.doc::<String>(&request_key(request_id))?
    {
        return Ok(existing);
    }
    let n: u64 = tx.doc("agent/next-submission")?.unwrap_or(1);
    tx.put_doc("agent/next-submission", &(n + 1))?;
    let id = format!("s{n}");
    if let Some(request_id) = &input.request_id {
        tx.put_doc(&request_key(request_id), &id)?;
    }
    tx.put_doc(
        &submission_key(&id),
        &Submission {
            id: id.clone(),
            thread: thread.to_string(),
            text: input.text,
            request_id: input.request_id,
            when_busy: input.when_busy,
            status: Status::Queued,
            run: None,
            answer: None,
            reason: None,
        },
    )?;

    let mut inbox = reconcile(tx, thread)?;
    inbox.queue.push(id.clone());
    if inbox.run.is_none() {
        start_next(tx, thread, &mut inbox)?;
    }
    save(tx, thread, &inbox)?;
    Ok(id)
}

/// A run that ended without settling its inputs (it faulted, say) leaves them unanswered and
/// lets the queue move on.
fn reconcile(tx: &mut Tx, thread: &str) -> Result<Inbox> {
    let mut inbox = load(tx, thread)?;
    if let Some(run) = inbox.run
        && tx.task(run).is_none()
    {
        let reason = "the run ended without an answer".to_string();
        settle_placed(tx, &mut inbox, Settled::Unanswered { reason })?;
        inbox.run = None;
        tx.thread(thread)?.end_turn()?;
    }
    Ok(inbox)
}

/// Starts a run for the oldest queued submission, if any.
pub(crate) fn start_next(tx: &mut Tx, thread: &str, inbox: &mut Inbox) -> Result<()> {
    if inbox.queue.is_empty() {
        return Ok(());
    }
    if let Some(idle) = inbox.idle.take() {
        tx.abort_task(idle, "a new run started")?;
    }
    let id = inbox.queue.remove(0);
    let text = submission(tx, &id)?.text;
    tx.thread(thread)?.start_turn(Message::user(text))?;
    let run = tx.create_task_named(
        GENERATION,
        thread,
        serde_json::Value::Null,
        TaskOptions::default(),
    )?;
    update(tx, &id, |s| {
        s.status = Status::Placed;
        s.run = Some(run);
    })?;
    inbox.run = Some(run);
    inbox.placed = vec![id];
    Ok(())
}

/// Places queued steers into the running turn, after its tool round. Returns their texts.
pub(crate) fn place_steers(tx: &mut Tx, thread: &str, inbox: &mut Inbox) -> Result<()> {
    let run = inbox.run;
    let mut kept = Vec::new();
    for id in std::mem::take(&mut inbox.queue) {
        let queued = submission(tx, &id)?;
        if queued.when_busy != WhenBusy::Steer {
            kept.push(id);
            continue;
        }
        tx.thread(thread)?.push(Message::user(queued.text))?;
        update(tx, &id, |s| {
            s.status = Status::Placed;
            s.run = run;
        })?;
        inbox.placed.push(id);
    }
    inbox.queue = kept;
    Ok(())
}

/// Ends the run: closes the turn, settles its inputs, and starts the next queued one. With
/// `withdraw`, the queue is settled unanswered too, and nothing starts.
pub(crate) fn end_run(tx: &mut Tx, thread: &str, settled: Settled, withdraw: bool) -> Result<()> {
    let mut inbox = load(tx, thread)?;
    tx.thread(thread)?.end_turn()?;
    settle_placed(tx, &mut inbox, settled.clone())?;
    inbox.run = None;
    if withdraw {
        for id in std::mem::take(&mut inbox.queue) {
            settle(tx, &id, settled.clone())?;
        }
    } else {
        start_next(tx, thread, &mut inbox)?;
    }
    if inbox.run.is_none() {
        let since = serde_json::json!({ "since": tx.now() });
        let idle = tx.create_task_named(IDLE, thread, since, TaskOptions::default())?;
        inbox.idle = Some(idle);
    }
    save(tx, thread, &inbox)
}

fn settle_placed(tx: &mut Tx, inbox: &mut Inbox, settled: Settled) -> Result<()> {
    for id in std::mem::take(&mut inbox.placed) {
        settle(tx, &id, settled.clone())?;
    }
    Ok(())
}

fn settle(tx: &mut Tx, id: &str, settled: Settled) -> Result<()> {
    update(tx, id, |s| match settled {
        Settled::Done { answer } => {
            s.status = Status::Done;
            s.answer = Some(answer);
        }
        Settled::Unanswered { reason } => {
            s.status = Status::Unanswered;
            s.reason = Some(reason);
        }
    })
}
