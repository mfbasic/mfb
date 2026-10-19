//! plan-176-D: `rt_canvas_curves`'s pictures clipped by curves: the clip tolerance,
//! holes and outline, the true extent, clip modes, and a picture on a poly curve. The
//! scene helpers live in the root, `rt_canvas_curves.rs`.

use super::*;

// ---- plan-164-B Phase 4: a `Picture` cut to a `CurvePolygon` -------------------------

const RED: (u8, u8, u8, u8) = (255, 0, 0, 255);

/// `RES name AS canvas::Image = …`: an image of `w` x `h` opaque pixels, row by row.
fn image(name: &str, w: usize, h: usize, px: &[(u8, u8, u8)]) -> String {
    assert_eq!(px.len(), w * h);
    let bytes: Vec<String> = px
        .iter()
        .flat_map(|&(r, g, b)| [r, g, b, 255])
        .map(|v| format!("toByte({v})"))
        .collect();
    format!(
        "  RES {name} AS canvas::Image = canvas::createImage({w}, {h}, [{}])\n",
        bytes.join(", ")
    )
}

/// The coverage a white-on-black pixel shows, 0 to 1: the software renderer blends in
/// linear light, so a channel `v` is the sRGB encoding of the coverage.
fn coverage_of(v: u8) -> f64 {
    let c = v as f64 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// The largest distance between the edge a white picture's clip draws and the true
/// circle of radius `r` about `(cx, cy)`, in surface pixels, read off the frame: an
/// edge pixel's coverage is `0.5 - d` for its centre's signed distance `d` to the
/// clip's edge, so `d` is `0.5 - coverage`, and the true circle's is the centre's
/// distance less `r`. Only pixels between 15% and 85% covered are read — outside that
/// a byte of the frame is a larger step of distance than the gap being measured.
fn edge_gap(run: &Scene, cx: f64, cy: f64, r: f64) -> (f64, usize) {
    let mut worst: f64 = 0.0;
    let mut read = 0;
    let reach = r as i32 + 3;
    for dy in -reach..=reach {
        for dx in -reach..=reach {
            let (x, y) = ((cx as i32 + dx) as usize, (cy as i32 + dy) as usize);
            let c = coverage_of(run.pixel(x, y).0);
            if !(0.15..=0.85).contains(&c) {
                continue;
            }
            let true_d = ((x as f64 + 0.5 - cx).powi(2) + (y as f64 + 0.5 - cy).powi(2)).sqrt() - r;
            worst = worst.max(((0.5 - c) - true_d).abs());
            read += 1;
        }
    }
    (worst, read)
}

/// The image's clip is the curve flattened, and the gap between that clip's edge and
/// the true curve is within the reduction's tolerance, 0.05 surface pixels — under the
/// identity, and under a x8 `transform`, where the shape is a circle of radius 7.5 and
/// the flattening has to be eight times finer in its own space to stay there.
///
/// Beside each, the same white picture on a `canvas::Circle`, whose outline is
/// flattened by the package's own ring: the two edges are each within 0.05 px of the
/// circle, so no pixel of one differs from the other by more than a tenth of a pixel's
/// coverage, and none away from the edge differs at all.
#[test]
fn canvas_picture_curve_clip_is_within_the_tolerance_of_the_curve() {
    let white = [(255, 255, 255)];
    let body = image("img", 1, 1, &white)
        + r#"  LET plain AS canvas::Paint = canvas::fill(color::rgb(255, 255, 255))
  LET big AS canvas::Transform = canvas::Transform[8.0, 0.0, 0.0, 8.0, 0.0, 0.0]
  LET disc AS canvas::AutoShape = canvas::Circle[id := "", position := at(150.5, 150.25), radius := 60.0, paint := plain, transform := none()]
  canvas::present([canvas::Picture[id := "", shape := disc, clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img], canvas::Picture[id := "", shape := curved("", 450.5, 150.25, [circle(0.0, 0.0, 60.0)], canvas::FillRule.EvenOdd, plain, none()), clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img], canvas::Picture[id := "", shape := curved("", 150.5, 450.25, [circle(0.0, 0.0, 7.5)], canvas::FillRule.EvenOdd, plain, big), clip := canvas::PictureClip.ClipAndTransform, fit := canvas::PictureFit.Stretch, image := img], canvas::Picture[id := "", shape := curved("", 450.5, 450.25, [circle(0.0, 0.0, 7.5)], canvas::FillRule.EvenOdd, plain, big), clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img]])
"#;
    let run = scene("canvas_picture_curve_gap", &body, "", &[]);
    assert_eq!(run.pixel(450, 150), WHITE, "the middle of the picture");
    for (what, cx, cy) in [
        ("the identity", 450.5, 150.25),
        ("x8, ClipAndTransform", 150.5, 450.25),
        ("x8, ClipOnly", 450.5, 450.25),
    ] {
        let (gap, read) = edge_gap(&run, cx, cy, 60.0);
        println!("picture clip vs the true circle, {what}: worst gap {gap:.4} px over {read} edge pixels");
        assert!(read > 150, "{what}: the edge was read ({read} pixels)");
        // 0.05 is the flattening's own bound; the rest is the frame's resolution of a
        // distance, one byte of an sRGB channel at mid coverage.
        assert!(
            gap <= 0.05 + 0.006,
            "{what}: the clip is {gap} px from the curve"
        );
    }
    let mut worst: f64 = 0.0;
    for y in 80..221 {
        for x in 80..221 {
            let (a, b) = (run.pixel(x, y), run.pixel(x + 300, y));
            let delta = (coverage_of(a.0) - coverage_of(b.0)).abs();
            worst = worst.max(delta);
            let from_edge =
                (((x as f64 + 0.5 - 150.5).powi(2) + (y as f64 + 0.5 - 150.25).powi(2)).sqrt()
                    - 60.0)
                    .abs();
            if from_edge > 1.0 {
                assert_eq!(a, b, "({x}, {y}) is {from_edge:.2} px from the edge");
            }
        }
    }
    println!(
        "picture on a CurvePolygon circle vs on a Circle: worst coverage difference {worst:.4}"
    );
    assert!(
        worst <= 0.1,
        "the two outlines are {worst} of a pixel apart"
    );
}

/// A picture on a two-contour `CurvePolygon` shows its image in the ring and not in
/// the hole, under both rules (the inner contour is wound against the outer, which is
/// a hole under both). And a stroked shape draws its outline over the image as the
/// curve: at (600, 150), radius 60 with a 6 px white stroke over a red image — the
/// outline is white on the radius, the image red 5 px inside it, nothing 5 px outside.
#[test]
fn canvas_picture_curve_holes_and_outline() {
    let body = image("img", 1, 1, &[(255, 0, 0)])
        + r#"  LET plain AS canvas::Paint = canvas::fill(color::rgb(255, 255, 255))
  LET ring AS List OF List OF canvas::BezierCurve = [circle(0.0, 0.0, 80.0), circleBack(0.0, 0.0, 35.0)]
  LET edged AS canvas::Paint = canvas::fillStroke(color::rgb(255, 255, 255), color::rgb(255, 255, 255), 6.0)
  canvas::present([canvas::Picture[id := "", shape := curved("", 150.0, 150.0, ring, canvas::FillRule.EvenOdd, plain, none()), clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img], canvas::Picture[id := "", shape := curved("", 380.0, 150.0, ring, canvas::FillRule.NonZero, plain, none()), clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img], canvas::Picture[id := "", shape := curved("", 600.0, 150.0, [circle(0.0, 0.0, 60.0)], canvas::FillRule.EvenOdd, edged, none()), clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img]])
"#;
    let run = scene("canvas_picture_curve_holes", &body, "", &[]);
    for (what, cx) in [("EvenOdd", 150), ("NonZero", 380)] {
        assert_eq!(run.pixel(cx + 55, 150), RED, "{what}: the ring");
        assert_eq!(run.pixel(cx, 150 - 55), RED, "{what}: the ring, above");
        assert_eq!(run.pixel(cx, 150), BLACK, "{what}: the hole");
        assert_eq!(
            run.pixel(cx + 20, 150 + 20),
            BLACK,
            "{what}: the hole again"
        );
        assert_eq!(run.pixel(cx + 84, 150), BLACK, "{what}: outside");
    }
    assert_eq!(
        run.pixel(600 + 59, 150),
        WHITE,
        "the outline, on the radius"
    );
    assert_eq!(
        run.pixel(600 + 61, 150),
        WHITE,
        "the outline, just outside it"
    );
    assert_eq!(run.pixel(600 + 54, 150), RED, "the image, 5.5 px inside");
    assert_eq!(run.pixel(600 + 65, 150), BLACK, "5.5 px outside");
    assert_eq!(run.pixel(600, 150), RED, "the middle");
}

/// The box an image is fitted to is the curve's TRUE extent, not its control points'.
///
/// A bump: one quadratic from (0, 0) through a control point at (100, -160) to
/// (200, 0), closed along y = 0. The curve only reaches y = -80; its control point is
/// twice as far. The image is 1 x 2, red over blue.
///
/// * `Stretch` (at x = 50): the box is 200 x 80, the seam at y = -40. Local (100, -60)
///   is red and (100, -20) blue. Fitted to the control points' box (200 x 160) the
///   seam would be at -80 and (100, -60) blue.
/// * `Contain` (at x = 330): the image keeps 1:2, so it is 40 x 80, centred on x = 100:
///   (100, -60) red, (100, -20) blue, and (60, -20) — inside the bump, outside the
///   image — nothing.
/// * `Cover` (at x = 610): the image is 200 x 400 centred on the box, so the seam is
///   still at -40: (100, -60) red, (100, -20) blue.
#[test]
fn canvas_picture_curve_fits_the_true_extent() {
    let body = image("img", 1, 2, &[(255, 0, 0), (0, 0, 255)])
        + r#"  LET plain AS canvas::Paint = canvas::fill(color::rgb(255, 255, 255))
  LET bump AS List OF List OF canvas::BezierCurve = [[canvas::BezierCurve[points := [at(0.0, 0.0), at(100.0, 0.0 - 160.0), at(200.0, 0.0)], weights := []]]]
  canvas::present([canvas::Picture[id := "", shape := curved("", 50.0, 300.0, bump, canvas::FillRule.EvenOdd, plain, none()), clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img], canvas::Picture[id := "", shape := curved("", 330.0, 300.0, bump, canvas::FillRule.EvenOdd, plain, none()), clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Contain, image := img], canvas::Picture[id := "", shape := curved("", 610.0, 300.0, bump, canvas::FillRule.EvenOdd, plain, none()), clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Cover, image := img]])
"#;
    let run = scene("canvas_picture_curve_fit", &body, "", &[]);
    for (what, x) in [("Stretch", 50), ("Contain", 330), ("Cover", 610)] {
        assert_eq!(run.pixel(x + 100, 300 - 60), RED, "{what}: above the seam");
        assert_eq!(run.pixel(x + 100, 300 - 20), BLUE, "{what}: below it");
        assert_eq!(
            run.pixel(x + 100, 300 - 42),
            RED,
            "{what}: 1.5 px above the seam"
        );
        assert_eq!(
            run.pixel(x + 100, 300 - 39),
            BLUE,
            "{what}: 1.5 px below it"
        );
        assert_eq!(
            run.pixel(x + 100, 300 - 84),
            BLACK,
            "{what}: above the bump"
        );
        assert_eq!(
            run.pixel(x + 100, 300 + 3),
            BLACK,
            "{what}: below the closing segment"
        );
    }
    assert_eq!(
        run.pixel(50 + 60, 300 - 20),
        BLUE,
        "Stretch: the image fills the bump"
    );
    assert_eq!(
        run.pixel(330 + 60, 300 - 20),
        BLACK,
        "Contain: inside the bump, outside the image"
    );
    assert_eq!(
        run.pixel(610 + 60, 300 - 20),
        BLUE,
        "Cover: the image covers the bump"
    );
}

/// `ClipAndTransform` turns the image with the shape; `ClipOnly` turns only the outline
/// and leaves the image upright, fitted to the box of the turned outline. The bump,
/// turned 30° about its `position`, with a 2 x 1 image, red beside blue.
///
/// * `ClipAndTransform` (at (150, 200)): the seam is local x = 100. Local (90, -60)
///   lands at (107.9, -7.0) and is red; local (110, -60) at (125.3, 3.0), blue.
/// * `ClipOnly` (at (500, 200)): the turned outline spans x from 0 to 173.2, so the
///   seam is the upright line x = 86.6. The same point, (107.9, -7.0), is blue; local
///   (30, -10), at (31.0, 6.3), is red.
///
/// And the shape's `paint.clip`, which keeps local x from 100 on (at (150, 450)), cuts
/// the picture as well as the outline does.
#[test]
fn canvas_picture_curve_clip_modes_and_the_paints_clip() {
    let body = image("img", 2, 1, &[(255, 0, 0), (0, 0, 255)])
        + r#"  LET plain AS canvas::Paint = canvas::fill(color::rgb(255, 255, 255))
  LET turn AS canvas::Transform = canvas::Transform[0.8660254037844386, 0.5, 0.0 - 0.5, 0.8660254037844386, 0.0, 0.0]
  LET bump AS List OF List OF canvas::BezierCurve = [[canvas::BezierCurve[points := [at(0.0, 0.0), at(100.0, 0.0 - 160.0), at(200.0, 0.0)], weights := []]]]
  LET cut AS canvas::Paint = WITH plain { clip := [[at(100.0, 0.0 - 200.0), at(300.0, 0.0 - 200.0), at(300.0, 200.0), at(100.0, 200.0)]] }
  canvas::present([canvas::Picture[id := "", shape := curved("", 150.0, 200.0, bump, canvas::FillRule.EvenOdd, plain, turn), clip := canvas::PictureClip.ClipAndTransform, fit := canvas::PictureFit.Stretch, image := img], canvas::Picture[id := "", shape := curved("", 500.0, 200.0, bump, canvas::FillRule.EvenOdd, plain, turn), clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img], canvas::Picture[id := "", shape := curved("", 150.0, 450.0, bump, canvas::FillRule.EvenOdd, cut, none()), clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img]])
"#;
    let run = scene("canvas_picture_curve_modes", &body, "", &[]);
    assert_eq!(
        run.pixel(150 + 107, 200 - 7),
        RED,
        "ClipAndTransform: local (90, -60)"
    );
    assert_eq!(
        run.pixel(150 + 125, 200 + 3),
        BLUE,
        "ClipAndTransform: local (110, -60)"
    );
    assert_eq!(
        run.pixel(150 + 31, 200 + 6),
        RED,
        "ClipAndTransform: local (30, -10)"
    );
    assert_eq!(
        run.pixel(500 + 107, 200 - 7),
        BLUE,
        "ClipOnly: right of the upright seam"
    );
    assert_eq!(run.pixel(500 + 31, 200 + 6), RED, "ClipOnly: left of it");
    // The unturned bump's top is local (100, -80), which is not on the turned one:
    // it is turned-local (46.6, -119).
    assert_eq!(
        run.pixel(150 + 100, 200 - 75),
        BLACK,
        "the outline is turned"
    );
    assert_eq!(run.pixel(500 + 100, 200 - 75), BLACK, "under both modes");
    // The paint's clip: the image is fitted to the whole bump (seam at x = 100), and
    // only local x >= 100 is left.
    assert_eq!(
        run.pixel(150 + 140, 450 - 30),
        BLUE,
        "right of the paint's clip"
    );
    assert_eq!(run.pixel(150 + 60, 450 - 30), BLACK, "left of it: cut away");
    assert_eq!(
        run.pixel(150 + 98, 450 - 30),
        BLACK,
        "1.5 px left of the clip's edge"
    );
    assert_eq!(run.pixel(150 + 101, 450 - 30), BLUE, "1.5 px right of it");
}

/// A picture on a `PolyCurve` draws nothing and is hit by nothing: a chain has no
/// inside to cut an image to, as a `PolyLine` has none. The frame is the square
/// beneath it alone, and a press where the chain would be reports the square.
#[cfg(unix)]
#[test]
fn canvas_picture_on_a_poly_curve_draws_nothing() {
    let body = image("img", 1, 1, &[(0, 255, 0)])
        + r#"  MUT under AS canvas::Rectangle
  under = WITH under { id := "under", position := at(100.0, 100.0), size := canvas::Size[width := 200.0, height := 200.0], paint := canvas::fill(color::rgb(255, 0, 0)) }
  LET underItem AS canvas::DrawItem = under
  LET chain AS canvas::AutoShape = canvas::PolyCurve[id := "chain", position := at(100.0, 200.0), curves := [line(0.0, 0.0, 200.0, 0.0)], paint := canvas::stroke(color::rgb(0, 0, 255), 40.0), transform := none()]
  canvas::present([underItem, canvas::Picture[id := "pic", shape := chain, clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img]])
  report("over")
"#;
    let run = scene("canvas_picture_poly_curve", &body, "", &[(200, 200)]);
    assert_eq!(run.over(), ["under"]);
    assert_eq!(run.pixel(200, 200), RED);
    let other = run
        .frame
        .chunks(4)
        .filter(|p| *p != [0, 0, 0, 255] && *p != [255, 0, 0, 255])
        .count();
    assert_eq!(
        other, 0,
        "nothing but the square and the background is drawn"
    );
}
