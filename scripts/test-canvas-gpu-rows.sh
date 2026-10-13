#!/usr/bin/env bash
# The bug-686 scale rows on the Vulkan backend, and the shared native geometry paths on
# the Vulkan targets (bug-688).
#
# bug-686 made Metal draw ordinary scenes on the GPU: 5,000 quads, 40,000 polygon edges,
# 4,200 gradient stops, a 2,000-tile tilemap, a 1080p background, polygons of any edge
# count, a layered scene. `tests/canvas/rt_canvas_metal.rs` holds those rows for Metal.
# This script holds the SAME programs to the same bar on Vulkan: each is rendered on the
# box twice, once with `MFB_CANVAS_GPU=1` and once without, and the GPU frame must
#
#   * come from the GPU (`vulkanReady=TRUE` and `gpuFrames` non-zero — a declined scene
#     falls back to software, and software agrees with itself), and
#   * agree with the software oracle within `Tolerance::GPU_DEFAULT`.
#
# The scenes are read from `tests/canvas/scenes/`, the files the Rust suites include, so
# the Metal rows and these rows cannot drift into testing different programs.
#
# The 40,000-edge row is `forty_thousand_opaque_edges.mfb`, not the translucent
# `forty_thousand_edges.mfb` Metal also draws: that one stacks 200 alpha-60 rings a pixel
# apart, and on Mesa's lavapipe every pixel 20 or more blends deep drifts one step from
# the oracle (11.8% of the frame, past GPU_DEFAULT's 2%) — blend precision, not edges.
#
# The six `*_on_black` rows are bug-687's: shapes over the default black surface, where
# one step of coverage is several sRGB levels, so a GPU distance that rounds to a different
# coverage step than the oracle's shows past `GPU_DEFAULT`. The eleven after them are
# bug-735's: the same bar for every other shape kind, a non-grid stroke half-width, a
# non-grid group offset, transformed and clipped shapes, and rectangle and clip edges
# placed beside a coverage step; and `hairline`, a blended item stroked under the 16.16
# step, which the draw list and the emitters used to split differently. `groupaffine` is
# plan-172-H's: groups turned, scaled unevenly and nested, carrying a gradient, a
# rectangle clip, a stroke and a run of text (the fixture font is shipped with it).
# `autopaint` and `autopaintgroup` are plan-172-K's: one scene of `canvas::AutoPaint`
# buttons rendered HOVERED, the pointer put on a button by the row's fourth field — an
# environment assignment the row's runs are given (`MFB_CANVAS_POINTER`, the `--debug`
# pointer script). The first hovers a button of the scene itself, the second one use of
# a group that is drawn twice, which is the case that needs a group run of its own.
# The six `curve*` rows are plan-164-C's: `canvas::PolyCurve` and `canvas::CurvePolygon`
# under every cap, join, dash and fill rule, under their own transforms and inside turned
# groups, under an `AutoPaint` (at rest and hovered), as a `Picture`'s shape, and a few
# thousandths of a unit across under a 30,000-fold transform, which draws right only if
# every piece's residual words reached the buffer. They are judged under
# `Tolerance::GPU_CURVE`. `vertexrow` is the bug that plan found in the POLYGON path:
# polygons with a vertex exactly on a pixel-centre row, where the double-float crossing
# test rebuilt an edge's end as `a + (b - a)` and two edges disagreed about the vertex
# they share -- one whole row of the shape inverted.
# `pictureshapes` and `pictureauto` are plan-172-L's: pictures inside shapes — each a
# picture record with a polygonal clip and, where it strokes, the shape itself — at
# rest, and with the pointer on the one whose shape is an `AutoPaint`, both of whose
# entries are then swapped for the hover paint's.
# `xformclips` is bug-995's: path clips on items under a transform (scales 0.1, 1 and 4, a
# shear) and on untransformed items inside groups with a matrix, banded and not, with a
# rectangle clip under a sheared group -- every pixel takes the five-sample clip path
# unless its centre sample decides it, and that shortcut has to change no pixel. Compare
# two builds' frames of it byte for byte with `--keep`.
# `texeledge` is bug-1079's: plain pictures whose texel boundaries are pixel centres, in
# x and in y, where the texel taken used to hang on the last bit of the driver's
# division (box 206's Intel driver drew the neighbouring row).
# The `items*` and `pass*` rows are bug-765's: a frame that does not fit one frame buffer
# is drawn in passes. `many_items.mfb` at 65,535, 65,536 (one buffer's item region),
# 65,537 (a second pass of one item) and 140,000 items (three passes), its translucent
# squares lying over their neighbours and over the lap before; and one frame each that is
# two passes long for its polygon edges, its gradient stops and its picture texels alone;
# and `passes_grouped.mfb`, where a group is drawn in both passes of a frame and a run of
# the scene's own items is cut by the end of the first.
#
# Every run sets `MFB_CANVAS_GEO_VERIFY=1`, and four more rows are the
# `tests/canvas/rt_canvas_geo_native.rs` programs. bug-686's native geometry builder,
# item hash, frame pass and scene sequence lock are emitted through arch-neutral code and
# were only ever run on macOS AArch64; this is where they run on x86-64 (Windows, Linux)
# and on Linux AArch64, and `geoVerifyMismatches` must be 0 on every one. The publish-race
# row runs WITHOUT `MFB_CANVAS_SYNC`, so the graphics thread renders while the worker
# publishes — the case the sequence lock exists for.
#
# Usage: scripts/test-canvas-gpu-rows.sh <mfb-exe> [--target <t>] [--box <port>]
#                                        [--libc glibc|musl] [--icd auto|<manifest>]
#                                        [--rows <name,name,...>] [--keep <dir>]
#
#   --target  linux-x86_64 (default, box 2228), linux-aarch64 (box 2226) or
#             windows-x86_64 (box 206, `test@192.168.1.206`: real hardware, an Intel
#             UHD 630 with Intel's own Vulkan driver).
#   --box     the ssh port, when it is not the target's default box. A port names one
#             of the forwarded boxes on 127.0.0.1 (2230 for Windows lavapipe).
#
# Windows rows run on 206, not 2230, by owner decision (bug-741): 2230's lavapipe faults
# inside `vulkan_lvp.dll` in about 1 GPU run in 7 whatever the scene, and 206's real
# driver ran the same program 40 of 40 times clean. 2230 stays for non-GPU Windows
# tests.
#   --libc    which AppImage to ship on Linux (default glibc; box 2227 is musl).
#   --icd     a Linux Vulkan driver manifest, or `auto` to provision Mesa's software
#             driver on an Alpine box — as `test-canvas-vulkan.sh --icd`.
#   --rows    run only these rows (names as listed below).
#   --keep    copy each render row's two frames there as `<row>-sw.rgba` and
#             `<row>-gpu.rgba` (raw RGBA8, 900 wide), to compare a GPU frame across two
#             builds byte for byte.
#
# A box whose Vulkan device does not build (`vulkanReady=FALSE`) FAILS the render rows
# rather than skipping them: a row that cannot reach the GPU proves nothing about it. Run
# it where a Vulkan driver exists.
set -euo pipefail
. "$(dirname "$0")/remote-common.sh"

MFB_EXE="${1:?usage: test-canvas-gpu-rows.sh <mfb-exe> [--target <t>] [--box <port>]}"
shift || true
TARGET=linux-x86_64
PORT=""
LIBC=glibc
ICD=""
ONLY=""
KEEP=""
while [ $# -gt 0 ]; do
  case "$1" in
    --target) TARGET="$2"; shift 2 ;;
    --box) PORT="$2"; shift 2 ;;
    --libc) LIBC="$2"; shift 2 ;;
    --icd) ICD="$2"; shift 2 ;;
    --rows) ONLY="$2"; shift 2 ;;
    --keep) KEEP="$2"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
case "$TARGET" in
  linux-x86_64) PORT="${PORT:-2228}" ;;
  linux-aarch64) PORT="${PORT:-2226}" ;;
  windows-x86_64) PORT="${PORT:-22}" ;;
  *) echo "unsupported --target $TARGET" >&2; exit 2 ;;
esac

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MFB_EXE="$(cd "$(dirname "$MFB_EXE")" && pwd)/$(basename "$MFB_EXE")"
cd "$ROOT"
rc_workdir
SCENES="tests/canvas/scenes"
host="test@127.0.0.1"
# Port 22 is box 206 on the LAN; every other port is a box forwarded to localhost.
[ "$PORT" = 22 ] && host="test@192.168.1.206"

# name | scene file | kind [| NAME=value]
#   render   sw + gpu, compared; the gpu frame must be a GPU frame
#   curve    as `render`, judged under Tolerance::GPU_CURVE (a frame that draws a curve
#            shape; `tests/common/canvas_image.rs`)
#   geo      one GPU run with MFB_CANVAS_SYNC; geometry verification only
#   race     one GPU run WITHOUT MFB_CANVAS_SYNC; geometry verification only
# The optional fourth field is one environment assignment every run of the row gets.
ROWS="
quads|five_thousand_quads|render
edges|forty_thousand_opaque_edges|render
gradients|many_gradients|render
tilemap|tilemap|render
background|background|render
polygons|large_polygons|render
huge|huge_polygon|render
layers|layered|render
star251x97|star_251_97_on_black|render
star301x97|star_301_97_on_black|render
star301x2|star_301_2_on_black|render
star301x2nz|star_301_2_nonzero_on_black|render
fillrules|fill_rules_on_black|render
squarecaps|square_caps_on_black|render
joins|joins_on_black|render
dashes|dashes_on_black|render
pathclips|path_clips_on_black|render
xformclips|transformed_clips_on_black|render
gradientsblack|gradients_on_black|render
polygonsblack|large_polygons_on_black|render
smallpolysblack|small_polygons_on_black|render
rectsblack|rectangles_on_black|render
circlesblack|circles_on_black|render
ellipsesblack|ellipses_on_black|render
linesblack|lines_on_black|render
arcsblack|arcs_on_black|render
strokedpolysblack|stroked_polygons_on_black|render
polylinesblack|polylines_on_black|render
groupsblack|grouped_polygons_on_black|render
strokedblack|stroked_shapes_on_black|render
rectstepblack|rectangles_at_a_step_on_black|render
clipstepblack|clips_at_a_step_on_black|render
xformblack|transformed_shapes_on_black|render
cornersblack|rect_corners_on_black|render
groupaffine|group_affine_on_black|render
autopaint|autopaint_on_black|render|MFB_CANVAS_POINTER=0,280,120,0,1
autopaintgroup|autopaint_on_black|render|MFB_CANVAS_POINTER=0,150,320,0,1
pictureshapes|picture_shapes_on_black|render
pictureauto|picture_shapes_on_black|render|MFB_CANVAS_POINTER=0,770,470,0,1
texeledge|picture_texel_edge|render
hairline|blended_hairline_stroke|render
curves|curves|curve
curveshover|curves|curve|MFB_CANVAS_POINTER=0,200,440,0,1
curvestrokes|curve_strokes_on_black|curve
curvesplaced|curves_placed_on_black|curve
curvepictures|curve_pictures_on_black|curve
curveresiduals|curve_residuals_on_black|curve
vertexrow|vertex_on_a_pixel_row|render
items65535|many_items|render|MFB_TEST_ITEMS=65535
items65536|many_items|render|MFB_TEST_ITEMS=65536
items65537|many_items|render|MFB_TEST_ITEMS=65537
items140000|many_items|render|MFB_TEST_ITEMS=140000
passedges|past_the_edge_region_opaque|render
passstops|many_gradient_stops|render
passtexels|many_texels|render
passgroups|passes_grouped|render
matrix|canvas_geo_native_matrix|geo
rehash|canvas_geo_native_rehash|geo
framepass|canvas_geo_native_frame_pass|geo
race|canvas_geo_native_publish_race|race
"

# One `name=value` field of a stats line.
field() { echo "$1" | tr ' ' '\n' | grep "^$2=" | head -1 | cut -d= -f2; }

# Linux: the Vulkan driver, provisioned exactly as test-canvas-vulkan.sh does.
icd_env=""
if [ -n "$ICD" ] && [ "${TARGET%%-*}" = linux ]; then
  if [ "$ICD" = auto ]; then
    remote_ssh "$PORT" "$host" '
      set -e
      dir=/tmp/mfb-vulkan-icd
      manifest=$dir/usr/share/vulkan/icd.d/lvp_icd.x86_64.json
      if [ ! -f "$manifest" ]; then
        mkdir -p $dir && cd $dir
        base=http://dl-cdn.alpinelinux.org/alpine/v3.24/main/x86_64
        for pkg in mesa-vulkan-swrast libdisplay-info; do
          file=$(wget -qO- "$base/" | grep -o "${pkg}-[0-9][^\"]*\.apk" | head -1)
          wget -q "$base/$file" -O "$pkg.apk"
          tar -xzf "$pkg.apk" 2>/dev/null || true
        done
        sed -i "s|/usr/lib/libvulkan_lvp.so|$dir/usr/lib/libvulkan_lvp.so|" "$manifest"
      fi
      test -f "$manifest"
    '
    ICD=/tmp/mfb-vulkan-icd/usr/share/vulkan/icd.d/lvp_icd.x86_64.json
  fi
  icd_env="VK_ICD_FILENAMES=$ICD LD_LIBRARY_PATH=$(dirname "$(dirname "$(dirname "$(dirname "$ICD")")")")/lib"
fi

# usage: run_remote <row> <tag> <sync 0|1> <gpu 0|1>
# Runs the row's program once on the box and fetches `<tag>.txt` (stats) and, for a
# render row, `<tag>.rgba` (the dump) into $work/<row>/.
run_remote() {
  local row=$1 tag=$2 sync=$3 gpu=$4
  if [ "$TARGET" = windows-x86_64 ]; then
    local dir="C:\\mfbrows-$$\\$row"
    {
      echo "@echo off"
      echo "setlocal"
      echo "set MFB_HEADLESS=1"
      echo "set MFB_CANVAS_GEO_VERIFY=1"
      [ "$sync" = 1 ] && echo "set MFB_CANVAS_SYNC=1"
      [ "$gpu" = 1 ] && echo "set MFB_CANVAS_GPU=1"
      [ -z "$ROW_ENV" ] || echo "set $ROW_ENV"
      echo "set MFB_CANVAS_STATS=$dir\\$tag.txt"
      echo "set MFB_CANVAS_DUMP=$dir\\$tag.rgba"
      echo "cd /d $dir"
      echo "del /q $tag.txt $tag.rgba 2>nul"
      echo "$row.exe > $tag.out 2>&1"
      echo "echo rc=%errorlevel%"
    } | sed 's/$/\r/' > "$work/$row/$tag.bat"
    remote_scp "$PORT" "$work/$row/$tag.bat" "$host:C:/mfbrows-$$/$row/$tag.bat" >/dev/null
    remote_ssh "$PORT" "$host" "C:\\mfbrows-$$\\$row\\$tag.bat" 2>/dev/null | tr -d '\r' | grep '^rc=' > "$work/$row/$tag.rc" || true
    remote_scp "$PORT" "$host:C:/mfbrows-$$/$row/$tag.txt" "$work/$row/$tag.txt" >/dev/null 2>&1 || true
    remote_scp "$PORT" "$host:C:/mfbrows-$$/$row/$tag.rgba" "$work/$row/$tag.rgba" >/dev/null 2>&1 || true
  else
    local dir="/tmp/mfb-rows-$$/$row" env="MFB_HEADLESS=1 MFB_CANVAS_GEO_VERIFY=1"
    [ "$sync" = 1 ] && env="$env MFB_CANVAS_SYNC=1"
    [ "$gpu" = 1 ] && env="$env MFB_CANVAS_GPU=1"
    [ -z "$ROW_ENV" ] || env="$env $ROW_ENV"
    remote_ssh "$PORT" "$host" "
      cd $dir
      $icd_env $env MFB_CANVAS_STATS=$dir/$tag.txt MFB_CANVAS_DUMP=$dir/$tag.rgba \
        timeout 1800 ./squashfs-root/usr/bin/$row > $tag.out 2>&1
      echo rc=\$?
    " > "$work/$row/$tag.rc" 2>/dev/null || true
    remote_scp "$PORT" "$host:$dir/$tag.txt" "$work/$row/$tag.txt" >/dev/null 2>&1 || true
    remote_scp "$PORT" "$host:$dir/$tag.rgba" "$work/$row/$tag.rgba" >/dev/null 2>&1 || true
  fi
}

# usage: ship <row> — build the row's program for the target and put it on the box.
ship() {
  local row=$1 scene=$2
  local proj="$work/$row"
  scaffold_project "$proj" "$row"
  cp "$SCENES/$scene.mfb" "$proj/src/main.mfb"
  "$MFB_EXE" build --app --debug --target "$TARGET" "$proj" >/dev/null
  # A scene that draws text loads the fixture font from beside the program, which is the
  # directory the row runs in.
  local font=""
  if grep -q 'fixture\.ttf' "$proj/src/main.mfb"; then
    font="$proj/fixture.ttf"
    rc_fixture_font "$font"
  fi
  if [ "$TARGET" = windows-x86_64 ]; then
    win_ship "$PORT" "$host" "C:\\mfbrows-$$\\$row" "$proj/build/$row.exe"
    [ -z "$font" ] || remote_scp "$PORT" "$font" "$host:C:/mfbrows-$$/$row/fixture.ttf" >/dev/null
  else
    local dir="/tmp/mfb-rows-$$/$row"
    remote_ssh "$PORT" "$host" "rm -rf $dir && mkdir -p $dir"
    remote_scp "$PORT" "$proj/build/$row-$LIBC.AppImage" "$host:$dir/app.AppImage" >/dev/null
    [ -z "$font" ] || remote_scp "$PORT" "$font" "$host:$dir/fixture.ttf" >/dev/null
    remote_ssh "$PORT" "$host" "cd $dir && ./app.AppImage --appimage-extract >/dev/null 2>&1"
  fi
}

# Every geometry run: the native records and resolved indices the verifier checked, and
# no mismatch among them. `rc` first: a run that faulted wrote a partial stats file.
check_geometry() {
  local row=$1 tag=$2 stats=$3
  if ! grep -q '^rc=0' "$work/$row/$tag.rc"; then
    fail "$row/$tag: the program did not exit 0 ($(cat "$work/$row/$tag.rc"))"
  fi
  local mismatches
  mismatches="$(field "$stats" geoVerifyMismatches)"
  if [ "$mismatches" = 0 ]; then
    pass "$row/$tag: geoVerifyMismatches=0 (geoVerified=$(field "$stats" geoVerified) geoResolvedChecked=$(field "$stats" geoResolvedChecked) drawsVerified=$(field "$stats" drawsVerified))"
  else
    fail "$row/$tag: geoVerifyMismatches=${mismatches:-missing} — a native geometry record, hash fold or draw layout differs from the MFBASIC one on $TARGET: $stats"
  fi
}

echo "=== bug-688 rows on $TARGET, $host port $PORT ==="
if [ "$TARGET" != windows-x86_64 ]; then
  remote_ssh "$PORT" "$host" "rm -rf /tmp/mfb-rows-$$"
fi
# fd 3, not stdin: every ssh in the loop reads stdin and would swallow the rest of the list.
ROW_ENV=""
while IFS='|' read -r row scene kind ROW_ENV <&3; do
  [ -n "$row" ] || continue
  if [ -n "$ONLY" ] && ! echo ",$ONLY," | grep -q ",$row,"; then
    continue
  fi
  echo "--- $row ($scene, $kind) ---"
  ship "$row" "$scene"
  case "$kind" in
    render|curve)
      run_remote "$row" sw 1 0
      run_remote "$row" gpu 1 1
      sw_stats="$(tail -1 "$work/$row/sw.txt" 2>/dev/null || true)"
      gpu_stats="$(tail -1 "$work/$row/gpu.txt" 2>/dev/null || true)"
      echo "    gpu: $gpu_stats"
      if [ -z "$sw_stats" ] || [ -z "$gpu_stats" ]; then
        fail "$row: a run wrote no stats line (sw $(cat "$work/$row/sw.rc"), gpu $(cat "$work/$row/gpu.rc"))"
        continue
      fi
      check_geometry "$row" sw "$sw_stats"
      check_geometry "$row" gpu "$gpu_stats"
      case "$gpu_stats" in
        *vulkanReady=TRUE*) ;;
        *) fail "$row: vulkanReady is not TRUE — no Vulkan device, so nothing below is evidence"; continue ;;
      esac
      frames="$(field "$gpu_stats" gpuFrames)"
      if [ "${frames:-0}" -gt 0 ]; then
        pass "$row: drawn on Vulkan (gpuFrames=$frames)"
      else
        fail "$row: declined to software (gpuFrames=${frames:-missing}) — __canvas_vulkanRenderable refused the frame"
      fi
      lit="$(python3 -c "import sys; d=open(sys.argv[1],'rb').read(); print(sum(1 for i in range(0,len(d),4) if d[i:i+3]!=b'\0\0\0'))" "$work/$row/sw.rgba" 2>/dev/null || echo 0)"
      if [ "$lit" -eq 0 ]; then
        fail "$row: the software frame is empty, so a comparison would prove nothing"
        continue
      fi
      if [ -n "$KEEP" ]; then
        mkdir -p "$KEEP"
        cp "$work/$row/sw.rgba" "$KEEP/$row-sw.rgba"
        cp "$work/$row/gpu.rgba" "$KEEP/$row-gpu.rgba"
      fi
      tolerance=""
      [ "$kind" = curve ] && tolerance=curve
      verdict="$(python3 scripts/rgba_compare.py "$work/$row/sw.rgba" "$work/$row/gpu.rgba" 900 $tolerance 2>&1 || true)"
      case "$verdict" in
        ok*) pass "$row: Vulkan matches the software oracle ($verdict, $lit lit)" ;;
        *) fail "$row: Vulkan disagrees with the software oracle: $verdict" ;;
      esac
      ;;
    geo|race)
      sync=1
      [ "$kind" = race ] && sync=0
      run_remote "$row" gpu "$sync" 1
      stats="$(tail -1 "$work/$row/gpu.txt" 2>/dev/null || true)"
      echo "    $stats"
      if [ -z "$stats" ]; then
        fail "$row: no stats line ($(cat "$work/$row/gpu.rc"))"
        continue
      fi
      check_geometry "$row" gpu "$stats"
      if [ "$kind" = race ]; then
        # The race needs frames to OVERLAP the publishes. Since bug-734 the scene keeps
        # publishing until the graphics thread has drawn 3 frames, so a slow renderer
        # gets a longer window instead of too few frames: at a fixed 60 publishes the
        # emulated Windows box drew 2 on either renderer. The software fall-back below
        # (bug-688) stays for a GPU run that still ends short; the sequence lock is the
        # same code under either renderer.
        tag=gpu
        frames="$(wc -l < "$work/$row/gpu.txt" | tr -d ' ')"
        if [ "$frames" -lt 3 ]; then
          echo "    the GPU run rendered only $frames frames; racing on the software renderer"
          run_remote "$row" sw 0 0
          tag=sw
          stats="$(tail -1 "$work/$row/sw.txt" 2>/dev/null || true)"
          echo "    $stats"
          check_geometry "$row" sw "$stats"
          frames="$(wc -l < "$work/$row/sw.txt" 2>/dev/null | tr -d ' ')"
        fi
        checked="$(field "$stats" geoResolvedChecked)"
        if [ "${frames:-0}" -ge 3 ] && [ "${checked:-0}" -ge 6000 ]; then
          pass "$row/$tag: $frames frames rendered during the publishes, $checked resolved indices checked"
        else
          fail "$row/$tag: the race did not happen (${frames:-0} frames, geoResolvedChecked=${checked:-missing})"
        fi
      else
        verified="$(field "$stats" geoVerified)"
        if [ "${verified:-0}" -gt 0 ]; then
          pass "$row: the native builder ran and was checked (geoVerified=$verified)"
        else
          fail "$row: nothing was verified (geoVerified=${verified:-missing})"
        fi
      fi
      ;;
  esac
done 3<<< "$ROWS"

# The work dirs carry this run's pid, on Windows as on Linux, so two runs against one box
# (several agents share a box) never overwrite each other's programs or frames.
if [ "$TARGET" != windows-x86_64 ]; then
  remote_ssh "$PORT" "$host" "rm -rf /tmp/mfb-rows-$$"
else
  remote_ssh "$PORT" "$host" "rmdir /s /q C:\\mfbrows-$$" >/dev/null 2>&1 || true
fi
if [ "$rc_failures" -eq 0 ]; then
  echo "bug-688 rows passed on $TARGET"
else
  echo "$rc_failures failure(s) on $TARGET"
  exit 1
fi
