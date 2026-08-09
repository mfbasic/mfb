use super::descriptor::{
    BuiltinFlags, BuiltinFunction, BuiltinModule, BuiltinOverload, DefaultResolver, Implementation,
    Lowering, Parameter, ReturnType,
};

// plan-89: the `astrings` package — construction, mutation, query, and rendering
// for the opaque, value-semantic `AttributedString` type (registered as a
// hardcoded Family-B type in the resolver/syntaxcheck/binary_repr layers, not as
// a descriptor `types` entry — it is always in scope, like `Error`). Letter A
// lands the package shell with a single native constructor, `fromString`; letters
// B–E add the `Attribute` model, mutation/query members, `strings::` overloads,
// and `toMarkdown`.

const FROM_STRING: &str = "astrings.fromString";

// `astrings` is descriptor-authoritative: `resolve_call_return_type`, `arity`, and
// `expected_arguments` all derive from this table (`fromString` renders as
// `"String"` through `DefaultResolver`, so no hand-authored `expected_arguments`
// is needed). Only `call_param_names` returns a `&'static` borrowed shape the
// owned `DefaultResolver` cannot produce, so it stays a static table below.
const P_FROM_STRING: &[Parameter] = &[Parameter::required("text", "String")];

const OV_FROM_STRING: &[BuiltinOverload] = &[BuiltinOverload {
    params: P_FROM_STRING,
    return_type: ReturnType::Fixed("AttributedString"),
}];

const fn astrings_fn(
    name: &'static str,
    slug: &'static str,
    overloads: &'static [BuiltinOverload],
) -> BuiltinFunction {
    BuiltinFunction {
        name,
        doc_slug: slug,
        overloads,
        implementation: Implementation::Same,
        // Native codegen (see `builder_astrings.rs::lower_astrings_package_call`),
        // reached through the shared `bl`/helper lowering class like `strings::`.
        lowering: Lowering::Helper,
        flags: BuiltinFlags {
            internal_only: false,
            return_type_overloaded: false,
        },
    }
}

const ASTRINGS_FUNCTIONS: &[BuiltinFunction] =
    &[astrings_fn(FROM_STRING, "fromString", OV_FROM_STRING)];

pub(crate) static ASTRINGS: BuiltinModule = BuiltinModule {
    name: "astrings",
    functions: ASTRINGS_FUNCTIONS,
    // `AttributedString` is a hardcoded always-in-scope type, not a descriptor
    // type contributed by this package (see the module comment).
    types: &[],
    // Letter A is native-only; letter B adds the `Attribute` model source
    // companion.
    source: None,
    resolver: None,
};

pub(crate) fn is_astrings_call(name: &str) -> bool {
    DefaultResolver::contains(&ASTRINGS, name)
}

// A `&'static` borrowed shape the owned `DefaultResolver::param_names` cannot
// produce, PINNED equal to `ASTRINGS` by the parity test.
pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    match name {
        FROM_STRING => Some(&[&["text"]]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_astrings_call_recognizes_members_and_rejects_others() {
        assert!(is_astrings_call(FROM_STRING));
        assert!(!is_astrings_call("astrings.unknown"));
        assert!(!is_astrings_call("strings.trim"));
        assert!(!is_astrings_call(""));
    }

    #[test]
    fn from_string_descriptor_shape() {
        assert_eq!(
            call_param_names(FROM_STRING),
            Some(&[&["text"][..]][..])
        );
        assert_eq!(
            DefaultResolver::resolve_call(&ASTRINGS, FROM_STRING, &["String".to_string()]),
            Some("AttributedString")
        );
        assert_eq!(DefaultResolver::arity(&ASTRINGS, FROM_STRING), Some((1, 1)));
    }
}
