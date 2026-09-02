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
    let build = Command::new(common::mfb_exe())
        .arg("build")
        .arg("-app")
        .arg(&project)
        .output()
        .expect("run mfb build -app");
    assert!(
        build.status.success(),
        "mfb build -app failed:\n{}\n{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr),
    );

    let frame = project.join("frame.rgba");
    let stats = project.join("stats.txt");
    let binary = app_binary(&project, name);
    let run = Command::new(&binary)
        .env("MFB_MACAPP_HEADLESS", "1")
        .env("MFB_WINAPP_HEADLESS", "1")
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
        "program exited {:?}:\n{}\n{}",
        run.status.code(),
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

/// The built executable, which on macOS is inside an `.app` bundle.
fn app_binary(project: &std::path::Path, name: &str) -> std::path::PathBuf {
    let bundle = project
        .join("build")
        .join(format!("{name}.app"))
        .join("Contents")
        .join("MacOS")
        .join(name);
    if bundle.exists() {
        return bundle;
    }
    let plain = project.join("build").join(name);
    if plain.exists() {
        return plain;
    }
    project
        .join("build")
        .join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
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
