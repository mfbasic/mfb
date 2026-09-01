//! The canvas render loop runs on its own thread (plan-98-D Phase 2).
//!
//! The claims here are about *when and how often* a frame happens, which is
//! invisible in the pixels — an identical picture results whether it was drawn once
//! or a hundred times, on the worker or on a graphics thread. `MFB_CANVAS_STATS`
//! appends one line per rendered frame, so the frame count is observable; and
//! `MFB_CANVAS_SYNC` makes `present` wait for the frame it asked for, so a test can
//! assert per-present behaviour without racing the scheduler.
//!
//! `.ai/canvas-threading.md` is the protocol these check.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

/// Build a `--app` program and run it headless, returning `(stdout, frame lines)`.
///
/// `sync` selects whether `present` waits for its frame. Both modes are exercised:
/// the deterministic one for per-present claims, the default one for the claim that
/// a program does not *need* to wait.
fn run(name: &str, source: &str, sync: bool) -> (String, Vec<String>) {
    run_with(name, source, sync, false)
}

/// As `run`, plus the `MFB_CANVAS_GPU` selector (plan-98-E).
fn run_with(name: &str, source: &str, sync: bool, metal: bool) -> (String, Vec<String>) {
    let project = common::temp_project(name, source);
    let build = Command::new(common::mfb_exe())
        .arg("build")
        .arg("-app")
        .arg(&project)
        .output()
        .expect("run mfb build -app");
    assert!(
        build.status.success(),
        "mfb build -app failed:\n{}",
        String::from_utf8_lossy(&build.stderr),
    );

    let stats = project.join("stats.txt");
    let binary = app_binary(&project, name);
    let mut command = Command::new(&binary);
    command
        .env("MFB_MACAPP_HEADLESS", "1")
        .env("MFB_WINAPP_HEADLESS", "1")
        .env("MFB_CANVAS_STATS", &stats);
    if sync {
        command.env("MFB_CANVAS_SYNC", "1");
    }
    if metal {
        command.env("MFB_CANVAS_GPU", "1");
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

    let lines = std::fs::read_to_string(&stats)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect();
    let out = String::from_utf8_lossy(&run.stdout).to_string();
    let _ = std::fs::remove_dir_all(&project);
    (out, lines)
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

/// A program body, wrapped in the canvas-mode boilerplate.
fn program(body: &str) -> String {
    format!(
        "IMPORT app\nIMPORT canvas\nIMPORT io\n\nSUB main()\n  \
         app::setMode(Mode.Canvas)\n{body}END SUB\n"
    )
}

const ONE_BOX: &str = "  LET box AS DrawItem = Rectangle[x := 10.0, y := 10.0, w := 50.0, \
                       h := 50.0, paint := canvas::fill(canvas::rgb(255, 0, 0))]\n";

/// A program that presents and returns immediately still gets its frame drawn.
///
/// This is the one that matters most, and the one that was broken: the render is
/// asynchronous now, so `main` returning starts the shutdown while the graphics
/// thread may not have woken yet. Shutdown must **drain** the pending frame rather
/// than cancel it — the first version checked "stopping" before "pending" and dropped
/// the frame, nondeterministically. Run from a shell it drew; run under `cargo test`
/// it did not.
///
/// Deliberately **not** in sync mode: waiting for the frame is exactly what a program
/// must not have to do.
#[test]
fn a_present_then_immediate_return_still_renders() {
    let (stdout, frames) = run(
        "canvas_gfx_drain",
        &program(&format!(
            "{ONE_BOX}  canvas::present([box])\n  io::print(\"done\")\n"
        )),
        false,
    );
    assert!(
        stdout.contains("done"),
        "the program must run to completion"
    );
    assert_eq!(
        frames.len(),
        1,
        "the pending frame must be drained by shutdown, not cancelled: {frames:?}",
    );
}

/// A program that never presents starts no graphics thread and draws nothing.
///
/// The thread is spawned lazily by the first `present`, so entering canvas mode and
/// doing nothing costs neither a thread nor a frame. It also proves the shutdown join
/// is a no-op when nothing was started — otherwise this would hang.
#[test]
fn canvas_mode_without_a_present_renders_nothing() {
    let (stdout, frames) = run(
        "canvas_gfx_idle",
        &program("  io::print(\"idle\")\n"),
        false,
    );
    assert!(stdout.contains("idle"));
    assert!(
        frames.is_empty(),
        "no present means no frame and no graphics thread: {frames:?}",
    );
}

/// A static scene costs exactly one frame, however long the program lives.
///
/// Time is deliberately not a redraw trigger (`.ai/canvas-threading.md` §4), so the
/// render loop must be a real condition wait rather than a poll. A spinning loop
/// would show many frames here — and would also burn a core.
#[test]
fn a_static_scene_renders_once_and_does_not_spin() {
    let source = program(&format!(
        "{ONE_BOX}  canvas::present([box])\n  \
         LET deadline AS Integer = datetime::monotonicNanos() + 1000000000\n  \
         MUT spins AS Integer = 0\n  \
         WHILE datetime::monotonicNanos() < deadline\n    spins = spins + 1\n  END WHILE\n  \
         io::print(\"waited\")\n"
    ))
    .replace("IMPORT io\n", "IMPORT io\nIMPORT datetime\n");
    let (stdout, frames) = run("canvas_gfx_static", &source, false);
    assert!(stdout.contains("waited"));
    assert_eq!(
        frames.len(),
        1,
        "a scene that never changes must not be redrawn while the program idles: \
         {frames:?}",
    );
}

/// An identical re-present publishes nothing, so it draws nothing.
///
/// The frame skip is plan-98-B's, and it has to survive the move to a graphics
/// thread: `present` only signals when `publishScene` reports it actually published.
#[test]
fn an_identical_re_present_draws_no_second_frame() {
    let (_, frames) = run(
        "canvas_gfx_skip",
        &program(&format!(
            "{ONE_BOX}  canvas::present([box])\n  canvas::present([box])\n  \
             canvas::present([box])\n  io::print(\"done\")\n"
        )),
        true,
    );
    assert_eq!(
        frames.len(),
        1,
        "three presents of identical content must publish — and draw — once: \
         {frames:?}",
    );
}

/// In sync mode every present gets its own frame, which is what makes the
/// per-present assertions elsewhere deterministic.
///
/// Without it the frame count is a scheduling detail: presents that arrive between
/// two frames coalesce by design, and the same three-present program was observed
/// producing one, two and three frames across runs.
#[test]
fn sync_mode_gives_one_frame_per_changed_present() {
    let body = format!(
        "{ONE_BOX}  canvas::present([box])\n  \
         LET two AS DrawItem = Rectangle[x := 80.0, y := 10.0, w := 50.0, h := 50.0, \
         paint := canvas::fill(canvas::rgb(0, 255, 0))]\n  \
         canvas::present([box, two])\n  \
         LET three AS DrawItem = Rectangle[x := 150.0, y := 10.0, w := 50.0, h := 50.0, \
         paint := canvas::fill(canvas::rgb(0, 0, 255))]\n  \
         canvas::present([box, two, three])\n  io::print(\"done\")\n"
    );
    let (_, frames) = run("canvas_gfx_sync", &program(&body), true);
    assert_eq!(
        frames.len(),
        3,
        "sync mode must give each changed present its own frame: {frames:?}",
    );
}

/// D's frame counter, driven through the Metal renderer, still advances once per
/// changed present — and only after the GPU has finished the frame.
///
/// This is plan-98-E Phase 3's claim (Correction 12). The counter *is*
/// `lastCompletedFrame`: `__canvas_renderLoop` calls `canvas::frameDone()` after
/// `__canvas_renderFrame()` returns, and on the Metal path that return is behind
/// `[commandBuffer waitUntilCompleted]` — the GPU has finished before the counter can
/// move. Every consumer D built on it (the scene ring's retirement gate,
/// `MFB_CANVAS_SYNC`) therefore sees GPU-completion ordering with no change of its
/// own, which is exactly what "renderer-swappable frame-completion signal" was
/// supposed to buy.
///
/// Counting frames is what makes that observable from outside: a counter that moved
/// early would produce more frames than presents, and one that never moved would hang
/// the sync wait.
#[test]
fn the_metal_path_gives_one_completed_frame_per_changed_present() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let body = format!(
        "{ONE_BOX}  canvas::present([box])\n  \
         LET two AS DrawItem = Rectangle[x := 80.0, y := 10.0, w := 50.0, h := 50.0, \
         paint := canvas::fill(canvas::rgb(0, 255, 0))]\n  \
         canvas::present([box, two])\n  \
         LET three AS DrawItem = Rectangle[x := 150.0, y := 10.0, w := 50.0, h := 50.0, \
         paint := canvas::fill(canvas::rgb(0, 0, 255))]\n  \
         canvas::present([box, two, three])\n  io::print(\"done\")\n"
    );
    let (out, frames) = run_with("canvas_gfx_sync_metal", &program(&body), true, true);
    if !frames
        .last()
        .is_some_and(|line| line.contains("metalReady=TRUE"))
    {
        return; // no Metal device on this host
    }
    assert!(
        out.contains("done"),
        "the program must run to completion: {out}"
    );
    assert_eq!(
        frames.len(),
        3,
        "each changed present must produce exactly one completed Metal frame: {frames:?}",
    );
}

/// The scene ring's frame skip survives the Metal renderer.
///
/// The skip is decided in `canvas::present` on the *worker*, so it should be
/// renderer-independent — but "should be" is the claim under test. If the Metal path
/// advanced the frame counter differently the retirement gate would drain at a
/// different rate, and three identical presents would stop collapsing to one frame.
#[test]
fn an_identical_re_present_draws_no_second_metal_frame() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let (_, frames) = run_with(
        "canvas_gfx_skip_metal",
        &program(&format!(
            "{ONE_BOX}  canvas::present([box])\n  canvas::present([box])\n  \
             canvas::present([box])\n  io::print(\"done\")\n"
        )),
        true,
        true,
    );
    if !frames
        .last()
        .is_some_and(|line| line.contains("metalReady=TRUE"))
    {
        return;
    }
    assert_eq!(
        frames.len(),
        1,
        "three identical presents must publish — and draw — once on Metal too: \
         {frames:?}",
    );
}

/// The renderer seam reports a real Metal device, and selecting it is opt-in.
///
/// This is plan-98-E's first checkable claim, and it is deliberately about the
/// *plumbing* rather than about pixels: `metal=TRUE` means `Metal.framework`'s
/// install name resolved, its import-table row bound, and
/// `MTLCreateSystemDefaultDevice` returned a device. Every later piece of the Metal
/// backend rests on those three, and a failure in any of them is far cheaper to read
/// here than as a blank window several hundred lines later.
///
/// `gpuSelected` must be FALSE by default. The software renderer is the oracle the
/// GPU path is measured against (plan-98-A invariant 7) and its goldens are
/// exact-match; if selecting Metal were the default, every one of those goldens would
/// silently become a tolerance test against a reference that no longer exists.
#[test]
fn the_renderer_seam_finds_a_metal_device_and_leaves_it_unselected() {
    let source = program(&format!(
        "{ONE_BOX}  canvas::present([box])\n  io::print(\"done\")\n"
    ));
    let field = |lines: &[String], name: &str| -> String {
        lines
            .first()
            .and_then(|l| {
                l.split_whitespace()
                    .find_map(|f| f.strip_prefix(name).map(str::to_string))
            })
            .unwrap_or_else(|| panic!("no {name} field in {lines:?}"))
    };

    let (_, default_run) = run_with("canvas_metal_default", &source, true, false);
    assert_eq!(
        field(&default_run, "metal="),
        "TRUE",
        "MTLCreateSystemDefaultDevice must bind and return a device on this host",
    );
    assert_eq!(
        field(&default_run, "gpuSelected="),
        "FALSE",
        "the software renderer must stay the default — it is the exact-match oracle",
    );

    let (_, selected) = run_with("canvas_metal_selected", &source, true, true);
    assert_eq!(
        field(&selected, "gpuSelected="),
        "TRUE",
        "MFB_CANVAS_GPU must select the Metal renderer",
    );
}
