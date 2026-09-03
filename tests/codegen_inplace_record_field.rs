//! plan-121-C: a collection held in a **record field** reaches the same in-place
//! path a plain local does, via `rec = WITH rec { f := OP(rec.f, …) }`.
//!
//! Two things need a codegen-inspection test here, and neither is visible to a
//! behavioural fixture:
//!
//! * **Taken.** The container is the whole difference. `list (Record-Fixed) set`
//!   measured 1630× c -O0 while `list (Record-Dynamic) append` — same record, same
//!   field, same list — ran at 0.839×, because one had an arm and the other did
//!   not. A missed match is silently slow, never red.
//! * **Refused.** `G14` (`updates.len() == 1`) is load-bearing twice over here:
//!   returning `true` elides the whole-record rebuild, so a second updated field in
//!   the same `WITH` would have its new value **dropped**. That is a wrong answer,
//!   not a slow one, and only the emitted code shows which path was taken.

mod common;

use serde_json::Value;

const TARGET: &str = "linux-x86_64";

fn label_count(plan: &Value, symbol: &str, needle: &str) -> usize {
    function(plan, symbol)["instructions"]
        .as_array()
        .expect("instructions array")
        .iter()
        .filter(|instr| {
            instr["op"].as_str() == Some("label")
                && instr["name"]
                    .as_str()
                    .is_some_and(|name| name.contains(needle))
        })
        .count()
}

fn stack_slot_count(plan: &Value, symbol: &str, slot_type: &str) -> usize {
    function(plan, symbol)["stackSlots"]
        .as_array()
        .expect("stackSlots array")
        .iter()
        .filter(|slot| slot["type"].as_str() == Some(slot_type))
        .count()
}

fn function<'a>(plan: &'a Value, symbol: &str) -> &'a Value {
    plan["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .find(|f| f["symbol"].as_str() == Some(symbol))
        .unwrap_or_else(|| panic!("code plan has no function '{symbol}'"))
}

fn ncode(name: &str, source: &str) -> Value {
    let project = common::temp_project(name, source);
    let plan = common::build_ncode(&project, TARGET, name);
    let _ = std::fs::remove_dir_all(&project);
    plan
}

const BOX_TYPE: &str = "IMPORT collections\n\
     TYPE Box\n\
    \x20 before AS Integer\n\
    \x20 m AS Map OF Integer TO Integer\n\
    \x20 after AS Integer\n\
     END TYPE\n";

/// The whole point of plan-121-C: `removeKey` on a record-held map deletes inside
/// the record's own block instead of rebuilding the record *and* copying the map.
#[test]
fn remove_key_on_a_record_field_mutates_in_place() {
    let plan = ncode(
        "inplace_recfield_removekey",
        &format!(
            "{BOX_TYPE}\
             FUNC drain(n AS Integer) AS Integer\n\
            \x20 MUT rec AS Box = Box[1, Map OF Integer TO Integer {{ 1 := 10, 2 := 20 }}, 2]\n\
            \x20 FOR i = 1 TO n\n\
            \x20   rec = WITH rec {{ m := collections::removeKey(rec.m, i) }}\n\
            \x20 NEXT\n\
            \x20 RETURN len(rec.m)\n\
             END FUNC\n\
             FUNC main AS Integer\n\
            \x20 RETURN drain(2)\n\
             END FUNC\n"
        ),
    );
    assert!(
        label_count(&plan, "_mfb_fn_drain", "mrk_scan") >= 1,
        "removeKey on a record field must reach `lower_map_remove_key_in_place`, \
         not rebuild the record and copy the map"
    );
    assert!(
        stack_slot_count(&plan, "_mfb_fn_drain", "inplace_inlined_subblock") >= 1,
        "the in-place path reaches the map through the inlined sub-block address \
         (`open_inplace_inlined_subblock`), which is what lets the plain-local \
         lowering serve a record field unchanged"
    );
}

/// `G14` — a second updated field in the same `WITH` must take the rebuild path.
///
/// This is the decline that matters most in this container. The in-place arm
/// signals "handled" by returning `true`, which **elides the whole-record
/// rebuild**; if it matched a two-field update, `after`'s new value would never be
/// stored. A behavioural fixture catches that only if it happens to read the other
/// field afterwards — the emitted code shows it always.
#[test]
fn a_second_updated_field_declines_to_the_record_rebuild() {
    let plan = ncode(
        "inplace_recfield_removekey_multi",
        &format!(
            "{BOX_TYPE}\
             FUNC drain(n AS Integer) AS Integer\n\
            \x20 MUT rec AS Box = Box[1, Map OF Integer TO Integer {{ 1 := 10, 2 := 20 }}, 2]\n\
            \x20 FOR i = 1 TO n\n\
            \x20   rec = WITH rec {{ m := collections::removeKey(rec.m, i), after := i }}\n\
            \x20 NEXT\n\
            \x20 RETURN len(rec.m) + rec.after\n\
             END FUNC\n\
             FUNC main AS Integer\n\
            \x20 RETURN drain(2)\n\
             END FUNC\n"
        ),
    );
    assert_eq!(
        stack_slot_count(&plan, "_mfb_fn_drain", "inplace_inlined_subblock"),
        0,
        "a two-field `WITH` must NOT take the in-place path: the arm elides the \
         whole-record rebuild, so the sibling field's new value would be dropped"
    );
}

/// The container gate is not "is it a record field" but "is it the **last inlined**
/// field" — growing or compacting any earlier sub-block would shift the siblings
/// that follow it and the offsets stored for them.
///
/// Paired with the admit above so the arm cannot be widened into matching every
/// record shape: here `m` is followed by an inlined `String`, so it must decline.
#[test]
fn a_field_followed_by_an_inlined_sibling_declines() {
    let plan = ncode(
        "inplace_recfield_removekey_notlast",
        "IMPORT collections\n\
         TYPE Box\n\
        \x20 before AS Integer\n\
        \x20 m AS Map OF Integer TO Integer\n\
        \x20 tail AS String\n\
         END TYPE\n\
         FUNC drain(n AS Integer) AS Integer\n\
        \x20 MUT rec AS Box = Box[1, Map OF Integer TO Integer { 1 := 10, 2 := 20 }, \"t\"]\n\
        \x20 FOR i = 1 TO n\n\
        \x20   rec = WITH rec { m := collections::removeKey(rec.m, i) }\n\
        \x20 NEXT\n\
        \x20 RETURN len(rec.m) + len(rec.tail)\n\
         END FUNC\n\
         FUNC main AS Integer\n\
        \x20 RETURN drain(2)\n\
         END FUNC\n",
    );
    assert_eq!(
        stack_slot_count(&plan, "_mfb_fn_drain", "inplace_inlined_subblock"),
        0,
        "a collection field followed by another INLINED field must decline: \
         compacting its sub-block would shift the sibling that follows it"
    );
}


/// The first **growing** operation to reach a record field, and the one whose
/// failure mode is heap corruption rather than a wrong value.
///
/// A growing op reallocates, and an inlined collection has no allocation of its
/// own to replace — so `lower_map_set_in_place` is handed an `InlineGrow` and its
/// two grow sites instead allocate `fieldOffset + mapSize`, copy the record
/// prefix, publish the new **record** pointer, and free the old *record*. Handing
/// it the sub-block address instead would make `emit_free_pre_grow_buffer` call
/// `free()` on a pointer into the middle of a live allocation.
///
/// The behavioural fixture (`p121c-record-field-add-grow-rt`) forces hundreds of
/// geometric grows and re-probes every element; this pins that the grow path is
/// the one being emitted, which no output can show.
#[test]
fn add_on_a_record_field_grows_the_record_in_place() {
    let plan = ncode(
        "inplace_recfield_add",
        "IMPORT collections\n\
         TYPE SBox\n\
        \x20 before AS Integer\n\
        \x20 s AS Set OF Integer\n\
        \x20 after AS Integer\n\
         END TYPE\n\
         FUNC fill(n AS Integer) AS Integer\n\
        \x20 MUT rec AS SBox = SBox[1, Set OF Integer { }, 2]\n\
        \x20 FOR i = 0 TO n - 1\n\
        \x20   rec = WITH rec { s := collections::add(rec.s, i) }\n\
        \x20 NEXT\n\
        \x20 RETURN len(rec.s)\n\
         END FUNC\n\
         FUNC main AS Integer\n\
        \x20 RETURN fill(8)\n\
         END FUNC\n",
    );
    assert!(
        stack_slot_count(&plan, "_mfb_fn_fill", "inplace_inlined_field_off") >= 1,
        "a growing record-field op must hoist the field offset before the realloc: \
         the offset survives the grow (the prefix is copied verbatim) but the \
         sub-block ADDRESS does not"
    );
    assert!(
        label_count(&plan, "_mfb_fn_fill", "inline_grow_prefix") >= 1,
        "the grow must copy the record prefix into the newly allocated RECORD \
         block — without it the record's fixed slots are lost"
    );
}

/// The other half of the pair: a **plain local** Set `add` must NOT acquire the
/// record-grow machinery. `lower_map_set_in_place` is shared, so an `InlineGrow`
/// leaking into the plain-local path would grow a record that does not exist.
#[test]
fn add_on_a_plain_local_does_not_use_the_record_grow() {
    let plan = ncode(
        "inplace_local_add",
        "IMPORT collections\n\
         FUNC fill(n AS Integer) AS Integer\n\
        \x20 MUT s AS Set OF Integer = Set OF Integer { }\n\
        \x20 FOR i = 0 TO n - 1\n\
        \x20   s = collections::add(s, i)\n\
        \x20 NEXT\n\
        \x20 RETURN len(s)\n\
         END FUNC\n\
         FUNC main AS Integer\n\
        \x20 RETURN fill(8)\n\
         END FUNC\n",
    );
    assert_eq!(
        label_count(&plan, "_mfb_fn_fill", "inline_grow_prefix"),
        0,
        "a plain local owns its block outright: growing it must not copy a record \
         prefix or free a record that is not there"
    );
}


/// `set` on a record-held `List` of a **fixed-width** element takes the cheap
/// route: the replacement is always exactly the size of what it replaces, so
/// nothing grows and the plain-local lowering serves the inlined sub-block
/// directly.
///
/// This is `list (Record-Fixed) set`, the plan's headline record row at 1630x
/// c -O0 against `list (Record-Dynamic) append`'s 0.839x on the same record.
#[test]
fn set_on_a_fixed_width_record_list_writes_in_place() {
    let plan = ncode(
        "inplace_recfield_set_fixed",
        "IMPORT collections\n\
         TYPE LBox\n\
        \x20 before AS Integer\n\
        \x20 xs AS List OF Integer\n\
        \x20 after AS Integer\n\
         END TYPE\n\
         FUNC poke(n AS Integer) AS Integer\n\
        \x20 MUT rec AS LBox = LBox[1, [10, 20, 30], 2]\n\
        \x20 FOR i = 0 TO n - 1\n\
        \x20   rec = WITH rec { xs := collections::set(rec.xs, 1, i) }\n\
        \x20 NEXT\n\
        \x20 RETURN len(rec.xs)\n\
         END FUNC\n\
         FUNC main AS Integer\n\
        \x20 RETURN poke(4)\n\
         END FUNC\n",
    );
    assert!(
        stack_slot_count(&plan, "_mfb_fn_poke", "inplace_inlined_subblock") >= 1,
        "a same-size list `set` must write through the inlined sub-block"
    );
    // Nothing can grow, so the record-grow machinery must not appear.
    assert_eq!(
        label_count(&plan, "_mfb_fn_poke", "inline_grow_prefix"),
        0,
        "a fixed-width element is replaced by one of exactly its own size, so this \
         path must not carry the record-realloc machinery"
    );
}

/// The decline that keeps the fixed-width route sound: a **variable-width**
/// element can outgrow the slot it replaces, which makes
/// `lower_list_set_in_place`'s rebuild branch reachable — and that branch installs
/// a fresh block, which a sub-block address must never receive.
///
/// This is `list (Record-Dynamic) set`, which the plan assigns to plan-121-F.
/// Without this case the fixed-width test above would still pass if the arm were
/// widened to every list, and the result would be a freed record.
#[test]
fn set_on_a_variable_width_record_list_declines() {
    let plan = ncode(
        "inplace_recfield_set_var",
        "IMPORT collections\n\
         TYPE DBox\n\
        \x20 head AS Integer\n\
        \x20 ss AS List OF String\n\
         END TYPE\n\
         FUNC poke(n AS Integer) AS Integer\n\
        \x20 MUT rec AS DBox = DBox[1, [\"a\", \"bb\"]]\n\
        \x20 FOR i = 0 TO n - 1\n\
        \x20   rec = WITH rec { ss := collections::set(rec.ss, 1, \"wwwwww\") }\n\
        \x20 NEXT\n\
        \x20 RETURN len(rec.ss)\n\
         END FUNC\n\
         FUNC main AS Integer\n\
        \x20 RETURN poke(3)\n\
         END FUNC\n",
    );
    assert_eq!(
        stack_slot_count(&plan, "_mfb_fn_poke", "inplace_inlined_subblock"),
        0,
        "a variable-width element may not fit the slot it replaces, so the rebuild \
         branch is reachable and the sub-block route is unsound — decline"
    );
}

/// `set` on a record-held `Map` goes the third way: a new key grows the map and
/// therefore the record, so it takes the `InlineGrow` route `add` established.
#[test]
fn set_on_a_record_map_grows_the_record() {
    let plan = ncode(
        "inplace_recfield_set_map",
        "IMPORT collections\n\
         TYPE MBox\n\
        \x20 tag AS Integer\n\
        \x20 m AS Map OF Integer TO Integer\n\
        \x20 tail AS Integer\n\
         END TYPE\n\
         FUNC fill(n AS Integer) AS Integer\n\
        \x20 MUT rec AS MBox = MBox[1, Map OF Integer TO Integer { }, 2]\n\
        \x20 FOR i = 0 TO n - 1\n\
        \x20   rec = WITH rec { m := collections::set(rec.m, i, i) }\n\
        \x20 NEXT\n\
        \x20 RETURN len(rec.m)\n\
         END FUNC\n\
         FUNC main AS Integer\n\
        \x20 RETURN fill(8)\n\
         END FUNC\n",
    );
    assert!(
        label_count(&plan, "_mfb_fn_fill", "inline_grow_prefix") >= 1,
        "a map `set` can add a key, so it must grow the RECORD block and copy the \
         prefix — the same route `add` takes"
    );
}
