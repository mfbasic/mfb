//! `canvas::enableMouse` — turn mouse reporting on or off for the canvas surface.
//!
//! plan-94-C gives it its real body: it writes `_mfb_rt_mouse_mode = 2` (pixels),
//! which is what each backend's UI-thread mouse handler reads to decide whether to
//! emit and in which unit, and allocates the per-thread ring plan-94-B's decoder
//! fills. `enableMouse(FALSE)` clears the word and frees the ring.
//!
//! It writes **no terminal escapes**, unlike its `term::` sibling. A canvas
//! program has no terminal to ask: the events come from the window's own handlers,
//! which the mode word switches on.

use crate::codegen::app::hook::app::{prepend_wrong_mode_gate, ModeRequirement};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::util::Vregs;
use crate::codegen::error::constants::*;
use crate::codegen::io::mouse::ring as mouse_ring;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::codegen::term::core::{emit_mouse_inject, emit_store_mouse_mode};
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
/// Writes the mode word, allocates or frees the ring, and honours
/// `MFB_MOUSE_INJECT`. No terminal escapes: a canvas program has no terminal to
/// ask, and in an `--app` build stdout is the window, where an escape would be
/// displayed rather than obeyed.
pub(crate) fn lower_enable_mouse(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let mouse_state_offset = ctx.mouse_state_offset.ok_or_else(|| {
        format!("native code plan emits '{symbol}' without reserving mouse state")
    })?;
    let enabled = args
        .first()
        .ok_or_else(|| format!("'{symbol}' expects the enabled argument"))?
        .location
        .clone();

    let disable = builder.label("canvas_enable_mouse_disable");
    let done = builder.label("canvas_enable_mouse_done");
    // The decoder stamps each event, so the ring's allocation path needs the same
    // clock scratch the read path does.
    let clock_scratch = builder.allocate_stack_object("canvas_mouse_clock", 16);

    let flag = builder.temporary_vreg();
    builder.emit(abi::move_register(&flag, &enabled));
    builder.emit(abi::compare_immediate(&flag, "0"));
    builder.emit(abi::branch_eq(&disable));

    // --- enable ---
    {
        let mut vregs = Vregs::new();
        let mut ctx2 = EmitCtx {
            symbol: &symbol,
            platform_imports: ctx.platform_imports,
            platform: ctx.platform,
            instructions: &mut builder.instructions,
            relocations: &mut builder.relocations,
        };
        mouse_ring::emit_ring_alloc(mouse_state_offset, &mut ctx2, &mut vregs)?;
        emit_store_mouse_mode(&mut ctx2, MOUSE_MODE_PIXELS, &mut vregs);
        emit_mouse_inject(&mut ctx2, mouse_state_offset, clock_scratch, &mut vregs)?;
    }
    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.emit(abi::branch(&done));

    // --- disable ---
    builder.emit(abi::label(&disable));
    {
        let mut vregs = Vregs::new();
        let mut ctx2 = EmitCtx {
            symbol: &symbol,
            platform_imports: ctx.platform_imports,
            platform: ctx.platform,
            instructions: &mut builder.instructions,
            relocations: &mut builder.relocations,
        };
        emit_store_mouse_mode(&mut ctx2, MOUSE_MODE_OFF, &mut vregs);
        mouse_ring::emit_ring_free(mouse_state_offset, &mut ctx2, &mut vregs)?;
    }
    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.emit(abi::label(&done));
    builder.emit(abi::return_());

    // The same gate every surface-touching member takes: outside `Mode.Canvas`
    // there is no surface, so there is no window to ask for mouse events.
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
