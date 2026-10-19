//! plan-176-D: `rt_canvas_metal`'s paths, pictures and text spacing, the large scenes,
//! the curves, and the frames that draw in more than one buffer pass. The build/render
//! helpers and `assert_metal_draws_like_software` live in the root, `rt_canvas_metal.rs`.

use super::*;

/// Many polygons that individually fit but together overflow the frame's edge region.
///
/// plan-116-A declined this scene: 40,000 edges was past the 16,384-edge frame region
/// that letter introduced. bug-686 raised the region to 262,144 edges, and the scene is
/// now a Metal acceptance case (`forty_thousand_polygon_edges_draw_on_metal`). The
/// decline it used to prove was `PAST_THE_FRAME_EDGE_REGION`'s job until bug-765, which
/// made that scene two passes; `TOO_MANY_EDGES` is the decline that is left.
const TOO_MANY_FRAME_EDGES: &str = include_str!("../scenes/forty_thousand_edges.mfb");

/// Polygons whose edges together pass Metal's frame edge region
/// (`METAL_MAX_FRAME_EDGES` = 262,144 since bug-686).
///
/// 1,025 rings of 256 edges is 262,400 edges — 256 past the region, and no ring past
/// any per-polygon limit, so only the *sum* can end a pass. The rings are 4 px across so
/// the software oracle, which walks every edge for every pixel of a polygon's bounds,
/// draws the frame in seconds rather than minutes. The rectangle keeps the frame from
/// being blank.
const PAST_THE_FRAME_EDGE_REGION: &str = include_str!("../scenes/past_the_edge_region.mfb");

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
  LET label AS canvas::DrawItem = canvas::Text[id := "", position := vector::Float2[x := 10.0, y := 240.0], text := "Pictures 01", fonts := [face], fontSize := 22.0, letterSpacing := 0.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
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
  LET words AS canvas::DrawItem = canvas::Text[id := "", position := vector::Float2[x := 20.0, y := 40.0], text := "A B C", fonts := [face], fontSize := 22.0, letterSpacing := 0.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
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

/// plan-150-A: the particle scene `rt_canvas_golden` holds a reference for — a firework
/// burst and a fountain, 320 live particles between them.
const PARTICLES: &str = include_str!("../scenes/particles_on_black.mfb");

/// plan-150-A: a system as a group's child, the group drawn through a turned, scaled
/// node and through a moved one.
const PARTICLES_GROUPED: &str = include_str!("../scenes/particles_grouped_on_black.mfb");

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
const FIVE_THOUSAND_QUADS: &str = include_str!("../scenes/five_thousand_quads.mfb");

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
const MANY_GRADIENTS: &str = include_str!("../scenes/many_gradients.mfb");

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
const TILEMAP: &str = include_str!("../scenes/tilemap.mfb");

const TILE: &str = include_str!("../scenes/tile.mfb");

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
const TILE_EDGE: &str = include_str!("../scenes/tile_edge.mfb");

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
const BACKGROUND: &str = include_str!("../scenes/background.mfb");

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
const LARGE_POLYGONS: &str = include_str!("../scenes/large_polygons.mfb");

/// A scene installed with `canvas::presentLayers`: two layers, one item each.
const LAYERED: &str = include_str!("../scenes/layered.mfb");

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
const HUGE_POLYGON: &str = include_str!("../scenes/huge_polygon.mfb");

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
const OPAQUE_FRAME_EDGES: &str = include_str!("../scenes/forty_thousand_opaque_edges.mfb");

#[test]
fn forty_thousand_opaque_polygon_edges_draw_on_metal() {
    assert_metal_draws_like_software(
        "canvas_metal_40k_opaque_edges",
        OPAQUE_FRAME_EDGES,
        "200 opaque rings of 200 edges (40,000 edges)",
    );
}

// ---------------------------------------------------------------------------------------
// plan-164-C: the curve shapes on Metal. Each row requires that the frame was drawn on
// the GPU and agrees with the software oracle within `Tolerance::GPU_CURVE`, the
// tolerance measured for a frame that draws a curve (`tests/common/canvas_image.rs`).
// The same scene files are `scripts/test-canvas-gpu-rows.sh`'s `curve*` rows on Vulkan.
// ---------------------------------------------------------------------------------------

/// A rounded star, a dashed spiral, an `AutoPaint` pill and a picture cut to a blob.
const CURVES: &str = include_str!("../scenes/curves.mfb");

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
const CURVE_STROKES: &str = include_str!("../scenes/curve_strokes_on_black.mfb");

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
const CURVES_PLACED: &str = include_str!("../scenes/curves_placed_on_black.mfb");

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
const CURVE_PICTURES: &str = include_str!("../scenes/curve_pictures_on_black.mfb");

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
const CURVE_RESIDUALS: &str = include_str!("../scenes/curve_residuals_on_black.mfb");

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
const VERTEX_ON_A_ROW: &str = include_str!("../scenes/vertex_on_a_pixel_row.mfb");

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
const MANY_ITEMS: &str = include_str!("../scenes/many_items.mfb");

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
const MANY_GRADIENT_STOPS: &str = include_str!("../scenes/many_gradient_stops.mfb");

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
const MANY_TEXELS: &str = include_str!("../scenes/many_texels.mfb");

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
const PASSES_GROUPED: &str = include_str!("../scenes/passes_grouped.mfb");

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
