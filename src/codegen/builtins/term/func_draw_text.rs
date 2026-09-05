//! `term::drawText` — abi_function member (native terminal I/O).
//!
//! Registers its own [`lower_draw_text`] `Body::abi_function`
//! body; the `abi_function` wrapper finalizes it. The heavy terminal emission stays
//! in the shared code layer (`code::lower_term_helper` / `emit_app_term_helper`).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Draw a string at a position without moving the cursor"#;

const DESC: &str = r#"`term::drawText` stamps `text` onto the surface on `row` starting at `column`,
one grid position per **grapheme cluster** — what a reader calls a character, so
`e` plus a combining acute occupies one cell, and an emoji built from joined
scalars occupies one position — using the colours and attributes currently in
effect. A cluster that the terminal draws **double-width** (most CJK, most emoji)
occupies **two** columns; if only one column is left before the right edge, that
cluster is **dropped and the run stops** — a wide glyph is never split across the
edge, and `drawText` never wraps to the next row.

Coordinates are **zero-based** from the top-left, and — like every other
`term::` position — are written **row first, then column**. Unlike
`io::print`/`io::write`, it **does not move the cursor**, so it is the tool for
placing a label, status line, or field value at a fixed position without
disturbing cursor-relative output.

The text is drawn on a **single row**: it does not wrap and does not scroll.
Characters that fall past the right edge are **clipped**, and columns before 0
(when `column` is negative) are skipped, so only the on-screen part is drawn. If
`row` is outside `0 .. rows-1` the call draws nothing. Control characters (below
U+0020, including newline and tab) are **skipped** — they advance one column but
stamp nothing — so a stray control character can never corrupt the presented
frame; use `io::write` for flowing text with newline handling. The run is shown on
the next `term::sync`.

The call is gated: while TUI mode is off it does nothing and reports no
error (in a Linux or Windows `mfb build --app` build the gate is
not enforced — see `mfb man term`).

**Two app-mode gaps apply to this call** (see `mfb man term`). In a **Linux**
`--app` build it is not implemented and stamps nothing; a Linux terminal is
unaffected. A **Windows** `--app` build draws it but does not cluster: it
stamps one grid position per Unicode scalar, so a combining mark or a joined emoji
takes its own cells there, and it does not apply the "drop a wide cluster that
would not fit" rule — a double-width scalar in the last column is drawn rather
than dropped. The console backend on every platform, and macOS app mode, behave
exactly as described above.

An overload accepts an `astrings::AttributedString` in the `text` position. It
stamps the same visible text as the `String` overload but honours the per-scalar
styling the value carries: the attributes the terminal surface can represent —
**bold**, **underline**, **foreground colour** and **background colour** — are
applied per run, and every other attribute (italic, strikethrough, overline, font,
font size) is silently ignored. Grapheme-cluster and wide-glyph handling is
identical to the `String` overload. Your own current bold, underline, foreground
and background settings are put back afterwards, so like the `String` overload the
call leaves the pen it found. Using this overload requires `IMPORT astrings` (the
only way to build an `AttributedString`)."#;

const EX: &str = r#"Draw a title and a status line at fixed positions:

```
IMPORT term

SUB main()
  term::on()
  term::drawText(0, 2, "My Application")
  LET size AS term::TermSize = term::terminalSize()
  term::drawText(size.rows - 1, 0, "Press q to quit")
  term::sync()
  term::off()
END SUB
```

Draw styled text, applying its bold/underline attributes:

```
IMPORT term
IMPORT astrings

SUB main()
  term::on()
  MUT label AS AttributedString = astrings::fromString("Save  Quit")
  label = astrings::addAttribute(label, 0, 3, astrings::bold())
  label = astrings::addAttribute(label, 6, 9, astrings::underline())
  term::drawText(0, 2, label)
  term::sync()
  term::off()
END SUB
```"#;

/// `abi_function` body for `term::draw_text` — delegates to the shared family-generic
/// [`super::gen_shared::lower_term_helper`] with its own runtime-call name (the
/// app-vs-console dispatch and the heavy per-member emitters live in the shared code
/// layer).
pub(crate) fn lower_draw_text(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) = super::gen_shared::lower_term_helper(
        ctx.call,
        &symbol,
        ctx.term_state_offset,
        ctx.presentation_mode_offset,
        ctx.build_mode,
        ctx.platform_imports,
        ctx.platform,
    )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "drawText",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Integer, Integer, String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "row",
                    desc: "Zero-based row, counting from 0 at the top. Outside `0 .. rows-1` the call draws nothing.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "column",
                    desc: "Zero-based start column, counting from 0 at the left. Negative columns are skipped; the run clips at the right edge.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "text",
                    desc: "The text to stamp, one grid position per grapheme cluster (a double-width cluster takes two columns). Control characters are skipped. An `AttributedString` additionally applies its per-scalar bold, underline, foreground and background (other attributes ignored).",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_draw_text),
        }],
    });
}
