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
- **Each space picks its provider; the keys come from the environment.** Ternt sets every key
  the user has on the process, `FIREWORKS_API_KEY` and `ANTHROPIC_API_KEY` (tokidoki also knows
  `BASETEN_API_KEY`; Ternt doesn't offer it), and tells the process per space, with `configure`
  before that space's first other request, which provider its agent runs on and which tools it
  has. The agent answers, routes and remembers with that provider. Ternt is bring-your-own-key:
  the keys live in the user's Keychain (service `com.jokar.ternt`, accounts `fireworks`,
  `anthropic`, `exa` and `typesafe`; `.dev` in debug builds), put there by Settings → Agent → Keys
  (`src-tauri/src/agent/keys.rs`), and reach the process this way and no other. A variable
  already in Ternt's own environment wins, for development (`doppler run -- just dev tokidoki`).
  Saving or removing a key restarts the process, since its environment is fixed at start;
  changing a space's provider or tools does not, `configure` reaches the running agent.
  **Without a provider's key the process must still start and say it is ready**; an agent on
  that provider opens asleep, answers `threads`, `history` and the memory methods from disk,
  and `send` with an error. `configure` says so (`has_key: false`, `asleep: "…"`), which is how
  the panel knows to point at Settings instead of sending (`agent_space_status`).
- **Tools are switched on per space** (Settings → Agent → Space), by the names the process
  knows. Today there is one, `web_search`, which needs `EXA_API_KEY` (Keychain account `exa`),
  set on the process like the providers' keys. A space that asks for a tool the process has
  no key for still opens, without it: `configure` answers with the tools the agent really
  has. Settings lets a tool be switched on only when its key is there. A tool call shows in
  the panel as a tool row, from the thread's checkpoints like any message.
- **The agent can be off** (`Prefs::agent_enabled`, asked at first launch in the setup window,
  kept in Settings → Agent). Off, Ternt doesn't run the process at all: its status is `down`
  with the reason saying so, ⌃A and the command bar's `tok` are put away, and a saved key restarts
  nothing. Switching it on starts the process with the keys as they are then.
- **Jev decides the small things, when there is a TypeSafe key** (`TYPESAFE_API_KEY`,
  Keychain account `typesafe`, optional, set on the process like the others). It is not a
  provider and not a tool: in every space, whichever provider answers there, the router
  asks Jev which conversation a message belongs to before it would ask the provider's fast
  model, and takes the answer only when Jev is sure; and each turn Jev picks the memories
  worth recalling for the message and flags a message worth remembering. Without the key,
  or when Jev fails, the agent works as before (the provider routes, a keyword search
  recalls). Nothing in `configure` or the ready line says so.
- The provider and tools are per space (`Space::agent` in the session, `setup.rs`); a key per
  space (a work account for a work space) is not designed: it would need a key in `configure`.
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
| `configure` | `{ "provider": str, "tools": [str] }` | how the agent runs now: `{ provider, model, tools, has_key, asleep }`. Same setup: the agent stays as it is; a change: it is stopped and opened again so, its work resuming. Ternt sends it before a space's first other request in a process's life and when Settings changes the space |
| `send`     | `{ "input": Input, "label": str }`  | `Sent`                           |
| `threads`  |                                     | `[ThreadSummary]`, newest first  |
| `history`  | `{ "thread": id, "since": n }`      | `[Event]`: checkpoints from `n`  |
| `abort`    | `{ "thread": id }`                  | `true` if a turn was stopped     |
| `memories` |                                     | `[Memory]`, newest first         |
| `revise`   | `{ "id": n, "text": str }`          | the `Memory` that replaced it (facts only) |
| `forget`   | `{ "id": n }`                       | `null`; the memory and its history removed |
| `wipe`     |                                     | `null`; everything the agent learned removed (facts, procedures, episodes), its conversations kept |
| `delete`   |                                     | `null`; the agent stopped and **all its files removed, for good** |
| `close`    |                                     | `null`; the agent may be reopened|
| `shutdown` |                                     | `null`, then exit                |

Settings → Agent → Memory lists a space's facts through `memories`, edits one with `revise`,
removes one with `forget`, and Wipe calls `wipe`; it reads the list again when the window is
focused rather than following `memory` events (the panel's forwarder only delivers events while
the panel shows). Ternt calls `delete` when the user deletes a space (`agent::forget`), so the
conversations and memory go with it. It must succeed for a space that was never opened or has no files. Ternt
doesn't call `close` or `shutdown` today: it signals instead (below), and agents stay open for
the life of the process.

All types are `tokidoki::interface` (`crates/interface/protocol.ts` in TypeScript;
`src/agent/protocol.ts` is Ternt's copy, `src-tauri/src/agent/protocol.rs` the same in Rust).
`label` is the space's current name. tokidoki tells each new thread what space it is the agent
of, after the system prompt; threads already started keep what they were told, so a rename
reaches the next new conversation.

An agent opens on the first method naming its space (`Spaces::open`), and stays open until
`close`, `delete` or exit. Ternt may name spaces it has never named before at any time.

### Served by tokidoki, not called by Ternt yet

All take `space` like the others.

| method     | params                        | result                                  |
|------------|-------------------------------|-----------------------------------------|
| `remember` | `{ "text": str }`             | the `Memory` it now has                 |
| `feedback` | `{ "feedback": Feedback }`    | `null`; "that was wrong", kept for debugging |

Memory changes arrive as `memory` events whoever made them. The stub answers `unknown method`
to these two; it serves the memory methods above with one pretend fact per answered question.

## Not in the contract yet

- **Requests from the sidecar to Ternt.** Nothing flows that way, so an agent can't act on its
  space (read a tab, run something in a terminal). Tools need it. The shape that fits the lines
  above: `{"ask":<id>,"space":"space-7","method":"tabs","params":{}}` out of the process, and
  `{"answer":<id>,"result":…}` or `{"answer":<id>,"error":"…"}` back in, with Ternt answering
  from its own state. **Deferred**: the agent is a chat with memory for now, by decision
  (2026-11-21), and neither side should build this until that changes.
- **A key per space** (above).
- **`when_busy`**: Ternt's panel holds input while a turn is under way (Stop ends it), so inputs
  are sent with the default and `steer` has never been exercised against a real model.

## What Ternt does when the process dies

Every pending request fails with "The agent process exited.", the panel shows "starting it
again", and the process is started over after a pause of 0, 1, 3 then 10 seconds. A fifth death
within two minutes leaves it down, with a Restart button. After a restart the panel re-reads
`threads` and `history`, so an agent that resumes from disk (tokidoki) shows everything it had,
and one that doesn't (the stub) shows a fresh start.

A restart Ternt asks for (the Restart button, saving or removing a key) is not a death: the process gets SIGTERM, then SIGKILL 300 ms later if it is still there, and comes back
with no pause, no crash banner and the restart budget cleared; asked during a back-off pause, it
cuts the pause short. When Ternt quits it does the same with no restart to follow. The status
carries a `generation` that changes with every process started, for anyone who needs to tell a
new process from the old one.

Two stub inputs exist for testing this: `/crash` aborts the process, `/exit` ends it cleanly.
