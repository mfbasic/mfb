//! bug-573: who owns the `ErrorLoc` a raised error's park was handed, asserted on
//! the emitted code.
//!
//! `_mfb_make_error_result` allocates an `ErrorLoc` — filename, line, column, with
//! the filename block INLINED — and `_mfb_rt_park_error` builds the single owned
//! flat `Error` block by inlining a COPY of it. The original then had no owner on
//! any path: ~200 B per raised error for `src/main.mfb` and ~682 B for a
//! 117-character source path, because the cost is the filename's length.
//!
//! The RSS cases in `tests/runtime/rt_scope_drop_leaks.rs` prove the leak is gone.
//! They cannot prove the *shape* of the fix, and the shape is what keeps it sound:
//!
//! * **No free** is the leak, and nothing goes red.
//! * **An unguarded free** frees a null: an error with no origin (every `LINK`
//!   thunk returns one, and so does an `ErrorLoc` allocation that itself hit OOM)
//!   arrives with `x3 == 0`.
//! * **Leaving `x3` dangling** is a use-after-free the moment anything downstream
//!   reads the loose source register. Nothing does today — every consumer of an
//!   `ERR_BLOCK` error ADOPTS the parked block and reads the origin from inside it
//!   — but the fix does not rest on that: it re-points `x3` at the parked block's
//!   own inlined copy.
//!
//! So this file asserts all three: exactly one guarded free in
//! `_mfb_rt_park_error`, the guard on the source pointer, and the re-point.
//!
//! Build-only `-ncode` cross-built for `linux-x86_64`, matching the sibling
//! codegen-inspection suites: ownership is target-independent codegen.

#[path = "../common/mod.rs"]
mod common;

use serde_json::Value;

const TARGET: &str = "linux-x86_64";

/// The park's own scratch labels. `_scratch_kept_` (bug-574) is a DIFFERENT
/// prefix and must not be confused with these.
const SOURCE_KEPT: &str = "park_error_source_kept";
const INTERIOR_NULL: &str = "park_error_source_interior_null";
const INTERIOR_DONE: &str = "park_error_source_interior_done";

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

fn instructions<'a>(plan: &'a Value, symbol: &str) -> &'a Vec<Value> {
    function(plan, symbol)["instructions"]
        .as_array()
        .expect("instructions array")
}

fn labels(plan: &Value, symbol: &str, prefix: &str) -> usize {
    instructions(plan, symbol)
        .iter()
        .filter(|i| {
            i["op"].as_str() == Some("label")
                && i["name"].as_str().is_some_and(|n| n.starts_with(prefix))
        })
        .count()
}

fn calls(plan: &Value, symbol: &str, target: &str) -> usize {
    instructions(plan, symbol)
        .iter()
        .filter(|i| i["op"].as_str() == Some("bl") && i["target"].as_str() == Some(target))
        .count()
}

/// A program with one raising call under an inline `TRAP` — the report's own
/// reproduction, which is also the smallest program that emits `_mfb_rt_park_error`.
const RAISES: &str = "IMPORT io\n\
IMPORT collections\n\
SUB main()\n\
\x20 LET xs AS List OF String = [\"aa\", \"bb\"]\n\
\x20 LET g AS String = collections::get(xs, 9) TRAP(e)\n\
\x20   RECOVER \"zz\"\n\
\x20 END TRAP\n\
\x20 io::print(g)\n\
END SUB\n";

/// The park is ONE synthesized function, so the free is emitted exactly once
/// however many sites call it — which is the whole reason it went in there rather
/// than at each of the three raise sites.
#[test]
fn the_park_frees_the_error_loc_exactly_once() {
    let plan = ncode("b573_park_free", RAISES);
    assert_eq!(
        labels(&plan, "_mfb_rt_park_error", SOURCE_KEPT),
        1,
        "`_mfb_rt_park_error` must carry exactly one guarded source release"
    );
    assert_eq!(
        calls(&plan, "_mfb_rt_park_error", "_mfb_arena_free"),
        1,
        "the guarded release must be a real `_mfb_arena_free`, and the only one \
         the park emits — a second would be freeing the parked block itself"
    );
    // The park still builds exactly one block: the owned flat `Error`. A change
    // that made the free cost an extra allocation would be a different fix.
    assert_eq!(
        calls(&plan, "_mfb_rt_park_error", "_mfb_arena_alloc"),
        1,
        "the park allocates only the `Error` block it parks"
    );
}

/// The guard, and the re-point, in the order they must appear: the null test and
/// its branch BEFORE the free, and the interior-pointer computation AFTER it.
///
/// A refactor that dropped the null test would free a pointer that an
/// origin-less error (`x3 == 0`) never allocated; one that dropped the re-point
/// would leave `RESULT_ERROR_SOURCE_REGISTER` pointing at memory the arena has
/// just reclaimed.
#[test]
fn the_release_is_guarded_and_leaves_a_live_source_pointer() {
    let plan = ncode("b573_park_shape", RAISES);
    let ins = instructions(&plan, "_mfb_rt_park_error");
    let index_of = |prefix: &str| {
        ins.iter()
            .position(|i| {
                i["op"].as_str() == Some("label")
                    && i["name"].as_str().is_some_and(|n| n.starts_with(prefix))
            })
            .unwrap_or_else(|| panic!("`_mfb_rt_park_error` has no `{prefix}` label"))
    };
    let kept = index_of(SOURCE_KEPT);
    let interior_null = index_of(INTERIOR_NULL);
    let interior_done = index_of(INTERIOR_DONE);
    assert!(
        kept < interior_null && interior_null < interior_done,
        "the release must run before the re-point, and the re-point's null arm \
         before its join"
    );

    let free = ins
        .iter()
        .position(|i| {
            i["op"].as_str() == Some("bl") && i["target"].as_str() == Some("_mfb_arena_free")
        })
        .expect("the park must free the ErrorLoc");
    assert!(
        free < kept,
        "the free must sit inside the guard, before its `{SOURCE_KEPT}` label"
    );
    // The guard is the branch that targets the release's own label — located by
    // that target rather than by adjacency, because the `ErrorLoc` sizing between
    // the two emits its own `record_size_field_absent` branch (the offset-0
    // sentinel walk, bug-371).
    let guard = ins
        .iter()
        .position(|i| {
            i["op"].as_str() == Some("b.eq")
                && i["target"]
                    .as_str()
                    .is_some_and(|t| t.starts_with(SOURCE_KEPT))
        })
        .expect("the free must be guarded by a branch to its own kept label");
    assert!(guard < free, "the guard must precede the free it skips");
    assert_eq!(
        ins[guard - 1]["op"].as_str(),
        Some("cmp_imm"),
        "the guard must be preceded by a compare"
    );
    assert_eq!(
        ins[guard - 1]["rhs"].as_str(),
        Some("0"),
        "the guard must compare the `ErrorLoc` pointer against 0 — an error with \
         no origin (every `LINK` thunk, and an OOM-degraded raise) arrives with a \
         null source"
    );
}

/// The park is the funnel every raise goes through, so the free must not depend
/// on which of the three park sites raised. All three shapes reach the SAME
/// `_mfb_rt_park_error`, and the assertion is that its body is identical in each
/// program — the enumeration is a compile-time one
/// (`ParkedErrorSource`, matched exhaustively with no wildcard), and this is its
/// emitted-code counterpart.
#[test]
fn every_raise_shape_reaches_the_same_park_body() {
    let builtin = ncode("b573_site_builtin", RAISES);
    let helper = ncode(
        "b573_site_helper",
        "IMPORT io\n\
         IMPORT fs\n\
         SUB main()\n\
        \x20 LET s AS String = fs::readText(\"/tmp/b573_absent.txt\") TRAP(e)\n\
        \x20   RECOVER \"x\"\n\
        \x20 END TRAP\n\
        \x20 io::print(s)\n\
         END SUB\n",
    );
    let user_fail = ncode(
        "b573_site_fail",
        "IMPORT io\n\
         FUNC boom() AS Integer\n\
        \x20 FAIL error(90000001, \"boom\")\n\
         END FUNC\n\
         SUB main()\n\
        \x20 LET v AS Integer = boom() TRAP(e)\n\
        \x20   RECOVER e.code\n\
        \x20 END TRAP\n\
        \x20 io::print(toString(v))\n\
         END SUB\n",
    );
    for (name, plan) in [
        ("builtin", &builtin),
        ("helper", &helper),
        ("user FAIL", &user_fail),
    ] {
        assert_eq!(
            labels(plan, "_mfb_rt_park_error", SOURCE_KEPT),
            1,
            "the {name} raise shape must reach a park that releases its ErrorLoc"
        );
        assert_eq!(
            calls(plan, "_mfb_rt_park_error", "_mfb_arena_free"),
            1,
            "the {name} raise shape's park must free exactly once"
        );
    }
    assert_eq!(
        function(&builtin, "_mfb_rt_park_error")["instructions"],
        function(&helper, "_mfb_rt_park_error")["instructions"],
        "the park is one synthesized function; its body cannot vary with the \
         program that calls it"
    );
    assert_eq!(
        function(&builtin, "_mfb_rt_park_error")["instructions"],
        function(&user_fail, "_mfb_rt_park_error")["instructions"],
        "the park is one synthesized function; its body cannot vary with the \
         program that calls it"
    );
}

/// The POSITIVE pin: the release is confined to `_mfb_rt_park_error`.
///
/// The change only ever adds code inside that one synthesized function. If a
/// `park_error_source_*` label or an extra `_mfb_arena_free` ever appeared in
/// another function — the user body, a runtime helper, `_mfb_build_error_loc` —
/// something is freeing an `ErrorLoc` on a path that did not park one, and the
/// park's argument (its input is a block THIS FRAME just allocated) does not
/// cover it.
#[test]
fn nothing_but_the_park_releases_an_error_loc() {
    let plan = ncode("b573_confined", RAISES);
    for f in plan["functions"].as_array().expect("functions array") {
        let symbol = f["symbol"].as_str().expect("symbol");
        if symbol == "_mfb_rt_park_error" {
            continue;
        }
        let strays = f["instructions"]
            .as_array()
            .expect("instructions array")
            .iter()
            .filter(|i| {
                i["op"].as_str() == Some("label")
                    && i["name"]
                        .as_str()
                        .is_some_and(|n| n.starts_with("park_error_source"))
            })
            .count();
        assert_eq!(
            strays, 0,
            "{symbol} carries a park source release; only `_mfb_rt_park_error` may"
        );
    }
}
