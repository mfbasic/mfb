use super::*;

use super::fallible::{
    comparison_can_raise, float_operator_raises_only_when_observed,
    float_unary_raises_only_when_observed, is_total_literal_negation, operator_can_raise,
    unary_operator_can_raise, Fallibility,
};
use super::lower_link::{link_aliases, link_cstructs, link_functions, native_resources};
use crate::hir::{
    HirCallArg, HirConstructorArg, HirExpression, HirFunction, HirItem, HirMatchCase,
    HirMatchPattern, HirParam, HirProject, HirStatement, HirTopLevelBinding, HirTypeDecl,
    HirTypeField,
};
use crate::operators::{BinaryOp, UnaryOp};
use crate::types::ParameterType;

mod call_args;
mod captures;
mod concat_fold;
mod construct;
mod decls;
mod expression;
mod inline_trap;
mod literal_retype;
mod statement;
mod trap_hoist;
mod typing;
use call_args::*;
use captures::*;
use concat_fold::*;
use construct::*;
pub(super) use decls::function_return_type;
use decls::*;
use expression::*;
use inline_trap::*;
use statement::*;
use trap_hoist::*;
use typing::*;
// `ir::tests` (`type_inference_and_named_arguments.rs`) reaches `default_kind` as `lower::default_kind`.
pub(crate) use captures::lambda_captured_names;
#[cfg(test)]
pub(super) use decls::default_kind;
pub(super) use literal_retype::literal_retyped_argument_types;
pub(super) use statement::{
    collection_iteration_type, make_temp_local_name, match_case_binding, match_expression_type,
};
pub(super) use typing::{expression_type, lambda_param_type};

/// The type environment `ir::lower`'s `expression_type` oracle consults.
///
/// plan-106-A: every type-valued entry is a [`ParameterType`]. The *keys* stay
/// `String` because they are NAMES (functions, bindings, import aliases). Before
/// plan-106-A each of these was a `HashMap<String, String>` and the engine
/// inferred over rendered spellings — the plan-102-C3 staging residue this
/// letter retires.
/// The lowering state: the project-wide tables (borrowed from a
/// [`LowerFacts`]) plus the per-file / per-function cursor. `pub(super)` so
/// `ir::shape` — the pre-lowering shape pass (plan-107-E) — can type HIR
/// expressions with [`expression_type`] against the same tables lowering
/// itself uses, rather than carrying a third inference.
pub(super) struct LowerContext<'a> {
    function_returns: &'a HashMap<String, ParameterType>,
    function_types: &'a HashMap<String, ParameterType>,
    function_params: &'a HashMap<String, Vec<CallParam>>,
    /// Native LINK functions' parameters, keyed `alias.func` (plan-136-A), so a
    /// LINK call that omits a defaulted argument is filled.
    link_params: &'a HashMap<String, Vec<CallParam>>,
    /// plan-121-G: reducers whose body is exactly `RETURN acc & rhs`, so a
    /// `collections::reduce` over one can be rewritten into the loop it is sugar
    /// for. Only the MATCHED shape is carried, never an arbitrary body.
    concat_reducers: HashMap<String, ConcatReducer>,
    binding_types: HashMap<String, ParameterType>,
    bindings: Vec<IrBinding>,
    type_index: &'a TypeIndex,
    pub(super) current_imports: HashMap<String, String>,
    /// Project-relative path of the source file currently being lowered, used to
    /// populate `IrFunction::file` and `ErrorLoc.filename` for generated errors.
    pub(super) current_file: String,
    lambdas: Vec<IrFunction>,
    next_lambda_id: usize,
    /// bug-989: the synthesized converter for each `(actual, expected)` pair a
    /// union-collection conversion needed, by the lifted function's name, so a
    /// pair is emitted once however many positions convert it.
    union_converters: HashMap<(ParameterType, ParameterType), String>,
    /// bug-1021: the synthesized default-value function per (record, omitted field).
    field_defaults: HashMap<(ParameterType, String), String>,
    next_temp_id: usize,
    /// Declared return type of the function currently being lowered, used to
    /// implicitly wrap a `RETURN`ed member constructor into its union (so the
    /// wrap is explicit in the IR rather than re-derived during codegen).
    pub(super) current_return_type: Option<ParameterType>,
    /// The `FOR`/`FOR EACH` loops enclosing the statement being lowered, innermost last:
    /// (source variable, IR loop name). A named EXIT/CONTINUE resolves its variable here.
    /// Taken and restored at every function and lambda boundary, so a loop never
    /// targets an exit outside its own function.
    pub(super) loop_targets: Vec<(String, String)>,
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
    /// Which call targets can raise (bug-457). The inline-`TRAP` desugar needs
    /// this to decide which *nested* calls in the trapped expression get their
    /// own `CallResult` check; see [`super::fallible`].
    fallible: &'a Fallibility,
}

/// A type an imported (non-builtin) package exports, decoded from its `.mfp` so
/// IR lowering can *type* accesses to its fields. Without this, a consumer names
/// an imported type but has no field/variant layout for it, so every
/// `record.field` on an imported record or union variant lowers to `Unknown` —
/// tolerated by `getOr`/`len` but not by `collections::keys`/`values`, which need
/// the element type. Built-in packages need no entry here: their source is folded
/// into the AST by `augmented_project`, so their types are already in `TypeIndex`.
/// An exported top-level `LET`/`MUT` of an imported (non-builtin) package,
/// decoded from its `.mfp` GLOBAL table (bug-551).
///
/// `mfb spec language modules-and-packages` §13 makes an `EXPORT LET`/`EXPORT
/// MUT` importer-visible and covers it by the qualified-name rule ("variables
/// and constants" heads its list of name kinds). The symbol half already worked
/// — `resolver::packages::install_package_type_names` unions the GLOBAL table
/// into the package's visible surface — but nothing carried the declared TYPE,
/// so `pkg::Answer` typed as `Unknown` and died at its use site with
/// `TYPE_UNKNOWN_VALUE`, or reached an unrelated call as an `(Unknown)`
/// argument.
///
/// `name` is the spelling a CONSUMER writes it by: `package.Name`, which is
/// exactly what `ir::package::package_qualified_reference_names` collects and
/// `apply_package_identity` rewrites to the merged `<id>.package.Name`. So a
/// read lowered against this name needs no further lowering support — the merge
/// already re-points it at the definition.
#[derive(Clone)]
pub struct ImportedGlobal {
    pub name: String,
    pub type_: ParameterType,
    /// `EXPORT MUT` (assignable) rather than `EXPORT LET`. Carried so
    /// `ir::verify` refuses an importer's write to a constant with the same rule
    /// it applies to the project's own bindings.
    pub mutable: bool,
}

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
    /// plan-111-B: typed. It is decoded from the imported package's `.mfp` type
    /// table by `src/manifest/package.rs` (boundary #5), which is where the
    /// spelling stops being one.
    pub type_: crate::types::ParameterType,
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
    imported_globals: &[ImportedGlobal],
) -> IrProject {
    let augmented = crate::codegen::registry::registry()
        .augment_project(ast, false)
        .expect("clean-room registry package source must parse");

    // `term`'s injected source (the registry-modeled `LineStyle`/`FillStyle` enums)
    // and the `term`↔`astrings` `drawText(AttributedString)` bridge are injected by
    // the clean-room `registry::augment_project` above (the package's `get_mfb`
    // assembly, and the bridge as a `WhenBothImported("term", "astrings")` gated
    // helper chunk).
    // `astrings`' injected source is emitted by the clean-room
    // `registry::augment_project` above (plan-99 PART C) whenever a program
    // `IMPORT astrings`.
    // app + datetime + money source is injected by the clean-room
    // `registry::augment_project` above.
    // `vector` source (its nine `TYPE`s + `__vector_*` FUNC bodies) is injected by the
    // clean-room `registry::augment_project` above.
    // `http` before `net`: http's injected source imports `net`, so net's late pass
    // must run after http's to see the transitive `IMPORT net` (plan-03-http.md Phase 4).
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
    // sees crypto's injected `IMPORT encoding`).
    // `strings`' scalar-seam companion (which `IMPORT encoding`s, plan-41-D) is
    // injected by the clean-room `registry::augment_project` above (plan-99 PART B),
    // as a `WhenUsed` gated helper — before this `encoding` late pass, so
    // `encoding::uses_package` still sees the seam's transitive `IMPORT encoding`.
    let augmented = crate::codegen::builtins::encoding::augmented_project(&augmented)
        .expect("built-in encoding package source must parse");
    // `color` after the generic pass: canvas's injected companion carries
    // `IMPORT color` and calls `color::toLinear`/`fromLinear` from its blend and
    // gradient helpers, which the generic pass over the pre-injection AST cannot
    // see (plan-122-B).
    let augmented = crate::codegen::builtins::color::augmented_project(&augmented)
        .expect("built-in color package source must parse");
    // `compress` after it: canvas's injected PNG decoder calls `compress::zlibDecode`
    // (plan-137-C).
    let augmented = crate::codegen::builtins::compress::augmented_project(&augmented)
        .expect("built-in compress package source must parse");
    // `vector` after it: canvas's injected companion imports it (plan-172-B).
    let augmented = crate::codegen::builtins::vector::augmented_project(&augmented)
        .expect("built-in vector package source must parse");
    let mut ir = lower_augmented_project(
        &crate::hir::elaborate(&augmented),
        entry,
        external_signatures,
        imported_types,
        imported_globals,
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
    imported_globals: &[ImportedGlobal],
) -> IrProject {
    let augmented =
        crate::resolver::augment_hir_project(concrete).expect("built-in package source must parse");
    let mut ir = lower_augmented_project(
        &augmented,
        entry,
        external_signatures,
        imported_types,
        imported_globals,
    );
    ir.docs = collect_project_docs(
        &crate::resolver::augment_project(source, false)
            .expect("built-in package source must parse"),
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
    imported_globals: &[ImportedGlobal],
) -> IrProject {
    let mut types = Vec::new();
    let mut functions = Vec::new();
    let facts = lower_facts(hir, external_signatures, imported_types, imported_globals);
    let type_index = &facts.type_index;
    let mut context = facts.context();
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
                    types.push(lower_type(type_decl, type_index, &context.current_file))
                }
                // A LINK block's native functions are surfaced to package metadata
                // separately (plan-link-update.md §10) and are not lowered to
                // ordinary IR functions; only their computed parameter defaults
                // get hidden default functions (plan-136-A).
                HirItem::Link(link) => lower_link_defaults(link, &mut context),
                // Native LINK resource declarations and re-export aliases carry no
                // executable body.
                HirItem::Resource(_) | HirItem::FuncAlias(_) => {}
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
        canvas_auto_paint_ms: crate::manifest::DEFAULT_CANVAS_AUTO_PAINT_MS,
        selected_config: Default::default(),
    }
}

/// The project-wide tables lowering computes before it touches a single
/// statement: declared/imported function signatures, the top-level binding
/// types (declared and inferred), and the record/union/enum index.
///
/// Owned separately from [`LowerContext`] so the pre-lowering shape pass
/// (`ir::shape`, plan-107-E) can build the same context lowering does — from
/// the same inputs the build path hands `lower_augmented_project` — and type HIR
/// expressions with lowering's own inference.
pub(super) struct LowerFacts {
    function_returns: HashMap<String, ParameterType>,
    function_types: HashMap<String, ParameterType>,
    function_params: HashMap<String, Vec<CallParam>>,
    link_params: HashMap<String, Vec<CallParam>>,
    /// plan-121-G: reducers recognized as `RETURN acc & rhs`.
    concat_reducers: HashMap<String, ConcatReducer>,
    binding_types: HashMap<String, ParameterType>,
    type_index: TypeIndex,
    /// Project-wide fallibility verdicts (bug-457), computed once here so both
    /// lowering and the shape pass read the same answers.
    fallible: Fallibility,
}

impl LowerContext<'_> {
    /// A fresh generated local name. plan-121-G's fold rewrite binds an
    /// accumulator and a loop variable that must not collide with anything the
    /// source declared, so both come from here.
    fn next_temp_name(&mut self, tag: &str) -> String {
        let id = self.next_temp_id;
        self.next_temp_id += 1;
        format!("$g_{tag}{id}")
    }

    /// The declared or inferred type of a top-level binding, for the shape
    /// pass's call rules (a global of FUNC type is callable like a local).
    pub(super) fn binding_type(&self, name: &str) -> Option<&ParameterType> {
        self.binding_types
            .get(name)
            // bug-551: an imported package's exported globals are keyed by the
            // canonical `package.Name`, so an `IMPORT pkg AS p` write target
            // (`p.Name`) has to resolve the alias before it is called unknown.
            .or_else(|| self.binding_types.get(&canonical_import_name(name, self)))
    }

    /// bug-695: whether `type_` names a `RESOURCE` this project declares.
    pub(super) fn is_declared_resource(&self, type_: &ParameterType) -> bool {
        self.type_index.resources.contains(type_)
    }

    /// The declared-kind oracle (plan-140-A) this context reads, for the built-in
    /// censuses that answer per overload. bug-679 needs it in `ir::fallible`, where
    /// `toInt(<enum>)`'s infallibility cannot be seen from the `ParameterType`.
    /// Returned as the trait object, not the concrete `TypeIndex`, which is
    /// private to this module: the caller only ever asks it a kind question.
    pub(super) fn type_index(&self) -> &dyn builtins::TypeKinds {
        self.type_index
    }
}

impl LowerFacts {
    /// A fresh lowering context over these facts, positioned before the first
    /// file (no imports, no current file, no function cursor).
    pub(super) fn context(&self) -> LowerContext<'_> {
        LowerContext {
            function_returns: &self.function_returns,
            function_types: &self.function_types,
            function_params: &self.function_params,
            link_params: &self.link_params,
            concat_reducers: self.concat_reducers.clone(),
            binding_types: self.binding_types.clone(),
            type_index: &self.type_index,
            current_imports: HashMap::new(),
            current_file: String::new(),
            bindings: Vec::new(),
            lambdas: Vec::new(),
            next_lambda_id: 0,
            union_converters: HashMap::new(),
            field_defaults: HashMap::new(),
            next_temp_id: 0,
            current_return_type: None,
            loop_targets: Vec::new(),
            recover_targets: Vec::new(),
            mutable_locals: HashSet::new(),
            nonescaping_callback: false,
            current_loc: IrSourceLoc::default(),
            fallible: &self.fallible,
        }
    }
}

/// Compute the [`LowerFacts`] for `hir` — the prologue of
/// [`lower_augmented_project`], including the top-level binding type
/// inference that runs before any binding or function is lowered.
pub(super) fn lower_facts(
    hir: &crate::hir::HirProject,
    external_signatures: &HashMap<String, ExternalSignature>,
    imported_types: &[ImportedTypeDef],
    imported_globals: &[ImportedGlobal],
) -> LowerFacts {
    let mut function_returns = function_returns(hir);
    let mut function_types = function_types(hir);
    let mut function_params = function_params(hir);
    let mut binding_types = declared_binding_types(hir);
    // bug-551: the imported packages' exported globals, under the `package.Name`
    // spelling a consumer reads them by. The project's own declarations win — an
    // importer never overrides a binding it can see the source of — and a
    // package name cannot collide with a local binding anyway, since a local
    // name has no dot in it.
    for global in imported_globals {
        binding_types
            .entry(global.name.clone())
            .or_insert_with(|| global.type_.clone());
    }
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
                    // plan-136-B: an imported call that omits an argument passes the
                    // exporting package's default exactly as a local call does.
                    default: match &param.default {
                        ExternalDefault::None => None,
                        ExternalDefault::Literal { type_, value } => {
                            Some(CallDefault::Constant(IrValue::Const {
                                type_: type_.clone(),
                                value: value.clone(),
                            }))
                        }
                        ExternalDefault::Function(name) => {
                            Some(CallDefault::Function(name.clone()))
                        }
                    },
                })
                .collect(),
        );
        function_returns.insert(name.clone(), signature.returns.clone());
    }
    let type_index = TypeIndex::new(hir, imported_types);
    let mut facts = LowerFacts {
        function_returns,
        function_types,
        function_params,
        link_params: link_params(hir),
        concat_reducers: concat_reducers(hir),
        binding_types,
        type_index,
        // Filled in below. bug-486: the fallibility fixpoint needs a type oracle
        // (the built-in census answers per overload, not per name), and a context
        // can only be built once the rest of the facts exist — so this starts empty
        // and is replaced rather than computed in the initializer as it used to be.
        fallible: Fallibility::default(),
    };
    // Inference reads the declared tables through a context and writes the
    // inferred binding types back; the throwaway context borrows `facts`, so the
    // result is moved out through it before the facts are handed back.
    let mut context = facts.context();
    infer_binding_types(hir, &mut context);
    let binding_types = std::mem::take(&mut context.binding_types);
    facts.binding_types = binding_types;
    // Now that every binding is typed, run the fallibility fixpoint through a
    // context over the completed facts. `expression_type` never consults
    // `fallible`, so the placeholder above cannot bias the verdicts it produces.
    let mut context = facts.context();
    let fallible = super::fallible::analyze(hir, &mut context);
    drop(context);
    facts.fallible = fallible;
    facts
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
    /// How a call that omits this argument fills it (plan-136-A).
    default: Option<CallDefault>,
}

/// How a call that omits a defaulted argument fills it (plan-136-A).
#[derive(Clone)]
enum CallDefault {
    /// A [`DefaultKind::Literal`] default, lowered at the call site typed by the
    /// parameter. It names nothing a caller could capture; a package constant is
    /// stored under the canonical `package.NAME` of the DECLARING file's imports.
    Literal(HirExpression),
    /// A [`DefaultKind::Computed`] default: a zero-argument call to this hidden
    /// default function.
    Function(String),
    /// An imported package's literal default, already the `IrValue::Const` the
    /// exporting package stored (plan-136-B). It is passed as-is: re-lowering its
    /// text could classify a number differently from the declaration.
    Constant(IrValue),
}

/// The two kinds of parameter default (plan-136-A).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DefaultKind {
    /// A string, number, scalar or boolean literal, `NOTHING`, or a built-in
    /// package constant: exactly the forms `lower_expression_with_expected`
    /// lowers to a scalar `IrValue::Const`. It names nothing, so the scope it is
    /// lowered in cannot matter, and it lowers at the call site as it always did.
    Literal,
    /// Everything else: lowered once into a hidden default function.
    Computed,
}

#[derive(Clone)]
struct CapturedLocal {
    name: String,
    type_: ParameterType,
}

/// Lowering's picture of the module's declared types — the front-end twin of
/// codegen's `TypeModel`.
///
/// plan-111-B: keyed BY THE TYPE. A declared type is a nominal, so a key is
/// `ParameterType::named(<decl name>)` — the same interned `Symbol` a `Named`
/// out of `parse` carries, so a lookup with a type in hand renders nothing.
struct TypeIndex {
    records: HashMap<ParameterType, Vec<IrField>>,
    enums: HashMap<ParameterType, Vec<String>>,
    /// A union VARIANT type -> the union type that declares it.
    variants: HashMap<ParameterType, ParameterType>,
    variant_unions: HashMap<ParameterType, HashSet<ParameterType>>,
    variant_fields: HashMap<ParameterType, Vec<IrField>>,
    /// The `RESOURCE` types this project declares (bug-695's hoist asks).
    resources: std::collections::HashSet<ParameterType>,
}

/// plan-140-B: the built-in resolver's enum oracle over the declared and
/// imported enums. A qualified built-in spelling falls back to its bare leaf.
impl builtins::TypeKinds for TypeIndex {
    fn is_enum(&self, t: &ParameterType) -> bool {
        self.enums.contains_key(t) || {
            let name = t.name();
            let bare = builtins::builtin_qualified_bare_leaf(&name);
            bare != name.as_ref() && self.enums.contains_key(&ParameterType::named(bare))
        }
    }

    /// bug-988: exactly the membership `wrap_union_value` wraps by, so a generic
    /// call accepts a variant only where lowering will tag it.
    fn union_accepts(&self, union: &ParameterType, actual: &ParameterType) -> bool {
        self.variant_belongs_to_union(actual, union)
    }
}

impl TypeIndex {
    /// Record types with at least one `RES`-marked field (plan-114-C).
    ///
    /// The resource escape analysis needs this because a record binding's
    /// declared type is a bare nominal — `Named("Holder")` cannot say whether
    /// `Holder` owns a resource, the way `List OF RES File` says it structurally.
    ///
    /// Only a **direct** `RES` field counts, not one reached through a nested
    /// record. A resource inside a nested record floats to *that* record's
    /// binding, so the outer type is not the container the ordering gate is
    /// asking about.
    fn res_field_record_types(&self) -> HashSet<ParameterType> {
        self.records
            .iter()
            .filter(|(_, fields)| {
                fields
                    .iter()
                    .any(|field| matches!(field.type_, ParameterType::Res(_)))
            })
            .map(|(type_, _)| type_.clone())
            .collect()
    }

    fn new(hir: &HirProject, imported_types: &[ImportedTypeDef]) -> Self {
        let mut records = HashMap::new();
        let mut enums = HashMap::new();
        let mut variants = HashMap::new();
        let mut variant_unions = HashMap::<ParameterType, HashSet<ParameterType>>::new();
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
                            ParameterType::declared(&type_decl.name),
                            type_decl.fields.iter().map(lower_field).collect(),
                        );
                    }
                    TypeDeclKind::Union => {
                        for variant in
                            expanded_union_variants(type_decl, &union_decls, &mut HashSet::new())
                        {
                            let variant_type = variant.type_.clone();
                            let union_type = ParameterType::declared(&type_decl.name);
                            variants
                                .entry(variant_type.clone())
                                .or_insert_with(|| union_type.clone());
                            variant_unions
                                .entry(variant_type.clone())
                                .or_default()
                                .insert(union_type);
                            variant_fields.insert(
                                variant_type.clone(),
                                records.get(&variant_type).cloned().unwrap_or_default(),
                            );
                        }
                    }
                    TypeDeclKind::Enum => {
                        enums.insert(
                            ParameterType::declared(&type_decl.name),
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
        // plan-111-B: `ImportedTypeField.type_` arrives typed — the `.mfp`
        // decode parses it in `src/manifest/package.rs` (boundary #5), where the
        // package entry is read.
        let imported_field = |field: &ImportedTypeField| IrField {
            visibility: None,
            name: field.name.clone(),
            type_: field.type_.clone(),
            loc: IrSourceLoc::default(),
        };
        for imported in imported_types {
            let imported_type = ParameterType::declared(&imported.name);
            match imported.kind {
                ImportedTypeKind::Record => {
                    records
                        .entry(imported_type)
                        .or_insert_with(|| imported.fields.iter().map(imported_field).collect());
                }
                ImportedTypeKind::Enum => {
                    enums
                        .entry(imported_type)
                        .or_insert_with(|| imported.members.clone());
                }
                ImportedTypeKind::Union => {
                    for variant in &imported.variants {
                        let variant_type = ParameterType::declared(&variant.name);
                        variants
                            .entry(variant_type.clone())
                            .or_insert_with(|| imported_type.clone());
                        variant_unions
                            .entry(variant_type.clone())
                            .or_default()
                            .insert(imported_type.clone());
                        variant_fields
                            .entry(variant_type)
                            .or_insert_with(|| variant.fields.iter().map(imported_field).collect());
                    }
                }
            }
        }
        let resources = super::lower_link::native_resource_types(hir);
        Self {
            records,
            enums,
            variants,
            variant_unions,
            variant_fields,
            resources,
        }
    }

    fn constructor_result(&self, type_: &ParameterType) -> Option<ParameterType> {
        // plan-111-B: `Error` is a nominal (it has no variant), so this is an
        // interned-`Symbol` compare rather than name equality.
        if type_.is_named("Error") {
            Some(ParameterType::named("Error"))
        } else if self.records.contains_key(type_) {
            Some(type_.clone())
        } else {
            // A union variant's owning union, held as a NAME (the index is keyed by
            // variant name); it denotes a nominal.
            self.variants.get(type_).cloned()
        }
    }

    /// The type a field READ yields.
    ///
    /// plan-114-E: the field's declared type with its top-level `RES ` marker
    /// stripped, leaving `Stateful { base, state }` (or the bare resource when
    /// the field carries no `STATE`). The value is unchanged — the slot holds
    /// the handle pointer either way; only the spelling differs.
    ///
    /// **This is the one place to strip it.** `IrValue::MemberAccess` carries an
    /// annotated `type_` produced from here, and `ir::verify`'s `infer_type`
    /// *prefers that annotation* over re-resolving the field
    /// (`ir/verify/env.rs:infer_type_depth`) — so stripping only in the verifier or only in
    /// codegen leaves the IR itself saying `RES fs.File` while its consumers say
    /// otherwise. Stripping at the annotation site keeps the `.ir`, the type
    /// checker and codegen telling the same story.
    ///
    /// Why it is needed at all: `split_state` matches `Stateful` only at the top
    /// level (`src/types.rs:629`), so `Res(Stateful{..}).state()` is `None` and
    /// `h.handle.state` is refused with `TYPE_STATE_INVALID` claiming
    /// "`fs.File` here has no STATE" — a message naming the wrong problem.
    ///
    /// Unconditional, exactly as a collection element does it
    /// (`list_element("List OF RES Socket") == "Socket"`): one rule for both
    /// positions keeps them from drifting.
    fn record_field_type(&self, type_: &ParameterType, member: &str) -> Option<ParameterType> {
        self.records
            .get(type_)
            .or_else(|| self.variant_fields.get(type_))?
            .iter()
            .find(|field| field.name == member)
            .map(|field| match &field.type_ {
                ParameterType::Res(inner) => (**inner).clone(),
                other => other.clone(),
            })
    }

    fn variant_belongs_to_union(
        &self,
        variant: &ParameterType,
        union_type: &ParameterType,
    ) -> bool {
        // bug-1018: which variant a value is does not depend on the `RES` alias
        // marker a slot spells it with or on a `STATE` clause it carries (`STATE`
        // agreement is its own check). The table is keyed on the bare types, so
        // this is the one place the question is normalized; no caller strips.
        self.variant_unions
            .get(&bare_variant(variant))
            .is_some_and(|unions| unions.contains(&bare_variant(union_type)))
    }

    /// bug-989: whether a collection of type `actual` converts to `expected`
    /// element by element — a `List` whose element, or a `Map` (same key type)
    /// whose value, is a variant of the expected union or itself such a
    /// collection. Equal types do not convert (there is nothing to do), and a
    /// `Set` element or `Map` key never does: a union is not comparable, so no
    /// `Set OF <union>` or union-keyed `Map` exists to convert to.
    pub(super) fn union_collection_converts(
        &self,
        actual: &ParameterType,
        expected: &ParameterType,
    ) -> bool {
        let part = |from: &ParameterType, to: &ParameterType| {
            self.variant_belongs_to_union(from, to) || self.union_collection_converts(from, to)
        };
        match (actual, expected) {
            (ParameterType::ListOf(from), ParameterType::ListOf(to)) => part(from, to),
            (ParameterType::MapOf(from_key, from), ParameterType::MapOf(to_key, to)) => {
                from_key == to_key && part(from, to)
            }
            _ => false,
        }
    }
}

fn expanded_union_variants<'a>(
    type_decl: &'a HirTypeDecl,
    union_decls: &HashMap<String, &'a HirTypeDecl>,
    visiting: &mut HashSet<String>,
) -> Vec<&'a crate::hir::HirUnionVariant> {
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
        if let Some(included) = union_decls.get(include.name().as_ref()) {
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
