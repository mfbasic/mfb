//! GTK app-mode mouse input (plan-94-D).
//!
//! Three GTK4 event controllers, attached to the **window** beside the existing
//! key controller, each converting its event to surface coordinates and writing
//! the SGR bytes plan-94-B's decoder expects into the fd-0 window-input pipe the
//! backend already uses for keystrokes. No GTK-specific event queue.
//!
//! # Why the window and not the drawing area
//!
//! The key controller is on the window, and the comment at its wiring records
//! why: the window outlives the child swap that `gtk_window_set_child` performs
//! when the program moves between the transcript, the term area and the canvas
//! area — it is "the reason canvas mode inherits keyboard input for free". The
//! same argument applies here, and more strongly: one controller set then serves
//! all three surfaces with nothing to re-attach on a mode change.
//!
//! The price is that a gesture reports **window-relative** coordinates, so every
//! handler translates into whichever child is live before converting. That is the
//! cost the key controller does not pay, because a keystroke has no position.
//!
//! # Threading
//!
//! These run on the GTK main loop, which has no arena base — `x19` is not the
//! arena-state register here. Everything they need therefore comes from the
//! address-based `_mfb_gtkapp_state` global (`load_state`), exactly as the
//! `didResize` resize handler does, or from the process-global mouse-mode word.

use super::*;
use crate::codegen::error::constants::MOUSE_MODE_SYMBOL;
use crate::codegen::io::mouse::sgr::{
    emit_format_report, SgrReport, SgrScratch, SGR_REPORT_BYTES, SGR_SCRATCH_BYTES,
};
use crate::codegen::runtime::canvas::{
    GRAPHICS_OFFSET_HEIGHT, GRAPHICS_OFFSET_WIDTH, GRAPHICS_STATE_SYMBOL,
};

/// `GtkGestureClick::pressed` — a button went down.
pub(super) const MOUSE_PRESSED_SYMBOL: &str = "_mfb_gtkapp_mouse_pressed";
/// `GtkGestureClick::released` — a button came up.
pub(super) const MOUSE_RELEASED_SYMBOL: &str = "_mfb_gtkapp_mouse_released";
/// `GtkEventControllerMotion::motion` — the pointer moved, with or without a
/// button held.
pub(super) const MOUSE_MOTION_SYMBOL: &str = "_mfb_gtkapp_mouse_motion";
/// `GtkEventControllerScroll::scroll` — the wheel turned.
pub(super) const MOUSE_SCROLL_SYMBOL: &str = "_mfb_gtkapp_mouse_scroll";

/// The signal names the controllers are connected under.
pub(super) const STR_PRESSED: (&str, &str) = ("_mfb_gtkapp_str_pressed", "pressed");
pub(super) const STR_RELEASED: (&str, &str) = ("_mfb_gtkapp_str_released", "released");
pub(super) const STR_MOTION: (&str, &str) = ("_mfb_gtkapp_str_motion", "motion");
pub(super) const STR_SCROLL: (&str, &str) = ("_mfb_gtkapp_str_scroll", "scroll");

/// `GTK_EVENT_CONTROLLER_SCROLL_BOTH_AXES` — the flags
/// `gtk_event_controller_scroll_new` is created with.
///
/// Both axes rather than vertical-only: a horizontal-only scroll still arrives,
/// is recognised as carrying no vertical delta, and is dropped by the handler.
/// Asking for vertical alone would make GTK filter it instead, which is the same
/// outcome reached less explicitly.
const SCROLL_FLAGS_BOTH_AXES: &str = "3";

// GDK modifier bits, and the SGR bits they map onto.
const GDK_SHIFT_MASK: u64 = 1;
const GDK_CONTROL_MASK: u64 = 1 << 2;
const GDK_ALT_MASK: u64 = 1 << 3;
const SGR_SHIFT: u64 = 4;
const SGR_ALT: u64 = 8;
const SGR_CTRL: u64 = 16;
/// Bit 5 of the SGR button code: this report is motion, not a click.
const SGR_MOTION: u64 = 32;
/// The low-bits value meaning "no button" — what a bare motion report carries.
const SGR_NO_BUTTON: u64 = 3;
const SGR_WHEEL_UP: u64 = 64;
const SGR_WHEEL_DOWN: u64 = 65;

/// What the handler being generated reports.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum GtkMouseKind {
    /// `pressed`: the button comes from `gtk_gesture_single_get_current_button`.
    Press,
    /// `released`: same button source, `'m'` terminator.
    Release,
    /// `motion`: Move or Drag depending on the held-button mask in the current
    /// event state.
    Motion,
    /// `scroll`: direction from the `dy` delta.
    Scroll,
}

// Frame layout. Everything crosses a call, so nothing stays in a register.
const OFF_LR: usize = 0;
const OFF_CONTROLLER: usize = 8;
const OFF_X: usize = 16;
const OFF_Y: usize = 24;
const OFF_CODE: usize = 32;
const OFF_TERMINATOR: usize = 40;
const OFF_LEN: usize = 48;
const OFF_SAVED: usize = 56;
const OFF_REPORT: usize = 64;
const OFF_SCRATCH: usize = OFF_REPORT + SGR_REPORT_BYTES;
const FRAME: usize = {
    let raw = OFF_SCRATCH + SGR_SCRATCH_BYTES;
    (raw + 15) / 16 * 16
};

/// Emit one controller callback.
///
/// The GTK signal signatures differ in where the coordinates sit, which is the
/// only reason these are four functions rather than one:
///
/// ```text
/// pressed (self, n_press, x, y, user)   x,y in d0/d1; n_press in the 2nd int arg
/// released(self, n_press, x, y, user)   same
/// motion  (self, x, y, user)            x,y in d0/d1, no n_press
/// scroll  (self, dx, dy, user)          DELTAS in d0/d1, not a position
/// ```
pub(super) fn emit_mouse_handler(
    kind: GtkMouseKind,
    uses_canvas: bool,
) -> Result<CodeFunction, String> {
    let symbol = match kind {
        GtkMouseKind::Press => MOUSE_PRESSED_SYMBOL,
        GtkMouseKind::Release => MOUSE_RELEASED_SYMBOL,
        GtkMouseKind::Motion => MOUSE_MOTION_SYMBOL,
        GtkMouseKind::Scroll => MOUSE_SCROLL_SYMBOL,
    };
    let mut asm = Asm::new(symbol);
    let done = "mouse_done";

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
    // The coordinate pair is in d0/d1 for every one of the four signatures; only
    // its MEANING differs (a position, or a scroll delta).
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

    // --- Gate ---------------------------------------------------------------
    //
    // Zero means the program never asked; a non-zero value also tells the handler
    // which surface to translate into and which unit to report.
    asm.local_address(abi::SCRATCH[0], MOUSE_MODE_SYMBOL);
    asm.push(abi::load_u64(abi::LOCAL[0], abi::SCRATCH[0], 0));
    asm.push(abi::compare_immediate(abi::LOCAL[0], "0"));
    asm.push(abi::branch_eq(done));

    if kind == GtkMouseKind::Scroll {
        emit_scroll_code(&mut asm, done);
        // A scroll reports the position the pointer is at, which the signal does
        // not carry; the deltas in d0/d1 are not it. Zero is the honest stand-in
        // and it is what a wheel report's coordinates mean in practice — every
        // consumer reads a wheel event for its direction.
        asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
        asm.push(abi::store_u64(abi::SCRATCH[0], abi::stack_pointer(), OFF_X));
        asm.push(abi::store_u64(abi::SCRATCH[0], abi::stack_pointer(), OFF_Y));
    } else {
        emit_translate_and_convert(&mut asm, done, uses_canvas)?;
        emit_button_code(&mut asm, kind);
    }
    emit_modifiers(&mut asm);

    // --- Format and write ---------------------------------------------------
    {
        let scratch = SgrScratch {
            value: abi::SCRATCH[0].into(),
            ten: abi::SCRATCH[1].into(),
            quotient: abi::SCRATCH[2].into(),
            digit: abi::SCRATCH[3].into(),
            count: abi::SCRATCH[4].into(),
            addr: abi::SCRATCH[5].into(),
            scratch_base: abi::SCRATCH[6].into(),
        };
        asm.push(abi::add_immediate(
            abi::SCRATCH[7],
            abi::stack_pointer(),
            OFF_REPORT,
        ));
        asm.push(abi::load_u64(
            abi::SCRATCH[8],
            abi::stack_pointer(),
            OFF_CODE,
        ));
        asm.push(abi::load_u64(abi::SCRATCH[9], abi::stack_pointer(), OFF_X));
        asm.push(abi::load_u64(abi::SCRATCH[10], abi::stack_pointer(), OFF_Y));
        asm.push(abi::load_u64(
            abi::SCRATCH[11],
            abi::stack_pointer(),
            OFF_TERMINATOR,
        ));
        // One-based on the wire, in both units.
        asm.push(abi::add_immediate(abi::SCRATCH[9], abi::SCRATCH[9], 1));
        asm.push(abi::add_immediate(abi::SCRATCH[10], abi::SCRATCH[10], 1));
        emit_format_report(
            &SgrReport {
                button_code: abi::SCRATCH[8].into(),
                x: abi::SCRATCH[9].into(),
                y: abi::SCRATCH[10].into(),
                terminator: abi::SCRATCH[11].into(),
            },
            &abi::SCRATCH[7].into(),
            &abi::SCRATCH[12].into(),
            OFF_SCRATCH,
            symbol,
            &scratch,
            &mut asm.ins,
        );
        asm.push(abi::store_u64(
            abi::SCRATCH[12],
            abi::stack_pointer(),
            OFF_LEN,
        ));
    }
    // write(pipeFd, report, len). O_NONBLOCK (bug-114): a full pipe drops the
    // report rather than hanging the GTK main loop, which is the right trade for
    // motion — the next event is along in milliseconds.
    asm.load_state(abi::c_arg(0), ST_PIPE_WRITE_FD);
    asm.push(abi::add_immediate(
        abi::c_arg(1),
        abi::stack_pointer(),
        OFF_REPORT,
    ));
    asm.push(abi::load_u64(abi::c_arg(2), abi::stack_pointer(), OFF_LEN));
    asm.call_external("write");

    asm.push(abi::label(done));
    // `scroll` is the one signal with a return value: FALSE lets the event
    // propagate, which keeps a scrollable transcript scrollable while a program
    // is also reading wheel events.
    asm.push(abi::move_immediate(abi::c_return(0), "Integer", "0"));
    asm.push(abi::load_u64(
        abi::link_register(),
        abi::stack_pointer(),
        OFF_LR,
    ));
    asm.push(abi::add_stack(FRAME));
    asm.push(abi::return_());

    let returns = if kind == GtkMouseKind::Scroll {
        "Boolean"
    } else {
        "Nothing"
    };
    asm.finish(symbol, returns)
}

/// Translate the window-relative point into the live child area and convert it to
/// surface coordinates, or branch to `done` if it lands outside.
fn emit_translate_and_convert(asm: &mut Asm, done: &str, uses_canvas: bool) -> Result<(), String> {
    let cells = "mouse_cells";
    let have_area = "mouse_have_area";
    let convert_pixels = "mouse_pixels";

    // Which child is live follows from the mode word: 1 = the term area, 2 = the
    // canvas area. Reading it rather than probing the widget tree keeps this in
    // step with what the program asked for.
    asm.push(abi::compare_immediate(abi::LOCAL[0], "1"));
    asm.push(abi::branch_eq(cells));
    asm.load_state(abi::LOCAL[1], ST_CANVAS_AREA);
    asm.push(abi::branch(have_area));
    asm.push(abi::label(cells));
    asm.load_state(abi::LOCAL[1], ST_TERM_AREA);
    asm.push(abi::label(have_area));
    // No live child means no surface to report a position on.
    asm.push(abi::compare_immediate(abi::LOCAL[1], "0"));
    asm.push(abi::branch_eq(done));

    // gtk_widget_translate_coordinates(window, area, x, y, &ox, &oy) -> gboolean
    //
    // This is what attaching to the window costs. The two widgets share a toplevel
    // by construction, so the only way this returns FALSE is a child that has been
    // unparented mid-event — in which case there is nothing to report against.
    asm.load_state(abi::c_arg(0), ST_WINDOW);
    asm.push(abi::move_register(abi::c_arg(1), abi::LOCAL[1]));
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
    asm.push(abi::add_immediate(
        abi::c_arg(2),
        abi::stack_pointer(),
        OFF_X,
    ));
    asm.push(abi::add_immediate(
        abi::c_arg(3),
        abi::stack_pointer(),
        OFF_Y,
    ));
    asm.call_external("gtk_widget_translate_coordinates");
    asm.push(abi::compare_immediate(abi::c_return(0), "0"));
    asm.push(abi::branch_eq(done));

    // Cells: divide by the cached px-per-cell metrics and reject anything off the
    // grid. Rejecting rather than clamping, for the reason plan-94-C Corrections
    // C4 gives — an edge-clamped phantom event is indistinguishable from a real
    // one.
    asm.push(abi::compare_immediate(abi::LOCAL[0], "1"));
    asm.push(abi::branch_ne(convert_pixels));
    asm.load_state(abi::SCRATCH[0], ST_TERM_CELL_W);
    asm.push(abi::float_move_d_from_x(
        abi::FP_SCRATCH[2],
        abi::SCRATCH[0],
    ));
    asm.load_state(abi::SCRATCH[0], ST_TERM_CELL_H);
    asm.push(abi::float_move_d_from_x(
        abi::FP_SCRATCH[3],
        abi::SCRATCH[0],
    ));
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
    asm.push(abi::float_divide_d(
        abi::FP_SCRATCH[0],
        abi::FP_SCRATCH[0],
        abi::FP_SCRATCH[2],
    ));
    asm.push(abi::float_divide_d(
        abi::FP_SCRATCH[1],
        abi::FP_SCRATCH[1],
        abi::FP_SCRATCH[3],
    ));
    asm.push(abi::float_floor_to_signed_x(
        abi::SCRATCH[2],
        abi::FP_SCRATCH[0],
    ));
    asm.push(abi::float_floor_to_signed_x(
        abi::SCRATCH[3],
        abi::FP_SCRATCH[1],
    ));
    asm.push(abi::store_u64(abi::SCRATCH[2], abi::stack_pointer(), OFF_X));
    asm.push(abi::store_u64(abi::SCRATCH[3], abi::stack_pointer(), OFF_Y));
    asm.load_state(abi::SCRATCH[4], ST_TERM_COLS);
    asm.load_state(abi::SCRATCH[5], ST_TERM_ROWS);
    emit_range_check(asm, abi::SCRATCH[2], abi::SCRATCH[4], done, "col");
    emit_range_check(asm, abi::SCRATCH[3], abi::SCRATCH[5], done, "row");
    asm.push(abi::branch("mouse_converted"));

    // Pixels: the translated point already is the surface point, and GTK's origin
    // is top-left like `canvas::Point`'s, so there is no flip. Clamp-check against
    // the published surface extent, which is what the program draws against.
    asm.push(abi::label(convert_pixels));
    // The pixel surface's extent lives in the canvas graphics state, a data object
    // only a program that uses `canvas::` has. Without it there is no pixel surface
    // to report a position on (and naming the symbol would fail the build).
    if uses_canvas {
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
            abi::SCRATCH[2],
            abi::FP_SCRATCH[0],
        ));
        asm.push(abi::float_floor_to_signed_x(
            abi::SCRATCH[3],
            abi::FP_SCRATCH[1],
        ));
        asm.push(abi::store_u64(abi::SCRATCH[2], abi::stack_pointer(), OFF_X));
        asm.push(abi::store_u64(abi::SCRATCH[3], abi::stack_pointer(), OFF_Y));
        asm.local_address(abi::SCRATCH[6], GRAPHICS_STATE_SYMBOL);
        asm.push(abi::load_u64(
            abi::SCRATCH[4],
            abi::SCRATCH[6],
            GRAPHICS_OFFSET_WIDTH,
        ));
        asm.push(abi::load_u64(
            abi::SCRATCH[5],
            abi::SCRATCH[6],
            GRAPHICS_OFFSET_HEIGHT,
        ));
        emit_range_check(asm, abi::SCRATCH[2], abi::SCRATCH[4], done, "x");
        emit_range_check(asm, abi::SCRATCH[3], abi::SCRATCH[5], done, "y");
    } else {
        asm.push(abi::branch(done));
    }
    asm.push(abi::label("mouse_converted"));
    Ok(())
}

/// Branch to `done` unless `0 <= value < limit`, signed.
fn emit_range_check(asm: &mut Asm, value: &str, limit: &str, done: &str, tag: &str) {
    let ok = format!("mouse_{tag}_ok");
    asm.push(abi::compare_immediate(value, "0"));
    asm.push(abi::branch_lt(done));
    asm.push(abi::compare_registers(value, limit));
    asm.push(abi::branch_lt(&ok));
    asm.push(abi::branch(done));
    asm.push(abi::label(&ok));
}

/// The SGR button code and terminator for a click or a motion report.
fn emit_button_code(asm: &mut Asm, kind: GtkMouseKind) {
    match kind {
        GtkMouseKind::Press | GtkMouseKind::Release => {
            // gtk_gesture_single_get_current_button() is 1-based (1 left,
            // 2 middle, 3 right) and SGR is 0-based in the same order, so the code
            // is the button minus one. A button past 3 (a thumb button) has no SGR
            // spelling in the low two bits, so it is reported as its modulo —
            // which is what every terminal does with the same encoding.
            asm.push(abi::load_u64(
                abi::c_arg(0),
                abi::stack_pointer(),
                OFF_CONTROLLER,
            ));
            asm.call_external("gtk_gesture_single_get_current_button");
            asm.push(abi::subtract_immediate(
                abi::SCRATCH[0],
                abi::c_return(0),
                1,
            ));
            asm.push(abi::move_immediate(abi::SCRATCH[1], "Integer", "3"));
            asm.push(abi::and_registers(
                abi::SCRATCH[0],
                abi::SCRATCH[0],
                abi::SCRATCH[1],
            ));
            asm.push(abi::store_u64(
                abi::SCRATCH[0],
                abi::stack_pointer(),
                OFF_CODE,
            ));
        }
        GtkMouseKind::Motion => {
            // Drag or Move, decided by whether any button is held in the current
            // event state. GDK's button masks are bits 8-12, so a single masked
            // test covers all five without naming each.
            const GDK_BUTTON_MASKS: u64 = 0x1f00;
            let no_button = "mouse_motion_free";
            let set = "mouse_motion_set";
            asm.push(abi::load_u64(
                abi::c_arg(0),
                abi::stack_pointer(),
                OFF_CONTROLLER,
            ));
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
            asm.push(abi::branch_eq(no_button));
            // A held button: report Drag with button 0. GDK does not say WHICH
            // button a motion is under in a way SGR can carry per-event, and a
            // drag's button is established by the press that began it — which the
            // program has already seen.
            asm.push(abi::move_immediate(
                abi::SCRATCH[0],
                "Integer",
                &SGR_MOTION.to_string(),
            ));
            asm.push(abi::branch(set));
            asm.push(abi::label(no_button));
            asm.push(abi::move_immediate(
                abi::SCRATCH[0],
                "Integer",
                &(SGR_MOTION | SGR_NO_BUTTON).to_string(),
            ));
            asm.push(abi::label(set));
            asm.push(abi::store_u64(
                abi::SCRATCH[0],
                abi::stack_pointer(),
                OFF_CODE,
            ));
        }
        GtkMouseKind::Scroll => unreachable!("the wheel sets its own code"),
    }

    let terminator = if kind == GtkMouseKind::Release {
        b'm'
    } else {
        b'M'
    };
    asm.push(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        &terminator.to_string(),
    ));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_TERMINATOR,
    ));
}

/// The wheel's code, from the sign of the `dy` delta in d1.
fn emit_scroll_code(asm: &mut Asm, done: &str) {
    let up = "mouse_wheel_up";
    let set = "mouse_wheel_set";
    asm.push(abi::load_double(
        abi::FP_SCRATCH[1],
        abi::stack_pointer(),
        OFF_Y,
    ));
    asm.push(abi::float_compare_zero_d(abi::FP_SCRATCH[1]));
    // A horizontal-only scroll has no vertical delta and so no up/down meaning.
    // Dropped rather than guessed at.
    asm.push(abi::branch_eq(done));
    // GTK's dy is positive DOWNWARD (the content moves down), which is the
    // opposite sense to the terminal's "wheel up" — hence the inversion here
    // rather than the obvious mapping.
    asm.push(abi::branch_lt(up));
    asm.push(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        &SGR_WHEEL_DOWN.to_string(),
    ));
    asm.push(abi::branch(set));
    asm.push(abi::label(up));
    asm.push(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        &SGR_WHEEL_UP.to_string(),
    ));
    asm.push(abi::label(set));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_CODE,
    ));
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "77")); // 'M'
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_TERMINATOR,
    ));
}

/// OR the SGR modifier bits into the code from the controller's current event
/// state.
fn emit_modifiers(asm: &mut Asm) {
    asm.push(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        OFF_CONTROLLER,
    ));
    asm.call_external("gtk_event_controller_get_current_event_state");
    asm.push(abi::store_u64(
        abi::c_return(0),
        abi::stack_pointer(),
        OFF_SAVED,
    ));
    asm.push(abi::load_u64(abi::LOCAL[1], abi::stack_pointer(), OFF_CODE));
    for (gdk_bit, sgr_bit, tag) in [
        (GDK_SHIFT_MASK, SGR_SHIFT, "shift"),
        (GDK_ALT_MASK, SGR_ALT, "alt"),
        (GDK_CONTROL_MASK, SGR_CTRL, "ctrl"),
    ] {
        let skip = format!("mouse_mod_{tag}");
        asm.push(abi::load_u64(
            abi::SCRATCH[1],
            abi::stack_pointer(),
            OFF_SAVED,
        ));
        asm.push(abi::move_immediate(
            abi::SCRATCH[0],
            "Integer",
            &gdk_bit.to_string(),
        ));
        asm.push(abi::and_registers(
            abi::SCRATCH[0],
            abi::SCRATCH[1],
            abi::SCRATCH[0],
        ));
        asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
        asm.push(abi::branch_eq(&skip));
        asm.push(abi::move_immediate(
            abi::SCRATCH[0],
            "Integer",
            &sgr_bit.to_string(),
        ));
        asm.push(abi::or_registers(
            abi::LOCAL[1],
            abi::LOCAL[1],
            abi::SCRATCH[0],
        ));
        asm.push(abi::label(&skip));
    }
    asm.push(abi::store_u64(
        abi::LOCAL[1],
        abi::stack_pointer(),
        OFF_CODE,
    ));
}

/// Attach the three controllers to the window, beside the key controller.
///
/// `g_signal_connect_data(controller, signal, handler, NULL, NULL, 0)` then
/// `gtk_widget_add_controller(window, controller)`, which takes ownership — the
/// same two-step the key controller uses.
pub(super) fn emit_attach_controllers(asm: &mut Asm) {
    // The click gesture carries both press and release.
    asm.call_external("gtk_gesture_click_new");
    asm.push(abi::move_register(abi::LOCAL[0], abi::c_return(0)));
    for (signal, handler) in [
        (STR_PRESSED, MOUSE_PRESSED_SYMBOL),
        (STR_RELEASED, MOUSE_RELEASED_SYMBOL),
    ] {
        connect(asm, abi::LOCAL[0], signal.0, handler);
    }
    add_controller(asm, abi::LOCAL[0]);

    asm.call_external("gtk_event_controller_motion_new");
    asm.push(abi::move_register(abi::LOCAL[0], abi::c_return(0)));
    connect(asm, abi::LOCAL[0], STR_MOTION.0, MOUSE_MOTION_SYMBOL);
    add_controller(asm, abi::LOCAL[0]);

    asm.push(abi::move_immediate(
        abi::c_arg(0),
        "Integer",
        SCROLL_FLAGS_BOTH_AXES,
    ));
    asm.call_external("gtk_event_controller_scroll_new");
    asm.push(abi::move_register(abi::LOCAL[0], abi::c_return(0)));
    connect(asm, abi::LOCAL[0], STR_SCROLL.0, MOUSE_SCROLL_SYMBOL);
    add_controller(asm, abi::LOCAL[0]);
}

fn connect(asm: &mut Asm, controller: &str, signal_symbol: &str, handler: &str) {
    asm.local_address(abi::c_arg(1), signal_symbol);
    asm.local_address(abi::c_arg(2), handler);
    asm.push(abi::move_immediate(abi::c_arg(3), "Integer", "0"));
    asm.push(abi::move_immediate(abi::c_arg(4), "Integer", "0"));
    asm.push(abi::move_immediate(abi::c_arg(5), "Integer", "0"));
    asm.push(abi::move_register(abi::c_arg(0), controller));
    asm.call_external("g_signal_connect_data");
}

fn add_controller(asm: &mut Asm, controller: &str) {
    asm.load_state(abi::c_arg(0), ST_WINDOW);
    asm.push(abi::move_register(abi::c_arg(1), controller));
    asm.call_external("gtk_widget_add_controller");
}
