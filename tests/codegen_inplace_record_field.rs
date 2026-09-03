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
