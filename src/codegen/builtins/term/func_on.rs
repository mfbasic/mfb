//! `term::on` — abi_function member (native terminal I/O).
//!
//! Registers its own [`lower_on`] `Body::abi_function`
//! body; the `abi_function` wrapper finalizes it. The heavy terminal emission stays
//! in the shared code layer (`code::lower_term_helper` / `emit_app_term_helper`).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Enter TUI mode: allocate the drawing surface and reset all `term::` state"#;

const DESC: &str = r#"`term::on` is the gate for the whole module. Every other `term::` call except
`term::isOn` short-circuits to a no-op (or, for the getters, to an inert default)
while TUI mode is off, so nothing a program draws takes effect until `term::on`
has returned. It takes no arguments.

The call does four things, in this order.

1. **Allocates the shadow grid.** It asks the terminal for its size with
   `TIOCGWINSZ`, falling back to 24 rows by 80 columns when that is unavailable,
   and allocates one arena block holding a back cell buffer, a front cell buffer,
   and the scratch the present builds its escape stream into. The block is
   zero-filled, which is a cleared surface, and its dirty flag is set so the first
   `term::sync` repaints in full. This happens **before** the active flag is set,
   so a program never observes TUI mode on with no surface behind it; if the
   allocation fails, `ErrOutOfMemory` is raised and the terminal is left
   completely untouched.
2. **Resets `term::` state to defaults**: foreground white (255, 255, 255),
   background black (0, 0, 0), bold off, underline off, cursor visible, and the
   shadow cursor at the home position (row 0, column 0).
3. **Switches the terminal to its alternate screen**, so the user's previous
   shell contents are preserved and restored by `term::off`, and resets the
   terminal's own colours.
4. **Puts a console tty into single-key mode**: `~ICANON`, `~ECHO`, `VMIN = 1`,
   `VTIME = 0`, so a `io::pollInput` + `io::readChar` loop registers bare
   keypresses without waiting for Return. The saved cooked line discipline is kept
   so `term::off`, `io::input`, and `io::readLine` can restore it. When standard
   input is not a terminal — piped input, a test harness — this step is inert, and
   if the terminal cannot be reconfigured it is abandoned rather than failing the
   call.

The surface `term::on` establishes is **retained and double-buffered**: from here
on, drawing calls — including `io::print` and `io::write` — mutate the back cell
buffer rather than the terminal, and only `term::sync` presents a frame. A
program that draws without calling `term::sync` displays nothing.

`term::on` is one of the two calls that are not gated, so calling it while TUI
mode is already on runs the whole sequence again: a fresh surface sized to the
terminal, defaults restored, and the previously drawn frame discarded. Guard with
`term::isOn` if that is not wanted."#;

const EX: &str = r#"Enter TUI mode, draw one frame, present it, and restore the terminal:

```
IMPORT term
IMPORT io

SUB main()
  term::on()
  term::clear()
  term::moveTo(0, 0)
  term::setForeground(255, 0, 0)
  io::print("hello in red")
  term::sync()
  term::off()
END SUB
```

Enter TUI mode only once:

```
IMPORT term

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
