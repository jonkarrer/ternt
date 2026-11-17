#!/bin/zsh
# End-to-end self-update test (macOS). Builds signed app bundles 0.1.0 to
# 0.1.3, "installs" 0.1.0 and serves the others as the latest release from a
# local server, one after another, checking each way an update lands (see
# e2e/update/test.js):
#   0.1.1  downloads in the background and says it's ready; the app is then
#          killed (no normal quit), and the next launch installs the kept
#          download and relaunches
#   0.1.2  required: downloads, installs and relaunches with no click
#   0.1.3  downloads in the background and installs when the app quits
#
# Needs an updater signing key: ~/.tauri/ternt.key (+ .password, .pub), or
# TAURI_SIGNING_PRIVATE_KEY / TAURI_SIGNING_PRIVATE_KEY_PASSWORD /
# TAURI_SIGNING_PUBLIC_KEY. The test bundles trust that key's public half,
# so any key works, not only the release key.
# Debug bundles, so it takes a few minutes; output in e2e/.out/update/.
# The hardened runtime is off for these test bundles only: outside `nix
# develop`, a Nix shell's linker puts /nix/store libraries in the app, which
# library validation rejects.
set -e
E2E=${0:A:h}
ROOT=${E2E:h}
WORK=$E2E/.out/update
PORT=8766
ARCH=$([[ $(uname -m) == arm64 ]] && echo aarch64 || echo x86_64)

export TAURI_SIGNING_PRIVATE_KEY=${TAURI_SIGNING_PRIVATE_KEY:-$(cat ~/.tauri/ternt.key)}
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-$(cat ~/.tauri/ternt.key.password)}
PUBKEY=${TAURI_SIGNING_PUBLIC_KEY:-$(cat ~/.tauri/ternt.key.pub)}
export VITE_TERNT_TEST=1

rm -rf $WORK && mkdir -p $WORK/install $WORK/server
BUNDLE=$ROOT/src-tauri/target/debug/bundle/macos

build() {
  echo "== building $1"
  (cd $ROOT && bun node_modules/@tauri-apps/cli/tauri.js build --debug --bundles app \
    --config "{\"version\":\"$1\",\"bundle\":{\"macOS\":{\"hardenedRuntime\":false}},\"plugins\":{\"updater\":{\"pubkey\":\"$PUBKEY\"}}}" > $WORK/build-$1.log 2>&1) || { tail -30 $WORK/build-$1.log; exit 1; }
}

build 0.1.0
cp -R $BUNDLE/Ternt.app $WORK/install/
for v in 0.1.1 0.1.2 0.1.3; do
  build $v
  cp $BUNDLE/Ternt.app.tar.gz $WORK/server/Ternt-$v.app.tar.gz
  cp $BUNDLE/Ternt.app.tar.gz.sig $WORK/sig-$v
done

# Each version gets its own manifest (the app asks for
# /<its version>/latest.json), so nothing changes under a running app.
manifest() { # manifest RUNNING VERSION [required]: what RUNNING is offered
  mkdir -p $WORK/server/$1
  local required=
  [[ -n $3 ]] && required=$'\n  "required": true,'
  cat > $WORK/server/$1/latest.json <<JSON
{
  "version": "$2",
  "notes": "Update test",
  "pub_date": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",$required
  "platforms": {
    "darwin-$ARCH": {
      "signature": "$(cat $WORK/sig-$2)",
      "url": "http://localhost:$PORT/Ternt-$2.app.tar.gz"
    }
  }
}
JSON
}
manifest 0.1.0 0.1.1
manifest 0.1.1 0.1.2 required
manifest 0.1.2 0.1.3

bun $E2E/update/serve.ts $WORK/server $PORT > $WORK/server.log 2>&1 &
server=$!
trap "kill $server 2>/dev/null" EXIT

APP=$WORK/install/Ternt.app
# Settings go in a file passed as an argument: the relaunch after updating
# goes through LaunchServices, which drops the environment and stdout.
cat > $WORK/test.env <<ENV
TERNT_TEST=$E2E/update/test.js
TERNT_TEST_LOG=$WORK/test.log
TERNT_SHOTS=$WORK
TERNT_UPDATE_URL=http://localhost:$PORT/{{current_version}}/latest.json
TERNT_UPDATE_DIR=$WORK/downloads
TERNT_SESSION_FILE=$WORK/session.json
TERNT_HISTORY_FILE=$WORK/history.json
ENV
launch() {
  echo "== launching the installed app"
  $APP/Contents/MacOS/$(ls $APP/Contents/MacOS) --ternt-test-env=$WORK/test.env >> $WORK/app.log 2>&1 &
}
running() { pgrep -f "$WORK/install/Ternt.app" > /dev/null; }
failed() {
  grep -qE "\[test\] FAIL" $WORK/test.log 2>/dev/null || grep -q "panicked" $WORK/app.log 2>/dev/null
}
# wait_for PATTERN [seconds]: until the test log has it
wait_for() {
  for i in {1..$(( ${2:-60} * 2 ))}; do
    grep -q "$1" $WORK/test.log 2>/dev/null && return 0
    failed && return 1
    sleep 0.5
  done
  echo "!! timed out waiting for: $1"
  return 1
}

result=1
{
  launch
  wait_for "READY 0.1.1" 120
  echo "== killing it (no normal quit)"
  pkill -9 -f "$WORK/install/Ternt.app"
  sleep 1
  launch
  wait_for "running version 0.1.1" 120
  wait_for "running version 0.1.2" 120
  wait_for "QUITTING" 120
  for i in {1..60}; do running || break; sleep 0.5; done
  grep -q "Ternt-0.1.1.app.tar.gz" $WORK/server.log && (( $(grep -c "Ternt-0.1.1.app.tar.gz" $WORK/server.log) == 1 )) \
    || { echo "!! 0.1.1 should download once (the relaunch uses the kept copy)"; grep tar.gz $WORK/server.log; false; }
  result=0
} || true
pkill -f "$WORK/install/Ternt.app" 2>/dev/null || true
grep -v "TEST DONE" $WORK/test.log
grep "panicked" $WORK/app.log || true
installed=$(/usr/libexec/PlistBuddy -c "Print CFBundleShortVersionString" $APP/Contents/Info.plist)
echo "installed bundle is now $installed"
[[ $result == 0 && $installed == 0.1.3 ]] && echo "update test passed" || { echo "!! update test FAILED (logs: $WORK/test.log, $WORK/app.log)"; exit 1; }
