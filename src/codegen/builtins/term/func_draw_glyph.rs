//! `term::drawGlyph` — abi_function member (native terminal I/O).
//!
//! Registers its own [`lower_draw_glyph`] `Body::abi_function`
//! body; the `abi_function` wrapper finalizes it. The heavy terminal emission stays
//! in the shared code layer (`code::lower_term_helper` / `emit_app_term_helper`).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Stamp a single glyph at a position by code point"#;

const DESC: &str = r#"`term::drawGlyph` stamps a single Unicode scalar — given by its `codepoint` — into
the cell at `row`/`column`, using the colours and attributes currently in effect.
Coordinates are **zero-based** from the top-left and, like every other `term::`
position, are written **row first, then column**. It does not move the cursor.
This is the low-level counterpart to `term::drawText`: use it to place one
arbitrary character (a marker, a cursor, a sprite cell) at a known position.

The cell is **bounds-checked, not clamped**: if `(row, column)` is off the surface
the call draws nothing and raises no error. It does not fall back to the nearest
edge cell, so an off-by-one in your coordinates loses the glyph silently rather
than putting it somewhere visible. Control code points (below U+0020) are **skipped**
— they would corrupt the presented frame — so `codepoint` should be a printable
scalar (for example `9731` for `☃`, or `65` for `A`). The glyph is shown on the
next `term::sync`.

The call is gated: while TUI mode is off it does nothing and reports no
error (in a Linux or Windows `mfb build --app` build the gate is
not enforced — see `mfb man term`).

**One app-mode gap applies to this call** (see `mfb man term`): in a **Linux**
`--app` build it is not implemented and stamps nothing. A Linux terminal, macOS
app mode and Windows app mode all draw it."#;

const EX: &str = r#"Place a marker character at the centre of the surface:

```
IMPORT term

SUB main()
  term::on()
  LET size AS term::TermSize = term::terminalSize()
  term::drawGlyph(size.rows / 2, size.columns / 2, 9731) ' ☃
  term::sync()
  term::off()
END SUB
```"#;

/// `abi_function` body for `term::draw_glyph` — delegates to the shared family-generic
/// [`super::gen_shared::lower_term_helper`] with its own runtime-call name (the
/// app-vs-console dispatch and the heavy per-member emitters live in the shared code
/// layer).
pub(crate) fn lower_draw_glyph(
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
        name: "drawGlyph",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Integer, Integer, Integer"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "row",
                    desc: "Zero-based row, counting from 0 at the top. Off-grid cells draw nothing.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "column",
                    desc: "Zero-based column, counting from 0 at the left. Off-grid cells draw nothing.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "codepoint",
                    desc: "The Unicode scalar to stamp. Control code points (< 0x20) are skipped.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_draw_glyph),
        }],
    });
}
