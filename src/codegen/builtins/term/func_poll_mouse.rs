//! `term::pollMouse` — abi_function member (native terminal I/O).
//!
//! Registers its own [`lower_poll_mouse`] `Body::abi_function` body; the
//! `abi_function` wrapper finalizes it. The heavy terminal emission stays in the
//! shared code layer (`code::lower_term_helper` / `emit_app_term_helper`).
//!
//! plan-94-A lands this as an inert stub: it always builds the all-zero
//! `MouseEvent`, whose `kind` is `MouseKind.None` because `None` is declared first
//! and so takes ordinal 0. That zero record is the permanent "no event" sentinel —
//! plan-94-B replaces the body with the ring reader, and the sentinel keeps its
//! meaning.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Take the next pending mouse event, or report that there is none"#;

const DESC: &str = r#"`term::pollMouse` returns the next mouse event the terminal has reported since
the last call, as a `term::MouseEvent`. It never waits: when nothing is pending
it returns immediately with `kind` set to `term::MouseKind.None`, which is the
answer a program tests for before looking at any other field.

Events only arrive after `term::enableMouse(TRUE)`. Before that — and on a
terminal that does not report mouse activity at all — every call reports
`MouseKind.None`, so a draw loop can poll unconditionally and simply see nothing
happen.

Each call takes **one** event, so a loop that calls it until it reports
`MouseKind.None` has drained everything pending and can then present a frame.
Events are delivered **freshest-first-wins**: an event the program does not
collect within a tenth of a second is dropped rather than queued up, so a
program that stops polling for a moment resumes with what the user is doing now
rather than replaying what they did while it was busy.

`row` and `column` are zero-based cells measured from the top-left corner of the
surface, the same coordinates `term::moveTo` and `term::drawText` take, so an
event position can be handed straight to a drawing call. `shift`, `ctrl` and
`alt` report which modifier keys were held. `button` says which button the event
is about, and is `term::MouseButton.None` for motion and wheel events, which
belong to no button.

In a `mfb build --app` window the same events come from the window rather than
from a terminal, and report the same cell coordinates."#;

const EX: &str = r#"Poll for an event and ignore the idle answer:

```
IMPORT term

SUB main()
  term::on()
  term::enableMouse(TRUE)
  LET event = term::pollMouse()
  IF event.kind <> term::MouseKind.None THEN
    term::drawText(event.row, event.column, "*")
    term::sync()
  END IF
  term::enableMouse(FALSE)
  term::off()
END SUB
```

Report where a click landed:

```
IMPORT io
IMPORT term

SUB main()
  term::on()
  term::enableMouse(TRUE)
  LET event = term::pollMouse()
  term::enableMouse(FALSE)
  term::off()
  IF event.kind = term::MouseKind.Down THEN
    io::print("clicked at row " & toString(event.row) & " column " & toString(event.column))
  ELSE
    io::print("nothing pending")
  END IF
END SUB
```"#;

/// `abi_function` body for `term::pollMouse` — delegates to the shared
/// family-generic [`super::gen_shared::lower_term_helper`] with its own runtime-call
/// name (the app-vs-console dispatch and the heavy per-member emitters live in the
/// shared code layer).
pub(crate) fn lower_poll_mouse(
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
        name: "pollMouse",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::named(super::MOUSE_EVENT_TYPE),
            errors: vec!["ErrOutOfMemory"],
            body: Body::abi_function(lower_poll_mouse),
        }],
    });
}
