//! GTK pointer publication (plan-172-J): where the pointer is, for the graphics thread.
//!
//! A canvas program's drawing area carries two controllers of its own — a motion
//! controller and a click gesture — whose only job is to publish the pointer to the
//! graphics state (`runtime::canvas::emit_publish_pointer`). They are attached for
//! every canvas program, whether or not it uses a mouse member, and they report
//! nothing to the program: the SGR path in `mouse.rs` is separate, on the window, and
//! untouched.
//!
//! # Why the area, when `mouse.rs` chose the window
//!
//! `mouse.rs` attaches to the window so one controller set serves the transcript, the
//! term area and the canvas, and pays for it by translating every point into whichever
//! child is live — which it knows from the mouse-mode word. Publication cannot use that
//! word (it must not depend on `canvas::enableMouse`) and has only one surface to care
//! about. On the area itself:
//!
//! * the coordinates GTK hands a controller are already the area's own, which are the
//!   surface's — there is no translation and no question of which child is live;
//! * the controllers are quiet whenever the canvas is not the window's child, because
//!   an unparented widget gets no events;
//! * `leave` on the motion controller is the pointer leaving the surface.
//!
//! Events bubble from the area to the window, and neither controller here claims its
//! sequence, so `mouse.rs`'s window controllers see everything they saw before.
//!
//! # Threading
//!
//! These run on the GTK main loop, as C callbacks from GTK's closure marshaller. They
//! write only the caller-saved scratch registers and park everything that crosses a
//! call on their own frame, so there is no callee-saved register to save (compare
//! `mouse.rs`, whose SGR formatter needs three).

use super::*;
use crate::codegen::runtime::canvas::{
    emit_publish_pointer, PointerRegs, GRAPHICS_OFFSET_HEIGHT, GRAPHICS_OFFSET_WIDTH,
    GRAPHICS_STATE_SYMBOL,
};

/// `GtkEventControllerMotion::motion` and `::enter` — both `(self, x, y, user)`.
pub(super) const POINTER_MOTION_SYMBOL: &str = "_mfb_gtkapp_pointer_motion";
/// `GtkEventControllerMotion::leave` — `(self, user)`.
pub(super) const POINTER_LEAVE_SYMBOL: &str = "_mfb_gtkapp_pointer_leave";
/// `GtkGestureClick::pressed` — `(self, n_press, x, y, user)`.
pub(super) const POINTER_PRESSED_SYMBOL: &str = "_mfb_gtkapp_pointer_pressed";
/// `GtkGestureClick::released` — the same shape.
pub(super) const POINTER_RELEASED_SYMBOL: &str = "_mfb_gtkapp_pointer_released";
/// Builds the two controllers and adds them to the canvas area.
pub(super) const POINTER_ATTACH_SYMBOL: &str = "_mfb_gtkapp_pointer_attach";

/// The buttons held over the canvas, as the published mask (bit 0 left, 1 middle, 2
/// right): one word, written only on the GTK main loop, by `pressed` and `released`.
///
/// **Kept here, not read from GDK.** A controller's event state has a bit per held
/// button, and reading the mask from it was the first cut — measured wrong in a real
/// GNOME Wayland window (box 2226): releasing the RIGHT button published "middle held",
/// and releasing the middle published "right held". GDK's Wayland backend numbers a
/// press's `button` left 1, middle 2, right 3, but sets the state bit by the evdev
/// order — left, RIGHT, middle — so bits 9 and 10 mean the opposite of
/// `GDK_BUTTON2_MASK` / `GDK_BUTTON3_MASK`, and of what X11 puts there. The button
/// NUMBER a press or release carries is the same on both, so the mask is built from
/// those. The state is still asked one question it answers the same way everywhere:
/// is ANY button held.
pub(super) const POINTER_HELD_SYMBOL: &str = "_mfb_gtkapp_pointer_held";

/// The two signal names `mouse.rs` does not already own a string for.
pub(super) const STR_ENTER: (&str, &str) = ("_mfb_gtkapp_str_enter", "enter");
pub(super) const STR_LEAVE: (&str, &str) = ("_mfb_gtkapp_str_leave", "leave");
/// The headless scripted pointer's two variables (`bootstrap.rs`).
pub(super) const STR_POINTER_X_ENV: (&str, &str) =
    ("_mfb_gtkapp_str_pointer_x", "MFB_CANVAS_POINTER_X");
pub(super) const STR_POINTER_Y_ENV: (&str, &str) =
    ("_mfb_gtkapp_str_pointer_y", "MFB_CANVAS_POINTER_Y");

/// Which callback is being generated.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum GtkPointerKind {
    /// `motion` and `enter`: a position.
    Motion,
    /// `leave`: no position; the pointer is off the surface.
    Leave,
    /// `pressed`: a position, and this event's button is down whatever the state says.
    Press,
    /// `released`: a position, and this event's button is up.
    Release,
}

impl GtkPointerKind {
    pub(super) const ALL: [GtkPointerKind; 4] = [
        GtkPointerKind::Motion,
        GtkPointerKind::Leave,
        GtkPointerKind::Press,
        GtkPointerKind::Release,
    ];

    fn symbol(self) -> &'static str {
        match self {
            GtkPointerKind::Motion => POINTER_MOTION_SYMBOL,
            GtkPointerKind::Leave => POINTER_LEAVE_SYMBOL,
            GtkPointerKind::Press => POINTER_PRESSED_SYMBOL,
            GtkPointerKind::Release => POINTER_RELEASED_SYMBOL,
        }
    }
}

// Frame layout. Everything crosses a call, so nothing stays in a register.
const OFF_LR: usize = 0;
const OFF_CONTROLLER: usize = 8;
const OFF_X: usize = 16;
const OFF_Y: usize = 24;
const OFF_BUTTONS: usize = 32;
const FRAME: usize = 48;

/// GDK's held-button bits in a modifier state, five of them from bit 8. Only "are any
/// set" is read — which button each one means differs between GDK's backends
/// (`POINTER_HELD_SYMBOL`).
const GDK_BUTTON_MASKS: u64 = 0x1f00;

/// Emit one controller callback.
///
/// `ordered` is whether the target orders the pointer words' accesses explicitly
/// (`emit_publish_pointer`): true for AArch64, false for x86-64.
///
/// **A null controller is "no event state".** The headless scripted pointer
/// (`bootstrap.rs`) calls the motion callback with no controller, as the scripted
/// resize calls the resize callback with no area: there is no GTK to ask, and the
/// buttons are simply none.
///
/// **The buttons are this module's own record** (`POINTER_HELD_SYMBOL`): `pressed` sets
/// its button's bit and `released` clears it, by the button NUMBER the gesture reports.
/// A release that never arrives — the grab was broken, the window lost the pointer
/// mid-press — would leave a bit set for good, so a motion whose event state shows no
/// button held at all clears the record.
///
/// **A point off the surface is published as outside, not dropped.** During a drag GTK
/// keeps delivering motion to the area with coordinates outside it and sends no
/// `leave`; the bounds test here is the only place the pointer is seen to go.
pub(super) fn emit_pointer_handler(
    kind: GtkPointerKind,
    ordered: bool,
) -> Result<CodeFunction, String> {
    let symbol = kind.symbol();
    let mut asm = Asm::new(symbol);
    let no_state = "pointer_no_state";
    let outside = "pointer_outside";

    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(FRAME));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        OFF_LR,
    ));
    asm.push(abi::store_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        OFF_CONTROLLER,
    ));
    if kind != GtkPointerKind::Leave {
        // The position is in d0/d1 for all three signatures that carry one.
        asm.push(abi::store_double(
            abi::FP_SCRATCH[0],
            abi::stack_pointer(),
            OFF_X,
        ));
        asm.push(abi::store_double(
            abi::FP_SCRATCH[1],
            abi::stack_pointer(),
            OFF_Y,
        ));
    }

    // --- The held buttons ---------------------------------------------------
    //
    // SCRATCH[0] is the record throughout; it is written back before the label.
    asm.local_address(abi::SCRATCH[4], POINTER_HELD_SYMBOL);
    asm.push(abi::load_u64(abi::SCRATCH[0], abi::SCRATCH[4], 0));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_BUTTONS,
    ));
    asm.push(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        OFF_CONTROLLER,
    ));
    asm.push(abi::compare_immediate(abi::c_arg(0), "0"));
    asm.push(abi::branch_eq(no_state));
    match kind {
        GtkPointerKind::Motion => {
            // No button held at all: whatever the record says, it is stale.
            let held = "pointer_some_held";
            asm.call_external("gtk_event_controller_get_current_event_state");
            asm.push(abi::move_immediate(
                abi::SCRATCH[1],
                "Integer",
                &GDK_BUTTON_MASKS.to_string(),
            ));
            asm.push(abi::and_registers(
                abi::SCRATCH[0],
                abi::c_return(0),
                abi::SCRATCH[1],
            ));
            asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
            asm.push(abi::branch_ne(held));
            asm.push(abi::store_u64(
                abi::SCRATCH[0],
                abi::stack_pointer(),
                OFF_BUTTONS,
            ));
            asm.local_address(abi::SCRATCH[4], POINTER_HELD_SYMBOL);
            asm.push(abi::store_u64(abi::SCRATCH[0], abi::SCRATCH[4], 0));
            asm.push(abi::label(held));
        }
        GtkPointerKind::Press | GtkPointerKind::Release => {
            // gtk_gesture_single_get_current_button() is 1 left, 2 middle, 3 right; a
            // button past 3 has no bit in the published mask and changes nothing.
            asm.call_external("gtk_gesture_single_get_current_button");
            asm.push(abi::move_immediate(abi::SCRATCH[1], "Integer", "0"));
            for (button, bit) in [("1", "1"), ("2", "2"), ("3", "4")] {
                let next = format!("pointer_not_button_{button}");
                asm.push(abi::compare_immediate(abi::c_return(0), button));
                asm.push(abi::branch_ne(&next));
                asm.push(abi::move_immediate(abi::SCRATCH[1], "Integer", bit));
                asm.push(abi::label(&next));
            }
            asm.push(abi::load_u64(
                abi::SCRATCH[0],
                abi::stack_pointer(),
                OFF_BUTTONS,
            ));
            if kind == GtkPointerKind::Press {
                asm.push(abi::or_registers(
                    abi::SCRATCH[0],
                    abi::SCRATCH[0],
                    abi::SCRATCH[1],
                ));
            } else {
                // record &= 7 ^ bit
                asm.push(abi::move_immediate(abi::SCRATCH[2], "Integer", "7"));
                asm.push(abi::exclusive_or_registers(
                    abi::SCRATCH[2],
                    abi::SCRATCH[2],
                    abi::SCRATCH[1],
                ));
                asm.push(abi::and_registers(
                    abi::SCRATCH[0],
                    abi::SCRATCH[0],
                    abi::SCRATCH[2],
                ));
            }
            asm.push(abi::store_u64(
                abi::SCRATCH[0],
                abi::stack_pointer(),
                OFF_BUTTONS,
            ));
            asm.local_address(abi::SCRATCH[4], POINTER_HELD_SYMBOL);
            asm.push(abi::store_u64(abi::SCRATCH[0], abi::SCRATCH[4], 0));
        }
        // The pointer left with whatever was held still held.
        GtkPointerKind::Leave => {}
    }
    asm.push(abi::label(no_state));

    // --- Position and inside ------------------------------------------------
    //
    // inside = 0 <= x < width && 0 <= y < height, against the published extent —
    // what the program draws against. `floor`, not a truncating convert: a point just
    // left of the area is -0.5, which truncation would call column 0.
    asm.push(abi::move_immediate(abi::SCRATCH[3], "Integer", "0"));
    if kind == GtkPointerKind::Leave {
        asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
        asm.push(abi::move_immediate(abi::SCRATCH[1], "Integer", "0"));
    } else {
        asm.push(abi::load_double(
            abi::FP_SCRATCH[0],
            abi::stack_pointer(),
            OFF_X,
        ));
        asm.push(abi::load_double(
            abi::FP_SCRATCH[1],
            abi::stack_pointer(),
            OFF_Y,
        ));
        asm.push(abi::float_floor_to_signed_x(
            abi::SCRATCH[0],
            abi::FP_SCRATCH[0],
        ));
        asm.push(abi::float_floor_to_signed_x(
            abi::SCRATCH[1],
            abi::FP_SCRATCH[1],
        ));
        asm.local_address(abi::SCRATCH[4], GRAPHICS_STATE_SYMBOL);
        asm.push(abi::load_u64(
            abi::SCRATCH[5],
            abi::SCRATCH[4],
            GRAPHICS_OFFSET_WIDTH,
        ));
        asm.push(abi::load_u64(
            abi::SCRATCH[6],
            abi::SCRATCH[4],
            GRAPHICS_OFFSET_HEIGHT,
        ));
        asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
        asm.push(abi::branch_lt(outside));
        asm.push(abi::compare_registers(abi::SCRATCH[0], abi::SCRATCH[5]));
        asm.push(abi::branch_ge(outside));
        asm.push(abi::compare_immediate(abi::SCRATCH[1], "0"));
        asm.push(abi::branch_lt(outside));
        asm.push(abi::compare_registers(abi::SCRATCH[1], abi::SCRATCH[6]));
        asm.push(abi::branch_ge(outside));
        asm.push(abi::move_immediate(abi::SCRATCH[3], "Integer", "1"));
        asm.push(abi::label(outside));
    }
    asm.push(abi::load_u64(
        abi::SCRATCH[2],
        abi::stack_pointer(),
        OFF_BUTTONS,
    ));

    emit_publish_pointer(
        symbol,
        ordered,
        &PointerRegs {
            x: abi::SCRATCH[0].into(),
            y: abi::SCRATCH[1].into(),
            buttons: abi::SCRATCH[2].into(),
            inside: abi::SCRATCH[3].into(),
            base: abi::SCRATCH[4].into(),
            seq: abi::SCRATCH[5].into(),
            addr: abi::SCRATCH[6].into(),
        },
        &mut asm.ins,
        &mut asm.rel,
    );

    asm.push(abi::load_u64(
        abi::link_register(),
        abi::stack_pointer(),
        OFF_LR,
    ));
    asm.push(abi::add_stack(FRAME));
    asm.push(abi::return_());
    asm.finish(symbol, "Nothing")
}

/// `_mfb_gtkapp_pointer_attach()` — build the two controllers and add them to the
/// canvas area. Called once, where the area is created (`bootstrap.rs`), on the GTK
/// main loop.
///
/// The click gesture is told to listen to EVERY button
/// (`gtk_gesture_single_set_button(gesture, 0)`): a `GtkGestureSingle` listens to the
/// primary button alone by default, and a held right or middle button is a held
/// button.
pub(super) fn emit_pointer_attach() -> Result<CodeFunction, String> {
    const OFF_ATTACH_LR: usize = 0;
    const OFF_ATTACH_CONTROLLER: usize = 8;
    const ATTACH_FRAME: usize = 16;

    let mut asm = Asm::new(POINTER_ATTACH_SYMBOL);
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(ATTACH_FRAME));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        OFF_ATTACH_LR,
    ));

    let connect = |asm: &mut Asm, signal: &str, handler: &str| {
        // g_signal_connect_data(controller, signal, handler, NULL, NULL, 0)
        asm.push(abi::load_u64(
            abi::c_arg(0),
            abi::stack_pointer(),
            OFF_ATTACH_CONTROLLER,
        ));
        asm.local_address(abi::c_arg(1), signal);
        asm.local_address(abi::c_arg(2), handler);
        asm.push(abi::move_immediate(abi::c_arg(3), "Integer", "0"));
        asm.push(abi::move_immediate(abi::c_arg(4), "Integer", "0"));
        asm.push(abi::move_immediate(abi::c_arg(5), "Integer", "0"));
        asm.call_external("g_signal_connect_data");
    };
    let add = |asm: &mut Asm| {
        // gtk_widget_add_controller(canvasArea, controller) — the area takes ownership.
        asm.load_state(abi::c_arg(0), ST_CANVAS_AREA);
        asm.push(abi::load_u64(
            abi::c_arg(1),
            abi::stack_pointer(),
            OFF_ATTACH_CONTROLLER,
        ));
        asm.call_external("gtk_widget_add_controller");
    };

    asm.call_external("gtk_event_controller_motion_new");
    asm.push(abi::store_u64(
        abi::c_return(0),
        abi::stack_pointer(),
        OFF_ATTACH_CONTROLLER,
    ));
    connect(&mut asm, mouse::STR_MOTION.0, POINTER_MOTION_SYMBOL);
    connect(&mut asm, STR_ENTER.0, POINTER_MOTION_SYMBOL);
    connect(&mut asm, STR_LEAVE.0, POINTER_LEAVE_SYMBOL);
    add(&mut asm);

    asm.call_external("gtk_gesture_click_new");
    asm.push(abi::store_u64(
        abi::c_return(0),
        abi::stack_pointer(),
        OFF_ATTACH_CONTROLLER,
    ));
    asm.push(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        OFF_ATTACH_CONTROLLER,
    ));
    asm.push(abi::move_immediate(abi::c_arg(1), "Integer", "0"));
    asm.call_external("gtk_gesture_single_set_button");
    connect(&mut asm, mouse::STR_PRESSED.0, POINTER_PRESSED_SYMBOL);
    connect(&mut asm, mouse::STR_RELEASED.0, POINTER_RELEASED_SYMBOL);
    add(&mut asm);

    asm.push(abi::load_u64(
        abi::link_register(),
        abi::stack_pointer(),
        OFF_ATTACH_LR,
    ));
    asm.push(abi::add_stack(ATTACH_FRAME));
    asm.push(abi::return_());
    asm.finish(POINTER_ATTACH_SYMBOL, "Nothing")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::error::constants::MOUSE_MODE_SYMBOL;

    /// The pointer is published whether or not the program uses a mouse member, and
    /// `_mfb_rt_mouse_mode` exists only in a program that does — so no callback here
    /// may name it, or a plain canvas program fails to link. Nor does one write to the
    /// input pipe: publication reports nothing to the program.
    #[test]
    fn a_pointer_callback_names_no_mode_word_and_no_pipe() {
        for ordered in [true, false] {
            for kind in GtkPointerKind::ALL {
                let func = emit_pointer_handler(kind, ordered).expect("emit");
                let names = |symbol: &str| func.relocations.iter().any(|r| r.to == symbol);
                assert!(!names(MOUSE_MODE_SYMBOL), "{}", kind.symbol());
                assert!(!names("write"), "{}", kind.symbol());
                assert!(names(GRAPHICS_STATE_SYMBOL), "{}", kind.symbol());
            }
        }
    }

    /// GTK enters each callback through its closure marshaller, a C caller that keeps
    /// its own state in the callee-saved registers. These bodies save none, so they
    /// must write none: `LOCAL[..]` are callee-saved on both targets.
    #[test]
    fn a_pointer_callback_writes_no_callee_saved_register() {
        let mut functions = vec![emit_pointer_attach().expect("attach")];
        for kind in GtkPointerKind::ALL {
            functions.push(emit_pointer_handler(kind, true).expect("emit"));
        }
        for func in &functions {
            for register in abi::LOCAL.iter() {
                let name = register.to_string();
                assert!(
                    !func
                        .instructions
                        .iter()
                        .any(|i| i.get("dst").as_deref() == Some(&name[..])),
                    "{}: writes {name}",
                    func.symbol
                );
            }
        }
    }

    /// The x86-64 lowering has no store-release, so the unordered shape must not hold
    /// one; and the ordered shape must, or AArch64 publishes with plain stores.
    #[test]
    fn only_the_ordered_shape_uses_release_stores() {
        use crate::arch::ops::CodeOp;
        for kind in GtkPointerKind::ALL {
            let releases = |ordered: bool| {
                emit_pointer_handler(kind, ordered)
                    .expect("emit")
                    .instructions
                    .iter()
                    .filter(|i| i.op == CodeOp::StlrU64)
                    .count()
            };
            assert_eq!(releases(false), 0, "{}", kind.symbol());
            // Two marks and four words.
            assert_eq!(releases(true), 6, "{}", kind.symbol());
        }
    }
}
