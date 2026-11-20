//! One tool call, as its own task: its intent is committed before it runs, so a crash
//! mid-call is noticed, and only a replay-safe tool runs again.

use anyhow::{Result, bail};
use engine::{Next, Runtime, Task};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Replay, Tools};

pub struct ToolCall {
    tools: Tools,
}

impl ToolCall {
    pub fn new(tools: Tools) -> Self {
        Self { tools }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallInput {
    pub call_id: String,
    pub name: String,
    pub arguments: Value,
}

/// What the model gets back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolOutput {
    pub content: String,
    pub is_error: bool,
}

impl ToolOutput {
    pub fn error(content: String) -> Self {
        Self {
            content,
            is_error: true,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum State {
    /// Check the tool exists, then commit the intent to run it.
    Call,
    /// Run it. Reached again only after a crash, when `replay` decides.
    Execute { replay: Replay },
}

impl Task for ToolCall {
    type Input = ToolCallInput;
    type State = State;
    type Output = ToolOutput;
    const NAME: &'static str = "agent.tool";

    fn initial(&self, _: &ToolCallInput) -> State {
        State::Call
    }

    async fn run(&self, rt: Runtime<Self>) -> Result<()> {
        let call = rt.input();
        let tool = self.tools.get(&call.name);
        match (rt.state(), tool) {
            (_, None) => {
                let output = ToolOutput::error(format!("tool {} is not available", call.name));
                rt.commit(|_| Ok(Some(Next::Complete(output))))
            }
            (State::Call, Some(tool)) => {
                let replay = tool.replay();
                rt.commit(|_| Ok(Some(Next::Continue(State::Execute { replay }))))
            }
            (&State::Execute { replay }, Some(tool)) => {
                let safe = replay == Replay::Safe && tool.replay() == Replay::Safe;
                if rt.restarts() > 0 && !safe {
                    let output = ToolOutput::error(format!(
                        "tool {} was interrupted and may have partially run; it was not run again",
                        call.name
                    ));
                    return rt.commit(|_| Ok(Some(Next::Complete(output))));
                }
                let result = tokio::select! {
                    result = tool.execute(call.arguments.clone()) => result,
                    _ = rt.cancelled() => bail!("cancelled"),
                };
                let output = match result {
                    Ok(content) => ToolOutput {
                        content,
                        is_error: false,
                    },
                    Err(error) => ToolOutput::error(format!("{error:#}")),
                };
                rt.commit(|_| Ok(Some(Next::Complete(output))))
            }
        }
    }
}
