//! Built-in `json` package (plan-72-O), migrated into the codegen layer
//! (planning/migrate.md). Every member is source-backed: its `__json_*` body
//! lives in its `func_*.rs` as `Implementation::Mfb` and is spliced into the
//! injected package source by `assembled_source()` in place of a
//! `'@@MFB_BODY:<slug>@@` marker; the private helpers, the seven `Json*` value
//! types (`EXPORT TYPE`/`EXPORT UNION`), and the internal parse-node types stay in
//! `package.mfb`. json is concrete (rewritten in IR lowering), so the `__json_*`
//! rewrite target comes from the explicit `IMPL_NAMES` table. A retained
//! `JsonResolver` validates the `Json` value-type argument unions the descriptor's
//! per-position match cannot express.

use std::borrow::Cow;

use crate::builtins::exact;
use crate::target::shared::registry::{
    BuiltinFunction, BuiltinModule, BuiltinResolver, BuiltinSource, BuiltinType, DefaultResolver,
    DefaultValue, Implementation, InjectionRule, Parameter, ParameterType, TypeKind,
};

mod func_get;
mod func_get_or;
mod func_parse;
mod func_stringify;

const PARSE: &str = "json.parse";
const STRINGIFY: &str = "json.stringify";
const GET: &str = "json.get";
const GET_OR: &str = "json.getOr";
const INTERNAL_PARSE: &str = "__json_parse";
const INTERNAL_STRINGIFY: &str = "__json_stringify";
const INTERNAL_GET: &str = "__json_get";
const INTERNAL_GET_OR: &str = "__json_getOr";

const fn req(name: &'static str, aliases: &'static [&'static str], ty: &'static str) -> Parameter {
    Parameter {
        name,
        aliases,
        ty: ParameterType::Named(ty),
        default: DefaultValue::None,
    }
}

pub(super) const P_PARSE: &[Parameter] = &[req("value", &["text"], "String")];
pub(super) const P_STRINGIFY: &[Parameter] = &[req("value", &[], "Json")];
pub(super) const P_GET: &[Parameter] = &[
    req("value", &[], "Json"),
    req("path", &["key"], "List OF String"),
];
pub(super) const P_GET_OR: &[Parameter] = &[
    req("value", &[], "Json"),
    req("path", &["key"], "List OF String"),
    req("default", &["defaultValue", "fallback"], "Json"),
];

// plan-72-O: `JSON` is the descriptor authority. Each member owns its source body
// in its `func_*.rs` (`Implementation::Mfb`); a call rewrites to the internal
// `__json_*` name via `IMPL_NAMES` at IR lowering. `WhenImported` source.
const JSON_FUNCTIONS: &[BuiltinFunction] = &[
    func_parse::PARSE,
    func_stringify::STRINGIFY,
    func_get::GET,
    func_get_or::GET_OR,
];

// The seven json value types are registered as opaque builtin types (bare names,
// no record fields), matching the legacy flat `is_builtin_type` list.
const JSON_TYPES: &[BuiltinType] = &[
    BuiltinType {
        name: "Json",
        kind: TypeKind::Opaque,
        fields: &[],
    },
    BuiltinType {
        name: "JsonNull",
        kind: TypeKind::Opaque,
        fields: &[],
    },
    BuiltinType {
        name: "JsonBool",
        kind: TypeKind::Opaque,
        fields: &[],
    },
    BuiltinType {
        name: "JsonNum",
        kind: TypeKind::Opaque,
        fields: &[],
    },
    BuiltinType {
        name: "JsonStr",
        kind: TypeKind::Opaque,
        fields: &[],
    },
    BuiltinType {
        name: "JsonArr",
        kind: TypeKind::Opaque,
        fields: &[],
    },
    BuiltinType {
        name: "JsonObj",
        kind: TypeKind::Opaque,
        fields: &[],
    },
];

/// Return-type resolution for the json calls, delegating to the hand-authored
/// `resolve_call` (which validates the `is_json_value_type` argument unions that
/// the descriptor's per-position match cannot). Exposed through the descriptor so
/// plan-72-BB can drive `json::` return types from the registry.
struct JsonResolver;
impl BuiltinResolver for JsonResolver {
    fn resolve_return_type(
        &self,
        _module: &BuiltinModule,
        name: &str,
        arg_types: &[String],
    ) -> Option<String> {
        resolve_call(name, arg_types).map(|resolved| resolved.return_type.into_owned())
    }
}
static JSON_RESOLVER: JsonResolver = JsonResolver;

const MODULE_INTRO: &str = r#"Parse, build, serialize, and read JSON values as a `Json` tree"#;
const MODULE_DESC: &str = r#"The `json` package converts between JSON text and a `Json` value tree and reads
members out of that tree. `json::parse` turns a UTF-8 `String` holding one
complete JSON document into a `Json` value, `json::stringify` renders a `Json`
value back into compact JSON text, and `json::get` and `json::getOr` walk a path
of object keys to a nested member. `json` is a built-in package written in
MFBASIC source over the `collections`, `strings`, and `encoding` packages, so
`IMPORT json` needs no manifest dependency.

The package defines the `Json` union and its six member types. `Json` is a
`UNION` over `JsonNull`, `JsonBool`, `JsonNum`, `JsonStr`, `JsonArr`, and
`JsonObj`, each a record wrapping one field: `JsonNull` holds `Nothing`,
`JsonBool` holds a `Boolean`, `JsonNum` holds a `Float`, `JsonStr` holds a
`String`, `JsonArr` holds a `List OF Json`, and `JsonObj` holds a
`Map OF String TO Json`. Every JSON form maps to exactly one variant, and
`json::stringify` accepts either the `Json` union or any one of its member types
directly. Because numbers are carried as `Float`, very large or very precise
values may lose precision in a parse/stringify round trip, and a `JsonNum`
holding a non-finite `Float` (NaN or infinity) has no JSON form.

Serialization is compact: `json::stringify` emits no insignificant whitespace,
preserves array item order, emits object pairs in the map's iteration order, and
applies the standard JSON string escapes. Parsing reads one complete document,
allows surrounding JSON whitespace, and rejects any trailing non-whitespace
content.

The path readers operate only on object members. `json::get` and `json::getOr`
follow a `List OF String` of object keys left to right from `value`, requiring a
`JsonObj` at each step; an empty path returns `value` unchanged. They do not copy
`value`: the located `Json` value is returned directly. `json::get` fails when a
key is missing or the current value is not an object, whereas `json::getOr`
returns its default value in those cases instead of failing."#;

pub(crate) static JSON: BuiltinModule = BuiltinModule {
    name: "json",
    doc_intro: MODULE_INTRO,
    doc_desc: MODULE_DESC,
    functions: JSON_FUNCTIONS,
    types: JSON_TYPES,
    source: Some(BuiltinSource {
        rule: InjectionRule::WhenImported,
        loader: source_file,
    }),
    resolver: Some(&JSON_RESOLVER),
};

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

pub(crate) fn is_builtin_type(name: &str) -> bool {
    JSON.types.iter().any(|ty| ty.name == name)
}

pub(crate) fn is_json_call(name: &str) -> bool {
    DefaultResolver::contains(&JSON, name)
}

// `call_param_names` returns a `&'static` borrowed shape the owned
// `DefaultResolver` (which yields `Vec`) cannot produce, and its consumers require
// the borrow, so it stays a static literal PINNED equal to `JSON` by
// `parity_matches_descriptor` until plan-72-BB.
pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    match name {
        PARSE => Some(&[&["value", "text"]]),
        STRINGIFY => Some(&[&["value"]]),
        GET => Some(&[&["value"], &["path", "key"]]),
        GET_OR => Some(&[
            &["value"],
            &["path", "key"],
            &["default", "defaultValue", "fallback"],
        ]),
        _ => None,
    }
}

pub(crate) fn resolve_call<'a>(name: &str, arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    let return_type = match name {
        PARSE if exact(arg_types, &["String"]) => Cow::Borrowed("Json"),
        STRINGIFY if arg_types.len() == 1 && is_json_value_type(&arg_types[0]) => {
            Cow::Borrowed("String")
        }
        GET if arg_types.len() == 2
            && is_json_value_type(&arg_types[0])
            && arg_types[1] == "List OF String" =>
        {
            Cow::Borrowed("Json")
        }
        GET_OR
            if arg_types.len() == 3
                && is_json_value_type(&arg_types[0])
                && arg_types[1] == "List OF String"
                && is_json_value_type(&arg_types[2]) =>
        {
            Cow::Borrowed("Json")
        }
        _ => return None,
    };
    Some(ResolvedCall { return_type })
}

// `expected_arguments` returns a `&'static str` the owned `DefaultResolver`
// (which yields `String`) cannot produce, so it stays a static literal PINNED
// equal to `JSON`'s per-position type rendering by `parity_matches_descriptor`.
pub(crate) fn expected_arguments(name: &str) -> Option<&'static str> {
    match name {
        PARSE => Some("String"),
        STRINGIFY => Some("Json"),
        GET => Some("Json, List OF String"),
        GET_OR => Some("Json, List OF String, Json"),
        _ => None,
    }
}

/// The internal `__json_*` symbol each public member rewrites to during IR
/// lowering. The members carry `Implementation::Mfb` (whose descriptor
/// `implementation_name` is `None`), so the rewrite target is provided here.
const IMPL_NAMES: &[(&str, &str)] = &[
    (PARSE, INTERNAL_PARSE),
    (STRINGIFY, INTERNAL_STRINGIFY),
    (GET, INTERNAL_GET),
    (GET_OR, INTERNAL_GET_OR),
];

pub(crate) fn implementation_name(name: &str) -> Option<&'static str> {
    IMPL_NAMES
        .iter()
        .find(|(public, _)| *public == name)
        .map(|(_, internal)| *internal)
}

/// A member of the json value-type set: the umbrella `Json` or any of its
/// concrete variants. `resolve_call` accepts any of these where the descriptor
/// lists the umbrella `Json` type, so this stays a hand-authored predicate over
/// `JSON.types` (the descriptor's exact per-position match cannot express a
/// type set).
fn is_json_value_type(type_name: &str) -> bool {
    JSON.types.iter().any(|ty| ty.name == type_name)
}

/// Synthetic path label / doc path for the injected json source. Preserved
/// byte-for-byte from the pre-migration `package_source_glue!` invocation so the
/// injected AST is unchanged.
const SOURCE_LABEL: &str = "<builtin-json>";
const SOURCE_DOC: &str = "builtins/json.mfb";

/// Parses the built-in `json` package source (the `package.mfb` companion plus
/// every `Implementation::Mfb` member body, spliced in by `assembled_source`).
pub(crate) fn source_file() -> Result<crate::ast::AstFile, ()> {
    crate::ast::parse_source_internal(
        std::path::Path::new(SOURCE_LABEL),
        SOURCE_DOC,
        &assembled_source(),
    )
}

/// The `json` package source: the `package.mfb` companion (helpers + type decls)
/// with each member's `FUNC __json_* ... END FUNC` body spliced in for its
/// `'@@MFB_BODY:<slug>@@` marker at the body's original position, keeping every
/// other line's number unchanged so the injected AST is byte-identical to the
/// pre-migration companion.
fn assembled_source() -> String {
    let mut source = String::from(include_str!("package.mfb"));
    for func in JSON_FUNCTIONS {
        if let Implementation::Mfb { body, .. } = func.implementation {
            let marker = format!("'@@MFB_BODY:{}@@", func.doc_slug);
            debug_assert!(
                source.contains(&marker),
                "json package.mfb is missing the '{marker}' body marker",
            );
            source = source.replacen(&marker, body, 1);
        }
    }
    source
}

pub(crate) fn uses_package(ast: &crate::ast::AstProject) -> bool {
    ast.files.iter().any(|file| {
        file.imports
            .iter()
            .any(|import| import.package_name() == "json")
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

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn project(src: &str) -> crate::ast::AstProject {
        let file = crate::ast::parse_source(std::path::Path::new("main.mfb"), "main.mfb", src)
            .expect("parse source");
        crate::ast::AstProject {
            name: "test".to_string(),
            files: vec![file],
        }
    }

    fn returns(name: &str, args: &[&str]) -> Option<String> {
        resolve_call(name, &strings(args)).map(|r| r.return_type.into_owned())
    }

    #[test]
    fn is_builtin_type_covers_json_family() {
        for name in [
            "Json", "JsonNull", "JsonBool", "JsonNum", "JsonStr", "JsonArr", "JsonObj",
        ] {
            assert!(is_builtin_type(name));
            assert!(is_json_value_type(name));
        }
        assert!(!is_builtin_type("String"));
        assert!(!is_json_value_type("Integer"));
    }

    #[test]
    fn recognizes_json_calls() {
        assert!(is_json_call(PARSE));
        assert!(is_json_call(STRINGIFY));
        assert!(is_json_call(GET));
        assert!(is_json_call(GET_OR));
        assert!(!is_json_call("json.other"));
    }

    #[test]
    fn param_names_cover_all_calls() {
        assert_eq!(call_param_names(PARSE), Some(&[&["value", "text"][..]][..]));
        assert_eq!(call_param_names(STRINGIFY), Some(&[&["value"][..]][..]));
        assert_eq!(
            call_param_names(GET),
            Some(&[&["value"][..], &["path", "key"][..]][..])
        );
        assert!(call_param_names(GET_OR).is_some());
        assert_eq!(call_param_names("json.other"), None);
    }

    #[test]
    fn resolve_call_accepts_valid_signatures() {
        assert_eq!(returns(PARSE, &["String"]), Some("Json".to_string()));
        assert_eq!(returns(STRINGIFY, &["Json"]), Some("String".to_string()));
        assert_eq!(returns(STRINGIFY, &["JsonObj"]), Some("String".to_string()));
        assert_eq!(
            returns(GET, &["Json", "List OF String"]),
            Some("Json".to_string())
        );
        assert_eq!(
            returns(GET_OR, &["Json", "List OF String", "JsonStr"]),
            Some("Json".to_string())
        );
    }

    #[test]
    fn resolve_call_rejects_bad_signatures() {
        assert!(returns(PARSE, &["Integer"]).is_none());
        assert!(returns(STRINGIFY, &["String"]).is_none());
        assert!(returns(GET, &["Json", "String"]).is_none());
        assert!(returns(GET, &["String", "List OF String"]).is_none());
        assert!(returns(GET_OR, &["Json", "List OF String", "String"]).is_none());
        assert!(returns(GET_OR, &["Json", "Integer", "Json"]).is_none());
        assert!(returns("json.other", &["String"]).is_none());
    }

    #[test]
    fn expected_arguments_and_impl_names() {
        assert_eq!(expected_arguments(PARSE), Some("String"));
        assert_eq!(expected_arguments(STRINGIFY), Some("Json"));
        assert_eq!(expected_arguments(GET), Some("Json, List OF String"));
        assert_eq!(
            expected_arguments(GET_OR),
            Some("Json, List OF String, Json")
        );
        assert_eq!(expected_arguments("json.other"), None);
        assert_eq!(implementation_name(PARSE), Some(INTERNAL_PARSE));
        assert_eq!(implementation_name(STRINGIFY), Some(INTERNAL_STRINGIFY));
        assert_eq!(implementation_name(GET), Some(INTERNAL_GET));
        assert_eq!(implementation_name(GET_OR), Some(INTERNAL_GET_OR));
        assert_eq!(implementation_name("json.other"), None);
    }

    #[test]
    fn source_file_parses() {
        assert!(source_file().is_ok());
    }

    #[test]
    fn augmented_project_injects_when_imported() {
        let ast = project("IMPORT json\nSUB main\nEND SUB\n");
        assert!(uses_package(&ast));
        let augmented = augmented_project(&ast).expect("augment");
        assert_eq!(augmented.files.len(), ast.files.len() + 1);
    }

    #[test]
    fn augmented_project_noop_without_import() {
        let ast = project("SUB main\nEND SUB\n");
        assert!(!uses_package(&ast));
        let augmented = augmented_project(&ast).expect("augment");
        assert_eq!(augmented.files.len(), ast.files.len());
    }
}
