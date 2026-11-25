# Ternt

A macOS browser and terminal with a sidebar of spaces (Tauri v2 + Svelte 5). `README.md` is the front page and `docs/` the map (GUIDE, ARCHITECTURE, DEVELOPMENT); this
file is what isn't in the code.

## Standing decisions

- **No AI in Ternt** (owner, 2026-11-25). The agent (tokidoki, run as a sidecar process, with
  its panel, Settings page, setup window and keys) was removed after v0.23.0; it is in history
  at tag `v0.23.0`. `src-tauri/src/retired.rs` deletes what it left on a Mac (Keychain keys,
  the `agents/` folder) at launch. Don't add AI features back unless the owner asks.
- **Secrets** come from Doppler (`doppler run --project ternt --config dev -- …`) or the
  Keychain. Never print a key, never put one on a command line, never commit the updater key.
- **License and notices** (owner, 2026-11-25): source-available (`LICENSE`), shown in the app at
  `ternt://license/` (Help menu, welcome page). Dependencies' notices ship in
  `src-tauri/third-party-licenses.txt`: run `just licenses` when dependencies change.

## Working with the owner

Short requests, often "pull and deploy": pull `main`, judge patch or minor from the commits,
`just upload <bump>`, verify `latest.json`, report. Step by step otherwise. The release Mac is
the Mac mini with the signing keychain (`~/.tauri/ternt-sign.keychain.password` unlocks it).

## Checks before a commit

`just check` (the third-party notices are current, svelte-check, clippy), `bun run test:unit`,
and the e2e suites that touch what changed: `e2e/run.sh <suite>` (needs someone logged in at the
Mac's screen; over SSH alone every suite fails with no output).
`rustfmt --edition 2024 <files you touched>`, not `cargo fmt` on the tree.
