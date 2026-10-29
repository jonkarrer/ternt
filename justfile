# Ternt — Arc-style browser and terminal for macOS (Tauri v2 + Svelte 5)

# Default: list available commands
default:
    @just --list

# ─── Core ────────────────────────────────────────────────

# Run the app with hot reload
dev:
    bun tauri dev

# Typecheck + lint — must be clean before commit
check:
    bun run check && cd src-tauri && cargo clippy -q --all-targets -- -D warnings

# Unit tests, type check and clippy (about a second); `just test-e2e` for the rest
test:
    bun run test:unit

# Format the tree (Rust; the Svelte/TS side has no formatter configured yet)
fmt:
    cd src-tauri && cargo fmt

# Release bundle for this machine's architecture (judge performance here, not in dev)
build:
    bun tauri build

# ─── Tests that need a desktop session ───────────────────

# End-to-end suites (e2e/run.sh); `just test-e2e palette` runs one
test-e2e suite="":
    e2e/run.sh {{suite}}

# Self-update test: installs 0.1.0, serves 0.1.1 locally, checks the swap (needs the updater key)
test-update:
    doppler run -- e2e/update.sh

# ─── Release (docs/RELEASING.md) ─────────────────────────

# Bump the version everywhere, commit and tag, e.g. `just release 0.2.0`
release version:
    scripts/release.sh {{version}}

# Build, sign, notarize and upload the tagged release to GitHub Releases
upload:
    doppler run -- scripts/publish.sh

# Build and verify the tagged release without Apple signing or upload
upload-dry-run:
    doppler run -- scripts/publish.sh --dry-run
