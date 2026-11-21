# Handoff to ternt: the real agent sidecar

## For ternt's settings panel: the user picks a provider (2026-11-21)

The user of ternt chooses who runs their agents (Anthropic, Fireworks or Baseten) and gives
their key for it. Tokidoki's side is done; ternt's panel and Keychain work are not started.

**What ternt sets on the sidecar's `Command`:**

| variable | value |
| --- | --- |
| `TOKIDOKI_PROVIDER` | `anthropic`, `fireworks` or `baseten`. Unset means `anthropic`. |
| the provider's key | `ANTHROPIC_API_KEY`, `FIREWORKS_API_KEY` or `BASETEN_API_KEY` |
| `TOKIDOKI_MODEL` | optional: the model that answers, in the provider's own naming |
| `TOKIDOKI_ROUTER_MODEL` | optional: the fast model that places a message in a thread |

- **One provider does everything.** Only the chosen provider's key is read. The old
  arrangement (Anthropic answers, Baseten routes) is gone, so `BASETEN_API_KEY` beside
  `anthropic` is ignored.
- **Read once at start.** Changing the provider, key or a model takes the same restart
  ternt already does when a key is saved.
- **The ready line now says what is running:**
  `{"ready":{"backend":"tokidoki","version":"0.1.0","provider":"fireworks","model":"accounts/fireworks/models/kimi-k3p1-pro","has_key":false}}`.
  Ternt's `Ready` type ignores the new fields today; the panel can show them.
- **No key:** the process starts, agents open asleep (see the reply below), and `send`
  fails with `<THE KEY'S VARIABLE> is not set`.
- **A provider name it doesn't know** ends the process with code 1 before the ready line.
  Only ever send one of the three.
- **Switching provider keeps the conversations.** Threads and memory are the same files.
- **Streaming** (`delta` events) now works for Fireworks and Baseten as well as Anthropic.

**Default models**, used when the model variables are unset:

| provider | answers | routes |
| --- | --- | --- |
| `anthropic` | `claude-opus-5-5` | `claude-haiku-5-5` |
| `fireworks` | `accounts/fireworks/models/kimi-k3p1-pro` | `accounts/fireworks/models/deepseek-v4p1-flash` |
| `baseten` | `deepseek-ai/DeepSeek-V4-Pro` | `deepseek-ai/DeepSeek-V4.1-Flash` |

The Fireworks and Baseten names come from their public docs and have **not been run with
a real key**. Both vendors retire model names often, so the panel should let the user type
a model, and a wrong one shows up as an unanswered submission whose reason quotes the
provider (for example `Fireworks returned 404 Not Found: Model not found…`).

**Suggested for the panel:** a provider picker, one key field per provider kept in the
Keychain under accounts `anthropic`, `fireworks` and `baseten` (so switching back doesn't
ask again), an optional model field, and the ready line's `provider` and `model` shown as
what is running. `AGENT-SIDECAR.md` needs the table above.

## Reply to `FROM-TERNT.md` (2026-11-21, later the same day)

- **Item 1 is fixed: reading no longer needs the key.** With no `ANTHROPIC_API_KEY` an agent
  now opens asleep. `threads`, `history`, `memories`, `remember`, `revise`, `forget`,
  `feedback`, `abort`, `close` and `delete` all answer as usual; only `send` fails, with
  `ANTHROPIC_API_KEY is not set`. Unfinished work is left alone and resumes at the first
  start with a key. Ternt can stop treating the `threads` error as an empty list and show
  past conversations behind the key card. Checked on the real binary and by a test
  (`asleep_it_can_be_read_and_takes_no_input`). Rebuild the sidecar to pick it up.
- **Item 4, a commit to build from:** still none. Committing is waiting on the owner's
  say-so; this file will name the commit when there is one.
- **Scope:** agreed. Nothing is being built for requests from the sidecar to ternt.
- **Still owed by either side:** the first run with a real Anthropic key.

The rest of this file is the original handoff. Where it says requests without a key all
fail, the reply above replaces it.

Written 2026-11-21 from the tokidoki repo (`~/devjon/projects/tokidoki`), for whoever picks
up the ternt side. Ternt's contract is `docs/AGENT-SIDECAR.md` in the ternt repo; tokidoki's
account of how it meets it is `docs/SIDECAR.md` here.

## Where things stand

Tokidoki now builds a binary, `tokidoki-serve`, that speaks ternt's sidecar contract. It is
meant to replace the stub in ternt's `src-tauri/agent`.

- **Checked:** the real binary was run by hand and every line it wrote (1 ready line,
  13 events, 9 responses) parsed with ternt's own `wire.rs` and `protocol.rs` types, as of
  ternt commit `2f38692`. It exits 5 ms after SIGTERM and 3 ms after `shutdown` with stdin
  held open and an agent mid-request, inside ternt's 300 ms. A restarted process lists the
  threads the last one left.
- **Not checked:** ternt has never been run against it. No exchange has been made with a
  real Anthropic key (the test used a fake one and got the 401 back as expected).
- **Nothing in tokidoki is committed.** All of this is uncommitted on `main`, so ternt can
  only build it from the working tree for now.
- **Nothing in the ternt repo was changed.**

## What ternt has to do

### 1. Build tokidoki's binary in place of the stub

`scripts/sidecar.sh` builds `-p ternt-agent` from ternt's own workspace. It needs to build
tokidoki's instead and copy it to the same names:

```sh
# debug
cargo build -q --manifest-path $TOKIDOKI/Cargo.toml -p tokidoki --bin tokidoki-serve
cp $TOKIDOKI/target/debug/tokidoki-serve binaries/ternt-agent-$host

# release: once per target, then lipo as now
cargo build -q --release --manifest-path $TOKIDOKI/Cargo.toml \
  -p tokidoki --bin tokidoki-serve --target $target
cp $TOKIDOKI/target/$target/release/tokidoki-serve binaries/ternt-agent-$target
```

To try it before touching the script, point ternt at a built binary:

```sh
cd ~/devjon/projects/tokidoki && cargo build -p tokidoki --bin tokidoki-serve
TERNT_AGENT_BIN=~/devjon/projects/tokidoki/target/debug/tokidoki-serve just dev
```

How ternt finds the tokidoki source for a release build (a path, a git dependency, a
submodule) is undecided. The binary links only system libraries, so it bundles and signs
like the stub.

### 2. Get the keys into the sidecar's environment

This is the one thing that will stop it working. The contract says keys are inherited from
ternt's environment, and ternt's spawn code sets none. Started from a terminal with
`ANTHROPIC_API_KEY` exported, that works. Started from Finder, ternt has no such variable.

Without `ANTHROPIC_API_KEY` the sidecar still starts and says it is ready; every request
that needs an agent then answers `{"id":…,"error":"ANTHROPIC_API_KEY is not set"}`. So ternt
needs somewhere to keep the key (the Keychain, as tokidoki's standalone app does in
`app/src-tauri/src/keys.rs`) and must set it on the `Command` that starts the sidecar.
A changed key takes a sidecar restart.

`BASETEN_API_KEY` is optional. Without it, a message the router can't place in code goes
to its best guess.

### 3. Know these behaviours

- **Space names** must be lowercase letters, digits, `-` and `_`, at most 64 long. Ternt's
  `space-<id>` fits.
- **A lock on the data folder.** `<dir>/.lock` is held while the process runs. A second
  sidecar on the same folder exits with code 1 and a message on stderr, before the ready
  line. If ternt ever starts a replacement before the old process is gone, the replacement
  dies; the restart loop should cover it, but it will show as a death.
- **`send` answers at once**, when the input is placed. The reply arrives as events.
- **A line that isn't a valid request** is reported on stderr and skipped, with no answer.
- **stdin closing** stops every agent cleanly with no deadline, then exits. `shutdown` and
  SIGTERM give agents 150 ms.
- **Files per space:** `<dir>/<space>/tokidoki.db`, `memory.db`, `tokidoki.journal.db`.

## Methods beyond the contract

Already served, and harmless if never called. All take `space` like the others.

| method | params | result |
| --- | --- | --- |
| `memories` | | `[Memory]`, newest first |
| `remember` | `{ "text": str }` | the `Memory` it now has |
| `revise` | `{ "id": n, "text": str }` | the `Memory` that replaced it |
| `forget` | `{ "id": n }` | `null` |
| `feedback` | `{ "feedback": Feedback }` | `null` |
| `delete` | | `null`; the agent stopped and all its files removed for good |

`delete` does not open the agent first, and succeeds for a space that has no files.
Memory changes arrive as `memory` events whoever made them, and ternt's `protocol.rs`
already has that event, so a memory view can be built from events plus `memories`.

The types are in `crates/interface/protocol.ts` (generated; `just types` regenerates it).
Ternt's hand-written `protocol.rs` matched it when checked. If tokidoki's protocol changes,
that file is what to diff against.

## Proposed changes to `AGENT-SIDECAR.md`

1. **Add the six methods above**, `delete` first: ternt has no way today to tell the
   sidecar a space is gone, so a deleted space's files stay on disk.
2. **Say how keys arrive when ternt is started from Finder** (item 2 above).
3. **A key per space.** One environment means one Anthropic key for every space. A work
   space on a work account would need a key in the request that opens the space. Not built.
4. **`label`.** Tokidoki reads it and does nothing with it: the agent isn't told what its
   space is called. Ternt reopens the agent on a rename so it "hears the new name"; today
   that reopening achieves nothing.
5. **Requests from the sidecar to ternt.** There are none, in the contract or in tokidoki,
   so an agent can't act on its space's tabs or terminals. Tools need them: a line like
   `{"ask":id,"space":…,"method":…,"params":…}` out and `{"answer":id,…}` in. This is the
   next piece of design work and neither side has started it.
6. **`when_busy`.** Ternt sends `steer`. Tokidoki accepts it; it was not exercised against
   an agent that was actually mid-answer with a real model.

## First test once it's wired

With a real key in the environment: open a space, send a message, and confirm text arrives
as `delta` events before the checkpoint that carries the whole reply. Then quit ternt
mid-answer and reopen: the thread should be listed and the answer resumed.
