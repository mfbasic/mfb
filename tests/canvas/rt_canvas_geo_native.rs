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
        .env("MFB_MACAPP_HEADLESS", "1")
        .env("MFB_WINAPP_HEADLESS", "1")
        .env("MFB_GTKAPP_HEADLESS", "1")
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
/// negative-zero lines, and polygons of 0, 1 and 2 points, collinear and with a
/// duplicate vertex.
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
        "the matrix is ~6,000 items of the five native kinds (distinct ones: over a \
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
         the two must agree slot for slot (`func_geo_build.rs`).\n{matrix}"
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
/// here alternates two scenes of the same length, A and B, 60 times, a few milliseconds
/// apart and without `MFB_CANVAS_SYNC`, so the graphics thread renders while publishes
/// land. A and B put
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
