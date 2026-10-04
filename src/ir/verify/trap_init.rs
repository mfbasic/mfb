//! bug-797: a function-level `TRAP` handler may read only the locals that are
//! initialized on **every** failure edge reaching it.
//!
//! The handler can be entered from any failure in the body before it, and a
//! failure in a declaration's own right-hand side happens before that local is
//! bound. A top-level local declared at op `i` is therefore safe in the handler
//! exactly when nothing in ops `0..=i` can fail. A parameter is always
//! initialized. A failed reassignment leaves the earlier value in place, so only
//! declarations matter, never later `Assign`s.
//!
//! "Can fail" mirrors the source-side fallibility census (`ir::fallible`) on IR,
//! so the source build and a decoded package answer alike:
//!
//! * `FAIL` fails.
//! * A raising arithmetic operator fails (`operator_can_raise`, with the
//!   negative-literal exemption), and so does a comparison that promotes an
//!   operand into `Fixed` (`comparison_can_raise`, bug-816).
//! * A call fails unless the built-in census proves it total, or it names a
//!   project function this walk proves cannot let an error out. An imported or
//!   unknown callee is assumed to fail.
//! * A call or operator under an inline `TRAP` (`CallResult`, `Checked`) is
//!   captured there, not an edge into this handler. That inline handler's own
//!   `FAIL`/`PROPAGATE` is an ordinary nested `FAIL` op.
//! * A lambda's body runs where it is called, so creating one is not an edge.
//!
//! The census over-approximates where it must guess. That is the safe side here:
//! it can only make a read *less* likely to be accepted.

use std::collections::{HashMap, HashSet};

use crate::codegen::builtins;
use crate::ir::fallible::{
    comparison_can_raise, is_total_literal_negation, operator_can_raise, unary_operator_can_raise,
};
use crate::ir::{IrFunction, IrMatchPattern, IrOp, IrValue};
use crate::types::ParameterType;

/// Project functions that cannot let an error reach their caller, solved to a
/// fixpoint. A function's *relevant block* is its `TRAP` handler when it has
/// one (the body's errors are routed there), else its body.
pub(super) fn total_functions(
    functions: &[IrFunction],
    globals: &HashMap<String, ParameterType>,
) -> HashSet<String> {
    let declared: HashSet<String> = functions.iter().map(|f| f.name.clone()).collect();
    // Start from "every project function is total" and remove the ones that are
    // shown to fail, until nothing changes.
    let mut total = declared.clone();
    loop {
        let mut changed = false;
        for function in functions {
            if !total.contains(&function.name) {
                continue;
            }
            let relevant = match function.body.last() {
                Some(IrOp::Trap { body, .. }) => body.as_slice(),
                _ => function.body.as_slice(),
            };
            let mut types = param_types(function);
            let census = Census {
                total: &total,
                declared: &declared,
                globals,
            };
            if census.ops_may_fail(relevant, &mut types) {
                total.remove(&function.name);
                changed = true;
            }
        }
        if !changed {
            return total;
        }
    }
}

/// The top-level locals of `body` (the ops before a function-level `TRAP`) that
/// some failure edge into the handler reaches before they are bound.
///
/// `types` starts as the types visible at the handler (parameters and the body's
/// declarations); a call's argument types are read from it.
pub(super) fn locals_unbound_on_some_failure(
    body: &[IrOp],
    types: &HashMap<String, ParameterType>,
    declared: &HashSet<String>,
    total: &HashSet<String>,
    globals: &HashMap<String, ParameterType>,
) -> HashSet<String> {
    let census = Census {
        total,
        declared,
        globals,
    };
    let mut types = types.clone();
    let mut failed_already = false;
    let mut unbound = HashSet::new();
    for op in body {
        // Judged before the bind is recorded: a failure in a declaration's own
        // right-hand side is an edge on which that local is not yet bound.
        failed_already |= census.op_may_fail(op, &mut types);
        if failed_already {
            if let IrOp::Bind { name, .. } = op {
                unbound.insert(name.clone());
            }
        }
    }
    unbound
}

/// Every name in `names` that the handler reads, in first-read order.
pub(super) fn handler_reads(handler: &[IrOp], names: &HashSet<String>) -> Vec<String> {
    let mut found = Vec::new();
    let mut note = |value: &IrValue| {
        let name = match value {
            IrValue::Local(name) | IrValue::LocalRef { name, .. } => name,
            _ => return,
        };
        if names.contains(name) && !found.contains(name) {
            found.push(name.clone());
        }
    };
    for_each_value_in_ops(handler, &mut note);
    found
}

fn param_types(function: &IrFunction) -> HashMap<String, ParameterType> {
    function
        .params
        .iter()
        .map(|param| (param.name.clone(), param.type_.clone()))
        .collect()
}

struct Census<'a> {
    total: &'a HashSet<String>,
    declared: &'a HashSet<String>,
    /// Global types, for a comparison operand that reads a global.
    globals: &'a HashMap<String, ParameterType>,
}

impl Census<'_> {
    fn ops_may_fail(&self, ops: &[IrOp], types: &mut HashMap<String, ParameterType>) -> bool {
        // Not short-circuited: a later op's binds still need their types.
        let mut any = false;
        for op in ops {
            any |= self.op_may_fail(op, types);
        }
        any
    }

    fn op_may_fail(&self, op: &IrOp, types: &mut HashMap<String, ParameterType>) -> bool {
        match op {
            IrOp::Fail { .. } => true,
            IrOp::Bind {
                name, type_, value, ..
            } => {
                let fails = value
                    .as_ref()
                    .is_some_and(|v| self.value_may_fail(v, types));
                types.insert(name.clone(), type_.clone());
                fails
            }
            IrOp::Assign { value, .. }
            | IrOp::AssignGlobal { value, .. }
            | IrOp::StateAssign { value, .. }
            | IrOp::Eval { value, .. } => self.value_may_fail(value, types),
            IrOp::Return { value, .. } => value
                .as_ref()
                .is_some_and(|v| self.value_may_fail(v, types)),
            IrOp::ExitProgram { code, .. } => self.value_may_fail(code, types),
            IrOp::ExitLoop { .. } | IrOp::ContinueLoop { .. } => false,
            IrOp::If {
                condition,
                then_body,
                else_body,
                ..
            } => {
                let condition = self.value_may_fail(condition, types);
                let then_fails = self.ops_may_fail(then_body, &mut types.clone());
                let else_fails = self.ops_may_fail(else_body, &mut types.clone());
                condition || then_fails || else_fails
            }
            IrOp::Match { value, cases, .. } => {
                let mut fails = self.value_may_fail(value, types);
                for case in cases {
                    match &case.pattern {
                        IrMatchPattern::Else => {}
                        IrMatchPattern::Value(pattern) => {
                            fails |= self.value_may_fail(pattern, types)
                        }
                        IrMatchPattern::OneOf(patterns) => {
                            for pattern in patterns {
                                fails |= self.value_may_fail(pattern, types);
                            }
                        }
                    }
                    if let Some(guard) = &case.guard {
                        fails |= self.value_may_fail(guard, types);
                    }
                    fails |= self.ops_may_fail(&case.body, &mut types.clone());
                }
                fails
            }
            IrOp::While {
                condition, body, ..
            }
            | IrOp::DoUntil {
                condition, body, ..
            } => {
                let condition = self.value_may_fail(condition, types);
                condition || self.ops_may_fail(body, &mut types.clone())
            }
            // The counter's increment is checked arithmetic and can overflow.
            IrOp::For { .. } => true,
            IrOp::ForEach {
                name,
                type_,
                iterable,
                body,
                ..
            } => {
                let iterable = self.value_may_fail(iterable, types);
                let mut inner = types.clone();
                inner.insert(name.clone(), type_.clone());
                iterable || self.ops_may_fail(body, &mut inner)
            }
            // A nested function-level TRAP does not occur; a handler's own
            // failures leave the function, which is what the census asks.
            IrOp::Trap { body, .. } => self.ops_may_fail(body, &mut types.clone()),
        }
    }

    fn value_may_fail(&self, value: &IrValue, types: &HashMap<String, ParameterType>) -> bool {
        let any = |values: &mut dyn Iterator<Item = &IrValue>| {
            values.fold(false, |acc, v| acc | self.value_may_fail(v, types))
        };
        match value {
            IrValue::Const { .. }
            | IrValue::Local(_)
            | IrValue::Global(_)
            | IrValue::LocalRef { .. }
            | IrValue::FunctionRef { .. }
            | IrValue::Capture { .. } => false,
            IrValue::Closure { captures, .. } => any(&mut captures.iter()),
            IrValue::Call { target, args, .. } => {
                self.call_may_fail(target, args, types) || any(&mut args.iter())
            }
            // The call's own error is captured by the inline TRAP; its arguments
            // are still evaluated here.
            IrValue::CallResult { args, .. } => any(&mut args.iter()),
            IrValue::Checked { .. } => false,
            IrValue::Constructor { args, .. } => any(&mut args.iter()),
            IrValue::UnionWrap { value, .. }
            | IrValue::UnionExtract { value, .. }
            | IrValue::ResultIsOk { value }
            | IrValue::ResultValue { value, .. }
            | IrValue::ResultError { value }
            | IrValue::MemberAccess { target: value, .. } => self.value_may_fail(value, types),
            IrValue::WithUpdate {
                target, updates, ..
            } => {
                self.value_may_fail(target, types)
                    || any(&mut updates.iter().map(|update| &update.value))
            }
            IrValue::ListLiteral { values, .. } | IrValue::SetLiteral { values, .. } => {
                any(&mut values.iter())
            }
            IrValue::MapLiteral { entries, .. } => {
                any(&mut entries.iter().flat_map(|(key, value)| [key, value]))
            }
            IrValue::Binary {
                op,
                type_,
                left,
                right,
                ..
            } => {
                operator_can_raise(*op, type_)
                    || comparison_can_raise(
                        *op,
                        &self.operand_type(left, types),
                        &self.operand_type(right, types),
                    )
                    || self.value_may_fail(left, types)
                    || self.value_may_fail(right, types)
            }
            IrValue::Unary {
                op, type_, operand, ..
            } => {
                let literal = matches!(operand.as_ref(), IrValue::Const { .. })
                    && is_total_literal_negation(*op, type_);
                (unary_operator_can_raise(*op, type_) && !literal)
                    || self.value_may_fail(operand, types)
            }
        }
    }

    /// An operand's type: a name from `types` or the globals, any other node
    /// from its own annotation, `Unknown` when neither answers.
    fn operand_type(
        &self,
        value: &IrValue,
        types: &HashMap<String, ParameterType>,
    ) -> ParameterType {
        match value {
            IrValue::Local(name) => types.get(name).cloned(),
            IrValue::Global(name) => self.globals.get(name).cloned(),
            other => other.annotated_parameter_type(),
        }
        .unwrap_or(ParameterType::Unknown)
    }

    fn call_may_fail(
        &self,
        target: &str,
        args: &[IrValue],
        types: &HashMap<String, ParameterType>,
    ) -> bool {
        if builtins::is_package_constant(target) {
            return false;
        }
        let arg_types: Vec<ParameterType> = args
            .iter()
            .map(|arg| {
                match arg {
                    IrValue::Local(name) => types.get(name).cloned(),
                    other => other.annotated_parameter_type(),
                }
                .unwrap_or(ParameterType::Unknown)
            })
            .collect();
        if builtins::inline_builtin_is_infallible(target, &arg_types, &builtins::NoTypeKinds) {
            return false;
        }
        if self.declared.contains(target) {
            return !self.total.contains(target);
        }
        true
    }
}

/// Visit every value an op list evaluates, nested bodies included. A lambda's
/// body is a separate function and is not visited (`visit_value`'s contract).
fn for_each_value_in_ops(ops: &[IrOp], f: &mut impl FnMut(&IrValue)) {
    for op in ops {
        let mut each = |value: &IrValue| crate::ir::value::visit_value(value, f);
        match op {
            IrOp::Bind { value, .. } => {
                if let Some(value) = value {
                    each(value);
                }
            }
            IrOp::Assign { value, .. }
            | IrOp::AssignGlobal { value, .. }
            | IrOp::StateAssign { value, .. }
            | IrOp::Eval { value, .. }
            | IrOp::Fail { error: value, .. }
            | IrOp::ExitProgram { code: value, .. } => each(value),
            IrOp::Return { value, .. } => {
                if let Some(value) = value {
                    each(value);
                }
            }
            IrOp::ExitLoop { .. } | IrOp::ContinueLoop { .. } => {}
            IrOp::If {
                condition,
                then_body,
                else_body,
                ..
            } => {
                each(condition);
                for_each_value_in_ops(then_body, f);
                for_each_value_in_ops(else_body, f);
            }
            IrOp::Match { value, cases, .. } => {
                each(value);
                for case in cases {
                    match &case.pattern {
                        IrMatchPattern::Else => {}
                        IrMatchPattern::Value(pattern) => crate::ir::value::visit_value(pattern, f),
                        IrMatchPattern::OneOf(patterns) => {
                            for pattern in patterns {
                                crate::ir::value::visit_value(pattern, f);
                            }
                        }
                    }
                    if let Some(guard) = &case.guard {
                        crate::ir::value::visit_value(guard, f);
                    }
                    for_each_value_in_ops(&case.body, f);
                }
            }
            IrOp::While {
                condition, body, ..
            }
            | IrOp::DoUntil {
                condition, body, ..
            } => {
                each(condition);
                for_each_value_in_ops(body, f);
            }
            IrOp::For {
                start,
                end,
                step,
                body,
                ..
            } => {
                each(start);
                each(end);
                each(step);
                for_each_value_in_ops(body, f);
            }
            IrOp::ForEach { iterable, body, .. } => {
                each(iterable);
                for_each_value_in_ops(body, f);
            }
            IrOp::Trap { body, .. } => for_each_value_in_ops(body, f),
        }
    }
}
