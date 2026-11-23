# Tokidoki (Ternt's copy)

Ternt's agent: durable execution and automatic long-term memory, in Rust, run as the sidecar
`tokidoki-serve`, one agent per Ternt space. **Forked into this repo on 2026-11-23** from
jonkarrer/tokidoki at `4d3b18b`; that repo is the owner's and grows on its own from here, so
nothing is synced either way. Develop this copy for what Ternt needs. `docs/HANDOFF.md` is the
long account of the design (its working rules predate the fork); `docs/SIDECAR.md` how the
sidecar behaves; the contract is `../docs/AGENT-SIDECAR.md`.

## Standing decisions

- **Scope: a chat with memory, and tools that need no browser** (web search first). No
  requests from the sidecar back to Ternt (tabs, pages, terminals), nothing shared between
  spaces, until the owner says so.
- **Bring your own key, macOS only.** Keys reach the process only through its environment
  (`ANTHROPIC_API_KEY`, `FIREWORKS_API_KEY`, `BASETEN_API_KEY`, whichever the host has). Never
  in a file, never in the repo. The sidecar reads nothing else from the host's environment
  but `TOKIDOKI_PROVIDER` (the default provider) and `TOKIDOKI_MODEL` / `TOKIDOKI_ROUTER_MODEL`.
- **Provider and tools are per space** (owner, 2026-11-23): the host says them with
  `configure`; keys stay per process.
- **Sidecar, not in-process.** A crash in an agent must never end the browser.
- When the contract moves, change `../src-tauri/src/agent` and this together; after
  `just types` here, `just sync-protocol` at the repo root copies the protocol over.

## Tests that call a model

Always on a very cheap model (the owner's rule): `claude-haiku-5-5`, `deepseek-v4p1-flash`,
`DeepSeek-V4.1-Flash`, which `Config::cheap_models` names and `TOKIDOKI_MODELS=cheap` selects.
A live test never names a provider's default model.

## Style

Every `lib.rs` opens with what the crate is for. Comments are plain prose saying why. Test
names are sentences (`the_sidecar_ends_when_its_input_does`). `cargo fmt` the files you touch
(edition 2024).

## Commands (from this directory)

```sh
just check                         # fmt, clippy -D warnings, protocol.ts check (also in the root's `just check`)
just test                          # all tests; `just test inference --live` hits real APIs
just types                         # regenerate crates/interface/protocol.ts
just serve                         # the sidecar here, type requests at it
```

From the repo root: `just sidecar tokidoki` builds this for Ternt, `just dev tokidoki` runs
Ternt with it, and `doppler run -- env TERNT_SIDECAR=tokidoki e2e/run.sh tokidoki` tests the pair.
