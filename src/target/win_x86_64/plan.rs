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

    fn entry_imports(&self, _module: &NirModule) -> Vec<PlatformImport> {
        // The entry maps/unmaps the arena (VirtualAlloc/VirtualFree), seeds the
        // arena start time (GetSystemTimePreciseAsFileTime) and the always-on
        // memory-fill RNG (BCryptGenRandom, bcrypt.dll).
        vec![
            import("VirtualAlloc", KERNEL32, "_start"),
            import("VirtualFree", KERNEL32, "_start"),
            import("GetSystemTimePreciseAsFileTime", KERNEL32, "_start"),
            import("BCryptGenRandom", "bcrypt.dll", "_start"),
            // The entry's implicit program exit (emit_program_exit) — the NIR of a
            // plain `RETURN` has no ExitProgram op, so this import rides the entry,
            // not `program_exit_imports`.
            import("ExitProcess", KERNEL32, "_start"),
        ]
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

    fn runtime_imports(&self, spec: &RuntimeHelperSpec) -> Vec<PlatformImport> {
        let required_by = crate::target::shared::runtime::symbol_for_call(spec.helper, spec.call);
        let required_by = required_by.as_str();
        // Every path-taking fs helper marshals UTF-8 → UTF-16 (MultiByteToWideChar)
        // before its `*W` Win32 call (plan-47-F §3.4).
        match spec.call {
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
            "fs.canonicalPath" => vec![
                import("MultiByteToWideChar", KERNEL32, required_by),
                import("GetFullPathNameW", KERNEL32, required_by),
                import("WideCharToMultiByte", KERNEL32, required_by),
                import("GetLastError", KERNEL32, required_by),
            ],
            // The File-resource surface: openFile yields a resource holding the
            // CreateFileW handle; the per-resource ops reuse ReadFile / WriteFile /
            // SetFilePointerEx / FlushFileBuffers / CloseHandle.
            "fs.openFile" | "fs.open" | "fs.openFileNoFollow" => vec![
                import("MultiByteToWideChar", KERNEL32, required_by),
                import("CreateFileW", KERNEL32, required_by),
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
            "term.terminalSize" | "term.on" | "term.off" | "term.isOn" => vec![
                import("GetStdHandle", KERNEL32, required_by),
                import("GetConsoleMode", KERNEL32, required_by),
                import("SetConsoleMode", KERNEL32, required_by),
                import("GetConsoleScreenBufferInfo", KERNEL32, required_by),
            ],
            _ => Vec::new(),
        }
    }

    fn native_call_imports(&self, _target: &str, _required_by: &str) -> Vec<PlatformImport> {
        Vec::new()
    }

    fn link_imports(&self, _required_by: &str) -> Vec<PlatformImport> {
        Vec::new()
    }
}
