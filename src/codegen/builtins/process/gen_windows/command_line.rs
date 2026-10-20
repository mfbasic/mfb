//! plan-176-T: the Windows `process` command-line builder (`emit_win_build_cmdline`) and its
//! frame offsets. The shared spawn helpers live in `builtins/process/gen_windows/mod.rs`.

use super::*;

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
