use super::descriptor::{
    BuiltinFlags, BuiltinFunction, BuiltinModule, BuiltinOverload, BuiltinSource, DefaultResolver,
    Implementation, InjectionRule, Lowering, Parameter, ReturnType,
};

// plan-89: the `astrings` package — construction, mutation, query, and rendering
// for the opaque, value-semantic `AttributedString` type (registered as a
// hardcoded Family-B type in the resolver/syntaxcheck/binary_repr layers, not as
// a descriptor `types` entry — it is always in scope, like `Error`).
//
// The public members split three ways by lowering:
//   - `fromString` is native-direct codegen (`builder_astrings.rs`).
//   - the `Attribute` model constructors (`bold`..`fontSize`) and the Tier-C
//     mutation/query members (`addAttribute`..`getAttributes`) are `.mfb` bodies
//     in the source companion (`astrings_package.mfb`), reached via
//     `Implementation::Rewrite("__astrings_*")`.
//   - `readSpans`/`writeSpans` are internal-only native primitives that bridge
//     the opaque attribute overlay for the companion (`.mfb` cannot touch the
//     opaque record's fields); users can never call them (internal_only + the
//     companion file is `internal`).

const FROM_STRING: &str = "astrings.fromString";
const BOLD: &str = "astrings.bold";
const ITALIC: &str = "astrings.italic";
const UNDERLINE: &str = "astrings.underline";
const STRIKE: &str = "astrings.strike";
const OVERLINE: &str = "astrings.overline";
const FONT: &str = "astrings.font";
const FONT_SIZE: &str = "astrings.fontSize";
const ADD_ATTRIBUTE: &str = "astrings.addAttribute";
const REMOVE_ATTRIBUTE: &str = "astrings.removeAttribute";
const CLEAR_ATTRIBUTES: &str = "astrings.clearAttributes";
const GET_ATTRIBUTES: &str = "astrings.getAttributes";
const TO_MARKDOWN: &str = "astrings.toMarkdown";
// Internal-only native overlay bridge (never user-callable).
const READ_SPANS: &str = "astrings.readSpans";
const WRITE_SPANS: &str = "astrings.writeSpans";
const SCALAR_LEN: &str = "astrings.scalarLen";

// The stored-span record type the overlay list holds — a codegen-internal record
// (see `validation.rs`), also declared in the companion so the `.mfb` bridge code
// can read/build it. Its spelling appears in the two bridge primitives' signatures.
const SPAN_LIST: &str = "List OF AttrSpan";

const P_FROM_STRING: &[Parameter] = &[Parameter::required("text", "String")];
const P_FONT: &[Parameter] = &[Parameter::required("name", "String")];
const P_FONT_SIZE: &[Parameter] = &[Parameter::required("size", "Integer")];
// The end-of-range parameter is `endIndex`, not `end`: `end` is a reserved
// keyword and cannot be an identifier (so it could be neither the `.mfb` body's
// parameter nor a usable named-argument spelling).
const P_ADD: &[Parameter] = &[
    Parameter::required("value", "AttributedString"),
    Parameter::required("start", "Integer"),
    Parameter::required("endIndex", "Integer"),
    Parameter::required("attr", "Attribute"),
];
const P_REMOVE: &[Parameter] = &[
    Parameter::required("value", "AttributedString"),
    Parameter::required("start", "Integer"),
    Parameter::required("endIndex", "Integer"),
    Parameter::required("attr", "Attribute"),
];
// `clearAttributes` overloads on arity: whole (1 arg) or ranged (3 args).
const P_CLEAR_ALL: &[Parameter] = &[Parameter::required("value", "AttributedString")];
const P_CLEAR_RANGE: &[Parameter] = &[
    Parameter::required("value", "AttributedString"),
    Parameter::required("start", "Integer"),
    Parameter::required("endIndex", "Integer"),
];
const P_GET: &[Parameter] = &[
    Parameter::required("value", "AttributedString"),
    Parameter::required("index", "Integer"),
];
const P_TO_MARKDOWN: &[Parameter] = &[Parameter::required("value", "AttributedString")];
const P_READ_SPANS: &[Parameter] = &[Parameter::required("value", "AttributedString")];
const P_WRITE_SPANS: &[Parameter] = &[
    Parameter::required("value", "AttributedString"),
    Parameter::required("spans", SPAN_LIST),
];
const P_SCALAR_LEN: &[Parameter] = &[Parameter::required("value", "AttributedString")];

const fn ov(params: &'static [Parameter], ret: &'static str) -> BuiltinOverload {
    BuiltinOverload {
        params,
        return_type: ReturnType::Fixed(ret),
    }
}

const OV_FROM_STRING: &[BuiltinOverload] = &[ov(P_FROM_STRING, "AttributedString")];
const OV_FLAG: &[BuiltinOverload] = &[ov(&[], "Attribute")];
const OV_FONT: &[BuiltinOverload] = &[ov(P_FONT, "Attribute")];
const OV_FONT_SIZE: &[BuiltinOverload] = &[ov(P_FONT_SIZE, "Attribute")];
const OV_ADD: &[BuiltinOverload] = &[ov(P_ADD, "AttributedString")];
const OV_REMOVE: &[BuiltinOverload] = &[ov(P_REMOVE, "AttributedString")];
const OV_CLEAR: &[BuiltinOverload] = &[
    ov(P_CLEAR_ALL, "AttributedString"),
    ov(P_CLEAR_RANGE, "AttributedString"),
];
const OV_GET: &[BuiltinOverload] = &[ov(P_GET, "List OF Attribute")];
const OV_TO_MARKDOWN: &[BuiltinOverload] = &[ov(P_TO_MARKDOWN, "String")];
const OV_READ_SPANS: &[BuiltinOverload] = &[ov(P_READ_SPANS, SPAN_LIST)];
const OV_WRITE_SPANS: &[BuiltinOverload] = &[ov(P_WRITE_SPANS, "AttributedString")];
const OV_SCALAR_LEN: &[BuiltinOverload] = &[ov(P_SCALAR_LEN, "Integer")];

const fn astrings_fn(
    name: &'static str,
    slug: &'static str,
    overloads: &'static [BuiltinOverload],
    implementation: Implementation,
) -> BuiltinFunction {
    BuiltinFunction {
        name,
        doc_slug: slug,
        overloads,
        implementation,
        lowering: Lowering::Helper,
        flags: BuiltinFlags {
            internal_only: false,
            return_type_overloaded: false,
        },
    }
}

const fn astrings_internal_fn(
    name: &'static str,
    slug: &'static str,
    overloads: &'static [BuiltinOverload],
) -> BuiltinFunction {
    BuiltinFunction {
        name,
        doc_slug: slug,
        overloads,
        implementation: Implementation::Same,
        lowering: Lowering::Helper,
        flags: BuiltinFlags {
            internal_only: true,
            return_type_overloaded: false,
        },
    }
}

const ASTRINGS_FUNCTIONS: &[BuiltinFunction] = &[
    // Native-direct constructor.
    astrings_fn(FROM_STRING, "fromString", OV_FROM_STRING, Implementation::Same),
    // Source-companion Attribute-model constructors.
    astrings_fn(BOLD, "bold", OV_FLAG, Implementation::Rewrite("__astrings_bold")),
    astrings_fn(
        ITALIC,
        "italic",
        OV_FLAG,
        Implementation::Rewrite("__astrings_italic"),
    ),
    astrings_fn(
        UNDERLINE,
        "underline",
        OV_FLAG,
        Implementation::Rewrite("__astrings_underline"),
    ),
    astrings_fn(
        STRIKE,
        "strike",
        OV_FLAG,
        Implementation::Rewrite("__astrings_strike"),
    ),
    astrings_fn(
        OVERLINE,
        "overline",
        OV_FLAG,
        Implementation::Rewrite("__astrings_overline"),
    ),
    astrings_fn(FONT, "font", OV_FONT, Implementation::Rewrite("__astrings_font")),
    astrings_fn(
        FONT_SIZE,
        "fontSize",
        OV_FONT_SIZE,
        Implementation::Rewrite("__astrings_fontSize"),
    ),
    // Source-companion Tier-C mutation/query members.
    astrings_fn(
        ADD_ATTRIBUTE,
        "addAttribute",
        OV_ADD,
        Implementation::Rewrite("__astrings_addAttribute"),
    ),
    astrings_fn(
        REMOVE_ATTRIBUTE,
        "removeAttribute",
        OV_REMOVE,
        Implementation::Rewrite("__astrings_removeAttribute"),
    ),
    astrings_fn(
        CLEAR_ATTRIBUTES,
        "clearAttributes",
        OV_CLEAR,
        Implementation::Rewrite("__astrings_clearAttributes"),
    ),
    astrings_fn(
        GET_ATTRIBUTES,
        "getAttributes",
        OV_GET,
        Implementation::Rewrite("__astrings_getAttributes"),
    ),
    astrings_fn(
        TO_MARKDOWN,
        "toMarkdown",
        OV_TO_MARKDOWN,
        Implementation::Rewrite("__astrings_toMarkdown"),
    ),
    // Internal-only native overlay bridge.
    astrings_internal_fn(READ_SPANS, "readSpans", OV_READ_SPANS),
    astrings_internal_fn(WRITE_SPANS, "writeSpans", OV_WRITE_SPANS),
    astrings_internal_fn(SCALAR_LEN, "scalarLen", OV_SCALAR_LEN),
];

pub(crate) static ASTRINGS: BuiltinModule = BuiltinModule {
    name: "astrings",
    // `AttributedString` is a hardcoded always-in-scope type, not a descriptor
    // type contributed by this package (see the module comment).
    functions: ASTRINGS_FUNCTIONS,
    types: &[],
    source: Some(BuiltinSource {
        rule: InjectionRule::WhenImported,
        loader: source_file,
    }),
    resolver: None,
};

pub(crate) fn is_astrings_call(name: &str) -> bool {
    DefaultResolver::contains(&ASTRINGS, name)
}

/// The internal-only native overlay-bridge members — never user-callable
/// (`resolution.rs` rejects them outside an `internal` file).
pub(crate) fn is_astrings_internal_call(name: &str) -> bool {
    matches!(name, READ_SPANS | WRITE_SPANS | SCALAR_LEN)
}

/// The source-companion implementation symbol for an `astrings` member, or `None`
/// for the native-direct members (`fromString`, the internal bridge) which keep
/// their native lowering. `clearAttributes` overloads on arity: the whole form
/// (1 arg) and the ranged form (3 args) select distinct `.mfb` bodies.
pub(crate) fn implementation_name(name: &str, argc: usize) -> Option<String> {
    if name == CLEAR_ATTRIBUTES {
        return Some(
            if argc <= 1 {
                "__astrings_clearAttributes"
            } else {
                "__astrings_clearAttributesRange"
            }
            .to_string(),
        );
    }
    DefaultResolver::implementation_name(&ASTRINGS, name).map(str::to_string)
}

// A `&'static` borrowed shape the owned `DefaultResolver::param_names` cannot
// produce, PINNED equal to `ASTRINGS` by the parity test.
pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    match name {
        FROM_STRING => Some(&[&["text"]]),
        BOLD | ITALIC | UNDERLINE | STRIKE | OVERLINE => Some(&[]),
        FONT => Some(&[&["name"]]),
        FONT_SIZE => Some(&[&["size"]]),
        ADD_ATTRIBUTE | REMOVE_ATTRIBUTE => {
            Some(&[&["value"], &["start"], &["endIndex"], &["attr"]])
        }
        GET_ATTRIBUTES => Some(&[&["value"], &["index"]]),
        TO_MARKDOWN => Some(&[&["value"]]),
        READ_SPANS | SCALAR_LEN => Some(&[&["value"]]),
        WRITE_SPANS => Some(&[&["value"], &["spans"]]),
        // `clearAttributes` overloads on arity (1 vs 3 args), so its per-position
        // spellings live in `param_name_overloads`, not here.
        _ => None,
    }
}

super::package_source_glue!(
    "astrings",
    "<builtin-astrings>",
    "builtins/astrings.mfb",
    include_str!("astrings_package.mfb")
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_astrings_call_recognizes_members_and_rejects_others() {
        assert!(is_astrings_call(FROM_STRING));
        assert!(is_astrings_call(BOLD));
        assert!(is_astrings_call(ADD_ATTRIBUTE));
        assert!(is_astrings_call(GET_ATTRIBUTES));
        assert!(!is_astrings_call("astrings.unknown"));
        assert!(!is_astrings_call("strings.trim"));
        assert!(!is_astrings_call(""));
    }

    #[test]
    fn from_string_descriptor_shape() {
        assert_eq!(call_param_names(FROM_STRING), Some(&[&["text"][..]][..]));
        assert_eq!(
            DefaultResolver::resolve_call(&ASTRINGS, FROM_STRING, &["String".to_string()]),
            Some("AttributedString")
        );
        assert_eq!(DefaultResolver::arity(&ASTRINGS, FROM_STRING), Some((1, 1)));
    }

    #[test]
    fn constructors_return_attribute() {
        assert_eq!(
            DefaultResolver::resolve_call(&ASTRINGS, BOLD, &[]),
            Some("Attribute")
        );
        assert_eq!(
            DefaultResolver::resolve_call(&ASTRINGS, FONT, &["String".to_string()]),
            Some("Attribute")
        );
    }

    #[test]
    fn clear_attributes_overloads_on_arity() {
        assert_eq!(DefaultResolver::arity(&ASTRINGS, CLEAR_ATTRIBUTES), Some((1, 3)));
    }

    #[test]
    fn internal_bridge_is_internal_only() {
        assert!(is_astrings_internal_call(READ_SPANS));
        assert!(is_astrings_internal_call(WRITE_SPANS));
        assert!(!is_astrings_internal_call(FROM_STRING));
    }

    #[test]
    fn source_companion_parses() {
        assert!(source_file().is_ok());
    }
}
