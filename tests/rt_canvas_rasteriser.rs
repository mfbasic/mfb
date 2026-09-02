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
             paint := canvas::stroke(canvas::rgb(0, 160, 0), 14.0)]\n  \
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
             paint := canvas::stroke(canvas::rgb(0, 160, 0), 8.0)]\n  canvas::present([a])\n",
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
         endAngle := 3.14159, paint := canvas::stroke(canvas::rgb(0, 160, 0), 8.0)]\n  \
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
