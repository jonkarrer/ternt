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
        let output = match (rt.state(), self.tools.get(&call.name)) {
            (_, None) => {
                tracing::warn!(tool = %call.name, "the model called a tool that isn't available");
                ToolOutput::error(format!("tool {} is not available", call.name))
            }
            // Like pi-durable: the intent is committed and the tool runs in this same phase, so
            // `Execute` is only ever reached when an invocation was cut off mid-call, by a crash
            // or a close alike, and the replay rule decides.
            (State::Call, Some(tool)) => {
                let replay = tool.replay();
                rt.commit(|_| Ok(Some(Next::Continue(State::Execute { replay }))))?;
                execute(&rt, tool.as_ref()).await?
            }
            (&State::Execute { replay }, Some(tool))
                if replay == Replay::Unsafe || tool.replay() == Replay::Unsafe =>
            {
                tracing::warn!(
                    tool = %call.name,
                    call_id = %call.call_id,
                    "unsafe tool call was interrupted; not run again"
                );
                ToolOutput::error(format!(
                    "tool {} was interrupted and may have partially run; it was not run again",
                    call.name
                ))
            }
            (State::Execute { .. }, Some(tool)) => execute(&rt, tool.as_ref()).await?,
        };
        rt.commit(|_| Ok(Some(Next::Complete(output))))
    }
}

async fn execute(rt: &Runtime<ToolCall>, tool: &dyn crate::Tool) -> Result<ToolOutput> {
    let call = rt.input();
    let started = std::time::Instant::now();
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
    tracing::info!(
        tool = %call.name,
        call_id = %call.call_id,
        arguments = %call.arguments,
        ms = started.elapsed().as_millis() as u64,
        is_error = output.is_error,
        output = %output.content,
        "tool call"
    );
    Ok(output)
}
