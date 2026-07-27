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
    AppEntrySpec, AppHookBody, MACAPP_PROGRAM_SYMBOL, RESULT_OK_TAG, RESULT_TAG_REGISTER,
    RESULT_VALUE_REGISTER,
};

const KERNEL32: &str = "kernel32.dll";
const USER32: &str = "user32.dll";

const MAIN_SYMBOL: &str = "_main";
const WORKER_SYMBOL: &str = "_mfb_winapp_worker";
const WNDPROC_SYMBOL: &str = "_mfb_winapp_wndproc";

const CLASS_NAME_SYM: &str = "_mfb_winapp_class";
const TITLE_SYM: &str = "_mfb_winapp_title";
const HEADLESS_ENV_SYM: &str = "_mfb_winapp_headless_env";

// WS_OVERLAPPEDWINDOW | WS_VISIBLE = 0x10CF0000; CW_USEDEFAULT = 0x80000000.
const WS_OVERLAPPED_VISIBLE: &str = "282001408"; // 0x10CF0000
const CW_USEDEFAULT: &str = "2147483648"; // 0x80000000
const FILE_FLAG_STDOUT_FD: usize = 11; // -(-11) STD_OUTPUT_HANDLE
const FILE_FLAG_STDERR_FD: usize = 12; // -(-12) STD_ERROR_HANDLE
const WM_DESTROY: &str = "2";

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
    Ok(vec![emit_main(), emit_worker(), emit_wndproc()])
}

/// `_main`: the PE entry. Frame (mirrors spike.rs): shadow [0x00..0x20], outgoing
/// stack args [0x20..0x60], WNDCLASSEXW [0x60..0xB0], MSG [0xB0..0xE0],
/// hInstance @0xE0, hwnd @0xE8, worker HANDLE @0xF0. FRAME 0xF8 keeps the PE
/// entry's `sp % 16 == 8` arrival 16-aligned before the first call.
fn emit_main() -> CodeFunction {
    const FRAME: usize = 0xF8;
    const WNDCLASS: usize = 0x60;
    const MSG: usize = 0xB0;
    const HINSTANCE: usize = 0xE0;
    const HWND: usize = 0xE8;
    const WORKERH: usize = 0xF0;
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

    // CreateWindowExW(0, &class, &title, style, CW, CW, 400, 300, 0, 0, hInst, 0)
    ins.push(abi::move_immediate(abi::ARG[0], "Integer", "0"));
    load_addr(abi::ARG[1], CLASS_NAME_SYM, from, &mut ins, &mut rel);
    load_addr(abi::ARG[2], TITLE_SYM, from, &mut ins, &mut rel);
    ins.push(abi::move_immediate(abi::ARG[3], "Integer", WS_OVERLAPPED_VISIBLE));
    // stack args (5th..11th) at [sp+0x20..0x58].
    ins.push(abi::move_immediate(abi::ARG[4], "Integer", CW_USEDEFAULT));
    ins.push(abi::store_u64(abi::ARG[4], abi::stack_pointer(), 0x20)); // x
    ins.push(abi::store_u64(abi::ARG[4], abi::stack_pointer(), 0x28)); // y
    ins.push(abi::move_immediate(abi::ARG[4], "Integer", "400"));
    ins.push(abi::store_u64(abi::ARG[4], abi::stack_pointer(), 0x30)); // width
    ins.push(abi::move_immediate(abi::ARG[4], "Integer", "300"));
    ins.push(abi::store_u64(abi::ARG[4], abi::stack_pointer(), 0x38)); // height
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x40)); // hWndParent
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x48)); // hMenu
    ins.push(abi::load_u64(abi::ARG[4], abi::stack_pointer(), HINSTANCE));
    ins.push(abi::store_u64(abi::ARG[4], abi::stack_pointer(), 0x50)); // hInstance
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x58)); // lpParam
    call_external(from, "CreateWindowExW", USER32, &mut ins, &mut rel);
    ins.push(abi::store_u64(abi::return_register(), abi::stack_pointer(), HWND));

    // CreateThread(NULL, 0, &worker, hwnd, 0, NULL)
    ins.push(abi::move_immediate(abi::ARG[0], "Integer", "0"));
    ins.push(abi::move_immediate(abi::ARG[1], "Integer", "0"));
    load_addr(abi::ARG[2], WORKER_SYMBOL, from, &mut ins, &mut rel);
    ins.push(abi::load_u64(abi::ARG[3], abi::stack_pointer(), HWND));
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x20)); // dwCreationFlags
    ins.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x28)); // lpThreadId
    call_external(from, "CreateThread", KERNEL32, &mut ins, &mut rel);
    ins.push(abi::store_u64(abi::return_register(), abi::stack_pointer(), WORKERH));

    // Message loop.
    ins.push(abi::label("msg_loop"));
    ins.push(abi::add_immediate(abi::ARG[0], abi::stack_pointer(), MSG));
    ins.push(abi::move_immediate(abi::ARG[1], "Integer", "0"));
    ins.push(abi::move_immediate(abi::ARG[2], "Integer", "0"));
    ins.push(abi::move_immediate(abi::ARG[3], "Integer", "0"));
    call_external(from, "GetMessageW", USER32, &mut ins, &mut rel);
    ins.push(abi::compare_immediate(abi::return_register(), "0"));
    ins.push(abi::branch_le("main_done")); // 0 = WM_QUIT, -1 = error
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

    // ExitProcess(0). (The worker's program body usually ExitProcess's first.)
    ins.push(abi::label("main_done"));
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
    let from = WNDPROC_SYMBOL;
    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel: Vec<CodeRelocation> = Vec::new();
    ins.push(abi::label("entry"));
    ins.push(abi::subtract_stack(0x28));
    // msg is ARG[1]; the ARG registers still hold the WndProc arguments.
    ins.push(abi::compare_immediate(abi::ARG[1], WM_DESTROY));
    ins.push(abi::branch_eq("wnd_destroy"));
    // default: DefWindowProcW(hwnd, msg, wParam, lParam) — args untouched.
    call_external(from, "DefWindowProcW", USER32, &mut ins, &mut rel);
    ins.push(abi::add_stack(0x28));
    ins.push(abi::return_());
    ins.push(abi::label("wnd_destroy"));
    ins.push(abi::move_immediate(abi::ARG[0], "Integer", "0"));
    call_external(from, "PostQuitMessage", USER32, &mut ins, &mut rel);
    ins.push(abi::move_immediate(abi::return_register(), "Integer", "0"));
    ins.push(abi::add_stack(0x28));
    ins.push(abi::return_());
    code_function("winapp.wndproc", WNDPROC_SYMBOL, ins, rel)
}

/// App-mode `io.print`/`io.write`/`io.printError`/`io.writeError` body (J-2): the
/// string object is in `ARG[0]` (`{u64 len @0; bytes @8}`); write it to the
/// inherited standard handle via `WriteFile(GetStdHandle(std), bytes, len,
/// &written, NULL)`. A GUI-subsystem `.exe` launched from a console inherits its
/// standard handles, so the box run observes the output. Returns `RESULT_OK_TAG`.
/// (J-3 routes this to the GDI transcript when a window is attached.)
pub(super) fn emit_app_io_write_helper(symbol: &str, stderr: bool, newline: bool) -> AppHookBody {
    const FRAME: usize = 0x50;
    const OVERLAPPED: usize = 0x20; // WriteFile 5th arg
    const NL_BYTE: usize = 0x30;
    const STR: usize = 0x38;
    const WRITTEN: usize = 0x40;
    const HANDLE: usize = 0x48;
    let std_fd = if stderr { FILE_FLAG_STDERR_FD } else { FILE_FLAG_STDOUT_FD };
    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel: Vec<CodeRelocation> = Vec::new();
    ins.push(abi::label("entry"));
    ins.push(abi::subtract_stack(FRAME));
    ins.push(abi::store_u64(abi::ARG[0], abi::stack_pointer(), STR));
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
    (
        CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        ins,
        rel,
    )
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
    ]
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
        assert_eq!(objs.len(), 3);
        let title = objs.iter().find(|o| o.symbol == TITLE_SYM).unwrap();
        // "MyProj" → 6 UTF-16 code units × 2 bytes + 2-byte NUL = 14.
        assert_eq!(title.size, 14);
        assert_eq!(title.align, 2);
        assert!(title.value.ends_with("0000"));
    }

    #[test]
    fn io_write_newline_variant_writes_twice() {
        let (_frame, ins, rel) = emit_app_io_write_helper("_test_io", false, true);
        let writes = rel.iter().filter(|r| r.to == "WriteFile").count();
        assert_eq!(writes, 2, "newline variant issues the text + '\\n' WriteFile");
        assert!(rel.iter().any(|r| r.to == "GetStdHandle"));
        assert!(!ins.is_empty());
    }
}
