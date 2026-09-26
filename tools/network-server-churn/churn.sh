#!/usr/bin/env bash
# network-server-churn — reproduces and gates bug-705: examples/network-server
# leaks dead-connection bookkeeping (and, on one path, the socket itself)
# because no code path ever splices a disconnected slot out of its parallel
# lists, and UDP peer admission has no cap.
#
# Usage: bash tools/network-server-churn/churn.sh <mfb-exe>
#
# Exercises the REAL example source under examples/network-server (built
# fresh by this script), not a private instrumented copy: the instrument is
# examples/network-server's own `--debug-counts` flag (added by this bug's
# fix), which prints "DEBUG counts <transport> <list>=<len>" once per tick.
#
# Phase A — TCP churn: connect N clients, read their one Hello line, then
# abruptly disconnect (SO_LINGER(1,0), so the close sends an RST rather than
# a FIN — the write-failure path, not the read-failure path, since an RST
# makes the read-failure sweep catch it before any write ever runs on that
# connection). Repeat for a second batch. Before the fix, `conns` grows by 40
# per batch and never drops (the bug doc's own measured 40-then-80). After the
# fix, `conns` converges back toward 0 within a bounded number of ticks after
# EACH batch — not just once at the very end, which would hide a
# never-splices bug behind belated convergence from an unrelated cause.
#
# Phase B — UDP admission cap: send more datagrams than
# wire.mfb:maxUdpPeers() from that many distinct source ports. Before the
# fix, the peer table grows unbounded (measured 397 of 400 in the bug doc).
# After the fix, it never exceeds the cap.
#
# Readiness: see .ai/testing-gates.md "An ephemeral port picked with
# bind-then-release hands a case the WRONG server" — this script confirms the
# SERVER PROCESS IT STARTED bound the port by grepping that child's own
# stdout for its "Listening" line, retrying on a fresh port if the child
# exited first, rather than a bare "is something listening" connect() probe.

set -u

if [ $# -lt 1 ]; then
  echo "usage: $0 <mfb-exe>" >&2
  exit 2
fi

MFB="$1"
if [ ! -x "$MFB" ]; then
  echo "not an executable: $MFB" >&2
  exit 2
fi

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SERVER_DIR="$REPO_ROOT/examples/network-server"

BIND_RETRIES=5
TICK_MS=500          # wire.mfb:tickNanos()
TICKS_TO_CONVERGE=6  # generous: splice runs every tick, so 1 should suffice

WORK="$(mktemp -d /tmp/net-server-churn.XXXXXX)"
SERVER_LOG="$WORK/server.log"

SERVER_PID=""

cleanup() {
  [ -n "$SERVER_PID" ] && kill -9 "$SERVER_PID" >/dev/null 2>&1
  wait >/dev/null 2>&1
  rm -rf "$WORK"
}
trap cleanup EXIT INT TERM

fail() {
  echo "FAIL: $1" >&2
  exit 1
}

echo "== building network-server =="
"$MFB" build "$SERVER_DIR" || fail "network-server did not build"
SERVER_BIN="$SERVER_DIR/build/network-server.out"
[ -x "$SERVER_BIN" ] || fail "missing $SERVER_BIN"

start_server() {
  local mode_args=("$@")
  local attempt=0
  while [ "$attempt" -lt "$BIND_RETRIES" ]; do
    attempt=$((attempt + 1))
    PORT=$(( (RANDOM % 20000) + 20000 ))
    : > "$SERVER_LOG"
    "$SERVER_BIN" "${mode_args[@]}" --port "$PORT" --debug-counts >> "$SERVER_LOG" 2>&1 &
    SERVER_PID=$!

    local waited=0
    while [ "$waited" -lt 50 ]; do
      if ! kill -0 "$SERVER_PID" >/dev/null 2>&1; then
        break
      fi
      if grep -q "Listening .*:$PORT" "$SERVER_LOG" 2>/dev/null; then
        return 0
      fi
      sleep 0.1
      waited=$((waited + 1))
    done

    kill -9 "$SERVER_PID" >/dev/null 2>&1
    wait "$SERVER_PID" >/dev/null 2>&1
    SERVER_PID=""
  done
  return 1
}

last_count() {
  local field="$1"
  grep "DEBUG counts $field=" "$SERVER_LOG" 2>/dev/null | tail -1 | sed -E "s/.*${field}=([0-9]+).*/\1/"
}

wait_for_convergence() {
  local field="$1"
  local ceiling="$2"
  local ticks="$3"
  local i=0
  while [ "$i" -lt "$ticks" ]; do
    sleep "$(awk "BEGIN{print $TICK_MS/1000}")"
    local n
    n=$(last_count "$field")
    if [ -n "$n" ] && [ "$n" -le "$ceiling" ]; then
      echo "$n"
      return 0
    fi
    i=$((i + 1))
  done
  last_count "$field"
  return 1
}

echo "== phase A: TCP churn (two batches of 40 abrupt disconnects) =="
start_server --tcp || fail "could not start network-server (tcp) after $BIND_RETRIES attempts (see $SERVER_LOG)"
echo "server pid=$SERVER_PID port=$PORT"
sleep 1 # let the first (empty) tick print a baseline DEBUG line

python3 - "$PORT" 40 > "$WORK/batch1.log" 2>&1 <<'PYEOF'
import socket, struct, sys, time
port = int(sys.argv[1])
n = int(sys.argv[2])
for _ in range(n):
    s = socket.create_connection(("127.0.0.1", port), timeout=5)
    s.recv(200)  # the one Hello line
    s.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, struct.pack('ii', 1, 0))
    s.close()
print(f"done {n} abrupt connections (batch 1)")
PYEOF
cat "$WORK/batch1.log"

n1=$(wait_for_convergence "tcp conns" 0 "$TICKS_TO_CONVERGE")
rc1=$?
if [ "$rc1" -eq 0 ]; then
  echo "PASS: phase A batch 1 — conns converged to $n1 within $TICKS_TO_CONVERGE ticks"
else
  echo "FAIL (expected pre-fix): phase A batch 1 — conns stuck at $n1, never converged"
fi

python3 - "$PORT" 40 > "$WORK/batch2.log" 2>&1 <<'PYEOF'
import socket, struct, sys, time
port = int(sys.argv[1])
n = int(sys.argv[2])
for _ in range(n):
    s = socket.create_connection(("127.0.0.1", port), timeout=5)
    s.recv(200)
    s.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, struct.pack('ii', 1, 0))
    s.close()
print(f"done {n} abrupt connections (batch 2)")
PYEOF
cat "$WORK/batch2.log"

n2=$(wait_for_convergence "tcp conns" 0 "$TICKS_TO_CONVERGE")
rc2=$?
if [ "$rc2" -eq 0 ]; then
  echo "PASS: phase A batch 2 — conns converged to $n2 within $TICKS_TO_CONVERGE ticks (not 40, not 80)"
else
  echo "FAIL (expected pre-fix): phase A batch 2 — conns stuck at $n2 (bug doc measured 80 here)"
fi

kill -9 "$SERVER_PID" >/dev/null 2>&1
wait "$SERVER_PID" >/dev/null 2>&1
SERVER_PID=""

echo "== phase B: UDP admission cap =="
start_server --udp || fail "could not start network-server (udp) after $BIND_RETRIES attempts (see $SERVER_LOG)"
echo "server pid=$SERVER_PID port=$PORT"
sleep 1

SEND_COUNT=400
python3 - "$PORT" "$SEND_COUNT" > "$WORK/spray.log" 2>&1 <<'PYEOF'
import socket, sys
port = int(sys.argv[1])
n = int(sys.argv[2])
for _ in range(n):
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    s.bind(("127.0.0.1", 0))
    s.sendto(b"hi\n", ("127.0.0.1", port))
    s.close()
print(f"sent {n} datagrams from {n} distinct source ports")
PYEOF
cat "$WORK/spray.log"

sleep 1.5 # let a tick or two register every datagram

peers=$(last_count "udp keys")
if [ -z "$peers" ]; then
  fail "no 'DEBUG counts udp keys=' line ever appeared (see $SERVER_LOG)"
fi
# 256 is wire.mfb:maxUdpPeers() — kept in sync manually; a change to that
# constant should update this probe too.
CAP=256
if [ "$peers" -le "$CAP" ]; then
  echo "PASS: phase B — udp peer table capped at $peers (<= $CAP) despite $SEND_COUNT distinct-port datagrams"
  B_OK=1
else
  echo "FAIL (expected pre-fix): phase B — udp peer table grew to $peers (> $CAP)"
  B_OK=0
fi

kill -9 "$SERVER_PID" >/dev/null 2>&1
wait "$SERVER_PID" >/dev/null 2>&1
SERVER_PID=""

if [ "$rc1" -eq 0 ] && [ "$rc2" -eq 0 ] && [ "${B_OK:-0}" -eq 1 ]; then
  echo "OVERALL PASS"
  exit 0
else
  echo "OVERALL FAIL"
  exit 1
fi
