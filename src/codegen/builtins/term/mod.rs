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
//! Each of the 24 members owns its `Body::abi_function` body
//! (`func_*.rs::lower_<name>`); the `abi_function` wrapper seeds the entry label,
//! binds the ABI argument registers, and finalizes. Each body calls the shared
//! family-generic [`gen_shared::lower_term_helper`] with its own runtime-call name,
//! which branches app-vs-console internally off the [`AbiCtx`] it is threaded
//! (`build_mode`/`term_state_offset`/`presentation_mode_offset`) and delegates to the
//! heavy terminal emitters kept in the shared code layer
//! (`code::lower_term_helper` for the console backend,
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
//! `ir::verify`/the former source checker). Their binary-repr wire ids stay the reserved high-band
//! `TYPE_TERM_COLOR`/`TYPE_TERM_SIZE` (name-keyed in `binary_repr::sections`). The two
//! enums `LineStyle` (the box-drawing weight) and `FillStyle` (the block/shade glyph)
//! are registry-modeled via [`RegistryPackage::add_enum`] and rendered into the
//! injected `<builtin-term>` source by `get_mfb` (their variant docs surface on the
//! man `types` page).
//!
//! The `term`↔`astrings` `drawText(AttributedString)` bridge (`helper_astrings_bridge.rs`,
//! carrying `__term_drawTextAttr`) is injected as a `WhenBothImported("term", "astrings")`
//! gated helper chunk named `"term_astrings_bridge"` — its body references
//! `AttributedString`, so it is injected only when both packages are imported. The
//! `drawText(String)` vs `drawText(AttributedString)` overload selection is a
//! co-located IR-level rewrite (the audio/strings idiom), read by `ir::lower` keyed on
//! [`DRAW_TEXT`], NOT the registry matcher (`AttributedString` is `astrings`'
//! still-hardcoded type).

// --- codegen tier imports (migration) ---
use crate::codegen::registry::{
    EnumVariant, RecordProp, Registry, RegistryEnum, RegistryPackage, RegistryRecord,
};
use crate::types::ParameterType;

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

mod helper_astrings_bridge;

mod gen_shared;

/// The `term::drawText` qualified call name — the co-located IR-level rewrite key for
/// the `drawText(AttributedString)` overload (routed to `__term_drawTextAttr`).
pub(crate) const DRAW_TEXT: &str = "term.drawText";

/// The read-only `TermSize` record type (`columns`/`rows` `Integer`), returned by
/// `term::terminalSize`.
///
/// plan-122-F retired the sibling `TermColor`: the colour members now speak
/// `color::Color` (`builtins::color::COLOR_TYPE_ID`), which is an ordinary value
/// record rather than a runtime-allocated read-only one, so it needs no constant
/// here, no read-only rule, no reserved wire id and no resolver seed.
pub(crate) const TERM_SIZE_TYPE: &str = "TermSize";
/// Its package-qualified identity — what a consumer must write, and what the
/// resolver seeds, so a bare `AS TermSize` is refused (bug-484).
pub(crate) const TERM_SIZE_TYPE_ID: &str = "term.TermSize";

/// Whether `type_name` is `term`'s compiler-owned, read-only record type
/// (`TermSize`): the runtime allocates it, so a program may neither construct nor
/// WITH-update one. Consulted by `ir::verify::read_only_record_type` and
/// the former source checker's `helpers::read_only_record_type`.
///
/// Asked through `is_builtin_named` so both spellings answer the same: a source
/// `AS term::TermSize` resolves to the qualified `term.TermSize`, while the
/// injected companion and any record field naming one stay bare (bug-483).
pub(crate) fn is_read_only_record(type_name: &ParameterType) -> bool {
    type_name.is_builtin_named("term", TERM_SIZE_TYPE)
}

/// One-line package intro (historically empty; the man page is the doc authority).
const INTRO: &str = r#"Full-screen terminal TUI surface: cursor, colors, attributes, and clearing"#;
/// Package-overview description (historically empty).
const DESC: &str = r#"The `term` package gives a program a structured, full-screen terminal surface
for text user interfaces: it moves the cursor, sets the foreground and
background colors and the bold and underline attributes, clears the screen,
shows or hides the cursor, reports the surface size, and reports whether the
surface was resized (`term::didResize`). The same surface is
rendered on the console backend (using the terminal's alternate screen and ANSI
sequences) and in windowed app mode (`mfb build --app`), so a program draws the
same way on both.

`term::on` is the gate for the whole module. It switches the terminal into TUI
mode and resets all `term::` state to its defaults (white foreground, black
background, bold and underline off, cursor visible, screen cleared, cursor at
the home position). While TUI mode is off, nearly every other `term::` call is a
no-op, so a program must call `term::on` before any cursor, color, attribute, or
clear call takes effect, and `term::off` later leaves TUI mode and restores the
user's previous screen. There are two exceptions to the no-op rule:
`term::isOn`, which answers either way, and **`term::terminalSize`, which raises
`ErrUnsupported`** rather than returning a meaningless size.

While TUI mode is on the surface is **retained** and drawing is **buffered**:
drawing calls (including `io::print`/`io::write`) update the surface rather
than the terminal, and nothing appears until the program calls `term::sync`, the
one operation that presents a frame. The console backend presents by writing only
the cells that changed since the previous frame, so a program that repaints every
frame shows no flicker and emits output proportional to what actually changed; in
app mode `term::sync` coalesces the frame into a single redraw. `term::off`
performs a final `term::sync` before restoring the screen, so the last frame is
always shown. A program that draws without a following `term::sync` displays
nothing - the canonical shape is to compose a whole frame, call `term::sync`
once, then read input.

Coordinates are zero-based and measured from the top-left corner of the surface:
row 0 is the topmost line and column 0 is the leftmost column, so (0, 0) is the
home position. The first coordinate is always the row (vertical) and the second
the column (horizontal). Negative coordinates are clamped to 0; in app mode they
are also clamped at the high end to the last valid cell. Colors are 24-bit RGB
triples of three `Byte` channels (red, green, blue), each 0 to 255. Color and
attribute changes take effect immediately for subsequently drawn text and do not
alter text already on the screen; each setting is independent, so changing one
leaves the others untouched, and the matching get function reads the current
value back.

Beyond text, the surface can draw box-drawing rules: `term::drawHLine` stamps a
horizontal run of a box-drawing glyph across a row, `term::drawVLine` stamps a
vertical run down a column, and `term::drawBox` draws a whole rectangle (four
edges plus matching corners) between two opposite points — all using the colours
and attributes in effect and all presented on the next `term::sync`. The glyph
weight is chosen with the `term::LineStyle` enum (`Light`, `Heavy`, `LightDash`,
`HeavyDash`, `LightDot`, `HeavyDot`, `Double`); each variant has a horizontal form
for `drawHLine` and a vertical form for `drawVLine`, and `drawBox` pairs the edge
glyphs with the matching corner glyphs (dash/dot styles reuse the Light or Heavy
corners). `term::fillRect` fills a rectangular region with a block or shade glyph
chosen by the `term::FillStyle` enum (`Filled`, `Light`, `Medium`, `Dark`, `Checker`,
`CheckerAlt`) — the region-filling counterpart to `clear`. `term::drawText` stamps
a string at an absolute position (without moving the cursor), and
`term::drawGlyph` stamps a single scalar by code point.

The package defines two built-in record types and two enums. `term::TermColor` has three
`Byte` fields `r`, `g`, and `b` holding the red, green, and blue channels of a
color, and is returned by `term::getForeground` and `term::getBackground`.
`term::TermSize` has two `Integer` fields `columns` (the width of the surface in
character cells) and `rows` (its height), and is returned by `term::terminalSize`;
the surface size can change between calls (for example when the terminal window is
resized), so a program that depends on it should query it again rather than caching
the result. `term::LineStyle` selects the box-drawing weight for `term::drawHLine`,
`term::drawVLine`, and `term::drawBox`; `term::FillStyle` selects the block or shade
glyph for `term::fillRect`."#;

/// Register the `term` package on the clean-room registry.
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("term", INTRO, DESC);

    // The one remaining read-only record (the runtime allocates it). Registered on
    // the registry so `is_builtin_type`/`qualified_builtin_type` resolve it and the
    // read-only check repoints here; its wire id stays the reserved high-band
    // `TYPE_TERM_SIZE` (name-keyed in `binary_repr::sections`).
    //
    // plan-122-F retired `TermColor`: the colour members speak `color::Color`, an
    // ordinary value record a program may build and `WITH`-update, so the type no
    // longer needs a read-only rule, a reserved wire id, or a resolver seed.
    pkg.add_record(RegistryRecord {
        name: TERM_SIZE_TYPE,
        export: true,
        description: "The size of the drawing surface in whole character cells, as \
                      returned by `term::terminalSize`. Ask again after a resize \
                      rather than caching the first answer.",
        props: vec![
            RecordProp {
                name: "columns",
                ty: ParameterType::Integer,
                description: "The width in character cells, never pixels. Valid \
                              columns are 0 through columns-1.",
            },
            RecordProp {
                name: "rows",
                ty: ParameterType::Integer,
                description: "The height in character cells, never pixels. Valid \
                              rows are 0 through rows-1.",
            },
        ],
    });

    // The box-drawing weight `term::drawHLine`/`drawVLine`/`drawBox` stamp. Each
    // variant has a horizontal form (drawHLine) and a vertical form (drawVLine); the
    // discriminants are the 0-based positions native codegen uses to select the
    // glyph (`Light = 0` … `Double = 6`).
    pkg.add_enum(RegistryEnum {
        name: "LineStyle",
        export: true,
        variants: vec![
            EnumVariant {
                name: "Light",
                description: "Thin single line (─ │).",
                advisory: None,
            },
            EnumVariant {
                name: "Heavy",
                description: "Thick single line (━ ┃).",
                advisory: None,
            },
            EnumVariant {
                name: "LightDash",
                description: "Thin triple-dash line (┄ ┆).",
                advisory: None,
            },
            EnumVariant {
                name: "HeavyDash",
                description: "Thick triple-dash line (┅ ┇).",
                advisory: None,
            },
            EnumVariant {
                name: "LightDot",
                description: "Thin quadruple-dot line (┈ ┊).",
                advisory: None,
            },
            EnumVariant {
                name: "HeavyDot",
                description: "Thick quadruple-dot line (┉ ┋).",
                advisory: None,
            },
            EnumVariant {
                name: "Double",
                description: "Double line (═ ║).",
                advisory: None,
            },
        ],
    });
    // The block or shade glyph `term::fillRect` stamps into every cell of a
    // rectangular region (`Filled = 0` … `CheckerAlt = 5`).
    pkg.add_enum(RegistryEnum {
        name: "FillStyle",
        export: true,
        variants: vec![
            EnumVariant {
                name: "Filled",
                description: "Solid full block (█).",
                advisory: None,
            },
            EnumVariant {
                name: "Light",
                description: "Light shade (░).",
                advisory: None,
            },
            EnumVariant {
                name: "Medium",
                description: "Medium shade (▒).",
                advisory: None,
            },
            EnumVariant {
                name: "Dark",
                description: "Dark shade (▓).",
                advisory: None,
            },
            EnumVariant {
                name: "Checker",
                description: "Upper-left + lower-right quadrants (▚).",
                advisory: None,
            },
            EnumVariant {
                name: "CheckerAlt",
                description: "Upper-right + lower-left quadrants (▞).",
                advisory: None,
            },
        ],
    });

    // The native members (each registers the shared `abi_function` body over the
    // shared terminal codegen carrier).
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

    // The `term`↔`astrings` `drawText(AttributedString)` bridge — a cross-package
    // gated helper chunk (see `helper_astrings_bridge.rs` for why it is not a
    // `Body::mfb` overload).
    helper_astrings_bridge::register(&mut pkg);

    r.add_package(pkg);
}
