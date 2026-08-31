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

const DESC: &str = r#"`term::drawText` stamps `text` onto the surface on row `y` starting at column `x`,
one grid cell per Unicode scalar, using the colours and attributes currently in
effect. Coordinates are **zero-based** from the top-left (`x` is the column, `y`
the row). Unlike `io::print`/`io::write`, it **does not move the cursor**,
so it is the tool for placing a label, status line, or field value at a fixed
position without disturbing cursor-relative output.

The text is drawn on a **single row**: it does not wrap and does not scroll.
Characters that fall past the right edge are **clipped**, and columns before 0
(when `x` is negative) are skipped, so only the on-screen part is drawn. If `y` is
outside `0 .. rows-1` the call draws nothing. Control characters (below U+0020,
including newline and tab) are **skipped** — they advance one column but stamp
nothing — so a stray control character can never corrupt the presented frame; use
`io::write` for flowing text with newline handling. The run is shown on the next
`term::sync`.

The call is gated: while TUI mode is off it does nothing and reports no error.

An overload accepts an `astrings::AttributedString` in the `text` position. It
stamps the same visible text as the `String` overload but honours the per-scalar
styling the value carries: the two attributes the terminal surface can represent —
**bold** and **underline** — are applied per run, and every other attribute
(italic, strikethrough, overline, font, font size) is silently ignored. The text
carries its own bold and underline as it is drawn, and grapheme-cluster and
wide-glyph handling is identical to the `String` overload. Your own current
bold and underline settings are put back afterwards, so like the `String`
overload the call leaves the pen it found. Using
this overload requires `IMPORT astrings` (the only way to build an
`AttributedString`)."#;

const EX: &str = r#"Draw a title and a status line at fixed positions:

```
IMPORT term

SUB main()
  term::on()
  term::drawText(2, 0, "My Application")
  LET size AS TermSize = term::terminalSize()
  term::drawText(0, size.rows - 1, "Press q to quit")
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
  term::drawText(2, 0, label)
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
                    name: "x",
                    desc: "Zero-based start column. Negative columns are skipped; the run clips at the right edge.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "y",
                    desc: "Zero-based row. Outside `0 .. rows-1` the call draws nothing.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "text",
                    desc: "The text to stamp, one cell per Unicode scalar. Control characters are skipped. An `AttributedString` additionally applies its per-scalar bold/underline (other attributes ignored).",
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
