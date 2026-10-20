//! plan-176-N: IR lowering of declarations — types, bindings, fields, variants, enum members,
//! functions, parameters and their defaults. The lowering context and entry points live in
//! `ir/lower/mod.rs`.

use super::*;

pub(super) fn lower_type(type_decl: &HirTypeDecl, type_index: &TypeIndex, file: &str) -> IrType {
    let kind = match type_decl.kind {
        TypeDeclKind::Type => "type",
        TypeDeclKind::Union => "union",
        TypeDeclKind::Enum => "enum",
    };
    IrType {
        kind: kind.to_string(),
        visibility: visibility_name(type_decl.visibility).to_string(),
        name: type_decl.name.clone(),
        fields: type_decl.fields.iter().map(lower_field).collect(),
        // The IR/`.mfp` wire keeps `includes` as spellings; HIR carries them
        // typed (plan-111-B), so they render here at the encode boundary.
        includes: type_decl
            .includes
            .iter()
            .map(|i| i.name().into_owned())
            .collect(),
        variants: type_decl
            .variants
            .iter()
            .map(|variant| lower_variant(variant, type_index))
            .collect(),
        members: type_decl.members.iter().map(lower_enum_member).collect(),
        loc: IrSourceLoc {
            line: type_decl.line as u32,
            column: 1,
        },
        file: file.to_string(),
    }
}

fn lower_binding(binding: &HirTopLevelBinding, context: &mut LowerContext<'_>) -> IrBinding {
    let loc = IrSourceLoc {
        line: binding.line as u32,
        column: 1,
    };
    context.current_loc = loc;
    // A top-level initializer has no locals (bug-612). Seeding this scope with
    // `binding_types` made every global it read lower as `IrValue::Local`, which
    // nothing defines: the `Identifier` arm consults `locals` before its
    // `binding_types` arms, and the package merge qualifies only `Global`s.
    let locals = HashMap::new();
    let type_ = if binding.explicit_type {
        binding.type_.clone()
    } else {
        binding
            .value
            .as_ref()
            .and_then(|value| expression_type(value, &locals, context))
            .unwrap_or(ParameterType::Unknown)
    };
    let value = binding
        .value
        .as_ref()
        .map(|value| lower_expression_with_expected(value, Some(&type_), &locals, context));
    IrBinding {
        name: binding.name.clone(),
        visibility: visibility_name(binding.visibility).to_string(),
        mutable: binding.mutable,
        // A top-level `RES` binding's `STATE T` rides in the lowered type, as a
        // local binding's does (`lower_statement`'s `Let` arm), bug-1030.
        type_: top_level_binding_type(binding, type_),
        value,
        loc,
        file: context.current_file.clone(),
        explicit_type: binding.explicit_type,
    }
}

pub(super) fn lower_bindings(hir: &HirProject, context: &mut LowerContext<'_>) -> Vec<IrBinding> {
    let mut lowered = Vec::new();
    for file in &hir.files {
        context.current_imports = file.import_bindings();
        context.current_file = file.path.clone();
        for item in &file.items {
            if let HirItem::Binding(binding) = item {
                lowered.push(lower_binding(binding, context));
            }
        }
    }
    lowered
}

pub(super) fn lower_field(field: &HirTypeField) -> IrField {
    IrField {
        visibility: field.visibility.map(visibility_name).map(str::to_string),
        name: field.name.clone(),
        type_: field.type_.clone(),
        loc: IrSourceLoc {
            line: field.line as u32,
            column: 1,
        },
    }
}

fn lower_variant(variant: &crate::hir::HirUnionVariant, type_index: &TypeIndex) -> IrVariant {
    IrVariant {
        name: variant.type_.name().into_owned(),
        // bug-657: keep the elaborated type, not just its rendering.
        type_: variant.type_.clone(),
        fields: type_index
            .records
            .get(&variant.type_)
            .cloned()
            .unwrap_or_default(),
        loc: IrSourceLoc {
            line: variant.line as u32,
            column: 1,
        },
    }
}

fn lower_enum_member(member: &EnumMember) -> IrEnumMember {
    IrEnumMember {
        name: member.name.clone(),
    }
}

/// A function's return type string, carrying its `STATE T` clause when it
/// declares one (plan-52-D) — mirroring what `lower_function` does for a `RES`
/// parameter and `lower_binding` for a `RES` binding.
///
/// Every site that derives a return type calls this, because the STATE must be in
/// the string uniformly or not at all: the string is what `check_return_type`
/// compares, what the STATE verify rules pattern-match `" STATE "` on, what
/// `.state` typing on a call expression reads, and what rides the `.mfp` as
/// `IrFunction.returns`. The append was missing here alone, which both **rejected**
/// the legal stateful `RETURN` (expected `File`, actual `File STATE Cursor`) and
/// **hid** the union-STATE / non-defaultable-STATE rules from a return, since a
/// return's string never contained `" STATE "` for them to match.
///
/// plan-106-A: the `STATE` fold is [`ParameterType::with_state`], the structural
/// equivalent of parsing the concatenated spelling (guarded by
/// `with_state_matches_parse_of_the_concatenated_spelling` in `src/types.rs`), so
/// the clause is attached without a render→parse round trip.
pub(in crate::ir) fn function_return_type(function: &HirFunction) -> ParameterType {
    match (&function.return_state_type, function.return_resource) {
        (Some(state), true) => function.returns.with_state(state),
        _ => function.returns.clone(),
    }
}

pub(super) fn lower_function(function: &HirFunction, context: &mut LowerContext<'_>) -> IrFunction {
    // bug-695: name every resource produced straight into a container, before the
    // escape analysis below and the body lowering both read the function.
    let hoisted = crate::ir::resource_hoist::hoist_resource_temporaries(function, context);
    let function = &hoisted;
    let kind = "func";
    let returns = function_return_type(function);
    let mut locals = HashMap::new();
    for param in &function.params {
        // Carry a resource parameter's `STATE T` in the local's type so `s.state`
        // resolves inside the callee, matching `lower_param`.
        let type_ = match &param.state_type {
            Some(state) => param.type_.with_state(state),
            None => param.type_.clone(),
        };
        locals.insert(param.name.clone(), type_);
    }
    let previous_return_type = context.current_return_type.take();
    let previous_loop_targets = std::mem::take(&mut context.loop_targets);
    context.current_return_type = Some(returns.clone());
    let body = lower_function_body(function, &locals, context);
    context.current_return_type = previous_return_type;
    context.loop_targets = previous_loop_targets;

    IrFunction {
        name: function.name.clone(),
        visibility: visibility_name(function.visibility).to_string(),
        kind: kind.to_string(),
        isolated: function.isolated,
        params: function
            .params
            .iter()
            .enumerate()
            .map(|(index, param)| lower_param(&function.name, index, param, context))
            .collect(),
        returns,
        body,
        file: context.current_file.clone(),
        loc: IrSourceLoc {
            line: function.line as u32,
            column: 1,
        },
        // plan-114-C: the escape analysis needs to know which record types carry
        // a `RES` field, because a record's type name alone cannot say. Without
        // it the bug-291 ordering gate cannot recognise a returned
        // resource-carrying record and degrades it to `Local` — a double close.
        //
        // bug-806: and which names are module-level bindings, so a store into one
        // is routed to the module instance rather than to the storing scope.
        resource_owners: crate::ir::resource_escape::analyze_function_with(
            function,
            &context.type_index.res_field_record_types(),
            &context.binding_types.keys().cloned().collect(),
        )
        .owners()
        .clone(),
    }
}

fn lower_function_body(
    function: &HirFunction,
    locals: &HashMap<String, ParameterType>,
    context: &mut LowerContext<'_>,
) -> Vec<IrOp> {
    // The function-level TRAP body sees the function body's own (top-level)
    // locals as well as the parameters (bug-285: a body local shadowing a
    // file PRIVATE wins in the trap too), so the body is lowered into a scope
    // the trap then inherits — typing `x` in the handler as the body's `x`
    // rather than leaving it `Unknown` (plan-107-E).
    let mut body_locals = locals.clone();
    let mut body: Vec<IrOp> = function
        .body
        .iter()
        .flat_map(|statement| lower_statement(statement, &mut body_locals, context, None))
        .collect();
    if let Some(trap) = &function.trap {
        let mut trap_locals = body_locals;
        trap_locals.insert(trap.name.clone(), ParameterType::named("Error"));
        body.push(IrOp::Trap {
            name: trap.name.clone(),
            body: lower_statement_block(
                &trap.body,
                &trap_locals,
                context,
                Some(trap.name.as_str()),
            ),
            loc: IrSourceLoc {
                line: trap.line as u32,
                column: 1,
            },
        });
    }
    body
}

fn lower_param(
    function: &str,
    index: usize,
    param: &HirParam,
    context: &mut LowerContext<'_>,
) -> IrParam {
    // A resource parameter's `STATE T` rides inside its type's nominal spelling so
    // the callee can address the pointed-to resource's shared state payload.
    let type_ = match &param.state_type {
        Some(state) => param.type_.with_state(state),
        None => param.type_.clone(),
    };
    IrParam {
        name: param.name.clone(),
        type_: type_.clone(),
        // The default is typed by the parameter, exactly as the call site
        // fills it (`lower_call_default`): a bare numeric literal — or a list of
        // them — coerces to a `Fixed`/`Money`/`Float` parameter here too, so
        // `a AS List OF Fixed = [1, 2]` lowers as the `List OF Fixed` the
        // declaration names rather than a `List OF Integer` the default-value
        // rule then rejects. plan-136-A: it is lowered in the declaration's
        // scope, and a computed one is the call to its hidden default function.
        default: param.default.as_ref().map(|value| {
            lower_parameter_default(function, index, &type_, value, param.line, context)
        }),
        loc: IrSourceLoc {
            line: param.line as u32,
            column: 1,
        },
    }
}

/// Lower parameter `index`'s default of `function` in the DECLARATION's scope
/// (plan-136-A): with no locals, and with the declaring file's imports (the
/// caller's context is never involved).
///
/// A [`DefaultKind::Literal`] lowers to its value. A [`DefaultKind::Computed`]
/// default is lowered once into the body of a hidden, private, parameterless
/// function — `RETURN <default>` through ordinary statement lowering, so every
/// statement-position desugar (an inline `TRAP`) applies — and the default
/// becomes a zero-argument call to it. Each call that omits the argument makes
/// that call, so the default is evaluated on every such call.
fn lower_parameter_default(
    function: &str,
    index: usize,
    type_: &ParameterType,
    default: &HirExpression,
    line: usize,
    context: &mut LowerContext<'_>,
) -> IrValue {
    match default_kind(default, &context.current_imports) {
        DefaultKind::Literal => {
            lower_expression_with_expected(default, Some(type_), &HashMap::new(), context)
        }
        DefaultKind::Computed => {
            let name = crate::internal_name::hidden_default_function_name(function, index);
            let loc = IrSourceLoc {
                line: line as u32,
                column: 1,
            };
            let previous_return_type = context.current_return_type.replace(type_.clone());
            let previous_loop_targets = std::mem::take(&mut context.loop_targets);
            let body = lower_statement(
                &HirStatement::Return {
                    value: Some(default.clone()),
                    line,
                },
                &mut HashMap::new(),
                context,
                None,
            );
            context.current_return_type = previous_return_type;
            context.loop_targets = previous_loop_targets;
            context.lambdas.push(IrFunction {
                name: name.clone(),
                visibility: "private".to_string(),
                kind: "func".to_string(),
                isolated: false,
                params: Vec::new(),
                returns: type_.clone(),
                body,
                file: context.current_file.clone(),
                loc,
                resource_owners: HashMap::new(),
            });
            IrValue::Call {
                target: name,
                args: Vec::new(),
                type_: type_.clone(),
                loc,
            }
        }
    }
}

/// Synthesize the hidden default function of every computed parameter default in
/// a LINK block (plan-136-A). A LINK function has no IR body, so its defaults are
/// used only by the calls `link_params` fills.
pub(super) fn lower_link_defaults(link: &crate::hir::HirLinkBlock, context: &mut LowerContext<'_>) {
    for native in &link.functions {
        let function = format!("{}.{}", link.alias, native.name);
        for (index, param) in native.params.iter().enumerate() {
            let Some(default) = &param.default else {
                continue;
            };
            if default_kind(default, &context.current_imports) == DefaultKind::Computed {
                let type_ = param.type_.clone().unwrap_or(ParameterType::Unknown);
                let _ =
                    lower_parameter_default(&function, index, &type_, default, param.line, context);
            }
        }
    }
}

/// The source line of a statement, as `IrSourceLoc` (column 1 — diagnostics
/// report statement-level positions).
pub(super) fn statement_loc(statement: &HirStatement) -> IrSourceLoc {
    let line = match statement {
        HirStatement::Let { line, .. }
        | HirStatement::Return { line, .. }
        | HirStatement::Exit { line, .. }
        | HirStatement::Continue { line, .. }
        | HirStatement::Fail { line, .. }
        | HirStatement::Propagate { line }
        | HirStatement::Recover { line, .. }
        | HirStatement::Assign { line, .. }
        | HirStatement::StateAssign { line, .. }
        | HirStatement::Expression { line, .. }
        | HirStatement::If { line, .. }
        | HirStatement::Match { line, .. }
        | HirStatement::For { line, .. }
        | HirStatement::ForEach { line, .. }
        | HirStatement::While { line, .. }
        | HirStatement::DoUntil { line, .. } => *line,
    };
    IrSourceLoc {
        line: line as u32,
        column: 1,
    }
}

/// Classify a parameter default. A total match with no wildcard arm, so a new
/// `HirExpression` variant is a build error here rather than a silent verdict.
/// `imports` are the declaring file's import bindings.
pub(in crate::ir) fn default_kind(
    default: &HirExpression,
    imports: &HashMap<String, String>,
) -> DefaultKind {
    match default {
        HirExpression::String(_)
        | HirExpression::Number(_)
        | HirExpression::Scalar(_)
        | HirExpression::Boolean(_) => DefaultKind::Literal,
        HirExpression::Identifier(name) if name == "NOTHING" => DefaultKind::Literal,
        // The identifier arm of `lower_expression_with_expected` folds a built-in
        // package constant to a `Const` — unless it is a record constant, which it
        // inlines as a constructor first.
        HirExpression::Identifier(name) => {
            let canonical = canonical_import_name_in(name, imports);
            if registry_record_constant(&canonical).is_none()
                && builtins::is_package_constant(&canonical)
            {
                DefaultKind::Literal
            } else {
                DefaultKind::Computed
            }
        }
        HirExpression::Binary { .. }
        | HirExpression::Unary { .. }
        | HirExpression::Call { .. }
        | HirExpression::Lambda { .. }
        | HirExpression::Constructor { .. }
        | HirExpression::WithUpdate { .. }
        | HirExpression::ListLiteral(_)
        | HirExpression::SetLiteral { .. }
        | HirExpression::MapLiteral { .. }
        | HirExpression::MemberAccess { .. }
        | HirExpression::Trapped { .. } => DefaultKind::Computed,
    }
}

/// The call-site fill for parameter `index`'s `default` of `function`, whose file
/// has `imports` (plan-136-A).
pub(super) fn call_default(
    function: &str,
    index: usize,
    default: &HirExpression,
    imports: &HashMap<String, String>,
) -> CallDefault {
    match default_kind(default, imports) {
        DefaultKind::Literal => CallDefault::Literal(match default {
            HirExpression::Identifier(name) => {
                HirExpression::Identifier(canonical_import_name_in(name, imports))
            }
            literal => literal.clone(),
        }),
        DefaultKind::Computed => CallDefault::Function(
            crate::internal_name::hidden_default_function_name(function, index),
        ),
    }
}

/// Fill an omitted defaulted argument at a call site, or `None` when `param` has
/// no default (plan-136-A). A literal lowers with NO locals and a computed default
/// is a call to its hidden default function, so a caller's locals can never
/// capture a default's names (bug-614).
pub(super) fn lower_call_default(
    param: &CallParam,
    context: &mut LowerContext<'_>,
) -> Option<IrValue> {
    match param.default.as_ref()? {
        CallDefault::Literal(literal) => Some(lower_expression_with_expected(
            literal,
            Some(&param.type_),
            &HashMap::new(),
            context,
        )),
        CallDefault::Function(name) => Some(IrValue::Call {
            target: name.clone(),
            args: Vec::new(),
            type_: param.type_.clone(),
            loc: context.current_loc,
        }),
        CallDefault::Constant(value) => Some(value.clone()),
    }
}
