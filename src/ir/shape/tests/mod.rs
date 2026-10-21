use super::*;
use crate::ast::{parse_source, AstProject};
use crate::ir::IrOp;

mod comparability;
mod export_surface;
mod instantiation_sites;
mod literal_lists;
mod packages_links_testing_and_fields;
mod walker_and_call_arguments;

/// Parse `src` as `main.mfb`, augment it with the builtin package sources
/// (the same chain the build path runs before lowering), and return the
/// concrete HIR.
fn hir_from(src: &str) -> HirProject {
    let file =
        parse_source(Path::new("main.mfb"), "main.mfb", src).expect("test source must lex+parse");
    let project = AstProject {
        name: "test".to_string(),
        files: vec![file],
    };
    // The build's order: registry injection into the AST, then the generic
    // HIR is monomorphized into the concrete program the pass walks.
    let augmented = crate::resolver::augment_project(&project, false)
        .expect("builtin augmentation must succeed");
    crate::monomorph::monomorphize_project(Path::new("."), &crate::hir::elaborate(&augmented))
        .expect("test source must monomorphize")
}

/// The shape pass's rule codes for `src`, in traversal order.
fn shape_codes(src: &str) -> Vec<String> {
    collect_diagnostics(
        Path::new("/proj"),
        &hir_from(src),
        &[],
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
