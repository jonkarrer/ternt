# tokidoki

A personal assistant that remembers: an agent with durable execution and automatic long-term
memory, as a Rust workspace. It ships as `tokidoki-serve`, the agent sidecar of
[ternt](https://github.com/jonkarrer/ternt): one agent per ternt space.

```bash
nix develop --impure   # dev shell (from ~/devjon/configs): just, jq, tursodb, turso
just                   # list the recipes
just chat              # talk to a fresh tokidoki in the terminal
just serve             # run the sidecar here and type requests at it
```

The flake doesn't provide Rust; bring your own toolchain (e.g. rustup). `just chat`, `just serve`
and the live tests read `TOKIDOKI_PROVIDER` (`anthropic`, `fireworks` or `baseten`) and that
provider's key (`ANTHROPIC_API_KEY`, `FIREWORKS_API_KEY`, `BASETEN_API_KEY`) from the environment
or `.env`; the live tests of each provider need its key.

## Layout

- `crates/`: the system. Each crate's `src/lib.rs` opens with what it is for. `tokidoki` puts
  them together as one library (input in, output out) and `interface` is its protocol, as
  plain serde types.
- `docs/SIDECAR.md`: tokidoki as a process another program runs (`tokidoki-serve`), one
  agent per space. This is how ternt hosts it; ternt's repo builds, bundles and releases it.

Shared dependency versions go in the root `Cargo.toml` under `[workspace.dependencies]`.
