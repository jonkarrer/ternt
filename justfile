# Ternt — Arc-style browser and terminal for macOS (Tauri v2 + Svelte 5)

# Default: list available commands
default:
    @just --list

# ─── Core ────────────────────────────────────────────────

# Run the app with hot reload (the agent sidecar first: tauri dev only builds Ternt). `just dev tokidoki` runs the real agent
dev flavor="": (sidecar flavor)
    bun tauri dev

# Build the agent sidecar into src-tauri/binaries; Ternt needs it to compile. `stub` (default) or `tokidoki` (from tokidoki/)
sidecar *args:
    scripts/sidecar.sh {{args}}

# Copy tokidoki's protocol (Rust and TypeScript) from tokidoki/ into the host code after it changes
sync-protocol:
    scripts/sync-protocol.sh

# Typecheck + lint — must be clean before commit (Ternt, then the agent in tokidoki/)
check: sidecar
    bun run check && cd src-tauri && cargo clippy -q --workspace --all-targets -- -D warnings
    cd tokidoki && just check

# Unit tests (Ternt's, then the agent's); `just test-e2e` for the rest
test:
    bun run test:unit
    cd tokidoki && just test

# Format the tree (Rust; the Svelte/TS side has no formatter configured yet)
fmt:
    cd src-tauri && cargo fmt

# Release bundle for this machine's architecture (judge performance here, not in dev); the sidecar in release too
build: (sidecar "--release")
    bun tauri build

# ─── Tests that need a desktop session ───────────────────

# End-to-end suites (e2e/run.sh); `just test-e2e palette` runs one
test-e2e suite="":
    e2e/run.sh {{suite}}

# Self-update test: installs 0.1.0, serves 0.1.1–0.1.3 locally, checks each way an update lands (needs the updater key)
test-update:
    doppler run -- e2e/update.sh

# ─── Release (docs/RELEASING.md) ─────────────────────────

# Bump the version everywhere, commit and tag: `just release minor` (or patch, major, 0.2.0)
release version:
    scripts/release.sh {{version}}

# Release: bump (asks patch/minor/major unless given: `just upload minor`), build, sign, notarize, upload
# Add --required for a release installed copies take right away (`just upload patch --required`)
upload *args:
    doppler run -- scripts/publish.sh {{args}}

# Publish the files the last `just upload` built, after a push or upload error (no rebuild)
upload-resume *args:
    doppler run -- scripts/publish.sh --resume {{args}}

# Build and verify the release tagged at HEAD without Apple signing or upload
upload-dry-run:
    doppler run -- scripts/publish.sh --dry-run
