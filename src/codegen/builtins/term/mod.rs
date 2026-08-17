//! The built-in `term` package (clean-room registry migration).
//!
//! `term` gives an MFBASIC program a structured terminal surface: the TUI-mode
//! toggle (`on`/`off`/`isOn`), colors and text attributes
//! (`setForeground`/`setBackground`/`setBold`/`setUnderline` + the `get*` readers),
//! cursor control (`showCursor`/`hideCursor`/`moveTo`), screen clearing / frame
//! presentation (`clear`/`sync`), box-drawing (`drawHLine`/`drawVLine`/`drawBox`/
//! `fillRect`), text/glyph stamping (`drawText`/`drawGlyph`), and size/resize queries
//! (`terminalSize`/`didResize`).
//!
//! Every one of the 24 members is a **native OS-seam** member: each registers a
//! `Body::native_os_seam` whose `posix`/`win` slots both hold the shared `term`
//! dispatcher ([`native::lower_term_helper`]), reached by the generic OS-seam dispatch
//! (`crate::codegen::os`). That dispatcher branches app-vs-console internally off the
//! per-compilation `OsLowerCtx` and delegates to the heavy terminal emitters kept in
//! the shared code layer (`code::lower_term_helper` for the console backend,
//! `CodegenPlatform::emit_app_term_helper` for the per-platform app backend). Those
//! ~150 KB of terminal-grid emission stay shared like the `strings`/`vector` codegen
//! carriers — relocating them would be byte-identity-risky and they are consumed by
//! the app backends too. Every member's return type is a function of the NAME alone
//! (each is one fixed-return, single overload), so there is no argument-typed
//! dispatch; the human `expected_arguments` phrasing (`"no arguments"`, `"Byte, Byte,
//! Byte"`, …) is kept hand-authored on the descriptor (the registry's per-position
//! render would say `"()"`), decoupled from the machine coercion table (bug-443).
//!
//! `term` owns four value types. The two records `TermColor` (`r`/`g`/`b` `Byte`) and
//! `TermSize` (`columns`/`rows` `Integer`) are registered via [`add_record`] — they are
//! **read-only** (the runtime allocates them; a program may neither construct nor
//! WITH-update one — see [`is_read_only_record`], consulted by
//! `ir::verify`/`syntaxcheck`). Their binary-repr wire ids stay the reserved high-band
//! `TYPE_TERM_COLOR`/`TYPE_TERM_SIZE` (name-keyed in `binary_repr::sections`). The two
//! source-companion enums `LineStyle` (the box-drawing weight) and `FillStyle` (the
//! block/shade glyph) are declared with their `DOC` blocks in the injected companion
//! (`package.mfb`) and registered by name via [`add_source_types`].
//!
//! The companion (`package.mfb`) is injected on IMPORT as an `Always` helper named
//! exactly `"term"`, deriving the legacy `<builtin-term>` label. The `term`↔`astrings`
//! `drawText(AttributedString)` bridge (`term_astrings_bridge.mfb`, carrying
//! `__term_drawTextAttr`) is injected as a `WhenImported("astrings")` gated helper
//! named `"term_astrings_bridge"` — its body references `AttributedString`, so it is
//! injected only when `astrings` is imported. The `drawText(String)` vs
//! `drawText(AttributedString)` overload selection is a co-located IR-level rewrite
//! (the audio/strings idiom), read by `ir::lower` keyed on [`DRAW_TEXT`], NOT the
//! registry matcher (`AttributedString` is `astrings`' still-hardcoded type).

use crate::codegen::registry::{
    HelperGate, RecordProp, Registry, RegistryHelper, RegistryPackage, RegistryRecord,
};
use crate::types::ParameterType;

pub(crate) mod native;

mod func_clear;
mod func_did_resize;
mod func_draw_box;
mod func_draw_glyph;
mod func_draw_hline;
mod func_draw_text;
mod func_draw_vline;
mod func_fill_rect;
mod func_get_background;
mod func_get_bold;
mod func_get_foreground;
mod func_get_underline;
mod func_hide_cursor;
mod func_is_on;
mod func_move_to;
mod func_off;
mod func_on;
mod func_set_background;
mod func_set_bold;
mod func_set_foreground;
mod func_set_underline;
mod func_show_cursor;
mod func_sync;
mod func_terminal_size;

/// The `term::drawText` qualified call name — the co-located IR-level rewrite key for
/// the `drawText(AttributedString)` overload (routed to `__term_drawTextAttr`).
pub(crate) const DRAW_TEXT: &str = "term.drawText";

/// The read-only `TermColor` record type (`r`/`g`/`b` `Byte`), returned by
/// `term::getForeground`/`getBackground`.
pub(crate) const TERM_COLOR_TYPE: &str = "TermColor";
/// The read-only `TermSize` record type (`columns`/`rows` `Integer`), returned by
/// `term::terminalSize`.
pub(crate) const TERM_SIZE_TYPE: &str = "TermSize";

/// Whether `type_name` is one of `term`'s compiler-owned, read-only record types
/// (`TermColor`/`TermSize`): the runtime allocates them, so a program may neither
/// construct nor WITH-update one. Consulted by `ir::verify::read_only_record_type` and
/// `syntaxcheck::helpers::read_only_record_type`.
pub(crate) fn is_read_only_record(type_name: &str) -> bool {
    type_name == TERM_COLOR_TYPE || type_name == TERM_SIZE_TYPE
}

/// One-line package intro (historically empty; the man page is the doc authority).
const INTRO: &str = "";
/// Package-overview description (historically empty).
const DESC: &str = "";

/// The source companion — the `LineStyle`/`FillStyle` enum declarations (with their
/// `DOC` blocks). Injected verbatim on IMPORT as an `Always` helper named `"term"`, so
/// its synthetic file derives the legacy `<builtin-term>` label (byte-identical to the
/// pre-migration `package_source_glue!` `include_str!`).
const COMPANION_SOURCE: &str = include_str!("package.mfb");

/// The `term`↔`astrings` `drawText(AttributedString)` bridge (`__term_drawTextAttr` +
/// its `__TermStyle`/color helpers). Injected as its own synthetic file only when a
/// program imports `astrings` (its body references `AttributedString`, undefined
/// otherwise); its label derives from the helper name as `<builtin-term_astrings_bridge>`.
const BRIDGE_SOURCE: &str = include_str!("term_astrings_bridge.mfb");

/// Register the `term` package on the clean-room registry.
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("term", INTRO, DESC);

    // The two read-only records (the runtime allocates them). Registered on the
    // registry so `is_builtin_type`/`qualified_builtin_type` resolve them and the
    // read-only checks repoint here; their wire ids stay the reserved high-band
    // `TYPE_TERM_COLOR`/`TYPE_TERM_SIZE` (name-keyed in `binary_repr::sections`).
    pkg.add_record(RegistryRecord {
        name: TERM_COLOR_TYPE,
        export: true,
        props: vec![
            RecordProp {
                name: "r",
                ty: ParameterType::Byte,
                description: "",
            },
            RecordProp {
                name: "g",
                ty: ParameterType::Byte,
                description: "",
            },
            RecordProp {
                name: "b",
                ty: ParameterType::Byte,
                description: "",
            },
        ],
    });
    pkg.add_record(RegistryRecord {
        name: TERM_SIZE_TYPE,
        export: true,
        props: vec![
            RecordProp {
                name: "columns",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "rows",
                ty: ParameterType::Integer,
                description: "",
            },
        ],
    });

    // The `LineStyle`/`FillStyle` enums are declared (with `DOC` blocks) in the injected
    // companion source; register their names so the type system recognizes them as
    // term-owned value types (the datetime idiom — source-declared, name-registered).
    pkg.add_source_types(&["LineStyle", "FillStyle"]);

    // Native OS-seam members (the shared terminal codegen carrier).
    func_on::register(&mut pkg);
    func_off::register(&mut pkg);
    func_is_on::register(&mut pkg);
    func_set_foreground::register(&mut pkg);
    func_set_background::register(&mut pkg);
    func_set_bold::register(&mut pkg);
    func_set_underline::register(&mut pkg);
    func_show_cursor::register(&mut pkg);
    func_hide_cursor::register(&mut pkg);
    func_clear::register(&mut pkg);
    func_sync::register(&mut pkg);
    func_move_to::register(&mut pkg);
    func_draw_hline::register(&mut pkg);
    func_draw_vline::register(&mut pkg);
    func_draw_box::register(&mut pkg);
    func_fill_rect::register(&mut pkg);
    func_draw_text::register(&mut pkg);
    func_draw_glyph::register(&mut pkg);
    func_get_foreground::register(&mut pkg);
    func_get_background::register(&mut pkg);
    func_get_bold::register(&mut pkg);
    func_get_underline::register(&mut pkg);
    func_terminal_size::register(&mut pkg);
    func_did_resize::register(&mut pkg);

    // The source companion (enums + DOC), injected on IMPORT. Named exactly `"term"` so
    // its synthetic file derives the legacy `<builtin-term>` label.
    pkg.add_helper(RegistryHelper::always("term", COMPANION_SOURCE));

    // The `term`↔`astrings` `drawText(AttributedString)` bridge — a cross-package gated
    // helper injected only when `astrings` is imported (its body references
    // `AttributedString`). The `strings` scalar seam the bridge calls rides in through
    // `strings`' own `WhenImported("astrings")` gate.
    pkg.add_helper(RegistryHelper {
        name: "term_astrings_bridge",
        gate: HelperGate::WhenImported("astrings"),
        body: Some(BRIDGE_SOURCE),
        import_name: None,
    });

    r.add_package(pkg);
}
