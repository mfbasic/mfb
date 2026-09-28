//! `term::showCursor` — abi_function member (native terminal I/O).
//!
//! Registers its own [`lower_show_cursor`] `Body::abi_function`
//! body; the `abi_function` wrapper finalizes it. The heavy terminal emission stays
//! in the shared code layer (`code::lower_term_helper` / `emit_app_term_helper`).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Make the terminal cursor visible in presented frames"#;

const DESC: &str = r#"`term::showCursor` marks the cursor as visible. It takes no arguments.

The change is not shown straight away. In a terminal program the cursor's
visibility changes when `term::sync` presents the next frame; a Linux `--app`
build repaints from this call itself. A macOS or Windows `--app` build accepts the
call and remembers the setting, but draws no cursor at all, so it has no visible
effect there. Each present leaves the terminal cursor at the position the next
drawing would use, so a visible cursor always tracks that position.

Visibility is independent of the colours and text attributes and of the cursor's
position: showing the cursor changes none of them, and `term::moveTo` does not
change visibility. Calling `term::showCursor` when the cursor is already visible
is harmless — it is a flag, not a toggle.

The flag persists until `term::hideCursor` or the next `term::on`, which resets
the cursor to visible. The call is gated: while TUI mode is off it does nothing
and reports no error. In an `mfb build --app` program, `term::showCursor` — like every other `term::` call — raises `ErrWrongMode` unless the program is in `app::Mode.Console`; see `mfb man app`."#;

const EX: &str = r#"Hide the cursor while a frame is drawn, then show it again for input:

```
IMPORT term
IMPORT io

FUNC main() AS Nothing
  term::on()
  term::hideCursor()
  term::clear()
  io::print("rendering...")
  term::sync()

  term::showCursor()
  term::moveTo(2, 0)
  io::write("Name: ")
  term::sync()
  term::off()
END FUNC
```"#;
/// `abi_function` body for `term::show_cursor` — delegates to the shared family-generic
/// [`super::gen_shared::lower_term_helper`] with its own runtime-call name (the
/// app-vs-console dispatch and the heavy per-member emitters live in the shared code
/// layer).
pub(crate) fn lower_show_cursor(
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
        name: "showCursor",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Nothing,
            errors: vec!["ErrWrongMode"],
            body: Body::abi_function(lower_show_cursor),
        }],
    });
}
