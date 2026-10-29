#!/bin/zsh
# End-to-end tests (macOS, debug build). Each test is a script evaluated in the
# UI webview with `t` = window.__moove (see src/lib/debug.ts).
#
#   e2e/run.sh            run every test in e2e/tests
#   e2e/run.sh browsing   run one test
#
# Output and screenshots go to e2e/.out/.
setopt null_glob
E2E=${0:A:h}
ROOT=${E2E:h}
OUT=$E2E/.out
mkdir -p $OUT

(cd $ROOT/src-tauri && cargo build -q) || exit 1
curl -s -o /dev/null localhost:1420 || { (cd $ROOT && bun run dev > $OUT/vite.log 2>&1 &); }
curl -s -o /dev/null localhost:8765 || { (bun $E2E/fixtures/server.ts > $OUT/fixtures.log 2>&1 &); }
for i in {1..80}; do curl -s -o /dev/null localhost:1420 && curl -s -o /dev/null localhost:8765 && break; sleep 0.25; done

tests=(${@:-$E2E/tests/*.js(:t:r)})
failed=0
for name in $tests; do
  echo "== $name"
  rm -f $OUT/$name-*.png
  # Tests share a session file by prefix (`session-1` saves, `session-2`
  # restores); the first test of a group starts from scratch.
  key=${name%%-*}
  [[ $name == $key || $name == *-1 ]] && rm -f $OUT/$key.session.json $OUT/$key.history.json
  MOOVE_TEST=$E2E/tests/$name.js MOOVE_SHOTS=$OUT MOOVE_SESSION_FILE=$OUT/$key.session.json \
    MOOVE_HISTORY_FILE=$OUT/$key.history.json $ROOT/src-tauri/target/debug/moove > $OUT/$name.log 2>&1 &
  pid=$!
  for i in {1..400}; do grep -q "TEST DONE" $OUT/$name.log && break; kill -0 $pid 2>/dev/null || break; sleep 0.25; done
  # Give the app a moment to exit on its own (it saves its session on exit).
  for i in {1..20}; do kill -0 $pid 2>/dev/null || break; sleep 0.1; done
  kill $pid 2>/dev/null; wait $pid 2>/dev/null
  grep -E "^\[test\]|panicked" $OUT/$name.log | grep -v "TEST DONE"
  if grep -qE "\[test\] FAIL|panicked" $OUT/$name.log || ! grep -q "TEST DONE" $OUT/$name.log; then
    echo "!! $name FAILED"; failed=1
  fi
done
exit $failed
