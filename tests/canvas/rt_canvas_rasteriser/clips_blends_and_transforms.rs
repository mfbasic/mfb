//! plan-176-D: `rt_canvas_rasteriser`'s rectangle and path clips, transparency and
//! blend modes, transforms, and the `#[ignore]`d transformed-distance measurement. The
//! render helpers live in the root, `rt_canvas_rasteriser.rs`.

use super::*;

/// A clip cuts a circle in half, leaving the other half untouched.
///
/// The circle is the right shape to clip with, because a clip that were secretly
/// implemented as "shrink the bounds" would still produce a half circle here — but the
/// *curved* boundary would be wrong if the clip and the shape did not compose. Sampling
/// on both sides of the clip edge at the same height is what tells the two apart.
#[test]
fn a_clip_cuts_a_circle_in_half() {
    let (frame, _) = render(
        "canvas_clip_half",
        &scene(
            "  LET c AS color::Color = color::rgb(255, 0, 0)\n  \
             LET p AS canvas::Paint = WITH canvas::fill(c) { clip := [[vector::Float2[x := 0.0 - 200.0, y := 0.0 - 200.0], vector::Float2[x := 0.0, y := 0.0 - 200.0], vector::Float2[x := 0.0, y := 440.0], vector::Float2[x := 0.0 - 200.0, y := 440.0]]] }\n  \
             LET dot AS canvas::DrawItem = canvas::Circle[id := \"\", position := vector::Float2[x := 200.0, y := 200.0], radius := 100.0, paint := p, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([dot])\n",
        ),
    );

    // Inside the clip and inside the circle.
    assert_eq!(
        pixel(&frame, 150, 200),
        (255, 0, 0, 255),
        "left of the clip edge and well inside the circle: must be painted"
    );
    // Outside the clip, but still inside the circle — the half that must be cut.
    assert_eq!(
        pixel(&frame, 250, 200),
        (0, 0, 0, 255),
        "right of the clip edge: inside the circle, so only the clip can have removed it"
    );
    // The circle's own curved edge still bounds it inside the clip.
    assert_eq!(
        pixel(&frame, 150, 60),
        (0, 0, 0, 255),
        "inside the clip but ABOVE the circle: the clip must not have widened the shape"
    );
}

/// A clip on a fractional pixel boundary antialiases its own edge.
///
/// The clip starts at x = 100.25, so pixel 100 (centre 100.5) is 75% inside and pixel
/// 99 is wholly outside. That is the whole reason the clip is a coverage multiply and
/// not a pixel-index comparison: a `x >= 100` test would paint pixel 100 fully and
/// there would be a hard, aliased edge where the shape's own edges are smooth.
#[test]
fn a_fractional_clip_edge_is_antialiased() {
    let (frame, _) = render(
        "canvas_clip_frac",
        &scene(
            "  LET c AS color::Color = color::rgb(255, 255, 255)\n  \
             LET p AS canvas::Paint = WITH canvas::fill(c) { clip := [[vector::Float2[x := 100.25, y := 0.0], vector::Float2[x := 400.25, y := 0.0], vector::Float2[x := 400.25, y := 640.0], vector::Float2[x := 100.25, y := 640.0]]] }\n  \
             LET box AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 500.0, height := 200.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := p, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([box])\n",
        ),
    );

    let (r, _, _, _) = pixel(&frame, 100, 100);
    assert!(
        r > 0 && r < 255,
        "pixel 100 straddles a clip edge at x = 100.25, so it must be PARTIALLY \
         covered — got {r}, which is {}",
        if r == 0 {
            "fully clipped"
        } else {
            "fully painted"
        }
    );
    assert_eq!(
        pixel(&frame, 99, 100),
        (0, 0, 0, 255),
        "pixel 99 is wholly left of the clip"
    );
    assert_eq!(
        pixel(&frame, 101, 100),
        (255, 255, 255, 255),
        "pixel 101 is wholly inside the clip"
    );
}

/// The empty ring list means "no clip" and renders identically to an unset one, while a
/// clip that encloses nothing — here a rectangle of zero area — hides the item.
///
/// The first half is the compatibility case, asserted as whole-frame byte equality
/// rather than by sampling: every `Paint` built without a clip carries `[]`, so a single
/// wrong pixel is every existing scene changing. The second is plan-163-C's decision
/// (2026-09-28, §4): only the empty list is the inert zero value, and a clip the program
/// wrote that encloses nothing is visible as such — it replaced plan-116-B's "a zero-area
/// `Bounds` is unclipped", which had no other way to spell "no clip".
#[test]
fn an_empty_clip_is_no_clip_and_a_clip_enclosing_nothing_hides_the_item() {
    let body = |clip: &str| {
        scene(&format!(
            "  LET c AS color::Color = color::rgb(0, 200, 255)\n  \
             LET p AS canvas::Paint = {clip}\n  \
             LET dot AS canvas::DrawItem = canvas::Circle[id := \"\", position := vector::Float2[x := 300.0, y := 300.0], radius := 120.0, paint := p, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([dot])\n"
        ))
    };
    let (unset, _) = render("canvas_clip_unset", &body("canvas::fill(c)"));
    let (empty, _) = render(
        "canvas_clip_empty",
        &body("WITH canvas::fill(c) { clip := [] }"),
    );
    assert_eq!(
        unset, empty,
        "an empty clip must render byte-identically to no clip at all"
    );
    let (zero, _) = render(
        "canvas_clip_zero",
        &body(
            "WITH canvas::fill(c) { clip := [[vector::Float2[x := 300.0, y := 300.0], vector::Float2[x := 300.0, y := 300.0], vector::Float2[x := 300.0, y := 300.0], vector::Float2[x := 300.0, y := 300.0]]] }",
        ),
    );
    for (x, y) in [(300usize, 300usize), (299, 299), (250, 300)] {
        assert_eq!(
            pixel(&zero, x, y),
            (0, 0, 0, 255),
            "a zero-area clip encloses nothing, so the circle at ({x}, {y}) is hidden"
        );
    }
}

/// plan-163-C: `Paint.transparency` fades an item's alphas at geometry build.
///
/// `T = round(clamp(t, 0, 1) * 255)` and each alpha becomes `(a * (255 - T) + 127) / 255`,
/// so an item at transparency `t` draws exactly as the same item with those alphas
/// spelled out. Hand-computed: `t = 0.5` gives `T = round(127.5) = 128`, so an opaque
/// fill, stroke and gradient stop become `(255 * 127 + 127) / 255 = 127`, and a stop of
/// alpha 200 becomes `(200 * 127 + 127) / 255 = 100`. `t = -1` clamps to 0 (opaque, the
/// identity) and `t = 2` to 1 (`T = 255`: every alpha `127 / 255 = 0`, nothing drawn).
#[test]
fn transparency_fades_fill_stroke_and_gradient_alphas() {
    let body = |faded: bool| {
        let (fill, stroke, s0, s1, t) = if faded {
            (
                "color::rgb(255, 0, 0)",
                "color::rgb(0, 255, 0)",
                "color::rgb(0, 0, 255)",
                "color::rgba(255, 255, 0, 200)",
                " transparency := 0.5,",
            )
        } else {
            (
                "color::rgba(255, 0, 0, 127)",
                "color::rgba(0, 255, 0, 127)",
                "color::rgba(0, 0, 255, 127)",
                "color::rgba(255, 255, 0, 100)",
                "",
            )
        };
        scene(&format!(
            "  LET ramp AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, startPoint := vector::Float2[x := 0.0, y := 0.0 - 50.0], endPoint := vector::Float2[x := 200.0, y := 0.0 - 50.0], \
             stops := [canvas::GradientStop[offset := 0.0, color := {s0}], canvas::GradientStop[offset := 1.0, color := {s1}]], spread := canvas::GradientSpread.Pad, transform := canvas::Transform[a := 0.0, b := 0.0, c := 0.0, d := 0.0, tx := 0.0, ty := 0.0], focalOffset := vector::Float2[x := 0.0, y := 0.0], focalRadius := 0.0, strokeApply := canvas::GradientApply.FillOnly]\n  \
             LET a AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 50.0, y := 50.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 200.0, height := 150.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH canvas::fillStroke({fill}, {stroke}, 12.0) {{{t} blend := canvas::BlendMode.Normal }}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET b AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 400.0, y := 50.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 200.0, height := 150.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH canvas::fill(color::rgb(9, 9, 9)) {{{t} gradient := ramp }}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([a, b])\n"
        ))
    };
    let (faded, _) = render("canvas_transparency", &body(true));
    let (spelled, _) = render("canvas_transparency_spelled", &body(false));
    assert!(
        faded == spelled,
        "transparency 0.5 must draw exactly as alpha 127 (and a 200 stop as 100)"
    );
    assert_ne!(
        pixel(&faded, 150, 120),
        (0, 0, 0, 255),
        "the faded fill still draws"
    );

    let item = |t: &str| {
        scene(&format!(
            "  LET a AS canvas::DrawItem = canvas::Circle[id := \"\", position := vector::Float2[x := 300.0, y := 300.0], radius := 100.0, paint := WITH canvas::fill(color::rgb(255, 0, 0)) {{ transparency := {t} }}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([a])\n"
        ))
    };
    let (opaque, _) = render("canvas_transparency_zero", &item("0.0"));
    let (low, _) = render("canvas_transparency_low", &item("0.0 - 1.0"));
    let (high, _) = render("canvas_transparency_high", &item("2.0"));
    assert!(
        low == opaque,
        "transparency -1 clamps to 0, the opaque identity"
    );
    assert_eq!(
        pixel(&opaque, 300, 300),
        (255, 0, 0, 255),
        "transparency 0 is opaque"
    );
    assert_eq!(
        pixel(&high, 300, 300),
        (0, 0, 0, 255),
        "transparency 2 clamps to 1: nothing drawn"
    );
}

/// A clip ring from points, as MFBASIC source.
fn clip_ring(points: &[(f64, f64)]) -> String {
    let pts: Vec<String> = points
        .iter()
        .map(|(x, y)| format!("vector::Float2[x := {x:?}, y := {y:?}]"))
        .collect();
    format!("[{}]", pts.join(", "))
}

/// plan-163-C: a path clip — any clip but one untransformed axis-aligned rectangle — in
/// software, every expectation a whole pixel inside or outside each edge.
///
/// * A white 900x640 ground clipped to the triangle (100, 100), (300, 100), (100, 300):
///   (150, 150) is inside, (250, 250) beyond the hypotenuse.
/// * A ring with a hole — (400, 50)-(600, 250) around (450, 100)-(550, 200), both
///   clockwise: under `EvenOdd` the hole (500, 150) is clipped away and the band
///   (420, 150) drawn; under `NonZero` the same-direction inner ring is more of the clip,
///   so (500, 150) is drawn.
/// * A 100x100 square centred on its own origin, rotated 30° and moved to (700, 450),
///   clipped to its own left half (x < 0): its point (-25, 0) lands at
///   (700 - 21.65, 450 - 12.5) and is drawn, (25, 0) at (721.65, 462.5) is not.
/// * A path clip inside a group translated by (300, 400): a 100x100 square at the origin
///   clipped to the triangle (0, 0), (100, 0), (0, 100) draws (320, 420) and not (380, 480).
#[test]
fn a_path_clip_draws_only_inside_its_rings() {
    let tri = clip_ring(&[(100.0, 100.0), (300.0, 100.0), (100.0, 300.0)]);
    let outer = clip_ring(&[(20.0, 50.0), (220.0, 50.0), (220.0, 250.0), (20.0, 250.0)]);
    let inner = clip_ring(&[(70.0, 100.0), (170.0, 100.0), (170.0, 200.0), (70.0, 200.0)]);
    let half = clip_ring(&[
        (0.0, 0.0),
        (50.0, 0.0),
        (50.0, 100.0),
        (0.0, 100.0),
        (0.0, 50.0),
    ]);
    let corner = clip_ring(&[(0.0, 0.0), (100.0, 0.0), (0.0, 100.0)]);
    let body = |rule: &str| {
        scene(&format!(
            "  LET w AS canvas::Paint = canvas::fill(color::rgb(255, 255, 255))\n  \
             LET a AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 380.0, height := 340.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH w {{ clip := [{tri}] }}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET b AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 380.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 260.0, height := 300.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH w {{ clip := [{outer}, {inner}], clipRule := canvas::FillRule.{rule} }}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET spin AS canvas::Transform = canvas::Transform[a := 0.8660254037844387, b := 0.5, c := 0.0 - 0.5, d := 0.8660254037844387, tx := 775.0 - 50.0 * 0.8660254037844387, ty := 475.0 - 50.0 * 0.8660254037844387]\n  \
             LET c AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0 - 50.0, y := 0.0 - 50.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 100.0, height := 100.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH w {{ clip := [{half}] }}, transform := spin]\n  \
             LET d AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 100.0, height := 100.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH w {{ clip := [{corner}] }}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::setGroup(\"g\", [d])\n  \
             canvas::present([a, b, c, canvas::Group[id := \"\", position := vector::Float2[x := 300.0, y := 400.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"g\"]])\n"
        ))
    };
    let white = (255, 255, 255, 255);
    let black = (0, 0, 0, 255);
    let (even, _) = render("canvas_path_clip", &body("EvenOdd"));
    assert_eq!(pixel(&even, 150, 150), white, "inside the triangle");
    assert_eq!(
        pixel(&even, 250, 250),
        black,
        "beyond the triangle's hypotenuse"
    );
    assert_eq!(pixel(&even, 420, 150), white, "the band between the rings");
    assert_eq!(pixel(&even, 500, 150), black, "the hole under EvenOdd");
    assert_eq!(
        pixel(&even, 678, 437),
        white,
        "the rotated square's clipped-in half"
    );
    assert_eq!(pixel(&even, 721, 462), black, "its clipped-out half");
    assert_eq!(
        pixel(&even, 320, 420),
        white,
        "the group's clip moved with it"
    );
    assert_eq!(pixel(&even, 380, 480), black, "beyond the moved triangle");
    let (non_zero, _) = render("canvas_path_clip_nz", &body("NonZero"));
    assert_eq!(
        pixel(&non_zero, 500, 150),
        white,
        "the same-direction inner ring is clip under NonZero"
    );
}

/// plan-163-C: a transformed clip's edge is antialiased exactly as a transformed shape's
/// edge — the clip distance is divided by the same direction-aware divisor.
///
/// Under the X shear `x' = x + 0.5y` (moved to (200, 450)), a 100x100 square clipped to its
/// own left half draws, along the clip's slanted edge, the very pixels the 50x100 square
/// of that half draws unclipped: the same geometry, one edge a clip and one a shape.
#[test]
fn a_sheared_clip_edge_matches_a_sheared_shape_edge() {
    let half = clip_ring(&[(0.0, 0.0), (50.0, 0.0), (50.0, 100.0), (0.0, 100.0)]);
    let body = |item: &str| {
        scene(&format!(
            "  LET shear AS canvas::Transform = canvas::Transform[a := 1.0, b := 0.0, c := 0.5, d := 1.0, tx := 200.0, ty := 450.0]\n  \
             LET w AS canvas::Paint = canvas::fill(color::rgb(255, 255, 255))\n  \
             {item}\n  \
             canvas::present([s])\n"
        ))
    };
    let (clipped, _) = render(
        "canvas_shear_clip",
        &body(&format!("LET s AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 100.0, height := 100.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH w {{ clip := [{half}] }}, transform := shear]")),
    );
    let (shape, _) = render(
        "canvas_shear_shape",
        &body("LET s AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 50.0, height := 100.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := w, transform := shear]"),
    );
    // Only around the clip's own edge, item x = 50, which lands at surface x =
    // 250 + 0.5 * (y + 0.5 - 450): the clip's left edge coincides with the shape's, where
    // the two coverages multiply and the shapes legitimately differ.
    for y in [460usize, 480, 500, 520, 540] {
        let edge = 250.0 + 0.5 * (y as f64 + 0.5 - 450.0);
        for x in (edge as usize - 6)..(edge as usize + 6) {
            assert_eq!(
                pixel(&clipped, x, y),
                pixel(&shape, x, y),
                "the clip's sheared edge differs from the shape's at ({x}, {y})"
            );
        }
    }
    assert_eq!(
        pixel(&clipped, 250, 500),
        (255, 255, 255, 255),
        "the half inside the clip draws"
    );
}

/// plan-163-C: an untransformed axis-aligned rectangle clip takes the rectangle path
/// (mode 1), and the same rectangle as a five-point ring — the corner repeated, so not a
/// four-point rectangle — takes the path clip (mode 2). They draw the same pixels, which
/// pins the fast path to the general one. A non-empty clip whose rings are all shorter
/// than two points encloses nothing and hides the item.
#[test]
fn a_rectangle_clip_draws_the_same_through_either_path() {
    let four = clip_ring(&[
        (100.25, 80.5),
        (300.75, 80.5),
        (300.75, 240.25),
        (100.25, 240.25),
    ]);
    let five = clip_ring(&[
        (100.25, 80.5),
        (300.75, 80.5),
        (300.75, 240.25),
        (100.25, 240.25),
        (100.25, 240.25),
    ]);
    let body = |clip: &str| {
        scene(&format!(
            "  LET a AS canvas::DrawItem = canvas::Circle[id := \"\", position := vector::Float2[x := 200.0, y := 160.0], radius := 120.0, paint := WITH canvas::fill(color::rgb(0, 200, 255)) {{ clip := [{clip}] }}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([a])\n"
        ))
    };
    let (rect, _) = render("canvas_clip_mode1", &body(&four));
    let (path, _) = render("canvas_clip_mode2", &body(&five));
    assert!(
        rect == path,
        "the rectangle clip and the same rectangle as a path differ"
    );
    let (none, _) = render(
        "canvas_clip_degenerate",
        &body("[vector::Float2[x := 200.0, y := 160.0]]"),
    );
    assert_eq!(
        pixel(&none, 200, 160),
        (0, 0, 0, 255),
        "a clip of one point encloses nothing"
    );
}

/// A clip entirely outside the item draws nothing.
///
/// The negative case for the loop bounds: `firstX` ends up past `lastX`, so the loop
/// body never runs. Asserted against a frame with a second, unclipped item in it, so
/// "nothing was drawn" cannot be confused with "the renderer failed".
#[test]
fn a_clip_outside_the_item_draws_nothing() {
    let (frame, _) = render(
        "canvas_clip_outside",
        &scene(
            "  LET red AS color::Color = color::rgb(255, 0, 0)\n  \
             LET green AS color::Color = color::rgb(0, 255, 0)\n  \
             LET p AS canvas::Paint = WITH canvas::fill(red) { clip := [[vector::Float2[x := 590.0, y := 390.0], vector::Float2[x := 690.0, y := 390.0], vector::Float2[x := 690.0, y := 490.0], vector::Float2[x := 590.0, y := 490.0]]] }\n  \
             LET hidden AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 10.0, y := 10.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 100.0, height := 100.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := p, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET shown AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 300.0, y := 10.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 50.0, height := 50.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(green), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([hidden, shown])\n",
        ),
    );

    assert_eq!(
        pixel(&frame, 60, 60),
        (0, 0, 0, 255),
        "the clipped rectangle's own centre: its clip is 600 px away, so nothing may be drawn"
    );
    assert_eq!(
        pixel(&frame, 320, 30),
        (0, 255, 0, 255),
        "the unclipped rectangle still drew, so the frame is not simply blank"
    );
}

/// A clip larger than the item changes nothing.
///
/// The other half of the zero-area case: a clip that contains the item must be as
/// inert as no clip, including on the item's own antialiased edges — which is why this
/// compares whole frames rather than an interior sample. A clip that quantized its
/// coverage differently from the shape would show up here and nowhere else.
#[test]
fn a_clip_larger_than_the_item_changes_nothing() {
    let body = |clip: &str| {
        scene(&format!(
            "  LET c AS color::Color = color::rgb(255, 200, 0)\n  \
             LET p AS canvas::Paint = {clip}\n  \
             LET dot AS canvas::DrawItem = canvas::Circle[id := \"\", position := vector::Float2[x := 300.0, y := 300.0], radius := 90.0, paint := p, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([dot])\n"
        ))
    };
    let (unset, _) = render("canvas_clip_big_unset", &body("canvas::fill(c)"));
    let (big, _) = render(
        "canvas_clip_big",
        &body(
            "WITH canvas::fill(c) { clip := [[vector::Float2[x := 0.0 - 300.0, y := 0.0 - 300.0], vector::Float2[x := 600.0, y := 0.0 - 300.0], vector::Float2[x := 600.0, y := 340.0], vector::Float2[x := 0.0 - 300.0, y := 340.0]]] }",
        ),
    );
    assert_eq!(
        unset, big,
        "a clip containing the whole item must render byte-identically to no clip"
    );
}

/// Each `BlendMode` composites to its own exact channel values.
///
/// One scene, four overlapping pairs, all over the **same mid-grey ground** — which is
/// what makes the four answers distinct. Over white or black they collapse: `Multiply`
/// with white is the source, `Screen` and `Add` with white are both white, and a test
/// that could not tell `Screen` from `Add` would pass with either wired to the other.
///
/// The expected values are derived from the mode definitions on **linear** values
/// (`06_canvas.md` §"Rendering conventions") against the checked-in sRGB table, not
/// read back from the renderer:
///
/// | mode | rgb(200,100,50) over rgb(128,128,128) |
/// |---|---|
/// | `Normal` | `(200, 100, 50)` — the source, unchanged |
/// | `Multiply` | `(99, 46, 20)` — darker than both |
/// | `Screen` | `(213, 152, 135)` — lighter than both |
/// | `Add` | `(230, 158, 136)` — lighter still, and distinct from `Screen` |
///
/// Asserted exactly rather than by inequality, because "darker" and "lighter" would
/// also hold for a blend that composited in sRGB space instead of linear — the very
/// mistake `translucent_fill_blends_in_linear_space` exists to catch for `Normal`.
#[test]
fn each_blend_mode_composites_to_its_own_values() {
    let over = |name: &str, mode: &str, x: f64| {
        format!(
            "  LET {name} AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := {x:.1}, y := 60.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 60.0, height := 60.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH canvas::fill(color::rgb(200, 100, 50)) {{ blend := canvas::BlendMode.{mode} }}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n"
        )
    };
    let body = format!(
        "  LET ground AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 600.0, height := 200.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(128, 128, 128)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n{}{}{}{}  \
         canvas::present([ground, normal, multiply, screen, add])\n",
        over("normal", "Normal", 20.0),
        over("multiply", "Multiply", 120.0),
        over("screen", "Screen", 220.0),
        over("add", "Add", 320.0),
    );
    let (frame, _) = render("canvas_blend_modes", &scene(&body));

    assert_eq!(
        pixel(&frame, 10, 90),
        (128, 128, 128, 255),
        "the mid-grey ground, away from every overlay"
    );
    for (mode, x, want) in [
        ("Normal", 50, (200u8, 100u8, 50u8)),
        ("Multiply", 150, (99, 46, 20)),
        ("Screen", 250, (213, 152, 135)),
        ("Add", 350, (230, 158, 136)),
    ] {
        let got = pixel(&frame, x, 90);
        assert_eq!(
            (got.0, got.1, got.2),
            want,
            "BlendMode.{mode} over mid grey: the linear-space equation for this mode \
             gives {want:?}, got {:?}",
            (got.0, got.1, got.2),
        );
    }
}

/// `BlendMode.Normal` is byte-for-byte what an unset `blend` renders.
///
/// The compatibility pair for `each_blend_mode_composites_to_its_own_values`, and the
/// reason `__canvas_blendChannelMode`'s mode-0 arm is the same *expression* as
/// `__canvas_blendChannel` rather than merely an equivalent one. `Normal` is the zero
/// value, so every `Paint` ever built carries it and every existing golden renders
/// through it — a one-step rounding drift here is every scene in the repository
/// changing at once.
///
/// Whole-frame equality, including the antialiased circle edge, where a rounding
/// difference would show up first.
#[test]
fn blend_mode_normal_is_identical_to_an_unset_blend() {
    let body = |paint: &str| {
        scene(&format!(
            "  LET ground AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 600.0, height := 400.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(128, 128, 128)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET dot AS canvas::DrawItem = canvas::Circle[id := \"\", position := vector::Float2[x := 300.0, y := 200.0], radius := 90.0, paint := {paint}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([ground, dot])\n"
        ))
    };
    let (unset, _) = render(
        "canvas_blend_unset",
        &body("canvas::fill(color::rgba(255, 200, 0, 160))"),
    );
    let (normal, _) = render(
        "canvas_blend_normal",
        &body(
            "WITH canvas::fill(color::rgba(255, 200, 0, 160)) { blend := canvas::BlendMode.Normal }",
        ),
    );
    assert_eq!(
        unset, normal,
        "BlendMode.Normal must render byte-identically to an unset blend"
    );
}

/// plan-116-C Phase 1: how wrong is the transformed-distance correction?
///
/// Kept and `#[ignore]`d rather than deleted. It is not a regression test — it
/// measures a *design* question and its answer is written into
/// `planning/completed/plan-116-C-canvas-transform.md` §4.2. Re-run it with
/// `cargo test --release --test rt_canvas_rasteriser -- --ignored --nocapture` if that
/// choice is ever revisited.
///
/// **The question.** Evaluating a shape at `T⁻¹(p)` yields a distance in *shape* space.
/// Coverage must be computed in *surface* space, so the distance needs dividing by the
/// local scale of `T⁻¹`. §4.2 proposed `sqrt(|det M|)`, which is exact for a similarity
/// and an approximation otherwise, and required the error to be measured before
/// anything was built on it.
///
/// **The answer, in 1/255 coverage steps, against a 32×32 supersampled ground truth:**
///
/// | | `sqrt(\|det M\|)` | `d / ‖∇d‖` |
/// |---|---|---|
/// | identity (the control) | 3.19 | 3.19 |
/// | 2:1 non-uniform scale | **37.34** | 3.19 |
/// | 30° shear | **18.18** | 9.71 |
///
/// 3.19/255 is the measurement floor — the supersampling grid quantises a straight
/// edge's area at 1/32 per axis — so `d / ‖∇d‖` is *exact* for the non-uniform scale.
///
/// And the shear's residual 9.71 is not the correction's fault either: an
/// **untransformed** 30° edge measures 9.71 too, and an untransformed 45° edge 13.69.
/// That is the inherent error of the `clamp(0.5 - d, 0, 1)` coverage model on an edge
/// that is not axis-aligned — the model `06_canvas.md` §"Rendering conventions"
/// specifies, which every rotated shape in the renderer has always been drawn with. So
/// the gradient form introduces **no error the renderer did not already have**, and
/// `sqrt(|det M|)` introduces up to 37 steps of new error.
///
/// Hence §4.2's formula changed. The gradient is taken by explicit central differences
/// at a fixed epsilon, which is deterministic — `+ - * /` and `sqrt` only — and so does
/// not fall foul of the same section's ban on `fwidth`-style hardware derivatives,
/// whose whole problem is that they vary between platforms.
#[test]
#[ignore = "a design measurement, not a regression gate; see plan-116-C §4.2"]
fn measure_the_transformed_distance_correction() {
    // A half-plane, so the only error source is the correction — a curved shape would
    // mix in the coverage model's curvature error and confuse the two.
    fn sdf(x: f64, _y: f64) -> f64 {
        x
    }
    fn mapped(m: [f64; 4], px: f64, py: f64) -> f64 {
        sdf(m[0] * px + m[2] * py, m[1] * px + m[3] * py)
    }
    fn cover(d: f64) -> f64 {
        (0.5 - d).clamp(0.0, 1.0)
    }
    /// Fraction of the pixel at `(px, py)` whose inverse-mapped point is inside.
    fn truth(m: [f64; 4], px: f64, py: f64) -> f64 {
        const N: usize = 32;
        let mut inside = 0;
        for i in 0..N {
            for j in 0..N {
                let sx = px - 0.5 + (i as f64 + 0.5) / N as f64;
                let sy = py - 0.5 + (j as f64 + 0.5) / N as f64;
                if mapped(m, sx, sy) <= 0.0 {
                    inside += 1;
                }
            }
        }
        inside as f64 / (N * N) as f64
    }
    fn by_sqrt_det(m: [f64; 4], px: f64, py: f64) -> f64 {
        mapped(m, px, py) / (m[0] * m[3] - m[1] * m[2]).abs().sqrt()
    }
    fn by_gradient(m: [f64; 4], px: f64, py: f64) -> f64 {
        const EPS: f64 = 0.5;
        let d = mapped(m, px, py);
        let gx = (mapped(m, px + EPS, py) - mapped(m, px - EPS, py)) / (2.0 * EPS);
        let gy = (mapped(m, px, py + EPS) - mapped(m, px, py - EPS)) / (2.0 * EPS);
        let g = gx.hypot(gy);
        if g > 1e-9 {
            d / g
        } else {
            d
        }
    }
    fn worst(m: [f64; 4], f: fn([f64; 4], f64, f64) -> f64) -> f64 {
        let mut e: f64 = 0.0;
        for j in -20..=20 {
            for i in -40..=40 {
                let (px, py) = (i as f64 * 0.05, j as f64 * 0.5);
                e = e.max((cover(f(m, px, py)) - truth(m, px, py)).abs());
            }
        }
        e
    }

    let shear = (30.0f64).to_radians().tan();
    let cases = [
        ("identity", [1.0, 0.0, 0.0, 1.0]),
        ("2:1 scale", [0.5, 0.0, 0.0, 1.0]),
        ("30deg shear", [1.0, 0.0, -shear, 1.0]),
    ];
    let mut worst_det = 0.0f64;
    let mut worst_grad = 0.0f64;
    for (name, m) in cases {
        let det = worst(m, by_sqrt_det);
        let grad = worst(m, by_gradient);
        eprintln!(
            "{name:12} sqrt(|det|) {:6.2}/255   d/||grad|| {:6.2}/255",
            det * 255.0,
            grad * 255.0
        );
        if name != "identity" {
            worst_det = worst_det.max(det);
            worst_grad = worst_grad.max(grad);
        }
    }

    // The floor: 32x32 supersampling quantises a straight edge's area, and even the
    // identity measures this much.
    let floor = worst([1.0, 0.0, 0.0, 1.0], by_gradient);
    eprintln!("measurement floor {:.2}/255", floor * 255.0);

    assert!(
        worst_det * 255.0 > 30.0,
        "sqrt(|det M|) was expected to be badly wrong for a non-similarity — if this \
         no longer holds, §4.2's conclusion needs re-deriving, not just re-running"
    );
    assert!(
        worst_grad <= worst_det,
        "the gradient form must never be worse than sqrt(|det M|)"
    );
}

/// A 90°-rotated rectangle lands where the matrix says, not where its bounds were.
///
/// A rotation is the case that proves the bounds are transformed too: the item's
/// generator computes a shape-space box, and a renderer that clipped to *that* would
/// keep only the overlap of the rotated shape with its own unrotated box — which for a
/// 90° rotation of a wide, short rectangle is a small square in the middle.
///
/// 90° exactly, so every assertion is a whole pixel and none of them is a judgement
/// call about an antialiased edge. `Transform` is `[a, b, c, d, tx, ty]` applied as
/// `x' = a*x + c*y + tx`, so a 90° rotation about the origin is `a=0, b=1, c=-1, d=0`,
/// and `tx`/`ty` put it back on screen.
#[test]
fn a_rotated_rectangle_lands_where_the_matrix_says() {
    let (frame, _) = render(
        "canvas_xform_rot",
        &scene(
            "  LET t AS canvas::Transform = canvas::Transform[a := 0.0, b := 1.0, c := 0.0 - 1.0, d := 0.0, tx := 400.0, ty := 100.0]\n  \
             LET p AS canvas::Paint = canvas::fill(color::rgb(255, 0, 0))\n  \
             LET bar AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 200.0, height := 40.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := p, transform := t]\n  \
             canvas::present([bar])\n",
        ),
    );

    // The shape-space rectangle is x 0..200, y 0..40. After the rotation and the
    // translation it occupies surface x 360..400, y 100..300.
    assert_eq!(
        pixel(&frame, 380, 200),
        (255, 0, 0, 255),
        "the middle of the ROTATED bar"
    );
    assert_eq!(
        pixel(&frame, 380, 110),
        (255, 0, 0, 255),
        "near the rotated bar's top — 200 px from the pivot, so only a transformed \
         BOUNDS reaches here"
    );
    assert_eq!(
        pixel(&frame, 380, 290),
        (255, 0, 0, 255),
        "near the rotated bar's bottom"
    );
    // Where the UNROTATED rectangle would have been.
    assert_eq!(
        pixel(&frame, 100, 20),
        (0, 0, 0, 255),
        "the untransformed position must be empty — the transform moved the item, it \
         did not draw it twice"
    );
    assert_eq!(
        pixel(&frame, 420, 200),
        (0, 0, 0, 255),
        "just outside the rotated bar"
    );
}

/// A 2× uniform scale doubles the radius **and** the stroke.
///
/// §4.3's decision, asserted rather than assumed: the stroke scales with the shape,
/// because the band is `|d| - half` evaluated in shape space and scaling `d` scales the
/// band. A renderer that corrected the stroke separately would keep it 10 px wide here.
#[test]
fn a_uniform_scale_scales_the_shape_and_its_stroke() {
    let (frame, _) = render(
        "canvas_xform_scale",
        &scene(
            "  LET t AS canvas::Transform = canvas::Transform[a := 2.0, b := 0.0, c := 0.0, d := 2.0, tx := 150.0, ty := 150.0]\n  \
             LET p AS canvas::Paint = canvas::stroke(color::rgb(0, 255, 0), 10.0)\n  \
             LET ring AS canvas::DrawItem = canvas::Circle[id := \"\", position := vector::Float2[x := 150.0, y := 150.0], radius := 50.0, paint := p, transform := t]\n  \
             canvas::present([ring])\n",
        ),
    );

    // Centre (150,150) and radius 50 scale to centre (300,300) and radius 100; the
    // 10 px stroke becomes 20 px, so the band spans radius 90..110.
    let lit = |x: usize, y: usize| pixel(&frame, x, y).1 > 0;
    assert!(
        lit(400, 300),
        "the scaled ring's rightmost band, radius 100"
    );
    assert!(lit(395, 300), "inside the scaled band (radius 95)");
    assert!(lit(405, 300), "outside-ish, still in the band (radius 105)");
    assert!(
        !lit(300, 300),
        "the centre must be hollow — a stroke-only paint fills nothing"
    );
    assert!(
        lit(200, 300),
        "radius 100 on the OTHER side of the scaled centre — the ring is a ring, so \
         both sides are lit"
    );
    assert!(
        !lit(440, 300),
        "radius 140: outside the scaled band's outer edge at 110"
    );
    assert!(
        !lit(250, 300),
        "radius 50 — where the UNSCALED ring would have been"
    );
}

/// An all-zero `Transform` is byte-identical to naming no transform at all.
///
/// The compatibility case, and the reason `__canvas_invertTransform` maps all-zero to
/// the identity in one place rather than each renderer deciding: every `Paint` built
/// before this letter carries exactly this value, so one wrong pixel here is every
/// existing scene changing. Whole-frame equality, including the antialiased edge.
#[test]
fn an_all_zero_transform_is_identical_to_no_transform() {
    let body = |transform: &str| {
        scene(&format!(
            "  LET dot AS canvas::DrawItem = canvas::Circle[id := \"\", position := vector::Float2[x := 300.0, y := 300.0], radius := 90.0, paint := canvas::fill(color::rgb(255, 200, 0)), transform := {transform}]\n  \
             canvas::present([dot])\n"
        ))
    };
    let (unset, _) = render(
        "canvas_xform_unset",
        &body("canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]"),
    );
    let (zero, _) = render(
        "canvas_xform_zero",
        &body("canvas::Transform[a := 0.0, b := 0.0, c := 0.0, d := 0.0, tx := 0.0, ty := 0.0]"),
    );
    assert_eq!(
        unset, zero,
        "the all-zero Transform is the documented identity spelling; it must render \
         byte-identically to naming no transform"
    );
}

/// A singular transform renders untransformed rather than invisible.
///
/// §4.4's choice, and it is about debuggability rather than mathematics: an item that
/// vanishes is indistinguishable from one that was never presented, whereas an
/// obviously untransformed item is a visible bug. It also keeps an infinity out of the
/// distance field, which would poison the whole frame rather than one item.
///
/// `[1, 2, 2, 4]` has determinant zero — it collapses the plane onto a line.
#[test]
fn a_singular_transform_renders_untransformed() {
    let body = |transform: &str| {
        scene(&format!(
            "  LET box AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 100.0, y := 100.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 120.0, height := 80.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(0, 200, 255)), transform := {transform}]\n  \
             canvas::present([box])\n"
        ))
    };
    let (plain, _) = render(
        "canvas_xform_plain",
        &body("canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]"),
    );
    let (singular, _) = render(
        "canvas_xform_singular",
        &body("canvas::Transform[a := 1.0, b := 2.0, c := 2.0, d := 4.0, tx := 0.0, ty := 0.0]"),
    );
    assert_eq!(
        singular, plain,
        "a determinant-zero transform must fall back to the identity, not collapse the \
         item to a line or draw nothing"
    );
}

/// A rotated shape is not clipped to its untransformed bounds.
///
/// The sharpest bounds case: a 45° rotation makes a square's diagonal its widest
/// extent, so the transformed hull is ~1.41× the original box in both axes. A renderer
/// that kept the shape-space bounds would slice all four corners off.
#[test]
fn a_rotated_shape_is_not_clipped_to_its_untransformed_bounds() {
    // cos 45 = sin 45 = 0.7071067811865476
    let (frame, _) = render(
        "canvas_xform_hull",
        &scene(
            "  LET k AS Float = 0.7071067811865476\n  \
             LET t AS canvas::Transform = canvas::Transform[a := k, b := k, c := 0.0 - k, d := k, tx := 400.0, ty := 400.0 - 200.0 * k]\n  \
             LET p AS canvas::Paint = canvas::fill(color::rgb(255, 255, 255))\n  \
             LET sq AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0 - 100.0, y := 0.0 - 100.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 200.0, height := 200.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := p, transform := t]\n  \
             canvas::present([sq])\n",
        ),
    );

    // The square is 200x200 about the origin; rotated 45° its corners reach
    // ±141 along each axis from (300,300), while its untransformed box reached ±100.
    assert_eq!(
        pixel(&frame, 300, 170),
        (255, 255, 255, 255),
        "130 px above the centre — inside the rotated diamond, but OUTSIDE the \
         untransformed box's half-height of 100. This is the pixel a stale bounds \
         rectangle would have cut."
    );
    assert_eq!(
        pixel(&frame, 430, 300),
        (255, 255, 255, 255),
        "130 px right of the centre, same argument"
    );
    assert_eq!(
        pixel(&frame, 380, 380),
        (0, 0, 0, 255),
        "the diamond's flank: inside the untransformed box's corner, outside the \
         rotated shape — so the bounds were widened, not the shape"
    );
}
