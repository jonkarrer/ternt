set dotenv-load

# List the recipes.
default:
    @just --list

# Build every crate.
build:
    cargo build --workspace

# Run tests: `just test`, `just test inference`, or `just test inference --live` to hit real APIs.
[arg("live", long, value="true")]
test crate="" live="":
    cargo test {{ if crate == "" { "--workspace" } else { "-p " + crate } }} {{ if live == "true" { "-- --ignored --nocapture" } else { "" } }}

# Format, then lint with warnings as errors; type-check the app's page.
check:
    cargo fmt --all
    cargo clippy --workspace --all-targets -- -D warnings
    cargo test -q -p interface --features ts
    cd app && bun install --frozen-lockfile && bun run check

# Regenerate crates/interface/protocol.ts, the protocol as TypeScript, from the Rust types.
types:
    TOKIDOKI_WRITE_TYPES=1 cargo test -q -p interface --features ts

# Open a local database in Turso's shell, e.g. `just db data/tokidoki.db`.
db path="data/tokidoki.db":
    mkdir -p "$(dirname {{path}})"
    tursodb {{path}}

# Read what the agent did, e.g. `just inspect recent` or `just inspect search vegetarian`.
inspect *args:
    cargo run -q -p telemetry --bin inspect -- {{args}}

# Talk to a fresh tokidoki in the terminal; its data is deleted on exit unless `--keep`.
chat *args:
    cargo run -q -p tokidoki --bin chat -- {{args}}

# Run tokidoki as a sidecar process, speaking docs/SIDECAR.md on stdin and stdout.
serve data="data/spaces":
    cargo run -q -p tokidoki --bin tokidoki-serve -- --data {{data}}

# ─── The app (app/, Tauri + Svelte) ──────────────────────

# Run the app with hot reload. Its data is apart from the installed app's (…/com.jokar.tokidoki/dev).
app:
    cd app && bun install && bun tauri dev

# Release bundle for this machine's architecture, signed ad hoc, no update archive: target/release/bundle/macos/Tokidoki.app
app-build:
    cd app && bun install && bun tauri build --config '{"bundle":{"createUpdaterArtifacts":false}}'

# ─── Release (docs/RELEASING.md) ─────────────────────────

# Bump the app's version everywhere, commit and tag: `just release minor` (or patch, major, 0.2.0)
release version:
    scripts/release.sh {{version}}

# Release: bump (asks patch/minor/major unless given: `just upload minor`), build, sign, notarize, upload
upload *args:
    scripts/publish.sh {{args}}

# Publish the files the last `just upload` built, after a push or upload error (no rebuild)
upload-resume:
    scripts/publish.sh --resume

# Build and verify the release tagged at HEAD without Apple signing or upload
upload-dry-run:
    scripts/publish.sh --dry-run
