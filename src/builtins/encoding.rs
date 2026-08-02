use std::borrow::Cow;

use super::descriptor::{
    BuiltinFlags, BuiltinFunction, BuiltinModule, BuiltinOverload, BuiltinResolver, BuiltinSource,
    DefaultResolver, DefaultValue, Implementation, InjectionRule, Lowering, Parameter,
    ParameterType, ReturnType,
};

// Byte<->text and Unicode codecs, implemented in MFBASIC source over `bits`,
// `strings`, and `collections` (see `encoding_package.mfb`). Public names map to
// internal `__encoding_*` helpers via `implementation_name`; the two overloaded
// names (`utf8Encode` return-type overload, `utf8Decode` parameter overload) are
// resolved in the type checker and monomorphizer (see `resolve_overload_target`).
// See `plan-02-encoding.md` Part B.

const UTF8_ENCODE: &str = "encoding.utf8Encode";
const UTF8_DECODE: &str = "encoding.utf8Decode";
const UTF16_ENCODE: &str = "encoding.utf16Encode";
const UTF16_DECODE: &str = "encoding.utf16Decode";
const UTF32_ENCODE: &str = "encoding.utf32Encode";
const UTF32_DECODE: &str = "encoding.utf32Decode";
const HEX_ENCODE: &str = "encoding.hexEncode";
const HEX_DECODE: &str = "encoding.hexDecode";
const BASE32_ENCODE: &str = "encoding.base32Encode";
const BASE32_DECODE: &str = "encoding.base32Decode";
const BASE64_ENCODE: &str = "encoding.base64Encode";
const BASE64_DECODE: &str = "encoding.base64Decode";
const BASE64URL_ENCODE: &str = "encoding.base64UrlEncode";
const BASE64URL_DECODE: &str = "encoding.base64UrlDecode";
const PERCENT_ENCODE: &str = "encoding.percentEncode";
const PERCENT_DECODE: &str = "encoding.percentDecode";
const HTML_ESCAPE: &str = "encoding.htmlEscape";
const HTML_UNESCAPE: &str = "encoding.htmlUnescape";
const FORM_URL_ENCODE: &str = "encoding.formUrlEncode";
const FORM_URL_DECODE: &str = "encoding.formUrlDecode";
const PUNYCODE_ENCODE: &str = "encoding.punycodeEncode";
const PUNYCODE_DECODE: &str = "encoding.punycodeDecode";
const ULEB128_ENCODE: &str = "encoding.uleb128Encode";
const ULEB128_DECODE: &str = "encoding.uleb128Decode";
const SLEB128_ENCODE: &str = "encoding.sleb128Encode";
const SLEB128_DECODE: &str = "encoding.sleb128Decode";
const VARINT_ENCODE: &str = "encoding.varintEncode";
const VARINT_DECODE: &str = "encoding.varintDecode";

// The concrete dispatch targets the overloaded `utf8Encode`/`utf8Decode` names
// resolve to during monomorphization. They are package-qualified (so the
// post-monomorph resolver accepts them as built-in members) and map to their
// internal implementation in `implementation_name`, exactly like the other
// non-overloaded functions.
const UTF8_ENCODE_BYTES: &str = "encoding.utf8EncodeBytes";
const UTF8_ENCODE_INTS: &str = "encoding.utf8EncodeInts";
const UTF8_DECODE_BYTES: &str = "encoding.utf8DecodeBytes";
const UTF8_DECODE_INTS: &str = "encoding.utf8DecodeInts";

const BYTES: &str = "List OF Byte";
const INTS: &str = "List OF Integer";

// plan-72-I: `ENCODING` is the descriptor authority. Every function is unary
// with a fixed return, so `is_encoding_call`/`arity`/`call_return_type_name`/
// `implementation_name` derive from the descriptor. Non-overloaded functions (and
// the 4 monomorph targets) carry `Implementation::Rewrite(__encoding_*)`; the two
// overloaded names `utf8Encode`/`utf8Decode` are `Implementation::Custom`
// (`is_overloaded`), resolved by `EncodingResolver::resolve_overload_target`.
// `resolve_call` argument validation is also resolver-owned. `WhenImported` source.
const fn p(name: &'static str, aliases: &'static [&'static str], ty: &'static str) -> Parameter {
    Parameter {
        name,
        aliases,
        ty: ParameterType::Named(ty),
        default: DefaultValue::None,
    }
}
const fn ov(params: &'static [Parameter], ret: &'static str) -> BuiltinOverload {
    BuiltinOverload {
        params,
        return_type: ReturnType::Fixed(ret),
    }
}
const fn ef(
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
const VALTEXT: &[&str] = &["text"];

const ENCODING_FUNCTIONS: &[BuiltinFunction] = &[
    // The two overloaded names: Custom implementation (resolved by the resolver).
    ef(UTF8_ENCODE, "utf8Encode", &[ov(&[p("value", VALTEXT, "String")], BYTES)], Implementation::Custom),
    ef(UTF8_DECODE, "utf8Decode", &[ov(&[p("value", &[], BYTES)], "String")], Implementation::Custom),
    // The 4 monomorph targets.
    ef(UTF8_ENCODE_BYTES, "utf8EncodeBytes", &[ov(&[p("value", &[], "String")], BYTES)], Implementation::Rewrite("__encoding_utf8EncodeBytes")),
    ef(UTF8_ENCODE_INTS, "utf8EncodeInts", &[ov(&[p("value", &[], "String")], INTS)], Implementation::Rewrite("__encoding_utf8EncodeInts")),
    ef(UTF8_DECODE_BYTES, "utf8DecodeBytes", &[ov(&[p("value", &[], BYTES)], "String")], Implementation::Rewrite("__encoding_utf8DecodeBytes")),
    ef(UTF8_DECODE_INTS, "utf8DecodeInts", &[ov(&[p("value", &[], INTS)], "String")], Implementation::Rewrite("__encoding_utf8DecodeInts")),
    // Non-overloaded codecs.
    ef(UTF16_ENCODE, "utf16Encode", &[ov(&[p("value", VALTEXT, "String")], INTS)], Implementation::Rewrite("__encoding_utf16Encode")),
    ef(UTF16_DECODE, "utf16Decode", &[ov(&[p("value", &[], INTS)], "String")], Implementation::Rewrite("__encoding_utf16Decode")),
    ef(UTF32_ENCODE, "utf32Encode", &[ov(&[p("value", VALTEXT, "String")], INTS)], Implementation::Rewrite("__encoding_utf32Encode")),
    ef(UTF32_DECODE, "utf32Decode", &[ov(&[p("value", &[], INTS)], "String")], Implementation::Rewrite("__encoding_utf32Decode")),
    ef(HEX_ENCODE, "hexEncode", &[ov(&[p("data", &[], BYTES)], "String")], Implementation::Rewrite("__encoding_hexEncode")),
    ef(HEX_DECODE, "hexDecode", &[ov(&[p("text", &[], "String")], BYTES)], Implementation::Rewrite("__encoding_hexDecode")),
    ef(BASE32_ENCODE, "base32Encode", &[ov(&[p("data", &[], BYTES)], "String")], Implementation::Rewrite("__encoding_base32Encode")),
    ef(BASE32_DECODE, "base32Decode", &[ov(&[p("text", &[], "String")], BYTES)], Implementation::Rewrite("__encoding_base32Decode")),
    ef(BASE64_ENCODE, "base64Encode", &[ov(&[p("data", &[], BYTES)], "String")], Implementation::Rewrite("__encoding_base64Encode")),
    ef(BASE64_DECODE, "base64Decode", &[ov(&[p("text", &[], "String")], BYTES)], Implementation::Rewrite("__encoding_base64Decode")),
    ef(BASE64URL_ENCODE, "base64UrlEncode", &[ov(&[p("data", &[], BYTES)], "String")], Implementation::Rewrite("__encoding_base64UrlEncode")),
    ef(BASE64URL_DECODE, "base64UrlDecode", &[ov(&[p("text", &[], "String")], BYTES)], Implementation::Rewrite("__encoding_base64UrlDecode")),
    ef(PERCENT_ENCODE, "percentEncode", &[ov(&[p("value", VALTEXT, "String")], "String")], Implementation::Rewrite("__encoding_percentEncode")),
    ef(PERCENT_DECODE, "percentDecode", &[ov(&[p("value", VALTEXT, "String")], "String")], Implementation::Rewrite("__encoding_percentDecode")),
    ef(HTML_ESCAPE, "htmlEscape", &[ov(&[p("value", VALTEXT, "String")], "String")], Implementation::Rewrite("__encoding_htmlEscape")),
    ef(HTML_UNESCAPE, "htmlUnescape", &[ov(&[p("value", VALTEXT, "String")], "String")], Implementation::Rewrite("__encoding_htmlUnescape")),
    ef(FORM_URL_ENCODE, "formUrlEncode", &[ov(&[p("value", VALTEXT, "String")], "String")], Implementation::Rewrite("__encoding_formUrlEncode")),
    ef(FORM_URL_DECODE, "formUrlDecode", &[ov(&[p("value", VALTEXT, "String")], "String")], Implementation::Rewrite("__encoding_formUrlDecode")),
    ef(PUNYCODE_ENCODE, "punycodeEncode", &[ov(&[p("domain", &[], "String")], "String")], Implementation::Rewrite("__encoding_punycodeEncode")),
    ef(PUNYCODE_DECODE, "punycodeDecode", &[ov(&[p("asciiDomain", &[], "String")], "String")], Implementation::Rewrite("__encoding_punycodeDecode")),
    ef(ULEB128_ENCODE, "uleb128Encode", &[ov(&[p("value", &[], "Integer")], BYTES)], Implementation::Rewrite("__encoding_uleb128Encode")),
    ef(ULEB128_DECODE, "uleb128Decode", &[ov(&[p("data", &[], BYTES)], "Integer")], Implementation::Rewrite("__encoding_uleb128Decode")),
    ef(SLEB128_ENCODE, "sleb128Encode", &[ov(&[p("value", &[], "Integer")], BYTES)], Implementation::Rewrite("__encoding_sleb128Encode")),
    ef(SLEB128_DECODE, "sleb128Decode", &[ov(&[p("data", &[], BYTES)], "Integer")], Implementation::Rewrite("__encoding_sleb128Decode")),
    ef(VARINT_ENCODE, "varintEncode", &[ov(&[p("value", &[], "Integer")], BYTES)], Implementation::Rewrite("__encoding_varintEncode")),
    ef(VARINT_DECODE, "varintDecode", &[ov(&[p("data", &[], BYTES)], "Integer")], Implementation::Rewrite("__encoding_varintDecode")),
];

/// Argument-dependent resolution for encoding: `resolve_call` validation and the
/// overloaded `utf8Encode`/`utf8Decode` monomorph-target selection. Both delegate
/// to the retained `dispatch_*` helpers.
struct EncodingResolver;
impl BuiltinResolver for EncodingResolver {
    fn resolve_return_type(
        &self,
        _module: &BuiltinModule,
        name: &str,
        arg_types: &[String],
    ) -> Option<String> {
        dispatch_resolve(name, arg_types).map(|resolved| resolved.return_type.into_owned())
    }

    fn resolve_overload_target(
        &self,
        _module: &BuiltinModule,
        name: &str,
        arg_types: &[String],
        expected_type: Option<&str>,
    ) -> Result<Option<String>, ()> {
        dispatch_overload_target(name, arg_types, expected_type)
            .map(|opt| opt.map(str::to_string))
    }
}
static ENCODING_RESOLVER: EncodingResolver = EncodingResolver;

pub(crate) static ENCODING: BuiltinModule = BuiltinModule {
    name: "encoding",
    functions: ENCODING_FUNCTIONS,
    types: &[],
    source: Some(BuiltinSource {
        rule: InjectionRule::WhenImported,
        loader: source_file,
    }),
    resolver: Some(&ENCODING_RESOLVER),
};

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

pub(crate) fn is_encoding_call(name: &str) -> bool {
    DefaultResolver::contains(&ENCODING, name)
}

// `call_param_names`/`expected_arguments`/`argument_types` return `&'static`
// borrowed shapes the owned `DefaultResolver` cannot produce (and the latter two
// use bespoke phrasing). They stay static: `call_param_names` PINNED equal to
// `ENCODING` by the parity test; the others verified by the existing tests. BB
// removes them.
pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    match name {
        UTF8_ENCODE | UTF16_ENCODE | UTF32_ENCODE | PERCENT_ENCODE | PERCENT_DECODE
        | HTML_ESCAPE | HTML_UNESCAPE | FORM_URL_ENCODE | FORM_URL_DECODE => {
            Some(&[&["value", "text"]])
        }
        UTF8_DECODE | UTF16_DECODE | UTF32_DECODE => Some(&[&["value"]]),
        HEX_ENCODE | BASE32_ENCODE | BASE64_ENCODE | BASE64URL_ENCODE => Some(&[&["data"]]),
        HEX_DECODE | BASE32_DECODE | BASE64_DECODE | BASE64URL_DECODE => Some(&[&["text"]]),
        PUNYCODE_ENCODE => Some(&[&["domain"]]),
        PUNYCODE_DECODE => Some(&[&["asciiDomain"]]),
        ULEB128_ENCODE | SLEB128_ENCODE | VARINT_ENCODE => Some(&[&["value"]]),
        ULEB128_DECODE | SLEB128_DECODE | VARINT_DECODE => Some(&[&["data"]]),
        _ => None,
    }
}

pub(crate) fn call_return_type_name(name: &str) -> Option<&'static str> {
    // utf8Encode reports its default (List OF Byte) form; the precise type is
    // resolved with the contextual expected type in the checker/monomorphizer.
    DefaultResolver::return_type_name(&ENCODING, name)
}

pub(crate) fn arity(name: &str) -> Option<(usize, usize)> {
    DefaultResolver::arity(&ENCODING, name)
}

pub(crate) fn expected_arguments(name: &str) -> Option<&'static str> {
    match name {
        UTF8_ENCODE | UTF8_ENCODE_BYTES | UTF8_ENCODE_INTS | UTF16_ENCODE | UTF32_ENCODE
        | PERCENT_ENCODE | PERCENT_DECODE | HTML_ESCAPE | HTML_UNESCAPE | FORM_URL_ENCODE
        | FORM_URL_DECODE | PUNYCODE_ENCODE | PUNYCODE_DECODE | HEX_DECODE | BASE32_DECODE
        | BASE64_DECODE | BASE64URL_DECODE => Some("String"),
        UTF8_DECODE => Some("List OF Byte or List OF Integer"),
        UTF8_DECODE_BYTES => Some(BYTES),
        UTF8_DECODE_INTS | UTF16_DECODE | UTF32_DECODE => Some(INTS),
        HEX_ENCODE | BASE32_ENCODE | BASE64_ENCODE | BASE64URL_ENCODE | ULEB128_DECODE
        | SLEB128_DECODE | VARINT_DECODE => Some(BYTES),
        ULEB128_ENCODE | SLEB128_ENCODE | VARINT_ENCODE => Some("Integer"),
        _ => None,
    }
}

/// The machine-readable positional argument-type signature (bug-340 A1). Every
/// `encoding::` member is unary, so each entry is a one-element slice — except
/// `utf8Decode`, which is overloaded on `List OF Byte | List OF Integer` and so
/// has no single positional signature (`None`, as before). IR lowering reads this
/// directly instead of parsing the `expected_arguments` diagnostic string.
pub(crate) fn argument_types(name: &str) -> Option<&'static [&'static str]> {
    match name {
        UTF8_ENCODE | UTF8_ENCODE_BYTES | UTF8_ENCODE_INTS | UTF16_ENCODE | UTF32_ENCODE
        | PERCENT_ENCODE | PERCENT_DECODE | HTML_ESCAPE | HTML_UNESCAPE | FORM_URL_ENCODE
        | FORM_URL_DECODE | PUNYCODE_ENCODE | PUNYCODE_DECODE | HEX_DECODE | BASE32_DECODE
        | BASE64_DECODE | BASE64URL_DECODE => Some(&["String"]),
        UTF8_DECODE => None,
        UTF8_DECODE_BYTES => Some(&[BYTES]),
        UTF8_DECODE_INTS | UTF16_DECODE | UTF32_DECODE => Some(&[INTS]),
        HEX_ENCODE | BASE32_ENCODE | BASE64_ENCODE | BASE64URL_ENCODE | ULEB128_DECODE
        | SLEB128_DECODE | VARINT_DECODE => Some(&[BYTES]),
        ULEB128_ENCODE | SLEB128_ENCODE | VARINT_ENCODE => Some(&["Integer"]),
        _ => None,
    }
}

pub(crate) fn resolve_call<'a>(name: &str, arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    let return_type = ENCODING
        .resolver?
        .resolve_return_type(&ENCODING, name, arg_types)?;
    Some(ResolvedCall {
        return_type: Cow::Owned(return_type),
    })
}

/// The argument-validating return-type resolution, invoked through the descriptor
/// resolver by `resolve_call`. Every member is unary; `utf8Decode` accepts either
/// `List OF Byte` or `List OF Integer`.
fn dispatch_resolve<'a>(name: &str, arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    if arg_types.len() != 1 {
        return None;
    }
    let arg = arg_types[0].as_str();
    let return_type: Cow<'a, str> = match name {
        // utf8Encode: String -> List OF Byte | List OF Integer (return overload).
        // Resolved precisely via the expected type; default to List OF Byte here.
        UTF8_ENCODE if arg == "String" => Cow::Borrowed(BYTES),
        UTF8_ENCODE_BYTES if arg == "String" => Cow::Borrowed(BYTES),
        UTF8_ENCODE_INTS if arg == "String" => Cow::Borrowed(INTS),
        UTF8_DECODE if arg == BYTES || arg == INTS => Cow::Borrowed("String"),
        UTF8_DECODE_BYTES if arg == BYTES => Cow::Borrowed("String"),
        UTF8_DECODE_INTS if arg == INTS => Cow::Borrowed("String"),
        UTF16_ENCODE | UTF32_ENCODE if arg == "String" => Cow::Borrowed(INTS),
        UTF16_DECODE | UTF32_DECODE if arg == INTS => Cow::Borrowed("String"),
        HEX_ENCODE | BASE32_ENCODE | BASE64_ENCODE | BASE64URL_ENCODE if arg == BYTES => {
            Cow::Borrowed("String")
        }
        HEX_DECODE | BASE32_DECODE | BASE64_DECODE | BASE64URL_DECODE if arg == "String" => {
            Cow::Borrowed(BYTES)
        }
        PERCENT_ENCODE | PERCENT_DECODE | HTML_ESCAPE | HTML_UNESCAPE | FORM_URL_ENCODE
        | FORM_URL_DECODE | PUNYCODE_ENCODE | PUNYCODE_DECODE
            if arg == "String" =>
        {
            Cow::Borrowed("String")
        }
        ULEB128_ENCODE | SLEB128_ENCODE | VARINT_ENCODE if arg == "Integer" => Cow::Borrowed(BYTES),
        ULEB128_DECODE | SLEB128_DECODE | VARINT_DECODE if arg == BYTES => Cow::Borrowed("Integer"),
        _ => return None,
    };
    Some(ResolvedCall { return_type })
}

/// The non-overloaded public encoding functions map one-to-one onto their
/// internal `__encoding_*` implementation. The two overloaded names
/// (`utf8Encode`/`utf8Decode`) return `None`; they are rewritten by
/// `resolve_overload_target` during monomorphization using the call's argument
/// and expected types.
pub(crate) fn implementation_name(name: &str) -> Option<&'static str> {
    // Non-overloaded names carry an `Implementation::Rewrite(__encoding_*)`; the
    // two overloaded names are `Custom` and resolve to a target via
    // `resolve_overload_target`, so they return `None` here.
    DefaultResolver::implementation_name(&ENCODING, name)
}

/// Resolve the overloaded `utf8Encode`/`utf8Decode` public calls to a concrete
/// internal implementation, using the call's argument types and the expected
/// (contextual) type. Returns `Ok(Some(name))` on a unique match, `Ok(None)`
/// when the callee is not an overloaded encoding name, and `Err(())` when a
/// return-type overload cannot be resolved without an expected type
/// (`utf8Encode` with no `List OF Byte`/`List OF Integer` context). Invoked
/// through the descriptor resolver by `builtins::resolve_overload_target`.
fn dispatch_overload_target(
    callee: &str,
    arg_types: &[String],
    expected_type: Option<&str>,
) -> Result<Option<&'static str>, ()> {
    match callee {
        UTF8_ENCODE if arg_types == ["String"] => match expected_type {
            Some(BYTES) => Ok(Some(UTF8_ENCODE_BYTES)),
            Some(INTS) => Ok(Some(UTF8_ENCODE_INTS)),
            _ => Err(()),
        },
        UTF8_DECODE if arg_types == [BYTES] => Ok(Some(UTF8_DECODE_BYTES)),
        UTF8_DECODE if arg_types == [INTS] => Ok(Some(UTF8_DECODE_INTS)),
        _ => Ok(None),
    }
}

/// Whether `callee` is one of the overloaded encoding public names: derived from
/// the descriptor (an overloaded name carries `Implementation::Custom`).
pub(crate) fn is_overloaded(callee: &str) -> bool {
    ENCODING
        .function(callee)
        .is_some_and(|function| matches!(function.implementation, Implementation::Custom))
}

super::package_source_glue!(
    "encoding",
    "<builtin-encoding>",
    "builtins/encoding.mfb",
    include_str!("encoding_package.mfb")
);

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

    fn rt(name: &str, args: &[&str]) -> Option<String> {
        resolve_call(name, &strings(args)).map(|r| r.return_type.into_owned())
    }

    const ALL_PUBLIC: &[&str] = &[
        UTF8_ENCODE,
        UTF8_DECODE,
        UTF16_ENCODE,
        UTF16_DECODE,
        UTF32_ENCODE,
        UTF32_DECODE,
        HEX_ENCODE,
        HEX_DECODE,
        BASE32_ENCODE,
        BASE32_DECODE,
        BASE64_ENCODE,
        BASE64_DECODE,
        BASE64URL_ENCODE,
        BASE64URL_DECODE,
        PERCENT_ENCODE,
        PERCENT_DECODE,
        HTML_ESCAPE,
        HTML_UNESCAPE,
        FORM_URL_ENCODE,
        FORM_URL_DECODE,
        PUNYCODE_ENCODE,
        PUNYCODE_DECODE,
        ULEB128_ENCODE,
        ULEB128_DECODE,
        SLEB128_ENCODE,
        SLEB128_DECODE,
        VARINT_ENCODE,
        VARINT_DECODE,
    ];

    #[test]
    fn is_call_recognizes_and_rejects() {
        for n in ALL_PUBLIC {
            assert!(is_encoding_call(n), "{n}");
        }
        for n in [
            UTF8_ENCODE_BYTES,
            UTF8_ENCODE_INTS,
            UTF8_DECODE_BYTES,
            UTF8_DECODE_INTS,
        ] {
            assert!(is_encoding_call(n), "{n}");
        }
        assert!(!is_encoding_call("encoding.nope"));
        assert!(!is_encoding_call("other.utf8Encode"));
    }

    #[test]
    fn param_names_branches() {
        assert_eq!(
            call_param_names(UTF8_ENCODE),
            Some(&[&["value", "text"][..]][..])
        );
        assert_eq!(call_param_names(UTF8_DECODE), Some(&[&["value"][..]][..]));
        assert_eq!(call_param_names(HEX_ENCODE), Some(&[&["data"][..]][..]));
        assert_eq!(call_param_names(HEX_DECODE), Some(&[&["text"][..]][..]));
        assert_eq!(
            call_param_names(PUNYCODE_ENCODE),
            Some(&[&["domain"][..]][..])
        );
        assert_eq!(
            call_param_names(PUNYCODE_DECODE),
            Some(&[&["asciiDomain"][..]][..])
        );
        assert_eq!(
            call_param_names(ULEB128_ENCODE),
            Some(&[&["value"][..]][..])
        );
        assert_eq!(call_param_names(ULEB128_DECODE), Some(&[&["data"][..]][..]));
        assert!(call_param_names("encoding.nope").is_none());
    }

    #[test]
    fn return_type_name_branches() {
        assert_eq!(call_return_type_name(UTF8_ENCODE), Some(BYTES));
        assert_eq!(call_return_type_name(UTF8_ENCODE_BYTES), Some(BYTES));
        assert_eq!(call_return_type_name(HEX_DECODE), Some(BYTES));
        assert_eq!(call_return_type_name(ULEB128_ENCODE), Some(BYTES));
        assert_eq!(call_return_type_name(UTF16_ENCODE), Some(INTS));
        assert_eq!(call_return_type_name(UTF8_ENCODE_INTS), Some(INTS));
        assert_eq!(call_return_type_name(UTF8_DECODE), Some("String"));
        assert_eq!(call_return_type_name(HEX_ENCODE), Some("String"));
        assert_eq!(call_return_type_name(PERCENT_ENCODE), Some("String"));
        assert_eq!(call_return_type_name(PUNYCODE_DECODE), Some("String"));
        assert_eq!(call_return_type_name(ULEB128_DECODE), Some("Integer"));
        assert_eq!(call_return_type_name(VARINT_DECODE), Some("Integer"));
        assert!(call_return_type_name("encoding.nope").is_none());
    }

    #[test]
    fn arity_is_unary_for_all() {
        for n in ALL_PUBLIC {
            assert_eq!(arity(n), Some((1, 1)), "{n}");
        }
        assert!(arity("encoding.nope").is_none());
    }

    #[test]
    fn expected_arguments_branches() {
        assert_eq!(expected_arguments(UTF8_ENCODE), Some("String"));
        assert_eq!(expected_arguments(HEX_DECODE), Some("String"));
        assert_eq!(
            expected_arguments(UTF8_DECODE),
            Some("List OF Byte or List OF Integer")
        );
        assert_eq!(expected_arguments(UTF8_DECODE_BYTES), Some(BYTES));
        assert_eq!(expected_arguments(UTF8_DECODE_INTS), Some(INTS));
        assert_eq!(expected_arguments(UTF16_DECODE), Some(INTS));
        assert_eq!(expected_arguments(HEX_ENCODE), Some(BYTES));
        assert_eq!(expected_arguments(ULEB128_DECODE), Some(BYTES));
        assert_eq!(expected_arguments(ULEB128_ENCODE), Some("Integer"));
        assert!(expected_arguments("encoding.nope").is_none());
    }

    #[test]
    fn argument_types_machine_table() {
        // bug-340 A1: the machine-readable positional signature IR lowering reads.
        // Every member is unary, so each is a one-element slice — except the
        // overloaded `utf8Decode`, which has no single signature.
        assert_eq!(argument_types(UTF8_ENCODE), Some(&["String"][..]));
        assert_eq!(argument_types(UTF8_DECODE), None);
        assert_eq!(argument_types(UTF8_DECODE_BYTES), Some(&[BYTES][..]));
        assert_eq!(argument_types(UTF32_DECODE), Some(&[INTS][..]));
        assert_eq!(argument_types(HEX_ENCODE), Some(&[BYTES][..]));
        assert_eq!(argument_types(ULEB128_ENCODE), Some(&["Integer"][..]));
        assert!(argument_types("encoding.nope").is_none());
    }

    #[test]
    fn resolve_wrong_arity_none() {
        assert_eq!(rt(HEX_ENCODE, &[]), None);
        assert_eq!(rt(HEX_ENCODE, &[BYTES, BYTES]), None);
    }

    #[test]
    fn resolve_each_branch() {
        assert_eq!(rt(UTF8_ENCODE, &["String"]), Some(BYTES.to_string()));
        assert_eq!(rt(UTF8_ENCODE_BYTES, &["String"]), Some(BYTES.to_string()));
        assert_eq!(rt(UTF8_ENCODE_INTS, &["String"]), Some(INTS.to_string()));
        assert_eq!(rt(UTF8_DECODE, &[BYTES]), Some("String".to_string()));
        assert_eq!(rt(UTF8_DECODE, &[INTS]), Some("String".to_string()));
        assert_eq!(rt(UTF8_DECODE_BYTES, &[BYTES]), Some("String".to_string()));
        assert_eq!(rt(UTF8_DECODE_INTS, &[INTS]), Some("String".to_string()));
        assert_eq!(rt(UTF16_ENCODE, &["String"]), Some(INTS.to_string()));
        assert_eq!(rt(UTF32_ENCODE, &["String"]), Some(INTS.to_string()));
        assert_eq!(rt(UTF16_DECODE, &[INTS]), Some("String".to_string()));
        assert_eq!(rt(UTF32_DECODE, &[INTS]), Some("String".to_string()));
        assert_eq!(rt(HEX_ENCODE, &[BYTES]), Some("String".to_string()));
        assert_eq!(rt(BASE32_ENCODE, &[BYTES]), Some("String".to_string()));
        assert_eq!(rt(BASE64_ENCODE, &[BYTES]), Some("String".to_string()));
        assert_eq!(rt(BASE64URL_ENCODE, &[BYTES]), Some("String".to_string()));
        assert_eq!(rt(HEX_DECODE, &["String"]), Some(BYTES.to_string()));
        assert_eq!(rt(BASE32_DECODE, &["String"]), Some(BYTES.to_string()));
        assert_eq!(rt(BASE64_DECODE, &["String"]), Some(BYTES.to_string()));
        assert_eq!(rt(BASE64URL_DECODE, &["String"]), Some(BYTES.to_string()));
        assert_eq!(rt(PERCENT_ENCODE, &["String"]), Some("String".to_string()));
        assert_eq!(rt(PERCENT_DECODE, &["String"]), Some("String".to_string()));
        assert_eq!(rt(HTML_ESCAPE, &["String"]), Some("String".to_string()));
        assert_eq!(rt(HTML_UNESCAPE, &["String"]), Some("String".to_string()));
        assert_eq!(rt(FORM_URL_ENCODE, &["String"]), Some("String".to_string()));
        assert_eq!(rt(FORM_URL_DECODE, &["String"]), Some("String".to_string()));
        assert_eq!(rt(PUNYCODE_ENCODE, &["String"]), Some("String".to_string()));
        assert_eq!(rt(PUNYCODE_DECODE, &["String"]), Some("String".to_string()));
        assert_eq!(rt(ULEB128_ENCODE, &["Integer"]), Some(BYTES.to_string()));
        assert_eq!(rt(SLEB128_ENCODE, &["Integer"]), Some(BYTES.to_string()));
        assert_eq!(rt(VARINT_ENCODE, &["Integer"]), Some(BYTES.to_string()));
        assert_eq!(rt(ULEB128_DECODE, &[BYTES]), Some("Integer".to_string()));
        assert_eq!(rt(SLEB128_DECODE, &[BYTES]), Some("Integer".to_string()));
        assert_eq!(rt(VARINT_DECODE, &[BYTES]), Some("Integer".to_string()));
    }

    #[test]
    fn resolve_wrong_types_none() {
        assert_eq!(rt(UTF8_ENCODE, &["Integer"]), None);
        assert_eq!(rt(UTF8_DECODE, &["String"]), None);
        assert_eq!(rt(HEX_ENCODE, &["String"]), None);
        assert_eq!(rt(HEX_DECODE, &[BYTES]), None);
        assert_eq!(rt(ULEB128_ENCODE, &[BYTES]), None);
        assert_eq!(rt("encoding.nope", &["String"]), None);
    }

    #[test]
    fn implementation_name_flat_map() {
        assert_eq!(
            implementation_name(UTF8_ENCODE_BYTES),
            Some("__encoding_utf8EncodeBytes")
        );
        assert_eq!(
            implementation_name(UTF8_ENCODE_INTS),
            Some("__encoding_utf8EncodeInts")
        );
        assert_eq!(
            implementation_name(UTF8_DECODE_BYTES),
            Some("__encoding_utf8DecodeBytes")
        );
        assert_eq!(
            implementation_name(UTF8_DECODE_INTS),
            Some("__encoding_utf8DecodeInts")
        );
        assert_eq!(
            implementation_name(HEX_ENCODE),
            Some("__encoding_hexEncode")
        );
        assert_eq!(
            implementation_name(VARINT_DECODE),
            Some("__encoding_varintDecode")
        );
        assert_eq!(
            implementation_name(PUNYCODE_ENCODE),
            Some("__encoding_punycodeEncode")
        );
        assert_eq!(
            implementation_name(FORM_URL_DECODE),
            Some("__encoding_formUrlDecode")
        );
        // overloaded names are not in the flat map
        assert_eq!(implementation_name(UTF8_ENCODE), None);
        assert_eq!(implementation_name(UTF8_DECODE), None);
        assert_eq!(implementation_name("encoding.nope"), None);
    }

    #[test]
    fn resolve_overload_target_all_paths() {
        // Route through the generic descriptor entry point (which delegates to
        // `EncodingResolver::resolve_overload_target`), the same path monomorph
        // uses. Results are owned `String`s.
        let target = |callee: &str, args: &[&str], expected: Option<&str>| {
            crate::builtins::resolve_overload_target(callee, &strings(args), expected)
        };
        assert_eq!(target(UTF8_ENCODE, &["String"], Some(BYTES)), Ok(Some(UTF8_ENCODE_BYTES.to_string())));
        assert_eq!(target(UTF8_ENCODE, &["String"], Some(INTS)), Ok(Some(UTF8_ENCODE_INTS.to_string())));
        // no expected type -> Err
        assert_eq!(target(UTF8_ENCODE, &["String"], None), Err(()));
        assert_eq!(target(UTF8_ENCODE, &["String"], Some("String")), Err(()));
        // utf8Encode with wrong arg types is not the overload arm -> Ok(None)
        assert_eq!(target(UTF8_ENCODE, &["Integer"], Some(BYTES)), Ok(None));
        assert_eq!(target(UTF8_DECODE, &[BYTES], None), Ok(Some(UTF8_DECODE_BYTES.to_string())));
        assert_eq!(target(UTF8_DECODE, &[INTS], None), Ok(Some(UTF8_DECODE_INTS.to_string())));
        // non-overloaded callee -> Ok(None)
        assert_eq!(target(HEX_ENCODE, &[BYTES], None), Ok(None));
    }

    #[test]
    fn is_overloaded_only_utf8() {
        assert!(is_overloaded(UTF8_ENCODE));
        assert!(is_overloaded(UTF8_DECODE));
        assert!(!is_overloaded(UTF16_ENCODE));
        assert!(!is_overloaded(HEX_ENCODE));
    }

    #[test]
    fn source_file_parses() {
        assert!(source_file().is_ok());
    }

    #[test]
    fn augmented_project_injects_when_imported() {
        let ast = project("IMPORT encoding\nSUB main\nEND SUB\n");
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

    // plan-72-I migration gate: prove `ENCODING` + `EncodingResolver` reproduce
    // the legacy answers — membership/arity/param-names + the flat implementation
    // rewrite map via the descriptor, and resolve_call validation + the
    // `utf8Encode`/`utf8Decode` overload targets via resolver samples. Keep until BB.
    #[test]
    fn parity_matches_descriptor() {
        use crate::builtins::descriptor::parity;

        let legacy = parity::LegacySet {
            is_call: &is_encoding_call,
            arity: &arity,
            param_names: &|name| {
                call_param_names(name).map(|rows| rows.iter().map(|row| row.to_vec()).collect())
            },
            return_type_name: &|_| None, // resolver-backed: skipped; verified by existing tests
            expected_arguments: None,    // custom "or"-phrased strings
            param_name_overloads: None,  // encoding has no per-overload name tables
            argument_types: None,
            implementation_name: None,   // descriptor-derived; verified by implementation_name_flat_map
            default_padding: None,
            builtin_type_fields: None,
        };
        // The 4 monomorph targets are members with arity (1,1) but no
        // `call_param_names` entry (they are never named-arg-bound), so they are
        // verified explicitly below rather than through the param-name harness.
        let mut probe: Vec<&str> = ALL_PUBLIC.to_vec();
        probe.push("encoding.nope");

        let ret = |call, args, r: &'static str| parity::ResolverSample {
            call, arg_types: args, expected_return: Some(r), expected_impl: None,
            expected_padding: None, expected_type: None, expected_overload_target: None,
        };
        let target = |call, args, expected: Option<&'static str>, t: &'static str| parity::ResolverSample {
            call, arg_types: args, expected_return: None, expected_impl: None,
            expected_padding: None, expected_type: expected, expected_overload_target: Some(t),
        };
        let samples = [
            ret(UTF8_ENCODE, &["String"], BYTES),
            ret(UTF8_ENCODE_INTS, &["String"], INTS),
            ret(UTF8_DECODE, &[BYTES], "String"),
            ret(UTF8_DECODE, &[INTS], "String"),
            ret(UTF16_ENCODE, &["String"], INTS),
            ret(HEX_ENCODE, &[BYTES], "String"),
            ret(HEX_DECODE, &["String"], BYTES),
            ret(PERCENT_ENCODE, &["String"], "String"),
            ret(ULEB128_ENCODE, &["Integer"], BYTES),
            ret(VARINT_DECODE, &[BYTES], "Integer"),
            // Overloaded utf8Encode/utf8Decode monomorph target selection.
            target(UTF8_ENCODE, &["String"], Some(BYTES), UTF8_ENCODE_BYTES),
            target(UTF8_ENCODE, &["String"], Some(INTS), UTF8_ENCODE_INTS),
            target(UTF8_DECODE, &[BYTES], None, UTF8_DECODE_BYTES),
            target(UTF8_DECODE, &[INTS], None, UTF8_DECODE_INTS),
        ];
        parity::assert_parity(&ENCODING, &probe, &legacy, &samples);

        // is_overloaded derives from the descriptor (Custom implementation).
        assert!(is_overloaded(UTF8_ENCODE));
        assert!(is_overloaded(UTF8_DECODE));
        assert!(!is_overloaded(HEX_ENCODE));
        assert!(!is_overloaded(UTF8_ENCODE_BYTES));
        // implementation_name (descriptor-derived) — overloaded names have none.
        assert_eq!(implementation_name(HEX_ENCODE), Some("__encoding_hexEncode"));
        assert_eq!(implementation_name(UTF8_ENCODE), None);
        assert_eq!(implementation_name(UTF8_ENCODE_BYTES), Some("__encoding_utf8EncodeBytes"));

        // The 4 monomorph targets: members with arity (1,1), the right return
        // type and implementation, and no call_param_names entry.
        for (target, ret, imp) in [
            (UTF8_ENCODE_BYTES, BYTES, "__encoding_utf8EncodeBytes"),
            (UTF8_ENCODE_INTS, INTS, "__encoding_utf8EncodeInts"),
            (UTF8_DECODE_BYTES, "String", "__encoding_utf8DecodeBytes"),
            (UTF8_DECODE_INTS, "String", "__encoding_utf8DecodeInts"),
        ] {
            assert!(is_encoding_call(target), "{target}");
            assert_eq!(arity(target), Some((1, 1)), "{target}");
            assert_eq!(call_return_type_name(target), Some(ret), "{target}");
            assert_eq!(implementation_name(target), Some(imp), "{target}");
            assert_eq!(call_param_names(target), None, "{target}");
        }
    }
}
