//! Windows (`win_x86_64`) native backend for the `process` package (plan-90-D).
//!
//! Reimplements the `process` surface over Win32 — `CreateProcessA` +
//! `WaitForSingleObject`/`GetExitCodeProcess` + `TerminateProcess` — sharing the
//! tag-10 record and resource record envelope. The handle word (`RESOURCE_OFFSET_HANDLE`)
//! holds the process `HANDLE`; the child pid is cached in `PROC_STATUS`,
//! the exit code in `PROC_EXITCODE`. Landed in phases, gated by the `win_x86_64`
//! capability list. plan-119 finished the surface: `shell` (over `cmd.exe /S /C`)
//! and the four-argument `spawn` (over `CreateProcessA`'s `lpEnvironment` and
//! `lpCurrentDirectory`) were the last two gaps, so no `unimplemented` arm
//! remains — every `process` member the registry declares has a Windows body.

// --- codegen tier imports (migration) ---
use super::gen_shared::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::error::emission::emit_fail;
use crate::target::shared::abi;
use crate::target::shared::abi::Reg;
use std::collections::HashMap;

mod command_line;
mod environment;
pub(crate) use command_line::*;
pub(crate) use environment::*;

// ---------------------------------------------------------------------------
// The shared Windows spawn tail (plan-119-A)
//
// The Win64 twin of `gen_unix/spawn.rs`'s `emit_spawn_tail`. Every Windows helper that
// creates a child (`spawn`, `shell`, `spawnEnv`) differs only in how it builds a
// command line, an environment block and a working directory; everything after
// that — three inheritable pipes, a `STARTUPINFOEXA` wired to the child ends
// and carrying the handle list that names them as the ONLY handles the child
// inherits (bug-499), `CreateProcessA`, handle hygiene, the tag-10 record — is
// identical, and lives here once.
//
// The frame below is the one the shipped one-argument spawn used, extended with
// the three input slots. It is `sp`-relative at **stack-adjust depth 1**: the
// caller owns a single `subtract_stack(FRAME)`/`add_stack(FRAME)` bracket and the
// whole body is written without abstract vregs, precisely so `finalize_frame`
// cannot spill anything and shift the six outgoing `CreateProcessA` stack args
// out from under the callee (`.ai/arch-abi.md`). A caller's own scratch slots
// therefore start at `WIN_SPAWN_SCRATCH`, and its `FRAME` must cover them and be
// 16-aligned.
//
//   [0x00..0x20)  shadow space for callees
//   [0x20..0x50)  CreateProcessA stack args 5..10
//   [SI..SI+112)  STARTUPINFOEXA (dwFlags@60, hStdInput@80/hStdOutput@88/hStdError@96,
//                 lpAttributeList@104)
//   [PI..PI+24)   PROCESS_INFORMATION (hProcess@0, hThread@8, dwProcessId@16)
//   [SA..SA+24)   SECURITY_ATTRIBUTES (nLength@0, lpSD@8, bInheritHandle@16)
//   IN_R/IN_W/OUT_R/OUT_W/ERR_R/ERR_W  CreatePipe out-handle slots
//   REC           the allocated resource record
//   CMD/ENV/CWD   the caller's three inputs (see `emit_win_spawn_tail`)
//   ATTR_SIZE     SIZE_T the attribute-list size query fills (bug-499)
//   HANDLES       HANDLE[3] the child may inherit: IN_R, OUT_W, ERR_W (bug-499)
//   ATTR_LIST     the opaque PROC_THREAD_ATTRIBUTE_LIST buffer (bug-499)
//   CP_RESULT     CreateProcessA's BOOL, kept across the list's deletion (bug-499)
// ---------------------------------------------------------------------------

/// `STARTUPINFOEXA` (112 bytes: the 104-byte `STARTUPINFOA` followed by
/// `lpAttributeList`, bug-499).
pub(crate) const WIN_SPAWN_SI: usize = 0x50;
/// `PROCESS_INFORMATION` (24 bytes).
pub(crate) const WIN_SPAWN_PI: usize = 0xC0;
/// `SECURITY_ATTRIBUTES` (24 bytes).
pub(crate) const WIN_SPAWN_SA: usize = 0xD8;
/// Child stdin read end (the child inherits it).
pub(crate) const WIN_SPAWN_IN_R: usize = 0xF0;
/// Parent stdin write end (kept in the record).
pub(crate) const WIN_SPAWN_IN_W: usize = 0xF8;
/// Parent stdout read end (kept in the record).
pub(crate) const WIN_SPAWN_OUT_R: usize = 0x100;
/// Child stdout write end (the child inherits it).
pub(crate) const WIN_SPAWN_OUT_W: usize = 0x108;
/// Parent stderr read end (kept in the record).
pub(crate) const WIN_SPAWN_ERR_R: usize = 0x110;
/// Child stderr write end (the child inherits it).
pub(crate) const WIN_SPAWN_ERR_W: usize = 0x118;
/// The allocated `Process` record.
pub(crate) const WIN_SPAWN_REC: usize = 0x120;
/// **Caller input**: the NUL-terminated `lpCommandLine`. Never NULL.
pub(crate) const WIN_SPAWN_CMD: usize = 0x128;
/// **Caller input**: `lpEnvironment` — an ANSI `name=value\0…\0\0` block, or 0
/// to inherit the parent's environment.
pub(crate) const WIN_SPAWN_ENV: usize = 0x130;
/// **Caller input**: `lpCurrentDirectory` — a NUL-terminated path, or 0 to
/// inherit the parent's working directory.
pub(crate) const WIN_SPAWN_CWD: usize = 0x138;
/// bug-499: the `SIZE_T` `InitializeProcThreadAttributeList`'s size query fills.
const WIN_SPAWN_ATTR_SIZE: usize = 0x140;
/// bug-499: `HANDLE[3]` — the only handles the child inherits (IN_R, OUT_W, ERR_W).
const WIN_SPAWN_HANDLES: usize = 0x148;
/// bug-499: the opaque `PROC_THREAD_ATTRIBUTE_LIST` buffer, 16-aligned.
const WIN_SPAWN_ATTR_LIST: usize = 0x160;
/// Capacity of that buffer. The list is opaque, so the size query's answer is
/// checked against this before the list is initialized in place: a Windows that
/// ever needs more fails the spawn (`ErrSpawnFailed`) instead of overrunning the
/// frame.
const WIN_SPAWN_ATTR_LIST_CAP: &str = "128";
/// bug-499: `CreateProcessA`'s BOOL, kept across `DeleteProcThreadAttributeList`
/// (which clobbers the C return register) so the failure branch reads the truth.
const WIN_SPAWN_CP_RESULT: usize = 0x1E0;
/// First offset a caller may use for its own scratch slots.
pub(crate) const WIN_SPAWN_SCRATCH: usize = 0x1E8;

const SI_DWFLAGS: usize = 60;
const SI_HSTDIN: usize = 80;
const SI_HSTDOUT: usize = 88;
const SI_HSTDERR: usize = 96;
const HANDLE_FLAG_INHERIT: &str = "1";
const STARTF_USESTDHANDLES: &str = "256"; // 0x100
/// bug-499: `STARTUPINFOEXA.lpAttributeList` follows the 104-byte `STARTUPINFOA`.
const SI_LPATTRIBUTELIST: usize = 104;
/// bug-499: `sizeof(STARTUPINFOEXA)` — what `cb` must carry for the EX struct.
const STARTUPINFOEXA_SIZE: usize = 112;
/// bug-499: `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` = `ProcThreadAttributeValue(2, FALSE, TRUE, FALSE)`
/// = `2 | 0x20000` = 0x20002.
const PROC_THREAD_ATTRIBUTE_HANDLE_LIST: &str = "131074";
/// bug-499: `EXTENDED_STARTUPINFO_PRESENT` (0x00080000) in `dwCreationFlags`
/// tells `CreateProcessA` that `lpStartupInfo` is a `STARTUPINFOEXA`.
const EXTENDED_STARTUPINFO_PRESENT: &str = "524288";

/// Emit everything a Windows child needs once its command line is built.
///
/// On entry (all at depth-1 `sp`): `WIN_SPAWN_CMD` holds the NUL-terminated
/// command line, `WIN_SPAWN_ENV` an environment block pointer or 0, and
/// `WIN_SPAWN_CWD` a working-directory C string or 0. **All three must be
/// stored by the caller** — a helper that inherits both simply stores `ZERO`
/// into `ENV` and `CWD`.
///
/// On success the `Process` record is left in `RESULT_VALUE_REGISTER` with the
/// OK tag and control branches to `done`. A `CreatePipe`/`CreateProcessA`
/// failure branches to `spawn_fail`; an arena failure to `alloc_fail`. The
/// caller emits both handlers (`ErrSpawnFailed` / `ErrOutOfMemory`) and the
/// `done` label itself.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_win_spawn_tail(
    symbol: &str,
    alloc_fail: &str,
    spawn_fail: &str,
    done: &str,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let sp = abi::stack_pointer();
    // SECURITY_ATTRIBUTES{ nLength = 24, lpSecurityDescriptor = NULL,
    // bInheritHandle = TRUE } — both pipe ends inheritable, then the parent
    // end of each is stripped of inheritance via SetHandleInformation.
    instructions.extend([
        abi::move_immediate(abi::mfb_arg(0), "Integer", "24"),
        abi::store_u32(abi::mfb_arg(0), sp, WIN_SPAWN_SA),
        abi::store_u64(abi::ZERO, sp, WIN_SPAWN_SA + 8),
        abi::move_immediate(abi::mfb_arg(0), "Integer", "1"),
        abi::store_u32(abi::mfb_arg(0), sp, WIN_SPAWN_SA + 16),
    ]);
    // Three anonymous pipes: stdin (parent writes IN_W → child reads IN_R),
    // stdout (child writes OUT_W → parent reads OUT_R), stderr (ERR_W/ERR_R).
    // CreatePipe(&read, &write, &sa, 0); on FALSE → spawn_fail.
    for (read_slot, write_slot) in [
        (WIN_SPAWN_IN_R, WIN_SPAWN_IN_W),
        (WIN_SPAWN_OUT_R, WIN_SPAWN_OUT_W),
        (WIN_SPAWN_ERR_R, WIN_SPAWN_ERR_W),
    ] {
        instructions.extend([
            abi::add_immediate(abi::mfb_arg(0), sp, read_slot),
            abi::add_immediate(abi::mfb_arg(1), sp, write_slot),
            abi::add_immediate(abi::mfb_arg(2), sp, WIN_SPAWN_SA),
            abi::move_immediate(abi::mfb_arg(3), "Integer", "0"),
        ]);
        platform.emit_external_call(
            "CreatePipe",
            symbol,
            platform_imports,
            instructions,
            relocations,
        )?;
        instructions.extend([
            abi::sign_extend_word(abi::c_return(0), abi::c_return(0)),
            abi::compare_immediate(abi::c_return(0), "0"),
            abi::branch_eq(spawn_fail),
        ]);
    }
    // Strip inheritance from each parent-held end (IN_W/OUT_R/ERR_R) so the child
    // does not hold a duplicate that would keep a pipe from reaching EOF.
    for parent_slot in [WIN_SPAWN_IN_W, WIN_SPAWN_OUT_R, WIN_SPAWN_ERR_R] {
        instructions.extend([
            abi::load_u64(abi::mfb_arg(0), sp, parent_slot),
            abi::move_immediate(abi::mfb_arg(1), "Integer", HANDLE_FLAG_INHERIT),
            abi::move_immediate(abi::mfb_arg(2), "Integer", "0"),
        ]);
        platform.emit_external_call(
            "SetHandleInformation",
            symbol,
            platform_imports,
            instructions,
            relocations,
        )?;
    }
    // bug-499: build the PROC_THREAD_ATTRIBUTE_HANDLE_LIST naming the ONLY handles
    // the child may inherit — its three stdio pipe ends. `bInheritHandles` stays
    // TRUE: the list is honoured only then, and the listed handles must themselves
    // be inheritable (the SECURITY_ATTRIBUTES above made them so). Every other
    // inheritable handle in this process — every Winsock socket is one by
    // default, and so is a pipe end a concurrent spawn on another thread has just
    // created — is withheld from the child. Before this, `bInheritHandles = TRUE`
    // with no list handed the child all of them.
    //
    // 1. Size query: InitializeProcThreadAttributeList(NULL, 1, 0, &size) fails by
    //    design (ERROR_INSUFFICIENT_BUFFER) and writes the byte count. The list is
    //    opaque, so the count is checked against the frame buffer's capacity.
    let attr_fail = format!("{symbol}_attr_fail");
    instructions.extend([
        abi::store_u64(abi::ZERO, sp, WIN_SPAWN_ATTR_SIZE),
        abi::move_immediate(abi::mfb_arg(0), "Integer", "0"), // lpAttributeList = NULL
        abi::move_immediate(abi::mfb_arg(1), "Integer", "1"), // dwAttributeCount
        abi::move_immediate(abi::mfb_arg(2), "Integer", "0"), // dwFlags (reserved)
        abi::add_immediate(abi::mfb_arg(3), sp, WIN_SPAWN_ATTR_SIZE), // lpSize
    ]);
    platform.emit_external_call(
        "InitializeProcThreadAttributeList",
        symbol,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::load_u64(abi::mfb_arg(0), sp, WIN_SPAWN_ATTR_SIZE),
        abi::compare_immediate(abi::mfb_arg(0), WIN_SPAWN_ATTR_LIST_CAP),
        abi::branch_gt(spawn_fail),
        // 2. Initialize the list in the frame buffer: InitializeProcThreadAttributeList(
        //    &list, 1, 0, &size); FALSE → spawn_fail (nothing to delete yet).
        abi::add_immediate(abi::mfb_arg(0), sp, WIN_SPAWN_ATTR_LIST),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "1"),
        abi::move_immediate(abi::mfb_arg(2), "Integer", "0"),
        abi::add_immediate(abi::mfb_arg(3), sp, WIN_SPAWN_ATTR_SIZE),
    ]);
    platform.emit_external_call(
        "InitializeProcThreadAttributeList",
        symbol,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(abi::c_return(0), abi::c_return(0)),
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_eq(spawn_fail),
        // 3. HANDLE[3] = { IN_R, OUT_W, ERR_W }; UpdateProcThreadAttribute(&list, 0,
        //    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, handles, 3 * sizeof(HANDLE), NULL,
        //    NULL). Its stack args 5..7 use the same sp+0x20.. slots CreateProcessA's
        //    take — those are stored later, after this call has returned.
        abi::load_u64(abi::mfb_arg(0), sp, WIN_SPAWN_IN_R),
        abi::store_u64(abi::mfb_arg(0), sp, WIN_SPAWN_HANDLES),
        abi::load_u64(abi::mfb_arg(0), sp, WIN_SPAWN_OUT_W),
        abi::store_u64(abi::mfb_arg(0), sp, WIN_SPAWN_HANDLES + 8),
        abi::load_u64(abi::mfb_arg(0), sp, WIN_SPAWN_ERR_W),
        abi::store_u64(abi::mfb_arg(0), sp, WIN_SPAWN_HANDLES + 16),
        abi::move_immediate(abi::mfb_arg(0), "Integer", "24"),
        abi::store_u64(abi::mfb_arg(0), sp, 0x20), // 5th cbSize = 3 * sizeof(HANDLE)
        abi::store_u64(abi::ZERO, sp, 0x28),       // 6th lpPreviousValue = NULL
        abi::store_u64(abi::ZERO, sp, 0x30),       // 7th lpReturnSize = NULL
        abi::add_immediate(abi::mfb_arg(0), sp, WIN_SPAWN_ATTR_LIST),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "0"), // dwFlags (reserved)
        abi::move_immediate(
            abi::mfb_arg(2),
            "Integer",
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
        ),
        abi::add_immediate(abi::mfb_arg(3), sp, WIN_SPAWN_HANDLES), // lpValue
    ]);
    platform.emit_external_call(
        "UpdateProcThreadAttribute",
        symbol,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(abi::c_return(0), abi::c_return(0)),
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_eq(&attr_fail),
    ]);
    // Zero STARTUPINFOEXA (112 bytes), set cb = 112, dwFlags = STARTF_USESTDHANDLES,
    // the three child-end handles, and lpAttributeList.
    for off in (0..STARTUPINFOEXA_SIZE).step_by(8) {
        instructions.push(abi::store_u64(abi::ZERO, sp, WIN_SPAWN_SI + off));
    }
    instructions.extend([
        abi::move_immediate(abi::mfb_arg(0), "Integer", "112"),
        abi::store_u32(abi::mfb_arg(0), sp, WIN_SPAWN_SI),
        abi::move_immediate(abi::mfb_arg(0), "Integer", STARTF_USESTDHANDLES),
        abi::store_u32(abi::mfb_arg(0), sp, WIN_SPAWN_SI + SI_DWFLAGS),
        abi::load_u64(abi::mfb_arg(0), sp, WIN_SPAWN_IN_R),
        abi::store_u64(abi::mfb_arg(0), sp, WIN_SPAWN_SI + SI_HSTDIN),
        abi::load_u64(abi::mfb_arg(0), sp, WIN_SPAWN_OUT_W),
        abi::store_u64(abi::mfb_arg(0), sp, WIN_SPAWN_SI + SI_HSTDOUT),
        abi::load_u64(abi::mfb_arg(0), sp, WIN_SPAWN_ERR_W),
        abi::store_u64(abi::mfb_arg(0), sp, WIN_SPAWN_SI + SI_HSTDERR),
        abi::add_immediate(abi::mfb_arg(0), sp, WIN_SPAWN_ATTR_LIST),
        abi::store_u64(abi::mfb_arg(0), sp, WIN_SPAWN_SI + SI_LPATTRIBUTELIST),
        // CreateProcessA(NULL, cmd, NULL, NULL, TRUE, EXTENDED_STARTUPINFO_PRESENT,
        // env, cwd, &siex, &pi).
        // Win64: register args 0..3 in mfb_arg (rcx/rdx/r8/r9); stack args 5..10
        // stored directly at sp+0x20.. (after the 32-byte shadow).
        abi::move_immediate(abi::mfb_arg(0), "Integer", "1"),
        abi::store_u64(abi::mfb_arg(0), sp, 0x20), // 5th bInheritHandles = TRUE (the list needs it)
        abi::move_immediate(abi::mfb_arg(0), "Integer", EXTENDED_STARTUPINFO_PRESENT),
        abi::store_u64(abi::mfb_arg(0), sp, 0x28), // 6th dwCreationFlags
        abi::load_u64(abi::mfb_arg(0), sp, WIN_SPAWN_ENV),
        abi::store_u64(abi::mfb_arg(0), sp, 0x30), // 7th lpEnvironment
        abi::load_u64(abi::mfb_arg(0), sp, WIN_SPAWN_CWD),
        abi::store_u64(abi::mfb_arg(0), sp, 0x38), // 8th lpCurrentDirectory
        abi::add_immediate(abi::mfb_arg(0), sp, WIN_SPAWN_SI),
        abi::store_u64(abi::mfb_arg(0), sp, 0x40), // 9th &si
        abi::add_immediate(abi::mfb_arg(0), sp, WIN_SPAWN_PI),
        abi::store_u64(abi::mfb_arg(0), sp, 0x48), // 10th &pi
        // A register arg is zeroed with an immediate, NOT `move_register(_, ZERO)`:
        // x86-64 has no hardware zero register, so `ZERO` maps to a GPR holding
        // garbage (only `store_*` special-cases it to an immediate 0).
        abi::move_immediate(abi::mfb_arg(0), "Integer", "0"), // lpApplicationName NULL
        abi::load_u64(abi::mfb_arg(1), sp, WIN_SPAWN_CMD),    // lpCommandLine
        abi::move_immediate(abi::mfb_arg(2), "Integer", "0"), // lpProcessAttributes NULL
        abi::move_immediate(abi::mfb_arg(3), "Integer", "0"), // lpThreadAttributes NULL
    ]);
    platform.emit_external_call(
        "CreateProcessA",
        symbol,
        platform_imports,
        instructions,
        relocations,
    )?;
    // bug-499: the attribute list is deleted on BOTH outcomes. Its BOOL is parked
    // in the frame first because DeleteProcThreadAttributeList clobbers the C
    // return register.
    instructions.extend([
        abi::sign_extend_word(abi::c_return(0), abi::c_return(0)),
        abi::store_u64(abi::c_return(0), sp, WIN_SPAWN_CP_RESULT),
        abi::add_immediate(abi::mfb_arg(0), sp, WIN_SPAWN_ATTR_LIST),
    ]);
    platform.emit_external_call(
        "DeleteProcThreadAttributeList",
        symbol,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::load_u64(abi::mfb_arg(0), sp, WIN_SPAWN_CP_RESULT),
        abi::compare_immediate(abi::mfb_arg(0), "0"),
        abi::branch_eq(spawn_fail),
    ]);
    // Close the child-end handles the parent no longer needs + the thread handle.
    for close_slot in [
        WIN_SPAWN_PI + 8,
        WIN_SPAWN_IN_R,
        WIN_SPAWN_OUT_W,
        WIN_SPAWN_ERR_W,
    ] {
        instructions.push(abi::load_u64(abi::mfb_arg(0), sp, close_slot));
        platform.emit_external_call(
            "CloseHandle",
            symbol,
            platform_imports,
            instructions,
            relocations,
        )?;
    }
    // Allocate + stamp the record.
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", RESOURCE_RECORD_SIZE),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.extend([
        abi::store_u64(abi::mfb_return(1), sp, WIN_SPAWN_REC),
        abi::load_u64(abi::mfb_arg(0), sp, WIN_SPAWN_REC),
        abi::move_immediate(abi::mfb_arg(1), "Integer", RESOURCE_TAG_PROCESS),
        abi::store_u64(abi::mfb_arg(1), abi::mfb_arg(0), RESOURCE_OFFSET_TAG),
        abi::load_u64(abi::mfb_arg(1), sp, WIN_SPAWN_PI), // hProcess
        abi::store_u64(abi::mfb_arg(1), abi::mfb_arg(0), RESOURCE_OFFSET_HANDLE),
        abi::store_u64(abi::ZERO, abi::mfb_arg(0), RESOURCE_OFFSET_CLOSED),
        // bug-755: a fresh record is unowned until a binding claims it.
        abi::store_u64(abi::ZERO, abi::mfb_arg(0), RESOURCE_OFFSET_OWNER),
        abi::store_u64(abi::ZERO, abi::mfb_arg(0), RESOURCE_OFFSET_STATE),
        abi::load_u64(abi::mfb_arg(1), sp, WIN_SPAWN_IN_W),
        abi::store_u64(abi::mfb_arg(1), abi::mfb_arg(0), PROC_STDIN_W),
        abi::load_u64(abi::mfb_arg(1), sp, WIN_SPAWN_OUT_R),
        abi::store_u64(abi::mfb_arg(1), abi::mfb_arg(0), PROC_STDOUT_R),
        abi::load_u64(abi::mfb_arg(1), sp, WIN_SPAWN_ERR_R),
        abi::store_u64(abi::mfb_arg(1), abi::mfb_arg(0), PROC_STDERR_R),
        abi::store_u64(abi::ZERO, abi::mfb_arg(0), PROC_REAPED),
        abi::load_u32(abi::mfb_arg(1), sp, WIN_SPAWN_PI + 16), // dwProcessId
        abi::store_u64(abi::mfb_arg(1), abi::mfb_arg(0), PROC_STATUS), // pid cached here on Windows
        abi::store_u64(abi::ZERO, abi::mfb_arg(0), PROC_EXITCODE),
        abi::store_u64(abi::ZERO, abi::mfb_arg(0), PROC_STDOUT_BUF),
        abi::store_u64(abi::ZERO, abi::mfb_arg(0), PROC_STDERR_BUF),
        abi::move_register(RESULT_VALUE_REGISTER, abi::mfb_arg(0)),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(done),
        // bug-499: UpdateProcThreadAttribute failed after the list was initialized —
        // delete it, then report the spawn failure.
        abi::label(&attr_fail),
        abi::add_immediate(abi::mfb_arg(0), sp, WIN_SPAWN_ATTR_LIST),
    ]);
    platform.emit_external_call(
        "DeleteProcThreadAttributeList",
        symbol,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.push(abi::branch(spawn_fail));
    Ok(())
}

// process.closeInput — close the parent's stdin write handle (signals the child's
// stdin EOF); mark it -1. Idempotent per-record via the -1 sentinel.

/// Emit a blocking `WriteFile(fd, [src_slot], [rem_slot], &[written_slot], NULL)`
/// loop that drains `rem` bytes from `src` on the pipe handle in `fd_slot`. The
/// `lpOverlapped` (5th, stack) arg goes at `sp+0x20`. On a `FALSE` return or a
/// zero-byte write (broken pipe) it branches to `fail`. `tag` disambiguates the
/// per-call labels (payload vs the trailing newline).
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_write_all(
    symbol: &str,
    tag: &str,
    sp: impl Into<Reg>,
    fd_slot: usize,
    src_slot: usize,
    rem_slot: usize,
    written_slot: usize,
    fail: &str,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let sp: Reg = sp.into();
    let loop_l = format!("{symbol}_{tag}_loop");
    let progress = format!("{symbol}_{tag}_progress");
    let done = format!("{symbol}_{tag}_done");
    instructions.extend([
        abi::label(&loop_l),
        abi::load_u64(abi::mfb_arg(1), sp, rem_slot),
        abi::compare_immediate(abi::mfb_arg(1), "0"),
        abi::branch_eq(&done),
        // WriteFile(fd, src, rem, &written, NULL)
        abi::load_u64(abi::mfb_arg(0), sp, fd_slot),
        abi::load_u64(abi::mfb_arg(1), sp, src_slot),
        abi::load_u64(abi::mfb_arg(2), sp, rem_slot),
        abi::add_immediate(abi::mfb_arg(3), sp, written_slot),
        abi::store_u64(abi::ZERO, sp, 0x20),
    ]);
    platform.emit_external_call(
        "WriteFile",
        symbol,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(abi::c_return(0), abi::c_return(0)),
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_eq(fail),
        abi::load_u32(abi::mfb_arg(0), sp, written_slot),
        abi::compare_immediate(abi::mfb_arg(0), "0"),
        abi::branch_le(fail),
        abi::label(&progress),
        abi::load_u64(abi::mfb_arg(1), sp, src_slot),
        abi::add_registers(abi::mfb_arg(1), abi::mfb_arg(1), abi::mfb_arg(0)),
        abi::store_u64(abi::mfb_arg(1), sp, src_slot),
        abi::load_u64(abi::mfb_arg(1), sp, rem_slot),
        abi::subtract_registers(abi::mfb_arg(1), abi::mfb_arg(1), abi::mfb_arg(0)),
        abi::store_u64(abi::mfb_arg(1), sp, rem_slot),
        abi::branch(&loop_l),
        abi::label(&done),
    ]);
    Ok(())
}

// process.send / sendBytes / sendTimeout / sendBytesTimeout — write the payload
// to the child's stdin (parent's write end), then a trailing '\n' for the String
// form. `is_bytes` writes the raw List OF Byte contiguously (no newline);
// `with_timeout` accepts the extra `timeoutMs` arg. Windows anonymous pipes have
// no clean write-readiness poll, so the timeout is a best-effort upper bound: the
// blocking WriteFile returns immediately for a draining reader (the common case)
// and does not preempt a genuinely full pipe (a documented Windows limit,
// mirroring `didSignal`).
pub(crate) fn lower_process_send_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    is_bytes: bool,
    _with_timeout: bool,
) -> Result<ProcBodyParts, String> {
    const WRITTEN: usize = 0x28;
    const FILE: usize = 0x30;
    const FD: usize = 0x38;
    const SRCP: usize = 0x40;
    const REM: usize = 0x48;
    const LF: usize = 0x50;
    const FRAME: usize = 0x60;
    let sp = abi::stack_pointer();
    let closed_l = format!("{symbol}_closed");
    let done = format!("{symbol}_done");
    let mut relocations = Vec::new();
    // The payload pointer arrives in the 2nd MFB arg; stash it before it is
    // clobbered, then derive (srcp, rem).
    let mut instructions = vec![
        abi::subtract_stack(FRAME),
        abi::store_u64(abi::return_register(), sp, FILE),
        abi::store_u64(abi::mfb_arg(1), sp, SRCP), // payload object (temp)
        abi::load_u64(abi::mfb_arg(0), sp, FILE),
        abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), RESOURCE_OFFSET_CLOSED),
        abi::compare_immediate(abi::mfb_arg(1), "0"),
        abi::branch_ne(&closed_l),
        abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), PROC_STDIN_W),
        abi::compare_immediate(abi::mfb_arg(1), "0"),
        abi::branch_lt(&closed_l), // stdin -1 sentinel / closed
        abi::store_u64(abi::mfb_arg(1), sp, FD),
        abi::load_u64(abi::mfb_arg(0), sp, SRCP), // payload object
    ];
    if is_bytes {
        // List OF Byte: count in the header, bytes contiguous at data + HEADER
        // (byte_list_entry_stride() == 0).
        instructions.extend([
            abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), COLLECTION_OFFSET_COUNT),
            abi::store_u64(abi::mfb_arg(1), sp, REM),
            abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), COLLECTION_HEADER_SIZE),
            abi::store_u64(abi::mfb_arg(0), sp, SRCP),
        ]);
    } else {
        // String: length@0, bytes@8.
        instructions.extend([
            abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), 0),
            abi::store_u64(abi::mfb_arg(1), sp, REM),
            abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 8),
            abi::store_u64(abi::mfb_arg(0), sp, SRCP),
        ]);
    }
    emit_write_all(
        symbol,
        "payload",
        sp,
        FD,
        SRCP,
        REM,
        WRITTEN,
        &closed_l,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    if !is_bytes {
        // Trailing newline (line framing, matching the Unix send).
        instructions.extend([
            abi::move_immediate(abi::mfb_arg(0), "Integer", "10"),
            abi::store_u8(abi::mfb_arg(0), sp, LF),
            abi::add_immediate(abi::mfb_arg(0), sp, LF),
            abi::store_u64(abi::mfb_arg(0), sp, SRCP),
            abi::move_immediate(abi::mfb_arg(0), "Integer", "1"),
            abi::store_u64(abi::mfb_arg(0), sp, REM),
        ]);
        emit_write_all(
            symbol,
            "nl",
            sp,
            FD,
            SRCP,
            REM,
            WRITTEN,
            &closed_l,
            platform,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
    }
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&closed_l),
    ]);
    emit_fail(
        symbol,
        "ErrResourceClosed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::add_stack(FRAME), abi::return_()]);
    Ok((instructions, relocations, 0))
}
pub(crate) fn lower_process_drop_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    // Explicit Win64 frame (depth-1, no vregs): shadow [0x00..0x20) + FILE slot
    // (the record pointer, live across TerminateProcess/CloseHandle). The shadow
    // reservation is mandatory (see `lower_process_waitfor_helper`).
    const FILE: usize = 0x20;
    const FRAME: usize = 0x30;
    let sp = abi::stack_pointer();
    let done = format!("{symbol}_done");
    let done_ok = format!("{symbol}_done_ok");
    let already = format!("{symbol}_already");
    let mut relocations = Vec::new();
    let mut instructions = vec![
        abi::label("entry"),
        abi::subtract_stack(FRAME),
        abi::store_u64(abi::return_register(), sp, FILE),
        abi::load_u64(abi::mfb_arg(0), sp, FILE),
        abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), RESOURCE_OFFSET_CLOSED),
        abi::compare_immediate(abi::mfb_arg(1), "0"),
        abi::branch_ne(&done_ok),
        abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), PROC_REAPED),
        abi::compare_immediate(abi::mfb_arg(1), "0"),
        abi::branch_ne(&already),
        // TerminateProcess(hProcess, 1)
        abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), RESOURCE_OFFSET_HANDLE),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "1"),
    ];
    platform.emit_external_call(
        "TerminateProcess",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.push(abi::label(&already));
    // CloseHandle(hProcess)
    instructions.extend([
        abi::load_u64(abi::mfb_arg(0), sp, FILE),
        abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), RESOURCE_OFFSET_HANDLE),
    ]);
    platform.emit_external_call(
        "CloseHandle",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    // Hand back the two spill blocks `waitFor` may have grown (bug-475) — each can
    // reach SPILL_MAX_CAPACITY, and the handle is closed on this path, so nothing
    // can read them afterwards.
    for off in [PROC_STDOUT_BUF, PROC_STDERR_BUF] {
        let skip = format!("{symbol}_skip_spill_{off}");
        instructions.extend([
            abi::load_u64(abi::mfb_arg(0), sp, FILE),
            abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), off),
            abi::compare_immediate(abi::mfb_arg(1), "0"),
            abi::branch_eq(&skip),
            abi::store_u64(abi::ZERO, abi::mfb_arg(0), off),
            abi::load_u64(abi::mfb_arg(2), abi::mfb_arg(1), SPILL_CAPACITY),
            abi::add_immediate(abi::mfb_arg(2), abi::mfb_arg(2), SPILL_DATA),
            abi::move_register(abi::return_register(), abi::mfb_arg(1)),
            abi::move_register(abi::mfb_arg(1), abi::mfb_arg(2)),
        ]);
        emit_arena_free(symbol, &mut instructions, &mut relocations);
        instructions.push(abi::label(&skip));
    }
    instructions.extend([
        abi::load_u64(abi::mfb_arg(0), sp, FILE),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "1"),
        abi::store_u64(abi::mfb_arg(1), abi::mfb_arg(0), RESOURCE_OFFSET_CLOSED),
        abi::label(&done_ok),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::label(&done),
        abi::add_stack(FRAME),
        abi::return_(),
    ]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

#[cfg(test)]
mod tests {
    // bug-499: emit-inspection guards for the Windows spawn tail. Runtime proof is
    // Windows-only (box 2230, `scripts/test-winprocess.sh`); these lower the three
    // helpers that share `emit_win_spawn_tail` with the real Win64 platform and pin
    // the handle-inheritance contract in the neutral instruction stream.
    use super::*;
    use crate::arch::ops::CodeOp;
    use crate::codegen::builtins::process::func_shell::lower_process_shell_helper_win;
    use crate::codegen::builtins::process::func_spawn::lower_process_spawn_helper_win;
    use crate::codegen::engine::mir;

    fn kernel32_imports() -> HashMap<String, String> {
        [
            "CreateProcessA",
            "CreatePipe",
            "SetHandleInformation",
            "WriteFile",
            "ReadFile",
            "PeekNamedPipe",
            "SetNamedPipeHandleState",
            "GetTickCount64",
            "Sleep",
            "WaitForSingleObject",
            "GetExitCodeProcess",
            "TerminateProcess",
            "CloseHandle",
            "GetLastError",
            "GetEnvironmentStringsA",
            "FreeEnvironmentStringsA",
            "InitializeProcThreadAttributeList",
            "UpdateProcThreadAttribute",
            "DeleteProcThreadAttributeList",
        ]
        .iter()
        .map(|s| (s.to_string(), "kernel32".to_string()))
        .collect()
    }

    fn calls(ins: &[CodeInstruction], target: &str) -> usize {
        ins.iter()
            .filter(|i| i.op == CodeOp::BranchLink && i.get("target").as_deref() == Some(target))
            .count()
    }

    fn has_immediate(ins: &[CodeInstruction], value: &str) -> bool {
        ins.iter().any(|i| i.get("value").as_deref() == Some(value))
    }

    /// Every Windows child-creating helper (`spawn`, `spawnEnv`, `shell`) lowered
    /// with the real Win64 platform.
    fn lowered_tails() -> Vec<(&'static str, Vec<CodeInstruction>)> {
        let platform = crate::target::win_x86_64::code::Platform;
        mir::set_backend(platform.backend());
        let imports = kernel32_imports();
        let mut out = Vec::new();
        for call in ["process.spawn", "process.spawnEnv"] {
            let (ins, _, _) = lower_process_spawn_helper_win(call, "#t_spawn", &imports, &platform)
                .unwrap_or_else(|e| panic!("{call} lowers on Windows: {e}"));
            out.push((call, ins));
        }
        let (ins, _, _) =
            lower_process_shell_helper_win("process.shell", "#t_shell", &imports, &platform)
                .expect("process.shell lowers on Windows");
        out.push(("process.shell", ins));
        out
    }

    /// bug-499: a Windows child must receive ONLY its three stdio handles. The
    /// tail hands `CreateProcessA` an explicit `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`
    /// (0x20002) through a `STARTUPINFOEXA` (`EXTENDED_STARTUPINFO_PRESENT` =
    /// 0x80000 in `dwCreationFlags`), so an unrelated inheritable handle the
    /// parent holds — every Winsock socket is one by default — never crosses.
    /// Before the fix the tail passed `bInheritHandles = TRUE` with no list.
    #[test]
    fn spawn_tail_limits_inheritance_to_the_stdio_handle_list() {
        for (call, ins) in lowered_tails() {
            assert_eq!(
                calls(&ins, "InitializeProcThreadAttributeList"),
                2,
                "{call}: size query + initialization of the attribute list"
            );
            assert_eq!(
                calls(&ins, "UpdateProcThreadAttribute"),
                1,
                "{call}: the handle list is installed once"
            );
            assert!(
                has_immediate(&ins, "131074"),
                "{call}: PROC_THREAD_ATTRIBUTE_HANDLE_LIST (0x20002) is the attribute installed"
            );
            assert!(
                has_immediate(&ins, "524288"),
                "{call}: EXTENDED_STARTUPINFO_PRESENT (0x80000) tells CreateProcessA to read the EX struct"
            );
            assert!(
                calls(&ins, "DeleteProcThreadAttributeList") >= 2,
                "{call}: the list is deleted on the success path AND the failure path"
            );
            assert_eq!(
                calls(&ins, "CreateProcessA"),
                1,
                "{call}: one CreateProcessA"
            );
        }
    }

    /// The pre-existing contract this fix must not disturb: the three child ends
    /// are still created inheritable (one SECURITY_ATTRIBUTES with
    /// bInheritHandle = TRUE per pipe) and the three parent ends are still
    /// stripped of inheritance, so a child cannot hold its own pipes open.
    #[test]
    fn spawn_tail_still_pipes_the_intended_stdio() {
        for (call, ins) in lowered_tails() {
            assert_eq!(calls(&ins, "CreatePipe"), 3, "{call}: three stdio pipes");
            assert_eq!(
                calls(&ins, "SetHandleInformation"),
                3,
                "{call}: the three parent-held ends are stripped of HANDLE_FLAG_INHERIT"
            );
        }
    }
}
