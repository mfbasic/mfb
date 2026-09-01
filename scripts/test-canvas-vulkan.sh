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
# Smaller than the default 900x640 in both axes, so a renderer that ignored the resize
# and kept its old target would be caught by the frame length rather than only by the
# pixels.
RESIZE_W=640
RESIZE_H=480
# A user-local Vulkan driver, for a box that has the loader but no ICD.
#
# `--icd auto` provisions one into `/tmp` on an Alpine box and points the loader at it;
# `--icd <manifest>` uses one already there; unset uses whatever the box has installed.
# Alpine 2227 is the musl half of this test's evidence and ships `vulkan-loader` with no
# driver behind it, so without this the musl row is a skip — and a skip is what hid a
# *wrong* `vulkanReady` for a whole phase (Correction 13).
ICD=""
while [ $# -gt 0 ]; do
  case "$1" in
    --box) PORT="$2"; shift 2 ;;
    --libc) LIBC="$2"; shift 2 ;;
    --icd) ICD="$2"; shift 2 ;;
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

# The fixture font, so the scene can contain text.
#
# Synthesized rather than borrowed from the box: a system font would make the
# comparison depend on which fonts the box happens to have, and the point of this test
# is that the two *backends* agree — not that a particular typeface renders. This is
# the same twelve-glyph file `tests/rt_canvas_font.rs` builds: `unitsPerEm` 1000, one
# square glyph at (100,0)-(400,300), so a `Text` item is a row of squares whose pixels
# are easy to reason about and whose bitmaps are far inside both backends' caps.
base64 -d > "$proj/fixture.ttf" <<'TTF'
AAEAAAAGAAAAAAAAY21hcAAAAAAAAABsAAAANGdseWYAAAAAAAAAoAAAACJoZWFkAAAAAAAAAMIA
AAA2aGhlYQAAAAAAAAD4AAAAJGhtdHgAAAAAAAABHAAAAAxsb2NhAAAAAAAAASgAAAAIAAAAAQAD
AAoAAAAMAAwAAAAAACgAAAAAAAAAAgAAAEEAAABBAAAAAQAAAEIAAABCAAAAAgABAGQAAAGQASwA
AwAAAQEBAQBkASwAAP7UAAAAAAEsAAAAAAAAAAAAAAAAAAAAAAAAAAAD6AAAAAAAAAAAAAAAAAAA
AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAyD/OABkAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAMB
9AAAAPoAAAEsAAAAAAAAABEAEQ==
TTF

# Every primitive the Vulkan shader claims to draw, including `Polygon` — whose edges
# reach the shader through the descriptor-bound storage buffer rather than the push
# constants (Phase 2) — and `Text`, whose glyph bitmaps reach it through that same
# buffer's second region (plan-98-G Phase 2). Two polygons, so the per-item edge base
# is actually exercised: with one, a base of zero would pass whether or not it was
# ever written. Four glyphs, for the same reason applied to the glyph cursor.
#
# The convex triangle and the concave arrow are deliberate: the crossing-count sign
# test and the nearest-edge magnitude only disagree on a shape that is not convex, so
# a triangle alone cannot tell a correct fill rule from a wrong one.
cat > "$proj/src/main.mfb" <<'MFB'
IMPORT app
IMPORT canvas
IMPORT os
SUB main()
  app::setMode(Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e)
    EXIT SUB
  END TRAP
  LET yellow AS Color = canvas::rgb(255, 255, 0)
  LET green AS Color = canvas::rgb(0, 160, 0)
  LET head AS DrawItem = Circle[x := 450.0, y := 320.0, radius := 150.0, paint := canvas::fill(yellow)]
  LET eyeL AS DrawItem = Circle[x := 400.0, y := 280.0, radius := 22.0, paint := canvas::fill(green)]
  LET eyeR AS DrawItem = Circle[x := 500.0, y := 280.0, radius := 22.0, paint := canvas::fill(green)]
  LET smile AS DrawItem = Arc[x := 450.0, y := 335.0, radius := 90.0, startAngle := 0.0, endAngle := 3.14159, paint := canvas::stroke(green, 14.0)]
  LET box AS DrawItem = Rectangle[x := 10.0, y := 10.0, w := 50.0, h := 50.0, paint := canvas::fill(canvas::rgb(255, 0, 0))]
  LET rounded AS DrawItem = RoundedRect[x := 100.0, y := 10.0, w := 90.0, h := 60.0, cornerRadius := 18.0, paint := canvas::fillStroke(canvas::rgb(0, 0, 255), canvas::rgb(255, 255, 255), 4.0)]
  LET line AS DrawItem = Line[x1 := 220.0, y1 := 20.0, x2 := 380.0, y2 := 90.0, paint := canvas::stroke(canvas::rgb(255, 128, 0), 9.0)]
  LET faint AS DrawItem = Rectangle[x := 600.0, y := 40.0, w := 120.0, h := 80.0, paint := canvas::fill(canvas::rgba(0, 200, 255, 180))]
  LET tri AS DrawItem = Polygon[points := [Point[x := 620.0, y := 200.0], Point[x := 740.0, y := 200.0], Point[x := 680.0, y := 300.0]], paint := canvas::fill(canvas::rgb(200, 0, 200))]
  LET arrow AS DrawItem = Polygon[points := [Point[x := 60.0, y := 400.0], Point[x := 160.0, y := 400.0], Point[x := 160.0, y := 360.0], Point[x := 230.0, y := 430.0], Point[x := 160.0, y := 500.0], Point[x := 160.0, y := 460.0], Point[x := 60.0, y := 460.0]], paint := canvas::fillStroke(canvas::rgb(0, 180, 180), canvas::rgb(20, 20, 20), 6.0)]
  LET label AS DrawItem = Text[x := 300.0, y := 560.0, text := "AAAA", font := canvas::fontRef(face), size := 90.0, paint := canvas::fill(canvas::rgb(220, 40, 160))]
  canvas::present([box, rounded, line, faint, head, eyeL, eyeR, smile, tri, arrow, label])
  ' Stay alive for the resize case below. Without this the worker returns from main
  ' the moment its frame lands and the finish helper _exits the process, so the
  ' scripted resize on the main thread loses the race every time. Measured before the
  ' sleep was added: the dump stayed 900x640 and only one stats line was written.
  os::sleep(3000)
END SUB
MFB

echo "--- building for linux-x86_64 ---"
"$MFB_EXE" build --app --target linux-x86_64 "$proj" >/dev/null

host="test@127.0.0.1"
remote="/tmp/mfb-vkcanvas-$$"
ssh -p "$PORT" "$host" "rm -rf $remote && mkdir -p $remote"
scp -P "$PORT" "$proj/build/vkcanvas-$LIBC.AppImage" "$host:$remote/app.AppImage" >/dev/null
scp -P "$PORT" "$proj/fixture.ttf" "$host:$remote/fixture.ttf" >/dev/null

# Provision the driver before anything measures with it.
icd_env=""
if [ -n "$ICD" ]; then
  if [ "$ICD" = auto ]; then
    echo "--- provisioning a software Vulkan driver on box $PORT ---"
    ssh -p "$PORT" "$host" '
      set -e
      dir=/tmp/mfb-vulkan-icd
      manifest=$dir/usr/share/vulkan/icd.d/lvp_icd.x86_64.json
      if [ ! -f "$manifest" ]; then
        mkdir -p $dir && cd $dir
        base=http://dl-cdn.alpinelinux.org/alpine/v3.24/main/x86_64
        # The repository carries one build of each; pick it out of the index rather
        # than pinning a version that goes 404 on the next Alpine point release.
        for pkg in mesa-vulkan-swrast libdisplay-info; do
          file=$(wget -qO- "$base/" | grep -o "${pkg}-[0-9][^\"]*\.apk" | head -1)
          wget -q "$base/$file" -O "$pkg.apk"
          tar -xzf "$pkg.apk" 2>/dev/null || true
        done
        # The manifest names an absolute path that assumes the package was installed.
        sed -i "s|/usr/lib/libvulkan_lvp.so|$dir/usr/lib/libvulkan_lvp.so|" "$manifest"
      fi
      test -f "$manifest"
    '
    ICD=/tmp/mfb-vulkan-icd/usr/share/vulkan/icd.d/lvp_icd.x86_64.json
  fi
  icd_env="VK_ICD_FILENAMES=$ICD LD_LIBRARY_PATH=$(dirname "$(dirname "$(dirname "$(dirname "$ICD")")")")/lib"
  echo "    driver: $ICD"
fi

echo "--- running on box $PORT ---"
ssh -p "$PORT" "$host" "
  set -e
  cd $remote
  ./app.AppImage --appimage-extract >/dev/null 2>&1
  bin=./squashfs-root/usr/bin/vkcanvas
  $icd_env MFB_GTKAPP_HEADLESS=1 MFB_CANVAS_SYNC=1 MFB_CANVAS_STATS=$remote/sw.txt \
    MFB_CANVAS_DUMP=$remote/sw.rgba timeout 180 \$bin >/dev/null 2>&1
  $icd_env MFB_GTKAPP_HEADLESS=1 MFB_CANVAS_SYNC=1 MFB_CANVAS_GPU=1 MFB_CANVAS_STATS=$remote/gpu.txt \
    MFB_CANVAS_DUMP=$remote/gpu.rgba timeout 180 \$bin >/dev/null 2>&1
  $icd_env MFB_GTKAPP_HEADLESS=1 MFB_CANVAS_SYNC=1 MFB_CANVAS_RESIZE_W=$RESIZE_W MFB_CANVAS_RESIZE_H=$RESIZE_H \
    MFB_CANVAS_DUMP=$remote/sw2.rgba timeout 180 \$bin >/dev/null 2>&1
  $icd_env MFB_GTKAPP_HEADLESS=1 MFB_CANVAS_SYNC=1 MFB_CANVAS_GPU=1 MFB_CANVAS_RESIZE_W=$RESIZE_W \
    MFB_CANVAS_RESIZE_H=$RESIZE_H MFB_CANVAS_STATS=$remote/gpu2.txt \
    MFB_CANVAS_DUMP=$remote/gpu2.rgba timeout 180 \$bin >/dev/null 2>&1
  # plan-98-G Phase 3: a resize to the size the surface already has. It wakes the
  # renderer through the production resize path with nothing about the scene changed,
  # which is the one wake a program cannot produce by presenting: publishScene refuses
  # an unchanged scene before it ever signals a redraw. This box is where that case can
  # be reached at all -- macOS has no scripted-resize affordance. (No backticks in this
  # comment: it is inside a double-quoted ssh command, where the shell would run it.)
  $icd_env MFB_GTKAPP_HEADLESS=1 MFB_CANVAS_SYNC=1 MFB_CANVAS_DAMAGE=1 \
    MFB_CANVAS_RESIZE_W=900 MFB_CANVAS_RESIZE_H=640 MFB_CANVAS_STATS=$remote/dmg.txt \
    MFB_CANVAS_DUMP=$remote/dmg.rgba timeout 180 \$bin >/dev/null 2>&1
"
scp -P "$PORT" "$host:$remote/sw.rgba" "$work/sw.rgba" >/dev/null
scp -P "$PORT" "$host:$remote/gpu.rgba" "$work/gpu.rgba" >/dev/null
scp -P "$PORT" "$host:$remote/gpu.txt" "$work/gpu.txt" >/dev/null
scp -P "$PORT" "$host:$remote/sw2.rgba" "$work/sw2.rgba" >/dev/null
scp -P "$PORT" "$host:$remote/gpu2.rgba" "$work/gpu2.rgba" >/dev/null
scp -P "$PORT" "$host:$remote/gpu2.txt" "$work/gpu2.txt" >/dev/null
scp -P "$PORT" "$host:$remote/dmg.rgba" "$work/dmg.rgba" >/dev/null
scp -P "$PORT" "$host:$remote/dmg.txt" "$work/dmg.txt" >/dev/null
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

compare() {
python3 - "$1" "$2" "$3" <<'PY'
import sys

software = open(sys.argv[1], "rb").read()
gpu = open(sys.argv[2], "rb").read()
# The width only names the coordinate a beyond-tolerance pixel is reported at, so the
# resized case has to pass its own rather than inherit the default surface's.
width = int(sys.argv[3])
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
        first = (pixel % width, pixel // width, a.hex(), b.hex())
fraction = differing / total
if worst <= 2 and fraction <= 0.02:
    print(f"ok worst={worst} differing={fraction * 100:.4f}%")
else:
    print(f"worst={worst} differing={fraction * 100:.4f}% first-beyond-tolerance={first}")
PY
}

# Agreement is only meaningful if both frames actually contain the text. Two backends
# that each drew nothing agree perfectly, and the glyph arm is exactly the kind of thing
# that fails by drawing nothing — a wrong buffer offset reads zero coverage, which is
# transparent, which is invisible. So count the lit pixels in the label's own band
# first, in BOTH frames, before believing the diff.
glyphs() {
python3 - "$1" <<'PY'
import sys

frame = open(sys.argv[1], "rb").read()
width = 900
# The label sits at y=560 with size 90; the fixture glyph rises 0.3 em, so its ink runs
# from about y=533 to y=560. Sample the middle of that band.
row = 545
lit = sum(1 for x in range(width) if frame[(row * width + x) * 4 + 3] != 0)
print(lit)
PY
}

sw_lit="$(glyphs "$work/sw.rgba")"
gpu_lit="$(glyphs "$work/gpu.rgba")"
if [ "$sw_lit" -lt 50 ] || [ "$gpu_lit" -lt 50 ]; then
  fail "the text band is empty (software $sw_lit lit, GPU $gpu_lit lit) — the frames may agree only because neither drew it"
else
  pass "both backends drew the glyph run (software $sw_lit lit, GPU $gpu_lit lit)"
fi

verdict="$(compare "$work/sw.rgba" "$work/gpu.rgba" 900)"
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

# plan-98-F Phase 2: the resize handshake, end to end. `MFB_CANVAS_RESIZE_W`/`_H` make
# the headless main thread wait for the first completed frame and then call the very
# `_mfb_gtkapp_canvas_resize` GTK's "resize" signal calls — so what runs is the
# production path. Waiting for a frame first is the point: resizing before one exists
# would build the render target once at the new size and prove nothing, where resizing
# after one forces the Vulkan backend's tear-down-and-rebuild.
#
# `MFB_CANVAS_DUMP` overwrites, so the file left behind is the second frame and its
# length is the assertion that the new size actually reached the renderer.
expected=$((RESIZE_W * RESIZE_H * 4))
for who in sw2 gpu2; do
  actual=$(wc -c < "$work/$who.rgba" | tr -d ' ')
  if [ "$actual" = "$expected" ]; then
    pass "$who repainted at ${RESIZE_W}x${RESIZE_H} after the resize ($actual bytes)"
  else
    fail "$who is $actual bytes, expected $expected (${RESIZE_W}x${RESIZE_H}) — the resize did not reach the renderer"
  fi
done
if [ -s "$work/gpu2.txt" ] && [ "$(wc -l < "$work/gpu2.txt" | tr -d ' ')" -ge 2 ]; then
  pass "the resize produced a second frame rather than reusing the first"
else
  fail "the resized run wrote fewer than two stats lines — no repaint happened"
fi
verdict="$(compare "$work/sw2.rgba" "$work/gpu2.rgba" "$RESIZE_W")"
case "$verdict" in
  ok*) pass "the resized Vulkan render matches the software oracle ($verdict)" ;;
  *)   fail "the resized Vulkan render disagrees with the software oracle: $verdict" ;;
esac

# plan-98-G Phase 3: the damage union's empty case. The scene did not change and the
# surface did not change size, so the wake the resize produced owes no pixels — and the
# renderer returns before rasterising anything. Nothing else in the tree can reach this:
# a program's own repeated `present` is refused by `publishScene` long before the
# renderer sees it, so an unchanged wake has to come from the platform.
if [ ! -s "$work/dmg.txt" ]; then
  fail "the damage run wrote no stats line"
else
  skipped="$(tail -1 "$work/dmg.txt" | tr ' ' '\n' | grep '^skipped=' | cut -d= -f2)"
  frames="$(tail -1 "$work/dmg.txt" | tr ' ' '\n' | grep '^frames=' | cut -d= -f2)"
  if [ "${skipped:-0}" -ge 1 ]; then
    pass "a same-size resize repainted nothing (frames=$frames skipped=$skipped)"
  else
    fail "a same-size resize re-rendered the whole scene (frames=$frames skipped=${skipped:-0})"
  fi
  # And it still produced a correct frame — the skip must not cost the surface its
  # contents. Compared against the software oracle from the first run, which drew the
  # same scene at the same size.
  verdict="$(compare "$work/sw.rgba" "$work/dmg.rgba" 900)"
  case "$verdict" in
    ok*) pass "the frame surviving a skipped repaint still matches the oracle ($verdict)" ;;
    *)   fail "the frame after a skipped repaint disagrees with the oracle: $verdict" ;;
  esac
fi

if [ "$fails" -eq 0 ]; then
  echo "canvas Vulkan runtime tests passed"
else
  echo "$fails failure(s)"
  exit 1
fi
