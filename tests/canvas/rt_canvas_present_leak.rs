//! `canvas::present` holds steady-state memory when called in a loop (bug-683).
//!
//! `present` deep-copies the caller's scene into a fresh arena block before it does
//! anything else (`.ai/canvas-threading.md` §3 step 1). Two of the paths out of that
//! copy dropped the block on the floor, so a program presenting every frame grew
//! without bound at a rate proportional to its scene's geometry:
//!
//! 1. **The frame skip.** The copy is made *before* the content comparison, and the
//!    `skip` label returned without freeing it — so every re-present of an
//!    **unchanged** scene leaked a whole scene copy. That is exactly the case
//!    `mfb man canvas` calls a no-op.
//! 2. **Retire over an unreclaimed retirement.** A publish retires the block it
//!    displaces rather than freeing it, because the renderer may be reading it, and a
//!    *later* publish frees it once the frame counter has moved past. There was one
//!    retired slot and the retire store was unguarded, so a second present inside one
//!    rendered frame — the ordinary case at 60 Hz against a slower renderer —
//!    overwrote the pointer already there and lost it.
//!
//! Measured on the bug's own program (95 polygons of 400 points at ~60 Hz): RSS
//! climbed 361 MB → 1441 MB in 15 s, at the same ~90 MB/s whether the scene was
//! identical every frame or moving, while the same program building the same scenes
//! and never presenting was flat.
//!
//! **What these tests measure.** `arena.0.live_bytes` from the `--debug` report
//! (plan-130-C) — bytes the main thread's arena allocated and never got back, at
//! exit. It is a deterministic count, not a sampled RSS, so the signal is exact: a
//! leak-free `present` reports the same number at N frames and at 2N, and a
//! per-present leak reports twice as much. Every row was linear in the frame count
//! before the fix (measured: 3,230,624 B at 200 presents and 6,440,224 B at 400 for
//! the unchanged scene — 16,048 B per present, one full scene copy).
//!
//! Each program animates at ~60 Hz rather than spinning — see [`FRAME_MS`], which is
//! load-bearing, not decoration.
//!
//! **Why the two rows run under different schedulers.** The leaks live on the two
//! different exits, and the scheduler picks which exit a present takes:
//!
//! - The unchanged-scene row runs under `MFB_CANVAS_SYNC`, where the frame counter
//!   advances on every present so the retirement gate always fires. Retirement is
//!   therefore *not* leaking, and anything left over is leak 1 alone.
//! - The changing-scene row runs free-running (no `MFB_CANVAS_SYNC`) on purpose:
//!   every present publishes, they outpace the renderer's completed frames, and the
//!   gate does not fire. Under `MFB_CANVAS_SYNC` the same program is already flat, so
//!   a sync-only test would have been green against the bug.
//!
//! These are statements about *storage*. What a scene draws is not this suite's
//! business and must not change — `rt_canvas_golden` and `rt_canvas_rasteriser` are
//! the guard for that, and they have to stay green alongside these.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

/// The two frame counts each row is measured at. The assertion is on the *difference*
/// between them, so whatever the program holds for its own reasons — the installed
/// scene, the geometry cache, the arena's own chunking, and the retirements still in
/// flight when it exits — cancels, and only per-present growth survives.
///
/// 2x rather than a larger spread because the quantity is an exact byte count rather
/// than a measurement: 120 presents of the scene below leaked 1.9 MB, which is an order
/// of magnitude outside the slack below.
const FEW: usize = 120;
const MANY: usize = 240;

/// Milliseconds each program sleeps between presents, i.e. ~60 Hz — the rate the bug
/// report measured at, and a deliberate part of what these rows assert.
///
/// A present retires the block it displaces and cannot free it until a frame has
/// *completed*, so a program presenting faster than the renderer draws holds every
/// scene it published in between. That is the design, not a leak: which blocks are
/// still readable is the schedule's answer, and the alternative is freeing one the
/// renderer is mid-copy of. An unpaced `WHILE` loop presents thousands of times per
/// rendered frame and so holds thousands of scenes — measured at 905 KB over 200
/// presents on the FIXED compiler, purely as retirements in flight.
///
/// So the contract is about a program that animates, and `os::sleep` is what makes
/// these programs animate rather than spin. It also keeps the failure honest: at 60 Hz
/// the steady state is one or two retirements, and anything linear in the frame count
/// is the bug.
const FRAME_MS: usize = 16;

/// Items in the scene. Enough that one leaked copy is unmistakable (16 KB), few enough
/// that the program builds 240 of them quickly. The scene is deliberately smaller than
/// `__CANVAS_GEO_CAPACITY` (256) so this does not become a second geometry-cache test
/// — bug-682 covers the arena behind that cache, and this is the scene ring in front
/// of it.
const ITEMS: usize = 40;

/// Bytes of per-run slack tolerated between the two frame counts.
///
/// Not zero, because the two runs are separate processes: the arena's free lists are
/// not obliged to land identically, and each exits holding whatever retirements the
/// last frame left. Both of those are bounded by a couple of scene copies (measured:
/// 16 KB between the two counts of the changing row). Far below one leaked scene copy
/// per present — 16,048 B × 120 = 1.9 MB — which is the smallest failure this could
/// miss.
const SLACK: u64 = 512 * 1024;

/// `arena.0.live_bytes` at exit for `source` with `{N}` replaced by `frames`.
///
/// `arena.0` is the main thread's arena — the one the worker allocates the scene copy
/// in, and the only one permitted to free it (`.ai/canvas-threading.md` §3 "Who
/// frees"). The graphics thread's arena is `arena.1` and is not this bug.
fn live_bytes(name: &str, source: &str, frames: usize, sync: bool) -> u64 {
    let program = source.replace("{N}", &frames.to_string());
    let project = common::temp_project(name, &program);
    let binary = common::build_app_debug(&project, name);
    let mut run = Command::new(&binary);
    run.current_dir(&project)
        .env("MFB_MACAPP_HEADLESS", "1")
        .env("MFB_WINAPP_HEADLESS", "1")
        .env("MFB_GTKAPP_HEADLESS", "1");
    if sync {
        run.env("MFB_CANVAS_SYNC", "1");
    }
    let output = run
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", binary.display()));
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "{name} at {frames} frames {}:\n{stdout}\n{stderr}",
        common::exit_description(&output.status),
    );
    let lines = common::debug_report::arena_lines(name, &stderr);
    let bytes = common::debug_report::counter(name, &lines, 0, "live_bytes");
    let _ = std::fs::remove_dir_all(&project);
    bytes
}

/// The arena holds the same bytes at `MANY` presents as at `FEW`.
fn assert_flat(name: &str, source: &str, sync: bool, why: &str) {
    let few = live_bytes(name, source, FEW, sync);
    let many = live_bytes(name, source, MANY, sync);
    let grew = many.saturating_sub(few);
    assert!(
        grew <= SLACK,
        "{name}: the arena held {few} live bytes after {FEW} presents and {many} after \
         {MANY} — {grew} bytes of growth, {} per extra present. {why} Presenting in a \
         loop must reach a steady state (bug-683).",
        grew / (MANY - FEW) as u64,
    );
}

/// The identical scene, presented `{N}` times. Every present after the first compares
/// equal and takes the frame skip.
fn unchanged() -> String {
    format!(
        "IMPORT app\nIMPORT canvas\nIMPORT color\nIMPORT collections\nIMPORT io\nIMPORT os\n\n\
         SUB main()\n  \
         app::setMode(app::Mode.Canvas)\n  \
         LET paint AS canvas::Paint = canvas::fill(color::rgb(255, 255, 255))\n  \
         MUT items AS List OF canvas::DrawItem = []\n  \
         MUT i AS Integer = 0\n  \
         WHILE i < {ITEMS}\n    \
         LET r AS canvas::DrawItem = canvas::Rectangle[x := toFloat(i), y := 0.0, w := 10.0, h := 10.0, paint := paint]\n    \
         items = collections::append(items, r)\n    \
         i = i + 1\n  \
         END WHILE\n  \
         MUT frame AS Integer = 0\n  \
         WHILE frame < {{N}}\n    \
         canvas::present(items)\n    os::sleep({FRAME_MS})\n    \
         frame = frame + 1\n  \
         END WHILE\n  \
         io::print(\"presented\")\n\
         END SUB\n"
    )
}

/// A scene whose coordinates move every frame, presented **twice** per frame with
/// different content each time — back to back, with the frame's sleep after the pair.
///
/// Two publishes inside one rendered frame is the trigger leak 2 needs, and pairing
/// them like this is what makes it deterministic rather than a race against the
/// renderer. One paced present per frame does *not* reproduce it with a scene this
/// light: the renderer finishes well inside 16 ms, the frame counter advances between
/// presents, the gate fires every time and the single retired slot was enough. The bug
/// report hit it at one present per frame only because its scene — 95 polygons of 400
/// points — took the renderer longer than a frame to draw. Two presents in a row need
/// no such assumption about how fast anything is.
fn changing() -> String {
    format!(
        "IMPORT app\nIMPORT canvas\nIMPORT color\nIMPORT collections\nIMPORT io\nIMPORT os\n\n\
         SUB main()\n  \
         app::setMode(app::Mode.Canvas)\n  \
         LET paint AS canvas::Paint = canvas::fill(color::rgb(255, 255, 255))\n  \
         MUT frame AS Integer = 0\n  \
         WHILE frame < {{N}}\n    \
         MUT items AS List OF canvas::DrawItem = []\n    \
         MUT i AS Integer = 0\n    \
         WHILE i < {ITEMS}\n      \
         LET r AS canvas::DrawItem = canvas::Rectangle[x := toFloat(frame * {ITEMS} + i), y := 0.0, w := 10.0, h := 10.0, paint := paint]\n      \
         items = collections::append(items, r)\n      \
         i = i + 1\n    \
         END WHILE\n    \
         canvas::present(items)\n    \
         LET extra AS canvas::DrawItem = canvas::Rectangle[x := toFloat(frame), y := 20.0, w := 4.0, h := 4.0, paint := paint]\n    items = collections::append(items, extra)\n    \
         canvas::present(items)\n    os::sleep({FRAME_MS})\n    \
         frame = frame + 1\n  \
         END WHILE\n  \
         io::print(\"presented\")\n\
         END SUB\n"
    )
}

/// The same moving scene, paired the same way, in the layered shape through
/// `canvas::presentLayers`.
fn changing_layers() -> String {
    format!(
        "IMPORT app\nIMPORT canvas\nIMPORT color\nIMPORT collections\nIMPORT io\nIMPORT os\n\n\
         SUB main()\n  \
         app::setMode(app::Mode.Canvas)\n  \
         LET paint AS canvas::Paint = canvas::fill(color::rgb(255, 255, 255))\n  \
         MUT frame AS Integer = 0\n  \
         WHILE frame < {{N}}\n    \
         MUT items AS List OF canvas::DrawItem = []\n    \
         MUT i AS Integer = 0\n    \
         WHILE i < {ITEMS}\n      \
         LET r AS canvas::DrawItem = canvas::Rectangle[x := toFloat(frame * {ITEMS} + i), y := 0.0, w := 10.0, h := 10.0, paint := paint]\n      \
         items = collections::append(items, r)\n      \
         i = i + 1\n    \
         END WHILE\n    \
         LET layer AS canvas::DrawLayer = canvas::DrawLayer[items := items]\n    \
         canvas::presentLayers([layer])\n    \
         LET extra AS canvas::DrawItem = canvas::Rectangle[x := toFloat(frame), y := 20.0, w := 4.0, h := 4.0, paint := paint]\n    items = collections::append(items, extra)\n    \
         canvas::presentLayers([canvas::DrawLayer[items := items]])\n    os::sleep({FRAME_MS})\n    \
         frame = frame + 1\n  \
         END WHILE\n  \
         io::print(\"presented\")\n\
         END SUB\n"
    )
}

/// Leak 1: re-presenting an unchanged scene frees the copy it made to compare with.
///
/// `mfb man canvas` promises "re-presenting an unchanged scene is a no-op". It was the
/// most expensive call in the library: the deep copy still happens (that is the design
/// — the comparison needs something to compare), but the `skip` exit returned without
/// giving it back. Run under `MFB_CANVAS_SYNC` so retirement is never the explanation.
#[test]
fn re_presenting_an_unchanged_scene_holds_steady_memory() {
    assert_flat(
        "canvas_present_leak_unchanged",
        &unchanged(),
        true,
        "The frame skip deep-copies the scene to compare it, then returns from the \
         `skip` label without freeing the copy, so every no-op re-present leaks a \
         whole scene.",
    );
}

/// Leak 2: retirement absorbs more than one retirement per rendered frame.
///
/// Free-running on purpose — presents outpace completed frames, which is the only
/// schedule where the single retired slot was overwritten. The fix must keep the drain
/// gate exactly: a displaced block may be the one the renderer is copying right now,
/// and freeing it at the publish trades this leak for a use-after-free.
#[test]
fn presenting_a_changing_scene_holds_steady_memory() {
    assert_flat(
        "canvas_present_leak_changing",
        &changing(),
        false,
        "The retire loop overwrites the single retired slot whether or not the frame \
         gate let the block already there be freed, so any present that lands inside \
         an already-retired frame loses that block's pointer forever.",
    );
}

/// `presentLayers` shares `emit_publish`, so it shares both leaks and the same fix.
/// Its own row because the shape picks different slots (`CANVAS_SCENE_RETIRED_LAYERS`)
/// and a fix applied to one shape's offsets only would pass the two rows above.
#[test]
fn presenting_changing_layers_holds_steady_memory() {
    assert_flat(
        "canvas_present_leak_layers",
        &changing_layers(),
        false,
        "`presentLayers` publishes through the same retire/reclaim as `present`, into \
         the layered slots.",
    );
}
