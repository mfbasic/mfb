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

mod common;

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
    let binary = common::build_app(&project, name);
    let run = Command::new(&binary)
        .env("MFB_MACAPP_HEADLESS", "1")
        .env("MFB_WINAPP_HEADLESS", "1")
        .env("MFB_GTKAPP_HEADLESS", "1")
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

/// Build and run a program, returning its stdout — with no frame dump required.
///
/// `render` insists on a dump, which is right for every test that looks at pixels and
/// wrong for one whose program is *supposed* to fail before presenting anything. Those
/// tests assert on what the program printed from its `TRAP` handler, so the frame is
/// not merely unnecessary, it is the thing that must not exist.
fn render_stdout(name: &str, source: &str) -> (String, String) {
    let project = common::temp_project(name, source);
    let binary = common::build_app(&project, name);
    let run = Command::new(&binary)
        .env("MFB_MACAPP_HEADLESS", "1")
        .env("MFB_WINAPP_HEADLESS", "1")
        // The GTK one too. Omitting it is invisible on the macOS dev host, where the
        // MACAPP flag is the one that matters; on a Linux box the program tries to open
        // a display, fails, and exits 1 -- which these tests then report as a
        // use-after-free or a missing raise. `render` above has always set all three.
        .env("MFB_GTKAPP_HEADLESS", "1")
        .env("MFB_CANVAS_SYNC", "1")
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", binary.display()));
    let out = String::from_utf8_lossy(&run.stdout).to_string();
    let err = String::from_utf8_lossy(&run.stderr).to_string();
    let _ = std::fs::remove_dir_all(&project);
    (out, err)
}

fn pixel(frame: &[u8], x: usize, y: usize) -> (u8, u8, u8, u8) {
    let i = (y * WIDTH + x) * 4;
    (frame[i], frame[i + 1], frame[i + 2], frame[i + 3])
}

/// A scene program: `main` sets canvas mode, presents, and prints a marker so a
/// silent early exit cannot pass as a successful render.
fn scene(body: &str) -> String {
    format!(
        "IMPORT app\nIMPORT canvas\nIMPORT collections\nIMPORT io\n\nSUB main()\n  \
         app::setMode(app::Mode.Canvas)\n{body}  io::print(\"rendered\")\nEND SUB\n"
    )
}

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
            "  LET box AS canvas::DrawItem = canvas::Rectangle[x := 10.0, y := 20.0, w := 100.0, h := 50.0, \
             paint := canvas::fill(canvas::rgb(255, 0, 0))]\n  canvas::present([box])\n",
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
/// `__canvas_arcInSweep` exclude the last sliver of the arc.
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
            "  LET smile AS canvas::DrawItem = canvas::Arc[x := 450.0, y := 335.0, radius := 90.0, \
             startAngle := 0.0, endAngle := 3.14159, \
             cap := canvas::CapStyle.Butt, paint := canvas::stroke(canvas::rgb(0, 160, 0), 14.0)]\n  \
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
            "  LET disc AS canvas::DrawItem = canvas::Circle[x := 300.0, y := 200.0, radius := 80.0, \
             paint := canvas::fill(canvas::rgb(255, 255, 0))]\n  canvas::present([disc])\n",
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
            "  LET a AS canvas::DrawItem = canvas::Arc[x := 300.0, y := 210.0, radius := 50.0, \
             startAngle := 0.0, endAngle := 3.14159, \
             cap := canvas::CapStyle.Butt, paint := canvas::stroke(canvas::rgb(0, 160, 0), 8.0)]\n  canvas::present([a])\n",
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
            "  LET under AS canvas::DrawItem = canvas::Rectangle[x := 10.0, y := 10.0, w := 200.0, h := 200.0, \
             paint := canvas::fill(canvas::rgb(255, 0, 0))]\n  \
             LET over AS canvas::DrawItem = canvas::Rectangle[x := 50.0, y := 50.0, w := 100.0, h := 100.0, \
             paint := canvas::fill(canvas::rgba(255, 255, 255, 128))]\n  \
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
            "  MUT pts AS List OF canvas::Point = []\n  \
             pts = collections::append(pts, canvas::Point[x := 100.0, y := 100.0])\n  \
             pts = collections::append(pts, canvas::Point[x := 300.0, y := 100.0])\n  \
             pts = collections::append(pts, canvas::Point[x := 200.0, y := 300.0])\n  \
             LET tri AS canvas::DrawItem = canvas::Polygon[points := pts, \
             paint := canvas::fill(canvas::rgb(0, 0, 255))]\n  canvas::present([tri])\n",
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
/// The corner radius is what distinguishes it from `Rectangle`, so the assertion
/// that matters is the one *inside the corner's bounding box but outside its arc* —
/// a `RoundedRect` that ignored its radius would pass every other check here.
#[test]
fn rounded_rect_corners_are_cut() {
    let (frame, _) = render(
        "canvas_rounded",
        &scene(
            "  LET box AS canvas::DrawItem = canvas::RoundedRect[x := 100.0, y := 100.0, w := 200.0, h := 150.0, \
             cornerRadius := 40.0, paint := canvas::fill(canvas::rgb(0, 200, 200))]\n  \
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
        "  LET disc AS canvas::DrawItem = canvas::Circle[x := 300.0, y := 200.0, radius := 80.0, \
         paint := canvas::fill(canvas::rgb(255, 255, 0))]\n  \
         LET a AS canvas::DrawItem = canvas::Arc[x := 300.0, y := 210.0, radius := 50.0, startAngle := 0.0, \
         endAngle := 3.14159, cap := canvas::CapStyle.Butt, paint := canvas::stroke(canvas::rgb(0, 160, 0), 8.0)]\n  \
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
             MUT pts AS List OF canvas::Point = []\n  \
             pts = collections::append(pts, canvas::Point[x := x, y := 20.0])\n  \
             pts = collections::append(pts, canvas::Point[x := x + 60.0, y := 20.0])\n  \
             pts = collections::append(pts, canvas::Point[x := x + 30.0, y := 90.0])\n  \
             RETURN canvas::Polygon[points := pts, paint := canvas::fill(canvas::rgb(200, 30, 30))]\n\
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
/// The geometry cache keys an item by a hash of its 22-slot header and confirms a
/// hit by comparing only that header (`__canvas_headerMatches`). A polygon's point
/// coordinates live only in the *tail*, so these two triangles collide: identical
/// bounds (100..300 x 100..300), identical count (3), identical paint. The second
/// item must not be handed the first one's edges.
#[test]
fn polygons_sharing_a_header_keep_their_own_points() {
    let (frame, stats) = render(
        "canvas_polygon_cache_collision",
        &scene(
            "  MUT down AS List OF canvas::Point = []\n  \
             down = collections::append(down, canvas::Point[x := 100.0, y := 100.0])\n  \
             down = collections::append(down, canvas::Point[x := 300.0, y := 100.0])\n  \
             down = collections::append(down, canvas::Point[x := 200.0, y := 300.0])\n  \
             MUT up AS List OF canvas::Point = []\n  \
             up = collections::append(up, canvas::Point[x := 100.0, y := 300.0])\n  \
             up = collections::append(up, canvas::Point[x := 300.0, y := 300.0])\n  \
             up = collections::append(up, canvas::Point[x := 200.0, y := 100.0])\n  \
             LET a AS canvas::DrawItem = canvas::Polygon[points := down, \
             paint := canvas::fill(canvas::rgb(0, 0, 255))]\n  \
             LET b AS canvas::DrawItem = canvas::Polygon[points := up, \
             paint := canvas::fill(canvas::rgb(0, 0, 255))]\n  \
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
            "  LET c AS canvas::Color = canvas::rgb(255, 0, 0)\n  \
             LET p AS canvas::Paint = WITH canvas::fill(c) { clip := canvas::Bounds[x := 0.0, y := 0.0, w := 200.0, h := 640.0] }\n  \
             LET dot AS canvas::DrawItem = canvas::Circle[x := 200.0, y := 200.0, radius := 100.0, paint := p]\n  \
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
            "  LET c AS canvas::Color = canvas::rgb(255, 255, 255)\n  \
             LET p AS canvas::Paint = WITH canvas::fill(c) { clip := canvas::Bounds[x := 100.25, y := 0.0, w := 300.0, h := 640.0] }\n  \
             LET box AS canvas::DrawItem = canvas::Rectangle[x := 0.0, y := 0.0, w := 500.0, h := 200.0, paint := p]\n  \
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

/// A zero-area clip means "no clip", and renders identically to an unset one.
///
/// This is the compatibility case, and it is asserted as whole-frame byte equality
/// rather than by sampling: `canvas::Bounds`'s own description promises a zero-area
/// rectangle reads as unclipped, and every `Paint` built before plan-116-B carries
/// exactly that value. A single wrong pixel here is every existing scene changing.
#[test]
fn a_zero_area_clip_is_identical_to_no_clip() {
    let body = |clip: &str| {
        scene(&format!(
            "  LET c AS canvas::Color = canvas::rgb(0, 200, 255)\n  \
             LET p AS canvas::Paint = {clip}\n  \
             LET dot AS canvas::DrawItem = canvas::Circle[x := 300.0, y := 300.0, radius := 120.0, paint := p]\n  \
             canvas::present([dot])\n"
        ))
    };
    let (unset, _) = render("canvas_clip_unset", &body("canvas::fill(c)"));
    let (zero, _) = render(
        "canvas_clip_zero",
        &body(
            "WITH canvas::fill(c) { clip := canvas::Bounds[x := 50.0, y := 50.0, w := 0.0, h := 0.0] }",
        ),
    );
    assert_eq!(
        unset, zero,
        "a zero-area clip must render byte-identically to no clip at all"
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
            "  LET red AS canvas::Color = canvas::rgb(255, 0, 0)\n  \
             LET green AS canvas::Color = canvas::rgb(0, 255, 0)\n  \
             LET p AS canvas::Paint = WITH canvas::fill(red) { clip := canvas::Bounds[x := 600.0, y := 400.0, w := 100.0, h := 100.0] }\n  \
             LET hidden AS canvas::DrawItem = canvas::Rectangle[x := 10.0, y := 10.0, w := 100.0, h := 100.0, paint := p]\n  \
             LET shown AS canvas::DrawItem = canvas::Rectangle[x := 300.0, y := 10.0, w := 50.0, h := 50.0, paint := canvas::fill(green)]\n  \
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
            "  LET c AS canvas::Color = canvas::rgb(255, 200, 0)\n  \
             LET p AS canvas::Paint = {clip}\n  \
             LET dot AS canvas::DrawItem = canvas::Circle[x := 300.0, y := 300.0, radius := 90.0, paint := p]\n  \
             canvas::present([dot])\n"
        ))
    };
    let (unset, _) = render("canvas_clip_big_unset", &body("canvas::fill(c)"));
    let (big, _) = render(
        "canvas_clip_big",
        &body(
            "WITH canvas::fill(c) { clip := canvas::Bounds[x := 0.0, y := 0.0, w := 900.0, h := 640.0] }",
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
            "  LET {name} AS canvas::DrawItem = canvas::Rectangle[x := {x:.1}, y := 60.0, w := 60.0, h := 60.0, \
             paint := WITH canvas::fill(canvas::rgb(200, 100, 50)) {{ blend := canvas::BlendMode.{mode} }}]\n"
        )
    };
    let body = format!(
        "  LET ground AS canvas::DrawItem = canvas::Rectangle[x := 0.0, y := 0.0, w := 600.0, h := 200.0, \
         paint := canvas::fill(canvas::rgb(128, 128, 128))]\n{}{}{}{}  \
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
            "  LET ground AS canvas::DrawItem = canvas::Rectangle[x := 0.0, y := 0.0, w := 600.0, h := 400.0, \
             paint := canvas::fill(canvas::rgb(128, 128, 128))]\n  \
             LET dot AS canvas::DrawItem = canvas::Circle[x := 300.0, y := 200.0, radius := 90.0, paint := {paint}]\n  \
             canvas::present([ground, dot])\n"
        ))
    };
    let (unset, _) = render(
        "canvas_blend_unset",
        &body("canvas::fill(canvas::rgba(255, 200, 0, 160))"),
    );
    let (normal, _) = render(
        "canvas_blend_normal",
        &body(
            "WITH canvas::fill(canvas::rgba(255, 200, 0, 160)) { blend := canvas::BlendMode.Normal }",
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
             LET p AS canvas::Paint = WITH canvas::fill(canvas::rgb(255, 0, 0)) { transform := t }\n  \
             LET bar AS canvas::DrawItem = canvas::Rectangle[x := 0.0, y := 0.0, w := 200.0, h := 40.0, paint := p]\n  \
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
            "  LET t AS canvas::Transform = canvas::Transform[a := 2.0, b := 0.0, c := 0.0, d := 2.0, tx := 0.0, ty := 0.0]\n  \
             LET p AS canvas::Paint = WITH canvas::stroke(canvas::rgb(0, 255, 0), 10.0) { transform := t }\n  \
             LET ring AS canvas::DrawItem = canvas::Circle[x := 150.0, y := 150.0, radius := 50.0, paint := p]\n  \
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
    let body = |paint: &str| {
        scene(&format!(
            "  LET dot AS canvas::DrawItem = canvas::Circle[x := 300.0, y := 300.0, radius := 90.0, paint := {paint}]\n  \
             canvas::present([dot])\n"
        ))
    };
    let (unset, _) = render(
        "canvas_xform_unset",
        &body("canvas::fill(canvas::rgb(255, 200, 0))"),
    );
    let (zero, _) = render(
        "canvas_xform_zero",
        &body(
            "WITH canvas::fill(canvas::rgb(255, 200, 0)) { transform := canvas::Transform[a := 0.0, b := 0.0, c := 0.0, d := 0.0, tx := 0.0, ty := 0.0] }",
        ),
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
    let body = |paint: &str| {
        scene(&format!(
            "  LET box AS canvas::DrawItem = canvas::Rectangle[x := 100.0, y := 100.0, w := 120.0, h := 80.0, paint := {paint}]\n  \
             canvas::present([box])\n"
        ))
    };
    let (plain, _) = render(
        "canvas_xform_plain",
        &body("canvas::fill(canvas::rgb(0, 200, 255))"),
    );
    let (singular, _) = render(
        "canvas_xform_singular",
        &body(
            "WITH canvas::fill(canvas::rgb(0, 200, 255)) { transform := canvas::Transform[a := 1.0, b := 2.0, c := 2.0, d := 4.0, tx := 0.0, ty := 0.0] }",
        ),
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
             LET t AS canvas::Transform = canvas::Transform[a := k, b := k, c := 0.0 - k, d := k, tx := 300.0, ty := 300.0]\n  \
             LET p AS canvas::Paint = WITH canvas::fill(canvas::rgb(255, 255, 255)) { transform := t }\n  \
             LET sq AS canvas::DrawItem = canvas::Rectangle[x := 0.0 - 100.0, y := 0.0 - 100.0, w := 200.0, h := 200.0, paint := p]\n  \
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

/// A butt-capped line stops at its endpoint; the same line round-capped does not.
///
/// plan-116-D Phase 2. Asserted against **the same line** a few pixels apart, because
/// that difference is the only thing the cap changes — a weaker pair of assertions
/// would also pass on a renderer that ignored the flag entirely.
///
/// Horizontal, from `x = 200` to `x = 400` at `y = 300`, stroke width 20 (half-width
/// 10). Pixel centres sit at `x + 0.5`, so pixel 405's centre is 5.5 past the end
/// plane: outside for `Butt`, inside the end disc for `Round`.
#[test]
fn a_butt_cap_stops_at_the_endpoint_and_a_round_cap_does_not() {
    let line = |cap: &str| {
        format!(
            "  LET l AS canvas::DrawItem = canvas::Line[x1 := 200.0, y1 := 300.0, \
             x2 := 400.0, y2 := 300.0, cap := canvas::CapStyle.{cap}, \
             paint := canvas::stroke(canvas::rgb(255, 255, 255), 20.0)]\n  \
             canvas::present([l])\n"
        )
    };
    let (butt, _) = render("canvas_cap_butt", &scene(&line("Butt")));
    let (round, _) = render("canvas_cap_round", &scene(&line("Round")));

    // Both must draw the body, or every assertion below is vacuous: an item that was
    // dropped entirely would "pass" all the outside-the-cap checks.
    for (name, frame) in [("butt", &butt), ("round", &round)] {
        assert_eq!(
            pixel(frame, 300, 300),
            (255, 255, 255, 255),
            "the {name} line did not draw its own middle"
        );
        assert_eq!(
            pixel(frame, 399, 300),
            (255, 255, 255, 255),
            "the {name} line stops short of its own endpoint"
        );
    }

    assert_eq!(
        pixel(&butt, 405, 300),
        (0, 0, 0, 255),
        "past the endpoint is background for a butt cap — ink here is the round \
         distance, so the flag was not read"
    );
    assert_eq!(
        pixel(&round, 405, 300),
        (255, 255, 255, 255),
        "the same pixel is inside the round cap's disc (half-width 10), so a round \
         cap must still paint it"
    );
    assert_eq!(
        pixel(&round, 415, 300),
        (0, 0, 0, 255),
        "15 px past is outside even the round cap, so the disc has the stroke's \
         half-width and not some larger reach"
    );
}

/// A zero-length line is a dot when round-capped and nothing at all when butt-capped.
///
/// The degenerate case, and the one a `max` of three terms is most likely to get
/// wrong: with `len2 = 0` there is no direction for the two end planes to be
/// perpendicular to. Butt answers "outside everywhere" deliberately rather than
/// dividing by zero; Round keeps the pre-existing behaviour, where clamping `t` makes
/// the distance radial and the shape a disc.
///
/// Both halves are needed. Asserting only that the butt one is empty would also pass
/// if zero-length lines had stopped drawing altogether.
#[test]
fn a_zero_length_line_is_a_dot_only_when_round_capped() {
    let dot = |cap: &str| {
        format!(
            "  LET l AS canvas::DrawItem = canvas::Line[x1 := 300.0, y1 := 300.0, \
             x2 := 300.0, y2 := 300.0, cap := canvas::CapStyle.{cap}, \
             paint := canvas::stroke(canvas::rgb(255, 255, 255), 20.0)]\n  \
             canvas::present([l])\n"
        )
    };
    let (butt, _) = render("canvas_cap_zero_butt", &scene(&dot("Butt")));
    let (round, _) = render("canvas_cap_zero_round", &scene(&dot("Round")));

    assert_eq!(
        pixel(&round, 300, 300),
        (255, 255, 255, 255),
        "a zero-length ROUND line is a disc of the stroke's half-width, so its centre \
         is painted — the behaviour that existed before plan-116-D"
    );
    assert_eq!(
        pixel(&round, 305, 300),
        (255, 255, 255, 255),
        "5 px from the centre is inside that disc (half-width 10)"
    );
    assert_eq!(
        pixel(&butt, 300, 300),
        (0, 0, 0, 255),
        "a zero-length BUTT line has no length for its end planes to bound, so it is \
         empty — ink at the centre means the degenerate case fell through to the \
         round distance"
    );
}

/// A round-capped arc puts a disc at each sweep end; a butt-capped one does not.
///
/// plan-116-D Phase 3, and the pair is what makes it a test: an `Arc` was *butt* before
/// this letter — the sweep test already cuts the band along a radius at each end — so
/// `Butt` is the byte-identical side here and `Round` is the new geometry. That is the
/// opposite of `Line`, and getting the two backwards is the mistake this letter is
/// shaped to prevent.
///
/// The arc is centred at (300, 300), radius 100, sweeping `0.0`..`PI` — so it runs
/// below the centre (Y grows downward) and its start endpoint is at (400, 300), exactly
/// the +X extreme. Stroke width 24, so a cap disc there has half-width 12.
#[test]
fn a_round_capped_arc_caps_its_sweep_ends_and_a_butt_one_does_not() {
    let arc = |cap: &str| {
        format!(
            "  LET a AS canvas::DrawItem = canvas::Arc[x := 300.0, y := 300.0, \
             radius := 100.0, startAngle := 0.0, endAngle := 3.141592653589793, \
             cap := canvas::CapStyle.{cap}, \
             paint := canvas::stroke(canvas::rgb(255, 255, 255), 24.0)]\n  \
             canvas::present([a])\n"
        )
    };
    let (butt, _) = render("canvas_arccap_butt", &scene(&arc("Butt")));
    let (round, _) = render("canvas_arccap_round", &scene(&arc("Round")));

    // The band itself, well inside the sweep — both must draw it, or everything below
    // is vacuous.
    for (name, frame) in [("butt", &butt), ("round", &round)] {
        assert_eq!(
            pixel(frame, 300, 400),
            (255, 255, 255, 255),
            "the {name} arc did not draw the bottom of its own band"
        );
    }

    // Seven pixels above the start endpoint (400, 300). The sweep is 0..PI, so anything
    // with y < 300 is outside it and the radial cut removes it — unless a cap disc of
    // half-width 12 is centred there.
    assert_eq!(
        pixel(&round, 400, 293),
        (255, 255, 255, 255),
        "a round cap puts a disc of the stroke's half-width at the sweep endpoint, so \
         just outside the sweep is still painted"
    );
    assert_eq!(
        pixel(&butt, 400, 293),
        (0, 0, 0, 255),
        "a butt arc is cut along the radius at its end, so the same pixel is \
         background — this is the pre-plan-116-D behaviour"
    );
    // And the disc has the stroke's half-width, not some larger reach.
    assert_eq!(
        pixel(&round, 400, 285),
        (0, 0, 0, 255),
        "15 px past the endpoint is outside a 12 px cap disc"
    );
}

/// A round cap at the bounds' extreme is not clipped by the item's bounds.
///
/// The plan says to verify this rather than assume it. The arc header pads its hull by
/// `radius + half + 1.0`, and a cap disc of half-width `half` centred on a point at
/// distance `radius` from the centre reaches exactly `radius + half` — so it fits, with
/// one pixel to spare. That is an argument, not a measurement, and a hull one pixel
/// short would cut the cap's outer edge and nothing else.
///
/// The arc's start endpoint is (400, 300), the hull's +X extreme; the cap disc there
/// reaches x = 412 against a hull edge at x = 413.
#[test]
fn a_round_arc_cap_at_the_bounds_extreme_is_not_clipped() {
    let (frame, _) = render(
        "canvas_arccap_bounds",
        &scene(
            "  LET a AS canvas::DrawItem = canvas::Arc[x := 300.0, y := 300.0, \
             radius := 100.0, startAngle := 0.0, endAngle := 3.141592653589793, \
             cap := canvas::CapStyle.Round, \
             paint := canvas::stroke(canvas::rgb(255, 255, 255), 24.0)]\n  \
             canvas::present([a])\n",
        ),
    );
    assert_eq!(
        pixel(&frame, 411, 300),
        (255, 255, 255, 255),
        "the outermost column of the start cap's disc — a hull that did not grow for \
         the cap would have cut exactly here and left the rest of the arc intact"
    );
}

/// A full-circle arc looks the same either way, because it has no ends to cap.
///
/// The degenerate case for this phase. A `0..2*PI` sweep never leaves the sweep test,
/// so the cap discs are unioned into a band that already covers them — `min` with
/// something already inside changes nothing. Comparing the two frames byte for byte is
/// what rules out a disc drawn in the wrong place: on a closed arc that would be a
/// bulge, which no single-pixel check is positioned to see.
#[test]
fn a_full_circle_arc_is_identical_with_either_cap() {
    let ring = |cap: &str| {
        format!(
            "  LET a AS canvas::DrawItem = canvas::Arc[x := 300.0, y := 300.0, \
             radius := 100.0, startAngle := 0.0, endAngle := 6.283185307179586, \
             cap := canvas::CapStyle.{cap}, \
             paint := canvas::stroke(canvas::rgb(255, 255, 255), 24.0)]\n  \
             canvas::present([a])\n"
        )
    };
    let (butt, _) = render("canvas_arccap_ring_butt", &scene(&ring("Butt")));
    let (round, _) = render("canvas_arccap_ring_round", &scene(&ring("Round")));
    assert!(
        butt.iter().any(|&b| b != 0),
        "the ring drew nothing, so the comparison would be vacuous"
    );
    assert_eq!(
        butt, round,
        "a closed arc has no ends, so the cap must make no difference at all"
    );
}

/// plan-116-E Phase 1: how many Newton steps does the ellipse SDF need?
///
/// Kept and `#[ignore]`d rather than deleted, for the same reason
/// `measure_the_transformed_distance_correction` is: it measures a *design* question
/// whose answer is written into `planning/completed/plan-116-E-canvas-ellipse.md` §4.2.
/// Re-run it with
/// `cargo test --release --test rt_canvas_rasteriser -- --ignored --nocapture` if the
/// solve or the iteration count is ever revisited.
///
/// **Why the count has to be fixed rather than convergence-tested.** A
/// `WHILE |Δ| > ε` loop makes the number of steps depend on the input, which is fine
/// numerically and fatal for an oracle: the software rasteriser, Metal and Vulkan
/// would take different numbers of steps on the same pixel on different hardware, and
/// the software path would stop being predictive of the other two. So the count is
/// pinned by this measurement and shared by all three.
///
/// The ground truth is the true distance from the pixel centre to the ellipse,
/// obtained by dense sampling of the curve — not by another closed form, because the
/// closed forms this letter rejected are exactly what is being avoided. The error
/// reported is in **coverage steps of 1/255**, since that is the only thing a
/// difference in `d` can actually change: `clamp(0.5 - d, 0, 1)` quantised to 0..255.
#[test]
#[ignore = "a design measurement, not a regression gate; see plan-116-E §4.2"]
fn measure_the_ellipse_newton_iteration_count() {
    /// The true distance from `q` to the ellipse `(rx, ry)`, by dense sampling.
    ///
    /// 1 << 22 samples over the first quadrant, which at rx = 300 puts adjacent
    /// samples ~1e-4 px apart — two orders below the 1/255 coverage step this is
    /// measured against, so the ground truth is not the thing being measured.
    fn truth(qx: f64, qy: f64, rx: f64, ry: f64) -> f64 {
        // Coarse sweep to bracket the minimum, then twelve golden-section-style
        // halvings around it. Brute force at the resolution this needs (~1e-4 px at
        // rx = 300) is 4M samples per query and there are ~10^5 queries; bracket-then-
        // refine reaches the same place in ~4096 + 12·2. The distance along the curve
        // is unimodal in the first quadrant, so the bracket is sound.
        const COARSE: usize = 4096;
        let at = |t: f64| {
            let (s, c) = t.sin_cos();
            let dx = qx - rx * c;
            let dy = qy - ry * s;
            (dx * dx + dy * dy).sqrt()
        };
        let mut best_i = 0usize;
        let mut best = f64::INFINITY;
        for i in 0..=COARSE {
            let d = at(std::f64::consts::FRAC_PI_2 * (i as f64) / (COARSE as f64));
            if d < best {
                best = d;
                best_i = i;
            }
        }
        let step = std::f64::consts::FRAC_PI_2 / (COARSE as f64);
        let mut lo = (best_i.saturating_sub(1)) as f64 * step;
        let mut hi = ((best_i + 1).min(COARSE)) as f64 * step;
        for _ in 0..60 {
            let m1 = lo + (hi - lo) / 3.0;
            let m2 = hi - (hi - lo) / 3.0;
            if at(m1) < at(m2) {
                hi = m2;
            } else {
                lo = m1;
            }
        }
        let best = at((lo + hi) / 2.0).min(best);
        let inside = (qx / rx) * (qx / rx) + (qy / ry) * (qy / ry) < 1.0;
        if inside {
            -best
        } else {
            best
        }
    }

    /// §4.2's solve at `n` steps: Newton on the unit pair, never on an angle.
    fn solve(qx: f64, qy: f64, rx: f64, ry: f64, n: usize) -> f64 {
        // The seed is the gradient direction, exact in the folded first quadrant.
        let l = ((qx * rx) * (qx * rx) + (qy * ry) * (qy * ry)).sqrt();
        if l == 0.0 {
            // The exact centre. The sign test answers this without iterating.
            return -rx.min(ry);
        }
        let mut c = qx * rx / l;
        let mut s = qy * ry / l;
        for _ in 0..n {
            // Nearest-point residual: the component of (q - P) along the tangent,
            // over the second-order term. Ratios of dot products — `+ - * /` only.
            let px = rx * c;
            let py = ry * s;
            let ex = -rx * s;
            let ey = ry * c;
            let num = (qx - px) * ex + (qy - py) * ey;
            let den = ex * ex + ey * ey + (qx - px) * (rx * c) + (qy - py) * (ry * s);
            if den == 0.0 {
                break;
            }
            let delta = num / den;
            // Rotate the pair by the small-angle form and renormalise: an exact
            // rotation by atan(delta) rather than delta, i.e. a slightly damped step.
            let cp = c - s * delta;
            let sp = s + c * delta;
            let nn = (cp * cp + sp * sp).sqrt();
            c = cp / nn;
            s = sp / nn;
        }
        let dx = qx - rx * c;
        let dy = qy - ry * s;
        let d = (dx * dx + dy * dy).sqrt();
        let inside = (qx / rx) * (qx / rx) + (qy / ry) * (qy / ry) < 1.0;
        if inside {
            -d
        } else {
            d
        }
    }

    /// §4.2's **named fallback**: fixed-count bisection on the folded quadrant.
    ///
    /// Bisects the sign of `g(t) = (q - P(t)) · P'(t)`, the derivative of the squared
    /// distance. After the `|q|` fold, `g(0) = qy·ry ≥ 0` and `g(π/2) = −qx·rx ≤ 0`, so
    /// the bracket is guaranteed by construction rather than by a property of the
    /// input — which is the whole reason to prefer it over Newton here.
    ///
    /// The halving is the plan's midpoint-renormalise on the `(c, s)` pair, so no
    /// trigonometry appears: the angular midpoint of two unit vectors is their sum,
    /// normalised. Every operation is `+ - * /` and `sqrt`.
    fn bisect(qx: f64, qy: f64, rx: f64, ry: f64, n: usize) -> f64 {
        let g = |c: f64, s: f64| (qx - rx * c) * (-rx * s) + (qy - ry * s) * (ry * c);
        // The quadrant's endpoints, as (c, s) pairs.
        let (mut c0, mut s0) = (1.0f64, 0.0f64);
        let (mut c1, mut s1) = (0.0f64, 1.0f64);
        let (mut cm, mut sm) = (c0, s0);
        for _ in 0..n {
            let (cs, ss) = (c0 + c1, s0 + s1);
            let nn = (cs * cs + ss * ss).sqrt();
            cm = cs / nn;
            sm = ss / nn;
            if g(cm, sm) > 0.0 {
                c0 = cm;
                s0 = sm;
            } else {
                c1 = cm;
                s1 = sm;
            }
        }
        let dx = qx - rx * cm;
        let dy = qy - ry * sm;
        let d = (dx * dx + dy * dy).sqrt();
        let inside = (qx / rx) * (qx / rx) + (qy / ry) * (qy / ry) < 1.0;
        if inside {
            -d
        } else {
            d
        }
    }

    fn steps(a: f64, b: f64) -> f64 {
        let ca = (0.5 - a).clamp(0.0, 1.0) * 255.0;
        let cb = (0.5 - b).clamp(0.0, 1.0) * 255.0;
        (ca - cb).abs()
    }

    // Radii chosen at both ends of the plan's range, at the four eccentricities it
    // names. The 10:1 row is the one that decides `N`: the flat ends of a very
    // eccentric ellipse are where a seed from the gradient direction is furthest from
    // the true nearest point.
    //
    // 450 and 900 are past the plan's stated range deliberately: the bisection error
    // scales with the radius (the angular bracket after k halvings is
    // `(pi/2)/2^k`, so the arc it spans is proportional to r), and a canvas is 900 px
    // wide — an ellipse can legitimately be larger than the 300 the plan sampled. A
    // count chosen at 300 and deployed at 900 would be a third as accurate.
    let cases: &[(f64, f64)] = &[
        (5.0, 5.0),
        (5.0, 2.5),
        (5.0, 1.25),
        (5.0, 0.5),
        (300.0, 300.0),
        (300.0, 150.0),
        (300.0, 75.0),
        (300.0, 30.0),
        (450.0, 45.0),
        (900.0, 90.0),
    ];

    eprintln!("worst coverage error in 1/255 steps, over the antialiased band:");
    eprintln!(
        "  rx     ry     N=1      N=2      N=4      N=6      N=8     bis16    bis20    bis24"
    );
    let mut worst_by_n = [0.0f64; 9];
    let mut worst_bisect = [0.0f64; 3];
    for &(rx, ry) in cases {
        let mut row = format!("{rx:6.1} {ry:6.2}");
        // Sample the band where coverage is not saturated — |d| < 1 — since that is
        // the only place an error in `d` can move a pixel. Walk the curve and step off
        // it perpendicular. The ground truth is computed ONCE per query point and
        // reused across every N, which is what makes this run in seconds.
        const M: usize = 800;
        let mut queries = Vec::new();
        for i in 0..=M {
            let t = std::f64::consts::FRAC_PI_2 * (i as f64) / (M as f64);
            let (st, ct) = t.sin_cos();
            let bx = rx * ct;
            let by = ry * st;
            let nx = ct / rx;
            let ny = st / ry;
            let nl = (nx * nx + ny * ny).sqrt();
            for k in -4..=4 {
                let off = k as f64 * 0.25;
                let qx = (bx + nx / nl * off).abs();
                let qy = (by + ny / nl * off).abs();
                let t = truth(qx, qy, rx, ry);
                queries.push((qx, qy, t));
            }
        }
        for n in [1usize, 2, 4, 6, 8] {
            let mut worst = 0.0f64;
            for &(qx, qy, t) in &queries {
                let e = steps(solve(qx, qy, rx, ry, n), t);
                if e > worst {
                    worst = e;
                }
            }
            let slot = match n {
                1 => 1,
                2 => 2,
                4 => 3,
                6 => 4,
                _ => 5,
            };
            if worst > worst_by_n[slot] {
                worst_by_n[slot] = worst;
            }
            row.push_str(&format!(" {worst:8.4}"));
        }
        for (slot, n) in [16usize, 20, 24].iter().enumerate() {
            let mut worst = 0.0f64;
            for &(qx, qy, t) in &queries {
                let e = steps(bisect(qx, qy, rx, ry, *n), t);
                if e > worst {
                    worst = e;
                }
            }
            if worst > worst_bisect[slot] {
                worst_bisect[slot] = worst;
            }
            row.push_str(&format!(" {worst:8.4}"));
        }
        eprintln!("{row}");
    }
    eprintln!();
    for (slot, n) in [1usize, 2, 4, 6, 8].iter().enumerate() {
        eprintln!(
            "Newton N={n}: worst over all cases = {:.4} steps",
            worst_by_n[slot + 1]
        );
    }
    for (slot, n) in [16usize, 20, 24].iter().enumerate() {
        eprintln!(
            "bisection {n} halvings: worst over all cases = {:.4} steps",
            worst_bisect[slot]
        );
    }

    // The seed-basin check the plan asks for: a fixed-count Newton that starts in the
    // wrong quadrant does not converge and does not announce it. After the |q| fold
    // the seed is in the first quadrant by construction, so what is checked is that
    // the solved point stays there.
    //
    // **This assertion started as `d.is_finite() && d > 0.0` and was useless.** A
    // Newton step that converges to the stationary point on the FAR side of the
    // ellipse returns a distance that is finite and positive — it is just the wrong
    // one, six times too large. What has to be checked is agreement with the truth.
    let mut basin_failures = 0usize;
    for &(rx, ry) in cases {
        for i in 0..=200 {
            let t = std::f64::consts::FRAC_PI_2 * (i as f64) / 200.0;
            let (st, ct) = t.sin_cos();
            let qx = (rx * ct * 1.7).abs();
            let qy = (ry * st * 1.7).abs();
            let got = solve(qx, qy, rx, ry, 8);
            let want = truth(qx, qy, rx, ry);
            if (got - want).abs() > 0.01 {
                if basin_failures == 0 {
                    eprintln!(
                        "basin: Newton(8) converged to the wrong stationary point at \
                         rx={rx} ry={ry} q=({qx:.4}, {qy:.4}): got {got:.4}, want {want:.4}"
                    );
                }
                basin_failures += 1;
            }
            let got = bisect(qx, qy, rx, ry, 16);
            assert!(
                (got - want).abs() <= 0.01,
                "bisection(16) missed the nearest point at rx={rx} ry={ry} \
                 q=({qx}, {qy}): got {got}, want {want}"
            );
        }
    }
    eprintln!("basin: Newton(8) wrong-stationary-point failures = {basin_failures} of 1608");

    // The `rx == ry` seam. The question the plan asks — "is the guard introducing a
    // visible discontinuity?" — is NOT answered by comparing the solve at `ry != rx`
    // against a circle of radius `rx`: those are different shapes, and they differ by
    // about `|ry - rx|` in distance whatever the solve does. Measured that way, `ry =
    // rx·(1 + 1/4096)` at `rx = 300` reads as 18.7 steps, which is just `300/4096 ·
    // 255` and says nothing about the guard.
    //
    // What matters is (a) that the two arms agree AT the guard, where the exact float
    // compare hands over, and (b) that the difference off it goes to zero linearly
    // rather than jumping. Both are measured here.
    for &rx in &[5.0f64, 300.0, 900.0] {
        let mut worst_at = 0.0f64;
        for i in 0..=800 {
            let t = std::f64::consts::FRAC_PI_2 * (i as f64) / 800.0;
            let (st, ct) = t.sin_cos();
            for k in -4..=4 {
                let off = k as f64 * 0.25;
                let qx = (rx * ct + ct * off).abs();
                let qy = (rx * st + st * off).abs();
                let guard = (qx * qx + qy * qy).sqrt() - rx;
                let e = steps(bisect(qx, qy, rx, rx, 24), guard);
                if e > worst_at {
                    worst_at = e;
                }
            }
        }
        eprint!("seam rx={rx}: at the guard, solve vs circle arm = {worst_at:.4} steps;");
        // And off it, at three separations, to show the difference shrinks with the
        // shape difference rather than sitting at a step.
        for &denom in &[1024.0f64, 4096.0, 16384.0] {
            let ry = rx * (1.0 + 1.0 / denom);
            let mut worst = 0.0f64;
            for i in 0..=800 {
                let t = std::f64::consts::FRAC_PI_2 * (i as f64) / 800.0;
                let (st, ct) = t.sin_cos();
                for k in -4..=4 {
                    let off = k as f64 * 0.25;
                    let qx = (rx * ct + ct * off).abs();
                    let qy = (ry * st + st * off).abs();
                    let guard = (qx * qx + qy * qy).sqrt() - rx;
                    let e = steps(bisect(qx, qy, rx, ry, 24), guard);
                    if e > worst {
                        worst = e;
                    }
                }
            }
            eprint!(" 1/{denom:.0}: {worst:.3};");
        }
        eprintln!();
    }
}

/// An `Ellipse` with equal radii is **byte-identical** to the `Circle` of that radius.
///
/// plan-116-E §1's load-bearing case, and the cheapest available check on the solve
/// being right: if a 24-halving bisection and the closed-form circle distance disagree
/// anywhere, one of them is wrong about what "distance to this curve" means.
///
/// It holds *by construction* rather than by convergence — `__canvas_ellipseDistance`
/// short-circuits `rx = ry` to `sqrt(qx² + qy²) - rx`, literally the circle arm. A
/// fixed-count solve is never algebraically exact, and a last-bit residual can flip the
/// `clamp(0.5 - d, 0, 1)` quantisation on whichever edge pixel lands nearest a 1/255
/// step, so equality could not have been promised from the iteration. Phase 1 measured
/// the handover clean (0.0072 steps at `rx = 300`), which is what says the guard hides
/// nothing.
///
/// Whole frames rather than sampled pixels: the difference this guards against is a
/// one-step shift on the antialiased rim, which is exactly what spot checks miss.
#[test]
fn an_ellipse_with_equal_radii_is_identical_to_a_circle() {
    let paint = "canvas::fillStroke(canvas::rgb(90, 200, 255), canvas::rgb(255, 255, 255), 9.0)";
    let (circle, _) = render(
        "canvas_ellipse_as_circle_ref",
        &scene(&format!(
            "  LET a AS canvas::DrawItem = canvas::Circle[x := 400.0, y := 300.0, \
             radius := 120.0, paint := {paint}]\n  canvas::present([a])\n"
        )),
    );
    let (ellipse, _) = render(
        "canvas_ellipse_as_circle",
        &scene(&format!(
            "  LET a AS canvas::DrawItem = canvas::Ellipse[x := 400.0, y := 300.0, \
             radiusX := 120.0, radiusY := 120.0, angle := 0.0, paint := {paint}]\n  \
             canvas::present([a])\n"
        )),
    );
    assert!(
        circle.iter().any(|&b| b != 0),
        "the reference circle drew nothing, so the comparison would be vacuous"
    );
    assert_eq!(
        circle, ellipse,
        "an Ellipse with radiusX = radiusY and angle 0 must render byte-for-byte as \
         the Circle of that radius — a difference here is the rx == ry guard not \
         firing, or firing on a different value than the circle arm computes"
    );
}

/// An axis-aligned 3:1 ellipse covers its own extent and nothing beyond it.
///
/// The four extreme points are the ones a wrong SDF gets wrong first: an approximate
/// ellipse distance (the `(‖p/r‖ − 1)·min(rx, ry)` form this letter rejected) is worst
/// at the *flat* ends, where the curvature is lowest and the approximation's error is
/// largest. So the assertions bracket both ends of both axes, one pixel either side.
#[test]
fn an_axis_aligned_ellipse_covers_its_extent() {
    let (frame, _) = render(
        "canvas_ellipse_axis",
        &scene(
            "  LET a AS canvas::DrawItem = canvas::Ellipse[x := 400.0, y := 300.0, \
             radiusX := 300.0, radiusY := 100.0, angle := 0.0, \
             paint := canvas::fill(canvas::rgb(255, 255, 255))]\n  \
             canvas::present([a])\n",
        ),
    );
    // Inside, just short of each extreme.
    for (x, y, what) in [
        (698usize, 300usize, "the +X extreme"),
        (102, 300, "the -X extreme"),
        (400, 398, "the +Y extreme"),
        (400, 202, "the -Y extreme"),
        (400, 300, "the centre"),
    ] {
        assert_eq!(
            pixel(&frame, x, y),
            (255, 255, 255, 255),
            "{what} should be inside a 300x100 ellipse centred at (400, 300)"
        );
    }
    // Outside, just past each extreme.
    for (x, y, what) in [
        (702usize, 300usize, "past the +X extreme"),
        (98, 300, "past the -X extreme"),
        (400, 402, "past the +Y extreme"),
        (400, 198, "past the -Y extreme"),
        // The corner of the bounding box, well outside the curve — the case an
        // implementation that filled its bounds rather than its shape would fail.
        (650, 230, "the bounding box's corner region"),
    ] {
        assert_eq!(
            pixel(&frame, x, y),
            (0, 0, 0, 255),
            "{what} should be outside"
        );
    }
}

/// Rotating an ellipse by 90° swaps which axis is long.
///
/// The cheapest exact statement available about `angle`: a 3:1 ellipse turned a quarter
/// turn is the 1:3 ellipse, so the two frames must be identical. That catches a
/// rotation applied with the wrong sign, applied to the point instead of the frame, or
/// not applied at all — none of which a "does it look rotated" check would separate.
#[test]
fn rotating_an_ellipse_by_a_quarter_turn_swaps_its_axes() {
    let ell = |rx: f64, ry: f64, angle: &str| {
        format!(
            "  LET a AS canvas::DrawItem = canvas::Ellipse[x := 400.0, y := 300.0, \
             radiusX := {rx:.1}, radiusY := {ry:.1}, angle := {angle}, \
             paint := canvas::fill(canvas::rgb(255, 255, 255))]\n  \
             canvas::present([a])\n"
        )
    };
    let (turned, _) = render(
        "canvas_ellipse_turned",
        &scene(&ell(300.0, 100.0, "1.5707963267948966")),
    );
    let (swapped, _) = render("canvas_ellipse_swapped", &scene(&ell(100.0, 300.0, "0.0")));
    assert!(
        swapped.iter().any(|&b| b != 0),
        "the reference ellipse drew nothing, so the comparison would be vacuous"
    );

    // Localized rather than `assert_eq!` on the two buffers: a whole-frame equality
    // failure prints two 2.3 MB byte vectors, which is 16 MB of output and no
    // information. Count what differs and report the first one.
    let mut differing = 0usize;
    let mut worst = 0u8;
    let mut first = None;
    for i in 0..turned.len() / 4 {
        let (a, b) = (&turned[i * 4..i * 4 + 4], &swapped[i * 4..i * 4 + 4]);
        if a != b {
            differing += 1;
            let delta = (0..4).map(|k| a[k].abs_diff(b[k])).max().unwrap_or(0);
            if delta > worst {
                worst = delta;
            }
            if first.is_none() {
                first = Some((i % WIDTH, i / WIDTH, a.to_vec(), b.to_vec()));
            }
        }
    }
    // **Not byte-equality, and that is not a loosened assertion — byte-equality was
    // never the right claim here.** It would assert a property of the *trigonometry*
    // rather than of the rotation: `__canvas_cos(PI/2)` is the deterministic Taylor
    // pair's answer, not an exact zero, and it cannot be one. Its worst error over the
    // circle is 4.6e-7 (see `TRIG` in `helper_shapes.rs`), which at radius 300
    // displaces the rim by 1.4e-4 px — 0.035 of a 1/255 coverage step. A rim pixel
    // sitting within that of a quantisation boundary flips, and the sRGB encode turns
    // one coverage step into up to two output steps near mid-grey.
    //
    // Measured: **16 of 576000 pixels differ, worst channel delta 2** — first at
    // (432, 16), 211 against 210. The bound below is that measurement plus headroom,
    // and it keeps every discriminating power the test was written for: a rotation with
    // the wrong sign, applied to the shape instead of the query point, or not applied
    // at all each move *whole regions* — a 3:1 ellipse against a 1:3 one differs over
    // roughly 100,000 pixels, four orders of magnitude past this.
    assert!(
        worst <= 2 && differing * 500 < turned.len() / 4,
        "a 3:1 ellipse turned a quarter turn must be the 1:3 ellipse: {differing} \
         pixels differ (worst channel delta {worst}), first at {first:?}"
    );
    eprintln!(
        "quarter-turn vs swapped axes: {differing} of {} pixels differ by at most {worst}",
        turned.len() / 4
    );
}

/// A degenerate radius draws nothing, the same rule `Circle` follows.
#[test]
fn an_ellipse_with_a_zero_radius_draws_nothing() {
    for (name, rx, ry) in [
        ("canvas_ellipse_zero_rx", "0.0", "80.0"),
        ("canvas_ellipse_zero_ry", "80.0", "0.0"),
        ("canvas_ellipse_neg_rx", "0.0 - 5.0", "80.0"),
    ] {
        let (frame, _) = render(
            name,
            &scene(&format!(
                "  LET a AS canvas::DrawItem = canvas::Ellipse[x := 400.0, y := 300.0, \
                 radiusX := {rx}, radiusY := {ry}, angle := 0.0, \
                 paint := canvas::fill(canvas::rgb(255, 255, 255))]\n  \
                 canvas::present([a])\n"
            )),
        );
        assert!(
            frame
                .chunks_exact(4)
                .all(|p| p[0] == 0 && p[1] == 0 && p[2] == 0),
            "{name}: a degenerate radius must draw nothing at all"
        );
    }
}

/// A stroked ellipse draws a band, hollow inside and bounded outside.
///
/// The stroke rides the same `|d| - half` band every other primitive uses, so what this
/// checks is that `d` is a true *signed* distance: an unsigned or wrongly-signed one
/// would fill the interior instead of leaving it hollow, and every fill-only test above
/// would still pass.
#[test]
fn a_stroked_ellipse_is_hollow() {
    let (frame, _) = render(
        "canvas_ellipse_stroked",
        &scene(
            "  LET a AS canvas::DrawItem = canvas::Ellipse[x := 400.0, y := 300.0, \
             radiusX := 200.0, radiusY := 100.0, angle := 0.0, \
             paint := canvas::stroke(canvas::rgb(255, 255, 255), 20.0)]\n  \
             canvas::present([a])\n",
        ),
    );
    assert_eq!(
        pixel(&frame, 400, 300),
        (0, 0, 0, 255),
        "the centre of a stroked-only ellipse must be background — ink here means the \
         distance is unsigned, and the band swallowed the interior"
    );
    // The band at the +X extreme: 200 +/- 10 from the centre at x = 400.
    assert_eq!(
        pixel(&frame, 600, 300),
        (255, 255, 255, 255),
        "the band's centre line at the +X extreme"
    );
    assert_eq!(
        pixel(&frame, 585, 300),
        (0, 0, 0, 255),
        "15 px inside the +X extreme is inside the hollow"
    );
    assert_eq!(
        pixel(&frame, 615, 300),
        (0, 0, 0, 255),
        "15 px outside the +X extreme is past the band"
    );
    // And at the +Y extreme, where the curvature is highest — the band's width there
    // is the case an approximate distance gets wrong.
    assert_eq!(
        pixel(&frame, 400, 400),
        (255, 255, 255, 255),
        "the band's centre line at the +Y extreme"
    );
    assert_eq!(
        pixel(&frame, 400, 385),
        (0, 0, 0, 255),
        "15 px inside the +Y extreme is inside the hollow"
    );
}

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
             color := canvas::rgb(255, 0, 0)], canvas::GradientStop[offset := 1.0, \
             color := canvas::rgb(255, 0, 0)]]\n  \
             LET bStops AS List OF canvas::GradientStop = [canvas::GradientStop[offset := 0.0, \
             color := canvas::rgb(0, 0, 255)], canvas::GradientStop[offset := 1.0, \
             color := canvas::rgb(0, 0, 255)]]\n  \
             LET gA AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, \
             startPoint := canvas::Point[x := 100.0, y := 100.0], \
             endPoint := canvas::Point[x := 200.0, y := 100.0], stops := aStops]\n  \
             LET gB AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, \
             startPoint := canvas::Point[x := 100.0, y := 100.0], \
             endPoint := canvas::Point[x := 200.0, y := 100.0], stops := bStops]\n  \
             LET a AS canvas::DrawItem = canvas::Rectangle[x := 100.0, y := 100.0, w := 100.0, \
             h := 100.0, paint := WITH canvas::fill(canvas::rgb(255, 0, 0)) { fillGradient := gA }]\n  \
             LET b AS canvas::DrawItem = canvas::Rectangle[x := 400.0, y := 100.0, w := 100.0, \
             h := 100.0, paint := WITH canvas::fill(canvas::rgb(0, 0, 255)) { fillGradient := gB }]\n  \
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
         only the hash and __canvas_tailMatches separate them: {last}"
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
            "{extra}  LET a AS canvas::DrawItem = canvas::Rectangle[x := 100.0, y := 100.0, \
             w := 200.0, h := 150.0, paint := {}]\n  canvas::present([a])\n",
            if extra.is_empty() {
                "canvas::fill(canvas::rgb(220, 90, 40))".to_string()
            } else {
                "WITH canvas::fill(canvas::rgb(220, 90, 40)) { fillGradient := g }".to_string()
            }
        )
    };
    let (plain, _) = render("canvas_gradient_none", &scene(&flat("")));

    for (name, stops) in [
        ("canvas_gradient_zero", "[]"),
        (
            "canvas_gradient_one",
            "[canvas::GradientStop[offset := 0.0, color := canvas::rgb(0, 255, 0)]]",
        ),
    ] {
        let decl = format!(
            "  LET s AS List OF canvas::GradientStop = {stops}\n  \
             LET g AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, \
             startPoint := canvas::Point[x := 100.0, y := 100.0], \
             endPoint := canvas::Point[x := 300.0, y := 100.0], stops := s]\n"
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
             color := canvas::rgb(255, 0, 0)], canvas::GradientStop[offset := 1.0, \
             color := canvas::rgb(0, 0, 255)]]\n  \
             LET g AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, \
             startPoint := canvas::Point[x := 100.0, y := 0.0], \
             endPoint := canvas::Point[x := 500.0, y := 0.0], stops := s]\n  \
             LET r AS canvas::DrawItem = canvas::Rectangle[x := 100.0, y := 100.0, w := 400.0, \
             h := 200.0, paint := WITH canvas::fill(canvas::rgb(0, 255, 0)) { fillGradient := g }]\n  \
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
             color := canvas::rgb(255, 255, 255)], canvas::GradientStop[offset := 1.0, \
             color := canvas::rgb(0, 0, 0)]]\n  \
             LET g AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Radial, \
             startPoint := canvas::Point[x := 400.0, y := 300.0], \
             endPoint := canvas::Point[x := 520.0, y := 300.0], stops := s]\n  \
             LET c AS canvas::DrawItem = canvas::Circle[x := 400.0, y := 300.0, radius := 120.0, \
             paint := WITH canvas::fill(canvas::rgb(0, 255, 0)) { fillGradient := g }]\n  \
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
             color := canvas::rgb(255, 0, 0)], canvas::GradientStop[offset := 1.0, \
             color := canvas::rgb(255, 0, 0)]]\n  \
             LET bStops AS List OF canvas::GradientStop = [canvas::GradientStop[offset := 0.0, \
             color := canvas::rgb(0, 0, 255)], canvas::GradientStop[offset := 1.0, \
             color := canvas::rgb(0, 0, 255)]]\n  \
             LET gA AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, \
             startPoint := canvas::Point[x := 100.0, y := 100.0], \
             endPoint := canvas::Point[x := 200.0, y := 100.0], stops := aStops]\n  \
             LET gB AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, \
             startPoint := canvas::Point[x := 100.0, y := 100.0], \
             endPoint := canvas::Point[x := 200.0, y := 100.0], stops := bStops]\n  \
             LET base AS canvas::Paint = canvas::fill(canvas::rgb(0, 255, 0))\n  \
             LET a AS canvas::DrawItem = canvas::Rectangle[x := 100.0, y := 100.0, w := 100.0, \
             h := 100.0, paint := WITH base { fillGradient := gA }]\n  \
             LET b AS canvas::DrawItem = canvas::Rectangle[x := 400.0, y := 100.0, w := 100.0, \
             h := 100.0, paint := WITH base { fillGradient := gB }]\n  \
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
             color := canvas::rgb(30, 200, 90)], canvas::GradientStop[offset := 1.0, \
             color := canvas::rgb(200, 30, 90)]]\n  \
             LET g AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, \
             startPoint := canvas::Point[x := 250.0, y := 250.0], \
             endPoint := canvas::Point[x := 250.0, y := 250.0], stops := s]\n  \
             LET r AS canvas::DrawItem = canvas::Rectangle[x := 100.0, y := 100.0, w := 300.0, \
             h := 200.0, paint := WITH canvas::fill(canvas::rgb(0, 0, 0)) { fillGradient := g }]\n  \
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
             canvas::GradientStop[offset := 0.0, color := canvas::rgb(255, 0, 0)], \
             canvas::GradientStop[offset := 0.8, color := canvas::rgb(0, 255, 0)], \
             canvas::GradientStop[offset := 0.3, color := canvas::rgb(0, 0, 255)], \
             canvas::GradientStop[offset := 1.0, color := canvas::rgb(255, 255, 255)]]\n  \
             LET g AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, \
             startPoint := canvas::Point[x := 100.0, y := 0.0], \
             endPoint := canvas::Point[x := 500.0, y := 0.0], stops := s]\n  \
             LET r AS canvas::DrawItem = canvas::Rectangle[x := 100.0, y := 100.0, w := 400.0, \
             h := 100.0, paint := WITH canvas::fill(canvas::rgb(0, 0, 0)) { fillGradient := g }]\n  \
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
/// plan-116-F **F15**. `Paint.fillGradient`'s description and `06_canvas.md` both
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
             color := canvas::rgb(255, 0, 0)], canvas::GradientStop[offset := 1.0, \
             color := canvas::rgb(0, 0, 255)]]\n  \
             LET g AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, \
             startPoint := canvas::Point[x := 100.0, y := 0.0], \
             endPoint := canvas::Point[x := 500.0, y := 0.0], stops := s]\n  \
             LET r AS canvas::DrawItem = canvas::Rectangle[x := 100.0, y := 100.0, w := 400.0, \
             h := 200.0, paint := WITH canvas::fillStroke(canvas::rgb(0, 255, 0), \
             canvas::rgb(255, 255, 0), 24.0) { fillGradient := g }]\n  \
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
/// stored stops that nothing keyed on: `__canvas_hashItem` returns a bare `acc` for
/// both and `__canvas_tailMatches` returns `TRUE`, so two `Line`s differing only in
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
    const PLAIN: &str = "  LET a AS canvas::DrawItem = canvas::Line[x1 := 40.0, y1 := 60.0, \
         x2 := 360.0, y2 := 60.0, cap := canvas::CapStyle.Butt, \
         paint := canvas::stroke(canvas::rgb(255, 240, 120), 9.0)]\n  \
         LET b AS canvas::DrawItem = canvas::Arc[x := 200.0, y := 260.0, radius := 90.0, \
         startAngle := 0.0, endAngle := 3.14159, cap := canvas::CapStyle.Butt, \
         paint := canvas::stroke(canvas::rgb(120, 255, 200), 9.0)]\n  \
         canvas::present([a, b])\n";

    // The same two items, each carrying a DIFFERENT gradient. Different from each other
    // is the point: identical ones could not collide.
    const RAMPED: &str =
        "  LET s1 AS List OF canvas::GradientStop = [canvas::GradientStop[offset := 0.0, \
         color := canvas::rgb(255, 0, 0)], canvas::GradientStop[offset := 1.0, \
         color := canvas::rgb(0, 0, 255)]]\n  \
         LET s2 AS List OF canvas::GradientStop = [canvas::GradientStop[offset := 0.0, \
         color := canvas::rgb(0, 255, 0)], canvas::GradientStop[offset := 1.0, \
         color := canvas::rgb(255, 0, 255)]]\n  \
         LET g1 AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, \
         startPoint := canvas::Point[x := 40.0, y := 0.0], \
         endPoint := canvas::Point[x := 360.0, y := 0.0], stops := s1]\n  \
         LET g2 AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, \
         startPoint := canvas::Point[x := 40.0, y := 0.0], \
         endPoint := canvas::Point[x := 360.0, y := 0.0], stops := s2]\n  \
         LET a AS canvas::DrawItem = canvas::Line[x1 := 40.0, y1 := 60.0, \
         x2 := 360.0, y2 := 60.0, cap := canvas::CapStyle.Butt, \
         paint := WITH canvas::stroke(canvas::rgb(255, 240, 120), 9.0) { fillGradient := g1 }]\n  \
         LET b AS canvas::DrawItem = canvas::Arc[x := 200.0, y := 260.0, radius := 90.0, \
         startAngle := 0.0, endAngle := 3.14159, cap := canvas::CapStyle.Butt, \
         paint := WITH canvas::stroke(canvas::rgb(120, 255, 200), 9.0) { fillGradient := g2 }]\n  \
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
         entirely from `Paint.stroke` and have no interior, so `fillGradient` has no \
         fill to replace and `__canvas_paintHeader` must skip them — {differing} \
         pixels differ"
    );
}

/// A group's contents changing must produce a second frame, and that frame must show
/// the **new** contents — even though the scene list handed to `present` is
/// byte-for-byte the one already published.
///
/// This is plan-116-G §4.4's least obvious requirement, written before any of the
/// feature exists. `publishScene` decides whether to redraw by comparing the raw bytes
/// of the `DrawItem` list's data region (`emit_compare_bytes_branch` in
/// `gen_present.rs`). A `Group` node is `dx`, `dy` and `name` — and after
/// `setGroup("panel", …)` installs *different* items under the same name, all three are
/// unchanged. Two presents of the same list therefore compare equal, the second is
/// skipped, and the program draws the OLD panel forever with nothing raised: a stale
/// picture reported as success.
///
/// §4.4's answer is a parallel resolved-groups signature — `(slotIndex, revision)` per
/// resolved group node, in scene order — published and compared alongside the items.
/// This test does not care how it is done; it pins the observable consequence.
///
/// **Two assertions, and the second is the one with teeth.** The frame count alone
/// would pass for a fix that republishes but resolves to the old buffer. `A` is a red
/// box at the top-left and `A'` a green box far away at the bottom-right, so the dump —
/// which is the **last** frame, because `__canvas_presentSurface` writes it with
/// `fs::writeBytes` and that overwrites — must show green where `A'` is and background
/// where `A` was.
///
/// Un-ignored by **Phase 4**, which landed the resolution pass.
#[test]
fn a_group_replaced_between_two_identical_presents_redraws_with_the_new_contents() {
    let (frame, stats) = render(
        "canvas_group_revision",
        &scene(
            "  LET a AS canvas::DrawItem = canvas::Rectangle[x := 10.0, y := 10.0, w := 50.0, h := 50.0, \
             paint := canvas::fill(canvas::rgb(255, 0, 0))]\n  \
             canvas::setGroup(\"panel\", [a])\n  \
             LET node AS canvas::DrawItem = canvas::Group[dx := 0.0, dy := 0.0, name := \"panel\"]\n  \
             canvas::present([node])\n  \
             LET b AS canvas::DrawItem = canvas::Rectangle[x := 700.0, y := 500.0, w := 50.0, h := 50.0, \
             paint := canvas::fill(canvas::rgb(0, 255, 0))]\n  \
             canvas::setGroup(\"panel\", [b])\n  \
             canvas::present([node])\n",
        ),
    );

    assert_eq!(
        stats.len(),
        2,
        "replacing a group's contents must redraw, even though the scene list handed \
         to the second `present` is byte-identical to the first: the group's revision \
         has to reach the content comparison, or the program draws the old panel \
         forever and nothing raises. {stats:?}",
    );
    assert_eq!(
        pixel(&frame, 720, 520),
        (0, 255, 0, 255),
        "the last frame does not show the REPLACED group. A revision that reaches the \
         skip comparison but not the resolved buffer republishes and then redraws the \
         old contents — which the frame count above cannot distinguish, and this can.",
    );
    assert_ne!(
        pixel(&frame, 30, 30),
        (255, 0, 0, 255),
        "the old group's box is still on the surface, so the second frame drew the \
         previous contents on top of, or instead of, the new ones",
    );
}

/// Its sibling, and the half a careless fix breaks: with no `setGroup` between them,
/// three identical presents still draw **once**.
///
/// The cheap way to pass the test above is to stop comparing, or to fold something
/// per-frame-varying into the signature — either of which turns every group program
/// into an unconditional redraw and silently undoes the skip that
/// `rt_canvas_graphics_thread.rs`'s `an_identical_re_present_draws_no_second_frame`
/// protects for group-free scenes. Pairing the two is what makes the requirement
/// two-sided.
///
/// Un-ignored by **Phase 4**, with its sibling.
#[test]
fn three_identical_presents_of_an_unchanged_group_draw_one_frame() {
    let (_, stats) = render(
        "canvas_group_no_revision",
        &scene(
            "  LET a AS canvas::DrawItem = canvas::Rectangle[x := 10.0, y := 10.0, w := 50.0, h := 50.0, \
             paint := canvas::fill(canvas::rgb(255, 0, 0))]\n  \
             canvas::setGroup(\"panel\", [a])\n  \
             LET node AS canvas::DrawItem = canvas::Group[dx := 0.0, dy := 0.0, name := \"panel\"]\n  \
             canvas::present([node])\n  \
             canvas::present([node])\n  \
             canvas::present([node])\n",
        ),
    );

    assert_eq!(
        stats.len(),
        1,
        "three presents of an unchanged group must draw once: the resolved-groups \
         signature has to compare EQUAL when nothing changed, or every group program \
         redraws on every present forever. {stats:?}",
    );
}

/// One `name=value` field of one stats line.
fn stat(line: &str, name: &str) -> String {
    line.split_whitespace()
        .find_map(|f| f.strip_prefix(name).map(str::to_string))
        .unwrap_or_else(|| panic!("no {name} field in {line:?}"))
}

/// `setGroup` deep-copies its item list, and `removeGroup` of an absent name is a
/// no-op — both read off `MFB_CANVAS_STATS`, which `.ai/canvas-threading.md` §11 makes
/// the only window a test has onto worker-owned state (plan-116-G Phase 3).
///
/// Four frames, each asserting one thing the phase promises:
///
/// 1. nothing installed — `groups=0 groupBytes=0`, so the later numbers are deltas from
///    a known zero rather than from whatever a previous test left behind;
/// 2. one group installed — `groups=1` and `groupBytes` non-zero;
/// 3. **the caller's list mutated after installing** — `groupBytes` must be *unchanged*.
///    This is the deep copy. Publishing the caller's block would be cheaper and would
///    pass frames 1, 2 and 4; appending to that list afterwards is the only thing that
///    tells the two apart from outside;
/// 4. the group removed — `groups` drops to 0 while `groupBytes` **stays**, because
///    nothing is freed until the drain gate.
///
/// The fourth is an assertion that this phase **leaks, by construction**, and it is
/// deliberate: Phase 5's acceptance is that this number falls, and a gate with no
/// measured "before" cannot show that it moved. When Phase 5 lands, this assertion is
/// the one that has to change, and its message says so.
///
/// `removeGroup("absent")` is called in the same run rather than in a test of its own:
/// if it were not a no-op the program would raise and every assertion below would fail
/// at once, which is a clearer signal than a separate test asserting nothing happened.
#[test]
fn set_group_deep_copies_and_remove_group_frees_nothing_yet() {
    let (_, stats) = render(
        "canvas_group_deep_copy",
        &scene(
            "  LET red AS canvas::DrawItem = canvas::Rectangle[x := 10.0, y := 10.0, w := 50.0, h := 50.0, paint := canvas::fill(canvas::rgb(255, 0, 0))]\n  \
             LET green AS canvas::DrawItem = canvas::Rectangle[x := 80.0, y := 10.0, w := 50.0, h := 50.0, paint := canvas::fill(canvas::rgb(0, 255, 0))]\n  \
             LET blue AS canvas::DrawItem = canvas::Rectangle[x := 150.0, y := 10.0, w := 50.0, h := 50.0, paint := canvas::fill(canvas::rgb(0, 0, 255))]\n  \
             canvas::removeGroup(\"absent\")\n  \
             canvas::present([red])\n  \
             MUT items AS List OF canvas::DrawItem = [red]\n  \
             canvas::setGroup(\"panel\", items)\n  \
             canvas::present([red, green])\n  \
             items = collections::append(items, green)\n  \
             items = collections::append(items, blue)\n  \
             canvas::present([red, green, blue])\n  \
             canvas::removeGroup(\"panel\")\n  \
             canvas::present([green])\n",
        ),
    );

    assert_eq!(stats.len(), 4, "expected four frames: {stats:?}");
    let groups: Vec<String> = stats.iter().map(|l| stat(l, "groups=")).collect();
    let bytes: Vec<String> = stats.iter().map(|l| stat(l, "groupBytes=")).collect();

    assert_eq!(
        groups,
        vec!["0", "1", "1", "0"],
        "the table should hold nothing, then one group, then still one, then none \
         again after `removeGroup`: {stats:?}",
    );
    assert_eq!(bytes[0], "0", "the table owns nothing before any setGroup");
    assert_ne!(
        bytes[1], "0",
        "installing a group must charge its copied block to the table",
    );
    assert_eq!(
        bytes[1], bytes[2],
        "the caller appended two items to the list it passed to `setGroup` and the \
         installed group followed it — `setGroup` published the caller's block instead \
         of copying it. Nothing else in this test can tell those apart: frames 1, 2 and \
         4 pass either way.",
    );
    assert_eq!(
        bytes[3], bytes[2],
        "`removeGroup` freed the items immediately. It must not: a frame may be mid-copy \
         of that block, so it is retired and the free waits for the drain gate. \
         \n\nPhase 5 landed that gate and this assertion did **not** need to change, \
         which is worth stating because the phase expected it would: the drain runs at \
         the TOP of a present and requires a frame to have completed since the \
         retirement, so the earliest it can release this buffer is the present after \
         the one below. `a_removed_groups_buffer_is_retired_not_freed` and \
         `the_group_drain_does_not_depend_on_the_scene_changing` cover the release \
         itself.",
    );
}

/// A nested group renders at the **composed** offset, a diamond renders twice, and an
/// absent name renders nothing (plan-116-G Phase 4).
///
/// One scene rather than three, because the interesting failures are ones that would
/// pass a test of any single case: an implementation that ignored `dx`/`dy` entirely
/// draws everything at the origin, and one that used the innermost offset instead of
/// the accumulated one puts the nested group in the right *row* and the wrong column.
/// Probing four disjoint places at once separates them.
///
/// The `entries=` assertion is the other half, and it is the performance claim this
/// whole letter rests on: a group drawn at several offsets is **one** geometry cache
/// entry, because the offset is applied at draw time and never enters the cache key.
#[test]
fn a_group_renders_at_its_offset_nested_diamond_and_absent() {
    let (frame, stats) = render(
        "canvas_group_offsets",
        &scene(
            "  LET red AS canvas::DrawItem = canvas::Rectangle[x := 0.0, y := 0.0, w := 40.0, h := 40.0, paint := canvas::fill(canvas::rgb(255, 0, 0))]\n  \
             canvas::setGroup(\"leaf\", [red])\n  \
             canvas::setGroup(\"outer\", [canvas::Group[dx := 0.0, dy := 200.0, name := \"leaf\"]])\n  \
             LET a AS canvas::DrawItem = canvas::Group[dx := 100.0, dy := 100.0, name := \"leaf\"]\n  \
             LET b AS canvas::DrawItem = canvas::Group[dx := 300.0, dy := 100.0, name := \"leaf\"]\n  \
             LET nested AS canvas::DrawItem = canvas::Group[dx := 500.0, dy := 100.0, name := \"outer\"]\n  \
             LET absent AS canvas::DrawItem = canvas::Group[dx := 700.0, dy := 100.0, name := \"nope\"]\n  \
             canvas::present([a, b, nested, absent])\n",
        ),
    );

    assert_eq!(
        pixel(&frame, 110, 110),
        (255, 0, 0, 255),
        "the group did not render at its own (100,100) offset",
    );
    assert_eq!(
        pixel(&frame, 310, 110),
        (255, 0, 0, 255),
        "the SAME group did not render at a second offset — one node's offset is being \
         applied to both, or the walk emits a group once however often it is named",
    );
    assert_eq!(
        pixel(&frame, 510, 310),
        (255, 0, 0, 255),
        "the nested group did not render at the COMPOSED offset (500,100)+(0,200). Red \
         at (510,110) instead would mean the inner group's own offset was dropped; red \
         at (10,310) would mean the outer's was.",
    );
    assert_eq!(
        pixel(&frame, 710, 110),
        (0, 0, 0, 255),
        "a `Group` naming a group that was never installed must draw nothing — and must \
         not raise, which it would have before reaching this assertion",
    );
    assert_eq!(
        pixel(&frame, 10, 10),
        (0, 0, 0, 255),
        "something drew at the origin, which is where every group lands if `dx`/`dy` \
         are ignored entirely",
    );

    let entries = stats
        .first()
        .and_then(|l| {
            l.split_whitespace()
                .find_map(|f| f.strip_prefix("entries="))
        })
        .expect("an entries= field");
    assert_eq!(
        entries, "1",
        "one rectangle drawn at three offsets must be ONE geometry cache entry: the \
         offset is applied at draw time and is not part of the cache key. That is the \
         performance claim groups exist for — {stats:?}",
    );
}

/// A group nested past the depth limit raises, and so does one that references itself
/// (plan-116-G Phase 4).
///
/// The two are the same error deliberately: a cycle *is* unbounded depth, and a program
/// with a cycle and one with 65 honest levels need the same fix. Asserting they report
/// identically is what pins that decision rather than leaving it to look like an
/// accident.
///
/// The self-reference is the sharper of the two — it is one `setGroup` call, so an
/// implementation with no depth bound at all hangs or overflows the stack here rather
/// than failing an assertion.
#[test]
fn a_group_cycle_and_an_over_deep_chain_both_raise() {
    for (name, body) in [
        (
            "canvas_group_cycle",
            "  LET red AS canvas::DrawItem = canvas::Rectangle[x := 0.0, y := 0.0, w := 4.0, h := 4.0, paint := canvas::fill(canvas::rgb(255, 0, 0))]\n  \
             canvas::setGroup(\"self\", [red, canvas::Group[dx := 1.0, dy := 1.0, name := \"self\"]])\n  \
             canvas::present([canvas::Group[dx := 0.0, dy := 0.0, name := \"self\"]]) TRAP(e)\n  \
               io::print(\"raised \" & toString(e.code))\n  \
               EXIT SUB\n  \
             END TRAP\n  \
             io::print(\"NO RAISE\")\n",
        ),
        (
            "canvas_group_deep",
            "  LET red AS canvas::DrawItem = canvas::Rectangle[x := 0.0, y := 0.0, w := 4.0, h := 4.0, paint := canvas::fill(canvas::rgb(255, 0, 0))]\n  \
             canvas::setGroup(\"g0\", [red])\n  \
             MUT i AS Integer = 1\n  \
             WHILE i <= 70\n  \
               canvas::setGroup(\"g\" & toString(i), [canvas::Group[dx := 1.0, dy := 0.0, name := \"g\" & toString(i - 1)]])\n  \
               i = i + 1\n  \
             END WHILE\n  \
             canvas::present([canvas::Group[dx := 0.0, dy := 0.0, name := \"g70\"]]) TRAP(e)\n  \
               io::print(\"raised \" & toString(e.code))\n  \
               EXIT SUB\n  \
             END TRAP\n  \
             io::print(\"NO RAISE\")\n",
        ),
    ] {
        let (stdout, _) = render_stdout(name, &scene(body));
        assert!(
            stdout.contains("raised 77050024"),
            "{name} must raise ErrDepthExceeded (7-705-0024) from `present`; got: \
             {stdout:?}. \"NO RAISE\" means the depth bound is missing, and for the \
             cycle case that would otherwise recurse until the stack ends.",
        );
    }
}

/// A gradient inside a group **moves with the group** — settling plan-116-G §4.5's open
/// decision, and pinning it with the scene that can tell the two answers apart.
///
/// The decision was genuinely open. `Paint.fillGradient` is evaluated at the surface
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
            "  LET stops AS List OF canvas::GradientStop = [canvas::GradientStop[offset := 0.0, color := canvas::rgb(255, 0, 0)], canvas::GradientStop[offset := 1.0, color := canvas::rgb(0, 0, 255)]]\n  \
             LET g AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, startPoint := canvas::Point[x := 0.0, y := 0.0], endPoint := canvas::Point[x := 200.0, y := 0.0], stops := stops]\n  \
             LET bar AS canvas::DrawItem = canvas::Rectangle[x := 0.0, y := 0.0, w := 200.0, h := 60.0, paint := WITH canvas::fill(canvas::rgb(0, 0, 0)) { fillGradient := g }]\n  \
             canvas::setGroup(\"bar\", [bar])\n  \
             LET left AS canvas::DrawItem = canvas::Group[dx := 50.0, dy := 100.0, name := \"bar\"]\n  \
             LET right AS canvas::DrawItem = canvas::Group[dx := 400.0, dy := 300.0, name := \"bar\"]\n  \
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

/// A clipped item inside a translated group keeps its clip **where the surface
/// rectangle is** — the group moves the shape through the clip, not the clip with it
/// (plan-116-G §4.5, **G5**).
///
/// `Paint.clip` is defined as a surface rectangle that `Paint.transform` does not move
/// (plan-116-B), and a group offset is treated the same way. This is the asymmetry G5
/// flags: "evaluate the distance at `p - offset`" moves everything it reaches, and the
/// clip must be one of the two things it deliberately does not.
///
/// The scene is arranged so the two answers are visibly different rather than subtly:
/// a 200x200 square in a group translated by (300,0), clipped to the left half of where
/// it lands. If the clip moved with the group it would land 300px further right and the
/// square would be fully painted; if it stayed, the square's right half is cut.
#[test]
fn a_clip_inside_a_translated_group_stays_on_the_surface() {
    let (frame, _) = render(
        "canvas_group_clip",
        &scene(
            "  LET clipped AS canvas::DrawItem = canvas::Rectangle[x := 0.0, y := 0.0, w := 200.0, h := 200.0, paint := WITH canvas::fill(canvas::rgb(0, 200, 255)) { clip := canvas::Bounds[x := 300.0, y := 100.0, w := 100.0, h := 200.0] }]\n  \
             canvas::setGroup(\"g\", [clipped])\n  \
             canvas::present([canvas::Group[dx := 300.0, dy := 100.0, name := \"g\"]])\n",
        ),
    );

    // The square lands at (300,100)-(500,300). The clip covers (300,100)-(400,300).
    assert_eq!(
        pixel(&frame, 350, 200),
        (0, 200, 255, 255),
        "the left half of the square — inside both the shape and the clip — is not painted",
    );
    assert_eq!(
        pixel(&frame, 450, 200),
        (0, 0, 0, 255),
        "the right half of the square is painted, so the clip moved with the group. A \
         clip is a SURFACE rectangle: the group translates the shape through it, and \
         following the group would put this clip at (600,100) where it cuts nothing.",
    );
}

/// A removed group's buffer is **retired, not freed** (plan-116-G Phase 5).
///
/// The free cannot happen inside `removeGroup`: `canvas::groupItems` copies out of that
/// block on the graphics thread, so a render may be part-way through it. The block is
/// therefore handed to the drain gate, which releases it once a frame has completed —
/// the same rule `.ai/canvas-threading.md` §3 gives for retired scene blocks and §7 for
/// textures.
///
/// Observable because both states produce a frame: the name disappears immediately
/// (`groups=` drops), while the bytes stay charged (`groupBytes=` does not).
#[test]
fn a_removed_groups_buffer_is_retired_not_freed() {
    let (_, stats) = render(
        "canvas_group_retire",
        &scene(
            "  LET red AS canvas::DrawItem = canvas::Rectangle[x := 0.0, y := 0.0, w := 40.0, h := 40.0, paint := canvas::fill(canvas::rgb(255, 0, 0))]\n  \
             LET keep AS canvas::DrawItem = canvas::Circle[x := 700.0, y := 500.0, radius := 20.0, paint := canvas::fill(canvas::rgb(0, 255, 0))]\n  \
             canvas::setGroup(\"panel\", [red])\n  \
             LET node AS canvas::DrawItem = canvas::Group[dx := 100.0, dy := 100.0, name := \"panel\"]\n  \
             canvas::present([keep, node])\n  \
             canvas::removeGroup(\"panel\")\n  \
             canvas::present([keep, node])\n",
        ),
    );

    assert_eq!(stats.len(), 2, "expected two frames: {stats:?}");
    let groups: Vec<String> = stats.iter().map(|l| stat(l, "groups=")).collect();
    let bytes: Vec<String> = stats.iter().map(|l| stat(l, "groupBytes=")).collect();

    assert_eq!(
        groups,
        vec!["1", "0"],
        "the name must go the moment `removeGroup` is called: {stats:?}",
    );
    assert_ne!(bytes[0], "0", "the installed group must own bytes");
    assert_eq!(
        bytes[1], bytes[0],
        "the buffer was freed the instant `removeGroup` was called. It must be RETIRED \
         instead — a render may be mid-copy of exactly that block — and released only \
         once a frame has completed since: {stats:?}",
    );
}

/// The drain does not depend on the scene ever changing again (**G7**).
///
/// This is the row the placement of the gate is decided by, and it is written as a
/// **memory bound** rather than as a single free, because a single free cannot tell the
/// two placements apart: whichever present eventually publishes will run either gate.
///
/// The loop installs and removes a group under a name **the presented scene never
/// references**, so the scene is byte-identical every time and the signature never
/// moves — every one of those presents is skipped, and no publish happens at all. With
/// the free placed beside the scene ring's `emit_reclaim_retired`, which sits after the
/// publish label, nothing would ever be released and the table would accumulate one
/// buffer per iteration. With the gate at the top of `present`, each iteration's
/// predecessor is drained on the next call.
///
/// The final present changes the scene only so that a stats line is written to read the
/// answer off; by then the bound has already been established or lost.
#[test]
fn the_group_drain_does_not_depend_on_the_scene_changing() {
    let (_, stats) = render(
        "canvas_group_drain_bound",
        &scene(
            "  LET red AS canvas::DrawItem = canvas::Rectangle[x := 0.0, y := 0.0, w := 40.0, h := 40.0, paint := canvas::fill(canvas::rgb(255, 0, 0))]\n  \
             LET keep AS canvas::DrawItem = canvas::Circle[x := 700.0, y := 500.0, radius := 20.0, paint := canvas::fill(canvas::rgb(0, 255, 0))]\n  \
             canvas::present([keep])\n  \
             MUT i AS Integer = 0\n  \
             WHILE i < 200\n  \
               canvas::setGroup(\"scratch\" & toString(i), [red, red, red, red, red, red, red, red])\n  \
               canvas::present([keep])\n  \
               canvas::removeGroup(\"scratch\" & toString(i))\n  \
               canvas::present([keep])\n  \
               i = i + 1\n  \
             END WHILE\n  \
             LET last AS canvas::DrawItem = canvas::Circle[x := 300.0, y := 300.0, radius := 20.0, paint := canvas::fill(canvas::rgb(0, 0, 255))]\n  \
             canvas::present([keep, last])\n",
        ),
    );

    let bytes: i64 = stat(stats.last().expect("a final frame"), "groupBytes=")
        .parse()
        .expect("groupBytes is a number");
    let groups = stat(stats.last().unwrap(), "groups=");
    assert_eq!(groups, "0", "every scratch group was removed: {stats:?}");
    // One eight-item group is ~1 KB, so 200 undrained ones would be ~200 KB. The bound
    // allows a single outstanding buffer: the last iteration's may legitimately still
    // be retired, since the gate needs a frame to complete after it.
    assert!(
        bytes < 4096,
        "the table still owns {bytes} bytes after 200 install/remove cycles, so the \
         buffers were not drained. Every present in that loop showed an unchanged scene \
         and was skipped, which is exactly the case a free placed on the publish path \
         never reaches — the frame skip works and the memory is held anyway. {stats:?}",
    );
}

/// Replacing a live group frees the **old** buffer and not the new one, once a frame
/// has completed (plan-116-G Phase 5).
///
/// The sharp part is the second half: after the drain, `groupBytes` must equal what one
/// copy of the new contents costs — not zero (which would mean the live buffer was
/// freed too) and not the sum of both (which would mean the old one never was). The new
/// group is deliberately a *different size* from the old, so "one copy of the new
/// contents" is a number only the correct implementation produces.
#[test]
fn replacing_a_group_frees_only_the_displaced_buffer() {
    let (_, stats) = render(
        "canvas_group_replace_drain",
        &scene(
            "  LET a AS canvas::DrawItem = canvas::Rectangle[x := 0.0, y := 0.0, w := 40.0, h := 40.0, paint := canvas::fill(canvas::rgb(255, 0, 0))]\n  \
             LET b AS canvas::DrawItem = canvas::Circle[x := 10.0, y := 10.0, radius := 10.0, paint := canvas::fill(canvas::rgb(0, 255, 0))]\n  \
             LET node AS canvas::DrawItem = canvas::Group[dx := 100.0, dy := 100.0, name := \"panel\"]\n  \
             canvas::setGroup(\"one\", [a])\n  \
             canvas::present([canvas::Group[dx := 400.0, dy := 400.0, name := \"one\"]])\n  \
             canvas::setGroup(\"panel\", [a, a, a])\n  \
             canvas::present([canvas::Group[dx := 400.0, dy := 400.0, name := \"one\"], node])\n  \
             canvas::setGroup(\"panel\", [b])\n  \
             canvas::present([canvas::Group[dx := 400.0, dy := 400.0, name := \"one\"], node])\n  \
             canvas::present([canvas::Group[dx := 400.0, dy := 400.0, name := \"one\"], node])\n",
        ),
    );

    assert_eq!(stats.len(), 4, "expected four frames: {stats:?}");
    let n: Vec<i64> = stats
        .iter()
        .map(|l| stat(l, "groupBytes=").parse().unwrap())
        .collect();
    let groups: Vec<String> = stats.iter().map(|l| stat(l, "groups=")).collect();

    assert_eq!(
        groups,
        vec!["1", "2", "2", "2"],
        "the replacement reuses the slot, so the name count must not move: {stats:?}",
    );
    assert!(
        n[2] > n[1],
        "replacing a group must charge the new copy before the old one drains — both \
         are held for one frame, which is what the drain gate costs: {stats:?}",
    );
    assert!(
        n[3] < n[2],
        "the displaced buffer was never freed: {stats:?}",
    );
    assert!(
        n[3] > 0,
        "everything was freed, including the LIVE buffer — a replace must retire only \
         the block it displaced: {stats:?}",
    );
    // The three-item group is gone and the one-item group replaced it, so the total
    // must be below what the frame carrying both cost, and below the pre-replace total.
    assert!(
        n[3] < n[1],
        "after the drain the table should hold the one-item replacement plus the \
         unrelated group, which is less than the three-item version it replaced: {stats:?}",
    );
}

/// A group removed while a **parent group** still names it keeps drawing.
///
/// The parent holds no pointer into the child's buffer — a `canvas::Group` node carries
/// a name, and the renderer resolves it per frame — so "still referenced" here means the
/// name resolves, and removing the child makes the parent's node a silent no-op like any
/// other unresolved name. This asserts that outcome rather than a refcount, because the
/// refcount is what the design does not have (**G24**).
#[test]
fn removing_a_group_a_parent_names_makes_the_parents_node_a_no_op() {
    let (frame, _) = render(
        "canvas_group_child_removed",
        &scene(
            "  LET red AS canvas::DrawItem = canvas::Rectangle[x := 0.0, y := 0.0, w := 60.0, h := 60.0, paint := canvas::fill(canvas::rgb(255, 0, 0))]\n  \
             LET mark AS canvas::DrawItem = canvas::Rectangle[x := 700.0, y := 500.0, w := 60.0, h := 60.0, paint := canvas::fill(canvas::rgb(0, 0, 255))]\n  \
             canvas::setGroup(\"child\", [red])\n  \
             canvas::setGroup(\"parent\", [canvas::Group[dx := 0.0, dy := 0.0, name := \"child\"], mark])\n  \
             LET node AS canvas::DrawItem = canvas::Group[dx := 100.0, dy := 100.0, name := \"parent\"]\n  \
             canvas::present([node])\n  \
             canvas::removeGroup(\"child\")\n  \
             canvas::present([node])\n",
        ),
    );

    assert_eq!(
        pixel(&frame, 130, 130),
        (0, 0, 0, 255),
        "the removed child still drew. Removing a name makes every `canvas::Group` that \
         referenced it a no-op, including one inside another group.",
    );
    assert_eq!(
        pixel(&frame, 830, 630),
        (0, 0, 255, 255),
        "the parent group's OTHER item stopped drawing, so removing the child took the \
         parent down with it",
    );
}

/// A group removed while the graphics thread is **mid-frame** over it: the in-flight
/// frame completes normally and draws what it started with (plan-116-G Phase 5).
///
/// This is the row **G9** was written about, and it is deterministic rather than
/// probabilistic because this phase built the affordance G9 asked for.
/// `MFB_CANVAS_FRAME_HOLD_MS` parks the graphics thread inside `__canvas_renderFrame`,
/// immediately after `__canvas_sceneOffsets` has resolved every group name and copied
/// out its items — so the `removeGroup` below lands while a frame is demonstrably still
/// working from that block. Before the affordance, every proven "mid-render" row got
/// there through `MFB_CANVAS_RESIZE_W`/`_H` firing while the worker slept, which is
/// resize-specific; a row needing a different worker action was tested by luck.
///
/// `MFB_CANVAS_SYNC` is deliberately **off**: the whole point is that `present` returns
/// while the frame is still being drawn, so the worker can reach `removeGroup`.
///
/// What it asserts is that the frame completes and the picture is right — a
/// use-after-free here shows up as a crash, a torn frame, or nothing drawn where the
/// group was. It cannot assert the *timing* it arranged, so the hold is long enough
/// (600ms against a frame measured in single-digit ms) that the ordering is not in
/// doubt.
///
/// The worker's 120ms sleep before `removeGroup` is the other half of that ordering and
/// is not padding. Without it the worker races the graphics thread to the *start* of the
/// frame and usually wins — `present` returns as soon as it has signalled — so the group
/// is removed before it is ever resolved and the frame correctly draws nothing. That
/// tests the absent-name path, not this one, and it is what the first run of this test
/// measured.
#[test]
fn removing_a_group_mid_frame_lets_the_frame_finish() {
    let project = common::temp_project(
        "canvas_group_race",
        &scene(
            "  LET red AS canvas::DrawItem = canvas::Rectangle[x := 0.0, y := 0.0, w := 80.0, h := 80.0, paint := canvas::fill(canvas::rgb(255, 0, 0))]\n  \
             canvas::setGroup(\"panel\", [red])\n  \
             LET node AS canvas::DrawItem = canvas::Group[dx := 200.0, dy := 200.0, name := \"panel\"]\n  \
             LET mark AS canvas::DrawItem = canvas::Circle[x := 700.0, y := 500.0, radius := 30.0, paint := canvas::fill(canvas::rgb(0, 255, 0))]\n  \
             canvas::present([mark, node])\n  \
             os::sleep(120)\n  \
             canvas::removeGroup(\"panel\")\n  \
             os::sleep(1200)\n",
        )
        .replace("IMPORT collections", "IMPORT collections\nIMPORT os"),
    );
    let frame_path = project.join("frame.rgba");
    let binary = common::build_app(&project, "canvas_group_race");
    let run = Command::new(&binary)
        .env("MFB_MACAPP_HEADLESS", "1")
        .env("MFB_WINAPP_HEADLESS", "1")
        // The GTK one too. Omitting it is invisible on the macOS dev host, where the
        // MACAPP flag is the one that matters; on a Linux box the program tries to open
        // a display, fails, and exits 1 -- which these tests then report as a
        // use-after-free or a missing raise. `render` above has always set all three.
        .env("MFB_GTKAPP_HEADLESS", "1")
        .env("MFB_CANVAS_DUMP", &frame_path)
        .env("MFB_CANVAS_FRAME_HOLD_MS", "600")
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", binary.display()));
    assert!(
        run.status.success(),
        "the program did not exit cleanly — a use-after-free on the retired group buffer \
         is what this row exists to catch, and it presents as a signal here. exit {:?}\n{}\n{}",
        run.status.code(),
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr),
    );

    let frame =
        std::fs::read(&frame_path).expect("the in-flight frame must still have been written");
    assert_eq!(
        frame.len(),
        WIDTH * HEIGHT * 4,
        "the frame was torn or truncated",
    );
    assert_eq!(
        pixel(&frame, 240, 240),
        (255, 0, 0, 255),
        "the frame that was already drawing when `removeGroup` arrived did not finish \
         drawing the group. It had already copied the items, so removing the name must \
         not affect it — the retired block stays valid until a frame completes.",
    );
    assert_eq!(
        pixel(&frame, 700, 500),
        (0, 255, 0, 255),
        "the rest of the frame was lost",
    );
    let _ = std::fs::remove_dir_all(&project);
}

/// The program exits while a frame is still drawing a group: no use-after-free
/// (plan-116-G Phase 5 — R12's group analogue).
///
/// R12's own hazard is that the scene slots live in the worker's arena and the worker's
/// arena state lives on its *stack frame*, so a graphics thread still rendering after
/// the entry returns reads freed stack. A group's buffer is in the same arena, and this
/// phase gave the graphics thread a new reason to be inside one — `canvas::groupItems`
/// copies out of it — so the row is re-run against a group rather than assumed to be
/// covered.
///
/// `MFB_CANVAS_FRAME_HOLD_MS` is what makes it a real test rather than a hopeful one:
/// the frame is *guaranteed* to still be in progress when `main` returns, because the
/// graphics thread is parked in the middle of it.
#[test]
fn exiting_while_a_frame_draws_a_group_is_clean() {
    let project = common::temp_project(
        "canvas_group_exit_race",
        &scene(
            "  LET red AS canvas::DrawItem = canvas::Rectangle[x := 0.0, y := 0.0, w := 80.0, h := 80.0, paint := canvas::fill(canvas::rgb(255, 0, 0))]\n  \
             canvas::setGroup(\"panel\", [red])\n  \
             canvas::present([canvas::Group[dx := 200.0, dy := 200.0, name := \"panel\"]])\n",
        ),
    );
    let binary = common::build_app(&project, "canvas_group_exit_race");
    let run = Command::new(&binary)
        .env("MFB_MACAPP_HEADLESS", "1")
        .env("MFB_WINAPP_HEADLESS", "1")
        // The GTK one too. Omitting it is invisible on the macOS dev host, where the
        // MACAPP flag is the one that matters; on a Linux box the program tries to open
        // a display, fails, and exits 1 -- which these tests then report as a
        // use-after-free or a missing raise. `render` above has always set all three.
        .env("MFB_GTKAPP_HEADLESS", "1")
        .env("MFB_CANVAS_FRAME_HOLD_MS", "400")
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", binary.display()));
    assert!(
        run.status.success(),
        "the program did not exit cleanly with a frame in flight over a group. Shutdown \
         must drain the pending frame and join the graphics thread before the worker's \
         entry unwinds — its arena, which holds the group's buffer, is on that stack \
         frame. exit {:?}\n{}\n{}",
        run.status.code(),
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr),
    );
    assert!(
        String::from_utf8_lossy(&run.stdout).contains("rendered"),
        "the program did not reach the end of `main`",
    );
    let _ = std::fs::remove_dir_all(&project);
}

/// A group offset far outside the surface draws nothing and does not **raise**
/// (plan-116-G).
///
/// `dx`/`dy` are user-supplied `Float`s that this letter feeds into two new integer
/// conversions: `__canvas_hashFloat` folds them into the draw hash as
/// `toInt(value * 65536.0)`, and the glyph path takes `toInt(gdx)` to move a run's
/// origin. A conversion whose result does not fit raises `7-705-0010` — arithmetic
/// overflow or numeric conversion outside the destination range — and it would raise
/// from `canvas::present`, which no caller expects to fail because a shape was placed
/// off-screen.
///
/// Written after a peer found exactly that error class in the canvas font path on
/// linux-aarch64, where it raises rather than corrupting. That is not this code and this
/// test does not chase it; it pins that *this* letter's new conversions are not another
/// instance, on a value a program can hand them directly.
///
/// `1.0e9` is chosen to be absurd rather than borderline: multiplied by 65536 it is
/// ~6.5e13, comfortably past a 32-bit destination and comfortably inside a 64-bit one,
/// so the test states which of those the conversion actually uses.
#[test]
fn a_group_offset_far_off_surface_draws_nothing_and_does_not_raise() {
    let (frame, stats) = render(
        "canvas_group_huge_offset",
        &scene(
            "  LET red AS canvas::DrawItem = canvas::Rectangle[x := 0.0, y := 0.0, w := 40.0, h := 40.0, paint := canvas::fill(canvas::rgb(255, 0, 0))]\n  \
             LET here AS canvas::DrawItem = canvas::Rectangle[x := 100.0, y := 100.0, w := 40.0, h := 40.0, paint := canvas::fill(canvas::rgb(0, 255, 0))]\n  \
             canvas::setGroup(\"panel\", [red])\n  \
             LET huge AS canvas::DrawItem = canvas::Group[dx := 1.0e9, dy := 0.0 - 1.0e9, name := \"panel\"]\n  \
             LET big AS canvas::DrawItem = canvas::Group[dx := 100000.0, dy := 100000.0, name := \"panel\"]\n  \
             canvas::present([here, huge, big])\n",
        ),
    );

    // Reaching here at all is most of the assertion: a raise from `present` fails the
    // harness in `render`, which asserts the program exited successfully.
    assert_eq!(stats.len(), 1, "expected one frame: {stats:?}");
    assert_eq!(
        pixel(&frame, 110, 110),
        (0, 255, 0, 255),
        "the in-surface item was lost, so the off-surface groups did more than draw \
         nothing",
    );
    assert_eq!(
        pixel(&frame, 10, 10),
        (0, 0, 0, 255),
        "something was drawn at the origin — an offset that overflowed its conversion \
         and wrapped would land somewhere arbitrary, and the origin is the most likely \
         somewhere",
    );
}

/// A `Polygon` inside a translated group draws at the offset (plan-116-G).
///
/// Every other group test here uses a fixed-tail kind — `Rectangle`, `Circle`, `Text`.
/// A polygon is the only shape whose geometry record has a **variable-length tail**: its
/// points live past the 47-slot header as edges, and `__canvas_geoDistance` reads them
/// from `offset + __CANVAS_GEO_HEADER`. The group offset moves the query point rather
/// than the record, so those edge coordinates are consumed in *shape* space while the
/// bounds that select the pixels are in *surface* space — the one place this letter's
/// two directions meet on the same item.
///
/// Two polygons, deliberately: `__canvas_hashItem` folds a polygon's points in by hand
/// because two different polygons can share a header (same bounds, same count, same
/// paint), so a scene with one polygon cannot tell a correct per-item tail from a shared
/// one. Here they differ only in shape, not in bounding box, which is exactly the
/// collision that motivated the hand-folding.
#[test]
fn a_polygon_inside_a_translated_group_draws_at_the_offset() {
    let (frame, stats) = render(
        "canvas_group_polygon",
        &scene(
            "  LET up AS canvas::DrawItem = canvas::Polygon[points := [canvas::Point[x := 0.0, y := 0.0], canvas::Point[x := 100.0, y := 0.0], canvas::Point[x := 50.0, y := 100.0]], paint := canvas::fill(canvas::rgb(255, 0, 0))]\n  \
             LET down AS canvas::DrawItem = canvas::Polygon[points := [canvas::Point[x := 0.0, y := 100.0], canvas::Point[x := 100.0, y := 100.0], canvas::Point[x := 50.0, y := 0.0]], paint := canvas::fill(canvas::rgb(0, 255, 0))]\n  \
             canvas::setGroup(\"tri\", [up])\n  \
             canvas::setGroup(\"tri2\", [down])\n  \
             canvas::present([canvas::Group[dx := 200.0, dy := 200.0, name := \"tri\"], canvas::Group[dx := 500.0, dy := 200.0, name := \"tri2\"]])\n",
        ),
    );

    // `up` is widest at its top edge: at (250, 205) — 5px down from the apex row — the
    // triangle spans roughly x 202..298, so its centre column is inside.
    assert_eq!(
        pixel(&frame, 250, 210),
        (255, 0, 0, 255),
        "the up-pointing polygon did not draw inside its group at (200,200). Its edges \
         live in the record's variable-length tail and are read in shape space, so a \
         group offset applied to the record rather than to the query point puts the \
         shape somewhere else entirely.",
    );
    assert_eq!(
        pixel(&frame, 550, 290),
        (0, 255, 0, 255),
        "the down-pointing polygon did not draw inside its group at (500,200)",
    );
    // The two are different shapes in the same 100x100 box, so a per-item tail is the
    // only thing that keeps them apart: `up` is empty near its bottom corners, `down` is
    // filled there.
    assert_eq!(
        pixel(&frame, 210, 290),
        (0, 0, 0, 255),
        "the up-pointing polygon is filled at its bottom-left corner, so it drew the \
         OTHER polygon's tail — two polygons can share a header, which is why \
         `__canvas_hashItem` folds a polygon's points in by hand",
    );
    assert_eq!(
        pixel(&frame, 510, 290),
        (0, 255, 0, 255),
        "the down-pointing polygon is empty at its bottom-left corner, so the tails were \
         crossed the other way",
    );

    // Two distinct polygons, two cache entries — and the offset is not part of the key.
    let entries = stats
        .first()
        .and_then(|l| {
            l.split_whitespace()
                .find_map(|f| f.strip_prefix("entries="))
        })
        .expect("an entries= field");
    assert_eq!(
        entries, "2",
        "expected one cache entry per distinct polygon: {stats:?}",
    );
}

/// Installing and removing groups under **many distinct names** does not grow the arena
/// without bound (plan-116-G).
///
/// `setGroup` copies the caller's name into the arena, because the table outlives the
/// caller's binding. That copy has to be released too — and it was not: `removeGroup`
/// zeroed the name pointer and a replacing `setGroup` overwrote it, leaving the block
/// unreachable either way.
///
/// **`groupBytes=` could not see this until the fix widened it**, which is why the
/// existing drain test passed against the leak: the counter charged the *items* block
/// only, so a run that leaked 200 name copies reported a table owning nothing. Making
/// the counter mean "every byte this table is responsible for" is half the fix, and it
/// is what turns this test into a direct measurement rather than an indirect one — a
/// counter that does not cover everything the table owns cannot detect the table owning
/// too much.
///
/// Measured both ways before being trusted: with the retire suppressed this reports
/// **16,530 bytes** still owned after the loop, and with it restored, zero.
///
/// Names are long and distinct on purpose. A short name shares the arena's small-block
/// behaviour with everything else in the frame; a 67-byte one leaked 400 times (each
/// name is installed, replaced, then removed) is the ~16 KB above, on a path a
/// long-running program hits every time it rebuilds its groups.
#[test]
fn installing_and_removing_many_named_groups_does_not_grow_without_bound() {
    let (_, stats) = render(
        "canvas_group_name_churn",
        &scene(
            "  LET red AS canvas::DrawItem = canvas::Rectangle[x := 0.0, y := 0.0, w := 40.0, h := 40.0, paint := canvas::fill(canvas::rgb(255, 0, 0))]\n  \
             LET keep AS canvas::DrawItem = canvas::Circle[x := 700.0, y := 500.0, radius := 20.0, paint := canvas::fill(canvas::rgb(0, 255, 0))]\n  \
             LET pad AS String = \"a-deliberately-long-group-name-so-a-leaked-copy-is-worth-measuring-\"\n  \
             canvas::present([keep])\n  \
             MUT i AS Integer = 0\n  \
             WHILE i < 200\n  \
               LET n AS String = pad & toString(i)\n  \
               canvas::setGroup(n, [red])\n  \
               canvas::present([keep])\n  \
               canvas::setGroup(n, [red, red])\n  \
               canvas::present([keep])\n  \
               canvas::removeGroup(n)\n  \
               canvas::present([keep])\n  \
               i = i + 1\n  \
             END WHILE\n  \
             LET last AS canvas::DrawItem = canvas::Circle[x := 300.0, y := 300.0, radius := 20.0, paint := canvas::fill(canvas::rgb(0, 0, 255))]\n  \
             canvas::present([keep, last])\n",
        ),
    );

    let last = stats.last().expect("a final frame");
    assert_eq!(
        stat(last, "groups="),
        "0",
        "every name was removed, so the table should hold none: {stats:?}",
    );
    let bytes: i64 = stat(last, "groupBytes=").parse().expect("a number");
    assert!(
        bytes < 4096,
        "the table still owns {bytes} bytes after 200 install/replace/remove cycles. \
         The items drain on their own gate, so this is almost certainly the interned \
         NAME copies: `setGroup` copies the caller's name into the arena because the \
         table outlives the caller's binding, and both `removeGroup` and a replacing \
         `setGroup` have to retire that copy rather than just overwrite the pointer. \
         Suppressing the retire reports 16530 here: {stats:?}",
    );
}

/// `__canvas_sceneDraws` produces the expected `(base, count, dx, dy)` sequence, and a
/// **diamond's two draws name the same item base** (plan-116-H Phase 1).
///
/// That last property is what this whole letter is arranged around, and it is the
/// decision §4.3 asked Phase 1 to make: a shared group's blocks are written **once** and
/// referenced, not once per reference. The alternative would duplicate the blocks, at
/// which point a per-draw offset earns nothing — the translation could simply be baked
/// into each copy as it is written, and the push constant this letter adds would be
/// unnecessary machinery.
///
/// Read off `MFB_CANVAS_STATS`, which is the only window onto a structure built on the
/// graphics thread and handed straight to an emitter. `blocks=` is how many item blocks
/// the frame uploads; `draws=` is `base:count:dx:dy:mode` per entry, `|`-separated, with the
/// offsets in 16.16 as stored — 100.0 is `6553600`.
///
/// The four cases are one test rather than four because the interesting failures are
/// relative: a walk that emitted per-reference blocks passes the flat and single-group
/// cases unchanged and only diverges on the diamond, and a walk that dropped the
/// composed offset passes everything except the nested case.
#[test]
fn scene_draws_shares_one_base_between_a_diamonds_two_draws() {
    let d = |body: &str| -> (String, String) {
        let (_, stats) = render("canvas_scene_draws", &scene(body));
        let line = stats.last().expect("a frame").clone();
        (stat(&line, "blocks="), stat(&line, "draws="))
    };
    const RED: &str = "  LET red AS canvas::DrawItem = canvas::Rectangle[x := 0.0, y := 0.0, w := 40.0, h := 40.0, paint := canvas::fill(canvas::rgb(255, 0, 0))]\n  ";

    // 1. A flat scene: one run, no offset.
    let (blocks, draws) = d(&format!(
        "{RED}LET b AS canvas::DrawItem = canvas::Circle[x := 300.0, y := 300.0, radius := 20.0, paint := canvas::fill(canvas::rgb(0, 255, 0))]\n  canvas::present([red, b])\n"
    ));
    assert_eq!(
        (blocks.as_str(), draws.as_str()),
        ("2", "0:2:0:0:0"),
        "a group-free scene must be one run of every item at no offset — the shape this \
         letter must not change for scenes that use no groups. The trailing field is the \
         run's BLEND MODE, which Phase 2 added to `__canvas_drawsText` because it selects \
         the pipeline: a draw list can read correct in base, count and offset and still \
         bind the wrong program",
    );

    // 2. One group at (100, 100): its two items written once, one draw at the offset.
    let (blocks, draws) = d(&format!(
        "{RED}canvas::setGroup(\"g\", [red, red])\n  canvas::present([canvas::Group[dx := 100.0, dy := 100.0, name := \"g\"]])\n"
    ));
    assert_eq!(
        (blocks.as_str(), draws.as_str()),
        ("2", "0:2:6553600:6553600:0"),
        "one group should be its own items once, drawn at its offset in 16.16, in \
         Normal blend mode",
    );

    // 3. A nested group: the inner run is drawn at the COMPOSED offset.
    let (blocks, draws) = d(&format!(
        "{RED}canvas::setGroup(\"inner\", [red])\n  \
         canvas::setGroup(\"outer\", [canvas::Group[dx := 0.0, dy := 200.0, name := \"inner\"]])\n  \
         canvas::present([canvas::Group[dx := 500.0, dy := 100.0, name := \"outer\"]])\n"
    ));
    assert_eq!(
        blocks, "1",
        "the outer group has no items of its own — only the inner group's one block \
         should be laid out: {draws}",
    );
    assert!(
        draws.contains(&format!("{}:{}", 500 * 65536, 300 * 65536)),
        "the inner run must be drawn at the composed offset (500,100)+(0,200) = \
         (500,300) = {}:{} in 16.16; got {draws}",
        500 * 65536,
        300 * 65536,
    );

    // 4. The diamond: one group, two references. THE case.
    let (blocks, draws) = d(&format!(
        "{RED}canvas::setGroup(\"g\", [red, red])\n  \
         canvas::present([canvas::Group[dx := 100.0, dy := 100.0, name := \"g\"], canvas::Group[dx := 300.0, dy := 100.0, name := \"g\"]])\n"
    ));
    assert_eq!(
        blocks, "2",
        "a diamond must write the shared group's blocks ONCE. {blocks} blocks means they \
         were duplicated per reference, which is the decision §4.3 asked Phase 1 to make \
         and this test to enforce: draws={draws}",
    );
    let bases: Vec<&str> = draws
        .split('|')
        .map(|e| e.split(':').next().unwrap_or(""))
        .collect();
    assert_eq!(
        bases,
        vec!["0", "0"],
        "the diamond's two draws must name the SAME item base — that is the \
         buffer-sharing property the per-draw offset exists to make possible: {draws}",
    );
    assert_ne!(
        draws.split('|').next(),
        draws.split('|').nth(1),
        "the two draws are identical, so the offsets did not differ: {draws}",
    );

    // 5. A group whose items have DIFFERENT blend modes must split into two draws
    //    (H8). §4.1's rule -- a run ends at a group node or a Text item -- misses this,
    //    and the four cases above all use one blend mode, which is why they do not
    //    catch it. Each BlendMode is a separate pipeline and a pipeline is bound per
    //    draw, so one draw spanning both would render whichever was bound: a wrong
    //    colour, not a missing shape.
    let (blocks, draws) = d(&format!(
        "{RED}LET mul AS canvas::DrawItem = canvas::Circle[x := 20.0, y := 20.0, radius := 10.0, paint := WITH canvas::fill(canvas::rgb(0, 255, 0)) {{ blend := canvas::BlendMode.Multiply }}]
           canvas::setGroup(\"g\", [red, mul])
           canvas::present([canvas::Group[dx := 100.0, dy := 100.0, name := \"g\"]])
"
    ));
    assert_eq!(
        blocks, "2",
        "the group still lays out both blocks once: draws={draws}",
    );
    assert_eq!(
        draws.split('|').count(),
        2,
        "a group whose two items have different blend modes must become TWO draws, one \
         per pipeline. One draw spanning both renders the Multiply item with whichever \
         pipeline was bound -- a plausible wrong colour: {draws}",
    );
    assert!(
        draws.starts_with("0:1:") && draws.contains("|1:1:"),
        "the split must fall between the two blocks, at bases 0 and 1: {draws}",
    );
}
