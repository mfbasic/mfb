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

mod common;

use common::canvas_image::{compare_exact, compare_within_tolerance, Frame, Tolerance};
use std::path::{Path, PathBuf};
use std::process::Command;

const WIDTH: u32 = 900;
const HEIGHT: u32 = 640;

/// The smiley from `plan-98-api.md`, verbatim apart from being a `SUB` that returns
/// rather than blocking on input — a golden run must terminate.
const SMILEY: &str = r#"IMPORT app
IMPORT canvas
IMPORT io

SUB main()
  app::setMode(app::Mode.Canvas)

  LET yellow AS canvas::Color = canvas::rgb(255, 255, 0)
  LET green AS canvas::Color = canvas::rgb(0, 160, 0)

  LET canvasSize AS canvas::Size = canvas::getSize()
  LET cx AS Float = toFloat(canvasSize.width) / 2.0
  LET cy AS Float = toFloat(canvasSize.height) / 2.0

  LET face AS canvas::DrawItem = canvas::Circle[x := cx, y := cy, radius := 150.0, paint := canvas::fill(yellow)]
  LET eyeL AS canvas::DrawItem = canvas::Circle[x := cx - 50.0, y := cy - 40.0, radius := 22.0, paint := canvas::fill(green)]
  LET eyeR AS canvas::DrawItem = canvas::Circle[x := cx + 50.0, y := cy - 40.0, radius := 22.0, paint := canvas::fill(green)]
  LET smile AS canvas::DrawItem = canvas::Arc[x := cx, y := cy + 15.0, radius := 90.0, startAngle := 0.0, endAngle := 3.14159, cap := canvas::CapStyle.Butt, paint := canvas::stroke(green, 14.0)]

  LET scene AS List OF canvas::DrawItem = [face, eyeL, eyeR, smile]

  canvas::present(scene)
  io::print("rendered")
END SUB
"#;

/// The twelve-glyph fixture font, as bytes.
fn fixture_truetype() -> Vec<u8> {
    const B64: &str = concat!(
        "AAEAAAAGAAAAAAAAY21hcAAAAAAAAABsAAAANGdseWYAAAAAAAAAoAAAACJoZWFkAAAAAAAAAMIA",
        "AAA2aGhlYQAAAAAAAAD4AAAAJGhtdHgAAAAAAAABHAAAAAxsb2NhAAAAAAAAASgAAAAIAAAAAQAD",
        "AAoAAAAMAAwAAAAAACgAAAAAAAAAAgAAAEEAAABBAAAAAQAAAEIAAABCAAAAAgABAGQAAAGQASwA",
        "AwAAAQEBAQBkASwAAP7UAAAAAAEsAAAAAAAAAAAAAAAAAAAAAAAAAAAD6AAAAAAAAAAAAAAAAAAA",
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAyD/OABkAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAMB",
        "9AAAAPoAAAEsAAAAAAAAABEAEQ==",
    );
    let table: Vec<u8> =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/".to_vec();
    let mut out = Vec::new();
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    for ch in B64.bytes() {
        if ch == b'=' {
            break;
        }
        let v = table
            .iter()
            .position(|&c| c == ch)
            .expect("base64 alphabet") as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

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

fn render_inner(name: &str, source: &str, font: bool, extra: &[(&str, &str)]) -> (Frame, String) {
    let project = common::temp_project(name, source);
    if font {
        std::fs::write(project.join("fixture.ttf"), fixture_truetype())
            .expect("write the font fixture");
    }
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

    let frame_path = project.join("frame.rgba");
    let stats_path = project.join("stats.txt");
    let binary = app_binary(&project, name);
    let mut command = Command::new(&binary);
    command
        // The project directory, so a scene that opens `fixture.ttf` finds it. Running
        // from the repository root instead would resolve the relative path against
        // cargo's cwd, where the fixture is not.
        .current_dir(&project)
        .env("MFB_MACAPP_HEADLESS", "1")
        .env("MFB_WINAPP_HEADLESS", "1")
        .env("MFB_GTKAPP_HEADLESS", "1")
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
        "program exited {:?}:\n{}\n{}",
        run.status.code(),
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr),
    );

    let pixels = std::fs::read(&frame_path).expect("canvas dump written");
    let stats = std::fs::read_to_string(&stats_path).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&project);
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
    let plain = project.join("build").join(name);
    if plain.exists() {
        return plain;
    }
    project
        .join("build")
        .join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
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
IMPORT io

SUB main()
  app::setMode(app::Mode.Canvas)

  LET ground AS canvas::DrawItem = canvas::Rectangle[x := 0.0, y := 0.0, w := 900.0, h := 640.0, paint := canvas::fill(canvas::rgb(128, 128, 128))]

  LET warm AS canvas::Color = canvas::rgb(230, 120, 40)
  LET cool AS canvas::Color = canvas::rgb(40, 120, 230)

  LET boxNormal AS canvas::DrawItem = canvas::RoundedRect[x := 40.0, y := 80.0, w := 160.0, h := 160.0, cornerRadius := 24.0, paint := canvas::fillStroke(cool, canvas::rgb(255, 255, 255), 8.0)]
  LET dotNormal AS canvas::DrawItem = canvas::Circle[x := 160.0, y := 200.0, radius := 70.0, paint := WITH canvas::fill(warm) { blend := canvas::BlendMode.Normal }]

  LET boxMultiply AS canvas::DrawItem = canvas::RoundedRect[x := 260.0, y := 80.0, w := 160.0, h := 160.0, cornerRadius := 24.0, paint := canvas::fillStroke(cool, canvas::rgb(255, 255, 255), 8.0)]
  LET dotMultiply AS canvas::DrawItem = canvas::Circle[x := 380.0, y := 200.0, radius := 70.0, paint := WITH canvas::fill(warm) { blend := canvas::BlendMode.Multiply }]

  LET boxScreen AS canvas::DrawItem = canvas::RoundedRect[x := 480.0, y := 80.0, w := 160.0, h := 160.0, cornerRadius := 24.0, paint := canvas::fillStroke(cool, canvas::rgb(255, 255, 255), 8.0)]
  LET dotScreen AS canvas::DrawItem = canvas::Circle[x := 600.0, y := 200.0, radius := 70.0, paint := WITH canvas::fill(warm) { blend := canvas::BlendMode.Screen }]

  LET boxAdd AS canvas::DrawItem = canvas::RoundedRect[x := 700.0, y := 80.0, w := 160.0, h := 160.0, cornerRadius := 24.0, paint := canvas::fillStroke(cool, canvas::rgb(255, 255, 255), 8.0)]
  LET dotAdd AS canvas::DrawItem = canvas::Circle[x := 820.0, y := 200.0, radius := 70.0, paint := WITH canvas::fill(warm) { blend := canvas::BlendMode.Add }]

  ' A stroked arc under each mode too: a mode has to reach the stroke channel, not
  ' just the fill, and the stroke is the one that rides `salpha` rather than `alpha`.
  LET arcNormal AS canvas::DrawItem = canvas::Arc[x := 160.0, y := 450.0, radius := 80.0, startAngle := 0.0, endAngle := 3.14159, cap := canvas::CapStyle.Butt, paint := WITH canvas::stroke(warm, 16.0) { blend := canvas::BlendMode.Normal }]
  LET arcMultiply AS canvas::DrawItem = canvas::Arc[x := 380.0, y := 450.0, radius := 80.0, startAngle := 0.0, endAngle := 3.14159, cap := canvas::CapStyle.Butt, paint := WITH canvas::stroke(warm, 16.0) { blend := canvas::BlendMode.Multiply }]
  LET arcScreen AS canvas::DrawItem = canvas::Arc[x := 600.0, y := 450.0, radius := 80.0, startAngle := 0.0, endAngle := 3.14159, cap := canvas::CapStyle.Butt, paint := WITH canvas::stroke(warm, 16.0) { blend := canvas::BlendMode.Screen }]
  LET arcAdd AS canvas::DrawItem = canvas::Arc[x := 820.0, y := 450.0, radius := 80.0, startAngle := 0.0, endAngle := 3.14159, cap := canvas::CapStyle.Butt, paint := WITH canvas::stroke(warm, 16.0) { blend := canvas::BlendMode.Add }]

  ' One clipped item, so the reference covers the other half of this letter as well:
  ' a fractional clip edge that must stay antialiased.
  LET clipped AS canvas::DrawItem = canvas::Rectangle[x := 40.0, y := 560.0, w := 820.0, h := 60.0, paint := WITH canvas::fill(canvas::rgb(255, 255, 255)) { clip := canvas::Bounds[x := 100.25, y := 560.0, w := 700.5, h := 60.0] }]

  canvas::present([ground, boxNormal, dotNormal, boxMultiply, dotMultiply, boxScreen, dotScreen, boxAdd, dotAdd, arcNormal, arcMultiply, arcScreen, arcAdd, clipped])
  io::print("rendered")
END SUB
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
IMPORT io

SUB main()
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e)
    EXIT SUB
  END TRAP

  LET ground AS canvas::DrawItem = canvas::Rectangle[x := 0.0, y := 0.0, w := 900.0, h := 640.0, paint := canvas::fill(canvas::rgb(96, 96, 96))]

  ' 45 degrees: the hull is 1.41x the shape-space box in both axes.
  LET k AS Float = 0.7071067811865476
  LET rotT AS canvas::Transform = canvas::Transform[a := k, b := k, c := 0.0 - k, d := k, tx := 200.0, ty := 160.0]
  LET plainRect AS canvas::DrawItem = canvas::Rectangle[x := 480.0, y := 90.0, w := 140.0, h := 140.0, paint := canvas::fill(canvas::rgb(255, 190, 60))]
  LET rotRect AS canvas::DrawItem = canvas::Rectangle[x := 0.0 - 70.0, y := 0.0 - 70.0, w := 140.0, h := 140.0, paint := WITH canvas::fill(canvas::rgb(255, 190, 60)) { transform := rotT }]

  ' 2:1 in x only -- the case sqrt(|det M|) gets wrong.
  LET scaleT AS canvas::Transform = canvas::Transform[a := 2.0, b := 0.0, c := 0.0, d := 1.0, tx := 200.0, ty := 380.0]
  LET plainCircle AS canvas::DrawItem = canvas::Circle[x := 550.0, y := 380.0, radius := 60.0, paint := canvas::fillStroke(canvas::rgb(90, 200, 255), canvas::rgb(255, 255, 255), 8.0)]
  LET scaledCircle AS canvas::DrawItem = canvas::Circle[x := 0.0, y := 0.0, radius := 60.0, paint := WITH canvas::fillStroke(canvas::rgb(90, 200, 255), canvas::rgb(255, 255, 255), 8.0) { transform := scaleT }]

  ' A 30 degree shear in x.
  LET shearT AS canvas::Transform = canvas::Transform[a := 1.0, b := 0.0, c := 0.5773502691896258, d := 1.0, tx := 660.0, ty := 480.0]
  LET tri AS List OF canvas::Point = [canvas::Point[x := 0.0 - 60.0, y := 50.0], canvas::Point[x := 60.0, y := 50.0], canvas::Point[x := 0.0, y := 0.0 - 50.0]]
  LET plainPoly AS canvas::DrawItem = canvas::Polygon[points := [canvas::Point[x := 200.0, y := 590.0], canvas::Point[x := 320.0, y := 590.0], canvas::Point[x := 260.0, y := 490.0]], paint := canvas::fill(canvas::rgb(220, 120, 220))]
  LET shearPoly AS canvas::DrawItem = canvas::Polygon[points := tri, paint := WITH canvas::fill(canvas::rgb(220, 120, 220)) { transform := shearT }]

  ' 90 degrees, so the rotated run is a vertical column of the fixture's squares.
  LET textT AS canvas::Transform = canvas::Transform[a := 0.0, b := 1.0, c := 0.0 - 1.0, d := 0.0, tx := 860.0, ty := 120.0]
  LET plainText AS canvas::DrawItem = canvas::Text[x := 380.0, y := 300.0, text := "AA", font := canvas::fontRef(face), size := 50.0, paint := canvas::fill(canvas::rgb(200, 255, 120))]
  LET rotText AS canvas::DrawItem = canvas::Text[x := 0.0, y := 0.0, text := "AA", font := canvas::fontRef(face), size := 50.0, paint := WITH canvas::fill(canvas::rgb(200, 255, 120)) { transform := textT }]

  canvas::present([ground, plainRect, rotRect, plainCircle, scaledCircle, plainPoly, shearPoly, plainText, rotText])
  io::print("rendered")
END SUB
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
        stats.contains("gpuSelected=TRUE"),
        "the GPU pipeline built but the scene did not take it — a `*Renderable` \
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
