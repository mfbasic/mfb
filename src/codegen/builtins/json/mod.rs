//! Package: json
//! Type: Pure MFBasic

use crate::codegen::registry::{
    RecordProp, Registry, RegistryPackage, RegistryRecord, RegistryUnion, UnionVariant,
};
use crate::types::ParameterType;

mod func_get;
mod func_get_or;
mod func_parse;
mod func_stringify;

mod helper_code_point_to_string;
mod helper_collect_number;
mod helper_consume_digits;
mod helper_control_escape;
mod helper_decode_escape;
mod helper_depth_limit;
mod helper_escape_raw_control_char;
mod helper_escape_string;
mod helper_expect_literal;
mod helper_expect_literal_at;
mod helper_hex_digit;
mod helper_is_digit;
mod helper_is_high_surrogate;
mod helper_is_invalid_number_text;
mod helper_is_low_surrogate;
mod helper_is_non_zero_digit;
mod helper_is_raw_control_char;
mod helper_is_whitespace;
mod helper_parse_array;
mod helper_parse_array_items;
mod helper_parse_escape;
mod helper_parse_hex_quad;
mod helper_parse_number;
mod helper_parse_object;
mod helper_parse_object_items;
mod helper_parse_string;
mod helper_parse_unicode_escape;
mod helper_parse_value;
mod helper_skip_whitespace;
mod helper_stringify_number;
mod helper_to_number;
mod helper_trim_float_text;
mod helper_trim_float_text_at;
mod helper_unicode_control_escape;
mod helper_valid_number;

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

    // The shared private `__json_*` helpers the member bodies call. Each lives in
    // its own `helper_*.rs` and registers via `add_helper`; order preserved from
    // the old `package.mfb` blob so the compiled `.ncode` stays byte-identical.
    helper_stringify_number::register(&mut pkg);
    helper_is_invalid_number_text::register(&mut pkg);
    helper_trim_float_text::register(&mut pkg);
    helper_trim_float_text_at::register(&mut pkg);
    helper_escape_string::register(&mut pkg);
    helper_escape_raw_control_char::register(&mut pkg);
    helper_control_escape::register(&mut pkg);
    helper_unicode_control_escape::register(&mut pkg);
    helper_hex_digit::register(&mut pkg);
    helper_depth_limit::register(&mut pkg);
    helper_parse_value::register(&mut pkg);
    helper_parse_array::register(&mut pkg);
    helper_parse_array_items::register(&mut pkg);
    helper_parse_object::register(&mut pkg);
    helper_parse_object_items::register(&mut pkg);
    helper_parse_string::register(&mut pkg);
    helper_is_raw_control_char::register(&mut pkg);
    helper_parse_escape::register(&mut pkg);
    helper_decode_escape::register(&mut pkg);
    helper_parse_unicode_escape::register(&mut pkg);
    helper_parse_hex_quad::register(&mut pkg);
    helper_is_high_surrogate::register(&mut pkg);
    helper_is_low_surrogate::register(&mut pkg);
    helper_code_point_to_string::register(&mut pkg);
    helper_parse_number::register(&mut pkg);
    helper_collect_number::register(&mut pkg);
    helper_to_number::register(&mut pkg);
    helper_valid_number::register(&mut pkg);
    helper_consume_digits::register(&mut pkg);
    helper_expect_literal::register(&mut pkg);
    helper_expect_literal_at::register(&mut pkg);
    helper_skip_whitespace::register(&mut pkg);
    helper_is_whitespace::register(&mut pkg);
    helper_is_digit::register(&mut pkg);
    helper_is_non_zero_digit::register(&mut pkg);

    func_get::register(&mut pkg);
    func_get_or::register(&mut pkg);
    func_parse::register(&mut pkg);
    func_stringify::register(&mut pkg);

    r.add_package(pkg);
}

#[cfg(test)]
mod tests {
    use crate::codegen::registry::{self, registry};

    #[test]
    fn json_registered_on_the_clean_room_registry() {
        let pkg = registry().resolve_package("json").expect("json package");
        assert_eq!(pkg.functions().len(), 4);
        // The Json union and its member records are visible to the generic type query.
        assert!(registry().is_builtin_type("Json"));
        assert!(registry().is_builtin_type("JsonObj"));
        assert!(!registry().is_builtin_type("Nope"));
    }

    #[test]
    fn generic_dispatch_reaches_json() {
        assert!(registry().is_member("json.parse"));
        assert!(!registry().is_member("json.nope"));
        assert_eq!(
            registry::rewrite_target("json.parse", &[]),
            Some("__json_parse")
        );
        assert_eq!(
            registry::rewrite_target("json.getOr", &[]),
            Some("__json_getOr")
        );
        assert_eq!(registry::call_return_type("json.parse"), Some("Json"));
        assert_eq!(registry::call_return_type("json.stringify"), Some("String"));
        assert_eq!(registry().arity("json.parse"), Some((1, 1)));
        assert_eq!(registry().arity("json.get"), Some((2, 2)));
        assert_eq!(registry().arity("json.getOr"), Some((3, 3)));
    }

    #[test]
    fn reassembled_source_parses() {
        let source = registry().resolve_package("json").expect("json").get_mfb();
        crate::ast::parse_source_internal(
            std::path::Path::new("<builtin-json>"),
            "builtins/json.mfb",
            &source,
        )
        .expect("reassembled json source parses");
    }
}
