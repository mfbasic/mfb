//! Windows x86-64 native-plan platform (plan-47-D, minimal machine floor).
//!
//! Windows has no stable syscall ABI, so every OS primitive is an imported-DLL
//! call: the arena maps memory with `kernel32!VirtualAlloc`/`VirtualFree` and the
//! program exits with `kernel32!ExitProcess`. This platform declares exactly the
//! `PlatformImport`s the floor needs; each later surface (47-E–J) adds its own
//! DLL's worth on the same mechanism. Every import group is gated behind the
//! `runtime_calls` the backend advertises, so an unimplemented surface is a
//! compile-time rejection, never a dead IAT entry.

use crate::target::shared::nir::NirModule;
use crate::target::shared::plan::{self, NativePlan, NativePlanPlatform, PlatformImport};
use crate::target::shared::runtime::RuntimeHelperSpec;

const KERNEL32: &str = "kernel32.dll";
const WS2_32: &str = "ws2_32.dll";
const BCRYPT: &str = "bcrypt.dll";
const SECUR32: &str = "secur32.dll";
const CRYPT32: &str = "crypt32.dll";
const ADVAPI32: &str = "advapi32.dll";
const SHLWAPI: &str = "shlwapi.dll"; // bug-431: PathRemoveFileSpecA for the vendored-DLL path
const SHELL32: &str = "shell32.dll"; // os.args: CommandLineToArgvW (plan-66-B)
                                     // WASAPI audio (plan-66 G+H): the COM runtime (object activation) rides ole32; the
                                     // endpoint objects themselves are called through their vtables (no import).
const OLE32: &str = "ole32.dll";
/// `net::ping`'s ICMP API (plan-110-A). Present on every supported Windows: the
/// `Icmp*` exports were confirmed in `C:\Windows\System32\IPHLPAPI.DLL` on the
/// Windows 11 test box (10.0.26100.9168).
const IPHLPAPI: &str = "iphlpapi.dll";

pub(crate) fn lower_module(module: &NirModule) -> Result<NativePlan, String> {
    plan::lower_module_for_platform(module, &Platform)
}

pub(crate) struct Platform;

fn import(symbol: &str, library: &str, required_by: &str) -> PlatformImport {
    PlatformImport {
        library: library.to_string(),
        symbol: symbol.to_string(),
        required_by: required_by.to_string(),
    }
}

impl NativePlanPlatform for Platform {
    fn target(&self) -> &'static str {
        "windows-x86_64"
    }

    fn entry_imports(&self, module: &NirModule) -> Vec<PlatformImport> {
        // The entry maps/unmaps the arena (VirtualAlloc/VirtualFree), seeds the
        // arena start time (GetSystemTimePreciseAsFileTime) and the always-on
        // memory-fill RNG (BCryptGenRandom, bcrypt.dll).
        let mut imports = vec![
            import("VirtualAlloc", KERNEL32, "_start"),
            import("VirtualFree", KERNEL32, "_start"),
            import("GetSystemTimePreciseAsFileTime", KERNEL32, "_start"),
            import("BCryptGenRandom", "bcrypt.dll", "_start"),
            // The entry sets the console text code page to UTF-8 so verbatim UTF-8
            // output decodes as glyphs, not OEM-code-page mojibake (bug-392;
            // emit_console_utf8). Output + input, for symmetry.
            import("SetConsoleOutputCP", KERNEL32, "_start"),
            import("SetConsoleCP", KERNEL32, "_start"),
            // The entry's implicit program exit (emit_program_exit) — the NIR of a
            // plain `RETURN` has no ExitProgram op, so this import rides the entry,
            // not `program_exit_imports`.
            import("ExitProcess", KERNEL32, "_start"),
        ];
        if module
            .entry
            .as_ref()
            .is_some_and(|entry| entry.accepts_args)
        {
            imports.extend([
                import("GetCommandLineW", KERNEL32, "_start"),
                import("CommandLineToArgvW", SHELL32, "_start"),
                import("LocalFree", KERNEL32, "_start"),
                import("WideCharToMultiByte", KERNEL32, "_start"),
            ]);
        }
        imports
    }

    fn entry_error_imports(&self, _module: &NirModule) -> Vec<PlatformImport> {
        // The entry's error tail writes a diagnostic via GetStdHandle + WriteFile
        // (emit_write) before exiting.
        vec![
            import("GetStdHandle", KERNEL32, "_start"),
            import("WriteFile", KERNEL32, "_start"),
        ]
    }

    fn program_exit_imports(&self, required_by: &str) -> Vec<PlatformImport> {
        vec![import("ExitProcess", KERNEL32, required_by)]
    }

    fn app_mode_imports(&self) -> Vec<PlatformImport> {
        // plan-66-J: the Win32 app-mode floor (win_x86_64::app). `_main` builds a
        // RegisterClassExW/CreateWindowExW window and runs a GetMessageW loop; the
        // worker is a CreateThread routine; console output rides GetStdHandle +
        // WriteFile. Duplicates with kernel32 runtime imports (CreateThread,
        // GetStdHandle, WriteFile, GetEnvironmentVariableW) are deduped in the
        // merged import table. The `.rsrc`/subsystem work is plan-66-K/I.
        const USER32: &str = "user32.dll";
        const GDI32: &str = "gdi32.dll";
        vec![
            import("GetModuleHandleW", KERNEL32, "_main"),
            import("GetEnvironmentVariableW", KERNEL32, "_main"),
            import("CreateThread", KERNEL32, "_main"),
            import("WaitForSingleObject", KERNEL32, "_main"),
            // plan-98-F Phase 3: the headless scripted resize waits for the first
            // frame before resizing, and polls with `Sleep`.
            import("Sleep", KERNEL32, "_main"),
            import("ExitThread", KERNEL32, "_main"),
            import("GetStdHandle", KERNEL32, "_main"),
            import("WriteFile", KERNEL32, "_main"),
            import("ExitProcess", KERNEL32, "_main"),
            // plan-66-J-3 transcript: io::print → EDIT control. MultiByteToWideChar
            // converts the UTF-8 print text; SendMessageW appends it (EM_REPLACESEL).
            import("MultiByteToWideChar", KERNEL32, "_main"),
            // plan-66-J-4 input: a pipe feeds the worker's readLine (fd 0 via
            // SetStdHandle); the EDIT subclass (SetWindowLongPtrW/CallWindowProcW)
            // writes each typed WM_CHAR byte to the pipe (the per-character macOS
            // keyDown model — no line read-back).
            import("CreatePipe", KERNEL32, "_main"),
            import("SetStdHandle", KERNEL32, "_main"),
            import("SetWindowLongPtrW", USER32, "_main"),
            import("CallWindowProcW", USER32, "_main"),
            import("RegisterClassExW", USER32, "_main"),
            import("CreateWindowExW", USER32, "_main"),
            // plan-98-C Phase 3: the canvas frame blit. The worker allocates and
            // swizzles the frame into a process-heap block and posts it; WM_PAINT
            // draws it with SetDIBitsToDevice and the message arm frees the block it
            // replaces.
            import("GetProcessHeap", KERNEL32, "_main"),
            import("HeapAlloc", KERNEL32, "_main"),
            import("HeapFree", KERNEL32, "_main"),
            import("SetDIBitsToDevice", GDI32, "_main"),
            import("GetMessageW", USER32, "_main"),
            import("TranslateMessage", USER32, "_main"),
            import("DispatchMessageW", USER32, "_main"),
            import("DefWindowProcW", USER32, "_main"),
            import("PostQuitMessage", USER32, "_main"),
            import("PostMessageW", USER32, "_main"),
            import("SendMessageW", USER32, "_main"),
            // plan-66-J-5 term:: TUI grid: an off-screen GDI memory DC + fixed-pitch
            // stock font (built by term::on), drawn per-cell (SetTextColor/SetBkColor
            // + TextOutW) and BitBlt'd to the window on WM_PAINT; term::sync presents.
            import("GetDC", USER32, "_main"),
            import("ReleaseDC", USER32, "_main"),
            import("ShowWindow", USER32, "_main"),
            import("InvalidateRect", USER32, "_main"),
            import("UpdateWindow", USER32, "_main"),
            import("BeginPaint", USER32, "_main"),
            import("EndPaint", USER32, "_main"),
            import("CreateCompatibleDC", GDI32, "_main"),
            import("CreateCompatibleBitmap", GDI32, "_main"),
            import("SelectObject", GDI32, "_main"),
            import("GetStockObject", GDI32, "_main"),
            import("PatBlt", GDI32, "_main"),
            import("BitBlt", GDI32, "_main"),
            import("TextOutW", GDI32, "_main"),
            import("SetTextColor", GDI32, "_main"),
            import("SetBkColor", GDI32, "_main"),
            // plan-70-F: a CJK-capable fixed-pitch font (font-linking) + wide-glyph
            // rendering + UTF-8→UTF-16 decode for the TUI grid.
            import("CreateFontW", GDI32, "_main"),
            import("ExtTextOutW", GDI32, "_main"),
            import("MultiByteToWideChar", KERNEL32, "_main"),
        ]
    }

    fn runtime_imports(&self, spec: &RuntimeHelperSpec) -> Vec<PlatformImport> {
        let required_by = crate::target::shared::runtime::symbol_for_call(spec.helper, spec.call);
        let required_by = required_by.as_str();
        // Every path-taking fs helper marshals UTF-8 → UTF-16 (MultiByteToWideChar)
        // before its `*W` Win32 call (plan-47-F §3.4).
        match spec.call {
            // ===== plan-66 A/B/C/D import arms, dropped by the stale main→P-66
            // merge (same as the mod.rs advertising); restored. =====
            // Phase A — datetime (no libc clocks on Windows).
            "datetime.monotonicNanos" => vec![
                import("QueryPerformanceCounter", KERNEL32, required_by),
                import("QueryPerformanceFrequency", KERNEL32, required_by),
            ],
            "datetime.nowNanos" => vec![import(
                "GetSystemTimePreciseAsFileTime",
                KERNEL32,
                required_by,
            )],
            "datetime.localOffset" => vec![
                import("FileTimeToSystemTime", KERNEL32, required_by),
                import("SystemTimeToTzSpecificLocalTime", KERNEL32, required_by),
                import("SystemTimeToFileTime", KERNEL32, required_by),
            ],
            // Phase B — os. name/arch are const strings (no import → fall through).
            "os.pid" => vec![import("GetCurrentProcessId", KERNEL32, required_by)],
            "os.cpuCount" => vec![import("GetSystemInfo", KERNEL32, required_by)],
            "os.version" => vec![import("RtlGetVersion", "ntdll", required_by)],
            "os.uptime" => vec![import("GetTickCount64", KERNEL32, required_by)],
            // plan-99: `os::sleep` carries BOTH sleep branches in one body. The
            // main-thread branch is the shared relative-`nanosleep` block, which
            // `emit_windows_thread_call` maps to `Sleep(dwMilliseconds)`; the worker
            // branch is the condvar wait, whose `pthread_mutex_lock`/`unlock`/
            // `cond_timedwait`/`clock_gettime` map to the SRW + condition-variable
            // + precise-time set below.
            "os.sleep" => vec![
                import("Sleep", KERNEL32, required_by),
                import("AcquireSRWLockExclusive", KERNEL32, required_by),
                import("ReleaseSRWLockExclusive", KERNEL32, required_by),
                import("SleepConditionVariableSRW", KERNEL32, required_by),
                import("GetSystemTimePreciseAsFileTime", KERNEL32, required_by),
            ],
            "os.isAdmin" => vec![import("IsUserAnAdmin", "shell32", required_by)],
            "os.getEnv" | "os.getEnvOr" | "os.hasEnv" | "os.setEnv" | "os.unsetEnv" => vec![
                import("AcquireSRWLockExclusive", KERNEL32, required_by),
                import("ReleaseSRWLockExclusive", KERNEL32, required_by),
                import("MultiByteToWideChar", KERNEL32, required_by),
                import("WideCharToMultiByte", KERNEL32, required_by),
                import("GetEnvironmentVariableW", KERNEL32, required_by),
                import("SetEnvironmentVariableW", KERNEL32, required_by),
                import("GetLastError", KERNEL32, required_by),
            ],
            "os.hostName" => vec![
                import("GetComputerNameExW", KERNEL32, required_by),
                import("WideCharToMultiByte", KERNEL32, required_by),
            ],
            "os.userName" => vec![
                import("GetUserNameW", ADVAPI32, required_by),
                import("WideCharToMultiByte", KERNEL32, required_by),
            ],
            "os.executablePath" => vec![
                import("GetModuleFileNameW", KERNEL32, required_by),
                import("WideCharToMultiByte", KERNEL32, required_by),
            ],
            "os.environ" => vec![
                import("AcquireSRWLockExclusive", KERNEL32, required_by),
                import("ReleaseSRWLockExclusive", KERNEL32, required_by),
                import("GetEnvironmentStringsW", KERNEL32, required_by),
                import("FreeEnvironmentStringsW", KERNEL32, required_by),
                import("WideCharToMultiByte", KERNEL32, required_by),
            ],
            "os.args" => vec![
                import("GetCommandLineW", KERNEL32, required_by),
                import("CommandLineToArgvW", SHELL32, required_by),
                import("LocalFree", KERNEL32, required_by),
                import("WideCharToMultiByte", KERNEL32, required_by),
            ],
            // Phase C — io console input. isBuffered/setBuffered make no OS call
            // (fall through). The stdin-broadcast log rides heap + SRWLOCK/condvar.
            "io.input" | "io.readLine" | "io.readChar" | "io.readByte" | "io.pollInput"
            | "io.flush" => vec![
                import("GetStdHandle", KERNEL32, required_by),
                import("ReadFile", KERNEL32, required_by),
                import("WriteFile", KERNEL32, required_by),
                import("WaitForSingleObject", KERNEL32, required_by),
                import("GetConsoleMode", KERNEL32, required_by),
                import("SetConsoleMode", KERNEL32, required_by),
                import("GetProcessHeap", KERNEL32, required_by),
                import("HeapAlloc", KERNEL32, required_by),
                import("HeapFree", KERNEL32, required_by),
                import("InitializeSRWLock", KERNEL32, required_by),
                import("AcquireSRWLockExclusive", KERNEL32, required_by),
                import("ReleaseSRWLockExclusive", KERNEL32, required_by),
                import("InitializeConditionVariable", KERNEL32, required_by),
                import("WakeConditionVariable", KERNEL32, required_by),
                import("WakeAllConditionVariable", KERNEL32, required_by),
                import("SleepConditionVariableSRW", KERNEL32, required_by),
                import("CreateThread", KERNEL32, required_by),
                import("CloseHandle", KERNEL32, required_by),
                import("GetLastError", KERNEL32, required_by),
                // plan-66-J-4: io.pollInput on the app-mode input pipe checks queued
                // bytes with PeekNamedPipe (WaitForSingleObject false-positives on a
                // pipe), gated on GetFileType, backing off with Sleep.
                import("GetFileType", KERNEL32, required_by),
                import("PeekNamedPipe", KERNEL32, required_by),
                import("Sleep", KERNEL32, required_by),
            ],
            "fs.exists" | "fs.fileExists" | "fs.directoryExists" => vec![
                import("MultiByteToWideChar", KERNEL32, required_by),
                import("GetFileAttributesW", KERNEL32, required_by),
            ],
            "fs.readText" | "fs.readBytes" | "fs.readAll" | "fs.readAllBytes" => vec![
                import("MultiByteToWideChar", KERNEL32, required_by),
                import("CreateFileW", KERNEL32, required_by),
                import("ReadFile", KERNEL32, required_by),
                import("SetFilePointerEx", KERNEL32, required_by),
                import("CloseHandle", KERNEL32, required_by),
                import("GetLastError", KERNEL32, required_by),
            ],
            "fs.writeText" | "fs.writeBytes" | "fs.appendText" | "fs.appendBytes" => vec![
                import("MultiByteToWideChar", KERNEL32, required_by),
                import("CreateFileW", KERNEL32, required_by),
                import("WriteFile", KERNEL32, required_by),
                import("SetFilePointerEx", KERNEL32, required_by),
                import("FlushFileBuffers", KERNEL32, required_by),
                import("CloseHandle", KERNEL32, required_by),
                import("GetLastError", KERNEL32, required_by),
            ],
            // emit_fs_path_operation: one Win32 BOOL call each, over a marshaled path.
            "fs.deleteFile" => vec![
                import("MultiByteToWideChar", KERNEL32, required_by),
                import("DeleteFileW", KERNEL32, required_by),
                import("GetLastError", KERNEL32, required_by),
            ],
            "fs.createDirectory" | "fs.createDirectories" => vec![
                import("MultiByteToWideChar", KERNEL32, required_by),
                import("CreateDirectoryW", KERNEL32, required_by),
                import("GetLastError", KERNEL32, required_by),
            ],
            "fs.deleteDirectory" => vec![
                import("MultiByteToWideChar", KERNEL32, required_by),
                import("RemoveDirectoryW", KERNEL32, required_by),
                import("GetLastError", KERNEL32, required_by),
            ],
            "fs.setCurrentDirectory" => vec![
                import("MultiByteToWideChar", KERNEL32, required_by),
                import("SetCurrentDirectoryW", KERNEL32, required_by),
                import("GetLastError", KERNEL32, required_by),
            ],
            // Path-producing queries: convert the UTF-16 result back to UTF-8
            // (WideCharToMultiByte). No input path to marshal.
            "fs.currentDirectory" => vec![
                import("GetCurrentDirectoryW", KERNEL32, required_by),
                import("WideCharToMultiByte", KERNEL32, required_by),
                import("GetLastError", KERNEL32, required_by),
            ],
            "fs.tempDirectory" => vec![
                import("GetTempPathW", KERNEL32, required_by),
                import("WideCharToMultiByte", KERNEL32, required_by),
                import("GetLastError", KERNEL32, required_by),
            ],
            // Directory iteration: FindFirstFileW returns the first entry with the
            // handle; FindNextFileW walks the rest; each cFileName is UTF-16 →
            // UTF-8 (WideCharToMultiByte).
            "fs.listDirectory" => vec![
                import("MultiByteToWideChar", KERNEL32, required_by),
                import("FindFirstFileW", KERNEL32, required_by),
                import("FindNextFileW", KERNEL32, required_by),
                import("FindClose", KERNEL32, required_by),
                import("WideCharToMultiByte", KERNEL32, required_by),
                import("GetLastError", KERNEL32, required_by),
            ],
            // canonicalPath and isWithin both canonicalize via GetFullPathNameW
            // (isWithin canonicalizes base+path then does a prefix containment check
            // in the shared helper). plan-66-E adds isWithin here. (This arm's
            // isWithin merge was dropped by the stale-merge into P-66; restored.)
            "fs.canonicalPath" | "fs.isWithin" => vec![
                import("MultiByteToWideChar", KERNEL32, required_by),
                import("GetFullPathNameW", KERNEL32, required_by),
                import("WideCharToMultiByte", KERNEL32, required_by),
                import("GetLastError", KERNEL32, required_by),
            ],
            // The File-resource surface: openFile yields a resource holding the
            // CreateFileW handle; the per-resource ops reuse ReadFile / WriteFile /
            // SetFilePointerEx / FlushFileBuffers / CloseHandle.
            "fs.openFile" | "fs.open" => vec![
                import("MultiByteToWideChar", KERNEL32, required_by),
                import("CreateFileW", KERNEL32, required_by),
                import("GetLastError", KERNEL32, required_by),
            ],
            // plan-66-E: openFileNoFollow opens normally then verifies (via
            // emit_verify_nofollow) that the handle's GetFinalPathNameByHandleW
            // path equals the GetFullPathNameW lexical canonical of the requested
            // path — a symlink/junction at any component is refused (ELOOP analog).
            "fs.openFileNoFollow" => vec![
                import("MultiByteToWideChar", KERNEL32, required_by),
                import("CreateFileW", KERNEL32, required_by),
                import("CloseHandle", KERNEL32, required_by),
                import("GetFullPathNameW", KERNEL32, required_by),
                import("GetFinalPathNameByHandleW", KERNEL32, required_by),
                import("GetLastError", KERNEL32, required_by),
            ],
            // plan-66-E: openWithin canonicalizes the root lexically (realpath =
            // GetFullPathNameW → WideCharToMultiByte), joins relPath, opens, then
            // emit_verify_within opens the root as a directory
            // (FILE_FLAG_BACKUP_SEMANTICS) and checks the opened handle's resolved
            // final path stays under the root's resolved final path.
            "fs.openWithin" => vec![
                import("MultiByteToWideChar", KERNEL32, required_by),
                import("WideCharToMultiByte", KERNEL32, required_by),
                import("CreateFileW", KERNEL32, required_by),
                import("CloseHandle", KERNEL32, required_by),
                import("GetFullPathNameW", KERNEL32, required_by),
                import("GetFinalPathNameByHandleW", KERNEL32, required_by),
                import("GetLastError", KERNEL32, required_by),
            ],
            // createTempFile opens an O_EXCL file at a randomized name
            // (emit_open_file + emit_random_bytes over BCryptGenRandom). plan-66-E.
            // (This arm was dropped by the stale-merge into P-66; restored.)
            "fs.createTempFile" => vec![
                import("MultiByteToWideChar", KERNEL32, required_by),
                import("CreateFileW", KERNEL32, required_by),
                import("GetLastError", KERNEL32, required_by),
                import("BCryptGenRandom", BCRYPT, required_by),
            ],
            // Atomic writes (plan-66-E): mkstemps (emit_mkstemps: BCryptGenRandom +
            // CreateFileW CREATE_NEW over a marshaled template) → WriteFile →
            // FlushFileBuffers → CloseHandle → MoveFileExW rename; a failed rename
            // unlinks the temp (DeleteFileW). emit_write references GetStdHandle even
            // on the file-handle path (the console branch's reloc is always emitted).
            // (This arm was dropped by the stale-merge into P-66; restored.)
            "fs.writeTextAtomic" | "fs.writeBytesAtomic" => vec![
                import("BCryptGenRandom", BCRYPT, required_by),
                import("MultiByteToWideChar", KERNEL32, required_by),
                import("CreateFileW", KERNEL32, required_by),
                import("GetStdHandle", KERNEL32, required_by),
                import("WriteFile", KERNEL32, required_by),
                import("FlushFileBuffers", KERNEL32, required_by),
                import("CloseHandle", KERNEL32, required_by),
                import("MoveFileExW", KERNEL32, required_by),
                import("DeleteFileW", KERNEL32, required_by),
                import("GetLastError", KERNEL32, required_by),
            ],
            "fs.close" => vec![import("CloseHandle", KERNEL32, required_by)],
            "fs.readLine" | "fs.eof" => vec![
                import("ReadFile", KERNEL32, required_by),
                import("SetFilePointerEx", KERNEL32, required_by),
                import("GetLastError", KERNEL32, required_by),
            ],
            "fs.writeAll" | "fs.writeAllBytes" => vec![
                import("WriteFile", KERNEL32, required_by),
                import("SetFilePointerEx", KERNEL32, required_by),
                import("GetLastError", KERNEL32, required_by),
            ],
            "fs.flush" => vec![import("FlushFileBuffers", KERNEL32, required_by)],
            // Terminal queries (plan-47-G): GetConsoleMode succeeding IS isatty;
            // GetConsoleScreenBufferInfo gives the window size.
            "io.isInputTerminal" | "io.isOutputTerminal" | "io.isErrorTerminal" => vec![
                import("GetStdHandle", KERNEL32, required_by),
                import("GetConsoleMode", KERNEL32, required_by),
            ],
            // Terminal size AND the raw-mode line-discipline seam (the term module
            // links the raw-mode helper, whose isatty/tcgetattr/tcsetattr now route
            // to GetConsoleMode/SetConsoleMode via emit_terminal_control_call).
            // plan-66-D: on/off/sync emit ANSI via WriteFile (on also enables
            // ENABLE_VIRTUAL_TERMINAL_PROCESSING); terminalSize reads the buffer
            // info. The styling setters/getters + moveTo/clear touch only grid state
            // (no OS call → fall through). `term.sync`/WriteFile were dropped by the
            // stale merge; restored.
            "term.terminalSize" | "term.on" | "term.off" | "term.isOn" | "term.sync" => vec![
                import("GetStdHandle", KERNEL32, required_by),
                import("WriteFile", KERNEL32, required_by),
                import("GetConsoleMode", KERNEL32, required_by),
                import("SetConsoleMode", KERNEL32, required_by),
                import("GetConsoleScreenBufferInfo", KERNEL32, required_by),
            ],
            // Threads (plan-47-H): pthread_* -> CreateThread + SRWLOCK +
            // CONDITION_VARIABLE. Every thread.* helper may pull in any of the
            // sync primitives (they share the queue/broadcast machinery), so the
            // whole kernel32 set is declared; the merged import table dedups.
            // plan-98-D Phase 2: the graphics thread. `emit_thread_external_call`
            // translates each POSIX primitive it uses to these — SRWLOCK and
            // CONDITION_VARIABLE are pointer-sized and valid when zeroed, so they fit
            // the pthread-sized slots the shared code zeroes.
            "canvas.startGraphics"
            | "canvas.signalRedraw"
            | "canvas.waitForRedraw"
            | "canvas.frameDone"
            | "canvas.syncFrame"
            | "canvas.setSyncMode"
            | "canvas.setGpuMode"
            | "canvas.metalAvailable"
            | "canvas.vulkanReady"
            | "canvas.vulkanDrawScene"
            | "canvas.metalReady"
            | "canvas.metalDrawScene"
            | "canvas.useGpu"
            | "canvas.surfaceWidth"
            | "canvas.surfaceHeight" => vec![
                import("CreateThread", KERNEL32, required_by),
                import("WaitForSingleObject", KERNEL32, required_by),
                import("InitializeSRWLock", KERNEL32, required_by),
                import("AcquireSRWLockExclusive", KERNEL32, required_by),
                import("ReleaseSRWLockExclusive", KERNEL32, required_by),
                import("InitializeConditionVariable", KERNEL32, required_by),
                import("WakeConditionVariable", KERNEL32, required_by),
                import("WakeAllConditionVariable", KERNEL32, required_by),
                import("SleepConditionVariableSRW", KERNEL32, required_by),
                // plan-98-F Phase 3: the Vulkan loader arrives through
                // `LoadLibraryExA`/`GetProcAddress`, never an import-table entry on
                // `vulkan-1.dll` — a canvas program must still run on a Windows box
                // with no Vulkan installed, which is the same rule the Linux side
                // states for `dlopen`/`dlsym` (`linux_common/plan.rs`). Declared for
                // the whole canvas-graphics set rather than only `vulkanReady`, for
                // the reason given there: the merged table dedups, and scoping it
                // tighter means re-deriving which member reaches the loader every
                // time the renderer grows.
                import("LoadLibraryExA", KERNEL32, required_by),
                import("GetProcAddress", KERNEL32, required_by),
            ],
            call if call.starts_with("thread.") => {
                vec![
                    import("CreateThread", KERNEL32, required_by),
                    import("CloseHandle", KERNEL32, required_by),
                    import("WaitForSingleObject", KERNEL32, required_by),
                    import("InitializeSRWLock", KERNEL32, required_by),
                    import("AcquireSRWLockExclusive", KERNEL32, required_by),
                    import("ReleaseSRWLockExclusive", KERNEL32, required_by),
                    import("InitializeConditionVariable", KERNEL32, required_by),
                    import("WakeConditionVariable", KERNEL32, required_by),
                    import("WakeAllConditionVariable", KERNEL32, required_by),
                    import("SleepConditionVariableSRW", KERNEL32, required_by),
                    import("SwitchToThread", KERNEL32, required_by),
                    import("GetSystemTimePreciseAsFileTime", KERNEL32, required_by),
                    // The thread set keeps `Sleep`: `emit_windows_thread_call`
                    // reaches it from the shared relative-`nanosleep` block, and
                    // the queue helpers' backoff uses it directly.
                    import("Sleep", KERNEL32, required_by),
                    // `thread::openStdIn`/`closeStdIn` drive the same stdin-broadcast
                    // log as `io.input` (they share the broadcast machinery). Its
                    // growable buffer lives outside the arena and is malloc'd via the
                    // heap allocator (emit_heap_alloc/free → GetProcessHeap +
                    // HeapAlloc/HeapFree, plan-66-C), and the reader rides the console
                    // read/mode + pipe-probe surface. Declared with the rest of the
                    // thread set so every reloc resolves; the merged import table dedups
                    // against the io.input floor when both are reachable.
                    import("GetProcessHeap", KERNEL32, required_by),
                    import("HeapAlloc", KERNEL32, required_by),
                    import("HeapFree", KERNEL32, required_by),
                    import("GetStdHandle", KERNEL32, required_by),
                    import("ReadFile", KERNEL32, required_by),
                    import("WriteFile", KERNEL32, required_by),
                    import("GetConsoleMode", KERNEL32, required_by),
                    import("SetConsoleMode", KERNEL32, required_by),
                    import("GetLastError", KERNEL32, required_by),
                    import("GetFileType", KERNEL32, required_by),
                    import("PeekNamedPipe", KERNEL32, required_by),
                ]
            }
            // Networking (plan-47-I): every `net.*` helper is Winsock2 over
            // ws2_32.dll. WSAStartup/WSACleanup ride the entry (emit_net_startup),
            // but they belong to the same import set so the merged IAT carries them
            // whenever any socket call is reachable. `close`→`closesocket`,
            // `poll`→`WSAPoll`, the non-blocking toggle→`ioctlsocket`; the error
            // channel reuses kernel32's GetLastError (== WSAGetLastError on Win32).
            // plan-110-A: Windows has no unprivileged ICMP socket (Winsock's raw
            // ICMP needs Administrator), so ping rides iphlpapi's ICMP API, which
            // builds and matches the echo itself. It still needs Winsock for
            // getaddrinfo/inet_ntop, and QPC for the sub-millisecond round-trip
            // time (the API's own RoundTripTime is whole milliseconds and reads 0
            // on loopback). Matched before the general `net.` arm so ordinary
            // socket programs do not pull iphlpapi in.
            "net.ping" | "net.pingAddr" => vec![
                import("IcmpCreateFile", IPHLPAPI, required_by),
                import("IcmpSendEcho", IPHLPAPI, required_by),
                import("IcmpCloseHandle", IPHLPAPI, required_by),
                import("WSAStartup", WS2_32, required_by),
                import("WSACleanup", WS2_32, required_by),
                import("getaddrinfo", WS2_32, required_by),
                import("freeaddrinfo", WS2_32, required_by),
                import("inet_ntop", WS2_32, required_by),
                import("GetLastError", KERNEL32, required_by),
                import("QueryPerformanceCounter", KERNEL32, required_by),
                import("QueryPerformanceFrequency", KERNEL32, required_by),
            ],
            // plan-110-B: `tcp` is the same Winsock2 surface under a new package
            // name, so it takes the identical import set.
            call if call.starts_with("net.")
                || call.starts_with("tcp.")
                || call.starts_with("udp.") =>
            {
                vec![
                    import("WSAStartup", WS2_32, required_by),
                    import("WSACleanup", WS2_32, required_by),
                    import("socket", WS2_32, required_by),
                    import("connect", WS2_32, required_by),
                    import("bind", WS2_32, required_by),
                    import("listen", WS2_32, required_by),
                    import("accept", WS2_32, required_by),
                    import("recv", WS2_32, required_by),
                    import("send", WS2_32, required_by),
                    import("recvfrom", WS2_32, required_by),
                    import("sendto", WS2_32, required_by),
                    import("closesocket", WS2_32, required_by),
                    import("WSAPoll", WS2_32, required_by),
                    import("getaddrinfo", WS2_32, required_by),
                    import("freeaddrinfo", WS2_32, required_by),
                    import("setsockopt", WS2_32, required_by),
                    import("getsockopt", WS2_32, required_by),
                    import("getsockname", WS2_32, required_by),
                    import("getpeername", WS2_32, required_by),
                    import("inet_ntop", WS2_32, required_by),
                    import("ioctlsocket", WS2_32, required_by),
                    import("GetLastError", KERNEL32, required_by),
                    // A socket resource shares the File-record scope-drop and stream
                    // read/write glue: close is CloseHandle, and net.read/net.write reuse
                    // ReadFile/WriteFile — all valid on a SOCKET handle on Windows.
                    import("CloseHandle", KERNEL32, required_by),
                    import("ReadFile", KERNEL32, required_by),
                    import("WriteFile", KERNEL32, required_by),
                ]
            }
            // crypto:: NIST-EC over CNG/BCrypt (plan-47-J). randomBytes already
            // rides BCryptGenRandom in the entry floor; the EC ops pull the key/
            // hash/sign surface. Any crypto.* EC call declares the whole set; the
            // merged IAT dedups.
            "crypto.generate" | "crypto.sign" | "crypto.verify" => vec![
                import("BCryptOpenAlgorithmProvider", BCRYPT, required_by),
                import("BCryptCloseAlgorithmProvider", BCRYPT, required_by),
                import("BCryptGenerateKeyPair", BCRYPT, required_by),
                import("BCryptFinalizeKeyPair", BCRYPT, required_by),
                import("BCryptExportKey", BCRYPT, required_by),
                import("BCryptImportKeyPair", BCRYPT, required_by),
                import("BCryptDestroyKey", BCRYPT, required_by),
                import("BCryptHash", BCRYPT, required_by),
                import("BCryptSignHash", BCRYPT, required_by),
                import("BCryptVerifySignature", BCRYPT, required_by),
            ],
            "crypto.randomBytes" => vec![import("BCryptGenRandom", BCRYPT, required_by)],
            // plan-90-D: the process lifecycle over CreateProcessA. Over-importing
            // the whole kernel32 set for every process.* helper is harmless (the
            // merged IAT dedups).
            call if crate::codegen::registry::registry().owning_package(call)
                == Some("process")
                || call == "process.__drop" =>
            {
                vec![
                    import("CreateProcessA", KERNEL32, required_by),
                    import("CreatePipe", KERNEL32, required_by),
                    import("SetHandleInformation", KERNEL32, required_by),
                    // bug-499: the STARTUPINFOEXA handle list that limits what the
                    // child inherits to its three stdio pipe ends.
                    import("InitializeProcThreadAttributeList", KERNEL32, required_by),
                    import("UpdateProcThreadAttribute", KERNEL32, required_by),
                    import("DeleteProcThreadAttributeList", KERNEL32, required_by),
                    import("WriteFile", KERNEL32, required_by),
                    import("ReadFile", KERNEL32, required_by),
                    import("PeekNamedPipe", KERNEL32, required_by),
                    import("SetNamedPipeHandleState", KERNEL32, required_by),
                    import("GetTickCount64", KERNEL32, required_by),
                    import("Sleep", KERNEL32, required_by),
                    import("WaitForSingleObject", KERNEL32, required_by),
                    import("GetExitCodeProcess", KERNEL32, required_by),
                    import("TerminateProcess", KERNEL32, required_by),
                    import("CloseHandle", KERNEL32, required_by),
                    import("GetLastError", KERNEL32, required_by),
                    // plan-119-C: merge-mode `spawnEnv` reads the inherited
                    // environment as one block and gives it back.
                    import("GetEnvironmentStringsA", KERNEL32, required_by),
                    import("FreeEnvironmentStringsA", KERNEL32, required_by),
                ]
            }
            // WASAPI audio (plan-66 G+H). ole32 provides the COM runtime and object
            // activation (CoInitializeEx/CoCreateInstance/CoTaskMemFree); kernel32
            // provides the event-driven wait primitives. The IMMDevice*/IAudioClient*/
            // IAudio{Render,Capture}Client/IPropertyStore methods are called through
            // their COM vtables (an indirect `call r/m64`), so they need no import.
            // Any audio.* call declares the whole set; the merged IAT dedups.
            call if call.starts_with("audio.") => vec![
                import("CoInitializeEx", OLE32, required_by),
                import("CoCreateInstance", OLE32, required_by),
                import("CoTaskMemFree", OLE32, required_by),
                import("CreateEventW", KERNEL32, required_by),
                import("WaitForSingleObject", KERNEL32, required_by),
                import("CloseHandle", KERNEL32, required_by),
                import("GetTickCount64", KERNEL32, required_by),
            ],
            // TLS client over Schannel (plan-47-J): SSPI (secur32), the cert-chain
            // policy check (crypt32), the socket layer (ws2_32), and the wide-string
            // marshal for the SNI/target name (kernel32). Any tls.* call declares
            // the whole set; the merged IAT dedups.
            call if call.starts_with("tls.") => vec![
                // WSAStartup/WSACleanup ride the entry (needs_winsock is set for
                // tls too, since the Schannel client opens its own raw socket).
                import("WSAStartup", WS2_32, required_by),
                import("WSACleanup", WS2_32, required_by),
                import("AcquireCredentialsHandleW", SECUR32, required_by),
                import("FreeCredentialsHandle", SECUR32, required_by),
                import("InitializeSecurityContextW", SECUR32, required_by),
                // Server handshake (tls.listen/accept): AcceptSecurityContext has
                // no A/W variant (it takes no string args).
                import("AcceptSecurityContext", SECUR32, required_by),
                import("DeleteSecurityContext", SECUR32, required_by),
                import("FreeContextBuffer", SECUR32, required_by),
                import("QueryContextAttributesW", SECUR32, required_by),
                import("ApplyControlToken", SECUR32, required_by),
                import("EncryptMessage", SECUR32, required_by),
                import("DecryptMessage", SECUR32, required_by),
                import("CertGetCertificateChain", CRYPT32, required_by),
                import("CertVerifyCertificateChainPolicy", CRYPT32, required_by),
                import("CertFreeCertificateChain", CRYPT32, required_by),
                import("CertFreeCertificateContext", CRYPT32, required_by),
                // Server credential build: PEM → DER (CryptStringToBinaryA), the
                // cert context, the PKCS#8→PKCS#1 decode, and the property that
                // binds the private key to the cert.
                import("CryptStringToBinaryA", CRYPT32, required_by),
                import("CertCreateCertificateContext", CRYPT32, required_by),
                import("CertSetCertificateContextProperty", CRYPT32, required_by),
                import("CryptDecodeObjectEx", CRYPT32, required_by),
                // Legacy CryptoAPI ephemeral private-key import (advapi32): the
                // CryptImportKey-into-VERIFYCONTEXT + CERT_KEY_CONTEXT recipe.
                import("CryptAcquireContextW", ADVAPI32, required_by),
                import("CryptImportKey", ADVAPI32, required_by),
                import("CryptDestroyKey", ADVAPI32, required_by),
                import("CryptReleaseContext", ADVAPI32, required_by),
                import("getaddrinfo", WS2_32, required_by),
                import("freeaddrinfo", WS2_32, required_by),
                import("socket", WS2_32, required_by),
                import("connect", WS2_32, required_by),
                // Server socket: bind/listen/accept + the SO_REUSEADDR toggle and
                // the connection-wait poll.
                import("bind", WS2_32, required_by),
                import("listen", WS2_32, required_by),
                import("accept", WS2_32, required_by),
                import("setsockopt", WS2_32, required_by),
                // plan-73-D: the client's non-blocking connect + WSAPoll timeout path
                // needs ioctlsocket(FIONBIO) and getsockopt(SO_ERROR).
                import("getsockopt", WS2_32, required_by),
                import("ioctlsocket", WS2_32, required_by),
                import("WSAPoll", WS2_32, required_by),
                import("send", WS2_32, required_by),
                import("recv", WS2_32, required_by),
                import("closesocket", WS2_32, required_by),
                // plan-110-D: tls.localAddress/tls.remoteAddress. Schannel layers
                // over a plain SOCKET kept in the record's handle slot, so the
                // endpoint queries reuse the `net` address emitter verbatim.
                import("getsockname", WS2_32, required_by),
                import("getpeername", WS2_32, required_by),
                import("inet_ntop", WS2_32, required_by),
                // The PEM cert/key files are read via the Win32 file API.
                import("CreateFileW", KERNEL32, required_by),
                import("ReadFile", KERNEL32, required_by),
                import("CloseHandle", KERNEL32, required_by),
                import("MultiByteToWideChar", KERNEL32, required_by),
                import("GetLastError", KERNEL32, required_by),
            ],
            _ => Vec::new(),
        }
    }

    fn native_call_imports(&self, _target: &str, _required_by: &str) -> Vec<PlatformImport> {
        Vec::new()
    }

    fn link_imports(&self, required_by: &str) -> Vec<PlatformImport> {
        // bug-431: the runtime loader `_mfb_linker_init` emits `LoadLibraryExA`
        // + `GetProcAddress` (kernel32) instead of the POSIX `dlopen`/`dlsym`, and
        // resolves a vendored DLL from the exe-relative `vendor/` directory by
        // building `<exe_dir>\vendor\<name>` with `GetModuleFileNameA` +
        // `PathRemoveFileSpecA` (shlwapi) + `lstrcatA` (kernel32). Declared
        // unconditionally alongside the LINK support, which is itself only emitted
        // when the program has `LINK` bindings — a non-LINK Windows build declares
        // none of these and stays byte-identical.
        [
            ("LoadLibraryExA", KERNEL32),
            ("GetProcAddress", KERNEL32),
            ("GetModuleFileNameA", KERNEL32),
            ("lstrcatA", KERNEL32),
            ("PathRemoveFileSpecA", SHLWAPI),
        ]
        .into_iter()
        .map(|(symbol, library)| import(symbol, library, required_by))
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// bug-431: a Windows build that vendors a native `LINK` library needs the
    /// kernel32/shlwapi loader imports so `_mfb_linker_init` can resolve the DLL
    /// at runtime. Before the fix `link_imports` returned nothing, leaving the
    /// loader unbindable (there was not even a `dlopen` symbol to reference).
    #[test]
    fn link_imports_declare_the_win32_loader() {
        let imports = Platform.link_imports("_main");
        let symbols: std::collections::HashSet<&str> =
            imports.iter().map(|i| i.symbol.as_str()).collect();
        for required in [
            "LoadLibraryExA",
            "GetProcAddress",
            "GetModuleFileNameA",
            "lstrcatA",
            "PathRemoveFileSpecA",
        ] {
            assert!(
                symbols.contains(required),
                "win link_imports missing {required}; got {symbols:?}"
            );
        }
    }
}
