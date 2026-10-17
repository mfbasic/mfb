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
#                                      [--target linux-x86_64|linux-aarch64]
#                                      [--keep <dir>]
#
#   --box <port>   ssh port of the target box (default 2228, Ubuntu x86_64 glibc).
#   --target <t>   what to build for (default linux-x86_64). Box 2226 is linux-aarch64.
#   --keep <dir>   copy the groups scene's two frames there as `groups-sw.rgba` and
#                  `groups-gpu.rgba` (raw RGBA8, 900 wide). A change that must not move
#                  a translate-only group's GPU frame is checked by keeping the frame
#                  before and after and comparing the two files.
#   --libc <l>     which AppImage to ship (default glibc). It must match the box:
#                  musl's loader absorbs the glibc compat sonames, so shipping the
#                  wrong one does not fail cleanly — box 2227 is musl.
#
# The box needs a Vulkan loader and an ICD. It does NOT need a display server: the
# renderer draws offscreen and reads back (plan-98-F Correction 1), and
# `MFB_HEADLESS` skips GTK entirely. That is deliberate — no reachable Linux
# box has a display, so a design that needed one could not be tested at all.
#
# A box that cannot build a pipeline reports `vulkanReady=FALSE`; this skips rather
# than fails there, because "no usable GPU here" is a real configuration and not a
# regression. The skip gates on that one flag deliberately — it is the flag the
# renderer itself gates on, so the test and the runtime can never disagree about
# whether the GPU path was taken.
set -euo pipefail
. "$(dirname "$0")/remote-common.sh"

MFB_EXE="${1:?usage: test-canvas-vulkan.sh <mfb-exe> [--box <port>]}"
shift || true
PORT=2228
LIBC=glibc
TARGET=linux-x86_64
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
KEEP=""
while [ $# -gt 0 ]; do
  case "$1" in
    --box) PORT="$2"; shift 2 ;;
    --libc) LIBC="$2"; shift 2 ;;
    --target) TARGET="$2"; shift 2 ;;
    --icd) ICD="$2"; shift 2 ;;
    --keep) KEEP="$2"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MFB_EXE="$(cd "$(dirname "$MFB_EXE")" && pwd)/$(basename "$MFB_EXE")"
# Every relative path below (the GROUPS extraction included) is repo-relative, so the
# script runs from any cwd. After MFB_EXE is made absolute, which needs the caller cwd.
cd "$ROOT"
rc_workdir

proj="$work/vkcanvas"
mkdir -p "$proj/src"
scaffold_project "$proj" vkcanvas

# The fixture font, so the scene can contain text.
#
# Synthesized rather than borrowed from the box: a system font would make the
# comparison depend on which fonts the box happens to have, and the point of this test
# is that the two *backends* agree — not that a particular typeface renders. This is
# the same twelve-glyph file `tests/canvas/rt_canvas_font.rs` builds: `unitsPerEm` 1000, one
# square glyph at (100,0)-(400,300), so a `Text` item is a row of squares whose pixels
# are easy to reason about and whose bitmaps are far inside both backends' caps.
rc_fixture_font "$proj/fixture.ttf"

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
IMPORT vector
IMPORT color
IMPORT io
IMPORT math
IMPORT os
FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e)
    RETURN
  END TRAP
  LET yellow AS color::Color = color::rgb(255, 255, 0)
  LET green AS color::Color = color::rgb(0, 160, 0)
  LET head AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 450.0, y := 320.0], radius := 150.0, paint := canvas::fill(yellow), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET eyeL AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 400.0, y := 280.0], radius := 22.0, paint := canvas::fill(green), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET eyeR AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 500.0, y := 280.0], radius := 22.0, paint := canvas::fill(green), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET smile AS canvas::DrawItem = canvas::Arc[id := "", position := vector::Float2[x := 450.0, y := 335.0], radius := 90.0, startAngle := 0.0, endAngle := 3.14159, paint := WITH canvas::stroke(green, 14.0) { cap := canvas::CapStyle.Butt }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET box AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 10.0, y := 10.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 50.0, height := 50.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET rounded AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 100.0, y := 10.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 90.0, height := 60.0], radiusTL := 18.0, radiusTR := 18.0, radiusBL := 18.0, radiusBR := 18.0, paint := canvas::fillStroke(color::rgb(0, 0, 255), color::rgb(255, 255, 255), 4.0), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET line AS canvas::DrawItem = canvas::Line[id := "", position := vector::Float2[x := 0.0, y := 0.0], startAt := vector::Float2[x := 220.0, y := 20.0], endAt := vector::Float2[x := 380.0, y := 90.0], paint := WITH canvas::stroke(color::rgb(255, 128, 0), 9.0) { cap := canvas::CapStyle.Round }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET faint AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 600.0, y := 40.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 120.0, height := 80.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgba(0, 200, 255, 180)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET tri AS canvas::DrawItem = canvas::Polygon[id := "", position := vector::Float2[x := 0.0, y := 0.0], contours := [[vector::Float2[x := 620.0, y := 200.0], vector::Float2[x := 740.0, y := 200.0], vector::Float2[x := 680.0, y := 300.0]]], rule := canvas::FillRule.EvenOdd, paint := canvas::fill(color::rgb(200, 0, 200)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET arrow AS canvas::DrawItem = canvas::Polygon[id := "", position := vector::Float2[x := 0.0, y := 0.0], contours := [[vector::Float2[x := 60.0, y := 400.0], vector::Float2[x := 160.0, y := 400.0], vector::Float2[x := 160.0, y := 360.0], vector::Float2[x := 230.0, y := 430.0], vector::Float2[x := 160.0, y := 500.0], vector::Float2[x := 160.0, y := 460.0], vector::Float2[x := 60.0, y := 460.0]]], rule := canvas::FillRule.EvenOdd, paint := canvas::fillStroke(color::rgb(0, 180, 180), color::rgb(20, 20, 20), 6.0), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  ' TRANSLUCENT, deliberately (plan-116-A). The fixture glyph is an axis-aligned opaque
  ' square, so its coverage is binary -- and compositing an opaque square over itself is
  ' idempotent, which means a renderer that drew every glyph TWICE produced a
  ' byte-identical frame and no assertion here could see it. At alpha 160 a second
  ' composite is arithmetically different from one, so a duplicated glyph draw becomes a
  ' pixel difference against the oracle. That is what makes `tail` below a real gate.
  LET label AS canvas::DrawItem = canvas::Text[id := "", position := vector::Float2[x := 300.0, y := 560.0], text := "AAAA", fonts := [face], fontSize := 90.0, letterSpacing := 0.0, paint := canvas::fill(color::rgba(220, 40, 160, 160)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  ' plan-116-A: a shape AFTER the glyph run, and the scene's only reason for it.
  '
  ' The item block now rides a per-frame buffer indexed by instance, and consecutive
  ' non-text items are drawn as ONE instanced call. A glyph run ends that run, and the
  ' next run has to restart at the cursor the glyphs advanced to. With the label last,
  ' the final flush is always empty, so a renderer that forgot to move the run base past
  ' the glyphs would still pass -- and would then redraw every glyph quad as a shape the
  ' moment any scene put a shape after its text. Sat clear of the label's band (row 545)
  ' and of `faint`/`tri`, so the existing lit-pixel and diff assertions keep meaning what
  ' they meant.
  LET tail AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 820.0, y := 180.0], radius := 40.0, paint := canvas::fill(color::rgb(120, 220, 60)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  ' plan-116-B: one item per non-Normal BlendMode, and one clipped item.
  '
  ' Without these the frame never binds a pipeline other than Normal's and never takes
  ' the clip-coverage path, so three of the four pipelines this letter builds would go
  ' entirely unexercised on the GPU while the suite still reported success.
  '
  ' They sit on a mid-grey patch on purpose: over black, Multiply is a no-op and Screen
  ' and Add are indistinguishable from Normal, so a wrong pipeline would look right.
  '
  ' `blendStroke` is the sharp one -- a non-Normal item that BOTH fills and strokes,
  ' which the shaders cannot compose in one pass (the stroke-over-fill identity is
  ' Normal-only), so it must be emitted as two adjacent instances. If that split is
  ' missing, this item alone diverges from the oracle.
  LET ground AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 20.0, y := 240.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 360.0, height := 120.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(128, 128, 128)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  ' Deliberately SMALL. Each one only has to bind its pipeline and take its arm; the
  ' area buys nothing. It costs, though: a blended pixel agrees with the oracle to
  ' within one or two steps but rarely exactly, because the oracle blends through a
  ' 16-bit linear table and the hardware blends in float. Measured on this scene --
  ' with these four set to Normal the frame matches at worst=1 differing=0.4677%, and
  ' with the modes restored at radius 40 it was worst=2 differing=2.8012%, i.e. every
  ' extra blended pixel is a differing pixel. `Tolerance::GPU_DEFAULT`'s per-channel
  ' bound is the correctness signal and it holds either way; its 2% population budget
  ' is a fraction of the WHOLE frame, so a large blended patch would exhaust it
  ' without testing anything the small one does not.
  LET blendMul AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 70.0, y := 300.0], radius := 14.0, paint := WITH canvas::fill(color::rgb(230, 120, 40)) { blend := canvas::BlendMode.Multiply }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET blendScr AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 170.0, y := 300.0], radius := 14.0, paint := WITH canvas::fill(color::rgb(230, 120, 40)) { blend := canvas::BlendMode.Screen }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET blendAdd AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 270.0, y := 300.0], radius := 14.0, paint := WITH canvas::fill(color::rgb(230, 120, 40)) { blend := canvas::BlendMode.Add }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET blendStroke AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 350.0, y := 300.0], radius := 12.0, paint := WITH canvas::fillStroke(color::rgb(230, 120, 40), color::rgb(40, 120, 230), 8.0) { blend := canvas::BlendMode.Multiply }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET clippedBox AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 420.0, y := 240.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 300.0, height := 60.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH canvas::fill(color::rgb(255, 255, 255)) { clip := [[vector::Float2[x := 40.25, y := 0.0], vector::Float2[x := 240.75, y := 0.0], vector::Float2[x := 240.75, y := 60.0], vector::Float2[x := 40.25, y := 60.0]]] }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  ' plan-116-C: transformed items, so the shader's inverse-map path actually runs.
  ' Without these the frame never sets hasTransform and the whole of this letter's
  ' shader work would go unexercised while the suite still reported success.
  '
  ' A rotation, a non-uniform scale and a rotated TEXT run: the rotation exercises the
  ' gradient correction on a curved edge, the non-uniform scale is the case Phase 1
  ' measured sqrt(|det M|) as 37/255 wrong on, and the text takes the separate
  ' inverse-sample arm. Small, for the reason the blend items are small -- see the
  ' comment there.
  LET rotT AS canvas::Transform = canvas::Transform[a := 0.7071067811865476, b := 0.7071067811865476, c := 0.0 - 0.7071067811865476, d := 0.7071067811865476, tx := 0.7071067811865476 * (0.0 - 25.0) + (0.0 - 0.7071067811865476) * (0.0 - 25.0) + 120.0 + 25.0, ty := 0.7071067811865476 * (0.0 - 25.0) + 0.7071067811865476 * (0.0 - 25.0) + 560.0 + 25.0]
  LET rotBox AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 0.0 - 25.0, y := 0.0 - 25.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 50.0, height := 50.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 200, 40)), transform := rotT]
  LET scaleT AS canvas::Transform = canvas::Transform[a := 2.0, b := 0.0, c := 0.0, d := 1.0, tx := 250.0, ty := 560.0]
  LET scaleDot AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 0.0, y := 0.0], radius := 18.0, paint := canvas::fillStroke(color::rgb(90, 200, 255), color::rgb(255, 255, 255), 6.0), transform := scaleT]
  LET textT AS canvas::Transform = canvas::Transform[a := 0.0, b := 1.0, c := 0.0 - 1.0, d := 0.0, tx := 700.0, ty := 460.0]
  LET rotText AS canvas::DrawItem = canvas::Text[id := "", position := vector::Float2[x := 0.0, y := 0.0], text := "AA", fonts := [face], fontSize := 40.0, letterSpacing := 0.0, paint := canvas::fill(color::rgb(200, 255, 120)), transform := textT]
  ' plan-116-D: the same line twice, butt and round, so the SPIR-V cap arm actually
  ' runs. Without a butt one the branch is compiled into the blob and never taken, and
  ' the oracle comparison would agree everywhere the scene looks. Thick and short,
  ' because the cap is a half-width feature.
  LET capButt AS canvas::DrawItem = canvas::Line[id := "", position := vector::Float2[x := 0.0, y := 0.0], startAt := vector::Float2[x := 120.0, y := 600.0], endAt := vector::Float2[x := 240.0, y := 600.0], paint := WITH canvas::stroke(color::rgb(255, 240, 120), 24.0) { cap := canvas::CapStyle.Butt }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET capRound AS canvas::DrawItem = canvas::Line[id := "", position := vector::Float2[x := 0.0, y := 0.0], startAt := vector::Float2[x := 320.0, y := 600.0], endAt := vector::Float2[x := 440.0, y := 600.0], paint := WITH canvas::stroke(color::rgb(255, 240, 120), 24.0) { cap := canvas::CapStyle.Round }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  ' And a ROUND-capped arc: `smile` above is butt, which is what every arc was before
  ' plan-116-D, so without this the cap-disc arm is compiled into the SPIR-V and never
  ' taken.
  LET capArc AS canvas::DrawItem = canvas::Arc[id := "", position := vector::Float2[x := 620.0, y := 600.0], radius := 60.0, startAngle := 0.0, endAngle := 1.884955592153876, paint := WITH canvas::stroke(color::rgb(120, 255, 200), 20.0) { cap := canvas::CapStyle.Round }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  ' plan-116-E: a rotated, eccentric, stroked ellipse -- kind 7 is a brand-new geometry
  ' kind, so without one here the SPIR-V's ellipse arm is compiled and never taken and
  ' a predicate that accepted a kind the shader does not know would render it as
  ' NOTHING and report success (`.ai/canvas-threading.md` section 10).
  LET ell AS canvas::DrawItem = canvas::Ellipse[id := "", position := vector::Float2[x := 760.0, y := 430.0], radiusX := 110.0, radiusY := 38.0, paint := canvas::fillStroke(color::rgb(226, 150, 255), color::rgb(255, 255, 255), 8.0), transform := canvas::Transform[a := math::cos(0.5235987755982988), b := math::sin(0.5235987755982988), c := 0.0 - math::sin(0.5235987755982988), d := math::cos(0.5235987755982988), tx := 0.0, ty := 0.0]]
  ' plan-116-F: a linear ramp, a radial ramp, and a gradient-filled POLYGON. Without
  ' one here the SPIR-V's gradient walk is compiled into the blob and never taken, and
  ' a shader that could not read the stops would draw the flat `fill` beneath -- which
  ' every one of these sets to black on purpose, so that failure is a black shape and
  ' not a subtly wrong one.
  '
  ' The polygon is the one that earns its place. A gradient's stops sit at the END of
  ' the geometry record, so for a polygon the tail is edges *then* stops and the upload
  ' finds its base by subtracting from the record's own length. On any other kind the
  ' stops start right after the header and a base computed either way agrees -- so a
  ' polygon is the only shape that can tell a correct base from a lucky one.
  '
  ' Two gradient items rather than one, for the reason there are two polygons above:
  ' with a single one, a per-item first-stop index of zero would pass whether or not it
  ' was ever written.
  LET rampStops AS List OF canvas::GradientStop = [canvas::GradientStop[offset := 0.0, color := color::rgb(255, 64, 32)], canvas::GradientStop[offset := 0.55, color := color::rgb(250, 230, 90)], canvas::GradientStop[offset := 1.0, color := color::rgb(32, 96, 255)]]
  LET gLin AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, startPoint := vector::Float2[x := 0.0, y := 0.0], endPoint := vector::Float2[x := 150.0, y := 60.0], stops := rampStops, spread := canvas::GradientSpread.Pad, transform := canvas::Transform[a := 0.0, b := 0.0, c := 0.0, d := 0.0, tx := 0.0, ty := 0.0], focalOffset := vector::Float2[x := 0.0, y := 0.0], focalRadius := 0.0, strokeApply := canvas::GradientApply.FillOnly]
  LET gradBar AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 430.0, y := 100.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 150.0, height := 60.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH canvas::fill(color::rgb(0, 0, 0)) { gradient := gLin }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET orbStops AS List OF canvas::GradientStop = [canvas::GradientStop[offset := 0.0, color := color::rgb(255, 244, 214)], canvas::GradientStop[offset := 1.0, color := color::rgb(90, 30, 120)]]
  LET gRad AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Radial, startPoint := vector::Float2[x := 0.0, y := 0.0], endPoint := vector::Float2[x := 45.0, y := 0.0], stops := orbStops, spread := canvas::GradientSpread.Pad, transform := canvas::Transform[a := 0.0, b := 0.0, c := 0.0, d := 0.0, tx := 0.0, ty := 0.0], focalOffset := vector::Float2[x := 0.0, y := 0.0], focalRadius := 0.0, strokeApply := canvas::GradientApply.FillOnly]
  LET gradOrb AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 820.0, y := 300.0], radius := 42.0, paint := WITH canvas::fill(color::rgb(0, 0, 0)) { gradient := gRad }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET gPoly AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, startPoint := vector::Float2[x := 40.0, y := 120.0], endPoint := vector::Float2[x := 40.0, y := 210.0], stops := rampStops, spread := canvas::GradientSpread.Pad, transform := canvas::Transform[a := 0.0, b := 0.0, c := 0.0, d := 0.0, tx := 0.0, ty := 0.0], focalOffset := vector::Float2[x := 0.0, y := 0.0], focalRadius := 0.0, strokeApply := canvas::GradientApply.FillOnly]
  LET gradTri AS canvas::DrawItem = canvas::Polygon[id := "", position := vector::Float2[x := 0.0, y := 0.0], contours := [[vector::Float2[x := 20.0, y := 120.0], vector::Float2[x := 120.0, y := 120.0], vector::Float2[x := 70.0, y := 210.0]]], rule := canvas::FillRule.EvenOdd, paint := WITH canvas::fill(color::rgb(0, 0, 0)) { gradient := gPoly }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  ' bug-484: pictures. Their texels ride the glyph region one packed RGBA word each, and
  ' each block names its own slice -- so there are several, placed AFTER the text so their
  ' slices start past the glyphs' (a base never written, or a cursor the two kinds do not
  ' share, reads glyph coverage as colour). Nearest scaling of a 2x2, a translucent texel,
  ' a tint, a transform, a clip and a fractional destination; all in the free corner
  ' right of the cap arc and below the ellipse.
  LET quadPx AS List OF Byte = [toByte(255), toByte(0), toByte(0), toByte(255), toByte(0), toByte(255), toByte(0), toByte(255), toByte(0), toByte(0), toByte(255), toByte(255), toByte(255), toByte(255), toByte(255), toByte(255)]
  LET stripPx AS List OF Byte = [toByte(255), toByte(255), toByte(0), toByte(255), toByte(0), toByte(0), toByte(255), toByte(128), toByte(0), toByte(255), toByte(255), toByte(255)]
  RES quadImg AS canvas::Image = canvas::createImage(2, 2, quadPx)
  RES stripImg AS canvas::Image = canvas::createImage(3, 1, stripPx)
  LET picPlain AS canvas::DrawItem = canvas::Picture[id := "", shape := canvas::Rectangle[id := "", position := vector::Float2[x := 705.0, y := 510.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 60.0, height := 60.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]], clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := quadImg]
  LET picStrip AS canvas::DrawItem = canvas::Picture[id := "", shape := canvas::Rectangle[id := "", position := vector::Float2[x := 775.0, y := 510.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 90.0, height := 30.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]], clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := stripImg]
  LET picHalf AS canvas::Transform = canvas::Transform[a := 0.5, b := 0.0, c := 0.0, d := 0.5, tx := 840.0, ty := 548.0]
  LET picScaled AS canvas::DrawItem = canvas::Picture[id := "", shape := canvas::Rectangle[id := "", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 80.0, height := 80.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 128, 0)), transform := picHalf], clip := canvas::PictureClip.ClipAndTransform, fit := canvas::PictureFit.Stretch, image := quadImg]
  LET picEdge AS canvas::DrawItem = canvas::Picture[id := "", shape := canvas::Rectangle[id := "", position := vector::Float2[x := 705.5, y := 578.25], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 50.5, height := 50.3], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH canvas::fill(color::rgb(255, 255, 255)) { clip := [[vector::Float2[x := 0.0 - 0.5, y := 0.0 - 0.25], vector::Float2[x := 35.5, y := 0.0 - 0.25], vector::Float2[x := 35.5, y := 59.75], vector::Float2[x := 0.0 - 0.5, y := 59.75]]] }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]], clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := quadImg]
  ' plan-163-C: the tint's strength is the fill's alpha (half-strength red here), and a
  ' picture fades through `transparency`, the item block's `clipPath.w`.
  LET picTint AS canvas::DrawItem = canvas::Picture[id := "", shape := canvas::Rectangle[id := "", position := vector::Float2[x := 765.0, y := 550.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 30.0, height := 30.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgba(255, 0, 0, 128)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]], clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := quadImg]
  LET picFade AS canvas::DrawItem = canvas::Picture[id := "", shape := canvas::Rectangle[id := "", position := vector::Float2[x := 765.0, y := 590.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 30.0, height := 30.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH canvas::fill(color::rgb(255, 255, 255)) { transparency := 0.6 }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]], clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := quadImg]
  ' bug-484: a text run with a SPACE in it, early in the scene. A space has an empty
  ' bitmap and draws nothing, but `__canvas_blockInstances` counts it as one instance
  ' like every glyph -- so an emitter that skipped publishing its block shifted every
  ' later draw one block along (the next item drawn under its neighbour's pipeline, the
  ' last reading a block never written). "AAAA" above has no space, which is how that
  ' stayed invisible here; Metal's picture scene is where it was found.
  LET spaced AS canvas::DrawItem = canvas::Text[id := "", position := vector::Float2[x := 470.0, y := 636.0], text := "A A", fonts := [face], fontSize := 20.0, letterSpacing := 0.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET scene AS List OF canvas::DrawItem = [box, rounded, line, faint, spaced, head, eyeL, eyeR, smile, tri, arrow, label, tail, ground, blendMul, blendScr, blendAdd, blendStroke, clippedBox, rotBox, scaleDot, rotText, capButt, capRound, capArc, ell, gradBar, gradOrb, gradTri, picPlain, picStrip, picScaled, picEdge, picTint, picFade]
  canvas::present(scene)
  ' plan-98-G: `canvas::didResize` is TRUE exactly once per size change. Reported from
  ' here because this is the only harness with a scripted resize -- the macOS side can
  ' only ever check the "not yet" half (`tests/canvas/rt_canvas_damage.rs`).
  io::print("didResize-before:" & toString(canvas::didResize()))
  MUT edges AS Integer = 0
  MUT polls AS Integer = 0
  WHILE polls < 40
    IF canvas::didResize() THEN
      edges = edges + 1
    END IF
    os::sleep(50)
    polls = polls + 1
  END WHILE
  io::print("didResize-edges:" & toString(edges))
  ' The poll loop above already keeps the program alive for the scripted resize --
  ' without something here the worker returns from main the moment its frame lands and
  ' the finish helper _exits the process, so the resize on the main thread loses the
  ' race every time. Measured before a sleep was added: the dump stayed 900x640 and
  ' only one stats line was written.
END FUNC
MFB

echo "--- building for $TARGET ---"
"$MFB_EXE" build --app --debug --target "$TARGET" "$proj" >/dev/null

host="test@127.0.0.1"
remote="/tmp/mfb-vkcanvas-$$"
remote_ssh "$PORT" "$host" "rm -rf $remote && mkdir -p $remote"
remote_scp "$PORT" "$proj/build/vkcanvas-$LIBC.AppImage" "$host:$remote/app.AppImage" >/dev/null
remote_scp "$PORT" "$proj/fixture.ttf" "$host:$remote/fixture.ttf" >/dev/null

# Provision the driver before anything measures with it.
icd_env=""
if [ -n "$ICD" ]; then
  if [ "$ICD" = auto ]; then
    echo "--- provisioning a software Vulkan driver on box $PORT ---"
    remote_ssh "$PORT" "$host" '
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
remote_ssh "$PORT" "$host" "
  set -e
  cd $remote
  ./app.AppImage --appimage-extract >/dev/null 2>&1
  bin=./squashfs-root/usr/bin/vkcanvas
  $icd_env MFB_HEADLESS=1 MFB_CANVAS_SYNC=1 MFB_CANVAS_STATS=$remote/sw.txt \
    MFB_CANVAS_DUMP=$remote/sw.rgba timeout 180 \$bin >/dev/null 2>&1
  $icd_env MFB_HEADLESS=1 MFB_CANVAS_SYNC=1 MFB_CANVAS_GPU=1 MFB_CANVAS_STATS=$remote/gpu.txt \
    MFB_CANVAS_DUMP=$remote/gpu.rgba timeout 180 \$bin >/dev/null 2>&1
  $icd_env MFB_HEADLESS=1 MFB_CANVAS_SYNC=1 MFB_CANVAS_RESIZE_W=$RESIZE_W MFB_CANVAS_RESIZE_H=$RESIZE_H \
    MFB_CANVAS_DUMP=$remote/sw2.rgba timeout 180 \$bin >$remote/sw2.out 2>&1
  $icd_env MFB_HEADLESS=1 MFB_CANVAS_SYNC=1 MFB_CANVAS_GPU=1 MFB_CANVAS_RESIZE_W=$RESIZE_W \
    MFB_CANVAS_RESIZE_H=$RESIZE_H MFB_CANVAS_STATS=$remote/gpu2.txt \
    MFB_CANVAS_DUMP=$remote/gpu2.rgba timeout 180 \$bin >/dev/null 2>&1
  # plan-98-G Phase 3: a resize to the size the surface already has. It wakes the
  # renderer through the production resize path with nothing about the scene changed,
  # which is the one wake a program cannot produce by presenting: publishScene refuses
  # an unchanged scene before it ever signals a redraw. This box is where that case can
  # be reached at all -- macOS has no scripted-resize affordance. (No backticks in this
  # comment: it is inside a double-quoted ssh command, where the shell would run it.)
  $icd_env MFB_HEADLESS=1 MFB_CANVAS_SYNC=1 MFB_CANVAS_DAMAGE=1 \
    MFB_CANVAS_RESIZE_W=900 MFB_CANVAS_RESIZE_H=640 MFB_CANVAS_STATS=$remote/dmg.txt \
    MFB_CANVAS_DUMP=$remote/dmg.rgba timeout 180 \$bin >/dev/null 2>&1
"
remote_scp "$PORT" "$host:$remote/sw.rgba" "$work/sw.rgba" >/dev/null
remote_scp "$PORT" "$host:$remote/gpu.rgba" "$work/gpu.rgba" >/dev/null
remote_scp "$PORT" "$host:$remote/gpu.txt" "$work/gpu.txt" >/dev/null
remote_scp "$PORT" "$host:$remote/sw2.rgba" "$work/sw2.rgba" >/dev/null
remote_scp "$PORT" "$host:$remote/sw2.out" "$work/sw2.out" >/dev/null
remote_scp "$PORT" "$host:$remote/gpu2.rgba" "$work/gpu2.rgba" >/dev/null
remote_scp "$PORT" "$host:$remote/gpu2.txt" "$work/gpu2.txt" >/dev/null
remote_scp "$PORT" "$host:$remote/dmg.rgba" "$work/dmg.rgba" >/dev/null
remote_scp "$PORT" "$host:$remote/dmg.txt" "$work/dmg.txt" >/dev/null
remote_ssh "$PORT" "$host" "rm -rf $remote"

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
# Selected is not taken. `__canvas_vulkanRenderable` declines a scene it cannot draw and
# the frame falls back to software, and then every comparison below is the oracle against
# itself and passes. Until bug-484 this scene was never checked for it; the group scene
# further down always was.
case "$stats" in
  *gpuFrames=0*) fail "the main scene produced no GPU frame — the predicate declined it, so every pixel comparison below would be software against itself" ;;
  *) pass "the main scene reached the GPU (gpuFrames non-zero)" ;;
esac

compare() {
  # Tolerance::GPU_DEFAULT, shared with test-winapp.sh (scripts/rgba_compare.py).
  python3 "$ROOT/scripts/rgba_compare.py" "$1" "$2" "$3"
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

# plan-98-G: `canvas::didResize` reports a size change exactly once.
#
# This box is the only place the TRUE half can be checked at all — `MFB_CANVAS_RESIZE_W`
# drives the production resize path and macOS has no equivalent affordance, so
# `tests/canvas/rt_canvas_damage.rs` can only assert the "not yet" half. Both halves matter:
# a `didResize` stuck FALSE never tells a program to lay out again, and one stuck TRUE
# makes it lay out every frame while looking correct.
if [ ! -s "$work/sw2.out" ]; then
  fail "the resized run captured no output"
else
  before="$(grep -o 'didResize-before:[A-Z]*' "$work/sw2.out" | head -1 | cut -d: -f2)"
  edges="$(grep -o 'didResize-edges:[0-9]*' "$work/sw2.out" | head -1 | cut -d: -f2)"
  if [ "$before" = "FALSE" ]; then
    pass "didResize is FALSE before anything resizes"
  else
    fail "didResize answered '$before' before any resize — a program would lay out twice at startup"
  fi
  if [ "${edges:-0}" = "1" ]; then
    pass "didResize reported the resize exactly once (edges=$edges)"
  else
    fail "didResize reported $edges edges for one resize — it must be TRUE once and then FALSE"
  fi
fi

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


# ---------------------------------------------------------------------------------
# plan-116-H: groups on the GPU, against the checked-in reference.
#
# A second program rather than more items in the first, because it asserts a different
# thing. The first scene proves the primitives agree; this one proves the per-draw
# OFFSET agrees. Every item here lives inside a group, so a backend that ignored the
# offset would draw a picture that is complete, plausible and wrong — every shape
# stacked at the origin. That is the failure `.ai/canvas-threading.md` §10 is about, and
# only a comparison at a NON-ZERO offset can see it.
#
# **The scene is extracted from `tests/canvas/rt_canvas_golden.rs`, not copied.** It is the same
# `GROUPS` const that `groups_match_their_reference_exactly` renders, so this script and
# that test cannot drift into rendering different scenes and both claiming to check
# `groups.png`. A copy here would be a second source of truth for a reference image that
# has exactly one.
#
# What the scene covers, and why each case can only pass for the right reason:
#
#   the panel at (0,0) and again at (340,210)   the offset itself, side by side
#   `outer`                                     a NESTED group: (600,40)+(20,30)
#   `leaf` twice                                a DIAMOND: one group, two references
#   the panel's gradient bar                    sampled per fragment from an absolute
#                                               point, so a vertex-only offset shifts
#                                               the ramp inside a correctly placed quad
#   the panel's `Text` run                      glyph coverage, sampled the same way
#   the panel's clipped band                    the one thing that must NOT move: a clip
#                                               is a surface rectangle, so the band is
#                                               visible in the panel at the origin and
#                                               clipped away in both translated copies
echo "--- groups: building for $TARGET ---"
projg="$work/groups"
mkdir -p "$projg/src"
cp "$proj/fixture.ttf" "$projg/fixture.ttf"
sed 's/"name": "vkcanvas"/"name": "vkgroups"/' "$proj/project.json" > "$projg/project.json"
# The Rust const's body, between the raw-string delimiters.
sed -n '/^const GROUPS: &str = r#"/,/^"#;$/p' tests/canvas/rt_canvas_golden.rs \
  | sed -e '1s/^const GROUPS: &str = r#"//' -e '$d' > "$projg/src/main.mfb"
if ! grep -q "canvas::setGroup" "$projg/src/main.mfb"; then
  fail "could not extract the GROUPS scene from tests/canvas/rt_canvas_golden.rs — the const's shape changed"
  exit 1
fi

"$MFB_EXE" build --app --debug --target "$TARGET" "$projg" >/dev/null

# The reference, decoded to raw RGBA here so the box needs no PNG library. Same bytes
# `Frame::load_png` would hand the Rust comparators.
#   png_to_rgba <png> <out.rgba>
png_to_rgba() {
python3 - "$1" "$2" <<'PY'
import struct, sys, zlib

raw = open(sys.argv[1], "rb").read()
assert raw[:8] == b"\x89PNG\r\n\x1a\n", "not a PNG"
pos, idat = 8, bytearray()
width = height = None
while pos < len(raw):
    length, kind = struct.unpack(">I4s", raw[pos:pos + 8])
    body = raw[pos + 8:pos + 8 + length]
    if kind == b"IHDR":
        width, height, depth, colour, _, _, interlace = struct.unpack(">IIBBBBB", body)
        assert (depth, colour, interlace) == (8, 6, 0), "expected 8-bit RGBA, not interlaced"
    elif kind == b"IDAT":
        idat += body
    elif kind == b"IEND":
        break
    pos += 12 + length

data = zlib.decompress(bytes(idat))
stride = width * 4
out = bytearray()
prev = bytearray(stride)
at = 0
for _ in range(height):
    filt = data[at]; at += 1
    line = bytearray(data[at:at + stride]); at += stride
    for i in range(stride):
        a = line[i - 4] if i >= 4 else 0
        b = prev[i]
        c = prev[i - 4] if i >= 4 else 0
        if filt == 1:   line[i] = (line[i] + a) & 0xFF
        elif filt == 2: line[i] = (line[i] + b) & 0xFF
        elif filt == 3: line[i] = (line[i] + (a + b) // 2) & 0xFF
        elif filt == 4:
            pa, pb, pc = abs(b - c), abs(a - c), abs(a + b - 2 * c)
            pred = a if (pa <= pb and pa <= pc) else (b if pb <= pc else c)
            line[i] = (line[i] + pred) & 0xFF
        elif filt != 0:
            raise SystemExit(f"unknown PNG filter {filt}")
    out += line
    prev = line
open(sys.argv[2], "wb").write(bytes(out))
PY
}
png_to_rgba "tests/golden/canvas/groups.png" "$work/groups.rgba"

remoteg="$remote/groups"
remote_ssh "$PORT" "$host" "mkdir -p $remoteg"
remote_scp "$PORT" "$projg/build/vkgroups-$LIBC.AppImage" "$host:$remoteg/app.AppImage" >/dev/null
remote_scp "$PORT" "$projg/fixture.ttf" "$host:$remoteg/fixture.ttf" >/dev/null

echo "--- groups: running on box $PORT ---"
remote_ssh "$PORT" "$host" "
  set -e
  cd $remoteg
  ./app.AppImage --appimage-extract >/dev/null 2>&1
  # loadFont resolves against the working directory, so the fixture has to sit beside
  # the extracted tree and the run has to happen from there.
  cp fixture.ttf squashfs-root/fixture.ttf
  cd squashfs-root
  bin=./usr/bin/vkgroups
  $icd_env MFB_HEADLESS=1 MFB_CANVAS_SYNC=1 MFB_CANVAS_STATS=$remoteg/sw.txt \
    MFB_CANVAS_DUMP=$remoteg/sw.rgba timeout 180 \$bin >/dev/null 2>&1
  $icd_env MFB_HEADLESS=1 MFB_CANVAS_SYNC=1 MFB_CANVAS_GPU=1 MFB_CANVAS_STATS=$remoteg/gpu.txt \
    MFB_CANVAS_DUMP=$remoteg/gpu.rgba timeout 180 \$bin >/dev/null 2>&1
"
remote_scp "$PORT" "$host:$remoteg/sw.rgba" "$work/gsw.rgba" >/dev/null
remote_scp "$PORT" "$host:$remoteg/gpu.rgba" "$work/ggpu.rgba" >/dev/null
if [ -n "$KEEP" ]; then
  mkdir -p "$KEEP"
  cp "$work/gsw.rgba" "$KEEP/groups-sw.rgba"
  cp "$work/ggpu.rgba" "$KEEP/groups-gpu.rgba"
fi
remote_scp "$PORT" "$host:$remoteg/gpu.txt" "$work/ggpu.txt" >/dev/null

gstats="$(tail -1 "$work/ggpu.txt")"
echo "    $gstats"

# A group scene that DECLINED reads as a pass on the pixels — both sides would be the
# software renderer agreeing with itself. plan-116-G's decline did exactly that, so the
# frame count is asserted before any pixel is believed.
case "$gstats" in
  *gpuFrames=0*) fail "the group scene produced no GPU frame — the predicate declined it, so every pixel comparison below would be software against itself" ;;
  *) pass "the group scene reached the GPU (gpuFrames non-zero)" ;;
esac

# The software render against the reference, EXACTLY. This is the assertion that makes
# the GPU one below mean something on this box: it establishes that Linux's oracle is
# the same oracle the reference was made from on macOS.
verdict="$(compare "$work/groups.rgba" "$work/gsw.rgba" 900)"
case "$verdict" in
  ok*)
    if [ "$verdict" = "ok worst=0 differing=0.0000%" ]; then
      pass "groups: the software render reproduces tests/golden/canvas/groups.png exactly"
    else
      fail "groups: the software render is only NEAR the reference ($verdict) — the oracle must match it exactly, or the reference has drifted from the renderer that made it"
    fi
    ;;
  *) fail "groups: the software render disagrees with tests/golden/canvas/groups.png: $verdict" ;;
esac

verdict="$(compare "$work/groups.rgba" "$work/ggpu.rgba" 900)"
case "$verdict" in
  ok*) pass "groups: the Vulkan render matches tests/golden/canvas/groups.png ($verdict)" ;;
  *)   fail "groups: the Vulkan render disagrees with the reference: $verdict
    Localize by what moved and what did not. Everything at the ORIGIN means the
    per-draw offset never reached the backend. A shape in the right place whose
    gradient ramp or glyph ink is shifted means the offset reached the VERTEX stage
    only — distance, ramp and coverage are evaluated per fragment at an absolute
    point and need it too. A white band visible in a TRANSLATED panel means the
    opposite mistake: the clip is a surface rectangle and must not move with its
    group." ;;
esac

# ---------------------------------------------------------------------------------
# plan-154-C: `canvas::PolyLine` on the GPU, against the checked-in reference.
#
# Extracted from `tests/canvas/rt_canvas_golden.rs` for the same reason the group scene is:
# the `POLYLINES` const that `polylines_match_their_reference_exactly` renders is the one
# source of `polyline.png`. It carries round and butt zig-zags (the joins, and the butt
# planes on the first and last live segments), a switch-back, a butt path with repeated
# end points, per-point colours fading to transparent (the gradient lookup at the path
# parameter), fewer colours than points, a two-point path, and the degenerate paths.
echo "--- polylines: building for $TARGET ---"
projp="$work/polylines"
mkdir -p "$projp/src"
sed 's/"name": "vkcanvas"/"name": "vkpolylines"/' "$proj/project.json" > "$projp/project.json"
sed -n '/^const POLYLINES: &str = r#"/,/^"#;$/p' tests/canvas/rt_canvas_golden.rs \
  | sed -e '1s/^const POLYLINES: &str = r#"//' -e '$d' > "$projp/src/main.mfb"
if ! grep -q "canvas::PolyLine" "$projp/src/main.mfb"; then
  fail "could not extract the POLYLINES scene from tests/canvas/rt_canvas_golden.rs — the const's shape changed"
  exit 1
fi
"$MFB_EXE" build --app --debug --target "$TARGET" "$projp" >/dev/null
png_to_rgba "tests/golden/canvas/polyline.png" "$work/polyline.rgba"

remotep="$remote/polylines"
remote_ssh "$PORT" "$host" "mkdir -p $remotep"
remote_scp "$PORT" "$projp/build/vkpolylines-$LIBC.AppImage" "$host:$remotep/app.AppImage" >/dev/null

echo "--- polylines: running on box $PORT ---"
remote_ssh "$PORT" "$host" "
  set -e
  cd $remotep
  ./app.AppImage --appimage-extract >/dev/null 2>&1
  cd squashfs-root
  bin=./usr/bin/vkpolylines
  $icd_env MFB_HEADLESS=1 MFB_CANVAS_SYNC=1 MFB_CANVAS_STATS=$remotep/sw.txt \
    MFB_CANVAS_DUMP=$remotep/sw.rgba timeout 180 \$bin >/dev/null 2>&1
  $icd_env MFB_HEADLESS=1 MFB_CANVAS_SYNC=1 MFB_CANVAS_GPU=1 MFB_CANVAS_STATS=$remotep/gpu.txt \
    MFB_CANVAS_DUMP=$remotep/gpu.rgba timeout 180 \$bin >/dev/null 2>&1
"
remote_scp "$PORT" "$host:$remotep/sw.rgba" "$work/psw.rgba" >/dev/null
remote_scp "$PORT" "$host:$remotep/gpu.rgba" "$work/pgpu.rgba" >/dev/null
remote_scp "$PORT" "$host:$remotep/gpu.txt" "$work/pgpu.txt" >/dev/null

pstats="$(tail -1 "$work/pgpu.txt")"
echo "    $pstats"

# A declined PolyLine frame is software against itself, so the frame count comes first.
case "$pstats" in
  *gpuFrames=0*) fail "the polyline scene produced no GPU frame — __canvas_vulkanRenderable declined it" ;;
  *) pass "the polyline scene reached the GPU (gpuFrames non-zero)" ;;
esac

verdict="$(compare "$work/polyline.rgba" "$work/psw.rgba" 900)"
case "$verdict" in
  ok*)
    if [ "$verdict" = "ok worst=0 differing=0.0000%" ]; then
      pass "polylines: the software render reproduces tests/golden/canvas/polyline.png exactly"
    else
      fail "polylines: the software render is only NEAR the reference ($verdict) — the oracle must match it exactly"
    fi
    ;;
  *) fail "polylines: the software render disagrees with tests/golden/canvas/polyline.png: $verdict" ;;
esac

verdict="$(compare "$work/polyline.rgba" "$work/pgpu.rgba" 900)"
case "$verdict" in
  ok*) pass "polylines: the Vulkan render matches tests/golden/canvas/polyline.png ($verdict)" ;;
  *)   fail "polylines: the Vulkan render disagrees with the reference: $verdict
    A butt end that domes is the first/last segment words (gradient.x/.y, whole
    numbers); a colour step at a corner is the path parameter; a missing path is the
    edge upload or the band reach (the radius slot)." ;;
esac

# ---------------------------------------------------------------------------------
# plan-150-C: `canvas::ParticleSystem` on the GPU.
#
# The design's claim is that a particle is an ordinary item by the time a backend sees
# it, so Vulkan draws a particle scene with the shaders it already had -- neither `.spv`
# is touched by plan-150. plan-150-A proved that on Metal; this is the same proof on a
# Vulkan driver. Two scenes, both read from `tests/canvas/scenes/` (the files the Rust
# suites include):
#
#   particles          a burst of scaled, tinted discs and a stream of turned, stroked
#                      rectangles in the scene itself -- `tests/golden/canvas/particles.png`
#   particles_grouped  a system that is a GROUP's child, the group drawn through a
#                      turned, scaled node and through a moved one: its particles are
#                      blocks of the group's shared run, placed per use by the matrix in
#                      the draw entry, the newest thing either backend reads
#
# The first is also held to its checked-in reference EXACTLY in software. That is the
# feature's other claim, measured on a second platform: every particle is a closed form
# of `(seed, time)` computed with the package's own sine, cosine and exponential, so the
# picture made on macOS is the picture made here, to the bit.
particle_scene() { # particle_scene <tag> <scene file> <reference png or ''>
  local tag="$1" scene="$2" reference="$3"
  local projs="$work/$tag" remotes="$remote/$tag"
  echo "--- $tag: building for $TARGET ---"
  mkdir -p "$projs/src"
  sed "s/\"name\": \"vkcanvas\"/\"name\": \"vk$tag\"/" "$proj/project.json" > "$projs/project.json"
  cp "tests/canvas/scenes/$scene.mfb" "$projs/src/main.mfb"
  if ! grep -q "canvas::ParticleSystem" "$projs/src/main.mfb"; then
    fail "$tag: tests/canvas/scenes/$scene.mfb holds no canvas::ParticleSystem"
    return
  fi
  "$MFB_EXE" build --app --debug --target "$TARGET" "$projs" >/dev/null

  remote_ssh "$PORT" "$host" "mkdir -p $remotes"
  remote_scp "$PORT" "$projs/build/vk$tag-$LIBC.AppImage" "$host:$remotes/app.AppImage" >/dev/null
  echo "--- $tag: running on box $PORT ---"
  remote_ssh "$PORT" "$host" "
    set -e
    cd $remotes
    ./app.AppImage --appimage-extract >/dev/null 2>&1
    cd squashfs-root
    bin=./usr/bin/vk$tag
    $icd_env MFB_HEADLESS=1 MFB_CANVAS_SYNC=1 MFB_CANVAS_STATS=$remotes/sw.txt \
      MFB_CANVAS_DUMP=$remotes/sw.rgba timeout 180 \$bin >/dev/null 2>&1
    $icd_env MFB_HEADLESS=1 MFB_CANVAS_SYNC=1 MFB_CANVAS_GPU=1 MFB_CANVAS_STATS=$remotes/gpu.txt \
      MFB_CANVAS_DUMP=$remotes/gpu.rgba timeout 180 \$bin >/dev/null 2>&1
  "
  remote_scp "$PORT" "$host:$remotes/sw.rgba" "$work/$tag-sw.rgba" >/dev/null
  remote_scp "$PORT" "$host:$remotes/gpu.rgba" "$work/$tag-gpu.rgba" >/dev/null
  remote_scp "$PORT" "$host:$remotes/gpu.txt" "$work/$tag-gpu.txt" >/dev/null

  local stats verdict
  stats="$(tail -1 "$work/$tag-gpu.txt")"
  echo "    $stats"
  # A declined frame is the software renderer against itself: the frame count first.
  case "$stats" in
    *gpuFrames=0*) fail "$tag: the scene produced no GPU frame -- __canvas_vulkanRenderable declined it" ;;
    *) pass "$tag: the scene reached the GPU (gpuFrames non-zero)" ;;
  esac

  if [ -n "$reference" ]; then
    png_to_rgba "$reference" "$work/$tag-ref.rgba"
    verdict="$(compare "$work/$tag-ref.rgba" "$work/$tag-sw.rgba" 900)"
    if [ "$verdict" = "ok worst=0 differing=0.0000%" ]; then
      pass "$tag: the software render reproduces $reference exactly"
    else
      fail "$tag: the software render is not $reference to the bit ($verdict) -- a particle's place is a closed form of (seed, time) and must not depend on the platform; look for a libm call in the model"
    fi
  fi

  verdict="$(compare "$work/$tag-sw.rgba" "$work/$tag-gpu.rgba" 900)"
  case "$verdict" in
    ok*) pass "$tag: the Vulkan render matches the software oracle ($verdict)" ;;
    *)   fail "$tag: the Vulkan render disagrees with the software oracle: $verdict
    Do NOT regenerate the SPIR-V: the blobs being untouched is the evidence that the
    design held. A particle is a shape with a position, a transform and a paint, so a
    wrong picture here is the composition of those in
    src/codegen/builtins/canvas/helper_particles.rs (__canvas_particleCompose,
    __canvas_particleItem) and will be wrong on Metal too under some scene." ;;
  esac
}

particle_scene particles particles_on_black "tests/golden/canvas/particles.png"
particle_scene particlesgroup particles_grouped_on_black ""

if [ "$rc_failures" -eq 0 ]; then
  echo "canvas Vulkan runtime tests passed"
else
  echo "$rc_failures failure(s)"
  exit 1
fi
