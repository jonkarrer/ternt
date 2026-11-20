//! A thread's idle time, as a task: each finished run starts one, the next run aborts it. It
//! sleeps durably (the deadline is stored, so a restart doesn't lose it), dreams, sleeps again,
//! then settles the thread into its resume card.

use std::sync::Arc;

use anyhow::Result;
use engine::{Next, Runtime, Task};
use serde::{Deserialize, Serialize};

use crate::{Mind, Settings, Work, apply, on_copy};

pub(crate) const NAME: &str = "agent.idle";

pub struct Idle {
    mind: Option<Arc<dyn Mind>>,
    settings: Settings,
}

impl Idle {
    pub fn new(mind: Option<Arc<dyn Mind>>, settings: Settings) -> Self {
        Self { mind, settings }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IdleInput {
    /// When the run ended, ms since the epoch.
    pub since: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum State {
    Dream,
    Settle,
}

impl Task for Idle {
    type Input = IdleInput;
    type State = State;
    type Output = ();
    const NAME: &'static str = NAME;

    fn initial(&self, _: &IdleInput) -> State {
        State::Dream
    }

    async fn run(&self, rt: Runtime<Self>) -> Result<()> {
        let Some(mind) = &self.mind else {
            return rt.commit(|_| Ok(Some(Next::Complete(()))));
        };
        let since = rt.input().since;
        let (at, next) = match rt.state() {
            State::Dream => (since + self.settings.dream_after_ms, Some(State::Settle)),
            State::Settle => (since + self.settings.settle_after_ms, None),
        };
        if rt.now() < at {
            let state = match rt.state() {
                State::Dream => State::Dream,
                State::Settle => State::Settle,
            };
            return rt.sleep_until(at, state);
        }
        let work = match rt.state() {
            State::Dream => Work::Dream,
            State::Settle => Work::Settle,
        };
        let work = on_copy(&rt, mind.as_ref(), work).await;
        let thread = rt.thread_id().to_string();
        rt.commit(|tx| {
            // A failed dream or settle is skipped, not fatal: the thread just stays as it is.
            if let Ok((base, events)) = work {
                apply(tx, &thread, base, events)?;
            }
            Ok(Some(match next {
                Some(state) => Next::Continue(state),
                None => Next::Complete(()),
            }))
        })
    }
}
