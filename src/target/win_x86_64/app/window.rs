//! Win32 window title + fullscreen for the `app` window members
//! (`setTitle`/`getTitle`/`setFullscreen`/`getFullscreen`).
//!
//! The members keep their state in the process-global window data
//! (`builtins::app::gen_window`); this module makes the main window follow it.
//! Everything here is emitted only for a program that uses a window member
//! (`AppEntrySpec::uses_window`).
//!
//! * **Worker → UI thread.** `setTitle`/`setFullscreen` end in
//!   [`emit_window_sync_seam`]: `SendMessageW(main, WM_APP_SYNC_WINDOW, 0, 0)`,
//!   synchronous like the mode reconcile, and skipped headless (no window, so
//!   `MAIN_HWND_SYM` is 0 and there is no message pump).
//! * **The sync** ([`WINDOW_SYNC_SYMBOL`]) runs on the UI thread, from the new
//!   `WndProc` arm and from the reconcile after it shows the window.
//!   - Title: the title data is UTF-8; it is widened with `MultiByteToWideChar`
//!     into a process-heap buffer sized by a first counting call, handed to
//!     `SetWindowTextW` (which copies it) and freed. The title lock is held across
//!     the read and the conversion only — nothing that waits on another thread.
//!   - Fullscreen: Win32 has no fullscreen window state, so this is the borderless
//!     kind. Entering saves the window style and `WINDOWPLACEMENT`, strips
//!     `WS_OVERLAPPEDWINDOW` and covers the window's monitor with `SetWindowPos`;
//!     leaving restores the saved style and placement. [`FS_APPLIED_SYM`] records
//!     which of the two the window is in, so a sync that finds it already matching
//!     the fullscreen word does nothing (in particular, never saves a fullscreen
//!     placement over the windowed one). Only a visible window is changed — a
//!     hidden (`Mode.None`) one is synced again when the reconcile shows it.
//!
//! Every helper here names only the Win64 argument/return tokens and stack slots:
//! the scratch pool and `ARG[4..]` realize to callee-saved registers on Win64
//! (see `emit_main`).

use super::*;
use crate::codegen::builtins::app::gen_window::{
    APP_DEFAULT_TITLE_SYMBOL, APP_FULLSCREEN_SYMBOL, APP_TITLE_LOCK_SYMBOL, APP_TITLE_SYMBOL,
};

/// `WM_APP + 3`: the worker's "sync the window" request.
pub(super) const WM_APP_SYNC_WINDOW: &str = "32771"; // 0x8003
/// UI-thread sync: `_mfb_winapp_window_sync(hwnd)`.
pub(super) const WINDOW_SYNC_SYMBOL: &str = "_mfb_winapp_window_sync";
/// `1` while the window is in borderless fullscreen, `0` while windowed.
const FS_APPLIED_SYM: &str = "_mfb_winapp_fs_applied";
/// The window style saved on entering fullscreen.
const FS_SAVED_STYLE_SYM: &str = "_mfb_winapp_fs_saved_style";
/// The `WINDOWPLACEMENT` (44 bytes) saved on entering fullscreen.
const FS_PLACEMENT_SYM: &str = "_mfb_winapp_fs_placement";
const WINDOWPLACEMENT_SIZE: usize = 44;
const MONITORINFO_SIZE: &str = "40";
const CP_UTF8: &str = "65001";
/// `GWL_STYLE` is `-16`; built as `0 - 16` like `GWLP_WNDPROC`.
const GWL_STYLE_NEG: usize = 16;
const MONITOR_DEFAULTTONEAREST: &str = "2";
/// `WS_OVERLAPPEDWINDOW`: caption, sysmenu, thick frame, min/max boxes.
const WS_OVERLAPPEDWINDOW: &str = "13565952"; // 0x00CF0000
/// `SWP_NOOWNERZORDER | SWP_FRAMECHANGED`.
const SWP_ENTER: &str = "544"; // 0x0220
/// `SWP_NOSIZE | SWP_NOMOVE | SWP_NOZORDER | SWP_NOOWNERZORDER | SWP_FRAMECHANGED`.
const SWP_LEAVE: &str = "551"; // 0x0227

/// The fullscreen bookkeeping globals.
pub(crate) fn window_data_objects() -> Vec<CodeDataObject> {
    vec![
        writable_qword(FS_APPLIED_SYM),
        writable_qword(FS_SAVED_STYLE_SYM),
        CodeDataObject {
            symbol: FS_PLACEMENT_SYM.to_string(),
            kind: "raw".to_string(),
            layout: "WINDOWPLACEMENT (writable, saved on entering fullscreen)".to_string(),
            align: 8,
            size: 48,
            value: "00".repeat(48),
        },
    ]
}

/// The worker-side seam `setTitle`/`setFullscreen` append (a vreg stream: only
/// argument-role tokens, labels prefixed by the caller's symbol).
pub(crate) fn emit_window_sync_seam(
    from: &str,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) {
    let skip = format!("{from}_window_sync_skip");
    load_addr(abi::mfb_arg(0), MAIN_HWND_SYM, from, ins, rel);
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), 0));
    ins.push(abi::compare_immediate(abi::mfb_arg(0), "0"));
    ins.push(abi::branch_eq(&skip)); // headless: no window, no pump
    ins.push(abi::move_immediate(
        abi::mfb_arg(1),
        "Integer",
        WM_APP_SYNC_WINDOW,
    ));
    ins.push(abi::move_immediate(abi::mfb_arg(2), "Integer", "0"));
    ins.push(abi::move_immediate(abi::mfb_arg(3), "Integer", "0"));
    call_external(from, "SendMessageW", USER32, ins, rel);
    ins.push(abi::label(&skip));
}

/// `WndProc` arm for [`WM_APP_SYNC_WINDOW`], placed ahead of the reconcile test.
/// `ARG[1]` still holds `msg`; the hwnd is reloaded from its saved slot.
pub(super) fn emit_wndproc_arm(
    from: &str,
    frame: usize,
    hwnd_slot: usize,
    next: &str,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) {
    ins.push(abi::compare_immediate(abi::mfb_arg(1), WM_APP_SYNC_WINDOW));
    ins.push(abi::branch_ne(next));
    ins.push(abi::load_u64(
        abi::mfb_arg(0),
        abi::stack_pointer(),
        hwnd_slot,
    ));
    call_internal(from, WINDOW_SYNC_SYMBOL, ins, rel);
    ins.push(abi::move_immediate(abi::c_return(0), "Integer", "0"));
    ins.push(abi::add_stack(frame));
    ins.push(abi::return_());
    ins.push(abi::label(next));
}

/// Call the sync for the window whose hwnd is in `hwnd_slot` (the reconcile's show
/// arms, already on the UI thread).
pub(super) fn emit_sync_call(
    from: &str,
    hwnd_slot: usize,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) {
    ins.push(abi::load_u64(
        abi::mfb_arg(0),
        abi::stack_pointer(),
        hwnd_slot,
    ));
    call_internal(from, WINDOW_SYNC_SYMBOL, ins, rel);
}

/// `_mfb_winapp_window_sync(hwnd)` (UI thread).
pub(super) fn emit_window_sync() -> CodeFunction {
    // Frame: shadow[0x00..0x20], outgoing stack args [0x20..0x38] (SetWindowPos's
    // 5th-7th), hwnd@0x38, title-bytes@0x40, wide count@0x48, wide buffer@0x50,
    // MONITORINFO@0x58..0x80. FRAME ≡ 8 (mod 16): entered at sp%16==8 (post-call),
    // so 0x88 realigns before any call.
    const FRAME: usize = 0x88;
    const HWND: usize = 0x38;
    const SRC: usize = 0x40;
    const COUNT: usize = 0x48;
    const BUF: usize = 0x50;
    const MI: usize = 0x58;
    // MONITORINFO.rcMonitor: left/top/right/bottom LONGs after the cbSize DWORD.
    const MI_LEFT: usize = MI + 4;
    const MI_TOP: usize = MI + 8;
    const MI_RIGHT: usize = MI + 12;
    const MI_BOTTOM: usize = MI + 16;
    let from = WINDOW_SYNC_SYMBOL;
    let have_title = format!("{from}_have_title");
    let unlock = format!("{from}_unlock");
    let fullscreen = format!("{from}_fullscreen");
    let leave = format!("{from}_leave");
    let done = format!("{from}_done");
    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel: Vec<CodeRelocation> = Vec::new();
    ins.push(abi::label("entry"));
    ins.push(abi::subtract_stack(FRAME));
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), HWND));
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), BUF));

    // --- title ---
    load_addr(
        abi::mfb_arg(0),
        APP_TITLE_LOCK_SYMBOL,
        from,
        &mut ins,
        &mut rel,
    );
    call_external(
        from,
        "AcquireSRWLockExclusive",
        KERNEL32,
        &mut ins,
        &mut rel,
    );
    load_addr(abi::mfb_arg(0), APP_TITLE_SYMBOL, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), 0));
    ins.push(abi::compare_immediate(abi::mfb_arg(0), "0"));
    ins.push(abi::branch_ne(&have_title));
    load_addr(
        abi::mfb_arg(0),
        APP_DEFAULT_TITLE_SYMBOL,
        from,
        &mut ins,
        &mut rel,
    );
    ins.push(abi::label(&have_title));
    ins.push(abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 8)); // the bytes
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), SRC));
    // count = MultiByteToWideChar(CP_UTF8, 0, src, -1, NULL, 0) — includes the NUL.
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x20));
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x28));
    emit_widen_call(from, SRC, &mut ins, &mut rel);
    // An int: keep only its low 32 bits (0 = the bytes are not UTF-8).
    ins.push(abi::store_u32(
        abi::c_return(0),
        abi::stack_pointer(),
        COUNT,
    ));
    ins.push(abi::load_u32(abi::mfb_arg(0), abi::stack_pointer(), COUNT));
    ins.push(abi::compare_immediate(abi::mfb_arg(0), "0"));
    ins.push(abi::branch_eq(&unlock));
    // buf = HeapAlloc(GetProcessHeap(), 0, count * 2)
    call_external(from, "GetProcessHeap", KERNEL32, &mut ins, &mut rel);
    ins.push(abi::move_register(abi::mfb_arg(0), abi::c_return(0)));
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "0"));
    ins.push(abi::load_u32(abi::mfb_arg(2), abi::stack_pointer(), COUNT));
    ins.push(abi::add_registers(
        abi::mfb_arg(2),
        abi::mfb_arg(2),
        abi::mfb_arg(2),
    ));
    call_external(from, "HeapAlloc", KERNEL32, &mut ins, &mut rel);
    ins.push(abi::store_u64(abi::c_return(0), abi::stack_pointer(), BUF));
    ins.push(abi::compare_immediate(abi::c_return(0), "0"));
    ins.push(abi::branch_eq(&unlock));
    // MultiByteToWideChar(CP_UTF8, 0, src, -1, buf, count)
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), BUF));
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), 0x20));
    ins.push(abi::load_u32(abi::mfb_arg(0), abi::stack_pointer(), COUNT));
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), 0x28));
    emit_widen_call(from, SRC, &mut ins, &mut rel);
    ins.push(abi::label(&unlock));
    load_addr(
        abi::mfb_arg(0),
        APP_TITLE_LOCK_SYMBOL,
        from,
        &mut ins,
        &mut rel,
    );
    call_external(
        from,
        "ReleaseSRWLockExclusive",
        KERNEL32,
        &mut ins,
        &mut rel,
    );
    // SetWindowTextW(hwnd, buf); HeapFree(GetProcessHeap(), 0, buf)
    ins.push(abi::load_u64(abi::mfb_arg(1), abi::stack_pointer(), BUF));
    ins.push(abi::compare_immediate(abi::mfb_arg(1), "0"));
    ins.push(abi::branch_eq(&fullscreen));
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), HWND));
    call_external(from, "SetWindowTextW", USER32, &mut ins, &mut rel);
    call_external(from, "GetProcessHeap", KERNEL32, &mut ins, &mut rel);
    ins.push(abi::move_register(abi::mfb_arg(0), abi::c_return(0)));
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "0"));
    ins.push(abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), BUF));
    call_external(from, "HeapFree", KERNEL32, &mut ins, &mut rel);

    // --- fullscreen ---
    ins.push(abi::label(&fullscreen));
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), HWND));
    call_external(from, "IsWindowVisible", USER32, &mut ins, &mut rel);
    ins.push(abi::store_u32(
        abi::c_return(0),
        abi::stack_pointer(),
        COUNT,
    ));
    ins.push(abi::load_u32(abi::mfb_arg(0), abi::stack_pointer(), COUNT));
    ins.push(abi::compare_immediate(abi::mfb_arg(0), "0"));
    ins.push(abi::branch_eq(&done));
    load_addr(
        abi::mfb_arg(0),
        APP_FULLSCREEN_SYMBOL,
        from,
        &mut ins,
        &mut rel,
    );
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), 0));
    load_addr(abi::mfb_arg(1), FS_APPLIED_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(1), 0));
    ins.push(abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(1)));
    ins.push(abi::branch_eq(&done)); // already matching
    ins.push(abi::compare_immediate(abi::mfb_arg(0), "0"));
    ins.push(abi::branch_eq(&leave));

    // Enter: save style + placement, strip the frame, cover the monitor.
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), HWND));
    emit_gwl_style(&mut ins);
    call_external(from, "GetWindowLongPtrW", USER32, &mut ins, &mut rel);
    load_addr(
        abi::mfb_arg(1),
        FS_SAVED_STYLE_SYM,
        from,
        &mut ins,
        &mut rel,
    );
    ins.push(abi::store_u64(abi::c_return(0), abi::mfb_arg(1), 0));
    load_addr(abi::mfb_arg(1), FS_PLACEMENT_SYM, from, &mut ins, &mut rel);
    ins.push(abi::move_immediate(
        abi::mfb_arg(0),
        "Integer",
        &WINDOWPLACEMENT_SIZE.to_string(),
    ));
    ins.push(abi::store_u32(abi::mfb_arg(0), abi::mfb_arg(1), 0)); // .length
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), HWND));
    call_external(from, "GetWindowPlacement", USER32, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), HWND));
    ins.push(abi::move_immediate(
        abi::mfb_arg(1),
        "Integer",
        MONITOR_DEFAULTTONEAREST,
    ));
    call_external(from, "MonitorFromWindow", USER32, &mut ins, &mut rel);
    ins.push(abi::move_register(abi::mfb_arg(0), abi::c_return(0)));
    ins.push(abi::move_immediate(
        abi::mfb_arg(1),
        "Integer",
        MONITORINFO_SIZE,
    ));
    ins.push(abi::store_u32(abi::mfb_arg(1), abi::stack_pointer(), MI)); // cbSize
    ins.push(abi::add_immediate(
        abi::mfb_arg(1),
        abi::stack_pointer(),
        MI,
    ));
    call_external(from, "GetMonitorInfoW", USER32, &mut ins, &mut rel);
    // SetWindowLongPtrW(hwnd, GWL_STYLE, saved & ~WS_OVERLAPPEDWINDOW)
    load_addr(
        abi::mfb_arg(2),
        FS_SAVED_STYLE_SYM,
        from,
        &mut ins,
        &mut rel,
    );
    ins.push(abi::load_u64(abi::mfb_arg(2), abi::mfb_arg(2), 0));
    ins.push(abi::move_immediate(
        abi::mfb_arg(3),
        "Integer",
        WS_OVERLAPPEDWINDOW,
    ));
    ins.push(abi::bitwise_not(abi::mfb_arg(3), abi::mfb_arg(3)));
    ins.push(abi::and_registers(
        abi::mfb_arg(2),
        abi::mfb_arg(2),
        abi::mfb_arg(3),
    ));
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), HWND));
    emit_gwl_style(&mut ins);
    call_external(from, "SetWindowLongPtrW", USER32, &mut ins, &mut rel);
    // SetWindowPos(hwnd, HWND_TOP, left, top, right - left, bottom - top, SWP_ENTER)
    ins.push(abi::load_u32(
        abi::mfb_arg(0),
        abi::stack_pointer(),
        MI_LEFT,
    ));
    ins.push(abi::sign_extend_word(abi::mfb_arg(0), abi::mfb_arg(0)));
    ins.push(abi::load_u32(
        abi::mfb_arg(1),
        abi::stack_pointer(),
        MI_RIGHT,
    ));
    ins.push(abi::sign_extend_word(abi::mfb_arg(1), abi::mfb_arg(1)));
    ins.push(abi::subtract_registers(
        abi::mfb_arg(1),
        abi::mfb_arg(1),
        abi::mfb_arg(0),
    ));
    ins.push(abi::store_u64(abi::mfb_arg(1), abi::stack_pointer(), 0x20)); // cx
    ins.push(abi::load_u32(abi::mfb_arg(0), abi::stack_pointer(), MI_TOP));
    ins.push(abi::sign_extend_word(abi::mfb_arg(0), abi::mfb_arg(0)));
    ins.push(abi::load_u32(
        abi::mfb_arg(1),
        abi::stack_pointer(),
        MI_BOTTOM,
    ));
    ins.push(abi::sign_extend_word(abi::mfb_arg(1), abi::mfb_arg(1)));
    ins.push(abi::subtract_registers(
        abi::mfb_arg(1),
        abi::mfb_arg(1),
        abi::mfb_arg(0),
    ));
    ins.push(abi::store_u64(abi::mfb_arg(1), abi::stack_pointer(), 0x28)); // cy
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", SWP_ENTER));
    ins.push(abi::store_u64(abi::mfb_arg(1), abi::stack_pointer(), 0x30)); // flags
    ins.push(abi::load_u32(
        abi::mfb_arg(2),
        abi::stack_pointer(),
        MI_LEFT,
    ));
    ins.push(abi::sign_extend_word(abi::mfb_arg(2), abi::mfb_arg(2)));
    ins.push(abi::load_u32(abi::mfb_arg(3), abi::stack_pointer(), MI_TOP));
    ins.push(abi::sign_extend_word(abi::mfb_arg(3), abi::mfb_arg(3)));
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "0")); // HWND_TOP
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), HWND));
    call_external(from, "SetWindowPos", USER32, &mut ins, &mut rel);
    load_addr(abi::mfb_arg(0), FS_APPLIED_SYM, from, &mut ins, &mut rel);
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "1"));
    ins.push(abi::store_u64(abi::mfb_arg(1), abi::mfb_arg(0), 0));
    ins.push(abi::branch(&done));

    // Leave: restore the saved style and placement.
    ins.push(abi::label(&leave));
    load_addr(
        abi::mfb_arg(2),
        FS_SAVED_STYLE_SYM,
        from,
        &mut ins,
        &mut rel,
    );
    ins.push(abi::load_u64(abi::mfb_arg(2), abi::mfb_arg(2), 0));
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), HWND));
    emit_gwl_style(&mut ins);
    call_external(from, "SetWindowLongPtrW", USER32, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), HWND));
    load_addr(abi::mfb_arg(1), FS_PLACEMENT_SYM, from, &mut ins, &mut rel);
    call_external(from, "SetWindowPlacement", USER32, &mut ins, &mut rel);
    // SetWindowPos(hwnd, 0, 0, 0, 0, 0, SWP_LEAVE) — apply the restored frame.
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x20));
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x28));
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", SWP_LEAVE));
    ins.push(abi::store_u64(abi::mfb_arg(1), abi::stack_pointer(), 0x30));
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "0"));
    ins.push(abi::move_immediate(abi::mfb_arg(2), "Integer", "0"));
    ins.push(abi::move_immediate(abi::mfb_arg(3), "Integer", "0"));
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), HWND));
    call_external(from, "SetWindowPos", USER32, &mut ins, &mut rel);
    load_addr(abi::mfb_arg(0), FS_APPLIED_SYM, from, &mut ins, &mut rel);
    ins.push(abi::store_u64(abi::ZERO, abi::mfb_arg(0), 0));

    ins.push(abi::label(&done));
    ins.push(abi::add_stack(FRAME));
    ins.push(abi::return_());
    code_function("winapp.window_sync", WINDOW_SYNC_SYMBOL, ins, rel)
}

/// `MultiByteToWideChar(CP_UTF8, 0, [sp+src_slot], -1, <stack arg 5>, <stack arg 6>)`
/// — the caller stages the two stack arguments.
fn emit_widen_call(
    from: &str,
    src_slot: usize,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) {
    ins.push(abi::move_immediate(abi::mfb_arg(0), "Integer", CP_UTF8));
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "0"));
    ins.push(abi::load_u64(
        abi::mfb_arg(2),
        abi::stack_pointer(),
        src_slot,
    ));
    ins.push(abi::move_immediate(abi::mfb_arg(3), "Integer", "0"));
    ins.push(abi::subtract_immediate(abi::mfb_arg(3), abi::mfb_arg(3), 1)); // -1
    call_external(from, "MultiByteToWideChar", KERNEL32, ins, rel);
}

/// `ARG[1] = GWL_STYLE` (`-16`).
fn emit_gwl_style(ins: &mut Vec<CodeInstruction>) {
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "0"));
    ins.push(abi::subtract_immediate(
        abi::mfb_arg(1),
        abi::mfb_arg(1),
        GWL_STYLE_NEG,
    ));
}
