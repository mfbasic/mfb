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
//! the top-left origin `vector::Float2` documents. See plan-94-C Corrections.
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
    emit_publish_pointer, PointerRegs, GRAPHICS_OFFSET_HEIGHT, GRAPHICS_OFFSET_WIDTH,
    GRAPHICS_STATE_SYMBOL,
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
    /// plan-172-J: the pointer came onto the view. Published, never reported: the
    /// mouse surface has no "entered" event, and the first motion says where it is.
    Enter,
    /// plan-172-J: the pointer left the view. Published as outside, never reported.
    Exit,
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
/// `lr`, the IMP's own arguments and the caller's `LOCAL[0]` are parked at the
/// bottom; the report buffer and the decimal scratch sit above them. A mouse IMP
/// runs on the UI thread with no arena base, so every value it needs lives here,
/// because each ObjC call clobbers the caller-saved bank.
const OFF_LR: usize = 0;
const OFF_SELF: usize = 8;
const OFF_EVENT: usize = 16;
const OFF_POINT_X: usize = 24;
const OFF_POINT_Y: usize = 32;
const OFF_CODE: usize = 40;
const OFF_X: usize = 48;
const OFF_Y: usize = 56;
const OFF_TERMINATOR: usize = 64;
/// The caller's `LOCAL[0]` (`x19`), which the body writes: the coordinate
/// conversion, the modifier flags and the pipe write all park a value there
/// across an ObjC call (bug-993). An IMP is an AAPCS64 callee, and
/// `-[NSWindow sendEvent:]` keeps an object it later releases in `x19` across
/// the dispatch. Restored at `{sym}_done`, the single exit every path, early or
/// full, funnels through.
const OFF_SAVED_LOCAL0: usize = 72;
/// plan-172-J: the published button mask (bit 0 left, 1 middle, 2 right) and the
/// inside flag, parked across nothing — computed and published in one straight run —
/// but kept on the frame so the publish reads every value from one place.
const OFF_BUTTONS: usize = 80;
const OFF_INSIDE: usize = 88;
const OFF_REPORT: usize = 96;
const OFF_SCRATCH: usize = OFF_REPORT + SGR_REPORT_BYTES;
const FRAME: usize = {
    let raw = OFF_SCRATCH + SGR_SCRATCH_BYTES;
    // 16-aligned, as every AArch64 frame must be.
    (raw + 15) / 16 * 16
};

/// Emit one mouse IMP.
///
/// `reports` is whether the program uses the mouse members at all — whether there is
/// a mouse-mode word to gate on and a decoder at the other end of the pipe. A
/// `TermView` IMP exists only when it does. A `MFBCanvasView` IMP exists in every
/// canvas program (plan-172-J): it PUBLISHES the pointer to the graphics state first,
/// whatever the mode word says, and only then — when `reports` — gates and reports as
/// it always did. So in a program that never touches the mouse it is the publish and a
/// return.
pub(super) fn emit_mouse_imp(imp: &MouseImp, reports: bool) -> CodeFunction {
    let mut asm = Asm::new(imp.symbol);
    let sym = imp.symbol;
    let done = format!("{sym}_done");
    let pixels = imp.surface == MouseSurface::Pixels;
    let publish_only =
        pixels && (!reports || matches!(imp.action, MouseAction::Enter | MouseAction::Exit));

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
    asm.push(abi::store_u64(
        abi::LOCAL[0],
        abi::stack_pointer(),
        OFF_SAVED_LOCAL0,
    ));

    // --- 1. Gate on the mouse-mode word -------------------------------------
    //
    // First act on the cell surface, and cheap on purpose: an un-enabled program
    // pays one load and a compare per motion event, which is the cost of
    // registering the IMPs unconditionally. On the pixel surface the gate comes
    // AFTER the publish (step 3b): where the pointer is does not depend on whether
    // the program asked for mouse events.
    if !pixels {
        emit_mode_gate(&mut asm, imp, &done);
    }

    if imp.action == MouseAction::Exit {
        // Nothing to locate: the pointer is not on the view.
        emit_publish(&mut asm, imp, &done);
        return finish_imp(asm, imp, &done);
    }

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
    asm.push(abi::store_double(
        abi::FP_SCRATCH[0],
        abi::stack_pointer(),
        OFF_POINT_X,
    ));
    asm.push(abi::store_double(
        abi::FP_SCRATCH[1],
        abi::stack_pointer(),
        OFF_POINT_Y,
    ));

    // `[self convertPoint:loc fromView:nil]` — the point goes back in d0/d1 and
    // `nil` (the window's own space) in the third integer argument.
    asm.load_selector(SEL_CONVERT_POINT_FROM_VIEW.0);
    asm.push(abi::load_double(
        abi::FP_SCRATCH[0],
        abi::stack_pointer(),
        OFF_POINT_X,
    ));
    asm.push(abi::load_double(
        abi::FP_SCRATCH[1],
        abi::stack_pointer(),
        OFF_POINT_Y,
    ));
    asm.push(abi::move_immediate(abi::c_arg(2), "Integer", "0"));
    asm.push(abi::load_u64(abi::c_arg(0), abi::stack_pointer(), OFF_SELF));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    // No Y-flip: both views are `isFlipped`, so this is already top-left origin.
    asm.push(abi::store_double(
        abi::FP_SCRATCH[0],
        abi::stack_pointer(),
        OFF_POINT_X,
    ));
    asm.push(abi::store_double(
        abi::FP_SCRATCH[1],
        abi::stack_pointer(),
        OFF_POINT_Y,
    ));

    // --- 3. Convert to surface coordinates ----------------------------------
    emit_to_surface_coords(&mut asm, imp, &done);

    // --- 3b. Publish (pixel surface), then gate ------------------------------
    if pixels {
        emit_publish(&mut asm, imp, &done);
        if publish_only {
            return finish_imp(asm, imp, &done);
        }
        emit_mode_gate(&mut asm, imp, &done);
    }

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
        asm.push(abi::add_immediate(
            abi::c_arg(2),
            abi::stack_pointer(),
            OFF_REPORT,
        ));
        asm.push(abi::load_u64(abi::c_arg(3), abi::stack_pointer(), OFF_CODE));
        asm.push(abi::load_u64(abi::c_arg(4), abi::stack_pointer(), OFF_X));
        asm.push(abi::load_u64(abi::c_arg(5), abi::stack_pointer(), OFF_Y));
        asm.push(abi::load_u64(
            abi::c_arg(6),
            abi::stack_pointer(),
            OFF_TERMINATOR,
        ));
        // One-based on the wire, in both units, exactly as a terminal sends.
        asm.push(abi::add_immediate(abi::c_arg(4), abi::c_arg(4), 1));
        asm.push(abi::add_immediate(abi::c_arg(5), abi::c_arg(5), 1));
        emit_format_report(
            &SgrReport {
                button_code: abi::c_arg(3).into(),
                x: abi::c_arg(4).into(),
                y: abi::c_arg(5).into(),
                terminator: abi::c_arg(6).into(),
            },
            &abi::c_arg(2).into(),
            &abi::c_arg(7).into(),
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

    finish_imp(asm, imp, &done)
}

/// Branch to `done` unless the mouse-mode word names this IMP's surface.
fn emit_mode_gate(asm: &mut Asm, imp: &MouseImp, done: &str) {
    asm.local_address(abi::SCRATCH[0], MOUSE_MODE_SYMBOL);
    asm.push(abi::load_u64(abi::SCRATCH[0], abi::SCRATCH[0], 0));
    asm.push(abi::compare_immediate(
        abi::SCRATCH[0],
        imp.surface.mode_value(),
    ));
    asm.push(abi::branch_ne(done));
}

/// plan-172-J: publish the pointer to the graphics state — the position at
/// `OFF_X`/`OFF_Y`, the held buttons, and whether the position is on the surface —
/// then branch to `done` if it is not. `Exit` publishes "outside" and no position.
///
/// **The buttons are AppKit's current set, corrected by this event.**
/// `+[NSEvent pressedMouseButtons]` is the hardware's state NOW, and a quick click
/// can be released before its `mouseDown:` is dispatched — the press would publish
/// with nothing held and a reader waiting for it would never see it. So a press or a
/// drag sets its own button's bit and a release clears it, whatever AppKit says.
/// AppKit numbers the buttons left, right, other; the published mask is left, middle,
/// right (the SGR order), so bits 1 and 2 change places.
///
/// **A point off the surface is published, not dropped.** A drag that leaves the view
/// keeps arriving here with coordinates outside it; the old range check threw those
/// away, which is right for a report and wrong for the pointer — it is how the
/// graphics thread learns the pointer is gone while a button is held, when AppKit
/// sends no `mouseExited:`.
fn emit_publish(asm: &mut Asm, imp: &MouseImp, done: &str) {
    let sym = imp.symbol;
    let outside = format!("{sym}_outside");

    asm.load_selector(SEL_PRESSED_MOUSE_BUTTONS.0);
    asm.external_data(abi::c_arg(0), CLASS_NS_EVENT, LIB_APPKIT);
    asm.call_external("_objc_msgSend", LIB_OBJC);
    // left stays bit 0; AppKit's bit 1 (right) -> 2, its bit 2 (other) -> 1.
    asm.push(abi::move_immediate(abi::SCRATCH[4], "Integer", "1"));
    asm.push(abi::and_registers(
        abi::SCRATCH[2],
        abi::c_return(0),
        abi::SCRATCH[4],
    ));
    for (from_bit, to_bit) in [(1u8, 2u8), (2, 1)] {
        asm.push(abi::shift_right_immediate(
            abi::SCRATCH[3],
            abi::c_return(0),
            from_bit,
        ));
        asm.push(abi::and_registers(
            abi::SCRATCH[3],
            abi::SCRATCH[3],
            abi::SCRATCH[4],
        ));
        asm.push(abi::shift_left_immediate(
            abi::SCRATCH[3],
            abi::SCRATCH[3],
            to_bit,
        ));
        asm.push(abi::or_registers(
            abi::SCRATCH[2],
            abi::SCRATCH[2],
            abi::SCRATCH[3],
        ));
    }
    let bit = 1u64 << imp.button;
    match imp.action {
        MouseAction::Press | MouseAction::Drag => {
            asm.push(abi::move_immediate(
                abi::SCRATCH[3],
                "Integer",
                &bit.to_string(),
            ));
            asm.push(abi::or_registers(
                abi::SCRATCH[2],
                abi::SCRATCH[2],
                abi::SCRATCH[3],
            ));
        }
        MouseAction::Release => {
            asm.push(abi::move_immediate(
                abi::SCRATCH[3],
                "Integer",
                &(7 ^ bit).to_string(),
            ));
            asm.push(abi::and_registers(
                abi::SCRATCH[2],
                abi::SCRATCH[2],
                abi::SCRATCH[3],
            ));
        }
        MouseAction::Move | MouseAction::Wheel | MouseAction::Enter | MouseAction::Exit => {}
    }
    asm.push(abi::store_u64(
        abi::SCRATCH[2],
        abi::stack_pointer(),
        OFF_BUTTONS,
    ));

    // inside = 0 <= x < width && 0 <= y < height, against the published extent.
    asm.push(abi::move_immediate(abi::SCRATCH[3], "Integer", "0"));
    if imp.action == MouseAction::Exit {
        asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
        asm.push(abi::move_immediate(abi::SCRATCH[1], "Integer", "0"));
    } else {
        asm.push(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), OFF_X));
        asm.push(abi::load_u64(abi::SCRATCH[1], abi::stack_pointer(), OFF_Y));
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
        asm.push(abi::branch_lt(&outside));
        asm.push(abi::compare_registers(abi::SCRATCH[0], abi::SCRATCH[5]));
        asm.push(abi::branch_ge(&outside));
        asm.push(abi::compare_immediate(abi::SCRATCH[1], "0"));
        asm.push(abi::branch_lt(&outside));
        asm.push(abi::compare_registers(abi::SCRATCH[1], abi::SCRATCH[6]));
        asm.push(abi::branch_ge(&outside));
        asm.push(abi::move_immediate(abi::SCRATCH[3], "Integer", "1"));
        asm.push(abi::label(&outside));
    }
    asm.push(abi::store_u64(
        abi::SCRATCH[3],
        abi::stack_pointer(),
        OFF_INSIDE,
    ));

    emit_publish_pointer(
        sym,
        true,
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
    // Off the surface there is nothing to report against.
    asm.push(abi::load_u64(
        abi::SCRATCH[3],
        abi::stack_pointer(),
        OFF_INSIDE,
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[3], "0"));
    asm.push(abi::branch_eq(done));
}

/// The single exit every path funnels through, and the function around it.
fn finish_imp(mut asm: Asm, imp: &MouseImp, done: &str) -> CodeFunction {
    let sym = imp.symbol;
    asm.push(abi::label(done));
    asm.push(abi::load_u64(
        abi::LOCAL[0],
        abi::stack_pointer(),
        OFF_SAVED_LOCAL0,
    ));
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
            asm.push(abi::load_double(
                abi::FP_SCRATCH[2],
                abi::LOCAL[0],
                TV_CELL_W_OFFSET,
            ));
            asm.push(abi::load_double(
                abi::FP_SCRATCH[3],
                abi::LOCAL[0],
                TV_CELL_H_OFFSET,
            ));
            asm.push(abi::load_double(
                abi::FP_SCRATCH[0],
                abi::stack_pointer(),
                OFF_POINT_X,
            ));
            asm.push(abi::load_double(
                abi::FP_SCRATCH[1],
                abi::stack_pointer(),
                OFF_POINT_Y,
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
                abi::SCRATCH[0],
                abi::FP_SCRATCH[0],
            ));
            asm.push(abi::float_floor_to_signed_x(
                abi::SCRATCH[1],
                abi::FP_SCRATCH[1],
            ));
            asm.push(abi::store_u64(abi::SCRATCH[0], abi::stack_pointer(), OFF_X));
            asm.push(abi::store_u64(abi::SCRATCH[1], abi::stack_pointer(), OFF_Y));
            // Reject anything off the grid rather than clamping it. A click on the
            // window chrome is not a click on cell (0,0), and reporting it as one
            // would put a phantom event under the user's first row.
            asm.push(abi::load_u64(
                abi::SCRATCH[2],
                abi::LOCAL[0],
                TV_COLS_OFFSET,
            ));
            asm.push(abi::load_u64(
                abi::SCRATCH[3],
                abi::LOCAL[0],
                TV_ROWS_OFFSET,
            ));
            emit_range_check(asm, sym, abi::SCRATCH[0], abi::SCRATCH[2], done, "col");
            emit_range_check(asm, sym, abi::SCRATCH[1], abi::SCRATCH[3], done, "row");
        }
        MouseSurface::Pixels => {
            // No cell divide: the view-space point already IS the surface point,
            // because the canvas view is the surface.
            //
            // But it does need the Y-flip. `MFBCanvasView` does not override
            // `isFlipped` (unlike `TermView`), so `convertPoint:` hands back
            // NSView's bottom-left origin, while `vector::Float2` is documented
            // top-left with Y increasing downward — the origin every `DrawItem`
            // and every hit test uses. Without the flip a click near the top of
            // the window would be reported near the bottom of the scene.
            asm.local_address(abi::LOCAL[0], GRAPHICS_STATE_SYMBOL);
            asm.push(abi::load_u64(
                abi::SCRATCH[2],
                abi::LOCAL[0],
                GRAPHICS_OFFSET_WIDTH,
            ));
            asm.push(abi::load_u64(
                abi::SCRATCH[3],
                abi::LOCAL[0],
                GRAPHICS_OFFSET_HEIGHT,
            ));
            asm.push(abi::load_double(
                abi::FP_SCRATCH[0],
                abi::stack_pointer(),
                OFF_POINT_X,
            ));
            asm.push(abi::load_double(
                abi::FP_SCRATCH[1],
                abi::stack_pointer(),
                OFF_POINT_Y,
            ));
            // y = height - y. The published extent rather than the view's bounds:
            // it is what the program draws against, and `setFrameSize:` keeps the
            // two in step (plan-98-D Phase 3).
            asm.push(abi::signed_convert_to_float_d(
                abi::FP_SCRATCH[2],
                abi::SCRATCH[3],
            ));
            asm.push(abi::float_subtract_d(
                abi::FP_SCRATCH[1],
                abi::FP_SCRATCH[2],
                abi::FP_SCRATCH[1],
            ));
            asm.push(abi::float_floor_to_signed_x(
                abi::SCRATCH[0],
                abi::FP_SCRATCH[0],
            ));
            asm.push(abi::float_floor_to_signed_x(
                abi::SCRATCH[1],
                abi::FP_SCRATCH[1],
            ));
            asm.push(abi::store_u64(abi::SCRATCH[0], abi::stack_pointer(), OFF_X));
            asm.push(abi::store_u64(abi::SCRATCH[1], abi::stack_pointer(), OFF_Y));
            // No range check here (plan-172-J): `emit_publish` publishes a point off
            // the surface as "outside" and then leaves, which is the same exit the
            // check took, one step later.
        }
    }
}

/// Branch to `done` unless `0 <= value < limit`.
///
/// Signed, because a coordinate outside the window is genuinely negative and an
/// unsigned test would read it as enormous — which happens to reject it too, but
/// for the wrong reason and only by luck.
fn emit_range_check(asm: &mut Asm, sym: &str, value: Reg, limit: Reg, done: &str, tag: &str) {
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
        MouseAction::Enter | MouseAction::Exit => {
            unreachable!("an enter or exit IMP publishes and returns; it reports nothing")
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
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_CODE,
    ));

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
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_CODE,
    ));
    for (ns_bit, sgr_bit, tag) in [
        (NS_SHIFT, SGR_SHIFT, "shift"),
        (NS_OPTION, SGR_ALT, "alt"),
        (NS_CONTROL, SGR_CTRL, "ctrl"),
    ] {
        let skip = format!("{sym}_mod_{tag}");
        asm.push(abi::move_immediate(
            abi::SCRATCH[1],
            "Integer",
            &ns_bit.to_string(),
        ));
        asm.push(abi::and_registers(
            abi::SCRATCH[2],
            abi::LOCAL[0],
            abi::SCRATCH[1],
        ));
        asm.push(abi::compare_immediate(abi::SCRATCH[2], "0"));
        asm.push(abi::branch_eq(&skip));
        asm.push(abi::move_immediate(
            abi::SCRATCH[1],
            "Integer",
            &sgr_bit.to_string(),
        ));
        asm.push(abi::or_registers(
            abi::SCRATCH[0],
            abi::SCRATCH[0],
            abi::SCRATCH[1],
        ));
        asm.push(abi::label(&skip));
    }
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_CODE,
    ));

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
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_TERMINATOR,
    ));
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
    let mut imps = vec![
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
    ];
    // plan-172-J: the pixel surface also hears the pointer come and go, to publish
    // it. The cell surface publishes nothing and has neither.
    if !term {
        imps.push(MouseImp {
            symbol: CANVAS_MOUSE_ENTERED_SYMBOL,
            selector: SEL_MOUSE_ENTERED,
            surface,
            action: MouseAction::Enter,
            button: 0,
        });
        imps.push(MouseImp {
            symbol: CANVAS_MOUSE_EXITED_SYMBOL,
            selector: SEL_MOUSE_EXITED,
            surface,
            action: MouseAction::Exit,
            button: 0,
        });
    }
    imps
}

/// Emit every mouse IMP for the views this program can present. The pixel IMPs
/// read the canvas graphics state (`_mfb_rt_canvas_graphics`), a data object that
/// exists only in a program that uses `canvas::` — so a mouse program without it
/// gets only `TermView`'s cell IMPs, or its build names an undefined symbol.
///
/// plan-172-J: and a canvas program gets the pixel IMPs whether or not it uses the
/// mouse members, because they publish the pointer; `uses_mouse` then only decides
/// whether they go on to report (`emit_mouse_imp`'s `reports`).
pub(super) fn emit_mouse_imps(uses_canvas: bool, uses_mouse: bool) -> Vec<CodeFunction> {
    let mut surfaces = Vec::new();
    if uses_mouse {
        surfaces.push(MouseSurface::Cells);
    }
    if uses_canvas {
        surfaces.push(MouseSurface::Pixels);
    }
    surfaces
        .into_iter()
        .flat_map(|surface| {
            mouse_imps(surface)
                .into_iter()
                .map(|imp| emit_mouse_imp(&imp, uses_mouse))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// `class_addMethod` every mouse IMP onto the class in `class_reg`.
pub(super) fn emit_install_mouse_imps(
    asm: &mut Asm,
    class_reg: impl Into<Reg>,
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
///
/// plan-172-J: `entered_and_exited` adds `NSTrackingMouseEnteredAndExited` (0x01), which
/// is what makes AppKit send `mouseEntered:` and `mouseExited:` at all. The canvas view
/// asks for it, to publish the pointer leaving; the term view does not, and its area is
/// what it always was.
pub(super) fn emit_install_tracking_area(
    asm: &mut Asm,
    view_reg: impl Into<Reg>,
    entered_and_exited: bool,
) {
    const NS_TRACKING_ENTERED_AND_EXITED: u64 = 0x01;
    const NS_TRACKING_MOUSE_MOVED: u64 = 0x02;
    const NS_TRACKING_ACTIVE_ALWAYS: u64 = 0x80;
    const NS_TRACKING_IN_VISIBLE_RECT: u64 = 0x200;
    let options = NS_TRACKING_MOUSE_MOVED
        | NS_TRACKING_ACTIVE_ALWAYS
        | NS_TRACKING_IN_VISIBLE_RECT
        | if entered_and_exited {
            NS_TRACKING_ENTERED_AND_EXITED
        } else {
            0
        };

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
        &options.to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch::ops::CodeOp;

    /// bug-993: AppKit calls every IMP through the AAPCS64, and
    /// `-[NSWindow sendEvent:]` releases what it kept in `x19` after the dispatch.
    /// Each IMP saves `LOCAL[0]` in its prologue and reloads it in the `{sym}_done`
    /// epilogue, which every exit (the mode gate, an off-surface point, a zero
    /// wheel delta, and the full report path) reaches before the single `ret`.
    #[test]
    fn every_imp_saves_and_restores_local0() {
        let local0 = abi::LOCAL[0].to_string();
        let offset = OFF_SAVED_LOCAL0.to_string();
        // plan-172-J: the pixel IMPs exist in two shapes -- reporting, and publish-only
        // for a canvas program that uses no mouse member. Both are AAPCS64 callees.
        for (surface, reports) in [
            (MouseSurface::Cells, true),
            (MouseSurface::Pixels, true),
            (MouseSurface::Pixels, false),
        ] {
            for imp in mouse_imps(surface) {
                let func = emit_mouse_imp(&imp, reports);
                let ins = &func.instructions;
                let done = format!("{}_done", imp.symbol);
                let done_at = ins
                    .iter()
                    .position(|i| {
                        i.op == CodeOp::Label && i.get("name").as_deref() == Some(&done[..])
                    })
                    .expect("done label");
                // `mouseExited:` locates nothing and so never writes it; it is saved
                // and restored all the same, by the one prologue and epilogue.
                let first_write = ins
                    .iter()
                    .position(|i| i.get("dst").as_deref() == Some(&local0[..]))
                    .unwrap_or(ins.len());
                let saved_at = ins
                    .iter()
                    .position(|i| {
                        i.op == CodeOp::StrU64
                            && i.get("src").as_deref() == Some(&local0[..])
                            && i.get("offset").as_deref() == Some(&offset[..])
                    })
                    .expect("LOCAL[0] saved");
                assert!(
                    saved_at < first_write,
                    "{}: saved after its first write",
                    imp.symbol
                );
                assert_eq!(ins.iter().filter(|i| i.op == CodeOp::Ret).count(), 1);
                let ret = ins.iter().position(|i| i.op == CodeOp::Ret).expect("ret");
                assert!(
                    ins[done_at..ret].iter().any(|i| i.op == CodeOp::LdrU64
                        && i.get("dst").as_deref() == Some(&local0[..])
                        && i.get("offset").as_deref() == Some(&offset[..])),
                    "{}: LOCAL[0] not restored on the way out",
                    imp.symbol
                );
                // Nothing else callee-saved is written.
                for register in &abi::LOCAL[1..] {
                    let name = register.to_string();
                    assert!(
                        !ins.iter()
                            .any(|i| i.get("dst").as_deref() == Some(&name[..])),
                        "{}: writes {name} without saving it",
                        imp.symbol
                    );
                }
            }
        }
    }

    /// plan-172-J: a canvas program that uses no mouse member still gets the pixel
    /// IMPs, to publish the pointer -- and `_mfb_rt_mouse_mode` is emitted only for a
    /// program that does use one. So the publish-only shape must name neither the mode
    /// word nor the input pipe, or every plain canvas program fails to link.
    #[test]
    fn a_publish_only_imp_names_no_mouse_mode_word_and_writes_no_report() {
        for imp in mouse_imps(MouseSurface::Pixels) {
            let func = emit_mouse_imp(&imp, false);
            let names = |symbol: &str| func.relocations.iter().any(|r| r.to == symbol);
            assert!(
                !names(MOUSE_MODE_SYMBOL),
                "{}: names the mouse-mode word",
                imp.symbol
            );
            assert!(!names("_write"), "{}: writes to the input pipe", imp.symbol);
            assert!(
                names(GRAPHICS_STATE_SYMBOL),
                "{}: does not publish to the graphics state",
                imp.symbol
            );
        }
    }

    /// plan-172-J: on the pixel surface the pointer is published BEFORE the mouse-mode
    /// gate, so it is published whether or not the program enabled the mouse. A gate
    /// first -- the order the cell surface keeps, and the order these IMPs had -- would
    /// make the pointer the graphics thread sees depend on `canvas::enableMouse`.
    ///
    /// Read off the relocations, which are recorded in emission order: the graphics
    /// state is addressed before the mode word is. `mouseEntered:` and `mouseExited:`
    /// publish and return, so they gate on nothing at all.
    #[test]
    fn a_reporting_pixel_imp_publishes_before_it_gates() {
        for imp in mouse_imps(MouseSurface::Pixels) {
            let func = emit_mouse_imp(&imp, true);
            let first = |symbol: &str| func.relocations.iter().position(|r| r.to == symbol);
            let published = first(GRAPHICS_STATE_SYMBOL)
                .unwrap_or_else(|| panic!("{}: never addresses the graphics state", imp.symbol));
            match (imp.action, first(MOUSE_MODE_SYMBOL)) {
                (MouseAction::Enter | MouseAction::Exit, gate) => {
                    assert!(gate.is_none(), "{}: gates, but reports nothing", imp.symbol)
                }
                (_, Some(gate)) => assert!(
                    published < gate,
                    "{}: gates before it publishes",
                    imp.symbol
                ),
                (_, None) => panic!("{}: reports with no mode gate", imp.symbol),
            }
        }
    }
}
