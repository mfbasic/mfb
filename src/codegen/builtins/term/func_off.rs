//! `term::off` — abi_function member (native terminal I/O).
//!
//! Registers its own [`lower_off`] `Body::abi_function`
//! body; the `abi_function` wrapper finalizes it. The heavy terminal emission stays
//! in the shared code layer (`code::lower_term_helper` / `emit_app_term_helper`).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Leave TUI mode: present the final frame and restore the terminal"#;

const DESC: &str = r#"`term::off` tears down the TUI surface entered by `term::on` and returns the
terminal to the state it had before. It takes no arguments and is gated: while
TUI mode is already off the call does nothing at all and reports success. (In a
Linux or Windows `mfb build --app` build that idle-off case is not fully inert —
it still asks the window to restore itself — which is harmless but is why the
package overview lists the app-mode gate as a known gap.)

When TUI mode is on, the teardown runs in this order.

1. **A final `term::sync`.** `term::off` calls the present routine itself, so the
   last frame the program composed is displayed even if it never called
   `term::sync` explicitly.
2. **Normal line input is restored**, undoing the single-key mode `term::on` put
   the terminal into, so typing echoes again and lines are submitted with
   Return. Nothing happens here if that mode was never entered.
3. **The terminal is restored**: the cursor is made visible, the alternate screen
   is left so the user's previous shell contents reappear, and the terminal's
   colour and attribute state is reset so ordinary output that follows is drawn
   normally.
4. **TUI mode is switched off** and the drawing surface goes away.

After `term::off` returns, `term::isOn` reports `FALSE` and every `term::` call
except `term::on` and `term::isOn` is a no-op again. A later `term::on` starts
over with a fresh surface and the default state; nothing drawn before
`term::off` survives it.

Because the alternate screen and the terminal's line discipline are both process
state, a program that enters TUI mode should reach `term::off` on every exit path
— including its error paths — or leave the user's terminal in single-key mode on
the alternate screen."#;

const EX: &str = r#"Draw one frame and restore the terminal:

```
IMPORT term
IMPORT io

SUB main()
  term::on()
  term::moveTo(0, 0)
  io::print("done")
  term::off()          ' presents the frame, then restores the screen
END SUB
```

Leave TUI mode only if it was entered:

```
IMPORT term

SUB main()
  IF term::isOn() THEN
    term::off()
  END IF
END SUB
```"#;
/// `abi_function` body for `term::off` — delegates to the shared family-generic
/// [`super::gen_shared::lower_term_helper`] with its own runtime-call name (the
/// app-vs-console dispatch and the heavy per-member emitters live in the shared code
/// layer).
pub(crate) fn lower_off(
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
        name: "off",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_off),
        }],
    });
}
