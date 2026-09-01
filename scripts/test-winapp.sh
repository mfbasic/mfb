#!/usr/bin/env bash
# Runtime acceptance for Windows `--app` builds (bug-478).
#
# The Windows twin of `test-appimage.sh` and `test-macapp.sh`, and it exists because
# its absence was the bug: **no Windows binary this repository produces had ever been
# executed by an automated test.** Every `fs` write on Windows was broken, app mode
# exited before its worker ran, the headless flag was unreadable, and an empty
# `SUB main() END SUB` died with `0xC0000005` — four defects and a fifth, all shipped,
# all invisible to a green `cargo test` on a macOS host.
#
# Usage: scripts/test-winapp.sh <mfb-exe> [--box <port>]
#
#   --box <port>   ssh port of the Windows box (default 2230).
#
# The box needs no GPU and no display: `MFB_WINAPP_HEADLESS=1` skips the window and
# the program's own output is what is checked. That is deliberate — it is what makes
# this runnable at all on a headless VM.
set -euo pipefail

MFB_EXE="${1:?usage: test-winapp.sh <mfb-exe> [--box <port>]}"
shift || true
PORT=2230
while [ $# -gt 0 ]; do
  case "$1" in
    --box) PORT="$2"; shift 2 ;;
    *) echo "test-winapp: unknown argument $1" >&2; exit 2 ;;
  esac
done

host="test@127.0.0.1"
remote='C:\mfbwin'
fails=0
pass() { echo "ok: $1"; }
fail() { echo "FAIL: $1"; fails=$((fails + 1)); }

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# One program that exercises the three seams the four fixed defects lived in: the
# worker thread runs at all (CreateThread's handle), the entry reaches its first
# statement (the RNG seed's shadow space), and a file write actually writes
# (CreateFileW's handle). Reading the file back is what makes the last one an
# assertion rather than a hope — the original bug left a 0-byte file and reported
# success.
proj="$work/winapp"
mkdir -p "$proj/src"
cat > "$proj/project.json" <<'JSON'
{ "name": "winapp", "version": "0.1.0", "mfb": "1.0", "kind": "executable",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main", "targets": ["native"] }
JSON
cat > "$proj/src/main.mfb" <<'MFB'
IMPORT app
IMPORT fs
IMPORT io
IMPORT os

SUB main()
  io::print("worker reached main")
  fs::writeText("winapp.txt", "written by the worker")
  LET back AS String = fs::readText("winapp.txt")
  io::print("readback:" & back)
  ' bug-479: reading a variable that is actually SET is the case that was broken.
  ' `emit_env_get` answered in the aligned MFB bank while both consumers read the C
  ' result, so what came back was `WideCharToMultiByte`'s byte count — and the caller
  ' walked a C string from it. An unset variable happened to work, because the count
  ' was 0 and 0 means "not found".
  io::print("env:" & os::getEnvOr("MFB_WINAPP_PROBE", "MISSING"))
  io::print("has:" & toString(os::hasEnv("MFB_WINAPP_PROBE")))
  io::print("noenv:" & os::getEnvOr("MFB_WINAPP_ABSENT", "MISSING"))
END SUB
MFB

echo "--- building for windows-x86_64 ---"
"$MFB_EXE" build --app --target windows-x86_64 "$proj" >/dev/null

cat > "$work/runner.bat" <<'BAT'
@echo off
setlocal
set MFB_WINAPP_HEADLESS=1
set MFB_WINAPP_PROBE=probe-value
cd /d C:\mfbwin
winapp.exe > winapp.out 2>&1
echo rc=%errorlevel%
type winapp.out
BAT

echo "--- running on box $PORT ---"
ssh -p "$PORT" "$host" "if not exist $remote mkdir $remote" >/dev/null
ssh -p "$PORT" "$host" "del /q $remote\\winapp.txt $remote\\winapp.out 2>nul" >/dev/null 2>&1 || true
scp -P "$PORT" "$proj/build/winapp.exe" "$host:C:/mfbwin/winapp.exe" >/dev/null
scp -P "$PORT" "$work/runner.bat" "$host:C:/mfbwin/runner.bat" >/dev/null
out="$(ssh -p "$PORT" "$host" "$remote\\runner.bat" 2>&1 || true)"
echo "$out" | sed 's/^/    /'

case "$out" in
  *"rc=0"*) pass "the app-mode program exited cleanly" ;;
  *) fail "the app-mode program did not exit 0 — a worker that faults reports 0xC0000005 as rc=-1073741819" ;;
esac
case "$out" in
  *"worker reached main"*) pass "the worker thread reached the program's first statement" ;;
  *) fail "the worker never reached main — CreateThread's handle, or the RNG seed's shadow space (bug-478)" ;;
esac
case "$out" in
  *"readback:written by the worker"*) pass "a file written by the worker reads back with its contents" ;;
  *) fail "the file did not read back — CreateFileW's handle was the original defect, and it left a 0-byte file while reporting success" ;;
esac
case "$out" in
  *"env:probe-value"*) pass "a SET environment variable reads back its value" ;;
  *) fail "os::getEnvOr returned the wrong thing for a set variable — emit_env_get answering in the wrong return register is bug-479" ;;
esac
case "$out" in
  *"has:TRUE"*) pass "os::hasEnv sees a set variable" ;;
  *) fail "os::hasEnv missed a set variable" ;;
esac
case "$out" in
  *"noenv:MISSING"*) pass "an unset variable falls back" ;;
  *) fail "os::getEnvOr did not fall back for an unset variable" ;;
esac

if [ "$fails" -eq 0 ]; then
  echo "windows app-mode runtime tests passed"
else
  echo "$fails failure(s)"
  exit 1
fi
