#!/usr/bin/env bash
# Runtime validation for udp::send / udp::receive against a REAL external peer
# (plan-110-F Phase 1).
#
# Every `udp` fixture in the acceptance suite binds two MFBASIC sockets and
# sends between them. That proves the two halves agree with each other, not that
# either agrees with the wire. This check puts an ordinary POSIX-sockets echo
# peer (scripts/net_udp_echo_server.py) on the other end, so a round trip has to
# be correct on the wire, and asserts the three facts that are only observable
# with a foreign peer:
#
#   1. a payload survives the round trip byte for byte, including a multi-byte
#      UTF-8 sequence and a zero-length datagram;
#   2. `Datagram.from` reports the address the peer actually sent from -- the
#      echo server's own bound port, not the one we sent TO from;
#   3. datagram boundaries are preserved: three sends become three receives,
#      never merged.
#
# Loopback only; no public-network dependency and no mock.
#
# Usage: check-udp-echo.sh <mfb-exe>
set -u

if [ "$#" -lt 1 ]; then
  echo "usage: check-udp-echo.sh <mfb-exe>" >&2
  exit 2
fi

MFB_EXE=$1
ROOT=$(cd "$(dirname "$0")/.." && pwd)

work=$(mktemp -d)
server_pid=""
cleanup() {
  if [ -n "$server_pid" ]; then
    kill "$server_pid" 2>/dev/null
    wait "$server_pid" 2>/dev/null
  fi
  rm -rf "$work"
}
trap cleanup EXIT

port_file="$work/port"
python3 "$ROOT/scripts/net_udp_echo_server.py" 30 >"$port_file" &
server_pid=$!
for _ in $(seq 1 50); do
  [ -s "$port_file" ] && break
  sleep 0.1
done
port=$(head -n 1 "$port_file")
if [ -z "$port" ]; then
  echo "FAIL: udp echo server did not report a port" >&2
  exit 1
fi

mkdir -p "$work/src"
cat >"$work/project.json" <<'EOF'
{ "name": "udp_echo_check", "version": "0.1.0", "mfb": "1.0",
  "kind": "executable",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main", "targets": ["native"] }
EOF
cat >"$work/src/main.mfb" <<EOF
IMPORT collections
IMPORT encoding
IMPORT io
IMPORT net
IMPORT udp

FUNC main AS Integer
  LET peer AS net::Address = collections::get(net::lookup("127.0.0.1", $port), 0)
  RES sock = udp::bind("127.0.0.1", 0)
  udp::setReadTimeout(sock, 5000)

  ' 1. A multi-byte payload must survive verbatim.
  udp::send(sock, peer, "héllo")
  LET first = udp::receive(sock, 1024)
  io::print("payload=" & encoding::utf8Decode(first.bytes))

  ' 2. The sender address is the ECHO SERVER's port, not ours.
  io::print("fromHost=" & first.from.host)
  io::print("fromPeerPort=" & toString(first.from.port = $port))
  io::print("notOurPort=" & toString(first.from.port <> udp::localAddress(sock).port))

  ' 3. Boundaries are preserved: three sends, three receives, never merged.
  udp::send(sock, peer, "a")
  udp::send(sock, peer, "bb")
  udp::send(sock, peer, "ccc")
  LET d1 = udp::receive(sock, 1024)
  LET d2 = udp::receive(sock, 1024)
  LET d3 = udp::receive(sock, 1024)
  io::print("lens=" & toString(len(d1.bytes)) & "," & toString(len(d2.bytes)) & "," & toString(len(d3.bytes)))

  ' A zero-length datagram is ordinary, and is NOT end-of-stream.
  MUT empty AS List OF Byte = []
  udp::send(sock, peer, empty)
  LET zero = udp::receive(sock, 1024)
  io::print("zeroLen=" & toString(len(zero.bytes)))

  udp::send(sock, peer, "QUIT")
  RETURN 0
END FUNC
EOF

build_output=$("$MFB_EXE" build "$work" 2>&1)
if [ $? -ne 0 ]; then
  echo "FAIL: build error" >&2
  printf '%s\n' "$build_output" >&2
  exit 1
fi
exe=$(printf '%s\n' "$build_output" | sed -n 's/^Wrote executable to //p' | tail -n 1)

actual=$("$exe" 2>&1)
status=$?
if [ "$status" -ne 0 ]; then
  echo "FAIL: program exited $status" >&2
  printf '%s\n' "$actual" >&2
  exit 1
fi

expected="payload=héllo
fromHost=127.0.0.1
fromPeerPort=TRUE
notOurPort=TRUE
lens=1,2,3
zeroLen=0"

if [ "$actual" != "$expected" ]; then
  echo "FAIL: udp echo round trip mismatch" >&2
  echo "--- expected" >&2; printf '%s\n' "$expected" >&2
  echo "--- actual" >&2;   printf '%s\n' "$actual" >&2
  exit 1
fi

echo "PASS: udp round-tripped through a foreign peer (payload, sender address, boundaries, zero-length)"
