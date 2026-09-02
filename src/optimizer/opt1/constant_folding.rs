//! Constant folding (the Opt1 half) — a Level-1 catalog row
//! (`planning/optimizations.md`): evaluate constant expressions at compile
//! time. Driven per node by [`super::local_rewrites`], before the algebraic and
//! strength rows so a fold can feed them (`(1+1) * x` folds to `2 * x`, which
//! strength-reduces to `x + x`).
//!
//! **L1 is the non-trapping subset, by the table's own reclassification note:**
//! folding may never turn a conditionally-executed runtime trap into an
//! unconditional/earlier one (or erase it), so a constant expression that would
//! trap at runtime — overflow, `/ 0`, `MIN / -1`, a Byte result out of 0..=255,
//! a negative `^` exponent — is left exactly as written. Every arithmetic fold
//! below computes in the checked i64/Byte domain the runtime uses and bails to
//! "no fold" unless the runtime result provably exists.
//!
//! Folded domains: Integer and Byte arithmetic (`+ - * / MOD ^`, unary `-` on
//! Integer), Boolean logic (`AND OR XOR`, unary `NOT` — both operands are
//! constants, so short-circuit evaluation order is unobservable), String `&`,
//! and same-type comparisons on Integer/Byte/Boolean/String (comparisons never
//! trap; MFB orders Strings by Unicode scalar value, which for UTF-8 text is
//! exactly Rust's byte order). `DIV` never folds here (it returns Float,
//! below). Excluded, deliberately:
//!
//! - **Float / Fixed / Money, entirely** (arithmetic *and* comparisons). A NIR
//!   constant is a text literal, and the backend materializes it with the
//!   naive, not-correctly-rounded parser (`toFloat` caveat) — folding with
//!   host-exact f64 math could produce a literal (or a comparison verdict)
//!   that disagrees with the bits the runtime would have computed. Waits for a
//!   bit-exact constant channel; Fixed/Money add raw-representation rounding
//!   modes on top.
//! - **Cross-type comparisons** (`Integer = Float` is legal §4.11): folding
//!   would need the promotion the backend applies; same parser caveat.
//! - **`^` with an exponent past `u32`** (only bases −1/0/1 survive at
//!   runtime): rare enough to leave to the runtime's closed forms.

use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;

use super::local_rewrites::Scopes;
use crate::operators::{BinaryOp, UnaryOp};

/// Try to fold one node (children already rewritten — so nested constant
/// subtrees have collapsed by the time the parent is offered). Returns whether
/// a fold fired, for the driver's `mfb build -v` fire count.
pub(super) fn rewrite_value(value: &mut NirValue, _scopes: &Scopes) -> bool {
    let folded = match value {
        NirValue::Binary {
            op, left, right, ..
        } => fold_binary(*op, left, right),
        NirValue::Unary { op, operand, .. } => fold_unary(*op, operand),
        _ => None,
    };
    match folded {
        Some(folded) => {
            *value = folded;
            true
        }
        None => false,
    }
}

fn fold_unary(op: UnaryOp, operand: &NirValue) -> Option<NirValue> {
    let NirValue::Const { type_, value } = operand else {
        return None;
    };
    match (op, type_) {
        // Byte has no negative values (`-b` traps for any nonzero b): no fold.
        (UnaryOp::Negate, ParameterType::Integer) => {
            let negated = value.parse::<i64>().ok()?.checked_neg()?;
            Some(integer_const(negated))
        }
        (UnaryOp::Not, ParameterType::Boolean) => Some(boolean_const(value != "true")),
        _ => None,
    }
}

fn fold_binary(op: BinaryOp, left: &NirValue, right: &NirValue) -> Option<NirValue> {
    let (
        NirValue::Const {
            type_: left_type,
            value: left_text,
        },
        NirValue::Const {
            type_: right_type,
            value: right_text,
        },
    ) = (left, right)
    else {
        return None;
    };
    if left_type != right_type {
        // Same-type only: a mixed pairing folds under §4.1 promotion, which
        // drags in the excluded Float domain — leave it to the backend.
        return None;
    }
    match left_type {
        ParameterType::Integer => fold_integer(op, left_text, right_text),
        ParameterType::Byte => fold_byte(op, left_text, right_text),
        ParameterType::Boolean => fold_boolean(op, left_text, right_text),
        ParameterType::String => fold_string(op, left_text, right_text),
        _ => None,
    }
}

fn fold_integer(op: BinaryOp, left: &str, right: &str) -> Option<NirValue> {
    let a = left.parse::<i64>().ok()?;
    let b = right.parse::<i64>().ok()?;
    if let Some(verdict) = compare(op, &a, &b) {
        return Some(boolean_const(verdict));
    }
    // Checked in the runtime's own i64 domain: `None` (the would-trap cases —
    // overflow, b == 0, MIN / -1, negative exponent) means no fold, keeping the
    // runtime trap where and how the source had it.
    let result = match op {
        BinaryOp::Add => a.checked_add(b),
        BinaryOp::Subtract => a.checked_sub(b),
        BinaryOp::Multiply => a.checked_mul(b),
        BinaryOp::Divide => (b != 0).then(|| a.checked_div(b)).flatten(),
        BinaryOp::Mod => (b != 0).then(|| a.checked_rem(b)).flatten(),
        BinaryOp::Power => (b >= 0)
            .then(|| u32::try_from(b).ok())
            .flatten()
            .and_then(|exponent| a.checked_pow(exponent)),
        _ => None,
    }?;
    Some(integer_const(result))
}

fn fold_byte(op: BinaryOp, left: &str, right: &str) -> Option<NirValue> {
    let a = left.parse::<u8>().ok()?;
    let b = right.parse::<u8>().ok()?;
    if let Some(verdict) = compare(op, &a, &b) {
        return Some(boolean_const(verdict));
    }
    // Compute in i64 (every Byte op fits), then require the runtime's 0..=255
    // result range: outside it the runtime traps (ErrOverflow above,
    // ErrUnderflow below for `-`), so there is no fold.
    let (a, b) = (i64::from(a), i64::from(b));
    let result = match op {
        BinaryOp::Add => Some(a + b),
        BinaryOp::Subtract => Some(a - b),
        BinaryOp::Multiply => Some(a * b),
        BinaryOp::Divide => (b != 0).then(|| a / b),
        BinaryOp::Mod => (b != 0).then(|| a % b),
        BinaryOp::Power => u32::try_from(b)
            .ok()
            .and_then(|exponent| a.checked_pow(exponent)),
        _ => None,
    }?;
    let byte = u8::try_from(result).ok()?;
    Some(NirValue::Const {
        type_: ParameterType::Byte,
        value: byte.to_string(),
    })
}

fn fold_boolean(op: BinaryOp, left: &str, right: &str) -> Option<NirValue> {
    let a = left == "true";
    let b = right == "true";
    let result = match op {
        BinaryOp::And => a && b,
        BinaryOp::Or => a || b,
        BinaryOp::Xor => a != b,
        BinaryOp::Equal => a == b,
        BinaryOp::NotEqual => a != b,
        // Boolean is comparable but not orderable (§4.11).
        _ => return None,
    };
    Some(boolean_const(result))
}

fn fold_string(op: BinaryOp, left: &str, right: &str) -> Option<NirValue> {
    if op == BinaryOp::Concat {
        return Some(NirValue::Const {
            type_: ParameterType::String,
            value: format!("{left}{right}"),
        });
    }
    // §4.11: Strings order lexicographically by Unicode scalar value, which is
    // byte order for UTF-8 — exactly Rust's `str` ordering.
    compare(op, &left, &right).map(boolean_const)
}

/// The comparison verdict for an ordered, non-trapping domain, or `None` when
/// `op` is not a comparison.
fn compare<T: PartialOrd>(op: BinaryOp, a: &T, b: &T) -> Option<bool> {
    match op {
        BinaryOp::Equal => Some(a == b),
        BinaryOp::NotEqual => Some(a != b),
        BinaryOp::Less => Some(a < b),
        BinaryOp::Greater => Some(a > b),
        BinaryOp::LessEqual => Some(a <= b),
        BinaryOp::GreaterEqual => Some(a >= b),
        _ => None,
    }
}

fn integer_const(value: i64) -> NirValue {
    NirValue::Const {
        type_: ParameterType::Integer,
        value: value.to_string(),
    }
}

fn boolean_const(value: bool) -> NirValue {
    NirValue::Const {
        type_: ParameterType::Boolean,
        value: if value { "true" } else { "false" }.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::local_rewrites::testutil::*;
    use super::*;

    /// Apply this row alone (no walk — callers hand a leaf-simplified node).
    fn folded(mut value: NirValue, scopes: &Scopes) -> String {
        rewrite_value(&mut value, scopes);
        shape(&value)
    }

    #[test]
    fn integer_arithmetic_folds() {
        let scopes = Scopes::new();
        for (value, expected) in [
            (
                binary(BinaryOp::Add, int_const("2"), int_const("3")),
                "const(5)",
            ),
            (
                binary(BinaryOp::Subtract, int_const("2"), int_const("5")),
                "const(-3)",
            ),
            (
                binary(BinaryOp::Multiply, int_const("7"), int_const("6")),
                "const(42)",
            ),
            (
                binary(BinaryOp::Divide, int_const("7"), int_const("2")),
                "const(3)",
            ),
            (
                binary(BinaryOp::Divide, int_const("-7"), int_const("2")),
                "const(-3)",
            ),
            (
                binary(BinaryOp::Mod, int_const("-7"), int_const("2")),
                "const(-1)",
            ),
            (
                binary(BinaryOp::Power, int_const("3"), int_const("4")),
                "const(81)",
            ),
            (unary(UnaryOp::Negate, int_const("5")), "const(-5)"),
            (unary(UnaryOp::Negate, int_const("-5")), "const(5)"),
        ] {
            assert_eq!(folded(value, &scopes), expected);
        }
    }

    /// A constant expression that would trap at runtime must keep that trap:
    /// no fold for overflow, division by zero, `MIN / -1`, negative exponents,
    /// or the most-negative negation.
    #[test]
    fn would_trap_expressions_never_fold() {
        let scopes = Scopes::new();
        let max = i64::MAX.to_string();
        let min = i64::MIN.to_string();
        for value in [
            binary(BinaryOp::Add, int_const(&max), int_const("1")),
            binary(BinaryOp::Subtract, int_const(&min), int_const("1")),
            binary(BinaryOp::Multiply, int_const(&max), int_const("2")),
            binary(BinaryOp::Divide, int_const("1"), int_const("0")),
            binary(BinaryOp::Mod, int_const("1"), int_const("0")),
            binary(BinaryOp::Divide, int_const(&min), int_const("-1")),
            binary(BinaryOp::Mod, int_const(&min), int_const("-1")),
            binary(BinaryOp::Power, int_const("2"), int_const("-1")),
            binary(BinaryOp::Power, int_const("2"), int_const("64")),
            binary(BinaryOp::IntDiv, int_const("2"), int_const("3")),
            unary(UnaryOp::Negate, int_const(&min)),
        ] {
            let rendered = shape(&value);
            assert_eq!(folded(value, &scopes), rendered);
        }
    }

    #[test]
    fn byte_folds_only_inside_its_range() {
        let scopes = Scopes::new();
        let byte = |v: &str| typed_const(ParameterType::Byte, v);
        let value = binary(BinaryOp::Add, byte("100"), byte("100"));
        assert_eq!(folded(value, &scopes), "const(200)");
        let value = binary(BinaryOp::Power, byte("3"), byte("5"));
        assert_eq!(folded(value, &scopes), "const(243)");
        assert_eq!(
            folded(binary(BinaryOp::Multiply, byte("6"), byte("7")), &scopes),
            "const(42)"
        );
        assert_eq!(
            folded(binary(BinaryOp::Divide, byte("9"), byte("2")), &scopes),
            "const(4)"
        );
        assert_eq!(
            folded(binary(BinaryOp::Mod, byte("9"), byte("2")), &scopes),
            "const(1)"
        );
        assert_eq!(
            folded(binary(BinaryOp::Equal, byte("9"), byte("9")), &scopes),
            "const(true)"
        );
        // 200 + 100 = 300 and 5 - 9 = -4 both trap at runtime: no fold.
        for value in [
            binary(BinaryOp::Add, byte("200"), byte("100")),
            binary(BinaryOp::Subtract, byte("5"), byte("9")),
            binary(BinaryOp::Divide, byte("5"), byte("0")),
            binary(BinaryOp::IntDiv, byte("5"), byte("1")),
            unary(UnaryOp::Negate, byte("5")),
        ] {
            let rendered = shape(&value);
            assert_eq!(folded(value, &scopes), rendered);
        }
    }

    #[test]
    fn boolean_string_and_comparison_folds() {
        let scopes = Scopes::new();
        let boolean = |v: &str| typed_const(ParameterType::Boolean, v);
        let string = |v: &str| typed_const(ParameterType::String, v);
        for (value, expected) in [
            (
                binary(BinaryOp::And, boolean("true"), boolean("false")),
                "const(false)",
            ),
            (
                binary(BinaryOp::Or, boolean("false"), boolean("true")),
                "const(true)",
            ),
            (
                binary(BinaryOp::Xor, boolean("true"), boolean("true")),
                "const(false)",
            ),
            (unary(UnaryOp::Not, boolean("true")), "const(false)"),
            (
                binary(BinaryOp::Concat, string("ab"), string("cd")),
                "const(abcd)",
            ),
            (
                binary(BinaryOp::Less, string("apple"), string("b")),
                "const(true)",
            ),
            (
                binary(BinaryOp::Equal, string("a"), string("b")),
                "const(false)",
            ),
            (
                binary(BinaryOp::Less, int_const("2"), int_const("3")),
                "const(true)",
            ),
            (
                binary(BinaryOp::GreaterEqual, int_const("2"), int_const("3")),
                "const(false)",
            ),
            (
                binary(BinaryOp::Equal, boolean("true"), boolean("true")),
                "const(true)",
            ),
        ] {
            assert_eq!(folded(value, &scopes), expected);
        }
        // Boolean is comparable but not orderable: no `<` fold.
        let unordered = binary(BinaryOp::Less, boolean("false"), boolean("true"));
        let rendered = shape(&unordered);
        assert_eq!(folded(unordered, &scopes), rendered);
    }

    /// Float, Fixed, and Money never fold (naive-parser round-trip; rounding
    /// modes), and neither do mixed-type pairings (§4.1 promotion).
    #[test]
    fn excluded_domains_never_fold() {
        let scopes = Scopes::new();
        for value in [
            binary(
                BinaryOp::Add,
                typed_const(ParameterType::Float, "1.5"),
                typed_const(ParameterType::Float, "2.5"),
            ),
            binary(
                BinaryOp::Less,
                typed_const(ParameterType::Float, "1.5"),
                typed_const(ParameterType::Float, "2.5"),
            ),
            binary(
                BinaryOp::Add,
                typed_const(ParameterType::Byte, "1"),
                int_const("2"),
            ),
        ] {
            let rendered = shape(&value);
            assert_eq!(folded(value, &scopes), rendered);
        }
    }

    #[test]
    fn a_non_expression_value_is_unchanged() {
        let scopes = Scopes::new();
        assert_eq!(folded(int_const("7"), &scopes), "const(7)");
    }
}
