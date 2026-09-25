//! `term::enableMouse` — abi_function member (native terminal I/O).
//!
//! Registers its own [`lower_enable_mouse`] `Body::abi_function` body; the
//! `abi_function` wrapper finalizes it. The heavy terminal emission stays in the
//! shared code layer (`code::lower_term_helper` / `emit_app_term_helper`).
//!
//! plan-94-B gives it its real body: the 1000/1002/1006 mode set/reset, the
//! event ring's allocation and release, and the process-global mouse-mode word
//! every app backend's UI-thread handler reads.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Turn mouse reporting on or off for the terminal surface"#;

const DESC: &str = r#"`term::enableMouse(TRUE)` asks the terminal to report mouse activity — button
presses and releases, motion, drags and wheel scrolling — so that
`term::pollMouse` can hand those events back to the program.
`term::enableMouse(FALSE)` stops the reporting again.

Mouse reporting is **opt-in**. A program that never calls this sees exactly the
terminal behaviour it has always seen: no reporting is requested, the mouse
belongs to the terminal, and text selection and scrolling work as the user
expects. That is deliberate — taking the mouse away from the user is
disruptive, so a program has to ask.

The call is **best effort** and reports no error. Not every terminal supports
mouse reporting, and in that case `term::pollMouse` simply keeps reporting
`term::MouseKind.None` — which is the same thing it reports when the user is
not using the mouse, so a program that polls needs no separate check.

Two interactions are worth knowing. `io::readLine` and `io::input` **suspend**
mouse reporting while they read a line, because the terminal echoes during a line
read and a mouse report arriving then would be printed onto the screen in the
middle of what the user is typing; events are lost for the duration, which is the
right trade for a program that is asking someone to type. And `io::pollInput`
stays truthful: it still answers whether a following read will return without
waiting, even though some of what the terminal now sends is taken off the stream
as mouse input and never becomes a character at all.

The Esc key still works while reporting is on, a moment later than usual. A
mouse report starts with the same Esc character the key sends, so after an Esc
the program waits up to 25 milliseconds to see whether a report follows. If
nothing does, `io::readChar` returns the Esc and `io::pollInput` reports it
ready. Keys that send a longer sequence, such as the arrow keys, arrive whole
and in order, as they always have.

Turn reporting off before the program finishes so the mouse goes back to the
user. `term::off` does that for you when it leaves TUI mode, so a program that
pairs `term::on` with `term::off` is already tidy."#;

const EX: &str = r#"Ask for mouse events, then read one:

```
IMPORT term

SUB main()
  term::on()
  term::enableMouse(TRUE)
  LET event = term::pollMouse()
  term::enableMouse(FALSE)
  term::off()
END SUB
```

Turn reporting off again so the user gets the mouse back:

```
IMPORT io
IMPORT term

SUB main()
  term::on()
  term::enableMouse(TRUE)
  term::enableMouse(FALSE)
  term::off()
  io::print("the mouse is the terminal's again")
END SUB
```"#;

/// `abi_function` body for `term::enableMouse` — delegates to the shared
/// family-generic [`super::gen_shared::lower_term_helper`] with its own runtime-call
/// name (the app-vs-console dispatch and the heavy per-member emitters live in the
/// shared code layer).
pub(crate) fn lower_enable_mouse(
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
        name: "enableMouse",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Boolean"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "enabled",
                desc: "`TRUE` to ask the terminal to report mouse activity, \
                       `FALSE` to stop asking.",
                aliases: &[],
                ty: ParameterType::Boolean,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_enable_mouse),
        }],
    });
}
