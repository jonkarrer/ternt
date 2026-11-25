# Development

Building, testing and releasing Ternt.

## Setup

```sh
nix develop                # Bun, just, doppler (Rust comes from rustup; Xcode does C/linking)
bun install
just                       # lists every recipe
just dev                   # run the app
just build                 # release bundle (judge performance here, not in dev)
just check                 # type-check Svelte/TS, clippy
just test                  # unit tests, type check, clippy (about a second)
just test-e2e palette      # one e2e suite (VERBOSE=1 lists every check); no name runs all
```

The flake follows the org Nix standard, with the `mkDevShell` builder vendored into `nix/devshell.nix` so the build never depends on the state of the configs checkout.
Secrets live in the `ternt` Doppler project (`doppler.yaml` binds `dev`); recipes that need
them run under `doppler run --`.

After adding, removing or updating a dependency (Cargo or npm), run `just licenses`: it rewrites
`src-tauri/third-party-licenses.txt`, the notices *Help → Third-Party Licenses* shows, from what
the release links (`scripts/licenses.ts`). `just check` fails while that file is out of date.

`ternt <url>...` opens URLs as tabs (`bun tauri dev -- -- https://example.com`).

## Tests

`bun run test` runs, in order:

- **TypeScript unit tests** (`bun test`, `src/**/*.test.ts`): address parsing (`src/lib/url.ts`)
  and command bar ranking (`src/palette/rank.ts`, pure: data in, items with actions out).
- **Type check** (`svelte-check`).
- **Rust unit tests** (`cargo test`): the tab/space/folder model and profiles (`model.rs`),
  terminal pane trees (`panes.rs`), history ranking and storage (`history.rs`), profile ids
  (`browser.rs`). Then clippy with warnings as errors.
- **End-to-end** (`e2e/run.sh`, macOS): launches the debug build once per suite in
  `e2e/tests/` with `TERNT_TEST=<script>`. The script runs inside the sidebar webview and drives
  the app through its real commands and DOM events (`t` = `window.__ternt`, see
  `src/lib/debug.ts`; native checks like keyboard focus and the clipboard are in
  `src-tauri/src/debug.rs`; both are compiled out of release builds). The app stays in the background
  during test runs (it doesn't activate on launch or when it focuses a window), so you can keep working. Fixture pages are served
  from `e2e/fixtures` on port 8765. Suites named `x-1`, `x-2` share a session and history file,
  so `x-2` tests what survives a restart. Logs and screenshots land in `e2e/.out/`; a failed
  screenshot only warns.
- **Self-update** (`e2e/update.sh`, a few minutes, not part of `bun run test`): builds signed
  0.1.0 and 0.1.1 bundles, installs 0.1.0, serves 0.1.1 as the latest release from a local
  server, then checks that the installed app pops up the update card, hides it on *Later*,
  installs, swaps its bundle and relaunches as 0.1.1. Needs a signing key in `~/.tauri/`
  (any key: the test bundles trust its public half).

Not covered (macOS won't let tests send real input): physical key presses and trackpad
gestures, native context menus opening (their actions are tested), and real OAuth providers.

## Releases

```sh
just upload                # on the signing Mac: asks patch/minor/major, bumps, tags, builds,
                           # signs, notarizes, pushes, publishes (`just upload minor` skips the question)
just upload-resume         # publish what the last run built, after a push/upload error
```

Releases are built on a Mac with an Apple Developer ID, signed and notarized, and published to
GitHub Releases: the `.dmg` for new installs, plus the signed update archive and `latest.json`
that installed copies check shortly after launch, every 15 minutes and when you come back to the app
(`src-tauri/src/updater.rs`). Updates are verified against the public key in
`tauri.conf.json`; the private key lives in Doppler (`ternt` / `dev`, `TAURI_SIGNING_PRIVATE_KEY`) and in
`~/.tauri/ternt.key` (without it, installed copies can't be updated). Setup and details: [RELEASING.md](RELEASING.md).

CI (`.github/workflows/ci.yml`) runs the unit tests on every push.

CI (`.github/workflows/ci.yml`) runs the unit tests on every push.
