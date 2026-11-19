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

# Format, then lint with warnings as errors.
check:
    cargo fmt --all
    cargo clippy --workspace --all-targets -- -D warnings

# Open a local database in Turso's shell, e.g. `just db data/tokidoki.db`.
db path="data/tokidoki.db":
    mkdir -p "$(dirname {{path}})"
    tursodb {{path}}
