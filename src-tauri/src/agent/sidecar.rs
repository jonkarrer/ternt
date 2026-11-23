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
//! `externalBin`; `TERNT_AGENT_BIN` overrides the path): tokidoki-serve in a
//! release, the stub from `src-tauri/agent` in development
//! (`scripts/sidecar.sh`).

use std::{
    collections::HashMap,
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{ChildStdin, Command},
    sync::{Mutex as AsyncMutex, Notify, oneshot},
};

use super::{
    Outbox, folder_name, keys,
    protocol::*,
    setup::{Setup, SpaceStatus},
    wire::{Frame, Request},
};

/// How long a request may take (a model call is not a request: `send`
/// returns as soon as the input is placed).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// How long the process has to say it is ready after starting.
const READY_TIMEOUT: Duration = Duration::from_secs(15);
/// How long a process gets to leave after SIGTERM before SIGKILL. tokidoki
/// stops its agents in milliseconds; the contract promises 300.
const STOP_WITHIN: Duration = Duration::from_millis(300);
/// Pauses before each restart. One more death within `RESTART_WINDOW` than
/// there are entries here, and the process stays down.
const BACKOFF: [Duration; 4] = [
    Duration::ZERO,
    Duration::from_secs(1),
    Duration::from_secs(3),
    Duration::from_secs(10),
];
const RESTART_WINDOW: Duration = Duration::from_secs(120);

/// Why the process is down while the user has the agent switched off.
pub const OFF: &str = "The AI agent is switched off (Settings → Agent).";

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
        /// Counts the processes started: a change means a new one, whatever
        /// else looks the same. How each space's agent runs is asked per
        /// space (`configure`).
        generation: u64,
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

/// What Ternt wants of the running process, read by the loop when it exits.
#[derive(Clone, Copy, PartialEq)]
#[repr(u8)]
enum Intent {
    /// Nothing: an exit is a death.
    None = 0,
    /// Stop it and start another (Restart, a key or provider change).
    Restart = 1,
    /// Stop it for good (Ternt is quitting, or the agent was switched off).
    Stop = 2,
}

impl Intent {
    fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Restart,
            2 => Self::Stop,
            _ => Self::None,
        }
    }
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
    /// The running process, 0 when there is none.
    pid: AtomicU32,
    /// A `run` loop is going (so `restart` knows whether to start one).
    running: AtomicBool,
    intent: AtomicU8,
    /// Cuts a back-off pause short (Restart pressed during one).
    wake: Notify,
    generation: AtomicU64,
    /// What the running process was told each space runs as, and its
    /// answer; emptied with every new process, which knows nothing.
    configured: Mutex<HashMap<u32, (Setup, SpaceStatus)>>,
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

fn signal(pid: u32, sig: libc::c_int) {
    if pid != 0 {
        // Sending a signal to a pid we started and still hold is the one thing
        // `kill` is for; the process's death is seen by the loop that owns it.
        unsafe {
            libc::kill(pid as libc::pid_t, sig);
        }
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
            intent: AtomicU8::new(Intent::None as u8),
            wake: Notify::new(),
            generation: AtomicU64::new(0),
            configured: Mutex::default(),
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
            let outcome = self.spawn_once(restarted).await;
            // However it ended, there is no process now.
            self.pid.store(0, Ordering::Relaxed);
            *self.stdin.lock().await = None;
            let intent = Intent::from_u8(self.intent.swap(Intent::None as u8, Ordering::AcqRel));
            match (intent, outcome) {
                (Intent::Stop, _) => {
                    self.fail_pending("The agent was stopped.");
                    break;
                }
                (Intent::Restart, _) => {
                    self.fail_pending("The agent was restarted.");
                    self.set_status(Status::Restarting {
                        attempt: 0,
                        asked: true,
                    });
                    restarted = false;
                    continue;
                }
                (Intent::None, Err(reason)) => {
                    self.fail_pending(&reason);
                    self.set_status(Status::Down { reason });
                    break;
                }
                (Intent::None, Ok(())) => {}
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
            // The pause, unless Restart cuts it short.
            let _ = tokio::time::timeout(BACKOFF[attempt as usize - 1], self.wake.notified()).await;
            restarted = true;
        }
        self.running.store(false, Ordering::Release);
    }

    /// Run the process until it exits. `Err` when it couldn't be started or
    /// never said it was ready (restarting won't help).
    async fn spawn_once(&self, restarted: bool) -> Result<(), String> {
        let bin = binary();
        let _ = std::fs::create_dir_all(&self.data);
        // Only what is asked of the process started here counts for it.
        self.intent.store(Intent::None as u8, Ordering::Release);
        // The user's keys reach the agents this way and no other; which one a
        // space uses, the process is told per space (`configure`).
        let mut child = Command::new(&bin)
            .arg("--data")
            .arg(&self.data)
            .envs(keys::load())
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
        let ready = match tokio::time::timeout(READY_TIMEOUT, lines.next_line()).await {
            Ok(Ok(Some(line))) => match serde_json::from_str(&line) {
                Ok(Frame::Ready { ready }) => ready,
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
        // A new process knows no space's setup yet.
        self.configured.lock().unwrap().clear();
        self.set_status(Status::Running {
            backend: ready.backend,
            restarted,
            generation: self.generation.fetch_add(1, Ordering::Relaxed) + 1,
        });

        while let Ok(Some(line)) = lines.next_line().await {
            self.on_line(&line);
        }
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

    /// Every window hears (the panel and Settings both show it).
    fn set_status(&self, status: Status) {
        *self.status.lock().unwrap() = status.clone();
        let _ = self.app.emit("agent:status", status);
    }

    pub fn status(&self) -> Status {
        self.status.lock().unwrap().clone()
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
    async fn call(
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

    /// A request to one space's agent, which opens on first use, as its
    /// space's setup says.
    async fn ask<T: DeserializeOwned>(
        &self,
        space: u32,
        method: &str,
        params: Value,
    ) -> Result<T, String> {
        self.ensure(space).await?;
        let value = self.call(Some(folder_name(space)), method, params).await?;
        serde_json::from_value(value)
            .map_err(|err| format!("The agent answered with something unexpected: {err}"))
    }

    /// Tell the process how `space`'s agent runs before anything else is
    /// asked of it, once per process.
    async fn ensure(&self, space: u32) -> Result<(), String> {
        if self.configured.lock().unwrap().contains_key(&space) {
            return Ok(());
        }
        let setup = crate::browser::space_setup(&self.app, space).unwrap_or_default();
        self.configure(space, setup).await.map(|_| ())
    }

    /// Have `space`'s agent run as `setup` says (Settings → Agent), and say
    /// how it runs now. Nothing is sent when the process was already told
    /// this; a change reaches the agent at once, no restart. Every window
    /// hears the answer (`agent:space`), the panel among them.
    pub async fn configure(&self, space: u32, setup: Setup) -> Result<SpaceStatus, String> {
        if let Some((known, status)) = self.configured.lock().unwrap().get(&space)
            && *known == setup
        {
            return Ok(status.clone());
        }
        let params = json!({ "provider": setup.provider.as_str(), "tools": setup.tools });
        let value = self
            .call(Some(folder_name(space)), "configure", params)
            .await?;
        let status: SpaceStatus = serde_json::from_value(value)
            .map_err(|err| format!("The agent answered with something unexpected: {err}"))?;
        self.configured
            .lock()
            .unwrap()
            .insert(space, (setup, status.clone()));
        let _ = self
            .app
            .emit("agent:space", json!({ "space": space, "status": status }));
        Ok(status)
    }

    /// Returns as soon as the input is placed; the answer arrives as events.
    /// `label` is the space's name, which the agent tells its new threads.
    pub async fn send(&self, space: u32, label: &str, input: Input) -> Result<Sent, String> {
        self.ask(space, "send", json!({ "input": input, "label": label }))
            .await
    }

    pub async fn threads(&self, space: u32) -> Result<Vec<ThreadSummary>, String> {
        self.ask(space, "threads", Value::Null).await
    }

    /// A thread's checkpoints from `since` on.
    pub async fn history(
        &self,
        space: u32,
        thread: String,
        since: usize,
    ) -> Result<Vec<Event>, String> {
        self.ask(
            space,
            "history",
            json!({ "thread": thread, "since": since }),
        )
        .await
    }

    /// Stop the turn under way; `false` if there was none.
    pub async fn abort(&self, space: u32, thread: String) -> Result<bool, String> {
        self.ask(space, "abort", json!({ "thread": thread })).await
    }

    /// What the agent remembers, newest first (Settings → Agent → Memory).
    pub async fn memories(&self, space: u32) -> Result<Vec<Memory>, String> {
        self.ask(space, "memories", Value::Null).await
    }

    /// The user's correction of a memory: the one that replaces it.
    pub async fn revise(&self, space: u32, id: i64, text: String) -> Result<Memory, String> {
        self.ask(space, "revise", json!({ "id": id, "text": text }))
            .await
    }

    /// Remove one memory, with its history, for good.
    pub async fn forget(&self, space: u32, id: i64) -> Result<(), String> {
        self.ask::<Value>(space, "forget", json!({ "id": id }))
            .await
            .map(|_| ())
    }

    /// Remove everything the agent has learned; its conversations stay.
    pub async fn wipe(&self, space: u32) -> Result<(), String> {
        self.ask::<Value>(space, "wipe", Value::Null)
            .await
            .map(|_| ())
    }

    /// The agent stopped and its files removed, for good. Not through `ask`:
    /// the space is gone from the model, so there is no setup to send first.
    pub async fn delete(&self, space: u32) -> Result<(), String> {
        self.configured.lock().unwrap().remove(&space);
        self.call(Some(folder_name(space)), "delete", Value::Null)
            .await
            .map(|_| ())
    }

    /// Ask the running process to leave, with `intent` for the loop to read
    /// when it has. Returns its pid, or `None` when nothing was running.
    fn stop(&self, intent: Intent) -> Option<u32> {
        let pid = self.pid.load(Ordering::Relaxed);
        if pid == 0 {
            return None;
        }
        self.intent.store(intent as u8, Ordering::Release);
        signal(pid, libc::SIGTERM);
        Some(pid)
    }

    /// Start over, on purpose (the Restart button, a key or provider change):
    /// stop the process and let the loop start another, with the keys read
    /// again; cut a back-off pause short; or start a loop when it was left
    /// down.
    pub fn restart(self: &Arc<Self>) {
        self.deaths.lock().unwrap().clear();
        match self.stop(Intent::Restart) {
            Some(pid) => {
                let me = self.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(STOP_WITHIN).await;
                    if me.pid.load(Ordering::Relaxed) == pid {
                        signal(pid, libc::SIGKILL);
                    }
                });
            }
            None if self.running.load(Ordering::Acquire) => self.wake.notify_one(),
            None => {
                self.set_status(Status::Starting);
                self.start();
            }
        }
    }

    /// The user switched the agent off: stop the process, leave it stopped and say why.
    /// `restart` starts it again. Waits like `shutdown`, so not on the main thread.
    pub fn switch_off(&self) {
        self.shutdown();
        self.set_status(Status::Down { reason: OFF.into() });
    }

    /// Stop the process before Ternt exits (or while the agent is off), and keep the loop from starting
    /// another. Runs on the main thread at exit, so it waits by polling: the
    /// loop clears `pid` the moment the process is gone, usually within a few
    /// milliseconds.
    pub fn shutdown(&self) {
        let Some(pid) = self.stop(Intent::Stop) else {
            return;
        };
        let deadline = Instant::now() + STOP_WITHIN;
        while self.pid.load(Ordering::Relaxed) == pid && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        if self.pid.load(Ordering::Relaxed) == pid {
            signal(pid, libc::SIGKILL);
        }
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
    fn intents_survive_the_trip_through_a_byte() {
        for intent in [Intent::None, Intent::Restart, Intent::Stop] {
            assert!(Intent::from_u8(intent as u8) == intent);
        }
    }

    #[test]
    fn status_wire_shape() {
        let json = serde_json::to_value(Status::Running {
            backend: "stub".into(),
            restarted: true,
            generation: 2,
        })
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "state": "running", "backend": "stub", "restarted": true, "generation": 2 })
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
