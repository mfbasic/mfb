//! The canvas software rasteriser renders each primitive to hand-checked pixels
//! (plan-98-C Phase 1).
//!
//! These run a real headless `--app` program and read the frame back through
//! `MFB_CANVAS_DUMP`, rather than inspecting the emitted code. That is deliberate:
//! the rasteriser is the **oracle** plan-98-E/F are compared against, so what has to
//! be true is a statement about *pixels*, and a codegen-shape assertion would pass
//! just as happily while the arithmetic was wrong. The truncated sRGB table this
//! phase found — which rendered every antialiased pixel black and which every
//! structural check missed — is exactly that failure.
//!
//! Every expected value below is derived by hand from the documented conventions
//! (Y-down, pixel centres at `+0.5`, coverage `clamp(0.5 - d, 0, 1)`, blending in
//! linear space through the sRGB table), not copied from a run.

#[path = "../common/mod.rs"]
mod common;

#[path = "rt_canvas_rasteriser/clips_blends_and_transforms.rs"]
mod clips_blends_and_transforms;
#[path = "rt_canvas_rasteriser/ellipses_and_corners.rs"]
mod ellipses_and_corners;
#[path = "rt_canvas_rasteriser/gradients.rs"]
mod gradients;
#[path = "rt_canvas_rasteriser/groups.rs"]
mod groups;
#[path = "rt_canvas_rasteriser/shapes.rs"]
mod shapes;
#[path = "rt_canvas_rasteriser/strokes_and_dashes.rs"]
mod strokes_and_dashes;

use std::process::Command;

/// Surface dimensions, fixed by `__canvas_surfaceSize` until plan-98-D brings resize.
const WIDTH: usize = 900;
const HEIGHT: usize = 640;

/// Build a `--app` program, run it headless, and return the dumped RGBA frame plus
/// whatever the geometry cache reported for each frame.
fn render(name: &str, source: &str) -> (Vec<u8>, Vec<String>) {
    let project = common::temp_project(name, source);
    let frame = project.join("frame.rgba");
    let stats = project.join("stats.txt");
    let binary = common::build_app_debug(&project, name);
    let run = Command::new(&binary)
        .env("MFB_HEADLESS", "1")
        .env("MFB_CANVAS_DUMP", &frame)
        .env("MFB_CANVAS_STATS", &stats)
        // Render synchronously: since plan-98-D Phase 2 the render runs on a
        // graphics thread and presents that arrive between frames coalesce by
        // design, so how many frames a run produces is otherwise a scheduling
        // detail — and every assertion below is about a frame.
        .env("MFB_CANVAS_SYNC", "1")
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", binary.display()));
    assert!(
        run.status.success(),
        "program {}:\n{}\n{}",
        common::exit_description(&run.status),
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr),
    );

    let pixels = std::fs::read(&frame).unwrap_or_else(|e| {
        panic!(
            "canvas dump {} not written: {e}\nstdout:\n{}\nstderr:\n{}\nproject dir: {:?}\nstats: {:?}",
            frame.display(),
            String::from_utf8_lossy(&run.stdout),
            String::from_utf8_lossy(&run.stderr),
            std::fs::read_dir(&project)
                .map(|d| d.filter_map(|e| e.ok().map(|e| e.file_name())).collect::<Vec<_>>())
                .unwrap_or_default(),
            std::fs::read_to_string(&stats),
        )
    });
    assert_eq!(
        pixels.len(),
        WIDTH * HEIGHT * 4,
        "dump is not a {WIDTH}x{HEIGHT} RGBA frame",
    );
    let lines = std::fs::read_to_string(&stats)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect();
    let _ = std::fs::remove_dir_all(&project);
    (pixels, lines)
}

fn pixel(frame: &[u8], x: usize, y: usize) -> (u8, u8, u8, u8) {
    let i = (y * WIDTH + x) * 4;
    (frame[i], frame[i + 1], frame[i + 2], frame[i + 3])
}

/// A scene program: `main` sets canvas mode, presents, and prints a marker so a
/// silent early exit cannot pass as a successful render.
fn scene(body: &str) -> String {
    format!(
        "IMPORT app\nIMPORT canvas\nIMPORT vector\nIMPORT color\nIMPORT collections\nIMPORT io\nIMPORT math\n\nFUNC main() AS Nothing\n  \
         app::setMode(app::Mode.Canvas)\n{body}  io::print(\"rendered\")\nEND FUNC\n"
    )
}

/// `sq(x, y, s)`: the `s`-sided square at `(x, y)`, its corners clockwise under Y-down
/// (top-left, top-right, bottom-right, bottom-left). `sqCcw` is the same square run the
/// other way. Shared by the plan-163-A ring tests below.
const RING_HELPERS: &str =
    "\nFUNC sq(x AS Float, y AS Float, s AS Float) AS List OF vector::Float2\n  \
     RETURN [vector::Float2[x := x, y := y], vector::Float2[x := x + s, y := y], \
     vector::Float2[x := x + s, y := y + s], vector::Float2[x := x, y := y + s]]\n\
     END FUNC\n\
     FUNC sqCcw(x AS Float, y AS Float, s AS Float) AS List OF vector::Float2\n  \
     RETURN [vector::Float2[x := x, y := y], vector::Float2[x := x, y := y + s], \
     vector::Float2[x := x + s, y := y + s], vector::Float2[x := x + s, y := y]]\n\
     END FUNC\n";

// ---- plan-172-D: placement ---------------------------------------------------------
//
// A shape is drawn at `position + transform(local point)`: the transform pivots about
// the reference point. The four cases below are the placement rule's own tests — the
// helper that holds it (`__canvas_placement`) is MFBASIC inside the package and cannot
// be called from a unit test, so it is proved by what it draws.

/// The pivot is `position`. A 150 x 20 ellipse at (300, 200), turned a quarter turn:
/// its long axis is vertical through (300, 200), so 120 px above and below the centre
/// are inside it, and 120 px left and right — inside it had it not turned — are not.
///
/// The control that the turn is about the CENTRE and not the surface origin is the
/// centre pixel itself: a quarter turn about (0, 0) would carry (300, 200) to
/// (-200, 300), off the surface, and the frame would be empty.
#[test]
fn placement_a_transform_turns_a_shape_about_its_position() {
    let (frame, _) = render(
        "canvas_placement_pivot",
        &scene(
            "  LET quarter AS canvas::Transform = canvas::Transform[a := 0.0, b := 1.0, c := 0.0 - 1.0, d := 0.0, tx := 0.0, ty := 0.0]\n  \
             LET egg AS canvas::DrawItem = canvas::Ellipse[id := \"\", position := vector::Float2[x := 300.0, y := 200.0], radiusX := 150.0, radiusY := 20.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := quarter]\n  \
             canvas::present([egg])\n",
        ),
    );
    let white = (255, 255, 255, 255);
    let black = (0, 0, 0, 255);
    assert_eq!(
        pixel(&frame, 300, 200),
        white,
        "the centre stays where `position` puts it"
    );
    assert_eq!(
        pixel(&frame, 300, 80),
        white,
        "120 px above the centre, along the turned long axis"
    );
    assert_eq!(
        pixel(&frame, 300, 320),
        white,
        "120 px below the centre, along the turned long axis"
    );
    assert_eq!(
        pixel(&frame, 180, 200),
        black,
        "120 px left: inside the UNTURNED ellipse only"
    );
    assert_eq!(
        pixel(&frame, 420, 200),
        black,
        "120 px right: inside the UNTURNED ellipse only"
    );
}

/// `transform.tx` / `ty` add to `position`: the same turned ellipse, 100 px right and
/// 50 px down of where its `position` alone would put it.
#[test]
fn placement_a_transforms_translation_adds_to_the_position() {
    let (frame, _) = render(
        "canvas_placement_translation",
        &scene(
            "  LET quarter AS canvas::Transform = canvas::Transform[a := 0.0, b := 1.0, c := 0.0 - 1.0, d := 0.0, tx := 100.0, ty := 50.0]\n  \
             LET egg AS canvas::DrawItem = canvas::Ellipse[id := \"\", position := vector::Float2[x := 300.0, y := 200.0], radiusX := 150.0, radiusY := 20.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := quarter]\n  \
             canvas::present([egg])\n",
        ),
    );
    assert_eq!(
        pixel(&frame, 400, 250),
        (255, 255, 255, 255),
        "the centre, moved by (tx, ty)"
    );
    assert_eq!(
        pixel(&frame, 400, 370),
        (255, 255, 255, 255),
        "120 px below the moved centre"
    );
    assert_eq!(
        pixel(&frame, 300, 320),
        (0, 0, 0, 255),
        "where it would be with no translation"
    );
}

/// A shape moves WHOLE when only `position` changes: its clip and its gradient are in
/// its local space, so they go with it.
///
/// One circle with a two-stop linear gradient and a clip that cuts off its right
/// third, rendered at (250, 300) and again at (350, 300). Every pixel of the second
/// frame equals the pixel 100 px to its left in the first. Positions and the shift are
/// whole numbers, so the two frames are the same arithmetic one column over — an exact
/// comparison, not a tolerance. Had the gradient or the clip stayed in surface
/// coordinates, the ramp would slide across the disc and the cut would move over it.
#[test]
fn placement_a_clipped_gradient_shape_moves_whole_with_its_position() {
    let body_with = |x: &str, gradient_transform: &str| {
        scene(&format!(
            "  LET stops AS List OF canvas::GradientStop = [canvas::GradientStop[offset := 0.0, color := color::rgb(255, 0, 0)], canvas::GradientStop[offset := 1.0, color := color::rgb(0, 0, 255)]]\n  \
             LET ramp AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, startPoint := vector::Float2[x := 0.0 - 80.0, y := 0.0], endPoint := vector::Float2[x := 80.0, y := 0.0], stops := stops, spread := canvas::GradientSpread.Pad, transform := {gradient_transform}, focalOffset := vector::Float2[x := 0.0, y := 0.0], focalRadius := 0.0, strokeApply := canvas::GradientApply.FillOnly]\n  \
             LET cut AS List OF List OF vector::Float2 = [[vector::Float2[x := 0.0 - 90.0, y := 0.0 - 90.0], vector::Float2[x := 30.0, y := 0.0 - 90.0], vector::Float2[x := 30.0, y := 90.0], vector::Float2[x := 0.0 - 90.0, y := 90.0]]]\n  \
             LET disc AS canvas::DrawItem = canvas::Circle[id := \"\", position := vector::Float2[x := {x}, y := 300.0], radius := 80.0, paint := WITH canvas::fill(color::rgb(0, 0, 0)) {{ gradient := ramp, clip := cut }}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([disc])\n"
        ))
    };
    let body = |x: &str| body_with(x, "canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]");
    let (first, _) = render("canvas_placement_move_a", &body("250.0"));
    let (second, _) = render("canvas_placement_move_b", &body("350.0"));

    // The gradient's own transform is local too. A ramp scaled 2 x across and sheared,
    // drawn at the same two positions: it must move with the shape. Re-centring only
    // the two points and leaving this transform about the surface origin would slide
    // the ramp by (L - I) * 100 px between the two frames.
    let skew = "canvas::Transform[a := 2.0, b := 0.0, c := 0.5, d := 1.0, tx := 0.0, ty := 0.0]";
    let (skew_a, _) = render("canvas_placement_move_skew_a", &body_with("250.0", skew));
    let (skew_b, _) = render("canvas_placement_move_skew_b", &body_with("350.0", skew));
    assert_ne!(
        skew_a, first,
        "the gradient transform changes the picture, so it is being applied"
    );
    for y in 200..400usize {
        for x in 250..450usize {
            assert_eq!(
                pixel(&skew_b, x, y),
                pixel(&skew_a, x - 100, y),
                "transformed ramp: ({x}, {y}) of the moved shape differs from ({}, {y})",
                x - 100
            );
        }
    }

    // The scene is what it claims: red towards the left, the right third cut away.
    let left = pixel(&first, 190, 300);
    let right = pixel(&first, 270, 300);
    assert!(
        left.0 > 200 && left.0 > left.2 && right.2 > left.2 && right.0 < left.0,
        "the ramp runs red to blue, left to right: {left:?} then {right:?}"
    );
    assert_eq!(
        pixel(&first, 300, 300),
        (0, 0, 0, 255),
        "right of the cut, inside the disc"
    );

    let mut compared = 0usize;
    for y in 200..400usize {
        for x in 250..450usize {
            assert_eq!(
                pixel(&second, x, y),
                pixel(&first, x - 100, y),
                "({x}, {y}) of the moved shape differs from ({}, {y}) of the original",
                x - 100
            );
            compared += 1;
        }
    }
    assert_eq!(compared, 40_000);
    // And it did move: where the first disc's red edge was is background now.
    assert_eq!(
        pixel(&second, 190, 300),
        (0, 0, 0, 255),
        "the shape left its old place"
    );
}

/// A transform with no area is not in effect — the identity — so the shape is drawn
/// at its `position`, exactly as with the all-zero transform. `[1, 2, 2, 4]` has
/// determinant zero. Compared whole-frame against the all-zero spelling: before
/// plan-172-D a singular transform fell back to the identity about the surface origin,
/// which with a `position` would have left the shape at (0, 0).
#[test]
fn placement_a_transform_with_no_area_leaves_the_shape_at_its_position() {
    let body = |transform: &str| {
        scene(&format!(
            "  LET dot AS canvas::DrawItem = canvas::Circle[id := \"\", position := vector::Float2[x := 300.0, y := 300.0], radius := 90.0, paint := canvas::fill(color::rgb(255, 200, 0)), transform := {transform}]\n  \
             canvas::present([dot])\n"
        ))
    };
    let (plain, _) = render(
        "canvas_placement_plain",
        &body("canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]"),
    );
    let (singular, _) = render(
        "canvas_placement_singular",
        &body("canvas::Transform[a := 1.0, b := 2.0, c := 2.0, d := 4.0, tx := 0.0, ty := 0.0]"),
    );
    assert_eq!(
        pixel(&plain, 300, 300),
        (255, 200, 0, 255),
        "the disc is at its position"
    );
    assert_eq!(
        singular, plain,
        "a no-area transform must draw the shape where `position` puts it"
    );
}

// ---- plan-172-F: placement of the point-list shapes ----------------------------------
//
// `Line`, `Polygon` and `PolyLine` hold offsets from `position`. The rule is
// plan-172-D's, applied to every point: no transform in effect, each point is drawn at
// `position + point`; a transform in effect turns the points about `position`.

const NO_TRANSFORM: &str = "canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]";

/// The pivot is `position`. A triangle whose points sit around (0, 0) — apex 60 px
/// ABOVE the origin, base 40 px below it — at `position` (400, 300), turned half a
/// turn: the apex is now 60 px BELOW (400, 300) and the base 40 px above.
///
/// A half turn about the surface origin instead would carry the whole triangle to
/// around (-400, -300), off the surface; a turn about the triangle's first vertex
/// would put its base somewhere else again. The probes pin all three.
///
/// A line is the same rule with two points: 200 px long through its `position`, turned
/// a quarter turn, it is vertical through (400, 300).
#[test]
fn path_placement_points_turn_about_the_position() {
    let (frame, _) = render(
        "canvas_path_placement_pivot",
        &scene(
            "  LET half AS canvas::Transform = canvas::Transform[a := 0.0 - 1.0, b := 0.0, c := 0.0, d := 0.0 - 1.0, tx := 0.0, ty := 0.0]\n  \
             LET wedge AS canvas::DrawItem = canvas::Polygon[id := \"\", position := vector::Float2[x := 400.0, y := 300.0], contours := [[vector::Float2[x := 0.0, y := 0.0 - 60.0], vector::Float2[x := 50.0, y := 40.0], vector::Float2[x := 0.0 - 50.0, y := 40.0]]], rule := canvas::FillRule.EvenOdd, paint := canvas::fill(color::rgb(255, 255, 255)), transform := half]\n  \
             canvas::present([wedge])\n",
        ),
    );
    let white = (255, 255, 255, 255);
    let black = (0, 0, 0, 255);
    assert_eq!(
        pixel(&frame, 400, 300),
        white,
        "`position` is inside it still"
    );
    assert_eq!(
        pixel(&frame, 400, 350),
        white,
        "10 px short of the apex, which is now BELOW the position"
    );
    assert_eq!(
        pixel(&frame, 400, 250),
        black,
        "where the apex was before the turn: above the turned base"
    );
    assert_eq!(
        pixel(&frame, 440, 265),
        white,
        "the base's right end, now above the position"
    );
    assert_eq!(
        pixel(&frame, 440, 335),
        black,
        "where the base's right end was before the turn"
    );

    let (frame, _) = render(
        "canvas_path_placement_line",
        &scene(
            "  LET quarter AS canvas::Transform = canvas::Transform[a := 0.0, b := 1.0, c := 0.0 - 1.0, d := 0.0, tx := 0.0, ty := 0.0]\n  \
             LET bar AS canvas::DrawItem = canvas::Line[id := \"\", position := vector::Float2[x := 400.0, y := 300.0], startAt := vector::Float2[x := 0.0 - 100.0, y := 0.0], endAt := vector::Float2[x := 100.0, y := 0.0], paint := WITH canvas::stroke(color::rgb(255, 255, 255), 10.0) { cap := canvas::CapStyle.Butt }, transform := quarter]\n  \
             canvas::present([bar])\n",
        ),
    );
    assert_eq!(
        pixel(&frame, 400, 300),
        white,
        "the line's middle is its position"
    );
    assert_eq!(pixel(&frame, 400, 220), white, "80 px up the turned line");
    assert_eq!(pixel(&frame, 400, 380), white, "80 px down the turned line");
    assert_eq!(
        pixel(&frame, 320, 300),
        black,
        "80 px left: the unturned line only"
    );
    assert_eq!(
        pixel(&frame, 480, 300),
        black,
        "80 px right: the unturned line only"
    );
}

/// A polygon moves WHOLE when only `position` changes, and what it draws at a position
/// is what the same polygon draws with the position folded into its points.
///
/// A pentagon with a stroke, a dash pattern and a clip that cuts off its right side,
/// at `position` (250, 300) and at (350, 300): every pixel of the second frame equals
/// the pixel 100 px to its left in the first. Then the second frame against a third
/// scene — `position` at the origin and (350, 300) added to every point, to the clip
/// and to the gradient by hand: the two frames are identical, whole.
///
/// Run twice, because the two builders are separate code: a flat fill is built by the
/// native builder (`func_geo_build.rs`), a gradient fill by the MFBASIC one
/// (`helper_geometry/`). Positions and the shift are whole numbers, so the frames are
/// the same arithmetic one column over — an exact comparison, not a tolerance.
#[test]
fn path_placement_a_clipped_polygon_moves_whole_with_its_position() {
    let ring = [
        (0.0, -80.0),
        (76.0, -25.0),
        (47.0, 65.0),
        (-47.0, 65.0),
        (-76.0, -25.0),
    ];
    let cut = [(-90.0, -90.0), (30.0, -90.0), (30.0, 90.0), (-90.0, 90.0)];
    let points = |list: &[(f64, f64)], dx: f64, dy: f64| -> String {
        list.iter()
            .map(|(x, y)| {
                let (x, y) = (x + dx, y + dy);
                let lit = |v: f64| {
                    if v < 0.0 {
                        format!("0.0 - {:?}", -v)
                    } else {
                        format!("{v:?}")
                    }
                };
                format!("vector::Float2[x := {}, y := {}]", lit(x), lit(y))
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    // `at` is the item's position; `fold` is added to every local coordinate by hand.
    let body = |at: (f64, f64), fold: (f64, f64), gradient: bool| {
        let fill = if gradient {
            format!(
                "WITH canvas::fillStroke(color::rgb(0, 0, 0), color::rgb(255, 255, 255), 6.0) {{ gradient := canvas::Gradient[kind := canvas::GradientKind.Linear, startPoint := vector::Float2[x := {:?}, y := {:?}], endPoint := vector::Float2[x := {:?}, y := {:?}], stops := [canvas::GradientStop[offset := 0.0, color := color::rgb(255, 0, 0)], canvas::GradientStop[offset := 1.0, color := color::rgb(0, 0, 255)]], spread := canvas::GradientSpread.Pad, transform := {NO_TRANSFORM}, focalOffset := vector::Float2[x := 0.0, y := 0.0], focalRadius := 0.0, strokeApply := canvas::GradientApply.FillOnly], dashes := [14.0, 6.0], clip := [[{}]] }}",
                fold.0 - 80.0,
                fold.1,
                fold.0 + 80.0,
                fold.1,
                points(&cut, fold.0, fold.1)
            )
        } else {
            format!(
                "WITH canvas::fillStroke(color::rgb(255, 160, 0), color::rgb(255, 255, 255), 6.0) {{ dashes := [14.0, 6.0], clip := [[{}]] }}",
                points(&cut, fold.0, fold.1)
            )
        };
        scene(&format!(
            "  LET shape AS canvas::DrawItem = canvas::Polygon[id := \"\", position := vector::Float2[x := {:?}, y := {:?}], contours := [[{}]], rule := canvas::FillRule.EvenOdd, paint := {fill}, transform := {NO_TRANSFORM}]\n  \
             canvas::present([shape])\n",
            at.0,
            at.1,
            points(&ring, fold.0, fold.1)
        ))
    };
    for (tag, gradient) in [("flat", false), ("ramp", true)] {
        let (first, _) = render(
            &format!("canvas_path_move_{tag}_a"),
            &body((250.0, 300.0), (0.0, 0.0), gradient),
        );
        let (second, _) = render(
            &format!("canvas_path_move_{tag}_b"),
            &body((350.0, 300.0), (0.0, 0.0), gradient),
        );
        let (folded, _) = render(
            &format!("canvas_path_move_{tag}_c"),
            &body((0.0, 0.0), (350.0, 300.0), gradient),
        );
        // The scene is what it claims: filled left of the cut, nothing right of it.
        assert_ne!(
            pixel(&first, 230, 300),
            (0, 0, 0, 255),
            "{tag}: inside the pentagon, left of the cut"
        );
        assert_eq!(
            pixel(&first, 300, 300),
            (0, 0, 0, 255),
            "{tag}: inside the pentagon, right of the cut"
        );
        let mut compared = 0usize;
        for y in 200..400usize {
            for x in 250..450usize {
                assert_eq!(
                    pixel(&second, x, y),
                    pixel(&first, x - 100, y),
                    "{tag}: ({x}, {y}) of the moved polygon differs from ({}, {y})",
                    x - 100
                );
                compared += 1;
            }
        }
        assert_eq!(compared, 40_000);
        assert_eq!(
            pixel(&second, 180, 300),
            (0, 0, 0, 255),
            "{tag}: the polygon left its old place"
        );
        assert!(
            second == folded,
            "{tag}: a polygon at a position must draw exactly what the same polygon draws \
             with the position added to its points, its clip and its gradient"
        );
    }
}

/// A `PolyLine` with a colour per point at a non-zero `position` draws exactly what the
/// same path draws with the offset folded into its points.
///
/// The position is fractional — (120.5, 80.25) — and every sum is exact in binary, so
/// the two records are the same numbers and the frames are compared whole. The colours
/// ride the path parameter, not the coordinates, so a path moved by its position keeps
/// each point's colour; a round join and a butt cap are in the scene because both are
/// computed from the placed segments.
#[test]
fn path_placement_a_coloured_polyline_at_a_position_equals_its_points_moved() {
    let body = |position: &str, pts: &str| {
        scene(&format!(
            "  LET trail AS canvas::DrawItem = canvas::PolyLine[id := \"\", position := {position}, points := [{pts}], colors := [color::rgb(255, 0, 0), color::rgb(0, 255, 0), color::rgb(0, 0, 255), color::rgb(255, 255, 255)], paint := WITH canvas::stroke(color::rgb(255, 255, 255), 12.0) {{ cap := canvas::CapStyle.Butt, join := canvas::JoinStyle.Round }}, transform := {NO_TRANSFORM}]\n  \
             canvas::present([trail])\n"
        ))
    };
    let (placed, _) = render(
        "canvas_path_polyline_placed",
        &body(
            "vector::Float2[x := 120.5, y := 80.25]",
            "vector::Float2[x := 0.0, y := 0.0], vector::Float2[x := 100.0, y := 50.0], vector::Float2[x := 200.0, y := 0.0], vector::Float2[x := 260.0, y := 120.0]",
        ),
    );
    let (folded, _) = render(
        "canvas_path_polyline_folded",
        &body(
            "vector::Float2[x := 0.0, y := 0.0]",
            "vector::Float2[x := 120.5, y := 80.25], vector::Float2[x := 220.5, y := 130.25], vector::Float2[x := 320.5, y := 80.25], vector::Float2[x := 380.5, y := 200.25]",
        ),
    );
    let painted = placed
        .chunks_exact(4)
        .filter(|p| p[0] != 0 || p[1] != 0 || p[2] != 0)
        .count();
    assert!(painted > 3_000, "the path drew {painted} pixels");
    let start = pixel(&placed, 125, 82);
    assert!(
        start.0 > 200 && start.1 < 80 && start.2 < 80,
        "the path starts red at its position: {start:?}"
    );
    assert!(
        placed == folded,
        "a path at a position must draw exactly what the same path draws with the \
         position added to its points"
    );
}

// ---- plan-172-G: a group's transform -----------------------------------------------
//
// A group node is `T(position) . transform`, and nested nodes compose, the outer one
// last. `__canvas_composeGroup` is MFBASIC inside the package, so the composition rule
// is proved by where it puts a pixel: each case below works the expected place out by
// hand.

const WHITE_BOX: &str = "canvas::Rectangle[id := \"\", position := vector::Float2[x := 50.0, y := 0.0 - 10.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 60.0, height := 20.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]";
const QUARTER: &str =
    "canvas::Transform[a := 0.0, b := 1.0, c := 0.0 - 1.0, d := 0.0, tx := 0.0, ty := 0.0]";

/// A group turned a quarter turn about its `position`.
///
/// The group holds one box from (50, -10) to (110, 10) in its own space. A quarter turn
/// carries a point (x, y) to (-y, x), so the box becomes (-10, 50)..(10, 110), and the
/// node's position (400, 300) puts it at (390, 350)..(410, 410). Unturned it would be
/// at (450, 290)..(510, 310); turned about the surface origin it would be off the
/// surface entirely.
#[test]
fn group_affine_a_group_turns_its_items_about_its_position() {
    let (frame, _) = render(
        "canvas_group_turn",
        &scene(&format!(
            "  canvas::setGroup(\"g\", [{WHITE_BOX}])\n  \
             canvas::present([canvas::Group[id := \"\", position := vector::Float2[x := 400.0, y := 300.0], transform := {QUARTER}, name := \"g\"]])\n"
        )),
    );
    let white = (255, 255, 255, 255);
    let black = (0, 0, 0, 255);
    assert_eq!(
        pixel(&frame, 400, 380),
        white,
        "the middle of the turned box"
    );
    assert_eq!(pixel(&frame, 392, 352), white, "a corner of the turned box");
    assert_eq!(pixel(&frame, 408, 408), white, "the opposite corner");
    assert_eq!(pixel(&frame, 480, 300), black, "where the box is unturned");
    assert_eq!(
        pixel(&frame, 400, 300),
        black,
        "the group's position is not in the box"
    );
    assert_eq!(
        pixel(&frame, 414, 380),
        black,
        "just outside the turned box"
    );
    let lit = frame.chunks_exact(4).filter(|p| p[0] == 255).count();
    assert_eq!(lit, 20 * 60, "the turned box is still 20 by 60 pixels");
}

/// A nested group composes with its parent, the outer node last.
///
/// The inner node moves its box by (30, 0) and does not turn; the outer node turns a
/// quarter turn and sits at (100, 50). A point p of the box reaches the surface at
/// `outer(inner(p)) = turn(p + (30, 0)) + (100, 50)`. The box (50, -10)..(110, 10)
/// becomes (80, -10)..(140, 10), then (-10, 80)..(10, 140), then (90, 130)..(110, 190).
///
/// Composing the other way round — `inner(outer(p))` — would add the (30, 0) after the
/// turn and put the box at (120, 100)..(140, 160).
#[test]
fn group_affine_a_nested_group_composes_with_its_parent() {
    let (frame, _) = render(
        "canvas_group_nested",
        &scene(&format!(
            "  canvas::setGroup(\"leaf\", [{WHITE_BOX}])\n  \
             canvas::setGroup(\"outer\", [canvas::Group[id := \"\", position := vector::Float2[x := 30.0, y := 0.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"leaf\"]])\n  \
             canvas::present([canvas::Group[id := \"\", position := vector::Float2[x := 100.0, y := 50.0], transform := {QUARTER}, name := \"outer\"]])\n"
        )),
    );
    let white = (255, 255, 255, 255);
    let black = (0, 0, 0, 255);
    assert_eq!(pixel(&frame, 100, 160), white, "the middle of the box");
    assert_eq!(pixel(&frame, 92, 132), white, "its top-left corner");
    assert_eq!(pixel(&frame, 108, 188), white, "its bottom-right corner");
    assert_eq!(
        pixel(&frame, 130, 150),
        black,
        "where composing in the wrong order would put it"
    );
    let lit = frame.chunks_exact(4).filter(|p| p[0] == 255).count();
    assert_eq!(lit, 20 * 60, "one box, 20 by 60 pixels");
}

/// A group's scale scales the strokes of what it holds.
///
/// A horizontal line 100 px long with a 4 px butt-capped stroke, in a group scaled
/// 2 x at (400, 300): it is drawn 200 px long and 8 px thick, from x = 300 to 500 and
/// y = 296 to 304. An 8 px band over whole pixel rows is exactly eight white rows.
#[test]
fn group_affine_a_scaled_group_scales_its_strokes() {
    let (frame, _) = render(
        "canvas_group_scale",
        &scene(
            "  LET bar AS canvas::DrawItem = canvas::Line[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], startAt := vector::Float2[x := 0.0 - 50.0, y := 0.0], endAt := vector::Float2[x := 50.0, y := 0.0], paint := WITH canvas::stroke(color::rgb(255, 255, 255), 4.0) { cap := canvas::CapStyle.Butt }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::setGroup(\"g\", [bar])\n  \
             canvas::present([canvas::Group[id := \"\", position := vector::Float2[x := 400.0, y := 300.0], transform := canvas::Transform[a := 2.0, b := 0.0, c := 0.0, d := 2.0, tx := 0.0, ty := 0.0], name := \"g\"]])\n",
        ),
    );
    let white = (255, 255, 255, 255);
    let black = (0, 0, 0, 255);
    let column: Vec<usize> = (0..HEIGHT)
        .filter(|&y| pixel(&frame, 400, y) == white)
        .collect();
    assert_eq!(
        column,
        (296..304).collect::<Vec<_>>(),
        "a 4 px stroke in a group scaled 2 x is the eight rows 296..=303"
    );
    assert_eq!(
        pixel(&frame, 302, 300),
        white,
        "2 px inside the scaled line's left end"
    );
    assert_eq!(pixel(&frame, 497, 300), white, "3 px inside its right end");
    assert_eq!(
        pixel(&frame, 297, 300),
        black,
        "past the left end: a butt cap"
    );
    assert_eq!(pixel(&frame, 502, 300), black, "past the right end");
}

/// A group transform with no area is not in effect, exactly as an item's is: the group
/// is drawn at its `position`, unturned. `[1, 2, 2, 4]` has determinant zero. Compared
/// whole-frame against the all-zero spelling.
#[test]
fn group_affine_a_transform_with_no_area_leaves_the_group_as_built() {
    let body = |transform: &str| {
        scene(&format!(
            "  canvas::setGroup(\"g\", [{WHITE_BOX}])\n  \
             canvas::present([canvas::Group[id := \"\", position := vector::Float2[x := 400.0, y := 300.0], transform := {transform}, name := \"g\"]])\n"
        ))
    };
    let (plain, _) = render(
        "canvas_group_plain",
        &body("canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]"),
    );
    let (singular, _) = render(
        "canvas_group_singular",
        &body("canvas::Transform[a := 1.0, b := 2.0, c := 2.0, d := 4.0, tx := 0.0, ty := 0.0]"),
    );
    assert_eq!(
        pixel(&plain, 480, 300),
        (255, 255, 255, 255),
        "the box is where the group's position puts it"
    );
    assert!(
        singular == plain,
        "a no-area group transform must draw the group as an all-zero one does"
    );
}

/// A group's `transform.tx` / `ty` add to its `position`, as an item's do: the same
/// turned group, 60 px right and 40 px down.
#[test]
fn group_affine_a_transforms_translation_adds_to_the_position() {
    let (frame, _) = render(
        "canvas_group_translation",
        &scene(&format!(
            "  canvas::setGroup(\"g\", [{WHITE_BOX}])\n  \
             canvas::present([canvas::Group[id := \"\", position := vector::Float2[x := 400.0, y := 300.0], transform := canvas::Transform[a := 0.0, b := 1.0, c := 0.0 - 1.0, d := 0.0, tx := 60.0, ty := 40.0], name := \"g\"]])\n"
        )),
    );
    assert_eq!(
        pixel(&frame, 460, 420),
        (255, 255, 255, 255),
        "the turned box's middle, moved by (60, 40)"
    );
    assert_eq!(
        pixel(&frame, 400, 380),
        (0, 0, 0, 255),
        "where it would be without the translation"
    );
}

/// A `Float` literal as MFBASIC spells it: a negative one as `(0.0 - n)`.
fn lit(v: f64) -> String {
    if v < 0.0 {
        format!("(0.0 - {:?})", -v)
    } else {
        format!("{v:?}")
    }
}
