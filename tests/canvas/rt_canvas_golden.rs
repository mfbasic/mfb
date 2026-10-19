//! The canvas software rasteriser matches its stored reference image exactly
//! (plan-98-C Phase 2), and the tolerance comparator plan-98-E/F will use behaves at
//! its documented thresholds.
//!
//! The fixture is the smiley from `planning/plan-98-api.md` — the scene that shaped
//! the API — rendered headless. It exercises the analytic-SDF circle, the
//! wedge-clipped stroked arc, overlapping opaque items, and antialiased edges over
//! both the background and another shape, which between them cover every code path in
//! the rasteriser that a single scene can.
//!
//! **A mismatch here is a bug hunt, not a re-baseline.** The software path is
//! deterministic, so a difference means the rendering changed. Localize it from the
//! reported coordinate and root-cause the primitive. Only once the *reference* has
//! been proven wrong — per AGENTS.md's four-question rule — regenerate it with
//! `MFB_UPDATE_CANVAS_GOLDEN=1`, and say in the commit what proved it.

#[path = "../common/mod.rs"]
mod common;

#[path = "rt_canvas_golden/ellipses_gradients_polylines.rs"]
mod ellipses_gradients_polylines;

use common::canvas_image::{compare_exact, compare_within_tolerance, Frame, Tolerance};
use std::path::{Path, PathBuf};
use std::process::Command;

const WIDTH: u32 = 900;
const HEIGHT: u32 = 640;

/// The smiley from `plan-98-api.md`, verbatim apart from being an `AS Nothing` function that returns
/// rather than blocking on input — a golden run must terminate.
const SMILEY: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color
IMPORT io

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)

  LET yellow AS color::Color = color::rgb(255, 255, 0)
  LET green AS color::Color = color::rgb(0, 160, 0)

  LET canvasSize AS canvas::Size = canvas::getSize()
  LET cx AS Float = canvasSize.width / 2.0
  LET cy AS Float = canvasSize.height / 2.0

  LET face AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := cx, y := cy], radius := 150.0, paint := canvas::fill(yellow), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET eyeL AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := cx - 50.0, y := cy - 40.0], radius := 22.0, paint := canvas::fill(green), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET eyeR AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := cx + 50.0, y := cy - 40.0], radius := 22.0, paint := canvas::fill(green), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET smile AS canvas::DrawItem = canvas::Arc[id := "", position := vector::Float2[x := cx, y := cy + 15.0], radius := 90.0, startAngle := 0.0, endAngle := 3.14159, paint := WITH canvas::stroke(green, 14.0) { cap := canvas::CapStyle.Butt }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]

  LET scene AS List OF canvas::DrawItem = [face, eyeL, eyeR, smile]

  canvas::present(scene)
  io::print("rendered")
END FUNC
"#;

fn golden_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join("canvas")
        .join(format!("{name}.png"))
}

/// Render a scene that needs a font, dropping the fixture beside the project first.
///
/// The same twelve-glyph TrueType `tests/rt_canvas_font.rs` and
/// `scripts/test-canvas-vulkan.sh` build — `unitsPerEm` 1000, one square glyph — rather
/// than a system font, so the reference cannot depend on which typefaces the machine
/// happens to have. A solid square is also the right glyph for a *reference*: whether a
/// rotated run landed correctly is a whole-pixel question rather than a judgement about
/// an antialiased curve.
fn render_with_font(name: &str, source: &str) -> Frame {
    render_inner(name, source, true, &[]).0
}

fn render(name: &str, source: &str) -> Frame {
    render_inner(name, source, false, &[]).0
}

/// The same render on the GPU, with the stats line that says whether it *was* the GPU.
///
/// The stats are not decoration here. A backend that declines a scene falls back to the
/// software renderer and returns a frame that matches the reference perfectly — so a
/// GPU comparison with no `gpuSelected=TRUE` check is a test that passes hardest
/// exactly when the hardware path is broken enough to be refused.
fn render_gpu_with_font(name: &str, source: &str) -> (Frame, String) {
    render_inner(name, source, true, &[("MFB_CANVAS_GPU", "1")])
}

/// The same, for a scene that needs no font.
fn render_gpu(name: &str, source: &str) -> (Frame, String) {
    render_inner(name, source, false, &[("MFB_CANVAS_GPU", "1")])
}

fn render_inner(name: &str, source: &str, font: bool, extra: &[(&str, &str)]) -> (Frame, String) {
    let project = common::temp_project(name, source);
    if font {
        std::fs::write(project.join("fixture.ttf"), common::fixture_truetype())
            .expect("write the font fixture");
    }
    let frame_path = project.join("frame.rgba");
    let stats_path = project.join("stats.txt");
    let binary = common::build_app_debug(&project, name);
    let mut command = Command::new(&binary);
    command
        // The project directory, so a scene that opens `fixture.ttf` finds it. Running
        // from the repository root instead would resolve the relative path against
        // cargo's cwd, where the fixture is not.
        .current_dir(&project)
        .env("MFB_HEADLESS", "1")
        // Wait for the frame this scene asked for. Without it `present` returns at
        // once and `main` returns behind it, and the process tears down while the
        // graphics thread is still reading the scene: the geometry survives (the ring
        // holds a published copy) but a `canvas::Font`'s outlines do not, because they
        // live in the worker's own arena, which is per-thread. The frame then lands
        // with every shape and no text — silently, and identically on every run, so it
        // reads as a reference rather than a truncated one. Measured on the transform
        // scene: 0 text pixels without this, 840 with it. Every other canvas suite
        // sets it; this one was the exception because no golden scene used a font
        // until plan-116-C's did.
        .env("MFB_CANVAS_SYNC", "1")
        .env("MFB_CANVAS_STATS", &stats_path)
        .env("MFB_CANVAS_DUMP", &frame_path);
    for (key, value) in extra {
        command.env(key, value);
    }
    let run = command
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", binary.display()));
    assert!(
        run.status.success(),
        "program {}:\n{}\n{}",
        common::exit_description(&run.status),
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr),
    );

    let pixels = std::fs::read(&frame_path).expect("canvas dump written");
    let stats = std::fs::read_to_string(&stats_path).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&project);
    (Frame::from_rgba(WIDTH, HEIGHT, pixels), stats)
}

/// The software rasteriser reproduces its reference image exactly.
#[test]
fn smiley_matches_its_reference_exactly() {
    let rendered = render("canvas_golden_smiley", SMILEY);
    let reference = golden_path("smiley");

    if std::env::var_os("MFB_UPDATE_CANVAS_GOLDEN").is_some() {
        rendered.save_png(&reference);
        panic!(
            "regenerated {} — rerun without MFB_UPDATE_CANVAS_GOLDEN, and record in \
             the commit what proved the previous reference wrong",
            reference.display(),
        );
    }

    assert!(
        reference.exists(),
        "missing reference {}; generate it with MFB_UPDATE_CANVAS_GOLDEN=1",
        reference.display(),
    );
    let want = Frame::load_png(&reference);
    if let Err(diff) = compare_exact(&rendered, &want) {
        panic!(
            "the smiley no longer renders to its reference image: {diff}\n\
             This is deterministic output, so something changed. Localize the \
             primitive at that coordinate before considering the reference wrong.",
        );
    }
}

/// A frame is exactly equal to itself, and the tolerance comparator agrees.
///
/// The trivial case is worth pinning because it is the one both comparators must
/// share: a tolerance path that somehow rejected an identical frame would make every
/// GPU backend fail for a reason that has nothing to do with the GPU.
#[test]
fn identical_frames_pass_both_comparators() {
    let frame = Frame::load_png(&golden_path("smiley"));
    assert!(compare_exact(&frame, &frame).is_ok());
    assert!(compare_within_tolerance(&frame, &frame, Tolerance::GPU_DEFAULT).is_ok());
}

/// Perturb one pixel by one step: exact-match rejects it, tolerance accepts it.
///
/// This is the whole point of having two comparators. One step on one pixel is
/// exactly the drift a different-but-correct GPU rasteriser produces on an
/// antialiased edge, and exactly the drift that must still fail the deterministic
/// software gate.
#[test]
fn a_one_step_perturbation_fails_exact_but_passes_tolerance() {
    let reference = Frame::load_png(&golden_path("smiley"));
    let mut perturbed = reference.clone();
    // A pixel inside the face, so the change is to a real rendered colour rather
    // than to the background.
    let index = ((HEIGHT / 2) as usize * WIDTH as usize + (WIDTH / 2) as usize) * 4;
    perturbed.pixels[index] = perturbed.pixels[index].wrapping_sub(1);

    let exact = compare_exact(&perturbed, &reference);
    assert!(exact.is_err(), "exact match accepted a changed pixel");
    let diff = exact.unwrap_err();
    assert_eq!(diff.differing_pixels, 1);
    assert_eq!(diff.max_channel_delta, 1);

    assert!(
        compare_within_tolerance(&perturbed, &reference, Tolerance::GPU_DEFAULT).is_ok(),
        "tolerance rejected a one-step difference on a single pixel",
    );
}

/// A difference beyond the per-channel epsilon fails tolerance too.
///
/// The channel limit is what stops a systematically wrong frame — a wrong gamma, a
/// half-pixel offset — from passing as sampling noise.
#[test]
fn a_large_channel_delta_fails_tolerance() {
    let reference = Frame::load_png(&golden_path("smiley"));
    let mut perturbed = reference.clone();
    let index = ((HEIGHT / 2) as usize * WIDTH as usize + (WIDTH / 2) as usize) * 4;
    perturbed.pixels[index] = perturbed.pixels[index].wrapping_sub(40);

    let diff = compare_within_tolerance(&perturbed, &reference, Tolerance::GPU_DEFAULT)
        .expect_err("tolerance accepted a 40-step channel difference");
    assert_eq!(diff.differing_pixels, 1);
    assert_eq!(diff.max_channel_delta, 40);
}

/// Too many pixels differing fails tolerance even when each is within the epsilon.
///
/// The differing-pixel budget is the other half: every pixel being off by one is not
/// noise, it is a systematic error, and a per-channel epsilon alone would wave it
/// through.
#[test]
fn too_many_small_differences_fail_tolerance() {
    let reference = Frame::load_png(&golden_path("smiley"));
    let mut perturbed = reference.clone();
    // Nudge a tenth of the frame by one step — five times the 2% budget, with every
    // individual difference inside the 2-step channel epsilon.
    let nudge = (WIDTH as usize * HEIGHT as usize) / 10;
    for pixel in 0..nudge {
        let index = pixel * 4;
        perturbed.pixels[index] = perturbed.pixels[index].wrapping_add(1);
    }

    let diff = compare_within_tolerance(&perturbed, &reference, Tolerance::GPU_DEFAULT)
        .expect_err("tolerance accepted a systematic one-step shift over 10% of the frame");
    assert_eq!(
        diff.max_channel_delta, 1,
        "each difference is within the epsilon"
    );
    assert_eq!(diff.differing_pixels, nudge);
}

/// The four blend modes, each over the same mid-grey ground (plan-116-B).
///
/// A reference image rather than only channel assertions, because
/// `rt_canvas_rasteriser`'s per-mode test samples **one pixel per mode** at full
/// coverage. That is the right shape for pinning the equations, and it is blind to
/// everything else: an antialiased edge under a non-`Normal` mode, a mode applied to a
/// stroke rather than a fill, and the overlap where two blended items meet. Those are
/// exactly the places a mode that is right at coverage 255 can still be wrong.
///
/// Mid grey is load-bearing. Over white or black the four modes collapse into each
/// other — `Multiply` with white is the source, `Screen` and `Add` with white are both
/// white — so a reference taken over either could not distinguish a correct renderer
/// from one that had `Screen` and `Add` swapped.
///
/// Each pair is a filled circle over a stroked rounded rectangle, so every frame
/// carries both paint channels under every mode.
const BLEND_MODES: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color
IMPORT io

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)

  LET ground AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 900.0, height := 640.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(128, 128, 128)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]

  LET warm AS color::Color = color::rgb(230, 120, 40)
  LET cool AS color::Color = color::rgb(40, 120, 230)

  LET boxNormal AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 40.0, y := 80.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 160.0, height := 160.0], radiusTL := 24.0, radiusTR := 24.0, radiusBL := 24.0, radiusBR := 24.0, paint := canvas::fillStroke(cool, color::rgb(255, 255, 255), 8.0), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET dotNormal AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 160.0, y := 200.0], radius := 70.0, paint := WITH canvas::fill(warm) { blend := canvas::BlendMode.Normal }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]

  LET boxMultiply AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 260.0, y := 80.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 160.0, height := 160.0], radiusTL := 24.0, radiusTR := 24.0, radiusBL := 24.0, radiusBR := 24.0, paint := canvas::fillStroke(cool, color::rgb(255, 255, 255), 8.0), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET dotMultiply AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 380.0, y := 200.0], radius := 70.0, paint := WITH canvas::fill(warm) { blend := canvas::BlendMode.Multiply }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]

  LET boxScreen AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 480.0, y := 80.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 160.0, height := 160.0], radiusTL := 24.0, radiusTR := 24.0, radiusBL := 24.0, radiusBR := 24.0, paint := canvas::fillStroke(cool, color::rgb(255, 255, 255), 8.0), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET dotScreen AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 600.0, y := 200.0], radius := 70.0, paint := WITH canvas::fill(warm) { blend := canvas::BlendMode.Screen }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]

  LET boxAdd AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 700.0, y := 80.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 160.0, height := 160.0], radiusTL := 24.0, radiusTR := 24.0, radiusBL := 24.0, radiusBR := 24.0, paint := canvas::fillStroke(cool, color::rgb(255, 255, 255), 8.0), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET dotAdd AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 820.0, y := 200.0], radius := 70.0, paint := WITH canvas::fill(warm) { blend := canvas::BlendMode.Add }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]

  ' A stroked arc under each mode too: a mode has to reach the stroke channel, not
  ' just the fill, and the stroke is the one that rides `salpha` rather than `alpha`.
  LET arcNormal AS canvas::DrawItem = canvas::Arc[id := "", position := vector::Float2[x := 160.0, y := 450.0], radius := 80.0, startAngle := 0.0, endAngle := 3.14159, paint := WITH canvas::stroke(warm, 16.0) { blend := canvas::BlendMode.Normal, cap := canvas::CapStyle.Butt }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET arcMultiply AS canvas::DrawItem = canvas::Arc[id := "", position := vector::Float2[x := 380.0, y := 450.0], radius := 80.0, startAngle := 0.0, endAngle := 3.14159, paint := WITH canvas::stroke(warm, 16.0) { blend := canvas::BlendMode.Multiply, cap := canvas::CapStyle.Butt }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET arcScreen AS canvas::DrawItem = canvas::Arc[id := "", position := vector::Float2[x := 600.0, y := 450.0], radius := 80.0, startAngle := 0.0, endAngle := 3.14159, paint := WITH canvas::stroke(warm, 16.0) { blend := canvas::BlendMode.Screen, cap := canvas::CapStyle.Butt }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET arcAdd AS canvas::DrawItem = canvas::Arc[id := "", position := vector::Float2[x := 820.0, y := 450.0], radius := 80.0, startAngle := 0.0, endAngle := 3.14159, paint := WITH canvas::stroke(warm, 16.0) { blend := canvas::BlendMode.Add, cap := canvas::CapStyle.Butt }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]

  ' One clipped item, so the reference covers the other half of this letter as well:
  ' a fractional clip edge that must stay antialiased.
  LET clipped AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 40.0, y := 560.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 820.0, height := 60.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH canvas::fill(color::rgb(255, 255, 255)) { clip := [[vector::Float2[x := 60.25, y := 0.0], vector::Float2[x := 760.75, y := 0.0], vector::Float2[x := 760.75, y := 60.0], vector::Float2[x := 60.25, y := 60.0]]] }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]

  canvas::present([ground, boxNormal, dotNormal, boxMultiply, dotMultiply, boxScreen, dotScreen, boxAdd, dotAdd, arcNormal, arcMultiply, arcScreen, arcAdd, clipped])
  io::print("rendered")
END FUNC
"#;

/// The blend-mode reference renders exactly.
///
/// Same rule as `smiley_matches_its_reference_exactly`: a mismatch is a bug hunt, not
/// a re-baseline. The software path is deterministic, so a difference here means one
/// of the four equations, the clip's coverage, or the sRGB chain moved.
#[test]
fn blend_modes_match_their_reference_exactly() {
    let rendered = render("canvas_golden_blend", BLEND_MODES);
    let reference = golden_path("blendmodes");

    if std::env::var_os("MFB_UPDATE_CANVAS_GOLDEN").is_some() {
        rendered.save_png(&reference);
        panic!(
            "regenerated {} — rerun without MFB_UPDATE_CANVAS_GOLDEN, and record in \
             the commit what proved the previous reference wrong",
            reference.display(),
        );
    }

    assert!(
        reference.exists(),
        "missing reference {}; generate it with MFB_UPDATE_CANVAS_GOLDEN=1",
        reference.display(),
    );
    let want = Frame::load_png(&reference);
    if let Err(diff) = compare_exact(&rendered, &want) {
        panic!(
            "the blend-mode scene no longer renders to its reference image: {diff}\n\
             This is deterministic output. Localize the mode at that coordinate: the \
             four pairs run left to right as Normal, Multiply, Screen, Add, the arcs \
             below them are the same four on the STROKE channel, and the band at the \
             bottom is a fractional clip edge.",
        );
    }
}

/// A rotated rect, a non-uniformly scaled circle, a sheared polygon and rotated text
/// (plan-116-C).
///
/// The four cases the transform work has to get right, and each is chosen to fail
/// differently:
///
/// - **The rotated rect** is the bounds case: its transformed hull is wider than its
///   shape-space box in both axes, so a renderer that kept the original box would slice
///   the corners off.
/// - **The non-uniformly scaled circle** is the distance-correction case. Phase 1
///   measured `sqrt(|det M|)` as 37/255 coverage steps wrong here, so this ellipse's
///   edge is where a wrong correction shows.
/// - **The sheared polygon** exercises the correction on an edge that is neither
///   axis-aligned nor curved, and the polygon SDF's crossing-count fill rule under a
///   mapping that does not preserve angles.
/// - **The rotated text** takes the separate inverse-sample arm, not the SDF path at
///   all, and its per-glyph quad is the run's transformed hull.
///
/// Each is drawn beside its untransformed twin on a mid-grey ground, so the reference
/// shows the transform's effect rather than just its result — a reader can see at a
/// glance whether the pair differs the way the matrix says.
const TRANSFORMS: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color
IMPORT io

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e)
    RETURN
  END TRAP

  LET ground AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 900.0, height := 640.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(96, 96, 96)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]

  ' 45 degrees: the hull is 1.41x the shape-space box in both axes.
  LET k AS Float = 0.7071067811865476
  LET rotT AS canvas::Transform = canvas::Transform[a := k, b := k, c := 0.0 - k, d := k, tx := 270.0, ty := 230.0 - 140.0 * k]
  LET plainRect AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 480.0, y := 90.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 140.0, height := 140.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 190, 60)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET rotRect AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 0.0 - 70.0, y := 0.0 - 70.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 140.0, height := 140.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 190, 60)), transform := rotT]

  ' 2:1 in x only -- the case sqrt(|det M|) gets wrong.
  LET scaleT AS canvas::Transform = canvas::Transform[a := 2.0, b := 0.0, c := 0.0, d := 1.0, tx := 200.0, ty := 380.0]
  LET plainCircle AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 550.0, y := 380.0], radius := 60.0, paint := canvas::fillStroke(color::rgb(90, 200, 255), color::rgb(255, 255, 255), 8.0), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET scaledCircle AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 0.0, y := 0.0], radius := 60.0, paint := canvas::fillStroke(color::rgb(90, 200, 255), color::rgb(255, 255, 255), 8.0), transform := scaleT]

  ' A 30 degree shear in x.
  LET shearT AS canvas::Transform = canvas::Transform[a := 1.0, b := 0.0, c := 0.5773502691896258, d := 1.0, tx := 660.0, ty := 480.0]
  LET tri AS List OF vector::Float2 = [vector::Float2[x := 0.0 - 60.0, y := 50.0], vector::Float2[x := 60.0, y := 50.0], vector::Float2[x := 0.0, y := 0.0 - 50.0]]
  LET plainPoly AS canvas::DrawItem = canvas::Polygon[id := "", position := vector::Float2[x := 0.0, y := 0.0], contours := [[vector::Float2[x := 200.0, y := 590.0], vector::Float2[x := 320.0, y := 590.0], vector::Float2[x := 260.0, y := 490.0]]], rule := canvas::FillRule.EvenOdd, paint := canvas::fill(color::rgb(220, 120, 220)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET shearPoly AS canvas::DrawItem = canvas::Polygon[id := "", position := vector::Float2[x := 0.0, y := 0.0], contours := [tri], rule := canvas::FillRule.EvenOdd, paint := canvas::fill(color::rgb(220, 120, 220)), transform := shearT]

  ' 90 degrees, so the rotated run is a vertical column of the fixture's squares.
  LET textT AS canvas::Transform = canvas::Transform[a := 0.0, b := 1.0, c := 0.0 - 1.0, d := 0.0, tx := 860.0, ty := 120.0]
  LET plainText AS canvas::DrawItem = canvas::Text[id := "", position := vector::Float2[x := 380.0, y := 300.0], text := "AA", fonts := [face], fontSize := 50.0, letterSpacing := 0.0, paint := canvas::fill(color::rgb(200, 255, 120)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET rotText AS canvas::DrawItem = canvas::Text[id := "", position := vector::Float2[x := 0.0, y := 0.0], text := "AA", fonts := [face], fontSize := 50.0, letterSpacing := 0.0, paint := canvas::fill(color::rgb(200, 255, 120)), transform := textT]

  canvas::present([ground, plainRect, rotRect, plainCircle, scaledCircle, plainPoly, shearPoly, plainText, rotText])
  io::print("rendered")
END FUNC
"#;

/// The transform reference renders exactly.
///
/// Same rule as the other two references: a mismatch is a bug hunt, not a re-baseline.
#[test]
fn transforms_match_their_reference_exactly() {
    let rendered = render_with_font("canvas_golden_transforms", TRANSFORMS);
    let reference = golden_path("transforms");

    if std::env::var_os("MFB_UPDATE_CANVAS_GOLDEN").is_some() {
        rendered.save_png(&reference);
        panic!(
            "regenerated {} — rerun without MFB_UPDATE_CANVAS_GOLDEN, and record in \
             the commit what proved the previous reference wrong",
            reference.display(),
        );
    }

    assert!(
        reference.exists(),
        "missing reference {}; generate it with MFB_UPDATE_CANVAS_GOLDEN=1",
        reference.display(),
    );
    let want = Frame::load_png(&reference);
    if let Err(diff) = compare_exact(&rendered, &want) {
        panic!(
            "the transform scene no longer renders to its reference image: {diff}\n\
             Deterministic output, so localize by pair: the rotated rect is the BOUNDS \
             hull, the scaled circle is the distance correction (Phase 1 measured \
             sqrt(|det M|) as 37/255 wrong exactly here), the sheared triangle is the \
             polygon fill rule under a non-similarity, and the rotated label is the \
             glyph inverse-sample arm.",
        );
    }
}

/// The hardware backend draws the transform scene the reference shows.
///
/// This is plan-116-C Phase 4's acceptance in the form the plan states it: the whole
/// `transforms.png` scene — all four transformed items *and* their untransformed twins
/// — rendered by whichever GPU backend this host has, compared against the stored
/// reference within `Tolerance::GPU_DEFAULT`.
///
/// The reference rather than a fresh software render, deliberately. Comparing GPU
/// against a same-run oracle would let a change that broke both in the same direction
/// pass; comparing against the committed image means the picture a human looked at is
/// the one the hardware has to reproduce.
///
/// A tolerance rather than `compare_exact` because the GPU composites in linear space
/// with hardware blending while the oracle blends in sRGB, so antialiased edges land a
/// step or two apart. `Tolerance::GPU_DEFAULT` bounds both how far one pixel may move
/// and how many may move at all — it is not a lever to widen when something fails.
#[test]
fn the_gpu_draws_the_transform_scene_the_reference_shows() {
    let (rendered, stats) = render_gpu_with_font("canvas_golden_transforms_gpu", TRANSFORMS);
    if !stats.contains("metalReady=TRUE") && !stats.contains("vulkanReady=TRUE") {
        eprintln!("skip: this host built no GPU pipeline\n{stats}");
        return;
    }
    assert!(
        !stats.contains("gpuFrames=0"),
        "the GPU pipeline built but no frame was rendered on it — a `*Renderable` \
         predicate declined the transform scene, and every pixel below would then be \
         the software renderer marking its own work: {stats}"
    );

    let reference = golden_path("transforms");
    assert!(
        reference.exists(),
        "missing reference {}; generate it with MFB_UPDATE_CANVAS_GOLDEN=1",
        reference.display(),
    );
    let want = Frame::load_png(&reference);
    if let Err(diff) = compare_within_tolerance(&rendered, &want, Tolerance::GPU_DEFAULT) {
        panic!(
            "the GPU's transform scene disagrees with the reference: {diff}\n\
             Localize by pair against the picture: a rotated rect with sliced corners \
             is a stale vertex quad, an ellipse whose stroke thickens along one axis \
             is the distance correction, and a label that vanished or landed upright \
             is the glyph inverse-sample arm."
        );
    }
}

/// Butt and round, on a line and on an arc, at a stroke width wide enough to read
/// (plan-116-D).
///
/// Each style is drawn beside its twin with everything else held equal, so the picture
/// shows the cap's effect rather than just its result — and so a reader can see at a
/// glance whether the pair differs the way the type says. That matters more here than
/// for the other references, because the two variants are asymmetric: a `Line` was
/// **round** before this letter and an `Arc` was **butt**, so "the new one" is the
/// right-hand shape in one row and the left-hand shape in the other.
///
/// The stroke is 28 px, so a cap is a 14 px feature — large enough that a wrong cap is
/// a visible block rather than an antialiasing argument. Endpoints are marked with a
/// thin crossing line at each end of the horizontal pair, because the whole claim about
/// `Butt` is *where it stops*, and an unmarked end is only checkable against arithmetic.
///
/// The bottom row is the degenerate pair from the rasteriser tests, at reference scale:
/// a zero-length round line is a dot and a zero-length butt line is nothing at all. The
/// empty half is deliberately present in the picture as an absence — a reader comparing
/// against the type's description should find nothing there.
const ENDCAPS: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color
IMPORT io

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)

  LET ground AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 900.0, height := 640.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(64, 68, 78)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]

  ' Row 1 -- lines. Butt left, round right, same geometry and paint otherwise.
  LET ink AS color::Color = color::rgb(255, 214, 92)
  LET buttLine AS canvas::DrawItem = canvas::Line[id := "", position := vector::Float2[x := 0.0, y := 0.0], startAt := vector::Float2[x := 120.0, y := 120.0], endAt := vector::Float2[x := 340.0, y := 120.0], paint := WITH canvas::stroke(ink, 28.0) { cap := canvas::CapStyle.Butt }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET roundLine AS canvas::DrawItem = canvas::Line[id := "", position := vector::Float2[x := 0.0, y := 0.0], startAt := vector::Float2[x := 540.0, y := 120.0], endAt := vector::Float2[x := 760.0, y := 120.0], paint := WITH canvas::stroke(ink, 28.0) { cap := canvas::CapStyle.Round }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]

  ' The endpoints, marked. A butt cap's claim is that the stroke stops HERE, which is
  ' only readable against something that says where "here" is.
  LET mark AS color::Color = color::rgb(255, 255, 255)
  LET markA AS canvas::DrawItem = canvas::Line[id := "", position := vector::Float2[x := 0.0, y := 0.0], startAt := vector::Float2[x := 340.0, y := 88.0], endAt := vector::Float2[x := 340.0, y := 152.0], paint := WITH canvas::stroke(mark, 2.0) { cap := canvas::CapStyle.Butt }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET markB AS canvas::DrawItem = canvas::Line[id := "", position := vector::Float2[x := 0.0, y := 0.0], startAt := vector::Float2[x := 760.0, y := 88.0], endAt := vector::Float2[x := 760.0, y := 152.0], paint := WITH canvas::stroke(mark, 2.0) { cap := canvas::CapStyle.Butt }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]

  ' Row 2 -- arcs, swept 0..0.6*PI so both ends are in view. Butt left, round right.
  LET arcInk AS color::Color = color::rgb(120, 226, 255)
  LET buttArc AS canvas::DrawItem = canvas::Arc[id := "", position := vector::Float2[x := 230.0, y := 300.0], radius := 90.0, startAngle := 0.0, endAngle := 1.884955592153876, paint := WITH canvas::stroke(arcInk, 28.0) { cap := canvas::CapStyle.Butt }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET roundArc AS canvas::DrawItem = canvas::Arc[id := "", position := vector::Float2[x := 650.0, y := 300.0], radius := 90.0, startAngle := 0.0, endAngle := 1.884955592153876, paint := WITH canvas::stroke(arcInk, 28.0) { cap := canvas::CapStyle.Round }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]

  ' Row 3 -- a diagonal pair, so a cap is checked on an axis the two rows above miss.
  LET diagInk AS color::Color = color::rgb(228, 148, 255)
  LET buttDiag AS canvas::DrawItem = canvas::Line[id := "", position := vector::Float2[x := 0.0, y := 0.0], startAt := vector::Float2[x := 140.0, y := 470.0], endAt := vector::Float2[x := 320.0, y := 560.0], paint := WITH canvas::stroke(diagInk, 28.0) { cap := canvas::CapStyle.Butt }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET roundDiag AS canvas::DrawItem = canvas::Line[id := "", position := vector::Float2[x := 0.0, y := 0.0], startAt := vector::Float2[x := 560.0, y := 470.0], endAt := vector::Float2[x := 740.0, y := 560.0], paint := WITH canvas::stroke(diagInk, 28.0) { cap := canvas::CapStyle.Round }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]

  ' Row 4 -- the degenerate pair. The butt one draws nothing, and its absence is part
  ' of the reference.
  LET dotInk AS color::Color = color::rgb(160, 255, 170)
  LET buttDot AS canvas::DrawItem = canvas::Line[id := "", position := vector::Float2[x := 0.0, y := 0.0], startAt := vector::Float2[x := 230.0, y := 610.0], endAt := vector::Float2[x := 230.0, y := 610.0], paint := WITH canvas::stroke(dotInk, 28.0) { cap := canvas::CapStyle.Butt }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET roundDot AS canvas::DrawItem = canvas::Line[id := "", position := vector::Float2[x := 0.0, y := 0.0], startAt := vector::Float2[x := 650.0, y := 610.0], endAt := vector::Float2[x := 650.0, y := 610.0], paint := WITH canvas::stroke(dotInk, 28.0) { cap := canvas::CapStyle.Round }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]

  canvas::present([ground, buttLine, roundLine, markA, markB, buttArc, roundArc, buttDiag, roundDiag, buttDot, roundDot])
  io::print("rendered")
END FUNC
"#;

/// The endcap reference renders exactly.
///
/// Same rule as the other three references: a mismatch is a bug hunt, not a
/// re-baseline.
#[test]
fn endcaps_match_their_reference_exactly() {
    let rendered = render("canvas_golden_endcaps", ENDCAPS);
    let reference = golden_path("endcaps");

    if std::env::var_os("MFB_UPDATE_CANVAS_GOLDEN").is_some() {
        rendered.save_png(&reference);
        panic!(
            "regenerated {} — rerun without MFB_UPDATE_CANVAS_GOLDEN, and record in \
             the commit what proved the previous reference wrong",
            reference.display(),
        );
    }

    assert!(
        reference.exists(),
        "missing reference {}; generate it with MFB_UPDATE_CANVAS_GOLDEN=1",
        reference.display(),
    );
    let want = Frame::load_png(&reference);
    if let Err(diff) = compare_exact(&rendered, &want) {
        panic!(
            "the endcap scene no longer renders to its reference image: {diff}\n\
             Localize by row: rows 1 and 3 are the line cap (butt left, round right), \
             row 2 is the arc cap — and note the asymmetry, because a Line was ROUND \
             before plan-116-D and an Arc was BUTT, so the pre-existing shape is on \
             opposite sides in the two. Row 4 is the degenerate pair; the left half is \
             meant to be empty."
        );
    }
}

/// The hardware backend draws the endcap scene the reference shows.
///
/// plan-116-D Phase 4's acceptance, in the form the plan states it. The same reasoning
/// as the transform scene's GPU test: compared against the **committed reference**
/// rather than a same-run oracle, so a change that broke both in the same direction
/// cannot pass, and gated on `gpuSelected=TRUE` first, because a backend that declined
/// the scene would fall back to software and reproduce the reference perfectly.
#[test]
fn the_gpu_draws_the_endcap_scene_the_reference_shows() {
    let (rendered, stats) = render_gpu("canvas_golden_endcaps_gpu", ENDCAPS);
    if !stats.contains("metalReady=TRUE") && !stats.contains("vulkanReady=TRUE") {
        eprintln!("skip: this host built no GPU pipeline\n{stats}");
        return;
    }
    assert!(
        !stats.contains("gpuFrames=0"),
        "the GPU pipeline built but no frame was rendered on it — a `*Renderable` \
         predicate declined a cap, and every pixel below would then be the software \
         renderer marking its own work: {stats}"
    );

    let reference = golden_path("endcaps");
    assert!(
        reference.exists(),
        "missing reference {}; generate it with MFB_UPDATE_CANVAS_GOLDEN=1",
        reference.display(),
    );
    let want = Frame::load_png(&reference);
    if let Err(diff) = compare_within_tolerance(&rendered, &want, Tolerance::GPU_DEFAULT) {
        panic!(
            "the GPU's endcap scene disagrees with the reference: {diff}\n\
             A butt end that still domes is the shader reading the wrong cap word; a \
             round end that is cut square is the same in reverse. An arc whose cap \
             disc is in the wrong place is the `arcCaps` ivec4 — check the vertex and \
             fragment declarations of ItemBlock agree, since only one of them sets the \
             std430 stride."
        );
    }
}

/// plan-172-E: a rectangle's corners — every meaning of a radius, per-corner radii, the
/// flattened outline stroked and dashed, and `Center` against `TopLeft` binding.
const CORNERS: &str = include_str!("scenes/rect_corners_on_black.mfb");

/// The corner reference renders exactly, and renders the same twice.
///
/// Rendered twice because the third build path flattens a corner with this package's
/// own sine and cosine and a segment count taken from a square root: nothing in it may
/// depend on what else the process has allocated or drawn. A reference that only one
/// run reproduces is not a reference.
#[test]
fn corners_match_their_reference_exactly() {
    let rendered = render("canvas_golden_corners", CORNERS);
    let again = render("canvas_golden_corners_again", CORNERS);
    assert!(
        compare_exact(&rendered, &again).is_ok(),
        "the corner scene does not render identically on two consecutive runs"
    );
    let reference = golden_path("corners");

    if std::env::var_os("MFB_UPDATE_CANVAS_GOLDEN").is_some() {
        rendered.save_png(&reference);
        panic!(
            "regenerated {} — rerun without MFB_UPDATE_CANVAS_GOLDEN, and record in \
             the commit what proved the previous reference wrong",
            reference.display(),
        );
    }

    assert!(
        reference.exists(),
        "missing reference {}; generate it with MFB_UPDATE_CANVAS_GOLDEN=1",
        reference.display(),
    );
    let want = Frame::load_png(&reference);
    if let Err(diff) = compare_exact(&rendered, &want) {
        panic!(
            "the corner scene no longer renders to its reference image: {diff}\n\
             Localize by row: row 1 is one radius value's three meanings and the pill, \
             row 2 is per-corner radii (the flattened outline), row 3 is that outline \
             stroked and dashed beside the exact rounded and square ones, and row 4 is \
             `bind` under a turn."
        );
    }
}

/// plan-150-A: two particle systems at one fixed `(seed, time)` each — a firework burst
/// of fading, shrinking discs and a fountain of spinning outlined rectangles.
const PARTICLES: &str = include_str!("scenes/particles_on_black.mfb");

/// The particle reference renders exactly, and renders the same twice.
///
/// Rendered twice because that is the feature's first claim: a system holds no state
/// between frames and reads no clock, so the picture is a function of its fields alone.
/// Every particle here is placed by the package's own sine and cosine and its own
/// exponential, and nothing in it may depend on what else the process has drawn.
#[test]
fn particles_match_their_reference_exactly() {
    let rendered = render("canvas_golden_particles", PARTICLES);
    let again = render("canvas_golden_particles_again", PARTICLES);
    assert!(
        compare_exact(&rendered, &again).is_ok(),
        "the particle scene does not render identically on two consecutive runs"
    );
    let reference = golden_path("particles");

    if std::env::var_os("MFB_UPDATE_CANVAS_GOLDEN").is_some() {
        rendered.save_png(&reference);
        panic!(
            "regenerated {} — rerun without MFB_UPDATE_CANVAS_GOLDEN, and record in \
             the commit what proved the previous reference wrong",
            reference.display(),
        );
    }

    assert!(
        reference.exists(),
        "missing reference {}; generate it with MFB_UPDATE_CANVAS_GOLDEN=1",
        reference.display(),
    );
    let want = Frame::load_png(&reference);
    if let Err(diff) = compare_exact(&rendered, &want) {
        panic!(
            "the particle scene no longer renders to its reference image: {diff}\n\
             Localize by side: the left is the firework (a burst from a point; discs \
             that are scaled and tinted but never turned), the right is the fountain \
             (a stream from a line; rectangles that are turned, scaled and stroked)."
        );
    }
}

/// The GPU draws the particle scene the reference shows, and draws it ON the GPU.
///
/// This is the design's central claim, measured: a particle is an ordinary item by the
/// time any backend sees it, so no shader, pipeline or buffer layout knows particles
/// exist. Skips where no GPU backend selects itself.
#[test]
fn the_gpu_draws_the_particle_scene_the_reference_shows() {
    let (rendered, stats) = render_gpu("canvas_golden_particles_gpu", PARTICLES);
    if !stats.contains("metalReady=TRUE") && !stats.contains("vulkanReady=TRUE") {
        eprintln!("skip: this host built no GPU pipeline\n{stats}");
        return;
    }
    assert!(
        !stats.contains("gpuFrames=0"),
        "the GPU pipeline built but no frame was rendered on it — a `*Renderable` \
         predicate declined the particle scene, and every pixel below would then be \
         the software renderer marking its own work: {stats}"
    );
    let want = Frame::load_png(&golden_path("particles"));
    if let Err(diff) = compare_within_tolerance(&rendered, &want, Tolerance::GPU_DEFAULT) {
        panic!("the GPU's particle scene is outside tolerance of the reference: {diff}");
    }
}

/// plan-172-L: pictures inside shapes — a circle, a rounded rectangle and a star; the
/// three fits; a turned shape under each clip mode; a translucent picture; one in a
/// scaled group; a shape's outline intersected with its paint's clip; a tint; and a
/// picture on an `AutoPaint`, at rest.
const PICTURE_SHAPES: &str = include_str!("scenes/picture_shapes_on_black.mfb");

/// The shaped-picture reference renders exactly, and renders the same twice.
///
/// Twice for the corner scene's reason, and one more: a circle's and an ellipse's
/// outline is flattened with this package's own sine and cosine, and the intersection
/// of an outline with a clip sorts and merges coordinates — nothing in either may
/// depend on what else the process has done.
#[test]
fn picture_shapes_match_their_reference_exactly() {
    let rendered = render("canvas_golden_picture_shapes", PICTURE_SHAPES);
    let again = render("canvas_golden_picture_shapes_again", PICTURE_SHAPES);
    assert!(
        compare_exact(&rendered, &again).is_ok(),
        "the shaped-picture scene does not render identically on two consecutive runs"
    );
    let reference = golden_path("picture_shapes");

    if std::env::var_os("MFB_UPDATE_CANVAS_GOLDEN").is_some() {
        rendered.save_png(&reference);
        panic!(
            "regenerated {} — rerun without MFB_UPDATE_CANVAS_GOLDEN, and record in \
             the commit what proved the previous reference wrong",
            reference.display(),
        );
    }

    assert!(
        reference.exists(),
        "missing reference {}; generate it with MFB_UPDATE_CANVAS_GOLDEN=1",
        reference.display(),
    );
    let want = Frame::load_png(&reference);
    if let Err(diff) = compare_exact(&rendered, &want) {
        panic!(
            "the shaped-picture scene no longer renders to its reference image: {diff}\n\
             Localize by row: row 1 is the outline of each closed kind and the two \
             proportional fits, row 2 the two clip modes under a turn, transparency and \
             a scaled group, row 3 the clip intersection, the tint, per-corner radii \
             and the `AutoPaint` shape."
        );
    }
}

/// A scene containing a `canvas::Group` reaches the GPU, on whichever backend this host
/// has.
///
/// This assertion has been inverted once, deliberately, and the history is the point.
/// plan-116-G added it as "a group scene DECLINES to software": neither backend knew the
/// per-draw offset, so accepting the frame would have drawn every group's children at
/// the **origin** — the item blocks carry each shape's own coordinates and the group
/// translation lived only in the software walk. `.ai/canvas-threading.md` §10 records
/// that as having happened once already, and it is a plausible wrong picture reported as
/// success rather than a fault.
///
/// plan-116-H taught both backends the offset — Vulkan in Phase 2, Metal in Phase 3 —
/// so the decline became a refusal to do work the backend can do, and this test now
/// pins the other side of it.
///
/// **It asserts the frame count, not the pixels, and that is not laziness.** A declined
/// frame is drawn by the software renderer, which is the oracle, so it matches any
/// reference *by construction*: a pixel comparison here would pass whether the group
/// reached the GPU or not. The pixels are asserted where a real GPU frame exists to
/// compare — `every_group_case_matches_the_software_oracle` in `rt_canvas_metal.rs`, and
/// the group stage of `scripts/test-canvas-vulkan.sh`.
///
/// The subtlety worth keeping from the original: a predicate cannot decline by searching
/// the offsets list for a `Group` item, because by the time it sees that list the walk
/// has expanded every group away and a search finds nothing. plan-116-G solved that with
/// a flag the walk itself set; with both backends taught, the flag has no reader and is
/// gone, so a future backend that needs to decline has to set one again.
#[test]
fn a_scene_containing_a_group_reaches_the_gpu() {
    let (_, stats) = render_gpu("canvas_group_declines", GROUP_SCENE);
    if !stats.contains("metalReady=TRUE") && !stats.contains("vulkanReady=TRUE") {
        eprintln!("skip: this host built no GPU pipeline\n{stats}");
        return;
    }
    assert!(
        !stats.contains("gpuFrames=0"),
        "a scene containing a `canvas::Group` was declined to software. Both backends \
         have known the per-draw offset since plan-116-H, so declining means a \
         predicate is refusing work the backend can do: {stats}"
    );
    // And the control: the same scene with the group replaced by the item itself is
    // rendered on the GPU too, so a host that accepted everything would not make the
    // assertion above look like it proved something about groups.
    let (_, flat) = render_gpu("canvas_group_declines_control", FLAT_SCENE);
    if flat.contains("metalReady=TRUE") || flat.contains("vulkanReady=TRUE") {
        assert!(
            !flat.contains("gpuFrames=0"),
            "the group-free control scene was declined, so this host declines for some \
             reason other than groups and the assertion above proves nothing: {flat}"
        );
    }
}

const GROUP_SCENE: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color
IMPORT io

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  LET red AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 100.0, height := 100.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  canvas::setGroup("panel", [red])
  canvas::present([canvas::Group[id := "", position := vector::Float2[x := 200.0, y := 200.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := "panel"]])
  io::print("rendered")
END FUNC
"#;

const FLAT_SCENE: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color
IMPORT io

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  LET red AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 200.0, y := 200.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 100.0, height := 100.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  canvas::present([red])
  io::print("rendered")
END FUNC
"#;

/// plan-172-G: groups under a transform — turned, scaled unevenly, nested, at a
/// fraction of their size, and one leaf drawn through two nodes.
const GROUP_AFFINE: &str = include_str!("scenes/group_affine_on_black.mfb");

/// The group-affine reference renders exactly, and renders the same twice.
///
/// Exactly, because the software renderer is the oracle the GPU backends will be held
/// to (plan-172-H). Twice, because the composed inverse is built per frame from a table
/// the walk fills as it goes: nothing in it may depend on what the process drew before.
#[test]
fn group_affine_matches_its_reference_exactly() {
    let rendered = render_with_font("canvas_golden_group_affine", GROUP_AFFINE);
    let again = render_with_font("canvas_golden_group_affine_again", GROUP_AFFINE);
    assert!(
        compare_exact(&rendered, &again).is_ok(),
        "the group-affine scene does not render identically on two consecutive runs"
    );
    let reference = golden_path("group_affine");

    if std::env::var_os("MFB_UPDATE_CANVAS_GOLDEN").is_some() {
        rendered.save_png(&reference);
        panic!(
            "regenerated {} — rerun without MFB_UPDATE_CANVAS_GOLDEN, and record in \
             the commit what proved the previous reference wrong",
            reference.display(),
        );
    }

    assert!(
        reference.exists(),
        "missing reference {}; generate it with MFB_UPDATE_CANVAS_GOLDEN=1",
        reference.display(),
    );
    let expected = Frame::load_png(&reference);
    if let Err(diff) = compare_exact(&rendered, &expected) {
        panic!("the group-affine scene differs from its reference: {diff:?}");
    }
}

/// plan-172-H: the GPU draws the group-affine scene the reference shows.
///
/// The reference is the software renderer's (plan-172-G), and the scene carries what a
/// matrix has to carry beyond an outline: a gradient ramp, a rectangle clip, a stroke
/// under an uneven scale, a run of text, two nests and a translucent diamond. The frame
/// count is asserted first, because a predicate that still declined this scene would
/// hand the comparison the frame the reference was made from.
#[test]
fn the_gpu_draws_the_group_affine_scene_the_reference_shows() {
    let (rendered, stats) = render_gpu_with_font("canvas_golden_group_affine_gpu", GROUP_AFFINE);
    if !stats.contains("metalReady=TRUE") && !stats.contains("vulkanReady=TRUE") {
        eprintln!("skip: this host built no GPU pipeline\n{stats}");
        return;
    }
    assert!(
        !stats.contains("gpuFrames=0"),
        "the GPU pipeline built but no frame was rendered on it — a `*Renderable` \
         predicate declined the group-affine scene: {stats}"
    );
    let reference = golden_path("group_affine");
    let want = Frame::load_png(&reference);
    if let Err(diff) = compare_within_tolerance(&rendered, &want, Tolerance::GPU_DEFAULT) {
        panic!(
            "the GPU's group-affine scene disagrees with the reference: {diff}\n\
             Localize by panel. Every panel upright: the matrix never reached the \
             backend. Panels turned but cut off at a straight edge: the vertex stage's \
             quad. Right outlines with a ramp, a clipped band or letters in the wrong \
             place: the matrix reached the vertex stage only. Only the squashed ring's \
             stroke wrong: the distance is not rescaled by the composed map."
        );
    }
}

/// The group reference scene (plan-116-H Phase 4).
///
/// Every drawn item lives inside a group, so a renderer that ignored the per-draw offset
/// would still produce a complete, plausible picture — with everything stacked at the
/// origin. That is the failure `.ai/canvas-threading.md` §10 records, and only a
/// comparison at a NON-ZERO offset can see it.
///
/// The group's item list carries a **gradient-filled** item, a **`Text`** item and a
/// **clipped** item on purpose (plan-116-H **H2**). Those are the three positional reads
/// that do not follow the shifted query point for free: a gradient ramp and a glyph
/// bitmap are sampled per fragment from the absolute point, and a clip is deliberately
/// *not* shifted because it is a surface rectangle. A scene of plain filled shapes
/// cannot see any of the three go wrong.
///
/// The same group is drawn twice — once at the origin and once at an offset — so the
/// picture shows the offset's effect side by side rather than only its result. The
/// nested group and the diamond are the two structural cases: a flattened diamond would
/// change the picture, and so would a nested offset that failed to compose.
const GROUPS: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e)
    RETURN
  END TRAP
  LET stops AS List OF canvas::GradientStop = [canvas::GradientStop[offset := 0.0, color := color::rgb(255, 64, 32)], canvas::GradientStop[offset := 0.55, color := color::rgb(250, 230, 90)], canvas::GradientStop[offset := 1.0, color := color::rgb(32, 96, 255)]]
  LET ramp AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, startPoint := vector::Float2[x := 0.0, y := 0.0], endPoint := vector::Float2[x := 160.0, y := 70.0], stops := stops, spread := canvas::GradientSpread.Pad, transform := canvas::Transform[a := 0.0, b := 0.0, c := 0.0, d := 0.0, tx := 0.0, ty := 0.0], focalOffset := vector::Float2[x := 0.0, y := 0.0], focalRadius := 0.0, strokeApply := canvas::GradientApply.FillOnly]
  ' The panel: a gradient-filled bar, a clipped white band, a plain circle and a text
  ' run, all in the group's own coordinates.
  LET bar AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 160.0, height := 70.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH canvas::fill(color::rgb(0, 0, 0)) { gradient := ramp }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET band AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 0.0, y := 80.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 160.0, height := 30.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := WITH canvas::fill(color::rgb(255, 255, 255)) { clip := [[vector::Float2[x := 40.25, y := 0.0 - 80.0], vector::Float2[x := 130.75, y := 0.0 - 80.0], vector::Float2[x := 130.75, y := 560.0], vector::Float2[x := 40.25, y := 560.0]]] }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET dot AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 30.0, y := 140.0], radius := 22.0, paint := canvas::fillStroke(color::rgb(0, 170, 220), color::rgb(255, 255, 255), 5.0), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET tag AS canvas::DrawItem = canvas::Text[id := "", position := vector::Float2[x := 70.0, y := 155.0], text := "AA", fonts := [face], fontSize := 46.0, letterSpacing := 0.0, paint := canvas::fill(color::rgb(230, 60, 170)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  canvas::setGroup("panel", [bar, band, dot, tag])
  ' A nested group: the outer holds the panel, so the two offsets compose.
  canvas::setGroup("outer", [canvas::Group[id := "", position := vector::Float2[x := 20.0, y := 30.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := "panel"]])
  ' A leaf referenced twice from the scene -- the diamond.
  canvas::setGroup("leaf", [canvas::Rectangle[id := "", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 70.0, height := 45.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(120, 220, 60)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]])
  canvas::present([canvas::Group[id := "", position := vector::Float2[x := 0.0, y := 0.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := "panel"], canvas::Group[id := "", position := vector::Float2[x := 340.0, y := 210.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := "panel"], canvas::Group[id := "", position := vector::Float2[x := 600.0, y := 40.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := "outer"], canvas::Group[id := "", position := vector::Float2[x := 120.0, y := 420.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := "leaf"], canvas::Group[id := "", position := vector::Float2[x := 260.0, y := 520.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := "leaf"]])
END FUNC
"#;

/// The software rasteriser reproduces the group reference exactly.
///
/// Exactly, not within tolerance: the software renderer *is* the oracle, so its own
/// output against its own stored reference is a byte comparison. A tolerance here would
/// let the oracle drift and take both GPU assertions with it.
#[test]
fn groups_match_their_reference_exactly() {
    let rendered = render_with_font("canvas_golden_groups", GROUPS);
    let reference = golden_path("groups");

    if std::env::var_os("MFB_UPDATE_CANVAS_GOLDEN").is_some() {
        rendered.save_png(&reference);
        panic!(
            "regenerated {} — rerun without MFB_UPDATE_CANVAS_GOLDEN, and record in \
             the commit what proved the previous reference wrong",
            reference.display(),
        );
    }

    assert!(
        reference.exists(),
        "missing reference {}; generate it with MFB_UPDATE_CANVAS_GOLDEN=1",
        reference.display(),
    );
    let want = Frame::load_png(&reference);
    if let Err(diff) = compare_exact(&rendered, &want) {
        panic!(
            "the group scene no longer renders to its reference image: {diff}\n\
             Localize against the picture: the panel appears twice, at (0,0) and at \
             (340,210), so a difference in only one of them is the offset and a \
             difference in both is the panel's own drawing. The pair of green leaves \
             is the diamond — one group installed once and named twice — and the panel \
             at (620,70) is the nested case, whose offset is the outer's plus the \
             inner's."
        );
    }
}

/// Both GPU backends draw the group scene the reference shows.
///
/// Within `Tolerance::GPU_DEFAULT` rather than exactly, for the reason every GPU
/// reference assertion here is: the hardware blends in float and the oracle blends
/// through a 16-bit linear table, so a blended pixel agrees to within a step or two and
/// rarely bit for bit.
///
/// The frame count is asserted first. A `*Renderable` predicate that declined this scene
/// would hand the comparison a software frame — the very thing the reference was made
/// from — and every pixel would match while proving nothing about the GPU. plan-116-G's
/// decline did exactly that until Phase 2 and Phase 3 removed it.
#[test]
fn the_gpu_draws_the_group_scene_the_reference_shows() {
    let (rendered, stats) = render_gpu_with_font("canvas_golden_groups_gpu", GROUPS);
    if !stats.contains("metalReady=TRUE") && !stats.contains("vulkanReady=TRUE") {
        eprintln!("skip: this host built no GPU pipeline\n{stats}");
        return;
    }
    assert!(
        !stats.contains("gpuFrames=0"),
        "the GPU pipeline built but no frame was rendered on it — a `*Renderable` \
         predicate declined the group scene, and every pixel below would then be the \
         software renderer marking its own work: {stats}"
    );

    let reference = golden_path("groups");
    assert!(
        reference.exists(),
        "missing reference {}; generate it with MFB_UPDATE_CANVAS_GOLDEN=1",
        reference.display(),
    );
    let want = Frame::load_png(&reference);
    if let Err(diff) = compare_within_tolerance(&rendered, &want, Tolerance::GPU_DEFAULT) {
        panic!(
            "the GPU's group scene disagrees with the reference: {diff}\n\
             Localize by what moved and what did not. Every item at the ORIGIN means \
             the per-draw offset never reached the backend. A shape in the right place \
             whose gradient ramp or glyph ink is shifted means the offset reached the \
             VERTEX stage only — the fragment stage evaluates distance, ramp and \
             coverage at an absolute point and needs it too. A clipped band that moved \
             with its group means the opposite mistake: the clip is a surface rectangle \
             and must be evaluated at the un-shifted point."
        );
    }
}

/// `canvas::PolyLine` (plan-154-A): joins, both caps, per-point colours, and the
/// degenerate paths.
///
/// - **Row 1** is one zig-zag drawn twice, round caps left and butt caps right, with the
///   butt one's two end points marked. Every interior corner is round in both — a
///   polyline has no other join — so the two differ only at the ends.
/// - **Row 2** is a 180° switch-back, where the two segments overlap for most of their
///   length and a join that is not a true union would show a seam; and a butt path whose
///   end points are repeated, with a zero-length segment in the middle. The repeated
///   end points must leave the ends BUTT (plan-154-A Correction A2): the cut moves to the
///   last segment with length.
/// - **Row 3** is the per-point colours: an orange trail fading to transparent, and a
///   red→green→blue path, mixed in linear light as a gradient is.
/// - **Row 4** is fewer colours than points (the rest take `paint.stroke`, white), a
///   two-point diagonal, a path whose points all coincide with round caps (a dot) and
///   with butt caps (nothing), and a one-point path (nothing). The absences are part of
///   the reference.
const POLYLINES: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color
IMPORT io

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)

  LET ground AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 900.0, height := 640.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(52, 56, 66)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET mark AS color::Color = color::rgb(255, 255, 255)

  ' Row 1 -- one zig-zag, round caps left and butt caps right.
  LET ink AS color::Color = color::rgb(255, 214, 92)
  LET zigRound AS canvas::DrawItem = canvas::PolyLine[id := "", position := vector::Float2[x := 0.0, y := 0.0], points := [vector::Float2[x := 60.0, y := 150.0], vector::Float2[x := 140.0, y := 50.0], vector::Float2[x := 220.0, y := 150.0], vector::Float2[x := 300.0, y := 50.0], vector::Float2[x := 380.0, y := 150.0]], colors := [], paint := WITH canvas::stroke(ink, 22.0) { cap := canvas::CapStyle.Round }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET zigButt AS canvas::DrawItem = canvas::PolyLine[id := "", position := vector::Float2[x := 0.0, y := 0.0], points := [vector::Float2[x := 500.0, y := 150.0], vector::Float2[x := 580.0, y := 50.0], vector::Float2[x := 660.0, y := 150.0], vector::Float2[x := 740.0, y := 50.0], vector::Float2[x := 820.0, y := 150.0]], colors := [], paint := WITH canvas::stroke(ink, 22.0) { cap := canvas::CapStyle.Butt }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET markA AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 500.0, y := 150.0], radius := 2.0, paint := canvas::fill(mark), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET markB AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 820.0, y := 150.0], radius := 2.0, paint := canvas::fill(mark), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]

  ' Row 2 -- a switch-back, and a butt path with repeated end points and a zero-length
  ' middle segment.
  LET cool AS color::Color = color::rgb(120, 214, 255)
  LET switchBack AS canvas::DrawItem = canvas::PolyLine[id := "", position := vector::Float2[x := 0.0, y := 0.0], points := [vector::Float2[x := 60.0, y := 250.0], vector::Float2[x := 380.0, y := 250.0], vector::Float2[x := 120.0, y := 280.0]], colors := [], paint := WITH canvas::stroke(cool, 18.0) { cap := canvas::CapStyle.Round }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET repeated AS canvas::DrawItem = canvas::PolyLine[id := "", position := vector::Float2[x := 0.0, y := 0.0], points := [vector::Float2[x := 500.0, y := 230.0], vector::Float2[x := 500.0, y := 230.0], vector::Float2[x := 660.0, y := 230.0], vector::Float2[x := 660.0, y := 230.0], vector::Float2[x := 820.0, y := 300.0], vector::Float2[x := 820.0, y := 300.0]], colors := [], paint := WITH canvas::stroke(cool, 18.0) { cap := canvas::CapStyle.Butt }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET markC AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 500.0, y := 230.0], radius := 2.0, paint := canvas::fill(mark), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET markD AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 820.0, y := 300.0], radius := 2.0, paint := canvas::fill(mark), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]

  ' Row 3 -- per-point colours.
  LET trail AS canvas::DrawItem = canvas::PolyLine[id := "", position := vector::Float2[x := 0.0, y := 0.0], points := [vector::Float2[x := 60.0, y := 420.0], vector::Float2[x := 110.0, y := 370.0], vector::Float2[x := 170.0, y := 350.0], vector::Float2[x := 230.0, y := 360.0], vector::Float2[x := 290.0, y := 400.0], vector::Float2[x := 340.0, y := 440.0], vector::Float2[x := 390.0, y := 450.0]], colors := [color::rgba(255, 140, 40, 255), color::rgba(255, 140, 40, 212), color::rgba(255, 140, 40, 170), color::rgba(255, 140, 40, 128), color::rgba(255, 140, 40, 85), color::rgba(255, 140, 40, 42), color::rgba(255, 140, 40, 0)], paint := WITH canvas::stroke(color::rgba(0, 0, 0, 0), 16.0) { cap := canvas::CapStyle.Round }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET rainbow AS canvas::DrawItem = canvas::PolyLine[id := "", position := vector::Float2[x := 0.0, y := 0.0], points := [vector::Float2[x := 500.0, y := 440.0], vector::Float2[x := 660.0, y := 350.0], vector::Float2[x := 820.0, y := 440.0]], colors := [color::rgb(255, 0, 0), color::rgb(0, 255, 0), color::rgb(0, 0, 255)], paint := WITH canvas::stroke(color::rgb(255, 255, 255), 20.0) { cap := canvas::CapStyle.Butt }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]

  ' Row 4 -- fewer colours than points, a two-point path, the coincident pair, and a
  ' one-point path.
  LET partial AS canvas::DrawItem = canvas::PolyLine[id := "", position := vector::Float2[x := 0.0, y := 0.0], points := [vector::Float2[x := 60.0, y := 590.0], vector::Float2[x := 110.0, y := 540.0], vector::Float2[x := 160.0, y := 590.0], vector::Float2[x := 210.0, y := 540.0], vector::Float2[x := 260.0, y := 590.0], vector::Float2[x := 310.0, y := 540.0]], colors := [color::rgb(228, 148, 255), color::rgb(228, 148, 255), color::rgb(80, 40, 200)], paint := WITH canvas::stroke(color::rgb(255, 255, 255), 10.0) { cap := canvas::CapStyle.Round }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET twoPoint AS canvas::DrawItem = canvas::PolyLine[id := "", position := vector::Float2[x := 0.0, y := 0.0], points := [vector::Float2[x := 380.5, y := 530.25], vector::Float2[x := 520.75, y := 600.5]], colors := [], paint := WITH canvas::stroke(color::rgb(160, 255, 170), 14.0) { cap := canvas::CapStyle.Butt }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET dotRound AS canvas::DrawItem = canvas::PolyLine[id := "", position := vector::Float2[x := 0.0, y := 0.0], points := [vector::Float2[x := 620.0, y := 570.0], vector::Float2[x := 620.0, y := 570.0], vector::Float2[x := 620.0, y := 570.0]], colors := [], paint := WITH canvas::stroke(color::rgb(160, 255, 170), 24.0) { cap := canvas::CapStyle.Round }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET dotButt AS canvas::DrawItem = canvas::PolyLine[id := "", position := vector::Float2[x := 0.0, y := 0.0], points := [vector::Float2[x := 720.0, y := 570.0], vector::Float2[x := 720.0, y := 570.0]], colors := [], paint := WITH canvas::stroke(color::rgb(160, 255, 170), 24.0) { cap := canvas::CapStyle.Butt }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET single AS canvas::DrawItem = canvas::PolyLine[id := "", position := vector::Float2[x := 0.0, y := 0.0], points := [vector::Float2[x := 820.0, y := 570.0]], colors := [color::rgb(255, 0, 0)], paint := WITH canvas::stroke(color::rgb(160, 255, 170), 24.0) { cap := canvas::CapStyle.Round }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]

  canvas::present([ground, zigRound, zigButt, markA, markB, switchBack, repeated, markC, markD, trail, rainbow, partial, twoPoint, dotRound, dotButt, single])
  io::print("rendered")
END FUNC
"#;
