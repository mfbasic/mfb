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

const DESC: &str = r#"`term::on` is the gate for the `term` package. While TUI mode is off, the
drawing, cursor, colour and attribute calls do nothing, the getters return inert
defaults, and `term::terminalSize` and `term::measureText` raise
`ErrUnsupported` — so nothing a program draws takes effect until `term::on` has
returned. Only `term::isOn`, `term::didResize` and the mouse calls work the same
either way (`mfb man term isOn` lists every call). It takes no arguments.

The call does five things, in this order.

1. **Sets up the drawing surface.** It asks the terminal for its size, falling
   back to a 24-row by 80-column surface when the terminal cannot say — though
   `term::terminalSize` then still raises `ErrUnsupported` rather than reporting
   the fallback, so trap it if the program may run with output redirected. The surface
   starts cleared, and the first `term::sync` repaints all of it. If the surface
   cannot be set up, `ErrOutOfMemory` is raised and the terminal is left
   completely untouched — TUI mode is never left half-on.
2. **Resets `term::` state to defaults**: foreground white (255, 255, 255),
   background black (0, 0, 0), bold off, underline off, cursor visible, and the
   cursor at the home position (row 0, column 0). A resize that
   `term::didResize` has not yet reported is forgotten too, since the new
   surface is already sized to the terminal.
3. **Switches the terminal to its alternate screen**, so the user's previous
   shell contents are preserved and restored by `term::off`, and resets the
   terminal's own colours.
4. **Puts the terminal into single-key mode**, so a loop over `io::readKey`
   sees each keypress as it happens instead of waiting for Return, and keys are
   not echoed as the user types. `term::off` restores the normal line-editing
   mode. `io::input` and `io::readLine` switch to it just for that one read —
   waiting for Return and echoing as usual — and put single-key mode back
   afterwards. When standard input is not a terminal — piped input, a test
   harness — this step does nothing, and if the terminal will not change mode it
   is skipped rather than failing the call.
5. **Turns on full key reporting where the terminal has it.** A terminal that
   speaks the kitty keyboard protocol (kitty, among others) is asked to report
   every key with its modifiers, including repeats and releases; `io::readKey`
   then returns `io::KeyEvent.Repeat` and `io::KeyEvent.Release` as well as
   `Press`. Any other terminal is left as it is, and a key there reports
   `io::KeyEvent.NotSupported`. Ctrl+C still stops the program either way, but
   on a terminal using full key reporting it arrives as a key, so it takes
   effect the next time the program reads with an `io::` reader rather than at
   once. `term::off` turns the reporting back off. Only a program that reads
   from the console with an `io::` reader — a key reader, `io::input` or
   `io::readLine` — or polls the mouse asks. A local terminal answers at once;
   one that never answers delays `term::on` by up to half a second, after which
   it counts as not supporting full key reporting.

Drawing is **buffered**: from here on, every drawing call — including
`io::print` and `io::write` — updates the surface rather than the terminal.
Only `term::sync` presents a frame while TUI mode is on. `term::off` — and
program exit, which calls it for you — presents whatever is pending and then
leaves the alternate screen, so **a program that never calls `term::sync` shows
its frame only for the instant before the screen is restored, which in practice
is nothing.**

`term::on` is one of the calls that are not gated, so calling it while TUI mode is
already on runs the whole sequence again: a fresh surface sized to the
terminal, defaults restored, and the previously drawn frame discarded. The
terminal's line-editing mode is still the one from before the first `term::on`,
so `term::off` restores it however many times `term::on` ran.

In an `--app` program, `term::on` — like every `term::` call — raises
`ErrWrongMode` unless the program is in `app::Mode.Console`; see `mfb man app`."#;

const EX: &str = r#"Enter TUI mode, draw one frame, present it, and restore the terminal:

```
IMPORT term
IMPORT color
IMPORT io

FUNC main() AS Nothing
  term::on()
  term::clear()
  term::moveTo(0, 0)
  term::setForeground(color::rgb(255, 0, 0))
  io::print("hello in red")
  term::sync()
  term::off()
END FUNC
```

Enter TUI mode only once:

```
IMPORT term
IMPORT color

FUNC main() AS Nothing
  IF NOT term::isOn() THEN
    term::on()
  END IF
END FUNC
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
        name: "on",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Nothing,
            errors: vec!["ErrOutOfMemory", "ErrWrongMode"],
            body: Body::abi_function(lower_on),
        }],
    });
}
