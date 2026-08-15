//! Package: json
//! Type: Pure MFBasic

use crate::codegen::registry::{
    ParameterType, RecordProp, Registry, RegistryPackage, RegistryRecord, RegistryUnion,
    UnionVariant,
};

mod func_get;
mod func_get_or;
mod func_parse;
mod func_stringify;

const INTRO: &str = r#"Parse, build, serialize, and read JSON values as a `Json` tree"#;

const DESC: &str = r#"The `json` package converts between JSON text and a `Json` value tree and reads
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

/// Register the `json` package on the registry.
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("json", INTRO, DESC);

    pkg.add_imports(vec!["collections", "strings", "encoding"]);

    pkg.add_record(RegistryRecord {
        name: "JsonNull",
        export: true,
        props: vec![RecordProp {
            name: "value",
            ty: ParameterType::Nothing,
            description: "The JSON `null` value. The `Json` union variant carrying no data.",
        }],
    });

    pkg.add_record(RegistryRecord {
        name: "JsonBool",
        export: true,
        props: vec![RecordProp {
            name: "value",
            ty: ParameterType::Boolean,
            description: "A JSON boolean (`true` or `false`).",
        }],
    });

    pkg.add_record(RegistryRecord {
        name: "JsonNum",
        export: true,
        props: vec![RecordProp {
            name: "value",
            ty: ParameterType::Float,
            description: "A JSON number, held as a double-precision float.",
        }],
    });

    pkg.add_record(RegistryRecord {
        name: "JsonStr",
        export: true,
        props: vec![RecordProp {
            name: "value",
            ty: ParameterType::String,
            description: "A JSON string.",
        }],
    });

    pkg.add_record(RegistryRecord {
        name: "JsonArr",
        export: true,
        props: vec![RecordProp {
            name: "items",
            ty: ParameterType::list_of(ParameterType::Named("Json")),
            description: "A JSON array's elements.",
        }],
    });

    pkg.add_record(RegistryRecord {
        name: "JsonObj",
        export: true,
        props: vec![RecordProp {
            name: "fields",
            ty: ParameterType::map_of(ParameterType::String, ParameterType::Named("Json")),
            description: "The object's members, keyed by field name.",
        }],
    });

    pkg.add_record(RegistryRecord {
        name: "__json_Node",
        export: false,
        props: vec![
            RecordProp {
                name: "value",
                ty: ParameterType::Named("Json"),
                description: "",
            },
            RecordProp {
                name: "index",
                ty: ParameterType::Integer,
                description: "",
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "__json_StringNode",
        export: false,
        props: vec![
            RecordProp {
                name: "value",
                ty: ParameterType::String,
                description: "",
            },
            RecordProp {
                name: "index",
                ty: ParameterType::Integer,
                description: "",
            },
        ],
    });

    pkg.add_union(RegistryUnion {
        name: "Json",
        export: true,
        variants: vec![
            UnionVariant {
                name: "JsonNull",
                description: "The JSON `null`.",
            },
            UnionVariant {
                name: "JsonBool",
                description: "A JSON boolean.",
            },
            UnionVariant {
                name: "JsonNum",
                description: "The JSON number.",
            },
            UnionVariant {
                name: "JsonStr",
                description: "The JSON string.",
            },
            UnionVariant {
                name: "JsonArr",
                description: "The JSON array.",
            },
            UnionVariant {
                name: "JsonObj",
                description: "The JSON object.",
            },
        ],
    });

    // The shared private `__json_*` helpers the member bodies call.
    pkg.add_helper_functions(vec![include_str!("package.mfb")]);

    func_get::register(&mut pkg);
    func_get_or::register(&mut pkg);
    func_parse::register(&mut pkg);
    func_stringify::register(&mut pkg);

    r.add_package(pkg);
}

/*
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
*/
