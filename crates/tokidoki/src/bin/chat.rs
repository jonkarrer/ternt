//! A testing ground: talk to a fresh tokidoki in the terminal, `just chat`.
//!
//! Every run starts empty: its databases (state, memory, journal) live in a temporary directory
//! that is deleted when the process ends, by `/quit`, end of input, Ctrl-C at the prompt, a
//! hangup or termination signal, an error or a panic. `--keep` keeps it and prints where, for
//! `just inspect --db <dir>/tokidoki.db --memory <dir>/memory.db`. Ctrl-C during an answer
//! stops the answer.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::Result;
use tokidoki::interface::{Change, Content, Event, Feedback, Input, Role, Sent, Speaker, Status};
use tokidoki::{Config, Events, Tokidoki};
use tokio::io::{AsyncBufReadExt, BufReader};

const HELP: &str = "/threads          list threads
/thread <id>      send to this thread from now on
/auto             let the router choose the thread again (the default)
/history [id]     show a thread's messages (the last one used by default)
/bad <what>       record that the last answer was wrong, for `just inspect`
/quit             leave (everything from this run is deleted, unless --keep)
Anything else is sent. Ctrl-C stops the current answer, or leaves at the prompt.";

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let keep = std::env::args().skip(1).any(|a| a == "--keep");
    // Declared first, so it is dropped (and deleted) last, after tokidoki has closed.
    let scratch = Scratch::new(keep)?;
    let mut config = Config::from_env()?;
    config.data = scratch.path().to_path_buf();
    let tokidoki = Tokidoki::open(config, agent::Tools::new())?;
    println!(
        "tokidoki: a fresh one, in {}; /help for commands",
        scratch.path().display()
    );
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut pinned: Option<String> = None;
    let mut last: Option<String> = None;
    loop {
        print!("{}> ", pinned.as_deref().unwrap_or(""));
        std::io::stdout().flush()?;
        let line = tokio::select! {
            line = lines.next_line() => line?,
            _ = tokio::signal::ctrl_c() => None,
            _ = stop_signal() => None,
        };
        let Some(line) = line else {
            println!();
            break;
        };
        let line = line.trim();
        let (command, rest) = line.split_once(' ').unwrap_or((line, ""));
        let rest = rest.trim();
        match command {
            "" => {}
            "/quit" | "/exit" => break,
            "/help" => println!("{HELP}"),
            "/threads" => {
                for thread in tokidoki.threads()? {
                    println!(
                        "  {:<12} {:>3} turns  {}",
                        thread.id, thread.turns, thread.title
                    );
                }
            }
            "/thread" if !rest.is_empty() => {
                pinned = Some(rest.to_string());
                last = pinned.clone();
            }
            "/auto" => pinned = None,
            "/history" => match (Some(rest).filter(|r| !r.is_empty())).or(last.as_deref()) {
                Some(thread) => show_history(&tokidoki, thread).await?,
                None => println!("no thread yet"),
            },
            "/bad" => match &last {
                Some(thread) => {
                    tokidoki.feedback(Feedback {
                        from: Speaker::owner(),
                        thread: thread.clone(),
                        turn: None,
                        text: rest.to_string(),
                    })?;
                    println!("  noted on {thread}");
                }
                None => println!("no answer yet"),
            },
            _ if command.starts_with('/') => println!("unknown command; /help lists them"),
            _ => {
                // Listen before sending, so nothing about this input is missed.
                let mut events = tokidoki.events();
                let mut input = Input::text(Speaker::owner(), line);
                input.thread = pinned.clone();
                let sent = match tokidoki.send(input).await {
                    Ok(sent) => sent,
                    Err(error) => {
                        println!("  ✗ {error:#}");
                        continue;
                    }
                };
                if let Some(route) = &sent.route {
                    let new = if route.new { "new, " } else { "" };
                    println!("  [{} · {new}{}: {}]", sent.thread, route.by, route.reason);
                }
                last = Some(sent.thread.clone());
                if follow(&tokidoki, &mut events, &sent).await? == Followed::Stopped {
                    break;
                }
            }
        }
    }
    tokidoki.close()
}

#[derive(Debug, PartialEq)]
enum Followed {
    Settled,
    /// The process was told to end.
    Stopped,
}

/// Shows what happens on the thread until the input is settled.
async fn follow(tokidoki: &Tokidoki, events: &mut Events, sent: &Sent) -> Result<Followed> {
    let mut status = String::new();
    let mut stopping = false;
    loop {
        let event = tokio::select! {
            event = events.recv() => event,
            _ = stop_signal() => return Ok(Followed::Stopped),
            _ = tokio::signal::ctrl_c(), if !stopping => {
                stopping = true;
                if tokidoki.abort(&sent.thread)? {
                    println!("  · stopping");
                }
                continue;
            }
        };
        let event = match event {
            Some(Event::Lagged) | None => {
                // Fell behind: ask directly.
                match tokidoki.submission(&sent.submission)? {
                    Some(s) if s.is_settled() => Event::Submission { submission: s },
                    _ => continue,
                }
            }
            Some(event) => event,
        };
        match event {
            Event::Activity {
                thread,
                kind,
                phase,
                state,
                ..
            } if thread == sent.thread && state == "running" => {
                let now = match (kind.as_str(), phase.as_str()) {
                    ("agent.generation", "consider") => "remembering",
                    ("agent.generation", "request") => "thinking",
                    _ => continue,
                };
                if status != now {
                    println!("  · {now}");
                    status = now.to_string();
                }
            }
            Event::Checkpoint { thread, change, .. } if thread == sent.thread => match change {
                Change::Message { content, .. } => {
                    for part in content {
                        match part {
                            Content::ToolCall { name, input, .. } => {
                                println!("  → {name} {}", clip(&input.to_string(), 100));
                            }
                            Content::ToolResult {
                                content, is_error, ..
                            } => {
                                let mark = if is_error { "✗" } else { "←" };
                                println!("  {mark} {}", clip(&content, 100));
                            }
                            _ => {}
                        }
                    }
                }
                Change::Recalled { memories } if !memories.is_empty() => {
                    for memory in memories {
                        println!("  ◦ {}", clip(&memory, 100));
                    }
                }
                _ => {}
            },
            Event::Submission { submission }
                if submission.id == sent.submission && submission.is_settled() =>
            {
                match submission.status {
                    Status::Done => println!("\n{}\n", submission.answer.unwrap_or_default()),
                    _ => println!("  (no answer: {})", submission.reason.unwrap_or_default()),
                }
                return Ok(Followed::Settled);
            }
            _ => {}
        }
    }
}

async fn show_history(tokidoki: &Tokidoki, thread: &str) -> Result<()> {
    for event in tokidoki.history(thread, 0)? {
        let Event::Checkpoint {
            change: Change::Message { role, content },
            ..
        } = event
        else {
            continue;
        };
        let text: Vec<String> = content
            .into_iter()
            .filter_map(|c| match c {
                Content::Text { text } => Some(text),
                Content::ToolCall { name, .. } => Some(format!("[{name}]")),
                _ => None,
            })
            .collect();
        if text.is_empty() {
            continue;
        }
        let who = match role {
            Role::User => "you",
            Role::Assistant => "tokidoki",
        };
        println!("{who}: {}", text.join(" "));
    }
    Ok(())
}

fn clip(text: &str, max: usize) -> String {
    let flat = text.replace('\n', " ");
    match flat.chars().count() > max {
        true => format!("{}…", flat.chars().take(max).collect::<String>()),
        false => flat,
    }
}

/// Resolves when the process is told to end: hangup (the terminal closed) or termination.
async fn stop_signal() {
    use tokio::signal::unix::{SignalKind, signal};
    let (Ok(mut hangup), Ok(mut terminate)) = (
        signal(SignalKind::hangup()),
        signal(SignalKind::terminate()),
    ) else {
        return std::future::pending().await;
    };
    tokio::select! {
        _ = hangup.recv() => {}
        _ = terminate.recv() => {}
    }
}

/// This run's data directory: deleted on drop, unless kept.
struct Scratch {
    path: PathBuf,
    keep: bool,
}

impl Scratch {
    fn new(keep: bool) -> Result<Self> {
        let path = std::env::temp_dir().join(format!("tokidoki-chat-{}", std::process::id()));
        // A directory with our PID is left from a run that was killed outright: not ours.
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path)?;
        Ok(Self { path, keep })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if self.keep {
            eprintln!("kept {}", self.path.display());
        } else if let Err(error) = std::fs::remove_dir_all(&self.path) {
            eprintln!("couldn't delete {}: {error}", self.path.display());
        }
    }
}
