//! The built-in `strings` package (clean-room registry migration, plan-99 PART B).
//!
//! `strings` is a large, mostly-**native** package: 29 members
//! (`trim`/`upper`/`split`/`join`/`padLeft`/…) lower through the shared string
//! codegen carrier in `src/codegen/builtins/strings/builder_strings*` (kept in place like
//! `vector`'s SIMD carrier), reached by the registry's `Body::Native` `common`
//! slot; three (`find`/`mid`/`replace`) are `Body::Intrinsic`, sharing their bare
//! native lowering with the `collections::` `List` overloads through
//! `builtins::native_builtin_target`; and seven — the Unicode scalar seam
//! (`toScalars`/`fromScalars`) and the five classification predicates
//! (`isLetter`/`isDigit`/`isWhitespace`/`isUpper`/`isLower`) — are `Body::Rewrite`s
//! into the injected source companion (`seam.mfb`).
//!
//! The companion carries the heavy Unicode general-category table
//! (`__strings_genCat`, the same generated source `regex` uses as `__regex_genCat`,
//! renamed so the two file-local copies never collide), so it is injected only
//! `WhenUsed` — when a program both `IMPORT strings` AND references a seam member —
//! through the plan-99 `HelperGate::WhenUsed` facility. The helper is named exactly
//! `"strings"` so its synthetic file label derives as the legacy `<builtin-strings>`
//! (byte-identical injection).
//!
//! The `AttributedString` Tier-A/Tier-B resolver survives as a co-located IR-level
//! rewrite ([`is_tier_a_query`] / [`is_tier_b_transform`] / [`tier_b_transform_impl`],
//! read by `ir::lower`), the audio/vector idiom — NOT expressed through the registry
//! matcher, because `AttributedString` is `astrings`' type (still hardcoded, as
//! `astrings` has not migrated).

use crate::codegen::registry::{HelperGate, Registry, RegistryHelper, RegistryPackage};

mod func_byte_len;
mod func_case_fold;
mod func_contains;
mod func_count;
mod func_display_width;
mod func_ends_with;
mod func_ends_with_any;
mod func_find;
mod func_from_scalars;
mod func_grapheme_at;
mod func_graphemes;
mod func_graphemes_count;
mod func_is_digit;
mod func_is_letter;
mod func_is_lower;
mod func_is_upper;
mod func_is_whitespace;
mod func_join;
mod func_left;
mod func_lower;
mod func_mid;
mod func_normalize_nfc;
mod func_pad_left;
mod func_pad_right;
mod func_repeat;
mod func_replace;
mod func_right;
mod func_split;
mod func_starts_with;
mod func_starts_with_any;
mod func_strip_prefix;
mod func_strip_suffix;
mod func_to_bytes;
mod func_to_scalars;
mod func_trim;
mod func_trim_chars;
mod func_trim_end;
mod func_trim_start;
mod func_upper;

/// One-line package intro (was `BuiltinModule::doc_intro`, historically empty).
const INTRO: &str = "";
/// Package-overview description (historically empty; the man page is the doc
/// authority for `strings`).
const DESC: &str = "";

/// The Unicode general-category table, `__regex_genCat` renamed to `__strings_genCat`
/// so `strings`' file-local copy never collides with `regex`' when both are imported
/// (bug-339 B1: one SOURCE of truth, one COMPILED copy per package — language-mandated
/// because an injected builtin source is one file whose FUNCs are file-local).
const GENCAT_TABLE: &str = include_str!("../../string/unicode/unicode_gencat.mfb");

/// The scalar-seam source companion (the `__strings_toScalars`/`__strings_fromScalars`
/// seam + the five classification predicates), backing the seven `Body::Rewrite`
/// members. Injected `WhenUsed` alongside the renamed general-category table.
const SEAM_SOURCE: &str = include_str!("seam.mfb");

/// Register the `strings` package on the clean-room registry.
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("strings", INTRO, DESC);

    // Native members (shared codegen carrier).
    func_trim::register(&mut pkg);
    func_trim_start::register(&mut pkg);
    func_trim_end::register(&mut pkg);
    func_upper::register(&mut pkg);
    func_lower::register(&mut pkg);
    func_case_fold::register(&mut pkg);
    func_normalize_nfc::register(&mut pkg);
    func_graphemes::register(&mut pkg);
    func_starts_with::register(&mut pkg);
    func_ends_with::register(&mut pkg);
    func_contains::register(&mut pkg);
    func_split::register(&mut pkg);
    func_join::register(&mut pkg);
    func_byte_len::register(&mut pkg);
    func_starts_with_any::register(&mut pkg);
    func_ends_with_any::register(&mut pkg);
    func_strip_prefix::register(&mut pkg);
    func_strip_suffix::register(&mut pkg);
    func_count::register(&mut pkg);
    func_left::register(&mut pkg);
    func_right::register(&mut pkg);
    func_repeat::register(&mut pkg);
    func_pad_left::register(&mut pkg);
    func_pad_right::register(&mut pkg);
    func_grapheme_at::register(&mut pkg);
    func_graphemes_count::register(&mut pkg);
    func_display_width::register(&mut pkg);
    func_trim_chars::register(&mut pkg);
    func_to_bytes::register(&mut pkg);

    // Intrinsic members shared with `collections::` (find/mid/replace).
    func_find::register(&mut pkg);
    func_mid::register(&mut pkg);
    func_replace::register(&mut pkg);

    // Scalar-seam + classification predicates (source-companion rewrites).
    func_to_scalars::register(&mut pkg);
    func_from_scalars::register(&mut pkg);
    func_is_letter::register(&mut pkg);
    func_is_digit::register(&mut pkg);
    func_is_whitespace::register(&mut pkg);
    func_is_upper::register(&mut pkg);
    func_is_lower::register(&mut pkg);

    // The scalar-seam source companion, gated `WhenUsed`. Named exactly `"strings"`
    // so its synthetic file derives the legacy `<builtin-strings>` label. The heavy
    // general-category table is appended with `__regex_genCat` renamed to
    // `__strings_genCat` (a runtime rename over the shared generated source), leaked
    // to `'static` once behind the registry's `OnceLock` build (like the other
    // boundary leaks in `registry`).
    let seam_body: &'static str = Box::leak(
        format!(
            "{}\n{}",
            SEAM_SOURCE,
            GENCAT_TABLE.replace("__regex_genCat", "__strings_genCat"),
        )
        .into_boxed_str(),
    );
    pkg.add_helper(RegistryHelper {
        name: "strings",
        gate: HelperGate::WhenUsed(&[
            "toScalars",
            "fromScalars",
            "isLetter",
            "isDigit",
            "isWhitespace",
            "isUpper",
            "isLower",
        ]),
        body: Some(seam_body),
        import_name: None,
    });
    // The injected `astrings` companion (`astrings_package.mfb`) `IMPORT strings` and
    // calls the scalar seam (`strings::toScalars`/`fromScalars`) from its Tier-B/
    // attribute bodies. That companion is injected AFTER this generic registry pass, so
    // the `WhenUsed` gate above (which only sees the user AST) cannot observe the
    // transitive seam use for an `astrings`-only program. A second gate — same body,
    // deduped by the shared `"strings"` name — rides the seam in whenever `astrings` is
    // imported, reproducing the pre-migration late-pass `strings::uses_package` walk
    // that saw the companion's seam references (plan-99 PART B).
    pkg.add_helper(RegistryHelper {
        name: "strings",
        gate: HelperGate::WhenImported("astrings"),
        body: Some(seam_body),
        import_name: None,
    });

    r.add_package(pkg);
}

// ---------------------------------------------------------------------------
// AttributedString Tier-A / Tier-B resolver — co-located IR-level rewrite.
//
// `strings::` members can take/return an `AttributedString` (astrings' type, which
// STAYS hardcoded/always-in-scope — astrings has not migrated). This is a genuine
// non-registry behavior (the registry matcher speaks only its own type vocabulary),
// so it lives here as a surviving co-located rewrite (the audio/vector idiom) that
// `ir::lower` consults, NOT as a registry matcher entry. The frozen Tier-A/Tier-B
// partition is plan-89-C §4.1.
// ---------------------------------------------------------------------------

/// Argument-validated return type of a `strings::` call, reproducing the deleted
/// `StringsResolver::resolve_return_type` (the co-located survivor of the
/// `AttributedString` Tier-A/Tier-B typing, the vector idiom). A `strings::` member
/// can take an `AttributedString` (astrings' still-hardcoded type) at the text
/// position:
///
/// - a **Tier-A** query answers on the visible text, so its result type is exactly
///   the `String` overload's (substitute `String` for the leading `AttributedString`
///   and reuse the registry resolution — codegen rewrites the argument to
///   `toString(a)`);
/// - a **Tier-B** transform re-expresses the text, so it yields an
///   `AttributedString` (validate the trailing arguments against the `String`
///   overload, then report `AttributedString`).
///
/// Every other call defers to the generic `registry::resolve_call`. `strict` carries
/// through the bug-443 strict(validation)/lenient(inference) split.
pub(crate) fn resolve_return_type(
    name: &str,
    arg_types: &[String],
    strict: bool,
) -> Option<String> {
    if arg_types.first().map(String::as_str) == Some("AttributedString") {
        if is_tier_a_query(name) {
            let mut substituted = arg_types.to_vec();
            substituted[0] = "String".to_string();
            return crate::codegen::registry::resolve_call(name, &substituted, strict);
        }
        if is_tier_b_transform(name) {
            let mut substituted = arg_types.to_vec();
            substituted[0] = "String".to_string();
            return crate::codegen::registry::resolve_call(name, &substituted, strict)
                .map(|_| "AttributedString".to_string());
        }
    }
    crate::codegen::registry::resolve_call(name, arg_types, strict)
}

/// The Tier-A `strings::` query members (plan-89-C): they *interrogate* the text
/// (returning a measurement, a position, or a decomposition into a collection)
/// rather than re-expressing it, so an `AttributedString` argument is answered on its
/// visible text and the result type matches the `String` overload. `ir::lower` wraps
/// the leading argument in `toString(a)` for these. Keyed on the qualified dot name.
pub(crate) fn is_tier_a_query(name: &str) -> bool {
    matches!(
        name,
        "strings.byteLen"
            | "strings.contains"
            | "strings.count"
            | "strings.displayWidth"
            | "strings.endsWith"
            | "strings.endsWithAny"
            | "strings.find"
            | "strings.graphemes"
            | "strings.graphemesCount"
            | "strings.split"
            | "strings.startsWith"
            | "strings.startsWithAny"
            | "strings.toBytes"
            | "strings.toScalars"
            | "strings.graphemeAt"
    )
}

/// The Tier-B `strings::` transform members (plan-89-D): they *modify* the text
/// (re-express it — narrowed, extended, or rewritten), so an `AttributedString`
/// argument yields an `AttributedString` whose text is transformed exactly as the
/// `String` overload's and whose attribute spans are remapped by the same edit.
/// `ir::lower` routes these to their `__astrings_*` body. Keyed on the qualified name.
pub(crate) fn is_tier_b_transform(name: &str) -> bool {
    matches!(
        name,
        "strings.left"
            | "strings.right"
            | "strings.mid"
            | "strings.trim"
            | "strings.trimStart"
            | "strings.trimEnd"
            | "strings.trimChars"
            | "strings.stripPrefix"
            | "strings.stripSuffix"
            | "strings.padLeft"
            | "strings.padRight"
            | "strings.repeat"
            | "strings.replace"
            | "strings.upper"
            | "strings.lower"
            | "strings.caseFold"
            | "strings.normalizeNfc"
    )
}

/// The `astrings` source-companion implementation symbol for a Tier-B transform of an
/// `AttributedString` (plan-89-D). `None` for a non-Tier-B name. `ir::lower` routes a
/// `strings::<t>(AttributedString, …)` call to this `__astrings_*` body instead of
/// the native `String` transform.
pub(crate) fn tier_b_transform_impl(name: &str) -> Option<&'static str> {
    let symbol = match name {
        "strings.left" => "__astrings_left",
        "strings.right" => "__astrings_right",
        "strings.mid" => "__astrings_mid",
        "strings.trim" => "__astrings_trim",
        "strings.trimStart" => "__astrings_trimStart",
        "strings.trimEnd" => "__astrings_trimEnd",
        "strings.trimChars" => "__astrings_trimChars",
        "strings.stripPrefix" => "__astrings_stripPrefix",
        "strings.stripSuffix" => "__astrings_stripSuffix",
        "strings.padLeft" => "__astrings_padLeft",
        "strings.padRight" => "__astrings_padRight",
        "strings.repeat" => "__astrings_repeat",
        "strings.replace" => "__astrings_replace",
        "strings.upper" => "__astrings_upper",
        "strings.lower" => "__astrings_lower",
        "strings.caseFold" => "__astrings_caseFold",
        "strings.normalizeNfc" => "__astrings_normalizeNfc",
        _ => return None,
    };
    Some(symbol)
}

pub(crate) mod builder_strings_builtins;
pub(crate) mod builder_strings_package;
pub(crate) use builder_strings_package::*;
