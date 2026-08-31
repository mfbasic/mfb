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

const DESC: &str = r#"`term::setForeground` sets the 24-bit RGB colour that subsequent text drawn
through the `term::` surface will be written in. The three channels — red, green,
blue — are each a `Byte` from 0 to 255, so (0, 0, 0) is black, (255, 255, 255) is
white, and (255, 0, 0) is pure red. Exactly three arguments are required.

The colour is packed into the module's current-attribute state and **no escape
sequence is emitted**. Like every other drawing operation on this retained
surface, the effect becomes visible only when `term::sync` presents the frame.

Colour is per cell, not global. Each cell of the grid records the foreground,
background, bold, and underline that were current when its glyph was written, so
changing the foreground affects only text drawn *after* the call — text already in
the surface keeps the colour it was drawn with, and is not restyled.

The setting persists until the next `term::setForeground` or the next
`term::on`, which resets the foreground to white (255, 255, 255). The background
colour and the bold and underline attributes are independent and are left
untouched; the current value can be read back with `term::getForeground`.

The call is gated: while TUI mode is off it does nothing and reports no error."#;

const EX: &str = r#"Draw red text and present the frame:

```
IMPORT term
IMPORT io

SUB main()
  term::on()
  term::setForeground(255, 0, 0)
  io::print("hello in red")
  term::sync()
  term::off()
END SUB
```

Two colours in one frame — the first line keeps its colour:

```
IMPORT term
IMPORT io

SUB main()
  term::on()
  term::setForeground(0, 255, 0)
  io::print("green")
  term::setForeground(0, 128, 255)
  io::print("blue")
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
        expected_arguments: Some("Byte, Byte, Byte"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "r",
                    desc: "Red channel, 0 to 255.",
                    aliases: &[],
                    ty: ParameterType::Byte,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "g",
                    desc: "Green channel, 0 to 255.",
                    aliases: &[],
                    ty: ParameterType::Byte,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "b",
                    desc: "Blue channel, 0 to 255.",
                    aliases: &[],
                    ty: ParameterType::Byte,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_set_foreground),
        }],
    });
}
