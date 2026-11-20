//! A thread's idle time, as a task: each finished run starts one, the next run aborts it. It
//! sleeps durably (the deadline is stored, so a restart doesn't lose it), dreams, sleeps again,
//! then settles the thread into its resume card.

use std::sync::Arc;

use anyhow::Result;
use engine::{Next, Runtime, Task};
use serde::{Deserialize, Serialize};

use crate::{Mind, Settings, Work, think};

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

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
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
        let Some(mind) = self.mind.as_deref() else {
            return rt.commit(|_| Ok(Some(Next::Complete(()))));
        };
        let (after, work, next): (_, (&str, Work), _) = match rt.state() {
            State::Dream => (
                self.settings.dream_after_ms,
                ("dream", Mind::dream as Work),
                Next::Continue(State::Settle),
            ),
            State::Settle => (
                self.settings.settle_after_ms,
                ("settle", Mind::settle as Work),
                Next::Complete(()),
            ),
        };
        let at = rt.input().since + after;
        if rt.now() < at {
            return rt.sleep_until(at, *rt.state());
        }
        think(&rt, mind, work, next).await
    }
}
