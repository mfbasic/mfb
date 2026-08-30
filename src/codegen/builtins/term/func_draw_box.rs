//! `term::drawBox` — abi_function member (native terminal I/O).
//!
//! Registers its own [`lower_draw_box`] `Body::abi_function`
//! body; the `abi_function` wrapper finalizes it. The heavy terminal emission stays
//! in the shared code layer (`code::lower_term_helper` / `emit_app_term_helper`).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Draw a rectangular box in a box-drawing style"#;

const DESC: &str = r#"`term::drawBox` draws a rectangle into the retained surface in the chosen
`LineStyle`. The two points `(x1, y1)` and `(x2, y2)` are **opposite corners** —
`x` is the column and `y` is the row, both **zero-based** from the top-left — and
they may be given in any order. The box is drawn as the four edges followed by the
four corners: the top and bottom rows are horizontal runs and the left and right
columns are vertical runs, each using this style's own line glyph, and then the
four corner cells are overwritten with the matching corner glyph. Everything is
stamped with the colours and attributes currently in effect and shown on the next
`term::sync`.

Because the edges use the style's line glyph, a **dashed or dotted** style draws
dashed or dotted edges — but those styles have no dashed corner glyphs, so the
corners fall back to the solid **Light** or **Heavy** corner of the same weight
(`Double` uses the double corners). So `LineStyle.LightDash` draws `┄`/`┆` edges
with `┌┐└┘` corners, and `LineStyle.HeavyDot` draws `┉`/`┋` edges with `┏┓┗┛`
corners.

Each edge and each corner is **clamped to the surface independently**, so a box
that runs off one side still draws the parts that are on-screen (including the
edges along the visible sides), and a box entirely off the surface draws nothing.
No error is raised for an out-of-range request. A one-cell-wide or one-cell-tall
box collapses to a line or a single cell, with the corners drawn last. The same
surface renders identically on the console and in windowed app mode.

The call is gated: while TUI mode is off it does nothing and reports no error."#;

const EX: &str = r#"Draw a light box near the top-left corner:

```
IMPORT term

SUB main()
  term::on()
  term::drawBox(LineStyle.Light, 2, 1, 20, 8)
  term::sync()
  term::off()
END SUB
```

Frame the whole surface with a double-line border:

```
IMPORT term

SUB main()
  term::on()
  LET size AS TermSize = term::terminalSize()
  term::drawBox(LineStyle.Double, 0, 0, size.columns - 1, size.rows - 1)
  term::sync()
  term::off()
END SUB
```"#;

/// `abi_function` body for `term::draw_box` — delegates to the shared family-generic
/// [`super::gen_shared::lower_term_helper`] with its own runtime-call name (the
/// app-vs-console dispatch and the heavy per-member emitters live in the shared code
/// layer).
pub(crate) fn lower_draw_box(
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
        name: "drawBox",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("LineStyle, Integer, Integer, Integer, Integer"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "line",
                    desc: "The box-drawing style; the edges use its line glyph and the corners the matching Light/Heavy/Double corner.",
                    aliases: &[],
                    ty: ParameterType::named("LineStyle"),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "x1",
                    desc: "Column of the first corner (zero-based). Clamped to the surface.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "y1",
                    desc: "Row of the first corner (zero-based). Clamped to the surface.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "x2",
                    desc: "Column of the opposite corner; may be less or greater than `x1`.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "y2",
                    desc: "Row of the opposite corner; may be less or greater than `y1`.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_draw_box),
        }],
    });
}
