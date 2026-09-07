//! bug-571: who owns the block a `FOR EACH` loop variable holds.
//!
//! `FOR EACH e IN xs` over a `List OF String` materialises a fresh arena block
//! per element per pass (`emit_load_payload_with_stride`'s `String` arm, via
//! `emit_materialize_string_from_bytes`), because a packed `String` has no
//! standalone header to point at. Nothing freed it: 25 MB at 50 000 passes over
//! an 8-element list, 50 MB at 100 000, with no callback anywhere in the program.
//!
//! The RSS cases in `rt_scope_drop_leaks` measure the leak. These count the
//! OWNERS in the instruction stream, because the two failure directions are
//! invisible to each other and one of them is invisible to any behavioural test:
//!
//! * **Too few owners** is the leak. Nothing goes red.
//! * **Too many owners** is a double free of a block the container or the caller
//!   still holds — free-list corruption that surfaces as "Allocation failed" at
//!   some later, unrelated allocation, or as a wrong value read out of reused
//!   memory. A behavioural probe sees it only if it happens to reuse the block
//!   before the read.
//!
//! Every assertion is COMPARATIVE — one loop against a sibling that differs in
//! exactly one way — so it pins the difference the fix makes rather than an
//! absolute count that drifts with unrelated codegen changes.
//!
//! Build-only `-ncode` cross-built for `linux-x86_64`, matching the sibling
//! codegen-inspection suites: ownership is target-independent codegen.

#[path = "../common/mod.rs"]
mod common;

use serde_json::Value;

const TARGET: &str = "linux-x86_64";

fn ncode(name: &str, source: &str) -> Value {
    let project = common::temp_project(name, source);
    let plan = common::build_ncode(&project, TARGET, name);
    let _ = std::fs::remove_dir_all(&project);
    plan
}

fn function<'a>(plan: &'a Value, symbol: &str) -> &'a Value {
    plan["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .find(|f| f["symbol"].as_str() == Some(symbol))
        .unwrap_or_else(|| panic!("code plan has no function '{symbol}'"))
}

/// How many owned-`String` drops `symbol` emits — every place that will free a
/// `String` block.
fn string_drops(plan: &Value, symbol: &str) -> usize {
    function(plan, symbol)["instructions"]
        .as_array()
        .expect("instructions array")
        .iter()
        .filter(|instr| instr["target"].as_str() == Some("_mfb_rt_drop_owned_string"))
        .count()
}

/// How many alias witnesses the loop spills — one per payload the loop
/// MATERIALISES, and the direct count of guarded loop-item drops.
/// `lower_for_each` names the slots `for_each_item_alias` /
/// `for_each_key_alias` / `for_each_value_alias`.
fn alias_witnesses(plan: &Value, symbol: &str) -> usize {
    function(plan, symbol)["stackSlots"]
        .as_array()
        .expect("stack slots array")
        .iter()
        .filter(|slot| {
            matches!(
                slot["type"].as_str(),
                Some("for_each_item_alias" | "for_each_key_alias" | "for_each_value_alias")
            )
        })
        .count()
}

/// The same loop body over a `List OF <ELEM>`, so the only difference between two
/// instantiations is the element type.
fn list_loop(elem: &str, literal: &str, body: &str) -> String {
    format!(
        "IMPORT io\n\
         SUB main()\n\
        \x20 LET xs AS List OF {elem} = {literal}\n\
        \x20 MUT acc AS Integer = 0\n\
        \x20 FOR EACH e IN xs\n\
        \x20   {body}\n\
        \x20 NEXT\n\
        \x20 io::print(toString(acc))\n\
         END SUB\n"
    )
}

/// A `List OF String` loop must gain exactly ONE `String` drop over the SAME loop
/// over a `List OF Integer` — the per-iteration item, and nothing else.
///
/// The `Integer` sibling is the bug report's own flat contrast, and it is the
/// control that makes the count attributable: it walks the same collection shape
/// with the same body arithmetic and materialises nothing.
#[test]
fn a_string_element_loop_gains_exactly_one_owner_over_an_integer_one() {
    let integers = ncode(
        "b571_list_int",
        &list_loop("Integer", "[0, 1, 2, 3]", "acc = acc + e"),
    );
    let strings = ncode(
        "b571_list_str",
        &list_loop(
            "String",
            "[\"a\", \"bb\", \"ccc\", \"d\"]",
            "acc = acc + len(e)",
        ),
    );
    let int_drops = string_drops(&integers, "_mfb_fn_main");
    let str_drops = string_drops(&strings, "_mfb_fn_main");
    assert_eq!(
        str_drops,
        int_drops + 1,
        "`FOR EACH e IN <List OF String>` must own its per-iteration block with \
         exactly ONE drop more than the identical `List OF Integer` loop \
         (Integer {int_drops}, String {str_drops}); fewer is bug-571's leak, more \
         is a double free"
    );
    assert_eq!(
        alias_witnesses(&integers, "_mfb_fn_main"),
        0,
        "a fixed-width element materialises nothing, so the loop must spill no \
         alias witness and emit no guard — the `Integer` loop's codegen is \
         untouched by this fix"
    );
    assert_eq!(
        alias_witnesses(&strings, "_mfb_fn_main"),
        1,
        "the one materialised payload must carry exactly one alias witness"
    );
}

/// A `Map OF String TO String` materialises TWO blocks per entry — the key and
/// the value are separate `emit_load_map_payload` calls — so it must own two.
/// The one-sided maps are the discriminating controls: each owns exactly one, and
/// only on the side that is a `String`.
#[test]
fn a_map_owns_one_block_per_string_side() {
    fn map_loop(key: &str, value: &str) -> String {
        format!(
            "IMPORT io\n\
             IMPORT collections\n\
             SUB main()\n\
            \x20 MUT m AS Map OF {key} TO {value} = Map OF {key} TO {value} {{}}\n\
            \x20 m = collections::set(m, {}, {})\n\
            \x20 MUT acc AS Integer = 0\n\
            \x20 FOR EACH e IN m\n\
            \x20   acc = acc + 1\n\
            \x20 NEXT\n\
            \x20 io::print(toString(acc))\n\
             END SUB\n",
            if key == "String" { "\"k\"" } else { "1" },
            if value == "String" { "\"v\"" } else { "2" },
        )
    }
    let int_int = ncode("b571_map_ii", &map_loop("Integer", "Integer"));
    let str_int = ncode("b571_map_si", &map_loop("String", "Integer"));
    let int_str = ncode("b571_map_is", &map_loop("Integer", "String"));
    let str_str = ncode("b571_map_ss", &map_loop("String", "String"));
    assert_eq!(
        alias_witnesses(&int_int, "_mfb_fn_main"),
        0,
        "a `Map OF Integer TO Integer` materialises neither side — the bug \
         report's flat contrast, and its codegen must not move"
    );
    assert_eq!(
        alias_witnesses(&str_int, "_mfb_fn_main"),
        1,
        "a `String` KEY materialises; an `Integer` value does not"
    );
    assert_eq!(
        alias_witnesses(&int_str, "_mfb_fn_main"),
        1,
        "a `String` VALUE materialises; an `Integer` key does not"
    );
    assert_eq!(
        alias_witnesses(&str_str, "_mfb_fn_main"),
        2,
        "both sides materialise, so both must be owned — a `Map OF String TO \
         String` leaked TWO blocks per entry per pass (50 MB at 50 000 passes, \
         99 MB at 100 000)"
    );
    let ii = string_drops(&int_int, "_mfb_fn_main");
    assert_eq!(
        string_drops(&str_str, "_mfb_fn_main"),
        ii + 2,
        "two materialised payloads, two owners"
    );
}

/// A `Set OF String` reads the entry's KEY payload through the same materialising
/// arm, on its own code path in `lower_for_each`. It is the arm a `List`/`Map`
/// enumeration silently omits, so it gets its own assertion.
#[test]
fn a_set_of_string_owns_its_element() {
    fn set_loop(elem: &str, value: &str) -> String {
        format!(
            "IMPORT io\n\
             IMPORT collections\n\
             SUB main()\n\
            \x20 MUT s AS Set OF {elem} = Set OF {elem} {{}}\n\
            \x20 s = collections::add(s, {value})\n\
            \x20 MUT acc AS Integer = 0\n\
            \x20 FOR EACH e IN s\n\
            \x20   acc = acc + 1\n\
            \x20 NEXT\n\
            \x20 io::print(toString(acc))\n\
             END SUB\n"
        )
    }
    let integers = ncode("b571_set_int", &set_loop("Integer", "7"));
    let strings = ncode("b571_set_str", &set_loop("String", "\"seven\""));
    assert_eq!(alias_witnesses(&integers, "_mfb_fn_main"), 0);
    assert_eq!(
        alias_witnesses(&strings, "_mfb_fn_main"),
        1,
        "`FOR EACH e IN <Set OF String>` materialises its element the same way a \
         `List OF String` does, and must own it the same way"
    );
    assert_eq!(
        string_drops(&strings, "_mfb_fn_main"),
        string_drops(&integers, "_mfb_fn_main") + 1
    );
}

/// The POSITIVE pin for the alias direction, and the reason the free is guarded:
/// a `List OF <record>` and a `List OF List OF …` element IS a pointer into the
/// container's own data region (`emit_load_payload_with_stride` returns `data`
/// itself for those arms). Freeing one corrupts the collection.
///
/// The check that the fix cannot: neither loop registers an owner or a witness,
/// so their codegen is byte-for-byte what it was.
#[test]
fn an_aliasing_element_is_never_given_an_owner() {
    let records = ncode(
        "b571_list_record",
        "IMPORT io\n\
         TYPE Row\n  name AS String\n  n AS Integer\nEND TYPE\n\
         SUB main()\n\
        \x20 LET xs AS List OF Row = [Row[\"a\", 1], Row[\"bb\", 2]]\n\
        \x20 MUT acc AS Integer = 0\n\
        \x20 FOR EACH e IN xs\n\
        \x20   acc = acc + e.n\n\
        \x20 NEXT\n\
        \x20 io::print(toString(acc))\n\
         END SUB\n",
    );
    let nested = ncode(
        "b571_list_nested",
        &list_loop("List OF Integer", "[[1, 2], [3]]", "acc = acc + len(e)"),
    );
    assert_eq!(
        alias_witnesses(&records, "_mfb_fn_main"),
        0,
        "a record element aliases the container's inlined slot — the loop must \
         register no owner for it"
    );
    assert_eq!(
        alias_witnesses(&nested, "_mfb_fn_main"),
        0,
        "a nested-collection element aliases the container's data region — the \
         loop must register no owner for it"
    );
}

/// `EXIT FOR` and `CONTINUE FOR` jump AROUND the fall-through path at the bottom
/// of the body — `emit_cleanup_branch_to_depth` is their only drop emitter — so
/// each needs its own drop or the loop leaks one block per taken edge.
///
/// This is what makes registering the item in the BODY's cleanup scope (rather
/// than emitting a free at the bottom of the loop) the fix: §14.7 names every one
/// of these edges, and the existing scope-drop machinery already serves them.
#[test]
fn an_early_exit_edge_drops_the_item_too() {
    let plain = ncode(
        "b571_plain",
        &list_loop("String", "[\"a\", \"bb\"]", "acc = acc + len(e)"),
    );
    let exiting = ncode(
        "b571_exit_for",
        &list_loop(
            "String",
            "[\"a\", \"bb\"]",
            "IF len(e) > 1 THEN\n      EXIT FOR\n    END IF\n    acc = acc + len(e)",
        ),
    );
    let continuing = ncode(
        "b571_continue_for",
        &list_loop(
            "String",
            "[\"a\", \"bb\"]",
            "IF len(e) > 1 THEN\n      CONTINUE FOR\n    END IF\n    acc = acc + len(e)",
        ),
    );
    let plain_drops = string_drops(&plain, "_mfb_fn_main");
    assert_eq!(
        string_drops(&exiting, "_mfb_fn_main"),
        plain_drops + 1,
        "`EXIT FOR` branches straight to the loop's end label, so it needs its own \
         item drop — without one the loop leaks a block per taken exit"
    );
    assert_eq!(
        string_drops(&continuing, "_mfb_fn_main"),
        plain_drops + 1,
        "`CONTINUE FOR` branches straight to the TOP of the loop, past the \
         fall-through drop, so it needs its own"
    );
}
