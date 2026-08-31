#!/usr/bin/env bash
# Runtime acceptance for the canvas Vulkan backend (plan-98-F).
#
# The Linux counterpart of the Metal half of `cargo test --test rt_canvas_metal`,
# and it has to be a script for the same structural reason `test-appimage.sh` does:
# the dev host is macOS and cannot run a Linux binary, so the artifact travels.
#
# What it asserts is the letter's whole gate — that the GPU render matches the
# **software oracle**, not a stored image. It renders the same program twice on the
# box, once with `MFB_CANVAS_GPU=1` and once without, and diffs the two frames. That
# is stronger than a checked-in reference because it cannot go stale: if the
# rasteriser changes, both sides change together and the comparison still means "the
# two backends agree".
#
# Usage: scripts/test-canvas-vulkan.sh <mfb-exe> [--box <port>] [--libc glibc|musl]
#
#   --box <port>   ssh port of the target box (default 2228, Ubuntu x86_64 glibc).
#   --libc <l>     which AppImage to ship (default glibc). It must match the box:
#                  musl's loader absorbs the glibc compat sonames, so shipping the
#                  wrong one does not fail cleanly — box 2227 is musl.
#
# The box needs a Vulkan loader and an ICD. It does NOT need a display server: the
# renderer draws offscreen and reads back (plan-98-F Correction 1), and
# `MFB_GTKAPP_HEADLESS` skips GTK entirely. That is deliberate — no reachable Linux
# box has a display, so a design that needed one could not be tested at all.
#
# A box that cannot build a pipeline reports `vulkanReady=FALSE`; this skips rather
# than fails there, because "no usable GPU here" is a real configuration and not a
# regression. The skip gates on that one flag deliberately — it is the flag the
# renderer itself gates on, so the test and the runtime can never disagree about
# whether the GPU path was taken.
set -euo pipefail

MFB_EXE="${1:?usage: test-canvas-vulkan.sh <mfb-exe> [--box <port>]}"
shift || true
PORT=2228
LIBC=glibc
while [ $# -gt 0 ]; do
  case "$1" in
    --box) PORT="$2"; shift 2 ;;
    --libc) LIBC="$2"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MFB_EXE="$(cd "$(dirname "$MFB_EXE")" && pwd)/$(basename "$MFB_EXE")"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
fails=0
pass() { echo "ok: $1"; }
fail() { echo "FAIL: $1"; fails=$((fails + 1)); }

proj="$work/vkcanvas"
mkdir -p "$proj/src"
cat > "$proj/project.json" <<'JSON'
{ "name": "vkcanvas", "version": "0.1.0", "mfb": "1.0", "kind": "executable",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main", "targets": ["native"] }
JSON

# Every primitive the Vulkan shader claims to draw. No Polygon: its per-edge array
# does not fit a push-constant block, so `__canvas_vulkanRenderable` declines a scene
# containing one and the software path renders it — which the fallback case below is
# what checks.
cat > "$proj/src/main.mfb" <<'MFB'
IMPORT app
IMPORT canvas
SUB main()
  app::setMode(Mode.Canvas)
  LET yellow AS Color = canvas::rgb(255, 255, 0)
  LET green AS Color = canvas::rgb(0, 160, 0)
  LET face AS DrawItem = Circle[x := 450.0, y := 320.0, radius := 150.0, paint := canvas::fill(yellow)]
  LET eyeL AS DrawItem = Circle[x := 400.0, y := 280.0, radius := 22.0, paint := canvas::fill(green)]
  LET eyeR AS DrawItem = Circle[x := 500.0, y := 280.0, radius := 22.0, paint := canvas::fill(green)]
  LET smile AS DrawItem = Arc[x := 450.0, y := 335.0, radius := 90.0, startAngle := 0.0, endAngle := 3.14159, paint := canvas::stroke(green, 14.0)]
  LET box AS DrawItem = Rectangle[x := 10.0, y := 10.0, w := 50.0, h := 50.0, paint := canvas::fill(canvas::rgb(255, 0, 0))]
  LET rounded AS DrawItem = RoundedRect[x := 100.0, y := 10.0, w := 90.0, h := 60.0, cornerRadius := 18.0, paint := canvas::fillStroke(canvas::rgb(0, 0, 255), canvas::rgb(255, 255, 255), 4.0)]
  LET line AS DrawItem = Line[x1 := 220.0, y1 := 20.0, x2 := 380.0, y2 := 90.0, paint := canvas::stroke(canvas::rgb(255, 128, 0), 9.0)]
  LET faint AS DrawItem = Rectangle[x := 600.0, y := 40.0, w := 120.0, h := 80.0, paint := canvas::fill(canvas::rgba(0, 200, 255, 180))]
  canvas::present([box, rounded, line, faint, face, eyeL, eyeR, smile])
END SUB
MFB

echo "--- building for linux-x86_64 ---"
"$MFB_EXE" build --app --target linux-x86_64 "$proj" >/dev/null

host="test@127.0.0.1"
remote="/tmp/mfb-vkcanvas-$$"
ssh -p "$PORT" "$host" "rm -rf $remote && mkdir -p $remote"
scp -P "$PORT" "$proj/build/vkcanvas-$LIBC.AppImage" "$host:$remote/app.AppImage" >/dev/null

echo "--- running on box $PORT ---"
ssh -p "$PORT" "$host" "
  set -e
  cd $remote
  ./app.AppImage --appimage-extract >/dev/null 2>&1
  bin=./squashfs-root/usr/bin/vkcanvas
  MFB_GTKAPP_HEADLESS=1 MFB_CANVAS_SYNC=1 MFB_CANVAS_STATS=$remote/sw.txt \
    MFB_CANVAS_DUMP=$remote/sw.rgba timeout 90 \$bin >/dev/null 2>&1
  MFB_GTKAPP_HEADLESS=1 MFB_CANVAS_SYNC=1 MFB_CANVAS_GPU=1 MFB_CANVAS_STATS=$remote/gpu.txt \
    MFB_CANVAS_DUMP=$remote/gpu.rgba timeout 90 \$bin >/dev/null 2>&1
"
scp -P "$PORT" "$host:$remote/sw.rgba" "$work/sw.rgba" >/dev/null
scp -P "$PORT" "$host:$remote/gpu.rgba" "$work/gpu.rgba" >/dev/null
scp -P "$PORT" "$host:$remote/gpu.txt" "$work/gpu.txt" >/dev/null
ssh -p "$PORT" "$host" "rm -rf $remote"

if [ ! -s "$work/gpu.txt" ]; then
  fail "the program wrote no stats line — it did not reach a rendered frame (wrong --libc for this box?)"
  exit 1
fi
stats="$(tail -1 "$work/gpu.txt")"
echo "    $stats"
case "$stats" in
  *vulkanReady=FALSE*)
    echo "skip: box $PORT built no Vulkan device (loader present, no usable ICD)"
    exit 0
    ;;
esac
case "$stats" in
  *vulkanReady=TRUE*) pass "the Vulkan device and pipeline built" ;;
  *) fail "the box reports a Vulkan device but no pipeline — a broken shader, not a missing GPU"; exit 1 ;;
esac
case "$stats" in
  *gpuSelected=TRUE*) pass "MFB_CANVAS_GPU selected the GPU renderer" ;;
  *) fail "MFB_CANVAS_GPU did not select the GPU renderer" ;;
esac

verdict=$(python3 - "$work/sw.rgba" "$work/gpu.rgba" <<'PY'
import sys

software = open(sys.argv[1], "rb").read()
gpu = open(sys.argv[2], "rb").read()
if len(software) != len(gpu) or not software:
    print(f"frame sizes differ ({len(software)} vs {len(gpu)}) — a harness bug")
    raise SystemExit
# Tolerance::GPU_DEFAULT: no pixel may differ by more than 2 steps in any channel,
# and no more than 2% of pixels may differ at all.
worst = 0
differing = 0
first = None
total = len(software) // 4
for i in range(0, len(software), 4):
    a = software[i:i + 4]
    b = gpu[i:i + 4]
    if a == b:
        continue
    differing += 1
    delta = max(abs(x - y) for x, y in zip(a, b))
    if delta > worst:
        worst = delta
    if first is None and delta > 2:
        pixel = i // 4
        first = (pixel % 900, pixel // 900, a.hex(), b.hex())
fraction = differing / total
if worst <= 2 and fraction <= 0.02:
    print(f"ok worst={worst} differing={fraction * 100:.4f}%")
else:
    print(f"worst={worst} differing={fraction * 100:.4f}% first-beyond-tolerance={first}")
PY
)
case "$verdict" in
  ok*)
    pass "the Vulkan render matches the software oracle ($verdict)"
    ;;
  *)
    fail "the Vulkan render disagrees with the software oracle: $verdict"
    echo "    Localize the primitive at that coordinate. A whole-shape mismatch is a"
    echo "    distance function; a rim of edge pixels is coverage; a uniform shift is"
    echo "    the sRGB/linear chain."
    ;;
esac

if [ "$fails" -eq 0 ]; then
  echo "canvas Vulkan runtime tests passed"
else
  echo "$fails failure(s)"
  exit 1
fi
