//! Windows (`win_x86_64`) native backend for the `process` package (plan-90-D).
//!
//! Reimplements the `process` surface over Win32 — `CreateProcessA` +
//! `WaitForSingleObject`/`GetExitCodeProcess` + `TerminateProcess` — sharing the
//! tag-10 record and 96-byte envelope. The handle word (`RESOURCE_OFFSET_HANDLE`)
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
use std::collections::HashMap;
// ---------------------------------------------------------------------------
// The shared Windows spawn tail (plan-119-A)
//
// The Win64 twin of `gen_unix.rs`'s `emit_spawn_tail`. Every Windows helper that
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

// --- the command-line builder's own scratch, immediately above the tail's ---

/// The `List OF String` argv the builder walks. **Caller input.**
pub(crate) const WIN_CMD_LIST: usize = WIN_SPAWN_SCRATCH;
/// Element count of that list.
const WIN_CMD_N: usize = WIN_SPAWN_SCRATCH + 0x08;
/// Base of the list's string data region.
const WIN_CMD_DBASE: usize = WIN_SPAWN_SCRATCH + 0x10;
/// Running worst-case byte length while sizing the buffer.
const WIN_CMD_LEN: usize = WIN_SPAWN_SCRATCH + 0x18;
/// Write cursor into the allocated command line.
const WIN_CMD_DP: usize = WIN_SPAWN_SCRATCH + 0x20;
/// Outer argv index.
const WIN_CMD_IDX: usize = WIN_SPAWN_SCRATCH + 0x28;
/// Byte length of the argument currently being copied.
const WIN_CMD_VLEN: usize = WIN_SPAWN_SCRATCH + 0x30;
/// Read cursor inside the argument currently being quoted.
const WIN_CMD_SRCP: usize = WIN_SPAWN_SCRATCH + 0x38;
/// Bytes of that argument still unread.
const WIN_CMD_REM: usize = WIN_SPAWN_SCRATCH + 0x40;
/// Length of the backslash run pending output (see `emit_win_build_cmdline`).
const WIN_CMD_BS: usize = WIN_SPAWN_SCRATCH + 0x48;
/// First offset a caller may use once both the tail and the command-line builder
/// have taken their slots.
pub(crate) const WIN_CMDLINE_SCRATCH_END: usize = WIN_SPAWN_SCRATCH + 0x50;

/// Build a child's `lpCommandLine` from a `List OF String` argv and leave the
/// NUL-terminated buffer's address in `WIN_SPAWN_CMD`.
///
/// The caller stores the list pointer into `WIN_CMD_LIST` first; an empty list
/// branches to `invalid`, an arena failure to `alloc_fail`. `tag` disambiguates
/// the emitted labels so one helper can build more than one command line.
///
/// # Quoting
///
/// Windows hands a child ONE string and lets the child split it again, so the
/// joiner has to encode the argument boundaries the caller asked for. Joining
/// with bare spaces — what shipped until plan-119-A — silently violates
/// `process::spawn`'s documented "no splitting" contract, measured on box 2230:
/// `spawn(["argdump.exe", "a b", "c"])` reached the child as `argc=3`,
/// `arg=[a]`, `arg=[b]`, and `["argdump.exe", "q\"uote", "plain"]` collapsed to
/// a single `arg=[quote plain]`.
///
/// Each argument is therefore emitted as the inverse of the rule the child's CRT
/// (equivalently `CommandLineToArgvW`) applies when it splits the line:
///
/// - An argument that is non-empty and holds no space, tab or `"` is copied
///   through unchanged. That is what keeps `cmd.exe /C …` working — the program
///   and the switch stay bare, and only the command line itself gets wrapped.
/// - Otherwise it is wrapped in `"`. Inside the wrap a run of backslashes is
///   literal *unless* it precedes a `"` or the closing wrap quote, in which case
///   every backslash in the run is doubled; an embedded `"` is written `\"`.
///
/// The size pass allocates the worst case (`2 + 2*len` per argument) instead of
/// scanning twice: doubling every byte and adding both wrap quotes is the upper
/// bound the emit pass can reach, and one slightly larger arena block is cheaper
/// than a second full walk of the argv.
///
/// Same depth-1, no-vreg discipline as `emit_win_spawn_tail` — see its comment.
pub(crate) fn emit_win_build_cmdline(
    symbol: &str,
    tag: &str,
    invalid: &str,
    alloc_fail: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    const LIST_COUNT: usize = COLLECTION_OFFSET_COUNT;
    const LIST_CAP: usize = COLLECTION_OFFSET_CAPACITY;
    const HDR: usize = COLLECTION_HEADER_SIZE;
    const ENT: usize = COLLECTION_ENTRY_SIZE;
    const VOFF: usize = COLLECTION_ENTRY_OFFSET_VALUE_OFFSET;
    const VLEN: usize = COLLECTION_ENTRY_OFFSET_VALUE_LENGTH;
    const SPACE: &str = "32";
    const TAB: &str = "9";
    const QUOTE: &str = "34";
    const BACKSLASH: &str = "92";
    let sp = abi::stack_pointer();
    let sum_loop = format!("{symbol}_{tag}_sum_loop");
    let sum_done = format!("{symbol}_{tag}_sum_done");
    let copy_loop = format!("{symbol}_{tag}_copy_loop");
    let copy_done = format!("{symbol}_{tag}_copy_done");
    let no_space = format!("{symbol}_{tag}_no_space");
    let scan_loop = format!("{symbol}_{tag}_scan_loop");
    let raw_copy = format!("{symbol}_{tag}_raw_copy");
    let raw_loop = format!("{symbol}_{tag}_raw_loop");
    let need_quote = format!("{symbol}_{tag}_need_quote");
    let run_reset = format!("{symbol}_{tag}_run_reset");
    let run_loop = format!("{symbol}_{tag}_run_loop");
    let plain_byte = format!("{symbol}_{tag}_plain_byte");
    let inner_quote = format!("{symbol}_{tag}_inner_quote");
    let close_quote = format!("{symbol}_{tag}_close_quote");
    let arg_done = format!("{symbol}_{tag}_arg_done");

    // Emit `[WIN_CMD_BS]` backslashes, draining the slot. Three sites need it and
    // a label cannot be called, so it is inlined at each under its own labels.
    let flush_backslashes = |site: &str, out: &mut Vec<CodeInstruction>| {
        let loop_l = format!("{symbol}_{tag}_{site}_bs");
        let done_l = format!("{symbol}_{tag}_{site}_bs_done");
        out.extend([
            abi::label(&loop_l),
            abi::load_u64(abi::mfb_arg(2), sp, WIN_CMD_BS),
            abi::compare_immediate(abi::mfb_arg(2), "0"),
            abi::branch_eq(&done_l),
            abi::subtract_immediate(abi::mfb_arg(2), abi::mfb_arg(2), 1),
            abi::store_u64(abi::mfb_arg(2), sp, WIN_CMD_BS),
            abi::load_u64(abi::mfb_arg(1), sp, WIN_CMD_DP),
            abi::move_immediate(abi::mfb_arg(3), "Integer", BACKSLASH),
            abi::store_u8(abi::mfb_arg(3), abi::mfb_arg(1), 0),
            abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(1), 1),
            abi::store_u64(abi::mfb_arg(1), sp, WIN_CMD_DP),
            abi::branch(&loop_l),
            abi::label(&done_l),
        ]);
    };
    // Append the single byte `imm` at `[WIN_CMD_DP]` and advance the cursor.
    let emit_byte = |imm: &str, out: &mut Vec<CodeInstruction>| {
        out.extend([
            abi::load_u64(abi::mfb_arg(1), sp, WIN_CMD_DP),
            abi::move_immediate(abi::mfb_arg(2), "Integer", imm),
            abi::store_u8(abi::mfb_arg(2), abi::mfb_arg(1), 0),
            abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(1), 1),
            abi::store_u64(abi::mfb_arg(1), sp, WIN_CMD_DP),
        ]);
    };

    instructions.extend([
        abi::load_u64(abi::mfb_arg(0), sp, WIN_CMD_LIST),
        abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), LIST_COUNT),
        abi::compare_immediate(abi::mfb_arg(1), "0"),
        abi::branch_eq(invalid),
        abi::store_u64(abi::mfb_arg(1), sp, WIN_CMD_N),
        // dbase = list + cap*ENT + HDR
        abi::load_u64(abi::mfb_arg(2), abi::mfb_arg(0), LIST_CAP),
        abi::move_immediate(abi::mfb_arg(3), "Integer", &ENT.to_string()),
        abi::multiply_registers(abi::mfb_arg(2), abi::mfb_arg(2), abi::mfb_arg(3)),
        abi::add_immediate(abi::mfb_arg(2), abi::mfb_arg(2), HDR),
        abi::add_registers(abi::mfb_arg(2), abi::mfb_arg(0), abi::mfb_arg(2)),
        abi::store_u64(abi::mfb_arg(2), sp, WIN_CMD_DBASE),
        // Worst-case length = n (separators + NUL) + sum(2*vlen + 2).
        abi::store_u64(abi::mfb_arg(1), sp, WIN_CMD_LEN),
        abi::store_u64(abi::ZERO, sp, WIN_CMD_IDX),
        abi::label(&sum_loop),
        abi::load_u64(abi::mfb_arg(0), sp, WIN_CMD_IDX),
        abi::load_u64(abi::mfb_arg(1), sp, WIN_CMD_N),
        abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(1)),
        abi::branch_eq(&sum_done),
        // entry = list + idx*ENT + HDR
        abi::load_u64(abi::mfb_arg(2), sp, WIN_CMD_LIST),
        abi::move_immediate(abi::mfb_arg(3), "Integer", &ENT.to_string()),
        abi::multiply_registers(abi::mfb_arg(1), abi::mfb_arg(0), abi::mfb_arg(3)),
        abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(1), HDR),
        abi::add_registers(abi::mfb_arg(1), abi::mfb_arg(2), abi::mfb_arg(1)),
        abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(1), VLEN),
        abi::add_registers(abi::mfb_arg(1), abi::mfb_arg(1), abi::mfb_arg(1)), // 2*vlen
        abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(1), 2),               // + both wrap quotes
        abi::load_u64(abi::mfb_arg(2), sp, WIN_CMD_LEN),
        abi::add_registers(abi::mfb_arg(2), abi::mfb_arg(2), abi::mfb_arg(1)),
        abi::store_u64(abi::mfb_arg(2), sp, WIN_CMD_LEN),
        abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1),
        abi::store_u64(abi::mfb_arg(0), sp, WIN_CMD_IDX),
        abi::branch(&sum_loop),
        abi::label(&sum_done),
        // cmd = arena_alloc(len + 1, align 1)
        abi::load_u64(abi::return_register(), sp, WIN_CMD_LEN),
        abi::add_immediate(abi::return_register(), abi::return_register(), 1),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "1"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.extend([
        abi::store_u64(abi::mfb_return(1), sp, WIN_SPAWN_CMD),
        abi::store_u64(abi::mfb_return(1), sp, WIN_CMD_DP),
        abi::store_u64(abi::ZERO, sp, WIN_CMD_IDX),
        abi::label(&copy_loop),
        abi::load_u64(abi::mfb_arg(0), sp, WIN_CMD_IDX),
        abi::load_u64(abi::mfb_arg(1), sp, WIN_CMD_N),
        abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(1)),
        abi::branch_eq(&copy_done),
        // separator space before every argument but the first
        abi::compare_immediate(abi::mfb_arg(0), "0"),
        abi::branch_eq(&no_space),
    ]);
    emit_byte(SPACE, instructions);
    instructions.extend([
        abi::label(&no_space),
        // entry = list + idx*ENT + HDR
        abi::load_u64(abi::mfb_arg(2), sp, WIN_CMD_LIST),
        abi::load_u64(abi::mfb_arg(0), sp, WIN_CMD_IDX),
        abi::move_immediate(abi::mfb_arg(3), "Integer", &ENT.to_string()),
        abi::multiply_registers(abi::mfb_arg(1), abi::mfb_arg(0), abi::mfb_arg(3)),
        abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(1), HDR),
        abi::add_registers(abi::mfb_arg(1), abi::mfb_arg(2), abi::mfb_arg(1)),
        // vlen -> VLEN slot, srcp = dbase + voff -> SRCP slot
        abi::load_u64(abi::mfb_arg(2), abi::mfb_arg(1), VLEN),
        abi::store_u64(abi::mfb_arg(2), sp, WIN_CMD_VLEN),
        abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(1), VOFF),
        abi::load_u64(abi::mfb_arg(1), sp, WIN_CMD_DBASE),
        abi::add_registers(abi::mfb_arg(0), abi::mfb_arg(1), abi::mfb_arg(0)),
        abi::store_u64(abi::mfb_arg(0), sp, WIN_CMD_SRCP),
        // --- does this argument need wrapping? empty, or space/tab/quote inside
        abi::compare_immediate(abi::mfb_arg(2), "0"),
        abi::branch_eq(&need_quote),
        abi::move_immediate(abi::mfb_arg(3), "Integer", "0"), // j
        abi::label(&scan_loop),
        abi::load_u64(abi::mfb_arg(2), sp, WIN_CMD_VLEN),
        abi::compare_registers(abi::mfb_arg(3), abi::mfb_arg(2)),
        abi::branch_eq(&raw_copy),
        abi::load_u8(abi::mfb_arg(2), abi::mfb_arg(0), 0),
        abi::compare_immediate(abi::mfb_arg(2), SPACE),
        abi::branch_eq(&need_quote),
        abi::compare_immediate(abi::mfb_arg(2), TAB),
        abi::branch_eq(&need_quote),
        abi::compare_immediate(abi::mfb_arg(2), QUOTE),
        abi::branch_eq(&need_quote),
        abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1),
        abi::add_immediate(abi::mfb_arg(3), abi::mfb_arg(3), 1),
        abi::branch(&scan_loop),
        // --- pass-through copy ---
        abi::label(&raw_copy),
        abi::load_u64(abi::mfb_arg(0), sp, WIN_CMD_SRCP),
        abi::load_u64(abi::mfb_arg(1), sp, WIN_CMD_DP),
        abi::move_immediate(abi::mfb_arg(3), "Integer", "0"),
        abi::label(&raw_loop),
        abi::load_u64(abi::mfb_arg(2), sp, WIN_CMD_VLEN),
        abi::compare_registers(abi::mfb_arg(3), abi::mfb_arg(2)),
        abi::branch_eq(&arg_done),
        abi::load_u8(abi::mfb_arg(2), abi::mfb_arg(0), 0),
        abi::store_u8(abi::mfb_arg(2), abi::mfb_arg(1), 0),
        abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1),
        abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(1), 1),
        abi::store_u64(abi::mfb_arg(1), sp, WIN_CMD_DP),
        abi::add_immediate(abi::mfb_arg(3), abi::mfb_arg(3), 1),
        abi::branch(&raw_loop),
        // --- wrapped copy ---
        abi::label(&need_quote),
    ]);
    emit_byte(QUOTE, instructions);
    instructions.extend([
        abi::load_u64(abi::mfb_arg(2), sp, WIN_CMD_VLEN),
        abi::store_u64(abi::mfb_arg(2), sp, WIN_CMD_REM),
        // Each pass starts a fresh backslash run, counts it, and then decides what
        // the byte that ENDED the run means for it.
        abi::label(&run_reset),
        abi::store_u64(abi::ZERO, sp, WIN_CMD_BS),
        abi::label(&run_loop),
        abi::load_u64(abi::mfb_arg(2), sp, WIN_CMD_REM),
        abi::compare_immediate(abi::mfb_arg(2), "0"),
        abi::branch_eq(&close_quote),
        abi::load_u64(abi::mfb_arg(0), sp, WIN_CMD_SRCP),
        abi::load_u8(abi::mfb_arg(2), abi::mfb_arg(0), 0),
        abi::compare_immediate(abi::mfb_arg(2), BACKSLASH),
        abi::branch_ne(&plain_byte),
        abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1),
        abi::store_u64(abi::mfb_arg(0), sp, WIN_CMD_SRCP),
        abi::load_u64(abi::mfb_arg(2), sp, WIN_CMD_REM),
        abi::subtract_immediate(abi::mfb_arg(2), abi::mfb_arg(2), 1),
        abi::store_u64(abi::mfb_arg(2), sp, WIN_CMD_REM),
        abi::load_u64(abi::mfb_arg(2), sp, WIN_CMD_BS),
        abi::add_immediate(abi::mfb_arg(2), abi::mfb_arg(2), 1),
        abi::store_u64(abi::mfb_arg(2), sp, WIN_CMD_BS),
        abi::branch(&run_loop),
        // a non-backslash byte ended the run (mfb_arg(2) holds it)
        abi::label(&plain_byte),
        abi::compare_immediate(abi::mfb_arg(2), QUOTE),
        abi::branch_eq(&inner_quote),
    ]);
    // an ordinary byte: the run stays literal, then the byte itself
    flush_backslashes("plain", instructions);
    instructions.extend([
        abi::load_u64(abi::mfb_arg(0), sp, WIN_CMD_SRCP),
        abi::load_u8(abi::mfb_arg(2), abi::mfb_arg(0), 0),
        abi::load_u64(abi::mfb_arg(1), sp, WIN_CMD_DP),
        abi::store_u8(abi::mfb_arg(2), abi::mfb_arg(1), 0),
        abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(1), 1),
        abi::store_u64(abi::mfb_arg(1), sp, WIN_CMD_DP),
        abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1),
        abi::store_u64(abi::mfb_arg(0), sp, WIN_CMD_SRCP),
        abi::load_u64(abi::mfb_arg(2), sp, WIN_CMD_REM),
        abi::subtract_immediate(abi::mfb_arg(2), abi::mfb_arg(2), 1),
        abi::store_u64(abi::mfb_arg(2), sp, WIN_CMD_REM),
        abi::branch(&run_reset),
        // an embedded quote: the run doubles, then the quote is escaped
        abi::label(&inner_quote),
        abi::load_u64(abi::mfb_arg(2), sp, WIN_CMD_BS),
        abi::add_registers(abi::mfb_arg(2), abi::mfb_arg(2), abi::mfb_arg(2)),
        abi::store_u64(abi::mfb_arg(2), sp, WIN_CMD_BS),
    ]);
    flush_backslashes("quote", instructions);
    emit_byte(BACKSLASH, instructions);
    emit_byte(QUOTE, instructions);
    instructions.extend([
        abi::load_u64(abi::mfb_arg(0), sp, WIN_CMD_SRCP),
        abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1),
        abi::store_u64(abi::mfb_arg(0), sp, WIN_CMD_SRCP),
        abi::load_u64(abi::mfb_arg(2), sp, WIN_CMD_REM),
        abi::subtract_immediate(abi::mfb_arg(2), abi::mfb_arg(2), 1),
        abi::store_u64(abi::mfb_arg(2), sp, WIN_CMD_REM),
        abi::branch(&run_reset),
        // end of the argument: the pending run precedes the closing wrap quote,
        // so it doubles too
        abi::label(&close_quote),
        abi::load_u64(abi::mfb_arg(2), sp, WIN_CMD_BS),
        abi::add_registers(abi::mfb_arg(2), abi::mfb_arg(2), abi::mfb_arg(2)),
        abi::store_u64(abi::mfb_arg(2), sp, WIN_CMD_BS),
    ]);
    flush_backslashes("close", instructions);
    emit_byte(QUOTE, instructions);
    instructions.extend([
        abi::label(&arg_done),
        abi::load_u64(abi::mfb_arg(0), sp, WIN_CMD_IDX),
        abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1),
        abi::store_u64(abi::mfb_arg(0), sp, WIN_CMD_IDX),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::load_u64(abi::mfb_arg(1), sp, WIN_CMD_DP),
        abi::store_u8(abi::ZERO, abi::mfb_arg(1), 0), // NUL-terminate
    ]);
}

// --- spawnEnv's cwd + environment-block scratch, above the builder's ---
//
// plan-119-C. The four-argument `process::spawn` adds a working directory and an
// environment `Map OF String TO String` with a replace-vs-merge flag. On Unix
// those are applied *in the fork child* (`chdir` + `unsetenv`/`setenv` loops);
// `CreateProcessA` instead takes both as pointers it reads before the child
// exists, so Windows has to MATERIALIZE the result — one NUL-terminated path and
// one `name=value\0…\0\0` ANSI block — and the merge semantics are reproduced by
// *building* that block rather than by mutating anything.

/// **Caller input**: the cwd `String` object (length@0, bytes@8). An empty
/// string means "inherit", and becomes a NULL `lpCurrentDirectory`.
pub(crate) const WIN_ENV_CWDSTR: usize = WIN_CMDLINE_SCRATCH_END;
/// Its byte length.
const WIN_ENV_CWDLEN: usize = WIN_CMDLINE_SCRATCH_END + 0x08;
/// **Caller input**: the environment `Map OF String TO String`.
pub(crate) const WIN_ENV_MAP: usize = WIN_CMDLINE_SCRATCH_END + 0x10;
/// **Caller input**: the `envReplace` flag — nonzero means "only the map".
pub(crate) const WIN_ENV_REPLACE: usize = WIN_CMDLINE_SCRATCH_END + 0x18;
/// Running byte length of the block while sizing it.
const WIN_ENV_LEN: usize = WIN_CMDLINE_SCRATCH_END + 0x20;
/// Write cursor into the block.
const WIN_ENV_DP: usize = WIN_CMDLINE_SCRATCH_END + 0x28;
/// Map-entry index for the size and append walks.
const WIN_ENV_IDX: usize = WIN_CMDLINE_SCRATCH_END + 0x30;
/// The map's capacity — a Map is walked `0..capacity`, skipping unused slots.
const WIN_ENV_CAP: usize = WIN_CMDLINE_SCRATCH_END + 0x38;
/// Base of the map's string data region.
const WIN_ENV_DBASE: usize = WIN_CMDLINE_SCRATCH_END + 0x40;
/// Byte length of the key or value currently being copied.
const WIN_ENV_TMP: usize = WIN_CMDLINE_SCRATCH_END + 0x48;
/// Base of the inherited environment block (merge mode); 0 when replacing, which
/// is also what says "nothing to free".
const WIN_ENV_INHB: usize = WIN_CMDLINE_SCRATCH_END + 0x50;
/// Cursor into the inherited block.
const WIN_ENV_INHP: usize = WIN_CMDLINE_SCRATCH_END + 0x58;
/// Name length of the inherited entry at the cursor (bytes before its `=`).
const WIN_ENV_NLEN: usize = WIN_CMDLINE_SCRATCH_END + 0x60;
/// Total length of that entry (bytes before its NUL).
const WIN_ENV_ELEN: usize = WIN_CMDLINE_SCRATCH_END + 0x68;
/// Map-entry index for the override scan.
const WIN_ENV_MIDX: usize = WIN_CMDLINE_SCRATCH_END + 0x70;
/// Result of that scan: nonzero when the map overrides this inherited name.
const WIN_ENV_MATCH: usize = WIN_CMDLINE_SCRATCH_END + 0x78;
/// Map-key pointer during a name comparison.
const WIN_ENV_KP: usize = WIN_CMDLINE_SCRATCH_END + 0x80;
/// Inherited-name pointer during a name comparison.
const WIN_ENV_IP: usize = WIN_CMDLINE_SCRATCH_END + 0x88;
/// Byte counter, reused by every inner walk.
const WIN_ENV_CNT: usize = WIN_CMDLINE_SCRATCH_END + 0x90;
/// First offset above everything `spawnEnv` needs.
pub(crate) const WIN_ENV_SCRATCH_END: usize = WIN_CMDLINE_SCRATCH_END + 0x98;

/// Build `lpCurrentDirectory` from the cwd `String` in `WIN_ENV_CWDSTR` into
/// `WIN_SPAWN_CWD`, or store 0 there when the string is empty.
///
/// The Windows counterpart of the posix body's leading-NUL sentinel: Unix builds
/// a C string whose first byte is NUL so the fork child skips its `chdir`, while
/// here "inherit" is expressed directly as the NULL `CreateProcessA` accepts.
pub(crate) fn emit_win_build_cwd(
    symbol: &str,
    alloc_fail: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let sp = abi::stack_pointer();
    let inherit = format!("{symbol}_cwd_inherit");
    let copy = format!("{symbol}_cwd_copy");
    let copy_done = format!("{symbol}_cwd_copy_done");
    let done = format!("{symbol}_cwd_done");
    instructions.extend([
        abi::load_u64(abi::mfb_arg(0), sp, WIN_ENV_CWDSTR),
        abi::load_u64(abi::mfb_arg(2), abi::mfb_arg(0), 0),
        abi::store_u64(abi::mfb_arg(2), sp, WIN_ENV_CWDLEN),
        abi::compare_immediate(abi::mfb_arg(2), "0"),
        abi::branch_eq(&inherit),
        abi::add_immediate(abi::return_register(), abi::mfb_arg(2), 1),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "1"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.extend([
        abi::store_u64(abi::mfb_return(1), sp, WIN_SPAWN_CWD),
        abi::move_register(abi::mfb_arg(1), abi::mfb_return(1)),
        abi::load_u64(abi::mfb_arg(0), sp, WIN_ENV_CWDSTR),
        abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 8),
        abi::move_immediate(abi::mfb_arg(3), "Integer", "0"),
        abi::label(&copy),
        abi::load_u64(abi::mfb_arg(2), sp, WIN_ENV_CWDLEN),
        abi::compare_registers(abi::mfb_arg(3), abi::mfb_arg(2)),
        abi::branch_eq(&copy_done),
        abi::load_u8(abi::mfb_arg(2), abi::mfb_arg(0), 0),
        abi::store_u8(abi::mfb_arg(2), abi::mfb_arg(1), 0),
        abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1),
        abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(1), 1),
        abi::add_immediate(abi::mfb_arg(3), abi::mfb_arg(3), 1),
        abi::branch(&copy),
        abi::label(&copy_done),
        abi::store_u8(abi::ZERO, abi::mfb_arg(1), 0),
        abi::branch(&done),
        abi::label(&inherit),
        abi::store_u64(abi::ZERO, sp, WIN_SPAWN_CWD),
        abi::label(&done),
    ]);
}

/// Build `lpEnvironment` from the map in `WIN_ENV_MAP` and the flag in
/// `WIN_ENV_REPLACE` into `WIN_SPAWN_ENV`.
///
/// The block is `name=value\0…\0\0`, ANSI (no `CREATE_UNICODE_ENVIRONMENT`), and
/// `CreateProcess` does not require it sorted. Two passes over the same walks —
/// size, then copy — so exactly one arena block is allocated.
///
/// **Replace** (`envReplace` nonzero) is a single flat walk of the map. **Merge**
/// additionally walks the inherited block from `GetEnvironmentStringsA` and keeps
/// every entry the map does not override.
///
/// The override test is **case-insensitive**, ASCII-folded. It has to be: Windows
/// environment names are case-insensitive, so a byte-exact compare against a map
/// key `PATH` would let an inherited `Path` through and hand the child *both*
/// — after which which one wins is the child's business, not the caller's. Names
/// outside ASCII are outside the documented contract and fold to themselves.
///
/// A failure to allocate branches to `alloc_fail`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_win_build_env_block(
    symbol: &str,
    alloc_fail: &str,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    const HDR: usize = COLLECTION_HEADER_SIZE;
    const ENT: usize = COLLECTION_ENTRY_SIZE;
    const FLAGS: usize = COLLECTION_ENTRY_OFFSET_FLAGS;
    const KOFF: usize = COLLECTION_ENTRY_OFFSET_KEY_OFFSET;
    const KLEN: usize = COLLECTION_ENTRY_OFFSET_KEY_LENGTH;
    const VOFF: usize = COLLECTION_ENTRY_OFFSET_VALUE_OFFSET;
    const VLEN: usize = COLLECTION_ENTRY_OFFSET_VALUE_LENGTH;
    const EQUALS: &str = "61";
    let sp = abi::stack_pointer();
    let used = COLLECTION_ENTRY_FLAG_USED.to_string();
    let ent = ENT.to_string();

    // entry = map + HDR + [index_slot]*ENT, left in mfb_arg(2). Clobbers 0..3.
    let entry_at = |index_slot: usize, out: &mut Vec<CodeInstruction>| {
        out.extend([
            abi::load_u64(abi::mfb_arg(0), sp, index_slot),
            abi::load_u64(abi::mfb_arg(1), sp, WIN_ENV_MAP),
            abi::move_immediate(abi::mfb_arg(3), "Integer", &ent),
            abi::multiply_registers(abi::mfb_arg(2), abi::mfb_arg(0), abi::mfb_arg(3)),
            abi::add_immediate(abi::mfb_arg(2), abi::mfb_arg(2), HDR),
            abi::add_registers(abi::mfb_arg(2), abi::mfb_arg(1), abi::mfb_arg(2)),
        ]);
    };
    // Branch to `skip` unless the entry in mfb_arg(2) is USED. A Map's entry array
    // is capacity-sized and sparse, so this is not the same walk a List gets.
    let skip_unless_used = |skip: &str, out: &mut Vec<CodeInstruction>| {
        out.extend([
            abi::load_u8(abi::mfb_arg(3), abi::mfb_arg(2), FLAGS),
            abi::move_immediate(abi::mfb_arg(1), "Integer", &used),
            abi::and_registers(abi::mfb_arg(3), abi::mfb_arg(3), abi::mfb_arg(1)),
            abi::compare_immediate(abi::mfb_arg(3), "0"),
            abi::branch_eq(skip),
        ]);
    };
    // ASCII uppercase fold, in place, on one register.
    let fold = |reg: usize, site: &str, out: &mut Vec<CodeInstruction>| {
        let skip = format!("{symbol}_env_fold_{site}");
        out.extend([
            abi::compare_immediate(abi::mfb_arg(reg), "97"), // 'a'
            abi::branch_lt(&skip),
            abi::compare_immediate(abi::mfb_arg(reg), "122"), // 'z'
            abi::branch_gt(&skip),
            abi::subtract_immediate(abi::mfb_arg(reg), abi::mfb_arg(reg), 32),
            abi::label(&skip),
        ]);
    };
    // Measure the inherited entry at [WIN_ENV_INHP]: total bytes before its NUL
    // into WIN_ENV_ELEN, bytes before its first `=` into WIN_ENV_NLEN. An entry
    // with no `=` (and Windows really does have them — the `=C:` drive-cwd
    // pseudo-variables start with one) measures NLEN == ELEN, which matches no
    // map key and is therefore passed through.
    let measure = |site: &str, out: &mut Vec<CodeInstruction>| {
        let len_loop = format!("{symbol}_env_{site}_len");
        let len_done = format!("{symbol}_env_{site}_len_done");
        let nlen_loop = format!("{symbol}_env_{site}_nlen");
        let nlen_done = format!("{symbol}_env_{site}_nlen_done");
        out.extend([
            abi::store_u64(abi::ZERO, sp, WIN_ENV_CNT),
            abi::label(&len_loop),
            abi::load_u64(abi::mfb_arg(3), sp, WIN_ENV_CNT),
            abi::load_u64(abi::mfb_arg(2), sp, WIN_ENV_INHP),
            abi::add_registers(abi::mfb_arg(2), abi::mfb_arg(2), abi::mfb_arg(3)),
            abi::load_u8(abi::mfb_arg(0), abi::mfb_arg(2), 0),
            abi::compare_immediate(abi::mfb_arg(0), "0"),
            abi::branch_eq(&len_done),
            abi::add_immediate(abi::mfb_arg(3), abi::mfb_arg(3), 1),
            abi::store_u64(abi::mfb_arg(3), sp, WIN_ENV_CNT),
            abi::branch(&len_loop),
            abi::label(&len_done),
            abi::store_u64(abi::mfb_arg(3), sp, WIN_ENV_ELEN),
            abi::store_u64(abi::ZERO, sp, WIN_ENV_CNT),
            abi::label(&nlen_loop),
            abi::load_u64(abi::mfb_arg(3), sp, WIN_ENV_CNT),
            abi::load_u64(abi::mfb_arg(2), sp, WIN_ENV_ELEN),
            abi::compare_registers(abi::mfb_arg(3), abi::mfb_arg(2)),
            abi::branch_eq(&nlen_done),
            abi::load_u64(abi::mfb_arg(2), sp, WIN_ENV_INHP),
            abi::add_registers(abi::mfb_arg(2), abi::mfb_arg(2), abi::mfb_arg(3)),
            abi::load_u8(abi::mfb_arg(0), abi::mfb_arg(2), 0),
            abi::compare_immediate(abi::mfb_arg(0), EQUALS),
            abi::branch_eq(&nlen_done),
            abi::add_immediate(abi::mfb_arg(3), abi::mfb_arg(3), 1),
            abi::store_u64(abi::mfb_arg(3), sp, WIN_ENV_CNT),
            abi::branch(&nlen_loop),
            abi::label(&nlen_done),
            abi::store_u64(abi::mfb_arg(3), sp, WIN_ENV_NLEN),
        ]);
    };
    // WIN_ENV_MATCH = 1 when some USED map key equals the inherited name at
    // [WIN_ENV_INHP] (WIN_ENV_NLEN bytes), compared case-insensitively.
    let match_scan = |site: &str, out: &mut Vec<CodeInstruction>| {
        let loop_l = format!("{symbol}_env_{site}_m");
        let next = format!("{symbol}_env_{site}_m_next");
        let byte = format!("{symbol}_env_{site}_m_byte");
        let hit = format!("{symbol}_env_{site}_m_hit");
        let done = format!("{symbol}_env_{site}_m_done");
        out.extend([
            abi::store_u64(abi::ZERO, sp, WIN_ENV_MATCH),
            abi::store_u64(abi::ZERO, sp, WIN_ENV_MIDX),
            abi::label(&loop_l),
            abi::load_u64(abi::mfb_arg(0), sp, WIN_ENV_MIDX),
            abi::load_u64(abi::mfb_arg(2), sp, WIN_ENV_CAP),
            abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(2)),
            abi::branch_eq(&done),
        ]);
        entry_at(WIN_ENV_MIDX, out);
        skip_unless_used(&next, out);
        out.extend([
            // a different length can never match
            abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(2), KLEN),
            abi::load_u64(abi::mfb_arg(3), sp, WIN_ENV_NLEN),
            abi::compare_registers(abi::mfb_arg(1), abi::mfb_arg(3)),
            abi::branch_ne(&next),
            abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(2), KOFF),
            abi::load_u64(abi::mfb_arg(3), sp, WIN_ENV_DBASE),
            abi::add_registers(abi::mfb_arg(1), abi::mfb_arg(3), abi::mfb_arg(1)),
            abi::store_u64(abi::mfb_arg(1), sp, WIN_ENV_KP),
            abi::load_u64(abi::mfb_arg(1), sp, WIN_ENV_INHP),
            abi::store_u64(abi::mfb_arg(1), sp, WIN_ENV_IP),
            abi::store_u64(abi::ZERO, sp, WIN_ENV_CNT),
            abi::label(&byte),
            abi::load_u64(abi::mfb_arg(3), sp, WIN_ENV_CNT),
            abi::load_u64(abi::mfb_arg(2), sp, WIN_ENV_NLEN),
            abi::compare_registers(abi::mfb_arg(3), abi::mfb_arg(2)),
            abi::branch_eq(&hit),
            abi::load_u64(abi::mfb_arg(2), sp, WIN_ENV_KP),
            abi::add_registers(abi::mfb_arg(2), abi::mfb_arg(2), abi::mfb_arg(3)),
            abi::load_u8(abi::mfb_arg(0), abi::mfb_arg(2), 0),
            abi::load_u64(abi::mfb_arg(2), sp, WIN_ENV_IP),
            abi::add_registers(abi::mfb_arg(2), abi::mfb_arg(2), abi::mfb_arg(3)),
            abi::load_u8(abi::mfb_arg(1), abi::mfb_arg(2), 0),
        ]);
        fold(0, &format!("{site}k"), out);
        fold(1, &format!("{site}n"), out);
        out.extend([
            abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(1)),
            abi::branch_ne(&next),
            abi::load_u64(abi::mfb_arg(3), sp, WIN_ENV_CNT),
            abi::add_immediate(abi::mfb_arg(3), abi::mfb_arg(3), 1),
            abi::store_u64(abi::mfb_arg(3), sp, WIN_ENV_CNT),
            abi::branch(&byte),
            abi::label(&hit),
            abi::move_immediate(abi::mfb_arg(2), "Integer", "1"),
            abi::store_u64(abi::mfb_arg(2), sp, WIN_ENV_MATCH),
            abi::branch(&done),
            abi::label(&next),
            abi::load_u64(abi::mfb_arg(0), sp, WIN_ENV_MIDX),
            abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1),
            abi::store_u64(abi::mfb_arg(0), sp, WIN_ENV_MIDX),
            abi::branch(&loop_l),
            abi::label(&done),
        ]);
    };
    // Add each USED map entry's `key=value\0` length to WIN_ENV_LEN.
    let size_map = |out: &mut Vec<CodeInstruction>| {
        let loop_l = format!("{symbol}_env_msize");
        let next = format!("{symbol}_env_msize_next");
        let done = format!("{symbol}_env_msize_done");
        out.extend([
            abi::store_u64(abi::ZERO, sp, WIN_ENV_IDX),
            abi::label(&loop_l),
            abi::load_u64(abi::mfb_arg(0), sp, WIN_ENV_IDX),
            abi::load_u64(abi::mfb_arg(2), sp, WIN_ENV_CAP),
            abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(2)),
            abi::branch_eq(&done),
        ]);
        entry_at(WIN_ENV_IDX, out);
        skip_unless_used(&next, out);
        out.extend([
            abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(2), KLEN),
            abi::load_u64(abi::mfb_arg(3), abi::mfb_arg(2), VLEN),
            abi::add_registers(abi::mfb_arg(1), abi::mfb_arg(1), abi::mfb_arg(3)),
            abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(1), 2), // '=' and NUL
            abi::load_u64(abi::mfb_arg(3), sp, WIN_ENV_LEN),
            abi::add_registers(abi::mfb_arg(3), abi::mfb_arg(3), abi::mfb_arg(1)),
            abi::store_u64(abi::mfb_arg(3), sp, WIN_ENV_LEN),
            abi::label(&next),
            abi::load_u64(abi::mfb_arg(0), sp, WIN_ENV_IDX),
            abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1),
            abi::store_u64(abi::mfb_arg(0), sp, WIN_ENV_IDX),
            abi::branch(&loop_l),
            abi::label(&done),
        ]);
    };
    // Copy `[WIN_ENV_TMP]` bytes from the map data region at `off_field` into the
    // block at [WIN_ENV_DP], advancing the cursor. The entry must be in mfb_arg(2).
    let copy_field =
        |site: &str, len_field: usize, off_field: usize, out: &mut Vec<CodeInstruction>| {
            let loop_l = format!("{symbol}_env_{site}_cp");
            let done = format!("{symbol}_env_{site}_cp_done");
            out.extend([
                abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(2), len_field),
                abi::store_u64(abi::mfb_arg(1), sp, WIN_ENV_TMP),
                abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(2), off_field),
                abi::load_u64(abi::mfb_arg(1), sp, WIN_ENV_DBASE),
                abi::add_registers(abi::mfb_arg(0), abi::mfb_arg(1), abi::mfb_arg(0)),
                abi::load_u64(abi::mfb_arg(1), sp, WIN_ENV_DP),
                abi::move_immediate(abi::mfb_arg(3), "Integer", "0"),
                abi::label(&loop_l),
                abi::load_u64(abi::mfb_arg(2), sp, WIN_ENV_TMP),
                abi::compare_registers(abi::mfb_arg(3), abi::mfb_arg(2)),
                abi::branch_eq(&done),
                abi::load_u8(abi::mfb_arg(2), abi::mfb_arg(0), 0),
                abi::store_u8(abi::mfb_arg(2), abi::mfb_arg(1), 0),
                abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1),
                abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(1), 1),
                abi::add_immediate(abi::mfb_arg(3), abi::mfb_arg(3), 1),
                abi::branch(&loop_l),
                abi::label(&done),
                abi::store_u64(abi::mfb_arg(1), sp, WIN_ENV_DP),
            ]);
        };

    let skip_inh_size = format!("{symbol}_env_skip_inh_size");
    let isize_loop = format!("{symbol}_env_isize");
    let isize_next = format!("{symbol}_env_isize_next");
    let isize_done = format!("{symbol}_env_isize_done");
    let skip_inh_copy = format!("{symbol}_env_skip_inh_copy");
    let icopy_loop = format!("{symbol}_env_icopy");
    let icopy_next = format!("{symbol}_env_icopy_next");
    let icopy_done = format!("{symbol}_env_icopy_done");
    let ic_byte = format!("{symbol}_env_ic_byte");
    let ic_done = format!("{symbol}_env_ic_done");
    let acopy_loop = format!("{symbol}_env_acopy");
    let acopy_next = format!("{symbol}_env_acopy_next");
    let acopy_done = format!("{symbol}_env_acopy_done");
    let no_free = format!("{symbol}_env_no_free");

    // Map geometry, shared by every walk below.
    instructions.extend([
        abi::load_u64(abi::mfb_arg(0), sp, WIN_ENV_MAP),
        abi::load_u64(abi::mfb_arg(2), abi::mfb_arg(0), COLLECTION_OFFSET_CAPACITY),
        abi::store_u64(abi::mfb_arg(2), sp, WIN_ENV_CAP),
        abi::move_immediate(abi::mfb_arg(3), "Integer", &ent),
        abi::multiply_registers(abi::mfb_arg(2), abi::mfb_arg(2), abi::mfb_arg(3)),
        abi::add_immediate(abi::mfb_arg(2), abi::mfb_arg(2), HDR),
        abi::add_registers(abi::mfb_arg(2), abi::mfb_arg(0), abi::mfb_arg(2)),
        abi::store_u64(abi::mfb_arg(2), sp, WIN_ENV_DBASE),
        // The block always ends with two NULs; an empty map's block is exactly
        // those two, which is a valid "no variables" environment.
        abi::move_immediate(abi::mfb_arg(2), "Integer", "2"),
        abi::store_u64(abi::mfb_arg(2), sp, WIN_ENV_LEN),
        abi::store_u64(abi::ZERO, sp, WIN_ENV_INHB),
        // Merge mode takes the inherited block; replace mode never asks for it.
        abi::load_u64(abi::mfb_arg(2), sp, WIN_ENV_REPLACE),
        abi::compare_immediate(abi::mfb_arg(2), "0"),
        abi::branch_ne(&skip_inh_size),
    ]);
    platform.emit_external_call(
        "GetEnvironmentStringsA",
        symbol,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::store_u64(abi::c_return(0), sp, WIN_ENV_INHB),
        abi::store_u64(abi::c_return(0), sp, WIN_ENV_INHP),
        abi::label(&isize_loop),
        abi::load_u64(abi::mfb_arg(0), sp, WIN_ENV_INHP),
        abi::load_u8(abi::mfb_arg(2), abi::mfb_arg(0), 0),
        abi::compare_immediate(abi::mfb_arg(2), "0"),
        abi::branch_eq(&isize_done),
    ]);
    measure("size", instructions);
    match_scan("size", instructions);
    instructions.extend([
        abi::load_u64(abi::mfb_arg(2), sp, WIN_ENV_MATCH),
        abi::compare_immediate(abi::mfb_arg(2), "0"),
        abi::branch_ne(&isize_next),
        abi::load_u64(abi::mfb_arg(2), sp, WIN_ENV_ELEN),
        abi::add_immediate(abi::mfb_arg(2), abi::mfb_arg(2), 1),
        abi::load_u64(abi::mfb_arg(3), sp, WIN_ENV_LEN),
        abi::add_registers(abi::mfb_arg(3), abi::mfb_arg(3), abi::mfb_arg(2)),
        abi::store_u64(abi::mfb_arg(3), sp, WIN_ENV_LEN),
        abi::label(&isize_next),
        abi::load_u64(abi::mfb_arg(0), sp, WIN_ENV_INHP),
        abi::load_u64(abi::mfb_arg(2), sp, WIN_ENV_ELEN),
        abi::add_registers(abi::mfb_arg(0), abi::mfb_arg(0), abi::mfb_arg(2)),
        abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1),
        abi::store_u64(abi::mfb_arg(0), sp, WIN_ENV_INHP),
        abi::branch(&isize_loop),
        abi::label(&isize_done),
        abi::label(&skip_inh_size),
    ]);
    size_map(instructions);
    instructions.extend([
        abi::load_u64(abi::return_register(), sp, WIN_ENV_LEN),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "1"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.extend([
        abi::store_u64(abi::mfb_return(1), sp, WIN_SPAWN_ENV),
        abi::store_u64(abi::mfb_return(1), sp, WIN_ENV_DP),
        // Second pass over the inherited block, from the same base.
        abi::load_u64(abi::mfb_arg(2), sp, WIN_ENV_INHB),
        abi::compare_immediate(abi::mfb_arg(2), "0"),
        abi::branch_eq(&skip_inh_copy),
        abi::store_u64(abi::mfb_arg(2), sp, WIN_ENV_INHP),
        abi::label(&icopy_loop),
        abi::load_u64(abi::mfb_arg(0), sp, WIN_ENV_INHP),
        abi::load_u8(abi::mfb_arg(2), abi::mfb_arg(0), 0),
        abi::compare_immediate(abi::mfb_arg(2), "0"),
        abi::branch_eq(&icopy_done),
    ]);
    measure("copy", instructions);
    match_scan("copy", instructions);
    instructions.extend([
        abi::load_u64(abi::mfb_arg(2), sp, WIN_ENV_MATCH),
        abi::compare_immediate(abi::mfb_arg(2), "0"),
        abi::branch_ne(&icopy_next),
        // copy ELEN + 1 bytes (the entry and its NUL)
        abi::store_u64(abi::ZERO, sp, WIN_ENV_CNT),
        abi::label(&ic_byte),
        abi::load_u64(abi::mfb_arg(3), sp, WIN_ENV_CNT),
        abi::load_u64(abi::mfb_arg(2), sp, WIN_ENV_ELEN),
        abi::add_immediate(abi::mfb_arg(2), abi::mfb_arg(2), 1),
        abi::compare_registers(abi::mfb_arg(3), abi::mfb_arg(2)),
        abi::branch_eq(&ic_done),
        abi::load_u64(abi::mfb_arg(2), sp, WIN_ENV_INHP),
        abi::add_registers(abi::mfb_arg(2), abi::mfb_arg(2), abi::mfb_arg(3)),
        abi::load_u8(abi::mfb_arg(0), abi::mfb_arg(2), 0),
        abi::load_u64(abi::mfb_arg(1), sp, WIN_ENV_DP),
        abi::store_u8(abi::mfb_arg(0), abi::mfb_arg(1), 0),
        abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(1), 1),
        abi::store_u64(abi::mfb_arg(1), sp, WIN_ENV_DP),
        abi::add_immediate(abi::mfb_arg(3), abi::mfb_arg(3), 1),
        abi::store_u64(abi::mfb_arg(3), sp, WIN_ENV_CNT),
        abi::branch(&ic_byte),
        abi::label(&ic_done),
        abi::label(&icopy_next),
        abi::load_u64(abi::mfb_arg(0), sp, WIN_ENV_INHP),
        abi::load_u64(abi::mfb_arg(2), sp, WIN_ENV_ELEN),
        abi::add_registers(abi::mfb_arg(0), abi::mfb_arg(0), abi::mfb_arg(2)),
        abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1),
        abi::store_u64(abi::mfb_arg(0), sp, WIN_ENV_INHP),
        abi::branch(&icopy_loop),
        abi::label(&icopy_done),
        abi::label(&skip_inh_copy),
        // Then every USED map entry, `key=value\0`.
        abi::store_u64(abi::ZERO, sp, WIN_ENV_IDX),
        abi::label(&acopy_loop),
        abi::load_u64(abi::mfb_arg(0), sp, WIN_ENV_IDX),
        abi::load_u64(abi::mfb_arg(2), sp, WIN_ENV_CAP),
        abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(2)),
        abi::branch_eq(&acopy_done),
    ]);
    entry_at(WIN_ENV_IDX, instructions);
    skip_unless_used(&acopy_next, instructions);
    copy_field("key", KLEN, KOFF, instructions);
    instructions.extend([
        abi::load_u64(abi::mfb_arg(1), sp, WIN_ENV_DP),
        abi::move_immediate(abi::mfb_arg(2), "Integer", EQUALS),
        abi::store_u8(abi::mfb_arg(2), abi::mfb_arg(1), 0),
        abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(1), 1),
        abi::store_u64(abi::mfb_arg(1), sp, WIN_ENV_DP),
    ]);
    entry_at(WIN_ENV_IDX, instructions);
    copy_field("val", VLEN, VOFF, instructions);
    instructions.extend([
        abi::load_u64(abi::mfb_arg(1), sp, WIN_ENV_DP),
        abi::store_u8(abi::ZERO, abi::mfb_arg(1), 0),
        abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(1), 1),
        abi::store_u64(abi::mfb_arg(1), sp, WIN_ENV_DP),
        abi::label(&acopy_next),
        abi::load_u64(abi::mfb_arg(0), sp, WIN_ENV_IDX),
        abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1),
        abi::store_u64(abi::mfb_arg(0), sp, WIN_ENV_IDX),
        abi::branch(&acopy_loop),
        abi::label(&acopy_done),
        // The block terminator.
        abi::load_u64(abi::mfb_arg(1), sp, WIN_ENV_DP),
        abi::store_u8(abi::ZERO, abi::mfb_arg(1), 0),
        abi::store_u8(abi::ZERO, abi::mfb_arg(1), 1),
        abi::load_u64(abi::mfb_arg(2), sp, WIN_ENV_INHB),
        abi::compare_immediate(abi::mfb_arg(2), "0"),
        abi::branch_eq(&no_free),
        abi::move_register(abi::mfb_arg(0), abi::mfb_arg(2)),
    ]);
    platform.emit_external_call(
        "FreeEnvironmentStringsA",
        symbol,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.push(abi::label(&no_free));
    Ok(())
}

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
        abi::move_immediate(abi::mfb_arg(2), "Integer", PROC_THREAD_ATTRIBUTE_HANDLE_LIST),
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

// process.close — close the parent's stdin write handle (signals the child's
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
    sp: &str,
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
        ins.iter()
            .any(|i| i.get("value").as_deref() == Some(value))
    }

    /// Every Windows child-creating helper (`spawn`, `spawnEnv`, `shell`) lowered
    /// with the real Win64 platform.
    fn lowered_tails() -> Vec<(&'static str, Vec<CodeInstruction>)> {
        let platform = crate::target::win_x86_64::code::Platform;
        mir::set_backend(platform.backend());
        let imports = kernel32_imports();
        let mut out = Vec::new();
        for call in ["process.spawn", "process.spawnEnv"] {
            let (ins, _, _) =
                lower_process_spawn_helper_win(call, "#t_spawn", &imports, &platform)
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
            assert_eq!(calls(&ins, "CreateProcessA"), 1, "{call}: one CreateProcessA");
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
