use super::super::{ImportedOverload, Monomorphizer};
use crate::ast::AstProject;
use crate::types::ParameterType;

// From a part, `super` is `lower::tests`; part bodies spell `super::mangle_name` and
// `super::strip_qualifier_prefixes`, which live in `lower`.
use super::{mangle_name, strip_qualifier_prefixes};

mod instantiation_overloads_and_inference;
mod references_and_template_overloads;
mod structural_helpers;
mod type_compat_imports_and_placement;

/// Parse one or more `(relative_path, source)` files into an `AstProject`.
fn project(files: &[(&str, &str)]) -> AstProject {
    let ast_files = files
        .iter()
        .map(|(path, src)| {
            crate::ast::parse_source(std::path::Path::new(path), path, src).expect("parse source")
        })
        .collect::<Vec<_>>();
    AstProject {
        name: "testpkg".to_string(),
        files: ast_files,
    }
}

/// Monomorphize a single `main.mfb` source, returning `Ok(project)` or the
/// error flag. Diagnostics are silenced so error-path tests stay quiet.
///
/// plan-106-D: returns the concrete HIR the monomorphizer actually produces.
/// It used to be de-elaborated back to an AST purely so these assertions could
/// read it — a test-only backward path, which is how backward paths come back.
fn monomorphize(src: &str) -> Result<crate::hir::HirProject, ()> {
    monomorphize_files(&[("src/main.mfb", src)])
}

fn monomorphize_files(files: &[(&str, &str)]) -> Result<crate::hir::HirProject, ()> {
    let ast = project(files);
    let dir = std::env::temp_dir();
    // Silence the front end's diagnostic printing during error-path tests.
    crate::testutil::silence_panics(|| {
        super::super::monomorphize_project(&dir, &crate::hir::elaborate(&ast))
    })
}

fn functions(project: &crate::hir::HirProject) -> Vec<&crate::hir::HirFunction> {
    project
        .files
        .iter()
        .flat_map(|f| &f.items)
        .filter_map(|item| match item {
            crate::hir::HirItem::Function(function) => Some(function),
            _ => None,
        })
        .collect()
}

fn types(project: &crate::hir::HirProject) -> Vec<&crate::hir::HirTypeDecl> {
    project
        .files
        .iter()
        .flat_map(|f| &f.items)
        .filter_map(|item| match item {
            crate::hir::HirItem::Type(type_decl) => Some(type_decl),
            _ => None,
        })
        .collect()
}

fn function_names(project: &crate::hir::HirProject) -> Vec<String> {
    functions(project).iter().map(|f| f.name.clone()).collect()
}
