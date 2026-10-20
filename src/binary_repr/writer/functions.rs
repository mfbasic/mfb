//! plan-176-O: the `.mfp` writer's function lowering — functions, parameter defaults and
//! imported calls. Project lowering and `BinaryReprProject`'s encoder live in
//! `binary_repr/writer/mod.rs`.

use super::*;

/// A parameter record's `(flags, defaultConst)` pair (plan-136-B).
///
/// A literal default (`IrValue::Const`) is a `CONST_POOL` id under
/// [`PARAM_FLAG_DEFAULT`], exactly as before this plan, so an existing package's
/// bytes and `sigHash` are unchanged. A computed default is the zero-argument call
/// to its hidden default function (`ir::lower::lower_parameter_default`), stored as
/// that function's `FUNCTION_TABLE` index under `PARAM_FLAG_DEFAULT |
/// PARAM_FLAG_DEFAULT_FUNCTION`. Every other value shape is a lowering defect and is
/// refused here, naming the function and parameter, rather than written.
fn lower_param_default(
    function: &IrFunction,
    param: &crate::ir::IrParam,
    strings: &mut StringPool,
    constants: &mut ConstPool,
    function_ids: &HashMap<String, u32>,
) -> Result<(u32, u32), String> {
    let refuse = |shape: &str| {
        Err(format!(
            "function `{}` parameter `{}`: a package can store a default only as a constant \
             or a call to its hidden default function, not {shape}",
            function.name, param.name
        ))
    };
    let Some(default) = &param.default else {
        return Ok((0, u32::MAX));
    };
    match default {
        IrValue::Const { .. } => Ok((PARAM_FLAG_DEFAULT, constants.add(strings, default)?)),
        IrValue::Call { target, args, .. }
            if args.is_empty() && crate::internal_name::is_hidden_default_function(target) =>
        {
            let Some(index) = function_ids.get(target) else {
                return refuse(&format!(
                    "a call to `{target}`, which is not in the function table"
                ));
            };
            Ok((PARAM_FLAG_DEFAULT | PARAM_FLAG_DEFAULT_FUNCTION, *index))
        }
        IrValue::Call { .. } => refuse("a call to an ordinary function"),
        IrValue::Local(_)
        | IrValue::Global(_)
        | IrValue::LocalRef { .. }
        | IrValue::FunctionRef { .. }
        | IrValue::Closure { .. }
        | IrValue::Capture { .. }
        | IrValue::CallResult { .. }
        | IrValue::Checked { .. }
        | IrValue::Constructor { .. }
        | IrValue::UnionWrap { .. }
        | IrValue::UnionExtract { .. }
        | IrValue::ResultIsOk { .. }
        | IrValue::ResultValue { .. }
        | IrValue::ResultError { .. }
        | IrValue::WithUpdate { .. }
        | IrValue::ListLiteral { .. }
        | IrValue::SetLiteral { .. }
        | IrValue::MapLiteral { .. }
        | IrValue::MemberAccess { .. }
        | IrValue::Binary { .. }
        | IrValue::Unary { .. } => refuse("a non-constant value"),
    }
}

/// Lower an `IrFunction` to its container *metadata* (`Function`): name, kind,
/// flags, return type, and parameter signatures. Function *bodies* are no longer
/// flattened to opcodes here — they are carried verbatim in the structured
/// Binary Representation payload (`SECTION_BINARY_REPR`). The flat `code`/`registers`/`cleanups`
/// fields are therefore empty; only the signature-level tables (function table,
/// export table, ABI index, import table) consume this metadata.
pub(in crate::binary_repr) fn lower_function(
    function: &IrFunction,
    strings: &mut StringPool,
    types: &mut TypeTable,
    constants: &mut ConstPool,
    function_ids: &HashMap<String, u32>,
    external_function_abi_hashes: &HashMap<String, [u8; ABI_HASH_LEN]>,
    used_imported_functions: &mut HashSet<String>,
) -> Result<Function, String> {
    let mut params = Vec::new();
    for param in &function.params {
        let type_id = types.type_id(strings, &param.type_);
        let (flags, default_const) =
            lower_param_default(function, param, strings, constants, function_ids)?;
        params.push(Param {
            name: strings.intern(&param.name),
            type_id,
            flags,
            default_const,
        });
    }

    // Record which imported (cross-package) functions this body references so the
    // import table can pin the exact used symbols. Imported targets are exactly
    // the qualified names present in `external_function_abi_hashes`.
    for op in &function.body {
        collect_imported_calls_op(op, external_function_abi_hashes, used_imported_functions);
    }

    let mut flags = if function.visibility == "export" {
        0
    } else {
        FUNCTION_FLAG_PRIVATE
    };
    if function.returns == crate::types::ParameterType::Nothing {
        flags |= FUNCTION_FLAG_RETURNS_NOTHING;
    }
    if function.isolated {
        flags |= FUNCTION_FLAG_ISOLATED;
    }

    Ok(Function {
        name: strings.intern(&function.name),
        kind: FUNCTION_BINARY_REPR,
        flags,
        return_type: types.type_id(strings, &function.returns),
        params,
        registers: Vec::new(),
        cleanups: Vec::new(),
    })
}

/// Walk an `IrOp`, recording any call/reference target that names an imported
/// (cross-package) function into `used`.
pub(in crate::binary_repr) fn collect_imported_calls_op(
    op: &IrOp,
    imported: &HashMap<String, [u8; ABI_HASH_LEN]>,
    used: &mut HashSet<String>,
) {
    match op {
        IrOp::Bind { value, .. } => {
            if let Some(v) = value {
                collect_imported_calls_value(v, imported, used);
            }
        }
        IrOp::Assign { value, .. }
        | IrOp::AssignGlobal { value, .. }
        | IrOp::StateAssign { value, .. }
        | IrOp::Eval { value, .. }
        | IrOp::Fail { error: value, .. } => collect_imported_calls_value(value, imported, used),
        IrOp::Return { value, .. } => {
            if let Some(v) = value {
                collect_imported_calls_value(v, imported, used);
            }
        }
        IrOp::ExitLoop { .. } | IrOp::ContinueLoop { .. } => {}
        IrOp::ExitProgram { code, .. } => collect_imported_calls_value(code, imported, used),
        IrOp::If {
            condition,
            then_body,
            else_body,
            ..
        } => {
            collect_imported_calls_value(condition, imported, used);
            for op in then_body.iter().chain(else_body) {
                collect_imported_calls_op(op, imported, used);
            }
        }
        IrOp::Match { value, cases, .. } => {
            collect_imported_calls_value(value, imported, used);
            for case in cases {
                if let Some(guard) = &case.guard {
                    collect_imported_calls_value(guard, imported, used);
                }
                for op in &case.body {
                    collect_imported_calls_op(op, imported, used);
                }
            }
        }
        IrOp::While {
            condition, body, ..
        } => {
            collect_imported_calls_value(condition, imported, used);
            for op in body {
                collect_imported_calls_op(op, imported, used);
            }
        }
        IrOp::For {
            start,
            end,
            step,
            body,
            ..
        } => {
            collect_imported_calls_value(start, imported, used);
            collect_imported_calls_value(end, imported, used);
            collect_imported_calls_value(step, imported, used);
            for op in body {
                collect_imported_calls_op(op, imported, used);
            }
        }
        IrOp::DoUntil {
            body, condition, ..
        } => {
            for op in body {
                collect_imported_calls_op(op, imported, used);
            }
            collect_imported_calls_value(condition, imported, used);
        }
        IrOp::ForEach { iterable, body, .. } => {
            collect_imported_calls_value(iterable, imported, used);
            for op in body {
                collect_imported_calls_op(op, imported, used);
            }
        }
        IrOp::Trap { body, .. } => {
            for op in body {
                collect_imported_calls_op(op, imported, used);
            }
        }
    }
}

/// Walk an `IrValue`, recording imported function references into `used`.
pub(in crate::binary_repr) fn collect_imported_calls_value(
    value: &IrValue,
    imported: &HashMap<String, [u8; ABI_HASH_LEN]>,
    used: &mut HashSet<String>,
) {
    let note = |target: &str, used: &mut HashSet<String>| {
        if imported.contains_key(target) {
            used.insert(target.to_string());
        }
    };
    match value {
        IrValue::Call { target, args, .. } | IrValue::CallResult { target, args, .. } => {
            note(target, used);
            for arg in args {
                collect_imported_calls_value(arg, imported, used);
            }
        }
        IrValue::FunctionRef { name, .. } => note(name, used),
        IrValue::Closure { name, captures, .. } => {
            note(name, used);
            for capture in captures {
                collect_imported_calls_value(capture, imported, used);
            }
        }
        IrValue::Constructor { args, .. } => {
            for arg in args {
                collect_imported_calls_value(arg, imported, used);
            }
        }
        IrValue::UnionWrap { value, .. }
        | IrValue::UnionExtract { value, .. }
        | IrValue::ResultIsOk { value }
        | IrValue::ResultValue { value, .. }
        | IrValue::ResultError { value }
        | IrValue::Checked { value, .. }
        | IrValue::Unary { operand: value, .. }
        | IrValue::MemberAccess { target: value, .. } => {
            collect_imported_calls_value(value, imported, used)
        }
        IrValue::WithUpdate {
            target, updates, ..
        } => {
            collect_imported_calls_value(target, imported, used);
            for update in updates {
                collect_imported_calls_value(&update.value, imported, used);
            }
        }
        IrValue::ListLiteral { values, .. } => {
            for v in values {
                collect_imported_calls_value(v, imported, used);
            }
        }
        IrValue::SetLiteral { values, .. } => {
            for v in values {
                collect_imported_calls_value(v, imported, used);
            }
        }
        IrValue::MapLiteral { entries, .. } => {
            for (k, v) in entries {
                collect_imported_calls_value(k, imported, used);
                collect_imported_calls_value(v, imported, used);
            }
        }
        IrValue::Binary { left, right, .. } => {
            collect_imported_calls_value(left, imported, used);
            collect_imported_calls_value(right, imported, used);
        }
        IrValue::Const { .. }
        | IrValue::Local(_)
        | IrValue::LocalRef { .. }
        | IrValue::Global(_)
        | IrValue::Capture { .. } => {}
    }
}
