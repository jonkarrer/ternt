//! Reads what the agent did: `just inspect <command>`. See `USAGE`.

use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};
use telemetry::inspect::{Inspector, duration, now};

const USAGE: &str = "usage: inspect [--db PATH] [--memory PATH] [--json] <command>

  recent [N]                   the latest N turns (default 10), with their warnings and errors
  problems [--since 2h]        warnings and errors
  search <words...>            where conversations, the journal and memory mention these words
  thread <id> [--turn N] [--all]  a thread's checkpoints and journal entries, in time order
                               (--all adds routine task state changes)
  task <id>                    a task as stored, what it owns, its journal entries
  entry <id>                   one journal entry in full (a model call's exact request and reply)
  context <thread> <n>         the context the model saw at checkpoint n
  memory <words...>            memory items (every version), cards, episodes, resume cards
  replay <entry> [--body F]    re-send a recorded model call (or the request body in file F)
  prune <age>                  delete journal entries older than age (e.g. 30d)

--db defaults to $TOKIDOKI_DB or data/tokidoki.db; its journal sits beside it.
--memory defaults to $TOKIDOKI_MEMORY or data/memory.db. Times are UTC.";

#[tokio::main]
async fn main() {
    if let Err(error) = run(std::env::args().skip(1).collect()).await {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}

async fn run(mut args: Vec<String>) -> Result<()> {
    let db = option(&mut args, "--db")?
        .or_else(|| std::env::var("TOKIDOKI_DB").ok())
        .unwrap_or_else(|| "data/tokidoki.db".into());
    let memory = option(&mut args, "--memory")?
        .or_else(|| std::env::var("TOKIDOKI_MEMORY").ok())
        .unwrap_or_else(|| "data/memory.db".into());
    let json = flag(&mut args, "--json");
    let since = option(&mut args, "--since")?;
    let turn = option(&mut args, "--turn")?;
    let body = option(&mut args, "--body")?;
    let all = flag(&mut args, "--all");
    let Some(command) = args.first().cloned() else {
        bail!("{USAGE}");
    };
    let rest = &args[1..];
    let words = rest.join(" ");
    let arg = |i: usize| {
        rest.get(i)
            .with_context(|| format!("missing argument\n\n{USAGE}"))
    };

    let (db, memory) = (PathBuf::from(db), PathBuf::from(memory));
    if !db.exists() {
        bail!("no state database at {} (pass --db)", db.display());
    }
    let inspector = Inspector::open(&db, Some(&memory))?;
    let output = match command.as_str() {
        "recent" => inspector.recent(rest.first().map_or(Ok(10), |n| n.parse())?)?,
        "problems" => {
            let since = since.map(|s| duration(&s)).transpose()?;
            inspector.problems(since.map(|age| now().saturating_sub(age)))?
        }
        "search" => inspector.search(&words, 15)?,
        "thread" => inspector.thread(arg(0)?, turn.map(|t| t.parse()).transpose()?, all)?,
        "task" => inspector.task(arg(0)?.parse()?)?,
        "entry" => inspector.entry(arg(0)?.parse()?)?,
        "context" => inspector.context(arg(0)?, arg(1)?.parse()?)?,
        "memory" => inspector.memory(&words, 30)?,
        "replay" => {
            let id: i64 = arg(0)?.parse()?;
            let entry = inspector
                .journal()
                .entry(id)?
                .with_context(|| format!("no entry {id}"))?;
            let body = match body {
                Some(path) => Some(serde_json::from_str(&std::fs::read_to_string(&path)?)?),
                None => None,
            };
            telemetry::replay::resend(&entry, body).await?
        }
        "prune" => {
            let pruned = inspector
                .journal()
                .prune(now().saturating_sub(duration(arg(0)?)?))?;
            println!("pruned {pruned} entries");
            return Ok(());
        }
        _ => bail!("unknown command {command}\n\n{USAGE}"),
    };
    println!("{}", output.render(json));
    Ok(())
}

/// Removes `--name value` from `args`, returning the value.
fn option(args: &mut Vec<String>, name: &str) -> Result<Option<String>> {
    let Some(i) = args.iter().position(|a| a == name) else {
        return Ok(None);
    };
    if i + 1 >= args.len() {
        bail!("{name} needs a value");
    }
    let value = args.remove(i + 1);
    args.remove(i);
    Ok(Some(value))
}

fn flag(args: &mut Vec<String>, name: &str) -> bool {
    let found = args.iter().position(|a| a == name);
    if let Some(i) = found {
        args.remove(i);
    }
    found.is_some()
}
