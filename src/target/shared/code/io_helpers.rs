use super::*;

pub(super) fn lower_io_write_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    stderr: bool,
    append_newline: bool,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(16)];
    let mut relocations = Vec::new();
    if platform.preserves_link_register_in_runtime_helpers() {
        instructions.push(abi::store_u64(
            abi::link_register(),
            abi::stack_pointer(),
            0,
        ));
    }
    instructions.extend([
        abi::load_u64(abi::string_length_register(), abi::return_register(), 0),
        abi::add_immediate(abi::string_data_register(), abi::return_register(), 8),
        abi::move_immediate(
            abi::return_register(),
            "Integer",
            if stderr { "2" } else { "1" },
        ),
    ]);
    platform.emit_write(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    let write_error = format!("{symbol}_write_error");
    let done = format!("{symbol}_done");
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&write_error),
    ]);
    if append_newline {
        instructions.extend([
            abi::move_immediate(abi::newline_scratch_register(), "Integer", "10"),
            abi::store_u64(abi::newline_scratch_register(), abi::stack_pointer(), 8),
            abi::move_immediate(
                abi::return_register(),
                "Integer",
                if stderr { "2" } else { "1" },
            ),
            abi::add_immediate(abi::string_data_register(), abi::stack_pointer(), 8),
            abi::move_immediate(abi::string_length_register(), "Integer", "1"),
        ]);
        platform.emit_write(
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_lt(&write_error),
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
    if platform.preserves_link_register_in_runtime_helpers() {
        instructions.extend([
            abi::load_u64(abi::link_register(), abi::stack_pointer(), 0),
            abi::add_stack(16),
            abi::return_(),
        ]);
        Ok((
            CodeFrame {
                stack_size: 16,
                callee_saved: vec![abi::link_register().to_string()],
            },
            instructions,
            relocations,
        ))
    } else {
        instructions.extend([abi::add_stack(16), abi::return_()]);
        Ok((
            CodeFrame {
                stack_size: 16,
                callee_saved: Vec::new(),
            },
            instructions,
            relocations,
        ))
    }
}

pub(super) fn lower_io_flush_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    stderr: bool,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 16;
    const LR_OFFSET: usize = 0;
    const ERRNO_EINVAL: &str = "22";
    const ERRNO_ENOTSUP_DARWIN: &str = "45";
    const ERRNO_EOPNOTSUPP_LINUX: &str = "95";

    let sync_error = format!("{symbol}_sync_error");
    let ok = format!("{symbol}_ok");
    let output_error = format!("{symbol}_output_error");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    if platform.preserves_link_register_in_runtime_helpers() {
        instructions.push(abi::store_u64(
            abi::link_register(),
            abi::stack_pointer(),
            LR_OFFSET,
        ));
    }
    instructions.push(abi::move_immediate(
        abi::return_register(),
        "Integer",
        if stderr { "2" } else { "1" },
    ));
    platform.emit_sync_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&sync_error),
        abi::label(&ok),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&sync_error),
    ]);
    platform.emit_errno(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate("x9", ERRNO_EINVAL),
        abi::branch_eq(&ok),
        abi::compare_immediate("x9", ERRNO_ENOTSUP_DARWIN),
        abi::branch_eq(&ok),
        abi::compare_immediate("x9", ERRNO_EOPNOTSUPP_LINUX),
        abi::branch_eq(&ok),
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
    if platform.preserves_link_register_in_runtime_helpers() {
        instructions.extend([
            abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
            abi::add_stack(FRAME_SIZE),
            abi::return_(),
        ]);
        Ok((
            CodeFrame {
                stack_size: FRAME_SIZE,
                callee_saved: vec![abi::link_register().to_string()],
            },
            instructions,
            relocations,
        ))
    } else {
        instructions.extend([abi::add_stack(FRAME_SIZE), abi::return_()]);
        Ok((
            CodeFrame {
                stack_size: FRAME_SIZE,
                callee_saved: Vec::new(),
            },
            instructions,
            relocations,
        ))
    }
}

pub(super) fn lower_io_poll_input_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const POLLIN_PACKED_FD0: &str = "4294967296";
    const FRAME_SIZE: usize = 48;
    const POLLFD_OFFSET: usize = 8;
    const TIMEOUT_OFFSET: usize = 32;

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    if platform.preserves_link_register_in_runtime_helpers() {
        instructions.push(abi::store_u64(
            abi::link_register(),
            abi::stack_pointer(),
            0,
        ));
    }
    instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::move_immediate("x9", "Integer", POLLIN_PACKED_FD0),
        abi::store_u64("x9", abi::stack_pointer(), POLLFD_OFFSET),
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
    if platform.preserves_link_register_in_runtime_helpers() {
        instructions.extend([
            abi::load_u64(abi::link_register(), abi::stack_pointer(), 0),
            abi::add_stack(FRAME_SIZE),
            abi::return_(),
        ]);
        Ok((
            CodeFrame {
                stack_size: FRAME_SIZE,
                callee_saved: vec![abi::link_register().to_string()],
            },
            instructions,
            relocations,
        ))
    } else {
        instructions.extend([abi::add_stack(FRAME_SIZE), abi::return_()]);
        Ok((
            CodeFrame {
                stack_size: FRAME_SIZE,
                callee_saved: Vec::new(),
            },
            instructions,
            relocations,
        ))
    }
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
        abi::move_immediate("x9", "Integer", "1"),
        abi::store_u64("x9", abi::stack_pointer(), slots.active),
    ]);

    for offset in (0..termios_storage_size(platform)).step_by(8) {
        instructions.extend([
            abi::load_u64("x9", abi::stack_pointer(), slots.original + offset),
            abi::store_u64("x9", abi::stack_pointer(), slots.modified + offset),
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
            instructions.push(abi::load_u32("x9", abi::stack_pointer(), lflag_offset));
        } else {
            instructions.push(abi::load_u64("x9", abi::stack_pointer(), lflag_offset));
        }
        instructions.extend([
            abi::move_immediate("x10", "Integer", &clear_flags.to_string()),
            abi::bitwise_not("x10", "x10"),
            abi::and_registers("x9", "x9", "x10"),
        ]);
        if platform.termios_lflag_width() == 4 {
            instructions.push(abi::store_u32("x9", abi::stack_pointer(), lflag_offset));
        } else {
            instructions.push(abi::store_u64("x9", abi::stack_pointer(), lflag_offset));
        }
    }

    if disable_canonical {
        let cc_offset = slots.modified + platform.termios_cc_offset();
        instructions.extend([
            abi::move_immediate("x9", "Integer", "1"),
            abi::store_u8(
                "x9",
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
        abi::load_u64("x9", abi::stack_pointer(), slots.active),
        abi::compare_immediate("x9", "1"),
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
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 208;
    const LR_OFFSET: usize = 0;
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

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    if platform.preserves_link_register_in_runtime_helpers() {
        instructions.push(abi::store_u64(
            abi::link_register(),
            abi::stack_pointer(),
            LR_OFFSET,
        ));
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
    if platform.preserves_link_register_in_runtime_helpers() {
        instructions.extend([
            abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
            abi::add_stack(FRAME_SIZE),
            abi::return_(),
        ]);
        Ok((
            CodeFrame {
                stack_size: FRAME_SIZE,
                callee_saved: vec![abi::link_register().to_string()],
            },
            instructions,
            relocations,
        ))
    } else {
        instructions.extend([abi::add_stack(FRAME_SIZE), abi::return_()]);
        Ok((
            CodeFrame {
                stack_size: FRAME_SIZE,
                callee_saved: Vec::new(),
            },
            instructions,
            relocations,
        ))
    }
}

pub(super) fn lower_io_is_terminal_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    fd: u8,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 16;
    const LR_OFFSET: usize = 0;
    let yes = format!("{symbol}_yes");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    if platform.preserves_link_register_in_runtime_helpers() {
        instructions.push(abi::store_u64(
            abi::link_register(),
            abi::stack_pointer(),
            LR_OFFSET,
        ));
    }
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
    if platform.preserves_link_register_in_runtime_helpers() {
        instructions.extend([
            abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
            abi::add_stack(FRAME_SIZE),
            abi::return_(),
        ]);
        Ok((
            CodeFrame {
                stack_size: FRAME_SIZE,
                callee_saved: vec![abi::link_register().to_string()],
            },
            instructions,
            relocations,
        ))
    } else {
        instructions.extend([abi::add_stack(FRAME_SIZE), abi::return_()]);
        Ok((
            CodeFrame {
                stack_size: FRAME_SIZE,
                callee_saved: Vec::new(),
            },
            instructions,
            relocations,
        ))
    }
}

pub(super) fn lower_io_read_char_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    app_mode: bool,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 224;
    const LR_OFFSET: usize = 0;
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

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    if platform.preserves_link_register_in_runtime_helpers() {
        instructions.push(abi::store_u64(
            abi::link_register(),
            abi::stack_pointer(),
            LR_OFFSET,
        ));
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
        abi::load_u8("x10", abi::stack_pointer(), BYTES_OFFSET),
        abi::compare_immediate("x10", "127"),
        abi::branch_hi(&read_second),
        abi::move_immediate("x11", "Integer", "1"),
        abi::store_u64("x11", abi::stack_pointer(), LEN_OFFSET),
        abi::branch(&got_len),
        abi::label(&read_second),
        abi::compare_immediate("x10", "194"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x10", "223"),
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
        abi::load_u8("x11", abi::stack_pointer(), BYTES_OFFSET + 1),
        abi::compare_immediate("x11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x11", "191"),
        abi::branch_hi(&encoding_error),
        abi::move_immediate("x11", "Integer", "2"),
        abi::store_u64("x11", abi::stack_pointer(), LEN_OFFSET),
        abi::branch(&got_len),
        abi::label(&read_third),
        abi::compare_immediate("x10", "239"),
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
        abi::load_u8("x11", abi::stack_pointer(), BYTES_OFFSET + 1),
        abi::compare_immediate("x10", "224"),
        abi::branch_ne(&format!("{symbol}_three_not_e0")),
        abi::compare_immediate("x11", "160"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x11", "191"),
        abi::branch_hi(&encoding_error),
        abi::branch(&format!("{symbol}_three_second_ok")),
        abi::label(&format!("{symbol}_three_not_e0")),
        abi::compare_immediate("x10", "237"),
        abi::branch_ne(&format!("{symbol}_three_general")),
        abi::compare_immediate("x11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x11", "159"),
        abi::branch_hi(&encoding_error),
        abi::branch(&format!("{symbol}_three_second_ok")),
        abi::label(&format!("{symbol}_three_general")),
        abi::compare_immediate("x11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x11", "191"),
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
        abi::load_u8("x11", abi::stack_pointer(), BYTES_OFFSET + 2),
        abi::compare_immediate("x11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x11", "191"),
        abi::branch_hi(&encoding_error),
        abi::move_immediate("x11", "Integer", "3"),
        abi::store_u64("x11", abi::stack_pointer(), LEN_OFFSET),
        abi::branch(&got_len),
        abi::label(&read_fourth),
        abi::compare_immediate("x10", "240"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x10", "244"),
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
        abi::load_u8("x11", abi::stack_pointer(), BYTES_OFFSET + 1),
        abi::compare_immediate("x10", "240"),
        abi::branch_ne(&format!("{symbol}_four_not_f0")),
        abi::compare_immediate("x11", "144"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x11", "191"),
        abi::branch_hi(&encoding_error),
        abi::branch(&format!("{symbol}_four_second_ok")),
        abi::label(&format!("{symbol}_four_not_f0")),
        abi::compare_immediate("x10", "244"),
        abi::branch_ne(&format!("{symbol}_four_general")),
        abi::compare_immediate("x11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x11", "143"),
        abi::branch_hi(&encoding_error),
        abi::branch(&format!("{symbol}_four_second_ok")),
        abi::label(&format!("{symbol}_four_general")),
        abi::compare_immediate("x11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x11", "191"),
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
        abi::load_u8("x11", abi::stack_pointer(), BYTES_OFFSET + 2),
        abi::compare_immediate("x11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x11", "191"),
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
        abi::load_u8("x11", abi::stack_pointer(), BYTES_OFFSET + 3),
        abi::compare_immediate("x11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x11", "191"),
        abi::branch_hi(&encoding_error),
        abi::move_immediate("x11", "Integer", "4"),
        abi::store_u64("x11", abi::stack_pointer(), LEN_OFFSET),
        abi::label(&got_len),
        abi::load_u64("x10", abi::stack_pointer(), LEN_OFFSET),
        abi::add_immediate(abi::return_register(), "x10", 9),
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
        abi::load_u64("x10", abi::stack_pointer(), LEN_OFFSET),
        abi::store_u64("x10", "x1", 0),
        abi::add_immediate("x11", "x1", 8),
        abi::add_immediate("x12", abi::stack_pointer(), BYTES_OFFSET),
        abi::label(&copy_loop),
        abi::compare_immediate("x10", "0"),
        abi::branch_eq(&copy_done),
        abi::load_u8("x13", "x12", 0),
        abi::store_u8("x13", "x11", 0),
        abi::add_immediate("x11", "x11", 1),
        abi::add_immediate("x12", "x12", 1),
        abi::subtract_immediate("x10", "x10", 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8("x31", "x11", 0),
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
    if platform.preserves_link_register_in_runtime_helpers() {
        instructions.extend([
            abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
            abi::add_stack(FRAME_SIZE),
            abi::return_(),
        ]);
        Ok((
            CodeFrame {
                stack_size: FRAME_SIZE,
                callee_saved: vec![abi::link_register().to_string()],
            },
            instructions,
            relocations,
        ))
    } else {
        instructions.extend([abi::add_stack(FRAME_SIZE), abi::return_()]);
        Ok((
            CodeFrame {
                stack_size: FRAME_SIZE,
                callee_saved: Vec::new(),
            },
            instructions,
            relocations,
        ))
    }
}

pub(super) fn lower_io_read_line_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    with_prompt: bool,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 256;
    const LR_OFFSET: usize = 0;
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
    let prompt_ok = format!("{symbol}_prompt_ok");
    let prompt_flush = format!("{symbol}_prompt_flush");
    let prompt_flush_error = format!("{symbol}_prompt_flush_error");
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

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    if platform.preserves_link_register_in_runtime_helpers() {
        instructions.push(abi::store_u64(
            abi::link_register(),
            abi::stack_pointer(),
            LR_OFFSET,
        ));
    }
    if with_prompt {
        instructions.extend([
            abi::load_u64(abi::string_length_register(), abi::return_register(), 0),
            abi::compare_immediate(abi::string_length_register(), "0"),
            abi::branch_eq(&prompt_flush),
            abi::add_immediate(abi::string_data_register(), abi::return_register(), 8),
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
            abi::branch_lt(&output_error),
            abi::label(&prompt_flush),
            abi::move_immediate(abi::return_register(), "Integer", "1"),
        ]);
        platform.emit_sync_file(
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_lt(&prompt_flush_error),
            abi::label(&prompt_ok),
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
        abi::move_immediate("x10", "Integer", "32"),
        abi::store_u64("x10", abi::stack_pointer(), CAPACITY_OFFSET),
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
        abi::load_u8("x10", abi::stack_pointer(), BYTES_OFFSET),
        abi::compare_immediate("x10", "10"),
        abi::branch_eq(&trim_cr),
        abi::compare_immediate("x10", "127"),
        abi::branch_hi(&format!("{symbol}_multi_start")),
        abi::move_immediate("x11", "Integer", "1"),
        abi::store_u64("x11", abi::stack_pointer(), SEQ_LEN_OFFSET),
        abi::branch(&have_sequence),
        abi::label(&format!("{symbol}_multi_start")),
        abi::compare_immediate("x10", "194"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x10", "223"),
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
        abi::load_u8("x11", abi::stack_pointer(), BYTES_OFFSET + 1),
        abi::compare_immediate("x11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x11", "191"),
        abi::branch_hi(&encoding_error),
        abi::move_immediate("x11", "Integer", "2"),
        abi::store_u64("x11", abi::stack_pointer(), SEQ_LEN_OFFSET),
        abi::branch(&have_sequence),
        abi::label(&format!("{symbol}_line_read_third")),
        abi::compare_immediate("x10", "239"),
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
        abi::load_u8("x11", abi::stack_pointer(), BYTES_OFFSET + 1),
        abi::compare_immediate("x10", "224"),
        abi::branch_ne(&format!("{symbol}_line_three_not_e0")),
        abi::compare_immediate("x11", "160"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x11", "191"),
        abi::branch_hi(&encoding_error),
        abi::branch(&format!("{symbol}_line_three_second_ok")),
        abi::label(&format!("{symbol}_line_three_not_e0")),
        abi::compare_immediate("x10", "237"),
        abi::branch_ne(&format!("{symbol}_line_three_general")),
        abi::compare_immediate("x11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x11", "159"),
        abi::branch_hi(&encoding_error),
        abi::branch(&format!("{symbol}_line_three_second_ok")),
        abi::label(&format!("{symbol}_line_three_general")),
        abi::compare_immediate("x11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x11", "191"),
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
        abi::load_u8("x11", abi::stack_pointer(), BYTES_OFFSET + 2),
        abi::compare_immediate("x11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x11", "191"),
        abi::branch_hi(&encoding_error),
        abi::move_immediate("x11", "Integer", "3"),
        abi::store_u64("x11", abi::stack_pointer(), SEQ_LEN_OFFSET),
        abi::branch(&have_sequence),
        abi::label(&format!("{symbol}_line_read_fourth")),
        abi::compare_immediate("x10", "240"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x10", "244"),
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
        abi::load_u8("x11", abi::stack_pointer(), BYTES_OFFSET + 1),
        abi::compare_immediate("x10", "240"),
        abi::branch_ne(&format!("{symbol}_line_four_not_f0")),
        abi::compare_immediate("x11", "144"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x11", "191"),
        abi::branch_hi(&encoding_error),
        abi::branch(&format!("{symbol}_line_four_second_ok")),
        abi::label(&format!("{symbol}_line_four_not_f0")),
        abi::compare_immediate("x10", "244"),
        abi::branch_ne(&format!("{symbol}_line_four_general")),
        abi::compare_immediate("x11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x11", "143"),
        abi::branch_hi(&encoding_error),
        abi::branch(&format!("{symbol}_line_four_second_ok")),
        abi::label(&format!("{symbol}_line_four_general")),
        abi::compare_immediate("x11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x11", "191"),
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
        abi::load_u8("x11", abi::stack_pointer(), BYTES_OFFSET + 2),
        abi::compare_immediate("x11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x11", "191"),
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
        abi::load_u8("x11", abi::stack_pointer(), BYTES_OFFSET + 3),
        abi::compare_immediate("x11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x11", "191"),
        abi::branch_hi(&encoding_error),
        abi::move_immediate("x11", "Integer", "4"),
        abi::store_u64("x11", abi::stack_pointer(), SEQ_LEN_OFFSET),
        abi::label(&have_sequence),
        abi::load_u64("x10", abi::stack_pointer(), LENGTH_OFFSET),
        abi::load_u64("x11", abi::stack_pointer(), SEQ_LEN_OFFSET),
        abi::add_registers("x12", "x10", "x11"),
        abi::load_u64("x13", abi::stack_pointer(), CAPACITY_OFFSET),
        abi::compare_registers("x12", "x13"),
        abi::branch_gt(&grow),
        abi::branch(&grow_ok),
        abi::label(&grow),
        // Stash the soon-to-be-dead buffer (ptr + its size = old capacity) before
        // the new capacity overwrites CAPACITY_OFFSET, so it can be freed below.
        abi::store_u64("x13", abi::stack_pointer(), OLD_CAPACITY_OFFSET),
        abi::load_u64("x9", abi::stack_pointer(), BUFFER_OFFSET),
        abi::store_u64("x9", abi::stack_pointer(), OLD_BUFFER_OFFSET),
        abi::add_registers("x14", "x13", "x13"),
        abi::compare_registers("x14", "x12"),
        abi::branch_ge(&format!("{symbol}_grow_size_ok")),
        abi::move_register("x14", "x12"),
        abi::label(&format!("{symbol}_grow_size_ok")),
        abi::store_u64("x14", abi::stack_pointer(), CAPACITY_OFFSET),
        abi::move_register(abi::return_register(), "x14"),
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
        abi::load_u64("x10", abi::stack_pointer(), LENGTH_OFFSET),
        abi::load_u64("x12", abi::stack_pointer(), BUFFER_OFFSET),
        abi::move_register("x14", "x1"),
        abi::move_immediate("x15", "Integer", "0"),
        abi::label(&grow_copy_loop),
        abi::compare_registers("x15", "x10"),
        abi::branch_eq(&grow_copy_done),
        abi::load_u8("x16", "x12", 0),
        abi::store_u8("x16", "x14", 0),
        abi::add_immediate("x12", "x12", 1),
        abi::add_immediate("x14", "x14", 1),
        abi::add_immediate("x15", "x15", 1),
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
        abi::load_u64("x10", abi::stack_pointer(), LENGTH_OFFSET),
        abi::load_u64("x12", abi::stack_pointer(), BUFFER_OFFSET),
        abi::add_registers("x12", "x12", "x10"),
        abi::add_immediate("x13", abi::stack_pointer(), BYTES_OFFSET),
        abi::load_u64("x11", abi::stack_pointer(), SEQ_LEN_OFFSET),
        abi::label(&append_loop),
        abi::compare_immediate("x11", "0"),
        abi::branch_eq(&append_done),
        abi::load_u8("x14", "x13", 0),
        abi::store_u8("x14", "x12", 0),
        abi::add_immediate("x12", "x12", 1),
        abi::add_immediate("x13", "x13", 1),
        abi::subtract_immediate("x11", "x11", 1),
        abi::branch(&append_loop),
        abi::label(&append_done),
        abi::load_u64("x10", abi::stack_pointer(), LENGTH_OFFSET),
        abi::load_u64("x11", abi::stack_pointer(), SEQ_LEN_OFFSET),
        abi::add_registers("x10", "x10", "x11"),
        abi::store_u64("x10", abi::stack_pointer(), LENGTH_OFFSET),
        abi::branch(&read_loop),
        abi::label(&format!("{symbol}_read_eof")),
        abi::load_u64("x10", abi::stack_pointer(), LENGTH_OFFSET),
        abi::compare_immediate("x10", "0"),
        abi::branch_eq(&eof_error),
        abi::branch(&trim_cr),
        abi::label(&trim_cr),
        abi::load_u64("x10", abi::stack_pointer(), LENGTH_OFFSET),
        abi::compare_immediate("x10", "0"),
        abi::branch_eq(&format!("{symbol}_result_alloc")),
        abi::load_u64("x12", abi::stack_pointer(), BUFFER_OFFSET),
        abi::subtract_immediate("x13", "x10", 1),
        abi::add_registers("x12", "x12", "x13"),
        abi::load_u8("x14", "x12", 0),
        abi::compare_immediate("x14", "13"),
        abi::branch_ne(&format!("{symbol}_result_alloc")),
        abi::subtract_immediate("x10", "x10", 1),
        abi::store_u64("x10", abi::stack_pointer(), LENGTH_OFFSET),
        abi::label(&format!("{symbol}_result_alloc")),
        abi::load_u64("x10", abi::stack_pointer(), LENGTH_OFFSET),
        abi::add_immediate(abi::return_register(), "x10", 9),
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
        abi::load_u64("x10", abi::stack_pointer(), LENGTH_OFFSET),
        abi::store_u64("x10", "x1", 0),
        abi::add_immediate("x11", "x1", 8),
        abi::load_u64("x12", abi::stack_pointer(), BUFFER_OFFSET),
        abi::label(&result_copy_loop),
        abi::compare_immediate("x10", "0"),
        abi::branch_eq(&result_copy_done),
        abi::load_u8("x13", "x12", 0),
        abi::store_u8("x13", "x11", 0),
        abi::add_immediate("x11", "x11", 1),
        abi::add_immediate("x12", "x12", 1),
        abi::subtract_immediate("x10", "x10", 1),
        abi::branch(&result_copy_loop),
        abi::label(&result_copy_done),
        abi::store_u8("x31", "x11", 0),
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
    if with_prompt {
        instructions.push(abi::label(&prompt_flush_error));
        platform.emit_errno(
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::compare_immediate("x9", "22"),
            abi::branch_eq(&prompt_ok),
            abi::compare_immediate("x9", "45"),
            abi::branch_eq(&prompt_ok),
            abi::compare_immediate("x9", "95"),
            abi::branch_eq(&prompt_ok),
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
    }
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
    if platform.preserves_link_register_in_runtime_helpers() {
        instructions.extend([
            abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
            abi::add_stack(FRAME_SIZE),
            abi::return_(),
        ]);
        Ok((
            CodeFrame {
                stack_size: FRAME_SIZE,
                callee_saved: vec![abi::link_register().to_string()],
            },
            instructions,
            relocations,
        ))
    } else {
        instructions.extend([abi::add_stack(FRAME_SIZE), abi::return_()]);
        Ok((
            CodeFrame {
                stack_size: FRAME_SIZE,
                callee_saved: Vec::new(),
            },
            instructions,
            relocations,
        ))
    }
}
