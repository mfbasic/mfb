#!/usr/bin/env bash
#
# smoke.sh — build the package, then build an importer against the compiled
# .mfp and construct every exported shape through the package boundary.
#
# This is the compile-time half: it proves the EXPORTED types, the union, and
# the callback signature are all nameable and constructible from outside the
# package. runtime-smoke.sh is the half that proves the bytes come out right.
#
#     ./smoke.sh [path/to/mfb]

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
printf '%s\n' '{"name":"logger_smoke","version":"0.1.0","mfb":"1.0","kind":"executable","sources":[{"root":"src","role":"main","include":["**/*.mfb"]}],"entry":"main","packages":[{"name":"logger","version":"=0.1.0","source":"file:packages/logger.mfp"}]}' > "$WORK/project.json"
cat > "$WORK/src/main.mfb" <<'MFB'
IMPORT fs
IMPORT io
IMPORT logger
IMPORT net
IMPORT tcp

FUNC ignore(entry AS logger::LogEntry) AS Nothing
END FUNC

FUNC main() AS Integer
  RES file AS fs::File = fs::createTempFile()
  RES listener = tcp::listen("127.0.0.1", 0)
  LET bound = tcp::localAddress(listener)
  RES client AS tcp::Socket = tcp::connect("127.0.0.1", bound.port)
  RES peer AS tcp::Socket = tcp::accept(listener)
  LET console AS logger::LogBackend = logger::ConsoleBackend[]
  LET fileBackend AS logger::LogBackend = logger::FileBackend[file := file]
  LET network AS logger::LogBackend = logger::NetworkBackend[socket := client]
  LET callback AS logger::LogBackend = logger::CustomBackend[callback := ignore]
  LET entry AS logger::LogEntry = logger::LogEntry[level := logger::LogLevel.INFO, timestamp := "2026-01-01T00:00:00Z", message := "ok"]
  LET common AS List OF logger::LogBackend = [console, fileBackend, network, callback]
  LET levels AS Map OF logger::LogLevel TO List OF logger::LogBackend = Map OF logger::LogLevel TO List OF logger::LogBackend {logger::LogLevel.INFO := [callback]}
  LET configured AS logger::Logger = logger::Logger[common := common, backends := levels]
  io::print(entry.message & ":" & toString(len(configured.common)) & ":" & toString(tcp::localAddress(peer).port > 0))
  ' formatEntry is exported too, and renders without touching any handle.
  io::print(logger::formatEntry(entry))
  RETURN 0
END FUNC
MFB
"$MFB" build -q "$WORK"
OUTPUT="$("$WORK/build/logger_smoke.out")"
WANT=$'ok:4:TRUE\n[2026-01-01T00:00:00Z] [INFO] ok'
[[ "$OUTPUT" == "$WANT" ]] || fail "importer output did not match
  expected: $WANT
       got: $OUTPUT"

echo "logger smoke passed"
