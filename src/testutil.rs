//! Shared `#[cfg(test)]` fixtures for the unit-test suite (plan-12).
//!
//! These build the common source → AST → IR pipeline objects that most
//! front-end and codegen unit tests need, so individual `mod tests` blocks
//! don't each re-derive the same boilerplate. Keep helpers here small and
//! composable; anything file-specific stays in that file's own test module.

#![cfg(test)]
// Helpers are consumed incrementally as file test modules land; not every one
// is referenced from every build configuration.
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::Path;

use crate::ast::{parse_source, AstFile, AstProject};
use crate::ir::{self, IrProject};

/// Parse a single `.mfb` source string into an [`AstFile`], panicking on any
/// parse error (tests that want the error should call `parse_source` directly).
pub fn parse_file(source: &str) -> AstFile {
    parse_source(Path::new("main.mfb"), "main.mfb", source).expect("source should parse")
}

/// Wrap a single source string into a one-file [`AstProject`] named `main`.
/// (Named to avoid colliding with [`crate::ast::parse_project`] under a glob
/// import.)
pub fn project_from_src(source: &str) -> AstProject {
    AstProject {
        name: "main".to_string(),
        files: vec![parse_file(source)],
    }
}

/// Parse and lower a single source string into an [`IrProject`], with no entry
/// point and no external (native `LINK`) functions — the common shape for
/// exercising lowering, serialization, and codegen on hand-written programs.
pub fn lower_src(source: &str) -> IrProject {
    let project = project_from_src(source);
    ir::lower_project_with_external_functions(&project, None, &HashMap::new(), &HashMap::new())
}

/// A tiny but complete program: an entry `main` that does nothing observable.
/// Useful as a baseline IR for codegen smoke tests.
pub const EMPTY_MAIN: &str = "SUB main\nEND SUB\n";
