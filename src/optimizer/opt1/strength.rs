//! Strength reduction (non-loop) — a Level-1 Opt1 catalog row
//! (`planning/optimizations.md`): replace an expensive checked operation with a
//! cheaper one computing the same value *and the same trap*. Driven per node by
//! [`super::local_rewrites`].
//!
//! Under MFB's checked arithmetic the cost of an Integer/Byte operator is
//! dominated by its overflow check, and a rewrite is admissible only when the
//! replacement's trap set, error code, and source stamp are identical — all
//! three ops involved here raise through the same `raise_error_bare("ErrOverflow")`
//! (`builder_numeric.rs`) and the rewritten node keeps its original `loc`, so
//! the error is byte-identical. The rules:
//!
//! - **`x * 2` / `2 * x` → `x + x`.** A checked add (`adds` + `b.vc`) replaces
//!   the checked-multiply sequence (`smulh` + `mul` + `asr` + `cmp` + branch).
//!   Trap-equivalent: both raise exactly when `2x` leaves the type's range
//!   (Byte's shared >255 check included).
//! - **`x ^ 2` → `x * x`.** One checked multiply replaces the whole
//!   `emit_integer_pow` preamble + loop. Trap-equivalent by construction: the
//!   loop computes `(1 * x) * x`, whose first multiply can neither trap nor
//!   round, and the exponent-2 constant can never trip pow's negative-exponent
//!   check.
//!
//! Both rules require the variable operand to be one of the pure, cheaply
//! re-evaluable leaves (`Const`/`Local`/`Global`/`Capture`) — duplicating any
//! other shape would re-run effects or re-trap — and to have the same known
//! type as the constant, else dropping the operator would change the §4.1
//! promoted result type (the same gate as the algebraic row; the driver's
//! `scopes_type_is` answers only for exactly those leaf shapes, so one check
//! covers duplicability and typing). Scalar-typed rewrites only: duplicating a
//! `Local` read of an Integer/Byte touches no ownership machinery.
//!
//! Deliberately out of scope: `x / 2^k` → shift (signed division truncates
//! toward zero, a shift rounds toward −∞ — that fixup is the
//! "Division-by-constant lowering" codegen row); Float `x / C` → `x * (1/C)`
//! (the NIR Float constant would round-trip through a text literal the backend
//! parses — see the `toFloat` rounding caveat — so it waits for a bit-exact
//! constant channel); higher powers (`x ^ 3` → chained multiplies is equally
//! trap-faithful but code-expanding — revisit with a cost model).

use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;

use super::local_rewrites::{scopes_type_is, Scopes};

/// Try the row's rewrites at one node (children already rewritten). Returns
/// whether a rewrite fired, for the driver's `mfb build -v` fire count.
pub(super) fn rewrite_value(value: &mut NirValue, scopes: &Scopes) -> bool {
    let NirValue::Binary {
        op, left, right, ..
    } = value
    else {
        return false;
    };
    match op.as_str() {
        "*" => {
            // x * 2 → x + x (and mirrored).
            if let Some(const_type) = two_const(right) {
                if scopes_type_is(left, &const_type, scopes) {
                    *op = "+".to_string();
                    **right = (**left).clone();
                    return true;
                }
            }
            if let Some(const_type) = two_const(left) {
                if scopes_type_is(right, &const_type, scopes) {
                    *op = "+".to_string();
                    **left = (**right).clone();
                    return true;
                }
            }
            false
        }
        "^" => {
            // x ^ 2 → x * x. Exponent-side only: `2 ^ x` has no cheap form.
            if let Some(const_type) = two_const(right) {
                if scopes_type_is(left, &const_type, scopes) {
                    *op = "*".to_string();
                    **right = (**left).clone();
                    return true;
                }
            }
            false
        }
        _ => false,
    }
}

/// If `value` is the Integer/Byte constant `2`, its type (which the variable
/// operand must match — same §4.1 promotion gate as the algebraic row). Float
/// stays out: `x * 2.0` → `x + x` swaps fmul for fadd at no checked-op saving,
/// and `x ^ 2.0` rides the pow kernel whose rounding is not obligated to match
/// a bare multiply.
fn two_const(value: &NirValue) -> Option<ParameterType> {
    let NirValue::Const { type_, value } = value else {
        return None;
    };
    (matches!(type_, ParameterType::Integer | ParameterType::Byte)
        && value.parse::<i128>() == Ok(2))
    .then(|| type_.clone())
}

#[cfg(test)]
mod tests {
    use super::super::local_rewrites::testutil::*;
    use super::*;

    /// Apply this row alone (no walk — callers hand a leaf-simplified node).
    fn reduced(mut value: NirValue, scopes: &Scopes) -> String {
        rewrite_value(&mut value, scopes);
        shape(&value)
    }

    #[test]
    fn multiply_by_two_becomes_add() {
        let scopes = int_scope("x");
        for value in [
            binary("*", local("x"), int_const("2")),
            binary("*", int_const("2"), local("x")),
        ] {
            assert_eq!(reduced(value, &scopes), "(local(x) + local(x))");
        }
        // Byte, with a Byte-typed constant.
        let mut scopes = Scopes::new();
        scopes.insert("b".to_string(), ParameterType::Byte);
        let byte_two = typed_const(ParameterType::Byte, "2");
        let value = binary("*", local("b"), byte_two);
        assert_eq!(reduced(value, &scopes), "(local(b) + local(b))");
    }

    #[test]
    fn square_becomes_multiply() {
        let scopes = int_scope("x");
        let value = binary("^", local("x"), int_const("2"));
        assert_eq!(reduced(value, &scopes), "(local(x) * local(x))");
    }

    /// Duplicating a non-leaf operand would re-run its effects (a call) or
    /// re-trap, so only the pure leaf shapes rewrite.
    #[test]
    fn non_duplicable_operands_are_never_rewritten() {
        let scopes = int_scope("x");
        let call = NirValue::Call {
            target: "f".to_string(),
            args: vec![],
            loc: crate::target::shared::nir::NirSourceLoc::default(),
        };
        for value in [
            binary("*", call.clone(), int_const("2")),
            binary("^", call, int_const("2")),
            // Nested arithmetic is pure but would double the checked work —
            // and its type is unknown to the scope walk anyway.
            binary("*", binary("+", local("x"), local("x")), int_const("2")),
        ] {
            let rendered = shape(&value);
            assert_eq!(reduced(value, &scopes), rendered);
        }
    }

    /// The §4.1 promotion gate: `byte * integer-2` is an Integer expression, so
    /// rewriting it to `byte + byte` would retype it. Other constants and other
    /// exponents are not this row's business.
    #[test]
    fn mixed_types_and_other_constants_stay() {
        let mut scopes = Scopes::new();
        scopes.insert("b".to_string(), ParameterType::Byte);
        scopes.insert("x".to_string(), ParameterType::Integer);
        for value in [
            binary("*", local("b"), int_const("2")),
            binary("*", local("x"), int_const("3")),
            binary("^", local("x"), int_const("3")),
            binary("^", int_const("2"), local("x")),
            binary("*", local("f"), typed_const(ParameterType::Float, "2.0")),
        ] {
            let rendered = shape(&value);
            assert_eq!(reduced(value, &scopes), rendered);
        }
    }
}
