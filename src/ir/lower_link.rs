use super::*;

/// Collect `CSTRUCT` C-layout declarations from every `LINK` block (plan-50-B).
///
/// Field order is preserved verbatim: it is the layout input, so reordering here
/// would silently change every offset.
pub(super) fn link_cstructs(ast: &AstProject) -> Vec<crate::ir::IrCStruct> {
    let mut cstructs = Vec::new();
    for file in &ast.files {
        for item in &file.items {
            if let Item::Link(link) = item {
                for decl in &link.cstructs {
                    cstructs.push(crate::ir::IrCStruct {
                        alias: link.alias.clone(),
                        name: decl.name.clone(),
                        maps_to: decl.maps_to.clone(),
                        fields: decl
                            .fields
                            .iter()
                            .map(|f| crate::ir::IrCStructField {
                                name: f.name.clone(),
                                ctype: f.ctype.clone(),
                            })
                            .collect(),
                    });
                }
            }
        }
    }
    cstructs
}

/// Collect native `LINK` functions from the AST with their full ABI surface so
/// the backend can emit marshaling thunks and dlopen/dlsym initializers
/// (plan-linker.md §12).
pub(super) fn link_functions(ast: &AstProject) -> Vec<IrLinkFunction> {
    use crate::ir::IrBuffer;

    let mut functions = Vec::new();
    for file in &ast.files {
        for item in &file.items {
            if let Item::Link(link) = item {
                for native in &link.functions {
                    functions.push(IrLinkFunction {
                        alias: link.alias.clone(),
                        name: native.name.clone(),
                        library: link.library.clone(),
                        symbol: native.symbol.clone(),
                        params: native
                            .params
                            .iter()
                            .map(|param| {
                                let type_ = param
                                    .type_name
                                    .clone()
                                    .unwrap_or_else(|| "Unknown".to_string());
                                // plan-53-A: carry a `RES p AS R STATE S` param's
                                // STATE inline (`R STATE S`), so the close thunk can
                                // detect a stateful native resource param and load
                                // the handle from FD@0 of its record instead of
                                // passing the record pointer to the native symbol.
                                let type_ = match (param.resource, &param.state_type) {
                                    (true, Some(state)) => format!("{type_} STATE {state}"),
                                    _ => type_,
                                };
                                (param.name.clone(), type_)
                            })
                            .collect(),
                        return_type: native
                            .return_type
                            .clone()
                            .unwrap_or_else(|| "Nothing".to_string()),
                        return_resource: native.return_resource,
                        return_state_type: native.return_state_type.clone(),
                        abi_slots: native
                            .abi
                            .slots
                            .iter()
                            .map(|slot| IrAbiSlot {
                                name: slot.name.clone(),
                                ctype: slot.ctype.clone(),
                                direction: slot.direction,
                            })
                            .collect(),
                        abi_return_name: native.abi.return_name.clone(),
                        abi_return_ctype: native.abi.return_ctype.clone(),
                        consts: native
                            .consts
                            .iter()
                            .map(|pin| {
                                (
                                    pin.slot.clone(),
                                    eval_link_const(&pin.value, &link.cstructs),
                                )
                            })
                            .collect(),
                        bind_in: native
                            .bind_in
                            .iter()
                            .map(|b| crate::ir::IrBindIn {
                                slot: b.slot.clone(),
                                fields: b
                                    .fields
                                    .iter()
                                    .map(|f| lower_bind_in_field(f, &native.params))
                                    .collect(),
                            })
                            .collect(),
                        bind_state: native.bind_state.as_ref().map(|b| b.struct_slot.clone()),
                        bind_state_resource: native
                            .bind_state
                            .as_ref()
                            .map(|b| b.resource_slot.clone()),
                        success_on: native.success_on.as_ref().map(lower_link_expr),
                        result: native.result.as_ref().map(lower_link_expr),
                        free: native.free.as_ref().map(|f| IrFree {
                            slot: f.slot.clone(),
                            symbol: f.symbol.clone(),
                        }),
                        buffers: native
                            .buffers
                            .iter()
                            .map(|b| IrBuffer {
                                slot: b.slot.clone(),
                                size: lower_link_expr(&b.size),
                            })
                            .collect(),
                        result_length: native.result_length.as_ref().map(lower_link_expr),
                    });
                }
            }
        }
    }
    functions
}

/// Collect re-export aliases whose target is a native `LINK` function
/// (plan-link-update.md §5a).
pub(super) fn link_aliases(ast: &AstProject) -> Vec<(String, String)> {
    let mut link_targets: HashSet<String> = HashSet::new();
    for file in &ast.files {
        for item in &file.items {
            if let Item::Link(link) = item {
                for native in &link.functions {
                    link_targets.insert(format!("{}.{}", link.alias, native.name));
                }
            }
        }
    }
    let mut aliases = Vec::new();
    for file in &ast.files {
        for item in &file.items {
            if let Item::FuncAlias(alias) = item {
                if link_targets.contains(&alias.target) {
                    aliases.push((alias.name.clone(), alias.target.clone()));
                }
            }
        }
    }
    aliases
}

/// Resolve a `CONST slot = value` pin to an integer immediate (plan-link-update.md
/// §5c). `NOTHING` is a NULL pointer (`0`); numbers and a leading unary minus are
/// honored; booleans map to `0`/`1`.
/// Fold a `CONST` pin to its 64-bit immediate (plan-link-update.md §5c).
///
/// `cstructs` are the owning LINK block's declarations, so `SIZEOF <CStruct>`
/// folds to the layout `compute_c_layout` derives.
///
/// Returns `None` for a form this cannot fold. Every caller must treat that as an
/// error: until plan-50-G this ended in `_ => 0`, silently pinning **0** for any
/// unrecognized expression — the same "default rather than diagnose" mistake as
/// the unvalidated slot ctype (plan-50-A) and the nameless link-expr `Var`
/// (plan-50-I). `syntaxcheck` rejects an unfoldable pin, so by lowering the form
/// is already known-good.
fn eval_link_const_opt(expr: &Expression, cstructs: &[crate::ast::CStructDecl]) -> Option<i64> {
    match expr {
        Expression::Number(text) => Some(link_const_bits(text)),
        Expression::Boolean(value) => Some(i64::from(*value)),
        Expression::Identifier(name) if name == "NOTHING" => Some(0),
        Expression::Unary {
            operator, operand, ..
        } if operator == "SIZEOF" => {
            let Expression::Identifier(name) = operand.as_ref() else {
                return None;
            };
            let decl = cstructs.iter().find(|c| c.name == *name)?;
            let fields: Vec<(String, String)> = decl
                .fields
                .iter()
                .map(|f| (f.name.clone(), f.ctype.clone()))
                .collect();
            crate::ir::compute_c_layout(&fields, "")
                .ok()
                .map(|l| l.size as i64)
        }
        Expression::Unary {
            operator, operand, ..
        } if operator == "-" => eval_link_const_opt(operand, cstructs).map(i64::wrapping_neg),
        Expression::Unary {
            operator, operand, ..
        } if operator == "+" => eval_link_const_opt(operand, cstructs),
        _ => None,
    }
}

/// The pin's immediate, or `0` for a form that cannot be folded.
///
/// The `0` is NOT a silent default: `syntaxcheck` rejects an unfoldable pin
/// (`NATIVE_CONST_UNKNOWN_SLOT`), so the build fails and the lowered value is
/// never reached. It runs *after* lowering in this pipeline, which is why this
/// must return something rather than assert — the diagnostic still wins, but
/// lowering sees the bad pin first.
fn eval_link_const(expr: &Expression, cstructs: &[crate::ast::CStructDecl]) -> i64 {
    eval_link_const_opt(expr, cstructs).unwrap_or(0)
}

/// The 64-bit pattern a native-`LINK` integer literal denotes.
///
/// A radix/separator/exponent literal is canonicalized to decimal first, so
/// `0x10`/`1e3` resolve to their real integer rather than parsing to `0`
/// (plan-28). C flag and mask constants are conventionally *unsigned*, so a
/// value with bit 63 set (`0xFFFFFFFFFFFFFFFF`) exceeds `i64::MAX` and a signed
/// parse alone would default it to `0` — a NULL where the mask belongs, with no
/// diagnostic. The ABI cares about the bits, not the sign, so fall back to `u64`
/// and keep the exact pattern.
fn link_const_bits(text: &str) -> i64 {
    let decimal = numeric::expand_scientific_notation(&numeric::classify_literal(text).0);
    decimal
        .parse::<i64>()
        .or_else(|_| decimal.parse::<u64>().map(|bits| bits as i64))
        .unwrap_or(0)
}

/// Lower one `BIND IN` field binding (plan-50-E).
///
/// A value is either a wrapper parameter (marshaled from its incoming register)
/// or an integer literal. Anything else lowers to neither, and the checkers
/// reject it (`NATIVE_BIND_IN_INVALID`) — lowering cannot emit diagnostics, so an
/// unrecognized form must be *representable as invalid* rather than silently
/// folded to 0, which is the `eval_link_const` mistake.
fn lower_bind_in_field(
    field: &crate::ast::BindInField,
    params: &[Param],
) -> crate::ir::IrBindInField {
    let (param, literal) = match &field.value {
        Expression::Identifier(name) if params.iter().any(|p| p.name == *name) => {
            (Some(name.clone()), None)
        }
        Expression::Number(text) => (None, Some(link_const_bits(text))),
        Expression::Boolean(value) => (None, Some(i64::from(*value))),
        Expression::Unary {
            operator, operand, ..
        } if operator == "-" => match operand.as_ref() {
            Expression::Number(text) => (None, Some(link_const_bits(text).wrapping_neg())),
            _ => (None, None),
        },
        _ => (None, None),
    };
    crate::ir::IrBindInField {
        name: field.name.clone(),
        param,
        literal,
    }
}

/// Lower a `SUCCESS_ON`/`RESULT` expression to [`IrLinkExpr`], resolving each
/// identifier to the ABI slot (or ABI return) it names (plan-50-I).
///
/// An identifier that names no slot lowers to `Var(name)` unchanged and is
/// rejected by the checkers (`NATIVE_ABI_UNBOUND_SLOT`) — lowering cannot emit
/// diagnostics, so the name is carried through for them to catch. Before
/// plan-50-I every identifier collapsed onto one nameless variable meaning "the
/// native return", so `SUCCESS_ON typo = 0` silently meant `status = 0`.
///
/// `NOTHING` is matched first: it is a literal, not a slot, and a binding could
/// otherwise declare a slot named `NOTHING` and change what it means.
fn lower_link_expr(expr: &Expression) -> IrLinkExpr {
    match expr {
        Expression::Number(text) => IrLinkExpr::Int(link_const_bits(text)),
        Expression::Boolean(value) => IrLinkExpr::Int(i64::from(*value)),
        Expression::Identifier(name) if name == "NOTHING" => IrLinkExpr::Int(0),
        Expression::Identifier(name) => IrLinkExpr::Var(name.clone()),
        Expression::Unary {
            operator, operand, ..
        } if operator == "-" => match lower_link_expr(operand) {
            IrLinkExpr::Int(value) => IrLinkExpr::Int(value.wrapping_neg()),
            other => other,
        },
        Expression::Unary {
            operator, operand, ..
        } if operator.eq_ignore_ascii_case("NOT") => {
            IrLinkExpr::Not(Box::new(lower_link_expr(operand)))
        }
        Expression::Binary {
            left,
            operator,
            right,
            ..
        } => {
            let lhs = Box::new(lower_link_expr(left));
            let rhs = Box::new(lower_link_expr(right));
            match operator.to_ascii_uppercase().as_str() {
                "AND" => IrLinkExpr::And(lhs, rhs),
                "OR" => IrLinkExpr::Or(lhs, rhs),
                "=" | "<>" | "<" | ">" | "<=" | ">=" => IrLinkExpr::Compare {
                    op: operator.clone(),
                    lhs,
                    rhs,
                },
                // plan-58-B: integer arithmetic for `BUFFER … SIZE` / `LENGTH`.
                // Before these arms `SIZE frames * channels` fell into the `_`
                // below and lowered to the literal 0 — a silently zero-capacity
                // buffer, which is exactly the class of failure this whole
                // sub-plan exists to prevent.
                "*" => IrLinkExpr::Mul(lhs, rhs),
                "+" => IrLinkExpr::Add(lhs, rhs),
                "-" => IrLinkExpr::Sub(lhs, rhs),
                _ => IrLinkExpr::Int(0),
            }
        }
        _ => IrLinkExpr::Int(0),
    }
}

/// Collect native `LINK` resource declarations from the AST for package
/// metadata. `close_may_fail` is derived from whether the close wrapper has a
/// `SUCCESS_ON` gate (plan-link-update.md §9/§10).
pub(super) fn native_resources(ast: &AstProject) -> Vec<IrNativeResource> {
    let mut close_may_fail: HashMap<String, bool> = HashMap::new();
    for file in &ast.files {
        for item in &file.items {
            if let Item::Link(link) = item {
                for function in &link.functions {
                    close_may_fail.insert(
                        format!("{}.{}", link.alias, function.name),
                        function.success_on.is_some(),
                    );
                }
            }
        }
    }
    // An exported `FUNC alias AS pkg::close` is the importer-facing close op
    // (plan-link-update.md §5a). Map each re-exported close target to the bare
    // alias name; importers call `binding.alias`, so the serialized close name is
    // this bare alias (qualified on import), not the package-internal `link.func`.
    let mut export_alias_for_target: HashMap<String, String> = HashMap::new();
    for file in &ast.files {
        for item in &file.items {
            if let Item::FuncAlias(alias) = item {
                if matches!(alias.visibility, crate::ast::Visibility::Export) {
                    export_alias_for_target
                        .entry(alias.target.clone())
                        .or_insert_with(|| alias.name.clone());
                }
            }
        }
    }
    let mut resources = Vec::new();
    for file in &ast.files {
        for item in &file.items {
            if let Item::Resource(resource) = item {
                let close_function = export_alias_for_target
                    .get(&resource.close_fn)
                    .cloned()
                    .unwrap_or_else(|| resource.close_fn.clone());
                resources.push(IrNativeResource {
                    name: resource.name.clone(),
                    visibility: match resource.visibility {
                        crate::ast::Visibility::Export => "export",
                        crate::ast::Visibility::Public => "public",
                        crate::ast::Visibility::Private => "private",
                    }
                    .to_string(),
                    close_function,
                    sendable: resource.thread_sendable,
                    close_may_fail: close_may_fail
                        .get(&resource.close_fn)
                        .copied()
                        .unwrap_or(false),
                });
            }
        }
    }
    resources
}
