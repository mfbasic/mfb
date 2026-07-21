use super::*;

fn doc_prose(desc: &[crate::ast::DocProse]) -> Vec<(u8, String)> {
    desc.iter()
        .map(|prose| (prose.kind.code(), prose.text.clone()))
        .collect()
}

/// Collect the documentation surface from a project's `DOC` blocks. Only exported
/// declarations are recorded (plan-09-doc.md §3): a non-exported declaration is
/// documented in source but never persisted into the compiled package. Runs after
/// `DOC` validation, so every block here is well-formed.
pub(crate) fn collect_project_docs(ast: &crate::ast::AstProject) -> ProjectDocs {
    use crate::ast::{DocHeaderKind, Function, FunctionKind, Item, TypeDeclKind, Visibility};

    let mut funcs: HashMap<&str, Vec<&Function>> = HashMap::new();
    let mut types: HashMap<&str, (TypeDeclKind, Visibility, String)> = HashMap::new();
    for file in &ast.files {
        for item in &file.items {
            match item {
                Item::Function(function) => {
                    funcs
                        .entry(function.name.as_str())
                        .or_default()
                        .push(function);
                }
                Item::Type(type_decl) => {
                    types.entry(type_decl.name.as_str()).or_insert((
                        type_decl.kind,
                        type_decl.visibility,
                        type_decl.signature_line(),
                    ));
                }
                _ => {}
            }
        }
    }

    // Pick the overload a callable DOC block documents: the one matching the
    // header's parameter types, or the first matching-kind overload otherwise.
    let overload_for = |doc: &crate::ast::DocBlock, want_sub: bool| -> Option<&Function> {
        let list = funcs.get(doc.header_name.as_str())?;
        let matching = list
            .iter()
            .copied()
            .filter(|f| (f.kind == FunctionKind::Sub) == want_sub);
        match &doc.header_params {
            Some(wanted) => matching
                .clone()
                .find(|f| function_param_types(f) == normalize_types(wanted)),
            None => matching.clone().next(),
        }
    };

    let make_decl = |doc: &crate::ast::DocBlock, kind: IrDocKind, signature: String| IrDocDecl {
        kind,
        name: doc.header_name.clone(),
        signature,
        group: doc
            .groups
            .first()
            .map(|(name, _)| name.clone())
            .unwrap_or_default(),
        desc: doc_prose(&doc.desc),
        args: doc
            .args
            .iter()
            .map(|arg| (arg.name.clone(), arg.desc.clone()))
            .collect(),
        props: doc
            .props
            .iter()
            .map(|prop| (prop.name.clone(), prop.desc.clone()))
            .collect(),
        ret: doc
            .rets
            .first()
            .map(|(text, _)| text.clone())
            .unwrap_or_default(),
        errors: doc
            .errors
            .iter()
            .map(|error| (error.code.clone(), error.desc.clone()))
            .collect(),
        example: doc
            .examples
            .first()
            .map(|(text, _)| text.clone())
            .unwrap_or_default(),
        internal: doc
            .attrs
            .iter()
            .any(|attr| attr.eq_ignore_ascii_case("INTERNAL")),
        deprecated: doc.deprecated.first().map(|(message, _)| message.clone()),
    };

    let mut package = None;
    let mut decls = Vec::new();
    for file in &ast.files {
        for item in &file.items {
            let Item::Doc(doc) = item else {
                continue;
            };
            match doc.header_kind {
                DocHeaderKind::Package => {
                    if package.is_none() {
                        package = Some(IrPackageDoc {
                            name: ast.name.clone(),
                            desc: doc_prose(&doc.desc),
                            deprecated: doc.deprecated.first().map(|(message, _)| message.clone()),
                        });
                    }
                }
                DocHeaderKind::Func | DocHeaderKind::Sub => {
                    let want_sub = doc.header_kind == DocHeaderKind::Sub;
                    // Only exported overloads are persisted (plan-09-doc.md §3).
                    let Some(function) = overload_for(doc, want_sub) else {
                        continue;
                    };
                    if function.visibility != Visibility::Export {
                        continue;
                    }
                    let kind = if want_sub {
                        IrDocKind::Sub
                    } else {
                        IrDocKind::Func
                    };
                    decls.push(make_decl(doc, kind, function.signature_line()));
                }
                DocHeaderKind::Type | DocHeaderKind::Union | DocHeaderKind::Enum => {
                    let Some((_, vis, signature)) = types.get(doc.header_name.as_str()) else {
                        continue;
                    };
                    if *vis != Visibility::Export {
                        continue;
                    }
                    let kind = match doc.header_kind {
                        DocHeaderKind::Type => IrDocKind::Type,
                        DocHeaderKind::Union => IrDocKind::Union,
                        _ => IrDocKind::Enum,
                    };
                    decls.push(make_decl(doc, kind, signature.clone()));
                }
            }
        }
    }

    ProjectDocs { package, decls }
}

fn function_param_types(function: &crate::ast::Function) -> Vec<String> {
    function
        .params
        .iter()
        .map(|param| crate::ast::normalize_ws(param.type_name.as_deref().unwrap_or("")))
        .collect()
}

fn normalize_types(types: &[String]) -> Vec<String> {
    types.iter().map(|t| crate::ast::normalize_ws(t)).collect()
}

struct LowerContext<'a> {
    function_returns: &'a HashMap<String, String>,
    function_types: &'a HashMap<String, String>,
    function_params: &'a HashMap<String, Vec<CallParam>>,
    binding_types: HashMap<String, String>,
    bindings: Vec<IrBinding>,
    type_index: &'a TypeIndex,
    current_imports: HashMap<String, String>,
    /// Project-relative path of the source file currently being lowered, used to
    /// populate `IrFunction::file` and `ErrorLoc.filename` for generated errors.
    current_file: String,
    lambdas: Vec<IrFunction>,
    next_lambda_id: usize,
    next_temp_id: usize,
    /// Declared return type of the function currently being lowered, used to
    /// implicitly wrap a `RETURN`ed member constructor into its union (so the
    /// wrap is explicit in the IR rather than re-derived during codegen).
    current_return_type: Option<String>,
    /// Stack of inline-`TRAP` recover destinations (innermost last). Each entry
    /// is the local slot a `RECOVER` value should be stored into and its type,
    /// or `None` when the trapped value is discarded (bare-statement form).
    recover_targets: Vec<RecoverTarget>,
    /// Names of `MUT` local bindings in scope. A lambda in a non-escaping
    /// callback position captures these by slot-borrow rather than by value.
    /// Not scope-precise — only ever consulted
    /// for capture classification, where a stale non-`MUT` entry is impossible
    /// (only `MUT` binds are inserted) and a borrow is memory-safe regardless.
    mutable_locals: HashSet<String>,
    /// Set true only while lowering the argument in a compiler-known
    /// non-escaping callback position (e.g. `forEach`'s action). The lambda
    /// lowering consumes it to license `MUT` slot-borrow captures.
    nonescaping_callback: bool,
    /// Source location of the statement (or match case / declaration) currently
    /// being lowered. Stamped onto every `IrOp` so relocated diagnostics report
    /// at the same line the AST checker did (plan-20-A). Column is always 1,
    /// matching `show_diagnostic`'s statement-level reporting.
    current_loc: IrSourceLoc,
}

pub fn lower_project_with_external_functions(
    ast: &AstProject,
    entry: Option<EntryPoint>,
    external_function_types: &HashMap<String, String>,
    external_function_params: &HashMap<String, Vec<ExternalFunctionParam>>,
) -> IrProject {
    let augmented =
        builtins::json::augmented_project(ast).expect("built-in json package source must parse");
    let augmented = builtins::csv::augmented_project(&augmented)
        .expect("built-in csv package source must parse");
    let augmented = builtins::regex::augmented_project(&augmented)
        .expect("built-in regex package source must parse");
    let augmented = builtins::datetime::augmented_project(&augmented)
        .expect("built-in datetime package source must parse");
    let augmented = builtins::money::augmented_project(&augmented)
        .expect("built-in money package source must parse");
    // `vector` imports only intrinsic `math` (plan-06-vector.md §5).
    let augmented = builtins::vector::augmented_project(&augmented)
        .expect("built-in vector package source must parse");
    // `http` before `net`: `http_package.mfb` imports `net` (plan-03-http.md Phase 4).
    let augmented = builtins::http::augmented_project(&augmented)
        .expect("built-in http package source must parse");
    let augmented = builtins::net::augmented_project(&augmented)
        .expect("built-in net package source must parse");
    let augmented = builtins::audio::augmented_project(&augmented)
        .expect("built-in audio package source must parse");
    // `crypto` before `encoding`: `crypto_package.mfb` imports `encoding`
    // (mirrors `http` before `net`; plan-04-crypto.md Part C).
    let augmented = builtins::crypto::augmented_project(&augmented)
        .expect("built-in crypto package source must parse");
    // `strings` before `encoding`: `strings_package.mfb` imports `encoding`
    // (plan-41-D scalar seam).
    let augmented = builtins::strings::augmented_project(&augmented)
        .expect("built-in strings package source must parse");
    let augmented = builtins::encoding::augmented_project(&augmented)
        .expect("built-in encoding package source must parse");
    let ast = &augmented;
    let mut types = Vec::new();
    let mut functions = Vec::new();
    let mut function_returns = function_returns(ast);
    let mut function_types = function_types(ast);
    let mut function_params = function_params(ast);
    let binding_types = declared_binding_types(ast);
    function_types.extend(external_function_types.clone());
    for (name, params) in external_function_params {
        function_params.insert(
            name.clone(),
            params
                .iter()
                .map(|param| CallParam {
                    name: param.name.clone(),
                    type_: param.type_.clone(),
                    default: None,
                })
                .collect(),
        );
    }
    for (name, type_) in external_function_types {
        if let Some(return_type) = function_return_from_type(type_) {
            function_returns.insert(name.clone(), return_type);
        }
    }
    let type_index = TypeIndex::new(ast);
    let mut context = LowerContext {
        function_returns: &function_returns,
        function_types: &function_types,
        function_params: &function_params,
        binding_types,
        type_index: &type_index,
        current_imports: HashMap::new(),
        current_file: String::new(),
        bindings: Vec::new(),
        lambdas: Vec::new(),
        next_lambda_id: 0,
        next_temp_id: 0,
        current_return_type: None,
        recover_targets: Vec::new(),
        mutable_locals: HashSet::new(),
        nonescaping_callback: false,
        current_loc: IrSourceLoc::default(),
    };
    infer_binding_types(ast, &mut context);
    let bindings = lower_bindings(ast, &mut context);
    context.bindings = bindings.clone();

    for file in &ast.files {
        context.current_imports = file.import_bindings();
        context.current_file = file.path.clone();
        for item in &file.items {
            match item {
                Item::Binding(_) => {}
                Item::Function(function) => functions.push(lower_function(function, &mut context)),
                Item::Type(type_decl) => {
                    types.push(lower_type(type_decl, &type_index, &context.current_file))
                }
                // Native LINK resource declarations and re-export aliases carry no
                // executable body. The LINK block's native functions are surfaced
                // to package metadata separately (plan-link-update.md §10); they
                // are not lowered to ordinary IR functions here.
                Item::Resource(_) | Item::FuncAlias(_) | Item::Link(_) => {}
                // DOC blocks carry no executable body; documentation is collected
                // separately into the project's doc table.
                Item::Doc(_) => {}
                // TESTING blocks are lowered away before IR lowering (plan-18-A §3).
                Item::Testing(_) => {}
            }
        }
    }
    functions.extend(context.lambdas);

    IrProject {
        name: ast.name.clone(),
        entry,
        bindings,
        types,
        functions,
        native_resources: native_resources(ast),
        link_functions: link_functions(ast),
        link_cstructs: link_cstructs(ast),
        link_aliases: link_aliases(ast),
        docs: collect_project_docs(ast),
        // Assembled from the manifest by the build path (plan-46-B §4.3), which
        // is where project.json is read; the AST carries no manifest data.
        native_libraries: crate::binary_repr::NativeLibraryTable::default(),
    }
}

/// Collect `CSTRUCT` C-layout declarations from every `LINK` block (plan-50-B).
///
/// Field order is preserved verbatim: it is the layout input, so reordering here
/// would silently change every offset.
fn link_cstructs(ast: &AstProject) -> Vec<crate::ir::IrCStruct> {
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
fn link_functions(ast: &AstProject) -> Vec<IrLinkFunction> {
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
fn link_aliases(ast: &AstProject) -> Vec<(String, String)> {
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
fn native_resources(ast: &AstProject) -> Vec<IrNativeResource> {
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

fn lower_type(type_decl: &TypeDecl, type_index: &TypeIndex, file: &str) -> IrType {
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
        includes: type_decl.includes.clone(),
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

fn lower_binding(
    binding: &crate::ast::TopLevelBinding,
    context: &mut LowerContext<'_>,
) -> IrBinding {
    let loc = IrSourceLoc {
        line: binding.line as u32,
        column: 1,
    };
    context.current_loc = loc;
    let locals = context.binding_types.clone();
    let type_ = binding.type_name.clone().unwrap_or_else(|| {
        binding
            .value
            .as_ref()
            .and_then(|value| expression_type(value, &locals, context))
            .unwrap_or_else(|| "Unknown".to_string())
    });
    IrBinding {
        name: binding.name.clone(),
        visibility: visibility_name(binding.visibility).to_string(),
        mutable: binding.mutable,
        type_: type_.clone(),
        value: binding
            .value
            .as_ref()
            .map(|value| lower_expression_with_expected(value, Some(&type_), &locals, context)),
        loc,
        file: context.current_file.clone(),
        explicit_type: binding.type_name.is_some(),
    }
}

fn lower_bindings(ast: &AstProject, context: &mut LowerContext<'_>) -> Vec<IrBinding> {
    let mut lowered = Vec::new();
    for file in &ast.files {
        context.current_imports = file.import_bindings();
        context.current_file = file.path.clone();
        for item in &file.items {
            if let Item::Binding(binding) = item {
                lowered.push(lower_binding(binding, context));
            }
        }
    }
    lowered
}

fn lower_field(field: &TypeField) -> IrField {
    IrField {
        visibility: field.visibility.map(visibility_name).map(str::to_string),
        name: field.name.clone(),
        type_: field.type_name.clone(),
        loc: IrSourceLoc {
            line: field.line as u32,
            column: 1,
        },
    }
}

fn lower_variant(variant: &UnionVariant, type_index: &TypeIndex) -> IrVariant {
    IrVariant {
        name: variant.name.clone(),
        fields: type_index
            .records
            .get(&variant.name)
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
fn function_return_type(function: &Function) -> String {
    match function.kind {
        FunctionKind::Func => {
            let returns = function
                .return_type
                .clone()
                .unwrap_or_else(|| "Unknown".to_string());
            match (&function.return_state_type, function.return_resource) {
                (Some(state), true) => format!("{returns} STATE {state}"),
                _ => returns,
            }
        }
        FunctionKind::Sub => "Nothing".to_string(),
    }
}

fn lower_function(function: &Function, context: &mut LowerContext<'_>) -> IrFunction {
    let kind = match function.kind {
        FunctionKind::Func => "func",
        FunctionKind::Sub => "sub",
    };
    let returns = function_return_type(function);
    let mut locals = HashMap::new();
    for param in &function.params {
        let type_ = param
            .type_name
            .clone()
            .unwrap_or_else(|| "Unknown".to_string());
        // Carry a `RES` parameter's `STATE T` in the local type string so
        // `s.state` resolves inside the callee, matching `lower_param`.
        let type_ = match &param.state_type {
            Some(state) => format!("{type_} STATE {state}"),
            None => type_,
        };
        locals.insert(param.name.clone(), type_);
    }
    let previous_return_type = context.current_return_type.take();
    context.current_return_type = Some(returns.clone());
    let body = lower_function_body(function, &locals, context);
    context.current_return_type = previous_return_type;

    IrFunction {
        name: function.name.clone(),
        visibility: visibility_name(function.visibility).to_string(),
        kind: kind.to_string(),
        isolated: function.isolated,
        params: function
            .params
            .iter()
            .map(|param| lower_param(param, &locals, context))
            .collect(),
        returns,
        body,
        file: context.current_file.clone(),
        loc: IrSourceLoc {
            line: function.line as u32,
            column: 1,
        },
        resource_owners: crate::escape::analyze_function(function).owners().clone(),
    }
}

fn lower_function_body(
    function: &Function,
    locals: &HashMap<String, String>,
    context: &mut LowerContext<'_>,
) -> Vec<IrOp> {
    let mut body = lower_statement_block(&function.body, locals, context, None);
    if let Some(trap) = &function.trap {
        let mut trap_locals = locals.clone();
        trap_locals.insert(trap.name.clone(), "Error".to_string());
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
    param: &Param,
    locals: &HashMap<String, String>,
    context: &mut LowerContext<'_>,
) -> IrParam {
    let type_ = param
        .type_name
        .clone()
        .unwrap_or_else(|| "Unknown".to_string());
    // A `RES` parameter's `STATE T` rides in the type string so the callee can
    // address the borrowed resource's shared state payload.
    let type_ = match &param.state_type {
        Some(state) => format!("{type_} STATE {state}"),
        None => type_,
    };
    IrParam {
        name: param.name.clone(),
        type_,
        default: param
            .default
            .as_ref()
            .map(|value| lower_expression(value, locals, context)),
        loc: IrSourceLoc {
            line: param.line as u32,
            column: 1,
        },
    }
}

/// The source line of a statement, as `IrSourceLoc` (column 1 — diagnostics
/// report statement-level positions).
fn statement_loc(statement: &Statement) -> IrSourceLoc {
    let line = match statement {
        Statement::Let { line, .. }
        | Statement::Return { line, .. }
        | Statement::Exit { line, .. }
        | Statement::Continue { line, .. }
        | Statement::Fail { line, .. }
        | Statement::Propagate { line }
        | Statement::Recover { line, .. }
        | Statement::Assign { line, .. }
        | Statement::StateAssign { line, .. }
        | Statement::Expression { line, .. }
        | Statement::If { line, .. }
        | Statement::Match { line, .. }
        | Statement::For { line, .. }
        | Statement::ForEach { line, .. }
        | Statement::While { line, .. }
        | Statement::DoUntil { line, .. } => *line,
    };
    IrSourceLoc {
        line: line as u32,
        column: 1,
    }
}

#[derive(Clone)]
struct RecoverTarget {
    slot: Option<String>,
    type_: String,
}

#[derive(Clone)]
struct CallParam {
    name: String,
    type_: String,
    default: Option<Expression>,
}

#[derive(Clone)]
struct CapturedLocal {
    name: String,
    type_: String,
}

fn lower_statement(
    statement: &Statement,
    locals: &mut HashMap<String, String>,
    context: &mut LowerContext<'_>,
    trap_name: Option<&str>,
) -> Vec<IrOp> {
    // The statement's own span: captured locally (nested blocks re-set
    // `context.current_loc`, so the context copy cannot be reread after
    // lowering a child block) and published for expression-lowering helpers
    // that synthesize ops mid-expression.
    let loc = statement_loc(statement);
    context.current_loc = loc;
    match statement {
        Statement::Let {
            mutable,
            name,
            type_name,
            value,
            state_type,
            ..
        } => {
            if let Some(Expression::Trapped {
                expression,
                binding,
                handler,
                ..
            }) = value
            {
                let success_type = type_name
                    .clone()
                    .or_else(|| expression_type(expression, locals, context))
                    .unwrap_or_else(|| "Unknown".to_string());
                return lower_inline_trap(
                    expression,
                    binding,
                    handler,
                    InlineTrapTarget::Bind {
                        mutable: *mutable,
                        name: name.clone(),
                        type_: success_type,
                        explicit_type: type_name.is_some(),
                    },
                    locals,
                    context,
                );
            }
            let lowered_type = type_name.clone().unwrap_or_else(|| {
                value
                    .as_ref()
                    .and_then(|value| expression_type(value, locals, context))
                    .unwrap_or_else(|| "Unknown".to_string())
            });
            let lowered_value = value.as_ref().map(|value| {
                let base =
                    lower_expression_with_expected(value, Some(&lowered_type), locals, context);
                // Wrap a resource (or data) variant value into its union when the
                // binding is union-typed, so a `RES s AS Stream = <a File>` carries
                // the variant tag for tag-dispatched drop.
                wrap_union_value(base, value, Some(&lowered_type), locals, context)
            });
            // A `RES` binding's `STATE T` rides in the lowered type string
            // (`File STATE T`) so codegen can default-initialize and address the
            // state payload; the bare resource name is recovered for recognition.
            let lowered_type = match state_type {
                Some(state) => format!("{lowered_type} STATE {state}"),
                None => lowered_type,
            };
            locals.insert(name.clone(), lowered_type.clone());
            // Track `MUT` bindings so a non-escaping callback can borrow them by
            // slot rather than copy them by value.
            if *mutable {
                context.mutable_locals.insert(name.clone());
            }
            vec![IrOp::Bind {
                mutable: *mutable,
                name: name.clone(),
                type_: lowered_type,
                value: lowered_value,
                explicit_type: type_name.is_some(),
                loc,
            }]
        }
        Statement::Return { value, .. } => vec![IrOp::Return {
            value: value.as_ref().map(|value| {
                // Coerce a bare numeric literal to the declared return type,
                // exactly as `LET`/constructor-arg lowering does — otherwise an
                // unsuffixed literal returned from a `Fixed`/`Money`/`Float`
                // function is classified as `Integer` and its raw bits are
                // reinterpreted as the destination type (bug-156).
                let expected = context.current_return_type.clone();
                let base =
                    lower_expression_with_expected(value, expected.as_deref(), locals, context);
                // Implicitly wrap a returned member constructor into the
                // function's declared union return type, so the wrap is explicit
                // in the IR (and faithfully serialized into Binary Representation) rather
                // than re-derived during native codegen.
                wrap_union_value(base, value, expected.as_deref(), locals, context)
            }),
            loc,
        }],
        Statement::Exit { target, code, .. } => match target {
            ExitTarget::For => vec![IrOp::ExitLoop {
                kind: LoopKind::For,
                loc,
            }],
            ExitTarget::Do => vec![IrOp::ExitLoop {
                kind: LoopKind::Do,
                loc,
            }],
            ExitTarget::While => vec![IrOp::ExitLoop {
                kind: LoopKind::While,
                loc,
            }],
            ExitTarget::Sub => vec![IrOp::Return { value: None, loc }],
            ExitTarget::Func => Vec::new(),
            ExitTarget::Program => vec![IrOp::ExitProgram {
                code: lower_expression(
                    code.as_ref()
                        .expect("parser requires EXIT PROGRAM to include a code expression"),
                    locals,
                    context,
                ),
                loc,
            }],
        },
        Statement::Continue { kind, .. } => vec![IrOp::ContinueLoop { kind: *kind, loc }],
        Statement::Fail { error, .. } => vec![IrOp::Fail {
            error: lower_expression(error, locals, context),
            loc,
        }],
        Statement::Propagate { .. } => vec![IrOp::Fail {
            // Typecheck rejects PROPAGATE outside a trap body; total lowering
            // (plan-20-D) stamps a sentinel error local when the guard is
            // absent so it never panics on ill-typed input.
            error: IrValue::Local(trap_name.unwrap_or("$error").to_string()),
            loc,
        }],
        Statement::Recover { value, .. } => {
            // Typecheck rejects RECOVER outside an inline-TRAP handler; total
            // lowering falls back to a discard target rather than panicking.
            let target = context
                .recover_targets
                .last()
                .cloned()
                .unwrap_or_else(|| RecoverTarget {
                    slot: None,
                    type_: "Unknown".to_string(),
                });
            match (target.slot, value) {
                (Some(slot), Some(value)) => {
                    let lowered =
                        lower_expression_with_expected(value, Some(&target.type_), locals, context);
                    vec![IrOp::Assign {
                        name: slot,
                        value: lowered,
                        loc,
                    }]
                }
                (None, Some(value)) => vec![IrOp::Eval {
                    value: lower_expression_with_expected(
                        value,
                        Some(&target.type_),
                        locals,
                        context,
                    ),
                    loc,
                }],
                (_, None) => Vec::new(),
            }
        }
        Statement::Assign { name, value, .. } => {
            if let Expression::Trapped {
                expression,
                binding,
                handler,
                ..
            } = value
            {
                return lower_inline_trap(
                    expression,
                    binding,
                    handler,
                    InlineTrapTarget::Assign { name: name.clone() },
                    locals,
                    context,
                );
            }
            let expected = locals
                .get(name)
                .or_else(|| context.binding_types.get(name))
                .cloned();
            let lowered =
                lower_expression_with_expected(value, expected.as_deref(), locals, context);
            if locals.contains_key(name) {
                vec![IrOp::Assign {
                    name: name.clone(),
                    value: lowered,
                    loc,
                }]
            } else {
                vec![IrOp::AssignGlobal {
                    name: name.clone(),
                    value: lowered,
                    loc,
                }]
            }
        }
        Statement::StateAssign {
            resource, value, ..
        } => {
            let resource_type = locals
                .get(resource)
                .or_else(|| context.binding_types.get(resource))
                .cloned();
            let state_type = resource_type
                .as_deref()
                .and_then(crate::builtins::resource::state_type_name)
                .map(str::to_string);
            let lowered =
                lower_expression_with_expected(value, state_type.as_deref(), locals, context);
            vec![IrOp::StateAssign {
                resource: resource.clone(),
                value: lowered,
                loc,
            }]
        }
        Statement::Expression { expression, .. } => {
            // Assertion builtins (plan-18-B) desugar to ordinary statements —
            // comparisons + FAIL, or a trap-guarded evaluation — which are then
            // lowered through the normal path. Doing it here (post-typecheck)
            // sidesteps the source-level RECOVER-typing constraint on a
            // value-producing trapped expression.
            if let Expression::Call {
                callee,
                arguments,
                line: call_line,
                ..
            } = expression
            {
                if crate::builtins::testing::is_expect_call(callee) {
                    let uid = context.next_temp_id;
                    context.next_temp_id += 1;
                    let expanded =
                        crate::testing::expand_expect(callee, arguments, uid, *call_line);
                    return lower_statement_block(&expanded, locals, context, trap_name);
                }
            }
            if let Expression::Trapped {
                expression: inner,
                binding,
                handler,
                ..
            } = expression
            {
                return lower_inline_trap(
                    inner,
                    binding,
                    handler,
                    InlineTrapTarget::Discard,
                    locals,
                    context,
                );
            }
            vec![IrOp::Eval {
                value: lower_expression(expression, locals, context),
                loc,
            }]
        }
        Statement::If {
            condition,
            then_body,
            else_body,
            ..
        } => vec![IrOp::If {
            condition: lower_expression(condition, locals, context),
            then_body: lower_statement_block(then_body, locals, context, trap_name),
            else_body: lower_statement_block(else_body, locals, context, trap_name),
            loc,
        }],
        Statement::Match {
            expression, cases, ..
        } => {
            let matched_type = match_expression_type(expression, locals, context)
                .unwrap_or_else(|| "Unknown".to_string());
            let matched_name = make_temp_local_name(context, "match");
            let mut ops = vec![IrOp::Bind {
                mutable: false,
                name: matched_name.clone(),
                type_: matched_type.clone(),
                value: Some(lower_match_expression(
                    expression,
                    &matched_type,
                    locals,
                    context,
                )),
                loc,
                explicit_type: false,
            }];
            let mut match_locals = locals.clone();
            match_locals.insert(matched_name.clone(), matched_type);
            // coverage:off -- a `Result OF ...`-typed MATCH scrutinee is rejected
            // before lowering (TYPE_RESULT_NOT_MATCHABLE; see the
            // `result-not-matchable-invalid` fixture), so this Result-flag branch
            // is unreachable from valid source; kept for plan-20 total lowering.
            let match_value = if match_locals[&matched_name].starts_with("Result OF ") {
                let match_flag_name = make_temp_local_name(context, "match_ok");
                ops.push(IrOp::Bind {
                    mutable: false,
                    name: match_flag_name.clone(),
                    type_: "Boolean".to_string(),
                    value: Some(IrValue::ResultIsOk {
                        value: Box::new(IrValue::Local(matched_name.clone())),
                    }),
                    loc,
                    explicit_type: false,
                });
                match_locals.insert(match_flag_name.clone(), "Boolean".to_string());
                IrValue::Local(match_flag_name)
            } else {
                IrValue::Local(matched_name.clone())
            };
            // coverage:on
            ops.push(IrOp::Match {
                value: match_value,
                cases: cases
                    .iter()
                    .map(|case| {
                        lower_match_case(case, &matched_name, &match_locals, context, trap_name)
                    })
                    .collect(),
                loc,
            });
            ops
        }
        Statement::For {
            name,
            start,
            end,
            step,
            body,
            line,
        } => {
            let start_type =
                expression_type(start, locals, context).unwrap_or_else(|| "Unknown".to_string());
            let end_type =
                expression_type(end, locals, context).unwrap_or_else(|| "Unknown".to_string());
            let step_type = step
                .as_ref()
                .and_then(|value| expression_type(value, locals, context))
                .unwrap_or_else(|| "Integer".to_string());
            let loop_type = promote_loop_numeric_type_name(&start_type, &end_type, &step_type);
            let iter_name = make_temp_local_name(context, "for_iter");
            let end_name = make_temp_local_name(context, "for_end");
            let step_name = make_temp_local_name(context, "for_step");

            let start_value =
                lower_expression_with_expected(start, Some(&loop_type), locals, context);
            let end_value = lower_expression_with_expected(end, Some(&loop_type), locals, context);
            let step_value = step
                .as_ref()
                .map(|value| {
                    lower_expression_with_expected(value, Some(&loop_type), locals, context)
                })
                .unwrap_or_else(|| numeric_constant_for_type(&loop_type, "1"));

            locals.insert(iter_name.clone(), loop_type.clone());
            locals.insert(end_name.clone(), loop_type.clone());
            locals.insert(step_name.clone(), loop_type.clone());

            let step_local = IrValue::Local(step_name.clone());
            let iter_local = IrValue::Local(iter_name.clone());
            let end_local = IrValue::Local(end_name.clone());

            let mut nested = locals.clone();
            nested.insert(name.clone(), loop_type.clone());
            let mut loop_body = vec![IrOp::Bind {
                mutable: false,
                name: name.clone(),
                type_: loop_type.clone(),
                value: Some(iter_local.clone()),
                loc,
                explicit_type: false,
            }];
            loop_body.extend(lower_statement_block(body, &nested, context, trap_name));

            vec![
                IrOp::Bind {
                    mutable: false,
                    name: end_name,
                    type_: loop_type.clone(),
                    value: Some(end_value),
                    loc,
                    explicit_type: false,
                },
                IrOp::Bind {
                    mutable: false,
                    name: step_name,
                    type_: loop_type.clone(),
                    value: Some(step_value),
                    loc,
                    explicit_type: false,
                },
                IrOp::For {
                    name: iter_name,
                    type_: loop_type.clone(),
                    start: start_value,
                    end: end_local,
                    step: step_local,
                    body: loop_body,
                    loc: IrSourceLoc {
                        line: *line as u32,
                        column: 1,
                    },
                },
            ]
        }
        Statement::ForEach {
            name,
            iterable,
            body,
            ..
        } => {
            let iterable_type =
                expression_type(iterable, locals, context).unwrap_or_else(|| "Unknown".to_string());
            let element_type =
                collection_iteration_type(&iterable_type).unwrap_or_else(|| "Unknown".to_string());
            let mut nested = locals.clone();
            nested.insert(name.clone(), element_type.clone());
            vec![IrOp::ForEach {
                name: name.clone(),
                type_: element_type,
                iterable: lower_expression(iterable, locals, context),
                body: lower_statement_block(body, &nested, context, trap_name),
                loc,
            }]
        }
        Statement::While {
            kind,
            condition,
            body,
            ..
        } => vec![IrOp::While {
            kind: *kind,
            condition: lower_expression(condition, locals, context),
            body: lower_statement_block(body, locals, context, trap_name),
            loc,
        }],
        Statement::DoUntil {
            body, condition, ..
        } => {
            let body = lower_statement_block(body, locals, context, trap_name);
            // The trailing condition belongs to this statement, not the last
            // body statement: restore the loop's own span for any ops the
            // condition lowering synthesizes.
            context.current_loc = loc;
            vec![IrOp::DoUntil {
                body,
                condition: lower_expression(condition, locals, context),
                loc,
            }]
        }
    }
}

fn lower_statement_block(
    body: &[Statement],
    locals: &HashMap<String, String>,
    context: &mut LowerContext<'_>,
    trap_name: Option<&str>,
) -> Vec<IrOp> {
    let mut nested = locals.clone();
    body.iter()
        .flat_map(|statement| lower_statement(statement, &mut nested, context, trap_name))
        .collect()
}

/// Where the recovered/`Ok` value of an inline `TRAP` is delivered.
enum InlineTrapTarget {
    /// `LET`/`MUT name = <call> TRAP(e) …`
    Bind {
        mutable: bool,
        name: String,
        type_: String,
        explicit_type: bool,
    },
    /// `name = <call> TRAP(e) …`
    Assign { name: String },
    /// `<call> TRAP(e) …` as a bare statement (value discarded).
    Discard,
}

/// Lowers an inline `TRAP` to existing IR primitives (no backend support is
/// required). The trapped call is evaluated as a raw `Result`; on `Ok` its value
/// flows to the target; on `Err` the handler runs with `e` bound. `RECOVER`
/// stores its value into a shared slot and then falls through to the delivery of
/// the target, while diverging handler paths (`RETURN`/`FAIL`/`PROPAGATE`) leave
/// as usual. The handler is normalized so that statements following a `RECOVER`
/// in a branch do not execute after recovery (see [`treeify_handler`]).
fn lower_inline_trap(
    inner: &Expression,
    binding: &str,
    handler: &[Statement],
    target: InlineTrapTarget,
    locals: &mut HashMap<String, String>,
    context: &mut LowerContext<'_>,
) -> Vec<IrOp> {
    // The inline-TRAP statement's span: the handler block below re-sets
    // `context.current_loc` per handler statement, so ops synthesized after it
    // must use this captured copy.
    let stmt_loc = context.current_loc;
    let success_type =
        expression_type(inner, locals, context).unwrap_or_else(|| "Unknown".to_string());
    let result_type = format!("Result OF {success_type}");
    let raw = lower_expression(inner, locals, context);
    let call_result = match raw {
        IrValue::Call {
            target, args, loc, ..
        } => IrValue::CallResult {
            target,
            args,
            // The fallible form's success type is the call's own result type.
            type_: success_type.clone(),
            loc,
        },
        other => other,
    };

    let res_name = make_temp_local_name(context, "trap_res");
    let mut ops = vec![IrOp::Bind {
        mutable: false,
        name: res_name.clone(),
        type_: result_type.clone(),
        value: Some(call_result),
        loc: stmt_loc,
        explicit_type: false,
    }];
    locals.insert(res_name.clone(), result_type);

    // A shared slot carries the value on both the Ok and RECOVER paths so the
    // target binding/assignment is produced exactly once after the branch.
    let slot = match &target {
        InlineTrapTarget::Bind { .. } | InlineTrapTarget::Assign { .. } => {
            let val_name = make_temp_local_name(context, "trap_val");
            ops.push(IrOp::Bind {
                mutable: true,
                name: val_name.clone(),
                type_: success_type.clone(),
                value: None,
                loc: stmt_loc,
                explicit_type: false,
            });
            locals.insert(val_name.clone(), success_type.clone());
            Some(val_name)
        }
        InlineTrapTarget::Discard => None,
    };

    let then_body = match &slot {
        Some(val_name) => vec![IrOp::Assign {
            name: val_name.clone(),
            value: IrValue::ResultValue {
                type_: success_type.clone(),
                value: Box::new(IrValue::Local(res_name.clone())),
            },
            loc: stmt_loc,
        }],
        None => Vec::new(),
    };

    let mut handler_locals = locals.clone();
    handler_locals.insert(binding.to_string(), "Error".to_string());
    let mut else_body = vec![IrOp::Bind {
        mutable: false,
        name: binding.to_string(),
        type_: "Error".to_string(),
        value: Some(IrValue::ResultError {
            value: Box::new(IrValue::Local(res_name.clone())),
        }),
        loc: stmt_loc,
        explicit_type: false,
    }];
    context.recover_targets.push(RecoverTarget {
        slot: slot.clone(),
        type_: success_type.clone(),
    });
    let normalized = treeify_handler(handler);
    else_body.extend(lower_statement_block(
        &normalized,
        &handler_locals,
        context,
        Some(binding),
    ));
    context.recover_targets.pop();

    ops.push(IrOp::If {
        condition: IrValue::ResultIsOk {
            value: Box::new(IrValue::Local(res_name.clone())),
        },
        then_body,
        else_body,
        loc: stmt_loc,
    });

    match target {
        InlineTrapTarget::Bind {
            mutable,
            name,
            type_,
            explicit_type,
        } => {
            ops.push(IrOp::Bind {
                mutable,
                name: name.clone(),
                type_: type_.clone(),
                value: Some(IrValue::Local(slot.expect("bind target has a value slot"))),
                explicit_type,
                loc: stmt_loc,
            });
            if mutable {
                context.mutable_locals.insert(name.clone());
            }
            locals.insert(name, type_);
        }
        InlineTrapTarget::Assign { name } => {
            let value = IrValue::Local(slot.expect("assign target has a value slot"));
            if locals.contains_key(&name) {
                ops.push(IrOp::Assign {
                    name,
                    value,
                    loc: stmt_loc,
                });
            } else {
                ops.push(IrOp::AssignGlobal {
                    name,
                    value,
                    loc: stmt_loc,
                });
            }
        }
        InlineTrapTarget::Discard => {}
    }

    ops
}

/// Normalizes an inline-`TRAP` handler so that a `RECOVER` (which is lowered as
/// an assignment that falls through to the post-trap continuation) never lets
/// statements that follow it in a sibling position execute. Statements after a
/// branching statement (`IF`/`MATCH`) whose branch falls through are pushed into
/// that fall-through branch, so each leaf path ends in its own terminator and
/// the structured lowering needs no jumps. Statements after a terminator are
/// unreachable and dropped.
fn treeify_handler(stmts: &[Statement]) -> Vec<Statement> {
    let Some((head, tail)) = stmts.split_first() else {
        return Vec::new();
    };

    if tail.is_empty() {
        return vec![treeify_statement(head)];
    }
    if statement_terminates(head) {
        // Anything after a terminator cannot run.
        return vec![treeify_statement(head)];
    }

    match head {
        Statement::If {
            condition,
            then_body,
            else_body,
            line,
        } => {
            let then_body = distribute_continuation(then_body, tail);
            let else_body = distribute_continuation(else_body, tail);
            vec![Statement::If {
                condition: condition.clone(),
                then_body,
                else_body,
                line: *line,
            }]
        }
        Statement::Match {
            expression,
            cases,
            line,
        } => {
            let mut new_cases: Vec<MatchCase> = cases
                .iter()
                .map(|case| MatchCase {
                    pattern: case.pattern.clone(),
                    guard: case.guard.clone(),
                    body: distribute_continuation(&case.body, tail),
                    line: case.line,
                })
                .collect();
            // An unmatched scrutinee falls through to the continuation, so make
            // that path explicit unless an ELSE arm already covers it.
            let has_else = cases
                .iter()
                .any(|case| matches!(case.pattern, MatchPattern::Else) && case.guard.is_none());
            if !has_else {
                new_cases.push(MatchCase {
                    pattern: MatchPattern::Else,
                    guard: None,
                    body: treeify_handler(tail),
                    line: *line,
                });
            }
            vec![Statement::Match {
                expression: expression.clone(),
                cases: new_cases,
                line: *line,
            }]
        }
        _ => {
            // A non-branching, non-terminating statement falls through to the
            // continuation; keep it and continue normalizing the tail.
            let mut result = vec![treeify_statement(head)];
            result.extend(treeify_handler(tail));
            result
        }
    }
}

/// Appends `continuation` to a block's fall-through paths, then normalizes it.
fn distribute_continuation(body: &[Statement], continuation: &[Statement]) -> Vec<Statement> {
    if block_terminates(body) {
        treeify_handler(body)
    } else {
        let mut combined = body.to_vec();
        combined.extend_from_slice(continuation);
        treeify_handler(&combined)
    }
}

/// Recurses into a statement's nested blocks without distributing any
/// continuation (used when there is nothing following the statement).
fn treeify_statement(statement: &Statement) -> Statement {
    match statement {
        Statement::If {
            condition,
            then_body,
            else_body,
            line,
        } => Statement::If {
            condition: condition.clone(),
            then_body: treeify_handler(then_body),
            else_body: treeify_handler(else_body),
            line: *line,
        },
        Statement::Match {
            expression,
            cases,
            line,
        } => Statement::Match {
            expression: expression.clone(),
            cases: cases
                .iter()
                .map(|case| MatchCase {
                    pattern: case.pattern.clone(),
                    guard: case.guard.clone(),
                    body: treeify_handler(&case.body),
                    line: case.line,
                })
                .collect(),
            line: *line,
        },
        Statement::While {
            kind,
            condition,
            body,
            line,
        } => Statement::While {
            kind: *kind,
            condition: condition.clone(),
            body: treeify_handler(body),
            line: *line,
        },
        Statement::DoUntil {
            body,
            condition,
            line,
        } => Statement::DoUntil {
            body: treeify_handler(body),
            condition: condition.clone(),
            line: *line,
        },
        Statement::For {
            name,
            start,
            end,
            step,
            body,
            line,
        } => Statement::For {
            name: name.clone(),
            start: start.clone(),
            end: end.clone(),
            step: step.clone(),
            body: treeify_handler(body),
            line: *line,
        },
        Statement::ForEach {
            name,
            iterable,
            body,
            line,
        } => Statement::ForEach {
            name: name.clone(),
            iterable: iterable.clone(),
            body: treeify_handler(body),
            line: *line,
        },
        other => other.clone(),
    }
}

/// Whether executing `stmts` always ends in a terminator (never reaches the end
/// of the block).
fn block_terminates(stmts: &[Statement]) -> bool {
    stmts.iter().any(statement_terminates)
}

/// Whether a statement always diverges or recovers (ends its enclosing handler
/// path). Mirrors the syntaxcheck flow analysis for the constructs an inline-trap
/// handler may contain.
fn statement_terminates(statement: &Statement) -> bool {
    match statement {
        Statement::Return { .. }
        | Statement::Exit { .. }
        | Statement::Continue { .. }
        | Statement::Fail { .. }
        | Statement::Propagate { .. }
        | Statement::Recover { .. } => true,
        Statement::If {
            then_body,
            else_body,
            ..
        } => !else_body.is_empty() && block_terminates(then_body) && block_terminates(else_body),
        Statement::Match { cases, .. } => {
            let has_else = cases
                .iter()
                .any(|case| matches!(case.pattern, MatchPattern::Else) && case.guard.is_none());
            has_else && !cases.is_empty() && cases.iter().all(|case| block_terminates(&case.body))
        }
        _ => false,
    }
}

fn collection_iteration_type(type_: &str) -> Option<String> {
    type_
        .strip_prefix("List OF ")
        // Iterating `List OF RES File` yields a borrow of each element; the loop
        // variable's type is the bare resource (`File`), not `RES File` (§15.6).
        .map(|element| element.strip_prefix("RES ").unwrap_or(element).to_string())
        .or_else(|| {
            parse_map_type(type_).map(|(key, value)| {
                let value = value.strip_prefix("RES ").unwrap_or(value.as_str());
                format!("MapEntry OF {key} TO {value}")
            })
        })
}

fn make_temp_local_name(context: &mut LowerContext<'_>, prefix: &str) -> String {
    let name = format!("${prefix}{}", context.next_temp_id);
    context.next_temp_id += 1;
    name
}

fn promote_loop_numeric_type_name(start: &str, end: &str, step: &str) -> String {
    let first = numeric_binary_result_type("+", start, end);
    numeric_binary_result_type("+", first, step).to_string()
}

fn numeric_constant_for_type(type_: &str, value: &str) -> IrValue {
    IrValue::Const {
        type_: type_.to_string(),
        value: value.to_string(),
    }
}

fn parse_map_type(type_: &str) -> Option<(String, String)> {
    let rest = type_.strip_prefix("Map OF ")?;
    let (key, value) = rest.split_once(" TO ")?;
    Some((key.to_string(), value.to_string()))
}

fn parse_map_entry_type(type_: &str) -> Option<(String, String)> {
    let rest = type_.strip_prefix("MapEntry OF ")?;
    let (key, value) = rest.split_once(" TO ")?;
    Some((key.to_string(), value.to_string()))
}

fn lower_match_case(
    case: &MatchCase,
    matched_local: &str,
    locals: &HashMap<String, String>,
    context: &mut LowerContext<'_>,
    trap_name: Option<&str>,
) -> IrMatchCase {
    // The case arm's own span (syntaxcheck reports match-arm rules at the case
    // line); captured locally since the body block re-sets the context copy.
    let loc = IrSourceLoc {
        line: case.line as u32,
        column: 1,
    };
    context.current_loc = loc;
    let matched_type = locals
        .get(matched_local)
        .cloned()
        .unwrap_or_else(|| "Unknown".to_string());
    let pattern = match &case.pattern {
        MatchPattern::Else => IrMatchPattern::Else,
        MatchPattern::Literal(expression) => {
            IrMatchPattern::Value(lower_expression(expression, locals, context))
        }
        // coverage:off -- reachable only for a `Result OF ...` scrutinee, which is
        // rejected before lowering (TYPE_RESULT_NOT_MATCHABLE); kept for plan-20
        // total lowering when the AST checker is bypassed.
        MatchPattern::Union { type_name, .. } if matched_type.starts_with("Result OF ") => {
            let matched = match type_name.as_str() {
                "Ok" => "true",
                "Error" => "false",
                _ => "false",
            };
            IrMatchPattern::Value(IrValue::Const {
                type_: "Boolean".to_string(),
                value: matched.to_string(),
            })
        }
        // coverage:on
        MatchPattern::Union { type_name, .. } => {
            IrMatchPattern::Value(IrValue::Local(type_name.clone()))
        }
        MatchPattern::OneOf(expressions) => IrMatchPattern::OneOf(
            expressions
                .iter()
                .map(|expression| lower_expression(expression, locals, context))
                .collect(),
        ),
    };
    let mut case_locals = locals.clone();
    let mut body = Vec::new();
    if let Some((binding, binding_type, value)) =
        match_case_binding(&case.pattern, matched_local, &matched_type)
    {
        case_locals.insert(binding.clone(), binding_type.clone());
        body.push(IrOp::Bind {
            mutable: false,
            name: binding,
            type_: binding_type,
            value: Some(value),
            loc,
            explicit_type: false,
        });
    }
    body.extend(lower_statement_block(
        &case.body,
        &case_locals,
        context,
        trap_name,
    ));
    // The guard belongs to the case arm, not the last body statement: restore
    // the arm's span for any ops the guard lowering synthesizes.
    context.current_loc = loc;
    IrMatchCase {
        pattern,
        guard: case
            .guard
            .as_ref()
            .map(|guard| lower_expression(guard, &case_locals, context)),
        body,
        loc,
    }
}

fn match_case_binding(
    pattern: &MatchPattern,
    matched_local: &str,
    matched_type: &str,
) -> Option<(String, String, IrValue)> {
    match pattern {
        MatchPattern::Union { type_name, binding } => {
            // coverage:off -- a `Result OF ...` scrutinee is rejected before
            // lowering (TYPE_RESULT_NOT_MATCHABLE); this Ok/Error case binding is
            // kept only for plan-20 total lowering when the checker is bypassed.
            if let Some(success) = matched_type.strip_prefix("Result OF ") {
                return match type_name.as_str() {
                    "Ok" => Some((
                        binding.clone(),
                        success.to_string(),
                        IrValue::ResultValue {
                            type_: success.to_string(),
                            value: Box::new(IrValue::Local(matched_local.to_string())),
                        },
                    )),
                    "Error" => Some((
                        binding.clone(),
                        "Error".to_string(),
                        IrValue::ResultError {
                            value: Box::new(IrValue::Local(matched_local.to_string())),
                        },
                    )),
                    _ => None,
                };
            }
            // coverage:on
            Some((
                binding.clone(),
                type_name.clone(),
                IrValue::UnionExtract {
                    type_: type_name.clone(),
                    value: Box::new(IrValue::Local(matched_local.to_string())),
                },
            ))
        }
        _ => None,
    }
}

fn lower_match_expression(
    expression: &Expression,
    matched_type: &str,
    locals: &HashMap<String, String>,
    context: &mut LowerContext<'_>,
) -> IrValue {
    // A `MATCH` scrutinee that is a call auto-unwraps like any other call site
    // (local error handling now uses an inline `TRAP`), so the scrutinee lowers
    // to its ordinary value. A `Result`-typed *value* (a local or field) keeps
    // its `Result OF …` type and is matched with `CASE Ok`/`CASE Error`.
    lower_expression_with_expected(expression, Some(matched_type), locals, context)
}

fn match_expression_type(
    expression: &Expression,
    locals: &HashMap<String, String>,
    context: &LowerContext<'_>,
) -> Option<String> {
    // Call scrutinees auto-unwrap; only a value already of `Result` type keeps
    // its `Result OF …` shape for `CASE Ok`/`CASE Error` matching.
    expression_type(expression, locals, context)
}

fn function_returns(ast: &AstProject) -> HashMap<String, String> {
    let mut returns = HashMap::new();
    // Native LINK function return types, keyed `alias.func`, so callers like
    // `sqliteLink::open(...)` get a type during IR lowering (plan-link-update.md §5b).
    let mut native_returns: HashMap<String, String> = HashMap::new();
    for file in &ast.files {
        for item in &file.items {
            match item {
                Item::Function(function) => {
                    // Carries the STATE too, so `openTagged(p).state` resolves
                    // from the call expression (plan-52-D).
                    returns.insert(function.name.clone(), function_return_type(function));
                }
                Item::Link(link) => {
                    for native in &link.functions {
                        let return_type = native
                            .return_type
                            .clone()
                            .unwrap_or_else(|| "Nothing".to_string());
                        // Carry a stateful native producer's STATE, so a wrapper
                        // that calls `snd::rawOpen(p)` sees `SoundFile STATE
                        // FileInfo` and can RETURN it as its own stateful return
                        // (plan-53-A/B). Without this the call infers bare
                        // `SoundFile` and the wrapper's RETURN mismatches.
                        let return_type = match (native.return_resource, &native.return_state_type)
                        {
                            (true, Some(state)) => format!("{return_type} STATE {state}"),
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
    for file in &ast.files {
        for item in &file.items {
            if let Item::FuncAlias(alias) = item {
                if let Some(return_type) = native_returns.get(&alias.target) {
                    returns.insert(alias.name.clone(), return_type.clone());
                }
            }
        }
    }
    returns.extend(native_returns);
    returns
}

fn function_types(ast: &AstProject) -> HashMap<String, String> {
    let mut types = HashMap::new();
    for file in &ast.files {
        for item in &file.items {
            if let Item::Function(function) = item {
                let params = function
                    .params
                    .iter()
                    .map(|param| {
                        param
                            .type_name
                            .clone()
                            .unwrap_or_else(|| "Unknown".to_string())
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                // A first-class reference's return carries the STATE for the same
                // reason a direct call's does: without it `LET g = openTagged` would
                // launder the state away — `g(p)` would type as a bare `File`, and
                // binding that to `STATE Label` would read as a legal attach while
                // the runtime adopts and re-types openTagged's Cursor (plan-52-D §3).
                let returns = function_return_type(function);
                types.insert(
                    function.name.clone(),
                    format!(
                        "{}FUNC({params}) AS {returns}",
                        if function.isolated { "ISOLATED " } else { "" }
                    ),
                );
            }
        }
    }
    // Native LINK functions, keyed `alias.func`, so first-class references to them
    // type correctly during IR lowering (plan-link-update.md §5b).
    for file in &ast.files {
        for item in &file.items {
            if let Item::Link(link) = item {
                for native in &link.functions {
                    let params = native
                        .params
                        .iter()
                        .map(|param| {
                            param
                                .type_name
                                .clone()
                                .unwrap_or_else(|| "Unknown".to_string())
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    let returns = native
                        .return_type
                        .clone()
                        .unwrap_or_else(|| "Nothing".to_string());
                    // Stateful native producer: carry its STATE in the callable
                    // type too (plan-53-A/B), matching `native_returns` above.
                    let returns = match (native.return_resource, &native.return_state_type) {
                        (true, Some(state)) => format!("{returns} STATE {state}"),
                        _ => returns,
                    };
                    types.insert(
                        format!("{}.{}", link.alias, native.name),
                        format!("FUNC({params}) AS {returns}"),
                    );
                }
            }
        }
    }
    types
}

fn function_params(ast: &AstProject) -> HashMap<String, Vec<CallParam>> {
    let mut params = HashMap::new();
    for file in &ast.files {
        for item in &file.items {
            if let Item::Function(function) = item {
                params.insert(
                    function.name.clone(),
                    function
                        .params
                        .iter()
                        .map(|param| CallParam {
                            name: param.name.clone(),
                            type_: param
                                .type_name
                                .clone()
                                .unwrap_or_else(|| "Unknown".to_string()),
                            default: param.default.clone(),
                        })
                        .collect(),
                );
            }
        }
    }
    params
}

fn declared_binding_types(ast: &AstProject) -> HashMap<String, String> {
    let mut bindings = HashMap::new();
    for file in &ast.files {
        for item in &file.items {
            if let Item::Binding(binding) = item {
                let type_ = binding
                    .type_name
                    .clone()
                    .unwrap_or_else(|| "Unknown".to_string());
                bindings.insert(binding.name.clone(), type_);
            }
        }
    }
    bindings
}

fn infer_binding_types(ast: &AstProject, context: &mut LowerContext<'_>) {
    for file in &ast.files {
        context.current_imports = file.import_bindings();
        for item in &file.items {
            if let Item::Binding(binding) = item {
                if binding.type_name.is_some() {
                    continue;
                }
                if let Some(value) = &binding.value {
                    let locals = HashMap::new();
                    if let Some(type_) = expression_type(value, &locals, context) {
                        context.binding_types.insert(binding.name.clone(), type_);
                    }
                }
            }
        }
    }
}

fn expression_type(
    expression: &Expression,
    locals: &HashMap<String, String>,
    context: &LowerContext<'_>,
) -> Option<String> {
    match expression {
        Expression::String(_) => Some("String".to_string()),
        Expression::Number(value) => Some(
            match numeric::classify_literal(value).1 {
                numeric::LiteralType::Integer => "Integer",
                numeric::LiteralType::Float => "Float",
                numeric::LiteralType::Fixed => "Fixed",
                numeric::LiteralType::Money => "Money",
            }
            .to_string(),
        ),
        Expression::Scalar(_) => Some("Scalar".to_string()),
        Expression::Boolean(_) => Some("Boolean".to_string()),
        Expression::Identifier(value) if value == "NOTHING" => Some("Nothing".to_string()),
        Expression::Identifier(value) => {
            let canonical_value = canonical_import_name(value, context);
            if builtins::is_package_constant(&canonical_value) {
                builtins::package_constant_type_name(&canonical_value).map(str::to_string)
            } else {
                locals
                    .get(value)
                    .cloned()
                    .or_else(|| context.binding_types.get(value).cloned())
                    .or_else(|| context.function_types.get(value).cloned())
                    .or_else(|| context.function_types.get(&canonical_value).cloned())
            }
        }
        Expression::Constructor { type_name, .. } => {
            let canonical_type_name = canonical_import_name(type_name, context);
            context
                .type_index
                .constructor_result(&canonical_type_name)
                .or_else(|| context.type_index.constructor_result(type_name))
        }
        Expression::WithUpdate { target, .. } => expression_type(target, locals, context),
        Expression::ListLiteral(values) => {
            let Some(first) = values.first() else {
                return Some("List OF Unknown".to_string());
            };
            expression_type(first, locals, context).map(|element| format!("List OF {element}"))
        }
        Expression::MapLiteral {
            key_type,
            value_type,
            ..
        } => Some(format!("Map OF {key_type} TO {value_type}")),
        Expression::MemberAccess { target, member } => {
            if let Expression::Identifier(type_name) = target.as_ref() {
                if context
                    .type_index
                    .enums
                    .get(type_name)
                    .is_some_and(|members| members.iter().any(|name| name == member))
                {
                    return Some(type_name.clone());
                }
            }
            let target_type = expression_type(target, locals, context)?;
            // `s.state` on a `RES` value yields its `STATE` record type, carried
            // in the resource type string (`File STATE FileState`).
            if member == "state" {
                if let Some(state) = crate::builtins::resource::state_type_name(&target_type) {
                    return Some(state.to_string());
                }
            }
            // `t.result` is removed; worker outcomes are retrieved only via
            // `thread::waitFor`. (Typecheck rejects `.result` before IR.)
            if target_type == "Error" {
                return match member.as_str() {
                    "code" => Some("Integer".to_string()),
                    "message" => Some("String".to_string()),
                    _ => None,
                };
            }
            if let Some((key_type, value_type)) = parse_map_entry_type(&target_type) {
                return match member.as_str() {
                    "key" => Some(key_type),
                    "value" => Some(value_type),
                    _ => None,
                };
            }
            context.type_index.record_field_type(&target_type, member)
        }
        Expression::Call {
            callee, arguments, ..
        } => {
            let canonical_callee = canonical_import_name(callee, context);
            if builtins::general::is_general_call(&canonical_callee) {
                let normalized =
                    normalize_builtin_call_arguments(canonical_callee.as_str(), arguments);
                if builtins::collections::unary_callback_member_bare(callee)
                    && normalized.len() == 2
                {
                    if let Expression::Identifier(predicate) = normalized[1] {
                        if let Some(collection_type) =
                            expression_type(normalized[0], locals, context)
                        {
                            if let Some(predicate_type) = collection_type
                                .strip_prefix("List OF ")
                                .and_then(|element| {
                                    builtins::general::filter_predicate_type(predicate, element)
                                })
                            {
                                let arg_types = vec![collection_type, predicate_type];
                                return builtins::general::resolve_call(
                                    &canonical_callee,
                                    &arg_types,
                                )
                                .map(|resolved| resolved.return_type.to_string());
                            }
                        }
                    }
                }
                let arg_types = normalized
                    .iter()
                    .map(|argument| expression_type(argument, locals, context))
                    .collect::<Option<Vec<_>>>()?;
                return builtins::general::resolve_call(&canonical_callee, &arg_types)
                    .map(|resolved| resolved.return_type.to_string());
            }
            if builtins::collections::is_native_member_call(&canonical_callee) {
                let normalized =
                    normalize_builtin_call_arguments(canonical_callee.as_str(), arguments);
                if builtins::collections::unary_callback_member(&canonical_callee)
                    && normalized.len() == 2
                {
                    if let Expression::Identifier(predicate) = normalized[1] {
                        if let Some(collection_type) =
                            expression_type(normalized[0], locals, context)
                        {
                            if let Some(predicate_type) = collection_type
                                .strip_prefix("List OF ")
                                .and_then(|element| {
                                    builtins::general::filter_predicate_type(predicate, element)
                                })
                            {
                                let arg_types = vec![collection_type, predicate_type];
                                return builtins::collections::resolve_call(
                                    &canonical_callee,
                                    &arg_types,
                                )
                                .map(|resolved| resolved.return_type.to_string());
                            }
                        }
                    }
                }
                let arg_types = normalized
                    .iter()
                    .map(|argument| expression_type(argument, locals, context))
                    .collect::<Option<Vec<_>>>()?;
                return builtins::collections::resolve_call(&canonical_callee, &arg_types)
                    .map(|resolved| resolved.return_type.to_string());
            }
            if builtins::strings::is_strings_call(&canonical_callee) {
                let arg_types =
                    normalize_builtin_call_arguments(canonical_callee.as_str(), arguments)
                        .iter()
                        .map(|argument| expression_type(argument, locals, context))
                        .collect::<Option<Vec<_>>>()?;
                return builtins::strings::resolve_call(&canonical_callee, &arg_types)
                    .map(|resolved| resolved.return_type.to_string());
            }
            if builtins::math::is_math_call(&canonical_callee) {
                let arg_types =
                    normalize_builtin_call_arguments(canonical_callee.as_str(), arguments)
                        .iter()
                        .map(|argument| expression_type(argument, locals, context))
                        .collect::<Option<Vec<_>>>()?;
                return builtins::math::resolve_call(&canonical_callee, &arg_types)
                    .map(|resolved| resolved.return_type.to_string());
            }
            if builtins::vector::is_vector_call(&canonical_callee) {
                let arg_types =
                    normalize_builtin_call_arguments(canonical_callee.as_str(), arguments)
                        .iter()
                        .map(|argument| expression_type(argument, locals, context))
                        .collect::<Option<Vec<_>>>()?;
                return builtins::vector::resolve_call(&canonical_callee, &arg_types)
                    .map(|resolved| resolved.return_type.to_string());
            }
            if builtins::bits::is_bits_call(&canonical_callee) {
                let arg_types =
                    normalize_builtin_call_arguments(canonical_callee.as_str(), arguments)
                        .iter()
                        .map(|argument| expression_type(argument, locals, context))
                        .collect::<Option<Vec<_>>>()?;
                return builtins::bits::resolve_call(&canonical_callee, &arg_types)
                    .map(|resolved| resolved.return_type.to_string());
            }
            if builtins::fs::is_fs_call(&canonical_callee) {
                let arg_types =
                    normalize_builtin_call_arguments(canonical_callee.as_str(), arguments)
                        .iter()
                        .map(|argument| expression_type(argument, locals, context))
                        .collect::<Option<Vec<_>>>()?;
                return builtins::fs::resolve_call(&canonical_callee, &arg_types)
                    .map(|resolved| resolved.return_type.to_string());
            }
            if builtins::io::is_io_call(&canonical_callee) {
                let arg_types =
                    normalize_builtin_call_arguments(canonical_callee.as_str(), arguments)
                        .iter()
                        .map(|argument| expression_type(argument, locals, context))
                        .collect::<Option<Vec<_>>>()?;
                return builtins::io::resolve_call(&canonical_callee, &arg_types)
                    .map(|resolved| resolved.return_type.to_string());
            }
            if builtins::net::is_net_call(&canonical_callee) {
                let arg_types =
                    normalize_builtin_call_arguments(canonical_callee.as_str(), arguments)
                        .iter()
                        .map(|argument| expression_type(argument, locals, context))
                        .collect::<Option<Vec<_>>>()?;
                return builtins::net::resolve_call(&canonical_callee, &arg_types)
                    .map(|resolved| resolved.return_type.to_string());
            }
            if builtins::os::is_os_call(&canonical_callee) {
                let arg_types =
                    normalize_builtin_call_arguments(canonical_callee.as_str(), arguments)
                        .iter()
                        .map(|argument| expression_type(argument, locals, context))
                        .collect::<Option<Vec<_>>>()?;
                return builtins::os::resolve_call(&canonical_callee, &arg_types)
                    .map(|resolved| resolved.return_type.to_string());
            }
            if builtins::tls::is_tls_call(&canonical_callee) {
                let arg_types =
                    normalize_builtin_call_arguments(canonical_callee.as_str(), arguments)
                        .iter()
                        .map(|argument| expression_type(argument, locals, context))
                        .collect::<Option<Vec<_>>>()?;
                return builtins::tls::resolve_call(&canonical_callee, &arg_types)
                    .map(|resolved| resolved.return_type.to_string());
            }
            if builtins::audio::is_audio_call(&canonical_callee) {
                let arg_types =
                    normalize_builtin_call_arguments(canonical_callee.as_str(), arguments)
                        .iter()
                        .map(|argument| expression_type(argument, locals, context))
                        .collect::<Option<Vec<_>>>()?;
                return builtins::audio::resolve_call(&canonical_callee, &arg_types)
                    .map(|resolved| resolved.return_type.to_string());
            }
            if builtins::http::is_http_call(&canonical_callee) {
                let arg_types =
                    normalize_builtin_call_arguments(canonical_callee.as_str(), arguments)
                        .iter()
                        .map(|argument| expression_type(argument, locals, context))
                        .collect::<Option<Vec<_>>>()?;
                return builtins::http::resolve_call(&canonical_callee, &arg_types)
                    .map(|resolved| resolved.return_type.to_string());
            }
            if builtins::json::is_json_call(&canonical_callee) {
                let arg_types =
                    normalize_builtin_call_arguments(canonical_callee.as_str(), arguments)
                        .iter()
                        .map(|argument| expression_type(argument, locals, context))
                        .collect::<Option<Vec<_>>>()?;
                return builtins::json::resolve_call(&canonical_callee, &arg_types)
                    .map(|resolved| resolved.return_type.to_string());
            }
            if builtins::csv::is_csv_call(&canonical_callee) {
                let arg_types =
                    normalize_builtin_call_arguments(canonical_callee.as_str(), arguments)
                        .iter()
                        .map(|argument| expression_type(argument, locals, context))
                        .collect::<Option<Vec<_>>>()?;
                return builtins::csv::resolve_call(&canonical_callee, &arg_types)
                    .map(|resolved| resolved.return_type.to_string());
            }
            if builtins::regex::is_regex_call(&canonical_callee) {
                let arg_types =
                    normalize_builtin_call_arguments(canonical_callee.as_str(), arguments)
                        .iter()
                        .map(|argument| expression_type(argument, locals, context))
                        .collect::<Option<Vec<_>>>()?;
                return builtins::regex::resolve_call(&canonical_callee, &arg_types)
                    .map(|resolved| resolved.return_type.to_string());
            }
            if builtins::datetime::is_datetime_call(&canonical_callee) {
                let arg_types =
                    normalize_builtin_call_arguments(canonical_callee.as_str(), arguments)
                        .iter()
                        .map(|argument| expression_type(argument, locals, context))
                        .collect::<Option<Vec<_>>>()?;
                return builtins::datetime::resolve_call(&canonical_callee, &arg_types)
                    .map(|resolved| resolved.return_type.to_string());
            }
            if builtins::crypto::is_crypto_call(&canonical_callee) {
                let arg_types =
                    normalize_builtin_call_arguments(canonical_callee.as_str(), arguments)
                        .iter()
                        .map(|argument| expression_type(argument, locals, context))
                        .collect::<Option<Vec<_>>>()?;
                return builtins::crypto::resolve_call(&canonical_callee, &arg_types)
                    .map(|resolved| resolved.return_type.to_string());
            }
            if builtins::thread::is_thread_call(&canonical_callee) {
                let arg_types =
                    normalize_builtin_call_arguments(canonical_callee.as_str(), arguments)
                        .iter()
                        .map(|argument| expression_type(argument, locals, context))
                        .collect::<Option<Vec<_>>>()?;
                return builtins::thread::resolve_call(&canonical_callee, &arg_types)
                    .map(|resolved| resolved.return_type.to_string());
            }
            builtins::call_return_type_name(&canonical_callee)
                .map(str::to_string)
                .or_else(|| context.function_returns.get(callee).cloned())
                .or_else(|| context.function_returns.get(&canonical_callee).cloned())
                .or_else(|| {
                    locals
                        .get(callee)
                        .and_then(|type_| function_return_from_type(type_))
                })
                .or_else(|| {
                    // A global binding holding a function value is callable too
                    // (bug-198): infer its return type from the declared FUNC type,
                    // mirroring the local-binding fallback above.
                    context
                        .binding_types
                        .get(callee)
                        .and_then(|type_| function_return_from_type(type_))
                })
        }
        Expression::Lambda {
            params,
            body,
            assign_target,
        } => {
            let mut nested = locals.clone();
            let param_types = params
                .iter()
                .map(|param| {
                    let type_ = param
                        .type_name
                        .clone()
                        .unwrap_or_else(|| "Unknown".to_string());
                    nested.insert(param.name.clone(), type_.clone());
                    type_
                })
                .collect::<Vec<_>>();
            // An assignment-bodied lambda yields `Nothing`.
            let returns = if assign_target.is_some() {
                "Nothing".to_string()
            } else {
                expression_type(body, &nested, context)?
            };
            Some(format!("FUNC({}) AS {returns}", param_types.join(", ")))
        }
        Expression::Binary {
            left,
            operator,
            right,
            ..
        } => {
            if matches!(
                operator.as_str(),
                "=" | "<>" | "<" | ">" | "<=" | ">=" | "AND" | "OR" | "XOR"
            ) {
                return Some("Boolean".to_string());
            }
            if operator == "&" {
                return Some("String".to_string());
            }
            let left = expression_type(left, locals, context)?;
            let right = expression_type(right, locals, context)?;
            Some(numeric_binary_result_type(operator, &left, &right).to_string())
        }
        Expression::Unary {
            operator, operand, ..
        } => {
            if operator == "NOT" {
                Some("Boolean".to_string())
            } else {
                expression_type(operand, locals, context)
            }
        }
        Expression::Trapped { expression, .. } => expression_type(expression, locals, context),
    }
}

fn function_return_from_type(type_: &str) -> Option<String> {
    type_
        .strip_prefix("FUNC(")
        .or_else(|| type_.strip_prefix("ISOLATED FUNC("))
        .and_then(|rest| rest.split_once(") AS "))
        .map(|(_, return_type)| return_type.to_string())
}

fn function_param_types_from_type(type_: &str) -> Option<Vec<String>> {
    let rest = type_
        .strip_prefix("FUNC(")
        .or_else(|| type_.strip_prefix("ISOLATED FUNC("))?;
    let (params, _) = rest.split_once(") AS ")?;
    if params.trim().is_empty() {
        return Some(Vec::new());
    }
    Some(params.split(", ").map(str::to_string).collect())
}

/// Lower resource-plane thread calls to their dedicated runtime helpers. The
/// resource plane mirrors `send`/`receive` but runs on a separate per-thread
/// resource queue so a thread can carry both a data channel and a resource
/// channel at once (§7).
fn thread_resource_plane_target(name: &str) -> &str {
    match name {
        "thread.transfer" => "thread.transferResource",
        "thread.accept" => "thread.acceptResource",
        other => other,
    }
}

fn canonical_import_name(name: &str, context: &LowerContext<'_>) -> String {
    let Some((binding, rest)) = name.split_once('.') else {
        return name.to_string();
    };
    let Some(package) = context.current_imports.get(binding) else {
        return name.to_string();
    };
    format!("{package}.{rest}")
}

fn call_argument_expected_type(
    callee: &str,
    index: usize,
    arguments: &[CallArg],
    locals: &HashMap<String, String>,
    context: &LowerContext<'_>,
) -> Option<String> {
    let canonical_callee = canonical_import_name(callee, context);
    if callee == "toString" && index == 1 && arguments.len() == 2 {
        return Some("Byte".to_string());
    }
    if let Some(params) = builtin_argument_types(&canonical_callee) {
        return params.get(index).cloned();
    }
    context
        .function_params
        .get(callee)
        .or_else(|| context.function_params.get(&canonical_callee))
        .and_then(|params| params.get(index).map(|param| param.type_.clone()))
        .or_else(|| {
            locals
                .get(callee)
                .and_then(|type_| function_param_types_from_type(type_))
                .and_then(|params| params.get(index).cloned())
        })
}

fn builtin_argument_types(callee: &str) -> Option<Vec<String>> {
    let expected = builtins::general::expected_arguments(callee)
        .or_else(|| builtins::strings::expected_arguments(callee))
        .or_else(|| builtins::math::expected_arguments(callee))
        .or_else(|| builtins::bits::expected_arguments(callee))
        .or_else(|| builtins::fs::expected_arguments(callee))
        .or_else(|| builtins::os::expected_arguments(callee))
        .or_else(|| builtins::io::expected_arguments(callee))
        .or_else(|| builtins::json::expected_arguments(callee))
        .or_else(|| builtins::csv::expected_arguments(callee))
        .or_else(|| builtins::regex::expected_arguments(callee))
        .or_else(|| builtins::net::argument_types(callee))
        .or_else(|| builtins::tls::argument_types(callee))
        .or_else(|| builtins::audio::argument_types(callee))
        .or_else(|| builtins::crypto::argument_types(callee))
        .or_else(|| builtins::http::expected_arguments(callee))
        .or_else(|| builtins::thread::expected_arguments(callee))?;
    // Overloaded/optional-argument descriptions (e.g. `strings.find`'s
    // `"String, String[, Integer]"`) are not a concrete positional signature;
    // skip them so we don't hand the lowerer a bracket-mangled expected type.
    if expected.contains('[') || expected.contains(" or ") {
        return None;
    }
    let params = expected.split(", ").map(str::to_string).collect::<Vec<_>>();
    if params.iter().any(|param| uses_generic_placeholder(param)) {
        return None;
    }
    Some(params)
}

fn normalize_builtin_call_arguments<'a>(
    callee: &str,
    arguments: &'a [CallArg],
) -> Vec<&'a Expression> {
    if !arguments
        .iter()
        .any(|argument| matches!(argument, CallArg::Named { .. }))
    {
        return arguments.iter().map(call_arg_value).collect();
    }
    // A builtin whose overloads place a name at different positions selects the
    // overload first; the type checker has already proven one exists.
    if let Some(overloads) = builtins::call_param_name_overloads(callee) {
        return normalize_overloaded_builtin_call_arguments(overloads, arguments);
    }
    let Some(param_names) = builtins::call_param_names(callee) else {
        return arguments.iter().map(call_arg_value).collect();
    };
    let mut ordered = vec![None; param_names.len()];
    let mut next_positional = 0usize;
    let mut extras = Vec::new();
    for argument in arguments {
        match argument {
            CallArg::Positional(value) => {
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
            CallArg::Named { name, value, .. } => {
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
/// table, mirroring `syntaxcheck`'s selection so both agree on which parameter a
/// name binds to. An unresolvable call was already rejected by the type checker;
/// keep its source order so lowering has something well-formed to walk.
fn normalize_overloaded_builtin_call_arguments<'a>(
    overloads: &[&[&str]],
    arguments: &'a [CallArg],
) -> Vec<&'a Expression> {
    let positionals: Vec<&Expression> = arguments
        .iter()
        .filter_map(|argument| match argument {
            CallArg::Positional(value) => Some(value),
            CallArg::Named { .. } => None,
        })
        .collect();
    let named: Vec<(&str, &Expression)> = arguments
        .iter()
        .filter_map(|argument| match argument {
            CallArg::Named { name, value, .. } => Some((name.as_str(), value)),
            CallArg::Positional(_) => None,
        })
        .collect();
    let supplied_names: Vec<&str> = named.iter().map(|(name, _)| *name).collect();
    let Some(params) =
        builtins::select_param_name_overload(overloads, positionals.len(), &supplied_names)
    else {
        return arguments.iter().map(call_arg_value).collect();
    };

    let mut ordered: Vec<Option<&Expression>> = vec![None; params.len()];
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
    arguments: &'a [CallArg],
    context: &LowerContext<'_>,
) -> Vec<Option<&'a Expression>> {
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
            CallArg::Positional(value) => {
                while next_positional < ordered.len() && ordered[next_positional].is_some() {
                    next_positional += 1;
                }
                if next_positional < ordered.len() {
                    ordered[next_positional] = Some(value);
                    next_positional += 1;
                }
            }
            CallArg::Named { name, value, .. } => {
                if let Some(index) = params.iter().position(|param| param.name == *name) {
                    ordered[index] = Some(value);
                }
            }
        }
    }
    ordered
}

fn lower_local_call_arguments(
    callee: &str,
    arguments: &[CallArg],
    locals: &HashMap<String, String>,
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
            let expected = call_argument_expected_type(callee, index, arguments, locals, context);
            match argument {
                Some(argument) => Some(lower_expression_with_expected(
                    argument,
                    expected.as_deref(),
                    locals,
                    context,
                )),
                None => params.get(index).and_then(|param| {
                    param.default.as_ref().map(|default| {
                        lower_expression_with_expected(default, Some(&param.type_), locals, context)
                    })
                }),
            }
        })
        .collect()
}

fn call_arg_value(argument: &CallArg) -> &Expression {
    match argument {
        CallArg::Positional(value) => value,
        CallArg::Named { value, .. } => value,
    }
}

fn uses_generic_placeholder(type_: &str) -> bool {
    matches!(type_, "T" | "K" | "V")
        || type_.contains(" OF T")
        || type_.contains(" OF K")
        || type_.contains(" OF V")
        || type_.contains(" TO T")
        || type_.contains(" TO K")
        || type_.contains(" TO V")
}

fn lower_expression(
    expression: &Expression,
    locals: &HashMap<String, String>,
    context: &mut LowerContext<'_>,
) -> IrValue {
    lower_expression_with_expected(expression, None, locals, context)
}

/// The function type to give a general built-in predicate named in a value
/// position, when `expected` is a concrete unary Boolean function type it
/// accepts (bug-368).
///
/// Mirrors `syntaxcheck`'s `builtin_predicate_value_type`: both consult
/// `filter_predicate_type`, so the type the checker assigns and the type the
/// `FunctionRef` carries cannot diverge. A divergence would emit a wrapper under
/// one symbol and reference another.
fn builtin_predicate_ref_type(name: &str, expected: &str) -> Option<String> {
    let (params, returns) = function_type_parts_for_predicate(expected)?;
    if params.len() != 1 || returns != "Boolean" {
        return None;
    }
    builtins::general::filter_predicate_type(name, params[0])
}

/// Split `FUNC(A) AS R` into its parameter list and return type. Deliberately
/// local and minimal: it only needs to recognize the unary predicate shape.
fn function_type_parts_for_predicate(type_: &str) -> Option<(Vec<&str>, &str)> {
    let rest = type_.strip_prefix("FUNC(")?;
    let (params, returns) = rest.split_once(") AS ")?;
    let params: Vec<&str> = if params.trim().is_empty() {
        Vec::new()
    } else {
        params.split(", ").map(str::trim).collect()
    };
    Some((params, returns.trim()))
}

fn lower_expression_with_expected(
    expression: &Expression,
    expected: Option<&str>,
    locals: &HashMap<String, String>,
    context: &mut LowerContext<'_>,
) -> IrValue {
    match expression {
        Expression::String(value) => IrValue::Const {
            type_: "String".to_string(),
            value: value.clone(),
        },
        Expression::Number(value) => {
            let (canonical, literal_type) = numeric::classify_literal(value);
            // An explicit `f`/`F` *suffix* makes the literal intrinsically
            // Float/Fixed and wins over the expected type (plan-28-B §4.3). An
            // *unsuffixed* literal — including a `.`/exponent Float-shaped one — is
            // untyped and still coerces to a `Fixed`/`Byte` slot, so the expected
            // type wins there (the pre-existing rule). In plan-28-A no suffix or
            // exponent is lexed yet, so this is byte-identical to the previous
            // expected-first behavior.
            let is_suffixed = value.ends_with('f')
                || value.ends_with('F')
                || value.ends_with('m')
                || value.ends_with('M');
            let type_ = if is_suffixed {
                match literal_type {
                    numeric::LiteralType::Fixed => "Fixed",
                    numeric::LiteralType::Money => "Money",
                    _ => "Float",
                }
                .to_string()
            } else if expected == Some("Fixed") {
                "Fixed".to_string()
            } else if expected == Some("Byte") {
                "Byte".to_string()
            } else if expected == Some("Money") {
                // An unsuffixed decimal literal coerces to a Money slot
                // (`LET a AS Money = 1.25`), mirroring the Fixed/Byte paths
                // (plan-29-A §4.4).
                "Money".to_string()
            } else {
                match literal_type {
                    numeric::LiteralType::Float => "Float",
                    numeric::LiteralType::Fixed => "Fixed",
                    numeric::LiteralType::Money => "Money",
                    numeric::LiteralType::Integer => "Integer",
                }
                .to_string()
            };
            IrValue::Const {
                type_,
                value: canonical,
            }
        }
        Expression::Scalar(code_point) => IrValue::Const {
            type_: "Scalar".to_string(),
            value: code_point.to_string(),
        },
        Expression::Boolean(value) => IrValue::Const {
            type_: "Boolean".to_string(),
            value: value.to_string(),
        },
        Expression::Identifier(value) if value == "NOTHING" => IrValue::Const {
            type_: "Nothing".to_string(),
            value: "NOTHING".to_string(),
        },
        Expression::Identifier(value) => {
            let canonical_value = canonical_import_name(value, context);
            // A `vector::` record constant (`vector::upFloat3`) inlines a record
            // constructor at every use site, copying by value (plan-06-vector.md
            // §4.19). It must be handled before the scalar-fold path below, which
            // expects a single literal value.
            if let Some((type_, components)) =
                builtins::vector::constant_components(&canonical_value)
            {
                return IrValue::Constructor {
                    type_,
                    args: components
                        .into_iter()
                        .map(|(component_type, component_value)| IrValue::Const {
                            type_: component_type,
                            value: component_value,
                        })
                        .collect(),
                };
            }
            if builtins::is_package_constant(&canonical_value) {
                let type_ = builtins::package_constant_type_name(&canonical_value)
                    .unwrap_or("Unknown")
                    .to_string();
                let value = builtins::package_constant_value(&canonical_value)
                    .expect("recognized package constant has a value")
                    .to_string();
                return IrValue::Const { type_, value };
            }

            let base = if locals.contains_key(value) {
                IrValue::Local(value.clone())
            } else if let Some(type_) = context
                .function_types
                .get(value)
                .or_else(|| context.function_types.get(&canonical_value))
            {
                IrValue::FunctionRef {
                    name: canonical_value,
                    type_: type_.clone(),
                }
            } else if context.binding_types.contains_key(value) {
                IrValue::Global(value.clone())
            } else if let Some(type_) = expected.and_then(|expected| {
                // A general built-in predicate in a value position (bug-368).
                // These are lowered inline at a direct call site and so have no
                // entry in `function_types`; the out-of-line body is emitted on
                // demand from the `FunctionRef`s collected here
                // (`builtin_function_refs` -> `lower_builtin_function_wrapper`).
                // Without this arm the reference survived as a `Local` that
                // nothing defines, and surfaced to the user as the internal
                // `NIR local reference '<x>' does not resolve`.
                builtin_predicate_ref_type(value, expected)
            }) {
                IrValue::FunctionRef {
                    name: value.clone(),
                    type_,
                }
            } else {
                IrValue::Local(value.clone())
            };
            wrap_union_value(base, expression, expected, locals, context)
        }
        Expression::Call {
            callee,
            arguments,
            line,
            column,
        } => {
            let canonical_callee = canonical_import_name(callee, context);
            let loc = IrSourceLoc {
                line: *line as u32,
                column: *column as u32,
            };
            // `error(code, message)` is a language built-in that produces a
            // read-only `Error` record stamped with the source location of this
            // call expression. Lower it to ordinary record constructors so the
            // rest of the pipeline treats `Error`/`ErrorLoc` as plain records.
            if canonical_callee == "error"
                && !context.function_params.contains_key(callee)
                && !context.function_params.contains_key(&canonical_callee)
            {
                let mut lowered = arguments
                    .iter()
                    .map(|argument| lower_expression(call_arg_value(argument), locals, context));
                // Typecheck guarantees error(code, message) has both args;
                // total lowering (plan-20-D) substitutes Unknown-typed const
                // placeholders when they are absent rather than panicking.
                let placeholder = || IrValue::Const {
                    type_: "Unknown".to_string(),
                    value: String::new(),
                };
                let code = lowered.next().unwrap_or_else(placeholder);
                let message = lowered.next().unwrap_or_else(placeholder);
                return build_error_value(code, message, &context.current_file, loc);
            }
            let normalized_builtin =
                normalize_builtin_call_arguments(canonical_callee.as_str(), arguments);
            // A bare general built-in predicate as the callback of a native
            // higher-order member (bug-368). Only this exact shape diverts: the
            // callback's parameter type is the list's element type, which is not
            // written at the call site, so the `FunctionRef` has to be built
            // here where both are in hand.
            //
            // Everything else — a lambda, a named FUNC, an already-typed
            // function value — MUST fall through to the general path below,
            // which supplies the expected type and sets `nonescaping_callback`.
            // Diverting those too silently dropped `forEach`'s licence for a
            // lambda to slot-borrow a `MUT` capture.
            let builtin_predicate_arg =
                (builtins::collections::unary_callback_member(&canonical_callee)
                    && normalized_builtin.len() == 2)
                    .then(|| match normalized_builtin[1] {
                        Expression::Identifier(predicate) => {
                            expression_type(normalized_builtin[0], locals, context)
                                .and_then(|collection_type| {
                                    collection_type
                                        .strip_prefix("List OF ")
                                        .and_then(|element| {
                                            builtins::general::filter_predicate_type(
                                                predicate, element,
                                            )
                                        })
                                })
                                .map(|predicate_type| IrValue::FunctionRef {
                                    name: predicate.clone(),
                                    type_: predicate_type,
                                })
                        }
                        _ => None,
                    })
                    .flatten();
            let args = if let Some(predicate_ref) = builtin_predicate_arg {
                vec![
                    lower_expression(normalized_builtin[0], locals, context),
                    predicate_ref,
                ]
            } else if context.function_params.contains_key(callee)
                || context.function_params.contains_key(&canonical_callee)
            {
                lower_local_call_arguments(callee, arguments, locals, context)
            } else {
                normalized_builtin
                    .iter()
                    .enumerate()
                    .map(|(index, argument)| {
                        let expected =
                            call_argument_expected_type(callee, index, arguments, locals, context);
                        // License a `MUT` slot-borrow capture for a lambda in a
                        // non-escaping callback position (e.g. `forEach`'s action).
                        // The lambda lowering consumes it; reset afterward so a
                        // non-lambda argument never carries it.
                        context.nonescaping_callback =
                            builtins::is_nonescaping_callback_arg(&canonical_callee, index);
                        let value = lower_expression_with_expected(
                            argument,
                            expected.as_deref(),
                            locals,
                            context,
                        );
                        context.nonescaping_callback = false;
                        value
                    })
                    .collect()
            };
            // Pad optional trailing arguments (`tls.connect` defaults)
            // with constants so the fixed-ABI runtime helper always receives
            // every parameter (plan-03-net.md §4).
            let mut args = args;
            for (type_, value) in
                builtins::tls::default_argument_padding(&canonical_callee, args.len())
            {
                args.push(IrValue::Const {
                    type_: (*type_).to_string(),
                    value: (*value).to_string(),
                });
            }
            for (type_, value) in
                builtins::regex::default_argument_padding(&canonical_callee, args.len())
            {
                args.push(IrValue::Const {
                    type_: (*type_).to_string(),
                    value: (*value).to_string(),
                });
            }
            for (type_, value) in
                builtins::datetime::default_argument_padding(&canonical_callee, args.len())
            {
                args.push(IrValue::Const {
                    type_: (*type_).to_string(),
                    value: (*value).to_string(),
                });
            }
            // `crypto`'s AEAD `aad` argument defaults to the empty byte list; a
            // `List OF ...` default lowers to an empty list literal, not a scalar
            // const (plan-04-crypto.md §A.5).
            for (type_, value) in
                builtins::crypto::default_argument_padding(&canonical_callee, args.len())
            {
                if type_.starts_with("List OF ") {
                    args.push(IrValue::ListLiteral {
                        type_: (*type_).to_string(),
                        values: Vec::new(),
                    });
                } else {
                    args.push(IrValue::Const {
                        type_: (*type_).to_string(),
                        value: (*value).to_string(),
                    });
                }
            }
            // `http::read`/`write` default the `headers` argument to the empty map
            // and the method to a literal (plan-03-http.md §B.1). A `Map OF ...`
            // default lowers to an empty map literal, not a scalar const.
            for (type_, value) in
                builtins::http::default_argument_padding(&canonical_callee, args.len())
            {
                if parse_map_type(type_).is_some() {
                    args.push(IrValue::MapLiteral {
                        type_: (*type_).to_string(),
                        entries: Vec::new(),
                    });
                } else {
                    args.push(IrValue::Const {
                        type_: (*type_).to_string(),
                        value: (*value).to_string(),
                    });
                }
            }
            // Dequalify migrated `collections::`/`strings::` native members back
            // to their bare lowering names (plan-01-functions.md §5): the native
            // code generator stays keyed on `get`/`transform`/`find`/... .
            // Migrated `collections::`/`strings::` members keep their qualified,
            // dot-containing target all the way to codegen (plan-01-functions.md
            // §5). The native code generator dispatches on the qualified name, so
            // the freed bare names (`get`, `transform`, ...) can be redefined by
            // user code without colliding with the native lowering.
            // `implementation_name` returns the `__pkg_name` form; the injected
            // package's function is lexed in internal mode, so its actual name
            // carries the internal sigil. Internalize the dispatch target to match.
            // `datetime::` is arity-aware: the overloaded constructors and
            // `parse` select a distinct internal name by argument count (§5.1.1).
            // Its OS-seam intrinsics return `None`, staying `datetime.*` runtime
            // helper calls.
            // A general built-in call (`toString(x)`, `len(x)`, …) over a built-in
            // package value type routes to that package's internal override helper
            // (plan-01-overload.md §B.2 / Phase 6), e.g. `toString(net::Url)` ->
            // `#net_urlToString`. User overrides need no routing here — the
            // monomorphizer already rewrote them to a concrete symbol (Phase 5).
            let package_override = if builtins::general::is_overridable(&canonical_callee) {
                arguments
                    .first()
                    .map(call_arg_value)
                    .and_then(|argument| expression_type(argument, locals, context))
                    .and_then(|type_| builtins::general_override_target(&canonical_callee, &type_))
                    .map(crate::internal_name::internalize)
            } else {
                None
            };
            let resolved_target = package_override
                .or_else(|| {
                    // `tls::close` spans two record shapes; a `TlsListener`
                    // operand routes to the listener-shaped internal close
                    // helper while `tls::close` stays the single user-facing
                    // name (plan-06-tls-server.md §4.1/§6.4). The target is a
                    // runtime helper, not a source companion, so it is not
                    // internalized.
                    if canonical_callee != "tls.close" {
                        return None;
                    }
                    arguments
                        .first()
                        .map(call_arg_value)
                        .and_then(|argument| expression_type(argument, locals, context))
                        .filter(|type_| type_ == builtins::tls::TLS_LISTENER_TYPE)
                        .map(|_| builtins::tls::CLOSE_LISTENER.to_string())
                })
                .or_else(|| {
                    // `audio::` rewrites the overloads whose *body* differs while
                    // no user error is reachable onto their own internal
                    // runtime-helper name: the named-device opens, timed
                    // `read`/`poll`, and per-direction `close` (plan-33-A §5).
                    // The target is a runtime helper, not a source companion, so
                    // it is not internalized.
                    if !builtins::audio::is_audio_call(&canonical_callee) {
                        return None;
                    }
                    let arg_types: Vec<String> = arguments
                        .iter()
                        .map(call_arg_value)
                        .map(|argument| {
                            expression_type(argument, locals, context).unwrap_or_default()
                        })
                        .collect();
                    builtins::audio::implementation_name(&canonical_callee, &arg_types)
                        .map(str::to_string)
                })
                .or_else(|| {
                    builtins::datetime::implementation_name(&canonical_callee, args.len())
                        .map(|name| crate::internal_name::internalize(&name))
                })
                .or_else(|| {
                    // `vector::` resolves a type-specific internal name from the
                    // call's argument record types (plan-06-vector.md §5), e.g.
                    // `vector.length(Float3)` -> `#vector_length_float3`.
                    if !builtins::vector::is_vector_call(&canonical_callee) {
                        return None;
                    }
                    let arg_types: Vec<String> = arguments
                        .iter()
                        .map(call_arg_value)
                        .map(|argument| {
                            expression_type(argument, locals, context).unwrap_or_default()
                        })
                        .collect();
                    builtins::vector::implementation_name(&canonical_callee, &arg_types)
                        .map(|name| crate::internal_name::internalize(&name))
                })
                .or_else(|| {
                    // `crypto::` maps most calls 1:1, but the hash/HMAC/PBKDF2
                    // functions carry a `String` overload selected by the
                    // relevant argument's type (plan-04-crypto.md §A.2). The
                    // native entry points (`randomBytes`, NIST-EC) return `None`
                    // and stay `crypto.*` runtime-helper calls.
                    if !builtins::crypto::is_crypto_call(&canonical_callee) {
                        return None;
                    }
                    let arg_types: Vec<String> = arguments
                        .iter()
                        .map(call_arg_value)
                        .map(|argument| {
                            expression_type(argument, locals, context).unwrap_or_default()
                        })
                        .collect();
                    builtins::crypto::implementation_name(&canonical_callee, &arg_types)
                        .map(|name| crate::internal_name::internalize(&name))
                })
                .or_else(|| {
                    // `http::handleRequest` is overloaded by listener type
                    // (net::Listener vs tls::Listener), selecting one of two
                    // transport bodies from the first argument's type
                    // (plan-05 §F.5.1). The other http calls map 1:1.
                    if !builtins::http::is_http_call(&canonical_callee) {
                        return None;
                    }
                    let arg_types: Vec<String> = arguments
                        .iter()
                        .map(call_arg_value)
                        .map(|argument| {
                            expression_type(argument, locals, context).unwrap_or_default()
                        })
                        .collect();
                    builtins::http::implementation_name(&canonical_callee, &arg_types)
                        .map(crate::internal_name::internalize)
                })
                .or_else(|| {
                    // `audio::render`/`audio::play` are source-companion members
                    // (`audio_package.mfb`); `play` selects its single- vs
                    // multi-track body from the second argument's type. The
                    // native capture/playback surface returned `None` above and
                    // stays a runtime-helper call.
                    if !builtins::audio::is_audio_call(&canonical_callee) {
                        return None;
                    }
                    let arg_types: Vec<String> = arguments
                        .iter()
                        .map(call_arg_value)
                        .map(|argument| {
                            expression_type(argument, locals, context).unwrap_or_default()
                        })
                        .collect();
                    builtins::audio::source_implementation_name(&canonical_callee, &arg_types)
                        .map(crate::internal_name::internalize)
                })
                .or_else(|| {
                    builtins::json::implementation_name(&canonical_callee)
                        .or_else(|| builtins::csv::implementation_name(&canonical_callee))
                        .or_else(|| builtins::regex::implementation_name(&canonical_callee))
                        .or_else(|| builtins::net::implementation_name(&canonical_callee))
                        .or_else(|| builtins::encoding::implementation_name(&canonical_callee))
                        .or_else(|| builtins::strings::implementation_name(&canonical_callee))
                        .map(crate::internal_name::internalize)
                })
                .unwrap_or_else(|| canonical_callee.clone());
            let result_type = expression_type(expression, locals, context)
                .unwrap_or_else(|| "Unknown".to_string());
            IrValue::Call {
                // The resource plane reuses the proven data-channel runtime:
                // `thread::transfer`/`accept` lower exactly like `send`/`receive`
                // (syntaxcheck already enforced their resource semantics).
                target: thread_resource_plane_target(&resolved_target).to_string(),
                args,
                type_: result_type,
                loc,
            }
        }
        Expression::Lambda {
            params,
            body,
            assign_target,
        } => {
            // Consume the non-escaping callback licence so it applies only to this
            // lambda, not to lambdas nested inside its body.
            let nonescaping = context.nonescaping_callback;
            context.nonescaping_callback = false;
            let name = format!("$lambda{}", context.next_lambda_id);
            context.next_lambda_id += 1;
            let param_names = params
                .iter()
                .map(|param| param.name.clone())
                .collect::<HashSet<_>>();
            let mut captures = captured_locals(body, locals, &param_names);
            // The assignment target is a capture too even if it never appears on
            // the right-hand side (mirrors the type checker).
            if let Some(target) = assign_target {
                if !param_names.contains(target)
                    && !captures.iter().any(|capture| &capture.name == target)
                {
                    if let Some(type_) = locals.get(target) {
                        captures.push(CapturedLocal {
                            name: target.clone(),
                            type_: type_.clone(),
                        });
                    }
                }
            }
            // A `MUT` capture in a proven non-escaping position is a borrow of the
            // parent's slot, not a by-value copy. Everything else
            // is an ordinary copy capture.
            let by_ref = captures
                .iter()
                .map(|capture| nonescaping && context.mutable_locals.contains(&capture.name))
                .collect::<Vec<_>>();
            // Lambdas carry the enclosing statement's span (syntaxcheck reports
            // lambda rules at the threaded statement line).
            let loc = context.current_loc;
            let mut lambda_locals = HashMap::new();
            let ir_params = params
                .iter()
                .map(|param| {
                    let type_ = param
                        .type_name
                        .clone()
                        .unwrap_or_else(|| "Unknown".to_string());
                    lambda_locals.insert(param.name.clone(), type_.clone());
                    IrParam {
                        name: param.name.clone(),
                        type_,
                        default: None,
                        loc,
                    }
                })
                .collect::<Vec<_>>();
            let mut body_ops = captures
                .iter()
                .zip(by_ref.iter())
                .enumerate()
                .map(|(index, (capture, &by_ref))| IrOp::Bind {
                    mutable: by_ref,
                    name: capture.name.clone(),
                    type_: capture.type_.clone(),
                    value: Some(IrValue::Capture {
                        // A closure's environment is far smaller than `u32::MAX`
                        // slots; the cast cannot lose an index a program produces.
                        index: index as u32,
                        type_: capture.type_.clone(),
                        by_ref,
                    }),
                    loc,
                    explicit_type: false,
                })
                .collect::<Vec<_>>();
            for capture in &captures {
                lambda_locals.insert(capture.name.clone(), capture.type_.clone());
            }
            // An assignment-bodied lambda lowers to `target = <body>` followed by a
            // value-less return (it yields `Nothing`); a plain lambda returns its
            // body value.
            let returns = match assign_target {
                Some(target) => {
                    let value = lower_expression(body, &lambda_locals, context);
                    body_ops.push(IrOp::Assign {
                        name: target.clone(),
                        value,
                        loc,
                    });
                    body_ops.push(IrOp::Return { value: None, loc });
                    "Nothing".to_string()
                }
                None => {
                    let returns = expression_type(body, &lambda_locals, context)
                        .unwrap_or_else(|| "Unknown".to_string());
                    let value = lower_expression(body, &lambda_locals, context);
                    body_ops.push(IrOp::Return {
                        value: Some(value),
                        loc,
                    });
                    returns
                }
            };
            context.lambdas.push(IrFunction {
                name: name.clone(),
                visibility: "private".to_string(),
                kind: "func".to_string(),
                isolated: false,
                params: ir_params,
                returns: returns.clone(),
                body: body_ops,
                file: context.current_file.clone(),
                loc,
                resource_owners: HashMap::new(),
            });
            let params = params
                .iter()
                .map(|param| {
                    param
                        .type_name
                        .clone()
                        .unwrap_or_else(|| "Unknown".to_string())
                })
                .collect::<Vec<_>>()
                .join(", ");
            let type_ = format!("FUNC({params}) AS {returns}");
            if captures.is_empty() {
                IrValue::FunctionRef { name, type_ }
            } else {
                IrValue::Closure {
                    name,
                    type_,
                    captures: captures
                        .iter()
                        .zip(by_ref.iter())
                        .map(|(capture, &by_ref)| {
                            if by_ref {
                                // Capture the parent slot's address (a borrow), so
                                // the callback observes and updates the live binding.
                                IrValue::LocalRef {
                                    name: capture.name.clone(),
                                    type_: capture.type_.clone(),
                                }
                            } else {
                                lower_expression(
                                    &Expression::Identifier(capture.name.clone()),
                                    locals,
                                    context,
                                )
                            }
                        })
                        .collect(),
                }
            }
        }
        Expression::Constructor {
            type_name,
            arguments,
        } => {
            let canonical_type_name = canonical_import_name(type_name, context);
            let fields = context
                .type_index
                .records
                .get(&canonical_type_name)
                .or_else(|| context.type_index.records.get(type_name))
                .or_else(|| context.type_index.variant_fields.get(&canonical_type_name))
                .or_else(|| context.type_index.variant_fields.get(type_name));
            let base = IrValue::Constructor {
                type_: canonical_type_name,
                args: lower_constructor_args(arguments, fields, locals, context),
            };
            wrap_union_value(base, expression, expected, locals, context)
        }
        Expression::WithUpdate { target, updates } => {
            let type_ =
                expression_type(target, locals, context).unwrap_or_else(|| "Unknown".to_string());
            let lowered_target = Box::new(lower_expression(target, locals, context));
            let lowered_updates = updates
                .iter()
                .map(|update| {
                    // Coerce a bare numeric literal to the record field's
                    // declared type, mirroring `lower_constructor_args` — else an
                    // unsuffixed literal updating a `Fixed`/`Money` field is typed
                    // `Integer` and reinterpreted as raw bits (bug-156).
                    let field_type = context.type_index.record_field_type(&type_, &update.field);
                    IrRecordUpdate {
                        field: update.field.clone(),
                        value: lower_expression_with_expected(
                            &update.value,
                            field_type.as_deref(),
                            locals,
                            context,
                        ),
                    }
                })
                .collect();
            IrValue::WithUpdate {
                type_,
                target: lowered_target,
                updates: lowered_updates,
            }
        }
        Expression::ListLiteral(values) => {
            let expected_element = expected.and_then(|type_| type_.strip_prefix("List OF "));
            let lowered = values
                .iter()
                .map(|value| {
                    lower_expression_with_expected(value, expected_element, locals, context)
                })
                .collect::<Vec<_>>();
            let element_type = expected_element.map(str::to_string).unwrap_or_else(|| {
                values
                    .first()
                    .and_then(literal_expression_type)
                    .unwrap_or_else(|| "Unknown".to_string())
            });
            IrValue::ListLiteral {
                type_: format!("List OF {element_type}"),
                values: lowered,
            }
        }
        Expression::MapLiteral {
            key_type,
            value_type,
            entries,
        } => {
            let expected_map = expected.and_then(parse_map_type);
            let expected_key = expected_map.as_ref().map(|(key, _)| key.as_str());
            let expected_value = expected_map.as_ref().map(|(_, value)| value.as_str());
            IrValue::MapLiteral {
                type_: format!("Map OF {key_type} TO {value_type}"),
                entries: entries
                    .iter()
                    .map(|(key, value)| {
                        (
                            lower_expression_with_expected(key, expected_key, locals, context),
                            lower_expression_with_expected(value, expected_value, locals, context),
                        )
                    })
                    .collect(),
            }
        }
        Expression::MemberAccess { target, member } => {
            let member_type = expression_type(expression, locals, context)
                .unwrap_or_else(|| "Unknown".to_string());
            IrValue::MemberAccess {
                target: Box::new(lower_expression(target, locals, context)),
                member: member.clone(),
                type_: member_type,
            }
        }
        Expression::Trapped { .. } => {
            // Inline traps are only constructed as the value of a binding,
            // assignment, or bare-expression statement, where `lower_statement`
            // desugars them directly; they never reach value lowering.
            unreachable!("inline TRAP must be lowered as a statement value")
        }
        Expression::Binary {
            left,
            operator,
            right,
            line,
            column,
        } => {
            let result_type = expression_type(expression, locals, context)
                .unwrap_or_else(|| "Unknown".to_string());
            IrValue::Binary {
                op: operator.clone(),
                left: Box::new(lower_expression(left, locals, context)),
                right: Box::new(lower_expression(right, locals, context)),
                type_: result_type,
                loc: IrSourceLoc {
                    line: *line as u32,
                    column: *column as u32,
                },
            }
        }
        Expression::Unary {
            operator,
            operand,
            line,
            column,
        } => {
            // A negated decimal literal in an exact-numeric slot (`LET a AS Money =
            // -1.25`, `LET a AS Fixed = -1.25`) must lower its operand as a const of
            // that type, so the raw negate operates on the scaled i64 rather than an
            // f64 bit pattern, and the node is annotated to match the binding
            // (plan-29-A §4.4).
            //
            // `Fixed` was originally excluded here "so their goldens are unchanged".
            // That was silent corruption, not a neutral choice: the operand stayed a
            // *Float* const, so `LET a AS Fixed = -1.25` stored the f64 bit pattern
            // of 1.25, negated, into a Q32.32 slot and read back as
            // -1074528256.0 (bug-367). Every negative `Fixed` literal was affected;
            // the positive form was always correct, which is why it went unnoticed.
            let exact_literal_negation = operator == "-"
                && matches!(expected, Some("Money") | Some("Fixed"))
                && matches!(operand.as_ref(), Expression::Number(_));
            let result_type = if exact_literal_negation {
                expected.unwrap_or("Unknown").to_string()
            } else {
                expression_type(expression, locals, context)
                    .unwrap_or_else(|| "Unknown".to_string())
            };
            let lowered_operand = if exact_literal_negation {
                lower_expression_with_expected(operand, expected, locals, context)
            } else {
                lower_expression(operand, locals, context)
            };
            // bug-07: the minimum `Fixed` (`-2147483648.0`) parses as
            // `-(2147483648.0F)`, but the positive magnitude overflows the i64
            // raw (2^63), so the constant can never materialize on its own. Fold
            // the negation into the literal here — `fixed_raw_from_decimal`
            // handles the signed string correctly. The guard is exact: it fires
            // only when the positive magnitude overflows *and* the negated value
            // fits, which is true solely at the min boundary (raw == 2^63), so
            // every in-range negated literal keeps its `Unary` shape and no
            // existing codegen/golden shifts.
            if operator == "-" {
                if let IrValue::Const { type_, value } = &lowered_operand {
                    if type_ == "Fixed"
                        && numeric::fixed_raw_from_decimal(value).is_err()
                        && numeric::fixed_raw_from_decimal(&format!("-{value}")).is_ok()
                    {
                        return IrValue::Const {
                            type_: "Fixed".to_string(),
                            value: format!("-{value}"),
                        };
                    }
                    // bug-286: the same fold for the most-negative `Integer`
                    // (`-9223372036854775808`). Syntaxcheck and `ir::verify`
                    // deliberately accept `-N` where `N == i64::MAX + 1`
                    // (spec §4.12), but without this arm the `Unary` shape
                    // survives to codegen, which materializes the u64 bit
                    // pattern and then negates it at runtime — an overflow that
                    // traps on every run. The guard is exact for the same
                    // reason the `Fixed`/`Money` guards are: it fires only when
                    // the positive magnitude does not fit an i64 *and* the
                    // negated form does, which is true solely at `i64::MIN`, so
                    // every in-range negated literal keeps its `Unary` shape.
                    if type_ == "Integer"
                        && value.parse::<i64>().is_err()
                        && format!("-{value}").parse::<i64>().is_ok()
                    {
                        return IrValue::Const {
                            type_: "Integer".to_string(),
                            value: format!("-{value}"),
                        };
                    }
                    // The same fold for the most-negative Money
                    // (`-92233720368547.75808`), whose positive magnitude
                    // overflows the i64 raw (plan-29-B §4.2).
                    if type_ == "Money"
                        && numeric::money_raw_from_decimal(value).is_err()
                        && numeric::money_raw_from_decimal(&format!("-{value}")).is_ok()
                    {
                        return IrValue::Const {
                            type_: "Money".to_string(),
                            value: format!("-{value}"),
                        };
                    }
                }
            }
            IrValue::Unary {
                op: operator.clone(),
                operand: Box::new(lowered_operand),
                type_: result_type,
                loc: IrSourceLoc {
                    line: *line as u32,
                    column: *column as u32,
                },
            }
        }
    }
}

/// Build an `ErrorLoc` record value for a compile-time source location.
fn error_loc_value(file: &str, loc: IrSourceLoc) -> IrValue {
    IrValue::Constructor {
        type_: "ErrorLoc".to_string(),
        args: vec![
            IrValue::Const {
                type_: "String".to_string(),
                value: file.to_string(),
            },
            IrValue::Const {
                type_: "Integer".to_string(),
                value: loc.line.to_string(),
            },
            IrValue::Const {
                type_: "Integer".to_string(),
                value: loc.column.to_string(),
            },
        ],
    }
}

/// Build an `Error` record value (code, message, source) for `error(...)`.
fn build_error_value(code: IrValue, message: IrValue, file: &str, loc: IrSourceLoc) -> IrValue {
    IrValue::Constructor {
        type_: "Error".to_string(),
        args: vec![code, message, error_loc_value(file, loc)],
    }
}

fn wrap_union_value(
    base: IrValue,
    expression: &Expression,
    expected: Option<&str>,
    locals: &HashMap<String, String>,
    context: &LowerContext<'_>,
) -> IrValue {
    let Some(union_type) = expected else {
        return base;
    };
    // Avoid double-wrapping when the value's own lowering already wrapped it
    // (e.g. a variant constructor assigned to a union-typed binding).
    if matches!(base, IrValue::UnionWrap { .. }) {
        return base;
    }
    let Some(actual_type) = expression_type(expression, locals, context) else {
        return base;
    };
    if context
        .type_index
        .variant_belongs_to_union(&actual_type, union_type)
    {
        return IrValue::UnionWrap {
            union_type: union_type.to_string(),
            member_type: actual_type,
            value: Box::new(base),
        };
    }
    base
}

fn lower_constructor_args(
    arguments: &[ConstructorArg],
    fields: Option<&Vec<IrField>>,
    locals: &HashMap<String, String>,
    context: &mut LowerContext<'_>,
) -> Vec<IrValue> {
    let Some(fields) = fields else {
        return arguments
            .iter()
            .map(|argument| lower_expression(constructor_arg_value(argument), locals, context))
            .collect();
    };
    if arguments
        .iter()
        .all(|argument| matches!(argument, ConstructorArg::Named { .. }))
    {
        return fields
            .iter()
            .filter_map(|field| {
                arguments.iter().find_map(|argument| match argument {
                    ConstructorArg::Named { name, value, .. } if name == &field.name => Some(
                        lower_expression_with_expected(value, Some(&field.type_), locals, context),
                    ),
                    _ => None,
                })
            })
            .collect();
    }
    arguments
        .iter()
        .enumerate()
        .map(|(index, argument)| {
            let expected = fields.get(index).map(|field| field.type_.as_str());
            lower_expression_with_expected(
                constructor_arg_value(argument),
                expected,
                locals,
                context,
            )
        })
        .collect()
}

fn constructor_arg_value(argument: &ConstructorArg) -> &Expression {
    match argument {
        ConstructorArg::Positional(value) => value,
        ConstructorArg::Named { value, .. } => value,
    }
}

fn captured_locals(
    expression: &Expression,
    outer_locals: &HashMap<String, String>,
    local_names: &HashSet<String>,
) -> Vec<CapturedLocal> {
    let mut captures = Vec::new();
    let mut seen = HashSet::new();
    collect_captured_locals(
        expression,
        outer_locals,
        local_names,
        &mut seen,
        &mut captures,
    );
    captures
}

fn collect_captured_locals(
    expression: &Expression,
    outer_locals: &HashMap<String, String>,
    local_names: &HashSet<String>,
    seen: &mut HashSet<String>,
    captures: &mut Vec<CapturedLocal>,
) {
    match expression {
        Expression::Identifier(name) => {
            if let Some(type_) = outer_locals.get(name) {
                if !local_names.contains(name) && seen.insert(name.clone()) {
                    captures.push(CapturedLocal {
                        name: name.clone(),
                        type_: type_.clone(),
                    });
                }
            }
        }
        Expression::Call {
            callee, arguments, ..
        } => {
            if let Some(type_) = outer_locals.get(callee) {
                if !local_names.contains(callee) && seen.insert(callee.clone()) {
                    captures.push(CapturedLocal {
                        name: callee.clone(),
                        type_: type_.clone(),
                    });
                }
            }
            for argument in arguments {
                collect_captured_locals(
                    call_arg_value(argument),
                    outer_locals,
                    local_names,
                    seen,
                    captures,
                );
            }
        }
        Expression::Lambda { .. } => {}
        Expression::Binary { left, right, .. } => {
            collect_captured_locals(left, outer_locals, local_names, seen, captures);
            collect_captured_locals(right, outer_locals, local_names, seen, captures);
        }
        Expression::Unary { operand, .. } => {
            collect_captured_locals(operand, outer_locals, local_names, seen, captures);
        }
        Expression::Constructor { arguments, .. } => {
            for argument in arguments {
                collect_captured_locals(
                    constructor_arg_value(argument),
                    outer_locals,
                    local_names,
                    seen,
                    captures,
                );
            }
        }
        Expression::ListLiteral(values) => {
            for value in values {
                collect_captured_locals(value, outer_locals, local_names, seen, captures);
            }
        }
        Expression::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                collect_captured_locals(key, outer_locals, local_names, seen, captures);
                collect_captured_locals(value, outer_locals, local_names, seen, captures);
            }
        }
        Expression::MemberAccess { target, .. } => {
            collect_captured_locals(target, outer_locals, local_names, seen, captures);
        }
        Expression::WithUpdate { target, updates } => {
            collect_captured_locals(target, outer_locals, local_names, seen, captures);
            for update in updates {
                collect_captured_locals(&update.value, outer_locals, local_names, seen, captures);
            }
        }
        Expression::Trapped { expression, .. } => {
            collect_captured_locals(expression, outer_locals, local_names, seen, captures);
        }
        Expression::String(_)
        | Expression::Number(_)
        | Expression::Scalar(_)
        | Expression::Boolean(_) => {}
    }
}

fn numeric_binary_result_type(operator: &str, left: &str, right: &str) -> &'static str {
    numeric::binary_result_type(operator, left, right).unwrap_or(numeric::TYPE_INTEGER)
}

fn literal_expression_type(expression: &Expression) -> Option<String> {
    match expression {
        Expression::String(_) => Some("String".to_string()),
        Expression::Number(value) => Some(
            match numeric::classify_literal(value).1 {
                numeric::LiteralType::Integer => "Integer",
                numeric::LiteralType::Float => "Float",
                numeric::LiteralType::Fixed => "Fixed",
                numeric::LiteralType::Money => "Money",
            }
            .to_string(),
        ),
        Expression::Scalar(_) => Some("Scalar".to_string()),
        Expression::Boolean(_) => Some("Boolean".to_string()),
        Expression::Identifier(value) if value == "NOTHING" => Some("Nothing".to_string()),
        _ => None,
    }
}

struct TypeIndex {
    records: HashMap<String, Vec<IrField>>,
    enums: HashMap<String, Vec<String>>,
    variants: HashMap<String, String>,
    variant_unions: HashMap<String, HashSet<String>>,
    variant_fields: HashMap<String, Vec<IrField>>,
}

impl TypeIndex {
    fn new(ast: &AstProject) -> Self {
        let mut records = HashMap::new();
        let mut enums = HashMap::new();
        let mut variants = HashMap::new();
        let mut variant_unions = HashMap::<String, HashSet<String>>::new();
        let mut variant_fields = HashMap::new();
        let union_decls = ast
            .files
            .iter()
            .flat_map(|file| &file.items)
            .filter_map(|item| {
                let Item::Type(type_decl) = item else {
                    return None;
                };
                if matches!(type_decl.kind, TypeDeclKind::Union) {
                    Some((type_decl.name.clone(), type_decl))
                } else {
                    None
                }
            })
            .collect::<HashMap<_, _>>();
        for file in &ast.files {
            for item in &file.items {
                let Item::Type(type_decl) = item else {
                    continue;
                };
                match type_decl.kind {
                    TypeDeclKind::Type => {
                        records.insert(
                            type_decl.name.clone(),
                            type_decl.fields.iter().map(lower_field).collect(),
                        );
                    }
                    TypeDeclKind::Union => {
                        for variant in
                            expanded_union_variants(type_decl, &union_decls, &mut HashSet::new())
                        {
                            variants
                                .entry(variant.name.clone())
                                .or_insert_with(|| type_decl.name.clone());
                            variant_unions
                                .entry(variant.name.clone())
                                .or_default()
                                .insert(type_decl.name.clone());
                            variant_fields.insert(
                                variant.name.clone(),
                                records.get(&variant.name).cloned().unwrap_or_default(),
                            );
                        }
                    }
                    TypeDeclKind::Enum => {
                        enums.insert(
                            type_decl.name.clone(),
                            type_decl
                                .members
                                .iter()
                                .map(|member| member.name.clone())
                                .collect(),
                        );
                    }
                }
            }
        }
        Self {
            records,
            enums,
            variants,
            variant_unions,
            variant_fields,
        }
    }

    fn constructor_result(&self, name: &str) -> Option<String> {
        if name == "Error" {
            Some("Error".to_string())
        } else if name == "Ok" {
            Some("Result OF Unknown".to_string())
        } else if self.records.contains_key(name) {
            Some(name.to_string())
        } else {
            self.variants.get(name).cloned()
        }
    }

    fn record_field_type(&self, type_name: &str, member: &str) -> Option<String> {
        if let Some(type_) = builtins::io::builtin_type_fields(type_name)
            .or_else(|| builtins::net::builtin_type_fields(type_name))
            .or_else(|| builtins::term::builtin_type_fields(type_name))
            .or_else(|| builtins::audio::builtin_type_fields(type_name))
            .and_then(|fields| fields.iter().find(|(name, _)| *name == member))
            .map(|(_, type_)| (*type_).to_string())
        {
            return Some(type_);
        }
        self.records
            .get(type_name)
            .or_else(|| self.variant_fields.get(type_name))?
            .iter()
            .find(|field| field.name == member)
            .map(|field| field.type_.clone())
    }

    fn variant_belongs_to_union(&self, variant_name: &str, union_name: &str) -> bool {
        self.variant_unions
            .get(variant_name)
            .is_some_and(|unions| unions.contains(union_name))
    }
}

fn expanded_union_variants<'a>(
    type_decl: &'a TypeDecl,
    union_decls: &HashMap<String, &'a TypeDecl>,
    visiting: &mut HashSet<String>,
) -> Vec<&'a UnionVariant> {
    // Guard against an `INCLUDES` cycle (a self- or mutually-including union):
    // without this the recursion is unbounded and overflows the native stack with
    // no diagnostic (bug-194). Insert-before/remove-after tracks only the current
    // DFS path, so a genuine cycle short-circuits while a legitimate diamond
    // include still expands each edge (preserving acyclic-union output).
    if !visiting.insert(type_decl.name.clone()) {
        return Vec::new();
    }
    let mut variants = Vec::new();
    for include in &type_decl.includes {
        if let Some(included) = union_decls.get(include) {
            variants.extend(expanded_union_variants(included, union_decls, visiting));
        }
    }
    variants.extend(type_decl.variants.iter());
    visiting.remove(&type_decl.name);
    variants
}

pub fn write_ir(project_dir: &Path, ir: &IrProject) -> Result<PathBuf, String> {
    let ir_path = project_dir.join(format!("{}.ir", ir.name));
    fs::write(&ir_path, ir.to_json())
        .map_err(|err| format!("failed to write '{}': {err}", ir_path.display()))?;
    Ok(ir_path)
}
