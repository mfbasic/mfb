//! Shared `#[cfg(test)]` fixtures for the unit-test suite (plan-12).
//!
//! These build the common source → AST → IR pipeline objects that most
//! front-end and codegen unit tests need, so individual `mod tests` blocks
//! don't each re-derive the same boilerplate. Keep helpers here small and
//! composable; anything file-specific stays in that file's own test module.

#![cfg(test)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::ast::{parse_source, AstFile, AstProject};
use crate::ir::{self, IrProject};

/// Locate a committed test fixture directory by its leaf name, searching
/// recursively under `tests/`. After the tests reorganization fixtures live
/// under `tests/{syntax,rt-error,rt-behavior}/<feature>/<name>` (plus the
/// `tests/acceptance` app), and leaf names are unique — so a by-name search
/// keeps unit tests independent of the exact bucket/feature a fixture lives in.
/// Panics if no matching fixture directory (one holding a `project.json`)
/// exists.
pub fn fixture_dir(name: &str) -> PathBuf {
    fn find(dir: &Path, name: &str) -> Option<PathBuf> {
        for entry in std::fs::read_dir(dir).ok()?.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if entry.file_name() == *name && path.join("project.json").is_file() {
                return Some(path);
            }
            if let Some(found) = find(&path, name) {
                return Some(found);
            }
        }
        None
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    find(&root, name).unwrap_or_else(|| panic!("test fixture `{name}` not found under tests/"))
}

/// Parse a single `.mfb` source string into an [`AstFile`], panicking on any
/// parse error (tests that want the error should call `parse_source` directly).
pub fn parse_file(source: &str) -> AstFile {
    parse_source(Path::new("main.mfb"), "main.mfb", source).expect("source should parse")
}

/// Wrap a single source string into a one-file [`AstProject`], appending the
/// compiler-owned prelude (`Pair`, `Partition`) exactly as the real project
/// loader does so the front end sees the always-in-scope generic templates.
/// (Named to avoid colliding with [`crate::ast::parse_project`] under a glob
/// import.)
pub fn project_from_src(source: &str) -> AstProject {
    let project = AstProject {
        name: "test".to_string(),
        files: vec![parse_file(source)],
    };
    // Mirror `ast::manifest::parse_project`: append the prelude last so the
    // user's file stays `files[0]`.
    crate::ast::augment_with_prelude(project)
}

/// Parse and lower a single source string into an [`IrProject`], with no entry
/// point and no external (native `LINK`) functions — the common shape for
/// exercising lowering, serialization, and codegen on hand-written programs.
pub fn lower_src(source: &str) -> IrProject {
    let project = project_from_src(source);
    ir::lower_project_with_external_functions(&project, None, &HashMap::new(), &HashMap::new())
}

/// Run the syntax checker over `src` and return the emitted diagnostic rule
/// codes (in traversal order). An empty vector means the program is accepted.
pub fn check_src(source: &str) -> Vec<String> {
    let project = project_from_src(source);
    let diagnostics = crate::syntaxcheck::check_project_collect(Path::new("."), &project)
        .expect("augmentation should not fail for test sources");
    diagnostics.into_iter().map(|d| d.rule).collect()
}

/// True when the checker accepts `src` with zero diagnostics.
pub fn accepts(source: &str) -> bool {
    check_src(source).is_empty()
}

/// True when the checker rejects `src` with at least one diagnostic whose rule
/// code equals `rule`.
pub fn rejects_with(source: &str, rule: &str) -> bool {
    check_src(source).iter().any(|r| r == rule)
}
