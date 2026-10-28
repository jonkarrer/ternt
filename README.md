# moove

An Arc-style browser and terminal. Tauri v2 (Rust) + Svelte 5 (Vite, TypeScript), macOS-first.

## Development

```sh
bun install
bun tauri dev              # run the app
bun tauri build            # release bundle (judge performance here, not in dev)
bun run check              # type-check Svelte/TS
e2e/run.sh                 # end-to-end tests (macOS); e2e/run.sh <name> for one
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

Right-click a tab to pin, duplicate, copy its link or close it. Drag tabs to reorder.

## Architecture

One window, several native webviews (see `src-tauri/src/browser.rs`):

| Webview | Page | Role |
|---|---|---|
| `ui` | `index.html` → `src/App.svelte` | Sidebar, transparent over native vibrancy |
| `tab-<id>` | the web | One per *loaded* web tab, no IPC access |
| `term` | `term.html` → `src/term/` | All terminal tabs (xterm.js), created on first use |
| `palette` | `palette.html` → `src/palette/` | Command bar overlay, pre-created hidden |

- **Rust owns state and layout.** Tab state lives in `Browser`; webviews mirror it via
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
