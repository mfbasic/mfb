//! Algebraic simplification — a Level-1 Opt1 catalog row
//! (`planning/optimizations.md`): identity-element rewrites (`x * 1` → `x`) on
//! structured NIR, before Plan1 storage/symbol assignment. Driven per node by
//! [`super::local_rewrites`].
//!
//! Every rewrite here replaces a `Binary`/`Unary` node with one of its own
//! operands, so the surviving operand's value, evaluation order, and traps are
//! exactly the source's — the dropped operand is always a trap-free literal.
//! Two MFB-specific constraints shape the rule set:
//!
//! - **Never drop a non-constant operand.** Under checked arithmetic an operand
//!   can trap (or call), so `x * 0` → `0`, `x MOD 1` → `0`, and
//!   `FALSE AND x` → `FALSE` are *not* identity rewrites — they belong to the
//!   constant-folding / branch-simplification rows, not this one.
//! - **A numeric rewrite requires the surviving operand's known type to equal
//!   the constant's type.** Dropping the operator would otherwise change the
//!   §4.1 promoted result type: `byte + integer-0` is an `Integer` expression,
//!   and rewriting it to the bare `Byte` operand would retype it. The type of a
//!   `Local` comes from the driver's lexical scope walk — an operand whose type
//!   it cannot see (calls, member reads) is left alone.
//!
//! Per-type notes: `Float` keeps only its exact IEEE identities — `x * 1.0`,
//! `x / 1.0`, and `x - 0.0` (all bit-preserving, including for `-0.0`/non-finite
//! transients); `x + 0.0` is **not** an identity (`-0.0 + 0.0` is `+0.0`) and
//! `x ^ 1.0` rides the pow kernel, so both stay. `Fixed`/`Money` are excluded
//! entirely (raw-representation kernels and runtime rounding modes). `DIV` is
//! never an identity (`x DIV 1` is fractional division returning `Float`, §11).
//! `AND`/`OR`/`XOR` and `&` need no type lookup: §11 requires their operands to
//! be `Boolean`/`String` already, and `TRUE AND x` / `x AND TRUE` both still
//! evaluate `x` under short-circuiting, so the surviving side is exactly the
//! evaluated one.

use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;

use super::local_rewrites::{scopes_type_is, take, Scopes};

/// Try the row's rewrites at one node (children already rewritten). Returns
/// whether a rewrite fired, for the driver's `mfb build -v` fire count.
pub(super) fn rewrite_value(value: &mut NirValue, scopes: &Scopes) -> bool {
    match value {
        NirValue::Binary {
            op, left, right, ..
        } => {
            let survivor = match algebraic_survivor(op, left, right, scopes) {
                Some(Survivor::Left) => left,
                Some(Survivor::Right) => right,
                None => return false,
            };
            *value = take(survivor);
            true
        }
        NirValue::Unary { op, operand, .. } if op == "NOT" => {
            // NOT NOT x → x: NOT is Boolean-only (§11) and both negations
            // evaluate their operand, so peeling the pair preserves everything.
            if let NirValue::Unary {
                op: inner_op,
                operand: inner,
                ..
            } = operand.as_mut()
            {
                if inner_op == "NOT" {
                    *value = take(inner);
                    return true;
                }
            }
            false
        }
        _ => false,
    }
}

enum Survivor {
    Left,
    Right,
}

/// Which operand (if either) the whole binary expression can be replaced by.
fn algebraic_survivor(
    op: &str,
    left: &NirValue,
    right: &NirValue,
    scopes: &Scopes,
) -> Option<Survivor> {
    match op {
        // §11 types both `&` operands as String and both AND/OR/XOR operands as
        // Boolean, so no scope lookup is needed; the surviving operand is
        // evaluated in the original too (TRUE short-circuits AND onward, FALSE
        // short-circuits OR onward, XOR always evaluates both).
        "&" => two_sided(left, right, |v| is_string_const(v, "")),
        "AND" => two_sided(left, right, |v| is_boolean_const(v, "true")),
        "OR" | "XOR" => two_sided(left, right, |v| is_boolean_const(v, "false")),
        "+" | "-" | "*" | "/" | "^" => {
            // Right-side identity first (covers the non-commutative ops), then
            // the mirrored constant for + and *.
            if let Some(const_type) = numeric_identity_const(op, right) {
                if scopes_type_is(left, &const_type, scopes) {
                    return Some(Survivor::Left);
                }
            }
            if matches!(op, "+" | "*") {
                if let Some(const_type) = numeric_identity_const(op, left) {
                    if scopes_type_is(right, &const_type, scopes) {
                        return Some(Survivor::Right);
                    }
                }
            }
            None
        }
        _ => None,
    }
}

fn two_sided(
    left: &NirValue,
    right: &NirValue,
    is_identity: impl Fn(&NirValue) -> bool,
) -> Option<Survivor> {
    if is_identity(right) {
        Some(Survivor::Left)
    } else if is_identity(left) {
        Some(Survivor::Right)
    } else {
        None
    }
}

/// If `value` is the identity constant for `op`, its type (which the surviving
/// operand must then match exactly, so the §4.1 promoted result type is
/// unchanged by dropping the operator).
fn numeric_identity_const(op: &str, value: &NirValue) -> Option<ParameterType> {
    let NirValue::Const { type_, value } = value else {
        return None;
    };
    let matches = match (op, type_) {
        ("+", ParameterType::Integer | ParameterType::Byte) => integer_text_is(value, 0),
        ("-", ParameterType::Integer | ParameterType::Byte) => integer_text_is(value, 0),
        // `x - 0.0` is exact for every Float, including `-0.0` (unlike
        // `x + 0.0`); the subtrahend must be positive zero, since
        // `x - (-0.0)` is `x + 0.0`.
        ("-", ParameterType::Float) => float_text_is_positive_zero(value),
        ("*" | "/", ParameterType::Integer | ParameterType::Byte) => integer_text_is(value, 1),
        ("*" | "/", ParameterType::Float) => float_text_is_one(value),
        ("^", ParameterType::Integer | ParameterType::Byte) => integer_text_is(value, 1),
        _ => false,
    };
    matches.then(|| type_.clone())
}

fn integer_text_is(text: &str, expected: i128) -> bool {
    text.parse::<i128>() == Ok(expected)
}

fn float_text_is_one(text: &str) -> bool {
    text.parse::<f64>() == Ok(1.0)
}

fn float_text_is_positive_zero(text: &str) -> bool {
    matches!(text.parse::<f64>(), Ok(parsed) if parsed == 0.0 && parsed.is_sign_positive())
}

fn is_string_const(value: &NirValue, expected: &str) -> bool {
    matches!(value, NirValue::Const { type_, value }
        if *type_ == ParameterType::String && value == expected)
}

fn is_boolean_const(value: &NirValue, expected: &str) -> bool {
    matches!(value, NirValue::Const { type_, value }
        if *type_ == ParameterType::Boolean && value == expected)
}

#[cfg(test)]
mod tests {
    use super::super::local_rewrites::testutil::*;
    use super::*;

    /// Apply this row alone (no walk — callers hand a leaf-simplified node).
    fn simplified(mut value: NirValue, scopes: &Scopes) -> String {
        rewrite_value(&mut value, scopes);
        shape(&value)
    }

    #[test]
    fn integer_identities_rewrite_to_the_operand() {
        let scopes = int_scope("x");
        for value in [
            binary("+", local("x"), int_const("0")),
            binary("+", int_const("0"), local("x")),
            binary("-", local("x"), int_const("0")),
            binary("*", local("x"), int_const("1")),
            binary("*", int_const("1"), local("x")),
            binary("/", local("x"), int_const("1")),
            binary("^", local("x"), int_const("1")),
        ] {
            assert_eq!(simplified(value, &scopes), "local(x)");
        }
    }

    #[test]
    fn boolean_and_string_identities_need_no_scope() {
        let scopes = Scopes::new();
        for (value, expected) in [
            (
                binary(
                    "AND",
                    local("b"),
                    typed_const(ParameterType::Boolean, "true"),
                ),
                "local(b)",
            ),
            (
                binary(
                    "AND",
                    typed_const(ParameterType::Boolean, "true"),
                    local("b"),
                ),
                "local(b)",
            ),
            (
                binary(
                    "OR",
                    local("b"),
                    typed_const(ParameterType::Boolean, "false"),
                ),
                "local(b)",
            ),
            (
                binary(
                    "XOR",
                    typed_const(ParameterType::Boolean, "false"),
                    local("b"),
                ),
                "local(b)",
            ),
            (
                binary("&", local("s"), typed_const(ParameterType::String, "")),
                "local(s)",
            ),
            (
                binary("&", typed_const(ParameterType::String, ""), local("s")),
                "local(s)",
            ),
            (unary("NOT", unary("NOT", local("b"))), "local(b)"),
        ] {
            assert_eq!(simplified(value, &scopes), expected);
        }
    }

    #[test]
    fn float_keeps_only_its_exact_identities() {
        let mut scopes = Scopes::new();
        scopes.insert("f".to_string(), ParameterType::Float);
        for value in [
            binary("*", local("f"), typed_const(ParameterType::Float, "1.0")),
            binary("*", typed_const(ParameterType::Float, "1.0"), local("f")),
            binary("/", local("f"), typed_const(ParameterType::Float, "1.0")),
            binary("-", local("f"), typed_const(ParameterType::Float, "0.0")),
        ] {
            assert_eq!(simplified(value, &scopes), "local(f)");
        }
        // `f + 0.0` maps -0.0 to +0.0 and `f ^ 1.0` rides the pow kernel:
        // neither is an identity, so both stay.
        for (value, expected) in [
            (
                binary("+", local("f"), typed_const(ParameterType::Float, "0.0")),
                "(local(f) + const(0.0))",
            ),
            (
                binary("^", local("f"), typed_const(ParameterType::Float, "1.0")),
                "(local(f) ^ const(1.0))",
            ),
        ] {
            assert_eq!(simplified(value, &scopes), expected);
        }
    }

    /// Rewrites that would drop a non-constant operand — and with it a possible
    /// trap or call — are out of this row's scope, even when constant-folding
    /// could justify them.
    #[test]
    fn operand_dropping_shapes_are_never_rewritten() {
        let scopes = int_scope("x");
        for value in [
            binary("*", local("x"), int_const("0")),
            binary("MOD", local("x"), int_const("1")),
            binary("-", int_const("0"), local("x")),
            binary("^", int_const("1"), local("x")),
            binary("DIV", local("x"), int_const("1")),
        ] {
            let rendered = shape(&value);
            assert_eq!(simplified(value, &scopes), rendered);
        }
        let false_and = binary(
            "AND",
            typed_const(ParameterType::Boolean, "false"),
            local("b"),
        );
        let rendered = shape(&false_and);
        assert_eq!(simplified(false_and, &scopes), rendered);
    }

    /// `byte + integer-0` promotes to Integer (§4.1); dropping the `+` would
    /// retype the expression, so a type mismatch between the constant and the
    /// surviving operand blocks the rewrite — as does an operand whose type the
    /// scope walk cannot see.
    #[test]
    fn promotion_changing_or_unknown_typed_operands_block_the_rewrite() {
        let mut scopes = Scopes::new();
        scopes.insert("b".to_string(), ParameterType::Byte);
        let mixed = binary("+", local("b"), int_const("0"));
        assert_eq!(simplified(mixed, &scopes), "(local(b) + const(0))");

        let unknown = binary("+", local("nope"), int_const("0"));
        assert_eq!(simplified(unknown, &scopes), "(local(nope) + const(0))");

        // Same-type Byte identity still fires.
        let byte_zero = typed_const(ParameterType::Byte, "0");
        let same = binary("+", local("b"), byte_zero);
        assert_eq!(simplified(same, &scopes), "local(b)");
    }
}
