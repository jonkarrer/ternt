# Releasing Ternt

Releases are built on a Mac that has an Apple **Developer ID**, so the app is signed and
notarized: it opens without warnings on any Mac. GitHub Releases hosts the downloads and the
update manifest; installed copies update themselves (see `src-tauri/src/updater.rs`).

```sh
scripts/release.sh 0.2.0   # bump the version, commit, tag v0.2.0 (any machine)
scripts/publish.sh         # on the signing Mac: build, sign, notarize, push, publish
```

## One-time setup on the signing Mac

1. **Tools.** Xcode (or `xcode-select --install`), Rust via [rustup](https://rustup.rs), and
   either `nix develop` in the repo (the flake provides Bun and the GitHub CLI) or
   [Bun](https://bun.sh) and `brew install gh`. Release binaries must link only system libraries,
   so C compiling and linking must use Apple's toolchain, not a Nix one (which links
   `/nix/store` libraries other Macs don't have). The flake's shell takes care of that, even when
   entered from another Nix shell; `publish.sh` checks the result either way. Then:

   ```sh
   rustup target add aarch64-apple-darwin x86_64-apple-darwin
   gh auth login
   ```

2. **Developer ID certificate.** In the keychain you need a *Developer ID Application*
   certificate (Xcode → Settings → Accounts → Manage Certificates → + → Developer ID
   Application). Check with:

   ```sh
   security find-identity -v -p codesigning   # should list "Developer ID Application: …"
   ```

3. **Notarization password.** Create an app-specific password at
   [account.apple.com](https://account.apple.com) → Sign-In and Security → App-Specific Passwords,
   then store it in the keychain (it prompts for the password, so it stays out of shell history):

   ```sh
   security add-generic-password -s ternt-notary -a "you@example.com" -w
   ```

   and set your Apple ID for the scripts, e.g. in `~/.zshrc`:

   ```sh
   export APPLE_ID="you@example.com"
   ```

4. **Updater signing key.** Copy `~/.tauri/ternt.key` and `~/.tauri/ternt.key.password` from the
   machine that generated them (AirDrop, or your password manager), into `~/.tauri/` with
   `chmod 600`. This key signs every update; installed copies only accept updates signed with
   it. **Never commit it, and keep a backup**: losing it means existing installs can't update.

5. **Check it all**, without publishing anything:

   ```sh
   git tag v$(grep -m1 '"version"' src-tauri/tauri.conf.json | sed -E 's/.*"([0-9.]+)".*/\1/')
   scripts/publish.sh --dry-run   # builds and verifies, skips Apple signing and the upload
   git tag -d v0.1.0              # (or whichever version it printed)
   ```

   The first build may ask to let Terminal control Finder (it lays out the .dmg window): allow it.

## Each release

1. On `main`, with everything committed and pushed: `scripts/release.sh <version>`
   (e.g. `0.2.0`). It bumps the version in `tauri.conf.json`, `package.json` and `Cargo.toml`,
   commits and tags. For the very first release the version is already `0.1.0`: just
   `git tag -a v0.1.0 -m "Ternt 0.1.0"`.
2. `scripts/publish.sh`. It checks the setup, builds a universal app, signs and notarizes it
   (a few minutes, waiting on Apple), verifies the signature and Gatekeeper assessment, signs the
   update archive, writes `latest.json`, pushes `main` and the tag, and creates the GitHub Release
   with:
   - `Ternt_<version>_universal.dmg`: for new installs
   - `Ternt_<version>_universal.app.tar.gz`: the update installed copies download
   - `latest.json`: what installed copies check, at
     `https://github.com/jonkarrer/ternt/releases/latest/download/latest.json`
3. Within a few seconds of launch (or via *Ternt → Check for Updates…*), installed copies show
   an **Update** button at the bottom left of the sidebar.

If something fails halfway, nothing is published until the last step; fix it and run
`scripts/publish.sh` again. To redo a published release, delete it on GitHub first.

## Testing the updater without publishing

`bun run test:update` (any Mac with the updater key) builds 0.1.0 and 0.1.1 test bundles,
installs 0.1.0, serves 0.1.1 from a local server and checks the full update: button, download,
swap, relaunch as 0.1.1.
