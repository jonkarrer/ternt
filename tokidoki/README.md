# tokidoki (Ternt's copy)

A personal assistant that remembers: an agent with durable execution and automatic long-term
memory, as a Rust workspace. It ships as `tokidoki-serve`, Ternt's agent sidecar: one agent per
space. Forked into Ternt's repo on 2026-11-23 from
[jonkarrer/tokidoki](https://github.com/jonkarrer/tokidoki) (`4d3b18b`), which grows on its own
from there; this copy is developed for Ternt.

```bash
just                   # list the recipes (Ternt's dev shell has what they need)
just chat              # talk to a fresh tokidoki in the terminal
just serve             # run the sidecar here and type requests at it
```

`just chat`, `just serve` and the live tests read the provider keys (`ANTHROPIC_API_KEY`,
`FIREWORKS_API_KEY`, `BASETEN_API_KEY`) from the environment or `.env`, and `TOKIDOKI_PROVIDER`
(`anthropic`, `fireworks` or `baseten`) for the default provider; the live tests of each provider
need its key.

## Layout

- `crates/`: the system. Each crate's `src/lib.rs` opens with what it is for. `tokidoki` puts
  them together as one library (input in, output out) and `interface` is its protocol, as
  plain serde types.
- `docs/SIDECAR.md`: tokidoki as a process another program runs (`tokidoki-serve`), one
  agent per space. This is how Ternt hosts it (`../scripts/sidecar.sh` builds it, releases
  bundle it); the contract is `../docs/AGENT-SIDECAR.md`.

Shared dependency versions go in the root `Cargo.toml` under `[workspace.dependencies]`.
