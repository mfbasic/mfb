use super::*;

fn emit_append_to_stdout_buffer(
    ctx: &mut EmitCtx,
    src: &str,
    len: &str,
    tag: &str,
    write_error: &str,
) -> Result<(), String> {
    let cap = OUT_BUFFER_CAPACITY.to_string();
    let sink = BufferSink {
        state_reg: ARENA_STATE_REGISTER,
        buf_ptr_off: ARENA_OUT_PTR_OFFSET,
        filled_off: ARENA_OUT_FILLED_OFFSET,
        drain_symbol: STDOUT_DRAIN_SYMBOL,
        drain_handle: None,
        cap: &cap,
        prefix: "buf",
        v: [
            "%v20", "%v21", "%v22", "%v23", "%v24", "%v25", "%v26", "%v27", "%v28",
        ],
        fd: None,
    };
    emit_append_to_buffer(ctx, src, len, tag, write_error, &sink)
}

pub(super) fn lower_io_write_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    stderr: bool,
    append_newline: bool,
    term_state_offset: Option<usize>,
) -> HelperResult {
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    // plan-35-B: while TUI mode is on, stdout writes mutate the shadow grid's back
    // buffer instead of the terminal (the mirror of app mode's `active`-gated grid
    // routing). Only stdout (not stderr) is retained, and only when the program
    // uses `term::` (`term_state_offset` is `Some`) — so a non-term program's
    // `io::write` is byte-identical. The grid path is emitted just before `done`.
    let grid_path = format!("{symbol}_grid");
    // The String object arrives in the return register. Capture it into a vreg
    // that stays live across the active-check branch: the check's own load may be
    // allocated into the return register (rax on x86), clobbering the pointer
    // before the grid path reads it — so save it here and restore the return
    // register for the fall-through (non-TUI) path.
    let strobj_vreg = "%v31";
    let grid_target = if let Some(tso) = term_state_offset.filter(|_| !stderr) {
        instructions.push(abi::move_register(strobj_vreg, abi::return_register()));
        instructions.push(abi::load_u64(
            "%v29",
            ARENA_STATE_REGISTER,
            tso + TERM_STATE_ACTIVE_OFFSET,
        ));
        instructions.push(abi::compare_immediate("%v29", "0"));
        instructions.push(abi::branch_ne(&grid_path));
        instructions.push(abi::move_register(abi::return_register(), strobj_vreg));
        Some(tso)
    } else {
        None
    };
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
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: &mut instructions,
                relocations: &mut relocations,
            },
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
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
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
    emit_transfer_loop_tail(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        abi::return_register(),
        write_uses_raw_syscall(platform),
        "%v13",
        "%v14",
        &direct_loop,
        &write_error,
    )?;
    instructions.push(abi::label(&direct_written));
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
        emit_transfer_loop_tail(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: &mut instructions,
                relocations: &mut relocations,
            },
            abi::return_register(),
            write_uses_raw_syscall(platform),
            "%v13",
            "%v14",
            &newline_loop,
            &write_error,
        )?;
        instructions.push(abi::label(&newline_written));
    }
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&write_error),
    ]);
    raise_error_into(
        symbol,
        "ErrWriteFailed",
        &mut instructions,
        &mut relocations,
    );
    if let Some(tso) = grid_target {
        // TUI-active stdout: route the string (still in the return register) into
        // the shadow-grid back buffer. No terminal write happens here; the frame
        // is shown when the program calls `term::sync`.
        instructions.push(abi::label(&grid_path));
        term_grid::emit_grid_write(
            symbol,
            tso,
            strobj_vreg,
            append_newline,
            &mut instructions,
            &mut relocations,
        );
        instructions.push(abi::move_immediate(
            RESULT_TAG_REGISTER,
            "Integer",
            RESULT_OK_TAG,
        ));
        instructions.push(abi::branch(&done));
    }
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
) -> HelperResult {
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
    // buffering is off.
    //
    // There used to be a `stderr: bool` parameter gating this drain, on the
    // reasoning that stderr is never buffered and so has nothing to flush. No
    // caller ever passed `true` — `io::flush()` is stdout-only — so the guarded
    // and unguarded halves were the same program (bug-326-A23).
    instructions.push(abi::branch_link(STDOUT_DRAIN_SYMBOL));
    relocations.push(internal_branch(symbol, STDOUT_DRAIN_SYMBOL));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&output_error),
    ]);
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&output_error),
    ]);
    raise_error_into(
        symbol,
        "ErrWriteFailed",
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
pub(super) fn lower_io_is_buffered_helper(symbol: &str, app_mode: bool) -> HelperResult {
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
pub(super) fn lower_io_set_buffered_helper(symbol: &str, app_mode: bool) -> HelperResult {
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
            abi::store_u64(abi::ZERO, ARENA_STATE_REGISTER, ARENA_OUT_ENABLED_OFFSET),
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
