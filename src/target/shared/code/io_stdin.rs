use super::*;

pub(super) fn lower_io_poll_input_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    app_mode: bool,
) -> HelperResult {
    const POLLIN_PACKED_FD0: &str = "4294967296";
    const FRAME_SIZE: usize = 48;
    const POLLFD_OFFSET: usize = 8;
    const TIMEOUT_OFFSET: usize = 32;

    let poll_error = format!("{symbol}_poll_error");
    let poll_eintr_check = format!("{symbol}_poll_eintr");
    let poll_ready = format!("{symbol}_poll_ready");
    let os_poll = format!("{symbol}_os_poll");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    // Save the caller's timeout before the log-ready check clobbers x0.
    instructions.push(abi::store_u64(
        abi::return_register(),
        abi::stack_pointer(),
        TIMEOUT_OFFSET,
    ));
    // plan-15 §4.4: a byte already staged for this thread in the broadcast log is
    // invisible to `poll(fd 0)`, so check the log first (ready => report TRUE) and
    // only `poll(fd 0)` when the log has nothing for us. App mode reads the window
    // pipe (no broadcast log), so it skips straight to `poll(fd 0)`.
    if !app_mode {
        emit_stdin_poll_ready_check(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: &mut instructions,
                relocations: &mut relocations,
            },
            &poll_ready,
            &os_poll,
        )?;
    }
    instructions.extend([
        abi::label(&os_poll),
        abi::move_immediate("%v9", "Integer", POLLIN_PACKED_FD0),
        abi::store_u64("%v9", abi::stack_pointer(), POLLFD_OFFSET),
    ]);

    instructions.push(abi::load_u64(
        abi::ARG[2],
        abi::stack_pointer(),
        TIMEOUT_OFFSET,
    ));

    instructions.extend([
        abi::add_immediate(abi::return_register(), abi::stack_pointer(), POLLFD_OFFSET),
        abi::move_immediate(abi::ARG[1], "Integer", "1"),
    ]);
    platform.emit_poll_input(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&poll_eintr_check),
        abi::branch_gt(&poll_ready),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&poll_ready),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&poll_eintr_check),
    ]);
    // bug-314 H1: a negative return used to go straight to ErrInput. Every other
    // blocking primitive retries EINTR -- read/write/seek (bug-62) and net poll
    // (bug-115) -- but fd-0 poll was left unwrapped, so any handled signal
    // (SIGWINCH in a TUI, SIGCHLD, the console SIGINT/SIGTERM handler where the
    // program continues) interrupting a blocked `io::pollInput()` surfaced as a
    // spurious ErrInput instead of ready/not-ready. Retry at `os_poll`, which
    // re-arms the pollfd from scratch.
    emit_eintr_retry_or_error(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        abi::return_register(),
        false,
        &os_poll,
        &poll_error,
    )?;
    instructions.extend([
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

/// Emit one EINTR-guarded UTF-8 continuation-byte `read` for `io::readChar` /
/// `io::readLine` (bug-97.2). A signal delivered mid-multibyte-sequence returns
/// `-1`/`EINTR`; before this the bare `compare/branch_lt(<input_error>)` treated
/// that as a fatal input error (discarding `readLine`'s partial line). This
/// replicates the lead-read guard: `retry_label` re-issues the identical 1-byte
/// read into `stack[byte_offset]`, and the guard leaves the `cmp x0, 0` flags
/// live so the caller's follow-on `branch_eq(<encoding_error>)` (a 0-byte read
/// mid-sequence is a truncated sequence) fuses on every backend. Reads always go
/// through libc, so the guard uses the `errno`-accessor convention (both read
/// helpers already import it for the lead read).
fn emit_continuation_read(
    ctx: &mut EmitCtx,
    app_mode: bool,
    byte_offset: usize,
    retry_label: &str,
    resume_label: &str,
    input_error: &str,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    // plan-15: in console mode the continuation byte comes from the stdin broadcast
    // log (`_mfb_rt_stdin_next_byte`); in app mode it is a direct per-byte read of
    // the window pipe. A continuation byte from an unsubscribed thread is the same
    // ErrInvalidContext as the lead byte, routed to the helper's shared handler.
    emit_stdin_byte_read(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        app_mode,
        byte_offset,
        retry_label,
        resume_label,
        input_error,
        &format!("{symbol}_invalid_context"),
    )
}

/// Emit one stdin byte read for a read helper, choosing the source by mode. In
/// console mode (`!app_mode`) the byte comes from the stdin broadcast log
/// (`_mfb_rt_stdin_next_byte`, plan-15). In app mode stdin is the window input
/// pipe, not fd 0, so the log is not built — keep the direct per-byte
/// `read(0,…,1)` + EINTR guard. Both paths push `retry_label` (the loop/retry head)
/// and leave the `x0 vs 0` flags live for the caller's follow-on `branch_eq`.
fn emit_stdin_byte_read(
    ctx: &mut EmitCtx,
    app_mode: bool,
    byte_offset: usize,
    retry_label: &str,
    resume_label: &str,
    input_error: &str,
    invalid_context: &str,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    if app_mode {
        ctx.instructions.extend([
            abi::label(retry_label),
            abi::move_immediate(abi::return_register(), "Integer", "0"),
            abi::add_immediate(abi::ARG[1], abi::stack_pointer(), byte_offset),
            abi::move_immediate(abi::ARG[2], "Integer", "1"),
        ]);
        platform.emit_read_file(symbol, platform_imports, ctx.instructions, ctx.relocations)?;
        ctx.instructions
            .push(abi::compare_immediate(abi::return_register(), "0"));
        emit_single_op_eintr_guard(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: ctx.instructions,
                relocations: ctx.relocations,
            },
            retry_label,
            resume_label,
            input_error,
        )?;
    } else {
        ctx.instructions.push(abi::label(retry_label));
        emit_stdin_next_byte(
            symbol,
            byte_offset,
            retry_label,
            input_error,
            invalid_context,
            ctx.instructions,
            ctx.relocations,
        );
    }
    Ok(())
}

pub(super) fn lower_io_read_byte_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    app_mode: bool,
) -> HelperResult {
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
    let invalid_context = format!("{symbol}_invalid_context");
    let read_retry = format!("{symbol}_read_retry");
    let read_resume = format!("{symbol}_read_resume");
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
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        &terminal_slots,
        abi::stack_pointer(),
        true,
        true,
        &input_error,
    )?;
    // plan-15: read the byte from the stdin broadcast log. EINTR/blocking are
    // handled inside `_mfb_rt_stdin_next_byte`; a 0-byte return is EOF.
    emit_stdin_byte_read(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        app_mode,
        BYTE_OFFSET,
        &read_retry,
        &read_resume,
        &input_error,
        &invalid_context,
    )?;
    instructions.extend([
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
    instructions.extend([
        abi::branch(&done),
        abi::label(&invalid_context),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INVALID_CONTEXT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INVALID_CONTEXT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::label(&done));
    emit_restore_stdin_terminal(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        &terminal_slots,
    )?;
    instructions.push(abi::return_());
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], FRAME_SIZE);
    Ok((frame, instructions, relocations, stack_slots))
}

pub(super) fn lower_io_read_char_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    app_mode: bool,
) -> HelperResult {
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
    let invalid_context = format!("{symbol}_invalid_context");
    let encoding_error = format!("{symbol}_encoding_error");
    let alloc_error = format!("{symbol}_alloc_error");
    let read_retry = format!("{symbol}_read_retry");
    let read_resume = format!("{symbol}_read_resume");
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
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        &terminal_slots,
        abi::stack_pointer(),
        true,
        true,
        &input_error,
    )?;
    // plan-15: read the lead byte from the stdin broadcast log; a 0-byte return is
    // EOF. EINTR/blocking are handled inside `_mfb_rt_stdin_next_byte`.
    emit_stdin_byte_read(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        app_mode,
        BYTES_OFFSET,
        &read_retry,
        &read_resume,
        &input_error,
        &invalid_context,
    )?;
    instructions.extend([
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
    ]);
    emit_continuation_read(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        app_mode,
        BYTES_OFFSET + 1,
        &format!("{symbol}_cont1_retry"),
        &format!("{symbol}_cont1_resume"),
        &input_error,
    )?;
    instructions.extend([
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
    ]);
    emit_continuation_read(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        app_mode,
        BYTES_OFFSET + 1,
        &format!("{symbol}_cont2_retry"),
        &format!("{symbol}_cont2_resume"),
        &input_error,
    )?;
    instructions.extend([
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
    ]);
    emit_continuation_read(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        app_mode,
        BYTES_OFFSET + 2,
        &format!("{symbol}_cont3_retry"),
        &format!("{symbol}_cont3_resume"),
        &input_error,
    )?;
    instructions.extend([
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
    ]);
    emit_continuation_read(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        app_mode,
        BYTES_OFFSET + 1,
        &format!("{symbol}_cont4_retry"),
        &format!("{symbol}_cont4_resume"),
        &input_error,
    )?;
    instructions.extend([
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
    ]);
    emit_continuation_read(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        app_mode,
        BYTES_OFFSET + 2,
        &format!("{symbol}_cont5_retry"),
        &format!("{symbol}_cont5_resume"),
        &input_error,
    )?;
    instructions.extend([
        abi::branch_eq(&encoding_error),
        abi::load_u8("%v11", abi::stack_pointer(), BYTES_OFFSET + 2),
        abi::compare_immediate("%v11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("%v11", "191"),
        abi::branch_hi(&encoding_error),
    ]);
    emit_continuation_read(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        app_mode,
        BYTES_OFFSET + 3,
        &format!("{symbol}_cont6_retry"),
        &format!("{symbol}_cont6_resume"),
        &input_error,
    )?;
    instructions.extend([
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
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::store_u64(abi::RET[1], abi::stack_pointer(), RESULT_OFFSET),
        abi::load_u64("%v10", abi::stack_pointer(), LEN_OFFSET),
        abi::store_u64("%v10", abi::RET[1], 0),
        abi::add_immediate("%v11", abi::RET[1], 8),
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
        abi::store_u8(abi::ZERO, "%v11", 0),
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
    instructions.extend([
        abi::branch(&done),
        abi::label(&invalid_context),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INVALID_CONTEXT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INVALID_CONTEXT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::label(&done));
    emit_restore_stdin_terminal(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
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
    // `Some(term_state_offset)` for a console build that also uses `term::`
    // (bug-149): `io::input`/`io::readLine` bracket their line read with a
    // cooked-mode restore while TUI single-key mode is active. `None` in app
    // mode (no tty) or when the program never uses `term::`.
    console_term_state: Option<usize>,
) -> HelperResult {
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
    let read_resume = format!("{symbol}_read_resume");
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
    let invalid_context = format!("{symbol}_invalid_context");
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
            "%v41",
            "%v42",
            &prompt_loop,
            &output_error,
        )?;
        instructions.push(abi::label(&prompt_flush));
    }
    // While console TUI single-key mode is active (`term::on`), stdin is in raw
    // mode; restore the saved cooked line discipline so this read waits for
    // Return and echoes (bug-149). A no-op otherwise. Must precede the read
    // helper's own `emit_configure_stdin_terminal` so its `tcgetattr` snapshots
    // the cooked flags.
    if let Some(term_state_offset) = console_term_state {
        emit_console_raw_line_mode(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: &mut instructions,
                relocations: &mut relocations,
            },
            term_state_offset,
            true,
            false,
        )?;
    }
    if !with_prompt {
        emit_configure_stdin_terminal(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: &mut instructions,
                relocations: &mut relocations,
            },
            &terminal_slots,
            abi::stack_pointer(),
            true,
            false,
            &input_error,
        )?;
    }
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "32"),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::store_u64(abi::RET[1], abi::stack_pointer(), BUFFER_OFFSET),
        abi::move_immediate("%v10", "Integer", "32"),
        abi::store_u64("%v10", abi::stack_pointer(), CAPACITY_OFFSET),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), LENGTH_OFFSET),
    ]);
    // plan-15: each line byte comes from the stdin broadcast log in console mode (or
    // the window pipe in app mode). `read_loop` is the per-byte loop head (pushed by
    // the helper); EINTR/blocking are handled inside the reader, and a 0-byte return
    // is EOF.
    emit_stdin_byte_read(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        app_mode,
        BYTES_OFFSET,
        &read_loop,
        &read_resume,
        &input_error,
        &invalid_context,
    )?;
    instructions.extend([
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
    ]);
    emit_continuation_read(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        app_mode,
        BYTES_OFFSET + 1,
        &format!("{symbol}_cont1_retry"),
        &format!("{symbol}_cont1_resume"),
        &input_error,
    )?;
    instructions.extend([
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
    ]);
    emit_continuation_read(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        app_mode,
        BYTES_OFFSET + 1,
        &format!("{symbol}_cont2_retry"),
        &format!("{symbol}_cont2_resume"),
        &input_error,
    )?;
    instructions.extend([
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
    ]);
    emit_continuation_read(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        app_mode,
        BYTES_OFFSET + 2,
        &format!("{symbol}_cont3_retry"),
        &format!("{symbol}_cont3_resume"),
        &input_error,
    )?;
    instructions.extend([
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
    ]);
    emit_continuation_read(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        app_mode,
        BYTES_OFFSET + 1,
        &format!("{symbol}_cont4_retry"),
        &format!("{symbol}_cont4_resume"),
        &input_error,
    )?;
    instructions.extend([
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
    ]);
    emit_continuation_read(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        app_mode,
        BYTES_OFFSET + 2,
        &format!("{symbol}_cont5_retry"),
        &format!("{symbol}_cont5_resume"),
        &input_error,
    )?;
    instructions.extend([
        abi::branch_eq(&encoding_error),
        abi::load_u8("%v11", abi::stack_pointer(), BYTES_OFFSET + 2),
        abi::compare_immediate("%v11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("%v11", "191"),
        abi::branch_hi(&encoding_error),
    ]);
    emit_continuation_read(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        app_mode,
        BYTES_OFFSET + 3,
        &format!("{symbol}_cont6_retry"),
        &format!("{symbol}_cont6_resume"),
        &input_error,
    )?;
    instructions.extend([
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
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
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
        abi::move_register("%v14", abi::RET[1]),
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
        abi::store_u64(abi::RET[1], abi::stack_pointer(), BUFFER_OFFSET),
        // The old buffer's bytes are now copied into the new one and dead — return
        // it to the free-list. arena_free clobbers x0/x1/x9–x16; grow_ok reloads
        // everything it needs from the stack, so nothing live is lost.
        abi::load_u64(abi::ARG[0], abi::stack_pointer(), OLD_BUFFER_OFFSET),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), OLD_CAPACITY_OFFSET),
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
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&result_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&result_alloc_ok),
        abi::store_u64(abi::RET[1], abi::stack_pointer(), RESULT_OFFSET),
        abi::load_u64("%v10", abi::stack_pointer(), LENGTH_OFFSET),
        abi::store_u64("%v10", abi::RET[1], 0),
        abi::add_immediate("%v11", abi::RET[1], 8),
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
        abi::store_u8(abi::ZERO, "%v11", 0),
        // The working line buffer is now fully copied into the result String and
        // is dead. Return it to the free-list before returning Ok, so a
        // line-processing loop (`WHILE ... io::readLine ...`) doesn't leak
        // max(32, ~2×line) bytes of arena on every call — an unbounded growth
        // that scope-drop (user values only) never reclaims (bug-95).
        // `arena_free` clobbers x0/x1/x9–x16; the result pointer/tag are reloaded
        // from the stack immediately afterward, so nothing live is lost.
        abi::load_u64(abi::ARG[0], abi::stack_pointer(), BUFFER_OFFSET),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), CAPACITY_OFFSET),
        abi::branch_link(ARENA_FREE_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_FREE_SYMBOL));
    instructions.extend([
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
    instructions.extend([
        abi::branch(&done),
        abi::label(&invalid_context),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INVALID_CONTEXT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INVALID_CONTEXT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::label(&done));
    if !with_prompt {
        emit_restore_stdin_terminal(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: &mut instructions,
                relocations: &mut relocations,
            },
            &terminal_slots,
        )?;
    }
    // Re-apply raw single-key mode after the line read so a `pollInput` +
    // `readChar` TUI loop resumes seeing bare keypresses (bug-149). Guarded by
    // the raw-active flag and preserves the staged `Result` registers across the
    // `tcsetattr` call. A no-op outside console TUI mode.
    if let Some(term_state_offset) = console_term_state {
        emit_console_raw_line_mode(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: &mut instructions,
                relocations: &mut relocations,
            },
            term_state_offset,
            false,
            true,
        )?;
    }
    instructions.push(abi::return_());
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], FRAME_SIZE);
    Ok((frame, instructions, relocations, stack_slots))
}
