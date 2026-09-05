//! `term::on` — abi_function member (native terminal I/O).
//!
//! Registers its own [`lower_on`] `Body::abi_function`
//! body; the `abi_function` wrapper finalizes it. The heavy terminal emission stays
//! in the shared code layer (`code::lower_term_helper` / `emit_app_term_helper`).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Enter TUI mode: take over the screen and reset all `term::` state"#;

const DESC: &str = r#"`term::on` is the gate for the whole module. Every other `term::` call except
`term::isOn` and `term::didResize` short-circuits while TUI mode is off — to a
no-op, or to an inert default for the getters, or to `ErrUnsupported` for
`term::terminalSize` — so nothing a program draws takes effect until `term::on`
has returned. It takes no arguments.

The call does four things, in this order.

1. **Sets up the drawing surface.** It asks the terminal for its size, falling
   back to 24 rows by 80 columns when the terminal cannot say. The surface
   starts cleared, and the first `term::sync` repaints all of it. If the surface
   cannot be set up, `ErrOutOfMemory` is raised and the terminal is left
   completely untouched — TUI mode is never left half-on.
2. **Resets `term::` state to defaults**: foreground white (255, 255, 255),
   background black (0, 0, 0), bold off, underline off, cursor visible, and the
   cursor at the home position (row 0, column 0).
3. **Switches the terminal to its alternate screen**, so the user's previous
   shell contents are preserved and restored by `term::off`, and resets the
   terminal's own colours.
4. **Puts the terminal into single-key mode**, so an `io::pollInput` +
   `io::readChar` loop sees each keypress as it happens instead of waiting for
   Return, and keys are not echoed as the user types. The normal line-editing
   mode is remembered, so `term::off`, `io::input` and `io::readLine` restore
   it. When standard input is not a terminal — piped input, a test harness —
   this step does nothing, and if the terminal will not change mode it is
   skipped rather than failing the call.

Drawing is **buffered**: from here on, every drawing call — including
`io::print` and `io::write` — updates the surface rather than the terminal, and
only `term::sync` puts a frame on screen. **A program that draws without
calling `term::sync` displays nothing.**

`term::on` is one of the calls that are not gated, so calling it while TUI mode is
already on runs the whole sequence again: a fresh surface sized to the
terminal, defaults restored, and the previously drawn frame discarded. Guard with
`term::isOn` if that is not wanted."#;

const EX: &str = r#"Enter TUI mode, draw one frame, present it, and restore the terminal:

```
IMPORT term
IMPORT color
IMPORT io

SUB main()
  term::on()
  term::clear()
  term::moveTo(0, 0)
  term::setForeground(color::rgb(255, 0, 0))
  io::print("hello in red")
  term::sync()
  term::off()
END SUB
```

Enter TUI mode only once:

```
IMPORT term
IMPORT color

SUB main()
  IF NOT term::isOn() THEN
    term::on()
  END IF
END SUB
```"#;
/// `abi_function` body for `term::on` — delegates to the shared family-generic
/// [`super::gen_shared::lower_term_helper`] with its own runtime-call name (the
/// app-vs-console dispatch and the heavy per-member emitters live in the shared code
/// layer).
pub(crate) fn lower_on(
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
        name: "on",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_on),
        }],
    });
}
