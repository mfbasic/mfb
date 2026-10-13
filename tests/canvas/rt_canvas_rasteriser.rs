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

/// Build and run a program, returning its stdout — with no frame dump required.
///
/// `render` insists on a dump, which is right for every test that looks at pixels and
/// wrong for one whose program is *supposed* to fail before presenting anything. Those
/// tests assert on what the program printed from its `TRAP` handler, so the frame is
/// not merely unnecessary, it is the thing that must not exist.
fn render_stdout(name: &str, source: &str) -> (String, String) {
    let project = common::temp_project(name, source);
    let binary = common::build_app_debug(&project, name);
    let run = Command::new(&binary)
        .env("MFB_HEADLESS", "1")
        // The GTK one too. Omitting it is invisible on the macOS dev host, where the
        // MACAPP flag is the one that matters; on a Linux box the program tries to open
        // a display, fails, and exits 1 -- which these tests then report as a
        // use-after-free or a missing raise. `render` above has always set all three.
        .env("MFB_HEADLESS", "1")
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
        "IMPORT app\nIMPORT canvas\nIMPORT vector\nIMPORT color\nIMPORT collections\nIMPORT io\nIMPORT math\n\nFUNC main() AS Nothing\n  \
         app::setMode(app::Mode.Canvas)\n{body}  io::print(\"rendered\")\nEND FUNC\n"
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

/// plan-163-B: `Paint.join` at a corner, in the software renderer — the oracle, so every
/// expectation is a hand-computed fact about a white 20 px stroke (`half` = 10) over
/// black, at pixel centres at least half a pixel inside or outside each shape.
///
/// * **A 90° polygon corner.** Each square's top-left vertex V has the bands' outer
///   corners A = V − (10, 0) and B = V − (0, 10):
///   - `Round` (today's join) is the disc of radius 10 about V: the centre (−6.5, −6.5)
///     from V is 9.19 away, covered; (−8.5, −8.5) is 12.02 away, not;
///   - `Miter` adds the square V, A, (V − (10, 10)), B: (−8.5, −8.5) and (−9.5, −9.5)
///     are inside it, (−10.5, −10.5) is not;
///   - `Bevel` adds the triangle V, A, B, the half-plane `x + y ≥ −10` from V:
///     (−4.5, −4.5) is inside by 0.71, (−5.5, −5.5) outside by 0.71 and (−6.5, −6.5)
///     outside by 2.12 — where the round join covers.
/// * **A 20° spike.** The apex V = (x, 300) of a triangle whose sides leave V at ±10°
///   from straight down: a miter would reach `10 / sin(10°) = 57.6` px above V, a ratio
///   of 5.76. Under `miterLimit := 4.0` that bevels — (x, 290) is 7.8 px above the
///   bevel's top edge, empty — and under `10.0` it miters — (x, 290) is inside the
///   spike, where it is 8.5 px wide each side, and so is (x, 250).
/// * **A PolyLine's interior corner** (100, 500) → (200, 500) → (200, 600), Butt ends:
///   V = (200, 500), A = (200, 490), B = (210, 500): the round join covers the centre
///   (206.5, 494.5) (8.5 from V) but not (209.5, 491.5) (12.75 from V); the bevel
///   covers (204.5, 496.5) and not (206.5, 494.5); the miter covers (209.5, 491.5). The
///   path's two ends are Butt and take no join: (99, 500) is empty under every join.
/// * **Rectangles**: a mitered one's corner is square — its top-left (−8.5, −8.5) is
///   covered — and a beveled one's is cut, as the polygon's.
#[test]
fn a_join_shapes_every_corner_of_a_stroke() {
    let corners = |x: f64, y: f64| -> String {
        format!(
            "[[vector::Float2[x := {x:?}, y := {y:?}], vector::Float2[x := {:?}, y := {y:?}], \
             vector::Float2[x := {:?}, y := {:?}], vector::Float2[x := {x:?}, y := {:?}]]]",
            x + 120.0,
            x + 120.0,
            y + 120.0,
            y + 120.0
        )
    };
    let spike = |x: f64| -> String {
        format!(
            "[[vector::Float2[x := {x:?}, y := 300.0], vector::Float2[x := {:?}, y := 447.72116], \
             vector::Float2[x := {:?}, y := 447.72116]]]",
            x + 26.04723,
            x - 26.04723
        )
    };
    let bend = |x: f64| -> String {
        format!(
            "[vector::Float2[x := {:?}, y := 500.0], vector::Float2[x := {:?}, y := 500.0], \
             vector::Float2[x := {:?}, y := 600.0]]",
            x,
            x + 100.0,
            x + 100.0
        )
    };
    let body = format!(
        "  LET ink AS canvas::Paint = canvas::stroke(color::rgb(255, 255, 255), 20.0)\n  \
         LET miter AS canvas::Paint = WITH ink {{ join := canvas::JoinStyle.Miter }}\n  \
         LET bevel AS canvas::Paint = WITH ink {{ join := canvas::JoinStyle.Bevel }}\n  \
         LET r1 AS canvas::DrawItem = canvas::Polygon[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], contours := {}, rule := canvas::FillRule.EvenOdd, paint := ink, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
         LET r2 AS canvas::DrawItem = canvas::Polygon[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], contours := {}, rule := canvas::FillRule.EvenOdd, paint := miter, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
         LET r3 AS canvas::DrawItem = canvas::Polygon[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], contours := {}, rule := canvas::FillRule.EvenOdd, paint := bevel, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
         LET s4 AS canvas::DrawItem = canvas::Polygon[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], contours := {}, rule := canvas::FillRule.EvenOdd, paint := miter, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
         LET s10 AS canvas::DrawItem = canvas::Polygon[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], contours := {}, rule := canvas::FillRule.EvenOdd, paint := WITH miter {{ miterLimit := 10.0 }}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
         LET b1 AS canvas::DrawItem = canvas::PolyLine[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], points := {}, colors := [], paint := ink, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
         LET b2 AS canvas::DrawItem = canvas::PolyLine[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], points := {}, colors := [], paint := miter, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
         LET b3 AS canvas::DrawItem = canvas::PolyLine[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], points := {}, colors := [], paint := bevel, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
         LET q1 AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 700.0, y := 100.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 120.0, height := 120.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := miter, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
         LET q2 AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 700.0, y := 300.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 120.0, height := 120.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := bevel, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
         canvas::present([r1, r2, r3, s4, s10, b1, b2, b3, q1, q2])\n",
        corners(100.0, 100.0),
        corners(300.0, 100.0),
        corners(500.0, 100.0),
        spike(400.0),
        spike(600.0),
        bend(50.0),
        bend(300.0),
        bend(550.0),
    );
    let (frame, _) = render("canvas_joins", &scene(&body));
    let white = (255, 255, 255, 255);
    let black = (0, 0, 0, 255);
    // Round, V = (100, 100).
    assert_eq!(
        pixel(&frame, 93, 93),
        white,
        "a round join covers 9.19 from its vertex"
    );
    assert_eq!(pixel(&frame, 91, 91), black, "but not 12.02");
    // Miter, V = (300, 100).
    assert_eq!(
        pixel(&frame, 291, 91),
        white,
        "a miter fills the square corner"
    );
    assert_eq!(pixel(&frame, 290, 90), white, "to its point");
    assert_eq!(pixel(&frame, 289, 89), black, "and no further");
    // Bevel, V = (500, 100).
    assert_eq!(
        pixel(&frame, 495, 95),
        white,
        "a bevel covers inside its cut"
    );
    assert_eq!(pixel(&frame, 494, 94), black, "and stops at it");
    assert_eq!(
        pixel(&frame, 493, 93),
        black,
        "where a round join would cover"
    );
    // The 20° spike: limit 4 bevels, limit 10 miters.
    assert_eq!(
        pixel(&frame, 400, 290),
        black,
        "miterLimit 4 bevels a 20° spike"
    );
    assert_eq!(
        pixel(&frame, 400, 250),
        black,
        "so nothing reaches up the spike"
    );
    assert_eq!(pixel(&frame, 600, 290), white, "miterLimit 10 miters it");
    assert_eq!(pixel(&frame, 600, 250), white, "up to its point");
    // PolyLine corners, V = (x + 100, 500) for x = 50, 300, 550.
    assert_eq!(pixel(&frame, 156, 494), white, "a round polyline turn");
    assert_eq!(pixel(&frame, 159, 491), black, "is round");
    assert_eq!(
        pixel(&frame, 409, 491),
        white,
        "a mitered polyline turn is square"
    );
    assert_eq!(
        pixel(&frame, 654, 496),
        white,
        "a beveled polyline turn covers inside its cut"
    );
    assert_eq!(pixel(&frame, 656, 494), black, "and not past it");
    for x in [49usize, 299, 549] {
        assert_eq!(
            pixel(&frame, x, 500),
            black,
            "a Butt end at x = {x} takes no join"
        );
    }
    // Rectangles.
    assert_eq!(
        pixel(&frame, 691, 91),
        white,
        "a mitered rectangle's corner is square"
    );
    assert_eq!(pixel(&frame, 689, 89), black, "and ends at its point");
    assert_eq!(
        pixel(&frame, 695, 295),
        white,
        "a beveled rectangle covers inside its cut"
    );
    assert_eq!(pixel(&frame, 693, 293), black, "and not past it");
}

/// plan-163-B: a dash that crosses a corner stays one dash round it — the premise the
/// dash design rests on (§8), for each cap.
///
/// Three 120 px squares (first point the top-left, so `s = 0` there, going clockwise)
/// stroked 20 px wide (`half` = 10) with `dashes := [150.0, 30.0]`: the perimeter is
/// 480, so the dashes are `s` ∈ [0, 150) — the top side and 30 px down the right side —
/// [180, 330) — the right side's last 60 px and 90 px of the bottom side, from its right
/// end — and [360, 510), which starts EXACTLY at the bottom-left corner and runs up the
/// left side to the seam, where it meets the first dash: on at both sides of the seam,
/// the two are one dash through the top-left corner. With the square at X:
///
/// * the top-right corner (X + 120, 100) is inside the first dash, so it takes its
///   (Round) join: the centre (X + 125.5, 95.5) is 7.1 from it, covered; the inner corner
///   (X + 112.5, 108.5) is inside both sides' bands;
/// * the right side's gap is y ∈ [130, 160): its centre row 145.5 is empty under every
///   cap — 15.5 and 14.5 from the two dash ends, beyond a Round disc and a Square's 10;
/// * at the dash end y = 130: row 129.5 (`along` = −0.5) is covered under every cap; row
///   131.5 only under Round (1.58 from the end point (X + 120, 130)) and Square (1.5 past
///   the end, within 10); row 137.5 under Round (7.5) and Square; row 139.5 under Square
///   alone (9.5 past, coverage exactly 255) and row 141.5 under none;
/// * the bottom-left corner (X, 220) is where the third dash STARTS, so it takes no join:
///   the centre (X − 4.5, 225.5) — 5.5 below the corner, beside the left side's line — is
///   empty under Butt, inside the Round end's disc (7.1 from the corner), and inside the
///   Square end, which runs 10 straight down the left side's line from the corner;
/// * the bottom side's gap is x ∈ (X, X + 30]: x = X + 15.5 is empty under every cap,
///   and X + 25.5 — 4.5 past the second dash's end at X + 30 — is covered by its Round
///   disc and its Square extension, not by Butt;
/// * the seam corner (X, 100) is inside the merged dash, so it takes its join:
///   (X − 4.5, 95.5) is 6.4 from it.
#[test]
fn a_dash_crossing_a_corner_stays_one_dash() {
    let (frame, _) = render(
        "canvas_dash_corners",
        &(scene(
            "  LET ink AS canvas::Paint = WITH canvas::stroke(color::rgb(255, 255, 255), 20.0) { dashes := [150.0, 30.0] }\n  \
             LET butt AS canvas::DrawItem = canvas::Polygon[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], contours := [sq(100.0, 100.0, 120.0)], rule := canvas::FillRule.EvenOdd, paint := ink, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET round AS canvas::DrawItem = canvas::Polygon[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], contours := [sq(300.0, 100.0, 120.0)], rule := canvas::FillRule.EvenOdd, paint := WITH ink { cap := canvas::CapStyle.Round }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET square AS canvas::DrawItem = canvas::Polygon[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], contours := [sq(500.0, 100.0, 120.0)], rule := canvas::FillRule.EvenOdd, paint := WITH ink { cap := canvas::CapStyle.Square }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([butt, round, square])\n",
        ) + RING_HELPERS),
    );
    let white = (255, 255, 255, 255);
    let black = (0, 0, 0, 255);
    for (x, cap) in [(100usize, "Butt"), (300, "Round"), (500, "Square")] {
        let on = |px: usize, py: usize, what: &str| {
            assert_eq!(pixel(&frame, px, py), white, "{cap}: {what}");
        };
        let off = |px: usize, py: usize, what: &str| {
            assert_eq!(pixel(&frame, px, py), black, "{cap}: {what}");
        };
        on(
            x + 125,
            95,
            "the dash crossing the top-right corner takes its join",
        );
        on(x + 112, 108, "and the corner's inner side");
        off(x + 120, 145, "the right side's gap");
        on(x + 120, 129, "the dash runs to its end");
        on(x - 5, 95, "the seam corner joins the dash through it");
        off(x + 15, 220, "the bottom side's gap");
        if cap == "Butt" {
            off(x + 120, 131, "a butt dash end stops at the end");
            off(x + 120, 137, "a butt dash end does not reach 7.5 past");
            off(x - 5, 225, "a dash starting at a corner takes no join");
            off(x + 25, 220, "a butt end leaves the gap");
        }
        if cap == "Round" {
            on(x + 120, 131, "a round dash end's disc");
            on(x + 120, 137, "reaches 7.5 past the end");
            off(x + 120, 141, "not 11.5");
            on(
                x - 5,
                225,
                "the dash starting at the corner takes its round cap",
            );
            on(x + 25, 220, "the round end's disc reaches 4.5 into the gap");
        }
        if cap == "Square" {
            on(x + 120, 131, "a square dash end");
            on(x + 120, 139, "reaches 9.5 past the end");
            off(x + 120, 141, "not 11.5");
            on(
                x - 5,
                225,
                "the dash starting at the corner runs its square end straight on",
            );
            on(x + 25, 220, "the square end reaches 4.5 into the gap");
        }
    }
}

/// The arc length of an ellipse from parametric angle 0 to `t`, by Simpson's rule over
/// 200,000 steps — far finer than anything the renderer uses, so it serves as the exact
/// value the dash table is measured against.
fn ellipse_arc_length(rx: f64, ry: f64, t: f64) -> f64 {
    let n = 200_000;
    let h = t / n as f64;
    let f = |a: f64| (rx * rx * a.sin() * a.sin() + ry * ry * a.cos() * a.cos()).sqrt();
    let mut sum = f(0.0) + f(t);
    for i in 1..n {
        let w = if i % 2 == 1 { 4.0 } else { 2.0 };
        sum += w * f(i as f64 * h);
    }
    sum * h / 3.0
}

/// The parametric angle at which an ellipse's arc length from angle 0 reaches `s`.
fn ellipse_angle_at(rx: f64, ry: f64, s: f64) -> f64 {
    let (mut lo, mut hi) = (0.0f64, std::f64::consts::TAU);
    for _ in 0..50 {
        let mid = (lo + hi) / 2.0;
        if ellipse_arc_length(rx, ry, mid) < s {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (lo + hi) / 2.0
}

/// plan-163-B: a dashed Ellipse is cut by arc length, from angle 0 (+X) the way angles
/// increase (towards +Y), measured against an independent arc length
/// (`ellipse_arc_length`, not the renderer's table).
///
/// rx = 160, ry = 80, 8 px wide, `dashes := [30.0, 15.0]`: every dash's middle (`s` =
/// 15 + 45k) is drawn and every gap's middle (`s` = 37.5 + 45k) is not. The pixel tested
/// is the one holding the outline point, whose centre is within 0.71 of it: well inside
/// the 4 px half-width, and 7.5 or more along the outline from the nearest dash end.
#[test]
fn a_dashed_ellipse_is_cut_by_arc_length() {
    let (frame, _) = render(
        "canvas_dash_ellipse",
        &scene(
            "  LET e AS canvas::DrawItem = canvas::Ellipse[id := \"\", position := vector::Float2[x := 450.0, y := 320.0], radiusX := 160.0, radiusY := 80.0, paint := WITH canvas::stroke(color::rgb(255, 255, 255), 8.0) { dashes := [30.0, 15.0] }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([e])\n",
        ),
    );
    let (rx, ry) = (160.0, 80.0);
    let perimeter = ellipse_arc_length(rx, ry, std::f64::consts::TAU);
    let at = |s: f64| {
        let t = ellipse_angle_at(rx, ry, s);
        let x = 450.0 + rx * t.cos();
        let y = 320.0 + ry * t.sin();
        pixel(&frame, x.floor() as usize, y.floor() as usize)
    };
    let mut k = 0;
    while 37.5 + 45.0 * k as f64 + 7.5 < perimeter {
        let s = 15.0 + 45.0 * k as f64;
        assert_eq!(
            at(s),
            (255, 255, 255, 255),
            "the middle of dash {k}, s = {s}"
        );
        let g = 37.5 + 45.0 * k as f64;
        assert_eq!(at(g), (0, 0, 0, 255), "the middle of gap {k}, s = {g}");
        k += 1;
    }
    assert!(
        k >= 16,
        "the ellipse's perimeter {perimeter} holds 16 periods"
    );
}

/// plan-163-B: the ellipse dash table's resolution, `__CANVAS_DASH_ELLIPSE_STEPS`,
/// chosen by measurement (§8 left it UNVERIFIED).
///
/// The table is the cumulative arc length at N + 1 evenly spaced parametric angles, each
/// step Simpson's rule over 8 sub-steps, and a pixel's position is the linear
/// interpolation between two entries. This reproduces that construction exactly (in f64)
/// and measures its worst error against `ellipse_arc_length` at 16 points per step,
/// for ellipses up to 800 px across and 80:1 eccentric.
#[test]
fn an_ellipse_dash_table_is_within_a_tenth_of_a_pixel() {
    const STEPS: usize = 256;
    let mut worst: f64 = 0.0;
    for (rx, ry) in [
        (400.0f64, 5.0f64),
        (400.0, 200.0),
        (200.0, 10.0),
        (50.0, 40.0),
        (8.0, 400.0),
    ] {
        let dt = std::f64::consts::TAU / STEPS as f64;
        let f = |a: f64| (rx * rx * a.sin() * a.sin() + ry * ry * a.cos() * a.cos()).sqrt();
        let mut table = vec![0.0f64];
        for k in 0..STEPS {
            let t0 = k as f64 * dt;
            let mut sum = 0.0;
            for j in 0..=8 {
                let w = if j == 0 || j == 8 {
                    1.0
                } else if j % 2 == 1 {
                    4.0
                } else {
                    2.0
                };
                sum += w * f(t0 + j as f64 * dt / 8.0);
            }
            table.push(table[k] + sum * dt / 24.0);
        }
        // Exact arc length at the probe points, by integrating step to step.
        for k in 0..STEPS {
            for i in 1..16 {
                let t = (k as f64 + i as f64 / 16.0) * dt;
                let frac = i as f64 / 16.0;
                let interpolated = table[k] + (table[k + 1] - table[k]) * frac;
                let exact = ellipse_arc_length(rx, ry, t);
                worst = worst.max((interpolated - exact).abs());
            }
        }
    }
    println!("ellipse dash table, {STEPS} steps: worst along-path error {worst:.5} px");
    assert!(worst < 0.1, "the worst along-path error is {worst} px");
}

/// plan-163-B: `Paint.dashes` on a Line, in the software renderer — the pattern, its
/// normalization (§4) and `dashOffset`.
///
/// Each line runs from x = 100 to x = 300 along a row centre (y = r + 0.5), 4 px wide,
/// Butt: a pixel at column 100 + i is drawn exactly when `s` = i + 0.5 is in a dash —
/// every dash boundary here is a whole number, so every centre is half a pixel from one.
///
/// * `[10, 5]`: on for `s` mod 15 ∈ [0, 10);
/// * `[10, 5, 5]` is odd, so it repeats once to `[10, 5, 5, 10, 5, 5]`: on for `s` mod 40
///   ∈ [0, 10) ∪ [15, 20) ∪ [30, 35);
/// * `[]`, `[0, 0]` (sum 0) and `[10, -5]` (a negative entry) are solid;
/// * `dashOffset := 3` starts 3 into `[10, 5]`: on for (`s` + 3) mod 15 ∈ [0, 10);
///   `dashOffset := -3` is 12 into it.
#[test]
fn a_dashed_line_draws_its_pattern() {
    let rows: [(&str, usize); 7] = [
        ("[10.0, 5.0]", 40),
        ("[10.0, 5.0, 5.0]", 60),
        ("[]", 80),
        ("[0.0, 0.0]", 100),
        ("[10.0, 0.0 - 5.0]", 120),
        ("[10.0, 5.0]", 140),
        ("[10.0, 5.0]", 160),
    ];
    let offsets = ["0.0", "0.0", "0.0", "0.0", "0.0", "3.0", "0.0 - 3.0"];
    let mut body = String::from(
        "  LET ink AS canvas::Paint = canvas::stroke(color::rgb(255, 255, 255), 4.0)\n",
    );
    let mut names = Vec::new();
    for (i, ((dashes, y), offset)) in rows.iter().zip(offsets).enumerate() {
        body.push_str(&format!(
            "  LET l{i} AS canvas::DrawItem = canvas::Line[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], startAt := vector::Float2[x := 100.0, y := {y}.5], endAt := vector::Float2[x := 300.0, y := {y}.5], paint := WITH ink {{ dashes := {dashes}, dashOffset := {offset} }}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n"
        ));
        names.push(format!("l{i}"));
    }
    body.push_str(&format!("  canvas::present([{}])\n", names.join(", ")));
    let (frame, _) = render("canvas_dash_line", &scene(&body));
    let white = (255, 255, 255, 255);
    let black = (0, 0, 0, 255);
    let expect = |y: usize, what: &str, on: &dyn Fn(f64) -> bool| {
        for i in 0..200usize {
            let s = i as f64 + 0.5;
            let want = if on(s) { white } else { black };
            assert_eq!(
                pixel(&frame, 100 + i, y),
                want,
                "{what}: column {}, s = {s}",
                100 + i
            );
        }
    };
    expect(40, "[10, 5]", &|s| s % 15.0 < 10.0);
    expect(60, "[10, 5, 5] repeated once", &|s| {
        let w = s % 40.0;
        w < 10.0 || (15.0..20.0).contains(&w) || (30.0..35.0).contains(&w)
    });
    expect(80, "an empty list is solid", &|_| true);
    expect(100, "a zero-sum list is solid", &|_| true);
    expect(120, "a negative entry is solid", &|_| true);
    expect(140, "dashOffset 3", &|s| (s + 3.0) % 15.0 < 10.0);
    expect(160, "dashOffset -3", &|s| (s + 12.0) % 15.0 < 10.0);
}

/// plan-163-B: a dash end takes the paint's cap, and so do a dashed path's own ends.
///
/// Lines from x = 100 to 300 at y = 50, 100, 150, 8 px wide (`half` = 4), `[10.0, 10.0]`:
/// the first dash is [100, 110). The centre (113.5, 46.5) is 3.5 past that dash's end and
/// 3.5 off the line: within a Square end (both under 4) but 4.95 from the end point,
/// outside a Round one; Butt draws neither. (111.5, 50.5) is 1.5 past the end — inside
/// Round and Square. The path's start x = 100 is a dash end too: (96.5, 150.5) is 3.5
/// before it, inside a Square end, and (97.5, 100.5) is 2.55 from it, inside a Round one;
/// Butt draws neither.
#[test]
fn a_dash_end_takes_the_cap() {
    let (frame, _) = render(
        "canvas_dash_caps",
        &scene(
            "  LET ink AS canvas::Paint = WITH canvas::stroke(color::rgb(255, 255, 255), 8.0) { dashes := [10.0, 10.0] }\n  \
             LET butt AS canvas::DrawItem = canvas::Line[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], startAt := vector::Float2[x := 100.0, y := 50.0], endAt := vector::Float2[x := 300.0, y := 50.0], paint := ink, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET round AS canvas::DrawItem = canvas::Line[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], startAt := vector::Float2[x := 100.0, y := 100.0], endAt := vector::Float2[x := 300.0, y := 100.0], paint := WITH ink { cap := canvas::CapStyle.Round }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET square AS canvas::DrawItem = canvas::Line[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], startAt := vector::Float2[x := 100.0, y := 150.0], endAt := vector::Float2[x := 300.0, y := 150.0], paint := WITH ink { cap := canvas::CapStyle.Square }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([butt, round, square])\n",
        ),
    );
    let white = (255, 255, 255, 255);
    let black = (0, 0, 0, 255);
    assert_eq!(
        pixel(&frame, 111, 50),
        black,
        "a butt dash stops at its end"
    );
    assert_eq!(pixel(&frame, 113, 46), black, "and has no corner past it");
    assert_eq!(pixel(&frame, 96, 50), black, "nor before the path's start");
    assert_eq!(
        pixel(&frame, 111, 100),
        white,
        "a round dash end reaches past it"
    );
    assert_eq!(pixel(&frame, 113, 96), black, "as a disc");
    assert_eq!(
        pixel(&frame, 97, 100),
        white,
        "and the path's start is capped round"
    );
    assert_eq!(
        pixel(&frame, 111, 150),
        white,
        "a square dash end reaches past it"
    );
    assert_eq!(pixel(&frame, 113, 146), white, "with a square corner");
    assert_eq!(
        pixel(&frame, 96, 150),
        white,
        "and the path's start is capped square"
    );
}

/// plan-163-B: each Polygon ring restarts the pattern at its own first point, and a
/// dashed Circle's `s = 0` is its +X point, running clockwise (towards +Y).
///
/// * Two square rings, sides 27.5 (perimeter 110) at (100, 300) and (200, 300), 4 px
///   wide, `[10.0, 10.0]`: each ring's top side is on for x ∈ [first.x, first.x + 10).
///   Had the second ring continued the first's pattern, it would start at `s` = 110,
///   mid-gap. The centre (205.5, 300.5) is 5.5 along the second ring.
/// * A Circle at (600, 300), radius 50, 6 px wide, `[20.0, 300.0]` (one dash per
///   perimeter of 314.16): the dash covers angles [0, 0.4) clockwise from +X. The centre
///   (648.5, 309.5) is at angle 0.194 (`s` = 9.7) and radius 49.42; (648.5, 290.5) is at
///   angle −0.194, `s` = 304.4, mid-gap.
#[test]
fn each_ring_restarts_the_pattern_and_a_circle_starts_at_plus_x() {
    let (frame, _) = render(
        "canvas_dash_rings",
        &format!(
            "{}{RING_HELPERS}",
            scene(
                "  LET ink AS canvas::Paint = WITH canvas::stroke(color::rgb(255, 255, 255), 4.0) { dashes := [10.0, 10.0] }\n  \
                 LET rings AS canvas::DrawItem = canvas::Polygon[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], contours := [sq(100.0, 300.0, 27.5), sq(200.0, 300.0, 27.5)], rule := canvas::FillRule.EvenOdd, paint := ink, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
                 LET disc AS canvas::DrawItem = canvas::Circle[id := \"\", position := vector::Float2[x := 600.0, y := 300.0], radius := 50.0, paint := WITH canvas::stroke(color::rgb(255, 255, 255), 6.0) { dashes := [20.0, 300.0] }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
                 canvas::present([rings, disc])\n",
            )
        ),
    );
    let white = (255, 255, 255, 255);
    let black = (0, 0, 0, 255);
    assert_eq!(
        pixel(&frame, 105, 300),
        white,
        "the first ring starts with a dash"
    );
    assert_eq!(pixel(&frame, 115, 300), black, "then a gap");
    assert_eq!(
        pixel(&frame, 205, 300),
        white,
        "the second ring restarts with a dash"
    );
    assert_eq!(pixel(&frame, 215, 300), black, "then a gap");
    assert_eq!(
        pixel(&frame, 648, 309),
        white,
        "the circle's dash starts at +X, going clockwise"
    );
    assert_eq!(pixel(&frame, 648, 290), black, "not counter-clockwise");
    assert_eq!(pixel(&frame, 550, 300), black, "the far side is in the gap");
}

/// plan-163-B: a dashed Rectangle-with-radius's `s = 0` is the left end of its top straight side,
/// running clockwise, and a dashed Arc's is `startAngle`, running the way the sweep does.
///
/// * The Rectangle with radius at (100, 400), 200 x 100, corner radius 20, 4 px wide, `[30.0,
///   400.0]`: its top side runs x ∈ [120, 280) at y = 400, so the dash covers x ∈ [120,
///   150). The centre (125.5, 400.5) is at `s` = 5.5; (155.5, 400.5) at `s` = 35.5, in
///   the gap; (115.5, 400.5) is in the top-left corner's arc, `s` ≈ 565 of a 565.66
///   perimeter, in the gap too — the dash starts at the corner's end, it does not run
///   back round it.
/// * The Arc at (600, 450), radius 50, from π/2 (straight down) to π, 4 px wide, `[20.0,
///   200.0]`, Butt: the dash covers the first 20 px of the sweep, towards −X. The centre
///   (590.5, 499.5) is at radius 50.40, angle 1.7602, `s` = 9.47; (553.5, 468.5) is at
///   `s` = 59.6, in the gap; (609.5, 499.5) is 9.47 BEFORE the start, past the butt end.
#[test]
fn a_rounded_rect_and_an_arc_start_their_dashes_where_the_contract_says() {
    let (frame, _) = render(
        "canvas_dash_starts",
        &scene(
            "  LET ink AS canvas::Paint = canvas::stroke(color::rgb(255, 255, 255), 4.0)\n  \
             LET box AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 100.0, y := 400.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 200.0, height := 100.0], radiusTL := 20.0, radiusTR := 20.0, radiusBL := 20.0, radiusBR := 20.0, paint := WITH ink { dashes := [30.0, 400.0] }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET bow AS canvas::DrawItem = canvas::Arc[id := \"\", position := vector::Float2[x := 600.0, y := 450.0], radius := 50.0, startAngle := 1.5707963267948966, endAngle := 3.141592653589793, paint := WITH ink { dashes := [20.0, 200.0] }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([box, bow])\n",
        ),
    );
    let white = (255, 255, 255, 255);
    let black = (0, 0, 0, 255);
    assert_eq!(
        pixel(&frame, 125, 400),
        white,
        "the rounded rectangle's dash starts on its top side"
    );
    assert_eq!(pixel(&frame, 155, 400), black, "and ends 30 along it");
    assert_eq!(
        pixel(&frame, 115, 400),
        black,
        "the corner before the start is the pattern's end, in the gap"
    );
    assert_eq!(
        pixel(&frame, 590, 499),
        white,
        "the arc's dash starts at startAngle"
    );
    assert_eq!(
        pixel(&frame, 553, 468),
        black,
        "and ends 20 along the sweep"
    );
    assert_eq!(
        pixel(&frame, 609, 499),
        black,
        "nothing is drawn before the start"
    );
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
            "  LET l AS canvas::DrawItem = canvas::Line[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], startAt := vector::Float2[x := 200.0, y := 300.0], endAt := vector::Float2[x := 400.0, y := 300.0], paint := WITH canvas::stroke(color::rgb(255, 255, 255), 20.0) {{ cap := canvas::CapStyle.{cap} }}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
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
            "  LET l AS canvas::DrawItem = canvas::Line[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], startAt := vector::Float2[x := 300.0, y := 300.0], endAt := vector::Float2[x := 300.0, y := 300.0], paint := WITH canvas::stroke(color::rgb(255, 255, 255), 20.0) {{ cap := canvas::CapStyle.{cap} }}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
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
            "  LET a AS canvas::DrawItem = canvas::Arc[id := \"\", position := vector::Float2[x := 300.0, y := 300.0], radius := 100.0, startAngle := 0.0, endAngle := 3.141592653589793, paint := WITH canvas::stroke(color::rgb(255, 255, 255), 24.0) {{ cap := canvas::CapStyle.{cap} }}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
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
            "  LET a AS canvas::DrawItem = canvas::Arc[id := \"\", position := vector::Float2[x := 300.0, y := 300.0], radius := 100.0, startAngle := 0.0, endAngle := 3.141592653589793, paint := WITH canvas::stroke(color::rgb(255, 255, 255), 24.0) { cap := canvas::CapStyle.Round }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
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
            "  LET a AS canvas::DrawItem = canvas::Arc[id := \"\", position := vector::Float2[x := 300.0, y := 300.0], radius := 100.0, startAngle := 0.0, endAngle := 6.283185307179586, paint := WITH canvas::stroke(color::rgb(255, 255, 255), 24.0) {{ cap := canvas::CapStyle.{cap} }}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
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
    let paint = "canvas::fillStroke(color::rgb(90, 200, 255), color::rgb(255, 255, 255), 9.0)";
    let (circle, _) = render(
        "canvas_ellipse_as_circle_ref",
        &scene(&format!(
            "  LET a AS canvas::DrawItem = canvas::Circle[id := \"\", position := vector::Float2[x := 400.0, y := 300.0], radius := 120.0, paint := {paint}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  canvas::present([a])\n"
        )),
    );
    let (ellipse, _) = render(
        "canvas_ellipse_as_circle",
        &scene(&format!(
            "  LET a AS canvas::DrawItem = canvas::Ellipse[id := \"\", position := vector::Float2[x := 400.0, y := 300.0], radiusX := 120.0, radiusY := 120.0, paint := {paint}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
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
            "  LET a AS canvas::DrawItem = canvas::Ellipse[id := \"\", position := vector::Float2[x := 400.0, y := 300.0], radiusX := 300.0, radiusY := 100.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
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
            "  LET a AS canvas::DrawItem = canvas::Ellipse[id := \"\", position := vector::Float2[x := 400.0, y := 300.0], radiusX := {rx:.1}, radiusY := {ry:.1}, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[a := math::cos({angle}), b := math::sin({angle}), c := 0.0 - math::sin({angle}), d := math::cos({angle}), tx := 0.0, ty := 0.0]]\n  \
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

/// plan-172-C: an ellipse has no `angle`; it is turned by its `transform`.
///
/// A 100 x 20 ellipse at (300, 300) turned half a radian about its own centre. Its long
/// axis now runs along (cos 0.5, sin 0.5) = (0.8776, 0.4794), so 92 px out along it is
/// (380.7, 344.1) on one side and (219.3, 255.9) on the other — both well inside the
/// ellipse. The control is the pixel 92 px out along +X, (392, 300): inside the ellipse
/// had it NOT turned, and 44 px off the turned axis against a short radius of 20.
#[test]
fn an_ellipse_is_turned_by_its_transform() {
    let (frame, _) = render(
        "canvas_ellipse_turned",
        &scene(
            "  LET co AS Float = 0.8775825618903728\n  \
             LET si AS Float = 0.479425538604203\n  \
             LET turn AS canvas::Transform = canvas::Transform[a := co, b := si, c := 0.0 - si, d := co, tx := 0.0, ty := 0.0]\n  \
             LET egg AS canvas::DrawItem = canvas::Ellipse[id := \"\", position := vector::Float2[x := 300.0, y := 300.0], radiusX := 100.0, radiusY := 20.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := turn]\n  \
             canvas::present([egg])\n",
        ),
    );
    assert_eq!(
        pixel(&frame, 380, 344),
        (255, 255, 255, 255),
        "92 px along the turned long axis, towards the bottom-right"
    );
    assert_eq!(
        pixel(&frame, 219, 255),
        (255, 255, 255, 255),
        "92 px along the turned long axis, towards the top-left"
    );
    assert_eq!(
        pixel(&frame, 300, 300),
        (255, 255, 255, 255),
        "the centre does not move: the turn is about it"
    );
    assert_eq!(
        pixel(&frame, 392, 300),
        (0, 0, 0, 255),
        "92 px along +X is inside the UNTURNED ellipse only; drawing here means the \
         transform did not turn it"
    );
}

// ---- plan-172-E: a rectangle's corners ---------------------------------------------
//
// The rule is in the package's own MFBASIC (`__canvas_cornerRadii`), which no unit test
// can call, so it is proved by what it draws. `corner_reach` is the rule written a
// second time, in Rust, straight from `planning/new_canvas.md`: the oracle the pixels
// are held to.

/// The horizontal and vertical radius a corner value `r` resolves to on a `w` x `h`
/// rectangle.
fn corner_reach(w: f64, h: f64, r: f64) -> (f64, f64) {
    if r <= 0.0 {
        (0.0, 0.0)
    } else if r <= 1.0 {
        (r * w / 2.0, r * h / 2.0)
    } else {
        let c = r.min(w / 2.0).min(h / 2.0);
        (c, c)
    }
}

/// A rectangle at (100, 100) of the given size whose TOP-LEFT corner has radius `r`
/// and whose other three are square.
fn top_left_rounded(w: f64, h: f64, r: &str) -> String {
    scene(&format!(
        "  LET box AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 100.0, y := 100.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := {w:?}, height := {h:?}], radiusTL := {r}, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
         canvas::present([box])\n"
    ))
}

/// The seven cases of the corner rule, each held to `corner_reach`.
///
/// For a resolved corner (rx, ry) the outline's corner is the ellipse quadrant centred
/// (100 + rx, 100 + ry). Two probes sit on the diagonal from that centre towards the
/// rectangle's corner: one 2 px short of the curve (inside, white) and one 2 px past it
/// (outside the shape but inside the rectangle's box, black). A square corner has no
/// curve; its probe is the corner pixel itself, which is white. Two pixels clears the
/// antialiased edge and the 0.05 px flattening error by a wide margin, so each probe is
/// a fact about which side of the corner it is on.
#[test]
fn corner_rule_every_meaning_of_a_radius_draws_the_corner_it_names() {
    let white = (255, 255, 255, 255);
    let black = (0, 0, 0, 255);
    for (name, w, h, r, text) in [
        ("zero", 100.0, 40.0, 0.0, "0.0"),
        ("negative", 100.0, 40.0, -1.0, "0.0 - 1.0"),
        ("half", 100.0, 40.0, 0.5, "0.5"),
        ("one", 100.0, 40.0, 1.0, "1.0"),
        ("just_past_one", 100.0, 40.0, 1.01, "1.01"),
        ("clamped", 100.0, 40.0, 500.0, "500.0"),
        ("pixels", 100.0, 40.0, 12.0, "12.0"),
    ] {
        let (rx, ry) = corner_reach(w, h, r);
        let (frame, _) = render(
            &format!("canvas_corner_{name}"),
            &top_left_rounded(w, h, text),
        );
        // The far corner is square in every case: the rectangle is really there.
        assert_eq!(
            pixel(&frame, 100 + w as usize - 1, 100 + h as usize - 1),
            white,
            "{name}: the bottom-right corner pixel"
        );
        if rx < 3.0 || ry < 3.0 {
            // No curve to speak of (a square corner, or the one-pixel corner of 1.01):
            // 3 px in along both sides is inside whatever corner there is.
            assert_eq!(
                pixel(&frame, 103, 103),
                white,
                "{name}: just inside the top-left corner"
            );
            if rx == 0.0 {
                assert_eq!(
                    pixel(&frame, 100, 100),
                    white,
                    "{name}: a square corner's own pixel"
                );
            }
            continue;
        }
        let (cx, cy) = (100.0 + rx, 100.0 + ry);
        // The point of the ellipse on the diagonal towards the corner, parametrically at
        // 225 degrees, and the outward normal there.
        let k = std::f64::consts::FRAC_1_SQRT_2;
        let (ex, ey) = (cx - rx * k, cy - ry * k);
        let (nx, ny) = (-ry * k, -rx * k);
        let len = (nx * nx + ny * ny).sqrt();
        let (nx, ny) = (nx / len, ny / len);
        let inside = ((ex - 2.0 * nx) as usize, (ey - 2.0 * ny) as usize);
        let outside = ((ex + 2.0 * nx) as usize, (ey + 2.0 * ny) as usize);
        assert_eq!(
            pixel(&frame, inside.0, inside.1),
            white,
            "{name} (r = {r}, corner {rx} x {ry}): 2 px inside the curve at {inside:?}"
        );
        assert_eq!(
            pixel(&frame, outside.0, outside.1),
            black,
            "{name} (r = {r}, corner {rx} x {ry}): 2 px outside the curve at {outside:?}"
        );
    }
}

/// A 1 x 1 rectangle with a fractional radius: both radii are half a pixel. It must
/// draw its pixel and nothing else, and must not fail — the degenerate end of the rule.
#[test]
fn corner_rule_a_one_pixel_rectangle_is_still_one_pixel() {
    let (frame, _) = render("canvas_corner_unit", &top_left_rounded(1.0, 1.0, "1.0"));
    let lit = frame.chunks_exact(4).filter(|p| p[0] != 0).count();
    assert!(lit <= 1, "a 1 x 1 rectangle lit {lit} pixels");
    assert_eq!(
        pixel(&frame, 102, 100),
        (0, 0, 0, 255),
        "right of it is background"
    );
}

/// A `radiusTL`-only rectangle has one rounded corner and three square ones.
#[test]
fn corner_rule_only_the_named_corner_is_rounded() {
    let (frame, _) = render(
        "canvas_corner_only_tl",
        &top_left_rounded(200.0, 120.0, "40.0"),
    );
    let white = (255, 255, 255, 255);
    let black = (0, 0, 0, 255);
    assert_eq!(
        pixel(&frame, 102, 102),
        black,
        "top-left: cut away by the 40 px corner"
    );
    assert_eq!(pixel(&frame, 297, 102), white, "top-right: square");
    assert_eq!(pixel(&frame, 102, 217), white, "bottom-left: square");
    assert_eq!(pixel(&frame, 297, 217), white, "bottom-right: square");
    assert_eq!(
        pixel(&frame, 130, 130),
        white,
        "inside the rounded corner's curve"
    );
}

/// `BindTo.Center` puts the rectangle's CENTRE at `position`; `TopLeft` puts its corner
/// there. The same 100 x 60 rectangle at (300, 300) both ways.
#[test]
fn corner_rule_bind_names_the_point_position_is() {
    let body = |bind: &str| {
        scene(&format!(
            "  LET box AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 300.0, y := 300.0], bind := canvas::BindTo.{bind}, size := canvas::Size[width := 100.0, height := 60.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([box])\n"
        ))
    };
    let white = (255, 255, 255, 255);
    let black = (0, 0, 0, 255);
    let (centre, _) = render("canvas_bind_center", &body("Center"));
    assert_eq!(
        pixel(&centre, 300, 300),
        white,
        "Center: the centre pixel is at position"
    );
    assert_eq!(
        pixel(&centre, 251, 271),
        white,
        "Center: 49 left and 29 up is inside"
    );
    assert_eq!(
        pixel(&centre, 348, 328),
        white,
        "Center: 48 right and 28 down is inside"
    );
    assert_eq!(
        pixel(&centre, 352, 300),
        black,
        "Center: 52 right is outside a 100-wide box"
    );
    assert_eq!(
        pixel(&centre, 300, 332),
        black,
        "Center: 32 down is outside a 60-high box"
    );

    let (corner, _) = render("canvas_bind_top_left", &body("TopLeft"));
    assert_eq!(
        pixel(&corner, 300, 300),
        white,
        "TopLeft: position is the corner pixel"
    );
    assert_eq!(
        pixel(&corner, 398, 358),
        white,
        "TopLeft: the box runs right and down"
    );
    assert_eq!(
        pixel(&corner, 298, 298),
        black,
        "TopLeft: nothing above or left of position"
    );
}

/// A `Center`-bound rectangle turns in place: a quarter turn of a 200 x 40 bar centred
/// (300, 300) is a 40 x 200 bar centred (300, 300).
#[test]
fn corner_rule_a_centre_bound_rectangle_spins_in_place() {
    let (frame, _) = render(
        "canvas_bind_center_turn",
        &scene(
            "  LET quarter AS canvas::Transform = canvas::Transform[a := 0.0, b := 1.0, c := 0.0 - 1.0, d := 0.0, tx := 0.0, ty := 0.0]\n  \
             LET bar AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 300.0, y := 300.0], bind := canvas::BindTo.Center, size := canvas::Size[width := 200.0, height := 40.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := quarter]\n  \
             canvas::present([bar])\n",
        ),
    );
    let white = (255, 255, 255, 255);
    let black = (0, 0, 0, 255);
    assert_eq!(pixel(&frame, 300, 300), white, "the centre stays put");
    assert_eq!(
        pixel(&frame, 300, 210),
        white,
        "90 px above the centre: along the turned bar"
    );
    assert_eq!(
        pixel(&frame, 300, 390),
        white,
        "90 px below the centre: along the turned bar"
    );
    assert_eq!(
        pixel(&frame, 390, 300),
        black,
        "90 px right: where the UNTURNED bar was"
    );
    assert_eq!(
        pixel(&frame, 210, 300),
        black,
        "90 px left: where the UNTURNED bar was"
    );
}

/// The flattened outline against the exact one, measured.
///
/// Four equal 24 px corners are the exact rounded-rectangle record. Making one of them
/// 24.000001 px makes the four unequal, which sends the same shape — to a millionth of
/// a pixel — down the flattened path. The two frames are compared pixel by pixel.
///
/// The comparison is of COVERAGE, not of frame bytes. A frame byte is sRGB-encoded, and
/// near black the encoding is steep: a coverage of 0.034 is byte 53, so two frames that
/// agree to a thirtieth of a pixel differ by 53 of 255 there. Decoded, the difference is
/// what the design says it is — a chord stays within 0.05 px of its arc, so a pixel's
/// coverage moves by at most 0.05. Measured: 656 pixels differ, all on the four corners
/// (the straight sides agree to one rounding step), and the worst is 0.0499.
#[test]
fn corner_rule_the_flattened_outline_is_within_its_bound_of_the_exact_one() {
    let body = |last: &str| {
        scene(&format!(
            "  LET box AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 100.25, y := 100.5], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 300.0, height := 180.0], radiusTL := 24.0, radiusTR := 24.0, radiusBL := 24.0, radiusBR := {last}, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([box])\n"
        ))
    };
    // White on black, so a byte decoded to linear light IS the coverage.
    let coverage = |byte: u8| -> f64 {
        let c = f64::from(byte) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let (exact, _) = render("canvas_corner_exact", &body("24.0"));
    let (flat, _) = render("canvas_corner_flat", &body("24.000001"));
    let mut worst = 0.0f64;
    let mut differing = 0usize;
    for (a, b) in exact.chunks_exact(4).zip(flat.chunks_exact(4)) {
        if a[0] != b[0] {
            differing += 1;
            worst = worst.max((coverage(a[0]) - coverage(b[0])).abs());
        }
    }
    println!(
        "flattened vs exact rounded rectangle: {differing} pixels differ, worst coverage delta {worst:.4}"
    );
    assert!(
        exact.chunks_exact(4).filter(|p| p[0] == 255).count() > 40_000,
        "the rectangle drew, so the comparison is of something"
    );
    assert!(
        differing > 0,
        "the two frames are identical, so the second rectangle did not take the flattened path"
    );
    assert!(
        worst <= 0.055,
        "the flattened outline's coverage differs from the exact one's by {worst:.4} in some \
         pixel; a chord within 0.05 px of its arc allows 0.05"
    );
}

/// `canvas::Size` holds Floats, and `canvas::getSize()` returns the surface's whole
/// pixel dimensions in them.
#[test]
fn corner_rule_the_surface_size_is_whole_pixels_held_as_floats() {
    let source = "IMPORT app\nIMPORT canvas\nIMPORT io\n\nFUNC main() AS Nothing\n  \
                  app::setMode(app::Mode.Canvas)\n  \
                  LET size AS canvas::Size = canvas::getSize()\n  \
                  LET half AS Float = size.width / 2.0\n  \
                  io::print(\"size \" & toString(size.width) & \" x \" & toString(size.height) & \" half \" & toString(half) & \" ints \" & toString(toInt(size.width)) & \" \" & toString(toInt(size.height)))\n  \
                  canvas::present([])\n  io::print(\"rendered\")\nEND FUNC\n";
    let project = common::temp_project("canvas_size_floats", source);
    let binary = common::build_app_debug(&project, "canvas_size_floats");
    let run = Command::new(&binary)
        .env("MFB_HEADLESS", "1")
        .env("MFB_CANVAS_SYNC", "1")
        .output()
        .expect("run the size program");
    let out = String::from_utf8_lossy(&run.stdout).to_string();
    let _ = std::fs::remove_dir_all(&project);
    assert!(
        out.contains("size 900.00 x 640.00 half 450.00 ints 900 640"),
        "the surface is 900 x 640, as Floats:\n{out}"
    );
}

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
/// (`helper_geometry.rs`). Positions and the shift are whole numbers, so the frames are
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
                "  LET a AS canvas::DrawItem = canvas::Ellipse[id := \"\", position := vector::Float2[x := 400.0, y := 300.0], radiusX := {rx}, radiusY := {ry}, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
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
            "  LET a AS canvas::DrawItem = canvas::Ellipse[id := \"\", position := vector::Float2[x := 400.0, y := 300.0], radiusX := 200.0, radiusY := 100.0, paint := canvas::stroke(color::rgb(255, 255, 255), 20.0), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
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
            "  LET a AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 10.0, y := 10.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 50.0, height := 50.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::setGroup(\"panel\", [a])\n  \
             LET node AS canvas::DrawItem = canvas::Group[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"panel\"]\n  \
             canvas::present([node])\n  \
             LET b AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 700.0, y := 500.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 50.0, height := 50.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(0, 255, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
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
            "  LET a AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 10.0, y := 10.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 50.0, height := 50.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::setGroup(\"panel\", [a])\n  \
             LET node AS canvas::DrawItem = canvas::Group[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"panel\"]\n  \
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
            "  LET red AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 10.0, y := 10.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 50.0, height := 50.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET green AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 80.0, y := 10.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 50.0, height := 50.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(0, 255, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET blue AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 150.0, y := 10.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 50.0, height := 50.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(0, 0, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
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
            "  LET red AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 40.0, height := 40.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::setGroup(\"leaf\", [red])\n  \
             canvas::setGroup(\"outer\", [canvas::Group[id := \"\", position := vector::Float2[x := 0.0, y := 200.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"leaf\"]])\n  \
             LET a AS canvas::DrawItem = canvas::Group[id := \"\", position := vector::Float2[x := 100.0, y := 100.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"leaf\"]\n  \
             LET b AS canvas::DrawItem = canvas::Group[id := \"\", position := vector::Float2[x := 300.0, y := 100.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"leaf\"]\n  \
             LET nested AS canvas::DrawItem = canvas::Group[id := \"\", position := vector::Float2[x := 500.0, y := 100.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"outer\"]\n  \
             LET absent AS canvas::DrawItem = canvas::Group[id := \"\", position := vector::Float2[x := 700.0, y := 100.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"nope\"]\n  \
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
            "  LET red AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 4.0, height := 4.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::setGroup(\"self\", [red, canvas::Group[id := \"\", position := vector::Float2[x := 1.0, y := 1.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"self\"]])\n  \
             canvas::present([canvas::Group[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"self\"]]) TRAP(e)\n  \
               io::print(\"raised \" & toString(e.code))\n  \
               RETURN\n  \
             END TRAP\n  \
             io::print(\"NO RAISE\")\n",
        ),
        (
            "canvas_group_deep",
            "  LET red AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 4.0, height := 4.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::setGroup(\"g0\", [red])\n  \
             MUT i AS Integer = 1\n  \
             WHILE i <= 70\n  \
               canvas::setGroup(\"g\" & toString(i), [canvas::Group[id := \"\", position := vector::Float2[x := 1.0, y := 0.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"g\" & toString(i - 1)]])\n  \
               i = i + 1\n  \
             END WHILE\n  \
             canvas::present([canvas::Group[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"g70\"]]) TRAP(e)\n  \
               io::print(\"raised \" & toString(e.code))\n  \
               RETURN\n  \
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

/// A clipped item inside a translated group carries its clip with it: the clip is in the
/// item's own coordinates, so the group's translation moves both (plan-163-C §3, which
/// reverses plan-116-G's G5 on purpose — a surface-rectangle clip was right for
/// `Bounds`, and wrong for an SVG `clip-path`, which moves with its element).
///
/// A 200x200 square in a group translated by (300, 100), clipped to its own left half,
/// (0, 0)-(100, 200). The square lands at (300, 100)-(500, 300) and the clip with it at
/// (300, 100)-(400, 300): the left half is painted and the right half is not. Had the
/// clip stayed on the surface it would cut (0, 0)-(100, 200), which the square never
/// reaches, and nothing would be painted.
#[test]
fn a_clip_inside_a_translated_group_moves_with_it() {
    let (frame, _) = render(
        "canvas_group_clip",
        &scene(
            "  LET clipped AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 200.0, height := 200.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH canvas::fill(color::rgb(0, 200, 255)) { clip := [[vector::Float2[x := 0.0, y := 0.0], vector::Float2[x := 100.0, y := 0.0], vector::Float2[x := 100.0, y := 200.0], vector::Float2[x := 0.0, y := 200.0]]] }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::setGroup(\"g\", [clipped])\n  \
             canvas::present([canvas::Group[id := \"\", position := vector::Float2[x := 300.0, y := 100.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"g\"]])\n",
        ),
    );
    assert_eq!(
        pixel(&frame, 350, 200),
        (0, 200, 255, 255),
        "the square's left half, inside its moved clip, is painted",
    );
    assert_eq!(
        pixel(&frame, 450, 200),
        (0, 0, 0, 255),
        "the square's right half is outside its clip, which moved with the group",
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
            "  LET red AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 40.0, height := 40.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET keep AS canvas::DrawItem = canvas::Circle[id := \"\", position := vector::Float2[x := 700.0, y := 500.0], radius := 20.0, paint := canvas::fill(color::rgb(0, 255, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::setGroup(\"panel\", [red])\n  \
             LET node AS canvas::DrawItem = canvas::Group[id := \"\", position := vector::Float2[x := 100.0, y := 100.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"panel\"]\n  \
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
            "  LET red AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 40.0, height := 40.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET keep AS canvas::DrawItem = canvas::Circle[id := \"\", position := vector::Float2[x := 700.0, y := 500.0], radius := 20.0, paint := canvas::fill(color::rgb(0, 255, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([keep])\n  \
             MUT i AS Integer = 0\n  \
             WHILE i < 200\n  \
               canvas::setGroup(\"scratch\" & toString(i), [red, red, red, red, red, red, red, red])\n  \
               canvas::present([keep])\n  \
               canvas::removeGroup(\"scratch\" & toString(i))\n  \
               canvas::present([keep])\n  \
               i = i + 1\n  \
             END WHILE\n  \
             LET last AS canvas::DrawItem = canvas::Circle[id := \"\", position := vector::Float2[x := 300.0, y := 300.0], radius := 20.0, paint := canvas::fill(color::rgb(0, 0, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([keep, last])\n",
        ),
    );

    let bytes: i64 = stat(stats.last().expect("a final frame"), "groupBytes=")
        .parse()
        .expect("groupBytes is a number");
    let groups = stat(stats.last().unwrap(), "groups=");
    assert_eq!(groups, "0", "every scratch group was removed: {stats:?}");
    // One eight-item group is 4914 bytes (measured, plan-163-D: 4082 before the gradient
    // gained five fields, 104 bytes an item), so 200 undrained ones would be ~1 MB. The
    // bound allows a single outstanding buffer and not two: the last iteration's may
    // legitimately still be retired, since the gate needs a frame to complete after it.
    assert!(
        bytes < 8192,
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
///
/// The last present puts the same two nodes in the other order: an identical
/// re-present publishes nothing and draws no frame (bug-968), and the drain needs one.
#[test]
fn replacing_a_group_frees_only_the_displaced_buffer() {
    let (_, stats) = render(
        "canvas_group_replace_drain",
        &scene(
            "  LET a AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 40.0, height := 40.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET b AS canvas::DrawItem = canvas::Circle[id := \"\", position := vector::Float2[x := 10.0, y := 10.0], radius := 10.0, paint := canvas::fill(color::rgb(0, 255, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET node AS canvas::DrawItem = canvas::Group[id := \"\", position := vector::Float2[x := 100.0, y := 100.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"panel\"]\n  \
             canvas::setGroup(\"one\", [a])\n  \
             canvas::present([canvas::Group[id := \"\", position := vector::Float2[x := 400.0, y := 400.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"one\"]])\n  \
             canvas::setGroup(\"panel\", [a, a, a])\n  \
             canvas::present([canvas::Group[id := \"\", position := vector::Float2[x := 400.0, y := 400.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"one\"], node])\n  \
             canvas::setGroup(\"panel\", [b])\n  \
             canvas::present([canvas::Group[id := \"\", position := vector::Float2[x := 400.0, y := 400.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"one\"], node])\n  \
             canvas::present([node, canvas::Group[id := \"\", position := vector::Float2[x := 400.0, y := 400.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"one\"]])\n",
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
            "  LET red AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 60.0, height := 60.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET mark AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 700.0, y := 500.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 60.0, height := 60.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(0, 0, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::setGroup(\"child\", [red])\n  \
             canvas::setGroup(\"parent\", [canvas::Group[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"child\"], mark])\n  \
             LET node AS canvas::DrawItem = canvas::Group[id := \"\", position := vector::Float2[x := 100.0, y := 100.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"parent\"]\n  \
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
            "  LET red AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 80.0, height := 80.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::setGroup(\"panel\", [red])\n  \
             LET node AS canvas::DrawItem = canvas::Group[id := \"\", position := vector::Float2[x := 200.0, y := 200.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"panel\"]\n  \
             LET mark AS canvas::DrawItem = canvas::Circle[id := \"\", position := vector::Float2[x := 700.0, y := 500.0], radius := 30.0, paint := canvas::fill(color::rgb(0, 255, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([mark, node])\n  \
             os::sleep(120)\n  \
             canvas::removeGroup(\"panel\")\n  \
             os::sleep(1200)\n",
        )
        .replace("IMPORT collections", "IMPORT collections\nIMPORT os"),
    );
    let frame_path = project.join("frame.rgba");
    let binary = common::build_app_debug(&project, "canvas_group_race");
    let run = Command::new(&binary)
        .env("MFB_HEADLESS", "1")
        // The GTK one too. Omitting it is invisible on the macOS dev host, where the
        // MACAPP flag is the one that matters; on a Linux box the program tries to open
        // a display, fails, and exits 1 -- which these tests then report as a
        // use-after-free or a missing raise. `render` above has always set all three.
        .env("MFB_HEADLESS", "1")
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
            "  LET red AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 80.0, height := 80.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::setGroup(\"panel\", [red])\n  \
             canvas::present([canvas::Group[id := \"\", position := vector::Float2[x := 200.0, y := 200.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"panel\"]])\n",
        ),
    );
    let binary = common::build_app_debug(&project, "canvas_group_exit_race");
    let run = Command::new(&binary)
        .env("MFB_HEADLESS", "1")
        // The GTK one too. Omitting it is invisible on the macOS dev host, where the
        // MACAPP flag is the one that matters; on a Linux box the program tries to open
        // a display, fails, and exits 1 -- which these tests then report as a
        // use-after-free or a missing raise. `render` above has always set all three.
        .env("MFB_HEADLESS", "1")
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
            "  LET red AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 40.0, height := 40.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET here AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 100.0, y := 100.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 40.0, height := 40.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(0, 255, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::setGroup(\"panel\", [red])\n  \
             LET huge AS canvas::DrawItem = canvas::Group[id := \"\", position := vector::Float2[x := 1.0e9, y := 0.0 - 1.0e9], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"panel\"]\n  \
             LET big AS canvas::DrawItem = canvas::Group[id := \"\", position := vector::Float2[x := 100000.0, y := 100000.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"panel\"]\n  \
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
            "  LET up AS canvas::DrawItem = canvas::Polygon[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], contours := [[vector::Float2[x := 0.0, y := 0.0], vector::Float2[x := 100.0, y := 0.0], vector::Float2[x := 50.0, y := 100.0]]], rule := canvas::FillRule.EvenOdd, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET down AS canvas::DrawItem = canvas::Polygon[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], contours := [[vector::Float2[x := 0.0, y := 100.0], vector::Float2[x := 100.0, y := 100.0], vector::Float2[x := 50.0, y := 0.0]]], rule := canvas::FillRule.EvenOdd, paint := canvas::fill(color::rgb(0, 255, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::setGroup(\"tri\", [up])\n  \
             canvas::setGroup(\"tri2\", [down])\n  \
             canvas::present([canvas::Group[id := \"\", position := vector::Float2[x := 200.0, y := 200.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"tri\"], canvas::Group[id := \"\", position := vector::Float2[x := 500.0, y := 200.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"tri2\"]])\n",
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
            "  LET red AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 40.0, height := 40.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET keep AS canvas::DrawItem = canvas::Circle[id := \"\", position := vector::Float2[x := 700.0, y := 500.0], radius := 20.0, paint := canvas::fill(color::rgb(0, 255, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
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
             LET last AS canvas::DrawItem = canvas::Circle[id := \"\", position := vector::Float2[x := 300.0, y := 300.0], radius := 20.0, paint := canvas::fill(color::rgb(0, 0, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
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
    const RED: &str = "  LET red AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 40.0, height := 40.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  ";

    // 1. A flat scene: one run, no offset.
    let (blocks, draws) = d(&format!(
        "{RED}LET b AS canvas::DrawItem = canvas::Circle[id := \"\", position := vector::Float2[x := 300.0, y := 300.0], radius := 20.0, paint := canvas::fill(color::rgb(0, 255, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  canvas::present([red, b])\n"
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
        "{RED}canvas::setGroup(\"g\", [red, red])\n  canvas::present([canvas::Group[id := \"\", position := vector::Float2[x := 100.0, y := 100.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"g\"]])\n"
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
         canvas::setGroup(\"outer\", [canvas::Group[id := \"\", position := vector::Float2[x := 0.0, y := 200.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"inner\"]])\n  \
         canvas::present([canvas::Group[id := \"\", position := vector::Float2[x := 500.0, y := 100.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"outer\"]])\n"
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
         canvas::present([canvas::Group[id := \"\", position := vector::Float2[x := 100.0, y := 100.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"g\"], canvas::Group[id := \"\", position := vector::Float2[x := 300.0, y := 100.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"g\"]])\n"
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
        "{RED}LET mul AS canvas::DrawItem = canvas::Circle[id := \"\", position := vector::Float2[x := 20.0, y := 20.0], radius := 10.0, paint := WITH canvas::fill(color::rgb(0, 255, 0)) {{ blend := canvas::BlendMode.Multiply }}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
           canvas::setGroup(\"g\", [red, mul])
           canvas::present([canvas::Group[id := \"\", position := vector::Float2[x := 100.0, y := 100.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"g\"]])
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

/// An image destroyed while a scene still names it: the frame renders, the item
/// contributes nothing, and nothing raises (plan-116-I Phase 3).
///
/// The reachable shape, and the *only* one. `canvas::destroyImage` consumes its
/// binding, so a program cannot name the image again afterwards — "destroy, then build
/// a new `Picture` from it" is a compile error (`2-203-0055 TYPE_USE_AFTER_MOVE`), not
/// a runtime case. What remains expressible is this: build the item while the image is
/// live, destroy the image, and present the item that already holds it. The scene's
/// copy outlives the binding, which is exactly the lifetime question this letter opened
/// by putting a resource in a record field.
///
/// Asserted as "a frame came back, black" rather than as "the picture is absent",
/// because a `Picture` draws nothing today anyway (bug-484). What would fail here is a
/// raise or a crash reaching through a resource the program has finished with — and
/// that is the property worth pinning now, before `Picture` learns to draw.
#[test]
fn an_image_destroyed_while_a_scene_names_it_renders_a_frame_and_does_not_raise() {
    let (frame, stats) = render(
        "canvas_image_destroyed_in_scene",
        &scene(
            "  LET px AS List OF Byte = [toByte(255), toByte(0), toByte(0), toByte(255)]\n  \
             RES img AS canvas::Image = canvas::createImage(1, 1, px) TRAP(e)\n  \
             RETURN\n  \
             END TRAP\n  \
             LET tile AS canvas::DrawItem = canvas::Picture[id := \"\", shape := canvas::Rectangle[id := \"\", position := vector::Float2[x := 10.0, y := 10.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 40.0, height := 40.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]], clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img]\n  \
             canvas::destroyImage(img)\n  \
             canvas::present([tile])\n",
        ),
    );
    assert!(
        !frame.is_empty(),
        "no frame came back at all — presenting a scene that names a destroyed image \
         must still render: {stats:?}"
    );
    assert!(
        frame.chunks(4).all(|p| p[0] == 0 && p[1] == 0 && p[2] == 0),
        "an item naming a destroyed image contributed ink",
    );
}

/// Destroying a font while the graphics thread is mid-frame is benign (plan-116-I
/// Phase 3, §3's claim — asserted rather than argued).
///
/// `MFB_CANVAS_FRAME_HOLD_MS` is what makes this a real test rather than a hopeful one:
/// it parks the graphics thread inside `__canvas_renderFrame` so the `destroyFont`
/// below lands *while a frame holding that font is being drawn*, not before or after.
/// Without it the two never overlap and the run proves nothing.
///
/// Placed here rather than in `rt_canvas_graphics_thread.rs`, where plan-116-I Phase 3
/// names it, because the hold affordance and the two sibling mid-frame race tests live
/// in this file — a third race test next to them is easier to keep honest than one that
/// has to re-derive the timing (**I6**).
///
/// What would fail: the graphics thread reads the item's resource record to get the
/// backend id, and the worker frees that record. §3 argues the read is safe because the
/// record lives in the worker's arena and is retained for the thread's lifetime — this
/// is that argument, run.
#[test]
fn destroying_a_font_mid_frame_is_clean() {
    let project = common::temp_project(
        "canvas_font_destroy_race",
        &scene(
            "  RES face AS canvas::Font = canvas::loadFont(\"fixture.ttf\") TRAP(e)\n  \
             RETURN\n  \
             END TRAP\n  \
             LET label AS canvas::DrawItem = canvas::Text[id := \"\", position := vector::Float2[x := 100.0, y := 200.0], text := \"AA\", font := face, fontSize := 90.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET mark AS canvas::DrawItem = canvas::Circle[id := \"\", position := vector::Float2[x := 700.0, y := 500.0], radius := 30.0, paint := canvas::fill(color::rgb(0, 255, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([mark, label])\n  \
             os::sleep(120)\n  \
             canvas::destroyFont(face)\n  \
             os::sleep(1200)\n",
        )
        .replace("IMPORT collections", "IMPORT collections\nIMPORT os"),
    );
    std::fs::write(project.join("fixture.ttf"), common::fixture_truetype())
        .expect("write the font fixture");
    let frame_path = project.join("frame.rgba");
    let binary = common::build_app_debug(&project, "canvas_font_destroy_race");
    let run = Command::new(&binary)
        .current_dir(&project)
        .env("MFB_HEADLESS", "1")
        .env("MFB_CANVAS_DUMP", &frame_path)
        .env("MFB_CANVAS_FRAME_HOLD_MS", "600")
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", binary.display()));
    assert!(
        run.status.success(),
        "destroying a font while a frame holding it was mid-render did not exit \
         cleanly. The graphics thread reads the item's resource record for the backend \
         id; if the worker can free that record out from under it, this is where it \
         shows. exit {:?}\n{}\n{}",
        run.status.code(),
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr),
    );
    assert!(
        frame_path.exists(),
        "no frame was written, so the race never actually ran",
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

/// A `Float` literal as MFBASIC spells it: a negative one as `(0.0 - n)`.
fn lit(v: f64) -> String {
    if v < 0.0 {
        format!("(0.0 - {:?})", -v)
    } else {
        format!("{v:?}")
    }
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

/// A `setGroup` under a name a LAYERED scene already draws is seen by the next
/// `presentLayers`, as `present` has always seen it for a flat one — and a group cycle
/// under a layer raises, as it does under `present`.
///
/// Found by plan-150-A: `presentLayers` published the layers and never took the
/// resolved-groups signature. The layers themselves are identical before and after the
/// `setGroup` — a `Group` node is a name — so the second call reported "unchanged" and
/// the new group was never drawn: one frame where there should be two, and the old
/// 20 px box still on screen. The same missing walk is the one that refuses a group
/// nested past the depth limit, which a layered scene therefore never raised for.
#[test]
fn present_layers_sees_a_group_replaced_under_the_same_name() {
    const BOX: &str = "canvas::Rectangle[id := \"\", position := vector::Float2[x := 10.0, y := 10.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := WIDTH, height := 20.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]";
    const NODE: &str = "canvas::Group[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := \"NAME\"]";
    let node = NODE.replace("NAME", "g");
    let (frame, stats) = render(
        "canvas_layers_group_replaced",
        &scene(&format!(
            "  canvas::setGroup(\"g\", [{}])\n  canvas::presentLayers([canvas::DrawLayer[items := [{node}]]])\n  canvas::setGroup(\"g\", [{}])\n  canvas::presentLayers([canvas::DrawLayer[items := [{node}]]])\n",
            BOX.replace("WIDTH", "20.0"),
            BOX.replace("WIDTH", "200.0"),
        )),
    );
    assert_eq!(
        stats.len(),
        2,
        "the second presentLayers, after setGroup replaced the group, drew no frame"
    );
    // (150, 20) is inside the 200 px box and outside the 20 px one.
    let at = ((20 * 900 + 150) * 4) as usize;
    assert_eq!(&frame[at..at + 4], &[255, 0, 0, 255]);

    let cycle = NODE.replace("NAME", "self");
    let (stdout, _) = render_stdout(
        "canvas_layers_group_cycle",
        &scene(&format!(
            "  canvas::setGroup(\"self\", [{cycle}])\n  canvas::presentLayers([canvas::DrawLayer[items := [{cycle}]]]) TRAP(e)\n    io::print(\"raised \" & toString(e.code))\n    RETURN\n  END TRAP\n  io::print(\"NO RAISE\")\n"
        )),
    );
    assert!(
        stdout.contains("raised 77050024"),
        "a group cycle under a layer must raise ErrDepthExceeded from `presentLayers`; \
         got: {stdout}"
    );
}
