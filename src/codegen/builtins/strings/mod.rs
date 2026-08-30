//! The built-in `strings` package (clean-room registry migration, plan-99 PART B).
//!
//! `strings` is a large, mostly-**native** package: 29 members
//! (`trim`/`upper`/`split`/`join`/`padLeft`/…) each lower in their own
//! `func_<name>.rs` (registry `Body::abi_inline` — the inline
//! mode for the type-aware/static-fold members) — a single-use body is
//! inlined there, and logic shared by several members lives in a `gen_<area>.rs`
//! seam (`gen_case_map`/`gen_graphemes`/`gen_trim`/`gen_with_any`/`gen_strip`/
//! `gen_left_right`/`gen_pad`), with the shared string primitives + `UnicodeCaseMap`
//! + the static-fold helper in `gen_strings_support.rs` (the clean-room shape that
//! replaced the old `builder_strings_*` carrier). Three (`find`/`mid`/`replace`) are
//! `Body::Intrinsic`, sharing their bare
//! native lowering with the `collections::` `List` overloads through
//! `builtins::native_builtin_target`; and seven — the Unicode scalar seam
//! (`toScalars`/`fromScalars`) and the five classification predicates
//! (`isLetter`/`isDigit`/`isWhitespace`/`isUpper`/`isLower`) — are `Body::Rewrite`s
//! into the injected scalar-seam chunk (`helper_scalar_seam.rs`, a gated helper).
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

use crate::codegen::registry::{Registry, RegistryPackage};

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

mod gen_case_map;
mod gen_graphemes;
mod gen_left_right;
mod gen_pad;
mod gen_strings_support;
mod gen_strip;
mod gen_trim;
mod gen_with_any;
mod helper_scalar_seam;
pub(crate) use gen_strings_support::*;

/// One-line package intro (was `BuiltinModule::doc_intro`, historically empty).
const INTRO: &str = r#"Unicode-aware helpers for `String` values"#;
/// Package-overview description (historically empty; the man page is the doc
/// authority for `strings`).
const DESC: &str = r#"The `strings` package provides package-qualified helpers for `String` values:
trimming and case mapping (`trim`, `trimStart`, `trimEnd`, `trimChars`, `upper`,
`lower`, `caseFold`), Unicode normalization and segmentation (`normalizeNfc`,
`graphemes`, `graphemeAt`, `graphemesCount`), tests and search (`startsWith`,
`endsWith`, `contains`, `startsWithAny`, `endsWithAny`, `find`, `count`), slicing
and reshaping (`left`, `right`, `mid`, `stripPrefix`, `stripSuffix`, `split`,
`join`, `replace`, `repeat`, `padLeft`, `padRight`), length and byte queries
(`byteLen`, `toBytes`), and the Unicode-scalar seam (`toScalars`, `fromScalars`,
and the `Scalar` classifiers `isLetter`, `isDigit`, `isWhitespace`, `isUpper`,
`isLower`).

These helpers do not mutate their arguments. Functions that transform text return
a new `String`; `graphemes` and `split` return a `List OF String`, `toBytes`
returns a `List OF Byte`, `toScalars` returns a `List OF Scalar`, and the
original value is left unchanged. The scalar seam bridges `String` and the
`Scalar` primitive: `toScalars` walks a string one Unicode scalar at a time and
`fromScalars` rebuilds one, an exact round trip; the five `isX(Scalar)`
predicates classify a single scalar by its Unicode general category.

Index- and count-based functions (`find`, `mid`, `left`, `right`) measure
positions in zero-based Unicode scalar values, not bytes or graphemes. The
grapheme helpers `graphemes`, `graphemeAt`, and `graphemesCount` are the
exception: they operate on user-perceived extended grapheme clusters. `byteLen`
reports the length of the UTF-8 encoding in bytes, and `toBytes` returns those
raw UTF-8 bytes one element per byte. Case-insensitive comparison should use
`caseFold` rather than `upper` or `lower`, and content that may combine
characters differently can be normalized with `normalizeNfc` before comparison.

Several functions accept an optional or defaulted argument: `find` takes an
optional `start` position, and `padLeft` and `padRight` take an optional
`padChar` that defaults to a single space. The pad character, when supplied, must
be exactly one Unicode scalar value.

`strings` is a built-in package: `IMPORT strings` needs no manifest dependency.

Many `strings` functions also accept an `astrings::AttributedString` at the text
position; the overload is documented on each function's own page. A query returns
exactly what the `String` overload returns, computed on the visible text; a
text-transforming function returns an `AttributedString`, remapping the attribute
spans by the same edit (`upper`, `lower`, `caseFold`, and `normalizeNfc` change
scalar counts within a span, so they drop attributes)."#;

/// Register the `strings` package on the clean-room registry.
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("strings", INTRO, DESC);

    // Native members (per-member `func_*` lowerings + shared `gen_*` seams).
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

    // The scalar seam + classification predicates + general-category table, as one
    // gated chunk (see `helper_scalar_seam.rs` for why it cannot be `Body::mfb`).
    helper_scalar_seam::register(&mut pkg);

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
    arg_types: &[crate::types::ParameterType],
    strict: bool,
) -> Option<crate::types::ParameterType> {
    use crate::types::ParameterType;
    // plan-111-C: typed. `AttributedString` has no variant, so the leading
    // argument is recognized as the nominal it is; the substitution swaps in the
    // `String` variant, which is what `parse("String")` produced before.
    if arg_types
        .first()
        .is_some_and(|a| a.is_named("AttributedString"))
    {
        if is_tier_a_query(name) {
            let mut substituted = arg_types.to_vec();
            substituted[0] = ParameterType::String;
            return crate::codegen::registry::resolve_call_typed(name, &substituted, strict);
        }
        if is_tier_b_transform(name) {
            let mut substituted = arg_types.to_vec();
            substituted[0] = ParameterType::String;
            return crate::codegen::registry::resolve_call_typed(name, &substituted, strict)
                .map(|_| ParameterType::named("AttributedString"));
        }
    }
    crate::codegen::registry::resolve_call_typed(name, arg_types, strict)
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
/// `ir::lower` routes these to their `__astrings_*` body. Keyed on the qualified
/// name; each row pairs the member with its source-companion implementation symbol.
const TIER_B_TRANSFORMS: &[(&str, &str)] = &[
    ("strings.left", "__astrings_left"),
    ("strings.right", "__astrings_right"),
    ("strings.mid", "__astrings_mid"),
    ("strings.trim", "__astrings_trim"),
    ("strings.trimStart", "__astrings_trimStart"),
    ("strings.trimEnd", "__astrings_trimEnd"),
    ("strings.trimChars", "__astrings_trimChars"),
    ("strings.stripPrefix", "__astrings_stripPrefix"),
    ("strings.stripSuffix", "__astrings_stripSuffix"),
    ("strings.padLeft", "__astrings_padLeft"),
    ("strings.padRight", "__astrings_padRight"),
    ("strings.repeat", "__astrings_repeat"),
    ("strings.replace", "__astrings_replace"),
    ("strings.upper", "__astrings_upper"),
    ("strings.lower", "__astrings_lower"),
    ("strings.caseFold", "__astrings_caseFold"),
    ("strings.normalizeNfc", "__astrings_normalizeNfc"),
];

pub(crate) fn is_tier_b_transform(name: &str) -> bool {
    TIER_B_TRANSFORMS.iter().any(|(member, _)| *member == name)
}

/// The `astrings` source-companion implementation symbol for a Tier-B transform of an
/// `AttributedString` (plan-89-D). `None` for a non-Tier-B name. `ir::lower` routes a
/// `strings::<t>(AttributedString, …)` call to this `__astrings_*` body instead of
/// the native `String` transform.
pub(crate) fn tier_b_transform_impl(name: &str) -> Option<&'static str> {
    TIER_B_TRANSFORMS
        .iter()
        .find(|(member, _)| *member == name)
        .map(|(_, symbol)| *symbol)
}

/// The Tier-B member whose `AttributedString` call lowers to `symbol` (either
/// the `__astrings_*` spelling or the internalized `#astrings_*` the IR
/// carries) — the inverse of [`tier_b_transform_impl`], for the IR-level call
/// rules that report the member the source wrote (plan-107-E).
pub(crate) fn tier_b_transform_owner(symbol: &str) -> Option<&'static str> {
    TIER_B_TRANSFORMS
        .iter()
        .find(|(_, implementation)| {
            *implementation == symbol || crate::internal_name::internalize(implementation) == symbol
        })
        .map(|(member, _)| *member)
}
