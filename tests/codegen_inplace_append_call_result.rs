//! Regression test: `list = collections::append(list, someFunc(x))` must take
//! the in-place grow path.
//!
//! The in-place gates (`try_inplace_append_assign` and its set/map/record-field
//! siblings) ask one question of the appended operand — "is its static type
//! exactly the collection's element type?" — to tell a single-element append
//! from a bulk `append(list, otherList)`. They asked it through
//! `static_type_name`, whose `NirValue::Call` arm is a hand-written table of a
//! few builtin names and answers `None` for **every user function**. So an
//! accumulate loop whose element came from a call fell off the fast path and
//! copied the whole buffer per element — O(n²).
//!
//! Measured before the fix, appending 50 000 elements to a `List OF Integer`:
//! 3 ms when the operand was a plain local, 60 243 ms when it was
//! `clamp(i MOD 1000)` — a 20 000× cliff on an ordinary idiom. It is what made
//! `audio::play`'s multi-track mixer (`collections::append(out,
//! __audio_clampS16(acc))`) take minutes to play eighteen seconds of audio.
//!
//! The fix reads the callee's declared `returns` when the table misses
//! (`static_item_type`). These are build-only `--ncode` checks cross-built for
//! `linux-x86_64` — the gate is shared, target-independent codegen — so there is
//! no wall-clock timing here: `lower_list_append_in_place` emits an
//! `append_inplace_realloc` label, and its presence *is* the fast path.

mod common;

use serde_json::Value;

const TARGET: &str = "linux-x86_64";

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

/// The baseline the fix has to match: appending a plain local has always taken
/// the in-place path. Pins the signal this file reads, so a failure of the test
/// below means "the call operand was refused", not "the label was renamed".
#[test]
fn append_of_a_plain_local_grows_in_place() {
    let plan = ncode(
        "inplace_local",
        "IMPORT collections\n\
         FUNC accum(n AS Integer) AS List OF Integer\n\
        \x20 MUT out AS List OF Integer = []\n\
        \x20 FOR i = 0 TO n - 1\n\
        \x20   LET v AS Integer = i\n\
        \x20   out = collections::append(out, v)\n\
        \x20 NEXT\n\
        \x20 RETURN out\n\
         END FUNC\n\
         FUNC main AS Integer\n\
        \x20 RETURN len(accum(4))\n\
         END FUNC\n",
    );
    assert!(
        label_count(&plan, "_mfb_fn_accum", "append_inplace_realloc") >= 1,
        "appending a plain local must take the in-place grow path"
    );
}

/// The regression: the appended element comes from a **user function** call.
/// `static_type_name` has no row for a user call, so the gate saw `None` and
/// fell through to the copying path, making the loop O(n²).
#[test]
fn append_of_a_user_call_result_grows_in_place() {
    let plan = ncode(
        "inplace_call",
        "IMPORT collections\n\
         FUNC clamp(v AS Integer) AS Integer\n\
        \x20 IF v > 100 THEN\n\
        \x20   RETURN 100\n\
        \x20 END IF\n\
        \x20 RETURN v\n\
         END FUNC\n\
         FUNC accum(n AS Integer) AS List OF Integer\n\
        \x20 MUT out AS List OF Integer = []\n\
        \x20 FOR i = 0 TO n - 1\n\
        \x20   out = collections::append(out, clamp(i))\n\
        \x20 NEXT\n\
        \x20 RETURN out\n\
         END FUNC\n\
         FUNC main AS Integer\n\
        \x20 RETURN len(accum(4))\n\
         END FUNC\n",
    );
    assert!(
        label_count(&plan, "_mfb_fn_accum", "append_inplace_realloc") >= 1,
        "appending a user function's result must take the in-place grow path: \
         the gate has to read the callee's declared return type, not give up \
         because `static_type_name` has no row for a user call. Falling through \
         copies the whole buffer per element (O(n^2))."
    );
}

/// NON-GOAL guard. The gate distinguishes a single element from a whole list by
/// the operand's static type; reading a callee's return type must not blur that.
/// A call returning `List OF Integer` is a bulk concatenation and must NOT be
/// lowered as a single-element in-place append — the element-count and payload
/// layout differ, so treating it as one element would corrupt the list.
#[test]
fn append_of_a_call_returning_a_list_is_not_a_single_element() {
    let plan = ncode(
        "inplace_bulk",
        "IMPORT collections\n\
         FUNC chunk(v AS Integer) AS List OF Integer\n\
        \x20 RETURN [v, v + 1]\n\
         END FUNC\n\
         FUNC accum(n AS Integer) AS List OF Integer\n\
        \x20 MUT out AS List OF Integer = []\n\
        \x20 FOR i = 0 TO n - 1\n\
        \x20   out = collections::append(out, chunk(i))\n\
        \x20 NEXT\n\
        \x20 RETURN out\n\
         END FUNC\n\
         FUNC main AS Integer\n\
        \x20 RETURN len(accum(4))\n\
         END FUNC\n",
    );
    assert_eq!(
        label_count(&plan, "_mfb_fn_accum", "append_inplace_realloc"),
        0,
        "a call returning `List OF Integer` is a bulk concatenation, not one \
         element — it must not take the single-element in-place append path."
    );
}
