//! `canvas::enableMouse` — turn mouse reporting on or off for the canvas surface.
//!
//! plan-94-A lands this as an inert stub: the argument is accepted and ignored, the
//! process-global mode word is left alone, and `canvas::pollMouse` keeps reporting
//! `MouseKind.None`. plan-94-C/D/E give it its real body — writing
//! `_mfb_rt_mouse_mode = 2` (pixels), which is what each backend's UI-thread mouse
//! handler reads to decide whether to emit and in which unit.

use crate::codegen::app::hook::app::{prepend_wrong_mode_gate, ModeRequirement};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

const INTRO: &str = r#"Turn mouse reporting on or off for the canvas surface."#;

const DESC: &str = r#"`canvas::enableMouse(TRUE)` asks the window to report mouse activity — button
presses and releases, motion, drags and wheel scrolling — so that
`canvas::pollMouse` can hand those events back to the program.
`canvas::enableMouse(FALSE)` stops the reporting again.

Mouse reporting is **opt-in**, and for a reason that matters more here than it
does on a terminal: motion is continuous. A window that reported every movement
whether or not the program was listening would spend its time delivering events
nobody reads. Asking first means a program that only draws pays nothing.

The call is **best effort** and reports no error.

Turn reporting off when the program stops caring. A program that never turns it
off is not broken — the events simply go unread — but a draw loop that has
stopped polling should stop asking.

Requires `Mode.Canvas`; elsewhere it raises the trappable `ErrWrongMode`."#;

const EX: &str = r#"Ask for mouse events before the draw loop:

```
IMPORT app
IMPORT canvas

SUB main()
  app::setMode(app::Mode.Canvas)
  canvas::enableMouse(TRUE)
  LET event AS canvas::MouseEvent = canvas::pollMouse()
  canvas::enableMouse(FALSE)
END SUB
```

Stop reporting once the program is done listening:

```
IMPORT app
IMPORT canvas
IMPORT io

SUB main()
  app::setMode(app::Mode.Canvas)
  canvas::enableMouse(TRUE)
  canvas::enableMouse(FALSE)
  io::print("no longer listening")
END SUB
```"#;

/// `canvas::enableMouse(enabled AS Boolean)`.
///
/// **A no-op stub in plan-94-A.** It deliberately does NOT write
/// `_mfb_rt_mouse_mode` yet: that word is what tells each backend's UI-thread
/// handler to start writing SGR bytes into the window input pipe, and until
/// plan-94-B's decoder exists to consume them there is nothing on the other end.
/// Setting it here would fill the pipe with reports that arrive at
/// `io::readChar` as garbage.
pub(crate) fn lower_enable_mouse(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.emit(abi::return_());

    // The same gate every surface-touching member takes: outside `Mode.Canvas` there
    // is no surface, so there is no window to ask for mouse events.
    prepend_wrong_mode_gate(
        &mut builder.instructions,
        &mut builder.relocations,
        &symbol,
        ctx.presentation_mode_offset,
        ModeRequirement::Canvas,
    );

    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Nothing,
        location: Operand::from("void"),
        text: symbol,
    })
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "enableMouse",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "enabled",
                desc: "`TRUE` to ask the window to report mouse activity, `FALSE` to \
                       stop asking.",
                aliases: &[],
                ty: ParameterType::Boolean,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Nothing,
            errors: vec!["ErrWrongMode"],
            body: Body::abi_function(lower_enable_mouse),
        }],
    });
}
