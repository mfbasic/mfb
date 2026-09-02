#!/usr/bin/env bash
# Runtime acceptance for the Windows `process` backend (plan-119).
#
# The sibling of `test-winapp.sh`, and it exists for the same reason: **`cargo test`
# never executes a PE.** It only compiles them, so every claim about what a Windows
# child actually receives — its argv, its environment, its working directory — is a
# guess until a real Windows box runs it. plan-119-A's first measurement is the
# proof: `spawn(["argdump.exe", "a b", "c"])` had been shipping `argc=3` /
# `arg=[a]` / `arg=[b]` for as long as the backend existed, and a green suite on a
# macOS host could not see it.
#
# This drives the **console** entry path (test-winapp.sh drives `--app`); the two
# differ by exactly 8 bytes of stack alignment, so they get separate scripts.
#
# Usage: scripts/test-winprocess.sh <mfb-exe> [--box <port>]
#
#   --box <port>   ssh port of the Windows box (default 2230).
set -euo pipefail

MFB_EXE="${1:?usage: test-winprocess.sh <mfb-exe> [--box <port>]}"
shift || true
PORT=2230
while [ $# -gt 0 ]; do
  case "$1" in
    --box) PORT="$2"; shift 2 ;;
    *) echo "test-winprocess: unknown argument $1" >&2; exit 2 ;;
  esac
done

host="test@127.0.0.1"
remote='C:\mfbproc'
fails=0
pass() { echo "ok: $1"; }
fail() { echo "FAIL: $1"; fails=$((fails + 1)); }

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# ---------------------------------------------------------------------------
# argdump.exe — the child whose whole job is to report the argv it was handed.
#
# `os::args()` returns the arguments after the program name, which is exactly the
# vector the child's CRT parsed out of the single command-line string
# `CreateProcessA` was given. It is therefore a direct readout of the joiner: a
# quoting bug shows up as a different `argc`, not as a subtly wrong byte.
adump="$work/argdump"
mkdir -p "$adump/src"
cat > "$adump/project.json" <<'JSON'
{ "name": "argdump", "version": "0.1.0", "mfb": "1.0", "kind": "executable",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main", "targets": ["native"] }
JSON
cat > "$adump/src/main.mfb" <<'MFB'
IMPORT io
IMPORT os

SUB main()
  LET a AS List OF String = os::args()
  io::print("argc=" & toString(len(a)))
  FOR EACH s IN a
    io::print("arg=[" & s & "]")
  NEXT
END SUB
MFB

# ---------------------------------------------------------------------------
# procprobe.exe — the parent. Every case prints `<label>:<line>` for each line the
# child produced and then `<label>:rc=<code>`, so the assertions below are plain
# substring matches on a stable vocabulary.
#
# `strings::trimEnd` is deliberate: a Windows child's lines end `\r\n` and
# `process::receive` keeps the `\r` (there is no CR handling in the receive path).
# That is pre-existing, documented behavior — the probe normalizes it rather than
# pretending it does not happen.
probe="$work/procprobe"
mkdir -p "$probe/src"
cat > "$probe/project.json" <<'JSON'
{ "name": "procprobe", "version": "0.1.0", "mfb": "1.0", "kind": "executable",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main", "targets": ["native"] }
JSON
cat > "$probe/src/main.mfb" <<'MFB'
IMPORT io
IMPORT process
IMPORT strings

SUB runArgs(label AS String, args AS List OF String)
  RES p = process::spawn(args)
  WHILE TRUE
    LET line AS String = process::receive(p) TRAP(e)
      EXIT WHILE
    END TRAP
    io::print(label & ":" & strings::trimEnd(line))
  END WHILE
  io::print(label & ":rc=" & toString(process::waitFor(p)))
END SUB

SUB runStdin(label AS String, args AS List OF String, a AS String, b AS String)
  RES p = process::spawn(args)
  process::send(p, a)
  process::send(p, b)
  process::close(p)
  WHILE TRUE
    LET line AS String = process::receive(p) TRAP(e)
      EXIT WHILE
    END TRAP
    io::print(label & ":" & strings::trimEnd(line))
  END WHILE
  io::print(label & ":rc=" & toString(process::waitFor(p)))
END SUB

SUB main()
  ' --- plan-119-A Phase 1: the cmd.exe matrix the research spike proved, re-run
  ' through the shared spawn tail. Command sequencing, exit-code propagation,
  ' redirection, a pipeline, and stdin streamed into a filter.
  runArgs("seq", ["cmd.exe", "/C", "echo one& echo two"])
  runArgs("exit", ["cmd.exe", "/C", "exit 7"])
  runArgs("redir", ["cmd.exe", "/C", "echo filed>procout.txt"])
  runArgs("cat", ["cmd.exe", "/C", "type procout.txt"])
  runArgs("pipe", ["cmd.exe", "/C", "(echo banana& echo apple) | sort"])
  runStdin("stdin", ["cmd.exe", "/C", "sort"], "banana", "apple")
END SUB
MFB

echo "--- building argdump + procprobe for windows-x86_64 ---"
"$MFB_EXE" build --target windows-x86_64 "$adump" >/dev/null
"$MFB_EXE" build --target windows-x86_64 "$probe" >/dev/null

cat > "$work/runner.bat" <<'BAT'
@echo off
cd /d C:\mfbproc
del /q procout.txt 2>nul
procprobe.exe 2>&1
echo rc=%errorlevel%
BAT

echo "--- running on box $PORT ---"
ssh -p "$PORT" "$host" "if not exist $remote mkdir $remote" >/dev/null
scp -P "$PORT" "$adump/build/argdump.exe" "$host:C:/mfbproc/argdump.exe" >/dev/null
scp -P "$PORT" "$probe/build/procprobe.exe" "$host:C:/mfbproc/procprobe.exe" >/dev/null
scp -P "$PORT" "$work/runner.bat" "$host:C:/mfbproc/runner.bat" >/dev/null
out="$(ssh -p "$PORT" "$host" "$remote\\runner.bat" 2>&1 || true)"
echo "$out" | sed 's/^/    /'

# `expect <label> <needle> <why>` — a substring assertion over the whole transcript.
expect() {
  case "$out" in
    *"$2"*) pass "$1" ;;
    *) fail "$1 — expected [$2]; $3" ;;
  esac
}
reject() {
  case "$out" in
    *"$2"*) fail "$1 — [$2] must NOT appear; $3" ;;
    *) pass "$1" ;;
  esac
}

expect "the probe exited cleanly" "rc=0" "a nonzero exit means a case raised past main"
expect "cmd sequenced two commands (first)" "seq:one" "cmd.exe /C did not run the first command"
expect "cmd sequenced two commands (second)" "seq:two" "the '&' separator did not reach cmd"
expect "a child's exit code propagates" "exit:rc=7" "waitFor lost the child's status"
expect "redirection wrote the file" "cat:filed" "'>' did not redirect, or 'type' could not read it back"
expect "a pipeline ran (sorted first)" "pipe:apple" "'|' did not reach cmd"
expect "a pipeline ran (sorted second)" "pipe:banana" "the pipeline produced only one line"
expect "stdin streamed into a filter (sorted first)" "stdin:apple" "send/close did not reach the child's stdin"
expect "stdin streamed into a filter (sorted second)" "stdin:banana" "the second send was lost"

if [ "$fails" -eq 0 ]; then
  echo "windows process runtime tests passed"
else
  echo "$fails failure(s)"
  exit 1
fi
