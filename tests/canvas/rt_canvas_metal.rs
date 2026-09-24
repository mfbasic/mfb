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

#[path = "../common/mod.rs"]
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
IMPORT color

SUB main()
  app::setMode(app::Mode.Canvas)
  LET red AS canvas::DrawItem = canvas::Rectangle[x := 10.0, y := 10.0, w := 50.0, h := 50.0, paint := canvas::fill(color::rgb(255, 0, 0))]
  LET grey AS canvas::DrawItem = canvas::Rectangle[x := 100.0, y := 10.0, w := 60.0, h := 60.0, paint := canvas::fill(color::rgb(128, 128, 128))]
  LET half AS canvas::DrawItem = canvas::Rectangle[x := 120.0, y := 30.0, w := 60.0, h := 60.0, paint := canvas::fill(color::rgba(0, 0, 255, 128))]
  LET faint AS canvas::DrawItem = canvas::Rectangle[x := 200.0, y := 10.0, w := 80.0, h := 40.0, paint := canvas::fill(color::rgba(255, 255, 255, 40))]
  LET wide AS canvas::DrawItem = canvas::Rectangle[x := 300.0, y := 100.0, w := 500.0, h := 300.0, paint := canvas::fill(color::rgb(17, 200, 90))]
  LET over AS canvas::DrawItem = canvas::Rectangle[x := 400.0, y := 150.0, w := 200.0, h := 100.0, paint := canvas::fill(color::rgba(255, 200, 0, 200))]
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
IMPORT color

SUB main()
  app::setMode(app::Mode.Canvas)

  LET yellow AS color::Color = color::rgb(255, 255, 0)
  LET green AS color::Color = color::rgb(0, 160, 0)

  LET face AS canvas::DrawItem = canvas::Circle[x := 450.0, y := 320.0, radius := 150.0, paint := canvas::fill(yellow)]
  LET eyeL AS canvas::DrawItem = canvas::Circle[x := 400.0, y := 280.0, radius := 22.0, paint := canvas::fill(green)]
  LET eyeR AS canvas::DrawItem = canvas::Circle[x := 500.0, y := 280.0, radius := 22.0, paint := canvas::fill(green)]
  LET smile AS canvas::DrawItem = canvas::Arc[x := 450.0, y := 335.0, radius := 90.0, startAngle := 0.0, endAngle := 3.14159, cap := canvas::CapStyle.Butt, paint := canvas::stroke(green, 14.0)]
  LET box AS canvas::DrawItem = canvas::Rectangle[x := 10.0, y := 10.0, w := 50.0, h := 50.0, paint := canvas::fill(color::rgb(255, 0, 0))]
  LET rounded AS canvas::DrawItem = canvas::RoundedRect[x := 100.0, y := 10.0, w := 90.0, h := 60.0, cornerRadius := 18.0, paint := canvas::fillStroke(color::rgb(0, 0, 255), color::rgb(255, 255, 255), 4.0)]
  LET line AS canvas::DrawItem = canvas::Line[x1 := 220.0, y1 := 20.0, x2 := 380.0, y2 := 90.0, cap := canvas::CapStyle.Round, paint := canvas::stroke(color::rgb(255, 128, 0), 9.0)]
  LET tri AS canvas::DrawItem = canvas::Polygon[points := [canvas::Point[x := 600.0, y := 40.0], canvas::Point[x := 700.0, y := 40.0], canvas::Point[x := 650.0, y := 130.0]], paint := canvas::fill(color::rgba(0, 200, 255, 180))]
  ' The second polygon, and concave on purpose: it is the one item in this scene drawn
  ' from a non-zero edge base, and the crossing-count sign test only disagrees with the
  ' nearest-edge magnitude on a shape that is not convex -- so a wrong base here shows as
  ' a wrong FILL, not merely a shifted outline.
  LET arrow AS canvas::DrawItem = canvas::Polygon[points := [canvas::Point[x := 60.0, y := 400.0], canvas::Point[x := 160.0, y := 400.0], canvas::Point[x := 160.0, y := 360.0], canvas::Point[x := 230.0, y := 430.0], canvas::Point[x := 160.0, y := 500.0], canvas::Point[x := 160.0, y := 460.0], canvas::Point[x := 60.0, y := 460.0]], paint := canvas::fill(color::rgba(0, 180, 180, 200))]

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
  LET ground AS canvas::DrawItem = canvas::Rectangle[x := 20.0, y := 400.0, w := 360.0, h := 120.0, paint := canvas::fill(color::rgb(128, 128, 128))]
  LET blendMul AS canvas::DrawItem = canvas::Circle[x := 70.0, y := 460.0, radius := 14.0, paint := WITH canvas::fill(color::rgb(230, 120, 40)) { blend := canvas::BlendMode.Multiply }]
  LET blendScr AS canvas::DrawItem = canvas::Circle[x := 170.0, y := 460.0, radius := 14.0, paint := WITH canvas::fill(color::rgb(230, 120, 40)) { blend := canvas::BlendMode.Screen }]
  LET blendAdd AS canvas::DrawItem = canvas::Circle[x := 270.0, y := 460.0, radius := 14.0, paint := WITH canvas::fill(color::rgb(230, 120, 40)) { blend := canvas::BlendMode.Add }]
  LET blendStroke AS canvas::DrawItem = canvas::Circle[x := 350.0, y := 460.0, radius := 12.0, paint := WITH canvas::fillStroke(color::rgb(230, 120, 40), color::rgb(40, 120, 230), 8.0) { blend := canvas::BlendMode.Multiply }]
  LET clippedBox AS canvas::DrawItem = canvas::Rectangle[x := 420.0, y := 400.0, w := 300.0, h := 60.0, paint := WITH canvas::fill(color::rgb(255, 255, 255)) { clip := canvas::Bounds[x := 460.25, y := 400.0, w := 200.5, h := 60.0] }]

  ' plan-116-C: transformed items, so the shader's inverse-map path actually runs.
  ' A rotation exercises the gradient correction on a curved edge; the non-uniform
  ' scale is the case Phase 1 measured sqrt(|det M|) as 37/255 wrong on. Rotated TEXT
  ' is covered by rt_canvas_font's a_rotated_text_run_draws_rotated, which owns the
  ' font fixture -- this scene has no font.
  LET rotT AS canvas::Transform = canvas::Transform[a := 0.7071067811865476, b := 0.7071067811865476, c := 0.0 - 0.7071067811865476, d := 0.7071067811865476, tx := 120.0, ty := 560.0]
  LET rotBox AS canvas::DrawItem = canvas::Rectangle[x := 0.0 - 25.0, y := 0.0 - 25.0, w := 50.0, h := 50.0, paint := WITH canvas::fill(color::rgb(255, 200, 40)) { transform := rotT }]
  LET scaleT AS canvas::Transform = canvas::Transform[a := 2.0, b := 0.0, c := 0.0, d := 1.0, tx := 250.0, ty := 560.0]
  LET scaleDot AS canvas::DrawItem = canvas::Circle[x := 0.0, y := 0.0, radius := 18.0, paint := WITH canvas::fillStroke(color::rgb(90, 200, 255), color::rgb(255, 255, 255), 6.0) { transform := scaleT }]

  ' plan-116-D: the same line twice, butt and round, so the shader's cap arm actually
  ' runs on the GPU. `line` above is round-capped and pre-dates this letter; without a
  ' butt one the new branch would be emitted and never executed, and a backend whose
  ' butt arm is wrong would still match the oracle everywhere the scene looks.
  ' Thick and short, because the cap is a half-width feature: at 24 px wide the two
  ' styles differ over a visibly large region rather than one antialiased pixel.
  LET capButt AS canvas::DrawItem = canvas::Line[x1 := 120.0, y1 := 600.0, x2 := 240.0, y2 := 600.0, cap := canvas::CapStyle.Butt, paint := canvas::stroke(color::rgb(255, 240, 120), 24.0)]
  LET capRound AS canvas::DrawItem = canvas::Line[x1 := 320.0, y1 := 600.0, x2 := 440.0, y2 := 600.0, cap := canvas::CapStyle.Round, paint := canvas::stroke(color::rgb(255, 240, 120), 24.0)]
  ' And a ROUND-capped arc, for the same reason. `smile` above is butt-capped -- which
  ' is what every arc was before plan-116-D -- so without this the arc's cap-disc arm
  ' is compiled into both shaders and never taken. The sweep stops at 0.6*PI so both
  ' ends are visible rather than one running off the item's own band.
  LET capArc AS canvas::DrawItem = canvas::Arc[x := 620.0, y := 600.0, radius := 60.0, startAngle := 0.0, endAngle := 1.884955592153876, cap := canvas::CapStyle.Round, paint := canvas::stroke(color::rgb(120, 255, 200), 20.0)]

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
IMPORT color
SUB main()
  app::setMode(app::Mode.Canvas)

  ' 1. a group drawn at (0,0) -- the offset is present but zero
  LET atOrigin AS List OF canvas::DrawItem = [canvas::Rectangle[x := 20.0, y := 20.0, w := 80.0, h := 40.0, paint := canvas::fill(color::rgb(255, 0, 0))]]
  canvas::setGroup("atOrigin", atOrigin)

  ' 2. a group drawn at (37, 53) -- a non-zero offset, which a vertex-only
  '    implementation cannot get right for anything position-dependent
  LET moved AS List OF canvas::DrawItem = [canvas::Circle[x := 200.0, y := 60.0, radius := 30.0, paint := canvas::fillStroke(color::rgb(0, 160, 220), color::rgb(255, 255, 255), 5.0)]]
  canvas::setGroup("moved", moved)

  ' 3. a NESTED group: outer holds inner, so the offsets compose
  LET inner AS List OF canvas::DrawItem = [canvas::Rectangle[x := 0.0, y := 0.0, w := 50.0, h := 50.0, paint := canvas::fill(color::rgb(200, 200, 0))]]
  canvas::setGroup("inner", inner)
  LET outer AS List OF canvas::DrawItem = [canvas::Group[name := "inner", dx := 15.0, dy := 25.0]]
  canvas::setGroup("outer", outer)

  ' 4. a DIAMOND: one leaf referenced twice, at two offsets
  LET leaf AS List OF canvas::DrawItem = [canvas::Rectangle[x := 0.0, y := 0.0, w := 60.0, h := 40.0, paint := canvas::fill(color::rgb(120, 220, 60))]]
  canvas::setGroup("leaf", leaf)

  ' 5. a CLIPPED item inside a translated group -- the clip is evaluated in SURFACE
  '    space while the shape is evaluated in shape space (section 4.2)
  LET clipped AS List OF canvas::DrawItem = [canvas::Rectangle[x := 0.0, y := 0.0, w := 300.0, h := 60.0, paint := WITH canvas::fill(color::rgb(255, 255, 255)) { clip := canvas::Bounds[x := 40.25, y := 0.0, w := 200.5, h := 60.0] }]]
  canvas::setGroup("clipped", clipped)

  ' 6. a GRADIENT-filled item inside a translated group -- the ramp is sampled from
  '    the shape-space point, so a missing fragment offset shows as a shifted ramp
  LET stops AS List OF canvas::GradientStop = [canvas::GradientStop[offset := 0.0, color := color::rgb(255, 64, 32)], canvas::GradientStop[offset := 0.55, color := color::rgb(250, 230, 90)], canvas::GradientStop[offset := 1.0, color := color::rgb(32, 96, 255)]]
  LET ramp AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, startPoint := canvas::Point[x := 0.0, y := 0.0], endPoint := canvas::Point[x := 150.0, y := 60.0], stops := stops]
  LET grad AS List OF canvas::DrawItem = [canvas::Rectangle[x := 0.0, y := 0.0, w := 150.0, h := 60.0, paint := WITH canvas::fill(color::rgb(0, 0, 0)) { fillGradient := ramp }]]
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
IMPORT color
SUB main()
  app::setMode(app::Mode.Canvas)
  LET a AS canvas::DrawItem = canvas::Rectangle[x := 0.0, y := 0.0, w := 70.0, h := 45.0, paint := canvas::fill(color::rgb(120, 220, 60))]
  LET b AS canvas::DrawItem = canvas::Circle[x := 100.0, y := 22.0, radius := 18.0, paint := canvas::fillStroke(color::rgb(0, 170, 220), color::rgb(255, 255, 255), 4.0)]
  canvas::setGroup("pair", [a, b])
  canvas::present([canvas::Group[name := "pair", dx := 120.0, dy := 90.0], canvas::Group[name := "pair", dx := 430.0, dy := 300.0]])
END SUB
"#;

/// The same picture with no group in it — the two shapes written out twice, at the
/// coordinates the group offsets would have put them.
const DIAMOND_FLAT: &str = r#"IMPORT app
IMPORT canvas
IMPORT color
SUB main()
  app::setMode(app::Mode.Canvas)
  LET a1 AS canvas::DrawItem = canvas::Rectangle[x := 120.0, y := 90.0, w := 70.0, h := 45.0, paint := canvas::fill(color::rgb(120, 220, 60))]
  LET b1 AS canvas::DrawItem = canvas::Circle[x := 220.0, y := 112.0, radius := 18.0, paint := canvas::fillStroke(color::rgb(0, 170, 220), color::rgb(255, 255, 255), 4.0)]
  LET a2 AS canvas::DrawItem = canvas::Rectangle[x := 430.0, y := 300.0, w := 70.0, h := 45.0, paint := canvas::fill(color::rgb(120, 220, 60))]
  LET b2 AS canvas::DrawItem = canvas::Circle[x := 530.0, y := 322.0, radius := 18.0, paint := canvas::fillStroke(color::rgb(0, 170, 220), color::rgb(255, 255, 255), 4.0)]
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

/// ONE polygon with more edges than Metal's whole frame edge region holds
/// (`METAL_MAX_FRAME_EDGES` = 262,144): 262,145 edges.
///
/// This scene was a 300-edge ring until bug-686, declined by a per-polygon cap of 256
/// that bug deleted — 300 edges now draw on Metal
/// (`polygons_past_two_hundred_fifty_six_edges_draw_on_metal`). No polygon size is
/// refused any more; a polygon that alone passes the frame's region still is, and that
/// is the decline this proves. 6 px across, so the software oracle stays quick.
const TOO_MANY_EDGES: &str = r#"IMPORT app
IMPORT canvas
IMPORT color
IMPORT collections
IMPORT math

SUB main()
  app::setMode(app::Mode.Canvas)
  MUT points AS List OF canvas::Point = []
  MUT i AS Integer = 0
  WHILE i < 262145
    LET a AS Float = toFloat(i) * 6.283185307179586 / 262145.0
    points = collections::append(points, canvas::Point[x := 450.0 + 3.0 * math::cos(a), y := 320.0 + 3.0 * math::sin(a)])
    i = i + 1
  END WHILE
  LET ring AS canvas::DrawItem = canvas::Polygon[points := points, paint := canvas::fill(color::rgb(0, 200, 255))]
  LET box AS canvas::DrawItem = canvas::Rectangle[x := 10.0, y := 10.0, w := 50.0, h := 50.0, paint := canvas::fill(color::rgb(0, 255, 0))]
  canvas::present([box, ring])
END SUB
"#;

/// Many polygons that individually fit but together overflow the frame's edge region.
///
/// plan-116-A declined this scene: 40,000 edges was past the 16,384-edge frame region
/// that letter introduced. bug-686 raised the region to 262,144 edges, and the scene is
/// now a Metal acceptance case (`forty_thousand_polygon_edges_draw_on_metal`). The
/// decline it used to prove is `PAST_THE_FRAME_EDGE_REGION`'s job below.
const TOO_MANY_FRAME_EDGES: &str = r#"IMPORT app
IMPORT canvas
IMPORT color
IMPORT collections
IMPORT math

SUB main()
  app::setMode(app::Mode.Canvas)
  MUT scene AS List OF canvas::DrawItem = [canvas::Rectangle[x := 10.0, y := 10.0, w := 50.0, h := 50.0, paint := canvas::fill(color::rgb(0, 255, 0))]]
  MUT ring AS Integer = 0
  WHILE ring < 200
    MUT points AS List OF canvas::Point = []
    MUT i AS Integer = 0
    WHILE i < 200
      LET a AS Float = toFloat(i) * 6.283185307179586 / 200.0
      points = collections::append(points, canvas::Point[x := 450.0 + toFloat(ring) + 100.0 * math::cos(a), y := 320.0 + 100.0 * math::sin(a)])
      i = i + 1
    END WHILE
    LET poly AS canvas::DrawItem = canvas::Polygon[points := points, paint := canvas::fill(color::rgba(0, 200, 255, 60))]
    scene = collections::append(scene, poly)
    ring = ring + 1
  END WHILE
  canvas::present(scene)
END SUB
"#;

/// Polygons whose edges together pass Metal's frame edge region
/// (`METAL_MAX_FRAME_EDGES` = 262,144 since bug-686).
///
/// 1,025 rings of 256 edges is 262,400 edges — 256 past the region, and no ring past
/// any per-polygon limit, so only the *sum* can decline it. The rings are 4 px across so
/// the software oracle, which walks every edge for every pixel of a polygon's bounds,
/// draws the frame in seconds rather than minutes. The rectangle keeps the frame from
/// being blank.
const PAST_THE_FRAME_EDGE_REGION: &str = r#"IMPORT app
IMPORT canvas
IMPORT color
IMPORT collections
IMPORT math

SUB main()
  app::setMode(app::Mode.Canvas)
  MUT scene AS List OF canvas::DrawItem = [canvas::Rectangle[x := 10.0, y := 10.0, w := 50.0, h := 50.0, paint := canvas::fill(color::rgb(0, 255, 0))]]
  MUT ring AS Integer = 0
  WHILE ring < 1025
    LET cx AS Float = 20.0 + toFloat((ring MOD 70) * 12)
    LET cy AS Float = 80.0 + toFloat((ring / 70) * 12)
    MUT points AS List OF canvas::Point = []
    MUT i AS Integer = 0
    WHILE i < 256
      LET a AS Float = toFloat(i) * 6.283185307179586 / 256.0
      points = collections::append(points, canvas::Point[x := cx + 4.0 * math::cos(a), y := cy + 4.0 * math::sin(a)])
      i = i + 1
    END WHILE
    LET poly AS canvas::DrawItem = canvas::Polygon[points := points, paint := canvas::fill(color::rgba(0, 200, 255, 160))]
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
    let binary = common::build_app_debug(&project, name);
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
///
/// The scene is one polygon past the frame's edge region (`TOO_MANY_EDGES`); since
/// bug-686 there is no per-polygon cap for a smaller one to trip.
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
    // bug-686: a polygon this small draws on Metal byte-identically to software (a
    // 262,144-edge twin measured 0 differing bytes), so pixel equality alone would pass
    // whether or not the frame was declined. The decline is asserted directly.
    assert!(
        stats.contains("gpuFrames=0"),
        "a 262,145-edge polygon is past METAL_MAX_FRAME_EDGES, but Metal drew the frame: \
         {stats}"
    );
    if let Err(diff) = compare_exact(&gpu, &software) {
        panic!(
            "a 262,145-edge polygon does not fit Metal's frame edge region, so the \
             renderer must decline the whole scene and let the software oracle draw it \
             — the two frames must be byte-identical, but {diff}"
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
/// at the region's end would still report `gpuSelected=TRUE` and look healthy.
///
/// bug-686 raised the region from 16,384 edges to 262,144, so the 40,000-edge scene this
/// test used to decline now draws on Metal (`forty_thousand_polygon_edges_draw_on_metal`)
/// and this one is past the new region instead.
#[test]
fn a_frame_whose_polygons_together_overflow_the_edge_region_falls_back() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let program = build("canvas_metal_frame_edges", PAST_THE_FRAME_EDGE_REGION);
    let (software, _) = render(&program, false, "sw");
    let (gpu, stats) = render(&program, true, "gpu");
    if !metal_built(&stats) {
        return;
    }
    assert!(
        software.pixels.iter().any(|&b| b != 0),
        "the software render drew nothing, so the comparison would be vacuous",
    );
    // bug-686: rings this small can draw on Metal byte-identically to software, so pixel
    // equality alone would pass whether or not the frame was declined.
    assert!(
        stats.contains("gpuFrames=0"),
        "262,400 edges is past METAL_MAX_FRAME_EDGES, but Metal drew the frame: {stats}"
    );
    if let Err(diff) = compare_exact(&gpu, &software) {
        panic!(
            "1,025 rings of 256 edges is 262,400 edges, past METAL_MAX_FRAME_EDGES — the \
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

/// A line of distinct glyphs, drawn with a real system font. The fixture font the
/// other text tests use gives every character one glyph shape, which is exactly
/// what hid the bug below.
const TEXT_LINE: &str = r#"IMPORT app
IMPORT canvas
IMPORT color

SUB main()
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("/System/Library/Fonts/Supplemental/Arial.ttf")
  LET bg AS canvas::DrawItem = canvas::Rectangle[x := 0.0, y := 0.0, w := 900.0, h := 640.0, paint := canvas::fill(color::rgb(20, 24, 30))]
  LET a AS canvas::DrawItem = canvas::Text[x := 20.0, y := 40.0, text := "LEVEL 01", font := face, size := 22.0, paint := canvas::fill(color::rgb(255, 255, 255))]
  LET b AS canvas::DrawItem = canvas::Text[x := 300.0, y := 40.0, text := "BUGS quick", font := face, size := 22.0, paint := canvas::fill(color::rgb(255, 214, 90))]
  canvas::present([bg, a, b])
END SUB
"#;

/// Each glyph of a text run samples its OWN bitmap on the GPU.
///
/// plan-116-H made a text item one instanced draw, but left each glyph's bitmap as a
/// per-glyph `setFragmentBytes:` at index 2 set in the publish loop — so by the time
/// the draw ran, only the run's LAST glyph was bound, and every glyph drew that
/// glyph's pixels at its own width: "LEVEL 01" came out as hatching and a clean "1".
/// It passed every existing test because those compare whole frames within a 2%
/// pixel budget, and a line of 22 px text is far less than 2% of 900x640.
///
/// So this compares only the rows the text occupies, where garbled glyphs are most of
/// the pixels.
#[test]
fn every_glyph_of_a_text_run_draws_its_own_bitmap() {
    if !cfg!(target_os = "macos")
        || !std::path::Path::new("/System/Library/Fonts/Supplemental/Arial.ttf").exists()
    {
        return;
    }
    let program = build("canvas_metal_text_line", TEXT_LINE);
    let (software, _) = render(&program, false, "sw");
    let (gpu, stats) = render(&program, true, "gpu");
    if !metal_built(&stats) {
        return; // no Metal device on this host (§metal_built)
    }
    assert!(
        !stats.contains("gpuFrames=0"),
        "MFB_CANVAS_GPU=1 did not select the Metal renderer: {stats}"
    );
    let band = |frame: &Frame| {
        let rows = 60usize;
        let stride = WIDTH as usize * 4;
        Frame::from_rgba(WIDTH, rows as u32, frame.pixels[..rows * stride].to_vec())
    };
    if let Err(diff) =
        compare_within_tolerance(&band(&gpu), &band(&software), Tolerance::GPU_DEFAULT)
    {
        panic!(
            "the Metal backend draws a text run differently from the software oracle: \
             {diff}\nA glyph drawn with ANOTHER glyph's bitmap (hatching, or one letter \
             repeated) is the per-glyph payload being overwritten before the draw."
        );
    }
}

/// Every `canvas::Picture` path, in one scene (bug-484).
///
/// Pictures ride the frame buffer's glyph region, one packed RGBA texel per word, and
/// each picture's block names its own slice — so the scene carries **several** pictures
/// and a text run between them: with one picture its slice starts at zero, and a base
/// that was never written would pass. The text interleaves glyph slices with picture
/// slices in the same region, which is what proves the two share one cursor.
///
/// Beyond that it covers each thing the picture inherits from the rectangle: nearest
/// scaling of a 2x2 image, the fill tint, a translucent texel, a 90-degree transform, a
/// clip, a non-Normal blend that also strokes (the split into two instances), a group
/// offset, and a destination on fractional coordinates for the antialiased edge.
const PICTURES: &str = r#"IMPORT app
IMPORT canvas
IMPORT color

SUB main()
  app::setMode(app::Mode.Canvas)
  LET quad AS List OF Byte = [toByte(255), toByte(0), toByte(0), toByte(255), toByte(0), toByte(255), toByte(0), toByte(255), toByte(0), toByte(0), toByte(255), toByte(255), toByte(255), toByte(255), toByte(255), toByte(255)]
  LET strip AS List OF Byte = [toByte(255), toByte(255), toByte(0), toByte(255), toByte(0), toByte(0), toByte(255), toByte(128), toByte(0), toByte(255), toByte(255), toByte(255)]
  RES img AS canvas::Image = canvas::createImage(2, 2, quad)
  RES thin AS canvas::Image = canvas::createImage(3, 1, strip)
  RES grouped AS canvas::Image = canvas::createImage(2, 2, quad)
  RES face AS canvas::Font = canvas::loadFont("/System/Library/Fonts/Supplemental/Arial.ttf")
  LET white AS canvas::Paint = canvas::fill(color::rgb(255, 255, 255))
  LET ground AS canvas::DrawItem = canvas::Rectangle[x := 0.0, y := 0.0, w := 900.0, h := 260.0, paint := canvas::fill(color::rgb(96, 96, 96))]
  LET plain AS canvas::DrawItem = canvas::Picture[x := 10.0, y := 10.0, w := 80.0, h := 80.0, image := img, paint := white]
  LET label AS canvas::DrawItem = canvas::Text[x := 10.0, y := 240.0, text := "Pictures 01", font := face, size := 22.0, paint := canvas::fill(color::rgb(255, 255, 255))]
  LET tinted AS canvas::DrawItem = canvas::Picture[x := 100.0, y := 10.0, w := 80.0, h := 80.0, image := img, paint := canvas::fill(color::rgb(255, 128, 0))]
  LET translucent AS canvas::DrawItem = canvas::Picture[x := 200.0, y := 10.0, w := 90.0, h := 30.0, image := thin, paint := white]
  LET t AS canvas::Transform = canvas::Transform[a := 0.0, b := 1.0, c := 0.0 - 1.0, d := 0.0, tx := 400.0, ty := 10.0]
  LET rotated AS canvas::DrawItem = canvas::Picture[x := 0.0, y := 0.0, w := 80.0, h := 60.0, image := img, paint := WITH white { transform := t }]
  LET clipped AS canvas::DrawItem = canvas::Picture[x := 420.0, y := 10.0, w := 80.0, h := 80.0, image := img, paint := WITH white { clip := canvas::Bounds[x := 420.0, y := 10.0, w := 40.0, h := 80.0] }]
  LET blended AS canvas::DrawItem = canvas::Picture[x := 520.0, y := 10.0, w := 60.0, h := 60.0, image := img, paint := WITH canvas::fillStroke(color::rgb(255, 255, 255), color::rgb(40, 120, 230), 6.0) { blend := canvas::BlendMode.Multiply }]
  LET fractional AS canvas::DrawItem = canvas::Picture[x := 700.5, y := 10.25, w := 50.5, h := 33.3, image := thin, paint := white]
  LET inner AS canvas::DrawItem = canvas::Picture[x := 0.0, y := 0.0, w := 40.0, h := 40.0, image := grouped, paint := white]
  canvas::setGroup("pics", [inner])
  LET node AS canvas::DrawItem = canvas::Group[name := "pics", dx := 600.0, dy := 120.0]
  canvas::present([ground, plain, label, tinted, translucent, rotated, clipped, blended, fractional, node])
END SUB
"#;

/// The Metal backend draws every picture path the same as the software oracle, and
/// the frame is proven to have been drawn by Metal (bug-484 Phase 3).
///
/// Until Phase 3 `__canvas_metalRenderable` declined any scene with a picture, so this
/// fails on `gpuFrames=0` rather than on pixels — the honesty gate working, not the
/// backend drawing. Compared on the band the pictures and the text occupy, for the
/// reason the text-run test crops: they are far less than the whole frame's budget.
#[test]
fn every_picture_path_matches_the_software_oracle() {
    if !cfg!(target_os = "macos")
        || !std::path::Path::new("/System/Library/Fonts/Supplemental/Arial.ttf").exists()
    {
        return;
    }
    let program = build("canvas_metal_pictures", PICTURES);
    let (software, _) = render(&program, false, "sw");
    let (gpu, stats) = render(&program, true, "gpu");
    if !metal_built(&stats) {
        return; // no Metal device on this host (§metal_built)
    }
    assert!(
        !stats.contains("gpuFrames=0"),
        "MFB_CANVAS_GPU=1 did not draw the picture scene on Metal: {stats}"
    );
    let band = |frame: &Frame| {
        let rows = 260usize;
        let stride = WIDTH as usize * 4;
        Frame::from_rgba(WIDTH, rows as u32, frame.pixels[..rows * stride].to_vec())
    };
    if let Err(diff) =
        compare_within_tolerance(&band(&gpu), &band(&software), Tolerance::GPU_DEFAULT)
    {
        panic!(
            "the Metal backend draws a picture differently from the software oracle: \
             {diff}\nA picture showing ANOTHER picture's or a glyph's texels is a slice \
             base that was not written or a cursor the two kinds do not share; a whole \
             picture off by one texel column is the nearest-sampling arithmetic."
        );
    }
}

/// A text run containing a space, followed by items whose draw state differs: a
/// blended circle and a translated group (bug-484, found by the picture scene).
///
/// A space has an empty bitmap and draws nothing, but `__canvas_blockInstances` counts
/// it as one instance like every glyph. The Metal emitter used to skip publishing its
/// block, so every later draw entry named the block one position on: the circle drew
/// under the next item's pipeline and the group lost its offset. `TEXT_LINE` above has
/// spaces too, but nothing after its runs has a draw state of its own, so the shift was
/// invisible there.
const SPACE_THEN_SHAPES: &str = r#"IMPORT app
IMPORT canvas
IMPORT color

SUB main()
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("/System/Library/Fonts/Supplemental/Arial.ttf")
  LET ground AS canvas::DrawItem = canvas::Rectangle[x := 0.0, y := 0.0, w := 900.0, h := 200.0, paint := canvas::fill(color::rgb(128, 128, 128))]
  LET words AS canvas::DrawItem = canvas::Text[x := 20.0, y := 40.0, text := "A B C", font := face, size := 22.0, paint := canvas::fill(color::rgb(255, 255, 255))]
  LET mul AS canvas::DrawItem = canvas::Circle[x := 300.0, y := 100.0, radius := 40.0, paint := WITH canvas::fill(color::rgb(230, 120, 40)) { blend := canvas::BlendMode.Multiply }]
  LET plain AS canvas::DrawItem = canvas::Circle[x := 450.0, y := 100.0, radius := 40.0, paint := canvas::fill(color::rgb(40, 200, 90))]
  LET dot AS canvas::DrawItem = canvas::Rectangle[x := 0.0, y := 0.0, w := 30.0, h := 30.0, paint := canvas::fill(color::rgb(20, 60, 230))]
  canvas::setGroup("g", [dot])
  canvas::present([ground, words, mul, plain, canvas::Group[name := "g", dx := 600.0, dy := 80.0]])
END SUB
"#;

#[test]
fn a_space_in_a_text_run_does_not_shift_the_draws_after_it() {
    if !cfg!(target_os = "macos")
        || !std::path::Path::new("/System/Library/Fonts/Supplemental/Arial.ttf").exists()
    {
        return;
    }
    let program = build("canvas_metal_space_shift", SPACE_THEN_SHAPES);
    let (software, _) = render(&program, false, "sw");
    let (gpu, stats) = render(&program, true, "gpu");
    if !metal_built(&stats) {
        return; // no Metal device on this host (§metal_built)
    }
    assert!(
        !stats.contains("gpuFrames=0"),
        "MFB_CANVAS_GPU=1 did not draw the scene on Metal: {stats}"
    );
    let band = |frame: &Frame| {
        let rows = 200usize;
        let stride = WIDTH as usize * 4;
        Frame::from_rgba(WIDTH, rows as u32, frame.pixels[..rows * stride].to_vec())
    };
    if let Err(diff) =
        compare_within_tolerance(&band(&gpu), &band(&software), Tolerance::GPU_DEFAULT)
    {
        panic!(
            "items after a text run containing a space draw differently on Metal: {diff}\n\
             A blank glyph must still publish its item block, or every later draw entry \
             names its neighbour's."
        );
    }
}

// ---------------------------------------------------------------------------------------
// bug-686: ordinary scenes that used to fall back to software must draw on Metal.
//
// Every test below asserts two things, in this order: that Metal drew the frame
// (`gpuFrames` is not 0 — otherwise the pixel comparison would be the software renderer
// against itself, and pass for no reason), and that what it drew agrees with the
// software oracle within `Tolerance::GPU_DEFAULT`.
// ---------------------------------------------------------------------------------------

/// Build `source`, render it on both backends, and require that Metal drew it and
/// agrees with software. `what` names the scene in the failure message.
fn assert_metal_draws_like_software(name: &str, source: &str, what: &str) {
    if !cfg!(target_os = "macos") {
        return;
    }
    let program = build(name, source);
    let (software, _) = render(&program, false, "sw");
    let (gpu, stats) = render(&program, true, "gpu");
    if !metal_built(&stats) {
        return; // no Metal device on this host (§metal_built)
    }
    assert!(
        software.pixels.chunks(4).any(|p| p != [0, 0, 0, 255]),
        "the software render of {what} drew nothing, so the comparison would be vacuous",
    );
    assert!(
        !stats.contains("gpuFrames=0"),
        "{what} fell back to the software renderer instead of drawing on Metal (bug-686): \
         {stats}"
    );
    if let Err(diff) = compare_within_tolerance(&gpu, &software, Tolerance::GPU_DEFAULT) {
        panic!("Metal draws {what} differently from the software oracle: {diff}");
    }
}

/// 5,000 small rectangles: past the old 4,096-quad frame cap (`CANVAS_MAX_FRAME_ITEMS`).
const FIVE_THOUSAND_QUADS: &str = r#"IMPORT app
IMPORT canvas
IMPORT color
IMPORT collections

SUB main()
  app::setMode(app::Mode.Canvas)
  MUT scene AS List OF canvas::DrawItem = []
  MUT k AS Integer = 0
  WHILE k < 5000
    LET r AS canvas::DrawItem = canvas::Rectangle[x := toFloat((k MOD 100) * 9), y := toFloat((k / 100) * 12), w := 7.0, h := 9.0, paint := canvas::fill(color::rgb((k * 7) MOD 256, (k * 13) MOD 256, 200))]
    scene = collections::append(scene, r)
    k = k + 1
  END WHILE
  canvas::present(scene)
END SUB
"#;

#[test]
fn a_scene_past_four_thousand_quads_draws_on_metal() {
    assert_metal_draws_like_software(
        "canvas_metal_5000_quads",
        FIVE_THOUSAND_QUADS,
        "a 5,000-rectangle scene",
    );
}

/// The frame-edge scene above (200 rings × 200 edges = 40,000 edges), now expected to draw.
#[test]
fn forty_thousand_polygon_edges_draw_on_metal() {
    assert_metal_draws_like_software(
        "canvas_metal_40k_edges",
        TOO_MANY_FRAME_EDGES,
        "200 translucent rings of 200 edges (40,000 edges)",
    );
}

/// 2,100 gradient-filled rectangles: 4,200 stops, past the old 4,096-stop frame cap.
const MANY_GRADIENTS: &str = r#"IMPORT app
IMPORT canvas
IMPORT color
IMPORT collections

SUB main()
  app::setMode(app::Mode.Canvas)
  LET stops AS List OF canvas::GradientStop = [canvas::GradientStop[offset := 0.0, color := color::rgb(255, 64, 32)], canvas::GradientStop[offset := 1.0, color := color::rgb(32, 96, 255)]]
  MUT scene AS List OF canvas::DrawItem = []
  MUT k AS Integer = 0
  WHILE k < 2100
    LET x AS Float = toFloat((k MOD 60) * 15)
    LET y AS Float = toFloat((k / 60) * 18)
    LET ramp AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, startPoint := canvas::Point[x := x, y := y], endPoint := canvas::Point[x := x + 12.0, y := y + 15.0], stops := stops]
    LET r AS canvas::DrawItem = canvas::Rectangle[x := x, y := y, w := 12.0, h := 15.0, paint := WITH canvas::fill(color::rgb(0, 0, 0)) { fillGradient := ramp }]
    scene = collections::append(scene, r)
    k = k + 1
  END WHILE
  canvas::present(scene)
END SUB
"#;

#[test]
fn a_scene_past_four_thousand_gradient_stops_draws_on_metal() {
    assert_metal_draws_like_software(
        "canvas_metal_gradient_stops",
        MANY_GRADIENTS,
        "2,100 gradient-filled rectangles (4,200 stops)",
    );
}

/// The `pattern` helper the picture scenes share: a `w × h` RGBA image whose texels vary
/// by position and `seed`, so a picture drawn from the wrong texels shows.
const PATTERN: &str = r#"FUNC pattern(w AS Integer, h AS Integer, seed AS Integer) AS List OF Byte
  MUT px AS List OF Byte = []
  MUT i AS Integer = 0
  WHILE i < w * h
    px = collections::append(px, toByte(((i MOD w) * 7 + seed) MOD 256))
    px = collections::append(px, toByte(((i / w) * 5 + seed * 3) MOD 256))
    px = collections::append(px, toByte((((i MOD w) + (i / w)) * 3) MOD 256))
    px = collections::append(px, toByte(255))
    i = i + 1
  END WHILE
  RETURN px
END FUNC
"#;

/// A tilemap: 2,000 32×32 tiles drawn from TWO images. Counted per item the texels are
/// 2,048,000, past the old 1M-texel frame cap; there are only 2,048 distinct texels.
fn tilemap_scene() -> String {
    format!(
        "IMPORT app\nIMPORT canvas\nIMPORT color\nIMPORT collections\n\n{PATTERN}\n\
         SUB main()\n  \
         app::setMode(app::Mode.Canvas)\n  \
         RES a AS canvas::Image = canvas::createImage(32, 32, pattern(32, 32, 1))\n  \
         RES b AS canvas::Image = canvas::createImage(32, 32, pattern(32, 32, 90))\n  \
         LET white AS canvas::Paint = canvas::fill(color::rgb(255, 255, 255))\n  \
         MUT scene AS List OF canvas::DrawItem = []\n  \
         MUT k AS Integer = 0\n  \
         WHILE k < 2000\n    \
         LET x AS Float = toFloat((k * 11) MOD 870)\n    \
         LET y AS Float = toFloat((k * 7) MOD 610)\n    \
         IF k MOD 2 = 0 THEN\n      \
         LET p AS canvas::DrawItem = canvas::Picture[x := x, y := y, w := 32.0, h := 32.0, image := a, paint := white]\n      \
         scene = collections::append(scene, p)\n    \
         ELSE\n      \
         LET p AS canvas::DrawItem = canvas::Picture[x := x, y := y, w := 32.0, h := 32.0, image := b, paint := white]\n      \
         scene = collections::append(scene, p)\n    \
         END IF\n    \
         k = k + 1\n  \
         END WHILE\n  \
         canvas::present(scene)\n\
         END SUB\n"
    )
}

#[test]
fn a_tilemap_of_two_thousand_tiles_draws_on_metal() {
    assert_metal_draws_like_software(
        "canvas_metal_tilemap",
        &tilemap_scene(),
        "2,000 32-pixel tiles from two images",
    );
}

/// A 1920×1080 background scaled to the surface, plus one tile over it: 2,074,624
/// texels, past the old 1M-texel frame cap even counted per distinct image.
fn background_scene() -> String {
    format!(
        "IMPORT app\nIMPORT canvas\nIMPORT color\nIMPORT collections\n\n{PATTERN}\n\
         SUB main()\n  \
         app::setMode(app::Mode.Canvas)\n  \
         RES bg AS canvas::Image = canvas::createImage(1920, 1080, pattern(1920, 1080, 40))\n  \
         RES tile AS canvas::Image = canvas::createImage(32, 32, pattern(32, 32, 1))\n  \
         LET white AS canvas::Paint = canvas::fill(color::rgb(255, 255, 255))\n  \
         LET back AS canvas::DrawItem = canvas::Picture[x := 0.0, y := 0.0, w := 900.0, h := 640.0, image := bg, paint := white]\n  \
         LET front AS canvas::DrawItem = canvas::Picture[x := 100.0, y := 100.0, w := 64.0, h := 64.0, image := tile, paint := white]\n  \
         canvas::present([back, front])\n\
         END SUB\n"
    )
}

#[test]
fn a_full_hd_background_draws_on_metal() {
    assert_metal_draws_like_software(
        "canvas_metal_background",
        &background_scene(),
        "a 1920x1080 background picture under a tile",
    );
}

/// Polygons past the old 256-edge per-polygon cap: a filled ring of 1,000 points, one of
/// 4,000, a stroked ring of 600, a rotated ring of 500, and a self-intersecting star
/// polygon {301/97} (even-odd fill), all on one surface.
const LARGE_POLYGONS: &str = r#"IMPORT app
IMPORT canvas
IMPORT color
IMPORT collections
IMPORT math

FUNC ring(n AS Integer, cx AS Float, cy AS Float, r AS Float, wobble AS Float) AS List OF canvas::Point
  MUT points AS List OF canvas::Point = []
  MUT i AS Integer = 0
  WHILE i < n
    LET a AS Float = toFloat(i) * 6.283185307179586 / toFloat(n)
    LET rr AS Float = r + wobble * math::sin(a * 7.0) + wobble * 0.3 * math::sin(a * 53.0)
    points = collections::append(points, canvas::Point[x := cx + rr * math::cos(a), y := cy + rr * math::sin(a)])
    i = i + 1
  END WHILE
  RETURN points
END FUNC

FUNC star(n AS Integer, k AS Integer, cx AS Float, cy AS Float, r AS Float) AS List OF canvas::Point
  MUT points AS List OF canvas::Point = []
  MUT i AS Integer = 0
  WHILE i < n
    LET a AS Float = toFloat((i * k) MOD n) * 6.283185307179586 / toFloat(n)
    points = collections::append(points, canvas::Point[x := cx + r * math::cos(a), y := cy + r * math::sin(a)])
    i = i + 1
  END WHILE
  RETURN points
END FUNC

SUB main()
  app::setMode(app::Mode.Canvas)
  LET a AS canvas::DrawItem = canvas::Polygon[points := ring(1000, 120.0, 130.0, 90.0, 12.0), paint := canvas::fill(color::rgb(0, 200, 255))]
  LET b AS canvas::DrawItem = canvas::Polygon[points := ring(4000, 340.0, 130.0, 90.0, 12.0), paint := canvas::fill(color::rgba(255, 160, 0, 200))]
  LET c AS canvas::DrawItem = canvas::Polygon[points := ring(600, 560.0, 130.0, 90.0, 12.0), paint := canvas::stroke(color::rgb(120, 255, 90), 4.0)]
  LET t AS canvas::Transform = canvas::Transform[a := 0.8, b := 0.6, c := 0.0 - 0.6, d := 0.8, tx := 780.0, ty := 130.0]
  LET d AS canvas::DrawItem = canvas::Polygon[points := ring(500, 0.0, 0.0, 90.0, 12.0), paint := WITH canvas::fill(color::rgb(230, 80, 200)) { transform := t }]
  LET e AS canvas::DrawItem = canvas::Polygon[points := star(301, 97, 450.0, 440.0, 180.0), paint := canvas::fill(color::rgb(250, 250, 120))]
  canvas::present([a, b, c, d, e])
END SUB
"#;

/// A scene installed with `canvas::presentLayers`: two layers, one item each.
const LAYERED: &str = r#"IMPORT app
IMPORT canvas
IMPORT color

SUB main()
  app::setMode(app::Mode.Canvas)
  LET a AS canvas::DrawItem = canvas::Rectangle[x := 50.0, y := 50.0, w := 200.0, h := 100.0, paint := canvas::fill(color::rgb(255, 0, 0))]
  LET b AS canvas::DrawItem = canvas::Circle[x := 400.0, y := 300.0, radius := 80.0, paint := canvas::fill(color::rgb(0, 200, 255))]
  LET c AS canvas::DrawItem = canvas::Rectangle[x := 360.0, y := 280.0, w := 120.0, h := 40.0, paint := canvas::fill(color::rgba(255, 255, 0, 160))]
  canvas::presentLayers([canvas::DrawLayer[items := [a]], canvas::DrawLayer[items := [b, c]]])
END SUB
"#;

/// A layered scene draws its layers on Metal (found fixing bug-686).
///
/// The GPU draw list (`__canvas_sceneDraws`) walked `canvas::installedItems()` only, so a
/// scene installed with `presentLayers` — whose items all live in its layers — published
/// no blocks at all: Metal drew an empty frame and reported `gpuFrames=1`, the honesty
/// gate's worst case (measured: 0 lit pixels against software's 40,332).
#[test]
fn a_layered_scene_draws_its_layers_on_metal() {
    assert_metal_draws_like_software("canvas_metal_layers", LAYERED, "a two-layer scene");
}

#[test]
fn polygons_past_two_hundred_fifty_six_edges_draw_on_metal() {
    assert_metal_draws_like_software(
        "canvas_metal_large_polygons",
        LARGE_POLYGONS,
        "filled, stroked, transformed and self-intersecting polygons of 300-4,000 edges",
    );
}
