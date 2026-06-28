use crate::ast::{
    AstFile, AstProject, CallArg, ConstructorArg, Expression, Function, Item, MatchCase,
    MatchPattern, RecordUpdate, Statement, TopLevelBinding, TypeDecl, TypeDeclKind, TypeField,
    UnionVariant,
};
use crate::numeric;
use crate::rules;
use std::collections::{HashMap, HashSet};
use std::path::Path;

pub fn monomorphize_project(project_dir: &Path, ast: &AstProject) -> Result<AstProject, ()> {
    let mut mono = Monomorphizer::new(project_dir, ast);
    mono.run();
    if mono.had_error {
        Err(())
    } else {
        Ok(mono.into_project())
    }
}

struct Monomorphizer<'a> {
    project_dir: &'a Path,
    source: &'a AstProject,
    type_templates: HashMap<String, TypeDecl>,
    function_templates: HashMap<String, Function>,
    concrete_types: HashMap<String, TypeDecl>,
    concrete_functions: HashMap<String, Function>,
    function_overloads: HashMap<String, Vec<Function>>,
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
    /// Import-binding names that refer to the built-in `collections` package
    /// (including aliases). A call `binding.member` with `binding` in this set
    /// and `member` a `collections::` function is rewritten to the internal
    /// generic implementation `__collections_member` before instantiation.
    collections_bindings: HashSet<String>,
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
    record_fields: HashMap<String, Vec<TypeField>>,
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
            enclosing_return: self.enclosing_return.clone(),
        }
    }
}

mod helpers;
mod lower;

use helpers::*;
