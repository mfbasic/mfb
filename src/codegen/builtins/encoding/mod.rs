use std::borrow::Cow;

use crate::codegen::registry::{
    BuiltinFunction, BuiltinModule, BuiltinOverload, BuiltinResolver, BuiltinSource,
    DefaultResolver, DefaultValue, Implementation, InjectionRule, Parameter, ParameterType,
    ReturnType,
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

mod func_base32_decode;
mod func_base32_encode;
mod func_base64_decode;
mod func_base64_encode;
mod func_base64_url_decode;
mod func_base64_url_encode;
mod func_form_url_decode;
mod func_form_url_encode;
mod func_hex_decode;
mod func_hex_encode;
mod func_html_escape;
mod func_html_unescape;
mod func_percent_decode;
mod func_percent_encode;
mod func_punycode_decode;
mod func_punycode_encode;
mod func_sleb128_decode;
mod func_sleb128_encode;
mod func_uleb128_decode;
mod func_uleb128_encode;
mod func_utf16_decode;
mod func_utf16_encode;
mod func_utf32_decode;
mod func_utf32_encode;
mod func_utf8_decode;
mod func_utf8_encode;
mod func_varint_decode;
mod func_varint_encode;

const BYTES: &str = "List OF Byte";
const INTS: &str = "List OF Integer";

// plan-72-I: `ENCODING` is the descriptor authority. Every function is unary
// with a fixed return, so `is_encoding_call`/`arity`/`call_return_type_name`/
// `implementation_name` derive from the descriptor. Non-overloaded functions (and
// the 4 monomorph targets) carry `Implementation::Mfb` (their `__encoding_*` body
// lives in the owning `func_*.rs`; the rewrite target comes from `IMPL_NAMES`);
// `utf8Encode` (return-type overload) and `utf8Decode` (parameter overload) are
// `Implementation::Resolve`: each owns its selector and its two **private** variant
// bodies inline in its `func_*.rs`. Both are `is_overloaded`. `resolve_call`
// argument validation is resolver-owned. `WhenImported` source.
// `ov`/`p` build the documentation overloads; each member's descriptor is
// constructed in its `func_*.rs` via `BuiltinFunction::mfb`/`::resolve`.
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
const VALTEXT: &[&str] = &["text"];

const ENCODING_FUNCTIONS: &[BuiltinFunction] = &[
    // The two overloaded names (`Implementation::Resolve`). Their byte/int monomorph
    // targets are private variants owned inline by their `func_*.rs`, not public
    // functions, so they do not appear in this list.
    func_utf8_encode::UTF8_ENCODE,
    func_utf8_decode::UTF8_DECODE,
    // Non-overloaded codecs.
    func_utf16_encode::UTF16_ENCODE,
    func_utf16_decode::UTF16_DECODE,
    func_utf32_encode::UTF32_ENCODE,
    func_utf32_decode::UTF32_DECODE,
    func_hex_encode::HEX_ENCODE,
    func_hex_decode::HEX_DECODE,
    func_base32_encode::BASE32_ENCODE,
    func_base32_decode::BASE32_DECODE,
    func_base64_encode::BASE64_ENCODE,
    func_base64_decode::BASE64_DECODE,
    func_base64_url_encode::BASE64_URL_ENCODE,
    func_base64_url_decode::BASE64_URL_DECODE,
    func_percent_encode::PERCENT_ENCODE,
    func_percent_decode::PERCENT_DECODE,
    func_html_escape::HTML_ESCAPE,
    func_html_unescape::HTML_UNESCAPE,
    func_form_url_encode::FORM_URL_ENCODE,
    func_form_url_decode::FORM_URL_DECODE,
    func_punycode_encode::PUNYCODE_ENCODE,
    func_punycode_decode::PUNYCODE_DECODE,
    func_uleb128_encode::ULEB128_ENCODE,
    func_uleb128_decode::ULEB128_DECODE,
    func_sleb128_encode::SLEB128_ENCODE,
    func_sleb128_decode::SLEB128_DECODE,
    func_varint_encode::VARINT_ENCODE,
    func_varint_decode::VARINT_DECODE,
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

    // `resolve_overload_target` is intentionally not overridden: both overloaded
    // names (`utf8Encode`/`utf8Decode`) are now `Implementation::Resolve` and are
    // resolved by their own descriptor selectors before this package resolver.
}
static ENCODING_RESOLVER: EncodingResolver = EncodingResolver;

pub(crate) static ENCODING: BuiltinModule = BuiltinModule {
    name: "encoding",
    doc_intro: "",
    doc_desc: "",
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
        // `UTF8_ENCODE`/`UTF8_DECODE` are `Implementation::Resolve`: their selectors
        // (and the no-context `TYPE_OVERLOAD_AMBIGUOUS` error) live on the descriptor
        // in their `func_*.rs`, reached before this resolver. Only the concrete
        // monomorph-target variants remain here.
        UTF8_ENCODE_BYTES if arg == "String" => Cow::Borrowed(BYTES),
        UTF8_ENCODE_INTS if arg == "String" => Cow::Borrowed(INTS),
        // `UTF8_DECODE` (the overloaded parent) is `Implementation::Resolve`: its
        // return-type resolution is owned by its descriptor selector in
        // `func_utf8_decode.rs`, not this package resolver. Only the concrete
        // monomorph targets remain here.
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

/// The internal `__encoding_*` symbol each non-overloaded public member (and the
/// four `utf8Encode`/`utf8Decode` monomorph targets) rewrites to during IR
/// lowering. These members now carry [`Implementation::Mfb`], whose descriptor
/// `implementation_name` is `None` (the body is assembled into the injected
/// package rather than named by a fixed rewrite symbol), so the rewrite target is
/// provided here explicitly. The two overloaded parents (`utf8Encode`/`utf8Decode`,
/// `Implementation::Resolve`) are absent: they resolve to a concrete variant target
/// via `resolve_overload_target` first, and that target then rewrites through here.
const IMPL_NAMES: &[(&str, &str)] = &[
    ("encoding.utf8EncodeBytes", "__encoding_utf8EncodeBytes"),
    ("encoding.utf8EncodeInts", "__encoding_utf8EncodeInts"),
    ("encoding.utf8DecodeBytes", "__encoding_utf8DecodeBytes"),
    ("encoding.utf8DecodeInts", "__encoding_utf8DecodeInts"),
    ("encoding.utf16Encode", "__encoding_utf16Encode"),
    ("encoding.utf16Decode", "__encoding_utf16Decode"),
    ("encoding.utf32Encode", "__encoding_utf32Encode"),
    ("encoding.utf32Decode", "__encoding_utf32Decode"),
    ("encoding.hexEncode", "__encoding_hexEncode"),
    ("encoding.hexDecode", "__encoding_hexDecode"),
    ("encoding.base32Encode", "__encoding_base32Encode"),
    ("encoding.base32Decode", "__encoding_base32Decode"),
    ("encoding.base64Encode", "__encoding_base64Encode"),
    ("encoding.base64Decode", "__encoding_base64Decode"),
    ("encoding.base64UrlEncode", "__encoding_base64UrlEncode"),
    ("encoding.base64UrlDecode", "__encoding_base64UrlDecode"),
    ("encoding.percentEncode", "__encoding_percentEncode"),
    ("encoding.percentDecode", "__encoding_percentDecode"),
    ("encoding.htmlEscape", "__encoding_htmlEscape"),
    ("encoding.htmlUnescape", "__encoding_htmlUnescape"),
    ("encoding.formUrlEncode", "__encoding_formUrlEncode"),
    ("encoding.formUrlDecode", "__encoding_formUrlDecode"),
    ("encoding.punycodeEncode", "__encoding_punycodeEncode"),
    ("encoding.punycodeDecode", "__encoding_punycodeDecode"),
    ("encoding.uleb128Encode", "__encoding_uleb128Encode"),
    ("encoding.uleb128Decode", "__encoding_uleb128Decode"),
    ("encoding.sleb128Encode", "__encoding_sleb128Encode"),
    ("encoding.sleb128Decode", "__encoding_sleb128Decode"),
    ("encoding.varintEncode", "__encoding_varintEncode"),
    ("encoding.varintDecode", "__encoding_varintDecode"),
];

pub(crate) fn implementation_name(name: &str) -> Option<&'static str> {
    IMPL_NAMES
        .iter()
        .find(|(public, _)| *public == name)
        .map(|(_, internal)| *internal)
}

/// Whether `callee` is one of the overloaded encoding public names: derived from
/// the descriptor. Both `utf8Encode` (return-type overload) and `utf8Decode`
/// (parameter overload) carry `Implementation::Resolve`, with their selector and
/// private variants owned by their own `func_*.rs`.
pub(crate) fn is_overloaded(callee: &str) -> bool {
    ENCODING
        .function(callee)
        .is_some_and(|function| matches!(function.implementation, Implementation::Resolve { .. }))
}

/// Synthetic path label for the injected encoding source. `parse_source_internal`
/// records it as the file path; `AstProject::to_json` filters this sentinel out of
/// `-ast` output. Preserved byte-for-byte from the pre-migration
/// `package_source_glue!` invocation so the injected AST is unchanged.
const SOURCE_LABEL: &str = "<builtin-encoding>";
const SOURCE_DOC: &str = "builtins/encoding.mfb";

/// Parses the built-in `encoding` package source (dual path: the `package.mfb`
/// companion plus every [`Implementation::Mfb`] member's body, spliced in by
/// [`assembled_source`]).
pub(crate) fn source_file() -> Result<crate::ast::AstFile, ()> {
    crate::ast::parse_source_internal(
        std::path::Path::new(SOURCE_LABEL),
        SOURCE_DOC,
        &assembled_source(),
    )
}

/// The `encoding` package source, assembled from the dual path: the external
/// `package.mfb` companion is the base, and every member carrying
/// [`Implementation::Mfb`] contributes its `FUNC __encoding_<name> ... END FUNC`
/// body in place of a one-line `'@@MFB_BODY:<slug>@@` marker at the body's
/// original position. Splicing at the original position keeps every helper's
/// source line numbers unchanged, so the injected AST — and every derived golden —
/// is byte-identical to the pre-migration companion. Mirrors
/// `collections::assembled_source`.
fn assembled_source() -> String {
    let mut source = String::from(include_str!("package.mfb"));
    let mut splice = |slug: &str, body: &str| {
        let marker = format!("'@@MFB_BODY:{slug}@@");
        debug_assert!(
            source.contains(&marker),
            "encoding package.mfb is missing the '{marker}' body marker",
        );
        source = source.replacen(&marker, body, 1);
    };
    for func in ENCODING_FUNCTIONS {
        match func.implementation {
            Implementation::Mfb { body, .. } => splice(func.doc_slug, body),
            // A `Resolve` member's private variants each contribute their own `Mfb`
            // body (`func_utf8_decode.rs`), spliced at their own slug marker.
            Implementation::Resolve { variants, .. } => {
                for variant in variants {
                    if let Implementation::Mfb { body, .. } = variant.implementation {
                        splice(variant.doc_slug, body);
                    }
                }
            }
            _ => {}
        }
    }
    source
}

pub(crate) fn uses_package(ast: &crate::ast::AstProject) -> bool {
    ast.files.iter().any(|file| {
        file.imports
            .iter()
            .any(|import| import.package_name() == "encoding")
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
        // Both overloads' byte/int targets are private `Resolve` variants — not
        // public calls, so they are not recognized as encoding members.
        for n in [
            UTF8_ENCODE_BYTES,
            UTF8_ENCODE_INTS,
            UTF8_DECODE_BYTES,
            UTF8_DECODE_INTS,
        ] {
            assert!(!is_encoding_call(n), "{n}");
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
        assert_eq!(
            target(UTF8_ENCODE, &["String"], Some(BYTES)),
            Ok(Some(UTF8_ENCODE_BYTES.to_string()))
        );
        assert_eq!(
            target(UTF8_ENCODE, &["String"], Some(INTS)),
            Ok(Some(UTF8_ENCODE_INTS.to_string()))
        );
        // no expected type -> Err
        assert_eq!(target(UTF8_ENCODE, &["String"], None), Err(()));
        assert_eq!(target(UTF8_ENCODE, &["String"], Some("String")), Err(()));
        // utf8Encode with wrong arg types is not the overload arm -> Ok(None)
        assert_eq!(target(UTF8_ENCODE, &["Integer"], Some(BYTES)), Ok(None));
        assert_eq!(
            target(UTF8_DECODE, &[BYTES], None),
            Ok(Some(UTF8_DECODE_BYTES.to_string()))
        );
        assert_eq!(
            target(UTF8_DECODE, &[INTS], None),
            Ok(Some(UTF8_DECODE_INTS.to_string()))
        );
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

    #[test]
    fn descriptor_constructors_execute_at_runtime() {
        // `p`/`ov` are const fns used only in const context, so their bodies never
        // run at runtime and show as uncovered. Call them at runtime to exercise
        // (and pin the shape of) each overload/parameter builder.
        let param = p("value", VALTEXT, "String");
        assert_eq!(param.name, "value");
        assert_eq!(param.aliases, VALTEXT);
        assert_eq!(param.ty, ParameterType::Named("String"));
        assert_eq!(param.default, DefaultValue::None);

        // E0716: `ov` borrows `&'static` slices, so they must be named consts.
        const PARAMS: &[Parameter] = &[p("value", &[], "String")];
        let overload = ov(PARAMS, BYTES);
        assert_eq!(overload.params.len(), 1);
        assert_eq!(overload.params[0].name, "value");
        assert_eq!(overload.return_type, ReturnType::Fixed(BYTES));

        // The two overloaded names use `BuiltinFunction::resolve` (selector + inline
        // private variants); every other member uses `::mfb` in its own `func_*.rs`.
        let entry = func_utf8_encode::UTF8_ENCODE;
        assert_eq!(entry.doc_slug, "utf8Encode");
        assert!(matches!(
            entry.implementation,
            Implementation::Resolve { .. }
        ));
        assert!(!entry.doc_intro.is_empty());

        let hex = func_hex_encode::HEX_ENCODE;
        assert_eq!(hex.doc_slug, "hexEncode");
        assert!(matches!(hex.implementation, Implementation::Mfb { .. }));
        assert!(!hex.doc_example.is_empty());
    }

    #[test]
    fn dispatch_resolve_all_branches() {
        // `dispatch_resolve` is reached in production only through the descriptor
        // resolver; call it directly to exercise every return-type arm and the
        // arity guard.
        let ret = |name: &str, args: &[&str]| {
            dispatch_resolve(name, &strings(args)).map(|r| r.return_type.into_owned())
        };
        let resolve = ret;

        // Arity guard: only unary calls resolve.
        assert!(resolve(UTF8_ENCODE, &["String", "String"]).is_none());
        assert!(resolve(UTF8_ENCODE, &[]).is_none());
        // utf8 family. The overloaded parents (`UTF8_ENCODE`/`UTF8_DECODE`) are
        // `Implementation::Resolve`, no longer `dispatch_resolve` arms; only their
        // concrete monomorph targets remain here.
        assert_eq!(ret(UTF8_ENCODE_BYTES, &["String"]).as_deref(), Some(BYTES));
        assert_eq!(ret(UTF8_ENCODE_INTS, &["String"]).as_deref(), Some(INTS));
        // `UTF8_DECODE` (the parent) is `Implementation::Resolve`, no longer a
        // `dispatch_resolve` arm; only its concrete monomorph targets remain here.
        assert_eq!(ret(UTF8_DECODE_BYTES, &[BYTES]).as_deref(), Some("String"));
        assert_eq!(ret(UTF8_DECODE_INTS, &[INTS]).as_deref(), Some("String"));
        // utf16/utf32.
        assert_eq!(ret(UTF16_ENCODE, &["String"]).as_deref(), Some(INTS));
        assert_eq!(ret(UTF32_ENCODE, &["String"]).as_deref(), Some(INTS));
        assert_eq!(ret(UTF16_DECODE, &[INTS]).as_deref(), Some("String"));
        assert_eq!(ret(UTF32_DECODE, &[INTS]).as_deref(), Some("String"));
        // hex/base32/base64 encode -> String, decode -> Bytes.
        assert_eq!(ret(BASE64_ENCODE, &[BYTES]).as_deref(), Some("String"));
        assert_eq!(ret(BASE64URL_DECODE, &["String"]).as_deref(), Some(BYTES));
        // percent/html/formUrl/punycode String -> String.
        assert_eq!(ret(PERCENT_ENCODE, &["String"]).as_deref(), Some("String"));
        assert_eq!(ret(PUNYCODE_DECODE, &["String"]).as_deref(), Some("String"));
        // leb128/varint.
        assert_eq!(ret(VARINT_ENCODE, &["Integer"]).as_deref(), Some(BYTES));
        assert_eq!(ret(VARINT_DECODE, &[BYTES]).as_deref(), Some("Integer"));

        // Wrong argument type falls through to the `_ => None` arm.
        assert!(resolve(UTF8_ENCODE, &["Integer"]).is_none());
        assert!(resolve("encoding.nope", &["String"]).is_none());
    }
}
