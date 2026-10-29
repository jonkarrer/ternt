#!/bin/zsh
# End-to-end self-update test (macOS). Builds two signed app bundles, 0.1.0
# and 0.1.1, "installs" 0.1.0, serves 0.1.1 as the latest release from a
# local server, and lets the installed app find it, install it when its
# button is clicked, and relaunch as 0.1.1 (see e2e/update/test.js).
#
# Needs the updater signing key: ~/.tauri/ternt.key (+ .password), or
# TAURI_SIGNING_PRIVATE_KEY / TAURI_SIGNING_PRIVATE_KEY_PASSWORD.
# Debug bundles, so it takes a few minutes; output in e2e/.out/update/.
# The hardened runtime is off for these test bundles only: with a Nix
# toolchain, local builds link /nix/store libraries, which library
# validation rejects. (Release builds come from CI with the system toolchain.)
set -e
E2E=${0:A:h}
ROOT=${E2E:h}
WORK=$E2E/.out/update
PORT=8766
ARCH=$([[ $(uname -m) == arm64 ]] && echo aarch64 || echo x86_64)

export TAURI_SIGNING_PRIVATE_KEY=${TAURI_SIGNING_PRIVATE_KEY:-$(cat ~/.tauri/ternt.key)}
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-$(cat ~/.tauri/ternt.key.password)}
export VITE_TERNT_TEST=1

rm -rf $WORK && mkdir -p $WORK/install $WORK/server
BUNDLE=$ROOT/src-tauri/target/debug/bundle/macos

build() {
  echo "== building $1"
  (cd $ROOT && bun node_modules/@tauri-apps/cli/tauri.js build --debug --bundles app \
    --config "{\"version\":\"$1\",\"bundle\":{\"macOS\":{\"hardenedRuntime\":false}}}" > $WORK/build-$1.log 2>&1) || { tail -30 $WORK/build-$1.log; exit 1; }
}

build 0.1.0
cp -R $BUNDLE/Ternt.app $WORK/install/
build 0.1.1
cp $BUNDLE/Ternt.app.tar.gz $WORK/server/
cat > $WORK/server/latest.json <<JSON
{
  "version": "0.1.1",
  "notes": "Update test",
  "pub_date": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "platforms": {
    "darwin-$ARCH": {
      "signature": "$(cat $BUNDLE/Ternt.app.tar.gz.sig)",
      "url": "http://localhost:$PORT/Ternt.app.tar.gz"
    }
  }
}
JSON

bun $E2E/update/serve.ts $WORK/server $PORT > $WORK/server.log 2>&1 &
server=$!
trap "kill $server 2>/dev/null" EXIT

echo "== running the installed 0.1.0"
APP=$WORK/install/Ternt.app
# Settings go in a file passed as an argument: the relaunch after updating
# goes through LaunchServices, which drops the environment and stdout.
cat > $WORK/test.env <<ENV
TERNT_TEST=$E2E/update/test.js
TERNT_TEST_LOG=$WORK/test.log
TERNT_SHOTS=$WORK
TERNT_UPDATE_URL=http://localhost:$PORT/latest.json
TERNT_SESSION_FILE=$WORK/session.json
TERNT_HISTORY_FILE=$WORK/history.json
ENV
$APP/Contents/MacOS/$(ls $APP/Contents/MacOS) --ternt-test-env=$WORK/test.env > $WORK/app.log 2>&1 &

result=1
for i in {1..240}; do
  grep -q "UPDATE TEST PASSED" $WORK/test.log 2>/dev/null && { result=0; break; }
  grep -qE "\[test\] FAIL" $WORK/test.log 2>/dev/null && break
  grep -q "panicked" $WORK/app.log && break
  sleep 0.5
done
sleep 1
pkill -f "$WORK/install/Ternt.app" 2>/dev/null || true
grep -v "TEST DONE" $WORK/test.log
grep "panicked" $WORK/app.log || true
installed=$(/usr/libexec/PlistBuddy -c "Print CFBundleShortVersionString" $APP/Contents/Info.plist)
echo "installed bundle is now $installed"
[[ $result == 0 && $installed == 0.1.1 ]] && echo "update test passed" || { echo "!! update test FAILED (logs: $WORK/test.log, $WORK/app.log)"; exit 1; }
