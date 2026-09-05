//! `term::drawVLine` — abi_function member (native terminal I/O).
//!
//! Registers its own [`lower_draw_vline`] `Body::abi_function`
//! body; the `abi_function` wrapper finalizes it. The heavy terminal emission stays
//! in the shared code layer (`code::lower_term_helper` / `emit_app_term_helper`).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Draw a vertical box-drawing line down a column of the surface"#;

const DESC: &str = r#"`term::drawVLine` stamps a vertical run of a box-drawing glyph into the retained
surface: on `column`, it fills every row from `rowA` to `rowB` with the
vertical form of the chosen `term::LineStyle`. The glyph is drawn with the colours and
attributes currently in effect (`term::setForeground`/`setBackground`/`setBold`/
`setUnderline`), exactly as `io::write` stamps text, and — like every drawing call
on this surface — it updates the surface only and appears on the next
`term::sync`.

Coordinates are **zero-based** and measured from the top-left corner: row 0 is the
topmost line and column 0 is the leftmost column. Like every `term::` position, the
run's start is written **row first, then column** (`rowA`, `column`), and `rowB` is
the far end of the run. The two row endpoints may be given in **either order** —
`rowA` and `rowB` are normalised so the lower one starts the run — and the run is
**inclusive of both ends**. The span is then **clamped to the surface**: a negative
start becomes 0 and an end past the bottom edge becomes `rows-1`. If `column` is
outside `0 .. columns-1`, or the clamped span
covers no on-grid cell, the call draws nothing rather than clamping the line onto
an edge. No error is raised for an out-of-range request.

The `line` argument is a `term::LineStyle` enum value selecting the weight and pattern:
`term::LineStyle.Light` (`│`), `term::LineStyle.Heavy` (`┃`), `term::LineStyle.LightDash` (`┆`),
`term::LineStyle.HeavyDash` (`┇`), `term::LineStyle.LightDot` (`┊`), `term::LineStyle.HeavyDot`
(`┋`), and `term::LineStyle.Double` (`║`). `term::drawHLine` draws the matching
horizontal forms.

Drawing a line does not move the cursor and does not change the current
colours or attributes; it overwrites only the cells in the run, so a later draw
over the same cell (for example a crossing horizontal line) wins. The same surface
is rendered on the console backend and in windowed app mode.

**Two app-mode gaps apply to this call** (see `mfb man term`). In a **Linux**
`--app` build it is not implemented and draws nothing; a Linux terminal is
unaffected. In a **Windows** `--app` build it draws, but ignores `line` and always
uses the `Light` glyph. The console backend on every platform, and macOS app mode,
honour the style.

The call is gated: while TUI mode is off it does nothing and reports no
error (in a Linux or Windows `mfb build --app` build the gate is
not enforced — see `mfb man term`)."#;

const EX: &str = r#"Draw a double vertical rule down the left edge of the surface:

```
IMPORT term

SUB main()
  term::on()
  LET size AS term::TermSize = term::terminalSize()
  term::drawVLine(term::LineStyle.Double, 0, 0, size.rows - 1)
  term::sync()
  term::off()
END SUB
```

Draw a vertical divider between two panes:

```
IMPORT term

SUB main()
  term::on()
  term::drawVLine(term::LineStyle.Light, 0, 40, 23)
  term::sync()
  term::off()
END SUB
```"#;

/// `abi_function` body for `term::draw_vline` — delegates to the shared family-generic
/// [`super::gen_shared::lower_term_helper`] with its own runtime-call name (the
/// app-vs-console dispatch and the heavy per-member emitters live in the shared code
/// layer).
pub(crate) fn lower_draw_vline(
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
        name: "drawVLine",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("LineStyle, Integer, Integer, Integer"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "line",
                    desc: "The box-drawing weight/pattern; its vertical form is used.",
                    aliases: &[],
                    ty: ParameterType::named("LineStyle"),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "rowA",
                    desc: "One end of the row span (inclusive); may be greater or less than `rowB`. Clamped to the surface.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "column",
                    desc: "Zero-based column the line is drawn on, counting from 0 at the left. Outside `0 .. columns-1` the call draws nothing.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "rowB",
                    desc: "The other end of the row span (inclusive). Clamped to the surface.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_draw_vline),
        }],
    });
}
