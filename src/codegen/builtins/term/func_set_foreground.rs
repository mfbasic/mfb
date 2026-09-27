//! `term::setForeground` — abi_function member (native terminal I/O).
//!
//! Registers its own [`lower_set_foreground`] `Body::abi_function`
//! body; the `abi_function` wrapper finalizes it. The heavy terminal emission stays
//! in the shared code layer (`code::lower_term_helper` / `emit_app_term_helper`).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Set the foreground colour used for subsequently drawn text"#;

const DESC: &str = r#"`term::setForeground` sets the colour that subsequent text drawn through the
`term::` surface will be written in, from a single `color::Color`.

**A program calling this must `IMPORT color`** as well as `term` — imports are not
transitive and a package cannot re-export another's types.

**`alpha` is ignored.** A terminal cell has no alpha channel, so a
half-transparent colour draws exactly the cells an opaque one draws. The alpha is
not an error and is not clamped away; it simply has nowhere to go. Synthesizing a
blend against whatever is already in the cell would disagree with what a canvas
surface draws for the same colour, so the terminal does not attempt one.

Nothing is written to the terminal by this call. Like every other drawing
operation on this surface, the effect becomes visible only when `term::sync`
presents the frame. While TUI mode is on, every call that writes cells takes the
current foreground colour: `io::print` and `io::write`, and `term::drawText`, `drawGlyph`, `drawHLine`,
`drawVLine`, `drawBox` and `fillRect`.

Colour is per cell, not global. Each cell of the grid records the foreground,
background, bold, and underline that were current when its glyph was written, so
changing the foreground affects only text drawn *after* the call — text already in
the surface keeps the colour it was drawn with, and is not restyled.

The setting lasts for the current TUI session: it persists until the next
`term::setForeground`, and is reset to white (255, 255, 255) by `term::off` and again
by `term::on`. The background colour and the bold and underline
attributes are independent and are left untouched; the current value can be read
back with `term::getForeground`. One exception: `term::drawText` with an
`AttributedString` uses the colour the string carries for each run that sets one,
and this setting for runs that do not; it restores this setting when it finishes.

While TUI mode is off the call is ignored and raises nothing; `term::getForeground`
reports white (`#ffffffff`) until the next `term::on`. In an `--app`
program outside `app::Mode.Console` it raises `ErrWrongMode`, as every `term::`
call does."#;

const EX: &str = r#"Draw coloured text and present the frame:

```
IMPORT term
IMPORT color
IMPORT io

SUB main()
  term::on()
  term::setForeground(color::rgb(255, 0, 0))
  io::print("alert")
  term::sync()
  term::off()
END SUB
```

The red, green and blue channels round-trip through `term::getForeground`, so saving
and restoring needs no channel unpacking. The colour read back always carries an
alpha of 255, whatever alpha was passed in:

```
IMPORT term
IMPORT color

SUB main()
  term::on()
  LET saved AS color::Color = term::getForeground()
  term::setForeground(color::fromName("teal"))
  term::drawText(0, 0, "teal")
  term::setForeground(saved)
  term::drawText(1, 0, "default again")
  term::sync()
  term::off()
END SUB
```"#;

/// `abi_function` body for `term::set_foreground` — delegates to the shared family-generic
/// [`super::gen_shared::lower_term_helper`] with its own runtime-call name (the
/// app-vs-console dispatch and the heavy per-member emitters live in the shared code
/// layer).
pub(crate) fn lower_set_foreground(
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
        ctx.mouse_state_offset,
        ctx.input_state_offset,
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
        name: "setForeground",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("color::Color"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "base",
                desc: "The colour to draw in. Its alpha is ignored — a terminal \
                       cell has no alpha channel.",
                aliases: &[],
                ty: ParameterType::named(crate::codegen::builtins::color::COLOR_TYPE_ID),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_set_foreground),
        }],
    });
}
