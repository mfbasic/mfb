//! `term::hideCursor` — abi_function member (native terminal I/O).
//!
//! Registers its own [`lower_hide_cursor`] `Body::abi_function`
//! body; the `abi_function` wrapper finalizes it. The heavy terminal emission stays
//! in the shared code layer (`code::lower_term_helper` / `emit_app_term_helper`).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Hide the terminal cursor in presented frames"#;

const DESC: &str = r#"`term::hideCursor` marks the cursor as hidden. It takes no arguments.

Like everything else on this retained surface, the call **emits no escape
sequence**. It clears a single visibility flag in the module's state; the terminal
is only told about it when `term::sync` presents a frame, whose trailing sequence
shows or hides the cursor according to this flag. Until the next present the
cursor stays as the previous frame left it.

Hiding the cursor is the usual choice for a full-screen program that repaints
every frame: the terminal cursor would otherwise be parked at whatever cell the
last write ended on, blinking in the middle of the drawing.

Visibility is independent of the colours and text attributes and of the cursor's
position — hiding it does not move it, and `term::moveTo` still works normally
while it is hidden. Calling `term::hideCursor` twice is harmless, since this is a
flag rather than a toggle.

The flag persists until `term::showCursor` or the next `term::on`, which resets
the cursor to visible; `term::off` also makes the cursor visible again as part of
restoring the terminal. The call is gated: while TUI mode is off it does nothing (in a Linux or Windows `mfb build --app` build the gate is
not enforced — see `mfb man term`)
and reports no error."#;

const EX: &str = r#"Draw a full-screen frame with no blinking cursor:

```
IMPORT term
IMPORT io

SUB main()
  term::on()
  term::hideCursor()
  term::clear()
  term::moveTo(0, 0)
  io::print("drawing without a blinking cursor")
  term::sync()
  term::off()
END SUB
```"#;
/// `abi_function` body for `term::hide_cursor` — delegates to the shared family-generic
/// [`super::gen_shared::lower_term_helper`] with its own runtime-call name (the
/// app-vs-console dispatch and the heavy per-member emitters live in the shared code
/// layer).
pub(crate) fn lower_hide_cursor(
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
        name: "hideCursor",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_hide_cursor),
        }],
    });
}
