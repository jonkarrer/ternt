# moove

An Arc-style browser and terminal. Tauri v2 (Rust) + Svelte 5 (Vite, TypeScript), macOS-first.

## Development

```sh
bun install
bun tauri dev              # run the app
bun tauri build            # release bundle (judge performance here, not in dev)
bun run check              # type-check Svelte/TS
e2e/run.sh                 # end-to-end tests (macOS); e2e/run.sh <name> for one
(cd src-tauri && cargo test)  # model unit tests
```

`moove <url>...` opens URLs as tabs (`bun tauri dev -- -- https://example.com`).

## Shortcuts

| Shortcut | Action |
|---|---|
| ⌘T / ⌘L | Command bar: new tab / edit current URL |
| ⌥⌘T | New terminal tab |
| ⌘W / ⇧⌘T | Close tab (unloads pinned tabs) / reopen closed tab |
| ⌃Tab / ⌃⇧Tab, ⌘1–⌘9 | Switch tabs |
| ⌘[ / ⌘] / ⌘R | Back / forward / reload |
| ⌘S | Toggle sidebar |
| ⌃1–⌃9, ⌘⌥← / ⌘⌥→ | Switch spaces (or swipe two fingers across the sidebar) |

## Sidebar

Arc-style, top to bottom:

- **Favorites**: icon tiles shared by every space.
- **Pinned**: per space, optionally grouped in folders.
- **Today**: per space, newest first.

Drag tabs between sections, into folders, or to reorder. Closing a favorite or pinned tab
unloads it (frees memory, keeps it in the sidebar). Right-click a tab, folder or space dot
for more. Double-click a space or folder name to rename it.

**Spaces** share cookies and logins by default. Right-click a space dot → *Separate Profile*
to give it its own cookie jar (macOS 14+); turning it off deletes that profile's data.

## Architecture

One window, several native webviews (see `src-tauri/src/browser.rs`):

| Webview | Page | Role |
|---|---|---|
| `ui` | `index.html` → `src/App.svelte` | Sidebar, transparent over native vibrancy |
| `tab-<id>` | the web | One per *loaded* web tab, no IPC access |
| `term` | `term.html` → `src/term/` | All terminal tabs (xterm.js), created on first use |
| `palette` | `palette.html` → `src/palette/` | Command bar overlay, pre-created hidden |

- **Rust owns state and layout.** The model (spaces, zones, folders, ordering) is plain data in
  `src-tauri/src/model.rs`, unit-tested with `cargo test`; webviews mirror it via
  `browser:state` events. Content views use native AppKit autoresizing, so window resizes
  never wait on IPC.
- **Tabs load lazily.** A web tab's webview is created when first shown; restoring a large
  session costs about as much as one tab.
- **Navigation state** (URL, title, loading, back/forward) comes from KVO on each
  `WKWebView` (`src-tauri/src/native.rs`).
- **Terminals** stream raw PTY bytes over a Tauri channel with ack-based flow control
  (`src-tauri/src/pty.rs`).
- **Sessions** are saved to the app data dir (`session.json`), debounced and on exit.

## Tests

`e2e/run.sh` launches the debug build with `MOOVE_TEST=<script>`; the script runs inside the
UI webview and drives the app through its real commands (`window.__moove`, see
`src/lib/debug.ts` and `src-tauri/src/debug.rs`, both compiled out of release builds).
Fixture pages are served from `e2e/fixtures` on port 8765. Logs and screenshots land in
`e2e/.out/`.
