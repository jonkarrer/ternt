# tokidoki

A Rust workspace. Crates go in `crates/` (then set `members = ["crates/*"]`); shared dependency versions go in the root `Cargo.toml` under `[workspace.dependencies]`.

```bash
nix develop --impure   # dev shell (from ~/devjon/configs): just, jq, tursodb (Turso's Rust engine), turso (Turso Cloud CLI)
just                   # list the recipes: build, test, check, db
```

The flake doesn't provide Rust; bring your own toolchain (e.g. rustup).
