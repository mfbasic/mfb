//! `canvas::pollMouse` — take the next pending mouse event, in canvas pixels.
//!
//! plan-94-A lands this as an inert stub: it always builds the all-zero
//! `MouseEvent`, whose `kind` is `MouseKind.None` because `None` is declared first
//! and so takes ordinal 0. That zero record is the permanent "no event" sentinel —
//! plan-94-C/D/E feed the ring that replaces its contents, and the sentinel keeps
//! its meaning as the idle answer.
//!
//! This is `term::pollMouse`'s sibling, not a wrapper of it: `term::` is gated on
//! `Mode.Console` and `canvas::` on `Mode.Canvas`, so a canvas program must never be
//! made to `IMPORT term` to ask where the mouse is. The two packages carry
//! independent `MouseEvent`/`MouseKind`/`MouseButton` sets for exactly that reason —
//! the same shape `term::didResize` and `canvas::didResize` already have.

use crate::codegen::app::hook::app::{prepend_wrong_mode_gate, ModeRequirement};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::target::shared::abi;
use crate::types::ParameterType;

const INTRO: &str = r#"Take the next pending mouse event, or report that there is none."#;

const DESC: &str = r#"`canvas::pollMouse` returns the next mouse event the window has reported since
the last call, as a `canvas::MouseEvent`. It never waits: when nothing is
pending it returns immediately with `kind` set to `canvas::MouseKind.None`,
which is the answer a program tests for before looking at any other field.

Events only arrive after `canvas::enableMouse(TRUE)`. Before that, every call
reports `MouseKind.None`, so a draw loop can poll unconditionally and simply see
nothing happen.

Each call takes **one** event, so a loop that calls it until it reports
`MouseKind.None` has drained everything pending and can then present a frame.
That is the shape a draw loop wants: drain, update, present. Events are
delivered **freshest-first-wins** — an event the program does not collect within
a tenth of a second is dropped rather than queued up, so a program that spends a
long frame rendering resumes with where the mouse is now rather than replaying
where it has been.

`position` is a `canvas::Point` in the same pixel coordinates every
`canvas::DrawItem` uses — top-left origin, Y increasing downward — so an event
position can be handed straight to a hit test against the items just presented,
with no conversion. `shift`, `ctrl` and `alt` report which modifier keys were
held. `button` says which button the event is about, and is
`canvas::MouseButton.None` for motion and wheel events, which belong to no
button.

Requires `Mode.Canvas`; elsewhere it raises the trappable `ErrWrongMode`."#;

const EX: &str = r#"Poll for an event and ignore the idle answer:

```
IMPORT app
IMPORT canvas

SUB main()
  app::setMode(app::Mode.Canvas)
  canvas::enableMouse(TRUE)
  LET event AS canvas::MouseEvent = canvas::pollMouse()
  IF event.kind <> canvas::MouseKind.None THEN
    canvas::enableMouse(FALSE)
  END IF
END SUB
```

Report where a click landed, in surface pixels:

```
IMPORT app
IMPORT canvas
IMPORT io

SUB main()
  app::setMode(app::Mode.Canvas)
  canvas::enableMouse(TRUE)
  LET event AS canvas::MouseEvent = canvas::pollMouse()
  canvas::enableMouse(FALSE)
  IF event.kind = canvas::MouseKind.Down THEN
    io::print("clicked at " & toString(event.position.x) & "," & toString(event.position.y))
  ELSE
    io::print("nothing pending")
  END IF
END SUB
```"#;

/// Byte size of the `canvas::MouseEvent` block.
///
/// Six props — `kind`, `button`, `position`, `shift`, `ctrl`, `alt` — at one 8-byte
/// value slot each is the 48-byte fixed region, and `position` is a **record**, so
/// it is an *inlined* field: its slot holds the block-relative byte offset of the
/// sub-block rather than a value, and the sub-block follows the fixed region. A
/// `Point` is two `Float`s = 16 bytes, so the whole block is 48 + 16 = 64.
///
/// (Measured against what the compiler itself emits for a hand-written record of
/// the identical shape: fixed = `8 * fields.len()`, the inlined field's slot holds
/// the 8-aligned offset past the fixed region, and the sub-block is memcpy'd there
/// — `emit_record_block_size_to_slot`.)
const MOUSE_EVENT_BLOCK_BYTES: usize = 64;
/// Byte offset of the `position` slot — prop index 2, so `8 * 2`. It holds an
/// OFFSET, not a `Point` pointer.
const MOUSE_EVENT_POSITION_SLOT: usize = 16;
/// The block-relative byte offset the `position` slot points at: immediately past
/// the six fixed slots, which is already 8-aligned.
const MOUSE_EVENT_POSITION_INLINE_OFFSET: usize = 48;

/// `canvas::pollMouse() AS MouseEvent`.
///
/// **A stub in plan-94-A** — it always builds the all-zero record. Zero is not
/// arbitrary filler: `MouseKind.None` and `MouseButton.None` are declared first in
/// their enums and so take ordinal 0, an all-zero `Point` is `(0.0, 0.0)` because
/// IEEE-754 zero is all-zero bits, and a zero `Boolean` is `FALSE`. So the zeroed
/// block reads exactly as "nothing pending, no button, at the origin, no
/// modifiers" — the permanent idle answer, which the real implementation keeps.
///
/// The one field that is not simply zeroed is `position`'s slot, which must carry
/// the inline offset rather than 0: a 0 there is the "sub-block absent" sentinel
/// (`emit_record_block_size_to_slot`), and a reader that trusted it would size the
/// block short.
pub(crate) fn lower_poll_mouse(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let alloc_ok = builder.label("canvas_poll_mouse_alloc_ok");

    builder.emit(abi::move_immediate(
        abi::c_arg(0),
        "Integer",
        &MOUSE_EVENT_BLOCK_BYTES.to_string(),
    ));
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
    builder.emit_arena_alloc_call();
    builder.emit(abi::branch_eq(&alloc_ok));
    builder.raise_error_bare("ErrOutOfMemory")?;
    builder.emit(abi::label(&alloc_ok));

    let event = builder.temporary_vreg();
    builder.emit(abi::move_register(&event, abi::mfb_return(1)));

    // Zero the whole block: every prop's idle value, and the inlined `Point`'s two
    // `Float`s, are all-zero bits.
    let zero = builder.temporary_vreg();
    builder.emit(abi::move_immediate(&zero, "Integer", "0"));
    for word in 0..MOUSE_EVENT_BLOCK_BYTES / 8 {
        builder.emit(abi::store_u64(&zero, &event, word * 8));
    }
    // …then overwrite `position`'s slot with the inline offset. 0 would mean
    // "absent", not "(0,0)".
    let inline_offset = builder.temporary_vreg();
    builder.emit(abi::move_immediate(
        &inline_offset,
        "Integer",
        &MOUSE_EVENT_POSITION_INLINE_OFFSET.to_string(),
    ));
    builder.emit(abi::store_u64(
        &inline_offset,
        &event,
        MOUSE_EVENT_POSITION_SLOT,
    ));

    builder.emit(abi::move_register(RESULT_VALUE_REGISTER, &event));
    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.emit(abi::return_());

    // The same gate every surface-touching member takes: outside `Mode.Canvas` there
    // is no surface, so a position in surface pixels has no meaning to report.
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
        name: "pollMouse",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::named(super::MOUSE_EVENT_TYPE),
            errors: vec!["ErrWrongMode", "ErrOutOfMemory"],
            body: Body::abi_function(lower_poll_mouse),
        }],
    });
}
