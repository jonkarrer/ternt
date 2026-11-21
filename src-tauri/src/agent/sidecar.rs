//! The agent process and Ternt's end of the sidecar protocol (`wire.rs`,
//! docs/AGENT-SIDECAR.md): start it, send it requests, read its events, and
//! start it again when it dies.
//!
//! Why a process: Ternt's release profile aborts on panic, so a panic inside
//! an agent (or SQLite, or running out of memory) ends whatever process it
//! runs in. In its own process that is the agent, not the browser. The
//! supervisor restarts it with a short back-off, tells the panel
//! (`agent:status`), and gives up after `BACKOFF.len()` deaths in
//! `RESTART_WINDOW`, leaving a Restart button.
//!
//! The binary is `ternt-agent` beside Ternt's own executable (Tauri's
//! `externalBin`; `TERNT_AGENT_BIN` overrides the path): the stub from
//! `src/bin/ternt-agent` today, tokidoki's `serve` later.

use std::{
    collections::HashMap,
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, EventTarget};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{ChildStdin, Command},
    sync::{Mutex as AsyncMutex, oneshot},
};

use super::{
    AGENT_WEBVIEW, Agent, Outbox, Reply, keys,
    protocol::*,
    wire::{Frame, Request},
};

/// How long a request may take (a model call is not a request: `send`
/// returns as soon as the input is placed).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// How long the process has to say it is ready after starting.
const READY_TIMEOUT: Duration = Duration::from_secs(15);
/// Pauses before each restart. One more death within `RESTART_WINDOW` than
/// there are entries here, and the process stays down.
const BACKOFF: [Duration; 4] = [
    Duration::ZERO,
    Duration::from_secs(1),
    Duration::from_secs(3),
    Duration::from_secs(10),
];
const RESTART_WINDOW: Duration = Duration::from_secs(120);

/// What the panel shows about the process (`agent:status`).
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum Status {
    Starting,
    #[serde(rename_all = "camelCase")]
    Running {
        /// `stub` or `tokidoki`, from its first line.
        backend: String,
        /// It came back after dying: the panel reloads what it shows.
        restarted: bool,
        /// It was given a key it can answer with (Fireworks or Anthropic).
        has_key: bool,
    },
    Restarting {
        /// Which restart this is within the budget; 0 when `asked`.
        attempt: u32,
        /// Restarted on purpose (the Restart button, a key change), not a death.
        asked: bool,
    },
    Down {
        reason: String,
    },
}

pub struct Sidecar {
    app: AppHandle,
    outbox: Outbox,
    /// Passed as `--data`: where agents keep their files.
    data: PathBuf,
    stdin: AsyncMutex<Option<ChildStdin>>,
    pending: Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>,
    next_id: AtomicU64,
    status: Mutex<Status>,
    /// When the process died, for the restart budget.
    deaths: Mutex<Vec<Instant>>,
    pid: AtomicU32,
    /// A `run` loop is going (so `restart` knows whether to start one).
    running: AtomicBool,
    /// The next exit is a restart Ternt asked for, not a death.
    asked: AtomicBool,
}

/// The agent process to start.
pub fn binary() -> PathBuf {
    if let Some(path) = std::env::var_os("TERNT_AGENT_BIN") {
        return path.into();
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("ternt-agent")))
        .unwrap_or_else(|| "ternt-agent".into())
}

/// The space a folder name stands for (`space-7` → 7).
pub fn space_id(folder: &str) -> Option<u32> {
    folder.strip_prefix("space-")?.parse().ok()
}

/// Record a death and return which restart this would be (1-based), after
/// forgetting deaths older than `RESTART_WINDOW`.
fn restart_attempt(deaths: &mut Vec<Instant>, now: Instant) -> u32 {
    deaths.retain(|at| now.duration_since(*at) < RESTART_WINDOW);
    deaths.push(now);
    deaths.len() as u32
}

fn signal(pid: u32, sig: &str) {
    if pid != 0 {
        let _ = std::process::Command::new("kill")
            .arg(sig)
            .arg(pid.to_string())
            .status();
    }
}

impl Sidecar {
    pub fn new(app: AppHandle, outbox: Outbox, data: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            app,
            outbox,
            data,
            stdin: AsyncMutex::new(None),
            pending: Mutex::default(),
            next_id: AtomicU64::new(0),
            status: Mutex::new(Status::Starting),
            deaths: Mutex::default(),
            pid: AtomicU32::new(0),
            running: AtomicBool::new(false),
            asked: AtomicBool::new(false),
        })
    }

    /// Start the process and keep it running.
    pub fn start(self: &Arc<Self>) {
        if self.running.swap(true, Ordering::AcqRel) {
            return;
        }
        let me = self.clone();
        tauri::async_runtime::spawn(async move { me.run().await });
    }

    async fn run(self: Arc<Self>) {
        let mut restarted = false;
        loop {
            if let Err(reason) = self.spawn_once(restarted).await {
                self.set_status(Status::Down { reason });
                break;
            }
            if self.asked.swap(false, Ordering::AcqRel) {
                self.fail_pending("The agent was restarted.");
                self.set_status(Status::Restarting {
                    attempt: 0,
                    asked: true,
                });
                restarted = false;
                continue;
            }
            self.fail_pending("The agent process exited.");
            let attempt = restart_attempt(&mut self.deaths.lock().unwrap(), Instant::now());
            if attempt as usize > BACKOFF.len() {
                self.set_status(Status::Down {
                    reason: format!(
                        "The agent crashed {} times in two minutes and was left off.",
                        BACKOFF.len() + 1
                    ),
                });
                break;
            }
            self.set_status(Status::Restarting {
                attempt,
                asked: false,
            });
            tokio::time::sleep(BACKOFF[attempt as usize - 1]).await;
            restarted = true;
        }
        self.running.store(false, Ordering::Release);
    }

    /// Run the process until it exits. `Err` when it couldn't be started or
    /// never said it was ready (restarting won't help).
    async fn spawn_once(&self, restarted: bool) -> Result<(), String> {
        let bin = binary();
        let _ = std::fs::create_dir_all(&self.data);
        // Only an exit of the process started here can be one `restart` asked for.
        self.asked.store(false, Ordering::Release);
        // The user's keys reach the agent this way and no other.
        let keys = keys::load();
        let has_key = keys
            .iter()
            .any(|(variable, _)| keys::ANSWERING.contains(variable));
        let mut child = Command::new(&bin)
            .arg("--data")
            .arg(&self.data)
            .envs(keys)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|err| format!("Couldn't start the agent ({}): {err}", bin.display()))?;
        self.pid.store(child.id().unwrap_or(0), Ordering::Relaxed);
        let stdin = child.stdin.take().ok_or("the agent process has no stdin")?;
        let stdout = child
            .stdout
            .take()
            .ok_or("the agent process has no stdout")?;
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    eprintln!("ternt-agent: {line}");
                }
            });
        }

        let mut lines = BufReader::new(stdout).lines();
        let backend = match tokio::time::timeout(READY_TIMEOUT, lines.next_line()).await {
            Ok(Ok(Some(line))) => match serde_json::from_str(&line) {
                Ok(Frame::Ready { ready }) => ready.backend,
                _ => {
                    let _ = child.start_kill();
                    return Err(format!(
                        "The agent process didn't introduce itself (said: {line})"
                    ));
                }
            },
            _ => {
                let _ = child.start_kill();
                return Err("The agent process didn't say it was ready.".into());
            }
        };
        *self.stdin.lock().await = Some(stdin);
        self.set_status(Status::Running {
            backend,
            restarted,
            has_key,
        });

        while let Ok(Some(line)) = lines.next_line().await {
            self.on_line(&line);
        }
        *self.stdin.lock().await = None;
        self.pid.store(0, Ordering::Relaxed);
        let _ = child.wait().await;
        Ok(())
    }

    fn on_line(&self, line: &str) {
        match serde_json::from_str::<Frame>(line) {
            Ok(Frame::Event { space, event }) => {
                if let Some(space) = space_id(&space) {
                    self.outbox.push(space, event);
                }
            }
            Ok(Frame::Response { id, result, error }) => {
                if let Some(reply) = self.pending.lock().unwrap().remove(&id) {
                    let _ = reply.send(match error {
                        Some(error) => Err(error),
                        None => Ok(result.unwrap_or(Value::Null)),
                    });
                }
            }
            Ok(Frame::Ready { .. }) => {}
            Err(err) => eprintln!("ternt-agent: unreadable line ({err}): {line}"),
        }
    }

    fn fail_pending(&self, reason: &str) {
        for (_, reply) in self.pending.lock().unwrap().drain() {
            let _ = reply.send(Err(reason.into()));
        }
    }

    fn set_status(&self, status: Status) {
        *self.status.lock().unwrap() = status.clone();
        let _ = self
            .app
            .emit_to(EventTarget::webview(AGENT_WEBVIEW), "agent:status", status);
    }

    pub fn status(&self) -> Status {
        self.status.lock().unwrap().clone()
    }

    /// What answers, once the process is up.
    pub fn backend(&self) -> Option<String> {
        match self.status() {
            Status::Running { backend, .. } => Some(backend),
            _ => None,
        }
    }

    fn unavailable(&self) -> String {
        match self.status() {
            Status::Down { reason } => reason,
            Status::Restarting { .. } => "The agent is restarting; try again in a moment.".into(),
            Status::Starting => "The agent is still starting.".into(),
            Status::Running { .. } => "The agent process isn't reachable.".into(),
        }
    }

    /// One request, answered or failed within `REQUEST_TIMEOUT`.
    pub async fn call(
        &self,
        space: Option<String>,
        method: &str,
        params: Value,
    ) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let (reply, answer) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, reply);
        let request = Request {
            id,
            method: method.into(),
            space,
            params,
        };
        let mut line = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
        line.push(b'\n');
        {
            let mut stdin = self.stdin.lock().await;
            let Some(stdin) = stdin.as_mut() else {
                self.pending.lock().unwrap().remove(&id);
                return Err(self.unavailable());
            };
            if let Err(err) = stdin.write_all(&line).await {
                self.pending.lock().unwrap().remove(&id);
                return Err(format!("Couldn't reach the agent process: {err}"));
            }
        }
        match tokio::time::timeout(REQUEST_TIMEOUT, answer).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("The agent process exited.".into()),
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                Err("The agent didn't answer in time.".into())
            }
        }
    }

    /// Start over, on purpose (the Restart button, a key change): stop a
    /// running process (the loop brings it back, with the keys read again),
    /// or start one when it was left down.
    pub fn restart(self: &Arc<Self>) {
        self.deaths.lock().unwrap().clear();
        let pid = self.pid.load(Ordering::Relaxed);
        if pid != 0 {
            self.asked.store(true, Ordering::Release);
            // A TERM lets it finish what it is writing (tokidoki stops its
            // agents cleanly in milliseconds); a KILL follows if it won't go.
            signal(pid, "-TERM");
            let me = self.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_millis(300)).await;
                if me.pid.load(Ordering::Relaxed) == pid {
                    signal(pid, "-KILL");
                }
            });
        } else if !self.running.load(Ordering::Acquire) {
            self.set_status(Status::Starting);
            self.start();
        }
    }

    /// Stop the process before Ternt exits: a TERM to let it finish writing,
    /// then a KILL. (Runs on the main thread at exit, so no async here.)
    pub fn shutdown(&self) {
        let pid = self.pid.load(Ordering::Relaxed);
        if pid == 0 {
            return;
        }
        signal(pid, "-TERM");
        std::thread::sleep(Duration::from_millis(300));
        signal(pid, "-KILL");
    }
}

fn parse<T: DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value)
        .map_err(|err| format!("The agent answered with something unexpected: {err}"))
}

/// A space's agent, in the sidecar.
pub struct Remote {
    sidecar: Arc<Sidecar>,
    /// Its folder name (`space-7`).
    space: String,
    /// The space's name, passed along for the agent to use.
    label: String,
}

impl Remote {
    pub fn new(sidecar: Arc<Sidecar>, space: String, label: String) -> Self {
        Self {
            sidecar,
            space,
            label,
        }
    }

    fn call<T: DeserializeOwned + Send + 'static>(
        &self,
        method: &'static str,
        params: Value,
    ) -> Reply<T> {
        let sidecar = self.sidecar.clone();
        let space = self.space.clone();
        Box::pin(async move { parse(sidecar.call(Some(space), method, params).await?) })
    }
}

impl Agent for Remote {
    fn send(&self, input: Input) -> Reply<Sent> {
        self.call("send", json!({ "input": input, "label": self.label }))
    }

    fn threads(&self) -> Reply<Vec<ThreadSummary>> {
        self.call("threads", Value::Null)
    }

    fn history(&self, thread: String, since: usize) -> Reply<Vec<Event>> {
        self.call("history", json!({ "thread": thread, "since": since }))
    }

    fn abort(&self, thread: String) -> Reply<bool> {
        self.call("abort", json!({ "thread": thread }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_names_map_back_to_spaces() {
        assert_eq!(space_id("space-7"), Some(7));
        assert_eq!(space_id("space-"), None);
        assert_eq!(space_id("other"), None);
    }

    #[test]
    fn the_restart_budget_forgets_old_deaths() {
        let mut deaths = Vec::new();
        let t0 = Instant::now();
        for n in 1..=BACKOFF.len() as u32 {
            assert_eq!(
                restart_attempt(&mut deaths, t0 + Duration::from_secs(n as u64)),
                n
            );
        }
        // One more inside the window: over budget.
        assert_eq!(
            restart_attempt(&mut deaths, t0 + Duration::from_secs(10)) as usize,
            BACKOFF.len() + 1
        );
        // Long after: a fresh start.
        assert_eq!(
            restart_attempt(&mut deaths, t0 + RESTART_WINDOW + Duration::from_secs(60)),
            1
        );
    }

    #[test]
    fn status_wire_shape() {
        let json = serde_json::to_value(Status::Running {
            backend: "stub".into(),
            restarted: true,
            has_key: false,
        })
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "state": "running", "backend": "stub", "restarted": true, "hasKey": false })
        );
        let json = serde_json::to_value(Status::Restarting {
            attempt: 0,
            asked: true,
        })
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "state": "restarting", "attempt": 0, "asked": true })
        );
        assert_eq!(
            serde_json::to_value(Status::Starting).unwrap(),
            serde_json::json!({ "state": "starting" })
        );
    }
}
