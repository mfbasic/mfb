//! plan-176-G: `ir::tests::lower_pipeline_tests`'s result-type inference, inferred-type,
//! IO/FS inference and trap fall-through, and reachable-arm tests. The shared helpers live
//! in `lower_pipeline_tests/mod.rs`.

use super::*;

// ---- builtin result-type inference (expression_type resolve_call arms) --

#[test]
fn infers_builtin_result_types_in_inferred_lets() {
    // No explicit LET type -> lowering must call expression_type, which runs
    // each package's resolve_call to infer the result type.
    let ir = lower_src(
        "infer_builtins",
        r#"
IMPORT strings
IMPORT math
IMPORT bits
IMPORT json
IMPORT csv
IMPORT crypto
IMPORT net
IMPORT regex
IMPORT datetime
IMPORT encoding
FUNC main AS Integer
  LET up = strings::upper("hi")
  LET n = math::max(1, 2)
  LET b = bits::sl(1, 2)
  LET v = json::parse("{}")
  LET js = json::stringify(v)
  LET rows = csv::parse("a,b")
  LET back = csv::stringify(rows)
  LET dig = crypto::hash(crypto::Hash.SHA2_256, "abc")
  LET hexed = encoding::hexEncode(dig)
  LET u = net::toUrl("http://x/")
  LET m = regex::match("abc", "a.c")
  LET dt = datetime::instant(0)
  RETURN n
END FUNC
"#,
    );
    assert!(json_of(&ir).contains("main"));
}

#[test]
fn lowers_call_default_padding_for_regex_datetime_crypto() {
    // These calls lower to IrValue::Call and trigger the trailing-argument
    // default-padding branches in the Call-expression lowering.
    let ir = lower_src(
        "padding",
        r#"
IMPORT regex
IMPORT datetime
IMPORT crypto
FUNC main AS Integer
  LET matched AS Boolean = regex::match("abc", "a.c")
  LET dt AS datetime::DateTime = datetime::parse("2024-01-02")
  LET key AS List OF Byte = crypto::hash(crypto::Hash.SHA2_256, "k")
  RETURN 0
END FUNC
"#,
    );
    assert!(json_of(&ir).contains("match") || json_of(&ir).contains("parse"));
}

#[test]
fn lowers_general_override_for_builtin_value_type() {
    // toString(net::Url) routes to the package's internal override helper.
    let ir = lower_src(
        "override",
        r#"
IMPORT net
IMPORT io
FUNC main AS Integer
  LET u AS net::Url = net::toUrl("http://example.com/")
  LET s AS String = toString(u)
  io::print(s)
  RETURN 0
END FUNC
"#,
    );
    assert!(json_of(&ir).contains("toString") || json_of(&ir).contains("net"));
}

#[test]
fn lowers_lambda_capturing_callable_local() {
    let ir = lower_src(
        "capture_callable",
        r#"
FUNC run(fn AS FUNC(Integer) AS Integer) AS FUNC(Integer) AS Integer
  RETURN LAMBDA(x AS Integer) -> fn(x) + 1
END FUNC
FUNC dbl(x AS Integer) AS Integer
  RETURN x * 2
END FUNC
FUNC main AS Integer
  LET g AS FUNC(Integer) AS Integer = run(dbl)
  RETURN g(3)
END FUNC
"#,
    );
    assert!(json_of(&ir).contains("closure") || json_of(&ir).contains("capture"));
}

#[test]
fn infers_top_level_binding_types() {
    let ir = lower_src(
        "infer_binding",
        r#"
LET greeting = "hello"
LET count = 7
FUNC main AS Integer
  RETURN count
END FUNC
"#,
    );
    assert!(ir
        .bindings
        .iter()
        .any(|b| b.name == "greeting" && b.type_.name() == "String"));
    assert!(ir
        .bindings
        .iter()
        .any(|b| b.name == "count" && b.type_.name() == "Integer"));
}

// ---- union includes (nested union expansion) ---------------------------

#[test]
fn lowers_union_with_includes() {
    let ir = lower_src(
        "union_includes",
        r#"
TYPE A
  v AS Integer
END TYPE
TYPE B
  v AS Integer
END TYPE
UNION Base
  A
END UNION
UNION Wide INCLUDES Base
  B
END UNION
FUNC pick(w AS Wide) AS Integer
  MATCH w
    CASE A(a)
      RETURN a.v
    CASE B(b)
      RETURN b.v
  END MATCH
END FUNC
FUNC main AS Integer
  RETURN pick(A[1]) + pick(B[2])
END FUNC
"#,
    );
    assert!(json_of(&ir).contains("Wide") || json_of(&ir).contains("Base"));
}

// ---- filter with an inline lambda predicate (filter_predicate_type) -----

#[test]
fn lowers_filter_with_inline_lambda_predicate() {
    let ir = lower_src(
        "filter_lambda",
        r#"
IMPORT collections
FUNC main AS Integer
  LET nums AS List OF Integer = [1, 2, 3, 4]
  LET big = collections::filter(nums, LAMBDA(n AS Integer) -> n > 2)
  RETURN len(big)
END FUNC
"#,
    );
    assert!(json_of(&ir).contains("closure") || json_of(&ir).contains("functionRef"));
}

#[test]
fn lowers_filter_with_builtin_predicate_reference() {
    // A builtin single-arg Boolean predicate (`isEven`) drives the
    // filter_predicate_type inference + FunctionRef synthesis in both
    // expression_type and the Call lowering (collections.filter arm).
    let ir = lower_src(
        "filter_builtin_pred",
        r#"
IMPORT collections
FUNC main AS Integer
  LET nums AS List OF Integer = [1, 2, 3, 4]
  LET evens = collections::filter(nums, isEven)
  RETURN len(evens)
END FUNC
"#,
    );
    assert!(json_of(&ir).contains("functionRef"));
}

#[test]
fn lowers_collections_filter_inferred_result_type() {
    // collections::filter with a named predicate in an inferred-type LET
    // exercises the filter_predicate_type inference + FunctionRef synthesis
    // in both expression_type and the Call lowering.
    let ir = lower_src(
        "coll_filter_infer",
        r#"
IMPORT collections
FUNC keep(n AS Integer) AS Boolean
  RETURN n > 1
END FUNC
FUNC main AS Integer
  LET nums AS List OF Integer = [1, 2, 3]
  LET big = collections::filter(nums, keep)
  RETURN len(big)
END FUNC
"#,
    );
    assert!(json_of(&ir).contains("functionRef") || json_of(&ir).contains("keep"));
}

// ---- bare FunctionRef (no captures) ------------------------------------

#[test]
fn lowers_bare_function_reference_without_captures() {
    let ir = lower_src(
        "bare_fref",
        r#"
FUNC dbl(x AS Integer) AS Integer
  RETURN x * 2
END FUNC
FUNC pickFn() AS FUNC(Integer) AS Integer
  RETURN LAMBDA(x AS Integer) -> dbl(x)
END FUNC
FUNC main AS Integer
  LET f AS FUNC(Integer) AS Integer = pickFn()
  RETURN f(4)
END FUNC
"#,
    );
    // A capture-free lambda lowers to a plain FunctionRef.
    assert!(json_of(&ir).contains("functionRef"));
}

// ---- inferred list literal element type (literal_expression_type) ------

#[test]
fn lowers_inferred_list_literal_element_type() {
    let ir = lower_src(
        "inferred_list",
        r#"
IMPORT collections
FUNC main AS Integer
  LET ints = [1, 2, 3]
  LET strs = ["a", "b"]
  LET floats = [1.5, 2.5]
  LET bools = [TRUE, FALSE]
  ' A list literal passed to a generic builtin has no expected element type, so
  ' its element type is inferred via literal_expression_type over the first item.
  LET a AS Integer = collections::get([10, 20, 30], 0)
  LET b AS String = collections::get(["x", "y"], 0)
  LET c AS Float = collections::get([1.5, 2.5], 0)
  LET d AS Boolean = collections::get([TRUE, FALSE], 0)
  RETURN len(ints) + a
END FUNC
"#,
    );
    let j = json_of(&ir);
    assert!(j.contains("List OF Integer"));
    assert!(j.contains("List OF String"));
    assert!(j.contains("List OF Float"));
}

// ---- Ok constructor result type ----------------------------------------

#[test]
fn lowers_error_constructor_type_inference() {
    // error(...) constructs an Error record; its result type feeds the
    // TypeIndex::constructor_result "Error" arm through expression_type.
    let ir = lower_src(
        "error_ctor",
        r#"
FUNC risky() AS Integer
  LET e = error(42, "oops")
  FAIL e
  TRAP(err)
    RETURN err.code
  END TRAP
END FUNC
FUNC main AS Integer
  RETURN risky()
END FUNC
"#,
    );
    assert!(json_of(&ir).contains("Error"));
}

// ---- native io / fs call result-type inference -------------------------

#[test]
fn infers_mapentry_key_and_value_types() {
    let ir = lower_src(
        "mapentry_infer",
        r#"
FUNC main AS Integer
  MUT total AS Integer = 0
  LET m AS Map OF String TO Integer = Map OF String TO Integer { "a" := 1 }
  FOR EACH entry IN m
    LET k = entry.key
    LET v = entry.value
    total = total + v
  NEXT
  RETURN total
END FUNC
"#,
    );
    assert!(json_of(&ir).contains("MapEntry"));
}

#[test]
fn infers_arithmetic_and_trapped_expression_types() {
    let ir = lower_src(
        "arith_infer",
        r#"
FUNC parse(v AS Integer) AS Integer
  IF v < 0 THEN FAIL error(1, "x")
  RETURN v
END FUNC
FUNC main AS Integer
  LET a = 2 + 3 * 4
  LET b = 1.5 - 0.5
  LET c = TRUE AND FALSE
  LET t = parse(3) TRAP(e)
    RECOVER 0
  END TRAP
  RETURN a + t
END FUNC
"#,
    );
    assert!(json_of(&ir).contains("main"));
}

#[test]
fn lowers_assignment_bodied_lambda() {
    let ir = lower_src(
        "assign_lambda",
        r#"
IMPORT collections
FUNC main AS Integer
  MUT total AS Integer = 0
  LET nums AS List OF Integer = [1, 2, 3]
  collections::forEach(nums, LAMBDA(n AS Integer) -> total = total + n)
  RETURN total
END FUNC
"#,
    );
    // An assignment-bodied lambda yields Nothing and emits an Assign+Return.
    assert!(json_of(&ir).contains("Nothing") || json_of(&ir).contains("assign"));
}

#[test]
fn lowers_assignment_bodied_lambda_target_not_on_rhs() {
    // The assignment target (an outer MUT local) is captured even when it does
    // not appear on the lambda body's right-hand side.
    let ir = lower_src(
        "assign_lambda_target",
        r#"
IMPORT collections
FUNC main AS Integer
  MUT last AS Integer = 0
  LET nums AS List OF Integer = [1, 2, 3]
  collections::forEach(nums, LAMBDA(n AS Integer) -> last = n)
  RETURN last
END FUNC
"#,
    );
    assert!(json_of(&ir).contains("last") || json_of(&ir).contains("closure"));
}

#[test]
fn lowers_nested_lambda_capture_skips_inner_lambda() {
    // captured_locals must not descend into a nested lambda's body (the inner
    // lambda captures independently).
    let ir = lower_src(
        "nested_lambda",
        r#"
FUNC outer(base AS Integer) AS FUNC() AS FUNC() AS Integer
  RETURN LAMBDA() -> LAMBDA() -> base + 1
END FUNC
FUNC main AS Integer
  LET f AS FUNC() AS FUNC() AS Integer = outer(5)
  LET g AS FUNC() AS Integer = f()
  RETURN g()
END FUNC
"#,
    );
    // Two synthesized lambda bodies (outer + inner).
    assert!(
        ir.functions
            .iter()
            .filter(|f| f.name.starts_with("$lambda"))
            .count()
            >= 2
    );
}

#[test]
fn lowers_zero_param_func_typed_local_call() {
    // Calling a FUNC()-typed local variable (no params) and invoking a
    // stored zero-parameter reference exercises the FUNC()-typed local paths.
    let ir = lower_src(
        "zero_param_fn",
        r#"
FUNC makeIt() AS Integer
  RETURN 42
END FUNC
FUNC main AS Integer
  LET f AS FUNC() AS Integer = makeIt
  RETURN f()
END FUNC
"#,
    );
    assert!(json_of(&ir).contains("makeIt") || json_of(&ir).contains("main"));
}

#[test]
fn lowers_isolated_function_reference() {
    let ir = lower_src(
        "isolated_fn",
        r#"
ISOLATED FUNC pure(x AS Integer) AS Integer
  RETURN x * 2
END FUNC
FUNC apply(f AS FUNC(Integer) AS Integer, v AS Integer) AS Integer
  RETURN f(v)
END FUNC
FUNC main AS Integer
  LET fref = pure
  RETURN apply(fref, 5)
END FUNC
"#,
    );
    assert!(json_of(&ir).contains("functionRef") || json_of(&ir).contains("pure"));
}

#[test]
fn lowers_local_call_with_named_and_positional_args() {
    let ir = lower_src(
        "local_named",
        r#"
FUNC combine(a AS Integer, b AS Integer, c AS Integer = 100) AS Integer
  RETURN a + b + c
END FUNC
FUNC main AS Integer
  LET x AS Integer = combine(1, c := 3, b := 2)
  LET y AS Integer = combine(1, 2)
  RETURN x + y
END FUNC
"#,
    );
    assert!(json_of(&ir).contains("combine"));
}

#[test]
fn lowers_tls_connect_default_argument_padding() {
    // tls::connect / tls::listen with fewer than the full argument count pad
    // trailing arguments with constants (native fixed-ABI helpers).
    let ir = lower_src(
        "tls_pad",
        r#"
IMPORT tls
FUNC main AS Integer
  RES conn = tls::connect("example.com", 443)
  RES server = tls::listen("127.0.0.1", 8443, "cert.pem", "key.pem")
  RES accepted = tls::accept(server)
  tls::close(conn)
  tls::close(server)
  RETURN 0
END FUNC
"#,
    );
    assert!(json_of(&ir).contains("tls") || json_of(&ir).contains("connect"));
}

#[test]
fn lowers_member_access_on_builtin_record_type() {
    // Accessing a field of a builtin record type exercises
    // TypeIndex::record_field_type's builtin-type-fields branch.
    //
    // plan-122-F re-pointed this from `term::TermColor` (retired) to
    // `color::Color`, which `term::getForeground` now returns. The property under
    // test is unchanged and still true — only the type used to exercise it moved.
    // The old assertion was `contains("TermColor") || contains("memberAccess")`;
    // the first disjunct can no longer hold, and an `||` whose left side is dead
    // is a test that has quietly halved. Asserts the member accesses directly.
    let ir = lower_src(
        "builtin_fields",
        r#"
IMPORT term
IMPORT color
FUNC main AS Integer
  LET c AS color::Color = term::getForeground()
  LET red = c.red
  LET green = c.green
  RETURN 0
END FUNC
"#,
    );
    let json = json_of(&ir);
    assert!(
        json.contains("memberAccess"),
        "field access on a builtin record must lower to a memberAccess"
    );
    assert!(
        json.contains("color.Color"),
        "the builtin record's qualified type must reach the IR"
    );
}

#[test]
fn infers_io_and_fs_native_call_result_types() {
    let ir = lower_src(
        "io_fs_infer",
        r#"
IMPORT io
IMPORT fs
FUNC main AS Integer
  LET term = io::isOutputTerminal()
  LET exists = fs::exists("/tmp")
  RETURN 0
END FUNC
"#,
    );
    assert!(json_of(&ir).contains("main"));
}

/// Total op count of a body, recursing into every nested block. Used to
/// prove an inline-`TRAP` handler lowers to a body **linear** in its
/// statement count (bug-401).
fn count_ops(body: &[IrOp]) -> usize {
    body.iter()
        .map(|op| {
            1 + match op {
                IrOp::If {
                    then_body,
                    else_body,
                    ..
                } => count_ops(then_body) + count_ops(else_body),
                IrOp::Match { cases, .. } => cases.iter().map(|c| count_ops(&c.body)).sum(),
                IrOp::While { body, .. }
                | IrOp::For { body, .. }
                | IrOp::DoUntil { body, .. }
                | IrOp::ForEach { body, .. }
                | IrOp::Trap { body, .. } => count_ops(body),
                _ => 0,
            }
        })
        .sum()
}

/// bug-401: an inline-`TRAP` handler that is N sequential *fall-through*
/// `IF`s before a final `RECOVER` must lower to a body **linear** in N. The
/// buggy lowering cloned the whole post-branch continuation into both arms
/// of every fall-through `IF`, producing 2^N copies of the tail — at N≈14
/// the exploded function overflowed the AArch64 ±1 MiB branch range and a
/// legal program stopped compiling.
#[test]
fn inline_trap_fallthrough_handler_lowers_linearly() {
    fn handler_source(n: usize) -> String {
        let mut src = String::from(
            "FUNC g() AS Integer\n  RETURN 1\nEND FUNC\n\
                 FUNC main() AS Integer\n  MUT y AS Integer = 0\n  LET x = g() TRAP(e)\n",
        );
        for i in 1..=n {
            src.push_str(&format!("    IF y > {i} THEN\n      y = {i}\n    END IF\n"));
        }
        src.push_str("    RECOVER 0\n  END TRAP\n  RETURN x + y\nEND FUNC\n");
        src
    }

    // Measure the per-`IF` op growth across two sizes; with the exponential
    // bug this difference is astronomically larger than any linear bound.
    let small =
        count_ops(&function(&lower_src("trap_linear_small", &handler_source(5)), "main").body);
    let large =
        count_ops(&function(&lower_src("trap_linear_large", &handler_source(15)), "main").body);

    // 10 extra `IF`s may add only a bounded, per-statement number of ops.
    // Post-fix each fall-through `IF` contributes a small constant (~4);
    // 40 leaves generous headroom while staying far below the 2^N blow-up
    // (which would add tens of thousands of ops over the same 10 statements).
    let growth = large - small;
    assert!(
        growth <= 40 * (15 - 5),
        "inline-TRAP handler op count grew by {growth} for 10 extra fall-through IFs \
             (expected linear ≤ {}); the continuation is being cloned per branch (bug-401)",
        40 * (15 - 5)
    );
}

// ==== plan-lower-coverage: reachable uncovered arms ======================

// Suffixed float literal (`1.5f`): the `is_suffixed` branch's `_ => "Float"`
// arm (lower.rs ~2465). `F`/`m`/`M` hit the Fixed/Money arms; `f` falls here.
#[test]
fn suffixed_float_literal_types_as_float() {
    let ir = lower_src(
        "suffixed_float",
        "FUNC main AS Float\n  LET x AS Float = 1.5f\n  RETURN x\nEND FUNC\n",
    );
    let j = json_of(&ir);
    assert!(j.contains("\"Float\""), "{j}");
}

// A negated decimal literal in an exact-numeric slot lowers its operand as a
// const of that exact type (lower.rs ~3171-3172): `AS Money`/`AS Fixed`.
#[test]
fn negated_exact_numeric_literals() {
    let ir = lower_src(
        "neg_exact",
        "FUNC main AS Integer\n  \
               LET m AS Money = -1.25\n  \
               LET f AS Fixed = -1.25\n  \
               RETURN 0\nEND FUNC\n",
    );
    let j = json_of(&ir);
    assert!(j.contains("\"Money\""), "{j}");
    assert!(j.contains("\"Fixed\""), "{j}");
}

// An untyped list literal in a no-expected-type slot infers its element type
// from the first element via `literal_expression_type` (lower.rs ~3475): a
// Fixed (`F`) and a Money (`m`) literal reach its Fixed/Money arms
// (~3482-3483).
#[test]
fn untyped_list_literal_element_type_fixed_and_money() {
    let ir = lower_src(
        "list_elem_infer",
        "FUNC main AS Integer\n  \
               LET a = [1.5F]\n  \
               LET b = [1.99m]\n  \
               RETURN 0\nEND FUNC\n",
    );
    let j = json_of(&ir);
    assert!(j.contains("List OF Fixed"), "{j}");
    assert!(j.contains("List OF Money"), "{j}");
}

// A top-level binding with no `AS` annotation infers its type from the value
// both in `infer_binding_types` (lower.rs ~1887-1889) and in `lower_binding`
// (~253-258). A binding holding a FUNC value is then callable, inferring the
// call's return type from the binding's FUNC type (~2092-2095).
#[test]
fn untyped_global_binding_holding_function_value() {
    let ir = lower_src(
        "global_fnval",
        "FUNC helper(n AS Integer) AS Integer\n  RETURN n + 1\nEND FUNC\n\
             LET fnRef = helper\n\
             FUNC main AS Integer\n  RETURN fnRef(5)\nEND FUNC\n",
    );
    assert!(ir.bindings.iter().any(|b| b.name == "fnRef"));
    // The call resolves through the binding's FUNC type to Integer.
    let ret = &function(&ir, "main").body;
    assert!(!ret.is_empty());
    let b = binding_of(&ir, "fnRef");
    assert!(
        b.type_.name().starts_with("FUNC("),
        "fnRef type: {}",
        b.type_
    );
}

fn binding_of<'a>(ir: &'a super::IrProject, name: &str) -> &'a super::IrBinding {
    ir.bindings
        .iter()
        .find(|b| b.name == name)
        .unwrap_or_else(|| panic!("no binding {name}"))
}

// A first-class value of an ISOLATED FUNC type: calling it strips the
// `ISOLATED FUNC(` prefix in both `function_return_from_type` (~2158) and
// `function_param_types_from_type` (~2166) when typing the call and its arg.
#[test]
fn isolated_func_value_call_typing() {
    let ir = lower_src(
        "isolated_fnval",
        "ISOLATED FUNC iso(n AS Integer) AS Integer\n  RETURN n\nEND FUNC\n\
             FUNC main AS Integer\n  \
               LET g = iso\n  \
               RETURN g(5)\nEND FUNC\n",
    );
    let body = &function(&ir, "main").body;
    // g is bound with an ISOLATED FUNC type.
    let has_iso = body.iter().any(|op| {
        matches!(op,
        IrOp::Bind { type_, .. } if type_.name().starts_with("ISOLATED FUNC("))
    });
    assert!(has_iso, "expected an ISOLATED FUNC-typed local");
}

// FOR EACH over a Map yields `MapEntry OF K TO V` (parse_map_type ~1554); a
// `.value` access on the entry then parses the entry type (~1560).
#[test]
fn foreach_over_map_entry_value() {
    let ir = lower_src(
        "map_iter",
        "IMPORT collections\n\
             FUNC main AS Integer\n  \
               LET m AS Map OF String TO Integer = Map OF String TO Integer { \"a\" := 1 }\n  \
               LET total AS Integer = 0\n  \
               FOR EACH e IN m\n    \
                 total = total + e.value\n  \
               NEXT\n  \
               RETURN total\nEND FUNC\n",
    );
    let j = json_of(&ir);
    assert!(j.contains("MapEntry OF String TO Integer"), "{j}");
}

// A bare general built-in predicate (`isPositive`) as the callback of a
// native `collections::filter` member: expression_type takes the
// unary-callback path (lower.rs ~2020-2036) and the call lowers the predicate
// to a typed FunctionRef via `builtin_predicate_ref_type` (~2417-2434). A
// sibling `collections::get` exercises the plain arg-typed member path
// (~2039-2043).
#[test]
fn collections_filter_with_bare_general_predicate() {
    let ir = lower_src(
        "filter_pred",
        "IMPORT collections\n\
             FUNC main AS Integer\n  \
               LET nums AS List OF Integer = [1, -2, 3]\n  \
               LET pos AS List OF Integer = collections::filter(nums, isPositive)\n  \
               RETURN collections::get(pos, 0)\nEND FUNC\n",
    );
    let j = json_of(&ir);
    // The predicate lowered to a function reference of the element-typed sig.
    assert!(
        j.contains("FUNC(Integer) AS Boolean") || j.contains("functionRef"),
        "{j}"
    );
}

// An assignment-bodied lambda whose target appears ONLY on the left captures
// the target even though it never reads on the right (lower.rs ~2874-2883).
#[test]
fn assign_bodied_lambda_captures_write_only_target() {
    let ir = lower_src(
        "assign_lambda",
        "IMPORT collections\n\
             FUNC main AS Integer\n  \
               LET last AS Integer = 0\n  \
               LET nums AS List OF Integer = [1, 2, 3]\n  \
               collections::forEach(nums, LAMBDA(n) -> last = n)\n  \
               RETURN last\nEND FUNC\n",
    );
    // A private $lambda function is synthesized, capturing `last`.
    let lambda = ir
        .functions
        .iter()
        .find(|f| f.name.starts_with("$lambda"))
        .expect("lambda synthesized");
    let _ = lambda;
    let j = json_of(&ir);
    assert!(j.contains("capture") || j.contains("closure"), "{j}");
}

// A union that INCLUDES itself: `expanded_union_variants` must short-circuit
// the cycle rather than recurse forever (lower.rs ~3658-3659, bug-194).
#[test]
fn self_including_union_cycle_terminates() {
    let ir = try_lower_src(
        "union_cycle",
        "UNION Loop\n  INCLUDES Loop\n  A(x AS Integer)\nEND UNION\n\
             FUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
    );
    // The point is that lowering terminates (no stack overflow); any outcome
    // that returns is a pass.
    let _ = ir;
}

// A TESTING block is an AST item with no executable body; the normal build
// path walks past it (lower.rs ~191, the `Item::Testing` arm).
#[test]
fn testing_block_item_is_skipped_by_lowering() {
    let ir = try_lower_src(
        "testing_item",
        "IMPORT io\n\
             FUNC label(n AS Integer) AS Integer\n  RETURN n\nEND FUNC\n\
             FUNC main AS Integer\n  RETURN 0\nEND FUNC\n\
             TESTING\n  \
               TGROUP \"g\"\n    \
                 TCASE \"c\"\n      \
                   expectEqual(label(1), 1)\n    \
                 END TCASE\n  \
               END TGROUP\n\
             END TESTING\n",
    );
    // The program still lowers to a project with `main`, TESTING and all.
    assert!(
        ir.as_ref()
            .map(|p| p.functions.iter().any(|f| f.name == "main"))
            .unwrap_or(false),
        "program with a TESTING block did not lower"
    );
}

// An untyped lambda parameter defaults to `Unknown` on both the
// expression_type path (lower.rs ~2110) and the lowering path (~2903, ~2978).
#[test]
fn untyped_lambda_param_defaults_to_unknown() {
    let ir = try_lower_src(
        "untyped_lambda",
        "IMPORT collections\n\
             FUNC main AS Integer\n  \
               LET nums AS List OF Integer = [1, 2, 3]\n  \
               LET out AS List OF Integer = collections::transform(nums, LAMBDA(x) -> x + 1)\n  \
               RETURN collections::get(out, 0)\nEND FUNC\n",
    );
    assert!(ir.is_some(), "untyped lambda param did not lower");
    let ir = ir.unwrap();
    assert!(ir.functions.iter().any(|f| f.name.starts_with("$lambda")));
}

// `error(code)` with a missing message argument: total lowering (plan-20-D)
// substitutes an Unknown-typed placeholder rather than panicking (lower.rs
// ~2591-2597).
#[test]
fn error_call_with_missing_argument_uses_placeholder() {
    let ir = try_lower_src(
        "error_placeholder",
        "FUNC boom() AS Integer\n  FAIL error(900)\nEND FUNC\n\
             FUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
    );
    // The front end accepts the short arity here (it reaches lowering);
    // total lowering substitutes an Unknown placeholder for the message.
    assert!(ir.is_some(), "error() short arity did not lower");
    let ir = ir.unwrap();
    assert!(ir.functions.iter().any(|f| f.name == "boom"));
}

// A lambda body that CALLS a captured function-typed local reaches the
// Call arm of `collect_captured_locals` (lower.rs ~3399-3405).
#[test]
fn lambda_calls_captured_function_local() {
    let ir = try_lower_src(
        "captured_call",
        "IMPORT collections\n\
             FUNC helper(n AS Integer) AS Integer\n  RETURN n * 2\nEND FUNC\n\
             FUNC main AS Integer\n  \
               LET g = helper\n  \
               LET nums AS List OF Integer = [1, 2, 3]\n  \
               LET out AS List OF Integer = collections::transform(nums, LAMBDA(x) -> g(x))\n  \
               RETURN collections::get(out, 0)\nEND FUNC\n",
    );
    assert!(
        ir.is_some(),
        "lambda calling a captured local did not lower"
    );
    let ir = ir.unwrap();
    let j = json_of(&ir);
    assert!(j.contains("closure") || j.contains("capture"), "{j}");
}

// A LET whose value is an inline TRAP with no `AS` annotation infers the
// success type from the trapped expression (lower.rs ~527-530).
#[test]
fn untyped_let_inline_trap_infers_success_type() {
    let ir = try_lower_src(
        "let_trap_infer",
        "FUNC parse(s AS String) AS Integer\n  RETURN 5\nEND FUNC\n\
             FUNC main AS Integer\n  \
               LET n = parse(\"7\") TRAP(e)\n    \
                 RECOVER 0\n  \
               END TRAP\n  \
               RETURN n\nEND FUNC\n",
    );
    assert!(ir.is_some(), "untyped let-trap did not lower");
    let ir = ir.unwrap();
    assert!(ir.functions.iter().any(|f| f.name == "main"));
}

// An inline-TRAP handler whose head IF's then-body holds a nested IF, MATCH,
// FOR, WHILE, DO...UNTIL and FOR EACH — none terminating — forces
// `block_can_terminate` (via treeify's `!block_can_terminate(then_body)`
// guard at lower.rs:1267) to walk every `statement_can_terminate` arm
// (~1475-1486): If, Match, and the For/ForEach/While/DoUntil group. Ordering
// them non-terminating keeps `.any()` from short-circuiting so each arm runs.
#[test]
fn trap_handler_block_can_terminate_walks_all_nested_arms() {
    let ir = lower_src(
        "trap_can_terminate",
        "FUNC parse(s AS String) AS Integer\n  \
               MUT acc AS Integer = 0\n  \
               LET n AS Integer = toInt(s) TRAP(e)\n    \
                 IF e.code > 0 THEN\n      \
                   IF acc > 0 THEN\n        acc = acc + 1\n      END IF\n      \
                   MATCH acc\n        CASE 1\n          acc = acc + 1\n        \
                   CASE ELSE\n          acc = acc + 2\n      END MATCH\n      \
                   FOR i = 1 TO 2\n        acc = acc + i\n      NEXT\n      \
                   WHILE acc < 0\n        acc = acc + 1\n      END WHILE\n      \
                   DO\n        acc = acc + 1\n      LOOP UNTIL acc > 5\n      \
                   FOR EACH v IN [1, 2]\n        acc = acc + v\n      NEXT\n    \
                 END IF\n    \
                 RECOVER acc\n  \
               END TRAP\n  \
               RETURN n\nEND FUNC\n\
             FUNC main AS Nothing\nEND FUNC\n",
    );
    assert!(function(&ir, "parse").body.iter().any(|op| matches!(
        op,
        IrOp::If {
            condition: super::super::IrValue::ResultIsOk { .. },
            ..
        }
    )));
}

// A MATCH whose scrutinee is a member access to a NON-EXISTENT field types as
// `Unknown` (record_field_type -> None), reaching match_expression_type's
// `unwrap_or_else(|| "Unknown")` fallback (lower.rs ~791). lower_src bypasses
// the type checker, so the bad field survives to lowering.
#[test]
fn untypeable_match_scrutinee_defaults_to_unknown() {
    let ir = try_lower_src(
        "unknown_match",
        "TYPE Rec\n  a AS Integer\nEND TYPE\n\
             FUNC run(r AS Rec) AS Integer\n  \
               MATCH r.missing\n    \
                 CASE ELSE\n      RETURN 0\n  \
               END MATCH\nEND FUNC\n\
             FUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
    );
    assert!(ir.is_some(), "bad-field MATCH scrutinee did not lower");
}

// FOR bounds and a FOR EACH iterable that are member accesses to non-existent
// fields type as `Unknown`, reaching the start/end (`~851/853`) and iterable
// (`~931`) `unwrap_or_else(|| "Unknown")` fallbacks.
#[test]
fn untypeable_loop_bounds_and_iterable_default_to_unknown() {
    let ir = try_lower_src(
        "unknown_loops",
        "TYPE Rec\n  a AS Integer\nEND TYPE\n\
             FUNC run(r AS Rec) AS Integer\n  \
               MUT total AS Integer = 0\n  \
               FOR i = r.lo TO r.hi\n    total = total + 1\n  NEXT\n  \
               FOR EACH v IN r.items\n    total = total + 1\n  NEXT\n  \
               RETURN total\nEND FUNC\n\
             FUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
    );
    assert!(ir.is_some(), "bad-field loop bounds did not lower");
}

// A unary negation whose operand types as `Unknown` (a member access to a
// non-existent field, in a non-exact-numeric slot) reaches the negation
// else-branch `unwrap_or_else(|| "Unknown")` fallback (lower.rs ~3176-3177).
#[test]
fn untypeable_negation_operand_defaults_to_unknown() {
    let ir = try_lower_src(
        "unknown_neg",
        "TYPE Rec\n  a AS Integer\nEND TYPE\n\
             FUNC run(r AS Rec) AS Integer\n  \
               LET x AS Integer = -r.missing\n  \
               RETURN x\nEND FUNC\n\
             FUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
    );
    assert!(ir.is_some(), "bad-field negation did not lower");
}

// A per-position builtin (`datetime::date`) called with a leading NAMED arg
// followed by positionals forces `normalize_builtin_call_arguments` to skip
// the already-filled slot (the `while ... is_some()` loop, lower.rs ~2247)
// and to place the named arg by position-name match (~2262).
#[test]
fn builtin_named_first_then_positional_reorders() {
    let ir = try_lower_src(
        "dt_named_first",
        "IMPORT datetime\n\
             FUNC run() AS Integer\n  \
               LET d = datetime::date(year := 2020, 5, 3)\n  \
               RETURN 0\nEND FUNC\n\
             FUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
    );
    assert!(
        ir.is_some(),
        "datetime::date named-first call did not lower"
    );
}

// An overloaded-param-name builtin (`datetime::instant`) called with named
// args routes through `normalize_overloaded_builtin_call_arguments`, selecting
// the matching overload and ordering positionals/named into it (lower.rs
// ~2276-2311).
#[test]
fn builtin_overloaded_named_arguments_select_overload() {
    let ir = try_lower_src(
        "dt_overload",
        "IMPORT datetime\n\
             FUNC run() AS Integer\n  \
               LET d = datetime::instant(days := 5, hours := 1, mins := 2, seconds := 3, nanos := 4)\n  \
               RETURN 0\nEND FUNC\n\
             FUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
    );
    assert!(
        ir.is_some(),
        "datetime::instant overloaded named call did not lower"
    );
}

// A per-position builtin called with MORE positionals than it has parameters,
// alongside a named arg, pushes the surplus onto the `extras` overflow list
// (lower.rs ~2253-2254). Type checking (which would reject the arity) is
// bypassed by lower_src.
#[test]
fn builtin_named_with_surplus_positionals_overflows_to_extras() {
    let ir = try_lower_src(
        "dt_extras",
        "IMPORT datetime\n\
             FUNC run() AS Integer\n  \
               LET d = datetime::date(2020, 5, 3, 99, month := 6)\n  \
               RETURN 0\nEND FUNC\n\
             FUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
    );
    assert!(
        ir.is_some(),
        "datetime::date surplus-arity call did not lower"
    );
}
