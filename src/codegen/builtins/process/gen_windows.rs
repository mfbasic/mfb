//! Windows (`win_x86_64`) native backend for the `process` package (plan-90-D).
//!
//! Reimplements the `process` surface over Win32 — `CreateProcessA` +
//! `WaitForSingleObject`/`GetExitCodeProcess` + `TerminateProcess` — sharing the
//! tag-10 record and 96-byte envelope. The handle word (`RESOURCE_OFFSET_HANDLE`)
//! holds the process `HANDLE`; the child pid is cached in `PROC_STATUS`,
//! the exit code in `PROC_EXITCODE`. Landed in phases, gated by the `win_x86_64`
//! capability list (a call whose capability is not advertised never reaches its
//! helper, so the `unimplemented_on_windows` arms below are unreachable
//! placeholders, not live stubs).

// --- codegen tier imports (migration) ---
use super::gen_shared::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::error::emission::emit_fail;
use crate::target::shared::abi;
use std::collections::HashMap;
pub(crate) fn unimplemented_on_windows(op: &str) -> Result<ProcBodyParts, String> {
    Err(format!(
        "process::{op} native Windows backend is not yet emitted (plan-90-D)"
    ))
}

// ---------------------------------------------------------------------------
// The shared Windows spawn tail (plan-119-A)
//
// The Win64 twin of `gen_unix.rs`'s `emit_spawn_tail`. Every Windows helper that
// creates a child (`spawn`, `shell`, `spawnEnv`) differs only in how it builds a
// command line, an environment block and a working directory; everything after
// that — three inheritable pipes, a `STARTUPINFOA` wired to the child ends,
// `CreateProcessA`, handle hygiene, the tag-10 record — is identical, and lives
// here once.
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
//   [SI..SI+104)  STARTUPINFOA (dwFlags@60, hStdInput@80/hStdOutput@88/hStdError@96)
//   [PI..PI+24)   PROCESS_INFORMATION (hProcess@0, hThread@8, dwProcessId@16)
//   [SA..SA+24)   SECURITY_ATTRIBUTES (nLength@0, lpSD@8, bInheritHandle@16)
//   IN_R/IN_W/OUT_R/OUT_W/ERR_R/ERR_W  CreatePipe out-handle slots
//   REC           the allocated resource record
//   CMD/ENV/CWD   the caller's three inputs (see `emit_win_spawn_tail`)
// ---------------------------------------------------------------------------

/// `STARTUPINFOA` (104 bytes).
pub(crate) const WIN_SPAWN_SI: usize = 0x50;
/// `PROCESS_INFORMATION` (24 bytes).
pub(crate) const WIN_SPAWN_PI: usize = 0xB8;
/// `SECURITY_ATTRIBUTES` (24 bytes).
pub(crate) const WIN_SPAWN_SA: usize = 0xD0;
/// Child stdin read end (the child inherits it).
pub(crate) const WIN_SPAWN_IN_R: usize = 0xE8;
/// Parent stdin write end (kept in the record).
pub(crate) const WIN_SPAWN_IN_W: usize = 0xF0;
/// Parent stdout read end (kept in the record).
pub(crate) const WIN_SPAWN_OUT_R: usize = 0xF8;
/// Child stdout write end (the child inherits it).
pub(crate) const WIN_SPAWN_OUT_W: usize = 0x100;
/// Parent stderr read end (kept in the record).
pub(crate) const WIN_SPAWN_ERR_R: usize = 0x108;
/// Child stderr write end (the child inherits it).
pub(crate) const WIN_SPAWN_ERR_W: usize = 0x110;
/// The allocated `Process` record.
pub(crate) const WIN_SPAWN_REC: usize = 0x118;
/// **Caller input**: the NUL-terminated `lpCommandLine`. Never NULL.
pub(crate) const WIN_SPAWN_CMD: usize = 0x120;
/// **Caller input**: `lpEnvironment` — an ANSI `name=value\0…\0\0` block, or 0
/// to inherit the parent's environment.
pub(crate) const WIN_SPAWN_ENV: usize = 0x128;
/// **Caller input**: `lpCurrentDirectory` — a NUL-terminated path, or 0 to
/// inherit the parent's working directory.
pub(crate) const WIN_SPAWN_CWD: usize = 0x130;
/// First offset a caller may use for its own scratch slots.
pub(crate) const WIN_SPAWN_SCRATCH: usize = 0x138;

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

const SI_DWFLAGS: usize = 60;
const SI_HSTDIN: usize = 80;
const SI_HSTDOUT: usize = 88;
const SI_HSTDERR: usize = 96;
const HANDLE_FLAG_INHERIT: &str = "1";
const STARTF_USESTDHANDLES: &str = "256"; // 0x100

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
    // Zero STARTUPINFOA (104 bytes), set cb = 104, dwFlags = STARTF_USESTDHANDLES,
    // and the three child-end handles.
    for off in (0..104).step_by(8) {
        instructions.push(abi::store_u64(abi::ZERO, sp, WIN_SPAWN_SI + off));
    }
    instructions.extend([
        abi::move_immediate(abi::mfb_arg(0), "Integer", "104"),
        abi::store_u32(abi::mfb_arg(0), sp, WIN_SPAWN_SI),
        abi::move_immediate(abi::mfb_arg(0), "Integer", STARTF_USESTDHANDLES),
        abi::store_u32(abi::mfb_arg(0), sp, WIN_SPAWN_SI + SI_DWFLAGS),
        abi::load_u64(abi::mfb_arg(0), sp, WIN_SPAWN_IN_R),
        abi::store_u64(abi::mfb_arg(0), sp, WIN_SPAWN_SI + SI_HSTDIN),
        abi::load_u64(abi::mfb_arg(0), sp, WIN_SPAWN_OUT_W),
        abi::store_u64(abi::mfb_arg(0), sp, WIN_SPAWN_SI + SI_HSTDOUT),
        abi::load_u64(abi::mfb_arg(0), sp, WIN_SPAWN_ERR_W),
        abi::store_u64(abi::mfb_arg(0), sp, WIN_SPAWN_SI + SI_HSTDERR),
        // CreateProcessA(NULL, cmd, NULL, NULL, TRUE, 0, env, cwd, &si, &pi).
        // Win64: register args 0..3 in mfb_arg (rcx/rdx/r8/r9); stack args 5..10
        // stored directly at sp+0x20.. (after the 32-byte shadow).
        abi::move_immediate(abi::mfb_arg(0), "Integer", "1"),
        abi::store_u64(abi::mfb_arg(0), sp, 0x20), // 5th bInheritHandles = TRUE
        abi::store_u64(abi::ZERO, sp, 0x28),       // 6th dwCreationFlags
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
    instructions.extend([
        abi::sign_extend_word(abi::c_return(0), abi::c_return(0)),
        abi::compare_immediate(abi::c_return(0), "0"),
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
    ]);
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
