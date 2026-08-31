//! The Metal backend renders the same picture as the software oracle (plan-98-E).
//!
//! The oracle is the software rasteriser, not a stored image: plan-98-A invariant 7
//! makes the software path the reference every GPU backend is measured against, so
//! these tests render the *same program twice* — once with `MFB_CANVAS_METAL=1` and
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
use std::path::{Path, PathBuf};
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
  app::setMode(Mode.Canvas)
  LET red AS DrawItem = Rectangle[x := 10.0, y := 10.0, w := 50.0, h := 50.0, paint := canvas::fill(canvas::rgb(255, 0, 0))]
  LET grey AS DrawItem = Rectangle[x := 100.0, y := 10.0, w := 60.0, h := 60.0, paint := canvas::fill(canvas::rgb(128, 128, 128))]
  LET half AS DrawItem = Rectangle[x := 120.0, y := 30.0, w := 60.0, h := 60.0, paint := canvas::fill(canvas::rgba(0, 0, 255, 128))]
  LET faint AS DrawItem = Rectangle[x := 200.0, y := 10.0, w := 80.0, h := 40.0, paint := canvas::fill(canvas::rgba(255, 255, 255, 40))]
  LET wide AS DrawItem = Rectangle[x := 300.0, y := 100.0, w := 500.0, h := 300.0, paint := canvas::fill(canvas::rgb(17, 200, 90))]
  LET over AS DrawItem = Rectangle[x := 400.0, y := 150.0, w := 200.0, h := 100.0, paint := canvas::fill(canvas::rgba(255, 200, 0, 200))]
  canvas::present([red, grey, half, faint, wide, over])
END SUB
"#;

/// Every primitive the SDF shader draws, in one scene.
///
/// The smiley from `plan-98-api.md` — the scene that shaped the API — plus a rounded
/// rect with both a fill and a stroke, a thick line, and a translucent polygon. That
/// covers each arm of the distance dispatch, both paint channels, the corner-radius
/// term, an arc's sweep test, and the polygon edge buffer, which between them are
/// every path the fragment shader has.
const PRIMITIVES: &str = r#"IMPORT app
IMPORT canvas

SUB main()
  app::setMode(Mode.Canvas)

  LET yellow AS Color = canvas::rgb(255, 255, 0)
  LET green AS Color = canvas::rgb(0, 160, 0)

  LET face AS DrawItem = Circle[x := 450.0, y := 320.0, radius := 150.0, paint := canvas::fill(yellow)]
  LET eyeL AS DrawItem = Circle[x := 400.0, y := 280.0, radius := 22.0, paint := canvas::fill(green)]
  LET eyeR AS DrawItem = Circle[x := 500.0, y := 280.0, radius := 22.0, paint := canvas::fill(green)]
  LET smile AS DrawItem = Arc[x := 450.0, y := 335.0, radius := 90.0, startAngle := 0.0, endAngle := 3.14159, paint := canvas::stroke(green, 14.0)]
  LET box AS DrawItem = Rectangle[x := 10.0, y := 10.0, w := 50.0, h := 50.0, paint := canvas::fill(canvas::rgb(255, 0, 0))]
  LET rounded AS DrawItem = RoundedRect[x := 100.0, y := 10.0, w := 90.0, h := 60.0, cornerRadius := 18.0, paint := canvas::fillStroke(canvas::rgb(0, 0, 255), canvas::rgb(255, 255, 255), 4.0)]
  LET line AS DrawItem = Line[x1 := 220.0, y1 := 20.0, x2 := 380.0, y2 := 90.0, paint := canvas::stroke(canvas::rgb(255, 128, 0), 9.0)]
  LET tri AS DrawItem = Polygon[points := [Point[x := 600.0, y := 40.0], Point[x := 700.0, y := 40.0], Point[x := 650.0, y := 130.0]], paint := canvas::fill(canvas::rgba(0, 200, 255, 180))]

  canvas::present([box, rounded, line, tri, face, eyeL, eyeR, smile])
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
  app::setMode(Mode.Canvas)
  MUT points AS List OF Point = []
  MUT i AS Integer = 0
  WHILE i < 300
    LET a AS Float = toFloat(i) * 6.283185307179586 / 300.0
    points = collections::append(points, Point[x := 450.0 + 200.0 * math::cos(a), y := 320.0 + 200.0 * math::sin(a)])
    i = i + 1
  END WHILE
  LET ring AS DrawItem = Polygon[points := points, paint := canvas::fill(canvas::rgb(0, 200, 255))]
  LET box AS DrawItem = Rectangle[x := 10.0, y := 10.0, w := 50.0, h := 50.0, paint := canvas::fill(canvas::rgb(0, 255, 0))]
  canvas::present([box, ring])
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
    let binary = app_binary(&project, name);
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
        command.env("MFB_CANVAS_METAL", "1");
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

fn app_binary(project: &Path, name: &str) -> PathBuf {
    let bundle = project
        .join("build")
        .join(format!("{name}.app"))
        .join("Contents")
        .join("MacOS")
        .join(name);
    if bundle.exists() {
        return bundle;
    }
    project
        .join("build")
        .join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
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
        stats.contains("metalSelected=TRUE"),
        "MFB_CANVAS_METAL=1 did not select the Metal renderer: {stats}"
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
/// This is the test that keeps `MFB_CANVAS_METAL=1` honest. A backend that drew a
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
