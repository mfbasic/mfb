use std::borrow::Cow;

use crate::codegen::registry::{
    BuiltinFlags, BuiltinFunction, BuiltinModule, BuiltinOverload, BuiltinResolver, BuiltinSource,
    BuiltinType, DefaultResolver, DefaultValue, Implementation, InjectionRule, Lowering, Parameter,
    ParameterType, ReturnType, TypeKind,
};

const PARSE: &str = "json.parse";
const STRINGIFY: &str = "json.stringify";
const GET: &str = "json.get";
const GET_OR: &str = "json.getOr";
const INTERNAL_PARSE: &str = "__json_parse";
const INTERNAL_STRINGIFY: &str = "__json_stringify";
const INTERNAL_GET: &str = "__json_get";
const INTERNAL_GET_OR: &str = "__json_getOr";

// plan-72-O: `JSON` is the descriptor authority for this package. Every function
// has a single fixed-return overload and a fixed per-name implementation rewrite
// (`__json_*`), so `is_json_call`/`arity`/`call_return_type_name`/
// `implementation_name` derive from the descriptor with no resolver. The seven
// `Json*` value types are opaque builtin types. The `package_source_glue!`
// companion is `WhenImported`. `resolve_call` stays hand-authored: it accepts any
// member of the json value-type set where the descriptor lists the umbrella `Json`
// type (e.g. `stringify(JsonObj)`), which the descriptor's exact per-position type
// match cannot reproduce — a bespoke facet like io's "no arguments" phrasing.
const fn ov(params: &'static [Parameter], ret: &'static str) -> BuiltinOverload {
    BuiltinOverload {
        params,
        return_type: ReturnType::Fixed(ret),
    }
}

const fn json_fn(
    name: &'static str,
    slug: &'static str,
    overloads: &'static [BuiltinOverload],
    implementation: Implementation,
) -> BuiltinFunction {
    BuiltinFunction {
        name,
        doc_slug: slug,
        doc_intro: "",
        doc_desc: "",
        errors: &[],
        overloads,
        doc_example: "",
        implementation,
        lowering: Lowering::Helper,
        flags: BuiltinFlags {
            internal_only: false,
            return_type_overloaded: false,
        },
    }
}

const fn req(name: &'static str, aliases: &'static [&'static str], ty: &'static str) -> Parameter {
    Parameter {
        name,
        aliases,
        ty: ParameterType::Named(ty),
        default: DefaultValue::None,
    }
}

const P_PARSE: &[Parameter] = &[req("value", &["text"], "String")];
const P_STRINGIFY: &[Parameter] = &[req("value", &[], "Json")];
const P_GET: &[Parameter] = &[
    req("value", &[], "Json"),
    req("path", &["key"], "List OF String"),
];
const P_GET_OR: &[Parameter] = &[
    req("value", &[], "Json"),
    req("path", &["key"], "List OF String"),
    req("default", &["defaultValue", "fallback"], "Json"),
];

const JSON_FUNCTIONS: &[BuiltinFunction] = &[
    json_fn(
        PARSE,
        "parse",
        &[ov(P_PARSE, "Json")],
        Implementation::Rewrite(INTERNAL_PARSE),
    ),
    json_fn(
        STRINGIFY,
        "stringify",
        &[ov(P_STRINGIFY, "String")],
        Implementation::Rewrite(INTERNAL_STRINGIFY),
    ),
    json_fn(
        GET,
        "get",
        &[ov(P_GET, "Json")],
        Implementation::Rewrite(INTERNAL_GET),
    ),
    json_fn(
        GET_OR,
        "getOr",
        &[ov(P_GET_OR, "Json")],
        Implementation::Rewrite(INTERNAL_GET_OR),
    ),
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

pub(crate) static JSON: BuiltinModule = BuiltinModule {
    name: "json",
    doc_intro: "",
    doc_desc: "",
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

pub(crate) fn implementation_name(name: &str) -> Option<&'static str> {
    DefaultResolver::implementation_name(&JSON, name)
}

super::package_source_glue!(
    "json",
    "<builtin-json>",
    "builtins/json.mfb",
    include_str!("json_package.mfb")
);

use super::exact;

/// A member of the json value-type set: the umbrella `Json` or any of its
/// concrete variants. `resolve_call` accepts any of these where the descriptor
/// lists the umbrella `Json` type, so this stays a hand-authored predicate over
/// `JSON.types` (the descriptor's exact per-position match cannot express a
/// type set).
fn is_json_value_type(type_name: &str) -> bool {
    JSON.types.iter().any(|ty| ty.name == type_name)
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

    #[test]
    fn descriptor_constructors_execute_at_runtime() {
        // `ov`/`json_fn`/`req` are const fns used only in const context; call them
        // at runtime so their bodies are exercised.
        let param = req("value", &["text"], "String");
        assert_eq!(param.name, "value");
        assert_eq!(param.aliases, &["text"]);
        assert_eq!(param.ty, ParameterType::Named("String"));
        assert_eq!(param.default, DefaultValue::None);

        let overload = ov(P_PARSE, "Json");
        assert_eq!(overload.params.len(), 1);
        assert_eq!(overload.return_type, ReturnType::Fixed("Json"));

        const OV_PARSE: &[BuiltinOverload] = &[ov(P_PARSE, "Json")];
        let func = json_fn(
            PARSE,
            "parse",
            OV_PARSE,
            Implementation::Rewrite(INTERNAL_PARSE),
        );
        assert_eq!(func.name, PARSE);
        assert_eq!(func.doc_slug, "parse");
        assert_eq!(func.implementation, Implementation::Rewrite(INTERNAL_PARSE));
        assert_eq!(func.lowering, Lowering::Helper);
        assert!(!func.flags.internal_only);
    }
}
