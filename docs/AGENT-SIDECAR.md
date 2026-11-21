# The agent sidecar protocol

Ternt runs its AI agents in a separate process, the *sidecar*, so that a panic, an abort or
running out of memory in an agent ends that process and never the browser. Ternt starts the
process, talks to it over its stdin and stdout, restarts it when it dies, and shows the panel
what happened. This document is the contract between the two, for tokidoki's `serve` command
to implement. The stub in `src-tauri/agent/` implements it today; `src-tauri/src/agent/wire.rs`
is the Rust definition, `src-tauri/src/agent/sidecar.rs` is Ternt's side.

## Process

```
ternt-agent --data <dir>
```

- `<dir>` is where agents keep their files (`~/Library/Application Support/com.jokar.ternt/agents`,
  with `/dev` in the path for debug builds). Each agent uses `<dir>/<space>`; `<space>` is the
  space's folder name, `space-<id>`, so a renamed space keeps its agent.
- Keys come from the environment (`ANTHROPIC_API_KEY`, `BASETEN_API_KEY`), inherited from Ternt.
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
`params` is left out when empty. Requests may be answered in any order, and `send` must return
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
| `shutdown` |                                     | `null`, then exit                |

All types are `tokidoki::interface` (`crates/interface/protocol.ts` in TypeScript). `label` is
the space's current name, informational: an agent may use it to know what its space is called.

An agent opens on the first method naming its space (`Spaces::open`), and stays open until
`close` or exit. Ternt may name spaces it has never named before at any time.

## What Ternt does when the process dies

Every pending request fails with "The agent process exited.", the panel shows "starting it
again", and the process is started over after a pause of 0, 1, 3 then 10 seconds. A fifth death
within two minutes leaves it down, with a Restart button. After a restart the panel re-reads
`threads` and `history`, so an agent that resumes from disk (tokidoki) shows everything it had,
and one that doesn't (the stub) shows a fresh start.

Two stub inputs exist for testing this: `/crash` aborts the process, `/exit` ends it cleanly.
