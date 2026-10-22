//! Literal coercion at a GENERIC built-in parameter.
//!
//! An unsuffixed numeric literal adopts the type of the slot it is written in
//! (`mfb spec architecture type-inference`, "Literal Coercion"): `LET b AS Byte
//! = 7`, a user `FUNC` taking `Byte`, and an element of a typed `List OF Byte`
//! literal all take `7` as a `Byte`. At a generic built-in parameter the slot's
//! type is only known once the registry's overload selection has unified the
//! call — and that selection sees the literal as its intrinsic `Integer` (or
//! `Float`), so `collections::append(bytes, 7)` pinned `T := Byte` from the list
//! and then rejected the `Integer` item. This module is the retry both the shape
//! pass (validation) and lowering (expected type and result type) run when that
//! exact-typed selection fails: re-type literal arguments toward the types a
//! literal can coerce to and ask the registry again.

use super::*;

/// Most literal positions one call may re-type: every combination of their
/// candidates is tried, so the bound keeps the retry's worst case small
/// (4^6 selections) on a call that is already failing.
const MAX_RETYPED_POSITIONS: usize = 6;

/// The digits of a numeric literal written plainly or negated.
fn literal_number(expression: &HirExpression) -> Option<&str> {
    match expression {
        HirExpression::Number(text) => Some(text),
        HirExpression::Unary {
            operator, operand, ..
        } if *operator == UnaryOp::Negate => match operand.as_ref() {
            HirExpression::Number(text) => Some(text),
            _ => None,
        },
        _ => None,
    }
}

/// The types a numeric literal spelled `text` may be re-typed to, in trial
/// order. These are the slot types of `expression_compatible`'s literal arms.
/// `Byte` is offered for any integer literal, in range or not: selection only
/// decides the slot, and the range is then judged against that slot by the
/// caller — so `append(bytes, 300)` is the argument mismatch a built-in's
/// monomorphic Byte parameter gives (`toString(1.5, 300)`), and an out-of-range
/// element of `append(bytes, [4, 256])` lowers against `Byte` and reports the
/// Byte overflow a typed list literal reports. A suffixed literal keeps its type,
/// except that an `f`-suffixed one converts into a Fixed slot (never Money,
/// bug-820).
fn number_candidates(text: &str) -> Vec<ParameterType> {
    match crate::numeric::classify_literal(text).1 {
        crate::numeric::LiteralType::Integer => {
            vec![
                ParameterType::Byte,
                ParameterType::Fixed,
                ParameterType::Money,
            ]
        }
        crate::numeric::LiteralType::Float if text.ends_with('f') => vec![ParameterType::Fixed],
        crate::numeric::LiteralType::Float => vec![ParameterType::Fixed, ParameterType::Money],
        crate::numeric::LiteralType::Fixed | crate::numeric::LiteralType::Money => Vec::new(),
    }
}

/// The types a literal argument may be re-typed to; empty for anything that is
/// not a literal shape `expression_compatible` coerces. A non-empty list
/// literal of numeric literals re-types element-wise: its candidates are the
/// element types every element can take, wrapped in a list.
fn literal_candidates(expression: &HirExpression) -> Vec<ParameterType> {
    if let Some(text) = literal_number(expression) {
        return number_candidates(text);
    }
    let HirExpression::ListLiteral(values) = expression else {
        return Vec::new();
    };
    let mut shared: Option<Vec<ParameterType>> = None;
    for value in values {
        let Some(text) = literal_number(value) else {
            return Vec::new();
        };
        let candidates = number_candidates(text);
        shared = Some(match shared {
            None => candidates,
            Some(seen) => seen
                .into_iter()
                .filter(|type_| candidates.contains(type_))
                .collect(),
        });
    }
    shared
        .unwrap_or_default()
        .into_iter()
        .map(|element| ParameterType::ListOf(Box::new(element)))
        .collect()
}

/// The argument types under which the generic built-in `canonical_callee`
/// selects an overload once literal arguments are re-typed toward their
/// coercion targets, or `None` when no retry applies.
///
/// Only literals at a GENERIC position are re-typed
/// (`registry::generic_parameter_position`): a concrete overload set such as
/// `math::cos`'s `Float` | `Fixed` keeps exact selection, so `math::cos(1)`
/// stays a mismatch rather than quietly selecting `Fixed`.
///
/// The retry runs only for a call whose exact-typed selection FAILS, so a call
/// that already selects is never re-typed (and nothing that compiles today
/// changes meaning). Each literal position is tried at its own type first and
/// then at each candidate, positions varying odometer-style; the first
/// combination that selects wins. Selection is strict — the shape pass's
/// validation rule — and the caller still checks each re-typed literal against
/// its slot (`expression_compatible`: the Byte range, the Money suffix rule).
pub(crate) fn literal_retyped_argument_types(
    canonical_callee: &str,
    arg_types: &[ParameterType],
    arguments: &[&HirExpression],
    kinds: &dyn builtins::TypeKinds,
) -> Option<Vec<ParameterType>> {
    if builtins::resolve_call_return_type_with_kinds(canonical_callee, arg_types, true, kinds)
        .is_some()
    {
        return None;
    }
    let positions: Vec<(usize, Vec<ParameterType>)> = arguments
        .iter()
        .enumerate()
        .take(arg_types.len())
        .filter(|(index, _)| {
            crate::codegen::registry::generic_parameter_position(canonical_callee, *index)
        })
        .filter_map(|(index, argument)| {
            let candidates = literal_candidates(argument);
            (!candidates.is_empty()).then_some((index, candidates))
        })
        .collect();
    if positions.is_empty() || positions.len() > MAX_RETYPED_POSITIONS {
        return None;
    }
    // Choice 0 at a position keeps the argument's own type; choice k > 0 is its
    // (k-1)-th candidate. The all-zero combination is the failed selection.
    let mut choice = vec![0usize; positions.len()];
    loop {
        let mut slot = 0;
        loop {
            if slot == positions.len() {
                return None;
            }
            choice[slot] += 1;
            if choice[slot] <= positions[slot].1.len() {
                break;
            }
            choice[slot] = 0;
            slot += 1;
        }
        let mut trial = arg_types.to_vec();
        for ((index, candidates), &pick) in positions.iter().zip(&choice) {
            if pick > 0 {
                trial[*index] = candidates[pick - 1].clone();
            }
        }
        if builtins::resolve_call_return_type_with_kinds(canonical_callee, &trial, true, kinds)
            .is_some()
        {
            return Some(trial);
        }
    }
}
