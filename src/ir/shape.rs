//! The pre-lowering shape pass (plan-107-E).
//!
//! `ir::verify` checks the IR, on both the source path and the package path.
//! A handful of source rules cannot live there because lowering *erases* the
//! evidence they need — a named argument is normalized into positional order,
//! an omitted named argument is filled from the parameter's default, and so on.
//! Those rules run here, over the concrete `HirProject` lowering is about to
//! consume, in the build's first diagnostic stream (before `ir::verify`'s).
//!
//! The pass carries no type inference of its own. Where a rule needs the type
//! of an expression it asks lowering — [`lower::expression_type`] against a
//! [`lower::LowerContext`] built from the same [`lower::LowerFacts`] the build
//! path lowers with — and it tracks its local scopes exactly as
//! `lower_statement_block` does (parameters, `LET`/`MUT`/`RES` binds, `FOR` /
//! `FOR EACH` variables, `MATCH` case bindings, trap bindings, lambda
//! parameters), so a rule sees precisely the type the lowered `IrValue` will
//! carry. Total lowering (plan-20-D) already tolerates ill-typed input, so the
//! oracle never panics on the erroneous programs these rules exist for.
//!
//! Every rule this pass emits carries a one-line justification naming the
//! erased evidence that keeps it out of `ir::verify`.

use super::lower::{self, LowerContext, LowerFacts};
use super::{ExternalSignature, ImportedTypeDef};
use crate::ast::{ExitTarget, FunctionKind, Visibility};
use crate::codegen::builtins;
use crate::hir::{
    HirCallArg, HirConstructorArg, HirExpression, HirFile, HirFunction, HirItem, HirMatchCase,
    HirProject, HirStatement,
};
use crate::operators::{BinaryOp, UnaryOp};
use crate::rules::PendingDiagnostic;
use crate::types::ParameterType;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Run the shape pass over `hir` and return its diagnostics in traversal
/// (source) order, un-rendered, for the build path to merge with the other
/// streams. `imported_types` is the same input the build path hands
/// `lower_augmented_project`; `imported_signatures` is the UNFILTERED signature
/// table of every imported `.mfp` (the parameter-name and callable-type source
/// for a call into an imported package).
pub(crate) fn collect_diagnostics(
    project_dir: &Path,
    hir: &HirProject,
    imported_types: &[ImportedTypeDef],
    imported_signatures: &HashMap<String, ExternalSignature>,
    imported_resource_types: &[String],
) -> Vec<PendingDiagnostic> {
    // The typing seam is built over the UNFILTERED imported-signature table:
    // the source checker typed a reference to an imported function by its
    // `.mfp` signature, so a `thread::start(pkg::worker, …)` argument must type
    // as that ISOLATED FUNC here too. (Lowering's own facts keep only the
    // resource-returning subset for `ir::verify`'s sake.)
    let facts = lower::lower_facts(hir, imported_signatures, imported_types);
    let mut walker = Walker::new(
        project_dir,
        &facts,
        hir,
        imported_types,
        imported_signatures,
        imported_resource_types,
    );
    walker.walk_project(hir);
    walker.diagnostics
}

/// Standalone form for callers that render rather than merge (`mfb audit`):
/// prints the diagnostics and reports whether any was an error.
pub(crate) fn check_project(
    project_dir: &Path,
    hir: &HirProject,
    imported_signatures: &HashMap<String, ExternalSignature>,
) -> Result<(), ()> {
    let diagnostics = collect_diagnostics(project_dir, hir, &[], imported_signatures, &[]);
    let had_error = diagnostics.iter().any(|d| crate::rules::is_error(&d.rule));
    crate::rules::render_pending(diagnostics);
    if had_error {
        Err(())
    } else {
        Ok(())
    }
}

/// A callee's parameter names as a rule needs them: the declared user/imported
/// function's list, or a builtin's per-position alias table.
enum CalleeParams {
    /// A user-declared or imported-package function: one parameter per position.
    Declared(Vec<ShapeParam>),
    /// A builtin with a merged per-position alias table.
    Builtin(Vec<Vec<&'static str>>),
    /// A builtin whose overloads place a name at different positions, listed
    /// one overload at a time.
    BuiltinOverloads(Vec<Vec<&'static str>>),
    /// A builtin with no parameter-name metadata: names cannot bind at all.
    BuiltinUnnamed,
    /// A local or global binding of FUNC type: the callable type carries a
    /// parameter count but no names.
    FunctionValue(Vec<ParameterType>),
}

/// The walk state: lowering's context (positioned per file / per function
/// exactly as lowering positions it), the project's own function visibility
/// table, and the diagnostics collected so far.
struct Walker<'a> {
    project_dir: &'a Path,
    context: LowerContext<'a>,
    /// Every declared function's parameter names, visibility and owner file —
    /// the visibility filter the source checker applied to a call target
    /// (`PRIVATE` is callable from its own file only; an invisible target is
    /// simply not a function call to it).
    functions: HashMap<String, DeclaredFunction>,
    imported_signatures: &'a HashMap<String, ExternalSignature>,
    /// Whether any file of the project imports `astrings` — the gate for the
    /// `term::drawText(AttributedString)` bridge body, which is injected only
    /// then.
    astrings_imported: bool,
    /// Whether the function being walked is a `SUB` (the EXIT/RETURN forms
    /// that depend on it).
    current_is_sub: bool,
    /// The success types of the enclosing inline-`TRAP` handlers (innermost
    /// last): what a `RECOVER` must (or must not) supply.
    inline_trap_types: Vec<ParameterType>,
    /// Resource type names beyond the builtin ones — the project's native
    /// (`LINK`) resources and the imported packages' `RESOURCE_TABLE` rows —
    /// the checker's resource registry knew (a resource is never
    /// `=`-comparable).
    resource_types: HashSet<ParameterType>,
    /// How many inline-TRAP handlers enclose the current statement:
    /// `treeify_handler` drops every statement after a terminator inside a
    /// handler, so no exit's unreachable tail survives lowering there.
    handler_depth: usize,
    /// Every declared and imported type, for the compatibility rule's union
    /// and same-declaration questions. plan-111-B: keyed BY THE TYPE — a
    /// declared type is a nominal, so a key is `ParameterType::named(<name>)`.
    types: HashMap<ParameterType, TypeShape>,
    /// Project-relative path of the file being walked, for diagnostic paths.
    file: String,
    /// Source line of the statement (or declaration) being walked, for the
    /// expression-level rules that report at the statement.
    current_line: usize,
    /// Whether the call just checked is one the source checker typed
    /// `Unknown` — a builtin whose count or argument types failed, or a
    /// `thread.start` with a bad entry — so a binding of its value cascades
    /// TYPE_UNKNOWN_VALUE. Set by `check_call_shape`, read by the Call arm.
    call_typed_unknown: bool,
    /// The verdict above per call expression (keyed by the HIR node's address,
    /// stable for the walk), so a binding can ask about its initializer's
    /// outermost call after the walk descended through it.
    call_verdicts: HashMap<usize, bool>,
    /// Locals bound by a plain `LET`/`MUT` (not `RES`) to a stateful resource:
    /// the source checker kept a binding's `STATE` only on the `RES` axis, so
    /// `.state` on such a local typed `Unknown` there (bug-376's displaced
    /// error, pinned by its fixture).
    state_dropped: HashSet<String>,
    /// Every builtin package RECORD, keyed by its type, mapped to the package
    /// that declares it — the authority for the field-access import rule
    /// (bug-466). Built from the REGISTRY rather than from the project's type
    /// table, because that table holds whatever the injected companion sources
    /// happened to declare: `udp`'s `Datagram` names a `net.Address`, so an
    /// unrelated `IMPORT udp` used to drag `Address`'s declaration in and make
    /// an otherwise-identical `tcp` program compile. Keying on the registry and
    /// the file's own imports is the only spelling that gives one verdict.
    ///
    /// A name the project itself declares — or an imported `.mfp` exports, or
    /// two builtin packages both declare — is excluded: there the nominal does
    /// not unambiguously mean the builtin's record, so there is no import to
    /// name and the rule stays silent.
    builtin_record_owner: HashMap<ParameterType, &'static str>,
    /// Whether the file being walked is compiler-injected builtin package
    /// source. Such a file declares the very types the rule above guards and is
    /// authored against the registry's own import graph, so the rule does not
    /// apply to it — and a diagnostic reported at `<builtin-net>` would name a
    /// path the user cannot open.
    current_file_internal: bool,
    /// The packages the file being walked declared itself — `HirFile::own_imports`,
    /// NOT `self.context.current_imports`. Monomorphization widens a file's
    /// `imports` with the project-wide union, so the resolution scope is not the
    /// author's list; the foreign-record field rule needs the author's.
    current_own_imports: HashSet<String>,
    diagnostics: Vec<PendingDiagnostic>,
    /// Every `LET`/`MUT`/`RES` binding's computed type in walk order — the
    /// seam-fidelity probe the unit tests compare against lowering's stamped
    /// `IrOp::Bind` types.
    #[cfg(test)]
    bound_types: Vec<(String, ParameterType)>,
}

struct DeclaredFunction {
    params: Vec<ShapeParam>,
    visibility: Visibility,
    owner_file: String,
    isolated: bool,
    kind: crate::ast::FunctionKind,
}

/// What the call-shape rules know about one declared parameter: its name (the
/// named-argument rules) and whether it may be omitted (the arity rule).
#[derive(Clone)]
struct ShapeParam {
    name: String,
    type_: ParameterType,
    has_default: bool,
}

/// `EXPORT` is only meaningful in a package project — it is the flag that writes a
/// symbol into the compiled `.mfp` public API. An executable produces no `.mfp`,
/// so a top-level `EXPORT` declaration there is an error (`EXPORT_IN_EXECUTABLE`);
/// project-wide visibility inside an executable is `PUBLIC` (the default). This
/// runs in the build pipeline, where the manifest `kind` is known — beside the
/// shape pass, whose stream it follows (plan-107-D), but over the original AST
/// rather than the concrete HIR the pass walks.
pub(crate) fn export_in_executable_diagnostics(
    is_package: bool,
    ast: &crate::ast::AstProject,
) -> Vec<crate::rules::PendingDiagnostic> {
    // Reads the ORIGINAL source AST at the build boundary, before elaboration —
    // `EXPORT` placement is a source-syntax fact, and the pre-monomorph AST is
    // where the user's own declarations still are.
    use crate::ast::Item;
    if is_package {
        return Vec::new();
    }
    let mut diagnostics = Vec::new();
    for file in &ast.files {
        // Skip toolchain-provided source: injected builtin packages
        // (`HirFile::internal`) and the synthetic prelude (`<builtin …>` path),
        // which legitimately carry EXPORT declarations.
        if file.internal || file.path.starts_with('<') {
            continue;
        }
        for item in &file.items {
            let (visibility, line) = match item {
                Item::Binding(binding) => (binding.visibility, binding.line),
                Item::Function(function) => (function.visibility, function.line),
                Item::Type(type_decl) => (type_decl.visibility, type_decl.line),
                Item::Resource(resource) => (resource.visibility, resource.line),
                Item::FuncAlias(alias) => (alias.visibility, alias.line),
                Item::Link(_) | Item::Doc(_) | Item::Testing(_) => continue,
            };
            if matches!(visibility, Visibility::Export) {
                diagnostics.push(crate::rules::PendingDiagnostic {
                    rule: "EXPORT_IN_EXECUTABLE".to_string(),
                    detail: "EXPORT is only valid in a package project; use PUBLIC (the \
                             default) in an executable."
                        .to_string(),
                    path: std::path::PathBuf::from(&file.path),
                    line,
                });
            }
        }
    }
    diagnostics
}

/// One declared or imported type as the compatibility rule needs it: its
/// identity (so two distinct declarations sharing a bare name never unify)
/// and, for a union, its variant names.
struct TypeShape {
    id: usize,
    variants: Vec<String>,
    /// A `TYPE` (record) — the one kind a constructor or `WITH` produces.
    is_record: bool,
    is_union: bool,
    is_enum: bool,
    /// A record's field types, for the `=`-comparability rule.
    fields: Vec<ParameterType>,
    /// A union's referenced types — its variant records (declared) or its
    /// variants' field types (imported) — for the package-metadata walk.
    variant_types: Vec<ParameterType>,
    /// An `ENUM`'s member names, for MATCH exhaustiveness.
    members: Vec<String>,
    visibility: Visibility,
    /// Declaring file (`PRIVATE` types are visible only there); empty for an
    /// imported type.
    file: String,
}

/// The `RES` element marker is an ownership-axis annotation, not a value type.
fn strip_res(type_: &ParameterType) -> &ParameterType {
    match type_ {
        ParameterType::Res(inner) => inner,
        other => other,
    }
}

/// The type a numeric literal (negated or not) carries, for list-literal
/// element coercion; `None` for anything else.
fn numeric_literal_type(expression: &HirExpression) -> Option<ParameterType> {
    match expression {
        HirExpression::Number(number) => Some(match crate::numeric::classify_literal(number).1 {
            crate::numeric::LiteralType::Integer => ParameterType::Integer,
            crate::numeric::LiteralType::Float => ParameterType::Float,
            crate::numeric::LiteralType::Fixed => ParameterType::Fixed,
            crate::numeric::LiteralType::Money => ParameterType::Money,
        }),
        HirExpression::Unary {
            operator, operand, ..
        } if *operator == UnaryOp::Negate
            && matches!(operand.as_ref(), HirExpression::Number(_)) =>
        {
            numeric_literal_type(operand)
        }
        _ => None,
    }
}

/// Whether an argument is a non-negative integer literal that fits in a `Byte`.
fn expr_is_byte_literal(expression: &HirExpression) -> bool {
    matches!(expression, HirExpression::Number(text)
        if text.parse::<u16>().is_ok_and(|n| n <= u8::MAX as u16))
}

/// Resolve a table-driven builtin call, retrying with `Integer`-literal
/// arguments coerced to `Byte` when the exact-typed resolution fails (the
/// checker's `resolve_table_call_with_byte_literals`): each subset of the
/// eligible positions is tried, so a literal that is validly either `Integer`
/// or `Byte` resolves against whichever the overload expects.
fn resolve_table_call_with_byte_literals(
    callee: &str,
    arg_types: &[ParameterType],
    arguments: &[&HirExpression],
) -> Option<ParameterType> {
    // plan-111-B: typed throughout. `resolve_call_return_type_typed` (plan-104-C)
    // is the exact twin — it routes the three bespoke per-package resolvers
    // through the same string path they already had, and takes the generic
    // registry path with no strings at all. That the twin exists is what let
    // this conversion happen here rather than moving to letter C, which
    // plan-111-B §2 left open pending exactly that check.
    if let Some(return_type) = builtins::resolve_call_return_type_typed(callee, arg_types, true) {
        return Some(return_type);
    }
    let eligible: Vec<usize> = arg_types
        .iter()
        .enumerate()
        .filter(|(index, type_)| {
            matches!(type_, ParameterType::Integer)
                && arguments
                    .get(*index)
                    .is_some_and(|argument| expr_is_byte_literal(argument))
        })
        .map(|(index, _)| index)
        .collect();
    if eligible.is_empty() || eligible.len() > 6 {
        return None;
    }
    for mask in 1u32..(1u32 << eligible.len()) {
        let mut trial: Vec<ParameterType> = arg_types.to_vec();
        for (bit, &index) in eligible.iter().enumerate() {
            if mask & (1 << bit) != 0 {
                trial[index] = ParameterType::Byte;
            }
        }
        if let Some(return_type) = builtins::resolve_call_return_type_typed(callee, &trial, true) {
            return Some(return_type);
        }
    }
    None
}

/// How a block ends, as the source checker judged it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Flow {
    FallsThrough,
    AlwaysReturns,
}

/// The source line a statement reports at.
fn statement_line(statement: &HirStatement) -> usize {
    match statement {
        HirStatement::Let { line, .. }
        | HirStatement::Return { line, .. }
        | HirStatement::Exit { line, .. }
        | HirStatement::Continue { line, .. }
        | HirStatement::Fail { line, .. }
        | HirStatement::Propagate { line, .. }
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
    }
}

/// A compiler-owned record the language never constructs or updates from
/// source (the checker's `read_only_record_type`).
fn read_only_record(type_: &ParameterType) -> bool {
    if matches!(type_, ParameterType::MapEntryOf(..)) {
        return true;
    }
    // Both spellings, via `is_builtin_named` — see `verify::read_only_record_type`,
    // this rule's twin. bug-480 Phase 4b package-qualified builtin value types,
    // and matching the bare leaf alone silently stopped recognising the very
    // records this rule exists to protect (bug-483).
    crate::codegen::builtins::term::is_read_only_record(type_)
        || type_.is_builtin_named("net", crate::codegen::builtins::net::ADDRESS_TYPE)
        || type_.is_builtin_named("audio", crate::codegen::builtins::audio::AUDIO_DEVICE_TYPE)
}

/// A CONST pin expression the compiler folds to an immediate (plan-50-G): an
/// integer or boolean literal, `NOTHING`, `SIZEOF <CStruct>` of a struct the
/// owning LINK declares, or a sign applied to one of those.
fn link_const_foldable(expression: &crate::ast::Expression, cstructs: &[&str]) -> bool {
    use crate::ast::Expression;
    match expression {
        Expression::Number(_) | Expression::Boolean(_) => true,
        Expression::Identifier(name) => name == "NOTHING",
        Expression::Unary {
            operator, operand, ..
        } => match operator {
            UnaryOp::SizeOf => {
                matches!(operand.as_ref(), Expression::Identifier(name) if cstructs.contains(&name.as_str()))
            }
            UnaryOp::Negate => link_const_foldable(operand, cstructs),
            // A boolean negation is not a foldable integer constant; it reached
            // the catch-all `false` before the operator became an enum.
            UnaryOp::Not => false,
        },
        _ => false,
    }
}

/// An unsuffixed decimal literal that classifies as `Float` (a suffixed
/// `1.08f`/`1.08F`/`1.08m` is intrinsically typed and never the culprit); a
/// negated one counts.
fn is_bare_decimal_float(expression: &HirExpression) -> bool {
    match expression {
        HirExpression::Number(text) => {
            !text.ends_with(['f', 'F', 'm', 'M'])
                && matches!(
                    crate::numeric::classify_literal(text).1,
                    crate::numeric::LiteralType::Float
                )
        }
        HirExpression::Unary {
            operator, operand, ..
        } if *operator == UnaryOp::Negate => is_bare_decimal_float(operand),
        _ => false,
    }
}

/// The checker's `is_numeric` (`Unknown` included, so a prior error does not
/// cascade).
fn is_numeric(type_: &ParameterType) -> bool {
    matches!(
        type_,
        ParameterType::Byte
            | ParameterType::Fixed
            | ParameterType::Float
            | ParameterType::Integer
            | ParameterType::Money
            | ParameterType::Unknown
    )
}

/// Whether a value of `type_` can be rendered by `toString` for an assertion
/// failure message (`Unknown` is printable to avoid cascades).
fn is_printable(type_: &ParameterType) -> bool {
    match type_ {
        ParameterType::Integer
        | ParameterType::Float
        | ParameterType::Fixed
        | ParameterType::Money
        | ParameterType::Boolean
        | ParameterType::String
        | ParameterType::Byte
        | ParameterType::Unknown => true,
        ParameterType::Named(_) => type_.is_named("Scalar"),
        ParameterType::ListOf(inner) => matches!(**inner, ParameterType::Byte),
        _ => false,
    }
}

/// The call's argument values in source order.
fn source_order(arguments: &[HirCallArg]) -> Vec<&HirExpression> {
    arguments
        .iter()
        .map(|argument| match argument {
            HirCallArg::Positional(value) | HirCallArg::Named { value, .. } => value,
        })
        .collect()
}

impl<'a> Walker<'a> {
    fn new(
        project_dir: &'a Path,
        facts: &'a LowerFacts,
        hir: &HirProject,
        imported_types: &[ImportedTypeDef],
        imported_signatures: &'a HashMap<String, ExternalSignature>,
        imported_resource_types: &[String],
    ) -> Self {
        // plan-111-B: a resource type is a nominal, so the set holds types.
        let resource_types: HashSet<ParameterType> = super::lower_link::native_resources(hir)
            .iter()
            .map(|resource| ParameterType::declared(&resource.name))
            .chain(
                imported_resource_types
                    .iter()
                    .map(|name| ParameterType::declared(name)),
            )
            .collect();
        // Declared unions with their own variants and INCLUDES, for the
        // transitive expansion below (a `UNION B INCLUDES A` matches A's
        // variants too — the checker's and lowering's shared rule).
        let union_decls: HashMap<&str, (&crate::hir::HirTypeDecl, &str)> = hir
            .files
            .iter()
            .flat_map(|file| file.items.iter().map(move |item| (file, item)))
            .filter_map(|(file, item)| match item {
                HirItem::Type(type_decl) if type_decl.kind == crate::ast::TypeDeclKind::Union => {
                    Some((type_decl.name.as_str(), (type_decl, file.path.as_str())))
                }
                _ => None,
            })
            .collect();
        fn expanded_variants(
            name: &str,
            union_decls: &HashMap<&str, (&crate::hir::HirTypeDecl, &str)>,
            visiting: &mut HashSet<String>,
        ) -> Vec<String> {
            let Some((type_decl, _)) = union_decls.get(name) else {
                return Vec::new();
            };
            if !visiting.insert(name.to_string()) {
                return Vec::new();
            }
            let mut variants = Vec::new();
            for include in &type_decl.includes {
                variants.extend(expanded_variants(
                    include.name().as_ref(),
                    union_decls,
                    visiting,
                ));
            }
            variants.extend(
                type_decl
                    .variants
                    .iter()
                    .map(|variant| variant.type_.name().into_owned()),
            );
            visiting.remove(name);
            variants
        }
        let mut types = HashMap::new();
        for file in &hir.files {
            for item in &file.items {
                if let HirItem::Type(type_decl) = item {
                    let id = types.len();
                    let variants = if type_decl.kind == crate::ast::TypeDeclKind::Union {
                        expanded_variants(&type_decl.name, &union_decls, &mut HashSet::new())
                    } else {
                        Vec::new()
                    };
                    types.insert(
                        ParameterType::declared(&type_decl.name),
                        TypeShape {
                            id,
                            variants,
                            is_record: type_decl.kind == crate::ast::TypeDeclKind::Type,
                            is_union: type_decl.kind == crate::ast::TypeDeclKind::Union,
                            is_enum: type_decl.kind == crate::ast::TypeDeclKind::Enum,
                            fields: type_decl
                                .fields
                                .iter()
                                .map(|field| field.type_.clone())
                                .collect(),
                            variant_types: type_decl
                                .variants
                                .iter()
                                .map(|variant| variant.type_.clone())
                                .collect(),
                            members: type_decl
                                .members
                                .iter()
                                .map(|member| member.name.clone())
                                .collect(),
                            visibility: type_decl.visibility,
                            file: file.path.clone(),
                        },
                    );
                }
            }
        }
        for imported in imported_types {
            let id = types.len();
            let variants = if imported.kind == super::ImportedTypeKind::Union {
                imported
                    .variants
                    .iter()
                    .map(|variant| variant.name.clone())
                    .collect()
            } else {
                Vec::new()
            };
            types.insert(
                ParameterType::declared(&imported.name),
                TypeShape {
                    id,
                    variants,
                    is_record: imported.kind == super::ImportedTypeKind::Record,
                    is_union: imported.kind == super::ImportedTypeKind::Union,
                    is_enum: imported.kind == super::ImportedTypeKind::Enum,
                    fields: imported
                        .fields
                        .iter()
                        .map(|field| field.type_.clone())
                        .collect(),
                    variant_types: imported
                        .variants
                        .iter()
                        .flat_map(|variant| variant.fields.iter())
                        .map(|field| field.type_.clone())
                        .collect(),
                    members: imported.members.clone(),
                    visibility: Visibility::Export,
                    file: String::new(),
                },
            );
        }
        let mut functions = HashMap::new();
        for file in &hir.files {
            for item in &file.items {
                if let HirItem::Function(function) = item {
                    functions.insert(
                        function.name.clone(),
                        DeclaredFunction {
                            params: function
                                .params
                                .iter()
                                .map(|param| ShapeParam {
                                    name: param.name.clone(),
                                    type_: param.type_.clone(),
                                    has_default: param.default.is_some(),
                                })
                                .collect(),
                            visibility: function.visibility,
                            owner_file: file.path.clone(),
                            isolated: function.isolated,
                            kind: function.kind,
                        },
                    );
                }
            }
        }
        let astrings_imported = hir.files.iter().any(|file| {
            file.imports
                .iter()
                .any(|import| import.package_name() == "astrings")
        });
        // bug-466: the builtin records whose FIELDS need their own package's
        // `IMPORT`. A nominal the project itself declares (in a file the
        // compiler did not inject) or an imported `.mfp` exports means that
        // type, not the builtin's, so it is excluded — as is a name two builtin
        // packages both declare, which no single import would name.
        //
        // `shadowed` is read off the `types` table built above rather than
        // re-walking the HIR: that table already holds exactly these two
        // populations, keyed by the very `ParameterType` a lookup will use, so
        // the two cannot disagree about a key. An entry's `file` is the
        // declaring file, or empty for an imported type.
        let internal_files: HashSet<&str> = hir
            .files
            .iter()
            .filter(|file| file.internal)
            .map(|file| file.path.as_str())
            .collect();
        let shadowed: HashSet<&ParameterType> = types
            .iter()
            .filter(|(_, shape)| !internal_files.contains(shape.file.as_str()))
            .map(|(type_, _)| type_)
            .collect();
        let mut builtin_record_owner: HashMap<ParameterType, &'static str> = HashMap::new();
        let mut ambiguous: HashSet<ParameterType> = HashSet::new();
        for package in crate::codegen::registry::registry().packages() {
            for record in package.records() {
                // bug-480 Phase 4b: a builtin record's DECLARED identity is
                // package-qualified (`net.Address`), so that is the type a value
                // of it now carries and the key this map must use. Keyed bare, the
                // lookup missed and bug-466's gate silently stopped firing --
                // `tcp::localAddress(s).port` became readable without
                // `IMPORT net`.
                //
                // `named`, not `declared`: this is a constructor call from a
                // `&'static str`, not a grammar entry.
                let type_ =
                    ParameterType::named(&format!("{}.{}", package.import_name(), record.name));
                if shadowed.contains(&type_) {
                    continue;
                }
                if builtin_record_owner
                    .insert(type_.clone(), package.import_name())
                    .is_some()
                {
                    ambiguous.insert(type_);
                }
            }
        }
        for type_ in &ambiguous {
            builtin_record_owner.remove(type_);
        }
        Walker {
            project_dir,
            context: facts.context(),
            functions,
            imported_signatures,
            astrings_imported,
            current_is_sub: false,
            inline_trap_types: Vec::new(),
            resource_types,
            handler_depth: 0,
            types,
            file: String::new(),
            current_line: 0,
            call_typed_unknown: false,
            call_verdicts: HashMap::new(),
            state_dropped: HashSet::new(),
            builtin_record_owner,
            current_file_internal: false,
            current_own_imports: HashSet::new(),
            diagnostics: Vec::new(),
            #[cfg(test)]
            bound_types: Vec::new(),
        }
    }

    fn walk_project(&mut self, hir: &HirProject) {
        self.check_imported_packages(hir);
        for file in &hir.files {
            self.walk_file(file);
        }
    }

    /// PACKAGE_INVALID (plan-107-D row 20, an (I) relocation from the source
    /// checker's package collectors): an imported package's three metadata
    /// tables must decode, and every type its exported records/unions reference
    /// must be declared — here or in an imported package — with comparable map
    /// keys. The container as a whole is verified at the decode boundary
    /// (`verify_and_report_packages`, before any checker runs), so what the
    /// three read checks catch is an unreadable TABLE inside a well-formed
    /// container. The type walk lives here rather than at that boundary because
    /// it needs the full type table and resource registry this pass already
    /// owns (`is_comparable`). The checker walked types + resources per import
    /// and the function signatures in a second pass; the order is kept.
    fn check_imported_packages(&mut self, hir: &HirProject) {
        let mut seen_packages = HashSet::new();
        for file in &hir.files {
            self.file = file.path.clone();
            for import in &file.imports {
                let package = import.package_name();
                if package == crate::ast::SELF_IMPORT
                    || builtins::is_builtin_import(package)
                    || !seen_packages.insert(package.to_string())
                {
                    continue;
                }
                // bug-480: resolve the compiled interface through the shared
                // resolver, so a dependency declared by source directory (whose
                // `.mfp` this build compiled into `build/packages/`) is walked
                // exactly like an installed one instead of being skipped.
                let Some(package_file) =
                    crate::manifest::package::resolved_package_file(self.project_dir, package)
                else {
                    continue;
                };
                match crate::binary_repr::read_package_type_exports(&package_file) {
                    Ok(type_exports) => {
                        for export in &type_exports {
                            let context = match export.kind {
                                crate::binary_repr::BinaryReprExportKind::Type => {
                                    format!("exported type `{}`", export.name)
                                }
                                crate::binary_repr::BinaryReprExportKind::Union => {
                                    format!("exported union `{}`", export.name)
                                }
                                _ => continue,
                            };
                            self.validate_package_type(
                                &package_file,
                                &ParameterType::declared(&export.name),
                                &context,
                                import.line,
                                &mut HashSet::new(),
                            );
                        }
                    }
                    Err(_) => self.emit(
                        "PACKAGE_INVALID",
                        format!(
                            "Imported package `{package}` has unreadable or invalid type metadata."
                        ),
                        import.line,
                    ),
                }
                if crate::binary_repr::read_package_resources(&package_file).is_err() {
                    self.emit(
                        "PACKAGE_INVALID",
                        format!(
                            "Imported package `{}` has an unreadable resource table.",
                            package_file.display()
                        ),
                        import.line,
                    );
                }
            }
        }
        let mut seen_bindings = HashSet::new();
        for file in &hir.files {
            self.file = file.path.clone();
            for import in &file.imports {
                let package = import.package_name();
                if package == crate::ast::SELF_IMPORT
                    || builtins::is_builtin_import(package)
                    || !seen_bindings.insert(import.binding_name().to_string())
                {
                    continue;
                }
                // bug-480: resolve the compiled interface through the shared
                // resolver, so a dependency declared by source directory (whose
                // `.mfp` this build compiled into `build/packages/`) is walked
                // exactly like an installed one instead of being skipped.
                let Some(package_file) =
                    crate::manifest::package::resolved_package_file(self.project_dir, package)
                else {
                    continue;
                };
                match crate::binary_repr::read_package_exports(&package_file) {
                    Ok(exports) => {
                        for export in &exports {
                            if !matches!(
                                export.kind,
                                crate::binary_repr::BinaryReprExportKind::Func
                                    | crate::binary_repr::BinaryReprExportKind::Sub
                            ) {
                                continue;
                            }
                            // Every exported function's parameter and return
                            // types (the checker's
                            // `validate_imported_function_signature`).
                            let mut seen = HashSet::new();
                            for param in &export.params {
                                self.validate_package_type(
                                    &package_file,
                                    &param.type_,
                                    &format!(
                                        "exported function `{}` parameter `{}`",
                                        export.name, param.name
                                    ),
                                    import.line,
                                    &mut seen,
                                );
                            }
                            self.validate_package_type(
                                &package_file,
                                &export.return_type,
                                &format!("exported function `{}` return type", export.name),
                                import.line,
                                &mut seen,
                            );
                        }
                    }
                    Err(_) => self.emit(
                        "PACKAGE_INVALID",
                        format!(
                            "Imported package `{package}` has unreadable or invalid function metadata."
                        ),
                        import.line,
                    ),
                }
            }
        }
    }

    /// The checker's `validate_package_metadata_type`: every nominal a package
    /// type reaches must be declared (a resource or a built-in nominal is always
    /// in scope), and a map key must be comparable.
    fn validate_package_type(
        &mut self,
        package_file: &Path,
        type_: &ParameterType,
        context: &str,
        line: usize,
        seen: &mut HashSet<ParameterType>,
    ) {
        match type_ {
            ParameterType::ListOf(element)
            | ParameterType::SetOf(element)
            | ParameterType::ResultOf(element)
            | ParameterType::Res(element) => {
                self.validate_package_type(package_file, element, context, line, seen);
            }
            ParameterType::MapOf(key, value) => {
                self.validate_package_type(package_file, key, context, line, seen);
                self.validate_package_type(package_file, value, context, line, seen);
                if !self.is_comparable(key) {
                    self.emit(
                        "PACKAGE_INVALID",
                        format!(
                            "Imported package `{}` has {context} with non-comparable map key type `{}`.",
                            package_file.display(),
                            key.name()
                        ),
                        line,
                    );
                }
            }
            ParameterType::Func(params, return_type, _) => {
                for param in params {
                    self.validate_package_type(package_file, param, context, line, seen);
                }
                self.validate_package_type(package_file, return_type, context, line, seen);
            }
            ParameterType::ThreadHandle { msg, res, out, .. } => {
                self.validate_package_type(package_file, msg, context, line, seen);
                // An absent resource plane is `Nothing`; the plane's ` STATE T`
                // rides inside its spelling (plan-106-C rung 2e).
                let (plane_resource, plane_state) = res.split_state();
                if !matches!(plane_resource, ParameterType::Nothing) {
                    self.validate_package_type(package_file, &plane_resource, context, line, seen);
                }
                if let Some(plane_state) = &plane_state {
                    self.validate_package_type(package_file, plane_state, context, line, seen);
                }
                self.validate_package_type(package_file, out, context, line, seen);
            }
            ParameterType::Named(_) => {
                let name = type_.clone();
                // A built-in nominal is always in scope and declares no fields.
                if matches!(&name, ParameterType::Named(sym)
                    if matches!(sym.resolve(), "AttributedString" | "Error" | "ErrorLoc" | "Scalar"))
                {
                    return;
                }
                if self.is_resource_type(type_) || !seen.insert(name.clone()) {
                    return;
                }
                let Some(shape) = self.types.get(type_) else {
                    self.emit(
                        "PACKAGE_INVALID",
                        format!(
                            "Imported package `{}` has {context} that references unknown type `{name}`.",
                            package_file.display()
                        ),
                        line,
                    );
                    return;
                };
                let referenced = if shape.is_record {
                    shape.fields.clone()
                } else if shape.is_union {
                    shape.variant_types.clone()
                } else {
                    Vec::new()
                };
                for referenced in &referenced {
                    self.validate_package_type(package_file, referenced, context, line, seen);
                }
                seen.remove(&name);
            }
            // A stateful resource, as an imported signature spells it
            // (`Db STATE DbInfo`). plan-111-B: this arm is new because the
            // clause is now STRUCTURE. Before it, the type reached the `other`
            // arm below, was re-wrapped as one opaque `Named`, and the old
            // `is_resource_type(&str)` split the base back out of that
            // spelling — accepting the whole thing without walking the STATE
            // payload. `without_state` cannot peel a re-wrapped nominal, so the
            // peel has to happen before the re-wrap. Same outcome, same
            // non-recursion; a stateful type whose base is NOT a resource still
            // falls through and is reported by its full spelling.
            ParameterType::Stateful { .. } if self.is_resource_type(type_) => {}
            ParameterType::Boolean
            | ParameterType::Byte
            | ParameterType::Fixed
            | ParameterType::Float
            | ParameterType::Integer
            | ParameterType::Money
            | ParameterType::Nothing
            | ParameterType::String
            | ParameterType::Unknown => {}
            // Every other spelling took the checker's nominal arm.
            other => self.validate_package_type(
                package_file,
                &ParameterType::named(&other.name()),
                context,
                line,
                seen,
            ),
        }
    }

    fn walk_file(&mut self, file: &HirFile) {
        self.context.current_imports = file.import_bindings();
        self.context.current_file = file.path.clone();
        self.file = file.path.clone();
        self.current_file_internal = file.internal;
        self.current_own_imports = file.own_imports.iter().cloned().collect();
        for item in &file.items {
            match item {
                HirItem::Binding(binding) => {
                    if let Some(value) = &binding.value {
                        let locals = HashMap::new();
                        self.current_line = binding.line;
                        self.walk_expression(value, &locals);
                        self.check_initializer_known(
                            &binding.name,
                            value,
                            &locals,
                            binding.explicit_type.then_some(&binding.type_),
                            binding.line,
                        );
                    }
                }
                HirItem::Function(function) => self.walk_function(function),
                HirItem::Link(link) => self.walk_link(link),
                // Declarations without executable bodies: their rules are
                // `ir::verify`'s (types, LINK blocks, resources) or the parser's.
                HirItem::Type(_)
                | HirItem::Resource(_)
                | HirItem::FuncAlias(_)
                | HirItem::Doc(_)
                | HirItem::Testing(_) => {}
            }
        }
    }

    fn walk_function(&mut self, function: &HirFunction) {
        // Parameter locals as `lower_function` seeds them: a `RES` parameter's
        // `STATE T` rides in its type.
        let mut locals = HashMap::new();
        for param in &function.params {
            let type_ = match &param.state_type {
                Some(state) => param.type_.with_state(state),
                None => param.type_.clone(),
            };
            if let Some(default) = &param.default {
                self.current_line = param.line;
                self.walk_expression(default, &locals);
                if self.checker_types_unknown(default, &locals, Some(&param.type_)) {
                    self.emit(
                        "TYPE_UNKNOWN_VALUE",
                        format!(
                            "Default value for `{}` does not have a known type.",
                            param.name
                        ),
                        param.line,
                    );
                }
            }
            locals.insert(param.name.clone(), type_);
        }
        let previous_return_type = self.context.current_return_type.take();
        self.context.current_return_type = Some(lower::function_return_type(function));
        self.state_dropped.clear();
        self.current_is_sub = function.kind == FunctionKind::Sub;
        // The body's top-level locals stay visible to the function-level
        // TRAP body (bug-285), exactly as `lower_function_body` scopes them.
        let mut body_locals = locals.clone();
        self.walk_statements_flow(&function.body, &mut body_locals);
        if let Some(trap) = &function.trap {
            let mut trap_locals = body_locals;
            trap_locals.insert(trap.name.clone(), ParameterType::named("Error"));
            self.walk_block(&trap.body, &trap_locals);
        }
        self.context.current_return_type = previous_return_type;
    }

    /// The two native-ABI facts lowering erases from a `LINK` block (every
    /// other native rule is `ir::verify`'s, plan-107-C):
    ///
    /// - NATIVE_CONST_UNKNOWN_SLOT, the "not a constant the compiler can fold"
    ///   form: lowering folds a CONST pin's expression to its immediate
    ///   (`eval_link_const`), so the IR holds a number where the source held
    ///   `"literal"` or `1 + 1`. (The unknown-slot form is verify's.)
    /// - NATIVE_FREE_INVALID, the deallocator-signature form: `IrFree` keeps
    ///   only the freed slot and the symbol; the `RETURN`ed slot, the return
    ///   ctype and the deallocator's parameter/return ctypes are gone. (The
    ///   `AS RES` producer form and the empty-symbol form are verify's — both
    ///   end the checker's FREE check, so neither doubles with this one.)
    fn walk_link(&mut self, link: &crate::hir::HirLinkBlock) {
        let cstructs: Vec<&str> = link.cstructs.iter().map(|c| c.name.as_str()).collect();
        for function in &link.functions {
            for pin in &function.consts {
                if !link_const_foldable(&pin.value, &cstructs) {
                    self.emit(
                        "NATIVE_CONST_UNKNOWN_SLOT",
                        format!(
                            "Native function `{}` CONST pin `{}` is not a constant the compiler can fold: it must be an integer or boolean literal, NOTHING, or SIZEOF <CStruct>.",
                            function.name, pin.slot
                        ),
                        pin.line,
                    );
                }
            }
            let Some(free) = &function.free else {
                continue;
            };
            if function.return_resource || free.symbol.is_empty() {
                continue;
            }
            // The freed slot must be the C return, that return must be what
            // `RETURN` surfaces, it must be a CPtr copied into an owned wrapper
            // value, and the deallocator takes one CPtr and returns CVoid.
            let returns_the_c_value = matches!(
                &function.result,
                Some(crate::ast::Expression::Identifier(name)) if *name == function.abi.return_name
            );
            let well_formed = free.slot == function.abi.return_name
                && returns_the_c_value
                && function.abi.return_ctype == "CPtr"
                && free.param_ctype == "CPtr"
                && free.return_ctype == "CVoid";
            if !well_formed {
                self.emit(
                    "NATIVE_FREE_INVALID",
                    format!(
                        "Native function `{}` has a malformed FREE block: it must name the CPtr produced slot that `RETURN` surfaces, and its deallocator must take one CPtr parameter and return CVoid.",
                        function.name
                    ),
                    free.line,
                );
            }
        }
    }

    /// Walk a block in a scope of its own — `lower_statement_block` clones the
    /// enclosing locals per block, so a binding never leaks out of it.
    fn walk_block(&mut self, body: &[HirStatement], locals: &HashMap<String, ParameterType>) {
        self.walk_block_flow(body, locals);
    }

    /// Walk a block and report how it ends, the way the source checker's
    /// `check_block` did: the first diverging statement ends the walk (the
    /// checker checked nothing after it), and when that statement is an `EXIT
    /// SUB`/`EXIT FUNC`/`EXIT PROGRAM` every statement after it is
    /// UNREACHABLE_AFTER_EXIT — those three lower to a bare Return, to
    /// nothing, and to an `ExitProgram` op that `ir::verify`'s loop-exit form
    /// does not treat as an exit, so the IR cannot report them (the `EXIT
    /// FOR`/`DO`/`WHILE`/`CONTINUE` forms are verify's). Inside an inline-TRAP
    /// handler every exit form is shape's: `treeify_handler` drops the
    /// statements after any terminator before lowering sees them.
    fn walk_block_flow(
        &mut self,
        body: &[HirStatement],
        locals: &HashMap<String, ParameterType>,
    ) -> Flow {
        let mut nested = locals.clone();
        self.walk_statements_flow(body, &mut nested)
    }

    /// `walk_block_flow` over a scope the caller keeps (a function body's
    /// top-level locals feed its TRAP body).
    fn walk_statements_flow(
        &mut self,
        body: &[HirStatement],
        locals: &mut HashMap<String, ParameterType>,
    ) -> Flow {
        for (index, statement) in body.iter().enumerate() {
            let flow = self.walk_statement(statement, locals);
            if flow == Flow::AlwaysReturns {
                let erased_exit = match statement {
                    HirStatement::Exit {
                        target: ExitTarget::Sub | ExitTarget::Func | ExitTarget::Program,
                        ..
                    } => true,
                    HirStatement::Exit { .. } | HirStatement::Continue { .. } => {
                        self.handler_depth > 0
                    }
                    _ => false,
                };
                if erased_exit {
                    for unreachable in &body[index + 1..] {
                        self.emit(
                            "UNREACHABLE_AFTER_EXIT",
                            "Statement is unreachable after EXIT or CONTINUE.".to_string(),
                            statement_line(unreachable),
                        );
                    }
                }
                return Flow::AlwaysReturns;
            }
        }
        Flow::FallsThrough
    }

    /// The checker's verdict on whether a block always diverges — for the
    /// inline-TRAP handler rule and for the walk order above.
    fn statement_flow(&self, statement: &HirStatement) -> Flow {
        match statement {
            HirStatement::Return { .. }
            | HirStatement::Exit { .. }
            | HirStatement::Continue { .. }
            | HirStatement::Fail { .. }
            | HirStatement::Propagate { .. }
            | HirStatement::Recover { .. } => Flow::AlwaysReturns,
            HirStatement::If {
                then_body,
                else_body,
                ..
            } => {
                if self.block_flow(then_body) == Flow::AlwaysReturns
                    && self.block_flow(else_body) == Flow::AlwaysReturns
                {
                    Flow::AlwaysReturns
                } else {
                    Flow::FallsThrough
                }
            }
            HirStatement::Match { cases, .. } => {
                // Exhaustiveness is judged as the checker did: an unguarded
                // CASE ELSE, or every variant/member of the scrutinee's type
                // named by an unguarded case. The scrutinee type is not
                // needed here — a MATCH whose every case diverges but whose
                // coverage cannot be established falls through, which is
                // what the checker answered for an untyped scrutinee too.
                let all_return = !cases.is_empty()
                    && cases
                        .iter()
                        .all(|case| self.block_flow(&case.body) == Flow::AlwaysReturns);
                if all_return && self.match_covered(cases) {
                    Flow::AlwaysReturns
                } else {
                    Flow::FallsThrough
                }
            }
            _ => Flow::FallsThrough,
        }
    }

    fn block_flow(&self, body: &[HirStatement]) -> Flow {
        for statement in body {
            if self.statement_flow(statement) == Flow::AlwaysReturns {
                return Flow::AlwaysReturns;
            }
        }
        Flow::FallsThrough
    }

    /// Whether a MATCH's unguarded cases cover its scrutinee (a `CASE ELSE`,
    /// or every variant of the union / member of the enum the cases name).
    fn match_covered(&self, cases: &[HirMatchCase]) -> bool {
        use crate::hir::HirMatchPattern;
        let mut covered: HashSet<String> = HashSet::new();
        for case in cases {
            if case.guard.is_some() {
                continue;
            }
            match &case.pattern {
                HirMatchPattern::Else => return true,
                HirMatchPattern::Union { type_, .. } => {
                    covered.insert(type_.name().into_owned());
                }
                HirMatchPattern::Literal(HirExpression::MemberAccess { target, member }) => {
                    if let HirExpression::Identifier(type_name) = target.as_ref() {
                        covered.insert(format!("{type_name}::{member}"));
                    }
                }
                _ => {}
            }
        }
        // The cases name one type; its declaration decides the full set.
        let Some(first) = covered.iter().next() else {
            return false;
        };
        if let Some((type_name, _)) = first.split_once("::") {
            return self
                .types
                .get(&ParameterType::declared(type_name))
                .is_some_and(|info| {
                    !info.members.is_empty()
                        && info
                            .members
                            .iter()
                            .all(|member| covered.contains(&format!("{type_name}::{member}")))
                });
        }
        self.types.values().any(|info| {
            info.is_union
                && info
                    .variants
                    .iter()
                    .any(|variant| covered.contains(variant))
                && info
                    .variants
                    .iter()
                    .all(|variant| covered.contains(variant))
        })
    }

    fn walk_statement(
        &mut self,
        statement: &HirStatement,
        locals: &mut HashMap<String, ParameterType>,
    ) -> Flow {
        self.current_line = statement_line(statement);
        let flow = self.statement_flow(statement);
        match statement {
            HirStatement::Let {
                resource,
                state_type,
                name,
                type_,
                explicit_type,
                value,
                line,
                ..
            } => {
                let declared_type = explicit_type.then(|| type_.clone());
                if let Some(HirExpression::Trapped {
                    expression,
                    binding,
                    handler,
                    line: trap_line,
                }) = value
                {
                    // The inline-TRAP form: the binding takes the trapped
                    // expression's success type (`lower_inline_trap`).
                    let success_type = declared_type
                        .clone()
                        .or_else(|| lower::expression_type(expression, locals, &self.context))
                        .unwrap_or(ParameterType::Unknown);
                    let success_type = match (&declared_type, state_type) {
                        (Some(declared_type), Some(state)) => declared_type.with_state(state),
                        _ => success_type,
                    };
                    self.walk_expression(expression, locals);
                    self.check_trap_short_circuit(expression, locals, *trap_line);
                    let trapped_type = self.type_of(expression, locals);
                    self.walk_handler(binding, handler, locals, trapped_type, *trap_line);
                    // The checker typed the binding by the trapped call.
                    self.check_initializer_known(
                        name,
                        expression,
                        locals,
                        declared_type.as_ref(),
                        *line,
                    );
                    self.bind(name, success_type, locals);
                    return flow;
                }
                let lowered_type = declared_type.clone().unwrap_or_else(|| {
                    value
                        .as_ref()
                        .and_then(|value| lower::expression_type(value, locals, &self.context))
                        .unwrap_or(ParameterType::Unknown)
                });
                let lowered_type = match state_type {
                    Some(state) => lowered_type.with_state(state),
                    None => lowered_type,
                };
                if let Some(value) = value {
                    self.walk_expression(value, locals);
                    self.check_initializer_known(
                        name,
                        value,
                        locals,
                        declared_type.as_ref(),
                        *line,
                    );
                }
                if !*resource && lowered_type.state().is_some() {
                    self.state_dropped.insert(name.clone());
                } else {
                    self.state_dropped.remove(name);
                }
                self.bind(name, lowered_type, locals);
            }
            HirStatement::Return { value, line } => {
                // SUB_RETURN_FORBIDDEN, the bare form: `RETURN` in a SUB lowers
                // to the same bare `Return` op as `EXIT SUB`, so only the HIR
                // knows which the source wrote (the valued form is verify's).
                if self.current_is_sub && value.is_none() {
                    self.emit(
                        "SUB_RETURN_FORBIDDEN",
                        "A SUB returns no value; use `EXIT SUB`.".to_string(),
                        *line,
                    );
                }
                if let Some(value) = value {
                    self.walk_expression(value, locals);
                    // TYPE_UNKNOWN_VALUE, the RETURN form (see
                    // `check_initializer_known`); the declared return type is the
                    // expectation the checker inferred the value under.
                    let expected = self.context.current_return_type.clone();
                    if self.checker_types_unknown(value, locals, expected.as_ref()) {
                        self.emit(
                            "TYPE_UNKNOWN_VALUE",
                            "RETURN value does not have a known type.".to_string(),
                            *line,
                        );
                    }
                }
            }
            HirStatement::Exit { target, code, line } => {
                match target {
                    // EXIT FOR/DO/WHILE outside a matching loop is `ir::verify`'s.
                    ExitTarget::For | ExitTarget::Do | ExitTarget::While => {}
                    // EXIT_SUB_IN_FUNC: `EXIT SUB` lowers to a bare `Return`,
                    // which a FUNC's IR cannot tell from a fall-through — the
                    // statement's own kind is gone.
                    ExitTarget::Sub => {
                        if !self.current_is_sub {
                            self.emit(
                                "EXIT_SUB_IN_FUNC",
                                "EXIT SUB is valid only inside a SUB; use RETURN <value> in a FUNC."
                                    .to_string(),
                                *line,
                            );
                        }
                    }
                    // EXIT_FUNC_FORBIDDEN: `ExitTarget::Func` lowers to NOTHING —
                    // the fact does not exist in the IR.
                    ExitTarget::Func => {
                        self.emit(
                            "EXIT_FUNC_FORBIDDEN",
                            "Functions must RETURN a value; EXIT FUNC is not allowed.".to_string(),
                            *line,
                        );
                    }
                    ExitTarget::Program => {}
                }
                if let Some(code) = code {
                    self.walk_expression(code, locals);
                }
            }
            HirStatement::Continue { .. } | HirStatement::Propagate { .. } => {}
            HirStatement::Fail { error, .. } => self.walk_expression(error, locals),
            HirStatement::Recover { value, line } => {
                if let Some(value) = value {
                    self.walk_expression(value, locals);
                }
                // TYPE_RECOVER_OUTSIDE_INLINE_TRAP: a stray RECOVER lowers to a
                // `$recover_stray` bind (plan-107-B) — the statement is gone.
                let Some(recover_type) = self.inline_trap_types.last().cloned() else {
                    self.emit(
                        "TYPE_RECOVER_OUTSIDE_INLINE_TRAP",
                        "RECOVER is valid only inside an inline TRAP handler.".to_string(),
                        *line,
                    );
                    return flow;
                };
                // TYPE_RECOVER_TYPE_MISMATCH, the two count forms: lowering
                // stores a RECOVER value into the trap slot only when both exist
                // (a valueless RECOVER for a value-producing trap and a value for
                // a value-less one lower to nothing / an `Eval`), so the IR
                // keeps no trace of the mismatch; the value-TYPE form is verify's.
                let produces_value = !matches!(recover_type, ParameterType::Nothing);
                match (value, produces_value) {
                    (None, true) => self.emit(
                        "TYPE_RECOVER_TYPE_MISMATCH",
                        format!(
                            "RECOVER must supply a {} value for the trapped expression.",
                            recover_type.name()
                        ),
                        *line,
                    ),
                    (Some(_), false) => self.emit(
                        "TYPE_RECOVER_TYPE_MISMATCH",
                        "RECOVER must not supply a value for a value-less trapped expression."
                            .to_string(),
                        *line,
                    ),
                    _ => {}
                }
            }
            HirStatement::Assign { name, value, line } => {
                // TYPE_UNKNOWN_VALUE, the assignment-target form: a target that
                // is neither a local nor a top-level binding. Lowering emits an
                // `AssignGlobal` for any non-local name, so the IR does not
                // know the name resolved to nothing.
                if !locals.contains_key(name) && self.context.binding_type(name).is_none() {
                    self.emit(
                        "TYPE_UNKNOWN_VALUE",
                        format!("Assignment target `{name}` is not a local binding."),
                        *line,
                    );
                }
                self.walk_value(value, locals);
            }
            HirStatement::StateAssign {
                resource,
                value,
                line,
            } => {
                // TYPE_UNKNOWN_VALUE, the state-assignment form: `res.state = …`
                // needs a LOCAL resource binding (a file-PRIVATE top-level
                // resource arrives mangled and is reported by its source name).
                if !locals.contains_key(resource) {
                    self.emit(
                        "TYPE_UNKNOWN_VALUE",
                        format!(
                            "State assignment target `{}` is not a local binding.",
                            crate::internal_name::display_name(resource)
                        ),
                        *line,
                    );
                }
                self.walk_value(value, locals);
            }
            HirStatement::Expression { expression, .. } => {
                self.walk_value(expression, locals);
            }
            HirStatement::If {
                condition,
                then_body,
                else_body,
                ..
            } => {
                self.walk_expression(condition, locals);
                self.walk_block(then_body, locals);
                self.walk_block(else_body, locals);
            }
            HirStatement::Match {
                expression, cases, ..
            } => {
                let matched_type = lower::match_expression_type(expression, locals, &self.context)
                    .unwrap_or(ParameterType::Unknown);
                self.walk_expression(expression, locals);
                for case in cases {
                    self.walk_match_case(case, &matched_type, locals);
                }
            }
            HirStatement::For {
                name,
                start,
                end,
                step,
                body,
                ..
            } => {
                // The loop variable's type is the promoted numeric type of the
                // three bounds, as lowering computes it.
                let start_type = lower::expression_type(start, locals, &self.context)
                    .unwrap_or(ParameterType::Unknown);
                let end_type = lower::expression_type(end, locals, &self.context)
                    .unwrap_or(ParameterType::Unknown);
                let step_type = step
                    .as_ref()
                    .and_then(|value| lower::expression_type(value, locals, &self.context))
                    .unwrap_or(ParameterType::Integer);
                let loop_type = crate::numeric::typed_promote_loop_numeric_type(
                    &start_type,
                    &end_type,
                    &step_type,
                );
                self.walk_expression(start, locals);
                self.walk_expression(end, locals);
                if let Some(step) = step {
                    self.walk_expression(step, locals);
                }
                let mut nested = locals.clone();
                self.bind(name, loop_type, &mut nested);
                self.walk_block(body, &nested);
            }
            HirStatement::ForEach {
                name,
                iterable,
                body,
                ..
            } => {
                let iterable_type = lower::expression_type(iterable, locals, &self.context)
                    .unwrap_or(ParameterType::Unknown);
                let element_type = lower::collection_iteration_type(&iterable_type)
                    .unwrap_or(ParameterType::Unknown);
                self.walk_expression(iterable, locals);
                let mut nested = locals.clone();
                self.bind(name, element_type, &mut nested);
                self.walk_block(body, &nested);
            }
            HirStatement::While {
                condition, body, ..
            } => {
                self.walk_expression(condition, locals);
                self.walk_block(body, locals);
            }
            HirStatement::DoUntil {
                body, condition, ..
            } => {
                self.walk_block(body, locals);
                self.walk_expression(condition, locals);
            }
        }
        flow
    }

    /// A statement-position value: an inline-TRAP form walks its handler in the
    /// binding's scope; anything else is an ordinary expression.
    fn walk_value(&mut self, value: &HirExpression, locals: &HashMap<String, ParameterType>) {
        if let HirExpression::Trapped {
            expression,
            binding,
            handler,
            line,
        } = value
        {
            self.walk_expression(expression, locals);
            self.check_trap_short_circuit(expression, locals, *line);
            let trapped_type = self.type_of(expression, locals);
            self.walk_handler(binding, handler, locals, trapped_type, *line);
            return;
        }
        self.walk_expression(value, locals);
    }

    /// TYPE_INLINE_TRAP_SHORT_CIRCUIT_CALL (bug-457): a fallible call in a
    /// **short-circuited** operand of the trapped expression.
    ///
    /// `lower_inline_trap` covers a nested fallible call by lifting it into its
    /// own `CallResult` check ahead of the residual expression. That is sound
    /// only where the call is evaluated unconditionally: `AND`/`OR` evaluate
    /// their right operand only when the left one does not already decide the
    /// result (`mfb spec language operators`), so lifting a call out of one
    /// would call it every time. Reporting is the alternative to leaving the
    /// error to escape the handler unnoticed, which is the bug this rule closes
    /// the last corner of; the author binds the call to its own `LET … TRAP`
    /// first, which is what the desugar would have had to do anyway.
    ///
    /// Lowering erases the evidence — the operand structure is gone by the time
    /// `ir::verify` sees the lifted chain — so the rule lives here.
    fn check_trap_short_circuit(
        &mut self,
        expression: &HirExpression,
        locals: &HashMap<String, ParameterType>,
        line: usize,
    ) {
        let mut offender = None;
        self.find_short_circuited_call(expression, locals, false, &mut offender);
        if let Some(callee) = offender {
            self.emit(
                "TYPE_INLINE_TRAP_SHORT_CIRCUIT_CALL",
                format!(
                    "Inline TRAP cannot cover `{callee}`: AND/OR evaluate their right operand \
                     only conditionally, so it cannot be lifted ahead of the expression. \
                     Bind it to its own LET with a TRAP first, then use that value here."
                ),
                line,
            );
        }
    }

    /// Records a raising operator sitting in a short-circuited operand (bug-471).
    ///
    /// `lower_inline_trap` covers a raising operator by lifting it into its own
    /// `Checked` bind ahead of the residual expression — the same lift, and the
    /// same restriction, as a fallible call: an operand `AND`/`OR` evaluates only
    /// conditionally cannot be hoisted, because hoisting evaluates it every time.
    /// Reporting it is the alternative to letting the division-by-zero escape the
    /// handler unnoticed, which is the whole of bug-471.
    fn note_short_circuited_binary_operator(
        &self,
        expression: &HirExpression,
        operator: BinaryOp,
        locals: &HashMap<String, ParameterType>,
        conditional: bool,
        offender: &mut Option<String>,
    ) {
        if offender.is_some() || !conditional {
            return;
        }
        let type_ = self.type_of(expression, locals);
        if super::fallible::operator_can_raise(operator, &type_) {
            *offender = Some(operator.name().to_string());
        }
    }

    /// The unary half of [`Self::note_short_circuited_binary_operator`]. Split
    /// by arity because the raise-set is: before the operator became an enum
    /// both arities shared one `&str` list in which `"-"` stood for subtraction
    /// and negation at once, so which one a lookup meant depended on the caller.
    fn note_short_circuited_unary_operator(
        &self,
        expression: &HirExpression,
        operator: UnaryOp,
        locals: &HashMap<String, ParameterType>,
        conditional: bool,
        offender: &mut Option<String>,
    ) {
        if offender.is_some() || !conditional {
            return;
        }
        let type_ = self.type_of(expression, locals);
        // The same exemption `lower::trap_hoist_kind` applies: a unary `-` over a
        // numeric literal is the spelling of a negative literal and cannot raise,
        // so `t AND -1 > 0` must not be reported. Kept in step with the lift by
        // reading the one predicate both sides share.
        if let HirExpression::Unary { operand, .. } = expression {
            if matches!(operand.as_ref(), HirExpression::Number(_))
                && super::fallible::is_total_literal_negation(operator, &type_)
            {
                return;
            }
        }
        if super::fallible::unary_operator_can_raise(operator, &type_) {
            *offender = Some(operator.name().to_string());
        }
    }

    /// Records the first fallible call — or, since bug-471, raising **operator**
    /// — reached through a short-circuited operand. `conditional` is true once
    /// the walk has entered the right side of an `AND`/`OR`; a lambda body is
    /// skipped because it runs at the callback's call site, not in this
    /// expression.
    fn find_short_circuited_call(
        &self,
        expression: &HirExpression,
        locals: &HashMap<String, ParameterType>,
        conditional: bool,
        offender: &mut Option<String>,
    ) {
        if offender.is_some() {
            return;
        }
        match expression {
            HirExpression::String(_)
            | HirExpression::Number(_)
            | HirExpression::Scalar(_)
            | HirExpression::Boolean(_)
            | HirExpression::Identifier(_)
            | HirExpression::Lambda { .. } => {}
            HirExpression::Binary {
                left,
                operator,
                right,
                ..
            } => {
                self.find_short_circuited_call(left, locals, conditional, offender);
                let short_circuit = lower::is_short_circuit_operator(*operator);
                self.find_short_circuited_call(
                    right,
                    locals,
                    conditional || short_circuit,
                    offender,
                );
                self.note_short_circuited_binary_operator(
                    expression,
                    *operator,
                    locals,
                    conditional,
                    offender,
                );
            }
            HirExpression::Unary {
                operand, operator, ..
            } => {
                self.find_short_circuited_call(operand, locals, conditional, offender);
                self.note_short_circuited_unary_operator(
                    expression,
                    *operator,
                    locals,
                    conditional,
                    offender,
                );
            }
            HirExpression::Call {
                callee, arguments, ..
            } => {
                for argument in arguments {
                    match argument {
                        HirCallArg::Positional(value) | HirCallArg::Named { value, .. } => {
                            self.find_short_circuited_call(value, locals, conditional, offender)
                        }
                    }
                }
                if offender.is_none()
                    && conditional
                    && self
                        .context
                        .call_is_fallible(&self.canonical_callee(callee))
                {
                    *offender = Some(callee.replace('.', "::"));
                }
            }
            HirExpression::Constructor { arguments, .. } => {
                for argument in arguments {
                    match argument {
                        HirConstructorArg::Positional(value)
                        | HirConstructorArg::Named { value, .. } => {
                            self.find_short_circuited_call(value, locals, conditional, offender)
                        }
                    }
                }
            }
            HirExpression::WithUpdate { target, updates } => {
                self.find_short_circuited_call(target, locals, conditional, offender);
                for update in updates {
                    self.find_short_circuited_call(&update.value, locals, conditional, offender);
                }
            }
            HirExpression::ListLiteral(values) => {
                for value in values {
                    self.find_short_circuited_call(value, locals, conditional, offender);
                }
            }
            HirExpression::SetLiteral { elements, .. } => {
                for element in elements {
                    self.find_short_circuited_call(element, locals, conditional, offender);
                }
            }
            HirExpression::MapLiteral { entries, .. } => {
                for (key, value) in entries {
                    self.find_short_circuited_call(key, locals, conditional, offender);
                    self.find_short_circuited_call(value, locals, conditional, offender);
                }
            }
            HirExpression::MemberAccess { target, .. } => {
                self.find_short_circuited_call(target, locals, conditional, offender)
            }
            // A nested inline TRAP routes its own expression's errors to its own
            // handler, so nothing inside it escapes into this one.
            HirExpression::Trapped { .. } => {}
        }
    }

    /// TYPE_UNKNOWN_VALUE, the foreign-record field form (bug-466). Reading a
    /// field off a record a BUILTIN package declares requires that package's own
    /// `IMPORT` in this file. Imports are not transitive and a package cannot
    /// re-export another's types, so `tcp::localAddress`'s `net::Address` result
    /// has no field table in a file that imported only `tcp` — the read has no
    /// type, and the value it produces is `Unknown`.
    ///
    /// **Why here and not in `ir::verify`, and why not by asking whether the
    /// read typed.** The `Unknown` was already caught wherever it was BOUND
    /// (`LET p AS Integer = b.port`) or fed to an operator, but not where it was
    /// a call argument: a declared `Integer` parameter accepted it, and an
    /// overloaded builtin unified it against a candidate rather than failing
    /// resolution. It then survived to native lowering, which asserted its own
    /// invariant with no source location to attach — `native plan has no storage
    /// class for type 'Unknown'`, or, one call away from the mistake, `native
    /// code field access target 'Address' is not a record or variant while
    /// lowering eval call io.print`. Gating the FIELD ACCESS ITSELF makes the
    /// verdict independent of where the `Unknown` happens to land, so a position
    /// nobody thought to check cannot reopen the hole.
    ///
    /// And it keys on the file's own imports rather than on whether the read
    /// resolved, because whether it resolved was never a property of this file:
    /// the type table is populated by whichever companion sources got injected,
    /// and `udp`'s `Datagram` names a `net.Address`, so adding an unreferenced
    /// `IMPORT udp` used to drag `Address`'s declaration in and make the
    /// identical `tcp` program compile. Same program, different verdict, decided
    /// by an unrelated import. This asks the registry instead, so both spellings
    /// are refused.
    ///
    /// `.state` is excluded: it is not a record field, and its own rules
    /// (`ir::verify`'s TYPE_STATE_INVALID, and the `state_dropped` cascade
    /// below) already report it more precisely.
    fn check_foreign_record_field(
        &mut self,
        target: &HirExpression,
        member: &str,
        locals: &HashMap<String, ParameterType>,
    ) {
        if self.current_file_internal || member == "state" {
            return;
        }
        let target_type = strip_res(&self.type_of(target, locals)).without_state();
        let Some(owner) = self.builtin_record_owner.get(&target_type) else {
            return;
        };
        if self.current_own_imports.contains(*owner) {
            return;
        }
        let type_name = target_type.name();
        self.emit(
            "TYPE_UNKNOWN_VALUE",
            format!(
                "Field `{member}` belongs to `{owner}::{type_name}`, whose fields are not visible in this file. Imports are not transitive and a package cannot re-export another's types, so add `IMPORT {owner}`."
            ),
            self.current_line,
        );
    }

    /// An inline-TRAP handler block: the error binding is an `Error` local
    /// visible only inside it (`lower_inline_trap`).
    fn walk_handler(
        &mut self,
        binding: &str,
        handler: &[HirStatement],
        locals: &HashMap<String, ParameterType>,
        trapped_type: ParameterType,
        trap_line: usize,
    ) {
        let mut handler_locals = locals.clone();
        handler_locals.insert(binding.to_string(), ParameterType::named("Error"));
        self.inline_trap_types.push(trapped_type);
        self.handler_depth += 1;
        let handler_flow = self.walk_block_flow(handler, &handler_locals);
        self.handler_depth -= 1;
        self.inline_trap_types.pop();
        // TYPE_INLINE_TRAP_FALLS_THROUGH: the handler's fall-through edge is a
        // source shape — lowering emits the handler as the `If`'s else arm and
        // nothing marks where a path ended without RECOVER.
        if handler_flow != Flow::AlwaysReturns {
            self.emit(
                "TYPE_INLINE_TRAP_FALLS_THROUGH",
                "Inline TRAP handler must end every path in RECOVER or a diverging statement (RETURN, FAIL, or PROPAGATE)."
                    .to_string(),
                trap_line,
            );
        }
    }

    fn walk_match_case(
        &mut self,
        case: &HirMatchCase,
        matched_type: &ParameterType,
        locals: &HashMap<String, ParameterType>,
    ) {
        use crate::hir::HirMatchPattern;
        match &case.pattern {
            HirMatchPattern::Else | HirMatchPattern::Union { .. } => {}
            HirMatchPattern::Literal(expression) => self.walk_expression(expression, locals),
            HirMatchPattern::OneOf(expressions) => {
                for expression in expressions {
                    self.walk_expression(expression, locals);
                }
            }
        }
        let mut case_locals = locals.clone();
        // The matched local's name is irrelevant to the binding's TYPE (it only
        // names the extract's source); the pattern and scrutinee type decide it.
        // The checker bound a variant pattern only when the scrutinee is a
        // union declaring that variant (a `CASE Ok`/`CASE Error` arm, a
        // non-union scrutinee or an unknown variant leaves the name unbound,
        // so a read of it types `Unknown` — the cascade those fixtures pin).
        if self.checker_binds_pattern(&case.pattern, matched_type) {
            if let Some((binding, binding_type, _)) =
                lower::match_case_binding(&case.pattern, "$match", matched_type)
            {
                case_locals.insert(binding, binding_type);
            }
        }
        if let Some(guard) = &case.guard {
            self.walk_expression(guard, &case_locals);
        }
        self.walk_block(&case.body, &case_locals);
    }

    fn walk_expression(
        &mut self,
        expression: &HirExpression,
        locals: &HashMap<String, ParameterType>,
    ) {
        match expression {
            HirExpression::String(_)
            | HirExpression::Number(_)
            | HirExpression::Scalar(_)
            | HirExpression::Boolean(_)
            | HirExpression::Identifier(_) => {}
            HirExpression::Binary {
                left,
                operator,
                right,
                ..
            } => {
                self.walk_expression(left, locals);
                self.walk_expression(right, locals);
                // MONEY_INEXACT_FLOAT_LITERAL (Warn, plan-29-F §4.6): scaling a
                // Money by a BARE decimal literal (`* 1.08` / `/ 1.08`) takes the
                // inexact Float path. The literal's spelling is the evidence —
                // lowering stamps the same `Float` const for `1.08` and `1.08f`.
                let money = |walker: &Self, value: &HirExpression| {
                    matches!(walker.type_of(value, locals), ParameterType::Money)
                };
                let bare_float = |walker: &Self, value: &HirExpression| {
                    matches!(walker.type_of(value, locals), ParameterType::Float)
                        && is_bare_decimal_float(value)
                };
                let culprit = match operator {
                    BinaryOp::Multiply => {
                        (money(self, left) && bare_float(self, right))
                            || (money(self, right) && bare_float(self, left))
                    }
                    BinaryOp::Divide => money(self, left) && bare_float(self, right),
                    // Only a scaling operator can silently make Money inexact:
                    // `+`/`-` require two Money operands and the rest are not
                    // Money operators at all.
                    BinaryOp::Or
                    | BinaryOp::Xor
                    | BinaryOp::And
                    | BinaryOp::Equal
                    | BinaryOp::NotEqual
                    | BinaryOp::Less
                    | BinaryOp::LessEqual
                    | BinaryOp::Greater
                    | BinaryOp::GreaterEqual
                    | BinaryOp::Concat
                    | BinaryOp::Add
                    | BinaryOp::Subtract
                    | BinaryOp::Mod
                    | BinaryOp::IntDiv
                    | BinaryOp::Power => false,
                };
                if culprit {
                    self.emit(
                        "MONEY_INEXACT_FLOAT_LITERAL",
                        "scaling Money by a bare decimal literal uses inexact Float arithmetic; append `F` for exact fixed-point scaling, or `f` to confirm the Float is intentional.".to_string(),
                        self.current_line,
                    );
                }
            }
            HirExpression::Unary { operand, .. } => self.walk_expression(operand, locals),
            HirExpression::Call {
                callee,
                arguments,
                line,
                ..
            } => {
                // The call's own shape rules report before its arguments are
                // walked — the source checker normalized the argument list
                // before inferring any argument, so a nested call's rule follows
                // the enclosing call's.
                self.call_typed_unknown = false;
                self.check_call_shape(callee, arguments, *line, locals);
                self.call_verdicts.insert(
                    expression as *const HirExpression as usize,
                    self.call_typed_unknown,
                );
                for argument in arguments {
                    match argument {
                        HirCallArg::Positional(value) | HirCallArg::Named { value, .. } => {
                            self.walk_expression(value, locals)
                        }
                    }
                }
            }
            HirExpression::Lambda {
                params,
                body,
                assign_target,
            } => {
                // Lambda parameters shadow the enclosing scope for the body
                // (`expression_type`'s Lambda arm).
                let mut nested = locals.clone();
                for param in params {
                    nested.insert(param.name.clone(), param.type_.clone());
                }
                // TYPE_UNKNOWN_VALUE, the assignment-bodied lambda's target form.
                if let Some(target) = assign_target {
                    if !nested.contains_key(target) {
                        self.emit(
                            "TYPE_UNKNOWN_VALUE",
                            format!("Assignment target `{target}` is not a local binding."),
                            self.current_line,
                        );
                    }
                }
                self.walk_expression(body, &nested);
            }
            HirExpression::Constructor { type_, arguments } => {
                // TYPE_READ_ONLY_RECORD_CONSTRUCTOR, the `Error`/`ErrorLoc` form:
                // lowering itself emits `Constructor{Error}` for the `error()`
                // builtin and the trap machinery, so on the IR a user `Error[..]`
                // is indistinguishable from a legitimate synthesized one (A
                // Corrections C-split-49). verify keeps the compiler-owned form
                // and the `AttributedString` form.
                let type_name = type_.name();
                if matches!(type_name.as_ref(), "Error" | "ErrorLoc") {
                    self.emit(
                        "TYPE_READ_ONLY_RECORD_CONSTRUCTOR",
                        format!(
                            "`{type_name}` is a read-only built-in record and cannot be constructed; use `error(code, message)` to create an Error."
                        ),
                        self.current_line,
                    );
                }
                // TYPE_DUPLICATE_FIELD, the constructor form: lowering reorders
                // the named arguments into field order (the last spelling of a
                // repeated field wins), so the IR holds one value per field and
                // the repetition is gone. The checker checked it only for a
                // declared, visible record (`check_constructor_arguments` — the
                // built-in nominals and compiler-owned records return before
                // it); verify keeps the WITH form.
                if self.declared_record_constructible(type_) {
                    let mut seen = HashSet::new();
                    for argument in arguments {
                        if let HirConstructorArg::Named { name, line, .. } = argument {
                            if !seen.insert(name.as_str()) {
                                self.emit(
                                    "TYPE_DUPLICATE_FIELD",
                                    format!(
                                        "Constructor `{}` sets field `{name}` more than once.",
                                        type_.name()
                                    ),
                                    *line,
                                );
                            }
                        }
                    }
                }
                for argument in arguments {
                    match argument {
                        HirConstructorArg::Positional(value)
                        | HirConstructorArg::Named { value, .. } => {
                            self.walk_expression(value, locals)
                        }
                    }
                }
            }
            HirExpression::WithUpdate { target, updates } => {
                self.walk_expression(target, locals);
                for update in updates {
                    self.walk_expression(&update.value, locals);
                }
            }
            HirExpression::ListLiteral(elements) | HirExpression::SetLiteral { elements, .. } => {
                for element in elements {
                    self.walk_expression(element, locals);
                }
            }
            HirExpression::MapLiteral { entries, .. } => {
                for (key, value) in entries {
                    self.walk_expression(key, locals);
                    self.walk_expression(value, locals);
                }
            }
            HirExpression::MemberAccess { target, member } => {
                self.walk_expression(target, locals);
                self.check_foreign_record_field(target, member, locals);
            }
            HirExpression::Trapped {
                expression,
                binding,
                handler,
                line,
            } => {
                self.walk_expression(expression, locals);
                let trapped_type = self.type_of(expression, locals);
                self.walk_handler(binding, handler, locals, trapped_type, *line);
            }
        }
    }

    /// The parameter names of the function a call names, resolved the way the
    /// source checker resolved a call target: a TESTING expectation or a package
    /// constant is not a function call; a builtin (by canonical name) comes
    /// before any declared function; a declared function must be visible from
    /// the calling file; an imported package's function is looked up under its
    /// canonical `package.member` name; last, a local or global binding of
    /// FUNC type is a function value. Anything else (an unresolved dotted name,
    /// a non-callable binding) has no signature to check against.
    fn callee_params(
        &self,
        callee: &str,
        locals: &HashMap<String, ParameterType>,
    ) -> Option<CalleeParams> {
        if crate::codegen::builtins_testing::is_testing_call(callee) {
            return None;
        }
        let binding = callee.split_once('.').map(|(binding, _)| binding);
        let resolved_package =
            binding.and_then(|binding| self.context.current_imports.get(binding));
        let canonical = self.canonical_callee(callee);
        if builtins::is_package_constant(&canonical) {
            return None;
        }
        if builtins::is_builtin_call(&canonical) {
            if !builtins::checks_call_arguments(&canonical) {
                return None;
            }
            if let Some(overloads) = builtins::call_param_name_overloads(&canonical) {
                return Some(CalleeParams::BuiltinOverloads(overloads));
            }
            return Some(match builtins::call_param_names(&canonical) {
                Some(names) => CalleeParams::Builtin(names),
                None => CalleeParams::BuiltinUnnamed,
            });
        }
        let declared = self
            .functions
            .get(callee)
            .or_else(|| self.functions.get(&canonical))
            .filter(|function| match function.visibility {
                Visibility::Export | Visibility::Public => true,
                Visibility::Private => function.owner_file == self.file,
            });
        if let Some(function) = declared {
            return Some(CalleeParams::Declared(function.params.clone()));
        }
        // An imported package's function: only through an import binding of
        // this file (a dotted name whose prefix is not an import is not a call
        // into a package, whatever the dependency table happens to hold).
        if resolved_package.is_some() {
            if let Some(signature) = self.imported_signatures.get(&canonical) {
                return Some(CalleeParams::Declared(
                    signature
                        .params
                        .iter()
                        .map(|param| ShapeParam {
                            name: param.name.clone(),
                            type_: param.type_.clone(),
                            has_default: param.has_default,
                        })
                        .collect(),
                ));
            }
        }
        // A dotted name that resolved to nothing is not a call to a binding.
        if binding.is_some() {
            return None;
        }
        let value_type = locals
            .get(callee)
            .or_else(|| self.context.binding_type(callee))?;
        match value_type {
            ParameterType::Func(params, _, _) => Some(CalleeParams::FunctionValue(params.clone())),
            _ => None,
        }
    }

    /// The canonical `package.member` spelling of a call target, through this
    /// file's import bindings (`IMPORT self` binds the package's own exports
    /// under their bare names) — lowering's `canonical_import_name`.
    fn canonical_callee(&self, callee: &str) -> String {
        let Some((binding, member)) = callee.split_once('.') else {
            return callee.to_string();
        };
        match self.context.current_imports.get(binding) {
            Some(package) if package == crate::ast::SELF_IMPORT => member.to_string(),
            Some(package) => format!("{package}.{member}"),
            None => callee.to_string(),
        }
    }

    /// Whether a `thread::start` entry argument names an exported ISOLATED
    /// FUNC of an imported package — through an import binding (`pkg::f`, the
    /// `.mfp`'s export table) or the package's own `self::` binding (an
    /// `EXPORT ISOLATED FUNC` of this project). A bare project function is not
    /// an import, whatever it declares. The checker resolved the NORMALIZED
    /// first argument, so `entry` is that.
    fn thread_start_entry_valid(&self, entry: Option<&HirExpression>) -> bool {
        let Some(HirExpression::Identifier(name)) = entry else {
            return false;
        };
        let Some((binding, member)) = name.split_once('.') else {
            return false;
        };
        match self.context.current_imports.get(binding) {
            Some(package) if package == crate::ast::SELF_IMPORT => {
                self.functions.get(member).is_some_and(|function| {
                    function.visibility == Visibility::Export
                        && function.isolated
                        && function.kind == crate::ast::FunctionKind::Func
                })
            }
            Some(package) => self
                .imported_signatures
                .get(&format!("{package}.{member}"))
                .is_some_and(|signature| signature.isolated && !signature.sub),
            None => false,
        }
    }

    /// TYPE_CALL_ARGUMENT_MISMATCH, the `thread.start` entry form — a source
    /// fact (`self::` vs a bare name both lower to one `FunctionRef`).
    fn report_thread_entry(&mut self, line: usize) {
        self.call_typed_unknown = true;
        self.emit(
            "TYPE_CALL_ARGUMENT_MISMATCH",
            "thread.start entry point must be an exported ISOLATED FUNC from an imported package."
                .to_string(),
            line,
        );
    }

    /// The builtin-call family on the source path — the former source checker's
    /// `check_builtin_call` over the HIR argument list: the count against the
    /// registry's arity range, then the argument types through the same
    /// arg-typed overload resolution the checker used (the `general`,
    /// `collections`, `term` and `thread` arms ahead of the package table).
    /// Every type here is lowering's `expression_type` of the argument the
    /// SOURCE wrote — the list lowering then pads with defaults and coerces
    /// literal by literal, which is why the IR cannot report this form with
    /// the checker's wording (`ir::verify` keeps the IR-level check for the
    /// package path).
    fn check_builtin_call(
        &mut self,
        callee: &str,
        canonical: &str,
        normalized: &[&HirExpression],
        line: usize,
        locals: &HashMap<String, ParameterType>,
    ) {
        let registry = crate::codegen::registry::registry();
        let arg_types: Vec<ParameterType> = normalized
            .iter()
            .map(|argument| self.type_of(argument, locals).without_state())
            .collect();
        let names: Vec<String> = arg_types
            .iter()
            .map(|type_| type_.name().into_owned())
            .collect();
        let expected_overloads = || {
            builtins::expected_arguments(canonical)
                .unwrap_or_else(|| "supported overload".to_string())
        };
        let mismatch = |names: &[String], expected: String| {
            format!(
                "Call to `{callee}` has argument type(s) ({}), expected {expected}.",
                names.join(", ")
            )
        };

        if crate::codegen::builtins::general::is_general_call(canonical) {
            if self.check_builtin_arity(callee, canonical, normalized.len(), line) {
                return;
            }
            if builtins::resolve_call_return_type_typed(canonical, &arg_types, true).is_none() {
                // A package-provided override may accept what the built-in
                // rejects (plan-01-overload §A.3.2) — never reject those.
                if crate::codegen::builtins::general::is_overridable(canonical)
                    && arg_types.len() == 1
                    && builtins::general_override_target(canonical, &arg_types[0]).is_some()
                {
                    return;
                }
                let detail = mismatch(&names, expected_overloads());
                self.emit_call_typed_unknown(detail, line);
            }
            return;
        }

        if registry.owning_package(canonical) == Some("collections") {
            // A bare general built-in predicate in the callback position
            // (bug-368): its type derives from the list's element type, and the
            // diagnostic quotes the predicate's NAME.
            if crate::codegen::registry::callback_member(canonical) && normalized.len() == 2 {
                if let HirExpression::Identifier(predicate) = normalized[1] {
                    if crate::codegen::builtins::general::builtin_function_id(predicate).is_some() {
                        let collection = arg_types[0].clone();
                        // plan-111-C: the typed twin, so the predicate type is a
                        // `Func` rather than a `format!`ed spelling.
                        let predicate_type = match &arg_types[0] {
                            ParameterType::ListOf(element) => {
                                crate::codegen::builtins::general::filter_predicate_type_typed(
                                    predicate, element,
                                )
                            }
                            _ => None,
                        };
                        let Some(predicate_type) = predicate_type else {
                            let detail = mismatch(
                                &[collection.name().into_owned(), predicate.clone()],
                                expected_overloads(),
                            );
                            self.emit_call_typed_unknown(detail, line);
                            return;
                        };
                        let trial = vec![collection, predicate_type];
                        if builtins::resolve_call_return_type_typed(canonical, &trial, true)
                            .is_none()
                        {
                            let names: Vec<String> =
                                trial.iter().map(|t| t.name().into_owned()).collect();
                            let detail = mismatch(&names, expected_overloads());
                            self.emit_call_typed_unknown(detail, line);
                        }
                        return;
                    }
                }
            }
            if self.check_builtin_arity(callee, canonical, normalized.len(), line) {
                return;
            }
            if builtins::resolve_call_return_type_typed(canonical, &arg_types, true).is_none() {
                let detail = mismatch(&names, expected_overloads());
                self.emit_call_typed_unknown(detail, line);
            }
            return;
        }

        if registry.owning_package(canonical) == Some("term") {
            if self.check_builtin_arity(callee, canonical, normalized.len(), line) {
                // `term` types by name alone, so a count failure leaves the call typed.
                self.call_typed_unknown = false;
                return;
            }
            // `term::drawText` additionally accepts an `AttributedString` at the
            // text position; the source-companion overload honours its
            // attributes. Its body lives in a bridge injected only when the
            // project imports `astrings`, so the import is required here — a
            // source fact the IR does not keep.
            let third_is_attributed = canonical == crate::codegen::builtins::term::DRAW_TEXT
                && names.len() == 3
                && names[2] == "AttributedString";
            if third_is_attributed && !self.astrings_imported {
                self.emit(
                    "TYPE_CALL_ARGUMENT_MISMATCH",
                    format!(
                        "Call to `{callee}` with an `AttributedString` requires `IMPORT astrings`."
                    ),
                    line,
                );
                return;
            }
            // plan-111-B: `argument_types` is literally
            // `argument_types_typed(..).map(name())`, so the typed twin is exact
            // and the per-argument parse below disappears.
            let param_types: Vec<ParameterType> = if third_is_attributed {
                vec![
                    ParameterType::Integer,
                    ParameterType::Integer,
                    ParameterType::named("AttributedString"),
                ]
            } else {
                builtins::argument_types_typed(canonical).unwrap_or_default()
            };
            let mismatched = param_types
                .iter()
                .zip(arg_types.iter())
                .zip(normalized.iter())
                .any(|((expected, actual), argument)| {
                    !self.expression_compatible(expected, actual, argument)
                });
            if mismatched {
                let expected = builtins::expected_arguments(canonical)
                    .unwrap_or_else(|| "no arguments".to_string());
                let detail = mismatch(&names, expected);
                self.emit("TYPE_CALL_ARGUMENT_MISMATCH", detail, line);
            }
            return;
        }

        if crate::codegen::builtins::thread::is_thread_call(canonical) {
            if self.check_builtin_arity(callee, canonical, normalized.len(), line) {
                return;
            }
            if builtins::resolve_call_return_type_typed(canonical, &arg_types, true).is_none() {
                let detail = mismatch(&names, expected_overloads());
                self.emit_call_typed_unknown(detail, line);
            }
            return;
        }

        // The shared package table: arity, then arg-typed overload resolution
        // with the literal→`Byte` retry the checker gave the table packages.
        if self.check_builtin_arity(callee, canonical, normalized.len(), line) {
            return;
        }
        if resolve_table_call_with_byte_literals(canonical, &arg_types, normalized).is_none() {
            let detail = mismatch(&names, expected_overloads());
            self.emit_call_typed_unknown(detail, line);
        }
    }

    /// Lowering's type for an argument the source wrote; `Unknown` when it
    /// has none — the checker's own spelling for it. Callers strip a resource
    /// local's `STATE T` clause (`without_state`): the checker compared and
    /// printed the bare resource type, as the parameter tables spell it.
    fn type_of(
        &self,
        expression: &HirExpression,
        locals: &HashMap<String, ParameterType>,
    ) -> ParameterType {
        lower::expression_type(expression, locals, &self.context).unwrap_or(ParameterType::Unknown)
    }

    /// TESTING_EXPECT_ARITY / TYPE_MISMATCH / INCOMPARABLE / NOT_PRINTABLE /
    /// CODE_TYPE / TRAP_REQUIRES_FALLIBLE: an assertion builtin's argument
    /// constraints (plan-18-B). Lowering expands `expectX(...)` into plain
    /// comparisons + FAIL, or a trap-guarded evaluation (`testing::expand_expect`,
    /// `lower_statement`), so the IR never sees the assertion — only the HIR
    /// call names it. The rules and their order are the source checker's
    /// `check_expect_call`.
    fn check_expect_call(
        &mut self,
        callee: &str,
        arguments: &[HirCallArg],
        line: usize,
        locals: &HashMap<String, ParameterType>,
    ) {
        use crate::codegen::builtins_testing::{
            expect_arity, expect_operand_type, is_equality_assert, is_inequality_assert,
            EXPECT_NTRAP, EXPECT_TRAP,
        };
        if let Some((min, max)) = expect_arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                self.emit(
                    "TESTING_EXPECT_ARITY",
                    format!(
                        "`{callee}` expects {} argument(s), got {}.",
                        if min == max {
                            min.to_string()
                        } else {
                            format!("{min}\u{2013}{max}")
                        },
                        arguments.len()
                    ),
                    line,
                );
            }
        }
        let values = source_order(arguments);
        // The checker's verdict on an operand: its inferred type, or `Unknown`
        // where the checker could not type it (see `checker_types_unknown`).
        let operand = |walker: &Self, index: usize| {
            values.get(index).map_or(ParameterType::Unknown, |value| {
                if walker.checker_types_unknown(value, locals, None) {
                    ParameterType::Unknown
                } else {
                    walker.type_of(value, locals)
                }
            })
        };
        if is_equality_assert(callee) || is_inequality_assert(callee) {
            let left = operand(self, 0);
            let right = operand(self, 1);
            match expect_operand_type(callee) {
                // A typed assertion requires both operands to be exactly the
                // named type.
                Some(want) => {
                    for operand in [&left, &right] {
                        if !matches!(operand, ParameterType::Unknown) && operand.name() != want {
                            self.emit(
                                "TESTING_EXPECT_TYPE_MISMATCH",
                                format!(
                                    "`{callee}` operands must both be {want}; got {}.",
                                    operand.name()
                                ),
                                line,
                            );
                        }
                    }
                }
                // `expectEqual`/`expectNEqual` accept any `=`-comparable,
                // printable operands (the language's `=` acceptance).
                None => {
                    let comparable = (is_numeric(&left) && is_numeric(&right))
                        || ((self.compatible(&left, &right) || self.compatible(&right, &left))
                            && self.is_comparable(&left)
                            && self.is_comparable(&right));
                    if !comparable
                        && !matches!(left, ParameterType::Unknown)
                        && !matches!(right, ParameterType::Unknown)
                    {
                        self.emit(
                            "TESTING_EXPECT_INCOMPARABLE",
                            format!(
                                "`{callee}` operands must be comparable with `=`; got {} and {}.",
                                left.name(),
                                right.name()
                            ),
                            line,
                        );
                    }
                    for operand in [&left, &right] {
                        if !is_printable(operand) {
                            self.emit(
                                "TESTING_EXPECT_NOT_PRINTABLE",
                                format!(
                                    "`{callee}` operands must be printable (a scalar, String, Byte, or List OF Byte); got {}.",
                                    operand.name()
                                ),
                                line,
                            );
                        }
                    }
                }
            }
        } else if callee == EXPECT_TRAP {
            if let Some(value) = values.first() {
                self.check_trap_guardable(callee, value, line);
            }
            if values.get(1).is_some() {
                let code = operand(self, 1);
                if !self.compatible(&ParameterType::Integer, &code) {
                    self.emit(
                        "TESTING_EXPECT_CODE_TYPE",
                        format!(
                            "`{callee}` expected-code argument must be an Integer; got {}.",
                            code.name()
                        ),
                        line,
                    );
                }
            }
        } else if callee == EXPECT_NTRAP {
            if let Some(value) = values.first() {
                self.check_trap_guardable(callee, value, line);
            }
        }
    }

    /// `expectTrap`/`expectNTrap` evaluate their argument under a trap guard
    /// built on the inline-TRAP machinery, so the gate rejects exactly what an
    /// inline `TRAP` rejects (plan-26-C): a scrutinee with no runtime call to
    /// trap — a non-call, or a package constant.
    fn check_trap_guardable(&mut self, callee: &str, expression: &HirExpression, line: usize) {
        let HirExpression::Call {
            callee: inner_callee,
            ..
        } = expression
        else {
            self.emit(
                "TESTING_EXPECT_TRAP_REQUIRES_FALLIBLE",
                format!("`{callee}` requires a call to trap-guard (got a non-call)."),
                line,
            );
            return;
        };
        if builtins::is_package_constant(&self.canonical_callee(inner_callee)) {
            self.emit(
                "TESTING_EXPECT_TRAP_REQUIRES_FALLIBLE",
                format!(
                    "`{callee}` requires a call to trap-guard; a package constant is not a call."
                ),
                line,
            );
        }
    }

    /// Whether a type is an `=`-comparable operand, as the source checker
    /// judged it (the former source checker's `is_comparable_with_seen`): primitives,
    /// `Error`/`ErrorLoc`/`Scalar`, enums, records of comparable fields, and any
    /// nominal it cannot resolve; never a collection, a callable, a result, a
    /// thread handle, a resource, a union or `AttributedString`.
    fn is_comparable(&self, type_: &ParameterType) -> bool {
        self.is_comparable_seen(type_, &mut HashSet::new())
    }

    fn is_comparable_seen(&self, type_: &ParameterType, seen: &mut HashSet<ParameterType>) -> bool {
        match type_ {
            ParameterType::Boolean
            | ParameterType::Byte
            | ParameterType::Fixed
            | ParameterType::Float
            | ParameterType::Integer
            | ParameterType::Money
            | ParameterType::Nothing
            | ParameterType::String
            | ParameterType::Unknown => true,
            ParameterType::ListOf(_)
            | ParameterType::SetOf(_)
            | ParameterType::MapOf(_, _)
            | ParameterType::Func(..)
            | ParameterType::ResultOf(_)
            | ParameterType::Res(_)
            | ParameterType::ThreadHandle { .. } => false,
            ParameterType::Named(_) => {
                // plan-111-B: four nominals, four interned-`Symbol` compares.
                if type_.is_named("Error") || type_.is_named("ErrorLoc") || type_.is_named("Scalar")
                {
                    return true;
                }
                // Wraps a list overlay (like `List`), plan-89-A.
                if type_.is_named("AttributedString") {
                    return false;
                }
                let name = type_.clone();
                if self.is_resource_type(type_) || !seen.insert(name.clone()) {
                    return false;
                }
                let Some(shape) = self.types.get(type_) else {
                    return true;
                };
                let result = if shape.is_enum {
                    true
                } else if shape.is_record {
                    shape
                        .fields
                        .iter()
                        .all(|field| self.is_comparable_seen(field, seen))
                } else {
                    false
                };
                seen.remove(&name);
                result
            }
            // A stateful resource is a resource, so it is not comparable —
            // plan-111-B: the same trap as `validate_package_type`'s `Stateful`
            // arm. The `other` arm below re-wraps into one opaque nominal, and
            // the structural `without_state` cannot peel a re-wrapped spelling,
            // so a `Db STATE DbInfo` would have fallen through to the
            // unknown-type tail and been called COMPARABLE. Peel before the
            // re-wrap. A stateful type whose base is not a resource still falls
            // through, exactly as its opaque nominal did.
            ParameterType::Stateful { .. } if self.is_resource_type(type_) => false,
            // The checker routed every other spelling through its nominal arm.
            other => self.is_comparable_seen(&ParameterType::named(&other.name()), seen),
        }
    }

    /// The checker's resource registry: builtin resources plus the native and
    /// imported ones this project declares/imports (an imported resource is
    /// spelled `binding.Type` in source; its table row carries the bare name).
    /// A stateful resource carries its ` STATE T` clause inside the spelling
    /// (`SoundFile STATE SoundInfo`); recognition keys on the base name.
    fn is_resource_type(&self, type_: &ParameterType) -> bool {
        // plan-111-B: the STATE peel is structural (`without_state`); the
        // builtin close table is registry surface that still speaks names, so
        // the base renders for that one lookup and for the bare-name fallback
        // (an imported resource is spelled `binding.Type` in source while its
        // table row carries the bare name).
        let base = type_.without_state();
        let base_name = base.name();
        crate::codegen::resource::builtin_resource_close_function(&base).is_some()
            || self.resource_types.contains(&base)
            || base_name.rsplit_once('.').is_some_and(|(_, bare)| {
                self.resource_types.contains(&ParameterType::declared(bare))
            })
    }

    /// Type compatibility as the source checker judged it (the former source checker's `compatible`):
    /// `Unknown` on either side is compatible; the `RES` marker is stripped;
    /// containers, thread handles and callable types recurse (parameters
    /// contravariant); a union accepts any of its variants; a qualified nominal
    /// equates to its bare form unless both name distinct declarations.
    fn compatible(&self, expected: &ParameterType, actual: &ParameterType) -> bool {
        if matches!(expected, ParameterType::Unknown) || matches!(actual, ParameterType::Unknown) {
            return true;
        }
        // The `RES` marker and a resource's ` STATE T` clause are both
        // ownership-axis annotations the checker compared without.
        let expected = strip_res(expected).without_state();
        let actual = strip_res(actual).without_state();
        match (&expected, &actual) {
            (ParameterType::ListOf(expected), ParameterType::ListOf(actual))
            | (ParameterType::SetOf(expected), ParameterType::SetOf(actual))
            | (ParameterType::ResultOf(expected), ParameterType::ResultOf(actual)) => {
                self.compatible(expected, actual)
            }
            (
                ParameterType::MapOf(expected_key, expected_value),
                ParameterType::MapOf(actual_key, actual_value),
            ) => {
                self.compatible(expected_key, actual_key)
                    && self.compatible(expected_value, actual_value)
            }
            (
                ParameterType::ThreadHandle {
                    worker: expected_worker,
                    msg: expected_message,
                    res: expected_resource,
                    out: expected_output,
                },
                ParameterType::ThreadHandle {
                    worker: actual_worker,
                    msg: actual_message,
                    res: actual_resource,
                    out: actual_output,
                },
            ) => {
                expected_worker == actual_worker
                    && self.compatible(expected_message, actual_message)
                    && self.compatible(expected_resource, actual_resource)
                    && self.compatible(expected_output, actual_output)
            }
            (
                ParameterType::Func(expected_params, expected_return, expected_isolated),
                ParameterType::Func(actual_params, actual_return, actual_isolated),
            ) => {
                (!expected_isolated || *actual_isolated)
                    && expected_params.len() == actual_params.len()
                    && expected_params
                        .iter()
                        .zip(actual_params.iter())
                        // Parameters are contravariant (bug-173 A).
                        .all(|(expected, actual)| self.compatible(actual, expected))
                    && self.compatible(expected_return, actual_return)
            }
            (ParameterType::Named(_), ParameterType::Named(_)) => {
                let (expected_name, actual_name) = (expected.name(), actual.name());
                if expected_name == actual_name {
                    return true;
                }
                let expected_bare = expected_name.rsplit('.').next().unwrap_or(&expected_name);
                let actual_bare = actual_name.rsplit('.').next().unwrap_or(&actual_name);
                let expected_info = self
                    .types
                    .get(&expected)
                    .or_else(|| self.types.get(&ParameterType::declared(expected_bare)));
                // A union accepts any of its variant values; a variant may be
                // spelled qualified (`fs.File`) or bare.
                if expected_info.is_some_and(|info| {
                    info.variants
                        .iter()
                        .any(|variant| variant == actual_name.as_ref() || variant == actual_bare)
                }) {
                    return true;
                }
                if expected_bare != actual_bare {
                    return false;
                }
                // Shared bare names unify only when both resolve to the SAME
                // declaration (bug-41), or when either side is unregistered (a
                // built-in nominal such as `net.Url`).
                let actual_info = self
                    .types
                    .get(&actual)
                    .or_else(|| self.types.get(&ParameterType::declared(actual_bare)));
                match (expected_info, actual_info) {
                    (Some(expected_info), Some(actual_info)) => expected_info.id == actual_info.id,
                    _ => true,
                }
            }
            _ => expected == actual,
        }
    }

    /// the former source checker's `expression_compatible`: `compatible`, plus the literal
    /// coercions a constant argument enjoys — an in-range integer literal into
    /// `Byte`, a numeric literal (negated or not) into `Fixed`/`Money`, and a
    /// list literal of such literals into a list of them.
    fn expression_compatible(
        &self,
        expected: &ParameterType,
        actual: &ParameterType,
        expression: &HirExpression,
    ) -> bool {
        if self.compatible(expected, actual) {
            return true;
        }
        match (expected, actual, expression) {
            (ParameterType::Byte, ParameterType::Integer, HirExpression::Number(value)) => value
                .parse::<u16>()
                .is_ok_and(|number| number <= u8::MAX as u16),
            (
                ParameterType::Fixed | ParameterType::Money,
                ParameterType::Integer | ParameterType::Float,
                HirExpression::Number(_),
            ) => true,
            (
                ParameterType::Fixed | ParameterType::Money,
                ParameterType::Integer | ParameterType::Float,
                HirExpression::Unary {
                    operator, operand, ..
                },
            ) if *operator == UnaryOp::Negate
                && matches!(operand.as_ref(), HirExpression::Number(_)) =>
            {
                true
            }
            (
                ParameterType::ListOf(expected_element),
                ParameterType::ListOf(_),
                HirExpression::ListLiteral(values),
            ) => values.iter().all(|value| {
                let Some(actual_element) = numeric_literal_type(value) else {
                    return false;
                };
                self.expression_compatible(expected_element, &actual_element, value)
            }),
            _ => false,
        }
    }

    /// TYPE_CALL_ARITY_MISMATCH, the builtin count form: the checker counted
    /// the normalized argument list (names bound, unknown names dropped,
    /// extras kept) against the registry's arity range. True when the count
    /// was out of range (the checker ended the call's checks there).
    fn check_builtin_arity(
        &mut self,
        callee: &str,
        canonical: &str,
        count: usize,
        line: usize,
    ) -> bool {
        let Some((min, max)) = builtins::arity(canonical) else {
            return false;
        };
        if count < min || count > max {
            let expected = if min == max {
                min.to_string()
            } else {
                format!("{min} to {max}")
            };
            self.emit(
                "TYPE_CALL_ARITY_MISMATCH",
                format!("Call to `{callee}` has {count} argument(s), expected {expected}."),
                line,
            );
            self.call_typed_unknown = true;
            return true;
        }
        false
    }

    /// The call-shape rules — the ones lowering's argument normalization
    /// erases, so the evidence exists only in the HIR:
    ///
    /// - TYPE_UNKNOWN_ARGUMENT_NAME / TYPE_DUPLICATE_ARGUMENT_NAME: a name
    ///   that binds to no parameter (or binds one twice) is silently dropped by
    ///   lowering, so the lowered `IrValue::Call` carries no trace of it.
    /// - TYPE_CALL_ARITY_MISMATCH, the declared-function form: lowering drops
    ///   the positional arguments a signature cannot take and fills every
    ///   omitted slot from its default, so the count the source wrote is gone.
    /// - TYPE_CALL_ARITY_MISMATCH, the builtin and function-value count forms:
    ///   lowering pads a builtin's optional trailing arguments with their
    ///   defaults (the fixed-ABI runtime helpers take a full list) and appends
    ///   the extras, so the count the source wrote is gone from the IR; the
    ///   named-argument omission forms of a builtin call (a parameter left
    ///   unsupplied before a later named one, a name set no overload takes) go
    ///   the same way — lowering binds what it can and moves on.
    ///
    /// A builtin's argument TYPES survive lowering and are `ir::verify`'s.
    fn check_call_shape(
        &mut self,
        callee: &str,
        arguments: &[HirCallArg],
        line: usize,
        locals: &HashMap<String, ParameterType>,
    ) {
        if crate::codegen::builtins_testing::is_testing_call(callee) {
            self.check_expect_call(callee, arguments, line, locals);
            return;
        }
        let has_named = arguments
            .iter()
            .any(|argument| matches!(argument, HirCallArg::Named { .. }));
        let Some(params) = self.callee_params(callee, locals) else {
            return;
        };
        // The builtin count rule reads the canonical name's arity table.
        let canonical = self.canonical_callee(callee);
        match params {
            CalleeParams::FunctionValue(params) => {
                // TYPE_CALL_ARGUMENT_MISMATCH, the function-value named form: a
                // callable type keeps no parameter names, so a name cannot bind
                // — and lowering discards the name, so only the HIR shows it.
                if has_named {
                    self.emit(
                        "TYPE_CALL_ARGUMENT_MISMATCH",
                        format!(
                            "Call to function value `{callee}` cannot use named arguments because the callable type does not preserve parameter names."
                        ),
                        line,
                    );
                }
                // A callable type carries no defaults: exactly its count.
                if arguments.len() != params.len() {
                    self.emit(
                        "TYPE_CALL_ARITY_MISMATCH",
                        format!(
                            "Call to `{callee}` has {} argument(s), expected {}.",
                            arguments.len(),
                            params.len()
                        ),
                        line,
                    );
                }
                for (index, argument) in arguments.iter().enumerate() {
                    let value = match argument {
                        HirCallArg::Positional(value) | HirCallArg::Named { value, .. } => value,
                    };
                    let Some(expected) = params.get(index) else {
                        continue;
                    };
                    let actual = self.type_of(value, locals).without_state();
                    if !self.expression_compatible(expected, &actual, value) {
                        self.emit(
                            "TYPE_CALL_ARGUMENT_MISMATCH",
                            format!(
                                "Argument {} for `{callee}` has type {}, expected {}.",
                                index + 1,
                                actual.name(),
                                expected.name()
                            ),
                            line,
                        );
                    }
                }
            }
            CalleeParams::BuiltinUnnamed => {
                // No parameter-name metadata: a name cannot bind at all (bug-173 B);
                // the arguments stay in source order for the count.
                for argument in arguments {
                    if let HirCallArg::Named { name, line, .. } = argument {
                        self.report_unknown_name(callee, name, *line);
                    }
                }
                let source_order = source_order(arguments);
                self.check_builtin_call(callee, &canonical, &source_order, line, locals);
            }
            CalleeParams::BuiltinOverloads(overloads) => {
                // Whichever way selection ends, the checker's normalized list has
                // every argument — bound in overload order when an overload is
                // selected, else left in source order.
                let source_order = source_order(arguments);
                if !has_named {
                    self.check_builtin_call(callee, &canonical, &source_order, line, locals);
                    return;
                }
                // Overload selection needs a well-formed name set: the first
                // duplicate, else the first unknown name, ends the check.
                let named: Vec<(&String, usize)> = arguments
                    .iter()
                    .filter_map(|argument| match argument {
                        HirCallArg::Named { name, line, .. } => Some((name, *line)),
                        HirCallArg::Positional(_) => None,
                    })
                    .collect();
                for (index, (name, arg_line)) in named.iter().enumerate() {
                    if named[..index].iter().any(|(earlier, _)| earlier == name) {
                        self.report_duplicate_name(callee, name, *arg_line);
                        self.check_builtin_call(callee, &canonical, &source_order, line, locals);
                        return;
                    }
                }
                if let Some((name, arg_line)) = named.iter().find(|(name, _)| {
                    !overloads
                        .iter()
                        .any(|params| params.contains(&name.as_str()))
                }) {
                    self.report_unknown_name(callee, name, *arg_line);
                    self.check_builtin_call(callee, &canonical, &source_order, line, locals);
                    return;
                }
                let positionals = arguments.len() - named.len();
                let supplied_names: Vec<&str> =
                    named.iter().map(|(name, _)| name.as_str()).collect();
                if let Some(params) =
                    builtins::select_param_name_overload(&overloads, positionals, &supplied_names)
                {
                    let positional_values: Vec<&HirExpression> = arguments
                        .iter()
                        .filter_map(|argument| match argument {
                            HirCallArg::Positional(value) => Some(value),
                            HirCallArg::Named { .. } => None,
                        })
                        .collect();
                    let ordered: Vec<&HirExpression> = params
                        .iter()
                        .enumerate()
                        .filter_map(|(index, param)| {
                            if index < positionals {
                                positional_values.get(index).copied()
                            } else {
                                arguments.iter().find_map(|argument| match argument {
                                    HirCallArg::Named { name, value, .. } if name == param => {
                                        Some(value)
                                    }
                                    _ => None,
                                })
                            }
                        })
                        .collect();
                    self.check_builtin_call(callee, &canonical, &ordered, line, locals);
                    return;
                }
                // Every supplied name exists, but no overload's arity and layout
                // accept this combination: report the first parameter left
                // unsupplied by the smallest overload that names them all
                // (`connectTcp(host:, timeoutMs:)` omits `port`).
                let covering = overloads
                    .iter()
                    .filter(|params| supplied_names.iter().all(|name| params.contains(name)))
                    .min_by_key(|params| params.len());
                if let Some(params) = covering {
                    let missing = params.iter().enumerate().find(|(index, param)| {
                        *index >= positionals && !supplied_names.contains(param)
                    });
                    if let Some((_, missing)) = missing {
                        self.emit(
                            "TYPE_CALL_ARITY_MISMATCH",
                            format!(
                                "Call to `{callee}` omits parameter `{missing}` before a later supplied argument."
                            ),
                            line,
                        );
                        self.check_builtin_call(callee, &canonical, &source_order, line, locals);
                        return;
                    }
                }
                self.emit(
                    "TYPE_CALL_ARITY_MISMATCH",
                    format!("Call to `{callee}` has no overload taking these arguments."),
                    line,
                );
                self.check_builtin_call(callee, &canonical, &source_order, line, locals);
            }
            CalleeParams::Builtin(aliases) => {
                if !has_named {
                    let normalized: Vec<&HirExpression> = arguments
                        .iter()
                        .map(|argument| match argument {
                            HirCallArg::Positional(value) | HirCallArg::Named { value, .. } => {
                                value
                            }
                        })
                        .collect();
                    // `thread::start`'s entry check precedes its count (the
                    // checker ended the call's checks on a bad entry).
                    if canonical == "thread.start"
                        && !self.thread_start_entry_valid(normalized.first().copied())
                    {
                        self.report_thread_entry(line);
                        return;
                    }
                    self.check_builtin_call(callee, &canonical, &normalized, line, locals);
                    return;
                }
                let mut ordered: Vec<Option<&HirExpression>> = vec![None; aliases.len()];
                let mut extras: Vec<&HirExpression> = Vec::new();
                let mut next_positional = 0usize;
                let mut saw_unknown_named = false;
                for argument in arguments {
                    match argument {
                        HirCallArg::Positional(value) => {
                            while next_positional < ordered.len()
                                && ordered[next_positional].is_some()
                            {
                                next_positional += 1;
                            }
                            if next_positional < ordered.len() {
                                ordered[next_positional] = Some(value);
                                next_positional += 1;
                            } else {
                                extras.push(value);
                            }
                        }
                        HirCallArg::Named { name, value, line } => {
                            let Some(index) = aliases
                                .iter()
                                .position(|aliases| aliases.iter().any(|alias| alias == name))
                            else {
                                self.report_unknown_name(callee, name, *line);
                                saw_unknown_named = true;
                                continue;
                            };
                            if ordered[index].is_some() {
                                // Reported under the parameter's canonical
                                // (first) alias, whichever alias the call wrote.
                                self.report_duplicate_name(callee, aliases[index][0], *line);
                                continue;
                            }
                            ordered[index] = Some(value);
                        }
                    }
                }
                // The checker's normalized list: the bound slots in parameter
                // order, then the extras; an unknown or duplicate name binds
                // nowhere and drops out of it.
                let normalized: Vec<&HirExpression> =
                    ordered.iter().flatten().copied().chain(extras).collect();
                // An unknown name has already disarmed the omission check: the
                // gap it left is its own diagnostic's.
                if !saw_unknown_named {
                    for (index, names) in aliases.iter().enumerate() {
                        if ordered[index].is_none()
                            && ordered[index + 1..].iter().any(|filled| filled.is_some())
                        {
                            self.emit(
                                "TYPE_CALL_ARITY_MISMATCH",
                                format!(
                                    "Call to `{callee}` omits parameter `{}` before a later supplied argument.",
                                    names[0]
                                ),
                                line,
                            );
                            break;
                        }
                    }
                }
                if canonical == "thread.start"
                    && !self.thread_start_entry_valid(normalized.first().copied())
                {
                    self.report_thread_entry(line);
                    return;
                }
                self.check_builtin_call(callee, &canonical, &normalized, line, locals);
            }
            CalleeParams::Declared(params) => {
                let mut ordered: Vec<Option<&HirExpression>> = vec![None; params.len()];
                let mut next_positional = 0usize;
                let mut supplied = 0usize;
                let mut arity_error = false;
                for argument in arguments {
                    match argument {
                        HirCallArg::Positional(value) => {
                            while next_positional < ordered.len()
                                && ordered[next_positional].is_some()
                            {
                                next_positional += 1;
                            }
                            if next_positional >= ordered.len() {
                                arity_error = true;
                                continue;
                            }
                            ordered[next_positional] = Some(value);
                            next_positional += 1;
                            supplied += 1;
                        }
                        HirCallArg::Named { name, value, line } => {
                            let Some(index) = params.iter().position(|param| param.name == *name)
                            else {
                                self.report_unknown_name(callee, name, *line);
                                continue;
                            };
                            if ordered[index].is_some() {
                                self.report_duplicate_name(callee, name, *line);
                                continue;
                            }
                            ordered[index] = Some(value);
                            supplied += 1;
                        }
                    }
                }
                let required = params.iter().filter(|param| !param.has_default).count();
                let missing_required = ordered
                    .iter()
                    .zip(params.iter())
                    .any(|(slot, param)| slot.is_none() && !param.has_default);
                if arity_error || supplied < required || supplied > params.len() || missing_required
                {
                    self.emit(
                        "TYPE_CALL_ARITY_MISMATCH",
                        format!(
                            "Call to `{callee}` has {supplied} argument(s), expected {required} to {}.",
                            params.len()
                        ),
                        line,
                    );
                }
                // TYPE_CALL_ARGUMENT_MISMATCH, the declared-function form over
                // the SUPPLIED arguments only: lowering fills every omitted slot
                // from its default, so the IR cannot tell a supplied argument
                // from a filled one (and a literal supplied argument is coerced
                // to the parameter type before the IR sees it).
                for (index, slot) in ordered.iter().enumerate() {
                    let Some(argument) = slot else {
                        continue;
                    };
                    let actual = self.type_of(argument, locals).without_state();
                    if !self.expression_compatible(&params[index].type_, &actual, argument) {
                        self.emit(
                            "TYPE_CALL_ARGUMENT_MISMATCH",
                            format!(
                                "Argument {} for `{callee}` has type {}, expected {}.",
                                index + 1,
                                actual.name(),
                                params[index].type_.name()
                            ),
                            line,
                        );
                    }
                }
            }
        }
    }

    /// A builtin argument-type rejection after which the checker typed the
    /// call `Unknown` (so a binding of it cascades TYPE_UNKNOWN_VALUE).
    fn emit_call_typed_unknown(&mut self, detail: String, line: usize) {
        self.call_typed_unknown = true;
        self.emit("TYPE_CALL_ARGUMENT_MISMATCH", detail, line);
    }

    /// TYPE_UNKNOWN_VALUE, the initializer form — the source checker's cascade
    /// for a binding whose initializer it could not type. Its evidence is the
    /// checker's own typing verdict on the HIR: lowering stamps a lenient type
    /// on a failed builtin call and fills a `$`-temp for a trapped one, so the
    /// IR cannot tell the checker's `Unknown` from a typed value. `ir::verify`
    /// keeps the cascade for the values ITS rules poison (a mismatched
    /// operator, a bad constructor) — those the shape pass does not judge.
    fn check_initializer_known(
        &mut self,
        name: &str,
        value: &HirExpression,
        locals: &HashMap<String, ParameterType>,
        expected: Option<&ParameterType>,
        line: usize,
    ) {
        if self.checker_types_unknown(value, locals, expected) {
            self.emit(
                "TYPE_UNKNOWN_VALUE",
                format!("Initializer for binding `{name}` does not have a known type."),
                line,
            );
        }
    }

    /// Whether the source checker would have typed `value` `Unknown` — its
    /// cascade condition — reconstructed from lowering's seam plus the few
    /// verdicts the checker made that the seam does not: a call its rules
    /// typed `Unknown`, a constructor or `WITH` of something it would not
    /// construct, an arithmetic on `Money` its lattice rejects or on a
    /// value-less `SUB` call, `.state` on a plain `LET` of a stateful resource.
    /// (`ir::verify` cascades for the operator misuses its own rules poison.)
    fn checker_types_unknown(
        &self,
        value: &HirExpression,
        locals: &HashMap<String, ParameterType>,
        expected: Option<&ParameterType>,
    ) -> bool {
        match value {
            HirExpression::Trapped { expression, .. } => {
                return self.checker_types_unknown(expression, locals, expected);
            }
            // A bare general built-in predicate in a value position types from
            // the expectation (`LET f AS FUNC(Integer) AS Boolean = isEven`,
            // bug-368) — lowering's seam has no expectation to type it by.
            HirExpression::Identifier(name)
                if crate::codegen::builtins::general::builtin_function_id(name).is_some() =>
            {
                if let Some(ParameterType::Func(params, returns, _)) = expected {
                    if params.len() == 1
                        && **returns == ParameterType::Boolean
                        && crate::codegen::builtins::general::filter_predicate_type_typed(
                            name, &params[0],
                        )
                        .is_some()
                    {
                        return false;
                    }
                }
            }
            HirExpression::Constructor { type_, .. } => return !self.constructor_typed(type_),
            HirExpression::WithUpdate { target, .. } => {
                return !self.with_update_typed(target, locals);
            }
            HirExpression::Call { callee, .. } => {
                let canonical = self.canonical_callee(callee);
                if builtins::is_package_constant(&canonical) {
                    return false;
                }
                if self
                    .call_verdicts
                    .get(&(value as *const HirExpression as usize))
                    .copied()
                    .unwrap_or(false)
                {
                    return true;
                }
                // A package-table builtin call the argument checker matched is
                // typed by the overload's DECLARED return type, even when an
                // argument's own type is unknown (`crypto::hash(Hash.Nope, s)`:
                // the unknown member is its own rule, and `Unknown` is
                // compatible with the `Hash` slot) — where lowering's exact
                // registry resolution answers `Unknown` and would cascade.
                if builtins::is_builtin_call(&canonical) && builtins::table_checked_call(&canonical)
                {
                    return false;
                }
            }
            HirExpression::Binary {
                left,
                operator,
                right,
                ..
            } if matches!(
                operator,
                BinaryOp::Add
                    | BinaryOp::Subtract
                    | BinaryOp::Multiply
                    | BinaryOp::Divide
                    | BinaryOp::IntDiv
                    | BinaryOp::Mod
                    | BinaryOp::Power
            ) =>
            {
                // A value-less (`SUB`) or untyped operand: the checker's
                // promotion had nothing numeric to promote.
                let (left, right) = (self.type_of(left, locals), self.type_of(right, locals));
                if matches!(left, ParameterType::Nothing | ParameterType::Unknown)
                    || matches!(right, ParameterType::Nothing | ParameterType::Unknown)
                {
                    return true;
                }
                let (left_money, right_money) = (
                    matches!(left, ParameterType::Money),
                    matches!(right, ParameterType::Money),
                );
                if (left_money || right_money)
                    && crate::numeric::is_numeric(&left)
                    && crate::numeric::is_numeric(&right)
                    // plan-112 Phase 4 retypes `numeric` and deletes this seam.
                    && crate::numeric::typed_money_result_type(
                        operator.name(),
                        left_money,
                        right_money,
                    )
                        .is_none()
                {
                    return true;
                }
            }
            HirExpression::MemberAccess { target, member } if member == "state" => {
                if let HirExpression::Identifier(name) = target.as_ref() {
                    if self.state_dropped.contains(name) {
                        return true;
                    }
                }
            }
            _ => {}
        }
        matches!(
            lower::expression_type(value, locals, &self.context),
            None | Some(ParameterType::Unknown)
        )
    }

    /// Whether the checker typed a `T[…]` constructor: the three built-in
    /// nominals (rejected as read-only but typed), else a visible declared
    /// RECORD; `Ok`/`Result`, a compiler-owned record, a union, an enum or an
    /// unknown name typed `Unknown`.
    fn constructor_typed(&self, type_: &ParameterType) -> bool {
        // Nominal tests, not a render-and-match: every name here is a bare
        // `Named`, so `is_named` decides identically for every input while
        // dropping the `name()` render. (plan-111-D Correction D1 — a letter-B
        // site the arm scanner could not see until it learned to read a whole
        // pattern rather than only a leading spelling.)
        if type_.is_named("AttributedString")
            || type_.is_named("Error")
            || type_.is_named("ErrorLoc")
        {
            return true;
        }
        if type_.is_named("Ok") || type_.is_named("Result") {
            return false;
        }
        self.declared_record_constructible(type_)
    }

    /// A declared (or imported) record the calling file may construct — the
    /// one kind whose arguments the checker went on to check.
    fn declared_record_constructible(&self, type_: &ParameterType) -> bool {
        if read_only_record(type_) {
            return false;
        }
        self.types
            .get(type_)
            .is_some_and(|info| info.is_record && self.type_visible(info))
    }

    /// Whether the checker typed a `WITH target { … }`: a built-in nominal or a
    /// map entry keeps its type (the read-only rejection is `ir::verify`'s);
    /// otherwise the target must be a declared record that is not compiler-owned.
    fn with_update_typed(
        &self,
        target: &HirExpression,
        locals: &HashMap<String, ParameterType>,
    ) -> bool {
        let target_type = self.type_of(target, locals);
        if matches!(target_type, ParameterType::MapEntryOf(..)) {
            return true;
        }
        let ParameterType::Named(_) = &target_type else {
            return false;
        };
        let name = target_type.name();
        if matches!(
            name.as_ref(),
            "AttributedString" | "Error" | "ErrorLoc" | "Scalar"
        ) {
            return true;
        }
        if read_only_record(&target_type) {
            return false;
        }
        self.types
            .get(&target_type)
            .is_some_and(|info| info.is_record)
    }

    /// The checker's visibility rule for a declared type: `PRIVATE` only from
    /// its own file.
    fn type_visible(&self, info: &TypeShape) -> bool {
        match info.visibility {
            Visibility::Export | Visibility::Public => true,
            Visibility::Private => info.file == self.file,
        }
    }

    /// Whether the checker bound a MATCH pattern's name (see `walk_match_case`).
    fn checker_binds_pattern(
        &self,
        pattern: &crate::hir::HirMatchPattern,
        matched_type: &ParameterType,
    ) -> bool {
        let crate::hir::HirMatchPattern::Union { type_, .. } = pattern else {
            return false;
        };
        let variant = type_.name();
        if matches!(variant.as_ref(), "Ok" | "Error" | "Err") {
            return false;
        }
        // Correction G6: peel the STATE clause BEFORE asking "is this a
        // nominal?". Until plan-111-B a stateful resource union arrived here as
        // one opaque `Named("Stream STATE Cursor")` — it passed this guard, and
        // the string-splitting `without_state` below peeled the clause out of
        // the spelling. `Stateful` is a NEW variant, so the unpeeled value is no
        // longer a `Named`, the guard rejected every `CASE File(f)` over a
        // stateful union, `f` went unbound, and `f.state.pos` typed `Unknown`.
        // Peeled first, this decides exactly as it did before for every input.
        let union = matched_type.without_state();
        let ParameterType::Named(_) = union else {
            return false;
        };
        self.types
            .get(&union)
            .is_some_and(|info| info.is_union && info.variants.iter().any(|v| *v == variant))
    }

    fn report_unknown_name(&mut self, callee: &str, name: &str, line: usize) {
        self.emit(
            "TYPE_UNKNOWN_ARGUMENT_NAME",
            format!("Call to `{callee}` does not have a parameter named `{name}`."),
            line,
        );
    }

    fn report_duplicate_name(&mut self, callee: &str, name: &str, line: usize) {
        self.emit(
            "TYPE_DUPLICATE_ARGUMENT_NAME",
            format!("Call to `{callee}` supplies parameter `{name}` more than once."),
            line,
        );
    }

    /// Bind `name` in `locals` at `type_`, exactly where lowering emits its
    /// `IrOp::Bind` for the same binding.
    fn bind(
        &mut self,
        name: &str,
        type_: ParameterType,
        locals: &mut HashMap<String, ParameterType>,
    ) {
        #[cfg(test)]
        self.bound_types.push((name.to_string(), type_.clone()));
        locals.insert(name.to_string(), type_);
    }

    /// Record a diagnostic at `line` of the current file.
    fn emit(&mut self, rule: &str, detail: String, line: usize) {
        self.diagnostics.push(PendingDiagnostic {
            rule: rule.to_string(),
            detail,
            path: self.project_dir.join(&self.file),
            line,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{parse_source, AstProject};
    use crate::ir::IrOp;

    /// Parse `src` as `main.mfb`, augment it with the builtin package sources
    /// (the same chain the build path runs before lowering), and return the
    /// concrete HIR.
    fn hir_from(src: &str) -> HirProject {
        let file = parse_source(Path::new("main.mfb"), "main.mfb", src)
            .expect("test source must lex+parse");
        let project = AstProject {
            name: "test".to_string(),
            files: vec![file],
        };
        // The build's order: registry injection into the AST, then the generic
        // HIR is monomorphized into the concrete program the pass walks.
        let augmented =
            crate::resolver::augment_project(&project).expect("builtin augmentation must succeed");
        crate::monomorph::monomorphize_project(Path::new("."), &crate::hir::elaborate(&augmented))
            .expect("test source must monomorphize")
    }

    /// The shape pass's rule codes for `src`, in traversal order.
    fn shape_codes(src: &str) -> Vec<String> {
        collect_diagnostics(
            Path::new("/proj"),
            &hir_from(src),
            &[],
            &HashMap::new(),
            &[],
        )
        .into_iter()
        .map(|d| d.rule)
        .collect()
    }

    fn rejects_with(src: &str, rule: &str) -> bool {
        shape_codes(src).iter().any(|r| r == rule)
    }

    fn accepts(src: &str) -> bool {
        shape_codes(src).is_empty()
    }

    fn wrap_import(import: &str, body: &str) -> String {
        format!("IMPORT {import}\nFUNC main AS Integer\n{body}\n  RETURN 0\nEND FUNC\n")
    }

    /// Every named `Bind` in the lowered IR (the `$` temps lowering mints have
    /// no HIR counterpart), in emission order.
    fn lowered_binds(ops: &[IrOp], out: &mut Vec<(String, ParameterType)>) {
        for op in ops {
            match op {
                IrOp::Bind { name, type_, .. } => {
                    if !name.starts_with('$') {
                        out.push((name.clone(), type_.clone()));
                    }
                }
                IrOp::If {
                    then_body,
                    else_body,
                    ..
                } => {
                    lowered_binds(then_body, out);
                    lowered_binds(else_body, out);
                }
                IrOp::Match { cases, .. } => {
                    for case in cases {
                        lowered_binds(&case.body, out);
                    }
                }
                // The FOR EACH element is bound by the loop op itself, not by a
                // `Bind` inside its body.
                IrOp::ForEach {
                    name, type_, body, ..
                } => {
                    out.push((name.clone(), type_.clone()));
                    lowered_binds(body, out);
                }
                IrOp::For { body, .. }
                | IrOp::While { body, .. }
                | IrOp::DoUntil { body, .. }
                | IrOp::Trap { body, .. } => lowered_binds(body, out),
                _ => {}
            }
        }
    }

    // The seam-fidelity proof: the walker's scope tracking (parameters, LET
    // with/without annotation, inline-TRAP LET, FOR with numeric promotion,
    // FOR EACH element, MATCH case binding, trap binding, lambda parameter)
    // yields, for every binding, the type lowering stamps on its `IrOp::Bind`.
    #[test]
    fn walker_types_bindings_exactly_as_lowering_does() {
        let src = "IMPORT collections\n\
                   TYPE Point\n  x AS Integer\n  y AS Integer\nEND TYPE\n\
                   UNION Shape\n  Point\nEND UNION\n\
                   FUNC helper(n AS Integer) AS Float\n  RETURN n * 1.5\nEND FUNC\n\
                   FUNC main AS Integer\n\
                   \x20 LET a = 1\n\
                   \x20 LET b AS Float = 2.5\n\
                   \x20 LET c = helper(a)\n\
                   \x20 LET items = [1, 2, 3]\n\
                   \x20 FOR i = a TO 10 STEP 0.5\n\
                   \x20   LET d = i\n\
                   \x20 NEXT\n\
                   \x20 FOR EACH item IN items\n\
                   \x20   LET e = item\n\
                   \x20 NEXT\n\
                   \x20 LET s AS Shape = Point[1, 2]\n\
                   \x20 MATCH s\n\
                   \x20   CASE Point(p)\n\
                   \x20     LET f = p\n\
                   \x20 END MATCH\n\
                   \x20 LET g = collections::filter(items, LAMBDA(v AS Integer) -> v > 1)\n\
                   \x20 LET h = helper(a) TRAP(err)\n\
                   \x20   LET i2 = err\n\
                   \x20   RECOVER 0.0\n\
                   \x20 END TRAP\n\
                   \x20 RETURN 0\n\
                   TRAP(e2)\n\
                   \x20 LET j = e2\n\
                   \x20 RETURN 1\n\
                   END TRAP\n\
                   END FUNC\n";
        let hir = hir_from(src);
        let facts = lower::lower_facts(&hir, &HashMap::new(), &[]);
        let no_imports = HashMap::new();
        let mut walker = Walker::new(Path::new("/proj"), &facts, &hir, &[], &no_imports, &[]);
        walker.walk_project(&hir);
        let emitted: Vec<_> = walker
            .diagnostics
            .iter()
            .map(|d| format!("{}:{} {}", d.line, d.rule, d.detail))
            .collect();
        assert!(
            emitted.is_empty(),
            "a clean program emits nothing: {emitted:?}"
        );

        let ir = lower::lower_augmented_project(&hir, None, &HashMap::new(), &[]);
        let mut lowered = Vec::new();
        for function in ir.functions.iter().filter(|f| f.name == "main") {
            lowered_binds(&function.body, &mut lowered);
        }
        // Lowering emits the MATCH case binding and the FOR loop variable as
        // `Bind`s of their own; the walker records them through `bind` too, so
        // the two sequences line up name-for-name.
        let walked: Vec<_> = walker
            .bound_types
            .iter()
            .filter(|(name, _)| !name.starts_with('$'))
            .cloned()
            .collect();
        let lowered_by_name: HashMap<_, _> = lowered.iter().cloned().collect();
        for (name, type_) in &walked {
            let stamped = lowered_by_name
                .get(name)
                .unwrap_or_else(|| panic!("lowering emitted no Bind for `{name}`"));
            assert_eq!(stamped, type_, "binding `{name}` typed differently");
        }
        // Coverage of the scope forms the walker mirrors.
        let names: Vec<_> = walked.iter().map(|(n, _)| n.as_str()).collect();
        for expected in [
            "a", "b", "c", "items", "i", "d", "item", "e", "s", "f", "g", "h", "i2", "j",
        ] {
            assert!(names.contains(&expected), "walker never bound `{expected}`");
        }
        // The promoted FOR type and the inline-TRAP success type are the
        // non-trivial computations; pin them so a silent fallback to `Unknown`
        // on both sides cannot pass.
        assert_eq!(lowered_by_name["i"], ParameterType::Float);
        assert_eq!(lowered_by_name["h"], ParameterType::Float);
        assert_eq!(lowered_by_name["e"], ParameterType::Integer);
    }

    // The standalone entry renders nothing and passes on a clean program.
    #[test]
    fn check_project_accepts_clean_source() {
        let hir = hir_from("FUNC main AS Integer\n  RETURN 0\nEND FUNC\n");
        assert!(check_project(Path::new("/proj"), &hir, &HashMap::new()).is_ok());
    }

    // ---- named arguments: user functions ----------------------------------

    #[test]
    fn user_named_argument_valid() {
        assert!(accepts(
            "FUNC g(a AS Integer, b AS Integer) AS Integer\n  RETURN a + b\nEND FUNC\nFUNC main AS Integer\n  RETURN g(b := 2, a := 1)\nEND FUNC\n"
        ));
    }

    #[test]
    fn user_named_argument_unknown_name() {
        assert!(rejects_with(
            "FUNC g(a AS Integer) AS Integer\n  RETURN a\nEND FUNC\nFUNC main AS Integer\n  RETURN g(z := 1)\nEND FUNC\n",
            "TYPE_UNKNOWN_ARGUMENT_NAME"
        ));
    }

    #[test]
    fn user_named_argument_duplicate() {
        // `g(1, a := 2)`: the positional already filled `a`.
        assert!(rejects_with(
            "FUNC g(a AS Integer) AS Integer\n  RETURN a\nEND FUNC\nFUNC main AS Integer\n  RETURN g(1, a := 2)\nEND FUNC\n",
            "TYPE_DUPLICATE_ARGUMENT_NAME"
        ));
    }

    #[test]
    fn user_named_positional_after_named_walk() {
        // A positional after a named argument fills the first free slot.
        assert!(accepts(
            "FUNC g(a AS Integer, b AS Integer) AS Integer\n  RETURN a + b\nEND FUNC\nFUNC main AS Integer\n  RETURN g(b := 2, 1)\nEND FUNC\n"
        ));
    }

    #[test]
    fn user_private_function_in_another_file_is_not_a_call_target() {
        // The source checker never bound names against a function invisible
        // from the calling file; the rule stays silent there too.
        let main = parse_source(
            Path::new("main.mfb"),
            "main.mfb",
            "FUNC main AS Integer\n  RETURN g(z := 1)\nEND FUNC\n",
        )
        .expect("parses");
        let other = parse_source(
            Path::new("other.mfb"),
            "other.mfb",
            "PRIVATE FUNC g(a AS Integer) AS Integer\n  RETURN a\nEND FUNC\n",
        )
        .expect("parses");
        let project = AstProject {
            name: "test".to_string(),
            files: vec![main, other],
        };
        let hir = crate::resolver::augment_hir_project(&crate::hir::elaborate(&project))
            .expect("augments");
        let codes: Vec<_> =
            collect_diagnostics(Path::new("/proj"), &hir, &[], &HashMap::new(), &[])
                .into_iter()
                .map(|d| d.rule)
                .collect();
        assert!(codes.is_empty(), "{codes:?}");
    }

    #[test]
    fn imported_package_function_named_arguments() {
        // A `.mfp` function's parameter names come from the imported-signature
        // table; the call is spelled through the file's import binding.
        let mut imported = HashMap::new();
        imported.insert(
            "shapes.area".to_string(),
            ExternalSignature {
                params: vec![
                    crate::ir::ExternalFunctionParam {
                        name: "width".to_string(),
                        type_: ParameterType::Integer,
                        has_default: false,
                    },
                    crate::ir::ExternalFunctionParam {
                        name: "height".to_string(),
                        type_: ParameterType::Integer,
                        has_default: false,
                    },
                ],
                returns: ParameterType::Integer,
                isolated: false,
                sub: false,
            },
        );
        let src = "IMPORT shapes AS sh\nFUNC main AS Integer\n  LET a = sh::area(width := 1, depth := 2)\n  LET b = sh::area(1, width := 2)\n  RETURN a + b\nEND FUNC\n";
        let file = parse_source(Path::new("main.mfb"), "main.mfb", src).expect("parses");
        let project = AstProject {
            name: "test".to_string(),
            files: vec![file],
        };
        let hir = crate::hir::elaborate(&project);
        let diagnostics = collect_diagnostics(Path::new("/proj"), &hir, &[], &imported, &[]);
        let codes: Vec<_> = diagnostics.iter().map(|d| d.rule.as_str()).collect();
        // Line 3 supplies one bindable name of two required parameters, so the
        // arity rule follows the unknown name; line 4's duplicate leaves `height`
        // unsupplied, so it is followed by the arity rule too.
        assert_eq!(
            codes,
            [
                "TYPE_UNKNOWN_ARGUMENT_NAME",
                "TYPE_CALL_ARITY_MISMATCH",
                "TYPE_DUPLICATE_ARGUMENT_NAME",
                "TYPE_CALL_ARITY_MISMATCH",
            ]
        );
        assert_eq!(
            diagnostics[0].detail,
            "Call to `sh.area` does not have a parameter named `depth`."
        );
        assert_eq!(diagnostics[0].line, 3);
        assert_eq!(diagnostics[0].path, Path::new("/proj/main.mfb"));
    }

    // ---- named arguments: builtins ----------------------------------------

    #[test]
    fn builtin_named_argument_valid() {
        assert!(accepts(
            "IMPORT json\nIMPORT io\nFUNC main AS Integer\n  io::print(json::stringify(json::parse(value := \"null\")))\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn builtin_named_argument_unknown_name() {
        assert!(rejects_with(
            "IMPORT json\nFUNC main AS Integer\n  LET x = json::parse(nope := \"null\")\n  RETURN 0\nEND FUNC\n",
            "TYPE_UNKNOWN_ARGUMENT_NAME"
        ));
    }

    #[test]
    fn builtin_named_argument_matching_parameter_name_accepted() {
        // Every registry builtin carries its parameter names (`math::abs`'s is
        // `value`), so a matching name binds; the source checker's old test of
        // this call assumed a name-less fallback that no builtin reaches today.
        assert!(accepts(
            "IMPORT math\nFUNC main AS Integer\n  LET x = math::abs(value := -1)\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn builtin_named_argument_duplicate() {
        assert!(rejects_with(
            "IMPORT json\nFUNC main AS Integer\n  LET x = json::parse(\"a\", value := \"b\")\n  RETURN 0\nEND FUNC\n",
            "TYPE_DUPLICATE_ARGUMENT_NAME"
        ));
    }

    #[test]
    fn builtin_named_then_positional_reorders() {
        assert!(accepts(
            "IMPORT strings\nFUNC main AS Integer\n  LET b = strings::startsWith(prefix := \"a\", \"abc\")\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn builtin_general_call_named_argument() {
        // `general` members dispatch through their own arm in the source
        // checker; the rule reaches them all the same.
        assert!(rejects_with(
            "FUNC main AS Integer\n  LET s = toString(v := 1)\n  RETURN 0\nEND FUNC\n",
            "TYPE_UNKNOWN_ARGUMENT_NAME"
        ));
    }

    #[test]
    fn overloaded_named_unknown_argument_rejected() {
        assert!(rejects_with(
            &wrap_import("datetime", "  LET z = datetime::fixedOffset(bogus := 1)"),
            "TYPE_UNKNOWN_ARGUMENT_NAME"
        ));
    }

    #[test]
    fn overloaded_named_duplicate_argument_rejected() {
        assert!(rejects_with(
            &wrap_import(
                "datetime",
                "  LET z = datetime::fixedOffset(hours := 1, hours := 2)"
            ),
            "TYPE_DUPLICATE_ARGUMENT_NAME"
        ));
    }

    #[test]
    fn overloaded_duplicate_reported_once_and_before_unknown_names() {
        // A duplicate ends overload selection before any unknown name is
        // considered, and only the first duplicate is reported; the count rule
        // still runs over the source-order list the selection fell back to.
        let codes = shape_codes(&wrap_import(
            "datetime",
            "  LET z = datetime::fixedOffset(hours := 1, hours := 2, bogus := 3, bogus := 4)",
        ));
        assert_eq!(
            codes,
            [
                "TYPE_DUPLICATE_ARGUMENT_NAME",
                "TYPE_CALL_ARITY_MISMATCH",
                "TYPE_UNKNOWN_VALUE",
            ]
        );
    }

    #[test]
    fn overloaded_named_valid_selection_accepted() {
        assert!(accepts(&wrap_import(
            "datetime",
            "  LET z = datetime::fixedOffset(hours := 1, mins := 2)",
        )));
    }

    #[test]
    fn function_value_call_count_rejected() {
        // A local of FUNC type takes exactly its type's parameter count.
        let src = "FUNC main AS Integer\n  LET f AS FUNC(Integer) AS Integer = LAMBDA(x AS Integer) -> x + 1\n  LET r AS Integer = f(1, 2)\n  RETURN r\nEND FUNC\n";
        assert_eq!(shape_codes(src), ["TYPE_CALL_ARITY_MISMATCH"]);
        // The same program through the monomorphizer, as the build runs it.
        let file = parse_source(Path::new("main.mfb"), "main.mfb", src).expect("parses");
        let project = AstProject {
            name: "test".to_string(),
            files: vec![file],
        };
        let augmented = crate::resolver::augment_project(&project).expect("augments");
        let concrete = crate::monomorph::monomorphize_project(
            Path::new("."),
            &crate::hir::elaborate(&augmented),
        )
        .expect("monomorphizes");
        let codes: Vec<_> =
            collect_diagnostics(Path::new("/proj"), &concrete, &[], &HashMap::new(), &[])
                .into_iter()
                .map(|d| d.rule)
                .collect();
        assert_eq!(codes, ["TYPE_CALL_ARITY_MISMATCH"]);
    }

    // ---- argument types: the source-path forms ----------------------------

    #[test]
    fn declared_function_argument_type_mismatch() {
        let diagnostics = collect_diagnostics(
            Path::new("/proj"),
            &hir_from(
                "FUNC g(a AS Integer, b AS String = \"x\") AS Integer\n  RETURN a\nEND FUNC\nFUNC main AS Integer\n  RETURN g(\"no\")\nEND FUNC\n",
            ),
            &[],
            &HashMap::new(),
            &[],
        );
        let details: Vec<_> = diagnostics.iter().map(|d| d.detail.as_str()).collect();
        // Only the SUPPLIED argument is judged; the defaulted `b` is not.
        assert_eq!(
            details,
            ["Argument 1 for `g` has type String, expected Integer."]
        );
    }

    #[test]
    fn declared_function_literal_coercions_accepted() {
        // A `Byte`/`Fixed` parameter accepts a fitting literal, and a list of
        // literals coerces element by element (the checker's rules).
        assert!(accepts(
            "FUNC g(a AS Byte, b AS Fixed, c AS List OF Fixed) AS Integer\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  RETURN g(200, -1, [1, 2.5])\nEND FUNC\n"
        ));
    }

    #[test]
    fn builtin_table_argument_type_mismatch() {
        let diagnostics = collect_diagnostics(
            Path::new("/proj"),
            &hir_from(
                "IMPORT math\nFUNC main AS Integer\n  LET p = math::pow(\"a\", 2)\n  RETURN 0\nEND FUNC\n",
            ),
            &[],
            &HashMap::new(),
            &[],
        );
        let details: Vec<_> = diagnostics.iter().map(|d| d.detail.as_str()).collect();
        // The failed call types the binding Unknown: the cascade follows.
        assert_eq!(details.len(), 2, "{details:?}");
        assert!(
            details[0]
                .starts_with("Call to `math.pow` has argument type(s) (String, Integer), expected"),
            "{details:?}"
        );
        assert_eq!(
            details[1],
            "Initializer for binding `p` does not have a known type."
        );
    }

    #[test]
    fn general_error_call_argument_mismatch() {
        // `error(code, message)` lowers to record constructors, so only the
        // shape pass can judge its arguments.
        assert!(rejects_with(
            "FUNC main AS Integer\n  LET e = error(1)\n  RETURN 0\nEND FUNC\n",
            "TYPE_CALL_ARGUMENT_MISMATCH"
        ));
    }

    #[test]
    fn thread_start_entry_must_be_imported_isolated_func() {
        let diagnostics = collect_diagnostics(
            Path::new("/proj"),
            &hir_from(
                "IMPORT thread\nFUNC main AS Integer\n  LET t = thread::start(main, \"x\", 1, 1)\n  RETURN 0\nEND FUNC\n",
            ),
            &[],
            &HashMap::new(),
            &[],
        );
        let details: Vec<_> = diagnostics.iter().map(|d| d.detail.as_str()).collect();
        // The entry rejection ends the call's checks (no count or type form
        // follows) and types the call Unknown, so the binding cascades.
        assert_eq!(
            details,
            [
                "thread.start entry point must be an exported ISOLATED FUNC from an imported package.",
                "Initializer for binding `t` does not have a known type.",
            ]
        );
    }

    #[test]
    fn term_draw_text_attributed_requires_astrings_import() {
        // An `AttributedString` value reaches the call through a parameter
        // without any `IMPORT astrings`, so the bridge body is not injected.
        let diagnostics = collect_diagnostics(
            Path::new("/proj"),
            &hir_from(
                "IMPORT term\nFUNC show(a AS AttributedString) AS Integer\n  term::drawText(1, 1, a)\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
            ),
            &[],
            &HashMap::new(),
            &[],
        );
        let details: Vec<_> = diagnostics.iter().map(|d| d.detail.as_str()).collect();
        assert_eq!(
            details,
            ["Call to `term.drawText` with an `AttributedString` requires `IMPORT astrings`."]
        );
    }

    #[test]
    fn cascade_spares_typed_seam_cases() {
        // A package override of an overridable general builtin types by the
        // builtin's result (`toString(net::Url)` → String), and a MATCH on a
        // union that INCLUDES another binds the included variants — neither
        // is an `Unknown` for the checker.
        assert!(accepts(
            "IMPORT net\nTYPE Circle\n  r AS Integer\nEND TYPE\nUNION Shape\n  Circle\nEND UNION\nUNION Extra INCLUDES Shape\n  Circle\nEND UNION\nFUNC score(s AS Extra) AS Integer\n  MATCH s\n    CASE Circle(c)\n      RETURN c.r + 1\n  END MATCH\nEND FUNC\nFUNC main AS Integer\n  LET u AS net::Url = net::toUrl(\"http://x/\")\n  LET rendered AS String = toString(u)\n  RETURN len(rendered)\nEND FUNC\n"
        ));
    }

    // ---- export_in_executable_diagnostics (build-pipeline entry point) ------

    #[test]
    fn export_in_executable_flags_each_item_kind() {
        use crate::ast::{parse_source, AstProject};
        // Cover every item kind the visibility match walks: binding, type,
        // function, resource (HirItem::Resource arm), and func alias (HirItem::FuncAlias
        // arm). The resource/alias targets need only parse — this entry point runs
        // before import resolution.
        let src = "EXPORT LET g AS Integer = 5\nEXPORT TYPE Rec\n  x AS Integer\nEND TYPE\nEXPORT FUNC f() AS Integer\n  RETURN 1\nEND FUNC\nEXPORT RESOURCE Db CLOSE BY x::close\nEXPORT FUNC ff AS x::gg\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n";
        let file = parse_source(Path::new("main.mfb"), "main.mfb", src).expect("parse");
        let project = AstProject {
            name: "t".to_string(),
            files: vec![file],
        };
        let diags = export_in_executable_diagnostics(false, &project);
        assert!(diags.iter().all(|d| d.rule == "EXPORT_IN_EXECUTABLE"));
        assert!(diags.len() >= 5, "expected an EXPORT diagnostic per item");
    }

    #[test]
    fn export_in_executable_empty_for_package_project() {
        use crate::ast::{parse_source, AstProject};
        let src = "EXPORT FUNC f() AS Integer\n  RETURN 1\nEND FUNC\n";
        let file = parse_source(Path::new("main.mfb"), "main.mfb", src).expect("parse");
        let project = AstProject {
            name: "t".to_string(),
            files: vec![file],
        };
        // A package project never flags EXPORT (that is its purpose).
        assert!(export_in_executable_diagnostics(true, &project).is_empty());
    }

    #[test]
    fn export_in_executable_no_export_no_diagnostic() {
        use crate::ast::{parse_source, AstProject};
        let src = "FUNC f() AS Integer\n  RETURN 1\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n";
        let file = parse_source(Path::new("main.mfb"), "main.mfb", src).expect("parse");
        let project = AstProject {
            name: "t".to_string(),
            files: vec![file],
        };
        assert!(export_in_executable_diagnostics(false, &project).is_empty());
    }

    #[test]
    fn matched_table_builtin_call_with_an_unknown_argument_is_typed() {
        // `Hash.SHA224` is an unknown enum member (verify's
        // TYPE_UNKNOWN_ENUM_MEMBER); the checker still typed the matched
        // `crypto::hash` overload by its declared return type, so the binding
        // does not cascade.
        let src = "IMPORT crypto\nFUNC main AS Integer\n  LET a AS List OF Byte = crypto::hash(crypto::Hash.SHA224, \"abc\")\n  RETURN len(a)\nEND FUNC\n";
        assert_eq!(shape_codes(src), Vec::<String>::new());
    }

    // ---- imported package metadata ---------------------------------------

    #[test]
    fn corrupt_package_tables_are_package_invalid() {
        // A garbage `.mfp` reaches this pass only when the decode boundary is
        // bypassed (as a unit test does); each of the three table reads
        // reports, in the checker's order: types, resources, then functions.
        let dir = std::env::temp_dir().join(format!("mfb_shape_pkg_{}", std::process::id()));
        let pkgs = dir.join("packages");
        std::fs::create_dir_all(&pkgs).unwrap();
        std::fs::write(pkgs.join("brokenpkg.mfp"), b"not a valid mfp container").unwrap();
        let file = crate::ast::parse_source(
            Path::new("main.mfb"),
            "main.mfb",
            "IMPORT brokenpkg\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
        )
        .unwrap();
        let hir = crate::hir::elaborate(&crate::ast::AstProject {
            name: "t".into(),
            files: vec![file],
        });
        let diagnostics = collect_diagnostics(&dir, &hir, &[], &HashMap::new(), &[]);
        let _ = std::fs::remove_dir_all(&dir);
        let details: Vec<_> = diagnostics
            .iter()
            .map(|d| (d.rule.as_str(), d.detail.as_str(), d.line))
            .collect();
        assert_eq!(details.len(), 3, "{details:?}");
        assert!(details
            .iter()
            .all(|(rule, _, line)| *rule == "PACKAGE_INVALID" && *line == 1));
        assert!(details[0]
            .1
            .ends_with("has unreadable or invalid type metadata."));
        assert!(details[1].1.ends_with("has an unreadable resource table."));
        assert!(details[2]
            .1
            .ends_with("has unreadable or invalid function metadata."));
    }

    #[test]
    fn package_type_validation_arms() {
        let src = "ENUM Color\n  Red, Green\nEND ENUM\nTYPE Point\n  x AS Integer\nEND TYPE\nUNION Shape\n  Point\nEND UNION\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n";
        let hir = hir_from(src);
        let facts = lower::lower_facts(&hir, &HashMap::new(), &[]);
        let no_imports = HashMap::new();
        let mut walker = Walker::new(Path::new("/proj"), &facts, &hir, &[], &no_imports, &[]);
        let pkg = Path::new("packages/fake.mfp");
        let validate = |walker: &mut Walker, type_: &ParameterType| {
            walker.validate_package_type(pkg, type_, "ctx", 7, &mut HashSet::new());
            walker
                .diagnostics
                .drain(..)
                .map(|d| d.detail)
                .collect::<Vec<_>>()
        };
        // A map keyed by a List is not comparable; its Res value recurses silently.
        let map = ParameterType::MapOf(
            Box::new(ParameterType::ListOf(Box::new(ParameterType::Integer))),
            Box::new(ParameterType::Res(Box::new(ParameterType::String))),
        );
        assert_eq!(
            validate(&mut walker, &map),
            ["Imported package `packages/fake.mfp` has ctx with non-comparable map key type `List OF Integer`."]
        );
        // An undeclared nominal, directly and through a wrapper.
        assert_eq!(
            validate(&mut walker, &ParameterType::named("Nope")),
            ["Imported package `packages/fake.mfp` has ctx that references unknown type `Nope`."]
        );
        assert_eq!(
            validate(
                &mut walker,
                &ParameterType::SetOf(Box::new(ParameterType::named("Nope")))
            )
            .len(),
            1
        );
        // Declared enum / record / union, built-in nominals, resources, and the
        // structural shapes all walk silently.
        for accepted in [
            ParameterType::named("Color"),
            ParameterType::named("Point"),
            ParameterType::named("Shape"),
            ParameterType::named("Error"),
            ParameterType::named("tcp.Socket"),
            ParameterType::ResultOf(Box::new(ParameterType::Money)),
            ParameterType::Func(
                vec![ParameterType::Integer, ParameterType::String],
                Box::new(ParameterType::Boolean),
                false,
            ),
            ParameterType::ThreadHandle {
                worker: false,
                msg: Box::new(ParameterType::Integer),
                res: Box::new(ParameterType::String.with_state(&ParameterType::Boolean)),
                out: Box::new(ParameterType::Float),
            },
            ParameterType::ThreadHandle {
                worker: true,
                msg: Box::new(ParameterType::Integer),
                res: Box::new(ParameterType::Nothing),
                out: Box::new(ParameterType::Nothing),
            },
        ] {
            assert!(
                validate(&mut walker, &accepted).is_empty(),
                "{}",
                accepted.name()
            );
        }
        // plan-111-B: a stateful resource must be recognized as a RESOURCE both
        // when validating an imported signature and when asking comparability.
        // Both paths route "every other spelling" through a re-wrap into one
        // opaque nominal, and the structural `without_state` cannot peel a
        // re-wrapped spelling — so both need the clause peeled BEFORE the
        // re-wrap. Without that, `Db STATE DbInfo` reads as an unknown type
        // here and as a COMPARABLE one below.
        //
        // An imported package's stateful resource (`Db STATE DbInfo`, as a
        // signature spells it) is a resource, not an unknown type — the
        // libsnd / native-resource-state fixtures build clean.
        let mut walker = Walker::new(
            Path::new("/proj"),
            &facts,
            &hir,
            &[],
            &no_imports,
            &["Db".to_string()],
        );
        for spelled in ["Db", "Db STATE DbInfo", "pkg.Db STATE DbInfo"] {
            assert!(
                validate(&mut walker, &ParameterType::parse(spelled)).is_empty(),
                "{spelled}"
            );
            // ...and a resource is never comparable, stateful or not.
            assert!(
                !walker.is_comparable(&ParameterType::parse(spelled)),
                "`{spelled}` is a resource and must not be comparable"
            );
        }
        // A stateful type whose base is NOT a resource keeps the opaque-nominal
        // behaviour: unknown to the package validator, permissively comparable.
        assert!(
            !validate(&mut walker, &ParameterType::parse("Nope STATE S")).is_empty(),
            "a non-resource stateful type is still an unknown type"
        );
        assert!(walker.is_comparable(&ParameterType::parse("Nope STATE S")));
        // A union whose variant record carries an unknown field type reports it.
        assert_eq!(
            validate(
                &mut walker,
                &ParameterType::ThreadHandle {
                    worker: false,
                    msg: Box::new(ParameterType::named("Ghost")),
                    res: Box::new(ParameterType::Nothing),
                    out: Box::new(ParameterType::Nothing),
                }
            )
            .len(),
            1
        );
    }

    // ---- native LINK facts lowering folds away ---------------------------

    fn link_project(body: &str) -> String {
        format!(
            "RESOURCE Handle CLOSE BY demoLink::release\nLINK \"demo\" AS demoLink\n  FUNC release(RES h AS Handle) AS Nothing\n    SYMBOL \"demo_release\"\n    ABI (h CPtr) AS status CInt32\n    SUCCESS_ON status = 0\n  END FUNC\n{body}END LINK\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n"
        )
    }

    #[test]
    fn const_pin_must_fold_to_an_immediate() {
        let src = link_project(
            "  FUNC f(n AS Integer) AS Integer\n    SYMBOL \"demo_f\"\n    ABI (n CInt32, s CInt32, t CInt32, u CInt32, value OUT CInt32) AS status CInt32\n    CONST s = \"literal\"\n    CONST t = 1 + 1\n    CONST u = -1\n    RETURN value\n    SUCCESS_ON status = 0\n  END FUNC\n",
        );
        let diagnostics = collect_diagnostics(
            Path::new("/proj"),
            &hir_from(&src),
            &[],
            &HashMap::new(),
            &[],
        );
        let lines: Vec<_> = diagnostics
            .iter()
            .map(|d| (d.rule.as_str(), d.line))
            .collect();
        // `s` and `t` are unfoldable; the negated literal `u` folds.
        assert_eq!(
            lines,
            [
                ("NATIVE_CONST_UNKNOWN_SLOT", 11),
                ("NATIVE_CONST_UNKNOWN_SLOT", 12)
            ]
        );
    }

    #[test]
    fn free_deallocator_signature() {
        let describe = |abi: &str| {
            link_project(&format!(
                "  FUNC describe(RES h AS Handle) AS String\n    SYMBOL \"demo_describe\"\n    ABI (h CPtr) AS text CPtr\n    RETURN text\n    FREE text\n      SYMBOL \"demo_free\"\n      ABI {abi}\n    END FREE\n  END FUNC\n"
            ))
        };
        assert!(accepts(&describe("(ptr CPtr) AS CVoid")));
        for malformed in ["(ptr CInt32) AS CVoid", "(ptr CPtr) AS CInt32"] {
            let diagnostics = collect_diagnostics(
                Path::new("/proj"),
                &hir_from(&describe(malformed)),
                &[],
                &HashMap::new(),
                &[],
            );
            let lines: Vec<_> = diagnostics
                .iter()
                .map(|d| (d.rule.as_str(), d.line))
                .collect();
            assert_eq!(lines, [("NATIVE_FREE_INVALID", 12)], "{malformed}");
        }
    }

    // ---- record constructors ---------------------------------------------

    #[test]
    fn error_records_cannot_be_constructed() {
        let src = "FUNC main AS Integer\n  LET e = Error[1, \"boom\"]\n  LET l = ErrorLoc[\"f\", 1]\n  RETURN 0\nEND FUNC\n";
        let diagnostics = collect_diagnostics(
            Path::new("/proj"),
            &hir_from(src),
            &[],
            &HashMap::new(),
            &[],
        );
        let details: Vec<_> = diagnostics
            .iter()
            .map(|d| (d.rule.as_str(), d.detail.as_str(), d.line))
            .collect();
        assert_eq!(
            details,
            [
                (
                    "TYPE_READ_ONLY_RECORD_CONSTRUCTOR",
                    "`Error` is a read-only built-in record and cannot be constructed; use `error(code, message)` to create an Error.",
                    2
                ),
                (
                    "TYPE_READ_ONLY_RECORD_CONSTRUCTOR",
                    "`ErrorLoc` is a read-only built-in record and cannot be constructed; use `error(code, message)` to create an Error.",
                    3
                ),
            ]
        );
        // The `AttributedString` and compiler-owned forms are verify's.
        let src = "FUNC main AS Integer\n  LET a AS AttributedString = AttributedString[\"hi\"]\n  RETURN 0\nEND FUNC\n";
        assert!(!rejects_with(src, "TYPE_READ_ONLY_RECORD_CONSTRUCTOR"));
    }

    #[test]
    fn constructor_named_field_set_twice() {
        let src = "TYPE Point\n  x AS Integer\n  y AS Integer\nEND TYPE\nFUNC main AS Integer\n  LET a AS Point = Point[x := 1, x := 2]\n  LET b AS Point = WITH a { y := 10, y := 20 }\n  RETURN a.x + b.y\nEND FUNC\n";
        // The constructor form only; the WITH form is verify's.
        let diagnostics = collect_diagnostics(
            Path::new("/proj"),
            &hir_from(src),
            &[],
            &HashMap::new(),
            &[],
        );
        let details: Vec<_> = diagnostics
            .iter()
            .map(|d| (d.rule.as_str(), d.detail.as_str(), d.line))
            .collect();
        assert_eq!(
            details,
            [(
                "TYPE_DUPLICATE_FIELD",
                "Constructor `Point` sets field `x` more than once.",
                6
            )]
        );
        // Not for an unconstructible record (the checker never reached the
        // argument check); the read-only rule reports instead.
        let src =
            "FUNC main AS Integer\n  LET e = Error[code := 1, code := 2]\n  RETURN 0\nEND FUNC\n";
        assert!(!rejects_with(src, "TYPE_DUPLICATE_FIELD"));
    }

    // ---- the Money exactness nudge ---------------------------------------

    #[test]
    fn money_scaled_by_a_bare_decimal_literal_warns() {
        let src = |expr: &str| {
            format!("FUNC main AS Integer\n  LET price AS Money = 10.00m\n  LET scaled AS Money = {expr}\n  RETURN 0\nEND FUNC\n")
        };
        for warned in [
            "price * 1.08",
            "1.08 * price",
            "price / 1.08",
            "price * -1.08",
        ] {
            assert_eq!(
                shape_codes(&src(warned)),
                ["MONEY_INEXACT_FLOAT_LITERAL"],
                "{warned}"
            );
        }
        // A suffixed literal is intrinsically typed; a Float variable never
        // warns; `literal / Money` is not the scaling shape; `+` is not scaling.
        for silent in [
            "price * 1.08F",
            "price * 1.08f",
            "price * 2",
            "price + 1.08m",
        ] {
            assert!(
                accepts(&src(silent)),
                "{silent}: {:?}",
                shape_codes(&src(silent))
            );
        }
        let float_var = "FUNC main AS Integer\n  LET price AS Money = 10.00m\n  LET rate AS Float = 1.08\n  LET scaled AS Money = price * rate\n  RETURN 0\nEND FUNC\n";
        assert!(accepts(float_var), "{:?}", shape_codes(float_var));
    }

    // ---- TESTING assertions lowering expands away -------------------------

    fn tcase(body: &str) -> String {
        // The assertion builtins are recognized by name in the Call arm
        // (`is_testing_call`), so a plain FUNC body reaches `check_expect_call`
        // without the TESTING desugaring (`mfb test` only).
        format!("FUNC main AS Integer\n{body}\n  RETURN 0\nEND FUNC\n")
    }

    #[test]
    fn expect_arity_forms() {
        assert_eq!(
            shape_codes(&tcase("  expectEqual(1)")),
            ["TESTING_EXPECT_ARITY"]
        );
        // `expectTrap` has a 1–2 range: the range wording.
        let diagnostics = collect_diagnostics(
            Path::new("/proj"),
            &hir_from(&tcase("  expectTrap()")),
            &[],
            &HashMap::new(),
            &[],
        );
        assert_eq!(
            diagnostics
                .iter()
                .map(|d| d.detail.as_str())
                .collect::<Vec<_>>(),
            ["`expectTrap` expects 1\u{2013}2 argument(s), got 0."]
        );
    }

    #[test]
    fn expect_typed_operands_must_be_the_named_type() {
        assert_eq!(
            shape_codes(&tcase("  expectFloat(1, 2)")),
            [
                "TESTING_EXPECT_TYPE_MISMATCH",
                "TESTING_EXPECT_TYPE_MISMATCH"
            ]
        );
        assert!(accepts(&tcase("  expectInteger(1, 2)")));
        assert!(accepts(&tcase("  expectNString(\"a\", \"b\")")));
    }

    #[test]
    fn expect_equal_operands_must_be_comparable_and_printable() {
        assert_eq!(
            shape_codes(&tcase("  expectEqual(\"a\", 1)")),
            ["TESTING_EXPECT_INCOMPARABLE"]
        );
        // A Map is neither `=`-comparable nor printable — both, per operand.
        let body = "  LET m AS Map OF String TO Integer = Map OF String TO Integer {}\n  expectEqual(m, m)";
        assert_eq!(
            shape_codes(&tcase(body)),
            [
                "TESTING_EXPECT_INCOMPARABLE",
                "TESTING_EXPECT_NOT_PRINTABLE",
                "TESTING_EXPECT_NOT_PRINTABLE"
            ]
        );
        // A record of comparable fields compares but does not print.
        let src = "TYPE Point\n  x AS Integer\nEND TYPE\nFUNC main AS Integer\n  LET p AS Point = Point[1]\n  expectEqual(p, p)\n  RETURN 0\nEND FUNC\n";
        assert_eq!(
            shape_codes(src),
            [
                "TESTING_EXPECT_NOT_PRINTABLE",
                "TESTING_EXPECT_NOT_PRINTABLE"
            ]
        );
        assert!(accepts(&tcase("  expectEqual(1, 1)")));
        assert!(accepts(&tcase("  expectEqual(1, 2.5)")));
    }

    #[test]
    fn expect_trap_code_must_be_integer() {
        let body =
            "  LET xs AS List OF Integer = [1, 2, 3]\n  expectTrap(collections::get(xs, 0), \"x\")";
        let src = format!("IMPORT collections\n{}", tcase(body));
        assert_eq!(shape_codes(&src), ["TESTING_EXPECT_CODE_TYPE"]);
    }

    #[test]
    fn expect_trap_requires_a_call_to_guard() {
        assert_eq!(
            shape_codes(&tcase("  expectTrap(5)")),
            ["TESTING_EXPECT_TRAP_REQUIRES_FALLIBLE"]
        );
        assert_eq!(
            shape_codes(&tcase("  expectNTrap(42)")),
            ["TESTING_EXPECT_TRAP_REQUIRES_FALLIBLE"]
        );
        let src = format!("IMPORT math\n{}", tcase("  expectTrap(math::pi)"));
        assert_eq!(shape_codes(&src), ["TESTING_EXPECT_TRAP_REQUIRES_FALLIBLE"]);
        let src = format!("IMPORT math\n{}", tcase("  expectTrap(math::pi())"));
        assert_eq!(shape_codes(&src), ["TESTING_EXPECT_TRAP_REQUIRES_FALLIBLE"]);
    }

    // ---- control-flow shapes lowering erases -------------------------------

    #[test]
    fn exit_forms_and_bare_sub_return() {
        let codes = shape_codes(
            "SUB tick()\n  RETURN\nEND SUB\nFUNC main AS Integer\n  EXIT SUB\n  EXIT FUNC\n  RETURN 0\nEND FUNC\n",
        );
        assert_eq!(
            codes,
            [
                "SUB_RETURN_FORBIDDEN",
                "EXIT_SUB_IN_FUNC",
                "UNREACHABLE_AFTER_EXIT",
                "UNREACHABLE_AFTER_EXIT",
            ]
        );
    }

    #[test]
    fn unreachable_after_exit_func_names_each_following_statement() {
        let diagnostics = collect_diagnostics(
            Path::new("/proj"),
            &hir_from("FUNC main AS Integer\n  EXIT FUNC\n  LET a = 1\n  RETURN a\nEND FUNC\n"),
            &[],
            &HashMap::new(),
            &[],
        );
        let lines: Vec<_> = diagnostics
            .iter()
            .map(|d| (d.rule.as_str(), d.line))
            .collect();
        assert_eq!(
            lines,
            [
                ("EXIT_FUNC_FORBIDDEN", 2),
                ("UNREACHABLE_AFTER_EXIT", 3),
                ("UNREACHABLE_AFTER_EXIT", 4),
            ]
        );
    }

    #[test]
    fn loop_exit_tail_inside_a_handler_is_shapes() {
        // `treeify_handler` truncates the handler after `EXIT FOR`, so the
        // RECOVER/PROPAGATE behind it never reach the IR.
        let diagnostics = collect_diagnostics(
            Path::new("/proj"),
            &hir_from(
                "FUNC f(v AS Integer) AS Integer\n  RETURN v\nEND FUNC\nFUNC main AS Integer\n  FOR i = 1 TO 3\n    LET a = f(i) TRAP(e)\n      EXIT FOR\n      RECOVER 0\n      PROPAGATE\n    END TRAP\n  NEXT\n  FOR j = 1 TO 3\n    CONTINUE FOR\n    LET dead = 1\n  NEXT\n  RETURN 0\nEND FUNC\n",
            ),
            &[],
            &HashMap::new(),
            &[],
        );
        let lines: Vec<_> = diagnostics
            .iter()
            .map(|d| (d.rule.as_str(), d.line))
            .collect();
        // The second loop's tail is outside any handler: verify's form.
        assert_eq!(
            lines,
            [("UNREACHABLE_AFTER_EXIT", 8), ("UNREACHABLE_AFTER_EXIT", 9)]
        );
    }

    #[test]
    fn recover_outside_a_handler_and_the_count_forms() {
        let diagnostics = collect_diagnostics(
            Path::new("/proj"),
            &hir_from(
                "FUNC f(v AS Integer) AS Integer\n  RETURN v\nEND FUNC\nSUB g()\n  EXIT SUB\nEND SUB\nFUNC main AS Integer\n  LET a = f(1) TRAP(e)\n    RECOVER\n  END TRAP\n  g() TRAP(e)\n    RECOVER 2\n  END TRAP\n  RECOVER 1\n  RETURN a\nEND FUNC\n",
            ),
            &[],
            &HashMap::new(),
            &[],
        );
        let details: Vec<_> = diagnostics.iter().map(|d| d.detail.as_str()).collect();
        assert_eq!(
            details,
            [
                "RECOVER must supply a Integer value for the trapped expression.",
                "RECOVER must not supply a value for a value-less trapped expression.",
                "RECOVER is valid only inside an inline TRAP handler.",
            ]
        );
    }

    #[test]
    fn inline_trap_handler_must_diverge_on_every_path() {
        let diagnostics = collect_diagnostics(
            Path::new("/proj"),
            &hir_from(
                "IMPORT io\nFUNC f(v AS Integer) AS Integer\n  RETURN v\nEND FUNC\nFUNC main AS Integer\n  LET a = f(1) TRAP(e)\n    io::print(e.message)\n  END TRAP\n  LET b = f(2) TRAP(e)\n    IF a > 0 THEN\n      RECOVER 0\n    END IF\n  END TRAP\n  LET c = f(3) TRAP(e)\n    IF a > 0 THEN\n      RECOVER 0\n    ELSE\n      RETURN 1\n    END IF\n  END TRAP\n  RETURN a + b + c\nEND FUNC\n",
            ),
            &[],
            &HashMap::new(),
            &[],
        );
        let lines: Vec<_> = diagnostics
            .iter()
            .map(|d| (d.rule.as_str(), d.line))
            .collect();
        assert_eq!(
            lines,
            [
                ("TYPE_INLINE_TRAP_FALLS_THROUGH", 6),
                ("TYPE_INLINE_TRAP_FALLS_THROUGH", 9),
            ]
        );
    }

    #[test]
    fn nested_call_rule_follows_enclosing_call_rule() {
        // Outer `g(z := ...)` reports before the inner `g(y := 1)` argument.
        let diagnostics = collect_diagnostics(
            Path::new("/proj"),
            &hir_from(
                "FUNC g(a AS Integer) AS Integer\n  RETURN a\nEND FUNC\nFUNC main AS Integer\n  RETURN g(z := g(y := 1))\nEND FUNC\n",
            ),
            &[], &HashMap::new(),
            &[],
        );
        let details: Vec<_> = diagnostics.iter().map(|d| d.detail.as_str()).collect();
        assert_eq!(
            details,
            [
                "Call to `g` does not have a parameter named `z`.",
                "Call to `g` has 0 argument(s), expected 1 to 1.",
                "Call to `g` does not have a parameter named `y`.",
                "Call to `g` has 0 argument(s), expected 1 to 1.",
            ]
        );
    }
    // --- bug-466: field access on a record whose owning package is not imported ---
    //
    // `tcp::localAddress` returns a `net::Address`. Imports are not transitive
    // and a builtin package cannot re-export another's types, so `Address`'s
    // FIELDS are readable only where the file itself imports `net` (the rule
    // `mfb man tcp localAddress` states). The checker caught the resulting
    // `Unknown` at a BINDING and at an operator, but not when it was passed as a
    // call argument: there it survived to native lowering, which failed with the
    // bare, unlocated `native plan has no storage class for type 'Unknown'`.
    //
    // The gate below is a pre-lowering rule on the field access itself, so the
    // verdict no longer depends on where the `Unknown` happened to land — nor,
    // as `unrelated_import_does_not_make_a_foreign_record_field_readable` pins,
    // on which UNRELATED package the file also imported.

    /// The rule/line pairs for `src`, for the location assertions.
    fn shape_reports(src: &str) -> Vec<(String, usize)> {
        collect_diagnostics(
            Path::new("/proj"),
            &hir_from(src),
            &[],
            &HashMap::new(),
            &[],
        )
        .into_iter()
        .map(|d| (d.rule, d.line))
        .collect()
    }

    /// bug-466 Reproduction 1: the `Unknown` field read as an argument to an
    /// OVERLOADED BUILTIN (`tcp::connect`), which unified it against a candidate
    /// rather than rejecting it.
    #[test]
    fn foreign_record_field_as_builtin_argument_is_rejected() {
        let src = "IMPORT tcp\n\
                   FUNC main AS Integer\n\
                   \x20 RES s = tcp::listen(\"127.0.0.1\", 0)\n\
                   \x20 LET b = tcp::localAddress(s)\n\
                   \x20 RES c = tcp::connect(\"127.0.0.1\", b.port)\n\
                   \x20 RETURN 0\n\
                   END FUNC\n";
        assert_eq!(
            shape_reports(src),
            [("TYPE_UNKNOWN_VALUE".to_string(), 5)],
            "the field read must be refused AT ITS OWN LINE, not absorbed by lowering"
        );
    }

    /// bug-466 Reproduction 2: the same `Unknown` as an argument to a USER
    /// `FUNC`, whose declared `Integer` parameter accepted it. The unlocated
    /// codegen error this replaces blamed `io.print`, three calls away.
    #[test]
    fn foreign_record_field_as_user_func_argument_is_rejected() {
        let src = "IMPORT tcp\n\
                   IMPORT io\n\
                   FUNC take(n AS Integer) AS Integer\n\
                   \x20 RETURN n\n\
                   END FUNC\n\
                   FUNC main AS Integer\n\
                   \x20 RES s = tcp::listen(\"127.0.0.1\", 0)\n\
                   \x20 LET b = tcp::localAddress(s)\n\
                   \x20 io::print(toString(take(b.port)))\n\
                   \x20 RETURN 0\n\
                   END FUNC\n";
        assert_eq!(
            shape_reports(src),
            [("TYPE_UNKNOWN_VALUE".to_string(), 9)],
            "the field read must be refused at its own line, not at `io::print`"
        );
    }

    /// bug-466 Reproduction 4: `IMPORT udp` (never referenced) used to make the
    /// identical `tcp` program compile, because `udp` declares a record whose
    /// field is a `net::Address` and that dragged `Address`'s definition into the
    /// project-wide type table. Whether a program compiled therefore depended on
    /// which UNRELATED packages it imported. The gate keys on the file's own
    /// imports, so the verdict is now the same either way.
    #[test]
    fn unrelated_import_does_not_make_a_foreign_record_field_readable() {
        let src = "IMPORT tcp\n\
                   IMPORT udp\n\
                   FUNC main AS Integer\n\
                   \x20 RES s = tcp::listen(\"127.0.0.1\", 0)\n\
                   \x20 LET b = tcp::localAddress(s)\n\
                   \x20 RES c = tcp::connect(\"127.0.0.1\", b.port)\n\
                   \x20 RETURN 0\n\
                   END FUNC\n";
        assert_eq!(
            shape_reports(src),
            [("TYPE_UNKNOWN_VALUE".to_string(), 6)],
            "an unrelated import must not change the verdict"
        );
    }

    /// The rule this bug is about ENFORCING, not tightening: with `IMPORT net`
    /// the field read is exactly as legal as it always was.
    #[test]
    fn foreign_record_field_with_the_owning_import_is_accepted() {
        let src = "IMPORT tcp\n\
                   IMPORT net\n\
                   IMPORT io\n\
                   FUNC main AS Integer\n\
                   \x20 RES s = tcp::listen(\"127.0.0.1\", 0)\n\
                   \x20 LET b = tcp::localAddress(s)\n\
                   \x20 io::print(b.host & \":\" & toString(b.port))\n\
                   \x20 RES c = tcp::connect(\"127.0.0.1\", b.port)\n\
                   \x20 RETURN 0\n\
                   END FUNC\n";
        assert!(accepts(src), "got {:?}", shape_codes(src));
    }

    /// bug-466 Reproduction 3, characterization: the position the checker
    /// ALREADY caught — the annotated binding — keeps reporting, now preceded by
    /// the field access's own report.
    #[test]
    fn foreign_record_field_in_a_binding_still_reports() {
        let src = "IMPORT tcp\n\
                   FUNC main AS Integer\n\
                   \x20 RES s = tcp::listen(\"127.0.0.1\", 0)\n\
                   \x20 LET b = tcp::localAddress(s)\n\
                   \x20 LET p AS Integer = b.port\n\
                   \x20 RETURN 0\n\
                   END FUNC\n";
        assert_eq!(
            shape_reports(src),
            [
                ("TYPE_UNKNOWN_VALUE".to_string(), 5),
                ("TYPE_UNKNOWN_VALUE".to_string(), 5),
            ],
            "the binding cascade must survive alongside the new field-access gate"
        );
    }
}
