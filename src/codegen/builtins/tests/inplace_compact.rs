//! plan-142-B: the in-place compaction primitive, `lower_list_compact_in_place`.
//!
//! One primitive serves every shrinking self-update, in two forms — a keep
//! *range* (`take`/`drop`/`mid`) and per-element keep *marks*
//! (`filter`/`distinct`) — over two list representations: a fixed-width list
//! (element `i` at `i * width`, no entry table) and an entry-table list, whose
//! payloads need not be in entry order. These tests pin which lowering each
//! shape gets, by the labels `main` emits; `tests/runtime/rt_inplace_self_update.rs`
//! and the differential cases in `rt_inplace_failure_atomic.rs` pin behaviour.
//!
//! The primitive has no entry point of its own — the arms are its only callers —
//! so each test drives it through the arm for its form.

use std::collections::BTreeSet;

use crate::codegen::engine::tests::test_support::Stream;
use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_src_cached, code_function, CodeTarget};

fn label_stems(source: &str) -> BTreeSet<String> {
    let plan = code_for_src_cached(source, CodeTarget::LinuxX86_64, Console);
    Stream::of(code_function(plan, "main"))
        .labels()
        .into_iter()
        .map(|(_, name)| {
            name.trim_end_matches(|c: char| c.is_ascii_digit())
                .trim_end_matches('_')
                .to_string()
        })
        .collect()
}

fn program(element: &str, init: &str, helper: &str, statement: &str) -> String {
    format!(
        "IMPORT collections\nIMPORT io\n\n{helper}\
         FUNC main() AS Integer\n  MUT x AS List OF {element} = {init}\n  \
         FOR i = 1 TO 3\n    {statement}\n  NEXT\n  \
         io::print(toString(len(x)))\n  RETURN 0\nEND FUNC\n"
    )
}

const IS_POSITIVE: &str = "FUNC isPositive(n AS Integer) AS Boolean\n  RETURN n > 0\nEND FUNC\n\n";
const LONGISH: &str = "FUNC longish(s AS String) AS Boolean\n  RETURN len(s) > 1\nEND FUNC\n\n";

/// Range form, fixed width: one block move of the kept run to the front — no
/// per-element loop, no probe.
#[test]
fn compact_in_place_range_form_on_a_fixed_width_list_is_one_block_move() {
    let stems = label_stems(&program(
        "Integer",
        "[1, 2, 3, 4]",
        "",
        "x = collections::drop(x, 1)",
    ));
    assert!(
        stems
            .iter()
            .any(|stem| stem.starts_with("compact_k2_range")),
        "`drop` on a List OF Integer must slide the kept run with one block copy: {stems:?}"
    );
    assert!(
        !stems.contains("compact_k2_loop") && !stems.contains("compact_probe_loop"),
        "a fixed-width range needs neither the per-element loop nor the order probe: {stems:?}"
    );
}

/// Marks form, fixed width: the per-element loop, no probe (a fixed-width list
/// has no entry table and is in order by construction).
#[test]
fn compact_in_place_marks_form_on_a_fixed_width_list_walks_the_elements() {
    let stems = label_stems(&program(
        "Integer",
        "[1, -2, 3]",
        IS_POSITIVE,
        "x = collections::filter(x, isPositive)",
    ));
    assert!(
        stems.contains("compact_k2_loop") && stems.contains("inplace_filter_loop"),
        "`filter` on a List OF Integer must mark in one pass and compact in another: {stems:?}"
    );
    assert!(
        !stems.contains("compact_probe_loop"),
        "a fixed-width list has no payload order to probe: {stems:?}"
    );
}

/// Both forms, entry table: probe the payload order, then take the in-order
/// slide or the out-of-order entry move (with its dead-byte repack).
#[test]
fn compact_in_place_on_an_entry_list_probes_the_payload_order() {
    for (form, statement) in [
        ("marks", "x = collections::filter(x, longish)"),
        ("range", "x = collections::take(x, 2)"),
    ] {
        let stems = label_stems(&program(
            "String",
            "[\"a\", \"bb\", \"ccc\"]",
            LONGISH,
            statement,
        ));
        for label in [
            "compact_probe_loop",
            "compact_ord_loop",
            "compact_dis_loop",
            "compact_dis_repack",
        ] {
            assert!(
                stems.contains(label),
                "{form} form on a List OF String must emit `{label}`: {stems:?}"
            );
        }
        assert!(
            !stems.iter().any(|stem| stem.starts_with("compact_k2")),
            "{form} form on an entry list must not take a fixed-width path: {stems:?}"
        );
    }
}

/// bug-652: the range form on an entry list first tries the cases that cost only
/// the removed entries — nothing removed, everything removed, a truncation whose
/// removed payloads are the data tail — before the probe; the marks form has no
/// such case and goes straight to the probe.
#[test]
fn compact_in_place_range_form_on_an_entry_list_tries_the_truncation_first() {
    let range = label_stems(&program(
        "String",
        "[\"a\", \"bb\", \"ccc\"]",
        "",
        "x = collections::take(x, 2)",
    ));
    for label in [
        "compact_range_retract",
        "compact_range_tidy",
        "compact_range_general",
    ] {
        assert!(
            range.contains(label),
            "`take` on a List OF String must try the truncation (`{label}`): {range:?}"
        );
    }
    let marks = label_stems(&program(
        "String",
        "[\"a\", \"bb\", \"ccc\"]",
        LONGISH,
        "x = collections::filter(x, longish)",
    ));
    assert!(
        !marks.iter().any(|stem| stem.starts_with("compact_range")),
        "`filter` must keep the full walk: {marks:?}"
    );
}

/// bug-652: a range's dropped-element pass jumps over the kept run instead of
/// testing every entry; the marks form tests every entry.
#[test]
fn compact_in_place_range_drop_pass_skips_the_kept_run() {
    const NODE: &str = "TYPE Node\n  kids AS List OF Node\n  tag AS Integer\nEND TYPE\n\n\
                        FUNC keepAll(n AS Node) AS Boolean\n  RETURN n.tag >= 0\nEND FUNC\n\n";
    let init = "[Node[kids := [], tag := 1], Node[kids := [], tag := 2]]";
    let range = label_stems(&program("Node", init, NODE, "x = collections::drop(x, 1)"));
    assert!(
        range.contains("compact_drop_loop") && range.contains("compact_drop_outside"),
        "`drop` on a graph-owning list must skip the kept run in its drop pass: {range:?}"
    );
    let marks = label_stems(&program(
        "Node",
        init,
        NODE,
        "x = collections::filter(x, keepAll)",
    ));
    assert!(
        marks.contains("compact_drop_loop") && !marks.contains("compact_drop_outside"),
        "`filter` must test every entry in its drop pass: {marks:?}"
    );
}

/// plan-142-C: `sort`/`sortBy` apply their index permutation in place — one
/// cycle-following pass over the fixed-width lanes or the entry table — instead of
/// gathering a new list.
#[test]
fn permute_in_place_reorders_lanes_and_entries_without_a_gather() {
    for (element, init) in [("Integer", "[3, 1, 2]"), ("String", "[\"b\", \"a\"]")] {
        let stems = label_stems(&program(element, init, "", "x = collections::sort(x)"));
        for label in [
            "permute_outer",
            "permute_cycle",
            "permute_close",
            "inplace_sort_merge",
        ] {
            assert!(
                stems.contains(label),
                "`sort` on a List OF {element} must permute in place (`{label}`): {stems:?}"
            );
        }
        assert!(
            !stems.iter().any(|stem| stem.starts_with("sort_gather")),
            "the in-place sort must not gather a fresh list: {stems:?}"
        );
    }
}

/// plan-142-D: `intersection`/`difference` write one membership mark per entry and
/// then compact the entry table once (`lower_map_compact_in_place`); `union` reserves
/// room for the whole batch once (`emit_map_reserve`) and then adds in place.
#[test]
fn map_compact_in_place_marks_then_compacts_and_union_reserves_once() {
    let set_program = |statement: &str| {
        format!(
            "IMPORT collections\nIMPORT io\n\nFUNC main() AS Integer\n  \
             MUT x AS Set OF Integer = Set OF Integer {{ 1, 2, 3 }}\n  \
             LET t AS Set OF Integer = Set OF Integer {{ 2, 3, 4 }}\n  \
             FOR i = 1 TO 3\n    {statement}\n  NEXT\n  \
             io::print(toString(len(x)))\n  RETURN 0\nEND FUNC\n"
        )
    };
    for op in ["intersection", "difference"] {
        let stems = label_stems(&set_program(&format!("x = collections::{op}(x, t)")));
        for label in ["mcompact_loop", &format!("inplace_{op}_mark_loop")] {
            assert!(
                stems.contains(label),
                "`{op}` must mark then compact in place (`{label}`): {stems:?}"
            );
        }
        assert!(
            !stems.iter().any(|stem| stem.starts_with("mreserve")),
            "`{op}` only removes entries and must never reserve: {stems:?}"
        );
    }
    let stems = label_stems(&set_program("x = collections::union(x, t)"));
    for label in ["mreserve_grow", "inplace_add_all_loop"] {
        assert!(
            stems.contains(label),
            "`union` must reserve once and add in place (`{label}`): {stems:?}"
        );
    }
    assert!(
        !stems.contains("mcompact_loop"),
        "`union` removes nothing: {stems:?}"
    );
}
