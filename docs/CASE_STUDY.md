# Ternt: a case study

How a macOS browser and terminal built around a sidebar of spaces, with an AI agent in every space, went from an
empty folder to 28 signed releases in 27 days. This covers what was intended, what was chosen
and why, what went wrong, and what is still unproven. The sources are the commit history, the
docs in this repo, and the working sessions between the owner and Claude.

## At a glance

| | |
|---|---|
| What | A browser and terminal in one window: spaces in a sidebar, real terminals as tabs, one AI agent per space |
| Stack | Tauri v2 (Rust), Svelte 5, WebKit, xterm.js; the agent (tokidoki) is a separate Rust workspace run as a sidecar process |
| Platform | macOS 14 or later, universal (Apple silicon and Intel) |
| Period | 2026-10-28 to 2026-11-23: 164 commits on 16 working days |
| Releases | 28, v0.1.0 to v0.22.0, each signed, notarized and self-updating |
| Size | About 36,000 lines of Rust, 9,600 of Svelte and TypeScript |
| Tests | About 300 Rust tests, 66 TypeScript unit tests, 45 end-to-end suites |
| Team | One owner deciding, and writing parts of the agent and its settings; Claude writing, reviewing and releasing |

## The intent

The owner wanted the browser they actually work in, not a demo:

- **Organized by spaces.** Spaces, favorites, pinned tabs in folders, a Today list that is
  cleared, a command bar instead of an address bar.
- **Terminals as first-class tabs.** A shell sits in the sidebar next to the pages it relates
  to, splits into panes, and can be grouped on screen with web pages.
- **An agent that belongs to a space.** The dev space's agent learns about dev work; none of
  it reaches the marketing space.
- **Shipped, not staged.** Every change worth having goes out as a release that installed
  copies pick up by themselves.

Two constraints shaped everything else: the owner works in short requests ("pull and deploy",
"fix them all") and expects each piece to be finished and verified before the next; and
secrets never appear on a command line, in a log or in the repo.

## Timeline

| Dates | Phase | What happened |
|---|---|---|
| Oct 28 | The shell | Browser shell, command bar, terminal tabs, sessions, spaces, favorites, folders, per-space profiles: nine commits in a day |
| Oct 29–30 | Becoming a product | Renamed to Ternt; test suite; self-update; signing and notarization from a headless Mac; icon; Settings; split view; Little Ternt and default-browser support. v0.1.0 to v0.4.0 |
| Nov 2–6 | Daily-driver gaps | Terminal panes, help sheet, downloads, JS dialogs, tab groups of up to four, swipes between spaces, and the web-compatibility bugs that only daily use finds. v0.5.0 to v0.14.1 |
| Nov 16–18 | Hardening | Cursor flicker (and the crash its first fix caused), background updates, a durability and performance audit, idle tab unloading, the downloads box. v0.15.0 to v0.19.0 |
| Nov 19–20 | Two tracks | In Ternt: site search by Tab, link rules, real `window.open`, video calls (v0.20.0). In parallel the owner built tokidoki, the agent, as its own project |
| Nov 21–23 | The agent | Panel, sidecar process, real agent, bring-your-own-key, per-space provider, memory editor, fork into this repo, web search, Jev, first-launch setup, ⌃A and `tok` ⇥. v0.21.0 and v0.22.0 |

## Architecture choices

### One window, many native webviews

Each loaded web tab is its own `WKWebView`; the sidebar, command bar, terminal view, agent
panel and collapsed-sidebar peek are separate webviews layered in one window.

- **Why:** pages must be real browser views with their own processes and data stores, and the
  UI must not share a JavaScript world with them.
- **Cost:** layering native views is where most of the hard bugs came from (see the cursor
  story below).

### Rust owns state and layout

The tab, space and folder model is plain data in `model.rs`. Webviews mirror it through
`browser:state` events and never hold the truth. Content views resize with AppKit
autoresizing, so a window resize never waits on a message.

- **Why:** the model can be unit-tested without a window (13 tests on day one), and a webview
  that crashes or reloads loses nothing.
- **Cost:** every update is a whole new state. That later needed structural sharing
  (`lib/share.ts`) so only changed sidebar rows re-render.

### Tabs load lazily and unload when idle

A tab's webview is created when first shown, so restoring a large session costs about one
tab. A web tab off screen for 30 minutes drops its view and reloads when shown; tabs playing
sound or in a call stay loaded.

### Terminals: flow control, not buffering

Rust reads each PTY and streams raw bytes over a channel; xterm.js acknowledges what it has
parsed, and Rust stops reading the PTY when too much is unacknowledged. A flood throttles the
program, not the UI. All terminals share one webview, and only the visible ones hold a WebGL
renderer.

### Web pages get nothing of Ternt

Tab webviews have no IPC access, and Tauri's own injected scripts are removed from them. This
began as a bug fix (Google Docs, below) and became a rule: pages never talk to Ternt and
cannot see its internals. Even the welcome page is a plain web tab on a custom scheme.

### Profiles are the identity, spaces are the view

A profile is a WebKit data store (cookies, logins, history). New spaces get their own;
several spaces can share one. The sidebar only ever shows spaces.

## The agent

### Tokidoki, built apart and then forked in

The owner wrote tokidoki as a standalone project (Nov 19–20): inference with a provider
abstraction, a context layer, durable state, memory, a router for threads, an engine. Ternt
first integrated it as an external binary. On Nov 23 it was forked into `tokidoki/` here,
history included, so Ternt's agent can change with Ternt while the owner's own project grows
separately. The standalone repo is not touched or synced from this one.

### A sidecar process, not a library

Ternt's release build aborts on panic. A panic in an agent, in SQLite, or an out-of-memory
kill would take the browser down if the agent were linked in. So agents run in their own
process:

- JSON lines over stdin and stdout; a ready line, requests with ids, one answer each, events
  tagged with their space (`docs/AGENT-SIDECAR.md`).
- Ternt supervises: restart after 0, 1, 3, 10 seconds; a fifth death in two minutes leaves it
  down with a Restart button; the panel says what happened.
- A stub speaks the same protocol with canned replies, so the app is developed and tested
  without a key. `/crash` kills it on purpose, and a test checks Ternt carries on.
- The protocol's types have one source (tokidoki); `just sync-protocol` copies the Rust and
  TypeScript into the host, after a hand-kept mirror had already drifted.

### Scope held deliberately small

The agent is a chat with memory plus tools that need no browser. Requests from the sidecar
back into Ternt (read this page, open that tab) were designed for and then deferred by the
owner's decision. Web search through Exa is the first tool because it needs nothing from the
browser.

### Bring your own key, chosen per space

- Keys live in the macOS Keychain and reach the sidecar through its environment and no other
  way. They are never logged, journaled or written to disk; `Config`'s debug output cannot
  print one.
- Each space picks its provider (Fireworks by default, Anthropic) and its tools. The choice
  travels as a `configure` request, takes effect on the running agent, and the conversations
  go on.
- A tool whose key is missing is left out and the space still opens.
- There is no model picker. Each provider has one answering model; on Fireworks it is the
  flash model, cheap by choice.

### Jev for the small decisions

Much of an agent's work is small judgement, not writing: which thread a message belongs to,
which memories matter for it, whether it says anything worth keeping. With a TypeSafe key
these go to Jev, a fast typed-answer model, taken only when it is sure (0.8 confidence).
The provider's model still writes every word. Without the key, or when Jev fails or is
unsure, everything works as before, which makes the key recommended and never required.

### Reached by keyboard only

The agent had a button in the title bar. The owner removed it: ⌃A opens the panel, and in the
command bar `tok` ⇥ turns what is typed into the first message of a new conversation. The
agent became one more engine in the command bar, beside YouTube and Google.

### A real off switch

Setup at first launch asks whether to use the agent at all. Off means no shortcut, no command
bar entry and no sidecar process, not a hidden panel.

## Obstacles

### Google Docs would not load

Docs, Sheets and Forms showed "Loading issue". Tauri's IPC bridge defines a locked
`window.ipc` in every webview; Google's editors declare their own global `ipc` function, so
their main script threw and never started. **Fix:** remove Tauri's user scripts from web page
views right after creation.

### Google Docs was blurry

`outerWidth` and `outerHeight` were 0, because the webview library's UI delegate lacks
WebKit's window-frame callback. Docs scales its canvas by outer over inner width and drew at
half resolution. **Fix:** answer the callback with the view's frame on screen.

### The cursor flickered, and the fix crashed the app

AppKit delivers mouse moves to every tracking area, so the UI webview under a page kept
setting the arrow while the page set the pointer.

1. First fix: the webview class drops moves where another view is on top. It missed most of
   them, because WebKit's tracking belongs to a private observer object, not the view.
2. Second fix (v0.16.0): guard the observer too, and tell a covered view the mouse left. It
   passed a mouse-moved event to `mouseExited:`, which raises, and through Rust frames that
   aborted the app on mouse movement.
3. Third fix (v0.16.1, the same day): build a real mouse-exited event, and stop any exception
   in the mouse handlers.

This is the clearest case of a release going out broken. What followed from it: `just test`
now also checks the release build, and updates gained a `required` flag so a fix can install
without a click.

### Releasing from a headless Mac

The release machine is a Mac mini reached over SSH. Each step failed in its own way:

- Tauri's `.dmg` bundler arranges the window through Finder AppleScript, which fails outside
  a GUI login (error -10810). Replaced with `hdiutil`.
- `hdiutil` straight to a compressed image began producing files Apple's notary rejects as
  corrupt. Now built read-write, converted, and checked before notarizing.
- Code signing needs an unlocked keychain. A dedicated signing keychain is unlocked from a
  password file at the start of each release.
- Notarization uses an App Store Connect API key rather than an Apple ID password.
- GitHub uploads use a token from Doppler rather than the GitHub CLI.
- A tag-triggered CI release was removed so it could never publish a competing unsigned
  build.

The result is one command, `just upload <bump>`, with `--resume` for a failed push or upload.

### New windows

Refusing `window.open` and reopening the URL in a tab broke pages that open a window and then
set its location, forms posted to a new window (sent as GET), and named targets. **Fix:**
answer WebKit's create-webview call with a tab built from WebKit's own configuration, so the
page keeps a real window.

### Video calls

Releases use the hardened runtime and had no camera or microphone entitlements, so WebKit hid
`navigator.mediaDevices` from every page. Fixed with entitlements, usage descriptions, a
per-site permission sheet (the library granted every site), and a private delegate method for
screen sharing.

### Durability

A mid-November audit found that a damaged `session.json` wiped every tab, and that pasting
into a program not reading input froze the whole app. Sessions now keep a backup, an
unreadable file is set aside rather than overwritten, all saves go through one atomic writer,
and each terminal writes input from its own thread.

### The agent's lifecycle

Supervising a process turned out to have more states than it looked:

- A process that never became ready left its pid behind, so Restart signalled a dead pid.
- Quitting could start a new process inside the shutdown delay and orphan it holding a lock.
- Switching the agent off during a back-off pause or a spawn did not hold; a late "off" could
  undo a later "on".

The final shape keeps what is wanted in one flag, read wherever a process could be started or
kept, and the loop checks again after each ending.

### Switching a tool off mid-conversation

After web search was switched off, Anthropic returned empty replies: the history held tool
calls for a tool no longer offered. **Fix:** before a model request, calls and results of
tools not on offer are retold as plain text.

### A test that proved nothing

The real-agent suite's restart check passed whether or not a restart happened. The status now
carries a generation number so a new process can be told from the old.

## How the work was done

- **Step by step.** One piece built, verified and reported; then the next. Commits only when
  asked.
- **Review as its own pass.** After each large piece, a fresh review and a simplify pass. The
  reviews found real bugs each time: three in the agent host, four in the sidecar, ten after
  the owner's Jev and setup commits. Review also removed code: a trait, boxed futures and a
  per-space map went once the sidecar was the only agent.
- **Tests drive the real app.** End-to-end scripts run inside the sidebar webview and use the
  app's own commands and DOM events, one launch per suite, in the background so the owner can
  keep working. Suites named `x-1`, `x-2` share a session to test what survives a restart. The
  self-update test installs one signed version and watches it become the next.
- **Cheap models in tests, always.** The owner's rule after the first live runs: any test that
  calls a model uses each provider's cheapest (`TOKIDOKI_MODELS=cheap`).
- **Secrets by reference.** Doppler in development, the Keychain in the product. No key on a
  command line or in output, in the sessions as in the code.
- **Small releases, often.** 28 in 27 days. A patch or minor is judged from the commits; the
  update manifest is checked after every publish.
- **Docs carry the decisions.** `docs/` is the map. `CLAUDE.md` holds what the code cannot
  say: standing decisions with their date and who made them. `PICKUP.md` was written so the
  agent work could be resumed from a cold start.

## What is not proven

- macOS will not let tests send real input. Physical key presses (⌃A among them), trackpad
  gestures and native context menus opening are untested; their actions are tested.
- Screenshots could not be taken in the release sessions, so the look of the newest Settings
  page, the setup window and the `tok` pill was never checked there.
- The first-launch path that stores a typed key in the Keychain, and a live conversation with
  a Jev key in the installed app, have not been walked by hand.
- ⌃A takes start-of-line (and the screen/tmux prefix) away from terminals and text fields.
  This is a known trade the owner has not yet lived with.
- One agent suite failed twice right after front-end edits and then passed repeatedly. The
  cause was not found.
- Real OAuth providers are not covered by tests.

## What it shows

1. **A thin native layer is not a small one.** Most of the hard problems were in the seam
   between WebKit, AppKit and the webview library: private observers, missing delegate
   callbacks, injected globals. Each needed the real cause found, not a workaround.
2. **Isolation paid for itself twice.** Keeping Ternt out of web pages fixed Google's editors
   and closed a security question. Keeping the agent out of Ternt's process let an unfinished
   agent ship inside a browser people depend on.
3. **A stub behind a written protocol decouples two projects.** The panel, supervisor and
   tests were finished before the real agent was connected, and connecting it changed little.
4. **Deferring is a design choice.** "No requests back to Ternt yet" kept the agent shippable
   and its failure modes few. The capability the product is ultimately for is still ahead.
5. **Fast releases need a fast way back.** The one crash release was fixed in hours because
   the pipeline was one command, and it led directly to checking release builds in tests and
   to required updates.
6. **Reviews find what tests are not asked.** Lifecycle races, a vacuous check and a drifted
   protocol mirror were all found by reading, then pinned by tests.

## Next

Offered and not yet decided: a per-space model picker, a cheaper model for memory work, and
letting the agent read and act on its space, which waits on the owner lifting the decision
that the sidecar makes no requests of Ternt.
