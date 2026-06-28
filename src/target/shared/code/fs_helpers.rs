use super::*;

pub(super) fn lower_fs_exists_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 32;
    const LR_OFFSET: usize = 0;
    const PATH_OFFSET: usize = 8;
    const ALLOC_OFFSET: usize = 16;

    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let alloc_ok = format!("{symbol}_alloc_ok");
    let exists = format!("{symbol}_exists");
    let missing = format!("{symbol}_missing");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), PATH_OFFSET),
        abi::load_u64("x9", abi::return_register(), 0),
        abi::add_immediate(abi::return_register(), "x9", 1),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    let alloc_symbol = ERR_ALLOCATION_SYMBOL.to_string();
    instructions.push(
        CodeInstruction::new("adrp")
            .field("dst", RESULT_ERROR_MESSAGE_REGISTER)
            .field("symbol", &alloc_symbol),
    );
    instructions.push(
        CodeInstruction::new("add_pageoff")
            .field("dst", RESULT_ERROR_MESSAGE_REGISTER)
            .field("src", RESULT_ERROR_MESSAGE_REGISTER)
            .field("symbol", &alloc_symbol),
    );
    relocations.extend([
        CodeRelocation {
            from: symbol.to_string(),
            to: alloc_symbol.clone(),
            kind: "page21".to_string(),
            binding: "data".to_string(),
            library: None,
        },
        CodeRelocation {
            from: symbol.to_string(),
            to: alloc_symbol,
            kind: "pageoff12".to_string(),
            binding: "data".to_string(),
            library: None,
        },
    ]);
    instructions.extend([
        abi::branch(&done),
        abi::label(&alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), ALLOC_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), PATH_OFFSET),
        abi::load_u64("x11", "x10", 0),
        abi::add_immediate("x12", "x10", 8),
        abi::move_register("x13", "x1"),
        abi::move_immediate("x14", "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers("x14", "x11"),
        abi::branch_eq(&copy_done),
        abi::load_u8("x15", "x12", 0),
        abi::store_u8("x15", "x13", 0),
        abi::add_immediate("x12", "x12", 1),
        abi::add_immediate("x13", "x13", 1),
        abi::add_immediate("x14", "x14", 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8("x31", "x13", 0),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), ALLOC_OFFSET),
    ]);
    platform.emit_path_exists(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&exists),
        abi::label(&missing),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&exists),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
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

pub(super) fn lower_fs_kind_exists_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    expected_kind: &str,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 288;
    const LR_OFFSET: usize = 0;
    const PATH_OFFSET: usize = 8;
    const ALLOC_OFFSET: usize = 16;
    const STAT_OFFSET: usize = 32;

    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let alloc_ok = format!("{symbol}_alloc_ok");
    let found = format!("{symbol}_found");
    let missing = format!("{symbol}_missing");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), PATH_OFFSET),
        abi::load_u64("x9", abi::return_register(), 0),
        abi::add_immediate(abi::return_register(), "x9", 1),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    let alloc_symbol = ERR_ALLOCATION_SYMBOL.to_string();
    instructions.push(
        CodeInstruction::new("adrp")
            .field("dst", RESULT_ERROR_MESSAGE_REGISTER)
            .field("symbol", &alloc_symbol),
    );
    instructions.push(
        CodeInstruction::new("add_pageoff")
            .field("dst", RESULT_ERROR_MESSAGE_REGISTER)
            .field("src", RESULT_ERROR_MESSAGE_REGISTER)
            .field("symbol", &alloc_symbol),
    );
    relocations.extend([
        CodeRelocation {
            from: symbol.to_string(),
            to: alloc_symbol.clone(),
            kind: "page21".to_string(),
            binding: "data".to_string(),
            library: None,
        },
        CodeRelocation {
            from: symbol.to_string(),
            to: alloc_symbol,
            kind: "pageoff12".to_string(),
            binding: "data".to_string(),
            library: None,
        },
    ]);
    instructions.extend([
        abi::branch(&done),
        abi::label(&alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), ALLOC_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), PATH_OFFSET),
        abi::load_u64("x11", "x10", 0),
        abi::add_immediate("x12", "x10", 8),
        abi::move_register("x13", "x1"),
        abi::move_immediate("x14", "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers("x14", "x11"),
        abi::branch_eq(&copy_done),
        abi::load_u8("x15", "x12", 0),
        abi::store_u8("x15", "x13", 0),
        abi::add_immediate("x12", "x12", 1),
        abi::add_immediate("x13", "x13", 1),
        abi::add_immediate("x14", "x14", 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8("x31", "x13", 0),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), ALLOC_OFFSET),
        abi::add_immediate("x1", abi::stack_pointer(), STAT_OFFSET),
    ]);
    platform.emit_path_stat(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&missing),
        abi::load_u16(
            "x9",
            abi::stack_pointer(),
            STAT_OFFSET + platform.stat_mode_offset(),
        ),
        abi::move_immediate("x10", "Integer", FS_MODE_TYPE_MASK),
        abi::and_registers("x9", "x9", "x10"),
        abi::move_immediate("x10", "Integer", expected_kind),
        abi::compare_registers("x9", "x10"),
        abi::branch_eq(&found),
        abi::label(&missing),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&found),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
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

pub(super) fn lower_fs_current_directory_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 48;
    const LR_OFFSET: usize = 0;
    const BUFFER_OFFSET: usize = 8;
    const LENGTH_OFFSET: usize = 16;
    const GETCWD_CAPACITY: &str = "4096";

    let temp_alloc_ok = format!("{symbol}_temp_alloc_ok");
    let string_alloc_ok = format!("{symbol}_string_alloc_ok");
    let count_loop = format!("{symbol}_count_loop");
    let count_done = format!("{symbol}_count_done");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let read_error = format!("{symbol}_read_error");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::move_immediate(abi::return_register(), "Integer", GETCWD_CAPACITY),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&temp_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&temp_alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), BUFFER_OFFSET),
        abi::move_register(abi::return_register(), "x1"),
        abi::move_immediate("x1", "Integer", GETCWD_CAPACITY),
    ]);
    platform.emit_current_directory(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(&read_error),
        abi::load_u64("x10", abi::stack_pointer(), BUFFER_OFFSET),
        abi::move_register("x11", "x10"),
        abi::move_immediate("x12", "Integer", "0"),
        abi::label(&count_loop),
        abi::load_u8("x13", "x11", 0),
        abi::compare_immediate("x13", "0"),
        abi::branch_eq(&count_done),
        abi::add_immediate("x11", "x11", 1),
        abi::add_immediate("x12", "x12", 1),
        abi::branch(&count_loop),
        abi::label(&count_done),
        abi::store_u64("x12", abi::stack_pointer(), LENGTH_OFFSET),
        abi::add_immediate(abi::return_register(), "x12", 9),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&string_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&string_alloc_ok),
        abi::load_u64("x10", abi::stack_pointer(), LENGTH_OFFSET),
        abi::store_u64("x10", "x1", 0),
        abi::load_u64("x11", abi::stack_pointer(), BUFFER_OFFSET),
        abi::add_immediate("x12", "x1", 8),
        abi::move_immediate("x13", "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers("x13", "x10"),
        abi::branch_eq(&copy_done),
        abi::load_u8("x14", "x11", 0),
        abi::store_u8("x14", "x12", 0),
        abi::add_immediate("x11", "x11", 1),
        abi::add_immediate("x12", "x12", 1),
        abi::add_immediate("x13", "x13", 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8("x31", "x12", 0),
        abi::move_register(RESULT_VALUE_REGISTER, "x1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&read_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_READ_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_READ_SYMBOL, &mut instructions, &mut relocations);
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

pub(super) fn lower_fs_temp_directory_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 48;
    const LR_OFFSET: usize = 0;
    const BUFFER_OFFSET: usize = 8;
    const LENGTH_OFFSET: usize = 16;
    const TEMP_CAPACITY: &str = "4096";

    let temp_alloc_ok = format!("{symbol}_temp_alloc_ok");
    let string_alloc_ok = format!("{symbol}_string_alloc_ok");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let read_error = format!("{symbol}_read_error");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::move_immediate(abi::return_register(), "Integer", TEMP_CAPACITY),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&temp_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&temp_alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), BUFFER_OFFSET),
        abi::move_register(abi::return_register(), "x1"),
        abi::move_immediate("x1", "Integer", TEMP_CAPACITY),
    ]);
    platform.emit_temp_directory(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(&read_error),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), LENGTH_OFFSET),
        abi::add_immediate(abi::return_register(), abi::return_register(), 9),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&string_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&string_alloc_ok),
        abi::load_u64("x10", abi::stack_pointer(), LENGTH_OFFSET),
        abi::store_u64("x10", "x1", 0),
        abi::load_u64("x11", abi::stack_pointer(), BUFFER_OFFSET),
        abi::add_immediate("x12", "x1", 8),
        abi::move_immediate("x13", "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers("x13", "x10"),
        abi::branch_eq(&copy_done),
        abi::load_u8("x14", "x11", 0),
        abi::store_u8("x14", "x12", 0),
        abi::add_immediate("x11", "x11", 1),
        abi::add_immediate("x12", "x12", 1),
        abi::add_immediate("x13", "x13", 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8("x31", "x12", 0),
        abi::move_register(RESULT_VALUE_REGISTER, "x1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&read_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_READ_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_READ_SYMBOL, &mut instructions, &mut relocations);
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

pub(super) fn lower_fs_path_operation_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    operation: FsPathOperation,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 32;
    const LR_OFFSET: usize = 0;
    const PATH_OFFSET: usize = 8;
    const ALLOC_OFFSET: usize = 16;

    let alloc_ok = format!("{symbol}_alloc_ok");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let invalid_path = format!("{symbol}_invalid_path");
    let call_error = format!("{symbol}_call_error");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), PATH_OFFSET),
        abi::load_u64("x9", abi::return_register(), 0),
        abi::compare_immediate("x9", "0"),
        abi::branch_eq(&invalid_path),
        abi::add_immediate(abi::return_register(), "x9", 1),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
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
        abi::label(&alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), ALLOC_OFFSET),
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
        abi::branch_eq(&invalid_path),
        abi::store_u8("x15", "x13", 0),
        abi::add_immediate("x12", "x12", 1),
        abi::add_immediate("x13", "x13", 1),
        abi::add_immediate("x14", "x14", 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8("x31", "x13", 0),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), ALLOC_OFFSET),
    ]);
    platform.emit_fs_path_operation(
        symbol,
        operation,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&call_error),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&call_error),
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
        abi::label(&invalid_path),
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

pub(super) fn lower_fs_create_directories_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 64;
    const LR_OFFSET: usize = 0;
    const PATH_OFFSET: usize = 8;
    const ALLOC_OFFSET: usize = 16;
    const CURSOR_OFFSET: usize = 24;

    let alloc_ok = format!("{symbol}_alloc_ok");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let invalid_path = format!("{symbol}_invalid_path");
    let scan_loop = format!("{symbol}_scan_loop");
    let mkdir_prefix = format!("{symbol}_mkdir_prefix");
    let prefix_ok = format!("{symbol}_prefix_ok");
    let final_mkdir = format!("{symbol}_final_mkdir");
    let final_ok = format!("{symbol}_final_ok");
    let call_error = format!("{symbol}_call_error");
    let err_not_found = format!("{symbol}_err_not_found");
    let err_access_denied = format!("{symbol}_err_access_denied");
    let err_output = format!("{symbol}_err_output");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), PATH_OFFSET),
        abi::load_u64("x9", abi::return_register(), 0),
        abi::compare_immediate("x9", "0"),
        abi::branch_eq(&invalid_path),
        abi::add_immediate(abi::return_register(), "x9", 1),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), ALLOC_OFFSET),
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
        abi::branch_eq(&invalid_path),
        abi::store_u8("x15", "x13", 0),
        abi::add_immediate("x12", "x12", 1),
        abi::add_immediate("x13", "x13", 1),
        abi::add_immediate("x14", "x14", 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8("x31", "x13", 0),
        abi::load_u64("x10", abi::stack_pointer(), ALLOC_OFFSET),
        abi::load_u8("x11", "x10", 0),
        abi::compare_immediate("x11", "47"),
        abi::branch_ne(&scan_loop),
        abi::add_immediate("x10", "x10", 1),
        abi::label(&scan_loop),
        abi::store_u64("x10", abi::stack_pointer(), CURSOR_OFFSET),
        abi::load_u8("x11", "x10", 0),
        abi::compare_immediate("x11", "0"),
        abi::branch_eq(&final_mkdir),
        abi::compare_immediate("x11", "47"),
        abi::branch_eq(&mkdir_prefix),
        abi::add_immediate("x10", "x10", 1),
        abi::branch(&scan_loop),
        abi::label(&mkdir_prefix),
        abi::store_u8("x31", "x10", 0),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), ALLOC_OFFSET),
    ]);
    platform.emit_fs_path_operation(
        symbol,
        FsPathOperation::Mkdir,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64("x10", abi::stack_pointer(), CURSOR_OFFSET),
        abi::move_immediate("x11", "Integer", "47"),
        abi::store_u8("x11", "x10", 0),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&prefix_ok),
    ]);
    platform.emit_errno(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate("x9", "17"),
        abi::branch_ne(&call_error),
        abi::label(&prefix_ok),
        abi::load_u64("x10", abi::stack_pointer(), CURSOR_OFFSET),
        abi::add_immediate("x10", "x10", 1),
        abi::branch(&scan_loop),
        abi::label(&final_mkdir),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), ALLOC_OFFSET),
    ]);
    platform.emit_fs_path_operation(
        symbol,
        FsPathOperation::Mkdir,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&final_ok),
    ]);
    platform.emit_errno(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate("x9", "17"),
        abi::branch_eq(&final_ok),
        abi::branch(&call_error),
        abi::label(&final_ok),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&call_error),
        abi::compare_immediate("x9", "2"),
        abi::branch_eq(&err_not_found),
        abi::compare_immediate("x9", "13"),
        abi::branch_eq(&err_access_denied),
        abi::branch(&err_output),
        abi::label(&invalid_path),
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
        abi::label(&err_not_found),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_NOT_FOUND_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_NOT_FOUND_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&err_access_denied),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_ACCESS_DENIED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ACCESS_DENIED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&err_output),
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

pub(super) fn lower_fs_list_directory_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 128;
    const LR_OFFSET: usize = 0;
    const PATH_OFFSET: usize = 8;
    const C_PATH_OFFSET: usize = 16;
    const DIR_OFFSET: usize = 24;
    const COUNT_OFFSET: usize = 32;
    const DATA_LEN_OFFSET: usize = 40;
    const COLLECTION_OFFSET: usize = 48;
    const ENTRY_CURSOR_OFFSET: usize = 56;
    const DATA_CURSOR_OFFSET: usize = 64;
    const DATA_OFFSET_OFFSET: usize = 72;

    let path_alloc_ok = format!("{symbol}_path_alloc_ok");
    let path_copy_loop = format!("{symbol}_path_copy_loop");
    let path_copy_done = format!("{symbol}_path_copy_done");
    let first_open_ok = format!("{symbol}_first_open_ok");
    let count_loop = format!("{symbol}_count_loop");
    let count_done = format!("{symbol}_count_done");
    let count_skip = format!("{symbol}_count_skip");
    let second_open_ok = format!("{symbol}_second_open_ok");
    let alloc_ok = format!("{symbol}_alloc_ok");
    let fill_loop = format!("{symbol}_fill_loop");
    let fill_done = format!("{symbol}_fill_done");
    let fill_skip = format!("{symbol}_fill_skip");
    let copy_name_loop = format!("{symbol}_copy_name_loop");
    let copy_name_done = format!("{symbol}_copy_name_done");
    let invalid = format!("{symbol}_invalid");
    let open_error = format!("{symbol}_open_error");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let name_offset = platform.dirent_name_offset();
    let namlen_offset = platform.dirent_name_length_offset();
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
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&path_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&path_alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), C_PATH_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), PATH_OFFSET),
        abi::load_u64("x11", "x10", 0),
        abi::add_immediate("x12", "x10", 8),
        abi::move_register("x13", "x1"),
        abi::move_immediate("x14", "Integer", "0"),
        abi::label(&path_copy_loop),
        abi::compare_registers("x14", "x11"),
        abi::branch_eq(&path_copy_done),
        abi::load_u8("x15", "x12", 0),
        abi::compare_immediate("x15", "0"),
        abi::branch_eq(&invalid),
        abi::store_u8("x15", "x13", 0),
        abi::add_immediate("x12", "x12", 1),
        abi::add_immediate("x13", "x13", 1),
        abi::add_immediate("x14", "x14", 1),
        abi::branch(&path_copy_loop),
        abi::label(&path_copy_done),
        abi::store_u8("x31", "x13", 0),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), C_PATH_OFFSET),
    ]);
    platform.emit_opendir(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_gt(&first_open_ok),
        abi::branch(&open_error),
        abi::label(&first_open_ok),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), DIR_OFFSET),
        abi::store_u64("x31", abi::stack_pointer(), COUNT_OFFSET),
        abi::store_u64("x31", abi::stack_pointer(), DATA_LEN_OFFSET),
        abi::label(&count_loop),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), DIR_OFFSET),
    ]);
    platform.emit_readdir(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    if platform.target() == "linux-aarch64" {
        let name_len_loop = format!("{symbol}_count_name_len_loop");
        let name_len_done = format!("{symbol}_count_name_len_done");
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_eq(&count_done),
            abi::add_immediate("x11", abi::return_register(), name_offset),
            abi::move_register("x13", "x11"),
            abi::move_immediate("x10", "Integer", "0"),
            abi::label(&name_len_loop),
            abi::load_u8("x12", "x13", 0),
            abi::compare_immediate("x12", "0"),
            abi::branch_eq(&name_len_done),
            abi::add_immediate("x10", "x10", 1),
            abi::add_immediate("x13", "x13", 1),
            abi::branch(&name_len_loop),
            abi::label(&name_len_done),
        ]);
    } else {
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_eq(&count_done),
            abi::load_u16("x10", abi::return_register(), namlen_offset),
            abi::add_immediate("x11", abi::return_register(), name_offset),
        ]);
    }
    instructions.extend([
        abi::compare_immediate("x10", "1"),
        abi::branch_ne(&count_skip),
        abi::load_u8("x12", "x11", 0),
        abi::compare_immediate("x12", "46"),
        abi::branch_eq(&count_loop),
        abi::label(&count_skip),
        abi::compare_immediate("x10", "2"),
        abi::branch_ne(&count_skip.replace("skip", "keep")),
    ]);
    let count_keep = count_skip.replace("skip", "keep");
    instructions.extend([
        abi::load_u8("x12", "x11", 0),
        abi::compare_immediate("x12", "46"),
        abi::branch_ne(&count_keep),
        abi::load_u8("x12", "x11", 1),
        abi::compare_immediate("x12", "46"),
        abi::branch_eq(&count_loop),
        abi::label(&count_keep),
        abi::load_u64("x12", abi::stack_pointer(), COUNT_OFFSET),
        abi::add_immediate("x12", "x12", 1),
        abi::store_u64("x12", abi::stack_pointer(), COUNT_OFFSET),
        abi::load_u64("x12", abi::stack_pointer(), DATA_LEN_OFFSET),
        abi::add_registers("x12", "x12", "x10"),
        abi::store_u64("x12", abi::stack_pointer(), DATA_LEN_OFFSET),
        abi::branch(&count_loop),
        abi::label(&count_done),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), DIR_OFFSET),
    ]);
    platform.emit_closedir(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64("x10", abi::stack_pointer(), COUNT_OFFSET),
        abi::move_immediate("x11", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("x12", "x10", "x11"),
        abi::load_u64("x13", abi::stack_pointer(), DATA_LEN_OFFSET),
        abi::add_registers("x12", "x12", "x13"),
        abi::add_immediate(abi::return_register(), "x12", COLLECTION_HEADER_SIZE),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), COLLECTION_OFFSET),
        abi::move_immediate("x9", "Byte", &COLLECTION_KIND_LIST.to_string()),
        abi::store_u8("x9", "x1", COLLECTION_OFFSET_KIND),
        abi::move_immediate("x9", "Byte", &COLLECTION_TYPE_NONE.to_string()),
        abi::store_u8("x9", "x1", COLLECTION_OFFSET_KEY_TYPE),
        abi::move_immediate("x9", "Byte", &COLLECTION_TYPE_STRING.to_string()),
        abi::store_u8("x9", "x1", COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate("x9", "Byte", "1"),
        abi::store_u8("x9", "x1", COLLECTION_OFFSET_FLAGS_VERSION),
        abi::load_u64("x10", abi::stack_pointer(), COUNT_OFFSET),
        abi::store_u64("x10", "x1", COLLECTION_OFFSET_COUNT),
        abi::store_u64("x10", "x1", COLLECTION_OFFSET_CAPACITY),
        abi::load_u64("x11", abi::stack_pointer(), DATA_LEN_OFFSET),
        abi::store_u64("x11", "x1", COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64("x11", "x1", COLLECTION_OFFSET_DATA_CAPACITY),
        abi::add_immediate("x12", "x1", COLLECTION_HEADER_SIZE),
        abi::store_u64("x12", abi::stack_pointer(), ENTRY_CURSOR_OFFSET),
        abi::move_immediate("x13", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("x14", "x10", "x13"),
        abi::add_registers("x12", "x12", "x14"),
        abi::store_u64("x12", abi::stack_pointer(), DATA_CURSOR_OFFSET),
        abi::store_u64("x31", abi::stack_pointer(), DATA_OFFSET_OFFSET),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), C_PATH_OFFSET),
    ]);
    platform.emit_opendir(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_gt(&second_open_ok),
        abi::branch(&open_error),
        abi::label(&second_open_ok),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), DIR_OFFSET),
        abi::label(&fill_loop),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), DIR_OFFSET),
    ]);
    platform.emit_readdir(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    if platform.target() == "linux-aarch64" {
        let name_len_loop = format!("{symbol}_fill_name_len_loop");
        let name_len_done = format!("{symbol}_fill_name_len_done");
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_eq(&fill_done),
            abi::add_immediate("x11", abi::return_register(), name_offset),
            abi::move_register("x13", "x11"),
            abi::move_immediate("x10", "Integer", "0"),
            abi::label(&name_len_loop),
            abi::load_u8("x12", "x13", 0),
            abi::compare_immediate("x12", "0"),
            abi::branch_eq(&name_len_done),
            abi::add_immediate("x10", "x10", 1),
            abi::add_immediate("x13", "x13", 1),
            abi::branch(&name_len_loop),
            abi::label(&name_len_done),
        ]);
    } else {
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_eq(&fill_done),
            abi::load_u16("x10", abi::return_register(), namlen_offset),
            abi::add_immediate("x11", abi::return_register(), name_offset),
        ]);
    }
    instructions.extend([
        abi::compare_immediate("x10", "1"),
        abi::branch_ne(&fill_skip),
        abi::load_u8("x12", "x11", 0),
        abi::compare_immediate("x12", "46"),
        abi::branch_eq(&fill_loop),
        abi::label(&fill_skip),
    ]);
    let fill_keep = fill_skip.replace("skip", "keep");
    instructions.extend([
        abi::compare_immediate("x10", "2"),
        abi::branch_ne(&fill_keep),
        abi::load_u8("x12", "x11", 0),
        abi::compare_immediate("x12", "46"),
        abi::branch_ne(&fill_keep),
        abi::load_u8("x12", "x11", 1),
        abi::compare_immediate("x12", "46"),
        abi::branch_eq(&fill_loop),
        abi::label(&fill_keep),
        abi::load_u64("x12", abi::stack_pointer(), ENTRY_CURSOR_OFFSET),
        abi::move_immediate("x13", "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
        abi::store_u8("x13", "x12", COLLECTION_ENTRY_OFFSET_FLAGS),
        abi::store_u64("x31", "x12", COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
        abi::store_u64("x31", "x12", COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
        abi::load_u64("x13", abi::stack_pointer(), DATA_OFFSET_OFFSET),
        abi::store_u64("x13", "x12", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::store_u64("x10", "x12", COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        abi::load_u64("x14", abi::stack_pointer(), DATA_CURSOR_OFFSET),
        abi::move_immediate("x15", "Integer", "0"),
        abi::label(&copy_name_loop),
        abi::compare_registers("x15", "x10"),
        abi::branch_eq(&copy_name_done),
        abi::load_u8("x16", "x11", 0),
        abi::store_u8("x16", "x14", 0),
        abi::add_immediate("x11", "x11", 1),
        abi::add_immediate("x14", "x14", 1),
        abi::add_immediate("x15", "x15", 1),
        abi::branch(&copy_name_loop),
        abi::label(&copy_name_done),
        abi::store_u64("x14", abi::stack_pointer(), DATA_CURSOR_OFFSET),
        abi::load_u64("x13", abi::stack_pointer(), DATA_OFFSET_OFFSET),
        abi::add_registers("x13", "x13", "x10"),
        abi::store_u64("x13", abi::stack_pointer(), DATA_OFFSET_OFFSET),
        abi::load_u64("x12", abi::stack_pointer(), ENTRY_CURSOR_OFFSET),
        abi::add_immediate("x12", "x12", COLLECTION_ENTRY_SIZE),
        abi::store_u64("x12", abi::stack_pointer(), ENTRY_CURSOR_OFFSET),
        abi::branch(&fill_loop),
        abi::label(&fill_done),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), DIR_OFFSET),
    ]);
    platform.emit_closedir(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        COLLECTION_OFFSET,
    ));
    instructions.push(abi::branch_link(SORT_STRING_LIST_SYMBOL));
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: SORT_STRING_LIST_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::load_u64(
            RESULT_VALUE_REGISTER,
            abi::stack_pointer(),
            COLLECTION_OFFSET,
        ),
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
        kind: "branch26".to_string(),
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
        kind: "branch26".to_string(),
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

pub(super) fn lower_fs_open_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    no_follow: bool,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 48;
    const LR_OFFSET: usize = 0;
    const PATH_OFFSET: usize = 8;
    const MODE_OFFSET: usize = 16;
    const C_PATH_OFFSET: usize = 24;
    const FLAGS_OFFSET: usize = 32;

    let alloc_ok = format!("{symbol}_path_alloc_ok");
    let copy_loop = format!("{symbol}_path_copy_loop");
    let copy_done = format!("{symbol}_path_copy_done");
    let invalid = format!("{symbol}_invalid");
    let read = format!("{symbol}_mode_read");
    let write = format!("{symbol}_mode_write");
    let read_write = format!("{symbol}_mode_read_write");
    let append = format!("{symbol}_mode_append");
    let flags_done = format!("{symbol}_flags_done");
    let open_ok = format!("{symbol}_open_ok");
    let file_alloc_ok = format!("{symbol}_file_alloc_ok");
    let open_error = format!("{symbol}_open_error");
    let done = format!("{symbol}_done");

    let flags = open_flag_set(platform.target(), no_follow);
    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), PATH_OFFSET),
        abi::store_u64("x1", abi::stack_pointer(), MODE_OFFSET),
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
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
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
        abi::load_u64("x9", abi::stack_pointer(), MODE_OFFSET),
        abi::load_u64("x10", "x9", 0),
    ]);
    emit_branch_if_ascii_literal(&mut instructions, "x9", "x10", b"r", &read, symbol);
    emit_branch_if_ascii_literal(&mut instructions, "x9", "x10", b"read", &read, symbol);
    emit_branch_if_ascii_literal(&mut instructions, "x9", "x10", b"w", &write, symbol);
    emit_branch_if_ascii_literal(&mut instructions, "x9", "x10", b"write", &write, symbol);
    emit_branch_if_ascii_literal(&mut instructions, "x9", "x10", b"rw", &read_write, symbol);
    emit_branch_if_ascii_literal(
        &mut instructions,
        "x9",
        "x10",
        b"readWrite",
        &read_write,
        symbol,
    );
    emit_branch_if_ascii_literal(&mut instructions, "x9", "x10", b"a", &append, symbol);
    emit_branch_if_ascii_literal(&mut instructions, "x9", "x10", b"append", &append, symbol);
    instructions.extend([
        abi::branch(&invalid),
        abi::label(&read),
        abi::move_immediate("x11", "Integer", flags.read),
        abi::branch(&flags_done),
        abi::label(&write),
        abi::move_immediate("x11", "Integer", flags.write),
        abi::branch(&flags_done),
        abi::label(&read_write),
        abi::move_immediate("x11", "Integer", flags.read_write),
        abi::branch(&flags_done),
        abi::label(&append),
        abi::move_immediate("x11", "Integer", flags.append),
        abi::label(&flags_done),
        abi::store_u64("x11", abi::stack_pointer(), FLAGS_OFFSET),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), C_PATH_OFFSET),
        abi::move_register("x1", "x11"),
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
        abi::store_u64(abi::return_register(), abi::stack_pointer(), FLAGS_OFFSET),
        abi::move_immediate(abi::return_register(), "Integer", RESOURCE_RECORD_SIZE),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&file_alloc_ok),
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
        abi::label(&file_alloc_ok),
        abi::load_u64("x9", abi::stack_pointer(), FLAGS_OFFSET),
        abi::store_u64("x9", "x1", FILE_OFFSET_FD),
        abi::store_u64("x31", "x1", FILE_OFFSET_CLOSED),
        abi::store_u64("x31", "x1", FILE_OFFSET_STATE),
        abi::move_register(RESULT_VALUE_REGISTER, "x1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
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
        no_follow,
        &mut instructions,
        &mut relocations,
        &done,
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

pub(super) fn lower_fs_close_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 16;
    let already_closed = format!("{symbol}_already_closed");
    let close_error = format!("{symbol}_close_error");
    let done = format!("{symbol}_done");
    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), 0),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), 8),
        abi::load_u64("x9", abi::return_register(), FILE_OFFSET_CLOSED),
        abi::compare_immediate("x9", "0"),
        abi::branch_ne(&already_closed),
        abi::load_u64(
            abi::return_register(),
            abi::return_register(),
            FILE_OFFSET_FD,
        ),
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
        abi::move_immediate("x9", "Integer", "1"),
        abi::load_u64("x10", abi::stack_pointer(), 8),
        abi::store_u64("x9", "x10", FILE_OFFSET_CLOSED),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&already_closed),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_RESOURCE_CLOSED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&close_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_CLOSE_FAILED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_CLOSE_FAILED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::label(&done),
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
}

pub(super) fn lower_fs_write_all_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 48;
    const FD_OFFSET: usize = 24;
    const REMAINING_OFFSET: usize = 32;
    const CURSOR_OFFSET: usize = 40;
    let loop_label = format!("{symbol}_write_loop");
    let done_write = format!("{symbol}_write_done");
    let closed = format!("{symbol}_closed");
    let write_error = format!("{symbol}_write_error");
    let done = format!("{symbol}_done");
    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), 0),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), 8),
        abi::store_u64("x1", abi::stack_pointer(), 16),
        abi::load_u64("x9", abi::return_register(), FILE_OFFSET_CLOSED),
        abi::compare_immediate("x9", "0"),
        abi::branch_ne(&closed),
        abi::load_u64("x9", abi::return_register(), FILE_OFFSET_FD),
        abi::load_u64("x10", "x1", 0),
        abi::add_immediate("x11", "x1", 8),
        abi::store_u64("x9", abi::stack_pointer(), FD_OFFSET),
        abi::store_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::store_u64("x11", abi::stack_pointer(), CURSOR_OFFSET),
        abi::label(&loop_label),
        abi::load_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::compare_immediate("x10", "0"),
        abi::branch_eq(&done_write),
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
        abi::branch(&loop_label),
        abi::label(&done_write),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&closed),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_RESOURCE_CLOSED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&write_error),
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
}

pub(super) fn lower_fs_read_all_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 80;
    const LR_OFFSET: usize = 0;
    const FILE_OFFSET: usize = 8;
    const FD_OFFSET: usize = 16;
    const START_OFFSET: usize = 24;
    const END_OFFSET: usize = 32;
    const LEN_OFFSET: usize = 40;
    const STRING_OFFSET: usize = 48;
    const REMAINING_OFFSET: usize = 56;
    const CURSOR_OFFSET: usize = 64;

    let closed = format!("{symbol}_closed");
    let seek_error = format!("{symbol}_seek_error");
    let alloc_ok = format!("{symbol}_alloc_ok");
    let alloc_error = format!("{symbol}_alloc_error");
    let read_loop = format!("{symbol}_read_loop");
    let read_done = format!("{symbol}_read_done");
    let read_error = format!("{symbol}_read_error");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), FILE_OFFSET),
        abi::load_u64("x9", abi::return_register(), FILE_OFFSET_CLOSED),
        abi::compare_immediate("x9", "0"),
        abi::branch_ne(&closed),
        abi::load_u64("x9", abi::return_register(), FILE_OFFSET_FD),
        abi::store_u64("x9", abi::stack_pointer(), FD_OFFSET),
        abi::move_register(abi::return_register(), "x9"),
        abi::move_immediate("x1", "Integer", "0"),
        abi::move_immediate("x2", "Integer", "1"),
    ]);
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&seek_error),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), START_OFFSET),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
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
        abi::branch_lt(&seek_error),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), END_OFFSET),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64("x1", abi::stack_pointer(), START_OFFSET),
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
        abi::branch_lt(&seek_error),
        abi::load_u64("x10", abi::stack_pointer(), END_OFFSET),
        abi::load_u64("x11", abi::stack_pointer(), START_OFFSET),
        abi::compare_registers("x10", "x11"),
        abi::branch_lt(&seek_error),
        abi::subtract_registers("x10", "x10", "x11"),
        abi::store_u64("x10", abi::stack_pointer(), LEN_OFFSET),
        abi::add_immediate(abi::return_register(), "x10", 9),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
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
        abi::load_u64("x0", abi::stack_pointer(), STRING_OFFSET),
        abi::load_u64("x1", "x0", 0),
        abi::add_immediate("x0", "x0", 8),
    ]);
    let encoding_error = format!("{symbol}_encoding_error");
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
        abi::label(&closed),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_RESOURCE_CLOSED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&seek_error),
        abi::label(&read_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_READ_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_READ_SYMBOL, &mut instructions, &mut relocations);
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

pub(super) fn lower_fs_write_all_bytes_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 48;
    const FD_OFFSET: usize = 24;
    const REMAINING_OFFSET: usize = 32;
    const CURSOR_OFFSET: usize = 40;
    let loop_label = format!("{symbol}_write_loop");
    let done_write = format!("{symbol}_write_done");
    let closed = format!("{symbol}_closed");
    let write_error = format!("{symbol}_write_error");
    let done = format!("{symbol}_done");
    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), 0),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), 8),
        abi::store_u64("x1", abi::stack_pointer(), 16),
        abi::load_u64("x9", abi::return_register(), FILE_OFFSET_CLOSED),
        abi::compare_immediate("x9", "0"),
        abi::branch_ne(&closed),
        abi::load_u64("x9", abi::return_register(), FILE_OFFSET_FD),
        abi::load_u64("x10", "x1", COLLECTION_OFFSET_DATA_LENGTH),
        abi::add_immediate("x11", "x1", COLLECTION_HEADER_SIZE),
        abi::load_u64("x12", "x1", COLLECTION_OFFSET_CAPACITY),
        abi::move_immediate("x13", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("x12", "x12", "x13"),
        abi::add_registers("x11", "x11", "x12"),
        abi::store_u64("x9", abi::stack_pointer(), FD_OFFSET),
        abi::store_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::store_u64("x11", abi::stack_pointer(), CURSOR_OFFSET),
        abi::label(&loop_label),
        abi::load_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::compare_immediate("x10", "0"),
        abi::branch_eq(&done_write),
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
        abi::branch(&loop_label),
        abi::label(&done_write),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&closed),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_RESOURCE_CLOSED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&write_error),
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
}

pub(super) fn lower_fs_read_all_bytes_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 112;
    const LR_OFFSET: usize = 0;
    const FD_OFFSET: usize = 8;
    const START_OFFSET: usize = 16;
    const END_OFFSET: usize = 24;
    const LEN_OFFSET: usize = 32;
    const COLLECTION_OFFSET: usize = 40;
    const DATA_OFFSET: usize = 48;
    const REMAINING_OFFSET: usize = 56;
    const CURSOR_OFFSET: usize = 64;

    let closed = format!("{symbol}_closed");
    let seek_error = format!("{symbol}_seek_error");
    let alloc_ok = format!("{symbol}_alloc_ok");
    let entry_loop = format!("{symbol}_entry_loop");
    let entry_done = format!("{symbol}_entry_done");
    let read_loop = format!("{symbol}_read_loop");
    let read_done = format!("{symbol}_read_done");
    let read_error = format!("{symbol}_read_error");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::load_u64("x9", abi::return_register(), FILE_OFFSET_CLOSED),
        abi::compare_immediate("x9", "0"),
        abi::branch_ne(&closed),
        abi::load_u64("x9", abi::return_register(), FILE_OFFSET_FD),
        abi::store_u64("x9", abi::stack_pointer(), FD_OFFSET),
        abi::move_register(abi::return_register(), "x9"),
        abi::move_immediate("x1", "Integer", "0"),
        abi::move_immediate("x2", "Integer", "1"),
    ]);
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&seek_error),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), START_OFFSET),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
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
        abi::branch_lt(&seek_error),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), END_OFFSET),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64("x1", abi::stack_pointer(), START_OFFSET),
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
        abi::branch_lt(&seek_error),
        abi::load_u64("x10", abi::stack_pointer(), END_OFFSET),
        abi::load_u64("x11", abi::stack_pointer(), START_OFFSET),
        abi::compare_registers("x10", "x11"),
        abi::branch_lt(&seek_error),
        abi::subtract_registers("x10", "x10", "x11"),
        abi::store_u64("x10", abi::stack_pointer(), LEN_OFFSET),
        abi::move_immediate("x11", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("x12", "x10", "x11"),
        abi::add_immediate("x12", "x12", COLLECTION_HEADER_SIZE),
        abi::add_registers(abi::return_register(), "x12", "x10"),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), COLLECTION_OFFSET),
        abi::move_immediate("x9", "Byte", &COLLECTION_KIND_LIST.to_string()),
        abi::store_u8("x9", "x1", COLLECTION_OFFSET_KIND),
        abi::move_immediate("x9", "Byte", &COLLECTION_TYPE_NONE.to_string()),
        abi::store_u8("x9", "x1", COLLECTION_OFFSET_KEY_TYPE),
        abi::move_immediate("x9", "Byte", &COLLECTION_TYPE_BYTE.to_string()),
        abi::store_u8("x9", "x1", COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate("x9", "Byte", "1"),
        abi::store_u8("x9", "x1", COLLECTION_OFFSET_FLAGS_VERSION),
        abi::load_u64("x10", abi::stack_pointer(), LEN_OFFSET),
        abi::store_u64("x10", "x1", COLLECTION_OFFSET_COUNT),
        abi::store_u64("x10", "x1", COLLECTION_OFFSET_CAPACITY),
        abi::store_u64("x10", "x1", COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64("x10", "x1", COLLECTION_OFFSET_DATA_CAPACITY),
        abi::add_immediate("x11", "x1", COLLECTION_HEADER_SIZE),
        abi::move_immediate("x12", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("x13", "x10", "x12"),
        abi::add_registers("x14", "x11", "x13"),
        abi::store_u64("x14", abi::stack_pointer(), DATA_OFFSET),
        abi::move_immediate("x15", "Integer", "0"),
        abi::label(&entry_loop),
        abi::compare_registers("x15", "x10"),
        abi::branch_eq(&entry_done),
        abi::move_immediate("x16", "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
        abi::store_u8("x16", "x11", COLLECTION_ENTRY_OFFSET_FLAGS),
        abi::store_u64("x31", "x11", COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
        abi::store_u64("x31", "x11", COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
        abi::store_u64("x15", "x11", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::move_immediate("x16", "Integer", "1"),
        abi::store_u64("x16", "x11", COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        abi::add_immediate("x11", "x11", COLLECTION_ENTRY_SIZE),
        abi::add_immediate("x15", "x15", 1),
        abi::branch(&entry_loop),
        abi::label(&entry_done),
        abi::load_u64("x10", abi::stack_pointer(), LEN_OFFSET),
        abi::load_u64("x11", abi::stack_pointer(), DATA_OFFSET),
        abi::store_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
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
        abi::load_u64(
            RESULT_VALUE_REGISTER,
            abi::stack_pointer(),
            COLLECTION_OFFSET,
        ),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&closed),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_RESOURCE_CLOSED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&seek_error),
        abi::label(&read_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_READ_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_READ_SYMBOL, &mut instructions, &mut relocations);
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

pub(super) fn lower_fs_eof_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 48;
    const LR_OFFSET: usize = 0;
    const FD_OFFSET: usize = 8;
    const START_OFFSET: usize = 16;
    const END_OFFSET: usize = 24;
    let closed = format!("{symbol}_closed");
    let seek_error = format!("{symbol}_seek_error");
    let at_eof = format!("{symbol}_at_eof");
    let not_eof = format!("{symbol}_not_eof");
    let done = format!("{symbol}_done");
    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::load_u64("x9", abi::return_register(), FILE_OFFSET_CLOSED),
        abi::compare_immediate("x9", "0"),
        abi::branch_ne(&closed),
        abi::load_u64("x9", abi::return_register(), FILE_OFFSET_FD),
        abi::store_u64("x9", abi::stack_pointer(), FD_OFFSET),
        abi::move_register(abi::return_register(), "x9"),
        abi::move_immediate("x1", "Integer", "0"),
        abi::move_immediate("x2", "Integer", "1"),
    ]);
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&seek_error),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), START_OFFSET),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
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
        abi::branch_lt(&seek_error),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), END_OFFSET),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64("x1", abi::stack_pointer(), START_OFFSET),
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
        abi::branch_lt(&seek_error),
        abi::load_u64("x10", abi::stack_pointer(), START_OFFSET),
        abi::load_u64("x11", abi::stack_pointer(), END_OFFSET),
        abi::compare_registers("x10", "x11"),
        abi::branch_ge(&at_eof),
        abi::branch(&not_eof),
        abi::label(&at_eof),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&not_eof),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&closed),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_RESOURCE_CLOSED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&seek_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_READ_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_READ_SYMBOL, &mut instructions, &mut relocations);
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

pub(super) fn lower_fs_canonical_path_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 80;
    const LR_OFFSET: usize = 0;
    const PATH_OFFSET: usize = 8;
    const C_PATH_OFFSET: usize = 16;
    const BUFFER_OFFSET: usize = 24;
    const LENGTH_OFFSET: usize = 32;
    const RESULT_OFFSET: usize = 40;
    const PATH_MAX_PLUS_NUL: usize = 4097;

    let path_alloc_ok = format!("{symbol}_path_alloc_ok");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let buffer_alloc_ok = format!("{symbol}_buffer_alloc_ok");
    let realpath_ok = format!("{symbol}_realpath_ok");
    let length_loop = format!("{symbol}_length_loop");
    let length_done = format!("{symbol}_length_done");
    let result_alloc_ok = format!("{symbol}_result_alloc_ok");
    let result_copy_loop = format!("{symbol}_result_copy_loop");
    let result_copy_done = format!("{symbol}_result_copy_done");
    let invalid = format!("{symbol}_invalid");
    let alloc_error = format!("{symbol}_alloc_error");
    let realpath_error = format!("{symbol}_realpath_error");
    let done = format!("{symbol}_done");

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
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&path_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&path_alloc_ok),
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
        abi::move_immediate(
            abi::return_register(),
            "Integer",
            &PATH_MAX_PLUS_NUL.to_string(),
        ),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&buffer_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&buffer_alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), BUFFER_OFFSET),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), C_PATH_OFFSET),
        abi::load_u64("x1", abi::stack_pointer(), BUFFER_OFFSET),
    ]);
    platform.emit_realpath(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&realpath_ok),
        abi::branch(&realpath_error),
        abi::label(&realpath_ok),
        abi::load_u64("x10", abi::stack_pointer(), BUFFER_OFFSET),
        abi::move_immediate("x11", "Integer", "0"),
        abi::label(&length_loop),
        abi::add_registers("x12", "x10", "x11"),
        abi::load_u8("x13", "x12", 0),
        abi::compare_immediate("x13", "0"),
        abi::branch_eq(&length_done),
        abi::add_immediate("x11", "x11", 1),
        abi::branch(&length_loop),
        abi::label(&length_done),
        abi::store_u64("x11", abi::stack_pointer(), LENGTH_OFFSET),
        abi::add_immediate(abi::return_register(), "x11", 9),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&result_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&result_alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), RESULT_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), LENGTH_OFFSET),
        abi::store_u64("x10", "x1", 0),
        abi::load_u64("x11", abi::stack_pointer(), BUFFER_OFFSET),
        abi::add_immediate("x12", "x1", 8),
        abi::label(&result_copy_loop),
        abi::compare_immediate("x10", "0"),
        abi::branch_eq(&result_copy_done),
        abi::load_u8("x13", "x11", 0),
        abi::store_u8("x13", "x12", 0),
        abi::add_immediate("x11", "x11", 1),
        abi::add_immediate("x12", "x12", 1),
        abi::subtract_immediate("x10", "x10", 1),
        abi::branch(&result_copy_loop),
        abi::label(&result_copy_done),
        abi::store_u8("x31", "x12", 0),
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), RESULT_OFFSET),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&realpath_error),
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

pub(super) fn lower_fs_is_within_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 112;
    const LR_OFFSET: usize = 0;
    const BASE_OFFSET: usize = 8;
    const CHILD_OFFSET: usize = 16;
    const C_BASE_OFFSET: usize = 24;
    const C_CHILD_OFFSET: usize = 32;
    const BASE_BUFFER_OFFSET: usize = 40;
    const CHILD_BUFFER_OFFSET: usize = 48;
    const PATH_MAX_PLUS_NUL: usize = 4097;

    let base_alloc_ok = format!("{symbol}_base_alloc_ok");
    let child_alloc_ok = format!("{symbol}_child_alloc_ok");
    let base_copy_loop = format!("{symbol}_base_copy_loop");
    let base_copy_done = format!("{symbol}_base_copy_done");
    let child_copy_loop = format!("{symbol}_child_copy_loop");
    let child_copy_done = format!("{symbol}_child_copy_done");
    let base_buffer_alloc_ok = format!("{symbol}_base_buffer_alloc_ok");
    let child_buffer_alloc_ok = format!("{symbol}_child_buffer_alloc_ok");
    let base_realpath_ok = format!("{symbol}_base_realpath_ok");
    let child_realpath_ok = format!("{symbol}_child_realpath_ok");
    let root_true = format!("{symbol}_root_true");
    let compare_loop = format!("{symbol}_compare_loop");
    let base_ended = format!("{symbol}_base_ended");
    let true_label = format!("{symbol}_true");
    let false_label = format!("{symbol}_false");
    let invalid = format!("{symbol}_invalid");
    let alloc_error = format!("{symbol}_alloc_error");
    let realpath_error = format!("{symbol}_realpath_error");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), BASE_OFFSET),
        abi::store_u64("x1", abi::stack_pointer(), CHILD_OFFSET),
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
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&base_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&base_alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), C_BASE_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), BASE_OFFSET),
        abi::load_u64("x11", "x10", 0),
        abi::add_immediate("x12", "x10", 8),
        abi::move_register("x13", "x1"),
        abi::move_immediate("x14", "Integer", "0"),
        abi::label(&base_copy_loop),
        abi::compare_registers("x14", "x11"),
        abi::branch_eq(&base_copy_done),
        abi::load_u8("x15", "x12", 0),
        abi::compare_immediate("x15", "0"),
        abi::branch_eq(&invalid),
        abi::store_u8("x15", "x13", 0),
        abi::add_immediate("x12", "x12", 1),
        abi::add_immediate("x13", "x13", 1),
        abi::add_immediate("x14", "x14", 1),
        abi::branch(&base_copy_loop),
        abi::label(&base_copy_done),
        abi::store_u8("x31", "x13", 0),
        abi::load_u64("x9", abi::stack_pointer(), CHILD_OFFSET),
        abi::load_u64(abi::return_register(), "x9", 0),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&invalid),
        abi::add_immediate(abi::return_register(), abi::return_register(), 1),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&child_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&child_alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), C_CHILD_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), CHILD_OFFSET),
        abi::load_u64("x11", "x10", 0),
        abi::add_immediate("x12", "x10", 8),
        abi::move_register("x13", "x1"),
        abi::move_immediate("x14", "Integer", "0"),
        abi::label(&child_copy_loop),
        abi::compare_registers("x14", "x11"),
        abi::branch_eq(&child_copy_done),
        abi::load_u8("x15", "x12", 0),
        abi::compare_immediate("x15", "0"),
        abi::branch_eq(&invalid),
        abi::store_u8("x15", "x13", 0),
        abi::add_immediate("x12", "x12", 1),
        abi::add_immediate("x13", "x13", 1),
        abi::add_immediate("x14", "x14", 1),
        abi::branch(&child_copy_loop),
        abi::label(&child_copy_done),
        abi::store_u8("x31", "x13", 0),
        abi::move_immediate(
            abi::return_register(),
            "Integer",
            &PATH_MAX_PLUS_NUL.to_string(),
        ),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&base_buffer_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&base_buffer_alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), BASE_BUFFER_OFFSET),
        abi::move_immediate(
            abi::return_register(),
            "Integer",
            &PATH_MAX_PLUS_NUL.to_string(),
        ),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&child_buffer_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&child_buffer_alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), CHILD_BUFFER_OFFSET),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), C_BASE_OFFSET),
        abi::load_u64("x1", abi::stack_pointer(), BASE_BUFFER_OFFSET),
    ]);
    platform.emit_realpath(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&base_realpath_ok),
        abi::branch(&realpath_error),
        abi::label(&base_realpath_ok),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), C_CHILD_OFFSET),
        abi::load_u64("x1", abi::stack_pointer(), CHILD_BUFFER_OFFSET),
    ]);
    platform.emit_realpath(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&child_realpath_ok),
        abi::branch(&realpath_error),
        abi::label(&child_realpath_ok),
        abi::load_u64("x10", abi::stack_pointer(), BASE_BUFFER_OFFSET),
        abi::load_u64("x11", abi::stack_pointer(), CHILD_BUFFER_OFFSET),
        abi::load_u8("x12", "x10", 0),
        abi::compare_immediate("x12", "47"),
        abi::branch_ne(&compare_loop),
        abi::load_u8("x12", "x10", 1),
        abi::compare_immediate("x12", "0"),
        abi::branch_eq(&root_true),
        abi::label(&compare_loop),
        abi::load_u8("x12", "x10", 0),
        abi::load_u8("x13", "x11", 0),
        abi::compare_immediate("x12", "0"),
        abi::branch_eq(&base_ended),
        abi::compare_registers("x12", "x13"),
        abi::branch_ne(&false_label),
        abi::add_immediate("x10", "x10", 1),
        abi::add_immediate("x11", "x11", 1),
        abi::branch(&compare_loop),
        abi::label(&base_ended),
        abi::compare_immediate("x13", "0"),
        abi::branch_eq(&true_label),
        abi::compare_immediate("x13", "47"),
        abi::branch_eq(&true_label),
        abi::branch(&false_label),
        abi::label(&root_true),
        abi::label(&true_label),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&false_label),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&realpath_error),
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
        kind: "branch26".to_string(),
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
        kind: "branch26".to_string(),
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
        kind: "branch26".to_string(),
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
        kind: "branch26".to_string(),
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
        kind: "branch26".to_string(),
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
        kind: "branch26".to_string(),
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
        kind: "branch26".to_string(),
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
        kind: "branch26".to_string(),
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
        kind: "branch26".to_string(),
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
        kind: "branch26".to_string(),
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

pub(super) fn lower_fs_read_line_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 128;
    const LR_OFFSET: usize = 0;
    const FD_OFFSET: usize = 8;
    const START_OFFSET: usize = 16;
    const END_OFFSET: usize = 24;
    const LEN_OFFSET: usize = 32;
    const TEMP_OFFSET: usize = 40;
    const REMAINING_OFFSET: usize = 48;
    const CURSOR_OFFSET: usize = 56;
    const LINE_LEN_OFFSET: usize = 64;
    const CONSUMED_OFFSET: usize = 72;
    const RESULT_OFFSET: usize = 80;

    let closed = format!("{symbol}_closed");
    let seek_error = format!("{symbol}_seek_error");
    let eof_error = format!("{symbol}_eof_error");
    let temp_alloc_ok = format!("{symbol}_temp_alloc_ok");
    let read_loop = format!("{symbol}_read_loop");
    let read_done = format!("{symbol}_read_done");
    let read_error = format!("{symbol}_read_error");
    let scan_loop = format!("{symbol}_scan_loop");
    let scan_no_newline = format!("{symbol}_scan_no_newline");
    let scan_newline = format!("{symbol}_scan_newline");
    let trim_done = format!("{symbol}_trim_done");
    let result_alloc_ok = format!("{symbol}_result_alloc_ok");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::load_u64("x9", abi::return_register(), FILE_OFFSET_CLOSED),
        abi::compare_immediate("x9", "0"),
        abi::branch_ne(&closed),
        abi::load_u64("x9", abi::return_register(), FILE_OFFSET_FD),
        abi::store_u64("x9", abi::stack_pointer(), FD_OFFSET),
        abi::move_register(abi::return_register(), "x9"),
        abi::move_immediate("x1", "Integer", "0"),
        abi::move_immediate("x2", "Integer", "1"),
    ]);
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&seek_error),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), START_OFFSET),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
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
        abi::branch_lt(&seek_error),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), END_OFFSET),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64("x1", abi::stack_pointer(), START_OFFSET),
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
        abi::branch_lt(&seek_error),
        abi::load_u64("x10", abi::stack_pointer(), END_OFFSET),
        abi::load_u64("x11", abi::stack_pointer(), START_OFFSET),
        abi::compare_registers("x10", "x11"),
        abi::branch_le(&eof_error),
        abi::subtract_registers("x10", "x10", "x11"),
        abi::store_u64("x10", abi::stack_pointer(), LEN_OFFSET),
        abi::add_immediate(abi::return_register(), "x10", 9),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&temp_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&temp_alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), TEMP_OFFSET),
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
        abi::load_u64("x10", abi::stack_pointer(), TEMP_OFFSET),
        abi::add_immediate("x11", "x10", 8),
        abi::load_u64("x12", abi::stack_pointer(), LEN_OFFSET),
        abi::move_immediate("x13", "Integer", "0"),
        abi::move_immediate("x14", "Integer", "0"),
        abi::label(&scan_loop),
        abi::compare_immediate("x12", "0"),
        abi::branch_eq(&scan_no_newline),
        abi::load_u8("x15", "x11", 0),
        abi::add_immediate("x14", "x14", 1),
        abi::compare_immediate("x15", "10"),
        abi::branch_eq(&scan_newline),
        abi::add_immediate("x13", "x13", 1),
        abi::add_immediate("x11", "x11", 1),
        abi::subtract_immediate("x12", "x12", 1),
        abi::branch(&scan_loop),
        abi::label(&scan_no_newline),
        abi::store_u64("x13", abi::stack_pointer(), LINE_LEN_OFFSET),
        abi::store_u64("x13", abi::stack_pointer(), CONSUMED_OFFSET),
        abi::branch(&trim_done),
        abi::label(&scan_newline),
        abi::store_u64("x13", abi::stack_pointer(), LINE_LEN_OFFSET),
        abi::store_u64("x14", abi::stack_pointer(), CONSUMED_OFFSET),
        abi::compare_immediate("x13", "0"),
        abi::branch_eq(&trim_done),
        abi::subtract_immediate("x16", "x11", 1),
        abi::load_u8("x15", "x16", 0),
        abi::compare_immediate("x15", "13"),
        abi::branch_ne(&trim_done),
        abi::subtract_immediate("x13", "x13", 1),
        abi::store_u64("x13", abi::stack_pointer(), LINE_LEN_OFFSET),
        abi::label(&trim_done),
        abi::load_u64("x10", abi::stack_pointer(), START_OFFSET),
        abi::load_u64("x11", abi::stack_pointer(), CONSUMED_OFFSET),
        abi::add_registers("x1", "x10", "x11"),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
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
        abi::branch_lt(&seek_error),
        abi::load_u64("x10", abi::stack_pointer(), LINE_LEN_OFFSET),
        abi::add_immediate(abi::return_register(), "x10", 9),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&result_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&result_alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), RESULT_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), LINE_LEN_OFFSET),
        abi::store_u64("x10", "x1", 0),
        abi::add_immediate("x11", "x1", 8),
        abi::load_u64("x12", abi::stack_pointer(), TEMP_OFFSET),
        abi::add_immediate("x12", "x12", 8),
        abi::label(&copy_loop),
        abi::compare_immediate("x10", "0"),
        abi::branch_eq(&copy_done),
        abi::load_u8("x13", "x12", 0),
        abi::store_u8("x13", "x11", 0),
        abi::add_immediate("x12", "x12", 1),
        abi::add_immediate("x11", "x11", 1),
        abi::subtract_immediate("x10", "x10", 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8("x31", "x11", 0),
        abi::load_u64("x0", abi::stack_pointer(), RESULT_OFFSET),
        abi::load_u64("x1", "x0", 0),
        abi::add_immediate("x0", "x0", 8),
    ]);
    let encoding_error = format!("{symbol}_encoding_error");
    emit_call_validate_utf8(symbol, &encoding_error, &mut instructions, &mut relocations);
    instructions.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), RESULT_OFFSET),
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
        abi::label(&closed),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_RESOURCE_CLOSED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&eof_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_EOF_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_EOF_SYMBOL, &mut instructions, &mut relocations);
    instructions.extend([
        abi::branch(&done),
        abi::label(&seek_error),
        abi::label(&read_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_READ_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_READ_SYMBOL, &mut instructions, &mut relocations);
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

struct OpenFlagSet {
    read: &'static str,
    write: &'static str,
    read_write: &'static str,
    append: &'static str,
}

fn open_flag_set(target: &str, no_follow: bool) -> OpenFlagSet {
    match (target, no_follow) {
        ("linux-aarch64", false) => OpenFlagSet {
            read: "0",
            write: "577",
            read_write: "66",
            append: "1089",
        },
        ("linux-aarch64", true) => OpenFlagSet {
            read: "32768",
            write: "33345",
            read_write: "32834",
            append: "33857",
        },
        (_, false) => OpenFlagSet {
            read: "0",
            write: "1537",
            read_write: "514",
            append: "521",
        },
        (_, true) => OpenFlagSet {
            read: "256",
            write: "1793",
            read_write: "770",
            append: "777",
        },
    }
}

fn emit_branch_if_ascii_literal(
    instructions: &mut Vec<CodeInstruction>,
    ptr: &str,
    len: &str,
    literal: &[u8],
    target: &str,
    symbol: &str,
) {
    let next = format!(
        "{symbol}_literal_{}_{}",
        target.rsplit('_').next().unwrap_or("next"),
        literal.len()
    );
    instructions.extend([
        abi::compare_immediate(len, &literal.len().to_string()),
        abi::branch_ne(&next),
    ]);
    for (index, byte) in literal.iter().enumerate() {
        instructions.extend([
            abi::load_u8("x12", ptr, 8 + index),
            abi::compare_immediate("x12", &byte.to_string()),
            abi::branch_ne(&next),
        ]);
    }
    instructions.extend([abi::branch(target), abi::label(&next)]);
}

fn emit_errno_error_mapping(
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    done: &str,
) {
    let err_not_found = format!("{symbol}_errno_not_found");
    let err_access_denied = format!("{symbol}_errno_access_denied");
    let err_already_exists = format!("{symbol}_errno_already_exists");
    let err_output = format!("{symbol}_errno_output");
    instructions.extend([
        abi::compare_immediate("x9", "2"),
        abi::branch_eq(&err_not_found),
        abi::compare_immediate("x9", "13"),
        abi::branch_eq(&err_access_denied),
        abi::compare_immediate("x9", "17"),
        abi::branch_eq(&err_already_exists),
        abi::branch(&err_output),
        abi::label(&err_not_found),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_NOT_FOUND_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_NOT_FOUND_SYMBOL, instructions, relocations);
    instructions.extend([
        abi::branch(done),
        abi::label(&err_access_denied),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_ACCESS_DENIED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_ACCESS_DENIED_SYMBOL, instructions, relocations);
    instructions.extend([
        abi::branch(done),
        abi::label(&err_already_exists),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_ALREADY_EXISTS_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_ALREADY_EXISTS_SYMBOL, instructions, relocations);
    instructions.extend([
        abi::branch(done),
        abi::label(&err_output),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUTPUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_OUTPUT_SYMBOL, instructions, relocations);
}

/// Filesystem-context errno mapping for path-based helpers.
///
/// Like [`emit_errno_error_mapping`], but maps missing paths to the
/// filesystem-specific `ErrPathNotFound` instead of the generic `ErrNotFound`,
/// routes host errnos that indicate an unusable path string to `ErrInvalidPath`,
/// and (for no-follow opens) maps a final-symlink `ELOOP` to `ErrAccessDenied`.
/// The host errno is expected in `x9`, as produced by `emit_errno`.
fn emit_fs_path_errno_error_mapping(
    symbol: &str,
    target: &str,
    no_follow: bool,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    done: &str,
) {
    let linux = target == "linux-aarch64";
    let eloop = if linux { "40" } else { "62" };
    let enametoolong = if linux { "36" } else { "63" };
    let eilseq = if linux { "84" } else { "92" };
    let enotempty = if linux { "39" } else { "66" };

    let err_path_not_found = format!("{symbol}_errno_path_not_found");
    let err_access_denied = format!("{symbol}_errno_access_denied");
    let err_already_exists = format!("{symbol}_errno_already_exists");
    let err_not_empty = format!("{symbol}_errno_not_empty");
    let err_invalid_path = format!("{symbol}_errno_invalid_path");
    let err_output = format!("{symbol}_errno_output");
    let eloop_target = if no_follow {
        err_access_denied.clone()
    } else {
        err_invalid_path.clone()
    };

    instructions.extend([
        abi::compare_immediate("x9", "2"),
        abi::branch_eq(&err_path_not_found),
        abi::compare_immediate("x9", "13"),
        abi::branch_eq(&err_access_denied),
        abi::compare_immediate("x9", "17"),
        abi::branch_eq(&err_already_exists),
        abi::compare_immediate("x9", enotempty),
        abi::branch_eq(&err_not_empty),
        abi::compare_immediate("x9", "20"),
        abi::branch_eq(&err_invalid_path),
        abi::compare_immediate("x9", enametoolong),
        abi::branch_eq(&err_invalid_path),
        abi::compare_immediate("x9", eilseq),
        abi::branch_eq(&err_invalid_path),
        abi::compare_immediate("x9", eloop),
        abi::branch_eq(&eloop_target),
        abi::branch(&err_output),
        abi::label(&err_path_not_found),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_PATH_NOT_FOUND_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_PATH_NOT_FOUND_SYMBOL, instructions, relocations);
    instructions.extend([
        abi::branch(done),
        abi::label(&err_access_denied),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_ACCESS_DENIED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_ACCESS_DENIED_SYMBOL, instructions, relocations);
    instructions.extend([
        abi::branch(done),
        abi::label(&err_already_exists),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_ALREADY_EXISTS_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_ALREADY_EXISTS_SYMBOL, instructions, relocations);
    instructions.extend([
        abi::branch(done),
        abi::label(&err_not_empty),
        abi::move_immediate(
            RESULT_VALUE_REGISTER,
            "Integer",
            ERR_DIRECTORY_NOT_EMPTY_CODE,
        ),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_DIRECTORY_NOT_EMPTY_SYMBOL,
        instructions,
        relocations,
    );
    instructions.extend([
        abi::branch(done),
        abi::label(&err_invalid_path),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INVALID_PATH_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_INVALID_PATH_SYMBOL, instructions, relocations);
    instructions.extend([
        abi::branch(done),
        abi::label(&err_output),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUTPUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_OUTPUT_SYMBOL, instructions, relocations);
    instructions.push(abi::branch(done));
}

/// Symbol of the shared standalone UTF-8 validation runtime helper.
pub(super) const VALIDATE_UTF8_SYMBOL: &str = "_mfb_rt_validate_utf8";

/// Symbol of the shared standalone string-list sort runtime helper.
pub(super) const SORT_STRING_LIST_SYMBOL: &str = "_mfb_rt_sort_string_list";

/// Symbol of the shared standalone `fs::pathJoin` runtime helper.
pub(super) const FS_PATH_JOIN_SYMBOL: &str = "_mfb_rt_fs_path_join";

/// Lower the standalone `fs::pathJoin` helper. It takes a `List OF String`
/// collection pointer in `x0` and returns a `Result`-shaped value: `x0` holds
/// the tag (`RESULT_OK_TAG`/`RESULT_ERR_TAG`) and, on success, `x1` holds the
/// resulting `String` pointer (on allocation failure it returns `ErrOutOfMemory`).
/// Implementing it as a shared `bl`-reachable helper lets both root native code
/// and imported-package binary_repr lower `pathJoin` identically. Components are
/// joined with `/`, empty components are skipped, an absolute component discards
/// everything accumulated so far, and duplicate separators are avoided.
pub(super) fn lower_fs_path_join_helper(platform: &dyn CodegenPlatform) -> CodeFunction {
    const SEP: &str = "47";
    const FRAME_SIZE: usize = 32;
    const LR_OFFSET: usize = 0;
    const PARTS_OFFSET: usize = 8;
    const RESULT_OFFSET: usize = 16;
    let symbol = FS_PATH_JOIN_SYMBOL;
    let _ = platform;

    let length_loop = format!("{symbol}_length_loop");
    let length_done = format!("{symbol}_length_done");
    let alloc_ok = format!("{symbol}_alloc_ok");
    let alloc_error = format!("{symbol}_alloc_error");
    let build_loop = format!("{symbol}_build_loop");
    let build_done = format!("{symbol}_build_done");
    let skip_part = format!("{symbol}_skip_part");
    let absolute = format!("{symbol}_absolute");
    let copy_part = format!("{symbol}_copy_part");
    let no_separator = format!("{symbol}_no_separator");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let done = format!("{symbol}_done");

    let entry_size = COLLECTION_ENTRY_SIZE.to_string();
    let mut instructions = vec![
        abi::label("entry"),
        abi::subtract_stack(FRAME_SIZE),
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), PARTS_OFFSET),
        // Pass 1: upper-bound length = sum(component lengths) + count separators.
        abi::load_u64("x9", abi::return_register(), COLLECTION_OFFSET_COUNT),
        abi::move_immediate("x11", "Integer", "0"),
        abi::move_immediate("x12", "Integer", "0"),
        abi::add_immediate("x13", abi::return_register(), COLLECTION_HEADER_SIZE),
        abi::label(&length_loop),
        abi::compare_registers("x12", "x9"),
        abi::branch_ge(&length_done),
        abi::load_u64("x14", "x13", COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        abi::add_registers("x11", "x11", "x14"),
        abi::add_immediate("x13", "x13", COLLECTION_ENTRY_SIZE),
        abi::add_immediate("x12", "x12", 1),
        abi::branch(&length_loop),
        abi::label(&length_done),
        abi::add_registers(abi::return_register(), "x11", "x9"),
        abi::add_immediate(abi::return_register(), abi::return_register(), 9),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ];
    let mut relocations = vec![CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    }];
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), RESULT_OFFSET),
        // Pass 2: build the joined path.
        abi::load_u64("x16", abi::stack_pointer(), PARTS_OFFSET),
        abi::load_u64("x9", "x16", COLLECTION_OFFSET_COUNT),
        // data base = collection + header + capacity * entry_size (plan-01 §4.2:
        // a grown list has capacity > count, so the data region sits past the
        // full lookup capacity, not just the live entries).
        abi::load_u64("x8", "x16", COLLECTION_OFFSET_CAPACITY),
        abi::add_immediate("x14", "x16", COLLECTION_HEADER_SIZE),
        abi::move_immediate("x5", "Integer", &entry_size),
        abi::multiply_registers("x5", "x8", "x5"),
        abi::add_registers("x14", "x14", "x5"),
        abi::add_immediate("x15", "x16", COLLECTION_HEADER_SIZE),
        abi::load_u64("x1", abi::stack_pointer(), RESULT_OFFSET),
        abi::add_immediate("x6", "x1", 8),
        abi::move_register("x13", "x6"),
        abi::move_immediate("x12", "Integer", "0"),
        abi::label(&build_loop),
        abi::compare_registers("x12", "x9"),
        abi::branch_ge(&build_done),
        abi::load_u64("x3", "x15", COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        abi::compare_immediate("x3", "0"),
        abi::branch_eq(&skip_part),
        abi::load_u64("x2", "x15", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::add_registers("x2", "x14", "x2"),
        abi::load_u8("x4", "x2", 0),
        abi::compare_immediate("x4", SEP),
        abi::branch_eq(&absolute),
        abi::compare_registers("x13", "x6"),
        abi::branch_eq(&no_separator),
        abi::subtract_immediate("x7", "x13", 1),
        abi::load_u8("x5", "x7", 0),
        abi::compare_immediate("x5", SEP),
        abi::branch_eq(&no_separator),
        abi::move_immediate("x5", "Byte", SEP),
        abi::store_u8("x5", "x13", 0),
        abi::add_immediate("x13", "x13", 1),
        abi::branch(&copy_part),
        abi::label(&absolute),
        abi::move_register("x13", "x6"),
        abi::label(&no_separator),
        abi::label(&copy_part),
        abi::label(&copy_loop),
        abi::compare_immediate("x3", "0"),
        abi::branch_eq(&copy_done),
        abi::load_u8("x4", "x2", 0),
        abi::store_u8("x4", "x13", 0),
        abi::add_immediate("x2", "x2", 1),
        abi::add_immediate("x13", "x13", 1),
        abi::subtract_immediate("x3", "x3", 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::label(&skip_part),
        abi::add_immediate("x15", "x15", COLLECTION_ENTRY_SIZE),
        abi::add_immediate("x12", "x12", 1),
        abi::branch(&build_loop),
        abi::label(&build_done),
        abi::subtract_registers("x4", "x13", "x6"),
        abi::load_u64("x1", abi::stack_pointer(), RESULT_OFFSET),
        abi::store_u64("x4", "x1", 0),
        abi::move_immediate("x5", "Integer", "0"),
        abi::store_u8("x5", "x13", 0),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
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
    CodeFunction {
        name: "runtime.fsPathJoin".to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "String".to_string(),
        frame: CodeFrame {
            stack_size: FRAME_SIZE,
            callee_saved: vec![abi::link_register().to_string()],
        },
        stack_slots: Vec::new(),
        instructions,
        relocations,
    }
}
