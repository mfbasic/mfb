//! bug-566: who owns the block a RUNTIME HELPER returned, when the call sits
//! under an inline `TRAP`.
//!
//! `materialize_current_result` copies the helper's block into an intermediate,
//! copies that into the flat `Result`, and frees the intermediate (bug-379). The
//! helper's ORIGINAL was then dead with no owner. Outside a `TRAP` the same call
//! is bound directly and its scope drop frees it — so the leak is created purely
//! by the `Result` lowering.
//!
//! `rt_scope_drop_leaks` measures the megabytes, comparatively (a separate,
//! pre-existing argument leak makes flatness unavailable here). These count the
//! OWNERS per function, because the two failure directions are invisible to each
//! other: too few is the leak, too many is a free of a block another OWNER — or,
//! for `thread::waitFor`, another ARENA — still holds, and `x19` is per-thread, so
//! that one is not a double free but memory corruption in a different heap.
//!
//! Every assertion is COMPARATIVE against a sibling that differs in one way: the
//! `TRAP`, the payload's type, or the package the call comes from.
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

/// How many raw helper results `symbol` takes ownership of. The guard label is
/// emitted once per free, and only on the path bug-566 added, so it counts the new
/// owners exactly — and it is zero for every program that never inline-`TRAP`s a
/// block-returning runtime helper, which is what keeps this fix off everyone
/// else's codegen.
fn helper_result_owners(plan: &Value, symbol: &str) -> usize {
    function(plan, symbol)["instructions"]
        .as_array()
        .expect("instructions array")
        .iter()
        .filter(|i| {
            i["op"].as_str() == Some("label")
                && i["name"]
                    .as_str()
                    .is_some_and(|n| n.starts_with("raw_helper_result_kept"))
        })
        .count()
}

/// How many `Result` blocks `symbol` materialises — the DENOMINATOR. One per
/// inline `TRAP` over a runtime helper, whether or not its payload is a block.
fn materialised_results(plan: &Value, symbol: &str) -> usize {
    function(plan, symbol)["stackSlots"]
        .as_array()
        .expect("stack slots array")
        .iter()
        .filter(|slot| slot["type"].as_str() == Some("raw_result"))
        .count()
}

fn main_of(name: &str, body: &str) -> Value {
    ncode(
        name,
        &format!(
            "IMPORT io\n\
             IMPORT fs\n\
             IMPORT os\n\
             IMPORT thread\n\
             ISOLATED FUNC worker(w AS ThreadWorker OF String TO String, seed AS String) AS String\n\
            \x20 RETURN seed\n\
             END FUNC\n\
             SUB main()\n\
             {body}\n\
             END SUB\n"
        ),
    )
}

/// The bug and its contrast: the SAME call, once trapped and once not.
///
/// Without the `TRAP` the binding owns the helper's block and its scope drop frees
/// it — one owner, and the program is not this bug. With the `TRAP` the block is
/// copied into a `Result` and the binding owns the `Result`, so the original needs
/// an owner of its own. Zero here is the leak; two would free it twice.
#[test]
fn a_trapped_helper_string_result_gains_exactly_one_owner_and_the_plain_call_none() {
    let trapped = main_of(
        "b566_trapped",
        "\x20 LET s AS String = fs::readText(\"p.txt\") TRAP(e)\n\
        \x20   RECOVER \"x\"\n\
        \x20 END TRAP\n\
        \x20 io::print(s)",
    );
    let plain = main_of(
        "b566_plain",
        "\x20 LET s AS String = fs::readText(\"p.txt\")\n\
        \x20 io::print(s)",
    );
    assert_eq!(
        materialised_results(&plain, "_mfb_fn_main"),
        0,
        "no `TRAP`, no materialised `Result` — the binding takes the helper's \
         block directly"
    );
    assert_eq!(
        helper_result_owners(&plain, "_mfb_fn_main"),
        0,
        "and therefore no new owner: the untrapped spelling's codegen is \
         byte-identical to the pre-fix compiler's"
    );
    assert_eq!(
        materialised_results(&trapped, "_mfb_fn_main"),
        1,
        "one inline `TRAP`, one materialised `Result`"
    );
    assert_eq!(
        helper_result_owners(&trapped, "_mfb_fn_main"),
        1,
        "the helper's own block must have exactly ONE owner; zero is bug-566's \
         leak, two frees a block already freed"
    );
}

/// The payload TYPE is what decides, not the helper: `fs::exists` runs the same
/// `TRAP` machinery and materialises the same `Result`, but its `Boolean` payload
/// has no block at all, so `result_payload_is_block` is false and nothing is
/// emitted. This is the row that attributes the fix to the PAYLOAD.
#[test]
fn a_trapped_helper_scalar_result_gains_no_owner() {
    let scalar = main_of(
        "b566_scalar",
        "\x20 LET b AS Boolean = fs::exists(\"p.txt\") TRAP(e)\n\
        \x20   RECOVER FALSE\n\
        \x20 END TRAP\n\
        \x20 io::print(toString(b))",
    );
    assert_eq!(
        materialised_results(&scalar, "_mfb_fn_main"),
        1,
        "the same `TRAP` lowering runs"
    );
    assert_eq!(
        helper_result_owners(&scalar, "_mfb_fn_main"),
        0,
        "but a `Boolean` payload is stored inline in the `Result`; there is no \
         producer block to free"
    );
}

/// A resource HANDLE is a `Named` type but not a flat value block: its lifetime is
/// the §15 close obligation, and `arena_free`ing a handle would corrupt the free
/// list AND close nothing.
#[test]
fn a_trapped_helper_resource_result_gains_no_owner() {
    let resource = ncode(
        "b566_resource",
        "IMPORT io\n\
         IMPORT fs\n\
         SUB main()\n\
        \x20 LET ok AS Boolean = TRUE\n\
        \x20 RES f AS fs::File = fs::openFile(\"p.txt\", \"read\") TRAP(e)\n\
        \x20   EXIT SUB\n\
        \x20 END TRAP\n\
        \x20 io::print(toString(ok))\n\
         END SUB\n",
    );
    assert_eq!(
        materialised_results(&resource, "_mfb_fn_main"),
        1,
        "the same `TRAP` lowering runs"
    );
    assert_eq!(
        helper_result_owners(&resource, "_mfb_fn_main"),
        0,
        "`is_freeable_flat_value` is false for a resource handle, so the audit \
         declines it"
    );
}

/// THE counter-example the audit exists for.
///
/// `thread::waitFor` returns the worker's value, and the worker allocated it in
/// the WORKER's arena — `x19` is per-thread. Its declared return type is the type
/// VARIABLE `Out`, so at this call site it is a perfectly ordinary `String` and
/// nothing in the type says otherwise; only the call's identity does. Freeing it
/// here is not a leak fix, it is a write into another thread's heap, and it would
/// show up as a wrong value somewhere else entirely.
///
/// The comparison is against `fs::readText` in the SAME program with the SAME
/// `String` payload: one owner there, none here.
#[test]
fn a_trapped_thread_wait_for_result_is_never_given_an_owner() {
    let plan = main_of(
        "b566_thread",
        "\x20 LET t AS Thread OF String TO String = thread::start(worker, \"seed\")\n\
        \x20 LET out AS String = thread::waitFor(t) TRAP(e)\n\
        \x20   RECOVER \"x\"\n\
        \x20 END TRAP\n\
        \x20 LET s AS String = fs::readText(\"p.txt\") TRAP(e)\n\
        \x20   RECOVER \"y\"\n\
        \x20 END TRAP\n\
        \x20 io::print(out & s)",
    );
    assert_eq!(
        materialised_results(&plan, "_mfb_fn_main"),
        2,
        "two inline `TRAP`s over runtime helpers, two materialised `Result`s"
    );
    assert_eq!(
        helper_result_owners(&plan, "_mfb_fn_main"),
        1,
        "exactly one of the two payload blocks may be freed here. The \
         `fs::readText` block is this arena's; the `thread::waitFor` block is the \
         WORKER's, and freeing it is cross-arena corruption. If this reads 2, the \
         `thread` exclusion in `runtime_call_result_is_foreign_arena` stopped \
         working"
    );
}

/// A COLLECTION payload — a different block shape and a different drop — and a
/// second package, so the licence is not read off `fs` alone.
#[test]
fn a_trapped_helper_collection_result_gains_one_owner() {
    let plan = main_of(
        "b566_collection",
        "\x20 LET b AS List OF Byte = fs::readBytes(\"p.txt\") TRAP(e)\n\
        \x20   RECOVER []\n\
        \x20 END TRAP\n\
        \x20 LET env AS String = os::getEnvOr(\"B566_NOT_SET\", \"d\") TRAP(e)\n\
        \x20   RECOVER \"x\"\n\
        \x20 END TRAP\n\
        \x20 io::print(toString(len(b)) & env)",
    );
    assert_eq!(
        materialised_results(&plan, "_mfb_fn_main"),
        2,
        "two trapped helpers"
    );
    assert_eq!(
        helper_result_owners(&plan, "_mfb_fn_main"),
        2,
        "both payloads are blocks this arena allocated: a `List OF Byte` from \
         `fs` and a `String` from `os`"
    );
}
