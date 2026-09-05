//! `term::setBackground` — abi_function member (native terminal I/O).
//!
//! Registers its own [`lower_set_background`] `Body::abi_function`
//! body; the `abi_function` wrapper finalizes it. The heavy terminal emission stays
//! in the shared code layer (`code::lower_term_helper` / `emit_app_term_helper`).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Set the background colour used for subsequently drawn text"#;

const DESC: &str = r#"`term::setBackground` sets the colour that subsequent text drawn through the
`term::` surface will be written in, from a single `color::Color`.

**A program calling this must `IMPORT color`** as well as `term` — imports are not
transitive and a package cannot re-export another's types.

**`alpha` is ignored.** A terminal cell has no alpha channel, so a
half-transparent colour draws exactly the cells an opaque one draws. The alpha is
not an error and is not clamped away; it simply has nowhere to go. Synthesizing a
blend against whatever is already in the cell would disagree with what a canvas
surface draws for the same colour, so the terminal does not attempt one.

The colour is packed into the module's current-attribute state and **no escape
sequence is emitted**. Like every other drawing operation on this retained
surface, the effect becomes visible only when `term::sync` presents the frame.

Colour is per cell, not global. Each cell of the grid records the foreground,
background, bold, and underline that were current when its glyph was written, so
changing the background affects only text drawn *after* the call — text already in
the surface keeps the colour it was drawn with, and is not restyled.

The setting persists until the next `term::setBackground` or the next `term::on`,
which resets the background to black (0, 0, 0). The foreground colour and the bold and underline
attributes are independent and are left untouched; the current value can be read
back with `term::getBackground`.

The call is gated: while TUI mode is off it does nothing and reports no
error (in a Linux or Windows `mfb build --app` build the gate is
not enforced — see `mfb man term`)."#;

const EX: &str = r#"Draw coloured text and present the frame:

```
IMPORT term
IMPORT color
IMPORT io

SUB main()
  term::on()
  term::setBackground(color::rgb(255, 0, 0))
  io::print("alert")
  term::sync()
  term::off()
END SUB
```

The colour round-trips through `term::getBackground`, so saving and restoring needs no
channel unpacking:

```
IMPORT term
IMPORT color

SUB main()
  term::on()
  LET saved AS color::Color = term::getBackground()
  term::setBackground(color::fromName("teal"))
  term::setBackground(saved)
  term::off()
END SUB
```"#;

/// `abi_function` body for `term::set_background` — delegates to the shared family-generic
/// [`super::gen_shared::lower_term_helper`] with its own runtime-call name (the
/// app-vs-console dispatch and the heavy per-member emitters live in the shared code
/// layer).
pub(crate) fn lower_set_background(
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
        name: "setBackground",
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
            body: Body::abi_function(lower_set_background),
        }],
    });
}
