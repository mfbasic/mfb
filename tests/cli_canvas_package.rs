//! The `canvas::` builtin package's language-visible surface (plan-98-B Phase 1).
//!
//! These drive the real `mfb` CLI and, on macOS, run the produced bundle headless.
//! They exist because the whole type surface is registry *data* — a wrong field
//! type, a name collision, or a variant that cannot be constructed is invisible to
//! the Rust unit tests (which only inspect the descriptors) and shows up only when
//! a program actually names the types.

mod common;
use common::temp_project;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn build(name: &str, source: &str, app: bool) -> (PathBuf, bool, String) {
    let project = temp_project(name, source);
    let mut command = Command::new(common::mfb_exe());
    command.arg("build");
    if app {
        command.arg("-app");
    }
    let output = command.arg(&project).output().expect("run mfb build");
    let combined = format!(
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (project, output.status.success(), combined)
}

/// Every value type, every `DrawItem` variant, both `Paint` constructors and both
/// colour constructors, exercised from source. Exit codes name what failed so a
/// regression says which property broke rather than just "non-zero".
const SURFACE_SOURCE: &str = "IMPORT app\n\
     IMPORT canvas\n\
     IMPORT io\n\
     FUNC main AS Integer\n\
    \x20 LET yellow AS canvas::Color = canvas::rgb(255, 255, 0)\n\
    \x20 LET green AS canvas::Color = canvas::rgb(0, 160, 0)\n\
    \x20 LET clamped AS canvas::Color = canvas::rgba(300, -20, 128, 255)\n\
    \x20 IF yellow.red <> toByte(255) THEN\n\
    \x20   RETURN 1\n\
    \x20 END IF\n\
    \x20 IF yellow.alpha <> toByte(255) THEN\n\
    \x20   RETURN 2\n\
    \x20 END IF\n\
    \x20 IF clamped.red <> toByte(255) THEN\n\
    \x20   RETURN 3\n\
    \x20 END IF\n\
    \x20 IF clamped.green <> toByte(0) THEN\n\
    \x20   RETURN 4\n\
    \x20 END IF\n\
    \x20 LET pts AS List OF canvas::Point = [canvas::Point[x := 0.0, y := 0.0], canvas::Point[x := 1.0, y := 0.0]]\n\
    \x20 LET img AS canvas::ImageRef = canvas::ImageRef[id := 0]\n\
    \x20 LET fnt AS canvas::FontRef = canvas::FontRef[id := 0]\n\
    \x20 LET a AS canvas::DrawItem = canvas::Circle[x := 1.0, y := 2.0, radius := 3.0, paint := canvas::fill(yellow)]\n\
    \x20 LET b AS canvas::DrawItem = canvas::Arc[x := 1.0, y := 2.0, radius := 3.0, startAngle := 0.0, endAngle := 3.14159, paint := canvas::stroke(green, 4.0)]\n\
    \x20 LET c AS canvas::DrawItem = canvas::Rectangle[x := 0.0, y := 0.0, w := 10.0, h := 10.0, paint := canvas::fill(green)]\n\
    \x20 LET d AS canvas::DrawItem = canvas::Line[x1 := 0.0, y1 := 0.0, x2 := 5.0, y2 := 5.0, paint := canvas::stroke(green, 1.0)]\n\
    \x20 LET e AS canvas::DrawItem = canvas::Polygon[points := pts, paint := canvas::fill(yellow)]\n\
    \x20 LET f AS canvas::DrawItem = canvas::RoundedRect[x := 0.0, y := 0.0, w := 4.0, h := 4.0, cornerRadius := 1.0, paint := canvas::fill(yellow)]\n\
    \x20 LET g AS canvas::DrawItem = canvas::Text[x := 0.0, y := 0.0, text := \"hi\", font := fnt, size := 12.0, paint := canvas::fill(green)]\n\
    \x20 LET h AS canvas::DrawItem = canvas::Picture[x := 0.0, y := 0.0, w := 8.0, h := 8.0, image := img, paint := canvas::fillStroke(yellow, green, 1.0)]\n\
    \x20 LET scene AS List OF canvas::DrawItem = [a, b, c, d, e, f, g, h]\n\
    \x20 IF len(scene) <> 8 THEN\n\
    \x20   RETURN 5\n\
    \x20 END IF\n\
    \x20 LET layer AS canvas::DrawLayer = canvas::DrawLayer[items := scene]\n\
    \x20 IF len(layer.items) <> 8 THEN\n\
    \x20   RETURN 6\n\
    \x20 END IF\n\
    \x20 io::print(\"CANVAS_SURFACE_OK\")\n\
    \x20 RETURN 0\n\
     END FUNC\n";

/// Each `Paint` field's zero value must be that field's no-op — the rule that lets
/// `canvas::fill(c)` mean "just a filled shape" without the caller naming five more
/// fields. Checked through the constructors, since that is how a program gets one.
///
/// Gated to macOS with its one consumer, `macos_paint_zero_values_are_no_ops`, which
/// runs an `.app` bundle. Without the gate this is dead code on every other host and
/// warns there — invisible from a macOS development host, and noise on the Linux CI
/// axis for every canvas change.
#[cfg(target_os = "macos")]
const PAINT_DEFAULTS_SOURCE: &str = "IMPORT app\n\
     IMPORT canvas\n\
     FUNC main AS Integer\n\
    \x20 LET red AS canvas::Color = canvas::rgb(255, 0, 0)\n\
    \x20 LET filled AS canvas::Paint = canvas::fill(red)\n\
    \x20 IF filled.stroke.alpha <> toByte(0) THEN\n\
    \x20   RETURN 1\n\
    \x20 END IF\n\
    \x20 IF filled.strokeWidth <> 0.0 THEN\n\
    \x20   RETURN 2\n\
    \x20 END IF\n\
    \x20 IF filled.blend <> canvas::BlendMode.Normal THEN\n\
    \x20   RETURN 3\n\
    \x20 END IF\n\
    \x20 IF filled.transform.a <> 0.0 THEN\n\
    \x20   RETURN 4\n\
    \x20 END IF\n\
    \x20 IF filled.clip.w <> 0.0 THEN\n\
    \x20   RETURN 5\n\
    \x20 END IF\n\
    \x20 LET outlined AS canvas::Paint = canvas::stroke(red, 3.0)\n\
    \x20 IF outlined.fill.alpha <> toByte(0) THEN\n\
    \x20   RETURN 6\n\
    \x20 END IF\n\
    \x20 IF outlined.strokeWidth <> 3.0 THEN\n\
    \x20   RETURN 7\n\
    \x20 END IF\n\
    \x20 LET both AS canvas::Paint = canvas::fillStroke(red, red, 2.0)\n\
    \x20 IF both.fill.red <> toByte(255) THEN\n\
    \x20   RETURN 8\n\
    \x20 END IF\n\
    \x20 IF both.stroke.red <> toByte(255) THEN\n\
    \x20   RETURN 9\n\
    \x20 END IF\n\
    \x20 ' A WITH update is how the advanced fields are set.\n\
    \x20 LET added AS canvas::Paint = WITH filled { blend := canvas::BlendMode.Add }\n\
    \x20 IF added.blend <> canvas::BlendMode.Add THEN\n\
    \x20   RETURN 10\n\
    \x20 END IF\n\
    \x20 IF added.fill.red <> toByte(255) THEN\n\
    \x20   RETURN 11\n\
    \x20 END IF\n\
    \x20 RETURN 0\n\
     END FUNC\n";

/// `canvas` draws on a window surface a console binary does not have, so importing
/// it outside an `--app` build is a compile error — the same gate `app` has.
const CONSOLE_IMPORT_SOURCE: &str = "IMPORT canvas\n\
     FUNC main AS Integer\n\
    \x20 LET c AS canvas::Color = canvas::rgb(1, 2, 3)\n\
    \x20 RETURN toInteger(c.red)\n\
     END FUNC\n";

#[test]
fn canvas_surface_compiles_in_an_app_build() {
    let (project, ok, log) = build("canvas_surface", SURFACE_SOURCE, true);
    assert!(ok, "the canvas type surface should compile:\n{log}");
    let _ = fs::remove_dir_all(&project);
}

#[test]
fn canvas_import_is_rejected_in_a_console_build() {
    let (project, ok, log) = build("canvas_console", CONSOLE_IMPORT_SOURCE, false);
    assert!(!ok, "a console build importing canvas must fail:\n{log}");
    assert!(
        log.contains("the `canvas` package requires app mode"),
        "the rejection must name `canvas`, not `app`:\n{log}"
    );
    let _ = fs::remove_dir_all(&project);
}

/// The GTK backend must accept the package too — the types are backend-neutral
/// registry data, so a target that rejected them would be rejecting the package.
#[test]
fn canvas_surface_compiles_for_the_linux_app_target() {
    let project = temp_project("canvas_linux", SURFACE_SOURCE);
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg("-app")
        .arg("-target")
        .arg("linux-aarch64")
        .arg(&project)
        .output()
        .expect("run mfb build");
    assert!(
        output.status.success(),
        "linux-aarch64 canvas build failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let _ = fs::remove_dir_all(&project);
}

#[cfg(target_os = "macos")]
fn run_headless(exe: &std::path::Path) -> (i32, String) {
    let output = Command::new(exe)
        .env("MFB_MACAPP_HEADLESS", "1")
        .output()
        .expect("run headless app bundle");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
    )
}

#[cfg(target_os = "macos")]
#[test]
fn macos_canvas_surface_runs() {
    let (project, ok, log) = build("canvas_surface_rt", SURFACE_SOURCE, true);
    assert!(ok, "build should succeed:\n{log}");
    let exe = project.join("build/canvas_surface_rt.app/Contents/MacOS/canvas_surface_rt");
    let (code, stdout) = run_headless(&exe);
    assert_eq!(code, 0, "canvas type surface check failed with code {code}");
    assert_eq!(stdout, "CANVAS_SURFACE_OK\n");
    let _ = fs::remove_dir_all(&project);
}

/// Every path through `present`'s skip/publish branch, in one program: first
/// present (nothing installed), identical re-present (skip), different content
/// (publish), back to the first content (publish — the comparison is against what
/// is *currently* installed, not anything remembered), an empty scene both ways,
/// and non-empty again.
///
/// A runtime test cannot see *whether* a frame was skipped — nothing reads the
/// published scene until plan-98-D — so what this covers is that the branch itself
/// is sound on every shape, including the empty list and the size-changed case that
/// the byte comparison has to short-circuit rather than read past.
#[cfg(target_os = "macos")]
const PRESENT_SKIP_SOURCE: &str = "IMPORT app\n\
     IMPORT canvas\n\
     FUNC one(r AS Float) AS List OF canvas::DrawItem\n\
    \x20 LET c AS canvas::Color = canvas::rgb(10, 20, 30)\n\
    \x20 LET a AS canvas::DrawItem = canvas::Circle[x := 1.0, y := 2.0, radius := r, paint := canvas::fill(c)]\n\
    \x20 LET b AS canvas::DrawItem = canvas::Text[x := 0.0, y := 0.0, text := \"abc\", font := canvas::FontRef[id := 3], size := 8.0, paint := canvas::fill(c)]\n\
    \x20 RETURN [a, b]\n\
     END FUNC\n\
     FUNC main AS Integer\n\
    \x20 app::setMode(app::Mode.Canvas)\n\
    \x20 canvas::present(one(5.0))\n\
    \x20 canvas::present(one(5.0))\n\
    \x20 canvas::present(one(7.0))\n\
    \x20 canvas::present(one(7.0))\n\
    \x20 canvas::present(one(5.0))\n\
    \x20 LET empty AS List OF canvas::DrawItem = []\n\
    \x20 canvas::present(empty)\n\
    \x20 canvas::present(empty)\n\
    \x20 canvas::present(one(5.0))\n\
    \x20 RETURN 0\n\
     END FUNC\n";

#[cfg(target_os = "macos")]
#[test]
fn macos_repeated_and_changed_presents_are_sound() {
    let (project, ok, log) = build("canvas_present_skip_rt", PRESENT_SKIP_SOURCE, true);
    assert!(ok, "build should succeed:\n{log}");
    let exe =
        project.join("build/canvas_present_skip_rt.app/Contents/MacOS/canvas_present_skip_rt");
    let (code, _) = run_headless(&exe);
    assert_eq!(
        code, 0,
        "repeated, changed, and empty presents must all be sound"
    );
    let _ = fs::remove_dir_all(&project);
}

/// `presentLayers` installs the same scene in a different shape, and a scene is
/// exactly one shape at a time. The interesting cases are the transitions: switching
/// shapes must always publish (the scene really did change), and an empty layer list
/// is a valid scene rather than a degenerate one.
#[cfg(target_os = "macos")]
const PRESENT_LAYERS_SOURCE: &str = "IMPORT app\n\
     IMPORT canvas\n\
     IMPORT errorCode\n\
     FUNC layers(r AS Float) AS List OF canvas::DrawLayer\n\
    \x20 LET sky AS canvas::Color = canvas::rgb(20, 30, 60)\n\
    \x20 LET dot AS canvas::Color = canvas::rgb(255, 200, 0)\n\
    \x20 LET backdrop AS canvas::DrawItem = canvas::Rectangle[x := 0.0, y := 0.0, w := 400.0, h := 300.0, paint := canvas::fill(sky)]\n\
    \x20 LET marker AS canvas::DrawItem = canvas::Circle[x := 100.0, y := 150.0, radius := r, paint := canvas::fill(dot)]\n\
    \x20 LET back AS canvas::DrawLayer = canvas::DrawLayer[items := [backdrop]]\n\
    \x20 LET front AS canvas::DrawLayer = canvas::DrawLayer[items := [marker]]\n\
    \x20 RETURN [back, front]\n\
     END FUNC\n\
     FUNC flat() AS List OF canvas::DrawItem\n\
    \x20 LET dot AS canvas::Color = canvas::rgb(255, 200, 0)\n\
    \x20 LET marker AS canvas::DrawItem = canvas::Circle[x := 1.0, y := 2.0, radius := 3.0, paint := canvas::fill(dot)]\n\
    \x20 RETURN [marker]\n\
     END FUNC\n\
     FUNC main AS Integer\n\
    \x20 LET none AS List OF canvas::DrawLayer = []\n\
    \x20 canvas::presentLayers(none) TRAP(err)\n\
    \x20   IF err.code <> errorCode::ErrWrongMode THEN\n\
    \x20     RETURN 60\n\
    \x20   END IF\n\
    \x20   app::setMode(app::Mode.Canvas)\n\
    \x20   canvas::presentLayers(layers(12.0))\n\
    \x20   canvas::presentLayers(layers(12.0))\n\
    \x20   canvas::presentLayers(layers(20.0))\n\
    \x20   canvas::present(flat())\n\
    \x20   canvas::present(flat())\n\
    \x20   canvas::presentLayers(layers(12.0))\n\
    \x20   LET empty AS List OF canvas::DrawLayer = []\n\
    \x20   canvas::presentLayers(empty)\n\
    \x20   canvas::presentLayers(empty)\n\
    \x20   canvas::presentLayers(layers(12.0))\n\
    \x20   RETURN 0\n\
    \x20 END TRAP\n\
    \x20 RETURN 50\n\
     END FUNC\n";

#[cfg(target_os = "macos")]
#[test]
fn macos_present_layers_and_shape_switching_are_sound() {
    let (project, ok, log) = build("canvas_layers_rt", PRESENT_LAYERS_SOURCE, true);
    assert!(ok, "build should succeed:\n{log}");
    let exe = project.join("build/canvas_layers_rt.app/Contents/MacOS/canvas_layers_rt");
    let (code, _) = run_headless(&exe);
    assert_eq!(
        code, 0,
        "presentLayers must trap outside canvas mode (50 = did not, 60 = wrong \
         code) and be sound across repeats, changes, shape switches and empty scenes"
    );
    let _ = fs::remove_dir_all(&project);
}

#[cfg(target_os = "macos")]
#[test]
fn macos_paint_zero_values_are_no_ops() {
    let (project, ok, log) = build("canvas_paint", PAINT_DEFAULTS_SOURCE, true);
    assert!(ok, "build should succeed:\n{log}");
    let exe = project.join("build/canvas_paint.app/Contents/MacOS/canvas_paint");
    let (code, _) = run_headless(&exe);
    assert_eq!(
        code, 0,
        "every unnamed Paint field must be its own no-op (code names the field)"
    );
    let _ = fs::remove_dir_all(&project);
}
