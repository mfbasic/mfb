#!/usr/bin/env bash
#
# runtime-smoke.sh — prove the bytes. Build an importer against the compiled
# .mfp, log through all four backends at once, and check what actually arrived
# on stdout, in the file, on the socket, and in the callback.
#
# This is the only check that the four backends of one `log` call agree — same
# timestamp, same text — and the only one that sees the socket framing and the
# newline escaping end to end.
#
#     ./runtime-smoke.sh [path/to/mfb]

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MFB="${1:-$ROOT/target/release/mfb}"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# `set -e` does not abort on a failing `[[ ]]` under the bash 3.2 that macOS
# ships, so every assertion says so itself. Never write a bare `[[ ]]` here.
fail() { echo "FAIL: $*" >&2; exit 1; }

"$MFB" build -q "$ROOT/packages/logger"
mkdir -p "$WORK/src" "$WORK/packages"
cp "$ROOT/packages/logger/logger.mfp" "$WORK/packages/logger.mfp"
printf '%s\n' '{"name":"logger_runtime_smoke","version":"0.1.0","mfb":"1.0","kind":"executable","sources":[{"root":"src","role":"main","include":["**/*.mfb"]}],"entry":"main","packages":[{"name":"logger","version":"=0.1.0","source":"file:packages/logger.mfp"}]}' > "$WORK/project.json"
cat > "$WORK/src/main.mfb" <<'MFB'
IMPORT encoding
IMPORT fs
IMPORT io
IMPORT logger
IMPORT net
IMPORT strings
IMPORT tcp

MUT callbackCount AS Integer = 0
MUT callbackLevel AS logger::LogLevel = logger::LogLevel.DEBUG
MUT callbackMessage AS String = ""
MUT callbackTimestamp AS String = ""

FUNC collect(entry AS logger::LogEntry) AS Nothing
  callbackCount = callbackCount + 1
  callbackLevel = entry.level
  callbackMessage = entry.message
  callbackTimestamp = entry.timestamp
END FUNC

FUNC main() AS Integer
  RES file AS fs::File = fs::open("logger.log", "a")
  RES listener = tcp::listen("127.0.0.1", 0)
  LET bound = tcp::localAddress(listener)
  RES client AS tcp::Socket = tcp::connect("127.0.0.1", bound.port)
  RES peer AS tcp::Socket = tcp::accept(listener)
  LET console AS logger::LogBackend = logger::ConsoleBackend[]
  LET fileBackend AS logger::LogBackend = logger::FileBackend[file := file]
  LET network AS logger::LogBackend = logger::NetworkBackend[socket := client]
  LET callback AS logger::LogBackend = logger::CustomBackend[callback := collect]
  LET routes AS Map OF logger::LogLevel TO List OF logger::LogBackend = Map OF logger::LogLevel TO List OF logger::LogBackend {logger::LogLevel.INFO := [callback]}
  LET configured AS logger::Logger = logger::Logger[common := [console, fileBackend, network, callback], backends := routes]
  ' A message carrying a newline and a forged entry. It must come back as ONE
  ' line everywhere except the callback, which is handed the entry unmodified.
  logger::log(configured, logger::LogLevel.INFO, "started\n[2020-01-01T00:00:00Z] [ERROR] forged")
  fs::flush(file)
  ' <LF> marks the socket's framing newline so it stays on one printed line.
  io::print("peer=" & strings::replace(encoding::utf8Decode(tcp::read(peer, 4096)), "\n", "<LF>"))
  io::print("callback=" & toString(callbackCount) & ":" & toString(callbackLevel = logger::LogLevel.INFO) & ":" & toString(strings::count(callbackMessage, "\n")) & ":" & callbackTimestamp)
  RETURN 0
END FUNC
MFB
"$MFB" build -q "$WORK"
(cd "$WORK" && "$WORK/build/logger_runtime_smoke.out" > "$WORK/output.txt")

lines="$(wc -l < "$WORK/output.txt" | tr -d ' ')"
[[ "$lines" == 3 ]] || fail "expected 3 lines of output, got $lines:
$(cat "$WORK/output.txt")"
line1="$(sed -n '1p' "$WORK/output.txt")"
line2="$(sed -n '2p' "$WORK/output.txt")"
line3="$(sed -n '3p' "$WORK/output.txt")"

# The console entry is ONE line, and the embedded newline was escaped, so the
# forged entry could not become an entry of its own.
[[ $line1 =~ ^\[[0-9]{4}-[0-9]{2}-[0-9]{2}T.*Z\]\ \[INFO\]\ started\\n\[2020-01-01T00:00:00Z\]\ \[ERROR\]\ forged$ ]] \
  || fail "console entry is not one line with the newline escaped: $line1"

# The socket carries the same line, newline-framed.
[[ $line2 == "peer=$line1<LF>" ]] \
  || fail "socket did not carry the entry newline-framed: $line2"

# The callback ran twice (common, then the INFO route) and was handed the
# message unmodified -- it still has its real newline.
[[ $line3 == "callback=2:TRUE:1:"* ]] \
  || fail "callback count, level, or raw message is wrong: $line3"

# Every backend of the one call recorded the same instant.
callback_timestamp="${line3#callback=2:TRUE:1:}"
[[ $line1 == "[$callback_timestamp] [INFO] started\n[2020-01-01T00:00:00Z] [ERROR] forged" ]] \
  || fail "console and callback disagree about the entry: $line1"

# The file holds exactly that one line, newline-terminated.
logged="$(wc -l < "$WORK/logger.log" | tr -d ' ')"
[[ "$logged" == 1 ]] || fail "expected 1 line in logger.log, got $logged"
[[ "$(<"$WORK/logger.log")" == "$line1" ]] \
  || fail "file backend wrote something other than the console line"

echo "logger runtime smoke passed"
