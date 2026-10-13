//! The pointer words in the graphics state (plan-172-J).
//!
//! Each platform's main thread publishes where the pointer is — position, held
//! buttons, and whether it is over the surface — and the graphics thread reads the
//! four as one consistent set. Nothing a program can call reports them, so these read
//! them the only way a test can see graphics-thread state: the `pointer=` field of a
//! `--debug` build's `MFB_CANVAS_STATS` line (`.ai/canvas-threading.md` section 11).
//!
//! The pointer is put somewhere by `MFB_CANVAS_POINTER`, the `--debug` build's script
//! (`helper_render.rs:POINTER_SCRIPT_DEBUG`), which calls the same writer the
//! platforms' handlers call. That proves the words, the writer and the reader on every
//! target these tests run on; that each platform's REAL handler calls the writer is a
//! separate claim, proved per platform below and in `scripts/test-macapp.sh`.

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

/// Build `source` as a `--debug` app, run it headless and synchronously with `envs`
/// added, and return the `pointer=` field of each stats line it wrote, in order.
fn pointers(name: &str, source: &str, envs: &[(&str, &str)]) -> Vec<String> {
    let project = common::temp_project(name, source);
    let stats = project.join("stats.txt");
    let binary = common::build_app_debug(&project, name);
    let mut command = Command::new(&binary);
    command
        .env("MFB_HEADLESS", "1")
        .env("MFB_CANVAS_STATS", &stats)
        // One frame per present, so the n-th stats line is the n-th present's frame.
        .env("MFB_CANVAS_SYNC", "1");
    for (key, value) in envs {
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
    let text = std::fs::read_to_string(&stats).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&project);
    text.lines()
        .map(|line| {
            line.split_whitespace()
                .find_map(|field| field.strip_prefix("pointer="))
                .unwrap_or_else(|| panic!("a stats line has no pointer= field: {line}"))
                .to_string()
        })
        .collect()
}

/// Three presents, each of a different scene so each is a frame: 2.5 s after the first,
/// then 3.5 s after the second.
const THREE_FRAMES: &str = r#"IMPORT app
IMPORT canvas
IMPORT color
IMPORT os
IMPORT vector

FUNC box(x AS Float) AS canvas::DrawItem
  MUT r AS canvas::Rectangle
  r = WITH r { position := vector::Float2[x := x, y := 100.0], size := canvas::Size[width := 50.0, height := 50.0], paint := canvas::fill(color::rgb(200, 200, 200)) }
  LET item AS canvas::DrawItem = r
  RETURN item
END FUNC

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  canvas::present([box(100.0)])
  os::sleep(2500)
  canvas::present([box(200.0)])
  os::sleep(3500)
  canvas::present([box(300.0)])
END FUNC
"#;

/// The script's entries are applied at the start of the first frame at or after their
/// time, in order, and the graphics thread reads back exactly what was published.
///
/// Three entries, at 0, 1.5 and 5 s on the script's clock, which starts with the first
/// frame: inside at (10, 20) with nothing held; inside at (30, 40) with the left
/// button; then gone from the surface. The second frame starts 2.5 s after the first
/// ENDS and the third at least 6 s after it started, so the second entry is due by
/// the second frame and the third is not unless the first frame took 2.5 s to draw —
/// a `--debug` build's software frame is slow, and the first cut of this test, with
/// 100 ms margins, saw the third entry land in the second frame.
#[test]
fn the_graphics_thread_reads_each_scripted_pointer_state() {
    let seen = pointers(
        "canvas_pointer_script",
        THREE_FRAMES,
        &[(
            "MFB_CANVAS_POINTER",
            "0,10,20,0,1;1500,30,40,1,1;5000,0,0,0,0",
        )],
    );
    assert_eq!(seen, vec!["10,20,0,1", "30,40,1,1", "0,0,0,0"]);
}

/// With no script the words are what the block starts as: the pointer is nowhere.
/// And two entries due at once are both applied, the later one winning — an entry is
/// never skipped, only superseded.
#[test]
fn an_unscripted_pointer_is_outside_and_due_entries_apply_in_order() {
    assert_eq!(
        pointers("canvas_pointer_none", THREE_FRAMES, &[]),
        vec!["0,0,0,0", "0,0,0,0", "0,0,0,0"]
    );
    let seen = pointers(
        "canvas_pointer_burst",
        THREE_FRAMES,
        &[("MFB_CANVAS_POINTER", "0,1,2,0,1;0,3,4,4,1;1500,5,6,2,1")],
    );
    assert_eq!(seen, vec!["3,4,4,1", "5,6,2,1", "5,6,2,1"]);
}

/// The platform's own pointer callback, called headless with a scripted position
/// (`MFB_CANVAS_POINTER_X` / `_Y`): GTK's motion callback, Windows' pointer-move
/// function — the code a real pointer event reaches, on the scripted-resize precedent.
///
/// Not on macOS, and the reason is the platform's, not the test's: AppKit has no
/// headless path that could call a view's `mouseMoved:` — the IMP sends messages to a
/// real `NSEvent` — so macOS has no such variable. Its handlers are proved in a real
/// window by `scripts/test-macapp.sh` case 6c.
///
/// The size is scripted too. "Inside" is asked of the PUBLISHED surface size, which a
/// headless run has none of until something publishes one, exactly as a window would
/// have before any pointer reached it. The first frame is from before the move; a point
/// off the 900x640 surface is published, as outside.
#[cfg(not(target_os = "macos"))]
#[test]
fn the_platform_callback_publishes_a_scripted_position() {
    let size = [
        ("MFB_CANVAS_RESIZE_W", "900"),
        ("MFB_CANVAS_RESIZE_H", "640"),
    ];
    let mut inside = size.to_vec();
    inside.extend([
        ("MFB_CANVAS_POINTER_X", "123"),
        ("MFB_CANVAS_POINTER_Y", "45"),
    ]);
    let mut seen = pointers("canvas_pointer_native_in", THREE_FRAMES, &inside);
    seen.dedup();
    assert_eq!(seen, vec!["0,0,0,0", "123,45,0,1"]);

    let mut outside = size.to_vec();
    outside.extend([
        ("MFB_CANVAS_POINTER_X", "5000"),
        ("MFB_CANVAS_POINTER_Y", "45"),
    ]);
    let mut seen = pointers("canvas_pointer_native_out", THREE_FRAMES, &outside);
    seen.dedup();
    assert_eq!(seen, vec!["0,0,0,0", "5000,45,0,0"]);
}
