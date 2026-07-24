//! Windows x86-64 codegen platform (plan-47-D, minimal machine floor).
//!
//! A standalone `CodegenPlatform` for the Windows OS (mirroring macOS's shape,
//! not `linux_common`'s arch-parameterized one). It reuses the x86-64 ISA
//! selection/encoder unchanged (via [`Win64Backend`]) and realizes the OS
//! primitives as `kernel32` IAT calls: the arena maps with `VirtualAlloc` /
//! `VirtualFree`, the program exits with `ExitProcess`. There is NO syscall path.
//!
//! CodeInstructions are built with the neutral `abi::*` builders and role tokens;
//! `remap_x86_abi(_, Win64)` realizes them to the Win64 homes (rcx/rdx/r8/r9).
//!
//! Scope is the machine floor only (entry/exit/arena + the `emit_libc_call` IAT
//! seam). Every other surface — fs, terminal, threads, sockets, TLS — is a
//! deliberate stub: the backend advertises a minimal `runtime_calls`, so a
//! program using an unimplemented surface is rejected at the capability gate, and
//! these methods are never reached. The POSIX-struct constant accessors return a
//! placeholder (Windows has no such structs; 47-E raises that seam). None of
//! these placeholders is reachable until a later sub-plan advertises its surface.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::arch::aarch64::abi;
use crate::arch::x86_64::backend::WIN64_BACKEND;
use crate::target::shared::code::{
    self, CodeDataObject, CodeFunction, CodeInstruction, CodeRelocation, FsPathOperation, MirPlan,
    NativeCodePlan, ProgramEntrySpec, RelocIntent,
};
use crate::target::shared::nir::NirModule;
use crate::target::shared::plan::NativePlan;

const KERNEL32: &str = "kernel32.dll";
// VirtualAlloc flAllocationType = MEM_COMMIT (0x1000) | MEM_RESERVE (0x2000).
const MEM_COMMIT_RESERVE: &str = "12288";
// VirtualAlloc flProtect = PAGE_READWRITE (0x04).
const PAGE_READWRITE: &str = "4";
// VirtualFree dwFreeType = MEM_RELEASE (0x8000).
const MEM_RELEASE: &str = "32768";

pub(crate) fn lower_module(
    module: &NirModule,
    native_plan: &NativePlan,
    packages: &[PathBuf],
) -> Result<NativeCodePlan, String> {
    code::lower_module_for_platform(module, native_plan, packages, &Platform)
}

pub(crate) fn lower_module_mir(
    module: &NirModule,
    native_plan: &NativePlan,
    packages: &[PathBuf],
) -> Result<MirPlan, String> {
    code::lower_module_mir_for_platform(module, native_plan, packages, &Platform)
}

struct Platform;

/// Push an external `kernel32`-style call whose reloc names the DLL directly
/// (the trait methods that need it carry no `platform_imports`, exactly like the
/// macOS `_exit`/import path). The x86 encoder additionally auto-relocates any
/// `bl <imported symbol>` from the plan's import set, but naming the library here
/// keeps the reloc self-describing.
fn call_external(
    from: &str,
    symbol: &str,
    library: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    instructions.push(abi::branch_link(symbol));
    relocations.push(CodeRelocation {
        from: from.to_string(),
        to: symbol.to_string(),
        kind: RelocIntent::Call,
        binding: "external".to_string(),
        library: Some(library.to_string()),
    });
}

/// Every surface this floor does not implement rejects loudly if it is ever
/// reached — which it cannot be, since the backend does not advertise it.
fn unsupported(surface: &str) -> String {
    format!("windows-x86_64: the {surface} surface is not yet implemented (plan-47-D..J)")
}

impl code::CodegenPlatform for Platform {
    fn target(&self) -> &'static str {
        "windows-x86_64"
    }

    fn arch(&self) -> &'static str {
        "x86_64"
    }

    fn backend(&self) -> &'static dyn code::mir::Backend {
        // Wires the Win64 ABI backend (plan-47-B A1) — the production consumer
        // that removes A1's dead-code allows.
        &WIN64_BACKEND
    }

    fn entry_stack_misaligned_on_entry(&self) -> bool {
        // The PE loader `call`s the image entry, so it arrives at `sp % 16 == 8`;
        // the shared preamble realigns with one `sub rsp, 8`.
        true
    }

    // --- the machine floor -------------------------------------------------

    fn emit_program_entry(
        &self,
        spec: &ProgramEntrySpec<'_>,
        platform_imports: &HashMap<String, String>,
    ) -> Result<CodeFunction, String> {
        code::lower_program_entry(
            spec.entry_symbol,
            spec.language_entry_symbol,
            spec.language_entry_returns,
            spec.language_entry_accepts_args,
            spec.global_initializer_symbol,
            spec.link_init_symbol,
            spec.closure_init_symbol,
            spec.entry_stack_size,
            spec.global_slot_count,
            platform_imports,
            self,
            spec.emit_cleanup_failure_audit,
            spec.seed_rng,
            spec.register_signal_handlers,
            spec.capture_args,
            spec.subscribe_stdin,
            spec.entry_called_as_function,
        )
    }

    fn emit_program_exit(
        &self,
        from: &str,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // ExitProcess(uExitCode): the exit code arrives in the neutral `x0`,
        // which the Win64 remap realizes to `rcx` (arg 0). Never returns.
        call_external(from, "ExitProcess", KERNEL32, instructions, relocations);
        instructions.extend([abi::branch_self(), abi::return_()]);
        Ok(())
    }

    fn emit_arena_start_time(
        &self,
        entry_symbol: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // GetSystemTimePreciseAsFileTime(&FILETIME): writes an 8-byte FILETIME
        // (100ns intervals since 1601) — a fine start-time seed — into a 16-byte
        // stack buffer left allocated for the entry's entropy block (plan-47-D
        // §3.1). Mirrors the default's buffer contract exactly.
        instructions.extend([
            abi::subtract_stack(16),
            abi::add_immediate(abi::ARG[0], abi::stack_pointer(), 0),
        ]);
        call_external(
            entry_symbol,
            "GetSystemTimePreciseAsFileTime",
            KERNEL32,
            instructions,
            relocations,
        );
        instructions.extend([
            abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), 0),
            abi::store_u64(
                abi::SCRATCH[0],
                code::ARENA_STATE_REGISTER,
                code::ARENA_START_TIME_OFFSET,
            ),
            // Balance the buffer reservation (matching the default's
            // `subtract_stack(16)`/`add_stack(16)` contract) so the entry's stack
            // pointer is unchanged across the seed.
            abi::add_stack(16),
        ]);
        Ok(())
    }

    fn emit_arena_map(
        &self,
        size_reg: &str,
        instructions: &mut Vec<CodeInstruction>,
    ) -> Result<(), String> {
        // VirtualAlloc(NULL, size, MEM_COMMIT|MEM_RESERVE, PAGE_READWRITE).
        // Args in the neutral x0..x3 → Win64 rcx/rdx/r8/r9. The reloc is
        // auto-generated by the encoder from the plan's VirtualAlloc import.
        instructions.extend([
            abi::move_immediate(abi::ARG[0], "Integer", "0"),
            abi::move_register(abi::ARG[1], size_reg),
            abi::move_immediate(abi::ARG[2], "Integer", MEM_COMMIT_RESERVE),
            abi::move_immediate(abi::ARG[3], "Integer", PAGE_READWRITE),
            abi::branch_link("VirtualAlloc"),
            // VirtualAlloc returns NULL(0) on failure; the shared arena caller
            // routes a *negative* result to the OOM path (the negative-errno
            // convention the Linux backend returns), so normalize 0 → -1.
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_ne("arena_map_succeeded"),
            abi::bitwise_not(abi::return_register(), abi::ZERO),
            abi::label("arena_map_succeeded"),
        ]);
        Ok(())
    }

    fn emit_arena_unmap(&self, instructions: &mut Vec<CodeInstruction>) -> Result<(), String> {
        // VirtualFree(lpAddress, 0, MEM_RELEASE). `arena_destroy` hands the block
        // address in `return_register()` (the neutral x0 slot, where the Linux
        // `munmap` syscall reads its first arg). On Win64 the first arg is rcx
        // (ARG[0]) while return_register() is rax — distinct registers — so move
        // the address into ARG[0] before the call. VirtualFree requires
        // dwSize == 0 with MEM_RELEASE.
        instructions.extend([
            abi::move_register(abi::ARG[0], abi::return_register()),
            abi::move_immediate(abi::ARG[1], "Integer", "0"),
            abi::move_immediate(abi::ARG[2], "Integer", MEM_RELEASE),
            abi::branch_link("VirtualFree"),
        ]);
        Ok(())
    }

    fn emit_libc_call(
        &self,
        base: &str,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // The IAT-call seam every OS call rides (master §3): a `bl <symbol>` +
        // external reloc, the DLL taken from the plan's import map. Identical to
        // the shared Linux emitter; only the import *library* differs (kernel32/
        // bcrypt vs libc), and that lives in `platform_imports`.
        crate::target::linux_common::code::emit_linux_c_call(
            from,
            base,
            platform_imports,
            instructions,
            relocations,
        )
    }

    fn emit_variadic_call(
        &self,
        base: &str,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // Win64 has no separate variadic marker (unlike SysV's `al`); a plain
        // call suffices.
        self.emit_libc_call(base, from, platform_imports, instructions, relocations)
    }

    // --- surfaces owned by later sub-plans (unreachable stubs) -------------

    fn emit_write(
        &self,
        from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // write(fd, buf, len): fd in x0, buf in x1, len in x2. Windows has no fd —
        // resolve the POSIX fd to a console HANDLE via GetStdHandle, then WriteFile.
        //   fd 1 (stdout) → STD_OUTPUT_HANDLE (-11); fd 2 (stderr) → STD_ERROR_HANDLE
        //   (-12); i.e. nStdHandle = -10 - fd.
        //
        // NOTE (plan-47-D machine floor): this emits the 4-register-arg form
        // (handle, buf, len, lpNumberOfBytesWritten=NULL) and omits WriteFile's 5th
        // (stack) argument lpOverlapped. That is correct for the only path that
        // *reaches* this in the minimal floor — the entry's dead error tail, which
        // an integer-returning program never executes. `io.print`'s fully-correct
        // write (a bytesWritten slot + the lpOverlapped stack arg via the outgoing
        // tail) lands with the console surface, tested on the Win11 box.
        instructions.extend([
            abi::move_register(abi::SCRATCH[0], abi::ARG[1]), // save buf
            abi::move_register(abi::SCRATCH[1], abi::ARG[2]), // save len
            // nStdHandle = -(fd + 10)  (fd 1 → -11 STD_OUTPUT, fd 2 → -12 STD_ERROR);
            // computed without a negative immediate (the encoder rejects those).
            abi::add_immediate(abi::SCRATCH[2], abi::ARG[0], 10), // fd + 10
            abi::move_immediate(abi::ARG[0], "Integer", "0"),
            abi::subtract_registers(abi::ARG[0], abi::ARG[0], abi::SCRATCH[2]), // -(fd+10)
        ]);
        call_external(from, "GetStdHandle", KERNEL32, instructions, relocations);
        instructions.extend([
            abi::move_register(abi::ARG[0], abi::return_register()), // hFile = handle
            abi::move_register(abi::ARG[1], abi::SCRATCH[0]),        // lpBuffer
            abi::move_register(abi::ARG[2], abi::SCRATCH[1]),        // nNumberOfBytesToWrite
            abi::move_immediate(abi::ARG[3], "Integer", "0"),        // lpNumberOfBytesWritten
        ]);
        call_external(from, "WriteFile", KERNEL32, instructions, relocations);
        Ok(())
    }

    fn emit_poll_input(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Err(unsupported("console input"))
    }

    fn emit_is_terminal(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Err(unsupported("is-terminal"))
    }

    fn emit_terminal_size(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Err(unsupported("terminal size"))
    }

    fn emit_path_exists(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Err(unsupported("filesystem"))
    }

    fn emit_path_stat(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Err(unsupported("filesystem"))
    }

    fn emit_current_directory(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Err(unsupported("filesystem"))
    }

    fn emit_environ_pointer(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Err(unsupported("environment"))
    }

    fn emit_fs_path_operation(
        &self,
        _from: &str,
        _operation: FsPathOperation,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Err(unsupported("filesystem"))
    }

    fn emit_errno(
        &self,
        _from: &str,
        _dst: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Err(unsupported("errno (GetLastError)"))
    }

    fn emit_open_file(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Err(unsupported("filesystem"))
    }

    fn emit_read_file(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Err(unsupported("filesystem"))
    }

    fn emit_close_file(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Err(unsupported("filesystem"))
    }

    fn emit_sync_file(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Err(unsupported("filesystem"))
    }

    fn emit_seek_file(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Err(unsupported("filesystem"))
    }

    fn emit_rename_path(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Err(unsupported("filesystem"))
    }

    fn emit_mkstemps(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Err(unsupported("filesystem"))
    }

    fn emit_random_bytes(
        &self,
        from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // On entry (the shared convention): the buffer pointer is in the neutral
        // `x0` and the length in `x1`. BCryptGenRandom(hAlgorithm, pbBuffer,
        // cbBuffer, dwFlags) takes 4 Win64 args (rcx/rdx/r8/r9), so shuffle:
        // hAlgorithm=NULL, pbBuffer=buf, cbBuffer=len,
        // dwFlags=BCRYPT_USE_SYSTEM_PREFERRED_RNG (0x02). The shared caller only
        // reads the buffer afterward, so the NTSTATUS return is ignored (the seed
        // scratch was pre-filled with the arena address as a fallback).
        const BCRYPT: &str = "bcrypt.dll";
        const BCRYPT_USE_SYSTEM_PREFERRED_RNG: &str = "2";
        instructions.extend([
            abi::move_register(abi::SCRATCH[0], abi::ARG[0]), // save buf (x0)
            abi::move_register(abi::SCRATCH[1], abi::ARG[1]), // save len (x1)
            abi::move_immediate(abi::ARG[0], "Integer", "0"), // hAlgorithm = NULL
            abi::move_register(abi::ARG[1], abi::SCRATCH[0]), // pbBuffer
            abi::move_register(abi::ARG[2], abi::SCRATCH[1]), // cbBuffer
            abi::move_immediate(abi::ARG[3], "Integer", BCRYPT_USE_SYSTEM_PREFERRED_RNG),
        ]);
        call_external(from, "BCryptGenRandom", BCRYPT, instructions, relocations);
        Ok(())
    }

    fn emit_temp_directory(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Err(unsupported("filesystem"))
    }

    fn emit_opendir(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Err(unsupported("directory iteration"))
    }

    fn emit_readdir(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Err(unsupported("directory iteration"))
    }

    fn emit_closedir(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Err(unsupported("directory iteration"))
    }

    fn emit_realpath(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        Err(unsupported("filesystem"))
    }

    // --- POSIX-struct constant accessors ----------------------------------
    // Windows has no termios/dirent/stat/addrinfo structs; 47-E raises this seam
    // to intent-level methods. Unreachable until a later sub-plan advertises the
    // surface, so a placeholder 0 is safe (and never read).

    fn termios_size(&self) -> usize {
        0
    }
    fn termios_lflag_offset(&self) -> usize {
        0
    }
    fn termios_lflag_width(&self) -> usize {
        0
    }
    fn termios_cc_offset(&self) -> usize {
        0
    }
    fn termios_echo_flag(&self) -> u64 {
        0
    }
    fn termios_icanon_flag(&self) -> u64 {
        0
    }
    fn termios_vmin_index(&self) -> usize {
        0
    }
    fn termios_vtime_index(&self) -> usize {
        0
    }
    fn stat_mode_offset(&self) -> usize {
        0
    }
    fn dirent_name_offset(&self) -> usize {
        0
    }
    fn dirent_name_length_offset(&self) -> usize {
        0
    }
    fn addrinfo_addr_offset(&self) -> usize {
        0
    }
    fn sol_socket(&self) -> &'static str {
        "0"
    }
    fn so_reuseaddr(&self) -> &'static str {
        "0"
    }
    fn so_rcvtimeo(&self) -> &'static str {
        "0"
    }
    fn so_sndtimeo(&self) -> &'static str {
        "0"
    }
    fn eagain(&self) -> &'static str {
        "0"
    }
    fn emsgsize(&self) -> &'static str {
        "0"
    }
    fn o_nonblock(&self) -> &'static str {
        "0"
    }
    fn einprogress(&self) -> &'static str {
        "0"
    }
    fn so_error(&self) -> &'static str {
        "0"
    }

    // --- threads / TLS (owned by 47-H/47-J) -------------------------------

    fn emit_thread_trampoline(
        &self,
        _platform_imports: &HashMap<String, String>,
        _uses_stdin: bool,
        _arena_init: code::ArenaInitSymbols,
    ) -> Result<CodeFunction, String> {
        Err(unsupported("threads (CreateThread)"))
    }

    fn app_mode_data_objects(&self, _project_name: &str) -> Vec<CodeDataObject> {
        // Console subsystem only (master §Non-goals); no app-mode data objects.
        Vec::new()
    }
}
