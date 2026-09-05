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
const GROUPS: &str = r#"IMPORT app
IMPORT canvas
SUB main()
  app::setMode(app::Mode.Canvas)

  ' 1. a group drawn at (0,0) -- the offset is present but zero
  LET atOrigin AS List OF canvas::DrawItem = [canvas::Rectangle[x := 20.0, y := 20.0, w := 80.0, h := 40.0, paint := canvas::fill(canvas::rgb(255, 0, 0))]]
  canvas::setGroup("atOrigin", atOrigin)

  ' 2. a group drawn at (37, 53) -- a non-zero offset, which a vertex-only
  '    implementation cannot get right for anything position-dependent
  LET moved AS List OF canvas::DrawItem = [canvas::Circle[x := 200.0, y := 60.0, radius := 30.0, paint := canvas::fillStroke(canvas::rgb(0, 160, 220), canvas::rgb(255, 255, 255), 5.0)]]
  canvas::setGroup("moved", moved)

  ' 3. a NESTED group: outer holds inner, so the offsets compose
  LET inner AS List OF canvas::DrawItem = [canvas::Rectangle[x := 0.0, y := 0.0, w := 50.0, h := 50.0, paint := canvas::fill(canvas::rgb(200, 200, 0))]]
  canvas::setGroup("inner", inner)
  LET outer AS List OF canvas::DrawItem = [canvas::Group[name := "inner", dx := 15.0, dy := 25.0]]
  canvas::setGroup("outer", outer)

  ' 4. a DIAMOND: one leaf referenced twice, at two offsets
  LET leaf AS List OF canvas::DrawItem = [canvas::Rectangle[x := 0.0, y := 0.0, w := 60.0, h := 40.0, paint := canvas::fill(canvas::rgb(120, 220, 60))]]
  canvas::setGroup("leaf", leaf)

  ' 5. a CLIPPED item inside a translated group -- the clip is evaluated in SURFACE
  '    space while the shape is evaluated in shape space (section 4.2)
  LET clipped AS List OF canvas::DrawItem = [canvas::Rectangle[x := 0.0, y := 0.0, w := 300.0, h := 60.0, paint := WITH canvas::fill(canvas::rgb(255, 255, 255)) { clip := canvas::Bounds[x := 40.25, y := 0.0, w := 200.5, h := 60.0] }]]
  canvas::setGroup("clipped", clipped)

  ' 6. a GRADIENT-filled item inside a translated group -- the ramp is sampled from
  '    the shape-space point, so a missing fragment offset shows as a shifted ramp
  LET stops AS List OF canvas::GradientStop = [canvas::GradientStop[offset := 0.0, color := canvas::rgb(255, 64, 32)], canvas::GradientStop[offset := 0.55, color := canvas::rgb(250, 230, 90)], canvas::GradientStop[offset := 1.0, color := canvas::rgb(32, 96, 255)]]
  LET ramp AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, startPoint := canvas::Point[x := 0.0, y := 0.0], endPoint := canvas::Point[x := 150.0, y := 60.0], stops := stops]
  LET grad AS List OF canvas::DrawItem = [canvas::Rectangle[x := 0.0, y := 0.0, w := 150.0, h := 60.0, paint := WITH canvas::fill(canvas::rgb(0, 0, 0)) { fillGradient := ramp }]]
  canvas::setGroup("grad", grad)


  LET scene AS List OF canvas::DrawItem = [canvas::Group[name := "atOrigin", dx := 0.0, dy := 0.0], canvas::Group[name := "moved", dx := 37.0, dy := 53.0], canvas::Group[name := "outer", dx := 380.0, dy := 40.0], canvas::Group[name := "leaf", dx := 600.0, dy := 40.0], canvas::Group[name := "leaf", dx := 700.0, dy := 140.0], canvas::Group[name := "clipped", dx := 60.0, dy := 200.0], canvas::Group[name := "grad", dx := 420.0, dy := 200.0]]
  canvas::present(scene)
END SUB
"#;

/// A diamond: one group installed once and named twice, at two offsets.
///
/// Paired with `DIAMOND_FLAT`, which draws the same four rectangles written out
/// longhand at the same absolute coordinates and uses no group at all.
const DIAMOND_GROUPED: &str = r#"IMPORT app
IMPORT canvas
SUB main()
  app::setMode(app::Mode.Canvas)
  LET a AS canvas::DrawItem = canvas::Rectangle[x := 0.0, y := 0.0, w := 70.0, h := 45.0, paint := canvas::fill(canvas::rgb(120, 220, 60))]
  LET b AS canvas::DrawItem = canvas::Circle[x := 100.0, y := 22.0, radius := 18.0, paint := canvas::fillStroke(canvas::rgb(0, 170, 220), canvas::rgb(255, 255, 255), 4.0)]
  canvas::setGroup("pair", [a, b])
  canvas::present([canvas::Group[name := "pair", dx := 120.0, dy := 90.0], canvas::Group[name := "pair", dx := 430.0, dy := 300.0]])
END SUB
"#;

/// The same picture with no group in it — the two shapes written out twice, at the
/// coordinates the group offsets would have put them.
const DIAMOND_FLAT: &str = r#"IMPORT app
IMPORT canvas
SUB main()
  app::setMode(app::Mode.Canvas)
  LET a1 AS canvas::DrawItem = canvas::Rectangle[x := 120.0, y := 90.0, w := 70.0, h := 45.0, paint := canvas::fill(canvas::rgb(120, 220, 60))]
  LET b1 AS canvas::DrawItem = canvas::Circle[x := 220.0, y := 112.0, radius := 18.0, paint := canvas::fillStroke(canvas::rgb(0, 170, 220), canvas::rgb(255, 255, 255), 4.0)]
  LET a2 AS canvas::DrawItem = canvas::Rectangle[x := 430.0, y := 300.0, w := 70.0, h := 45.0, paint := canvas::fill(canvas::rgb(120, 220, 60))]
  LET b2 AS canvas::DrawItem = canvas::Circle[x := 530.0, y := 322.0, radius := 18.0, paint := canvas::fillStroke(canvas::rgb(0, 170, 220), canvas::rgb(255, 255, 255), 4.0)]
  canvas::present([a1, b1, a2, b2])
END SUB
"#;

/// A scene whose only item names a group that was never installed.
const ABSENT_GROUP: &str = r#"IMPORT app
IMPORT canvas
SUB main()
  app::setMode(app::Mode.Canvas)
  canvas::present([canvas::Group[name := "neverInstalled", dx := 40.0, dy := 60.0]])
END SUB
"#;

/// The same scene with nothing in it at all — the control for `ABSENT_GROUP`.
const EMPTY_SCENE: &str = r#"IMPORT app
IMPORT canvas
SUB main()
  app::setMode(app::Mode.Canvas)
  canvas::present([])
END SUB
"#;

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
        "program {} (metal={metal}):\n{}\n{}",
        common::exit_description(&run.status),
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

/// Every group case matches the software oracle — plan-116-H Phase 3's acceptance.
///
/// What this asserts is not "groups draw" but "the per-draw OFFSET reaches both shader
/// stages". Every item in the scene lives inside a group, so a backend that ignored the
/// offset would produce a complete, plausible picture with every shape stacked at the
/// origin — the failure `.ai/canvas-threading.md` §10 is about, and one that only a
/// comparison against the software oracle at a NON-ZERO offset can see.
///
/// The cases are chosen so each can only pass for the right reason:
///
/// * a group at `(0,0)` — the offset is present and zero;
/// * a group at `(37,53)` — non-zero;
/// * a **nested** group — `(380,40)` and `(15,25)` must compose to `(395,65)`;
/// * a **diamond** — one leaf, two references, two offsets, sharing one base;
/// * a **clipped** item inside a translated group — the clip is evaluated in surface
///   space while the shape is evaluated in shape space, so a backend that moved both
///   together would drag the clip window along with the group;
/// * a **gradient-filled** item inside a translated group — the ramp is sampled per
///   fragment, so a vertex-stage-only offset draws the shape correctly with the ramp
///   shifted inside it.
///
/// That last case is why `setVertexBytes:` alone is not enough and the fragment stage
/// gets the offset too. The `Text`-in-a-group case needs a font file beside the binary
/// and is covered by `scripts/test-canvas-vulkan.sh`, which ships one.
///
/// Measured when it landed: **worst channel delta 1, 0.0028% of pixels differing** —
/// inside `Tolerance::GPU_DEFAULT`'s per-pixel bound, not merely its population budget.
#[test]
fn every_group_case_matches_the_software_oracle() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let program = build("canvas_metal_groups", GROUPS);
    let (software, _) = render(&program, false, "sw");
    let (gpu, stats) = render(&program, true, "gpu");
    if !metal_built(&stats) {
        return; // no Metal device on this host (§metal_built)
    }
    // Asserted BEFORE the pixels. A declined group scene falls back to software, and
    // then this test would be comparing the software renderer against itself and
    // passing for no reason — which is exactly what it did while plan-116-G's decline
    // was in place.
    assert!(
        !stats.contains("gpuFrames=0"),
        "the group scene never reached Metal, so the comparison below would be the \
         software renderer against itself: {stats}"
    );
    if let Err(diff) = compare_within_tolerance(&gpu, &software, Tolerance::GPU_DEFAULT) {
        panic!(
            "the Metal backend disagrees with the software oracle on a group scene: \
             {diff}\n\
             Each case sits in its own region, so the coordinate localizes it: (20,20) \
             at-origin; (237,113) moved; (395,65) nested; (600,40) and (700,140) the \
             diamond; (60,200) clipped; (420,200) gradient. A shape at the ORIGIN \
             instead of its group offset means the offset never reached the stage that \
             shape depends on — the vertex stage moves the quad, the fragment stage \
             moves the query point, and a gradient needs both."
        );
    }
}

/// Grouping changes cost, not pixels.
///
/// The strongest check available for this letter, and the one that does not depend on a
/// stored reference: the same picture is drawn twice, once as a group named at two
/// offsets and once as four items written out longhand at the absolute coordinates those
/// offsets imply. The two frames must be **byte-identical**.
///
/// Compared exactly rather than within tolerance, and both sides on the GPU. Both frames
/// come from the same backend on the same host in the same run, so every source of the
/// slack `Tolerance::GPU_DEFAULT` exists for — float blending, sRGB conversion, rounding
/// — is common to both and cancels. What is left is precisely the question being asked:
/// does routing an item through a group change the pixels it produces? A tolerance here
/// would hide exactly the small offset errors this letter is about, because a shape one
/// pixel off still agrees with itself to within two steps almost everywhere.
#[test]
fn a_diamond_renders_identically_to_the_flat_scene_it_stands_for() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let grouped = build("canvas_metal_diamond_grouped", DIAMOND_GROUPED);
    let flat = build("canvas_metal_diamond_flat", DIAMOND_FLAT);
    let (grouped_frame, stats) = render(&grouped, true, "gpu");
    if !metal_built(&stats) {
        return; // no Metal device on this host (§metal_built)
    }
    assert!(
        !stats.contains("gpuFrames=0"),
        "the grouped scene never reached the GPU, so this would compare two software \
         frames and prove nothing about the backend: {stats}"
    );
    let (flat_frame, flat_stats) = render(&flat, true, "gpu");
    assert!(
        !flat_stats.contains("gpuFrames=0"),
        "the flat control scene never reached the GPU: {flat_stats}"
    );
    if let Err(diff) = compare_exact(&grouped_frame, &flat_frame) {
        panic!(
            "a group's children do not land where the same items written out longhand \
             do: {diff}\n\
             The two scenes differ only in that one routes its items through a group at \
             (120,90) and (430,300). A difference at one of those two places is the \
             per-draw offset; a difference at both is the group expansion itself; and a \
             difference in the stroked circle only is the fragment stage, whose distance \
             is evaluated at an absolute point."
        );
    }
}

/// A scene whose only item names a group that was never installed draws nothing — on the
/// GPU as well as in software.
///
/// The GPU arm is the point. `canvas::groupResolve` returns no slot, the walk emits no
/// blocks, and the draw list is empty; a backend that treated "no entries" as "draw
/// everything published so far" or that walked a zero-length list badly would show it
/// here and nowhere else. Compared against a genuinely empty scene rather than against a
/// stored image, so it cannot pass by both sides being equally broken in the same way as
/// a reference made from one of them.
#[test]
fn a_group_naming_an_absent_group_draws_nothing_on_the_gpu() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let absent = build("canvas_metal_absent_group", ABSENT_GROUP);
    let empty = build("canvas_metal_empty_scene", EMPTY_SCENE);
    let (absent_frame, stats) = render(&absent, true, "gpu");
    if !metal_built(&stats) {
        return; // no Metal device on this host (§metal_built)
    }
    let (empty_frame, _) = render(&empty, true, "gpu");
    if let Err(diff) = compare_exact(&absent_frame, &empty_frame) {
        panic!(
            "a `canvas::Group` naming a group that was never installed drew something: \
             {diff}\n\
             It must resolve to no blocks and no draw entries, exactly as an empty \
             scene does."
        );
    }
}
