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
  app::setMode(Mode.Canvas)

  LET yellow AS Color = canvas::rgb(255, 255, 0)
  LET green AS Color = canvas::rgb(0, 160, 0)

  LET canvasSize AS Size = canvas::getSize()
  LET cx AS Float = toFloat(canvasSize.width) / 2.0
  LET cy AS Float = toFloat(canvasSize.height) / 2.0

  LET face AS DrawItem = Circle[x := cx, y := cy, radius := 150.0, paint := canvas::fill(yellow)]
  LET eyeL AS DrawItem = Circle[x := cx - 50.0, y := cy - 40.0, radius := 22.0, paint := canvas::fill(green)]
  LET eyeR AS DrawItem = Circle[x := cx + 50.0, y := cy - 40.0, radius := 22.0, paint := canvas::fill(green)]
  LET smile AS DrawItem = Arc[x := cx, y := cy + 15.0, radius := 90.0, startAngle := 0.0, endAngle := 3.14159, paint := canvas::stroke(green, 14.0)]

  LET scene AS List OF DrawItem = [face, eyeL, eyeR, smile]

  canvas::present(scene)
  io::print("rendered")
END SUB
"#;

fn golden_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join("canvas")
        .join(format!("{name}.png"))
}

/// Build and run a `--app` program headless, returning its rendered frame.
fn render(name: &str, source: &str) -> Frame {
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

    let frame_path = project.join("frame.rgba");
    let binary = app_binary(&project, name);
    let run = Command::new(&binary)
        .env("MFB_MACAPP_HEADLESS", "1")
        .env("MFB_WINAPP_HEADLESS", "1")
        .env("MFB_CANVAS_DUMP", &frame_path)
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
    let _ = std::fs::remove_dir_all(&project);
    Frame::from_rgba(WIDTH, HEIGHT, pixels)
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
