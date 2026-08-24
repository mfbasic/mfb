use crate::ast::{TypeDeclKind, UnionVariant};
use crate::hir::{
    HirCallArg, HirConstructorArg, HirExpression, HirFile, HirFunction, HirItem, HirMatchCase,
    HirMatchPattern, HirParam, HirProject, HirRecordUpdate, HirStatement, HirTopLevelBinding,
    HirTypeDecl, HirTypeField,
};
use crate::numeric;
use crate::rules;
use crate::types::ParameterType;
use std::collections::{HashMap, HashSet};
use std::path::Path;

pub fn monomorphize_project(project_dir: &Path, hir: &HirProject) -> Result<HirProject, ()> {
    // NOTE: `hir` is the elaborated mirror of the already-augmented AST (the
    // builtin package sources injected by `resolver::augment_project`, run before
    // this in the build), so the overload machinery below can mangle a builtin's
    // native overload set. Monomorphization walks and produces HIR (plan-102-D3);
    // every type field carries a `ParameterType` read back with `.name()`, which
    // round-trips byte-exact, so the concrete HIR is byte-identical to elaborating
    // the pre-D3 concrete AST.
    let mut mono = Monomorphizer::new(project_dir, hir);
    mono.run();
    if mono.had_error {
        Err(())
    } else {
        Ok(mono.into_project())
    }
}

struct Monomorphizer<'a> {
    project_dir: &'a Path,
    source: &'a HirProject,
    type_templates: HashMap<String, HirTypeDecl>,
    function_templates: HashMap<String, HirFunction>,
    concrete_types: HashMap<String, HirTypeDecl>,
    concrete_functions: HashMap<String, HirFunction>,
    function_overloads: HashMap<String, Vec<HirFunction>>,
    overload_names: HashMap<String, String>,
    /// Overloaded functions exported by imported packages, keyed by the
    /// importer-facing `binding.base` name. Lets a call to an imported overload
    /// be rewritten to the package's mangled `package.base$Types` name, which the
    /// package merge then identity-prefixes (plan-linker.md §12, overloads).
    imported_overloads: HashMap<String, Vec<ImportedOverload>>,
    /// All known import-binding/package qualifier prefixes (e.g. `sqlite.`), used
    /// to normalize an argument's qualified user/resource type to the bare name
    /// the package stored in its mangled overload names.
    package_qualifiers: Vec<String>,
    type_instantiations: HashMap<String, (String, Vec<String>)>,
    emitted_type_keys: HashSet<String>,
    emitted_function_keys: HashSet<String>,
    /// Claimed concrete symbol -> the unambiguous `name<args>` key that owns it.
    /// `mangle_name` is lossy, so this detects a symbol collision between two
    /// distinct type-argument tuples and lets the loser be suffixed (bug-226).
    concrete_symbol_keys: HashMap<String, String>,
    /// Import-binding names that refer to the built-in `collections` package
    /// (including aliases). A call `binding.member` with `binding` in this set
    /// and `member` a `collections::` function is rewritten to the internal
    /// generic implementation `__collections_member` before instantiation.
    collections_bindings: HashSet<String>,
    /// Source-file path (project-relative) for each declared function, keyed by
    /// both its original and concrete/mangled name. Lets a monomorph diagnostic
    /// be attributed to the file the offending function actually lives in rather
    /// than always the first project file (bug-107).
    function_files: HashMap<String, String>,
    /// The file whose function body is currently being lowered, if known;
    /// diagnostics are attributed here. Saved/restored across nested
    /// instantiation so the attribution follows the frame being lowered.
    current_file: Option<String>,
    template_instantiation_depth: usize,
    /// Count of concrete generic instantiations (functions + user types) actually
    /// lowered so far. Bounds *wide* fan-out the per-path `template_instantiation_
    /// depth` cap cannot: a generic that recurses through ≥2 distinct type-widening
    /// self-calls fans into an exponential tree of distinct `name<args>` keys, none
    /// of which the depth cap collapses (bug-399). Checked against
    /// `MAX_TOTAL_INSTANTIATIONS` at each instantiation entry point.
    total_instantiations: usize,
    /// Set once any instantiation limit (the total budget or the depth cap) trips,
    /// so every subsequent instantiation short-circuits without recursing or
    /// re-reporting — halting the enumeration after a single bounded diagnostic
    /// instead of exploring the remaining (exponential) tree (bug-399).
    instantiation_limit_reached: bool,
    had_error: bool,
}

/// One overload of an imported package function.
struct ImportedOverload {
    /// Declared parameter types in order (bare, as the package stored them).
    param_types: Vec<String>,
    /// The fully package-qualified mangled name (`package.base$Types`) the merge
    /// expects.
    qualified_name: String,
}

#[derive(Default)]
struct FunctionContext {
    locals: HashMap<String, String>,
    function_returns: HashMap<String, String>,
    function_types: HashMap<String, String>,
    record_fields: HashMap<String, Vec<HirTypeField>>,
    /// Declared type of each top-level `LET`/`MUT` binding, keyed by name. Lets
    /// `expression_type` resolve an identifier that names a global so a generic /
    /// overloaded call taking that global infers its type instead of being falsely
    /// rejected (bug-103).
    globals: HashMap<String, String>,
    /// Declared return type of the function whose body is being lowered. Supplies
    /// the expected (contextual) type for a `RETURN` operand so a return-type
    /// overload set resolves there (plan-01-overload.md §F.2).
    enclosing_return: Option<String>,
}

impl Clone for FunctionContext {
    fn clone(&self) -> Self {
        Self {
            locals: self.locals.clone(),
            function_returns: self.function_returns.clone(),
            function_types: self.function_types.clone(),
            record_fields: self.record_fields.clone(),
            globals: self.globals.clone(),
            enclosing_return: self.enclosing_return.clone(),
        }
    }
}

mod helpers;
mod lower;

use helpers::*;

const MAX_TEMPLATE_INSTANTIATION_DEPTH: usize = 256;

/// Global ceiling on the total number of concrete generic instantiations
/// (functions + user types) monomorphization will lower for one project. The
/// depth cap bounds a single recursion *path*; this bounds the *breadth* of a
/// fan-out — a generic recursing through ≥2 type-widening self-calls produces an
/// exponential tree of distinct `name<args>` keys that the per-leaf depth cap
/// never halts (bug-399). A few thousand is far above any hand-written generic
/// program yet stops the fan-out promptly with a single bounded diagnostic.
const MAX_TOTAL_INSTANTIATIONS: usize = 4096;
