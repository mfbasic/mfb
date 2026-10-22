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
use crate::ast::Visibility;
use crate::codegen::builtins;
use crate::hir::{
    HirCallArg, HirConstructorArg, HirExpression, HirFile, HirFunction, HirItem, HirMatchCase,
    HirProject, HirStatement,
};
use crate::operators::{BinaryOp, UnaryOp};
use crate::rules::PendingDiagnostic;
use crate::types::{CAbiType, ParameterType};
use std::collections::{HashMap, HashSet};
use std::path::Path;

mod calls;
mod compat;
mod declarations;
mod expressions;
mod statements;

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
    imported_globals: &[crate::ir::ImportedGlobal],
    imported_signatures: &HashMap<String, ExternalSignature>,
    imported_resource_types: &[String],
) -> Vec<PendingDiagnostic> {
    // The typing seam is built over the UNFILTERED imported-signature table:
    // the source checker typed a reference to an imported function by its
    // `.mfp` signature, so a `thread::start(pkg::worker, …)` argument must type
    // as that ISOLATED FUNC here too. (Lowering's own facts keep only the
    // resource-returning subset for `ir::verify`'s sake.)
    let facts = lower::lower_facts(hir, imported_signatures, imported_types, imported_globals);
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
    let diagnostics = collect_diagnostics(project_dir, hir, &[], &[], imported_signatures, &[]);
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
    /// bug-642: the user calls that instantiated the built-in package body being
    /// walked. Its diagnostics are reported at each: the body's own lines belong
    /// to the package source, not to `file`, which only hosts the instantiation.
    sites: Vec<crate::hir::InstantiationSite>,
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

/// A built-in nominal that is always in scope and declares no fields, so an
/// importer's package-metadata walk (`validate_package_type`) accepts it without
/// a type-table row.
fn is_always_in_scope_nominal(type_: &ParameterType) -> bool {
    matches!(type_, ParameterType::Named(sym)
        if matches!(sym.resolve(), "AttributedString" | "Error" | "ErrorLoc" | "Scalar"))
}

/// bug-823: a value type a BUILTIN package declares, spelled with its package
/// qualifier (`json.Json`, `datetime.DateTime`), as an imported package's
/// exported signatures and type fields carry it.
///
/// The compiler owns every builtin type's definition, so an importer's package
/// metadata walk (`validate_package_type`) accepts one whether or not the
/// importer imports that builtin. The dependency does not export it (bug-750),
/// so the importer has no type-table row for it unless its own `IMPORT`
/// injected the builtin's source, and asking for that import rejected a package
/// over a type the importer may never touch. Only naming the type in the
/// importer's own source (name resolution) or reading a builtin record's fields
/// (bug-466, `builtin_record_owner`) needs the importer's own import. Neither is
/// decided here, and this does not add the type to the walker's type table,
/// which would mark it `shadowed` and switch bug-466's field rule off.
fn is_builtin_package_type(type_: &ParameterType) -> bool {
    let ParameterType::Named(symbol) = type_ else {
        return false;
    };
    let spelled = symbol.resolve();
    spelled
        .split_once('.')
        .is_some_and(|(package, _)| builtins::is_builtin_import(package))
        && crate::codegen::registry::registry().is_builtin_type(spelled)
}

/// bug-624 B: in a package build, reject an `EXPORT` declaration that names a
/// type this package declares without `EXPORT` (`EXPORT_NAMES_NON_EXPORTED_TYPE`).
///
/// The `.mfp` type table carries only exported types. So such a package used to
/// build cleanly, and then every importer rejected it in
/// `validate_package_type` (`PACKAGE_INVALID ... references unknown type`). An
/// exported binding of that type types as `Unknown` in the importer instead. The
/// rule reports at the exporting declaration, where the fix belongs.
///
/// It walks exactly what the importer reads off the `.mfp`, through every
/// structural wrapper (`for_each_nominal`):
///
/// * an `EXPORT TYPE`'s fields at every field visibility, since the writer
///   serializes `PRIVATE` fields too;
/// * an `EXPORT UNION`'s variant fields, with its `includes` expanded as the
///   writer's `concrete_union_variants` does. The variant record's own name is
///   not checked. Neither the importer nor native codegen needs the record
///   exported, because the union row carries each variant's fields inline;
/// * every `EXPORT FUNC` parameter and return type;
/// * every `EXPORT LET`/`MUT` binding type.
///
/// An exported type reached this way is checked as its own declaration, so the
/// walk stops at the first type the importer could not resolve. A nominal this
/// package does not declare is never flagged: an imported (re-exported) type, a
/// resource, a builtin type (qualified, sigil or always-in-scope), or a
/// template parameter. The input is the lowered source IR, so it carries the
/// same `visibility == "export"` flags and concrete (monomorphized) types the
/// writer encodes.
pub(crate) fn export_names_non_exported_type_diagnostics(
    is_package: bool,
    ir: &crate::ir::IrProject,
    project_dir: &Path,
) -> Vec<PendingDiagnostic> {
    const EXPORT: &str = "export";
    if !is_package {
        return Vec::new();
    }
    let declared: HashMap<&str, &crate::ir::IrType> =
        ir.types.iter().map(|ty| (ty.name.as_str(), ty)).collect();
    let hidden_in = |type_: &ParameterType| -> Vec<String> {
        let mut hidden: Vec<String> = Vec::new();
        let mut walked = type_.clone();
        super::package::for_each_nominal(&mut walked, &mut |name| {
            let spelled = name.resolve();
            if is_always_in_scope_nominal(&ParameterType::Named(*name))
                || super::package::is_builtin_type_name(spelled)
                || declared
                    .get(spelled)
                    .is_none_or(|ty| ty.visibility == EXPORT)
                || hidden.iter().any(|seen| seen == spelled)
            {
                return;
            }
            hidden.push(spelled.to_string());
        });
        hidden
    };
    let mut diagnostics = Vec::new();
    let mut check =
        |file: &str, line: u32, declaration: &str, position: &str, type_: &ParameterType| {
            for hidden in hidden_in(type_) {
                let hidden = crate::internal_name::display_name(&hidden);
                diagnostics.push(PendingDiagnostic {
                    rule: "EXPORT_NAMES_NON_EXPORTED_TYPE".to_string(),
                    detail: format!(
                        "{declaration} names `{hidden}` in its {position}, but `{hidden}` is not \
                     exported, so no importer of this package can resolve it. EXPORT \
                     `{hidden}`, or keep it out of the exported declaration."
                    ),
                    path: if file.is_empty() {
                        project_dir.join("<generated>")
                    } else {
                        project_dir.join(file)
                    },
                    line: line as usize,
                });
            }
        };

    for ty in &ir.types {
        if ty.visibility != EXPORT || super::package::is_builtin_type_name(&ty.name) {
            continue;
        }
        let declaration = format!(
            "EXPORT {} `{}`",
            ty.kind.to_uppercase(),
            crate::internal_name::display_name(&ty.name)
        );
        for field in &ty.fields {
            let position = format!("field `{}`", field.name);
            check(&ty.file, ty.loc.line, &declaration, &position, &field.type_);
        }
        // A union's variants, its `includes` expanded depth-first before its own
        // variants, exactly as the writer serializes the union row.
        let mut unions = vec![ty];
        let mut expanded: HashSet<&str> = HashSet::new();
        while let Some(union) = unions.pop() {
            if !expanded.insert(union.name.as_str()) {
                continue;
            }
            unions.extend(
                union
                    .includes
                    .iter()
                    .filter_map(|include| declared.get(include.as_str()).copied()),
            );
            for variant in &union.variants {
                for field in &variant.fields {
                    let position = format!(
                        "variant `{}` field `{}`",
                        crate::internal_name::display_name(&variant.name),
                        field.name
                    );
                    check(&ty.file, ty.loc.line, &declaration, &position, &field.type_);
                }
            }
        }
    }
    for function in &ir.functions {
        if function.visibility != EXPORT || super::package::is_builtin_type_name(&function.name) {
            continue;
        }
        let declaration = format!(
            "EXPORT {} `{}`",
            function.kind.to_uppercase(),
            crate::internal_name::display_name(&function.name)
        );
        for param in &function.params {
            let position = format!("parameter `{}`", param.name);
            check(
                &function.file,
                function.loc.line,
                &declaration,
                &position,
                &param.type_,
            );
        }
        check(
            &function.file,
            function.loc.line,
            &declaration,
            "return type",
            &function.returns,
        );
    }
    for binding in &ir.bindings {
        if binding.visibility != EXPORT {
            continue;
        }
        let declaration = format!(
            "EXPORT {} `{}`",
            if binding.mutable { "MUT" } else { "LET" },
            crate::internal_name::display_name(&binding.name)
        );
        check(
            &binding.file,
            binding.loc.line,
            &declaration,
            "type",
            &binding.type_,
        );
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

/// Whether an `os::getConfigOr` argument is a literal: a String, a Boolean, or
/// a number, negated or not (plan-167-B).
fn is_config_literal(expression: &HirExpression) -> bool {
    match expression {
        HirExpression::String(_) | HirExpression::Number(_) | HirExpression::Boolean(_) => true,
        HirExpression::Unary {
            operator, operand, ..
        } => *operator == UnaryOp::Negate && matches!(operand.as_ref(), HirExpression::Number(_)),
        _ => false,
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
    kinds: &dyn builtins::TypeKinds,
) -> Option<ParameterType> {
    // plan-111-B: typed throughout. `resolve_call_return_type_typed` (plan-104-C)
    // is the exact twin — it routes the three bespoke per-package resolvers
    // through the same string path they already had, and takes the generic
    // registry path with no strings at all. That the twin exists is what let
    // this conversion happen here rather than moving to letter C, which
    // plan-111-B §2 left open pending exactly that check.
    if let Some(return_type) =
        builtins::resolve_call_return_type_with_kinds(callee, arg_types, true, kinds)
    {
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
        if let Some(return_type) =
            builtins::resolve_call_return_type_with_kinds(callee, &trial, true, kinds)
        {
            return Some(return_type);
        }
    }
    None
}

/// plan-140-A: the built-in resolver's [`builtins::TypeKinds`] oracle over the
/// walker's declared and imported types. A qualified built-in spelling falls back
/// to its bare leaf, as the compatibility rule's lookup does.
struct ShapeKinds<'a>(&'a HashMap<ParameterType, TypeShape>);

impl builtins::TypeKinds for ShapeKinds<'_> {
    fn is_enum(&self, t: &ParameterType) -> bool {
        let name = t.name();
        self.0
            .get(t)
            .or_else(|| {
                let bare = builtins::builtin_qualified_bare_leaf(&name);
                self.0.get(&ParameterType::named(bare))
            })
            .is_some_and(|shape| shape.is_enum)
    }

    /// bug-988: a declared or imported union's variant list, spelled as the
    /// compatibility rule's union arm matches it (qualified or bare), else the
    /// built-in packages' unions.
    fn union_accepts(&self, union: &ParameterType, actual: &ParameterType) -> bool {
        let union = union.without_state();
        let actual = actual.without_state();
        let union_name = union.name();
        let actual_name = actual.name();
        let union_bare = builtins::builtin_qualified_bare_leaf(&union_name);
        let actual_bare = builtins::builtin_qualified_bare_leaf(&actual_name);
        let declared =
            self.0
                .get(&union)
                .or_else(|| self.0.get(&ParameterType::named(union_bare)))
                .is_some_and(|shape| {
                    shape.is_union
                        && shape.variants.iter().any(|variant| {
                            variant == actual_name.as_ref() || variant == actual_bare
                        })
                });
        declared || crate::codegen::registry::registry_union_accepts(&union, &actual)
    }
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
    // The qualified identity only — see `verify::read_only_record_type`, this
    // rule's twin. A source constructor of the builtin can only spell it
    // qualified (bug-483 is why the qualified form must match); a bare leaf here
    // is a project type of the same name and must not be refused (plan-132 D1).
    crate::codegen::builtins::term::is_read_only_record(type_)
        || type_.is_builtin_qualified("net", crate::codegen::builtins::net::ADDRESS_TYPE)
        || type_.is_builtin_qualified("audio", crate::codegen::builtins::audio::AUDIO_DEVICE_TYPE)
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

#[cfg(test)]
mod tests;
