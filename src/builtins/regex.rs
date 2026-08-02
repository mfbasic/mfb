use std::path::Path;

use super::descriptor::{
    BuiltinFlags, BuiltinFunction, BuiltinModule, BuiltinOverload, BuiltinSource, DefaultResolver,
    DefaultValue, Implementation, InjectionRule, Lowering, Parameter, ParameterType, ReturnType,
};

const MATCH: &str = "regex.match";
const FIND: &str = "regex.find";
const FIND_ALL: &str = "regex.findAll";
const REPLACE: &str = "regex.replace";
const INTERNAL_MATCH: &str = "__regex_match";
const INTERNAL_FIND: &str = "__regex_find";
const INTERNAL_FIND_ALL: &str = "__regex_findAll";
const INTERNAL_REPLACE: &str = "__regex_replace";

// plan-72-T: `REGEX` is the descriptor authority. Despite the census `custom 2`,
// `regex` needs NO resolver — its two "custom" helpers are both data-shaped:
// `implementation_name` is a fixed per-name `Implementation::Rewrite(__regex_*)`
// (argument-independent), and `default_argument_padding` is a plain trailing
// `DefaultValue::Fill("Integer","0")` on `find`/`findAll`'s `start` (verified
// equal to the legacy padding for every provided count by the parity test). Each
// function has one fixed-return overload, so `resolve_call`, `call_return_type_
// name`, `arity`, and `default_padding` all derive from `DefaultResolver`. Only
// `expected_arguments` stays hand-authored: `find`/`findAll` render the optional
// `start` as `String, String[, Integer]`, a bracket phrasing the descriptor's
// per-position type list cannot produce (the `collections` precedent). The source
// companion is bespoke (engine + generated Unicode table combined into one file)
// but its injection rule is the standard `WhenImported`.
const PARAMS_MATCH: &[Parameter] = &[
    Parameter::required("value", "String"),
    Parameter::required("pattern", "String"),
];
// find/findAll(value, pattern, [start=0]) — trailing `start` default-pads to 0.
const PARAMS_FIND: &[Parameter] = &[
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
const PARAMS_REPLACE: &[Parameter] = &[
    Parameter::required("value", "String"),
    Parameter::required("pattern", "String"),
    Parameter::required("replacement", "String"),
];

const fn regex_fn(
    name: &'static str,
    slug: &'static str,
    overloads: &'static [BuiltinOverload],
    implementation: &'static str,
) -> BuiltinFunction {
    BuiltinFunction {
        name,
        doc_slug: slug,
        overloads,
        implementation: Implementation::Rewrite(implementation),
        lowering: Lowering::Helper,
        flags: BuiltinFlags {
            internal_only: false,
            return_type_overloaded: false,
        },
    }
}

const OV_MATCH: &[BuiltinOverload] = &[BuiltinOverload {
    params: PARAMS_MATCH,
    return_type: ReturnType::Fixed("Boolean"),
}];
const OV_FIND: &[BuiltinOverload] = &[BuiltinOverload {
    params: PARAMS_FIND,
    return_type: ReturnType::Fixed("Integer"),
}];
const OV_FIND_ALL: &[BuiltinOverload] = &[BuiltinOverload {
    params: PARAMS_FIND,
    return_type: ReturnType::Fixed("List OF Integer"),
}];
const OV_REPLACE: &[BuiltinOverload] = &[BuiltinOverload {
    params: PARAMS_REPLACE,
    return_type: ReturnType::Fixed("String"),
}];

const REGEX_FUNCTIONS: &[BuiltinFunction] = &[
    regex_fn(MATCH, "match", OV_MATCH, INTERNAL_MATCH),
    regex_fn(FIND, "find", OV_FIND, INTERNAL_FIND),
    regex_fn(FIND_ALL, "findAll", OV_FIND_ALL, INTERNAL_FIND_ALL),
    regex_fn(REPLACE, "replace", OV_REPLACE, INTERNAL_REPLACE),
];

pub(crate) static REGEX: BuiltinModule = BuiltinModule {
    name: "regex",
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

pub(crate) fn implementation_name(name: &str) -> Option<&'static str> {
    DefaultResolver::implementation_name(&REGEX, name)
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
    // The engine and the generated Unicode general-category table
    // (`unicode_gencat.mfb`, see `scripts/gen_regex_unicode.py`) are kept as
    // separate physical files so the table can be regenerated mechanically, but
    // they compile as one source file: MFBASIC `FUNC`s are file-local unless
    // exported, and `PACKAGE` visibility is not valid in an executable, so the
    // engine's calls to `__regex_genCat` must be intra-file.
    let combined = format!(
        "{}\n{}",
        include_str!("regex_package.mfb"),
        include_str!("unicode_gencat.mfb"),
    );
    crate::ast::parse_source_internal(
        Path::new("<builtin-regex>"),
        "builtins/regex.mfb",
        &combined,
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
    fn descriptor_constructors_execute_at_runtime() {
        // `regex_fn` is a const fn invoked only in const context
        // (`REGEX_FUNCTIONS`), so its body never runs at runtime and shows as
        // uncovered. Call it at runtime to exercise (and pin the shape of) the
        // constructor. The E0716 gotcha: a `&[ov(...)]` temporary cannot be
        // passed as `&'static [BuiltinOverload]`, so use a named const slice.
        const OV: &[BuiltinOverload] = &[BuiltinOverload {
            params: PARAMS_MATCH,
            return_type: ReturnType::Fixed("Boolean"),
        }];
        let func = regex_fn(MATCH, "match", OV, INTERNAL_MATCH);
        assert_eq!(func.name, MATCH);
        assert_eq!(func.doc_slug, "match");
        assert_eq!(func.overloads.len(), 1);
        assert_eq!(
            func.implementation,
            Implementation::Rewrite(INTERNAL_MATCH)
        );
        assert_eq!(func.lowering, Lowering::Helper);
        assert!(!func.flags.internal_only);
        assert!(!func.flags.return_type_overloaded);
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
