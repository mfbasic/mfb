//! plan-176-N: IR lowering of type queries — function and parameter types, binding inference
//! and an expression's type. The lowering context and entry points live in `ir/lower/mod.rs`.

use super::*;

pub(super) fn function_returns(hir: &HirProject) -> HashMap<String, ParameterType> {
    let mut returns = HashMap::new();
    // Native LINK function return types, keyed `alias.func`, so callers like
    // `sqliteLink::open(...)` get a type during IR lowering (plan-link-update.md §5b).
    let mut native_returns: HashMap<String, ParameterType> = HashMap::new();
    for file in &hir.files {
        for item in &file.items {
            match item {
                HirItem::Function(function) => {
                    // Carries the STATE too, so `openTagged(p).state` resolves
                    // from the call expression (plan-52-D).
                    returns.insert(function.name.clone(), function_return_type(function));
                }
                HirItem::Link(link) => {
                    for native in &link.functions {
                        let return_type = native_type(native.return_type.as_ref());
                        // Carry a stateful native producer's STATE, so a wrapper
                        // that calls `snd::rawOpen(p)` sees `SoundFile STATE
                        // FileInfo` and can RETURN it as its own stateful return
                        // (plan-53-A/B). Without this the call infers bare
                        // `SoundFile` and the wrapper's RETURN mismatches.
                        let return_type = match (native.return_resource, &native.return_state_type)
                        {
                            (true, Some(state)) => return_type.with_state(state),
                            _ => return_type,
                        };
                        native_returns
                            .insert(format!("{}.{}", link.alias, native.name), return_type);
                    }
                }
                _ => {}
            }
        }
    }
    // Re-export aliases adopt their target's return type (plan-link-update.md §5a).
    for file in &hir.files {
        for item in &file.items {
            if let HirItem::FuncAlias(alias) = item {
                if let Some(return_type) = native_returns.get(&alias.target) {
                    returns.insert(alias.name.clone(), return_type.clone());
                }
            }
        }
    }
    returns.extend(native_returns);
    returns
}

pub(super) fn function_types(hir: &HirProject) -> HashMap<String, ParameterType> {
    let mut types = HashMap::new();
    for file in &hir.files {
        for item in &file.items {
            if let HirItem::Function(function) = item {
                // A first-class reference's parameters carry their `STATE T` clause,
                // as the declared FUNC's own parameters do (`lower_param`) and as a
                // lambda's callable type does (`lambda_param_type`). A function
                // value is called through this type alone, so without the clause
                // `LET j = p` then `j(bareFile)` handed a resource with no state
                // payload to a body that reads `x.state` (bug-1054).
                let params = function
                    .params
                    .iter()
                    .map(lambda_param_type)
                    .collect::<Vec<_>>();
                // A first-class reference's return carries the STATE for the same
                // reason a direct call's does: without it `LET g = openTagged` would
                // launder the state away — `g(p)` would type as a bare `File`, and
                // binding that to `STATE Label` would read as a legal attach while
                // the runtime adopts and re-types openTagged's Cursor (plan-52-D §3).
                let returns = function_return_type(function);
                types.insert(
                    function.name.clone(),
                    ParameterType::Func(params, Box::new(returns), function.isolated),
                );
            }
        }
    }
    // Native LINK functions, keyed `alias.func`, so first-class references to them
    // type correctly during IR lowering (plan-link-update.md §5b).
    for file in &hir.files {
        for item in &file.items {
            if let HirItem::Link(link) = item {
                for native in &link.functions {
                    let params = native
                        .params
                        .iter()
                        .map(|param| param.type_.clone().unwrap_or(ParameterType::Unknown))
                        .collect::<Vec<_>>();
                    let returns = native_type(native.return_type.as_ref());
                    // Stateful native producer: carry its STATE in the callable
                    // type too (plan-53-A/B), matching `native_returns` above.
                    let returns = match (native.return_resource, &native.return_state_type) {
                        (true, Some(state)) => returns.with_state(state),
                        _ => returns,
                    };
                    // A LINK native is never `ISOLATED`, matching the un-prefixed
                    // `FUNC(…) AS …` spelling this used to `format!`.
                    types.insert(
                        format!("{}.{}", link.alias, native.name),
                        ParameterType::Func(params, Box::new(returns), false),
                    );
                }
            }
        }
    }
    types
}

pub(super) fn function_params(hir: &HirProject) -> HashMap<String, Vec<CallParam>> {
    let mut params = HashMap::new();
    for file in &hir.files {
        // A default is classified against the file that DECLARES it (plan-136-A).
        let imports = file.import_bindings();
        for item in &file.items {
            if let HirItem::Function(function) = item {
                params.insert(
                    function.name.clone(),
                    function
                        .params
                        .iter()
                        .enumerate()
                        .map(|(index, param)| CallParam {
                            name: param.name.clone(),
                            type_: param.type_.clone(),
                            default: param.default.as_ref().map(|default| {
                                call_default(&function.name, index, default, &imports)
                            }),
                        })
                        .collect(),
                );
            }
        }
    }
    params
}

/// Every native LINK function's parameters, keyed `alias.func` like
/// [`function_types`], so a LINK call that omits a defaulted argument passes its
/// default (plan-136-A — the omitted argument used to be simply not passed).
pub(super) fn link_params(hir: &HirProject) -> HashMap<String, Vec<CallParam>> {
    let mut params = HashMap::new();
    for file in &hir.files {
        let imports = file.import_bindings();
        for item in &file.items {
            if let HirItem::Link(link) = item {
                for native in &link.functions {
                    let function = format!("{}.{}", link.alias, native.name);
                    let call_params = native
                        .params
                        .iter()
                        .enumerate()
                        .map(|(index, param)| CallParam {
                            name: param.name.clone(),
                            type_: param.type_.clone().unwrap_or(ParameterType::Unknown),
                            default: param
                                .default
                                .as_ref()
                                .map(|default| call_default(&function, index, default, &imports)),
                        })
                        .collect();
                    params.insert(function, call_params);
                }
            }
        }
    }
    params
}

/// The type a top-level binding is known by: `type_` with the binding's `STATE T`
/// clause folded in, the way a local binding's and a parameter's type carry
/// theirs (bug-1030). Without the clause `g.state` has no state type to read.
pub(super) fn top_level_binding_type(
    binding: &HirTopLevelBinding,
    type_: ParameterType,
) -> ParameterType {
    match &binding.state_type {
        Some(state) => type_.with_state(state),
        None => type_,
    }
}

/// The type of a lambda parameter: its `STATE T` clause rides in the type,
/// exactly as a declared FUNC's parameter carries it (`lower_param`), bug-1030.
///
/// The lambda's own callable type carries the clause too. A function value is
/// called through that type alone, so it is the only place the rule "a
/// parameter observes a resource's state, it cannot re-type it" can be checked:
/// with the bare type there, `j(bareFile)` would hand a resource with no state
/// payload to a body that reads `x.state`.
pub(in crate::ir) fn lambda_param_type(param: &HirParam) -> ParameterType {
    match &param.state_type {
        Some(state) => param.type_.with_state(state),
        None => param.type_.clone(),
    }
}

pub(super) fn declared_binding_types(hir: &HirProject) -> HashMap<String, ParameterType> {
    let mut bindings = HashMap::new();
    for file in &hir.files {
        for item in &file.items {
            if let HirItem::Binding(binding) = item {
                bindings.insert(
                    binding.name.clone(),
                    top_level_binding_type(binding, binding.type_.clone()),
                );
            }
        }
    }
    bindings
}

/// The declared type of a LINK native's return slot, defaulting an absent
/// annotation to `Nothing` exactly as the string form did.
///
/// plan-111-B did the elaboration plan-106-E recorded as a task:
/// [`HirItem::Link`](crate::hir::HirItem::Link) now carries a
/// [`HirLinkBlock`](crate::hir::HirLinkBlock) whose types are already
/// `ParameterType`, so this is a defaulting helper rather than an AST→typed
/// boundary and nothing here parses.
fn native_type(declared: Option<&ParameterType>) -> ParameterType {
    declared.cloned().unwrap_or(ParameterType::Nothing)
}

pub(super) fn infer_binding_types(hir: &HirProject, context: &mut LowerContext<'_>) {
    for file in &hir.files {
        context.current_imports = file.import_bindings();
        for item in &file.items {
            if let HirItem::Binding(binding) = item {
                if binding.explicit_type {
                    continue;
                }
                if let Some(value) = &binding.value {
                    let locals = HashMap::new();
                    if let Some(type_) = expression_type(value, &locals, context) {
                        context
                            .binding_types
                            .insert(binding.name.clone(), top_level_binding_type(binding, type_));
                    }
                }
            }
        }
    }
}

pub(in crate::ir) fn expression_type(
    expression: &HirExpression,
    locals: &HashMap<String, ParameterType>,
    context: &LowerContext<'_>,
) -> Option<ParameterType> {
    match expression {
        HirExpression::String(_) => Some(ParameterType::String),
        HirExpression::Number(value) => Some(match numeric::classify_literal(value).1 {
            numeric::LiteralType::Integer => ParameterType::Integer,
            numeric::LiteralType::Float => ParameterType::Float,
            numeric::LiteralType::Fixed => ParameterType::Fixed,
            numeric::LiteralType::Money => ParameterType::Money,
        }),
        HirExpression::Scalar(_) => Some(ParameterType::named("Scalar")),
        HirExpression::Boolean(_) => Some(ParameterType::Boolean),
        HirExpression::Identifier(value) if value == "NOTHING" => Some(ParameterType::Nothing),
        HirExpression::Identifier(value) => {
            let canonical_value = canonical_import_name(value, context);
            if builtins::is_package_constant(&canonical_value) {
                builtins::package_constant_type(&canonical_value)
            } else {
                locals
                    .get(value)
                    .cloned()
                    .or_else(|| context.binding_types.get(value).cloned())
                    // bug-551: an imported package's exported global is keyed by
                    // the CANONICAL `package.Name`, so an aliased `IMPORT cst AS
                    // c` reaches it through `c.Answer` too.
                    .or_else(|| context.binding_types.get(&canonical_value).cloned())
                    .or_else(|| context.function_types.get(value).cloned())
                    .or_else(|| context.function_types.get(&canonical_value).cloned())
            }
        }
        HirExpression::Constructor { type_, .. } => {
            let canonical_type = canonical_import_type(type_, context);
            context
                .type_index
                .constructor_result(&canonical_type)
                .or_else(|| context.type_index.constructor_result(type_))
        }
        HirExpression::WithUpdate { target, .. } => expression_type(target, locals, context),
        HirExpression::ListLiteral(values) => {
            let Some(first) = values.first() else {
                return Some(ParameterType::list_of(ParameterType::Unknown));
            };
            expression_type(first, locals, context).map(ParameterType::list_of)
        }
        HirExpression::SetLiteral { element_type, .. } => {
            Some(ParameterType::set_of(element_type.clone()))
        }
        HirExpression::MapLiteral {
            key_type,
            value_type,
            ..
        } => Some(ParameterType::map_of(key_type.clone(), value_type.clone())),
        HirExpression::MemberAccess { target, member } => {
            if let HirExpression::Identifier(type_name) = target.as_ref() {
                if context
                    .type_index
                    .enums
                    .get(&ParameterType::declared(type_name))
                    .is_some_and(|members| members.iter().any(|name| name == member))
                {
                    return Some(ParameterType::declared(type_name));
                }
            }
            if let Some(bare) = qualified_imported_enum(target, member, context) {
                return Some(ParameterType::declared(&bare));
            }
            let target_type = expression_type(target, locals, context)?;
            // `s.state` on a `RES` value yields its `STATE` record type, split
            // out structurally (plan-106-C's `ParameterType::state`).
            if member == "state" {
                if let Some(state) = target_type.state() {
                    return Some(state);
                }
            }
            // `t.result` is removed; worker outcomes are retrieved only via
            // `thread::waitFor`. (Typecheck rejects `.result` before IR.)
            // `Error`/`ErrorLoc` are the compiler-generated records
            // (`build_error_value`): their fields are typed here exactly as the
            // source checker typed them, so `e.source.line` is an `Integer` to
            // every consumer of this seam (plan-107-E).
            if target_type == ParameterType::named("Error") {
                return match member.as_str() {
                    "code" => Some(ParameterType::Integer),
                    "message" => Some(ParameterType::String),
                    "source" => Some(ParameterType::named("ErrorLoc")),
                    _ => None,
                };
            }
            if target_type == ParameterType::named("ErrorLoc") {
                return match member.as_str() {
                    "filename" => Some(ParameterType::String),
                    "line" | "char" => Some(ParameterType::Integer),
                    _ => None,
                };
            }
            if let ParameterType::MapEntryOf(key_type, value_type) = &target_type {
                return match member.as_str() {
                    "key" => Some((**key_type).clone()),
                    "value" => Some((**value_type).clone()),
                    _ => None,
                };
            }
            context.type_index.record_field_type(&target_type, member)
        }
        HirExpression::Call {
            callee, arguments, ..
        } => {
            let canonical_callee = canonical_import_name(callee, context);
            if crate::codegen::builtins::general::is_general_call(&canonical_callee) {
                let normalized =
                    normalize_builtin_call_arguments(canonical_callee.as_str(), arguments);
                if crate::codegen::registry::callback_member_bare(callee) && normalized.len() == 2 {
                    if let HirExpression::Identifier(predicate) = normalized[1] {
                        if let Some(collection_type) =
                            expression_type(normalized[0], locals, context)
                        {
                            if let Some(predicate_type) =
                                filter_predicate_arg_type(predicate, &collection_type)
                            {
                                let arg_types = vec![collection_type, predicate_type];
                                return builtins::resolve_call_return_type_typed(
                                    &canonical_callee,
                                    &arg_types,
                                    false,
                                );
                            }
                        }
                    }
                }
                let arg_types = normalized
                    .iter()
                    .map(|argument| expression_type(argument, locals, context))
                    .collect::<Option<Vec<_>>>()?;
                // plan-140-B: with the type index as the enum oracle, so
                // `toInt(<enum>)` types as `Integer`.
                let resolved = builtins::resolve_call_return_type_with_kinds(
                    &canonical_callee,
                    &arg_types,
                    false,
                    context.type_index,
                );
                // A package-provided override of an overridable general builtin
                // (`toString(net::Url)` → the package's renderer, plan-01-overload
                // §B.2) yields the builtin's conventional result type — the same
                // answer the source checker gave, so the seam types it (plan-107-E).
                if resolved.is_none()
                    && crate::codegen::builtins::general::is_overridable(&canonical_callee)
                    && arg_types.len() == 1
                    && builtins::general_override_target(&canonical_callee, &arg_types[0]).is_some()
                {
                    return crate::codegen::builtins::general::override_result_type(
                        &canonical_callee,
                    )
                    .map(ParameterType::parse);
                }
                return resolved;
            }
            if crate::codegen::registry::abi_inline_lower(&canonical_callee).is_some() {
                let normalized =
                    normalize_builtin_call_arguments(canonical_callee.as_str(), arguments);
                if crate::codegen::registry::callback_member(&canonical_callee)
                    && normalized.len() == 2
                {
                    if let HirExpression::Identifier(predicate) = normalized[1] {
                        if let Some(collection_type) =
                            expression_type(normalized[0], locals, context)
                        {
                            if let Some(predicate_type) =
                                filter_predicate_arg_type(predicate, &collection_type)
                            {
                                let arg_types = vec![collection_type, predicate_type];
                                return builtins::resolve_call_return_type_typed(
                                    &canonical_callee,
                                    &arg_types,
                                    false,
                                );
                            }
                        }
                    }
                }
                let arg_types = normalized
                    .iter()
                    .map(|argument| expression_type(argument, locals, context))
                    .collect::<Option<Vec<_>>>()?;
                return generic_builtin_call_type(
                    &canonical_callee,
                    &arg_types,
                    &normalized,
                    context,
                );
            }
            // The remaining builtin packages share one arg-typed dispatch
            // (bug-342 A1 — was 17 byte-identical is_*_call → resolve_call
            // blocks). Gate on exactly this package set rather than the
            // all-packages `builtins::is_builtin_call`: `encoding`, `money`, and
            // `term` deliberately resolve through the name-based
            // `call_return_type_name` fallthrough below, not here, so which
            // calls resolve at this point is byte-for-byte unchanged. The
            // shared `resolve_call_return_type` dispatches in the same order as
            // `ir::verify`, keeping the two return-type oracles in lockstep.
            //
            // `encoding` migrated to the clean-room registry, so it now matches
            // `registry::is_member`; it is excluded here so it keeps resolving via
            // the static `call_return_type_name` fallthrough (its fixed nominal
            // return type is reported even for an argument-invalid call, which is
            // the byte-identical pre-migration behavior — see the encoding
            // `func_*_invalid` acceptance goldens).
            //
            // `term` migrated to the clean-room registry too, but its return type is a
            // function of the NAME alone (the legacy `TermResolver` ignored argument
            // types), so it is likewise excluded and keeps resolving via the name-based
            // `call_return_type_name` fallthrough. That remains correct: every term
            // member is a single overload with one fixed return, so the name decides it.
            //
            // The exclusion's ORIGINAL motivation is gone, and saying so matters because
            // it read as the reason. It was that routing term through the arg-typed path
            // would mis-resolve a `Byte`-parameter setter called with `Integer` literals
            // — `term::setForeground(255, 128, 0)`, whose un-coerced `Integer` arguments
            // fail to match `Byte` parameters, regressing `Nothing` to `Unknown`.
            // plan-122-F gave those setters a single `color::Color` parameter, and
            // `grep -rn "ParameterType::Byte" src/codegen/builtins/term/*.rs` now returns
            // nothing, so no term member has a `Byte` parameter left to mis-resolve. The
            // exclusion is kept because it is harmless and name-resolution is right for
            // this package, not because that hazard still exists.
            let owner = crate::codegen::registry::registry().owning_package(&canonical_callee);
            let migrated_arg_typed = crate::codegen::registry::registry()
                .is_member(&canonical_callee)
                && owner != Some("encoding")
                && owner != Some("term");
            if
            // `astrings`/`strings`/`math`/`vector`/`fs`/`io`/`net`/`tls`/`http`/
            // `audio` migrated to the clean-room registry — covered by
            // `migrated_arg_typed` (`registry::is_member`) below.
            migrated_arg_typed
                || crate::codegen::registry::registry().owning_package(&canonical_callee)
                    == Some("datetime")
                // `crypto` migrated to the clean-room registry — covered by
                // `migrated_arg_typed` (`registry::is_member`) above.
                || crate::codegen::builtins::thread::is_thread_call(&canonical_callee)
            {
                let normalized =
                    normalize_builtin_call_arguments(canonical_callee.as_str(), arguments);
                let arg_types = normalized
                    .iter()
                    .map(|argument| expression_type(argument, locals, context))
                    .collect::<Option<Vec<_>>>()?;
                return generic_builtin_call_type(
                    &canonical_callee,
                    &arg_types,
                    &normalized,
                    context,
                );
            }
            builtins::call_return_type(&canonical_callee)
                .or_else(|| context.function_returns.get(callee).cloned())
                .or_else(|| context.function_returns.get(&canonical_callee).cloned())
                .or_else(|| locals.get(callee).and_then(function_return_from_type))
                .or_else(|| {
                    // A global binding holding a function value is callable too
                    // (bug-198): infer its return type from the declared FUNC type,
                    // mirroring the local-binding fallback above.
                    context
                        .binding_types
                        .get(callee)
                        .and_then(function_return_from_type)
                })
        }
        HirExpression::Lambda {
            params,
            body,
            assign_target,
        } => {
            let mut nested = locals.clone();
            let param_types = params
                .iter()
                .map(|param| {
                    let type_ = lambda_param_type(param);
                    nested.insert(param.name.clone(), type_.clone());
                    type_
                })
                .collect::<Vec<_>>();
            // An assignment-bodied lambda yields `Nothing`.
            let returns = if assign_target.is_some() {
                ParameterType::Nothing
            } else {
                expression_type(body, &nested, context)?
            };
            // A lambda is never `ISOLATED` (that marker is only written on a
            // declared FUNC), reproducing the un-prefixed `FUNC(…) AS …` spelling
            // this arm used to `format!`.
            Some(ParameterType::Func(param_types, Box::new(returns), false))
        }
        HirExpression::Binary {
            left,
            operator,
            right,
            ..
        } => {
            if operator.is_comparison()
                || matches!(operator, BinaryOp::And | BinaryOp::Or | BinaryOp::Xor)
            {
                return Some(ParameterType::Boolean);
            }
            if *operator == BinaryOp::Concat {
                // plan-89-D: `AttributedString & AttributedString` yields an
                // AttributedString (both operands attributed); otherwise String.
                //
                // NB `AttributedString` is a NOMINAL, not the `ParameterType::
                // AttributeString` variant — that variant renders
                // `"AttributeString"` (no `d`), a different spelling the language
                // never uses here, so `parse("AttributedString")` yields
                // `Named("AttributedString")` and the comparison must too.
                if expression_type(left, locals, context) == Some(attributed_string_type()) {
                    return Some(attributed_string_type());
                }
                return Some(ParameterType::String);
            }
            let left = expression_type(left, locals, context)?;
            let right = expression_type(right, locals, context)?;
            Some(
                numeric::typed_binary_result_type(*operator, &left, &right)
                    .unwrap_or(ParameterType::Integer),
            )
        }
        HirExpression::Unary {
            operator, operand, ..
        } => {
            if *operator == UnaryOp::Not {
                Some(ParameterType::Boolean)
            } else {
                expression_type(operand, locals, context)
            }
        }
        HirExpression::Trapped { expression, .. } => expression_type(expression, locals, context),
    }
}

/// The return type of a declared function-value type, or `None` when it is not a
/// `FUNC(…) AS R` at all.
///
/// plan-106-A: the input arrives as a [`ParameterType`] (a local's or a global
/// binding's declared type, now held typed in the lowering context), so this is a
/// variant match. It used to `ParameterType::parse` a rendered spelling — itself
/// already an improvement over the private `strip_prefix("FUNC(")` splitter that
/// preceded it, which cut at the FIRST `") AS "` and split parameters on a bare
/// `", "`, mis-typing a higher-order declared type
/// (`FUNC(FUNC(Integer) AS String) AS File` → return `String) AS File`).
fn function_return_from_type(type_: &ParameterType) -> Option<ParameterType> {
    match type_ {
        ParameterType::Func(_, returns, _) => Some((**returns).clone()),
        _ => None,
    }
}

/// The parameter types of a declared function-value type — the sibling of
/// [`function_return_from_type`].
pub(super) fn function_param_types_from_type(type_: &ParameterType) -> Option<Vec<ParameterType>> {
    match type_ {
        ParameterType::Func(params, _, _) => Some(params.clone()),
        _ => None,
    }
}

/// Lower resource-plane thread calls to their dedicated runtime helpers. The
/// resource plane mirrors `send`/`receive` but runs on a separate per-thread
/// resource queue so a thread can carry both a data channel and a resource
/// channel at once (§7).
pub(super) fn thread_resource_plane_target(name: &str) -> &str {
    match name {
        "thread.transfer" => "thread.transferResource",
        "thread.accept" => "thread.acceptResource",
        other => other,
    }
}

/// [`canonical_import_name`] on a type, structurally.
///
/// plan-111-B. The name-domain form splits the WHOLE rendered spelling on its
/// FIRST `.` and looks the prefix up as an import binding, so it only ever
/// rewrites a HEAD nominal: `alias.Type` and `alias.Pair OF (A, B)` are rewritten,
/// while `List OF alias.T` looks up `"List OF alias"`, misses, and comes back
/// unchanged — as do a container's or a user generic's ARGUMENTS. This
/// reproduces exactly that, which is why it does not recurse.
/// bug-554: the enum TYPE behind a package-qualified enum-member read
/// (`pkg::Colour.Red`), or `None` when the target is not one.
///
/// The parser's type-position normalizer never sees this spelling because a
/// member read is a VALUE, so the target arrives as `binding.Colour`. bug-632: an
/// imported enum's identity is `<package>.Colour` (the import binding resolved
/// to its package), which is how `TypeIndex` keys it, so that is both the lookup
/// and the answer. It used to be the bare leaf, when imported types were
/// installed bare — the spelling that let a consumer's own `Colour` and the
/// package's collide.
///
/// Deliberately fails CLOSED: the prefix must be a live `IMPORT` binding, the
/// leaf must be a single segment, and the qualified name must already be a known
/// enum **declaring this very member**. Anything else keeps the spelling as
/// written, so resolution still reports it.
pub(super) fn qualified_imported_enum(
    target: &HirExpression,
    member: &str,
    context: &LowerContext<'_>,
) -> Option<String> {
    let HirExpression::Identifier(type_name) = target else {
        return None;
    };
    let (binding, leaf) = type_name.split_once('.')?;
    if leaf.contains('.') {
        return None;
    }
    let package = context.current_imports.get(binding)?;
    let qualified = format!("{package}.{leaf}");
    context
        .type_index
        .enums
        .get(&ParameterType::declared(&qualified))
        .is_some_and(|members| members.iter().any(|name| name == member))
        .then_some(qualified)
}

pub(super) fn canonical_import_type(
    type_: &ParameterType,
    context: &LowerContext<'_>,
) -> ParameterType {
    match type_ {
        ParameterType::Named(sym) => {
            ParameterType::named(&canonical_import_name(sym.resolve(), context))
        }
        ParameterType::UserOf(head, args) => ParameterType::user_of(
            &canonical_import_name(head.resolve(), context),
            args.clone(),
        ),
        other => other.clone(),
    }
}

/// The (lenient) return type of a built-in call resolved through the registry's
/// generic matcher, with the declared types as its oracle, so union-variant
/// subsumption reaches a generic call (bug-988, bug-989): `append(shapes,
/// Square[2.0])` and `append(shapes, squares)` both type `List OF Shape`.
///
/// A literal argument the selection only accepts re-typed to its slot
/// (`append(bytes, 7)`, see [`literal_retyped_argument_types`]) types the call
/// under that re-typing, the types its arguments lower as.
fn generic_builtin_call_type(
    canonical_callee: &str,
    arg_types: &[ParameterType],
    arguments: &[&HirExpression],
    context: &LowerContext<'_>,
) -> Option<ParameterType> {
    builtins::resolve_call_return_type_with_kinds(
        canonical_callee,
        arg_types,
        false,
        context.type_index,
    )
    .or_else(|| {
        let retyped = literal_retyped_argument_types(
            canonical_callee,
            arg_types,
            arguments,
            context.type_index,
        )?;
        builtins::resolve_call_return_type_with_kinds(
            canonical_callee,
            &retyped,
            false,
            context.type_index,
        )
    })
}

pub(super) fn canonical_import_name(name: &str, context: &LowerContext<'_>) -> String {
    canonical_import_name_in(name, &context.current_imports)
}

/// [`canonical_import_name`] against an explicit file's import bindings — the
/// DECLARING file's, for a parameter default (plan-136-A).
pub(super) fn canonical_import_name_in(name: &str, imports: &HashMap<String, String>) -> String {
    let Some((binding, rest)) = name.split_once('.') else {
        return name.to_string();
    };
    let Some(package) = imports.get(binding) else {
        return name.to_string();
    };
    // `IMPORT self` binds the current package's own exported interface, so a
    format!("{package}.{rest}")
}
