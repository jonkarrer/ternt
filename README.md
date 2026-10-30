# Ternt

An Arc-style browser and terminal. Tauri v2 (Rust) + Svelte 5 (Vite, TypeScript), macOS-first.

## Install

Download the `.dmg` from the [latest release](https://github.com/jonkarrer/ternt/releases/latest),
open it and drag Ternt to Applications. Needs macOS 14 or later.

The first launch opens a welcome page with the shortcuts (later: *Help → Welcome to Ternt*).
No tabs are created for you: press ⌘T to open the first one.

After that Ternt updates itself: when a new version is out, an **Update** button appears at the
bottom left of the sidebar. Clicking it downloads the update, swaps the app in place and relaunches
(your tabs come back). *Ternt → Check for Updates…* checks right away.

## Development

```sh
nix develop                # Bun, just, doppler (Rust comes from rustup; Xcode does C/linking)
bun install
just                       # lists every recipe
just dev                   # run the app
just build                 # release bundle (judge performance here, not in dev)
just check                 # type-check Svelte/TS, clippy
just test                  # unit tests, type check, clippy (about a second)
just test-e2e palette      # one e2e suite (VERBOSE=1 lists every check); no name runs all
```

The flake follows the org Nix standard (`mkDevShell` from `~/devjon/configs/shells/shared`).
Secrets live in the `ternt` Doppler project (`doppler.yaml` binds `dev`); recipes that need
them run under `doppler run --`.

`ternt <url>...` opens URLs as tabs (`bun tauri dev -- -- https://example.com`).

## Shortcuts

| Shortcut | Action |
|---|---|
| ⌘T | Command bar: open a URL, search Google, switch tabs or spaces, run commands (initials work: "nt" = New Terminal); `> cmd` runs `cmd` in a new terminal |
| ⌘L | Edit the current URL |
| ⌥⌘T | New terminal tab |
| ⌘W / ⇧⌘T | Close tab (unloads pinned tabs) / reopen closed tab |
| ⌃Tab / ⌃⇧Tab, ⌘1–⌘9 | Switch tabs |
| ⌘[ / ⌘] / ⌘R | Back / forward / reload |
| ⌘S | Toggle sidebar |
| ⌘F | Explore tabs: j/k or ↑/↓ move, h/l or ←/→ switch space, Space peeks, Enter opens, Esc goes back |
| ⌃1–⌃9, ⌘⌥← / ⌘⌥→ | Switch spaces (or swipe two fingers across the sidebar) |

## Sidebar

Arc-style, top to bottom:

- **Favorites**: icon tiles shared by every space with the same profile.
- **Pinned**: per space, optionally grouped in folders.
- **Today**: per space, newest first, below a divider. Hover the divider for *Clear*, which
  closes the space's Today web tabs (terminals stay; ⇧⌘T reopens).

Drag tabs between sections, into folders, or to reorder. Closing a favorite or pinned tab
unloads it (frees memory, keeps it in the sidebar). Right-click a tab, folder or space dot
for more. Double-click a space or folder name to rename it.

**Profiles**: each new space gets its own cookie jar (macOS 14+), so logins (Google, etc.) stay
in their space; sign-in popups and favorites use their space's profile. Right-click a space dot →
*Profile* to share the default profile, share another space's, or start a fresh one. A profile no
space uses any more is deleted.

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
- **Sessions** are saved to the app data dir (`session.json`), debounced and on exit. Debug
  builds use a `dev/` subfolder there, so `just dev` never leaves tabs in the installed app.
- **Welcome page** (`src-tauri/welcome.html`, served on `ternt://welcome` by
  `src-tauri/src/welcome.rs`): a plain web tab on a custom scheme, so it needs no IPC. Opened
  when there is no session file at all (first launch) and from the Help menu.
- **App icon**: `src-tauri/icons/icon.svg` is the source; render it to a 1024px PNG and run
  `bun tauri icon <png> -o src-tauri/icons` to regenerate the set.
- **History** (`src-tauri/src/history.rs`) records each page that finishes loading, per profile, in
  `history.json`; the command bar searches it in Rust as you type (match quality × frecency).

## Releases

```sh
just upload                # on the signing Mac: asks patch/minor/major, bumps, tags, builds,
                           # signs, notarizes, pushes, publishes (`just upload minor` skips the question)
just upload-resume         # publish what the last run built, after a push/upload error
```

Releases are built on a Mac with an Apple Developer ID, signed and notarized, and published to
GitHub Releases: the `.dmg` for new installs, plus the signed update archive and `latest.json`
that installed copies check shortly after launch and every 4 hours
(`src-tauri/src/updater.rs`). Updates are verified against the public key in
`tauri.conf.json`; the private key lives in Doppler (`ternt` / `dev`, `TAURI_SIGNING_PRIVATE_KEY`) and in
`~/.tauri/ternt.key` (without it, installed copies can't be updated). Setup and details: [docs/RELEASING.md](docs/RELEASING.md).

CI (`.github/workflows/ci.yml`) runs the unit tests on every push.

## Tests

`bun run test` runs, in order:

- **TypeScript unit tests** (`bun test`, `src/**/*.test.ts`): address parsing (`src/lib/url.ts`)
  and command bar ranking (`src/palette/rank.ts`, pure: data in, items with actions out).
- **Type check** (`svelte-check`).
- **Rust unit tests** (`cargo test`): the tab/space/folder model and profiles (`model.rs`),
  history ranking and storage (`history.rs`), profile ids (`browser.rs`). Then clippy with
  warnings as errors.
- **End-to-end** (`e2e/run.sh`, macOS): launches the debug build once per suite in
  `e2e/tests/` with `TERNT_TEST=<script>`. The script runs inside the sidebar webview and drives
  the app through its real commands and DOM events (`t` = `window.__ternt`, see
  `src/lib/debug.ts`; native checks like keyboard focus and the clipboard are in
  `src-tauri/src/debug.rs`; both are compiled out of release builds). Fixture pages are served
  from `e2e/fixtures` on port 8765. Suites named `x-1`, `x-2` share a session and history file,
  so `x-2` tests what survives a restart. Logs and screenshots land in `e2e/.out/`; a failed
  screenshot only warns.
- **Self-update** (`e2e/update.sh`, a few minutes, not part of `bun run test`): builds signed
  0.1.0 and 0.1.1 bundles, installs 0.1.0, serves 0.1.1 as the latest release from a local
  server, then checks that the installed app shows the Update button, installs on click, swaps
  its bundle and relaunches as 0.1.1. Needs the signing key in `~/.tauri/`.

Not covered (macOS won't let tests send real input): physical key presses and trackpad
gestures, native context menus opening (their actions are tested), and real OAuth providers.
