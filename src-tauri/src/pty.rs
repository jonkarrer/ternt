//! Terminal sessions: one PTY per terminal tab, streamed to xterm.js.
//!
//! Output path: a reader thread pulls bytes off the PTY into a buffer; a
//! flusher thread sends whatever has accumulated as one raw (non-JSON) channel
//! message at most every few ms, so heavy output becomes a few large writes
//! instead of thousands of small ones.
//!
//! Flow control: xterm.js acks bytes once it has parsed them. When too much is
//! unacked the flusher pauses, the buffer fills, and the reader stops reading,
//! so the PTY's own backpressure throttles the program (`cat huge.log` slows
//! down instead of freezing the UI).

use std::{
    collections::HashMap,
    io::{Read, Write},
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};

use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use tauri::{AppHandle, Manager, ipc::Channel, ipc::InvokeResponseBody};

/// Stop sending once this many bytes are in flight to xterm.js.
const MAX_UNACKED: usize = 512 * 1024;
/// Stop reading the PTY once this much output is buffered.
const MAX_BUFFERED: usize = 4 * 1024 * 1024;
/// Minimum gap between sends, so bursts coalesce.
const FLUSH_INTERVAL: Duration = Duration::from_millis(4);

#[derive(Default)]
struct Pipe {
    data: Vec<u8>,
    unacked: usize,
    closed: bool,
    channel: Option<Channel<InvokeResponseBody>>,
}

struct Output {
    pipe: Mutex<Pipe>,
    changed: Condvar,
}

impl Output {
    fn lock(&self) -> std::sync::MutexGuard<'_, Pipe> {
        self.pipe.lock().unwrap()
    }
}

struct Session {
    master: Box<dyn MasterPty + Send>,
    // Separate lock so a blocked write (program not reading input) never
    // holds up the session map.
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    output: Arc<Output>,
}

#[derive(Default)]
pub struct Terminals(Mutex<HashMap<u32, Session>>);

/// Input typed into a terminal as soon as its shell starts (`(tab, text)`),
/// for terminals opened to run a command.
static STARTUP_INPUT: Mutex<Vec<(u32, String)>> = Mutex::new(Vec::new());

pub fn queue_startup_input(id: u32, text: String) {
    STARTUP_INPUT.lock().unwrap().push((id, text));
}

fn size(cols: u16, rows: u16) -> PtySize {
    PtySize {
        rows: rows.max(1),
        cols: cols.max(1),
        pixel_width: 0,
        pixel_height: 0,
    }
}

fn spawn(app: &AppHandle, id: u32, cols: u16, rows: u16) -> Result<Session, String> {
    let pair = native_pty_system().openpty(size(cols, rows)).map_err(|e| e.to_string())?;

    // The user's login shell, like Terminal.app.
    let mut cmd = CommandBuilder::new_default_prog();
    if let Some(home) = std::env::var_os("HOME") {
        cmd.cwd(home);
    }
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    cmd.env("TERM_PROGRAM", "Ternt");
    if std::env::var_os("LANG").is_none() {
        cmd.env("LANG", "en_US.UTF-8");
    }

    let mut child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
    // Drop our copy of the slave so the reader sees EOF when the shell exits.
    drop(pair.slave);
    let killer = child.clone_killer();
    std::thread::spawn(move || {
        let _ = child.wait();
    });

    let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let writer = Arc::new(Mutex::new(pair.master.take_writer().map_err(|e| e.to_string())?));
    // The tty buffers this until the shell reads it, so no need to wait.
    let startup = {
        let mut queue = STARTUP_INPUT.lock().unwrap();
        queue.iter().position(|(tab, _)| *tab == id).map(|i| queue.remove(i).1)
    };
    if let Some(text) = startup {
        let _ = writer.lock().unwrap().write_all(text.as_bytes());
    }
    let output = Arc::new(Output {
        pipe: Mutex::new(Pipe::default()),
        changed: Condvar::new(),
    });

    let out = output.clone();
    std::thread::spawn(move || {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            // EOF or EIO both mean the shell is gone.
            let n = match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            let mut pipe = out.lock();
            while pipe.data.len() > MAX_BUFFERED && !pipe.closed {
                pipe = out.changed.wait(pipe).unwrap();
            }
            pipe.data.extend_from_slice(&buf[..n]);
            out.changed.notify_all();
        }
        out.lock().closed = true;
        out.changed.notify_all();
    });

    let out = output.clone();
    let app = app.clone();
    std::thread::spawn(move || {
        loop {
            let (chunk, channel) = {
                let mut pipe = out.lock();
                loop {
                    let ready = !pipe.data.is_empty() && pipe.unacked < MAX_UNACKED && pipe.channel.is_some();
                    if ready || (pipe.closed && pipe.data.is_empty()) {
                        break;
                    }
                    pipe = out.changed.wait(pipe).unwrap();
                }
                if pipe.data.is_empty() {
                    break; // closed and drained
                }
                let chunk = std::mem::take(&mut pipe.data);
                pipe.unacked += chunk.len();
                out.changed.notify_all();
                (chunk, pipe.channel.clone().unwrap())
            };
            let _ = channel.send(InvokeResponseBody::Raw(chunk));
            std::thread::sleep(FLUSH_INTERVAL);
        }
        on_exit(&app, id);
    });

    Ok(Session {
        master: pair.master,
        writer,
        killer,
        output,
    })
}

/// The shell exited on its own: close its tab, like a terminal app would.
fn on_exit(app: &AppHandle, id: u32) {
    if app.state::<Terminals>().0.lock().unwrap().remove(&id).is_some() {
        let _ = crate::browser::close_tab_by_id(app, id);
    }
}

/// Kill a tab's shell (its tab is being closed).
pub fn kill(app: &AppHandle, id: u32) {
    if let Some(mut session) = app.state::<Terminals>().0.lock().unwrap().remove(&id) {
        let _ = session.killer.kill();
        let mut pipe = session.output.lock();
        pipe.closed = true;
        pipe.channel = None;
        session.output.changed.notify_all();
    }
}

/// Start the shell for a terminal tab, or reattach to it if it's running
/// (e.g. after the terminal webview reloads).
#[tauri::command]
pub async fn pty_spawn(
    app: AppHandle,
    id: u32,
    cols: u16,
    rows: u16,
    channel: Channel<InvokeResponseBody>,
) -> Result<(), String> {
    if !crate::browser::is_terminal_tab(&app, id) {
        return Err(format!("tab {id} is not a terminal"));
    }
    let terminals = app.state::<Terminals>();
    let mut sessions = terminals.0.lock().unwrap();
    let session = match sessions.entry(id) {
        std::collections::hash_map::Entry::Occupied(entry) => {
            let session = entry.into_mut();
            let _ = session.master.resize(size(cols, rows));
            session
        }
        std::collections::hash_map::Entry::Vacant(entry) => {
            entry.insert(spawn(&app, id, cols, rows)?)
        }
    };
    let mut pipe = session.output.lock();
    pipe.channel = Some(channel);
    pipe.unacked = 0;
    session.output.changed.notify_all();
    Ok(())
}

#[tauri::command]
pub fn pty_write(app: AppHandle, id: u32, data: String) -> Result<(), String> {
    let writer = {
        let terminals = app.state::<Terminals>();
        let sessions = terminals.0.lock().unwrap();
        sessions.get(&id).ok_or("no such terminal")?.writer.clone()
    };
    let mut writer = writer.lock().unwrap();
    writer.write_all(data.as_bytes()).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn pty_resize(app: AppHandle, id: u32, cols: u16, rows: u16) -> Result<(), String> {
    let terminals = app.state::<Terminals>();
    let sessions = terminals.0.lock().unwrap();
    let session = sessions.get(&id).ok_or("no such terminal")?;
    session.master.resize(size(cols, rows)).map_err(|e| e.to_string())
}

/// xterm.js has parsed `bytes` more bytes.
#[tauri::command]
pub fn pty_ack(app: AppHandle, id: u32, bytes: usize) {
    let terminals = app.state::<Terminals>();
    let sessions = terminals.0.lock().unwrap();
    if let Some(session) = sessions.get(&id) {
        let mut pipe = session.output.lock();
        pipe.unacked = pipe.unacked.saturating_sub(bytes);
        session.output.changed.notify_all();
    }
}
