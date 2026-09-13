#!/usr/bin/env bash
# Self-test for scripts/remote-common.sh and scripts/rgba_compare.py (plan-131-D). Needs no
# remote box: every case runs locally. Prints one line per case; exits non-zero on any miss.
#
# Usage: bash scripts/remote-common-selftest.sh
set -u
DIR="$(cd "$(dirname "$0")" && pwd)"
. "$DIR/remote-common.sh"

bad=0
check() { # <label> <condition-exit-code>
  if [ "$2" -eq 0 ]; then echo "  ok   $1"; else echo "  BAD  $1"; bad=$((bad + 1)); fi
}
rc_workdir

# 1. watchdog kills a sleep 30 at 1 s and returns 99.
start=$(date +%s)
watchdog 1 sleep 30 >/dev/null
status=$?
elapsed=$(( $(date +%s) - start ))
[ "$status" -eq 99 ] && [ "$elapsed" -le 3 ]
check "watchdog: sleep 30 killed at 1 s (exit $status, ${elapsed}s)" $?

# 2. watchdog passes stdout and the exit code through for a command that finishes.
out=$(watchdog 5 sh -c 'echo hello; exit 3')
status=$?
[ "$out" = "hello" ] && [ "$status" -eq 3 ]
check "watchdog: passes output and exit code (out=$out, exit $status)" $?

# 3. remote_ssh against an unused local port fails within ConnectTimeout + 2 s.
port=1
start=$(date +%s)
MFB_SSH_CONNECT_TIMEOUT=2 remote_ssh "$port" test@127.0.0.1 true >/dev/null 2>&1
status=$?
elapsed=$(( $(date +%s) - start ))
[ "$status" -ne 0 ] && [ "$elapsed" -le 12 ]
check "remote_ssh: closed port fails fast (exit $status, ${elapsed}s)" $?

# 4. pass/fail counting, both fail streams.
rc_failures=0
pass "a" >/dev/null
fail "b" >/dev/null
fail "c" >/dev/null
[ "$rc_failures" -eq 2 ]
check "pass/fail: two fails counted (rc_failures=$rc_failures)" $?
err=$( { RC_FAIL_TO_STDERR=1; fail "to-stderr"; } 2>&1 >/dev/null )
[ "$err" = "FAIL: to-stderr" ]
check "fail: RC_FAIL_TO_STDERR=1 writes to stderr" $?
rc_failures=0

# 5. rgba_compare at tolerance (delta 2 on 2% of pixels) is ok; just beyond (delta 3) is not.
python3 - "$work" <<'PY'
import os, sys
w = sys.argv[1]
n = 100 * 100
ref = bytearray([10, 20, 30, 255] * n)
at = bytearray(ref)
beyond = bytearray(ref)
for p in range(n // 50):          # exactly 2% of pixels
    at[p * 4] += 2
    beyond[p * 4] += 3
for name, data in (("ref", ref), ("at", at), ("beyond", beyond)):
    open(os.path.join(w, name + ".rgba"), "wb").write(data)
PY
v_at=$(python3 "$DIR/rgba_compare.py" "$work/ref.rgba" "$work/at.rgba" 100)
v_beyond=$(python3 "$DIR/rgba_compare.py" "$work/ref.rgba" "$work/beyond.rgba" 100)
case "$v_at" in ok*) true ;; *) false ;; esac
check "rgba_compare: delta 2 on 2% → ok ($v_at)" $?
case "$v_beyond" in ok*) false ;; *) true ;; esac
check "rgba_compare: delta 3 → beyond ($v_beyond)" $?

echo
if [ "$bad" -ne 0 ]; then
  echo "remote-common selftest: $bad case(s) failed"
  exit 1
fi
echo "remote-common selftest: all cases passed"
