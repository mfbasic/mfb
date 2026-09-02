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

SUB runEnv(label AS String, args AS List OF String, cwd AS String, env AS Map OF String TO String, replace AS Boolean)
  RES p = process::spawn(args, cwd, env, replace)
  WHILE TRUE
    LET line AS String = process::receive(p) TRAP(e)
      EXIT WHILE
    END TRAP
    io::print(label & ":" & strings::trimEnd(line))
  END WHILE
  io::print(label & ":rc=" & toString(process::waitFor(p)))
END SUB

SUB runShell(label AS String, cmd AS String)
  RES p = process::shell(cmd)
  WHILE TRUE
    LET line AS String = process::receive(p) TRAP(e)
      EXIT WHILE
    END TRAP
    io::print(label & ":" & strings::trimEnd(line))
  END WHILE
  io::print(label & ":rc=" & toString(process::waitFor(p)))
END SUB

SUB runShellStdin(label AS String, cmd AS String, a AS String, b AS String)
  RES p = process::shell(cmd)
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

  ' --- plan-119-A Phase 2: argv quoting. Each case is a boundary the bare-space
  ' joiner destroyed. argdump.exe reports what its CRT actually parsed, so
  ' `argc` alone distinguishes "one argument containing a space" from "two".
  runArgs("q1", ["argdump.exe", "a b", "c"])
  runArgs("q2", ["argdump.exe", "q\"uote", "plain"])
  runArgs("q3", ["argdump.exe", "", "after"])
  runArgs("q4", ["argdump.exe", "back\\slash", "tail\\"])
  runArgs("q5", ["argdump.exe", "C:\\my dir\\", "z"])

  ' --- plan-119-B: process::shell, which on Windows is `cmd.exe /S /C "<line>"`.
  ' Same five shell behaviours the spawn matrix drove through cmd by hand, now
  ' through the shell surface itself, plus the two quote edges `/S` exists to
  ' make deterministic.
  runShell("sh1", "echo one& echo two")
  runShell("sh2", "exit 7")
  runShell("sh3", "echo shelled>shellout.txt")
  runShell("sh4", "type shellout.txt")
  runShell("sh5", "(echo banana& echo apple) | sort")
  runShellStdin("sh6", "sort", "banana", "apple")
  ' A line that STARTS with a quote. Without /S, cmd re-guesses whether to keep
  ' the quotes by inspecting what is between them; with /S it always strips the
  ' first and last quote, so this runs the quoted program name.
  runShell("sh7", "\"cmd\" /C echo quoted")
  ' A line containing a quoted argument in the middle.
  runShell("sh8", "echo \"a b\"")

  ' --- plan-119-C: the four-argument spawn (cwd, env map, replace flag).
  ' `cmd /C cd` prints the working directory; `cmd /C set` prints the whole
  ' environment, which is what makes both PRESENCE and ABSENCE checkable — a
  ' wrong environment block is silently wrong otherwise.
  LET one AS Map OF String TO String = Map OF String TO String { "MFBPROBE" := "one" }
  LET two AS Map OF String TO String = Map OF String TO String { "MFBPROBE" := "two", "MFBOTHER" := "three" }
  LET pathish AS Map OF String TO String = Map OF String TO String { "path" := "MFB-OVERRIDE" }
  LET none AS Map OF String TO String = Map OF String TO String { }

  ' cwd: an explicit directory, and the empty string meaning "inherit".
  runEnv("e1", ["cmd.exe", "/C", "cd"], "C:\\Windows", none, TRUE)
  runEnv("e2", ["cmd.exe", "/C", "cd"], "", none, FALSE)

  ' replace: ONLY the map. PATH must be gone.
  runEnv("e3", ["cmd.exe", "/C", "set"], "", one, TRUE)
  ' replace with an EMPTY map: a block of two NULs, so the child sees nothing
  ' of ours. cmd still injects its own PROMPT/COMSPEC-style variables, so the
  ' assertion is that OUR probe is absent, not that `set` prints nothing.
  runEnv("e4", ["cmd.exe", "/C", "set"], "", none, TRUE)

  ' merge: the map wins, everything else is inherited.
  runEnv("e5", ["cmd.exe", "/C", "set"], "", two, FALSE)
  ' merge with a case-VARIANT key. Windows env names are case-insensitive, so a
  ' byte-exact skip would hand the child both `Path` and `path`.
  runEnv("e6", ["cmd.exe", "/C", "set"], "", pathish, FALSE)
END SUB
MFB

echo "--- building argdump + procprobe for windows-x86_64 ---"
"$MFB_EXE" build --target windows-x86_64 "$adump" >/dev/null
"$MFB_EXE" build --target windows-x86_64 "$probe" >/dev/null

cat > "$work/runner.bat" <<'BAT'
@echo off
cd /d C:\mfbproc
del /q procout.txt shellout.txt 2>nul
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

# --- plan-119-A Phase 2: the argv the child's CRT actually parsed.
#
# The `argc` assertions are the load-bearing ones. Before the fix, q1 reported
# `argc=3` with `arg=[a]` and `arg=[b]`, q2 collapsed to one `arg=[quote plain]`,
# and q3 lost the empty argument entirely.
expect "an argument containing a space stays ONE argument" "q1:argc=2" \
  "a bare-space join splits it; this is the shipped bug plan-119-A fixes"
expect "  ...and arrives with its space intact" "q1:arg=[a b]" "the wrap quotes leaked or the space was eaten"
expect "  ...and the next argument still follows" "q1:arg=[c]" "the separator was lost"
reject "  ...and it did NOT arrive split" "q1:arg=[a]" "the joiner is still splitting on the embedded space"
expect "an embedded quote survives as one argument" "q2:argc=2" "an unescaped quote merges the arguments"
expect "  ...with the quote itself delivered" "q2:arg=[q\"uote]" "the quote was dropped or doubled"
expect "  ...and the following argument intact" "q2:arg=[plain]" "the quote swallowed the next argument"
expect "an EMPTY argument survives" "q3:argc=2" "an empty argument vanished — it needs an explicit \"\" wrap"
expect "  ...as an empty string" "q3:arg=[]" "the empty argument came through as something else"
expect "  ...followed by the real one" "q3:arg=[after]" "the argument after the empty one was lost"
expect "backslashes with no space pass through literally" "q4:argc=2" "a backslash was treated as an escape outside a wrap"
expect "  ...mid-string" "q4:arg=[back\\slash]" "the backslash was doubled where it should stay literal"
expect "  ...and trailing" "q4:arg=[tail\\]" "a trailing backslash was doubled or eaten"
expect "a trailing backslash inside a QUOTED argument is doubled correctly" "q5:argc=2" \
  "the trailing backslash escaped the closing wrap quote and merged the arguments"
expect "  ...and the path arrives verbatim" "q5:arg=[C:\\my dir\\]" "the doubling leaked into the delivered value"
expect "  ...with the next argument intact" "q5:arg=[z]" "the run-away quote swallowed it"

# --- plan-119-B: process::shell over cmd.exe /S /C.
expect "shell sequenced two commands (first)" "sh1:one" "cmd.exe did not run the first command"
expect "shell sequenced two commands (second)" "sh1:two" "the '&' separator did not reach cmd"
expect "a shell child's exit code propagates" "sh2:rc=7" "waitFor lost the shell child's status"
expect "shell redirection wrote the file" "sh4:shelled" "'>' did not redirect, or 'type' could not read it back"
expect "a shell pipeline ran (sorted first)" "sh5:apple" "'|' did not reach cmd"
expect "a shell pipeline ran (sorted second)" "sh5:banana" "the pipeline produced only one line"
expect "stdin streamed into a shell filter (first)" "sh6:apple" "send/close did not reach the shell child's stdin"
expect "stdin streamed into a shell filter (second)" "sh6:banana" "the second send was lost"
# The two cases /S exists for. Without it, cmd's legacy heuristic decides whether
# to keep or strip the wrap quotes by inspecting the quote count, the characters
# between them, and whether the quoted text names an executable — so a
# quote-LEADING line is exactly where the two branches disagree.
expect "a quote-leading line runs (the /S choice)" "sh7:quoted" \
  "cmd kept the wrap quotes and could not find the program — /S is not in effect"
# `echo "a b"` in cmd prints the quotes — echo is literal. That is exactly what
# makes this a useful assertion: the inner quotes reached cmd THROUGH the outer
# `/S /C "…"` wrap and came back untouched. A wrap that swallowed or doubled them
# would print `a b`, or nothing at all.
expect "inner quotes reach cmd through the wrap" 'sh8:"a b"' \
  "the wrap quote and the inner quotes interfered"

# --- plan-119-C: cwd + environment block.
#
# Every environment case asserts PRESENCE **and** ABSENCE. A block that is merely
# plausible — right variables, wrong survivors — is the failure mode here, and
# only the absence checks can see it.
expect "an explicit cwd reaches the child" "e1:C:\\Windows" \
  "lpCurrentDirectory did not take effect"
expect "an empty cwd inherits the parent's" "e2:C:\\mfbproc" \
  "an empty cwd string must mean inherit, not chdir-to-nothing"
expect "replace mode delivers the map" "e3:MFBPROBE=one" "the replace block did not reach the child"
reject "replace mode drops the inherited environment" "e3:PATH=" \
  "envReplace=TRUE must hand the child ONLY the map — PATH survived"
reject "an empty replace map delivers nothing of ours" "e4:MFBPROBE=" \
  "an empty map must produce an empty block, not a stale or inherited one"
expect "  ...and the child still runs" "e4:rc=0" "the two-NUL empty block was rejected by CreateProcess"
expect "merge mode delivers the map (first key)" "e5:MFBPROBE=two" "a merged map entry is missing"
expect "merge mode delivers the map (second key)" "e5:MFBOTHER=three" "the second map entry is missing"
expect "merge mode keeps an inherited variable" "e5:SystemRoot=" \
  "envReplace=FALSE must keep what it did not override"
expect "a case-variant key overrides the inherited one" "e6:path=MFB-OVERRIDE" \
  "the map key did not win"
reject "  ...and the inherited spelling is NOT also present" "e6:Path=" \
  "the override skip is byte-exact, so the child got BOTH Path and path"

if [ "$fails" -eq 0 ]; then
  echo "windows process runtime tests passed"
else
  echo "$fails failure(s)"
  exit 1
fi
