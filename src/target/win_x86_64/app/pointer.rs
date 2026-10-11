//! Windows pointer publication (plan-172-J): where the pointer is, for the graphics
//! thread.
//!
//! A canvas program's window procedure publishes the pointer to the graphics state
//! (`runtime::canvas::emit_publish_pointer`) on every mouse message it is sent while a
//! canvas surface is live, whether or not the program uses a mouse member, and reports
//! nothing to the program: the arms here never consume a message. The SGR arms in
//! `mouse.rs` run after them, unchanged, and gate on the mouse-mode word as they always
//! did.
//!
//! Two functions and one block:
//!
//! * [`emit_pointer_move`] — `_mfb_winapp_pointer_move(x, y, mk)`: the position in
//!   client pixels and the `MK_*` button flags a mouse message carries. It works out
//!   the published mask and whether the point is on the surface, and publishes.
//! * [`emit_pointer_leave`] — `_mfb_winapp_pointer_leave()`: publishes "outside",
//!   with whatever was held still held.
//! * [`emit_pointer_arms`] — the `WndProc` block that calls them.
//!
//! They are functions rather than inline arms so that the headless scripted pointer
//! (`MFB_CANVAS_POINTER_X` / `_Y`, in `_main`) calls the SAME code a `WM_MOUSEMOVE`
//! does — headless has no window to send one to.
//!
//! # Leaving
//!
//! Windows sends no message when the pointer leaves a window unless asked:
//! `TrackMouseEvent(TME_LEAVE)` arms one `WM_MOUSELEAVE`, and has to be re-armed after
//! each. `POINTER_TRACKING_SYM` says whether it is armed, so a stream of
//! `WM_MOUSEMOVE`s arms it once.
//!
//! # Registers
//!
//! Both functions are leaves that touch only Win64's volatile registers, so they have
//! no frame and no save area. The names are the architecture-neutral ones and the x86
//! scratch pool overlaps the argument registers (`SCRATCH[3]` is `r8`, `SCRATCH[4]` is
//! `r9` — see the note at the headless resize in `mod.rs`), so each value below is
//! spelled by the register it actually lives in.

use super::*;
use crate::codegen::runtime::canvas::{
    emit_publish_pointer, emit_skip_unless_autopaint_live, state_base, PointerRegs,
    GRAPHICS_OFFSET_HEIGHT, GRAPHICS_OFFSET_POINTER_BUTTONS, GRAPHICS_OFFSET_WIDTH,
};

pub(super) const POINTER_MOVE_SYMBOL: &str = "_mfb_winapp_pointer_move";
pub(super) const POINTER_LEAVE_SYMBOL: &str = "_mfb_winapp_pointer_leave";
/// 1 while a `WM_MOUSELEAVE` is armed for the window.
pub(super) const POINTER_TRACKING_SYM: &str = "_mfb_winapp_pointer_tracking";
/// The headless scripted pointer's two variable names (UTF-16).
pub(super) const POINTER_X_ENV_SYM: &str = "_mfb_winapp_pointer_x_env";
pub(super) const POINTER_Y_ENV_SYM: &str = "_mfb_winapp_pointer_y_env";

/// `WM_MOUSEFIRST`..`WM_MBUTTONDBLCLK`: every message that carries a client-space
/// point in `lParam` and the `MK_*` flags in `wParam`. The wheel (0x020A) is outside
/// the range on purpose — its point is in SCREEN coordinates.
const WM_MOUSEMOVE: &str = "512"; // 0x0200
const WM_MBUTTONDBLCLK: &str = "521"; // 0x0209
const WM_MOUSELEAVE: &str = "675"; // 0x02A3
/// `TRACKMOUSEEVENT.dwFlags`: post `WM_MOUSELEAVE` when the pointer leaves.
const TME_LEAVE: &str = "2";
/// `sizeof(TRACKMOUSEEVENT)` on Win64: two DWORDs, an HWND, a DWORD, padded to 24.
const TRACKMOUSEEVENT_BYTES: &str = "24";

/// `wParam` button flags and the published bit each one is (bit 0 left, 1 middle, 2
/// right). `wParam` is the state AFTER the event — a button-down message has its own
/// flag set and a button-up has it clear — so the mask needs no correcting.
const MK_TO_MASK: [(u64, u64); 3] = [(0x0001, 1), (0x0010, 2), (0x0002, 4)];

/// Extra `WndProc` frame bytes the arms reserve past their base: one
/// `TRACKMOUSEEVENT`, 8-aligned. A multiple of 16, so the frame's parity is unchanged.
pub(super) const POINTER_FRAME_EXTRA: usize = 32;

/// `_mfb_winapp_pointer_move(x: rcx, y: rdx, mk: r8)`.
///
/// **A point off the surface is published as outside, not dropped.** With the mouse
/// captured, or during a drag, messages keep arriving with coordinates outside the
/// client area and no `WM_MOUSELEAVE`; the bounds test here is the only place the
/// pointer is seen to go.
pub(super) fn emit_pointer_move() -> CodeFunction {
    let from = POINTER_MOVE_SYMBOL;
    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel: Vec<CodeRelocation> = Vec::new();
    let x = abi::mfb_arg(0); // rcx
    let y = abi::mfb_arg(1); // rdx
    let mk = abi::mfb_arg(2); // r8
    let inside = abi::mfb_arg(3); // r9
    let base = abi::c_return(0); // rax
    let buttons = abi::SCRATCH[5]; // r10
    let temp = abi::SCRATCH[6]; // r11

    ins.push(abi::label("entry"));
    ins.push(abi::move_immediate(buttons, "Integer", "0"));
    for (flag, bit) in MK_TO_MASK {
        let skip = format!("{from}_mk_{flag}");
        ins.push(abi::move_immediate(temp, "Integer", &flag.to_string()));
        ins.push(abi::and_registers(temp, mk, temp));
        ins.push(abi::compare_immediate(temp, "0"));
        ins.push(abi::branch_eq(&skip));
        ins.push(abi::move_immediate(temp, "Integer", &bit.to_string()));
        ins.push(abi::or_registers(buttons, buttons, temp));
        ins.push(abi::label(&skip));
    }

    // inside = 0 <= x < width && 0 <= y < height, against the published extent.
    let outside = format!("{from}_outside");
    ins.push(abi::move_immediate(inside, "Integer", "0"));
    state_base(from, base, &mut ins, &mut rel);
    ins.push(abi::compare_immediate(x, "0"));
    ins.push(abi::branch_lt(&outside));
    ins.push(abi::load_u64(temp, base, GRAPHICS_OFFSET_WIDTH));
    ins.push(abi::compare_registers(x, temp));
    ins.push(abi::branch_ge(&outside));
    ins.push(abi::compare_immediate(y, "0"));
    ins.push(abi::branch_lt(&outside));
    ins.push(abi::load_u64(temp, base, GRAPHICS_OFFSET_HEIGHT));
    ins.push(abi::compare_registers(y, temp));
    ins.push(abi::branch_ge(&outside));
    ins.push(abi::move_immediate(inside, "Integer", "1"));
    ins.push(abi::label(&outside));

    emit_publish_pointer(
        from,
        false,
        &PointerRegs {
            x: x.into(),
            y: y.into(),
            buttons: buttons.into(),
            inside: inside.into(),
            base: base.into(),
            seq: temp.into(),
            // Unused: x86-64 publishes with plain stores, which need no address.
            addr: mk.into(),
        },
        &mut ins,
        &mut rel,
    );
    ins.push(abi::return_());
    code_function("winapp.pointer_move", from, ins, rel)
}

/// `_mfb_winapp_pointer_leave()` — the pointer is off the surface. The buttons are the
/// ones last published: a `WM_MOUSELEAVE` says nothing about them.
pub(super) fn emit_pointer_leave() -> CodeFunction {
    let from = POINTER_LEAVE_SYMBOL;
    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel: Vec<CodeRelocation> = Vec::new();
    let x = abi::mfb_arg(0); // rcx
    let y = abi::mfb_arg(1); // rdx
    let inside = abi::mfb_arg(3); // r9
    let base = abi::c_return(0); // rax
    let buttons = abi::SCRATCH[5]; // r10
    let temp = abi::SCRATCH[6]; // r11

    ins.push(abi::label("entry"));
    state_base(from, base, &mut ins, &mut rel);
    ins.push(abi::load_u64(
        buttons,
        base,
        GRAPHICS_OFFSET_POINTER_BUTTONS,
    ));
    ins.push(abi::move_immediate(x, "Integer", "0"));
    ins.push(abi::move_immediate(y, "Integer", "0"));
    ins.push(abi::move_immediate(inside, "Integer", "0"));
    emit_publish_pointer(
        from,
        false,
        &PointerRegs {
            x: x.into(),
            y: y.into(),
            buttons: buttons.into(),
            inside: inside.into(),
            base: base.into(),
            seq: temp.into(),
            addr: abi::mfb_arg(2).into(),
        },
        &mut ins,
        &mut rel,
    );
    ins.push(abi::return_());
    code_function("winapp.pointer_leave", from, ins, rel)
}

/// Emit the pointer block into the `WndProc`, with its `TRACKMOUSEEVENT` at `base`.
///
/// Reached where the dispatch chain runs out. It never consumes a message: every path
/// ends at `next`, which is the SGR mouse arms when the program has them and the
/// default handler otherwise — both reload the message from its slots.
///
/// Gated on a live canvas surface (`CANVAS_HWND_SYM`), like `WM_SIZE`'s publish: the
/// same window is the transcript and the `term::` grid at other times, and the pointer
/// over those is not over a canvas.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_pointer_arms(
    from: &str,
    base: usize,
    msg_slot: usize,
    wparam_slot: usize,
    lparam_slot: usize,
    hwnd_slot: usize,
    next: &str,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) {
    let leave = format!("{from}_pointer_leave");
    // plan-172-K, trigger 6: after a publish, and while the scene holds an `AutoPaint`
    // — only then — the render loop is woken to look. The argument is 1, "the pointer
    // changed": not a redraw that is owed. Here and not in the two functions that
    // publish: they are leaves with no frame, and the signal is a call.
    let signal = |tag: &str, ins: &mut Vec<CodeInstruction>, rel: &mut Vec<CodeRelocation>| {
        let quiet = format!("{from}_pointer_{tag}_no_autopaint");
        emit_skip_unless_autopaint_live(from, abi::SCRATCH[0], &quiet, ins, rel);
        ins.push(abi::move_immediate(abi::mfb_arg(0), "Integer", "1"));
        call_internal(from, super::CANVAS_SIGNAL_REDRAW_SYMBOL, ins, rel);
        ins.push(abi::label(&quiet));
    };
    let live = |ins: &mut Vec<CodeInstruction>, rel: &mut Vec<CodeRelocation>| {
        load_addr(abi::SCRATCH[0], CANVAS_HWND_SYM, from, ins, rel);
        ins.push(abi::load_u64(abi::SCRATCH[0], abi::SCRATCH[0], 0));
        ins.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
        ins.push(abi::branch_eq(next));
    };

    ins.push(abi::load_u64(
        abi::mfb_arg(0),
        abi::stack_pointer(),
        msg_slot,
    ));
    ins.push(abi::compare_immediate(abi::mfb_arg(0), WM_MOUSELEAVE));
    ins.push(abi::branch_eq(&leave));
    ins.push(abi::compare_immediate(abi::mfb_arg(0), WM_MOUSEMOVE));
    ins.push(abi::branch_lt(next));
    ins.push(abi::compare_immediate(abi::mfb_arg(0), WM_MBUTTONDBLCLK));
    ins.push(abi::branch_gt(next));
    live(ins, rel);

    // x = (SHORT)LOWORD(lParam), y = (SHORT)HIWORD(lParam) — signed, because a point
    // left of or above the client area is genuinely negative.
    ins.push(abi::load_u64(
        abi::SCRATCH[2],
        abi::stack_pointer(),
        lparam_slot,
    ));
    ins.push(abi::shift_left_immediate(
        abi::mfb_arg(0),
        abi::SCRATCH[2],
        48,
    ));
    ins.push(abi::arithmetic_shift_right_immediate(
        abi::mfb_arg(0),
        abi::mfb_arg(0),
        48,
    ));
    ins.push(abi::shift_left_immediate(
        abi::mfb_arg(1),
        abi::SCRATCH[2],
        32,
    ));
    ins.push(abi::arithmetic_shift_right_immediate(
        abi::mfb_arg(1),
        abi::mfb_arg(1),
        48,
    ));
    ins.push(abi::load_u64(
        abi::mfb_arg(2),
        abi::stack_pointer(),
        wparam_slot,
    ));
    call_internal(from, POINTER_MOVE_SYMBOL, ins, rel);
    signal("move", ins, rel);

    // Arm one WM_MOUSELEAVE, unless one is armed already.
    load_addr(abi::SCRATCH[0], POINTER_TRACKING_SYM, from, ins, rel);
    ins.push(abi::load_u64(abi::SCRATCH[1], abi::SCRATCH[0], 0));
    ins.push(abi::compare_immediate(abi::SCRATCH[1], "0"));
    ins.push(abi::branch_ne(next));
    ins.push(abi::move_immediate(abi::SCRATCH[1], "Integer", "1"));
    ins.push(abi::store_u64(abi::SCRATCH[1], abi::SCRATCH[0], 0));
    // TRACKMOUSEEVENT { cbSize, dwFlags, hwndTrack, dwHoverTime }
    ins.push(abi::move_immediate(
        abi::SCRATCH[1],
        "Integer",
        TRACKMOUSEEVENT_BYTES,
    ));
    ins.push(abi::store_u32(abi::SCRATCH[1], abi::stack_pointer(), base));
    ins.push(abi::move_immediate(abi::SCRATCH[1], "Integer", TME_LEAVE));
    ins.push(abi::store_u32(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        base + 4,
    ));
    ins.push(abi::load_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        hwnd_slot,
    ));
    ins.push(abi::store_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        base + 8,
    ));
    ins.push(abi::move_immediate(abi::SCRATCH[1], "Integer", "0"));
    ins.push(abi::store_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        base + 16,
    ));
    ins.push(abi::add_immediate(
        abi::mfb_arg(0),
        abi::stack_pointer(),
        base,
    ));
    call_external(from, "TrackMouseEvent", USER32, ins, rel);
    ins.push(abi::branch(next));

    // WM_MOUSELEAVE: outside, and the one armed notification is spent.
    ins.push(abi::label(&leave));
    load_addr(abi::SCRATCH[0], POINTER_TRACKING_SYM, from, ins, rel);
    ins.push(abi::move_immediate(abi::SCRATCH[1], "Integer", "0"));
    ins.push(abi::store_u64(abi::SCRATCH[1], abi::SCRATCH[0], 0));
    live(ins, rel);
    call_internal(from, POINTER_LEAVE_SYMBOL, ins, rel);
    signal("leave", ins, rel);
    ins.push(abi::branch(next));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch::ops::CodeOp;
    use crate::codegen::error::constants::MOUSE_MODE_SYMBOL;
    use crate::codegen::runtime::canvas::GRAPHICS_STATE_SYMBOL;

    /// The pointer is published whether or not the program uses a mouse member, and
    /// `_mfb_rt_mouse_mode` exists only in a program that does — so nothing here may
    /// name it, or a plain canvas program fails to link.
    #[test]
    fn the_pointer_code_names_no_mouse_mode_word() {
        let mut ins = Vec::new();
        let mut rel = Vec::new();
        emit_pointer_arms(
            "f", 0x108, 0x68, 0x70, 0x78, 0x60, "next", &mut ins, &mut rel,
        );
        let mut all: Vec<String> = rel.iter().map(|r| r.to.clone()).collect();
        for func in [emit_pointer_move(), emit_pointer_leave()] {
            assert!(func
                .relocations
                .iter()
                .any(|r| r.to == GRAPHICS_STATE_SYMBOL));
            all.extend(func.relocations.iter().map(|r| r.to.clone()));
        }
        assert!(!all.iter().any(|to| to == MOUSE_MODE_SYMBOL));
        assert!(all.iter().any(|to| to == "TrackMouseEvent"));
    }

    /// plan-172-K, trigger 6: the block signals after each of its two publishes, with
    /// 1 — "the pointer changed" — and only past the check that the scene holds an
    /// `AutoPaint`. A 0 here would turn every pointer move into a frame that is drawn.
    #[test]
    fn the_pointer_block_signals_a_pointer_change_after_each_publish() {
        use crate::codegen::runtime::canvas::{
            signal_redraw_arguments, GRAPHICS_OFFSET_AUTOPAINT_LIVE,
        };
        let mut ins = Vec::new();
        let mut rel = Vec::new();
        emit_pointer_arms(
            "f", 0x108, 0x68, 0x70, 0x78, 0x60, "next", &mut ins, &mut rel,
        );
        assert_eq!(signal_redraw_arguments(&ins), vec!["1", "1"]);
        let live = GRAPHICS_OFFSET_AUTOPAINT_LIVE.to_string();
        let reads = ins
            .iter()
            .filter(|i| i.get("offset").as_deref() == Some(live.as_str()))
            .count();
        assert_eq!(
            reads, 2,
            "each signal is behind its own read of the live word"
        );
    }

    /// x86-64 has no store-release lowering: the publish must be plain stores.
    #[test]
    fn the_pointer_functions_use_no_release_store() {
        for func in [emit_pointer_move(), emit_pointer_leave()] {
            assert!(!func.instructions.iter().any(|i| i.op == CodeOp::StlrU64));
            // Leaves: no call, so no frame and nothing to align.
            assert!(!func.instructions.iter().any(|i| i.op == CodeOp::BranchLink));
        }
    }

    /// Every path through the `WndProc` block ends at `next`: the block publishes and
    /// never consumes a message, so the SGR arms and the default handler still see it.
    #[test]
    fn the_pointer_block_never_returns_on_its_own() {
        let mut ins = Vec::new();
        let mut rel = Vec::new();
        emit_pointer_arms(
            "f", 0x108, 0x68, 0x70, 0x78, 0x60, "next", &mut ins, &mut rel,
        );
        assert!(!ins.iter().any(|i| i.op == CodeOp::Ret));
        let last = ins.last().expect("instructions");
        assert_eq!(
            last.get("label").or(last.get("target")).as_deref(),
            Some("next")
        );
    }
}
