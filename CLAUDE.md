# Ternt

An Arc-style macOS browser and terminal (Tauri v2 + Svelte 5). The README is the map; this
file is what isn't in the code.

## Standing decisions

- **The agent** (⌘J, `src-tauri/src/agent/`) is tokidoki, run as a sidecar process; the repo
  is `~/devjon/projects/tokidoki` and the contract is `docs/AGENT-SIDECAR.md` here. Both
  repos are developed by the same agent since 2026-11-21: when the contract moves, change
  both, then `just sync-protocol` here copies tokidoki's protocol (Rust and TypeScript) over.
- **Scope of the agent: a chat with memory, plus tools that need no browser** (web search
  first). No requests from the sidecar back to Ternt until the owner says so.
- **Bring your own key; provider and tools per space** (owner, 2026-11-23). Settings → Agent →
  Keys keeps the Fireworks AI and Anthropic keys in the Keychain; the sidecar gets every key
  through its environment and nothing else does. Each space picks its provider (Fireworks
  default) and tools, kept with the space and sent as `configure`; no key per space.
  Development uses the team keys: `doppler run -- just dev tokidoki`.
- **Secrets** come from Doppler (`doppler run --project ternt --config dev -- …`) or the
  Keychain. Never print a key, never put one on a command line, never commit the updater key.

## Working with the owner

Short requests, often "pull and deploy": pull `main`, judge patch or minor from the commits,
`just upload <bump>`, verify `latest.json`, report. Step by step otherwise. The release Mac is
the Mac mini with the signing keychain (`~/.tauri/ternt-sign.keychain.password` unlocks it).

## Tests that call a model

Always on a very cheap model (the owner's rule): the real-agent suite runs the sidecar with
`TOKIDOKI_MODELS=cheap` (`e2e/tests/tokidoki.env`), which picks each provider's cheapest model.
Never run a test against a provider's default model.

## Checks before a commit

`just check` (sidecar build, svelte-check, clippy on the workspace), `bun run test:unit`, and
the e2e suites that touch what changed: `e2e/run.sh <suite>` (needs a GUI session). For the
agent: `e2e/run.sh agent` against the stub, and
`doppler run -- env TERNT_SIDECAR=tokidoki e2e/run.sh tokidoki` against the real one.
`rustfmt --edition 2024 <files you touched>`, not `cargo fmt` on the tree.
