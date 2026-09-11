#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MFB="${1:-$ROOT/target/release/mfb}"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

"$MFB" build -q "$ROOT/packages/logger"
mkdir -p "$WORK/src" "$WORK/packages"
cp "$ROOT/packages/logger/logger.mfp" "$WORK/packages/logger.mfp"
printf '%s\n' '{"name":"logger_runtime_smoke","version":"0.1.0","mfb":"1.0","kind":"executable","sources":[{"root":"src","role":"main","include":["**/*.mfb"]}],"entry":"main","packages":[{"name":"logger","version":"=0.1.0","source":"file:packages/logger.mfp"}]}' > "$WORK/project.json"
printf '%s\n' 'IMPORT encoding' 'IMPORT fs' 'IMPORT io' 'IMPORT logger' 'IMPORT net' 'IMPORT tcp' '' 'MUT callbackCount AS Integer = 0' 'MUT callbackLevel AS logger::LogLevel = logger::LogLevel.DEBUG' 'MUT callbackMessage AS String = ""' 'MUT callbackTimestamp AS String = ""' '' 'FUNC collect(entry AS logger::LogEntry) AS Nothing' '  callbackCount = callbackCount + 1' '  callbackLevel = entry.level' '  callbackMessage = entry.message' '  callbackTimestamp = entry.timestamp' 'END FUNC' '' 'FUNC main() AS Integer' '  RES file AS fs::File = fs::open("logger.log", "a")' '  RES listener = tcp::listen("127.0.0.1", 0)' '  LET bound = tcp::localAddress(listener)' '  RES client AS tcp::Socket = tcp::connect("127.0.0.1", bound.port)' '  RES peer AS tcp::Socket = tcp::accept(listener)' '  LET console AS logger::LogBackend = logger::ConsoleBackend[use_colors := FALSE]' '  LET fileBackend AS logger::LogBackend = logger::FileBackend[file := file]' '  LET network AS logger::LogBackend = logger::NetworkBackend[socket := client]' '  LET callback AS logger::LogBackend = logger::CustomBackend[callback := collect]' '  LET routes AS Map OF logger::LogLevel TO List OF logger::LogBackend = Map OF logger::LogLevel TO List OF logger::LogBackend {logger::LogLevel.INFO := [callback]}' '  LET configured AS logger::Logger = logger::Logger[common := [console, fileBackend, network, callback], backends := routes]' '  logger::log(configured, logger::LogLevel.INFO, "started")' '  fs::flush(file)' '  io::print("peer=" & encoding::utf8Decode(tcp::read(peer, 4096)))' '  io::print("callback=" & toString(callbackCount) & ":" & toString(callbackLevel = logger::LogLevel.INFO) & ":" & callbackMessage & ":" & callbackTimestamp)' '  RETURN 0' 'END FUNC' > "$WORK/src/main.mfb"
"$MFB" build -q "$WORK"
(cd "$WORK" && "$WORK/build/logger_runtime_smoke.out" > "$WORK/output.txt")
[[ "$(wc -l < "$WORK/output.txt" | tr -d ' ')" == 3 ]]
line1="$(sed -n '1p' "$WORK/output.txt")"
line2="$(sed -n '2p' "$WORK/output.txt")"
line3="$(sed -n '3p' "$WORK/output.txt")"
[[ $line1 =~ ^\[[0-9]{4}-[0-9]{2}-[0-9]{2}T.*Z\]\ \[INFO\]\ started$ ]]
[[ $line2 == "peer=$line1" ]]
[[ $line3 == "callback=2:TRUE:started:"* ]]
callback_timestamp="${line3#callback=2:TRUE:started:}"
[[ $line1 == "[$callback_timestamp] [INFO] started" ]]
[[ "$(<"$WORK/logger.log")" == "$line1" ]]
