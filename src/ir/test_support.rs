//! Shared test fixtures for the IR unit suites (bug-342 C3). One minimal
//! `IrProject` builder consumed by `ir::tests`, `ir::coverage_tests`, and
//! `ir::verify::tests`, so a new `IrProject` field is threaded in exactly one
//! place instead of three hand-copied literals that had already drifted (three
//! copies disagreed on `IrFunction::kind`).

use super::*;

/// A minimal well-formed `IrProject`: no entry point, no bindings, the given
/// `types` and `functions`, and default everything else. The three former
/// per-suite builders (`project`, `empty_project`, `project_named`) all reduce
/// to this — they built byte-identical shells and differed only in which of
/// `name`/`functions`/`types` they let the caller supply.
pub(crate) fn project_fixture(
    name: &str,
    functions: Vec<IrFunction>,
    types: Vec<IrType>,
) -> IrProject {
    IrProject {
        name: name.to_string(),
        entry: None,
        bindings: vec![],
        types,
        functions,
        native_resources: vec![],
        link_functions: vec![],
        link_cstructs: Vec::new(),
        link_aliases: vec![],
        docs: ProjectDocs::default(),
        native_libraries: Default::default(),
        max_buffer_bytes: crate::manifest::DEFAULT_MAX_BUFFER_MIB * 1024 * 1024,
    }
}
