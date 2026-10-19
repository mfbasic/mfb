//! plan-176-D: `rt_canvas_rasteriser`'s gradients: stops and ramps, strokes and stop
//! order, gradient transforms and spread, and focal radial gradients. The render helpers
//! and `lit` live in the root, `rt_canvas_rasteriser.rs`.

use super::*;

/// Two gradients with identical headers and different stop colours get their own
/// cache entries.
///
/// plan-116-F Phase 2, and the gradient sibling of
/// `polygons_sharing_a_header_keep_their_own_points`. A gradient's stops live only in
/// the record's **tail**: the header carries the count, the kind and the two points, so
/// two ramps that differ only in colour produce byte-identical headers. Before the
/// 2026-09-01 fix a polygon in exactly that position deterministically shared one entry
/// and the second item drew the first's shape — a wrong picture, reported as success.
///
/// The two shapes here are the same size and same position apart from x, with the same
/// stop offsets and different colours, which is the strongest form of the collision:
/// everything the header can see is equal.
#[test]
fn gradients_sharing_a_header_keep_their_own_stops() {
    let (frame, stats) = render(
        "canvas_gradient_cache",
        &scene(
            "  LET aStops AS List OF canvas::GradientStop = [canvas::GradientStop[offset := 0.0, \
             color := color::rgb(255, 0, 0)], canvas::GradientStop[offset := 1.0, \
             color := color::rgb(255, 0, 0)]]\n  \
             LET bStops AS List OF canvas::GradientStop = [canvas::GradientStop[offset := 0.0, \
             color := color::rgb(0, 0, 255)], canvas::GradientStop[offset := 1.0, \
             color := color::rgb(0, 0, 255)]]\n  \
             LET gA AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, \
             startPoint := vector::Float2[x := 0.0, y := 0.0], \
             endPoint := vector::Float2[x := 100.0, y := 0.0], stops := aStops, spread := canvas::GradientSpread.Pad, transform := canvas::Transform[a := 0.0, b := 0.0, c := 0.0, d := 0.0, tx := 0.0, ty := 0.0], focalOffset := vector::Float2[x := 0.0, y := 0.0], focalRadius := 0.0, strokeApply := canvas::GradientApply.FillOnly]\n  \
             LET gB AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, \
             startPoint := vector::Float2[x := 0.0, y := 0.0], \
             endPoint := vector::Float2[x := 100.0, y := 0.0], stops := bStops, spread := canvas::GradientSpread.Pad, transform := canvas::Transform[a := 0.0, b := 0.0, c := 0.0, d := 0.0, tx := 0.0, ty := 0.0], focalOffset := vector::Float2[x := 0.0, y := 0.0], focalRadius := 0.0, strokeApply := canvas::GradientApply.FillOnly]\n  \
             LET a AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 100.0, y := 100.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 100.0, height := 100.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH canvas::fill(color::rgb(255, 0, 0)) { gradient := gA }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET b AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 400.0, y := 100.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 100.0, height := 100.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH canvas::fill(color::rgb(0, 0, 255)) { gradient := gB }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([a, b])\n",
        ),
    );

    // Two distinct cache entries. `entries=` is the only window onto the cache, since
    // it lives in globals the graphics thread owns (.ai/canvas-threading.md §1).
    let last = stats.last().expect("a stats line per frame");
    assert!(
        last.contains("entries=2"),
        "two gradients differing only in their stop COLOURS must not share a cache \
         entry — the stops live in the tail, so the headers are byte-identical and \
         only the content hash (`__canvas_hashItem`) separates them: {last}"
    );

    // Both items drew *something*, so `entries=2` is about two live records rather
    // than one live and one empty.
    //
    // Deliberately NOT asserting the two ramps' colours here. Nothing evaluates a
    // gradient until Phase 3, so at this phase each square draws its flat `fill` — and
    // those differ between the two items, which would make a colour assertion pass
    // without touching the gradient at all. `gradients_draw_their_own_ramps` in
    // Phase 3 is where that becomes a real check, with both items sharing one flat
    // fill so only the stops can separate them.
    assert_ne!(
        pixel(&frame, 150, 150),
        (0, 0, 0, 255),
        "the first item drew nothing"
    );
    assert_ne!(
        pixel(&frame, 450, 150),
        (0, 0, 0, 255),
        "the second item drew nothing"
    );
}

/// A gradient with fewer than two stops is not a gradient, and costs no tail.
///
/// The no-op rule from §4.1, checked where it is cheapest to get wrong: the *record
/// length*. A one-stop gradient that still appended five floats would give a record
/// whose slot 1 disagreed with what `__canvas_gradientStopBase` derives, and every
/// later reader would index five slots off the end.
#[test]
fn a_gradient_with_fewer_than_two_stops_is_byte_identical_to_a_flat_fill() {
    let flat = |extra: &str| {
        format!(
            "{extra}  LET a AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 100.0, y := 100.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 200.0, height := 150.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := {}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  canvas::present([a])\n",
            if extra.is_empty() {
                "canvas::fill(color::rgb(220, 90, 40))".to_string()
            } else {
                "WITH canvas::fill(color::rgb(220, 90, 40)) { gradient := g }".to_string()
            }
        )
    };
    let (plain, _) = render("canvas_gradient_none", &scene(&flat("")));

    for (name, stops) in [
        ("canvas_gradient_zero", "[]"),
        (
            "canvas_gradient_one",
            "[canvas::GradientStop[offset := 0.0, color := color::rgb(0, 255, 0)]]",
        ),
    ] {
        let decl = format!(
            "  LET s AS List OF canvas::GradientStop = {stops}\n  \
             LET g AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, \
             startPoint := vector::Float2[x := 100.0, y := 100.0], \
             endPoint := vector::Float2[x := 300.0, y := 100.0], stops := s, spread := canvas::GradientSpread.Pad, transform := canvas::Transform[a := 0.0, b := 0.0, c := 0.0, d := 0.0, tx := 0.0, ty := 0.0], focalOffset := vector::Float2[x := 0.0, y := 0.0], focalRadius := 0.0, strokeApply := canvas::GradientApply.FillOnly]\n"
        );
        let (got, _) = render(name, &scene(&flat(&decl)));
        assert_eq!(
            got, plain,
            "{name}: fewer than two stops must render byte-for-byte as the flat fill — \
             a difference means the no-op rule leaked, and an off-by-one in the record \
             length is the likeliest cause"
        );
    }
}

/// A two-stop linear gradient shows its endpoints' colours at the ends and their
/// blend in the middle.
///
/// plan-116-F Phase 3. The midpoint is the assertion that matters: it is where the
/// interpolation *space* shows. Red-to-blue at `t = 0.5` is `(187, 0, 188)` in linear
/// light; interpolating the encoded bytes instead would give `(128, 0, 128)`, and the
/// two are 59 steps apart — far outside anything a rounding argument could excuse.
#[test]
fn a_linear_gradient_interpolates_between_its_stops() {
    let (frame, _) = render(
        "canvas_gradient_linear",
        &scene(
            "  LET s AS List OF canvas::GradientStop = [canvas::GradientStop[offset := 0.0, \
             color := color::rgb(255, 0, 0)], canvas::GradientStop[offset := 1.0, \
             color := color::rgb(0, 0, 255)]]\n  \
             LET g AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, \
             startPoint := vector::Float2[x := 0.0, y := 0.0 - 100.0], \
             endPoint := vector::Float2[x := 400.0, y := 0.0 - 100.0], stops := s, spread := canvas::GradientSpread.Pad, transform := canvas::Transform[a := 0.0, b := 0.0, c := 0.0, d := 0.0, tx := 0.0, ty := 0.0], focalOffset := vector::Float2[x := 0.0, y := 0.0], focalRadius := 0.0, strokeApply := canvas::GradientApply.FillOnly]\n  \
             LET r AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 100.0, y := 100.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 400.0, height := 200.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH canvas::fill(color::rgb(0, 255, 0)) { gradient := g }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([r])\n",
        ),
    );
    // The flat `fill` is green and appears nowhere: a gradient with two or more stops
    // replaces it entirely. Any green here means the gradient was not read.
    for x in [105usize, 300, 495] {
        assert_eq!(
            pixel(&frame, x, 200).1,
            0,
            "the flat fill's green leaked through at x={x} — the gradient replaces the \
             fill colour, it does not blend with it"
        );
    }
    // Near each end, the end stop's own colour.
    let (r0, _, b0, _) = pixel(&frame, 105, 200);
    assert!(
        r0 > 240 && b0 < 60,
        "just inside the start the colour should be nearly the first stop's red, got \
         {:?}",
        pixel(&frame, 105, 200)
    );
    let (r1, _, b1, _) = pixel(&frame, 495, 200);
    assert!(
        r1 < 60 && b1 > 240,
        "just inside the end the colour should be nearly the last stop's blue, got {:?}",
        pixel(&frame, 495, 200)
    );
    // The midpoint, in linear light.
    assert_eq!(
        pixel(&frame, 300, 200),
        (187, 0, 188, 255),
        "the midpoint of a red-to-blue ramp, interpolated in LINEAR light. sRGB-space \
         interpolation would give (128, 0, 128) here — 59 steps away, which is the \
         whole reason the space is a documented decision rather than an accident"
    );
}

/// A radial gradient runs outward from its centre.
#[test]
fn a_radial_gradient_runs_outward_from_its_centre() {
    let (frame, _) = render(
        "canvas_gradient_radial",
        &scene(
            "  LET s AS List OF canvas::GradientStop = [canvas::GradientStop[offset := 0.0, \
             color := color::rgb(255, 255, 255)], canvas::GradientStop[offset := 1.0, \
             color := color::rgb(0, 0, 0)]]\n  \
             LET g AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Radial, \
             startPoint := vector::Float2[x := 0.0, y := 0.0], \
             endPoint := vector::Float2[x := 120.0, y := 0.0], stops := s, spread := canvas::GradientSpread.Pad, transform := canvas::Transform[a := 0.0, b := 0.0, c := 0.0, d := 0.0, tx := 0.0, ty := 0.0], focalOffset := vector::Float2[x := 0.0, y := 0.0], focalRadius := 0.0, strokeApply := canvas::GradientApply.FillOnly]\n  \
             LET c AS canvas::DrawItem = canvas::Circle[id := \"\", position := vector::Float2[x := 400.0, y := 300.0], radius := 120.0, paint := WITH canvas::fill(color::rgb(0, 255, 0)) { gradient := g }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([c])\n",
        ),
    );
    let centre = pixel(&frame, 400, 300);
    assert!(
        centre.0 > 250,
        "the centre is at t = 0, so it takes the first stop's white: got {centre:?}"
    );
    let mid = pixel(&frame, 460, 300);
    assert!(
        mid.0 > 20 && mid.0 < 230,
        "half way out is between the two stops rather than at either: got {mid:?}"
    );
    assert!(mid.0 < centre.0, "the ramp must darken outward, not inward");
    // Radial, not linear: the same distance in the other three directions matches.
    //
    // The four pixels are 340, 459 in x and 240, 359 in y, which looks asymmetric and
    // is not. A pixel is sampled at its CENTRE, `x + 0.5`, and this circle's centre is
    // `400.0` — a pixel *boundary*. So pixel 340 sits 59.5 away and pixel 460 sits
    // 60.5; the mirror of 340 about 400.0 is 459, not 460. Getting that wrong reads as
    // a one-step colour difference and looks exactly like a broken radial arm.
    let ref_px = pixel(&frame, 459, 300);
    for (x, y, what) in [
        (340usize, 300usize, "left"),
        (400, 240, "up"),
        (400, 359, "down"),
    ] {
        assert_eq!(
            pixel(&frame, x, y),
            ref_px,
            "a radial ramp depends on distance only, so {what} must equal right — a \
             difference means the linear arm ran"
        );
    }
}

/// Two gradients differing only in their stops draw their own ramps.
///
/// The half deferred from Phase 2's `gradients_sharing_a_header_keep_their_own_stops`,
/// now that a gradient is actually evaluated. Both items carry the **same flat fill**,
/// so the only thing that can separate them is the stop tail — which is exactly the
/// content the header cannot see and the cache seams had to be taught about.
#[test]
fn gradients_draw_their_own_ramps() {
    let (frame, stats) = render(
        "canvas_gradient_own_ramps",
        &scene(
            "  LET aStops AS List OF canvas::GradientStop = [canvas::GradientStop[offset := 0.0, \
             color := color::rgb(255, 0, 0)], canvas::GradientStop[offset := 1.0, \
             color := color::rgb(255, 0, 0)]]\n  \
             LET bStops AS List OF canvas::GradientStop = [canvas::GradientStop[offset := 0.0, \
             color := color::rgb(0, 0, 255)], canvas::GradientStop[offset := 1.0, \
             color := color::rgb(0, 0, 255)]]\n  \
             LET gA AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, \
             startPoint := vector::Float2[x := 0.0, y := 0.0], \
             endPoint := vector::Float2[x := 100.0, y := 0.0], stops := aStops, spread := canvas::GradientSpread.Pad, transform := canvas::Transform[a := 0.0, b := 0.0, c := 0.0, d := 0.0, tx := 0.0, ty := 0.0], focalOffset := vector::Float2[x := 0.0, y := 0.0], focalRadius := 0.0, strokeApply := canvas::GradientApply.FillOnly]\n  \
             LET gB AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, \
             startPoint := vector::Float2[x := 0.0, y := 0.0], \
             endPoint := vector::Float2[x := 100.0, y := 0.0], stops := bStops, spread := canvas::GradientSpread.Pad, transform := canvas::Transform[a := 0.0, b := 0.0, c := 0.0, d := 0.0, tx := 0.0, ty := 0.0], focalOffset := vector::Float2[x := 0.0, y := 0.0], focalRadius := 0.0, strokeApply := canvas::GradientApply.FillOnly]\n  \
             LET base AS canvas::Paint = canvas::fill(color::rgb(0, 255, 0))\n  \
             LET a AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 100.0, y := 100.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 100.0, height := 100.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH base { gradient := gA }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET b AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 400.0, y := 100.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 100.0, height := 100.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH base { gradient := gB }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([a, b])\n",
        ),
    );
    let last = stats.last().expect("a stats line per frame");
    assert!(
        last.contains("entries=2"),
        "the two records must not share a cache entry: {last}"
    );
    assert_eq!(
        pixel(&frame, 150, 150),
        (255, 0, 0, 255),
        "the first item's own ramp"
    );
    assert_eq!(
        pixel(&frame, 450, 150),
        (0, 0, 255, 255),
        "the second item's own ramp — red here is the cache handing it the first \
         item's record, and the two paints are identical apart from their stops"
    );
}

/// A zero-length axis fills with the first stop's colour rather than dividing by zero.
#[test]
fn a_zero_length_gradient_axis_takes_the_first_stops_colour() {
    let (frame, _) = render(
        "canvas_gradient_zero_axis",
        &scene(
            "  LET s AS List OF canvas::GradientStop = [canvas::GradientStop[offset := 0.0, \
             color := color::rgb(30, 200, 90)], canvas::GradientStop[offset := 1.0, \
             color := color::rgb(200, 30, 90)]]\n  \
             LET g AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, \
             startPoint := vector::Float2[x := 150.0, y := 150.0], \
             endPoint := vector::Float2[x := 150.0, y := 150.0], stops := s, spread := canvas::GradientSpread.Pad, transform := canvas::Transform[a := 0.0, b := 0.0, c := 0.0, d := 0.0, tx := 0.0, ty := 0.0], focalOffset := vector::Float2[x := 0.0, y := 0.0], focalRadius := 0.0, strokeApply := canvas::GradientApply.FillOnly]\n  \
             LET r AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 100.0, y := 100.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 300.0, height := 200.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH canvas::fill(color::rgb(0, 0, 0)) { gradient := g }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([r])\n",
        ),
    );
    for (x, y) in [(105usize, 105usize), (250, 200), (395, 295)] {
        assert_eq!(
            pixel(&frame, x, y),
            (30, 200, 90, 255),
            "a zero-length axis leaves t at 0 everywhere, so the whole shape takes the \
             first stop's colour — defined, not a divide by zero"
        );
    }
}

/// Offsets out of order are clamped monotonically, not sorted.
///
/// §4.1's rule: sorting would silently redraw something other than what the program
/// asked for, so a stop whose offset goes backwards is pulled up to its predecessor's
/// instead. The scene here gives `0.0, 0.8, 0.3, 1.0` — the third is clamped to 0.8,
/// which collapses the second-to-third span to nothing and makes the colour jump there
/// rather than ramp. Asserted as: the region after 0.8 is the *fourth* stop's ramp, and
/// the third stop's colour never appears on its own.
#[test]
fn gradient_offsets_out_of_order_are_clamped_not_sorted() {
    let (frame, _) = render(
        "canvas_gradient_unsorted",
        &scene(
            "  LET s AS List OF canvas::GradientStop = [\
             canvas::GradientStop[offset := 0.0, color := color::rgb(255, 0, 0)], \
             canvas::GradientStop[offset := 0.8, color := color::rgb(0, 255, 0)], \
             canvas::GradientStop[offset := 0.3, color := color::rgb(0, 0, 255)], \
             canvas::GradientStop[offset := 1.0, color := color::rgb(255, 255, 255)]]\n  \
             LET g AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, \
             startPoint := vector::Float2[x := 0.0, y := 0.0 - 100.0], \
             endPoint := vector::Float2[x := 400.0, y := 0.0 - 100.0], stops := s, spread := canvas::GradientSpread.Pad, transform := canvas::Transform[a := 0.0, b := 0.0, c := 0.0, d := 0.0, tx := 0.0, ty := 0.0], focalOffset := vector::Float2[x := 0.0, y := 0.0], focalRadius := 0.0, strokeApply := canvas::GradientApply.FillOnly]\n  \
             LET r AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 100.0, y := 100.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 400.0, height := 100.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH canvas::fill(color::rgb(0, 0, 0)) { gradient := g }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([r])\n",
        ),
    );
    // t = 0.4 lands between stop 0 (0.0 red) and stop 1 (0.8 green) — so red-green,
    // with no blue. Under sorting it would fall between blue at 0.3 and green at 0.8,
    // which would show blue.
    let at40 = pixel(&frame, 260, 150);
    assert!(
        at40.2 < 40,
        "t = 0.4 must lie on the red-to-green span, so there is no blue in it: got \
         {at40:?}. Blue here means the stops were sorted, which is what §4.1 forbids"
    );
    assert!(
        at40.0 > 40 && at40.1 > 40,
        "t = 0.4 is part way from red to green: got {at40:?}"
    );
    // Past 0.8 the ramp runs green to white, so every channel rises.
    let at90 = pixel(&frame, 460, 150);
    assert!(
        at90.0 > 100 && at90.2 > 100,
        "past the clamped pair the ramp runs green to white: got {at90:?}"
    );
}

/// A gradient fills the interior and leaves the outline a flat colour.
///
/// plan-116-F **F15**. `Paint.gradient`'s description and `06_canvas.md` both
/// promise this — *"`stroke` is unaffected — an outline is always a flat colour"* — and
/// before this test the seven gradient cases here were all fill-only, so nothing in the
/// tree would have noticed the ramp leaking into the stroke.
///
/// Two assertions, and both are load-bearing. That the two outline samples **equal each
/// other** catches a stroke that took the gradient. That they also **equal the stroke
/// colour** catches a stroke painted flat from the wrong source — which the first
/// assertion alone would pass.
///
/// The stroke is thick (24px) and the samples sit at opposite ends of the ramp's axis,
/// where a leaked gradient differs most: at x=105 the ramp is nearly pure red and at
/// x=495 nearly pure blue, so a leak is a ~240-step difference and not a rounding one.
#[test]
fn a_gradients_stroke_stays_a_flat_colour() {
    let (frame, _) = render(
        "canvas_gradient_stroke_flat",
        &scene(
            "  LET s AS List OF canvas::GradientStop = [canvas::GradientStop[offset := 0.0, \
             color := color::rgb(255, 0, 0)], canvas::GradientStop[offset := 1.0, \
             color := color::rgb(0, 0, 255)]]\n  \
             LET g AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, \
             startPoint := vector::Float2[x := 0.0, y := 0.0 - 100.0], \
             endPoint := vector::Float2[x := 400.0, y := 0.0 - 100.0], stops := s, spread := canvas::GradientSpread.Pad, transform := canvas::Transform[a := 0.0, b := 0.0, c := 0.0, d := 0.0, tx := 0.0, ty := 0.0], focalOffset := vector::Float2[x := 0.0, y := 0.0], focalRadius := 0.0, strokeApply := canvas::GradientApply.FillOnly]\n  \
             LET r AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 100.0, y := 100.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 400.0, height := 200.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH canvas::fillStroke(color::rgb(0, 255, 0), \
             color::rgb(255, 255, 0), 24.0) { gradient := g }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([r])\n",
        ),
    );
    // Inside the outline band at each end of the axis: 12px of stroke sits either side
    // of the edge, so y=100 is squarely in it.
    let left = pixel(&frame, 105, 100);
    let right = pixel(&frame, 495, 100);
    assert_eq!(
        left, right,
        "the outline took the gradient: its two ends differ, and a stroke is a flat \
         colour by construction — the ramp replaces `fill` and nothing else"
    );
    assert_eq!(
        left,
        (255, 255, 0, 255),
        "the outline is not the `stroke` colour it was given. Equal-to-each-other above \
         would pass a stroke painted flat from the wrong source, which is why this \
         second assertion exists"
    );
    // And the interior still ramps, so the item really did carry a gradient — without
    // this the two assertions above pass on an item that has no gradient at all.
    let (ri, _, bi, _) = pixel(&frame, 300, 200);
    assert!(
        ri > 150 && ri < 220 && bi > 150 && bi < 220,
        "the interior should be the ramp's midpoint; got {:?}, which means this test \
         proved nothing about a gradient",
        pixel(&frame, 300, 200)
    );
}

/// A gradient on a `Line` or an `Arc` is ignored, and two such items never share a
/// cache entry through it.
///
/// plan-116-F **F17**. `__canvas_paintHeader` says *"a kind with no interior takes no
/// gradient"* and originally skipped only `Text` and `NONE`. A `Line` and an `Arc` have
/// no interior either — `mfb spec app canvas` puts it in the same words — so they
/// stored stops that nothing keyed on: `__canvas_hashItem` returned a bare `acc` for
/// both and the since-deleted `__canvas_tailMatches` returned `TRUE`, so two `Line`s differing only in
/// their gradient hashed alike, hit one geometry-cache entry, and the second drew the
/// first's stops.
///
/// Two assertions, and the second is the one that would have failed before the fix.
/// The first pins the rule (a gradient changes nothing on a stroke-only kind) by
/// rendering the same scene twice, once with a gradient and once without, and requiring
/// byte equality. The second puts two differently-gradiented lines in **one** scene:
/// with the stops unkeyed they collide, and with them skipped there is nothing to
/// collide over — either way the frame must equal the no-gradient one.
#[test]
fn a_gradient_on_a_stroke_only_kind_is_ignored() {
    const PLAIN: &str = "  LET a AS canvas::DrawItem = canvas::Line[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], startAt := vector::Float2[x := 40.0, y := 60.0], endAt := vector::Float2[x := 360.0, y := 60.0], paint := WITH canvas::stroke(color::rgb(255, 240, 120), 9.0) { cap := canvas::CapStyle.Butt }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
         LET b AS canvas::DrawItem = canvas::Arc[id := \"\", position := vector::Float2[x := 200.0, y := 260.0], radius := 90.0, startAngle := 0.0, endAngle := 3.14159, paint := WITH canvas::stroke(color::rgb(120, 255, 200), 9.0) { cap := canvas::CapStyle.Butt }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
         canvas::present([a, b])\n";

    // The same two items, each carrying a DIFFERENT gradient. Different from each other
    // is the point: identical ones could not collide.
    const RAMPED: &str =
        "  LET s1 AS List OF canvas::GradientStop = [canvas::GradientStop[offset := 0.0, \
         color := color::rgb(255, 0, 0)], canvas::GradientStop[offset := 1.0, \
         color := color::rgb(0, 0, 255)]]\n  \
         LET s2 AS List OF canvas::GradientStop = [canvas::GradientStop[offset := 0.0, \
         color := color::rgb(0, 255, 0)], canvas::GradientStop[offset := 1.0, \
         color := color::rgb(255, 0, 255)]]\n  \
         LET g1 AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, \
         startPoint := vector::Float2[x := 40.0, y := 0.0], \
         endPoint := vector::Float2[x := 360.0, y := 0.0], stops := s1, spread := canvas::GradientSpread.Pad, transform := canvas::Transform[a := 0.0, b := 0.0, c := 0.0, d := 0.0, tx := 0.0, ty := 0.0], focalOffset := vector::Float2[x := 0.0, y := 0.0], focalRadius := 0.0, strokeApply := canvas::GradientApply.FillOnly]\n  \
         LET g2 AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, \
         startPoint := vector::Float2[x := 40.0, y := 0.0], \
         endPoint := vector::Float2[x := 360.0, y := 0.0], stops := s2, spread := canvas::GradientSpread.Pad, transform := canvas::Transform[a := 0.0, b := 0.0, c := 0.0, d := 0.0, tx := 0.0, ty := 0.0], focalOffset := vector::Float2[x := 0.0, y := 0.0], focalRadius := 0.0, strokeApply := canvas::GradientApply.FillOnly]\n  \
         LET a AS canvas::DrawItem = canvas::Line[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], startAt := vector::Float2[x := 40.0, y := 60.0], endAt := vector::Float2[x := 360.0, y := 60.0], paint := WITH canvas::stroke(color::rgb(255, 240, 120), 9.0) { gradient := g1, cap := canvas::CapStyle.Butt }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
         LET b AS canvas::DrawItem = canvas::Arc[id := \"\", position := vector::Float2[x := 200.0, y := 260.0], radius := 90.0, startAngle := 0.0, endAngle := 3.14159, paint := WITH canvas::stroke(color::rgb(120, 255, 200), 9.0) { gradient := g2, cap := canvas::CapStyle.Butt }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
         canvas::present([a, b])\n";

    let (plain, _) = render("canvas_stroke_only_plain", &scene(PLAIN));
    let (ramped, _) = render("canvas_stroke_only_ramped", &scene(RAMPED));
    assert_eq!(
        plain.len(),
        ramped.len(),
        "the two frames are different sizes, so the comparison below would be \
         meaningless"
    );
    let differing = plain
        .chunks(4)
        .zip(ramped.chunks(4))
        .filter(|(p, r)| p != r)
        .count();
    assert_eq!(
        differing, 0,
        "a gradient changed what a stroke-only item draws: `Line` and `Arc` are drawn \
         entirely from `Paint.stroke` and have no interior, so `gradient` has no \
         fill to replace and `__canvas_paintHeader` must skip them — {differing} \
         pixels differ"
    );
}

/// A gradient inside a group **moves with the group** — settling plan-116-G §4.5's open
/// decision, and pinning it with the scene that can tell the two answers apart.
///
/// The decision was genuinely open. `Paint.gradient` is evaluated at the surface
/// point against an axis read straight from the record, so a gradient is
/// surface-anchored and `Paint.transform` does **not** drag it (`06_canvas.md` says so
/// deliberately). A group offset could consistently have gone either way.
///
/// It moves, on this letter's own stated goal: a group exists to be *reused* — drawn
/// somewhere else — and an item whose colours depend on where the group was placed is
/// not reusable. `Paint.transform` is a different thing; it reshapes one item in place,
/// so following it is not obviously the consistent choice.
///
/// The **diamond** is the test that can see this: one group referenced at two offsets.
/// If the gradient moved with the group, corresponding points inside the two copies
/// have the same colour. If it stayed surface-anchored, the second copy is a different
/// slice of the ramp and the two disagree. Nothing simpler separates them — a single
/// group at a single offset looks identical under both rules.
#[test]
fn a_gradient_inside_a_group_moves_with_the_group() {
    let (frame, _) = render(
        "canvas_group_gradient",
        &scene(
            "  LET stops AS List OF canvas::GradientStop = [canvas::GradientStop[offset := 0.0, color := color::rgb(255, 0, 0)], canvas::GradientStop[offset := 1.0, color := color::rgb(0, 0, 255)]]\n  \
             LET g AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, startPoint := vector::Float2[x := 0.0, y := 0.0], endPoint := vector::Float2[x := 200.0, y := 0.0], stops := stops, spread := canvas::GradientSpread.Pad, transform := canvas::Transform[a := 0.0, b := 0.0, c := 0.0, d := 0.0, tx := 0.0, ty := 0.0], focalOffset := vector::Float2[x := 0.0, y := 0.0], focalRadius := 0.0, strokeApply := canvas::GradientApply.FillOnly]\n  \
             LET bar AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 200.0, height := 60.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH canvas::fill(color::rgb(0, 0, 0)) { gradient := g }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::setGroup(\"bar\", [bar])\n  \
             LET left AS canvas::DrawItem = canvas::Group[id := \"\", position := vector::Float2[x := 50.0, y := 100.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"bar\"]\n  \
             LET right AS canvas::DrawItem = canvas::Group[id := \"\", position := vector::Float2[x := 400.0, y := 300.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"bar\"]\n  \
             canvas::present([left, right])\n",
        ),
    );

    // The same point WITHIN each copy: 30 px along a 200 px ramp, and 150 px along it.
    for (dx, dy) in [(50usize, 100usize), (400usize, 300usize)] {
        assert_eq!(
            pixel(&frame, dx + 30, dy + 30),
            pixel(&frame, 50 + 30, 100 + 30),
            "the two copies of the group disagree 30px into the ramp, so the gradient \
             did not move with the group — it stayed anchored to the surface, and the \
             second copy is showing a different slice of it",
        );
        assert_eq!(
            pixel(&frame, dx + 150, dy + 30),
            pixel(&frame, 50 + 150, 100 + 30),
            "the two copies disagree 150px into the ramp",
        );
    }

    // And the ramp is a ramp, not a flat fill — otherwise the assertions above would
    // hold trivially for any rule at all.
    assert_ne!(
        pixel(&frame, 50 + 10, 100 + 30),
        pixel(&frame, 50 + 190, 100 + 30),
        "the two ends of the ramp are the same colour, so nothing above was tested: a \
         flat fill satisfies both anchoring rules",
    );
}

/// A red-to-blue ramp's stops and a `canvas::Gradient` over them with every plan-163-D
/// field spelled: `kind`, the two points, the spread (`Pad`/`Reflect`/`Repeat`) and the
/// gradient's own transform as `a, b, c, d, tx, ty`.
fn ramp(kind: &str, from: (f64, f64), to: (f64, f64), spread: &str, t: [f64; 6]) -> String {
    format!(
        "canvas::Gradient[kind := canvas::GradientKind.{kind}, \
         startPoint := vector::Float2[x := {}, y := {}], endPoint := vector::Float2[x := {}, y := {}], \
         stops := [canvas::GradientStop[offset := 0.0, color := color::rgb(255, 0, 0)], \
         canvas::GradientStop[offset := 1.0, color := color::rgb(0, 0, 255)]], \
         spread := canvas::GradientSpread.{spread}, \
         transform := canvas::Transform[a := {}, b := {}, c := {}, d := {}, tx := {}, ty := {}], \
         focalOffset := vector::Float2[x := 0.0, y := 0.0], focalRadius := 0.0, \
         strokeApply := canvas::GradientApply.FillOnly]",
        lit(from.0), lit(from.1), lit(to.0), lit(to.1), lit(t[0]), lit(t[1]), lit(t[2]), lit(t[3]), lit(t[4]), lit(t[5])
    )
}

/// The all-zero transform: the identity.
const NO_T: [f64; 6] = [0.0; 6];

/// plan-163-D: a linear gradient turned 90° by its own `transform` runs top to bottom.
///
/// Hand-computed. `G` maps `(x, y)` to `(-y, x)`, so `G^-1 q = (q.y, -q.x)` and the ramp
/// from `(0, 0)` to `(101, 0)` gives `t = q.y / 101`: constant along a row, `0.5` at the
/// row whose centre is `y = 50.5`. The reference is the same ramp written as `(0, 0)` to
/// `(0, 101)` with no transform, whose `t` is the same expression; the turned one goes
/// through the build-time fold (`__canvas_gradientHeader`), so its points carry the
/// fold's rounding and the two may differ by a step at a quantisation boundary, never
/// more.
#[test]
fn a_linear_gradient_turned_by_its_transform_runs_top_to_bottom() {
    let turned = ramp(
        "Linear",
        (-100.0, 0.0),
        (1.0, 0.0),
        "Pad",
        [0.0, 1.0, -1.0, 0.0, -100.0, 100.0],
    );
    let upright = ramp("Linear", (-400.0, 0.0), (-400.0, 101.0), "Pad", NO_T);
    let (frame, _) = render(
        "canvas_gradient_turned",
        &scene(&format!(
            "  LET a AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 100.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 200.0, height := 101.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH canvas::fill(color::rgb(0, 255, 0)) {{ gradient := {turned} }}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET b AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 400.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 200.0, height := 101.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH canvas::fill(color::rgb(0, 255, 0)) {{ gradient := {upright} }}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([a, b])\n"
        )),
    );
    for y in [3usize, 25, 50, 75, 97] {
        let row = pixel(&frame, 110, y);
        for x in [150usize, 200, 290] {
            assert_eq!(
                pixel(&frame, x, y),
                row,
                "the turned ramp varies along row {y}, so it does not run top to bottom"
            );
        }
        let reference = pixel(&frame, 500, y);
        for (got, want) in [
            (row.0, reference.0),
            (row.1, reference.1),
            (row.2, reference.2),
        ] {
            assert!(
                got.abs_diff(want) <= 1,
                "row {y}: the turned ramp is {row:?}, the upright reference {reference:?}"
            );
        }
    }
    let mid = pixel(&frame, 200, 50);
    assert!(
        mid.0.abs_diff(187) <= 1 && mid.1 == 0 && mid.2.abs_diff(188) <= 1,
        "t = 0.5 at y = 50.5 is the red-to-blue midpoint (187, 0, 188) in linear light, got {mid:?}"
    );
    assert!(
        pixel(&frame, 200, 3).0 > 240 && pixel(&frame, 200, 97).2 > 240,
        "red at the top, blue at the bottom"
    );
}

/// plan-163-D: a linear gradient on a rotated item rotates with it -- the ramp is in the
/// item's own coordinates (the user's 2026-09-28 decision reversed the surface-anchored
/// ramp).
///
/// The same item twice: translated to (200, 200), and turned 90° and translated to
/// (500, 200). The turn maps the shape point (x, y) to (-y, x), so the upright copy's
/// pixel (200 + i, 200 + j) and the turned copy's (499 - j, 200 + i) have the same
/// shape-space centre (0.5 + i, 0.5 + j) exactly -- the matrix terms are 0 and +-1 -- and
/// must draw the same colour. A surface-anchored ramp would run across the turned copy
/// left to right instead and fail at every sample.
#[test]
fn a_linear_gradient_on_a_rotated_item_rotates_with_it() {
    let g = ramp("Linear", (0.0, 50.0), (100.0, 50.0), "Pad", NO_T);
    let (frame, _) = render(
        "canvas_gradient_item_space",
        &scene(&format!(
            "  LET g AS canvas::Gradient = {g}\n  \
             LET up AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0 - 50.0, y := 0.0 - 50.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 100.0, height := 100.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH canvas::fill(color::rgb(0, 255, 0)) {{ gradient := g }}, transform := canvas::Transform[a := 1.0, b := 0.0, c := 0.0, d := 1.0, tx := 200.0, ty := 200.0]]\n  \
             LET turned AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0 - 50.0, y := 0.0 - 50.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 100.0, height := 100.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH canvas::fill(color::rgb(0, 255, 0)) {{ gradient := g }}, transform := canvas::Transform[a := 0.0, b := 1.0, c := 0.0 - 1.0, d := 0.0, tx := 600.0, ty := 200.0]]\n  \
             canvas::present([up, turned])\n"
        )),
    );
    for (i, j) in [(-45i64, -40i64), (-20, 10), (0, 0), (17, -33), (44, 44)] {
        let up = pixel(&frame, (200 + i) as usize, (200 + j) as usize);
        let turned = pixel(&frame, (499 - j) as usize, (200 + i) as usize);
        assert_eq!(
            turned, up,
            "shape point ({i}.5, {j}.5): the turned copy's ramp did not turn with it"
        );
    }
    assert_ne!(
        pixel(&frame, 160, 200),
        pixel(&frame, 240, 200),
        "the ramp is flat, so nothing above was tested"
    );
}

/// plan-163-D: the spread past the ramp's end. The ramp runs from x = 99.5 to 199.5, so
/// the pixel centre x + 0.5 is at `t = (x - 99) / 100` -- exactly 1.25 at x = 224, 0.75
/// at x = 174 and 0.25 at x = 124, every one exact in binary. `Pad` there is the last
/// stop, `(0, 0, 255)`; `Reflect` folds 1.25 to 0.75 and `Repeat` wraps it to 0.25, so
/// each must equal its own ramp's pixel at that `t` inside the ramp.
#[test]
fn spread_pads_reflects_and_repeats_past_the_end() {
    let item = |name: &str, y: f64, spread: &str| {
        let g = ramp("Linear", (-0.5, -y), (99.5, -y), spread, NO_T);
        format!(
            "  LET {name} AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 100.0, y := {y:?}], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 300.0, height := 40.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH canvas::fill(color::rgb(0, 255, 0)) {{ gradient := {g} }}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n"
        )
    };
    let (frame, _) = render(
        "canvas_gradient_spread",
        &scene(&format!(
            "{}{}{}  canvas::present([pad, reflect, again])\n",
            item("pad", 100.0, "Pad"),
            item("reflect", 200.0, "Reflect"),
            item("again", 300.0, "Repeat")
        )),
    );
    assert_eq!(
        pixel(&frame, 224, 120),
        (0, 0, 255, 255),
        "Pad at t = 1.25 is the last stop"
    );
    assert_eq!(
        pixel(&frame, 224, 220),
        pixel(&frame, 174, 220),
        "Reflect at t = 1.25 is t = 0.75"
    );
    assert_eq!(
        pixel(&frame, 224, 320),
        pixel(&frame, 124, 320),
        "Repeat at t = 1.25 is t = 0.25"
    );
    assert_ne!(
        pixel(&frame, 174, 220),
        pixel(&frame, 124, 320),
        "t = 0.75 and t = 0.25 must differ, or the two above prove nothing"
    );
    // Inside the ramp all three agree: the spread only acts past the ends.
    assert_eq!(pixel(&frame, 150, 120), pixel(&frame, 150, 220));
    assert_eq!(pixel(&frame, 150, 120), pixel(&frame, 150, 320));
}

/// A radial ramp with plan-163-D's focal fields: centre `c`, end circle radius `r` (the
/// end point is `c + (r, 0)`), the focal offset and radius, and the gradient's own
/// transform.
fn focal(c: (f64, f64), r: f64, offset: (f64, f64), fr: f64, t: [f64; 6]) -> String {
    format!(
        "canvas::Gradient[kind := canvas::GradientKind.Radial, \
         startPoint := vector::Float2[x := {}, y := {}], endPoint := vector::Float2[x := {}, y := {}], \
         stops := [canvas::GradientStop[offset := 0.0, color := color::rgb(255, 0, 0)], \
         canvas::GradientStop[offset := 1.0, color := color::rgb(0, 0, 255)]], \
         spread := canvas::GradientSpread.Pad, \
         transform := canvas::Transform[a := {}, b := {}, c := {}, d := {}, tx := {}, ty := {}], \
         focalOffset := vector::Float2[x := {}, y := {}], focalRadius := {}, \
         strokeApply := canvas::GradientApply.FillOnly]",
        lit(c.0), lit(c.1), lit(c.0 + r), lit(c.1), lit(t[0]), lit(t[1]), lit(t[2]), lit(t[3]),
        lit(t[4]), lit(t[5]), lit(offset.0), lit(offset.1), lit(fr)
    )
}

/// One filled rectangle over the whole test area, its paint's gradient `g`.
fn ramp_box(name: &str, g: &str) -> Vec<u8> {
    render(
        name,
        &scene(&format!(
            "  LET box AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 100.0, y := 50.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 450.0, height := 300.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH canvas::fill(color::rgb(0, 255, 0)) {{ gradient := {g} }}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([box])\n"
        )),
    )
    .0
}

/// plan-163-D: a radial gradient scaled `(2, 1)` by its own transform is an ellipse twice
/// as wide as tall.
///
/// Hand-computed. The ramp is a circle of radius 50 about the origin; `G` scales by
/// `(2, 1)` and moves it to (300.5, 200.5), so `G^-1` maps the pixel centre
/// (300.5 + 2i, 200.5) to (i, 0) and (300.5, 200.5 + i) to (0, i) -- every term exact in
/// float32 -- and both are at `t = i / 50`. So pixel (300 + 2i, 200) is pixel
/// (300, 200 + i), and pixel (300 + i, 200), half as far out in the gradient, is not.
#[test]
fn a_radial_gradient_scaled_two_to_one_is_an_ellipse() {
    let frame = ramp_box(
        "canvas_gradient_ellipse",
        &focal(
            (-100.0, -50.0),
            50.0,
            (0.0, 0.0),
            0.0,
            [2.0, 0.0, 0.0, 1.0, 400.5, 200.5],
        ),
    );
    for i in [10usize, 20, 30, 40] {
        assert_eq!(
            pixel(&frame, 300 + 2 * i, 200),
            pixel(&frame, 300, 200 + i),
            "{i}: twice as far along x must be the same ring as once along y"
        );
        assert_ne!(
            pixel(&frame, 300 + i, 200),
            pixel(&frame, 300, 200 + i),
            "{i}: the ramp is still circular"
        );
    }
}

/// plan-163-D: an off-centre focus. Hand-computed with `c = (300.5, 200.5)`, `r = 100`
/// and the focus `f = c + (50, 0)`: the pixel whose centre is `f` has `e = 0`, so `t = (0 -
/// sqrt(0)) / a = 0`, the first stop exactly; the pixel at `c - (r, 0)` has `e = (-150, 0)`
/// and `w = (-50, 0)`, so `a = -7500`, `b = 7500`, `c = 22500`, the discriminant
/// `225000000` and `t = (7500 - 15000) / -7500 = 1`, the last stop exactly. A centred ramp
/// would put the focus pixel at `t = 0.5`.
#[test]
fn a_focal_offset_moves_the_first_stop_to_the_focus() {
    let frame = ramp_box(
        "canvas_gradient_focal",
        &focal((200.5, 150.5), 100.0, (50.0, 0.0), 0.0, NO_T),
    );
    assert_eq!(
        pixel(&frame, 350, 200),
        (255, 0, 0, 255),
        "the focus is the first stop"
    );
    assert_eq!(
        pixel(&frame, 200, 200),
        (0, 0, 255, 255),
        "c - (r, 0) is on the end circle: the last stop"
    );
}

/// plan-163-D: a focal offset past the end circle is pulled just inside it, so the ramp
/// is defined everywhere: every sampled pixel is a mix of the two stops (no green from the
/// flat fill, which a NaN `t` would not reach either, but an undefined colour would), and
/// the ramp still runs from near the clamped focus to the far side.
#[test]
fn a_focal_offset_beyond_the_circle_is_clamped() {
    let frame = ramp_box(
        "canvas_gradient_focal_clamped",
        &focal((200.5, 150.5), 100.0, (500.0, 0.0), 0.0, NO_T),
    );
    for y in (60usize..340).step_by(20) {
        for x in (110usize..540).step_by(20) {
            let (r, g, b, a) = pixel(&frame, x, y);
            assert!(
                g == 0 && a == 255 && u16::from(r) + u16::from(b) > 0,
                "({x}, {y}) is {:?}, not a ramp colour",
                (r, g, b, a)
            );
        }
    }
    // The clamped focus is c + (99.90234375, 0) = (400.40.., 200.5): beside it the ramp is
    // barely started, on the far side of the circle it has ended.
    let near = pixel(&frame, 399, 200);
    let far = pixel(&frame, 201, 200);
    assert!(
        near.0 > 240 && near.2 < 40,
        "beside the clamped focus the ramp is near its first stop: {near:?}"
    );
    assert!(
        far.0 < 40 && far.2 > 240,
        "across the circle it is near its last stop: {far:?}"
    );
}

/// plan-163-D: a focal radius of `r / 4` fills a disc of that radius with the first stop.
/// Hand-computed with no offset, `r = 100`, `fr = 25`: a point at distance `d` from the
/// centre has `a = -5625`, `b = 1875`, `c = d^2 - 625`, so `t <= 0` exactly when `d <=
/// 25` (Pad: the first stop), and at `d = 30` `t = (1875 - 2250) / -5625 = 1/15`.
#[test]
fn a_focal_radius_fills_a_disc_with_the_first_stop() {
    let frame = ramp_box(
        "canvas_gradient_focal_radius",
        &focal((200.5, 150.5), 100.0, (0.0, 0.0), 25.0, NO_T),
    );
    for i in [0usize, 10, 20, 24] {
        assert_eq!(
            pixel(&frame, 300 + i, 200),
            (255, 0, 0, 255),
            "distance {i}: inside the focal disc"
        );
        assert_eq!(
            pixel(&frame, 300, 200 + i),
            (255, 0, 0, 255),
            "distance {i} down: inside the focal disc"
        );
    }
    assert_ne!(
        pixel(&frame, 330, 200),
        (255, 0, 0, 255),
        "distance 30 is past the disc, t = 1/15"
    );
}

/// A linear red-to-blue ramp along x from 100 to 500 (surface coordinates), with the given
/// stroke apply, written relative to an item at `origin`: a `Rectangle`'s gradient points
/// are offsets from its `position`, a `Line`'s stay on the surface (`origin` (0, 0)).
fn stroke_ramp(apply: &str, origin: (f64, f64)) -> String {
    format!(
        "canvas::Gradient[kind := canvas::GradientKind.Linear, \
         startPoint := vector::Float2[x := {}, y := {}], endPoint := vector::Float2[x := {}, y := {}], \
         stops := [canvas::GradientStop[offset := 0.0, color := color::rgb(255, 0, 0)], \
         canvas::GradientStop[offset := 1.0, color := color::rgb(0, 0, 255)]], \
         spread := canvas::GradientSpread.Pad, \
         transform := canvas::Transform[a := 0.0, b := 0.0, c := 0.0, d := 0.0, tx := 0.0, ty := 0.0], \
         focalOffset := vector::Float2[x := 0.0, y := 0.0], focalRadius := 0.0, \
         strokeApply := canvas::GradientApply.{apply}]",
        lit(100.0 - origin.0),
        lit(0.0 - origin.1),
        lit(500.0 - origin.0),
        lit(0.0 - origin.1)
    )
}

/// plan-163-D: `FillAndStroke` paints the outline with the ramp, tinted by the stroke
/// colour at the strength of its alpha -- the picture tint (`__canvas_tintChannel`).
///
/// Hand-computed against the fill: the ramp runs along x only, so the outline pixel at
/// (x, 100) -- squarely inside the 24px band on the top edge -- has the ramp colour of
/// the fill pixel at (x, 200). A white stroke at alpha 255 tints by `ramp * 255 / 255`,
/// the ramp itself; a stroke at alpha 0 tints by nothing; both draw the plain ramp. A red
/// stroke at alpha 128 keeps red and moves blue by `-(B * 128 + 127) / 255` (half away
/// from zero) and green stays 0. A `Line`, which has no interior, is drawn in the ramp
/// too, matching the fill at the same x.
#[test]
fn fill_and_stroke_paints_the_outline_with_the_tinted_ramp() {
    let item = |name: &str, y: f64, stroke: &str| {
        format!(
            "  LET {name} AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 100.0, y := {y:?}], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 400.0, height := 150.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH canvas::fillStroke(color::rgb(0, 255, 0), {stroke}, 24.0) {{ gradient := {} }}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n",
            stroke_ramp("FillAndStroke", (100.0, y))
        )
    };
    let (frame, _) = render(
        "canvas_gradient_stroke_apply",
        &scene(&format!(
            "{}{}{}  LET line AS canvas::DrawItem = canvas::Line[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], startAt := vector::Float2[x := 100.0, y := 620.0], endAt := vector::Float2[x := 500.0, y := 620.0], paint := WITH canvas::stroke(color::rgb(255, 255, 255), 20.0) {{ gradient := {} }}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([white, clear, red, line])\n",
            item("white", 30.0, "color::rgb(255, 255, 255)"),
            item("clear", 230.0, "color::rgba(255, 255, 255, 0)"),
            item("red", 430.0, "color::rgba(255, 0, 0, 128)"),
            stroke_ramp("FillAndStroke", (0.0, 0.0))
        )),
    );
    // Clear of the 24px bands on the left and right edges (x 88..112 and 488..512).
    for x in [130usize, 200, 300, 400, 470] {
        let fill = pixel(&frame, x, 105);
        assert_eq!(
            fill.1, 0,
            "x = {x}: the fill is the ramp, not the flat green"
        );
        assert_eq!(
            pixel(&frame, x, 30),
            fill,
            "x = {x}: a white stroke at alpha 255 shows the plain ramp"
        );
        assert_eq!(
            pixel(&frame, x, 230),
            pixel(&frame, x, 305),
            "x = {x}: a stroke at alpha 0 shows the plain ramp"
        );
        let (r, g, b, _) = pixel(&frame, x, 505);
        let want_b = i32::from(b) - (i32::from(b) * 128 + 127) / 255;
        assert_eq!(
            pixel(&frame, x, 430),
            (r, g, want_b as u8, 255),
            "x = {x}: red at alpha 128 tints half way"
        );
        assert_eq!(
            pixel(&frame, x, 620),
            fill,
            "x = {x}: a Line under FillAndStroke is drawn in the ramp"
        );
    }
}

/// plan-163-D's negative cases: each draws exactly as its no-op form.
///
/// A `gradient.transform` that flattens the ramp (a zero determinant) is the identity,
/// the `Paint.transform` rule; a gradient with fewer than two stops is no gradient, so
/// `FillAndStroke` leaves a flat stroke and a zero-alpha stroke draws nothing; and a
/// `focalRadius` above the outer radius is clamped to just under it, so `2r` and the
/// clamp value `r (1 - 2^-10)` draw the same.
#[test]
fn degenerate_gradient_fields_draw_as_their_no_op_forms() {
    let pair = |name: &str, a: &str, b: &str| {
        let body = |g: &str| {
            scene(&format!(
                "  LET box AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 100.0, y := 50.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 450.0, height := 300.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH canvas::fillStroke(color::rgb(0, 255, 0), color::rgba(255, 255, 0, 0), 12.0) {{ gradient := {g} }}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
                 canvas::present([box])\n"
            ))
        };
        let (left, _) = render(&format!("{name}_a"), &body(a));
        let (right, _) = render(&format!("{name}_b"), &body(b));
        assert!(
            left == right,
            "{name}: the degenerate form does not draw as its no-op"
        );
    };
    let flat = |t: [f64; 6]| ramp("Linear", (0.0, -50.0), (400.0, -50.0), "Pad", t);
    pair(
        "canvas_gradient_singular",
        &flat([1.0, 2.0, 2.0, 4.0, 130.0, 350.0]),
        &flat(NO_T),
    );
    let one_stop = "canvas::Gradient[kind := canvas::GradientKind.Linear, startPoint := vector::Float2[x := 100.0, y := 0.0], \
         endPoint := vector::Float2[x := 500.0, y := 0.0], stops := [canvas::GradientStop[offset := 0.0, color := color::rgb(255, 0, 0)]], \
         spread := canvas::GradientSpread.Pad, transform := canvas::Transform[a := 0.0, b := 0.0, c := 0.0, d := 0.0, tx := 0.0, ty := 0.0], \
         focalOffset := vector::Float2[x := 0.0, y := 0.0], focalRadius := 0.0, strokeApply := canvas::GradientApply.FillAndStroke]";
    let no_gradient = "canvas::Gradient[kind := canvas::GradientKind.Linear, startPoint := vector::Float2[x := 0.0, y := 0.0], \
         endPoint := vector::Float2[x := 0.0, y := 0.0], stops := [], \
         spread := canvas::GradientSpread.Pad, transform := canvas::Transform[a := 0.0, b := 0.0, c := 0.0, d := 0.0, tx := 0.0, ty := 0.0], \
         focalOffset := vector::Float2[x := 0.0, y := 0.0], focalRadius := 0.0, strokeApply := canvas::GradientApply.FillOnly]";
    pair("canvas_gradient_one_stop_stroke", one_stop, no_gradient);
    pair(
        "canvas_gradient_focal_radius_clamped",
        &focal((200.5, 150.5), 100.0, (0.0, 0.0), 200.0, NO_T),
        &focal((200.5, 150.5), 100.0, (0.0, 0.0), 99.90234375, NO_T),
    );
}
