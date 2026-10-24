//! plan-176-P: static type names and builtin function references. `string_symbols`, the error-message data and the raw/string data
//! objects live in `memory/data/data_objects/mod.rs`.

use super::*;

pub(crate) fn static_type_name_with_types(
    value: &NirValue,
    types: &HashMap<String, ParameterType>,
    fields: &FieldTypes,
) -> Option<ParameterType> {
    match value {
        NirValue::Const { type_, .. } => Some(type_.clone()),
        NirValue::Local(name) => types.get(name).cloned(),
        NirValue::LocalRef { type_, .. } => Some(type_.clone()),
        NirValue::Global { type_, .. } if !is_unset_type(type_) => Some(type_.clone()),
        NirValue::Global { .. } => None,
        NirValue::FunctionRef { type_, .. }
        | NirValue::Closure { type_, .. }
        | NirValue::Capture { type_, .. }
        | NirValue::Constructor { type_, .. }
        | NirValue::WithUpdate { type_, .. }
        | NirValue::ListLiteral { type_, .. }
        | NirValue::SetLiteral { type_, .. }
        | NirValue::MapLiteral { type_, .. } => Some(type_.clone()),
        NirValue::UnionWrap { union_type, .. } => Some(union_type.clone()),
        NirValue::UnionExtract { type_, .. } => Some(type_.clone()),
        // `Checked` reports its success type, as `CallResult` below reports the
        // callee's return type rather than the `Result OF` wrapper (bug-471).
        NirValue::Checked { type_, .. } => Some(type_.clone()),
        NirValue::Call { target, .. }
        | NirValue::CallResult { target, .. }
        | NirValue::RuntimeCall { target, .. } => match target.as_str() {
            "typeName" | "toString" => Some(ParameterType::String),
            "len" | "toInt" => Some(ParameterType::Integer),
            // Migrated find/mid/replace: strings:: returns Integer/String; the
            // collections:: List overloads return the list type and are resolved
            // by the precise type path, so only `find` (always Integer) is mapped
            // here (plan-01-functions.md §5).
            "collections.find" | "strings.find" => Some(ParameterType::Integer),
            "strings.mid" | "strings.replace" => Some(ParameterType::String),
            "strings.trim"
            | "strings.trimStart"
            | "strings.trimEnd"
            | "strings.upper"
            | "strings.lower"
            | "strings.caseFold"
            | "strings.normalizeNfc"
            | "strings.join" => Some(ParameterType::String),
            "strings.graphemes" | "strings.split" => {
                Some(ParameterType::list_of(ParameterType::String))
            }
            "strings.startsWith" | "strings.endsWith" | "strings.contains" => {
                Some(ParameterType::Boolean)
            }
            "strings.byteLen" => Some(ParameterType::Integer),
            "toFloat" => Some(ParameterType::Float),
            "toFixed" => Some(ParameterType::Fixed),
            "toByte" => Some(ParameterType::Byte),
            "toMoney" => Some(ParameterType::Money),
            "toScalar" => Some(scalar_type()),
            "isNumeric" => Some(ParameterType::Boolean),
            _ => None,
        },
        NirValue::ResultIsOk { .. } => Some(ParameterType::Boolean),
        NirValue::ResultValue { value } => {
            match static_type_name_with_types(value, types, fields)? {
                // A non-`Result` operand answers with its own type, as the
                // `strip_prefix(…).or_else(…)` this replaces did.
                ParameterType::ResultOf(success) => Some(*success),
                other => Some(other),
            }
        }
        NirValue::ResultError { .. } => Some(error_type()),
        NirValue::Binary {
            op, left, right, ..
        } => {
            if op.is_comparison() || matches!(op, BinaryOp::And | BinaryOp::Or | BinaryOp::Xor) {
                return Some(ParameterType::Boolean);
            }
            if *op == BinaryOp::Concat {
                return Some(ParameterType::String);
            }
            let left = static_type_name_with_types(left, types, fields)?;
            let right = static_type_name_with_types(right, types, fields)?;
            Some(promoted_binary_type(*op, &left, &right))
        }
        NirValue::Unary { op, operand, .. } => {
            if *op == UnaryOp::Not {
                Some(ParameterType::Boolean)
            } else {
                static_type_name_with_types(operand, types, fields)
            }
        }
        NirValue::MemberAccess { target, member } => {
            let target_type = static_type_name_with_types(target, types, fields)?;
            // Record and union-variant fields, then the two `MapEntry` members —
            // the same sources `static_nir_value_type` consults. Without the
            // field table this arm answered `None` for every record field, which
            // silently under-reported in every predicate built on this seam:
            // `typeName(rec.field)` failed to lower at all, and the
            // ERR_INVALID_FORMAT gate missed a promoting Float operand (bug-366).
            // `FieldTypes` keys are nominal type NAMES, so the lookup renders
            // the (scalar-cheap) name — a name-keyed table probe.
            if let Some(field_type) = fields.get(&(target_type.name().into_owned(), member.clone()))
            {
                return Some(field_type.clone());
            }
            let (key_type, value_type) = typed_map_entry_type_parts(&target_type)?;
            match member.as_str() {
                "key" => Some(key_type.clone()),
                "value" => Some(value_type.clone()),
                _ => None,
            }
        }
    }
}

/// The pre-pass twin of [`crate::codegen::engine::builder::CodeBuilder::static_type_name_for_fold`]: static
/// type of `value`, resolving builtin calls that [`static_type_name_with_types`]'s
/// hand-written table misses via `builtins::resolve_call_return_type`.
///
/// Used **only** for the `typeName` compile-time fold (bug-354), where the pre-pass
/// interns the folded type-name string the builder later looks up — so this must
/// agree with the builder's `static_type_name_for_fold`, and both delegate to the
/// same resolver. It does NOT widen `static_type_name_with_types`, whose other
/// consumers (the float-numeric-error gate, module analysis, binary typing) must
/// keep their exact current answers.
pub(crate) fn static_type_name_for_fold_with_types(
    value: &NirValue,
    types: &HashMap<String, ParameterType>,
    fields: &FieldTypes,
) -> Option<ParameterType> {
    if let Some(type_) = static_type_name_with_types(value, types, fields) {
        return Some(type_);
    }
    match value {
        // bug-874's twin: `Colour.Red` names a member of an enum TYPE, so its
        // target is the type's name, not a binding, and the operand walk above
        // has no local to type. The builder's `static_type_name_for_fold` folds
        // it to the enum's spelling; this pre-pass must intern that same string,
        // or `"x" & typeName(Colour.Red)` folds to a literal with no data object.
        NirValue::MemberAccess { target, member } => match target.as_ref() {
            NirValue::Local(type_name) if !types.contains_key(type_name) => fields
                .enums()
                .find(|(enum_type, members)| {
                    enum_type.name() == *type_name && members.contains(member)
                })
                .map(|(enum_type, _)| enum_type.clone()),
            // The builder's fold also names a stateful resource's `.state` and a
            // field read off it (`typeName(h.state.pos)`): the same arms here, or
            // `"x" & typeName(h.state.pos)` folds to an uninterned literal.
            _ => {
                let target_type = static_type_name_for_fold_with_types(target, types, fields)?;
                if member == "state" {
                    target_type.state()
                } else {
                    fields
                        .get(&(target_type.name().into_owned(), member.clone()))
                        .cloned()
                }
            }
        },
        NirValue::Call { target, args, .. }
        | NirValue::CallResult { target, args, .. }
        | NirValue::RuntimeCall { target, args, .. } => {
            // bug-569/672's twin: a callable LOCAL shadows any function of the same
            // name and its result is the function type's return. The builder folds
            // `typeName(f(1))` through it (`static_type_name_for_fold`); without
            // the same arm here `"x" & typeName(f(1))` folds to an uninterned literal.
            if let Some(local) = types.get(target.as_str()) {
                return match local {
                    ParameterType::Func(_, returns, _) => Some((**returns).clone()),
                    _ => None,
                };
            }
            // bug-672's twin: a call to a module function has its declared return.
            if let Some(returns) = fields.function_return(target) {
                return Some(returns.clone());
            }
            let arg_types = args
                .iter()
                .map(|arg| static_type_name_for_fold_with_types(arg, types, fields))
                .collect::<Option<Vec<_>>>()?;
            builtins::resolve_call_return_type_typed(target, &arg_types, false)
        }
        _ => None,
    }
}

pub(crate) fn builtin_function_symbol_for_type(
    name: &str,
    type_: &ParameterType,
) -> Option<String> {
    crate::codegen::builtins::general::builtin_function_id_for_type(name, type_)?;
    Some(format!(
        "_mfb_builtin_{}_{}",
        nir::symbol_fragment(name),
        nir::symbol_fragment(&type_.name())
    ))
}

pub(crate) fn builtin_function_refs(module: &NirModule) -> Vec<(String, ParameterType, String)> {
    let mut refs = Vec::new();
    let mut seen = HashSet::new();
    for function in &module.functions {
        collect_builtin_function_refs_in_ops(&function.body, &mut refs, &mut seen);
    }
    refs
}

/// plan-86 K1: names of every top-level function referenced as a `FunctionRef`
/// (used as a callback / function value) ANYWHERE in the module. Such a function
/// is invoked through the generic callback ABI, which takes OWNERSHIP of the
/// callback's return value — e.g. `collections::groupBy` stores each per-element
/// result into a bucket and frees the callee's returned block after use. A
/// parameter-passthrough callback that returned a borrow of its argument would
/// hand that ABI a non-owned pointer to a per-iteration temporary, causing a
/// double-free / UAF (observed: grouped String values came back empty). So a
/// function used as a `FunctionRef` MUST keep returning a fresh owned copy;
/// `function_returns_param_borrow` excludes every name in this set.
///
/// bug-569: "and frees the callee's returned block after use" described no code
/// until then — the ABI freed only the per-iteration ARGUMENT
/// (`free_collection_loop_item`), never the RESULT, so every `String` callback
/// leaked one block per element per call. `lower_transform` frees it now, which
/// is `transform`, `sortBy` and `groupBy` (both reach the callback through it);
/// `mapValues` invokes its callback directly from its `.mfb` body and is owned by
/// the ordinary statement-scope temp instead. Both halves of this contract are
/// therefore live: the callee owes a solely-owned block (bug-562) and the ABI
/// drops it.
pub(crate) fn collect_function_ref_names(module: &NirModule) -> HashSet<String> {
    use nir::visit::{walk_value, NirVisitor};
    struct Collector<'a> {
        out: &'a mut HashSet<String>,
    }
    impl NirVisitor for Collector<'_> {
        fn visit_value(&mut self, value: &NirValue) {
            if let NirValue::FunctionRef { name, .. } = value {
                self.out.insert(name.clone());
            }
            walk_value(self, value);
        }
    }
    let mut out = HashSet::new();
    for function in &module.functions {
        Collector { out: &mut out }.visit_ops(&function.body);
    }
    out
}

fn collect_builtin_function_refs_in_ops(
    ops: &[NirOp],
    refs: &mut Vec<(String, ParameterType, String)>,
    seen: &mut HashSet<String>,
) {
    use nir::visit::{walk_value, NirVisitor};
    struct Collector<'a> {
        refs: &'a mut Vec<(String, ParameterType, String)>,
        seen: &'a mut HashSet<String>,
    }
    impl NirVisitor for Collector<'_> {
        fn visit_value(&mut self, value: &NirValue) {
            if let NirValue::FunctionRef { name, type_ } = value {
                if let Some(symbol) = builtin_function_symbol_for_type(name, &type_) {
                    let key = format!("{name}\0{type_}");
                    if self.seen.insert(key) {
                        self.refs.push((name.clone(), type_.clone(), symbol));
                    }
                }
            }
            walk_value(self, value);
        }
    }
    Collector { refs, seen }.visit_ops(ops);
}
