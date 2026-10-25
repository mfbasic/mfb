//! The native geometry builder (`canvas::geoBuild`) and the native structural item hash
//! (`canvas::itemHash` / `canvas::sceneHashes`) — bug-686.
//!
//! `canvas::geoBuild` replaces the MFBASIC header builders for the common kinds, and its
//! record must be **bit-identical** to theirs: the software rasteriser reads it, and its
//! goldens are exact. A `--debug` build run with `MFB_CANVAS_GEO_VERIFY=1` rebuilds every
//! natively built record with the MFBASIC builders and counts the ones that differ in any
//! bit. The stats line reports `geoNative=` (records the native builder produced),
//! `geoVerified=` (records compared) and `geoVerifyMismatches=`.
//!
//! The hash half is checked by what it must do for the cache: an identical scene rebuilt
//! from scratch — whose lists carry different headroom, so its bytes differ — must hit.

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

/// Build a `--debug --app` program, run it headless, synchronously and with the geometry
/// check on, and return one `MFB_CANVAS_STATS` line per frame.
fn stats(name: &str, source: &str) -> Vec<String> {
    stats_with(name, source, true)
}

/// [`stats`], optionally WITHOUT `MFB_CANVAS_SYNC`: the worker then presents as fast as
/// it can while the graphics thread renders whatever is installed, which is the only way
/// a frame can overlap a publish.
fn stats_with(name: &str, source: &str, sync: bool) -> Vec<String> {
    let project = common::temp_project(name, source);
    let binary = common::build_app_debug(&project, name);
    let stats = project.join("stats.txt");
    let mut command = Command::new(&binary);
    command
        .current_dir(&project)
        .env("MFB_HEADLESS", "1")
        .env("MFB_CANVAS_STATS", &stats)
        .env("MFB_CANVAS_GEO_VERIFY", "1");
    if sync {
        command.env("MFB_CANVAS_SYNC", "1");
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

/// Every paint the matrix crosses with every shape: fill and stroke each opaque-ish or
/// fully transparent, four stroke widths (none, thin, fractional, negative), the four
/// blend modes, and no clip / a fractional clip / a negative-width clip. A second group
/// keeps a one-stop gradient (not a gradient, so still native, but its kind and points
/// are still written) and an all-negative-zero transform (the identity by float `=`).
///
/// Every native kind at fractional, negative, large and tiny coordinates, and every
/// degenerate size the builders special-case: zero and negative width, height and
/// radius, a corner radius below zero and past the limit, zero-length and
/// negative-zero lines, polygons of 0, 1 and 2 points, collinear and with a
/// duplicate vertex, and polylines (plan-154-A) with no colours, fewer and more colours
/// than points, repeated end points, every point coincident under each cap, and 0 and
/// 1 points.
///
/// Frame 1 is the whole native matrix. Frame 2 is items the native builder must
/// decline — a real transform, a two-stop gradient, an `Ellipse`, an `Arc` — so the
/// MFBASIC builders still run for them.
///
/// The paints (`paints()`) and shapes (`shapes()`) live in the scene file. It and the
/// other scenes in `scenes/` are shared with `scripts/test-canvas-gpu-rows.sh`, which
/// runs them on x86-64 and AArch64 Linux and Windows (bug-688).
const MATRIX: &str = include_str!("scenes/canvas_geo_native_matrix.mfb");

/// Every record the native builder produces for the matrix is bit-identical to the one
/// the MFBASIC builders produce, and the native builder is what built every item of it.
#[test]
fn the_native_geometry_matches_the_mfbasic_builders_bit_for_bit() {
    let lines = stats("canvas_geo_native_matrix", MATRIX);
    assert_eq!(lines.len(), 2, "one frame per present: {lines:?}");
    let (matrix, declined) = (&lines[0], &lines[1]);

    let native = field(matrix, "geoNative");
    let built = field(matrix, "generations");
    assert!(
        native > 1000,
        "the matrix is ~7,800 items of the six native kinds (distinct ones: over a \
         thousand), but the native builder produced {native} records.\n{matrix}"
    );
    assert_eq!(
        native, built,
        "every item of the matrix is a native kind with an identity transform and no \
         gradient, so every geometry build must have been the native one.\n{matrix}"
    );
    assert_eq!(
        field(matrix, "geoVerified"),
        native,
        "with MFB_CANVAS_GEO_VERIFY=1 every native record is checked.\n{matrix}"
    );
    assert_eq!(
        field(matrix, "geoVerifyMismatches"),
        0,
        "a native geometry record differs from the MFBASIC builders' in at least one \
         bit. The software rasteriser reads this record and its goldens are exact, so \
         the two must agree slot for slot (`func_geo_build/`).\n{matrix}"
    );

    assert_eq!(
        field(declined, "geoNative"),
        native,
        "a transform, a two-stop gradient, an Ellipse and an Arc are the MFBASIC \
         builders' — the native builder must decline all four.\n{declined}"
    );
    assert_eq!(
        field(declined, "generations") - built,
        4,
        "the four declined items must still be built, by the MFBASIC path.\n{declined}"
    );
    assert_eq!(field(declined, "geoVerifyMismatches"), 0, "{declined}");
}

/// The item hash is STRUCTURAL: the same scene rebuilt from scratch — its point lists
/// built by append (with headroom) instead of by literal, its paints rebuilt — hashes
/// the same, so the geometry cache hits and nothing is rebuilt but the one item that
/// moved. A hash over the item's bytes would miss on every item: the blocks differ in
/// capacity and padding although the values are equal.
#[test]
fn a_rebuilt_identical_scene_hits_the_geometry_cache() {
    let source = include_str!("scenes/canvas_geo_native_rehash.mfb");
    let lines = stats("canvas_geo_native_rehash", source);
    assert_eq!(lines.len(), 3, "one frame per present: {lines:?}");
    assert_eq!(
        field(&lines[0], "generations"),
        101,
        "the first frame builds its 101 items.\n{}",
        lines[0]
    );
    for pair in lines.windows(2) {
        let built = field(&pair[1], "generations") - field(&pair[0], "generations");
        assert_eq!(
            built, 1,
            "a rebuilt scene equal in value to the last one must rebuild only the one \
             item that moved; {built} were rebuilt, so equal items hashed differently.\n\
             before: {}\nafter:  {}",
            pair[0], pair[1],
        );
    }
    assert_eq!(field(&lines[2], "geoVerifyMismatches"), 0, "{}", lines[2]);
}

/// The frame pass itself (`canvas::sceneResolve`, `canvas::sceneLayout`,
/// `canvas::sceneDrawsFlat`), over a moving scene that mixes the kinds it builds with
/// every kind it hands to MFBASIC: a transformed rectangle, an `Ellipse`, a `Picture`,
/// then a `Group` node, then a layered scene. With `MFB_CANVAS_GEO_VERIFY=1` every record
/// it built is rebuilt by the MFBASIC builders, every draw hash it folded is refolded,
/// and every group-free draw list it laid out is laid out again by the MFBASIC walk —
/// all bit for bit. Every item moves every frame, so the arena compacts and the index is
/// rebuilt on most frames.
#[test]
fn the_native_frame_pass_matches_the_mfbasic_walk_on_a_mixed_moving_scene() {
    let source = include_str!("scenes/canvas_geo_native_frame_pass.mfb");
    let lines = stats("canvas_geo_native_frame_pass", source);
    assert_eq!(lines.len(), 9, "one frame per present: {lines:?}");
    let last = &lines[8];
    assert_eq!(
        field(last, "geoVerifyMismatches"),
        0,
        "the native frame pass disagreed with the MFBASIC builders, fold or walk.\n{last}"
    );
    assert!(
        field(last, "geoVerified") >= 8 * 480,
        "every moving shape of the eight frames is built natively and re-checked.\n{last}"
    );
    assert!(
        field(last, "drawsVerified") >= 7,
        "six group-free frames and the layered one are laid out natively and \
         re-checked.\n{last}"
    );
    assert!(
        field(last, "geoCompactions") >= 3,
        "a scene whose items all move compacts the arena at frame boundaries.\n{last}"
    );
    for pair in lines[..8].windows(2) {
        let built = field(&pair[1], "generations") - field(&pair[0], "generations");
        assert!(
            (480..=490).contains(&built),
            "every one of the 480 moving shapes changed, plus the transformed rectangles \
             and ellipses (8) and the picture (1): {built} builds.\nbefore: {}\nafter:  {}",
            pair[0],
            pair[1],
        );
    }
}

/// A frame draws ONE scene: every index it resolves — cache hits included, of every kind
/// — draws the geometry of the item the frame holds at that index (bug-686).
///
/// A hit is trusted on the item's hash alone, and the hashes are published by a second
/// call (`canvas::publishHashes`) after the scene (`canvas::publishScene`). The worker
/// here alternates two scenes of the same length, A and B, at least 60 times and until 3
/// frames are drawn (bug-734), a few milliseconds apart and without `MFB_CANVAS_SYNC`, so
/// the graphics thread renders while publishes land. A and B put
/// different kinds at the same indices and different positions on them, and every item of
/// both is cached after the first two frames — so a frame that pairs one scene's items
/// with the other's hashes draws a HIT of the wrong item, and one that files a miss under
/// the other scene's hash keeps drawing it. Every tenth index is a kind the MFBASIC path
/// builds (an ellipse in A, a transformed rectangle in B).
///
/// `MFB_CANVAS_GEO_VERIFY=1` checks every resolved index of every frame against the
/// MFBASIC builders' record for the item at that index (`geoResolvedChecked=`).
#[test]
fn a_frame_never_draws_one_scenes_geometry_for_anothers_items() {
    let source = include_str!("scenes/canvas_geo_native_publish_race.mfb");
    let lines = stats_with("canvas_geo_native_publish_race", source, false);
    let last = lines.last().expect("at least one frame rendered");
    assert!(
        lines.len() >= 3,
        "the graphics thread rendered only {} frames while the worker presented 60 \
         scenes; the race needs frames to overlap presents.\n{last}",
        lines.len()
    );
    assert!(
        field(last, "geoResolvedChecked") >= 6000,
        "every resolved index of every frame is checked.\n{last}"
    );
    assert_eq!(
        field(last, "geoVerifyMismatches"),
        0,
        "a frame drew geometry that is not its item's: the items, the layers and the \
         hashes a frame reads must come from ONE publish of the scene, and a hit must be \
         taken by the hash published for that scene.\n{last}"
    );
}

/// [`stats`] with the last frame kept too (`MFB_CANVAS_DUMP`), drawn by the software
/// renderer or, with `gpu`, by the GPU backend.
fn stats_and_frame(name: &str, source: &str, gpu: bool) -> (Vec<String>, Vec<u8>) {
    let project = common::temp_project(name, source);
    let binary = common::build_app_debug(&project, name);
    let stats = project.join("stats.txt");
    let frame = project.join("frame.rgba");
    let mut command = Command::new(&binary);
    command
        .current_dir(&project)
        .env("MFB_HEADLESS", "1")
        .env("MFB_CANVAS_SYNC", "1")
        .env("MFB_CANVAS_STATS", &stats)
        .env("MFB_CANVAS_DUMP", &frame)
        .env("MFB_CANVAS_GEO_VERIFY", "1");
    if gpu {
        command.env("MFB_CANVAS_GPU", "1");
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
    let lines: Vec<String> = std::fs::read_to_string(&stats)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect();
    let pixels = std::fs::read(&frame).expect("canvas dump written");
    let _ = std::fs::remove_dir_all(&project);
    (lines, pixels)
}

/// plan-175-A Phase 2: A, B, A, where B changes only paint, of every kind the walk
/// handles -- the natively resolved ones and the ones the MFBASIC walk rebuilds.
const REPAINT: &str = include_str!("scenes/canvas_geo_native_repaint.mfb");

/// A change of paint alone builds no record and inserts none: every record is found by
/// its item's geometry identity and rewritten in place (`refreshes=`), and every
/// rewritten record is bit-identical to a fresh build of the item (the verifier).
#[test]
fn a_paint_only_change_builds_nothing_and_refreshes_every_record() {
    let lines = stats("canvas_geo_native_repaint", REPAINT);
    assert_eq!(lines.len(), 3, "one frame per present: {lines:?}");
    let built = field(&lines[0], "generations");
    let entries = field(&lines[0], "entries");
    assert!(
        built > 400,
        "the first frame builds every record.\n{}",
        lines[0]
    );
    for pair in lines.windows(2) {
        assert_eq!(
            field(&pair[1], "generations"),
            built,
            "a frame that changed only paint built a record: its key is not geometry \
             alone.\nbefore: {}\nafter:  {}",
            pair[0],
            pair[1],
        );
        assert_eq!(
            field(&pair[1], "entries"),
            entries,
            "a frame that changed only paint added a cache entry.\n{}",
            pair[1],
        );
        assert_eq!(
            field(&pair[1], "refreshes") - field(&pair[0], "refreshes"),
            entries,
            "every record of a frame that changed every item's paint is refreshed.\n\
             before: {}\nafter:  {}",
            pair[0],
            pair[1],
        );
    }
    let last = lines.last().unwrap();
    assert_eq!(field(last, "geoVerifyMismatches"), 0, "{last}");
    assert!(field(last, "geoResolvedChecked") > 0, "{last}");
}

/// A frame drawn from refreshed records is the frame a fresh run on that scene draws,
/// byte for byte, on the software renderer and on the GPU: the run that went A, B, A
/// ends on records refreshed twice; the other draws A once, from records built once.
#[test]
fn a_frame_after_a_refresh_is_the_fresh_runs_frame() {
    let once = REPAINT.replace(
        "  canvas::present(scene(img, FALSE))\n  canvas::present(scene(img, TRUE))\n  canvas::present(scene(img, FALSE))\n",
        "  canvas::present(scene(img, FALSE))\n",
    );
    assert_ne!(once, REPAINT, "the scene presents A, B, A");
    for gpu in [false, true] {
        let (refreshed_lines, refreshed) =
            stats_and_frame(&format!("canvas_geo_native_refreshed_{gpu}"), REPAINT, gpu);
        let (fresh_lines, fresh) =
            stats_and_frame(&format!("canvas_geo_native_fresh_{gpu}"), &once, gpu);
        assert!(
            field(refreshed_lines.last().unwrap(), "refreshes") > 0,
            "the A, B, A run refreshed nothing.\n{refreshed_lines:?}"
        );
        assert_eq!(field(fresh_lines.last().unwrap(), "refreshes"), 0);
        assert!(
            refreshed == fresh,
            "gpu={gpu}: the frame drawn from refreshed records differs from a fresh \
             run's in {} bytes",
            refreshed.iter().zip(&fresh).filter(|(a, b)| a != b).count()
        );
    }
}

/// Each appearance field alone, then each geometry predicate alone, one per frame:
/// frame `f` changes the one thing its `IF f >= ...` line says.
const LANES: &str = include_str!("scenes/canvas_geo_native_lanes.mfb");

/// plan-175-A Phase 2, and the value tests Phase 1 left here (its Correction 4): what
/// the geometry identity folds. A change of one appearance field alone -- fill, stroke
/// colour with the stroke still drawn, blend, transparency, a ramp's colours -- builds
/// nothing (the record is refreshed); a change of one geometry predicate alone -- the
/// stroke's alpha leaving zero, one stop becoming two, `FillOnly` becoming
/// `FillAndStroke` on a `Line`, a `PolyLine` gaining colours -- builds one record.
#[test]
fn appearance_fields_refresh_and_geometry_predicates_build() {
    let lines = stats("canvas_geo_native_lanes", LANES);
    assert_eq!(lines.len(), 10, "one frame per present: {lines:?}");
    let expected = [
        (2, "fill", 0),
        (3, "stroke colour", 0),
        (4, "blend", 0),
        (5, "transparency", 0),
        (6, "a ramp's colours", 0),
        (7, "stroke alpha 0 -> 255", 1),
        (8, "one stop -> two", 1),
        (9, "FillOnly -> FillAndStroke", 1),
        (10, "no colours -> three", 1),
    ];
    for (frame, what, builds) in expected {
        let before = &lines[frame - 2];
        let after = &lines[frame - 1];
        assert_eq!(
            field(after, "generations") - field(before, "generations"),
            builds,
            "frame {frame} changed {what} alone and should build {builds} record(s).\n\
             before: {before}\nafter:  {after}",
        );
    }
    for line in &lines {
        assert_eq!(field(line, "geoVerifyMismatches"), 0, "{line}");
    }
}

/// Two items of one geometry and two paints in one frame share no record: the second
/// one this frame resolves is resolved under its damage identity. Swapping which comes
/// first each frame exercises a refresh of the record the other used last frame. Each
/// frame draws the later item's colour on top, and every record verifies.
#[test]
fn two_items_of_one_geometry_and_two_paints_draw_right_in_either_order() {
    let source = include_str!("scenes/canvas_geo_native_shared_record.mfb");
    for gpu in [false, true] {
        let (lines, frame) =
            stats_and_frame(&format!("canvas_geo_native_shared_{gpu}"), source, gpu);
        assert_eq!(lines.len(), 4, "one frame per present: {lines:?}");
        for line in &lines {
            assert_eq!(field(line, "geoVerifyMismatches"), 0, "{line}");
        }
        // The last frame is [blue, red]: red on top.
        let i = (120 * 900 + 120) * 4;
        assert_eq!(
            &frame[i..i + 4],
            &[255, 0, 0, 255],
            "gpu={gpu}: the last frame's top item is the red disc"
        );
    }
}
