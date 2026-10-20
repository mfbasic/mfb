//! plan-176-U: the value effect predicates — may-raise, effect-free, cannot-fail and infallible
//! builtins. The statement entry (`lower_ops`) and shared types live in
//! `engine/control/builder_control/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// plan-145-C: whether the field arm for `target` can raise beyond its
    /// operands — an index out of range. Only the operations known not to are
    /// `false`; anything else answers `true`.
    pub(super) fn field_arm_may_raise(target: &str, field_type: &ParameterType) -> bool {
        match crate::codegen::collection::assign::self_update::self_update_builtin(target) {
            Some("append" | "add" | "prepend" | "removeKey" | "remove") => false,
            // `set` raises on a `List` index, never on a `Map` key.
            Some("set") => typed_list_element_type(field_type).is_some(),
            _ => true,
        }
    }

    /// plan-145-C: `value` has no observable effect: it reads locals, globals and
    /// fields, computes on them, and calls only built-ins that cannot fail (the
    /// pure queries of `inline_builtin_is_infallible`). A user function might
    /// print, so any other call has an effect.
    pub(crate) fn nir_value_is_effect_free(&self, value: &NirValue) -> bool {
        match value {
            NirValue::Const { .. }
            | NirValue::Local(_)
            | NirValue::Global { .. }
            | NirValue::Capture { .. }
            | NirValue::LocalRef { .. } => true,
            NirValue::MemberAccess { target, .. } => self.nir_value_is_effect_free(target),
            NirValue::Binary { left, right, .. } => {
                self.nir_value_is_effect_free(left) && self.nir_value_is_effect_free(right)
            }
            NirValue::Unary { operand, .. } => self.nir_value_is_effect_free(operand),
            NirValue::Call { .. } | NirValue::RuntimeCall { .. } => {
                self.nir_call_is_infallible_builtin(value)
            }
            _ => false,
        }
    }

    /// plan-145-C: `value` cannot raise: reads, comparisons and logic over values
    /// that cannot raise, and infallible built-ins. Arithmetic can overflow and a
    /// negation can too (the minimum `Integer`), so neither qualifies.
    pub(crate) fn nir_value_cannot_fail(&self, value: &NirValue) -> bool {
        use crate::operators::{BinaryOp, UnaryOp};
        match value {
            NirValue::Const { .. }
            | NirValue::Local(_)
            | NirValue::Global { .. }
            | NirValue::Capture { .. }
            | NirValue::LocalRef { .. } => true,
            NirValue::MemberAccess { target, .. } => self.nir_value_cannot_fail(target),
            NirValue::Binary {
                op, left, right, ..
            } => {
                matches!(
                    op,
                    BinaryOp::Equal
                        | BinaryOp::NotEqual
                        | BinaryOp::Less
                        | BinaryOp::LessEqual
                        | BinaryOp::Greater
                        | BinaryOp::GreaterEqual
                        | BinaryOp::And
                        | BinaryOp::Or
                ) && self.nir_value_cannot_fail(left)
                    && self.nir_value_cannot_fail(right)
                    && self.nir_comparison_promotion_cannot_fail(*op, left, right)
            }
            NirValue::Unary { op, operand, .. } => {
                *op == UnaryOp::Not && self.nir_value_cannot_fail(operand)
            }
            NirValue::Call { .. } | NirValue::RuntimeCall { .. } => {
                self.nir_call_is_infallible_builtin(value)
            }
            _ => false,
        }
    }

    /// bug-816: a comparison promotes its operands as arithmetic does (§4.1), and
    /// promotion into `Fixed` is checked — the IR census's own rule,
    /// `ir::fallible::comparison_can_raise`. An unproven operand type is
    /// `Unknown`, the side that only declines an optimization.
    fn nir_comparison_promotion_cannot_fail(
        &self,
        op: crate::operators::BinaryOp,
        left: &NirValue,
        right: &NirValue,
    ) -> bool {
        let type_of = |value: &NirValue| {
            self.static_item_type(value)
                .unwrap_or(ParameterType::Unknown)
        };
        !crate::ir::fallible::comparison_can_raise(op, &type_of(left), &type_of(right))
    }

    /// A call to a built-in that cannot fail, over arguments that cannot fail and
    /// whose types are known (the census is keyed on them — `toString` of a `List
    /// OF Byte` decodes UTF-8 and can fail).
    pub(super) fn nir_call_is_infallible_builtin(&self, value: &NirValue) -> bool {
        let (NirValue::Call { target, args, .. } | NirValue::RuntimeCall { target, args, .. }) =
            value
        else {
            return false;
        };
        let Some(arg_types) = args
            .iter()
            .map(|arg| self.static_item_type(arg))
            .collect::<Option<Vec<_>>>()
        else {
            return false;
        };
        // bug-679: `NoTypeKinds` deliberately. This is a codegen optimizer running
        // on NIR, where no declaration table is in scope to tell an enum from a
        // record, and the oracle's `false` is the over-approximating side — a
        // `toInt(<enum>)` here simply stays "can fail" and the value is not
        // treated as effect-free. Losing an optimization is the safe direction;
        // the other one would let a failing call be elided.
        crate::codegen::builtins::inline_builtin_is_infallible(
            target,
            &arg_types,
            &crate::codegen::builtins::NoTypeKinds,
        ) && args.iter().all(|arg| self.nir_value_cannot_fail(arg))
    }
}
