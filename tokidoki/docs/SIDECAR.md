# Tokidoki as a sidecar

A program that wants agents without having them in its own process runs `tokidoki-serve` and
talks to it over its stdin and stdout. Ternt does: an agent per space, and a crash in one
ends the sidecar, never the browser.

**The contract is ternt's**: `docs/AGENT-SIDECAR.md` at the repo root. This page says how
tokidoki meets it, and what it does beyond it. `crates/tokidoki/src/serve.rs` is the
implementation; `crates/tokidoki/src/bin/serve.rs` is the process around it.

```sh
cargo build --release -p tokidoki --bin tokidoki-serve     # the binary a host bundles
just serve                                                # or run it here, and type requests
```

## The process

```
tokidoki-serve --data <dir>
```

- Each space's agent keeps its files in `<dir>/<space>`: `tokidoki.db` (conversations and
  unfinished work), `memory.db` (what it knows), `tokidoki.journal.db` (what it did, for
  `just inspect --data <dir>/<space>`). Nothing is shared between spaces. A space's name is
  lowercase letters, digits, `-` and `_`.
- **Which provider a space runs on is the host's choice per space**, said with `configure`
  (below); **the keys are the process's**, from its environment, whichever the host has:

  | provider | key | answers with | routes with |
  | --- | --- | --- | --- |
  | `anthropic` | `ANTHROPIC_API_KEY` | `claude-opus-5-5` | `claude-haiku-5-5` |
  | `fireworks` | `FIREWORKS_API_KEY` | `accounts/fireworks/models/deepseek-v4p1-flash` | `accounts/fireworks/models/deepseek-v4p1-flash` |
  | `baseten` | `BASETEN_API_KEY` | `deepseek-ai/DeepSeek-V4-Pro` | `deepseek-ai/DeepSeek-V4.1-Flash` |

  A space's provider does everything for it: answering, the memory work, and routing a
  message to its thread. A space never configured runs on `TOKIDOKI_PROVIDER` (`anthropic`
  unless set). `TOKIDOKI_MODELS=cheap` picks each provider's cheapest models (what tests run
  on); `TOKIDOKI_MODEL` and `TOKIDOKI_ROUTER_MODEL` name models outright, for every provider.
  The environment is read once, at start: a changed key or model takes a restart; a changed
  provider or tool set for a space takes only another `configure`. Nothing else in the host's
  environment is read: `TOKIDOKI_USER`, `TOKIDOKI_SPEAKERS` and `TOKIDOKI_SYSTEM` are
  `chat`'s, not a host's.
- Without its provider's key an agent opens asleep: `threads`, `history` and the memory
  methods answer from what is on disk, unfinished work waits for a start with a key, and
  `send` answers with an error such as `FIREWORKS_API_KEY is not set`. `configure` says so
  too (`has_key: false`, `asleep: "FIREWORKS_API_KEY is not set"`).
- A thread begun on one provider goes on under another: what was said is kept, and the
  earlier model's thinking is left out of what the new one is sent.
- stdout carries the protocol and nothing else. Warnings go to stderr.
- `<dir>/.lock` is held while the process runs: a second process on the same folder exits
  at once, with a message on stderr, before saying it is ready.
- It ends when stdin closes (every agent stopped cleanly first), after `shutdown`, or on
  SIGTERM. The last two give agents 150 ms to stop, then go: the host kills at 300 ms.
  Whatever is cut off, by this or by a crash, resumes when the space is next opened.

## The lines

JSON, one object per line.

```
{"ready":{"backend":"tokidoki","version":"0.1.0",                      the first line out
          "providers":{"anthropic":true,"fireworks":false,"baseten":false},
          "default_provider":"anthropic","tools":{"web_search":true}}}
{"id":12,"method":"send","space":"space-7","params":{"input":{...}}}    a request, in
{"id":12,"result":{...}}   or   {"id":12,"error":"..."}                 its answer, out
{"space":"space-7","event":{...}}                                       an event, out
```

`ready` says which providers the process has a key for, and the tools it knows, each `true`
if it has what the tool needs to run (see Tools). An agent opens on the first request that names its space. Requests are answered as each
finishes, in any order. A space's events come in the order they happened, and each is an
`interface::Event` (`crates/interface/protocol.ts` in TypeScript). A line that isn't a
request is reported on stderr and skipped.

## Methods

In ternt's contract:

| method | params | result |
| --- | --- | --- |
| `configure` | `{ "provider": str?, "tools": [str]? }` | `{ provider, model, tools, has_key, asleep }`: how the space's agent runs from now on. A part left out keeps its value. Nothing changed: the agent stays as it is; something did: it is stopped and opened again so (its work resumes). The host calls this before a space's first other request and whenever its settings change; a space never configured runs on the process's default provider with no tools |
| `send` | `{ "input": Input, "label": str }` | `Sent`, as soon as the input is placed. `label` is the space's name as the host calls it: threads started from then on are told it after the system prompt |
| `threads` | | `[ThreadSummary]`, most recently active first |
| `history` | `{ "thread": id, "since": n }` | `[Event]`: the thread's checkpoints from `n` |
| `abort` | `{ "thread": id }` | `true` if an answer was stopped |
| `close` | | `null`; the agent opens again on its next request |
| `memories` | | `[Memory]`, newest first: what Settings → Agent → Memory lists |
| `revise` | `{ "id": n, "text": str }` | the `Memory` that replaced it (facts only) |
| `forget` | `{ "id": n }` | `null` |
| `wipe` | | `null`: every memory gone (facts, procedures, episodes); the conversations stay |
| `delete` | | `null`: the agent stopped and **all its files removed, for good** |
| `shutdown` | | `null`, then the process ends |

Beyond it, for the host to use when it has somewhere to show them:

| method | params | result |
| --- | --- | --- |
| `remember` | `{ "text": str }` | the `Memory` it now has |
| `feedback` | `{ "feedback": Feedback }` | `null`: "that was wrong", kept for debugging |

Memory changes arrive as `Event::Memory` whoever made them, so a host that never calls
these still sees what an agent learns.

## Tools

A space has the tools `configure` names, from those `ready` lists. One whose key the process
lacks is left out: the space opens all the same, and `configure` answers with the tools the
agent really has. A name the process doesn't know is an error.

| tool | needs | what it does |
| --- | --- | --- |
| `web_search` | `EXA_API_KEY` | Searches the web through Exa: up to 10 pages, each with its address, date and most relevant passages; optionally only pages from the last so many days. 15 seconds at most. Safe to run again after a crash |

A thread offers the model the tools its agent has when something is said in it, so a tool
switched on or off reaches the conversations already going, from their next message. While a
tool runs, the thread's activity is `Doing::Tool` with its name.

## Where tokidoki and the contract don't quite meet

- **Nothing flows the other way.** The process never asks the host for anything, so an
  agent can't yet act on its space (tabs, terminals). Such tools will need requests from the
  process to the host, answered like the host's own.
- **Keys are per process, not per space.** A space picks its provider; it can't bring its
  own key for it (a work account, say). That would take a key in `configure`.
