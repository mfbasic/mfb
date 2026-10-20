//! plan-176-P: collecting a program's static string values and type names. `string_symbols`, the error-message data and the raw/string data
//! objects live in `memory/data/data_objects/mod.rs`.

use super::*;

/// Error messages emitted by native `LINK` thunks and their initializer
/// (plan-linker.md §12): the allocation message is already covered by the
/// standard set, so only the two binding-specific messages are listed here.

pub(super) fn collect_type_name_values(module: &NirModule, values: &mut Vec<String>) {
    for value in [
        "Boolean", "Byte", "Error", "Fixed", "Float", "Integer", "Money", "Nothing", "Scalar",
        "String",
    ] {
        push_string_value(values, value.to_string());
    }
    for type_ in &module.types {
        push_string_value(values, type_.name.clone());
        for field in &type_.fields {
            push_string_value(values, field.type_.name().into_owned());
        }
        for variant in &type_.variants {
            push_string_value(values, variant.name.clone());
            for field in &variant.fields {
                push_string_value(values, field.type_.name().into_owned());
            }
        }
    }
    for function in &module.functions {
        push_string_value(values, function.returns.name().into_owned());
        for param in &function.params {
            push_string_value(values, param.type_.name().into_owned());
        }
        collect_type_name_values_from_ops(&function.body, values);
    }
}

fn collect_type_name_values_from_ops(ops: &[NirOp], values: &mut Vec<String>) {
    use nir::visit::{walk_op, walk_value, NirVisitor};
    struct Collector<'a> {
        values: &'a mut Vec<String>,
    }
    impl NirVisitor for Collector<'_> {
        fn visit_op(&mut self, op: &NirOp) {
            match op {
                NirOp::Bind { type_, .. } => {
                    push_string_value(self.values, type_.name().into_owned())
                }
                NirOp::StoreGlobal { type_, .. } if !type_.name().is_empty() => {
                    push_string_value(self.values, type_.name().into_owned())
                }
                NirOp::For { type_, .. } | NirOp::ForEach { type_, .. } => {
                    push_string_value(self.values, type_.name().into_owned())
                }
                _ => {}
            }
            walk_op(self, op);
        }
        fn visit_value(&mut self, value: &NirValue) {
            match value {
                NirValue::Const { type_, .. }
                | NirValue::FunctionRef { type_, .. }
                | NirValue::Constructor { type_, .. }
                | NirValue::ListLiteral { type_, .. }
                | NirValue::SetLiteral { type_, .. }
                | NirValue::MapLiteral { type_, .. }
                | NirValue::UnionExtract { type_, .. }
                | NirValue::WithUpdate { type_, .. } => {
                    push_string_value(self.values, type_.name().into_owned())
                }
                NirValue::UnionWrap {
                    union_type,
                    member_type,
                    ..
                } => {
                    push_string_value(self.values, union_type.name().into_owned());
                    push_string_value(self.values, member_type.name().into_owned());
                }
                _ => {}
            }
            walk_value(self, value);
        }
    }
    Collector { values }.visit_ops(ops);
}

/// Walk one function's body for string literals that need a data object.
///
/// The local-type map is seeded with the function's parameters. The code
/// builder records every parameter as a local carrying its declared type, so it
/// folds `typeName(param)` — and any `&` concatenation around it — to a literal.
/// Starting this pass with an empty map made its view of local types strictly
/// weaker than the builder's, so a fold the builder performed produced a literal
/// this pass had never seen and the build aborted with no data object for it
/// (bug-361B).
pub(super) fn collect_string_values_from_function(
    function: &NirFunction,
    values: &mut Vec<String>,
    fields: &FieldTypes,
) {
    let mut constants = HashMap::new();
    let mut types: HashMap<String, ParameterType> = function
        .params
        .iter()
        .map(|param| (param.name.clone(), param.type_.clone()))
        .collect();
    collect_string_values_from_ops_with_constants(
        &function.body,
        values,
        &mut constants,
        &mut types,
        fields,
    );
}

fn collect_string_values_from_ops_with_constants(
    ops: &[NirOp],
    values: &mut Vec<String>,
    constants: &mut HashMap<String, NirValue>,
    types: &mut HashMap<String, ParameterType>,
    fields: &FieldTypes,
) {
    for op in ops {
        match op {
            NirOp::Bind {
                name, type_, value, ..
            } => {
                types.insert(name.clone(), type_.clone());
                if let Some(value) = value {
                    collect_string_values_from_value(value, values, constants, types, fields);
                    if let Some(constant) =
                        local_constant_value_with_constants(value, constants, types, fields)
                    {
                        constants.insert(name.clone(), constant);
                    } else {
                        constants.remove(name);
                    }
                } else {
                    constants.remove(name);
                }
            }
            NirOp::StoreGlobal { value, .. } => {
                if let Some(value) = value {
                    collect_string_values_from_value(value, values, constants, types, fields);
                }
            }
            NirOp::Return { value } => {
                if let Some(value) = value {
                    collect_string_values_from_value(value, values, constants, types, fields);
                }
            }
            NirOp::ExitLoop { .. } | NirOp::ContinueLoop { .. } => {}
            NirOp::ExitProgram { code } => {
                collect_string_values_from_value(code, values, constants, types, fields);
            }
            NirOp::Fail { error } => {
                collect_string_values_from_value(error, values, constants, types, fields);
            }
            NirOp::StateAssign { value, .. } => {
                collect_string_values_from_value(value, values, constants, types, fields);
            }
            NirOp::Assign { name, value } => {
                collect_string_values_from_value(value, values, constants, types, fields);
                if let Some(constant) =
                    local_constant_value_with_constants(value, constants, types, fields)
                {
                    constants.insert(name.clone(), constant);
                } else {
                    constants.remove(name);
                }
            }
            NirOp::Eval { value } => {
                collect_string_values_from_value(value, values, constants, types, fields);
            }
            NirOp::If {
                condition,
                then_body,
                else_body,
            } => {
                collect_string_values_from_value(condition, values, constants, types, fields);
                let mut then_constants = constants.clone();
                let mut else_constants = constants.clone();
                let mut then_types = types.clone();
                let mut else_types = types.clone();
                collect_string_values_from_ops_with_constants(
                    then_body,
                    values,
                    &mut then_constants,
                    &mut then_types,
                    fields,
                );
                collect_string_values_from_ops_with_constants(
                    else_body,
                    values,
                    &mut else_constants,
                    &mut else_types,
                    fields,
                );
            }
            NirOp::Match { value, cases } => {
                collect_string_values_from_value(value, values, constants, types, fields);
                for case in cases {
                    // Exhaustive on purpose: an `if let` here silently skipped
                    // `OneOf`, so `CASE "B", "C"` reached codegen with no data
                    // object for either literal (bug-361A). Keeping the match
                    // exhaustive makes the next pattern variant a build error
                    // rather than another silent miss.
                    match &case.pattern {
                        NirMatchPattern::Value(value) => {
                            collect_string_values_from_value(
                                value, values, constants, types, fields,
                            );
                        }
                        NirMatchPattern::OneOf(patterns) => {
                            for pattern in patterns {
                                collect_string_values_from_value(
                                    pattern, values, constants, types, fields,
                                );
                            }
                        }
                        NirMatchPattern::Else => {}
                    }
                    // A guard is a value expression that may hold string
                    // literals; without walking it, `fs::exists("/tmp/x")` in a
                    // `WHEN` guard has no data object at codegen (bug-118).
                    if let Some(guard) = &case.guard {
                        collect_string_values_from_value(guard, values, constants, types, fields);
                    }
                    let mut case_constants = constants.clone();
                    let mut case_types = types.clone();
                    collect_string_values_from_ops_with_constants(
                        &case.body,
                        values,
                        &mut case_constants,
                        &mut case_types,
                        fields,
                    );
                }
            }
            NirOp::While {
                condition, body, ..
            } => {
                collect_string_values_from_value(condition, values, constants, types, fields);
                let mut body_constants = constants.clone();
                let mut body_types = types.clone();
                collect_string_values_from_ops_with_constants(
                    body,
                    values,
                    &mut body_constants,
                    &mut body_types,
                    fields,
                );
            }
            NirOp::For {
                name,
                type_,
                start,
                end,
                step,
                body,
                ..
            } => {
                collect_string_values_from_value(start, values, constants, types, fields);
                collect_string_values_from_value(end, values, constants, types, fields);
                collect_string_values_from_value(step, values, constants, types, fields);
                // bug-816: the counter takes the loop's type, and the bound and
                // step tests compare it with `end`/`step`: a `Float` start, bound
                // or step of a `Fixed` loop is converted into Fixed, which needs
                // ErrInvalidFormat's message.
                let counter = NirValue::Local(name.clone());
                let mut loop_types = types.clone();
                loop_types.insert(name.clone(), type_.clone());
                if [start, end, step].into_iter().any(|bound| {
                    binary_may_consume_float_into_exact(
                        BinaryOp::LessEqual,
                        &counter,
                        bound,
                        &loop_types,
                        fields,
                    )
                }) {
                    push_string_value(values, err_msg("ErrInvalidFormat"));
                }
                let mut body_constants = constants.clone();
                let mut body_types = types.clone();
                body_constants.remove(name);
                body_types.insert(name.clone(), type_.clone());
                collect_string_values_from_ops_with_constants(
                    body,
                    values,
                    &mut body_constants,
                    &mut body_types,
                    fields,
                );
            }
            NirOp::DoUntil { body, condition } => {
                let mut body_constants = constants.clone();
                let mut body_types = types.clone();
                collect_string_values_from_ops_with_constants(
                    body,
                    values,
                    &mut body_constants,
                    &mut body_types,
                    fields,
                );
                collect_string_values_from_value(condition, values, constants, types, fields);
            }
            NirOp::ForEach {
                name,
                type_,
                iterable,
                body,
            } => {
                collect_string_values_from_value(iterable, values, constants, types, fields);
                let mut body_constants = constants.clone();
                let mut body_types = types.clone();
                body_constants.remove(name);
                body_types.insert(name.clone(), type_.clone());
                collect_string_values_from_ops_with_constants(
                    body,
                    values,
                    &mut body_constants,
                    &mut body_types,
                    fields,
                );
            }
            NirOp::Trap { body, .. } => {
                let mut trap_constants = constants.clone();
                let mut trap_types = types.clone();
                collect_string_values_from_ops_with_constants(
                    body,
                    values,
                    &mut trap_constants,
                    &mut trap_types,
                    fields,
                );
            }
        }
    }
}

fn collect_string_values_from_value(
    value: &NirValue,
    values: &mut Vec<String>,
    constants: &HashMap<String, NirValue>,
    types: &HashMap<String, ParameterType>,
    fields: &FieldTypes,
) {
    if let Some(value) = static_string_value_with_constants(value, constants, types, fields) {
        push_string_value(values, value);
    }
    // plan-140-C: `toString(<enum>)` loads one of the enum's member names, so
    // each must exist as string data. Registered only for an enum a program
    // actually passes to `toString`.
    if let NirValue::Call { target, args, .. }
    | NirValue::CallResult { target, args, .. }
    | NirValue::RuntimeCall { target, args, .. } = value
    {
        if target == "toString" && args.len() == 1 {
            if let Some(members) = to_string_enum_members(&args[0], types, fields) {
                for member in members {
                    push_string_value(values, member.clone());
                }
            }
        }
    }
    if let NirValue::Call { target, args, .. }
    | NirValue::CallResult { target, args, .. }
    | NirValue::RuntimeCall { target, args, .. } = value
    {
        if target == "strings.graphemes" && args.len() == 1 {
            if let Some(value) =
                static_string_value_with_constants(&args[0], constants, types, fields)
            {
                for grapheme in crate::unicode::backend::graphemes(&value) {
                    push_string_value(values, grapheme);
                }
            }
        }
        if target == "fs.pathJoin" && args.len() == 1 {
            push_string_value(values, "/".to_string());
        }
        if target == "fs.pathDirName" && args.len() == 1 {
            push_string_value(values, ".".to_string());
            push_string_value(values, "/".to_string());
        }
    }
    if value_may_return_invalid_format(value, constants, types, fields) {
        push_string_value(values, err_msg("ErrInvalidFormat"));
    }
    match value {
        NirValue::Const { type_, value } if matches!(type_, ParameterType::String) => {
            push_string_value(values, value.clone());
        }
        NirValue::Call { args, .. }
        | NirValue::CallResult { args, .. }
        | NirValue::RuntimeCall { args, .. }
        | NirValue::Constructor { args, .. } => {
            for arg in args {
                collect_string_values_from_value(arg, values, constants, types, fields);
            }
        }
        NirValue::UnionWrap { value, .. }
        | NirValue::UnionExtract { value, .. }
        | NirValue::ResultIsOk { value }
        | NirValue::ResultValue { value }
        | NirValue::ResultError { value }
        | NirValue::Checked { value, .. } => {
            collect_string_values_from_value(value, values, constants, types, fields)
        }
        NirValue::WithUpdate {
            target, updates, ..
        } => {
            collect_string_values_from_value(target, values, constants, types, fields);
            for update in updates {
                collect_string_values_from_value(&update.value, values, constants, types, fields);
            }
        }
        NirValue::ListLiteral { values: items, .. }
        | NirValue::SetLiteral { values: items, .. } => {
            for item in items {
                collect_string_values_from_value(item, values, constants, types, fields);
            }
        }
        NirValue::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                collect_string_values_from_value(key, values, constants, types, fields);
                collect_string_values_from_value(value, values, constants, types, fields);
            }
        }
        NirValue::MemberAccess { target, .. } => {
            collect_string_values_from_value(target, values, constants, types, fields)
        }
        NirValue::Binary { left, right, .. } => {
            collect_string_values_from_value(left, values, constants, types, fields);
            collect_string_values_from_value(right, values, constants, types, fields);
        }
        NirValue::Unary { operand, .. } => {
            collect_string_values_from_value(operand, values, constants, types, fields)
        }
        NirValue::Closure { captures, .. } => {
            for value in captures {
                collect_string_values_from_value(value, values, constants, types, fields);
            }
        }
        NirValue::Capture { .. }
        | NirValue::Const { .. }
        | NirValue::Local(_)
        | NirValue::LocalRef { .. }
        | NirValue::Global { .. }
        | NirValue::FunctionRef { .. } => {}
    }
}

pub(super) fn push_string_value(values: &mut Vec<String>, value: String) {
    if !values.contains(&value) {
        values.push(value);
    }
}

pub(crate) fn static_string_value_with_constants(
    value: &NirValue,
    constants: &HashMap<String, NirValue>,
    types: &HashMap<String, ParameterType>,
    fields: &FieldTypes,
) -> Option<String> {
    match value {
        NirValue::Const { type_, value } if matches!(type_, ParameterType::String) => {
            Some(value.clone())
        }
        NirValue::Local(name) => constants.get(name).and_then(|constant| {
            static_string_value_with_constants(constant, constants, types, fields)
        }),
        NirValue::Call { target, args, .. } if target == "toString" && args.len() == 1 => {
            static_primitive_text_with_constants(&args[0], constants)
        }
        NirValue::RuntimeCall { target, args, .. } if target == "toString" && args.len() == 1 => {
            static_primitive_text_with_constants(&args[0], constants)
        }
        NirValue::Call { target, args, .. }
        | NirValue::CallResult { target, args, .. }
        | NirValue::RuntimeCall { target, args, .. }
            if target == "typeName" && args.len() == 1 =>
        {
            // `typeName` yields the argument type's SPELLING — the rendered
            // name IS the program-visible string value here.
            static_type_name_for_fold_with_types(&args[0], types, fields)
                .map(|type_| type_.name().into_owned())
        }
        NirValue::Call { target, args, .. }
        | NirValue::CallResult { target, args, .. }
        | NirValue::RuntimeCall { target, args, .. } => {
            strings_package_static_string_value(target, args, constants, types, fields)
        }
        NirValue::Binary {
            op, left, right, ..
        } if *op == BinaryOp::Concat => {
            let left = static_string_value_with_constants(left, constants, types, fields)?;
            let right = static_string_value_with_constants(right, constants, types, fields)?;
            Some(format!("{left}{right}"))
        }
        _ => None,
    }
}
