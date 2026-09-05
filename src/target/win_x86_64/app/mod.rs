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
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::types::AppEntrySpec;
use crate::codegen::engine::types::CodeDataObject;
use crate::codegen::engine::types::CodeFrame;
use crate::codegen::engine::types::CodeFunction;
use crate::codegen::engine::types::CodeInstruction;
use crate::codegen::engine::types::CodeRelocation;
use crate::codegen::engine::types::PresentationMode;
use crate::codegen::engine::types::RelocIntent;
use crate::codegen::error::constants::ARENA_ALLOC_SYMBOL;
use crate::codegen::error::constants::ARENA_STATE_REGISTER;
use crate::codegen::error::constants::MACAPP_PROGRAM_SYMBOL;
use crate::codegen::error::constants::RESULT_OK_TAG;
use crate::codegen::error::constants::RESULT_TAG_REGISTER;
use crate::codegen::error::constants::RESULT_VALUE_REGISTER;
use crate::codegen::error::constants::TERM_STATE_ACTIVE_OFFSET;
use crate::codegen::error::constants::TERM_STATE_BG_OFFSET;
use crate::codegen::error::constants::TERM_STATE_BOLD_OFFSET;
use crate::codegen::error::constants::TERM_STATE_CURSOR_VISIBLE_OFFSET;
use crate::codegen::error::constants::TERM_STATE_FG_OFFSET;
use crate::codegen::error::constants::TERM_STATE_UNDERLINE_OFFSET;

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
const TUI_CELL_W: usize = 8; // px per cell (matches the SYSTEM_FIXED_FONT metrics we request)
const TUI_CELL_H: usize = 16;
/// Writable u64 globals for the TUI surface, all 0 until `term::on` builds them.
const TUI_MEMDC_SYM: &str = "_mfb_winapp_tui_memdc"; // off-screen HDC
const TUI_ROW_SYM: &str = "_mfb_winapp_tui_row"; // cursor row (0-based)
const TUI_COL_SYM: &str = "_mfb_winapp_tui_col"; // cursor col (0-based)
                                                 // plan-70-F: a real fixed-pitch CJK-capable font (CreateFontW) replaces the legacy
                                                 // SYSTEM_FIXED_FONT bitmap face, which has no CJK/emoji glyphs. DEFAULT_CHARSET lets
                                                 // GDI font-linking supply CJK from the system fallback (MS Gothic/JhengHei/Malgun,
                                                 // present on the box). The HFONT is cached so it is created once with the surface.
const TUI_FONT_SYM: &str = "_mfb_winapp_tui_font"; // cached HFONT (0 until term::on)
const FONT_NAME_SYM: &str = "_mfb_winapp_tui_fontname"; // L"Consolas"
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
/// plan-98-A Phase 3: a custom worker→UI presentation-mode reconcile signal
/// (`WM_APP + 1`), `wParam` = the new `Mode` discriminant. Sent with `SendMessageW`,
/// not posted: `SendMessageW` from a non-owning thread blocks until the owning
/// thread's message pump dispatches it, which is the Win32 analogue of macOS's
/// `performSelectorOnMainThread:waitUntilDone:YES` — the worker's next `getMode` or
/// `io::` call must observe the reconciled surface, so the marshal has to be
/// synchronous.
const WM_APP_RECONCILE: &str = "32769"; // WM_APP + 1 (0x8001)
/// plan-98-C Phase 3: a rendered frame is ready. `wParam` carries the frame block,
/// and the UI thread takes ownership of it.
///
/// `PostMessageW`, not `SendMessageW` like the reconcile: the worker has nothing to
/// wait for once the block is handed over, and blocking it on the UI thread for
/// every frame would serialise rendering behind painting.
const WM_APP_BLIT: &str = "32770"; // WM_APP + 2 (0x8002)
/// `DIB_RGB_COLORS` — the colour table is literal RGB, not palette indices.
const DIB_RGB_COLORS: &str = "0";
/// Writable 8-byte global holding the main window's HWND *while in `Mode.Canvas`*,
/// and 0 otherwise. On Windows the HWND itself is the native surface handle
/// (`VK_KHR_win32_surface` takes an `HWND`), so there is nothing to create — this
/// records that a canvas surface is currently presented, which is what makes
/// "retrievable in canvas mode, released after exit" observable, and is where
/// plan-98-F reads the handle from.
const CANVAS_HWND_SYM: &str = "_mfb_winapp_canvas_hwnd";
/// plan-98-C Phase 3: the committed frame, as one `HeapAlloc` block holding its own
/// width at +0, height at +8 and BGRX pixels from +16.
///
/// Width and height travel inside the block so one pointer carries a whole frame,
/// which is what makes the handoff race-free without a lock: the worker builds a
/// block nobody else can see, posts the pointer, and every read *and* write of this
/// symbol happens on the UI thread. Publishing the dimensions separately would let
/// `WM_PAINT` read a new frame with the previous frame's height — past the end of
/// the block.
const CANVAS_FRAME_SYM: &str = "_mfb_winapp_canvas_frame";
/// plan-98-C Phase 3: the worker-side frame blit.
const CANVAS_BLIT_SYMBOL: &str = "_mfb_winapp_canvas_blit";
/// Writable 8-byte global holding the transcript EDIT's HWND *unconditionally* —
/// the surviving copy of [`EDIT_HWND_SYM`], which the reconcile zeroes outside
/// `Console` so `io::` writes degrade to the fd sink (the mode's write contract, and
/// the exact analogue of the macOS `ASSOC_KEY` clear). Restoring `Console` copies
/// this back, so the transcript control is never lost, only unrouted.
const EDIT_HWND_SAVED_SYM: &str = "_mfb_winapp_edit_hwnd_saved";

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
/// `WM_SIZE`. `lParam` carries the new **client** size: width in the low word,
/// height in the high word.
const WM_SIZE: &str = "5";
/// The scripted-resize environment variables, and the wide buffer their values are
/// read into. plan-98-F Phase 3's Windows twin of the GTK backend's
/// `emit_headless_scripted_resize`: a resize is a *window* event and headless has no
/// window, so without this the WM_SIZE arm below could be implemented and never
/// executed on the one box that can run it.
const RESIZE_W_ENV_SYM: &str = "_mfb_winapp_resize_w_env";
const RESIZE_H_ENV_SYM: &str = "_mfb_winapp_resize_h_env";
const RESIZE_BUF_SYM: &str = "_mfb_winapp_resize_buf";
/// Wide chars in [`RESIZE_BUF_SYM`]. A surface dimension is at most five digits.
const RESIZE_BUF_CHARS: usize = 16;
/// `canvas::signalRedraw` — the scripted resize asks for the repaint the WM_SIZE it
/// simulates would have asked for. Spelled here as the GTK backend spells it; the
/// registry owns the name.
const CANVAS_SIGNAL_REDRAW_SYMBOL: &str = "_mfb_rt_canvas_canvas_signalRedraw";
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
    reg: impl Into<Operand>,
    symbol: &str,
    from: &str,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) {
    let reg = reg.into();
    ins.push(abi::load_page_address(&reg, symbol));
    rel.push(CodeRelocation {
        from: from.to_string(),
        to: symbol.to_string(),
        kind: RelocIntent::DataAddrHi,
        binding: "data".to_string(),
        library: None,
    });
    ins.push(abi::add_page_offset(&reg, &reg, symbol));
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
fn arena_alloc(
    size: &str,
    from: &str,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) {
    ins.push(abi::move_immediate(abi::return_register(), "Integer", size));
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "2"));
    ins.push(abi::branch_link(ARENA_ALLOC_SYMBOL));
    rel.push(CodeRelocation {
        from: from.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    });
}

/// plan-70-F: store the display width (1 or 2) of the codepoint at `sp+cp_off` into
/// `sp+w_off`. A compact East-Asian-Wide range check (the standard wcwidth-style
/// approximation) instead of A's full utf8proc table — the Win64 backend has only
/// ARG[0..3] usable (no SCRATCH pool), so the two-stage trie is impractical here, and
/// a range test keeps the ~1.5 MB table out of every Windows app. Covers CJK
/// ideographs, Kana, Hangul, fullwidth forms, and astral emoji/CJK-ext. Uses
/// ARG[0]/ARG[1] only; labels `ww_*` are function-local (emitted once per helper).
fn emit_win_wide_width(ins: &mut Vec<CodeInstruction>, cp_off: usize, w_off: usize) {
    const WIDE_RANGES: [(u32, u32); 13] = [
        (0x1100, 0x115F),
        (0x2E80, 0x303E),
        (0x3041, 0x33FF),
        (0x3400, 0x4DBF),
        (0x4E00, 0x9FFF),
        (0xA000, 0xA4CF),
        (0xAC00, 0xD7A3),
        (0xF900, 0xFAFF),
        (0xFE30, 0xFE4F),
        (0xFF00, 0xFF60),
        (0xFFE0, 0xFFE6),
        (0x1F300, 0x1FAFF),
        (0x20000, 0x3FFFD),
    ];
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "1"));
    ins.push(abi::store_u64(abi::mfb_arg(1), abi::stack_pointer(), w_off)); // width = 1
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), cp_off)); // cp
    for (i, (lo, hi)) in WIDE_RANGES.iter().enumerate() {
        let next = format!("ww_next_{i}");
        ins.push(abi::move_immediate(
            abi::mfb_arg(1),
            "Integer",
            &lo.to_string(),
        ));
        ins.push(abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(1)));
        ins.push(abi::branch_lt(&next));
        ins.push(abi::move_immediate(
            abi::mfb_arg(1),
            "Integer",
            &hi.to_string(),
        ));
        ins.push(abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(1)));
        ins.push(abi::branch_gt(&next));
        ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "2"));
        ins.push(abi::store_u64(abi::mfb_arg(1), abi::stack_pointer(), w_off)); // width = 2
        ins.push(abi::branch("ww_done"));
        ins.push(abi::label(&next));
    }
    ins.push(abi::label("ww_done"));
}

fn code_function(
    name: &str,
    symbol: &str,
    ins: Vec<CodeInstruction>,
    rel: Vec<CodeRelocation>,
) -> CodeFunction {
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
    spec: &AppEntrySpec,
    _platform_imports: &HashMap<String, String>,
) -> Result<Vec<CodeFunction>, String> {
    Ok(vec![
        emit_main(spec.initial_mode, spec.uses_canvas),
        emit_worker(),
        emit_wndproc(spec.uses_canvas),
        // plan-98-C Phase 3: the frame blit's worker side. Emitted unconditionally
        // like the wndproc it posts to — whether a program ever enters canvas mode is
        // a runtime question, not a static one.
        emit_canvas_blit_helper(),
        emit_editproc(),
        emit_finish(),
    ])
}

/// `_main`: the PE entry. Frame (mirrors spike.rs): shadow [0x00..0x20], outgoing
/// stack args [0x20..0x60], WNDCLASSEXW [0x60..0xB0], MSG [0xB0..0xE0],
/// hInstance @0xE0, hwnd @0xE8, worker HANDLE @0xF0. FRAME 0xF8 keeps the PE
/// entry's `sp % 16 == 8` arrival 16-aligned before the first call.
fn emit_main(initial_mode: PresentationMode, uses_canvas: bool) -> CodeFunction {
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
    ins.push(abi::move_immediate(abi::mfb_arg(0), "Integer", "0"));
    call_external(from, "GetModuleHandleW", KERNEL32, &mut ins, &mut rel);
    ins.push(abi::store_u64(
        abi::c_return(0),
        abi::stack_pointer(),
        HINSTANCE,
    ));

    // headless = GetEnvironmentVariableW(L"MFB_WINAPP_HEADLESS", NULL, 0) != 0
    load_addr(abi::mfb_arg(0), HEADLESS_ENV_SYM, from, &mut ins, &mut rel);
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "0"));
    ins.push(abi::move_immediate(abi::mfb_arg(2), "Integer", "0"));
    call_external(
        from,
        "GetEnvironmentVariableW",
        KERNEL32,
        &mut ins,
        &mut rel,
    );
    ins.push(abi::compare_immediate(abi::c_return(0), "0"));
    ins.push(abi::branch_ne("headless_spawn"));

    // ---- GUI path: build + show the window (byte-equivalent to spike.rs) ----
    // Zero the 80-byte WNDCLASSEXW (10 qwords).
    for i in 0..10 {
        ins.push(abi::store_u64(
            abi::ZERO,
            abi::stack_pointer(),
            WNDCLASS + i * 8,
        ));
    }
    // cbSize = 80 (store_u64 → cbSize@0=80, style@4=0).
    ins.push(abi::move_immediate(abi::mfb_arg(0), "Integer", "80"));
    ins.push(abi::store_u64(
        abi::mfb_arg(0),
        abi::stack_pointer(),
        WNDCLASS,
    ));
    // lpfnWndProc = &WndProc (@+8).
    load_addr(abi::mfb_arg(0), WNDPROC_SYMBOL, from, &mut ins, &mut rel);
    ins.push(abi::store_u64(
        abi::mfb_arg(0),
        abi::stack_pointer(),
        WNDCLASS + 8,
    ));
    // hInstance (@+24).
    ins.push(abi::load_u64(
        abi::mfb_arg(0),
        abi::stack_pointer(),
        HINSTANCE,
    ));
    ins.push(abi::store_u64(
        abi::mfb_arg(0),
        abi::stack_pointer(),
        WNDCLASS + 24,
    ));
    // lpszClassName = &class (@+64).
    load_addr(abi::mfb_arg(0), CLASS_NAME_SYM, from, &mut ins, &mut rel);
    ins.push(abi::store_u64(
        abi::mfb_arg(0),
        abi::stack_pointer(),
        WNDCLASS + 64,
    ));

    // RegisterClassExW(&wndclass)
    ins.push(abi::add_immediate(
        abi::mfb_arg(0),
        abi::stack_pointer(),
        WNDCLASS,
    ));
    call_external(from, "RegisterClassExW", USER32, &mut ins, &mut rel);

    // CreateWindowExW(0, &class, &title, style, CW, CW, 400, 300, 0, 0, hInst, 0).
    // Stage the seven stack args (5th..11th) at [sp+0x20..0x58] through ARG[2] as a
    // caller-saved scratch BEFORE it becomes lpClassName's sibling — the SCRATCH
    // pool and ARG[4..] must NOT be used (their Win64 realizations are callee-saved,
    // and clobbering them corrupts the pinned/arena registers — see emit_open_file).
    ins.push(abi::move_immediate(
        abi::mfb_arg(2),
        "Integer",
        CW_USEDEFAULT,
    ));
    ins.push(abi::store_u64(abi::mfb_arg(2), abi::stack_pointer(), 0x20)); // x
    ins.push(abi::store_u64(abi::mfb_arg(2), abi::stack_pointer(), 0x28)); // y
    ins.push(abi::move_immediate(abi::mfb_arg(2), "Integer", "400"));
    ins.push(abi::store_u64(abi::mfb_arg(2), abi::stack_pointer(), 0x30)); // width
    ins.push(abi::move_immediate(abi::mfb_arg(2), "Integer", "300"));
    ins.push(abi::store_u64(abi::mfb_arg(2), abi::stack_pointer(), 0x38)); // height
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x40)); // hWndParent
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x48)); // hMenu
    ins.push(abi::load_u64(
        abi::mfb_arg(2),
        abi::stack_pointer(),
        HINSTANCE,
    ));
    ins.push(abi::store_u64(abi::mfb_arg(2), abi::stack_pointer(), 0x50)); // hInstance
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x58)); // lpParam
                                                                     // Register args, ARG[2] (lpWindowName = &title) set last.
    ins.push(abi::move_immediate(abi::mfb_arg(0), "Integer", "0"));
    load_addr(abi::mfb_arg(1), CLASS_NAME_SYM, from, &mut ins, &mut rel);
    ins.push(abi::move_immediate(
        abi::mfb_arg(3),
        "Integer",
        WS_OVERLAPPED_VISIBLE,
    ));
    load_addr(abi::mfb_arg(2), TITLE_SYM, from, &mut ins, &mut rel);
    call_external(from, "CreateWindowExW", USER32, &mut ins, &mut rel);
    ins.push(abi::store_u64(abi::c_return(0), abi::stack_pointer(), HWND));
    // Stash the main HWND so the worker's finish helper can signal the UI thread.
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), HWND));
    load_addr(abi::mfb_arg(1), MAIN_HWND_SYM, from, &mut ins, &mut rel);
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::mfb_arg(1), 0));

    // Transcript = a multiline EDIT child filling the window (the stock
    // control that IS the scrollback — the Win32 analog of macOS's NSTextView).
    // CreateWindowExW(0, L"EDIT", NULL, EDIT_STYLE, 0, 0, 400, 300, mainHwnd, 0,
    //                 hInstance, NULL). SendMessage cross-thread marshaling (the
    // worker → this UI thread) makes io::print append synchronously.
    // Stage the stack args through ARG[2] (set to lpWindowName=NULL last).
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x20)); // x = 0
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x28)); // y = 0
    ins.push(abi::move_immediate(abi::mfb_arg(2), "Integer", "400"));
    ins.push(abi::store_u64(abi::mfb_arg(2), abi::stack_pointer(), 0x30)); // width
    ins.push(abi::move_immediate(abi::mfb_arg(2), "Integer", "300"));
    ins.push(abi::store_u64(abi::mfb_arg(2), abi::stack_pointer(), 0x38)); // height
    ins.push(abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), HWND));
    ins.push(abi::store_u64(abi::mfb_arg(2), abi::stack_pointer(), 0x40)); // hWndParent = main
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x48)); // hMenu = 0 (child id)
    ins.push(abi::load_u64(
        abi::mfb_arg(2),
        abi::stack_pointer(),
        HINSTANCE,
    ));
    ins.push(abi::store_u64(abi::mfb_arg(2), abi::stack_pointer(), 0x50)); // hInstance
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x58)); // lpParam
    ins.push(abi::move_immediate(abi::mfb_arg(0), "Integer", "0"));
    load_addr(abi::mfb_arg(1), EDIT_CLASS_SYM, from, &mut ins, &mut rel);
    ins.push(abi::move_immediate(abi::mfb_arg(3), "Integer", EDIT_STYLE));
    ins.push(abi::move_immediate(abi::mfb_arg(2), "Integer", "0")); // lpWindowName = NULL (last)
    call_external(from, "CreateWindowExW", USER32, &mut ins, &mut rel);
    // Store the EDIT HWND into its writable global (load_addr writes ARG[1], not
    // the return register, so the fresh handle survives the address computation).
    load_addr(abi::mfb_arg(1), EDIT_HWND_SYM, from, &mut ins, &mut rel);
    ins.push(abi::store_u64(abi::c_return(0), abi::mfb_arg(1), 0));
    // plan-98-A Phase 3: the surviving copy. The reconcile zeroes EDIT_HWND_SYM
    // outside `Console` so `io::` writes degrade to the fd sink; without this second
    // global the control's handle would be lost on the first mode switch and
    // `Console` could never be restored.
    load_addr(
        abi::mfb_arg(1),
        EDIT_HWND_SAVED_SYM,
        from,
        &mut ins,
        &mut rel,
    );
    ins.push(abi::store_u64(abi::return_register(), abi::mfb_arg(1), 0));

    // ---- plan-66-J-4 input wiring (GUI path) ----
    // CreatePipe(&hRead, &hWrite, NULL, 0): a byte pipe whose READ end becomes the
    // worker's stdin (fd 0) and whose WRITE end the EDIT subclass feeds keystrokes.
    ins.push(abi::add_immediate(
        abi::mfb_arg(0),
        abi::stack_pointer(),
        PIPEREAD,
    )); // &hRead
    ins.push(abi::add_immediate(
        abi::mfb_arg(1),
        abi::stack_pointer(),
        PIPEWRITE,
    )); // &hWrite
    ins.push(abi::move_immediate(abi::mfb_arg(2), "Integer", "0")); // lpPipeAttributes = NULL
    ins.push(abi::move_immediate(abi::mfb_arg(3), "Integer", "0")); // nSize = 0 (default buffer)
    call_external(from, "CreatePipe", KERNEL32, &mut ins, &mut rel);
    // SetStdHandle(STD_INPUT_HANDLE = -10, hRead): the worker's io::readLine reads
    // fd 0, which win emit_read_file resolves via GetStdHandle(-10); redirecting it
    // to the pipe read end makes readLine drain window keystrokes (plan-66-J-4).
    ins.push(abi::move_immediate(abi::mfb_arg(0), "Integer", "0"));
    ins.push(abi::subtract_immediate(
        abi::mfb_arg(0),
        abi::mfb_arg(0),
        STD_INPUT_FD,
    )); // -10
    ins.push(abi::load_u64(
        abi::mfb_arg(1),
        abi::stack_pointer(),
        PIPEREAD,
    )); // hRead
    call_external(from, "SetStdHandle", KERNEL32, &mut ins, &mut rel);
    // Stash hWrite in its global so the EDIT subclass (UI thread) can commit bytes.
    ins.push(abi::load_u64(
        abi::mfb_arg(0),
        abi::stack_pointer(),
        PIPEWRITE,
    ));
    load_addr(abi::mfb_arg(1), STDIN_WRITE_SYM, from, &mut ins, &mut rel);
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::mfb_arg(1), 0));
    // Subclass the transcript EDIT: oldproc = SetWindowLongPtrW(edit, GWLP_WNDPROC =
    // -4, &editproc). editproc writes each WM_CHAR to the pipe then chains to
    // oldproc, so the stock EDIT behaviour (J-3 transcript appends) is preserved.
    load_addr(abi::mfb_arg(0), EDIT_HWND_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), 0)); // edit hwnd
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "0"));
    ins.push(abi::subtract_immediate(
        abi::mfb_arg(1),
        abi::mfb_arg(1),
        GWLP_WNDPROC,
    )); // -4
    load_addr(abi::mfb_arg(2), EDITPROC_SYMBOL, from, &mut ins, &mut rel); // &editproc
    call_external(from, "SetWindowLongPtrW", USER32, &mut ins, &mut rel);
    load_addr(abi::mfb_arg(1), EDIT_OLDPROC_SYM, from, &mut ins, &mut rel);
    ins.push(abi::store_u64(abi::c_return(0), abi::mfb_arg(1), 0)); // oldproc

    // CreateThread(NULL, 0, &worker, hwnd, 0, NULL)
    ins.push(abi::move_immediate(abi::mfb_arg(0), "Integer", "0"));
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "0"));
    load_addr(abi::mfb_arg(2), WORKER_SYMBOL, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::mfb_arg(3), abi::stack_pointer(), HWND));
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x20)); // dwCreationFlags
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x28)); // lpThreadId
    call_external(from, "CreateThread", KERNEL32, &mut ins, &mut rel);
    ins.push(abi::store_u64(
        abi::c_return(0),
        abi::stack_pointer(),
        WORKERH,
    ));

    // ---- plan-98-A Phase 3: honour the static initial presentation mode ----
    // A program that references `app::setMode` starts in `None` (the compiler's
    // static rule), so it must start windowless — otherwise a canvas program would
    // flash a transcript window before its first `setMode`. The window and its EDIT
    // are still fully built above: only their visibility differs, so a later
    // `setMode(Console)` re-shows an already-wired transcript, and the io routing
    // global starts cleared so writes degrade to the fd sink until then.
    if initial_mode == PresentationMode::None {
        load_addr(abi::mfb_arg(0), EDIT_HWND_SYM, from, &mut ins, &mut rel);
        ins.push(abi::store_u64(abi::ZERO, abi::mfb_arg(0), 0));
        ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), HWND));
        ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", SW_HIDE));
        call_external(from, "ShowWindow", USER32, &mut ins, &mut rel);
    }

    // ---- plan-66-J-4 keystroke injection (test affordance) ----
    // If MFB_WINAPP_INPUT is set, post each of its characters as a WM_CHAR to the
    // EDIT (then a final Enter), simulating typing so the subclass → pipe → readLine
    // round-trip is box-provable over ssh without a keyboard. The message loop below
    // dispatches these to editproc, which feeds the pipe on the UI thread.
    // n = GetEnvironmentVariableW(L"MFB_WINAPP_INPUT", inputbuf, 250)
    load_addr(abi::mfb_arg(0), INPUT_ENV_SYM, from, &mut ins, &mut rel);
    load_addr(abi::mfb_arg(1), INPUT_BUF_SYM, from, &mut ins, &mut rel);
    ins.push(abi::move_immediate(abi::mfb_arg(2), "Integer", "250"));
    call_external(
        from,
        "GetEnvironmentVariableW",
        KERNEL32,
        &mut ins,
        &mut rel,
    );
    ins.push(abi::store_u64(
        abi::return_register(),
        abi::stack_pointer(),
        INJ_N,
    )); // count
    ins.push(abi::compare_immediate(abi::return_register(), "0"));
    ins.push(abi::branch_eq("inject_enter")); // unset/empty → just send Enter? no — skip all
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), INJ_I)); // i = 0
    ins.push(abi::label("inject_loop"));
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), INJ_I));
    ins.push(abi::load_u64(abi::mfb_arg(1), abi::stack_pointer(), INJ_N));
    ins.push(abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(1)));
    ins.push(abi::branch_ge("inject_enter")); // i >= n → done, send Enter
                                              // ch = inputbuf[i] (a UTF-16 code unit); PostMessageW(edit, WM_CHAR, ch, 0).
    load_addr(abi::mfb_arg(1), INPUT_BUF_SYM, from, &mut ins, &mut rel);
    ins.push(abi::shift_left_immediate(
        abi::mfb_arg(0),
        abi::mfb_arg(0),
        1,
    )); // i*2
    ins.push(abi::add_registers(
        abi::mfb_arg(1),
        abi::mfb_arg(1),
        abi::mfb_arg(0),
    ));
    ins.push(abi::load_u16(abi::mfb_arg(2), abi::mfb_arg(1), 0)); // wParam = ch
    load_addr(abi::mfb_arg(0), EDIT_HWND_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), 0)); // edit hwnd
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", WM_CHAR));
    ins.push(abi::move_immediate(abi::mfb_arg(3), "Integer", "0")); // lParam
    call_external(from, "PostMessageW", USER32, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), INJ_I));
    ins.push(abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1));
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), INJ_I));
    ins.push(abi::branch("inject_loop"));
    ins.push(abi::label("inject_enter"));
    // A final Enter (WM_CHAR '\r') so readLine terminates the line.
    load_addr(abi::mfb_arg(0), INPUT_ENV_SYM, from, &mut ins, &mut rel);
    load_addr(abi::mfb_arg(1), INPUT_BUF_SYM, from, &mut ins, &mut rel);
    ins.push(abi::move_immediate(abi::mfb_arg(2), "Integer", "250"));
    call_external(
        from,
        "GetEnvironmentVariableW",
        KERNEL32,
        &mut ins,
        &mut rel,
    );
    ins.push(abi::compare_immediate(abi::c_return(0), "0"));
    ins.push(abi::branch_eq("inject_done")); // MFB_WINAPP_INPUT unset → no injection at all
    load_addr(abi::mfb_arg(0), EDIT_HWND_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), 0));
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", WM_CHAR));
    ins.push(abi::move_immediate(abi::mfb_arg(2), "Integer", VK_RETURN)); // '\r'
    ins.push(abi::move_immediate(abi::mfb_arg(3), "Integer", "0"));
    call_external(from, "PostMessageW", USER32, &mut ins, &mut rel);
    ins.push(abi::label("inject_done"));

    // Message loop.
    ins.push(abi::label("msg_loop"));
    ins.push(abi::add_immediate(
        abi::mfb_arg(0),
        abi::stack_pointer(),
        MSG,
    ));
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "0"));
    ins.push(abi::move_immediate(abi::mfb_arg(2), "Integer", "0"));
    ins.push(abi::move_immediate(abi::mfb_arg(3), "Integer", "0"));
    call_external(from, "GetMessageW", USER32, &mut ins, &mut rel);
    ins.push(abi::compare_immediate(abi::c_return(0), "0"));
    ins.push(abi::branch_le("main_done")); // 0 = WM_QUIT, -1 = error
                                           // The worker's finish posts WM_APP_QUIT (msg.message @ MSG+8); catch it and exit
                                           // the loop so the UI thread does teardown (a worker ExitProcess faults in GDI).
    ins.push(abi::load_u64(
        abi::mfb_arg(1),
        abi::stack_pointer(),
        MSG + 8,
    ));
    ins.push(abi::move_immediate(
        abi::mfb_arg(2),
        "Integer",
        "4294967295",
    ));
    ins.push(abi::and_registers(
        abi::mfb_arg(1),
        abi::mfb_arg(1),
        abi::mfb_arg(2),
    )); // low 32 = message
    ins.push(abi::compare_immediate(abi::mfb_arg(1), WM_APP_QUIT));
    ins.push(abi::branch_eq("main_done"));
    ins.push(abi::add_immediate(
        abi::mfb_arg(0),
        abi::stack_pointer(),
        MSG,
    ));
    call_external(from, "TranslateMessage", USER32, &mut ins, &mut rel);
    ins.push(abi::add_immediate(
        abi::mfb_arg(0),
        abi::stack_pointer(),
        MSG,
    ));
    call_external(from, "DispatchMessageW", USER32, &mut ins, &mut rel);
    ins.push(abi::branch("msg_loop"));

    // ---- headless path: spawn the worker and wait for it ----
    ins.push(abi::label("headless_spawn"));
    ins.push(abi::move_immediate(abi::mfb_arg(0), "Integer", "0"));
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "0"));
    load_addr(abi::mfb_arg(2), WORKER_SYMBOL, from, &mut ins, &mut rel);
    ins.push(abi::move_immediate(abi::mfb_arg(3), "Integer", "0")); // lpParameter = NULL
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x20));
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x28));
    call_external(from, "CreateThread", KERNEL32, &mut ins, &mut rel);
    ins.push(abi::store_u64(
        abi::c_return(0),
        abi::stack_pointer(),
        WORKERH,
    ));
    // ---- plan-98-F Phase 3: one scripted resize, when the env says so ----
    //
    // The Windows twin of the GTK backend's `emit_headless_scripted_resize`, and it
    // exists for the same reason: a resize is a *window* event, headless has no window,
    // and box 2230 is the only machine that can run this at all. Without it the WM_SIZE
    // arm would be implemented and never executed.
    //
    // It waits for a first completed frame before resizing, which is the whole point —
    // resizing before any frame exists would build the render target once at the new
    // size and prove nothing, where resizing after one forces the tear-down-and-rebuild
    // the Vulkan backend has for it. `MFB_CANVAS_DUMP` overwrites, so the file left
    // behind is the second frame and its length is `newWidth * newHeight * 4`.
    //
    // This runs on the main thread, which is the right one: publishing the size is the
    // main thread's job on all three backends. It calls the same publisher WM_SIZE
    // calls, so what runs here is the production path and not a stand-in for it.
    //
    // Gated on `uses_canvas`, and it has to be: this block names the graphics-state
    // symbol, and a program that never imports `canvas` does not emit that data
    // object — an ungated reference fails the build with "relocation target
    // `_mfb_rt_canvas_graphics` is not a data object or defined symbol".
    if uses_canvas {
        emit_parse_wide_env(
            RESIZE_W_ENV_SYM,
            abi::LOCAL[0],
            "hl_rw",
            from,
            &mut ins,
            &mut rel,
        );
        ins.push(abi::compare_immediate(abi::LOCAL[0], "0"));
        ins.push(abi::branch_le("headless_wait_worker"));
        emit_parse_wide_env(
            RESIZE_H_ENV_SYM,
            abi::LOCAL[1],
            "hl_rh",
            from,
            &mut ins,
            &mut rel,
        );
        ins.push(abi::compare_immediate(abi::LOCAL[1], "0"));
        ins.push(abi::branch_le("headless_wait_worker"));
        // Poll until the render loop has completed a frame at the original size.
        ins.push(abi::label("headless_resize_wait"));
        crate::codegen::runtime::canvas::state_base(from, abi::LOCAL[2], &mut ins, &mut rel);
        // **`SCRATCH[7]`, and not `SCRATCH[0]`.** The x86 scratch pool is
        // `POOL[(n - 9) % 11]` over `[rbx, rsi, rdi, r8, r9, r10, r11, r12, r13, rcx, rbp]`,
        // so `SCRATCH[0]` (x9) and `LOCAL[1]` (x20) are BOTH `rbx` — loading the frame
        // counter into `SCRATCH[0]` overwrote the parsed height, and since the loop only
        // exits once the counter is non-zero, the height published was the frame count.
        // It resized to 640x1 and dumped 2560 bytes. This is exactly the latent hazard
        // `src/arch/x86_64/select.rs` records above its POOL — two architecture-neutral
        // spellings that are distinct on AArch64 collapsing onto one x86 register.
        // `SCRATCH[7]` is `r12`, callee-saved under Win64 and clear of `LOCAL[0..2]`
        // (`rbp`/`rbx`/`rsi`).
        ins.push(abi::load_u64(
            abi::SCRATCH[7],
            abi::LOCAL[2],
            crate::codegen::runtime::canvas::GRAPHICS_OFFSET_FRAMES,
        ));
        ins.push(abi::compare_immediate(abi::SCRATCH[7], "0"));
        ins.push(abi::branch_ne("headless_resize_ready"));
        ins.push(abi::move_immediate(abi::mfb_arg(0), "Integer", "20"));
        call_external(from, "Sleep", KERNEL32, &mut ins, &mut rel);
        ins.push(abi::branch("headless_resize_wait"));

        ins.push(abi::label("headless_resize_ready"));
        crate::codegen::runtime::canvas::emit_publish_surface_size(
            from,
            abi::LOCAL[2],
            abi::LOCAL[0],
            abi::LOCAL[1],
            &mut ins,
            &mut rel,
        );
        call_internal(from, CANVAS_SIGNAL_REDRAW_SYMBOL, &mut ins, &mut rel);
    }

    ins.push(abi::label("headless_wait_worker"));
    // WaitForSingleObject(worker, INFINITE = 0xFFFFFFFF via 0 - 1).
    ins.push(abi::load_u64(
        abi::mfb_arg(0),
        abi::stack_pointer(),
        WORKERH,
    ));
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "0"));
    ins.push(abi::subtract_immediate(abi::mfb_arg(1), abi::mfb_arg(1), 1));
    call_external(from, "WaitForSingleObject", KERNEL32, &mut ins, &mut rel);

    // The loop ended (worker posted WM_APP_QUIT) or the headless worker exited.
    // Test affordance (plan-66-J-3 box proof): when MFB_WINAPP_DUMP is set, the UI
    // thread reads the transcript back (WM_GETTEXT) and writes the raw UTF-16 to
    // stdout, so an ssh box run can confirm io::print reached the window without a
    // visible display. Off by default (no side effect for real GUI runs).
    ins.push(abi::label("main_done"));
    load_addr(abi::mfb_arg(0), EDIT_HWND_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), 0));
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), 0x60));
    ins.push(abi::compare_immediate(abi::mfb_arg(0), "0"));
    ins.push(abi::branch_eq("main_exit"));
    load_addr(abi::mfb_arg(0), DUMP_ENV_SYM, from, &mut ins, &mut rel);
    load_addr(
        abi::mfb_arg(1),
        "_mfb_winapp_testbuf",
        from,
        &mut ins,
        &mut rel,
    );
    ins.push(abi::move_immediate(abi::mfb_arg(2), "Integer", "200"));
    call_external(
        from,
        "GetEnvironmentVariableW",
        KERNEL32,
        &mut ins,
        &mut rel,
    );
    ins.push(abi::compare_immediate(abi::c_return(0), "0"));
    ins.push(abi::branch_eq("main_exit"));
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), 0x60));
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "13")); // WM_GETTEXT
    ins.push(abi::move_immediate(abi::mfb_arg(2), "Integer", "250"));
    load_addr(
        abi::mfb_arg(3),
        "_mfb_winapp_testbuf",
        from,
        &mut ins,
        &mut rel,
    );
    call_external(from, "SendMessageW", USER32, &mut ins, &mut rel);
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "65535"));
    ins.push(abi::and_registers(
        abi::c_return(0),
        abi::c_return(0),
        abi::mfb_arg(1),
    ));
    ins.push(abi::shift_left_immediate(
        abi::mfb_arg(0),
        abi::c_return(0),
        1,
    ));
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), 0x68)); // nbytes
    ins.push(abi::move_immediate(abi::mfb_arg(0), "Integer", "0"));
    ins.push(abi::subtract_immediate(
        abi::mfb_arg(0),
        abi::mfb_arg(0),
        FILE_FLAG_STDOUT_FD,
    ));
    call_external(from, "GetStdHandle", KERNEL32, &mut ins, &mut rel);
    ins.push(abi::store_u64(abi::c_return(0), abi::stack_pointer(), 0x70));
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x78));
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x20));
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), 0x70));
    load_addr(
        abi::mfb_arg(1),
        "_mfb_winapp_testbuf",
        from,
        &mut ins,
        &mut rel,
    );
    ins.push(abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), 0x68));
    ins.push(abi::add_immediate(
        abi::mfb_arg(3),
        abi::stack_pointer(),
        0x78,
    ));
    call_external(from, "WriteFile", KERNEL32, &mut ins, &mut rel);
    ins.push(abi::label("main_exit"));
    ins.push(abi::move_immediate(abi::mfb_arg(0), "Integer", "0"));
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
    // 0x20, not 0x28 — and the difference is the whole of bug-478's second half.
    //
    // A Windows thread start routine is entered with `rsp` **already** 16-byte
    // aligned: `BaseThreadInitThunk` does not leave the ordinary `call`'s 8-byte
    // skew. A function reached by a normal `call` does, which is why an ordinary
    // prologue reserves an odd multiple of 8 and this one must not.
    //
    // The program body assumes `rsp % 16 == 0` throughout — every external-call
    // emitter in `win_x86_64/code.rs` reserves a multiple of 16 on top of it — and
    // `entry_stack_misaligned_on_entry` shaves the 8 the loader's own `call` leaves.
    // So the body's alignment is exactly this call site's, and reserving 0x28 here
    // handed the entire app-mode program a stack that was 8 bytes out for every
    // Win32 call it would ever make. Measured: the same emitter frame that ran clean
    // on the console path faulted on the worker, and vice versa.
    ins.push(abi::subtract_stack(0x20));
    // No kernel argc/argv on a worker stack; the program body captures os::args
    // (if used) via GetCommandLineW itself (plan-66-B). Pass 0/0.
    ins.push(abi::move_immediate(abi::mfb_arg(0), "Integer", "0"));
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "0"));
    call_internal(from, MACAPP_PROGRAM_SYMBOL, &mut ins, &mut rel);
    ins.push(abi::move_immediate(abi::mfb_arg(0), "Integer", "0"));
    call_external(from, "ExitThread", KERNEL32, &mut ins, &mut rel);
    ins.push(abi::add_stack(0x20));
    ins.push(abi::return_());
    code_function("winapp.worker", WORKER_SYMBOL, ins, rel)
}

/// Read the decimal value of wide environment variable `name_sym` into `dst`, or 0
/// when it is unset or does not start with a digit.
///
/// Emitted inline rather than as a callable symbol because it is wanted exactly twice,
/// in one function, and a symbol would need registering with the plan. `GetEnvironment-
/// VariableW` answers with the character count in the C result — 0 when absent — and
/// writes UTF-16 into the shared buffer, so the digits are two bytes apart.
fn emit_parse_wide_env(
    name_sym: &str,
    dst: &str,
    tag: &str,
    from: &str,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) {
    let loop_head = format!("{tag}_digit_head");
    let done = format!("{tag}_digit_done");
    load_addr(abi::mfb_arg(0), name_sym, from, ins, rel);
    load_addr(abi::mfb_arg(1), RESIZE_BUF_SYM, from, ins, rel);
    ins.push(abi::move_immediate(
        abi::mfb_arg(2),
        "Integer",
        &RESIZE_BUF_CHARS.to_string(),
    ));
    call_external(from, "GetEnvironmentVariableW", KERNEL32, ins, rel);
    ins.push(abi::move_immediate(dst, "Integer", "0"));
    ins.push(abi::compare_immediate(abi::c_return(0), "0"));
    ins.push(abi::branch_eq(&done)); // unset
    load_addr(abi::SCRATCH[6], RESIZE_BUF_SYM, from, ins, rel);
    ins.push(abi::label(&loop_head));
    ins.push(abi::load_u16(abi::SCRATCH[7], abi::SCRATCH[6], 0));
    ins.push(abi::compare_immediate(abi::SCRATCH[7], "48")); // '0'
    ins.push(abi::branch_lt(&done));
    ins.push(abi::compare_immediate(abi::SCRATCH[7], "57")); // '9'
    ins.push(abi::branch_gt(&done));
    ins.push(abi::move_immediate(abi::SCRATCH[8], "Integer", "10"));
    ins.push(abi::multiply_registers(dst, dst, abi::SCRATCH[8]));
    ins.push(abi::subtract_immediate(
        abi::SCRATCH[7],
        abi::SCRATCH[7],
        48,
    ));
    ins.push(abi::add_registers(dst, dst, abi::SCRATCH[7]));
    ins.push(abi::add_immediate(abi::SCRATCH[6], abi::SCRATCH[6], 2));
    ins.push(abi::branch(&loop_head));
    ins.push(abi::label(&done));
}

/// `WndProc(hwnd, msg, wParam, lParam)`: quit on `WM_DESTROY`, else default.
fn emit_wndproc(uses_canvas: bool) -> CodeFunction {
    // Frame (plan-66-J-5 added the WM_PAINT TUI present; plan-98-C Phase 3 the
    // canvas present): shadow[0..0x20], outgoing stack args [0x20..0x60] —
    // `SetDIBitsToDevice` has 8 stack args, the widest call here — saved
    // hwnd@0x60/msg@0x68/wParam@0x70/lParam@0x78, hdc@0x80,
    // PAINTSTRUCT@0x88..0xD0, BITMAPINFOHEADER@0xD0..0xF8, frame block@0xF8.
    // FRAME ≡ 8 (mod 16): entered at sp%16==8, so 0x108 realigns before any call.
    const FRAME: usize = 0x108;
    const H0: usize = 0x60; // hwnd
    const H1: usize = 0x68; // msg
    const H2: usize = 0x70; // wParam
    const H3: usize = 0x78; // lParam
    const HDC: usize = 0x80;
    const PS: usize = 0x88;
    /// A 40-byte `BITMAPINFOHEADER`, built per paint. Built rather than kept in
    /// static storage because it names the *frame's* width and height, which change
    /// with the surface — and a stale header would describe the new pixels with the
    /// old dimensions.
    const BMI: usize = 0xD0;
    /// The frame block, parked across the `BeginPaint` call that clobbers arg
    /// registers. Its own slot rather than a spare corner of `BMI`, which is exactly
    /// 40 bytes with no spare corner.
    const BLOCK: usize = 0xF8;
    let from = WNDPROC_SYMBOL;
    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel: Vec<CodeRelocation> = Vec::new();
    ins.push(abi::label("entry"));
    ins.push(abi::subtract_stack(FRAME));
    // Save the four WndProc args — the WM_PAINT path below clobbers ARG registers,
    // and the default DefWindowProcW tail needs them intact.
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), H0));
    ins.push(abi::store_u64(abi::mfb_arg(1), abi::stack_pointer(), H1));
    ins.push(abi::store_u64(abi::mfb_arg(2), abi::stack_pointer(), H2));
    ins.push(abi::store_u64(abi::mfb_arg(3), abi::stack_pointer(), H3));
    // WM_PAINT + a live TUI surface → BitBlt the off-screen grid to the client.
    ins.push(abi::compare_immediate(abi::mfb_arg(1), WM_PAINT));
    ins.push(abi::branch_ne("wnd_check_destroy"));
    // plan-98-A Phase 3 (Open Decision 3): never BitBlt the term grid over a canvas
    // surface. `term::on` traps in `Mode.Canvas`, but a program can call it in
    // `Console` — leaving `TUI_MEMDC_SYM` live — and *then* switch to `Canvas`. The
    // memDC outlives the switch, so without this gate every WM_PAINT would repaint
    // the stale character grid on top of the canvas client area. Gating the paint
    // (rather than destroying the memDC on mode exit) also keeps switching back to
    // `Console` cheap: the grid is still there, just not presented.
    load_addr(abi::mfb_arg(0), CANVAS_HWND_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), 0));
    ins.push(abi::compare_immediate(abi::mfb_arg(0), "0"));
    ins.push(abi::branch_ne("wnd_canvas_paint")); // canvas presented → paint the frame
    load_addr(abi::mfb_arg(0), TUI_MEMDC_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), 0));
    ins.push(abi::compare_immediate(abi::mfb_arg(0), "0"));
    ins.push(abi::branch_eq("wnd_default")); // no surface → normal paint
                                             // BeginPaint(hwnd, &ps) → hdc
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), H0));
    ins.push(abi::add_immediate(
        abi::mfb_arg(1),
        abi::stack_pointer(),
        PS,
    ));
    call_external(from, "BeginPaint", USER32, &mut ins, &mut rel);
    ins.push(abi::store_u64(abi::c_return(0), abi::stack_pointer(), HDC));
    // BitBlt(hdc, 0, 0, W, H, memDC, 0, 0, SRCCOPY) — args 5..9 on the stack.
    ins.push(abi::move_immediate(
        abi::mfb_arg(2),
        "Integer",
        &(TUI_ROWS * TUI_CELL_H).to_string(),
    ));
    ins.push(abi::store_u64(abi::mfb_arg(2), abi::stack_pointer(), 0x20)); // height (5th)
    load_addr(abi::mfb_arg(2), TUI_MEMDC_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::mfb_arg(2), abi::mfb_arg(2), 0));
    ins.push(abi::store_u64(abi::mfb_arg(2), abi::stack_pointer(), 0x28)); // hdcSrc (6th)
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x30)); // xSrc (7th)
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x38)); // ySrc (8th)
    ins.push(abi::move_immediate(abi::mfb_arg(2), "Integer", SRCCOPY));
    ins.push(abi::store_u64(abi::mfb_arg(2), abi::stack_pointer(), 0x40)); // rop (9th)
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), HDC)); // hdcDest
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "0")); // xDest
    ins.push(abi::move_immediate(abi::mfb_arg(2), "Integer", "0")); // yDest
    ins.push(abi::move_immediate(
        abi::mfb_arg(3),
        "Integer",
        &(TUI_COLS * TUI_CELL_W).to_string(),
    )); // width
    call_external(from, "BitBlt", GDI32, &mut ins, &mut rel);
    // EndPaint(hwnd, &ps)
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), H0));
    ins.push(abi::add_immediate(
        abi::mfb_arg(1),
        abi::stack_pointer(),
        PS,
    ));
    call_external(from, "EndPaint", USER32, &mut ins, &mut rel);
    // A WNDPROC is a **C callback**: Windows reads its `LRESULT` from `rax`
    // (`%retC`), not from the aligned MFB bank. The `DefWindowProcW` tail below
    // gets this right by accident — it leaves `rax` untouched — but every handled
    // arm was writing 0 into `rcx` and returning whatever the arm's last Win32
    // call had left in `rax`.
    ins.push(abi::move_immediate(abi::c_return(0), "Integer", "0"));
    ins.push(abi::add_stack(FRAME));
    ins.push(abi::return_());
    // ---- plan-98-C Phase 3: WM_PAINT while a canvas surface is presented ----
    //
    // `SetDIBitsToDevice` rather than a `CreateDIBSection` + `BitBlt` pair like the
    // term grid uses: the term surface is a persistent off-screen bitmap the app
    // draws into incrementally, while a canvas frame arrives complete and is drawn
    // once. A DIB section would add an allocation, a `SelectObject` and a lifetime
    // to manage, all to hold pixels that are already in memory.
    //
    // The frame's own width and height drive the call — not the client rect — so a
    // resize that has not yet produced a new frame draws the old one at its real
    // size rather than reading past the end of its block. Scaling to the client area
    // is plan-98-D's, with the resize handshake.
    ins.push(abi::label("wnd_canvas_paint"));
    load_addr(abi::mfb_arg(0), CANVAS_FRAME_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), 0));
    ins.push(abi::compare_immediate(abi::mfb_arg(0), "0"));
    ins.push(abi::branch_eq("wnd_default")); // nothing presented yet
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), BLOCK));
    // BeginPaint(hwnd, &ps) → hdc
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), H0));
    ins.push(abi::add_immediate(
        abi::mfb_arg(1),
        abi::stack_pointer(),
        PS,
    ));
    call_external(from, "BeginPaint", USER32, &mut ins, &mut rel);
    ins.push(abi::store_u64(abi::c_return(0), abi::stack_pointer(), HDC));
    // Build the BITMAPINFOHEADER: 40-byte size, the frame's width, a NEGATIVE
    // height (top-down rows — the rasteriser's row 0 is the top, and a DIB's
    // default bottom-up order would render the picture upside down), one plane,
    // 32 bits, BI_RGB, and zeros for the rest.
    ins.push(abi::load_u64(abi::mfb_arg(3), abi::stack_pointer(), BLOCK)); // frame block
    ins.push(abi::move_immediate(abi::mfb_arg(0), "Integer", "40"));
    ins.push(abi::store_u32(abi::mfb_arg(0), abi::stack_pointer(), BMI));
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(3), 0)); // width
    ins.push(abi::store_u32(
        abi::mfb_arg(0),
        abi::stack_pointer(),
        BMI + 4,
    ));
    ins.push(abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(3), 8)); // height
    ins.push(abi::subtract_registers(
        abi::mfb_arg(2),
        abi::ZERO,
        abi::mfb_arg(1),
    )); // -height
    ins.push(abi::store_u32(
        abi::mfb_arg(2),
        abi::stack_pointer(),
        BMI + 8,
    ));
    ins.push(abi::move_immediate(abi::mfb_arg(2), "Integer", "1"));
    ins.push(abi::store_u16(
        abi::mfb_arg(2),
        abi::stack_pointer(),
        BMI + 12,
    )); // planes
    ins.push(abi::move_immediate(abi::mfb_arg(2), "Integer", "32"));
    ins.push(abi::store_u16(
        abi::mfb_arg(2),
        abi::stack_pointer(),
        BMI + 14,
    )); // bit count
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), BMI + 16));
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), BMI + 24));
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), BMI + 32));
    // SetDIBitsToDevice(hdc, 0, 0, w, h, 0, 0, 0, h, bits, &bmi, DIB_RGB_COLORS)
    // — four in registers, eight on the stack from +0x20.
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(3), 8)); // height (5th)
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), 0x20));
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x28)); // xSrc (6th)
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x30)); // ySrc (7th)
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x38)); // uStartScan (8th)
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), 0x40)); // cLines (9th)
    ins.push(abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(3), 16)); // bits (10th)
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), 0x48));
    ins.push(abi::add_immediate(
        abi::mfb_arg(0),
        abi::stack_pointer(),
        BMI,
    )); // &bmi (11th)
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), 0x50));
    ins.push(abi::move_immediate(
        abi::mfb_arg(0),
        "Integer",
        DIB_RGB_COLORS,
    ));
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), 0x58)); // iUsage (12th)
    ins.push(abi::load_u64(abi::mfb_arg(3), abi::stack_pointer(), BLOCK));
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), HDC)); // hdc
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "0")); // xDest
    ins.push(abi::move_immediate(abi::mfb_arg(2), "Integer", "0")); // yDest
    ins.push(abi::load_u64(abi::mfb_arg(3), abi::mfb_arg(3), 0)); // width (4th)
    call_external(from, "SetDIBitsToDevice", GDI32, &mut ins, &mut rel);
    // EndPaint(hwnd, &ps)
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), H0));
    ins.push(abi::add_immediate(
        abi::mfb_arg(1),
        abi::stack_pointer(),
        PS,
    ));
    call_external(from, "EndPaint", USER32, &mut ins, &mut rel);
    ins.push(abi::move_immediate(abi::c_return(0), "Integer", "0"));
    ins.push(abi::add_stack(FRAME));
    ins.push(abi::return_());

    ins.push(abi::label("wnd_check_destroy"));
    ins.push(abi::compare_immediate(abi::mfb_arg(1), WM_DESTROY));
    ins.push(abi::branch_ne("wnd_check_size"));
    ins.push(abi::move_immediate(abi::mfb_arg(0), "Integer", "0"));
    call_external(from, "PostQuitMessage", USER32, &mut ins, &mut rel);
    ins.push(abi::move_immediate(abi::c_return(0), "Integer", "0"));
    ins.push(abi::add_stack(FRAME));
    ins.push(abi::return_());

    // ---- plan-98-F Phase 3: WM_SIZE publishes the new surface size ----
    //
    // Windows had no caller of `emit_publish_surface_size` at all, so the graphics
    // thread never learned that the window had changed size: it kept rendering at the
    // startup 900x640 whatever the user dragged the frame to. macOS publishes from
    // `setFrameSize:` and GTK from its `notify::default-width` handler; this is the
    // third.
    //
    // Gated on a live canvas surface, so in `Console`/`None` the message chains to
    // `DefWindowProcW` exactly as before and nothing about the TUI path moves.
    //
    // `lParam`'s low word is the client width and its high word the client height —
    // already the client area, so no `GetClientRect` and no frame arithmetic. Both are
    // masked to 16 bits; `emit_publish_surface_size` itself ignores a repeat of the
    // size it already holds, which matters because Windows sends `WM_SIZE` for moves
    // and restores that change nothing.
    //
    // Arg registers are free to clobber here: every exit from this arm goes to
    // `wnd_default`, which reloads all four from H0..H3.
    ins.push(abi::label("wnd_check_size"));
    // Gated on `uses_canvas` for the same reason the scripted resize in `_main` is:
    // `emit_publish_surface_size` names the graphics-state data object, which a
    // program that never imports `canvas` does not emit. The arm is inert anyway in
    // that case — `CANVAS_HWND_SYM` would be 0 — so what the gate removes is a dead
    // reference, not behaviour.
    if uses_canvas {
        ins.push(abi::compare_immediate(abi::mfb_arg(1), WM_SIZE));
        ins.push(abi::branch_ne("wnd_check_char"));
        load_addr(abi::SCRATCH[0], CANVAS_HWND_SYM, from, &mut ins, &mut rel);
        ins.push(abi::load_u64(abi::SCRATCH[0], abi::SCRATCH[0], 0));
        ins.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
        ins.push(abi::branch_eq("wnd_default")); // not in canvas mode → unchanged
        ins.push(abi::load_u64(abi::SCRATCH[1], abi::stack_pointer(), H3));
        ins.push(abi::move_immediate(abi::SCRATCH[3], "Integer", "65535"));
        ins.push(abi::and_registers(
            abi::SCRATCH[2],
            abi::SCRATCH[1],
            abi::SCRATCH[3],
        )); // width  = LOWORD(lParam)
        ins.push(abi::shift_right_immediate(
            abi::SCRATCH[1],
            abi::SCRATCH[1],
            16,
        ));
        ins.push(abi::and_registers(
            abi::SCRATCH[1],
            abi::SCRATCH[1],
            abi::SCRATCH[3],
        )); // height = HIWORD(lParam)
            // A minimised window reports 0x0. Publishing that would build a zero-sized render
            // target; skip it and keep the last real size, which is what the other two
            // backends' `> 0` guards do.
        ins.push(abi::compare_immediate(abi::SCRATCH[2], "0"));
        ins.push(abi::branch_le("wnd_default"));
        ins.push(abi::compare_immediate(abi::SCRATCH[1], "0"));
        ins.push(abi::branch_le("wnd_default"));
        crate::codegen::runtime::canvas::emit_publish_surface_size(
            from,
            abi::SCRATCH[0],
            abi::SCRATCH[2],
            abi::SCRATCH[1],
            &mut ins,
            &mut rel,
        );
        ins.push(abi::branch("wnd_default"));
    }

    // ---- plan-98-A Phase 4: WM_CHAR while a canvas surface is presented ----
    //
    // In `Console` the transcript EDIT has focus and its subclass (`editproc`) feeds
    // the input pipe. Canvas mode hides that EDIT, so focus falls to the top-level
    // window and its WM_CHAR arrives here instead — without this arm a canvas
    // program's `io::readByte` would block forever on a pipe nobody writes to.
    //
    // Gated on `CANVAS_HWND_SYM` being non-zero rather than on the message alone, so
    // in `Console`/`None` this is inert and the message chains to DefWindowProcW
    // exactly as before.
    //
    // Same byte contract as `editproc`: Enter (`\r`) is translated to `\n` so
    // `io::readLine` terminates the line, everything else is the low byte verbatim.
    // No echo — a canvas has no text surface to echo into, and the program draws its
    // own UI.
    //
    // The byte and the WriteFile `written` out-param borrow the `HDC` and `PS`
    // frame slots: WM_PAINT and WM_CHAR are different messages, so one invocation
    // never uses both, and reusing them keeps the frame size (and its `≡ 8 mod 16`
    // property) unchanged.
    ins.push(abi::label("wnd_check_char"));
    ins.push(abi::compare_immediate(abi::mfb_arg(1), WM_CHAR));
    ins.push(abi::branch_ne("wnd_check_blit"));
    load_addr(abi::mfb_arg(0), CANVAS_HWND_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), 0));
    ins.push(abi::compare_immediate(abi::mfb_arg(0), "0"));
    ins.push(abi::branch_eq("wnd_default")); // not in canvas mode → unchanged
    ins.push(abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), H2)); // wParam
    ins.push(abi::compare_immediate(abi::mfb_arg(2), VK_RETURN));
    ins.push(abi::branch_ne("wnd_char_not_cr"));
    ins.push(abi::move_immediate(abi::mfb_arg(0), "Integer", "10")); // '\n'
    ins.push(abi::branch("wnd_char_store"));
    ins.push(abi::label("wnd_char_not_cr"));
    ins.push(abi::move_immediate(abi::mfb_arg(3), "Integer", "255"));
    ins.push(abi::and_registers(
        abi::mfb_arg(0),
        abi::mfb_arg(2),
        abi::mfb_arg(3),
    )); // low byte
    ins.push(abi::label("wnd_char_store"));
    ins.push(abi::store_u8(abi::mfb_arg(0), abi::stack_pointer(), HDC)); // byte scratch
    load_addr(abi::mfb_arg(0), STDIN_WRITE_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), 0));
    ins.push(abi::compare_immediate(abi::mfb_arg(0), "0"));
    ins.push(abi::branch_eq("wnd_default")); // pipe never wired (headless)
                                             // WriteFile(hWrite, &byte, 1, &written, NULL)
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x20)); // 5th arg NULL
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), PS)); // written
    ins.push(abi::add_immediate(
        abi::mfb_arg(1),
        abi::stack_pointer(),
        HDC,
    ));
    ins.push(abi::move_immediate(abi::mfb_arg(2), "Integer", "1"));
    ins.push(abi::add_immediate(
        abi::mfb_arg(3),
        abi::stack_pointer(),
        PS,
    ));
    call_external(from, "WriteFile", KERNEL32, &mut ins, &mut rel);
    ins.push(abi::move_immediate(abi::c_return(0), "Integer", "0"));
    ins.push(abi::add_stack(FRAME));
    ins.push(abi::return_());

    // ---- plan-98-A Phase 3: WM_APP_RECONCILE (wParam = the new Mode) ----
    //
    // Runs on the UI thread, which owns the window, because the worker sent this
    // with `SendMessageW` and is blocked until it returns. Three-way dispatch:
    //
    //   Console (0): route io back to the transcript, show the EDIT and the window.
    //   None    (1): unroute io (writes degrade to the fd sink), hide the window.
    //   Canvas  (2): unroute io, hide the EDIT so the client area is bare, show the
    //                window, and publish the HWND as the canvas surface handle.
    //
    // On Windows the HWND *is* the native surface handle (`VK_KHR_win32_surface`
    // takes one), so there is nothing to create: baring the client area and
    // publishing the handle is the whole surface build, and clearing the handle is
    // the whole teardown.
    // ---- plan-98-C Phase 3: a rendered frame arrives ----
    //
    // `wParam` is a block the worker allocated and no longer owns. Freeing the
    // previous one and publishing the new one both happen here, on the UI thread,
    // which is what lets `WM_PAINT` read `CANVAS_FRAME_SYM` without a lock.
    //
    // `InvalidateRect` with `bErase = FALSE`: the frame covers its whole area
    // opaquely, so erasing to the class background first would only produce a flash.
    ins.push(abi::label("wnd_check_blit"));
    ins.push(abi::compare_immediate(abi::mfb_arg(1), WM_APP_BLIT));
    ins.push(abi::branch_ne("wnd_check_reconcile"));
    load_addr(abi::mfb_arg(3), CANVAS_FRAME_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(3), 0)); // previous block
    ins.push(abi::compare_immediate(abi::mfb_arg(0), "0"));
    ins.push(abi::branch_eq("wnd_blit_no_previous"));
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), BLOCK));
    call_external(from, "GetProcessHeap", KERNEL32, &mut ins, &mut rel);
    ins.push(abi::move_register(abi::mfb_arg(0), abi::c_return(0)));
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "0")); // flags
    ins.push(abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), BLOCK));
    call_external(from, "HeapFree", KERNEL32, &mut ins, &mut rel);
    ins.push(abi::label("wnd_blit_no_previous"));
    load_addr(abi::mfb_arg(3), CANVAS_FRAME_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), H2)); // wParam
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::mfb_arg(3), 0));
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), H0)); // hwnd
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "0")); // whole client
    ins.push(abi::move_immediate(abi::mfb_arg(2), "Integer", "0")); // bErase FALSE
    call_external(from, "InvalidateRect", USER32, &mut ins, &mut rel);
    ins.push(abi::move_immediate(abi::c_return(0), "Integer", "0"));
    ins.push(abi::add_stack(FRAME));
    ins.push(abi::return_());

    ins.push(abi::label("wnd_check_reconcile"));
    ins.push(abi::compare_immediate(abi::mfb_arg(1), WM_APP_RECONCILE));
    ins.push(abi::branch_ne("wnd_default"));
    ins.push(abi::compare_immediate(abi::mfb_arg(2), "2"));
    ins.push(abi::branch_eq("wnd_reconcile_canvas"));
    ins.push(abi::compare_immediate(abi::mfb_arg(2), "0"));
    ins.push(abi::branch_ne("wnd_reconcile_none"));

    // --- Console: re-route io to the transcript, show the EDIT, show the window ---
    // EDIT_HWND_SYM = EDIT_HWND_SAVED_SYM (the io write helper's routing test).
    load_addr(
        abi::mfb_arg(0),
        EDIT_HWND_SAVED_SYM,
        from,
        &mut ins,
        &mut rel,
    );
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), 0));
    load_addr(abi::mfb_arg(1), EDIT_HWND_SYM, from, &mut ins, &mut rel);
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::mfb_arg(1), 0));
    // The canvas teardown: clear the published handle and re-show the EDIT. Safe to
    // run unconditionally — the mode being *left* is recorded nowhere, so a
    // conditional would have to invent that state, and ShowWindow on an already
    // visible control is a no-op.
    load_addr(abi::mfb_arg(0), CANVAS_HWND_SYM, from, &mut ins, &mut rel);
    ins.push(abi::store_u64(abi::ZERO, abi::mfb_arg(0), 0));
    load_addr(
        abi::mfb_arg(0),
        EDIT_HWND_SAVED_SYM,
        from,
        &mut ins,
        &mut rel,
    );
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), 0));
    ins.push(abi::compare_immediate(abi::mfb_arg(0), "0"));
    ins.push(abi::branch_eq("wnd_reconcile_show"));
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", SW_SHOW));
    call_external(from, "ShowWindow", USER32, &mut ins, &mut rel);
    ins.push(abi::label("wnd_reconcile_show"));
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), H0));
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", SW_SHOW));
    call_external(from, "ShowWindow", USER32, &mut ins, &mut rel);
    ins.push(abi::branch("wnd_reconcile_done"));

    // --- Canvas: bare client area, window shown, HWND published ---
    ins.push(abi::label("wnd_reconcile_canvas"));
    // Unroute io: canvas mode has no transcript, so writes degrade to the fd sink.
    load_addr(abi::mfb_arg(0), EDIT_HWND_SYM, from, &mut ins, &mut rel);
    ins.push(abi::store_u64(abi::ZERO, abi::mfb_arg(0), 0));
    // Hide the transcript EDIT so the client area is the canvas surface.
    load_addr(
        abi::mfb_arg(0),
        EDIT_HWND_SAVED_SYM,
        from,
        &mut ins,
        &mut rel,
    );
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), 0));
    ins.push(abi::compare_immediate(abi::mfb_arg(0), "0"));
    ins.push(abi::branch_eq("wnd_reconcile_canvas_show"));
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", SW_HIDE));
    call_external(from, "ShowWindow", USER32, &mut ins, &mut rel);
    ins.push(abi::label("wnd_reconcile_canvas_show"));
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), H0));
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", SW_SHOW));
    call_external(from, "ShowWindow", USER32, &mut ins, &mut rel);
    // Publish the HWND *after* showing it: a hidden window is still a valid
    // `VK_KHR_win32_surface` target, but publishing only once presented keeps the
    // handle's meaning "a canvas surface is currently on screen".
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), H0));
    load_addr(abi::mfb_arg(1), CANVAS_HWND_SYM, from, &mut ins, &mut rel);
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::mfb_arg(1), 0));
    ins.push(abi::branch("wnd_reconcile_done"));

    // --- None: unroute io, tear the canvas down, hide the window ---
    ins.push(abi::label("wnd_reconcile_none"));
    load_addr(abi::mfb_arg(0), EDIT_HWND_SYM, from, &mut ins, &mut rel);
    ins.push(abi::store_u64(abi::ZERO, abi::mfb_arg(0), 0));
    load_addr(abi::mfb_arg(0), CANVAS_HWND_SYM, from, &mut ins, &mut rel);
    ins.push(abi::store_u64(abi::ZERO, abi::mfb_arg(0), 0));
    load_addr(
        abi::mfb_arg(0),
        EDIT_HWND_SAVED_SYM,
        from,
        &mut ins,
        &mut rel,
    );
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), 0));
    ins.push(abi::compare_immediate(abi::mfb_arg(0), "0"));
    ins.push(abi::branch_eq("wnd_reconcile_hide"));
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", SW_SHOW));
    call_external(from, "ShowWindow", USER32, &mut ins, &mut rel);
    ins.push(abi::label("wnd_reconcile_hide"));
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), H0));
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", SW_HIDE));
    call_external(from, "ShowWindow", USER32, &mut ins, &mut rel);

    ins.push(abi::label("wnd_reconcile_done"));
    ins.push(abi::move_immediate(abi::c_return(0), "Integer", "0"));
    ins.push(abi::add_stack(FRAME));
    ins.push(abi::return_());
    // default: DefWindowProcW(hwnd, msg, wParam, lParam) — reload the saved args.
    ins.push(abi::label("wnd_default"));
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), H0));
    ins.push(abi::load_u64(abi::mfb_arg(1), abi::stack_pointer(), H1));
    ins.push(abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), H2));
    ins.push(abi::load_u64(abi::mfb_arg(3), abi::stack_pointer(), H3));
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
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), HWND));
    ins.push(abi::store_u64(abi::mfb_arg(1), abi::stack_pointer(), MSG));
    ins.push(abi::store_u64(
        abi::mfb_arg(2),
        abi::stack_pointer(),
        WPARAM,
    ));
    ins.push(abi::store_u64(
        abi::mfb_arg(3),
        abi::stack_pointer(),
        LPARAM,
    ));
    // Only WM_CHAR feeds the pipe; everything else chains straight through.
    ins.push(abi::compare_immediate(abi::mfb_arg(1), WM_CHAR));
    ins.push(abi::branch_ne("chain"));
    // byte = (wParam == '\r') ? '\n' : (wParam & 0xFF). readLine terminates on '\n'.
    ins.push(abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), WPARAM));
    ins.push(abi::compare_immediate(abi::mfb_arg(2), VK_RETURN));
    ins.push(abi::branch_ne("not_cr"));
    ins.push(abi::move_immediate(abi::mfb_arg(0), "Integer", "10")); // '\n'
    ins.push(abi::branch("store_byte"));
    ins.push(abi::label("not_cr"));
    ins.push(abi::move_immediate(abi::mfb_arg(3), "Integer", "255"));
    ins.push(abi::and_registers(
        abi::mfb_arg(0),
        abi::mfb_arg(2),
        abi::mfb_arg(3),
    )); // low byte
    ins.push(abi::label("store_byte"));
    ins.push(abi::store_u8(
        abi::mfb_arg(0),
        abi::stack_pointer(),
        BYTEBUF,
    ));
    // hWrite = *_mfb_winapp_stdin_write; skip if the pipe was never wired (headless).
    load_addr(abi::mfb_arg(0), STDIN_WRITE_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), 0));
    ins.push(abi::compare_immediate(abi::mfb_arg(0), "0"));
    ins.push(abi::branch_eq("chain"));
    // WriteFile(hWrite, &byte, 1, &written, NULL)
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), OVERLAPPED)); // 5th arg NULL
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), WRITTEN));
    ins.push(abi::add_immediate(
        abi::mfb_arg(1),
        abi::stack_pointer(),
        BYTEBUF,
    )); // &byte
    ins.push(abi::move_immediate(abi::mfb_arg(2), "Integer", "1"));
    ins.push(abi::add_immediate(
        abi::mfb_arg(3),
        abi::stack_pointer(),
        WRITTEN,
    )); // &written
    call_external(from, "WriteFile", KERNEL32, &mut ins, &mut rel);
    // chain: CallWindowProcW(oldproc, hwnd, msg, wParam, lParam) — 5th arg on stack.
    ins.push(abi::label("chain"));
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), LPARAM));
    ins.push(abi::store_u64(
        abi::mfb_arg(0),
        abi::stack_pointer(),
        OVERLAPPED,
    )); // lParam (5th)
    load_addr(abi::mfb_arg(0), EDIT_OLDPROC_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), 0)); // oldproc (rcx)
    ins.push(abi::load_u64(abi::mfb_arg(1), abi::stack_pointer(), HWND)); // rdx
    ins.push(abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), MSG)); // r8
    ins.push(abi::load_u64(abi::mfb_arg(3), abi::stack_pointer(), WPARAM)); // r9
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
    load_addr(abi::mfb_arg(0), MAIN_HWND_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), 0));
    // Headless has no window, and `PostMessageW(NULL, ...)` does not no-op: it posts
    // to the *calling* thread's queue, creating one on a worker that is about to
    // exit. Skip it — there is nothing to ask to quit.
    ins.push(abi::compare_immediate(abi::mfb_arg(0), "0"));
    ins.push(abi::branch_eq("finish_no_window"));
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", WM_APP_QUIT));
    ins.push(abi::move_immediate(abi::mfb_arg(2), "Integer", "0"));
    ins.push(abi::move_immediate(abi::mfb_arg(3), "Integer", "0"));
    call_external(from, "PostMessageW", USER32, &mut ins, &mut rel);
    ins.push(abi::label("finish_no_window"));
    ins.push(abi::move_immediate(abi::mfb_arg(0), "Integer", "0"));
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
pub(super) fn emit_app_io_write(
    symbol: &str,
    stderr: bool,
    newline: bool,
    term_state_offset: Option<usize>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    // Append shape (plan-101): no own frame. The `abi_function` vreg finalizer
    // builds the frame, reserves the Win64 shadow space + call padding, keeps the
    // stack 16-aligned at every call site, and reserves the local scratch the
    // member requested (`APP_WRITE_SCRATCH`, sized for the slots below). Local
    // scratch is addressed at `sp+<slot>`; outgoing 5th/6th Win64 stack arguments
    // go through the `outgoing_stack_arg_store` sentinel, which the finalizer sizes
    // and resolves to the frame bottom above the shadow — the body no longer
    // hardcodes `sp+0x20`/`sp+0x28`. The lowest slot is `NL_BYTE = 0x30`, leaving
    // `[0,0x30)` unused in the reserved region (harmless).
    const NL_BYTE: usize = 0x30;
    const STR: usize = 0x38;
    const WRITTEN: usize = 0x40;
    const HANDLE: usize = 0x48;
    const EDITH: usize = 0x50; // transcript EDIT HWND
    const WBUF: usize = 0x58; // arena UTF-16 buffer
                              // plan-66-J-5 TUI grid path slots.
    const GI: usize = 0x60; // per-unit loop index (UTF-16 units)
    const GMEMDC: usize = 0x70; // cached memory DC
                                // plan-70-F TUI decode slots.
    const WCCOUNT: usize = 0x78; // UTF-16 unit count from MultiByteToWideChar
    const CPSLOT: usize = 0x80; // decoded codepoint (astral-combined)
    const UCOUNT: usize = 0x88; // UTF-16 units this cluster advances (1 BMP / 2 astral)
    const WIDTHSLOT: usize = 0x90; // display width (1 or 2)
    let std_fd = if stderr {
        FILE_FLAG_STDERR_FD
    } else {
        FILE_FLAG_STDOUT_FD
    };
    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel: Vec<CodeRelocation> = Vec::new();
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), STR));
    // plan-66-J-5: while TUI mode is active, render into the GDI grid instead of the
    // transcript EDIT (the grid is what the window shows in TUI mode).
    if let Some(tso) = term_state_offset {
        ins.push(abi::load_u64(
            abi::mfb_arg(0),
            ARENA_STATE_REGISTER,
            tso + TERM_STATE_ACTIVE_OFFSET,
        ));
        ins.push(abi::compare_immediate(abi::mfb_arg(0), "0"));
        ins.push(abi::branch_ne("term_grid_path"));
    }
    // If a transcript EDIT control is attached (non-headless), route there; the
    // SendMessageW below marshals to the UI thread synchronously. Else fall through
    // to the inherited standard handle (headless / no window — the J-2 path).
    load_addr(abi::mfb_arg(0), EDIT_HWND_SYM, symbol, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), 0));
    ins.push(abi::compare_immediate(abi::mfb_arg(0), "0"));
    ins.push(abi::branch_eq("std_path"));
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), EDITH));

    // --- transcript path: append to the EDIT control ---
    // wbuf = arena UTF-16 scratch.
    arena_alloc("65536", symbol, &mut ins, &mut rel);
    ins.push(abi::store_u64(
        abi::mfb_return(1),
        abi::stack_pointer(),
        WBUF,
    ));
    // MultiByteToWideChar(CP_UTF8, 0, str+8, len, wbuf, 32767). Stage the two stack
    // args (5th/6th) through ARG[2] before it becomes lpMultiByteStr (the SCRATCH
    // pool must not be used on Win64 — see emit_marshal_path).
    ins.push(abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), WBUF));
    ins.push(abi::outgoing_stack_arg_store(abi::mfb_arg(2), 0)); // lpWideCharStr (5th)
    ins.push(abi::move_immediate(abi::mfb_arg(2), "Integer", "32767"));
    ins.push(abi::outgoing_stack_arg_store(abi::mfb_arg(2), 1)); // cchWideChar (6th)
    ins.push(abi::move_immediate(abi::mfb_arg(0), "Integer", CP_UTF8));
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "0"));
    ins.push(abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), STR)); // str ptr
    ins.push(abi::load_u64(abi::mfb_arg(3), abi::mfb_arg(2), 0)); // cbMultiByte = len
    ins.push(abi::add_immediate(abi::mfb_arg(2), abi::mfb_arg(2), 8)); // lpMultiByteStr = str+8
    call_external(symbol, "MultiByteToWideChar", KERNEL32, &mut ins, &mut rel);
    // NUL-terminate wbuf at the TRUE converted length. MultiByteToWideChar returns
    // the wchar count written (≤ 32767, since cchWideChar=32767) in %ret0. The old
    // code instead offset by the untrusted UTF-8 byte length `str[0]*2` as a "safe
    // upper bound", but str[0]*2 exceeds the 65536-byte wbuf for any print ≥ 32768
    // bytes, storing the NUL past the arena block and corrupting adjacent data
    // (bug-418). Mask the `int` return's garbage high bits (the SIGSEGV the
    // byte-length hack originally dodged — same fix the WM_GETTEXTLENGTH return
    // below uses), clamp to ≤ 32767, then use that wchar count. A failed conversion
    // returns 0 → NUL at wbuf[0], which is safe.
    ins.push(abi::move_immediate(
        abi::mfb_arg(1),
        "Integer",
        "4294967295",
    ));
    ins.push(abi::and_registers(
        abi::mfb_arg(0),
        abi::return_register(),
        abi::mfb_arg(1),
    )); // low 32 bits
    ins.push(abi::compare_immediate(abi::mfb_arg(0), "32767"));
    ins.push(abi::branch_le("nul_len_ok"));
    ins.push(abi::move_immediate(abi::mfb_arg(0), "Integer", "32767")); // clamp to wbuf capacity
    ins.push(abi::label("nul_len_ok"));
    ins.push(abi::shift_left_immediate(
        abi::mfb_arg(0),
        abi::mfb_arg(0),
        1,
    )); // wchar count → byte offset
    ins.push(abi::load_u64(abi::mfb_arg(1), abi::stack_pointer(), WBUF));
    ins.push(abi::add_registers(
        abi::mfb_arg(0),
        abi::mfb_arg(1),
        abi::mfb_arg(0),
    )); // wbuf + len*2
    ins.push(abi::store_u16(abi::ZERO, abi::mfb_arg(0), 0));
    // caretEnd = SendMessageW(edit, WM_GETTEXTLENGTH, 0, 0)
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), EDITH));
    ins.push(abi::move_immediate(
        abi::mfb_arg(1),
        "Integer",
        WM_GETTEXTLENGTH,
    ));
    ins.push(abi::move_immediate(abi::mfb_arg(2), "Integer", "0"));
    ins.push(abi::move_immediate(abi::mfb_arg(3), "Integer", "0"));
    call_external(symbol, "SendMessageW", USER32, &mut ins, &mut rel);
    // SendMessageW(edit, EM_SETSEL, caretEnd, caretEnd) — collapse selection at end.
    // WM_GETTEXTLENGTH returns a C `int`; mask off the garbage high word of rax so
    // the caret position is not a wild value.
    ins.push(abi::move_immediate(
        abi::mfb_arg(2),
        "Integer",
        "4294967295",
    ));
    // `c_return(0)`, not `return_register()`. The comment two lines up already says the
    // length arrives in `rax`; the aligned MFB bank is `rcx` on Win64, so masking
    // `return_register()` masked whatever the last call happened to leave there and the
    // caret was set from it. Same family as bug-478.
    ins.push(abi::and_registers(
        abi::c_return(0),
        abi::c_return(0),
        abi::mfb_arg(2),
    ));
    ins.push(abi::move_register(abi::mfb_arg(2), abi::c_return(0)));
    ins.push(abi::move_register(abi::mfb_arg(3), abi::c_return(0)));
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), EDITH));
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", EM_SETSEL));
    call_external(symbol, "SendMessageW", USER32, &mut ins, &mut rel);
    // SendMessageW(edit, EM_REPLACESEL, 0, wbuf) — insert the text at the caret.
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), EDITH));
    ins.push(abi::move_immediate(
        abi::mfb_arg(1),
        "Integer",
        EM_REPLACESEL,
    ));
    ins.push(abi::move_immediate(abi::mfb_arg(2), "Integer", "0"));
    ins.push(abi::load_u64(abi::mfb_arg(3), abi::stack_pointer(), WBUF));
    call_external(symbol, "SendMessageW", USER32, &mut ins, &mut rel);
    if newline {
        // EDIT controls need CRLF, not a lone LF; append it after the inserted text.
        ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), EDITH));
        ins.push(abi::move_immediate(
            abi::mfb_arg(1),
            "Integer",
            EM_REPLACESEL,
        ));
        ins.push(abi::move_immediate(abi::mfb_arg(2), "Integer", "0"));
        load_addr(abi::mfb_arg(3), CRLF_SYM, symbol, &mut ins, &mut rel);
        call_external(symbol, "SendMessageW", USER32, &mut ins, &mut rel);
    }
    ins.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    ins.push(abi::return_());

    // --- headless / no-window path: write to the inherited standard handle ---
    ins.push(abi::label("std_path"));
    // GetStdHandle(std) — std handle = -(fd) built without a negative immediate.
    ins.push(abi::move_immediate(abi::mfb_arg(0), "Integer", "0"));
    ins.push(abi::subtract_immediate(
        abi::mfb_arg(0),
        abi::mfb_arg(0),
        std_fd,
    ));
    call_external(symbol, "GetStdHandle", KERNEL32, &mut ins, &mut rel);
    ins.push(abi::store_u64(
        abi::c_return(0),
        abi::stack_pointer(),
        HANDLE,
    ));
    // WriteFile(handle, str+8, str[0], &written, NULL)
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), WRITTEN));
    ins.push(abi::outgoing_stack_arg_store(abi::ZERO, 0)); // lpOverlapped = NULL (5th)
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), HANDLE));
    ins.push(abi::load_u64(abi::mfb_arg(1), abi::stack_pointer(), STR)); // str ptr
    ins.push(abi::load_u64(abi::mfb_arg(2), abi::mfb_arg(1), 0)); // len = str[0]
    ins.push(abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(1), 8)); // buf = str+8
    ins.push(abi::add_immediate(
        abi::mfb_arg(3),
        abi::stack_pointer(),
        WRITTEN,
    ));
    call_external(symbol, "WriteFile", KERNEL32, &mut ins, &mut rel);
    if newline {
        ins.push(abi::move_immediate(abi::mfb_arg(0), "Integer", "10"));
        ins.push(abi::store_u8(
            abi::mfb_arg(0),
            abi::stack_pointer(),
            NL_BYTE,
        ));
        ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), WRITTEN));
        ins.push(abi::outgoing_stack_arg_store(abi::ZERO, 0)); // lpOverlapped = NULL (5th)
        ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), HANDLE));
        ins.push(abi::add_immediate(
            abi::mfb_arg(1),
            abi::stack_pointer(),
            NL_BYTE,
        ));
        ins.push(abi::move_immediate(abi::mfb_arg(2), "Integer", "1"));
        ins.push(abi::add_immediate(
            abi::mfb_arg(3),
            abi::stack_pointer(),
            WRITTEN,
        ));
        call_external(symbol, "WriteFile", KERNEL32, &mut ins, &mut rel);
    }
    ins.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    ins.push(abi::return_());

    // --- plan-66-J-5 TUI grid path: draw the string cell-by-cell into the memory DC.
    // Reached only when TUI mode is active. `\n` advances the row (col=0); `\r`
    // homes the col; every other byte is drawn (ASCII → the same UTF-16 unit) at the
    // cursor with the current fg/bg, then the col advances (wrapping at TUI_COLS).
    if let Some(tso) = term_state_offset {
        ins.push(abi::label("term_grid_path"));
        load_addr(abi::mfb_arg(0), TUI_MEMDC_SYM, symbol, &mut ins, &mut rel);
        ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), 0));
        ins.push(abi::store_u64(
            abi::mfb_arg(0),
            abi::stack_pointer(),
            GMEMDC,
        ));
        ins.push(abi::compare_immediate(abi::mfb_arg(0), "0"));
        ins.push(abi::branch_eq("std_path")); // no surface built → inherited handle
                                              // SetTextColor(memDC, fg); SetBkColor(memDC, bg).
        ins.push(abi::load_u64(
            abi::mfb_arg(1),
            ARENA_STATE_REGISTER,
            tso + TERM_STATE_FG_OFFSET,
        ));
        ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), GMEMDC));
        call_external(symbol, "SetTextColor", GDI32, &mut ins, &mut rel);
        ins.push(abi::load_u64(
            abi::mfb_arg(1),
            ARENA_STATE_REGISTER,
            tso + TERM_STATE_BG_OFFSET,
        ));
        ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), GMEMDC));
        call_external(symbol, "SetBkColor", GDI32, &mut ins, &mut rel);
        // plan-70-F: convert the whole UTF-8 string to UTF-16 once (into a 64 KB arena
        // buffer), then iterate UTF-16 units so a multi-byte scalar reaches the CJK
        // font as a real codepoint instead of per-byte tofu. Astral scalars draw as
        // their 2-unit surrogate pair (one glyph); an East-Asian-wide codepoint takes
        // two columns and wraps at the edge.
        arena_alloc("65536", symbol, &mut ins, &mut rel);
        ins.push(abi::store_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            WBUF,
        ));
        ins.push(abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), WBUF));
        ins.push(abi::outgoing_stack_arg_store(abi::mfb_arg(2), 0)); // 5th lpWideCharStr
        ins.push(abi::move_immediate(abi::mfb_arg(2), "Integer", "32767"));
        ins.push(abi::outgoing_stack_arg_store(abi::mfb_arg(2), 1)); // 6th cchWideChar
        ins.push(abi::move_immediate(abi::mfb_arg(0), "Integer", CP_UTF8));
        ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "0"));
        ins.push(abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), STR));
        ins.push(abi::load_u64(abi::mfb_arg(3), abi::mfb_arg(2), 0)); // cbMultiByte = len
        ins.push(abi::add_immediate(abi::mfb_arg(2), abi::mfb_arg(2), 8)); // lpMultiByteStr = str+8
        call_external(symbol, "MultiByteToWideChar", KERNEL32, &mut ins, &mut rel);
        ins.push(abi::move_immediate(
            abi::mfb_arg(1),
            "Integer",
            "4294967295",
        ));
        // The wide-char count is a C result (`rax`), not the aligned MFB bank.
        ins.push(abi::and_registers(
            abi::mfb_arg(0),
            abi::c_return(0),
            abi::mfb_arg(1),
        ));
        ins.push(abi::compare_immediate(abi::mfb_arg(0), "32767"));
        ins.push(abi::branch_le("term_wc_ok"));
        ins.push(abi::move_immediate(abi::mfb_arg(0), "Integer", "32767"));
        ins.push(abi::label("term_wc_ok"));
        ins.push(abi::store_u64(
            abi::mfb_arg(0),
            abi::stack_pointer(),
            WCCOUNT,
        ));
        ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), GI));
        ins.push(abi::label("term_loop"));
        ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), GI));
        ins.push(abi::load_u64(
            abi::mfb_arg(1),
            abi::stack_pointer(),
            WCCOUNT,
        ));
        ins.push(abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(1)));
        ins.push(abi::branch_ge("term_grid_done"));
        // unit = wbuf[i]; default cp = unit (BMP), unitCount = 1.
        ins.push(abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), WBUF));
        ins.push(abi::shift_left_immediate(
            abi::mfb_arg(1),
            abi::mfb_arg(0),
            1,
        )); // i*2
        ins.push(abi::add_registers(
            abi::mfb_arg(2),
            abi::mfb_arg(2),
            abi::mfb_arg(1),
        )); // &wbuf[i]
        ins.push(abi::load_u16(abi::mfb_arg(0), abi::mfb_arg(2), 0)); // unit
        ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "1"));
        ins.push(abi::store_u64(
            abi::mfb_arg(1),
            abi::stack_pointer(),
            UCOUNT,
        ));
        ins.push(abi::store_u64(
            abi::mfb_arg(0),
            abi::stack_pointer(),
            CPSLOT,
        ));
        ins.push(abi::compare_immediate(abi::mfb_arg(0), "10"));
        ins.push(abi::branch_eq("term_nl"));
        ins.push(abi::compare_immediate(abi::mfb_arg(0), "13"));
        ins.push(abi::branch_eq("term_cr"));
        // astral: high surrogate 0xD800..0xDBFF followed by an in-bounds unit.
        ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "55296")); // 0xD800
        ins.push(abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(1)));
        ins.push(abi::branch_lt("term_have_cp"));
        ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "56320")); // 0xDC00
        ins.push(abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(1)));
        ins.push(abi::branch_ge("term_have_cp"));
        ins.push(abi::load_u64(abi::mfb_arg(1), abi::stack_pointer(), GI));
        ins.push(abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(1), 1));
        ins.push(abi::load_u64(
            abi::mfb_arg(3),
            abi::stack_pointer(),
            WCCOUNT,
        ));
        ins.push(abi::compare_registers(abi::mfb_arg(1), abi::mfb_arg(3)));
        ins.push(abi::branch_ge("term_have_cp"));
        // lo = wbuf[i+1]
        ins.push(abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), WBUF));
        ins.push(abi::shift_left_immediate(
            abi::mfb_arg(1),
            abi::mfb_arg(1),
            1,
        )); // (i+1)*2
        ins.push(abi::add_registers(
            abi::mfb_arg(2),
            abi::mfb_arg(2),
            abi::mfb_arg(1),
        ));
        ins.push(abi::load_u16(abi::mfb_arg(1), abi::mfb_arg(2), 0)); // lo
                                                                      // cp = 0x10000 + ((hi-0xD800)<<10) + (lo-0xDC00); hi=ARG[0], lo=ARG[1].
        ins.push(abi::move_immediate(abi::mfb_arg(2), "Integer", "55296"));
        ins.push(abi::subtract_registers(
            abi::mfb_arg(0),
            abi::mfb_arg(0),
            abi::mfb_arg(2),
        ));
        ins.push(abi::shift_left_immediate(
            abi::mfb_arg(0),
            abi::mfb_arg(0),
            10,
        ));
        ins.push(abi::move_immediate(abi::mfb_arg(2), "Integer", "56320"));
        ins.push(abi::subtract_registers(
            abi::mfb_arg(1),
            abi::mfb_arg(1),
            abi::mfb_arg(2),
        ));
        ins.push(abi::add_registers(
            abi::mfb_arg(0),
            abi::mfb_arg(0),
            abi::mfb_arg(1),
        ));
        ins.push(abi::move_immediate(abi::mfb_arg(2), "Integer", "65536"));
        ins.push(abi::add_registers(
            abi::mfb_arg(0),
            abi::mfb_arg(0),
            abi::mfb_arg(2),
        ));
        ins.push(abi::store_u64(
            abi::mfb_arg(0),
            abi::stack_pointer(),
            CPSLOT,
        ));
        ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "2"));
        ins.push(abi::store_u64(
            abi::mfb_arg(1),
            abi::stack_pointer(),
            UCOUNT,
        ));
        ins.push(abi::label("term_have_cp"));
        // plan-70-F: fold trailing combining marks (U+0300..U+036F) and ZWJ sequences
        // (U+200D + the joined scalar) into this cluster's unit run so a single
        // TextOutW composes them (café NFD, ZWJ emoji families). Combining marks are
        // zero-width, so the cluster keeps the base's display width.
        ins.push(abi::label("term_extend"));
        ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), GI));
        ins.push(abi::load_u64(abi::mfb_arg(1), abi::stack_pointer(), UCOUNT));
        ins.push(abi::add_registers(
            abi::mfb_arg(0),
            abi::mfb_arg(0),
            abi::mfb_arg(1),
        )); // j = i + uc
        ins.push(abi::load_u64(
            abi::mfb_arg(1),
            abi::stack_pointer(),
            WCCOUNT,
        ));
        ins.push(abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(1)));
        ins.push(abi::branch_ge("term_extend_done"));
        ins.push(abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), WBUF));
        ins.push(abi::shift_left_immediate(
            abi::mfb_arg(1),
            abi::mfb_arg(0),
            1,
        ));
        ins.push(abi::add_registers(
            abi::mfb_arg(2),
            abi::mfb_arg(2),
            abi::mfb_arg(1),
        ));
        ins.push(abi::load_u16(abi::mfb_arg(0), abi::mfb_arg(2), 0)); // nextUnit
        ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "8205")); // ZWJ U+200D
        ins.push(abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(1)));
        ins.push(abi::branch_eq("term_ext_zwj"));
        ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "768")); // U+0300
        ins.push(abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(1)));
        ins.push(abi::branch_lt("term_extend_done"));
        ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "879")); // U+036F
        ins.push(abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(1)));
        ins.push(abi::branch_gt("term_extend_done"));
        // combining mark → extend by one unit and re-test.
        ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), UCOUNT));
        ins.push(abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1));
        ins.push(abi::store_u64(
            abi::mfb_arg(0),
            abi::stack_pointer(),
            UCOUNT,
        ));
        ins.push(abi::branch("term_extend"));
        ins.push(abi::label("term_ext_zwj"));
        // ZWJ: consume the joiner, then the joined scalar (BMP 1 / astral 2 units).
        ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), UCOUNT));
        ins.push(abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1));
        ins.push(abi::store_u64(
            abi::mfb_arg(0),
            abi::stack_pointer(),
            UCOUNT,
        ));
        ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), GI));
        ins.push(abi::load_u64(abi::mfb_arg(1), abi::stack_pointer(), UCOUNT));
        ins.push(abi::add_registers(
            abi::mfb_arg(0),
            abi::mfb_arg(0),
            abi::mfb_arg(1),
        )); // k
        ins.push(abi::load_u64(
            abi::mfb_arg(1),
            abi::stack_pointer(),
            WCCOUNT,
        ));
        ins.push(abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(1)));
        ins.push(abi::branch_ge("term_extend_done")); // ZWJ at end (defensive)
        ins.push(abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), WBUF));
        ins.push(abi::shift_left_immediate(
            abi::mfb_arg(1),
            abi::mfb_arg(0),
            1,
        ));
        ins.push(abi::add_registers(
            abi::mfb_arg(2),
            abi::mfb_arg(2),
            abi::mfb_arg(1),
        ));
        ins.push(abi::load_u16(abi::mfb_arg(0), abi::mfb_arg(2), 0)); // unit after ZWJ
        ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "55296")); // 0xD800
        ins.push(abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(1)));
        ins.push(abi::branch_lt("term_zwj_bmp"));
        ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "56320")); // 0xDC00
        ins.push(abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(1)));
        ins.push(abi::branch_ge("term_zwj_bmp"));
        ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), UCOUNT));
        ins.push(abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 2)); // astral scalar
        ins.push(abi::store_u64(
            abi::mfb_arg(0),
            abi::stack_pointer(),
            UCOUNT,
        ));
        ins.push(abi::branch("term_extend"));
        ins.push(abi::label("term_zwj_bmp"));
        ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), UCOUNT));
        ins.push(abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1)); // BMP scalar
        ins.push(abi::store_u64(
            abi::mfb_arg(0),
            abi::stack_pointer(),
            UCOUNT,
        ));
        ins.push(abi::branch("term_extend"));
        ins.push(abi::label("term_extend_done"));
        emit_win_wide_width(&mut ins, CPSLOT, WIDTHSLOT);
        // wide-at-edge: a width-2 glyph that would straddle the right edge wraps first.
        ins.push(abi::load_u64(
            abi::mfb_arg(0),
            abi::stack_pointer(),
            WIDTHSLOT,
        ));
        ins.push(abi::compare_immediate(abi::mfb_arg(0), "2"));
        ins.push(abi::branch_ne("term_edge_ok"));
        load_addr(abi::mfb_arg(2), TUI_COL_SYM, symbol, &mut ins, &mut rel);
        ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(2), 0));
        ins.push(abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1));
        ins.push(abi::compare_immediate(
            abi::mfb_arg(0),
            &TUI_COLS.to_string(),
        ));
        ins.push(abi::branch_lt("term_edge_ok"));
        ins.push(abi::store_u64(abi::ZERO, abi::mfb_arg(2), 0)); // col = 0
        load_addr(abi::mfb_arg(1), TUI_ROW_SYM, symbol, &mut ins, &mut rel);
        ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(1), 0));
        ins.push(abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1));
        ins.push(abi::store_u64(abi::mfb_arg(0), abi::mfb_arg(1), 0)); // row++
        ins.push(abi::label("term_edge_ok"));
        // TextOutW(memDC, col*8, row*16, &wbuf[i], unitCount)
        ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), UCOUNT));
        ins.push(abi::outgoing_stack_arg_store(abi::mfb_arg(0), 0)); // 5th arg c (unit count)
        ins.push(abi::load_u64(abi::mfb_arg(3), abi::stack_pointer(), WBUF));
        ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), GI));
        ins.push(abi::shift_left_immediate(
            abi::mfb_arg(0),
            abi::mfb_arg(0),
            1,
        ));
        ins.push(abi::add_registers(
            abi::mfb_arg(3),
            abi::mfb_arg(3),
            abi::mfb_arg(0),
        )); // &wbuf[i]
        load_addr(abi::mfb_arg(1), TUI_COL_SYM, symbol, &mut ins, &mut rel);
        ins.push(abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(1), 0));
        ins.push(abi::shift_left_immediate(
            abi::mfb_arg(1),
            abi::mfb_arg(1),
            3,
        )); // x = col*8
        load_addr(abi::mfb_arg(2), TUI_ROW_SYM, symbol, &mut ins, &mut rel);
        ins.push(abi::load_u64(abi::mfb_arg(2), abi::mfb_arg(2), 0));
        ins.push(abi::shift_left_immediate(
            abi::mfb_arg(2),
            abi::mfb_arg(2),
            4,
        )); // y = row*16
        ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), GMEMDC));
        call_external(symbol, "TextOutW", GDI32, &mut ins, &mut rel);
        // col += width; wrap at TUI_COLS → col=0, row++.
        load_addr(abi::mfb_arg(2), TUI_COL_SYM, symbol, &mut ins, &mut rel);
        ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(2), 0));
        ins.push(abi::load_u64(
            abi::mfb_arg(1),
            abi::stack_pointer(),
            WIDTHSLOT,
        ));
        ins.push(abi::add_registers(
            abi::mfb_arg(0),
            abi::mfb_arg(0),
            abi::mfb_arg(1),
        ));
        ins.push(abi::compare_immediate(
            abi::mfb_arg(0),
            &TUI_COLS.to_string(),
        ));
        ins.push(abi::branch_ge("term_wrap"));
        ins.push(abi::store_u64(abi::mfb_arg(0), abi::mfb_arg(2), 0)); // col += width
        ins.push(abi::branch("term_next"));
        ins.push(abi::label("term_wrap"));
        ins.push(abi::store_u64(abi::ZERO, abi::mfb_arg(2), 0)); // col = 0
        load_addr(abi::mfb_arg(1), TUI_ROW_SYM, symbol, &mut ins, &mut rel);
        ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(1), 0));
        ins.push(abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1));
        ins.push(abi::store_u64(abi::mfb_arg(0), abi::mfb_arg(1), 0)); // row++
        ins.push(abi::branch("term_next"));
        // '\n' → row++, col=0.
        ins.push(abi::label("term_nl"));
        load_addr(abi::mfb_arg(1), TUI_ROW_SYM, symbol, &mut ins, &mut rel);
        ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(1), 0));
        ins.push(abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1));
        ins.push(abi::store_u64(abi::mfb_arg(0), abi::mfb_arg(1), 0));
        load_addr(abi::mfb_arg(1), TUI_COL_SYM, symbol, &mut ins, &mut rel);
        ins.push(abi::store_u64(abi::ZERO, abi::mfb_arg(1), 0));
        ins.push(abi::branch("term_next"));
        // '\r' → col=0.
        ins.push(abi::label("term_cr"));
        load_addr(abi::mfb_arg(1), TUI_COL_SYM, symbol, &mut ins, &mut rel);
        ins.push(abi::store_u64(abi::ZERO, abi::mfb_arg(1), 0));
        ins.push(abi::label("term_next"));
        ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), GI));
        ins.push(abi::load_u64(abi::mfb_arg(1), abi::stack_pointer(), UCOUNT));
        ins.push(abi::add_registers(
            abi::mfb_arg(0),
            abi::mfb_arg(0),
            abi::mfb_arg(1),
        ));
        ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), GI));
        ins.push(abi::branch("term_loop"));
        ins.push(abi::label("term_grid_done"));
        invalidate_main(symbol, &mut ins, &mut rel);
        ins.push(abi::move_immediate(
            RESULT_TAG_REGISTER,
            "Integer",
            RESULT_OK_TAG,
        ));
        ins.push(abi::return_());
    }
    instructions.extend(ins);
    relocations.extend(rel);
}

/// App-mode `io.input` body (plan-66-J-4): render the prompt to the transcript
/// (via the shared `io.write` helper, whose app-mode body appends to the EDIT),
/// then read a line from fd 0 — which `_main` has redirected to the window input
/// pipe — via the shared `io.readLine` helper. The prompt string arrives in
/// `ARG[0]` and is consumed by `io.write`; `io.readLine` needs no argument and
/// leaves its `String` Result in the Result registers, which this tail returns.
/// Mirrors macOS `emit_app_io_input`; on Win64 the return address is on the
/// stack (no link register), so the frame only reserves shadow space for the calls.
pub(super) fn emit_app_io_input(
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    // Append shape (plan-101): no own frame — the `abi_function` vreg finalizer
    // builds it and, because the body makes calls, reserves the Win64 shadow space
    // and realigns the stack (see `finalize_frame`). The prompt is already in
    // `ARG[0]` on entry and nothing is live across the two internal calls, so no
    // vregs are needed.
    let from = symbol;
    call_internal(from, IO_WRITE_SYMBOL, instructions, relocations); // ARG[0] = prompt
    call_internal(from, IO_READ_LINE_SYMBOL, instructions, relocations); // result in Result regs
    instructions.push(abi::return_());
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
pub(super) fn emit_app_io_flush(
    _symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    _relocations: &mut Vec<CodeRelocation>,
) {
    instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    instructions.push(abi::return_());
}

/// App-mode `io.isInputTerminal`/`isOutputTerminal`/`isErrorTerminal` body: the
/// window (or its inherited console) IS the terminal, so all three return
/// `OK(TRUE)` — `RESULT_OK_TAG` in the tag register, `1` in the value register.
pub(super) fn emit_app_io_is_terminal(
    _symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    _relocations: &mut Vec<CodeRelocation>,
) {
    instructions.push(abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "1"));
    instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    instructions.push(abi::return_());
}

/// plan-66-J-5 / plan-101: dispatch a `term::` call to its GDI-grid app-mode body,
/// appending the body's instructions/relocations into the caller's `abi_function`
/// stream (the wrapper then finalizes the frame — no standalone frame here). Every
/// call Windows advertises in app mode is handled; the shared gate prepends the
/// presentation-mode `ErrWrongMode` guard. Returns `None` for a call with no app
/// body (falls through to the console ANSI backend).
///
/// plan-70-F Phase 3: the six positioned draw helpers stamp directly into the
/// persistent memDC (immediate mode). Box/line/fill use Light box-drawing glyphs;
/// drawText/drawGlyph render through the CJK font at correct width.
pub(super) fn emit_app_term_helper(
    call: &str,
    symbol: &str,
    tso: usize,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Option<Result<(), String>> {
    match call {
        "term.on" => emit_term_on(symbol, tso, instructions, relocations),
        "term.off" => emit_term_off(symbol, tso, instructions, relocations),
        "term.clear" => emit_term_clear(symbol, instructions, relocations),
        "term.moveTo" => emit_term_move_to(symbol, instructions, relocations),
        "term.setForeground" => {
            emit_term_set_color(tso, TERM_STATE_FG_OFFSET, instructions, relocations)
        }
        "term.setBackground" => {
            emit_term_set_color(tso, TERM_STATE_BG_OFFSET, instructions, relocations)
        }
        "term.setBold" => {
            emit_term_set_flag(tso, TERM_STATE_BOLD_OFFSET, instructions, relocations)
        }
        "term.setUnderline" => {
            emit_term_set_flag(tso, TERM_STATE_UNDERLINE_OFFSET, instructions, relocations)
        }
        "term.showCursor" => emit_term_cursor_visible(tso, "1", instructions, relocations),
        "term.hideCursor" => emit_term_cursor_visible(tso, "0", instructions, relocations),
        "term.sync" => emit_term_sync(symbol, instructions, relocations),
        "term.terminalSize" => emit_term_size(symbol, instructions, relocations),
        "term.drawHLine" => emit_term_draw_line(symbol, tso, true, instructions, relocations),
        "term.drawVLine" => emit_term_draw_line(symbol, tso, false, instructions, relocations),
        "term.drawBox" => emit_term_draw_box(symbol, tso, instructions, relocations),
        "term.fillRect" => emit_term_fill_rect(symbol, tso, instructions, relocations),
        "term.drawText" => emit_term_draw_text_at(symbol, tso, instructions, relocations),
        "term.drawGlyph" => emit_term_draw_glyph_at(symbol, tso, instructions, relocations),
        _ => return None,
    }
    Some(Ok(()))
}

/// Swap the two stack slots `a_off`/`b_off` when `a > b`, so a caller-supplied
/// coordinate pair reads low-to-high whichever order the program wrote it in.
///
/// Every `term::` span and region is documented to accept its two endpoints in
/// EITHER order (`mfb man term drawHLine`/`drawBox`/`fillRect`), and the console
/// and macOS backends normalise. This backend's loops only ever count UP, so
/// without this a reversed pair drew nothing at all. `tag` names the skip label.
/// Clobbers ARG[0..1].
fn win_normalize_pair(ins: &mut Vec<CodeInstruction>, tag: &str, a_off: usize, b_off: usize) {
    let done = format!("{tag}_ord");
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), a_off));
    ins.push(abi::load_u64(abi::mfb_arg(1), abi::stack_pointer(), b_off));
    ins.push(abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(1)));
    ins.push(abi::branch_le(&done));
    ins.push(abi::store_u64(abi::mfb_arg(1), abi::stack_pointer(), a_off));
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), b_off));
    ins.push(abi::label(&done));
}

/// Clamp `reg` in place into `0 ..= max`. The stack-slot form below is the one the
/// looping emitters use; `term::moveTo` is a leaf that never parks its arguments,
/// so it clamps the incoming registers directly. Clobbers ARG[2] as scratch.
fn win_clamp_register(ins: &mut Vec<CodeInstruction>, tag: &str, reg: Operand, max: usize) {
    let lo = format!("{tag}_clo");
    let hi = format!("{tag}_chi");
    ins.push(abi::compare_immediate(reg.clone(), "0"));
    ins.push(abi::branch_ge(&lo));
    ins.push(abi::move_immediate(reg.clone(), "Integer", "0"));
    ins.push(abi::label(&lo));
    ins.push(abi::move_immediate(
        abi::mfb_arg(2),
        "Integer",
        &max.to_string(),
    ));
    ins.push(abi::compare_registers(reg.clone(), abi::mfb_arg(2)));
    ins.push(abi::branch_le(&hi));
    ins.push(abi::move_register(reg, abi::mfb_arg(2)));
    ins.push(abi::label(&hi));
}

/// Clip the already-normalised span `lo_off ..= hi_off` to `0 ..= max`, branching to
/// `empty` when the span and the grid do not intersect at all.
///
/// The intersection test is not optional, and saturating the two endpoints
/// independently is not a substitute for it: a span wholly left of the grid
/// (`-2 ..= -1`) saturates to `0 ..= 0` and would stamp column 0 — a cell the
/// program never asked for. The console backend's `emit_stamp_run` rejects the
/// empty intersection first, and this is the same order.
///
/// The clamp is also what makes the loop FINITE. These emitters loop one
/// `TextOutW` per cell from the low endpoint to the high one, so an unclamped span
/// is an unbounded loop: `term::drawHLine(style, 0, 0, 2000000000)` — a legal call
/// the console backend clamps to the last column — spun two billion calls here.
///
/// `tag` names the generated labels. Clobbers ARG[0..1].
fn win_clip_span(
    ins: &mut Vec<CodeInstruction>,
    tag: &str,
    lo_off: usize,
    hi_off: usize,
    max: usize,
    empty: &str,
) {
    // No intersection: hi < 0, or lo > max.
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), hi_off));
    ins.push(abi::compare_immediate(abi::mfb_arg(0), "0"));
    ins.push(abi::branch_lt(empty));
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), lo_off));
    ins.push(abi::move_immediate(
        abi::mfb_arg(1),
        "Integer",
        &max.to_string(),
    ));
    ins.push(abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(1)));
    ins.push(abi::branch_gt(empty));
    // The intersection is non-empty, so saturating each endpoint is now correct.
    win_clamp_slot(ins, &format!("{tag}_lo"), lo_off, max);
    win_clamp_slot(ins, &format!("{tag}_hi"), hi_off, max);
}

/// Saturate the stack slot `off` into `0 ..= max`. Only ever reached from
/// [`win_clip_span`], which has already established that the span meets the grid —
/// on its own this would slide a wholly off-grid endpoint onto the rim.
/// Clobbers ARG[0..1].
fn win_clamp_slot(ins: &mut Vec<CodeInstruction>, tag: &str, off: usize, max: usize) {
    let lo = format!("{tag}_clo");
    let hi = format!("{tag}_chi");
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), off));
    ins.push(abi::compare_immediate(abi::mfb_arg(0), "0"));
    ins.push(abi::branch_ge(&lo));
    ins.push(abi::move_immediate(abi::mfb_arg(0), "Integer", "0"));
    ins.push(abi::label(&lo));
    ins.push(abi::move_immediate(
        abi::mfb_arg(1),
        "Integer",
        &max.to_string(),
    ));
    ins.push(abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(1)));
    ins.push(abi::branch_le(&hi));
    ins.push(abi::move_register(abi::mfb_arg(0), abi::mfb_arg(1)));
    ins.push(abi::label(&hi));
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), off));
}

/// Branch to `skip` unless the stack slot `off` holds a value in `0 ..= max`.
///
/// The console backend draws NOTHING for a line whose fixed coordinate is off the
/// grid, and skips a box corner whose cell is off the grid, rather than sliding
/// either onto the edge. This is the test that keeps this backend agreeing.
/// Clobbers ARG[0..1].
fn win_guard_on_grid(ins: &mut Vec<CodeInstruction>, off: usize, max: usize, skip: &str) {
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), off));
    ins.push(abi::compare_immediate(abi::mfb_arg(0), "0"));
    ins.push(abi::branch_lt(skip));
    ins.push(abi::move_immediate(
        abi::mfb_arg(1),
        "Integer",
        &max.to_string(),
    ));
    ins.push(abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(1)));
    ins.push(abi::branch_gt(skip));
}

/// plan-70-F: `SetTextColor`/`SetBkColor` on the memDC (stack slot `memdc_off`) from
/// the current term state colours.
fn win_set_colors(
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
    from: &str,
    tso: usize,
    memdc_off: usize,
) {
    ins.push(abi::load_u64(
        abi::mfb_arg(1),
        ARENA_STATE_REGISTER,
        tso + TERM_STATE_FG_OFFSET,
    ));
    ins.push(abi::load_u64(
        abi::mfb_arg(0),
        abi::stack_pointer(),
        memdc_off,
    ));
    call_external(from, "SetTextColor", GDI32, ins, rel);
    ins.push(abi::load_u64(
        abi::mfb_arg(1),
        ARENA_STATE_REGISTER,
        tso + TERM_STATE_BG_OFFSET,
    ));
    ins.push(abi::load_u64(
        abi::mfb_arg(0),
        abi::stack_pointer(),
        memdc_off,
    ));
    call_external(from, "SetBkColor", GDI32, ins, rel);
}

/// plan-70-F: stamp one BMP glyph (slot `glyph_off`) at grid `(col slot, row slot)`
/// into the memDC (slot `memdc_off`), staging the UTF-16 unit at `wch_off`. The 5th
/// TextOutW arg (count = 1) goes through the `outgoing_stack_arg_store` sentinel,
/// which the `abi_function` finalizer sizes and resolves. Clobbers ARG[0..3].
fn win_stamp_bmp(
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
    from: &str,
    memdc_off: usize,
    col_off: usize,
    row_off: usize,
    glyph_off: usize,
    wch_off: usize,
) {
    ins.push(abi::load_u64(
        abi::mfb_arg(0),
        abi::stack_pointer(),
        glyph_off,
    ));
    ins.push(abi::store_u16(
        abi::mfb_arg(0),
        abi::stack_pointer(),
        wch_off,
    ));
    ins.push(abi::move_immediate(abi::mfb_arg(0), "Integer", "1"));
    ins.push(abi::outgoing_stack_arg_store(abi::mfb_arg(0), 0)); // 5th arg count
    ins.push(abi::add_immediate(
        abi::mfb_arg(3),
        abi::stack_pointer(),
        wch_off,
    )); // &wch
    ins.push(abi::load_u64(
        abi::mfb_arg(1),
        abi::stack_pointer(),
        col_off,
    ));
    ins.push(abi::shift_left_immediate(
        abi::mfb_arg(1),
        abi::mfb_arg(1),
        3,
    )); // x = col*8
    ins.push(abi::load_u64(
        abi::mfb_arg(2),
        abi::stack_pointer(),
        row_off,
    ));
    ins.push(abi::shift_left_immediate(
        abi::mfb_arg(2),
        abi::mfb_arg(2),
        4,
    )); // y = row*16
    ins.push(abi::load_u64(
        abi::mfb_arg(0),
        abi::stack_pointer(),
        memdc_off,
    ));
    call_external(from, "TextOutW", GDI32, ins, rel);
}

/// plan-70-F: `term::drawGlyph(row, column, codepoint)` — stamp one glyph at the
/// cell, astral-capable, in the current colours. Args: ARG[0]=row, ARG[1]=column,
/// ARG[2]=code point (row before column, like every `term::` position).
fn emit_term_draw_glyph_at(
    symbol: &str,
    tso: usize,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    // Append shape (plan-101): no own frame — the `abi_function` finalizer builds it,
    // reserves the requested local scratch (addressed at `sp+<slot>`), the Win64
    // shadow, and the outgoing-arg tail. Local scratch slots below.
    const WCH: usize = 0x30;
    const MEMDC: usize = 0x38;
    const SX: usize = 0x40;
    const SY: usize = 0x48;
    const SCP: usize = 0x50;
    let from = symbol;
    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel: Vec<CodeRelocation> = Vec::new();
    // `drawGlyph(row, column, codepoint)` — the point is row-first.
    ins.push(abi::store_u64(abi::mfb_arg(1), abi::stack_pointer(), SX));
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), SY));
    ins.push(abi::store_u64(abi::mfb_arg(2), abi::stack_pointer(), SCP));
    // Control code points (< U+0020) are skipped, as on the console
    // (`emit_draw_glyph`) and macOS (`emit_app_draw_glyph`) — `mfb man term
    // drawGlyph` states it, and handing a C0 byte to `TextOutW` draws whatever the
    // font maps it to. The cell is bounds-checked, not clamped, so an off-grid
    // point draws nothing either.
    ins.push(abi::compare_immediate(abi::mfb_arg(2), "32"));
    ins.push(abi::branch_lt("dg_done"));
    win_guard_on_grid(&mut ins, SY, TUI_ROWS - 1, "dg_done");
    win_guard_on_grid(&mut ins, SX, TUI_COLS - 1, "dg_done");
    load_addr(abi::mfb_arg(0), TUI_MEMDC_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), 0));
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), MEMDC));
    ins.push(abi::compare_immediate(abi::mfb_arg(0), "0"));
    ins.push(abi::branch_eq("dg_done"));
    win_set_colors(&mut ins, &mut rel, from, tso, MEMDC);
    // Build UTF-16 units from cp; count → 0x20 (TextOutW 5th arg).
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), SCP));
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "65536"));
    ins.push(abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(1)));
    ins.push(abi::branch_lt("dg_bmp"));
    ins.push(abi::subtract_registers(
        abi::mfb_arg(0),
        abi::mfb_arg(0),
        abi::mfb_arg(1),
    )); // cp - 0x10000
    ins.push(abi::move_immediate(abi::mfb_arg(2), "Integer", "1023")); // 0x3FF
    ins.push(abi::and_registers(
        abi::mfb_arg(2),
        abi::mfb_arg(0),
        abi::mfb_arg(2),
    )); // low 10
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "56320")); // 0xDC00
    ins.push(abi::add_registers(
        abi::mfb_arg(2),
        abi::mfb_arg(2),
        abi::mfb_arg(1),
    )); // lo
    ins.push(abi::shift_right_immediate(
        abi::mfb_arg(0),
        abi::mfb_arg(0),
        10,
    )); // high 10
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "55296")); // 0xD800
    ins.push(abi::add_registers(
        abi::mfb_arg(0),
        abi::mfb_arg(0),
        abi::mfb_arg(1),
    )); // hi
    ins.push(abi::store_u16(abi::mfb_arg(0), abi::stack_pointer(), WCH)); // hi @ +0
    ins.push(abi::store_u16(
        abi::mfb_arg(2),
        abi::stack_pointer(),
        WCH + 2,
    )); // lo @ +2
    ins.push(abi::move_immediate(abi::mfb_arg(0), "Integer", "2"));
    ins.push(abi::outgoing_stack_arg_store(abi::mfb_arg(0), 0)); // TextOutW 5th arg (count)
    ins.push(abi::branch("dg_units"));
    ins.push(abi::label("dg_bmp"));
    ins.push(abi::store_u16(abi::mfb_arg(0), abi::stack_pointer(), WCH));
    ins.push(abi::move_immediate(abi::mfb_arg(0), "Integer", "1"));
    ins.push(abi::outgoing_stack_arg_store(abi::mfb_arg(0), 0)); // TextOutW 5th arg (count)
    ins.push(abi::label("dg_units"));
    // TextOutW(memDC, x*8, y*16, &WCH, count)
    ins.push(abi::add_immediate(
        abi::mfb_arg(3),
        abi::stack_pointer(),
        WCH,
    ));
    ins.push(abi::load_u64(abi::mfb_arg(1), abi::stack_pointer(), SX));
    ins.push(abi::shift_left_immediate(
        abi::mfb_arg(1),
        abi::mfb_arg(1),
        3,
    ));
    ins.push(abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), SY));
    ins.push(abi::shift_left_immediate(
        abi::mfb_arg(2),
        abi::mfb_arg(2),
        4,
    ));
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), MEMDC));
    call_external(from, "TextOutW", GDI32, &mut ins, &mut rel);
    ins.push(abi::label("dg_done"));
    invalidate_main(from, &mut ins, &mut rel);
    ins.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    ins.push(abi::return_());
    instructions.extend(ins);
    relocations.extend(rel);
}

/// plan-70-F: `term::drawHLine`/`drawVLine(style, fixed, a, b)` — stamp a Light
/// box-drawing run. Args: ARG[0]=style (ignored — Light), ARG[1]=fixed
/// (row for H / col for V), ARG[2]=a, ARG[3]=b. Clips to the grid.
fn emit_term_draw_line(
    symbol: &str,
    tso: usize,
    horizontal: bool,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    // Append shape (plan-101): no own frame — the finalizer builds it. Local scratch
    // slots below; `win_stamp_bmp` stages its TextOutW 5th arg via the outgoing sentinel.
    const WCH: usize = 0x30;
    const MEMDC: usize = 0x38;
    const FIXED: usize = 0x40; // row (H) or column (V)
    const POS: usize = 0x48; // running a..b
    const ENDV: usize = 0x50;
    const GLYPH: usize = 0x58;
    let from = symbol;
    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel: Vec<CodeRelocation> = Vec::new();
    // Row-before-column argument order: `drawHLine(line, row, columnA, columnB)` puts
    // the fixed row in arg 1, `drawVLine(line, rowA, column, rowB)` the fixed column
    // in arg 2.
    let (fixed_arg, pos_arg, end_arg) = if horizontal { (1, 2, 3) } else { (2, 1, 3) };
    ins.push(abi::store_u64(
        abi::mfb_arg(fixed_arg),
        abi::stack_pointer(),
        FIXED,
    ));
    ins.push(abi::store_u64(
        abi::mfb_arg(pos_arg),
        abi::stack_pointer(),
        POS,
    ));
    ins.push(abi::store_u64(
        abi::mfb_arg(end_arg),
        abi::stack_pointer(),
        ENDV,
    ));
    // The span endpoints may be written in either order, and the span is clamped to
    // the grid; a fixed coordinate off the grid draws nothing. Same three rules the
    // console `emit_draw_line` applies — and the clamp is what bounds the loop below.
    let (fixed_max, span_max) = if horizontal {
        (TUI_ROWS - 1, TUI_COLS - 1)
    } else {
        (TUI_COLS - 1, TUI_ROWS - 1)
    };
    win_guard_on_grid(&mut ins, FIXED, fixed_max, "dln_done");
    win_normalize_pair(&mut ins, "dln", POS, ENDV);
    win_clip_span(&mut ins, "dln", POS, ENDV, span_max, "dln_done");
    // glyph = ─ (9472) for H, │ (9474) for V.
    ins.push(abi::move_immediate(
        abi::mfb_arg(0),
        "Integer",
        if horizontal { "9472" } else { "9474" },
    ));
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), GLYPH));
    load_addr(abi::mfb_arg(0), TUI_MEMDC_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), 0));
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), MEMDC));
    ins.push(abi::compare_immediate(abi::mfb_arg(0), "0"));
    ins.push(abi::branch_eq("dln_done"));
    win_set_colors(&mut ins, &mut rel, from, tso, MEMDC);
    ins.push(abi::label("dln_loop"));
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), POS));
    ins.push(abi::load_u64(abi::mfb_arg(1), abi::stack_pointer(), ENDV));
    ins.push(abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(1)));
    ins.push(abi::branch_gt("dln_done"));
    // H: col=POS, row=FIXED ; V: col=FIXED, row=POS.
    let (col_off, row_off) = if horizontal {
        (POS, FIXED)
    } else {
        (FIXED, POS)
    };
    win_stamp_bmp(
        &mut ins, &mut rel, from, MEMDC, col_off, row_off, GLYPH, WCH,
    );
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), POS));
    ins.push(abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1));
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), POS));
    ins.push(abi::branch("dln_loop"));
    ins.push(abi::label("dln_done"));
    invalidate_main(from, &mut ins, &mut rel);
    ins.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    ins.push(abi::return_());
    instructions.extend(ins);
    relocations.extend(rel);
}

/// plan-70-F: `term::drawBox(line, rowA, columnA, rowB, columnB)` — two H edges, two
/// V edges, four corners. Args ARG[0]=`LineStyle` ordinal, ARG[1]=rowA,
/// ARG[2]=columnA, ARG[3]=rowB, and columnB is the 5th (incoming stack) arg — row
/// before column, like every `term::` position. The corners may be given in either
/// order and the region is clamped to the grid (see `win_normalize_pair` /
/// `win_clamp_slot`).
///
/// **This backend still ignores the `LineStyle` ordinal** and always draws the
/// Light glyphs, unlike the console and macOS backends, which select per style.
/// The same gap applies to `emit_term_draw_line` and `emit_term_fill_rect`.
fn emit_term_draw_box(
    symbol: &str,
    tso: usize,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    // Append shape (plan-101): no own frame — the finalizer builds it. Local scratch
    // slots below; the 5th incoming arg (columnB) is read via the incoming-arg
    // sentinel.
    const WCH: usize = 0x30;
    const MEMDC: usize = 0x38;
    const X1: usize = 0x40;
    const Y1: usize = 0x48;
    const X2: usize = 0x50;
    const Y2: usize = 0x58;
    const POS: usize = 0x60;
    const GLYPH: usize = 0x68;
    // The corners as the program wrote them (normalised but NOT clamped), kept so
    // an off-grid corner is skipped rather than drawn on the rim.
    const CX1: usize = 0x70;
    const CY1: usize = 0x78;
    const CX2: usize = 0x80;
    const CY2: usize = 0x88;
    let from = symbol;
    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel: Vec<CodeRelocation> = Vec::new();
    // Corners arrive as `(rowA, columnA, rowB, columnB)` — every `term::` point is
    // written row before column.
    ins.push(abi::store_u64(abi::mfb_arg(1), abi::stack_pointer(), Y1));
    ins.push(abi::store_u64(abi::mfb_arg(2), abi::stack_pointer(), X1));
    ins.push(abi::store_u64(abi::mfb_arg(3), abi::stack_pointer(), Y2));
    // columnB = 5th incoming (stack) arg — resolved by the finalizer to the caller's
    // outgoing tail above this frame.
    ins.push(abi::incoming_stack_arg_load(abi::mfb_arg(0), 0));
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), X2));
    // Either corner may be given first, so normalise both pairs low-to-high; the
    // edge loops below then count up. The unclamped corners are kept in
    // `CX1`..`CY2` because a corner GLYPH is only stamped when its own cell is on
    // the grid — the console backend clamps each edge but skips an off-grid corner
    // rather than sliding it onto the rim.
    win_normalize_pair(&mut ins, "dbx_x", X1, X2);
    win_normalize_pair(&mut ins, "dbx_y", Y1, Y2);
    for (raw, keep) in [(X1, CX1), (X2, CX2), (Y1, CY1), (Y2, CY2)] {
        ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), raw));
        ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), keep));
    }
    win_clip_span(&mut ins, "dbx_x", X1, X2, TUI_COLS - 1, "dbx_done");
    win_clip_span(&mut ins, "dbx_y", Y1, Y2, TUI_ROWS - 1, "dbx_done");
    load_addr(abi::mfb_arg(0), TUI_MEMDC_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), 0));
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), MEMDC));
    ins.push(abi::compare_immediate(abi::mfb_arg(0), "0"));
    ins.push(abi::branch_eq("dbx_done"));
    win_set_colors(&mut ins, &mut rel, from, tso, MEMDC);
    // Top + bottom edges (─) across x1..x2 at y1 / y2. An edge whose row is off the
    // grid is skipped entirely (the clamped X range still bounds the loop).
    for (yslot, cyslot, tag) in [(Y1, CY1, "dbx_top"), (Y2, CY2, "dbx_bot")] {
        win_guard_on_grid(&mut ins, cyslot, TUI_ROWS - 1, &format!("{tag}_done"));
        ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), X1));
        ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), POS));
        ins.push(abi::move_immediate(abi::mfb_arg(0), "Integer", "9472")); // ─
        ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), GLYPH));
        ins.push(abi::label(&format!("{tag}_loop")));
        ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), POS));
        ins.push(abi::load_u64(abi::mfb_arg(1), abi::stack_pointer(), X2));
        ins.push(abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(1)));
        ins.push(abi::branch_gt(&format!("{tag}_done")));
        win_stamp_bmp(&mut ins, &mut rel, from, MEMDC, POS, yslot, GLYPH, WCH);
        ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), POS));
        ins.push(abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1));
        ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), POS));
        ins.push(abi::branch(&format!("{tag}_loop")));
        ins.push(abi::label(&format!("{tag}_done")));
    }
    // Left + right edges (│) down y1..y2 at x1 / x2. Skipped when the column is off
    // the grid.
    for (xslot, cxslot, tag) in [(X1, CX1, "dbx_left"), (X2, CX2, "dbx_right")] {
        win_guard_on_grid(&mut ins, cxslot, TUI_COLS - 1, &format!("{tag}_done"));
        ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), Y1));
        ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), POS));
        ins.push(abi::move_immediate(abi::mfb_arg(0), "Integer", "9474")); // │
        ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), GLYPH));
        ins.push(abi::label(&format!("{tag}_loop")));
        ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), POS));
        ins.push(abi::load_u64(abi::mfb_arg(1), abi::stack_pointer(), Y2));
        ins.push(abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(1)));
        ins.push(abi::branch_gt(&format!("{tag}_done")));
        win_stamp_bmp(&mut ins, &mut rel, from, MEMDC, xslot, POS, GLYPH, WCH);
        ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), POS));
        ins.push(abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1));
        ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), POS));
        ins.push(abi::branch(&format!("{tag}_loop")));
        ins.push(abi::label(&format!("{tag}_done")));
    }
    // Corners: ┌ x1y1, ┐ x2y1, └ x1y2, ┘ x2y2 — each stamped only when its own cell
    // is on the grid, read from the unclamped copies so a corner off the rim is
    // skipped rather than slid onto it.
    for (xslot, yslot, cp, tag) in [
        (CX1, CY1, "9484", "tl"),
        (CX2, CY1, "9488", "tr"),
        (CX1, CY2, "9492", "bl"),
        (CX2, CY2, "9496", "br"),
    ] {
        let skip = format!("dbx_c{tag}_skip");
        win_guard_on_grid(&mut ins, xslot, TUI_COLS - 1, &skip);
        win_guard_on_grid(&mut ins, yslot, TUI_ROWS - 1, &skip);
        ins.push(abi::move_immediate(abi::mfb_arg(0), "Integer", cp));
        ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), GLYPH));
        win_stamp_bmp(&mut ins, &mut rel, from, MEMDC, xslot, yslot, GLYPH, WCH);
        ins.push(abi::label(&skip));
    }
    ins.push(abi::label("dbx_done"));
    invalidate_main(from, &mut ins, &mut rel);
    ins.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    ins.push(abi::return_());
    instructions.extend(ins);
    relocations.extend(rel);
}

/// plan-70-F: `term::fillRect(fill, rowA, columnA, rowB, columnB)` — fill the cell
/// rect with a space in the current bg (the block glyph would ignore bg). Args as
/// drawBox: row before column, either corner first, region clamped to the grid.
/// **This backend still ignores the `FillStyle` ordinal**, so every style renders
/// as the solid background wash rather than the block/shade glyph.
fn emit_term_fill_rect(
    symbol: &str,
    tso: usize,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    // Append shape (plan-101): no own frame — the finalizer builds it.
    const WCH: usize = 0x30;
    const MEMDC: usize = 0x38;
    const X1: usize = 0x40;
    const Y1: usize = 0x48;
    const X2: usize = 0x50;
    const Y2: usize = 0x58;
    const CX: usize = 0x60;
    const CY: usize = 0x68;
    const GLYPH: usize = 0x70;
    let from = symbol;
    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel: Vec<CodeRelocation> = Vec::new();
    // Corners arrive as `(rowA, columnA, rowB, columnB)` — row before column.
    ins.push(abi::store_u64(abi::mfb_arg(1), abi::stack_pointer(), Y1));
    ins.push(abi::store_u64(abi::mfb_arg(2), abi::stack_pointer(), X1));
    ins.push(abi::store_u64(abi::mfb_arg(3), abi::stack_pointer(), Y2));
    ins.push(abi::incoming_stack_arg_load(abi::mfb_arg(0), 0)); // columnB (5th incoming arg)
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), X2));
    // Either corner may be given first, and the region is clamped to the grid — the
    // clamp is also what bounds the two nested loops below.
    win_normalize_pair(&mut ins, "dfr_x", X1, X2);
    win_normalize_pair(&mut ins, "dfr_y", Y1, Y2);
    win_clip_span(&mut ins, "dfr_x", X1, X2, TUI_COLS - 1, "dfr_done");
    win_clip_span(&mut ins, "dfr_y", Y1, Y2, TUI_ROWS - 1, "dfr_done");
    ins.push(abi::move_immediate(abi::mfb_arg(0), "Integer", "32")); // space (paints bg)
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), GLYPH));
    load_addr(abi::mfb_arg(0), TUI_MEMDC_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), 0));
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), MEMDC));
    ins.push(abi::compare_immediate(abi::mfb_arg(0), "0"));
    ins.push(abi::branch_eq("dfr_done"));
    win_set_colors(&mut ins, &mut rel, from, tso, MEMDC);
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), Y1));
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), CY));
    ins.push(abi::label("dfr_row"));
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), CY));
    ins.push(abi::load_u64(abi::mfb_arg(1), abi::stack_pointer(), Y2));
    ins.push(abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(1)));
    ins.push(abi::branch_gt("dfr_done"));
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), X1));
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), CX));
    ins.push(abi::label("dfr_col"));
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), CX));
    ins.push(abi::load_u64(abi::mfb_arg(1), abi::stack_pointer(), X2));
    ins.push(abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(1)));
    ins.push(abi::branch_gt("dfr_row_next"));
    win_stamp_bmp(&mut ins, &mut rel, from, MEMDC, CX, CY, GLYPH, WCH);
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), CX));
    ins.push(abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1));
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), CX));
    ins.push(abi::branch("dfr_col"));
    ins.push(abi::label("dfr_row_next"));
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), CY));
    ins.push(abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1));
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), CY));
    ins.push(abi::branch("dfr_row"));
    ins.push(abi::label("dfr_done"));
    invalidate_main(from, &mut ins, &mut rel);
    ins.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    ins.push(abi::return_());
    instructions.extend(ins);
    relocations.extend(rel);
}

/// plan-70-F: `term::drawText(row, column, text)` — stamp a UTF-8 string starting at
/// `(row, column)`, one grapheme per cell at its display width, no wrap (clips at
/// the right edge). Args: ARG[0]=row, ARG[1]=column, ARG[2]=text ptr
/// `{len@0, bytes@8}` — row before column, like every `term::` position.
fn emit_term_draw_text_at(
    symbol: &str,
    tso: usize,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    // Append shape (plan-101): no own frame — the finalizer builds it. Local scratch
    // slots below (text-ptr scratch at 0x88); MultiByteToWideChar's 5th/6th and
    // TextOutW's 5th args go through the outgoing-arg sentinel.
    const MEMDC: usize = 0x38;
    const SX: usize = 0x40; // starting column (arg 2)
    const SY: usize = 0x48; // row (arg 1)
    const WBUF: usize = 0x50;
    const WCC: usize = 0x58; // UTF-16 unit count
    const GI: usize = 0x60; // unit index
    const CPSLOT: usize = 0x68;
    const UCOUNT: usize = 0x70;
    const WIDTHSLOT: usize = 0x78;
    const CURCOL: usize = 0x80;
    let from = symbol;
    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel: Vec<CodeRelocation> = Vec::new();
    // `drawText(row, column, text)` — the point is row-first.
    ins.push(abi::store_u64(abi::mfb_arg(1), abi::stack_pointer(), SX));
    ins.push(abi::store_u64(
        abi::mfb_arg(1),
        abi::stack_pointer(),
        CURCOL,
    )); // running col = column
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), SY));
    ins.push(abi::store_u64(abi::mfb_arg(2), abi::stack_pointer(), 0x88)); // text ptr scratch
    load_addr(abi::mfb_arg(0), TUI_MEMDC_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), 0));
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), MEMDC));
    ins.push(abi::compare_immediate(abi::mfb_arg(0), "0"));
    ins.push(abi::branch_eq("dt_done"));
    win_set_colors(&mut ins, &mut rel, from, tso, MEMDC);
    // Convert UTF-8 → UTF-16 into a 64 KB arena buffer.
    arena_alloc("65536", from, &mut ins, &mut rel);
    ins.push(abi::store_u64(
        abi::mfb_return(1),
        abi::stack_pointer(),
        WBUF,
    ));
    ins.push(abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), WBUF));
    ins.push(abi::outgoing_stack_arg_store(abi::mfb_arg(2), 0)); // 5th lpWideCharStr
    ins.push(abi::move_immediate(abi::mfb_arg(2), "Integer", "32767"));
    ins.push(abi::outgoing_stack_arg_store(abi::mfb_arg(2), 1)); // 6th cchWideChar
    ins.push(abi::move_immediate(abi::mfb_arg(0), "Integer", CP_UTF8));
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "0"));
    ins.push(abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), 0x88)); // text ptr
    ins.push(abi::load_u64(abi::mfb_arg(3), abi::mfb_arg(2), 0)); // len
    ins.push(abi::add_immediate(abi::mfb_arg(2), abi::mfb_arg(2), 8)); // bytes
    call_external(from, "MultiByteToWideChar", KERNEL32, &mut ins, &mut rel);
    ins.push(abi::move_immediate(
        abi::mfb_arg(1),
        "Integer",
        "4294967295",
    ));
    // The wide-char count is a C result (`rax`), not the aligned MFB bank.
    ins.push(abi::and_registers(
        abi::mfb_arg(0),
        abi::c_return(0),
        abi::mfb_arg(1),
    ));
    ins.push(abi::compare_immediate(abi::mfb_arg(0), "32767"));
    ins.push(abi::branch_le("dt_wc_ok"));
    ins.push(abi::move_immediate(abi::mfb_arg(0), "Integer", "32767"));
    ins.push(abi::label("dt_wc_ok"));
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), WCC));
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), GI));
    ins.push(abi::label("dt_loop"));
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), GI));
    ins.push(abi::load_u64(abi::mfb_arg(1), abi::stack_pointer(), WCC));
    ins.push(abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(1)));
    ins.push(abi::branch_ge("dt_done"));
    // clip at the right edge (col only grows).
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), CURCOL));
    ins.push(abi::compare_immediate(
        abi::mfb_arg(0),
        &TUI_COLS.to_string(),
    ));
    ins.push(abi::branch_ge("dt_done"));
    ins.push(abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), WBUF));
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), GI));
    ins.push(abi::shift_left_immediate(
        abi::mfb_arg(1),
        abi::mfb_arg(0),
        1,
    ));
    ins.push(abi::add_registers(
        abi::mfb_arg(2),
        abi::mfb_arg(2),
        abi::mfb_arg(1),
    ));
    ins.push(abi::load_u16(abi::mfb_arg(0), abi::mfb_arg(2), 0)); // unit
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "1"));
    ins.push(abi::store_u64(
        abi::mfb_arg(1),
        abi::stack_pointer(),
        UCOUNT,
    ));
    ins.push(abi::store_u64(
        abi::mfb_arg(0),
        abi::stack_pointer(),
        CPSLOT,
    ));
    // astral?
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "55296"));
    ins.push(abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(1)));
    ins.push(abi::branch_lt("dt_have_cp"));
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "56320"));
    ins.push(abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(1)));
    ins.push(abi::branch_ge("dt_have_cp"));
    ins.push(abi::load_u64(abi::mfb_arg(1), abi::stack_pointer(), GI));
    ins.push(abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(1), 1));
    ins.push(abi::load_u64(abi::mfb_arg(3), abi::stack_pointer(), WCC));
    ins.push(abi::compare_registers(abi::mfb_arg(1), abi::mfb_arg(3)));
    ins.push(abi::branch_ge("dt_have_cp"));
    ins.push(abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), WBUF));
    ins.push(abi::shift_left_immediate(
        abi::mfb_arg(1),
        abi::mfb_arg(1),
        1,
    ));
    ins.push(abi::add_registers(
        abi::mfb_arg(2),
        abi::mfb_arg(2),
        abi::mfb_arg(1),
    ));
    ins.push(abi::load_u16(abi::mfb_arg(1), abi::mfb_arg(2), 0)); // lo
    ins.push(abi::move_immediate(abi::mfb_arg(2), "Integer", "55296"));
    ins.push(abi::subtract_registers(
        abi::mfb_arg(0),
        abi::mfb_arg(0),
        abi::mfb_arg(2),
    ));
    ins.push(abi::shift_left_immediate(
        abi::mfb_arg(0),
        abi::mfb_arg(0),
        10,
    ));
    ins.push(abi::move_immediate(abi::mfb_arg(2), "Integer", "56320"));
    ins.push(abi::subtract_registers(
        abi::mfb_arg(1),
        abi::mfb_arg(1),
        abi::mfb_arg(2),
    ));
    ins.push(abi::add_registers(
        abi::mfb_arg(0),
        abi::mfb_arg(0),
        abi::mfb_arg(1),
    ));
    ins.push(abi::move_immediate(abi::mfb_arg(2), "Integer", "65536"));
    ins.push(abi::add_registers(
        abi::mfb_arg(0),
        abi::mfb_arg(0),
        abi::mfb_arg(2),
    ));
    ins.push(abi::store_u64(
        abi::mfb_arg(0),
        abi::stack_pointer(),
        CPSLOT,
    ));
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "2"));
    ins.push(abi::store_u64(
        abi::mfb_arg(1),
        abi::stack_pointer(),
        UCOUNT,
    ));
    ins.push(abi::label("dt_have_cp"));
    emit_win_wide_width(&mut ins, CPSLOT, WIDTHSLOT);
    // TextOutW(memDC, curcol*8, y*16, &wbuf[i], unitCount)
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), UCOUNT));
    ins.push(abi::outgoing_stack_arg_store(abi::mfb_arg(0), 0)); // TextOutW 5th arg (count)
    ins.push(abi::load_u64(abi::mfb_arg(3), abi::stack_pointer(), WBUF));
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), GI));
    ins.push(abi::shift_left_immediate(
        abi::mfb_arg(0),
        abi::mfb_arg(0),
        1,
    ));
    ins.push(abi::add_registers(
        abi::mfb_arg(3),
        abi::mfb_arg(3),
        abi::mfb_arg(0),
    ));
    ins.push(abi::load_u64(abi::mfb_arg(1), abi::stack_pointer(), CURCOL));
    ins.push(abi::shift_left_immediate(
        abi::mfb_arg(1),
        abi::mfb_arg(1),
        3,
    ));
    ins.push(abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), SY));
    ins.push(abi::shift_left_immediate(
        abi::mfb_arg(2),
        abi::mfb_arg(2),
        4,
    ));
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), MEMDC));
    call_external(from, "TextOutW", GDI32, &mut ins, &mut rel);
    // curcol += width; i += unitCount.
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), CURCOL));
    ins.push(abi::load_u64(
        abi::mfb_arg(1),
        abi::stack_pointer(),
        WIDTHSLOT,
    ));
    ins.push(abi::add_registers(
        abi::mfb_arg(0),
        abi::mfb_arg(0),
        abi::mfb_arg(1),
    ));
    ins.push(abi::store_u64(
        abi::mfb_arg(0),
        abi::stack_pointer(),
        CURCOL,
    ));
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), GI));
    ins.push(abi::load_u64(abi::mfb_arg(1), abi::stack_pointer(), UCOUNT));
    ins.push(abi::add_registers(
        abi::mfb_arg(0),
        abi::mfb_arg(0),
        abi::mfb_arg(1),
    ));
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), GI));
    ins.push(abi::branch("dt_loop"));
    ins.push(abi::label("dt_done"));
    invalidate_main(from, &mut ins, &mut rel);
    ins.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    ins.push(abi::return_());
    instructions.extend(ins);
    relocations.extend(rel);
}

/// `term::on()`: build the off-screen grid surface on first use (memory DC +
/// bitmap + a fixed-pitch stock font), clear it, mark TUI state active, hide the
/// transcript EDIT so the grid shows through, and invalidate the window.
fn emit_term_on(
    symbol: &str,
    tso: usize,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    // Append shape (plan-101): no own frame — the finalizer builds it, reserves the
    // Win64 shadow, and sizes the outgoing-arg tail. plan-70-F: CreateFontW takes 14
    // args — its 5th-14th (and PatBlt's 5th/6th) go through the outgoing-arg sentinel
    // (`outgoing_stack_arg_store`), which the finalizer resolves; `HDC_SCREEN` is the
    // one persistent local-scratch slot.
    const HDC_SCREEN: usize = 0x70;
    let from = symbol;
    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel: Vec<CodeRelocation> = Vec::new();
    // Build the surface once (memDC == 0 means not built yet).
    load_addr(abi::mfb_arg(0), TUI_MEMDC_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), 0));
    ins.push(abi::compare_immediate(abi::mfb_arg(0), "0"));
    ins.push(abi::branch_ne("on_have_dc"));
    // hdcScreen = GetDC(NULL)
    ins.push(abi::move_immediate(abi::mfb_arg(0), "Integer", "0"));
    call_external(from, "GetDC", USER32, &mut ins, &mut rel);
    ins.push(abi::store_u64(
        abi::c_return(0),
        abi::stack_pointer(),
        HDC_SCREEN,
    ));
    // memDC = CreateCompatibleDC(hdcScreen); store the global. Reloaded from the slot
    // the line above just wrote rather than from a register: the HDC is a C result and
    // `return_register()` is a different register on Win64, which is how this read the
    // wrong handle (same family as bug-478).
    ins.push(abi::load_u64(
        abi::mfb_arg(0),
        abi::stack_pointer(),
        HDC_SCREEN,
    ));
    call_external(from, "CreateCompatibleDC", GDI32, &mut ins, &mut rel);
    load_addr(abi::mfb_arg(1), TUI_MEMDC_SYM, from, &mut ins, &mut rel);
    ins.push(abi::store_u64(abi::c_return(0), abi::mfb_arg(1), 0));
    // bmp = CreateCompatibleBitmap(hdcScreen, W, H)
    ins.push(abi::load_u64(
        abi::mfb_arg(0),
        abi::stack_pointer(),
        HDC_SCREEN,
    ));
    ins.push(abi::move_immediate(
        abi::mfb_arg(1),
        "Integer",
        &(TUI_COLS * TUI_CELL_W).to_string(),
    ));
    ins.push(abi::move_immediate(
        abi::mfb_arg(2),
        "Integer",
        &(TUI_ROWS * TUI_CELL_H).to_string(),
    ));
    call_external(from, "CreateCompatibleBitmap", GDI32, &mut ins, &mut rel);
    // SelectObject(memDC, bmp) — stage bmp (rax) into ARG[1] before loading memDC.
    ins.push(abi::move_register(abi::mfb_arg(1), abi::c_return(0)));
    load_addr(abi::mfb_arg(0), TUI_MEMDC_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), 0));
    call_external(from, "SelectObject", GDI32, &mut ins, &mut rel);
    // ReleaseDC(NULL, hdcScreen)
    ins.push(abi::move_immediate(abi::mfb_arg(0), "Integer", "0"));
    ins.push(abi::load_u64(
        abi::mfb_arg(1),
        abi::stack_pointer(),
        HDC_SCREEN,
    ));
    call_external(from, "ReleaseDC", USER32, &mut ins, &mut rel);
    // plan-70-F: font = CreateFontW(16, 0, 0, 0, FW_NORMAL=400, 0,0,0,
    //   DEFAULT_CHARSET=1, 0,0,0, FIXED_PITCH|FF_MODERN=49, L"Consolas"); cache it +
    //   SelectObject(memDC, font). DEFAULT_CHARSET drives GDI font-linking so CJK
    //   renders through the system fallback instead of tofu. The 5th-14th args ride
    //   the stack at 0x20..0x68 (Win64), the shadow space is 0x00..0x1F.
    ins.push(abi::move_immediate(abi::mfb_arg(0), "Integer", "400"));
    ins.push(abi::outgoing_stack_arg_store(abi::mfb_arg(0), 0)); // 5th fnWeight
    ins.push(abi::outgoing_stack_arg_store(abi::ZERO, 1)); // 6th fdwItalic
    ins.push(abi::outgoing_stack_arg_store(abi::ZERO, 2)); // 7th fdwUnderline
    ins.push(abi::outgoing_stack_arg_store(abi::ZERO, 3)); // 8th fdwStrikeOut
    ins.push(abi::move_immediate(abi::mfb_arg(0), "Integer", "1"));
    ins.push(abi::outgoing_stack_arg_store(abi::mfb_arg(0), 4)); // 9th DEFAULT_CHARSET
    ins.push(abi::outgoing_stack_arg_store(abi::ZERO, 5)); // 10th OutputPrecision
    ins.push(abi::outgoing_stack_arg_store(abi::ZERO, 6)); // 11th ClipPrecision
    ins.push(abi::outgoing_stack_arg_store(abi::ZERO, 7)); // 12th Quality
    ins.push(abi::move_immediate(abi::mfb_arg(0), "Integer", "49"));
    ins.push(abi::outgoing_stack_arg_store(abi::mfb_arg(0), 8)); // 13th FIXED_PITCH|FF_MODERN
    load_addr(abi::mfb_arg(0), FONT_NAME_SYM, from, &mut ins, &mut rel);
    ins.push(abi::outgoing_stack_arg_store(abi::mfb_arg(0), 9)); // 14th lpszFace
    ins.push(abi::move_immediate(
        abi::mfb_arg(0),
        "Integer",
        &TUI_CELL_H.to_string(),
    )); // nHeight
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "0")); // nWidth (font default)
    ins.push(abi::move_immediate(abi::mfb_arg(2), "Integer", "0")); // nEscapement
    ins.push(abi::move_immediate(abi::mfb_arg(3), "Integer", "0")); // nOrientation
    call_external(from, "CreateFontW", GDI32, &mut ins, &mut rel);
    // cache the HFONT, then SelectObject(memDC, font).
    load_addr(abi::mfb_arg(1), TUI_FONT_SYM, from, &mut ins, &mut rel);
    ins.push(abi::store_u64(abi::c_return(0), abi::mfb_arg(1), 0));
    ins.push(abi::move_register(abi::mfb_arg(1), abi::c_return(0)));
    load_addr(abi::mfb_arg(0), TUI_MEMDC_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), 0));
    call_external(from, "SelectObject", GDI32, &mut ins, &mut rel);
    ins.push(abi::label("on_have_dc"));
    // Clear the grid to black: PatBlt(memDC, 0, 0, W, H, BLACKNESS = 0x42).
    ins.push(abi::move_immediate(
        abi::mfb_arg(2),
        "Integer",
        &(TUI_ROWS * TUI_CELL_H).to_string(),
    ));
    ins.push(abi::outgoing_stack_arg_store(abi::mfb_arg(2), 0)); // height (5th)
    ins.push(abi::move_immediate(abi::mfb_arg(2), "Integer", "66")); // BLACKNESS (6th)
    ins.push(abi::outgoing_stack_arg_store(abi::mfb_arg(2), 1));
    load_addr(abi::mfb_arg(0), TUI_MEMDC_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), 0));
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "0"));
    ins.push(abi::move_immediate(abi::mfb_arg(2), "Integer", "0"));
    ins.push(abi::move_immediate(
        abi::mfb_arg(3),
        "Integer",
        &(TUI_COLS * TUI_CELL_W).to_string(),
    ));
    call_external(from, "PatBlt", GDI32, &mut ins, &mut rel);
    // cursor = (0, 0); term state: active = 1, fg = white, bg = black.
    reset_cursor(from, &mut ins, &mut rel);
    ins.push(abi::move_immediate(abi::mfb_arg(0), "Integer", "1"));
    ins.push(abi::store_u64(
        abi::mfb_arg(0),
        ARENA_STATE_REGISTER,
        tso + TERM_STATE_ACTIVE_OFFSET,
    ));
    ins.push(abi::move_immediate(abi::mfb_arg(0), "Integer", "16777215")); // 0xFFFFFF white
    ins.push(abi::store_u64(
        abi::mfb_arg(0),
        ARENA_STATE_REGISTER,
        tso + TERM_STATE_FG_OFFSET,
    ));
    ins.push(abi::store_u64(
        abi::ZERO,
        ARENA_STATE_REGISTER,
        tso + TERM_STATE_BG_OFFSET,
    ));
    // Hide the transcript EDIT, then invalidate the window to present the grid.
    load_addr(abi::mfb_arg(0), EDIT_HWND_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), 0));
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", SW_HIDE));
    call_external(from, "ShowWindow", USER32, &mut ins, &mut rel);
    invalidate_main(from, &mut ins, &mut rel);
    ins.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    ins.push(abi::return_());
    instructions.extend(ins);
    relocations.extend(rel);
}

/// `term::off()`: leave TUI mode — clear the active flag and re-show the EDIT.
fn emit_term_off(
    symbol: &str,
    tso: usize,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    // Append shape (plan-101): no own frame — the finalizer builds it (shadow only).
    let from = symbol;
    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel: Vec<CodeRelocation> = Vec::new();
    ins.push(abi::store_u64(
        abi::ZERO,
        ARENA_STATE_REGISTER,
        tso + TERM_STATE_ACTIVE_OFFSET,
    ));
    load_addr(abi::mfb_arg(0), EDIT_HWND_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), 0));
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", SW_SHOW));
    call_external(from, "ShowWindow", USER32, &mut ins, &mut rel);
    invalidate_main(from, &mut ins, &mut rel);
    ins.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    ins.push(abi::return_());
    instructions.extend(ins);
    relocations.extend(rel);
}

/// `term::clear()`: black out the grid and home the cursor.
fn emit_term_clear(
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    // Append shape (plan-101): no own frame — the finalizer builds it. PatBlt's 5th/6th
    // args go through the outgoing-arg sentinel.
    let from = symbol;
    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel: Vec<CodeRelocation> = Vec::new();
    load_addr(abi::mfb_arg(0), TUI_MEMDC_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), 0));
    ins.push(abi::compare_immediate(abi::mfb_arg(0), "0"));
    ins.push(abi::branch_eq("clear_done"));
    ins.push(abi::move_immediate(
        abi::mfb_arg(2),
        "Integer",
        &(TUI_ROWS * TUI_CELL_H).to_string(),
    ));
    ins.push(abi::outgoing_stack_arg_store(abi::mfb_arg(2), 0));
    ins.push(abi::move_immediate(abi::mfb_arg(2), "Integer", "66"));
    ins.push(abi::outgoing_stack_arg_store(abi::mfb_arg(2), 1));
    load_addr(abi::mfb_arg(0), TUI_MEMDC_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), 0));
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "0"));
    ins.push(abi::move_immediate(abi::mfb_arg(2), "Integer", "0"));
    ins.push(abi::move_immediate(
        abi::mfb_arg(3),
        "Integer",
        &(TUI_COLS * TUI_CELL_W).to_string(),
    ));
    call_external(from, "PatBlt", GDI32, &mut ins, &mut rel);
    reset_cursor(from, &mut ins, &mut rel);
    ins.push(abi::label("clear_done"));
    ins.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    ins.push(abi::return_());
    instructions.extend(ins);
    relocations.extend(rel);
}

/// `term::moveTo(row, column)`: set the grid cursor (0-based), no frame/call needed.
///
/// Both coordinates are clamped to the grid — `row` into `0 ..= TUI_ROWS-1`,
/// `column` into `0 ..= TUI_COLS-1` — which is the contract every other backend
/// already honoured (`emit_move_to` on the console, `emit_app_move_to` on macOS,
/// `emit_app_term_move_to` on GTK) and the one `mfb man term moveTo` states: "Both
/// coordinates are clamped at both ends, on every backend." This body used to
/// store the raw incoming registers, so an out-of-range `moveTo` parked the cursor
/// off the 80x25 surface and the next `io::write` stamped at a negative or
/// past-the-edge device coordinate.
fn emit_term_move_to(
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    // Append shape (plan-101): no own frame — the finalizer builds it (call-free/leaf).
    let from = symbol;
    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel: Vec<CodeRelocation> = Vec::new();
    win_clamp_register(&mut ins, "mt_row", abi::mfb_arg(0), TUI_ROWS - 1);
    win_clamp_register(&mut ins, "mt_col", abi::mfb_arg(1), TUI_COLS - 1);
    load_addr(abi::mfb_arg(2), TUI_ROW_SYM, from, &mut ins, &mut rel);
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::mfb_arg(2), 0)); // row
    load_addr(abi::mfb_arg(2), TUI_COL_SYM, from, &mut ins, &mut rel);
    ins.push(abi::store_u64(abi::mfb_arg(1), abi::mfb_arg(2), 0)); // column
    ins.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    ins.push(abi::return_());
    instructions.extend(ins);
    relocations.extend(rel);
}

/// `term::setForeground/setBackground(r, g, b)`: pack `r | g<<8 | b<<16` (already
/// GDI COLORREF order) into the term-state color field.
fn emit_term_set_color(
    tso: usize,
    field: usize,
    instructions: &mut Vec<CodeInstruction>,
    _relocations: &mut Vec<CodeRelocation>,
) {
    // Append shape (plan-101): no own frame — the finalizer builds it (call-free/leaf).
    let mut ins: Vec<CodeInstruction> = Vec::new();
    ins.push(abi::shift_left_immediate(
        abi::mfb_arg(1),
        abi::mfb_arg(1),
        8,
    ));
    ins.push(abi::shift_left_immediate(
        abi::mfb_arg(2),
        abi::mfb_arg(2),
        16,
    ));
    ins.push(abi::or_registers(
        abi::mfb_arg(0),
        abi::mfb_arg(0),
        abi::mfb_arg(1),
    ));
    ins.push(abi::or_registers(
        abi::mfb_arg(0),
        abi::mfb_arg(0),
        abi::mfb_arg(2),
    ));
    ins.push(abi::store_u64(
        abi::mfb_arg(0),
        ARENA_STATE_REGISTER,
        tso + field,
    ));
    ins.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    ins.push(abi::return_());
    instructions.extend(ins);
}

/// `term::setBold/setUnderline(on)`: store the boolean into its term-state field.
fn emit_term_set_flag(
    tso: usize,
    field: usize,
    instructions: &mut Vec<CodeInstruction>,
    _relocations: &mut Vec<CodeRelocation>,
) {
    // Append shape (plan-101): no own frame — the finalizer builds it (call-free/leaf).
    let mut ins: Vec<CodeInstruction> = Vec::new();
    ins.push(abi::store_u64(
        abi::mfb_arg(0),
        ARENA_STATE_REGISTER,
        tso + field,
    ));
    ins.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    ins.push(abi::return_());
    instructions.extend(ins);
}

/// `term::showCursor/hideCursor()`: set the cursor-visible term-state flag.
fn emit_term_cursor_visible(
    tso: usize,
    value: &str,
    instructions: &mut Vec<CodeInstruction>,
    _relocations: &mut Vec<CodeRelocation>,
) {
    // Append shape (plan-101): no own frame — the finalizer builds it (call-free/leaf).
    let mut ins: Vec<CodeInstruction> = Vec::new();
    ins.push(abi::move_immediate(abi::mfb_arg(0), "Integer", value));
    ins.push(abi::store_u64(
        abi::mfb_arg(0),
        ARENA_STATE_REGISTER,
        tso + TERM_STATE_CURSOR_VISIBLE_OFFSET,
    ));
    ins.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    ins.push(abi::return_());
    instructions.extend(ins);
}

/// `term::sync()`: present the coalesced grid — invalidate + update the window so
/// WndProc BitBlts the memory DC.
fn emit_term_sync(
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    // Append shape (plan-101): no own frame — the finalizer builds it (shadow only).
    let from = symbol;
    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel: Vec<CodeRelocation> = Vec::new();
    invalidate_main(from, &mut ins, &mut rel);
    load_addr(abi::mfb_arg(0), MAIN_HWND_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), 0));
    call_external(from, "UpdateWindow", USER32, &mut ins, &mut rel);
    ins.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    ins.push(abi::return_());
    instructions.extend(ins);
    relocations.extend(rel);
}

/// plan-98-A Phase 3 (worker side): the `app::setMode` reconcile seam. `setMode`
/// has already stored the new mode into the presentation slot; this reloads it and
/// `SendMessageW`s `WM_APP_RECONCILE` to the main window, so the UI thread — which
/// owns the window — performs the surface build/teardown.
///
/// `SendMessageW`, not `PostMessageW`: a cross-thread `SendMessageW` blocks until
/// the owning thread's message pump dispatches it, so `setMode` returns only once
/// the surface matches. That is required, not stylistic — the worker's next
/// `getMode`/`io::` call must observe the reconciled surface, and it is the same
/// synchronous contract macOS gets from `waitUntilDone:YES`.
///
/// A no-op when `MAIN_HWND_SYM` is 0. That is the headless case: `_main` takes the
/// `headless_spawn` path, which builds no window and runs no message loop, so a
/// `SendMessageW` would have no pump to dispatch it and would block the worker
/// forever. Same shape, same reason, as the macOS marshal's nil-delegate skip.
///
/// plan-98-C Phase 3 (worker side): `_mfb_winapp_canvas_blit`.
///
/// `mfb_arg(0)` = the frame's first pixel, `(1)` = width, `(2)` = height. Packs the
/// frame into a `HeapAlloc` block — width at +0, height at +8, pixels from +16 — and
/// posts the pointer to the window, which takes ownership.
///
/// **Why the dimensions travel inside the block.** One pointer then carries a whole
/// frame, so the handoff needs no lock: the worker builds a block nobody else can
/// see, and every read *and* write of `CANVAS_FRAME_SYM` happens on the UI thread.
/// Publishing width separately would let `WM_PAINT` pair new pixels with an old
/// height and read past the end of the block.
///
/// **Why the copy swizzles.** A 32-bit `BI_RGB` DIB is B, G, R, X in memory, while
/// the rasteriser produces R, G, B, A. The reorder rides along on a copy that has to
/// happen anyway — the caller's block belongs to the next frame the moment this
/// returns — so it costs no extra pass.
///
/// Every value lives in a stack slot rather than a callee-saved register: on Win64
/// the `SCRATCH`/`c_arg` banks alias each other in ways that are invisible until
/// execution (`div` writes `rdx`, low `SCRATCH` maps onto `rcx`/`rdx`/`r8`/`r9`), and
/// this function cannot be executed on the build host.
pub(super) fn emit_canvas_blit_helper() -> CodeFunction {
    // shadow[0..0x20], pixels@0x20, width@0x28, height@0x30, bytes@0x38,
    // block@0x40, cursor@0x48. FRAME ≡ 8 (mod 16).
    const FRAME: usize = 0x58;
    const PIXELS: usize = 0x20;
    const WIDTH: usize = 0x28;
    const HEIGHT: usize = 0x30;
    const BYTES: usize = 0x38;
    const BLOCK: usize = 0x40;
    const CURSOR: usize = 0x48;
    let from = CANVAS_BLIT_SYMBOL;
    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel: Vec<CodeRelocation> = Vec::new();
    ins.push(abi::label("entry"));
    ins.push(abi::subtract_stack(FRAME));
    ins.push(abi::store_u64(
        abi::mfb_arg(0),
        abi::stack_pointer(),
        PIXELS,
    ));
    ins.push(abi::store_u64(abi::mfb_arg(1), abi::stack_pointer(), WIDTH));
    ins.push(abi::store_u64(
        abi::mfb_arg(2),
        abi::stack_pointer(),
        HEIGHT,
    ));
    // bytes = width * height * 4
    ins.push(abi::multiply_registers(
        abi::mfb_arg(3),
        abi::mfb_arg(1),
        abi::mfb_arg(2),
    ));
    ins.push(abi::move_immediate(abi::mfb_arg(0), "Integer", "4"));
    ins.push(abi::multiply_registers(
        abi::mfb_arg(3),
        abi::mfb_arg(3),
        abi::mfb_arg(0),
    ));
    ins.push(abi::store_u64(abi::mfb_arg(3), abi::stack_pointer(), BYTES));

    // block = HeapAlloc(GetProcessHeap(), 0, bytes + 16)
    call_external(from, "GetProcessHeap", KERNEL32, &mut ins, &mut rel);
    ins.push(abi::move_register(abi::mfb_arg(0), abi::c_return(0)));
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "0"));
    ins.push(abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), BYTES));
    ins.push(abi::add_immediate(abi::mfb_arg(2), abi::mfb_arg(2), 16));
    call_external(from, "HeapAlloc", KERNEL32, &mut ins, &mut rel);
    ins.push(abi::compare_immediate(abi::c_return(0), "0"));
    // Out of memory drops the frame rather than failing the call: a renderer that
    // killed the program because one frame could not be shown would be worse than
    // one that skips it, and the next frame re-renders the same scene.
    ins.push(abi::branch_eq("blit_done"));
    // `c_return(0)`, not `return_register()`. `HeapAlloc` hands the block back in the C
    // result (`rax`); the aligned MFB bank is `rcx` on Win64. The null check two lines
    // up already reads `c_return(0)` — it is the *uses* that read the wrong one, which
    // is the shape every member of this family has (bug-478).
    //
    // What it cost: the header stores wrote the width and height THROUGH the width, so
    // the graphics thread faulted storing to address 900 on the first frame of every
    // Windows canvas program (bug-479).
    ins.push(abi::store_u64(
        abi::c_return(0),
        abi::stack_pointer(),
        BLOCK,
    ));
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), WIDTH));
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::c_return(0), 0));
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), HEIGHT));
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::c_return(0), 8));

    // Swizzle-copy RGBA -> BGRX.
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), CURSOR));
    ins.push(abi::label("blit_loop"));
    ins.push(abi::load_u64(abi::mfb_arg(3), abi::stack_pointer(), CURSOR));
    ins.push(abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), BYTES));
    ins.push(abi::compare_registers(abi::mfb_arg(3), abi::mfb_arg(2)));
    ins.push(abi::branch_ge("blit_post"));
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), PIXELS));
    ins.push(abi::add_registers(
        abi::mfb_arg(0),
        abi::mfb_arg(0),
        abi::mfb_arg(3),
    )); // &src[i]
    ins.push(abi::load_u64(abi::mfb_arg(1), abi::stack_pointer(), BLOCK));
    ins.push(abi::add_registers(
        abi::mfb_arg(1),
        abi::mfb_arg(1),
        abi::mfb_arg(3),
    ));
    ins.push(abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(1), 16)); // &dst[i]
    ins.push(abi::load_u8(abi::mfb_arg(2), abi::mfb_arg(0), 2)); // B
    ins.push(abi::store_u8(abi::mfb_arg(2), abi::mfb_arg(1), 0));
    ins.push(abi::load_u8(abi::mfb_arg(2), abi::mfb_arg(0), 1)); // G
    ins.push(abi::store_u8(abi::mfb_arg(2), abi::mfb_arg(1), 1));
    ins.push(abi::load_u8(abi::mfb_arg(2), abi::mfb_arg(0), 0)); // R
    ins.push(abi::store_u8(abi::mfb_arg(2), abi::mfb_arg(1), 2));
    ins.push(abi::move_immediate(abi::mfb_arg(2), "Integer", "255"));
    ins.push(abi::store_u8(abi::mfb_arg(2), abi::mfb_arg(1), 3));
    ins.push(abi::add_immediate(abi::mfb_arg(3), abi::mfb_arg(3), 4));
    ins.push(abi::store_u64(
        abi::mfb_arg(3),
        abi::stack_pointer(),
        CURSOR,
    ));
    ins.push(abi::branch("blit_loop"));

    // PostMessageW(canvasHwnd, WM_APP_BLIT, block, 0) — ownership passes here.
    ins.push(abi::label("blit_post"));
    load_addr(abi::mfb_arg(0), CANVAS_HWND_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), 0));
    ins.push(abi::compare_immediate(abi::mfb_arg(0), "0"));
    ins.push(abi::branch_eq("blit_orphan"));
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", WM_APP_BLIT));
    ins.push(abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), BLOCK));
    ins.push(abi::move_immediate(abi::mfb_arg(3), "Integer", "0"));
    call_external(from, "PostMessageW", USER32, &mut ins, &mut rel);
    ins.push(abi::branch("blit_done"));

    // No canvas window — headless, or the mode changed between render and blit.
    // The block was allocated and nobody will take it, so free it here: leaking one
    // frame buffer per present would be megabytes a second.
    ins.push(abi::label("blit_orphan"));
    call_external(from, "GetProcessHeap", KERNEL32, &mut ins, &mut rel);
    ins.push(abi::move_register(abi::mfb_arg(0), abi::c_return(0)));
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "0"));
    ins.push(abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), BLOCK));
    call_external(from, "HeapFree", KERNEL32, &mut ins, &mut rel);

    ins.push(abi::label("blit_done"));
    ins.push(abi::add_stack(FRAME));
    ins.push(abi::return_());
    code_function("winapp.canvas_blit", CANVAS_BLIT_SYMBOL, ins, rel)
}

/// plan-98-C Phase 3: the worker-side `canvas::blitSurface` seam.
///
/// The caller has already staged the frame pointer, width and height in the MFB
/// argument registers, which is what [`emit_canvas_blit_helper`] expects, so this is
/// a plain call.
pub(super) fn emit_canvas_blit_seam(
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel: Vec<CodeRelocation> = Vec::new();
    call_internal(symbol, CANVAS_BLIT_SYMBOL, &mut ins, &mut rel);
    instructions.extend(ins);
    relocations.extend(rel);
}

/// Appended into the shared (vreg-lowered) `setMode` helper, so it names only the
/// `mfb_arg` role tokens — never a physical register (plan-34-D), and never
/// `c_arg`/`SCRATCH`, whose Win64 realizations alias the callee-saved bank.
pub(super) fn emit_reconcile_seam(
    symbol: &str,
    presentation_mode_offset: usize,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let from = symbol;
    let skip = format!("{symbol}_reconcile_skip");
    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel: Vec<CodeRelocation> = Vec::new();
    load_addr(abi::mfb_arg(0), MAIN_HWND_SYM, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), 0));
    ins.push(abi::compare_immediate(abi::mfb_arg(0), "0"));
    ins.push(abi::branch_eq(&skip));
    ins.push(abi::move_immediate(
        abi::mfb_arg(1),
        "Integer",
        WM_APP_RECONCILE,
    ));
    // wParam = the authoritative mode, reloaded from the slot rather than trusted to
    // a register the address loads above have already clobbered.
    ins.push(abi::load_u64(
        abi::mfb_arg(2),
        ARENA_STATE_REGISTER,
        presentation_mode_offset,
    ));
    ins.push(abi::move_immediate(abi::mfb_arg(3), "Integer", "0")); // lParam
    call_external(from, "SendMessageW", USER32, &mut ins, &mut rel);
    ins.push(abi::label(&skip));
    instructions.extend(ins);
    relocations.extend(rel);
}

/// `term::terminalSize()`: return `{ columns, rows }` (the fixed grid dims) as an
/// arena-allocated 16-byte record. Result value = record ptr, tag = OK.
fn emit_term_size(
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    // Append shape (plan-101): no own frame — the finalizer builds it (shadow only).
    let from = symbol;
    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel: Vec<CodeRelocation> = Vec::new();
    // record = _mfb_arena_alloc(16, align 8) → RET[1] = ptr.
    ins.push(abi::move_immediate(abi::return_register(), "Integer", "16"));
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "8"));
    ins.push(abi::branch_link(ARENA_ALLOC_SYMBOL));
    rel.push(CodeRelocation {
        from: from.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    });
    ins.push(abi::move_immediate(
        abi::mfb_arg(0),
        "Integer",
        &TUI_COLS.to_string(),
    ));
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::mfb_return(1), 0)); // columns@0
    ins.push(abi::move_immediate(
        abi::mfb_arg(0),
        "Integer",
        &TUI_ROWS.to_string(),
    ));
    ins.push(abi::store_u64(abi::mfb_arg(0), abi::mfb_return(1), 8)); // rows@8
    ins.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    )); // RET[1]=ptr survives
    ins.push(abi::return_());
    instructions.extend(ins);
    relocations.extend(rel);
}

/// Zero the grid cursor row/col globals (uses ARG[0]/ARG[1] as scratch).
fn reset_cursor(from: &str, ins: &mut Vec<CodeInstruction>, rel: &mut Vec<CodeRelocation>) {
    load_addr(abi::mfb_arg(0), TUI_ROW_SYM, from, ins, rel);
    ins.push(abi::store_u64(abi::ZERO, abi::mfb_arg(0), 0));
    load_addr(abi::mfb_arg(0), TUI_COL_SYM, from, ins, rel);
    ins.push(abi::store_u64(abi::ZERO, abi::mfb_arg(0), 0));
}

/// `InvalidateRect(mainHwnd, NULL, TRUE)` — request a repaint of the whole client.
fn invalidate_main(from: &str, ins: &mut Vec<CodeInstruction>, rel: &mut Vec<CodeRelocation>) {
    load_addr(abi::mfb_arg(0), MAIN_HWND_SYM, from, ins, rel);
    ins.push(abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), 0));
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "0"));
    ins.push(abi::move_immediate(abi::mfb_arg(2), "Integer", "1"));
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
        utf16z_data_object(RESIZE_W_ENV_SYM, "MFB_CANVAS_RESIZE_W"),
        utf16z_data_object(RESIZE_H_ENV_SYM, "MFB_CANVAS_RESIZE_H"),
        CodeDataObject {
            symbol: RESIZE_BUF_SYM.to_string(),
            kind: "raw".to_string(),
            layout: "UTF-16 scratch for one scripted-resize dimension".to_string(),
            align: 2,
            size: RESIZE_BUF_CHARS * 2,
            value: "00".repeat(RESIZE_BUF_CHARS * 2),
        },
        utf16z_data_object(EDIT_CLASS_SYM, "EDIT"),
        utf16z_data_object(DUMP_ENV_SYM, "MFB_WINAPP_DUMP"),
        utf16z_data_object(CRLF_SYM, "\r\n"),
        utf16z_data_object(INPUT_ENV_SYM, "MFB_WINAPP_INPUT"),
        // Writable 8-byte globals (kind:"raw" → the writable data partition): the
        // transcript EDIT HWND and the main window HWND, both 0 until built.
        writable_qword(EDIT_HWND_SYM),
        writable_qword(MAIN_HWND_SYM),
        // plan-98-A Phase 3: the presentation-mode reconcile's globals — the
        // canvas-mode HWND (0 outside canvas mode) and the unrouted-but-surviving
        // copy of the transcript EDIT HWND.
        writable_qword(CANVAS_HWND_SYM),
        writable_qword(CANVAS_FRAME_SYM),
        writable_qword(EDIT_HWND_SAVED_SYM),
        // plan-66-J-4 input state: the pipe write handle and the EDIT's original
        // window proc, both written by `_main` at window build (0 until then).
        writable_qword(STDIN_WRITE_SYM),
        writable_qword(EDIT_OLDPROC_SYM),
        // plan-66-J-5 term:: TUI grid state: the off-screen memory DC (built
        // lazily by term::on), and the grid cursor (row, col). 0 until on.
        writable_qword(TUI_MEMDC_SYM),
        writable_qword(TUI_ROW_SYM),
        writable_qword(TUI_COL_SYM),
        // plan-70-F: cached CJK-capable HFONT + its face name.
        writable_qword(TUI_FONT_SYM),
        utf16z_data_object(FONT_NAME_SYM, "Consolas"),
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
    use crate::arch::ops::CodeOp;
    use crate::codegen::engine::types::PresentationMode;

    fn spec() -> AppEntrySpec {
        AppEntrySpec {
            language_entry_accepts_args: false,
            uses_term: false,
            initial_mode: PresentationMode::Console,
            uses_canvas: true,
        }
    }

    #[test]
    fn emits_main_worker_wndproc() {
        let fns = emit_app_program_entry(&spec(), &HashMap::new()).expect("app entry");
        let symbols: Vec<&str> = fns.iter().map(|f| f.symbol.as_str()).collect();
        // The PE entry MUST be "_main" (the image entry symbol in app mode), plus
        // the worker and WndProc the bootstrap references.
        assert!(
            symbols.contains(&MAIN_SYMBOL),
            "entry _main present: {symbols:?}"
        );
        assert!(
            symbols.contains(&WORKER_SYMBOL),
            "worker present: {symbols:?}"
        );
        assert!(
            symbols.contains(&WNDPROC_SYMBOL),
            "wndproc present: {symbols:?}"
        );
    }

    /// The worker's frame is a multiple of 16, because a thread start routine is
    /// entered already aligned.
    ///
    /// `BaseThreadInitThunk` does not leave the 8-byte skew an ordinary `call` does, so
    /// the odd-multiple-of-8 frame that is right for a normal prologue is exactly wrong
    /// here — and what it costs is not this function, it is *the whole program*: the
    /// body's alignment is this call site's, and every Win32 call the program ever makes
    /// inherits it. bug-478's second half was `0x28` here, which handed app mode a stack
    /// 8 bytes out for its entire life. The console path was unaffected and green
    /// throughout, which is why it survived four earlier Windows fixes.
    #[test]
    fn the_worker_frame_keeps_the_stack_16_byte_aligned() {
        let fns = emit_app_program_entry(&spec(), &HashMap::new()).expect("app entry");
        let worker = fns
            .iter()
            .find(|f| f.symbol == WORKER_SYMBOL)
            .expect("worker present");
        let reserved: usize = worker
            .instructions
            .iter()
            .find(|ins| ins.op == crate::arch::ops::CodeOp::SubSp)
            .and_then(|ins| ins.get("imm"))
            .and_then(|v| v.parse().ok())
            .expect("the worker reserves a frame");
        assert!(
            reserved >= 0x20,
            "the worker reserves {reserved} bytes; Win64 needs 32 for the callee's \
             shadow space"
        );
        assert_eq!(
            reserved % 16,
            0,
            "the worker reserves {reserved} bytes, which is not a multiple of 16 — a \
             thread start routine is entered ALREADY aligned, so an odd multiple of 8 \
             here misaligns the whole program body (bug-478)"
        );
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
            assert!(
                targets.contains(&want),
                "_main references {want}: {targets:?}"
            );
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
        let mut ins = Vec::new();
        let mut rel = Vec::new();
        emit_app_io_write("_test_io", false, true, None, &mut ins, &mut rel);
        let writes = rel.iter().filter(|r| r.to == "WriteFile").count();
        assert_eq!(
            writes, 2,
            "newline variant issues the text + '\\n' WriteFile"
        );
        assert!(rel.iter().any(|r| r.to == "GetStdHandle"));
        assert!(!ins.is_empty());
    }

    #[test]
    fn io_write_has_transcript_path() {
        // plan-66-J-3: io_write routes to the EDIT control (transcript) when the
        // edit-hwnd global is set — MultiByteToWideChar the print text, then append
        // via EM_REPLACESEL SendMessageW — and falls back to the std handle otherwise.
        let mut _ins = Vec::new();
        let mut rel = Vec::new();
        emit_app_io_write("_test_io", false, true, None, &mut _ins, &mut rel);
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
        assert!(
            targets.contains(&EDIT_CLASS_SYM),
            "references the L\"EDIT\" class"
        );
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
        assert!(
            targets.contains(&EDITPROC_SYMBOL),
            "installs editproc as the subclass"
        );
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
        assert!(
            targets.contains(&"WriteFile"),
            "editproc writes keystrokes to the pipe"
        );
        assert!(
            targets.contains(&"CallWindowProcW"),
            "editproc chains to the original EDIT proc (no J-3 regression)"
        );
        assert!(
            targets.contains(&STDIN_WRITE_SYM),
            "reads the pipe write handle global"
        );
        assert!(
            targets.contains(&EDIT_OLDPROC_SYM),
            "reads the saved original proc"
        );
    }

    #[test]
    fn input_helper_writes_prompt_then_reads_line() {
        // io.input renders the prompt (io.write) then reads a line (io.readLine),
        // which drains fd 0 — the window input pipe.
        let mut _ins = Vec::new();
        let mut rel = Vec::new();
        emit_app_io_input("_test_input", &mut _ins, &mut rel);
        let targets: Vec<&str> = rel.iter().map(|r| r.to.as_str()).collect();
        assert!(
            targets.contains(&IO_WRITE_SYMBOL),
            "renders the prompt via io.write"
        );
        assert!(
            targets.contains(&IO_READ_LINE_SYMBOL),
            "reads the line via io.readLine"
        );
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
            // Positioned draw helpers. plan-70-F replaced the former
            // `ErrUnsupported` stubs with real memDC emitters, so what this asserts
            // now is that each still has a Windows arm rather than falling through
            // to the console lowering (which would find no grid and no-op silently).
            // The `LineStyle`/`FillStyle` ordinal is deliberately ignored by this
            // backend — see `emit_term_draw_box` and `mfb spec app term-backend`.
            "term.drawHLine",
            "term.drawVLine",
            "term.drawBox",
            "term.fillRect",
            "term.drawText",
            "term.drawGlyph",
        ] {
            assert!(
                emit_app_term_helper(call, "_t", 0, &mut Vec::new(), &mut Vec::new()).is_some(),
                "no app-mode term body for {call}"
            );
        }
        // A non-term call falls through (None → console backend).
        assert!(
            emit_app_term_helper("term.bogus", "_t", 0, &mut Vec::new(), &mut Vec::new()).is_none()
        );
    }

    #[test]
    fn term_on_builds_and_shows_grid() {
        let mut _i = Vec::new();
        let mut rel = Vec::new();
        emit_term_on("_t", 0, &mut _i, &mut rel);
        let t: Vec<&str> = rel.iter().map(|r| r.to.as_str()).collect();
        // Builds the off-screen surface, clears it, and hides the transcript EDIT.
        // plan-70-F: the font is a CJK-capable CreateFontW face (font-linking),
        // NOT the legacy SYSTEM_FIXED_FONT bitmap face (GetStockObject).
        for want in [
            "CreateCompatibleDC",
            "CreateCompatibleBitmap",
            "CreateFontW",
            "PatBlt",
            "ShowWindow",
        ] {
            assert!(t.contains(&want), "term::on missing {want}");
        }
        assert!(
            !t.contains(&"GetStockObject"),
            "term::on should no longer use the stock font"
        );
        assert!(
            t.contains(&TUI_MEMDC_SYM) && t.contains(&EDIT_HWND_SYM) && t.contains(&TUI_FONT_SYM)
        );
    }

    #[test]
    fn wndproc_bitblts_the_grid_on_paint() {
        let fns = emit_app_program_entry(&spec(), &HashMap::new()).unwrap();
        let wp = fns.iter().find(|f| f.symbol == WNDPROC_SYMBOL).unwrap();
        let t: Vec<&str> = wp.relocations.iter().map(|r| r.to.as_str()).collect();
        assert!(t.contains(&"BeginPaint") && t.contains(&"BitBlt") && t.contains(&"EndPaint"));
        assert!(
            t.contains(&TUI_MEMDC_SYM),
            "WM_PAINT gates on the memory DC"
        );
    }

    #[test]
    fn io_write_routes_to_grid_when_term_active() {
        // With a term-state offset, io.write gains the TUI grid branch (TextOutW +
        // SetTextColor); without it (None), the body is the J-3 transcript path only.
        let mut _i = Vec::new();
        let mut rel_term = Vec::new();
        emit_app_io_write("_t", false, true, Some(0), &mut _i, &mut rel_term);
        let t: Vec<&str> = rel_term.iter().map(|r| r.to.as_str()).collect();
        assert!(t.contains(&"TextOutW") && t.contains(&"SetTextColor"));
        let mut _i2 = Vec::new();
        let mut rel_plain = Vec::new();
        emit_app_io_write("_t", false, true, None, &mut _i2, &mut rel_plain);
        assert!(
            !rel_plain.iter().any(|r| r.to == "TextOutW"),
            "no grid path without term state"
        );
    }

    #[test]
    fn transcript_nul_offset_is_clamped_within_wbuf() {
        // bug-418: `wbuf` is arena_alloc("65536") (32768 wchars) and
        // MultiByteToWideChar is capped at cchWideChar=32767, so the conversion stays
        // in bounds. But the NUL terminator's offset must ALSO stay in bounds. The
        // buggy code wrote the NUL at `wbuf + str[0]*2`, where str[0] is the untrusted
        // UTF-8 BYTE length: any print ≥ 32768 bytes makes the offset ≥ 65536 and the
        // store lands past the 64 KiB arena block, corrupting adjacent arena data.
        //
        // The fix derives the offset from the converted wchar count clamped to ≤ 32767
        // (max byte offset 32767*2 = 65534 < 65536). Structurally: the transcript NUL
        // store (the unique `str_u16` of the zero register) must be preceded by a
        // `cmp_imm rhs=32767` clamp. The raw `str[0]*2` form has no such clamp — only
        // `cmp_imm rhs=0` guards (the path selectors), never a 32767 bound.
        let mut ins = Vec::new();
        let mut _rel = Vec::new();
        emit_app_io_write("_test_io", false, false, None, &mut ins, &mut _rel);
        let nul_idx = ins
            .iter()
            .position(|i| i.op == CodeOp::StrU16 && i.get("src").as_deref() == Some(abi::ZERO))
            .expect("transcript path NUL-terminates wbuf via a str_u16 of the zero register");
        let clamped = ins[..nul_idx]
            .iter()
            .any(|i| i.op == CodeOp::CmpImm && i.get("rhs").as_deref() == Some("32767"));
        assert!(
            clamped,
            "the wbuf NUL offset must be clamped to ≤ 32767 wchars so it stays within \
             the 65536-byte wbuf (bug-418); found no `cmp_imm rhs=32767` before the store"
        );
    }
}

#[cfg(test)]
/// plan-98-A Phase 3: the Windows presentation-mode reconcile and its
/// `Mode.Canvas` arm.
///
/// Structural, not behavioral: the dev/CI host is macOS and cannot execute a PE.
/// This mirrors how the rest of this backend's coverage works.
mod canvas_reconcile_tests {
    use super::*;
    use crate::arch::ops::CodeOp;

    fn spec(initial_mode: PresentationMode) -> AppEntrySpec {
        AppEntrySpec {
            language_entry_accepts_args: false,
            uses_term: false,
            initial_mode,
            uses_canvas: true,
        }
    }

    fn func(symbol: &str, initial_mode: PresentationMode) -> CodeFunction {
        emit_app_program_entry(&spec(initial_mode), &HashMap::new())
            .expect("app entry")
            .into_iter()
            .find(|f| f.symbol == symbol)
            .unwrap_or_else(|| panic!("{symbol} must be emitted"))
    }

    fn externals(f: &CodeFunction, name: &str) -> usize {
        f.relocations.iter().filter(|r| r.to == name).count()
    }

    fn compare_immediates(f: &CodeFunction) -> Vec<String> {
        f.instructions
            .iter()
            .filter(|i| i.op == CodeOp::CmpImm)
            .filter_map(|i| {
                i.fields
                    .iter()
                    .find(|(name, _)| *name == "rhs")
                    .map(|(_, value)| value.to_string())
            })
            .collect()
    }

    /// The seam must marshal to the UI thread with SendMessageW, not PostMessageW:
    /// a cross-thread SendMessageW blocks until the owning thread's pump dispatches
    /// it, so `setMode` returns only once the surface matches. Posting would let
    /// the worker's next `getMode`/`io::` call observe the OLD surface.
    #[test]
    fn reconcile_seam_marshals_synchronously() {
        let mut ins = Vec::new();
        let mut rel = Vec::new();
        emit_reconcile_seam("_mfb_rt_app_app_setMode", 4096, &mut ins, &mut rel);
        assert_eq!(
            rel.iter().filter(|r| r.to == "SendMessageW").count(),
            1,
            "the seam must SendMessageW (synchronous) the reconcile to the UI thread"
        );
        assert_eq!(
            rel.iter().filter(|r| r.to == "PostMessageW").count(),
            0,
            "PostMessageW would return before the surface was reconciled"
        );
    }

    /// Headless builds no window and runs no message pump, so a SendMessageW would
    /// have nothing to dispatch it and would block the worker forever. The seam
    /// must therefore test the main-window global and skip when it is null — the
    /// same shape, and the same reason, as the macOS nil-delegate skip.
    #[test]
    fn reconcile_seam_skips_when_there_is_no_window() {
        let mut ins = Vec::new();
        let mut rel = Vec::new();
        emit_reconcile_seam("_mfb_rt_app_app_setMode", 4096, &mut ins, &mut rel);
        assert!(
            rel.iter().any(|r| r.to == MAIN_HWND_SYM),
            "the seam must read the main-window global to detect headless"
        );
        let guard = ins.iter().position(|i| i.op == CodeOp::BranchEq);
        let send = ins.iter().position(|i| i.op == CodeOp::BranchLink);
        assert!(
            matches!((guard, send), (Some(g), Some(s)) if g < s),
            "the null-window guard must precede the SendMessageW, or a headless \
             worker deadlocks"
        );
    }

    /// `Canvas` (2) must be dispatched before the `Console`-or-not test: with a
    /// third variant "not Console" no longer implies `None`, so the two-way shape
    /// would hide the window the instant a program entered canvas mode.
    #[test]
    fn wndproc_dispatches_canvas_before_the_console_test() {
        let wndproc = func(WNDPROC_SYMBOL, PresentationMode::Console);
        let immediates = compare_immediates(&wndproc);
        let message = immediates
            .iter()
            .position(|value| value == WM_APP_RECONCILE)
            .expect("the wndproc must handle WM_APP_RECONCILE");
        let canvas = immediates
            .iter()
            .skip(message)
            .position(|value| value == "2")
            .expect("the reconcile arm must test for the Canvas discriminant");
        let console = immediates
            .iter()
            .skip(message)
            .position(|value| value == "0")
            .expect("the reconcile arm must test for the Console discriminant");
        assert!(
            canvas < console,
            "Canvas (2) must be dispatched before the Console/not-Console test; \
             got {immediates:?}"
        );
    }

    /// The canvas arm publishes the HWND (Windows' native surface handle) and both
    /// non-canvas arms clear it, so "retrievable in canvas mode, released after
    /// exit" is a real invariant rather than a stale last-value. The Phase 4 WM_CHAR
    /// arm reads it a fourth time, as its "am I in canvas mode?" gate.
    #[test]
    fn canvas_hwnd_is_published_by_one_arm_and_cleared_by_the_others() {
        let wndproc = func(WNDPROC_SYMBOL, PresentationMode::Console);
        assert_eq!(
            externals(&wndproc, CANVAS_HWND_SYM),
            12,
            "6 address loads — Canvas publishes, Console and None clear, WM_CHAR \
             gates on it, WM_PAINT gates on it, and WM_SIZE gates on it \
             (plan-98-F Phase 3) — each an adrp/add pair = 2 relocations"
        );
    }

    /// Open Decision 3: `term::on` traps in `Mode.Canvas`, but a program can call it
    /// in `Console` and *then* switch — the memDC outlives the switch, so every
    /// WM_PAINT would repaint the stale character grid over the canvas client area.
    /// The canvas gate must therefore come **before** the memDC test.
    #[test]
    fn wm_paint_checks_canvas_mode_before_the_term_grid() {
        let wndproc = func(WNDPROC_SYMBOL, PresentationMode::Console);
        let order: Vec<&str> = wndproc
            .relocations
            .iter()
            .map(|r| r.to.as_str())
            .filter(|name| {
                *name == CANVAS_HWND_SYM || *name == TUI_MEMDC_SYM || *name == "BeginPaint"
            })
            .collect();
        let canvas = order
            .iter()
            .position(|name| *name == CANVAS_HWND_SYM)
            .expect("WM_PAINT must consult canvas mode");
        let memdc = order
            .iter()
            .position(|name| *name == TUI_MEMDC_SYM)
            .expect("WM_PAINT must consult the term memDC");
        let paint = order
            .iter()
            .position(|name| *name == "BeginPaint")
            .expect("the term paint path must BeginPaint");
        assert!(
            canvas < memdc && memdc < paint,
            "canvas gate, then memDC test, then paint; got {order:?}"
        );
    }

    /// plan-98-C Phase 3: the blit copies the frame before posting it.
    ///
    /// The caller's block belongs to the next frame the moment `canvas::blitSurface`
    /// returns, so a pointer posted without a copy would be painted after it had been
    /// overwritten. `HeapAlloc` before `PostMessageW` is what proves the copy happens
    /// on the worker's side of the handoff.
    #[test]
    fn canvas_blit_copies_the_frame_before_posting_it() {
        let blit = emit_canvas_blit_helper();
        let order: Vec<&str> = blit
            .relocations
            .iter()
            .map(|r| r.to.as_str())
            .filter(|name| *name == "HeapAlloc" || *name == "PostMessageW")
            .collect();
        assert_eq!(
            order,
            vec!["HeapAlloc", "PostMessageW"],
            "the frame must be copied into its own block before the window is given \
             the pointer; got {order:?}"
        );
    }

    /// A frame the window never takes is freed by the worker.
    ///
    /// `PostMessageW` fails when there is no canvas window — headless, or the mode
    /// changed between the render and the blit — and nobody else will ever see that
    /// block. Leaking one per present is megabytes a second.
    #[test]
    fn canvas_blit_frees_a_frame_no_window_will_take() {
        let blit = emit_canvas_blit_helper();
        assert_eq!(
            blit.relocations
                .iter()
                .filter(|r| r.to.as_str() == "HeapFree")
                .count(),
            1,
            "the blit must free the block on the no-window path"
        );
    }

    /// The window frees the frame it replaces, and only the window frees a live one.
    ///
    /// This is what makes `CANVAS_FRAME_SYM` safe without a lock: the free, the
    /// publish and the WM_PAINT read all happen on the UI thread.
    #[test]
    fn wndproc_replaces_the_committed_frame_and_repaints() {
        let wndproc = func(WNDPROC_SYMBOL, PresentationMode::Console);
        let names: Vec<&str> = wndproc.relocations.iter().map(|r| r.to.as_str()).collect();
        assert!(
            names.contains(&"HeapFree"),
            "the blit message arm must free the frame it replaces"
        );
        assert!(
            names.contains(&"InvalidateRect"),
            "committing a frame must invalidate the client area that paints it"
        );
        assert!(
            names.contains(&"SetDIBitsToDevice"),
            "WM_PAINT must draw the committed frame"
        );
    }

    /// WM_PAINT draws the canvas frame instead of falling through to the default.
    ///
    /// Before this phase the canvas gate branched to `wnd_default`, which is correct
    /// only while there is nothing to paint. The frame symbol must be consulted
    /// after the canvas gate and before `SetDIBitsToDevice`, so a canvas program that
    /// has not presented yet still takes the default path rather than drawing a null
    /// block.
    #[test]
    fn wm_paint_draws_the_frame_only_once_one_is_committed() {
        let wndproc = func(WNDPROC_SYMBOL, PresentationMode::Console);
        let order: Vec<&str> = wndproc
            .relocations
            .iter()
            .map(|r| r.to.as_str())
            .filter(|name| {
                *name == CANVAS_HWND_SYM
                    || *name == CANVAS_FRAME_SYM
                    || *name == "SetDIBitsToDevice"
            })
            .collect();
        let canvas = order
            .iter()
            .position(|name| *name == CANVAS_HWND_SYM)
            .expect("WM_PAINT must consult canvas mode");
        let frame = order
            .iter()
            .position(|name| *name == CANVAS_FRAME_SYM)
            .expect("WM_PAINT must consult the committed frame");
        let draw = order
            .iter()
            .position(|name| *name == "SetDIBitsToDevice")
            .expect("the canvas paint path must draw the frame");
        assert!(
            canvas < frame && frame < draw,
            "canvas gate, then the committed-frame test, then the draw; got {order:?}"
        );
    }

    /// plan-98-A Phase 4: canvas keyboard input. `Console` mode's keys reach the
    /// pipe through the transcript EDIT's subclass, but canvas mode hides that EDIT,
    /// so focus falls to the top-level window and its WM_CHAR must feed the pipe
    /// here instead — otherwise `io::readByte` in canvas mode blocks forever.
    #[test]
    fn wndproc_feeds_the_input_pipe_on_wm_char_in_canvas_mode() {
        let wndproc = func(WNDPROC_SYMBOL, PresentationMode::Console);
        let immediates = compare_immediates(&wndproc);
        assert!(
            immediates.iter().any(|value| value == WM_CHAR),
            "the wndproc must handle WM_CHAR; got {immediates:?}"
        );
        assert!(
            externals(&wndproc, STDIN_WRITE_SYM) >= 2,
            "the WM_CHAR arm must read the input pipe's write handle"
        );
        assert!(
            externals(&wndproc, "WriteFile") >= 1,
            "the WM_CHAR arm must write the typed byte to the pipe"
        );
        assert!(
            immediates.iter().any(|value| value == VK_RETURN),
            "Enter must be translated to '\\n' or io::readLine never terminates"
        );
    }

    /// The transcript EDIT's handle must survive being unrouted. The reconcile
    /// zeroes EDIT_HWND_SYM outside Console so io writes degrade to the fd sink;
    /// without the saved copy the control would be unreachable forever after the
    /// first mode switch and Console could never be restored.
    #[test]
    fn edit_hwnd_has_a_surviving_copy() {
        let main = func(MAIN_SYMBOL, PresentationMode::Console);
        assert!(
            externals(&main, EDIT_HWND_SAVED_SYM) >= 2,
            "_main must stash the EDIT hwnd where the reconcile can restore it from"
        );
        let wndproc = func(WNDPROC_SYMBOL, PresentationMode::Console);
        assert!(
            externals(&wndproc, EDIT_HWND_SAVED_SYM) >= 2,
            "the reconcile must read the saved EDIT hwnd to restore Console routing"
        );
    }

    /// A program referencing `app::setMode` starts in `None`, so it must start
    /// windowless — otherwise a canvas program flashes a transcript window before
    /// its first setMode. Console-default `_main` must NOT gain that hide.
    #[test]
    fn none_default_starts_windowless_and_console_default_does_not() {
        let none = func(MAIN_SYMBOL, PresentationMode::None);
        let console = func(MAIN_SYMBOL, PresentationMode::Console);
        assert_eq!(
            externals(&console, "ShowWindow"),
            0,
            "a Console-default program must keep its exact startup path"
        );
        assert_eq!(
            externals(&none, "ShowWindow"),
            1,
            "a None-default program must hide the window at startup"
        );
    }

    /// Every global the arm addresses must be emitted, or the relocations dangle.
    #[test]
    fn canvas_globals_are_emitted() {
        let objects = app_mode_data_objects("proj");
        for symbol in [CANVAS_HWND_SYM, EDIT_HWND_SAVED_SYM] {
            assert!(
                objects.iter().any(|d| d.symbol == symbol),
                "{symbol} must be emitted as a writable global"
            );
        }
    }
}
