# Tokidoki

A personal-assistant agent in Rust: durable execution and automatic long-term memory. It ships
as `tokidoki-serve`, the agent sidecar of **ternt** (`~/devjon/projects/ternt`), one agent per
ternt space. `docs/HANDOFF.md` is the long account of the design; `docs/SIDECAR.md` how the
sidecar behaves; the contract between the two is ternt's `docs/AGENT-SIDECAR.md`.

## Standing decisions

- **Scope: a chat with memory.** No requests from the sidecar back to ternt (tabs, pages,
  terminals), no key per space, nothing shared between spaces, until the owner says so.
- **Bring your own key, macOS only.** Keys reach the process only through its environment:
  `TOKIDOKI_PROVIDER` (`fireworks`, ternt's default; `anthropic`; `baseten`) and that
  provider's key. Never in a file, never in the repo.
- **Sidecar, not in-process.** A crash in an agent must never end the browser.
- Both repos are developed by the same agent since 2026-11-21: change ternt and tokidoki
  together when the contract moves, and keep `protocol.ts` copied into ternt
  (`src/agent/protocol.ts`) after `just types`.

## Working with the owner

Step by step: one piece, reported, then the next. Expect a review and a simplify pass after a
chunk of work. Secrets come from Doppler (`doppler run --project ternt --config dev -- …`) or
the Keychain; never print a key or put one on a command line.

## Style

Every `lib.rs` opens with what the crate is for. Comments are plain prose saying why. Test
names are sentences (`the_sidecar_ends_when_its_input_does`). `cargo fmt` the files you touch
(edition 2024).

## Commands

```sh
just check                         # fmt, clippy -D warnings, protocol.ts check, app type-check
just test                          # all tests; `just test inference --live` hits real APIs
just types                         # regenerate crates/interface/protocol.ts
just serve                         # the sidecar here, type requests at it
cargo build -p tokidoki --bin tokidoki-serve
```

Ternt builds this working tree with `scripts/sidecar.sh tokidoki` and tests the pair with
`doppler run -- env TERNT_SIDECAR=tokidoki e2e/run.sh tokidoki` (in the ternt repo).
