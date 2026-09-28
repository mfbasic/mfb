#!/usr/bin/env bash
# Runtime leak check for the Windows backend's UTF-16 marshalling (bug-696).
#
# Every Win32 path and environment call converts its UTF-8 arguments to UTF-16 (and
# its results back) through arena buffers. Before bug-696 none were freed, so 100
# `os::getEnvOr` calls held 26 MB and 100 `fs::fileExists` calls 6.8 MB — with
# correct output, on the one target that marshals, so nothing else could see it.
#
# `cargo test` checks that each seam frees what it allocates
# (`marshal_ownership_tests`), but whether the SHARED consumer releases a result the
# seam handed back is only visible by running the program. So this builds one
# `--debug` probe, runs every mode on the Windows box once with 1 iteration and once
# with 25, and requires the same `arena.0.live_bytes` at exit: a per-call leak shows
# up as 24 × its size.
#
# Usage: scripts/test-winleak.sh <mfb-exe> [--box <port>]
#
#   --box <port>   ssh port of the Windows box (default 2230).
set -euo pipefail
. "$(dirname "$0")/remote-common.sh"

MFB_EXE="${1:?usage: test-winleak.sh <mfb-exe> [--box <port>]}"
shift || true
PORT=2230
rc_parse_box test-winleak "$@"

host="test@127.0.0.1"
remote='C:\mfbleak'
rc_workdir

# One mode per marshalling seam (and per shared consumer of a seam's result).
MODES="getEnvOr getEnv hasEnv setEnv environ hostName userName executablePath
appResourcePath appDataPath userHomePath fileExists directoryExists readText
writeText directory writeTextAtomic listDirectory canonicalPath currentDirectory
tempDirectory openFileNoFollow openWithin"

probe="$work/winleak"
scaffold_project "$probe" winleak
cat > "$probe/src/main.mfb" <<'MFB'
IMPORT collections
IMPORT fs
IMPORT io
IMPORT os
IMPORT strings

' One Win32 marshalling seam per mode. Every call's result is consumed and
' dropped, so the arena's live bytes at exit must not depend on how many calls
' ran.
FUNC once(which AS String, dir AS String) AS Nothing
  IF which = "getEnvOr" THEN
    IF os::getEnvOr("PATH", "") = "" THEN io::print("no PATH")
  ELSEIF which = "getEnv" THEN
    IF os::getEnv("PATH") = "" THEN io::print("no PATH")
  ELSEIF which = "hasEnv" THEN
    IF NOT os::hasEnv("PATH") THEN io::print("no PATH")
  ELSEIF which = "setEnv" THEN
    os::setEnv("MFB_LEAK_PROBE", "value")
    os::unsetEnv("MFB_LEAK_PROBE")
  ELSEIF which = "environ" THEN
    IF len(os::environ()) = 0 THEN io::print("empty environ")
  ELSEIF which = "hostName" THEN
    IF os::hostName() = "" THEN io::print("no host")
  ELSEIF which = "userName" THEN
    IF os::userName() = "" THEN io::print("no user")
  ELSEIF which = "executablePath" THEN
    IF os::executablePath() = "" THEN io::print("no exe")
  ELSEIF which = "appResourcePath" THEN
    IF os::appResourcePath("x") = "" THEN io::print("no resource path")
  ELSEIF which = "appDataPath" THEN
    IF os::appDataPath("x") = "" THEN io::print("no app data")
  ELSEIF which = "userHomePath" THEN
    IF os::userHomePath() = "" THEN io::print("no home")
  ELSEIF which = "fileExists" THEN
    IF NOT fs::fileExists(dir & "\\f.txt") THEN io::print("no file")
  ELSEIF which = "directoryExists" THEN
    IF NOT fs::directoryExists(dir) THEN io::print("no dir")
  ELSEIF which = "readText" THEN
    IF fs::readText(dir & "\\f.txt") <> "x" THEN io::print("bad read")
  ELSEIF which = "writeText" THEN
    fs::writeText(dir & "\\g.txt", "y")
    fs::deleteFile(dir & "\\g.txt")
  ELSEIF which = "directory" THEN
    fs::createDirectory(dir & "\\d")
    fs::deleteDirectory(dir & "\\d")
  ELSEIF which = "writeTextAtomic" THEN
    fs::writeTextAtomic(dir & "\\a.txt", "z")
  ELSEIF which = "listDirectory" THEN
    IF len(fs::listDirectory(dir)) = 0 THEN io::print("empty dir")
  ELSEIF which = "canonicalPath" THEN
    IF fs::canonicalPath(dir) = "" THEN io::print("no canonical")
  ELSEIF which = "currentDirectory" THEN
    fs::setCurrentDirectory(fs::currentDirectory())
  ELSEIF which = "tempDirectory" THEN
    IF fs::tempDirectory() = "" THEN io::print("no temp")
  ELSEIF which = "openFileNoFollow" THEN
    RES f = fs::openFileNoFollow(dir & "\\f.txt")
    IF fs::readAll(f) <> "x" THEN io::print("bad nofollow read")
  ELSEIF which = "openWithin" THEN
    RES f = fs::openWithin(dir, "f.txt")
    IF fs::readAll(f) <> "x" THEN io::print("bad within read")
  ELSE
    io::print("unknown mode " & which)
  END IF
END FUNC

' The buffers are sized to what they carry, so check what they carry: a value
' or a name with 2-, 3- and 4-byte UTF-8 sequences survives every marshalling
' direction byte for byte.
FUNC check(label AS String, ok AS Boolean) AS Nothing
  IF ok THEN
    io::print("check " & label & " ok")
  ELSE
    io::print("check " & label & " FAILED")
  END IF
END FUNC

FUNC values(dir AS String) AS Nothing
  LET v AS String = "h\u{E9}llo \u{4E16}\u{754C} \u{1F600}=x"
  os::setEnv("MFB_LEAK_VALUE", v)
  check("getEnv", os::getEnv("MFB_LEAK_VALUE") = v)
  check("getEnvOr", os::getEnvOr("MFB_LEAK_VALUE", "") = v)
  check("hasEnv", os::hasEnv("MFB_LEAK_VALUE"))
  LET env AS Map OF String TO String = os::environ()
  check("environ", collections::getOr(env, "MFB_LEAK_VALUE", "") = v)
  os::unsetEnv("MFB_LEAK_VALUE")
  check("unsetEnv", NOT os::hasEnv("MFB_LEAK_VALUE"))
  check("getEnvOr fallback", os::getEnvOr("MFB_LEAK_VALUE", "fb") = "fb")
  ' A variable set to "" is present, with an empty value (Win32 answers 0 for
  ' both that and unset).
  os::setEnv("MFB_LEAK_EMPTY", "")
  check("hasEnv empty", os::hasEnv("MFB_LEAK_EMPTY"))
  check("getEnvOr empty", os::getEnvOr("MFB_LEAK_EMPTY", "fb") = "")
  os::unsetEnv("MFB_LEAK_EMPTY")
  LET name AS String = "\u{E9}t\u{E9} \u{1F600}.txt"
  fs::writeText(dir & "\\" & name, v)
  check("fileExists", fs::fileExists(dir & "\\" & name))
  check("readText", fs::readText(dir & "\\" & name) = v)
  MUT listed AS Boolean = FALSE
  FOR EACH entry IN fs::listDirectory(dir)
    IF entry = name THEN listed = TRUE
  NEXT
  check("listDirectory", listed)
  LET canonical AS String = fs::canonicalPath(dir & "\\" & name)
  check("canonicalPath", strings::endsWith(canonical, "\\" & name))
  fs::deleteFile(dir & "\\" & name)
  check("deleteFile", NOT fs::fileExists(dir & "\\" & name))
  LET args AS List OF String = os::args()
  check("args", len(args) = 2 AND collections::get(args, 0) = "plain" AND collections::get(args, 1) = v)
  check("executablePath", strings::endsWith(os::executablePath(), "\\winleak.exe"))
END FUNC

FUNC main() AS Integer
  LET which AS String = os::getEnvOr("W", "none")
  MUT n AS Integer = 1
  IF os::getEnvOr("N", "1") <> "1" THEN n = 25
  LET dir AS String = fs::currentDirectory()
  IF which = "values" THEN
    values(dir)
    RETURN 0
  END IF
  fs::writeText(dir & "\\f.txt", "x")
  FOR i = 1 TO n
    once(which, dir)
  NEXT
  io::print("ran " & which & " " & toString(n))
  RETURN 0
END FUNC
MFB

echo "--- building the probe for windows-x86_64 (--debug) ---"
"$MFB_EXE" build -q --debug -target windows-x86_64 "$probe" >/dev/null

# `set X=v && prog` gives X a trailing space under cmd, so each assignment ends at
# `&&` with no space. Each run is bracketed `=== <mode> <n>` … `rc=<code>`, its
# stdout in between; the arena report is on stderr, of which only the live_bytes
# line is kept. `chcp 65001` makes cmd read this file's UTF-8 argument as UTF-8.
value=$'h\xc3\xa9llo \xe4\xb8\x96\xe7\x95\x8c \xf0\x9f\x98\x80=x'
{
  printf '@echo off\r\n'
  printf 'chcp 65001 >nul\r\n'
  printf 'cd /d %s\r\n' "$remote"
  for m in $MODES; do
    for n in 1 25; do
      printf 'echo === %s %s\r\n' "$m" "$n"
      printf 'set W=%s&& set N=%s&& winleak.exe 2>report.txt\r\n' "$m" "$n"
      printf 'echo rc=%%errorlevel%%\r\n'
      printf 'findstr /B /C:"arena.0.live_bytes " report.txt\r\n'
    done
  done
  printf 'echo === values 1\r\n'
  printf 'set W=values&& winleak.exe plain "%s" 2>report.txt\r\n' "$value"
  printf 'echo rc=%%errorlevel%%\r\n'
} > "$work/runner.bat"

echo "--- running on box $PORT ---"
win_ship "$PORT" "$host" "$remote" "$probe/build/winleak.exe" "$work/runner.bat"
out="$(remote_ssh "$PORT" "$host" "$remote\\runner.bat" 2>&1 | tr -d '\r' || true)"

# One run's lines from the transcript, `=== <mode> <n>` exclusive to the next `===`.
run() {
  printf '%s\n' "$out" | awk -v m="$1" -v n="$2" '
    $1 == "===" { hit = ($2 == m && $3 == n); next }
    hit { print }'
}

for m in $MODES; do
  for n in 1 25; do
    r="$(run "$m" "$n")"
    rc="$(printf '%s\n' "$r" | sed -n 's/^rc=//p')"
    printed="$(printf '%s\n' "$r" | grep -v -e '^rc=' -e '^arena\.0\.live_bytes ' || true)"
    live="$(printf '%s\n' "$r" | sed -n 's/^arena\.0\.live_bytes //p')"
    eval "rc_$n=\$rc printed_$n=\$printed live_$n=\$live"
  done
  if [ "$rc_1" != 0 ] || [ "$rc_25" != 0 ]; then
    fail "$m — the probe exited [$rc_1] / [$rc_25]"
  elif [ "$printed_1" != "ran $m 1" ] || [ "$printed_25" != "ran $m 25" ]; then
    fail "$m — wrong output: [$printed_1] / [$printed_25]"
  elif [ -z "$live_1" ] || [ -z "$live_25" ]; then
    fail "$m — no arena report"
  elif [ "$live_1" = "$live_25" ]; then
    pass "$m — $live_1 live bytes after 1 call and after 25"
  else
    fail "$m — $live_1 live bytes after 1 call, $live_25 after 25: $(( (live_25 - live_1) / 24 )) bytes leak per call"
  fi
done

# The values run: every `check` line must say ok, and there must be all 15.
r="$(run values 1)"
if [ "$(printf '%s\n' "$r" | sed -n 's/^rc=//p')" != 0 ]; then
  fail "values — the probe did not exit 0: $r"
fi
checks="$(printf '%s\n' "$r" | grep -c ' ok$' || true)"
if [ "$checks" = 15 ] && ! printf '%s\n' "$r" | grep -q 'FAILED'; then
  pass "values — 15 marshalled values survive byte for byte"
else
  fail "values — $checks of 15 checks ok; the run printed:"
  printf '%s\n' "$r" | sed 's/^/    /'
fi

if [ "$rc_failures" -eq 0 ]; then
  echo "windows marshalling leak check passed"
else
  echo "$rc_failures failure(s)"
  exit 1
fi
