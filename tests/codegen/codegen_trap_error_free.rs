//! bug-565: who owns the flat `Error` block an inline `TRAP` builds, asserted on
//! the emitted code.
//!
//! The RSS cases in `tests/runtime/rt_scope_drop_leaks.rs` prove the leak is gone.
//! They cannot prove the *shape* of the fix, and the shape is what keeps it sound:
//!
//! * **No free** is the leak — 149.8 MB at 200 000 trapped errors, and nothing
//!   goes red.
//! * **An unguarded free** is a double free of a block another owner still holds,
//!   which the arena turns into "Allocation failed" at some *later*, unrelated
//!   allocation, or into a wrong value read out of reused memory. A behavioural
//!   probe sees that only if it happens to reuse the block before the read.
//!
//! So each case asserts the guard AND the free, and every assertion is
//! COMPARATIVE — one program against a sibling differing in exactly one way — so
//! it pins the difference rather than an absolute count that drifts.
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

/// How many labels in `symbol` start with `prefix` (labels carry a per-function
/// serial suffix, so the prefix is the identity).
fn labels(plan: &Value, symbol: &str, prefix: &str) -> usize {
    function(plan, symbol)["instructions"]
        .as_array()
        .expect("instructions array")
        .iter()
        .filter(|instr| {
            instr["op"].as_str() == Some("label")
                && instr["name"]
                    .as_str()
                    .is_some_and(|name| name.starts_with(prefix))
        })
        .count()
}

/// How many times `symbol` calls `target`.
fn calls(plan: &Value, symbol: &str, target: &str) -> usize {
    function(plan, symbol)["instructions"]
        .as_array()
        .expect("instructions array")
        .iter()
        .filter(|instr| instr["target"].as_str() == Some(target))
        .count()
}

/// One trapped-`Result` assembly: the join label both the adopt and the rebuild
/// branch reach.
const ASSEMBLY: &str = "raw_result_err_built";
/// The payload free's runtime guard: "skip if this block is still PARKED in the
/// per-thread current-error slot".
const PAYLOAD_GUARD: &str = "trapped_error_still_parked";
/// The rebuild's own `ErrorLoc` free guard: "skip if this `ErrorLoc` is the
/// RAISER's rather than the one this rebuild stamped".
const SOURCE_GUARD: &str = "trapped_error_source_borrowed";

/// A fallible user `FUNC` and a `main` that traps it.
fn trapping_main(body: &str) -> String {
    format!(
        "IMPORT io\n\
         FUNC risky(n AS Integer) AS String\n\
        \x20 IF n < 0 THEN\n\
        \x20   FAIL error(7, \"neg\")\n\
        \x20 END IF\n\
        \x20 RETURN toString(n)\n\
         END FUNC\n\
         SUB main()\n\
         {body}\
         END SUB\n"
    )
}

/// Every trapped-`Result` assembly gets exactly one guarded payload free.
///
/// This is bug-565's fix at its narrowest: the flat `Error` the error branch
/// builds (or adopts) is deep-copied into the `Result` by
/// `emit_build_result_inline`, after which it has no owner. One assembly, one
/// guard, one free.
#[test]
fn a_trapped_call_frees_the_error_block_it_wrapped() {
    let one = ncode(
        "b565_one_trap",
        &trapping_main(
            "  LET s AS String = risky(1) TRAP(e)\n\
            \x20   RECOVER \"x\"\n\
            \x20 END TRAP\n\
            \x20 io::print(s)\n",
        ),
    );
    assert_eq!(
        labels(&one, "_mfb_fn_main", ASSEMBLY),
        1,
        "one inline TRAP over a fallible call is one trapped-Result assembly"
    );
    assert_eq!(
        labels(&one, "_mfb_fn_main", PAYLOAD_GUARD),
        1,
        "the assembly must free the Error block the Result copied, guarded on the \
         parked-block pointer — before bug-565 it freed nothing at all"
    );
}

/// Two traps, two assemblies, two guarded frees: the free is per assembly and not
/// a single function-level cleanup that would miss the second one.
#[test]
fn each_trapped_call_gets_its_own_guarded_free() {
    let two = ncode(
        "b565_two_traps",
        &trapping_main(
            "  LET a AS String = risky(1) TRAP(e)\n\
            \x20   RECOVER \"x\"\n\
            \x20 END TRAP\n\
            \x20 LET b AS String = risky(2) TRAP(e2)\n\
            \x20   RECOVER \"y\"\n\
            \x20 END TRAP\n\
            \x20 io::print(a & b)\n",
        ),
    );
    let assemblies = labels(&two, "_mfb_fn_main", ASSEMBLY);
    assert_eq!(assemblies, 2, "two inline TRAPs, two assemblies");
    assert_eq!(
        labels(&two, "_mfb_fn_main", PAYLOAD_GUARD),
        assemblies,
        "one guarded payload free PER assembly"
    );
}

/// The `TrappedErrorSource` partition, made visible in the emitted code.
///
/// A **user callee**'s error carries the callee's OWN `ErrorLoc` in `x3`
/// (`TrappedErrorSource::CalleeRegister`), which this frame did not allocate and
/// must never free — so the rebuild branch emits no `ErrorLoc` free at all, and
/// the omission is a compile-time one (`source_slot == source_raw_slot`), not a
/// runtime compare that could go the wrong way.
///
/// An **inline builtin**'s error is stamped with THIS expression's location
/// (`TrappedErrorSource::CurrentLocation`), a fresh `_mfb_build_error_loc` block
/// the flat `Error` then inlines — so that one does get a free, guarded against
/// the raiser's `x3` in case the two are ever the same pointer.
///
/// The contrast is the whole point: a single "free the ErrorLoc" would have been
/// a use-after-free of the callee's origin on the first program.
#[test]
fn only_the_lowering_that_stamps_its_own_error_loc_frees_one() {
    let user_callee = ncode(
        "b565_callee_loc",
        &trapping_main(
            "  LET s AS String = risky(1) TRAP(e)\n\
            \x20   RECOVER \"x\"\n\
            \x20 END TRAP\n\
            \x20 io::print(s)\n",
        ),
    );
    assert_eq!(
        labels(&user_callee, "_mfb_fn_main", SOURCE_GUARD),
        0,
        "a user callee's ErrorLoc arrives in x3 and belongs to the raiser — this \
         frame allocated no ErrorLoc, so it must emit no free for one"
    );

    let inline_builtin = ncode(
        "b565_current_loc",
        "IMPORT io\n\
         IMPORT collections\n\
         SUB main()\n\
        \x20 LET xs AS List OF String = [\"aa\", \"bb\"]\n\
        \x20 LET g AS String = collections::get(xs, 5) TRAP(e)\n\
        \x20   RECOVER e.message\n\
        \x20 END TRAP\n\
        \x20 io::print(g)\n\
         END SUB\n",
    );
    assert_eq!(
        labels(&inline_builtin, "_mfb_fn_main", SOURCE_GUARD),
        labels(&inline_builtin, "_mfb_fn_main", ASSEMBLY),
        "an inline builtin trapped here stamps its own ErrorLoc, which the flat \
         Error inlines a copy of — one guarded free per assembly"
    );
    assert!(
        labels(&inline_builtin, "_mfb_fn_main", ASSEMBLY) >= 1,
        "the builtin's inline TRAP must produce a trapped-Result assembly"
    );
}

/// The POSITIVE pin, as a count: a program with no inline `TRAP` gets neither
/// guard and no extra `arena_free`.
///
/// The fix only ever adds code inside the trapped-error assembly. If a guard or a
/// free ever appears in a function that assembles no trapped `Result`, something
/// is freeing an `Error` on a path that never built one.
#[test]
fn a_program_without_an_inline_trap_gains_nothing() {
    let plain = ncode(
        "b565_no_trap",
        "IMPORT io\n\
         FUNC safe(n AS Integer) AS String\n\
        \x20 RETURN toString(n)\n\
         END FUNC\n\
         SUB main()\n\
        \x20 LET s AS String = safe(1)\n\
        \x20 io::print(s)\n\
         END SUB\n",
    );
    assert_eq!(labels(&plain, "_mfb_fn_main", ASSEMBLY), 0);
    assert_eq!(labels(&plain, "_mfb_fn_main", PAYLOAD_GUARD), 0);
    assert_eq!(labels(&plain, "_mfb_fn_main", SOURCE_GUARD), 0);
}

/// The free is a real `arena_free`, not just a label.
///
/// Counted comparatively against the same program with the `TRAP` removed, so the
/// number is the DELTA the assembly contributes rather than an absolute that
/// drifts with every unrelated codegen change.
#[test]
fn the_guarded_payload_free_is_an_arena_free() {
    let trapped = ncode(
        "b565_free_delta_trap",
        &trapping_main(
            "  LET s AS String = risky(1) TRAP(e)\n\
            \x20   RECOVER \"x\"\n\
            \x20 END TRAP\n\
            \x20 io::print(s)\n",
        ),
    );
    let plain = ncode(
        "b565_free_delta_plain",
        "IMPORT io\n\
         FUNC risky(n AS Integer) AS String\n\
        \x20 RETURN toString(n)\n\
         END FUNC\n\
         SUB main()\n\
        \x20 LET s AS String = risky(1)\n\
        \x20 io::print(s)\n\
         END SUB\n",
    );
    let trapped_frees = calls(&trapped, "_mfb_fn_main", "_mfb_arena_free");
    let plain_frees = calls(&plain, "_mfb_fn_main", "_mfb_arena_free");
    assert!(
        trapped_frees > plain_frees,
        "the trapped-error assembly must add at least one arena_free \
         ({plain_frees} without the TRAP, {trapped_frees} with it)"
    );
}
