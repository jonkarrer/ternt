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
search through Exa first. The plumbing is in place (`Setup.tools`, `configure`, the `tools`
map in the ready line); no tool exists yet, so any tool name is refused as unknown.

**Blocked on a key for the live parts:** `EXA_API_KEY` is not in Doppler (`ternt` / `dev`).
The owner was asked and has not answered. The tool can be built against a mocked test first.

Step 3, the tool, in `tokidoki/`:

- A new crate with one tool, `web_search`: Exa `/search` with highlights; arguments `query`,
  `num_results` (at most 10), optional recency; 15 second timeout; safe to replay, since
  tokidoki resumes interrupted work.
- Register it in `tools(setup)` in `bin/serve.rs`; read the key in `Keys::from_env()`.
- Ready line reports `tools: {web_search: <key present>}`.
- A live test gated on the key, on a cheap model.

Step 4, Settings:

- Tools rows with a switch in the Space card, enabled only when the Exa key is present.
- An Exa row under Keys: Keychain account `exa`, environment variable `EXA_API_KEY`, added to
  `keys::load()`.
- e2e in `settings.js` against the stub, then the real-agent suite.

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
