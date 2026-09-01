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


# ---------------------------------------------------------------------------
# Mode.Canvas (bug-479)
#
# The graphics thread is a SECOND thread entry, and it had the x86-64 realign that
# `_pthread_start` gets but `BaseThreadInitThunk` was assumed not to need. It does:
# both reach a start routine through a `call`, so both arrive at `rsp % 16 == 8`.
# Eight bytes out, `SleepConditionVariableSRW` faulted inside ntdll on the FIRST
# wait — with the condition variable initialised, the lock genuinely held and every
# argument correct — because it tags its stack wait-block pointer in the low 4 bits.
#
# Checking the exit code alone would not have caught it: the fault is on the graphics
# thread, at shutdown, AFTER the program has printed everything it prints. So this
# asserts the pixels too, which is also the frame plan-98-F Phase 3 compares Vulkan
# against.
cproj="$work/wincanvas"
mkdir -p "$cproj/src"
cat > "$cproj/project.json" <<'JSON'
{ "name": "wincanvas", "version": "0.1.0", "mfb": "1.0", "kind": "executable",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main", "targets": ["native"] }
JSON
cat > "$cproj/src/main.mfb" <<'MFB'
IMPORT app
IMPORT canvas
IMPORT io
IMPORT os

SUB main()
  app::setMode(app::Mode.Canvas)
  LET box AS canvas::DrawItem = canvas::Rectangle[x := 100.0, y := 100.0, w := 200.0, h := 120.0, paint := canvas::fill(canvas::rgb(200, 40, 40))]
  LET dot AS canvas::DrawItem = canvas::Circle[x := 600.0, y := 400.0, radius := 60.0, paint := canvas::fill(canvas::rgb(40, 200, 120))]
  canvas::present([box, dot])
  io::print("canvas presented")
  ' The scripted resize runs on the MAIN thread and waits for the first frame before
  ' publishing the new size. Returning from `main` here would retire the worker, and
  ' `_main` would fall out of WaitForSingleObject and exit the process before the
  ' second frame was ever drawn — the resize assertions would then fail for a reason
  ' that has nothing to do with the resize.
  os::sleep(1500)
END SUB
MFB

echo "--- building the canvas program for windows-x86_64 ---"
"$MFB_EXE" build --app --target windows-x86_64 "$cproj" >/dev/null

cat > "$work/canvas.bat" <<'BAT'
@echo off
setlocal
set MFB_WINAPP_HEADLESS=1
set MFB_CANVAS_SYNC=1
set MFB_CANVAS_DUMP=C:\mfbwin\wincanvas.raw
cd /d C:\mfbwin
wincanvas.exe > wincanvas.out 2>&1
echo rc=%errorlevel%
type wincanvas.out
BAT

ssh -p "$PORT" "$host" "del /q $remote\\wincanvas.raw $remote\\wincanvas.out 2>nul" >/dev/null 2>&1 || true
scp -P "$PORT" "$cproj/build/wincanvas.exe" "$host:C:/mfbwin/wincanvas.exe" >/dev/null
scp -P "$PORT" "$work/canvas.bat" "$host:C:/mfbwin/canvas.bat" >/dev/null
cout="$(ssh -p "$PORT" "$host" "$remote\\canvas.bat" 2>&1 || true)"
echo "$cout" | sed 's/^/    /'

case "$cout" in
  *"canvas presented"*) pass "canvas::present returned on Windows" ;;
  *) fail "canvas::present never returned — the graphics thread died before completing a frame" ;;
esac
case "$cout" in
  *"rc=0"*) pass "the canvas program exited cleanly" ;;
  *) fail "the canvas program did not exit 0 — the graphics thread faults at shutdown when its stack is 8 bytes out (bug-479)" ;;
esac

# The frame itself. 900x640 BGRA/RGBA, 4 bytes a pixel.
scp -P "$PORT" "$host:C:/mfbwin/wincanvas.raw" "$work/wincanvas.raw" >/dev/null 2>&1 || true
if [ -f "$work/wincanvas.raw" ]; then
  size="$(wc -c < "$work/wincanvas.raw" | tr -d ' ')"
  if [ "$size" = "2304000" ]; then
    pass "the dumped frame is 900x640x4 bytes"
  else
    fail "the dumped frame is $size bytes, expected 2304000 (900*640*4)"
  fi
  # (5,5) background, (150,150) inside the rectangle, (600,400) inside the circle.
  probe() { od -An -tu1 -j "$1" -N 4 "$work/wincanvas.raw" | tr -s ' ' | sed 's/^ //;s/ $//'; }
  bg="$(probe 18020)"; rect="$(probe 540600)"; circ="$(probe 1442400)"
  [ "$bg" = "0 0 0 255" ] && pass "the background is opaque black" \
    || fail "the background is [$bg], expected [0 0 0 255] — every backend clears to opaque black"
  [ "$rect" = "200 40 40 255" ] && pass "the rectangle rendered at its requested colour" \
    || fail "the rectangle pixel is [$rect], expected [200 40 40 255]"
  [ "$circ" = "40 200 120 255" ] && pass "the circle rendered at its requested colour" \
    || fail "the circle pixel is [$circ], expected [40 200 120 255]"
else
  fail "no frame was dumped — MFB_CANVAS_DUMP produced nothing, so no frame completed"
fi


# ---------------------------------------------------------------------------
# The Vulkan backend on Windows (plan-98-F Phase 3)
#
# Same binary, same scene, run again with MFB_CANVAS_GPU=1, and the two frames
# compared on `Tolerance::GPU_DEFAULT` — no channel off by more than 2, no more than
# 2% of pixels differing — which is the comparator test-canvas-vulkan.sh applies on
# Linux.
#
# **`vulkanReady=TRUE` is asserted before the diff, and that assertion is the whole
# point.** A backend that silently fell back to the software path would produce a
# byte-IDENTICAL frame and sail through a tolerance comparison. Agreement is only
# evidence when the two frames were produced by different renderers.
cat > "$work/canvasgpu.bat" <<'BAT'
@echo off
setlocal
set MFB_WINAPP_HEADLESS=1
set MFB_CANVAS_SYNC=1
set MFB_CANVAS_GPU=1
set MFB_CANVAS_STATS=C:\mfbwin\wincanvas.stats
set MFB_CANVAS_DUMP=C:\mfbwin\wincanvas.gpu.raw
cd /d C:\mfbwin
wincanvas.exe > wincanvas.gpu.out 2>&1
echo rc=%errorlevel%
type wincanvas.gpu.out
type C:\mfbwin\wincanvas.stats
BAT

ssh -p "$PORT" "$host" "del /q $remote\\wincanvas.gpu.raw $remote\\wincanvas.stats 2>nul" >/dev/null 2>&1 || true
scp -P "$PORT" "$work/canvasgpu.bat" "$host:C:/mfbwin/canvasgpu.bat" >/dev/null
gout="$(ssh -p "$PORT" "$host" "$remote\\canvasgpu.bat" 2>&1 || true)"
echo "$gout" | sed 's/^/    /'

case "$gout" in
  *"vulkanReady=TRUE"*) pass "the Vulkan device built on Windows" ;;
  *) fail "vulkanReady is not TRUE — the loader, the instance or the device did not come up, and any frame comparison below would be software-vs-software" ;;
esac
case "$gout" in
  *"gpuSelected=TRUE"*) pass "the GPU path was the one that rendered" ;;
  *) fail "gpuSelected is not TRUE — MFB_CANVAS_GPU did not take effect" ;;
esac
case "$gout" in
  *"rc=0"*) pass "the Vulkan canvas program exited cleanly" ;;
  *) fail "the Vulkan canvas program did not exit 0" ;;
esac

scp -P "$PORT" "$host:C:/mfbwin/wincanvas.gpu.raw" "$work/wincanvas.gpu.raw" >/dev/null 2>&1 || true
if [ -f "$work/wincanvas.gpu.raw" ] && [ -f "$work/wincanvas.raw" ]; then
  # The GPU frame must contain the scene in its own right — two blank frames agree.
  gprobe() { od -An -tu1 -j "$1" -N 4 "$work/wincanvas.gpu.raw" | tr -s ' ' | sed 's/^ //;s/ $//'; }
  grect="$(gprobe 540600)"; gcirc="$(gprobe 1442400)"
  [ "$grect" = "200 40 40 255" ] && pass "the Vulkan frame drew the rectangle" \
    || fail "the Vulkan rectangle pixel is [$grect], expected [200 40 40 255] — a frame that drew nothing would still match a blank reference"
  [ "$gcirc" = "40 200 120 255" ] && pass "the Vulkan frame drew the circle" \
    || fail "the Vulkan circle pixel is [$gcirc], expected [40 200 120 255]"

  verdict="$(python3 - "$work/wincanvas.raw" "$work/wincanvas.gpu.raw" <<'PY'
import sys
a = open(sys.argv[1], "rb").read()
b = open(sys.argv[2], "rb").read()
if len(a) != len(b) or not a:
    print(f"frame sizes differ ({len(a)} vs {len(b)}) — a harness bug")
    raise SystemExit
# Tolerance::GPU_DEFAULT, the same bound test-canvas-vulkan.sh applies on Linux.
worst = 0
differing = 0
total = len(a) // 4
for i in range(0, len(a), 4):
    pa, pb = a[i:i + 4], b[i:i + 4]
    if pa == pb:
        continue
    differing += 1
    worst = max(worst, max(abs(x - y) for x, y in zip(pa, pb)))
fraction = differing / total
verdict = "ok" if worst <= 2 and fraction <= 0.02 else "BEYOND"
print(f"{verdict} worst={worst} differing={fraction * 100:.4f}%")
PY
)"
  case "$verdict" in
    ok*) pass "the Vulkan frame matches the software reference within tolerance ($verdict)" ;;
    *) fail "the Vulkan frame is outside Tolerance::GPU_DEFAULT — $verdict" ;;
  esac
else
  fail "no Vulkan frame was dumped — the GPU path completed no frame"
fi


# ---------------------------------------------------------------------------
# The resize handshake on Windows (plan-98-F Phase 3)
#
# Windows had NO caller of `emit_publish_surface_size`: the graphics thread never
# learned the window had changed size and kept rendering at the startup 900x640.
# The WM_SIZE arm publishes it now, and `MFB_CANVAS_RESIZE_W/_H` drive one scripted
# resize through that same publisher on the headless path — a resize is a window
# event and headless has no window, so this is the only way the arm is reachable on
# the one box that can run it.
#
# The frame's LENGTH is the assertion that matters: MFB_CANVAS_DUMP overwrites, so
# the file left behind is the second frame, and 640*480*4 can only be produced by a
# render target that was actually rebuilt at the new size.
resize_run() { # $1 = tag, $2 = extra env line
  # The Windows paths are built HERE, not inside the here-document. An unquoted
  # here-doc still honours `\$` as an escape, so `C:\mfbwin\$1.raw` written inline
  # collapses to the literal `C:\mfbwin$1.raw` — the dump lands in a file called
  # `mfbwin$1.raw` at the drive root and every later assertion reports "no frame".
  stats_path="C:\\mfbwin\\$1.stats"
  dump_path="C:\\mfbwin\\$1.raw"
  cat > "$work/rs.bat" <<BAT
@echo off
setlocal
set MFB_WINAPP_HEADLESS=1
set MFB_CANVAS_SYNC=1
set MFB_CANVAS_RESIZE_W=640
set MFB_CANVAS_RESIZE_H=480
$2
set MFB_CANVAS_STATS=$stats_path
set MFB_CANVAS_DUMP=$dump_path
cd /d C:\mfbwin
wincanvas.exe > $1.out 2>&1
echo rc=%errorlevel%
type $stats_path
BAT
  ssh -p "$PORT" "$host" "del /q $remote\\$1.raw $remote\\$1.stats 2>nul" >/dev/null 2>&1 || true
  scp -P "$PORT" "$work/rs.bat" "$host:C:/mfbwin/rs.bat" >/dev/null
  ssh -p "$PORT" "$host" "$remote\\rs.bat" 2>&1 || true
}

rsw="$(resize_run rssw '')"
rsg="$(resize_run rsgpu 'set MFB_CANVAS_GPU=1')"
echo "$rsg" | sed 's/^/    /'

case "$rsg" in
  *"frames=2"*) pass "the resize produced a second frame rather than reusing the first" ;;
  *) fail "no second frame after the resize — WM_SIZE published nothing, or the render loop never woke" ;;
esac
case "$rsg" in
  *"damage=0,0,640,480"*) pass "the repaint covered the new 640x480 surface" ;;
  *) fail "the damage rect is not the new surface — the publisher did not take effect" ;;
esac

for tag in rssw rsgpu; do
  scp -P "$PORT" "$host:C:/mfbwin/$tag.raw" "$work/$tag.raw" >/dev/null 2>&1 || true
  if [ -f "$work/$tag.raw" ]; then
    size="$(wc -c < "$work/$tag.raw" | tr -d ' ')"
    [ "$size" = "1228800" ] && pass "$tag repainted at 640x480 (1228800 bytes)" \
      || fail "$tag is $size bytes, expected 1228800 (640*480*4) — the render target was not rebuilt at the new size"
  else
    fail "$tag produced no frame after the resize"
  fi
done

if [ -f "$work/rssw.raw" ] && [ -f "$work/rsgpu.raw" ]; then
  verdict="$(python3 - "$work/rssw.raw" "$work/rsgpu.raw" <<'PY'
import sys
a = open(sys.argv[1], "rb").read()
b = open(sys.argv[2], "rb").read()
if len(a) != len(b) or not a:
    print(f"frame sizes differ ({len(a)} vs {len(b)}) — a harness bug")
    raise SystemExit
worst = 0
differing = 0
total = len(a) // 4
for i in range(0, len(a), 4):
    pa, pb = a[i:i + 4], b[i:i + 4]
    if pa == pb:
        continue
    differing += 1
    worst = max(worst, max(abs(x - y) for x, y in zip(pa, pb)))
fraction = differing / total
verdict = "ok" if worst <= 2 and fraction <= 0.02 else "BEYOND"
print(f"{verdict} worst={worst} differing={fraction * 100:.4f}%")
PY
)"
  case "$verdict" in
    ok*) pass "the resized Vulkan frame matches the resized software reference ($verdict)" ;;
    *) fail "the resized Vulkan frame is outside Tolerance::GPU_DEFAULT — $verdict" ;;
  esac
fi

if [ "$fails" -eq 0 ]; then
  echo "windows app-mode, canvas and Vulkan runtime tests passed"
else
  echo "$fails failure(s)"
  exit 1
fi
