//! Package: json
//! Type: Pure MFBasic

use crate::codegen::registry::{
    RecordProp, Registry, RegistryPackage, RegistryRecord, RegistryUnion, UnionVariant,
};
use crate::types::ParameterType;

mod func_get;
mod func_get_or;
mod func_parse;
mod func_sci_parts;
mod func_stringify;

mod helper_array_index;
mod helper_code_point_to_string;
mod helper_consume_digits;
mod helper_control_escape;
mod helper_decode_escape;
mod helper_depth_limit;
mod helper_escape_raw_control_char;
mod helper_escape_string;
mod helper_expect_literal;
mod helper_expect_literal_at;
mod helper_hex_digit;
mod helper_indent_text;
mod helper_is_high_surrogate;
mod helper_is_low_surrogate;
mod helper_is_non_zero_digit;
mod helper_is_raw_control_char;
mod helper_is_whitespace;
mod helper_next_digit;
mod helper_number_end;
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
mod helper_place_digits;
mod helper_require_finite_number_text;
mod helper_revive;
mod helper_round_digits;
mod helper_round_trips;
mod helper_skip_whitespace;
mod helper_stringify_indent;
mod helper_stringify_number;
mod helper_to_number;
mod helper_trim_float_text;
mod helper_trim_float_text_at;
mod helper_unicode_control_escape;
mod helper_valid_number;

/// The `Json` union's package-qualified identity — what a consumer must write,
/// and what the resolver seeds, so a bare `AS Json` is refused (bug-484). Inside
/// `json`'s own companion the leaf is local and stays bare.
pub(crate) const JSON_TYPE_ID: &str = "json.Json";

const INTRO: &str = r#"Parse, build, serialize, and read JSON values as a `json::Json` tree"#;

const DESC: &str = r#"The `json` package converts between JSON text and a `json::Json` value tree and reads
members out of that tree. `json::parse` turns a UTF-8 `String` holding one
complete JSON document into a `json::Json` value, `json::stringify` renders a `json::Json`
value back into compact JSON text, and `json::get` and `json::getOr` walk a path
of object keys to a nested member. `json` is a built-in package written in
MFBASIC source over the `collections`, `strings`, and `encoding` packages, so
`IMPORT json` needs no manifest dependency.

The package defines the `json::Json` union and its six member types. `json::Json` is a
`UNION` over `json::JsonNull`, `json::JsonBool`, `json::JsonNum`, `json::JsonStr`, `json::JsonArr`, and
`json::JsonObj`, each a record wrapping one field: `json::JsonNull` holds `Nothing`,
`json::JsonBool` holds a `Boolean`, `json::JsonNum` holds a `Float`, `json::JsonStr` holds a
`String`, `json::JsonArr` holds a `List OF json::Json`, and `json::JsonObj` holds a
`Map OF String TO json::Json`. Every JSON form maps to exactly one variant, and
`json::stringify` accepts either the `json::Json` union or any one of its member types
directly. Because numbers are carried as `Float`, a number carrying more digits
than binary64 holds is rounded to the nearest `Float` at parse time — a real
precision loss, silent by design, exactly as it is in JavaScript. Magnitude is
not silent: a number too large for `Float` (`1e400`) fails at parse with
`errorCode::ErrOverflow`. Nothing fails on the way out: every finite `Float` has
a JSON rendering, including values as small as `5e-324`.
A `json::JsonNum` holding a non-finite `Float` (NaN or infinity) has no JSON form
at all; it cannot be built in the first place, because a non-finite `Float` fails
at the observation boundary before it reaches a record field.

Numbers are rendered exactly as JavaScript's `JSON.stringify` renders them —
the shortest digits that read back as the same `Float`, placed plainly while
`1e-6 <= |value| < 1e21` and exponentially outside — so a document written here
and read there, or the reverse, carries the same numbers with the same text.

Serialization is compact: `json::stringify` emits no insignificant whitespace,
preserves array item order, emits object members in the order the object holds
them — document order, for anything `json::parse` produced — and applies the
standard JSON string escapes, leaving `/` unescaped. Parsing reads one complete
document, allows surrounding JSON whitespace, and rejects any trailing
non-whitespace content.

The path readers walk a whole tree. `json::get` and `json::getOr` follow a
`List OF String` left to right from `value`; a step is an object key on a
`json::JsonObj` and a zero-based decimal index on a `json::JsonArr`, so
`["items", "1", "name"]` crosses both. An empty path returns `value` unchanged.
They do not copy `value`: the located `json::Json` value is returned directly.
`json::get` fails when a step finds nothing, whereas `json::getOr` returns its
default value in those cases instead of failing."#;

/// Register the `json` package on the registry.
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("json", INTRO, DESC);

    pkg.add_imports(vec!["collections", "strings", "encoding"]);

    pkg.add_record(RegistryRecord {
        name: "JsonNull",
        export: true,
        description: "",
        props: vec![RecordProp {
            name: "value",
            ty: ParameterType::Nothing,
            description: "The JSON `null` value. The `json::Json` union variant carrying no data.",
        }],
    });

    pkg.add_record(RegistryRecord {
        name: "JsonBool",
        export: true,
        description: "",
        props: vec![RecordProp {
            name: "value",
            ty: ParameterType::Boolean,
            description: "A JSON boolean (`true` or `false`).",
        }],
    });

    pkg.add_record(RegistryRecord {
        name: "JsonNum",
        export: true,
        description: "",
        props: vec![RecordProp {
            name: "value",
            ty: ParameterType::Float,
            description: "A JSON number, held as a double-precision float.",
        }],
    });

    pkg.add_record(RegistryRecord {
        name: "JsonStr",
        export: true,
        description: "",
        props: vec![RecordProp {
            name: "value",
            ty: ParameterType::String,
            description: "A JSON string.",
        }],
    });

    pkg.add_record(RegistryRecord {
        name: "JsonArr",
        export: true,
        description: "",
        props: vec![RecordProp {
            name: "items",
            ty: ParameterType::list_of(ParameterType::named("Json")),
            description: "A JSON array's elements.",
        }],
    });

    pkg.add_record(RegistryRecord {
        name: "JsonObj",
        export: true,
        description: "",
        props: vec![RecordProp {
            name: "fields",
            ty: ParameterType::map_of(ParameterType::String, ParameterType::named("Json")),
            description: "The object's members, keyed by field name.",
        }],
    });

    pkg.add_record(RegistryRecord {
        name: "__json_Node",
        export: false,
        description: "",
        props: vec![
            RecordProp {
                name: "value",
                ty: ParameterType::named("Json"),
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
        description: "",
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
    helper_next_digit::register(&mut pkg);
    helper_round_digits::register(&mut pkg);
    helper_round_trips::register(&mut pkg);
    helper_place_digits::register(&mut pkg);
    helper_stringify_number::register(&mut pkg);
    helper_require_finite_number_text::register(&mut pkg);
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
    helper_number_end::register(&mut pkg);
    helper_to_number::register(&mut pkg);
    helper_valid_number::register(&mut pkg);
    helper_consume_digits::register(&mut pkg);
    helper_expect_literal::register(&mut pkg);
    helper_expect_literal_at::register(&mut pkg);
    helper_skip_whitespace::register(&mut pkg);
    helper_is_whitespace::register(&mut pkg);
    helper_is_non_zero_digit::register(&mut pkg);
    // plan-120-B. Appended rather than slotted next to the other predicates so
    // the helpers above keep the order the old `package.mfb` blob had.
    helper_array_index::register(&mut pkg);
    // plan-120-D: the indented renderer and its two clamp helpers, appended for
    // the same reason as `helper_array_index` above.
    helper_stringify_indent::register(&mut pkg);
    helper_indent_text::register(&mut pkg);
    // plan-120-E: the reviver walk, appended for the same reason.
    func_sci_parts::register(&mut pkg);
    helper_revive::register(&mut pkg);

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
        // plan-120-G: assert the PUBLIC surface rather than a raw count. The
        // count used to be 4 and became 5 when `sciParts` was added as an
        // internal helper; bumping the number would have made the test pass
        // again while no longer saying anything — it would equally have passed
        // if `sciParts` were public and `getOr` had been dropped.
        let public: Vec<&str> = pkg
            .functions()
            .iter()
            .filter(|function| !function.internal_only)
            .map(|function| function.name)
            .collect();
        assert_eq!(public, vec!["get", "getOr", "parse", "stringify"]);
        assert!(
            registry().is_internal_only_member("json.sciParts"),
            "sciParts is an implementation detail and must not be callable"
        );
        // The Json union and its member records are visible to the generic type query.
        assert!(registry().is_builtin_type("json.Json"));
        assert!(registry().is_builtin_type("json.JsonObj"));
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
        assert_eq!(
            registry::call_return_type_typed("json.parse")
                .map(|t| t.name().into_owned())
                .as_deref(),
            Some("json.Json")
        );
        assert_eq!(
            registry::call_return_type_typed("json.stringify")
                .map(|t| t.name().into_owned())
                .as_deref(),
            Some("String")
        );
        // plan-120-E: parse gained the `(text, reviver)` overload, so it now
        // accepts 1 OR 2 arguments. The 1-arg form is unchanged.
        assert_eq!(registry().arity("json.parse"), Some((1, 2)));
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
