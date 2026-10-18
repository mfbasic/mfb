//! `canvas::pollMouse` — take the next pending mouse event, in canvas pixels.
//!
//! plan-172-I split it in two. The native function here is `canvas::pollMouseRaw`,
//! internal, which dequeues one event into a whole `MouseEvent` whose `overID` is
//! empty; the public `canvas::pollMouse` is MFBASIC (`helper_hit.rs:__canvas_pollMouse`),
//! which calls it and, for a real event over a scene that has an `id` in it, answers a
//! copy with `overID` filled in. The split is `canvas::setBytes` over `setBytesRaw`'s:
//! the answer needs a `MATCH` over `DrawItem`, which codegen should not open-code. The
//! native half returns the finished record, not a shorter one, so that an idle poll and
//! a poll over a scene with no `id` — nearly every poll there is — hand the native
//! block straight back.
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

Events arrive whether or not the program reads the keyboard: each call first
takes in what the window has already reported, without waiting for more. Keys
typed meanwhile are kept, in order, for the program's next `io::` read; a program
that uses no `io::` reader has nowhere to keep them, and they are discarded.

`position` is a `vector::Float2` in the same pixel coordinates every
`canvas::DrawItem` uses — top-left origin, Y increasing downward. `shift`, `ctrl`
and `alt` report which modifier keys were held. `button` says which button the
event is about, and is `canvas::MouseButton.None` for motion and wheel events,
which belong to no button.

`overID` says which of your items the mouse was over. Give an item an `id` and an
event over it reports that `id`; an event over nothing, or over an item with no
`id`, reports `""`. It is always the **topmost** item at that pixel, so an unnamed
item lying on a named one reports `""` — name the upper one, or draw it first.

- A closed shape — a rectangle, circle, ellipse, polygon or `canvas::CurvePolygon` — is over on its inside and on its outline, whether or not it is filled.
- A picture is over wherever its shape is, whatever part of the shape its image covers. A picture that draws nothing — its image is closed, or its shape is a line, arc, polyline, `canvas::PolyCurve` or text — is never over.
- A `canvas::Line`, `canvas::Arc`, `canvas::PolyLine` or `canvas::PolyCurve` is over on its stroke, and on at least three pixels either side of its centre line, so a thin line can still be pointed at. A gap in a dashed one counts as the line. A `canvas::PolyCurve` that curls round does not enclose anything: the space between it and its own ends is not over.
- A `canvas::Text` is over anywhere in its box — the width of the string by the font's ascent and descent — not only on the ink of its letters.
- A `paint.clip` cuts the item for the mouse as it does for the eye, and a `transform`, the item's or any group's, moves and turns what is tested with what is drawn.

An item inside a group is reported with the group's `id` in front: a group node
with `id` `"save"` holding an item with `id` `"label"` reports `"save::label"`,
and groups inside groups chain, outermost first. A part that is `""` is left out,
so an unnamed item in a named group reports the group's `id` alone. That is how
one group drawn twice tells you which of the two was clicked: give each
`canvas::Group` node its own `id`.

A `canvas::Picture` works the same way with its shape: a picture with `id`
`"avatar"` on a shape with `id` `"frame"` reports `"avatar::frame"`, so two
pictures that share one shape can still be told apart.

The answer is for the scene you last passed to `canvas::present` or
`canvas::presentLayers`, at the moment you poll — not for whatever was on screen
when the mouse moved. Poll after presenting and the two are the same.

What it costs depends on whether anything has an `id`. A scene with no `id`
anywhere costs `pollMouse` nothing extra, however large it is. With an `id` in it,
the first event after each `present` that changed the scene looks the whole scene
over once, and every further event until the next change only walks the items from
the top one down to the one under the mouse. Measured on an Apple M2 Max with every item named, that first event
took about 0.8 milliseconds for 1,000 items and about 9 milliseconds for 10,000;
each further one about 9 microseconds and about 170. A scene that changes every
frame while the mouse moves pays the first figure every frame, and one that is
presented only when something changed pays it only then.

Requires `Mode.Canvas`; elsewhere it raises the trappable `ErrWrongMode`."#;

const EX: &str = r#"Poll for an event and ignore the idle answer:

```
IMPORT app
IMPORT canvas
IMPORT vector

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  canvas::enableMouse(TRUE)
  LET event AS canvas::MouseEvent = canvas::pollMouse()
  IF event.kind <> canvas::MouseKind.None THEN
    canvas::enableMouse(FALSE)
  END IF
END FUNC
```

Report where a click landed, in surface pixels:

```
IMPORT app
IMPORT canvas
IMPORT vector
IMPORT io

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  canvas::enableMouse(TRUE)
  LET event AS canvas::MouseEvent = canvas::pollMouse()
  canvas::enableMouse(FALSE)
  IF event.kind = canvas::MouseKind.Down THEN
    io::print("clicked at " & toString(event.position.x) & "," & toString(event.position.y))
  ELSE
    io::print("nothing pending")
  END IF
END FUNC
```

Name two buttons and report which one a press landed on. Each draw of the `button`
group has its own `id`, so `overID` is `"save::face"` over one and `"quit::face"` over
the other:

```
IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color
IMPORT io

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  MUT face AS canvas::Rectangle
  face = WITH face { id := "face", size := canvas::Size[width := 120.0, height := 40.0], paint := canvas::fill(color::rgb(60, 120, 200)) }
  canvas::setGroup("button", [face])

  MUT save AS canvas::Group
  save = WITH save { id := "save", name := "button", position := vector::Float2[x := 40.0, y := 40.0] }
  MUT quit AS canvas::Group
  quit = WITH quit { id := "quit", name := "button", position := vector::Float2[x := 200.0, y := 40.0] }
  canvas::present([save, quit])

  canvas::enableMouse(TRUE)
  LET event AS canvas::MouseEvent = canvas::pollMouse()
  canvas::enableMouse(FALSE)
  IF event.kind = canvas::MouseKind.Down AND event.overID <> "" THEN
    io::print("pressed " & event.overID)
  END IF
END FUNC
```"#;

/// Byte size of the `canvas::MouseEvent` block this function builds.
///
/// Seven props — `kind`, `button`, `position`, `shift`, `ctrl`, `alt`, `overID` — at
/// one 8-byte slot each is the 56-byte fixed region. Two of them are *inlined* fields,
/// whose slot holds the block-relative byte offset of a sub-block in the trailing
/// region rather than a value, in field order:
///
/// * `position`, a `vector::Float2`: two `Float`s, 16 bytes, at 56;
/// * `overID`, a `String`: `{len, bytes, NUL}`, which for the empty string is a zero
///   length word and one NUL byte — 9 bytes, 8-aligned to 16 — at 72.
///
/// So the block is 56 + 16 + 16 = 88 (`.ai/codegen-invariants.md`, "Records inline
/// their String fields": block size = `8*n + Σ align8(len+9)`).
///
/// (The `position` half was measured against what the compiler itself emits for a
/// hand-written record of the identical shape — `emit_record_block_size_to_slot`;
/// `tests/canvas/rt_canvas_hit.rs:a_polled_event_is_a_whole_record` holds both halves
/// by copying a polled event and reading every field back.)
const MOUSE_EVENT_BLOCK_BYTES: usize = 88;
/// Byte offset of the `position` slot — prop index 2, so `8 * 2`. It holds an
/// OFFSET, not a `vector::Float2` pointer.
const MOUSE_EVENT_POSITION_SLOT: usize = 16;
/// The block-relative byte offset the `position` slot points at: immediately past
/// the seven fixed slots, which is already 8-aligned.
const MOUSE_EVENT_POSITION_INLINE_OFFSET: usize = 56;
/// Byte offset of the `overID` slot — prop index 6.
const MOUSE_EVENT_OVER_ID_SLOT: usize = 48;
/// Where the `overID` slot points: past `position`'s 16 bytes. The sub-block is the
/// empty string, which is all zero, so zeroing the block writes it.
const MOUSE_EVENT_OVER_ID_INLINE_OFFSET: usize = 72;

/// `canvas::pollMouseRaw() AS MouseEvent`: the event, with `overID` empty.
///
/// Zero is not arbitrary filler for the idle answer: `MouseKind.None` and
/// `MouseButton.None` are declared first in their enums and so take ordinal 0, an
/// all-zero `vector::Float2` is `(0.0, 0.0)` because IEEE-754 zero is all-zero bits, a
/// zero `Boolean` is `FALSE`, and a zero length word is the empty `String`. So the
/// zeroed block already reads as "nothing pending, no button, at the origin, no
/// modifiers, over nothing", and only a real event has to write anything.
///
/// The fields that are not simply zeroed are the two inlined slots, `position`'s and
/// `overID`'s, which must carry their offsets rather than 0: a 0 there is the
/// "sub-block absent" sentinel (`emit_record_block_size_to_slot`), and a reader that
/// trusted it would size the block short.
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
    // bug-1015: the pending-input drain's slots (its clock scratch is the
    // dequeue's too).
    let pending_base = builder.allocate_stack_object(
        "canvas_poll_pending",
        crate::codegen::builtins::io::gen_read_family::PendingInputSlots::BYTES,
    );
    let pending = crate::codegen::builtins::io::gen_read_family::PendingInput {
        app_mode: ctx.build_mode.is_app(),
        input_state_offset: ctx.input_state_offset,
        slots: crate::codegen::builtins::io::gen_read_family::PendingInputSlots::at(pending_base),
    };

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
        let event = mouse_ring::emit_dequeue(mouse_state_offset, &pending, &mut ctx2, &mut vregs)?;
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

    // `position` is an INLINED `vector::Float2`, so its slot carries the block-relative
    // offset of the sub-block rather than a value, and `0` there would mean
    // "absent" (plan-94-A Corrections C5).
    builder.emit(abi::move_immediate(
        &value,
        "Integer",
        &MOUSE_EVENT_POSITION_INLINE_OFFSET.to_string(),
    ));
    builder.emit(abi::store_u64(&value, &event, MOUSE_EVENT_POSITION_SLOT));
    // `overID` is an INLINED `String`: its slot carries the offset of the empty
    // string's sub-block, which the zeroing above already wrote.
    builder.emit(abi::move_immediate(
        &value,
        "Integer",
        &MOUSE_EVENT_OVER_ID_INLINE_OFFSET.to_string(),
    ));
    builder.emit(abi::store_u64(&value, &event, MOUSE_EVENT_OVER_ID_SLOT));

    // The ring's coordinates are integers; `vector::Float2` is two `Float`s.
    //
    // **The pair transposes here.** The decoder stores the wire's `y` in
    // `coord_a` and its `x` in `coord_b`, because `term::MouseEvent` is
    // row-then-column. `vector::Float2` is x-then-y, so `position.x` reads
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
        name: "pollMouseRaw",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::named(super::MOUSE_EVENT_TYPE),
            errors: vec!["ErrWrongMode", "ErrOutOfMemory"],
            body: Body::abi_function(lower_poll_mouse),
        }],
    });
    // The body is `helper_hit.rs`'s, gated on this name being called: a canvas program
    // that never polls the mouse compiles neither it nor the raw poll above.
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
            // Beyond the raw poll's two: `overID` builds the scene's geometry on this
            // thread (`helper_hit.rs:__canvas_hitBuild`), and those builders call the
            // checked conversions and the `math` members the renderer's do. The
            // sweep in `tests/mfb_body_declared_errors.rs` reads the set off the
            // source; nothing here was chosen by hand.
            errors: vec![
                "ErrWrongMode",
                "ErrOutOfMemory",
                "ErrOverflow",
                "ErrFloatDomain",
                "ErrInvalidArgument",
                "ErrIndexOutOfRange",
                "ErrInvalidFormat",
            ],
            body: Body::Rewrite("__canvas_pollMouse"),
        }],
    });
}
