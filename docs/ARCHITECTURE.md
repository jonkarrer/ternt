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
- **What a removed feature left behind** (`src-tauri/src/retired.rs`): Ternt had an AI agent up
  to v0.23. Each launch deletes its API keys from the Keychain and its `agents/` folder, since
  nothing in the app can show or manage them any more.
