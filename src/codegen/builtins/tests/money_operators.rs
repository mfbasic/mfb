//! Which operators a `Money` operand admits, and the reason each refusal gives.
//!
//! `Money` is a scaled integer with currency semantics, so the arithmetic that
//! is meaningful on it is a strict subset of the arithmetic that is meaningful
//! on a `Fixed` with the same representation. `ir::verify`'s
//! `TYPE_MONEY_OPERATION_INVALID` is that subset, and it does not just refuse —
//! it names WHY, with a different clause per shape, because "you cannot do that
//! to money" is not actionable and "money squared is not money" is.
//!
//! The rule fires; its `_` fallback reason does not, and neither do several of
//! the named ones. This is the table of every shape, asserted by the clause it
//! produces, so a refusal that started giving the generic reason for a shape
//! that has a specific one would show up here rather than in a user's terminal.

use crate::testutil::check_src_details;

const PRELUDE: &str = "\
IMPORT io

FUNC main() AS Integer
  LET m AS Money = 5m
  LET n AS Money = 2m
  LET x AS Integer = 3
  LET f AS Fixed = 1.5F
";

/// The detail of the single `TYPE_MONEY_OPERATION_INVALID` `expr` raises.
fn refusal(expr: &str) -> String {
    let source = format!("{PRELUDE}  io::print(toString({expr}))\n  RETURN 0\nEND FUNC\n");
    let diagnostics = check_src_details(&source);
    let money: Vec<&crate::rules::PendingDiagnostic> = diagnostics
        .iter()
        .filter(|d| d.rule == "TYPE_MONEY_OPERATION_INVALID")
        .collect();
    assert_eq!(
        money.len(),
        1,
        "`{expr}` must raise TYPE_MONEY_OPERATION_INVALID exactly once; the \
         checkers said {:?}",
        diagnostics
            .iter()
            .map(|d| (d.rule.as_str(), d.detail.as_str()))
            .collect::<Vec<_>>()
    );
    money[0].detail.clone()
}

/// Every refused shape, and the clause it must give.
#[test]
fn each_refused_money_operation_says_why() {
    for (expr, reason) in [
        // Mixing money with a bare number is the common mistake, and the
        // parenthetical is the actionable half: the fix is to make the other
        // operand Money, not to cast the result.
        (
            "m + x",
            "requires both operands to be Money (a Money and a non-Money value cannot be combined)",
        ),
        (
            "m - x",
            "requires both operands to be Money (a Money and a non-Money value cannot be combined)",
        ),
        (
            "m MOD x",
            "requires both operands to be Money (a Money and a non-Money value cannot be combined)",
        ),
        // Dimensional, not representational: money x money has no unit.
        (
            "m * n",
            "cannot multiply two Money values (money² is not Money)",
        ),
        ("x / m", "cannot divide a non-Money value by a Money value"),
        ("m ^ x", "does not support exponentiation of a Money value"),
    ] {
        let detail = refusal(expr);
        assert!(
            detail.contains(reason),
            "`{expr}` must be refused with {reason:?}; it said {detail:?}"
        );
    }
}

/// A shape with no clause of its own gets the generic reason.
///
/// The `_` arm, derived from `numeric::typed_money_result_type` rather than
/// guessed: `IntDiv` yields a result only when the LEFT operand is the Money
/// one, and `IntDiv` has no clause of its own in the reason table. So
/// `<Integer> DIV <Money>` is the shape that reaches it — dividing a count by
/// an amount, which has no meaning in either direction the named clauses
/// describe.
///
/// (`m DIV n` was the first guess and is ACCEPTED — `Money DIV Money` is a
/// Float. The reason table's arms are about the WORDING, not the validity; the
/// validity question is settled before this match runs.)
#[test]
fn a_money_operation_with_no_specific_reason_gets_the_generic_one() {
    let detail = refusal("x DIV m");
    assert!(
        detail.contains("is not valid for Money operands"),
        "an operator with no clause of its own falls back to the generic \
         reason; it said {detail:?}"
    );
}

/// The operations `Money` DOES admit are accepted.
///
/// The row that makes the rest mean something: every assertion above is a
/// refusal, and all of them would hold against a checker that refused every
/// operator on a `Money`.
#[test]
fn the_money_arithmetic_that_is_meaningful_is_accepted() {
    for expr in ["m + n", "m - n", "m * x", "m / x", "m / n", "m MOD n"] {
        let source = format!("{PRELUDE}  io::print(toString({expr}))\n  RETURN 0\nEND FUNC\n");
        let rules: Vec<String> = check_src_details(&source)
            .into_iter()
            .map(|d| d.rule)
            .collect();
        assert!(
            rules.is_empty(),
            "`{expr}` is meaningful Money arithmetic; the checkers refused it \
             with {rules:?}"
        );
    }
}
