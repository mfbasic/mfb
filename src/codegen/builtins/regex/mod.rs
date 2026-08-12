//! Built-in `regex` package (plan-72-T), migrated into the codegen layer
//! (planning/migrate.md). Each public member is source-backed: its `__regex_*`
//! body lives in its `func_*.rs` as `Implementation::Mfb` and is spliced into the
//! injected engine source by `assembled_source()` in place of a
//! `'@@MFB_BODY:<slug>@@` marker; the backtracking engine's private helpers stay
//! in `package.mfb`. regex is the three-file source case: `assembled_source`
//! appends two generated Unicode tables to the engine as one file, exactly as the
//! pre-migration `source_file` did. Both are generated Unicode data shared from
//! the neutral `src/codegen/unicode/` — the Script-property table
//! (`unicode_script_of.mfb`) and the general-category table (`unicode_gencat.mfb`,
//! also used by `strings`). regex is concrete (rewritten in IR lowering), so the
//! `__regex_*` rewrite target comes from the explicit `IMPL_NAMES` table.

use std::path::Path;

use crate::target::shared::registry::{
    BuiltinFunction, BuiltinModule, BuiltinSource, DefaultResolver, DefaultValue, Implementation,
    InjectionRule, Parameter, ParameterType,
};

mod func_find;
mod func_find_all;
mod func_match;
mod func_replace;

const MATCH: &str = "regex.match";
const FIND: &str = "regex.find";
const FIND_ALL: &str = "regex.findAll";
const REPLACE: &str = "regex.replace";
const INTERNAL_MATCH: &str = "__regex_match";
const INTERNAL_FIND: &str = "__regex_find";
const INTERNAL_FIND_ALL: &str = "__regex_findAll";
const INTERNAL_REPLACE: &str = "__regex_replace";

pub(super) const PARAMS_MATCH: &[Parameter] = &[
    Parameter::required("value", "String"),
    Parameter::required("pattern", "String"),
];
// find/findAll(value, pattern, [start=0]) — trailing `start` default-pads to 0.
pub(super) const PARAMS_FIND: &[Parameter] = &[
    Parameter::required("value", "String"),
    Parameter::required("pattern", "String"),
    Parameter {
        name: "start",
        aliases: &[],
        ty: ParameterType::Named("Integer"),
        default: DefaultValue::Fill {
            type_name: "Integer",
            expr: "0",
        },
    },
];
pub(super) const PARAMS_REPLACE: &[Parameter] = &[
    Parameter::required("value", "String"),
    Parameter::required("pattern", "String"),
    Parameter::required("replacement", "String"),
];

// plan-72-T: `REGEX` is the descriptor authority. Each member owns its source body
// in its `func_*.rs` (`Implementation::Mfb`); a call rewrites to the internal
// `__regex_*` name via `IMPL_NAMES` at IR lowering. `WhenImported` source.
const REGEX_FUNCTIONS: &[BuiltinFunction] = &[
    func_match::MATCH,
    func_find::FIND,
    func_find_all::FIND_ALL,
    func_replace::REPLACE,
];

const MODULE_INTRO: &str = r#"Match, search, and replace text with regular expressions"#;
const MODULE_DESC: &str = r#"The `regex` package searches and rewrites text with a single portable
regular-expression dialect that is MFBASIC's own. Its syntax and semantics are
defined entirely by `mfb spec stdlib regex` and produce byte-for-byte identical
results on every target, never deferring to a host libc, locale, or OS regex
library. `regex` is a built-in package: `IMPORT regex` needs no manifest
dependency. For the full pattern language, run `mfb man regex language`.

The package defines no new types. `pattern` and `replacement` are ordinary
runtime `String` values, so they may be literals, built at run time, or read from
input; a pattern is compiled at the moment a function is called. An invalid
pattern fails the call with `ErrInvalidFormat` rather than being silently treated
as "no match". Because MFBASIC `String` literals process their own backslash
escapes, a backslash the regex needs is written `"\\"` in a source literal
(`"\\d"` is the pattern `\d`); a pattern read from a file or user input has no
such doubling.

Matching operates over Unicode scalar values. Every position and index a regex
function accepts or reports is a zero-based Unicode scalar index — never a byte
offset and never a grapheme-cluster index — consistent with `len` and the
`strings` package. A string of `n` scalars has positions `0` through `n`;
position `n` is after the last scalar, so a `start` argument may equal
`len(value)`. All Unicode-dependent behavior (the `\d`/`\w`/`\s` shorthands,
`\p{...}` properties, and `(?i)` case folding) resolves against a single pinned
Unicode version, identical across every target.

The functions differ only in what they report. `match` returns a `Boolean` for
whether the pattern matches anywhere; `find` returns the start index of the first
match at or after `start`, or `-1` when there is none; `findAll` returns a
`List OF Integer` of the start index of every non-overlapping match; and
`replace` returns a new `String` with every non-overlapping match rewritten by a
replacement template. Every search is unanchored and leftmost: the reported match
is the one beginning at the smallest position where any match exists. `find` and
`findAll` take an optional `start` (default `0`) restricting only where a match
may begin — the absolute anchors `\A`, `\z`, and unflagged `^`/`$` are still
evaluated against the whole value. A zero-length match is valid; iteration
advances one scalar past an empty match so it always terminates.

No `regex` function fails on the absence of a match: `match` returns `FALSE`,
`find` returns `-1`, `findAll` returns an empty list, and `replace` returns
`value` unchanged. `ErrNotFound` is never raised by this package. None of the
functions mutate their arguments or have side effects."#;

pub(crate) static REGEX: BuiltinModule = BuiltinModule {
    name: "regex",
    doc_intro: MODULE_INTRO,
    doc_desc: MODULE_DESC,
    functions: REGEX_FUNCTIONS,
    types: &[],
    source: Some(BuiltinSource {
        rule: InjectionRule::WhenImported,
        loader: source_file,
    }),
    resolver: None,
};

pub(crate) fn is_regex_call(name: &str) -> bool {
    DefaultResolver::contains(&REGEX, name)
}

// `call_param_names` returns a `&'static` borrowed shape the owned
// `DefaultResolver::param_names` cannot produce, so it stays a static table,
// PINNED equal to `REGEX` by the parity test until plan-72-BB. Each position has a
// single spelling (no aliases).
pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    match name {
        MATCH => Some(&[&["value"], &["pattern"]]),
        FIND | FIND_ALL => Some(&[&["value"], &["pattern"], &["start"]]),
        REPLACE => Some(&[&["value"], &["pattern"], &["replacement"]]),
        _ => None,
    }
}

// Bespoke `[, Integer]` bracket phrasing for the optional `start` — the
// descriptor's per-position type list renders `String, String, Integer`, so this
// stays hand-authored (the `collections` precedent) and is NOT asserted against
// the descriptor by the parity test.
pub(crate) fn expected_arguments(name: &str) -> Option<&'static str> {
    match name {
        MATCH => Some("String, String"),
        FIND | FIND_ALL => Some("String, String[, Integer]"),
        REPLACE => Some("String, String, String"),
        _ => None,
    }
}

/// The internal `__regex_*` symbol each public member rewrites to during IR
/// lowering. The members carry `Implementation::Mfb` (whose descriptor
/// `implementation_name` is `None`), so the rewrite target is provided here.
const IMPL_NAMES: &[(&str, &str)] = &[
    (MATCH, INTERNAL_MATCH),
    (FIND, INTERNAL_FIND),
    (FIND_ALL, INTERNAL_FIND_ALL),
    (REPLACE, INTERNAL_REPLACE),
];

pub(crate) fn implementation_name(name: &str) -> Option<&'static str> {
    IMPL_NAMES
        .iter()
        .find(|(public, _)| *public == name)
        .map(|(_, internal)| *internal)
}

/// Default trailing arguments injected during IR lowering so the internal
/// `__regex_find`/`__regex_findAll` always receive `start`. Returns a `&'static`
/// slice the owned `DefaultResolver::default_padding` cannot produce, so it stays
/// a static table, PINNED equal to `REGEX`'s trailing `Fill` by the parity test
/// (asserted over every provided count) until plan-72-BB.
pub(crate) fn default_argument_padding(
    name: &str,
    provided: usize,
) -> &'static [(&'static str, &'static str)] {
    const FIND_DEFAULTS: &[(&str, &str)] = &[("Integer", "0")];
    match name {
        // find/findAll(value, pattern, [start=0])
        FIND | FIND_ALL => &FIND_DEFAULTS[provided.saturating_sub(2).min(FIND_DEFAULTS.len())..],
        _ => &[],
    }
}

pub(crate) fn source_file() -> Result<crate::ast::AstFile, ()> {
    crate::ast::parse_source_internal(
        Path::new("<builtin-regex>"),
        "builtins/regex.mfb",
        &assembled_source(),
    )
}

/// The `regex` package source. The engine `package.mfb` is the base: each member's
/// `FUNC __regex_* ... END FUNC` body is spliced in for its `'@@MFB_BODY:<slug>@@`
/// marker at the body's original position (keeping every other engine line's number
/// unchanged). The engine and the two generated Unicode tables — general-category
/// (`src/codegen/unicode/unicode_gencat.mfb`, see `scripts/gen_regex_unicode.py`)
/// and the Script property (`src/codegen/unicode/unicode_script_of.mfb`, see `scripts/gen_regex_scripts.py`, plan-77 R2)
/// — are kept as separate physical files so each table can be regenerated
/// mechanically, but they compile as one source file: MFBASIC `FUNC`s are
/// file-local unless exported, and `PACKAGE` visibility is not valid in an
/// executable, so the engine's calls to `__regex_genCat` / `__regex_scriptOf` /
/// `__regex_scriptCanonName` must be intra-file. Combining them here reproduces the
/// pre-migration `source_file` byte-for-byte.
fn assembled_source() -> String {
    let mut engine = String::from(include_str!("package.mfb"));
    for func in REGEX_FUNCTIONS {
        if let Implementation::Mfb { body, .. } = func.implementation {
            let marker = format!("'@@MFB_BODY:{}@@", func.doc_slug);
            debug_assert!(
                engine.contains(&marker),
                "regex package.mfb is missing the '{marker}' body marker",
            );
            engine = engine.replacen(&marker, body, 1);
        }
    }
    format!(
        "{}\n{}\n{}",
        engine,
        // Both tables are generated Unicode data shared from the neutral
        // `src/codegen/unicode/`: the general-category table (also used by
        // `strings`, which renames `__regex_genCat`) and the Script-property table.
        include_str!("../../unicode/unicode_gencat.mfb"),
        include_str!("../../unicode/unicode_script_of.mfb"),
    )
}

pub(crate) fn uses_package(ast: &crate::ast::AstProject) -> bool {
    ast.files.iter().any(|file| {
        file.imports
            .iter()
            .any(|import| import.package_name() == "regex")
    })
}

pub(crate) fn augmented_project(
    ast: &crate::ast::AstProject,
) -> Result<crate::ast::AstProject, ()> {
    if !uses_package(ast) {
        return Ok(ast.clone());
    }

    let mut augmented = ast.clone();
    augmented.files.push(source_file()?);
    Ok(augmented)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(src: &str) -> crate::ast::AstProject {
        let file = crate::ast::parse_source(std::path::Path::new("main.mfb"), "main.mfb", src)
            .expect("parse source");
        crate::ast::AstProject {
            name: "test".to_string(),
            files: vec![file],
        }
    }

    #[test]
    fn is_call_and_reject() {
        for n in [MATCH, FIND, FIND_ALL, REPLACE] {
            assert!(is_regex_call(n), "{n}");
        }
        assert!(!is_regex_call("regex.nope"));
        assert!(!is_regex_call(INTERNAL_MATCH));
    }

    #[test]
    fn param_names_branches() {
        assert_eq!(
            call_param_names(MATCH),
            Some(&[&["value"][..], &["pattern"]][..])
        );
        assert_eq!(
            call_param_names(FIND),
            Some(&[&["value"][..], &["pattern"], &["start"]][..])
        );
        assert_eq!(call_param_names(FIND), call_param_names(FIND_ALL));
        assert_eq!(
            call_param_names(REPLACE),
            Some(&[&["value"][..], &["pattern"], &["replacement"]][..])
        );
        assert!(call_param_names("regex.nope").is_none());
    }

    #[test]
    fn expected_arguments_branches() {
        assert_eq!(expected_arguments(MATCH), Some("String, String"));
        assert_eq!(expected_arguments(FIND), Some("String, String[, Integer]"));
        assert_eq!(
            expected_arguments(FIND_ALL),
            Some("String, String[, Integer]")
        );
        assert_eq!(expected_arguments(REPLACE), Some("String, String, String"));
        assert!(expected_arguments("regex.nope").is_none());
    }

    #[test]
    fn implementation_name_branches() {
        assert_eq!(implementation_name(MATCH), Some(INTERNAL_MATCH));
        assert_eq!(implementation_name(FIND), Some(INTERNAL_FIND));
        assert_eq!(implementation_name(FIND_ALL), Some(INTERNAL_FIND_ALL));
        assert_eq!(implementation_name(REPLACE), Some(INTERNAL_REPLACE));
        assert!(implementation_name("regex.nope").is_none());
    }

    #[test]
    fn default_padding_branches() {
        assert_eq!(default_argument_padding(FIND, 2).len(), 1);
        assert_eq!(default_argument_padding(FIND, 3).len(), 0);
        assert_eq!(default_argument_padding(FIND_ALL, 2).len(), 1);
        assert_eq!(default_argument_padding(MATCH, 2), &[]);
    }

    #[test]
    fn source_file_parses() {
        assert!(source_file().is_ok());
    }

    #[test]
    fn augmented_project_injects_when_imported() {
        let ast = project("IMPORT regex\nSUB main\nEND SUB\n");
        assert!(uses_package(&ast));
        assert_eq!(
            augmented_project(&ast).expect("a").files.len(),
            ast.files.len() + 1
        );
    }

    #[test]
    fn augmented_project_noop_without_import() {
        let ast = project("SUB main\nEND SUB\n");
        assert!(!uses_package(&ast));
        assert_eq!(
            augmented_project(&ast).expect("a").files.len(),
            ast.files.len()
        );
    }

    #[test]
    fn descriptor_owns_source_body_and_rewrite() {
        // The member descriptor carries its MFBASIC body (Mfb) and rewrites to the
        // internal `__regex_*` name via `IMPL_NAMES`, not a descriptor rewrite.
        let m = func_match::MATCH;
        assert_eq!(m.name, MATCH);
        assert_eq!(m.doc_slug, "match");
        assert!(matches!(m.implementation, Implementation::Mfb { .. }));
        assert_eq!(implementation_name(MATCH), Some(INTERNAL_MATCH));
    }

    #[test]
    fn augmented_project_pushes_injected_source_file() {
        // Exercise the `augmented.files.push(source_file()?)` path and assert the
        // appended file is the compiler-injected regex source companion.
        let ast = project("IMPORT regex\nSUB main\nEND SUB\n");
        let augmented = augmented_project(&ast).expect("augment");
        let injected = augmented.files.last().expect("injected file");
        assert_eq!(injected.path, "builtins/regex.mfb");
        assert!(injected.internal);
    }
}
