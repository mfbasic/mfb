//! `app::Mode.Canvas` presentation-mode plumbing (plan-98-A).
//!
//! `Canvas` is the third `Mode` enum variant (`Console` = 0, `None` = 1,
//! `Canvas` = 2). Variant declaration order fixes the discriminants and those are
//! the values `app::setMode`/`app::getMode` store into and load from the
//! presentation-mode arena slot, so appending `Canvas` must round-trip through
//! that slot with no per-variant codegen at all.
//!
//! These cases drive the real `mfb` CLI. The macOS ones additionally *run* the
//! produced bundle under `MFB_HEADLESS=1` (the same AppKit construction +
//! worker-thread path the GUI build takes, minus the window and the run loop), so
//! the round-trip is proven at runtime and not merely at compile time.

#[path = "../common/mod.rs"]
mod common;
use common::temp_project;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
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
#[cfg(target_os = "macos")]
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

/// Every `--app` target must accept `app::Mode.Canvas` at compile time: the variant is
/// registry data with no per-backend surface of its own, so a backend that
/// rejected it would be rejecting the whole `Mode` enum.
#[test]
fn canvas_mode_compiles_for_the_host_app_target() {
    let (project, ok, log) = build_app("app_canvas_build", CANVAS_ROUNDTRIP_SOURCE, &[]);
    assert!(ok, "an app::Mode.Canvas app build should succeed:\n{log}");
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
        .env("MFB_HEADLESS", "1")
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
    assert!(ok, "an app::Mode.Canvas app build should succeed:\n{log}");
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
    assert!(ok, "an app::Mode.Canvas app build should succeed:\n{log}");
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
        "an app::Mode.Canvas app build for linux-aarch64 should succeed:\n{log}"
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
    \x20 LET measured AS Integer = term::measureText(\"x\") TRAP(measureErr)\n\
    \x20   IF measureErr.code <> errorCode::ErrWrongMode THEN\n\
    \x20     RETURN 62\n\
    \x20   END IF\n\
    \x20   RECOVER -1\n\
    \x20 END TRAP\n\
    \x20 IF measured <> -1 THEN\n\
    \x20   RETURN 63\n\
    \x20 END IF\n\
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

/// Reads four bytes in `app::Mode.Canvas` and reports the first mismatch by position, so
/// a reordering fails differently from a wrong value. Then asserts the fifth read
/// hits EOF, which the runtime reports as `ErrEndOfFile` exactly as it does for a
/// console program whose stdin closed.
#[cfg(target_os = "macos")]
const CANVAS_READ_ORDER_SOURCE: &str = "IMPORT app\n\
     IMPORT io\n\
     IMPORT errorCode\n\
     FUNC main AS Integer\n\
    \x20 app::setMode(app::Mode.Canvas)\n\
    \x20 LET a AS Byte = io::readRaw()\n\
    \x20 IF a <> 65 THEN\n\
    \x20   RETURN 10\n\
    \x20 END IF\n\
    \x20 LET b AS Byte = io::readRaw()\n\
    \x20 IF b <> 66 THEN\n\
    \x20   RETURN 11\n\
    \x20 END IF\n\
    \x20 LET c AS Byte = io::readRaw()\n\
    \x20 IF c <> 67 THEN\n\
    \x20   RETURN 12\n\
    \x20 END IF\n\
    \x20 LET d AS Byte = io::readRaw()\n\
    \x20 IF d <> 10 THEN\n\
    \x20   RETURN 13\n\
    \x20 END IF\n\
    \x20 LET e AS Byte = io::readRaw() TRAP(err)\n\
    \x20   IF err.code = errorCode::ErrEndOfFile THEN\n\
    \x20     RETURN 0\n\
    \x20   END IF\n\
    \x20   RETURN 20\n\
    \x20 END TRAP\n\
    \x20 RETURN 21\n\
     END FUNC\n";

#[cfg(target_os = "macos")]
#[test]
fn macos_canvas_readraw_returns_bytes_in_order_then_eof() {
    let (project, ok, log) = build_app("app_canvas_bytes", CANVAS_READ_ORDER_SOURCE, &[]);
    assert!(ok, "build should succeed:\n{log}");
    let exe = project.join("build/app_canvas_bytes.app/Contents/MacOS/app_canvas_bytes");
    let (code, _) = run_headless_with_stdin(&exe, "ABC\n");
    assert_eq!(
        code, 0,
        "io::readRaw in Canvas must return A, B, C, LF in order (10-13 = wrong \
         byte at that position) and then EOF (21 = no EOF, 20 = wrong error)"
    );
    let _ = fs::remove_dir_all(&project);
}

/// Both group members trap `ErrWrongMode` outside `Canvas` (plan-116-G Phase 3).
///
/// The gate is spliced ahead of everything the body does, so a wrong-mode `setGroup`
/// returns having allocated nothing and touched no slot — which matters more here than
/// for a member that only reads, because this one copies two blocks and writes into a
/// process-global table before it would otherwise notice.
///
/// `removeGroup` is checked in the same program: it is the member whose whole contract
/// is "does nothing when there is nothing to do", so a gate that let it through would
/// be invisible from its behaviour alone.
#[cfg(target_os = "macos")]
const GROUPS_TRAP_OUTSIDE_CANVAS_SOURCE: &str = "IMPORT app\n\
     IMPORT canvas\n\
     IMPORT vector\n\
     IMPORT color\n\
     IMPORT errorCode\n\
     FUNC main AS Integer\n\
    \x20 LET c AS color::Color = color::rgb(1, 2, 3)\n\
    \x20 LET a AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 4.0, height := 4.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(c), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n\
    \x20 canvas::setGroup(\"panel\", [a]) TRAP(err)\n\
    \x20   IF err.code <> errorCode::ErrWrongMode THEN\n\
    \x20     RETURN 60\n\
    \x20   END IF\n\
    \x20   canvas::removeGroup(\"panel\") TRAP(err2)\n\
    \x20     IF err2.code <> errorCode::ErrWrongMode THEN\n\
    \x20       RETURN 62\n\
    \x20     END IF\n\
    \x20     app::setMode(app::Mode.Canvas)\n\
    \x20     canvas::setGroup(\"panel\", [a]) TRAP(err3)\n\
    \x20       RETURN 63\n\
    \x20     END TRAP\n\
    \x20     canvas::removeGroup(\"panel\") TRAP(err4)\n\
    \x20       RETURN 64\n\
    \x20     END TRAP\n\
    \x20     RETURN 0\n\
    \x20   END TRAP\n\
    \x20   RETURN 51\n\
    \x20 END TRAP\n\
    \x20 RETURN 50\n\
     END FUNC\n";

#[cfg(target_os = "macos")]
#[test]
fn macos_group_members_trap_wrong_mode_outside_canvas() {
    let (project, ok, log) = build_app(
        "app_canvas_groups_mode",
        GROUPS_TRAP_OUTSIDE_CANVAS_SOURCE,
        &[],
    );
    assert!(ok, "build should succeed:\n{log}");
    let exe =
        project.join("build/app_canvas_groups_mode.app/Contents/MacOS/app_canvas_groups_mode");
    let (code, _) = run_headless_with_stdin(&exe, "");
    assert_eq!(
        code, 0,
        "canvas::setGroup and canvas::removeGroup must raise ErrWrongMode outside \
         Canvas and succeed inside it (50 = setGroup did not trap, 51 = removeGroup \
         did not trap, 60/62 = wrong code, 63/64 = wrongly trapped in Canvas)"
    );
    let _ = fs::remove_dir_all(&project);
}

/// `setGroup` past `CANVAS_MAX_GROUPS` raises `ErrCanvasGroupLimit` rather than
/// evicting (plan-116-G Phase 3).
///
/// Silently dropping some other group to make room would draw a picture the program
/// did not describe, and would do it in a part of the scene the author was not
/// touching. A raise is the only outcome that is both correct and diagnosable.
///
/// The loop installs exactly `CANVAS_MAX_GROUPS` names, asserts all of them succeed —
/// an off-by-one that raised at 256 would otherwise pass a test that only checked
/// "the 257th raises" — and then asserts the next one raises with the right code.
///
/// The names are `toString(i)`, so they are distinct and of differing byte lengths,
/// which also exercises the scan's length-first rejection: 1 and 11 share a first byte
/// and must not match.
#[cfg(target_os = "macos")]
const GROUP_LIMIT_SOURCE: &str = "IMPORT app\n\
     IMPORT canvas\n\
     IMPORT vector\n\
     IMPORT color\n\
     IMPORT errorCode\n\
     FUNC main AS Integer\n\
    \x20 app::setMode(app::Mode.Canvas)\n\
    \x20 LET c AS color::Color = color::rgb(1, 2, 3)\n\
    \x20 LET a AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 4.0, height := 4.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(c), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n\
    \x20 MUT i AS Integer = 0\n\
    \x20 WHILE i < 256\n\
    \x20   canvas::setGroup(toString(i), [a]) TRAP(early)\n\
    \x20     RETURN 70\n\
    \x20   END TRAP\n\
    \x20   i = i + 1\n\
    \x20 END WHILE\n\
    \x20 canvas::setGroup(\"one-too-many\", [a]) TRAP(err)\n\
    \x20   IF err.code <> errorCode::ErrCanvasGroupLimit THEN\n\
    \x20     RETURN 71\n\
    \x20   END IF\n\
    \x20   canvas::setGroup(\"0\", [a]) TRAP(replace)\n\
    \x20     RETURN 72\n\
    \x20   END TRAP\n\
    \x20   RETURN 0\n\
    \x20 END TRAP\n\
    \x20 RETURN 73\n\
     END FUNC\n";

#[cfg(target_os = "macos")]
#[test]
fn macos_set_group_past_the_table_limit_raises() {
    let (project, ok, log) = build_app("app_canvas_group_limit", GROUP_LIMIT_SOURCE, &[]);
    assert!(ok, "build should succeed:\n{log}");
    let exe =
        project.join("build/app_canvas_group_limit.app/Contents/MacOS/app_canvas_group_limit");
    let (code, _) = run_headless_with_stdin(&exe, "");
    assert_eq!(
        code, 0,
        "the 257th distinct name must raise ErrCanvasGroupLimit, and replacing an \
         existing name must still work with the table full — it needs no new slot \
         (70 = one of the first 256 raised, 71 = wrong code, 72 = replacing an \
         installed name wrongly raised, 73 = the 257th did not raise at all)"
    );
    let _ = fs::remove_dir_all(&project);
}

// ---------------------------------------------------------------------------
// bug-966 — a mode change runs the implicit `term::off`.
//
// `mfb spec app presentation-mode` ("The surface-reconcile seam"): every
// `setMode` that CHANGES the mode tears the surface down with an implicit
// `term::off`, so no raw/grid state survives the switch; a same-mode `setMode`
// is not a change and keeps the TUI. Before the fix no backend did it, so after
// `Console(term::on) -> Canvas -> Console` the TUI was still "on", `io::print`
// routed into the uninstalled grid, and (macOS GUI) nothing the program wrote
// afterwards was visible.
// ---------------------------------------------------------------------------

/// Exit codes name the failed observation: 1 = `term::on` did not turn the TUI on,
/// 2 = a same-mode `setMode` tore it down, 3 = `Canvas -> Console` left it on,
/// 4 = `None -> Console` left it on.
const MODE_CHANGE_TERM_OFF_SOURCE: &str = "IMPORT app\n\
     IMPORT term\n\
     FUNC main() AS Integer\n\
    \x20 app::setMode(app::Mode.Console)\n\
    \x20 term::on()\n\
    \x20 IF NOT term::isOn() THEN\n\
    \x20   RETURN 1\n\
    \x20 END IF\n\
    \x20 app::setMode(app::Mode.Console)\n\
    \x20 IF NOT term::isOn() THEN\n\
    \x20   RETURN 2\n\
    \x20 END IF\n\
    \x20 app::setMode(app::Mode.Canvas)\n\
    \x20 app::setMode(app::Mode.Console)\n\
    \x20 IF term::isOn() THEN\n\
    \x20   RETURN 3\n\
    \x20 END IF\n\
    \x20 term::on()\n\
    \x20 app::setMode(app::Mode.None)\n\
    \x20 app::setMode(app::Mode.Console)\n\
    \x20 IF term::isOn() THEN\n\
    \x20   RETURN 4\n\
    \x20 END IF\n\
    \x20 RETURN 0\n\
     END FUNC\n";

#[cfg(target_os = "macos")]
#[test]
fn macos_mode_change_turns_the_tui_off_and_same_mode_keeps_it() {
    let (project, ok, log) = build_app("app_mode_term_off", MODE_CHANGE_TERM_OFF_SOURCE, &[]);
    assert!(ok, "build should succeed:\n{log}");
    let exe = project.join("build/app_mode_term_off.app/Contents/MacOS/app_mode_term_off");
    assert_eq!(
        run_headless(&exe),
        0,
        "a mode change must run the implicit term::off and a same-mode setMode must \
         not (1 = term::on failed, 2 = same-mode setMode tore the TUI down, \
         3 = Canvas -> Console left it on, 4 = None -> Console left it on)"
    );
    let _ = fs::remove_dir_all(&project);
}

/// The instruction list of `symbol` in the `-ncode` plan of an app build.
fn ncode_body(project: &Path, name: &str, symbol: &str) -> Vec<serde_json::Value> {
    let text =
        fs::read_to_string(project.join(format!("{name}.ncode"))).expect("read the ncode plan");
    let plan: serde_json::Value = serde_json::from_str(&text).expect("ncode is JSON");
    plan["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .find(|function| function["symbol"] == symbol)
        .unwrap_or_else(|| panic!("{symbol} is emitted"))["instructions"]
        .as_array()
        .expect("instructions")
        .clone()
}

/// The implicit `term::off` lives in the one place every backend passes through
/// (`lower_set_mode`), so every app target's `setMode` helper calls the
/// `term::off` helper, guarded by a compare against the mode already in effect,
/// before it stores the new mode. Only macOS can run an app bundle here; the
/// Linux GTK and Windows rows were run on boxes 2226 and 206 (see bug-966) and
/// this pins the shared shape for them. A program with no `term::` state has no
/// TUI to turn off, so its `setMode` must not reference the helper at all.
#[test]
fn every_app_target_set_mode_calls_term_off_on_a_mode_change() {
    for target in [
        "linux-aarch64",
        "linux-x86_64",
        "windows-x86_64",
        "macos-aarch64",
    ] {
        let (project, ok, log) = build_app(
            "app_mode_term_off_ncode",
            MODE_CHANGE_TERM_OFF_SOURCE,
            &["-target", target, "-ncode"],
        );
        assert!(ok, "{target}: -ncode build should succeed:\n{log}");
        let body = ncode_body(
            &project,
            "app_mode_term_off_ncode",
            "_mfb_rt_app_app_setMode",
        );
        let call = body
            .iter()
            .position(|i| i["op"] == "bl" && i["target"] == "_mfb_rt_term_term_off")
            .unwrap_or_else(|| panic!("{target}: setMode must call the term::off helper"));
        let skip = body
            .iter()
            .position(|i| i["op"] == "b.eq")
            .unwrap_or_else(|| panic!("{target}: the term::off call must be mode-change gated"));
        assert!(
            skip < call,
            "{target}: the same-mode skip must precede the term::off call"
        );
        let _ = fs::remove_dir_all(&project);

        let (project, ok, log) = build_app(
            "app_mode_no_term_ncode",
            CANVAS_ROUNDTRIP_SOURCE,
            &["-target", target, "-ncode"],
        );
        assert!(ok, "{target}: -ncode build should succeed:\n{log}");
        let body = ncode_body(
            &project,
            "app_mode_no_term_ncode",
            "_mfb_rt_app_app_setMode",
        );
        assert!(
            !body.iter().any(|i| i["target"] == "_mfb_rt_term_term_off"),
            "{target}: a program with no term:: state must not call term::off"
        );
        let _ = fs::remove_dir_all(&project);
    }
}
