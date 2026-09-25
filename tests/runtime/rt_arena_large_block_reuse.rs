//! A freed large arena block is reused by any later request it fits (bug-690).
//!
//! Chunks larger than `ARENA_QUICK_BIN_MAX` (2048 bytes) used to park on 64 bins
//! hashed by their EXACT size, and a large request only ever took a node of exactly
//! its own size. A block whose size never recurred stayed parked until the arena was
//! destroyed, so a program whose large sizes vary mapped memory in proportion to the
//! sum of the sizes it had freed, not to its live data — `canvas::present` of a scene
//! whose item count changed every frame reached 13 GB (`examples/wind`).
//!
//! **What these tests measure.** `mapped_bytes` and `peak_live_bytes` from the
//! `--debug` report (plan-130-C): exact counts of the bytes the arena mapped and the
//! most it ever held live. A reusing allocator maps a small multiple of its peak live
//! bytes whatever sequence of sizes it is asked for; the bug maps the sum.
//!
//! Measured on macos-aarch64 before the fix → after:
//!
//! | case | before | after |
//! | --- | --- | --- |
//! | varying lengths vs one fixed length | 11,202,560 vs 159,744 B mapped | 139,264 vs 139,264 |
//! | a climbing length, 1.36 MB peak live | 678,772,736 B mapped | 3,506,176 |
//! | canvas worker arena, 26 MB peak live | 2,639,196,160 B mapped | 43,126,784 |
//! | canvas graphics arena, 10 MB peak live | 98,750,464 B mapped | 18,997,248 |

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

/// A reusing allocator maps at most this multiple of its peak live bytes. The arena
/// maps whole blocks (4 KiB, or a big request's own size rounded to a page, plus a
/// quarter when it is climbing), keeps the freed buffers of a list's doubling growth,
/// and never merges two mapped blocks, so 1× is not reachable; every row above ends
/// at 2.6× or less, and the bug is 9× to 500×.
const MAX_MAPPED_PER_PEAK_LIVE: u64 = 4;

/// `arena.<index>.<counter>` of `exe args…`'s `--debug` report.
fn report(name: &str, exe: &std::path::Path, args: &[&str]) -> (String, Vec<String>) {
    let output = Command::new(exe)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", exe.display()));
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "{name} {args:?} {}:\n{stdout}\n{stderr}",
        common::exit_description(&output.status),
    );
    let lines = common::debug_report::arena_lines(name, &stderr);
    (stdout, lines)
}

/// The bug report's console reproduction: an exact-length copy of a list, dropped at
/// once, 20,000 times. Its length cycles through 1,000 values — or, with any
/// argument, is always 2,500. Live data is the same either way.
const VARYING_OR_FIXED: &str = "\
IMPORT collections
IMPORT io
IMPORT os

FUNC main AS Integer
  LET fixed AS Boolean = len(os::args()) > 0
  MUT big AS List OF Integer = []
  FOR k = 1 TO 4000
    big = collections::append(big, k)
  NEXT
  MUT total AS Integer = 0
  FOR i = 1 TO 20000
    MUT n AS Integer = 2000 + ((i * 37) MOD 1000)
    IF fixed THEN n = 2500
    LET xs AS List OF Integer = collections::mid(big, 0, n)
    total = total + len(xs)
  NEXT
  io::print(toString(total))
  RETURN 0
END FUNC
";

/// Copies whose length never repeats map about what copies of one length map.
#[test]
fn varying_large_sizes_map_no_more_than_a_fixed_size() {
    let exe = common::debug_report::build_debug("arena_large_varying", VARYING_OR_FIXED);
    let (out, varying) = report("arena_large_varying", &exe, &[]);
    assert_eq!(out.trim(), "49990000", "the varying copies' total length");
    let (out, fixed) = report("arena_large_varying", &exe, &["fixed"]);
    assert_eq!(out.trim(), "50000000", "the fixed copies' total length");

    let counter = |lines: &[String], name: &str| {
        common::debug_report::counter("arena_large_varying", lines, 0, name)
    };
    let (varying_mapped, fixed_mapped) = (
        counter(&varying, "mapped_bytes"),
        counter(&fixed, "mapped_bytes"),
    );
    assert_eq!(
        counter(&varying, "peak_live_bytes"),
        counter(&fixed, "peak_live_bytes"),
        "both runs hold the same live data at their peak"
    );
    assert!(
        varying_mapped <= 2 * fixed_mapped,
        "copies of 1,000 different lengths mapped {varying_mapped} bytes, copies of one \
         length {fixed_mapped}: a freed large block must be reused by any request it \
         fits, not only by one of exactly its size (bug-690)"
    );
}

/// A copy that grows a little every time — a scene gaining an item a frame — maps a
/// small multiple of the largest copy, not the sum of all of them. Blocks are mapped
/// separately and never merge, so this needs a climbing request's block to carry
/// slack for the next few steps.
#[test]
fn a_climbing_large_size_maps_a_multiple_of_its_peak() {
    let source = "\
IMPORT collections
IMPORT io

FUNC main AS Integer
  MUT big AS List OF Integer = []
  FOR k = 1 TO 82000
    big = collections::append(big, k)
  NEXT
  MUT total AS Integer = 0
  FOR i = 1 TO 2000
    LET xs AS List OF Integer = collections::mid(big, 0, 2000 + i * 40)
    total = total + len(xs)
  NEXT
  io::print(toString(total))
  RETURN 0
END FUNC
";
    let exe = common::debug_report::build_debug("arena_large_climb", source);
    let (out, lines) = report("arena_large_climb", &exe, &[]);
    assert_eq!(out.trim(), "84040000", "the copies' total length");
    let mapped = common::debug_report::counter("arena_large_climb", &lines, 0, "mapped_bytes");
    let peak = common::debug_report::counter("arena_large_climb", &lines, 0, "peak_live_bytes");
    assert!(
        mapped <= MAX_MAPPED_PER_PEAK_LIVE * peak,
        "2,000 copies climbing 40 elements at a time mapped {mapped} bytes for a peak \
         of {peak} live: more than {MAX_MAPPED_PER_PEAK_LIVE}× (bug-690)"
    );
}

/// The canvas shape the bug was found in: a scene whose item count changes every
/// frame, presented 300 times. `present` deep-copies the scene and builds hash lists
/// sized to it on the worker, and the graphics thread allocates per-frame blocks
/// sized by the item count in its own arena; every one of those sizes changes from
/// frame to frame. Both arenas must reach a steady size.
///
/// `MFB_CANVAS_GPU=1` asks for the Metal renderer where there is one (it falls back to
/// software elsewhere), which is where the graphics arena's growth was fastest
/// (98.7 MB for 10 MB live before the fix); the software renderer allocates less per
/// frame but through the same allocator.
#[cfg(unix)]
#[test]
fn a_canvas_scene_of_changing_size_reaches_a_steady_arena_size() {
    let name = "arena_large_canvas";
    let source = "\
IMPORT app
IMPORT canvas
IMPORT collections
IMPORT color
IMPORT os

FUNC main AS Integer
  app::setMode(app::Mode.Canvas)
  MUT all AS List OF canvas::DrawItem = []
  FOR i = 0 TO 10999
    LET x AS Float = toFloat((i * 37) MOD 800) + 0.013
    LET y AS Float = toFloat((i * 13) MOD 600) + toFloat(i) * 0.0007
    LET p AS canvas::Paint = canvas::stroke(color::withAlpha(color::rgb(120 + (i MOD 100), 170, 215), 40 + (i MOD 195)), 1.35)
    LET item AS canvas::DrawItem = canvas::Line[x1 := x, y1 := y, x2 := x + 5.0, y2 := y + 3.0, cap := canvas::CapStyle.Round, paint := p]
    all = collections::append(all, item)
  NEXT
  FOR frame = 1 TO 300
    LET items AS List OF canvas::DrawItem = collections::take(all, 10000 + ((frame * 37) MOD 1000))
    canvas::present(items)
    os::sleep(16)
  NEXT
  RETURN 0
END FUNC
";
    let project = common::temp_project(name, source);
    let binary = common::build_app_debug(&project, name);
    let output = Command::new(&binary)
        .current_dir(&project)
        .env("MFB_MACAPP_HEADLESS", "1")
        .env("MFB_WINAPP_HEADLESS", "1")
        .env("MFB_GTKAPP_HEADLESS", "1")
        .env("MFB_CANVAS_GPU", "1")
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", binary.display()));
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "{name} {}:\n{}\n{stderr}",
        common::exit_description(&output.status),
        String::from_utf8_lossy(&output.stdout),
    );
    let lines = common::debug_report::arena_lines(name, &stderr);
    // Both arenas are checked before failing, so a red run names every one that grew.
    let mut grown = Vec::new();
    for kind in ["main", "graphics"] {
        let index = (0..)
            .take_while(|index| {
                lines
                    .iter()
                    .any(|l| l.starts_with(&format!("arena.{index}.")))
            })
            .find(|index| lines.contains(&format!("arena.{index}.kind {kind}")))
            .unwrap_or_else(|| panic!("{name}: no `{kind}` arena in {lines:?}"));
        let mapped = common::debug_report::counter(name, &lines, index, "mapped_bytes");
        let peak = common::debug_report::counter(name, &lines, index, "peak_live_bytes");
        if mapped > MAX_MAPPED_PER_PEAK_LIVE * peak {
            grown.push(format!(
                "the {kind} arena mapped {mapped} bytes for a peak of {peak} live"
            ));
        }
    }
    assert!(
        grown.is_empty(),
        "{name}: {} — more than {MAX_MAPPED_PER_PEAK_LIVE}×. A scene whose size changes \
         every frame must reuse the blocks earlier frames freed (bug-690).",
        grown.join("; ")
    );
    let _ = std::fs::remove_dir_all(&project);
}
