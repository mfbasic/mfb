//! `term::setBold` — abi_function member (native terminal I/O).
//!
//! Registers its own [`lower_set_bold`] `Body::abi_function`
//! body; the `abi_function` wrapper finalizes it. The heavy terminal emission stays
//! in the shared code layer (`code::lower_term_helper` / `emit_app_term_helper`).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Turn the bold attribute on or off for subsequently drawn text"#;

const DESC: &str = r#"`term::setBold` sets whether text drawn through the `term::` surface from now on
is bold. It takes exactly one `Boolean`: `TRUE` enables the attribute, `FALSE`
disables it.

The flag is stored in the module's current-attribute state and **no escape
sequence is emitted**. Like every other drawing operation on this retained
surface, the change becomes visible only when `term::sync` presents the frame.

Boldness is per cell, not global. Each cell of the grid records the foreground,
background, bold, and underline that were current when its glyph was written, so
this call affects text drawn *after* it; text already on the surface keeps the
attributes it was drawn with and is not restyled.

The setting persists until the next `term::setBold` or the next `term::on`, which
resets bold to off. It is independent of the foreground and background colours and
of underline, so changing it leaves those alone, and the current value can be read
back with `term::getBold`. Setting the same value twice is harmless — the state is
a flag, not a toggle.

The call is gated: while TUI mode is off it does nothing and reports no
error (in a Linux or Windows `mfb build --app` build the gate is
not enforced — see `mfb man term`)."#;

const EX: &str = r#"Draw a bold heading above plain body text:

```
IMPORT term
IMPORT io

SUB main()
  term::on()
  term::setBold(TRUE)
  io::print("Heading")
  term::setBold(FALSE)
  io::print("body text")
  term::sync()
  term::off()
END SUB
```"#;

/// `abi_function` body for `term::set_bold` — delegates to the shared family-generic
/// [`super::gen_shared::lower_term_helper`] with its own runtime-call name (the
/// app-vs-console dispatch and the heavy per-member emitters live in the shared code
/// layer).
pub(crate) fn lower_set_bold(
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
        name: "setBold",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Boolean"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "enabled",
                desc: "`TRUE` to draw subsequent text bold, `FALSE` to draw it normally.",
                aliases: &[],
                ty: ParameterType::Boolean,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_set_bold),
        }],
    });
}
