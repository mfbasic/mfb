//! plan-66-J Win32 app-mode floor.
//!
//! App mode on Windows is a MODE, not a target (plan-66-I): a `-app` build links
//! GUI-subsystem (Subsystem=2) and this module supplies the toolkit bootstrap.
//! The structure mirrors the macOS backend (`macos_aarch64/app/`) and the
//! box-proven message-loop premise in `src/os/windows/link/spike.rs`:
//!
//! - `_main` (the PE entry) creates a `RegisterClassExW`/`CreateWindowExW` window,
//!   spawns a worker thread, and runs a `GetMessageW`/`DispatchMessageW` loop that
//!   owns the main thread (the AppKit `[NSApp run]` / GTK `g_application_run`
//!   analog). An `MFB_WINAPP_HEADLESS` env var skips the window + loop for CI/box
//!   runs that cannot open a GUI (mirrors macOS's `MFB_MACAPP_HEADLESS`).
//! - the worker runs the standard program body under `MACAPP_PROGRAM_SYMBOL`
//!   (emitted separately by the shared entry with `entry_called_as_function:true`),
//!   which sets up the arena on the worker thread and runs MFBASIC.
//! - `WndProc` handles `WM_DESTROY` (→ `PostQuitMessage`) and defers the rest to
//!   `DefWindowProcW`.
//!
//! This is J-2 (the bootstrap floor): console output goes to the inherited
//! standard handle (a GUI-subsystem `.exe` launched from a console still inherits
//! its stdout), which the box run over ssh observes. J-3 adds the GDI transcript
//! window; J-4 the input pipe; J-5 the `term::` TUI grid + mode reconcile.

use std::collections::HashMap;

use crate::arch::aarch64::abi;
use crate::target::shared::code::{
    CodeDataObject, CodeFrame, CodeFunction, CodeInstruction, CodeRelocation, RelocIntent,
    AppEntrySpec, AppHookBody, ARENA_ALLOC_SYMBOL, MACAPP_PROGRAM_SYMBOL, RESULT_OK_TAG,
    RESULT_TAG_REGISTER, RESULT_VALUE_REGISTER, ARENA_STATE_REGISTER, TERM_STATE_ACTIVE_OFFSET,
    TERM_STATE_FG_OFFSET, TERM_STATE_BG_OFFSET, TERM_STATE_BOLD_OFFSET, TERM_STATE_UNDERLINE_OFFSET,
    TERM_STATE_CURSOR_VISIBLE_OFFSET, RESULT_ERR_TAG, RESULT_ERROR_MESSAGE_REGISTER,
    ERR_UNSUPPORTED_CODE, ERR_UNSUPPORTED_SYMBOL,
};

const KERNEL32: &str = "kernel32.dll";
const USER32: &str = "user32.dll";
const GDI32: &str = "gdi32.dll";

const MAIN_SYMBOL: &str = "_main";
const WORKER_SYMBOL: &str = "_mfb_winapp_worker";
const WNDPROC_SYMBOL: &str = "_mfb_winapp_wndproc";
const EDITPROC_SYMBOL: &str = "_mfb_winapp_editproc";

// ---- plan-66-J-5 term:: TUI grid (GDI cell grid painted on the main window) ----
// A fixed 80x25 monospace grid rendered into an off-screen memory DC; term:: ops
// draw into the memDC and `term::sync` InvalidateRects the main window, whose
// WndProc BitBlts the memDC to the client area when TUI mode is active. `term::on`
// hides the transcript EDIT so the grid shows through; `term::off` restores it.
const TUI_COLS: usize = 80;
const TUI_ROWS: usize = 25;
const TUI_CELL_W: usize = 8; // px per cell (matches the Consolas metrics we request)
const TUI_CELL_H: usize = 16;
/// Writable u64 globals for the TUI surface, all 0 until `term::on` builds them.
const TUI_MEMDC_SYM: &str = "_mfb_winapp_tui_memdc"; // off-screen HDC
const TUI_FONT_SYM: &str = "_mfb_winapp_tui_font"; // cached monospace HFONT
const TUI_ROW_SYM: &str = "_mfb_winapp_tui_row"; // cursor row (0-based)
const TUI_COL_SYM: &str = "_mfb_winapp_tui_col"; // cursor col (0-based)
// GDI / window message constants.
const WM_PAINT: &str = "15"; // 0x000F
const SW_HIDE: &str = "0";
const SW_SHOW: &str = "5";
const SRCCOPY: &str = "13369376"; // 0x00CC0020 (BitBlt raster op)

/// The shared runtime io helpers the app-mode `io.input` body chains to (the
/// same symbols the console lowering emits): render the prompt, then read a line
/// from fd 0 — which app mode has redirected to the window input pipe.
const IO_WRITE_SYMBOL: &str = "_mfb_rt_io_io_write";
const IO_READ_LINE_SYMBOL: &str = "_mfb_rt_io_io_readLine";

pub(super) const FINISH_SYMBOL: &str = "_mfb_winapp_program_finish";

const CLASS_NAME_SYM: &str = "_mfb_winapp_class";
const TITLE_SYM: &str = "_mfb_winapp_title";
const HEADLESS_ENV_SYM: &str = "_mfb_winapp_headless_env";
const EDIT_CLASS_SYM: &str = "_mfb_winapp_edit_class"; // L"EDIT"
const DUMP_ENV_SYM: &str = "_mfb_winapp_dump_env"; // L"MFB_WINAPP_DUMP" (test readback gate)
const CRLF_SYM: &str = "_mfb_winapp_crlf"; // L"\r\n"
/// Writable 8-byte global holding the transcript EDIT control's HWND (0 until the
/// window is built). Written by `_main` (UI thread), read by `io_write` (worker
/// thread). `kind:"raw"` → the writable data partition.
const EDIT_HWND_SYM: &str = "_mfb_winapp_edit_hwnd";
/// Writable 8-byte global holding the main window HWND (the worker's finish helper
/// reads it to signal the UI thread to quit).
const MAIN_HWND_SYM: &str = "_mfb_winapp_main_hwnd";
/// Writable 8-byte global holding the input pipe's WRITE handle. `_main` (UI
/// thread) creates the pipe and stores the write end here; the EDIT subclass
/// (`editproc`, also UI thread) writes each typed byte to it. The worker thread
/// drains the READ end via fd 0 (`SetStdHandle(STD_INPUT, readEnd)`), so
/// `io::readLine`/`readChar` consume window keystrokes (plan-66-J-4).
const STDIN_WRITE_SYM: &str = "_mfb_winapp_stdin_write";
/// Writable 8-byte global holding the transcript EDIT control's original window
/// procedure (`SetWindowLongPtrW` returns it). `editproc` chains every message it
/// does not consume back to this proc, so the stock EDIT behaviour (painting,
/// `EM_REPLACESEL` transcript appends from J-3) is preserved.
const EDIT_OLDPROC_SYM: &str = "_mfb_winapp_edit_oldproc";
/// Read-only UTF-16 name of the `MFB_WINAPP_INPUT` env var: a test affordance that
/// makes `_main` inject each character of its value as a `WM_CHAR` to the EDIT
/// (then a final Enter), so the full subclass → pipe → `readLine` round-trip is
/// box-provable over ssh without a real keyboard.
const INPUT_ENV_SYM: &str = "_mfb_winapp_input_env";
/// Writable UTF-16 scratch buffer the keystroke-injection reads `MFB_WINAPP_INPUT`
/// into (250 wide chars + slack).
const INPUT_BUF_SYM: &str = "_mfb_winapp_inputbuf";
/// A custom worker→UI quit signal (`WM_APP`). The message loop catches it and exits
/// so the UI thread — which owns the window — performs teardown; a worker-thread
/// `ExitProcess` while the window/message-loop is live faults in GDI teardown.
const WM_APP_QUIT: &str = "32768"; // WM_APP (0x8000)

// WS_CHILD|WS_VISIBLE|WS_VSCROLL|ES_MULTILINE|ES_AUTOVSCROLL. NOT ES_READONLY: a
// read-only EDIT ignores EM_REPLACESEL, so programmatic transcript appends would
// silently no-op. A console transcript is also the input surface (like macOS's
// NSTextView), so an editable multiline control is the right model.
const EDIT_STYLE: &str = "1344274500"; // 0x50200044
const WM_GETTEXTLENGTH: &str = "14"; // 0x000E
const EM_SETSEL: &str = "177"; // 0x00B1
const EM_REPLACESEL: &str = "194"; // 0x00C2
const CP_UTF8: &str = "65001";

// WS_OVERLAPPEDWINDOW | WS_VISIBLE = 0x10CF0000; CW_USEDEFAULT = 0x80000000.
const WS_OVERLAPPED_VISIBLE: &str = "282001408"; // 0x10CF0000
const CW_USEDEFAULT: &str = "2147483648"; // 0x80000000
const FILE_FLAG_STDOUT_FD: usize = 11; // -(-11) STD_OUTPUT_HANDLE
const FILE_FLAG_STDERR_FD: usize = 12; // -(-12) STD_ERROR_HANDLE
const STD_INPUT_FD: usize = 10; // -(-10) STD_INPUT_HANDLE
const WM_DESTROY: &str = "2";
const WM_CHAR: &str = "258"; // 0x0102
const VK_RETURN: &str = "13"; // '\r' (WM_CHAR wParam on Enter)
const GWLP_WNDPROC: usize = 4; // -(-4) the SetWindowLongPtrW index for the wndproc

/// `bl symbol` to an imported DLL function + its external relocation.
fn call_external(
    from: &str,
    symbol: &str,
    library: &str,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) {
    ins.push(abi::branch_link(symbol));
    rel.push(CodeRelocation {
        from: from.to_string(),
        to: symbol.to_string(),
        kind: RelocIntent::Call,
        binding: "external".to_string(),
        library: Some(library.to_string()),
    });
}

/// `bl symbol` to an internal function + its internal-call relocation.
fn call_internal(
    from: &str,
    symbol: &str,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) {
    ins.push(abi::branch_link(symbol));
    rel.push(CodeRelocation {
        from: from.to_string(),
        to: symbol.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    });
}

/// Load the address of an internal symbol (a data object or a function) into
/// `reg` via the `adrp`/`add :lo12:` page pair. The thread-trampoline spawn
/// (`runtime_helpers.rs`) loads a *function* address with exactly this
/// `DataAddrHi/Lo` + `binding: "data"` shape, so it works for both.
fn load_addr(
    reg: &str,
    symbol: &str,
    from: &str,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) {
    ins.push(abi::load_page_address(reg, symbol));
    rel.push(CodeRelocation {
        from: from.to_string(),
        to: symbol.to_string(),
        kind: RelocIntent::DataAddrHi,
        binding: "data".to_string(),
        library: None,
    });
    ins.push(abi::add_page_offset(reg, reg, symbol));
    rel.push(CodeRelocation {
        from: from.to_string(),
        to: symbol.to_string(),
        kind: RelocIntent::DataAddrLo,
        binding: "data".to_string(),
        library: None,
    });
}

/// `_mfb_arena_alloc(size, align=2) -> RET[1] = ptr`. Size in the return register,
/// align in `ARG[1]` (matches `emit_marshal_path`). The 64 KiB requests never OOM
/// (the arena maps fresh 1 MiB+ blocks), so the Result tag is not checked.
fn arena_alloc(size: &str, from: &str, ins: &mut Vec<CodeInstruction>, rel: &mut Vec<CodeRelocation>) {
    ins.push(abi::move_immediate(abi::return_register(), "Integer", size));
    ins.push(abi::move_immediate(abi::ARG[1], "Integer", "2"));
    ins.push(abi::branch_link(ARENA_ALLOC_SYMBOL));
    rel.push(CodeRelocation {
        from: from.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    });
}

fn code_function(name: &str, symbol: &str, ins: Vec<CodeInstruction>, rel: Vec<CodeRelocation>) -> CodeFunction {
    CodeFunction {
        name: name.to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions: ins,
        relocations: rel,
    }
}

/// Emit the app-mode function set: `_main` (bootstrap + PE entry), the worker
/// shim, and `WndProc`. The io/term bodies are supplied by the separate
/// `emit_app_*_helper` trait methods.
pub(super) fn emit_app_program_entry(
    _spec: &AppEntrySpec,
    _platform_imports: &HashMap<String, String>,
) -> Result<Vec<CodeFunction>, String> {
    Ok(vec![
        emit_main(),
        emit_worker(),
        emit_wndproc(),
        emit_editproc(),
        emit_finish(),
    ])
}

/// `_main`: the PE entry. Frame (mirrors spike.rs): shadow [0x00..0x20], outgoing
/// stack args [0x20..0x60], WNDCLASSEXW [0x60..0xB0], MSG [0xB0..0xE0],
/// hInstance @0xE0, hwnd @0xE8, worker HANDLE @0xF0. FRAME 0xF8 keeps the PE
/// entry's `sp % 16 == 8` arrival 16-aligned before the first call.
fn emit_main() -> CodeFunction {
    const FRAME: usize = 0x118;
    const WNDCLASS: usize = 0x60;
    const MSG: usize = 0xB0;
    const HINSTANCE: usize = 0xE0;
    const HWND: usize = 0xE8;
    const WORKERH: usize = 0xF0;
    // plan-66-J-4 input-pipe slots (above the J-2/J-3 frame; FRAME stays ≡8 mod 16).
    const PIPEREAD: usize = 0xF8; // CreatePipe hReadPipe out-param
    const PIPEWRITE: usize = 0x100; // CreatePipe hWritePipe out-param
    const INJ_I: usize = 0x108; // keystroke-injection loop index
    const INJ_N: usize = 0x110; // keystroke-injection wide-char count
    let from = MAIN_SYMBOL;
    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel: Vec<CodeRelocation> = Vec::new();
    ins.push(abi::label("entry"));
    ins.push(abi::subtract_stack(FRAME));

    // hInstance = GetModuleHandleW(NULL)
    ins.push(abi::move_immediate(abi::ARG[0], "Integer", "0"));
    call_external(from, "GetModuleHandleW", KERNEL32, &mut ins, &mut rel);
    ins.push(abi::store_u64(abi::return_register(), abi::stack_pointer(), HINSTANCE));

    // headless = GetEnvironmentVariableW(L"MFB_WINAPP_HEADLESS", NULL, 0) != 0
    load_addr(abi::ARG[0], HEADLESS_ENV_SYM, from, &mut ins, &mut rel);
    ins.push(abi::move_immediate(abi::ARG[1], "Integer", "0"));
    ins.push(abi::move_immediate(abi::ARG[2], "Integer", "0"));
    call_external(from, "GetEnvironmentVariableW", KERNEL32, &mut ins, &mut rel);
    ins.push(abi::compare_immediate(abi::return_register(), "0"));
    ins.push(abi::branch_ne("headless_spawn"));

    // ---- GUI path: build + show the window (byte-equivalent to spike.rs) ----
    // Zero the 80-byte WNDCLASSEXW (10 qwords).
    for i in 0..10 {
        ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), WNDCLASS + i * 8));
    }
    // cbSize = 80 (store_u64 → cbSize@0=80, style@4=0).
    ins.push(abi::move_immediate(abi::ARG[0], "Integer", "80"));
    ins.push(abi::store_u64(abi::ARG[0], abi::stack_pointer(), WNDCLASS));
    // lpfnWndProc = &WndProc (@+8).
    load_addr(abi::ARG[0], WNDPROC_SYMBOL, from, &mut ins, &mut rel);
    ins.push(abi::store_u64(abi::ARG[0], abi::stack_pointer(), WNDCLASS + 8));
    // hInstance (@+24).
    ins.push(abi::load_u64(abi::ARG[0], abi::stack_pointer(), HINSTANCE));
    ins.push(abi::store_u64(abi::ARG[0], abi::stack_pointer(), WNDCLASS + 24));
    // lpszClassName = &class (@+64).
    load_addr(abi::ARG[0], CLASS_NAME_SYM, from, &mut ins, &mut rel);
    ins.push(abi::store_u64(abi::ARG[0], abi::stack_pointer(), WNDCLASS + 64));

    // RegisterClassExW(&wndclass)
    ins.push(abi::add_immediate(abi::ARG[0], abi::stack_pointer(), WNDCLASS));
    call_external(from, "RegisterClassExW", USER32, &mut ins, &mut rel);

    // CreateWindowExW(0, &class, &title, style, CW, CW, 400, 300, 0, 0, hInst, 0).
    // Stage the seven stack args (5th..11th) at [sp+0x20..0x58] through ARG[2] as a
    // caller-saved scratch BEFORE it becomes lpClassName's sibling — the SCRATCH
    // pool and ARG[4..] must NOT be used (their Win64 realizations are callee-saved,
    // and clobbering them corrupts the pinned/arena registers — see emit_open_file).
    ins.push(abi::move_immediate(abi::ARG[2], "Integer", CW_USEDEFAULT));
    ins.push(abi::store_u64(abi::ARG[2], abi::stack_pointer(), 0x20)); // x
    ins.push(abi::store_u64(abi::ARG[2], abi::stack_pointer(), 0x28)); // y
    ins.push(abi::move_immediate(abi::ARG[2], "Integer", "400"));
    ins.push(abi::store_u64(abi::ARG[2], abi::stack_pointer(), 0x30)); // width
    ins.push(abi::move_immediate(abi::ARG[2], "Integer", "300"));
    ins.push(abi::store_u64(abi::ARG[2], abi::stack_pointer(), 0x38)); // height
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x40)); // hWndParent
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x48)); // hMenu
    ins.push(abi::load_u64(abi::ARG[2], abi::stack_pointer(), HINSTANCE));
    ins.push(abi::store_u64(abi::ARG[2], abi::stack_pointer(), 0x50)); // hInstance
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x58)); // lpParam
    // Register args, ARG[2] (lpWindowName = &title) set last.
    ins.push(abi::move_immediate(abi::ARG[0], "Integer", "0"));
    load_addr(abi::ARG[1], CLASS_NAME_SYM, from, &mut ins, &mut rel);
    ins.push(abi::move_immediate(abi::ARG[3], "Integer", WS_OVERLAPPED_VISIBLE));
    load_addr(abi::ARG[2], TITLE_SYM, from, &mut ins, &mut rel);
    call_external(from, "CreateWindowExW", USER32, &mut ins, &mut rel);
    ins.push(abi::store_u64(abi::return_register(), abi::stack_pointer(), HWND));
    // Stash the main HWND so the worker's finish helper can signal the UI thread.
    ins.push(abi::load_u64(abi::ARG[0], abi::stack_pointer(), HWND));
    load_addr(abi::ARG[1], MAIN_HWND_SYM, from, &mut ins, &mut rel);
    ins.push(abi::store_u64(abi::ARG[0], abi::ARG[1], 0));

    // Transcript = a multiline EDIT child filling the window (the stock
    // control that IS the scrollback — the Win32 analog of macOS's NSTextView).
    // CreateWindowExW(0, L"EDIT", NULL, EDIT_STYLE, 0, 0, 400, 300, mainHwnd, 0,
    //                 hInstance, NULL). SendMessage cross-thread marshaling (the
    // worker → this UI thread) makes io::print append synchronously.
    // Stage the stack args through ARG[2] (set to lpWindowName=NULL last).
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x20)); // x = 0
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x28)); // y = 0
    ins.push(abi::move_immediate(abi::ARG[2], "Integer", "400"));
    ins.push(abi::store_u64(abi::ARG[2], abi::stack_pointer(), 0x30)); // width
    ins.push(abi::move_immediate(abi::ARG[2], "Integer", "300"));
    ins.push(abi::store_u64(abi::ARG[2], abi::stack_pointer(), 0x38)); // height
    ins.push(abi::load_u64(abi::ARG[2], abi::stack_pointer(), HWND));
    ins.push(abi::store_u64(abi::ARG[2], abi::stack_pointer(), 0x40)); // hWndParent = main
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x48)); // hMenu = 0 (child id)
    ins.push(abi::load_u64(abi::ARG[2], abi::stack_pointer(), HINSTANCE));
    ins.push(abi::store_u64(abi::ARG[2], abi::stack_pointer(), 0x50)); // hInstance
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x58)); // lpParam
    ins.push(abi::move_immediate(abi::ARG[0], "Integer", "0"));
    load_addr(abi::ARG[1], EDIT_CLASS_SYM, from, &mut ins, &mut rel);
    ins.push(abi::move_immediate(abi::ARG[3], "Integer", EDIT_STYLE));
    ins.push(abi::move_immediate(abi::ARG[2], "Integer", "0")); // lpWindowName = NULL (last)
    call_external(from, "CreateWindowExW", USER32, &mut ins, &mut rel);
    // Store the EDIT HWND into its writable global (load_addr writes ARG[1], not
    // the return register, so the fresh handle survives the address computation).
    load_addr(abi::ARG[1], EDIT_HWND_SYM, from, &mut ins, &mut rel);
    ins.push(abi::store_u64(abi::return_register(), abi::ARG[1], 0));

    // ---- plan-66-J-4 input wiring (GUI path) ----
    // CreatePipe(&hRead, &hWrite, NULL, 0): a byte pipe whose READ end becomes the
    // worker's stdin (fd 0) and whose WRITE end the EDIT subclass feeds keystrokes.
    ins.push(abi::add_immediate(abi::ARG[0], abi::stack_pointer(), PIPEREAD)); // &hRead
    ins.push(abi::add_immediate(abi::ARG[1], abi::stack_pointer(), PIPEWRITE)); // &hWrite
    ins.push(abi::move_immediate(abi::ARG[2], "Integer", "0")); // lpPipeAttributes = NULL
    ins.push(abi::move_immediate(abi::ARG[3], "Integer", "0")); // nSize = 0 (default buffer)
    call_external(from, "CreatePipe", KERNEL32, &mut ins, &mut rel);
    // SetStdHandle(STD_INPUT_HANDLE = -10, hRead): the worker's io::readLine reads
    // fd 0, which win emit_read_file resolves via GetStdHandle(-10); redirecting it
    // to the pipe read end makes readLine drain window keystrokes (plan-66-J-4).
    ins.push(abi::move_immediate(abi::ARG[0], "Integer", "0"));
    ins.push(abi::subtract_immediate(abi::ARG[0], abi::ARG[0], STD_INPUT_FD)); // -10
    ins.push(abi::load_u64(abi::ARG[1], abi::stack_pointer(), PIPEREAD)); // hRead
    call_external(from, "SetStdHandle", KERNEL32, &mut ins, &mut rel);
    // Stash hWrite in its global so the EDIT subclass (UI thread) can commit bytes.
    ins.push(abi::load_u64(abi::ARG[0], abi::stack_pointer(), PIPEWRITE));
    load_addr(abi::ARG[1], STDIN_WRITE_SYM, from, &mut ins, &mut rel);
    ins.push(abi::store_u64(abi::ARG[0], abi::ARG[1], 0));
    // Subclass the transcript EDIT: oldproc = SetWindowLongPtrW(edit, GWLP_WNDPROC =
    // -4, &editproc). editproc writes each WM_CHAR to the pipe then chains to
    // oldproc, so the stock EDIT behaviour (J-3 transcript appends) is preserved.
    load_addr(abi::ARG[0], EDIT_HWND_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::ARG[0], abi::ARG[0], 0)); // edit hwnd
    ins.push(abi::move_immediate(abi::ARG[1], "Integer", "0"));
    ins.push(abi::subtract_immediate(abi::ARG[1], abi::ARG[1], GWLP_WNDPROC)); // -4
    load_addr(abi::ARG[2], EDITPROC_SYMBOL, from, &mut ins, &mut rel); // &editproc
    call_external(from, "SetWindowLongPtrW", USER32, &mut ins, &mut rel);
    load_addr(abi::ARG[1], EDIT_OLDPROC_SYM, from, &mut ins, &mut rel);
    ins.push(abi::store_u64(abi::return_register(), abi::ARG[1], 0)); // oldproc

    // CreateThread(NULL, 0, &worker, hwnd, 0, NULL)
    ins.push(abi::move_immediate(abi::ARG[0], "Integer", "0"));
    ins.push(abi::move_immediate(abi::ARG[1], "Integer", "0"));
    load_addr(abi::ARG[2], WORKER_SYMBOL, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::ARG[3], abi::stack_pointer(), HWND));
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x20)); // dwCreationFlags
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x28)); // lpThreadId
    call_external(from, "CreateThread", KERNEL32, &mut ins, &mut rel);
    ins.push(abi::store_u64(abi::return_register(), abi::stack_pointer(), WORKERH));

    // ---- plan-66-J-4 keystroke injection (test affordance) ----
    // If MFB_WINAPP_INPUT is set, post each of its characters as a WM_CHAR to the
    // EDIT (then a final Enter), simulating typing so the subclass → pipe → readLine
    // round-trip is box-provable over ssh without a keyboard. The message loop below
    // dispatches these to editproc, which feeds the pipe on the UI thread.
    // n = GetEnvironmentVariableW(L"MFB_WINAPP_INPUT", inputbuf, 250)
    load_addr(abi::ARG[0], INPUT_ENV_SYM, from, &mut ins, &mut rel);
    load_addr(abi::ARG[1], INPUT_BUF_SYM, from, &mut ins, &mut rel);
    ins.push(abi::move_immediate(abi::ARG[2], "Integer", "250"));
    call_external(from, "GetEnvironmentVariableW", KERNEL32, &mut ins, &mut rel);
    ins.push(abi::store_u64(abi::return_register(), abi::stack_pointer(), INJ_N)); // count
    ins.push(abi::compare_immediate(abi::return_register(), "0"));
    ins.push(abi::branch_eq("inject_enter")); // unset/empty → just send Enter? no — skip all
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), INJ_I)); // i = 0
    ins.push(abi::label("inject_loop"));
    ins.push(abi::load_u64(abi::ARG[0], abi::stack_pointer(), INJ_I));
    ins.push(abi::load_u64(abi::ARG[1], abi::stack_pointer(), INJ_N));
    ins.push(abi::compare_registers(abi::ARG[0], abi::ARG[1]));
    ins.push(abi::branch_ge("inject_enter")); // i >= n → done, send Enter
    // ch = inputbuf[i] (a UTF-16 code unit); PostMessageW(edit, WM_CHAR, ch, 0).
    load_addr(abi::ARG[1], INPUT_BUF_SYM, from, &mut ins, &mut rel);
    ins.push(abi::shift_left_immediate(abi::ARG[0], abi::ARG[0], 1)); // i*2
    ins.push(abi::add_registers(abi::ARG[1], abi::ARG[1], abi::ARG[0]));
    ins.push(abi::load_u16(abi::ARG[2], abi::ARG[1], 0)); // wParam = ch
    load_addr(abi::ARG[0], EDIT_HWND_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::ARG[0], abi::ARG[0], 0)); // edit hwnd
    ins.push(abi::move_immediate(abi::ARG[1], "Integer", WM_CHAR));
    ins.push(abi::move_immediate(abi::ARG[3], "Integer", "0")); // lParam
    call_external(from, "PostMessageW", USER32, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::ARG[0], abi::stack_pointer(), INJ_I));
    ins.push(abi::add_immediate(abi::ARG[0], abi::ARG[0], 1));
    ins.push(abi::store_u64(abi::ARG[0], abi::stack_pointer(), INJ_I));
    ins.push(abi::branch("inject_loop"));
    ins.push(abi::label("inject_enter"));
    // A final Enter (WM_CHAR '\r') so readLine terminates the line.
    load_addr(abi::ARG[0], INPUT_ENV_SYM, from, &mut ins, &mut rel);
    load_addr(abi::ARG[1], INPUT_BUF_SYM, from, &mut ins, &mut rel);
    ins.push(abi::move_immediate(abi::ARG[2], "Integer", "250"));
    call_external(from, "GetEnvironmentVariableW", KERNEL32, &mut ins, &mut rel);
    ins.push(abi::compare_immediate(abi::return_register(), "0"));
    ins.push(abi::branch_eq("inject_done")); // MFB_WINAPP_INPUT unset → no injection at all
    load_addr(abi::ARG[0], EDIT_HWND_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::ARG[0], abi::ARG[0], 0));
    ins.push(abi::move_immediate(abi::ARG[1], "Integer", WM_CHAR));
    ins.push(abi::move_immediate(abi::ARG[2], "Integer", VK_RETURN)); // '\r'
    ins.push(abi::move_immediate(abi::ARG[3], "Integer", "0"));
    call_external(from, "PostMessageW", USER32, &mut ins, &mut rel);
    ins.push(abi::label("inject_done"));

    // Message loop.
    ins.push(abi::label("msg_loop"));
    ins.push(abi::add_immediate(abi::ARG[0], abi::stack_pointer(), MSG));
    ins.push(abi::move_immediate(abi::ARG[1], "Integer", "0"));
    ins.push(abi::move_immediate(abi::ARG[2], "Integer", "0"));
    ins.push(abi::move_immediate(abi::ARG[3], "Integer", "0"));
    call_external(from, "GetMessageW", USER32, &mut ins, &mut rel);
    ins.push(abi::compare_immediate(abi::return_register(), "0"));
    ins.push(abi::branch_le("main_done")); // 0 = WM_QUIT, -1 = error
    // The worker's finish posts WM_APP_QUIT (msg.message @ MSG+8); catch it and exit
    // the loop so the UI thread does teardown (a worker ExitProcess faults in GDI).
    ins.push(abi::load_u64(abi::ARG[1], abi::stack_pointer(), MSG + 8));
    ins.push(abi::move_immediate(abi::ARG[2], "Integer", "4294967295"));
    ins.push(abi::and_registers(abi::ARG[1], abi::ARG[1], abi::ARG[2])); // low 32 = message
    ins.push(abi::compare_immediate(abi::ARG[1], WM_APP_QUIT));
    ins.push(abi::branch_eq("main_done"));
    ins.push(abi::add_immediate(abi::ARG[0], abi::stack_pointer(), MSG));
    call_external(from, "TranslateMessage", USER32, &mut ins, &mut rel);
    ins.push(abi::add_immediate(abi::ARG[0], abi::stack_pointer(), MSG));
    call_external(from, "DispatchMessageW", USER32, &mut ins, &mut rel);
    ins.push(abi::branch("msg_loop"));

    // ---- headless path: spawn the worker and wait for it ----
    ins.push(abi::label("headless_spawn"));
    ins.push(abi::move_immediate(abi::ARG[0], "Integer", "0"));
    ins.push(abi::move_immediate(abi::ARG[1], "Integer", "0"));
    load_addr(abi::ARG[2], WORKER_SYMBOL, from, &mut ins, &mut rel);
    ins.push(abi::move_immediate(abi::ARG[3], "Integer", "0")); // lpParameter = NULL
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x20));
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x28));
    call_external(from, "CreateThread", KERNEL32, &mut ins, &mut rel);
    ins.push(abi::store_u64(abi::return_register(), abi::stack_pointer(), WORKERH));
    // WaitForSingleObject(worker, INFINITE = 0xFFFFFFFF via 0 - 1).
    ins.push(abi::load_u64(abi::ARG[0], abi::stack_pointer(), WORKERH));
    ins.push(abi::move_immediate(abi::ARG[1], "Integer", "0"));
    ins.push(abi::subtract_immediate(abi::ARG[1], abi::ARG[1], 1));
    call_external(from, "WaitForSingleObject", KERNEL32, &mut ins, &mut rel);

    // The loop ended (worker posted WM_APP_QUIT) or the headless worker exited.
    // Test affordance (plan-66-J-3 box proof): when MFB_WINAPP_DUMP is set, the UI
    // thread reads the transcript back (WM_GETTEXT) and writes the raw UTF-16 to
    // stdout, so an ssh box run can confirm io::print reached the window without a
    // visible display. Off by default (no side effect for real GUI runs).
    ins.push(abi::label("main_done"));
    load_addr(abi::ARG[0], EDIT_HWND_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::ARG[0], abi::ARG[0], 0));
    ins.push(abi::store_u64(abi::ARG[0], abi::stack_pointer(), 0x60));
    ins.push(abi::compare_immediate(abi::ARG[0], "0"));
    ins.push(abi::branch_eq("main_exit"));
    load_addr(abi::ARG[0], DUMP_ENV_SYM, from, &mut ins, &mut rel);
    load_addr(abi::ARG[1], "_mfb_winapp_testbuf", from, &mut ins, &mut rel);
    ins.push(abi::move_immediate(abi::ARG[2], "Integer", "200"));
    call_external(from, "GetEnvironmentVariableW", KERNEL32, &mut ins, &mut rel);
    ins.push(abi::compare_immediate(abi::return_register(), "0"));
    ins.push(abi::branch_eq("main_exit"));
    ins.push(abi::load_u64(abi::ARG[0], abi::stack_pointer(), 0x60));
    ins.push(abi::move_immediate(abi::ARG[1], "Integer", "13")); // WM_GETTEXT
    ins.push(abi::move_immediate(abi::ARG[2], "Integer", "250"));
    load_addr(abi::ARG[3], "_mfb_winapp_testbuf", from, &mut ins, &mut rel);
    call_external(from, "SendMessageW", USER32, &mut ins, &mut rel);
    ins.push(abi::move_immediate(abi::ARG[1], "Integer", "65535"));
    ins.push(abi::and_registers(abi::return_register(), abi::return_register(), abi::ARG[1]));
    ins.push(abi::shift_left_immediate(abi::ARG[0], abi::return_register(), 1));
    ins.push(abi::store_u64(abi::ARG[0], abi::stack_pointer(), 0x68)); // nbytes
    ins.push(abi::move_immediate(abi::ARG[0], "Integer", "0"));
    ins.push(abi::subtract_immediate(abi::ARG[0], abi::ARG[0], FILE_FLAG_STDOUT_FD));
    call_external(from, "GetStdHandle", KERNEL32, &mut ins, &mut rel);
    ins.push(abi::store_u64(abi::return_register(), abi::stack_pointer(), 0x70));
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x78));
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x20));
    ins.push(abi::load_u64(abi::ARG[0], abi::stack_pointer(), 0x70));
    load_addr(abi::ARG[1], "_mfb_winapp_testbuf", from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::ARG[2], abi::stack_pointer(), 0x68));
    ins.push(abi::add_immediate(abi::ARG[3], abi::stack_pointer(), 0x78));
    call_external(from, "WriteFile", KERNEL32, &mut ins, &mut rel);
    ins.push(abi::label("main_exit"));
    ins.push(abi::move_immediate(abi::ARG[0], "Integer", "0"));
    call_external(from, "ExitProcess", KERNEL32, &mut ins, &mut rel);
    ins.push(abi::branch_self());
    ins.push(abi::return_());
    code_function("winapp.bootstrap", MAIN_SYMBOL, ins, rel)
}

/// The worker thread: run the standard program body (which sets up the arena and
/// runs MFBASIC, then `ExitProcess`es). If it ever returns, `ExitThread(0)`.
fn emit_worker() -> CodeFunction {
    let from = WORKER_SYMBOL;
    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel: Vec<CodeRelocation> = Vec::new();
    ins.push(abi::label("entry"));
    ins.push(abi::subtract_stack(0x28));
    // No kernel argc/argv on a worker stack; the program body captures os::args
    // (if used) via GetCommandLineW itself (plan-66-B). Pass 0/0.
    ins.push(abi::move_immediate(abi::ARG[0], "Integer", "0"));
    ins.push(abi::move_immediate(abi::ARG[1], "Integer", "0"));
    call_internal(from, MACAPP_PROGRAM_SYMBOL, &mut ins, &mut rel);
    ins.push(abi::move_immediate(abi::ARG[0], "Integer", "0"));
    call_external(from, "ExitThread", KERNEL32, &mut ins, &mut rel);
    ins.push(abi::add_stack(0x28));
    ins.push(abi::return_());
    code_function("winapp.worker", WORKER_SYMBOL, ins, rel)
}

/// `WndProc(hwnd, msg, wParam, lParam)`: quit on `WM_DESTROY`, else default.
fn emit_wndproc() -> CodeFunction {
    // Frame (plan-66-J-5 added the WM_PAINT TUI present): shadow[0..0x20],
    // outgoing stack args [0x20..0x48] (BitBlt has 4 stack args), saved
    // hwnd@0x48/msg@0x50/wParam@0x58/lParam@0x60, hdc@0x68, PAINTSTRUCT@0x70..0xB8.
    // FRAME ≡ 8 (mod 16): entered at sp%16==8, so 0xB8 realigns before any call.
    const FRAME: usize = 0xB8;
    const H0: usize = 0x48; // hwnd
    const H1: usize = 0x50; // msg
    const H2: usize = 0x58; // wParam
    const H3: usize = 0x60; // lParam
    const HDC: usize = 0x68;
    const PS: usize = 0x70;
    let from = WNDPROC_SYMBOL;
    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel: Vec<CodeRelocation> = Vec::new();
    ins.push(abi::label("entry"));
    ins.push(abi::subtract_stack(FRAME));
    // Save the four WndProc args — the WM_PAINT path below clobbers ARG registers,
    // and the default DefWindowProcW tail needs them intact.
    ins.push(abi::store_u64(abi::ARG[0], abi::stack_pointer(), H0));
    ins.push(abi::store_u64(abi::ARG[1], abi::stack_pointer(), H1));
    ins.push(abi::store_u64(abi::ARG[2], abi::stack_pointer(), H2));
    ins.push(abi::store_u64(abi::ARG[3], abi::stack_pointer(), H3));
    // WM_PAINT + a live TUI surface → BitBlt the off-screen grid to the client.
    ins.push(abi::compare_immediate(abi::ARG[1], WM_PAINT));
    ins.push(abi::branch_ne("wnd_check_destroy"));
    load_addr(abi::ARG[0], TUI_MEMDC_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::ARG[0], abi::ARG[0], 0));
    ins.push(abi::compare_immediate(abi::ARG[0], "0"));
    ins.push(abi::branch_eq("wnd_default")); // no surface → normal paint
    // BeginPaint(hwnd, &ps) → hdc
    ins.push(abi::load_u64(abi::ARG[0], abi::stack_pointer(), H0));
    ins.push(abi::add_immediate(abi::ARG[1], abi::stack_pointer(), PS));
    call_external(from, "BeginPaint", USER32, &mut ins, &mut rel);
    ins.push(abi::store_u64(abi::return_register(), abi::stack_pointer(), HDC));
    // BitBlt(hdc, 0, 0, W, H, memDC, 0, 0, SRCCOPY) — args 5..9 on the stack.
    ins.push(abi::move_immediate(abi::ARG[2], "Integer", &(TUI_ROWS * TUI_CELL_H).to_string()));
    ins.push(abi::store_u64(abi::ARG[2], abi::stack_pointer(), 0x20)); // height (5th)
    load_addr(abi::ARG[2], TUI_MEMDC_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::ARG[2], abi::ARG[2], 0));
    ins.push(abi::store_u64(abi::ARG[2], abi::stack_pointer(), 0x28)); // hdcSrc (6th)
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x30)); // xSrc (7th)
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x38)); // ySrc (8th)
    ins.push(abi::move_immediate(abi::ARG[2], "Integer", SRCCOPY));
    ins.push(abi::store_u64(abi::ARG[2], abi::stack_pointer(), 0x40)); // rop (9th)
    ins.push(abi::load_u64(abi::ARG[0], abi::stack_pointer(), HDC)); // hdcDest
    ins.push(abi::move_immediate(abi::ARG[1], "Integer", "0")); // xDest
    ins.push(abi::move_immediate(abi::ARG[2], "Integer", "0")); // yDest
    ins.push(abi::move_immediate(abi::ARG[3], "Integer", &(TUI_COLS * TUI_CELL_W).to_string())); // width
    call_external(from, "BitBlt", GDI32, &mut ins, &mut rel);
    // EndPaint(hwnd, &ps)
    ins.push(abi::load_u64(abi::ARG[0], abi::stack_pointer(), H0));
    ins.push(abi::add_immediate(abi::ARG[1], abi::stack_pointer(), PS));
    call_external(from, "EndPaint", USER32, &mut ins, &mut rel);
    ins.push(abi::move_immediate(abi::return_register(), "Integer", "0"));
    ins.push(abi::add_stack(FRAME));
    ins.push(abi::return_());
    ins.push(abi::label("wnd_check_destroy"));
    ins.push(abi::compare_immediate(abi::ARG[1], WM_DESTROY));
    ins.push(abi::branch_ne("wnd_default"));
    ins.push(abi::move_immediate(abi::ARG[0], "Integer", "0"));
    call_external(from, "PostQuitMessage", USER32, &mut ins, &mut rel);
    ins.push(abi::move_immediate(abi::return_register(), "Integer", "0"));
    ins.push(abi::add_stack(FRAME));
    ins.push(abi::return_());
    // default: DefWindowProcW(hwnd, msg, wParam, lParam) — reload the saved args.
    ins.push(abi::label("wnd_default"));
    ins.push(abi::load_u64(abi::ARG[0], abi::stack_pointer(), H0));
    ins.push(abi::load_u64(abi::ARG[1], abi::stack_pointer(), H1));
    ins.push(abi::load_u64(abi::ARG[2], abi::stack_pointer(), H2));
    ins.push(abi::load_u64(abi::ARG[3], abi::stack_pointer(), H3));
    call_external(from, "DefWindowProcW", USER32, &mut ins, &mut rel);
    ins.push(abi::add_stack(FRAME));
    ins.push(abi::return_());
    code_function("winapp.wndproc", WNDPROC_SYMBOL, ins, rel)
}

/// `editproc(hwnd, msg, wParam, lParam)`: the transcript EDIT's subclass (plan-66-
/// J-4). On `WM_CHAR` it writes the typed byte to the input pipe — translating
/// Enter (`\r`) to `\n` so `io::readLine` terminates the line — then chains to the
/// stock EDIT proc so the character still echoes into the transcript. Every other
/// message chains straight through, so J-3's programmatic transcript appends
/// (`EM_REPLACESEL` via `SendMessageW`) are untouched. The keystrokes reach the
/// pipe per-character (the macOS keyDown model), so there is no fragile line
/// read-back to distinguish typed text from program output.
fn emit_editproc() -> CodeFunction {
    // Frame: shadow[0..0x20], 5th-arg slot@0x20, written@0x28, byte@0x30,
    // hwnd@0x38, msg@0x40, wParam@0x48, lParam@0x50. FRAME ≡ 8 (mod 16): the proc is
    // entered at sp%16==8 (post-call), so 0x58 realigns to 16 before any call.
    const FRAME: usize = 0x58;
    const OVERLAPPED: usize = 0x20;
    const WRITTEN: usize = 0x28;
    const BYTEBUF: usize = 0x30;
    const HWND: usize = 0x38;
    const MSG: usize = 0x40;
    const WPARAM: usize = 0x48;
    const LPARAM: usize = 0x50;
    let from = EDITPROC_SYMBOL;
    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel: Vec<CodeRelocation> = Vec::new();
    ins.push(abi::label("entry"));
    ins.push(abi::subtract_stack(FRAME));
    // Save the four WndProc arguments (calls below clobber the ARG registers).
    ins.push(abi::store_u64(abi::ARG[0], abi::stack_pointer(), HWND));
    ins.push(abi::store_u64(abi::ARG[1], abi::stack_pointer(), MSG));
    ins.push(abi::store_u64(abi::ARG[2], abi::stack_pointer(), WPARAM));
    ins.push(abi::store_u64(abi::ARG[3], abi::stack_pointer(), LPARAM));
    // Only WM_CHAR feeds the pipe; everything else chains straight through.
    ins.push(abi::compare_immediate(abi::ARG[1], WM_CHAR));
    ins.push(abi::branch_ne("chain"));
    // byte = (wParam == '\r') ? '\n' : (wParam & 0xFF). readLine terminates on '\n'.
    ins.push(abi::load_u64(abi::ARG[2], abi::stack_pointer(), WPARAM));
    ins.push(abi::compare_immediate(abi::ARG[2], VK_RETURN));
    ins.push(abi::branch_ne("not_cr"));
    ins.push(abi::move_immediate(abi::ARG[0], "Integer", "10")); // '\n'
    ins.push(abi::branch("store_byte"));
    ins.push(abi::label("not_cr"));
    ins.push(abi::move_immediate(abi::ARG[3], "Integer", "255"));
    ins.push(abi::and_registers(abi::ARG[0], abi::ARG[2], abi::ARG[3])); // low byte
    ins.push(abi::label("store_byte"));
    ins.push(abi::store_u8(abi::ARG[0], abi::stack_pointer(), BYTEBUF));
    // hWrite = *_mfb_winapp_stdin_write; skip if the pipe was never wired (headless).
    load_addr(abi::ARG[0], STDIN_WRITE_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::ARG[0], abi::ARG[0], 0));
    ins.push(abi::compare_immediate(abi::ARG[0], "0"));
    ins.push(abi::branch_eq("chain"));
    // WriteFile(hWrite, &byte, 1, &written, NULL)
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), OVERLAPPED)); // 5th arg NULL
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), WRITTEN));
    ins.push(abi::add_immediate(abi::ARG[1], abi::stack_pointer(), BYTEBUF)); // &byte
    ins.push(abi::move_immediate(abi::ARG[2], "Integer", "1"));
    ins.push(abi::add_immediate(abi::ARG[3], abi::stack_pointer(), WRITTEN)); // &written
    call_external(from, "WriteFile", KERNEL32, &mut ins, &mut rel);
    // chain: CallWindowProcW(oldproc, hwnd, msg, wParam, lParam) — 5th arg on stack.
    ins.push(abi::label("chain"));
    ins.push(abi::load_u64(abi::ARG[0], abi::stack_pointer(), LPARAM));
    ins.push(abi::store_u64(abi::ARG[0], abi::stack_pointer(), OVERLAPPED)); // lParam (5th)
    load_addr(abi::ARG[0], EDIT_OLDPROC_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::ARG[0], abi::ARG[0], 0)); // oldproc (rcx)
    ins.push(abi::load_u64(abi::ARG[1], abi::stack_pointer(), HWND)); // rdx
    ins.push(abi::load_u64(abi::ARG[2], abi::stack_pointer(), MSG)); // r8
    ins.push(abi::load_u64(abi::ARG[3], abi::stack_pointer(), WPARAM)); // r9
    call_external(from, "CallWindowProcW", USER32, &mut ins, &mut rel);
    // CallWindowProcW's LRESULT is already in the return register; return it.
    ins.push(abi::add_stack(FRAME));
    ins.push(abi::return_());
    code_function("winapp.editproc", EDITPROC_SYMBOL, ins, rel)
}

/// App-mode program-completion path (`emit_program_exit` routes the worker here
/// when `from == MACAPP_PROGRAM_SYMBOL`). The worker must NOT `ExitProcess` while
/// the GUI window/message-loop is live (that faults in GDI teardown), so it asks
/// the UI thread to quit: `PostMessageW(mainHwnd, WM_APP_QUIT)` — the message loop
/// catches it and exits to `main_done`, where the UI thread `ExitProcess`es. Then
/// `ExitThread(0)` retires the worker. In headless mode `mainHwnd` is 0, so
/// `PostMessageW` no-ops and the worker just exits, waking `_main`'s
/// `WaitForSingleObject`. (Keeping the window open after the program exits — the
/// macOS "park" behavior — is a J-5 refinement.)
fn emit_finish() -> CodeFunction {
    let from = FINISH_SYMBOL;
    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel: Vec<CodeRelocation> = Vec::new();
    ins.push(abi::label("entry"));
    ins.push(abi::subtract_stack(0x28));
    load_addr(abi::ARG[0], MAIN_HWND_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::ARG[0], abi::ARG[0], 0));
    ins.push(abi::move_immediate(abi::ARG[1], "Integer", WM_APP_QUIT));
    ins.push(abi::move_immediate(abi::ARG[2], "Integer", "0"));
    ins.push(abi::move_immediate(abi::ARG[3], "Integer", "0"));
    call_external(from, "PostMessageW", USER32, &mut ins, &mut rel);
    ins.push(abi::move_immediate(abi::ARG[0], "Integer", "0"));
    call_external(from, "ExitThread", KERNEL32, &mut ins, &mut rel);
    ins.push(abi::branch_self());
    ins.push(abi::return_());
    code_function("winapp.finish", FINISH_SYMBOL, ins, rel)
}

/// App-mode `io.print`/`io.write`/`io.printError`/`io.writeError` body (J-2): the
/// string object is in `ARG[0]` (`{u64 len @0; bytes @8}`); write it to the
/// inherited standard handle via `WriteFile(GetStdHandle(std), bytes, len,
/// &written, NULL)`. A GUI-subsystem `.exe` launched from a console inherits its
/// standard handles, so the box run observes the output. Returns `RESULT_OK_TAG`.
/// (J-3 routes this to the GDI transcript when a window is attached.)
pub(super) fn emit_app_io_write_helper(
    symbol: &str,
    stderr: bool,
    newline: bool,
    term_state_offset: Option<usize>,
) -> AppHookBody {
    // FRAME ≡ 8 (mod 16): entered at sp%16==8 (post-call), so this realigns to 16
    // before the SendMessageW/GDI calls (which use aligned SSE and fault otherwise).
    const FRAME: usize = 0x88;
    const OVERLAPPED: usize = 0x20; // WriteFile 5th arg / MultiByteToWideChar staging
    const NL_BYTE: usize = 0x30;
    const STR: usize = 0x38;
    const WRITTEN: usize = 0x40;
    const HANDLE: usize = 0x48;
    const EDITH: usize = 0x50; // transcript EDIT HWND
    const WBUF: usize = 0x58; // arena UTF-16 buffer
    // plan-66-J-5 TUI grid path slots.
    const GI: usize = 0x60; // per-char loop index
    const GWCH: usize = 0x68; // one UTF-16 code unit for TextOutW
    const GMEMDC: usize = 0x70; // cached memory DC
    let std_fd = if stderr { FILE_FLAG_STDERR_FD } else { FILE_FLAG_STDOUT_FD };
    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel: Vec<CodeRelocation> = Vec::new();
    ins.push(abi::label("entry"));
    ins.push(abi::subtract_stack(FRAME));
    ins.push(abi::store_u64(abi::ARG[0], abi::stack_pointer(), STR));
    // plan-66-J-5: while TUI mode is active, render into the GDI grid instead of the
    // transcript EDIT (the grid is what the window shows in TUI mode).
    if let Some(tso) = term_state_offset {
        ins.push(abi::load_u64(abi::ARG[0], ARENA_STATE_REGISTER, tso + TERM_STATE_ACTIVE_OFFSET));
        ins.push(abi::compare_immediate(abi::ARG[0], "0"));
        ins.push(abi::branch_ne("term_grid_path"));
    }
    // If a transcript EDIT control is attached (non-headless), route there; the
    // SendMessageW below marshals to the UI thread synchronously. Else fall through
    // to the inherited standard handle (headless / no window — the J-2 path).
    load_addr(abi::ARG[0], EDIT_HWND_SYM, symbol, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::ARG[0], abi::ARG[0], 0));
    ins.push(abi::compare_immediate(abi::ARG[0], "0"));
    ins.push(abi::branch_eq("std_path"));
    ins.push(abi::store_u64(abi::ARG[0], abi::stack_pointer(), EDITH));

    // --- transcript path: append to the EDIT control ---
    // wbuf = arena UTF-16 scratch.
    arena_alloc("65536", symbol, &mut ins, &mut rel);
    ins.push(abi::store_u64(abi::RET[1], abi::stack_pointer(), WBUF));
    // MultiByteToWideChar(CP_UTF8, 0, str+8, len, wbuf, 32767). Stage the two stack
    // args (5th/6th) through ARG[2] before it becomes lpMultiByteStr (the SCRATCH
    // pool must not be used on Win64 — see emit_marshal_path).
    ins.push(abi::load_u64(abi::ARG[2], abi::stack_pointer(), WBUF));
    ins.push(abi::store_u64(abi::ARG[2], abi::stack_pointer(), 0x20)); // lpWideCharStr (5th)
    ins.push(abi::move_immediate(abi::ARG[2], "Integer", "32767"));
    ins.push(abi::store_u64(abi::ARG[2], abi::stack_pointer(), 0x28)); // cchWideChar (6th)
    ins.push(abi::move_immediate(abi::ARG[0], "Integer", CP_UTF8));
    ins.push(abi::move_immediate(abi::ARG[1], "Integer", "0"));
    ins.push(abi::load_u64(abi::ARG[2], abi::stack_pointer(), STR)); // str ptr
    ins.push(abi::load_u64(abi::ARG[3], abi::ARG[2], 0)); // cbMultiByte = len
    ins.push(abi::add_immediate(abi::ARG[2], abi::ARG[2], 8)); // lpMultiByteStr = str+8
    call_external(symbol, "MultiByteToWideChar", KERNEL32, &mut ins, &mut rel);
    // NUL-terminate wbuf. The UTF-16 unit count ≤ the input byte count `str[0]`, so
    // `wbuf + str[0]*2` is a safe upper bound (exact for ASCII); use the trusted
    // byte length rather than MultiByteToWideChar's `int` return (whose garbage high
    // rax bits SIGSEGV'd `wbuf + garbage` in the non-headless transcript path). A
    // precise NUL for multi-byte text is a later refinement.
    ins.push(abi::load_u64(abi::ARG[0], abi::stack_pointer(), STR));
    ins.push(abi::load_u64(abi::ARG[0], abi::ARG[0], 0)); // len = str[0]
    ins.push(abi::shift_left_immediate(abi::ARG[0], abi::ARG[0], 1)); // len*2
    ins.push(abi::load_u64(abi::ARG[1], abi::stack_pointer(), WBUF));
    ins.push(abi::add_registers(abi::ARG[0], abi::ARG[1], abi::ARG[0])); // wbuf + len*2
    ins.push(abi::store_u16(abi::ZERO, abi::ARG[0], 0));
    // caretEnd = SendMessageW(edit, WM_GETTEXTLENGTH, 0, 0)
    ins.push(abi::load_u64(abi::ARG[0], abi::stack_pointer(), EDITH));
    ins.push(abi::move_immediate(abi::ARG[1], "Integer", WM_GETTEXTLENGTH));
    ins.push(abi::move_immediate(abi::ARG[2], "Integer", "0"));
    ins.push(abi::move_immediate(abi::ARG[3], "Integer", "0"));
    call_external(symbol, "SendMessageW", USER32, &mut ins, &mut rel);
    // SendMessageW(edit, EM_SETSEL, caretEnd, caretEnd) — collapse selection at end.
    // WM_GETTEXTLENGTH returns a C `int`; mask off the garbage high word of rax so
    // the caret position is not a wild value.
    ins.push(abi::move_immediate(abi::ARG[2], "Integer", "4294967295"));
    ins.push(abi::and_registers(abi::return_register(), abi::return_register(), abi::ARG[2]));
    ins.push(abi::move_register(abi::ARG[2], abi::return_register()));
    ins.push(abi::move_register(abi::ARG[3], abi::return_register()));
    ins.push(abi::load_u64(abi::ARG[0], abi::stack_pointer(), EDITH));
    ins.push(abi::move_immediate(abi::ARG[1], "Integer", EM_SETSEL));
    call_external(symbol, "SendMessageW", USER32, &mut ins, &mut rel);
    // SendMessageW(edit, EM_REPLACESEL, 0, wbuf) — insert the text at the caret.
    ins.push(abi::load_u64(abi::ARG[0], abi::stack_pointer(), EDITH));
    ins.push(abi::move_immediate(abi::ARG[1], "Integer", EM_REPLACESEL));
    ins.push(abi::move_immediate(abi::ARG[2], "Integer", "0"));
    ins.push(abi::load_u64(abi::ARG[3], abi::stack_pointer(), WBUF));
    call_external(symbol, "SendMessageW", USER32, &mut ins, &mut rel);
    if newline {
        // EDIT controls need CRLF, not a lone LF; append it after the inserted text.
        ins.push(abi::load_u64(abi::ARG[0], abi::stack_pointer(), EDITH));
        ins.push(abi::move_immediate(abi::ARG[1], "Integer", EM_REPLACESEL));
        ins.push(abi::move_immediate(abi::ARG[2], "Integer", "0"));
        load_addr(abi::ARG[3], CRLF_SYM, symbol, &mut ins, &mut rel);
        call_external(symbol, "SendMessageW", USER32, &mut ins, &mut rel);
    }
    ins.push(abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG));
    ins.push(abi::add_stack(FRAME));
    ins.push(abi::return_());

    // --- headless / no-window path: write to the inherited standard handle ---
    ins.push(abi::label("std_path"));
    // GetStdHandle(std) — std handle = -(fd) built without a negative immediate.
    ins.push(abi::move_immediate(abi::ARG[0], "Integer", "0"));
    ins.push(abi::subtract_immediate(abi::ARG[0], abi::ARG[0], std_fd));
    call_external(symbol, "GetStdHandle", KERNEL32, &mut ins, &mut rel);
    ins.push(abi::store_u64(abi::return_register(), abi::stack_pointer(), HANDLE));
    // WriteFile(handle, str+8, str[0], &written, NULL)
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), WRITTEN));
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), OVERLAPPED));
    ins.push(abi::load_u64(abi::ARG[0], abi::stack_pointer(), HANDLE));
    ins.push(abi::load_u64(abi::ARG[1], abi::stack_pointer(), STR)); // str ptr
    ins.push(abi::load_u64(abi::ARG[2], abi::ARG[1], 0)); // len = str[0]
    ins.push(abi::add_immediate(abi::ARG[1], abi::ARG[1], 8)); // buf = str+8
    ins.push(abi::add_immediate(abi::ARG[3], abi::stack_pointer(), WRITTEN));
    call_external(symbol, "WriteFile", KERNEL32, &mut ins, &mut rel);
    if newline {
        ins.push(abi::move_immediate(abi::ARG[0], "Integer", "10"));
        ins.push(abi::store_u8(abi::ARG[0], abi::stack_pointer(), NL_BYTE));
        ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), WRITTEN));
        ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), OVERLAPPED));
        ins.push(abi::load_u64(abi::ARG[0], abi::stack_pointer(), HANDLE));
        ins.push(abi::add_immediate(abi::ARG[1], abi::stack_pointer(), NL_BYTE));
        ins.push(abi::move_immediate(abi::ARG[2], "Integer", "1"));
        ins.push(abi::add_immediate(abi::ARG[3], abi::stack_pointer(), WRITTEN));
        call_external(symbol, "WriteFile", KERNEL32, &mut ins, &mut rel);
    }
    ins.push(abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG));
    ins.push(abi::add_stack(FRAME));
    ins.push(abi::return_());

    // --- plan-66-J-5 TUI grid path: draw the string cell-by-cell into the memory DC.
    // Reached only when TUI mode is active. `\n` advances the row (col=0); `\r`
    // homes the col; every other byte is drawn (ASCII → the same UTF-16 unit) at the
    // cursor with the current fg/bg, then the col advances (wrapping at TUI_COLS).
    if let Some(tso) = term_state_offset {
        ins.push(abi::label("term_grid_path"));
        load_addr(abi::ARG[0], TUI_MEMDC_SYM, symbol, &mut ins, &mut rel);
        ins.push(abi::load_u64(abi::ARG[0], abi::ARG[0], 0));
        ins.push(abi::store_u64(abi::ARG[0], abi::stack_pointer(), GMEMDC));
        ins.push(abi::compare_immediate(abi::ARG[0], "0"));
        ins.push(abi::branch_eq("std_path")); // no surface built → inherited handle
        // SetTextColor(memDC, fg); SetBkColor(memDC, bg).
        ins.push(abi::load_u64(abi::ARG[1], ARENA_STATE_REGISTER, tso + TERM_STATE_FG_OFFSET));
        ins.push(abi::load_u64(abi::ARG[0], abi::stack_pointer(), GMEMDC));
        call_external(symbol, "SetTextColor", GDI32, &mut ins, &mut rel);
        ins.push(abi::load_u64(abi::ARG[1], ARENA_STATE_REGISTER, tso + TERM_STATE_BG_OFFSET));
        ins.push(abi::load_u64(abi::ARG[0], abi::stack_pointer(), GMEMDC));
        call_external(symbol, "SetBkColor", GDI32, &mut ins, &mut rel);
        // for i in 0..len(str[0]) { ... }
        ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), GI));
        ins.push(abi::label("term_loop"));
        ins.push(abi::load_u64(abi::ARG[0], abi::stack_pointer(), GI));
        ins.push(abi::load_u64(abi::ARG[1], abi::stack_pointer(), STR));
        ins.push(abi::load_u64(abi::ARG[1], abi::ARG[1], 0)); // len = str[0]
        ins.push(abi::compare_registers(abi::ARG[0], abi::ARG[1]));
        ins.push(abi::branch_ge("term_grid_done"));
        // byte = *(str + 8 + i)
        ins.push(abi::load_u64(abi::ARG[2], abi::stack_pointer(), STR));
        ins.push(abi::add_immediate(abi::ARG[2], abi::ARG[2], 8));
        ins.push(abi::add_registers(abi::ARG[2], abi::ARG[2], abi::ARG[0]));
        ins.push(abi::load_u8(abi::ARG[0], abi::ARG[2], 0));
        ins.push(abi::compare_immediate(abi::ARG[0], "10"));
        ins.push(abi::branch_eq("term_nl"));
        ins.push(abi::compare_immediate(abi::ARG[0], "13"));
        ins.push(abi::branch_eq("term_cr"));
        // wch = byte; TextOutW(memDC, col<<3, row<<4, &wch, 1).
        ins.push(abi::store_u16(abi::ARG[0], abi::stack_pointer(), GWCH));
        ins.push(abi::move_immediate(abi::ARG[0], "Integer", "1"));
        ins.push(abi::store_u64(abi::ARG[0], abi::stack_pointer(), 0x20)); // 5th arg c=1
        ins.push(abi::add_immediate(abi::ARG[3], abi::stack_pointer(), GWCH)); // &wch
        load_addr(abi::ARG[1], TUI_COL_SYM, symbol, &mut ins, &mut rel);
        ins.push(abi::load_u64(abi::ARG[1], abi::ARG[1], 0));
        ins.push(abi::shift_left_immediate(abi::ARG[1], abi::ARG[1], 3)); // x = col*8
        load_addr(abi::ARG[2], TUI_ROW_SYM, symbol, &mut ins, &mut rel);
        ins.push(abi::load_u64(abi::ARG[2], abi::ARG[2], 0));
        ins.push(abi::shift_left_immediate(abi::ARG[2], abi::ARG[2], 4)); // y = row*16
        ins.push(abi::load_u64(abi::ARG[0], abi::stack_pointer(), GMEMDC));
        call_external(symbol, "TextOutW", GDI32, &mut ins, &mut rel);
        // col++; wrap at TUI_COLS → col=0, row++. NB: an earlier structure that
        // branched OVER an inline wrap block to a SHARED store (branch_lt skip →
        // fall-through store) mis-advanced the cursor on the x86 backend (cdb showed
        // col running backwards). This separated-paths form (each branch owns its
        // store) is verified correct on 2230. branch_lt itself is used correctly
        // elsewhere in code.rs, so the fault was the shared-store structure, not the
        // condition.
        load_addr(abi::ARG[2], TUI_COL_SYM, symbol, &mut ins, &mut rel);
        ins.push(abi::load_u64(abi::ARG[0], abi::ARG[2], 0));
        ins.push(abi::add_immediate(abi::ARG[0], abi::ARG[0], 1));
        ins.push(abi::compare_immediate(abi::ARG[0], &TUI_COLS.to_string()));
        ins.push(abi::branch_ge("term_wrap"));
        ins.push(abi::store_u64(abi::ARG[0], abi::ARG[2], 0)); // col = col+1
        ins.push(abi::branch("term_next"));
        ins.push(abi::label("term_wrap"));
        ins.push(abi::store_u64(abi::ZERO, abi::ARG[2], 0)); // col = 0
        load_addr(abi::ARG[1], TUI_ROW_SYM, symbol, &mut ins, &mut rel);
        ins.push(abi::load_u64(abi::ARG[0], abi::ARG[1], 0));
        ins.push(abi::add_immediate(abi::ARG[0], abi::ARG[0], 1));
        ins.push(abi::store_u64(abi::ARG[0], abi::ARG[1], 0)); // row++
        ins.push(abi::branch("term_next"));
        // '\n' → row++, col=0.
        ins.push(abi::label("term_nl"));
        load_addr(abi::ARG[1], TUI_ROW_SYM, symbol, &mut ins, &mut rel);
        ins.push(abi::load_u64(abi::ARG[0], abi::ARG[1], 0));
        ins.push(abi::add_immediate(abi::ARG[0], abi::ARG[0], 1));
        ins.push(abi::store_u64(abi::ARG[0], abi::ARG[1], 0));
        load_addr(abi::ARG[1], TUI_COL_SYM, symbol, &mut ins, &mut rel);
        ins.push(abi::store_u64(abi::ZERO, abi::ARG[1], 0));
        ins.push(abi::branch("term_next"));
        // '\r' → col=0.
        ins.push(abi::label("term_cr"));
        load_addr(abi::ARG[1], TUI_COL_SYM, symbol, &mut ins, &mut rel);
        ins.push(abi::store_u64(abi::ZERO, abi::ARG[1], 0));
        ins.push(abi::label("term_next"));
        ins.push(abi::load_u64(abi::ARG[0], abi::stack_pointer(), GI));
        ins.push(abi::add_immediate(abi::ARG[0], abi::ARG[0], 1));
        ins.push(abi::store_u64(abi::ARG[0], abi::stack_pointer(), GI));
        ins.push(abi::branch("term_loop"));
        ins.push(abi::label("term_grid_done"));
        invalidate_main(symbol, &mut ins, &mut rel);
        ins.push(abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG));
        ins.push(abi::add_stack(FRAME));
        ins.push(abi::return_());
    }
    (
        CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        ins,
        rel,
    )
}

/// App-mode `io.input` body (plan-66-J-4): render the prompt to the transcript
/// (via the shared `io.write` helper, whose app-mode body appends to the EDIT),
/// then read a line from fd 0 — which `_main` has redirected to the window input
/// pipe — via the shared `io.readLine` helper. The prompt string arrives in
/// `ARG[0]` and is consumed by `io.write`; `io.readLine` needs no argument and
/// leaves its `String` Result in the Result registers, which this tail returns.
/// Mirrors macOS `emit_app_io_input_helper`; on Win64 the return address is on the
/// stack (no link register), so the frame only reserves shadow space for the calls.
pub(super) fn emit_app_io_input_helper(symbol: &str) -> AppHookBody {
    // Entered at sp%16==8 (post-call); 0x28 (shadow 0x20 + 8) realigns to 16.
    let from = symbol;
    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel: Vec<CodeRelocation> = Vec::new();
    ins.push(abi::label("entry"));
    ins.push(abi::subtract_stack(0x28));
    call_internal(from, IO_WRITE_SYMBOL, &mut ins, &mut rel); // ARG[0] = prompt
    call_internal(from, IO_READ_LINE_SYMBOL, &mut ins, &mut rel); // result in Result regs
    ins.push(abi::add_stack(0x28));
    ins.push(abi::return_());
    (
        CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        ins,
        rel,
    )
}

/// App-mode setup for immediate, no-echo key reads (`io.readChar`/`readByte`).
/// On Windows the input pipe already delivers each keystroke byte as it is typed
/// (the EDIT subclass writes per `WM_CHAR`, unbuffered), so a single-byte read of
/// fd 0 returns the next key with no cooked-mode line buffering to disable — the
/// raw-mode flip is a no-op. Returns `Ok(())` so the shared read helpers treat raw
/// mode as supported (the trait's `None` would mean "not app mode").
pub(super) fn emit_app_raw_input_mode() -> Result<(), String> {
    Ok(())
}

/// App-mode `io.flush` body (J-2): standard-handle writes are unbuffered, so this
/// is a no-op that returns `RESULT_OK_TAG`. (J-3 drives the transcript present.)
pub(super) fn emit_app_io_flush_helper(_symbol: &str) -> AppHookBody {
    let mut ins: Vec<CodeInstruction> = Vec::new();
    ins.push(abi::label("entry"));
    ins.push(abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG));
    ins.push(abi::return_());
    (
        CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        ins,
        Vec::new(),
    )
}

/// App-mode `io.isInputTerminal`/`isOutputTerminal`/`isErrorTerminal` body: the
/// window (or its inherited console) IS the terminal, so all three return
/// `OK(TRUE)` — `RESULT_OK_TAG` in the tag register, `1` in the value register.
pub(super) fn emit_app_io_is_terminal_helper(_symbol: &str) -> AppHookBody {
    let mut ins: Vec<CodeInstruction> = Vec::new();
    ins.push(abi::label("entry"));
    ins.push(abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "1"));
    ins.push(abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG));
    ins.push(abi::return_());
    (
        CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        ins,
        Vec::new(),
    )
}

/// Wrap a raw instruction/relocation stream as a standalone app-hook body (its
/// own frame, entered at `sp%16==8`).
fn term_body(ins: Vec<CodeInstruction>, rel: Vec<CodeRelocation>) -> AppHookBody {
    (
        CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        ins,
        rel,
    )
}

/// plan-66-J-5: dispatch a `term::` call to its GDI-grid app-mode body. Every call
/// Windows advertises in app mode is handled here; the shared `mod.rs` gate then
/// prepends the presentation-mode `ErrWrongMode` guard. Returns `None` for a call
/// with no app body (falls through to the console ANSI backend).
pub(super) fn emit_app_term_helper(call: &str, symbol: &str, tso: usize) -> Option<AppHookBody> {
    let b = match call {
        "term.on" => emit_term_on(symbol, tso),
        "term.off" => emit_term_off(symbol, tso),
        "term.clear" => emit_term_clear(symbol),
        "term.moveTo" => emit_term_move_to(symbol),
        "term.setForeground" => emit_term_set_color(tso, TERM_STATE_FG_OFFSET),
        "term.setBackground" => emit_term_set_color(tso, TERM_STATE_BG_OFFSET),
        "term.setBold" => emit_term_set_flag(tso, TERM_STATE_BOLD_OFFSET),
        "term.setUnderline" => emit_term_set_flag(tso, TERM_STATE_UNDERLINE_OFFSET),
        "term.showCursor" => emit_term_cursor_visible(tso, "1"),
        "term.hideCursor" => emit_term_cursor_visible(tso, "0"),
        "term.sync" => emit_term_sync(symbol),
        "term.terminalSize" => emit_term_size(symbol),
        // The draw helpers stamp into the shared console cell grid, which the
        // GDI app surface (immediate-mode `TextOutW`, no cell buffer) does not
        // maintain. Rather than fall through to the console lowering — which would
        // dereference a grid app mode never allocates — raise a clean, trappable
        // `ErrUnsupported` here. Console builds still get the real draw via the
        // shared lowering (this arm is app-mode-only). GDI drawing is a follow-up
        // (plan-69 open decision / plan-66 GDI TUI backend).
        "term.drawHLine" | "term.drawVLine" | "term.drawBox" | "term.fillRect"
        | "term.drawText" | "term.drawGlyph" => emit_term_draw_unsupported(symbol),
        _ => return None,
    };
    Some(b)
}

/// App-mode guard for the `term::` draw helpers: raise the trappable
/// `ErrUnsupported` (the GDI surface has no cell grid to stamp into). Mirrors the
/// shared console `emit_unsupported` error shape — tag/code/message with no frame,
/// so the `ErrWrongMode` gate the caller prepends stays valid.
fn emit_term_draw_unsupported(symbol: &str) -> AppHookBody {
    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel: Vec<CodeRelocation> = Vec::new();
    ins.push(abi::label("entry"));
    ins.push(abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_UNSUPPORTED_CODE));
    ins.push(abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG));
    load_addr(RESULT_ERROR_MESSAGE_REGISTER, ERR_UNSUPPORTED_SYMBOL, symbol, &mut ins, &mut rel);
    ins.push(abi::return_());
    term_body(ins, rel)
}

/// `term::on()`: build the off-screen grid surface on first use (memory DC +
/// bitmap + a fixed-pitch stock font), clear it, mark TUI state active, hide the
/// transcript EDIT so the grid shows through, and invalidate the window.
fn emit_term_on(symbol: &str, tso: usize) -> AppHookBody {
    // Frame: shadow[0..0x20], PatBlt stack args h@0x20/rop@0x28, hdcScreen@0x30.
    const FRAME: usize = 0x38;
    let from = symbol;
    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel: Vec<CodeRelocation> = Vec::new();
    ins.push(abi::label("entry"));
    ins.push(abi::subtract_stack(FRAME));
    // Build the surface once (memDC == 0 means not built yet).
    load_addr(abi::ARG[0], TUI_MEMDC_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::ARG[0], abi::ARG[0], 0));
    ins.push(abi::compare_immediate(abi::ARG[0], "0"));
    ins.push(abi::branch_ne("on_have_dc"));
    // hdcScreen = GetDC(NULL)
    ins.push(abi::move_immediate(abi::ARG[0], "Integer", "0"));
    call_external(from, "GetDC", USER32, &mut ins, &mut rel);
    ins.push(abi::store_u64(abi::return_register(), abi::stack_pointer(), 0x30));
    // memDC = CreateCompatibleDC(hdcScreen); store the global.
    ins.push(abi::move_register(abi::ARG[0], abi::return_register()));
    call_external(from, "CreateCompatibleDC", GDI32, &mut ins, &mut rel);
    load_addr(abi::ARG[1], TUI_MEMDC_SYM, from, &mut ins, &mut rel);
    ins.push(abi::store_u64(abi::return_register(), abi::ARG[1], 0));
    // bmp = CreateCompatibleBitmap(hdcScreen, W, H)
    ins.push(abi::load_u64(abi::ARG[0], abi::stack_pointer(), 0x30));
    ins.push(abi::move_immediate(abi::ARG[1], "Integer", &(TUI_COLS * TUI_CELL_W).to_string()));
    ins.push(abi::move_immediate(abi::ARG[2], "Integer", &(TUI_ROWS * TUI_CELL_H).to_string()));
    call_external(from, "CreateCompatibleBitmap", GDI32, &mut ins, &mut rel);
    // SelectObject(memDC, bmp) — stage bmp (rax) into ARG[1] before loading memDC.
    ins.push(abi::move_register(abi::ARG[1], abi::return_register()));
    load_addr(abi::ARG[0], TUI_MEMDC_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::ARG[0], abi::ARG[0], 0));
    call_external(from, "SelectObject", GDI32, &mut ins, &mut rel);
    // ReleaseDC(NULL, hdcScreen)
    ins.push(abi::move_immediate(abi::ARG[0], "Integer", "0"));
    ins.push(abi::load_u64(abi::ARG[1], abi::stack_pointer(), 0x30));
    call_external(from, "ReleaseDC", USER32, &mut ins, &mut rel);
    // font = GetStockObject(SYSTEM_FIXED_FONT = 16); SelectObject(memDC, font).
    ins.push(abi::move_immediate(abi::ARG[0], "Integer", "16"));
    call_external(from, "GetStockObject", GDI32, &mut ins, &mut rel);
    ins.push(abi::move_register(abi::ARG[1], abi::return_register()));
    load_addr(abi::ARG[0], TUI_MEMDC_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::ARG[0], abi::ARG[0], 0));
    call_external(from, "SelectObject", GDI32, &mut ins, &mut rel);
    ins.push(abi::label("on_have_dc"));
    // Clear the grid to black: PatBlt(memDC, 0, 0, W, H, BLACKNESS = 0x42).
    ins.push(abi::move_immediate(abi::ARG[2], "Integer", &(TUI_ROWS * TUI_CELL_H).to_string()));
    ins.push(abi::store_u64(abi::ARG[2], abi::stack_pointer(), 0x20)); // height (5th)
    ins.push(abi::move_immediate(abi::ARG[2], "Integer", "66")); // BLACKNESS (6th)
    ins.push(abi::store_u64(abi::ARG[2], abi::stack_pointer(), 0x28));
    load_addr(abi::ARG[0], TUI_MEMDC_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::ARG[0], abi::ARG[0], 0));
    ins.push(abi::move_immediate(abi::ARG[1], "Integer", "0"));
    ins.push(abi::move_immediate(abi::ARG[2], "Integer", "0"));
    ins.push(abi::move_immediate(abi::ARG[3], "Integer", &(TUI_COLS * TUI_CELL_W).to_string()));
    call_external(from, "PatBlt", GDI32, &mut ins, &mut rel);
    // cursor = (0, 0); term state: active = 1, fg = white, bg = black.
    reset_cursor(from, &mut ins, &mut rel);
    ins.push(abi::move_immediate(abi::ARG[0], "Integer", "1"));
    ins.push(abi::store_u64(abi::ARG[0], ARENA_STATE_REGISTER, tso + TERM_STATE_ACTIVE_OFFSET));
    ins.push(abi::move_immediate(abi::ARG[0], "Integer", "16777215")); // 0xFFFFFF white
    ins.push(abi::store_u64(abi::ARG[0], ARENA_STATE_REGISTER, tso + TERM_STATE_FG_OFFSET));
    ins.push(abi::store_u64(abi::ZERO, ARENA_STATE_REGISTER, tso + TERM_STATE_BG_OFFSET));
    // Hide the transcript EDIT, then invalidate the window to present the grid.
    load_addr(abi::ARG[0], EDIT_HWND_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::ARG[0], abi::ARG[0], 0));
    ins.push(abi::move_immediate(abi::ARG[1], "Integer", SW_HIDE));
    call_external(from, "ShowWindow", USER32, &mut ins, &mut rel);
    invalidate_main(from, &mut ins, &mut rel);
    ins.push(abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG));
    ins.push(abi::add_stack(FRAME));
    ins.push(abi::return_());
    term_body(ins, rel)
}

/// `term::off()`: leave TUI mode — clear the active flag and re-show the EDIT.
fn emit_term_off(symbol: &str, tso: usize) -> AppHookBody {
    const FRAME: usize = 0x28;
    let from = symbol;
    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel: Vec<CodeRelocation> = Vec::new();
    ins.push(abi::label("entry"));
    ins.push(abi::subtract_stack(FRAME));
    ins.push(abi::store_u64(abi::ZERO, ARENA_STATE_REGISTER, tso + TERM_STATE_ACTIVE_OFFSET));
    load_addr(abi::ARG[0], EDIT_HWND_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::ARG[0], abi::ARG[0], 0));
    ins.push(abi::move_immediate(abi::ARG[1], "Integer", SW_SHOW));
    call_external(from, "ShowWindow", USER32, &mut ins, &mut rel);
    invalidate_main(from, &mut ins, &mut rel);
    ins.push(abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG));
    ins.push(abi::add_stack(FRAME));
    ins.push(abi::return_());
    term_body(ins, rel)
}

/// `term::clear()`: black out the grid and home the cursor.
fn emit_term_clear(symbol: &str) -> AppHookBody {
    const FRAME: usize = 0x38;
    let from = symbol;
    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel: Vec<CodeRelocation> = Vec::new();
    ins.push(abi::label("entry"));
    ins.push(abi::subtract_stack(FRAME));
    load_addr(abi::ARG[0], TUI_MEMDC_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::ARG[0], abi::ARG[0], 0));
    ins.push(abi::compare_immediate(abi::ARG[0], "0"));
    ins.push(abi::branch_eq("clear_done"));
    ins.push(abi::move_immediate(abi::ARG[2], "Integer", &(TUI_ROWS * TUI_CELL_H).to_string()));
    ins.push(abi::store_u64(abi::ARG[2], abi::stack_pointer(), 0x20));
    ins.push(abi::move_immediate(abi::ARG[2], "Integer", "66"));
    ins.push(abi::store_u64(abi::ARG[2], abi::stack_pointer(), 0x28));
    load_addr(abi::ARG[0], TUI_MEMDC_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::ARG[0], abi::ARG[0], 0));
    ins.push(abi::move_immediate(abi::ARG[1], "Integer", "0"));
    ins.push(abi::move_immediate(abi::ARG[2], "Integer", "0"));
    ins.push(abi::move_immediate(abi::ARG[3], "Integer", &(TUI_COLS * TUI_CELL_W).to_string()));
    call_external(from, "PatBlt", GDI32, &mut ins, &mut rel);
    reset_cursor(from, &mut ins, &mut rel);
    ins.push(abi::label("clear_done"));
    ins.push(abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG));
    ins.push(abi::add_stack(FRAME));
    ins.push(abi::return_());
    term_body(ins, rel)
}

/// `term::moveTo(row, col)`: set the grid cursor (0-based), no frame/call needed.
fn emit_term_move_to(symbol: &str) -> AppHookBody {
    let from = symbol;
    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel: Vec<CodeRelocation> = Vec::new();
    ins.push(abi::label("entry"));
    load_addr(abi::ARG[2], TUI_ROW_SYM, from, &mut ins, &mut rel);
    ins.push(abi::store_u64(abi::ARG[0], abi::ARG[2], 0)); // row
    load_addr(abi::ARG[2], TUI_COL_SYM, from, &mut ins, &mut rel);
    ins.push(abi::store_u64(abi::ARG[1], abi::ARG[2], 0)); // col
    ins.push(abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG));
    ins.push(abi::return_());
    term_body(ins, rel)
}

/// `term::setForeground/setBackground(r, g, b)`: pack `r | g<<8 | b<<16` (already
/// GDI COLORREF order) into the term-state color field.
fn emit_term_set_color(tso: usize, field: usize) -> AppHookBody {
    let mut ins: Vec<CodeInstruction> = Vec::new();
    ins.push(abi::label("entry"));
    ins.push(abi::shift_left_immediate(abi::ARG[1], abi::ARG[1], 8));
    ins.push(abi::shift_left_immediate(abi::ARG[2], abi::ARG[2], 16));
    ins.push(abi::or_registers(abi::ARG[0], abi::ARG[0], abi::ARG[1]));
    ins.push(abi::or_registers(abi::ARG[0], abi::ARG[0], abi::ARG[2]));
    ins.push(abi::store_u64(abi::ARG[0], ARENA_STATE_REGISTER, tso + field));
    ins.push(abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG));
    ins.push(abi::return_());
    term_body(ins, Vec::new())
}

/// `term::setBold/setUnderline(on)`: store the boolean into its term-state field.
fn emit_term_set_flag(tso: usize, field: usize) -> AppHookBody {
    let mut ins: Vec<CodeInstruction> = Vec::new();
    ins.push(abi::label("entry"));
    ins.push(abi::store_u64(abi::ARG[0], ARENA_STATE_REGISTER, tso + field));
    ins.push(abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG));
    ins.push(abi::return_());
    term_body(ins, Vec::new())
}

/// `term::showCursor/hideCursor()`: set the cursor-visible term-state flag.
fn emit_term_cursor_visible(tso: usize, value: &str) -> AppHookBody {
    let mut ins: Vec<CodeInstruction> = Vec::new();
    ins.push(abi::label("entry"));
    ins.push(abi::move_immediate(abi::ARG[0], "Integer", value));
    ins.push(abi::store_u64(
        abi::ARG[0],
        ARENA_STATE_REGISTER,
        tso + TERM_STATE_CURSOR_VISIBLE_OFFSET,
    ));
    ins.push(abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG));
    ins.push(abi::return_());
    term_body(ins, Vec::new())
}

/// `term::sync()`: present the coalesced grid — invalidate + update the window so
/// WndProc BitBlts the memory DC.
fn emit_term_sync(symbol: &str) -> AppHookBody {
    const FRAME: usize = 0x28;
    let from = symbol;
    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel: Vec<CodeRelocation> = Vec::new();
    ins.push(abi::label("entry"));
    ins.push(abi::subtract_stack(FRAME));
    invalidate_main(from, &mut ins, &mut rel);
    load_addr(abi::ARG[0], MAIN_HWND_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::ARG[0], abi::ARG[0], 0));
    call_external(from, "UpdateWindow", USER32, &mut ins, &mut rel);
    ins.push(abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG));
    ins.push(abi::add_stack(FRAME));
    ins.push(abi::return_());
    term_body(ins, rel)
}

/// `term::terminalSize()`: return `{ columns, rows }` (the fixed grid dims) as an
/// arena-allocated 16-byte record. Result value = record ptr, tag = OK.
fn emit_term_size(symbol: &str) -> AppHookBody {
    const FRAME: usize = 0x28;
    let from = symbol;
    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel: Vec<CodeRelocation> = Vec::new();
    ins.push(abi::label("entry"));
    ins.push(abi::subtract_stack(FRAME));
    // record = _mfb_arena_alloc(16, align 8) → RET[1] = ptr.
    ins.push(abi::move_immediate(abi::return_register(), "Integer", "16"));
    ins.push(abi::move_immediate(abi::ARG[1], "Integer", "8"));
    ins.push(abi::branch_link(ARENA_ALLOC_SYMBOL));
    rel.push(CodeRelocation {
        from: from.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    });
    ins.push(abi::move_immediate(abi::ARG[0], "Integer", &TUI_COLS.to_string()));
    ins.push(abi::store_u64(abi::ARG[0], abi::RET[1], 0)); // columns@0
    ins.push(abi::move_immediate(abi::ARG[0], "Integer", &TUI_ROWS.to_string()));
    ins.push(abi::store_u64(abi::ARG[0], abi::RET[1], 8)); // rows@8
    ins.push(abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG)); // RET[1]=ptr survives
    ins.push(abi::add_stack(FRAME));
    ins.push(abi::return_());
    term_body(ins, rel)
}

/// Zero the grid cursor row/col globals (uses ARG[0]/ARG[1] as scratch).
fn reset_cursor(from: &str, ins: &mut Vec<CodeInstruction>, rel: &mut Vec<CodeRelocation>) {
    load_addr(abi::ARG[0], TUI_ROW_SYM, from, ins, rel);
    ins.push(abi::store_u64(abi::ZERO, abi::ARG[0], 0));
    load_addr(abi::ARG[0], TUI_COL_SYM, from, ins, rel);
    ins.push(abi::store_u64(abi::ZERO, abi::ARG[0], 0));
}

/// `InvalidateRect(mainHwnd, NULL, TRUE)` — request a repaint of the whole client.
fn invalidate_main(from: &str, ins: &mut Vec<CodeInstruction>, rel: &mut Vec<CodeRelocation>) {
    load_addr(abi::ARG[0], MAIN_HWND_SYM, from, ins, rel);
    ins.push(abi::load_u64(abi::ARG[0], abi::ARG[0], 0));
    ins.push(abi::move_immediate(abi::ARG[1], "Integer", "0"));
    ins.push(abi::move_immediate(abi::ARG[2], "Integer", "1"));
    call_external(from, "InvalidateRect", USER32, ins, rel);
}

fn utf16z_hex(s: &str) -> String {
    let mut hex = String::new();
    for unit in s.encode_utf16() {
        for byte in unit.to_le_bytes() {
            hex.push_str(&format!("{byte:02x}"));
        }
    }
    hex.push_str("0000"); // UTF-16 NUL terminator
    hex
}

fn utf16z_data_object(symbol: &str, text: &str) -> CodeDataObject {
    CodeDataObject {
        symbol: symbol.to_string(),
        kind: "raw".to_string(),
        layout: "UTF-16LE C string (NUL-terminated)".to_string(),
        align: 2,
        size: text.encode_utf16().count() * 2 + 2,
        value: utf16z_hex(text),
    }
}

/// Read-only data the bootstrap references: the window class name, the title
/// (the project name), and the headless env-var name.
pub(super) fn app_mode_data_objects(project_name: &str) -> Vec<CodeDataObject> {
    let title = if project_name.is_empty() {
        "MFBASIC App"
    } else {
        project_name
    };
    vec![
        utf16z_data_object(CLASS_NAME_SYM, "MFBWinApp"),
        utf16z_data_object(TITLE_SYM, title),
        utf16z_data_object(HEADLESS_ENV_SYM, "MFB_WINAPP_HEADLESS"),
        utf16z_data_object(EDIT_CLASS_SYM, "EDIT"),
        utf16z_data_object(DUMP_ENV_SYM, "MFB_WINAPP_DUMP"),
        utf16z_data_object(CRLF_SYM, "\r\n"),
        utf16z_data_object(INPUT_ENV_SYM, "MFB_WINAPP_INPUT"),
        // Writable 8-byte globals (kind:"raw" → the writable data partition): the
        // transcript EDIT HWND and the main window HWND, both 0 until built.
        writable_qword(EDIT_HWND_SYM),
        writable_qword(MAIN_HWND_SYM),
        // plan-66-J-4 input state: the pipe write handle and the EDIT's original
        // window proc, both written by `_main` at window build (0 until then).
        writable_qword(STDIN_WRITE_SYM),
        writable_qword(EDIT_OLDPROC_SYM),
        // plan-66-J-5 term:: TUI grid state: the off-screen memory DC + cached font
        // (built lazily by term::on), and the grid cursor (row, col). 0 until on.
        writable_qword(TUI_MEMDC_SYM),
        writable_qword(TUI_FONT_SYM),
        writable_qword(TUI_ROW_SYM),
        writable_qword(TUI_COL_SYM),
        CodeDataObject {
            symbol: "_mfb_winapp_testbuf".to_string(),
            kind: "raw".to_string(),
            layout: "u8[512] (writable readback scratch)".to_string(),
            align: 2,
            size: 512,
            value: "00".repeat(512),
        },
        CodeDataObject {
            symbol: INPUT_BUF_SYM.to_string(),
            kind: "raw".to_string(),
            layout: "u16[256] (writable keystroke-injection scratch)".to_string(),
            align: 2,
            size: 512,
            value: "00".repeat(512),
        },
    ]
}

/// A zero-initialized writable 8-byte global (`kind:"raw"` lands it in the
/// writable data region per `layout_data_objects`).
fn writable_qword(symbol: &str) -> CodeDataObject {
    CodeDataObject {
        symbol: symbol.to_string(),
        kind: "raw".to_string(),
        layout: "u64 (writable global)".to_string(),
        align: 8,
        size: 8,
        value: "0000000000000000".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::shared::code::PresentationMode;

    fn spec() -> AppEntrySpec {
        AppEntrySpec {
            language_entry_accepts_args: false,
            uses_term: false,
            initial_mode: PresentationMode::Console,
        }
    }

    #[test]
    fn emits_main_worker_wndproc() {
        let fns = emit_app_program_entry(&spec(), &HashMap::new()).expect("app entry");
        let symbols: Vec<&str> = fns.iter().map(|f| f.symbol.as_str()).collect();
        // The PE entry MUST be "_main" (the image entry symbol in app mode), plus
        // the worker and WndProc the bootstrap references.
        assert!(symbols.contains(&MAIN_SYMBOL), "entry _main present: {symbols:?}");
        assert!(symbols.contains(&WORKER_SYMBOL), "worker present: {symbols:?}");
        assert!(symbols.contains(&WNDPROC_SYMBOL), "wndproc present: {symbols:?}");
    }

    #[test]
    fn main_references_worker_and_wndproc_and_dll_calls() {
        let fns = emit_app_program_entry(&spec(), &HashMap::new()).unwrap();
        let main = fns.iter().find(|f| f.symbol == MAIN_SYMBOL).unwrap();
        let targets: Vec<&str> = main.relocations.iter().map(|r| r.to.as_str()).collect();
        for want in [
            WORKER_SYMBOL,
            WNDPROC_SYMBOL,
            "GetModuleHandleW",
            "RegisterClassExW",
            "CreateWindowExW",
            "CreateThread",
            "GetMessageW",
            "GetEnvironmentVariableW",
            "WaitForSingleObject",
            "ExitProcess",
        ] {
            assert!(targets.contains(&want), "_main references {want}: {targets:?}");
        }
    }

    #[test]
    fn data_objects_are_utf16() {
        let objs = app_mode_data_objects("MyProj");
        let title = objs.iter().find(|o| o.symbol == TITLE_SYM).unwrap();
        // "MyProj" → 6 UTF-16 code units × 2 bytes + 2-byte NUL = 14.
        assert_eq!(title.size, 14);
        assert_eq!(title.align, 2);
        assert!(title.value.ends_with("0000"));
        // The writable HWND globals are 8-byte zero-init (writable data partition).
        let edit = objs.iter().find(|o| o.symbol == EDIT_HWND_SYM).unwrap();
        assert_eq!(edit.size, 8);
        assert_eq!(edit.value, "0000000000000000");
    }

    #[test]
    fn io_write_newline_variant_writes_twice() {
        let (_frame, ins, rel) = emit_app_io_write_helper("_test_io", false, true, None);
        let writes = rel.iter().filter(|r| r.to == "WriteFile").count();
        assert_eq!(writes, 2, "newline variant issues the text + '\\n' WriteFile");
        assert!(rel.iter().any(|r| r.to == "GetStdHandle"));
        assert!(!ins.is_empty());
    }

    #[test]
    fn io_write_has_transcript_path() {
        // plan-66-J-3: io_write routes to the EDIT control (transcript) when the
        // edit-hwnd global is set — MultiByteToWideChar the print text, then append
        // via EM_REPLACESEL SendMessageW — and falls back to the std handle otherwise.
        let (_frame, _ins, rel) = emit_app_io_write_helper("_test_io", false, true, None);
        assert!(
            rel.iter().any(|r| r.to == EDIT_HWND_SYM),
            "reads the transcript EDIT-hwnd global to choose the path"
        );
        assert!(
            rel.iter().any(|r| r.to == "MultiByteToWideChar"),
            "converts the UTF-8 print text to UTF-16 for the EDIT control"
        );
        let sends = rel.iter().filter(|r| r.to == "SendMessageW").count();
        // WM_GETTEXTLENGTH + EM_SETSEL + EM_REPLACESEL (+ CRLF EM_REPLACESEL for the
        // newline variant) = 4.
        assert_eq!(sends, 4, "transcript append issues 4 SendMessageW: {sends}");
        // Still has the std-handle fallback for headless / no-window.
        assert!(rel.iter().any(|r| r.to == "GetStdHandle"));
    }

    #[test]
    fn main_creates_edit_and_finish_signals_ui() {
        let fns = emit_app_program_entry(&spec(), &HashMap::new()).unwrap();
        let main = fns.iter().find(|f| f.symbol == MAIN_SYMBOL).unwrap();
        let targets: Vec<&str> = main.relocations.iter().map(|r| r.to.as_str()).collect();
        // Two CreateWindowExW (main window + EDIT child) and the two window globals.
        assert_eq!(
            targets.iter().filter(|t| **t == "CreateWindowExW").count(),
            2,
            "_main creates the main window and the transcript EDIT child"
        );
        assert!(targets.contains(&EDIT_CLASS_SYM), "references the L\"EDIT\" class");
        assert!(targets.contains(&EDIT_HWND_SYM) && targets.contains(&MAIN_HWND_SYM));
        // The finish helper must NOT ExitProcess on the worker (that faults in GDI
        // teardown); it posts WM_APP_QUIT so the UI thread tears down.
        let finish = fns.iter().find(|f| f.symbol == FINISH_SYMBOL).unwrap();
        let ft: Vec<&str> = finish.relocations.iter().map(|r| r.to.as_str()).collect();
        assert!(ft.contains(&"PostMessageW"), "finish signals the UI thread");
        assert!(ft.contains(&"ExitThread") && !ft.contains(&"ExitProcess"));
    }

    #[test]
    fn main_wires_input_pipe_and_subclasses_edit() {
        // plan-66-J-4: _main creates the input pipe, redirects fd 0 to its read end,
        // and subclasses the transcript EDIT so typed keystrokes reach the worker.
        let fns = emit_app_program_entry(&spec(), &HashMap::new()).unwrap();
        let main = fns.iter().find(|f| f.symbol == MAIN_SYMBOL).unwrap();
        let targets: Vec<&str> = main.relocations.iter().map(|r| r.to.as_str()).collect();
        for want in ["CreatePipe", "SetStdHandle", "SetWindowLongPtrW"] {
            assert!(targets.contains(&want), "_main calls {want}: {targets:?}");
        }
        // Stashes the pipe write handle and the EDIT's original proc for editproc.
        assert!(targets.contains(&STDIN_WRITE_SYM) && targets.contains(&EDIT_OLDPROC_SYM));
        // The subclass function is emitted and installed.
        assert!(targets.contains(&EDITPROC_SYMBOL), "installs editproc as the subclass");
        assert!(
            fns.iter().any(|f| f.symbol == EDITPROC_SYMBOL),
            "editproc function is emitted"
        );
    }

    #[test]
    fn editproc_feeds_pipe_then_chains() {
        // editproc must write typed bytes to the pipe (WriteFile) AND chain every
        // message to the stock EDIT proc (CallWindowProcW) so J-3's transcript
        // appends are preserved.
        let fns = emit_app_program_entry(&spec(), &HashMap::new()).unwrap();
        let ep = fns.iter().find(|f| f.symbol == EDITPROC_SYMBOL).unwrap();
        let targets: Vec<&str> = ep.relocations.iter().map(|r| r.to.as_str()).collect();
        assert!(targets.contains(&"WriteFile"), "editproc writes keystrokes to the pipe");
        assert!(
            targets.contains(&"CallWindowProcW"),
            "editproc chains to the original EDIT proc (no J-3 regression)"
        );
        assert!(targets.contains(&STDIN_WRITE_SYM), "reads the pipe write handle global");
        assert!(targets.contains(&EDIT_OLDPROC_SYM), "reads the saved original proc");
    }

    #[test]
    fn input_helper_writes_prompt_then_reads_line() {
        // io.input renders the prompt (io.write) then reads a line (io.readLine),
        // which drains fd 0 — the window input pipe.
        let (_frame, _ins, rel) = emit_app_io_input_helper("_test_input");
        let targets: Vec<&str> = rel.iter().map(|r| r.to.as_str()).collect();
        assert!(targets.contains(&IO_WRITE_SYMBOL), "renders the prompt via io.write");
        assert!(targets.contains(&IO_READ_LINE_SYMBOL), "reads the line via io.readLine");
    }

    #[test]
    fn input_data_objects_present() {
        let objs = app_mode_data_objects("P");
        // The injection env-var name (UTF-16) and the two input-state writable globals.
        assert!(objs.iter().any(|o| o.symbol == INPUT_ENV_SYM));
        let w = objs.iter().find(|o| o.symbol == STDIN_WRITE_SYM).unwrap();
        assert_eq!(w.size, 8);
        assert!(objs.iter().any(|o| o.symbol == EDIT_OLDPROC_SYM));
        assert!(objs.iter().any(|o| o.symbol == INPUT_BUF_SYM));
    }

    #[test]
    fn term_helper_dispatches_every_advertised_call() {
        // plan-66-J-5: every term:: call Windows advertises in app mode gets a body.
        for call in [
            "term.on",
            "term.off",
            "term.clear",
            "term.moveTo",
            "term.setForeground",
            "term.setBackground",
            "term.setBold",
            "term.setUnderline",
            "term.showCursor",
            "term.hideCursor",
            "term.sync",
            "term.terminalSize",
            // Draw helpers: app mode raises a clean ErrUnsupported (guarded, not a
            // fall-through to the console lowering that would crash).
            "term.drawHLine",
            "term.drawVLine",
            "term.drawBox",
            "term.fillRect",
            "term.drawText",
            "term.drawGlyph",
        ] {
            assert!(
                emit_app_term_helper(call, "_t", 0).is_some(),
                "no app-mode term body for {call}"
            );
        }
        // A non-term call falls through (None → console backend).
        assert!(emit_app_term_helper("term.bogus", "_t", 0).is_none());
    }

    #[test]
    fn term_on_builds_and_shows_grid() {
        let (_f, _i, rel) = emit_term_on("_t", 0);
        let t: Vec<&str> = rel.iter().map(|r| r.to.as_str()).collect();
        // Builds the off-screen surface, clears it, and hides the transcript EDIT.
        for want in ["CreateCompatibleDC", "CreateCompatibleBitmap", "GetStockObject", "PatBlt", "ShowWindow"] {
            assert!(t.contains(&want), "term::on missing {want}");
        }
        assert!(t.contains(&TUI_MEMDC_SYM) && t.contains(&EDIT_HWND_SYM));
    }

    #[test]
    fn wndproc_bitblts_the_grid_on_paint() {
        let fns = emit_app_program_entry(&spec(), &HashMap::new()).unwrap();
        let wp = fns.iter().find(|f| f.symbol == WNDPROC_SYMBOL).unwrap();
        let t: Vec<&str> = wp.relocations.iter().map(|r| r.to.as_str()).collect();
        assert!(t.contains(&"BeginPaint") && t.contains(&"BitBlt") && t.contains(&"EndPaint"));
        assert!(t.contains(&TUI_MEMDC_SYM), "WM_PAINT gates on the memory DC");
    }

    #[test]
    fn io_write_routes_to_grid_when_term_active() {
        // With a term-state offset, io.write gains the TUI grid branch (TextOutW +
        // SetTextColor); without it (None), the body is the J-3 transcript path only.
        let (_f, _i, rel_term) = emit_app_io_write_helper("_t", false, true, Some(0));
        let t: Vec<&str> = rel_term.iter().map(|r| r.to.as_str()).collect();
        assert!(t.contains(&"TextOutW") && t.contains(&"SetTextColor"));
        let (_f2, _i2, rel_plain) = emit_app_io_write_helper("_t", false, true, None);
        assert!(!rel_plain.iter().any(|r| r.to == "TextOutW"), "no grid path without term state");
    }
}
