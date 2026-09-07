//! bug-567: who frees the INTERIOR temporaries of a `RETURN`'s own expression.
//!
//! `clear_pending_temps_to` truncated **every** pending temp a control-transfer
//! statement registered, on two justifications. The first is true — the returned
//! temp is moved to the caller — but it is true only of the ONE temp
//! `claim_pending_temp` has already popped. The second, "an interior free would be
//! unreachable", is false: the free is unreachable only because it would be
//! emitted *after* the branch, which is a property of placement, not of the
//! program. So `RETURN "<" & s & ">"` allocated two blocks, handed one out, and
//! abandoned the other.
//!
//! `rt_scope_drop_leaks` measures the megabytes. This counts the OWNERS per
//! function, because the two failure directions are invisible to each other: too
//! few is the leak, too many is a use-after-free on the block the CALLER now owns,
//! and that surfaces later as a wrong string rather than as a failing free.
//!
//! Every assertion is COMPARATIVE against a sibling that differs in one way —
//! usually one `&` operator.
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

fn slots(plan: &Value, symbol: &str, kind: &str) -> usize {
    function(plan, symbol)["stackSlots"]
        .as_array()
        .expect("stack slots array")
        .iter()
        .filter(|slot| slot["type"].as_str() == Some(kind))
        .count()
}

/// How many blocks `symbol` registered as statement-scope temporaries — the
/// DENOMINATOR every owner count below is read against.
fn pending_temps(plan: &Value, symbol: &str) -> usize {
    slots(plan, symbol, "pending_temp")
}

/// How many times `symbol` parks an escaping return value across an interior
/// free. One per `RETURN` that actually had an interior temp to free, and zero
/// everywhere else — which is what keeps this fix off everyone else's codegen.
fn escaping_parks(plan: &Value, symbol: &str) -> usize {
    slots(plan, symbol, "return_escaping_value")
}

/// How many owned-value frees `symbol` emits. `_mfb_rt_drop_owned_string` is the
/// out-of-lined `String` drop (plan-118-E); the collection drop is its sibling.
fn frees(plan: &Value, symbol: &str, symbol_name: &str) -> usize {
    function(plan, symbol)["relocations"]
        .as_array()
        .expect("relocations array")
        .iter()
        .filter(|r| r["to"].as_str() == Some(symbol_name))
        .count()
}

fn string_frees(plan: &Value, symbol: &str) -> usize {
    frees(plan, symbol, "_mfb_rt_drop_owned_string")
}

/// How many arena blocks the concatenations in `symbol` allocate.
fn concats(plan: &Value, symbol: &str) -> usize {
    frees(plan, symbol, "_mfb_rt_string_concat")
}

/// How many interior frees are guarded by the runtime pointer-identity compare
/// against the escaping block. Soundness here is local, not a whole-program
/// proof: even a lowering that handed the return the same pointer as an interior
/// temp cannot have it freed underneath the caller.
fn escape_guards(plan: &Value, symbol: &str) -> usize {
    function(plan, symbol)["instructions"]
        .as_array()
        .expect("instructions array")
        .iter()
        .filter(|i| {
            i["op"].as_str() == Some("label")
                && i["name"]
                    .as_str()
                    .is_some_and(|n| n.starts_with("return_temp_escaped"))
        })
        .count()
}

fn callee(name: &str, body: &str) -> Value {
    ncode(
        name,
        &format!(
            "IMPORT io\n\
             IMPORT strings\n\
             {body}\n\
             SUB main()\n\
            \x20 io::print(probe(\"ab\"))\n\
             END SUB\n"
        ),
    )
}

/// The bug and its control, one `&` operator apart.
///
/// `"<" & s & ">"` is `Binary{ Binary{ "<", s }, ">" }`: two allocations, one of
/// which is INTERIOR. `s & ">"` is one allocation, and it is the one that leaves.
/// That single token is the whole difference between 13.3 -> 25.6 MB and flat
/// 1.0 MB, so it is the difference the owner count has to show.
#[test]
fn a_nested_concat_return_frees_its_interior_block_and_a_flat_one_frees_nothing() {
    let nested = callee(
        "b567_nested",
        "FUNC probe(s AS String) AS String\n  RETURN \"<\" & s & \">\"\nEND FUNC",
    );
    let flat = callee(
        "b567_flat",
        "FUNC probe(s AS String) AS String\n  RETURN s & \">\"\nEND FUNC",
    );

    assert_eq!(
        concats(&flat, "_mfb_fn_probe"),
        1,
        "one operator, one allocation"
    );
    assert_eq!(
        pending_temps(&flat, "_mfb_fn_probe"),
        1,
        "and that one allocation is the value that LEAVES, so it is the temp the \
         return claims"
    );
    assert_eq!(
        escaping_parks(&flat, "_mfb_fn_probe"),
        0,
        "nothing interior to free, so nothing to park across a free: this \
         function's codegen is byte-identical to the pre-fix compiler's"
    );
    assert_eq!(
        string_frees(&flat, "_mfb_fn_probe"),
        0,
        "the returned block belongs to the caller — freeing it here is the \
         use-after-free direction, not the leak direction"
    );

    assert_eq!(
        concats(&nested, "_mfb_fn_probe"),
        2,
        "two operators, two allocations"
    );
    assert_eq!(
        pending_temps(&nested, "_mfb_fn_probe"),
        2,
        "both are registered; exactly one of them is claimed by the RETURN"
    );
    assert_eq!(
        escaping_parks(&nested, "_mfb_fn_probe"),
        1,
        "the escaping block is parked once, across the interior free — \
         `arena_free` clobbers every caller-saved register"
    );
    assert_eq!(
        string_frees(&nested, "_mfb_fn_probe"),
        1,
        "exactly ONE free: zero is bug-567's leak, two frees the block the \
         caller now owns"
    );
    assert_eq!(
        escape_guards(&nested, "_mfb_fn_probe"),
        1,
        "and that free is guarded by a pointer-identity compare against the \
         escaping block, so soundness does not rest on the claim never mismatching"
    );
}

/// The bug-536 shape B-2 corpus, which had no concat in it and must stay exactly
/// as it was: a bare call, a literal, and an owned local that MOVES
/// (`plan_returned_move`). Each returns the only block it makes, so each has
/// nothing interior and must gain no free.
#[test]
fn the_return_shapes_with_nothing_interior_gain_no_free() {
    for (name, body) in [
        (
            "b567_bare_call",
            "FUNC probe(s AS String) AS String\n  RETURN strings::upper(s)\nEND FUNC",
        ),
        (
            "b567_literal",
            "FUNC probe(s AS String) AS String\n  RETURN \"literal\"\nEND FUNC",
        ),
        (
            "b567_owned_local",
            "FUNC probe(s AS String) AS String\n  LET r AS String = strings::upper(s)\n  RETURN r\nEND FUNC",
        ),
        (
            "b567_param",
            "FUNC probe(s AS String) AS String\n  RETURN s\nEND FUNC",
        ),
    ] {
        let plan = callee(name, body);
        assert_eq!(
            escaping_parks(&plan, "_mfb_fn_probe"),
            0,
            "{name}: no interior temp, so nothing may be parked — this shape's \
             codegen must be untouched by bug-567"
        );
        assert_eq!(
            string_frees(&plan, "_mfb_fn_probe"),
            0,
            "{name}: the one block this function makes is the one it returns, and \
             the caller owns it"
        );
    }
}

/// A `RETURN` whose escaping value is a COLLECTION, built from interior `String`
/// concats. Different escaping type, same interior temps — and the collection is
/// the case where `store_pending_success_result` may re-materialise the payload,
/// so the free has to come after that and not before.
#[test]
fn a_returned_collection_frees_the_strings_that_built_it() {
    let plan = ncode(
        "b567_list",
        "IMPORT io\n\
         IMPORT strings\n\
         FUNC probe(n AS Integer) AS List OF String\n\
        \x20 RETURN [\"a\" & toString(n), \"b\" & toString(n)]\n\
         END FUNC\n\
         SUB main()\n\
        \x20 io::print(strings::join(probe(1), \",\"))\n\
         END SUB\n",
    );
    assert_eq!(concats(&plan, "_mfb_fn_probe"), 2, "one concat per element");
    assert_eq!(
        pending_temps(&plan, "_mfb_fn_probe"),
        5,
        "two `toString` results, two concat results, and the `List` block itself"
    );
    assert_eq!(
        escaping_parks(&plan, "_mfb_fn_probe"),
        1,
        "the `List` block is parked once while the other four blocks are freed"
    );
    assert_eq!(
        string_frees(&plan, "_mfb_fn_probe"),
        4,
        "five temps, one of which LEAVES: every element string and every \
         `toString` result is copied INTO the list block and is interior after \
         that. The pre-fix compiler emitted ZERO frees here — this shape leaked \
         four blocks per call, not one"
    );
}

/// The `RETURN` path with live cleanups — a function that owns a local goes
/// through `store_pending_success_result` + `emit_cleanup_sequence` instead of the
/// register fast path, and parks the escaping value in `pending_result_slots`
/// rather than a slot of its own.
#[test]
fn the_cleanup_bearing_return_path_frees_its_interior_block_too() {
    let plan = callee(
        "b567_with_local",
        "FUNC probe(s AS String) AS String\n\
        \x20 LET tag AS String = strings::upper(s)\n\
        \x20 RETURN tag & (\"-\" & s)\n\
         END FUNC",
    );
    assert_eq!(
        concats(&plan, "_mfb_fn_probe"),
        2,
        "the inner `\"-\" & s` and the outer join"
    );
    assert_eq!(
        escaping_parks(&plan, "_mfb_fn_probe"),
        0,
        "this path needs no park of its own: the escaping value is already in \
         `pending_result_slots.value` when the interior free runs"
    );
    assert_eq!(
        string_frees(&plan, "_mfb_fn_probe"),
        2,
        "the owned local `tag` (scope drop) plus the one interior concat; one \
         alone is bug-567's leak still live"
    );
    assert_eq!(
        escape_guards(&plan, "_mfb_fn_probe"),
        1,
        "and the interior free is guarded here too — against the slot \
         `store_pending_success_result` wrote, so the identity check is the same \
         on both `RETURN` lowerings rather than only on the parked one"
    );
}

/// `Fail` is classified `AdoptedByTheCatcher`, and this is that decision read off
/// the emitted code.
///
/// The `Error` block a `FAIL` registers is parked in the per-thread current-error
/// slot precisely BECAUSE the control transfer forgets it — `emit_direct_error_return`
/// says so — and the catcher frees it exactly once (design "b"). Extending
/// bug-567's interior free to this statement would free the block the catcher
/// adopts: a double free, on a path no leak test would flag.
///
/// The counts below are the PRE-FIX counts, read off the base compiler
/// (`ac421788a`) and unchanged by bug-567 — `error(...)`'s own constructor
/// lowering already owns the message temps, which is why the RSS pin
/// `a_failing_trap_does_not_leak_its_interior_temp` comes out as an equality
/// rather than a growth.
#[test]
fn a_fail_is_never_given_an_interior_free() {
    let interior = ncode(
        "b567_fail",
        "IMPORT io\n\
         FUNC probe(n AS Integer) AS String\n\
        \x20 FAIL error(7, \"x\" & toString(n))\n\
         END FUNC\n\
         SUB main()\n\
        \x20 LET s AS String = probe(1) TRAP(e)\n\
        \x20   RECOVER \"f\"\n\
        \x20 END TRAP\n\
        \x20 io::print(s)\n\
         END SUB\n",
    );
    let contrast = ncode(
        "b567_fail_flat",
        "IMPORT io\n\
         FUNC probe(n AS Integer) AS String\n\
        \x20 FAIL error(7, \"x\")\n\
         END FUNC\n\
         SUB main()\n\
        \x20 LET s AS String = probe(1) TRAP(e)\n\
        \x20   RECOVER \"f\"\n\
        \x20 END TRAP\n\
        \x20 io::print(s)\n\
         END SUB\n",
    );
    assert_eq!(
        concats(&interior, "_mfb_fn_probe"),
        1,
        "the message concat, whose block is interior to the `error(...)` call"
    );
    assert_eq!(
        escaping_parks(&interior, "_mfb_fn_probe"),
        0,
        "a `FAIL` parks nothing: bug-567's interior free is reachable only from a          `RETURN`, and this is the assertion that says so directly"
    );
    assert_eq!(
        escaping_parks(&contrast, "_mfb_fn_probe"),
        0,
        "nor does the contrast"
    );
    assert_eq!(
        string_frees(&interior, "_mfb_fn_probe"),
        2,
        "the two frees `error(...)`'s constructor lowering already emitted before          this fix. If this rises, check FIRST that the new one is not the `Error`          block itself — the catcher adopts that one, so freeing it here is a          double free rather than a leak fix"
    );
    assert_eq!(
        string_frees(&contrast, "_mfb_fn_probe"),
        0,
        "and with no concat in the message there is nothing for it to own — which \
         is what makes the 2 above attributable to the message temps rather than \
         to the `Error` block"
    );
}
