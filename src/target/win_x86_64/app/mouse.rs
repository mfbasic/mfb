//! Windows app-mode mouse input (plan-94-E).
//!
//! Eight `WM_*` arms in the app `WndProc`, each converting the message to
//! surface coordinates and writing the SGR bytes plan-94-B's decoder expects to
//! the worker input pipe the `editproc` subclass already uses for keystrokes.
//! No Windows-specific event queue.
//!
//! # Which proc, and why this one
//!
//! The transcript EDIT child fills the client area, so while it is **visible**
//! it — not the main `WndProc` — receives mouse messages. That made the routing
//! plan-94-E's one genuinely open question. It is settled by reading the
//! show/hide logic:
//!
//! * `term::on` hides the EDIT so the character grid shows through
//!   (`ShowWindow(edit, SW_HIDE)` in its body),
//! * entering `Mode.Canvas` hides it too (`wnd_reconcile_canvas`),
//! * and `Mode.None` hides it along with the window.
//!
//! So the EDIT is visible in exactly one state — the transcript — and mouse
//! there is an explicit non-goal: a transcript is a text log. Every mode that
//! *can* report mouse has the EDIT hidden, which leaves the main `WndProc`
//! receiving the messages. No `editproc` fallback is needed.
//!
//! # The two Win32 asymmetries
//!
//! * **Coordinates are signed.** `GET_X_LPARAM` is a *signed* 16-bit value: a
//!   drag that leaves the client area reports negative coordinates, and reading
//!   them unsigned would turn a point just off the left edge into ~65000.
//! * **The wheel is different from everything else.** `WM_MOUSEWHEEL` is
//!   delivered to the **focused** window in **screen** coordinates, while the
//!   button and move messages go to the window under the cursor in **client**
//!   coordinates. The wheel arm therefore needs a `ScreenToClient` the others do
//!   not. That is a Win32 fact, not a design choice.

use super::*;
use crate::codegen::error::constants::MOUSE_MODE_SYMBOL;
use crate::codegen::io::mouse::sgr::{
    emit_format_report, SgrReport, SgrScratch, SGR_REPORT_BYTES, SGR_SCRATCH_BYTES,
};
use crate::codegen::runtime::canvas::{
    GRAPHICS_OFFSET_HEIGHT, GRAPHICS_OFFSET_WIDTH, GRAPHICS_STATE_SYMBOL,
};

// The messages handled, as decimal strings for `compare_immediate`.
const WM_MOUSEMOVE: &str = "512"; // 0x0200
const WM_LBUTTONDOWN: &str = "513"; // 0x0201
const WM_LBUTTONUP: &str = "514"; // 0x0202
const WM_RBUTTONDOWN: &str = "516"; // 0x0204
const WM_RBUTTONUP: &str = "517"; // 0x0205
const WM_MBUTTONDOWN: &str = "519"; // 0x0207
const WM_MBUTTONUP: &str = "520"; // 0x0208
const WM_MOUSEWHEEL: &str = "522"; // 0x020A

/// `wParam` button flags: any of these set on a `WM_MOUSEMOVE` means a drag.
const MK_BUTTONS: u64 = 0x0001 | 0x0002 | 0x0010; // L | R | M
const MK_SHIFT: u64 = 0x0004;
const MK_CONTROL: u64 = 0x0008;
/// `VK_MENU` — the Alt key. `wParam` carries no Alt flag, unlike Shift and
/// Control, so it has to be asked for separately.
const VK_MENU: &str = "18";
/// The high bit of a `GetKeyState` result: the key is down *now*.
const KEY_DOWN_BIT: u64 = 0x8000;

const SGR_SHIFT: u64 = 4;
const SGR_ALT: u64 = 8;
const SGR_CTRL: u64 = 16;
/// Bit 5 of the SGR button code: this report is motion, not a click.
const SGR_MOTION: u64 = 32;
/// The low-bits value meaning "no button" — what a bare motion report carries.
const SGR_NO_BUTTON: u64 = 3;
const SGR_WHEEL_UP: u64 = 64;
const SGR_WHEEL_DOWN: u64 = 65;

/// What an arm reports.
#[derive(Clone, Copy, PartialEq)]
enum Arm {
    /// A button went down; `button` is its SGR code.
    Press(u64),
    /// A button came up.
    Release(u64),
    /// `WM_MOUSEMOVE`: Drag or Move, from the `wParam` button flags.
    Motion,
    /// `WM_MOUSEWHEEL`: direction from the signed `wParam` high word.
    Wheel,
}

/// The arms, in dispatch order.
fn arms() -> [(&'static str, Arm, &'static str); 8] {
    [
        (WM_LBUTTONDOWN, Arm::Press(0), "lbd"),
        (WM_LBUTTONUP, Arm::Release(0), "lbu"),
        (WM_MBUTTONDOWN, Arm::Press(1), "mbd"),
        (WM_MBUTTONUP, Arm::Release(1), "mbu"),
        (WM_RBUTTONDOWN, Arm::Press(2), "rbd"),
        (WM_RBUTTONUP, Arm::Release(2), "rbu"),
        (WM_MOUSEMOVE, Arm::Motion, "mm"),
        (WM_MOUSEWHEEL, Arm::Wheel, "mw"),
    ]
}

/// Extra frame bytes a mouse-capable `WndProc` reserves past its base frame.
///
/// Appended rather than carved out, so every existing slot offset is unchanged
/// and a `WndProc` without mouse keeps its exact frame — the same reason the
/// read helpers append their clock scratch in plan-94-B.
pub(super) const MOUSE_FRAME_EXTRA: usize = {
    let raw = 6 * 8 + SGR_REPORT_BYTES + SGR_SCRATCH_BYTES;
    (raw + 15) / 16 * 16
};

/// Emit the mouse arms into the `WndProc`, based at `base` (the old frame size).
///
/// Each arm falls through to `not_mouse` when the message is not its own, so the
/// chain ends where the existing dispatch continues.
pub(super) fn emit_mouse_arms(
    from: &str,
    base: usize,
    msg_slot: usize,
    wparam_slot: usize,
    lparam_slot: usize,
    hwnd_slot: usize,
    not_mouse: &str,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) {
    // Slots, past the base frame.
    let off_x = base;
    let off_y = base + 8;
    let off_code = base + 16;
    let off_terminator = base + 24;
    let off_len = base + 32;
    let off_point = base + 40; // a POINT for ScreenToClient (two LONGs)
    let off_report = base + 48;
    let off_scratch = off_report + SGR_REPORT_BYTES;

    let done = format!("{from}_mouse_done");

    for (message, arm, tag) in arms() {
        let next = format!("{from}_mouse_next_{tag}");
        let body = format!("{from}_mouse_body_{tag}");
        ins.push(abi::load_u64(
            abi::mfb_arg(0),
            abi::stack_pointer(),
            msg_slot,
        ));
        ins.push(abi::compare_immediate(abi::mfb_arg(0), message));
        ins.push(abi::branch_eq(&body));
        ins.push(abi::branch(&next));
        ins.push(abi::label(&body));

        // --- Gate ---------------------------------------------------------
        //
        // Zero means the program never asked. First act in every arm, so an
        // un-enabled program pays one load and a compare per motion message —
        // which is the whole cost of handling `WM_MOUSEMOVE` unconditionally.
        load_addr(abi::mfb_arg(0), MOUSE_MODE_SYMBOL, from, ins, rel);
        ins.push(abi::load_u64(abi::LOCAL[0], abi::mfb_arg(0), 0));
        ins.push(abi::compare_immediate(abi::LOCAL[0], "0"));
        ins.push(abi::branch_eq(&done));

        emit_coordinates(
            from,
            arm,
            tag,
            lparam_slot,
            hwnd_slot,
            off_point,
            off_x,
            off_y,
            ins,
            rel,
        );
        emit_code(
            from,
            arm,
            tag,
            wparam_slot,
            off_code,
            off_terminator,
            &done,
            ins,
            rel,
        );
        emit_modifiers(from, tag, wparam_slot, off_code, ins, rel);
        emit_format_and_write(
            from,
            tag,
            off_code,
            off_x,
            off_y,
            off_terminator,
            off_len,
            off_report,
            off_scratch,
            ins,
            rel,
        );
        ins.push(abi::branch(&done));
        ins.push(abi::label(&next));
    }
    ins.push(abi::branch(not_mouse));
    // Every handled mouse message is consumed: return 0 rather than chaining to
    // `DefWindowProcW`, which would let the default handler act on a message the
    // program has already been told about.
    ins.push(abi::label(&done));
    ins.push(abi::move_immediate(abi::c_return(0), "Integer", "0"));
}

/// Extract the client-space point and convert it to surface coordinates.
#[allow(clippy::too_many_arguments)]
fn emit_coordinates(
    from: &str,
    arm: Arm,
    tag: &str,
    lparam_slot: usize,
    hwnd_slot: usize,
    off_point: usize,
    off_x: usize,
    off_y: usize,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) {
    let pixels = format!("{from}_mouse_px_{tag}");
    let converted = format!("{from}_mouse_conv_{tag}");
    let done = format!("{from}_mouse_done");

    // x = (SHORT)LOWORD(lParam), y = (SHORT)HIWORD(lParam) — **signed**, because
    // a drag outside the client area reports negative coordinates and the range
    // check below has to see them as negative.
    ins.push(abi::load_u64(
        abi::LOCAL[1],
        abi::stack_pointer(),
        lparam_slot,
    ));
    ins.push(abi::shift_left_immediate(
        abi::SCRATCH[0],
        abi::LOCAL[1],
        48,
    ));
    ins.push(abi::arithmetic_shift_right_immediate(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        48,
    ));
    ins.push(abi::shift_left_immediate(
        abi::SCRATCH[1],
        abi::LOCAL[1],
        32,
    ));
    ins.push(abi::arithmetic_shift_right_immediate(
        abi::SCRATCH[1],
        abi::SCRATCH[1],
        48,
    ));
    ins.push(abi::store_u64(abi::SCRATCH[0], abi::stack_pointer(), off_x));
    ins.push(abi::store_u64(abi::SCRATCH[1], abi::stack_pointer(), off_y));

    if arm == Arm::Wheel {
        // The wheel alone arrives in SCREEN coordinates, at the FOCUSED window —
        // the other messages come in client coordinates to the window under the
        // cursor. So this one converts and the others do not.
        ins.push(abi::store_u32(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            off_point,
        ));
        ins.push(abi::store_u32(
            abi::SCRATCH[1],
            abi::stack_pointer(),
            off_point + 4,
        ));
        ins.push(abi::load_u64(
            abi::mfb_arg(0),
            abi::stack_pointer(),
            hwnd_slot,
        ));
        ins.push(abi::add_immediate(
            abi::mfb_arg(1),
            abi::stack_pointer(),
            off_point,
        ));
        call_external(from, "ScreenToClient", USER32, ins, rel);
        ins.push(abi::load_u32(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            off_point,
        ));
        ins.push(abi::load_u32(
            abi::SCRATCH[1],
            abi::stack_pointer(),
            off_point + 4,
        ));
        ins.push(abi::sign_extend_word(abi::SCRATCH[0], abi::SCRATCH[0]));
        ins.push(abi::sign_extend_word(abi::SCRATCH[1], abi::SCRATCH[1]));
        ins.push(abi::store_u64(abi::SCRATCH[0], abi::stack_pointer(), off_x));
        ins.push(abi::store_u64(abi::SCRATCH[1], abi::stack_pointer(), off_y));
    }

    // Cells (mode 1): a CONSTANT divide — the Windows app grid is fixed at
    // `TUI_COLS` x `TUI_ROWS` cells of `TUI_CELL_W` x `TUI_CELL_H`, with no
    // reflow, which is the same reason `term::didResize` always reads FALSE here.
    // No cached metrics to look up and no float arithmetic at all.
    ins.push(abi::compare_immediate(abi::LOCAL[0], "1"));
    ins.push(abi::branch_ne(&pixels));
    ins.push(abi::move_immediate(
        abi::SCRATCH[2],
        "Integer",
        &TUI_CELL_W.to_string(),
    ));
    ins.push(abi::move_immediate(
        abi::SCRATCH[3],
        "Integer",
        &TUI_CELL_H.to_string(),
    ));
    // A signed divide, so a negative coordinate stays negative and is rejected
    // below rather than wrapping to a huge cell index.
    ins.push(abi::signed_divide_registers(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        abi::SCRATCH[2],
    ));
    ins.push(abi::signed_divide_registers(
        abi::SCRATCH[1],
        abi::SCRATCH[1],
        abi::SCRATCH[3],
    ));
    ins.push(abi::store_u64(abi::SCRATCH[0], abi::stack_pointer(), off_x));
    ins.push(abi::store_u64(abi::SCRATCH[1], abi::stack_pointer(), off_y));
    ins.push(abi::move_immediate(
        abi::SCRATCH[2],
        "Integer",
        &TUI_COLS.to_string(),
    ));
    ins.push(abi::move_immediate(
        abi::SCRATCH[3],
        "Integer",
        &TUI_ROWS.to_string(),
    ));
    emit_range_check(
        from,
        tag,
        "col",
        abi::SCRATCH[0],
        abi::SCRATCH[2],
        &done,
        ins,
    );
    emit_range_check(
        from,
        tag,
        "row",
        abi::SCRATCH[1],
        abi::SCRATCH[3],
        &done,
        ins,
    );
    ins.push(abi::branch(&converted));

    // Pixels (mode 2): the client point already is the surface point, and Win32's
    // client origin is top-left like `canvas::Point`'s — so no flip, unlike macOS.
    ins.push(abi::label(&pixels));
    load_addr(abi::SCRATCH[4], GRAPHICS_STATE_SYMBOL, from, ins, rel);
    ins.push(abi::load_u64(
        abi::SCRATCH[2],
        abi::SCRATCH[4],
        GRAPHICS_OFFSET_WIDTH,
    ));
    ins.push(abi::load_u64(
        abi::SCRATCH[3],
        abi::SCRATCH[4],
        GRAPHICS_OFFSET_HEIGHT,
    ));
    emit_range_check(
        from,
        tag,
        "px",
        abi::SCRATCH[0],
        abi::SCRATCH[2],
        &done,
        ins,
    );
    emit_range_check(
        from,
        tag,
        "py",
        abi::SCRATCH[1],
        abi::SCRATCH[3],
        &done,
        ins,
    );
    ins.push(abi::label(&converted));
}

/// Branch to `done` unless `0 <= value < limit`, signed.
fn emit_range_check(
    from: &str,
    tag: &str,
    axis: &str,
    value: impl Into<Operand> + Copy,
    limit: impl Into<Operand> + Copy,
    done: &str,
    ins: &mut Vec<CodeInstruction>,
) {
    let ok = format!("{from}_mouse_{tag}_{axis}_ok");
    ins.push(abi::compare_immediate(value, "0"));
    ins.push(abi::branch_lt(done));
    ins.push(abi::compare_registers(value, limit));
    ins.push(abi::branch_lt(&ok));
    ins.push(abi::branch(done));
    ins.push(abi::label(&ok));
}

/// The SGR button code and terminator.
#[allow(clippy::too_many_arguments)]
fn emit_code(
    from: &str,
    arm: Arm,
    tag: &str,
    wparam_slot: usize,
    off_code: usize,
    off_terminator: usize,
    done: &str,
    ins: &mut Vec<CodeInstruction>,
    _rel: &mut Vec<CodeRelocation>,
) {
    match arm {
        Arm::Press(button) | Arm::Release(button) => {
            ins.push(abi::move_immediate(
                abi::SCRATCH[0],
                "Integer",
                &button.to_string(),
            ));
        }
        Arm::Motion => {
            // Drag or Move, from whether any button flag is set in `wParam`. One
            // masked test rather than three named ones.
            let free = format!("{from}_mouse_free_{tag}");
            let set = format!("{from}_mouse_set_{tag}");
            ins.push(abi::load_u64(
                abi::SCRATCH[1],
                abi::stack_pointer(),
                wparam_slot,
            ));
            ins.push(abi::move_immediate(
                abi::SCRATCH[0],
                "Integer",
                &MK_BUTTONS.to_string(),
            ));
            ins.push(abi::and_registers(
                abi::SCRATCH[0],
                abi::SCRATCH[1],
                abi::SCRATCH[0],
            ));
            ins.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
            ins.push(abi::branch_eq(&free));
            ins.push(abi::move_immediate(
                abi::SCRATCH[0],
                "Integer",
                &SGR_MOTION.to_string(),
            ));
            ins.push(abi::branch(&set));
            ins.push(abi::label(&free));
            ins.push(abi::move_immediate(
                abi::SCRATCH[0],
                "Integer",
                &(SGR_MOTION | SGR_NO_BUTTON).to_string(),
            ));
            ins.push(abi::label(&set));
        }
        Arm::Wheel => {
            // The delta is the SIGNED high word of `wParam`.
            let up = format!("{from}_mouse_up_{tag}");
            let set = format!("{from}_mouse_wset_{tag}");
            ins.push(abi::load_u64(
                abi::SCRATCH[1],
                abi::stack_pointer(),
                wparam_slot,
            ));
            ins.push(abi::shift_left_immediate(
                abi::SCRATCH[1],
                abi::SCRATCH[1],
                32,
            ));
            ins.push(abi::arithmetic_shift_right_immediate(
                abi::SCRATCH[1],
                abi::SCRATCH[1],
                48,
            ));
            ins.push(abi::compare_immediate(abi::SCRATCH[1], "0"));
            // A zero delta carries no direction; dropped rather than guessed at.
            ins.push(abi::branch_eq(done));
            // Win32's delta is positive when the wheel turns AWAY from the user,
            // which is the terminal's ScrollUp — so unlike GTK this needs no
            // inversion.
            ins.push(abi::branch_gt(&up));
            ins.push(abi::move_immediate(
                abi::SCRATCH[0],
                "Integer",
                &SGR_WHEEL_DOWN.to_string(),
            ));
            ins.push(abi::branch(&set));
            ins.push(abi::label(&up));
            ins.push(abi::move_immediate(
                abi::SCRATCH[0],
                "Integer",
                &SGR_WHEEL_UP.to_string(),
            ));
            ins.push(abi::label(&set));
        }
    }
    ins.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_code,
    ));

    let terminator = if matches!(arm, Arm::Release(_)) {
        b'm'
    } else {
        b'M'
    };
    ins.push(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        &terminator.to_string(),
    ));
    ins.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_terminator,
    ));
}

/// OR the SGR modifier bits into the code.
///
/// Shift and Control come from `wParam`; **Alt does not** — `wParam` has no
/// `MK_ALT`, so it takes a `GetKeyState(VK_MENU)` whose high bit says the key is
/// down now.
fn emit_modifiers(
    from: &str,
    tag: &str,
    wparam_slot: usize,
    off_code: usize,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) {
    ins.push(abi::load_u64(abi::LOCAL[1], abi::stack_pointer(), off_code));
    for (mk_bit, sgr_bit, name) in [(MK_SHIFT, SGR_SHIFT, "sh"), (MK_CONTROL, SGR_CTRL, "ct")] {
        let skip = format!("{from}_mouse_mod_{tag}_{name}");
        ins.push(abi::load_u64(
            abi::SCRATCH[1],
            abi::stack_pointer(),
            wparam_slot,
        ));
        ins.push(abi::move_immediate(
            abi::SCRATCH[0],
            "Integer",
            &mk_bit.to_string(),
        ));
        ins.push(abi::and_registers(
            abi::SCRATCH[0],
            abi::SCRATCH[1],
            abi::SCRATCH[0],
        ));
        ins.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
        ins.push(abi::branch_eq(&skip));
        ins.push(abi::move_immediate(
            abi::SCRATCH[0],
            "Integer",
            &sgr_bit.to_string(),
        ));
        ins.push(abi::or_registers(
            abi::LOCAL[1],
            abi::LOCAL[1],
            abi::SCRATCH[0],
        ));
        ins.push(abi::label(&skip));
    }
    // Alt, the one `wParam` does not carry.
    let skip_alt = format!("{from}_mouse_mod_{tag}_alt");
    ins.push(abi::move_immediate(abi::mfb_arg(0), "Integer", VK_MENU));
    call_external(from, "GetKeyState", USER32, ins, rel);
    ins.push(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        &KEY_DOWN_BIT.to_string(),
    ));
    ins.push(abi::and_registers(
        abi::SCRATCH[0],
        abi::c_return(0),
        abi::SCRATCH[0],
    ));
    ins.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
    ins.push(abi::branch_eq(&skip_alt));
    ins.push(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        &SGR_ALT.to_string(),
    ));
    ins.push(abi::or_registers(
        abi::LOCAL[1],
        abi::LOCAL[1],
        abi::SCRATCH[0],
    ));
    ins.push(abi::label(&skip_alt));
    ins.push(abi::store_u64(
        abi::LOCAL[1],
        abi::stack_pointer(),
        off_code,
    ));
}

/// Format the report and write it to the worker input pipe.
#[allow(clippy::too_many_arguments)]
fn emit_format_and_write(
    from: &str,
    tag: &str,
    off_code: usize,
    off_x: usize,
    off_y: usize,
    off_terminator: usize,
    off_len: usize,
    off_report: usize,
    off_scratch: usize,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) {
    let scratch = SgrScratch {
        value: abi::SCRATCH[0],
        ten: abi::SCRATCH[1],
        quotient: abi::SCRATCH[2],
        digit: abi::SCRATCH[3],
        count: abi::SCRATCH[4],
        addr: abi::SCRATCH[5],
        scratch_base: abi::SCRATCH[6],
    };
    ins.push(abi::add_immediate(
        abi::SCRATCH[7],
        abi::stack_pointer(),
        off_report,
    ));
    ins.push(abi::load_u64(
        abi::SCRATCH[8],
        abi::stack_pointer(),
        off_code,
    ));
    ins.push(abi::load_u64(abi::SCRATCH[9], abi::stack_pointer(), off_x));
    ins.push(abi::load_u64(abi::SCRATCH[10], abi::stack_pointer(), off_y));
    ins.push(abi::load_u64(
        abi::SCRATCH[11],
        abi::stack_pointer(),
        off_terminator,
    ));
    // One-based on the wire, in both units.
    ins.push(abi::add_immediate(abi::SCRATCH[9], abi::SCRATCH[9], 1));
    ins.push(abi::add_immediate(abi::SCRATCH[10], abi::SCRATCH[10], 1));
    emit_format_report(
        &SgrReport {
            button_code: abi::SCRATCH[8],
            x: abi::SCRATCH[9],
            y: abi::SCRATCH[10],
            terminator: abi::SCRATCH[11],
        },
        abi::SCRATCH[7],
        abi::SCRATCH[12],
        off_scratch,
        &format!("{from}_{tag}"),
        &scratch,
        ins,
    );
    ins.push(abi::store_u64(
        abi::SCRATCH[12],
        abi::stack_pointer(),
        off_len,
    ));

    // WriteFile(hWrite, report, len, NULL, NULL) — the same handle `editproc`
    // writes each WM_CHAR to, whose read end is dup2'd onto fd 0. Reusing it is
    // the whole injection design: the backend needs no channel of its own.
    load_addr(abi::mfb_arg(0), STDIN_WRITE_SYM, from, ins, rel);
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), 0));
    ins.push(abi::add_immediate(
        abi::mfb_arg(1),
        abi::stack_pointer(),
        off_report,
    ));
    ins.push(abi::load_u64(
        abi::mfb_arg(2),
        abi::stack_pointer(),
        off_len,
    ));
    ins.push(abi::move_immediate(abi::mfb_arg(3), "Integer", "0"));
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x20));
    call_external(from, "WriteFile", KERNEL32, ins, rel);
}
