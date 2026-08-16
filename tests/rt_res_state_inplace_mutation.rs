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
#[ignore = "bug-430: out-of-line growable STATE collection fields — pending (split out of bug-424 Layer 2)"]
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
