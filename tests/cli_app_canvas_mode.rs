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
    \x20 app::setMode(app::Mode.Canvas)\n\
    \x20 IF app::getMode() <> app::Mode.Canvas THEN\n\
    \x20   RETURN 1\n\
    \x20 END IF\n\
    \x20 app::setMode(app::Mode.None)\n\
    \x20 IF app::getMode() <> app::Mode.None THEN\n\
    \x20   RETURN 2\n\
    \x20 END IF\n\
    \x20 app::setMode(app::Mode.Canvas)\n\
    \x20 IF app::getMode() <> app::Mode.Canvas THEN\n\
    \x20   RETURN 3\n\
    \x20 END IF\n\
    \x20 RETURN 0\n\
     END FUNC\n";

/// The three variants must compare distinct — a wrong discriminant would make two
/// of them alias and this returns non-zero.
const CANVAS_DISTINCT_SOURCE: &str = "IMPORT app\n\
     FUNC main() AS Integer\n\
    \x20 app::setMode(app::Mode.Canvas)\n\
    \x20 IF app::getMode() = app::Mode.Console THEN\n\
    \x20   RETURN 1\n\
    \x20 END IF\n\
    \x20 IF app::getMode() = app::Mode.None THEN\n\
    \x20   RETURN 2\n\
    \x20 END IF\n\
    \x20 app::setMode(app::Mode.Console)\n\
    \x20 IF app::getMode() <> app::Mode.Console THEN\n\
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
    run_headless_with_stdin(exe, "").0
}

/// Run a bundle headlessly with `stdin` fed from a string; returns
/// `(exit code, stdout)`. Headless leaves fd 0 as real stdin and routes the `io::`
/// sink to fd 1 (no transcript view is attached), so both halves of the mode
/// contract — reads reaching the input path, writes degrading to stdout — are
/// observable here.
#[cfg(target_os = "macos")]
fn run_headless_with_stdin(exe: &Path, stdin: &str) -> (i32, String) {
    use std::io::Write;
    use std::process::Stdio;

    let mut child = Command::new(exe)
        .env("MFB_MACAPP_HEADLESS", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn headless app bundle");
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(stdin.as_bytes())
        .expect("feed stdin");
    let output = child.wait_with_output().expect("wait for headless bundle");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
    )
}

/// The runtime proof: the worker stores `2` into the presentation slot and reads
/// `2` back, across an intervening `None` excursion.
#[cfg(target_os = "macos")]
#[test]
fn macos_canvas_mode_round_trips_through_the_presentation_slot() {
    let (project, ok, log) = build_app("app_canvas_rt", CANVAS_ROUNDTRIP_SOURCE, &[]);
    assert!(ok, "a Mode.Canvas app build should succeed:\n{log}");
    let exe = project.join("build/app_canvas_rt.app/Contents/MacOS/app_canvas_rt");
    assert!(
        exe.is_file(),
        "expected app executable at {}",
        exe.display()
    );
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
    assert!(
        exe.is_file(),
        "expected app executable at {}",
        exe.display()
    );
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

// ---------------------------------------------------------------------------
// plan-98-A Phase 2 — the mode gate in `Canvas`.
//
// `term::` needs the transcript view's character grid, which a canvas surface
// (pixels, not cells) does not have, so it keeps the `Console`-only requirement
// and traps in `Canvas`. The console-read `io::` helpers need only an input
// source, and the canvas window has one, so their gate is relaxed to "trap only
// in `None`". `io::` writes were never gated and still are not.
// ---------------------------------------------------------------------------

/// `term::moveTo` must still trap `ErrWrongMode` in `Canvas` — the relaxation is
/// for `io::` reads only. Also checks `Console` still does *not* trap, so a gate
/// accidentally relaxed for `term::` too would fail here rather than pass silently.
#[cfg(target_os = "macos")]
const TERM_TRAPS_IN_CANVAS_SOURCE: &str = "IMPORT app\n\
     IMPORT term\n\
     IMPORT errorCode\n\
     FUNC main AS Integer\n\
    \x20 app::setMode(app::Mode.Canvas)\n\
    \x20 term::moveTo(1, 1) TRAP(err)\n\
    \x20   IF err.code <> errorCode::ErrWrongMode THEN\n\
    \x20     RETURN 60\n\
    \x20   END IF\n\
    \x20   app::setMode(app::Mode.Console)\n\
    \x20   term::moveTo(1, 1) TRAP(err2)\n\
    \x20     RETURN 61\n\
    \x20   END TRAP\n\
    \x20   RETURN 0\n\
    \x20 END TRAP\n\
    \x20 RETURN 50\n\
     END FUNC\n";

#[cfg(target_os = "macos")]
#[test]
fn macos_term_traps_wrong_mode_in_canvas() {
    let (project, ok, log) = build_app("app_canvas_term", TERM_TRAPS_IN_CANVAS_SOURCE, &[]);
    assert!(ok, "build should succeed:\n{log}");
    let exe = project.join("build/app_canvas_term.app/Contents/MacOS/app_canvas_term");
    let (code, _) = run_headless_with_stdin(&exe, "");
    assert_eq!(
        code, 0,
        "term:: must raise ErrWrongMode in Canvas (50 = did not trap, 60 = wrong \
         code, 61 = Console wrongly trapped)"
    );
    let _ = fs::remove_dir_all(&project);
}

/// The Phase 2 relaxation itself: `io::readLine` must **not** trap in `Canvas`
/// (`Canvas` has a window, so it has an input source), but must still trap in
/// `None` (no window, nowhere for input to come from). Both halves in one program
/// so a gate stuck at either extreme fails: always-trap fails the first half,
/// never-trap fails the second.
#[cfg(target_os = "macos")]
const IO_READ_GATE_SOURCE: &str = "IMPORT app\n\
     IMPORT io\n\
     IMPORT errorCode\n\
     FUNC main AS Integer\n\
    \x20 app::setMode(app::Mode.Canvas)\n\
    \x20 LET line AS String = io::readLine() TRAP(err)\n\
    \x20   IF err.code = errorCode::ErrWrongMode THEN\n\
    \x20     RETURN 50\n\
    \x20   END IF\n\
    \x20   RETURN 51\n\
    \x20 END TRAP\n\
    \x20 IF line <> \"canvas-input\" THEN\n\
    \x20   RETURN 52\n\
    \x20 END IF\n\
    \x20 app::setMode(app::Mode.None)\n\
    \x20 LET second AS String = io::readLine() TRAP(err2)\n\
    \x20   IF err2.code = errorCode::ErrWrongMode THEN\n\
    \x20     RETURN 0\n\
    \x20   END IF\n\
    \x20   RETURN 60\n\
    \x20 END TRAP\n\
    \x20 RETURN 61\n\
     END FUNC\n";

#[cfg(target_os = "macos")]
#[test]
fn macos_io_reads_are_permitted_in_canvas_and_still_trap_in_none() {
    let (project, ok, log) = build_app("app_canvas_read", IO_READ_GATE_SOURCE, &[]);
    assert!(ok, "build should succeed:\n{log}");
    let exe = project.join("build/app_canvas_read.app/Contents/MacOS/app_canvas_read");
    let (code, _) = run_headless_with_stdin(&exe, "canvas-input\nsecond-line\n");
    assert_eq!(
        code, 0,
        "io::readLine must read in Canvas (50 = wrongly trapped there) and still \
         trap in None (61 = wrongly permitted there)"
    );
    let _ = fs::remove_dir_all(&project);
}

/// `Console` reads are unchanged by the relaxation — the gate's fall-through case
/// still fires for mode 0.
#[cfg(target_os = "macos")]
const IO_READ_CONSOLE_SOURCE: &str = "IMPORT app\n\
     IMPORT io\n\
     FUNC main AS Integer\n\
    \x20 app::setMode(app::Mode.Console)\n\
    \x20 LET line AS String = io::readLine() TRAP(err)\n\
    \x20   RETURN 50\n\
    \x20 END TRAP\n\
    \x20 IF line <> \"console-input\" THEN\n\
    \x20   RETURN 51\n\
    \x20 END IF\n\
    \x20 RETURN 0\n\
     END FUNC\n";

#[cfg(target_os = "macos")]
#[test]
fn macos_console_reads_are_unchanged_by_the_relaxation() {
    let (project, ok, log) = build_app("app_canvas_console_read", IO_READ_CONSOLE_SOURCE, &[]);
    assert!(ok, "build should succeed:\n{log}");
    let exe =
        project.join("build/app_canvas_console_read.app/Contents/MacOS/app_canvas_console_read");
    let (code, _) = run_headless_with_stdin(&exe, "console-input\n");
    assert_eq!(code, 0, "a Console-mode io::readLine must still succeed");
    let _ = fs::remove_dir_all(&project);
}

/// `io::` writes are never gated: they degrade to the fd sink in every mode,
/// `Canvas` included.
#[cfg(target_os = "macos")]
const IO_WRITE_IN_CANVAS_SOURCE: &str = "IMPORT app\n\
     IMPORT io\n\
     FUNC main AS Integer\n\
    \x20 app::setMode(app::Mode.Canvas)\n\
    \x20 io::print(\"CANVAS_LINE\")\n\
    \x20 io::write(\"CANVAS_NONL\")\n\
    \x20 RETURN 0\n\
     END FUNC\n";

#[cfg(target_os = "macos")]
#[test]
fn macos_io_writes_degrade_to_stdout_in_canvas() {
    let (project, ok, log) = build_app("app_canvas_write", IO_WRITE_IN_CANVAS_SOURCE, &[]);
    assert!(ok, "build should succeed:\n{log}");
    let exe = project.join("build/app_canvas_write.app/Contents/MacOS/app_canvas_write");
    let (code, stdout) = run_headless_with_stdin(&exe, "");
    assert_eq!(code, 0, "io:: writes must not trap in Canvas");
    assert_eq!(
        stdout, "CANVAS_LINE\nCANVAS_NONL",
        "io::print/io::write must degrade to stdout in Canvas"
    );
    let _ = fs::remove_dir_all(&project);
}

// ---------------------------------------------------------------------------
// plan-98-A Phase 4 — canvas input.
//
// Two halves, and they need different harnesses:
//
//  - The READ CONTRACT (bytes arrive in order, EOF ends the stream) is testable
//    headless, because headless leaves fd 0 as the real stdin. That is what the
//    case below does.
//  - The WINDOW WIRING (the canvas view actually becomes first responder and its
//    keyDown: reaches the pipe) is not: headless installs no delegate, so the
//    reconcile never runs and no canvas view is ever built. That half is
//    `scripts/test-macapp.sh` Case 6b, which injects real keystrokes into a real
//    window via System Events.
// ---------------------------------------------------------------------------

/// Reads four bytes in `Mode.Canvas` and reports the first mismatch by position, so
/// a reordering fails differently from a wrong value. Then asserts the fifth read
/// hits EOF, which the runtime reports as `ErrEndOfFile` exactly as it does for a
/// console program whose stdin closed.
#[cfg(target_os = "macos")]
const CANVAS_READ_ORDER_SOURCE: &str = "IMPORT app\n\
     IMPORT io\n\
     IMPORT errorCode\n\
     FUNC main AS Integer\n\
    \x20 app::setMode(app::Mode.Canvas)\n\
    \x20 LET a AS Byte = io::readByte()\n\
    \x20 IF a <> 65 THEN\n\
    \x20   RETURN 10\n\
    \x20 END IF\n\
    \x20 LET b AS Byte = io::readByte()\n\
    \x20 IF b <> 66 THEN\n\
    \x20   RETURN 11\n\
    \x20 END IF\n\
    \x20 LET c AS Byte = io::readByte()\n\
    \x20 IF c <> 67 THEN\n\
    \x20   RETURN 12\n\
    \x20 END IF\n\
    \x20 LET d AS Byte = io::readByte()\n\
    \x20 IF d <> 10 THEN\n\
    \x20   RETURN 13\n\
    \x20 END IF\n\
    \x20 LET e AS Byte = io::readByte() TRAP(err)\n\
    \x20   IF err.code = errorCode::ErrEndOfFile THEN\n\
    \x20     RETURN 0\n\
    \x20   END IF\n\
    \x20   RETURN 20\n\
    \x20 END TRAP\n\
    \x20 RETURN 21\n\
     END FUNC\n";

#[cfg(target_os = "macos")]
#[test]
fn macos_canvas_readbyte_returns_bytes_in_order_then_eof() {
    let (project, ok, log) = build_app("app_canvas_bytes", CANVAS_READ_ORDER_SOURCE, &[]);
    assert!(ok, "build should succeed:\n{log}");
    let exe = project.join("build/app_canvas_bytes.app/Contents/MacOS/app_canvas_bytes");
    let (code, _) = run_headless_with_stdin(&exe, "ABC\n");
    assert_eq!(
        code, 0,
        "io::readByte in Canvas must return A, B, C, LF in order (10-13 = wrong \
         byte at that position) and then EOF (21 = no EOF, 20 = wrong error)"
    );
    let _ = fs::remove_dir_all(&project);
}
