//! bug-572: who owns a capturing `LAMBDA` written as a call argument.
//!
//! A capturing lambda allocates three arena blocks per evaluation — the env, one
//! deep copy per freeable-flat capture, and the 16-byte closure object — and as
//! an argument it had no owner at all. `register_pending_temp` declines it
//! (`is_freeable_flat_value` is false for `Func`, and must stay false: a `Func`
//! element in a collection is a shared POINTER, bug-73), and the only closure
//! free in the tree, `emit_closure_drop`, was reachable only through the `Bind`
//! gate for a NAMED, invoke-only binding. 13 MB at 50 000 `collections::filter`
//! calls, 25 MB at 100 000, with a `Boolean` predicate that allocates no result.
//!
//! The RSS cases in `rt_scope_drop_leaks` measure the leak. These count the
//! OWNERS, because the two failure directions are invisible to each other:
//! too few is the leak, too many is a use-after-free on a closure the callee
//! kept — and the second one surfaces later, somewhere else, as a wrong value or
//! an "Allocation failed" in an unrelated allocation.
//!
//! Every assertion is COMPARATIVE against a sibling that differs in one way.
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

/// How many closure temporaries `symbol` takes ownership of. `lower_value`
/// allocates exactly one `closure_temp` slot per closure a call has already
/// decided it may free, so this counts the owners directly — and it is zero for
/// every program that passes no capturing lambda to a callback position, which
/// is what keeps this fix off everyone else's codegen.
fn closure_temps(plan: &Value, symbol: &str) -> usize {
    function(plan, symbol)["stackSlots"]
        .as_array()
        .expect("stack slots array")
        .iter()
        .filter(|slot| slot["type"].as_str() == Some("closure_temp"))
        .count()
}

/// How many closure ENV blocks `symbol` allocates — one per capturing lambda it
/// constructs, freed or not. The denominator the owner count is read against.
fn closure_envs(plan: &Value, symbol: &str) -> usize {
    function(plan, symbol)["stackSlots"]
        .as_array()
        .expect("stack slots array")
        .iter()
        .filter(|slot| slot["type"].as_str() == Some("closure_env"))
        .count()
}

fn filter_program(name: &str, predicate: &str) -> Value {
    ncode(
        name,
        &format!(
            "IMPORT io\n\
             IMPORT collections\n\
             SUB main()\n\
            \x20 LET cap AS String = \"CAPTURED\"\n\
            \x20 LET xs AS List OF String = [\"a\", \"bb\", \"ccc\"]\n\
            \x20 LET c AS List OF String = collections::filter(xs, {predicate})\n\
            \x20 io::print(toString(len(c)) & cap)\n\
             END SUB\n"
        ),
    )
}

/// The bug's own shape against its own contrast: dropping the capture is the one
/// token that separates them, and it is what made the leak flat (13 -> 25 MB vs
/// 1.0 MB at both counts).
///
/// A capture-less lambda lowers to a `FunctionRef` over a STATIC BSS descriptor —
/// zero arena allocations — so it has nothing to own and must gain no owner.
#[test]
fn a_capturing_lambda_argument_gains_exactly_one_owner_and_a_captureless_one_none() {
    let capturing = filter_program("b572_capturing", "LAMBDA(s AS String) -> len(s) < len(cap)");
    let captureless = filter_program("b572_captureless", "LAMBDA(s AS String) -> len(s) < 3");
    assert_eq!(
        closure_envs(&captureless, "_mfb_fn_main"),
        0,
        "a capture-less lambda allocates no environment — it is a static \
         descriptor, and the bug report's flat contrast"
    );
    assert_eq!(
        closure_temps(&captureless, "_mfb_fn_main"),
        0,
        "nothing to own, so no owner: the capture-less program's codegen is \
         untouched by this fix"
    );
    assert_eq!(
        closure_envs(&capturing, "_mfb_fn_main"),
        1,
        "one capturing lambda, one environment block"
    );
    assert_eq!(
        closure_temps(&capturing, "_mfb_fn_main"),
        1,
        "the one environment must have exactly ONE owner; zero is bug-572's leak, \
         two is a double free"
    );
}

/// The POSITIVE pin the whole design turns on, and the one the enumeration got
/// wrong on the first try: `http::route`'s handler is ALSO a non-isolated `FUNC`
/// parameter, and the returned `http::Route` KEEPS it — the server invokes it per
/// request, long after the call returned. Freeing it there is a use-after-free in
/// the request loop.
///
/// So the gate is an explicit allow-list
/// (`registry::SYNCHRONOUS_CALLBACK_PARAMETERS`) rather than "any `FUNC`
/// parameter", and this is that decision read off the emitted code: the same
/// capturing lambda gains an owner at `collections::filter` and none at
/// `http::route`.
#[test]
fn a_retained_callback_argument_is_never_given_an_owner() {
    let retained = ncode(
        "b572_http_route",
        "IMPORT io\n\
         IMPORT http\n\
         SUB main()\n\
        \x20 LET cap AS String = \"CAPTURED\"\n\
        \x20 LET r AS http::Route = http::route(\"/x\", LAMBDA(req AS http::Request) -> http::ok(cap))\n\
        \x20 io::print(r.pattern)\n\
         END SUB\n",
    );
    assert_eq!(
        closure_envs(&retained, "_mfb_fn_main"),
        1,
        "the capturing handler still allocates its environment"
    );
    assert_eq!(
        closure_temps(&retained, "_mfb_fn_main"),
        0,
        "`http::route` STORES its handler on the returned `http::Route`, so the \
         caller must not free it — an owner here is a use-after-free the next \
         time the route is served"
    );
}

/// The other escapes, in one program: a closure appended to a collection, one
/// handed to a user function that RETURNS it, and one stored into a global. None
/// is a callback position, so none may gain an owner.
#[test]
fn an_escaping_closure_is_never_given_an_owner() {
    let escaping = ncode(
        "b572_escapes",
        "IMPORT io\n\
         IMPORT collections\n\
         MUT gfn AS FUNC(String) AS Boolean = LAMBDA(s AS String) -> len(s) < 3\n\
         FUNC hold(f AS FUNC(String) AS Boolean) AS FUNC(String) AS Boolean\n\
        \x20 RETURN f\n\
         END FUNC\n\
         SUB main()\n\
        \x20 LET cap AS String = \"CAPTURED\"\n\
        \x20 MUT fs AS List OF FUNC(String) AS Boolean = []\n\
        \x20 fs = collections::append(fs, LAMBDA(s AS String) -> len(s) < len(cap))\n\
        \x20 LET h AS FUNC(String) AS Boolean = hold(LAMBDA(s AS String) -> len(s) < len(cap))\n\
        \x20 gfn = LAMBDA(s AS String) -> len(s) < len(cap)\n\
        \x20 io::print(toString(h(\"ab\")) & toString(gfn(\"ab\")) & toString(len(fs)))\n\
         END SUB\n",
    );
    assert_eq!(
        closure_envs(&escaping, "_mfb_fn_main"),
        3,
        "three capturing lambdas, three environment blocks"
    );
    assert_eq!(
        closure_temps(&escaping, "_mfb_fn_main"),
        0,
        "an `append` argument is stored in the collection as a shared POINTER \
         (bug-73), `hold` RETURNS its parameter, and a global outlives the \
         statement — none is a callback position, and freeing any of them is a \
         use-after-free"
    );
}

/// A user-written HOF gets the same licence as a builtin one, from its own body
/// rather than the registry: the callable parameter must be a non-isolated `FUNC`
/// AND never read as a VALUE anywhere (`collect_value_used_locals`), which is
/// exactly the proof `is_non_escaping_closure` makes for a closure BINDING, made
/// one frame down.
///
/// `applyTwice` only invokes `f`, so it earns the free; `hold` returns it, so it
/// does not. The pair is the whole gate.
#[test]
fn a_user_hof_earns_the_free_only_when_its_parameter_is_invoke_only() {
    fn program(name: &str, callee: &str, body: &str) -> Value {
        ncode(
            name,
            &format!(
                "IMPORT io\n\
                 {callee}\
                 SUB main()\n\
                \x20 LET bump AS Integer = 5\n\
                 {body}\
                 END SUB\n"
            ),
        )
    }
    let invoke_only = program(
        "b572_user_invoke_only",
        "FUNC applyTwice(f AS FUNC(Integer) AS Integer, v AS Integer) AS Integer\n\
        \x20 RETURN f(f(v))\n\
         END FUNC\n",
        "  io::print(toString(applyTwice(LAMBDA(v AS Integer) -> v + bump, 1)))\n",
    );
    let returns_it = program(
        "b572_user_returns_it",
        "FUNC hold(f AS FUNC(Integer) AS Integer) AS FUNC(Integer) AS Integer\n\
        \x20 RETURN f\n\
         END FUNC\n",
        "  LET g AS FUNC(Integer) AS Integer = hold(LAMBDA(v AS Integer) -> v + bump)\n\
        \x20 io::print(toString(g(1)))\n",
    );
    assert_eq!(closure_envs(&invoke_only, "_mfb_fn_main"), 1);
    assert_eq!(closure_envs(&returns_it, "_mfb_fn_main"), 1);
    assert_eq!(
        closure_temps(&invoke_only, "_mfb_fn_main"),
        1,
        "`applyTwice` only ever INVOKES `f` — a `Call`'s target is a name, not a \
         value, so `f` is not in `collect_value_used_locals` and the closure is \
         dead when the call returns"
    );
    assert_eq!(
        closure_temps(&returns_it, "_mfb_fn_main"),
        0,
        "`hold` reads `f` as a VALUE (it returns it), so the caller may not free \
         it — this is the fail-closed half, and it is what keeps a user HOF from \
         being trusted by its signature alone"
    );
}

/// A nested call whose own argument is a capturing lambda: each `lower_value`
/// frame drains only the closures registered inside it, so the inner call frees
/// its own and the outer frees its own. The pairing is positional, and a
/// mismatch would free the wrong one — which is why it is counted rather than
/// reasoned about.
#[test]
fn nested_hof_calls_each_own_their_own_closure() {
    let nested = ncode(
        "b572_nested",
        "IMPORT io\n\
         IMPORT collections\n\
         SUB main()\n\
        \x20 LET cap AS String = \"CAPTURED\"\n\
        \x20 LET xs AS List OF String = [\"a\", \"bb\", \"ccc\"]\n\
        \x20 LET c AS List OF String = collections::filter(collections::filter(xs, LAMBDA(s AS String) -> len(s) < len(cap)), LAMBDA(s AS String) -> len(s) > 1)\n\
        \x20 io::print(toString(len(c)) & cap)\n\
         END SUB\n",
    );
    // The inner lambda captures `cap`; the outer one does not, so only the inner
    // allocates an environment — and it is the inner call that owns it.
    assert_eq!(closure_envs(&nested, "_mfb_fn_main"), 1);
    assert_eq!(
        closure_temps(&nested, "_mfb_fn_main"),
        1,
        "the inner call's own capturing lambda gets exactly one owner, and the \
         outer call's drain must not claim it a second time"
    );
}
