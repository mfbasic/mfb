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
    helper_puny_adapt::register(&mut pkg);
    helper_puny_digit::register(&mut pkg);
    helper_puny_value::register(&mut pkg);
    helper_puny_threshold::register(&mut pkg);
    helper_puny_encode_label::register(&mut pkg);
    helper_puny_decode_label::register(&mut pkg);
    helper_label_has_non_ascii::register(&mut pkg);
    // Generated: registers the `Codepage` enum AND `__encoding_codepageTable`,
    // from one `scripts/gen_codepage_tables.py` source of truth.
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
        // 29 public members (2 overloaded + 27 non-overloaded). `codepageDecode`
        // is the 29th (plan-123-A); `codepageEncode` makes it 30 in plan-123-B.
        assert_eq!(pkg.functions().len(), 29);
    }

    #[test]
    fn generic_dispatch_reaches_encoding() {
        assert!(registry().is_member("encoding.hexEncode"));
        assert!(!registry().is_member("encoding.nope"));
        // Fixed-return non-overloaded members have a static nominal return.
        assert_eq!(
            registry::call_return_type_typed("encoding.hexEncode")
                .map(|t| t.name().into_owned())
                .as_deref(),
            Some("String")
        );
    }

    #[test]
    fn non_overloaded_members_rewrite_to_their_internal_symbol() {
        assert_eq!(
            registry::rewrite_target("encoding.hexEncode", &[]),
            Some("__encoding_hexEncode")
        );
        assert_eq!(
            registry::rewrite_target("encoding.varintDecode", &[]),
            Some("__encoding_varintDecode")
        );
        // The overloaded names carry no registry rewrite target: IR lowering leaves
        // the canonical name in place for the monomorphizer.
        assert_eq!(registry::rewrite_target("encoding.utf8Encode", &[]), None);
        assert_eq!(registry::rewrite_target("encoding.utf8Decode", &[]), None);
    }

    #[test]
    fn resolve_call_types_the_members() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(
            registry::resolve_call("encoding.hexEncode", &s(&["List OF Byte"]), false),
            Some("String".to_string())
        );
        assert_eq!(
            registry::resolve_call("encoding.hexDecode", &s(&["String"]), false),
            Some("List OF Byte".to_string())
        );
        // utf8Encode default (pre-monomorph) return is List OF Byte.
        assert_eq!(
            registry::resolve_call("encoding.utf8Encode", &s(&["String"]), false),
            Some("List OF Byte".to_string())
        );
        // utf8Decode is a parameter overload: both element types resolve.
        assert_eq!(
            registry::resolve_call("encoding.utf8Decode", &s(&["List OF Byte"]), false),
            Some("String".to_string())
        );
        assert_eq!(
            registry::resolve_call("encoding.utf8Decode", &s(&["List OF Integer"]), false),
            Some("String".to_string())
        );
        // A wrong argument type resolves to nothing.
        assert_eq!(
            registry::resolve_call("encoding.utf8Encode", &s(&["Integer"]), false),
            None
        );
    }

    #[test]
    fn arity_is_unary_across_members() {
        assert_eq!(registry().arity("encoding.hexEncode"), Some((1, 1)));
        assert_eq!(registry().arity("encoding.utf8Decode"), Some((1, 1)));
    }

    #[test]
    fn diagnostic_metadata_via_registry() {
        // `utf8Decode`'s overloaded phrasing is a descriptor hint; every other
        // member's diagnostic equals the generic per-position render.
        assert_eq!(
            registry::expected_arguments("encoding.utf8Decode"),
            Some("List OF Byte or List OF Integer")
        );
        assert_eq!(
            registry::expected_arguments("encoding.hexEncode"),
            Some("List OF Byte")
        );
        // The overloaded member has no single positional signature; the per-position
        // render of a unary member is its one argument type.
        assert_eq!(
            crate::codegen::builtins::argument_types_typed("encoding.utf8Decode"),
            None
        );
        assert_eq!(
            crate::codegen::builtins::argument_types_typed("encoding.hexEncode"),
            Some(vec![crate::types::ParameterType::list_of(
                crate::types::ParameterType::Byte
            )])
        );
        assert_eq!(
            registry::call_param_names("encoding.utf8Encode"),
            Some(vec![vec!["value", "text"]])
        );
        assert_eq!(
            registry::call_param_names("encoding.punycodeDecode"),
            Some(vec![vec!["asciiDomain"]])
        );
        assert!(registry::expected_arguments("encoding.nope").is_none());
    }

    #[test]
    fn reassembled_source_parses() {
        let source = registry().resolve_package("encoding").unwrap().get_mfb();
        assert!(crate::ast::parse_source_internal(
            std::path::Path::new(SOURCE_LABEL),
            SOURCE_DOC,
            &source,
        )
        .is_ok());
    }

    #[test]
    fn augmented_project_injects_when_imported() {
        let ast = project("IMPORT encoding\nSUB main\nEND SUB\n");
        assert!(registry()
            .resolve_package("encoding")
            .unwrap()
            .is_imported_by(&crate::codegen::registry::ProjectView::of_ast(&ast)));
        assert_eq!(
            augmented_project(&ast).expect("augment").files.len(),
            ast.files.len() + 1
        );
    }

    #[test]
    fn augmented_project_noop_without_import() {
        let ast = project("SUB main\nEND SUB\n");
        assert!(!registry()
            .resolve_package("encoding")
            .unwrap()
            .is_imported_by(&crate::codegen::registry::ProjectView::of_ast(&ast)));
        assert_eq!(
            augmented_project(&ast).expect("augment").files.len(),
            ast.files.len()
        );
    }
}

/// Differential checks of the generated `Codepage` tables against the vendored
/// WHATWG index files (plan-123-A).
///
/// The tables are this feature's whole correctness surface and nobody can eyeball
/// 3,342 mappings, so they are never reviewed by hand: `scripts/gen_codepage_tables.py`
/// derives `helper_codepage_table.rs` from `tools/codepage-index/`, and the tests
/// below re-derive the same answer independently at test time and compare every
/// scalar of every table. A whole table shifted by one fails here; a spot check
/// would not.
#[cfg(test)]
mod codepage_tables {
    use super::helper_codepage_table::{BODY, VARIANTS};
    use crate::codegen::registry::registry;
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::PathBuf;

    /// The scalar a table carries for a byte its codepage leaves unmapped.
    const SENTINEL: u32 = 0xFFFD;

    fn index_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tools")
            .join("codepage-index")
    }

    /// Parse one vendored index file into `pointer -> code point`. The format is
    /// `<pointer>\t0x<hex>\t<char> (<NAME>)`, with `#` comment lines; the trailing
    /// prose column is dropped.
    fn read_index(label: &str) -> BTreeMap<u32, u32> {
        let path = index_dir().join(format!("index-{label}.txt"));
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let mut rows = BTreeMap::new();
        for line in text.lines() {
            let body = line.split('#').next().unwrap_or("").trim();
            if body.is_empty() {
                continue;
            }
            let mut fields = body.split_whitespace();
            let pointer: u32 = fields.next().expect("pointer").parse().expect("pointer");
            let hex = fields.next().expect("code point");
            let cp =
                u32::from_str_radix(hex.strip_prefix("0x").unwrap_or(hex), 16).expect("code point");
            rows.insert(pointer, cp);
        }
        rows
    }

    /// Decode an MFBASIC `\u{XXXX}`-escaped table literal into its scalars. The
    /// generator emits every entry as an escape, so anything else in the literal is
    /// a generator bug.
    fn scalars(literal: &str) -> Vec<u32> {
        let mut out = Vec::new();
        let mut rest = literal;
        while !rest.is_empty() {
            let (hex, tail) = rest
                .strip_prefix("\\u{")
                .and_then(|r| r.split_once('}'))
                .unwrap_or_else(|| panic!("table literal is not all escapes at {rest}"));
            out.push(u32::from_str_radix(hex, 16).expect("escape hex"));
            rest = tail;
        }
        out
    }

    /// `(variant name, table literal)` read out of the generated `MATCH` body, in
    /// declaration order.
    fn arms() -> Vec<(String, String)> {
        let mut out = Vec::new();
        let mut lines = BODY.lines();
        while let Some(line) = lines.next() {
            let Some(name) = line.trim().strip_prefix("CASE Codepage.") else {
                continue;
            };
            let ret = lines.next().expect("a CASE arm has a RETURN").trim();
            let literal = ret
                .strip_prefix("RETURN \"")
                .and_then(|r| r.strip_suffix('"'))
                .unwrap_or_else(|| panic!("CASE {name} does not return a string literal"));
            out.push((name.to_string(), literal.to_string()));
        }
        out
    }

    fn table_of(name: &str) -> String {
        arms()
            .into_iter()
            .find(|(n, _)| n == name)
            .unwrap_or_else(|| panic!("no CASE arm for {name}"))
            .1
    }

    /// Every scalar of every table equals the vendored index file's mapping for that
    /// byte, or the sentinel where the file has no row. This is the plan's real gate:
    /// it re-derives the answer from the upstream data rather than trusting the
    /// generated file, so an off-by-one anywhere in any table fails here.
    #[test]
    fn codepage_tables_match_the_vendored_index_files() {
        let arms = arms();
        assert_eq!(
            arms.len(),
            VARIANTS.len(),
            "every variant needs exactly one MATCH arm"
        );

        let mut checked = 0usize;
        for ((arm_name, literal), (name, label, _desc)) in arms.iter().zip(VARIANTS.iter()) {
            assert_eq!(
                arm_name.as_str(),
                *name,
                "arm order must match variant order"
            );
            if label.is_empty() {
                // `Utf8` has no single-byte table: both callers branch on it before
                // the lookup, so its arm answers with the empty string.
                assert_eq!(literal, "", "{name} has no index file, so no table");
                continue;
            }
            let table = scalars(literal);
            assert_eq!(table.len(), 128, "{name}: a table is 128 scalars");
            let index = read_index(label);
            for (i, got) in table.iter().enumerate() {
                let want = index.get(&(i as u32)).copied().unwrap_or(SENTINEL);
                assert_eq!(
                    *got,
                    want,
                    "{name} (index-{label}.txt) byte 0x{:02X}: table has U+{got:04X}, \
                     the index file has U+{want:04X}",
                    128 + i
                );
                checked += 1;
            }
            // The sentinel must never collide with a real mapping, or an unmapped
            // byte would decode to U+FFFD instead of raising.
            for cp in index.values() {
                assert_ne!(
                    *cp, SENTINEL,
                    "index-{label}.txt maps a byte to the U+FFFD hole sentinel"
                );
            }
        }
        assert_eq!(checked, 28 * 128, "28 single-byte variants x 128 bytes");
    }

    /// No index file is orphaned: every vendored table is reachable from a variant.
    /// Without this, adding a file and forgetting the variant is silent.
    #[test]
    fn every_vendored_index_file_has_a_codepage_variant() {
        let used: BTreeSet<String> = VARIANTS
            .iter()
            .map(|(_n, label, _d)| (*label).to_string())
            .filter(|l| !l.is_empty())
            .collect();
        let mut vendored = BTreeSet::new();
        for entry in std::fs::read_dir(index_dir()).expect("read tools/codepage-index") {
            let name = entry.expect("dir entry").file_name();
            let name = name.to_string_lossy().into_owned();
            if let Some(label) = name
                .strip_prefix("index-")
                .and_then(|n| n.strip_suffix(".txt"))
            {
                vendored.insert(label.to_string());
            }
        }
        assert_eq!(vendored, used, "vendored index files vs. variants in use");
        assert_eq!(vendored.len(), 27, "27 distinct WHATWG single-byte tables");
    }

    /// The registered enum is exactly `VARIANTS`, in order. Variant order fixes the
    /// discriminants and is a compatibility surface, so the pinned positions below
    /// also assert that a later change appends rather than reorders.
    #[test]
    fn codepage_enum_is_registered_in_generator_order() {
        let pkg = registry().resolve_package("encoding").expect("encoding");
        let cp = pkg
            .enums()
            .iter()
            .find(|e| e.name == "Codepage")
            .expect("Codepage enum");
        assert!(cp.export, "Codepage must be visible to importers");
        let names: Vec<&str> = cp.variants.iter().map(|v| v.name).collect();
        let want: Vec<&str> = VARIANTS.iter().map(|(n, _l, _d)| *n).collect();
        assert_eq!(names, want);
        assert_eq!(
            names.len(),
            29,
            "UTF-8 plus the 28 WHATWG single-byte labels"
        );
        assert_eq!(names[0], "Utf8", "discriminant 0 is pinned");
        assert_eq!(names[21], "Windows1252", "discriminant 21 is pinned");
        assert_eq!(names[28], "MacCyrillic", "discriminant 28 is pinned");
        assert!(
            cp.variants.iter().all(|v| !v.description.is_empty()),
            "every variant renders a description in `mfb man encoding types`"
        );
    }

    /// ISO-8859-8-I has no index file of its own (upstream serves HTTP 404 for it)
    /// and shares ISO-8859-8's mapping — the two differ only in display direction.
    /// Pinned so a later edit neither invents a table for it nor drops the variant.
    #[test]
    fn iso_8859_8_i_shares_the_iso_8859_8_table() {
        assert_eq!(table_of("Iso8859_8I"), table_of("Iso8859_8"));
        assert!(!index_dir().join("index-iso-8859-8-i.txt").exists());
    }

    /// The rt fixture's per-codepage digest lines, recomputed here from the vendored
    /// index files.
    ///
    /// `func_encoding_codepageDecode_rt` walks all 128 high bytes of every
    /// single-byte codepage and prints `<name> mapped=<n> sum=<s>`, where `sum` is
    /// **position-weighted** — byte `b` contributes `(b - 127) * code point`. That
    /// weight is what makes the line differential: a table shifted by one byte keeps
    /// the same code points and the same mapped count but cannot keep the same sum.
    /// Recomputing every line here is what stops the golden from being blessed wrong;
    /// without it the fixture would only pin whatever the compiler happened to do.
    #[test]
    fn codepage_digests_match_the_vendored_index_files() {
        let log = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/rt-behavior/encoding/func_encoding_codepageDecode_rt/golden/build.log");
        let text =
            std::fs::read_to_string(&log).unwrap_or_else(|e| panic!("read {}: {e}", log.display()));

        let mut got: BTreeMap<String, (usize, u64)> = BTreeMap::new();
        for line in text.lines() {
            let Some((name, rest)) = line.split_once(" mapped=") else {
                continue;
            };
            let (mapped, sum) = rest.split_once(" sum=").expect("digest line shape");
            got.insert(
                name.to_string(),
                (
                    mapped.parse().expect("mapped count"),
                    sum.parse().expect("weighted sum"),
                ),
            );
        }

        let mut want: BTreeMap<String, (usize, u64)> = BTreeMap::new();
        for (name, label, _desc) in VARIANTS {
            if label.is_empty() {
                // `Utf8` has no per-byte table, so the fixture does not digest it.
                continue;
            }
            let index = read_index(label);
            let sum: u64 = index
                .iter()
                .map(|(pointer, cp)| u64::from(pointer + 1) * u64::from(*cp))
                .sum();
            want.insert((*name).to_string(), (index.len(), sum));
        }

        assert_eq!(got, want, "rt fixture digests vs. the vendored index files");
        assert_eq!(got.len(), 28, "one digest line per single-byte codepage");
    }
}
