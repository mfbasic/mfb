//! The Metal backend renders the same picture as the software oracle (plan-98-E).
//!
//! The oracle is the software rasteriser, not a stored image: plan-98-A invariant 7
//! makes the software path the reference every GPU backend is measured against, so
//! these tests render the *same program twice* — once with `MFB_CANVAS_GPU=1` and
//! once without — and diff the two frames. That is stronger than diffing the GPU
//! frame against a checked-in PNG, because it cannot drift out of date: if the
//! rasteriser changes, both sides change together and the comparison still means
//! "the two backends agree".
//!
//! The gate is `Tolerance::GPU_DEFAULT` (plan-98-A invariant 5) — GPU output is not
//! required to be exact-match. It currently *is* exact for these scenes, which is a
//! measurement rather than a promise: the pipeline writes a `BGRA8Unorm_sRGB` target
//! and the shader emits linear premultiplied colour, so the same arithmetic the
//! software path does by hand happens in the raster hardware. Tightening the
//! assertion to exact equality would make any future antialiasing change in either
//! backend read as a failure when it is not one.
//!
//! **These tests are macOS-only and they skip rather than fail elsewhere.** They also
//! skip on a macOS host with no Metal device — a real case (some VMs, some CI
//! lanes) — which is why the skip is decided from the program's own
//! `MFB_CANVAS_STATS` line rather than from the platform: a host that reports a
//! device must render, and one that does not must not silently pass.

mod common;

use common::canvas_image::{compare_exact, compare_within_tolerance, Frame, Tolerance};
use std::path::PathBuf;
use std::process::Command;

const WIDTH: u32 = 900;
const HEIGHT: u32 = 640;

/// Axis-aligned rectangles, opaque and translucent, overlapping each other and the
/// background.
///
/// Every one of them is a shape Phase 1's flat-fill fragment shader reproduces, which
/// is what makes this the scene that measures the *colour* chain rather than the
/// geometry: `rgba(0,0,255,128)` over mid grey and `rgba(255,255,255,40)` over black
/// both land far from either endpoint, so an sRGB-versus-linear mistake anywhere in
/// the chain moves them.
const RECTANGLES: &str = r#"IMPORT app
IMPORT canvas

SUB main()
  app::setMode(app::Mode.Canvas)
  LET red AS canvas::DrawItem = canvas::Rectangle[x := 10.0, y := 10.0, w := 50.0, h := 50.0, paint := canvas::fill(canvas::rgb(255, 0, 0))]
  LET grey AS canvas::DrawItem = canvas::Rectangle[x := 100.0, y := 10.0, w := 60.0, h := 60.0, paint := canvas::fill(canvas::rgb(128, 128, 128))]
  LET half AS canvas::DrawItem = canvas::Rectangle[x := 120.0, y := 30.0, w := 60.0, h := 60.0, paint := canvas::fill(canvas::rgba(0, 0, 255, 128))]
  LET faint AS canvas::DrawItem = canvas::Rectangle[x := 200.0, y := 10.0, w := 80.0, h := 40.0, paint := canvas::fill(canvas::rgba(255, 255, 255, 40))]
  LET wide AS canvas::DrawItem = canvas::Rectangle[x := 300.0, y := 100.0, w := 500.0, h := 300.0, paint := canvas::fill(canvas::rgb(17, 200, 90))]
  LET over AS canvas::DrawItem = canvas::Rectangle[x := 400.0, y := 150.0, w := 200.0, h := 100.0, paint := canvas::fill(canvas::rgba(255, 200, 0, 200))]
  canvas::present([red, grey, half, faint, wide, over])
END SUB
"#;

/// Every primitive the SDF shader draws, in one scene.
///
/// The smiley from `plan-98-api.md` — the scene that shaped the API — plus a rounded
/// rect with both a fill and a stroke, a thick line, and **two** translucent polygons.
/// That covers each arm of the distance dispatch, both paint channels, the
/// corner-radius term, an arc's sweep test, and the polygon edge buffer, which between
/// them are every path the fragment shader has.
///
/// **Two polygons, not one, and that is load-bearing since plan-116-A.** Metal's edges
/// used to be copied into the command buffer per item, so every polygon's array started
/// at index 0 and the edge base was always zero. They now take a slice of one region
/// that serves the whole frame, so the base is a real per-item value — and with a single
/// polygon in the scene the base is *still* zero, so a base that was never written would
/// pass. The second polygon is the only thing that reads a non-zero one. (The Vulkan
/// harness has carried two for exactly this reason since plan-98-F; Metal needs it now
/// for the first time.)
const PRIMITIVES: &str = r#"IMPORT app
IMPORT canvas

SUB main()
  app::setMode(app::Mode.Canvas)

  LET yellow AS canvas::Color = canvas::rgb(255, 255, 0)
  LET green AS canvas::Color = canvas::rgb(0, 160, 0)

  LET face AS canvas::DrawItem = canvas::Circle[x := 450.0, y := 320.0, radius := 150.0, paint := canvas::fill(yellow)]
  LET eyeL AS canvas::DrawItem = canvas::Circle[x := 400.0, y := 280.0, radius := 22.0, paint := canvas::fill(green)]
  LET eyeR AS canvas::DrawItem = canvas::Circle[x := 500.0, y := 280.0, radius := 22.0, paint := canvas::fill(green)]
  LET smile AS canvas::DrawItem = canvas::Arc[x := 450.0, y := 335.0, radius := 90.0, startAngle := 0.0, endAngle := 3.14159, cap := canvas::CapStyle.Butt, paint := canvas::stroke(green, 14.0)]
  LET box AS canvas::DrawItem = canvas::Rectangle[x := 10.0, y := 10.0, w := 50.0, h := 50.0, paint := canvas::fill(canvas::rgb(255, 0, 0))]
  LET rounded AS canvas::DrawItem = canvas::RoundedRect[x := 100.0, y := 10.0, w := 90.0, h := 60.0, cornerRadius := 18.0, paint := canvas::fillStroke(canvas::rgb(0, 0, 255), canvas::rgb(255, 255, 255), 4.0)]
  LET line AS canvas::DrawItem = canvas::Line[x1 := 220.0, y1 := 20.0, x2 := 380.0, y2 := 90.0, cap := canvas::CapStyle.Round, paint := canvas::stroke(canvas::rgb(255, 128, 0), 9.0)]
  LET tri AS canvas::DrawItem = canvas::Polygon[points := [canvas::Point[x := 600.0, y := 40.0], canvas::Point[x := 700.0, y := 40.0], canvas::Point[x := 650.0, y := 130.0]], paint := canvas::fill(canvas::rgba(0, 200, 255, 180))]
  ' The second polygon, and concave on purpose: it is the one item in this scene drawn
  ' from a non-zero edge base, and the crossing-count sign test only disagrees with the
  ' nearest-edge magnitude on a shape that is not convex -- so a wrong base here shows as
  ' a wrong FILL, not merely a shifted outline.
  LET arrow AS canvas::DrawItem = canvas::Polygon[points := [canvas::Point[x := 60.0, y := 400.0], canvas::Point[x := 160.0, y := 400.0], canvas::Point[x := 160.0, y := 360.0], canvas::Point[x := 230.0, y := 430.0], canvas::Point[x := 160.0, y := 500.0], canvas::Point[x := 160.0, y := 460.0], canvas::Point[x := 60.0, y := 460.0]], paint := canvas::fill(canvas::rgba(0, 180, 180, 200))]

  ' plan-116-B: one item per non-Normal BlendMode, and one clipped item.
  '
  ' Without these the frame binds only Normal's pipeline and never takes the clip
  ' path, so three of the four pipelines this letter builds would go unexercised on
  ' Metal while the suite still reported success.
  '
  ' On a mid-grey patch on purpose: over black, Multiply is a no-op and Screen and Add
  ' are indistinguishable from Normal, so a wrong pipeline would look right.
  '
  ' `blendStroke` is the sharp one -- non-Normal AND both filled and stroked, which the
  ' shader cannot compose in one pass (the stroke-over-fill identity is Normal-only),
  ' so it must be emitted as two adjacent instances.
  '
  ' Small on purpose: a blended pixel agrees with the oracle to within a step or two
  ' but rarely exactly (the oracle blends through a 16-bit linear table, the hardware
  ' in float), and Tolerance::GPU_DEFAULT's population budget is a fraction of the
  ' WHOLE frame -- a large blended patch would exhaust it without testing anything more.
  LET ground AS canvas::DrawItem = canvas::Rectangle[x := 20.0, y := 400.0, w := 360.0, h := 120.0, paint := canvas::fill(canvas::rgb(128, 128, 128))]
  LET blendMul AS canvas::DrawItem = canvas::Circle[x := 70.0, y := 460.0, radius := 14.0, paint := WITH canvas::fill(canvas::rgb(230, 120, 40)) { blend := canvas::BlendMode.Multiply }]
  LET blendScr AS canvas::DrawItem = canvas::Circle[x := 170.0, y := 460.0, radius := 14.0, paint := WITH canvas::fill(canvas::rgb(230, 120, 40)) { blend := canvas::BlendMode.Screen }]
  LET blendAdd AS canvas::DrawItem = canvas::Circle[x := 270.0, y := 460.0, radius := 14.0, paint := WITH canvas::fill(canvas::rgb(230, 120, 40)) { blend := canvas::BlendMode.Add }]
  LET blendStroke AS canvas::DrawItem = canvas::Circle[x := 350.0, y := 460.0, radius := 12.0, paint := WITH canvas::fillStroke(canvas::rgb(230, 120, 40), canvas::rgb(40, 120, 230), 8.0) { blend := canvas::BlendMode.Multiply }]
  LET clippedBox AS canvas::DrawItem = canvas::Rectangle[x := 420.0, y := 400.0, w := 300.0, h := 60.0, paint := WITH canvas::fill(canvas::rgb(255, 255, 255)) { clip := canvas::Bounds[x := 460.25, y := 400.0, w := 200.5, h := 60.0] }]

  ' plan-116-C: transformed items, so the shader's inverse-map path actually runs.
  ' A rotation exercises the gradient correction on a curved edge; the non-uniform
  ' scale is the case Phase 1 measured sqrt(|det M|) as 37/255 wrong on. Rotated TEXT
  ' is covered by rt_canvas_font's a_rotated_text_run_draws_rotated, which owns the
  ' font fixture -- this scene has no font.
  LET rotT AS canvas::Transform = canvas::Transform[a := 0.7071067811865476, b := 0.7071067811865476, c := 0.0 - 0.7071067811865476, d := 0.7071067811865476, tx := 120.0, ty := 560.0]
  LET rotBox AS canvas::DrawItem = canvas::Rectangle[x := 0.0 - 25.0, y := 0.0 - 25.0, w := 50.0, h := 50.0, paint := WITH canvas::fill(canvas::rgb(255, 200, 40)) { transform := rotT }]
  LET scaleT AS canvas::Transform = canvas::Transform[a := 2.0, b := 0.0, c := 0.0, d := 1.0, tx := 250.0, ty := 560.0]
  LET scaleDot AS canvas::DrawItem = canvas::Circle[x := 0.0, y := 0.0, radius := 18.0, paint := WITH canvas::fillStroke(canvas::rgb(90, 200, 255), canvas::rgb(255, 255, 255), 6.0) { transform := scaleT }]

  ' plan-116-D: the same line twice, butt and round, so the shader's cap arm actually
  ' runs on the GPU. `line` above is round-capped and pre-dates this letter; without a
  ' butt one the new branch would be emitted and never executed, and a backend whose
  ' butt arm is wrong would still match the oracle everywhere the scene looks.
  ' Thick and short, because the cap is a half-width feature: at 24 px wide the two
  ' styles differ over a visibly large region rather than one antialiased pixel.
  LET capButt AS canvas::DrawItem = canvas::Line[x1 := 120.0, y1 := 600.0, x2 := 240.0, y2 := 600.0, cap := canvas::CapStyle.Butt, paint := canvas::stroke(canvas::rgb(255, 240, 120), 24.0)]
  LET capRound AS canvas::DrawItem = canvas::Line[x1 := 320.0, y1 := 600.0, x2 := 440.0, y2 := 600.0, cap := canvas::CapStyle.Round, paint := canvas::stroke(canvas::rgb(255, 240, 120), 24.0)]
  ' And a ROUND-capped arc, for the same reason. `smile` above is butt-capped -- which
  ' is what every arc was before plan-116-D -- so without this the arc's cap-disc arm
  ' is compiled into both shaders and never taken. The sweep stops at 0.6*PI so both
  ' ends are visible rather than one running off the item's own band.
  LET capArc AS canvas::DrawItem = canvas::Arc[x := 620.0, y := 600.0, radius := 60.0, startAngle := 0.0, endAngle := 1.884955592153876, cap := canvas::CapStyle.Round, paint := canvas::stroke(canvas::rgb(120, 255, 200), 20.0)]

  canvas::present([box, rounded, line, tri, arrow, face, eyeL, eyeR, smile, ground, blendMul, blendScr, blendAdd, blendStroke, clippedBox, rotBox, scaleDot, capButt, capRound, capArc])
END SUB
"#;

/// A polygon with more edges than the shader's edge buffer holds.
///
/// `setFragmentBytes:length:atIndex:` is capped at 4 KB and each edge crosses as four
/// 16.16 ints, so 300 edges do not fit. This is the one scene the Metal renderer
/// still declines, and the rectangle beside it is there so the frame is not blank —
/// a fallback that rendered nothing would compare equal to a fallback that rendered
/// nothing, and prove nothing.
const TOO_MANY_EDGES: &str = r#"IMPORT app
IMPORT canvas
IMPORT collections
IMPORT math

SUB main()
  app::setMode(app::Mode.Canvas)
  MUT points AS List OF canvas::Point = []
  MUT i AS Integer = 0
  WHILE i < 300
    LET a AS Float = toFloat(i) * 6.283185307179586 / 300.0
    points = collections::append(points, canvas::Point[x := 450.0 + 200.0 * math::cos(a), y := 320.0 + 200.0 * math::sin(a)])
    i = i + 1
  END WHILE
  LET ring AS canvas::DrawItem = canvas::Polygon[points := points, paint := canvas::fill(canvas::rgb(0, 200, 255))]
  LET box AS canvas::DrawItem = canvas::Rectangle[x := 10.0, y := 10.0, w := 50.0, h := 50.0, paint := canvas::fill(canvas::rgb(0, 255, 0))]
  canvas::present([box, ring])
END SUB
"#;

/// Many polygons that individually fit but together overflow the frame's edge region.
///
/// New in plan-116-A, and it covers a decline that did not exist before it. Metal's
/// edges used to ride an unbounded per-item `setFragmentBytes:` payload, so the only cap
/// was per *item*; they now take a slice of one region serving the whole frame
/// (`METAL_MAX_FRAME_EDGES` = 16384), so the cap is a frame total, exactly as Vulkan's
/// has always been.
///
/// 200 rings of 200 edges is 40,000 edges. Each ring is far inside the per-item
/// `__CANVAS_METAL_MAX_EDGES` (256), so `TOO_MANY_EDGES` above cannot reach this case —
/// only the *sum* is over, which is precisely the new condition. The rectangle is there
/// so the frame is not blank: a fallback that rendered nothing would compare equal to a
/// fallback that rendered nothing and prove nothing.
const TOO_MANY_FRAME_EDGES: &str = r#"IMPORT app
IMPORT canvas
IMPORT collections
IMPORT math

SUB main()
  app::setMode(app::Mode.Canvas)
  MUT scene AS List OF canvas::DrawItem = [canvas::Rectangle[x := 10.0, y := 10.0, w := 50.0, h := 50.0, paint := canvas::fill(canvas::rgb(0, 255, 0))]]
  MUT ring AS Integer = 0
  WHILE ring < 200
    MUT points AS List OF canvas::Point = []
    MUT i AS Integer = 0
    WHILE i < 200
      LET a AS Float = toFloat(i) * 6.283185307179586 / 200.0
      points = collections::append(points, canvas::Point[x := 450.0 + toFloat(ring) + 100.0 * math::cos(a), y := 320.0 + 100.0 * math::sin(a)])
      i = i + 1
    END WHILE
    LET poly AS canvas::DrawItem = canvas::Polygon[points := points, paint := canvas::fill(canvas::rgba(0, 200, 255, 60))]
    scene = collections::append(scene, poly)
    ring = ring + 1
  END WHILE
  canvas::present(scene)
END SUB
"#;

/// A built program, kept so both renders run the same binary.
struct Program {
    project: PathBuf,
    binary: PathBuf,
}

impl Drop for Program {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.project);
    }
}

fn build(name: &str, source: &str) -> Program {
    let project = common::temp_project(name, source);
    let binary = common::build_app(&project, name);
    Program { project, binary }
}

/// Render one frame, returning it with the stats line the run reported.
fn render(program: &Program, metal: bool, tag: &str) -> (Frame, String) {
    let frame_path = program.project.join(format!("frame-{tag}.rgba"));
    let stats_path = program.project.join(format!("stats-{tag}.txt"));
    let mut command = Command::new(&program.binary);
    command
        .env("MFB_MACAPP_HEADLESS", "1")
        .env("MFB_WINAPP_HEADLESS", "1")
        .env("MFB_CANVAS_SYNC", "1")
        .env("MFB_CANVAS_STATS", &stats_path)
        .env("MFB_CANVAS_DUMP", &frame_path);
    if metal {
        command.env("MFB_CANVAS_GPU", "1");
    }
    let run = command
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", program.binary.display()));
    assert!(
        run.status.success(),
        "program exited {:?} (metal={metal}):\n{}\n{}",
        run.status.code(),
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr),
    );
    let pixels = std::fs::read(&frame_path).expect("canvas dump written");
    let stats = std::fs::read_to_string(&stats_path).expect("canvas stats written");
    (Frame::from_rgba(WIDTH, HEIGHT, pixels), stats)
}

/// `true` when the run actually built a Metal pipeline; `false` only when the host
/// has no Metal device at all.
///
/// The distinction matters more than it looks. A test that returned early whenever
/// `metalReady=FALSE` would pass silently on a machine where the pipeline is *broken*
/// — an MSL syntax error, a missing entry point, a bad blend enum — which is exactly
/// the class of bug these tests exist to catch. So the only sanctioned skip is
/// `metal=FALSE`, meaning `MTLCreateSystemDefaultDevice` returned nil; a host that
/// reports a device and then fails to build a pipeline **fails the test**.
fn metal_built(stats: &str) -> bool {
    let line = stats
        .lines()
        .next_back()
        .expect("the stats file must carry at least one frame's line");
    if !line.contains("metal=TRUE") {
        return false; // no Metal device on this host — the one legitimate skip
    }
    assert!(
        line.contains("metalReady=TRUE"),
        "this host reports a Metal device but the pipeline did not build — that is a \
         broken shader or pipeline descriptor, not a missing GPU: {line}"
    );
    true
}

/// The Metal backend draws the rectangle scene the same as the software oracle.
#[test]
fn rectangles_match_the_software_oracle_within_tolerance() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let program = build("canvas_metal_rects", RECTANGLES);
    let (software, _) = render(&program, false, "sw");
    let (gpu, stats) = render(&program, true, "gpu");
    if !metal_built(&stats) {
        return; // no Metal device on this host (§metal_built)
    }
    assert!(
        !stats.contains("gpuFrames=0"),
        "MFB_CANVAS_GPU=1 did not select the Metal renderer: {stats}"
    );
    if let Err(diff) = compare_within_tolerance(&gpu, &software, Tolerance::GPU_DEFAULT) {
        panic!(
            "the Metal backend disagrees with the software oracle: {diff}\n\
             Root-cause this against the software reference — the premultiplied \
             linear blend, the sRGB target format and the Y-down NDC mapping are the \
             three places a whole-scene shift comes from."
        );
    }
}

/// The full primitive set matches the software oracle within tolerance.
///
/// This is Phase 2's acceptance: the SDF fragment shader evaluates the same distance
/// functions the software rasteriser does, so a circle is round on both, an arc
/// sweeps the same sector, and a rounded rect's corners have the same radius.
///
/// Measured when it landed: **worst channel delta 1**, and no pixel differs by more
/// than two steps — inside `Tolerance::GPU_DEFAULT`'s per-pixel bound rather than
/// merely inside its population budget.
///
/// It did not start there. Blending in float against an oracle that quantizes
/// coverage to a whole 0..255 gave a worst delta of 5 on 572 pixels, because the
/// sRGB encode near black is steep enough that ONE coverage step moves a dark channel
/// by up to 13 output steps (measured over the oracle's own table). The fix was to
/// quantize coverage in the shader the same way the oracle does — **not** to raise
/// the tolerance, which is what "these are placeholders, not guesses to be loosened
/// until something passes" in `Tolerance::GPU_DEFAULT` is there to prevent.
#[test]
fn the_full_primitive_set_matches_the_software_oracle() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let program = build("canvas_metal_primitives", PRIMITIVES);
    let (software, _) = render(&program, false, "sw");
    let (gpu, stats) = render(&program, true, "gpu");
    if !metal_built(&stats) {
        return; // no Metal device on this host (§metal_built)
    }
    if let Err(diff) = compare_within_tolerance(&gpu, &software, Tolerance::GPU_DEFAULT) {
        panic!(
            "the Metal backend disagrees with the software oracle on the primitive \
             set: {diff}\n\
             Localize the primitive at that coordinate. A whole-shape mismatch is a \
             distance function; a rim of edge pixels is coverage; a uniform shift is \
             the sRGB/linear chain."
        );
    }
}

/// A scene the shader cannot draw is declined, not drawn wrongly.
///
/// This is the test that keeps `MFB_CANVAS_GPU=1` honest. A backend that drew a
/// declined scene approximately — a truncated polygon, say — would still report
/// success, and the picture would be wrong in a way no other test looks at.
///
/// The assertion is deliberately **exact**, not within tolerance: the fallback runs
/// the identical software renderer on the identical scene, so anything other than
/// byte equality means the Metal path drew something it should not have.
#[test]
fn an_unsupported_scene_falls_back_to_the_software_renderer() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let program = build("canvas_metal_fallback", TOO_MANY_EDGES);
    let (software, _) = render(&program, false, "sw");
    let (gpu, stats) = render(&program, true, "gpu");
    if !metal_built(&stats) {
        return;
    }
    if let Err(diff) = compare_exact(&gpu, &software) {
        panic!(
            "a 300-edge polygon does not fit the shader's edge buffer, so the renderer \
             must decline the whole scene and let the software oracle draw it — the \
             two frames must be byte-identical, but {diff}"
        );
    }
}

/// A frame whose polygons *individually* fit but *together* overflow the edge region is
/// declined too.
///
/// plan-116-A's one named compatibility change, and the test that pins it as a decline
/// rather than as truncation. Before that letter Metal had no frame-wide edge budget at
/// all — each polygon's edges were copied into the command buffer as they were recorded
/// — so this scene rendered on the GPU. It now goes to software, which is the oracle, so
/// the picture is at least as correct.
///
/// Asserted **exactly**, like the per-item decline above: the fallback runs the identical
/// software renderer on the identical scene, so anything other than byte equality means
/// the Metal path drew part of a scene it should have refused. Asserting it by pixels
/// rather than by reading a stats flag is the point — a renderer that silently truncated
/// at 16384 edges would still report `gpuSelected=TRUE` and look healthy.
#[test]
fn a_frame_whose_polygons_together_overflow_the_edge_region_falls_back() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let program = build("canvas_metal_frame_edges", TOO_MANY_FRAME_EDGES);
    let (software, _) = render(&program, false, "sw");
    let (gpu, stats) = render(&program, true, "gpu");
    if !metal_built(&stats) {
        return;
    }
    assert!(
        software.pixels.iter().any(|&b| b != 0),
        "the software render drew nothing, so the comparison would be vacuous",
    );
    if let Err(diff) = compare_exact(&gpu, &software) {
        panic!(
            "200 rings of 200 edges is 40,000 edges, past METAL_MAX_FRAME_EDGES — the \
             renderer must decline the whole frame and let the software oracle draw it, \
             so the two frames must be byte-identical, but {diff}"
        );
    }
}
