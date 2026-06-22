#!/usr/bin/env bash
# Runtime acceptance for macOS app mode (plan-04-macos-app.md §7.2).
#
# Builds an app-mode `.app` bundle and launches its executable headlessly
# (MFB_MACAPP_HEADLESS=1) so the same AppKit construction + worker-thread code
# the GUI path uses runs without showing a window or blocking on the event loop.
# Proves: the Objective-C runtime / AppKit / Foundation bind and run, the worker
# thread executes the MFBASIC program entry, and the program's exit code
# propagates through the worker.
#
# Requires macOS with a window-server session (AppKit's NSApplication needs one).
#
# Usage: scripts/test-macapp.sh <mfb-exe>
set -u

if [ "$#" -lt 1 ]; then
  echo "usage: test-macapp.sh <mfb-exe>" >&2
  exit 2
fi
MFB_EXE=$1
ROOT=$(cd "$(dirname "$0")/.." && pwd)

if [ "$(uname -s)" != "Darwin" ]; then
  echo "skip: macOS app mode runtime test requires macOS" >&2
  exit 0
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
failures=0

# Run a bundle's executable headlessly with a watchdog; echo "code=N" or "signal=N".
run_headless() {
  local exe=$1
  MFB_MACAPP_HEADLESS=1 perl -e '
    my $pid = fork();
    if ($pid == 0) { exec($ARGV[0]) or exit 127; }
    local $SIG{ALRM} = sub { kill "KILL", $pid; print "timeout\n"; waitpid($pid,0); exit 99; };
    alarm 15; waitpid($pid, 0); my $st = $?;
    if ($st & 127) { printf "signal=%d\n", ($st & 127); }
    else { printf "code=%d\n", ($st >> 8); }
  ' "$exe"
}

# Case 1: FUNC main() AS Integer returns 42 -> process exits 42 (worker ran it).
proj="$work/exitcode"
mkdir -p "$proj/src"
cat > "$proj/project.json" <<'JSON'
{ "name": "exitcode", "version": "0.1.0", "mfb": "1.0", "kind": "executable",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main", "targets": ["native"] }
JSON
cat > "$proj/src/main.mfb" <<'MFB'
FUNC main() AS Integer
  RETURN 42
END FUNC
MFB
if ! "$MFB_EXE" build -app "$proj" >/dev/null 2>&1; then
  echo "FAIL: build -app exitcode" >&2
  failures=$((failures + 1))
else
  result=$(run_headless "$proj/exitcode.app/Contents/MacOS/exitcode")
  if [ "$result" = "code=42" ]; then
    echo "ok: worker ran program and propagated exit code ($result)"
  else
    echo "FAIL: expected code=42, got '$result'" >&2
    failures=$((failures + 1))
  fi
fi

# Case 2: SUB main() runs to completion -> process exits 0.
proj="$work/nothing"
mkdir -p "$proj/src"
cat > "$proj/project.json" <<'JSON'
{ "name": "nothing", "version": "0.1.0", "mfb": "1.0", "kind": "executable",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main", "targets": ["native"] }
JSON
cat > "$proj/src/main.mfb" <<'MFB'
SUB main()
END SUB
MFB
if ! "$MFB_EXE" build -app "$proj" >/dev/null 2>&1; then
  echo "FAIL: build -app nothing" >&2
  failures=$((failures + 1))
else
  result=$(run_headless "$proj/nothing.app/Contents/MacOS/nothing")
  if [ "$result" = "code=0" ]; then
    echo "ok: SUB main() worker ran and exited cleanly ($result)"
  else
    echo "FAIL: expected code=0, got '$result'" >&2
    failures=$((failures + 1))
  fi
fi

if [ "$failures" -ne 0 ]; then
  echo "macOS app mode runtime tests failed: $failures" >&2
  exit 1
fi
echo "macOS app mode runtime tests passed"
