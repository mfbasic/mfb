//! `TYPE_INLINE_TRAP_SHORT_CIRCUIT_CALL` — the one shape an inline `TRAP`
//! cannot desugar (bug-457, bug-471).
//!
//! An inline `TRAP` covers its whole expression by lifting every fallible node
//! out ahead of it: a call becomes its own `CallResult` + `If ResultIsOk`, a
//! raising operator becomes its own `Checked` bind, and both join the shared
//! `$trap_failed` chain. A node in a **short-circuited** operand — the right
//! side of `AND`/`OR` — cannot be lifted, because lifting evaluates it
//! unconditionally and the source said it must not be. The desugar's answer is
//! to refuse the program and name the node, so the author binds it to its own
//! `LET … TRAP` first; the alternative is the error escaping the handler
//! unnoticed, which is bug-457 and bug-471 themselves.
//!
//! The rule already had regression tests, but only in `tests/` — subprocess
//! builds through `mfb_exe()`, whose coverage lands in the child process and
//! never in `src/**`. The walk that finds the offender
//! (`Analyzer::find_short_circuited_call`, ~120 lines of expression cases) read
//! as entirely unexecuted. These run the same checkers in-process, and go
//! further than the subprocess pair in one respect that matters: they assert
//! *which* node the message names, not just that some diagnostic fired.

use crate::testutil::{check_src, check_src_details};

const RULE: &str = "TYPE_INLINE_TRAP_SHORT_CIRCUIT_CALL";

/// Declarations every case draws on. `inner` is fallible so a call can be the
/// offender; `positive` wraps it so the offender is a plain user function
/// rather than a builtin; `pick` and `take` are infallible, so anything they
/// are handed is the only thing in the expression that can fail.
const PRELUDE: &str = "\
IMPORT io

TYPE Point
  x AS Integer
  y AS Integer
END TYPE

FUNC inner(n AS Integer) AS Integer
  IF n < 0 THEN FAIL error(90000001, \"inner failed\")
  RETURN n * 2
END FUNC

FUNC positive(n AS Integer) AS Boolean
  RETURN inner(n) > 0
END FUNC

FUNC pick(flag AS Boolean, b AS Integer) AS Integer
  RETURN b
END FUNC

FUNC take(label AS String, n AS Integer, flag AS Boolean, pt AS Point) AS Integer
  RETURN n + pt.x
END FUNC

";

/// The detail of the single `TYPE_INLINE_TRAP_SHORT_CIRCUIT_CALL` `body`
/// raises, or a panic naming what came out instead.
fn refusal(body: &str) -> String {
    let source = format!("{PRELUDE}{body}");
    let diagnostics = check_src_details(&source);
    let mut matching = diagnostics.iter().filter(|d| d.rule == RULE);
    let Some(first) = matching.next() else {
        panic!(
            "expected {RULE}, got {:?}",
            diagnostics
                .iter()
                .map(|d| (d.rule.as_str(), d.detail.as_str()))
                .collect::<Vec<_>>()
        );
    };
    assert!(
        matching.next().is_none(),
        "the rule reports the FIRST offender only, so a program must raise it \
         once; it raised {} times",
        diagnostics.iter().filter(|d| d.rule == RULE).count()
    );
    first.detail.clone()
}

/// Accepted, or a panic naming every diagnostic. Used for the shapes that must
/// NOT be refused — an exemption asserted only as "the rule did not fire" would
/// also pass on a program the front end rejected for an unrelated reason, so
/// these demand a clean bill.
fn accepted(body: &str) {
    let source = format!("{PRELUDE}{body}");
    let diagnostics = check_src_details(&source);
    assert!(
        diagnostics.is_empty(),
        "expected acceptance, got {:?}",
        diagnostics
            .iter()
            .map(|d| (d.rule.as_str(), d.detail.as_str()))
            .collect::<Vec<_>>()
    );
}

/// A fallible call in a short-circuited operand is refused, and the message
/// names the call.
///
/// `TRUE AND positive(-1)`: `positive` runs only if the left side is true, so
/// the desugar cannot hoist it. Naming `positive` is what makes the diagnostic
/// actionable — the expression may hold several calls and only one of them is
/// in the unliftable position.
#[test]
fn a_fallible_call_in_a_short_circuited_operand_names_the_call() {
    let detail = refusal(
        "\
FUNC main() AS Integer
  LET ok = TRUE AND positive(-1) TRAP(e)
    RECOVER FALSE
  END TRAP
  io::print(\"ok=\" & toString(ok))
  RETURN 0
END FUNC
",
    );
    assert!(
        detail.contains("`positive`"),
        "the refusal must name the call that cannot be lifted; it said {detail:?}"
    );
}

/// A raising *operator* in a short-circuited operand is refused too, and the
/// message names the operator (bug-471).
///
/// `t AND (1 / z) > 0` divides only when `t`; the division by zero is exactly
/// the error the handler was written for, and before bug-471 it propagated
/// straight past it. `/` is the offender, not the enclosing call — the walk
/// reports the operator node it found, and a message naming `pick` here would
/// send the author to the wrong place.
#[test]
fn a_raising_binary_operator_in_a_short_circuited_operand_names_the_operator() {
    let detail = refusal(
        "\
FUNC main() AS Integer
  MUT z AS Integer = 0
  MUT t AS Boolean = TRUE
  LET d = pick(t AND (1 / z) > 0, 2) TRAP(e)
    RECOVER -1
  END TRAP
  RETURN d
END FUNC
",
    );
    assert!(
        detail.contains("`/`"),
        "the refusal must name the operator that cannot be lifted; it said {detail:?}"
    );
}

/// The unary half of the same rule. Negating `Integer::MIN` overflows, so `-n`
/// over a variable can raise and is refused in a short-circuited operand.
///
/// The two arities are separate code paths — before they were split they shared
/// one `&str` list in which `"-"` meant subtraction and negation at once — so
/// the binary case above does not stand in for this one.
#[test]
fn a_raising_unary_operator_in_a_short_circuited_operand_names_the_operator() {
    let detail = refusal(
        "\
FUNC main() AS Integer
  MUT n AS Integer = 5
  MUT t AS Boolean = TRUE
  LET d = pick(t AND (-n) > 0, 2) TRAP(e)
    RECOVER -1
  END TRAP
  RETURN d
END FUNC
",
    );
    assert!(
        detail.contains("`-`"),
        "the refusal must name the operator that cannot be lifted; it said {detail:?}"
    );
}

/// `-1` is the spelling of a negative literal, not a negation that can
/// overflow, so it is exempt — `t AND -1 > 0` compiles.
///
/// The exemption reads the same predicate the lift reads
/// (`fallible::is_total_literal_negation`), so the checker and the desugar
/// cannot drift into disagreeing about which negations are total. Without it
/// every `AND`-guarded comparison against a negative constant inside a `TRAP`
/// would be refused, which is a common enough shape to be a usability bug.
///
/// The `TRAP` is warranted independently: `inner(3)` is fallible and sits in an
/// unconditional argument, so this asserts the exemption rather than the
/// absence of anything to lift.
#[test]
fn a_negative_literal_is_not_a_raising_negation() {
    accepted(
        "\
FUNC main() AS Integer
  MUT t AS Boolean = TRUE
  LET d = pick(t AND -1 > 0, inner(3)) TRAP(e)
    RECOVER -1
  END TRAP
  RETURN d
END FUNC
",
    );
}

/// A fallible call that is *unconditionally* evaluated is lifted, not refused —
/// including one on the LEFT of an `AND`, which always runs.
///
/// This is the rule's other edge. `conditional` turns on only when the walk
/// descends into a short-circuited operand, and a checker that set it for the
/// whole `AND` node would refuse the programs the desugar handles correctly.
#[test]
fn an_unconditionally_evaluated_call_is_lifted_not_refused() {
    accepted(
        "\
FUNC main() AS Integer
  LET ok = positive(3) AND TRUE TRAP(e)
    RECOVER FALSE
  END TRAP
  io::print(\"ok=\" & toString(ok))
  RETURN 0
END FUNC
",
    );
}

/// A lambda body is not part of this expression's evaluation.
///
/// The lambda here is a *value* handed to `accepts`, which never calls it —
/// whatever it calls would run at some later call site, under whatever error
/// handling is in force there, so this expression cannot raise it and the
/// handler has nothing to cover. Walking into the body would find `inner`,
/// report it, and refuse a program that is correct.
///
/// `accepts` has to be a function that does not invoke its argument, which is
/// the whole subtlety: a `FUNC`-typed parameter that *is* invoked makes the
/// enclosing function fallible, so a `collections::count`-shaped call in a
/// short-circuited operand is refused on its own account — under its own name,
/// not the lambda's. The `TRAP` is still warranted by `inner(3)` in the
/// unconditional argument.
#[test]
fn a_lambda_body_is_not_walked() {
    accepted(
        "\
FUNC accepts(f AS FUNC(Integer) AS Integer) AS Boolean
  RETURN TRUE
END FUNC

FUNC main() AS Integer
  MUT t AS Boolean = TRUE
  LET d = pick(t AND accepts(LAMBDA(v AS Integer) -> inner(v)), inner(3)) TRAP(e)
    RECOVER -1
  END TRAP
  RETURN d
END FUNC
",
    );
}

/// A fallible call *through* a `FUNC`-typed parameter in a short-circuited
/// operand is refused under the enclosing function's name.
///
/// The counterpart to the case above, and the reason its `accepts` is written
/// the way it is: invoking the parameter makes `apply` fallible, so `apply` is
/// the offender. Naming `apply` rather than `inner` is right — `inner` is not
/// in this expression at all, and sending the author to a lambda body they
/// would then have to trace back is the opposite of what the message is for.
#[test]
fn a_call_through_a_func_typed_parameter_is_refused_under_its_own_name() {
    let detail = refusal(
        "\
FUNC apply(f AS FUNC(Integer) AS Integer, n AS Integer) AS Integer
  RETURN f(n)
END FUNC

FUNC main() AS Integer
  MUT t AS Boolean = TRUE
  LET d = pick(t AND apply(LAMBDA(v AS Integer) -> inner(v), 3) > 0, 2) TRAP(e)
    RECOVER -1
  END TRAP
  RETURN d
END FUNC
",
    );
    assert!(
        detail.contains("`apply`") && !detail.contains("`inner`"),
        "the refusal must name the call in the operand, not something inside a \
         lambda it was handed; it said {detail:?}"
    );
}

/// The walk reaches every kind of expression node, so an offender cannot hide
/// behind one.
///
/// Each case buries the same unliftable call one node deeper than the last —
/// under a constructor argument, a `WITH` update, a list/set/map element, a
/// member access target, a nested call argument. A walk missing any one of
/// those arms would accept the program and miscompile it, which is the failure
/// bug-457 was filed for; the arms are individually load-bearing, and a single
/// combined program would stop at the first offender and prove only one.
#[test]
fn the_walk_finds_an_offender_under_every_expression_kind() {
    for (what, expression) in [
        (
            "a constructor argument",
            "take(\"t\", 1, TRUE, Point[pick(t AND positive(-1), 1), 2]).x",
        ),
        (
            "a WITH update",
            "(WITH base { x := pick(t AND positive(-1), 1) }).x",
        ),
        ("a list element", "len([pick(t AND positive(-1), 1)])"),
        (
            "a set element",
            "len(Set OF Integer { pick(t AND positive(-1), 1) })",
        ),
        (
            "a map value",
            "len(Map OF String TO Integer { \"a\" := pick(t AND positive(-1), 1) })",
        ),
        (
            "a member-access target",
            "Point[pick(t AND positive(-1), 1), 2].y",
        ),
        (
            "a nested call argument",
            "pick(TRUE, pick(t AND positive(-1), 1))",
        ),
    ] {
        let detail = refusal(&format!(
            "\
FUNC main() AS Integer
  MUT t AS Boolean = TRUE
  LET base AS Point = Point[1, 2]
  LET d = {expression} TRAP(e)
    RECOVER -1
  END TRAP
  RETURN d
END FUNC
"
        ));
        assert!(
            detail.contains("`positive`"),
            "an offender under {what} must be found and named; it said {detail:?}"
        );
    }
}

/// The rule is scoped to inline `TRAP`. The identical expression outside one is
/// none of its business — a short-circuited fallible call in an ordinary
/// binding propagates to the function-level handler, which is what it should
/// do.
#[test]
fn the_rule_only_applies_inside_an_inline_trap() {
    assert_eq!(
        check_src(&format!(
            "{PRELUDE}\
FUNC main() AS Integer
  MUT t AS Boolean = TRUE
  LET ok = t AND positive(-1)
  io::print(\"ok=\" & toString(ok))
  RETURN 0

  TRAP(e)
    RETURN 1
  END TRAP
END FUNC
"
        )),
        Vec::<String>::new(),
        "a short-circuited fallible call outside an inline TRAP is ordinary \
         error propagation, not an unliftable lift"
    );
}
