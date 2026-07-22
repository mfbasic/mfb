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

# GUI cases open real windows (stealing focus) and, in one case, inject
# keystrokes via System Events into the focused app. They are OPT-IN so the
# default run never disrupts an interactive session. Enable with MFB_MACAPP_GUI=1
# (only when you are not actively using the machine).
gui_enabled() { [ "${MFB_MACAPP_GUI:-0}" = "1" ]; }

# The compiler writes app bundles under the project's build directory
# (src/os/mod.rs:BUILD_DIR, src/os/macos/link/mod.rs:write_app_bundle).
# Keep this the single source of that knowledge: a future layout change
# breaks one line here, not every case below.
bundle() { printf '%s' "$1/build/$2.app"; }

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

# Run headless and capture the program's stdout (the io sink in headless mode).
run_headless_stdout() {
  local exe=$1
  MFB_MACAPP_HEADLESS=1 perl -e '
    my $pid = open(my $fh, "-|");
    if ($pid == 0) { exec($ARGV[0]) or exit 127; }
    local $SIG{ALRM} = sub { kill "KILL", $pid; exit 99; };
    alarm 15; local $/; my $o = <$fh>; close($fh); print $o;
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
  result=$(run_headless "$(bundle "$proj" exitcode)/Contents/MacOS/exitcode")
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
  result=$(run_headless "$(bundle "$proj" nothing)/Contents/MacOS/nothing")
  if [ "$result" = "code=0" ]; then
    echo "ok: SUB main() worker ran and exited cleanly ($result)"
  else
    echo "FAIL: expected code=0, got '$result'" >&2
    failures=$((failures + 1))
  fi
fi

# Case 3: app-mode io output. Headless leaves no transcript view attached, so the
# io helpers fall back to the file descriptor sink (plan §7.2 Strategy A) where
# the output is observable. Proves the app-mode print/write helpers run and
# format correctly (print adds a newline, write does not).
proj="$work/output"
mkdir -p "$proj/src"
cat > "$proj/project.json" <<'JSON'
{ "name": "output", "version": "0.1.0", "mfb": "1.0", "kind": "executable",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main", "targets": ["native"] }
JSON
cat > "$proj/src/main.mfb" <<'MFB'
IMPORT io
SUB main()
  io::print("APPMODE_LINE")
  io::write("APPMODE_NONL")
END SUB
MFB
if ! "$MFB_EXE" build -app "$proj" >/dev/null 2>&1; then
  echo "FAIL: build -app output" >&2
  failures=$((failures + 1))
else
  out=$(run_headless_stdout "$(bundle "$proj" output)/Contents/MacOS/output")
  if [ "$out" = $'APPMODE_LINE\nAPPMODE_NONL' ]; then
    echo "ok: app-mode io::print/io::write produced expected output"
  else
    echo "FAIL: unexpected app-mode io output: $(printf '%q' "$out")" >&2
    failures=$((failures + 1))
  fi
fi

# Case 4 (GUI): keep window open after completion (plan §5.7). Launched WITHOUT
# the headless gate so the real window + event loop run; a program whose main
# returns immediately must leave the process alive (window open) rather than
# exiting. This briefly opens a window and requires a window-server session.
proj="$work/keepopen"
mkdir -p "$proj/src"
cat > "$proj/project.json" <<'JSON'
{ "name": "keepopen", "version": "0.1.0", "mfb": "1.0", "kind": "executable",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main", "targets": ["native"] }
JSON
cat > "$proj/src/main.mfb" <<'MFB'
IMPORT io
SUB main()
  io::print("finished")
END SUB
MFB
if ! gui_enabled; then
  echo "skip: keep-window-open GUI test (set MFB_MACAPP_GUI=1 when idle)"
elif ! "$MFB_EXE" build -app "$proj" >/dev/null 2>&1; then
  echo "FAIL: build -app keepopen" >&2
  failures=$((failures + 1))
else
  result=$(perl -e '
    use POSIX ":sys_wait_h";
    my $pid = fork();
    if ($pid == 0) {
      open(STDOUT, ">", "/dev/null"); open(STDERR, ">", "/dev/null");
      exec($ARGV[0]) or exit 127;
    }
    sleep 4;
    my $r = waitpid($pid, WNOHANG);
    if ($r == 0) { print "alive"; kill "KILL", $pid; waitpid($pid, 0); }
    else { printf "exited=%d", ($? >> 8); }
  ' "$(bundle "$proj" keepopen)/Contents/MacOS/keepopen")
  if [ "$result" = "alive" ]; then
    echo "ok: window stayed open after the program finished"
  else
    echo "FAIL: app did not keep the window open ($result)" >&2
    failures=$((failures + 1))
  fi
fi

# Case 5: app-mode input. Headless leaves fd 0 as real stdin (no window input
# pipe), so io::input/io::readLine read fed input and io::input's prompt goes to
# the fd sink. Proves the app-mode io.input composition (prompt via io.write +
# read via io.readLine) and that the read helpers work in app mode. (The GUI
# input field -> pipe path is manual, plan §7.4.)
proj="$work/input"
mkdir -p "$proj/src"
cat > "$proj/project.json" <<'JSON'
{ "name": "input", "version": "0.1.0", "mfb": "1.0", "kind": "executable",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main", "targets": ["native"] }
JSON
cat > "$proj/src/main.mfb" <<'MFB'
IMPORT io
SUB main()
  LET name AS String = io::input("Name? ")
  io::print("Hi " & name)
  LET line AS String = io::readLine()
  io::print("Echo " & line)
END SUB
MFB
if ! "$MFB_EXE" build -app "$proj" >/dev/null 2>&1; then
  echo "FAIL: build -app input" >&2
  failures=$((failures + 1))
else
  out=$(printf 'bob\nsecond\n' | MFB_MACAPP_HEADLESS=1 perl -e '
    my $pid = open(my $fh, "-|");
    if ($pid == 0) { exec($ARGV[0]) or exit 127; }
    local $SIG{ALRM} = sub { kill "KILL", $pid; exit 99; };
    alarm 15; local $/; my $o = <$fh>; close($fh); print $o;
  ' "$(bundle "$proj" input)/Contents/MacOS/input")
  if [ "$out" = $'Name? Hi bob\nEcho second' ]; then
    echo "ok: app-mode io::input + io::readLine consume input correctly"
  else
    echo "FAIL: unexpected app-mode input output: $(printf '%q' "$out")" >&2
    failures=$((failures + 1))
  fi
fi

# Case 5b (bug-247): app-mode io::input WITHOUT any io::readLine call. Case 5
# calls io::readLine too, which fires the readLine import row and declares the
# terminal probes (_isatty/_tcgetattr) that the composed readLine body needs --
# masking a build that would otherwise fail with "runtime helper requires
# _isatty import". Keep this case free of io::readLine.
proj="$work/inputonly"
mkdir -p "$proj/src"
cat > "$proj/project.json" <<'JSON'
{ "name": "inputonly", "version": "0.1.0", "mfb": "1.0", "kind": "executable",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main", "targets": ["native"] }
JSON
cat > "$proj/src/main.mfb" <<'MFB'
IMPORT io
SUB main()
  LET name AS String = io::input("Name? ")
  io::print("Hi " & name)
END SUB
MFB
if ! "$MFB_EXE" build -app "$proj" >/dev/null 2>&1; then
  echo "FAIL: build -app inputonly (bug-247: missing _isatty/_tcgetattr imports?)" >&2
  failures=$((failures + 1))
else
  out=$(printf 'bob\n' | MFB_MACAPP_HEADLESS=1 perl -e '
    my $pid = open(my $fh, "-|");
    if ($pid == 0) { exec($ARGV[0]) or exit 127; }
    local $SIG{ALRM} = sub { kill "KILL", $pid; exit 99; };
    alarm 15; local $/; my $o = <$fh>; close($fh); print $o;
  ' "$(bundle "$proj" inputonly)/Contents/MacOS/inputonly")
  if [ "$out" = 'Name? Hi bob' ]; then
    echo "ok: app-mode io::input alone (no io::readLine) builds and reads"
  else
    echo "FAIL: unexpected app-mode input-only output: $(printf '%q' "$out")" >&2
    failures=$((failures + 1))
  fi
fi

# Case 6 (GUI): terminal-style window input. Launch a real app, inject keystrokes
# into the window via System Events, and confirm the program's io::readLine read
# them (the program writes what it read to a file). Best-effort: keystroke
# injection needs Accessibility permission for the launching process, so a
# non-delivery is reported as a skip rather than a failure.
proj="$work/keyinput"
mkdir -p "$proj/src"
cat > "$proj/project.json" <<'JSON'
{ "name": "keyinput", "version": "0.1.0", "mfb": "1.0", "kind": "executable",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main", "targets": ["native"] }
JSON
cat > "$proj/src/main.mfb" <<MFB
IMPORT io
IMPORT fs
SUB main()
  LET name AS String = io::readLine()
  fs::writeText("$proj/got.txt", "got:" & name)
END SUB
MFB
if ! gui_enabled; then
  echo "skip: window keystroke GUI test (set MFB_MACAPP_GUI=1 when idle)"
elif ! "$MFB_EXE" build -app "$proj" >/dev/null 2>&1; then
  echo "FAIL: build -app keyinput" >&2
  failures=$((failures + 1))
else
  rm -f "$proj/got.txt"
  open "$(bundle "$proj" keyinput)"
  sleep 2
  osascript -e 'tell application "System Events" to keystroke "WindowKeys"' >/dev/null 2>&1
  osascript -e 'tell application "System Events" to key code 36' >/dev/null 2>&1
  sleep 1
  pkill -KILL keyinput >/dev/null 2>&1
  got=$(cat "$proj/got.txt" 2>/dev/null || true)
  if [ "$got" = "got:WindowKeys" ]; then
    echo "ok: window keypresses delivered to io::readLine"
  else
    echo "skip: window keystroke injection unavailable (need Accessibility); got '$got'"
  fi
fi

# Case 7: app-mode io::is*Terminal -> TRUE (plan §5.4). The window is the
# interactive console, so all three return TRUE even headless.
proj="$work/isterm"
mkdir -p "$proj/src"
cat > "$proj/project.json" <<'JSON'
{ "name": "isterm", "version": "0.1.0", "mfb": "1.0", "kind": "executable",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main", "targets": ["native"] }
JSON
cat > "$proj/src/main.mfb" <<'MFB'
IMPORT io
SUB main()
  IF io::isInputTerminal() AND io::isOutputTerminal() AND io::isErrorTerminal() THEN
    io::print("terminal:yes")
  ELSE
    io::print("terminal:no")
  END IF
END SUB
MFB
if ! "$MFB_EXE" build -app "$proj" >/dev/null 2>&1; then
  echo "FAIL: build -app isterm" >&2
  failures=$((failures + 1))
else
  out=$(MFB_MACAPP_HEADLESS=1 perl -e '
    my $pid = open(my $fh, "-|"); if ($pid == 0) { exec($ARGV[0]) or exit 127; }
    local $SIG{ALRM} = sub { kill "KILL", $pid; exit 99 }; alarm 10;
    local $/; my $o = <$fh>; close($fh); chomp $o; print $o;
  ' "$(bundle "$proj" isterm)/Contents/MacOS/isterm")
  if [ "$out" = "terminal:yes" ]; then
    echo "ok: app-mode io::is*Terminal return TRUE"
  else
    echo "FAIL: io::is*Terminal expected terminal:yes, got '$out'" >&2
    failures=$((failures + 1))
  fi
fi

# Case 8 (GUI): term::terminalSize reports the TermView surface grid.
# Launch a real window; the program writes the reported columns/rows to a file.
# This case used io::terminalSize until plan-01-term Phase 3 removed that
# builtin; because the whole case is GUI-gated it kept being skipped, so the
# stale source went unnoticed and MFB_MACAPP_GUI=1 failed at "build -app tsize".
# term::terminalSize is gated behind TUI mode, hence the term::on() first.
proj="$work/tsize"
mkdir -p "$proj/src"
cat > "$proj/project.json" <<'JSON'
{ "name": "tsize", "version": "0.1.0", "mfb": "1.0", "kind": "executable",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main", "targets": ["native"] }
JSON
cat > "$proj/src/main.mfb" <<MFB
IMPORT term
IMPORT fs
SUB main()
  term::on()
  LET s AS TermSize = term::terminalSize()
  term::off()
  fs::writeText("$proj/size.txt", toString(s.columns) & "x" & toString(s.rows))
END SUB
MFB
if ! gui_enabled; then
  echo "skip: terminalSize GUI test (set MFB_MACAPP_GUI=1 when idle)"
elif ! "$MFB_EXE" build -app "$proj" >/dev/null 2>&1; then
  echo "FAIL: build -app tsize" >&2
  failures=$((failures + 1))
else
  rm -f "$proj/size.txt"
  open "$(bundle "$proj" tsize)"
  sleep 2
  pkill -KILL tsize >/dev/null 2>&1
  size=$(cat "$proj/size.txt" 2>/dev/null || true)
  if printf '%s' "$size" | grep -Eq '^[1-9][0-9]*x[1-9][0-9]*$'; then
    echo "ok: term::terminalSize reported window surface ($size)"
  else
    echo "skip: term::terminalSize window check unavailable (need GUI session); got '$size'"
  fi
fi

if [ "$failures" -ne 0 ]; then
  echo "macOS app mode runtime tests failed: $failures" >&2
  exit 1
fi
echo "macOS app mode runtime tests passed"
