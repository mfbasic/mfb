//! macOS app-mode mouse input (plan-94-C).
//!
//! Every mouse IMP on both synthesized views is generated from one body:
//!
//! 1. **Gate** on the process-global `_mfb_rt_mouse_mode` word. A `TermView` IMP
//!    acts only when it reads *cells*, a `MFBCanvasView` IMP only when it reads
//!    *pixels*, and neither acts when it reads zero. The IMPs are always
//!    registered and the word decides whether they do anything — simpler than
//!    installing and removing tracking areas on enable/disable, and the same
//!    shape `didResize` uses (a state read inside an always-present IMP).
//! 2. **Locate** — `[event locationInWindow]`, then `convertPoint:fromView:nil`
//!    into the view.
//! 3. **Convert** — divide by the cached cell metrics for cells, or take the
//!    point as-is for pixels; clamp to the surface.
//! 4. **Encode and write** — format the SGR report plan-94-B's decoder expects
//!    and write the bytes to the window input pipe.
//!
//! **The Y-flip applies to one view and not the other**, which the plan did not
//! anticipate — it called for flipping on both. `TermView` overrides `isFlipped`
//! to return YES (`emit_term_view_is_flipped`), so `convertPoint:fromView:nil`
//! already hands back top-left-origin coordinates and flipping again would put
//! every event on the wrong half of the window. `MFBCanvasView` overrides only
//! `acceptsFirstResponder`, `keyDown:` and `setFrameSize:` — **not** `isFlipped`
//! — so it inherits NSView's bottom-left origin and does need the flip, to reach
//! the top-left origin `canvas::Point` documents. See plan-94-C Corrections.
//!
//! **No new event queue**: the bytes go into the pipe the two `keyDown:` IMPs
//! already write to, and the worker's stdin decoder turns them back into events.
//! That is what keeps the ring worker-local and atomics out of it.

use super::*;
use crate::codegen::error::constants::MOUSE_MODE_SYMBOL;
use crate::codegen::io::mouse::sgr::{
    emit_format_report, SgrReport, SgrScratch, SGR_REPORT_BYTES, SGR_SCRATCH_BYTES,
};
use crate::codegen::runtime::canvas::{
    GRAPHICS_OFFSET_HEIGHT, GRAPHICS_OFFSET_WIDTH, GRAPHICS_STATE_SYMBOL,
};

/// Which view an IMP belongs to, and therefore which mouse-mode value activates
/// it and how a point becomes a coordinate.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum MouseSurface {
    /// `TermView`: coordinates are character cells, from dividing by the cached
    /// `TV_CELL_W`/`TV_CELL_H`.
    Cells,
    /// `MFBCanvasView`: coordinates are surface pixels, taken directly.
    Pixels,
}

impl MouseSurface {
    /// The `_mfb_rt_mouse_mode` value that activates this surface's IMPs.
    fn mode_value(self) -> &'static str {
        match self {
            MouseSurface::Cells => "1",
            MouseSurface::Pixels => "2",
        }
    }
}

/// What the event means, which decides the SGR button code and terminator.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum MouseAction {
    /// A button went down. `button` names it.
    Press,
    /// A button came up.
    Release,
    /// Motion with `button` held.
    Drag,
    /// Motion with nothing held.
    Move,
    /// The wheel turned; the direction comes from the event's `deltaY` at run
    /// time, so this carries no static button.
    Wheel,
}

/// One generated IMP.
pub(super) struct MouseImp {
    /// The internal text symbol the IMP is registered under.
    pub(super) symbol: &'static str,
    /// The Objective-C selector it overrides.
    pub(super) selector: (&'static str, &'static str),
    pub(super) surface: MouseSurface,
    pub(super) action: MouseAction,
    /// SGR button number: 0 left, 1 middle, 2 right. Ignored for `Move` and
    /// `Wheel`.
    pub(super) button: u64,
}

// AppKit modifier-flag bits, and the SGR bits they map onto. The two encodings
// are unrelated, which is why the mapping is spelled out rather than masked.
const NS_SHIFT: u64 = 1 << 17;
const NS_CONTROL: u64 = 1 << 18;
const NS_OPTION: u64 = 1 << 19;
const SGR_SHIFT: u64 = 4;
const SGR_ALT: u64 = 8;
const SGR_CTRL: u64 = 16;
/// Bit 5 of the SGR button code: this report is motion, not a click.
const SGR_MOTION: u64 = 32;
/// The low-bits value meaning "no button", which a bare motion report carries.
const SGR_NO_BUTTON: u64 = 3;
const SGR_WHEEL_UP: u64 = 64;
const SGR_WHEEL_DOWN: u64 = 65;

/// Frame layout for a generated IMP.
///
/// `lr` and the callee-saved registers the ObjC calls clobber are parked at the
/// bottom; the report buffer and the decimal scratch sit above them. A mouse IMP
/// runs on the UI thread with no arena base, so every value it needs lives here
/// or in a callee-saved register.
const OFF_LR: usize = 0;
const OFF_SELF: usize = 8;
const OFF_EVENT: usize = 16;
const OFF_POINT_X: usize = 24;
const OFF_POINT_Y: usize = 32;
const OFF_CODE: usize = 40;
const OFF_X: usize = 48;
const OFF_Y: usize = 56;
const OFF_TERMINATOR: usize = 64;
const OFF_REPORT: usize = 72;
const OFF_SCRATCH: usize = OFF_REPORT + SGR_REPORT_BYTES;
const FRAME: usize = {
    let raw = OFF_SCRATCH + SGR_SCRATCH_BYTES;
    // 16-aligned, as every AArch64 frame must be.
    (raw + 15) / 16 * 16
};

/// Emit one mouse IMP.
pub(super) fn emit_mouse_imp(imp: &MouseImp) -> CodeFunction {
    let mut asm = Asm::new(imp.symbol);
    let sym = imp.symbol;
    let done = format!("{sym}_done");

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
        OFF_SELF,
    ));
    asm.push(abi::store_u64(
        abi::c_arg(2),
        abi::stack_pointer(),
        OFF_EVENT,
    ));

    // --- 1. Gate on the mouse-mode word -------------------------------------
    //
    // First act, and cheap on purpose: an un-enabled program pays one load and a
    // compare per motion event, which is the cost of registering the IMPs
    // unconditionally.
    asm.local_address(abi::SCRATCH[0], MOUSE_MODE_SYMBOL);
    asm.push(abi::load_u64(abi::SCRATCH[0], abi::SCRATCH[0], 0));
    asm.push(abi::compare_immediate(abi::SCRATCH[0], imp.surface.mode_value()));
    asm.push(abi::branch_ne(&done));

    // --- 2. Locate ----------------------------------------------------------
    //
    // `[event locationInWindow]` returns an NSPoint — two doubles in d0/d1 under
    // the AArch64 ObjC calling convention, not a pointer.
    asm.load_selector(SEL_LOCATION_IN_WINDOW.0);
    asm.push(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        OFF_EVENT,
    ));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::store_double(abi::FP_SCRATCH[0], abi::stack_pointer(), OFF_POINT_X));
    asm.push(abi::store_double(abi::FP_SCRATCH[1], abi::stack_pointer(), OFF_POINT_Y));

    // `[self convertPoint:loc fromView:nil]` — the point goes back in d0/d1 and
    // `nil` (the window's own space) in the third integer argument.
    asm.load_selector(SEL_CONVERT_POINT_FROM_VIEW.0);
    asm.push(abi::load_double(abi::FP_SCRATCH[0], abi::stack_pointer(), OFF_POINT_X));
    asm.push(abi::load_double(abi::FP_SCRATCH[1], abi::stack_pointer(), OFF_POINT_Y));
    asm.push(abi::move_immediate(abi::c_arg(2), "Integer", "0"));
    asm.push(abi::load_u64(abi::c_arg(0), abi::stack_pointer(), OFF_SELF));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    // No Y-flip: both views are `isFlipped`, so this is already top-left origin.
    asm.push(abi::store_double(abi::FP_SCRATCH[0], abi::stack_pointer(), OFF_POINT_X));
    asm.push(abi::store_double(abi::FP_SCRATCH[1], abi::stack_pointer(), OFF_POINT_Y));

    // --- 3. Convert to surface coordinates ----------------------------------
    emit_to_surface_coords(&mut asm, imp, &done);

    // --- 4. Compose the SGR button code -------------------------------------
    emit_button_code(&mut asm, imp, &done);

    // --- 5. Format and write ------------------------------------------------
    //
    // The formatter is pure arithmetic and stores, so it appends straight into
    // this IMP's instruction stream. It names PHYSICAL registers: these bodies
    // never reach the vreg allocator, so a `%v0` would arrive at the assembler
    // verbatim and be rejected. x9-x16 are the caller-save scratch bank and
    // nothing of ours is live in them here — the point, the code and the
    // coordinates are all parked on the stack across the ObjC calls above.
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
        // The report's own fields go in registers the formatter does not touch.
        asm.push(abi::add_immediate(abi::c_arg(2), abi::stack_pointer(), OFF_REPORT));
        asm.push(abi::load_u64(abi::c_arg(3), abi::stack_pointer(), OFF_CODE));
        asm.push(abi::load_u64(abi::c_arg(4), abi::stack_pointer(), OFF_X));
        asm.push(abi::load_u64(abi::c_arg(5), abi::stack_pointer(), OFF_Y));
        asm.push(abi::load_u64(abi::c_arg(6), abi::stack_pointer(), OFF_TERMINATOR));
        // One-based on the wire, in both units, exactly as a terminal sends.
        asm.push(abi::add_immediate(abi::c_arg(4), abi::c_arg(4), 1));
        asm.push(abi::add_immediate(abi::c_arg(5), abi::c_arg(5), 1));
        emit_format_report(
            &SgrReport {
                button_code: abi::c_arg(3),
                x: abi::c_arg(4),
                y: abi::c_arg(5),
                terminator: abi::c_arg(6),
            },
            &abi::c_arg(2),
            &abi::c_arg(7),
            OFF_SCRATCH,
            sym,
            &scratch,
            &mut asm.ins,
        );
        // The length lands where the x coordinate was; x is dead once formatted.
        asm.push(abi::store_u64(abi::c_arg(7), abi::stack_pointer(), OFF_X));
    }

    // write(pipeFd, report, len). The pipe write end is O_NONBLOCK (bug-114), so
    // a full pipe drops the report rather than hanging the UI thread — the right
    // trade for motion, where the next event is along in milliseconds anyway.
    emit_pipe_write(&mut asm, OFF_REPORT, OFF_X);

    asm.push(abi::label(&done));
    asm.push(abi::load_u64(
        abi::link_register(),
        abi::stack_pointer(),
        OFF_LR,
    ));
    asm.push(abi::add_stack(FRAME));
    asm.push(abi::return_());

    CodeFunction {
        name: format!("macapp.mouse.{}", short_name(imp)),
        symbol: sym.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions: asm.ins,
        relocations: asm.rel,
    }
}

/// A short human name for the `.ncode` dump, so a golden diff says which IMP
/// moved.
fn short_name(imp: &MouseImp) -> String {
    let surface = match imp.surface {
        MouseSurface::Cells => "term",
        MouseSurface::Pixels => "canvas",
    };
    format!("{surface}.{}", imp.selector.1.trim_end_matches(':'))
}

/// Turn the view-space point at `OFF_POINT_X`/`OFF_POINT_Y` into integer surface
/// coordinates at `OFF_X`/`OFF_Y`, or branch to `done` if it lands outside.
fn emit_to_surface_coords(asm: &mut Asm, imp: &MouseImp, done: &str) {
    let sym = imp.symbol;
    match imp.surface {
        MouseSurface::Cells => {
            // The cell metrics live in the view's TVSTATE associated object. A
            // null TVSTATE means the grid was never built, so there are no cells
            // to report a position in.
            asm.push(abi::load_u64(abi::c_arg(0), abi::stack_pointer(), OFF_SELF));
            asm.local_address(abi::c_arg(1), TVSTATE_ASSOC_KEY);
            asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
            asm.push(abi::compare_immediate(abi::c_return(0), "0"));
            asm.push(abi::branch_eq(done));
            asm.push(abi::move_register(abi::LOCAL[0], abi::c_return(0)));
            // col = floor(x / cellW), row = floor(y / cellH). `floor` and not a
            // truncating convert: a negative coordinate (a drag that left the
            // window) must land outside the grid so the clamp below rejects it,
            // and truncation would map -0.5 onto column 0.
            asm.push(abi::load_double(abi::FP_SCRATCH[2], abi::LOCAL[0], TV_CELL_W_OFFSET));
            asm.push(abi::load_double(abi::FP_SCRATCH[3], abi::LOCAL[0], TV_CELL_H_OFFSET));
            asm.push(abi::load_double(abi::FP_SCRATCH[0], abi::stack_pointer(), OFF_POINT_X));
            asm.push(abi::load_double(abi::FP_SCRATCH[1], abi::stack_pointer(), OFF_POINT_Y));
            asm.push(abi::float_divide_d(abi::FP_SCRATCH[0], abi::FP_SCRATCH[0], abi::FP_SCRATCH[2]));
            asm.push(abi::float_divide_d(abi::FP_SCRATCH[1], abi::FP_SCRATCH[1], abi::FP_SCRATCH[3]));
            asm.push(abi::float_floor_to_signed_x(abi::SCRATCH[0], abi::FP_SCRATCH[0]));
            asm.push(abi::float_floor_to_signed_x(abi::SCRATCH[1], abi::FP_SCRATCH[1]));
            asm.push(abi::store_u64(abi::SCRATCH[0], abi::stack_pointer(), OFF_X));
            asm.push(abi::store_u64(abi::SCRATCH[1], abi::stack_pointer(), OFF_Y));
            // Reject anything off the grid rather than clamping it. A click on the
            // window chrome is not a click on cell (0,0), and reporting it as one
            // would put a phantom event under the user's first row.
            asm.push(abi::load_u64(abi::SCRATCH[2], abi::LOCAL[0], TV_COLS_OFFSET));
            asm.push(abi::load_u64(abi::SCRATCH[3], abi::LOCAL[0], TV_ROWS_OFFSET));
            emit_range_check(asm, sym, abi::SCRATCH[0], abi::SCRATCH[2], done, "col");
            emit_range_check(asm, sym, abi::SCRATCH[1], abi::SCRATCH[3], done, "row");
        }
        MouseSurface::Pixels => {
            // No cell divide: the view-space point already IS the surface point,
            // because the canvas view is the surface.
            //
            // But it does need the Y-flip. `MFBCanvasView` does not override
            // `isFlipped` (unlike `TermView`), so `convertPoint:` hands back
            // NSView's bottom-left origin, while `canvas::Point` is documented
            // top-left with Y increasing downward — the origin every `DrawItem`
            // and every hit test uses. Without the flip a click near the top of
            // the window would be reported near the bottom of the scene.
            asm.local_address(abi::LOCAL[0], GRAPHICS_STATE_SYMBOL);
            asm.push(abi::load_u64(abi::SCRATCH[2], abi::LOCAL[0], GRAPHICS_OFFSET_WIDTH));
            asm.push(abi::load_u64(abi::SCRATCH[3], abi::LOCAL[0], GRAPHICS_OFFSET_HEIGHT));
            asm.push(abi::load_double(abi::FP_SCRATCH[0], abi::stack_pointer(), OFF_POINT_X));
            asm.push(abi::load_double(abi::FP_SCRATCH[1], abi::stack_pointer(), OFF_POINT_Y));
            // y = height - y. The published extent rather than the view's bounds:
            // it is what the program draws against, and `setFrameSize:` keeps the
            // two in step (plan-98-D Phase 3).
            asm.push(abi::signed_convert_to_float_d(abi::FP_SCRATCH[2], abi::SCRATCH[3]));
            asm.push(abi::float_subtract_d(abi::FP_SCRATCH[1], abi::FP_SCRATCH[2], abi::FP_SCRATCH[1]));
            asm.push(abi::float_floor_to_signed_x(abi::SCRATCH[0], abi::FP_SCRATCH[0]));
            asm.push(abi::float_floor_to_signed_x(abi::SCRATCH[1], abi::FP_SCRATCH[1]));
            asm.push(abi::store_u64(abi::SCRATCH[0], abi::stack_pointer(), OFF_X));
            asm.push(abi::store_u64(abi::SCRATCH[1], abi::stack_pointer(), OFF_Y));
            emit_range_check(asm, sym, abi::SCRATCH[0], abi::SCRATCH[2], done, "x");
            emit_range_check(asm, sym, abi::SCRATCH[1], abi::SCRATCH[3], done, "y");
        }
    }
}

/// Branch to `done` unless `0 <= value < limit`.
///
/// Signed, because a coordinate outside the window is genuinely negative and an
/// unsigned test would read it as enormous — which happens to reject it too, but
/// for the wrong reason and only by luck.
fn emit_range_check(asm: &mut Asm, sym: &str, value: &str, limit: &str, done: &str, tag: &str) {
    let ok = format!("{sym}_{tag}_ok");
    asm.push(abi::compare_immediate(value, "0"));
    asm.push(abi::branch_lt(done));
    asm.push(abi::compare_registers(value, limit));
    asm.push(abi::branch_lt(&ok));
    asm.push(abi::branch(done));
    asm.push(abi::label(&ok));
}

/// Compose the SGR button code at `OFF_CODE` and the terminator at
/// `OFF_TERMINATOR`.
fn emit_button_code(asm: &mut Asm, imp: &MouseImp, done: &str) {
    let sym = imp.symbol;

    // The base code, before modifiers.
    match imp.action {
        MouseAction::Press | MouseAction::Release => {
            asm.push(abi::move_immediate(
                abi::SCRATCH[0],
                "Integer",
                &imp.button.to_string(),
            ));
        }
        MouseAction::Drag => {
            asm.push(abi::move_immediate(
                abi::SCRATCH[0],
                "Integer",
                &(imp.button | SGR_MOTION).to_string(),
            ));
        }
        MouseAction::Move => {
            asm.push(abi::move_immediate(
                abi::SCRATCH[0],
                "Integer",
                &(SGR_NO_BUTTON | SGR_MOTION).to_string(),
            ));
        }
        MouseAction::Wheel => {
            // Direction from `[event deltaY]`, which returns a double. A zero
            // delta is a horizontal-only scroll and has no up/down meaning, so it
            // reports nothing at all rather than guessing.
            let up = format!("{sym}_wheel_up");
            let set = format!("{sym}_wheel_set");
            asm.load_selector(SEL_DELTA_Y.0);
            asm.push(abi::load_u64(
                abi::c_arg(0),
                abi::stack_pointer(),
                OFF_EVENT,
            ));
            asm.call_external("_objc_msgSend", LIB_OBJC);
            asm.push(abi::float_compare_zero_d(abi::FP_SCRATCH[0]));
            asm.push(abi::branch_eq(done));
            asm.push(abi::branch_gt(&up));
            asm.push(abi::move_immediate(
                abi::SCRATCH[0],
                "Integer",
                &SGR_WHEEL_DOWN.to_string(),
            ));
            asm.push(abi::branch(&set));
            asm.push(abi::label(&up));
            asm.push(abi::move_immediate(
                abi::SCRATCH[0],
                "Integer",
                &SGR_WHEEL_UP.to_string(),
            ));
            asm.push(abi::label(&set));
        }
    }
    asm.push(abi::store_u64(abi::SCRATCH[0], abi::stack_pointer(), OFF_CODE));

    // Modifiers. The wheel carries them too — a shift-scroll is a real gesture —
    // so this is unconditional.
    asm.load_selector(SEL_MODIFIER_FLAGS.0);
    asm.push(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        OFF_EVENT,
    ));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[0], abi::c_return(0)));
    asm.push(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), OFF_CODE));
    for (ns_bit, sgr_bit, tag) in [
        (NS_SHIFT, SGR_SHIFT, "shift"),
        (NS_OPTION, SGR_ALT, "alt"),
        (NS_CONTROL, SGR_CTRL, "ctrl"),
    ] {
        let skip = format!("{sym}_mod_{tag}");
        asm.push(abi::move_immediate(abi::SCRATCH[1], "Integer", &ns_bit.to_string()));
        asm.push(abi::and_registers(abi::SCRATCH[2], abi::LOCAL[0], abi::SCRATCH[1]));
        asm.push(abi::compare_immediate(abi::SCRATCH[2], "0"));
        asm.push(abi::branch_eq(&skip));
        asm.push(abi::move_immediate(abi::SCRATCH[1], "Integer", &sgr_bit.to_string()));
        asm.push(abi::or_registers(abi::SCRATCH[0], abi::SCRATCH[0], abi::SCRATCH[1]));
        asm.push(abi::label(&skip));
    }
    asm.push(abi::store_u64(abi::SCRATCH[0], abi::stack_pointer(), OFF_CODE));

    // `'m'` marks a release and `'M'` everything else — the one place the wire
    // distinguishes them, since the button code cannot.
    let terminator = if imp.action == MouseAction::Release {
        b'm'
    } else {
        b'M'
    };
    asm.push(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        &terminator.to_string(),
    ));
    asm.push(abi::store_u64(abi::SCRATCH[0], abi::stack_pointer(), OFF_TERMINATOR));
}

/// `write(pipeWriteFd, sp + buf_offset, [sp + len_offset])`.
///
/// The fd is the one the two `keyDown:` IMPs already use, reached the same way:
/// an associated object on `NSApp` under `PIPE_ASSOC_KEY`. Reusing it is the
/// whole point of the injection design — the backend needs no channel of its own.
fn emit_pipe_write(asm: &mut Asm, buf_offset: usize, len_offset: usize) {
    asm.external_data(abi::LOCAL[0], CLASS_NS_APPLICATION, LIB_APPKIT);
    asm.load_selector(SEL_SHARED_APPLICATION.0);
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[0]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(abi::c_arg(0), abi::c_return(0)));
    asm.local_address(abi::c_arg(1), PIPE_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(abi::c_arg(0), abi::c_return(0))); // write fd
    asm.push(abi::add_immediate(
        abi::c_arg(1),
        abi::stack_pointer(),
        buf_offset,
    ));
    asm.push(abi::load_u64(
        abi::c_arg(2),
        abi::stack_pointer(),
        len_offset,
    ));
    asm.call_external("_write", LIB_SYSTEM);
}

/// Every mouse IMP both views install, in registration order.
///
/// One table drives three things — the emitted functions, the `class_addMethod`
/// calls, and the tests — so a selector can never be registered against an IMP
/// that was not emitted, which is the failure mode that made the canvas
/// `setFrameSize:` gate necessary (an installed IMP naming a symbol that did not
/// exist).
///
/// Eleven per view. The three button families each contribute press, release and
/// drag; motion and the wheel contribute one apiece. `otherMouse*` is AppKit's
/// name for "neither left nor right", which on a wheel mouse is the wheel
/// pressed down — the middle button.
pub(super) fn mouse_imps(surface: MouseSurface) -> Vec<MouseImp> {
    let term = surface == MouseSurface::Cells;
    let sym = |t: &'static str, c: &'static str| if term { t } else { c };
    vec![
        MouseImp {
            symbol: sym(TERM_MOUSE_DOWN_SYMBOL, CANVAS_MOUSE_DOWN_SYMBOL),
            selector: SEL_MOUSE_DOWN,
            surface,
            action: MouseAction::Press,
            button: 0,
        },
        MouseImp {
            symbol: sym(TERM_MOUSE_UP_SYMBOL, CANVAS_MOUSE_UP_SYMBOL),
            selector: SEL_MOUSE_UP,
            surface,
            action: MouseAction::Release,
            button: 0,
        },
        MouseImp {
            symbol: sym(TERM_MOUSE_DRAGGED_SYMBOL, CANVAS_MOUSE_DRAGGED_SYMBOL),
            selector: SEL_MOUSE_DRAGGED,
            surface,
            action: MouseAction::Drag,
            button: 0,
        },
        MouseImp {
            symbol: sym(TERM_RMOUSE_DOWN_SYMBOL, CANVAS_RMOUSE_DOWN_SYMBOL),
            selector: SEL_RIGHT_MOUSE_DOWN,
            surface,
            action: MouseAction::Press,
            button: 2,
        },
        MouseImp {
            symbol: sym(TERM_RMOUSE_UP_SYMBOL, CANVAS_RMOUSE_UP_SYMBOL),
            selector: SEL_RIGHT_MOUSE_UP,
            surface,
            action: MouseAction::Release,
            button: 2,
        },
        MouseImp {
            symbol: sym(TERM_RMOUSE_DRAGGED_SYMBOL, CANVAS_RMOUSE_DRAGGED_SYMBOL),
            selector: SEL_RIGHT_MOUSE_DRAGGED,
            surface,
            action: MouseAction::Drag,
            button: 2,
        },
        MouseImp {
            symbol: sym(TERM_OMOUSE_DOWN_SYMBOL, CANVAS_OMOUSE_DOWN_SYMBOL),
            selector: SEL_OTHER_MOUSE_DOWN,
            surface,
            action: MouseAction::Press,
            button: 1,
        },
        MouseImp {
            symbol: sym(TERM_OMOUSE_UP_SYMBOL, CANVAS_OMOUSE_UP_SYMBOL),
            selector: SEL_OTHER_MOUSE_UP,
            surface,
            action: MouseAction::Release,
            button: 1,
        },
        MouseImp {
            symbol: sym(TERM_OMOUSE_DRAGGED_SYMBOL, CANVAS_OMOUSE_DRAGGED_SYMBOL),
            selector: SEL_OTHER_MOUSE_DRAGGED,
            surface,
            action: MouseAction::Drag,
            button: 1,
        },
        MouseImp {
            symbol: sym(TERM_MOUSE_MOVED_SYMBOL, CANVAS_MOUSE_MOVED_SYMBOL),
            selector: SEL_MOUSE_MOVED,
            surface,
            action: MouseAction::Move,
            button: 0,
        },
        MouseImp {
            symbol: sym(TERM_SCROLL_WHEEL_SYMBOL, CANVAS_SCROLL_SYMBOL),
            selector: SEL_SCROLL_WHEEL,
            surface,
            action: MouseAction::Wheel,
            button: 0,
        },
    ]
}

/// Emit every mouse IMP for the views this program can present. The pixel IMPs
/// read the canvas graphics state (`_mfb_rt_canvas_graphics`), a data object that
/// exists only in a program that uses `canvas::` — so a mouse program without it
/// gets only `TermView`'s cell IMPs, or its build names an undefined symbol.
pub(super) fn emit_mouse_imps(uses_canvas: bool) -> Vec<CodeFunction> {
    let surfaces: &[MouseSurface] = if uses_canvas {
        &[MouseSurface::Cells, MouseSurface::Pixels]
    } else {
        &[MouseSurface::Cells]
    };
    surfaces
        .iter()
        .copied()
        .flat_map(|surface| {
            mouse_imps(surface)
                .into_iter()
                .map(|imp| emit_mouse_imp(&imp))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// `class_addMethod` every mouse IMP onto the class in `class_reg`.
pub(super) fn emit_install_mouse_imps(
    asm: &mut Asm,
    class_reg: impl Into<Operand>,
    surface: MouseSurface,
) {
    let class_reg = class_reg.into();
    for imp in mouse_imps(surface) {
        asm.load_selector(imp.selector.0);
        asm.local_address(abi::c_arg(2), imp.symbol);
        asm.local_address(abi::c_arg(3), STR_INPUT_TYPES.0); // "v@:@"
        asm.push(abi::move_register(abi::c_arg(0), &class_reg));
        asm.call_external("_class_addMethod", LIB_OBJC);
    }
}

/// `[view addTrackingArea:[[NSTrackingArea alloc] initWithRect:… options:…
/// owner:view userInfo:nil]]`.
///
/// **This is why `mouseMoved:` is not enough on its own.** A view receives
/// `mouseMoved:` only if something asks for it; without a tracking area the IMP
/// is installed and simply never called — the quietest possible failure, where
/// drags work and plain motion does not.
///
/// Installed once at view construction rather than on `enableMouse`, so the mode
/// word stays the only thing that turns reporting on and off. The cost to a
/// program that never enables the mouse is one word load per motion event.
///
/// `NSTrackingInVisibleRect` (0x200) makes AppKit keep the area's geometry in
/// step with the view, so a window resize needs no re-install — and it makes the
/// rect argument ignored, which is why zeros are passed rather than a `bounds`
/// message whose result AppKit would discard. `NSTrackingActiveAlways` (0x80)
/// delivers motion whether or not the app is frontmost; `NSTrackingMouseMoved`
/// (0x02) is the event itself.
pub(super) fn emit_install_tracking_area(asm: &mut Asm, view_reg: impl Into<Operand>) {
    const NS_TRACKING_MOUSE_MOVED: u64 = 0x02;
    const NS_TRACKING_ACTIVE_ALWAYS: u64 = 0x80;
    const NS_TRACKING_IN_VISIBLE_RECT: u64 = 0x200;
    const OPTIONS: u64 =
        NS_TRACKING_MOUSE_MOVED | NS_TRACKING_ACTIVE_ALWAYS | NS_TRACKING_IN_VISIBLE_RECT;

    let view_reg = view_reg.into();
    // Sixteen bytes of its own, rather than a caller-supplied scratch register.
    // The tracking area has to survive two `objc_msgSend`s and a
    // `sel_registerName`, all of which clobber x0-x17 — and `load_selector`
    // clobbers x0 in particular, which is where each message leaves its result.
    // Borrowing a callee-saved register instead would mean knowing which ones are
    // live at each of the two call sites, and they differ.
    asm.push(abi::subtract_stack(16));

    // area = [NSTrackingArea alloc]
    asm.load_selector(SEL_ALLOC.0);
    asm.external_data(abi::c_arg(0), CLASS_NS_TRACKING_AREA, LIB_APPKIT);
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::store_u64(abi::c_return(0), abi::stack_pointer(), 0));

    // [area initWithRect:{0,0,0,0} options:OPTIONS owner:view userInfo:nil]
    //
    // The rect is zeros rather than the view's bounds because
    // `NSTrackingInVisibleRect` makes AppKit ignore it and track the view's
    // visible area instead — which is also what makes this survive a window
    // resize with no re-install.
    asm.load_selector(SEL_INIT_TRACKING.0);
    for reg in 0..4 {
        asm.push(abi::float_move_d_from_x(abi::FP_SCRATCH[reg], abi::ZERO));
    }
    asm.push(abi::move_immediate(
        abi::c_arg(2),
        "Integer",
        &OPTIONS.to_string(),
    ));
    asm.push(abi::move_register(abi::c_arg(3), &view_reg)); // owner
    asm.push(abi::move_immediate(abi::c_arg(4), "Integer", "0")); // userInfo
    asm.push(abi::load_u64(abi::c_arg(0), abi::stack_pointer(), 0));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::store_u64(abi::c_return(0), abi::stack_pointer(), 0));

    // [view addTrackingArea:area]
    asm.load_selector(SEL_ADD_TRACKING_AREA.0);
    asm.push(abi::load_u64(abi::c_arg(2), abi::stack_pointer(), 0));
    asm.push(abi::move_register(abi::c_arg(0), &view_reg));
    asm.call_external("_objc_msgSend", LIB_OBJC);

    asm.push(abi::add_stack(16));
}
