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

# Refuse to run concurrently with another test-macapp — concurrent runs thrash
# disk/CPU building the .app bundle and race for the window-server session two
# headless NSApplication launches would contend over.
#
# `pgrep -f` matches our OWN transient children too: the subshells/pipeline
# members bash fork()s for a `$(...)` still carry the parent
# `bash scripts/test-macapp.sh …` command line before they exec(). Excluding
# only `$$` (the main shell) missed those and reported a phantom "pid N" with no
# real concurrent run. Instead skip every candidate sharing our process group —
# our children inherit our PGID at fork() (excluded even mid-race), a separate
# run is launched into its own session/group. An already-exited candidate (empty
# PGID) is not a live run.
mypgid=$(ps -o pgid= -p "$$" | tr -d ' ')
other=""
for pid in $(pgrep -f 'test-macapp\.sh'); do
  [ "$pid" = "$$" ] && continue
  cpgid=$(ps -o pgid= -p "$pid" 2>/dev/null | tr -d ' ')
  [ -z "$cpgid" ] && continue
  [ "$cpgid" = "$mypgid" ] && continue
  other=$pid
  break
done
if [ -n "$other" ]; then
  echo "Another test-macapp (pid $other) is running." >&2
  exit 1
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
failures=0

# Result reporting, matching test-appimage.sh so the two macOS/Linux runtime
# gates read the same. `fail` is the single place `failures` is incremented.
pass() { echo "ok: $1"; }
fail() { echo "FAIL: $1" >&2; failures=$((failures + 1)); }

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

# Run a bundle's executable headlessly with a watchdog; echo "code=N" or
# "signal=N". The watchdog bound is a parameter (default 15s), matching
# test-appimage.sh's `timeout_run` so the two gates cannot diverge silently.
run_headless() {
  local exe=$1 limit=${2:-15}
  MFB_MACAPP_HEADLESS=1 perl -e '
    my $limit = shift @ARGV;
    my $pid = fork();
    if ($pid == 0) { exec($ARGV[0]) or exit 127; }
    local $SIG{ALRM} = sub { kill "KILL", $pid; print "timeout\n"; waitpid($pid,0); exit 99; };
    alarm $limit; waitpid($pid, 0); my $st = $?;
    if ($st & 127) { printf "signal=%d\n", ($st & 127); }
    else { printf "code=%d\n", ($st >> 8); }
  ' "$limit" "$exe"
}

# Run headless and capture the program's stdout (the io sink in headless mode).
# stdin is inherited, so a caller can pipe fed input in. Bound is a parameter
# (default 15s). Callers use `$(...)`, which strips the trailing newline.
run_headless_stdout() {
  local exe=$1 limit=${2:-15}
  MFB_MACAPP_HEADLESS=1 perl -e '
    my $limit = shift @ARGV;
    my $pid = open(my $fh, "-|");
    if ($pid == 0) { exec($ARGV[0]) or exit 127; }
    local $SIG{ALRM} = sub { kill "KILL", $pid; exit 99; };
    alarm $limit; local $/; my $o = <$fh>; close($fh); print $o;
  ' "$limit" "$exe"
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
  fail "build -app exitcode"
else
  result=$(run_headless "$(bundle "$proj" exitcode)/Contents/MacOS/exitcode")
  if [ "$result" = "code=42" ]; then
    pass "worker ran program and propagated exit code ($result)"
  else
    fail "expected code=42, got '$result'"
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
  fail "build -app nothing"
else
  result=$(run_headless "$(bundle "$proj" nothing)/Contents/MacOS/nothing")
  if [ "$result" = "code=0" ]; then
    pass "SUB main() worker ran and exited cleanly ($result)"
  else
    fail "expected code=0, got '$result'"
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
  fail "build -app output"
else
  out=$(run_headless_stdout "$(bundle "$proj" output)/Contents/MacOS/output")
  if [ "$out" = $'APPMODE_LINE\nAPPMODE_NONL' ]; then
    pass "app-mode io::print/io::write produced expected output"
  else
    fail "unexpected app-mode io output: $(printf '%q' "$out")"
  fi
fi

# Case 3b (plan-62-B): app:: presentation-mode state. getMode/setMode read and
# write the per-arena presentation-mode slot; the static default is Console unless
# the program references app::setMode (then None). Proven headlessly through the
# worker's exit code (0 = all assertions held).
#
#   - default Console: a program that never references setMode observes Console at
#     startup (the slot zero-inits to 0 = Console).
proj="$work/appdefault"
mkdir -p "$proj/src"
cat > "$proj/project.json" <<'JSON'
{ "name": "appdefault", "version": "0.1.0", "mfb": "1.0", "kind": "executable",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main", "targets": ["native"] }
JSON
cat > "$proj/src/main.mfb" <<'MFB'
IMPORT app
FUNC main() AS Integer
  IF app::getMode() = Mode.Console THEN
    RETURN 0
  END IF
  RETURN 1
END FUNC
MFB
if ! "$MFB_EXE" build -app "$proj" >/dev/null 2>&1; then
  fail "build -app appdefault"
else
  result=$(run_headless "$(bundle "$proj" appdefault)/Contents/MacOS/appdefault")
  if [ "$result" = "code=0" ]; then
    pass "app:: default presentation mode is Console ($result)"
  else
    fail "expected app default Console (code=0), got '$result'"
  fi
fi

#   - setMode round-trip: setMode(None) then getMode observes None; setMode(Console)
#     then getMode observes Console.
proj="$work/approundtrip"
mkdir -p "$proj/src"
cat > "$proj/project.json" <<'JSON'
{ "name": "approundtrip", "version": "0.1.0", "mfb": "1.0", "kind": "executable",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main", "targets": ["native"] }
JSON
cat > "$proj/src/main.mfb" <<'MFB'
IMPORT app
FUNC main() AS Integer
  app::setMode(Mode.None)
  IF app::getMode() = Mode.Console THEN
    RETURN 1
  END IF
  app::setMode(Mode.Console)
  IF app::getMode() = Mode.None THEN
    RETURN 2
  END IF
  RETURN 0
END FUNC
MFB
if ! "$MFB_EXE" build -app "$proj" >/dev/null 2>&1; then
  fail "build -app approundtrip"
else
  result=$(run_headless "$(bundle "$proj" approundtrip)/Contents/MacOS/approundtrip")
  if [ "$result" = "code=0" ]; then
    pass "app::setMode/getMode round-trip through the mode slot ($result)"
  else
    fail "expected app round-trip (code=0), got '$result'"
  fi
fi

#   - None static default: a program that references setMode anywhere starts in
#     None, observable at the very first statement (the entry seeds the slot to 1).
proj="$work/appnone"
mkdir -p "$proj/src"
cat > "$proj/project.json" <<'JSON'
{ "name": "appnone", "version": "0.1.0", "mfb": "1.0", "kind": "executable",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main", "targets": ["native"] }
JSON
cat > "$proj/src/main.mfb" <<'MFB'
IMPORT app
FUNC main() AS Integer
  IF app::getMode() = Mode.None THEN
    app::setMode(Mode.Console)
    RETURN 0
  END IF
  RETURN 1
END FUNC
MFB
if ! "$MFB_EXE" build -app "$proj" >/dev/null 2>&1; then
  fail "build -app appnone"
else
  result=$(run_headless "$(bundle "$proj" appnone)/Contents/MacOS/appnone")
  if [ "$result" = "code=0" ]; then
    pass "app:: static default is None when setMode is referenced ($result)"
  else
    fail "expected app None static default (code=0), got '$result'"
  fi
fi

# Case 3c (plan-62-E): mode gating. In an app build, `term::*` and the console-read
# `io::` calls require the `Console` presentation mode; outside it they raise the
# trappable `ErrWrongMode`. `io::print`/`io::write` are never gated. Proven headlessly
# through the worker's exit code.
#
#   - term::moveTo in None traps ErrWrongMode; the same call in Console does not.
proj="$work/wrongmode_term"
mkdir -p "$proj/src"
cat > "$proj/project.json" <<'JSON'
{ "name": "wrongmode_term", "version": "0.1.0", "mfb": "1.0", "kind": "executable",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main", "targets": ["native"] }
JSON
cat > "$proj/src/main.mfb" <<'MFB'
IMPORT app
IMPORT term
IMPORT errorCode
FUNC main AS Integer
  app::setMode(Mode.None)
  term::moveTo(1, 1) TRAP(err)
    IF err.code = errorCode::ErrWrongMode THEN
      app::setMode(Mode.Console)
      term::moveTo(1, 1) TRAP(err2)
        RETURN 61       ' Console must NOT trap
      END TRAP
      RETURN 0          ' None trapped, Console succeeded
    END IF
    RETURN 60           ' trapped, wrong code
  END TRAP
  RETURN 50             ' None must trap
END FUNC
MFB
if ! "$MFB_EXE" build -app "$proj" >/dev/null 2>&1; then
  fail "build -app wrongmode_term"
else
  result=$(run_headless "$(bundle "$proj" wrongmode_term)/Contents/MacOS/wrongmode_term")
  if [ "$result" = "code=0" ]; then
    pass "term:: raises ErrWrongMode outside Console, works in Console ($result)"
  else
    fail "expected term wrong-mode gate (code=0), got '$result'"
  fi
fi

#   - io::readLine in None traps ErrWrongMode; io::print is never gated (it printed).
proj="$work/wrongmode_io"
mkdir -p "$proj/src"
cat > "$proj/project.json" <<'JSON'
{ "name": "wrongmode_io", "version": "0.1.0", "mfb": "1.0", "kind": "executable",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main", "targets": ["native"] }
JSON
cat > "$proj/src/main.mfb" <<'MFB'
IMPORT app
IMPORT io
IMPORT errorCode
FUNC main AS Integer
  app::setMode(Mode.None)
  LET line AS String = io::readLine() TRAP(err)
    IF err.code = errorCode::ErrWrongMode THEN
      RETURN 0
    END IF
    RETURN 60
  END TRAP
  RETURN 50
END FUNC
MFB
if ! "$MFB_EXE" build -app "$proj" >/dev/null 2>&1; then
  fail "build -app wrongmode_io"
else
  result=$(run_headless "$(bundle "$proj" wrongmode_io)/Contents/MacOS/wrongmode_io")
  if [ "$result" = "code=0" ]; then
    pass "io::readLine raises ErrWrongMode outside Console ($result)"
  else
    fail "expected io wrong-mode gate (code=0), got '$result'"
  fi
fi

# Case 3d (plan-62-C Phase 2, GUI): the runtime setMode reconcile flips io routing.
# A None-start program prints to stdout (no window), then setMode(Console) builds +
# shows the transcript window on the main thread, so a following io::print lands in
# the transcript, NOT stdout. Verified by capturing stdout of a real (non-headless)
# run: only the pre-switch line appears. GUI-opt-in — it briefly opens a window.
proj="$work/reconcile"
mkdir -p "$proj/src"
cat > "$proj/project.json" <<'JSON'
{ "name": "reconcile", "version": "0.1.0", "mfb": "1.0", "kind": "executable",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main", "targets": ["native"] }
JSON
cat > "$proj/src/main.mfb" <<'MFB'
IMPORT app
IMPORT io
SUB main()
  app::setMode(Mode.None)
  io::print("RECONCILE_BEFORE")
  io::flush()
  app::setMode(Mode.Console)
  io::print("RECONCILE_AFTER")
  io::flush()
  WHILE TRUE
  END WHILE
END SUB
MFB
if ! gui_enabled; then
  echo "skip: setMode reconcile GUI test (set MFB_MACAPP_GUI=1 when idle)"
elif ! "$MFB_EXE" build -app "$proj" >/dev/null 2>&1; then
  fail "build -app reconcile"
else
  out=$(perl -e '
    my $pid = open(my $fh, "-|");
    if ($pid == 0) { exec($ARGV[0]) or exit 127; }
    local $SIG{ALRM} = sub { kill "KILL", $pid; };
    alarm 8;
    my @l; while (my $x = <$fh>) { chomp $x; push @l, $x; last if @l >= 3; }
    kill "KILL", $pid; waitpid($pid, 0);
    print join("|", @l);
  ' "$(bundle "$proj" reconcile)/Contents/MacOS/reconcile")
  if [ "$out" = "RECONCILE_BEFORE" ]; then
    pass "setMode reconcile flips io from stdout to the transcript window ($out)"
  else
    fail "expected only RECONCILE_BEFORE on stdout, got '$out'"
  fi
fi

# Case 3e (plan-98-A Phase 3, GUI): the Mode.Canvas reconcile arm builds and tears
# down the canvas surface across a full enter -> exit -> re-enter cycle.
#
# This case MUST be GUI: the reconcile IMP is unreachable headless. Headless
# installs no app delegate, and _mfb_macapp_reconcile_marshal no-ops on a nil
# delegate (waitUntilDone:YES with no run loop to drain the perform would
# deadlock), so a headless run observes the mode slot but never the surface.
#
# io routing is the observable. Console points the io ASSOC_KEY at the transcript
# view, so a Console print does NOT reach stdout; entering Canvas installs the
# layer-backed canvas view and clears that key, so a Canvas print DOES. Hence:
#
#   CANVAS_HIDDEN  Console  -> transcript, absent from stdout (the reconcile ran)
#   CANVAS_ON      Canvas   -> stdout      (the canvas arm ran and cleared the key)
#   CANVAS_OFF     None     -> stdout      (teardown restored the transcript view)
#   CANVAS_AGAIN   Canvas   -> stdout      (re-entry reused the stashed view, no crash)
#
# CANVAS_AGAIN is the part that would catch a teardown that released the canvas
# view: re-entry would then message a freed object rather than print.
proj="$work/canvasmode"
mkdir -p "$proj/src"
cat > "$proj/project.json" <<'JSON'
{ "name": "canvasmode", "version": "0.1.0", "mfb": "1.0", "kind": "executable",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main", "targets": ["native"] }
JSON
cat > "$proj/src/main.mfb" <<'MFB'
IMPORT app
IMPORT io
SUB main()
  app::setMode(Mode.Console)
  io::print("CANVAS_HIDDEN")
  io::flush()
  app::setMode(Mode.Canvas)
  io::print("CANVAS_ON")
  io::flush()
  app::setMode(Mode.None)
  io::print("CANVAS_OFF")
  io::flush()
  app::setMode(Mode.Canvas)
  io::print("CANVAS_AGAIN")
  io::flush()
  WHILE TRUE
  END WHILE
END SUB
MFB
if ! gui_enabled; then
  echo "skip: Mode.Canvas reconcile GUI test (set MFB_MACAPP_GUI=1 when idle)"
elif ! "$MFB_EXE" build -app "$proj" >/dev/null 2>&1; then
  fail "build -app canvasmode"
else
  out=$(perl -e '
    my $pid = open(my $fh, "-|");
    if ($pid == 0) { exec($ARGV[0]) or exit 127; }
    local $SIG{ALRM} = sub { kill "KILL", $pid; };
    alarm 10;
    my @l; while (my $x = <$fh>) { chomp $x; push @l, $x; last if @l >= 4; }
    kill "KILL", $pid; waitpid($pid, 0);
    print join("|", @l);
  ' "$(bundle "$proj" canvasmode)/Contents/MacOS/canvasmode")
  if [ "$out" = "CANVAS_ON|CANVAS_OFF|CANVAS_AGAIN" ]; then
    pass "Mode.Canvas builds, tears down and rebuilds the canvas surface ($out)"
  else
    fail "expected CANVAS_ON|CANVAS_OFF|CANVAS_AGAIN, got '$out'"
  fi
fi

# Case 3f (plan-98-C Phase 3, GUI): the rendered frame actually reaches the window.
#
# This case MUST be GUI *and* must capture the screen: every other check in this
# plan can be satisfied by a renderer whose output never leaves memory. The
# headless MFB_CANVAS_DUMP path proves the rasteriser; only a screenshot proves
# the blit — the CGImage, the main-thread marshal, and the layer contents.
#
# The program draws two flat colour bars with no antialiased edge between them and
# then blocks, so the check is a coordinate lookup rather than an image diff: a
# capture is scaled by the display's backing factor and composited with the
# window's own chrome, neither of which the software golden knows about. Solid
# regions survive both.
#
# Requires Screen Recording permission (System Settings > Privacy & Security), the
# same requirement snap-macos.py documents. Without it macOS silently returns
# wallpaper, so the script fails loudly rather than passing on a blank image.
proj="$work/canvasblit"
mkdir -p "$proj/src"
cat > "$proj/project.json" <<'JSON'
{ "name": "canvasblit", "version": "0.1.0", "mfb": "1.0", "kind": "executable",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main", "targets": ["native"] }
JSON
cat > "$proj/src/main.mfb" <<'MFB'
IMPORT app
IMPORT canvas
IMPORT io
SUB main()
  app::setMode(Mode.Canvas)
  LET size AS Size = canvas::getSize()
  LET w AS Float = toFloat(size.width)
  LET h AS Float = toFloat(size.height)
  LET left AS DrawItem = Rectangle[x := 0.0, y := 0.0, w := w / 2.0, h := h, paint := canvas::fill(canvas::rgb(255, 0, 0))]
  LET right AS DrawItem = Rectangle[x := w / 2.0, y := 0.0, w := w / 2.0, h := h, paint := canvas::fill(canvas::rgb(0, 0, 255))]
  canvas::present([left, right])
  io::print("BLIT_PRESENTED")
  io::flush()
  WHILE TRUE
  END WHILE
END SUB
MFB
if ! gui_enabled; then
  echo "skip: canvas blit screenshot GUI test (set MFB_MACAPP_GUI=1 when idle)"
elif ! "$MFB_EXE" build -app "$proj" >/dev/null 2>&1; then
  fail "build -app canvasblit"
else
  shot="$work/canvasblit.png"
  if ! python3 "$ROOT/scripts/snap-macos.py" \
       "$(bundle "$proj" canvasblit)" "${shot%.png}" >/dev/null 2>&1; then
    fail "canvas blit screenshot (grant Screen Recording permission)"
  else
    verdict=$(python3 - "$shot" <<'PY'
import sys

from PIL import Image

image = Image.open(sys.argv[1]).convert("RGB")
w, h = image.size
# Sample well inside each half, and below any title bar, so neither window chrome
# nor the seam between the bars can be mistaken for the fill.
left = image.getpixel((w // 4, h * 3 // 4))
right = image.getpixel((w * 3 // 4, h * 3 // 4))


def near(got, want, slack=24):
    return all(abs(a - b) <= slack for a, b in zip(got, want))


if near(left, (255, 0, 0)) and near(right, (0, 0, 255)):
    print("ok")
else:
    print(f"left={left} right={right}")
PY
)
    if [ "$verdict" = "ok" ]; then
      pass "the rendered frame is blitted to the canvas layer (red|blue captured)"
    else
      fail "captured window is not the presented frame: $verdict"
    fi
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
  fail "build -app keepopen"
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
    pass "window stayed open after the program finished"
  else
    fail "app did not keep the window open ($result)"
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
  fail "build -app input"
else
  out=$(printf 'bob\nsecond\n' | run_headless_stdout "$(bundle "$proj" input)/Contents/MacOS/input")
  if [ "$out" = $'Name? Hi bob\nEcho second' ]; then
    pass "app-mode io::input + io::readLine consume input correctly"
  else
    fail "unexpected app-mode input output: $(printf '%q' "$out")"
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
  fail "build -app inputonly (bug-247: missing _isatty/_tcgetattr imports?)"
else
  out=$(printf 'bob\n' | run_headless_stdout "$(bundle "$proj" inputonly)/Contents/MacOS/inputonly")
  if [ "$out" = 'Name? Hi bob' ]; then
    pass "app-mode io::input alone (no io::readLine) builds and reads"
  else
    fail "unexpected app-mode input-only output: $(printf '%q' "$out")"
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
  fail "build -app keyinput"
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
    pass "window keypresses delivered to io::readLine"
  else
    echo "skip: window keystroke injection unavailable (need Accessibility); got '$got'"
  fi
fi

# Case 6b (plan-98-A Phase 4, GUI): the same keystroke round-trip, but in
# Mode.Canvas. The canvas surface is a synthesized MFBCanvasView whose keyDown:
# writes straight to the window input pipe — no echo, no line buffering, since a
# canvas has no text surface to echo into. Two things this proves that Case 6
# cannot: that the canvas view is made first responder at all (a plain NSView
# returns NO from acceptsFirstResponder and would receive nothing), and that a
# None-default program gets an input pipe (before Phase 4 the pipe was wired only
# in the Console-default startup arm, so PIPE_ASSOC_KEY was nil here).
#
# Return arrives as CR from [event characters] but io::readLine terminates on LF,
# so a missing CR->LF translation shows up as a hang, not as wrong text.
proj="$work/canvaskeys"
mkdir -p "$proj/src"
cat > "$proj/project.json" <<'JSON'
{ "name": "canvaskeys", "version": "0.1.0", "mfb": "1.0", "kind": "executable",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main", "targets": ["native"] }
JSON
cat > "$proj/src/main.mfb" <<MFB
IMPORT app
IMPORT io
IMPORT fs
SUB main()
  app::setMode(Mode.Canvas)
  LET name AS String = io::readLine()
  fs::writeText("$proj/got.txt", "got:" & name)
END SUB
MFB
if ! gui_enabled; then
  echo "skip: Mode.Canvas keystroke GUI test (set MFB_MACAPP_GUI=1 when idle)"
elif ! "$MFB_EXE" build -app "$proj" >/dev/null 2>&1; then
  fail "build -app canvaskeys"
else
  rm -f "$proj/got.txt"
  open "$(bundle "$proj" canvaskeys)"
  sleep 2
  osascript -e 'tell application "System Events" to keystroke "CanvasKeys"' >/dev/null 2>&1
  osascript -e 'tell application "System Events" to key code 36' >/dev/null 2>&1
  sleep 1
  pkill -KILL canvaskeys >/dev/null 2>&1
  got=$(cat "$proj/got.txt" 2>/dev/null || true)
  if [ "$got" = "got:CanvasKeys" ]; then
    pass "canvas-window keypresses delivered to io::readLine"
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
  fail "build -app isterm"
else
  out=$(run_headless_stdout "$(bundle "$proj" isterm)/Contents/MacOS/isterm" 10)
  if [ "$out" = "terminal:yes" ]; then
    pass "app-mode io::is*Terminal return TRUE"
  else
    fail "io::is*Terminal expected terminal:yes, got '$out'"
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
  fail "build -app tsize"
else
  rm -f "$proj/size.txt"
  open "$(bundle "$proj" tsize)"
  sleep 2
  pkill -KILL tsize >/dev/null 2>&1
  size=$(cat "$proj/size.txt" 2>/dev/null || true)
  if printf '%s' "$size" | grep -Eq '^[1-9][0-9]*x[1-9][0-9]*$'; then
    pass "term::terminalSize reported window surface ($size)"
  else
    echo "skip: term::terminalSize window check unavailable (need GUI session); got '$size'"
  fi
fi

if [ "$failures" -ne 0 ]; then
  echo "macOS app mode runtime tests failed: $failures" >&2
  exit 1
fi
echo "macOS app mode runtime tests passed"
