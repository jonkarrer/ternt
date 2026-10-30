# Releasing Ternt

Releases are built on a Mac that has an Apple **Developer ID**, so the app is signed and
notarized: it opens without warnings on any Mac. GitHub Releases hosts the downloads and the
update manifest; installed copies update themselves (see `src-tauri/src/updater.rs`).

```sh
just release 0.2.0   # bump the version, commit, tag v0.2.0 (any machine)
just upload          # on the signing Mac: build, sign, notarize, push, publish
```

Both run from `nix develop` (Bun, just, doppler). `just upload` runs `scripts/publish.sh`
under `doppler run --`, so the release secrets in the `ternt` Doppler project (`dev` config,
bound by `doppler setup --no-interactive`) reach the script as environment variables:

| Doppler secret                         | Used for                                           | Fallback if absent                        |
| -------------------------------------- | -------------------------------------------------- | ----------------------------------------- |
| `TAURI_SIGNING_PRIVATE_KEY` (+`_PASSWORD`) | signs the update archive                        | `~/.tauri/ternt.key` (+`.password`)       |
| `GITHUB_ACCESS_TOKEN`                  | creates the release, uploads files                 | keychain item `ternt-github`              |
| `APPLE_CLI_UPLOAD_KEY_ID` + `_ISSUER_ID` | App Store Connect API key for notarization (same key as bernt/lernt; the `.p8` is in `~/devjon/ids`) | none: required |

Add one with `doppler secrets set NAME` (it prompts, so the value stays out of shell history).

## One-time setup on the signing Mac

1. **Tools.** Xcode (or `xcode-select --install`), Rust via [rustup](https://rustup.rs), and
   `nix develop` in the repo (Bun, just, doppler; `doppler login` once). Release binaries
   must link only system libraries, so C compiling and linking must use Apple's toolchain, not a
   Nix one (which links `/nix/store` libraries other Macs don't have). The flake's shell takes
   care of that, even when entered from another Nix shell; `publish.sh` checks the result either
   way. Then:

   ```sh
   rustup target add aarch64-apple-darwin x86_64-apple-darwin
   ```

2. **GitHub token** (to create releases and upload files; `git push` keeps using your SSH key).
   On GitHub: Settings → Developer settings → Personal access tokens → **Fine-grained tokens** →
   Generate new token:
   - Repository access: **Only select repositories** → `jonkarrer/ternt`
   - Permissions → Repository permissions → **Contents: Read and write** (nothing else)
   - Expiration: your call; `publish.sh` says so when it has expired.

   Store it in the keychain (it prompts for the token, so it stays out of shell history):

   ```sh
   security add-generic-password -s ternt-github -a jonkarrer -w
   ```

   `publish.sh` reads it from there (or from `$GITHUB_TOKEN`) and checks it before building.

3. **Developer ID certificate.** In the keychain you need a *Developer ID Application*
   certificate (Xcode → Settings → Accounts → Manage Certificates → + → Developer ID
   Application). Headless Macs (SSH, remote agents) can't use the login keychain for signing:
   there, import the certificate and key from Doppler (`APPLE_DEVELOPER_ID_P12`, base64 PKCS12,
   and `APPLE_DEVELOPER_ID_P12_PASSWORD`) into a dedicated keychain, as this Mac does
   (`ternt-sign.keychain-db`, password in `~/.tauri/ternt-sign.keychain.password`):

   ```sh
   security create-keychain -p "<pw>" ~/Library/Keychains/ternt-sign.keychain-db
   security set-keychain-settings ~/Library/Keychains/ternt-sign.keychain-db   # never auto-lock
   security import ternt-devid.p12 -k ~/Library/Keychains/ternt-sign.keychain-db -P "<p12 pw>" -T /usr/bin/codesign
   security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "<pw>" ~/Library/Keychains/ternt-sign.keychain-db
   security list-keychains -d user -s ~/Library/Keychains/login.keychain-db ~/Library/Keychains/ternt-sign.keychain-db
   ```

   Check with:

   ```sh
   security find-identity -v -p codesigning   # should list "Developer ID Application: …"
   ```

4. **Notarization key.** The App Store Connect API key bernt and lernt upload with:
   `~/devjon/ids/AuthKey_<key id>.p8` on disk, key id and issuer id in Doppler
   (`APPLE_CLI_UPLOAD_KEY_ID`, `APPLE_CLI_UPLOAD_ISSUER_ID`). No Apple ID password is involved.

5. **Updater signing key.** It is in Doppler (`TAURI_SIGNING_PRIVATE_KEY` and its password), so
   `just upload` and `just test-update` need nothing else. Without Doppler, put the key in
   `~/.tauri/ternt.key` and `~/.tauri/ternt.key.password` with `chmod 600`. This key signs
   every update; installed copies only accept updates signed with it. **Never commit it**:
   losing it means existing installs can't update.

6. **Check it all**, without publishing anything:

   ```sh
   git tag v$(grep -m1 '"version"' src-tauri/tauri.conf.json | sed -E 's/.*"([0-9.]+)".*/\1/')
   just upload-dry-run            # builds and verifies, skips Apple signing and the upload
   git tag -d v0.1.0              # (or whichever version it printed)
   ```

## Each release

1. On `main`, with everything committed: `just upload`. HEAD isn't tagged yet, so it asks
   whether this is a patch, minor or major release (or take the choice up front:
   `just upload minor`), bumps the version in `tauri.conf.json`, `package.json` and
   `Cargo.toml`, commits and tags. To pick an exact version instead, run
   `just release 0.2.0` first; `just upload` then publishes the tag at HEAD without asking.
2. The same run then checks the setup, builds a universal app, signs and notarizes it
   (a few minutes, waiting on Apple), verifies the signature and Gatekeeper assessment, signs the
   update archive, writes `latest.json`, pushes `main` and the tag, and creates the GitHub Release (a draft until every file is
   uploaded, so installed copies never see a half-published update)
   with:
   - `Ternt_<version>_universal.dmg`: for new installs
   - `Ternt_<version>_universal.app.tar.gz`: the update installed copies download
   - `latest.json`: what installed copies check, at
     `https://github.com/jonkarrer/ternt/releases/latest/download/latest.json`
3. Within a few seconds of launch (or via *Ternt → Check for Updates…*), installed copies show
   an **Update** button at the bottom left of the sidebar.

If something fails halfway, nothing is published until the last step. The bump commit and tag
stay local, and the built, notarized files stay in the bundle's `publish` folder: fix the cause
(deleting any leftover draft release on GitHub) and run `just upload-resume`, which publishes
those files without rebuilding or notarizing again. `just upload` instead rebuilds from the
tag at HEAD. To redo a published release, delete it on GitHub first.

## Testing the updater without publishing

`just test-update` (any Mac with Doppler access or the updater key) builds 0.1.0 and 0.1.1 test bundles,
installs 0.1.0, serves 0.1.1 from a local server and checks the full update: button, download,
swap, relaunch as 0.1.1.
