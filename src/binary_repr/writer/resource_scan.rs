//! plan-176-O: the `.mfp` writer's resource scan — which resource types a project and its ops
//! use. Project lowering and `BinaryReprProject`'s encoder live in `binary_repr/writer/mod.rs`.

use super::*;

pub(in crate::binary_repr) fn ir_uses_resource_type(ir: &IrProject) -> bool {
    ir.functions.iter().any(|function| {
        function
            .params
            .iter()
            .any(|param| is_resource_type_name(&param.type_.name()))
            || is_resource_type_name(&function.returns.name())
            || ops_use_resource_type(&function.body)
    })
}

pub(in crate::binary_repr) fn ops_use_resource_type(ops: &[IrOp]) -> bool {
    ops.iter().any(|op| match op {
        IrOp::Bind { type_, value, .. } => {
            is_resource_type_name(&type_.name())
                || value.as_ref().is_some_and(value_uses_resource_type)
        }
        IrOp::Assign { value, .. }
        | IrOp::AssignGlobal { value, .. }
        | IrOp::StateAssign { value, .. }
        | IrOp::Eval { value, .. } => value_uses_resource_type(value),
        IrOp::Return { value, .. } => value.as_ref().is_some_and(value_uses_resource_type),
        IrOp::ExitLoop { .. } | IrOp::ContinueLoop { .. } => false,
        IrOp::ExitProgram { code, .. } => value_uses_resource_type(code),
        IrOp::Fail { error, .. } => value_uses_resource_type(error),
        IrOp::If {
            condition,
            then_body,
            else_body,
            ..
        } => {
            value_uses_resource_type(condition)
                || ops_use_resource_type(then_body)
                || ops_use_resource_type(else_body)
        }
        IrOp::Match { value, cases, .. } => {
            value_uses_resource_type(value)
                || cases.iter().any(|case| ops_use_resource_type(&case.body))
        }
        IrOp::While {
            condition, body, ..
        } => value_uses_resource_type(condition) || ops_use_resource_type(body),
        IrOp::For {
            start,
            end,
            step,
            body,
            ..
        } => {
            value_uses_resource_type(start)
                || value_uses_resource_type(end)
                || value_uses_resource_type(step)
                || ops_use_resource_type(body)
        }
        IrOp::DoUntil {
            body, condition, ..
        } => ops_use_resource_type(body) || value_uses_resource_type(condition),
        IrOp::ForEach {
            type_,
            iterable,
            body,
            ..
        } => {
            is_resource_type_name(&type_.name())
                || value_uses_resource_type(iterable)
                || ops_use_resource_type(body)
        }
        IrOp::Trap { body, .. } => ops_use_resource_type(body),
    })
}

pub(in crate::binary_repr) fn value_uses_resource_type(value: &IrValue) -> bool {
    match value {
        IrValue::Const { type_, .. }
        | IrValue::LocalRef { type_, .. }
        | IrValue::FunctionRef { type_, .. }
        | IrValue::Closure { type_, .. }
        | IrValue::Capture { type_, .. }
        | IrValue::Constructor { type_, .. }
        | IrValue::ListLiteral { type_, .. }
        | IrValue::SetLiteral { type_, .. }
        | IrValue::MapLiteral { type_, .. } => is_resource_type_name(&type_.name()),
        IrValue::Call { target, args, .. } | IrValue::CallResult { target, args, .. } => {
            builtins::call_return_type_name(target)
                .as_deref()
                .is_some_and(is_resource_type_name)
                || args.iter().any(value_uses_resource_type)
        }
        IrValue::UnionWrap { value, .. }
        | IrValue::UnionExtract { value, .. }
        | IrValue::ResultIsOk { value }
        | IrValue::ResultValue { value, .. }
        | IrValue::ResultError { value }
        | IrValue::Checked { value, .. } => value_uses_resource_type(value),
        IrValue::MemberAccess { target, .. } => value_uses_resource_type(target),
        IrValue::WithUpdate {
            target, updates, ..
        } => {
            value_uses_resource_type(target)
                || updates
                    .iter()
                    .any(|update| value_uses_resource_type(&update.value))
        }
        IrValue::Binary { left, right, .. } => {
            value_uses_resource_type(left) || value_uses_resource_type(right)
        }
        IrValue::Unary { operand, .. } => value_uses_resource_type(operand),
        IrValue::Local(_) | IrValue::Global(_) => false,
    }
}

pub(in crate::binary_repr) fn is_resource_type_name(type_name: &str) -> bool {
    // A wire-section type NAME (boundary file #4), classified through the one
    // grammar rather than a second copy of it.
    builtins::is_resource_type(&crate::types::ParameterType::declared(type_name))
}

/// Collect the package-qualified resource type identities (`fs.File`,
/// `tcp.Socket`, `tcp.Listener`) actually referenced by the project so only the
/// resource tables that are used get emitted. Resource handles cannot appear
/// inside collections, so a resource type string is always a single type
/// identity (never a `List OF ...` wrapper).
pub(in crate::binary_repr) fn collect_resource_type_names(
    ir: &IrProject,
    names: &mut HashSet<String>,
) {
    let mut record = |type_: &str, names: &mut HashSet<String>| {
        if is_resource_type_name(type_) {
            names.insert(type_.to_string());
        }
    };
    for function in &ir.functions {
        for param in &function.params {
            record(&param.type_.name(), names);
        }
        record(&function.returns.name(), names);
        collect_resource_names_in_ops(&function.body, names, &mut record);
    }
}

pub(in crate::binary_repr) fn collect_resource_names_in_ops(
    ops: &[IrOp],
    names: &mut HashSet<String>,
    record: &mut impl FnMut(&str, &mut HashSet<String>),
) {
    for op in ops {
        match op {
            IrOp::Bind { type_, value, .. } => {
                record(&type_.name(), names);
                if let Some(value) = value {
                    collect_resource_names_in_value(value, names, record);
                }
            }
            IrOp::Assign { value, .. }
            | IrOp::AssignGlobal { value, .. }
            | IrOp::StateAssign { value, .. }
            | IrOp::Eval { value, .. } => collect_resource_names_in_value(value, names, record),
            IrOp::Return { value, .. } => {
                if let Some(value) = value {
                    collect_resource_names_in_value(value, names, record);
                }
            }
            IrOp::ExitLoop { .. } | IrOp::ContinueLoop { .. } => {}
            IrOp::ExitProgram { code, .. } => collect_resource_names_in_value(code, names, record),
            IrOp::Fail { error, .. } => collect_resource_names_in_value(error, names, record),
            IrOp::If {
                condition,
                then_body,
                else_body,
                ..
            } => {
                collect_resource_names_in_value(condition, names, record);
                collect_resource_names_in_ops(then_body, names, record);
                collect_resource_names_in_ops(else_body, names, record);
            }
            IrOp::Match { value, cases, .. } => {
                collect_resource_names_in_value(value, names, record);
                for case in cases {
                    collect_resource_names_in_ops(&case.body, names, record);
                }
            }
            IrOp::While {
                condition, body, ..
            } => {
                collect_resource_names_in_value(condition, names, record);
                collect_resource_names_in_ops(body, names, record);
            }
            IrOp::For {
                start,
                end,
                step,
                body,
                ..
            } => {
                collect_resource_names_in_value(start, names, record);
                collect_resource_names_in_value(end, names, record);
                collect_resource_names_in_value(step, names, record);
                collect_resource_names_in_ops(body, names, record);
            }
            IrOp::DoUntil {
                body, condition, ..
            } => {
                collect_resource_names_in_ops(body, names, record);
                collect_resource_names_in_value(condition, names, record);
            }
            IrOp::ForEach {
                type_,
                iterable,
                body,
                ..
            } => {
                record(&type_.name(), names);
                collect_resource_names_in_value(iterable, names, record);
                collect_resource_names_in_ops(body, names, record);
            }
            IrOp::Trap { body, .. } => collect_resource_names_in_ops(body, names, record),
        }
    }
}

pub(in crate::binary_repr) fn collect_resource_names_in_value(
    value: &IrValue,
    names: &mut HashSet<String>,
    record: &mut impl FnMut(&str, &mut HashSet<String>),
) {
    match value {
        IrValue::Const { type_, .. }
        | IrValue::LocalRef { type_, .. }
        | IrValue::FunctionRef { type_, .. }
        | IrValue::Closure { type_, .. }
        | IrValue::Capture { type_, .. }
        | IrValue::Constructor { type_, .. }
        | IrValue::ListLiteral { type_, .. }
        | IrValue::SetLiteral { type_, .. }
        | IrValue::MapLiteral { type_, .. } => record(&type_.name(), names),
        IrValue::Call { target, args, .. } | IrValue::CallResult { target, args, .. } => {
            if let Some(returns) = builtins::call_return_type_name(target) {
                record(&returns, names);
            }
            for arg in args {
                collect_resource_names_in_value(arg, names, record);
            }
        }
        IrValue::UnionWrap { value, .. }
        | IrValue::UnionExtract { value, .. }
        | IrValue::ResultIsOk { value }
        | IrValue::ResultValue { value, .. }
        | IrValue::ResultError { value }
        | IrValue::Checked { value, .. } => collect_resource_names_in_value(value, names, record),
        IrValue::MemberAccess { target, .. } => {
            collect_resource_names_in_value(target, names, record)
        }
        IrValue::WithUpdate {
            target, updates, ..
        } => {
            collect_resource_names_in_value(target, names, record);
            for update in updates {
                collect_resource_names_in_value(&update.value, names, record);
            }
        }
        IrValue::Binary { left, right, .. } => {
            collect_resource_names_in_value(left, names, record);
            collect_resource_names_in_value(right, names, record);
        }
        IrValue::Unary { operand, .. } => collect_resource_names_in_value(operand, names, record),
        IrValue::Local(_) | IrValue::Global(_) => {}
    }
}
