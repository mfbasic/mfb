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

/// Many polygons that individually fit but together overflow the frame's edge region.
///
/// plan-116-A declined this scene: 40,000 edges was past the 16,384-edge frame region
/// that letter introduced. bug-686 raised the region to 262,144 edges, and the scene is
/// now a Metal acceptance case (`forty_thousand_polygon_edges_draw_on_metal`). The
/// decline it used to prove was `PAST_THE_FRAME_EDGE_REGION`'s job until bug-765, which
/// made that scene two passes; `TOO_MANY_EDGES` is the decline that is left.
const TOO_MANY_FRAME_EDGES: &str = include_str!("scenes/forty_thousand_edges.mfb");

/// Polygons whose edges together pass Metal's frame edge region
/// (`METAL_MAX_FRAME_EDGES` = 262,144 since bug-686).
///
/// 1,025 rings of 256 edges is 262,400 edges — 256 past the region, and no ring past
/// any per-polygon limit, so only the *sum* can end a pass. The rings are 4 px across so
/// the software oracle, which walks every edge for every pixel of a polygon's bounds,
/// draws the frame in seconds rather than minutes. The rectangle keeps the frame from
/// being blank.
const PAST_THE_FRAME_EDGE_REGION: &str = include_str!("scenes/past_the_edge_region.mfb");

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
    // Every item is one block at least, and the scene's two polygons have edges.
    assert!(
        stat(line, "profBytes") >= stat(line, "blocks") * 368,
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

/// Two `canvas::PolyLine`s (plan-154-A): one with a colour per point, one plain with
/// butt ends, beside a polygon so the frame is otherwise one Metal draws. The third is
/// long enough (40 segments) to get a band index, so the band reach for a polyline is
/// exercised, and it doubles back across its own rows.
const POLYLINES: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT collections
IMPORT color

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  LET tri AS canvas::DrawItem = canvas::Polygon[id := "", position := vector::Float2[x := 0.0, y := 0.0], contours := [[vector::Float2[x := 600.0, y := 40.0], vector::Float2[x := 700.0, y := 40.0], vector::Float2[x := 650.0, y := 130.0]]], rule := canvas::FillRule.EvenOdd, paint := canvas::fill(color::rgba(0, 200, 255, 180)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET trail AS canvas::DrawItem = canvas::PolyLine[id := "", position := vector::Float2[x := 0.0, y := 0.0], points := [vector::Float2[x := 60.0, y := 300.0], vector::Float2[x := 160.0, y := 200.0], vector::Float2[x := 260.0, y := 300.0], vector::Float2[x := 360.0, y := 200.0]], colors := [color::rgba(255, 140, 40, 255), color::rgba(40, 255, 140, 170), color::rgba(140, 40, 255, 85), color::rgba(255, 255, 255, 0)], paint := WITH canvas::stroke(color::rgba(0, 0, 0, 0), 18.0) { cap := canvas::CapStyle.Round }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET plain AS canvas::DrawItem = canvas::PolyLine[id := "", position := vector::Float2[x := 0.0, y := 0.0], points := [vector::Float2[x := 460.0, y := 500.0], vector::Float2[x := 600.0, y := 420.0], vector::Float2[x := 760.0, y := 520.0]], colors := [], paint := WITH canvas::stroke(color::rgb(255, 214, 92), 12.0) { cap := canvas::CapStyle.Butt }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  MUT wave AS List OF vector::Float2 = []
  MUT hues AS List OF color::Color = []
  MUT k AS Integer = 0
  WHILE k <= 40
    LET x AS Float = 80.0 + toFloat(k) * 18.0
    LET y AS Float = 420.0 + toFloat((k MOD 4) * 22) - toFloat(k) * 2.5
    wave = collections::append(wave, vector::Float2[x := x, y := y])
    hues = collections::append(hues, color::rgba(40 + k * 5, 200 - k * 4, 255, 255 - k * 5))
    k = k + 1
  END WHILE
  LET long AS canvas::DrawItem = canvas::PolyLine[id := "", position := vector::Float2[x := 0.0, y := 0.0], points := wave, colors := hues, paint := WITH canvas::stroke(color::rgb(255, 255, 255), 7.0) { cap := canvas::CapStyle.Butt }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  canvas::present([tri, trail, plain, long])
END FUNC
"#;

/// Metal draws `canvas::PolyLine` itself and matches the software oracle (plan-154-B).
///
/// Gated on the frame having been drawn on the GPU first — `gpuFrames` equal to
/// `frames` — because a declined frame falls back to the oracle and would match it
/// exactly, which is the passing picture a broken shader branch hides behind.
///
/// The scene carries a per-point colour trail (the path parameter's gradient lookup),
/// a plain butt-capped path (the one-sided butt planes and the round joins), and a
/// polygon, so the frame is one Metal draws in every other respect.
#[test]
fn polylines_match_the_software_oracle_on_metal() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let program = build("canvas_metal_polylines", POLYLINES);
    let (software, _) = render(&program, false, "sw");
    let (gpu, stats) = render(&program, true, "gpu");
    if !metal_built(&stats) {
        return;
    }
    let line = stats.lines().next_back().expect("a stats line");
    let count = |name: &str| -> i64 {
        line.split_whitespace()
            .find_map(|f| f.strip_prefix(&format!("{name}=")))
            .unwrap_or_else(|| panic!("no `{name}=` in {line}"))
            .parse()
            .expect("a number")
    };
    assert!(
        count("frames") > 0 && count("gpuFrames") == count("frames"),
        "every PolyLine frame must be drawn on Metal, not declined to software: {line}"
    );
    let painted = software
        .pixels
        .chunks(4)
        .filter(|p| p[0] != 0 || p[1] != 0 || p[2] != 0)
        .count();
    assert!(
        painted > 5_000,
        "the scene drew {painted} pixels, so it proves nothing"
    );
    if let Err(diff) = compare_within_tolerance(&gpu, &software, Tolerance::GPU_DEFAULT) {
        panic!(
            "Metal's PolyLine disagrees with the software oracle: {diff}\n\
             A butt end that domes is the first/last segment words (`gradient.x`/`.y`); \
             a colour that jumps at a corner is the path parameter; a missing path is \
             the edge upload or the band reach (`emit_band_index`, radius slot)."
        );
    }
}

const PIVOTED: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  LET quarter AS canvas::Transform = canvas::Transform[a := 0.0, b := 1.0, c := 0.0 - 1.0, d := 0.0, tx := 0.0, ty := 0.0]
  LET egg AS canvas::DrawItem = canvas::Ellipse[id := "", position := vector::Float2[x := 300.0, y := 200.0], radiusX := 150.0, radiusY := 20.0, paint := canvas::fillStroke(color::rgb(255, 255, 255), color::rgb(255, 120, 0), 6.0), transform := quarter]
  LET ring AS canvas::DrawItem = canvas::Arc[id := "", position := vector::Float2[x := 600.0, y := 300.0], radius := 90.0, startAngle := 0.0, endAngle := 2.0, paint := canvas::stroke(color::rgb(120, 220, 255), 12.0), transform := quarter]
  LET dot AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 600.0, y := 520.0], radius := 60.0, paint := canvas::fill(color::rgb(160, 255, 160)), transform := canvas::Transform[a := 1.5, b := 0.0, c := 0.0, d := 0.5, tx := 0.0, ty := 0.0]]
  canvas::present([egg, ring, dot])
END FUNC
"#;

/// plan-172-D: Metal draws a shape turned about its `position` where the software
/// oracle does. A turned ellipse, a turned arc and a circle squashed in place: each is
/// built at its local origin and carried by an affine whose translation is its
/// position, which is the path the GPU's inverse-mapped query point has to agree on.
///
/// Gated on the frame having been drawn on the GPU, as every parity case here is.
#[test]
fn shapes_turned_about_their_position_match_the_software_oracle() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let program = build("canvas_metal_pivoted", PIVOTED);
    let (software, _) = render(&program, false, "sw");
    let (gpu, stats) = render(&program, true, "gpu");
    if !metal_built(&stats) {
        return;
    }
    let line = stats.lines().next_back().expect("a stats line");
    let count = |name: &str| -> i64 {
        line.split_whitespace()
            .find_map(|f| f.strip_prefix(&format!("{name}=")))
            .unwrap_or_else(|| panic!("no `{name}=` in {line}"))
            .parse()
            .expect("a number")
    };
    assert!(
        count("frames") > 0 && count("gpuFrames") == count("frames"),
        "the pivoted scene must be drawn on Metal, not declined to software: {line}"
    );
    let painted = software
        .pixels
        .chunks(4)
        .filter(|p| p[0] != 0 || p[1] != 0 || p[2] != 0)
        .count();
    assert!(
        painted > 5_000,
        "the scene drew {painted} pixels, so it proves nothing"
    );
    if let Err(diff) = compare_within_tolerance(&gpu, &software, Tolerance::GPU_DEFAULT) {
        panic!("Metal's pivoted shapes disagree with the software oracle: {diff}");
    }
}

/// plan-172-F: the point-list shapes, each turned about its `position`.
const PATH_PLACEMENT: &str = r#"IMPORT app
IMPORT canvas
IMPORT color
IMPORT vector

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  LET half AS canvas::Transform = canvas::Transform[a := 0.0 - 1.0, b := 0.0, c := 0.0, d := 0.0 - 1.0, tx := 0.0, ty := 0.0]
  LET tilt AS canvas::Transform = canvas::Transform[a := 0.8, b := 0.6, c := 0.0 - 0.6, d := 0.8, tx := 0.0, ty := 0.0]
  LET wedge AS canvas::DrawItem = canvas::Polygon[id := "", position := vector::Float2[x := 250.0, y := 200.0], contours := [[vector::Float2[x := 0.0, y := 0.0 - 120.0], vector::Float2[x := 100.0, y := 80.0], vector::Float2[x := 0.0 - 100.0, y := 80.0]]], rule := canvas::FillRule.EvenOdd, paint := canvas::fillStroke(color::rgb(255, 160, 0), color::rgb(255, 255, 255), 6.0), transform := half]
  LET bar AS canvas::DrawItem = canvas::Line[id := "", position := vector::Float2[x := 600.0, y := 200.0], startAt := vector::Float2[x := 0.0 - 150.0, y := 0.0], endAt := vector::Float2[x := 150.0, y := 0.0], paint := canvas::stroke(color::rgb(120, 200, 255), 14.0), transform := tilt]
  LET trail AS canvas::DrawItem = canvas::PolyLine[id := "", position := vector::Float2[x := 450.0, y := 470.0], points := [vector::Float2[x := 0.0 - 200.0, y := 0.0], vector::Float2[x := 0.0 - 60.0, y := 0.0 - 70.0], vector::Float2[x := 60.0, y := 70.0], vector::Float2[x := 200.0, y := 0.0]], colors := [color::rgb(255, 0, 0), color::rgb(0, 255, 0), color::rgb(0, 0, 255), color::rgb(255, 255, 255)], paint := WITH canvas::stroke(color::rgb(255, 255, 255), 10.0) { join := canvas::JoinStyle.Round }, transform := tilt]
  LET moved AS canvas::DrawItem = canvas::Polygon[id := "", position := vector::Float2[x := 760.0, y := 480.0], contours := [[vector::Float2[x := 0.0 - 60.0, y := 0.0 - 60.0], vector::Float2[x := 60.0, y := 0.0 - 60.0], vector::Float2[x := 0.0, y := 60.0]]], rule := canvas::FillRule.EvenOdd, paint := canvas::fill(color::rgb(160, 255, 160)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  canvas::present([wedge, bar, trail, moved])
END FUNC
"#;

/// plan-172-F: Metal draws a `Polygon`, a `Line` and a `PolyLine` turned about their
/// `position` where the software oracle does, and an unturned polygon at its position.
/// The first three are built at the local origin and carried by an affine whose
/// translation is the position; the last is built at its position.
///
/// Gated on the frame having been drawn on the GPU, as every parity case here is.
#[test]
fn path_placement_matches_the_software_oracle() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let program = build("canvas_metal_path_placement", PATH_PLACEMENT);
    let (software, _) = render(&program, false, "sw");
    let (gpu, stats) = render(&program, true, "gpu");
    if !metal_built(&stats) {
        return;
    }
    let line = stats.lines().next_back().expect("a stats line");
    let count = |name: &str| -> i64 {
        line.split_whitespace()
            .find_map(|f| f.strip_prefix(&format!("{name}=")))
            .unwrap_or_else(|| panic!("no `{name}=` in {line}"))
            .parse()
            .expect("a number")
    };
    assert!(
        count("frames") > 0 && count("gpuFrames") == count("frames"),
        "the path-placement scene must be drawn on Metal, not declined to software: {line}"
    );
    let painted = software
        .pixels
        .chunks(4)
        .filter(|p| p[0] != 0 || p[1] != 0 || p[2] != 0)
        .count();
    assert!(
        painted > 20_000,
        "the scene drew {painted} pixels, so it proves nothing"
    );
    if let Err(diff) = compare_within_tolerance(&gpu, &software, Tolerance::GPU_DEFAULT) {
        panic!("Metal's placed paths disagree with the software oracle: {diff}");
    }
}

/// A frame whose polygons individually fit but together overflow the edge region is
/// drawn in passes, on Metal, and matches the oracle (bug-765).
///
/// Until bug-765 this frame was declined: the edge region was a FRAME cap, and 262,400
/// edges past it sent the whole frame to the software renderer, byte for byte. The
/// owner ruled that out on 2026-10-12 (the bug document's Cause note: no scene size is
/// a limit; the region is the size of one pass). What the decline protected still
/// holds and is still what this asserts: the frame is never drawn truncated at the
/// region's end, and no ring reads another's edges. 1,025 rings of 256 edges is one
/// ring past the region, so the frame is two passes, split where the EDGE region fills
/// (the item region holds all 1,026 blocks sixty times over). A renderer that put all
/// of them in one pass would leave the last ring undrawn or drawn from the wrong
/// edges, and the comparison fails.
///
/// `gpuFrames` is asserted because rings this small draw on Metal byte-identically to
/// software: pixel agreement alone would pass for a frame that was declined.
#[test]
fn a_frame_whose_polygons_together_overflow_the_edge_region_draws_in_passes() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let program = build("canvas_metal_frame_edges", PAST_THE_FRAME_EDGE_REGION);
    let (software, _) = render(&program, false, "sw");
    let (gpu, stats) = render(&program, true, "gpu");
    if !metal_built(&stats) {
        return;
    }
    assert!(
        software.pixels.iter().any(|&b| b != 0),
        "the software render drew nothing, so the comparison would be vacuous",
    );
    assert!(
        !stats.contains("gpuFrames=0"),
        "262,400 edges is two passes of METAL_MAX_FRAME_EDGES, but the software renderer \
         drew the frame: {stats}"
    );
    if let Err(diff) = compare_within_tolerance(&gpu, &software, Tolerance::GPU_DEFAULT) {
        panic!(
            "1,025 rings of 256 edges is 262,400 edges, one ring past one pass's edge \
             region — Metal must draw the frame in two passes and agree with the oracle, \
             but {diff}"
        );
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
  LET a AS canvas::DrawItem = canvas::Text[id := "", position := vector::Float2[x := 20.0, y := 40.0], text := "LEVEL 01", font := face, fontSize := 22.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET b AS canvas::DrawItem = canvas::Text[id := "", position := vector::Float2[x := 300.0, y := 40.0], text := "BUGS quick", font := face, fontSize := 22.0, paint := canvas::fill(color::rgb(255, 214, 90)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
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

/// Every `canvas::Picture` path, in one scene (bug-484).
///
/// Pictures ride the frame buffer's glyph region, one packed RGBA texel per word, and
/// each picture's block names its own slice — so the scene carries **several** pictures
/// and a text run between them: with one picture its slice starts at zero, and a base
/// that was never written would pass. The text interleaves glyph slices with picture
/// slices in the same region, which is what proves the two share one cursor.
///
/// Beyond that it covers each thing the picture inherits from the rectangle: nearest
/// scaling of a 2x2 image, the fill tint, a translucent texel, a 90-degree transform, a
/// clip, a non-Normal blend that also strokes (the split into two instances), a group
/// offset, and a destination on fractional coordinates for the antialiased edge.
const PICTURES: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  LET quad AS List OF Byte = [toByte(255), toByte(0), toByte(0), toByte(255), toByte(0), toByte(255), toByte(0), toByte(255), toByte(0), toByte(0), toByte(255), toByte(255), toByte(255), toByte(255), toByte(255), toByte(255)]
  LET strip AS List OF Byte = [toByte(255), toByte(255), toByte(0), toByte(255), toByte(0), toByte(0), toByte(255), toByte(128), toByte(0), toByte(255), toByte(255), toByte(255)]
  RES img AS canvas::Image = canvas::createImage(2, 2, quad)
  RES thin AS canvas::Image = canvas::createImage(3, 1, strip)
  RES grouped AS canvas::Image = canvas::createImage(2, 2, quad)
  RES face AS canvas::Font = canvas::loadFont("/System/Library/Fonts/Supplemental/Arial.ttf")
  LET white AS canvas::Paint = canvas::fill(color::rgb(255, 255, 255))
  LET ground AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 900.0, height := 260.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(96, 96, 96)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET plain AS canvas::DrawItem = canvas::Picture[id := "", shape := canvas::Rectangle[id := "", position := vector::Float2[x := 10.0, y := 10.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 80.0, height := 80.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := white, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]], clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img]
  LET label AS canvas::DrawItem = canvas::Text[id := "", position := vector::Float2[x := 10.0, y := 240.0], text := "Pictures 01", font := face, fontSize := 22.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET tinted AS canvas::DrawItem = canvas::Picture[id := "", shape := canvas::Rectangle[id := "", position := vector::Float2[x := 100.0, y := 10.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 80.0, height := 80.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 128, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]], clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img]
  LET translucent AS canvas::DrawItem = canvas::Picture[id := "", shape := canvas::Rectangle[id := "", position := vector::Float2[x := 200.0, y := 10.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 90.0, height := 30.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := white, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]], clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := thin]
  LET t AS canvas::Transform = canvas::Transform[a := 0.0, b := 1.0, c := 0.0 - 1.0, d := 0.0, tx := 400.0, ty := 10.0]
  LET rotated AS canvas::DrawItem = canvas::Picture[id := "", shape := canvas::Rectangle[id := "", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 80.0, height := 60.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := white, transform := t], clip := canvas::PictureClip.ClipAndTransform, fit := canvas::PictureFit.Stretch, image := img]
  LET clipped AS canvas::DrawItem = canvas::Picture[id := "", shape := canvas::Rectangle[id := "", position := vector::Float2[x := 420.0, y := 10.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 80.0, height := 80.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH white { clip := [[vector::Float2[x := 0.0, y := 0.0], vector::Float2[x := 40.0, y := 0.0], vector::Float2[x := 40.0, y := 80.0], vector::Float2[x := 0.0, y := 80.0]]] }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]], clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img]
  LET blended AS canvas::DrawItem = canvas::Picture[id := "", shape := canvas::Rectangle[id := "", position := vector::Float2[x := 520.0, y := 10.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 60.0, height := 60.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH canvas::fillStroke(color::rgb(255, 255, 255), color::rgb(40, 120, 230), 6.0) { blend := canvas::BlendMode.Multiply }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]], clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img]
  LET fractional AS canvas::DrawItem = canvas::Picture[id := "", shape := canvas::Rectangle[id := "", position := vector::Float2[x := 700.5, y := 10.25], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 50.5, height := 33.3], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := white, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]], clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := thin]
  LET inner AS canvas::DrawItem = canvas::Picture[id := "", shape := canvas::Rectangle[id := "", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 40.0, height := 40.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := white, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]], clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := grouped]
  canvas::setGroup("pics", [inner])
  LET node AS canvas::DrawItem = canvas::Group[id := "", position := vector::Float2[x := 600.0, y := 120.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := "pics"]
  ' plan-163-C: the tint's strength is the fill's alpha, the opacity is transparency.
  LET untinted AS canvas::DrawItem = canvas::Picture[id := "", shape := canvas::Rectangle[id := "", position := vector::Float2[x := 10.0, y := 120.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 80.0, height := 80.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH white { fill := color::rgba(0, 0, 0, 0) }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]], clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img]
  LET halfRed AS canvas::DrawItem = canvas::Picture[id := "", shape := canvas::Rectangle[id := "", position := vector::Float2[x := 100.0, y := 120.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 80.0, height := 80.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgba(255, 0, 0, 128)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]], clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img]
  LET faded AS canvas::DrawItem = canvas::Picture[id := "", shape := canvas::Rectangle[id := "", position := vector::Float2[x := 200.0, y := 120.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 80.0, height := 80.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH white { transparency := 0.6 }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]], clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img]
  ' plan-163-D: a blended stroked picture whose fill is transparent (the zero tint) still
  ' fills with its image, so the GPUs publish it as two records like any blended item
  ' that fills and strokes.
  LET blendedClear AS canvas::DrawItem = canvas::Picture[id := "", shape := canvas::Rectangle[id := "", position := vector::Float2[x := 300.0, y := 120.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 80.0, height := 80.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH canvas::fillStroke(color::rgba(0, 0, 0, 0), color::rgb(40, 120, 230), 6.0) { blend := canvas::BlendMode.Multiply }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]], clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img]
  canvas::present([ground, plain, label, tinted, translucent, rotated, clipped, blended, fractional, node, untinted, halfRed, faded, blendedClear])
END FUNC
"#;

/// The Metal backend draws every picture path the same as the software oracle, and
/// the frame is proven to have been drawn by Metal (bug-484 Phase 3).
///
/// Until Phase 3 `__canvas_metalRenderable` declined any scene with a picture, so this
/// fails on `gpuFrames=0` rather than on pixels — the honesty gate working, not the
/// backend drawing. Compared on the band the pictures and the text occupy, for the
/// reason the text-run test crops: they are far less than the whole frame's budget.
#[test]
fn every_picture_path_matches_the_software_oracle() {
    if !cfg!(target_os = "macos")
        || !std::path::Path::new("/System/Library/Fonts/Supplemental/Arial.ttf").exists()
    {
        return;
    }
    let program = build("canvas_metal_pictures", PICTURES);
    let (software, _) = render(&program, false, "sw");
    let (gpu, stats) = render(&program, true, "gpu");
    if !metal_built(&stats) {
        return; // no Metal device on this host (§metal_built)
    }
    assert!(
        !stats.contains("gpuFrames=0"),
        "MFB_CANVAS_GPU=1 did not draw the picture scene on Metal: {stats}"
    );
    let band = |frame: &Frame| {
        let rows = 260usize;
        let stride = WIDTH as usize * 4;
        Frame::from_rgba(WIDTH, rows as u32, frame.pixels[..rows * stride].to_vec())
    };
    if let Err(diff) =
        compare_within_tolerance(&band(&gpu), &band(&software), Tolerance::GPU_DEFAULT)
    {
        panic!(
            "the Metal backend draws a picture differently from the software oracle: \
             {diff}\nA picture showing ANOTHER picture's or a glyph's texels is a slice \
             base that was not written or a cursor the two kinds do not share; a whole \
             picture off by one texel column is the nearest-sampling arithmetic."
        );
    }
}

/// A text run containing a space, followed by items whose draw state differs: a
/// blended circle and a translated group (bug-484, found by the picture scene).
///
/// A space has an empty bitmap and draws nothing, but `__canvas_blockInstances` counts
/// it as one instance like every glyph. The Metal emitter used to skip publishing its
/// block, so every later draw entry named the block one position on: the circle drew
/// under the next item's pipeline and the group lost its offset. `TEXT_LINE` above has
/// spaces too, but nothing after its runs has a draw state of its own, so the shift was
/// invisible there.
const SPACE_THEN_SHAPES: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("/System/Library/Fonts/Supplemental/Arial.ttf")
  LET ground AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 900.0, height := 200.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(128, 128, 128)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET words AS canvas::DrawItem = canvas::Text[id := "", position := vector::Float2[x := 20.0, y := 40.0], text := "A B C", font := face, fontSize := 22.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET mul AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 300.0, y := 100.0], radius := 40.0, paint := WITH canvas::fill(color::rgb(230, 120, 40)) { blend := canvas::BlendMode.Multiply }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET plain AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 450.0, y := 100.0], radius := 40.0, paint := canvas::fill(color::rgb(40, 200, 90)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET dot AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 30.0, height := 30.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(20, 60, 230)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  canvas::setGroup("g", [dot])
  canvas::present([ground, words, mul, plain, canvas::Group[id := "", position := vector::Float2[x := 600.0, y := 80.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := "g"]])
END FUNC
"#;

#[test]
fn a_space_in_a_text_run_does_not_shift_the_draws_after_it() {
    if !cfg!(target_os = "macos")
        || !std::path::Path::new("/System/Library/Fonts/Supplemental/Arial.ttf").exists()
    {
        return;
    }
    let program = build("canvas_metal_space_shift", SPACE_THEN_SHAPES);
    let (software, _) = render(&program, false, "sw");
    let (gpu, stats) = render(&program, true, "gpu");
    if !metal_built(&stats) {
        return; // no Metal device on this host (§metal_built)
    }
    assert!(
        !stats.contains("gpuFrames=0"),
        "MFB_CANVAS_GPU=1 did not draw the scene on Metal: {stats}"
    );
    let band = |frame: &Frame| {
        let rows = 200usize;
        let stride = WIDTH as usize * 4;
        Frame::from_rgba(WIDTH, rows as u32, frame.pixels[..rows * stride].to_vec())
    };
    if let Err(diff) =
        compare_within_tolerance(&band(&gpu), &band(&software), Tolerance::GPU_DEFAULT)
    {
        panic!(
            "items after a text run containing a space draw differently on Metal: {diff}\n\
             A blank glyph must still publish its item block, or every later draw entry \
             names its neighbour's."
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

/// plan-150-A: the particle scene `rt_canvas_golden` holds a reference for — a firework
/// burst and a fountain, 320 live particles between them.
const PARTICLES: &str = include_str!("scenes/particles_on_black.mfb");
/// plan-150-A: a system as a group's child, the group drawn through a turned, scaled
/// node and through a moved one.
const PARTICLES_GROUPED: &str = include_str!("scenes/particles_grouped_on_black.mfb");

/// The particle design's central claim, measured: a particle is an ordinary item by the
/// time a backend sees it — a shape with a position, a transform and a paint — so Metal
/// draws a particle scene with the shaders, pipelines and buffer layout it already had.
/// The scene's particles are scaled, turned, stroked and tinted, each a block of its own.
#[test]
fn a_particle_system_draws_on_metal_and_matches_the_software_oracle() {
    assert_metal_draws_like_software(
        "canvas_metal_particles",
        PARTICLES,
        "the firework and the fountain",
    );
}

/// The second road to the GPU: a system inside a group is blocks of that group's shared
/// run, placed per use by the node's matrix in the draw entry.
#[test]
fn a_particle_system_in_a_turned_group_draws_on_metal() {
    assert_metal_draws_like_software(
        "canvas_metal_particles_grouped",
        PARTICLES_GROUPED,
        "a particle system in a group drawn turned and scaled, and again moved",
    );
}

/// 5,000 small rectangles: past the old 4,096-quad frame cap (`CANVAS_MAX_FRAME_ITEMS`).
const FIVE_THOUSAND_QUADS: &str = include_str!("scenes/five_thousand_quads.mfb");

#[test]
fn a_scene_past_four_thousand_quads_draws_on_metal() {
    assert_metal_draws_like_software(
        "canvas_metal_5000_quads",
        FIVE_THOUSAND_QUADS,
        "a 5,000-rectangle scene",
    );
}

/// The frame-edge scene above (200 rings × 200 edges = 40,000 edges), now expected to draw.
#[test]
fn forty_thousand_polygon_edges_draw_on_metal() {
    assert_metal_draws_like_software(
        "canvas_metal_40k_edges",
        TOO_MANY_FRAME_EDGES,
        "200 translucent rings of 200 edges (40,000 edges)",
    );
}

/// 2,100 gradient-filled rectangles: 4,200 stops, past the old 4,096-stop frame cap.
const MANY_GRADIENTS: &str = include_str!("scenes/many_gradients.mfb");

#[test]
fn a_scene_past_four_thousand_gradient_stops_draws_on_metal() {
    assert_metal_draws_like_software(
        "canvas_metal_gradient_stops",
        MANY_GRADIENTS,
        "2,100 gradient-filled rectangles (4,200 stops)",
    );
}

/// A tilemap: 2,000 32×32 tiles drawn from TWO images. Counted per item the texels are
/// 2,048,000, past the old 1M-texel frame cap; there are only 2,048 distinct texels.
/// The images come from the scene's `pattern` helper: texels that vary by position and
/// seed, so a picture drawn from the wrong texels shows.
///
/// The scenes in `scenes/` are shared with `scripts/test-canvas-gpu-rows.sh`, which
/// renders them on Vulkan (bug-688), so both backends are held to the same programs.
const TILEMAP: &str = include_str!("scenes/tilemap.mfb");
const TILE: &str = include_str!("scenes/tile.mfb");

#[test]
fn tile_atlas_regions_match_the_software_oracle_on_metal() {
    assert_metal_draws_like_software(
        "canvas_metal_tile",
        TILE,
        "Tile atlas regions including transparent out-of-bounds samples",
    );
}

/// bug-1093: a tile's partly covered edge pixel takes the source rectangle's own edge
/// texel on Metal too -- stated outright, not only against the software oracle, so a
/// bleed that both renderers shared could not pass.
const TILE_EDGE: &str = include_str!("scenes/tile_edge.mfb");

#[test]
fn a_tile_edge_pixel_samples_inside_its_source_rectangle_on_metal() {
    assert_metal_draws_like_software(
        "canvas_metal_tile_edge",
        TILE_EDGE,
        "Tile edge pixels at fractional destinations",
    );
    if !cfg!(target_os = "macos") {
        return;
    }
    let program = build("canvas_metal_tile_edge_px", TILE_EDGE);
    let (gpu, stats) = render(&program, true, "gpu");
    if !metal_built(&stats) {
        return;
    }
    let at = |x: usize, y: usize| {
        let i = (y * WIDTH as usize + x) * 4;
        (gpu.pixels[i], gpu.pixels[i + 1], gpu.pixels[i + 2])
    };
    let (r, g, b) = at(30, 70);
    assert!(
        (r, b) == (0, 0) && g > 150,
        "half-covered bottom edge is green, not the blue row below the source: {:?}",
        (r, g, b)
    );
    assert_eq!(at(120, 30).2, 0, "no blue below the third tile's source");
    assert_eq!(at(120, 10).0, 0, "no red above the third tile's source");
}

#[test]
fn a_tilemap_of_two_thousand_tiles_draws_on_metal() {
    assert_metal_draws_like_software(
        "canvas_metal_tilemap",
        TILEMAP,
        "2,000 32-pixel tiles from two images",
    );
}

/// A 1920×1080 background scaled to the surface, plus one tile over it: 2,074,624
/// texels, past the old 1M-texel frame cap even counted per distinct image.
const BACKGROUND: &str = include_str!("scenes/background.mfb");

#[test]
fn a_full_hd_background_draws_on_metal() {
    assert_metal_draws_like_software(
        "canvas_metal_background",
        BACKGROUND,
        "a 1920x1080 background picture under a tile",
    );
}

/// Polygons past the old 256-edge per-polygon cap: a filled ring of 1,000 points, one of
/// 4,000, a stroked ring of 600, a rotated ring of 500, and a self-intersecting star
/// polygon {301/5} (even-odd fill), all on one surface.
///
/// {301/5} and not a star of long chords such as {301/97}: fp32 against the f64 oracle
/// flips a pixel's quantised coverage by one step where a distance lands within ~3e-5 px
/// of a step boundary, and near black one step is up to 9 sRGB levels -- past
/// `GPU_DEFAULT`'s 2. That is pre-existing and independent of edge count (a {251/97}
/// star, under the old cap, measures max delta 4 at 42cea38df), so a long-chord star
/// would test it rather than this bug; bug-687 records it.
///
/// The scene sits on a mid-grey ground since bug-688 for the same reason: on Vulkan the
/// rotated ring's rim over black flipped one coverage step (4/255 against 5/255, which
/// encodes 29 against 33).
const LARGE_POLYGONS: &str = include_str!("scenes/large_polygons.mfb");

/// A scene installed with `canvas::presentLayers`: two layers, one item each.
const LAYERED: &str = include_str!("scenes/layered.mfb");

/// A layered scene draws its layers on Metal (found fixing bug-686).
///
/// The GPU draw list (`__canvas_sceneDraws`) walked `canvas::installedItems()` only, so a
/// scene installed with `presentLayers` — whose items all live in its layers — published
/// no blocks at all: Metal drew an empty frame and reported `gpuFrames=1`, the honesty
/// gate's worst case (measured: 0 lit pixels against software's 40,332).
#[test]
fn a_layered_scene_draws_its_layers_on_metal() {
    assert_metal_draws_like_software("canvas_metal_layers", LAYERED, "a two-layer scene");
}

#[test]
fn polygons_past_two_hundred_fifty_six_edges_draw_on_metal() {
    assert_metal_draws_like_software(
        "canvas_metal_large_polygons",
        LARGE_POLYGONS,
        "filled, stroked, transformed and self-intersecting polygons of 300-4,000 edges",
    );
}

/// One 64,000-edge polygon, filled and stroked: bug-686 section E's large polygon, which
/// the band index draws in a frame budget where the full per-pixel edge loop took
/// 252 ms. Shared with `scripts/test-canvas-gpu-rows.sh` (bug-688), which draws it on
/// Vulkan.
const HUGE_POLYGON: &str = include_str!("scenes/huge_polygon.mfb");

#[test]
fn a_sixty_four_thousand_edge_polygon_draws_on_metal() {
    assert_metal_draws_like_software(
        "canvas_metal_huge_polygon",
        HUGE_POLYGON,
        "a filled and stroked 64,000-edge polygon",
    );
}

/// 40,000 polygon edges as 200 OPAQUE rings side by side (bug-688): the frame-edge
/// capacity row without `TOO_MANY_FRAME_EDGES`'s 200-deep translucent stack, whose
/// blend drift measures blending rather than edges (see the scene file). Shared with
/// `scripts/test-canvas-gpu-rows.sh`, which draws it on Vulkan.
const OPAQUE_FRAME_EDGES: &str = include_str!("scenes/forty_thousand_opaque_edges.mfb");

#[test]
fn forty_thousand_opaque_polygon_edges_draw_on_metal() {
    assert_metal_draws_like_software(
        "canvas_metal_40k_opaque_edges",
        OPAQUE_FRAME_EDGES,
        "200 opaque rings of 200 edges (40,000 edges)",
    );
}

// ---------------------------------------------------------------------------------------
// bug-687: polygons over black, where one coverage step is several sRGB levels.
//
// The shader and the software oracle quantise coverage identically
// (`int(clamp(0.5 - d, 0, 1) * 255 + 0.5)`), so they agree on a pixel exactly when they
// agree on its distance to within the gap to the nearest quantisation boundary. A
// distance evaluated in fp32 misses that on a few pixels of every long edge, and near
// black one step of coverage moves a channel by up to 9-13 levels, past `GPU_DEFAULT`'s
// 2. Every scene here draws on the default opaque-black surface for that reason. Shared
// with `scripts/test-canvas-gpu-rows.sh`, which draws them on Vulkan.
// ---------------------------------------------------------------------------------------

/// The star polygon {251/97}: 251 long chords (about 300 px each). bug-687 measured
/// max delta 4 at `42cea38df`, under the old 256-edge cap.
const STAR_251_97: &str = include_str!("scenes/star_251_97_on_black.mfb");

#[test]
fn a_long_chord_star_over_black_matches_the_software_oracle() {
    assert_metal_draws_like_software(
        "canvas_metal_star_251_97",
        STAR_251_97,
        "the {251/97} star polygon over black",
    );
}

/// The star polygon {301/97}: past the old 256-edge cap, and the most edge length of
/// the stars. bug-687 measured max delta 9.
const STAR_301_97: &str = include_str!("scenes/star_301_97_on_black.mfb");

#[test]
fn a_three_hundred_one_point_long_chord_star_over_black_matches_the_software_oracle() {
    assert_metal_draws_like_software(
        "canvas_metal_star_301_97",
        STAR_301_97,
        "the {301/97} star polygon over black",
    );
}

/// The star polygon {301/2}: short edges (about 7.5 px each). bug-687 measured max
/// delta 3.
const STAR_301_2: &str = include_str!("scenes/star_301_2_on_black.mfb");

#[test]
fn a_short_edge_star_over_black_matches_the_software_oracle() {
    assert_metal_draws_like_software(
        "canvas_metal_star_301_2",
        STAR_301_2,
        "the {301/2} star polygon over black",
    );
}

/// plan-163-A: the {301/2} star under `NonZero`. It winds twice round its centre, so the
/// disc even-odd leaves empty is filled — the GPUs' signed winding count, over hundreds
/// of short edges, against the oracle's.
const STAR_301_2_NONZERO: &str = include_str!("scenes/star_301_2_nonzero_on_black.mfb");

#[test]
fn a_short_edge_star_filled_non_zero_matches_the_software_oracle() {
    assert_metal_draws_like_software(
        "canvas_metal_star_301_2_nonzero",
        STAR_301_2_NONZERO,
        "the {301/2} star polygon filled NonZero over black",
    );
}

/// plan-163-A: both fill rules where they differ — a ring with a hole wound each way, a
/// {5/2} pentagram, a stroked translucent three-ring item and a transformed one — at
/// fractional coordinates, so the rims are antialiased under each rule.
const FILL_RULES: &str = include_str!("scenes/fill_rules_on_black.mfb");

#[test]
fn both_fill_rules_match_the_software_oracle_on_metal() {
    assert_metal_draws_like_software(
        "canvas_metal_fill_rules",
        FILL_RULES,
        "the NonZero and EvenOdd ring scene over black",
    );
}

/// plan-163-B: Square caps on lines (several angles and a zero-length one), arcs and
/// polylines, at fractional coordinates, plus a transformed line.
const SQUARE_CAPS: &str = include_str!("scenes/square_caps_on_black.mfb");

#[test]
fn square_caps_match_the_software_oracle_on_metal() {
    assert_metal_draws_like_software(
        "canvas_metal_square_caps",
        SQUARE_CAPS,
        "the Square-capped lines, arcs and polylines over black",
    );
}

/// plan-163-B: Miter and Bevel joins on polygons (a pentagram at both miter limits, a
/// concave arrow, two rings), polylines (sharp turns, a repeated point, per-point colours),
/// rectangles, a rotated polygon and a translucent stroke over a fill.
const JOINS: &str = include_str!("scenes/joins_on_black.mfb");

#[test]
fn joins_match_the_software_oracle_on_metal() {
    assert_metal_draws_like_software(
        "canvas_metal_joins",
        JOINS,
        "the mitered and beveled polygons, polylines and rectangles over black",
    );
}

/// plan-163-D: gradients in the item's own space -- turned by their own transform and by
/// the item's, the three spreads, a 2:1 radial, a focus and a focal disc, the ramp painting
/// strokes under three stroke colours and a Line, and two stroked gradients under
/// Multiply.
const GRADIENTS: &str = include_str!("scenes/gradients_on_black.mfb");

#[test]
fn gradients_match_the_software_oracle_on_metal() {
    assert_metal_draws_like_software(
        "canvas_metal_gradients",
        GRADIENTS,
        "the gradients over black",
    );
}

/// plan-163-C: path clips -- a triangle, a ring with a hole under each rule, rotated,
/// sheared and grouped items clipped in their own coordinates, a rectangle clip by either
/// path, and a dashed stroke clipped by a triangle.
const PATH_CLIPS: &str = include_str!("scenes/path_clips_on_black.mfb");

#[test]
fn path_clips_match_the_software_oracle_on_metal() {
    assert_metal_draws_like_software(
        "canvas_metal_path_clips",
        PATH_CLIPS,
        "the path-clipped shapes over black",
    );
}

/// plan-172-E: a rectangle's corners -- the exact square and rounded records beside the
/// flattened outline of per-corner and elliptical corners, filled, stroked and dashed,
/// and `Center`- and `TopLeft`-bound rectangles under a turn.
const CORNERS: &str = include_str!("scenes/rect_corners_on_black.mfb");

#[test]
fn corners_match_the_software_oracle_on_metal() {
    assert_metal_draws_like_software(
        "canvas_metal_corners",
        CORNERS,
        "the rectangle corners over black",
    );
}

/// bug-995: path clips on transformed items and on items inside groups with a matrix
/// (`scenes/transformed_clips_on_black.mfb`) -- the five-sample clip path, at scales 0.1,
/// 1 and 4, under a shear, banded and unbanded, with a rectangle clip under a group
/// matrix. A pixel whose centre clip sample is past the clip's reach is answered from
/// that sample alone; this row holds that shortcut to the oracle's every pixel.
const TRANSFORMED_CLIPS: &str = include_str!("scenes/transformed_clips_on_black.mfb");

#[test]
fn transformed_path_clips_match_the_software_oracle_on_metal() {
    assert_metal_draws_like_software(
        "canvas_metal_transformed_clips",
        TRANSFORMED_CLIPS,
        "path clips under item transforms and group matrices",
    );
}

/// plan-172-L: pictures inside shapes (`scenes/picture_shapes_on_black.mfb`). Every
/// shaped picture is a picture record with a polygonal clip and, where it strokes, the
/// shape itself -- two entries the GPU already draws -- so this is the check that the
/// two walks lay the same entries out: a top-level picture that is two entries is
/// resolved by the draw walk, not the native layout.
const PICTURE_SHAPES: &str = include_str!("scenes/picture_shapes_on_black.mfb");

#[test]
fn picture_shapes_match_the_software_oracle_on_metal() {
    assert_metal_draws_like_software(
        "canvas_metal_picture_shapes",
        PICTURE_SHAPES,
        "pictures inside shapes over black",
    );
}

/// bug-1079: plain pictures whose texel boundaries are pixel centres, in x and in y
/// (`scenes/picture_texel_edge.mfb`). The texel there used to hang on the last bit of
/// the backend's division; Apple's GPUs divide exactly, so this row passed before the
/// index was made exact and holds Metal to the same frame after.
const PICTURE_TEXEL_EDGE: &str = include_str!("scenes/picture_texel_edge.mfb");

#[test]
fn texel_boundaries_on_pixel_centres_match_the_software_oracle_on_metal() {
    assert_metal_draws_like_software(
        "canvas_metal_texel_edge",
        PICTURE_TEXEL_EDGE,
        "pictures with texel boundaries on pixel centres",
    );
}

/// The same scene with the pointer on its `AutoPaint` picture, the circle about
/// (770, 470): both of the picture's entries are swapped for the hover paint's -- the
/// image under an orange tint, the outline 6 px of red -- and the GPU draw list must
/// name those two records, not the ones the walk appended.
///
/// (818, 470) is 48.5 px from the centre of a circle of radius 48: on the hover
/// outline, which covers 45 to 51, and red. At rest the circle's own 2 px outline
/// there is white.
#[test]
fn a_hovered_autopaint_picture_matches_the_software_oracle_on_metal() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let program = build("canvas_metal_picture_auto", PICTURE_SHAPES);
    let pixel = |frame: &Frame, x: usize, y: usize| {
        let at = (y * WIDTH as usize + x) * 4;
        [frame.pixels[at], frame.pixels[at + 1], frame.pixels[at + 2]]
    };
    let envs = [("MFB_CANVAS_POINTER", "0,770,470,0,1")];
    let (software, software_stats) = render_env(&program, false, "sw", &envs);
    let (gpu, stats) = render_env(&program, true, "gpu", &envs);
    if !metal_built(&stats) {
        return; // no Metal device on this host (§metal_built)
    }
    assert!(
        software_stats.contains(" auto=3"),
        "the software run did not hover the picture: {software_stats}"
    );
    assert!(
        !stats.contains("gpuFrames=0"),
        "the hovered scene fell back to the software renderer: {stats}"
    );
    if let Err(diff) = compare_within_tolerance(&gpu, &software, Tolerance::GPU_DEFAULT) {
        panic!("Metal draws the hovered picture differently from the software oracle: {diff}");
    }
    let (rest, _) = render(&program, false, "rest");
    for (name, frame) in [("software", &software), ("Metal", &gpu)] {
        let outline = pixel(frame, 818, 470);
        assert!(
            outline[0] > 200 && outline[1] < 80 && outline[2] < 80,
            "the hover outline is {outline:?} on {name}"
        );
        assert_ne!(
            pixel(frame, 770, 470),
            pixel(&rest, 770, 470),
            "the hover tint did not change the image on {name}"
        );
    }
}

/// plan-163-B: dashed strokes -- polygons whose dashes cross their corners under each cap
/// and under Miter and Bevel joins, an odd pattern with an offset, dashed polylines and
/// lines, a circle, a rotated ellipse, a rounded rectangle, an arc, a rectangle and a
/// rotated polygon.
const DASHES: &str = include_str!("scenes/dashes_on_black.mfb");

#[test]
fn dashes_match_the_software_oracle_on_metal() {
    assert_metal_draws_like_software(
        "canvas_metal_dashes",
        DASHES,
        "the dashed polygons, polylines, lines, curves and rectangles over black",
    );
}

/// `LARGE_POLYGONS` without its grey ground: filled, translucent, stroked, transformed
/// and self-intersecting polygons of 300-4,000 edges, every rim over black. bug-688
/// measured max delta 4 on Vulkan (lavapipe) at (736, 52), the rotated ring's rim.
const LARGE_POLYGONS_ON_BLACK: &str = include_str!("scenes/large_polygons_on_black.mfb");

#[test]
fn polygons_past_two_hundred_fifty_six_edges_over_black_match_the_software_oracle() {
    assert_metal_draws_like_software(
        "canvas_metal_large_polygons_on_black",
        LARGE_POLYGONS_ON_BLACK,
        "filled, stroked, transformed and self-intersecting polygons over black",
    );
}

/// 140 small filled polygons over black (bug-687): every rim pixel of every ring is near
/// black. Before the double-float polygon path Metal put 3 pixels past `GPU_DEFAULT`
/// here, worst 4.
const SMALL_POLYGONS_ON_BLACK: &str = include_str!("scenes/small_polygons_on_black.mfb");

#[test]
fn small_polygons_over_black_match_the_software_oracle() {
    assert_metal_draws_like_software(
        "canvas_metal_small_polygons_on_black",
        SMALL_POLYGONS_ON_BLACK,
        "140 small filled polygons over black",
    );
}

/// 140 rectangles over black (bug-687): a guard. Rectangles keep their fp32 distance and
/// measured worst 2 on Metal and on Vulkan -- inside `GPU_DEFAULT`, only just.
const RECTANGLES_ON_BLACK: &str = include_str!("scenes/rectangles_on_black.mfb");

#[test]
fn rectangles_over_black_match_the_software_oracle() {
    assert_metal_draws_like_software(
        "canvas_metal_rectangles_on_black",
        RECTANGLES_ON_BLACK,
        "140 filled rectangles over black",
    );
}

// ---------------------------------------------------------------------------------------
// bug-735: every other shape kind over black. bug-687 fixed the filled polygon; these rows
// hold the rest -- every distance input carried at double-float precision and every
// distance evaluated in double-float, in both shaders. Shared with
// `scripts/test-canvas-gpu-rows.sh`, which draws them on Vulkan.
// ---------------------------------------------------------------------------------------

/// 140 filled circles over black (bug-735). Metal put 2 pixels past `GPU_DEFAULT` here before
/// bug-735, worst 6.
const CIRCLES_ON_BLACK: &str = include_str!("scenes/circles_on_black.mfb");

#[test]
fn circles_over_black_match_the_software_oracle() {
    assert_metal_draws_like_software(
        "canvas_metal_circles_on_black",
        CIRCLES_ON_BLACK,
        "140 filled circles over black",
    );
}

/// 140 filled rotated ellipses over black (bug-735): the 24-step bisection in fp32 took a
/// different branch than the oracle's f64 one. 8 pixels past `GPU_DEFAULT` before, worst 9.
const ELLIPSES_ON_BLACK: &str = include_str!("scenes/ellipses_on_black.mfb");

#[test]
fn rotated_ellipses_over_black_match_the_software_oracle() {
    assert_metal_draws_like_software(
        "canvas_metal_ellipses_on_black",
        ELLIPSES_ON_BLACK,
        "140 filled rotated ellipses over black",
    );
}

/// 140 round-capped 2-px lines over black (bug-735). 1 pixel past `GPU_DEFAULT` before, worst 3.
const LINES_ON_BLACK: &str = include_str!("scenes/lines_on_black.mfb");

#[test]
fn round_capped_lines_over_black_match_the_software_oracle() {
    assert_metal_draws_like_software(
        "canvas_metal_lines_on_black",
        LINES_ON_BLACK,
        "140 round-capped lines over black",
    );
}

/// 140 butt-capped 2-px arcs over black (bug-735). 3 pixels past `GPU_DEFAULT` before, worst 9.
const ARCS_ON_BLACK: &str = include_str!("scenes/arcs_on_black.mfb");

#[test]
fn butt_capped_arcs_over_black_match_the_software_oracle() {
    assert_metal_draws_like_software(
        "canvas_metal_arcs_on_black",
        ARCS_ON_BLACK,
        "140 butt-capped arcs over black",
    );
}

/// 140 stroked polygons over black whose half-width is off the 16.16 grid (bug-735): the
/// half-width reached the shader as 16.16. 1 pixel past `GPU_DEFAULT` before, worst 3.
const STROKED_POLYGONS_ON_BLACK: &str = include_str!("scenes/stroked_polygons_on_black.mfb");

#[test]
fn polygon_strokes_off_the_fixed_point_grid_over_black_match_the_software_oracle() {
    assert_metal_draws_like_software(
        "canvas_metal_stroked_polygons_on_black",
        STROKED_POLYGONS_ON_BLACK,
        "140 stroked polygons over black",
    );
}

/// 140 round-capped 40-point polylines over black (bug-735). 2 pixels past `GPU_DEFAULT`
/// before, worst 6.
const POLYLINES_ON_BLACK: &str = include_str!("scenes/polylines_on_black.mfb");

#[test]
fn polylines_over_black_match_the_software_oracle() {
    assert_metal_draws_like_software(
        "canvas_metal_polylines_on_black",
        POLYLINES_ON_BLACK,
        "140 round-capped polylines over black",
    );
}

/// 140 filled polygons over black, each in a group drawn at an offset off the 16.16 grid
/// (bug-735): the draw offset reached the shader as 16.16. 4 pixels past `GPU_DEFAULT`
/// before, worst 4.
const GROUPED_POLYGONS_ON_BLACK: &str = include_str!("scenes/grouped_polygons_on_black.mfb");

#[test]
fn polygons_in_groups_at_off_grid_offsets_over_black_match_the_software_oracle() {
    assert_metal_draws_like_software(
        "canvas_metal_grouped_polygons_on_black",
        GROUPED_POLYGONS_ON_BLACK,
        "140 grouped polygons over black",
    );
}

/// Butt-capped lines, round-capped arcs, filled-and-stroked circles, ellipses and rounded
/// rectangles, and butt-capped polylines over black, every stroke width off the 16.16 grid
/// (bug-735). 5 pixels past `GPU_DEFAULT` before, worst 12.
const STROKED_SHAPES_ON_BLACK: &str = include_str!("scenes/stroked_shapes_on_black.mfb");

#[test]
fn stroked_and_capped_shapes_over_black_match_the_software_oracle() {
    assert_metal_draws_like_software(
        "canvas_metal_stroked_shapes_on_black",
        STROKED_SHAPES_ON_BLACK,
        "butt lines, round arcs, stroked closed shapes and butt polylines over black",
    );
}

/// 140 rectangles over black whose every edge sits a few 1e-6 px from a coverage step
/// (bug-735). `rectangles_on_black` passes at worst 2 only because its edges happen to
/// land away from one; these do not, and the 16.16 narrowing alone moved 3,623 pixels
/// past `GPU_DEFAULT` before (worst 12).
const RECTANGLES_AT_A_STEP_ON_BLACK: &str =
    include_str!("scenes/rectangles_at_a_step_on_black.mfb");

#[test]
fn rectangles_with_edges_at_a_coverage_step_over_black_match_the_software_oracle() {
    assert_metal_draws_like_software(
        "canvas_metal_rectangles_at_a_step",
        RECTANGLES_AT_A_STEP_ON_BLACK,
        "rectangles with every edge beside a coverage step",
    );
}

/// The same edges as `rectangles_at_a_step_on_black`, but as a `Paint.clip` (bug-735):
/// the clip's own coverage is the same rectangle distance. 4,684 pixels past
/// `GPU_DEFAULT` before, worst 12.
const CLIPS_AT_A_STEP_ON_BLACK: &str = include_str!("scenes/clips_at_a_step_on_black.mfb");

#[test]
fn clips_with_edges_at_a_coverage_step_over_black_match_the_software_oracle() {
    assert_metal_draws_like_software(
        "canvas_metal_clips_at_a_step",
        CLIPS_AT_A_STEP_ON_BLACK,
        "clip rectangles with every edge beside a coverage step",
    );
}

/// The items of `stroked_shapes_on_black`, each under a rotation and scale about its
/// centre, every other one clipped (bug-735): the five-sample transformed path and the
/// clip in double-float. 5 pixels past `GPU_DEFAULT` before, worst 9.
const TRANSFORMED_SHAPES_ON_BLACK: &str = include_str!("scenes/transformed_shapes_on_black.mfb");

#[test]
fn transformed_and_clipped_shapes_over_black_match_the_software_oracle() {
    assert_metal_draws_like_software(
        "canvas_metal_transformed_shapes_on_black",
        TRANSFORMED_SHAPES_ON_BLACK,
        "transformed, clipped, stroked and capped shapes over black",
    );
}

/// A Multiply-blended filled rectangle stroked 1e-5 px wide, then two opaque items
/// (bug-735). The draw list splits it into two records on the f64 `half > 0`; before
/// bug-735 both emitters split on the 16.16 word, 0 here, and published one, so every
/// later draw named the wrong block: 20,632 pixels wrong on Metal, worst 101.
const BLENDED_HAIRLINE_STROKE: &str = include_str!("scenes/blended_hairline_stroke.mfb");

#[test]
fn a_blended_item_stroked_under_the_fixed_point_step_matches_the_software_oracle() {
    assert_metal_draws_like_software(
        "canvas_metal_blended_hairline_stroke",
        BLENDED_HAIRLINE_STROKE,
        "a blended rectangle with a 1e-5 px stroke, then two opaque items",
    );
}

/// plan-172-K: `canvas::AutoPaint` on the GPU. The scene's own comment says what each
/// pointer position puts under test.
const AUTOPAINT: &str = include_str!("scenes/autopaint_on_black.mfb");

/// A hovered `AutoPaint` draws on Metal as the software oracle draws it — for a button
/// in the scene itself, and for one use of a group that is drawn twice.
///
/// The state is the graphics thread's, resolved after the walk, and the two renderers
/// read it by different routes: software from the frame's draw entries, Metal from the
/// GPU draw list, which is laid out from `__CANVAS_TOP_OFFSETS` and, for a group, from
/// a run shared by every node that draws the group. So a hover that reached one route
/// and not the other is a wrong frame on one backend only, and this is where it shows.
///
/// Each case also checks the Metal frame itself — the hovered button is the hover
/// paint's orange and the other use of the group is still blue — because two frames
/// that agree on NO hover agree within tolerance too.
#[test]
fn a_hovered_autopaint_matches_the_software_oracle_on_metal() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let program = build("canvas_metal_autopaint", AUTOPAINT);
    std::fs::write(
        program.project.join("fixture.ttf"),
        common::fixture_truetype(),
    )
    .expect("write the font fixture");
    let pixel = |frame: &Frame, x: usize, y: usize| {
        let at = (y * WIDTH as usize + x) * 4;
        [frame.pixels[at], frame.pixels[at + 1], frame.pixels[at + 2]]
    };
    let lit = |rgb: [u8; 3]| rgb[0] > 200 && rgb[2] < 120;
    let unlit = |rgb: [u8; 3]| rgb[0] < 100 && rgb[2] > 150;
    for (what, pointer, hovered, other) in [
        // (280, 120) is on the scene's own button; (150, 320) is the group's first use.
        (
            "the scene's own button",
            "0,280,120,0,1",
            (280, 120),
            (150, 320),
        ),
        // (150, 320) is the group's first use; (550, 320) its second.
        (
            "one use of a group",
            "0,150,320,0,1",
            (150, 320),
            (550, 320),
        ),
    ] {
        let envs = [("MFB_CANVAS_POINTER", pointer)];
        let (software, software_stats) = render_env(&program, false, "sw", &envs);
        let (gpu, stats) = render_env(&program, true, "gpu", &envs);
        if !metal_built(&stats) {
            return; // no Metal device on this host (§metal_built)
        }
        assert!(
            software_stats.contains(" auto=300") || software_stats.contains(" auto=030"),
            "{what}: the software run did not hover exactly one button: {software_stats}"
        );
        assert!(
            !stats.contains("gpuFrames=0"),
            "{what}: the hovered scene fell back to the software renderer: {stats}"
        );
        if let Err(diff) = compare_within_tolerance(&gpu, &software, Tolerance::GPU_DEFAULT) {
            panic!("Metal draws {what}, hovered, differently from the software oracle: {diff}");
        }
        for (name, frame) in [("software", &software), ("Metal", &gpu)] {
            let on = pixel(frame, hovered.0, hovered.1);
            let off = pixel(frame, other.0, other.1);
            assert!(lit(on), "{what}: the hovered button is {on:?} on {name}");
            assert!(unlit(off), "{what}: the other button is {off:?} on {name}");
        }
    }
}

// ---------------------------------------------------------------------------------------
// plan-164-C: the curve shapes on Metal. Each row requires that the frame was drawn on
// the GPU and agrees with the software oracle within `Tolerance::GPU_CURVE`, the
// tolerance measured for a frame that draws a curve (`tests/common/canvas_image.rs`).
// The same scene files are `scripts/test-canvas-gpu-rows.sh`'s `curve*` rows on Vulkan.
// ---------------------------------------------------------------------------------------

/// A rounded star, a dashed spiral, an `AutoPaint` pill and a picture cut to a blob.
const CURVES: &str = include_str!("scenes/curves.mfb");

#[test]
fn the_curve_shapes_match_the_software_oracle_on_metal() {
    assert_metal_draws_within(
        "canvas_metal_curves",
        CURVES,
        "a CurvePolygon, a dashed PolyCurve, an AutoPaint over one and a Picture in one",
        Tolerance::GPU_CURVE,
        &[],
    );
}

/// The same scene with the pointer on the pill: an `AutoPaint` over a `CurvePolygon`
/// in its hover paint. The pointer is answered on the worker, in software, and what
/// reaches the GPU is the curve record under another paint.
#[test]
fn a_hovered_autopaint_over_a_curve_matches_the_software_oracle_on_metal() {
    assert_metal_draws_within(
        "canvas_metal_curves_hover",
        CURVES,
        "an AutoPaint over a CurvePolygon, hovered",
        Tolerance::GPU_CURVE,
        &[("MFB_CANVAS_POINTER", "0,200,440,0,1")],
    );
}

/// Every cap, join, dash and fill rule, a stroke a gradient runs along and a
/// half-transparent fill under a radial one.
const CURVE_STROKES: &str = include_str!("scenes/curve_strokes_on_black.mfb");

#[test]
fn every_curve_stroke_and_fill_matches_the_software_oracle_on_metal() {
    assert_metal_draws_within(
        "canvas_metal_curve_strokes",
        CURVE_STROKES,
        "curve shapes under every cap, join, dash, fill rule and gradient",
        Tolerance::GPU_CURVE,
        &[],
    );
}

/// Each shape under its own transform, and both inside a group drawn twice, turned and
/// scaled unevenly: the pieces are read at a point that has been through the draw
/// entry's inverse and the item's, and an arm that skipped one shows here.
const CURVES_PLACED: &str = include_str!("scenes/curves_placed_on_black.mfb");

#[test]
fn turned_and_grouped_curves_match_the_software_oracle_on_metal() {
    assert_metal_draws_within(
        "canvas_metal_curves_placed",
        CURVES_PLACED,
        "curve shapes under their own transforms and inside turned, scaled groups",
        Tolerance::GPU_CURVE,
        &[],
    );
}

/// Pictures in curved shapes, in both `PictureClip` modes and three fits, stroked,
/// dashed and unstroked, and one inside a turned group. The image is clipped to the
/// flattened outline (edge slots); the outline is the curve (piece slots).
const CURVE_PICTURES: &str = include_str!("scenes/curve_pictures_on_black.mfb");

#[test]
fn pictures_in_curved_shapes_match_the_software_oracle_on_metal() {
    assert_metal_draws_within(
        "canvas_metal_curve_pictures",
        CURVE_PICTURES,
        "pictures inside CurvePolygons",
        Tolerance::GPU_CURVE,
        &[],
    );
}

/// Shapes a few thousandths of a unit across under a 30,000-fold transform: the 16.16
/// word of a piece's coordinate places it to about half a pixel there, so the frame
/// agrees with the oracle only if every piece's residual words -- chords and control
/// points alike -- reached the buffer in their own slots. With the residuals zeroed in
/// the emitter this row measured a worst delta of 127 over 3,759 pixels; as shipped, 1.
const CURVE_RESIDUALS: &str = include_str!("scenes/curve_residuals_on_black.mfb");

#[test]
fn the_pieces_reach_the_gpu_buffer_with_their_residuals() {
    assert_metal_draws_within(
        "canvas_metal_curve_residuals",
        CURVE_RESIDUALS,
        "tiny curve shapes under a 30,000-fold transform",
        Tolerance::GPU_CURVE,
        &[],
    );
}

/// Polygons with a vertex exactly on a pixel-centre row. The double-float crossing test
/// rebuilt an edge's end as `a + (b - a)`, so the two edges sharing such a vertex could
/// disagree about which side of the row it is on, and the row came out inverted: before
/// the fix this scene differed from the oracle by a whole row of two of its eighteen
/// polygons (384 pixels, delta 255). A POLYGON bug, older than the curve shapes that
/// found it, so it is held to `GPU_DEFAULT`.
const VERTEX_ON_A_ROW: &str = include_str!("scenes/vertex_on_a_pixel_row.mfb");

#[test]
fn a_polygon_vertex_on_a_pixel_row_is_counted_once_on_metal() {
    assert_metal_draws_like_software(
        "canvas_metal_vertex_row",
        VERTEX_ON_A_ROW,
        "polygons with a vertex on a pixel-centre row",
    );
}

/// `@RINGS@` rings, each a `CurvePolygon` whose one contour is 256 straight curves --
/// 256 pieces, so 512 slots of the frame's edge region: a piece takes TWO, its chord
/// and its control point with the weight.
const PIECE_RINGS: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color
IMPORT collections
IMPORT math

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  LET none AS canvas::Transform = canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
  MUT scene AS List OF canvas::DrawItem = [canvas::Rectangle[id := "", position := vector::Float2[x := 10.0, y := 10.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 50.0, height := 50.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := none]]
  MUT ring AS Integer = 0
  WHILE ring < @RINGS@
    LET cx AS Float = 20.0 + toFloat((ring MOD 70) * 12)
    LET cy AS Float = 80.0 + toFloat((ring / 70) * 12)
    MUT sides AS List OF canvas::BezierCurve = []
    MUT i AS Integer = 0
    WHILE i < 256
      LET a AS Float = toFloat(i) * 6.283185307179586 / 256.0
      LET b AS Float = toFloat(i + 1) * 6.283185307179586 / 256.0
      sides = collections::append(sides, canvas::BezierCurve[points := [vector::Float2[x := 4.0 * math::cos(a), y := 4.0 * math::sin(a)], vector::Float2[x := 4.0 * math::cos(b), y := 4.0 * math::sin(b)]], weights := []])
      i = i + 1
    END WHILE
    LET item AS canvas::DrawItem = canvas::CurvePolygon[id := "", position := vector::Float2[x := cx, y := cy], contours := [sides], rule := canvas::FillRule.EvenOdd, paint := canvas::fill(color::rgb(0, 200, 255)), transform := none]
    scene = collections::append(scene, item)
    ring = ring + 1
  END WHILE
  canvas::present(scene)
END FUNC
"#;

/// A frame's curve pieces are counted against the edge region, two slots a piece
/// (plan-164-C), and a frame past it is a second pass's (bug-765).
///
/// 512 rings is 262,144 slots, the region exactly: one pass. 513 is 262,656: two. The
/// pair is what pins the factor -- a count of ONE slot a piece would put 513 rings in
/// one pass, and the emitter would then leave the rings past the region undrawn, which
/// the comparison with the oracle sees. Until bug-765 the 513-ring frame was declined
/// to software instead; the owner's ruling of 2026-10-12 (the bug document's Cause
/// note) made the region the size of a pass, not of a frame.
#[test]
fn curve_pieces_are_counted_two_slots_each_against_the_edge_region() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let fits = build(
        "canvas_metal_piece_cap_fits",
        &PIECE_RINGS.replace("@RINGS@", "512"),
    );
    let (software, _) = render(&fits, false, "sw");
    let (gpu, stats) = render(&fits, true, "gpu");
    if !metal_built(&stats) {
        return;
    }
    assert!(
        !stats.contains("gpuFrames=0"),
        "131,072 pieces is exactly METAL_MAX_FRAME_EDGES slots, but Metal declined: {stats}"
    );
    if let Err(diff) = compare_within_tolerance(&gpu, &software, Tolerance::GPU_CURVE) {
        panic!("Metal draws 512 rings of 256 pieces differently from the oracle: {diff}");
    }
    let past = build(
        "canvas_metal_piece_cap_past",
        &PIECE_RINGS.replace("@RINGS@", "513"),
    );
    let (software, _) = render(&past, false, "sw");
    let (gpu, stats) = render(&past, true, "gpu");
    assert!(
        software.pixels.iter().any(|&b| b != 0),
        "the software render drew nothing, so the comparison would be vacuous",
    );
    assert!(
        !stats.contains("gpuFrames=0"),
        "131,328 pieces is 262,656 slots, two passes of METAL_MAX_FRAME_EDGES, but the \
         software renderer drew the frame: {stats}"
    );
    if let Err(diff) = compare_within_tolerance(&gpu, &software, Tolerance::GPU_CURVE) {
        panic!("Metal draws 513 rings of 256 pieces differently from the oracle: {diff}");
    }
}

// ---------------------------------------------------------------------------------------
// bug-765: no scene size is a limit. A frame whose draw list does not fit one frame
// buffer is drawn in passes -- consecutive runs of the list, each the longest that fits
// every region of the buffer, into one target -- and presented or read back once.
// ---------------------------------------------------------------------------------------

/// `MFB_TEST_ITEMS` squares, most opaque, every ninety-seventh translucent over its
/// neighbours, wrapping over the squares before them past 63,900.
const MANY_ITEMS: &str = include_str!("scenes/many_items.mfb");

/// A frame just under one buffer's item region, exactly at it, one item over it and
/// several passes long draws on Metal and matches the oracle.
///
/// 65,536 items is the region (`METAL_MAX_FRAME_ITEMS`). 65,537 is two passes, the
/// second of ONE item: a pass that cleared the target instead of loading it would leave
/// that square alone on black, and one that drew before the first had finished would
/// read the first pass's blocks. 140,000 is three passes. The translucent squares lie
/// over their neighbours and over the squares of the lap before, so a square drawn out
/// of order, or twice, is another colour.
#[test]
fn a_frame_past_one_buffers_items_draws_in_passes_on_metal() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let program = build("canvas_metal_many_items", MANY_ITEMS);
    for count in ["65535", "65536", "65537", "140000"] {
        let envs = [("MFB_TEST_ITEMS", count)];
        let (software, _) = render_env(&program, false, &format!("sw{count}"), &envs);
        let (gpu, stats) = render_env(&program, true, &format!("gpu{count}"), &envs);
        if !metal_built(&stats) {
            return;
        }
        assert!(
            software.pixels.chunks(4).any(|p| p != [0, 0, 0, 255]),
            "the software render of {count} items drew nothing",
        );
        assert!(
            stats.contains(&format!("blocks={count} ")),
            "the frame must hold {count} blocks: {stats}"
        );
        assert!(
            !stats.contains("gpuFrames=0"),
            "a frame of {count} items was drawn by the software renderer, not Metal: {stats}"
        );
        if let Err(diff) = compare_within_tolerance(&gpu, &software, Tolerance::GPU_DEFAULT) {
            panic!("Metal draws {count} items differently from the software oracle: {diff}");
        }
    }
}

/// 44,000 three-stop gradients: 132,000 stops, past the gradient region, in fewer
/// items than the item region holds.
const MANY_GRADIENT_STOPS: &str = include_str!("scenes/many_gradient_stops.mfb");

/// A frame that fits every region but the gradient stops' is split where THAT region
/// fills. A boundary found from the item count alone would put all 132,000 stops in one
/// pass, and the stops past the region would be another item's.
#[test]
fn a_frame_past_one_buffers_gradient_stops_draws_in_passes_on_metal() {
    assert_metal_draws_like_software(
        "canvas_metal_many_stops",
        MANY_GRADIENT_STOPS,
        "44,000 three-stop gradients (132,000 stops)",
    );
}

/// Four distinct 1,920 by 1,200 images: 9,216,000 texels, past the texel region.
const MANY_TEXELS: &str = include_str!("scenes/many_texels.mfb");

/// A frame whose pictures' texels pass one buffer's glyph and picture region is split
/// on texels, and the table that points a second picture of one block at the texels
/// already copied starts empty in each pass.
#[test]
fn a_frame_past_one_buffers_texels_draws_in_passes_on_metal() {
    assert_metal_draws_like_software(
        "canvas_metal_many_texels",
        MANY_TEXELS,
        "four overlapping 1,920 by 1,200 pictures (9,216,000 texels)",
    );
}

/// A group of 500 squares drawn three times around 70,000 squares of the scene, then
/// Multiply items that both fill and stroke.
const PASSES_GROUPED: &str = include_str!("scenes/passes_grouped.mfb");

/// A pass is a run of the frame's DRAW ENTRIES, not of its block list.
///
/// A group's blocks are recorded once, at its first use, and named again by every later
/// use. Here the third use falls in the second pass while the blocks' place in the
/// block list is the first pass's: a walk of the block list would draw that use in the
/// first pass, under 30,000 squares that belong beneath it, or not at all. The second
/// use is in the pass that already holds the blocks and is drawn from them. The scene's
/// own run of squares is cut in two by the end of the first pass, and the pass ends
/// with entries under another pipeline whose blocks are two records each.
#[test]
fn a_group_drawn_again_in_a_later_pass_is_drawn_in_paint_order_on_metal() {
    assert_metal_draws_like_software(
        "canvas_metal_passes_grouped",
        PASSES_GROUPED,
        "a group drawn in two passes of one frame",
    );
}
