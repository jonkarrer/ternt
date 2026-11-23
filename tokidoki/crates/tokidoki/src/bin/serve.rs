//! `tokidoki-serve --data <dir>`: tokidoki as a sidecar process (`tokidoki::serve`).
//!
//! Each space's agent keeps its files in `<dir>/<space>`. The host says per space, with
//! `configure`, whose models it runs on and which tools it has; the keys are the process's,
//! from its environment: `ANTHROPIC_API_KEY`, `FIREWORKS_API_KEY`, `BASETEN_API_KEY`, whichever
//! the host has, `EXA_API_KEY` for web search, and `TYPESAFE_API_KEY` for Jev, which then
//! routes inputs to their threads ahead of the fast model. A space never configured runs on `TOKIDOKI_PROVIDER` (`anthropic` unless
//! set). `TOKIDOKI_MODEL` and `TOKIDOKI_ROUTER_MODEL` run other models than a provider's
//! usual. stdout is the protocol and nothing else; what would be logged goes to stderr.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use tokidoki::serve::{self, Open};
use tokidoki::tools::WebSearch;
use tokidoki::{Config, Jev, Setup, Spaces, Tokidoki, Tools, Vendor};
use tokio::io::BufReader;
use tokio::signal::unix::{SignalKind, signal};

#[tokio::main]
async fn main() {
    let served = run().await;
    if let Err(error) = &served {
        eprintln!("tokidoki: {error:#}");
    }
    // Not by returning: the runtime would wait for the thread still reading stdin, which the
    // host keeps open until this process is gone.
    std::process::exit(served.is_err().into());
}

/// The keys the host gave: one per provider it has one for, and the tools' own.
struct Keys {
    providers: Vec<(Vendor, String)>,
    exa: Option<String>,
    /// TypeSafe's, for Jev: every space's router, whichever provider answers there.
    jev: Option<String>,
}

impl Keys {
    /// Read once: the environment is the host's settings as they were when it started this.
    fn from_env() -> Self {
        let var = |name: &str| std::env::var(name).ok().filter(|key| !key.is_empty());
        Self {
            providers: Vendor::ALL
                .into_iter()
                .filter_map(|vendor| Some((vendor, var(vendor.key_var())?)))
                .collect(),
            exa: var(WebSearch::KEY_VAR),
            jev: var(Jev::KEY_VAR),
        }
    }

    fn get(&self, vendor: Vendor) -> Option<&str> {
        self.providers
            .iter()
            .find(|(v, _)| *v == vendor)
            .map(|(_, key)| key.as_str())
    }

    /// The tools this process knows, and whether it has what each needs to run: `ready`
    /// tells the host, which offers the user only those it can.
    fn tools(&self) -> serde_json::Value {
        serde_json::json!({ WebSearch::NAME: self.exa.is_some() })
    }

    /// Those of them `setup` asks for. One whose key is missing is left out, so the space
    /// still opens and `configure` answers with the tools it really has; a name this
    /// process doesn't know is the host's mistake.
    fn tools_for(&self, setup: &Setup) -> Result<Tools> {
        let mut tools = Tools::new();
        for name in &setup.tools {
            match name.as_str() {
                WebSearch::NAME => {
                    if let Some(key) = &self.exa {
                        tools.add(WebSearch::new(key));
                    }
                }
                _ => bail!("unknown tool {name}"),
            }
        }
        Ok(tools)
    }
}

async fn run() -> Result<()> {
    let data = match std::env::args().skip(1).collect::<Vec<_>>().as_slice() {
        [flag, dir] if flag == "--data" => PathBuf::from(dir),
        _ => bail!("usage: tokidoki-serve --data <dir>"),
    };
    std::fs::create_dir_all(&data).with_context(|| format!("creating {}", data.display()))?;
    // Two processes on the same files would each run the other's unfinished work.
    let lock = std::fs::File::create(data.join(".lock"))?;
    if lock.try_lock().is_err() {
        bail!("another tokidoki is serving {}", data.display());
    }

    let keys = Keys::from_env();
    let default = Setup {
        provider: Config::provider_from_env()?,
        tools: Vec::new(),
    };
    let providers: serde_json::Map<String, serde_json::Value> = Vendor::ALL
        .iter()
        .map(|v| (v.id().to_string(), keys.get(*v).is_some().into()))
        .collect();
    let about = serde_json::json!({
        "providers": providers,
        "default_provider": default.provider.id(),
        "tools": keys.tools(),
    });
    // Only the provider, tools and model settings are the host's to pass; the user, speakers
    // and system prompt are tokidoki's own (`chat` reads them, a host's environment shouldn't).
    // Without the provider's key an agent runs nothing, and what it has can still be read.
    let open = move |data: &Path, setup: &Setup| {
        let key = keys.get(setup.provider);
        let mut config = Config::new(data, setup.provider, key.unwrap_or_default().to_string())
            .with_models_from_env();
        config.jev = keys.jev.clone();
        match key {
            Some(_) => Tokidoki::open(config, keys.tools_for(setup)?),
            None => {
                Tokidoki::open_asleep(config, format!("{} is not set", setup.provider.key_var()))
            }
        }
    };

    let spaces = Arc::new(Spaces::new(&data).with_default(default));
    let serving = serve::run(
        spaces.clone(),
        Arc::new(open) as Open,
        about,
        BufReader::new(tokio::io::stdin()),
        tokio::io::stdout(),
    );
    let mut terminate = signal(SignalKind::terminate())?;
    tokio::select! {
        served = serving => served,
        // The program is going, and kills this process shortly: stop writing now.
        _ = terminate.recv() => {
            (tokio::time::timeout(serve::STOP_BUDGET, spaces.close_all()).await).unwrap_or(Ok(()))
        }
    }
}
