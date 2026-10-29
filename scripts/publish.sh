#!/bin/zsh
# Build, sign, notarize and publish a release from a Mac with an Apple
# Developer ID. See docs/RELEASING.md for the one-time setup.
#
#   scripts/release.sh 0.2.0     # bump, commit, tag (any machine)
#   scripts/publish.sh           # on the signing Mac: build + publish the tag at HEAD
#   scripts/publish.sh --dry-run # build and check only: no Apple signing, no upload
#
# Produces a universal (Apple Silicon + Intel) app, notarized and stapled, in a
# .dmg made with hdiutil (Tauri's dmg bundler lays out the window through
# Finder AppleScript, which fails outside a GUI login session: SSH, CI, agents),
# and uploads to GitHub Releases: the .dmg for new installs, the signed
# update archive and latest.json for installed copies (src-tauri/src/updater.rs).
# Uploads use a GitHub token from the keychain (item "ternt-github") or
# $GITHUB_TOKEN.
set -e
cd ${0:A:h:h}
ROOT=$PWD
DRY=$([[ $1 == --dry-run ]] && echo 1 || true)
REPO=jonkarrer/ternt
TEAM=6ZS622Z3H8 # Apple Developer team (same as bernt and lernt); public, it is in every signed app
TARGET=${TERNT_TARGET:-universal-apple-darwin}
OUT=$ROOT/src-tauri/target/$TARGET/release/bundle
fail() { echo "!! $*"; exit 1; }
step() { echo "\n== $*"; }

# GitHub's REST API (GITHUB_API overrides it, for testing). The token reaches
# curl through a file descriptor, never the command line (no `ps` exposure).
API=${GITHUB_API:-https://api.github.com}
github() { # github METHOD PATH|URL [curl args...]: response body on stdout
  local method=$1 url=$2; shift 2
  [[ $url == http* ]] || url=$API$url
  curl -sS --fail-with-body -X $method -K <(printf 'header = "Authorization: Bearer %s"\n' "$GITHUB_TOKEN") \
    -H "Accept: application/vnd.github+json" -H "X-GitHub-Api-Version: 2022-11-28" "$@" "$url"
}
json() { bun -e "const v = JSON.parse(await Bun.stdin.text()); console.log($1)"; } # json EXPR < body

version=$(grep -m1 '"version"' src-tauri/tauri.conf.json | sed -E 's/.*"([0-9.]+)".*/\1/')
tag=v$version

step "checking Ternt $version"
[[ -z $(git status --porcelain) ]] || fail "commit or stash your changes first"
[[ $(git describe --exact-match --tags HEAD 2>/dev/null) == $tag ]] ||
  fail "HEAD isn't tagged $tag; run scripts/release.sh <version> first"
for tool in bun cargo rustup xcrun; do command -v $tool >/dev/null || fail "$tool not found"; done
if [[ -z $DRY ]]; then
  export GITHUB_TOKEN=${GITHUB_TOKEN:-$(security find-generic-password -s ternt-github -w 2>/dev/null)}
  [[ -n $GITHUB_TOKEN ]] || fail "no GitHub token: see docs/RELEASING.md (keychain item 'ternt-github')"
  # Also proves the token works before the long build.
  case $(github GET /repos/$REPO/releases/tags/$tag -o /dev/null -w '%{http_code}' 2>/dev/null || true) in
    404) ;;
    200) fail "release $tag already exists on GitHub" ;;
    401) fail "GitHub rejected the token (expired or revoked?)" ;;
    *) fail "couldn't reach GitHub's API with the token" ;;
  esac
fi
[[ $TARGET != universal-apple-darwin ]] || for t in aarch64-apple-darwin x86_64-apple-darwin; do
  rustup target list --installed | grep -qx $t || fail "missing Rust target $t (rustup target add $t)"
done

# Updater key: signs the update archive (verified by installed copies).
export TAURI_SIGNING_PRIVATE_KEY=${TAURI_SIGNING_PRIVATE_KEY:-$(cat ~/.tauri/ternt.key 2>/dev/null)}
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-$(cat ~/.tauri/ternt.key.password 2>/dev/null)}
[[ -n $TAURI_SIGNING_PRIVATE_KEY ]] || fail "updater key missing: copy ~/.tauri/ternt.key and ternt.key.password here"

config='{}'
if [[ -z $DRY ]]; then
  # Developer ID: signs the app; Apple ID + app-specific password: notarizes it.
  identity=${APPLE_SIGNING_IDENTITY:-$(security find-identity -v -p codesigning | grep -m1 -o "\"Developer ID Application: [^\"]*($TEAM)\"" | tr -d '"')}
  [[ -n $identity ]] || fail "no 'Developer ID Application' certificate for team $TEAM in the keychain (Xcode > Settings > Accounts > Manage Certificates)"
  export APPLE_SIGNING_IDENTITY=$identity
  export APPLE_TEAM_ID=$TEAM
  [[ -n $APPLE_ID ]] || fail "set APPLE_ID to your Apple developer account email"
  export APPLE_PASSWORD=${APPLE_PASSWORD:-$(security find-generic-password -s ternt-notary -a "$APPLE_ID" -w 2>/dev/null)}
  [[ -n $APPLE_PASSWORD ]] || fail "no app-specific password: see docs/RELEASING.md (keychain item 'ternt-notary')"
  config="{\"bundle\":{\"macOS\":{\"signingIdentity\":\"$identity\"}}}"
  echo "signing as: $identity (team $APPLE_TEAM_ID)"
fi

step "building ($TARGET; signing and notarizing take a few minutes)"
rm -rf $OUT
bun install --frozen-lockfile
bun node_modules/@tauri-apps/cli/tauri.js build --target $TARGET --config "$config"

app=$OUT/macos/Ternt.app
archive=$OUT/macos/Ternt.app.tar.gz
[[ -d $app && -f $archive && -f $archive.sig ]] || fail "missing build outputs in $OUT"

step "packaging the dmg"
dmg=$OUT/dmg/Ternt_${version}_universal.dmg
stage=$OUT/dmg/stage
rm -rf $OUT/dmg && mkdir -p $stage
ditto $app $stage/Ternt.app
ln -s /Applications $stage/Applications
hdiutil create -quiet -volname Ternt -srcfolder $stage -fs HFS+ -format UDZO -ov $dmg
rm -rf $stage
[[ -n $DRY ]] || codesign --sign "$identity" --timestamp $dmg

step "verifying"
codesign --verify --deep --strict $app
lipo -archs $app/Contents/MacOS/* | tr '\n' ' '; echo
[[ $(/usr/libexec/PlistBuddy -c "Print CFBundleShortVersionString" $app/Contents/Info.plist) == $version ]] ||
  fail "built app isn't version $version"
# Only system libraries: a Nix toolchain would link /nix/store paths.
otool -L $app/Contents/MacOS/* | grep -E '^\s+/(nix|opt|usr/local)/' && fail "links non-system libraries (build with Apple's toolchain)"
if [[ -z $DRY ]]; then
  /usr/bin/xcrun stapler validate $app
  spctl --assess --type execute -vv $app
  step "notarizing the dmg"
  /usr/bin/xcrun notarytool submit $dmg --apple-id $APPLE_ID --team-id $APPLE_TEAM_ID --password $APPLE_PASSWORD --wait
  /usr/bin/xcrun stapler staple $dmg
  spctl --assess --type open --context context:primary-signature -vv $dmg
fi

step "writing latest.json"
dist=$OUT/publish
rm -rf $dist && mkdir -p $dist
cp $dmg $dist/Ternt_${version}_universal.dmg
cp $archive $dist/Ternt_${version}_universal.app.tar.gz
url=https://github.com/$REPO/releases/download/$tag/Ternt_${version}_universal.app.tar.gz
sig=$(cat $archive.sig)
cat > $dist/latest.json <<JSON
{
  "version": "$version",
  "notes": "Ternt $version",
  "pub_date": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "platforms": {
    "darwin-aarch64": { "signature": "$sig", "url": "$url" },
    "darwin-x86_64": { "signature": "$sig", "url": "$url" }
  }
}
JSON
ls -la $dist

if [[ -n $DRY ]]; then
  echo "\ndry run done: nothing signed by Apple, nothing uploaded ($dist)"
  exit 0
fi

step "publishing $tag"
git push origin HEAD:main $tag
# A draft first, published once every file is up: installed copies never
# see a latest.json whose update archive isn't there yet.
notes="Download **Ternt_${version}_universal.dmg**, open it and drag Ternt to Applications. Installed copies update themselves: an Update button appears at the bottom left of the sidebar."
release=$(TAG=$tag NAME="Ternt $version" NOTES=$notes bun -e \
  'const e = process.env; console.log(JSON.stringify({ tag_name: e.TAG, name: e.NAME, body: e.NOTES, draft: true }))' |
  github POST /repos/$REPO/releases --data-binary @-) || fail "couldn't create the release (does the token have Contents: write?)"
id=$(json 'v.id' <<< $release)
uploads=$(json 'v.upload_url.replace(/{.*}$/, "")' <<< $release)
for file in $dist/Ternt_${version}_universal.dmg $dist/Ternt_${version}_universal.app.tar.gz $dist/latest.json; do
  echo "uploading ${file:t}"
  github POST "$uploads?name=${file:t}" -H "Content-Type: application/octet-stream" --data-binary @$file > /dev/null ||
    fail "upload failed; delete the draft release on GitHub, then run scripts/publish.sh again"
done
github PATCH /repos/$REPO/releases/$id --data-binary '{"draft":false,"make_latest":"true"}' > /dev/null ||
  fail "couldn't publish the draft; publish it on GitHub by hand"
echo "\npublished: https://github.com/$REPO/releases/tag/$tag"
