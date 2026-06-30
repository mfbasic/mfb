use super::*;

pub(super) fn lower_fs_create_temp_file_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 128;
    const LR_OFFSET: usize = 0;
    const DIR_OFFSET: usize = 8;
    const PATH_OFFSET: usize = 16;
    const FD_OFFSET: usize = 24;
    const FILE_OFFSET: usize = 32;
    const RANDOM_OFFSET: usize = 48;
    const CURSOR_OFFSET: usize = 64;
    const UUID_FILE_EXTRA: usize = 46;

    let alloc_ok = format!("{symbol}_alloc_ok");
    let copy_dir = format!("{symbol}_copy_dir");
    let copy_done = format!("{symbol}_copy_done");
    let random_ok = format!("{symbol}_random_ok");
    let fd_ok = format!("{symbol}_fd_ok");
    let file_alloc_ok = format!("{symbol}_file_alloc_ok");
    let invalid = format!("{symbol}_invalid");
    let alloc_error = format!("{symbol}_alloc_error");
    let open_error = format!("{symbol}_open_error");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), DIR_OFFSET),
        abi::load_u64("x9", abi::return_register(), 0),
        abi::compare_immediate("x9", "0"),
        abi::branch_eq(&invalid),
        abi::add_immediate(abi::return_register(), "x9", UUID_FILE_EXTRA),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), PATH_OFFSET),
        abi::move_register("x13", "x1"),
        abi::load_u64("x9", abi::stack_pointer(), DIR_OFFSET),
        abi::load_u64("x10", "x9", 0),
        abi::add_immediate("x11", "x9", 8),
        abi::move_immediate("x12", "Integer", "0"),
        abi::label(&copy_dir),
        abi::compare_registers("x12", "x10"),
        abi::branch_eq(&copy_done),
        abi::load_u8("x14", "x11", 0),
        abi::compare_immediate("x14", "0"),
        abi::branch_eq(&invalid),
        abi::store_u8("x14", "x13", 0),
        abi::add_immediate("x11", "x11", 1),
        abi::add_immediate("x13", "x13", 1),
        abi::add_immediate("x12", "x12", 1),
        abi::branch(&copy_dir),
        abi::label(&copy_done),
    ]);
    for byte in b"/mfb-" {
        instructions.extend([
            abi::move_immediate("x14", "Byte", &byte.to_string()),
            abi::store_u8("x14", "x13", 0),
            abi::add_immediate("x13", "x13", 1),
        ]);
    }
    instructions.extend([
        abi::store_u64("x13", abi::stack_pointer(), CURSOR_OFFSET),
        abi::add_immediate(abi::return_register(), abi::stack_pointer(), RANDOM_OFFSET),
        abi::move_immediate("x1", "Integer", "16"),
    ]);
    platform.emit_random_bytes(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ge(&random_ok),
        abi::branch(&open_error),
        abi::label(&random_ok),
        abi::load_u64("x13", abi::stack_pointer(), CURSOR_OFFSET),
    ]);
    emit_uuid_v4_to_path(symbol, &mut instructions, RANDOM_OFFSET, "x13");
    for byte in b".tmp" {
        instructions.extend([
            abi::move_immediate("x14", "Byte", &byte.to_string()),
            abi::store_u8("x14", "x13", 0),
            abi::add_immediate("x13", "x13", 1),
        ]);
    }
    instructions.extend([
        abi::store_u8("x31", "x13", 0),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), PATH_OFFSET),
        abi::move_immediate("x1", "Integer", temp_file_open_flags(platform.target())),
        abi::move_immediate("x2", "Integer", "384"),
    ]);
    platform.emit_open_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ge(&fd_ok),
        abi::branch(&open_error),
        abi::label(&fd_ok),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::move_immediate(abi::return_register(), "Integer", RESOURCE_RECORD_SIZE),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&file_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&file_alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), FILE_OFFSET),
        abi::load_u64("x9", abi::stack_pointer(), FD_OFFSET),
        abi::store_u64("x9", "x1", FILE_OFFSET_FD),
        abi::store_u64("x31", "x1", FILE_OFFSET_CLOSED),
        abi::store_u64("x31", "x1", FILE_OFFSET_STATE),
        abi::move_register(RESULT_VALUE_REGISTER, "x1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&open_error),
    ]);
    platform.emit_errno(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_errno_error_mapping(symbol, &mut instructions, &mut relocations, &done);
    instructions.extend([
        abi::label(&invalid),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INVALID_ARGUMENT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INVALID_ARGUMENT_SYMBOL,
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
        abi::label(&done),
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
}

fn temp_file_open_flags(target: &str) -> &'static str {
    match target {
        "linux-aarch64" => "524482",
        _ => "2562",
    }
}

fn emit_uuid_v4_to_path(
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    random_offset: usize,
    cursor: &str,
) {
    for index in 0..16 {
        if matches!(index, 4 | 6 | 8 | 10) {
            instructions.extend([
                abi::move_immediate("x14", "Byte", "45"),
                abi::store_u8("x14", cursor, 0),
                abi::add_immediate(cursor, cursor, 1),
            ]);
        }
        instructions.push(abi::load_u8(
            "x9",
            abi::stack_pointer(),
            random_offset + index,
        ));
        if index == 6 {
            instructions.extend([
                abi::move_immediate("x10", "Integer", "15"),
                abi::and_registers("x9", "x9", "x10"),
                abi::move_immediate("x10", "Integer", "64"),
                abi::or_registers("x9", "x9", "x10"),
            ]);
        } else if index == 8 {
            instructions.extend([
                abi::move_immediate("x10", "Integer", "63"),
                abi::and_registers("x9", "x9", "x10"),
                abi::move_immediate("x10", "Integer", "128"),
                abi::or_registers("x9", "x9", "x10"),
            ]);
        }
        instructions.extend([
            abi::shift_right_immediate("x10", "x9", 4),
            abi::move_immediate("x11", "Integer", "15"),
            abi::and_registers("x11", "x9", "x11"),
        ]);
        emit_hex_nibble_to_path(symbol, instructions, index, "high", "x10", cursor);
        emit_hex_nibble_to_path(symbol, instructions, index, "low", "x11", cursor);
    }
}

fn emit_hex_nibble_to_path(
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    byte_index: usize,
    half: &str,
    nibble: &str,
    cursor: &str,
) {
    let digit = format!("{symbol}_uuid_{byte_index}_{half}_digit");
    let store = format!("{symbol}_uuid_{byte_index}_{half}_store");
    instructions.extend([
        abi::compare_immediate(nibble, "10"),
        abi::branch_lt(&digit),
        abi::add_immediate("x12", nibble, 87),
        abi::branch(&store),
        abi::label(&digit),
        abi::add_immediate("x12", nibble, 48),
        abi::label(&store),
        abi::store_u8("x12", cursor, 0),
        abi::add_immediate(cursor, cursor, 1),
    ]);
}

#[derive(Clone, Copy)]
pub(super) enum AtomicWriteValueKind {
    String,
    Bytes,
}

pub(super) fn lower_fs_atomic_write_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    value_kind: AtomicWriteValueKind,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 128;
    const LR_OFFSET: usize = 0;
    const PATH_OFFSET: usize = 8;
    const VALUE_OFFSET: usize = 16;
    const TEMP_PATH_OFFSET: usize = 24;
    const FD_OFFSET: usize = 32;
    const REMAINING_OFFSET: usize = 40;
    const CURSOR_OFFSET: usize = 48;
    const C_TEMP_OFFSET: usize = 56;
    const C_FINAL_OFFSET: usize = 64;
    const TEMPLATE_SUFFIX: &[u8] = b".mfb-XXXXXX.tmp";
    const MFB_PREFIX: &[u8] = b".mfb-";
    const X_MARKERS: &[u8] = b"XXXXXX";
    const TMP_SUFFIX: &[u8] = b".tmp";
    const MKTEMPS_SUFFIX_LEN: usize = TMP_SUFFIX.len();

    let temp_alloc_ok = format!("{symbol}_temp_alloc_ok");
    let copy_path_loop = format!("{symbol}_copy_path_loop");
    let copy_path_done = format!("{symbol}_copy_path_done");
    let mkstemps_ok = format!("{symbol}_mkstemps_ok");
    let write_loop = format!("{symbol}_write_loop");
    let write_ok = format!("{symbol}_write_ok");
    let write_error = format!("{symbol}_write_error");
    let sync_error = format!("{symbol}_sync_error");
    let close_error = format!("{symbol}_close_error");
    let c_temp_alloc_ok = format!("{symbol}_c_temp_alloc_ok");
    let c_final_alloc_ok = format!("{symbol}_c_final_alloc_ok");
    let c_temp_loop = format!("{symbol}_c_temp_loop");
    let c_temp_done = format!("{symbol}_c_temp_done");
    let c_final_loop = format!("{symbol}_c_final_loop");
    let c_final_done = format!("{symbol}_c_final_done");
    let rename_ok = format!("{symbol}_rename_ok");
    let invalid = format!("{symbol}_invalid");
    let alloc_error = format!("{symbol}_alloc_error");
    let rename_error = format!("{symbol}_rename_error");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), PATH_OFFSET),
        abi::store_u64("x1", abi::stack_pointer(), VALUE_OFFSET),
        abi::load_u64("x9", abi::return_register(), 0),
        abi::compare_immediate("x9", "0"),
        abi::branch_eq(&invalid),
        abi::add_immediate(abi::return_register(), "x9", 9 + TEMPLATE_SUFFIX.len()),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&temp_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&temp_alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), TEMP_PATH_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), PATH_OFFSET),
        abi::load_u64("x11", "x10", 0),
        abi::move_immediate("x12", "Integer", &(TEMPLATE_SUFFIX.len()).to_string()),
        abi::add_registers("x13", "x11", "x12"),
        abi::store_u64("x13", "x1", 0),
        abi::add_immediate("x14", "x10", 8),
        abi::add_immediate("x15", "x1", 8),
        abi::move_immediate("x16", "Integer", "0"),
        abi::label(&copy_path_loop),
        abi::compare_registers("x16", "x11"),
        abi::branch_eq(&copy_path_done),
        abi::load_u8("x17", "x14", 0),
        abi::compare_immediate("x17", "0"),
        abi::branch_eq(&invalid),
        abi::store_u8("x17", "x15", 0),
        abi::add_immediate("x14", "x14", 1),
        abi::add_immediate("x15", "x15", 1),
        abi::add_immediate("x16", "x16", 1),
        abi::branch(&copy_path_loop),
        abi::label(&copy_path_done),
    ]);
    for byte in MFB_PREFIX {
        instructions.extend([
            abi::move_immediate("x17", "Byte", &byte.to_string()),
            abi::store_u8("x17", "x15", 0),
            abi::add_immediate("x15", "x15", 1),
        ]);
    }
    for byte in X_MARKERS {
        instructions.extend([
            abi::move_immediate("x17", "Byte", &byte.to_string()),
            abi::store_u8("x17", "x15", 0),
            abi::add_immediate("x15", "x15", 1),
        ]);
    }
    for byte in TMP_SUFFIX {
        instructions.extend([
            abi::move_immediate("x17", "Byte", &byte.to_string()),
            abi::store_u8("x17", "x15", 0),
            abi::add_immediate("x15", "x15", 1),
        ]);
    }
    instructions.extend([
        abi::store_u8("x31", "x15", 0),
        abi::load_u64(
            abi::return_register(),
            abi::stack_pointer(),
            TEMP_PATH_OFFSET,
        ),
        abi::add_immediate(abi::return_register(), abi::return_register(), 8),
        abi::move_immediate("x1", "Integer", &MKTEMPS_SUFFIX_LEN.to_string()),
    ]);
    platform.emit_mkstemps(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ge(&mkstemps_ok),
        abi::branch(&rename_error),
        abi::label(&mkstemps_ok),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
    ]);
    match value_kind {
        AtomicWriteValueKind::String => {
            instructions.extend([
                abi::load_u64("x10", abi::stack_pointer(), VALUE_OFFSET),
                abi::load_u64("x11", "x10", 0),
                abi::add_immediate("x12", "x10", 8),
            ]);
        }
        AtomicWriteValueKind::Bytes => {
            instructions.extend([
                abi::load_u64("x10", abi::stack_pointer(), VALUE_OFFSET),
                abi::load_u64("x11", "x10", COLLECTION_OFFSET_DATA_LENGTH),
                abi::add_immediate("x12", "x10", COLLECTION_HEADER_SIZE),
                abi::load_u64("x13", "x10", COLLECTION_OFFSET_CAPACITY),
                abi::move_immediate("x14", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
                abi::multiply_registers("x13", "x13", "x14"),
                abi::add_registers("x12", "x12", "x13"),
            ]);
        }
    }
    instructions.extend([
        abi::store_u64("x11", abi::stack_pointer(), REMAINING_OFFSET),
        abi::store_u64("x12", abi::stack_pointer(), CURSOR_OFFSET),
        abi::label(&write_loop),
        abi::load_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::compare_immediate("x10", "0"),
        abi::branch_eq(&write_ok),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64("x1", abi::stack_pointer(), CURSOR_OFFSET),
        abi::move_register("x2", "x10"),
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
        abi::load_u64("x11", abi::stack_pointer(), CURSOR_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::add_registers("x11", "x11", abi::return_register()),
        abi::subtract_registers("x10", "x10", abi::return_register()),
        abi::store_u64("x11", abi::stack_pointer(), CURSOR_OFFSET),
        abi::store_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::branch(&write_loop),
        abi::label(&write_ok),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
    ]);
    platform.emit_sync_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&sync_error),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
    ]);
    platform.emit_close_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&close_error),
        abi::load_u64("x9", abi::stack_pointer(), TEMP_PATH_OFFSET),
        abi::load_u64(abi::return_register(), "x9", 0),
        abi::add_immediate(abi::return_register(), abi::return_register(), 1),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&c_temp_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&c_temp_alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), C_TEMP_OFFSET),
        abi::load_u64("x9", abi::stack_pointer(), PATH_OFFSET),
        abi::load_u64(abi::return_register(), "x9", 0),
        abi::add_immediate(abi::return_register(), abi::return_register(), 1),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&c_final_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&c_final_alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), C_FINAL_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), TEMP_PATH_OFFSET),
        abi::load_u64("x11", "x10", 0),
        abi::add_immediate("x12", "x10", 8),
        abi::load_u64("x13", abi::stack_pointer(), C_TEMP_OFFSET),
        abi::move_immediate("x14", "Integer", "0"),
        abi::label(&c_temp_loop),
        abi::compare_registers("x14", "x11"),
        abi::branch_eq(&c_temp_done),
        abi::load_u8("x15", "x12", 0),
        abi::store_u8("x15", "x13", 0),
        abi::add_immediate("x12", "x12", 1),
        abi::add_immediate("x13", "x13", 1),
        abi::add_immediate("x14", "x14", 1),
        abi::branch(&c_temp_loop),
        abi::label(&c_temp_done),
        abi::store_u8("x31", "x13", 0),
        abi::load_u64("x10", abi::stack_pointer(), PATH_OFFSET),
        abi::load_u64("x11", "x10", 0),
        abi::add_immediate("x12", "x10", 8),
        abi::load_u64("x13", abi::stack_pointer(), C_FINAL_OFFSET),
        abi::move_immediate("x14", "Integer", "0"),
        abi::label(&c_final_loop),
        abi::compare_registers("x14", "x11"),
        abi::branch_eq(&c_final_done),
        abi::load_u8("x15", "x12", 0),
        abi::store_u8("x15", "x13", 0),
        abi::add_immediate("x12", "x12", 1),
        abi::add_immediate("x13", "x13", 1),
        abi::add_immediate("x14", "x14", 1),
        abi::branch(&c_final_loop),
        abi::label(&c_final_done),
        abi::store_u8("x31", "x13", 0),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), C_TEMP_OFFSET),
        abi::load_u64("x1", abi::stack_pointer(), C_FINAL_OFFSET),
    ]);
    platform.emit_rename_path(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&rename_ok),
        abi::branch(&rename_error),
        abi::label(&rename_ok),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&rename_error),
    ]);
    platform.emit_errno(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_errno_error_mapping(symbol, &mut instructions, &mut relocations, &done);
    instructions.extend([
        abi::label(&write_error),
        abi::label(&sync_error),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
    ]);
    platform.emit_close_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::label(&close_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUTPUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_OUTPUT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&invalid),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INVALID_ARGUMENT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INVALID_ARGUMENT_SYMBOL,
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
        abi::label(&done),
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
}

pub(super) fn lower_fs_write_text_path_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    append: bool,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 80;
    const LR_OFFSET: usize = 0;
    const PATH_OFFSET: usize = 8;
    const VALUE_OFFSET: usize = 16;
    const C_PATH_OFFSET: usize = 24;
    const FD_OFFSET: usize = 32;
    const REMAINING_OFFSET: usize = 40;
    const CURSOR_OFFSET: usize = 48;
    const CLOSE_STATUS_OFFSET: usize = 56;

    let alloc_ok = format!("{symbol}_alloc_ok");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let invalid = format!("{symbol}_invalid");
    let open_ok = format!("{symbol}_open_ok");
    let open_error = format!("{symbol}_open_error");
    let write_loop = format!("{symbol}_write_loop");
    let write_done = format!("{symbol}_write_done");
    let write_error = format!("{symbol}_write_error");
    let close_error = format!("{symbol}_close_error");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let flags = open_flag_set(platform.target(), false);
    let mode_flags = if append { flags.append } else { flags.write };
    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), PATH_OFFSET),
        abi::store_u64("x1", abi::stack_pointer(), VALUE_OFFSET),
        abi::load_u64("x9", abi::return_register(), 0),
        abi::compare_immediate("x9", "0"),
        abi::branch_eq(&invalid),
        abi::add_immediate(abi::return_register(), "x9", 1),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), C_PATH_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), PATH_OFFSET),
        abi::load_u64("x11", "x10", 0),
        abi::add_immediate("x12", "x10", 8),
        abi::move_register("x13", "x1"),
        abi::move_immediate("x14", "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers("x14", "x11"),
        abi::branch_eq(&copy_done),
        abi::load_u8("x15", "x12", 0),
        abi::compare_immediate("x15", "0"),
        abi::branch_eq(&invalid),
        abi::store_u8("x15", "x13", 0),
        abi::add_immediate("x12", "x12", 1),
        abi::add_immediate("x13", "x13", 1),
        abi::add_immediate("x14", "x14", 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8("x31", "x13", 0),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), C_PATH_OFFSET),
        abi::move_immediate("x1", "Integer", mode_flags),
        abi::move_immediate("x2", "Integer", "438"),
    ]);
    platform.emit_open_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ge(&open_ok),
        abi::branch(&open_error),
        abi::label(&open_ok),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), VALUE_OFFSET),
        abi::load_u64("x11", "x10", 0),
        abi::add_immediate("x12", "x10", 8),
        abi::store_u64("x11", abi::stack_pointer(), REMAINING_OFFSET),
        abi::store_u64("x12", abi::stack_pointer(), CURSOR_OFFSET),
        abi::label(&write_loop),
        abi::load_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::compare_immediate("x10", "0"),
        abi::branch_eq(&write_done),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64("x1", abi::stack_pointer(), CURSOR_OFFSET),
        abi::move_register("x2", "x10"),
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
        abi::load_u64("x11", abi::stack_pointer(), CURSOR_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::add_registers("x11", "x11", abi::return_register()),
        abi::subtract_registers("x10", "x10", abi::return_register()),
        abi::store_u64("x11", abi::stack_pointer(), CURSOR_OFFSET),
        abi::store_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::branch(&write_loop),
        abi::label(&write_done),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
    ]);
    platform.emit_sync_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&write_error),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
    ]);
    platform.emit_close_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::store_u64(
            abi::return_register(),
            abi::stack_pointer(),
            CLOSE_STATUS_OFFSET,
        ),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&close_error),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&write_error),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
    ]);
    platform.emit_close_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUTPUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_OUTPUT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&open_error)]);
    platform.emit_errno(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_errno_error_mapping(symbol, &mut instructions, &mut relocations, &done);
    instructions.extend([
        abi::label(&invalid),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INVALID_ARGUMENT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INVALID_ARGUMENT_SYMBOL,
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
        abi::label(&close_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUTPUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_OUTPUT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::label(&done),
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
}

pub(super) fn lower_fs_read_text_path_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 96;
    const LR_OFFSET: usize = 0;
    const PATH_OFFSET: usize = 8;
    const C_PATH_OFFSET: usize = 16;
    const FD_OFFSET: usize = 24;
    const END_OFFSET: usize = 32;
    const LEN_OFFSET: usize = 40;
    const STRING_OFFSET: usize = 48;
    const REMAINING_OFFSET: usize = 56;
    const CURSOR_OFFSET: usize = 64;

    let alloc_ok = format!("{symbol}_path_alloc_ok");
    let copy_loop = format!("{symbol}_path_copy_loop");
    let copy_done = format!("{symbol}_path_copy_done");
    let invalid = format!("{symbol}_invalid");
    let open_ok = format!("{symbol}_open_ok");
    let open_error = format!("{symbol}_open_error");
    let seek_error = format!("{symbol}_seek_error");
    let string_alloc_ok = format!("{symbol}_string_alloc_ok");
    let read_loop = format!("{symbol}_read_loop");
    let read_done = format!("{symbol}_read_done");
    let read_error = format!("{symbol}_read_error");
    let alloc_error = format!("{symbol}_alloc_error");
    let close_and_read_error = format!("{symbol}_close_and_read_error");
    let done = format!("{symbol}_done");

    let flags = open_flag_set(platform.target(), false);
    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), PATH_OFFSET),
        abi::load_u64("x9", abi::return_register(), 0),
        abi::compare_immediate("x9", "0"),
        abi::branch_eq(&invalid),
        abi::add_immediate(abi::return_register(), "x9", 1),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), C_PATH_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), PATH_OFFSET),
        abi::load_u64("x11", "x10", 0),
        abi::add_immediate("x12", "x10", 8),
        abi::move_register("x13", "x1"),
        abi::move_immediate("x14", "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers("x14", "x11"),
        abi::branch_eq(&copy_done),
        abi::load_u8("x15", "x12", 0),
        abi::compare_immediate("x15", "0"),
        abi::branch_eq(&invalid),
        abi::store_u8("x15", "x13", 0),
        abi::add_immediate("x12", "x12", 1),
        abi::add_immediate("x13", "x13", 1),
        abi::add_immediate("x14", "x14", 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8("x31", "x13", 0),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), C_PATH_OFFSET),
        abi::move_immediate("x1", "Integer", flags.read),
        abi::move_immediate("x2", "Integer", "0"),
    ]);
    platform.emit_open_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ge(&open_ok),
        abi::branch(&open_error),
        abi::label(&open_ok),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::move_register("x9", abi::return_register()),
        abi::move_register(abi::return_register(), "x9"),
        abi::move_immediate("x1", "Integer", "0"),
        abi::move_immediate("x2", "Integer", "2"),
    ]);
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&close_and_read_error),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), END_OFFSET),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::move_immediate("x1", "Integer", "0"),
        abi::move_immediate("x2", "Integer", "0"),
    ]);
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&close_and_read_error),
        abi::load_u64("x10", abi::stack_pointer(), END_OFFSET),
        abi::store_u64("x10", abi::stack_pointer(), LEN_OFFSET),
        abi::add_immediate(abi::return_register(), "x10", 9),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&string_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&string_alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), STRING_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), LEN_OFFSET),
        abi::store_u64("x10", "x1", 0),
        abi::store_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::add_immediate("x11", "x1", 8),
        abi::store_u64("x11", abi::stack_pointer(), CURSOR_OFFSET),
        abi::label(&read_loop),
        abi::load_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::compare_immediate("x10", "0"),
        abi::branch_eq(&read_done),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64("x1", abi::stack_pointer(), CURSOR_OFFSET),
        abi::move_register("x2", "x10"),
    ]);
    platform.emit_read_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(&read_error),
        abi::load_u64("x11", abi::stack_pointer(), CURSOR_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::add_registers("x11", "x11", abi::return_register()),
        abi::subtract_registers("x10", "x10", abi::return_register()),
        abi::store_u64("x11", abi::stack_pointer(), CURSOR_OFFSET),
        abi::store_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::branch(&read_loop),
        abi::label(&read_done),
        abi::load_u64("x11", abi::stack_pointer(), CURSOR_OFFSET),
        abi::store_u8("x31", "x11", 0),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
    ]);
    platform.emit_close_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    let encoding_error = format!("{symbol}_encoding_error");
    instructions.extend([
        abi::load_u64("x0", abi::stack_pointer(), STRING_OFFSET),
        abi::add_immediate("x0", "x0", 8),
        abi::load_u64("x1", abi::stack_pointer(), LEN_OFFSET),
    ]);
    emit_call_validate_utf8(symbol, &encoding_error, &mut instructions, &mut relocations);
    instructions.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), STRING_OFFSET),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
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
        abi::label(&read_error),
        abi::label(&close_and_read_error),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
    ]);
    platform.emit_close_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::label(&seek_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_READ_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_READ_SYMBOL, &mut instructions, &mut relocations);
    instructions.extend([abi::branch(&done), abi::label(&open_error)]);
    platform.emit_errno(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_fs_path_errno_error_mapping(
        symbol,
        platform.target(),
        false,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([
        abi::label(&invalid),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INVALID_ARGUMENT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INVALID_ARGUMENT_SYMBOL,
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
        abi::label(&done),
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
}

pub(super) fn lower_fs_write_bytes_path_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    append: bool,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 80;
    const LR_OFFSET: usize = 0;
    const PATH_OFFSET: usize = 8;
    const VALUE_OFFSET: usize = 16;
    const C_PATH_OFFSET: usize = 24;
    const FD_OFFSET: usize = 32;
    const REMAINING_OFFSET: usize = 40;
    const CURSOR_OFFSET: usize = 48;

    let alloc_ok = format!("{symbol}_alloc_ok");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let invalid = format!("{symbol}_invalid");
    let open_ok = format!("{symbol}_open_ok");
    let open_error = format!("{symbol}_open_error");
    let write_loop = format!("{symbol}_write_loop");
    let write_done = format!("{symbol}_write_done");
    let write_error = format!("{symbol}_write_error");
    let close_error = format!("{symbol}_close_error");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let flags = open_flag_set(platform.target(), false);
    let mode_flags = if append { flags.append } else { flags.write };
    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), PATH_OFFSET),
        abi::store_u64("x1", abi::stack_pointer(), VALUE_OFFSET),
        abi::load_u64("x9", abi::return_register(), 0),
        abi::compare_immediate("x9", "0"),
        abi::branch_eq(&invalid),
        abi::add_immediate(abi::return_register(), "x9", 1),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), C_PATH_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), PATH_OFFSET),
        abi::load_u64("x11", "x10", 0),
        abi::add_immediate("x12", "x10", 8),
        abi::move_register("x13", "x1"),
        abi::move_immediate("x14", "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers("x14", "x11"),
        abi::branch_eq(&copy_done),
        abi::load_u8("x15", "x12", 0),
        abi::compare_immediate("x15", "0"),
        abi::branch_eq(&invalid),
        abi::store_u8("x15", "x13", 0),
        abi::add_immediate("x12", "x12", 1),
        abi::add_immediate("x13", "x13", 1),
        abi::add_immediate("x14", "x14", 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8("x31", "x13", 0),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), C_PATH_OFFSET),
        abi::move_immediate("x1", "Integer", mode_flags),
        abi::move_immediate("x2", "Integer", "438"),
    ]);
    platform.emit_open_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ge(&open_ok),
        abi::branch(&open_error),
        abi::label(&open_ok),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), VALUE_OFFSET),
        abi::load_u64("x11", "x10", COLLECTION_OFFSET_DATA_LENGTH),
        abi::add_immediate("x12", "x10", COLLECTION_HEADER_SIZE),
        abi::load_u64("x13", "x10", COLLECTION_OFFSET_CAPACITY),
        abi::move_immediate("x14", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("x13", "x13", "x14"),
        abi::add_registers("x12", "x12", "x13"),
        abi::store_u64("x11", abi::stack_pointer(), REMAINING_OFFSET),
        abi::store_u64("x12", abi::stack_pointer(), CURSOR_OFFSET),
        abi::label(&write_loop),
        abi::load_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::compare_immediate("x10", "0"),
        abi::branch_eq(&write_done),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64("x1", abi::stack_pointer(), CURSOR_OFFSET),
        abi::move_register("x2", "x10"),
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
        abi::load_u64("x11", abi::stack_pointer(), CURSOR_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::add_registers("x11", "x11", abi::return_register()),
        abi::subtract_registers("x10", "x10", abi::return_register()),
        abi::store_u64("x11", abi::stack_pointer(), CURSOR_OFFSET),
        abi::store_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::branch(&write_loop),
        abi::label(&write_done),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
    ]);
    platform.emit_sync_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&write_error),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
    ]);
    platform.emit_close_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&close_error),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&write_error),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
    ]);
    platform.emit_close_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUTPUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_OUTPUT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&open_error)]);
    platform.emit_errno(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_errno_error_mapping(symbol, &mut instructions, &mut relocations, &done);
    instructions.extend([
        abi::label(&invalid),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INVALID_ARGUMENT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INVALID_ARGUMENT_SYMBOL,
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
        abi::label(&close_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUTPUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_OUTPUT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::label(&done),
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
}

pub(super) fn lower_fs_read_bytes_path_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 96;
    const LR_OFFSET: usize = 0;
    const PATH_OFFSET: usize = 8;
    const C_PATH_OFFSET: usize = 16;
    const FD_OFFSET: usize = 24;
    const FILE_OFFSET: usize = 32;
    const RESULT_TAG_OFFSET: usize = 48;
    const RESULT_VALUE_OFFSET: usize = 56;
    const RESULT_MESSAGE_OFFSET: usize = 64;

    let alloc_ok = format!("{symbol}_path_alloc_ok");
    let file_alloc_ok = format!("{symbol}_file_alloc_ok");
    let copy_loop = format!("{symbol}_path_copy_loop");
    let copy_done = format!("{symbol}_path_copy_done");
    let invalid = format!("{symbol}_invalid");
    let open_ok = format!("{symbol}_open_ok");
    let open_error = format!("{symbol}_open_error");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let flags = open_flag_set(platform.target(), false);
    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), PATH_OFFSET),
        abi::load_u64("x9", abi::return_register(), 0),
        abi::compare_immediate("x9", "0"),
        abi::branch_eq(&invalid),
        abi::add_immediate(abi::return_register(), "x9", 1),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), C_PATH_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), PATH_OFFSET),
        abi::load_u64("x11", "x10", 0),
        abi::add_immediate("x12", "x10", 8),
        abi::move_register("x13", "x1"),
        abi::move_immediate("x14", "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers("x14", "x11"),
        abi::branch_eq(&copy_done),
        abi::load_u8("x15", "x12", 0),
        abi::compare_immediate("x15", "0"),
        abi::branch_eq(&invalid),
        abi::store_u8("x15", "x13", 0),
        abi::add_immediate("x12", "x12", 1),
        abi::add_immediate("x13", "x13", 1),
        abi::add_immediate("x14", "x14", 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8("x31", "x13", 0),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), C_PATH_OFFSET),
        abi::move_immediate("x1", "Integer", flags.read),
        abi::move_immediate("x2", "Integer", "0"),
    ]);
    platform.emit_open_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ge(&open_ok),
        abi::branch(&open_error),
        abi::label(&open_ok),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::move_immediate(abi::return_register(), "Integer", RESOURCE_RECORD_SIZE),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&file_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&file_alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), FILE_OFFSET),
        abi::load_u64("x9", abi::stack_pointer(), FD_OFFSET),
        abi::store_u64("x9", "x1", FILE_OFFSET_FD),
        abi::store_u64("x31", "x1", FILE_OFFSET_CLOSED),
        abi::store_u64("x31", "x1", FILE_OFFSET_STATE),
        abi::move_register(abi::return_register(), "x1"),
        abi::branch_link("_mfb_rt_fs_fs_readAllBytes"),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: "_mfb_rt_fs_fs_readAllBytes".to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::store_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), RESULT_TAG_OFFSET),
        abi::store_u64(
            RESULT_VALUE_REGISTER,
            abi::stack_pointer(),
            RESULT_VALUE_OFFSET,
        ),
        abi::store_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            RESULT_MESSAGE_OFFSET,
        ),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
    ]);
    platform.emit_close_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), RESULT_TAG_OFFSET),
        abi::load_u64(
            RESULT_VALUE_REGISTER,
            abi::stack_pointer(),
            RESULT_VALUE_OFFSET,
        ),
        abi::load_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            RESULT_MESSAGE_OFFSET,
        ),
        abi::branch(&done),
        abi::label(&open_error),
    ]);
    platform.emit_errno(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_fs_path_errno_error_mapping(
        symbol,
        platform.target(),
        false,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([
        abi::label(&invalid),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INVALID_ARGUMENT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INVALID_ARGUMENT_SYMBOL,
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
        abi::label(&done),
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
}

