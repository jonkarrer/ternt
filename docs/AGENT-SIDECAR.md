# The agent sidecar protocol

Ternt runs its AI agents in a separate process, the *sidecar*, so that a panic, an abort or
running out of memory in an agent ends that process and never the browser. Ternt starts the
process, talks to it over its stdin and stdout, restarts it when it dies, and shows the panel
what happened. This document is the contract between the two. Two programs implement it: `tokidoki-serve`,
the real agent (tokidoki's `docs/SIDECAR.md` says how it meets this), and the stub in
`src-tauri/agent/`, which answers with canned text so Ternt can be tested without a key.
`scripts/sidecar.sh` builds either; `src-tauri/src/agent/wire.rs` is the Rust definition of
the lines and `src-tauri/src/agent/sidecar.rs` is Ternt's side.

## Process

```
ternt-agent --data <dir>
```

- `<dir>` is where agents keep their files (`~/Library/Application Support/com.jokar.ternt/agents`,
  with `/dev` in the path for debug builds). Each agent uses `<dir>/<space>`; `<space>` is the
  space's folder name, `space-<id>`, so a renamed space keeps its agent.
- A space's folder name is lowercase letters, digits, `-` and `_`, at most 64 characters (tokidoki
  rejects anything else). `space-<id>` always fits.
- Keys come from the environment: `ANTHROPIC_API_KEY` (required to answer) and `BASETEN_API_KEY`
  (optional). Ternt sets them on the process, and nowhere else, from the user's Keychain (service
  `com.jokar.ternt`, accounts `anthropic` and `baseten`; `.dev` in debug builds), where the
  panel's *API keys* card puts them (`src-tauri/src/agent/keys.rs`). A variable already in
  Ternt's own environment wins, for development. Saving a key restarts the process, since the
  environment is fixed at start. **Without an Anthropic key the process must still start and say
  it is ready**; it answers requests that need the model with an error. Ternt knows whether it
  gave a key (`hasKey` in the panel's status) and asks for one instead of sending.
- One environment means one Anthropic key for every space. A key per space (a work account for a
  work space) is not designed: it would need a key in the request that opens the space.
- tokidoki holds `<dir>/.lock` while it runs: a second process on the same folder exits with code 1
  before its ready line. Ternt only starts a replacement after `wait` on the old process, so this
  happens only if something else runs a sidecar on Ternt's folder.
- stdout is the protocol: nothing else may be written there. stderr is free-form and ends up in
  Ternt's log prefixed `ternt-agent:`.
- The process exits when stdin closes or after answering `shutdown`. When Ternt itself exits it
  sends SIGTERM, then SIGKILL 300 ms later: finish any write in progress on SIGTERM.

## Lines

JSON, one object per line (`\n`), UTF-8. No line may contain a raw newline.

**The first line** the process writes:

```json
{"ready":{"backend":"tokidoki","version":"0.3.1"}}
```

Ternt waits up to 15 seconds for it; without it the process is killed and the panel shows the
agent as down.

**Requests**, Ternt to the process:

```json
{"id":12,"method":"send","space":"space-7","params":{"input":{...},"label":"dev"}}
```

`id` is unique per process lifetime; `space` is present for every method but `shutdown`;
`params` is left out when empty. A line that isn't a valid request is reported on stderr and
skipped, with no answer. Requests may be answered in any order, and `send` must return
as soon as the input is placed, not when the answer is ready.

**Responses**, one per request:

```json
{"id":12,"result":{"thread":"t1","submission":"s9","route":null}}
{"id":13,"error":"no thread t9"}
```

**Events**, whenever an agent does something, unsolicited and interleaved with responses:

```json
{"space":"space-7","event":{"type":"delta","thread":"t1","delta":{"kind":"text","text":"Hel"}}}
```

The `event` is tokidoki's `interface::Event`, exactly as `events()` yields it. Ternt tags each
with its space on the way to the panel.

## Methods

| method     | params                              | result                           |
|------------|-------------------------------------|----------------------------------|
| `send`     | `{ "input": Input, "label": str }`  | `Sent`                           |
| `threads`  |                                     | `[ThreadSummary]`, newest first  |
| `history`  | `{ "thread": id, "since": n }`      | `[Event]`: checkpoints from `n`  |
| `abort`    | `{ "thread": id }`                  | `true` if a turn was stopped     |
| `close`    |                                     | `null`; the agent may be reopened|
| `delete`   |                                     | `null`; the agent stopped and **all its files removed, for good** |
| `shutdown` |                                     | `null`, then exit                |

Ternt calls `delete` when the user deletes a space (`agent::forget`), so the conversations and
memory go with it. It must succeed for a space that was never opened or has no files.

All types are `tokidoki::interface` (`crates/interface/protocol.ts` in TypeScript;
`src/agent/protocol.ts` is Ternt's copy, `src-tauri/src/agent/protocol.rs` the same in Rust).
`label` is the space's current name, informational: an agent may use it to know what its space
is called. tokidoki reads it and does nothing with it yet, so reopening an agent on a rename
achieves nothing today; Ternt keeps sending it.

An agent opens on the first method naming its space (`Spaces::open`), and stays open until
`close`, `delete` or exit. Ternt may name spaces it has never named before at any time.

### Served by tokidoki, not called by Ternt yet

For when the panel has somewhere to show them. All take `space` like the others.

| method     | params                        | result                                  |
|------------|-------------------------------|-----------------------------------------|
| `memories` |                               | `[Memory]`, newest first                |
| `remember` | `{ "text": str }`             | the `Memory` it now has                 |
| `revise`   | `{ "id": n, "text": str }`    | the `Memory` that replaced it           |
| `forget`   | `{ "id": n }`                 | `null`                                  |
| `feedback` | `{ "feedback": Feedback }`    | `null`; "that was wrong", kept for debugging |

Memory changes arrive as `memory` events whoever made them, so a memory view is these plus the
events. The stub answers `unknown method` to all of them.

## Not in the contract yet

- **Requests from the sidecar to Ternt.** Nothing flows that way, so an agent can't act on its
  space (read a tab, run something in a terminal). Tools need it. The shape that fits the lines
  above: `{"ask":<id>,"space":"space-7","method":"tabs","params":{}}` out of the process, and
  `{"answer":<id>,"result":…}` or `{"answer":<id>,"error":"…"}` back in, with Ternt answering
  from its own state. **Deferred**: the agent is a chat with memory for now, by decision
  (2026-11-21), and neither side should build this until that changes.
- **A key per space** (above).
- **`when_busy: steer`**: Ternt sends it when a thread is mid-answer; tokidoki accepts it but it
  hasn't been exercised against a real model yet.

## What Ternt does when the process dies

Every pending request fails with "The agent process exited.", the panel shows "starting it
again", and the process is started over after a pause of 0, 1, 3 then 10 seconds. A fifth death
within two minutes leaves it down, with a Restart button. After a restart the panel re-reads
`threads` and `history`, so an agent that resumes from disk (tokidoki) shows everything it had,
and one that doesn't (the stub) shows a fresh start.

A restart Ternt asks for (the Restart button, saving a key) is not a death: the process gets
SIGTERM, then SIGKILL 300 ms later if it is still there, and comes back with no pause, no crash
banner and the restart budget cleared.

Two stub inputs exist for testing this: `/crash` aborts the process, `/exit` ends it cleanly.
