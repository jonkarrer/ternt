# Releasing Tokidoki

The app (`app/`) is released the way ternt is: built on a Mac that has an Apple **Developer
ID**, so it is signed and notarized and opens without warnings on any Mac. GitHub Releases
hosts the downloads and the update manifest; installed copies update themselves (see
`app/src-tauri/src/updater.rs`).

```sh
just release 0.2.0   # bump the app's version, commit, tag v0.2.0 (any machine)
just upload          # on the signing Mac: build, sign, notarize, push, publish
```

## Where releases live

The source repo is private, and a private repo's release files can't be downloaded without
credentials, which the updater doesn't send. So releases go to a **separate repo that holds
nothing but releases**, `jonkarrer/tokidoki-dist`, whose files anyone with the link can
download. It is named once, in the updater endpoint in `app/src-tauri/tauri.conf.json`:

```json
"endpoints": ["https://github.com/jonkarrer/tokidoki-dist/releases/latest/download/latest.json"]
```

`scripts/publish.sh` reads the repo from that line. The endpoint is built into every copy of
the app, so changing it later strands the copies already installed. The repo must be public
(a private repo's release files need credentials) and must have at least one commit before
GitHub lets a release be created in it.

The app is private in the sense that nobody is told where it is: the repo isn't linked from
anywhere and holds no source. Every update is signed with the updater key, so a copy only
ever installs what this project published.

## What `just upload` needs

| Needed | Used for | Where it comes from |
| --- | --- | --- |
| `TAURI_SIGNING_PRIVATE_KEY` (+`_PASSWORD`) | signs the update archive | env, else `~/.tauri/tokidoki.key` (+`.password`) |
| `GITHUB_ACCESS_TOKEN` or `GITHUB_TOKEN` | creates the release, uploads files | env, else keychain item `tokidoki-github` |
| `APPLE_CLI_UPLOAD_KEY_ID` + `_ISSUER_ID` | App Store Connect API key for notarization (the one ternt uses; the `.p8` is in `~/devjon/ids`) | env: required |
| a *Developer ID Application* certificate | signs the app | the keychain (ternt's signing keychain is unlocked if present) |

The Apple values are in ternt's Doppler project, so until Tokidoki has its own:

```sh
doppler run --project ternt --config dev -- just upload
```

**The updater key** (`~/.tauri/tokidoki.key`, its password beside it) signs every update;
installed copies only accept updates signed with it, and its public half is in
`tauri.conf.json`. Back it up somewhere that outlives this Mac (Doppler, as ternt's is).
**Never commit it**: losing it means installed copies can't update.

**The GitHub token** is a fine-grained token with *Contents: Read and write* on the releases
repo only. Store it without it touching shell history:

```sh
security add-generic-password -s tokidoki-github -a jonkarrer -w
```

Release binaries must link only system libraries, so `publish.sh` compiles and links C with
Apple's toolchain whatever shell it runs from, and checks the result. Both Rust targets are
needed:

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
```

## Each release

1. On `main`, with everything committed: `just upload`. HEAD isn't tagged yet, so it asks
   whether this is a patch, minor or major release (or `just upload minor`), bumps the
   version in `tauri.conf.json`, `package.json` and the app's `Cargo.toml`, commits and tags.
2. The same run checks the setup, builds a universal app, signs and notarizes it (a few
   minutes, waiting on Apple), verifies the signature and Gatekeeper assessment, signs the
   update archive, writes `latest.json`, pushes `main` and the tag to the source repo, and
   creates the release (a draft until every file is uploaded) with:
   - `Tokidoki_<version>_universal.dmg`: for new installs. This is the link to hand out.
   - `Tokidoki_<version>_universal.app.tar.gz`: the update installed copies download
   - `latest.json`: what installed copies check
3. Within a few seconds of launch, and every 15 minutes after, installed copies download the
   update in the background and install it when they quit; meanwhile a card in the sidebar
   says it's ready (*Restart now* installs it right away).

If something fails halfway, nothing is published until the last step. The bump commit and tag
stay local and the built, notarized files stay in `target/universal-apple-darwin/release/bundle/publish`:
fix the cause (deleting any leftover draft release) and run `just upload-resume`.

`just upload-dry-run` builds and verifies the tag at HEAD with no Apple signing and no upload.
