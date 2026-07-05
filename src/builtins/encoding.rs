use std::borrow::Cow;
use std::path::Path;

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

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

pub(crate) fn is_encoding_call(name: &str) -> bool {
    matches!(
        name,
        UTF8_ENCODE
            | UTF8_DECODE
            | UTF16_ENCODE
            | UTF16_DECODE
            | UTF32_ENCODE
            | UTF32_DECODE
            | HEX_ENCODE
            | HEX_DECODE
            | BASE32_ENCODE
            | BASE32_DECODE
            | BASE64_ENCODE
            | BASE64_DECODE
            | BASE64URL_ENCODE
            | BASE64URL_DECODE
            | PERCENT_ENCODE
            | PERCENT_DECODE
            | HTML_ESCAPE
            | HTML_UNESCAPE
            | FORM_URL_ENCODE
            | FORM_URL_DECODE
            | PUNYCODE_ENCODE
            | PUNYCODE_DECODE
            | ULEB128_ENCODE
            | ULEB128_DECODE
            | SLEB128_ENCODE
            | SLEB128_DECODE
            | VARINT_ENCODE
            | VARINT_DECODE
            | UTF8_ENCODE_BYTES
            | UTF8_ENCODE_INTS
            | UTF8_DECODE_BYTES
            | UTF8_DECODE_INTS
    )
}

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
    match name {
        // utf8Encode is a return-type overload; report its default (List OF Byte)
        // form so the call is recognized. The precise type is resolved with the
        // expected (contextual) type in the checker and monomorphizer.
        UTF8_ENCODE | UTF8_ENCODE_BYTES | HEX_DECODE | BASE32_DECODE | BASE64_DECODE
        | BASE64URL_DECODE | ULEB128_ENCODE | SLEB128_ENCODE | VARINT_ENCODE => Some(BYTES),
        UTF16_ENCODE | UTF32_ENCODE | UTF8_ENCODE_INTS => Some(INTS),
        UTF8_DECODE | UTF8_DECODE_BYTES | UTF8_DECODE_INTS | UTF16_DECODE | UTF32_DECODE
        | HEX_ENCODE | BASE32_ENCODE | BASE64_ENCODE | BASE64URL_ENCODE | PERCENT_ENCODE
        | PERCENT_DECODE | HTML_ESCAPE | HTML_UNESCAPE | FORM_URL_ENCODE | FORM_URL_DECODE
        | PUNYCODE_ENCODE | PUNYCODE_DECODE => Some("String"),
        ULEB128_DECODE | SLEB128_DECODE | VARINT_DECODE => Some("Integer"),
        _ => None,
    }
}

pub(crate) fn arity(name: &str) -> Option<(usize, usize)> {
    is_encoding_call(name).then_some((1, 1))
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

pub(crate) fn resolve_call<'a>(name: &str, arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
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
    match name {
        UTF8_ENCODE_BYTES => Some("__encoding_utf8EncodeBytes"),
        UTF8_ENCODE_INTS => Some("__encoding_utf8EncodeInts"),
        UTF8_DECODE_BYTES => Some("__encoding_utf8DecodeBytes"),
        UTF8_DECODE_INTS => Some("__encoding_utf8DecodeInts"),
        UTF16_ENCODE => Some("__encoding_utf16Encode"),
        UTF16_DECODE => Some("__encoding_utf16Decode"),
        UTF32_ENCODE => Some("__encoding_utf32Encode"),
        UTF32_DECODE => Some("__encoding_utf32Decode"),
        HEX_ENCODE => Some("__encoding_hexEncode"),
        HEX_DECODE => Some("__encoding_hexDecode"),
        BASE32_ENCODE => Some("__encoding_base32Encode"),
        BASE32_DECODE => Some("__encoding_base32Decode"),
        BASE64_ENCODE => Some("__encoding_base64Encode"),
        BASE64_DECODE => Some("__encoding_base64Decode"),
        BASE64URL_ENCODE => Some("__encoding_base64UrlEncode"),
        BASE64URL_DECODE => Some("__encoding_base64UrlDecode"),
        PERCENT_ENCODE => Some("__encoding_percentEncode"),
        PERCENT_DECODE => Some("__encoding_percentDecode"),
        HTML_ESCAPE => Some("__encoding_htmlEscape"),
        HTML_UNESCAPE => Some("__encoding_htmlUnescape"),
        FORM_URL_ENCODE => Some("__encoding_formUrlEncode"),
        FORM_URL_DECODE => Some("__encoding_formUrlDecode"),
        PUNYCODE_ENCODE => Some("__encoding_punycodeEncode"),
        PUNYCODE_DECODE => Some("__encoding_punycodeDecode"),
        ULEB128_ENCODE => Some("__encoding_uleb128Encode"),
        ULEB128_DECODE => Some("__encoding_uleb128Decode"),
        SLEB128_ENCODE => Some("__encoding_sleb128Encode"),
        SLEB128_DECODE => Some("__encoding_sleb128Decode"),
        VARINT_ENCODE => Some("__encoding_varintEncode"),
        VARINT_DECODE => Some("__encoding_varintDecode"),
        _ => None,
    }
}

/// Resolve the overloaded `utf8Encode`/`utf8Decode` public calls to a concrete
/// internal implementation, using the call's argument types and the expected
/// (contextual) type. Returns `Ok(Some(name))` on a unique match, `Ok(None)`
/// when the callee is not an overloaded encoding name, and `Err(())` when a
/// return-type overload cannot be resolved without an expected type
/// (`utf8Encode` with no `List OF Byte`/`List OF Integer` context).
pub(crate) fn resolve_overload_target(
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

/// Whether `callee` is one of the overloaded encoding public names handled by
/// `resolve_overload_target` (rather than the flat `implementation_name` map).
pub(crate) fn is_overloaded(callee: &str) -> bool {
    matches!(callee, UTF8_ENCODE | UTF8_DECODE)
}

pub(crate) fn source_file() -> Result<crate::ast::AstFile, ()> {
    crate::ast::parse_source_internal(
        Path::new("<builtin-encoding>"),
        "builtins/encoding.mfb",
        include_str!("encoding_package.mfb"),
    )
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
        assert_eq!(
            resolve_overload_target(UTF8_ENCODE, &strings(&["String"]), Some(BYTES)),
            Ok(Some(UTF8_ENCODE_BYTES))
        );
        assert_eq!(
            resolve_overload_target(UTF8_ENCODE, &strings(&["String"]), Some(INTS)),
            Ok(Some(UTF8_ENCODE_INTS))
        );
        // no expected type -> Err
        assert_eq!(
            resolve_overload_target(UTF8_ENCODE, &strings(&["String"]), None),
            Err(())
        );
        assert_eq!(
            resolve_overload_target(UTF8_ENCODE, &strings(&["String"]), Some("String")),
            Err(())
        );
        // utf8Encode with wrong arg types is not the overload arm -> Ok(None)
        assert_eq!(
            resolve_overload_target(UTF8_ENCODE, &strings(&["Integer"]), Some(BYTES)),
            Ok(None)
        );
        assert_eq!(
            resolve_overload_target(UTF8_DECODE, &strings(&[BYTES]), None),
            Ok(Some(UTF8_DECODE_BYTES))
        );
        assert_eq!(
            resolve_overload_target(UTF8_DECODE, &strings(&[INTS]), None),
            Ok(Some(UTF8_DECODE_INTS))
        );
        // non-overloaded callee -> Ok(None)
        assert_eq!(
            resolve_overload_target(HEX_ENCODE, &strings(&[BYTES]), None),
            Ok(None)
        );
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
}
