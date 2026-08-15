//! Package: encoding
//! Type: Pure MFBasic (byte<->text and Unicode codecs)
//!
//! Migrated onto the clean-room registry (`crate::codegen::registry`). Every
//! non-overloaded member is unary with a fixed return and carries a
//! [`Body::mfb`](crate::codegen::registry::Body) source body (its
//! `FUNC __encoding_<name>` lives in the owning `func_*.rs`); the rewrite target is
//! the body-declared internal symbol, so IR lowering internalizes the public call
//! via the generic `registry::rewrite_target`.
//!
//! The two overloaded names — `utf8Encode` (return-type overload:
//! `String -> List OF Byte | List OF Integer`, chosen by the call-site expected
//! type) and `utf8Decode` (parameter overload:
//! `List OF Byte | List OF Integer -> String`) — carry
//! [`Body::Intrinsic`](crate::codegen::registry::Body): they have **no** registry
//! rewrite target, so IR lowering leaves the canonical `encoding.utf8Encode` /
//! `encoding.utf8Decode` name in place and the monomorphizer resolves + mangles the
//! selected overload to its private `#encoding_utf8Encode`/`#encoding_utf8Decode`
//! implementation (see `monomorph::lower`). Those four `__encoding_utf8Encode` /
//! `__encoding_utf8Decode` bodies live directly in `package.mfb`, which is
//! registered as a shared [`add_helper_functions`](crate::codegen::registry::RegistryPackage::add_helper_functions)
//! chunk (like csv's `package.mfb`).
//!
//! Injection is on the identical generic path as csv/json/regex: the package
//! registers its `IMPORT`s ([`add_imports`](crate::codegen::registry::RegistryPackage::add_imports)),
//! its shared helpers ([`add_helper_functions`](crate::codegen::registry::RegistryPackage::add_helper_functions)),
//! and each member's [`Body::Mfb`](crate::codegen::registry::Body) body, so
//! [`RegistryPackage::get_mfb`](crate::codegen::registry::RegistryPackage::get_mfb)
//! assembles the injected source in the generic
//! imports→records→unions→helpers→member-bodies order. The old byte-exact
//! `assembled_source` splice (which reinserted each member body at a
//! `'@@MFB_BODY:<name>@@` marker to preserve the pre-migration line numbers) is
//! gone. The only thing that stays bespoke is the injection *position*: because
//! `encoding` is a transitive dependency of the still-native `crypto`/`strings`
//! packages — whose companion source (`crypto_hash.mfb`, `strings_package.mfb`)
//! carries `IMPORT encoding` and calls `encoding::hexDecode`/`utf32Encode` — and
//! those are injected *after* the generic `registry::augment_project` pass, the
//! encoding source must be injected by a dedicated late pass ([`augmented_project`])
//! that runs after them, so the generic pass deliberately skips `encoding` (see
//! `Registry::augment_project`). The assembled source itself is now identical to
//! what the generic pass would produce.
//!
//! `expected_arguments`/`argument_types`/`call_param_names` are retained: the
//! generic registry renders one type per position, but `utf8Decode`'s
//! argument-mismatch diagnostic needs the bespoke union phrasing
//! `"List OF Byte or List OF Integer"` and `utf8Decode` has no single positional
//! signature.

use crate::codegen::registry::{registry, Registry, RegistryPackage};

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

// The qualified public names, shared by the retained diagnostic helpers below.
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

const BYTES: &str = "List OF Byte";
const INTS: &str = "List OF Integer";

const INTRO: &str = r#"Byte<->text and Unicode codecs: UTF-8/16/32, hex, base32/64, percent, HTML, form-url, punycode, and LEB128/varint."#;

const DESC: &str = r#"The `encoding` package converts between text and its various byte and code-unit
serializations. It is a built-in package: `IMPORT encoding` needs no manifest
dependency.

The Unicode codecs (`utf8Encode`/`utf8Decode`, `utf16Encode`/`utf16Decode`,
`utf32Encode`/`utf32Decode`) move between a `String` and its code units or bytes.
The binary codecs (`hexEncode`/`hexDecode`, `base32Encode`/`base32Decode`,
`base64Encode`/`base64Decode`, `base64UrlEncode`/`base64UrlDecode`) move between a
`List OF Byte` and its textual representation. The web codecs
(`percentEncode`/`percentDecode`, `htmlEscape`/`htmlUnescape`,
`formUrlEncode`/`formUrlDecode`, `punycodeEncode`/`punycodeDecode`) and the
integer codecs (`uleb128Encode`/`uleb128Decode`, `sleb128Encode`/`sleb128Decode`,
`varintEncode`/`varintDecode`) round-trip their respective forms.

Decoders reject malformed input with `ErrInvalidFormat` (`77050003`)."#;

/// Register the `encoding` package on the clean-room registry.
///
/// `encoding` injects through the identical generic path as csv/json/regex: it
/// registers its `IMPORT`s and its shared `__encoding_*` helper chunk
/// (`package.mfb`, which also holds the four overloaded
/// `__encoding_utf8Encode`/`utf8Decode` bodies), and each member carries its own
/// `Body::Mfb` body, so [`RegistryPackage::get_mfb`] assembles the injected source
/// in the generic imports→helpers→member-bodies order. It is NOT injected by the
/// generic `registry::augment_project` pass only because it is a transitive
/// dependency of the still-native `crypto`/`strings` packages (injected *after*
/// that pass); its own late [`augmented_project`] handles that (see the module docs
/// and `Registry::augment_project`).
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("encoding", INTRO, DESC);

    // Injected `IMPORT`s and shared helpers, rendered by `get_mfb` before the member
    // bodies — mirroring `package.mfb`'s original leading `IMPORT`s and `__encoding_*`
    // helper block (which also declares the four overloaded utf8 bodies).
    pkg.add_imports(vec!["bits", "strings", "collections"]);
    pkg.add_helper_functions(vec![include_str!("package.mfb")]);

    // The two overloaded names first, then the non-overloaded codecs, mirroring the
    // pre-migration descriptor order.
    func_utf8_encode::register(&mut pkg);
    func_utf8_decode::register(&mut pkg);
    func_utf16_encode::register(&mut pkg);
    func_utf16_decode::register(&mut pkg);
    func_utf32_encode::register(&mut pkg);
    func_utf32_decode::register(&mut pkg);
    func_hex_encode::register(&mut pkg);
    func_hex_decode::register(&mut pkg);
    func_base32_encode::register(&mut pkg);
    func_base32_decode::register(&mut pkg);
    func_base64_encode::register(&mut pkg);
    func_base64_decode::register(&mut pkg);
    func_base64_url_encode::register(&mut pkg);
    func_base64_url_decode::register(&mut pkg);
    func_percent_encode::register(&mut pkg);
    func_percent_decode::register(&mut pkg);
    func_html_escape::register(&mut pkg);
    func_html_unescape::register(&mut pkg);
    func_form_url_encode::register(&mut pkg);
    func_form_url_decode::register(&mut pkg);
    func_punycode_encode::register(&mut pkg);
    func_punycode_decode::register(&mut pkg);
    func_uleb128_encode::register(&mut pkg);
    func_uleb128_decode::register(&mut pkg);
    func_sleb128_encode::register(&mut pkg);
    func_sleb128_decode::register(&mut pkg);
    func_varint_encode::register(&mut pkg);
    func_varint_decode::register(&mut pkg);

    r.add_package(pkg);
}

// The human-readable expected-argument rendering for an argument-mismatch
// diagnostic. Retained (not served by the generic registry) because `utf8Decode`
// needs the union phrasing `"List OF Byte or List OF Integer"` the per-position
// renderer cannot produce; every other member's string equals the generic render,
// but keeping the whole table here pins the diagnostic byte-for-byte.
pub(crate) fn expected_arguments(name: &str) -> Option<&'static str> {
    match name {
        UTF8_ENCODE | UTF16_ENCODE | UTF32_ENCODE | PERCENT_ENCODE | PERCENT_DECODE
        | HTML_ESCAPE | HTML_UNESCAPE | FORM_URL_ENCODE | FORM_URL_DECODE | PUNYCODE_ENCODE
        | PUNYCODE_DECODE | HEX_DECODE | BASE32_DECODE | BASE64_DECODE | BASE64URL_DECODE => {
            Some("String")
        }
        UTF8_DECODE => Some("List OF Byte or List OF Integer"),
        UTF16_DECODE | UTF32_DECODE => Some(INTS),
        HEX_ENCODE | BASE32_ENCODE | BASE64_ENCODE | BASE64URL_ENCODE | ULEB128_DECODE
        | SLEB128_DECODE | VARINT_DECODE => Some(BYTES),
        ULEB128_ENCODE | SLEB128_ENCODE | VARINT_ENCODE => Some("Integer"),
        _ => None,
    }
}

/// The machine-readable positional argument-type signature IR lowering reads for
/// literal coercion (bug-340 A1). Every member is unary, so each entry is a
/// one-element slice — except `utf8Decode`, which is overloaded on
/// `List OF Byte | List OF Integer` and so has no single positional signature
/// (`None`). Retained because the generic registry would hand back the first
/// overload's `List OF Byte` for `utf8Decode`, wrongly coercing a
/// `List OF Integer` literal.
pub(crate) fn argument_types(name: &str) -> Option<&'static [&'static str]> {
    match name {
        UTF8_ENCODE | UTF16_ENCODE | UTF32_ENCODE | PERCENT_ENCODE | PERCENT_DECODE
        | HTML_ESCAPE | HTML_UNESCAPE | FORM_URL_ENCODE | FORM_URL_DECODE | PUNYCODE_ENCODE
        | PUNYCODE_DECODE | HEX_DECODE | BASE32_DECODE | BASE64_DECODE | BASE64URL_DECODE => {
            Some(&["String"])
        }
        UTF8_DECODE => None,
        UTF16_DECODE | UTF32_DECODE => Some(&[INTS]),
        HEX_ENCODE | BASE32_ENCODE | BASE64_ENCODE | BASE64URL_ENCODE | ULEB128_DECODE
        | SLEB128_DECODE | VARINT_DECODE => Some(&[BYTES]),
        ULEB128_ENCODE | SLEB128_ENCODE | VARINT_ENCODE => Some(&["Integer"]),
        _ => None,
    }
}

/// The per-position `[name, alias…]` keyword-matching lists. The generic registry
/// answers these identically (the aliases are mirrored onto each `Parameter`); this
/// is retained as the provenance anchor the man pages cite and as a redundant
/// fallback (the aggregate consults the registry first).
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

/// Synthetic path/doc labels for the injected encoding source (preserved from the
/// pre-migration `package_source_glue!` invocation so the injected AST is
/// unchanged). `parse_source_internal` records the path; `AstProject::to_json`
/// filters this sentinel out of `-ast` output.
const SOURCE_LABEL: &str = "<builtin-encoding>";
const SOURCE_DOC: &str = "builtins/encoding.mfb";

/// Whether `ast` imports `encoding` (directly or via another built-in's injected
/// `IMPORT encoding`).
pub(crate) fn uses_package(ast: &crate::ast::AstProject) -> bool {
    registry()
        .resolve_package("encoding")
        .is_some_and(|pkg| pkg.is_imported_by(ast))
}

/// Parse the built-in `encoding` package source — the generic
/// [`RegistryPackage::get_mfb`] assembly (imports → shared helpers → member
/// bodies), identical to the mechanism csv/json/regex use. The synthetic path/doc
/// labels match the generic pass's convention (`<builtin-encoding>` /
/// `builtins/encoding.mfb`), so the injected file is indistinguishable from one the
/// generic `registry::augment_project` would have produced.
fn source_file() -> Result<crate::ast::AstFile, ()> {
    let source = registry()
        .resolve_package("encoding")
        .expect("encoding package is registered")
        .get_mfb();
    crate::ast::parse_source_internal(std::path::Path::new(SOURCE_LABEL), SOURCE_DOC, &source)
}

/// Inject the `encoding` package source when a program (or another injected
/// built-in) imports it. `encoding` is a transitive dependency of the non-migrated
/// `crypto`/`strings` packages, so it is injected by this dedicated late pass —
/// after those packages contribute their own `IMPORT encoding` — rather than by the
/// generic `registry::augment_project`, which examines only the pre-injection AST.
/// The generic pass therefore skips `encoding` (see `Registry::augment_project`).
/// The injected *source* is the generic [`RegistryPackage::get_mfb`] assembly; only
/// the injection *position* (this late pass) is bespoke.
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
    use crate::codegen::registry::{self, registry};

    fn project(src: &str) -> crate::ast::AstProject {
        let file = crate::ast::parse_source(std::path::Path::new("main.mfb"), "main.mfb", src)
            .expect("parse source");
        crate::ast::AstProject {
            name: "test".to_string(),
            files: vec![file],
        }
    }

    #[test]
    fn encoding_registered_on_the_clean_room_registry() {
        let pkg = registry()
            .resolve_package("encoding")
            .expect("encoding package");
        // 28 public members (2 overloaded + 26 non-overloaded).
        assert_eq!(pkg.functions().len(), 28);
    }

    #[test]
    fn generic_dispatch_reaches_encoding() {
        assert!(registry::is_member("encoding.hexEncode"));
        assert!(!registry::is_member("encoding.nope"));
        // Fixed-return non-overloaded members have a static nominal return.
        assert_eq!(
            registry::call_return_type("encoding.hexEncode"),
            Some("String")
        );
    }

    #[test]
    fn non_overloaded_members_rewrite_to_their_internal_symbol() {
        assert_eq!(
            registry::rewrite_target("encoding.hexEncode"),
            Some("__encoding_hexEncode")
        );
        assert_eq!(
            registry::rewrite_target("encoding.varintDecode"),
            Some("__encoding_varintDecode")
        );
        // The overloaded names carry no registry rewrite target: IR lowering leaves
        // the canonical name in place for the monomorphizer.
        assert_eq!(registry::rewrite_target("encoding.utf8Encode"), None);
        assert_eq!(registry::rewrite_target("encoding.utf8Decode"), None);
    }

    #[test]
    fn resolve_call_types_the_members() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(
            registry::resolve_call("encoding.hexEncode", &s(&["List OF Byte"])),
            Some("String".to_string())
        );
        assert_eq!(
            registry::resolve_call("encoding.hexDecode", &s(&["String"])),
            Some("List OF Byte".to_string())
        );
        // utf8Encode default (pre-monomorph) return is List OF Byte.
        assert_eq!(
            registry::resolve_call("encoding.utf8Encode", &s(&["String"])),
            Some("List OF Byte".to_string())
        );
        // utf8Decode is a parameter overload: both element types resolve.
        assert_eq!(
            registry::resolve_call("encoding.utf8Decode", &s(&["List OF Byte"])),
            Some("String".to_string())
        );
        assert_eq!(
            registry::resolve_call("encoding.utf8Decode", &s(&["List OF Integer"])),
            Some("String".to_string())
        );
        // A wrong argument type resolves to nothing.
        assert_eq!(
            registry::resolve_call("encoding.utf8Encode", &s(&["Integer"])),
            None
        );
    }

    #[test]
    fn arity_is_unary_across_members() {
        assert_eq!(registry::arity("encoding.hexEncode"), Some((1, 1)));
        assert_eq!(registry::arity("encoding.utf8Decode"), Some((1, 1)));
    }

    #[test]
    fn retained_diagnostic_helpers() {
        assert_eq!(
            expected_arguments(UTF8_DECODE),
            Some("List OF Byte or List OF Integer")
        );
        assert_eq!(expected_arguments(HEX_ENCODE), Some(BYTES));
        assert_eq!(argument_types(UTF8_DECODE), None);
        assert_eq!(argument_types(HEX_ENCODE), Some(&[BYTES][..]));
        assert_eq!(
            call_param_names(UTF8_ENCODE),
            Some(&[&["value", "text"][..]][..])
        );
        assert_eq!(
            call_param_names(PUNYCODE_DECODE),
            Some(&[&["asciiDomain"][..]][..])
        );
        assert!(expected_arguments("encoding.nope").is_none());
    }

    #[test]
    fn reassembled_source_parses() {
        assert!(source_file().is_ok());
    }

    #[test]
    fn augmented_project_injects_when_imported() {
        let ast = project("IMPORT encoding\nSUB main\nEND SUB\n");
        assert!(uses_package(&ast));
        assert_eq!(
            augmented_project(&ast).expect("augment").files.len(),
            ast.files.len() + 1
        );
    }

    #[test]
    fn augmented_project_noop_without_import() {
        let ast = project("SUB main\nEND SUB\n");
        assert!(!uses_package(&ast));
        assert_eq!(
            augmented_project(&ast).expect("augment").files.len(),
            ast.files.len()
        );
    }
}
