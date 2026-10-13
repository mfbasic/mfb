//! The canvas reporting hooks and test switches exist only in a `--debug` build
//! (plan-130-E).
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
const ONE_FRAME: &str =
    "IMPORT app\nIMPORT canvas\nIMPORT vector\nIMPORT color\nIMPORT io\n\nFUNC main() AS Nothing\n  \
     app::setMode(app::Mode.Canvas)\n  \
     canvas::present([canvas::Rectangle[id := \"\", position := vector::Float2[x := 10.0, y := 10.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 50.0, height := 50.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(255, 0, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]])\n  \
     io::print(\"presented\")\nEND FUNC\n";

/// Run `binary` headless with both hooks pointed into `project`; return the two paths.
fn run_with_hooks(binary: &Path, project: &Path) -> (PathBuf, PathBuf) {
    let stats = project.join("stats.txt");
    let frame = project.join("frame.rgba");
    let run = Command::new(binary)
        .current_dir(project)
        .env("MFB_HEADLESS", "1")
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

/// Every `MFB_…` name in the files a build left under `dir`, as ASCII or UTF-16LE
/// (the Windows bootstrap's names are wide strings).
fn mfb_names_under(dir: &Path) -> std::collections::BTreeSet<String> {
    fn names_in(bytes: &[u8], step: usize, found: &mut std::collections::BTreeSet<String>) {
        let unit = |at: usize| -> Option<u8> {
            let byte = *bytes.get(at)?;
            (step == 1 || bytes.get(at + 1) == Some(&0)).then_some(byte)
        };
        let mut at = 0;
        while at + 4 * step <= bytes.len() {
            let starts = (0..4).all(|i| unit(at + i * step) == Some(b"MFB_"[i]));
            if !starts {
                at += 1;
                continue;
            }
            let mut name = String::from("MFB_");
            let mut next = at + 4 * step;
            while let Some(byte) = unit(next) {
                if !(byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_') {
                    break;
                }
                name.push(byte as char);
                next += step;
            }
            if name.len() > 4 {
                found.insert(name);
            }
            at = next;
        }
    }
    let mut found = std::collections::BTreeSet::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in std::fs::read_dir(&next).expect("read the build directory") {
            let path = entry.expect("a build directory entry").path();
            let meta = std::fs::symlink_metadata(&path).expect("stat a build output");
            if meta.is_dir() {
                pending.push(path);
            } else if meta.is_file() {
                let bytes = std::fs::read(&path).expect("read a build output");
                names_in(&bytes, 1, &mut found);
                names_in(&bytes, 2, &mut found);
            }
        }
    }
    found
}

/// Build `ONE_FRAME` as an app for `target` and return the `MFB_…` names in its output.
fn mfb_names_built(target: &str, debug: bool) -> std::collections::BTreeSet<String> {
    let name = format!(
        "canvas_env_{}_{}",
        target.replace('-', "_"),
        if debug { "debug" } else { "normal" }
    );
    let project = common::temp_project(&name, ONE_FRAME);
    let mut command = Command::new(common::mfb_exe());
    // `--app-debug` keeps the Linux AppDir beside the AppImage, whose payload is
    // compressed; it is `--app` everywhere else.
    command.arg("build").arg("-q").arg("--app-debug");
    if debug {
        command.arg("--debug");
    }
    let output = command
        .args(["--target", target])
        .arg(&project)
        .output()
        .expect("run mfb build");
    assert!(
        output.status.success(),
        "build for {target} failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let names = mfb_names_under(&project.join("build"));
    let _ = std::fs::remove_dir_all(&project);
    names
}

/// The environment variables a compiled program reads are a contract with two sides.
/// A normal build of this program names exactly three: the headless switch, the
/// renderer choice, and `MFB_CANVAS_SYNC`. Every reporting hook and every other switch
/// a test drives a program with exists only in a `--debug` build, name and reader both.
///
/// `MFB_CANVAS_SYNC` — and `MFB_MOUSE_INJECT`, which this program does not carry
/// because it never enables the mouse — stay in every build while canvas is in
/// development: `tools/canvas-bench` measures release builds with them.
#[test]
fn a_normal_build_names_only_the_headless_renderer_and_sync_variables() {
    let canvas_debug = [
        "MFB_CANVAS_AUTO_WINDOW_MS",
        "MFB_CANVAS_DAMAGE",
        "MFB_CANVAS_DISPLAY_HZ",
        "MFB_CANVAS_DUMP",
        "MFB_CANVAS_FRAME_HOLD_MS",
        "MFB_CANVAS_GEO_VERIFY",
        "MFB_CANVAS_GLYPH_BUDGET",
        "MFB_CANVAS_POINTER",
        "MFB_CANVAS_STATS",
        "MFB_TRANSCRIPT_DUMP",
    ];
    let scripted_window = [
        "MFB_CANVAS_POINTER_X",
        "MFB_CANVAS_POINTER_Y",
        "MFB_CANVAS_RESIZE_H",
        "MFB_CANVAS_RESIZE_W",
    ];
    let windows_only = [
        "MFB_WINAPP_INPUT",
        "MFB_WINAPP_TERM_RESIZE_H",
        "MFB_WINAPP_TERM_RESIZE_W",
    ];
    let normal: std::collections::BTreeSet<String> =
        ["MFB_CANVAS_GPU", "MFB_CANVAS_SYNC", "MFB_HEADLESS"]
            .iter()
            .map(|name| name.to_string())
            .collect();
    for (target, extra) in [
        ("macos-aarch64", Vec::new()),
        ("linux-x86_64", scripted_window.to_vec()),
        ("linux-aarch64", scripted_window.to_vec()),
        (
            "windows-x86_64",
            [&scripted_window[..], &windows_only[..]].concat(),
        ),
    ] {
        assert_eq!(
            mfb_names_built(target, false),
            normal,
            "{target}: a normal build names a variable it must not read"
        );
        let mut debug = normal.clone();
        debug.extend(
            canvas_debug
                .iter()
                .chain(&extra)
                .map(|name| name.to_string()),
        );
        assert_eq!(
            mfb_names_built(target, true),
            debug,
            "{target}: the --debug build's variables"
        );
    }
}
