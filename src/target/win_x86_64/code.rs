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
// MultiByteToWideChar CodePage = CP_UTF8 (65001).
const CP_UTF8: &str = "65001";

/// A UTF-16 path-marshaling frame for a Win32 `*W` filesystem call. The path
/// arrives as a NUL-terminated UTF-8 C-string; every path-taking Win32 API is the
/// wide (`W`) variant, so each call converts UTF-8 → UTF-16 via
/// `MultiByteToWideChar` first (plan-47-F §3.4). The 64 KiB (32767-wchar, Windows'
/// own max path length) buffer is allocated from the ARENA, not the stack: a large
/// `sub rsp` would skip the Windows stack guard page and fault on first write
/// (there is no inline `__chkstk` in this codegen). Only a tiny outgoing frame is
/// reserved on the stack — layout relative to `sp` after `subtract_stack`:
///   [0x00 .. 0x20)  shadow space for the callee
///   [0x20]          MultiByteToWideChar 5th arg (lpWideCharStr) — a stack arg
///   [0x28]          MultiByteToWideChar 6th arg (cchWideChar)   — a stack arg
///   [0x30]          saved UTF-8 path pointer (survives the arena/convert calls)
///   [0x38]          the arena UTF-16 buffer pointer (the caller reads this)
const MARSHAL_FRAME: usize = 0x40;
const MARSHAL_WBUF_SLOT: usize = 0x38;

/// Emit an arena allocation of the UTF-16 buffer and
/// `MultiByteToWideChar(CP_UTF8, 0, path, -1, wbuf, 32768)` into a
/// [`MARSHAL_FRAME`] the caller has already reserved with `subtract_stack`. On
/// entry the UTF-8 path pointer is in `ARG[0]`; on return the wide string's arena
/// pointer is at `sp + MARSHAL_WBUF_SLOT` (and also live in `ARG[0]`... clobbered —
/// the caller reloads from the slot).
fn emit_marshal_path(
    from: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    instructions.extend([
        abi::store_u64(abi::ARG[0], abi::stack_pointer(), 0x30), // save path
        // _mfb_arena_alloc(size = 65536, align = 2) -> RET[1] = buffer pointer.
        // A 64 KiB request never OOMs in practice (the arena maps fresh 1 MiB+
        // blocks via VirtualAlloc), so the Result tag is not checked here.
        abi::move_immediate(abi::return_register(), "Integer", "65536"),
        abi::move_immediate(abi::ARG[1], "Integer", "2"),
        abi::branch_link(code::ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: from.to_string(),
        to: code::ARENA_ALLOC_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::store_u64(abi::RET[1], abi::stack_pointer(), MARSHAL_WBUF_SLOT), // save wbuf
        abi::move_immediate(abi::ARG[0], "Integer", CP_UTF8),
        abi::move_immediate(abi::ARG[1], "Integer", "0"), // dwFlags
        // Stage the two stack args using ARG[2] as a scratch BEFORE it is set to its
        // real register value (the path). ARG[2] (rdx→r8) is caller-saved; the
        // machine-floor SCRATCH pool must NOT be used here — on Win64 its low slots
        // realize to callee-saved rbx/rsi/rdi, so writing them corrupts registers
        // the caller keeps live (map_scratch_register's documented Win64 hazard).
        abi::load_u64(abi::ARG[2], abi::stack_pointer(), MARSHAL_WBUF_SLOT), // wbuf (temp)
        abi::store_u64(abi::ARG[2], abi::stack_pointer(), 0x20), // lpWideCharStr (5th)
        abi::move_immediate(abi::ARG[2], "Integer", "32768"),
        abi::store_u64(abi::ARG[2], abi::stack_pointer(), 0x28), // cchWideChar (6th)
        abi::load_u64(abi::ARG[2], abi::stack_pointer(), 0x30), // lpMultiByteStr = path
        // cbMultiByte = -1 (the input is NUL-terminated); the encoder rejects a
        // negative immediate, so build it as 0 - 1.
        abi::move_immediate(abi::ARG[3], "Integer", "0"),
        abi::subtract_immediate(abi::ARG[3], abi::ARG[3], 1),
    ]);
    call_external(from, "MultiByteToWideChar", KERNEL32, instructions, relocations);
}

/// A reverse-marshaling frame for a Win32 `*W` call that PRODUCES a UTF-16 path
/// (GetCurrentDirectoryW / GetTempPathW / GetFullPathNameW). The wide result is
/// converted back to UTF-8 into the caller's arena buffer via WideCharToMultiByte.
/// Layout relative to `sp` after `subtract_stack(RMARSHAL_FRAME)`:
///   [0x00 .. 0x20)  shadow space
///   [0x20]          WideCharToMultiByte 5th arg (lpMultiByteStr = UTF-8 dst)
///   [0x28]          WideCharToMultiByte 6th arg (cbMultiByte    = capacity)
///   [0x30]          WideCharToMultiByte 7th arg (lpDefaultChar  = NULL)
///   [0x38]          WideCharToMultiByte 8th arg (lpUsedDefault  = NULL)
///   [0x40]          saved UTF-8 destination buffer pointer
///   [0x48]          saved destination capacity (bytes)
///   [0x50]          the arena UTF-16 scratch buffer pointer
const RMARSHAL_FRAME: usize = 0x60;
const RMARSHAL_DST_SLOT: usize = 0x40;
const RMARSHAL_CAP_SLOT: usize = 0x48;
const RMARSHAL_WBUF_SLOT: usize = 0x50;

/// Windows directory-iteration "DIR" structure (arena-allocated by emit_opendir).
/// POSIX `opendir` yields a handle and the first `readdir` fetches the first
/// entry, but `FindFirstFileW` RETURNS the first entry along with the search
/// handle (plan-47-F §risk). So the DIR carries a `first-pending` flag: the first
/// `readdir` consumes the already-fetched entry, later ones call `FindNextFileW`.
/// Layout:
///   [0x00]  FindFirstFileW search HANDLE
///   [0x08]  first-entry-pending flag (1 after opendir, 0 after the first readdir)
///   [0x10]  WIN32_FIND_DATAW (592 bytes); cFileName (WCHAR[260]) at +44 = 0x2c
///   [0x260] UTF-8 name buffer (the converted cFileName; read_dir_entry reads here)
const DIR_HANDLE_OFF: usize = 0x00;
const DIR_FIRST_OFF: usize = 0x08;
const DIR_FINDDATA_OFF: usize = 0x10;
const DIR_CFILENAME_OFF: usize = DIR_FINDDATA_OFF + 0x2c; // 0x3c
const DIR_NAME_OFF: usize = 0x260; // after 0x10 + 592 (0x250), rounded to 0x260
const DIR_SIZE: &str = "2144"; // 0x260 + 1024 name buffer, rounded
const DIR_NAME_CAP: &str = "1024";

/// Emit `WideCharToMultiByte(CP_UTF8, 0, wbuf, -1, dst, capacity, NULL, NULL)`,
/// converting the UTF-16 buffer at `sp + RMARSHAL_WBUF_SLOT` into the UTF-8 dest
/// at `sp + RMARSHAL_DST_SLOT` (capacity at `sp + RMARSHAL_CAP_SLOT`). Returns the
/// byte count in the return register (0 on failure).
fn emit_wide_to_utf8(
    from: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    instructions.extend([
        abi::move_immediate(abi::ARG[0], "Integer", CP_UTF8),
        abi::move_immediate(abi::ARG[1], "Integer", "0"), // dwFlags
        // Stage the four stack args using ARG[2] as a caller-saved scratch before
        // it is set to its register value (lpWideCharStr).
        abi::load_u64(abi::ARG[2], abi::stack_pointer(), RMARSHAL_DST_SLOT),
        abi::store_u64(abi::ARG[2], abi::stack_pointer(), 0x20), // lpMultiByteStr (5th)
        abi::load_u64(abi::ARG[2], abi::stack_pointer(), RMARSHAL_CAP_SLOT),
        abi::store_u64(abi::ARG[2], abi::stack_pointer(), 0x28), // cbMultiByte (6th)
        abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x30),   // lpDefaultChar (7th)
        abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x38),   // lpUsedDefaultChar (8th)
        abi::load_u64(abi::ARG[2], abi::stack_pointer(), RMARSHAL_WBUF_SLOT), // lpWideCharStr
        abi::move_immediate(abi::ARG[3], "Integer", "0"),
        abi::subtract_immediate(abi::ARG[3], abi::ARG[3], 1), // cchWideChar = -1
    ]);
    call_external(from, "WideCharToMultiByte", KERNEL32, instructions, relocations);
}

/// Emit a directory-path query (GetCurrentDirectoryW / GetTempPathW), both of
/// which take `(nBufferLength: DWORD, lpBuffer)` and write a UTF-16 path. The
/// arena UTF-8 destination buffer is in ARG[0] and its capacity in ARG[1]. The
/// two shared callers differ in what they expect back: `currentDirectory`
/// strlen's a returned BUFFER POINTER, while `tempDirectory` copies `return`
/// bytes from a pre-parked buffer — so `return_length` selects the UTF-8 byte
/// length (excluding the NUL) instead of the pointer. 0 on failure either way.
fn emit_dir_path_query(
    from: &str,
    symbol: &str,
    return_length: bool,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let n = instructions.len();
    let fail = format!("{from}_dirq_fail_{n}");
    let done = format!("{from}_dirq_done_{n}");
    instructions.extend([
        abi::subtract_stack(RMARSHAL_FRAME),
        abi::store_u64(abi::ARG[0], abi::stack_pointer(), RMARSHAL_DST_SLOT), // dst
        abi::store_u64(abi::ARG[1], abi::stack_pointer(), RMARSHAL_CAP_SLOT), // capacity
        // arena UTF-16 scratch (64 KiB, 32767 wchars = Windows max path).
        abi::move_immediate(abi::return_register(), "Integer", "65536"),
        abi::move_immediate(abi::ARG[1], "Integer", "2"),
        abi::branch_link(code::ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: from.to_string(),
        to: code::ARENA_ALLOC_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::store_u64(abi::RET[1], abi::stack_pointer(), RMARSHAL_WBUF_SLOT),
        abi::move_immediate(abi::ARG[0], "Integer", "32768"), // nBufferLength (wchars)
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), RMARSHAL_WBUF_SLOT), // lpBuffer
    ]);
    call_external(from, symbol, KERNEL32, instructions, relocations);
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&fail), // 0 chars written → failure
    ]);
    emit_wide_to_utf8(from, instructions, relocations);
    // On success WideCharToMultiByte left the UTF-8 byte count (including the NUL)
    // in the return register.
    if return_length {
        instructions.push(abi::subtract_immediate(
            abi::return_register(),
            abi::return_register(),
            1, // exclude the NUL — the caller copies exactly this many bytes
        ));
    } else {
        instructions.push(abi::load_u64(
            abi::return_register(),
            abi::stack_pointer(),
            RMARSHAL_DST_SLOT, // the buffer pointer
        ));
    }
    instructions.extend([
        abi::branch(&done),
        abi::label(&fail),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::label(&done),
        abi::add_stack(RMARSHAL_FRAME),
    ]);
}

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
            // ARG[0] (rcx) is a free caller-saved temp here (the void
            // GetSystemTimePreciseAsFileTime clobbered it); the SCRATCH pool must
            // not be used — its Win64 realizations are callee-saved.
            abi::load_u64(abi::ARG[0], abi::stack_pointer(), 0),
            abi::store_u64(
                abi::ARG[0],
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

    fn emit_write(
        &self,
        from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // The shared `write(fd, buf, len)` seam: fd in the x0 slot, buf in x1
        // (string_data_register), len in x2 (string_length_register); the contract
        // returns the byte count in the return register, negative on failure (like
        // the POSIX `write` every other backend calls). Windows has no fd, so
        // resolve the POSIX fd to a console HANDLE via GetStdHandle, then WriteFile.
        //   fd 1 (stdout) → STD_OUTPUT_HANDLE (-11); fd 2 (stderr) → STD_ERROR_HANDLE
        //   (-12); i.e. nStdHandle = -(fd + 10).
        //
        // WriteFile is `BOOL WriteFile(hFile, lpBuffer, nBytes, lpBytesWritten,
        // lpOverlapped)` — five arguments. The fifth (lpOverlapped) is a Win64
        // STACK argument at [sp+0x20], above the 32-byte shadow space, and MUST be
        // NULL for a synchronous console handle (a garbage slot makes WriteFile
        // fail). We carve a self-contained outgoing frame and drive both calls'
        // shadow space through it, so this composes with the caller's own frame
        // regardless of its shadow accounting.
        //
        //   [sp+0x00 .. 0x20)  shadow space for the callee (32 bytes)
        //   [sp+0x20]          lpOverlapped = NULL          (WriteFile's 5th arg)
        //   [sp+0x28]          lpNumberOfBytesWritten (out) (WriteFile's 4th arg target)
        //   [sp+0x30]          saved buf   (survives the GetStdHandle call)
        //   [sp+0x38]          saved len
        //   [sp+0x40]          resolved hFile (console handle or a file handle)
        // `emit_write` can be lowered more than once into a single function (e.g.
        // the entry's error tail alongside a buffered drain), so disambiguate the
        // branch labels by the current instruction offset — unique per call site.
        let n = instructions.len();
        let ok = format!("{from}_win_write_ok_{n}");
        let done = format!("{from}_win_write_done_{n}");
        let file_handle = format!("{from}_win_write_fileh_{n}");
        let have_handle = format!("{from}_win_write_haveh_{n}");
        instructions.extend([
            abi::subtract_stack(0x50),
            abi::store_u64(abi::ARG[1], abi::stack_pointer(), 0x30), // save buf
            abi::store_u64(abi::ARG[2], abi::stack_pointer(), 0x38), // save len
            // Resolve the destination handle. fd 1 (stdout) and 2 (stderr) are the
            // console POSIX fds and resolve via GetStdHandle(-(fd+10)); any larger
            // value is already a Win32 file HANDLE (CreateFileW) — fs writes pass
            // the handle straight through here (CreateFileW never returns 1/2).
            abi::compare_immediate(abi::ARG[0], "2"),
            abi::branch_gt(&file_handle),
            // console: nStdHandle = -(fd + 10), built without a negative immediate.
            // ARG[1] (rdx) is a free caller-saved temp now that buf is saved; the
            // SCRATCH pool must not be used — its Win64 realizations (rbx/rsi/rdi)
            // are callee-saved and would corrupt registers the caller keeps live.
            abi::add_immediate(abi::ARG[1], abi::ARG[0], 10), // fd + 10
            abi::move_immediate(abi::ARG[0], "Integer", "0"),
            abi::subtract_registers(abi::ARG[0], abi::ARG[0], abi::ARG[1]), // -(fd+10)
        ]);
        call_external(from, "GetStdHandle", KERNEL32, instructions, relocations);
        instructions.extend([
            abi::store_u64(abi::return_register(), abi::stack_pointer(), 0x40), // hFile
            abi::branch(&have_handle),
            abi::label(&file_handle),
            abi::store_u64(abi::ARG[0], abi::stack_pointer(), 0x40), // hFile = handle directly
            abi::label(&have_handle),
            abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x20), // lpOverlapped = NULL
            abi::load_u64(abi::ARG[0], abi::stack_pointer(), 0x40), // hFile
            abi::load_u64(abi::ARG[1], abi::stack_pointer(), 0x30), // lpBuffer
            abi::load_u64(abi::ARG[2], abi::stack_pointer(), 0x38), // nNumberOfBytesToWrite
            abi::add_immediate(abi::ARG[3], abi::stack_pointer(), 0x28), // &lpBytesWritten
            // Zero the whole 8-byte slot first: lpNumberOfBytesWritten is a DWORD
            // (32-bit) out-param, so WriteFile writes only the low 32 bits. Without
            // this, the load_u64 below picks up uninitialized garbage in the high
            // 32 bits and returns a huge count — the caller's write loop then does
            // `remaining -= huge`, underflows, and spins forever (this manifested
            // only when prior stack use left non-zero garbage there).
            abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x28),
        ]);
        call_external(from, "WriteFile", KERNEL32, instructions, relocations);
        instructions.extend([
            // WriteFile returns BOOL: nonzero = success (return the bytes written),
            // zero = failure (return -1, routing the caller to its error/retry tail).
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_ne(&ok),
            abi::move_immediate(abi::return_register(), "Integer", "0"),
            abi::subtract_immediate(abi::return_register(), abi::return_register(), 1), // -1
            abi::branch(&done),
            abi::label(&ok),
            abi::load_u64(abi::return_register(), abi::stack_pointer(), 0x28), // bytes written
            abi::label(&done),
            abi::add_stack(0x50),
        ]);
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
        from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // Contract (shared fs/paths.rs): the UTF-8 path C-string is in ARG[0];
        // return 0 in the return register iff the path exists. Windows:
        // GetFileAttributesW(wpath) returns INVALID_FILE_ATTRIBUTES ((DWORD)-1 =
        // 0xFFFFFFFF, bit 31 set) when the path does not exist, and a small
        // FILE_ATTRIBUTE_* bitmask (always < 0x80000000, bit 31 clear) when it
        // does. So `result >> 31` is exactly the contract: 1 (nonzero) for
        // missing, 0 for exists — no branch and no oversized-immediate compare.
        instructions.push(abi::subtract_stack(MARSHAL_FRAME));
        emit_marshal_path(from, instructions, relocations);
        instructions.push(abi::load_u64(
            abi::ARG[0],
            abi::stack_pointer(),
            MARSHAL_WBUF_SLOT,
        ));
        call_external(from, "GetFileAttributesW", KERNEL32, instructions, relocations);
        instructions.extend([
            abi::shift_right_immediate(abi::return_register(), abi::return_register(), 31),
            abi::add_stack(MARSHAL_FRAME),
        ]);
        Ok(())
    }

    fn emit_path_stat(
        &self,
        from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // Contract (shared fs/paths.rs kind helper): the UTF-8 path C-string is in
        // ARG[0] and a stat buffer pointer is in ARG[1]. Windows has no `struct
        // stat`; store the `GetFileAttributesW` DWORD (the attribute bitmask, or
        // INVALID_FILE_ATTRIBUTES when the path is missing) into the buffer, which
        // `emit_stat_is_kind` then interprets. Frame is MARSHAL_FRAME + one extra
        // slot at 0x48 to preserve the buffer pointer across the arena/convert calls.
        const FRAME: usize = 0x50;
        const STATBUF_SLOT: usize = 0x48;
        instructions.extend([
            abi::subtract_stack(FRAME),
            abi::store_u64(abi::ARG[1], abi::stack_pointer(), STATBUF_SLOT),
        ]);
        emit_marshal_path(from, instructions, relocations);
        instructions.push(abi::load_u64(
            abi::ARG[0],
            abi::stack_pointer(),
            MARSHAL_WBUF_SLOT,
        ));
        call_external(from, "GetFileAttributesW", KERNEL32, instructions, relocations);
        instructions.extend([
            // ARG[1] (rdx) is a free caller-saved temp for the buffer pointer, and
            // is distinct from the return register (rax) that holds the attributes.
            // The SCRATCH pool must not be used — callee-saved on Win64.
            abi::load_u64(abi::ARG[1], abi::stack_pointer(), STATBUF_SLOT),
            abi::store_u64(abi::return_register(), abi::ARG[1], 0),
            abi::move_immediate(abi::return_register(), "Integer", "0"),
            abi::add_stack(FRAME),
        ]);
        Ok(())
    }

    fn emit_current_directory(
        &self,
        from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        emit_dir_path_query(from, "GetCurrentDirectoryW", false, instructions, relocations);
        Ok(())
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
        from: &str,
        operation: FsPathOperation,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // Contract (shared fs): the arena UTF-8 path is in ARG[0]; return 0 on
        // success (any nonzero routes the caller to its error tail). Each Win32
        // call returns BOOL (nonzero = success), so the result is inverted.
        let (symbol, is_mkdir) = match operation {
            FsPathOperation::Chdir => ("SetCurrentDirectoryW", false),
            FsPathOperation::Unlink => ("DeleteFileW", false),
            FsPathOperation::Mkdir => ("CreateDirectoryW", true),
            FsPathOperation::Rmdir => ("RemoveDirectoryW", false),
        };
        let n = instructions.len();
        let ok = format!("{from}_fsop_ok_{n}");
        let done = format!("{from}_fsop_done_{n}");
        instructions.push(abi::subtract_stack(MARSHAL_FRAME));
        emit_marshal_path(from, instructions, relocations);
        instructions.push(abi::load_u64(
            abi::ARG[0],
            abi::stack_pointer(),
            MARSHAL_WBUF_SLOT,
        ));
        if is_mkdir {
            // CreateDirectoryW(path, lpSecurityAttributes = NULL).
            instructions.push(abi::move_immediate(abi::ARG[1], "Integer", "0"));
        }
        call_external(from, symbol, KERNEL32, instructions, relocations);
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_ne(&ok), // BOOL != 0 → success
            abi::move_immediate(abi::return_register(), "Integer", "1"), // failure
            abi::branch(&done),
            abi::label(&ok),
            abi::move_immediate(abi::return_register(), "Integer", "0"), // success
            abi::label(&done),
            abi::add_stack(MARSHAL_FRAME),
        ]);
        Ok(())
    }

    fn emit_errno(
        &self,
        from: &str,
        dst: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // Windows has no `errno`; the last error code comes from GetLastError().
        // The POSIX callers use this to detect EINTR (and retry) — a code Windows
        // never reports, so the retry never fires and the value flows to the
        // generic-failure path, which is correct (plan-47-F §3.3). GetLastError
        // takes no args and returns the DWORD in the return register.
        instructions.push(abi::subtract_stack(0x20)); // shadow space
        call_external(from, "GetLastError", KERNEL32, instructions, relocations);
        instructions.extend([
            abi::add_stack(0x20),
            abi::move_register(dst, abi::return_register()),
        ]);
        Ok(())
    }

    fn emit_open_file(
        &self,
        from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // Contract (shared fs/io.rs openFile helper): the arena UTF-8 path is in
        // ARG[0], the packed open flags (`open_flag_set`'s Windows arm:
        // (disposition<<32)|access) in ARG[1], the POSIX mode in ARG[2] (ignored).
        // Return the file HANDLE in the return register; the helper sign-extends
        // its low 32 bits and treats < 0 as failure — CreateFileW returns small
        // positive kernel handles and INVALID_HANDLE_VALUE (-1) on error, so that
        // check is correct. CreateFileW(lpFileName, dwDesiredAccess, dwShareMode,
        // NULL, dwCreationDisposition, dwFlagsAndAttributes, NULL) — three of its
        // seven args are on the stack (above the shadow), reusing the marshal
        // frame's now-dead path slot at 0x30 for the last one.
        const FRAME: usize = 0x50;
        const PACKED_SLOT: usize = 0x48;
        instructions.extend([
            abi::subtract_stack(FRAME),
            abi::store_u64(abi::ARG[1], abi::stack_pointer(), PACKED_SLOT), // save packed flags
        ]);
        emit_marshal_path(from, instructions, relocations);
        instructions.extend([
            abi::load_u64(abi::ARG[0], abi::stack_pointer(), MARSHAL_WBUF_SLOT), // lpFileName
            abi::load_u64(abi::ARG[1], abi::stack_pointer(), PACKED_SLOT),
            // Stage the three stack args using ARG[2] as a caller-saved scratch,
            // BEFORE it is set to its register value (dwShareMode=7). The SCRATCH
            // pool must not be used — its Win64 realizations are callee-saved.
            // dwCreationDisposition (5th, stack) = packed >> 32.
            abi::shift_right_immediate(abi::ARG[2], abi::ARG[1], 32),
            abi::store_u64(abi::ARG[2], abi::stack_pointer(), 0x20),
            abi::move_immediate(abi::ARG[2], "Integer", "128"), // FILE_ATTRIBUTE_NORMAL
            abi::store_u64(abi::ARG[2], abi::stack_pointer(), 0x28), // 6th (stack)
            abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x30),   // 7th hTemplateFile = NULL
            // dwDesiredAccess: CreateFileW reads it as the low 32 bits of rdx, so
            // the packed value in ARG[1] goes straight in — the disposition in the
            // high half is ignored by the DWORD parameter.
            abi::move_immediate(abi::ARG[2], "Integer", "7"), // FILE_SHARE_READ|WRITE|DELETE
            abi::move_immediate(abi::ARG[3], "Integer", "0"), // lpSecurityAttributes = NULL
        ]);
        call_external(from, "CreateFileW", KERNEL32, instructions, relocations);
        instructions.push(abi::add_stack(FRAME));
        Ok(())
    }

    fn emit_read_file(
        &self,
        from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // read(fd, buf, len): HANDLE in ARG[0], buffer in ARG[1], length in ARG[2];
        // return the byte count (0 = EOF, negative = error). ReadFile(hFile,
        // lpBuffer, nToRead, &nRead, NULL) — the 5th arg (lpOverlapped) is a stack
        // arg that MUST be NULL; the bytes-read out-param and the NULL live in the
        // outgoing frame. On BOOL failure return -1; otherwise return nRead (which
        // is 0 at end of file, exactly the read() contract).
        let n = instructions.len();
        let ok = format!("{from}_read_ok_{n}");
        let done = format!("{from}_read_done_{n}");
        instructions.extend([
            abi::subtract_stack(0x40),
            abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x20), // lpOverlapped = NULL (5th)
            // Zero the nRead slot first — it is a DWORD (32-bit) out-param, so
            // ReadFile writes only the low 32 bits; the load_u64 below would
            // otherwise return garbage in the high 32 bits (see emit_write).
            abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x28),
            abi::add_immediate(abi::ARG[3], abi::stack_pointer(), 0x28), // &nRead (4th)
        ]);
        call_external(from, "ReadFile", KERNEL32, instructions, relocations);
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_ne(&ok),
            abi::move_immediate(abi::return_register(), "Integer", "0"),
            abi::subtract_immediate(abi::return_register(), abi::return_register(), 1), // -1
            abi::branch(&done),
            abi::label(&ok),
            abi::load_u64(abi::return_register(), abi::stack_pointer(), 0x28), // nRead
            abi::label(&done),
            abi::add_stack(0x40),
        ]);
        Ok(())
    }

    fn emit_close_file(
        &self,
        from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // close(fd): HANDLE in ARG[0], return 0 on success. CloseHandle returns
        // BOOL (nonzero = success), so map nonzero → 0 and zero → -1.
        let n = instructions.len();
        let ok = format!("{from}_close_ok_{n}");
        let done = format!("{from}_close_done_{n}");
        instructions.push(abi::subtract_stack(0x20)); // shadow only
        call_external(from, "CloseHandle", KERNEL32, instructions, relocations);
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_ne(&ok),
            abi::move_immediate(abi::return_register(), "Integer", "0"),
            abi::subtract_immediate(abi::return_register(), abi::return_register(), 1), // -1
            abi::branch(&done),
            abi::label(&ok),
            abi::move_immediate(abi::return_register(), "Integer", "0"),
            abi::label(&done),
            abi::add_stack(0x20),
        ]);
        Ok(())
    }

    fn emit_sync_file(
        &self,
        from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // fsync(fd): HANDLE in ARG[0], return 0 on success. FlushFileBuffers
        // returns BOOL.
        let n = instructions.len();
        let ok = format!("{from}_sync_ok_{n}");
        let done = format!("{from}_sync_done_{n}");
        instructions.push(abi::subtract_stack(0x20));
        call_external(from, "FlushFileBuffers", KERNEL32, instructions, relocations);
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_ne(&ok),
            abi::move_immediate(abi::return_register(), "Integer", "0"),
            abi::subtract_immediate(abi::return_register(), abi::return_register(), 1),
            abi::branch(&done),
            abi::label(&ok),
            abi::move_immediate(abi::return_register(), "Integer", "0"),
            abi::label(&done),
            abi::add_stack(0x20),
        ]);
        Ok(())
    }

    fn emit_seek_file(
        &self,
        from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // lseek(fd, offset, whence): HANDLE in ARG[0], offset in ARG[1], whence in
        // ARG[2] (0=SET, 1=CUR, 2=END — the same values as FILE_BEGIN/CURRENT/END).
        // Return the new absolute position, or -1 on error. SetFilePointerEx(hFile,
        // liDistanceToMove, &liNewFilePointer, dwMoveMethod) — hFile and the 64-bit
        // distance are already in ARG[0]/ARG[1]; move whence into r9 and point r8 at
        // an output slot before the call, then read the new position back.
        let n = instructions.len();
        let ok = format!("{from}_seek_ok_{n}");
        let done = format!("{from}_seek_done_{n}");
        instructions.extend([
            abi::subtract_stack(0x30),
            abi::move_register(abi::ARG[3], abi::ARG[2]), // dwMoveMethod = whence
            abi::add_immediate(abi::ARG[2], abi::stack_pointer(), 0x28), // &liNewFilePointer
        ]);
        call_external(from, "SetFilePointerEx", KERNEL32, instructions, relocations);
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_ne(&ok),
            abi::move_immediate(abi::return_register(), "Integer", "0"),
            abi::subtract_immediate(abi::return_register(), abi::return_register(), 1), // -1
            abi::branch(&done),
            abi::label(&ok),
            abi::load_u64(abi::return_register(), abi::stack_pointer(), 0x28), // new position
            abi::label(&done),
            abi::add_stack(0x30),
        ]);
        Ok(())
    }

    fn emit_rename_path(
        &self,
        from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // rename(old, new): old (arena UTF-8) in ARG[0], new in ARG[1]; return 0 on
        // success. Marshal BOTH paths to UTF-16, then MoveFileExW(old, new,
        // MOVEFILE_REPLACE_EXISTING). The marshal helper works on one path at a
        // time via [0x20..0x38], so the frame adds slots at 0x48 (saved new path)
        // and 0x50 (first wide buffer) that survive the second marshal.
        const FRAME: usize = 0x60;
        const NEW_PATH_SLOT: usize = 0x48;
        const WBUF_OLD_SLOT: usize = 0x50;
        let n = instructions.len();
        let ok = format!("{from}_rename_ok_{n}");
        let done = format!("{from}_rename_done_{n}");
        instructions.extend([
            abi::subtract_stack(FRAME),
            abi::store_u64(abi::ARG[1], abi::stack_pointer(), NEW_PATH_SLOT), // save new path
        ]);
        emit_marshal_path(from, instructions, relocations); // old → [MARSHAL_WBUF_SLOT]
        instructions.extend([
            abi::load_u64(abi::ARG[0], abi::stack_pointer(), MARSHAL_WBUF_SLOT),
            abi::store_u64(abi::ARG[0], abi::stack_pointer(), WBUF_OLD_SLOT), // save wide old
            abi::load_u64(abi::ARG[0], abi::stack_pointer(), NEW_PATH_SLOT),  // new path
        ]);
        emit_marshal_path(from, instructions, relocations); // new → [MARSHAL_WBUF_SLOT]
        instructions.extend([
            abi::load_u64(abi::ARG[0], abi::stack_pointer(), WBUF_OLD_SLOT), // lpExistingFileName
            abi::load_u64(abi::ARG[1], abi::stack_pointer(), MARSHAL_WBUF_SLOT), // lpNewFileName
            abi::move_immediate(abi::ARG[2], "Integer", "1"), // MOVEFILE_REPLACE_EXISTING
        ]);
        call_external(from, "MoveFileExW", KERNEL32, instructions, relocations);
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_ne(&ok), // BOOL != 0 → success
            abi::move_immediate(abi::return_register(), "Integer", "1"), // failure
            abi::branch(&done),
            abi::label(&ok),
            abi::move_immediate(abi::return_register(), "Integer", "0"), // success
            abi::label(&done),
            abi::add_stack(FRAME),
        ]);
        Ok(())
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
            // Shuffle (buf x0→pbBuffer, len x1→cbBuffer) in an order that needs no
            // scratch: copy len up to ARG[2] before ARG[1] is overwritten, then buf
            // into ARG[1], then NULL into ARG[0]. The SCRATCH pool must not be used —
            // callee-saved on Win64.
            abi::move_register(abi::ARG[2], abi::ARG[1]), // cbBuffer = len
            abi::move_register(abi::ARG[1], abi::ARG[0]), // pbBuffer = buf
            abi::move_immediate(abi::ARG[0], "Integer", "0"), // hAlgorithm = NULL
            abi::move_immediate(abi::ARG[3], "Integer", BCRYPT_USE_SYSTEM_PREFERRED_RNG),
        ]);
        call_external(from, "BCryptGenRandom", BCRYPT, instructions, relocations);
        Ok(())
    }

    fn emit_temp_directory(
        &self,
        from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // GetTempPathW(nBufferLength, lpBuffer) — same 2-arg shape as
        // GetCurrentDirectoryW, returns the UTF-16 temp dir (with trailing '\').
        emit_dir_path_query(from, "GetTempPathW", true, instructions, relocations);
        Ok(())
    }

    fn emit_opendir(
        &self,
        from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // opendir(path): the arena UTF-8 dir path is in ARG[0]; return a DIR*
        // (> 0) on success, 0 on error. Marshal the path to UTF-16, append the
        // L"\*" search wildcard (FindFirstFileW lists a directory's contents only
        // with a wildcard), allocate the DIR struct, and FindFirstFileW into it.
        const FRAME: usize = 0x50;
        const DIR_SLOT: usize = 0x48;
        let n = instructions.len();
        let scan = format!("{from}_od_scan_{n}");
        let scan_done = format!("{from}_od_scandone_{n}");
        let fail = format!("{from}_od_fail_{n}");
        let done = format!("{from}_od_done_{n}");
        instructions.push(abi::subtract_stack(FRAME));
        emit_marshal_path(from, instructions, relocations); // wide path at [sp+MARSHAL_WBUF_SLOT]
        instructions.extend([
            // Find the NUL wchar terminating the wide path, then overwrite it with
            // L'\' L'*' L'\0'. ARG[0]=wide base, ARG[1]=byte index.
            abi::load_u64(abi::ARG[0], abi::stack_pointer(), MARSHAL_WBUF_SLOT),
            abi::move_immediate(abi::ARG[1], "Integer", "0"),
            abi::label(&scan),
            abi::add_registers(abi::ARG[2], abi::ARG[0], abi::ARG[1]),
            abi::load_u16(abi::ARG[3], abi::ARG[2], 0),
            abi::compare_immediate(abi::ARG[3], "0"),
            abi::branch_eq(&scan_done),
            abi::add_immediate(abi::ARG[1], abi::ARG[1], 2),
            abi::branch(&scan),
            abi::label(&scan_done),
            // ARG[2] = &NUL wchar. Write the wildcard suffix.
            abi::move_immediate(abi::ARG[3], "Integer", "92"), // L'\'
            abi::store_u16(abi::ARG[3], abi::ARG[2], 0),
            abi::move_immediate(abi::ARG[3], "Integer", "42"), // L'*'
            abi::store_u16(abi::ARG[3], abi::ARG[2], 2),
            abi::move_immediate(abi::ARG[3], "Integer", "0"), // L'\0'
            abi::store_u16(abi::ARG[3], abi::ARG[2], 4),
            // Allocate the DIR struct.
            abi::move_immediate(abi::return_register(), "Integer", DIR_SIZE),
            abi::move_immediate(abi::ARG[1], "Integer", "8"),
            abi::branch_link(code::ARENA_ALLOC_SYMBOL),
        ]);
        relocations.push(CodeRelocation {
            from: from.to_string(),
            to: code::ARENA_ALLOC_SYMBOL.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
        instructions.extend([
            abi::store_u64(abi::RET[1], abi::stack_pointer(), DIR_SLOT),
            // FindFirstFileW(lpFileName = wide pattern, lpFindFileData = &DIR.findData)
            abi::load_u64(abi::ARG[0], abi::stack_pointer(), MARSHAL_WBUF_SLOT),
            abi::load_u64(abi::ARG[1], abi::stack_pointer(), DIR_SLOT),
            abi::add_immediate(abi::ARG[1], abi::ARG[1], DIR_FINDDATA_OFF),
        ]);
        call_external(from, "FindFirstFileW", KERNEL32, instructions, relocations);
        instructions.extend([
            // INVALID_HANDLE_VALUE is (HANDLE)-1; a valid search handle is a small
            // positive value, so `<= 0` means failure.
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_le(&fail),
            abi::load_u64(abi::ARG[1], abi::stack_pointer(), DIR_SLOT),
            abi::store_u64(abi::return_register(), abi::ARG[1], DIR_HANDLE_OFF),
            abi::move_immediate(abi::ARG[2], "Integer", "1"),
            abi::store_u64(abi::ARG[2], abi::ARG[1], DIR_FIRST_OFF), // first pending
            abi::move_register(abi::return_register(), abi::ARG[1]), // return DIR*
            abi::branch(&done),
            abi::label(&fail),
            abi::move_immediate(abi::return_register(), "Integer", "0"),
            abi::label(&done),
            abi::add_stack(FRAME),
        ]);
        Ok(())
    }

    fn emit_readdir(
        &self,
        from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // readdir(DIR*): DIR* in ARG[0]; return the DIR* (nonzero) when an entry is
        // available (its UTF-8 name is left in DIR+DIR_NAME_OFF), or 0 at the end.
        // The first call consumes FindFirstFileW's entry; later calls FindNextFileW.
        const FRAME: usize = 0x60;
        const DIR_SLOT: usize = 0x50;
        let n = instructions.len();
        let have = format!("{from}_rd_have_{n}");
        let convert = format!("{from}_rd_conv_{n}");
        let end = format!("{from}_rd_end_{n}");
        let done = format!("{from}_rd_done_{n}");
        instructions.extend([
            abi::subtract_stack(FRAME),
            abi::store_u64(abi::ARG[0], abi::stack_pointer(), DIR_SLOT),
            abi::load_u64(abi::ARG[1], abi::ARG[0], DIR_FIRST_OFF),
            abi::compare_immediate(abi::ARG[1], "0"),
            abi::branch_ne(&have), // first entry already in findData
            // FindNextFileW(handle, &findData)
            abi::load_u64(abi::ARG[0], abi::stack_pointer(), DIR_SLOT),
            abi::add_immediate(abi::ARG[1], abi::ARG[0], DIR_FINDDATA_OFF),
            abi::load_u64(abi::ARG[0], abi::ARG[0], DIR_HANDLE_OFF),
        ]);
        call_external(from, "FindNextFileW", KERNEL32, instructions, relocations);
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_eq(&end), // BOOL 0 → no more entries
            abi::branch(&convert),
            abi::label(&have),
            abi::load_u64(abi::ARG[0], abi::stack_pointer(), DIR_SLOT),
            abi::store_u64(abi::ZERO, abi::ARG[0], DIR_FIRST_OFF), // consume the first entry
            abi::label(&convert),
            // WideCharToMultiByte(CP_UTF8, 0, DIR+cFileName, -1, DIR+name, cap, NULL, NULL)
            abi::move_immediate(abi::ARG[0], "Integer", CP_UTF8),
            abi::move_immediate(abi::ARG[1], "Integer", "0"),
            abi::load_u64(abi::ARG[2], abi::stack_pointer(), DIR_SLOT),
            abi::add_immediate(abi::ARG[2], abi::ARG[2], DIR_NAME_OFF),
            abi::store_u64(abi::ARG[2], abi::stack_pointer(), 0x20), // lpMultiByteStr (5th)
            abi::move_immediate(abi::ARG[2], "Integer", DIR_NAME_CAP),
            abi::store_u64(abi::ARG[2], abi::stack_pointer(), 0x28), // cbMultiByte (6th)
            abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x30), // 7th NULL
            abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x38), // 8th NULL
            abi::load_u64(abi::ARG[2], abi::stack_pointer(), DIR_SLOT),
            abi::add_immediate(abi::ARG[2], abi::ARG[2], DIR_CFILENAME_OFF), // lpWideCharStr
            abi::move_immediate(abi::ARG[3], "Integer", "0"),
            abi::subtract_immediate(abi::ARG[3], abi::ARG[3], 1), // cchWideChar = -1
        ]);
        call_external(from, "WideCharToMultiByte", KERNEL32, instructions, relocations);
        instructions.extend([
            abi::load_u64(abi::return_register(), abi::stack_pointer(), DIR_SLOT), // DIR*
            abi::branch(&done),
            abi::label(&end),
            abi::move_immediate(abi::return_register(), "Integer", "0"),
            abi::label(&done),
            abi::add_stack(FRAME),
        ]);
        Ok(())
    }

    fn emit_closedir(
        &self,
        from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // closedir(DIR*): DIR* in ARG[0]. FindClose(handle); return 0.
        instructions.extend([
            abi::load_u64(abi::ARG[0], abi::ARG[0], DIR_HANDLE_OFF),
            abi::subtract_stack(0x20),
        ]);
        call_external(from, "FindClose", KERNEL32, instructions, relocations);
        instructions.extend([
            abi::add_stack(0x20),
            abi::move_immediate(abi::return_register(), "Integer", "0"),
        ]);
        Ok(())
    }

    fn emit_realpath(
        &self,
        from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // realpath(path, resolved): path (arena UTF-8) in ARG[0], resolved buffer
        // (PATH_MAX+1 = 4097 bytes) in ARG[1]; return the resolved buffer pointer
        // (nonzero) on success. Marshal the input, GetFullPathNameW into an arena
        // UTF-16 scratch, then convert back to UTF-8 into the caller's buffer.
        let n = instructions.len();
        let fail = format!("{from}_rp_fail_{n}");
        let done = format!("{from}_rp_done_{n}");
        instructions.extend([
            abi::subtract_stack(RMARSHAL_FRAME),
            abi::store_u64(abi::ARG[1], abi::stack_pointer(), RMARSHAL_DST_SLOT), // resolved dst
            abi::move_immediate(abi::ARG[2], "Integer", "4097"),
            abi::store_u64(abi::ARG[2], abi::stack_pointer(), RMARSHAL_CAP_SLOT), // capacity
        ]);
        emit_marshal_path(from, instructions, relocations); // input → wide at [MARSHAL_WBUF_SLOT]
        instructions.extend([
            // arena UTF-16 output scratch.
            abi::move_immediate(abi::return_register(), "Integer", "65536"),
            abi::move_immediate(abi::ARG[1], "Integer", "2"),
            abi::branch_link(code::ARENA_ALLOC_SYMBOL),
        ]);
        relocations.push(CodeRelocation {
            from: from.to_string(),
            to: code::ARENA_ALLOC_SYMBOL.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
        instructions.extend([
            abi::store_u64(abi::RET[1], abi::stack_pointer(), RMARSHAL_WBUF_SLOT),
            // GetFullPathNameW(lpFileName=wide_in, nBufferLength=32768,
            //                  lpBuffer=wide_out, lpFilePart=NULL)
            abi::load_u64(abi::ARG[0], abi::stack_pointer(), MARSHAL_WBUF_SLOT),
            abi::move_immediate(abi::ARG[1], "Integer", "32768"),
            abi::load_u64(abi::ARG[2], abi::stack_pointer(), RMARSHAL_WBUF_SLOT),
            abi::move_immediate(abi::ARG[3], "Integer", "0"),
        ]);
        call_external(from, "GetFullPathNameW", KERNEL32, instructions, relocations);
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_eq(&fail),
        ]);
        emit_wide_to_utf8(from, instructions, relocations); // wide_out → resolved dst
        instructions.extend([
            abi::load_u64(abi::return_register(), abi::stack_pointer(), RMARSHAL_DST_SLOT),
            abi::branch(&done),
            abi::label(&fail),
            abi::move_immediate(abi::return_register(), "Integer", "0"),
            abi::label(&done),
            abi::add_stack(RMARSHAL_FRAME),
        ]);
        Ok(())
    }

    // --- POSIX-struct constant accessors ----------------------------------
    // Windows has no termios/dirent/stat/addrinfo structs; 47-E raises this seam
    // to intent-level methods. Unreachable until a later sub-plan advertises the
    // surface, so a placeholder 0 is safe (and never read).

    fn emit_apply_raw_mode(
        &self,
        _base_register: &str,
        _original_offset: usize,
        _modified_offset: usize,
        _disable_echo: bool,
        _disable_canonical: bool,
        _instructions: &mut Vec<CodeInstruction>,
    ) {
        // 47-G owns Windows raw mode (GetConsoleMode/SetConsoleMode — a DWORD
        // bitmask on a handle), which has no `struct termios`.
        unreachable!("47-G owns the Windows raw-mode toggle")
    }
    fn emit_stat_is_kind(
        &self,
        stat_offset: usize,
        expected_kind: &str,
        mode: &str,
        mask: &str,
        _expected: &str,
        found: &str,
        missing: &str,
        instructions: &mut Vec<CodeInstruction>,
    ) {
        // `emit_path_stat` stored the GetFileAttributesW DWORD at sp+stat_offset.
        // INVALID_FILE_ATTRIBUTES (bit 31 set) => the path is missing. Otherwise
        // the FILE_ATTRIBUTE_DIRECTORY (0x10) bit distinguishes a directory from a
        // regular file. `expected_kind` is the POSIX mode literal the shared caller
        // passes (FS_MODE_DIRECTORY / FS_MODE_REGULAR); map it to the directory-bit
        // test here.
        instructions.extend([
            abi::load_u32(mode, abi::stack_pointer(), stat_offset),
            abi::shift_right_immediate(mask, mode, 31), // 1 iff INVALID (missing)
            abi::compare_immediate(mask, "0"),
            abi::branch_ne(missing),
            abi::move_immediate(mask, "Integer", "16"), // FILE_ATTRIBUTE_DIRECTORY
            abi::and_registers(mode, mode, mask),        // 0x10 iff a directory
            abi::compare_immediate(mode, "0"),
        ]);
        if expected_kind == code::FS_MODE_DIRECTORY {
            instructions.push(abi::branch_ne(found)); // directory bit set => is a dir
        } else {
            instructions.push(abi::branch_eq(found)); // bit clear => a regular file
        }
        instructions.push(abi::branch(missing));
    }
    fn emit_read_dir_entry(
        &self,
        prefix: &str,
        nameptr: &str,
        namelen: &str,
        byte: &str,
        scratch: &str,
        instructions: &mut Vec<CodeInstruction>,
    ) {
        // `emit_readdir` returns the DIR* (or 0 at end) and leaves the entry's
        // UTF-8 name at DIR + DIR_NAME_OFF. Read it here: nameptr = DIR + NAME_OFF,
        // namelen = strlen (the name buffer is NUL-terminated by WideCharToMultiByte).
        let name_len_loop = format!("{prefix}_name_len_loop");
        let name_len_done = format!("{prefix}_name_len_done");
        let done = format!("{prefix}_done");
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_eq(&done),
            abi::add_immediate(nameptr, abi::return_register(), DIR_NAME_OFF),
            abi::move_register(scratch, nameptr),
            abi::move_immediate(namelen, "Integer", "0"),
            abi::label(&name_len_loop),
            abi::load_u8(byte, scratch, 0),
            abi::compare_immediate(byte, "0"),
            abi::branch_eq(&name_len_done),
            abi::add_immediate(namelen, namelen, 1),
            abi::add_immediate(scratch, scratch, 1),
            abi::branch(&name_len_loop),
            abi::label(&name_len_done),
        ]);
        // `done` is defined by the shared caller (the readdir loop's exit label);
        // the early `branch_eq(&done)` above jumps into it. Do not re-emit it.
        let _ = &done;
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
    fn socket_would_block_code(&self) -> &'static str {
        "10035" // WSAEWOULDBLOCK
    }
    fn socket_message_size_code(&self) -> &'static str {
        "10040" // WSAEMSGSIZE
    }
    fn socket_in_progress_code(&self) -> &'static str {
        // A non-blocking Winsock connect reports WSAEWOULDBLOCK (not WSAEINPROGRESS,
        // which is a legacy 1.1 code); 47-I wires the actual connect/poll path.
        "10035" // WSAEWOULDBLOCK
    }
    fn emit_set_nonblocking(
        &self,
        _fd_offset: usize,
        _flags_offset: usize,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // 47-I owns Windows non-blocking sockets: ioctlsocket(fd, FIONBIO, &1),
        // which has no fcntl / F_SETFL.
        unreachable!("47-I owns the Windows non-blocking-socket toggle")
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
