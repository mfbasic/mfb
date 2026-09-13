//! The canvas reporting hooks exist only in a `--debug` build (plan-130-E).
//!
//! `MFB_CANVAS_STATS` (one counters line per frame) and `MFB_CANVAS_DUMP` (the raw
//! frame bytes) are reporting hooks, not program behavior. A normal build does not
//! contain the code that reads them, so setting them changes nothing; a `--debug`
//! build honours both exactly as the canvas suites rely on.

#[path = "../common/mod.rs"]
mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

/// One presented frame, then exit.
const ONE_FRAME: &str = "IMPORT app\nIMPORT canvas\nIMPORT color\nIMPORT io\n\nSUB main()\n  \
     app::setMode(app::Mode.Canvas)\n  \
     canvas::present([canvas::Rectangle[x := 10.0, y := 10.0, w := 50.0, h := 50.0, \
     paint := canvas::fill(color::rgb(255, 0, 0))]])\n  \
     io::print(\"presented\")\nEND SUB\n";

/// Run `binary` headless with both hooks pointed into `project`; return the two paths.
fn run_with_hooks(binary: &Path, project: &Path) -> (PathBuf, PathBuf) {
    let stats = project.join("stats.txt");
    let frame = project.join("frame.rgba");
    let run = Command::new(binary)
        .current_dir(project)
        .env("MFB_MACAPP_HEADLESS", "1")
        .env("MFB_WINAPP_HEADLESS", "1")
        .env("MFB_GTKAPP_HEADLESS", "1")
        .env("MFB_CANVAS_SYNC", "1")
        .env("MFB_CANVAS_STATS", &stats)
        .env("MFB_CANVAS_DUMP", &frame)
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", binary.display()));
    assert!(
        run.status.success(),
        "program {}:\n{}\n{}",
        common::exit_description(&run.status),
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr),
    );
    assert!(
        String::from_utf8_lossy(&run.stdout).contains("presented"),
        "the program did not reach the end of main"
    );
    (stats, frame)
}

#[test]
fn a_normal_build_ignores_the_stats_and_dump_variables() {
    let project = common::temp_project("canvas_hooks_normal", ONE_FRAME);
    let binary = common::build_app(&project, "canvas_hooks_normal");
    let (stats, frame) = run_with_hooks(&binary, &project);
    let wrote_stats = stats.exists();
    let wrote_frame = frame.exists();
    let _ = std::fs::remove_dir_all(&project);
    assert!(
        !wrote_stats,
        "a build without --debug wrote MFB_CANVAS_STATS; the hook must not be in it"
    );
    assert!(
        !wrote_frame,
        "a build without --debug wrote MFB_CANVAS_DUMP; the hook must not be in it"
    );
}

#[test]
fn a_debug_build_writes_one_stats_line_per_frame_and_the_frame() {
    let project = common::temp_project("canvas_hooks_debug", ONE_FRAME);
    let binary = common::build_app_debug(&project, "canvas_hooks_debug");
    let (stats, frame) = run_with_hooks(&binary, &project);
    let lines: Vec<String> = std::fs::read_to_string(&stats)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect();
    let frame_len = std::fs::metadata(&frame).map(|m| m.len()).unwrap_or(0);
    let _ = std::fs::remove_dir_all(&project);
    assert_eq!(
        lines.len(),
        1,
        "one presented frame must append exactly one stats line: {lines:?}"
    );
    assert!(
        lines[0].contains("frames=1"),
        "the stats line must count the frame: {}",
        lines[0]
    );
    assert!(
        frame_len > 0 && frame_len % 4 == 0,
        "the dump must hold RGBA8 frame bytes, got {frame_len} bytes"
    );
}
