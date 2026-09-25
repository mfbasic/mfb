//! `canvas::pollMouse` — take the next pending mouse event, in canvas pixels.
//!
//! plan-94-C gives it its real body: it drains the per-thread ring plan-94-B
//! built, oldest first, skipping anything past its TTL. An empty or all-stale
//! ring still yields the all-zero record, whose `kind` is `MouseKind.None`
//! because `None` is declared first and so takes ordinal 0 — the same sentinel
//! plan-94-A established, now with a real answer behind it.
//!
//! This is `term::pollMouse`'s sibling, not a wrapper of it: `term::` is gated on
//! `Mode.Console` and `canvas::` on `Mode.Canvas`, so a canvas program must never be
//! made to `IMPORT term` to ask where the mouse is. The two packages carry
//! independent `MouseEvent`/`MouseKind`/`MouseButton` sets for exactly that reason —
//! the same shape `term::didResize` and `canvas::didResize` already have.

use crate::codegen::app::hook::app::{prepend_wrong_mode_gate, ModeRequirement};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::util::Vregs;
use crate::codegen::error::constants::*;
use crate::codegen::io::mouse::ring as mouse_ring;
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
/// Zero is not arbitrary filler for the idle answer: `MouseKind.None` and
/// `MouseButton.None` are declared first in their enums and so take ordinal 0, an
/// all-zero `Point` is `(0.0, 0.0)` because IEEE-754 zero is all-zero bits, and a
/// zero `Boolean` is `FALSE`. So the zeroed block already reads as "nothing
/// pending, no button, at the origin, no modifiers", and only a real event has to
/// write anything.
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
    let mouse_state_offset = ctx.mouse_state_offset.ok_or_else(|| {
        format!("native code plan emits '{symbol}' without reserving mouse state")
    })?;
    let alloc_ok = builder.label("canvas_poll_mouse_alloc_ok");
    let done = builder.label("canvas_poll_mouse_done");
    let clock_scratch = builder.allocate_stack_object("canvas_poll_clock", 16);

    // Drain first, then allocate: the arena call clobbers the argument and result
    // banks, so the five values are parked on the stack across it.
    let kind_slot = builder.allocate_stack_object("canvas_poll_kind", 8);
    let button_slot = builder.allocate_stack_object("canvas_poll_button", 8);
    let coord_a_slot = builder.allocate_stack_object("canvas_poll_coord_a", 8);
    let coord_b_slot = builder.allocate_stack_object("canvas_poll_coord_b", 8);
    let mods_slot = builder.allocate_stack_object("canvas_poll_mods", 8);
    {
        let mut vregs = Vregs::new();
        let mut ctx2 = EmitCtx {
            symbol: &symbol,
            platform_imports: ctx.platform_imports,
            platform: ctx.platform,
            instructions: &mut builder.instructions,
            relocations: &mut builder.relocations,
        };
        let event =
            mouse_ring::emit_dequeue(mouse_state_offset, clock_scratch, &mut ctx2, &mut vregs)?;
        for (slot, reg) in [
            (kind_slot, &event.kind),
            (button_slot, &event.button),
            (coord_a_slot, &event.coord_a),
            (coord_b_slot, &event.coord_b),
            (mods_slot, &event.mods),
        ] {
            ctx2.instructions
                .push(abi::store_u64(reg, abi::stack_pointer(), slot));
        }
    }

    builder.emit(abi::move_immediate(
        abi::c_arg(0),
        "Integer",
        &MOUSE_EVENT_BLOCK_BYTES.to_string(),
    ));
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
    builder.emit_arena_alloc_call();
    builder.emit(abi::branch_eq(&alloc_ok));
    builder.raise_error_bare("ErrOutOfMemory")?;
    builder.emit(abi::branch(&done));
    builder.emit(abi::label(&alloc_ok));

    let event = builder.temporary_vreg();
    let value = builder.temporary_vreg();
    builder.emit(abi::move_register(&event, abi::mfb_return(1)));

    // Zero the block first, so every prop has a defined value and only the ones
    // that differ from zero need writing.
    builder.emit(abi::move_immediate(&value, "Integer", "0"));
    for word in 0..MOUSE_EVENT_BLOCK_BYTES / 8 {
        builder.emit(abi::store_u64(&value, &event, word * 8));
    }

    // `kind` and `button` are enum ordinals in the first two slots.
    for (slot, offset) in [(kind_slot, 0usize), (button_slot, 8)] {
        builder.emit(abi::load_u64(&value, abi::stack_pointer(), slot));
        builder.emit(abi::store_u64(&value, &event, offset));
    }

    // `position` is an INLINED `Point`, so its slot carries the block-relative
    // offset of the sub-block rather than a value, and `0` there would mean
    // "absent" (plan-94-A Corrections C5).
    builder.emit(abi::move_immediate(
        &value,
        "Integer",
        &MOUSE_EVENT_POSITION_INLINE_OFFSET.to_string(),
    ));
    builder.emit(abi::store_u64(&value, &event, MOUSE_EVENT_POSITION_SLOT));

    // The ring's coordinates are integers; `Point` is two `Float`s.
    //
    // **The pair transposes here.** The decoder stores the wire's `y` in
    // `coord_a` and its `x` in `coord_b`, because `term::MouseEvent` is
    // row-then-column. `canvas::Point` is x-then-y, so `position.x` reads
    // `coord_b` and `position.y` reads `coord_a` — the one place the two
    // packages' field orders disagree, resolved where both are in view.
    let coord = builder.temporary_fp_vreg();
    for (slot, offset) in [
        (coord_b_slot, MOUSE_EVENT_POSITION_INLINE_OFFSET),
        (coord_a_slot, MOUSE_EVENT_POSITION_INLINE_OFFSET + 8),
    ] {
        builder.emit(abi::load_u64(&value, abi::stack_pointer(), slot));
        builder.emit(abi::signed_convert_to_float_d(&coord, &value));
        builder.emit(abi::store_double(&coord, &event, offset));
    }

    // The three modifier flags unpack from the packed bits into `Boolean`s.
    let mods = builder.temporary_vreg();
    let bit = builder.temporary_vreg();
    builder.emit(abi::load_u64(&mods, abi::stack_pointer(), mods_slot));
    for (offset, mask) in [
        (24usize, MOUSE_MOD_SHIFT),
        (32, MOUSE_MOD_CTRL),
        (40, MOUSE_MOD_ALT),
    ] {
        let set = builder.label("canvas_poll_mod_set");
        let store = builder.label("canvas_poll_mod_store");
        builder.emit(abi::move_immediate(&bit, "Integer", &mask.to_string()));
        builder.emit(abi::and_registers(&value, &mods, &bit));
        builder.emit(abi::compare_immediate(&value, "0"));
        builder.emit(abi::branch_ne(&set));
        builder.emit(abi::move_immediate(&value, "Boolean", "0"));
        builder.emit(abi::branch(&store));
        builder.emit(abi::label(&set));
        builder.emit(abi::move_immediate(&value, "Boolean", "1"));
        builder.emit(abi::label(&store));
        builder.emit(abi::store_u64(&value, &event, offset));
    }

    builder.emit(abi::move_register(RESULT_VALUE_REGISTER, &event));
    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.emit(abi::label(&done));
    builder.emit(abi::return_());

    // The same gate every surface-touching member takes: outside `Mode.Canvas`
    // there is no surface, so a position in surface pixels has no meaning.
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
