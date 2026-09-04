//! The built-in `astrings` package (clean-room registry migration).
//!
//! `astrings` provides construction, mutation, query, and rendering for the opaque,
//! value-semantic `AttributedString` type. That TYPE stays **hardcoded and
//! always-in-scope** (like `Error`) — spread across `ir/verify`,
//! `target/macos_aarch64`, `target/shared/registry`, and the code layer — and is NOT
//! migrated here; only `astrings`' FUNCTIONS and its injected source move.
//!
//! The public members split three ways by realization:
//!   - `fromString` is **native-direct** codegen — `Body::abi_inline`, a thin
//!     wrapper over the shared `AttributedString` carrier
//!     (`CodeBuilder::lower_astrings_package_call` in
//!     `src/codegen/builtins/astrings/gen_astrings.rs`).
//!   - the `Attribute`-model constructors (`bold`..`background`) and the Tier-C
//!     mutation/query members (`addAttribute`..`toMarkdown`) are **source members**
//!     — each `func_*.rs` descriptor carries its generic-free `__astrings_*` MFBASIC
//!     body as `Body::mfb`, rewritten through the registry's overload-aware
//!     `rewrite_target`. `clearAttributes` overloads on arity: the whole form
//!     (1 arg) rewrites to `__astrings_clearAttributes`, the ranged form (3 args)
//!     to `__astrings_clearAttributesRange`.
//!   - `readSpans`/`writeSpans`/`scalarLen` are **internal-only** native overlay-bridge
//!     primitives (`Body::abi_inline`, `internal_only: true`): they cross the
//!     opaque record boundary the injected source cannot touch. Users can never call
//!     them (the `internal_only` flag, honored by `builtins::is_internal_only_call`).
//!
//! The injected source (formerly a single `package.mfb` companion) is assembled
//! by `RegistryPackage::get_mfb` from parts registered here: the open `Attribute`
//! model (`add_record`/`add_union`/`add_enum` below), the PRIVATE `__astrings_*`
//! helper bodies (one `helper_*.rs` per FUNC, `add_helper` — private-only), and
//! the public members' `Body::Mfb` bodies from their `func_*.rs` descriptors. The assembled file keeps the legacy
//! `<builtin-astrings>` label (derived from the package import name) and is emitted
//! on IMPORT by `Registry::augment_project`. The source `IMPORT strings` and calls
//! the scalar seam (`strings::toScalars`/…); the `strings` package rides that seam
//! in whenever `astrings` is imported via its landed `WhenImported("astrings")` gate.
//!
//! The `.mfb` bodies own the risky inclusive-bound split arithmetic and the
//! higher-start-wins resolution (plan-89-B §3); the natives only cross the
//! opaque-record boundary.

use crate::codegen::registry::{
    EnumVariant, RecordProp, Registry, RegistryEnum, RegistryPackage, RegistryRecord,
    RegistryUnion, UnionVariant,
};
use crate::types::ParameterType;

mod func_add_attribute;
mod func_background;
mod func_bold;
mod func_clear_attributes;
mod func_font;
mod func_font_size;
mod func_foreground;
mod func_from_string;
mod func_get_attributes;
mod func_italic;
mod func_overline;
mod func_read_spans;
mod func_remove_attribute;
mod func_scalar_len;
mod func_strike;
mod func_to_markdown;
mod func_underline;
mod func_write_spans;

mod helper_assemble;
mod helper_attr_equals;
mod helper_case_fold;
mod helper_concat;
mod helper_decode_attr;
mod helper_encode_attr;
mod helper_find_matches;
mod helper_flag_from_member;
mod helper_flag_member;
mod helper_is_winner;
mod helper_leading_in_set;
mod helper_left;
mod helper_lower;
mod helper_md_escape;
mod helper_md_escape_font;
mod helper_md_state_at;
mod helper_mid;
mod helper_next_seq;
mod helper_normalize_nfc;
mod helper_number_from_member;
mod helper_number_member;
mod helper_pack_color;
mod helper_pad_left;
mod helper_pad_right;
mod helper_remap_segment;
mod helper_repeat;
mod helper_replace;
mod helper_right;
mod helper_scalar_count_str;
mod helper_shift_spans;
mod helper_split_span;
mod helper_strip_prefix;
mod helper_strip_suffix;
mod helper_trim;
mod helper_trim_chars;
mod helper_trim_end;
mod helper_trim_start;
mod helper_upper;
mod helper_validate_range;
mod helper_window_spans;

/// One-line package intro (ported from the archived `planning/old_man` page).
const INTRO: &str = r#"Attributed (styled) text: an opaque `AttributedString` value"#;
/// Package-overview description (ported from the archived `planning/old_man`
/// page, citation markers stripped).
const DESC: &str = r#"The `astrings` package works with `AttributedString`, an opaque built-in that
pairs visible `String` text with an attribute overlay describing per-range style
(bold, italic, font, size, foreground/background color, …). It is an **ordinary
value**: assigning or passing one copies it, text and attributes together, so
changing one cannot change another, and a default one has empty text and no
attributes. It is
**opaque** — it exposes no user-visible fields (`a.text` does not compile), cannot
be built with a record literal (`AttributedString[...]`), and cannot be
`WITH`-updated. It is copyable and defaultable but **not** comparable, so it is
never a `Map` key or `Set` element.

Reach the visible text with `toString(a)`; `io::print`/`io::write` emit it. The
text is never reached by an implicit coercion — only through `toString` or an
explicit overload.

`astrings::fromString(text)` constructs an `AttributedString` whose visible text
is `text` and whose attribute overlay is empty."#;

/// Register the `astrings` package on the clean-room registry.
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("astrings", INTRO, DESC);

    // The injected source's IMPORT lines (verbatim order from the old companion).
    // `astrings` imports itself so the assembled file can call the internal
    // overlay-bridge members (`astrings::readSpans`/`writeSpans`/`scalarLen`).
    pkg.add_imports(vec!["collections", "astrings", "strings", "bits"]);

    // The open `Attribute` model: a styling attribute is a flag (`AttrFlag`), a
    // String-valued attribute (`AttrText`), or an Integer-valued one (`AttrNumber`),
    // unioned as `Attribute`. Records render first in the assembled source, then the
    // union, then the enums.
    pkg.add_record(RegistryRecord {
        name: "AttrFlag",
        export: true,
        description: "A flag attribute: one member of `astrings::AttrTypeFlag`, carrying no value.",
        props: vec![RecordProp {
            name: "kind",
            ty: ParameterType::named("AttrTypeFlag"),
            description: "Which flag.",
        }],
    });
    pkg.add_record(RegistryRecord {
        name: "AttrText",
        export: true,
        description:
            "A String-valued attribute: an `astrings::AttrTypeText` member and its String value.",
        props: vec![
            RecordProp {
                name: "kind",
                ty: ParameterType::named("AttrTypeText"),
                description: "Which text attribute.",
            },
            RecordProp {
                name: "value",
                ty: ParameterType::String,
                description: "The String value (e.g. the font family name).",
            },
        ],
    });
    pkg.add_record(RegistryRecord {
        name: "AttrNumber",
        export: true,
        description:
            "An Integer-valued attribute: an `astrings::AttrTypeNumber` member and its value.",
        props: vec![
            RecordProp {
                name: "kind",
                ty: ParameterType::named("AttrTypeNumber"),
                description: "Which numeric attribute.",
            },
            RecordProp {
                name: "value",
                ty: ParameterType::Integer,
                description: "The Integer value (e.g. the font size in points).",
            },
        ],
    });
    // The internal stored-span record. Field-identical to the codegen-internal
    // `AttrSpan` registered in `validation.rs` (the `AttributedString` overlay
    // element). Not exported: only the injected helpers and the native bridge touch
    // it. `class`: 0=flag, 1=text, 2=number. `member`: the enum-member ordinal within
    // that class. `text`/`number`: the flat attribute payload. `last` (not `end`)
    // because `end` is a reserved keyword — it can be neither a member accessed with
    // `.` nor a parameter/field identifier.
    pkg.add_record(RegistryRecord {
        name: "AttrSpan",
        export: false,
        description: "",
        props: vec![
            RecordProp {
                name: "start",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "last",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "seq",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "class",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "member",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "text",
                ty: ParameterType::String,
                description: "",
            },
            RecordProp {
                name: "number",
                ty: ParameterType::Integer,
                description: "",
            },
        ],
    });
    // The internal resolved-styling-state record `toMarkdown` scans by (plan-89-E).
    // Comparable (all fields comparable), so a run boundary is simply a change in
    // this record.
    pkg.add_record(RegistryRecord {
        name: "MdState",
        export: false,
        description: "",
        props: vec![
            RecordProp {
                name: "bold",
                ty: ParameterType::Boolean,
                description: "",
            },
            RecordProp {
                name: "italic",
                ty: ParameterType::Boolean,
                description: "",
            },
            RecordProp {
                name: "underline",
                ty: ParameterType::Boolean,
                description: "",
            },
            RecordProp {
                name: "strike",
                ty: ParameterType::Boolean,
                description: "",
            },
            RecordProp {
                name: "overline",
                ty: ParameterType::Boolean,
                description: "",
            },
            RecordProp {
                name: "hasFont",
                ty: ParameterType::Boolean,
                description: "",
            },
            RecordProp {
                name: "font",
                ty: ParameterType::String,
                description: "",
            },
            RecordProp {
                name: "hasSize",
                ty: ParameterType::Boolean,
                description: "",
            },
            RecordProp {
                name: "size",
                ty: ParameterType::Integer,
                description: "",
            },
        ],
    });

    // One styling attribute: a flag, a String-valued, or an Integer-valued one.
    // Match on the variant to read the underlying value.
    pkg.add_union(RegistryUnion {
        name: "Attribute",
        export: true,
        variants: vec![
            UnionVariant {
                name: "AttrFlag",
                description: "A boolean flag (bold, italic, …).",
            },
            UnionVariant {
                name: "AttrText",
                description: "A String-valued attribute (font).",
            },
            UnionVariant {
                name: "AttrNumber",
                description: "An Integer-valued attribute (font size).",
            },
        ],
    });

    // A boolean styling flag with no value — either present on a run or not.
    pkg.add_enum(RegistryEnum {
        name: "AttrTypeFlag",
        export: true,
        variants: vec![
            EnumVariant {
                name: "Bold",
                description: "Bold weight.",
                advisory: None,
            },
            EnumVariant {
                name: "Italic",
                description: "Italic slant.",
                advisory: None,
            },
            EnumVariant {
                name: "Underline",
                description: "Underlined text.",
                advisory: None,
            },
            EnumVariant {
                name: "Strike",
                description: "Struck-through text.",
                advisory: None,
            },
            EnumVariant {
                name: "Overline",
                description: "Overlined text.",
                advisory: None,
            },
        ],
    });
    // A styling attribute whose value is a String (e.g. a font family name).
    pkg.add_enum(RegistryEnum {
        name: "AttrTypeText",
        export: true,
        variants: vec![EnumVariant {
            name: "Font",
            description: "The font family name.",
            advisory: None,
        }],
    });
    // A styling attribute whose value is an Integer (e.g. a font size, or a packed
    // `0xAARRGGBB` color for `Foreground`/`Background`).
    pkg.add_enum(RegistryEnum {
        name: "AttrTypeNumber",
        export: true,
        variants: vec![
            EnumVariant {
                name: "FontSize",
                description: "The font size in points.",
                advisory: None,
            },
            EnumVariant {
                name: "Foreground",
                description: "The text color, packed `0xAARRGGBB` — the same order `color::toPacked` produces. Terminals have no alpha; `term::drawText` ignores it.",
                advisory: None,
            },
            EnumVariant {
                name: "Background",
                description: "The background color, packed `0xAARRGGBB` — the same order `color::toPacked` produces. Terminals have no alpha; `term::drawText` ignores it.",
                advisory: None,
            },
        ],
    });

    // The injected `__astrings_*` FUNC bodies. Each lives in its own `helper_*.rs`
    // and registers via `add_helper`; they render (in this order) in the helper
    // section of the assembled source. Order is preserved from the old single
    // `package.mfb` blob.
    //
    // Convenience constructors.
    helper_pack_color::register(&mut pkg);
    // Attribute <-> AttrSpan encoding.
    helper_flag_member::register(&mut pkg);
    helper_flag_from_member::register(&mut pkg);
    helper_number_member::register(&mut pkg);
    helper_number_from_member::register(&mut pkg);
    helper_encode_attr::register(&mut pkg);
    helper_decode_attr::register(&mut pkg);
    helper_attr_equals::register(&mut pkg);
    // Bounds + sequence helpers.
    helper_validate_range::register(&mut pkg);
    helper_next_seq::register(&mut pkg);
    // Tier-C: mutation.
    helper_split_span::register(&mut pkg);
    // Tier-C: query (higher-start-wins resolution).
    helper_is_winner::register(&mut pkg);
    // Tier-B: attribute-preserving transforms (plan-89-D). Each transform runs the
    // existing String transform on the visible text, then remaps the stored spans by
    // the same edit (all inclusive scalar bounds). The text invariant
    // `toString(t(a)) == strings::t(toString(a))` holds by construction (the new
    // text IS `strings::t(text)`).
    helper_scalar_count_str::register(&mut pkg);
    helper_window_spans::register(&mut pkg);
    helper_shift_spans::register(&mut pkg);
    helper_assemble::register(&mut pkg);
    helper_left::register(&mut pkg);
    helper_right::register(&mut pkg);
    helper_mid::register(&mut pkg);
    helper_trim_start::register(&mut pkg);
    helper_trim_end::register(&mut pkg);
    helper_trim::register(&mut pkg);
    helper_leading_in_set::register(&mut pkg);
    helper_trim_chars::register(&mut pkg);
    helper_strip_prefix::register(&mut pkg);
    helper_strip_suffix::register(&mut pkg);
    helper_pad_left::register(&mut pkg);
    helper_pad_right::register(&mut pkg);
    helper_repeat::register(&mut pkg);
    helper_upper::register(&mut pkg);
    helper_lower::register(&mut pkg);
    helper_case_fold::register(&mut pkg);
    helper_normalize_nfc::register(&mut pkg);
    helper_find_matches::register(&mut pkg);
    helper_remap_segment::register(&mut pkg);
    helper_replace::register(&mut pkg);
    helper_concat::register(&mut pkg);
    // toMarkdown (plan-89-E): render resolved styling into a bespoke markdown-
    // flavored format. NOT CommonMark. Flags wrap each maximal run as nested pairs
    // in canonical order (bold ** , italic * , underline __ , strike ~~ , overline
    // ^^); font/size switch forward via a minimal-delta `::font;size::` marker at
    // run boundaries; delimiter characters in text and font names are
    // backslash-escaped. Pure `.mfb` over getAttributes/toString (Open Decision 1).
    helper_md_escape::register(&mut pkg);
    helper_md_escape_font::register(&mut pkg);
    helper_md_state_at::register(&mut pkg);

    // Native-direct constructor (shared codegen carrier).
    func_from_string::register(&mut pkg);

    // Source-rewrite `Attribute`-model constructors.
    func_bold::register(&mut pkg);
    func_italic::register(&mut pkg);
    func_underline::register(&mut pkg);
    func_strike::register(&mut pkg);
    func_overline::register(&mut pkg);
    func_font::register(&mut pkg);
    func_font_size::register(&mut pkg);
    func_foreground::register(&mut pkg);
    func_background::register(&mut pkg);

    // Source-rewrite Tier-C mutation/query members.
    func_add_attribute::register(&mut pkg);
    func_remove_attribute::register(&mut pkg);
    func_clear_attributes::register(&mut pkg);
    func_get_attributes::register(&mut pkg);
    func_to_markdown::register(&mut pkg);

    // Internal-only native overlay bridge (never user-callable).
    func_read_spans::register(&mut pkg);
    func_write_spans::register(&mut pkg);
    func_scalar_len::register(&mut pkg);

    r.add_package(pkg);
}

#[cfg(test)]
mod tests {
    use crate::codegen::registry::registry;

    #[test]
    fn astrings_types_registered_on_the_clean_room_registry() {
        // The EXPORT records/union/enums are visible to the generic type query.
        for name in [
            "AttrFlag",
            "AttrText",
            "AttrNumber",
            "Attribute",
            "AttrTypeFlag",
            "AttrTypeText",
            "AttrTypeNumber",
        ] {
            assert!(registry().is_builtin_type(name), "{name} not registered");
        }
        // The internal records are modeled too (they must render into the
        // assembled source for the injected helpers to compile).
        assert!(registry().is_builtin_type("AttrSpan"));
        assert!(registry().is_builtin_type("MdState"));
    }

    #[test]
    fn reassembled_source_parses() {
        let source = registry()
            .resolve_package("astrings")
            .expect("astrings")
            .get_mfb();
        crate::ast::parse_source_internal(
            std::path::Path::new("<builtin-astrings>"),
            "builtins/astrings.mfb",
            &source,
        )
        .expect("reassembled astrings source parses");
    }
}

mod gen_astrings;
