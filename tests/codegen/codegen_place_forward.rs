//! plan-149: place copy forwarding is what puts `LET cur = <place>` on the
//! in-place path.
//!
//! `LET cur = rec.xs` followed by `rec = WITH rec { xs := OP(cur, …) }` copies the
//! field and then rebuilds the record, because plan-145's in-place field seam
//! only matches when the operand *is* the field expression. The Level-1 row
//! rewrites the NIR to exactly that direct form. A missed forward is silently
//! slow, never red, so only the emitted code shows it.
//!
//! Each case builds the same source at `-O1` and `-O0`. `-O1` must show the
//! in-place slot and no rebuild (`with_target`) or copy; `-O0` is the control —
//! it must still show the rebuild or the copy, which proves the row (and not
//! some other path) is the difference, so a green result cannot come from a row
//! that never fires.

#[path = "../common/mod.rs"]
mod common;

use serde_json::Value;

const TARGET: &str = "linux-x86_64";

fn slots_where(plan: &Value, symbol: &str, keep: impl Fn(&str) -> bool) -> usize {
    plan["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .find(|f| f["symbol"].as_str() == Some(symbol))
        .unwrap_or_else(|| panic!("code plan has no function '{symbol}'"))["stackSlots"]
        .as_array()
        .expect("stackSlots array")
        .iter()
        .filter(|slot| slot["type"].as_str().is_some_and(&keep))
        .count()
}

fn slot_count(plan: &Value, symbol: &str, slot_type: &str) -> usize {
    slots_where(plan, symbol, |t| t == slot_type)
}

/// Deep-copy slots: `copy_collection_tight`'s `tight_copy_source` and
/// `copy_flat_block`'s `flat_copy_source`.
fn copy_count(plan: &Value, symbol: &str) -> usize {
    slots_where(plan, symbol, |t| t.ends_with("copy_source"))
}

/// The `-O1` and `-O0` code plans for one source.
fn plans(name: &str, source: &str) -> (Value, Value) {
    let project = common::temp_project(name, source);
    let o1 = common::build_ncode_opt(&project, TARGET, name, "1");
    let o0 = common::build_ncode_opt(&project, TARGET, name, "0");
    let _ = std::fs::remove_dir_all(&project);
    (o1, o0)
}

const RECORD: &str = "IMPORT collections\n\
     TYPE R\n\
    \x20 before AS Integer\n\
    \x20 xs AS List OF Integer\n\
    \x20 after AS Integer\n\
     END TYPE\n";

/// The benchmark's `list (Record-Fixed) set` row: `get` through `cur`, then the
/// owner's own `WITH` writes back through `cur`.
#[test]
fn the_record_set_shape_goes_in_place() {
    let (o1, o0) = plans(
        "place_forward_record_set",
        &format!(
            "{RECORD}\
             FUNC work(n AS Integer) AS Integer\n\
            \x20 MUT rec AS R = R[1, [1, 2, 3], 0]\n\
            \x20 FOR j = 1 TO n\n\
            \x20   LET cur AS List OF Integer = rec.xs\n\
            \x20   LET v AS Integer = collections::get(cur, 0)\n\
            \x20   rec = WITH rec {{ xs := collections::set(cur, 0, v + 1) }}\n\
            \x20 NEXT\n\
            \x20 RETURN collections::get(rec.xs, 0)\n\
             END FUNC\n\
             FUNC main AS Integer\n\
            \x20 RETURN work(3)\n\
             END FUNC\n"
        ),
    );
    let f = "_mfb_fn_work";
    assert!(
        slot_count(&o1, f, "inplace_recfield_set_index") >= 1,
        "-O1: the forwarded `set` must reach plan-145's record-field arm"
    );
    assert_eq!(
        slot_count(&o1, f, "with_target"),
        0,
        "-O1: no record rebuild"
    );
    assert_eq!(copy_count(&o1, f), 0, "-O1: no copy of the field");
    assert!(
        slot_count(&o0, f, "with_target") >= 1,
        "control: at -O0 the copy stays and the record is rebuilt"
    );
    assert_eq!(slot_count(&o0, f, "inplace_recfield_set_index"), 0);
}

#[test]
fn the_record_remove_at_shape_goes_in_place() {
    let (o1, o0) = plans(
        "place_forward_record_remove_at",
        &format!(
            "{RECORD}\
             FUNC work(n AS Integer) AS Integer\n\
            \x20 MUT rec AS R = R[1, [1, 2, 3, 4, 5, 6], 0]\n\
            \x20 FOR j = 1 TO n\n\
            \x20   LET cur AS List OF Integer = rec.xs\n\
            \x20   rec = WITH rec {{ xs := collections::removeAt(cur, 0) }}\n\
            \x20 NEXT\n\
            \x20 RETURN len(rec.xs)\n\
             END FUNC\n\
             FUNC main AS Integer\n\
            \x20 RETURN work(3)\n\
             END FUNC\n"
        ),
    );
    let f = "_mfb_fn_work";
    assert!(
        slot_count(&o1, f, "inplace_recfield_remove_at_index") >= 1,
        "-O1: the forwarded `removeAt` must reach plan-145's record-field arm"
    );
    assert!(slot_count(&o1, f, "inplace_inlined_subblock") >= 1);
    assert_eq!(
        slot_count(&o1, f, "with_target"),
        0,
        "-O1: no record rebuild"
    );
    assert_eq!(copy_count(&o1, f), 0, "-O1: no copy of the field");
    assert!(
        slot_count(&o0, f, "with_target") >= 1,
        "control: at -O0 the copy stays and the record is rebuilt"
    );
}

/// A bare local: `LET snap = nums; nums = set(snap, …)`. There is no `WITH`, so
/// the control is the copy itself and the missing plain-local arm.
#[test]
fn the_bare_local_shape_goes_in_place() {
    let (o1, o0) = plans(
        "place_forward_bare_local",
        "IMPORT collections\n\
         FUNC work(n AS Integer) AS Integer\n\
        \x20 MUT nums AS List OF Integer = [1, 2, 3]\n\
        \x20 FOR j = 1 TO n\n\
        \x20   LET snap AS List OF Integer = nums\n\
        \x20   nums = collections::set(snap, 0, collections::get(snap, 0) + j)\n\
        \x20 NEXT\n\
        \x20 RETURN collections::get(nums, 0)\n\
         END FUNC\n\
         FUNC main AS Integer\n\
        \x20 RETURN work(3)\n\
         END FUNC\n",
    );
    let f = "_mfb_fn_work";
    assert!(
        slot_count(&o1, f, "inplace_set_index") >= 1,
        "-O1: the forwarded `set` must reach the plain-local in-place arm"
    );
    assert_eq!(copy_count(&o1, f), 0, "-O1: no copy of `nums`");
    assert!(
        copy_count(&o0, f) >= 1,
        "control: at -O0 `LET snap = nums` still copies"
    );
    assert_eq!(slot_count(&o0, f, "inplace_set_index"), 0);
}

// ---- plan-149-B3: global and `STATE` owners ----

/// The `-O1` / `-O0` pair for a forwarded write-back through `cur`: the named
/// in-place slot at `-O1`, with no rebuild and no copy; the rebuild at `-O0`.
fn assert_forward_goes_in_place(o1: &Value, o0: &Value, f: &str, inplace_slot: &str) {
    assert!(
        slot_count(o1, f, inplace_slot) >= 1,
        "-O1: the forwarded update must reach `{inplace_slot}`"
    );
    assert_eq!(slot_count(o1, f, "with_target"), 0, "-O1: no rebuild");
    assert_eq!(copy_count(o1, f), 0, "-O1: no copy of the field");
    assert!(
        slot_count(o0, f, "with_target") >= 1,
        "control: at -O0 the copy stays and the owner is rebuilt"
    );
    assert_eq!(slot_count(o0, f, inplace_slot), 0);
}

/// The global `set` shape. A global's NIR read carries an empty type, so this
/// also pins that the call gate (W4) takes the type from the declaration.
#[test]
fn the_global_set_shape_goes_in_place() {
    let (o1, o0) = plans(
        "place_forward_global_set",
        &format!(
            "{RECORD}\
             MUT g AS R = R[1, [1, 2, 3], 0]\n\
             FUNC work(n AS Integer) AS Integer\n\
            \x20 FOR j = 1 TO n\n\
            \x20   LET cur AS List OF Integer = g.xs\n\
            \x20   LET v AS Integer = collections::get(cur, 0)\n\
            \x20   g = WITH g {{ xs := collections::set(cur, 0, v + 1) }}\n\
            \x20 NEXT\n\
            \x20 RETURN collections::get(g.xs, 0)\n\
             END FUNC\n\
             FUNC main AS Integer\n\
            \x20 RETURN work(3)\n\
             END FUNC\n"
        ),
    );
    assert_forward_goes_in_place(&o1, &o0, "_mfb_fn_work", "inplace_recfield_set_index");
}

const STATE_MAIN: &str = "FUNC main AS Integer\n\
    \x20 RES f AS fs::File STATE R = fs::openFile(\"project.json\")\n\
    \x20 f.state = R[1, [1, 2, 3, 4, 5, 6], 0]\n\
    \x20 LET n AS Integer = work(f, 2)\n\
    \x20 fs::close(f)\n\
    \x20 RETURN n\n\
     END FUNC\n";

/// The benchmark's `list (State-Fixed) set` row.
#[test]
fn the_state_set_shape_goes_in_place() {
    let (o1, o0) = plans(
        "place_forward_state_set",
        &format!(
            "IMPORT fs\n{RECORD}\
             FUNC work(f AS fs::File STATE R, n AS Integer) AS Integer\n\
            \x20 FOR j = 1 TO n\n\
            \x20   LET cur AS List OF Integer = f.state.xs\n\
            \x20   LET v AS Integer = collections::get(cur, 0)\n\
            \x20   f.state.xs = collections::set(cur, 0, v + 1)\n\
            \x20 NEXT\n\
            \x20 RETURN collections::get(f.state.xs, 0)\n\
             END FUNC\n\
             {STATE_MAIN}"
        ),
    );
    assert_forward_goes_in_place(&o1, &o0, "_mfb_fn_work", "inplace_state_set_index");
}

#[test]
fn the_state_remove_at_shape_goes_in_place() {
    let (o1, o0) = plans(
        "place_forward_state_remove_at",
        &format!(
            "IMPORT fs\n{RECORD}\
             FUNC work(f AS fs::File STATE R, n AS Integer) AS Integer\n\
            \x20 FOR j = 1 TO n\n\
            \x20   LET cur AS List OF Integer = f.state.xs\n\
            \x20   f.state.xs = collections::removeAt(cur, 0)\n\
            \x20 NEXT\n\
            \x20 RETURN len(f.state.xs)\n\
             END FUNC\n\
             {STATE_MAIN}"
        ),
    );
    assert_forward_goes_in_place(&o1, &o0, "_mfb_fn_work", "inplace_state_remove_at_index");
}
