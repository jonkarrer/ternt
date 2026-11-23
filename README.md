# Ternt

An Arc-style browser and terminal. Tauri v2 (Rust) + Svelte 5 (Vite, TypeScript), macOS-first.

## Install

Download the `.dmg` from the [latest release](https://github.com/jonkarrer/ternt/releases/latest),
open it and drag Ternt to Applications. Needs macOS 14 or later.

The first launch opens a welcome page with the shortcuts (later: *Help → Welcome to Ternt*), and
over it a short setup window (later: *Help → Set Up Ternt…*): whether to use the AI agent and, if
so, your API keys for it. A Fireworks AI or Anthropic key is required to go on with the agent on;
a Jev key is highly recommended; with the agent off, Ternt is a browser and a terminal.
No tabs are created for you: press ⌘T to open the first one.

After that Ternt updates itself. It checks at launch, every 15 minutes, and when you come back to it
(at most every 2 minutes). A new version downloads in the background and installs when you quit;
meanwhile a card at the bottom of the sidebar says it's ready, and *Restart Now* swaps it in right
away (your tabs come back). *Later* hides the card for that version. If Ternt didn't quit normally
(a crash), the next launch installs the downloaded update and relaunches. A release published as
required (`just upload patch --required`) installs and relaunches as soon as Ternt sees it, no
click needed. *Ternt → Check for Updates…* checks right away and answers on the same card.

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

The flake follows the org Nix standard, with the `mkDevShell` builder vendored into `nix/devshell.nix` so the build never depends on the state of the configs checkout.
Secrets live in the `ternt` Doppler project (`doppler.yaml` binds `dev`); recipes that need
them run under `doppler run --`.

`ternt <url>...` opens URLs as tabs (`bun tauri dev -- -- https://example.com`).

## Shortcuts

| Shortcut | Action |
|---|---|
| ⌘T | Command bar: open a URL, search Google, switch tabs or spaces, find pages from history or recently closed tabs, run commands (initials work: "nt" = New Terminal); `> cmd` runs `cmd` in a new terminal; the start of a site's name then Tab searches that site (`you` ⇥ YouTube, `goo` ⇥ Google, `perp` ⇥ Perplexity, `yt`, `gh`…; list in `src/lib/engines.ts`) |
| ⌘L | Edit the current URL |
| ⌥⌘T | New terminal tab |
| ⌘W / ⇧⌘T | Close tab (unloads pinned tabs; closes the focused pane of a split terminal) / reopen closed tab |
| ⌃Tab / ⌃⇧Tab, ⌘1–⌘9 | Switch tabs |
| ⌘[ / ⌘] / ⌘R | Back / forward / reload (in a split terminal, ⌘[ / ⌘] move between panes) |
| ⌥⌘I | Developer Tools: the Web Inspector for the page, in its own window (right-click → Inspect Element works too) |
| ⌘D / ⇧⌘D | Split the terminal: a new pane to the right / below |
| ⌘+ / ⌘− / ⌘0 | Terminal text bigger / smaller / back to normal (every terminal; remembered) |
| ⌘S | Toggle sidebar (hidden, it slides out over the page when the pointer touches the window's left edge) |
| ⌃F | Explore tabs: j/k or ↑/↓ move, h/l or ←/→ switch space, Space peeks, Enter opens, Esc goes back (in a terminal, ⌃F goes to the shell) |
| ⌃1–⌃9, ⌘⌥← / ⌘⌥→ | Switch spaces, stopping at the first and last (or swipe two fingers across the sidebar: left for the next space, right for the previous; with a mouse, scroll sideways or Shift-scroll over it) |
| ⌥⌘\ | A new terminal beside the current tab (in a group) |
| ⌘, | Settings (⌘1–⌘6 switch its pages, ⌘W closes it) |
| ⌃H | Shortcuts and how-tos, searchable (also in the Help menu). In a terminal, ⌃H goes to the shell (backspace) |
| ⌘H | Hide Ternt |
| ⌃A | Ask Tokidoki: this space's AI agent in a panel over the page. In the command bar (⌘T), `tok` ⇥ and what you type starts a conversation with it. One agent per space, each with its own memory; a stub answers until tokidoki is embedded (see Agent below) |

## Sidebar

Arc-style, top to bottom:

- **Favorites**: tiles showing each site's app icon (its `apple-touch-icon`, else its largest icon,
  read from the open page so signed-in sites and single-page apps get theirs), shared by every
  space with the same profile.
- **Pinned**: per space, optionally grouped in folders. An open folder is a tinted block with its
  tabs indented under it; a closed one shows how many tabs it holds. Drag tabs to reorder them
  (within a folder, or in and out of one), and drag a folder by its header to reorder folders.
  No folder is needed: while dragging, a spot under the folders pins a tab on its own.
- **Today**: per space, newest first, below a divider. Hover the divider for *Clear*, which
  closes the space's Today web tabs (terminals stay; ⇧⌘T reopens).
- **Terminals**: per space, at the bottom of the sidebar under *New Terminal*. A long Today list
  pushes them down, and they scroll with it.

**Idle tabs unload**: a web tab off screen for 30 minutes lets go of its page (each loaded page
keeps a WebKit process, often 50–150 MB). It stays in the sidebar and reloads when you open it.
Tabs playing sound or using the camera or microphone stay loaded; *Settings → General* turns this
off.

**Renaming tabs**: double-click a tab's title (or right-click → *Rename Tab…*) to give it a name;
Enter saves, Escape cancels. The name replaces the page's or shell's own title in the sidebar, the
top bar and the command bar (which still finds the tab by its title), is saved with the session and
comes back with ⇧⌘T. Clear it to go back to the title.

**Groups**: show up to four tabs together, pages and terminals alike. Drag a tab from the sidebar
onto the page area: a highlight shows where it will land (the left, right, top or bottom half of the
tab under the pointer), and dropping it there makes a group, or adds to the one on screen. That
gives two side by side or stacked, one half and two quarters (50/25/25, 25/25/50), or quarters.
Right-click a tab → *Open in Split View* adds it beside the current tab; ⌥⌘\ opens a terminal
beside it. Drag the gaps between panes to resize (double-click: back to half and half), and click
into a pane to make its tab active.

In the sidebar a group is a row (named *Group 1*, *Group 2*…; double-click to rename) with its tabs
under it, all in the group's color. Click the row to show the group; click one of its tabs to show
just that tab, full size. Closing a tab, dragging it out of the group in the sidebar, or *Remove
from Group* takes it out, and the others fill its space; the × on the group's row (or *Ungroup
Tabs*) ends the group and keeps its tabs. Groups live in Today, in one space, and are saved with
the session.

**Terminal panes**: ⌘D splits a terminal tab side by side, ⇧⌘D stacks; panes nest, and the tab
stays one row in the sidebar, named after the focused pane. Each pane runs its own shell, starting
in the folder of the pane it split from. Click a pane (or ⌘[ / ⌘]) to focus it; the others fade a
little. Drag the line between panes to resize. ⌘W or `exit` closes a pane; the last one closes the
tab. The layout is saved with the session (with fresh shells on relaunch).

**Copying from a terminal**: select and ⌘C (with nothing selected, ⌘C leaves the clipboard
alone). When a program has the mouse (tmux, vim…), ⌥-drag selects. Programs that copy for you
(tmux, vim, Claude Code, also over ssh) reach the clipboard too (OSC 52); they can't read it.

**Shift+Enter** types a new line rather than submitting (in Claude Code, for one), like ⌥Enter.

**Spaces**: hover the space name for **…** (rename, icon, color, profile, delete; also a
right-click on the name or on a space in the switcher). Every space has an icon, shown next to its
name, in the switcher and in the command bar: new spaces start with a line symbol in the space's
color; *Change Icon…* picks another symbol or an emoji (or type any emoji). *Remove* shows the
space's initial.

Drag tabs between sections, into folders, or to reorder. Closing a favorite or pinned tab
unloads it (frees memory, keeps it in the sidebar). Right-click a tab, folder or space icon
for more. Double-click a space or folder name to rename it.

**Profiles** hold your sign-ins: cookies, logins (OAuth, Google, etc.) and history. The **Default**
profile is for every space without its own; each new space gets its own (macOS 14+), named after the
space, and several spaces can share one. Sign-in popups and favorites use their space's profile.
Manage them in *Settings → Profiles* (rename a profile, move spaces between profiles, change a
space's color, clear its history), or right-click a space icon → *Profile*. The sidebar just shows
spaces: a profile is the identity sites see while you're in one (say, one Gmail for work, another
for side projects).
A profile no space uses any more is deleted.

**Air Traffic Control** (Settings → Links) sends links from other apps to a space: a rule matches
by host (`github.com` takes its subdomains too; `github.com/acme` only the pages under it) or by a
regex tried against the whole URL, and the first match opens the link as a tab in its space. Links
no rule takes go to Little Ternt or a new tab, as chosen above it. A tester there shows where a
pasted link would go.

**Downloads** (download links, files a site sends to save) go to `~/Downloads`, numbered rather
than overwriting, and the Dock's Downloads stack bounces when one finishes. A box at the bottom of
the sidebar shows each one's progress (Cancel stops it); click a finished file to open it, or the
magnifier to show it in Finder. The button at the bottom left shows or hides the box, with a ring
of progress while something downloads. A link opened in a new tab that turns out to be a file
leaves no empty tab behind. Links for other apps
(`mailto:`, `tel:`, Zoom, Slack…) open in those apps.

Links and buttons that open a new window open a tab, which the page keeps as its window (it can
point it somewhere after opening it, or post a form into it); sign-in popups that ask for a size
get a window of their own. ⌘-click (or middle-click) opens a link in a tab behind the one you're
on, ⇧⌘-click shows it.

Video calls (Meet, Zoom in the browser, Cal Video…) work: the first time a site wants your camera
or microphone it asks, and remembers if you allow it (*Settings → General* lists those sites, to
remove). Sharing your screen asks through macOS's picker.

## Little Ternt

Make Ternt your default browser (*Ternt → Make Default Browser…*, the palette, or the button in *Settings → Account* or *Links*) and links you click in other
apps open in **Little Ternt**: a small window with the page's address on top, signed in with the
space you're in. Read it and close it (⌘W), or click *Open in Ternt* to move the page into a tab.
*Settings → Links* can send those links straight to a new tab instead.

## Settings

*Ternt → Settings…* (⌘,) opens an Arc-style window with pages along the top: **Account** (your
name and email, kept on this Mac, and a summary of your profiles), **General** (version, updates, sites allowed the camera and microphone),
**Profiles** (Default and each separate profile),
**Links** (default browser, where links from other apps open), **Shortcuts** and **Advanced**
(data folder).

## Agent (Tokidoki)

Every space gets its own AI agent: tokidoki (`tokidoki/`, Ternt's copy, forked 2026-11-23 from
[jonkarrer/tokidoki](https://github.com/jonkarrer/tokidoki)), an agent with durable execution and
long-term memory. Press ⌃A, or type `tok` ⇥ and your question in the command bar (⌘T):
a panel opens over the page with that space's conversations. Agents share nothing, so what the dev
space's agent learns never reaches the marketing space's.

The agents run in a **sidecar process**, not in Ternt. Ternt's release build aborts on panic, so a
panic inside an agent (or in SQLite, or running out of memory) ends whatever process it runs in;
in its own process that is the agent, never the browser. Ternt starts the process, restarts it
with a short back-off when it dies, and the panel says so (a Restart button appears if it keeps
dying). The protocol, JSON lines over stdin and stdout, is in [docs/AGENT-SIDECAR.md](docs/AGENT-SIDECAR.md).
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

## Architecture

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

## Releases

```sh
just upload                # on the signing Mac: asks patch/minor/major, bumps, tags, builds,
                           # signs, notarizes, pushes, publishes (`just upload minor` skips the question)
just upload-resume         # publish what the last run built, after a push/upload error
```

Releases are built on a Mac with an Apple Developer ID, signed and notarized, and published to
GitHub Releases: the `.dmg` for new installs, plus the signed update archive and `latest.json`
that installed copies check shortly after launch, every 15 minutes and when you come back to the app
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
  terminal pane trees (`panes.rs`), history ranking and storage (`history.rs`), profile ids
  (`browser.rs`). Then clippy with warnings as errors.
- **End-to-end** (`e2e/run.sh`, macOS): launches the debug build once per suite in
  `e2e/tests/` with `TERNT_TEST=<script>`. The script runs inside the sidebar webview and drives
  the app through its real commands and DOM events (`t` = `window.__ternt`, see
  `src/lib/debug.ts`; native checks like keyboard focus and the clipboard are in
  `src-tauri/src/debug.rs`; both are compiled out of release builds). The app stays in the background
  during test runs (it doesn't activate on launch or when it focuses a window), so you can keep working. Fixture pages are served
  from `e2e/fixtures` on port 8765. Suites named `x-1`, `x-2` share a session and history file,
  so `x-2` tests what survives a restart. Logs and screenshots land in `e2e/.out/`; a failed
  screenshot only warns.
- **Self-update** (`e2e/update.sh`, a few minutes, not part of `bun run test`): builds signed
  0.1.0 and 0.1.1 bundles, installs 0.1.0, serves 0.1.1 as the latest release from a local
  server, then checks that the installed app pops up the update card, hides it on *Later*,
  installs, swaps its bundle and relaunches as 0.1.1. Needs a signing key in `~/.tauri/`
  (any key: the test bundles trust its public half).

Not covered (macOS won't let tests send real input): physical key presses and trackpad
gestures, native context menus opening (their actions are tested), and real OAuth providers.
