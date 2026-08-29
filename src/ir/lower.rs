use super::*;

use super::lower_link::{link_aliases, link_cstructs, link_functions, native_resources};
use crate::hir::{
    HirCallArg, HirConstructorArg, HirExpression, HirFunction, HirItem, HirMatchCase,
    HirMatchPattern, HirParam, HirProject, HirStatement, HirTopLevelBinding, HirTypeDecl,
    HirTypeField,
};
use crate::types::ParameterType;

/// The type environment `ir::lower`'s `expression_type` oracle consults.
///
/// plan-106-A: every type-valued entry is a [`ParameterType`]. The *keys* stay
/// `String` because they are NAMES (functions, bindings, import aliases). Before
/// plan-106-A each of these was a `HashMap<String, String>` and the engine
/// inferred over rendered spellings — the plan-102-C3 staging residue this
/// letter retires.
struct LowerContext<'a> {
    function_returns: &'a HashMap<String, ParameterType>,
    function_types: &'a HashMap<String, ParameterType>,
    function_params: &'a HashMap<String, Vec<CallParam>>,
    binding_types: HashMap<String, ParameterType>,
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
    current_return_type: Option<ParameterType>,
    /// Stack of inline-`TRAP` recover destinations (innermost last). Each entry
    /// is the local slot a `RECOVER` value should be stored into and its type,
    /// or `None` when the trapped value is discarded (bare-statement form).
    recover_targets: Vec<RecoverTarget>,
    /// Names of `MUT` local bindings in scope. A lambda in a non-escaping
    /// callback position captures these by slot reference rather than by value.
    /// Not scope-precise — only ever consulted
    /// for capture classification, where a stale non-`MUT` entry is impossible
    /// (only `MUT` binds are inserted) and a slot reference is memory-safe regardless.
    mutable_locals: HashSet<String>,
    /// Set true only while lowering the argument in a compiler-known
    /// non-escaping callback position (e.g. `forEach`'s action). The lambda
    /// lowering consumes it to license `MUT` slot-reference captures.
    nonescaping_callback: bool,
    /// Source location of the statement (or match case / declaration) currently
    /// being lowered. Stamped onto every `IrOp` so relocated diagnostics report
    /// at the same line the AST checker did (plan-20-A). Column is always 1,
    /// matching `show_diagnostic`'s statement-level reporting.
    current_loc: IrSourceLoc,
}

/// A type an imported (non-builtin) package exports, decoded from its `.mfp` so
/// IR lowering can *type* accesses to its fields. Without this, a consumer names
/// an imported type but has no field/variant layout for it, so every
/// `record.field` on an imported record or union variant lowers to `Unknown` —
/// tolerated by `getOr`/`len` but not by `collections::keys`/`values`, which need
/// the element type. Built-in packages need no entry here: their source is folded
/// into the AST by `augmented_project`, so their types are already in `TypeIndex`.
#[derive(Clone)]
pub struct ImportedTypeDef {
    pub name: String,
    pub kind: ImportedTypeKind,
    /// Record fields (for `ImportedTypeKind::Record`).
    pub fields: Vec<ImportedTypeField>,
    /// Union variants with their fields (for `ImportedTypeKind::Union`).
    pub variants: Vec<ImportedTypeVariant>,
    /// Enum member names (for `ImportedTypeKind::Enum`).
    pub members: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ImportedTypeKind {
    Record,
    Union,
    Enum,
}

#[derive(Clone)]
pub struct ImportedTypeField {
    pub name: String,
    pub type_: String,
}

#[derive(Clone)]
pub struct ImportedTypeVariant {
    pub name: String,
    pub fields: Vec<ImportedTypeField>,
}

/// Augmenting wrapper used by the in-process tests, which pass a bare (un-injected)
/// AST: it runs the builtin-source augmentation then lowers. The build path injects
/// the sources before monomorphization (`resolver::augment_project`) and lowers the
/// already-augmented AST with [`lower_augmented_project`] directly.
#[cfg(test)]
pub fn lower_project_with_external_functions(
    ast: &crate::ast::AstProject,
    entry: Option<EntryPoint>,
    external_signatures: &HashMap<String, ExternalSignature>,
    imported_types: &[ImportedTypeDef],
) -> IrProject {
    let augmented = crate::codegen::registry::registry()
        .augment_project(ast)
        .expect("clean-room registry package source must parse");

    // `term`'s source companion (`package.mfb` — the `LineStyle`/`FillStyle` enums)
    // and the `term`↔`astrings` `drawText(AttributedString)` bridge are injected by
    // the clean-room `registry::augment_project` above (the companion as an `Always`
    // helper on the migrated `term` package, the bridge as a `WhenImported("astrings")`
    // gated helper).
    // `astrings`' source companion (`package.mfb`) is injected by the clean-room
    // `registry::augment_project` above (plan-99 PART C), as an `Always` helper on
    // the migrated `astrings` package — emitted whenever a program `IMPORT astrings`.
    // app + datetime + money source is injected by the clean-room
    // `registry::augment_project` above.
    // `vector` source (its nine `TYPE`s + `__vector_*` FUNC bodies) is injected by the
    // clean-room `registry::augment_project` above.
    // `http` before `net`: `http_package.mfb` imports `net`, so net's late pass must
    // run after http's to see the transitive `IMPORT net` (plan-03-http.md Phase 4).
    let augmented = crate::codegen::builtins::http::augmented_project(&augmented)
        .expect("built-in http package source must parse");
    let augmented = crate::codegen::builtins::net::augmented_project(&augmented)
        .expect("built-in net package source must parse");
    // `audio` (its `render`/`play` synthesis companion + `AudioDevice`/`AudioEnvelope`/
    // `AudioNote` records) is injected by the generic clean-room
    // `registry::augment_project` above.
    // `process` (its `Stream`/`Signal` enum companion) is injected by the generic
    // clean-room `registry::augment_project` above.
    // `crypto` source is injected by the clean-room `registry::augment_project` above
    // (before the `strings`/`encoding` late passes, so `encoding::uses_package` still
    // sees `crypto_package.mfb`'s `IMPORT encoding`).
    // `strings`' scalar-seam companion (which `IMPORT encoding`s, plan-41-D) is
    // injected by the clean-room `registry::augment_project` above (plan-99 PART B),
    // as a `WhenUsed` gated helper — before this `encoding` late pass, so
    // `encoding::uses_package` still sees the seam's transitive `IMPORT encoding`.
    let augmented = crate::codegen::builtins::encoding::augmented_project(&augmented)
        .expect("built-in encoding package source must parse");
    let mut ir = lower_augmented_project(
        &crate::hir::elaborate(&augmented),
        entry,
        external_signatures,
        imported_types,
    );
    // Docs come from the source AST this wrapper owns (the lowering path holds
    // only HIR); the build's package path likewise collects from its original AST.
    ir.docs = collect_project_docs(&augmented);
    ir
}

/// Lower an already-monomorphized project, for the in-process tests.
///
/// The tests monomorphize a BARE (un-injected) project, so the builtin package
/// sources are injected here — the same chain the AST wrapper above runs, in the
/// HIR domain.
///
/// `ir.docs` comes from the ORIGINAL source AST, not the post-monomorph program.
/// That matches the build path (whose package path "likewise collects from its
/// original AST") and is the only thing that can be right: monomorphization
/// renames overloaded and generic declarations, so a `DOC` header can no longer
/// find the declaration it documents. Before plan-106-D this read a
/// de-elaborated post-monomorph AST.
#[cfg(test)]
pub fn lower_monomorphized_project(
    concrete: &crate::hir::HirProject,
    source: &crate::ast::AstProject,
    entry: Option<EntryPoint>,
    external_signatures: &HashMap<String, ExternalSignature>,
    imported_types: &[ImportedTypeDef],
) -> IrProject {
    let augmented =
        crate::resolver::augment_hir_project(concrete).expect("built-in package source must parse");
    let mut ir = lower_augmented_project(&augmented, entry, external_signatures, imported_types);
    ir.docs = collect_project_docs(
        &crate::resolver::augment_project(source).expect("built-in package source must parse"),
    );
    ir
}

/// Lower an already-augmented project (builtin package sources already injected by
/// `resolver::augment_project`, before monomorphization) to IR. The build path
/// calls this directly on the post-monomorph AST; [`lower_project_with_external_
/// functions`] is the augmenting wrapper the in-process tests use on a bare AST.
pub fn lower_augmented_project(
    hir: &crate::hir::HirProject,
    entry: Option<EntryPoint>,
    external_signatures: &HashMap<String, ExternalSignature>,
    imported_types: &[ImportedTypeDef],
) -> IrProject {
    let mut types = Vec::new();
    let mut functions = Vec::new();
    let mut function_returns = function_returns(hir);
    let mut function_types = function_types(hir);
    let mut function_params = function_params(hir);
    let binding_types = declared_binding_types(hir);
    // Imported-package signatures arrive TYPED (plan-105-A): the return type and
    // the parameter list are read straight off `ExternalSignature` instead of being
    // re-split out of a formatted `FUNC(…) AS R` string. plan-106-A closed the last
    // gap here — the lowering context's own maps are `ParameterType` now, so these
    // three entries are clones rather than `name()` renders.
    for (name, signature) in external_signatures {
        function_types.insert(name.clone(), signature.signature_type());
        function_params.insert(
            name.clone(),
            signature
                .params
                .iter()
                .map(|param| CallParam {
                    name: param.name.clone(),
                    type_: param.type_.clone(),
                    default: None,
                })
                .collect(),
        );
        function_returns.insert(name.clone(), signature.returns.clone());
    }
    let type_index = TypeIndex::new(hir, imported_types);
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
    infer_binding_types(hir, &mut context);
    let bindings = lower_bindings(hir, &mut context);
    context.bindings = bindings.clone();

    for file in &hir.files {
        context.current_imports = file.import_bindings();
        context.current_file = file.path.clone();
        for item in &file.items {
            match item {
                HirItem::Binding(_) => {}
                HirItem::Function(function) => {
                    functions.push(lower_function(function, &mut context))
                }
                HirItem::Type(type_decl) => {
                    types.push(lower_type(type_decl, &type_index, &context.current_file))
                }
                // Native LINK resource declarations and re-export aliases carry no
                // executable body. The LINK block's native functions are surfaced
                // to package metadata separately (plan-link-update.md §10); they
                // are not lowered to ordinary IR functions here.
                HirItem::Resource(_) | HirItem::FuncAlias(_) | HirItem::Link(_) => {}
                // DOC blocks carry no executable body; documentation is collected
                // separately into the project's doc table.
                HirItem::Doc(_) => {}
                // TESTING blocks are lowered away before IR lowering (plan-18-A §3).
                HirItem::Testing(_) => {}
            }
        }
    }
    functions.extend(context.lambdas);

    IrProject {
        name: hir.name.clone(),
        entry,
        bindings,
        types,
        functions,
        native_resources: native_resources(hir),
        link_functions: link_functions(hir),
        link_cstructs: link_cstructs(hir),
        link_aliases: link_aliases(hir),
        // Documentation is collected from the PRE-monomorphization source AST by
        // whoever owns one — the package build path (`cli/build/mod.rs`, which
        // overwrites this with the original declaration names monomorph renames
        // away, plan-09-doc.md §5) and the in-process test wrapper
        // (`lower_project_with_external_functions`). Executables ignore it. The
        // lowering path itself holds only HIR and does not render one back.
        docs: ProjectDocs::default(),
        // Assembled from the manifest by the build path (plan-46-B §4.3), which
        // is where project.json is read; the AST carries no manifest data.
        native_libraries: crate::binary_repr::NativeLibraryTable::default(),
        // Overwritten from project.json by `assemble_max_buffer` on the build
        // path; this default is what a synthesized or test-built project gets.
        max_buffer_bytes: crate::manifest::DEFAULT_MAX_BUFFER_MIB * 1024 * 1024,
    }
}

fn lower_type(type_decl: &HirTypeDecl, type_index: &TypeIndex, file: &str) -> IrType {
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

fn lower_binding(binding: &HirTopLevelBinding, context: &mut LowerContext<'_>) -> IrBinding {
    let loc = IrSourceLoc {
        line: binding.line as u32,
        column: 1,
    };
    context.current_loc = loc;
    let locals = context.binding_types.clone();
    let type_ = if binding.explicit_type {
        binding.type_.clone()
    } else {
        binding
            .value
            .as_ref()
            .and_then(|value| expression_type(value, &locals, context))
            .unwrap_or(ParameterType::Unknown)
    };
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
        explicit_type: binding.explicit_type,
    }
}

fn lower_bindings(hir: &HirProject, context: &mut LowerContext<'_>) -> Vec<IrBinding> {
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

fn lower_field(field: &HirTypeField) -> IrField {
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
///
/// plan-106-A: the `STATE` fold is [`ParameterType::with_state`], the structural
/// equivalent of parsing the concatenated spelling (guarded by
/// `with_state_matches_parse_of_the_concatenated_spelling` in `src/types.rs`), so
/// the clause is attached without a render→parse round trip.
fn function_return_type(function: &HirFunction) -> ParameterType {
    match function.kind {
        FunctionKind::Func => match (&function.return_state_type, function.return_resource) {
            (Some(state), true) => function.returns.with_state(state),
            _ => function.returns.clone(),
        },
        FunctionKind::Sub => ParameterType::Nothing,
    }
}

fn lower_function(function: &HirFunction, context: &mut LowerContext<'_>) -> IrFunction {
    let kind = match function.kind {
        FunctionKind::Func => "func",
        FunctionKind::Sub => "sub",
    };
    let returns = function_return_type(function);
    let mut locals = HashMap::new();
    for param in &function.params {
        // Carry a `RES` parameter's `STATE T` in the local's type so `s.state`
        // resolves inside the callee, matching `lower_param`.
        let type_ = match &param.state_type {
            Some(state) => param.type_.with_state(state),
            None => param.type_.clone(),
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
        resource_owners: crate::ir::resource_escape::analyze_function(function)
            .owners()
            .clone(),
    }
}

fn lower_function_body(
    function: &HirFunction,
    locals: &HashMap<String, ParameterType>,
    context: &mut LowerContext<'_>,
) -> Vec<IrOp> {
    let mut body = lower_statement_block(&function.body, locals, context, None);
    if let Some(trap) = &function.trap {
        let mut trap_locals = locals.clone();
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
    param: &HirParam,
    locals: &HashMap<String, ParameterType>,
    context: &mut LowerContext<'_>,
) -> IrParam {
    // A `RES` parameter's `STATE T` rides inside its type's nominal spelling so
    // the callee can address the pointed-to resource's shared state payload.
    let type_ = match &param.state_type {
        Some(state) => param.type_.with_state(state),
        None => param.type_.clone(),
    };
    IrParam {
        name: param.name.clone(),
        type_: type_.clone(),
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
fn statement_loc(statement: &HirStatement) -> IrSourceLoc {
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

#[derive(Clone)]
struct RecoverTarget {
    slot: Option<String>,
    type_: ParameterType,
}

#[derive(Clone)]
struct CallParam {
    name: String,
    type_: ParameterType,
    default: Option<HirExpression>,
}

#[derive(Clone)]
struct CapturedLocal {
    name: String,
    type_: ParameterType,
}

fn lower_statement(
    statement: &HirStatement,
    locals: &mut HashMap<String, ParameterType>,
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
        HirStatement::Let {
            mutable,
            name,
            type_,
            explicit_type,
            value,
            state_type,
            ..
        } => {
            // The explicit `AS T` annotation as the AST's `Option<String>`
            // (`None` when unannotated), reconstructed byte-exact from the HIR
            // `type_`/`explicit_type` pair.
            let declared_type: Option<ParameterType> = explicit_type.then(|| type_.clone());
            if let Some(HirExpression::Trapped {
                expression,
                binding,
                handler,
                ..
            }) = value
            {
                let success_type = declared_type
                    .clone()
                    .or_else(|| expression_type(expression, locals, context))
                    .unwrap_or(ParameterType::Unknown);
                // A `RES` binding's `STATE T` rides in the lowered type exactly as
                // on the non-trap path below. `expression_type` already carries it,
                // so only an explicit `AS T` needs it reattached — without this,
                // writing the annotation *caused* the TYPE_STATE_MISMATCH that
                // omitting it avoided (bug-372).
                let success_type = match (&declared_type, state_type) {
                    (Some(declared_type), Some(state)) => declared_type.with_state(state),
                    _ => success_type,
                };
                return lower_inline_trap(
                    expression,
                    binding,
                    handler,
                    InlineTrapTarget::Bind {
                        mutable: *mutable,
                        name: name.clone(),
                        type_: success_type,
                        explicit_type: *explicit_type,
                    },
                    locals,
                    context,
                );
            }
            let lowered_type = declared_type.clone().unwrap_or_else(|| {
                value
                    .as_ref()
                    .and_then(|value| expression_type(value, locals, context))
                    .unwrap_or(ParameterType::Unknown)
            });
            let lowered_value = value.as_ref().map(|value| {
                let base =
                    lower_expression_with_expected(value, Some(&lowered_type), locals, context);
                // Wrap a resource (or data) variant value into its union when the
                // binding is union-typed, so a `RES s AS Stream = <a File>` carries
                // the variant tag for tag-dispatched drop.
                wrap_union_value(base, value, Some(&lowered_type), locals, context)
            });
            // A `RES` binding's `STATE T` rides in the lowered type
            // (`File STATE T`) so codegen can default-initialize and address the
            // state payload; the bare resource name is recovered for recognition.
            let lowered_type = match state_type {
                Some(state) => lowered_type.with_state(state),
                None => lowered_type,
            };
            locals.insert(name.clone(), lowered_type.clone());
            // Track `MUT` bindings so a non-escaping callback can capture them by
            // slot rather than copy them by value.
            if *mutable {
                context.mutable_locals.insert(name.clone());
            }
            vec![IrOp::Bind {
                mutable: *mutable,
                name: name.clone(),
                type_: lowered_type,
                value: lowered_value,
                explicit_type: *explicit_type,
                loc,
            }]
        }
        HirStatement::Return { value, .. } => vec![IrOp::Return {
            value: value.as_ref().map(|value| {
                // Coerce a bare numeric literal to the declared return type,
                // exactly as `LET`/constructor-arg lowering does — otherwise an
                // unsuffixed literal returned from a `Fixed`/`Money`/`Float`
                // function is classified as `Integer` and its raw bits are
                // reinterpreted as the destination type (bug-156).
                let expected = context.current_return_type.clone();
                let base =
                    lower_expression_with_expected(value, expected.as_ref(), locals, context);
                // Implicitly wrap a returned member constructor into the
                // function's declared union return type, so the wrap is explicit
                // in the IR (and faithfully serialized into Binary Representation) rather
                // than re-derived during native codegen.
                wrap_union_value(base, value, expected.as_ref(), locals, context)
            }),
            loc,
        }],
        HirStatement::Exit { target, code, .. } => match target {
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
        HirStatement::Continue { kind, .. } => vec![IrOp::ContinueLoop { kind: *kind, loc }],
        HirStatement::Fail { error, .. } => vec![IrOp::Fail {
            error: lower_expression(error, locals, context),
            loc,
        }],
        HirStatement::Propagate { .. } => vec![IrOp::Fail {
            // Typecheck rejects PROPAGATE outside a trap body; total lowering
            // (plan-20-D) stamps a sentinel error local when the guard is
            // absent so it never panics on ill-typed input.
            error: IrValue::Local(trap_name.unwrap_or("$error").to_string()),
            loc,
        }],
        HirStatement::Recover { value, .. } => {
            // Typecheck rejects RECOVER outside an inline-TRAP handler
            // (TYPE_RECOVER_OUTSIDE_INLINE_TRAP); total lowering binds the stray
            // value to a `$recover_stray` temp rather than panicking. The temp —
            // not a discard `Eval` — because the statement's one surviving fact
            // must stay readable: the front end's flow analysis treats ANY
            // RECOVER as diverging, and `ir::verify`'s divergence rules
            // (TYPE_TRAP_FALLTHROUGH, TYPE_FUNC_MISSING_RETURN) key on the
            // temp's name to agree. Never reaches codegen: the program is
            // rejected.
            let Some(target) = context.recover_targets.last().cloned() else {
                let name = make_temp_local_name(context, "recover_stray");
                let value = value
                    .as_ref()
                    .map(|value| lower_expression(value, locals, context));
                return vec![IrOp::Bind {
                    mutable: false,
                    name,
                    type_: ParameterType::Unknown,
                    value,
                    explicit_type: false,
                    loc,
                }];
            };
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
        HirStatement::Assign { name, value, .. } => {
            if let HirExpression::Trapped {
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
            let lowered = lower_expression_with_expected(value, expected.as_ref(), locals, context);
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
        HirStatement::StateAssign {
            resource, value, ..
        } => {
            let resource_type = locals
                .get(resource)
                .or_else(|| context.binding_types.get(resource))
                .cloned();
            // plan-106-C: `ParameterType::state` splits the ` STATE T` clause out
            // structurally, so this no longer renders the type to read it back.
            let state_type = resource_type.as_ref().and_then(ParameterType::state);
            let lowered =
                lower_expression_with_expected(value, state_type.as_ref(), locals, context);
            vec![IrOp::StateAssign {
                resource: resource.clone(),
                value: lowered,
                loc,
            }]
        }
        HirStatement::Expression { expression, .. } => {
            // Assertion builtins (plan-18-B) desugar to ordinary statements —
            // comparisons + FAIL, or a trap-guarded evaluation — which are then
            // lowered through the normal path. Doing it here (post-typecheck)
            // sidesteps the source-level RECOVER-typing constraint on a
            // value-producing trapped expression.
            if let HirExpression::Call {
                callee,
                arguments,
                line: call_line,
                ..
            } = expression
            {
                if crate::codegen::builtins_testing::is_testing_call(callee) {
                    let uid = context.next_temp_id;
                    context.next_temp_id += 1;
                    let expanded =
                        crate::testing::expand_expect(callee, arguments, uid, *call_line);
                    return lower_statement_block(&expanded, locals, context, trap_name);
                }
            }
            if let HirExpression::Trapped {
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
        HirStatement::If {
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
        HirStatement::Match {
            expression, cases, ..
        } => {
            let matched_type = match_expression_type(expression, locals, context)
                .unwrap_or(ParameterType::Unknown);
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
            let match_value = if matches!(match_locals[&matched_name], ParameterType::ResultOf(_)) {
                let match_flag_name = make_temp_local_name(context, "match_ok");
                ops.push(IrOp::Bind {
                    mutable: false,
                    name: match_flag_name.clone(),
                    type_: ParameterType::Boolean,
                    value: Some(IrValue::ResultIsOk {
                        value: Box::new(IrValue::Local(matched_name.clone())),
                    }),
                    loc,
                    explicit_type: false,
                });
                match_locals.insert(match_flag_name.clone(), ParameterType::Boolean);
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
        HirStatement::For {
            name,
            start,
            end,
            step,
            body,
            line,
        } => {
            let start_type =
                expression_type(start, locals, context).unwrap_or(ParameterType::Unknown);
            let end_type = expression_type(end, locals, context).unwrap_or(ParameterType::Unknown);
            let step_type = step
                .as_ref()
                .and_then(|value| expression_type(value, locals, context))
                .unwrap_or(ParameterType::Integer);
            let loop_type =
                numeric::typed_promote_loop_numeric_type(&start_type, &end_type, &step_type);
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
        HirStatement::ForEach {
            name,
            iterable,
            body,
            ..
        } => {
            let iterable_type =
                expression_type(iterable, locals, context).unwrap_or(ParameterType::Unknown);
            let element_type =
                collection_iteration_type(&iterable_type).unwrap_or(ParameterType::Unknown);
            let mut nested = locals.clone();
            nested.insert(name.clone(), element_type.clone());
            vec![IrOp::ForEach {
                name: name.clone(),
                type_: element_type.clone(),
                iterable: lower_expression(iterable, locals, context),
                body: lower_statement_block(body, &nested, context, trap_name),
                loc,
            }]
        }
        HirStatement::While {
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
        HirStatement::DoUntil {
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
    body: &[HirStatement],
    locals: &HashMap<String, ParameterType>,
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
        type_: ParameterType,
        explicit_type: bool,
    },
    /// `name = <call> TRAP(e) …`
    Assign { name: String },
    /// `<call> TRAP(e) …` as a bare statement (value discarded).
    Discard,
}

/// Whether any op in `ops` (recursively, including nested control-flow bodies)
/// reads the local `name`. plan-64-I: an inline-`TRAP` handler that never reads
/// its bound error `binding` needs no `Bind err = ResultError`, so eliding it
/// drops a dead `Error` deep-copy (`ResultError` is an aliasing source, so the
/// bind lowers to `copy_flat_block`) on every err-ignoring `TRAP`. Conservative:
/// a shadowing rebind of the same name that is read only keeps the (correct)
/// full error assembly.
fn ops_read_local(ops: &[IrOp], name: &str) -> bool {
    fn value_reads(value: &IrValue, name: &str) -> bool {
        let mut found = false;
        crate::ir::value::visit_value(value, &mut |v| {
            if let IrValue::Local(local) = v {
                if local == name {
                    found = true;
                }
            }
        });
        found
    }
    ops.iter().any(|op| match op {
        IrOp::Bind {
            value: Some(value), ..
        }
        | IrOp::Assign { value, .. }
        | IrOp::AssignGlobal { value, .. }
        | IrOp::StateAssign { value, .. }
        | IrOp::ExitProgram { code: value, .. }
        | IrOp::Fail { error: value, .. }
        | IrOp::Eval { value, .. } => value_reads(value, name),
        IrOp::Bind { value: None, .. } | IrOp::ExitLoop { .. } | IrOp::ContinueLoop { .. } => false,
        IrOp::Return { value, .. } => value.as_ref().is_some_and(|v| value_reads(v, name)),
        IrOp::If {
            condition,
            then_body,
            else_body,
            ..
        } => {
            value_reads(condition, name)
                || ops_read_local(then_body, name)
                || ops_read_local(else_body, name)
        }
        IrOp::Match { value, cases, .. } => {
            value_reads(value, name)
                || cases.iter().any(|case| {
                    case.guard.as_ref().is_some_and(|g| value_reads(g, name))
                        || ops_read_local(&case.body, name)
                })
        }
        IrOp::While {
            condition, body, ..
        } => value_reads(condition, name) || ops_read_local(body, name),
        IrOp::For {
            start,
            end,
            step,
            body,
            ..
        } => {
            value_reads(start, name)
                || value_reads(end, name)
                || value_reads(step, name)
                || ops_read_local(body, name)
        }
        IrOp::DoUntil {
            body, condition, ..
        } => ops_read_local(body, name) || value_reads(condition, name),
        IrOp::ForEach { iterable, body, .. } => {
            value_reads(iterable, name) || ops_read_local(body, name)
        }
        IrOp::Trap { body, .. } => ops_read_local(body, name),
    })
}

/// Lowers an inline `TRAP` to existing IR primitives (no backend support is
/// required). The trapped call is evaluated as a raw `Result`; on `Ok` its value
/// flows to the target; on `Err` the handler runs with `e` bound. `RECOVER`
/// stores its value into a shared slot and then falls through to the delivery of
/// the target, while diverging handler paths (`RETURN`/`FAIL`/`PROPAGATE`) leave
/// as usual. The handler is normalized so that statements following a `RECOVER`
/// in a branch do not execute after recovery (see [`treeify_handler`]).
fn lower_inline_trap(
    inner: &HirExpression,
    binding: &str,
    handler: &[HirStatement],
    target: InlineTrapTarget,
    locals: &mut HashMap<String, ParameterType>,
    context: &mut LowerContext<'_>,
) -> Vec<IrOp> {
    // The inline-TRAP statement's span: the handler block below re-sets
    // `context.current_loc` per handler statement, so ops synthesized after it
    // must use this captured copy.
    let stmt_loc = context.current_loc;
    let success_type = expression_type(inner, locals, context).unwrap_or(ParameterType::Unknown);
    let result_type = ParameterType::result_of(success_type.clone());
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
    handler_locals.insert(binding.to_string(), ParameterType::named("Error"));
    context.recover_targets.push(RecoverTarget {
        slot: slot.clone(),
        type_: success_type.clone(),
    });
    let normalized = treeify_handler(handler);
    let handler_ops = lower_statement_block(&normalized, &handler_locals, context, Some(binding));
    context.recover_targets.pop();
    // plan-64-I: only bind the error `err` when the handler actually reads it.
    // `ResultError` is an aliasing source, so `Bind err = ResultError(res)`
    // lowers to a deep `copy_flat_block` of the `Error`; when `RECOVER` ignores
    // `err`, that copy (and, for a conversion `CallResult` receiver, the whole
    // ErrorLoc/Error assembly it forces — see `emit_error_register_return`'s
    // discard path) is dead work. Eliding it is what lets the conversion error
    // path materialize only a bare tag.
    let mut else_body = Vec::new();
    if ops_read_local(&handler_ops, binding) {
        else_body.push(IrOp::Bind {
            mutable: false,
            name: binding.to_string(),
            type_: ParameterType::named("Error"),
            value: Some(IrValue::ResultError {
                value: Box::new(IrValue::Local(res_name.clone())),
            }),
            loc: stmt_loc,
            explicit_type: false,
        });
    }
    else_body.extend(handler_ops);

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
fn treeify_handler(stmts: &[HirStatement]) -> Vec<HirStatement> {
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
        HirStatement::If {
            condition,
            then_body,
            else_body,
            line,
        } => {
            // The continuation only has to be pushed *inside* a branch when that
            // branch can reach a terminator (`RECOVER`/`RETURN`/…), so a
            // recovered path does not fall through into the shared continuation.
            // When *neither* branch can terminate, the `IF` always falls through
            // to the continuation regardless of which branch runs, so keep the
            // continuation as a single shared sibling instead of cloning it into
            // both branches. Cloning into both is what makes N sequential
            // fall-through branches emit 2^N copies of the tail (bug-401).
            if !block_can_terminate(then_body) && !block_can_terminate(else_body) {
                let mut result = vec![HirStatement::If {
                    condition: condition.clone(),
                    then_body: treeify_handler(then_body),
                    else_body: treeify_handler(else_body),
                    line: *line,
                }];
                result.extend(treeify_handler(tail));
                result
            } else {
                let then_body = distribute_continuation(then_body, tail);
                let else_body = distribute_continuation(else_body, tail);
                vec![HirStatement::If {
                    condition: condition.clone(),
                    then_body,
                    else_body,
                    line: *line,
                }]
            }
        }
        HirStatement::Match {
            expression,
            cases,
            line,
        } => {
            // As with `IF` above: only distribute the continuation into the arms
            // when some arm can terminate. Otherwise the match always falls
            // through (a matched non-terminating arm or an unmatched scrutinee
            // alike), so keep the continuation as a shared sibling — no per-arm
            // clone and no synthesized `ELSE` (bug-401).
            if !cases.iter().any(|case| block_can_terminate(&case.body)) {
                let new_cases: Vec<HirMatchCase> = cases
                    .iter()
                    .map(|case| HirMatchCase {
                        pattern: case.pattern.clone(),
                        guard: case.guard.clone(),
                        body: treeify_handler(&case.body),
                        line: case.line,
                    })
                    .collect();
                let mut result = vec![HirStatement::Match {
                    expression: expression.clone(),
                    cases: new_cases,
                    line: *line,
                }];
                result.extend(treeify_handler(tail));
                return result;
            }
            let mut new_cases: Vec<HirMatchCase> = cases
                .iter()
                .map(|case| HirMatchCase {
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
                .any(|case| matches!(case.pattern, HirMatchPattern::Else) && case.guard.is_none());
            if !has_else {
                new_cases.push(HirMatchCase {
                    pattern: HirMatchPattern::Else,
                    guard: None,
                    body: treeify_handler(tail),
                    line: *line,
                });
            }
            vec![HirStatement::Match {
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
fn distribute_continuation(
    body: &[HirStatement],
    continuation: &[HirStatement],
) -> Vec<HirStatement> {
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
fn treeify_statement(statement: &HirStatement) -> HirStatement {
    match statement {
        HirStatement::If {
            condition,
            then_body,
            else_body,
            line,
        } => HirStatement::If {
            condition: condition.clone(),
            then_body: treeify_handler(then_body),
            else_body: treeify_handler(else_body),
            line: *line,
        },
        HirStatement::Match {
            expression,
            cases,
            line,
        } => HirStatement::Match {
            expression: expression.clone(),
            cases: cases
                .iter()
                .map(|case| HirMatchCase {
                    pattern: case.pattern.clone(),
                    guard: case.guard.clone(),
                    body: treeify_handler(&case.body),
                    line: case.line,
                })
                .collect(),
            line: *line,
        },
        HirStatement::While {
            kind,
            condition,
            body,
            line,
        } => HirStatement::While {
            kind: *kind,
            condition: condition.clone(),
            body: treeify_handler(body),
            line: *line,
        },
        HirStatement::DoUntil {
            body,
            condition,
            line,
        } => HirStatement::DoUntil {
            body: treeify_handler(body),
            condition: condition.clone(),
            line: *line,
        },
        HirStatement::For {
            name,
            start,
            end,
            step,
            body,
            line,
        } => HirStatement::For {
            name: name.clone(),
            start: start.clone(),
            end: end.clone(),
            step: step.clone(),
            body: treeify_handler(body),
            line: *line,
        },
        HirStatement::ForEach {
            name,
            iterable,
            body,
            line,
        } => HirStatement::ForEach {
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
fn block_terminates(stmts: &[HirStatement]) -> bool {
    stmts.iter().any(statement_terminates)
}

/// Whether *some* path through the block can reach a terminator. Unlike
/// [`block_terminates`] (which requires *every* path to terminate), this is true
/// if a terminator is reachable on any path. `treeify_handler` uses it to decide
/// whether an inline-`TRAP` handler continuation must be distributed into a
/// branch (because a recovered path must not fall into it) or may be shared as a
/// single sibling after the branch (bug-401).
fn block_can_terminate(stmts: &[HirStatement]) -> bool {
    stmts.iter().any(statement_can_terminate)
}

/// Whether a statement can reach a terminator on some path. See
/// [`block_can_terminate`]. Being conservative (returning `true` when unsure)
/// only forces distribution, which is always semantically safe; a spurious
/// `false` would be a bug, so every terminator form and every nested block is
/// covered.
fn statement_can_terminate(statement: &HirStatement) -> bool {
    match statement {
        HirStatement::Return { .. }
        | HirStatement::Exit { .. }
        | HirStatement::Continue { .. }
        | HirStatement::Fail { .. }
        | HirStatement::Propagate { .. }
        | HirStatement::Recover { .. } => true,
        HirStatement::If {
            then_body,
            else_body,
            ..
        } => block_can_terminate(then_body) || block_can_terminate(else_body),
        HirStatement::Match { cases, .. } => {
            cases.iter().any(|case| block_can_terminate(&case.body))
        }
        HirStatement::For { body, .. }
        | HirStatement::ForEach { body, .. }
        | HirStatement::While { body, .. }
        | HirStatement::DoUntil { body, .. } => block_can_terminate(body),
        _ => false,
    }
}

/// Whether a statement always diverges or recovers (ends its enclosing handler
/// path). Mirrors the syntaxcheck flow analysis for the constructs an inline-trap
/// handler may contain.
fn statement_terminates(statement: &HirStatement) -> bool {
    match statement {
        HirStatement::Return { .. }
        | HirStatement::Exit { .. }
        | HirStatement::Continue { .. }
        | HirStatement::Fail { .. }
        | HirStatement::Propagate { .. }
        | HirStatement::Recover { .. } => true,
        HirStatement::If {
            then_body,
            else_body,
            ..
        } => !else_body.is_empty() && block_terminates(then_body) && block_terminates(else_body),
        HirStatement::Match { cases, .. } => {
            let has_else = cases
                .iter()
                .any(|case| matches!(case.pattern, HirMatchPattern::Else) && case.guard.is_none());
            has_else && !cases.is_empty() && cases.iter().all(|case| block_terminates(&case.body))
        }
        _ => false,
    }
}

/// The loop-variable type `FOR EACH x IN <collection>` binds, or `None` when the
/// value is not iterable.
///
/// plan-106-A: a structural match on the collection's [`ParameterType`], replacing
/// the `strip_prefix("List OF ")` / `strip_prefix("Set OF ")` / `parse_map_type`
/// cascade this grew from — `ir::lower` holds no copy of the type grammar.
fn collection_iteration_type(type_: &ParameterType) -> Option<ParameterType> {
    match type_ {
        // Iterating `List OF RES File` yields a pointer to each element; the loop
        // variable's type is the bare resource (`File`), not `RES File` (§15.6).
        ParameterType::ListOf(element) => Some(strip_res(element)),
        // `FOR EACH x IN set` yields the element type `T` (plan-63); a Set element
        // is always comparable, so it never carries a `RES` marker.
        ParameterType::SetOf(element) => Some((**element).clone()),
        ParameterType::MapOf(key, value) => Some(ParameterType::map_entry_of(
            (**key).clone(),
            strip_res(value),
        )),
        _ => None,
    }
}

/// A collection element with its `RES ` ownership marker removed, if it carries
/// one — the structural form of the `strip_prefix("RES ")` this replaced.
fn strip_res(type_: &ParameterType) -> ParameterType {
    match type_ {
        ParameterType::Res(inner) => (**inner).clone(),
        other => other.clone(),
    }
}

/// The `AttributedString` type (plan-89-D's attributed-text nominal).
///
/// Deliberately a NOMINAL and not [`ParameterType::AttributeString`]: that
/// variant renders `"AttributeString"` — no `d` — which is a different spelling
/// the language's attributed-text type never uses, so `parse("AttributedString")`
/// yields a `Named` and every structural comparison must build the same thing.
/// A function rather than a `const` because [`ParameterType::named`] interns.
fn attributed_string_type() -> ParameterType {
    ParameterType::named("AttributedString")
}

fn make_temp_local_name(context: &mut LowerContext<'_>, prefix: &str) -> String {
    let name = format!("${prefix}{}", context.next_temp_id);
    context.next_temp_id += 1;
    name
}

fn numeric_constant_for_type(type_: &ParameterType, value: &str) -> IrValue {
    IrValue::Const {
        type_: type_.clone(),
        value: value.to_string(),
    }
}

fn lower_match_case(
    case: &HirMatchCase,
    matched_local: &str,
    locals: &HashMap<String, ParameterType>,
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
        .unwrap_or(ParameterType::Unknown);
    let pattern = match &case.pattern {
        HirMatchPattern::Else => IrMatchPattern::Else,
        HirMatchPattern::Literal(expression) => {
            IrMatchPattern::Value(lower_expression(expression, locals, context))
        }
        // coverage:off -- reachable only for a `Result OF ...` scrutinee, which is
        // rejected before lowering (TYPE_RESULT_NOT_MATCHABLE); kept for plan-20
        // total lowering when the AST checker is bypassed.
        HirMatchPattern::Union { type_, .. }
            if matches!(matched_type, ParameterType::ResultOf(_)) =>
        {
            let matched = match type_.name().as_ref() {
                "Ok" => "true",
                "Error" => "false",
                _ => "false",
            };
            IrMatchPattern::Value(IrValue::Const {
                type_: ParameterType::Boolean,
                value: matched.to_string(),
            })
        }
        // coverage:on
        HirMatchPattern::Union { type_, .. } => {
            IrMatchPattern::Value(IrValue::Local(type_.name().into_owned()))
        }
        HirMatchPattern::OneOf(expressions) => IrMatchPattern::OneOf(
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
    pattern: &HirMatchPattern,
    matched_local: &str,
    matched_type: &ParameterType,
) -> Option<(String, ParameterType, IrValue)> {
    match pattern {
        HirMatchPattern::Union { type_, binding } => {
            let type_name = type_.name().into_owned();
            // coverage:off -- a `Result OF ...` scrutinee is rejected before
            // lowering (TYPE_RESULT_NOT_MATCHABLE); this Ok/Error case binding is
            // kept only for plan-20 total lowering when the checker is bypassed.
            if let ParameterType::ResultOf(success) = matched_type {
                let success = (**success).clone();
                return match type_name.as_str() {
                    "Ok" => Some((
                        binding.clone(),
                        success.clone(),
                        IrValue::ResultValue {
                            type_: success,
                            value: Box::new(IrValue::Local(matched_local.to_string())),
                        },
                    )),
                    "Error" => Some((
                        binding.clone(),
                        ParameterType::named("Error"),
                        IrValue::ResultError {
                            value: Box::new(IrValue::Local(matched_local.to_string())),
                        },
                    )),
                    _ => None,
                };
            }
            // coverage:on
            // A stateful resource union's STATE is uniform across variants, so the
            // extracted variant binding carries the same STATE suffix as the
            // scrutinee (plan-74) — this is what lets `f.state` on `CASE File(f)`
            // resolve and lower through the concrete-record path. The `UnionExtract`
            // itself stays keyed on the bare variant type (it loads the variant
            // record pointer at `+8`).
            // The scrutinee's `STATE` clause is split out structurally and
            // re-attached to the variant (plan-106-C).
            let binding_type = match matched_type.state() {
                Some(state) => type_.with_state(&state),
                None => type_.clone(),
            };
            Some((
                binding.clone(),
                binding_type,
                IrValue::UnionExtract {
                    type_: type_.clone(),
                    value: Box::new(IrValue::Local(matched_local.to_string())),
                },
            ))
        }
        _ => None,
    }
}

fn lower_match_expression(
    expression: &HirExpression,
    matched_type: &ParameterType,
    locals: &HashMap<String, ParameterType>,
    context: &mut LowerContext<'_>,
) -> IrValue {
    // A `MATCH` scrutinee that is a call auto-unwraps like any other call site
    // (local error handling now uses an inline `TRAP`), so the scrutinee lowers
    // to its ordinary value. A `Result`-typed *value* (a local or field) keeps
    // its `Result OF …` type and is matched with `CASE Ok`/`CASE Error`.
    lower_expression_with_expected(expression, Some(matched_type), locals, context)
}

fn match_expression_type(
    expression: &HirExpression,
    locals: &HashMap<String, ParameterType>,
    context: &LowerContext<'_>,
) -> Option<ParameterType> {
    // Call scrutinees auto-unwrap; only a value already of `Result` type keeps
    // its `Result OF …` shape for `CASE Ok`/`CASE Error` matching.
    expression_type(expression, locals, context)
}

fn function_returns(hir: &HirProject) -> HashMap<String, ParameterType> {
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
                        let return_type = native_type(native.return_type.as_deref());
                        // Carry a stateful native producer's STATE, so a wrapper
                        // that calls `snd::rawOpen(p)` sees `SoundFile STATE
                        // FileInfo` and can RETURN it as its own stateful return
                        // (plan-53-A/B). Without this the call infers bare
                        // `SoundFile` and the wrapper's RETURN mismatches.
                        let return_type = match (native.return_resource, &native.return_state_type)
                        {
                            (true, Some(state)) => {
                                return_type.with_state(&ParameterType::parse(state))
                            }
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

fn function_types(hir: &HirProject) -> HashMap<String, ParameterType> {
    let mut types = HashMap::new();
    for file in &hir.files {
        for item in &file.items {
            if let HirItem::Function(function) = item {
                let params = function
                    .params
                    .iter()
                    .map(|param| param.type_.clone())
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
                        .map(|param| {
                            param
                                .type_name
                                .as_deref()
                                .map_or(ParameterType::Unknown, ParameterType::parse)
                        })
                        .collect::<Vec<_>>();
                    let returns = native_type(native.return_type.as_deref());
                    // Stateful native producer: carry its STATE in the callable
                    // type too (plan-53-A/B), matching `native_returns` above.
                    let returns = match (native.return_resource, &native.return_state_type) {
                        (true, Some(state)) => returns.with_state(&ParameterType::parse(state)),
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

fn function_params(hir: &HirProject) -> HashMap<String, Vec<CallParam>> {
    let mut params = HashMap::new();
    for file in &hir.files {
        for item in &file.items {
            if let HirItem::Function(function) = item {
                params.insert(
                    function.name.clone(),
                    function
                        .params
                        .iter()
                        .map(|param| CallParam {
                            name: param.name.clone(),
                            type_: param.type_.clone(),
                            default: param.default.clone(),
                        })
                        .collect(),
                );
            }
        }
    }
    params
}

fn declared_binding_types(hir: &HirProject) -> HashMap<String, ParameterType> {
    let mut bindings = HashMap::new();
    for file in &hir.files {
        for item in &file.items {
            if let HirItem::Binding(binding) = item {
                bindings.insert(binding.name.clone(), binding.type_.clone());
            }
        }
    }
    bindings
}

/// The declared type of a LINK native's return slot, defaulting an absent
/// annotation to `Nothing` exactly as the string form did.
///
/// `HirItem::Link` is the one HIR item that is NOT elaborated — it carries the
/// raw [`crate::ast::LinkBlock`], whose `return_type`/`type_name` are still
/// source strings (`src/hir/mod.rs:435`). So this call site IS that item kind's
/// AST→typed boundary and the canonical parser belongs here; elaborating
/// `LinkBlock` properly is recorded as a task in plan-106-E.
fn native_type(declared: Option<&str>) -> ParameterType {
    declared.map_or(ParameterType::Nothing, ParameterType::parse)
}

fn infer_binding_types(hir: &HirProject, context: &mut LowerContext<'_>) {
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
                        context.binding_types.insert(binding.name.clone(), type_);
                    }
                }
            }
        }
    }
}

fn expression_type(
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
                    .or_else(|| context.function_types.get(value).cloned())
                    .or_else(|| context.function_types.get(&canonical_value).cloned())
            }
        }
        HirExpression::Constructor { type_, .. } => {
            let type_name = type_.name();
            let canonical_type_name = canonical_import_name(&type_name, context);
            context
                .type_index
                .constructor_result(&canonical_type_name)
                .or_else(|| context.type_index.constructor_result(&type_name))
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
                    .get(type_name)
                    .is_some_and(|members| members.iter().any(|name| name == member))
                {
                    return Some(ParameterType::named(type_name));
                }
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
            if target_type == ParameterType::named("Error") {
                return match member.as_str() {
                    "code" => Some(ParameterType::Integer),
                    "message" => Some(ParameterType::String),
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
            context
                .type_index
                .record_field_type(target_type.name().as_ref(), member)
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
                return builtins::resolve_call_return_type_typed(
                    &canonical_callee,
                    &arg_types,
                    false,
                );
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
                return builtins::resolve_call_return_type_typed(
                    &canonical_callee,
                    &arg_types,
                    false,
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
            // `call_return_type_name` fallthrough. Routing it through the arg-typed path
            // would mis-resolve a Byte-parameter setter called with Integer literals
            // (`term::setForeground(255, 128, 0)` — the un-coerced `Integer` argument
            // fails to match the `Byte` parameter), regressing `Nothing` to `Unknown`.
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
                let arg_types =
                    normalize_builtin_call_arguments(canonical_callee.as_str(), arguments)
                        .iter()
                        .map(|argument| expression_type(argument, locals, context))
                        .collect::<Option<Vec<_>>>()?;
                return builtins::resolve_call_return_type_typed(
                    &canonical_callee,
                    &arg_types,
                    false,
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
                    nested.insert(param.name.clone(), param.type_.clone());
                    param.type_.clone()
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
            if matches!(
                operator.as_str(),
                "=" | "<>" | "<" | ">" | "<=" | ">=" | "AND" | "OR" | "XOR"
            ) {
                return Some(ParameterType::Boolean);
            }
            if operator == "&" {
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
                numeric::typed_binary_result_type(operator, &left, &right)
                    .unwrap_or(ParameterType::Integer),
            )
        }
        HirExpression::Unary {
            operator, operand, ..
        } => {
            if operator == "NOT" {
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
fn function_param_types_from_type(type_: &ParameterType) -> Option<Vec<ParameterType>> {
    match type_ {
        ParameterType::Func(params, _, _) => Some(params.clone()),
        _ => None,
    }
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
    // `IMPORT self` binds the current package's own exported interface, so a
    // `self::worker` reference resolves to the current package's *own* function,
    // which lives in `function_types` under its bare name (never a prefixed
    // imported-package key). Canonicalize `self.worker` to bare `worker` so the
    // `Identifier` lowering finds it in `function_types` and emits a `FunctionRef`
    // to the already-compiled `_mfb_fn_worker` symbol, rather than a dangling
    // `Local` (plan-81-import-self.md §4.4).
    if package == crate::ast::SELF_IMPORT {
        return rest.to_string();
    }
    format!("{package}.{rest}")
}

fn call_argument_expected_type(
    callee: &str,
    index: usize,
    arguments: &[HirCallArg],
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

fn normalize_builtin_call_arguments<'a>(
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
        return normalize_overloaded_builtin_call_arguments(&overloads, arguments);
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
/// table, mirroring `syntaxcheck`'s selection so both agree on which parameter a
/// name binds to. An unresolvable call was already rejected by the type checker;
/// keep its source order so lowering has something well-formed to walk.
fn normalize_overloaded_builtin_call_arguments<'a>(
    overloads: &[Vec<&str>],
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
    let Some(params) =
        builtins::select_param_name_overload(overloads, positionals.len(), &supplied_names)
    else {
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

fn lower_local_call_arguments(
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
            let expected = call_argument_expected_type(callee, index, arguments, locals, context);
            match argument {
                Some(argument) => Some(lower_expression_with_expected(
                    argument,
                    expected.as_ref(),
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

fn call_arg_value(argument: &HirCallArg) -> &HirExpression {
    match argument {
        HirCallArg::Positional(value) => value,
        HirCallArg::Named { value, .. } => value,
    }
}

fn lower_expression(
    expression: &HirExpression,
    locals: &HashMap<String, ParameterType>,
    context: &mut LowerContext<'_>,
) -> IrValue {
    lower_expression_with_expected(expression, None, locals, context)
}

/// The predicate's function type for a `filter`/`forEach`-style callback whose
/// collection argument has type `collection_type`: take the element type and ask
/// `filter_predicate_type_typed` for the bare predicate's signature. `None` when
/// the argument is not a `List OF …` or the name is not a resolvable general
/// built-in predicate (bug-342 A6 — was written inline at three call sites).
///
/// plan-106-A: matches the `ListOf` variant instead of `strip_prefix("List OF ")`.
fn filter_predicate_arg_type(
    predicate: &str,
    collection_type: &ParameterType,
) -> Option<ParameterType> {
    match collection_type {
        ParameterType::ListOf(element) => {
            crate::codegen::builtins::general::filter_predicate_type_typed(predicate, element)
        }
        _ => None,
    }
}

/// The constructor a **migrated** package's record constant `name`
/// (`"vector.zeroFloat3"`) inlines to, reconstructed from its
/// [`RegistryConstant`](crate::codegen::registry) — the flat per-field literals paired
/// with the element types read from the package's record fields, in order. `None` for a
/// scalar constant or a package that declares no record constant of that name (the
/// migrated `vector` package is the sole record-constant provider today).
fn registry_record_constant(name: &str) -> Option<IrValue> {
    use crate::codegen::registry;
    let components = registry::constant_components(name)?;
    let type_name = registry::constant_type_name(name)?;
    // Each component's element type comes from the record's declared fields, in order.
    let (package, _) = name.split_once('.')?;
    // plan-106-A: the record's declared field types are cloned from the descriptor
    // rather than rendered and re-parsed. The record TYPE is a nominal, so it is
    // built with `named` (a record constant never names a generic).
    let field_types: Vec<ParameterType> =
        match registry::registry().resolve_type(&format!("{package}.{type_name}"))? {
            registry::ResolvedType::Record(record) => {
                record.props.iter().map(|prop| prop.ty.clone()).collect()
            }
            _ => return None,
        };
    Some(IrValue::Constructor {
        type_: ParameterType::named(type_name),
        args: components
            .iter()
            .enumerate()
            .map(|(index, value)| IrValue::Const {
                type_: field_types
                    .get(index)
                    .cloned()
                    .unwrap_or(ParameterType::Unknown),
                value: value.to_string(),
            })
            .collect(),
    })
}

/// The function type to give a general built-in predicate named in a value
/// position, when `expected` is a concrete unary Boolean function type it
/// accepts (bug-368).
///
/// Mirrors `syntaxcheck`'s `builtin_predicate_value_type`: both consult
/// `filter_predicate_type`, so the type the checker assigns and the type the
/// `FunctionRef` carries cannot diverge. A divergence would emit a wrapper under
/// one symbol and reference another.
/// plan-106-A: the unary-predicate shape is matched on the expected type's
/// [`Func`](ParameterType::Func) variant, deleting the private
/// `function_type_parts_for_predicate` splitter this used to call — a
/// `strip_prefix("FUNC(")` + `split_once(") AS ")` copy of the type grammar that
/// mis-cut a higher-order parameter (it split at the FIRST `") AS "`).
fn builtin_predicate_ref_type(name: &str, expected: &ParameterType) -> Option<ParameterType> {
    let ParameterType::Func(params, returns, _) = expected else {
        return None;
    };
    if params.len() != 1 || **returns != ParameterType::Boolean {
        return None;
    }
    crate::codegen::builtins::general::filter_predicate_type_typed(name, &params[0])
}

fn lower_expression_with_expected(
    expression: &HirExpression,
    expected: Option<&ParameterType>,
    locals: &HashMap<String, ParameterType>,
    context: &mut LowerContext<'_>,
) -> IrValue {
    match expression {
        HirExpression::String(value) => IrValue::Const {
            type_: ParameterType::String,
            value: value.clone(),
        },
        HirExpression::Number(value) => {
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
                    numeric::LiteralType::Fixed => ParameterType::Fixed,
                    numeric::LiteralType::Money => ParameterType::Money,
                    _ => ParameterType::Float,
                }
            } else if expected == Some(&ParameterType::Fixed) {
                ParameterType::Fixed
            } else if expected == Some(&ParameterType::Byte) {
                ParameterType::Byte
            } else if expected == Some(&ParameterType::Money) {
                // An unsuffixed decimal literal coerces to a Money slot
                // (`LET a AS Money = 1.25`), mirroring the Fixed/Byte paths
                // (plan-29-A §4.4).
                ParameterType::Money
            } else {
                match literal_type {
                    numeric::LiteralType::Float => ParameterType::Float,
                    numeric::LiteralType::Fixed => ParameterType::Fixed,
                    numeric::LiteralType::Money => ParameterType::Money,
                    numeric::LiteralType::Integer => ParameterType::Integer,
                }
            };
            IrValue::Const {
                type_,
                value: canonical,
            }
        }
        HirExpression::Scalar(code_point) => IrValue::Const {
            type_: ParameterType::named("Scalar"),
            value: code_point.to_string(),
        },
        HirExpression::Boolean(value) => IrValue::Const {
            type_: ParameterType::Boolean,
            value: value.to_string(),
        },
        HirExpression::Identifier(value) if value == "NOTHING" => IrValue::Const {
            type_: ParameterType::Nothing,
            value: "NOTHING".to_string(),
        },
        HirExpression::Identifier(value) => {
            let canonical_value = canonical_import_name(value, context);
            // A record constant (`vector::upFloat3`) inlines a record constructor at
            // every use site, copying by value (plan-06-vector.md §4.19). The migrated
            // `vector` package's registry entry supplies the per-field literals + the
            // record's element types. Handled before the scalar-fold path below, which
            // expects a single literal value.
            if let Some(constructor) = registry_record_constant(&canonical_value) {
                return constructor;
            }
            if builtins::is_package_constant(&canonical_value) {
                let type_ = builtins::package_constant_type(&canonical_value)
                    .unwrap_or(ParameterType::Unknown);
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
        HirExpression::Call {
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
                    type_: ParameterType::Unknown,
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
            // lambda to slot-reference a `MUT` capture.
            let builtin_predicate_arg =
                (crate::codegen::registry::callback_member(&canonical_callee)
                    && normalized_builtin.len() == 2)
                    .then(|| match normalized_builtin[1] {
                        HirExpression::Identifier(predicate) => {
                            expression_type(normalized_builtin[0], locals, context)
                                .and_then(|collection_type| {
                                    filter_predicate_arg_type(predicate, &collection_type)
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
                        // License a `MUT` slot-reference capture for a lambda in a
                        // non-escaping callback position (e.g. `forEach`'s action).
                        // The lambda lowering consumes it; reset afterward so a
                        // non-lambda argument never carries it.
                        context.nonescaping_callback =
                            builtins::is_nonescaping_callback_arg(&canonical_callee, index);
                        let value = lower_expression_with_expected(
                            argument,
                            expected.as_ref(),
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
            // Pad optional trailing arguments so the fixed-ABI runtime helper
            // always receives every parameter (plan-72-BB: through the builtins
            // aggregate). A `List OF ...` default (crypto's AEAD `aad`) lowers to an
            // empty list literal and a `Map OF ...` default (http's `headers`) to an
            // empty map literal, not a scalar const; every other default is a const.
            let mut args = args;
            // plan-89-C: a Tier-A `strings::` query whose text argument is an
            // `AttributedString` reads its visible text. Wrap the leading argument
            // in `toString(a)` here — before the native vs source-companion-rewrite
            // split — so both lowerings receive a `String` and the result equals
            // `strings::q(toString(a))`. (Tier-B transforms are plan-89-D and return
            // `AttributedString` instead.)
            if crate::codegen::builtins::strings::is_tier_a_query(&canonical_callee)
                && !args.is_empty()
                && normalized_builtin
                    .first()
                    .and_then(|arg| expression_type(arg, locals, context))
                    == Some(attributed_string_type())
            {
                let inner = args[0].clone();
                args[0] = IrValue::Call {
                    target: "toString".to_string(),
                    args: vec![inner],
                    type_: ParameterType::String,
                    loc,
                };
            }
            // plan-89-D: the attribute-preserving `padLeft`/`padRight` source-companion
            // bodies take a required `padChar`; fill the default single space for the
            // 2-arg form of an `AttributedString` call. The native `String` forms
            // default `padChar` in codegen, so this only affects the astrings-routed
            // calls and leaves the `String` IR unchanged.
            if matches!(
                canonical_callee.as_str(),
                "strings.padLeft" | "strings.padRight"
            ) && args.len() == 2
                && normalized_builtin
                    .first()
                    .and_then(|arg| expression_type(arg, locals, context))
                    == Some(attributed_string_type())
            {
                args.push(IrValue::Const {
                    type_: ParameterType::String,
                    value: " ".to_string(),
                });
            }
            // The `Fill` trailing params of a migrated (clean-room registry) call.
            let padding =
                crate::codegen::registry::default_argument_padding(&canonical_callee, args.len());
            for (type_, value) in &padding {
                // plan-106-A: the `Fill` type arrives typed, so the empty-collection
                // vs scalar choice is a variant match, not a prefix test.
                match type_ {
                    ParameterType::ListOf(_) => args.push(IrValue::ListLiteral {
                        type_: type_.clone(),
                        values: Vec::new(),
                    }),
                    ParameterType::MapOf(_, _) => args.push(IrValue::MapLiteral {
                        type_: type_.clone(),
                        entries: Vec::new(),
                    }),
                    _ => args.push(IrValue::Const {
                        type_: type_.clone(),
                        value: (*value).to_string(),
                    }),
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
            let package_override =
                if crate::codegen::builtins::general::is_overridable(&canonical_callee) {
                    arguments
                        .first()
                        .map(call_arg_value)
                        .and_then(|argument| expression_type(argument, locals, context))
                        // The per-package override table is keyed by type NAME
                        // (a hand-authored dispatch map in codegen), so the
                        // operand type renders for that lookup.
                        .and_then(|type_| {
                            builtins::general_override_target(&canonical_callee, &type_.name())
                        })
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
                        .filter(|type_| {
                            type_.name() == crate::codegen::builtins::tls::TLS_LISTENER_TYPE
                        })
                        .map(|_| crate::codegen::builtins::tls::CLOSE_LISTENER.to_string())
                })
                .or_else(|| {
                    // `audio::` rewrites the overloads whose *body* differs while no user
                    // error is reachable onto their own internal runtime-helper name: the
                    // named-device opens, the timed `read`/`poll`, and the per-direction
                    // `close` (plan-33-A §5). Done at IR level (the `tls.closeListener`
                    // idiom) so the NIR carries the exact runtime-call name and the spec
                    // catalog / required-helper emission / import planning stay
                    // byte-identical. The target is a runtime helper, not a source
                    // companion, so it is not internalized.
                    if crate::codegen::registry::registry().owning_package(&canonical_callee)
                        != Some("audio")
                    {
                        return None;
                    }
                    // These per-package selectors match on type NAMES (exact
                    // record-type dispatch tables in codegen), so the argument
                    // types render at that seam.
                    let arg_types: Vec<String> = arguments
                        .iter()
                        .map(call_arg_value)
                        .map(|argument| {
                            expression_type(argument, locals, context)
                                .map(|type_| type_.name().into_owned())
                                .unwrap_or_default()
                        })
                        .collect();
                    crate::codegen::builtins::audio::runtime_overload_name(
                        &canonical_callee,
                        &arg_types,
                    )
                    .map(str::to_string)
                })
                .or_else(|| {
                    // `vector::` selects its type-specific internal FUNC from the call's
                    // argument record types (`vector.length(Float3)` ->
                    // `#vector_length_float3`). Each `vector.<op>` overload carries a
                    // `Body::Rewrite("__vector_<op>_<type>")` on the registry, but the
                    // generic coarse-nominal matcher cannot distinguish the nine record
                    // types, so `vector` keeps an EXACT selector over that overload data.
                    if crate::codegen::registry::registry().owning_package(&canonical_callee)
                        != Some("vector")
                    {
                        return None;
                    }
                    // These per-package selectors match on type NAMES (exact
                    // record-type dispatch tables in codegen), so the argument
                    // types render at that seam.
                    let arg_types: Vec<String> = arguments
                        .iter()
                        .map(call_arg_value)
                        .map(|argument| {
                            expression_type(argument, locals, context)
                                .map(|type_| type_.name().into_owned())
                                .unwrap_or_default()
                        })
                        .collect();
                    crate::codegen::builtins::vector::rewrite_target(&canonical_callee, &arg_types)
                        .map(crate::internal_name::internalize)
                })
                .or_else(|| {
                    // `term::drawText(x, y, AttributedString)` routes to the
                    // `__term_drawTextAttr` source-companion body, which applies the
                    // per-scalar bold/underline attributes over the native `String`
                    // drawText. A `String` third argument stays the native
                    // `term.drawText` runtime helper. The companion body is a source
                    // rewrite, so its target is internalized.
                    if canonical_callee != crate::codegen::builtins::term::DRAW_TEXT {
                        return None;
                    }
                    let text_arg_type = arguments
                        .get(2)
                        .map(call_arg_value)
                        .and_then(|argument| expression_type(argument, locals, context));
                    if text_arg_type != Some(attributed_string_type()) {
                        return None;
                    }
                    Some(crate::internal_name::internalize("__term_drawTextAttr"))
                })
                .or_else(|| {
                    // plan-89-D: a Tier-B `strings::` transform whose text argument
                    // is an `AttributedString` routes to its `__astrings_*`
                    // attribute-preserving source-companion body (the native String
                    // transform stays for a String argument).
                    if !crate::codegen::builtins::strings::is_tier_b_transform(&canonical_callee) {
                        return None;
                    }
                    let first_arg_type = arguments
                        .first()
                        .map(call_arg_value)
                        .and_then(|argument| expression_type(argument, locals, context));
                    if first_arg_type != Some(attributed_string_type()) {
                        return None;
                    }
                    crate::codegen::builtins::strings::tier_b_transform_impl(&canonical_callee)
                        .map(crate::internal_name::internalize)
                })
                .or_else(|| {
                    // The migrated (clean-room registry) packages rewrite through the
                    // generic, overload-aware `registry::rewrite_target`: an arity-routed
                    // member (datetime's `instant`/`parse`) selects the overload matching
                    // the call's argument types and hands back that overload's target.
                    // `encoding`'s two return-type-overloaded names (`utf8Encode`/
                    // `utf8Decode`) are `Body::Intrinsic` and intentionally yield no
                    // target, so the canonical name reaches the monomorphizer.
                    // These per-package selectors match on type NAMES (exact
                    // record-type dispatch tables in codegen), so the argument
                    // types render at that seam.
                    let arg_types: Vec<String> = arguments
                        .iter()
                        .map(call_arg_value)
                        .map(|argument| {
                            expression_type(argument, locals, context)
                                .map(|type_| type_.name().into_owned())
                                .unwrap_or_default()
                        })
                        .collect();
                    // `strings`' seven scalar-seam members (`toScalars`/`isLetter`/…)
                    // migrated to `Body::Rewrite("__strings_*")`, so the generic
                    // `registry::rewrite_target` now hands back their internal target
                    // (plan-99 PART B), replacing the old
                    // `builtins::strings::implementation_name` fallback.
                    crate::codegen::registry::rewrite_target(&canonical_callee, &arg_types)
                        .map(crate::internal_name::internalize)
                })
                .unwrap_or_else(|| canonical_callee.clone());
            let result_type =
                expression_type(expression, locals, context).unwrap_or(ParameterType::Unknown);
            IrValue::Call {
                // The resource plane reuses the proven data-channel runtime:
                // `thread::transfer`/`accept` lower exactly like `send`/`receive`
                // (syntaxcheck already enforced their resource semantics).
                target: thread_resource_plane_target(&resolved_target).to_string(),
                args,
                type_: result_type.clone(),
                loc,
            }
        }
        HirExpression::Lambda {
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
            // A `MUT` capture in a proven non-escaping position is a reference to the
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
                    lambda_locals.insert(param.name.clone(), param.type_.clone());
                    IrParam {
                        name: param.name.clone(),
                        type_: param.type_.clone(),
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
                    ParameterType::Nothing
                }
                None => {
                    let returns = expression_type(body, &lambda_locals, context)
                        .unwrap_or(ParameterType::Unknown);
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
                .map(|param| param.type_.clone())
                .collect::<Vec<_>>();
            // A lambda is never `ISOLATED`, matching the un-prefixed
            // `FUNC(…) AS …` spelling this used to `format!`.
            let type_ = ParameterType::Func(params, Box::new(returns), false);
            if captures.is_empty() {
                IrValue::FunctionRef { name, type_ }
            } else {
                IrValue::Closure {
                    name,
                    type_: type_.clone(),
                    captures: captures
                        .iter()
                        .zip(by_ref.iter())
                        .map(|(capture, &by_ref)| {
                            if by_ref {
                                // Capture the parent slot's address (by-ref), so
                                // the callback observes and updates the live binding.
                                IrValue::LocalRef {
                                    name: capture.name.clone(),
                                    type_: capture.type_.clone(),
                                }
                            } else {
                                lower_expression(
                                    &HirExpression::Identifier(capture.name.clone()),
                                    locals,
                                    context,
                                )
                            }
                        })
                        .collect(),
                }
            }
        }
        HirExpression::Constructor {
            type_: constructor_type,
            arguments,
        } => {
            let type_name = constructor_type.name();
            let canonical_type_name = canonical_import_name(&type_name, context);
            let fields = context
                .type_index
                .records
                .get(&canonical_type_name)
                .or_else(|| context.type_index.records.get(type_name.as_ref()))
                .or_else(|| context.type_index.variant_fields.get(&canonical_type_name))
                .or_else(|| context.type_index.variant_fields.get(type_name.as_ref()));
            let base = IrValue::Constructor {
                // Deliberately `parse`, not `named`: `canonical_import_name` rewrites
                // an import ALIAS inside the type's spelling (a name-domain edit), and
                // the result must be re-classified — a user generic (`Pair OF A, B`)
                // has to come back as a `UserOf`, not an opaque nominal.
                type_: crate::types::ParameterType::parse(&canonical_type_name),
                args: lower_constructor_args(arguments, fields, locals, context),
            };
            wrap_union_value(base, expression, expected, locals, context)
        }
        HirExpression::WithUpdate { target, updates } => {
            let type_ = expression_type(target, locals, context).unwrap_or(ParameterType::Unknown);
            let lowered_target = Box::new(lower_expression(target, locals, context));
            let lowered_updates = updates
                .iter()
                .map(|update| {
                    // Coerce a bare numeric literal to the record field's
                    // declared type, mirroring `lower_constructor_args` — else an
                    // unsuffixed literal updating a `Fixed`/`Money` field is typed
                    // `Integer` and reinterpreted as raw bits (bug-156).
                    let field_type = context
                        .type_index
                        .record_field_type(type_.name().as_ref(), &update.field);
                    IrRecordUpdate {
                        field: update.field.clone(),
                        value: lower_expression_with_expected(
                            &update.value,
                            field_type.as_ref(),
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
        HirExpression::ListLiteral(values) => {
            let expected_element = match expected {
                Some(ParameterType::ListOf(element)) => Some((**element).clone()),
                _ => None,
            };
            let lowered = values
                .iter()
                .map(|value| {
                    lower_expression_with_expected(
                        value,
                        expected_element.as_ref(),
                        locals,
                        context,
                    )
                })
                .collect::<Vec<_>>();
            let element_type = expected_element.unwrap_or_else(|| {
                values
                    .first()
                    .and_then(literal_expression_type)
                    .unwrap_or(ParameterType::Unknown)
            });
            IrValue::ListLiteral {
                type_: ParameterType::list_of(element_type),
                values: lowered,
            }
        }
        HirExpression::SetLiteral {
            element_type,
            elements,
        } => {
            let expected_element = match expected {
                Some(ParameterType::SetOf(element)) => (**element).clone(),
                _ => element_type.clone(),
            };
            let lowered = elements
                .iter()
                .map(|value| {
                    lower_expression_with_expected(value, Some(&expected_element), locals, context)
                })
                .collect::<Vec<_>>();
            IrValue::SetLiteral {
                type_: ParameterType::set_of(element_type.clone()),
                values: lowered,
            }
        }
        HirExpression::MapLiteral {
            key_type,
            value_type,
            entries,
        } => {
            let (expected_key, expected_value) = match expected {
                Some(ParameterType::MapOf(key, value)) => {
                    (Some((**key).clone()), Some((**value).clone()))
                }
                _ => (None, None),
            };
            let expected_key = expected_key.as_ref();
            let expected_value = expected_value.as_ref();
            IrValue::MapLiteral {
                type_: ParameterType::map_of(key_type.clone(), value_type.clone()),
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
        HirExpression::MemberAccess { target, member } => {
            let member_type =
                expression_type(expression, locals, context).unwrap_or(ParameterType::Unknown);
            IrValue::MemberAccess {
                target: Box::new(lower_expression(target, locals, context)),
                member: member.clone(),
                type_: member_type,
            }
        }
        HirExpression::Trapped { .. } => {
            // Inline traps are only constructed as the value of a binding,
            // assignment, or bare-expression statement, where `lower_statement`
            // desugars them directly; they never reach value lowering.
            unreachable!("inline TRAP must be lowered as a statement value")
        }
        HirExpression::Binary {
            left,
            operator,
            right,
            line,
            column,
        } => {
            let result_type =
                expression_type(expression, locals, context).unwrap_or(ParameterType::Unknown);
            let loc = IrSourceLoc {
                line: *line as u32,
                column: *column as u32,
            };
            // plan-89-D: `AttributedString & AttributedString` concatenation routes
            // to the `__astrings_concat` source-companion body (text concatenated,
            // right operand's spans shifted by the left's scalar length).
            if operator == "&" && result_type == attributed_string_type() {
                return IrValue::Call {
                    target: crate::internal_name::internalize("__astrings_concat"),
                    args: vec![
                        lower_expression(left, locals, context),
                        lower_expression(right, locals, context),
                    ],
                    type_: attributed_string_type(),
                    loc,
                };
            }
            IrValue::Binary {
                op: operator.clone(),
                left: Box::new(lower_expression(left, locals, context)),
                right: Box::new(lower_expression(right, locals, context)),
                type_: result_type.clone(),
                loc,
            }
        }
        HirExpression::Unary {
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
                && matches!(
                    expected,
                    Some(ParameterType::Money) | Some(ParameterType::Fixed)
                )
                && matches!(operand.as_ref(), HirExpression::Number(_));
            let result_type = if exact_literal_negation {
                expected.cloned().unwrap_or(ParameterType::Unknown)
            } else {
                expression_type(expression, locals, context).unwrap_or(ParameterType::Unknown)
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
                    if matches!(type_, crate::types::ParameterType::Fixed)
                        && numeric::fixed_raw_from_decimal(value).is_err()
                        && numeric::fixed_raw_from_decimal(&format!("-{value}")).is_ok()
                    {
                        return IrValue::Const {
                            type_: ParameterType::Fixed,
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
                    if matches!(type_, crate::types::ParameterType::Integer)
                        && value.parse::<i64>().is_err()
                        && format!("-{value}").parse::<i64>().is_ok()
                    {
                        return IrValue::Const {
                            type_: ParameterType::Integer,
                            value: format!("-{value}"),
                        };
                    }
                    // The same fold for the most-negative Money
                    // (`-92233720368547.75808`), whose positive magnitude
                    // overflows the i64 raw (plan-29-B §4.2).
                    if matches!(type_, crate::types::ParameterType::Money)
                        && numeric::money_raw_from_decimal(value).is_err()
                        && numeric::money_raw_from_decimal(&format!("-{value}")).is_ok()
                    {
                        return IrValue::Const {
                            type_: ParameterType::Money,
                            value: format!("-{value}"),
                        };
                    }
                }
            }
            IrValue::Unary {
                op: operator.clone(),
                operand: Box::new(lowered_operand),
                type_: result_type.clone(),
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
        type_: ParameterType::named("ErrorLoc"),
        args: vec![
            IrValue::Const {
                type_: ParameterType::String,
                value: file.to_string(),
            },
            IrValue::Const {
                type_: ParameterType::Integer,
                value: loc.line.to_string(),
            },
            IrValue::Const {
                type_: ParameterType::Integer,
                value: loc.column.to_string(),
            },
        ],
    }
}

/// Build an `Error` record value (code, message, source) for `error(...)`.
fn build_error_value(code: IrValue, message: IrValue, file: &str, loc: IrSourceLoc) -> IrValue {
    IrValue::Constructor {
        type_: ParameterType::named("Error"),
        args: vec![code, message, error_loc_value(file, loc)],
    }
}

fn wrap_union_value(
    base: IrValue,
    expression: &HirExpression,
    expected: Option<&ParameterType>,
    locals: &HashMap<String, ParameterType>,
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
    // The variant/union membership index is keyed by NAME (a type-declaration
    // table), so both sides render for the lookup only.
    if context
        .type_index
        .variant_belongs_to_union(actual_type.name().as_ref(), union_type.name().as_ref())
    {
        return IrValue::UnionWrap {
            union_type: union_type.clone(),
            member_type: actual_type,
            value: Box::new(base),
        };
    }
    base
}

fn lower_constructor_args(
    arguments: &[HirConstructorArg],
    fields: Option<&Vec<IrField>>,
    locals: &HashMap<String, ParameterType>,
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
        .all(|argument| matches!(argument, HirConstructorArg::Named { .. }))
    {
        return fields
            .iter()
            .filter_map(|field| {
                arguments.iter().find_map(|argument| match argument {
                    HirConstructorArg::Named { name, value, .. } if name == &field.name => Some(
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
            let expected = fields.get(index).map(|field| &field.type_);
            lower_expression_with_expected(
                constructor_arg_value(argument),
                expected,
                locals,
                context,
            )
        })
        .collect()
}

fn constructor_arg_value(argument: &HirConstructorArg) -> &HirExpression {
    match argument {
        HirConstructorArg::Positional(value) => value,
        HirConstructorArg::Named { value, .. } => value,
    }
}

fn captured_locals(
    expression: &HirExpression,
    outer_locals: &HashMap<String, ParameterType>,
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
    expression: &HirExpression,
    outer_locals: &HashMap<String, ParameterType>,
    local_names: &HashSet<String>,
    seen: &mut HashSet<String>,
    captures: &mut Vec<CapturedLocal>,
) {
    match expression {
        HirExpression::Identifier(name) => {
            if let Some(type_) = outer_locals.get(name) {
                if !local_names.contains(name) && seen.insert(name.clone()) {
                    captures.push(CapturedLocal {
                        name: name.clone(),
                        type_: type_.clone(),
                    });
                }
            }
        }
        HirExpression::Call {
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
        HirExpression::Lambda { .. } => {}
        HirExpression::Binary { left, right, .. } => {
            collect_captured_locals(left, outer_locals, local_names, seen, captures);
            collect_captured_locals(right, outer_locals, local_names, seen, captures);
        }
        HirExpression::Unary { operand, .. } => {
            collect_captured_locals(operand, outer_locals, local_names, seen, captures);
        }
        HirExpression::Constructor { arguments, .. } => {
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
        HirExpression::ListLiteral(values) => {
            for value in values {
                collect_captured_locals(value, outer_locals, local_names, seen, captures);
            }
        }
        HirExpression::SetLiteral { elements, .. } => {
            for value in elements {
                collect_captured_locals(value, outer_locals, local_names, seen, captures);
            }
        }
        HirExpression::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                collect_captured_locals(key, outer_locals, local_names, seen, captures);
                collect_captured_locals(value, outer_locals, local_names, seen, captures);
            }
        }
        HirExpression::MemberAccess { target, .. } => {
            collect_captured_locals(target, outer_locals, local_names, seen, captures);
        }
        HirExpression::WithUpdate { target, updates } => {
            collect_captured_locals(target, outer_locals, local_names, seen, captures);
            for update in updates {
                collect_captured_locals(&update.value, outer_locals, local_names, seen, captures);
            }
        }
        HirExpression::Trapped { expression, .. } => {
            collect_captured_locals(expression, outer_locals, local_names, seen, captures);
        }
        HirExpression::String(_)
        | HirExpression::Number(_)
        | HirExpression::Scalar(_)
        | HirExpression::Boolean(_) => {}
    }
}

fn literal_expression_type(expression: &HirExpression) -> Option<ParameterType> {
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
    fn new(hir: &HirProject, imported_types: &[ImportedTypeDef]) -> Self {
        let mut records = HashMap::new();
        let mut enums = HashMap::new();
        let mut variants = HashMap::new();
        let mut variant_unions = HashMap::<String, HashSet<String>>::new();
        let mut variant_fields = HashMap::new();
        let union_decls = hir
            .files
            .iter()
            .flat_map(|file| &file.items)
            .filter_map(|item| {
                let HirItem::Type(type_decl) = item else {
                    return None;
                };
                if matches!(type_decl.kind, TypeDeclKind::Union) {
                    Some((type_decl.name.clone(), type_decl))
                } else {
                    None
                }
            })
            .collect::<HashMap<_, _>>();
        for file in &hir.files {
            for item in &file.items {
                let HirItem::Type(type_decl) = item else {
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
        // Fold in the types of imported (non-builtin) packages, decoded from
        // their `.mfp`. A locally-declared type always wins (`or_insert_with`),
        // so this only *adds* layouts the consumer would otherwise lack — the
        // reason an imported `record.field` used to type as `Unknown`. Built-in
        // packages are already covered above: their source is in the AST.
        let imported_field = |field: &ImportedTypeField| IrField {
            visibility: None,
            name: field.name.clone(),
            type_: ParameterType::parse(&field.type_),
            loc: IrSourceLoc::default(),
        };
        for imported in imported_types {
            match imported.kind {
                ImportedTypeKind::Record => {
                    records
                        .entry(imported.name.clone())
                        .or_insert_with(|| imported.fields.iter().map(imported_field).collect());
                }
                ImportedTypeKind::Enum => {
                    enums
                        .entry(imported.name.clone())
                        .or_insert_with(|| imported.members.clone());
                }
                ImportedTypeKind::Union => {
                    for variant in &imported.variants {
                        variants
                            .entry(variant.name.clone())
                            .or_insert_with(|| imported.name.clone());
                        variant_unions
                            .entry(variant.name.clone())
                            .or_default()
                            .insert(imported.name.clone());
                        variant_fields
                            .entry(variant.name.clone())
                            .or_insert_with(|| variant.fields.iter().map(imported_field).collect());
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

    fn constructor_result(&self, name: &str) -> Option<ParameterType> {
        if name == "Error" {
            Some(ParameterType::named("Error"))
        } else if name == "Ok" {
            Some(ParameterType::result_of(ParameterType::Unknown))
        } else if self.records.contains_key(name) {
            Some(ParameterType::named(name))
        } else {
            // A union variant's owning union, held as a NAME (the index is keyed by
            // variant name); it denotes a nominal.
            self.variants
                .get(name)
                .map(|union| ParameterType::named(union))
        }
    }

    fn record_field_type(&self, type_name: &str, member: &str) -> Option<ParameterType> {
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
    type_decl: &'a HirTypeDecl,
    union_decls: &HashMap<String, &'a HirTypeDecl>,
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
