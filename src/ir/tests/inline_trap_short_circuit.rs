//! Inline `TRAP` over a short-circuited operand (plan-165-C).
//!
//! An inline `TRAP` covers its whole expression by lifting every fallible node
//! out ahead of it. A node in a **short-circuited** operand — the right side of
//! `AND`/`OR` — could not be lifted that way, because lifting evaluates it
//! unconditionally, so bug-457/bug-471/bug-825 refused the program with
//! `TYPE_INLINE_TRAP_SHORT_CIRCUIT_CALL`. plan-165-C lowers the `AND`/`OR` as a
//! conditional step instead (`ir::lower::wrap_trap_hoists`): the right
//! operand's own lifts run only when it is selected, and their failures join the
//! shared `$trap_failed` chain. The rule is retired, and every shape it refused
//! is now accepted — these run the checkers in-process over the programs the
//! refusals named, in every expression position and every comparison promotion,
//! and demand a clean bill (a program that the IR verifier rejected, because the
//! lowering left a raise site uncaptured or a `Checked` over a call, would fail
//! here). The runtime behavior — selected failures caught, unselected operands
//! never evaluated — is `tests/runtime/rt_inline_trap_short_circuit.rs`.

use crate::testutil::check_src_details;

/// Declarations every case draws on. `inner` is fallible; `positive` wraps it so
/// the conditional call is a plain user function; `pick` and `take` are
/// infallible.
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

FUNC apply(f AS FUNC(Integer) AS Integer, k AS Integer) AS Integer
  RETURN f(k)
END FUNC

FUNC accepts(f AS FUNC(Integer) AS Integer) AS Boolean
  RETURN TRUE
END FUNC

";

/// Accepted, or a panic naming every diagnostic. "No diagnostic" also rules out
/// an unrelated rejection, so these demand a clean bill.
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

/// A `main` binding `d` to `expression` under an inline TRAP, with the locals
/// the expressions below read.
fn trapped(expression: &str) -> String {
    format!(
        "\
FUNC main() AS Integer
  MUT t AS Boolean = TRUE
  MUT u AS Boolean = TRUE
  MUT z AS Integer = 0
  MUT n AS Integer = 5
  MUT fx AS Fixed = 1F
  MUT big AS Integer = 3000000000
  MUT ratio AS Float = 3000000000.0
  MUT small AS Byte = toByte(3)
  LET base AS Point = Point[1, 2]
  LET d = {expression} TRAP(e)
    RECOVER -1
  END TRAP
  RETURN d
END FUNC
"
    )
}

/// The Boolean-root form: a fallible call as the whole right operand.
#[test]
fn a_fallible_call_in_a_short_circuited_root_is_accepted() {
    accepted(
        "\
FUNC main() AS Integer
  MUT t AS Boolean = TRUE
  LET ok = t AND positive(-1) TRAP(e)
    RECOVER FALSE
  END TRAP
  LET either = t OR positive(-1) TRAP(e)
    RECOVER FALSE
  END TRAP
  io::print(toString(ok) & toString(either))
  RETURN 0
END FUNC
",
    );
}

/// Raising binary and unary operators in a short-circuited operand, a call
/// through a `FUNC`-typed parameter, a lambda value, and a negative literal
/// (which cannot raise).
#[test]
fn raising_operators_and_indirect_calls_in_a_short_circuited_operand_are_accepted() {
    for expression in [
        "pick(t AND (1 / z) > 0, 2)",
        "pick(t AND (-n) > 0, 2)",
        "pick(t AND -1 > 0, inner(3))",
        "pick(t AND apply(LAMBDA(v AS Integer) -> inner(v), 3) > 0, 2)",
        "pick(t AND accepts(LAMBDA(v AS Integer) -> inner(v)), inner(3))",
    ] {
        accepted(&trapped(expression));
    }
}

/// The conditional step is found under every kind of expression node — a
/// constructor argument, a `WITH` update, a list/set/map element, a member
/// access target, a nested call argument — because the lift reaches it through
/// the same walk as every other lifted node.
#[test]
fn a_conditional_under_every_expression_kind_is_accepted() {
    for expression in [
        "take(\"t\", 1, TRUE, Point[pick(t AND positive(-1), 1), 2])",
        "(WITH base { x := pick(t AND positive(-1), 1) }).x",
        "len([pick(t AND positive(-1), 1)])",
        "len(Set OF Integer { pick(t AND positive(-1), 1) })",
        "len(Map OF String TO Integer { \"a\" := pick(t AND positive(-1), 1) })",
        "Point[pick(t AND positive(-1), 1), 2].y",
        "pick(TRUE, pick(t AND positive(-1), 1))",
        "pick(positive(3) AND TRUE, 1)",
    ] {
        accepted(&trapped(expression));
    }
}

/// Raising comparisons (bug-816) in every short-circuited position bug-825 named
/// — right of `AND`, of `OR`, under `NOT`, nested, and the left side of an
/// `AND` that is itself short-circuited — and the non-raising comparisons.
#[test]
fn raising_and_non_raising_comparisons_in_a_short_circuited_operand_are_accepted() {
    for expression in [
        "pick(t AND fx < big, 2)",
        "pick(t AND fx >= ratio, 2)",
        "pick(t OR big = fx, 2)",
        "pick(t AND NOT (fx <> big), 2)",
        "pick(t OR (u AND (fx <= ratio)), 2)",
        "pick(t AND (fx > big AND u), 2)",
        "pick(fx < big AND t, inner(3))",
        "pick(t AND n < 3, inner(3))",
        "pick(t AND small < fx, inner(3))",
        "pick(t OR fx = 2F, inner(3))",
    ] {
        accepted(&trapped(expression));
    }
}

/// Outside an inline `TRAP` the same expression is ordinary propagation.
#[test]
fn a_short_circuited_fallible_call_outside_an_inline_trap_is_accepted() {
    accepted(
        "\
FUNC main() AS Integer
  MUT t AS Boolean = TRUE
  LET ok = t AND positive(-1)
  io::print(\"ok=\" & toString(ok))
  RETURN 0

  TRAP(e)
    RETURN 1
  END TRAP
END FUNC
",
    );
}
