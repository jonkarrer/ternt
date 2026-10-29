# Releasing Ternt

Releases are built on a Mac that has an Apple **Developer ID**, so the app is signed and
notarized: it opens without warnings on any Mac. GitHub Releases hosts the downloads and the
update manifest; installed copies update themselves (see `src-tauri/src/updater.rs`).

```sh
just release 0.2.0   # bump the version, commit, tag v0.2.0 (any machine)
just publish         # on the signing Mac: build, sign, notarize, push, publish
```

Both run from `nix develop` (Bun, just, doppler). `just publish` runs `scripts/publish.sh`
under `doppler run --`, so the release secrets in the `ternt` Doppler project (`dev` config,
bound by `doppler setup --no-interactive`) reach the script as environment variables:

| Doppler secret                         | Used for                                           | Fallback if absent                        |
| -------------------------------------- | -------------------------------------------------- | ----------------------------------------- |
| `TAURI_SIGNING_PRIVATE_KEY` (+`_PASSWORD`) | signs the update archive                        | `~/.tauri/ternt.key` (+`.password`)       |
| `GITHUB_ACCESS_TOKEN`                  | creates the release, uploads files                 | keychain item `ternt-github`              |
| `APPLE_ID`                             | Apple developer account email, for notarization    | none: required                            |
| `APPLE_PASSWORD`                       | app-specific password, for notarization            | keychain item `ternt-notary`              |

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
   Application). Check with:

   ```sh
   security find-identity -v -p codesigning   # should list "Developer ID Application: …"
   ```

4. **Notarization password.** Create an app-specific password at
   [account.apple.com](https://account.apple.com) → Sign-In and Security → App-Specific Passwords,
   then store it in the keychain (it prompts for the password, so it stays out of shell history):

   ```sh
   security add-generic-password -s ternt-notary -a "you@example.com" -w
   ```

   and set your Apple ID for the scripts, e.g. in `~/.zshrc`:

   ```sh
   export APPLE_ID="you@example.com"
   ```

5. **Updater signing key.** It is in Doppler (`TAURI_SIGNING_PRIVATE_KEY` and its password), so
   `just publish` and `just test-update` need nothing else. Without Doppler, put the key in
   `~/.tauri/ternt.key` and `~/.tauri/ternt.key.password` with `chmod 600`. This key signs
   every update; installed copies only accept updates signed with it. **Never commit it**:
   losing it means existing installs can't update.

6. **Check it all**, without publishing anything:

   ```sh
   git tag v$(grep -m1 '"version"' src-tauri/tauri.conf.json | sed -E 's/.*"([0-9.]+)".*/\1/')
   just publish-dry-run           # builds and verifies, skips Apple signing and the upload
   git tag -d v0.1.0              # (or whichever version it printed)
   ```

## Each release

1. On `main`, with everything committed and pushed: `just release <version>`
   (e.g. `0.2.0`). It bumps the version in `tauri.conf.json`, `package.json` and `Cargo.toml`,
   commits and tags. For the very first release the version is already `0.1.0`: just
   `git tag -a v0.1.0 -m "Ternt 0.1.0"`.
2. `just publish`. It checks the setup, builds a universal app, signs and notarizes it
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

If something fails halfway, nothing is published until the last step; fix it (deleting any
leftover draft release on GitHub) and run `just publish` again. To redo a published release, delete it on GitHub first.

## Testing the updater without publishing

`just test-update` (any Mac with Doppler access or the updater key) builds 0.1.0 and 0.1.1 test bundles,
installs 0.1.0, serves 0.1.1 from a local server and checks the full update: button, download,
swap, relaunch as 0.1.1.
