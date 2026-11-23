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

# Format, then lint with warnings as errors; check the TypeScript protocol is current.
check:
    cargo fmt --all
    cargo clippy --workspace --all-targets -- -D warnings
    cargo test -q -p interface --features ts

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
