//! plan-176-D: `rt_canvas_golden`'s ellipse, gradient and polyline goldens, software and
//! GPU. The render helpers and the `POLYLINES` scene (pinned for
//! `scripts/test-canvas-vulkan.sh`) live in the root, `rt_canvas_golden.rs`.

use super::*;

/// Axis-aligned, rotated, high-eccentricity, filled and stroked (plan-116-E).
///
/// The four cases the ellipse SDF has to get right, and each fails differently:
///
/// - **The axis-aligned pair** is the baseline. A 3:1 filled ellipse beside the circle
///   its `radiusX` would give, so the picture shows the shape rather than just an oval.
/// - **The high-eccentricity one** is where the solve is worst. The approximate ellipse
///   distance this letter rejected has its largest error at the *flat* ends, where the
///   curvature is lowest, so a 10:1 ellipse's rim is the thing to look at.
/// - **The rotated pair** exercises the stored `cos`/`sin`. The two are the same shape a
///   quarter turn apart, so a rotation applied with the wrong sign is visible as an
///   asymmetry between them rather than as "some angle".
/// - **The stroked one** is the signed-distance case: hollow inside, bounded outside.
///   An unsigned distance fills the interior and every filled case above still passes.
///
/// The equal-radii ellipse in the bottom row is drawn beside a real `Circle` of the
/// same radius and paint. In the reference they are indistinguishable, which is the
/// picture of §1's load-bearing claim — and if the `rx == ry` guard ever stopped
/// firing, this is where a reader would see the rim shift.
const ELLIPSES: &str = r#"IMPORT app
IMPORT math
IMPORT canvas
IMPORT vector
IMPORT color
IMPORT io

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)

  LET ground AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 900.0, height := 640.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(58, 62, 72)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]

  ' Row 1 -- axis-aligned 3:1, beside the circle its radiusX would give.
  LET warm AS color::Color = color::rgb(255, 196, 84)
  LET wide AS canvas::DrawItem = canvas::Ellipse[id := "", position := vector::Float2[x := 240.0, y := 100.0], radiusX := 180.0, radiusY := 60.0, paint := canvas::fill(warm), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET round AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 560.0, y := 100.0], radius := 60.0, paint := canvas::fill(warm), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]

  ' Row 2 -- 10:1, the eccentricity where an approximate distance is worst, and the
  ' same shape turned a quarter turn. The turned one gets its own column on the right,
  ' because a 10:1 ellipse a quarter turn round is as tall as the row is wide.
  LET cool AS color::Color = color::rgb(120, 214, 255)
  LET thin AS canvas::DrawItem = canvas::Ellipse[id := "", position := vector::Float2[x := 220.0, y := 260.0], radiusX := 200.0, radiusY := 20.0, paint := canvas::fill(cool), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET thinTurned AS canvas::DrawItem = canvas::Ellipse[id := "", position := vector::Float2[x := 800.0, y := 300.0], radiusX := 200.0, radiusY := 20.0, paint := canvas::fill(cool), transform := canvas::Transform[a := math::cos(1.5707963267948966), b := math::sin(1.5707963267948966), c := 0.0 - math::sin(1.5707963267948966), d := math::cos(1.5707963267948966), tx := 0.0, ty := 0.0]]

  ' Row 3 -- rotated, filled-and-stroked and stroke-only, so the band's width is
  ' readable all the way round a curve whose curvature varies ninefold.
  LET violet AS color::Color = color::rgb(226, 150, 255)
  LET tilted AS canvas::DrawItem = canvas::Ellipse[id := "", position := vector::Float2[x := 200.0, y := 450.0], radiusX := 150.0, radiusY := 50.0, paint := canvas::fillStroke(violet, color::rgb(255, 255, 255), 10.0), transform := canvas::Transform[a := math::cos(0.5235987755982988), b := math::sin(0.5235987755982988), c := 0.0 - math::sin(0.5235987755982988), d := math::cos(0.5235987755982988), tx := 0.0, ty := 0.0]]
  LET hollow AS canvas::DrawItem = canvas::Ellipse[id := "", position := vector::Float2[x := 530.0, y := 450.0], radiusX := 120.0, radiusY := 70.0, paint := canvas::stroke(violet, 16.0), transform := canvas::Transform[a := math::cos(0.0 - 0.5235987755982988), b := math::sin(0.0 - 0.5235987755982988), c := 0.0 - math::sin(0.0 - 0.5235987755982988), d := math::cos(0.0 - 0.5235987755982988), tx := 0.0, ty := 0.0]]

  ' Row 4 -- the load-bearing pair: an Ellipse with equal radii beside the Circle of
  ' that radius, same paint. They must be indistinguishable.
  LET mint AS color::Color = color::rgb(150, 255, 190)
  LET asCircle AS canvas::DrawItem = canvas::Ellipse[id := "", position := vector::Float2[x := 250.0, y := 585.0], radiusX := 38.0, radiusY := 38.0, paint := canvas::fillStroke(mint, color::rgb(255, 255, 255), 6.0), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET realCircle AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 450.0, y := 585.0], radius := 38.0, paint := canvas::fillStroke(mint, color::rgb(255, 255, 255), 6.0), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]

  canvas::present([ground, wide, round, thin, thinTurned, tilted, hollow, asCircle, realCircle])
  io::print("rendered")
END FUNC
"#;

/// The ellipse reference renders exactly.
///
/// Same rule as the other three references: a mismatch is a bug hunt, not a
/// re-baseline.
#[test]
fn ellipses_match_their_reference_exactly() {
    let rendered = render("canvas_golden_ellipses", ELLIPSES);
    let reference = golden_path("ellipses");

    if std::env::var_os("MFB_UPDATE_CANVAS_GOLDEN").is_some() {
        rendered.save_png(&reference);
        panic!(
            "regenerated {} — rerun without MFB_UPDATE_CANVAS_GOLDEN, and record in \
             the commit what proved the previous reference wrong",
            reference.display(),
        );
    }

    assert!(
        reference.exists(),
        "missing reference {}; generate it with MFB_UPDATE_CANVAS_GOLDEN=1",
        reference.display(),
    );
    let want = Frame::load_png(&reference);
    if let Err(diff) = compare_exact(&rendered, &want) {
        panic!(
            "the ellipse scene no longer renders to its reference image: {diff}\n\
             Localize by row: row 1 is the axis-aligned baseline, row 2 is the 10:1 \
             case where an approximate distance is worst (look at the flat ends), row \
             3 is the stroke band under varying curvature, and row 4 is the equal-radii \
             pair — a difference there is the `rx == ry` guard not firing, and the \
             ellipse and the circle beside it will no longer match."
        );
    }
}

/// The hardware backend draws the ellipse scene the reference shows.
///
/// plan-116-E Phase 4's acceptance. Gated on `gpuSelected=TRUE` first for the reason
/// `.ai/canvas-threading.md` §10 records from experience: a predicate that accepts a
/// kind its shader does not know renders the item as *nothing* and reports success —
/// 4,536 pixels wrong, reported as a pass. Since `Ellipse` is a brand-new kind, that is
/// exactly the failure available here, and only the stats line separates it from a
/// backend that declined the scene and let software draw a perfect picture.
#[test]
fn the_gpu_draws_the_ellipse_scene_the_reference_shows() {
    let (rendered, stats) = render_gpu("canvas_golden_ellipses_gpu", ELLIPSES);
    if !stats.contains("metalReady=TRUE") && !stats.contains("vulkanReady=TRUE") {
        eprintln!("skip: this host built no GPU pipeline\n{stats}");
        return;
    }
    assert!(
        !stats.contains("gpuFrames=0"),
        "the GPU pipeline built but no frame was rendered on it — a `*Renderable` \
         predicate declined the new kind, and every pixel below would then be the \
         software renderer marking its own work: {stats}"
    );

    let reference = golden_path("ellipses");
    assert!(
        reference.exists(),
        "missing reference {}; generate it with MFB_UPDATE_CANVAS_GOLDEN=1",
        reference.display(),
    );
    let want = Frame::load_png(&reference);
    if let Err(diff) = compare_within_tolerance(&rendered, &want, Tolerance::GPU_DEFAULT) {
        panic!(
            "the GPU's ellipse scene disagrees with the reference: {diff}\n\
             An ellipse missing entirely is the shader's kind-7 arm not being reached; \
             a wrongly-oriented one is the `ellipse` ivec4 (check the vertex AND \
             fragment ItemBlock declarations agree, since only one sets the std430 \
             stride); a rim that is off by more than a step or two is the bisection \
             count differing between the shader and the oracle — both are 24."
        );
    }
}

/// A linear ramp, a radial ramp, a multi-stop ramp, and a black→white ramp
/// (plan-116-F).
///
/// The fourth row is the one that carries an argument rather than a demonstration.
/// §4.3 chooses to interpolate **in linear light**, and black→white is where that
/// choice is visible: in linear light the ramp's midpoint is a mid grey, whereas
/// interpolating the sRGB-encoded bytes spends half the ramp below 22% of the light
/// and reads as dark-heavy. Open the image to check the decision rather than take it
/// on trust — which is what "decide by looking at the image" in §Open Decisions means.
///
/// The multi-stop row also carries the out-of-order rule: its stops are given
/// `0.0, 0.75, 0.4, 1.0`, and the third is clamped up to 0.75 rather than sorted into
/// place. The visible consequence is a hard edge at 0.75 instead of a fourth band, and
/// a reader comparing against the type's description should find exactly that.
const GRADIENTS: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color
IMPORT io

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)

  LET ground AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 900.0, height := 640.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(48, 50, 58)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]

  ' Row 1 -- a two-stop linear ramp, and the same ramp on a rotated axis so the
  ' gradient is visibly independent of the shape's own orientation.
  LET warm AS List OF canvas::GradientStop = [canvas::GradientStop[offset := 0.0, color := color::rgb(255, 64, 32)], canvas::GradientStop[offset := 1.0, color := color::rgb(32, 96, 255)]]
  LET gLin AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, startPoint := vector::Float2[x := 0.0, y := 0.0 - 60.0], endPoint := vector::Float2[x := 340.0, y := 0.0 - 60.0], stops := warm, spread := canvas::GradientSpread.Pad, transform := canvas::Transform[a := 0.0, b := 0.0, c := 0.0, d := 0.0, tx := 0.0, ty := 0.0], focalOffset := vector::Float2[x := 0.0, y := 0.0], focalRadius := 0.0, strokeApply := canvas::GradientApply.FillOnly]
  LET linBar AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 60.0, y := 60.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 340.0, height := 110.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH canvas::fill(color::rgb(0, 0, 0)) { gradient := gLin }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET gDiag AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, startPoint := vector::Float2[x := 0.0, y := 0.0], endPoint := vector::Float2[x := 340.0, y := 110.0], stops := warm, spread := canvas::GradientSpread.Pad, transform := canvas::Transform[a := 0.0, b := 0.0, c := 0.0, d := 0.0, tx := 0.0, ty := 0.0], focalOffset := vector::Float2[x := 0.0, y := 0.0], focalRadius := 0.0, strokeApply := canvas::GradientApply.FillOnly]
  LET diagBar AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 500.0, y := 60.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 340.0, height := 110.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH canvas::fill(color::rgb(0, 0, 0)) { gradient := gDiag }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]

  ' Row 2 -- radial, on a circle and on an ellipse. The ramp is measured in surface
  ' pixels, so it does not follow the shape: the ellipse's ramp stays circular.
  LET glow AS List OF canvas::GradientStop = [canvas::GradientStop[offset := 0.0, color := color::rgb(255, 244, 214)], canvas::GradientStop[offset := 1.0, color := color::rgb(90, 30, 120)]]
  LET gRad AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Radial, startPoint := vector::Float2[x := 0.0, y := 0.0], endPoint := vector::Float2[x := 100.0, y := 0.0], stops := glow, spread := canvas::GradientSpread.Pad, transform := canvas::Transform[a := 0.0, b := 0.0, c := 0.0, d := 0.0, tx := 0.0, ty := 0.0], focalOffset := vector::Float2[x := 0.0, y := 0.0], focalRadius := 0.0, strokeApply := canvas::GradientApply.FillOnly]
  LET orb AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 200.0, y := 320.0], radius := 95.0, paint := WITH canvas::fill(color::rgb(0, 0, 0)) { gradient := gRad }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET gRad2 AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Radial, startPoint := vector::Float2[x := 0.0, y := 0.0], endPoint := vector::Float2[x := 120.0, y := 0.0], stops := glow, spread := canvas::GradientSpread.Pad, transform := canvas::Transform[a := 0.0, b := 0.0, c := 0.0, d := 0.0, tx := 0.0, ty := 0.0], focalOffset := vector::Float2[x := 0.0, y := 0.0], focalRadius := 0.0, strokeApply := canvas::GradientApply.FillOnly]
  LET blob AS canvas::DrawItem = canvas::Ellipse[id := "", position := vector::Float2[x := 620.0, y := 320.0], radiusX := 170.0, radiusY := 80.0, paint := WITH canvas::fill(color::rgb(0, 0, 0)) { gradient := gRad2 }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]

  ' A gradient-filled POLYGON, in the gap between the two round shapes of row 2. It is
  ' here for one reason: a gradient's stops sit at the END of the geometry record, so a
  ' polygon's tail is its edges and THEN its stops, and both emitters find the first
  ' stop by subtracting from the record's own length. Every other kind puts the stops
  ' directly after the header, where a base computed either way agrees — so a polygon
  ' is the only shape that can tell a correct base from a lucky one.
  LET gTri AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, startPoint := vector::Float2[x := 330.0, y := 240.0], endPoint := vector::Float2[x := 330.0, y := 400.0], stops := warm, spread := canvas::GradientSpread.Pad, transform := canvas::Transform[a := 0.0, b := 0.0, c := 0.0, d := 0.0, tx := 0.0, ty := 0.0], focalOffset := vector::Float2[x := 0.0, y := 0.0], focalRadius := 0.0, strokeApply := canvas::GradientApply.FillOnly]
  LET triBar AS canvas::DrawItem = canvas::Polygon[id := "", position := vector::Float2[x := 0.0, y := 0.0], contours := [[vector::Float2[x := 330.0, y := 240.0], vector::Float2[x := 450.0, y := 240.0], vector::Float2[x := 390.0, y := 400.0]]], rule := canvas::FillRule.EvenOdd, paint := WITH canvas::fill(color::rgb(0, 0, 0)) { gradient := gTri }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]

  ' Row 3 -- four stops, the third given OUT OF ORDER at 0.4 after 0.75. It clamps up
  ' to 0.75 rather than sorting, so the picture has a hard edge there and no fourth
  ' band. That is the rule made visible.
  LET many AS List OF canvas::GradientStop = [canvas::GradientStop[offset := 0.0, color := color::rgb(255, 0, 0)], canvas::GradientStop[offset := 0.75, color := color::rgb(255, 220, 0)], canvas::GradientStop[offset := 0.4, color := color::rgb(0, 160, 255)], canvas::GradientStop[offset := 1.0, color := color::rgb(255, 255, 255)]]
  LET gMany AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, startPoint := vector::Float2[x := 0.0, y := 0.0 - 440.0], endPoint := vector::Float2[x := 780.0, y := 0.0 - 440.0], stops := many, spread := canvas::GradientSpread.Pad, transform := canvas::Transform[a := 0.0, b := 0.0, c := 0.0, d := 0.0, tx := 0.0, ty := 0.0], focalOffset := vector::Float2[x := 0.0, y := 0.0], focalRadius := 0.0, strokeApply := canvas::GradientApply.FillOnly]
  LET manyBar AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 60.0, y := 440.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 780.0, height := 70.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH canvas::fill(color::rgb(0, 0, 0)) { gradient := gMany }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]

  ' Row 4 -- black to white. THE case that makes the interpolation space inspectable.
  LET mono AS List OF canvas::GradientStop = [canvas::GradientStop[offset := 0.0, color := color::rgb(0, 0, 0)], canvas::GradientStop[offset := 1.0, color := color::rgb(255, 255, 255)]]
  LET gMono AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, startPoint := vector::Float2[x := 0.0, y := 0.0 - 540.0], endPoint := vector::Float2[x := 780.0, y := 0.0 - 540.0], stops := mono, spread := canvas::GradientSpread.Pad, transform := canvas::Transform[a := 0.0, b := 0.0, c := 0.0, d := 0.0, tx := 0.0, ty := 0.0], focalOffset := vector::Float2[x := 0.0, y := 0.0], focalRadius := 0.0, strokeApply := canvas::GradientApply.FillOnly]
  LET monoBar AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 60.0, y := 540.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 780.0, height := 60.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH canvas::fill(color::rgb(0, 0, 0)) { gradient := gMono }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]

  canvas::present([ground, linBar, diagBar, orb, blob, triBar, manyBar, monoBar])
  io::print("rendered")
END FUNC
"#;

/// The gradient reference renders exactly.
#[test]
fn gradients_match_their_reference_exactly() {
    let rendered = render("canvas_golden_gradients", GRADIENTS);
    let reference = golden_path("gradients");

    if std::env::var_os("MFB_UPDATE_CANVAS_GOLDEN").is_some() {
        rendered.save_png(&reference);
        panic!(
            "regenerated {} — rerun without MFB_UPDATE_CANVAS_GOLDEN, and record in \
             the commit what proved the previous reference wrong",
            reference.display(),
        );
    }

    assert!(
        reference.exists(),
        "missing reference {}; generate it with MFB_UPDATE_CANVAS_GOLDEN=1",
        reference.display(),
    );
    let want = Frame::load_png(&reference);
    if let Err(diff) = compare_exact(&rendered, &want) {
        panic!(
            "the gradient scene no longer renders to its reference image: {diff}\n\
             Localize by row: row 1 is the linear arm (the second bar's axis is \
             diagonal, so a ramp that follows the shape rather than the axis shows \
             there), row 2 is the radial arm (the ellipse's ramp must stay CIRCULAR — \
             the gradient is measured in surface pixels, not in the shape's space), row \
             3 is the out-of-order clamp (a fourth colour band means the stops were \
             sorted), and row 4 is the interpolation space (a dark-heavy ramp means it \
             moved to sRGB space)."
        );
    }
}

/// The hardware backend draws the gradient scene the reference shows.
///
/// plan-116-F Phase 4's acceptance, and gated on the stats line for the reason
/// `.ai/canvas-threading.md` §10 records: a gradient the shader cannot read draws the
/// flat `fill` underneath — every one of these bars is `rgb(0, 0, 0)` beneath its ramp
/// precisely so that failure is a *black bar*, not a subtly wrong one. But a backend
/// that DECLINED the scene hands it to software, which is the oracle, and every pixel
/// then matches by construction. `gpuFrames` is the only thing that separates the two.
#[test]
fn the_gpu_draws_the_gradient_scene_the_reference_shows() {
    let (rendered, stats) = render_gpu("canvas_golden_gradients_gpu", GRADIENTS);
    if !stats.contains("metalReady=TRUE") && !stats.contains("vulkanReady=TRUE") {
        eprintln!("skip: this host built no GPU pipeline\n{stats}");
        return;
    }
    assert!(
        !stats.contains("gpuFrames=0"),
        "the GPU pipeline built but no frame was rendered on it — a `*Renderable` \
         predicate declined the scene, and every pixel below would then be the \
         software renderer marking its own work: {stats}"
    );

    let reference = golden_path("gradients");
    assert!(
        reference.exists(),
        "missing reference {}; generate it with MFB_UPDATE_CANVAS_GOLDEN=1",
        reference.display(),
    );
    let want = Frame::load_png(&reference);
    if let Err(diff) = compare_within_tolerance(&rendered, &want, Tolerance::GPU_DEFAULT) {
        panic!(
            "the GPU's gradient scene disagrees with the reference: {diff}\n\
             A solid BLACK bar is the shader never taking the gradient arm at all \
             (`item.ellipse.z >= 2`), so the flat fill showed; a ramp that runs the \
             wrong way or off the shape is the axis ivec4 (check the vertex AND \
             fragment ItemBlock declarations agree — only one sets the std430 stride); \
             a ramp whose midpoint is too DARK is the lerp having moved out of linear \
             light, which is the one thing §4.3 fixes; and a ramp built from another \
             item's colours is the per-item first-stop index, `item.ellipse.w`."
        );
    }
}

/// A frame whose gradient stops overflow the buffer's third region declines to
/// software rather than drawing from the wrong place.
///
/// The cap is a frame SUM, not a per-item bound, because the stops of every item share
/// one region — so past it an item's first-stop index addresses memory another item
/// owns, and the shader reads it as a colour ramp. That draws a plausible wrong picture
/// rather than failing, which is why the predicate declines instead of truncating.
///
/// Asserted on the stats line and NOT by comparing pixels: a declined frame is drawn by
/// software, which is the oracle, so it matches any reference by construction. Pixel
/// equality here would pass whether the cap worked or not — it is the false pass this
/// test exists to avoid.
#[test]
fn a_frame_past_the_gradient_stop_cap_declines_to_software() {
    let (_, stats) = render_gpu("canvas_golden_gradient_overflow", GRADIENT_OVERFLOW);
    if !stats.contains("metalReady=TRUE") && !stats.contains("vulkanReady=TRUE") {
        eprintln!("skip: this host built no GPU pipeline\n{stats}");
        return;
    }
    assert!(
        stats.contains("gpuFrames=0"),
        "a scene carrying more gradient stops than the frame's region holds was \
         rendered on the GPU: `__CANVAS_MAX_FRAME_GRADIENT_STOPS` is a frame sum and \
         this scene is past it, so accepting the frame means the stop uploads ran off \
         the region and one item's ramp was read from another's stops: {stats}"
    );
}

/// One gradient with more stops than the whole frame's region holds: 131,073, one past
/// Metal's `METAL_MAX_FRAME_GRADIENT_STOPS` (131,072 since bug-686; Vulkan's
/// `MAX_FRAME_GRADIENT_STOPS` is still 4,096, so this is past both).
///
/// Built in a loop rather than spelled out, and deliberately as a SINGLE item: the cap
/// is a sum, so one item past it on its own proves the sum is consulted before the
/// upload rather than after — the ordering that matters. The bar is 2x2 pixels because
/// a declined frame is drawn by the software renderer, which evaluates the ramp per
/// pixel against every stop.
const GRADIENT_OVERFLOW: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color
IMPORT collections
IMPORT io

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)

  MUT stops AS List OF canvas::GradientStop = []
  MUT i AS Integer = 0
  WHILE i < 131073
    LET t AS Float = toFloat(i) / 131072.0
    stops = collections::append(stops, canvas::GradientStop[offset := t, color := color::rgb(255 - i / 520, 40, i / 520)])
    i = i + 1
  END WHILE

  LET g AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, startPoint := vector::Float2[x := 0.0, y := 0.0], endPoint := vector::Float2[x := 900.0, y := 0.0], stops := stops, spread := canvas::GradientSpread.Pad, transform := canvas::Transform[a := 0.0, b := 0.0, c := 0.0, d := 0.0, tx := 0.0, ty := 0.0], focalOffset := vector::Float2[x := 0.0, y := 0.0], focalRadius := 0.0, strokeApply := canvas::GradientApply.FillOnly]
  LET bar AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 2.0, height := 2.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH canvas::fill(color::rgb(0, 0, 0)) { gradient := g }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]

  canvas::present([bar])
  io::print("rendered")
END FUNC
"#;

/// A gradient on a **stroked** `canvas::Text` is ignored, like every other kind with no
/// interior.
///
/// plan-116-F **F18**, and the sibling of `a_gradient_on_a_stroke_only_kind_is_ignored`
/// in `rt_canvas_rasteriser.rs` — the same defect reached a different way.
/// `__canvas_paintHeader` decides "has this kind an interior?" by reading slot 0, and
/// `__canvas_textHeader` sets slot 0 to `POLYGON` before calling it, because a stroked
/// text really is lowered to one. So the `Text` skip did not fire: the header counted
/// `stops * 5` into slot 1 while `__canvas_tailFor`'s `Text` arm appended only
/// `__canvas_textEdges(t)`. The record declared ten floats it did not have,
/// `__canvas_gradientStopBase` read from one past its end, and the run drew its fill
/// out of the following record's header.
///
/// Measured before the fix: **874 pixels** differ, the glyph fill going from the
/// `rgb(255, 255, 0)` it was given to `rgb(11, 0, 0)`. After the fix: 0.
///
/// **The stroke is what makes this test work.** An unstroked `Text` goes through
/// `__canvas_glyphRunHeader`, which sets slot 0 to `TEXT` before `paintHeader` and was
/// never affected — so a version of this test without `fillStroke` passes against the
/// bug and proves nothing.
#[test]
fn a_gradient_on_a_stroked_text_is_ignored() {
    const STROKED: &str =
        "canvas::fillStroke(color::rgb(255, 255, 0), color::rgb(0, 128, 255), 4.0)";

    fn program(paint: &str) -> String {
        format!(
            "IMPORT app\nIMPORT canvas\nIMPORT vector\nIMPORT color\nIMPORT io\n\nFUNC main() AS Nothing\n  \
             app::setMode(app::Mode.Canvas)\n  \
             RES face AS canvas::Font = canvas::loadFont(\"fixture.ttf\") TRAP(e)\n    \
             io::print(\"nofont\")\n    RETURN\n  END TRAP\n  \
             LET s AS List OF canvas::GradientStop = [canvas::GradientStop[offset := 0.0, \
             color := color::rgb(255, 0, 0)], canvas::GradientStop[offset := 1.0, \
             color := color::rgb(0, 0, 255)]]\n  \
             LET g AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, \
             startPoint := vector::Float2[x := 0.0, y := 0.0], \
             endPoint := vector::Float2[x := 400.0, y := 0.0], stops := s, spread := canvas::GradientSpread.Pad, transform := canvas::Transform[a := 0.0, b := 0.0, c := 0.0, d := 0.0, tx := 0.0, ty := 0.0], focalOffset := vector::Float2[x := 0.0, y := 0.0], focalRadius := 0.0, strokeApply := canvas::GradientApply.FillOnly]\n  \
             LET label AS canvas::DrawItem = canvas::Text[id := \"\", position := vector::Float2[x := 40.0, y := 120.0], text := \"AA\", fonts := [face], fontSize := 90.0, letterSpacing := 0.0, paint := {paint}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET box AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 300.0, y := 380.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 60.0, height := 60.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(0, 255, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([label, box])\n  io::print(\"rendered\")\nEND FUNC\n"
        )
    }

    let plain = render_with_font("canvas_stroked_text_plain", &program(STROKED));
    let ramped = render_with_font(
        "canvas_stroked_text_ramped",
        &program(&format!("WITH {STROKED} {{ gradient := g }}")),
    );
    if let Err(diff) = compare_exact(&ramped, &plain) {
        panic!(
            "a gradient changed what a stroked `Text` draws: {diff}\n\
             `canvas::Text` ignores `gradient` — a glyph is a cached coverage \
             bitmap and a stroked one is an outline polygon, and neither has a fill to \
             replace. Before this was fixed the count was 874, with the glyph fill \
             reading out of the following record's header."
        );
    }
}

/// The polyline reference renders exactly.
///
/// Same rule as the other references: a mismatch is a bug hunt, not a re-baseline.
#[test]
fn polylines_match_their_reference_exactly() {
    let rendered = render("canvas_golden_polylines", POLYLINES);
    let reference = golden_path("polyline");

    if std::env::var_os("MFB_UPDATE_CANVAS_GOLDEN").is_some() {
        rendered.save_png(&reference);
        panic!(
            "regenerated {} — rerun without MFB_UPDATE_CANVAS_GOLDEN, and record in \
             the commit what proved the previous reference wrong",
            reference.display(),
        );
    }

    assert!(
        reference.exists(),
        "missing reference {}; generate it with MFB_UPDATE_CANVAS_GOLDEN=1",
        reference.display(),
    );
    let want = Frame::load_png(&reference);
    if let Err(diff) = compare_exact(&rendered, &want) {
        panic!(
            "the polyline scene no longer renders to its reference image: {diff}\n\
             Localize by row: row 1 is the round/butt zig-zag, row 2 the switch-back \
             and the repeated-end butt path, row 3 the per-point colours, row 4 the \
             partial colours, the two-point path and the degenerate ones (the butt \
             coincident path and the one-point path are meant to be absent)."
        );
    }
}

/// A two-point `PolyLine` draws the pixels of the `Line` between the same points.
///
/// plan-154-A's falsifying check. A polyline measures each segment with
/// `__canvas_segmentDistance`'s own arithmetic and cuts a butt end with
/// `__canvas_segmentDistanceButt`'s planes; if either drifted by a last bit — the tail's
/// `invLenSq` multiply in place of the divide, say — an antialiased edge byte would move
/// and this would fail where the golden alone might not be looked at closely enough.
/// Both caps, a fractional diagonal, and a thin line whose edge is almost all
/// antialiasing.
#[test]
fn a_two_point_polyline_draws_the_line_between_its_points() {
    fn scene(item: &str) -> String {
        let draw = |x1: f64, y1: f64, x2: f64, y2: f64, cap: &str, width: f64| {
            match item {
            "line" => format!(
                "canvas::Line[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], startAt := vector::Float2[x := {x1:?}, y := {y1:?}], endAt := vector::Float2[x := {x2:?}, y := {y2:?}], paint := WITH canvas::stroke(color::rgb(250, 200, 90), {width:?}) {{ cap := canvas::CapStyle.{cap} }}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]"
            ),
            _ => format!(
                "canvas::PolyLine[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], points := [vector::Float2[x := {x1:?}, y := {y1:?}], vector::Float2[x := {x2:?}, y := {y2:?}]], colors := [], paint := WITH canvas::stroke(color::rgb(250, 200, 90), {width:?}) {{ cap := canvas::CapStyle.{cap} }}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]"
            ),
        }
        };
        let items = [
            draw(80.0, 100.0, 380.0, 100.0, "Round", 30.0),
            draw(500.0, 100.0, 800.0, 100.0, "Butt", 30.0),
            draw(90.25, 220.5, 400.75, 410.125, "Round", 17.5),
            draw(510.25, 220.5, 820.75, 410.125, "Butt", 17.5),
            draw(100.1, 520.3, 780.9, 590.7, "Butt", 1.5),
        ];
        let lets: String = items
            .iter()
            .enumerate()
            .map(|(i, item)| format!("  LET i{i} AS canvas::DrawItem = {item}\n"))
            .collect();
        format!(
            "IMPORT app\nIMPORT canvas\nIMPORT vector\nIMPORT color\nIMPORT io\n\nFUNC main() AS Nothing\n  \
             app::setMode(app::Mode.Canvas)\n{lets}  canvas::present([i0, i1, i2, i3, i4])\n  \
             io::print(\"rendered\")\nEND FUNC\n"
        )
    }
    let lines = render("canvas_polyline_as_line_a", &scene("line"));
    let polylines = render("canvas_polyline_as_line_b", &scene("polyline"));
    let painted = lines.pixels.chunks(4).filter(|p| p[0] != 0).count();
    assert!(
        painted > 10_000,
        "the Line scene drew {painted} pixels, so it proves nothing"
    );
    if let Err(diff) = compare_exact(&polylines, &lines) {
        panic!(
            "a two-point PolyLine differs from the Line between the same points: {diff}\n\
             `__canvas_polylineScan` must measure a segment with exactly \
             `__canvas_segmentDistance`'s and `__canvas_segmentDistanceButt`'s arithmetic."
        );
    }
}

/// The hardware backend draws the polyline scene the reference shows (plan-154-B).
///
/// Against the committed reference, like the other GPU twins, and gated on every frame
/// having been drawn on the GPU: a backend that declined the scene would fall back to
/// software and reproduce the reference exactly.
#[test]
fn the_gpu_draws_the_polyline_scene_the_reference_shows() {
    let (rendered, stats) = render_gpu("canvas_golden_polylines_gpu", POLYLINES);
    if !stats.contains("metalReady=TRUE") && !stats.contains("vulkanReady=TRUE") {
        eprintln!("skip: this host built no GPU pipeline\n{stats}");
        return;
    }
    assert!(
        !stats.contains("gpuFrames=0"),
        "the GPU pipeline built but no frame was rendered on it — a `*Renderable` \
         predicate declined the PolyLine frame, and every pixel below would then be the \
         software renderer marking its own work: {stats}"
    );
    let reference = golden_path("polyline");
    assert!(
        reference.exists(),
        "missing reference {}; generate it with MFB_UPDATE_CANVAS_GOLDEN=1",
        reference.display(),
    );
    let want = Frame::load_png(&reference);
    if let Err(diff) = compare_within_tolerance(&rendered, &want, Tolerance::GPU_DEFAULT) {
        panic!(
            "the GPU's polyline scene disagrees with the reference: {diff}\n\
             Localize by row as `polylines_match_their_reference_exactly` describes. A \
             butt end that domes is the first/last segment words; a colour step at a \
             corner is the path parameter; the coincident round dot is the first == last \
             zero-length case."
        );
    }
}
