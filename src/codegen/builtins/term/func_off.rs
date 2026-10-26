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
TUI mode is already off it leaves the screen and the input mode alone and does
not raise. One thing still happens in a terminal either way: if the program
turned mouse reporting on with `term::enableMouse` and never turned it off,
`term::off` turns it off, whether or not TUI mode was on.

When TUI mode is on, the teardown runs in this order.

1. **A final `term::sync`.** `term::off` calls the present routine itself, so the
   last frame the program composed is displayed even if it never called
   `term::sync` explicitly.
2. **Normal line input is restored**, undoing the single-key mode `term::on` put
   the terminal into, so typing echoes again and lines are submitted with
   Return, and any full key reporting `term::on` turned on is turned back off.
   Nothing happens here if that mode was never entered.
3. **The terminal is restored**: the cursor is made visible, the alternate screen
   is left so the user's previous shell contents reappear, and the terminal's
   colour and attribute state is reset so ordinary output that follows is drawn
   normally.
4. **TUI mode is switched off** and the drawing surface goes away.
5. **Mouse reporting is turned off**, if `term::enableMouse` turned it on and
   the program never turned it off.

In an `mfb build --app` window there is no cursor or alternate screen to restore:
after the final present, the window swaps back to its scrolling text view, which
regains keyboard focus; then typing returns to line input and TUI mode is
switched off.

After `term::off` returns, `term::isOn` reports `FALSE` and the surface is
inactive again: the drawing, cursor, colour and attribute calls do nothing, the
getters return inert defaults, and `term::terminalSize` and `term::measureText`
raise `ErrUnsupported`; `term::isOn`, `term::didResize` and the mouse calls work
as before (`mfb man term isOn` lists every call). A later `term::on` starts
over with a fresh surface and the default state; nothing drawn before
`term::off` survives it.

You do not have to reach `term::off` before the program ends: when a program that
uses `term::` exits — by returning from `main`, by an unhandled error, or on
Ctrl-C or `SIGTERM` — the runtime calls `term::off` for it, so the terminal is
restored. Call it explicitly when you want to print ordinary output after the
TUI part of the program, or to leave and re-enter TUI mode. A `kill -9`, or a
crash the runtime cannot intercept, still leaves the terminal on the alternate
screen in single-key mode; `reset` at the shell recovers it. In an `mfb build --app` program, `term::off` — like every other `term::` call — raises `ErrWrongMode` unless the program is in `app::Mode.Console`; see `mfb man app`."#;

const EX: &str = r#"Draw one frame and restore the terminal:

```
IMPORT term
IMPORT io

FUNC main() AS Nothing
  term::on()
  term::moveTo(0, 0)
  io::print("done")
  term::off()          ' presents the frame, then restores the screen
END FUNC
```

Leave TUI mode before printing ordinary output — the last line lands on the
user's normal screen, after the shell contents `term::on` set aside:

```
IMPORT term
IMPORT io

FUNC main() AS Nothing
  term::on()
  term::drawText(0, 0, "working...")
  term::sync()
  term::off()
  io::print("finished")
END FUNC
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
        name: "off",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Nothing,
            errors: vec!["ErrWrongMode"],
            body: Body::abi_function(lower_off),
        }],
    });
}
