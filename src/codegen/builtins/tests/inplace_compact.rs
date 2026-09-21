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
