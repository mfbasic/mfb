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
//! Scope is the machine floor only (entry/exit/arena + the `emit_external_call` IAT
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
use crate::codegen::engine::mir::MirPlan;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::types::AppEntrySpec;
use crate::codegen::engine::types::CodeDataObject;
use crate::codegen::engine::types::CodeFunction;
use crate::codegen::engine::types::CodeInstruction;
use crate::codegen::engine::types::CodeRelocation;
use crate::codegen::engine::types::FsPathOperation;
use crate::codegen::engine::types::NativeCodePlan;
use crate::codegen::engine::types::ProgramEntrySpec;
use crate::codegen::engine::types::RelocIntent;
use crate::target::shared::nir::NirModule;
use crate::target::shared::plan::NativePlan;
use crate::target::win_x86_64::app;

const KERNEL32: &str = "kernel32.dll";
const ADVAPI32: &str = "advapi32.dll";
const SHELL32: &str = "shell32.dll";
const SHLWAPI: &str = "shlwapi.dll"; // bug-431: PathRemoveFileSpecA for the vendored-DLL path
const WS2_32: &str = "ws2_32.dll";
/// bug-431: `LoadLibraryExA` flag — resolve the library (and its own
/// dependencies) from the directory of the absolute path passed, rather than the
/// default search order. Requires an absolute path, which the vendored loader
/// builds as `<exe_dir>\vendor\<name>`.
const LOAD_WITH_ALTERED_SEARCH_PATH: &str = "8";
// ioctlsocket command to toggle blocking mode: FIONBIO = 0x8004667E
// (_IOW('f', 126, u_long); the 'f' magic byte is 0x66 — bug-417).
const FIONBIO: &str = "2147772030";
// MAKEWORD(2, 2) — the Winsock version WSAStartup requests.
const WINSOCK_VERSION: &str = "514"; // 0x0202
                                     // VirtualAlloc flAllocationType = MEM_COMMIT (0x1000) | MEM_RESERVE (0x2000).
const MEM_COMMIT_RESERVE: &str = "12288";
/// The callee's shadow space, which every Win64 caller must reserve.
///
/// 32 bytes is the requirement; 0x20 is also a multiple of 16, which is what an
/// emitter appended INLINE into an already-aligned body needs. An ordinary function
/// prologue wants an odd multiple of 8 instead, because a `call` skews `rsp` by the
/// return address it pushes — the two are different problems and bug-478 is what
/// happens when they are confused.
pub(crate) const SHADOW_FRAME: usize = 0x20;

// VirtualAlloc flProtect = PAGE_READWRITE (0x04).
const PAGE_READWRITE: &str = "4";
// VirtualFree dwFreeType = MEM_RELEASE (0x8000).
const MEM_RELEASE: &str = "32768";
// MultiByteToWideChar CodePage = CP_UTF8 (65001).
const CP_UTF8: &str = "65001";
// The two GetLastError codes that a *successful* POSIX `read()` reports as a
// 0-byte (end-of-input) return rather than a failure: the write end of a pipe
// has closed (`ERROR_BROKEN_PIPE`, 0x6D), and a read started at end-of-file
// (`ERROR_HANDLE_EOF`, 0x26). See `emit_read_file`.
const ERROR_HANDLE_EOF: &str = "38";
const ERROR_BROKEN_PIPE: &str = "109";

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
/// `_mfb_arena_alloc(size, align=2) -> RET[1]`, storing the returned buffer
/// pointer at `sp + slot`. The 64 KiB-ish requests never OOM in practice (the
/// arena maps fresh 1 MiB+ blocks), so the Result tag is not checked (matching
/// `emit_marshal_path`). plan-66-B.
fn arena_alloc_to_slot(
    from: &str,
    size: &str,
    slot: usize,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", size),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "2"),
        abi::branch_link(crate::codegen::error::constants::ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: from.to_string(),
        to: crate::codegen::error::constants::ARENA_ALLOC_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    });
    instructions.push(abi::store_u64(
        abi::mfb_return(1),
        abi::stack_pointer(),
        slot,
    ));
}

/// Emit `WideCharToMultiByte(CP_UTF8, 0, [wide_slot], -1, [u8_slot], u8_cap, NULL,
/// NULL)`, converting the NUL-terminated UTF-16 buffer at `sp + wide_slot` into the
/// UTF-8 buffer at `sp + u8_slot`. The caller owns the frame (shadow space +
/// [0x20]/[0x28]/[0x30]/[0x38] stack-arg slots). Returns the byte count (incl. NUL)
/// in the return register. plan-66-B.
fn emit_wide_slot_to_utf8(
    from: &str,
    wide_slot: usize,
    u8_slot: usize,
    u8_cap: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    instructions.extend([
        abi::move_immediate(abi::mfb_arg(0), "Integer", CP_UTF8),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "0"),
        abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), u8_slot), // lpMultiByteStr (5th)
        abi::store_u64(abi::mfb_arg(2), abi::stack_pointer(), 0x20),
        abi::move_immediate(abi::mfb_arg(2), "Integer", u8_cap), // cbMultiByte (6th)
        abi::store_u64(abi::mfb_arg(2), abi::stack_pointer(), 0x28),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x30), // lpDefaultChar (7th) NULL
        abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x38), // lpUsedDefaultChar (8th) NULL
        abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), wide_slot), // lpWideCharStr (3rd)
        abi::move_immediate(abi::mfb_arg(3), "Integer", "0"),
        abi::subtract_immediate(abi::mfb_arg(3), abi::mfb_arg(3), 1), // cchWideChar = -1
    ]);
    call_external(
        from,
        "WideCharToMultiByte",
        KERNEL32,
        instructions,
        relocations,
    );
}

/// Emit `MultiByteToWideChar(CP_UTF8, 0, [src_slot], -1, [dst_slot], wchar_cap)`,
/// converting the NUL-terminated UTF-8 C-string at `sp + src_slot` into the UTF-16
/// buffer at `sp + dst_slot`. Stages the two stack args (5th/6th) through `ARG[2]`
/// as a caller-saved scratch before it is set to its real 3rd-arg value — the same
/// discipline as `emit_marshal_path` (the SCRATCH pool must not be used on Win64).
/// The caller owns the surrounding frame (shadow space + [0x20]/[0x28] arg slots).
fn emit_utf8_slot_to_wide(
    from: &str,
    src_slot: usize,
    dst_slot: usize,
    wchar_cap: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    instructions.extend([
        abi::move_immediate(abi::mfb_arg(0), "Integer", CP_UTF8),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "0"),
        abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), dst_slot), // lpWideCharStr (5th)
        abi::store_u64(abi::mfb_arg(2), abi::stack_pointer(), 0x20),
        abi::move_immediate(abi::mfb_arg(2), "Integer", wchar_cap), // cchWideChar (6th)
        abi::store_u64(abi::mfb_arg(2), abi::stack_pointer(), 0x28),
        abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), src_slot), // lpMultiByteStr (3rd)
        abi::move_immediate(abi::mfb_arg(3), "Integer", "0"),
        abi::subtract_immediate(abi::mfb_arg(3), abi::mfb_arg(3), 1), // cbMultiByte = -1 (NUL-terminated)
    ]);
    call_external(
        from,
        "MultiByteToWideChar",
        KERNEL32,
        instructions,
        relocations,
    );
}

fn emit_marshal_path(
    from: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    instructions.extend([
        abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), 0x30), // save path
        // _mfb_arena_alloc(size = 65536, align = 2) -> RET[1] = buffer pointer.
        // A 64 KiB request never OOMs in practice (the arena maps fresh 1 MiB+
        // blocks via VirtualAlloc), so the Result tag is not checked here.
        abi::move_immediate(abi::return_register(), "Integer", "65536"),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "2"),
        abi::branch_link(crate::codegen::error::constants::ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: from.to_string(),
        to: crate::codegen::error::constants::ARENA_ALLOC_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), MARSHAL_WBUF_SLOT), // save wbuf
        abi::move_immediate(abi::mfb_arg(0), "Integer", CP_UTF8),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "0"), // dwFlags
        // Stage the two stack args using ARG[2] as a scratch BEFORE it is set to its
        // real register value (the path). ARG[2] (rdx→r8) is caller-saved; the
        // machine-floor SCRATCH pool must NOT be used here — on Win64 its low slots
        // realize to callee-saved rbx/rsi/rdi, so writing them corrupts registers
        // the caller keeps live (map_scratch_register's documented Win64 hazard).
        abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), MARSHAL_WBUF_SLOT), // wbuf (temp)
        abi::store_u64(abi::mfb_arg(2), abi::stack_pointer(), 0x20), // lpWideCharStr (5th)
        abi::move_immediate(abi::mfb_arg(2), "Integer", "32768"),
        abi::store_u64(abi::mfb_arg(2), abi::stack_pointer(), 0x28), // cchWideChar (6th)
        abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), 0x30),  // lpMultiByteStr = path
        // cbMultiByte = -1 (the input is NUL-terminated); the encoder rejects a
        // negative immediate, so build it as 0 - 1.
        abi::move_immediate(abi::mfb_arg(3), "Integer", "0"),
        abi::subtract_immediate(abi::mfb_arg(3), abi::mfb_arg(3), 1),
    ]);
    call_external(
        from,
        "MultiByteToWideChar",
        KERNEL32,
        instructions,
        relocations,
    );
}

/// Emit `GetFinalPathNameByHandleW(hFile=[handle_slot], lpszFilePath=[outbuf_slot],
/// cchFilePath=32767, dwFlags=0)` — resolving the handle's fully-normalized DOS
/// path (all reparse points followed) into the arena UTF-16 buffer whose pointer is
/// at `sp + outbuf_slot`. All four args are register args (no stack tail). Leaves
/// the returned WCHAR count (0 on failure) in the return register. The result
/// carries a `\\?\` prefix (`VOLUME_NAME_DOS`). plan-66-E.
fn emit_final_path_call(
    from: &str,
    handle_slot: usize,
    outbuf_slot: usize,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    instructions.extend([
        abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), handle_slot),
        abi::load_u64(abi::mfb_arg(1), abi::stack_pointer(), outbuf_slot),
        abi::move_immediate(abi::mfb_arg(2), "Integer", "32767"),
        abi::move_immediate(abi::mfb_arg(3), "Integer", "0"),
    ]);
    call_external(
        from,
        "GetFinalPathNameByHandleW",
        KERNEL32,
        instructions,
        relocations,
    );
    // plan-85: the returned WCHAR count is a C result (`rax`); this helper leaves it in
    // the aligned MFB result register per its contract.
    instructions.push(abi::move_register(
        abi::return_register(),
        crate::target::shared::abi::c_return(0),
    ));
}

/// Fold an ASCII uppercase WCHAR in `reg` to lowercase in place (`A`..=`Z` → +0x20),
/// leaving every other code unit unchanged — a case-insensitive comparison of two
/// Windows path components. `n` disambiguates the skip label across call sites.
/// plan-66-E.
fn emit_ascii_fold(reg: impl Into<Operand>, n: usize, instructions: &mut Vec<CodeInstruction>) {
    let reg = reg.into();
    let skip = format!("fold_skip_{}_{n}", reg.render());
    instructions.extend([
        abi::compare_immediate(&reg, "65"), // 'A'
        abi::branch_lt(&skip),
        abi::compare_immediate(&reg, "90"), // 'Z'
        abi::branch_gt(&skip),
        abi::add_immediate(&reg, &reg, 0x20),
        abi::label(&skip),
    ]);
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
        abi::move_immediate(abi::mfb_arg(0), "Integer", CP_UTF8),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "0"), // dwFlags
        // Stage the four stack args using ARG[2] as a caller-saved scratch before
        // it is set to its register value (lpWideCharStr).
        abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), RMARSHAL_DST_SLOT),
        abi::store_u64(abi::mfb_arg(2), abi::stack_pointer(), 0x20), // lpMultiByteStr (5th)
        abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), RMARSHAL_CAP_SLOT),
        abi::store_u64(abi::mfb_arg(2), abi::stack_pointer(), 0x28), // cbMultiByte (6th)
        abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x30),       // lpDefaultChar (7th)
        abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x38),       // lpUsedDefaultChar (8th)
        abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), RMARSHAL_WBUF_SLOT), // lpWideCharStr
        abi::move_immediate(abi::mfb_arg(3), "Integer", "0"),
        abi::subtract_immediate(abi::mfb_arg(3), abi::mfb_arg(3), 1), // cchWideChar = -1
    ]);
    call_external(
        from,
        "WideCharToMultiByte",
        KERNEL32,
        instructions,
        relocations,
    );
    // plan-85: WideCharToMultiByte returns the byte count as a C result (`rax`); this
    // helper's contract is to leave it in the aligned MFB result register, so name the
    // C-result token explicitly and move it there.
    instructions.push(abi::move_register(
        abi::return_register(),
        crate::target::shared::abi::c_return(0),
    ));
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
        abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), RMARSHAL_DST_SLOT), // dst
        abi::store_u64(abi::mfb_arg(1), abi::stack_pointer(), RMARSHAL_CAP_SLOT), // capacity
        // arena UTF-16 scratch (64 KiB, 32767 wchars = Windows max path).
        abi::move_immediate(abi::return_register(), "Integer", "65536"),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "2"),
        abi::branch_link(crate::codegen::error::constants::ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: from.to_string(),
        to: crate::codegen::error::constants::ARENA_ALLOC_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), RMARSHAL_WBUF_SLOT),
        abi::move_immediate(abi::mfb_arg(0), "Integer", "32768"), // nBufferLength (wchars)
        abi::load_u64(abi::mfb_arg(1), abi::stack_pointer(), RMARSHAL_WBUF_SLOT), // lpBuffer
    ]);
    call_external(from, symbol, KERNEL32, instructions, relocations);
    instructions.extend([
        abi::compare_immediate(abi::c_return(0), "0"),
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
    // No inline stack probe is needed: the PE header commits 1 MiB of stack up
    // front (see `os/windows/link/pe.rs`), which covers every real frame, so a
    // large `sub rsp, N` never skips the guard page.
    crate::codegen::engine::builder::lower_module_for_platform(
        module,
        native_plan,
        packages,
        &Platform,
    )
}

pub(crate) fn lower_module_mir(
    module: &NirModule,
    native_plan: &NativePlan,
    packages: &[PathBuf],
) -> Result<MirPlan, String> {
    crate::codegen::engine::builder::lower_module_mir_for_platform(
        module,
        native_plan,
        packages,
        &Platform,
    )
}

pub(crate) struct Platform;

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

/// Emit `ioctlsocket(fd, FIONBIO, &argp)` where `argp` is 1 (non-blocking) or 0
/// (blocking). The socket is loaded from `fd_offset` (relative to the caller's
/// stack pointer) BEFORE the frame adjust, then the argp `u_long` is staged in a
/// self-contained frame slot above the shadow space, so the call never disturbs
/// the caller's frame. `ARG[3]` (r9) is a free scratch — ioctlsocket only reads
/// rcx/rdx/r8.
fn emit_ioctl_fionbio(
    from: &str,
    fd_offset: usize,
    nonblocking: bool,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    const FRAME: usize = 0x30;
    const ARGP_SLOT: usize = 0x28; // above the 0x20 shadow space
    instructions.push(abi::load_u64(
        abi::mfb_arg(0),
        abi::stack_pointer(),
        fd_offset,
    ));
    instructions.push(abi::subtract_stack(FRAME));
    if nonblocking {
        instructions.push(abi::move_immediate(abi::mfb_arg(3), "Integer", "1"));
        instructions.push(abi::store_u64(
            abi::mfb_arg(3),
            abi::stack_pointer(),
            ARGP_SLOT,
        ));
    } else {
        instructions.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), ARGP_SLOT));
    }
    instructions.extend([
        abi::move_immediate(abi::mfb_arg(1), "Integer", FIONBIO),
        abi::add_immediate(abi::mfb_arg(2), abi::stack_pointer(), ARGP_SLOT),
    ]);
    call_external(from, "ioctlsocket", WS2_32, instructions, relocations);
    instructions.push(abi::add_stack(FRAME));
}

impl crate::codegen::engine::types::CodegenPlatform for Platform {
    fn target(&self) -> &'static str {
        "windows-x86_64"
    }

    fn arch(&self) -> &'static str {
        "x86_64"
    }

    fn backend(&self) -> &'static dyn crate::codegen::engine::mir::Backend {
        // Wires the Win64 ABI backend (plan-47-B A1) — the production consumer
        // that removes A1's dead-code allows.
        &WIN64_BACKEND
    }

    fn entry_stack_misaligned_on_entry(&self) -> bool {
        // The PE loader `call`s the image entry, so it arrives at `sp % 16 == 8`;
        // the shared preamble realigns with one `sub rsp, 8`.
        true
    }

    fn defers_arg_capture(&self) -> bool {
        // A raw PE entry receives no argc/argv; os::args is built from
        // GetCommandLineW after the arena is mapped (emit_build_argv_utf8). plan-66-B.
        true
    }

    fn emit_build_argv_utf8(
        &self,
        entry_symbol: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // Build a POSIX UTF-8 argv from GetCommandLineW → CommandLineToArgvW, then
        // marshal each UTF-16 arg into the arena, leaving `argc` in ARG[0] and the
        // `char**` in ARG[1]. Frame (subtract_stack(0x70)): shadow [0x00..0x20),
        // marshal stack args [0x20..0x40), [0x40] argc (int, out), [0x48] wide argv
        // (LocalAlloc'd by CommandLineToArgvW), [0x50] UTF-8 argv array, [0x58] loop
        // index, [0x60] current wide arg ptr, [0x68] current UTF-8 buffer. plan-66-B.
        const ARGC: usize = 0x40;
        const WARGV: usize = 0x48;
        const U8ARGV: usize = 0x50;
        const IDX: usize = 0x58;
        const WARG: usize = 0x60;
        const U8ARG: usize = 0x68;
        const ARG_CAP: &str = "131072";
        let from = entry_symbol;
        let n = instructions.len();
        let l = |s: &str| format!("{from}_argv_{s}_{n}");
        let (loop_top, loop_done) = (l("lt"), l("ld"));
        instructions.push(abi::subtract_stack(0x70));
        call_external(from, "GetCommandLineW", KERNEL32, instructions, relocations);
        instructions.extend([
            // plan-85: GetCommandLineW's `LPWSTR` result is a C result (`rax`).
            abi::move_register(abi::mfb_arg(0), abi::c_return(0)), // lpCmdLine
            abi::add_immediate(abi::mfb_arg(1), abi::stack_pointer(), ARGC), // &argc
        ]);
        call_external(
            from,
            "CommandLineToArgvW",
            SHELL32,
            instructions,
            relocations,
        );
        instructions.extend([
            // plan-85: CommandLineToArgvW's `LPWSTR*` result is a C result (`rax`); the
            // argc math below reuses `return_register()` as a working register (loaded
            // from the ARGC slot), which is NOT a C result and stays as-is.
            abi::store_u64(abi::c_return(0), abi::stack_pointer(), WARGV),
            // arena-alloc the UTF-8 argv array: (argc+1) * 8 bytes.
            abi::load_u32(abi::return_register(), abi::stack_pointer(), ARGC),
            abi::add_immediate(abi::return_register(), abi::return_register(), 1),
            abi::shift_left_immediate(abi::return_register(), abi::return_register(), 3),
            abi::move_immediate(abi::mfb_arg(1), "Integer", "8"),
            abi::branch_link(crate::codegen::error::constants::ARENA_ALLOC_SYMBOL),
        ]);
        relocations.push(CodeRelocation {
            from: from.to_string(),
            to: crate::codegen::error::constants::ARENA_ALLOC_SYMBOL.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
        instructions.extend([
            abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), U8ARGV),
            abi::store_u64(abi::ZERO, abi::stack_pointer(), IDX),
            // for (idx = 0; idx < argc; idx++) argv8[idx] = utf8(wargv[idx]).
            abi::label(&loop_top),
            abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), IDX),
            abi::load_u32(abi::mfb_arg(1), abi::stack_pointer(), ARGC),
            abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(1)),
            abi::branch_ge(&loop_done),
            // wargv[idx] → WARG slot for the marshal helper.
            abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), WARGV),
            abi::shift_left_immediate(abi::mfb_arg(3), abi::mfb_arg(0), 3),
            abi::add_registers(abi::mfb_arg(2), abi::mfb_arg(2), abi::mfb_arg(3)),
            abi::load_u64(abi::mfb_arg(2), abi::mfb_arg(2), 0),
            abi::store_u64(abi::mfb_arg(2), abi::stack_pointer(), WARG),
        ]);
        arena_alloc_to_slot(from, ARG_CAP, U8ARG, instructions, relocations);
        emit_wide_slot_to_utf8(from, WARG, U8ARG, ARG_CAP, instructions, relocations);
        instructions.extend([
            // argv8[idx] = u8arg.
            abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), U8ARGV),
            abi::load_u64(abi::mfb_arg(1), abi::stack_pointer(), IDX),
            abi::shift_left_immediate(abi::mfb_arg(2), abi::mfb_arg(1), 3),
            abi::add_registers(abi::mfb_arg(0), abi::mfb_arg(0), abi::mfb_arg(2)),
            abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), U8ARG),
            abi::store_u64(abi::mfb_arg(2), abi::mfb_arg(0), 0),
            abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(1), 1),
            abi::store_u64(abi::mfb_arg(1), abi::stack_pointer(), IDX),
            abi::branch(&loop_top),
            abi::label(&loop_done),
            // argv8[argc] = NULL.
            abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), U8ARGV),
            abi::load_u32(abi::mfb_arg(1), abi::stack_pointer(), ARGC),
            abi::shift_left_immediate(abi::mfb_arg(1), abi::mfb_arg(1), 3),
            abi::add_registers(abi::mfb_arg(0), abi::mfb_arg(0), abi::mfb_arg(1)),
            abi::store_u64(abi::ZERO, abi::mfb_arg(0), 0),
            // LocalFree(wargv) — CommandLineToArgvW returns a single LocalAlloc block.
            abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), WARGV),
        ]);
        call_external(from, "LocalFree", KERNEL32, instructions, relocations);
        instructions.extend([
            // Leave argc in ARG[0], argv in ARG[1] for the shared entry stores.
            abi::load_u32(abi::mfb_arg(0), abi::stack_pointer(), ARGC),
            abi::load_u64(abi::mfb_arg(1), abi::stack_pointer(), U8ARGV),
            abi::add_stack(0x70),
        ]);
        Ok(())
    }

    // --- the machine floor -------------------------------------------------

    fn emit_program_entry(
        &self,
        spec: &ProgramEntrySpec<'_>,
        platform_imports: &HashMap<String, String>,
    ) -> Result<CodeFunction, String> {
        crate::codegen::engine::function::lower_program_entry(
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
            spec.needs_winsock,
            spec.seed_presentation_mode_offset,
        )
    }

    fn emit_program_exit(
        &self,
        from: &str,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // App mode (plan-66-J): the worker program routes completion through the
        // winapp finish helper (transcript readback/dump + ExitProcess) instead of
        // a bare ExitProcess, so the window transcript is observable. Console
        // programs (and the headless app fallback inside finish) ExitProcess here.
        if from == crate::codegen::error::constants::MACAPP_PROGRAM_SYMBOL {
            instructions.push(abi::branch_link(app::FINISH_SYMBOL));
            relocations.push(CodeRelocation {
                from: from.to_string(),
                to: app::FINISH_SYMBOL.to_string(),
                kind: RelocIntent::Call,
                binding: "internal".to_string(),
                library: None,
            });
            instructions.extend([abi::branch_self(), abi::return_()]);
            return Ok(());
        }
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
            abi::add_immediate(abi::mfb_arg(0), abi::stack_pointer(), 0),
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
            abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), 0),
            abi::store_u64(
                abi::mfb_arg(0),
                crate::codegen::error::constants::ARENA_STATE_REGISTER,
                crate::codegen::error::constants::ARENA_START_TIME_OFFSET,
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
        //
        // **No shadow-space frame here, and that is measured rather than an oversight.**
        // `VirtualAlloc` is a Win64 call like any other and its callee may spill four
        // registers into the caller's 32 bytes — but reserving them here regresses app
        // mode into `ErrOutOfMemory` on its first allocation, while leaving the console
        // path green. So this emitter's caller is not a plain body with an aligned,
        // sp-relative frame, and a `sub rsp` inside it is not the right fix. See
        // bug-479, which owns the remaining Windows canvas fault and this seam with it.
        instructions.extend([
            abi::move_immediate(abi::mfb_arg(0), "Integer", "0"),
            abi::move_register(abi::mfb_arg(1), size_reg),
            abi::move_immediate(abi::mfb_arg(2), "Integer", MEM_COMMIT_RESERVE),
            abi::move_immediate(abi::mfb_arg(3), "Integer", PAGE_READWRITE),
            abi::branch_link("VirtualAlloc"),
            // plan-85: VirtualAlloc returns the block pointer as a C result (`rax` =
            // `%retC`), not the aligned MFB result register — read it via `c_return`
            // into `return_register()` (this helper's own return). VirtualAlloc returns
            // NULL(0) on failure; the shared arena caller routes a *negative* result to
            // the OOM path (the negative-errno convention the Linux backend returns), so
            // normalize 0 → -1.
            abi::move_register(abi::return_register(), abi::c_return(0)),
            abi::compare_immediate(abi::c_return(0), "0"),
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
        // No shadow-space frame, for the reason `emit_arena_map` records (bug-479).
        instructions.extend([
            abi::move_register(abi::mfb_arg(0), abi::return_register()),
            abi::move_immediate(abi::mfb_arg(1), "Integer", "0"),
            abi::move_immediate(abi::mfb_arg(2), "Integer", MEM_RELEASE),
            abi::branch_link("VirtualFree"),
        ]);
        Ok(())
    }

    fn emit_external_call(
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
            // Win64's aligned MFB result bank starts at `rcx` (plan-85-A), not at
            // the C return `rax`, so the shared emitter stages `mov rcx, rax`
            // after the call exactly as it does for SysV's `rdi`.
            "windows-x86_64",
            from,
            base,
            platform_imports,
            instructions,
            relocations,
        )
    }

    fn emit_variadic_external_call(
        &self,
        base: &str,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // Win64 has no separate variadic marker (unlike SysV's `al`); a plain
        // call suffices.
        self.emit_external_call(base, from, platform_imports, instructions, relocations)
    }

    fn emit_lib_open(
        &self,
        filename_symbol: &str,
        vendored: bool,
        from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // bug-431: Windows has no `dlopen`. Load the DLL with `LoadLibraryExA`,
        // leaving the module handle in `return_register()` (0 on failure) so the
        // shared initializer's failure check and slot store are unchanged.
        //
        // `filename_symbol` names a read-only C string holding the resolved
        // `dlopen` filename — a bare DLL name. For a **vendored** library that
        // file lives in the exe-relative `vendor/` directory, which the default
        // DLL search never consults, so build the absolute path
        // `<exe_dir>\vendor\<name>` at load time and pass
        // `LOAD_WITH_ALTERED_SEARCH_PATH` (which requires an absolute path and
        // also resolves the DLL's own dependencies from `vendor/`). A **system**
        // library is loaded by bare name through the default search.
        //
        // This runs once at startup, single-threaded, so the shared writable
        // `WIN_LINK_PATHBUF` scratch is safe to reuse across libraries. The buffer
        // address is re-materialized (a RIP-relative `lea`) before each call, so
        // no register needs to survive one — only the buffer's memory does.
        use crate::codegen::link::thunk::{
            emit_data_address, WIN_LINK_PATHBUF_BYTES, WIN_LINK_PATHBUF_SYMBOL,
            WIN_LINK_VENDORSEP_SYMBOL,
        };
        if vendored {
            // GetModuleFileNameA(NULL, buf, WIN_LINK_PATHBUF_BYTES) -> full exe path.
            emit_data_address(
                from,
                abi::mfb_arg(1),
                WIN_LINK_PATHBUF_SYMBOL,
                instructions,
                relocations,
            );
            instructions.extend([
                abi::move_immediate(abi::mfb_arg(0), "Integer", "0"),
                abi::move_immediate(
                    abi::mfb_arg(2),
                    "Integer",
                    &WIN_LINK_PATHBUF_BYTES.to_string(),
                ),
            ]);
            call_external(
                from,
                "GetModuleFileNameA",
                KERNEL32,
                instructions,
                relocations,
            );
            // PathRemoveFileSpecA(buf) -> strip the trailing `\<exe>`, leaving <exe_dir>.
            emit_data_address(
                from,
                abi::mfb_arg(0),
                WIN_LINK_PATHBUF_SYMBOL,
                instructions,
                relocations,
            );
            call_external(
                from,
                "PathRemoveFileSpecA",
                SHLWAPI,
                instructions,
                relocations,
            );
            // lstrcatA(buf, "\vendor\") then lstrcatA(buf, name).
            for append_symbol in [WIN_LINK_VENDORSEP_SYMBOL, filename_symbol] {
                emit_data_address(
                    from,
                    abi::mfb_arg(0),
                    WIN_LINK_PATHBUF_SYMBOL,
                    instructions,
                    relocations,
                );
                emit_data_address(
                    from,
                    abi::mfb_arg(1),
                    append_symbol,
                    instructions,
                    relocations,
                );
                call_external(from, "lstrcatA", KERNEL32, instructions, relocations);
            }
            // LoadLibraryExA(buf, NULL, LOAD_WITH_ALTERED_SEARCH_PATH).
            emit_data_address(
                from,
                abi::mfb_arg(0),
                WIN_LINK_PATHBUF_SYMBOL,
                instructions,
                relocations,
            );
            instructions.extend([
                abi::move_immediate(abi::mfb_arg(1), "Integer", "0"),
                abi::move_immediate(abi::mfb_arg(2), "Integer", LOAD_WITH_ALTERED_SEARCH_PATH),
            ]);
            call_external(from, "LoadLibraryExA", KERNEL32, instructions, relocations);
        } else {
            // System DLL: LoadLibraryExA(name, NULL, 0) — resolved by the default
            // search order (no `vendor/` involvement).
            emit_data_address(
                from,
                abi::mfb_arg(0),
                filename_symbol,
                instructions,
                relocations,
            );
            instructions.extend([
                abi::move_immediate(abi::mfb_arg(1), "Integer", "0"),
                abi::move_immediate(abi::mfb_arg(2), "Integer", "0"),
            ]);
            call_external(from, "LoadLibraryExA", KERNEL32, instructions, relocations);
        }
        // Stage the C return (`rax`) into the aligned MFB result register the
        // shared loop reads, exactly like `emit_arena_map` (plan-85).
        instructions.push(abi::move_register(abi::return_register(), abi::c_return(0)));
        Ok(())
    }

    fn emit_lib_get_sym(
        &self,
        handle_reg: &str,
        symbol_symbol: &str,
        from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // bug-431: GetProcAddress(handle, symbolName) — the Windows `dlsym`. The
        // handle is a callee-saved vreg the shared loop keeps live across these
        // calls; the resolved address lands in `return_register()`.
        use crate::codegen::link::thunk::emit_data_address;
        instructions.push(abi::move_register(abi::mfb_arg(0), handle_reg));
        emit_data_address(
            from,
            abi::mfb_arg(1),
            symbol_symbol,
            instructions,
            relocations,
        );
        call_external(from, "GetProcAddress", KERNEL32, instructions, relocations);
        instructions.push(abi::move_register(abi::return_register(), abi::c_return(0)));
        Ok(())
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
            abi::store_u64(abi::mfb_arg(1), abi::stack_pointer(), 0x30), // save buf
            abi::store_u64(abi::mfb_arg(2), abi::stack_pointer(), 0x38), // save len
            // Resolve the destination handle. fd 1 (stdout) and 2 (stderr) are the
            // console POSIX fds and resolve via GetStdHandle(-(fd+10)); any larger
            // value is already a Win32 file HANDLE (CreateFileW) — fs writes pass
            // the handle straight through here (CreateFileW never returns 1/2).
            abi::compare_immediate(abi::mfb_arg(0), "2"),
            abi::branch_gt(&file_handle),
            // console: nStdHandle = -(fd + 10), built without a negative immediate.
            // ARG[1] (rdx) is a free caller-saved temp now that buf is saved; the
            // SCRATCH pool must not be used — its Win64 realizations (rbx/rsi/rdi)
            // are callee-saved and would corrupt registers the caller keeps live.
            abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(0), 10), // fd + 10
            abi::move_immediate(abi::mfb_arg(0), "Integer", "0"),
            abi::subtract_registers(abi::mfb_arg(0), abi::mfb_arg(0), abi::mfb_arg(1)), // -(fd+10)
        ]);
        call_external(from, "GetStdHandle", KERNEL32, instructions, relocations);
        instructions.extend([
            // plan-85: GetStdHandle returns the console HANDLE as a C result (`rax` =
            // `%retC`), not the aligned MFB result register — read it from `c_return`.
            abi::store_u64(abi::c_return(0), abi::stack_pointer(), 0x40), // hFile
            abi::branch(&have_handle),
            abi::label(&file_handle),
            abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), 0x40), // hFile = handle directly
            abi::label(&have_handle),
            abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x20), // lpOverlapped = NULL
            abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), 0x40), // hFile
            abi::load_u64(abi::mfb_arg(1), abi::stack_pointer(), 0x30), // lpBuffer
            abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), 0x38), // nNumberOfBytesToWrite
            abi::add_immediate(abi::mfb_arg(3), abi::stack_pointer(), 0x28), // &lpBytesWritten
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
            // plan-85: the BOOL is a C result (`rax` = `%retC`), read via `c_return`;
            // the -1 / byte-count below is this helper's own MFB return (`return_register`).
            abi::compare_immediate(abi::c_return(0), "0"),
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

    fn emit_heap_alloc(
        &self,
        from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // malloc(size) → HeapAlloc(GetProcessHeap(), 0, size). size in ARG[0], the
        // block pointer in the return register (0 on failure). Balanced frame so the
        // caller sees the plain malloc contract. plan-66-C.
        instructions.extend([
            abi::subtract_stack(0x30),
            abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), 0x28), // save size
        ]);
        call_external(from, "GetProcessHeap", KERNEL32, instructions, relocations);
        instructions.extend([
            abi::move_register(abi::mfb_arg(0), abi::c_return(0)), // hHeap (C result)
            abi::move_immediate(abi::mfb_arg(1), "Integer", "0"),  // dwFlags
            abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), 0x28), // dwBytes
        ]);
        call_external(from, "HeapAlloc", KERNEL32, instructions, relocations);
        instructions.extend([
            // plan-85: HeapAlloc returns the block pointer as a C result (`rax`); this
            // helper's `malloc` contract returns it in the aligned MFB result register.
            abi::move_register(abi::return_register(), abi::c_return(0)),
            abi::add_stack(0x30),
        ]);
        Ok(())
    }

    fn emit_heap_free(
        &self,
        from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // free(ptr) → HeapFree(GetProcessHeap(), 0, ptr). ptr in ARG[0]. plan-66-C.
        instructions.extend([
            abi::subtract_stack(0x30),
            abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), 0x28), // save ptr
        ]);
        call_external(from, "GetProcessHeap", KERNEL32, instructions, relocations);
        instructions.extend([
            abi::move_register(abi::mfb_arg(0), abi::c_return(0)), // hHeap (C result)
            abi::move_immediate(abi::mfb_arg(1), "Integer", "0"),  // dwFlags
            abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), 0x28), // lpMem
        ]);
        call_external(from, "HeapFree", KERNEL32, instructions, relocations);
        instructions.push(abi::add_stack(0x30));
        Ok(())
    }

    fn emit_poll_input(
        &self,
        from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // poll(&pollfd, 1, timeout) for fd 0: the shared caller passes the pollfd
        // pointer in the return register, nfds=1 in ARG[1], and the timeout (ms, < 0 =
        // infinite) in ARG[2]. Windows has no poll(). Two cases by stdin handle type:
        //
        //  * a console handle → WaitForSingleObject(hStdin, timeout) signals on input.
        //  * a PIPE (app mode redirects fd 0 to the window input pipe, plan-66-J-4) →
        //    WaitForSingleObject is USELESS: an anonymous pipe read handle is signaled
        //    whether or not bytes are queued, so it would report ready with no input.
        //    PeekNamedPipe reports the actual queued byte count; poll it in a Sleep(10)
        //    countdown until data arrives or the timeout elapses.
        //
        // Result: WAIT_OBJECT_0 / bytes-available → 1 (ready), timeout → 0, error → -1
        // (the caller routes <0 to its retry/error tail). plan-66-C / plan-66-J-4.
        let n = instructions.len();
        let ready = format!("{from}_poll_ready_{n}");
        let timeout = format!("{from}_poll_timeout_{n}");
        let done = format!("{from}_poll_done_{n}");
        let pipe_loop = format!("{from}_poll_pipe_{n}");
        let pipe_sleep = format!("{from}_poll_sleep_{n}");
        let console = format!("{from}_poll_console_{n}");
        let inf_set = format!("{from}_poll_inf_{n}");
        // Frame (multiple of 16, preserves the caller helper's alignment like the
        // original 0x30): &avail@0x20, NULL@0x28 (PeekNamedPipe stack args 5/6),
        // avail@0x30, hStdin@0x38, remaining@0x40, infinite@0x48.
        const F: usize = 0x50;
        instructions.extend([
            abi::subtract_stack(F),
            abi::store_u64(abi::mfb_arg(2), abi::stack_pointer(), 0x40), // remaining = timeout
            // infinite = (timeout < 0) ? 1 : 0 (poll() infinite semantics).
            abi::move_immediate(abi::mfb_arg(0), "Integer", "0"),
            abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), 0x48),
            abi::compare_immediate(abi::mfb_arg(2), "0"),
            abi::branch_ge(&inf_set),
            abi::move_immediate(abi::mfb_arg(0), "Integer", "1"),
            abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), 0x48),
            abi::label(&inf_set),
            // GetStdHandle(STD_INPUT_HANDLE = -10) without a negative immediate.
            abi::move_immediate(abi::mfb_arg(0), "Integer", "0"),
            abi::subtract_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 10),
        ]);
        call_external(from, "GetStdHandle", KERNEL32, instructions, relocations);
        instructions.extend([
            abi::store_u64(abi::c_return(0), abi::stack_pointer(), 0x38), // hStdin (C result)
            // GetFileType(hStdin): FILE_TYPE_PIPE (3) → the app-mode input pipe.
            abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), 0x38),
        ]);
        call_external(from, "GetFileType", KERNEL32, instructions, relocations);
        instructions.extend([
            abi::compare_immediate(abi::c_return(0), "3"), // FILE_TYPE_PIPE (C result)
            abi::branch_ne(&console),
            // ---- pipe path: PeekNamedPipe countdown ----
            abi::label(&pipe_loop),
            abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x30), // avail = 0
            abi::add_immediate(abi::mfb_arg(0), abi::stack_pointer(), 0x30), // &avail
            abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), 0x20), // 5th arg lpTotalBytesAvail
            abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x28),       // 6th arg NULL
            abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), 0x38),  // hStdin
            abi::move_immediate(abi::mfb_arg(1), "Integer", "0"),        // lpBuffer NULL
            abi::move_immediate(abi::mfb_arg(2), "Integer", "0"),        // nBufferSize 0
            abi::move_immediate(abi::mfb_arg(3), "Integer", "0"),        // lpBytesRead NULL
        ]);
        call_external(from, "PeekNamedPipe", KERNEL32, instructions, relocations);
        instructions.extend([
            abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), 0x30), // avail
            abi::compare_immediate(abi::mfb_arg(0), "0"),
            abi::branch_ne(&ready), // bytes queued → ready
            // not ready: infinite → keep polling; else if remaining <= 0 → timeout.
            abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), 0x48), // infinite
            abi::compare_immediate(abi::mfb_arg(0), "0"),
            abi::branch_ne(&pipe_sleep),
            abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), 0x40), // remaining
            abi::compare_immediate(abi::mfb_arg(0), "0"),
            abi::branch_le(&timeout),
            abi::label(&pipe_sleep),
            abi::move_immediate(abi::mfb_arg(0), "Integer", "10"), // Sleep(10 ms)
        ]);
        call_external(from, "Sleep", KERNEL32, instructions, relocations);
        instructions.extend([
            // Decrement the countdown unless infinite.
            abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), 0x48), // infinite
            abi::compare_immediate(abi::mfb_arg(0), "0"),
            abi::branch_ne(&pipe_loop),
            abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), 0x40), // remaining
            abi::subtract_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 10),
            abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), 0x40),
            abi::branch(&pipe_loop),
            // ---- console path: WaitForSingleObject ----
            abi::label(&console),
            abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), 0x38), // hStdin
            abi::load_u64(abi::mfb_arg(1), abi::stack_pointer(), 0x40), // dwMilliseconds
        ]);
        call_external(
            from,
            "WaitForSingleObject",
            KERNEL32,
            instructions,
            relocations,
        );
        instructions.extend([
            abi::compare_immediate(abi::c_return(0), "0"), // WAIT_OBJECT_0 (C result)
            abi::branch_eq(&ready),
            abi::compare_immediate(abi::c_return(0), "258"), // WAIT_TIMEOUT (C result)
            abi::branch_eq(&timeout),
            abi::move_immediate(abi::return_register(), "Integer", "0"),
            abi::subtract_immediate(abi::return_register(), abi::return_register(), 1), // -1 error
            abi::branch(&done),
            abi::label(&ready),
            abi::move_immediate(abi::return_register(), "Integer", "1"),
            abi::branch(&done),
            abi::label(&timeout),
            abi::move_immediate(abi::return_register(), "Integer", "0"),
            abi::label(&done),
            abi::add_stack(F),
        ]);
        Ok(())
    }

    fn emit_is_terminal(
        &self,
        from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // isatty(fd): fd in ARG[0]; return nonzero iff fd is a terminal. On Windows
        // GetConsoleMode succeeds only for a real console handle, so it IS the
        // isatty test. Resolve fd → std HANDLE (GetStdHandle(-(fd+10))), then
        // GetConsoleMode(handle, &mode).
        let n = instructions.len();
        let yes = format!("{from}_isatty_yes_{n}");
        let done = format!("{from}_isatty_done_{n}");
        instructions.extend([
            abi::subtract_stack(0x30), // shadow + &mode slot at 0x28
            abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(0), 10),
            abi::move_immediate(abi::mfb_arg(0), "Integer", "0"),
            abi::subtract_registers(abi::mfb_arg(0), abi::mfb_arg(0), abi::mfb_arg(1)), // -(fd+10)
        ]);
        call_external(from, "GetStdHandle", KERNEL32, instructions, relocations);
        instructions.extend([
            abi::move_register(abi::mfb_arg(0), abi::c_return(0)), // hConsole (C result)
            abi::add_immediate(abi::mfb_arg(1), abi::stack_pointer(), 0x28), // &mode
        ]);
        call_external(from, "GetConsoleMode", KERNEL32, instructions, relocations);
        instructions.extend([
            abi::compare_immediate(abi::c_return(0), "0"), // GetConsoleMode BOOL (C result)
            abi::branch_ne(&yes),
            abi::move_immediate(abi::return_register(), "Integer", "0"),
            abi::branch(&done),
            abi::label(&yes),
            abi::move_immediate(abi::return_register(), "Integer", "1"),
            abi::label(&done),
            abi::add_stack(0x30),
        ]);
        Ok(())
    }

    fn emit_terminal_size(
        &self,
        from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // ioctl(fd, TIOCGWINSZ, &winsize): fd in ARG[0], request in ARG[1]
        // (ignored), the winsize dst in ARG[2] (ws_row @0, ws_col @2). Return 0 on
        // success. Windows: GetConsoleScreenBufferInfo → srWindow; the window size
        // is Right-Left+1 (cols) by Bottom-Top+1 (rows). srWindow.Left@10, Top@12,
        // Right@14, Bottom@16 within CONSOLE_SCREEN_BUFFER_INFO (SHORTs). NOT dwSize
        // (the scrollback buffer). Frame: shadow + saved winsize dst (0x28) +
        // CONSOLE_SCREEN_BUFFER_INFO buffer (0x30, 22 bytes).
        const FRAME: usize = 0x50;
        const DST_SLOT: usize = 0x28;
        const CSBI_OFF: usize = 0x30;
        let n = instructions.len();
        let ok = format!("{from}_tsize_ok_{n}");
        let done = format!("{from}_tsize_done_{n}");
        instructions.extend([
            abi::subtract_stack(FRAME),
            abi::store_u64(abi::mfb_arg(2), abi::stack_pointer(), DST_SLOT), // save winsize dst
            abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(0), 10),
            abi::move_immediate(abi::mfb_arg(0), "Integer", "0"),
            abi::subtract_registers(abi::mfb_arg(0), abi::mfb_arg(0), abi::mfb_arg(1)), // -(fd+10)
        ]);
        call_external(from, "GetStdHandle", KERNEL32, instructions, relocations);
        instructions.extend([
            abi::move_register(abi::mfb_arg(0), abi::c_return(0)), // hConsole (C result)
            abi::add_immediate(abi::mfb_arg(1), abi::stack_pointer(), CSBI_OFF), // &csbi
        ]);
        call_external(
            from,
            "GetConsoleScreenBufferInfo",
            KERNEL32,
            instructions,
            relocations,
        );
        instructions.extend([
            abi::compare_immediate(abi::c_return(0), "0"), // Win32 BOOL (C result)
            abi::branch_ne(&ok),                           // BOOL != 0 → success
            abi::move_immediate(abi::return_register(), "Integer", "1"), // failure (nonzero)
            abi::branch(&done),
            abi::label(&ok),
            // rows = Bottom(+16) - Top(+12) + 1; cols = Right(+14) - Left(+10) + 1.
            abi::load_u16(abi::mfb_arg(0), abi::stack_pointer(), CSBI_OFF + 16), // Bottom
            abi::load_u16(abi::mfb_arg(1), abi::stack_pointer(), CSBI_OFF + 12), // Top
            abi::subtract_registers(abi::mfb_arg(0), abi::mfb_arg(0), abi::mfb_arg(1)),
            abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1), // rows
            abi::load_u16(abi::mfb_arg(1), abi::stack_pointer(), CSBI_OFF + 14), // Right
            abi::load_u16(abi::mfb_arg(2), abi::stack_pointer(), CSBI_OFF + 10), // Left
            abi::subtract_registers(abi::mfb_arg(1), abi::mfb_arg(1), abi::mfb_arg(2)),
            abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(1), 1), // cols
            abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), DST_SLOT), // winsize dst
            abi::store_u16(abi::mfb_arg(0), abi::mfb_arg(2), 0),     // ws_row
            abi::store_u16(abi::mfb_arg(1), abi::mfb_arg(2), 2),     // ws_col
            abi::move_immediate(abi::return_register(), "Integer", "0"), // success
            abi::label(&done),
            abi::add_stack(FRAME),
        ]);
        Ok(())
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
            abi::mfb_arg(0),
            abi::stack_pointer(),
            MARSHAL_WBUF_SLOT,
        ));
        call_external(
            from,
            "GetFileAttributesW",
            KERNEL32,
            instructions,
            relocations,
        );
        instructions.extend([
            // plan-85: GetFileAttributesW's DWORD is a C result (`rax`); `>>31` is bit
            // 31 (missing=1/exists=0), landing in this helper's MFB return register.
            abi::shift_right_immediate(abi::return_register(), abi::c_return(0), 31),
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
            abi::store_u64(abi::mfb_arg(1), abi::stack_pointer(), STATBUF_SLOT),
        ]);
        emit_marshal_path(from, instructions, relocations);
        instructions.push(abi::load_u64(
            abi::mfb_arg(0),
            abi::stack_pointer(),
            MARSHAL_WBUF_SLOT,
        ));
        call_external(
            from,
            "GetFileAttributesW",
            KERNEL32,
            instructions,
            relocations,
        );
        instructions.extend([
            // ARG[1] (rdx) is a free caller-saved temp for the buffer pointer, and
            // is distinct from the return register (rax) that holds the attributes.
            // The SCRATCH pool must not be used — callee-saved on Win64.
            abi::load_u64(abi::mfb_arg(1), abi::stack_pointer(), STATBUF_SLOT),
            abi::store_u64(abi::c_return(0), abi::mfb_arg(1), 0), // GetFileAttributesW DWORD (C result)
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
        emit_dir_path_query(
            from,
            "GetCurrentDirectoryW",
            false,
            instructions,
            relocations,
        );
        Ok(())
    }

    fn emit_environ_pointer(
        &self,
        from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // Synthesize a POSIX `char**` (NULL-terminated array of UTF-8 "KEY=VALUE"
        // C-strings) from GetEnvironmentStringsW, so the shared `lower_environ`
        // walker is reused unchanged. The wide block is "K=V\0K=V\0…\0\0". Windows
        // prepends hidden per-drive `=C:=…` entries whose key is empty; the shared
        // walker splits on the first `=`, so those must be skipped (a leading `=`).
        // Two passes over the block: count non-drive entries, then marshal each into
        // the arena and fill the pointer array. Registers do not survive the
        // arena_alloc / WideCharToMultiByte calls, so all loop state lives in stack
        // slots. plan-66-B.
        //
        // Frame (subtract_stack(0x70)): shadow [0x00..0x20), marshal stack args
        // [0x20..0x40), [0x40] block base, [0x48] array base, [0x50] cursor,
        // [0x58] index, [0x60] per-entry UTF-8 buffer, [0x68] next cursor.
        const BLOCK: usize = 0x40;
        const ARRAY: usize = 0x48;
        const CURSOR: usize = 0x50;
        const IDX: usize = 0x58;
        const U8BUF: usize = 0x60;
        const NEXT: usize = 0x68;
        // Fixed per-entry UTF-8 buffer: a single VAR=VALUE caps at ~32767 wchars →
        // ≤ ~128 KiB UTF-8; use 128 KiB with a matching WideCharToMultiByte cap so
        // the call can never overrun the buffer.
        const ENTRY_CAP: &str = "131072";
        let n = instructions.len();
        let l = |s: &str| format!("{from}_environ_{s}_{n}");
        let (count_loop, count_scan, count_scan_done, count_next, count_done) =
            (l("cl"), l("cs"), l("csd"), l("cn"), l("cd"));
        let (fill_loop, fill_scan, fill_scan_done, fill_skip, fill_done) =
            (l("fl"), l("fs"), l("fsd"), l("fk"), l("fd"));
        instructions.push(abi::subtract_stack(0x70));
        call_external(
            from,
            "GetEnvironmentStringsW",
            KERNEL32,
            instructions,
            relocations,
        );
        instructions.extend([
            // GetEnvironmentStringsW returns the block pointer as a C result (`rax`).
            abi::store_u64(abi::c_return(0), abi::stack_pointer(), BLOCK),
            abi::move_register(abi::SCRATCH[0], abi::c_return(0)), // cursor (entry start)
            abi::move_immediate(abi::SCRATCH[1], "Integer", "0"),  // count
            // --- Pass 1: count non-drive entries (no calls, so ARG regs survive) ---
            abi::label(&count_loop),
            abi::load_u16(abi::SCRATCH[3], abi::SCRATCH[0], 0), // first wide char
            abi::compare_immediate(abi::SCRATCH[3], "0"),
            abi::branch_eq(&count_done), // double-NUL → end of block
            abi::move_register(abi::SCRATCH[2], abi::SCRATCH[0]), // scan
            abi::label(&count_scan),
            abi::load_u16(abi::SCRATCH[3], abi::SCRATCH[2], 0),
            abi::compare_immediate(abi::SCRATCH[3], "0"),
            abi::branch_eq(&count_scan_done),
            abi::add_immediate(abi::SCRATCH[2], abi::SCRATCH[2], 2),
            abi::branch(&count_scan),
            abi::label(&count_scan_done),
            // reload the first char; skip if '=' (0x3D = 61, a hidden drive entry).
            abi::load_u16(abi::SCRATCH[3], abi::SCRATCH[0], 0),
            abi::compare_immediate(abi::SCRATCH[3], "61"),
            abi::branch_eq(&count_next),
            abi::add_immediate(abi::SCRATCH[1], abi::SCRATCH[1], 1), // count++
            abi::label(&count_next),
            abi::add_immediate(abi::SCRATCH[0], abi::SCRATCH[2], 2), // next entry
            abi::branch(&count_loop),
            abi::label(&count_done),
        ]);
        // --- Allocate the (count+1) pointer array ---
        instructions.extend([
            abi::add_immediate(abi::return_register(), abi::SCRATCH[1], 1),
            abi::shift_left_immediate(abi::return_register(), abi::return_register(), 3), // *8
            abi::move_immediate(abi::SCRATCH[1], "Integer", "8"),
            abi::branch_link(crate::codegen::error::constants::ARENA_ALLOC_SYMBOL),
        ]);
        relocations.push(CodeRelocation {
            from: from.to_string(),
            to: crate::codegen::error::constants::ARENA_ALLOC_SYMBOL.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
        instructions.extend([
            abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), ARRAY),
            abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), BLOCK),
            abi::store_u64(abi::SCRATCH[0], abi::stack_pointer(), CURSOR),
            abi::store_u64(abi::ZERO, abi::stack_pointer(), IDX),
            // --- Pass 2: marshal each non-drive entry into the array ---
            abi::label(&fill_loop),
            abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), CURSOR),
            abi::load_u16(abi::SCRATCH[1], abi::SCRATCH[0], 0), // first char
            abi::compare_immediate(abi::SCRATCH[1], "0"),
            abi::branch_eq(&fill_done),
            abi::move_register(abi::SCRATCH[2], abi::SCRATCH[0]), // scan
            abi::label(&fill_scan),
            abi::load_u16(abi::SCRATCH[3], abi::SCRATCH[2], 0),
            abi::compare_immediate(abi::SCRATCH[3], "0"),
            abi::branch_eq(&fill_scan_done),
            abi::add_immediate(abi::SCRATCH[2], abi::SCRATCH[2], 2),
            abi::branch(&fill_scan),
            abi::label(&fill_scan_done),
            abi::add_immediate(abi::SCRATCH[3], abi::SCRATCH[2], 2), // next cursor = NUL + 2
            abi::store_u64(abi::SCRATCH[3], abi::stack_pointer(), NEXT),
            // skip drive entries (leading '=', ARG[1] still holds the first char).
            abi::compare_immediate(abi::SCRATCH[1], "61"),
            abi::branch_eq(&fill_skip),
        ]);
        arena_alloc_to_slot(from, ENTRY_CAP, U8BUF, instructions, relocations);
        // WideCharToMultiByte(CP_UTF8, 0, [CURSOR], -1, [U8BUF], ENTRY_CAP, NULL, NULL).
        emit_wide_slot_to_utf8(from, CURSOR, U8BUF, ENTRY_CAP, instructions, relocations);
        instructions.extend([
            // array[idx] = u8buf; idx++.
            abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), ARRAY),
            abi::load_u64(abi::SCRATCH[1], abi::stack_pointer(), IDX),
            abi::shift_left_immediate(abi::SCRATCH[2], abi::SCRATCH[1], 3),
            abi::add_registers(abi::SCRATCH[0], abi::SCRATCH[0], abi::SCRATCH[2]),
            abi::load_u64(abi::SCRATCH[2], abi::stack_pointer(), U8BUF),
            abi::store_u64(abi::SCRATCH[2], abi::SCRATCH[0], 0),
            abi::add_immediate(abi::SCRATCH[1], abi::SCRATCH[1], 1),
            abi::store_u64(abi::SCRATCH[1], abi::stack_pointer(), IDX),
            abi::label(&fill_skip),
            abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), NEXT),
            abi::store_u64(abi::SCRATCH[0], abi::stack_pointer(), CURSOR),
            abi::branch(&fill_loop),
            abi::label(&fill_done),
            // NULL-terminate the array at [idx].
            abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), ARRAY),
            abi::load_u64(abi::SCRATCH[1], abi::stack_pointer(), IDX),
            abi::shift_left_immediate(abi::SCRATCH[1], abi::SCRATCH[1], 3),
            abi::add_registers(abi::SCRATCH[0], abi::SCRATCH[0], abi::SCRATCH[1]),
            abi::store_u64(abi::ZERO, abi::SCRATCH[0], 0),
            // FreeEnvironmentStringsW(block).
            abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), BLOCK),
        ]);
        call_external(
            from,
            "FreeEnvironmentStringsW",
            KERNEL32,
            instructions,
            relocations,
        );
        instructions.extend([
            abi::load_u64(abi::return_register(), abi::stack_pointer(), ARRAY),
            abi::add_stack(0x70),
        ]);
        Ok(())
    }

    fn emit_enable_vt_output(
        &self,
        from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // term::on calls this before its first ANSI write. Classic conhost does not
        // interpret VT sequences unless ENABLE_VIRTUAL_TERMINAL_PROCESSING (0x04) is
        // set on the stdout console mode; Windows Terminal sets it by default, but
        // enabling it is harmless there. Resolve STD_OUTPUT (GetStdHandle(-11)), read
        // the current mode, OR in the VT bit, and write it back. Best-effort: if
        // GetConsoleMode fails (stdout redirected to a file/pipe, not a console),
        // skip the SetConsoleMode so a non-console stdout is left untouched. Uses a
        // self-contained frame (shadow space + a mode slot + a saved-handle slot).
        let n = instructions.len();
        let skip = format!("{from}_vt_skip_{n}");
        instructions.extend([
            abi::subtract_stack(0x30),
            // GetStdHandle(STD_OUTPUT_HANDLE = -11), built without a negative immediate.
            abi::move_immediate(abi::mfb_arg(0), "Integer", "0"),
            abi::subtract_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 11),
        ]);
        call_external(from, "GetStdHandle", KERNEL32, instructions, relocations);
        instructions.extend([
            abi::store_u64(abi::c_return(0), abi::stack_pointer(), 0x28), // save handle (C result)
            abi::move_register(abi::mfb_arg(0), abi::c_return(0)),
            abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x20), // zero the DWORD mode slot
            abi::add_immediate(abi::mfb_arg(1), abi::stack_pointer(), 0x20), // &mode
        ]);
        call_external(from, "GetConsoleMode", KERNEL32, instructions, relocations);
        instructions.extend([
            abi::compare_immediate(abi::c_return(0), "0"),
            abi::branch_eq(&skip), // not a console → leave stdout untouched
            abi::load_u32(abi::mfb_arg(1), abi::stack_pointer(), 0x20), // current mode
            abi::move_immediate(abi::mfb_arg(2), "Integer", "4"), // ENABLE_VIRTUAL_TERMINAL_PROCESSING
            abi::or_registers(abi::mfb_arg(1), abi::mfb_arg(1), abi::mfb_arg(2)),
            abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), 0x28), // handle
        ]);
        call_external(from, "SetConsoleMode", KERNEL32, instructions, relocations);
        instructions.extend([abi::label(&skip), abi::add_stack(0x30)]);
        Ok(())
    }

    fn emit_console_utf8(
        &self,
        from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // bug-392: a fresh Windows console decodes the bytes WriteFile hands it
        // through the machine's OEM code page (437/850), so the runtime's verbatim
        // UTF-8 output mojibakes (`—` E2 80 94 → `ΓÇö`; box-drawing → `ΓöÇ`, which
        // also desyncs the term:: grid diff-renderer's one-column cursor advance).
        // Set the console *output* code page to UTF-8 (65001) once at entry so each
        // multi-byte sequence decodes as its intended glyph; set the *input* code
        // page too so typed non-ASCII reaches the console-mode ReadFile path as
        // UTF-8 (symmetric — the io:: input path reads bytes, not wide chars). Both
        // are best-effort: when stdout/stdin is redirected the target is a file or
        // pipe, not a console, and SetConsole*CP is a harmless no-op there, so
        // file/pipe output stays byte-identical raw UTF-8. Return values are
        // ignored — a legacy console that rejects 65001 merely keeps mojibaking, no
        // worse than before. Unlike emit_enable_vt_output (VT escape interpretation,
        // orthogonal), this governs how the raw text bytes themselves are decoded.
        // A self-contained, balanced shadow-space frame like the neighbouring entry
        // calls; touches only caller-saved ARG registers.
        instructions.extend([
            abi::subtract_stack(0x20),
            abi::move_immediate(abi::mfb_arg(0), "Integer", CP_UTF8),
        ]);
        call_external(
            from,
            "SetConsoleOutputCP",
            KERNEL32,
            instructions,
            relocations,
        );
        instructions.push(abi::move_immediate(abi::mfb_arg(0), "Integer", CP_UTF8));
        call_external(from, "SetConsoleCP", KERNEL32, instructions, relocations);
        instructions.push(abi::add_stack(0x20));
        Ok(())
    }

    fn emit_env_get(
        &self,
        from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // GetEnvironmentVariableW over a UTF-8 name in ARG[0]; leaves a UTF-8
        // NUL-terminated value C-string pointer in **`c_return(0)`** (0 = unset),
        // matching the POSIX getenv contract the shared helper consumes — and
        // `getenv`'s `char*` is a C result, so `c_return(0)` is what "the return
        // register" means for that contract.
        //
        // It used to answer in `return_register()`, which is the *aligned MFB* bank and
        // on Win64 is `rcx`, not `rax`. Both consumers (`os::getEnvOr` in
        // `builtins/os/gen_env.rs` and `os::hasEnv`) read `c_return(0)` — correctly, and
        // with a plan-85 comment saying so — so on Windows they read whatever
        // `WideCharToMultiByte` left in `rax`, which is a **byte count**. For
        // `MFB_CANVAS_SYNC=1` that count is 2 ("1" plus the NUL), and the caller then
        // walked a C string from address 2. That is bug-479: every Windows canvas
        // program died in `canvas::present`, because `__canvas_ensureGraphics` is the
        // first thing on that path to read an environment variable that is actually set.
        // Frame
        // (after subtract_stack(0x60)): [0x00..0x20) shadow, [0x20]/[0x28] marshal
        // 5th/6th stack args, [0x40] saved UTF-8 name, [0x48] wide name buf, [0x50]
        // wide value buf, [0x58] UTF-8 value buf. Env values cap at 32767 wchars, so
        // 64 KiB wide buffers hold any value; the UTF-8 out buffer is 128 KiB (worst
        // case ~3 bytes/char). plan-66-B.
        const NAME_SLOT: usize = 0x40;
        const WNAME_SLOT: usize = 0x48;
        const WVAL_SLOT: usize = 0x50;
        const U8VAL_SLOT: usize = 0x58;
        let n = instructions.len();
        let not_found = format!("{from}_env_get_nf_{n}");
        let done = format!("{from}_env_get_done_{n}");
        instructions.extend([
            abi::subtract_stack(0x60),
            abi::store_u64(abi::c_arg(0), abi::stack_pointer(), NAME_SLOT), // save name (arena_alloc clobbers)
        ]);
        arena_alloc_to_slot(from, "65536", WNAME_SLOT, instructions, relocations);
        arena_alloc_to_slot(from, "65536", WVAL_SLOT, instructions, relocations);
        arena_alloc_to_slot(from, "131072", U8VAL_SLOT, instructions, relocations);
        // name (UTF-8) -> wide name.
        emit_utf8_slot_to_wide(
            from,
            NAME_SLOT,
            WNAME_SLOT,
            "32768",
            instructions,
            relocations,
        );
        // GetEnvironmentVariableW(wideName, wideVal, 32768) -> char count (0 = unset).
        instructions.extend([
            abi::load_u64(abi::c_arg(0), abi::stack_pointer(), WNAME_SLOT),
            abi::load_u64(abi::c_arg(1), abi::stack_pointer(), WVAL_SLOT),
            abi::move_immediate(abi::c_arg(2), "Integer", "32768"),
        ]);
        call_external(
            from,
            "GetEnvironmentVariableW",
            KERNEL32,
            instructions,
            relocations,
        );
        instructions.extend([
            abi::compare_immediate(abi::c_return(0), "0"),
            abi::branch_eq(&not_found),
            // WideCharToMultiByte(CP_UTF8, 0, wideVal, -1, u8Val, 131072, NULL, NULL).
            abi::move_immediate(abi::c_arg(0), "Integer", CP_UTF8),
            abi::move_immediate(abi::c_arg(1), "Integer", "0"),
            abi::load_u64(abi::c_arg(2), abi::stack_pointer(), U8VAL_SLOT), // lpMultiByteStr (5th)
            abi::store_u64(abi::c_arg(2), abi::stack_pointer(), 0x20),
            abi::move_immediate(abi::c_arg(2), "Integer", "131072"), // cbMultiByte (6th)
            abi::store_u64(abi::c_arg(2), abi::stack_pointer(), 0x28),
            abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x30), // lpDefaultChar (7th) = NULL
            abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x38), // lpUsedDefaultChar (8th) = NULL
            abi::load_u64(abi::c_arg(2), abi::stack_pointer(), WVAL_SLOT), // lpWideCharStr (3rd)
            abi::move_immediate(abi::c_arg(3), "Integer", "0"),
            abi::subtract_immediate(abi::c_arg(3), abi::c_arg(3), 1), // cchWideChar = -1
        ]);
        call_external(
            from,
            "WideCharToMultiByte",
            KERNEL32,
            instructions,
            relocations,
        );
        instructions.extend([
            abi::load_u64(abi::c_return(0), abi::stack_pointer(), U8VAL_SLOT), // UTF-8 value ptr
            abi::branch(&done),
            abi::label(&not_found),
            abi::move_immediate(abi::c_return(0), "Integer", "0"),
            abi::label(&done),
            abi::add_stack(0x60),
        ]);
        Ok(())
    }

    fn emit_env_set(
        &self,
        from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // SetEnvironmentVariableW(wideName, wideValue|NULL). ARG[0] = UTF-8 name,
        // ARG[1] = UTF-8 value (0 to delete). Frame (after subtract_stack(0x50)):
        // shadow [0x00..0x20), [0x20]/[0x28] marshal stack args, [0x30] wide name,
        // [0x38] wide value (or NULL), [0x40] saved name, [0x48] saved value.
        // plan-66-B.
        const WNAME_SLOT: usize = 0x30;
        const WVAL_SLOT: usize = 0x38;
        const NAME_SLOT: usize = 0x40;
        const VAL_SLOT: usize = 0x48;
        let n = instructions.len();
        let set_null = format!("{from}_env_set_null_{n}");
        let do_set = format!("{from}_env_set_do_{n}");
        instructions.extend([
            abi::subtract_stack(0x50),
            abi::store_u64(abi::c_arg(0), abi::stack_pointer(), NAME_SLOT),
            abi::store_u64(abi::c_arg(1), abi::stack_pointer(), VAL_SLOT),
        ]);
        arena_alloc_to_slot(from, "65536", WNAME_SLOT, instructions, relocations);
        emit_utf8_slot_to_wide(
            from,
            NAME_SLOT,
            WNAME_SLOT,
            "32768",
            instructions,
            relocations,
        );
        // value == 0 → delete (wideValue = NULL); else marshal the value.
        instructions.extend([
            abi::load_u64(abi::c_arg(0), abi::stack_pointer(), VAL_SLOT),
            abi::compare_immediate(abi::c_arg(0), "0"),
            abi::branch_eq(&set_null),
        ]);
        arena_alloc_to_slot(from, "131072", WVAL_SLOT, instructions, relocations);
        emit_utf8_slot_to_wide(
            from,
            VAL_SLOT,
            WVAL_SLOT,
            "65536",
            instructions,
            relocations,
        );
        instructions.extend([
            abi::branch(&do_set),
            abi::label(&set_null),
            abi::store_u64(abi::ZERO, abi::stack_pointer(), WVAL_SLOT), // lpValue = NULL → delete
            abi::label(&do_set),
            abi::load_u64(abi::c_arg(0), abi::stack_pointer(), WNAME_SLOT),
            abi::load_u64(abi::c_arg(1), abi::stack_pointer(), WVAL_SLOT),
        ]);
        call_external(
            from,
            "SetEnvironmentVariableW",
            KERNEL32,
            instructions,
            relocations,
        );
        // SetEnvironmentVariableW returns BOOL (nonzero = success); invert to the
        // POSIX setenv/unsetenv convention (0 = success, nonzero = failure) the
        // shared helper's branch expects.
        let ok = format!("{from}_env_set_ok_{n}");
        let out = format!("{from}_env_set_out_{n}");
        instructions.extend([
            abi::compare_immediate(abi::c_return(0), "0"), // Win32 BOOL (C result)
            abi::branch_ne(&ok),                           // BOOL != 0 → success
            abi::move_immediate(abi::return_register(), "Integer", "1"), // failure
            abi::branch(&out),
            abi::label(&ok),
            abi::move_immediate(abi::return_register(), "Integer", "0"), // success
            abi::label(&out),
            abi::add_stack(0x50),
        ]);
        Ok(())
    }

    fn emit_os_wide_string(
        &self,
        which: &str,
        from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // Run the `*W` OS query for `which` into a wide buffer, marshal to a UTF-8
        // C-string, and leave the pointer in the return register (0 on failure).
        // Frame (after subtract_stack(0x60)): shadow [0x00..0x20), marshal stack args
        // [0x20..0x40), [0x40] size DWORD (in/out for the Ex/User queries), [0x48]
        // wide buffer (2048 wchars), [0x50] UTF-8 buffer (8 KiB). plan-66-B.
        const SIZE_SLOT: usize = 0x40;
        const WIDE_SLOT: usize = 0x48;
        const U8_SLOT: usize = 0x50;
        let n = instructions.len();
        let fail = format!("{from}_oswq_fail_{n}");
        let done = format!("{from}_oswq_done_{n}");
        instructions.push(abi::subtract_stack(0x60));
        arena_alloc_to_slot(from, "4096", WIDE_SLOT, instructions, relocations); // 2048 wchars
        arena_alloc_to_slot(from, "8192", U8_SLOT, instructions, relocations);
        match which {
            // GetComputerNameExW(ComputerNameDnsHostname=1, lpBuffer, &nSize) → BOOL.
            "hostName" => {
                instructions.extend([
                    abi::move_immediate(abi::c_arg(0), "Integer", "2048"),
                    abi::store_u32(abi::c_arg(0), abi::stack_pointer(), SIZE_SLOT),
                    abi::move_immediate(abi::c_arg(0), "Integer", "1"),
                    abi::load_u64(abi::c_arg(1), abi::stack_pointer(), WIDE_SLOT),
                    abi::add_immediate(abi::c_arg(2), abi::stack_pointer(), SIZE_SLOT),
                ]);
                call_external(
                    from,
                    "GetComputerNameExW",
                    KERNEL32,
                    instructions,
                    relocations,
                );
                instructions.push(abi::compare_immediate(abi::c_return(0), "0"));
                instructions.push(abi::branch_eq(&fail)); // BOOL 0 = failure
            }
            // GetUserNameW(lpBuffer, &pcbBuffer) → BOOL (advapi32).
            "userName" => {
                instructions.extend([
                    abi::move_immediate(abi::c_arg(0), "Integer", "2048"),
                    abi::store_u32(abi::c_arg(0), abi::stack_pointer(), SIZE_SLOT),
                    abi::load_u64(abi::c_arg(0), abi::stack_pointer(), WIDE_SLOT),
                    abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), SIZE_SLOT),
                ]);
                call_external(from, "GetUserNameW", ADVAPI32, instructions, relocations);
                instructions.push(abi::compare_immediate(abi::c_return(0), "0"));
                instructions.push(abi::branch_eq(&fail));
            }
            // GetModuleFileNameW(NULL, lpFilename, nSize) → char count (0 = failure).
            "executablePath" => {
                instructions.extend([
                    abi::move_immediate(abi::c_arg(0), "Integer", "0"),
                    abi::load_u64(abi::c_arg(1), abi::stack_pointer(), WIDE_SLOT),
                    abi::move_immediate(abi::c_arg(2), "Integer", "2048"),
                ]);
                call_external(
                    from,
                    "GetModuleFileNameW",
                    KERNEL32,
                    instructions,
                    relocations,
                );
                instructions.push(abi::compare_immediate(abi::c_return(0), "0"));
                instructions.push(abi::branch_eq(&fail));
            }
            other => return Err(format!("unknown os wide-string query '{other}'")),
        }
        emit_wide_slot_to_utf8(from, WIDE_SLOT, U8_SLOT, "8192", instructions, relocations);
        instructions.extend([
            abi::load_u64(abi::return_register(), abi::stack_pointer(), U8_SLOT),
            abi::branch(&done),
            abi::label(&fail),
            abi::move_immediate(abi::return_register(), "Integer", "0"),
            abi::label(&done),
            abi::add_stack(0x60),
        ]);
        Ok(())
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
            abi::mfb_arg(0),
            abi::stack_pointer(),
            MARSHAL_WBUF_SLOT,
        ));
        if is_mkdir {
            // CreateDirectoryW(path, lpSecurityAttributes = NULL).
            instructions.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "0"));
        }
        call_external(from, symbol, KERNEL32, instructions, relocations);
        instructions.extend([
            abi::compare_immediate(abi::c_return(0), "0"), // Win32 BOOL (C result)
            abi::branch_ne(&ok),                           // BOOL != 0 → success
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
        dst: Operand,
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
            abi::move_register(dst, abi::c_return(0)), // GetLastError DWORD (C result)
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
            abi::store_u64(abi::mfb_arg(1), abi::stack_pointer(), PACKED_SLOT), // save packed flags
        ]);
        emit_marshal_path(from, instructions, relocations);
        instructions.extend([
            abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), MARSHAL_WBUF_SLOT), // lpFileName
            abi::load_u64(abi::mfb_arg(1), abi::stack_pointer(), PACKED_SLOT),
            // Stage the three stack args using ARG[2] as a caller-saved scratch,
            // BEFORE it is set to its register value (dwShareMode=7). The SCRATCH
            // pool must not be used — its Win64 realizations are callee-saved.
            // dwCreationDisposition (5th, stack) = packed >> 32.
            abi::shift_right_immediate(abi::mfb_arg(2), abi::mfb_arg(1), 32),
            abi::store_u64(abi::mfb_arg(2), abi::stack_pointer(), 0x20),
            abi::move_immediate(abi::mfb_arg(2), "Integer", "128"), // FILE_ATTRIBUTE_NORMAL
            abi::store_u64(abi::mfb_arg(2), abi::stack_pointer(), 0x28), // 6th (stack)
            abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x30),  // 7th hTemplateFile = NULL
            // dwDesiredAccess: CreateFileW reads it as the low 32 bits of rdx, so
            // the packed value in ARG[1] goes straight in — the disposition in the
            // high half is ignored by the DWORD parameter.
            abi::move_immediate(abi::mfb_arg(2), "Integer", "7"), // FILE_SHARE_READ|WRITE|DELETE
            abi::move_immediate(abi::mfb_arg(3), "Integer", "0"), // lpSecurityAttributes = NULL
        ]);
        call_external(from, "CreateFileW", KERNEL32, instructions, relocations);
        instructions.extend([
            // plan-85: `CreateFileW` hands the HANDLE back as a **C** result (`rax` =
            // `%retC`), and this seam's contract above promises it in the *aligned MFB*
            // return register (`rcx` on Win64). Without this move the caller read
            // whatever `rcx` happened to hold — the `lpFileName` pointer, which is
            // positive, so the `< 0` open-failed check passed and the bogus value was
            // handed to `WriteFile` as its handle.
            //
            // The symptom was `fs::writeText` raising `ErrWriteFailed` while leaving a
            // **0-byte file behind**: the open genuinely worked, so the file was created
            // and truncated, and only the write failed. Every `fs` write on Windows was
            // affected. `emit_heap_alloc`, `emit_lib_open` and `emit_lib_get_sym` do
            // this move; this one was missed when plan-85 split `%retC` from the
            // aligned bank.
            abi::move_register(abi::return_register(), abi::c_return(0)),
            abi::add_stack(FRAME),
        ]);
        Ok(())
    }

    fn emit_read_file(
        &self,
        from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // read(fd, buf, len): fd/HANDLE in ARG[0], buffer in ARG[1], length in
        // ARG[2]; return the byte count (0 = EOF, negative = error). fd 0 (stdin —
        // the io:: input path passes the POSIX fd, not a handle) resolves to
        // GetStdHandle(STD_INPUT_HANDLE = -10) the same way emit_write resolves
        // stdout/stderr; a CreateFileW handle (always ≥ 4) passes through. ReadFile(
        // hFile, lpBuffer, nToRead, &nRead, NULL) — the 5th arg (lpOverlapped) is a
        // stack arg that MUST be NULL. On BOOL failure return -1; otherwise nRead
        // (0 at EOF, exactly the read() contract). plan-66-C.
        //
        // The `read()` contract also covers the case Win32 spells as a *failure*:
        // draining a pipe whose write end has closed. POSIX `read()` returns 0
        // there, but `ReadFile` returns FALSE/`ERROR_BROKEN_PIPE` — so a piped
        // stdin (`prog < file`, `echo x | prog`, any `Command::stdin(Stdio::piped)`
        // parent) reported a hard input error instead of EOF, and every reader
        // built on this seam raised `ErrInputFailed` where `ErrEndOfFile` is
        // specified. Map the two end-of-input error codes back onto a 0-byte
        // return so the seam matches its documented contract on every stdin shape.
        let n = instructions.len();
        let ok = format!("{from}_read_ok_{n}");
        let eof = format!("{from}_read_eof_{n}");
        let done = format!("{from}_read_done_{n}");
        let file_handle = format!("{from}_read_fileh_{n}");
        let have_handle = format!("{from}_read_haveh_{n}");
        instructions.extend([
            abi::subtract_stack(0x50),
            abi::store_u64(abi::mfb_arg(1), abi::stack_pointer(), 0x30), // save buf
            abi::store_u64(abi::mfb_arg(2), abi::stack_pointer(), 0x38), // save len
            abi::compare_immediate(abi::mfb_arg(0), "2"),
            abi::branch_gt(&file_handle),
            // std fd → GetStdHandle(-(fd+10)) without a negative immediate.
            abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(0), 10),
            abi::move_immediate(abi::mfb_arg(0), "Integer", "0"),
            abi::subtract_registers(abi::mfb_arg(0), abi::mfb_arg(0), abi::mfb_arg(1)),
        ]);
        call_external(from, "GetStdHandle", KERNEL32, instructions, relocations);
        instructions.extend([
            // plan-85: GetStdHandle returns the console HANDLE as a C result (`rax` =
            // `%retC`), not the aligned MFB result register — read it from `c_return`.
            abi::store_u64(abi::c_return(0), abi::stack_pointer(), 0x40), // hFile
            abi::branch(&have_handle),
            abi::label(&file_handle),
            abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), 0x40), // hFile = handle directly
            abi::label(&have_handle),
            abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x20), // lpOverlapped = NULL (5th)
            // Zero the nRead slot first — it is a DWORD (32-bit) out-param, so
            // ReadFile writes only the low 32 bits; the load_u64 below would
            // otherwise return garbage in the high 32 bits (see emit_write).
            abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x28),
            abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), 0x40), // hFile
            abi::load_u64(abi::mfb_arg(1), abi::stack_pointer(), 0x30), // lpBuffer
            abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), 0x38), // nToRead
            abi::add_immediate(abi::mfb_arg(3), abi::stack_pointer(), 0x28), // &nRead (4th)
        ]);
        call_external(from, "ReadFile", KERNEL32, instructions, relocations);
        instructions.extend([
            abi::compare_immediate(abi::c_return(0), "0"),
            abi::branch_ne(&ok),
        ]);
        // BOOL failure. GetLastError is called immediately, before any other Win32
        // call can overwrite the thread's last-error slot (the frame's shadow space
        // is already reserved, so no stack adjustment is needed here).
        call_external(from, "GetLastError", KERNEL32, instructions, relocations);
        instructions.extend([
            abi::compare_immediate(abi::c_return(0), ERROR_BROKEN_PIPE),
            abi::branch_eq(&eof),
            abi::compare_immediate(abi::c_return(0), ERROR_HANDLE_EOF),
            abi::branch_eq(&eof),
            abi::move_immediate(abi::return_register(), "Integer", "0"),
            abi::subtract_immediate(abi::return_register(), abi::return_register(), 1), // -1
            abi::branch(&done),
            abi::label(&eof),
            abi::move_immediate(abi::return_register(), "Integer", "0"),
            abi::branch(&done),
            abi::label(&ok),
            abi::load_u64(abi::return_register(), abi::stack_pointer(), 0x28), // nRead
            abi::label(&done),
            abi::add_stack(0x50),
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
            abi::compare_immediate(abi::c_return(0), "0"),
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
        call_external(
            from,
            "FlushFileBuffers",
            KERNEL32,
            instructions,
            relocations,
        );
        instructions.extend([
            abi::compare_immediate(abi::c_return(0), "0"),
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
            abi::move_register(abi::mfb_arg(3), abi::mfb_arg(2)), // dwMoveMethod = whence
            abi::add_immediate(abi::mfb_arg(2), abi::stack_pointer(), 0x28), // &liNewFilePointer
        ]);
        call_external(
            from,
            "SetFilePointerEx",
            KERNEL32,
            instructions,
            relocations,
        );
        instructions.extend([
            abi::compare_immediate(abi::c_return(0), "0"),
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
            abi::store_u64(abi::mfb_arg(1), abi::stack_pointer(), NEW_PATH_SLOT), // save new path
        ]);
        emit_marshal_path(from, instructions, relocations); // old → [MARSHAL_WBUF_SLOT]
        instructions.extend([
            abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), MARSHAL_WBUF_SLOT),
            abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), WBUF_OLD_SLOT), // save wide old
            abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), NEW_PATH_SLOT),  // new path
        ]);
        emit_marshal_path(from, instructions, relocations); // new → [MARSHAL_WBUF_SLOT]
        instructions.extend([
            abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), WBUF_OLD_SLOT), // lpExistingFileName
            abi::load_u64(abi::mfb_arg(1), abi::stack_pointer(), MARSHAL_WBUF_SLOT), // lpNewFileName
            abi::move_immediate(abi::mfb_arg(2), "Integer", "1"), // MOVEFILE_REPLACE_EXISTING
        ]);
        call_external(from, "MoveFileExW", KERNEL32, instructions, relocations);
        instructions.extend([
            abi::compare_immediate(abi::c_return(0), "0"), // Win32 BOOL (C result)
            abi::branch_ne(&ok),                           // BOOL != 0 → success
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
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // POSIX mkstemps over Win32 (plan-66-E). On entry: return_register() = a
        // mutable UTF-8 template C-string ending in "XXXXXX<suffix>", ARG[1] = suffix
        // byte length. Replace the 6 X markers with random lowercase letters, then
        // CreateFileW(CREATE_NEW) — retrying on a name collision — and return the
        // handle (a small positive value that the shared caller's sign-extend +
        // `>= 0` check treats as a valid fd) or -1. The template is modified in place
        // so the caller can rename the temp path afterward. Frame (subtract_stack(
        // 0x60)): shadow [0x00..0x20), CreateFileW stack args [0x20..0x38), [0x38]
        // template ptr, [0x40] X-markers ptr, [0x48] wide buffer, [0x50] retry count,
        // [0x58] 8-byte random scratch.
        const TMPL: usize = 0x38;
        const XSTART: usize = 0x40;
        const WIDE: usize = 0x48;
        const RETRY: usize = 0x50;
        const RAND: usize = 0x58;
        const X_MARKER_COUNT: usize = 6;
        let n = instructions.len();
        let l = |s: &str| format!("{from}_mkstemps_{s}_{n}");
        let (strlen_loop, strlen_done) = (l("sl"), l("sd"));
        let (retry_loop, fill_loop, fill_done, success, giveup, done) =
            (l("rl"), l("fl"), l("fd"), l("ok"), l("gu"), l("dn"));
        instructions.push(abi::subtract_stack(0x60));
        instructions.extend([
            abi::store_u64(abi::return_register(), abi::stack_pointer(), TMPL),
            // X-markers start = strlen(template) - suffix_len - 6.
            abi::move_register(abi::mfb_arg(0), abi::return_register()),
            abi::label(&strlen_loop),
            abi::load_u8(abi::mfb_arg(2), abi::mfb_arg(0), 0),
            abi::compare_immediate(abi::mfb_arg(2), "0"),
            abi::branch_eq(&strlen_done),
            abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1),
            abi::branch(&strlen_loop),
            abi::label(&strlen_done),
            abi::subtract_registers(abi::mfb_arg(0), abi::mfb_arg(0), abi::mfb_arg(1)), // - suffix_len
            abi::subtract_immediate(abi::mfb_arg(0), abi::mfb_arg(0), X_MARKER_COUNT),  // - 6
            abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), XSTART),
            abi::store_u64(abi::ZERO, abi::stack_pointer(), RETRY),
        ]);
        arena_alloc_to_slot(from, "65536", WIDE, instructions, relocations);
        instructions.push(abi::label(&retry_loop));
        // 6 random bytes into the RAND scratch, then map each to 'a'+(byte % 26).
        instructions.extend([
            abi::add_immediate(abi::mfb_arg(0), abi::stack_pointer(), RAND),
            abi::move_immediate(abi::mfb_arg(1), "Integer", "6"),
        ]);
        self.emit_random_bytes(from, platform_imports, instructions, relocations)?;
        instructions.extend([
            abi::move_immediate(abi::mfb_arg(0), "Integer", "0"), // i
            abi::label(&fill_loop),
            abi::compare_immediate(abi::mfb_arg(0), "6"),
            abi::branch_ge(&fill_done),
            abi::add_immediate(abi::mfb_arg(1), abi::stack_pointer(), RAND),
            abi::add_registers(abi::mfb_arg(1), abi::mfb_arg(1), abi::mfb_arg(0)),
            abi::load_u8(abi::mfb_arg(2), abi::mfb_arg(1), 0), // random byte
            abi::move_immediate(abi::mfb_arg(3), "Integer", "26"),
            abi::unsigned_divide_registers(abi::mfb_arg(1), abi::mfb_arg(2), abi::mfb_arg(3)), // q
            abi::multiply_subtract_registers(
                abi::mfb_arg(2),
                abi::mfb_arg(1),
                abi::mfb_arg(3),
                abi::mfb_arg(2),
            ), // r = b - q*26
            abi::add_immediate(abi::mfb_arg(2), abi::mfb_arg(2), 97), // 'a' + r
            abi::load_u64(abi::mfb_arg(1), abi::stack_pointer(), XSTART),
            abi::add_registers(abi::mfb_arg(1), abi::mfb_arg(1), abi::mfb_arg(0)),
            abi::store_u8(abi::mfb_arg(2), abi::mfb_arg(1), 0),
            abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1),
            abi::branch(&fill_loop),
            abi::label(&fill_done),
        ]);
        // Marshal template UTF-8 → UTF-16, then
        // CreateFileW(lpFileName, GENERIC_READ|GENERIC_WRITE, 0, NULL, CREATE_NEW,
        //   FILE_ATTRIBUTE_NORMAL, NULL). Stage the three stack args (5th/6th/7th)
        // with ARG[0] as a temp BEFORE loading the four register args.
        emit_utf8_slot_to_wide(from, TMPL, WIDE, "32768", instructions, relocations);
        instructions.extend([
            abi::move_immediate(abi::mfb_arg(0), "Integer", "1"), // CREATE_NEW
            abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), 0x20),
            abi::move_immediate(abi::mfb_arg(0), "Integer", "128"), // FILE_ATTRIBUTE_NORMAL
            abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), 0x28),
            abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x30), // hTemplateFile = NULL
            abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), WIDE), // lpFileName
            abi::move_immediate(abi::mfb_arg(1), "Integer", "3221225472"), // GENERIC_READ|GENERIC_WRITE
            abi::move_immediate(abi::mfb_arg(2), "Integer", "0"),          // dwShareMode
            abi::move_immediate(abi::mfb_arg(3), "Integer", "0"), // lpSecurityAttributes NULL
        ]);
        call_external(from, "CreateFileW", KERNEL32, instructions, relocations);
        instructions.extend([
            // INVALID_HANDLE_VALUE = -1; a real handle is a small positive value.
            abi::compare_immediate(abi::c_return(0), "0"),
            abi::branch_gt(&success),
            // collision or error: retry up to 100 times, then give up.
            abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), RETRY),
            abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1),
            abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), RETRY),
            abi::compare_immediate(abi::mfb_arg(0), "100"),
            abi::branch_lt(&retry_loop),
            abi::label(&giveup),
            abi::move_immediate(abi::return_register(), "Integer", "0"),
            abi::subtract_immediate(abi::return_register(), abi::return_register(), 1), // -1
            abi::branch(&done),
            abi::label(&success),
            // Stage the handle from the C result into the MFB result, exactly as
            // `emit_linux_c_call` does for every other Windows OS seam. `call_external`
            // above does NOT do it, and the two registers are not the same on Win64:
            // `c_return(0)` is `rax`, while `return_register()` is `mfb_return(0)` =
            // the call-argument bank's first slot, `rcx` (plan-85-A aligned the MFB
            // result onto the argument bank; `CALL_ARGS_WIN64[0] == "rcx"`).
            //
            // Without this the giveup path wrote -1 into `rcx` and the SUCCESS path
            // wrote nothing there at all, so `gen_atomic_write`'s shared caller — which
            // reads the fd through `return_register()` — sign-extended whatever
            // `CreateFileW` happened to leave in `rcx` and used it as a descriptor.
            // Measured on box 2230: `fs::createTempFile`, and therefore
            // `fs::writeTextAtomic`/`fs::writeBytesAtomic`, raised
            // `7-702-0002 ErrWriteFailed` on every call. Same class as plan-110-D,
            // which found `socket()`/`connect()`/`getsockname()` all checked against
            // `rcx` for the same reason.
            abi::move_register(abi::return_register(), abi::c_return(0)),
            abi::label(&done),
            abi::add_stack(0x60),
        ]);
        Ok(())
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
        // dwFlags=BCRYPT_USE_SYSTEM_PREFERRED_RNG (0x02). The NTSTATUS is staged into
        // the MFB result register below, because the shared callers check it.
        const BCRYPT: &str = "bcrypt.dll";
        const BCRYPT_USE_SYSTEM_PREFERRED_RNG: &str = "2";
        // **The frame is the whole point of this emitter.** Win64 makes the CALLER
        // reserve 32 bytes of shadow space, which the callee is free to spill its four
        // register arguments into — and those bytes sit *above* the caller's `rsp`, in
        // the caller's own frame. Without this `sub`, `BCryptGenRandom` writes over 32
        // bytes of whatever the enclosing function had there.
        //
        // Every other external-call emitter in this file reserves one; this one did not,
        // and the damage was invisible on the console path and fatal on app mode's
        // worker thread, where the corrupted frame belonged to code that then dereferenced
        // it. See bug-478 — an empty `SUB main() END SUB` faulted with `0xC0000005` inside
        // ntdll's activation-context machinery, several frames away from anything this
        // repository wrote.
        // 0x20, not 0x28. This emitter is *inline* in a larger body whose frame the
        // `abi_function` finalizer already established and already aligned to 16, so
        // the reservation has to be a multiple of 16 to keep it that way. 0x28 is the
        // right number for a function's own prologue — where entry `rsp` is 8 mod 16
        // because the call pushed a return address — and it is exactly wrong here:
        // measured, an empty `SUB main() END SUB` console build faulted with it and
        // ran clean with this.
        instructions.push(abi::subtract_stack(SHADOW_FRAME));
        instructions.extend([
            // Shuffle (buf x0→pbBuffer, len x1→cbBuffer) in an order that needs no
            // scratch: copy len up to ARG[2] before ARG[1] is overwritten, then buf
            // into ARG[1], then NULL into ARG[0]. The SCRATCH pool must not be used —
            // callee-saved on Win64.
            abi::move_register(abi::c_arg(2), abi::c_arg(1)), // cbBuffer = len
            abi::move_register(abi::c_arg(1), abi::c_arg(0)), // pbBuffer = buf
            abi::move_immediate(abi::c_arg(0), "Integer", "0"), // hAlgorithm = NULL
            abi::move_immediate(abi::c_arg(3), "Integer", BCRYPT_USE_SYSTEM_PREFERRED_RNG),
        ]);
        call_external(from, "BCryptGenRandom", BCRYPT, instructions, relocations);
        // Stage the NTSTATUS into the MFB result. `call_external` does not, and on
        // Win64 `c_return(0)` is `rax` while `return_register()` is `rcx` — so
        // without this the shared callers read whatever `BCryptGenRandom` left in
        // `rcx`. They DO read it: `gen_temp_file`'s `fs::createTempFile` sign-extends
        // it and takes the error path on a negative value, so a garbage `rcx` decided
        // at random whether the call "failed". Measured on box 2230:
        // `fs::createTempFile` raised `7-702-0002 ErrWriteFailed` every time.
        //
        // The comment that used to sit here — "the NTSTATUS return is ignored" — was
        // true of this emitter and false of its callers. An NTSTATUS is negative on
        // failure and `>= 0` on success, which is exactly the convention the shared
        // check applies, so staging it needs no translation.
        instructions.push(abi::move_register(abi::return_register(), abi::c_return(0)));
        instructions.push(abi::add_stack(SHADOW_FRAME));
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
            abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), MARSHAL_WBUF_SLOT),
            abi::move_immediate(abi::mfb_arg(1), "Integer", "0"),
            abi::label(&scan),
            abi::add_registers(abi::mfb_arg(2), abi::mfb_arg(0), abi::mfb_arg(1)),
            abi::load_u16(abi::mfb_arg(3), abi::mfb_arg(2), 0),
            abi::compare_immediate(abi::mfb_arg(3), "0"),
            abi::branch_eq(&scan_done),
            abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(1), 2),
            abi::branch(&scan),
            abi::label(&scan_done),
            // ARG[2] = &NUL wchar. Write the wildcard suffix.
            abi::move_immediate(abi::mfb_arg(3), "Integer", "92"), // L'\'
            abi::store_u16(abi::mfb_arg(3), abi::mfb_arg(2), 0),
            abi::move_immediate(abi::mfb_arg(3), "Integer", "42"), // L'*'
            abi::store_u16(abi::mfb_arg(3), abi::mfb_arg(2), 2),
            abi::move_immediate(abi::mfb_arg(3), "Integer", "0"), // L'\0'
            abi::store_u16(abi::mfb_arg(3), abi::mfb_arg(2), 4),
            // Allocate the DIR struct.
            abi::move_immediate(abi::return_register(), "Integer", DIR_SIZE),
            abi::move_immediate(abi::mfb_arg(1), "Integer", "8"),
            abi::branch_link(crate::codegen::error::constants::ARENA_ALLOC_SYMBOL),
        ]);
        relocations.push(CodeRelocation {
            from: from.to_string(),
            to: crate::codegen::error::constants::ARENA_ALLOC_SYMBOL.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
        instructions.extend([
            abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), DIR_SLOT),
            // FindFirstFileW(lpFileName = wide pattern, lpFindFileData = &DIR.findData)
            abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), MARSHAL_WBUF_SLOT),
            abi::load_u64(abi::mfb_arg(1), abi::stack_pointer(), DIR_SLOT),
            abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(1), DIR_FINDDATA_OFF),
        ]);
        call_external(from, "FindFirstFileW", KERNEL32, instructions, relocations);
        instructions.extend([
            // INVALID_HANDLE_VALUE is (HANDLE)-1; a valid search handle is a small
            // positive value, so `<= 0` means failure.
            abi::compare_immediate(abi::c_return(0), "0"),
            abi::branch_le(&fail),
            abi::load_u64(abi::mfb_arg(1), abi::stack_pointer(), DIR_SLOT),
            abi::store_u64(abi::c_return(0), abi::mfb_arg(1), DIR_HANDLE_OFF), // FindFirstFileW handle (C result)
            abi::move_immediate(abi::mfb_arg(2), "Integer", "1"),
            abi::store_u64(abi::mfb_arg(2), abi::mfb_arg(1), DIR_FIRST_OFF), // first pending
            abi::move_register(abi::return_register(), abi::mfb_arg(1)),     // return DIR*
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
            abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), DIR_SLOT),
            abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), DIR_FIRST_OFF),
            abi::compare_immediate(abi::mfb_arg(1), "0"),
            abi::branch_ne(&have), // first entry already in findData
            // FindNextFileW(handle, &findData)
            abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), DIR_SLOT),
            abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(0), DIR_FINDDATA_OFF),
            abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), DIR_HANDLE_OFF),
        ]);
        call_external(from, "FindNextFileW", KERNEL32, instructions, relocations);
        instructions.extend([
            abi::compare_immediate(abi::c_return(0), "0"),
            abi::branch_eq(&end), // BOOL 0 → no more entries
            abi::branch(&convert),
            abi::label(&have),
            abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), DIR_SLOT),
            abi::store_u64(abi::ZERO, abi::mfb_arg(0), DIR_FIRST_OFF), // consume the first entry
            abi::label(&convert),
            // WideCharToMultiByte(CP_UTF8, 0, DIR+cFileName, -1, DIR+name, cap, NULL, NULL)
            abi::move_immediate(abi::mfb_arg(0), "Integer", CP_UTF8),
            abi::move_immediate(abi::mfb_arg(1), "Integer", "0"),
            abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), DIR_SLOT),
            abi::add_immediate(abi::mfb_arg(2), abi::mfb_arg(2), DIR_NAME_OFF),
            abi::store_u64(abi::mfb_arg(2), abi::stack_pointer(), 0x20), // lpMultiByteStr (5th)
            abi::move_immediate(abi::mfb_arg(2), "Integer", DIR_NAME_CAP),
            abi::store_u64(abi::mfb_arg(2), abi::stack_pointer(), 0x28), // cbMultiByte (6th)
            abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x30),       // 7th NULL
            abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x38),       // 8th NULL
            abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), DIR_SLOT),
            abi::add_immediate(abi::mfb_arg(2), abi::mfb_arg(2), DIR_CFILENAME_OFF), // lpWideCharStr
            abi::move_immediate(abi::mfb_arg(3), "Integer", "0"),
            abi::subtract_immediate(abi::mfb_arg(3), abi::mfb_arg(3), 1), // cchWideChar = -1
        ]);
        call_external(
            from,
            "WideCharToMultiByte",
            KERNEL32,
            instructions,
            relocations,
        );
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
            abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), DIR_HANDLE_OFF),
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
            abi::store_u64(abi::mfb_arg(1), abi::stack_pointer(), RMARSHAL_DST_SLOT), // resolved dst
            abi::move_immediate(abi::mfb_arg(2), "Integer", "4097"),
            abi::store_u64(abi::mfb_arg(2), abi::stack_pointer(), RMARSHAL_CAP_SLOT), // capacity
        ]);
        emit_marshal_path(from, instructions, relocations); // input → wide at [MARSHAL_WBUF_SLOT]
        instructions.extend([
            // arena UTF-16 output scratch.
            abi::move_immediate(abi::return_register(), "Integer", "65536"),
            abi::move_immediate(abi::mfb_arg(1), "Integer", "2"),
            abi::branch_link(crate::codegen::error::constants::ARENA_ALLOC_SYMBOL),
        ]);
        relocations.push(CodeRelocation {
            from: from.to_string(),
            to: crate::codegen::error::constants::ARENA_ALLOC_SYMBOL.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
        instructions.extend([
            abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), RMARSHAL_WBUF_SLOT),
            // GetFullPathNameW(lpFileName=wide_in, nBufferLength=32768,
            //                  lpBuffer=wide_out, lpFilePart=NULL)
            abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), MARSHAL_WBUF_SLOT),
            abi::move_immediate(abi::mfb_arg(1), "Integer", "32768"),
            abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), RMARSHAL_WBUF_SLOT),
            abi::move_immediate(abi::mfb_arg(3), "Integer", "0"),
        ]);
        call_external(
            from,
            "GetFullPathNameW",
            KERNEL32,
            instructions,
            relocations,
        );
        instructions.extend([
            abi::compare_immediate(abi::c_return(0), "0"),
            abi::branch_eq(&fail),
        ]);
        emit_wide_to_utf8(from, instructions, relocations); // wide_out → resolved dst
        instructions.extend([
            abi::load_u64(
                abi::return_register(),
                abi::stack_pointer(),
                RMARSHAL_DST_SLOT,
            ),
            abi::branch(&done),
            abi::label(&fail),
            abi::move_immediate(abi::return_register(), "Integer", "0"),
            abi::label(&done),
            abi::add_stack(RMARSHAL_FRAME),
        ]);
        Ok(())
    }

    fn emit_verify_nofollow(
        &self,
        from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // ARG[0] = opened HANDLE, ARG[1] = requested UTF-8 path C-string. Returns
        // 0 (no link traversed) / 1 (refuse) in the return register. See the trait
        // doc: compare GetFinalPathNameByHandleW(handle) vs GetFullPathNameW(req).
        const FRAME: usize = 0x70;
        const HANDLE_SLOT: usize = 0x40;
        const REQCSTR_SLOT: usize = 0x48;
        const REQWIN_SLOT: usize = 0x50; // UTF-16 input to GetFullPathNameW
        const REQOUT_SLOT: usize = 0x58; // lexical canonical (no \\?\ prefix)
        const FILE_SLOT: usize = 0x60; // handle final path (\\?\ prefixed)
        let n = instructions.len();
        let fail = format!("{from}_nf_fail_{n}");
        let equal = format!("{from}_nf_equal_{n}");
        let loop_lbl = format!("{from}_nf_loop_{n}");
        let done = format!("{from}_nf_done_{n}");
        instructions.extend([
            abi::subtract_stack(FRAME),
            abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), HANDLE_SLOT),
            abi::store_u64(abi::mfb_arg(1), abi::stack_pointer(), REQCSTR_SLOT),
        ]);
        arena_alloc_to_slot(from, "65536", REQWIN_SLOT, instructions, relocations);
        arena_alloc_to_slot(from, "65536", REQOUT_SLOT, instructions, relocations);
        arena_alloc_to_slot(from, "65536", FILE_SLOT, instructions, relocations);
        emit_utf8_slot_to_wide(
            from,
            REQCSTR_SLOT,
            REQWIN_SLOT,
            "32768",
            instructions,
            relocations,
        );
        // GetFullPathNameW(lpFileName=reqWideIn, nBufferLength=32768,
        //                  lpBuffer=reqOut, lpFilePart=NULL) — lexical only.
        instructions.extend([
            abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), REQWIN_SLOT),
            abi::move_immediate(abi::mfb_arg(1), "Integer", "32768"),
            abi::load_u64(abi::mfb_arg(2), abi::stack_pointer(), REQOUT_SLOT),
            abi::move_immediate(abi::mfb_arg(3), "Integer", "0"),
        ]);
        call_external(
            from,
            "GetFullPathNameW",
            KERNEL32,
            instructions,
            relocations,
        );
        instructions.extend([
            abi::compare_immediate(abi::c_return(0), "0"),
            abi::branch_eq(&fail),
        ]);
        emit_final_path_call(from, HANDLE_SLOT, FILE_SLOT, instructions, relocations);
        instructions.extend([
            abi::compare_immediate(abi::c_return(0), "0"),
            abi::branch_eq(&fail),
            // Case-insensitive WCHAR compare: reqOut (C:\...) vs fileFinal+8 bytes
            // (skip the 4-WCHAR \\?\ prefix). A mismatch means a reparse point
            // redirected the open (O_NOFOLLOW_ANY analog).
            abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), REQOUT_SLOT),
            abi::load_u64(abi::mfb_arg(1), abi::stack_pointer(), FILE_SLOT),
            abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(1), 8),
            abi::label(&loop_lbl),
            abi::load_u16(abi::mfb_arg(2), abi::mfb_arg(0), 0),
            abi::load_u16(abi::mfb_arg(3), abi::mfb_arg(1), 0),
        ]);
        emit_ascii_fold(abi::mfb_arg(2), n, instructions);
        emit_ascii_fold(abi::mfb_arg(3), n + 1, instructions);
        instructions.extend([
            abi::compare_registers(abi::mfb_arg(2), abi::mfb_arg(3)),
            abi::branch_ne(&fail),
            abi::compare_immediate(abi::mfb_arg(2), "0"),
            abi::branch_eq(&equal),
            abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 2),
            abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(1), 2),
            abi::branch(&loop_lbl),
            abi::label(&equal),
            abi::move_immediate(abi::return_register(), "Integer", "0"),
            abi::branch(&done),
            abi::label(&fail),
            abi::move_immediate(abi::return_register(), "Integer", "1"),
            abi::label(&done),
            abi::add_stack(FRAME),
        ]);
        Ok(())
    }

    fn emit_verify_within(
        &self,
        from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // ARG[0] = opened HANDLE (the join), ARG[1] = trusted root UTF-8 C-string.
        // Returns 0 (contained) / 1 (refuse). See the trait doc: fileFinal must
        // start with rootFinal + '\'. Both final paths carry the same \\?\ prefix,
        // so the compare starts at index 0.
        const FRAME: usize = 0x70;
        const HANDLE_SLOT: usize = 0x40; // the opened file handle
        const ROOTCSTR_SLOT: usize = 0x48;
        const ROOTWIN_SLOT: usize = 0x50; // root UTF-16 (CreateFileW input)
        const ROOTH_SLOT: usize = 0x58; // root directory HANDLE
        const ROOTFINAL_SLOT: usize = 0x60; // root final path (\\?\ prefixed)
        const FILEFINAL_SLOT: usize = 0x68; // file final path (\\?\ prefixed)
        let n = instructions.len();
        let fail = format!("{from}_wi_fail_{n}");
        let fail_close = format!("{from}_wi_failclose_{n}");
        let contained = format!("{from}_wi_ok_{n}");
        let loop_lbl = format!("{from}_wi_loop_{n}");
        let root_end = format!("{from}_wi_rootend_{n}");
        let done = format!("{from}_wi_done_{n}");
        instructions.extend([
            abi::subtract_stack(FRAME),
            abi::store_u64(abi::mfb_arg(0), abi::stack_pointer(), HANDLE_SLOT),
            abi::store_u64(abi::mfb_arg(1), abi::stack_pointer(), ROOTCSTR_SLOT),
        ]);
        arena_alloc_to_slot(from, "65536", ROOTWIN_SLOT, instructions, relocations);
        arena_alloc_to_slot(from, "65536", ROOTFINAL_SLOT, instructions, relocations);
        arena_alloc_to_slot(from, "65536", FILEFINAL_SLOT, instructions, relocations);
        emit_utf8_slot_to_wide(
            from,
            ROOTCSTR_SLOT,
            ROOTWIN_SLOT,
            "32768",
            instructions,
            relocations,
        );
        // CreateFileW(rootWide, 0, FILE_SHARE_RWD=7, NULL, OPEN_EXISTING=3,
        //             FILE_FLAG_BACKUP_SEMANTICS=0x02000000, NULL) — a directory
        // handle to resolve the root's own symlinks. Stage the three stack args
        // through ARG[2] BEFORE it is set to its register value (the SCRATCH pool
        // must not be used on Win64 — see emit_open_file).
        instructions.extend([
            abi::move_immediate(abi::mfb_arg(2), "Integer", "3"), // OPEN_EXISTING
            abi::store_u64(abi::mfb_arg(2), abi::stack_pointer(), 0x20),
            abi::move_immediate(abi::mfb_arg(2), "Integer", "33554432"), // FILE_FLAG_BACKUP_SEMANTICS
            abi::store_u64(abi::mfb_arg(2), abi::stack_pointer(), 0x28),
            abi::store_u64(abi::ZERO, abi::stack_pointer(), 0x30), // hTemplateFile NULL
            abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), ROOTWIN_SLOT),
            abi::move_immediate(abi::mfb_arg(1), "Integer", "0"), // dwDesiredAccess = 0 (metadata)
            abi::move_immediate(abi::mfb_arg(2), "Integer", "7"), // FILE_SHARE_READ|WRITE|DELETE
            abi::move_immediate(abi::mfb_arg(3), "Integer", "0"), // lpSecurityAttributes NULL
        ]);
        call_external(from, "CreateFileW", KERNEL32, instructions, relocations);
        instructions.extend([
            abi::store_u64(abi::c_return(0), abi::stack_pointer(), ROOTH_SLOT), // CreateFileW handle (C result)
            // INVALID_HANDLE_VALUE (-1) is negative as i64; a valid kernel handle is
            // a small positive value. A missing/unresolved root → refuse.
            abi::compare_immediate(abi::c_return(0), "0"),
            abi::branch_lt(&fail),
        ]);
        emit_final_path_call(from, ROOTH_SLOT, ROOTFINAL_SLOT, instructions, relocations);
        instructions.extend([
            abi::compare_immediate(abi::c_return(0), "0"),
            abi::branch_eq(&fail_close),
            // Close the root directory handle; the file handle stays open for the
            // caller (which closes it on a containment violation).
            abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), ROOTH_SLOT),
        ]);
        call_external(from, "CloseHandle", KERNEL32, instructions, relocations);
        emit_final_path_call(from, HANDLE_SLOT, FILEFINAL_SLOT, instructions, relocations);
        instructions.extend([
            abi::compare_immediate(abi::c_return(0), "0"),
            abi::branch_eq(&fail),
            // Prefix compare: fileFinal must begin with rootFinal, then a '\'.
            abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), ROOTFINAL_SLOT),
            abi::load_u64(abi::mfb_arg(1), abi::stack_pointer(), FILEFINAL_SLOT),
            abi::label(&loop_lbl),
            abi::load_u16(abi::mfb_arg(2), abi::mfb_arg(0), 0), // root char
            abi::compare_immediate(abi::mfb_arg(2), "0"),
            abi::branch_eq(&root_end),
            abi::load_u16(abi::mfb_arg(3), abi::mfb_arg(1), 0), // file char
        ]);
        emit_ascii_fold(abi::mfb_arg(2), n, instructions);
        emit_ascii_fold(abi::mfb_arg(3), n + 1, instructions);
        instructions.extend([
            abi::compare_registers(abi::mfb_arg(2), abi::mfb_arg(3)),
            abi::branch_ne(&fail),
            abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 2),
            abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(1), 2),
            abi::branch(&loop_lbl),
            abi::label(&root_end),
            // Root exhausted: the next file char must be the '\' separator, so
            // "\\?\C:\root" contains "\\?\C:\root\x" but NOT "\\?\C:\rootX".
            abi::load_u16(abi::mfb_arg(3), abi::mfb_arg(1), 0),
            abi::compare_immediate(abi::mfb_arg(3), "92"), // '\'
            abi::branch_eq(&contained),
            abi::branch(&fail),
            abi::label(&fail_close),
            abi::load_u64(abi::mfb_arg(0), abi::stack_pointer(), ROOTH_SLOT),
        ]);
        call_external(from, "CloseHandle", KERNEL32, instructions, relocations);
        instructions.extend([
            abi::branch(&fail),
            abi::label(&contained),
            abi::move_immediate(abi::return_register(), "Integer", "0"),
            abi::branch(&done),
            abi::label(&fail),
            abi::move_immediate(abi::return_register(), "Integer", "1"),
            abi::label(&done),
            abi::add_stack(FRAME),
        ]);
        Ok(())
    }

    // --- POSIX-struct constant accessors ----------------------------------
    // Windows has no termios/dirent/stat/addrinfo structs; 47-E raises this seam
    // to intent-level methods. Unreachable until a later sub-plan advertises the
    // surface, so a placeholder 0 is safe (and never read).

    fn emit_apply_raw_mode(
        &self,
        base_register: &str,
        original_offset: usize,
        modified_offset: usize,
        disable_echo: bool,
        disable_canonical: bool,
        instructions: &mut Vec<CodeInstruction>,
    ) {
        // Windows has no `struct termios`: emit_terminal_control_call(GetAttrs)
        // stored the console-input mode DWORD at base+original. Compute the raw
        // mode by masking: clear ENABLE_ECHO_INPUT (0x04) and/or ENABLE_LINE_INPUT
        // (0x02); keep ENABLE_PROCESSED_INPUT (0x01) so Ctrl-C still raises (matching
        // the POSIX path, which leaves ISIG on — plan-47-G Open Decision 3); and for
        // full raw (canonical off) set ENABLE_VIRTUAL_TERMINAL_INPUT (0x200) so VT
        // key sequences arrive. Store the result at base+modified for SetAttrs.
        let clear = if disable_echo { 4 } else { 0 } + if disable_canonical { 2 } else { 0 };
        instructions.extend([
            abi::load_u32(abi::mfb_arg(0), base_register, original_offset),
            // Build a 32-bit all-ones mask, then clear the target bits: mask =
            // 0xFFFFFFFF - clear (the bits are distinct powers of two, so a subtract
            // clears exactly them).
            abi::move_immediate(abi::mfb_arg(1), "Integer", "1"),
            abi::shift_left_immediate(abi::mfb_arg(1), abi::mfb_arg(1), 32),
            abi::subtract_immediate(abi::mfb_arg(1), abi::mfb_arg(1), 1), // 0xFFFFFFFF
            abi::subtract_immediate(abi::mfb_arg(1), abi::mfb_arg(1), clear),
            abi::and_registers(abi::mfb_arg(0), abi::mfb_arg(0), abi::mfb_arg(1)),
        ]);
        if disable_canonical {
            instructions.extend([
                abi::move_immediate(abi::mfb_arg(1), "Integer", "512"), // ENABLE_VIRTUAL_TERMINAL_INPUT
                abi::or_registers(abi::mfb_arg(0), abi::mfb_arg(0), abi::mfb_arg(1)),
            ]);
        }
        instructions.push(abi::store_u32(
            abi::mfb_arg(0),
            base_register,
            modified_offset,
        ));
    }

    fn emit_terminal_control_call(
        &self,
        call: crate::codegen::engine::types::TerminalControlCall,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        use crate::codegen::engine::types::TerminalControlCall;
        // Windows realizes the three POSIX terminal-control calls over the Console
        // API. The fd is in ARG[0]; resolve it to a std HANDLE the same way as
        // emit_write (GetStdHandle(-(fd+10)) — fd 0 → STD_INPUT).
        match call {
            // isatty(fd): GetConsoleMode succeeding IS the tty test.
            TerminalControlCall::IsATty => {
                self.emit_is_terminal(from, platform_imports, instructions, relocations)
            }
            // tcgetattr(fd, &out): GetConsoleMode(handle, &out) — store the mode
            // DWORD at [ARG[1]]. Return 0 on success, -1 on error (POSIX contract).
            TerminalControlCall::GetAttrs => {
                let n = instructions.len();
                let ok = format!("{from}_tga_ok_{n}");
                let done = format!("{from}_tga_done_{n}");
                instructions.extend([
                    abi::subtract_stack(0x30),
                    abi::store_u64(abi::mfb_arg(1), abi::stack_pointer(), 0x28), // save &out
                    abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(0), 10),
                    abi::move_immediate(abi::mfb_arg(0), "Integer", "0"),
                    abi::subtract_registers(abi::mfb_arg(0), abi::mfb_arg(0), abi::mfb_arg(1)), // -(fd+10)
                ]);
                call_external(from, "GetStdHandle", KERNEL32, instructions, relocations);
                instructions.extend([
                    abi::move_register(abi::mfb_arg(0), abi::c_return(0)), // handle (GetStdHandle C result)
                    abi::load_u64(abi::mfb_arg(1), abi::stack_pointer(), 0x28), // lpMode = &out
                ]);
                call_external(from, "GetConsoleMode", KERNEL32, instructions, relocations);
                instructions.extend([
                    abi::compare_immediate(abi::c_return(0), "0"),
                    abi::branch_ne(&ok),
                    abi::move_immediate(abi::return_register(), "Integer", "0"),
                    abi::subtract_immediate(abi::return_register(), abi::return_register(), 1),
                    abi::branch(&done),
                    abi::label(&ok),
                    abi::move_immediate(abi::return_register(), "Integer", "0"),
                    abi::label(&done),
                    abi::add_stack(0x30),
                ]);
                Ok(())
            }
            // tcsetattr(fd, TCSANOW, &in): SetConsoleMode(handle, *(DWORD*)&in).
            TerminalControlCall::SetAttrs => {
                let n = instructions.len();
                let ok = format!("{from}_tsa_ok_{n}");
                let done = format!("{from}_tsa_done_{n}");
                instructions.extend([
                    abi::subtract_stack(0x30),
                    abi::load_u32(abi::mfb_arg(1), abi::mfb_arg(2), 0), // dwMode from [&in]
                    abi::store_u64(abi::mfb_arg(1), abi::stack_pointer(), 0x28), // save mode
                    abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(0), 10),
                    abi::move_immediate(abi::mfb_arg(0), "Integer", "0"),
                    abi::subtract_registers(abi::mfb_arg(0), abi::mfb_arg(0), abi::mfb_arg(1)), // -(fd+10)
                ]);
                call_external(from, "GetStdHandle", KERNEL32, instructions, relocations);
                instructions.extend([
                    abi::move_register(abi::mfb_arg(0), abi::c_return(0)), // handle (GetStdHandle C result)
                    abi::load_u64(abi::mfb_arg(1), abi::stack_pointer(), 0x28), // dwMode
                ]);
                call_external(from, "SetConsoleMode", KERNEL32, instructions, relocations);
                instructions.extend([
                    abi::compare_immediate(abi::c_return(0), "0"),
                    abi::branch_ne(&ok),
                    abi::move_immediate(abi::return_register(), "Integer", "0"),
                    abi::subtract_immediate(abi::return_register(), abi::return_register(), 1),
                    abi::branch(&done),
                    abi::label(&ok),
                    abi::move_immediate(abi::return_register(), "Integer", "0"),
                    abi::label(&done),
                    abi::add_stack(0x30),
                ]);
                Ok(())
            }
        }
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
            abi::and_registers(mode, mode, mask),       // 0x10 iff a directory
            abi::compare_immediate(mode, "0"),
        ]);
        if expected_kind == crate::codegen::error::constants::FS_MODE_DIRECTORY {
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
            // `emit_readdir` returns the DIR* (or 0 at end) in the MFB result register,
            // NOT a C result — check `return_register()`, not `c_return`.
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
        // Windows ADDRINFOA orders ai_canonname (24) before ai_addr, like macOS.
        32
    }
    fn sol_socket(&self) -> &'static str {
        "65535" // SOL_SOCKET = 0xFFFF on Winsock
    }
    fn so_reuseaddr(&self) -> &'static str {
        "4" // SO_REUSEADDR = 0x0004 on Winsock
    }
    fn so_rcvtimeo(&self) -> &'static str {
        "4102" // SO_RCVTIMEO = 0x1006 on Winsock
    }
    fn so_sndtimeo(&self) -> &'static str {
        "4101" // SO_SNDTIMEO = 0x1005 on Winsock
    }
    fn so_rcvbuf(&self) -> &'static str {
        "4098" // SO_RCVBUF = 0x1002 on Winsock
    }
    // plan-110-A: Windows reaches ICMP through `iphlpapi`'s `IcmpSendEcho`, which
    // takes the TTL in an `IP_OPTION_INFORMATION` struct and reports the reply TTL
    // in `ICMP_ECHO_REPLY.Options.Ttl` — it never sets an IP-level socket option and
    // never reads a control message. These four therefore have no Windows analogue
    // and are unreachable there; `clock_monotonic` is likewise unused because the
    // API reports its own round-trip time. Returning a wrong-looking value would be
    // worse than refusing: any caller reaching one is a routing bug in the ping
    // backend's platform dispatch, so say so loudly.
    fn ipproto_ip(&self) -> &'static str {
        unreachable!("Windows ICMP uses iphlpapi, not IP-level socket options")
    }
    fn ip_ttl(&self) -> &'static str {
        unreachable!("Windows ICMP sets TTL via IP_OPTION_INFORMATION, not IP_TTL")
    }
    fn ip_recvttl(&self) -> &'static str {
        unreachable!("Windows ICMP reports TTL in ICMP_ECHO_REPLY, not via a cmsg")
    }
    fn cmsg_ip_ttl_type(&self) -> &'static str {
        unreachable!("Windows ICMP delivers no control messages")
    }
    fn clock_monotonic(&self) -> &'static str {
        unreachable!("Windows ICMP reports its own RoundTripTime; no clock_gettime")
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
        fd_offset: usize,
        _flags_offset: usize,
        from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // Non-blocking: ioctlsocket(fd, FIONBIO, &1). Winsock has no F_GETFL/F_SETFL,
        // so `flags_offset` is unused (the shared caller skips the F_GETFL read on
        // Windows). The argp `u_long` lives in a self-contained frame slot, so this
        // never touches the caller's frame beyond loading the fd.
        emit_ioctl_fionbio(from, fd_offset, true, instructions, relocations);
        Ok(())
    }

    fn emit_restore_blocking(
        &self,
        fd_offset: usize,
        _scratch_offset: usize,
        from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // Blocking: ioctlsocket(fd, FIONBIO, &0).
        emit_ioctl_fionbio(from, fd_offset, false, instructions, relocations);
        Ok(())
    }

    fn emit_net_startup(
        &self,
        from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        // WSAStartup(MAKEWORD(2,2), &wsadata) — mandatory before any ws2_32 call
        // (plan-47-I §3.2). WSADATA is 408 bytes on x64; park it above the shadow
        // space in a self-contained frame. The return value (0 on success) is
        // ignored: a failure leaves every later socket call to fail on its own,
        // which the net helpers already surface as an error.
        const FRAME: usize = 0x1c0;
        instructions.extend([
            abi::subtract_stack(FRAME),
            abi::move_immediate(abi::mfb_arg(0), "Integer", WINSOCK_VERSION),
            abi::add_immediate(abi::mfb_arg(1), abi::stack_pointer(), 0x20), // &wsadata
        ]);
        call_external(from, "WSAStartup", WS2_32, instructions, relocations);
        instructions.push(abi::add_stack(FRAME));
        Ok(())
    }

    fn emit_net_shutdown(
        &self,
        from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        instructions.push(abi::subtract_stack(0x20)); // shadow space
        call_external(from, "WSACleanup", WS2_32, instructions, relocations);
        instructions.push(abi::add_stack(0x20));
        Ok(())
    }
    fn so_error(&self) -> &'static str {
        "4103" // SO_ERROR = 0x1007 on Winsock
    }

    // --- threads / TLS (owned by 47-H/47-J) -------------------------------

    fn emit_thread_trampoline(
        &self,
        platform_imports: &HashMap<String, String>,
        uses_stdin: bool,
        arena_init: crate::codegen::engine::types::ArenaInitSymbols,
    ) -> Result<CodeFunction, String> {
        // The shared trampoline; its pthread_* calls route through this platform's
        // emit_thread_external_call Windows arms (CreateThread/SRWLOCK/condvar).
        crate::codegen::runtime::thread::lower_thread_trampoline(
            platform_imports,
            self,
            uses_stdin,
            arena_init,
        )
    }

    // ---- plan-66-J Win32 app-mode floor (delegates to win_x86_64::app) --------

    fn emit_app_program_entry(
        &self,
        spec: &AppEntrySpec,
        platform_imports: &HashMap<String, String>,
    ) -> Option<Result<Vec<CodeFunction>, String>> {
        Some(app::emit_app_program_entry(spec, platform_imports))
    }

    fn emit_app_io_write(
        &self,
        symbol: &str,
        stderr: bool,
        newline: bool,
        term_state_offset: Option<usize>,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Option<Result<(), String>> {
        app::emit_app_io_write(
            symbol,
            stderr,
            newline,
            term_state_offset,
            instructions,
            relocations,
        );
        Some(Ok(()))
    }

    fn emit_app_io_flush(
        &self,
        symbol: &str,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Option<Result<(), String>> {
        app::emit_app_io_flush(symbol, instructions, relocations);
        Some(Ok(()))
    }

    fn emit_app_io_input(
        &self,
        symbol: &str,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Option<Result<(), String>> {
        app::emit_app_io_input(symbol, instructions, relocations);
        Some(Ok(()))
    }

    fn emit_app_raw_input_mode(
        &self,
        _symbol: &str,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Option<Result<(), String>> {
        Some(app::emit_app_raw_input_mode())
    }

    fn emit_app_io_is_terminal(
        &self,
        symbol: &str,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Option<Result<(), String>> {
        app::emit_app_io_is_terminal(symbol, instructions, relocations);
        Some(Ok(()))
    }

    fn emit_app_term_helper(
        &self,
        call: &str,
        symbol: &str,
        term_state_offset: usize,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Option<Result<(), String>> {
        app::emit_app_term_helper(call, symbol, term_state_offset, instructions, relocations)
    }

    fn emit_app_mode_reconcile(
        &self,
        symbol: &str,
        presentation_mode_offset: usize,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Option<Result<(), String>> {
        // plan-98-A Phase 3: `app::setMode` reconciles the window surface. The
        // worker reloads the just-stored mode and `SendMessageW`s it to the main
        // window, so the UI thread (which owns the window) applies it synchronously
        // — a no-op headless, where there is no window and no message pump.
        app::emit_reconcile_seam(symbol, presentation_mode_offset, instructions, relocations);
        Some(Ok(()))
    }

    fn emit_canvas_blit(
        &self,
        symbol: &str,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Option<Result<(), String>> {
        // plan-98-C Phase 3: copy the frame out of the caller's block and post it to
        // the window, which paints it from WM_PAINT with SetDIBitsToDevice.
        app::emit_canvas_blit_seam(symbol, instructions, relocations);
        Some(Ok(()))
    }

    fn app_mode_data_objects(&self, project_name: &str) -> Vec<CodeDataObject> {
        app::app_mode_data_objects(project_name)
    }
}

#[cfg(test)]
mod c_result_tests {
    use super::*;
    use crate::arch::ops::CodeOp;
    use crate::codegen::engine::types::types::CodegenPlatform;
    use std::collections::HashMap;

    /// Emit one seam and report whether it ends by moving the C result into the
    /// aligned MFB return register.
    fn returns_the_c_result(
        emit: impl Fn(
            &Platform,
            &str,
            &HashMap<String, String>,
            &mut Vec<CodeInstruction>,
            &mut Vec<CodeRelocation>,
        ) -> Result<(), String>,
    ) -> bool {
        let mut instructions = Vec::new();
        let mut relocations = Vec::new();
        let imports = HashMap::new();
        emit(
            &Platform,
            "t",
            &imports,
            &mut instructions,
            &mut relocations,
        )
        .expect("seam emits");
        instructions.iter().any(|ins| {
            ins.op == CodeOp::Mov
                && ins.get("dst").as_deref() == Some(abi::return_register().render().as_str())
                && ins.get("src").as_deref() == Some(abi::c_return(0).render().as_str())
        })
    }

    /// A Win32 seam that promises a value "in the return register" must actually
    /// put it there.
    ///
    /// plan-85 split `%retC` (`rax`) from the aligned MFB bank (`rcx` on Win64), so
    /// a seam that ends at `call_external` and returns leaves its result in `rax`
    /// while every caller reads `abi::return_register()`. There is no type error and
    /// no crash — the caller reads whatever that register happened to hold, which
    /// for `emit_open_file` was `CreateFileW`'s own `lpFileName` argument: positive,
    /// so the `handle < 0` open-failed check passed, and the pointer was handed to
    /// `WriteFile` as a file handle.
    ///
    /// The symptom was `fs::writeText` raising `ErrWriteFailed` while leaving a
    /// **0-byte file** behind — the open really had worked. Every `fs` write on
    /// Windows was broken, and nothing caught it: the host is macOS, the acceptance
    /// harness cannot run a PE, and no test opens a file through this seam.
    ///
    /// `emit_heap_alloc` is in the list as a control: it already did the move, so a
    /// test that could only ever pass would not prove anything about its own reach.
    #[test]
    fn value_returning_win32_seams_move_the_c_result_into_the_return_register() {
        for (name, ok) in [
            (
                "emit_open_file",
                returns_the_c_result(Platform::emit_open_file),
            ),
            (
                "emit_heap_alloc",
                returns_the_c_result(Platform::emit_heap_alloc),
            ),
        ] {
            assert!(
                ok,
                "{name} promises its result in abi::return_register() but never moves \
                 abi::c_return(0) there — the caller will read whatever the aligned \
                 bank happened to hold (plan-85 split the two)"
            );
        }
    }

    /// Emit one seam and report the largest stack reservation it makes before its
    /// first external call.
    fn shadow_before_first_call(
        emit: impl Fn(
            &Platform,
            &str,
            &HashMap<String, String>,
            &mut Vec<CodeInstruction>,
            &mut Vec<CodeRelocation>,
        ) -> Result<(), String>,
    ) -> usize {
        let mut instructions = Vec::new();
        let mut relocations = Vec::new();
        let imports = HashMap::new();
        emit(
            &Platform,
            "t",
            &imports,
            &mut instructions,
            &mut relocations,
        )
        .expect("seam emits");
        let mut reserved = 0usize;
        for ins in &instructions {
            if matches!(ins.op, CodeOp::BranchLink | CodeOp::BranchLinkRegister) {
                break;
            }
            if ins.op == CodeOp::SubSp {
                if let Some(imm) = ins.get("imm").and_then(|v| v.parse::<usize>().ok()) {
                    reserved = reserved.max(imm);
                }
            }
        }
        reserved
    }

    /// A Win64 caller reserves the callee's shadow space, and every seam that calls
    /// out has to.
    ///
    /// The 32 bytes are not scratch below `rsp` — they are *above* it, in the caller's
    /// own frame, so a seam that calls without reserving them hands the callee 32 bytes
    /// of its own locals to spill into. `emit_random_bytes` did exactly that: it emitted
    /// no frame at all and called `BCryptGenRandom` straight through.
    ///
    /// The damage was invisible on the console path and fatal in app mode, where the
    /// corrupted frame belonged to code that then dereferenced it — an **empty**
    /// `SUB main() END SUB` died with `0xC0000005` inside ntdll's activation-context
    /// machinery, ~20 frames from anything this repository wrote, and every Windows
    /// `--app` program was unrunnable (bug-478).
    ///
    /// `emit_temp_directory` is the control: it already reserved a frame, so a test
    /// that could only ever pass would prove nothing about its own reach.
    #[test]
    fn every_calling_win32_seam_reserves_the_callees_shadow_space() {
        for (name, reserved) in [
            (
                "emit_random_bytes",
                shadow_before_first_call(Platform::emit_random_bytes),
            ),
            (
                "emit_temp_directory",
                shadow_before_first_call(Platform::emit_temp_directory),
            ),
        ] {
            assert!(
                reserved >= 0x20,
                "{name} reserves {reserved} bytes before its first call; Win64 requires \
                 the caller to leave the callee 32 bytes of shadow space, and those \
                 bytes are in the CALLER's frame (bug-478)"
            );
        }
    }
}

#[cfg(test)]
mod fionbio_tests {
    use super::*;
    use crate::arch::ops::CodeOp;

    /// bug-417: the `ioctlsocket(fd, cmd, &argp)` command immediate emitted by
    /// `emit_ioctl_fionbio` must be the real Winsock `FIONBIO`
    /// (`_IOW('f', 126, u_long)` = 0x8004667E = 2147772030). Before the fix the
    /// literal was 2147767422 (0x8004547E) — the `'f'` magic byte (0x66) corrupted
    /// to 0x54 — so `ioctlsocket` returned WSAEINVAL and Windows sockets never went
    /// non-blocking (connect timeouts never fired). This pins the exact immediate
    /// moved into `ARG[1]` (the `cmd`), so the corruption cannot silently return.
    #[test]
    fn ioctl_cmd_immediate_is_fionbio() {
        for nonblocking in [true, false] {
            let mut instructions = Vec::new();
            let mut relocations = Vec::new();
            emit_ioctl_fionbio("t", 0x10, nonblocking, &mut instructions, &mut relocations);
            let cmd = instructions
                .iter()
                .find(|ins| {
                    ins.op == CodeOp::MovImm
                        && ins.get("dst").as_deref() == Some(abi::mfb_arg(1).render().as_str())
                })
                .and_then(|ins| ins.get("value"))
                .expect("emit_ioctl_fionbio must move the FIONBIO cmd into ARG[1]");
            assert_eq!(
                cmd, "2147772030",
                "bug-417: ioctlsocket cmd must be FIONBIO 0x8004667E (2147772030), \
                 not the corrupted 0x8004547E (nonblocking={nonblocking})"
            );
        }
    }
}
