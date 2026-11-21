# Tokidoki: handoff to the next owner

Written 2026-11-21 by the agent that did the work described here, for an agent picking the
project up cold. It says what tokidoki is, where it is going, what state the tree is in,
what has and hasn't been verified, and what to do next. Where this file and the code
disagree, the code is right.

## What it is

A personal-assistant agent in Rust: durable execution (work survives a crash and resumes)
and automatic long-term memory (it learns from conversations without being told to). The
owner's goal, in their words, is "an alive agent that grows with the user".

**Where it runs:** inside **ternt**, the owner's own Tauri browser/terminal app at
`~/devjon/projects/ternt`, as a sidecar process. Each ternt *space* (a "marketing" space, a
"dev" space, a day-job space) gets its own agent with its own files, so each grows into its
space separately. Nothing is shared between spaces yet; the owner may want some sharing
later.

**Scope, decided by the owner 2026-11-21:** the agent is **a chat with memory** for now.
Tools that act on ternt (tabs, pages, terminals) are deferred. Don't build them until the
owner says so.

## The first thing to know: nothing is committed

Everything from this body of work is uncommitted on `main` (33 changed or new paths; last
commit `75c0113`). That includes all of `app/`, `docs/`, `scripts/`, the sidecar, streaming
and the provider choice. Ternt builds the sidecar from this working tree, and its releases
keep shipping a stub until there is a tokidoki commit to name. **Ask the owner before
committing**; they have not asked for one. When you do commit, put the hash in
`docs/TERNT-HANDOFF.md`, because ternt is waiting for it.

## Working with the owner

- They work **step by step**: one piece, reported, then the next. They redirect often
  (standalone app → library in ternt → sidecar in ternt, all in one day), so keep each step
  small and don't build ahead.
- They ask for a **review, then a simplify pass**, after a chunk of work. Expect it.
- **Bring-your-own keys**, **macOS only**.
- **Ternt is theirs and has its own agent.** Don't edit the ternt repo from here. The two
  sides trade notes through files in *this* repo: `docs/TERNT-HANDOFF.md` (to ternt, newest
  section at the top) and `docs/FROM-TERNT.md` (from ternt). When the owner says "ternt
  gave you more notes", read `docs/FROM-TERNT.md`.
- **Code style in this repo:** every `lib.rs` opens with what the crate is for; comments
  are plain prose saying why; test names are sentences (`the_sidecar_ends_when_its_input_does`).
  Match it.

## Layout

A Cargo workspace (edition 2024). Each crate's `src/lib.rs` header is the real description;
this is the map.

| crate | what it does |
| --- | --- |
| `context` | What a model is sent: system prompt, tools, messages. Vendor-neutral. |
| `inference` | The HTTP to model providers. `Provider<A>`; `A` is the format: `Anthropic`, `OpenAi` (also Baseten, Fireworks), or `AnyApi` when the vendor is chosen at run time. Streaming is put back together into the non-streaming reply, so everything downstream sees one shape. |
| `state` | A thread is an append-only log of checkpoints; context is rebuilt from it. SQLite. |
| `engine` | Durable tasks: every step is one atomic commit; a reopened engine resumes. |
| `memory` | Long-term memory (semantic, procedural, episodic). Passive store. |
| `attention` | Cheap detectors that raise signals ("may need a recall"). No model calls. |
| `experience` | Decides what to recall and retain from those signals; dreams when idle. |
| `router` | Sends each input to a thread: rules first, a fast model as last resort. |
| `agent` | The agent loop as engine tasks; `Live` broadcasts text as it is written. |
| `interface` | The protocol as serde types. `protocol.ts` is generated from it. |
| `telemetry` | The journal (what the agent did) and the `inspect` binary that reads it. |
| `tokidoki` | Puts it all together: `Tokidoki`, `Spaces`, `serve`, and the binaries. |

Also:

- `crates/tokidoki/src/bin/serve.rs`: **`tokidoki-serve`**, the sidecar. This is the product.
- `crates/tokidoki/src/bin/chat.rs`: a terminal chat for trying things (`just chat`).
- `app/`: a standalone Tauri + Svelte app, built first and then **overtaken** by the move
  into ternt. See "Loose ends".
- `scripts/`, `docs/RELEASING.md`: signing, notarizing and publishing that standalone app.
- `docs/SIDECAR.md`: how the sidecar behaves. The contract itself is ternt's,
  `docs/AGENT-SIDECAR.md` in the ternt repo.

## How the sidecar works

`tokidoki-serve --data <dir>` speaks newline-delimited JSON on stdin and stdout. Read
`docs/SIDECAR.md` for the full account; the essentials:

- First line out is `{"ready":{…}}` with `backend`, `version`, `provider`, `model`, `has_key`.
- Requests `{id, method, space, params}`; answers `{id, result}` or `{id, error}` in any
  order; events `{space, event}`, each an `interface::Event`.
- An agent opens on the first request naming its space, in `<dir>/<space>/` (`tokidoki.db`,
  `memory.db`, `tokidoki.journal.db`). Space names: lowercase letters, digits, `-`, `_`.
- Methods: `send`, `threads`, `history`, `abort`, `close`, `delete`, `shutdown`, and beyond
  ternt's original contract `memories`, `remember`, `revise`, `forget`, `feedback`.
- `<dir>/.lock` stops a second process on the same folder.
- It exits on stdin close, `shutdown` or SIGTERM. Ternt kills at 300 ms, so the last two
  give agents 150 ms. `main` calls `std::process::exit` on purpose: returning would wait on
  the thread reading stdin, which the host holds open (this was a real 29-second hang).

**Provider and key** come from the environment, read once at start:

| `TOKIDOKI_PROVIDER` | key | answers with | routes with |
| --- | --- | --- | --- |
| `anthropic` (default) | `ANTHROPIC_API_KEY` | `claude-opus-5-5` | `claude-haiku-5-5` |
| `fireworks` | `FIREWORKS_API_KEY` | `accounts/fireworks/models/deepseek-v4p1-flash` | `accounts/fireworks/models/deepseek-v4p1-flash` |
| `baseten` | `BASETEN_API_KEY` | `deepseek-ai/DeepSeek-V4-Pro` | `deepseek-ai/DeepSeek-V4.1-Flash` |

`TOKIDOKI_MODEL` and `TOKIDOKI_ROUTER_MODEL` override the defaults (`Config::models`).
One provider does everything: answers, memory work and routing. Without the key, agents
open **asleep** (`Tokidoki::open_asleep`): reading and memory edits work, unfinished work is
left untouched, and only `send` fails.

## Design decisions worth not re-litigating

- **Sidecar, not in-process.** A crash in an agent must never end the browser. An earlier
  in-process embedding (`checks/embed`, `docs/EMBEDDING.md`) was deleted for this.
- **One agent per space, separate files.** `Spaces` owns the lifecycle; one async mutex
  serializes open, close and delete so a space is never opened while its folder is going away.
- **Each agent has its own journal** in a process that runs many. `telemetry::record`
  returns a span; events inside it go to that journal. The engine keeps the span it was
  opened in and applies it to tasks it spawns.
- **Deltas are ordered against commits.** A streamed piece carries the checkpoint count it
  is ahead of, so a client never sees text after the checkpoint that contains it.
- **`Tokidoki::close` is async and waits** (up to 10 s) for work to stop, so files can be
  reopened or deleted straight after.
- **One provider for everything** was my reading of "select the provider and provide the
  key". Before, Anthropic answered and Baseten routed. The owner has been told and has not
  objected or confirmed.

## What is verified and what is not

Verified:

- `cargo test --workspace`: 153 pass. `cargo clippy --workspace --all-targets -- -D warnings`
  clean. `cargo test -p interface --features ts` passes (the generated TypeScript is current).
- The real `tokidoki-serve` binary, by hand: ready line, lazy open, events, the folder lock,
  each provider setting, keyless reading, restart resuming threads, exit 5 ms after SIGTERM.
- Ternt's own wire types parse its output, and ternt's e2e harness has started it (per
  `docs/FROM-TERNT.md`).

**Not verified, and the biggest risk in the project:**

- **No model call has ever succeeded in this work.** There was no API key on this machine.
  Anthropic streaming, the OpenAI-style streaming for Fireworks and Baseten, and routing on
  `claude-haiku-5-5` are tested only against documented wire formats and fake keys.
- **The Fireworks and Baseten default model names are from public docs**, not from a
  working call. Both vendors retire names often. A fake-key call to Fireworks returned
  404 "Model not found, inaccessible, and/or not deployed", which doesn't tell a bad key
  from a bad name.
- **No UI has been seen.** The shell here has no display, so the standalone app's interface
  was never looked at.
- **The release pipeline** (`scripts/publish.sh`, notarization) has not run end to end.
  The `jonkarrer/tokidoki-dist` repo returned 404 when checked.

The ignored tests are the live ones: `just test inference --live` runs them with real keys
from `.env`. Running those is the cheapest way to retire most of the list above.

## What to do next

Update 2026-11-21, later the same day: ternt's owner handed both repos to one agent, so the
note-passing below is history. Done since: the baseline commit (`8c807b5`), a real-key run on
both providers through ternt (streamed deltas, checkpoint, thread back after a restart), the
Fireworks default model fixed to one the account serves (`deepseek-v4p1-flash`, cheap by the owner's choice, with live tests),
request bodies kept out of the stderr warnings, and `label` used: new threads are told their
space's name. Items 1, 2 and 4 below are therefore done; 3 landed on ternt's side. Later that evening,
after a review of both sides: the standalone app (`app/`, with `scripts/` and
`docs/RELEASING.md`) was retired, since ternt builds, bundles and releases the sidecar (it is
in git history at `8c807b5` if ever wanted); a request whose handler panics is still answered;
`close_all` lets nothing reopen; the sidecar reads only the provider and model variables.

In rough order; confirm with the owner before starting any of them.

1. **A real-key run**, for each provider the owner has a key for. Ternt's
   `TERNT_SIDECAR=tokidoki e2e/run.sh tokidoki` asserts the right sequence (`delta` events,
   then the checkpoint with the whole reply, then `turn_ended`, then the thread listed, and
   again after a restart).
2. **Commit**, once the owner says to, and name the commit in `docs/TERNT-HANDOFF.md`.
3. **Support ternt's settings panel.** Ternt builds it; the instructions are at the top of
   `docs/TERNT-HANDOFF.md`. Expect follow-up notes in `docs/FROM-TERNT.md`.
4. **Use `label`.** Ternt sends the space's name with every `send`; tokidoki reads it and
   does nothing. Telling the agent what its space is called fits "grows into its space".
5. **A memory view** in ternt needs nothing more from tokidoki: the methods and the
   `memory` events exist.

Deferred by the owner, don't start: requests from the sidecar to ternt (tools), a key per
space, anything shared between spaces. The older roadmap also had tools with approvals,
proactivity (the agent speaking first), a menu-bar presence and non-text inputs.

## Loose ends

- **`app/` is overtaken.** It still compiles, but it is pinned to Anthropic and its Baseten
  key field now does nothing. It needs the provider picker or retiring. The owner hasn't
  said which; ask. `README.md`'s "The app" section describes it as it was.
- **`README.md` line 14** says the live tests read `ANTHROPIC_API_KEY` and
  `BASETEN_API_KEY`; `just chat` now reads `TOKIDOKI_PROVIDER` and that provider's key.
- **The sidecar is 16 MB per architecture**, 31 MB universal: most of ternt's download.
  Nobody has tried to shrink it.
- **`when_busy: steer`**, which ternt sends when a thread is mid-answer, is accepted but
  has never been exercised against a real model.
- **There is no `CLAUDE.md`** in the repo. If the next owner is Claude Code, the standing
  parts of this file (the owner's preferences, the ternt note-passing, the style) belong there.

## Commands

```sh
nix develop --impure     # dev shell: just, jq, bun, tursodb (Rust is your own toolchain)
just check               # fmt, clippy -D warnings, the TypeScript protocol check, app type-check
just test                # all tests;  just test inference --live  for the real-API ones
just types               # regenerate crates/interface/protocol.ts after changing the protocol
just serve               # run the sidecar here and type requests at it
just chat                # talk to a fresh agent in the terminal (needs a key in .env)
just inspect recent      # read what an agent did; add --data <dir>/<space> for a sidecar's
```

To try the sidecar by hand without a key:

```sh
cargo build -p tokidoki --bin tokidoki-serve
echo '{"id":1,"method":"threads","space":"space-1"}' | ./target/debug/tokidoki-serve --data /tmp/t
```

## Outside this repo

- **Ternt:** `~/devjon/projects/ternt`. Its side of the sidecar is in
  `src-tauri/src/agent/` (`sidecar.rs`, `wire.rs`, `protocol.rs`, `keys.rs`);
  `scripts/sidecar.sh tokidoki` builds this repo's binary from `$TOKIDOKI_SRC` (default
  `../tokidoki`); `TERNT_AGENT_BIN` points ternt at any binary.
- **Updater signing key** for the standalone app: `~/.tauri/tokidoki.key`, `.key.password`,
  `.key.pub`. Apple notarization secrets are in ternt's Doppler project.
- **The owner's sibling apps** (ternt, bernt, lernt) share one Apple team and the same
  release pipeline, which `scripts/` here was adapted from.
