//! The geometry arena `__CANVAS_GEO_DATA` is bounded on a scene whose items change
//! every frame (bug-682).
//!
//! The cache *index* has always been bounded — `__CANVAS_GEO_CAPACITY` is 256 and
//! `__canvas_geoEvict` drops the least-recently-used slot. The backing store was not:
//! eviction dropped the slot and left the floats it owned in the arena forever, so a
//! program animating by re-presenting changed items grew the arena by one item's
//! geometry per miss and never gave any of it back. Measured on the program below at
//! 100 frames: `floats=282000` and a 21.6 GB peak RSS.
//!
//! Both assertions below are statements about *storage*, not about pixels. What a
//! scene draws is not this suite's business and must not change — `rt_canvas_golden`
//! and `rt_canvas_rasteriser` are the guard for that, and they have to stay green
//! alongside these.
//!
//! The numbers are read off `MFB_CANVAS_STATS`, which is the only window onto state
//! the graphics thread owns (`.ai/canvas-threading.md` §11): `floats=` is
//! `len(__CANVAS_GEO_DATA)`, `entries=` is the number of live cache slots, and
//! `geoCompactions=` counts the reclamation passes.

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

/// Frames the animating program presents. Every frame is a full miss on every item,
/// so the arena grows by `ITEMS` entries per frame on the unfixed compiler.
///
/// Kept small on purpose. The unfixed cost is **quadratic** — each miss copies the
/// whole arena — so this is not a knob that can be turned up for a clearer signal
/// without turning the suite into a minute-long test: 100 frames of this program took
/// 75 s and 21.6 GB before the fix, where 30 takes about seven seconds. Thirty frames
/// already separates the two behaviours by 2x, which is far outside any noise, because
/// the quantity is a deterministic count of floats rather than a measurement.
const FRAMES: usize = 30;

/// Items per frame, each with coordinates that differ from the previous frame's.
///
/// Fewer than `__CANVAS_GEO_CAPACITY` (256) deliberately: the scene is **not** larger
/// than the cache, so this test does not depend on the cache thrashing. Every probe
/// misses because the *content* changed, which is the ordinary shape of an animation
/// and the case the bug is about. A scene that overflowed the cache would also
/// reproduce it, but would confound "the cache is too small" with "the arena is never
/// reclaimed", and only the second is the bug.
const ITEMS: usize = 60;

/// A `canvas::Line` entry is a 22-float header plus a 25-float tail. The steady-state
/// arena is bounded by what the 256 live slots own; the reclamation pass runs at a
/// frame boundary and tolerates some slack before it pays for a rebuild, and a frame
/// can add `ITEMS` entries between two of them.
///
/// This bound is generous — roughly three times the live set — because the exact
/// trigger point is an implementation choice this test has no business pinning. What
/// it must not tolerate is growth *with the frame count*, which is what the flatness
/// assertion covers and what `floats=84600` at 30 frames on the unfixed compiler is.
const ARENA_BOUND: i64 = 40_000;

/// Build a `--debug --app` program, run it headless and synchronously, and return one
/// `MFB_CANVAS_STATS` line per frame.
fn stats(name: &str, source: &str) -> Vec<String> {
    let project = common::temp_project(name, source);
    let binary = common::build_app_debug(&project, name);
    let stats = project.join("stats.txt");
    let run = Command::new(&binary)
        .current_dir(&project)
        .env("MFB_MACAPP_HEADLESS", "1")
        .env("MFB_WINAPP_HEADLESS", "1")
        .env("MFB_GTKAPP_HEADLESS", "1")
        .env("MFB_CANVAS_STATS", &stats)
        // Synchronous, or presents coalesce and the frame count is the scheduler's
        // answer rather than the program's — and every number below is per frame.
        .env("MFB_CANVAS_SYNC", "1")
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", binary.display()));
    assert!(
        run.status.success(),
        "program {}:\n{}\n{}",
        common::exit_description(&run.status),
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr),
    );
    let lines: Vec<String> = std::fs::read_to_string(&stats)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect();
    let _ = std::fs::remove_dir_all(&project);
    lines
}

/// One `name=value` field of a stats line, as a number.
fn field(line: &str, name: &str) -> i64 {
    line.split_whitespace()
        .find_map(|f| f.strip_prefix(&format!("{name}=")))
        .unwrap_or_else(|| panic!("no `{name}=` field in {line:?}"))
        .parse()
        .unwrap_or_else(|e| panic!("`{name}` is not a number in {line:?}: {e}"))
}

/// `ITEMS` lines whose endpoints move every frame, presented `FRAMES` times.
fn animating() -> String {
    format!(
        "IMPORT app\nIMPORT canvas\nIMPORT color\nIMPORT collections\nIMPORT io\n\n\
         SUB main()\n  \
         app::setMode(app::Mode.Canvas)\n  \
         LET paint AS canvas::Paint = canvas::stroke(color::rgb(255, 255, 255), 1.0)\n  \
         MUT frame AS Integer = 0\n  \
         WHILE frame < {FRAMES}\n    \
         MUT items AS List OF canvas::DrawItem = []\n    \
         MUT i AS Integer = 0\n    \
         WHILE i < {ITEMS}\n      \
         LET t AS Float = toFloat(frame * {ITEMS} + i)\n      \
         LET seg AS canvas::DrawItem = canvas::Line[x1 := t, y1 := toFloat(i), x2 := t + 10.0, y2 := toFloat(i) + 10.0, cap := canvas::CapStyle.Butt, paint := paint]\n      \
         items = collections::append(items, seg)\n      \
         i = i + 1\n    \
         END WHILE\n    \
         canvas::present(items)\n    \
         frame = frame + 1\n  \
         END WHILE\n  \
         io::print(\"rendered\")\n\
         END SUB\n"
    )
}

/// The arena reaches a steady size and stays there while the program keeps animating.
///
/// Two assertions, and they fail differently on purpose. Flatness is the *contract* —
/// "a program can animate indefinitely at constant RSS" — and it is what a fix that
/// merely slowed the growth down would still fail. The absolute bound is the sanity
/// check underneath it: a fix that reclaimed nothing but happened to report equal
/// numbers at the two sampled frames would pass flatness and fail this.
///
/// On the compiler this test was written against: `floats=42300` at frame 15 and
/// `floats=84600` at frame 30 — exactly linear in the frame number, because every one
/// of the 1800 misses appended 47 floats that nothing ever reclaimed.
#[test]
fn a_changing_scene_holds_a_bounded_geometry_arena() {
    let lines = stats("canvas_geo_arena", &animating());
    assert_eq!(
        lines.len(),
        FRAMES,
        "every present changes every item, so every one must render: {lines:?}",
    );

    let mid = &lines[FRAMES / 2 - 1];
    let last = &lines[FRAMES - 1];
    let (mid_floats, last_floats) = (field(mid, "floats"), field(last, "floats"));

    assert!(
        last_floats <= mid_floats + mid_floats / 4,
        "the geometry arena grew from {mid_floats} floats at frame {} to {last_floats} \
         at frame {FRAMES}. `__canvas_geoEvict` drops an evicted entry's SLOT and \
         leaves the floats it owned in `__CANVAS_GEO_DATA` forever, so the arena is \
         linear in the number of frames presented and the program ends in an OOM kill \
         — 21.6 GB peak RSS at 100 frames of this program. Presenting a changing scene \
         must reach a steady state instead (bug-682).\nmid:  {mid}\nlast: {last}",
        FRAMES / 2,
    );

    let entries = field(last, "entries");
    assert!(
        last_floats <= ARENA_BOUND,
        "the arena holds {last_floats} floats for {entries} live cache entries, over \
         the {ARENA_BOUND} this scene should need. The steady-state size must be \
         bounded by the cache capacity and the scene, not by the frame count \
         (bug-682).\n{last}",
    );
}

/// A scene that does **not** change pays nothing for the reclamation pass.
///
/// The contrast case from the bug report, and the reason the pass is gated rather than
/// unconditional: ~95 identical-content items against a 256-entry cache are all hits,
/// `__canvas_geometryFor` returns at the probe loop, and the arena holds exactly what
/// its live entries own with no slack to reclaim. Rebuilding it every frame anyway
/// would add an O(arena) copy per frame to every static canvas program — a cost the
/// bug did not have and a fix must not introduce.
///
/// Presented with a moving item alongside the static ones so the frames are not
/// skipped outright by the damage diff; the assertion is on the arena, which the one
/// moving item cannot grow past its own entry.
#[test]
fn an_all_hit_scene_never_pays_for_reclamation() {
    let source = format!(
        "IMPORT app\nIMPORT canvas\nIMPORT color\nIMPORT collections\nIMPORT io\n\n\
         SUB main()\n  \
         app::setMode(app::Mode.Canvas)\n  \
         LET paint AS canvas::Paint = canvas::stroke(color::rgb(255, 255, 255), 1.0)\n  \
         MUT frame AS Integer = 0\n  \
         WHILE frame < {FRAMES}\n    \
         MUT items AS List OF canvas::DrawItem = []\n    \
         MUT i AS Integer = 0\n    \
         WHILE i < {ITEMS}\n      \
         LET seg AS canvas::DrawItem = canvas::Line[x1 := toFloat(i), y1 := 0.0, x2 := toFloat(i), y2 := 100.0, cap := canvas::CapStyle.Butt, paint := paint]\n      \
         items = collections::append(items, seg)\n      \
         i = i + 1\n    \
         END WHILE\n    \
         LET dot AS canvas::DrawItem = canvas::Circle[x := toFloat(frame) * 4.0, y := 300.0, radius := 5.0, paint := canvas::fill(color::rgb(255, 0, 0))]\n    \
         items = collections::append(items, dot)\n    \
         canvas::present(items)\n    \
         frame = frame + 1\n  \
         END WHILE\n  \
         io::print(\"rendered\")\n\
         END SUB\n"
    );
    let lines = stats("canvas_geo_arena_static", &source);
    let last = lines.last().expect("at least one frame");
    assert_eq!(
        field(last, "geoCompactions"),
        0,
        "a scene whose items are all cache hits has no dead floats to reclaim, so the \
         pass must not run at all. Running it unconditionally would add an O(arena) \
         rebuild to every frame of every static canvas program.\n{last}",
    );
}

/// A program whose items are `count` static rectangles plus one circle that moves every
/// frame, presented `frames` times. Every frame is a new scene (the circle moved), so
/// every one renders, while only one item's geometry actually changed.
fn static_bulk(count: usize, frames: usize) -> String {
    format!(
        "IMPORT app\nIMPORT canvas\nIMPORT color\nIMPORT collections\nIMPORT io\n\n\
         SUB main()\n  \
         app::setMode(app::Mode.Canvas)\n  \
         LET paint AS canvas::Paint = canvas::fill(color::rgb(0, 200, 255))\n  \
         MUT bulk AS List OF canvas::DrawItem = []\n  \
         MUT i AS Integer = 0\n  \
         WHILE i < {count}\n    \
         LET r AS canvas::DrawItem = canvas::Rectangle[x := toFloat((i * 37) MOD 880), y := toFloat((i * 53) MOD 620), w := 6.0, h := 6.0, paint := paint]\n    \
         bulk = collections::append(bulk, r)\n    \
         i = i + 1\n  \
         END WHILE\n  \
         MUT frame AS Integer = 0\n  \
         WHILE frame < {frames}\n    \
         MUT items AS List OF canvas::DrawItem = bulk\n    \
         LET dot AS canvas::DrawItem = canvas::Circle[x := toFloat(frame) * 4.0 + 10.0, y := 630.0, radius := 5.0, paint := canvas::fill(color::rgb(255, 0, 0))]\n    \
         items = collections::append(items, dot)\n    \
         canvas::present(items)\n    \
         frame = frame + 1\n  \
         END WHILE\n  \
         io::print(\"rendered\")\n\
         END SUB\n"
    )
}

/// `count` lines whose endpoints move every frame, presented `frames` times — the
/// animation above, widened past the old 256-entry cache.
fn moving_bulk(count: usize, frames: usize) -> String {
    animating()
        .replace(
            &format!("WHILE frame < {FRAMES}"),
            &format!("WHILE frame < {frames}"),
        )
        .replace(&format!("WHILE i < {ITEMS}"), &format!("WHILE i < {count}"))
        .replace(
            &format!("frame * {ITEMS} + i"),
            &format!("frame * {count} + i"),
        )
}

/// A scene larger than the old 256-entry cache whose items do not change builds each
/// item's geometry once, not once per frame (bug-686 Phase 1).
///
/// `generations=` counts geometry builds (`__CANVAS_GEO_GENERATIONS`). On the compiler
/// this was written against, a 1,000-item static scene built **2 per item per frame**:
/// the 256-entry cache thrashed to a 100% miss rate, and the frame walked the scene
/// twice (`__canvas_sceneOffsets`, then `__canvas_sceneDraws`), missing both times.
/// After warm-up only the one moving circle may be rebuilt.
#[test]
fn a_static_scene_larger_than_the_old_cache_builds_its_geometry_once() {
    const COUNT: usize = 1000;
    const RUN: usize = 8;
    let lines = stats("canvas_geo_static_bulk", &static_bulk(COUNT, RUN));
    assert_eq!(lines.len(), RUN, "one frame per present: {lines:?}");
    for pair in lines.windows(2).skip(1) {
        let built = field(&pair[1], "generations") - field(&pair[0], "generations");
        assert!(
            built <= 1,
            "a frame of {COUNT} unchanged rectangles plus one moving circle rebuilt \
             {built} items' geometry. Only the circle changed, so at most 1 may be \
             rebuilt once the cache is warm.\nbefore: {}\nafter:  {}",
            pair[0],
            pair[1],
        );
    }
}

/// A scene whose every item changes each frame builds each item's geometry exactly once
/// per frame, not once per walk of the scene (bug-686 Phase 1).
///
/// Every item misses because its content changed, so `COUNT` builds a frame is the
/// floor; the compiler this was written against built `2 × COUNT`, because
/// `__canvas_sceneDraws` resolved every item again after `__canvas_sceneOffsets` had
/// just built it.
#[test]
fn a_moving_scene_builds_each_item_once_per_frame() {
    const COUNT: usize = 300;
    const RUN: usize = 6;
    let lines = stats("canvas_geo_moving_bulk", &moving_bulk(COUNT, RUN));
    assert_eq!(lines.len(), RUN, "one frame per present: {lines:?}");
    for pair in lines.windows(2) {
        let built = field(&pair[1], "generations") - field(&pair[0], "generations");
        assert!(
            built <= COUNT as i64,
            "a frame of {COUNT} changed lines built {built} geometries: each item must \
             be built once per frame, not once per walk of the scene.\nbefore: {}\n\
             after:  {}",
            pair[0],
            pair[1],
        );
    }
}
