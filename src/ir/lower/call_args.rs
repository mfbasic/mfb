//! plan-176-N: IR lowering of call arguments — expected types and named/default argument
//! normalization. The lowering context and entry points live in `ir/lower/mod.rs`.

use super::*;

/// The expected type of argument `index` when `canonical_callee` names a builtin with
/// a GENERIC parameter there, resolved from the call's own actual argument types
/// (bug-550, `collections::append([], x)` type-checks and then fails to build).
///
/// The registry's overload selection is the same algorithm that already decides the
/// call's return type; asking it for the selected overload's *parameter* types with
/// the unified bindings substituted in is what turns `List OF T` into `List OF
/// Integer` for `collections::append([], 1)`. An argument whose own type does not
/// infer (a nested unresolved call) contributes `Unknown`, which unification treats
/// as a wildcard exactly as it does at return-type inference — so a call that used to
/// resolve still resolves, and one that never resolved still yields `None` here.
fn resolved_generic_argument_type(
    canonical_callee: &str,
    index: usize,
    arguments: &[HirCallArg],
    callee: &str,
    expected_return: Option<&ParameterType>,
    locals: &HashMap<String, ParameterType>,
    context: &LowerContext<'_>,
) -> Option<ParameterType> {
    let normalized = normalize_builtin_call_arguments(callee, arguments);
    let arg_types: Vec<ParameterType> = normalized
        .iter()
        .map(|argument| {
            expression_type(argument, locals, context).unwrap_or(ParameterType::Unknown)
        })
        .collect();
    builtins::resolved_argument_type(
        canonical_callee,
        index,
        &arg_types,
        expected_return,
        context.type_index,
    )
    .or_else(|| {
        // A literal argument the selection only accepts re-typed to its slot
        // (`append(bytes, 7)` binds `T := Byte`): the slot is the expected type,
        // so the literal lowers as that slot's constant.
        let retyped = literal_retyped_argument_types(
            canonical_callee,
            &arg_types,
            &normalized,
            context.type_index,
        )?;
        builtins::resolved_argument_type(
            canonical_callee,
            index,
            &retyped,
            expected_return,
            context.type_index,
        )
    })
}

pub(super) fn call_argument_expected_type(
    callee: &str,
    index: usize,
    arguments: &[HirCallArg],
    expected_return: Option<&ParameterType>,
    locals: &HashMap<String, ParameterType>,
    context: &LowerContext<'_>,
) -> Option<ParameterType> {
    let canonical_callee = canonical_import_name(callee, context);
    if callee == "toString" && index == 1 && arguments.len() == 2 {
        return Some(ParameterType::Byte);
    }
    if let Some(params) = builtins::argument_types_typed(&canonical_callee) {
        return params.get(index).cloned();
    }
    // plan-120-D: `argument_types_typed` declines for an overload SET, but the
    // positions the overloads agree on still have one expected type — and this
    // function decides union wrapping, so declining there silently lowers a bare
    // record where a tagged union is expected. Ask for the agreed type before
    // falling through to the user-function paths below.
    if let Some(agreed) = builtins::agreed_argument_type(&canonical_callee, index) {
        return Some(agreed);
    }
    // bug-550 (`append([], x)` does not build): a GENERIC parameter
    // (`collections::append`'s `List OF T`) has no
    // expected type independent of the call, so both lookups above decline — and the
    // argument lowers with no context at all. An empty `[]` written there then types
    // as `List OF Unknown`, which reaches codegen and fails the item-type check with
    // the internal "native collection list item must be Unknown, got Integer".
    // Overload selection can answer it: unifying the parameters against the ACTUAL
    // argument types binds `T := Unknown` from the literal and refines it to
    // `Integer` from the item, so parameter 0 substitutes to `List OF Integer`.
    if let Some(resolved) = resolved_generic_argument_type(
        &canonical_callee,
        index,
        arguments,
        callee,
        expected_return,
        locals,
        context,
    ) {
        return Some(resolved);
    }
    context
        .function_params
        .get(callee)
        .or_else(|| context.function_params.get(&canonical_callee))
        .and_then(|params| params.get(index).map(|param| param.type_.clone()))
        .or_else(|| {
            locals
                .get(callee)
                .and_then(function_param_types_from_type)
                .and_then(|params| params.get(index).cloned())
        })
}

pub(super) fn normalize_builtin_call_arguments<'a>(
    callee: &str,
    arguments: &'a [HirCallArg],
) -> Vec<&'a HirExpression> {
    if !arguments
        .iter()
        .any(|argument| matches!(argument, HirCallArg::Named { .. }))
    {
        return arguments.iter().map(call_arg_value).collect();
    }
    // A builtin whose overloads place a name at different positions selects the
    // overload first; the type checker has already proven one exists.
    if let Some(overloads) = builtins::call_param_name_overloads(callee) {
        let required = builtins::call_param_name_overload_required(callee).unwrap_or_default();
        return normalize_overloaded_builtin_call_arguments(&overloads, &required, arguments);
    }
    let Some(param_names) = builtins::call_param_names(callee) else {
        return arguments.iter().map(call_arg_value).collect();
    };
    let mut ordered = vec![None; param_names.len()];
    let mut next_positional = 0usize;
    let mut extras = Vec::new();
    for argument in arguments {
        match argument {
            HirCallArg::Positional(value) => {
                while next_positional < ordered.len() && ordered[next_positional].is_some() {
                    next_positional += 1;
                }
                if next_positional < ordered.len() {
                    ordered[next_positional] = Some(value);
                    next_positional += 1;
                } else {
                    extras.push(value);
                }
            }
            HirCallArg::Named { name, value, .. } => {
                if let Some(index) = param_names
                    .iter()
                    .position(|aliases| aliases.iter().any(|alias| alias == name))
                {
                    ordered[index] = Some(value);
                }
            }
        }
    }
    let mut normalized = ordered.into_iter().flatten().collect::<Vec<_>>();
    normalized.extend(extras);
    normalized
}

/// Order the arguments of a call to a builtin with a per-overload parameter-name
/// table, mirroring the former source checker's selection so both agree on which parameter a
/// name binds to. An unresolvable call was already rejected by the type checker;
/// keep its source order so lowering has something well-formed to walk.
fn normalize_overloaded_builtin_call_arguments<'a>(
    overloads: &[Vec<&str>],
    required: &[usize],
    arguments: &'a [HirCallArg],
) -> Vec<&'a HirExpression> {
    let positionals: Vec<&HirExpression> = arguments
        .iter()
        .filter_map(|argument| match argument {
            HirCallArg::Positional(value) => Some(value),
            HirCallArg::Named { .. } => None,
        })
        .collect();
    let named: Vec<(&str, &HirExpression)> = arguments
        .iter()
        .filter_map(|argument| match argument {
            HirCallArg::Named { name, value, .. } => Some((name.as_str(), value)),
            HirCallArg::Positional(_) => None,
        })
        .collect();
    let supplied_names: Vec<&str> = named.iter().map(|(name, _)| *name).collect();
    let Some(params) = builtins::select_param_name_overload(
        overloads,
        required,
        positionals.len(),
        &supplied_names,
    ) else {
        return arguments.iter().map(call_arg_value).collect();
    };

    let mut ordered: Vec<Option<&HirExpression>> = vec![None; params.len()];
    for (index, value) in positionals.into_iter().enumerate() {
        ordered[index] = Some(value);
    }
    for (name, value) in named {
        if let Some(index) = params.iter().position(|param| *param == name) {
            ordered[index] = Some(value);
        }
    }
    ordered.into_iter().flatten().collect()
}

fn normalize_local_call_arguments<'a>(
    callee: &str,
    arguments: &'a [HirCallArg],
    context: &LowerContext<'_>,
) -> Vec<Option<&'a HirExpression>> {
    let Some(params) = context.function_params.get(callee) else {
        return arguments
            .iter()
            .map(|argument| Some(call_arg_value(argument)))
            .collect();
    };
    let mut ordered = vec![None; params.len()];
    let mut next_positional = 0usize;
    for argument in arguments {
        match argument {
            HirCallArg::Positional(value) => {
                while next_positional < ordered.len() && ordered[next_positional].is_some() {
                    next_positional += 1;
                }
                if next_positional < ordered.len() {
                    ordered[next_positional] = Some(value);
                    next_positional += 1;
                }
            }
            HirCallArg::Named { name, value, .. } => {
                if let Some(index) = params.iter().position(|param| param.name == *name) {
                    ordered[index] = Some(value);
                }
            }
        }
    }
    ordered
}

pub(super) fn lower_local_call_arguments(
    callee: &str,
    arguments: &[HirCallArg],
    locals: &HashMap<String, ParameterType>,
    context: &mut LowerContext<'_>,
) -> Vec<IrValue> {
    let canonical_callee = canonical_import_name(callee, context);
    let params = context
        .function_params
        .get(callee)
        .or_else(|| context.function_params.get(&canonical_callee))
        .expect("local call lowering requires known function parameters");
    normalize_local_call_arguments(callee, arguments, context)
        .into_iter()
        .enumerate()
        .filter_map(|(index, argument)| {
            let expected =
                call_argument_expected_type(callee, index, arguments, None, locals, context);
            match argument {
                Some(argument) => Some(lower_expression_with_expected(
                    argument,
                    expected.as_ref(),
                    locals,
                    context,
                )),
                None => params
                    .get(index)
                    .and_then(|param| lower_call_default(param, context)),
            }
        })
        .collect()
}

pub(super) fn call_arg_value(argument: &HirCallArg) -> &HirExpression {
    match argument {
        HirCallArg::Positional(value) => value,
        HirCallArg::Named { value, .. } => value,
    }
}
