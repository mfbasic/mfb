use super::*;

/// `_mfb_rt_io_stdout_drain` (plan-14-A): flush the per-arena stdout output
/// buffer to fd 1. A no-op when buffering is off (`OUT_ENABLED == 0`) or nothing
/// is pending; otherwise a `write(1, OUT_PTR, OUT_FILLED)` loop that empties the
/// buffer and resets `OUT_FILLED = 0`. Returns `x0 = 0` on success (including the
/// no-op cases) and `x0 = 1` on a write failure — on failure the buffer is left
/// intact (`OUT_PTR`/`OUT_FILLED` unchanged) so a later flush can retry. Reads the
/// arena state through the pinned arena register (`x19`); shared by `io::flush`,
/// the buffered-write overflow path, `io::setBuffered(FALSE)`, every stdin read,
/// and `_mfb_shutdown`.
pub(super) fn lower_stdout_drain(
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<CodeFunction, String> {
    let symbol = STDOUT_DRAIN_SYMBOL;
    let ok = format!("{symbol}_ok");
    let drain_loop = format!("{symbol}_loop");
    let err = format!("{symbol}_err");
    let mut instructions = vec![
        abi::label("entry"),
        abi::load_u64("%v0", ARENA_STATE_REGISTER, ARENA_OUT_ENABLED_OFFSET),
        abi::compare_immediate("%v0", "0"),
        abi::branch_eq(&ok),
        abi::load_u64("%v1", ARENA_STATE_REGISTER, ARENA_OUT_FILLED_OFFSET),
        abi::compare_immediate("%v1", "0"),
        abi::branch_eq(&ok),
        abi::load_u64("%v2", ARENA_STATE_REGISTER, ARENA_OUT_PTR_OFFSET),
        abi::label(&drain_loop),
        abi::move_immediate(abi::return_register(), "Integer", "1"),
        abi::move_register(abi::string_data_register(), "%v2"),
        abi::move_register(abi::string_length_register(), "%v1"),
    ];
    let mut relocations = Vec::new();
    platform.emit_write(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::move_register("%v3", abi::return_register()),
        abi::compare_immediate("%v3", "0"),
        abi::branch_lt(&err),
        abi::add_registers("%v2", "%v2", "%v3"),
        abi::subtract_registers("%v1", "%v1", "%v3"),
        abi::compare_immediate("%v1", "0"),
        abi::branch_ne(&drain_loop),
        abi::store_u64("x31", ARENA_STATE_REGISTER, ARENA_OUT_FILLED_OFFSET),
        abi::label(&ok),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::return_(),
        abi::label(&err),
        abi::move_immediate(abi::return_register(), "Integer", "1"),
        abi::return_(),
    ]);
    Ok(finalize_vreg_helper(
        "runtime.io.stdout_drain",
        symbol,
        "Integer",
        instructions,
        relocations,
    ))
}

/// Emit the instructions that append the `len`-byte chunk at `src` to the
/// per-arena stdout buffer (plan-14-A §4.1), assuming buffering is enabled. `src`
/// and `len` are vreg names holding the source pointer and byte count; both are
/// preserved across the internal calls (the allocator spills any vreg live across
/// a `bl`). The buffer is lazily allocated on first use; if `filled + len` would
/// overflow the 4 KiB capacity the buffer is drained first, and a chunk larger
/// than the whole buffer is written directly after the drain (never split). Any
/// underlying `write` failure branches to `write_error`. `tag` disambiguates the
/// emitted labels so the helper can append more than one chunk (e.g. a line plus
/// its trailing newline). Uses vregs `%v20`..`%v29`.
fn emit_append_to_stdout_buffer(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    src: &str,
    len: &str,
    tag: &str,
    write_error: &str,
) -> Result<(), String> {
    let cap = OUT_BUFFER_CAPACITY.to_string();
    let have_buf = format!("{symbol}_buf_{tag}_have");
    let alloc_failed = format!("{symbol}_buf_{tag}_alloc_failed");
    let alloc_failed_loop = format!("{symbol}_buf_{tag}_alloc_failed_loop");
    let big_write_loop = format!("{symbol}_buf_{tag}_big_write_loop");
    let fits = format!("{symbol}_buf_{tag}_fits");
    let copy_loop = format!("{symbol}_buf_{tag}_copy_loop");
    let byte_tail = format!("{symbol}_buf_{tag}_byte_tail");
    let copy_done = format!("{symbol}_buf_{tag}_copy_done");
    let appended = format!("{symbol}_buf_{tag}_appended");
    instructions.extend([
        abi::load_u64("%v20", ARENA_STATE_REGISTER, ARENA_OUT_PTR_OFFSET),
        abi::compare_immediate("%v20", "0"),
        abi::branch_ne(&have_buf),
        // Lazily allocate the 4 KiB buffer on first buffered write.
        abi::move_immediate(abi::return_register(), "Integer", &cap),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_ne(&alloc_failed),
        abi::store_u64("x1", ARENA_STATE_REGISTER, ARENA_OUT_PTR_OFFSET),
        abi::move_register("%v20", "x1"),
        abi::branch(&have_buf),
        // Allocation failed: fall back to writing this chunk directly so no output
        // is lost — buffering is an optimization, never a correctness dependency.
        // Loop on short writes (bug-51): one write() may transfer fewer than
        // `remaining` bytes; advance the cursor and retry until nothing remains. A 0
        // or -1 return is a failure, never success. %v40/%v41 are vregs, so the
        // allocator spills the cursor/remaining across each `bl write`.
        abi::label(&alloc_failed),
        abi::move_register("%v40", src),
        abi::move_register("%v41", len),
        abi::label(&alloc_failed_loop),
        abi::compare_immediate("%v41", "0"),
        abi::branch_eq(&appended),
        abi::move_immediate(abi::return_register(), "Integer", "1"),
        abi::move_register(abi::string_data_register(), "%v40"),
        abi::move_register(abi::string_length_register(), "%v41"),
    ]);
    platform.emit_write(symbol, platform_imports, instructions, relocations)?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(write_error),
        abi::add_registers("%v40", "%v40", abi::return_register()),
        abi::subtract_registers("%v41", "%v41", abi::return_register()),
        abi::branch(&alloc_failed_loop),
        abi::label(&have_buf),
        abi::load_u64("%v21", ARENA_STATE_REGISTER, ARENA_OUT_FILLED_OFFSET),
        abi::add_registers("%v22", "%v21", len),
        abi::move_immediate("%v23", "Integer", &cap),
        abi::compare_registers("%v22", "%v23"),
        abi::branch_ls(&fits),
        // filled + len would overflow the buffer: drain what is pending first.
        abi::branch_link(STDOUT_DRAIN_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, STDOUT_DRAIN_SYMBOL));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(write_error),
        // After the drain OUT_FILLED is 0; reflect that locally.
        abi::move_immediate("%v21", "Integer", "0"),
        abi::move_immediate("%v23", "Integer", &cap),
        abi::compare_registers(len, "%v23"),
        abi::branch_ls(&fits),
        // The chunk is larger than the whole buffer: write it directly (the buffer
        // was just drained, so ordering is preserved) rather than splitting it.
        // Loop on short writes (bug-51) until the whole chunk lands; a 0/-1 return is
        // a failure. %v40/%v41 (cursor/remaining) are vregs → spilled/reloaded across
        // each `bl write`.
        abi::move_register("%v40", src),
        abi::move_register("%v41", len),
        abi::label(&big_write_loop),
        abi::compare_immediate("%v41", "0"),
        abi::branch_eq(&appended),
        abi::move_immediate(abi::return_register(), "Integer", "1"),
        abi::move_register(abi::string_data_register(), "%v40"),
        abi::move_register(abi::string_length_register(), "%v41"),
    ]);
    platform.emit_write(symbol, platform_imports, instructions, relocations)?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(write_error),
        abi::add_registers("%v40", "%v40", abi::return_register()),
        abi::subtract_registers("%v41", "%v41", abi::return_register()),
        abi::branch(&big_write_loop),
        abi::label(&fits),
        // Copy len bytes from src into OUT_PTR[filled..].
        abi::load_u64("%v20", ARENA_STATE_REGISTER, ARENA_OUT_PTR_OFFSET),
        abi::add_registers("%v24", "%v20", "%v21"),
        abi::move_register("%v25", src),
        abi::move_register("%v26", len),
        // Word-then-byte block copy (plan-25-D §D2, mirroring
        // emit_block_copy_advance): 8 bytes per iteration with a byte tail for the
        // remainder — an order of magnitude fewer iterations than the old per-byte
        // loop on payloads larger than a word.
        abi::label(&copy_loop),
        abi::compare_immediate("%v26", "8"),
        abi::branch_lo(&byte_tail),
        abi::load_u64("%v27", "%v25", 0),
        abi::store_u64("%v27", "%v24", 0),
        abi::add_immediate("%v24", "%v24", 8),
        abi::add_immediate("%v25", "%v25", 8),
        abi::subtract_immediate("%v26", "%v26", 8),
        abi::branch(&copy_loop),
        abi::label(&byte_tail),
        abi::compare_immediate("%v26", "0"),
        abi::branch_eq(&copy_done),
        abi::load_u8("%v27", "%v25", 0),
        abi::store_u8("%v27", "%v24", 0),
        abi::add_immediate("%v24", "%v24", 1),
        abi::add_immediate("%v25", "%v25", 1),
        abi::subtract_immediate("%v26", "%v26", 1),
        abi::branch(&byte_tail),
        abi::label(&copy_done),
        abi::add_registers("%v28", "%v21", len),
        abi::store_u64("%v28", ARENA_STATE_REGISTER, ARENA_OUT_FILLED_OFFSET),
        abi::label(&appended),
    ]);
    Ok(())
}

pub(super) fn lower_io_write_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    stderr: bool,
    append_newline: bool,
) -> Result<
    (
        CodeFrame,
        Vec<CodeInstruction>,
        Vec<CodeRelocation>,
        Vec<CodeStackSlot>,
    ),
    String,
> {
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    // Opt-in stdout buffering (plan-14-A): stderr is never buffered, so only the
    // stdout helper gets the prologue. When `OUT_ENABLED == 0` (the default) fall
    // straight through to the unbuffered direct-write path below, byte-identical
    // to pre-plan-14; when enabled, append into the per-arena buffer instead.
    if !stderr {
        let direct = format!("{symbol}_direct");
        let write_error = format!("{symbol}_write_error");
        instructions.extend([
            abi::load_u64("%v18", ARENA_STATE_REGISTER, ARENA_OUT_ENABLED_OFFSET),
            abi::compare_immediate("%v18", "0"),
            abi::branch_eq(&direct),
            // Capture the source pointer/length in vregs before any call clobbers x0.
            abi::load_u64("%v19", abi::return_register(), 0),
            abi::add_immediate("%v17", abi::return_register(), 8),
        ]);
        emit_append_to_stdout_buffer(
            symbol,
            platform_imports,
            platform,
            &mut instructions,
            &mut relocations,
            "%v17",
            "%v19",
            "line",
            &write_error,
        )?;
        if append_newline {
            instructions.extend([
                abi::move_immediate("%v16", "Integer", "10"),
                abi::store_u8("%v16", abi::stack_pointer(), 0),
                abi::add_immediate("%v17", abi::stack_pointer(), 0),
                abi::move_immediate("%v19", "Integer", "1"),
            ]);
            emit_append_to_stdout_buffer(
                symbol,
                platform_imports,
                platform,
                &mut instructions,
                &mut relocations,
                "%v17",
                "%v19",
                "newline",
                &write_error,
            )?;
        }
        instructions.extend([
            abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
            // The buffered success exit reuses the shared `done` epilogue emitted
            // below (the direct path lands there too), and any drain/write failure
            // above already branched to the shared `write_error` label.
            abi::branch(&format!("{symbol}_done")),
            abi::label(&direct),
        ]);
    }
    let write_error = format!("{symbol}_write_error");
    let done = format!("{symbol}_done");
    let fd_str = if stderr { "2" } else { "1" };
    let direct_loop = format!("{symbol}_direct_loop");
    let direct_written = format!("{symbol}_direct_written");
    // Loop on short writes (bug-51): a single write() may transfer fewer than the
    // string's byte count (pipe/FIFO, filling disk, signal); advance the cursor and
    // retry until nothing remains. A 0 or -1 return is a write failure, never
    // success. %v13/%v14 (cursor/remaining) are vregs, so the allocator spills them
    // across each `bl write` and reloads them afterward (compiler.md register
    // lifetimes) — the pointer/count are never read from a caller-saved register.
    instructions.extend([
        abi::load_u64("%v14", abi::return_register(), 0),
        abi::add_immediate("%v13", abi::return_register(), 8),
        abi::label(&direct_loop),
        abi::compare_immediate("%v14", "0"),
        abi::branch_eq(&direct_written),
        abi::move_register(abi::string_data_register(), "%v13"),
        abi::move_register(abi::string_length_register(), "%v14"),
        abi::move_immediate(abi::return_register(), "Integer", fd_str),
    ]);
    platform.emit_write(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(&write_error),
        abi::add_registers("%v13", "%v13", abi::return_register()),
        abi::subtract_registers("%v14", "%v14", abi::return_register()),
        abi::branch(&direct_loop),
        abi::label(&direct_written),
    ]);
    if append_newline {
        let newline_loop = format!("{symbol}_newline_loop");
        let newline_written = format!("{symbol}_newline_written");
        instructions.extend([
            abi::move_immediate("%v9", "Integer", "10"),
            abi::store_u64("%v9", abi::stack_pointer(), 8),
            abi::add_immediate("%v13", abi::stack_pointer(), 8),
            abi::move_immediate("%v14", "Integer", "1"),
            // A 1-byte write cannot short-count positively, but a 0 return still
            // means the byte was not written — loop and treat 0/-1 as a failure.
            abi::label(&newline_loop),
            abi::compare_immediate("%v14", "0"),
            abi::branch_eq(&newline_written),
            abi::move_register(abi::string_data_register(), "%v13"),
            abi::move_register(abi::string_length_register(), "%v14"),
            abi::move_immediate(abi::return_register(), "Integer", fd_str),
        ]);
        platform.emit_write(
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_le(&write_error),
            abi::add_registers("%v13", "%v13", abi::return_register()),
            abi::subtract_registers("%v14", "%v14", abi::return_register()),
            abi::branch(&newline_loop),
            abi::label(&newline_written),
        ]);
    }
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&write_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUTPUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    let output_error_symbol = ERR_OUTPUT_SYMBOL.to_string();
    instructions.push(
        CodeInstruction::new("adrp")
            .field("dst", RESULT_ERROR_MESSAGE_REGISTER)
            .field("symbol", &output_error_symbol),
    );
    instructions.push(
        CodeInstruction::new("add_pageoff")
            .field("dst", RESULT_ERROR_MESSAGE_REGISTER)
            .field("src", RESULT_ERROR_MESSAGE_REGISTER)
            .field("symbol", &output_error_symbol),
    );
    relocations.extend([
        CodeRelocation {
            from: symbol.to_string(),
            to: output_error_symbol.clone(),
            kind: RelocIntent::DataAddrHi,
            binding: "data".to_string(),
            library: None,
        },
        CodeRelocation {
            from: symbol.to_string(),
            to: output_error_symbol,
            kind: RelocIntent::DataAddrLo,
            binding: "data".to_string(),
            library: None,
        },
    ]);
    instructions.push(abi::label(&done));
    instructions.push(abi::return_());
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], 16);
    Ok((frame, instructions, relocations, stack_slots))
}

pub(super) fn lower_io_flush_helper(
    symbol: &str,
    // Flush is now drain-only (no fsync), so it no longer needs the platform to
    // emit a libc/syscall sequence; kept in the signature for parity with the
    // other io helper lowerings dispatched from mod.rs.
    _platform_imports: &HashMap<String, String>,
    _platform: &dyn CodegenPlatform,
    stderr: bool,
) -> Result<
    (
        CodeFrame,
        Vec<CodeInstruction>,
        Vec<CodeRelocation>,
        Vec<CodeStackSlot>,
    ),
    String,
> {
    const FRAME_SIZE: usize = 16;

    let output_error = format!("{symbol}_output_error");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    // io::flush() drains the per-arena MFBASIC stdout buffer via write() and
    // reports a write failure — nothing else. It deliberately does NOT fsync:
    // fsync's result depends on the fd *type* (EBADF only for a genuinely closed
    // fd, benign EINVAL on pipes/char devices, 0 on a regular file), which made
    // flush's success/failure depend on the runtime environment rather than on
    // what the program actually wrote. The buffer drain's write() is the one
    // portable failure signal — identical on every platform/libc. A no-op when
    // buffering is off, and stderr is never buffered, so flushing stderr always
    // succeeds (nothing to drain).
    if !stderr {
        instructions.push(abi::branch_link(STDOUT_DRAIN_SYMBOL));
        relocations.push(internal_branch(symbol, STDOUT_DRAIN_SYMBOL));
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_ne(&output_error),
        ]);
    }
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&output_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUTPUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_OUTPUT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::label(&done));
    instructions.push(abi::return_());
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], FRAME_SIZE);
    Ok((frame, instructions, relocations, stack_slots))
}

/// `io::isBuffered()` (plan-14-A §4.2): report whether opt-in stdout buffering is
/// on for this thread — `OUT_ENABLED != 0`. In app mode the buffer is inert, so it
/// always reports FALSE.
pub(super) fn lower_io_is_buffered_helper(
    symbol: &str,
    app_mode: bool,
) -> Result<
    (
        CodeFrame,
        Vec<CodeInstruction>,
        Vec<CodeRelocation>,
        Vec<CodeStackSlot>,
    ),
    String,
> {
    const FRAME_SIZE: usize = 16;
    let yes = format!("{symbol}_yes");
    let done = format!("{symbol}_done");
    let mut instructions = vec![abi::label("entry")];
    if app_mode {
        instructions.push(abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"));
    } else {
        instructions.extend([
            abi::load_u64("%v0", ARENA_STATE_REGISTER, ARENA_OUT_ENABLED_OFFSET),
            abi::compare_immediate("%v0", "0"),
            abi::branch_ne(&yes),
            abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
            abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
            abi::branch(&done),
            abi::label(&yes),
            abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
        ]);
    }
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::label(&done),
        abi::return_(),
    ]);
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], FRAME_SIZE);
    Ok((frame, instructions, Vec::new(), stack_slots))
}

/// `io::setBuffered(enabled)` (plan-14-A §4.2): turn opt-in stdout buffering on or
/// off for this thread. Enabling just sets `OUT_ENABLED = 1` (the 4 KiB buffer is
/// allocated lazily on the first buffered write). Disabling **drains the buffer
/// first** (so pending bytes are never stranded on the off transition) and then
/// clears `OUT_ENABLED`. Returns `Nothing`. In app mode buffering is inert, so it
/// is a no-op returning OK.
pub(super) fn lower_io_set_buffered_helper(
    symbol: &str,
    app_mode: bool,
) -> Result<
    (
        CodeFrame,
        Vec<CodeInstruction>,
        Vec<CodeRelocation>,
        Vec<CodeStackSlot>,
    ),
    String,
> {
    const FRAME_SIZE: usize = 16;
    let enable = format!("{symbol}_enable");
    let done = format!("{symbol}_done");
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    if !app_mode {
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_ne(&enable),
            // Disable: drain any pending bytes first, then clear the flag. The drain
            // result is best-effort here (setBuffered returns Nothing); a real write
            // failure still surfaces on the next io::flush / buffered write.
            abi::branch_link(STDOUT_DRAIN_SYMBOL),
        ]);
        relocations.push(internal_branch(symbol, STDOUT_DRAIN_SYMBOL));
        instructions.extend([
            abi::store_u64("x31", ARENA_STATE_REGISTER, ARENA_OUT_ENABLED_OFFSET),
            abi::branch(&done),
            abi::label(&enable),
            abi::move_immediate("%v0", "Integer", "1"),
            abi::store_u64("%v0", ARENA_STATE_REGISTER, ARENA_OUT_ENABLED_OFFSET),
            abi::label(&done),
        ]);
    }
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::return_(),
    ]);
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], FRAME_SIZE);
    Ok((frame, instructions, relocations, stack_slots))
}

pub(super) fn lower_io_poll_input_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<
    (
        CodeFrame,
        Vec<CodeInstruction>,
        Vec<CodeRelocation>,
        Vec<CodeStackSlot>,
    ),
    String,
> {
    const POLLIN_PACKED_FD0: &str = "4294967296";
    const FRAME_SIZE: usize = 48;
    const POLLFD_OFFSET: usize = 8;
    const TIMEOUT_OFFSET: usize = 32;

    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::move_immediate("%v9", "Integer", POLLIN_PACKED_FD0),
        abi::store_u64("%v9", abi::stack_pointer(), POLLFD_OFFSET),
    ]);

    instructions.push(abi::load_u64("x2", abi::stack_pointer(), TIMEOUT_OFFSET));

    instructions.extend([
        abi::add_immediate(abi::return_register(), abi::stack_pointer(), POLLFD_OFFSET),
        abi::move_immediate("x1", "Integer", "1"),
    ]);
    platform.emit_poll_input(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;

    let poll_error = format!("{symbol}_poll_error");
    let poll_ready = format!("{symbol}_poll_ready");
    let done = format!("{symbol}_done");
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&poll_error),
        abi::branch_gt(&poll_ready),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&poll_ready),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&poll_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INPUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    let input_error_symbol = ERR_INPUT_SYMBOL.to_string();
    instructions.push(
        CodeInstruction::new("adrp")
            .field("dst", RESULT_ERROR_MESSAGE_REGISTER)
            .field("symbol", &input_error_symbol),
    );
    instructions.push(
        CodeInstruction::new("add_pageoff")
            .field("dst", RESULT_ERROR_MESSAGE_REGISTER)
            .field("src", RESULT_ERROR_MESSAGE_REGISTER)
            .field("symbol", &input_error_symbol),
    );
    relocations.extend([
        CodeRelocation {
            from: symbol.to_string(),
            to: input_error_symbol.clone(),
            kind: RelocIntent::DataAddrHi,
            binding: "data".to_string(),
            library: None,
        },
        CodeRelocation {
            from: symbol.to_string(),
            to: input_error_symbol,
            kind: RelocIntent::DataAddrLo,
            binding: "data".to_string(),
            library: None,
        },
    ]);
    instructions.push(abi::label(&done));
    instructions.push(abi::return_());
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], FRAME_SIZE);
    Ok((frame, instructions, relocations, stack_slots))
}

fn termios_storage_size(platform: &dyn CodegenPlatform) -> usize {
    platform.termios_size().next_multiple_of(8)
}

struct TerminalModeSlots {
    active: usize,
    saved_tag: usize,
    saved_value: usize,
    saved_message: usize,
    original: usize,
    modified: usize,
}

fn emit_configure_stdin_terminal(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    slots: &TerminalModeSlots,
    disable_echo: bool,
    disable_canonical: bool,
    error_label: &str,
) -> Result<(), String> {
    let skip = format!("{symbol}_terminal_mode_skip");
    instructions.extend([
        abi::store_u64("x31", abi::stack_pointer(), slots.active),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
    ]);
    platform.emit_libc_call(
        "isatty",
        symbol,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(&skip),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::add_immediate("x1", abi::stack_pointer(), slots.original),
    ]);
    platform.emit_libc_call(
        "tcgetattr",
        symbol,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(error_label),
        abi::move_immediate("%v9", "Integer", "1"),
        abi::store_u64("%v9", abi::stack_pointer(), slots.active),
    ]);

    for offset in (0..termios_storage_size(platform)).step_by(8) {
        instructions.extend([
            abi::load_u64("%v9", abi::stack_pointer(), slots.original + offset),
            abi::store_u64("%v9", abi::stack_pointer(), slots.modified + offset),
        ]);
    }

    let mut clear_flags = 0;
    if disable_echo {
        clear_flags |= platform.termios_echo_flag();
    }
    if disable_canonical {
        clear_flags |= platform.termios_icanon_flag();
    }
    if clear_flags != 0 {
        let lflag_offset = slots.modified + platform.termios_lflag_offset();
        if platform.termios_lflag_width() == 4 {
            instructions.push(abi::load_u32("%v9", abi::stack_pointer(), lflag_offset));
        } else {
            instructions.push(abi::load_u64("%v9", abi::stack_pointer(), lflag_offset));
        }
        instructions.extend([
            abi::move_immediate("%v10", "Integer", &clear_flags.to_string()),
            abi::bitwise_not("%v10", "%v10"),
            abi::and_registers("%v9", "%v9", "%v10"),
        ]);
        if platform.termios_lflag_width() == 4 {
            instructions.push(abi::store_u32("%v9", abi::stack_pointer(), lflag_offset));
        } else {
            instructions.push(abi::store_u64("%v9", abi::stack_pointer(), lflag_offset));
        }
    }

    if disable_canonical {
        let cc_offset = slots.modified + platform.termios_cc_offset();
        instructions.extend([
            abi::move_immediate("%v9", "Integer", "1"),
            abi::store_u8(
                "%v9",
                abi::stack_pointer(),
                cc_offset + platform.termios_vmin_index(),
            ),
            abi::store_u8(
                "x31",
                abi::stack_pointer(),
                cc_offset + platform.termios_vtime_index(),
            ),
        ]);
    }

    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::move_immediate("x1", "Integer", "0"),
        abi::add_immediate("x2", abi::stack_pointer(), slots.modified),
    ]);
    platform.emit_libc_call(
        "tcsetattr",
        symbol,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(error_label),
        abi::label(&skip),
    ]);
    Ok(())
}

fn emit_restore_stdin_terminal(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    slots: &TerminalModeSlots,
) -> Result<(), String> {
    let restored = format!("{symbol}_terminal_mode_restored");
    let restore_failed = format!("{symbol}_terminal_mode_restore_failed");
    instructions.extend([
        abi::store_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), slots.saved_tag),
        abi::store_u64(
            RESULT_VALUE_REGISTER,
            abi::stack_pointer(),
            slots.saved_value,
        ),
        abi::store_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            slots.saved_message,
        ),
        abi::load_u64("%v9", abi::stack_pointer(), slots.active),
        abi::compare_immediate("%v9", "1"),
        abi::branch_ne(&restored),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::move_immediate("x1", "Integer", "0"),
        abi::add_immediate("x2", abi::stack_pointer(), slots.original),
    ]);
    platform.emit_libc_call(
        "tcsetattr",
        symbol,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&restore_failed),
        abi::label(&restored),
        abi::load_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), slots.saved_tag),
        abi::load_u64(
            RESULT_VALUE_REGISTER,
            abi::stack_pointer(),
            slots.saved_value,
        ),
        abi::load_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            slots.saved_message,
        ),
        abi::branch(&format!("{symbol}_terminal_mode_restore_done")),
        abi::label(&restore_failed),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INPUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_INPUT_SYMBOL, instructions, relocations);
    instructions.push(abi::label(&format!("{symbol}_terminal_mode_restore_done")));
    Ok(())
}

pub(super) fn lower_io_read_byte_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    app_mode: bool,
) -> Result<
    (
        CodeFrame,
        Vec<CodeInstruction>,
        Vec<CodeRelocation>,
        Vec<CodeStackSlot>,
    ),
    String,
> {
    const FRAME_SIZE: usize = 208;
    const BYTE_OFFSET: usize = 8;
    let terminal_slots = TerminalModeSlots {
        active: 16,
        saved_tag: 24,
        saved_value: 32,
        saved_message: 40,
        original: 48,
        modified: 120,
    };
    let eof = format!("{symbol}_eof");
    let input_error = format!("{symbol}_input_error");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    // Drain buffered stdout before blocking on input (plan-14-A §4.3 hook 2);
    // no-op when buffering is off, skipped in app mode (no stdout buffer).
    if !app_mode {
        instructions.push(abi::branch_link(STDOUT_DRAIN_SYMBOL));
        relocations.push(internal_branch(symbol, STDOUT_DRAIN_SYMBOL));
    }
    if app_mode {
        platform
            .emit_app_raw_input_mode(symbol, &mut instructions, &mut relocations)
            .ok_or_else(|| {
                format!(
                    "native target '{}' does not support app-mode raw input",
                    platform.target()
                )
            })??;
    }
    emit_configure_stdin_terminal(
        symbol,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
        &terminal_slots,
        true,
        true,
        &input_error,
    )?;
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::add_immediate("x1", abi::stack_pointer(), BYTE_OFFSET),
        abi::move_immediate("x2", "Integer", "1"),
    ]);
    platform.emit_read_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&input_error),
        abi::branch_eq(&eof),
        abi::load_u8(RESULT_VALUE_REGISTER, abi::stack_pointer(), BYTE_OFFSET),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&eof),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_EOF_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_EOF_SYMBOL, &mut instructions, &mut relocations);
    instructions.extend([
        abi::branch(&done),
        abi::label(&input_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INPUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INPUT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::label(&done));
    emit_restore_stdin_terminal(
        symbol,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
        &terminal_slots,
    )?;
    instructions.push(abi::return_());
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], FRAME_SIZE);
    Ok((frame, instructions, relocations, stack_slots))
}

pub(super) fn lower_io_is_terminal_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    fd: u8,
) -> Result<
    (
        CodeFrame,
        Vec<CodeInstruction>,
        Vec<CodeRelocation>,
        Vec<CodeStackSlot>,
    ),
    String,
> {
    const FRAME_SIZE: usize = 16;
    let yes = format!("{symbol}_yes");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    instructions.push(abi::move_immediate(
        abi::return_register(),
        "Integer",
        &fd.to_string(),
    ));
    platform.emit_is_terminal(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_gt(&yes),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&yes),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::label(&done),
    ]);
    instructions.push(abi::return_());
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], FRAME_SIZE);
    Ok((frame, instructions, relocations, stack_slots))
}

pub(super) fn lower_io_read_char_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    app_mode: bool,
) -> Result<
    (
        CodeFrame,
        Vec<CodeInstruction>,
        Vec<CodeRelocation>,
        Vec<CodeStackSlot>,
    ),
    String,
> {
    const FRAME_SIZE: usize = 224;
    const BYTES_OFFSET: usize = 8;
    const LEN_OFFSET: usize = 16;
    const RESULT_OFFSET: usize = 24;
    let terminal_slots = TerminalModeSlots {
        active: 32,
        saved_tag: 40,
        saved_value: 48,
        saved_message: 56,
        original: 64,
        modified: 136,
    };
    let read_second = format!("{symbol}_read_second");
    let read_third = format!("{symbol}_read_third");
    let read_fourth = format!("{symbol}_read_fourth");
    let got_len = format!("{symbol}_got_len");
    let alloc_ok = format!("{symbol}_alloc_ok");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let eof = format!("{symbol}_eof");
    let input_error = format!("{symbol}_input_error");
    let encoding_error = format!("{symbol}_encoding_error");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    // Drain buffered stdout before blocking on input (plan-14-A §4.3 hook 2);
    // no-op when buffering is off, skipped in app mode (no stdout buffer).
    if !app_mode {
        instructions.push(abi::branch_link(STDOUT_DRAIN_SYMBOL));
        relocations.push(internal_branch(symbol, STDOUT_DRAIN_SYMBOL));
    }
    if app_mode {
        platform
            .emit_app_raw_input_mode(symbol, &mut instructions, &mut relocations)
            .ok_or_else(|| {
                format!(
                    "native target '{}' does not support app-mode raw input",
                    platform.target()
                )
            })??;
    }
    emit_configure_stdin_terminal(
        symbol,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
        &terminal_slots,
        true,
        true,
        &input_error,
    )?;
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::add_immediate("x1", abi::stack_pointer(), BYTES_OFFSET),
        abi::move_immediate("x2", "Integer", "1"),
    ]);
    platform.emit_read_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&input_error),
        abi::branch_eq(&eof),
        abi::load_u8("%v10", abi::stack_pointer(), BYTES_OFFSET),
        abi::compare_immediate("%v10", "127"),
        abi::branch_hi(&read_second),
        abi::move_immediate("%v11", "Integer", "1"),
        abi::store_u64("%v11", abi::stack_pointer(), LEN_OFFSET),
        abi::branch(&got_len),
        abi::label(&read_second),
        abi::compare_immediate("%v10", "194"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("%v10", "223"),
        abi::branch_hi(&read_third),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::add_immediate("x1", abi::stack_pointer(), BYTES_OFFSET + 1),
        abi::move_immediate("x2", "Integer", "1"),
    ]);
    platform.emit_read_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&input_error),
        abi::branch_eq(&encoding_error),
        abi::load_u8("%v11", abi::stack_pointer(), BYTES_OFFSET + 1),
        abi::compare_immediate("%v11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("%v11", "191"),
        abi::branch_hi(&encoding_error),
        abi::move_immediate("%v11", "Integer", "2"),
        abi::store_u64("%v11", abi::stack_pointer(), LEN_OFFSET),
        abi::branch(&got_len),
        abi::label(&read_third),
        abi::compare_immediate("%v10", "239"),
        abi::branch_hi(&read_fourth),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::add_immediate("x1", abi::stack_pointer(), BYTES_OFFSET + 1),
        abi::move_immediate("x2", "Integer", "1"),
    ]);
    platform.emit_read_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&input_error),
        abi::branch_eq(&encoding_error),
        abi::load_u8("%v11", abi::stack_pointer(), BYTES_OFFSET + 1),
        abi::compare_immediate("%v10", "224"),
        abi::branch_ne(&format!("{symbol}_three_not_e0")),
        abi::compare_immediate("%v11", "160"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("%v11", "191"),
        abi::branch_hi(&encoding_error),
        abi::branch(&format!("{symbol}_three_second_ok")),
        abi::label(&format!("{symbol}_three_not_e0")),
        abi::compare_immediate("%v10", "237"),
        abi::branch_ne(&format!("{symbol}_three_general")),
        abi::compare_immediate("%v11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("%v11", "159"),
        abi::branch_hi(&encoding_error),
        abi::branch(&format!("{symbol}_three_second_ok")),
        abi::label(&format!("{symbol}_three_general")),
        abi::compare_immediate("%v11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("%v11", "191"),
        abi::branch_hi(&encoding_error),
        abi::label(&format!("{symbol}_three_second_ok")),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::add_immediate("x1", abi::stack_pointer(), BYTES_OFFSET + 2),
        abi::move_immediate("x2", "Integer", "1"),
    ]);
    platform.emit_read_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&input_error),
        abi::branch_eq(&encoding_error),
        abi::load_u8("%v11", abi::stack_pointer(), BYTES_OFFSET + 2),
        abi::compare_immediate("%v11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("%v11", "191"),
        abi::branch_hi(&encoding_error),
        abi::move_immediate("%v11", "Integer", "3"),
        abi::store_u64("%v11", abi::stack_pointer(), LEN_OFFSET),
        abi::branch(&got_len),
        abi::label(&read_fourth),
        abi::compare_immediate("%v10", "240"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("%v10", "244"),
        abi::branch_hi(&encoding_error),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::add_immediate("x1", abi::stack_pointer(), BYTES_OFFSET + 1),
        abi::move_immediate("x2", "Integer", "1"),
    ]);
    platform.emit_read_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&input_error),
        abi::branch_eq(&encoding_error),
        abi::load_u8("%v11", abi::stack_pointer(), BYTES_OFFSET + 1),
        abi::compare_immediate("%v10", "240"),
        abi::branch_ne(&format!("{symbol}_four_not_f0")),
        abi::compare_immediate("%v11", "144"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("%v11", "191"),
        abi::branch_hi(&encoding_error),
        abi::branch(&format!("{symbol}_four_second_ok")),
        abi::label(&format!("{symbol}_four_not_f0")),
        abi::compare_immediate("%v10", "244"),
        abi::branch_ne(&format!("{symbol}_four_general")),
        abi::compare_immediate("%v11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("%v11", "143"),
        abi::branch_hi(&encoding_error),
        abi::branch(&format!("{symbol}_four_second_ok")),
        abi::label(&format!("{symbol}_four_general")),
        abi::compare_immediate("%v11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("%v11", "191"),
        abi::branch_hi(&encoding_error),
        abi::label(&format!("{symbol}_four_second_ok")),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::add_immediate("x1", abi::stack_pointer(), BYTES_OFFSET + 2),
        abi::move_immediate("x2", "Integer", "1"),
    ]);
    platform.emit_read_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&input_error),
        abi::branch_eq(&encoding_error),
        abi::load_u8("%v11", abi::stack_pointer(), BYTES_OFFSET + 2),
        abi::compare_immediate("%v11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("%v11", "191"),
        abi::branch_hi(&encoding_error),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::add_immediate("x1", abi::stack_pointer(), BYTES_OFFSET + 3),
        abi::move_immediate("x2", "Integer", "1"),
    ]);
    platform.emit_read_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&input_error),
        abi::branch_eq(&encoding_error),
        abi::load_u8("%v11", abi::stack_pointer(), BYTES_OFFSET + 3),
        abi::compare_immediate("%v11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("%v11", "191"),
        abi::branch_hi(&encoding_error),
        abi::move_immediate("%v11", "Integer", "4"),
        abi::store_u64("%v11", abi::stack_pointer(), LEN_OFFSET),
        abi::label(&got_len),
        abi::load_u64("%v10", abi::stack_pointer(), LEN_OFFSET),
        abi::add_immediate(abi::return_register(), "%v10", 9),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), RESULT_OFFSET),
        abi::load_u64("%v10", abi::stack_pointer(), LEN_OFFSET),
        abi::store_u64("%v10", "x1", 0),
        abi::add_immediate("%v11", "x1", 8),
        abi::add_immediate("%v12", abi::stack_pointer(), BYTES_OFFSET),
        abi::label(&copy_loop),
        abi::compare_immediate("%v10", "0"),
        abi::branch_eq(&copy_done),
        abi::load_u8("%v13", "%v12", 0),
        abi::store_u8("%v13", "%v11", 0),
        abi::add_immediate("%v11", "%v11", 1),
        abi::add_immediate("%v12", "%v12", 1),
        abi::subtract_immediate("%v10", "%v10", 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8("x31", "%v11", 0),
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), RESULT_OFFSET),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&eof),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_EOF_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_EOF_SYMBOL, &mut instructions, &mut relocations);
    instructions.extend([
        abi::branch(&done),
        abi::label(&input_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INPUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INPUT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&encoding_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_ENCODING_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ENCODING_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&alloc_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::label(&done));
    emit_restore_stdin_terminal(
        symbol,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
        &terminal_slots,
    )?;
    instructions.push(abi::return_());
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], FRAME_SIZE);
    Ok((frame, instructions, relocations, stack_slots))
}

pub(super) fn lower_io_read_line_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    with_prompt: bool,
    app_mode: bool,
) -> Result<
    (
        CodeFrame,
        Vec<CodeInstruction>,
        Vec<CodeRelocation>,
        Vec<CodeStackSlot>,
    ),
    String,
> {
    const FRAME_SIZE: usize = 256;
    const BUFFER_OFFSET: usize = 8;
    const CAPACITY_OFFSET: usize = 16;
    const LENGTH_OFFSET: usize = 24;
    const SEQ_LEN_OFFSET: usize = 32;
    const RESULT_OFFSET: usize = 40;
    const BYTES_OFFSET: usize = 48;
    // Old line-buffer pointer/size stashed across a grow so the dead buffer can be
    // returned to the arena free-list (plan-01 §8.3 runtime-internal reuse). The
    // termios scratch ends at 240 (macOS) / 228 (Linux), so 240/248 are free.
    const OLD_BUFFER_OFFSET: usize = 240;
    const OLD_CAPACITY_OFFSET: usize = 248;
    let terminal_slots = TerminalModeSlots {
        active: 56,
        saved_tag: 64,
        saved_value: 72,
        saved_message: 80,
        original: 96,
        modified: 168,
    };
    let prompt_flush = format!("{symbol}_prompt_flush");
    let alloc_ok = format!("{symbol}_alloc_ok");
    let read_loop = format!("{symbol}_read_loop");
    let have_sequence = format!("{symbol}_have_sequence");
    let grow = format!("{symbol}_grow");
    let grow_ok = format!("{symbol}_grow_ok");
    let grow_copy_loop = format!("{symbol}_grow_copy_loop");
    let grow_copy_done = format!("{symbol}_grow_copy_done");
    let append_loop = format!("{symbol}_append_loop");
    let append_done = format!("{symbol}_append_done");
    let trim_cr = format!("{symbol}_trim_cr");
    let result_alloc_ok = format!("{symbol}_result_alloc_ok");
    let result_copy_loop = format!("{symbol}_result_copy_loop");
    let result_copy_done = format!("{symbol}_result_copy_done");
    let output_error = format!("{symbol}_output_error");
    let eof_error = format!("{symbol}_eof_error");
    let input_error = format!("{symbol}_input_error");
    let encoding_error = format!("{symbol}_encoding_error");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    // Drain any buffered stdout before blocking on input (plan-14-A §4.3 hook 2)
    // so already-produced output — including a buffered prompt — appears before
    // the read. A no-op when buffering is off; skipped in app mode, which has no
    // stdout buffer. The prompt pointer (x0) is parked across the drain call.
    if !app_mode {
        if with_prompt {
            instructions.push(abi::move_register("%v40", abi::return_register()));
        }
        instructions.push(abi::branch_link(STDOUT_DRAIN_SYMBOL));
        relocations.push(internal_branch(symbol, STDOUT_DRAIN_SYMBOL));
        if with_prompt {
            instructions.push(abi::move_register(abi::return_register(), "%v40"));
        }
    }
    if with_prompt {
        // Write the prompt directly and report a write failure via output_error.
        // Like io::flush, prompt "flushing" is just the write() — the portable,
        // platform-independent failure signal. No fsync (its errno depends on the
        // fd type, not on the write). An empty prompt writes nothing and so
        // cannot fail; it joins at `prompt_flush` and proceeds to the read.
        let prompt_loop = format!("{symbol}_prompt_loop");
        instructions.extend([
            abi::load_u64("%v42", abi::return_register(), 0),
            abi::add_immediate("%v41", abi::return_register(), 8),
            // Loop on short writes (bug-51): write the whole prompt or report
            // output_error; a 0 or -1 return is a failure, never success. An empty
            // prompt writes nothing (remaining == 0) and joins at prompt_flush.
            // %v41/%v42 (cursor/remaining) are vregs → spilled/reloaded across each
            // `bl write`.
            abi::label(&prompt_loop),
            abi::compare_immediate("%v42", "0"),
            abi::branch_eq(&prompt_flush),
            abi::move_register(abi::string_data_register(), "%v41"),
            abi::move_register(abi::string_length_register(), "%v42"),
            abi::move_immediate(abi::return_register(), "Integer", "1"),
        ]);
        platform.emit_write(
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_le(&output_error),
            abi::add_registers("%v41", "%v41", abi::return_register()),
            abi::subtract_registers("%v42", "%v42", abi::return_register()),
            abi::branch(&prompt_loop),
            abi::label(&prompt_flush),
        ]);
    }
    if !with_prompt {
        emit_configure_stdin_terminal(
            symbol,
            platform_imports,
            platform,
            &mut instructions,
            &mut relocations,
            &terminal_slots,
            true,
            false,
            &input_error,
        )?;
    }
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "32"),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), BUFFER_OFFSET),
        abi::move_immediate("%v10", "Integer", "32"),
        abi::store_u64("%v10", abi::stack_pointer(), CAPACITY_OFFSET),
        abi::store_u64("x31", abi::stack_pointer(), LENGTH_OFFSET),
        abi::label(&read_loop),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::add_immediate("x1", abi::stack_pointer(), BYTES_OFFSET),
        abi::move_immediate("x2", "Integer", "1"),
    ]);
    platform.emit_read_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&input_error),
        abi::branch_eq(&format!("{symbol}_read_eof")),
        abi::load_u8("%v10", abi::stack_pointer(), BYTES_OFFSET),
        abi::compare_immediate("%v10", "10"),
        abi::branch_eq(&trim_cr),
        abi::compare_immediate("%v10", "127"),
        abi::branch_hi(&format!("{symbol}_multi_start")),
        abi::move_immediate("%v11", "Integer", "1"),
        abi::store_u64("%v11", abi::stack_pointer(), SEQ_LEN_OFFSET),
        abi::branch(&have_sequence),
        abi::label(&format!("{symbol}_multi_start")),
        abi::compare_immediate("%v10", "194"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("%v10", "223"),
        abi::branch_hi(&format!("{symbol}_line_read_third")),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::add_immediate("x1", abi::stack_pointer(), BYTES_OFFSET + 1),
        abi::move_immediate("x2", "Integer", "1"),
    ]);
    platform.emit_read_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&input_error),
        abi::branch_eq(&encoding_error),
        abi::load_u8("%v11", abi::stack_pointer(), BYTES_OFFSET + 1),
        abi::compare_immediate("%v11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("%v11", "191"),
        abi::branch_hi(&encoding_error),
        abi::move_immediate("%v11", "Integer", "2"),
        abi::store_u64("%v11", abi::stack_pointer(), SEQ_LEN_OFFSET),
        abi::branch(&have_sequence),
        abi::label(&format!("{symbol}_line_read_third")),
        abi::compare_immediate("%v10", "239"),
        abi::branch_hi(&format!("{symbol}_line_read_fourth")),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::add_immediate("x1", abi::stack_pointer(), BYTES_OFFSET + 1),
        abi::move_immediate("x2", "Integer", "1"),
    ]);
    platform.emit_read_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&input_error),
        abi::branch_eq(&encoding_error),
        abi::load_u8("%v11", abi::stack_pointer(), BYTES_OFFSET + 1),
        abi::compare_immediate("%v10", "224"),
        abi::branch_ne(&format!("{symbol}_line_three_not_e0")),
        abi::compare_immediate("%v11", "160"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("%v11", "191"),
        abi::branch_hi(&encoding_error),
        abi::branch(&format!("{symbol}_line_three_second_ok")),
        abi::label(&format!("{symbol}_line_three_not_e0")),
        abi::compare_immediate("%v10", "237"),
        abi::branch_ne(&format!("{symbol}_line_three_general")),
        abi::compare_immediate("%v11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("%v11", "159"),
        abi::branch_hi(&encoding_error),
        abi::branch(&format!("{symbol}_line_three_second_ok")),
        abi::label(&format!("{symbol}_line_three_general")),
        abi::compare_immediate("%v11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("%v11", "191"),
        abi::branch_hi(&encoding_error),
        abi::label(&format!("{symbol}_line_three_second_ok")),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::add_immediate("x1", abi::stack_pointer(), BYTES_OFFSET + 2),
        abi::move_immediate("x2", "Integer", "1"),
    ]);
    platform.emit_read_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&input_error),
        abi::branch_eq(&encoding_error),
        abi::load_u8("%v11", abi::stack_pointer(), BYTES_OFFSET + 2),
        abi::compare_immediate("%v11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("%v11", "191"),
        abi::branch_hi(&encoding_error),
        abi::move_immediate("%v11", "Integer", "3"),
        abi::store_u64("%v11", abi::stack_pointer(), SEQ_LEN_OFFSET),
        abi::branch(&have_sequence),
        abi::label(&format!("{symbol}_line_read_fourth")),
        abi::compare_immediate("%v10", "240"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("%v10", "244"),
        abi::branch_hi(&encoding_error),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::add_immediate("x1", abi::stack_pointer(), BYTES_OFFSET + 1),
        abi::move_immediate("x2", "Integer", "1"),
    ]);
    platform.emit_read_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&input_error),
        abi::branch_eq(&encoding_error),
        abi::load_u8("%v11", abi::stack_pointer(), BYTES_OFFSET + 1),
        abi::compare_immediate("%v10", "240"),
        abi::branch_ne(&format!("{symbol}_line_four_not_f0")),
        abi::compare_immediate("%v11", "144"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("%v11", "191"),
        abi::branch_hi(&encoding_error),
        abi::branch(&format!("{symbol}_line_four_second_ok")),
        abi::label(&format!("{symbol}_line_four_not_f0")),
        abi::compare_immediate("%v10", "244"),
        abi::branch_ne(&format!("{symbol}_line_four_general")),
        abi::compare_immediate("%v11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("%v11", "143"),
        abi::branch_hi(&encoding_error),
        abi::branch(&format!("{symbol}_line_four_second_ok")),
        abi::label(&format!("{symbol}_line_four_general")),
        abi::compare_immediate("%v11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("%v11", "191"),
        abi::branch_hi(&encoding_error),
        abi::label(&format!("{symbol}_line_four_second_ok")),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::add_immediate("x1", abi::stack_pointer(), BYTES_OFFSET + 2),
        abi::move_immediate("x2", "Integer", "1"),
    ]);
    platform.emit_read_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&input_error),
        abi::branch_eq(&encoding_error),
        abi::load_u8("%v11", abi::stack_pointer(), BYTES_OFFSET + 2),
        abi::compare_immediate("%v11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("%v11", "191"),
        abi::branch_hi(&encoding_error),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::add_immediate("x1", abi::stack_pointer(), BYTES_OFFSET + 3),
        abi::move_immediate("x2", "Integer", "1"),
    ]);
    platform.emit_read_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&input_error),
        abi::branch_eq(&encoding_error),
        abi::load_u8("%v11", abi::stack_pointer(), BYTES_OFFSET + 3),
        abi::compare_immediate("%v11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("%v11", "191"),
        abi::branch_hi(&encoding_error),
        abi::move_immediate("%v11", "Integer", "4"),
        abi::store_u64("%v11", abi::stack_pointer(), SEQ_LEN_OFFSET),
        abi::label(&have_sequence),
        abi::load_u64("%v10", abi::stack_pointer(), LENGTH_OFFSET),
        abi::load_u64("%v11", abi::stack_pointer(), SEQ_LEN_OFFSET),
        abi::add_registers("%v12", "%v10", "%v11"),
        abi::load_u64("%v13", abi::stack_pointer(), CAPACITY_OFFSET),
        abi::compare_registers("%v12", "%v13"),
        abi::branch_gt(&grow),
        abi::branch(&grow_ok),
        abi::label(&grow),
        // Stash the soon-to-be-dead buffer (ptr + its size = old capacity) before
        // the new capacity overwrites CAPACITY_OFFSET, so it can be freed below.
        abi::store_u64("%v13", abi::stack_pointer(), OLD_CAPACITY_OFFSET),
        abi::load_u64("%v9", abi::stack_pointer(), BUFFER_OFFSET),
        abi::store_u64("%v9", abi::stack_pointer(), OLD_BUFFER_OFFSET),
        abi::add_registers("%v14", "%v13", "%v13"),
        abi::compare_registers("%v14", "%v12"),
        abi::branch_ge(&format!("{symbol}_grow_size_ok")),
        abi::move_register("%v14", "%v12"),
        abi::label(&format!("{symbol}_grow_size_ok")),
        abi::store_u64("%v14", abi::stack_pointer(), CAPACITY_OFFSET),
        abi::move_register(abi::return_register(), "%v14"),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    // The `bl _mfb_arena_free` that frees the old buffer (emitted at grow_copy_done
    // below) needs its branch relocation; order in the table is irrelevant.
    relocations.push(internal_branch(symbol, ARENA_FREE_SYMBOL));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&format!("{symbol}_grow_alloc_ok")),
        abi::branch(&alloc_error),
        abi::label(&format!("{symbol}_grow_alloc_ok")),
        // `bl _mfb_arena_alloc` clobbers x10 (the live byte count to copy), so
        // reload the length from the stack rather than trusting the register
        // across the call — otherwise the copy loop runs off the new buffer.
        abi::load_u64("%v10", abi::stack_pointer(), LENGTH_OFFSET),
        abi::load_u64("%v12", abi::stack_pointer(), BUFFER_OFFSET),
        abi::move_register("%v14", "x1"),
        abi::move_immediate("%v15", "Integer", "0"),
        abi::label(&grow_copy_loop),
        abi::compare_registers("%v15", "%v10"),
        abi::branch_eq(&grow_copy_done),
        abi::load_u8("%v16", "%v12", 0),
        abi::store_u8("%v16", "%v14", 0),
        abi::add_immediate("%v12", "%v12", 1),
        abi::add_immediate("%v14", "%v14", 1),
        abi::add_immediate("%v15", "%v15", 1),
        abi::branch(&grow_copy_loop),
        abi::label(&grow_copy_done),
        abi::store_u64("x1", abi::stack_pointer(), BUFFER_OFFSET),
        // The old buffer's bytes are now copied into the new one and dead — return
        // it to the free-list. arena_free clobbers x0/x1/x9–x16; grow_ok reloads
        // everything it needs from the stack, so nothing live is lost.
        abi::load_u64("x0", abi::stack_pointer(), OLD_BUFFER_OFFSET),
        abi::load_u64("x1", abi::stack_pointer(), OLD_CAPACITY_OFFSET),
        abi::branch_link(ARENA_FREE_SYMBOL),
        abi::label(&grow_ok),
        abi::load_u64("%v10", abi::stack_pointer(), LENGTH_OFFSET),
        abi::load_u64("%v12", abi::stack_pointer(), BUFFER_OFFSET),
        abi::add_registers("%v12", "%v12", "%v10"),
        abi::add_immediate("%v13", abi::stack_pointer(), BYTES_OFFSET),
        abi::load_u64("%v11", abi::stack_pointer(), SEQ_LEN_OFFSET),
        abi::label(&append_loop),
        abi::compare_immediate("%v11", "0"),
        abi::branch_eq(&append_done),
        abi::load_u8("%v14", "%v13", 0),
        abi::store_u8("%v14", "%v12", 0),
        abi::add_immediate("%v12", "%v12", 1),
        abi::add_immediate("%v13", "%v13", 1),
        abi::subtract_immediate("%v11", "%v11", 1),
        abi::branch(&append_loop),
        abi::label(&append_done),
        abi::load_u64("%v10", abi::stack_pointer(), LENGTH_OFFSET),
        abi::load_u64("%v11", abi::stack_pointer(), SEQ_LEN_OFFSET),
        abi::add_registers("%v10", "%v10", "%v11"),
        abi::store_u64("%v10", abi::stack_pointer(), LENGTH_OFFSET),
        abi::branch(&read_loop),
        abi::label(&format!("{symbol}_read_eof")),
        abi::load_u64("%v10", abi::stack_pointer(), LENGTH_OFFSET),
        abi::compare_immediate("%v10", "0"),
        abi::branch_eq(&eof_error),
        abi::branch(&trim_cr),
        abi::label(&trim_cr),
        abi::load_u64("%v10", abi::stack_pointer(), LENGTH_OFFSET),
        abi::compare_immediate("%v10", "0"),
        abi::branch_eq(&format!("{symbol}_result_alloc")),
        abi::load_u64("%v12", abi::stack_pointer(), BUFFER_OFFSET),
        abi::subtract_immediate("%v13", "%v10", 1),
        abi::add_registers("%v12", "%v12", "%v13"),
        abi::load_u8("%v14", "%v12", 0),
        abi::compare_immediate("%v14", "13"),
        abi::branch_ne(&format!("{symbol}_result_alloc")),
        abi::subtract_immediate("%v10", "%v10", 1),
        abi::store_u64("%v10", abi::stack_pointer(), LENGTH_OFFSET),
        abi::label(&format!("{symbol}_result_alloc")),
        abi::load_u64("%v10", abi::stack_pointer(), LENGTH_OFFSET),
        abi::add_immediate(abi::return_register(), "%v10", 9),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&result_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&result_alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), RESULT_OFFSET),
        abi::load_u64("%v10", abi::stack_pointer(), LENGTH_OFFSET),
        abi::store_u64("%v10", "x1", 0),
        abi::add_immediate("%v11", "x1", 8),
        abi::load_u64("%v12", abi::stack_pointer(), BUFFER_OFFSET),
        abi::label(&result_copy_loop),
        abi::compare_immediate("%v10", "0"),
        abi::branch_eq(&result_copy_done),
        abi::load_u8("%v13", "%v12", 0),
        abi::store_u8("%v13", "%v11", 0),
        abi::add_immediate("%v11", "%v11", 1),
        abi::add_immediate("%v12", "%v12", 1),
        abi::subtract_immediate("%v10", "%v10", 1),
        abi::branch(&result_copy_loop),
        abi::label(&result_copy_done),
        abi::store_u8("x31", "%v11", 0),
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), RESULT_OFFSET),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&output_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUTPUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_OUTPUT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::branch(&done));
    instructions.extend([
        abi::label(&eof_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_EOF_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_EOF_SYMBOL, &mut instructions, &mut relocations);
    instructions.extend([
        abi::branch(&done),
        abi::label(&input_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INPUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INPUT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&encoding_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_ENCODING_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ENCODING_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&alloc_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::label(&done));
    if !with_prompt {
        emit_restore_stdin_terminal(
            symbol,
            platform_imports,
            platform,
            &mut instructions,
            &mut relocations,
            &terminal_slots,
        )?;
    }
    instructions.push(abi::return_());
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], FRAME_SIZE);
    Ok((frame, instructions, relocations, stack_slots))
}
