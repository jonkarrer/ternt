# Pickup

Where the agent work stands as of 2026-11-23 (`main` at `dc89c11` before this file). Read
`CLAUDE.md` first for the standing decisions; this is the state of play and what is next.

## What just landed

1. **Settings → Agent → Memory** (`b595757`). Per space: the facts the agent has stored, each
   editable and forgettable, and a Wipe that clears the space's memory to a blank slate.
   Episodic and procedural memory are not shown. Sidecar methods `memories`, `revise`,
   `forget`, `wipe`.
2. **Provider and tools per space** (`4d3b18b`, `b3825e8`). Each space carries
   `Space::agent: Setup { provider, tools }`, saved with the session. Ternt sends it to the
   sidecar as `configure` before a space's first request in a process and whenever Settings
   changes it. A changed setup reopens that space's agent in place; the process does not
   restart and the conversation carries on. Keys are global: Keychain accounts `fireworks`
   and `anthropic`, all handed to the sidecar through its environment. Saving or removing a
   key restarts the process; changing a provider does not.
3. **Tokidoki forked into this repo** (`a896ff3`, `dc89c11`). `tokidoki/` is a git subtree of
   jonkarrer/tokidoki at `4d3b18b`, history included. It is Ternt's agent from here on. The
   standalone repo is the owner's separate project; nothing syncs and it is not edited for
   Ternt work. `scripts/sidecar.sh` builds from `tokidoki/` (`TOKIDOKI_SRC` overrides),
   `just sync-protocol` reads from it, and the root `just check` / `just test` run its checks
   and tests too.

## Where things are

| What | Where |
| --- | --- |
| The agent (own Cargo workspace, own CLAUDE.md) | `tokidoki/` |
| Sidecar protocol and its doc | `tokidoki/crates/tokidoki/src/serve.rs`, `tokidoki/docs/SIDECAR.md` |
| Sidecar binary: keys, tools registry, ready line | `tokidoki/crates/tokidoki/src/bin/serve.rs` |
| Per-space setups | `tokidoki/crates/tokidoki/src/spaces.rs` |
| Host side: supervisor, commands, keys, setup types | `src-tauri/src/agent/` (`sidecar.rs`, `mod.rs`, `keys.rs`, `setup.rs`) |
| The stub sidecar used by e2e | `src-tauri/agent/` |
| Contract between the two | `docs/AGENT-SIDECAR.md` |
| Settings page | `src/settings/Agent.svelte` |
| Chat panel | `src/agent/Agent.svelte` |
| e2e | `e2e/tests/settings.js`, `e2e/tests/agent.js`, `e2e/tests/tokidoki.js` |

The ready line is `{backend, version, providers: {anthropic, fireworks, baseten},
default_provider, tools: {}}`. `configure` takes `{provider?, tools?}` and answers
`{provider, model, tools, has_key, asleep}`.

## Next: tools, starting with Exa web search

The owner asked for basic tools that can be toggled in Settings → Agent, per space, with web
search through Exa first. `EXA_API_KEY` is in Doppler (`ternt` / `dev`).

Step 3, the tool, is **built** (`6ddf170`), all in `tokidoki/`:

- `crates/tools`: `web_search`, Exa `/search` with highlights; `query`, `num_results` (at
  most 10), optional `days`; 15 second timeout; replay-safe. Mocked tests, and a live one
  (`doppler run --project ternt --config dev -- just test tools --live`, no model called).
- `bin/serve.rs` registers it for a space that asks, when the key is there; the ready line
  says `tools: {web_search: <key present>}`. A tool whose key is missing is left out rather
  than failing the space.
- A thread now offers the tools its agent has at each message (`NewThreads::set_up`), so a
  switch reaches conversations already going. Calls of a tool no longer offered are sent to
  the model as plain text (`Context::retell_unoffered_calls`); without that Anthropic
  answered with nothing.
- Checked by driving the sidecar on the cheapest models, both providers: off, on, off.

Step 4, Settings, is **built** (2026-11-23):

- Settings → Agent → Space has a Web search switch per space; it saves `Space::agent.tools`
  and sends `configure`. It can be switched on only when the Exa key is there (the stub needs
  none), and one left on without a key can still be switched off.
- Keys has an Exa row: Keychain account `exa`, variable `EXA_API_KEY`, in `keys::load()`.
- e2e: `settings.js` (switch on, saved per space, no restart, off) and `tokidoki.js` (a real
  search in the panel on a cheap model, shown as a tool row, answer citing the page).

More tools go in `tokidoki/crates/tools`, `Keys::tools_for` in `bin/serve.rs`, and the
`TOOLS` list in `src/settings/Agent.svelte`.

## Also open

- **Performance.** The owner asked for gains but did not say what feels slow. Measured on a
  release build: launch about 190 ms, Ternt about 115 MB resident with WebKit helpers adding
  175 to 220 MB, a sidebar update under 1 ms, an agent turn 868 ms of which 860 ms is the
  model. Nothing stood out; needs a target from the owner.
- **No release has been cut** with any of the above. `just upload <bump>` on the release Mac
  would ship tokidoki built from `tokidoki/`. A local `just build` stops at updater signing
  without the signing key, which is expected.

## Running things

```sh
just check && just test          # both workspaces
bun run test:unit
e2e/run.sh settings              # needs a GUI session
e2e/run.sh agent                 # against the stub
doppler run --project ternt --config dev -- env TERNT_SIDECAR=tokidoki e2e/run.sh tokidoki
```

Pipe `just check` to a log and read its exit code; piping it to `tail` hides a failure.
Last full pass: settings and agent suites 94 checks, real-agent suite 26 checks, all green.

## Rules that are easy to forget

- Any test that calls a model uses a cheap one: `TOKIDOKI_MODELS=cheap`, set by
  `e2e/tests/tokidoki.env`. Never a provider's default.
- Keys come from Doppler or the Keychain. Never printed, logged, written to disk or put on a
  command line.
- `rustfmt --edition 2024` on the files touched, not `cargo fmt` on the tree.
- The owner works step by step: one piece, reported, then the next, with a review and
  simplify pass after a chunk. Commit when asked.
