#!/usr/bin/env bash
# Runtime validation for net::connectTcp's timeoutMs deadline.
#
# The acceptance golden suite cannot orchestrate an external peer, so this is a
# standalone check: it starts a blackhole TCP server (saturated listen backlog,
# see net_blackhole_server.py), then builds and runs a small program that calls
# net::connectTcp with a short timeout against it. The connect must fail with
# ErrTimeout (77050008) well before the OS default connect timeout (~75s),
# proving the non-blocking-connect + poll path enforces the deadline.
#
# Usage: check-net-connect-timeout.sh <mfb-exe>
set -u

if [ "$#" -lt 1 ]; then
  echo "usage: check-net-connect-timeout.sh <mfb-exe>" >&2
  exit 2
fi

MFB_EXE=$1
ROOT=$(cd "$(dirname "$0")/.." && pwd)
ERR_TIMEOUT=77050008
CONNECT_TIMEOUT_MS=500

work=$(mktemp -d)
cleanup() {
  if [ -n "${server_pid:-}" ]; then
    kill "$server_pid" 2>/dev/null
    wait "$server_pid" 2>/dev/null
  fi
  rm -rf "$work"
}
trap cleanup EXIT

# Start the blackhole and read the port it binds.
port_file="$work/port"
python3 "$ROOT/scripts/net_blackhole_server.py" 30 >"$port_file" &
server_pid=$!
for _ in $(seq 1 50); do
  [ -s "$port_file" ] && break
  sleep 0.1
done
port=$(head -n 1 "$port_file")
if [ -z "$port" ]; then
  echo "FAIL: blackhole server did not report a port" >&2
  exit 1
fi

mkdir -p "$work/src"
cat >"$work/project.json" <<EOF
{ "name": "net_connect_timeout_check", "version": "0.1.0", "mfb": "1.0",
  "kind": "executable",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main", "targets": ["native"] }
EOF
cat >"$work/src/main.mfb" <<EOF
IMPORT net
IMPORT io

FUNC tryConnect(port AS Integer) AS Integer
  LET sock = net::connectTcp("127.0.0.1", port, $CONNECT_TIMEOUT_MS)
  net::close(sock)
  RETURN 0
END FUNC

FUNC main AS Integer
  LET code = tryConnect(port) TRAP(e)
    RECOVER e.code
  END TRAP
  io::print(toString(code))
  RETURN 0
END FUNC
EOF
# Bind the port into the source.
sed -i.bak "s/tryConnect(port)/tryConnect($port)/" "$work/src/main.mfb" && rm -f "$work/src/main.mfb.bak"

build_output=$("$MFB_EXE" build "$work" 2>&1)
if [ $? -ne 0 ]; then
  echo "FAIL: build error" >&2
  printf '%s\n' "$build_output" >&2
  exit 1
fi
exe=$(printf '%s\n' "$build_output" | sed -n 's/^Wrote executable to //p' | tail -n 1)

start=$(date +%s)
result=$("$exe")
status=$?
elapsed=$(( $(date +%s) - start ))

if [ "$status" -ne 0 ]; then
  echo "FAIL: program exited $status" >&2
  exit 1
fi
if [ "$result" != "$ERR_TIMEOUT" ]; then
  echo "FAIL: expected ErrTimeout ($ERR_TIMEOUT), got '$result'" >&2
  exit 1
fi
if [ "$elapsed" -gt 10 ]; then
  echo "FAIL: connect did not honor timeout (took ${elapsed}s)" >&2
  exit 1
fi

echo "PASS: net::connectTcp timed out with ErrTimeout in ${elapsed}s"
