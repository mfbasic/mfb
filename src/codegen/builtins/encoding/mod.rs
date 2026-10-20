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
//! `__encoding_utf8Decode` bodies live in `helper_utf8_encode.rs` /
//! `helper_utf8_decode.rs`, each registered as a shared
//! [`add_helper`](crate::codegen::registry::RegistryPackage::add_helper) chunk —
//! one `helper_*.rs` file per helper, like csv/json/regex.
//!
//! Injection is on the identical generic path as csv/json/regex: the package
//! registers its `IMPORT`s ([`add_imports`](crate::codegen::registry::RegistryPackage::add_imports)),
//! its shared helpers ([`add_helper`](crate::codegen::registry::RegistryPackage::add_helper)),
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
//! Argument-mismatch metadata routes entirely through the generic registry: every
//! member's per-position render matches its old phrasing, and `utf8Decode`'s bespoke
//! union phrasing `"List OF Byte or List OF Integer"` (which also makes it decline a
//! single positional signature) rides on its
//! [`RegistryFunction::expected_arguments`](crate::codegen::registry::RegistryFunction)
//! descriptor field — no per-package `expected_arguments`/`argument_types`/
//! `call_param_names` seam remains.

// --- codegen tier imports (migration) ---
use crate::codegen::registry::{Registry, RegistryPackage};
mod func_base32_decode;
mod func_base32_encode;
mod func_base64_decode;
mod func_base64_encode;
mod func_base64_url_decode;
mod func_base64_url_encode;
mod func_codepage_decode;
mod func_codepage_encode;
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

mod helper_base32_value;
mod helper_base64_symbols;
mod helper_base64_value;
mod helper_base_decode_bits;
mod helper_base_encode;
mod helper_byte_char;
mod helper_codepage_table;
mod helper_codepoints;
mod helper_from_codepoint;
mod helper_hex_digit;
mod helper_hex_value;
mod helper_html_entity;
mod helper_is_alpha_num;
mod helper_is_unreserved;
mod helper_label_has_non_ascii;
mod helper_leb128_emit;
mod helper_leb128_read;
mod helper_low_bits;
mod helper_parse_decimal;
mod helper_parse_hex;
mod helper_percent_byte;
mod helper_percent_decode_bytes;
mod helper_puny_adapt;
mod helper_puny_decode_label;
mod helper_puny_digit;
mod helper_puny_encode_label;
mod helper_puny_threshold;
mod helper_puny_value;
mod helper_utf8_decode;
mod helper_utf8_encode;
mod helper_utf8_valid;

const INTRO: &str = r#"Byte<->text and Unicode codecs: UTF-8/16/32, hex, base32/64, percent, HTML, form-url, punycode, LEB128/varint, and the legacy single-byte codepages."#;

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

The codepage codecs (`codepageDecode`/`codepageEncode`) move between a
`List OF Byte` and a `String` in one of the `encoding::Codepage` members — the
legacy single-byte codepages of the WHATWG Encoding Standard, plus
`Codepage.Utf8`. They are how you read and write content that is not UTF-8: a
`windows-1252` page body, a `KOI8-R` mail part, an `IBM866` DOS file. A byte a
codepage leaves undefined, or a character it has no byte for, is rejected rather
than replaced, so a conversion either covers the whole input or fails.

Decoders reject malformed input with `ErrInvalidFormat` (`77050003`): an
invalid character, a bad length, or a misplaced pad. One thing they do **not**
reject is a non-canonical final group — `encoding::base64Decode("AB==")` and
`encoding::base32Decode("AB======")` each return a single `0` byte rather than
raising, because the unused trailing bits are ignored rather than required to be
zero. Do not use a decode round-trip as a canonical-form check."#;

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

    // The shared private `__encoding_*` helpers the member bodies call (including the
    // four overloaded `__encoding_utf8Encode`/`utf8Decode` bodies). Each lives in its
    // own `helper_*.rs` and registers via `add_helper`; order preserved from the old
    // `package.mfb` blob so the compiled `.ncode` stays byte-identical.
    helper_byte_char::register(&mut pkg);
    helper_low_bits::register(&mut pkg);
    helper_from_codepoint::register(&mut pkg);
    helper_utf8_valid::register(&mut pkg);
    helper_codepoints::register(&mut pkg);
    helper_utf8_encode::register(&mut pkg);
    helper_utf8_decode::register(&mut pkg);
    helper_hex_digit::register(&mut pkg);
    helper_hex_value::register(&mut pkg);
    helper_base_encode::register(&mut pkg);
    helper_base_decode_bits::register(&mut pkg);
    helper_base64_value::register(&mut pkg);
    helper_base32_value::register(&mut pkg);
    helper_base64_symbols::register(&mut pkg);
    helper_is_unreserved::register(&mut pkg);
    helper_is_alpha_num::register(&mut pkg);
    helper_percent_byte::register(&mut pkg);
    helper_percent_decode_bytes::register(&mut pkg);
    helper_html_entity::register(&mut pkg);
    helper_parse_decimal::register(&mut pkg);
    helper_parse_hex::register(&mut pkg);
    helper_leb128_emit::register(&mut pkg);
    helper_leb128_read::register(&mut pkg);
    helper_puny_adapt::register(&mut pkg);
    helper_puny_digit::register(&mut pkg);
    helper_puny_value::register(&mut pkg);
    helper_puny_threshold::register(&mut pkg);
    helper_puny_encode_label::register(&mut pkg);
    helper_puny_decode_label::register(&mut pkg);
    helper_label_has_non_ascii::register(&mut pkg);
    // Generated: registers the `Codepage` enum AND `__encoding_codepageTable`,
    // from one `tools/codepage-index/gen_codepage_tables.py` source of truth.
    helper_codepage_table::register(&mut pkg);

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
    func_codepage_decode::register(&mut pkg);
    func_codepage_encode::register(&mut pkg);

    r.add_package(pkg);
}

/// Synthetic path/doc labels for the injected encoding source (preserved from the
/// pre-migration `package_source_glue!` invocation so the injected AST is
/// unchanged). `parse_source_internal` records the path; `AstProject::to_json`
/// filters this sentinel out of `-ast` output.
const SOURCE_LABEL: &str = "<builtin-encoding>";
const SOURCE_DOC: &str = "builtins/encoding.mfb";

/// Inject the `encoding` package source when a program (or another injected
/// built-in) imports it. `encoding` is a transitive dependency of the non-migrated
/// `crypto`/`strings` packages, so it is injected by this dedicated late pass —
/// after those packages contribute their own `IMPORT encoding` — rather than by the
/// generic `registry::augment_project`, which examines only the pre-injection AST.
/// The generic pass therefore skips `encoding` (see `Registry::augment_project`).
/// The injected *source* is the generic [`RegistryPackage::get_mfb`] assembly
/// (imports → shared helpers → member bodies), identical to what the generic pass
/// would produce for csv/json/regex; only the injection *position* is bespoke.
/// #[deprecated(note = "migrate registry().augment_project once crypto/strings move")]
pub(crate) fn augmented_project(
    ast: &crate::ast::AstProject,
) -> Result<crate::ast::AstProject, ()> {
    crate::codegen::registry::inject_late_pass(ast, "encoding", SOURCE_LABEL, SOURCE_DOC)
}

/// The same injection onto the elaborated project the former source checker consumes
/// (plan-106-D).
#[cfg(test)] // the HIR-domain chain serves the in-process tests only (plan-107-D)
pub(crate) fn augmented_hir_project(
    hir: &crate::hir::HirProject,
) -> Result<crate::hir::HirProject, ()> {
    crate::codegen::registry::inject_late_pass_hir(hir, "encoding", SOURCE_LABEL, SOURCE_DOC)
}

#[cfg(test)]
mod tests;

/// Differential checks of the generated `Codepage` tables against the vendored
/// WHATWG index files (plan-123-A).
///
/// The tables are this feature's whole correctness surface and nobody can eyeball
/// 3,342 mappings, so they are never reviewed by hand: `tools/codepage-index/gen_codepage_tables.py`
/// derives `helper_codepage_table.rs` from `tools/codepage-index/`, and the tests
/// below re-derive the same answer independently at test time and compare every
/// scalar of every table. A whole table shifted by one fails here; a spot check
/// would not.
#[cfg(test)]
#[path = "tests/codepage_tables.rs"]
mod codepage_tables;
