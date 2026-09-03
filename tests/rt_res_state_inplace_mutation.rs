//! Regression test for bug-424: mutating a resource `STATE` field must mutate
//! the existing STATE record in place, not rebuild the whole record.
//!
//! Every `s.state.field = value` desugars to a single-field `WITH` update over
//! `s.state` (`src/ast/stmt.rs`), which lowered to a whole-record rebuild in
//! `NirOp::StateAssign` codegen: it allocated a fresh `Accum` record, re-copied
//! every field — including any *inlined* `List OF Byte` payload — and stored the
//! new pointer into the resource's STATE slot. Accumulating N chunks into a
//! STATE buffer was therefore O(n²), and even a scalar bump on a STATE record
//! that also held a large buffer re-copied that buffer every iteration.
//!
//! The two fix layers each leave a deterministic, host-independent signature in
//! the emitted native-code plan, so these are build-only `--ncode` checks (no
//! execution, no wall-clock timing) cross-built for `linux-x86_64` — the fix is
//! in shared, target-independent codegen:
//!
//! * A whole-record STATE replace allocates a `state_assign_value` stack slot
//!   (the temp holding the freshly built record pointer). An in-place field
//!   mutation allocates none. So `state_assign_value == 0` proves the rebuild is
//!   gone; `== 1` proves it is still taken (the non-goal guards below rely on
//!   this staying `1`).
//! * An in-place list grow emits an `append_inplace_realloc` label
//!   (`lower_list_append_in_place`). Its presence proves the STATE collection
//!   field grew in place with capacity headroom instead of copying the whole
//!   accumulated buffer.
//!
//! Correctness of the aliasing/visibility contract (§15) is guarded separately
//! by the `tests/rt-behavior/resources/bug424-*` runtime fixtures.

mod common;

use serde_json::Value;

const TARGET: &str = "linux-x86_64";

/// Count `stackSlots` of a given `type` in the named function of an `--ncode`
/// dump.
fn stack_slot_count(plan: &Value, symbol: &str, slot_type: &str) -> usize {
    function(plan, symbol)["stackSlots"]
        .as_array()
        .expect("stackSlots array")
        .iter()
        .filter(|slot| slot["type"].as_str() == Some(slot_type))
        .count()
}

/// Count `label` instructions whose name contains `needle` in the named
/// function.
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

/// A scalar STATE field assignment (`f.state.n = f.state.n + 1`) mutates the
/// existing STATE record in place: no whole-record rebuild, hence no
/// `state_assign_value` temp. Before the fix this allocated one (the rebuilt
/// record pointer) and re-copied the inlined `raw` buffer every call.
#[test]
fn scalar_state_field_assign_stores_in_place() {
    let plan = ncode(
        "b424_scalar",
        "IMPORT fs\n\
         TYPE Accum\n\
        \x20 raw AS List OF Byte\n\
        \x20 n   AS Integer\n\
         END TYPE\n\
         FUNC bump(RES f AS fs::File STATE Accum) AS Nothing\n\
        \x20 f.state.n = f.state.n + 1\n\
         END FUNC\n\
         FUNC main AS Integer\n\
        \x20 RES f AS fs::File STATE Accum = fs::openFile(\"project.json\")\n\
        \x20 bump(f)\n\
        \x20 fs::close(f)\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert_eq!(
        stack_slot_count(&plan, "_mfb_fn_bump", "state_assign_value"),
        0,
        "bug-424: a scalar STATE field assign must store in place, not rebuild \
         the whole STATE record (which re-copies the inlined `raw` buffer). A \
         `state_assign_value` temp means the whole-record replace path was taken."
    );
}

/// A collection STATE field grown via `append` grows the field's buffer in
/// place (amortized O(1)), not by rebuilding the record and re-inlining the
/// whole accumulated buffer. After the fix the append emits an
/// `append_inplace_realloc` label and no `state_assign_value` rebuild.
#[test]
fn collection_state_field_grows_in_place() {
    let plan = ncode(
        "b424_coll",
        "IMPORT fs\n\
         IMPORT collections\n\
         TYPE Accum\n\
        \x20 raw AS List OF Byte\n\
        \x20 n   AS Integer\n\
         END TYPE\n\
         FUNC grow(RES f AS fs::File STATE Accum, chunk AS List OF Byte) AS Nothing\n\
        \x20 f.state.raw = collections::append(f.state.raw, chunk)\n\
         END FUNC\n\
         FUNC main AS Integer\n\
        \x20 RES f AS fs::File STATE Accum = fs::openFile(\"project.json\")\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert_eq!(
        stack_slot_count(&plan, "_mfb_fn_grow", "state_assign_value"),
        0,
        "bug-424: a collection STATE field append must grow in place, not \
         rebuild the whole STATE record (`state_assign_value` == whole-record \
         replace)."
    );
    assert!(
        label_count(&plan, "_mfb_fn_grow", "append_inplace_realloc") >= 1,
        "bug-424: a collection STATE field append must take the in-place grow \
         path (`lower_list_append_in_place`, which emits `append_inplace_realloc`), \
         so accumulation is amortized O(1) instead of copying the whole buffer."
    );
}

/// NON-GOAL guard. A whole-state replace (`f.state = <record value>`) is not a
/// single-field mutation of the current state — it installs a different record
/// and MUST keep the whole-record replace path (a `state_assign_value` temp).
/// Too-aggressive an in-place rule would wrongly optimize it.
#[test]
fn whole_state_replace_still_rebuilds() {
    let plan = ncode(
        "b424_whole",
        "IMPORT fs\n\
         TYPE Accum\n\
        \x20 raw AS List OF Byte\n\
        \x20 n   AS Integer\n\
         END TYPE\n\
         FUNC reset(RES f AS fs::File STATE Accum) AS Nothing\n\
        \x20 f.state = Accum[[], 0]\n\
         END FUNC\n\
         FUNC main AS Integer\n\
        \x20 RES f AS fs::File STATE Accum = fs::openFile(\"project.json\")\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert_eq!(
        stack_slot_count(&plan, "_mfb_fn_reset", "state_assign_value"),
        1,
        "bug-424 non-goal: a whole-state replace installs a distinct record and \
         must keep the whole-record STATE replace path."
    );
}

/// NON-GOAL guard. A `String` STATE field is *inlined* (variable-length, stored
/// as a block-relative offset in the record's trailing data region), so it
/// cannot be stored in place at a fixed slot — assigning it must keep the
/// whole-record rebuild (which re-lays-out the inlined data). Only fixed-width
/// scalar fields get the in-place store.
#[test]
fn string_state_field_still_rebuilds() {
    let plan = ncode(
        "b424_string",
        "IMPORT fs\n\
         TYPE Label\n\
        \x20 name AS String\n\
        \x20 pos  AS Integer\n\
         END TYPE\n\
         FUNC rename(RES f AS fs::File STATE Label) AS Nothing\n\
        \x20 f.state.name = \"hello\"\n\
         END FUNC\n\
         FUNC main AS Integer\n\
        \x20 RES f AS fs::File STATE Label = fs::openFile(\"project.json\")\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert_eq!(
        stack_slot_count(&plan, "_mfb_fn_rename", "state_assign_value"),
        1,
        "bug-424 non-goal: an inlined `String` STATE field cannot be stored in \
         place at a fixed slot; it must keep the whole-record rebuild."
    );
}

/// bug-430 case B: the idiomatic MUT-record update
/// `rec = WITH rec { coll := collections::append(rec.coll, x) }`, when `coll` is
/// the last inlined field, grows the field's buffer in place (amortized O(1))
/// instead of rebuilding the whole record every append — the MUT-local analogue
/// of the STATE grow. It emits an `append_inplace_realloc` label. This is a
/// value-preserving `WITH` reassignment of a uniquely-owned mutable binding; the
/// language grows no `a.field = v` statement (records update only via `WITH`).
#[test]
fn record_field_append_grows_in_place() {
    let plan = ncode(
        "b430_recfield",
        "IMPORT collections\n\
         TYPE Doc\n\
        \x20 body AS List OF String\n\
        \x20 n    AS Integer\n\
         END TYPE\n\
         FUNC add(a AS Doc, line AS String) AS Doc\n\
        \x20 MUT b AS Doc = a\n\
        \x20 b = WITH b { body := collections::append(b.body, line) }\n\
        \x20 RETURN b\n\
         END FUNC\n\
         FUNC main AS Integer\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert!(
        label_count(&plan, "_mfb_fn_add", "append_inplace_realloc") >= 1,
        "bug-430: a MUT-record `WITH`-append of a last-inlined collection field \
         must grow the field in place (`append_inplace_realloc`), not rebuild the \
         whole record on every append."
    );
}

/// NON-GOAL guard for bug-430 case B. When the appended collection is NOT the
/// last inlined field (an inlined `String` field follows it), growing its
/// sub-block would shift the later sub-blocks, so the update must keep the
/// whole-record rebuild — no in-place grow label.
#[test]
fn record_field_append_not_last_inlined_rebuilds() {
    let plan = ncode(
        "b430_recfield_mid",
        "IMPORT collections\n\
         TYPE Doc\n\
        \x20 body  AS List OF String\n\
        \x20 title AS String\n\
         END TYPE\n\
         FUNC add(a AS Doc, line AS String) AS Doc\n\
        \x20 MUT b AS Doc = a\n\
        \x20 b = WITH b { body := collections::append(b.body, line) }\n\
        \x20 RETURN b\n\
         END FUNC\n\
         FUNC main AS Integer\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert_eq!(
        label_count(&plan, "_mfb_fn_add", "append_inplace_realloc"),
        0,
        "bug-430 non-goal: a collection that is not the last inlined field cannot \
         grow in place (a following inlined `String` field would shift); it must \
         keep the whole-record rebuild."
    );
}

// ---------------------------------------------------------------------------
// plan-121-D Phase 2 — `removeKey`, `add` and `set` on a STATE-held collection.
//
// These tests carry more weight than their record-field counterparts, and the
// reason is measured rather than stylistic: Correction D1 established that NO
// `.ncodesum` fixture in the tree contains a STATE collection update, so the
// artifact gate reports 0 diffs whether these arms fire or not. Codegen
// inspection is the only instrument that can see the path being taken at all.
//
// Each arm gets a pair, per the rule that a fast-path test alone is half a test:
//   * a POSITIVE — the arm fires (no `state_assign_value`, i.e. the whole-record
//     rebuild was elided, plus the slot the arm itself allocates); and
//   * a DECLINE — a neighbouring shape that must NOT take it, asserted by the
//     rebuild temp still being there. A missed decline miscompiles; only the
//     negative sees it.
// ---------------------------------------------------------------------------

/// Preamble shared by the STATE Phase 2 sources: a stateful `fs::File` whose
/// STATE holds one collection field and one sibling scalar.
fn state_src(state_ty: &str, field: &str, body: &str) -> String {
    format!(
        "IMPORT fs\n\
         IMPORT collections\n\
         TYPE St\n\
        \x20 {field} AS {state_ty}\n\
        \x20 n AS Integer\n\
         END TYPE\n\
         FUNC mutate(RES f AS fs::File STATE St, k AS String, v AS Integer) AS Nothing\n\
        \x20 {body}\n\
         END FUNC\n\
         FUNC main AS Integer\n\
        \x20 RES f AS fs::File STATE St = fs::openFile(\"project.json\")\n\
        \x20 RETURN 0\n\
         END FUNC\n"
    )
}

#[test]
fn remove_key_on_a_state_field_mutates_in_place() {
    let plan = ncode(
        "p121d_state_removekey",
        &state_src(
            "Map OF String TO Integer",
            "m",
            "f.state.m = collections::removeKey(f.state.m, k)",
        ),
    );
    assert_eq!(
        stack_slot_count(&plan, "_mfb_fn_mutate", "state_assign_value"),
        0,
        "plan-121-D: `removeKey` on a STATE-held Map must delete the entry inside \
         the existing STATE block, not rebuild the whole STATE record \
         (`state_assign_value` is the whole-record replace temp)."
    );
    assert_eq!(
        stack_slot_count(&plan, "_mfb_fn_mutate", "inplace_state_remove_key"),
        1,
        "plan-121-D: the STATE `removeKey` arm must be the one that fired -- this \
         slot is allocated by no other path, so it distinguishes `removeKey` \
         reaching the STATE container from some neighbouring arm eliding the \
         rebuild for a different reason."
    );
}

#[test]
fn add_on_a_state_field_grows_the_state_block() {
    let plan = ncode(
        "p121d_state_add",
        &state_src(
            "Set OF Integer",
            "s",
            "f.state.s = collections::add(f.state.s, v)",
        ),
    );
    assert_eq!(
        stack_slot_count(&plan, "_mfb_fn_mutate", "state_assign_value"),
        0,
        "plan-121-D: `add` on a STATE-held Set must grow the STATE block in place \
         rather than rebuilding the STATE record around a fresh copy of the set. \
         `set (State-Dynamic) add` is the worst element-type overhead row in the \
         suite at 701.6x, and that is exactly these two copies per call."
    );
    assert_eq!(
        stack_slot_count(&plan, "_mfb_fn_mutate", "inplace_state_add_item"),
        1,
        "plan-121-D: the STATE `add` arm must be the one that fired."
    );
}

/// The growing arms must take the `InlineGrow` route, NOT the sub-block one.
/// This is the distinction Correction C2 exists for, and getting it wrong is
/// heap corruption rather than a slow path: `lower_map_set_in_place` calls
/// `emit_free_pre_grow_buffer` on the slot it is given, so a sub-block address
/// would `free()` a pointer into the middle of the live STATE block.
#[test]
fn a_growing_state_arm_reads_the_field_offset_before_the_grow() {
    let plan = ncode(
        "p121d_state_add_off",
        &state_src(
            "Set OF Integer",
            "s",
            "f.state.s = collections::add(f.state.s, v)",
        ),
    );
    assert_eq!(
        stack_slot_count(&plan, "_mfb_fn_mutate", "inplace_inlined_field_off"),
        1,
        "plan-121-D: a growing STATE arm must hoist the field's block-relative \
         offset BEFORE the grow. The realloc copies the record prefix verbatim so \
         the offset survives it, while the sub-block ADDRESS does not -- reading \
         the address across the grow is the use-after-free this hoist prevents."
    );
}

#[test]
fn map_set_on_a_state_field_grows_the_state_block() {
    let plan = ncode(
        "p121d_state_mapset",
        &state_src(
            "Map OF String TO Integer",
            "m",
            "f.state.m = collections::set(f.state.m, k, v)",
        ),
    );
    assert_eq!(
        stack_slot_count(&plan, "_mfb_fn_mutate", "state_assign_value"),
        0,
        "plan-121-D: `set` on a STATE-held Map must assign the key inside the \
         existing STATE block (`map (State-Dynamic) set` is 370.1x)."
    );
    assert_eq!(
        stack_slot_count(&plan, "_mfb_fn_mutate", "inplace_state_set_key"),
        1,
        "plan-121-D: the STATE map-`set` arm must be the one that fired."
    );
}

#[test]
fn fixed_width_list_set_on_a_state_field_writes_in_place() {
    let plan = ncode(
        "p121d_state_listset_fixed",
        &state_src(
            "List OF Integer",
            "xs",
            "f.state.xs = collections::set(f.state.xs, v, v)",
        ),
    );
    assert_eq!(
        stack_slot_count(&plan, "_mfb_fn_mutate", "state_assign_value"),
        0,
        "plan-121-D: `set` of a FIXED-width element into a STATE-held List needs \
         no grow at all -- the payload is replaced by one of exactly its own size, \
         which makes `lower_list_set_in_place`'s rebuild branch unreachable -- so \
         it takes the cheaper sub-block route. `list (State-Fixed) set` is 284.9x."
    );
    assert_eq!(
        stack_slot_count(&plan, "_mfb_fn_mutate", "inplace_state_set_index"),
        1,
        "plan-121-D: the STATE list-`set` arm must be the one that fired."
    );
}

/// DECLINE, and the most important test in this file. A variable-width element
/// can be replaced by a LONGER one, which makes `lower_list_set_in_place`'s
/// rebuild branch reachable -- and that branch installs a fresh block, which an
/// inlined sub-block address must never receive. `list (State-Dynamic) set` is
/// the worst row in the whole suite at 17742x, so the temptation to take it here
/// is exactly proportional to how wrong it would be. plan-121-F owns that row.
#[test]
fn variable_width_list_set_on_a_state_field_declines() {
    let plan = ncode(
        "p121d_state_listset_var",
        &state_src(
            "List OF String",
            "ss",
            "f.state.ss = collections::set(f.state.ss, v, k)",
        ),
    );
    assert_eq!(
        stack_slot_count(&plan, "_mfb_fn_mutate", "state_assign_value"),
        1,
        "plan-121-D: `set` of a VARIABLE-width element into a STATE-held List must \
         DECLINE to the copying rebuild. The replacement can outgrow what it \
         replaces, making the rebuild branch reachable; taking the sub-block route \
         would hand that branch an address inside the live STATE block. Declining \
         is slow, never wrong -- and only this negative test can see it, because \
         no `.ncode` golden covers a STATE collection update at all (Correction D1)."
    );
    assert_eq!(
        stack_slot_count(&plan, "_mfb_fn_mutate", "inplace_state_set_index"),
        0,
        "plan-121-D: the declining shape must not allocate the in-place arm's slot."
    );
}

/// DECLINE. `G14` -- a `WITH` that updates a SECOND field cannot elide the
/// record rebuild, because the arm returning `true` is what drops the rebuild,
/// and the sibling's new value would go with it. A wrong answer, not a slow one.
#[test]
fn a_second_updated_state_field_declines_to_the_rebuild() {
    let plan = ncode(
        "p121d_state_two_fields",
        "IMPORT fs\n\
         IMPORT collections\n\
         TYPE St\n\
        \x20 m AS Map OF String TO Integer\n\
        \x20 n AS Integer\n\
         END TYPE\n\
         FUNC mutate(RES f AS fs::File STATE St, k AS String, v AS Integer) AS Nothing\n\
        \x20 f.state = WITH f.state { m := collections::removeKey(f.state.m, k), n := v }\n\
         END FUNC\n\
         FUNC main AS Integer\n\
        \x20 RES f AS fs::File STATE St = fs::openFile(\"project.json\")\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert_eq!(
        stack_slot_count(&plan, "_mfb_fn_mutate", "inplace_state_remove_key"),
        0,
        "plan-121-D: a two-field `WITH` over `.state` must NOT take any in-place \
         arm. `G14` (`updates.len() == 1`) is what makes eliding the rebuild sound; \
         match a second updated field and that field's new value is silently \
         dropped."
    );
}

// ---------------------------------------------------------------------------
// plan-121-D Phase 3 — `removeAt`, Set `remove`, `insert` and `prepend`.
//
// Same pairing rule as Phase 2, plus one inheritance worth pinning explicitly:
// `G24`. `removeAt` is the only operation in the family that RELOCATES existing
// payloads, so a recursive element type — whose `get` is not an independent deep
// copy — must decline. That is a property of the element type, not the
// container, which is exactly why the STATE container has to be shown obeying it
// too rather than assumed to.
// ---------------------------------------------------------------------------

#[test]
fn remove_at_on_a_state_field_mutates_in_place() {
    let plan = ncode(
        "p121d_state_removeat",
        &state_src(
            "List OF Integer",
            "xs",
            "f.state.xs = collections::removeAt(f.state.xs, v)",
        ),
    );
    assert_eq!(
        stack_slot_count(&plan, "_mfb_fn_mutate", "state_assign_value"),
        0,
        "plan-121-D: `removeAt` on a STATE-held List must shift down inside the \
         existing STATE block, not rebuild the whole STATE record."
    );
    assert_eq!(
        stack_slot_count(&plan, "_mfb_fn_mutate", "inplace_state_remove_at_index"),
        1,
        "plan-121-D: the STATE `removeAt` arm must be the one that fired."
    );
}

/// DECLINE — `G24`, inherited from plan-121-B B7 through plan-121-C. A recursive
/// element type is a pointer-linked graph, and `removeAt` compacts the data
/// region underneath it. Getting this wrong is a use-after-free, not a slow path,
/// and no black-box fixture can see the refusal.
#[test]
fn remove_at_on_a_recursive_element_declines_in_the_state_container() {
    let plan = ncode(
        "p121d_state_removeat_rec",
        "IMPORT fs\n\
         IMPORT collections\n\
         TYPE Node\n\
        \x20 kids AS List OF Node\n\
        \x20 tag AS Integer\n\
         END TYPE\n\
         TYPE St\n\
        \x20 xs AS List OF Node\n\
        \x20 n AS Integer\n\
         END TYPE\n\
         FUNC mutate(RES f AS fs::File STATE St, k AS String, v AS Integer) AS Nothing\n\
        \x20 f.state.xs = collections::removeAt(f.state.xs, v)\n\
         END FUNC\n\
         FUNC main AS Integer\n\
        \x20 RES f AS fs::File STATE St = fs::openFile(\"project.json\")\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert_eq!(
        stack_slot_count(&plan, "_mfb_fn_mutate", "inplace_state_remove_at_index"),
        0,
        "plan-121-D `G24`: `removeAt` of a RECURSIVE element type must decline in \
         the STATE container exactly as it does for a plain local and a record \
         field. The rule is a property of the ELEMENT TYPE -- `get` on a \
         pointer-linked element is not an independent deep copy -- so swapping \
         the container cannot make it safe."
    );
}

#[test]
fn set_remove_on_a_state_field_mutates_in_place() {
    let plan = ncode(
        "p121d_state_setremove",
        &state_src(
            "Set OF Integer",
            "s",
            "f.state.s = collections::remove(f.state.s, v)",
        ),
    );
    assert_eq!(
        stack_slot_count(&plan, "_mfb_fn_mutate", "state_assign_value"),
        0,
        "plan-121-D: Set `remove` on a STATE field must delete inside the existing \
         STATE block."
    );
    assert_eq!(
        stack_slot_count(&plan, "_mfb_fn_mutate", "inplace_state_set_remove"),
        1,
        "plan-121-D: the STATE Set-`remove` arm must be the one that fired."
    );
}

/// `insert` and `prepend` share ONE arm (a prepend is `SpliceAt::Front`), so they
/// are asserted independently — a regression confined to the `prepend` wrapper
/// would otherwise hide behind `insert`.
#[test]
fn insert_on_a_state_field_grows_the_state_block() {
    let plan = ncode(
        "p121d_state_insert",
        &state_src(
            "List OF Integer",
            "xs",
            "f.state.xs = collections::insert(f.state.xs, v, v)",
        ),
    );
    assert_eq!(
        stack_slot_count(&plan, "_mfb_fn_mutate", "state_assign_value"),
        0,
        "plan-121-D: `insert` on a STATE-held List must splice inside the STATE \
         block and grow it via InlineGrow."
    );
    assert_eq!(
        stack_slot_count(&plan, "_mfb_fn_mutate", "inplace_state_splice_index"),
        1,
        "plan-121-D: `insert` carries an index, so the splice arm must allocate \
         the index slot -- this is what distinguishes it from `prepend`."
    );
}

#[test]
fn prepend_on_a_state_field_grows_the_state_block() {
    let plan = ncode(
        "p121d_state_prepend",
        &state_src(
            "List OF Integer",
            "xs",
            "f.state.xs = collections::prepend(f.state.xs, v)",
        ),
    );
    assert_eq!(
        stack_slot_count(&plan, "_mfb_fn_mutate", "state_assign_value"),
        0,
        "plan-121-D: `prepend` on a STATE-held List must splice at the front \
         inside the STATE block."
    );
    assert_eq!(
        stack_slot_count(&plan, "_mfb_fn_mutate", "inplace_state_splice_item"),
        1,
        "plan-121-D: the STATE splice arm must have fired for `prepend` too."
    );
    assert_eq!(
        stack_slot_count(&plan, "_mfb_fn_mutate", "inplace_state_splice_index"),
        0,
        "plan-121-D: a `prepend` is `SpliceAt::Front` and carries NO index, so it \
         must not allocate the index slot. Asserting this separates the two \
         spellings that share one arm."
    );
}

/// Paired must-not-change. The splice and grow lowerings are shared with the
/// plain-local and record-field containers, so an `InlineGrow` leaking into the
/// plain-local path would free a record that is not there.
#[test]
fn a_plain_local_splice_does_not_use_the_state_grow() {
    let plan = ncode(
        "p121d_plain_prepend",
        "IMPORT collections\n\
         FUNC mutate(v AS Integer) AS Integer\n\
        \x20 MUT xs AS List OF Integer = []\n\
        \x20 xs = collections::prepend(xs, v)\n\
        \x20 RETURN len(xs)\n\
         END FUNC\n\
         FUNC main AS Integer\n\
        \x20 RETURN mutate(1)\n\
         END FUNC\n",
    );
    assert_eq!(
        stack_slot_count(&plan, "_mfb_fn_mutate", "inplace_state_splice_item"),
        0,
        "plan-121-D: a plain-local `prepend` must not take the STATE arm. The \
         splice lowering is shared, so an InlineGrow or a STATE write-back \
         leaking into this path would repoint and free a block that does not \
         exist here."
    );
    assert_eq!(
        stack_slot_count(&plan, "_mfb_fn_mutate", "inline_state_ptr"),
        0,
        "plan-121-D: a plain local has no STATE pointer to open."
    );
}
