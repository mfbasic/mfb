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

/// The growth budget has two terms, and it needs both.
///
/// [`FIXED_SLACK`] is what a run may hold *regardless of how many frames it ran*: the
/// two counts are separate processes, and each exits holding whatever retirements its
/// last frame left. Measured at 16,464 bytes between the two counts of the free-running
/// changing row — a scene copy and change — and exactly **zero** for the paced rows,
/// which report the same byte count at 120, 240 and 480 presents.
///
/// [`MAX_PER_PRESENT`] is the part that actually catches leaks, because a leak is
/// per-present by definition and a fixed allowance divided by the frame count is not.
/// A single flat allowance was the first version of this file and it was useless: at
/// 512 KB over a 120-present delta it tolerated 4,369 bytes per present, which
/// swallowed bug-684's 528 whole — both group rows passed against the unfixed compiler.
///
/// The per-present budget is **per row**, because what a row can distinguish depends on
/// its scheduler:
///
/// * A `MFB_CANVAS_SYNC` row drains its retirement list on every present, so its steady
///   state is exact — measured at *zero* bytes of growth between 120 and 240 presents.
///   [`TIGHT`] is what those rows use, and it is what catches bug-684's ~1,300.
/// * A free-running row legitimately holds every scene it published since the last
///   frame tick (see [`FRAME_MS`]), and how many that is depends on machine load. The
///   same layers row measured 0 bytes of growth on an idle machine and 1,129 per
///   present with five of these running in parallel. [`LOOSE`] is what those rows use.
///   They are catching 16,048–41,657 per present, so an order of magnitude of headroom
///   costs them nothing.
///
/// Giving every row [`LOOSE`] is what the first version of this file did, and it made
/// the two bug-684 rows useless — 4,369 per present of tolerance swallows a 1,300
/// per-present leak, and both passed against the unfixed compiler. Giving every row
/// [`TIGHT`] makes the two free-running rows flake with the load on the machine.
const FIXED_SLACK: u64 = 48 * 1024;
const TIGHT: u64 = 64;
const LOOSE: u64 = 4096;

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
fn assert_flat(name: &str, source: &str, sync: bool, per_present: u64, why: &str) {
    let few = live_bytes(name, source, FEW, sync);
    let many = live_bytes(name, source, MANY, sync);
    let grew = many.saturating_sub(few);
    let extra = (MANY - FEW) as u64;
    let budget = FIXED_SLACK + per_present * extra;
    assert!(
        grew <= budget,
        "{name}: the arena held {few} live bytes after {FEW} presents and {many} after \
         {MANY} — {grew} bytes of growth over {extra} extra presents, {} per present, \
         against a budget of {budget} ({FIXED_SLACK} fixed + {per_present}/present). \
         {why} Presenting in a loop must reach a steady state.",
        grew / extra,
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
        TIGHT,
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
        LOOSE,
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
        LOOSE,
        "`presentLayers` publishes through the same retire/reclaim as `present`, into \
         the layered slots.",
    );
}

/// A scene whose own bytes never change while the **group** it names is rebuilt every
/// frame — the one schedule on which `publishHashes` runs and `publishScene` does not
/// (bug-684).
///
/// `vary` picks which of the two halves of that sentence holds:
///
/// * `0` — the `Group` node is byte-identical every frame, so `publishScene` takes the
///   frame skip. It is still a new *frame*, because `__canvas_present` compares the
///   group signature separately and `setGroup` moved the group's revision — so
///   `installed` is FALSE, `moved` is TRUE, and the body publishes hashes without
///   publishing a scene. Nothing retires the hashes block that displaces.
/// * `1` — the node's `dx` moves with the frame, so `publishScene` publishes, and its
///   retire captures the displaced hashes along with the items. The contrast case, and
///   a **positive pin**: it must stay flat, so a fix that stopped publishing hashes
///   altogether would be caught here rather than passing.
/// Static items in the scene, beside the one `Group` node.
///
/// They are what give the leak its size: the hash list is one entry per scene item, so
/// the block `publishHashes` displaces — the thing bug-684 dropped — is proportional to
/// this. At 60 it was 528 bytes a present, which is real but close enough to the noise
/// floor to make a weak test; at 150 it is ~1,300, an order of magnitude over
/// [`MAX_PER_PRESENT`].
///
/// Still under `__CANVAS_GEO_CAPACITY` (256), so the geometry cache does not thrash and
/// this stays a test about the hash block rather than a second `rt_canvas_geo_arena`.
const SCENE_ITEMS: usize = 150;

fn group_animated(vary: usize) -> String {
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
         canvas::setGroup(\"panel\", items)\n    \
         MUT scene AS List OF canvas::DrawItem = []\n    \
         MUT k AS Integer = 0\n    \
         WHILE k < {SCENE_ITEMS}\n      \
         LET st AS canvas::DrawItem = canvas::Rectangle[x := toFloat(k), y := 50.0, w := 4.0, h := 4.0, paint := paint]\n      \
         scene = collections::append(scene, st)\n      \
         k = k + 1\n    \
         END WHILE\n    \
         LET g AS canvas::DrawItem = canvas::Group[name := \"panel\", dx := toFloat(frame * {vary}), dy := 0.0]\n    \
         scene = collections::append(scene, g)\n    \
         canvas::present(scene)\n    \
         os::sleep({FRAME_MS})\n    \
         frame = frame + 1\n  \
         END WHILE\n  \
         io::print(\"presented\")\n\
         END SUB\n"
    )
}

/// bug-684: `canvas::publishHashes` retires the hash block it displaces.
///
/// It used to overwrite `CANVAS_SCENE_HASHES_OFFSET` and drop the old pointer. That was
/// survivable only by accident: on the ordinary path the *next* `publishScene` retires
/// whatever is in the hashes slot, so the block a `publishHashes` displaced got picked
/// up one publish late. A group animating under an unchanged scene never takes that
/// path — `publishScene` skips — and nothing else reclaims it.
///
/// It must be **retired**, not freed: `__canvas_sceneDraws` and `__canvas_sceneOffsets`
/// both read the installed hashes through `canvas::installedHashes` at arbitrary points
/// inside a frame, so the displaced block can be one a render in flight is reading.
///
/// Runs under `MFB_CANVAS_SYNC`, unlike the two rows above. This leak needs no race —
/// it was measured at the same rate synchronous and free-running — and sync is what
/// makes the steady state exact (zero bytes of growth) rather than load-dependent,
/// which is what lets this row use [`TIGHT`] and so see a 1,300-byte leak at all.
#[test]
fn animating_a_group_under_an_unchanged_scene_holds_steady_memory() {
    assert_flat(
        "canvas_present_leak_group",
        &group_animated(0),
        true,
        TIGHT,
        "`canvas::publishHashes` overwrote the installed hashes pointer without \
         retiring the block it displaced. When the scene itself is unchanged \
         `publishScene` skips, so the retire that used to cover for it never runs.",
    );
}

/// The contrast: the same program with a scene that does change, so `publishScene`
/// publishes and its retire is what reclaims the hashes.
///
/// Flat before bug-684's fix and flat after — a positive pin on the path that already
/// worked, so a fix cannot buy the row above by breaking this one.
#[test]
fn animating_a_group_under_a_changing_scene_holds_steady_memory() {
    assert_flat(
        "canvas_present_leak_group_moving",
        &group_animated(1),
        true,
        TIGHT,
        "A publishing present retires the displaced hashes with the rest of the scene; \
         this path was never the leak and must stay flat.",
    );
}
