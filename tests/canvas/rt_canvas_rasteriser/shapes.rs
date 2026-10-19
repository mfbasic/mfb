//! plan-176-D: `rt_canvas_rasteriser`'s basic shapes (rectangles, arcs and butt arcs,
//! circles, polygons, rounded rectangles), the geometry cache, polygon rings, fill rules
//! and square caps. The render helpers and `RING_HELPERS` live in the root,
//! `rt_canvas_rasteriser.rs`.

use super::*;

/// A filled rectangle covers exactly its half-open pixel span, and leaves the
/// background alone outside it.
///
/// The rectangle is placed on integer coordinates so every pixel is wholly in or
/// wholly out — this test is about *placement*, and an antialiased edge would make
/// "is this pixel inside" a judgement call rather than a fact.
#[test]
fn rectangle_fills_its_exact_span() {
    let (frame, _) = render(
        "canvas_rect",
        &scene(
            "  LET box AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 10.0, y := 20.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 100.0, height := 50.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  canvas::present([box])\n",
        ),
    );

    // Interior.
    assert_eq!(pixel(&frame, 10, 20), (255, 0, 0, 255), "top-left corner");
    assert_eq!(
        pixel(&frame, 109, 69),
        (255, 0, 0, 255),
        "bottom-right corner"
    );
    assert_eq!(pixel(&frame, 60, 45), (255, 0, 0, 255), "centre");
    // Just outside, on all four sides.
    assert_eq!(
        pixel(&frame, 9, 45),
        (0, 0, 0, 255),
        "left of the rectangle"
    );
    assert_eq!(pixel(&frame, 110, 45), (0, 0, 0, 255), "right of it");
    assert_eq!(pixel(&frame, 60, 19), (0, 0, 0, 255), "above it");
    assert_eq!(pixel(&frame, 60, 70), (0, 0, 0, 255), "below it");
    // The surface stays opaque everywhere.
    assert_eq!(pixel(&frame, 800, 600).3, 255, "background alpha");
}

/// An arc swept to `endAngle = PI` reaches its end, rather than stopping short.
///
/// The sweep test turns the two angles into direction vectors with the rasteriser's
/// own deterministic `sin`/`cos` (`math::` is unusable here — libm is not correctly
/// rounded, so an arc endpoint would move between platforms). Those started as a
/// Taylor series about zero over `-PI..PI`, whose error is concentrated at the far
/// end of that interval: at `x = 3.14159` it gave `sin = 6.93e-3` against a true
/// `2.65e-6`, rotating the end direction ~1.4 degrees and making
/// the sweep test (then `__canvas_arcInSweep`) exclude the last sliver of the arc.
///
/// The check is the end cap, because that is where the error lands: an arc centred
/// at `(450, 335)` with radius 90, swept `0..PI`, must paint the pixels around
/// `(360, 335)` — its `endAngle` endpoint. The bug left them background.
///
/// Found by the Metal backend drawing 14 pixels here that the software path did not
/// (plan-98-E Phase 2), which is the GPU comparison doing exactly what an oracle
/// cross-check is for.
#[test]
fn an_arc_swept_to_pi_reaches_its_end_cap() {
    let (frame, _) = render(
        "canvas_arc_end_cap",
        &scene(
            "  LET smile AS canvas::DrawItem = canvas::Arc[id := \"\", position := vector::Float2[x := 450.0, y := 335.0], radius := 90.0, startAngle := 0.0, endAngle := 3.14159, paint := WITH canvas::stroke(color::rgb(0, 160, 0), 14.0) { cap := canvas::CapStyle.Butt }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([smile])\n",
        ),
    );

    // The stroke is 14 wide, so the end cap spans x = 353..366 on the centre row.
    for x in [354usize, 360, 366] {
        assert_eq!(
            pixel(&frame, x, 335),
            (0, 160, 0, 255),
            "the arc must reach its endAngle: ({x}, 335) is inside the end cap"
        );
    }
    // The start cap, at the other end, was never affected — it is at angle 0, where
    // the Taylor series was accurate. It is asserted anyway so a fix that moved the
    // *whole* arc would not pass.
    assert_eq!(
        pixel(&frame, 540, 335),
        (0, 160, 0, 255),
        "the arc's startAngle end cap"
    );
    // Above the centre row is outside a 0..PI sweep under Y-down.
    assert_eq!(
        pixel(&frame, 360, 320),
        (0, 0, 0, 255),
        "a 0..PI sweep runs below its centre, not above it"
    );
}

/// bug-1088: a butt-capped arc under a magnifying transform draws its band and nothing
/// else, and its ends are antialiased like any other edge.
///
/// The arc is radius 0.5 under a scale of 100, stroked 0.1 wide: on the surface a ring of
/// radius 50 and half-width 5 about (300.3, 300.7), swept 0.4..2.3 rad. Its distance
/// used to answer 1e6 out of sweep, and the central-difference gradient a transformed
/// distance is divided by, taken across that jump, lit dotted rays from the centre along
/// both end radii out to the item's box (x = 161 and x = 455 were lit). Now a pixel lit
/// by the arc is within the ring's 1-pixel antialiasing reach, `|rho - 50| <= 6`, and
/// within a pixel of the sweep's wedge; and the end radii, at their half-plane distance
/// 0, are partly covered, not cut hard.
#[test]
fn a_magnified_butt_arc_draws_its_band_and_antialiased_ends_only() {
    let (frame, _) = render(
        "canvas_butt_arc_rays",
        &scene(
            "  LET a AS canvas::DrawItem = canvas::Arc[id := \"\", position := vector::Float2[x := 300.3, y := 300.7], radius := 0.5, startAngle := 0.4, endAngle := 2.3, paint := WITH canvas::stroke(color::rgb(255, 255, 255), 0.1) { cap := canvas::CapStyle.Butt }, transform := canvas::Transform[a := 100.0, b := 0.0, c := 0.0, d := 100.0, tx := 0.0, ty := 0.0]]\n  \
             canvas::present([a])\n",
        ),
    );
    let (cx, cy) = (300.3f64, 300.7f64);
    let (sx, sy) = (0.4f64.cos(), 0.4f64.sin());
    let (ex, ey) = (2.3f64.cos(), 2.3f64.sin());
    let mut lit = 0;
    let mut partial_end = 0;
    let mut strays = Vec::new();
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let (r, g, b, _) = pixel(&frame, x, y);
            let level = r.max(g).max(b);
            let dx = x as f64 + 0.5 - cx;
            let dy = y as f64 + 0.5 - cy;
            let rho = (dx * dx + dy * dy).sqrt();
            // Signed distance outside each end radius's half-plane; the sweep is under a
            // half-turn, so the wedge is where both are <= 0.
            let plane = (sy * dx - sx * dy).max(ex * dy - ey * dx);
            if level > 0 {
                lit += 1;
                if (rho - 50.0).abs() > 6.0 || plane > 1.0 {
                    strays.push((x, y, level));
                }
            }
            if plane.abs() < 0.5 && (rho - 50.0).abs() < 3.0 && level > 0 && level < 255 {
                partial_end += 1;
            }
        }
    }
    assert!(
        strays.is_empty(),
        "{} pixels lit away from the arc's band (rays along the sweep's ends), first {:?}",
        strays.len(),
        &strays[..strays.len().min(8)]
    );
    assert!(
        lit > 1000,
        "the quarter-ring was not drawn: {lit} lit pixels"
    );
    assert!(
        partial_end > 0,
        "no pixel on an end radius is partly covered: the butt ends are cut hard rather \
         than antialiased"
    );
}

/// bug-1088 (round 2): a butt arc's sweep is a wedge, and two half-planes through the
/// centre also meet along the OPPOSITE ray when the sweep is near zero -- a wedge of 0
/// is the whole line through the centre. So the sweep is cut by the half-plane in front
/// of the centre along its bisector too, and a zero sweep under `Butt` draws nothing, as
/// a zero-length butt line does.
///
/// Two butt arcs of radius 100, stroke 20, at angle 0.5: one of zero sweep, one of 0.002
/// rad. Neither may light a pixel behind its centre (along `-(cos 0.5, sin 0.5)`), and the
/// zero sweep lights none at all. The tiny one lights its own 0.2 px sliver.
#[test]
fn a_zero_or_tiny_sweep_butt_arc_draws_nothing_behind_its_centre() {
    let arc = |x: f64, end: &str| {
        format!(
            "canvas::Arc[id := \"\", position := vector::Float2[x := {x:.1}, y := 300.7], radius := 100.0, startAngle := 0.5, endAngle := {end}, paint := WITH canvas::stroke(color::rgb(255, 255, 255), 20.0) {{ cap := canvas::CapStyle.Butt }}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]"
        )
    };
    let (frame, _) = render(
        "canvas_butt_arc_zero_sweep",
        &scene(&format!(
            "  LET zero AS canvas::DrawItem = {}\n  LET tiny AS canvas::DrawItem = {}\n  canvas::present([zero, tiny])\n",
            arc(300.3, "0.5"),
            arc(600.3, "0.502")
        )),
    );
    let (ux, uy) = (0.5f64.cos(), 0.5f64.sin());
    for (name, cx) in [("zero", 300.3f64), ("tiny", 600.3f64)] {
        let (mut front, mut behind) = (0, 0);
        for y in 0..HEIGHT {
            for x in (cx as usize - 140)..(cx as usize + 140) {
                let (r, g, b, _) = pixel(&frame, x, y);
                if r.max(g).max(b) == 0 {
                    continue;
                }
                let along = (x as f64 + 0.5 - cx) * ux + (y as f64 + 0.5 - 300.7) * uy;
                if along < 0.0 {
                    behind += 1;
                } else {
                    front += 1;
                }
            }
        }
        assert_eq!(
            behind, 0,
            "{name}: {behind} pixels lit behind the arc's centre"
        );
        if name == "zero" {
            assert_eq!(
                front, 0,
                "a zero-sweep butt arc draws nothing; it lit {front}"
            );
        } else {
            assert!(front > 0, "the 0.002 rad arc drew nothing at all");
        }
    }
}

/// bug-1088 (round 2): a butt arc swept a whole turn is the whole ring. Its two sweep
/// vectors are one, so a wedge built from them cannot tell "everything" from "nothing";
/// round 1 drew a half-covered seam along the start radius. The sweep's turn says so
/// instead (`__canvas_arcTurn`, `arc.y` 2), and the ring is the band alone: every pixel
/// within 8 px of the ring's centre line is fully covered.
#[test]
fn a_butt_arc_swept_a_whole_turn_is_the_whole_ring() {
    let (frame, _) = render(
        "canvas_butt_arc_whole_turn",
        &scene(
            "  LET ring AS canvas::DrawItem = canvas::Arc[id := \"\", position := vector::Float2[x := 300.3, y := 300.7], radius := 100.0, startAngle := 0.5, endAngle := 6.783185307179586, paint := WITH canvas::stroke(color::rgb(255, 255, 255), 20.0) { cap := canvas::CapStyle.Butt }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  canvas::present([ring])\n",
        ),
    );
    let mut short = Vec::new();
    for y in 190..412 {
        for x in 190..412 {
            let rho = ((x as f64 + 0.5 - 300.3).powi(2) + (y as f64 + 0.5 - 300.7).powi(2)).sqrt();
            if (rho - 100.0).abs() <= 8.0 && pixel(&frame, x, y) != (255, 255, 255, 255) {
                short.push((x, y, pixel(&frame, x, y)));
            }
        }
    }
    assert!(
        short.is_empty(),
        "{} ring pixels not fully covered (a seam), first {:?}",
        short.len(),
        &short[..short.len().min(6)]
    );
}

/// bug-1088 (round 2): a butt arc whose end comes before its start has no sweep, and
/// draws nothing, as a zero sweep does. Two half-planes through the centre meet in the
/// cone OPPOSITE such a sweep, which is what main drew (a 0.5 rad wedge behind the
/// centre, 1,057 px for this arc); the cone test's "ahead of the centre" removes it, and
/// what is left (hairlines where the end rays cross the band) is not an arc either.
#[test]
fn a_butt_arc_whose_end_is_before_its_start_draws_nothing() {
    let (frame, _) = render(
        "canvas_butt_arc_negative_sweep",
        &scene(
            "  LET back AS canvas::DrawItem = canvas::Arc[id := \"\", position := vector::Float2[x := 300.3, y := 300.7], radius := 100.0, startAngle := 0.5, endAngle := 0.0, paint := WITH canvas::stroke(color::rgb(255, 255, 255), 20.0) { cap := canvas::CapStyle.Butt }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  canvas::present([back])\n",
        ),
    );
    let lit = frame
        .chunks(4)
        .filter(|p| p[0].max(p[1]).max(p[2]) > 0)
        .count();
    assert_eq!(
        lit, 0,
        "a butt arc with its end before its start lit {lit} pixels"
    );
}

/// A filled circle is round, and its edge is antialiased with computable coverage.
///
/// The hand-check: at row `y = 143` the pixel centre is `(243.5, 143.5)`, so
/// `d = sqrt(56.5² + 56.5²) - 80 = -0.0969`, giving coverage
/// `round((0.5 + 0.0969) * 255) = 152`. Blending an opaque colour at alpha 152 over
/// black in linear space gives `round(srgb(65535 * 152 / 255)) = 203` per channel —
/// which is the value that was `0` while the sRGB table was truncated.
#[test]
fn circle_is_round_and_antialiased() {
    let (frame, _) = render(
        "canvas_circle",
        &scene(
            "  LET disc AS canvas::DrawItem = canvas::Circle[id := \"\", position := vector::Float2[x := 300.0, y := 200.0], radius := 80.0, paint := canvas::fill(color::rgb(255, 255, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  canvas::present([disc])\n",
        ),
    );

    assert_eq!(pixel(&frame, 300, 200), (255, 255, 0, 255), "centre");
    assert_eq!(
        pixel(&frame, 300, 130),
        (255, 255, 0, 255),
        "inside, near the top"
    );
    assert_eq!(pixel(&frame, 300, 110), (0, 0, 0, 255), "outside, above");
    assert_eq!(
        pixel(&frame, 220, 200),
        (255, 255, 0, 255),
        "inside, near the left"
    );

    // The 45-degree edge: one partially covered pixel with the coverage computed above.
    assert_eq!(
        pixel(&frame, 242, 143),
        (0, 0, 0, 255),
        "outside the diagonal edge"
    );
    assert_eq!(
        pixel(&frame, 243, 143),
        (203, 203, 0, 255),
        "the antialiased edge pixel"
    );
    assert_eq!(
        pixel(&frame, 244, 143),
        (255, 255, 0, 255),
        "inside the diagonal edge"
    );
}

/// A stroked arc sweeping `0..PI` appears below its centre and nowhere above it.
///
/// This is the angle convention from `plan-98-api.md` — radians, clockwise from `+X`
/// under Y-down — and getting it backwards is the single easiest way to render a
/// mirror image that still looks plausible. Asserting the *absence* above the centre
/// is what makes the test able to catch that.
#[test]
fn arc_sweeps_clockwise_from_positive_x() {
    let (frame, _) = render(
        "canvas_arc",
        &scene(
            "  LET a AS canvas::DrawItem = canvas::Arc[id := \"\", position := vector::Float2[x := 300.0, y := 210.0], radius := 50.0, startAngle := 0.0, endAngle := 3.14159, paint := WITH canvas::stroke(color::rgb(0, 160, 0), 8.0) { cap := canvas::CapStyle.Butt }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  canvas::present([a])\n",
        ),
    );

    let green = |x: usize, y: usize| {
        let (r, g, b, _) = pixel(&frame, x, y);
        g > 100 && r < 100 && b < 100
    };
    assert!(green(300, 260), "the arc's bottom, at centre + radius");
    assert!(
        !green(300, 160),
        "nothing at centre - radius: 0..PI must not sweep up"
    );
    assert!(green(350, 210), "the arc's right end, at angle 0");
    assert!(green(250, 210), "the arc's left end, at angle PI");
    assert!(
        !green(300, 210),
        "the arc has no interior — it is a stroke, not a disc"
    );
}

/// A translucent shape over an opaque one blends in linear space.
///
/// Hand-check: 50% white (alpha 128) over opaque red. Blending is
/// `dst + (src - dst) * alpha / 255` on the **linear** values. Red stays
/// `65535 → 255`; green and blue go `0 + (65535 * 128 + 127) / 255 = 32896`, whose
/// sRGB encode is `1.055 * (32896/65535)^(1/2.4) - 0.055 = 0.7367 → 188`.
///
/// The number that makes this test worth running is the one it must *not* produce:
/// a rasteriser blending in sRGB space would give `128`. Half-way in linear light is
/// not half-way in sRGB, and 188-vs-128 is the whole difference between a correct
/// compositor and a plausible-looking one.
#[test]
fn translucent_fill_blends_in_linear_space() {
    let (frame, _) = render(
        "canvas_blend",
        &scene(
            "  LET under AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 10.0, y := 10.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 200.0, height := 200.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET over AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 50.0, y := 50.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 100.0, height := 100.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgba(255, 255, 255, 128)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([under, over])\n",
        ),
    );

    assert_eq!(
        pixel(&frame, 20, 20),
        (255, 0, 0, 255),
        "the opaque under-layer"
    );
    let blended = pixel(&frame, 100, 100);
    assert_ne!(
        blended,
        (255, 128, 128, 255),
        "blended in sRGB space; compositing must happen on linear values",
    );
    assert_eq!(
        blended,
        (255, 188, 188, 255),
        "50% white over red, blended in linear space",
    );
}

/// A polygon fills its interior and antialiases its slanted edges.
///
/// A triangle is the smallest shape whose edges are neither axis-aligned nor
/// circular, so it exercises the cached edge array's distance and crossing tests
/// together — a sign error in the crossing count inverts inside and outside, which
/// the "outside stays background" assertions catch.
#[test]
fn polygon_fills_its_interior() {
    let (frame, _) = render(
        "canvas_polygon",
        &scene(
            "  MUT pts AS List OF vector::Float2 = []\n  \
             pts = collections::append(pts, vector::Float2[x := 100.0, y := 100.0])\n  \
             pts = collections::append(pts, vector::Float2[x := 300.0, y := 100.0])\n  \
             pts = collections::append(pts, vector::Float2[x := 200.0, y := 300.0])\n  \
             LET tri AS canvas::DrawItem = canvas::Polygon[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], contours := [pts], rule := canvas::FillRule.EvenOdd, paint := canvas::fill(color::rgb(0, 0, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  canvas::present([tri])\n",
        ),
    );

    assert_eq!(
        pixel(&frame, 200, 150),
        (0, 0, 255, 255),
        "well inside the triangle"
    );
    assert_eq!(
        pixel(&frame, 200, 110),
        (0, 0, 255, 255),
        "just below the flat top edge"
    );
    assert_eq!(pixel(&frame, 200, 90), (0, 0, 0, 255), "above the top edge");
    assert_eq!(
        pixel(&frame, 110, 250),
        (0, 0, 0, 255),
        "outside the left slanted edge"
    );
    assert_eq!(pixel(&frame, 200, 310), (0, 0, 0, 255), "below the apex");
}

/// A rounded rectangle's corners are actually round.
///
/// The corner radius is what distinguishes it from a square-cornered `Rectangle`, so the assertion
/// that matters is the one *inside the corner's bounding box but outside its arc* —
/// a `Rectangle` that ignored its radius would pass every other check here.
#[test]
fn rounded_rect_corners_are_cut() {
    let (frame, _) = render(
        "canvas_rounded",
        &scene(
            "  LET box AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 100.0, y := 100.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 200.0, height := 150.0], radiusTL := 40.0, radiusTR := 40.0, radiusBL := 40.0, radiusBR := 40.0, paint := canvas::fill(color::rgb(0, 200, 200)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([box])\n",
        ),
    );

    assert_eq!(pixel(&frame, 200, 175), (0, 200, 200, 255), "centre");
    assert_eq!(
        pixel(&frame, 102, 175),
        (0, 200, 200, 255),
        "mid-left edge is straight"
    );
    assert_eq!(
        pixel(&frame, 200, 102),
        (0, 200, 200, 255),
        "mid-top edge is straight"
    );
    // The top-left corner pixel is inside the rectangle but outside the 40px arc,
    // whose centre is (140, 140): distance sqrt(38.5² + 38.5²) = 54.4 > 40.
    assert_eq!(
        pixel(&frame, 101, 101),
        (0, 0, 0, 255),
        "the corner is cut away"
    );
}

/// Rendering is deterministic: the same scene renders to the same bytes.
///
/// Determinism is not a nicety here, it is what makes an exact-match golden possible
/// at all (plan-98-A invariant 5). Two independent builds and runs of the same source
/// must agree byte for byte, which also catches any dependence on address layout or
/// on a libm transcendental.
#[test]
fn rendering_is_byte_reproducible() {
    let source = scene(
        "  LET disc AS canvas::DrawItem = canvas::Circle[id := \"\", position := vector::Float2[x := 300.0, y := 200.0], radius := 80.0, paint := canvas::fill(color::rgb(255, 255, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
         LET a AS canvas::DrawItem = canvas::Arc[id := \"\", position := vector::Float2[x := 300.0, y := 210.0], radius := 50.0, startAngle := 0.0, endAngle := 3.14159, paint := WITH canvas::stroke(color::rgb(0, 160, 0), 8.0) { cap := canvas::CapStyle.Butt }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
         canvas::present([disc, a])\n",
    );
    let (first, _) = render("canvas_determinism_a", &source);
    let (second, _) = render("canvas_determinism_b", &source);
    assert!(
        first == second,
        "the same scene rendered differently across two runs",
    );
}

/// A cache hit skips geometry generation — plan-98-A invariant 2.
///
/// Three presents: three new polygons, then one of them moved, then back to the first
/// scene. The claim is that re-presenting an unchanged item costs no generation, and
/// it is invisible in the pixels (an identical frame results either way), which is
/// why `MFB_CANVAS_STATS` exists.
///
/// The `3, 4, 4` sequence is only deterministic because the harness sets
/// `MFB_CANVAS_SYNC`. Since plan-98-D Phase 2 the render runs on a graphics thread
/// and the redraw signal is a *flag*, so presents arriving between two frames
/// coalesce — deliberately (`.ai/canvas-threading.md` §3: an intermediate scene was
/// never on screen and nothing observed it). Without the sync mode this run produced
/// one, two or three frames depending on scheduling.
#[test]
fn cache_hit_skips_geometry_generation() {
    let (_, stats) = render(
        "canvas_geo_cache",
        &(scene(
            "  LET a AS canvas::DrawItem = __tri(10.0)\n  LET b AS canvas::DrawItem = __tri(100.0)\n  \
             LET c AS canvas::DrawItem = __tri(200.0)\n  canvas::present([a, b, c])\n  \
             canvas::present([a, b, __tri(300.0)])\n  canvas::present([a, b, c])\n",
        ) + "\nFUNC __tri(x AS Float) AS canvas::DrawItem\n  \
             MUT pts AS List OF vector::Float2 = []\n  \
             pts = collections::append(pts, vector::Float2[x := x, y := 20.0])\n  \
             pts = collections::append(pts, vector::Float2[x := x + 60.0, y := 20.0])\n  \
             pts = collections::append(pts, vector::Float2[x := x + 30.0, y := 90.0])\n  \
             RETURN canvas::Polygon[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], contours := [pts], rule := canvas::FillRule.EvenOdd, paint := canvas::fill(color::rgb(200, 30, 30)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n\
             END FUNC\n"),
    );

    let generations: Vec<i64> = stats
        .iter()
        .map(|line| {
            line.split_whitespace()
                .find_map(|field| field.strip_prefix("generations="))
                .unwrap_or_else(|| panic!("no generations field in {line:?}"))
                .parse()
                .expect("generations is a number")
        })
        .collect();
    assert_eq!(
        generations,
        vec![3, 4, 4],
        "expected 3 new items, then exactly 1 regeneration, then none: {stats:?}",
    );
}

/// Two polygons with the same bounding box, vertex count and paint — but different
/// points — must each draw their own shape.
///
/// The geometry cache once keyed an item by a hash of its header and confirmed a hit by
/// comparing only that header. A polygon's point coordinates live only in the *tail*,
/// so these two triangles collided: identical bounds (100..300 x 100..300), identical
/// count (3), identical paint. Since bug-686 a hit is trusted on the item's content hash
/// (`__canvas_hashItem`), which folds in every point — this test is what holds it to
/// that. The second item must not be handed the first one's edges.
#[test]
fn polygons_sharing_a_header_keep_their_own_points() {
    let (frame, stats) = render(
        "canvas_polygon_cache_collision",
        &scene(
            "  MUT down AS List OF vector::Float2 = []\n  \
             down = collections::append(down, vector::Float2[x := 100.0, y := 100.0])\n  \
             down = collections::append(down, vector::Float2[x := 300.0, y := 100.0])\n  \
             down = collections::append(down, vector::Float2[x := 200.0, y := 300.0])\n  \
             MUT up AS List OF vector::Float2 = []\n  \
             up = collections::append(up, vector::Float2[x := 100.0, y := 300.0])\n  \
             up = collections::append(up, vector::Float2[x := 300.0, y := 300.0])\n  \
             up = collections::append(up, vector::Float2[x := 200.0, y := 100.0])\n  \
             LET a AS canvas::DrawItem = canvas::Polygon[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], contours := [down], rule := canvas::FillRule.EvenOdd, paint := canvas::fill(color::rgb(0, 0, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET b AS canvas::DrawItem = canvas::Polygon[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], contours := [up], rule := canvas::FillRule.EvenOdd, paint := canvas::fill(color::rgb(0, 0, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([a, b])\n",
        ),
    );

    // Inside the apex-down triangle only (the up triangle spans x 195..205 here).
    assert_eq!(
        pixel(&frame, 110, 110),
        (0, 0, 255, 255),
        "inside the first (apex-down) triangle"
    );
    // Inside the apex-up triangle only (the down triangle spans x 195..205 here).
    assert_eq!(
        pixel(&frame, 110, 290),
        (0, 0, 255, 255),
        "inside the second (apex-up) triangle — a cache collision draws the first \
         triangle here instead and leaves this pixel background"
    );
    // The two polygons are different geometry, so the cache must hold two entries.
    let entries = stats
        .iter()
        .rev()
        .find_map(|l| {
            l.split_whitespace()
                .find_map(|w| w.strip_prefix("entries="))
                .map(str::to_string)
        })
        .expect("stats line with entries=");
    assert_eq!(entries, "2", "one cache entry per distinct polygon");
}

/// plan-163-A: one `Polygon` holds several rings, and each ring closes on itself.
///
/// Every sampled pixel is a whole pixel inside or outside every ring — each ring's
/// edges are on integer coordinates and every sample is at least 20 px from one — so
/// each expectation is the plain fill colour or the black background:
///
/// * a 200 px square with a concentric 100 px square inside it, under `EvenOdd`: the
///   inner ring is a hole, so its centre (200, 200) is background and the band between
///   the rings (125, 200), (275, 200), (200, 125) is the fill;
/// * two disjoint rings in one item both fill, and the gap between them does not;
/// * a 1-point ring beside a real ring adds nothing. The item is `fillStroke`d, so a
///   1-point ring turned into a zero-length edge would stroke a dot at (700, 500);
/// * a 2-point ring has no area and draws only its stroke: the stroke straddles the
///   segment y = 560 by 2 px each side, so rows 559 and 560 (centres 0.5 px from it,
///   distance −1.5 to the stroke's edge) are fully covered white, row 570 is not, and
///   the yellow fill is nowhere;
/// * `contours := []` draws nothing and does not trap (the program reaches its
///   `rendered` marker, which `render` requires).
#[test]
fn a_polygon_draws_each_of_its_rings() {
    let (frame, _) = render(
        "canvas_polygon_rings",
        &(scene(
            "  LET donut AS canvas::DrawItem = canvas::Polygon[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], contours := [sq(100.0, 100.0, 200.0), \
             sq(150.0, 150.0, 100.0)], rule := canvas::FillRule.EvenOdd, paint := canvas::fill(color::rgb(0, 0, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET pair AS canvas::DrawItem = canvas::Polygon[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], contours := [sq(400.0, 100.0, 50.0), \
             sq(500.0, 100.0, 50.0)], rule := canvas::FillRule.EvenOdd, paint := canvas::fill(color::rgb(0, 255, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET lone AS canvas::DrawItem = canvas::Polygon[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], contours := [[vector::Float2[x := 700.0, y := 500.0]], \
             sq(600.0, 400.0, 50.0)], rule := canvas::FillRule.EvenOdd, paint := canvas::fillStroke(color::rgb(255, 0, 0), color::rgb(255, 255, 255), 4.0), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET flat AS canvas::DrawItem = canvas::Polygon[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], contours := [[vector::Float2[x := 100.0, y := 560.0], \
             vector::Float2[x := 300.0, y := 560.0]]], rule := canvas::FillRule.EvenOdd, paint := canvas::fillStroke(color::rgb(255, 255, 0), color::rgb(255, 255, 255), 4.0), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET empty AS canvas::DrawItem = canvas::Polygon[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], contours := [], rule := canvas::FillRule.EvenOdd, paint := canvas::fillStroke(color::rgb(255, 0, 255), color::rgb(255, 0, 255), 4.0), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([donut, pair, lone, flat, empty])\n",
        ) + RING_HELPERS),
    );

    let blue = (0, 0, 255, 255);
    let black = (0, 0, 0, 255);
    assert_eq!(pixel(&frame, 200, 200), black, "the hole's centre");
    assert_eq!(
        pixel(&frame, 160, 200),
        black,
        "inside the hole, near its edge"
    );
    assert_eq!(pixel(&frame, 125, 200), blue, "the band left of the hole");
    assert_eq!(pixel(&frame, 275, 200), blue, "the band right of the hole");
    assert_eq!(pixel(&frame, 200, 125), blue, "the band above the hole");
    assert_eq!(pixel(&frame, 200, 320), black, "outside the outer ring");

    let green = (0, 255, 0, 255);
    assert_eq!(pixel(&frame, 425, 125), green, "the first disjoint ring");
    assert_eq!(pixel(&frame, 525, 125), green, "the second disjoint ring");
    assert_eq!(pixel(&frame, 475, 125), black, "between the two rings");

    assert_eq!(
        pixel(&frame, 625, 425),
        (255, 0, 0, 255),
        "the real ring beside a 1-point one"
    );
    assert_eq!(
        pixel(&frame, 700, 500),
        black,
        "a 1-point ring draws no dot"
    );
    assert_eq!(pixel(&frame, 699, 499), black, "nor next to it");

    let white = (255, 255, 255, 255);
    assert_eq!(
        pixel(&frame, 200, 559),
        white,
        "a 2-point ring's stroke, above the line"
    );
    assert_eq!(
        pixel(&frame, 200, 560),
        white,
        "a 2-point ring's stroke, below the line"
    );
    assert_eq!(pixel(&frame, 200, 570), black, "a 2-point ring has no fill");
    assert_eq!(pixel(&frame, 200, 550), black, "nor above it");
}

/// plan-163-A: a polygon's ring structure and its fill rule are part of its cache key.
///
/// A geometry-cache hit is trusted on the item hash alone (`.ai/canvas-threading.md`
/// §14), and the rule lives only in header slot 21 while the ring split lives only in
/// where one ring's wrap edge goes — so two items that fold the same points must still
/// hash apart when either differs, or the second is drawn with the first's record. Three
/// presents of the same eight points: two rings under `EvenOdd`, the same two rings under
/// `NonZero`, and all eight as one ring. Each is new geometry, so each costs exactly one
/// generation (`generations=1,2,3`); a hash that skipped the rule or the ring counts
/// would hit the cache and report 1 or 2 for a later frame.
#[test]
fn a_polygon_hash_covers_its_rings_and_rule() {
    let (_, stats) = render(
        "canvas_polygon_ring_hash",
        &(scene(
            "  LET outer AS List OF vector::Float2 = sq(100.0, 100.0, 200.0)\n  \
             LET inner AS List OF vector::Float2 = sq(150.0, 150.0, 100.0)\n  \
             canvas::present([canvas::Polygon[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], contours := [outer, inner], rule := canvas::FillRule.EvenOdd, paint := canvas::fill(color::rgb(0, 0, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]])\n  \
             canvas::present([canvas::Polygon[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], contours := [outer, inner], rule := canvas::FillRule.NonZero, paint := canvas::fill(color::rgb(0, 0, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]])\n  \
             canvas::present([canvas::Polygon[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], contours := [collections::flatten([outer, inner])], rule := canvas::FillRule.NonZero, paint := canvas::fill(color::rgb(0, 0, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]])\n",
        ) + RING_HELPERS),
    );
    let generations: Vec<i64> = stats
        .iter()
        .map(|line| {
            line.split_whitespace()
                .find_map(|field| field.strip_prefix("generations="))
                .unwrap_or_else(|| panic!("no generations field in {line:?}"))
                .parse()
                .expect("generations is a number")
        })
        .collect();
    assert_eq!(
        generations,
        vec![1, 2, 3],
        "changing only the rule, then only the ring split, must each build new \
         geometry: {stats:?}",
    );
}

/// plan-163-A: the vertices of a {5/2} pentagram of radius 100 about `(cx, cy)`, in the
/// order that traces the star (every second vertex of the regular pentagon, starting at
/// the top). The points are the pentagon's at angles -90°, 54°, 198°, 342° and 126°,
/// written out to four places, so no transcendental runs in the scene.
const PENTAGRAM: &str = "\nFUNC star(cx AS Float, cy AS Float) AS List OF vector::Float2\n  \
     RETURN [vector::Float2[x := cx, y := cy - 100.0], \
     vector::Float2[x := cx + 58.7785, y := cy + 80.9017], \
     vector::Float2[x := cx - 95.1057, y := cy - 30.9017], \
     vector::Float2[x := cx + 95.1057, y := cy - 30.9017], \
     vector::Float2[x := cx - 58.7785, y := cy + 80.9017]]\n\
     END FUNC\n";

/// plan-163-A: `NonZero` fills by winding number and `EvenOdd` by crossing parity, in the
/// software renderer — the oracle both GPUs are compared against, so it is checked here
/// against hand-counted windings rather than against anything else.
///
/// A ray from the sample point to the left crosses each ring's left edge once. Under
/// Y-down, `sq` runs its left edge bottom-to-top (−1) and `sqCcw` top-to-bottom (+1):
///
/// * outer `sq` + inner `sq` (same way): the hole's centre has winding −2, so `NonZero`
///   fills it and `EvenOdd` (two crossings) leaves it a hole;
/// * outer `sq` + inner `sqCcw` (opposite ways): winding −1 + 1 = 0, a hole under
///   `NonZero` too;
/// * the band between the rings has winding −1 and one crossing, so it is filled under
///   every rule and direction;
/// * a {5/2} pentagram's inner pentagon is wound twice (winding ±2, two crossings), so
///   its centre is filled under `NonZero` and a hole under `EvenOdd`; a point's tip is
///   wound once and filled under both.
///
/// Every sample is at least 5 px from every edge, so each is the plain fill or the black
/// background.
#[test]
fn non_zero_fills_by_winding_and_even_odd_by_parity() {
    let (frame, _) = render(
        "canvas_polygon_nonzero",
        &(scene(
            "  LET blue AS canvas::Paint = canvas::fill(color::rgb(0, 0, 255))\n  \
             LET same AS canvas::DrawItem = canvas::Polygon[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], contours := [sq(50.0, 50.0, 150.0), sq(100.0, 100.0, 50.0)], rule := canvas::FillRule.NonZero, paint := blue, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET sameEo AS canvas::DrawItem = canvas::Polygon[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], contours := [sq(250.0, 50.0, 150.0), sq(300.0, 100.0, 50.0)], rule := canvas::FillRule.EvenOdd, paint := blue, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET opposite AS canvas::DrawItem = canvas::Polygon[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], contours := [sq(450.0, 50.0, 150.0), sqCcw(500.0, 100.0, 50.0)], rule := canvas::FillRule.NonZero, paint := blue, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET oppositeEo AS canvas::DrawItem = canvas::Polygon[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], contours := [sq(650.0, 50.0, 150.0), sqCcw(700.0, 100.0, 50.0)], rule := canvas::FillRule.EvenOdd, paint := blue, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET starNz AS canvas::DrawItem = canvas::Polygon[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], contours := [star(225.0, 420.0)], rule := canvas::FillRule.NonZero, paint := blue, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET starEo AS canvas::DrawItem = canvas::Polygon[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], contours := [star(575.0, 420.0)], rule := canvas::FillRule.EvenOdd, paint := blue, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([same, sameEo, opposite, oppositeEo, starNz, starEo])\n",
        ) + RING_HELPERS
            + PENTAGRAM),
    );

    let blue = (0, 0, 255, 255);
    let black = (0, 0, 0, 255);
    assert_eq!(
        pixel(&frame, 125, 125),
        blue,
        "NonZero, inner ring wound the same way: filled"
    );
    assert_eq!(
        pixel(&frame, 325, 125),
        black,
        "EvenOdd, inner ring wound the same way: a hole"
    );
    assert_eq!(
        pixel(&frame, 525, 125),
        black,
        "NonZero, inner ring wound the other way: a hole"
    );
    assert_eq!(
        pixel(&frame, 725, 125),
        black,
        "EvenOdd, inner ring wound the other way: a hole"
    );
    for (x, label) in [
        (75, "NonZero same"),
        (275, "EvenOdd same"),
        (475, "NonZero opposite"),
        (675, "EvenOdd opposite"),
    ] {
        assert_eq!(
            pixel(&frame, x, 125),
            blue,
            "{label}: the band between the rings"
        );
        assert_eq!(
            pixel(&frame, x - 40, 125),
            black,
            "{label}: outside the outer ring"
        );
    }
    assert_eq!(
        pixel(&frame, 225, 420),
        blue,
        "NonZero pentagram: the centre is wound twice, filled"
    );
    assert_eq!(
        pixel(&frame, 575, 420),
        black,
        "EvenOdd pentagram: the centre is a hole"
    );
    assert_eq!(
        pixel(&frame, 225, 335),
        blue,
        "NonZero pentagram: a point's tip"
    );
    assert_eq!(
        pixel(&frame, 575, 335),
        blue,
        "EvenOdd pentagram: a point's tip"
    );
}

/// plan-163-A: a single convex ring has winding 0 or ±1 everywhere, so `NonZero` and
/// `EvenOdd` must draw it identically — edge pixels included. Two separate programs
/// render the same antialiased, stroked triangle under each rule and the frames are
/// compared byte for byte. This is a symmetry check on the renderer itself: it needs no
/// expectation computed by the oracle it is checking.
#[test]
fn a_convex_ring_draws_the_same_under_both_rules() {
    let program = |rule: &str| {
        scene(&format!(
            "  LET tri AS canvas::DrawItem = canvas::Polygon[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], contours := [[vector::Float2[x := 100.3, y := 120.7], \
             vector::Float2[x := 610.9, y := 140.2], vector::Float2[x := 330.5, y := 560.1]]], rule := canvas::FillRule.{rule}, paint := canvas::fillStroke(color::rgba(30, 140, 250, 200), color::rgb(250, 250, 120), 3.5), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([tri])\n"
        ))
    };
    let (even_odd, _) = render("canvas_convex_evenodd", &program("EvenOdd"));
    let (non_zero, _) = render("canvas_convex_nonzero", &program("NonZero"));
    let differing = even_odd
        .chunks(4)
        .zip(non_zero.chunks(4))
        .filter(|(a, b)| a != b)
        .count();
    assert_eq!(
        differing, 0,
        "a convex ring drew differently under NonZero than under EvenOdd in {differing} pixels",
    );
    assert_eq!(
        pixel(&even_odd, 330, 250).3,
        255,
        "the triangle drew at all"
    );
    assert_ne!(
        pixel(&even_odd, 330, 250),
        (0, 0, 0, 255),
        "the triangle drew at all"
    );
}

/// plan-163-B: a `Square` cap extends the stroke `half` past each end and cuts it square,
/// on a `Line`, an `Arc` and a `PolyLine`, and a zero-length square end is an axis-aligned
/// square (SVG 2 §13.5.2).
///
/// Every stroke is white, 20 px wide (`half` = 10), over black, so a fully covered pixel is
/// `(255, 255, 255)` and an empty one `(0, 0, 0)`. The distance of a square end is
/// `max(perp - half, -t·len - half, (t-1)·len - half)` (`__canvas_segmentDistanceSquare`):
///
/// * the horizontal line (200..400, 300): the pixel centre (190.5, 300.5) is 9.5 px before
///   the start, `9.5 - 10 = -0.5`, fully covered; (189.5, 300.5) is `+0.5`, empty. The
///   CORNER pixel (190.5, 290.5) is `max(9.5 - 10, 9.5 - 10) = -0.5`, fully covered — a
///   round end gives it `sqrt(9.5² + 9.5²) - 10 = +3.4` and a butt end leaves it out;
/// * the 45° line from (300, 100) towards (400, 200): its start's square has a corner at
///   `half·√2 = 14.14` px along −x, (285.86, 100). The pixel centre (286.5, 100.5) has
///   `perp = 14/√2 = 9.8995` and `t·len = −13/√2`, so `d = 9.8995 - 10 = −0.1005`:
///   coverage `int((0.5 + 0.1005)·255 + 0.5) = 153`, white at 153 over black is linear
///   `65535·153/255`, sRGB 203. (288.5, 100.5) is fully covered and (284.5, 100.5) empty;
/// * the zero-length line at (500, 500): its corner pixel (490.5, 490.5) is
///   `max(9.5, 9.5) - 10 = -0.5`, covered, where a round dot leaves it empty;
/// * the arc of radius 100 about (650, 300) swept `0..π`: its start (750, 300) runs down
///   the screen, so the square end is the box above it, x 740..760, y 290..300. Its corner
///   pixel (759.5, 291.5) is `max(9.5 - 10, 8.5 - 10)`, covered; (759.5, 289.5) is
///   `10.5 - 10`, empty. The arc's end (550, ~300) runs up the screen too, so (541.5,
///   291.5) is covered likewise;
/// * the polyline (100, 500) → (200, 500) → (200, 600): square OUTER ends — the corners
///   (90.5, 490.5) and (209.5, 609.5) are `-0.5`, covered — and a still-round turn: the
///   corner's outer pixel (209.5, 491.5) is `sqrt(9.5² + 8.5²) - 10 = +2.7` from both
///   segments, empty.
#[test]
fn a_square_cap_extends_the_stroke_square_past_each_end() {
    let (frame, _) = render(
        "canvas_square_caps",
        &scene(
            "  LET ink AS canvas::Paint = WITH canvas::stroke(color::rgb(255, 255, 255), 20.0) { cap := canvas::CapStyle.Square }\n  \
             LET flat AS canvas::DrawItem = canvas::Line[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], startAt := vector::Float2[x := 200.0, y := 300.0], endAt := vector::Float2[x := 400.0, y := 300.0], paint := ink, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET slant AS canvas::DrawItem = canvas::Line[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], startAt := vector::Float2[x := 300.0, y := 100.0], endAt := vector::Float2[x := 400.0, y := 200.0], paint := ink, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET dot AS canvas::DrawItem = canvas::Line[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], startAt := vector::Float2[x := 500.0, y := 500.0], endAt := vector::Float2[x := 500.0, y := 500.0], paint := ink, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET bow AS canvas::DrawItem = canvas::Arc[id := \"\", position := vector::Float2[x := 650.0, y := 300.0], radius := 100.0, startAngle := 0.0, endAngle := 3.141592653589793, paint := ink, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET bend AS canvas::DrawItem = canvas::PolyLine[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], points := [vector::Float2[x := 100.0, y := 500.0], \
             vector::Float2[x := 200.0, y := 500.0], vector::Float2[x := 200.0, y := 600.0]], colors := [], paint := ink, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([flat, slant, dot, bow, bend])\n",
        ),
    );
    let white = (255, 255, 255, 255);
    let black = (0, 0, 0, 255);
    assert_eq!(
        pixel(&frame, 190, 300),
        white,
        "half a stroke past the line's start"
    );
    assert_eq!(
        pixel(&frame, 189, 300),
        black,
        "more than half a stroke past it"
    );
    assert_eq!(pixel(&frame, 190, 290), white, "the square end's corner");
    assert_eq!(pixel(&frame, 409, 309), white, "the far end's corner");
    assert_eq!(pixel(&frame, 410, 300), black, "past the far end");

    assert_eq!(
        pixel(&frame, 286, 100),
        (203, 203, 203, 255),
        "the 45° end's corner tip, coverage 153"
    );
    assert_eq!(
        pixel(&frame, 288, 100),
        white,
        "inside the 45° end's square"
    );
    assert_eq!(pixel(&frame, 284, 100), black, "past the 45° end's corner");

    assert_eq!(
        pixel(&frame, 490, 490),
        white,
        "a zero-length square end is an axis-aligned square"
    );
    assert_eq!(pixel(&frame, 509, 509), white, "its opposite corner");
    assert_eq!(pixel(&frame, 489, 500), black, "outside it");

    assert_eq!(
        pixel(&frame, 759, 291),
        white,
        "the arc start's square corner"
    );
    assert_eq!(
        pixel(&frame, 759, 289),
        black,
        "past the arc start's square"
    );
    assert_eq!(
        pixel(&frame, 541, 291),
        white,
        "the arc end's square corner"
    );

    assert_eq!(
        pixel(&frame, 90, 490),
        white,
        "the polyline's square start corner"
    );
    assert_eq!(
        pixel(&frame, 209, 609),
        white,
        "the polyline's square end corner"
    );
    assert_eq!(
        pixel(&frame, 209, 491),
        black,
        "the polyline's turn stays round"
    );
}
