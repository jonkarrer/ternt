# Tokidoki as a sidecar

A program that wants agents without having them in its own process runs `tokidoki-serve` and
talks to it over its stdin and stdout. Ternt does: an agent per space, and a crash in one
ends the sidecar, never the browser.

**The contract is ternt's**: `docs/AGENT-SIDECAR.md` in the ternt repo. This page says how
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
- The provider is the user's choice, and comes from the environment with their key for it:

  | `TOKIDOKI_PROVIDER` | key | answers with | routes with |
  | --- | --- | --- | --- |
  | `anthropic` (the default) | `ANTHROPIC_API_KEY` | `claude-opus-5-5` | `claude-haiku-5-5` |
  | `fireworks` | `FIREWORKS_API_KEY` | `accounts/fireworks/models/deepseek-v4p1-flash` | `accounts/fireworks/models/deepseek-v4p1-flash` |
  | `baseten` | `BASETEN_API_KEY` | `deepseek-ai/DeepSeek-V4-Pro` | `deepseek-ai/DeepSeek-V4.1-Flash` |

  One provider does everything: answering, the memory work, and routing a message to its
  thread. `TOKIDOKI_MODEL` and `TOKIDOKI_ROUTER_MODEL` name other models of that provider.
  The environment is read once, at start: a changed provider, key or model takes a restart.
  A provider it doesn't know ends the process at once, with a message on stderr. Nothing else
  in the host's environment is read: `TOKIDOKI_USER`, `TOKIDOKI_SPEAKERS` and `TOKIDOKI_SYSTEM`
  are `chat`'s, not a host's.
- Without the provider's key the process still starts and its agents open asleep: `threads`,
  `history` and the memory methods answer from what is on disk, unfinished work waits for a
  start with a key, and `send` answers with an error such as `FIREWORKS_API_KEY is not set`.
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
          "provider":"anthropic","model":"claude-opus-5-5","has_key":true}}
{"id":12,"method":"send","space":"space-7","params":{"input":{...}}}    a request, in
{"id":12,"result":{...}}   or   {"id":12,"error":"..."}                 its answer, out
{"space":"space-7","event":{...}}                                       an event, out
```

An agent opens on the first request that names its space. Requests are answered as each
finishes, in any order. A space's events come in the order they happened, and each is an
`interface::Event` (`crates/interface/protocol.ts` in TypeScript). A line that isn't a
request is reported on stderr and skipped.

## Methods

In ternt's contract:

| method | params | result |
| --- | --- | --- |
| `send` | `{ "input": Input, "label": str }` | `Sent`, as soon as the input is placed. `label` is the space's name as the host calls it: threads started from then on are told it after the system prompt |
| `threads` | | `[ThreadSummary]`, most recently active first |
| `history` | `{ "thread": id, "since": n }` | `[Event]`: the thread's checkpoints from `n` |
| `abort` | `{ "thread": id }` | `true` if an answer was stopped |
| `close` | | `null`; the agent opens again on its next request |
| `shutdown` | | `null`, then the process ends |

Beyond it, for the host to use when it has somewhere to show them:

| method | params | result |
| --- | --- | --- |
| `memories` | | `[Memory]`, newest first |
| `remember` | `{ "text": str }` | the `Memory` it now has |
| `revise` | `{ "id": n, "text": str }` | the `Memory` that replaced it |
| `forget` | `{ "id": n }` | `null` |
| `feedback` | `{ "feedback": Feedback }` | `null`: "that was wrong", kept for debugging |
| `delete` | | `null`: the agent stopped and **all its files removed, for good** |

Memory changes arrive as `Event::Memory` whoever made them, so a host that never calls
these still sees what an agent learns.

## Where tokidoki and the contract don't quite meet

- **The provider.** The contract knows one key, Anthropic's. `TOKIDOKI_PROVIDER`, the two
  other keys, the model variables and the ready line's `provider`, `model` and `has_key`
  are tokidoki's additions.

- **One provider and key for every space.** The environment holds one, so a space can't
  have its own (a work account, say). That would take a key in the request that opens it.
- **Nothing flows the other way.** The process never asks the host for anything, so an
  agent can't yet act on its space (tabs, terminals). Tools will need requests from the
  process to the host, answered like the host's own.
- **Deleting a space.** The contract has no way to say a space is gone; `delete` above is
  what it would call.
