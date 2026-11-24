# Architecture

How Ternt is put together. For what it does, see [GUIDE.md](GUIDE.md).

One window, several native webviews (see `src-tauri/src/browser.rs`):

| Webview | Page | Role |
|---|---|---|
| `ui` | `index.html` → `src/App.svelte` | Sidebar, transparent over native vibrancy |
| `tab-<id>` | the web | One per *loaded* web tab, no IPC access |
| `term` | `term.html` → `src/term/` | All terminal tabs and their panes (xterm.js), created on first use |
| `peek` | `index.html` again | The collapsed sidebar shown over the page on hover, pre-created hidden |
| `palette` | `palette.html` → `src/palette/` | Command bar overlay, pre-created hidden |
| `agent` | `agent.html` → `src/agent/` | Tokidoki panel (⌃A), created on first open then kept hidden (`src-tauri/src/agent/`; the agents themselves run in the `ternt-agent` sidecar process) |
| `settings` | `settings.html` → `src/settings/` | Settings, its own window (`src-tauri/src/settings.rs`) |
| `little-N-bar` / `little-N-page` | `little.html` → `src/little/` / the web | Little Ternt windows (`src-tauri/src/little.rs`) |

- **Rust owns state and layout.** The model (spaces, zones, folders, ordering) is plain data in
  `src-tauri/src/model.rs`, unit-tested with `cargo test`; webviews mirror it via
  `browser:state` events. Content views use native AppKit autoresizing, so window resizes
  never wait on IPC.
- **Tabs load lazily.** A web tab's webview is created when first shown; restoring a large
  session costs about as much as one tab.
- **Navigation state** (URL, title, loading, back/forward) comes from KVO on each
  `WKWebView` (`src-tauri/src/native.rs`).
- **Terminals** stream raw PTY bytes over a Tauri channel with ack-based flow control
  (`src-tauri/src/pty.rs`), one PTY per pane. A tab's panes are a binary tree in the model
  (`src-tauri/src/panes.rs`); the terminal view lays it out.
- **Preferences** (`prefs.json`, `src-tauri/src/prefs.rs`) sit next to the session.
- **Links from other apps**: `src-tauri/Info.plist` claims http/https, so macOS can make Ternt the
  default browser; links arrive as `RunEvent::Opened` (`src-tauri/src/little.rs`).
- **Sessions** are saved to the app data dir (`session.json`), debounced and on exit. Debug
  builds use a `dev/` subfolder there, so `just dev` never leaves tabs in the installed app.
- **Welcome page** (`src-tauri/welcome.html`, served on `ternt://welcome` by
  `src-tauri/src/welcome.rs`): a plain web tab on a custom scheme, so it needs no IPC. Opened
  when there is no session file at all (first launch) and from the Help menu.
- **App icon**: `src-tauri/icons/icon.svg` is the source; `icons/README.md` has the
  regeneration steps and `icons/ideas/` the other backgrounds that were explored.
- **History** (`src-tauri/src/history.rs`) records each page that finishes loading, per profile, in
  `history.json`; the command bar searches it in Rust as you type (match quality × frecency).

## The agent (tokidoki)

Every space gets its own AI agent: tokidoki (`tokidoki/`, Ternt's copy, forked 2026-11-23 from
[jonkarrer/tokidoki](https://github.com/jonkarrer/tokidoki)), an agent with durable execution and
long-term memory. Press ⌃A, or type `tok` ⇥ and your question in the command bar (⌘T):
a panel opens over the page with that space's conversations. Agents share nothing, so what the dev
space's agent learns never reaches the marketing space's.

The agents run in a **sidecar process**, not in Ternt. Ternt's release build aborts on panic, so a
panic inside an agent (or in SQLite, or running out of memory) ends whatever process it runs in;
in its own process that is the agent, never the browser. Ternt starts the process, restarts it
with a short back-off when it dies, and the panel says so (a Restart button appears if it keeps
dying). The protocol, JSON lines over stdin and stdout, is in [AGENT-SIDECAR.md](AGENT-SIDECAR.md).
Two programs speak it: tokidoki's `tokidoki-serve`, the real agent, and the stub in
`src-tauri/agent/`, a streamed canned reply with the same shape of events a real turn has (`/tool`
adds a pretend tool call, `/error` a failed turn, `/crash` kills the process so you can watch it
come back). `just sidecar tokidoki` (or `TERNT_SIDECAR=tokidoki`) builds the real one from
`tokidoki/` (`TOKIDOKI_SRC` to point at another tree); the stub is the default for development
and what the e2e suites run against. **Releases ship tokidoki** (`just upload` builds it from the
tree, refusing uncommitted changes there); `TERNT_SIDECAR=stub just upload` would ship the stub. `TERNT_SIDECAR=tokidoki e2e/run.sh tokidoki` runs the panel
against the real agent (one model call if `ANTHROPIC_API_KEY` is set).

The agent can be **switched off** (the switch at the top of Settings → Agent, or in the setup
window): ⌃A does nothing, the command bar stops offering Tokidoki and the sidecar isn't run (`Prefs::agent_enabled`,
`agent::set_enabled`).

The agent is **bring your own key**: Settings → Agent → Keys takes your Fireworks AI and Anthropic
keys, kept in the macOS Keychain (`src-tauri/src/agent/keys.rs`); Ternt hands them to the sidecar
through its environment and nothing else ever sees them. A key already in Ternt's environment
wins, which is how development runs with the team's keys: `doppler run -- just dev tokidoki`.
**Each space picks its own provider** (Fireworks AI by default) on the same page; the change
reaches the running agent at once and its conversations go on. Each space also switches its
agent's **tools** on or off there: web search for now, through Exa, with your Exa key under Keys. An optional
TypeSafe key there lets Jev make the agent's small decisions, fast and for very little: which conversation a message
belongs to, which memories are worth recalling for it, and whether it says something worth remembering. The page also has a **Memory**
section: what the space's agent has learned, fact by fact, to correct or forget, and a Wipe that
forgets all of it (the conversations stay).

- `src-tauri/src/agent/protocol.rs` mirrors tokidoki's `interface` types (`src/agent/protocol.ts`
  is its generated TypeScript, copied from `tokidoki/` by `just sync-protocol`). The panel is written against the
  real wire format.
- `src-tauri/src/agent/mod.rs` owns the `Agent` boundary (`send`, `threads`, `history`, `abort`),
  one lazily opened agent per space (folder `agents/space-<id>`), the panel webview, and the
  **outbox**: every agent's events go through one bounded channel to one forwarder, which
  coalesces streamed text into one IPC message per ~30 ms and sends nothing while the panel is
  hidden. The page reloads a thread's history when it opens (checkpoints are numbered without
  gaps), so dropped events cost nothing; a full queue sends `lagged` and the page catches up.
- `src-tauri/src/agent/sidecar.rs` is the supervisor and Ternt's end of the protocol;
  `src-tauri/src/agent/wire.rs` the line format.

The sidecar is its own crate so it can be built before Ternt (Tauri's build script checks that
the binary named in `bundle.externalBin` exists): `just sidecar` (or `scripts/sidecar.sh`) puts it
in `src-tauri/binaries/`, which `just dev`, `just check`, the unit tests, the e2e runner and
`scripts/publish.sh --release` all do first. `TERNT_AGENT_BIN` points Ternt at another binary.
Keys for the real agent belong in the Keychain or Doppler, never the repo. The e2e suite `agent`
drives the panel against the stub, crash included; `src/agent/thread.test.ts` covers the view
model and `cargo test -p ternt-agent` the stub.
