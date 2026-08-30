//! `Mode.Canvas` presentation-mode plumbing (plan-98-A).
//!
//! `Canvas` is the third `Mode` enum variant (`Console` = 0, `None` = 1,
//! `Canvas` = 2). Variant declaration order fixes the discriminants and those are
//! the values `app::setMode`/`app::getMode` store into and load from the
//! presentation-mode arena slot, so appending `Canvas` must round-trip through
//! that slot with no per-variant codegen at all.
//!
//! These cases drive the real `mfb` CLI. The macOS ones additionally *run* the
//! produced bundle under `MFB_MACAPP_HEADLESS=1` (the same AppKit construction +
//! worker-thread path the GUI build takes, minus the window and the run loop), so
//! the round-trip is proven at runtime and not merely at compile time.

mod common;
use common::temp_project;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Sets `Canvas`, reads it back, then leaves via `None` — exit 0 only if every
/// observation matched. Written as exit codes rather than printed text because
/// headless app-mode stdout is the fd sink, and an exit code cannot be confused
/// with a partial write.
const CANVAS_ROUNDTRIP_SOURCE: &str = "IMPORT app\n\
     FUNC main() AS Integer\n\
    \x20 app::setMode(Mode.Canvas)\n\
    \x20 IF app::getMode() <> Mode.Canvas THEN\n\
    \x20   RETURN 1\n\
    \x20 END IF\n\
    \x20 app::setMode(Mode.None)\n\
    \x20 IF app::getMode() <> Mode.None THEN\n\
    \x20   RETURN 2\n\
    \x20 END IF\n\
    \x20 app::setMode(Mode.Canvas)\n\
    \x20 IF app::getMode() <> Mode.Canvas THEN\n\
    \x20   RETURN 3\n\
    \x20 END IF\n\
    \x20 RETURN 0\n\
     END FUNC\n";

/// The three variants must compare distinct — a wrong discriminant would make two
/// of them alias and this returns non-zero.
const CANVAS_DISTINCT_SOURCE: &str = "IMPORT app\n\
     FUNC main() AS Integer\n\
    \x20 app::setMode(Mode.Canvas)\n\
    \x20 IF app::getMode() = Mode.Console THEN\n\
    \x20   RETURN 1\n\
    \x20 END IF\n\
    \x20 IF app::getMode() = Mode.None THEN\n\
    \x20   RETURN 2\n\
    \x20 END IF\n\
    \x20 app::setMode(Mode.Console)\n\
    \x20 IF app::getMode() <> Mode.Console THEN\n\
    \x20   RETURN 3\n\
    \x20 END IF\n\
    \x20 RETURN 0\n\
     END FUNC\n";

fn build_app(name: &str, source: &str, extra: &[&str]) -> (PathBuf, bool, String) {
    let project = temp_project(name, source);
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg("-app")
        .args(extra)
        .arg(&project)
        .output()
        .expect("run mfb build -app");
    let combined = format!(
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (project, output.status.success(), combined)
}

/// Every `--app` target must accept `Mode.Canvas` at compile time: the variant is
/// registry data with no per-backend surface of its own, so a backend that
/// rejected it would be rejecting the whole `Mode` enum.
#[test]
fn canvas_mode_compiles_for_the_host_app_target() {
    let (project, ok, log) = build_app("app_canvas_build", CANVAS_ROUNDTRIP_SOURCE, &[]);
    assert!(ok, "a Mode.Canvas app build should succeed:\n{log}");
    let _ = fs::remove_dir_all(&project);
}

#[cfg(target_os = "macos")]
fn run_headless(exe: &Path) -> i32 {
    let output = Command::new(exe)
        .env("MFB_MACAPP_HEADLESS", "1")
        .output()
        .expect("run headless app bundle");
    output.status.code().unwrap_or(-1)
}

/// The runtime proof: the worker stores `2` into the presentation slot and reads
/// `2` back, across an intervening `None` excursion.
#[cfg(target_os = "macos")]
#[test]
fn macos_canvas_mode_round_trips_through_the_presentation_slot() {
    let (project, ok, log) = build_app("app_canvas_rt", CANVAS_ROUNDTRIP_SOURCE, &[]);
    assert!(ok, "a Mode.Canvas app build should succeed:\n{log}");
    let exe = project.join("build/app_canvas_rt.app/Contents/MacOS/app_canvas_rt");
    assert!(exe.is_file(), "expected app executable at {}", exe.display());
    assert_eq!(
        run_headless(&exe),
        0,
        "Canvas -> None -> Canvas must round-trip through the mode slot"
    );
    let _ = fs::remove_dir_all(&project);
}

/// `Canvas` must not alias `Console` (0) or `None` (1) — the appended-variant
/// slot-safety claim, checked at runtime rather than inferred from source order.
#[cfg(target_os = "macos")]
#[test]
fn macos_canvas_is_distinct_from_console_and_none() {
    let (project, ok, log) = build_app("app_canvas_distinct", CANVAS_DISTINCT_SOURCE, &[]);
    assert!(ok, "a Mode.Canvas app build should succeed:\n{log}");
    let exe = project.join("build/app_canvas_distinct.app/Contents/MacOS/app_canvas_distinct");
    assert!(exe.is_file(), "expected app executable at {}", exe.display());
    assert_eq!(
        run_headless(&exe),
        0,
        "Canvas must compare distinct from Console and None"
    );
    let _ = fs::remove_dir_all(&project);
}

/// Cross-compiled Linux app build: the GTK backend advertises `app.setMode` in
/// its `runtime_calls`, so a `Canvas` program must pass capability validation
/// there too. Build-only — the host cannot execute a Linux aarch64 GTK binary.
#[test]
fn linux_app_target_accepts_canvas_mode() {
    let (project, ok, log) = build_app(
        "app_canvas_linux",
        CANVAS_ROUNDTRIP_SOURCE,
        &["-target", "linux-aarch64"],
    );
    assert!(
        ok,
        "a Mode.Canvas app build for linux-aarch64 should succeed:\n{log}"
    );
    let _ = fs::remove_dir_all(&project);
}
