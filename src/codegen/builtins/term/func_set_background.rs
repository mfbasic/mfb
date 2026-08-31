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

const DESC: &str = r#"`term::setBackground` sets the 24-bit RGB colour drawn behind subsequent text on
the `term::` surface. The three channels — red, green, blue — are each a `Byte`
from 0 to 255, so (0, 0, 0) is black and (255, 255, 255) is white. Exactly three
arguments are required.

The colour is packed into the module's current-attribute state and **no escape
sequence is emitted**; the effect becomes visible when `term::sync` presents the
frame.

Background colour is per cell, and it colours only the cells that drawn text
occupies. Each cell records the attributes current when its glyph was written, so
this call affects text drawn *after* it and does not restyle what is already in
the surface. In particular, **`term::clear` does not paint the current
background**: it blanks to black regardless of this setting.
To get a coloured region, set the background and then draw over it — for example
by writing spaces across the cells you want filled.

The setting persists until the next `term::setBackground` or the next `term::on`,
which resets the background to black (0, 0, 0). The foreground colour and the
bold and underline attributes are independent and are left untouched; the current
value can be read back with `term::getBackground`.

The call is gated: while TUI mode is off it does nothing and reports no error."#;

const EX: &str = r#"Draw text on a blue background:

```
IMPORT term
IMPORT io

SUB main()
  term::on()
  term::setBackground(0, 0, 255)
  io::print("hello on blue")
  term::sync()
  term::off()
END SUB
```

Fill a banner row by drawing spaces over it:

```
IMPORT term
IMPORT io
IMPORT strings

SUB main()
  term::on()
  LET size AS TermSize = term::terminalSize()
  term::setBackground(0, 0, 128)
  term::moveTo(0, 0)
  io::write(strings::repeat(" ", size.columns))
  term::sync()
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
            body: Body::abi_function(lower_set_background),
        }],
    });
}
