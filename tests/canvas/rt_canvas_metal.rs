//! The Metal backend renders the same picture as the software oracle (plan-98-E).
//!
//! The oracle is the software rasteriser, not a stored image: plan-98-A invariant 7
//! makes the software path the reference every GPU backend is measured against, so
//! these tests render the *same program twice* — once with `MFB_CANVAS_GPU=1` and
//! once without — and diff the two frames. That is stronger than diffing the GPU
//! frame against a checked-in PNG, because it cannot drift out of date: if the
//! rasteriser changes, both sides change together and the comparison still means
//! "the two backends agree".
//!
//! The gate is `Tolerance::GPU_DEFAULT` (plan-98-A invariant 5) — GPU output is not
//! required to be exact-match. It currently *is* exact for these scenes, which is a
//! measurement rather than a promise: the pipeline writes a `BGRA8Unorm_sRGB` target
//! and the shader emits linear premultiplied colour, so the same arithmetic the
//! software path does by hand happens in the raster hardware. Tightening the
//! assertion to exact equality would make any future antialiasing change in either
//! backend read as a failure when it is not one.
//!
//! **These tests are macOS-only and they skip rather than fail elsewhere.** They also
//! skip on a macOS host with no Metal device — a real case (some VMs, some CI
//! lanes) — which is why the skip is decided from the program's own
//! `MFB_CANVAS_STATS` line rather than from the platform: a host that reports a
//! device must render, and one that does not must not silently pass.

#[path = "../common/mod.rs"]
mod common;

#[path = "rt_canvas_metal/scenes_and_passes.rs"]
mod scenes_and_passes;
#[path = "rt_canvas_metal/scenes_over_black.rs"]
mod scenes_over_black;

use common::canvas_image::{compare_exact, compare_within_tolerance, Frame, Tolerance};
use std::path::PathBuf;
use std::process::Command;

const WIDTH: u32 = 900;
const HEIGHT: u32 = 640;

/// Axis-aligned rectangles, opaque and translucent, overlapping each other and the
/// background.
///
/// Every one of them is a shape Phase 1's flat-fill fragment shader reproduces, which
/// is what makes this the scene that measures the *colour* chain rather than the
/// geometry: `rgba(0,0,255,128)` over mid grey and `rgba(255,255,255,40)` over black
/// both land far from either endpoint, so an sRGB-versus-linear mistake anywhere in
/// the chain moves them.
const RECTANGLES: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  LET red AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 10.0, y := 10.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 50.0, height := 50.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET grey AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 100.0, y := 10.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 60.0, height := 60.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(128, 128, 128)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET half AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 120.0, y := 30.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 60.0, height := 60.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgba(0, 0, 255, 128)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET faint AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 200.0, y := 10.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 80.0, height := 40.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgba(255, 255, 255, 40)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET wide AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 300.0, y := 100.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 500.0, height := 300.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(17, 200, 90)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET over AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 400.0, y := 150.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 200.0, height := 100.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgba(255, 200, 0, 200)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  canvas::present([red, grey, half, faint, wide, over])
END FUNC
"#;

/// Every primitive the SDF shader draws, in one scene.
///
/// The smiley from `plan-98-api.md` — the scene that shaped the API — plus a rounded
/// rect with both a fill and a stroke, a thick line, and **two** translucent polygons.
/// That covers each arm of the distance dispatch, both paint channels, the
/// corner-radius term, an arc's sweep test, and the polygon edge buffer, which between
/// them are every path the fragment shader has.
///
/// **Two polygons, not one, and that is load-bearing since plan-116-A.** Metal's edges
/// used to be copied into the command buffer per item, so every polygon's array started
/// at index 0 and the edge base was always zero. They now take a slice of one region
/// that serves the whole frame, so the base is a real per-item value — and with a single
/// polygon in the scene the base is *still* zero, so a base that was never written would
/// pass. The second polygon is the only thing that reads a non-zero one. (The Vulkan
/// harness has carried two for exactly this reason since plan-98-F; Metal needs it now
/// for the first time.)
const PRIMITIVES: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)

  LET yellow AS color::Color = color::rgb(255, 255, 0)
  LET green AS color::Color = color::rgb(0, 160, 0)

  LET face AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 450.0, y := 320.0], radius := 150.0, paint := canvas::fill(yellow), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET eyeL AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 400.0, y := 280.0], radius := 22.0, paint := canvas::fill(green), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET eyeR AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 500.0, y := 280.0], radius := 22.0, paint := canvas::fill(green), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET smile AS canvas::DrawItem = canvas::Arc[id := "", position := vector::Float2[x := 450.0, y := 335.0], radius := 90.0, startAngle := 0.0, endAngle := 3.14159, paint := WITH canvas::stroke(green, 14.0) { cap := canvas::CapStyle.Butt }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET box AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 10.0, y := 10.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 50.0, height := 50.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET rounded AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 100.0, y := 10.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 90.0, height := 60.0], radiusTL := 18.0, radiusTR := 18.0, radiusBL := 18.0, radiusBR := 18.0, paint := canvas::fillStroke(color::rgb(0, 0, 255), color::rgb(255, 255, 255), 4.0), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET line AS canvas::DrawItem = canvas::Line[id := "", position := vector::Float2[x := 0.0, y := 0.0], startAt := vector::Float2[x := 220.0, y := 20.0], endAt := vector::Float2[x := 380.0, y := 90.0], paint := WITH canvas::stroke(color::rgb(255, 128, 0), 9.0) { cap := canvas::CapStyle.Round }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  ' plan-154-B: a PolyLine, with a colour per point.
  LET path AS canvas::DrawItem = canvas::PolyLine[id := "", position := vector::Float2[x := 0.0, y := 0.0], points := [vector::Float2[x := 720.0, y := 180.0], vector::Float2[x := 800.0, y := 240.0], vector::Float2[x := 740.0, y := 300.0], vector::Float2[x := 850.0, y := 330.0]], colors := [color::rgb(255, 60, 60), color::rgb(60, 255, 60), color::rgba(60, 60, 255, 128)], paint := WITH canvas::stroke(color::rgb(255, 255, 255), 10.0) { cap := canvas::CapStyle.Round }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET tri AS canvas::DrawItem = canvas::Polygon[id := "", position := vector::Float2[x := 0.0, y := 0.0], contours := [[vector::Float2[x := 600.0, y := 40.0], vector::Float2[x := 700.0, y := 40.0], vector::Float2[x := 650.0, y := 130.0]]], rule := canvas::FillRule.EvenOdd, paint := canvas::fill(color::rgba(0, 200, 255, 180)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  ' The second polygon, and concave on purpose: it is the one item in this scene drawn
  ' from a non-zero edge base, and the crossing-count sign test only disagrees with the
  ' nearest-edge magnitude on a shape that is not convex -- so a wrong base here shows as
  ' a wrong FILL, not merely a shifted outline.
  LET arrow AS canvas::DrawItem = canvas::Polygon[id := "", position := vector::Float2[x := 0.0, y := 0.0], contours := [[vector::Float2[x := 60.0, y := 400.0], vector::Float2[x := 160.0, y := 400.0], vector::Float2[x := 160.0, y := 360.0], vector::Float2[x := 230.0, y := 430.0], vector::Float2[x := 160.0, y := 500.0], vector::Float2[x := 160.0, y := 460.0], vector::Float2[x := 60.0, y := 460.0]]], rule := canvas::FillRule.EvenOdd, paint := canvas::fill(color::rgba(0, 180, 180, 200)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]

  ' plan-116-B: one item per non-Normal BlendMode, and one clipped item.
  '
  ' Without these the frame binds only Normal's pipeline and never takes the clip
  ' path, so three of the four pipelines this letter builds would go unexercised on
  ' Metal while the suite still reported success.
  '
  ' On a mid-grey patch on purpose: over black, Multiply is a no-op and Screen and Add
  ' are indistinguishable from Normal, so a wrong pipeline would look right.
  '
  ' `blendStroke` is the sharp one -- non-Normal AND both filled and stroked, which the
  ' shader cannot compose in one pass (the stroke-over-fill identity is Normal-only),
  ' so it must be emitted as two adjacent instances.
  '
  ' Small on purpose: a blended pixel agrees with the oracle to within a step or two
  ' but rarely exactly (the oracle blends through a 16-bit linear table, the hardware
  ' in float), and Tolerance::GPU_DEFAULT's population budget is a fraction of the
  ' WHOLE frame -- a large blended patch would exhaust it without testing anything more.
  LET ground AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 20.0, y := 400.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 360.0, height := 120.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(128, 128, 128)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET blendMul AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 70.0, y := 460.0], radius := 14.0, paint := WITH canvas::fill(color::rgb(230, 120, 40)) { blend := canvas::BlendMode.Multiply }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET blendScr AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 170.0, y := 460.0], radius := 14.0, paint := WITH canvas::fill(color::rgb(230, 120, 40)) { blend := canvas::BlendMode.Screen }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET blendAdd AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 270.0, y := 460.0], radius := 14.0, paint := WITH canvas::fill(color::rgb(230, 120, 40)) { blend := canvas::BlendMode.Add }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET blendStroke AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 350.0, y := 460.0], radius := 12.0, paint := WITH canvas::fillStroke(color::rgb(230, 120, 40), color::rgb(40, 120, 230), 8.0) { blend := canvas::BlendMode.Multiply }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET clippedBox AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 420.0, y := 400.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 300.0, height := 60.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH canvas::fill(color::rgb(255, 255, 255)) { clip := [[vector::Float2[x := 40.25, y := 0.0], vector::Float2[x := 240.75, y := 0.0], vector::Float2[x := 240.75, y := 60.0], vector::Float2[x := 40.25, y := 60.0]]] }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]

  ' plan-116-C: transformed items, so the shader's inverse-map path actually runs.
  ' A rotation exercises the gradient correction on a curved edge; the non-uniform
  ' scale is the case Phase 1 measured sqrt(|det M|) as 37/255 wrong on. Rotated TEXT
  ' is covered by rt_canvas_font's a_rotated_text_run_draws_rotated, which owns the
  ' font fixture -- this scene has no font.
  LET rotT AS canvas::Transform = canvas::Transform[a := 0.7071067811865476, b := 0.7071067811865476, c := 0.0 - 0.7071067811865476, d := 0.7071067811865476, tx := 145.0, ty := 585.0 - 50.0 * 0.7071067811865476]
  LET rotBox AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 0.0 - 25.0, y := 0.0 - 25.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 50.0, height := 50.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 200, 40)), transform := rotT]
  LET scaleT AS canvas::Transform = canvas::Transform[a := 2.0, b := 0.0, c := 0.0, d := 1.0, tx := 250.0, ty := 560.0]
  LET scaleDot AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 0.0, y := 0.0], radius := 18.0, paint := canvas::fillStroke(color::rgb(90, 200, 255), color::rgb(255, 255, 255), 6.0), transform := scaleT]

  ' plan-116-D: the same line twice, butt and round, so the shader's cap arm actually
  ' runs on the GPU. `line` above is round-capped and pre-dates this letter; without a
  ' butt one the new branch would be emitted and never executed, and a backend whose
  ' butt arm is wrong would still match the oracle everywhere the scene looks.
  ' Thick and short, because the cap is a half-width feature: at 24 px wide the two
  ' styles differ over a visibly large region rather than one antialiased pixel.
  LET capButt AS canvas::DrawItem = canvas::Line[id := "", position := vector::Float2[x := 0.0, y := 0.0], startAt := vector::Float2[x := 120.0, y := 600.0], endAt := vector::Float2[x := 240.0, y := 600.0], paint := WITH canvas::stroke(color::rgb(255, 240, 120), 24.0) { cap := canvas::CapStyle.Butt }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET capRound AS canvas::DrawItem = canvas::Line[id := "", position := vector::Float2[x := 0.0, y := 0.0], startAt := vector::Float2[x := 320.0, y := 600.0], endAt := vector::Float2[x := 440.0, y := 600.0], paint := WITH canvas::stroke(color::rgb(255, 240, 120), 24.0) { cap := canvas::CapStyle.Round }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  ' And a ROUND-capped arc, for the same reason. `smile` above is butt-capped -- which
  ' is what every arc was before plan-116-D -- so without this the arc's cap-disc arm
  ' is compiled into both shaders and never taken. The sweep stops at 0.6*PI so both
  ' ends are visible rather than one running off the item's own band.
  LET capArc AS canvas::DrawItem = canvas::Arc[id := "", position := vector::Float2[x := 620.0, y := 600.0], radius := 60.0, startAngle := 0.0, endAngle := 1.884955592153876, paint := WITH canvas::stroke(color::rgb(120, 255, 200), 20.0) { cap := canvas::CapStyle.Round }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]

  canvas::present([box, rounded, line, path, tri, arrow, face, eyeL, eyeR, smile, ground, blendMul, blendScr, blendAdd, blendStroke, clippedBox, rotBox, scaleDot, capButt, capRound, capArc])
END FUNC
"#;

/// A polygon with more edges than the shader's edge buffer holds.
///
/// `setFragmentBytes:length:atIndex:` is capped at 4 KB and each edge crosses as four
/// 16.16 ints, so 300 edges do not fit. This is the one scene the Metal renderer
/// still declines, and the rectangle beside it is there so the frame is not blank —
/// a fallback that rendered nothing would compare equal to a fallback that rendered
/// nothing, and prove nothing.
const GROUPS: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color
FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)

  ' 1. a group drawn at (0,0) -- the offset is present but zero
  LET atOrigin AS List OF canvas::DrawItem = [canvas::Rectangle[id := "", position := vector::Float2[x := 20.0, y := 20.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 80.0, height := 40.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]]
  canvas::setGroup("atOrigin", atOrigin)

  ' 2. a group drawn at (37, 53) -- a non-zero offset, which a vertex-only
  '    implementation cannot get right for anything position-dependent
  LET moved AS List OF canvas::DrawItem = [canvas::Circle[id := "", position := vector::Float2[x := 200.0, y := 60.0], radius := 30.0, paint := canvas::fillStroke(color::rgb(0, 160, 220), color::rgb(255, 255, 255), 5.0), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]]
  canvas::setGroup("moved", moved)

  ' 3. a NESTED group: outer holds inner, so the offsets compose
  LET inner AS List OF canvas::DrawItem = [canvas::Rectangle[id := "", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 50.0, height := 50.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(200, 200, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]]
  canvas::setGroup("inner", inner)
  LET outer AS List OF canvas::DrawItem = [canvas::Group[id := "", position := vector::Float2[x := 15.0, y := 25.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := "inner"]]
  canvas::setGroup("outer", outer)

  ' 4. a DIAMOND: one leaf referenced twice, at two offsets
  LET leaf AS List OF canvas::DrawItem = [canvas::Rectangle[id := "", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 60.0, height := 40.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(120, 220, 60)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]]
  canvas::setGroup("leaf", leaf)

  ' 5. a CLIPPED item inside a translated group -- the clip is evaluated in SURFACE
  '    space while the shape is evaluated in shape space (section 4.2)
  LET clipped AS List OF canvas::DrawItem = [canvas::Rectangle[id := "", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 300.0, height := 60.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH canvas::fill(color::rgb(255, 255, 255)) { clip := [[vector::Float2[x := 40.25, y := 0.0], vector::Float2[x := 240.75, y := 0.0], vector::Float2[x := 240.75, y := 60.0], vector::Float2[x := 40.25, y := 60.0]]] }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]]
  canvas::setGroup("clipped", clipped)

  ' 6. a GRADIENT-filled item inside a translated group -- the ramp is sampled from
  '    the shape-space point, so a missing fragment offset shows as a shifted ramp
  LET stops AS List OF canvas::GradientStop = [canvas::GradientStop[offset := 0.0, color := color::rgb(255, 64, 32)], canvas::GradientStop[offset := 0.55, color := color::rgb(250, 230, 90)], canvas::GradientStop[offset := 1.0, color := color::rgb(32, 96, 255)]]
  LET ramp AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, startPoint := vector::Float2[x := 0.0, y := 0.0], endPoint := vector::Float2[x := 150.0, y := 60.0], stops := stops, spread := canvas::GradientSpread.Pad, transform := canvas::Transform[a := 0.0, b := 0.0, c := 0.0, d := 0.0, tx := 0.0, ty := 0.0], focalOffset := vector::Float2[x := 0.0, y := 0.0], focalRadius := 0.0, strokeApply := canvas::GradientApply.FillOnly]
  LET grad AS List OF canvas::DrawItem = [canvas::Rectangle[id := "", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 150.0, height := 60.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH canvas::fill(color::rgb(0, 0, 0)) { gradient := ramp }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]]
  canvas::setGroup("grad", grad)

  LET scene AS List OF canvas::DrawItem = [canvas::Group[id := "", position := vector::Float2[x := 0.0, y := 0.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := "atOrigin"], canvas::Group[id := "", position := vector::Float2[x := 37.0, y := 53.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := "moved"], canvas::Group[id := "", position := vector::Float2[x := 380.0, y := 40.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := "outer"], canvas::Group[id := "", position := vector::Float2[x := 600.0, y := 40.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := "leaf"], canvas::Group[id := "", position := vector::Float2[x := 700.0, y := 140.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := "leaf"], canvas::Group[id := "", position := vector::Float2[x := 60.0, y := 200.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := "clipped"], canvas::Group[id := "", position := vector::Float2[x := 420.0, y := 200.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := "grad"]]
  canvas::present(scene)
END FUNC
"#;

/// A diamond: one group installed once and named twice, at two offsets.
///
/// Paired with `DIAMOND_FLAT`, which draws the same four rectangles written out
/// longhand at the same absolute coordinates and uses no group at all.
const DIAMOND_GROUPED: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color
FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  LET a AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 70.0, height := 45.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(120, 220, 60)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET b AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 100.0, y := 22.0], radius := 18.0, paint := canvas::fillStroke(color::rgb(0, 170, 220), color::rgb(255, 255, 255), 4.0), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  canvas::setGroup("pair", [a, b])
  canvas::present([canvas::Group[id := "", position := vector::Float2[x := 120.0, y := 90.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := "pair"], canvas::Group[id := "", position := vector::Float2[x := 430.0, y := 300.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := "pair"]])
END FUNC
"#;

/// The same picture with no group in it — the two shapes written out twice, at the
/// coordinates the group offsets would have put them.
const DIAMOND_FLAT: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color
FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  LET a1 AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 120.0, y := 90.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 70.0, height := 45.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(120, 220, 60)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET b1 AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 220.0, y := 112.0], radius := 18.0, paint := canvas::fillStroke(color::rgb(0, 170, 220), color::rgb(255, 255, 255), 4.0), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET a2 AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 430.0, y := 300.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 70.0, height := 45.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(120, 220, 60)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET b2 AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 530.0, y := 322.0], radius := 18.0, paint := canvas::fillStroke(color::rgb(0, 170, 220), color::rgb(255, 255, 255), 4.0), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  canvas::present([a1, b1, a2, b2])
END FUNC
"#;

/// A scene whose only item names a group that was never installed.
const ABSENT_GROUP: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  canvas::present([canvas::Group[id := "", position := vector::Float2[x := 40.0, y := 60.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := "neverInstalled"]])
END FUNC
"#;

/// The same scene with nothing in it at all — the control for `ABSENT_GROUP`.
const EMPTY_SCENE: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  canvas::present([])
END FUNC
"#;

/// ONE polygon with more edges than Metal's whole frame edge region holds
/// (`METAL_MAX_FRAME_EDGES` = 262,144): 262,145 edges.
///
/// This scene was a 300-edge ring until bug-686, declined by a per-polygon cap of 256
/// that bug deleted — 300 edges now draw on Metal
/// (`polygons_past_two_hundred_fifty_six_edges_draw_on_metal`). No polygon size is
/// refused any more; a polygon that alone passes the frame's region still is, and that
/// is the decline this proves. 6 px across, so the software oracle stays quick.
const TOO_MANY_EDGES: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color
IMPORT collections
IMPORT math

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  MUT points AS List OF vector::Float2 = []
  MUT i AS Integer = 0
  WHILE i < 262145
    LET a AS Float = toFloat(i) * 6.283185307179586 / 262145.0
    points = collections::append(points, vector::Float2[x := 450.0 + 3.0 * math::cos(a), y := 320.0 + 3.0 * math::sin(a)])
    i = i + 1
  END WHILE
  LET ring AS canvas::DrawItem = canvas::Polygon[id := "", position := vector::Float2[x := 0.0, y := 0.0], contours := [points], rule := canvas::FillRule.EvenOdd, paint := canvas::fill(color::rgb(0, 200, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET box AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 10.0, y := 10.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 50.0, height := 50.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(0, 255, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  canvas::present([box, ring])
END FUNC
"#;

/// A built program, kept so both renders run the same binary.
struct Program {
    project: PathBuf,
    binary: PathBuf,
}

impl Drop for Program {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.project);
    }
}

fn build(name: &str, source: &str) -> Program {
    let project = common::temp_project(name, source);
    let binary = common::build_app_debug(&project, name);
    Program { project, binary }
}

/// Render one frame, returning it with the stats line the run reported.
fn render(program: &Program, metal: bool, tag: &str) -> (Frame, String) {
    render_env(program, metal, tag, &[])
}

/// [`render`] with more of the environment set, run from the project's directory so a
/// scene that opens a file beside itself finds it.
fn render_env(program: &Program, metal: bool, tag: &str, envs: &[(&str, &str)]) -> (Frame, String) {
    let frame_path = program.project.join(format!("frame-{tag}.rgba"));
    let stats_path = program.project.join(format!("stats-{tag}.txt"));
    let mut command = Command::new(&program.binary);
    command
        .current_dir(&program.project)
        .env("MFB_HEADLESS", "1")
        .env("MFB_CANVAS_SYNC", "1")
        .env("MFB_CANVAS_STATS", &stats_path)
        .env("MFB_CANVAS_DUMP", &frame_path);
    if metal {
        command.env("MFB_CANVAS_GPU", "1");
    }
    command.envs(envs.iter().copied());
    let run = command
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", program.binary.display()));
    assert!(
        run.status.success(),
        "program {} (metal={metal}):\n{}\n{}",
        common::exit_description(&run.status),
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr),
    );
    let pixels = std::fs::read(&frame_path).expect("canvas dump written");
    let stats = std::fs::read_to_string(&stats_path).expect("canvas stats written");
    (Frame::from_rgba(WIDTH, HEIGHT, pixels), stats)
}

/// `true` when the run actually built a Metal pipeline; `false` only when the host
/// has no Metal device at all.
///
/// The distinction matters more than it looks. A test that returned early whenever
/// `metalReady=FALSE` would pass silently on a machine where the pipeline is *broken*
/// — an MSL syntax error, a missing entry point, a bad blend enum — which is exactly
/// the class of bug these tests exist to catch. So the only sanctioned skip is
/// `metal=FALSE`, meaning `MTLCreateSystemDefaultDevice` returned nil; a host that
/// reports a device and then fails to build a pipeline **fails the test**.
fn metal_built(stats: &str) -> bool {
    let line = stats
        .lines()
        .next_back()
        .expect("the stats file must carry at least one frame's line");
    if !line.contains("metal=TRUE") {
        return false; // no Metal device on this host — the one legitimate skip
    }
    assert!(
        line.contains("metalReady=TRUE"),
        "this host reports a Metal device but the pipeline did not build — that is a \
         broken shader or pipeline descriptor, not a missing GPU: {line}"
    );
    true
}

/// The Metal backend draws the rectangle scene the same as the software oracle.
#[test]
fn rectangles_match_the_software_oracle_within_tolerance() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let program = build("canvas_metal_rects", RECTANGLES);
    let (software, _) = render(&program, false, "sw");
    let (gpu, stats) = render(&program, true, "gpu");
    if !metal_built(&stats) {
        return; // no Metal device on this host (§metal_built)
    }
    assert!(
        !stats.contains("gpuFrames=0"),
        "MFB_CANVAS_GPU=1 did not select the Metal renderer: {stats}"
    );
    if let Err(diff) = compare_within_tolerance(&gpu, &software, Tolerance::GPU_DEFAULT) {
        panic!(
            "the Metal backend disagrees with the software oracle: {diff}\n\
             Root-cause this against the software reference — the premultiplied \
             linear blend, the sRGB target format and the Y-down NDC mapping are the \
             three places a whole-scene shift comes from."
        );
    }
}

/// The full primitive set matches the software oracle within tolerance.
///
/// This is Phase 2's acceptance: the SDF fragment shader evaluates the same distance
/// functions the software rasteriser does, so a circle is round on both, an arc
/// sweeps the same sector, and a rounded rect's corners have the same radius.
///
/// Measured when it landed: **worst channel delta 1**, and no pixel differs by more
/// than two steps — inside `Tolerance::GPU_DEFAULT`'s per-pixel bound rather than
/// merely inside its population budget.
///
/// It did not start there. Blending in float against an oracle that quantizes
/// coverage to a whole 0..255 gave a worst delta of 5 on 572 pixels, because the
/// sRGB encode near black is steep enough that ONE coverage step moves a dark channel
/// by up to 13 output steps (measured over the oracle's own table). The fix was to
/// quantize coverage in the shader the same way the oracle does — **not** to raise
/// the tolerance, which is what "these are placeholders, not guesses to be loosened
/// until something passes" in `Tolerance::GPU_DEFAULT` is there to prevent.
#[test]
fn the_full_primitive_set_matches_the_software_oracle() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let program = build("canvas_metal_primitives", PRIMITIVES);
    let (software, _) = render(&program, false, "sw");
    let (gpu, stats) = render(&program, true, "gpu");
    if !metal_built(&stats) {
        return; // no Metal device on this host (§metal_built)
    }
    if let Err(diff) = compare_within_tolerance(&gpu, &software, Tolerance::GPU_DEFAULT) {
        panic!(
            "the Metal backend disagrees with the software oracle on the primitive \
             set: {diff}\n\
             Localize the primitive at that coordinate. A whole-shape mismatch is a \
             distance function; a rim of edge pixels is coverage; a uniform shift is \
             the sRGB/linear chain."
        );
    }
}

/// One integer field of a stats line, by name.
fn stat(line: &str, name: &str) -> i64 {
    line.split_whitespace()
        .find_map(|word| word.strip_prefix(name)?.strip_prefix('='))
        .unwrap_or_else(|| panic!("no `{name}=` on the stats line: {line}"))
        .parse()
        .unwrap_or_else(|e| panic!("`{name}=` is not an integer ({e}): {line}"))
}

/// A `--debug` build's Metal renderer reports where its own time went.
///
/// `phaseRenderMs` is one number for the whole backend call; the `prof…` fields split
/// it. The software run is the control: nothing but the Metal renderer writes the
/// block, so there every field is 0, and a field that is non-zero on the GPU run was
/// put there by the frame it describes.
///
/// A headless frame is read back, so it has a readback time and no present. The
/// acquire time is not asserted: with no drawable to take it is the few instructions
/// between two clock readings, which is under a microsecond unless the thread is
/// descheduled between them.
#[test]
fn a_debug_metal_frame_reports_its_own_phases() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let program = build("canvas_metal_profile", PRIMITIVES);
    let (_, software) = render(&program, false, "sw");
    let (_, stats) = render(&program, true, "gpu");
    if !metal_built(&stats) {
        return; // no Metal device on this host (§metal_built)
    }
    let fields = [
        "profAcquireUs",
        "profFillUs",
        "profEncodeUs",
        "profWaitUs",
        "profGpuUs",
        "profReadbackUs",
        "profPresentUs",
        "profPasses",
        "profBytes",
        "profTexels",
        "profDraws",
        "profVsyncOff",
    ];
    let software = software.lines().next_back().expect("a software stats line");
    for name in fields {
        assert_eq!(
            stat(software, name),
            0,
            "a software frame wrote `{name}`: {software}"
        );
    }

    let line = stats.lines().next_back().expect("a Metal stats line");
    assert_eq!(stat(line, "gpuFrames"), 1, "one Metal frame: {line}");
    assert_eq!(stat(line, "profPasses"), 1, "one frame, one pass: {line}");
    for name in [
        "profFillUs",
        "profEncodeUs",
        "profWaitUs",
        "profGpuUs",
        "profReadbackUs",
    ] {
        assert!(stat(line, name) > 0, "`{name}` was not timed: {line}");
    }
    assert_eq!(
        stat(line, "profPresentUs"),
        0,
        "a headless frame is read back, not presented: {line}"
    );
    // The GPU cannot have executed for longer than the thread waited on it.
    assert!(
        stat(line, "profGpuUs") <= stat(line, "profWaitUs"),
        "the GPU's own time exceeds the wait around it: {line}"
    );
    // Every item is one block at least, and the scene's two polygons have edges. 304 is
    // `ITEM_BLOCK_SIZE` (crate-private, so spelled here).
    assert!(
        stat(line, "profBytes") >= stat(line, "blocks") * 304,
        "fewer bytes than the frame's blocks alone: {line}"
    );
    assert!(stat(line, "profDraws") >= 1, "no draw call counted: {line}");
    assert_eq!(
        stat(line, "profTexels"),
        0,
        "the scene has no text and no picture: {line}"
    );
    // Display sync is a window layer's, and a headless run configures no layer.
    assert_eq!(
        stat(line, "profVsyncOff"),
        0,
        "a headless run turned a layer's display sync off: {line}"
    );
}

/// A scene the shader cannot draw is declined, not drawn wrongly.
///
/// This is the test that keeps `MFB_CANVAS_GPU=1` honest. A backend that drew a
/// declined scene approximately — a truncated polygon, say — would still report
/// success, and the picture would be wrong in a way no other test looks at.
///
/// The assertion is deliberately **exact**, not within tolerance: the fallback runs
/// the identical software renderer on the identical scene, so anything other than
/// byte equality means the Metal path drew something it should not have.
///
/// The scene is one polygon past the frame's edge region (`TOO_MANY_EDGES`); since
/// bug-686 there is no per-polygon cap for a smaller one to trip.
#[test]
fn an_unsupported_scene_falls_back_to_the_software_renderer() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let program = build("canvas_metal_fallback", TOO_MANY_EDGES);
    let (software, _) = render(&program, false, "sw");
    let (gpu, stats) = render(&program, true, "gpu");
    if !metal_built(&stats) {
        return;
    }
    // bug-686: a polygon this small draws on Metal byte-identically to software (a
    // 262,144-edge twin measured 0 differing bytes), so pixel equality alone would pass
    // whether or not the frame was declined. The decline is asserted directly.
    assert!(
        stats.contains("gpuFrames=0"),
        "a 262,145-edge polygon is past METAL_MAX_FRAME_EDGES, but Metal drew the frame: \
         {stats}"
    );
    if let Err(diff) = compare_exact(&gpu, &software) {
        panic!(
            "a 262,145-edge polygon does not fit Metal's frame edge region, so the \
             renderer must decline the whole scene and let the software oracle draw it \
             — the two frames must be byte-identical, but {diff}"
        );
    }
}

/// plan-172-G: one group node with a quarter turn.
const TURNED_GROUP: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  LET none AS canvas::Transform = canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
  LET bar AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 40.0, y := 0.0 - 15.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 120.0, height := 30.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(40, 200, 120)), transform := none]
  canvas::setGroup("panel", [bar])
  canvas::present([canvas::Group[id := "", position := vector::Float2[x := 400.0, y := 300.0], transform := canvas::Transform[a := 0.0, b := 1.0, c := 0.0 - 1.0, d := 0.0, tx := 0.0, ty := 0.0], name := "panel"]])
END FUNC
"#;

/// plan-172-H: Metal draws a group with a matrix, where the software oracle does.
///
/// plan-172-G had Metal decline this frame, because it bound a group's translation and
/// nothing else and would have drawn the bar at the group's position unturned — a
/// complete, plausible, wrong picture. It now binds the inverse linear part to both
/// stages. The frame count is asserted first: a leftover decline would hand the
/// comparison the software frame, and every pixel would match while proving nothing.
#[test]
fn a_group_with_a_matrix_draws_on_metal_and_matches_the_software_oracle() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let program = build("canvas_metal_group_matrix", TURNED_GROUP);
    let (software, _) = render(&program, false, "sw");
    let (gpu, stats) = render(&program, true, "gpu");
    if !metal_built(&stats) {
        return;
    }
    assert!(
        !stats.contains("gpuFrames=0"),
        "a group with a matrix must be drawn on Metal, not declined to software: {stats}"
    );
    let painted = software
        .pixels
        .chunks(4)
        .filter(|p| p[0] == 40 && p[1] == 200 && p[2] == 120)
        .count();
    assert_eq!(painted, 120 * 30, "the turned bar is 30 by 120 pixels");
    if let Err(diff) = compare_within_tolerance(&gpu, &software, Tolerance::GPU_DEFAULT) {
        panic!(
            "Metal's turned group disagrees with the software oracle: {diff}\n\
             The bar unturned means the matrix never reached the backend. The bar in the \
             right place but cut off means the vertex stage's quad was not carried \
             through the forward map. The right outline with the wrong inside means the \
             matrix reached the vertex stage only."
        );
    }
}

/// bug-1083: a circle of radius 0.5 drawn through its own transform, 100 in x and 1 in
/// y, inside a group whose transform scales by `@S@` — `bugs/repro/bug-991/
/// halo_in_shrinking_group.mfb` with the group's scale left open.
const HALO_IN_A_SHRINKING_GROUP: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  LET t AS canvas::Transform = canvas::Transform[a := 100.0, b := 0.0, c := 0.0, d := 1.0, tx := 0.0, ty := 0.0]
  canvas::setGroup("halo", [canvas::Circle[id := "", position := vector::Float2[x := 0.0, y := 0.0], radius := 0.5, paint := canvas::fill(color::rgb(255, 255, 255)), transform := t]])
  canvas::present([canvas::Group[id := "", position := vector::Float2[x := 300.0, y := 200.3], transform := canvas::Transform[a := @S@, b := 0.0, c := 0.0, d := @S@, tx := 0.0, ty := 0.0], name := "halo"]])
END FUNC
"#;

/// The lit pixels of a frame as `(x, y, red)`, in row order.
fn lit(frame: &Frame) -> Vec<(u32, u32, u8)> {
    frame
        .pixels
        .chunks(4)
        .enumerate()
        .filter(|(_, p)| p[0] != 0)
        .map(|(i, p)| (i as u32 % WIDTH, i as u32 / WIDTH, p[0]))
        .collect()
}

/// bug-1083: the box a group's matrix carries an item's coverage in holds every pixel
/// that takes coverage, when the group shrinks an item whose own transform is uneven.
///
/// Under a transform a pixel takes coverage where the distance in the item's space is
/// under half the length of its central differences, and that length is bounded by the
/// norm of the COMPOSED inverse — the item's, then the groups'. So the rim a box needs
/// past the item's extent is known only where the last matrix is: a rim sized by the
/// item's own inverse (bug-991) and carried through a group that scales by a tenth is a
/// tenth of what the group's inverse makes the pixels need. The thin ellipse here is
/// drawn with a faint row of pixels far past its ends (the first-order estimate's own
/// artefact, which every renderer draws alike); a box that is short cuts that row off.
///
/// At a tenth the row is 26 pixels and nothing cuts it. At three hundredths the row
/// before bug-991 was 12 pixels, and those have to stay, on both renderers.
#[test]
fn an_unevenly_transformed_item_in_a_shrinking_group_keeps_every_covered_pixel() {
    if !cfg!(target_os = "macos") {
        return;
    }
    // (x, red) along row 200, as drawn before bug-991 changed the item's rim.
    let tenth: [(u32, u8); 26] = [
        (287, 38),
        (288, 53),
        (289, 64),
        (290, 71),
        (291, 77),
        (292, 83),
        (293, 86),
        (294, 90),
        (295, 93),
        (296, 96),
        (297, 98),
        (298, 98),
        (299, 99),
        (300, 99),
        (301, 98),
        (302, 98),
        (303, 96),
        (304, 93),
        (305, 90),
        (306, 86),
        (307, 83),
        (308, 77),
        (309, 71),
        (310, 64),
        (311, 53),
        (312, 38),
    ];
    let three_hundredths: [(u32, u8); 12] = [
        (294, 34),
        (295, 42),
        (296, 46),
        (297, 50),
        (298, 53),
        (299, 56),
        (300, 56),
        (301, 53),
        (302, 50),
        (303, 46),
        (304, 42),
        (305, 34),
    ];
    for (scale, row, exact) in [
        ("0.1", &tenth[..], true),
        ("0.03", &three_hundredths[..], false),
    ] {
        let name = format!("canvas_metal_halo_group_{}", scale.replace('.', "_"));
        let program = build(&name, &HALO_IN_A_SHRINKING_GROUP.replace("@S@", scale));
        let (software, _) = render(&program, false, "sw");
        let (gpu, stats) = render(&program, true, "gpu");
        let drawn = lit(&software);
        for &(x, red) in row {
            assert!(
                drawn.contains(&(x, 200, red)),
                "group scale {scale}, software: pixel ({x}, 200) should be lit at {red}; the \
                 lit pixels are {drawn:?}"
            );
        }
        if exact {
            assert_eq!(
                drawn.len(),
                row.len(),
                "group scale {scale}, software: the lit pixels are {drawn:?}"
            );
        }
        if !metal_built(&stats) {
            continue;
        }
        assert!(
            !stats.contains("gpuFrames=0"),
            "group scale {scale}: the frame must be drawn on Metal: {stats}"
        );
        let on_metal: Vec<(u32, u32)> = lit(&gpu).iter().map(|&(x, y, _)| (x, y)).collect();
        let on_software: Vec<(u32, u32)> = drawn.iter().map(|&(x, y, _)| (x, y)).collect();
        assert_eq!(
            on_metal, on_software,
            "group scale {scale}: Metal lights a different set of pixels from the software \
             oracle — its quad is not the box the oracle walks"
        );
        if let Err(diff) = compare_within_tolerance(&gpu, &software, Tolerance::GPU_DEFAULT) {
            panic!("group scale {scale}: Metal disagrees with the software oracle: {diff}");
        }
    }
}

/// Every group case matches the software oracle — plan-116-H Phase 3's acceptance.
///
/// What this asserts is not "groups draw" but "the per-draw OFFSET reaches both shader
/// stages". Every item in the scene lives inside a group, so a backend that ignored the
/// offset would produce a complete, plausible picture with every shape stacked at the
/// origin — the failure `.ai/canvas-threading.md` §10 is about, and one that only a
/// comparison against the software oracle at a NON-ZERO offset can see.
///
/// The cases are chosen so each can only pass for the right reason:
///
/// * a group at `(0,0)` — the offset is present and zero;
/// * a group at `(37,53)` — non-zero;
/// * a **nested** group — `(380,40)` and `(15,25)` must compose to `(395,65)`;
/// * a **diamond** — one leaf, two references, two offsets, sharing one base;
/// * a **clipped** item inside a translated group — the clip is evaluated in surface
///   space while the shape is evaluated in shape space, so a backend that moved both
///   together would drag the clip window along with the group;
/// * a **gradient-filled** item inside a translated group — the ramp is sampled per
///   fragment, so a vertex-stage-only offset draws the shape correctly with the ramp
///   shifted inside it.
///
/// That last case is why `setVertexBytes:` alone is not enough and the fragment stage
/// gets the offset too. The `Text`-in-a-group case needs a font file beside the binary
/// and is covered by `scripts/test-canvas-vulkan.sh`, which ships one.
///
/// Measured when it landed: **worst channel delta 1, 0.0028% of pixels differing** —
/// inside `Tolerance::GPU_DEFAULT`'s per-pixel bound, not merely its population budget.
#[test]
fn every_group_case_matches_the_software_oracle() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let program = build("canvas_metal_groups", GROUPS);
    let (software, _) = render(&program, false, "sw");
    let (gpu, stats) = render(&program, true, "gpu");
    if !metal_built(&stats) {
        return; // no Metal device on this host (§metal_built)
    }
    // Asserted BEFORE the pixels. A declined group scene falls back to software, and
    // then this test would be comparing the software renderer against itself and
    // passing for no reason — which is exactly what it did while plan-116-G's decline
    // was in place.
    assert!(
        !stats.contains("gpuFrames=0"),
        "the group scene never reached Metal, so the comparison below would be the \
         software renderer against itself: {stats}"
    );
    if let Err(diff) = compare_within_tolerance(&gpu, &software, Tolerance::GPU_DEFAULT) {
        panic!(
            "the Metal backend disagrees with the software oracle on a group scene: \
             {diff}\n\
             Each case sits in its own region, so the coordinate localizes it: (20,20) \
             at-origin; (237,113) moved; (395,65) nested; (600,40) and (700,140) the \
             diamond; (60,200) clipped; (420,200) gradient. A shape at the ORIGIN \
             instead of its group offset means the offset never reached the stage that \
             shape depends on — the vertex stage moves the quad, the fragment stage \
             moves the query point, and a gradient needs both."
        );
    }
}

/// Grouping changes cost, not pixels.
///
/// The strongest check available for this letter, and the one that does not depend on a
/// stored reference: the same picture is drawn twice, once as a group named at two
/// offsets and once as four items written out longhand at the absolute coordinates those
/// offsets imply. The two frames must be **byte-identical**.
///
/// Compared exactly rather than within tolerance, and both sides on the GPU. Both frames
/// come from the same backend on the same host in the same run, so every source of the
/// slack `Tolerance::GPU_DEFAULT` exists for — float blending, sRGB conversion, rounding
/// — is common to both and cancels. What is left is precisely the question being asked:
/// does routing an item through a group change the pixels it produces? A tolerance here
/// would hide exactly the small offset errors this letter is about, because a shape one
/// pixel off still agrees with itself to within two steps almost everywhere.
#[test]
fn a_diamond_renders_identically_to_the_flat_scene_it_stands_for() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let grouped = build("canvas_metal_diamond_grouped", DIAMOND_GROUPED);
    let flat = build("canvas_metal_diamond_flat", DIAMOND_FLAT);
    let (grouped_frame, stats) = render(&grouped, true, "gpu");
    if !metal_built(&stats) {
        return; // no Metal device on this host (§metal_built)
    }
    assert!(
        !stats.contains("gpuFrames=0"),
        "the grouped scene never reached the GPU, so this would compare two software \
         frames and prove nothing about the backend: {stats}"
    );
    let (flat_frame, flat_stats) = render(&flat, true, "gpu");
    assert!(
        !flat_stats.contains("gpuFrames=0"),
        "the flat control scene never reached the GPU: {flat_stats}"
    );
    if let Err(diff) = compare_exact(&grouped_frame, &flat_frame) {
        panic!(
            "a group's children do not land where the same items written out longhand \
             do: {diff}\n\
             The two scenes differ only in that one routes its items through a group at \
             (120,90) and (430,300). A difference at one of those two places is the \
             per-draw offset; a difference at both is the group expansion itself; and a \
             difference in the stroked circle only is the fragment stage, whose distance \
             is evaluated at an absolute point."
        );
    }
}

/// A scene whose only item names a group that was never installed draws nothing — on the
/// GPU as well as in software.
///
/// The GPU arm is the point. `canvas::groupResolve` returns no slot, the walk emits no
/// blocks, and the draw list is empty; a backend that treated "no entries" as "draw
/// everything published so far" or that walked a zero-length list badly would show it
/// here and nowhere else. Compared against a genuinely empty scene rather than against a
/// stored image, so it cannot pass by both sides being equally broken in the same way as
/// a reference made from one of them.
#[test]
fn a_group_naming_an_absent_group_draws_nothing_on_the_gpu() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let absent = build("canvas_metal_absent_group", ABSENT_GROUP);
    let empty = build("canvas_metal_empty_scene", EMPTY_SCENE);
    let (absent_frame, stats) = render(&absent, true, "gpu");
    if !metal_built(&stats) {
        return; // no Metal device on this host (§metal_built)
    }
    let (empty_frame, _) = render(&empty, true, "gpu");
    if let Err(diff) = compare_exact(&absent_frame, &empty_frame) {
        panic!(
            "a `canvas::Group` naming a group that was never installed drew something: \
             {diff}\n\
             It must resolve to no blocks and no draw entries, exactly as an empty \
             scene does."
        );
    }
}

/// A line of distinct glyphs, drawn with a real system font. The fixture font the
/// other text tests use gives every character one glyph shape, which is exactly
/// what hid the bug below.
const TEXT_LINE: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("/System/Library/Fonts/Supplemental/Arial.ttf")
  LET bg AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 900.0, height := 640.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(20, 24, 30)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET a AS canvas::DrawItem = canvas::Text[id := "", position := vector::Float2[x := 20.0, y := 40.0], text := "LEVEL 01", fonts := [face], fontSize := 22.0, letterSpacing := 0.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET b AS canvas::DrawItem = canvas::Text[id := "", position := vector::Float2[x := 300.0, y := 40.0], text := "BUGS quick", fonts := [face], fontSize := 22.0, letterSpacing := 0.0, paint := canvas::fill(color::rgb(255, 214, 90)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  canvas::present([bg, a, b])
END FUNC
"#;

/// Each glyph of a text run samples its OWN bitmap on the GPU.
///
/// plan-116-H made a text item one instanced draw, but left each glyph's bitmap as a
/// per-glyph `setFragmentBytes:` at index 2 set in the publish loop — so by the time
/// the draw ran, only the run's LAST glyph was bound, and every glyph drew that
/// glyph's pixels at its own width: "LEVEL 01" came out as hatching and a clean "1".
/// It passed every existing test because those compare whole frames within a 2%
/// pixel budget, and a line of 22 px text is far less than 2% of 900x640.
///
/// So this compares only the rows the text occupies, where garbled glyphs are most of
/// the pixels.
#[test]
fn every_glyph_of_a_text_run_draws_its_own_bitmap() {
    if !cfg!(target_os = "macos")
        || !std::path::Path::new("/System/Library/Fonts/Supplemental/Arial.ttf").exists()
    {
        return;
    }
    let program = build("canvas_metal_text_line", TEXT_LINE);
    let (software, _) = render(&program, false, "sw");
    let (gpu, stats) = render(&program, true, "gpu");
    if !metal_built(&stats) {
        return; // no Metal device on this host (§metal_built)
    }
    assert!(
        !stats.contains("gpuFrames=0"),
        "MFB_CANVAS_GPU=1 did not select the Metal renderer: {stats}"
    );
    let band = |frame: &Frame| {
        let rows = 60usize;
        let stride = WIDTH as usize * 4;
        Frame::from_rgba(WIDTH, rows as u32, frame.pixels[..rows * stride].to_vec())
    };
    if let Err(diff) =
        compare_within_tolerance(&band(&gpu), &band(&software), Tolerance::GPU_DEFAULT)
    {
        panic!(
            "the Metal backend draws a text run differently from the software oracle: \
             {diff}\nA glyph drawn with ANOTHER glyph's bitmap (hatching, or one letter \
             repeated) is the per-glyph payload being overwritten before the draw."
        );
    }
}

// ---------------------------------------------------------------------------------------
// bug-686: ordinary scenes that used to fall back to software must draw on Metal.
//
// Every test below asserts two things, in this order: that Metal drew the frame
// (`gpuFrames` is not 0 — otherwise the pixel comparison would be the software renderer
// against itself, and pass for no reason), and that what it drew agrees with the
// software oracle within `Tolerance::GPU_DEFAULT`.
// ---------------------------------------------------------------------------------------

/// Build `source`, render it on both backends, and require that Metal drew it and
/// agrees with software. `what` names the scene in the failure message.
fn assert_metal_draws_like_software(name: &str, source: &str, what: &str) {
    assert_metal_draws_within(name, source, what, Tolerance::GPU_DEFAULT, &[]);
}

/// [`assert_metal_draws_like_software`] under a named tolerance and with more of the
/// environment set, for the rows that are not judged under `GPU_DEFAULT` or that script
/// the pointer.
fn assert_metal_draws_within(
    name: &str,
    source: &str,
    what: &str,
    tolerance: Tolerance,
    envs: &[(&str, &str)],
) {
    if !cfg!(target_os = "macos") {
        return;
    }
    let program = build(name, source);
    let (software, _) = render_env(&program, false, "sw", envs);
    let (gpu, stats) = render_env(&program, true, "gpu", envs);
    if !metal_built(&stats) {
        return; // no Metal device on this host (§metal_built)
    }
    assert!(
        software.pixels.chunks(4).any(|p| p != [0, 0, 0, 255]),
        "the software render of {what} drew nothing, so the comparison would be vacuous",
    );
    assert!(
        !stats.contains("gpuFrames=0"),
        "{what} fell back to the software renderer instead of drawing on Metal (bug-686): \
         {stats}"
    );
    if let Err(diff) = compare_within_tolerance(&gpu, &software, tolerance) {
        panic!("Metal draws {what} differently from the software oracle: {diff}");
    }
}
