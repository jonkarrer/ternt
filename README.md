# tokidoki

A personal assistant that remembers: an agent with durable execution and automatic long-term
memory, as a Rust workspace, and a macOS app that packages it.

```bash
nix develop --impure   # dev shell (from ~/devjon/configs): just, jq, bun, tursodb, turso
just                   # list the recipes
just chat              # talk to a fresh tokidoki in the terminal
just app               # run the app with hot reload
```

The flake doesn't provide Rust; bring your own toolchain (e.g. rustup). `just chat`, `just serve`
and the live tests read `TOKIDOKI_PROVIDER` (`anthropic`, `fireworks` or `baseten`) and that
provider's key (`ANTHROPIC_API_KEY`, `FIREWORKS_API_KEY`, `BASETEN_API_KEY`) from the environment
or `.env`; the live tests of each provider need its key.

## Layout

- `crates/`: the system. Each crate's `src/lib.rs` opens with what it is for. `tokidoki` puts
  them together as one library (input in, output out) and `interface` is its protocol, as
  plain serde types.
- `app/`: the macOS app, Tauri v2 + Svelte 5. `app/src-tauri` is a workspace member that runs
  the `tokidoki` library in-process; the page calls one command per library method and
  follows the same `interface` events any other client would.
- `scripts/`, `docs/RELEASING.md`: building, signing, notarizing and publishing the app.
- `docs/SIDECAR.md`: tokidoki as a process another program runs (`tokidoki-serve`), one
  agent per space. This is how ternt hosts it.

Shared dependency versions go in the root `Cargo.toml` under `[workspace.dependencies]`.

## The app

It runs on the user's own API keys, asked for at first launch and kept in the macOS Keychain:
an Anthropic key (required) and a Baseten key (optional: the router's small fast model, for
filing a message into the right conversation when the rules can't decide). Its data lives in
`~/Library/Application Support/com.jokar.tokidoki` (`…/dev` for `just app`); point
`just inspect --data <dir>` at it to read what the agent did.

Closing the window leaves it running, so idle conversations still dream; quitting stops it,
and the next launch resumes whatever was cut off.
