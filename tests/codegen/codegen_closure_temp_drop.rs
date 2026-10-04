//! bug-572 / bug-781: who owns a capturing `LAMBDA` written as a call argument.
//!
//! A capturing lambda allocates its environment (plus one deep copy per owned
//! capture) and a 16-byte closure object per evaluation, and as an argument it
//! had no owner at all — 13 MB at 50 000 `collections::filter` calls, 25 MB at
//! 100 000, with a `Boolean` predicate that allocates no result (bug-572).
//!
//! bug-781 made function values single-owner (`mfb spec memory closures`,
//! *Ownership*): every holder owns its own closure, copying one into a second
//! holder copies it (`_mfb_rt_closure_copy`), and dropping a holder frees it
//! (`_mfb_rt_closure_drop`). A closure argument is therefore the caller's
//! statement temporary, freed once the statement is done, at EVERY call position:
//! a callee that keeps it (`http::route`, a user function that returns it, an
//! `append`) keeps its own copy or takes the fresh value over, so no caller has to
//! know whether a callee retains its argument. The runtime counters
//! (`rt_closure_single_owner`) pin "exactly once"; these pin, from the emitted
//! code, where the owner's drop and the copies are — the two failure directions
//! being a leak (no drop) and a use-after-free (a retaining callee with no copy).
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

/// How many call sites in `symbol` reach `helper` — `_mfb_rt_closure_drop` (an
/// owner frees a function value; a statement temp's drop is emitted at the
/// statement's end and on each of its error exits, so a site count is not an
/// owner count) or `_mfb_rt_closure_copy` (a second holder takes its own copy).
fn calls(plan: &Value, symbol: &str, helper: &str) -> usize {
    function(plan, symbol)["relocations"]
        .as_array()
        .expect("relocations array")
        .iter()
        .filter(|relocation| relocation["to"].as_str() == Some(helper))
        .count()
}

fn drops(plan: &Value, symbol: &str) -> usize {
    calls(plan, symbol, "_mfb_rt_closure_drop")
}

fn copies(plan: &Value, symbol: &str) -> usize {
    calls(plan, symbol, "_mfb_rt_closure_copy")
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
             FUNC main() AS Nothing\n\
            \x20 LET cap AS String = \"CAPTURED\"\n\
            \x20 LET xs AS List OF String = [\"a\", \"bb\", \"ccc\"]\n\
            \x20 LET c AS List OF String = collections::filter(xs, {predicate})\n\
            \x20 io::print(toString(len(c)) & cap)\n\
             END FUNC\n"
        ),
    )
}

/// The bug's own shape against its own contrast: dropping the capture is the one
/// token that separates them, and it is what made the leak flat (13 -> 25 MB vs
/// 1.0 MB at both counts).
///
/// A capture-less lambda lowers to a `FunctionRef` over a STATIC BSS descriptor —
/// zero arena allocations — so it has nothing to own: no environment, no drop.
/// The capturing one is the statement's temp: dropped, and never copied, since
/// `filter` only borrows its callback.
#[test]
fn a_capturing_lambda_argument_is_dropped_by_its_statement_and_a_captureless_one_has_no_owner() {
    let capturing = filter_program("b572_capturing", "LAMBDA(s AS String) -> len(s) < len(cap)");
    let captureless = filter_program("b572_captureless", "LAMBDA(s AS String) -> len(s) < 3");
    assert_eq!(
        closure_envs(&captureless, "_mfb_fn_main"),
        0,
        "a capture-less lambda allocates no environment — it is a static \
         descriptor, and the bug report's flat contrast"
    );
    assert_eq!(
        drops(&captureless, "_mfb_fn_main") + copies(&captureless, "_mfb_fn_main"),
        0,
        "nothing to own, so nothing dropped or copied: a static descriptor is shared \
         by every reference to its function"
    );
    assert_eq!(
        closure_envs(&capturing, "_mfb_fn_main"),
        1,
        "one capturing lambda, one environment block"
    );
    assert!(
        drops(&capturing, "_mfb_fn_main") > 0,
        "the capturing argument must be dropped by its statement — no drop is \
         bug-572's leak"
    );
    assert_eq!(
        copies(&capturing, "_mfb_fn_main"),
        0,
        "`filter` borrows its callback: passing it copies nothing"
    );
}

/// The position the bug-572 enumeration once got wrong: `http::route`'s handler
/// is a non-isolated `FUNC` parameter too, and the returned `http::Route` KEEPS
/// it — the server invokes it per request, long after the call returned.
///
/// Under single ownership that is no longer a reason to leak the caller's
/// closure: `route` stores its OWN copy in the record (`__http_route`'s
/// construction copies the borrowed parameter), and the caller drops its temp
/// exactly as at `collections::filter`. Without the callee's copy the caller's
/// drop would be a use-after-free in the request loop, so both halves are pinned.
#[test]
fn a_retained_callback_argument_is_copied_by_the_callee_and_dropped_by_the_caller() {
    let retained = ncode(
        "b572_http_route",
        "IMPORT io\n\
         IMPORT http\n\
         FUNC main() AS Nothing\n\
        \x20 LET cap AS String = \"CAPTURED\"\n\
        \x20 LET r AS http::Route = http::route(\"/x\", LAMBDA(req AS http::Request) -> http::ok(cap))\n\
        \x20 io::print(r.pattern)\n\
         END FUNC\n",
    );
    assert_eq!(
        closure_envs(&retained, "_mfb_fn_main"),
        1,
        "the capturing handler still allocates its environment"
    );
    assert!(
        drops(&retained, "_mfb_fn_main") > 0,
        "the caller's handler is its statement temp and is dropped"
    );
    assert!(
        copies(&retained, "_mfb_ifn_http_5Froute") > 0,
        "`http::route` keeps the handler, so it must store a copy of its own — \
         the record field owns that copy, not the caller's closure"
    );
}

/// The other escapes, in one program: a closure appended to a collection, one
/// handed to a user function that RETURNS it, and one stored into a global. A
/// fresh closure stored by `append` or into the global is taken over by its new
/// holder; the one `hold` returns is the caller's own argument, so the binding
/// copies it before the statement's temp is dropped.
#[test]
fn an_escaping_closure_is_owned_by_where_it_escapes_to() {
    let escaping = ncode(
        "b572_escapes",
        "IMPORT io\n\
         IMPORT collections\n\
         MUT gfn AS FUNC(String) AS Boolean = LAMBDA(s AS String) -> len(s) < 3\n\
         FUNC hold(f AS FUNC(String) AS Boolean) AS FUNC(String) AS Boolean\n\
        \x20 RETURN f\n\
         END FUNC\n\
         FUNC main() AS Nothing\n\
        \x20 LET cap AS String = \"CAPTURED\"\n\
        \x20 MUT fs AS List OF FUNC(String) AS Boolean = []\n\
        \x20 fs = collections::append(fs, LAMBDA(s AS String) -> len(s) < len(cap))\n\
        \x20 LET h AS FUNC(String) AS Boolean = hold(LAMBDA(s AS String) -> len(s) < len(cap))\n\
        \x20 gfn = LAMBDA(s AS String) -> len(s) < len(cap)\n\
        \x20 io::print(toString(h(\"ab\")) & toString(gfn(\"ab\")) & toString(len(fs)))\n\
         END FUNC\n",
    );
    assert_eq!(
        closure_envs(&escaping, "_mfb_fn_main"),
        3,
        "three capturing lambdas, three environment blocks"
    );
    assert_eq!(
        copies(&escaping, "_mfb_fn_main"),
        1,
        "exactly the closure `hold` hands back is copied: it is the caller's own \
         argument, which the statement then drops; the appended and the global \
         closures are fresh and taken over where they land"
    );
}

/// A user-written HOF that only INVOKES its callable parameter borrows it, like
/// a builtin one; one that RETURNS it hands back the caller's own argument, which
/// the caller's binding copies before its statement temp is dropped. Neither
/// callee frees anything it was lent.
#[test]
fn a_user_hof_borrows_its_callable_parameter() {
    fn program(name: &str, callee: &str, body: &str) -> Value {
        ncode(
            name,
            &format!(
                "IMPORT io\n\
                 {callee}\
                 FUNC main() AS Nothing\n\
                \x20 LET bump AS Integer = 5\n\
                 {body}\
                 END FUNC\n"
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
    assert!(drops(&invoke_only, "_mfb_fn_main") > 0);
    assert!(drops(&returns_it, "_mfb_fn_main") > 0);
    assert_eq!(
        copies(&invoke_only, "_mfb_fn_main"),
        0,
        "`applyTwice` only ever INVOKES `f`: the argument is lent and nothing is copied"
    );
    assert_eq!(
        copies(&returns_it, "_mfb_fn_main"),
        1,
        "`hold` returns the caller's own argument, so `g` takes a copy — without it \
         the statement's drop would leave `g` dangling"
    );
    for (plan, callee) in [
        (&invoke_only, "_mfb_fn_applyTwice"),
        (&returns_it, "_mfb_fn_hold"),
    ] {
        assert_eq!(
            drops(plan, callee) + copies(plan, callee),
            0,
            "{callee} neither frees nor copies the closure it was lent"
        );
    }
}

/// A nested call whose own argument is a capturing lambda: each call's capturing
/// argument is its statement's temp, borrowed by the call and never copied.
#[test]
fn nested_hof_calls_borrow_their_closures() {
    let nested = ncode(
        "b572_nested",
        "IMPORT io\n\
         IMPORT collections\n\
         FUNC main() AS Nothing\n\
        \x20 LET cap AS String = \"CAPTURED\"\n\
        \x20 LET xs AS List OF String = [\"a\", \"bb\", \"ccc\"]\n\
        \x20 LET c AS List OF String = collections::filter(collections::filter(xs, LAMBDA(s AS String) -> len(s) < len(cap)), LAMBDA(s AS String) -> len(s) > 1)\n\
        \x20 io::print(toString(len(c)) & cap)\n\
         END FUNC\n",
    );
    // The inner lambda captures `cap`; the outer one does not, so only the inner
    // allocates an environment.
    assert_eq!(closure_envs(&nested, "_mfb_fn_main"), 1);
    assert!(drops(&nested, "_mfb_fn_main") > 0);
    assert_eq!(
        copies(&nested, "_mfb_fn_main"),
        0,
        "both calls borrow their callbacks"
    );
}
