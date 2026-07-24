use super::*;

pub(super) struct TerminalModeSlots {
    pub(super) active: usize,
    pub(super) saved_tag: usize,
    pub(super) saved_value: usize,
    pub(super) saved_message: usize,
    pub(super) original: usize,
    pub(super) modified: usize,
}

/// Configure stdin's line discipline for a single-key read: `tcgetattr` the
/// current `termios` into `slots.original`, copy it to `slots.modified` with
/// `ECHO`/`ICANON` optionally cleared and `VMIN=1`/`VTIME=0` set, and
/// `tcsetattr` the modified copy. `slots.active` records whether the change was
/// applied (so the paired restore knows to undo it). All `slots` offsets are
/// relative to `base_register` — `abi::stack_pointer()` for the transient
/// per-read toggle in the read helpers, or `ARENA_STATE_REGISTER` for
/// `term::on`'s persistent console raw mode (bug-149), which parks the buffers
/// in the term-state region rather than a read-scoped stack frame.
pub(super) fn emit_configure_stdin_terminal(
    ctx: &mut EmitCtx,
    slots: &TerminalModeSlots,
    base_register: &str,
    disable_echo: bool,
    disable_canonical: bool,
    error_label: &str,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    let skip = format!("{symbol}_terminal_mode_skip");
    ctx.instructions.extend([
        abi::store_u64(abi::ZERO, base_register, slots.active),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
    ]);
    platform.emit_libc_call(
        "isatty",
        symbol,
        platform_imports,
        ctx.instructions,
        ctx.relocations,
    )?;
    ctx.instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(&skip),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::add_immediate(abi::ARG[1], base_register, slots.original),
    ]);
    platform.emit_libc_call(
        "tcgetattr",
        symbol,
        platform_imports,
        ctx.instructions,
        ctx.relocations,
    )?;
    ctx.instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(error_label),
        abi::move_immediate("%v9", "Integer", "1"),
        abi::store_u64("%v9", base_register, slots.active),
    ]);

    platform.emit_apply_raw_mode(
        base_register,
        slots.original,
        slots.modified,
        disable_echo,
        disable_canonical,
        ctx.instructions,
    );

    ctx.instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
        abi::add_immediate(abi::ARG[2], base_register, slots.modified),
    ]);
    platform.emit_libc_call(
        "tcsetattr",
        symbol,
        platform_imports,
        ctx.instructions,
        ctx.relocations,
    )?;
    ctx.instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(error_label),
        abi::label(&skip),
    ]);
    Ok(())
}

pub(super) fn emit_restore_stdin_terminal(
    ctx: &mut EmitCtx,
    slots: &TerminalModeSlots,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    let restored = format!("{symbol}_terminal_mode_restored");
    let restore_failed = format!("{symbol}_terminal_mode_restore_failed");
    ctx.instructions.extend([
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
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
        abi::add_immediate(abi::ARG[2], abi::stack_pointer(), slots.original),
    ]);
    platform.emit_libc_call(
        "tcsetattr",
        symbol,
        platform_imports,
        ctx.instructions,
        ctx.relocations,
    )?;
    ctx.instructions.extend([
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
    push_error_message_address(symbol, ERR_INPUT_SYMBOL, ctx.instructions, ctx.relocations);
    ctx.instructions
        .push(abi::label(&format!("{symbol}_terminal_mode_restore_done")));
    Ok(())
}

/// Emit a guarded `tcsetattr` that swaps stdin's line discipline to one of
/// `term::on`'s persistent save buffers while TUI single-key (raw) mode is
/// active (bug-149). `line_mode = true` selects the saved cooked buffer (a line
/// read waits for Return and echoes as usual); `line_mode = false` re-applies
/// the raw buffer after the read. A no-op when the raw-active flag is 0 (stdin
/// was never put into raw mode — not a tty, or no `term::on`), so a program that
/// never enters TUI mode keeps the exact pre-bug-149 behavior. The `termios`
/// buffers live in the term-state region (addressed off `ARENA_STATE_REGISTER`),
/// so nothing is read from a read-scoped stack frame. When `preserve_result` is
/// set the `Result` registers are parked across the `tcsetattr` call (needed for
/// the post-read re-apply, which runs after the read result is already staged).
pub(super) fn emit_console_raw_line_mode(
    ctx: &mut EmitCtx,
    term_state_offset: usize,
    line_mode: bool,
    preserve_result: bool,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    let tag = if line_mode { "line" } else { "raw" };
    let skip = format!("{symbol}_console_{tag}_mode_skip");
    let buffer_offset = if line_mode {
        term_state_offset + TERM_STATE_COOKED_TERMIOS_OFFSET
    } else {
        term_state_offset + TERM_STATE_RAW_TERMIOS_OFFSET
    };
    ctx.instructions.extend([
        abi::load_u64(
            "%v9",
            ARENA_STATE_REGISTER,
            term_state_offset + TERM_STATE_RAW_ACTIVE_OFFSET,
        ),
        abi::compare_immediate("%v9", "0"),
        abi::branch_eq(&skip),
    ]);
    if preserve_result {
        ctx.instructions.extend([
            abi::move_register("%v20", RESULT_TAG_REGISTER),
            abi::move_register("%v21", RESULT_VALUE_REGISTER),
            abi::move_register("%v22", RESULT_ERROR_MESSAGE_REGISTER),
        ]);
    }
    ctx.instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
        abi::add_immediate(abi::ARG[2], ARENA_STATE_REGISTER, buffer_offset),
    ]);
    platform.emit_libc_call(
        "tcsetattr",
        symbol,
        platform_imports,
        ctx.instructions,
        ctx.relocations,
    )?;
    if preserve_result {
        ctx.instructions.extend([
            abi::move_register(RESULT_TAG_REGISTER, "%v20"),
            abi::move_register(RESULT_VALUE_REGISTER, "%v21"),
            abi::move_register(RESULT_ERROR_MESSAGE_REGISTER, "%v22"),
        ]);
    }
    ctx.instructions.push(abi::label(&skip));
    Ok(())
}

pub(super) fn lower_io_is_terminal_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    fd: u8,
) -> HelperResult {
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
