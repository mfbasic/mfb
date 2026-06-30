use super::*;

pub(super) fn lower_fs_exists_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>, Vec<CodeStackSlot>), String> {
    // Vreg-allocated (plan-00-G Phase 2). The path pointer is held across the
    // `arena_alloc` call and the allocated C-string across the libc `stat`; as
    // vregs the allocator spills the former and keeps the latter in a callee-saved
    // register across the (PCS) libc call, replacing the old manual stack slots.
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let alloc_ok = format!("{symbol}_alloc_ok");
    let exists = format!("{symbol}_exists");
    let missing = format!("{symbol}_missing");
    let done = format!("{symbol}_done");

    let mut vregs = Vregs::new();
    let path = vregs.next();
    let alloc = vregs.next();
    let len0 = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&path, abi::return_register()),
        abi::load_u64(&len0, &path, 0),
        abi::add_immediate(abi::return_register(), &len0, 1),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ];
    let mut relocations = vec![CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    }];
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
            kind: RelocIntent::DataAddrHi,
            binding: "data".to_string(),
            library: None,
        },
        CodeRelocation {
            from: symbol.to_string(),
            to: alloc_symbol,
            kind: RelocIntent::DataAddrLo,
            binding: "data".to_string(),
            library: None,
        },
    ]);
    let len = vregs.next();
    let src = vregs.next();
    let dst = vregs.next();
    let index = vregs.next();
    let byte = vregs.next();
    instructions.extend([
        abi::branch(&done),
        abi::label(&alloc_ok),
        abi::move_register(&alloc, "x1"),
        abi::load_u64(&len, &path, 0),
        abi::add_immediate(&src, &path, 8),
        abi::move_register(&dst, &alloc),
        abi::move_immediate(&index, "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers(&index, &len),
        abi::branch_eq(&copy_done),
        abi::load_u8(&byte, &src, 0),
        abi::store_u8(&byte, &dst, 0),
        abi::add_immediate(&src, &src, 1),
        abi::add_immediate(&dst, &dst, 1),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8("x31", &dst, 0),
        abi::move_register(abi::return_register(), &alloc),
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
        abi::return_(),
    ]);

    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

pub(super) fn lower_fs_kind_exists_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    expected_kind: &str,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>, Vec<CodeStackSlot>), String> {
    // Vreg-allocated (plan-00-G Phase 2). The `stat` struct the syscall fills is an
    // explicit on-stack buffer (`finalize_vreg_body_with_locals`) at `sp + 0`; the
    // path pointer (held across `arena_alloc`) spills and the allocated C-string
    // (held across the libc `stat`) stays in a callee-saved register.
    const STAT_OFFSET: usize = 0;
    const STAT_BUF_SIZE: usize = 256;

    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let alloc_ok = format!("{symbol}_alloc_ok");
    let found = format!("{symbol}_found");
    let missing = format!("{symbol}_missing");
    let done = format!("{symbol}_done");

    let mut vregs = Vregs::new();
    let path = vregs.next();
    let alloc = vregs.next();
    let len0 = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&path, abi::return_register()),
        abi::load_u64(&len0, &path, 0),
        abi::add_immediate(abi::return_register(), &len0, 1),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ];
    let mut relocations = vec![CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    }];
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
            kind: RelocIntent::DataAddrHi,
            binding: "data".to_string(),
            library: None,
        },
        CodeRelocation {
            from: symbol.to_string(),
            to: alloc_symbol,
            kind: RelocIntent::DataAddrLo,
            binding: "data".to_string(),
            library: None,
        },
    ]);
    let len = vregs.next();
    let src = vregs.next();
    let dst = vregs.next();
    let index = vregs.next();
    let byte = vregs.next();
    let mode = vregs.next();
    let mask = vregs.next();
    let expected = vregs.next();
    instructions.extend([
        abi::branch(&done),
        abi::label(&alloc_ok),
        abi::move_register(&alloc, "x1"),
        abi::load_u64(&len, &path, 0),
        abi::add_immediate(&src, &path, 8),
        abi::move_register(&dst, &alloc),
        abi::move_immediate(&index, "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers(&index, &len),
        abi::branch_eq(&copy_done),
        abi::load_u8(&byte, &src, 0),
        abi::store_u8(&byte, &dst, 0),
        abi::add_immediate(&src, &src, 1),
        abi::add_immediate(&dst, &dst, 1),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8("x31", &dst, 0),
        abi::move_register(abi::return_register(), &alloc),
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
            &mode,
            abi::stack_pointer(),
            STAT_OFFSET + platform.stat_mode_offset(),
        ),
        abi::move_immediate(&mask, "Integer", FS_MODE_TYPE_MASK),
        abi::and_registers(&mode, &mode, &mask),
        abi::move_immediate(&expected, "Integer", expected_kind),
        abi::compare_registers(&mode, &expected),
        abi::branch_eq(&found),
        abi::label(&missing),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&found),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::label(&done),
        abi::return_(),
    ]);

    let (frame, stack_slots) =
        finalize_vreg_body_with_locals(&mut instructions, &[], STAT_BUF_SIZE);
    Ok((frame, instructions, relocations, stack_slots))
}

pub(super) fn lower_fs_current_directory_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>, Vec<CodeStackSlot>), String> {
    // Vreg-allocated (plan-00-G Phase 2). The `getcwd` buffer is arena-allocated
    // (not on-stack); the buffer pointer and the measured length are held across
    // the second `arena_alloc`, so as vregs the allocator keeps them in callee-saved
    // registers / spills them, replacing the old BUFFER_OFFSET/LENGTH_OFFSET slots.
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

    let mut vregs = Vregs::new();
    let buffer = vregs.next();
    let length = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_immediate(abi::return_register(), "Integer", GETCWD_CAPACITY),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ];
    let mut relocations = vec![CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    }];
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&temp_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&temp_alloc_ok),
        abi::move_register(&buffer, "x1"),
        abi::move_register(abi::return_register(), "x1"),
        abi::move_immediate("x1", "Integer", GETCWD_CAPACITY),
    ]);
    platform.emit_current_directory(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    let cursor = vregs.next();
    let byte = vregs.next();
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(&read_error),
        abi::move_register(&cursor, &buffer),
        abi::move_immediate(&length, "Integer", "0"),
        abi::label(&count_loop),
        abi::load_u8(&byte, &cursor, 0),
        abi::compare_immediate(&byte, "0"),
        abi::branch_eq(&count_done),
        abi::add_immediate(&cursor, &cursor, 1),
        abi::add_immediate(&length, &length, 1),
        abi::branch(&count_loop),
        abi::label(&count_done),
        abi::add_immediate(abi::return_register(), &length, 9),
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
    let src = vregs.next();
    let dst = vregs.next();
    let index = vregs.next();
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&string_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&string_alloc_ok),
        abi::store_u64(&length, "x1", 0),
        abi::move_register(&src, &buffer),
        abi::add_immediate(&dst, "x1", 8),
        abi::move_immediate(&index, "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers(&index, &length),
        abi::branch_eq(&copy_done),
        abi::load_u8(&byte, &src, 0),
        abi::store_u8(&byte, &dst, 0),
        abi::add_immediate(&src, &src, 1),
        abi::add_immediate(&dst, &dst, 1),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8("x31", &dst, 0),
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
    instructions.extend([abi::label(&done), abi::return_()]);

    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

pub(super) fn lower_fs_temp_directory_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>, Vec<CodeStackSlot>), String> {
    // Vreg-allocated (plan-00-G Phase 2). The temp-dir path is read into an
    // arena buffer (not on-stack); the buffer pointer and length are held across
    // the second `arena_alloc` as vregs (allocator spills / callee-saves them).
    const TEMP_CAPACITY: &str = "4096";

    let temp_alloc_ok = format!("{symbol}_temp_alloc_ok");
    let string_alloc_ok = format!("{symbol}_string_alloc_ok");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let read_error = format!("{symbol}_read_error");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let mut vregs = Vregs::new();
    let buffer = vregs.next();
    let length = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_immediate(abi::return_register(), "Integer", TEMP_CAPACITY),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ];
    let mut relocations = vec![CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    }];
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&temp_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&temp_alloc_ok),
        abi::move_register(&buffer, "x1"),
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
        abi::move_register(&length, abi::return_register()),
        abi::add_immediate(abi::return_register(), abi::return_register(), 9),
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
    let src = vregs.next();
    let dst = vregs.next();
    let index = vregs.next();
    let byte = vregs.next();
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&string_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&string_alloc_ok),
        abi::store_u64(&length, "x1", 0),
        abi::move_register(&src, &buffer),
        abi::add_immediate(&dst, "x1", 8),
        abi::move_immediate(&index, "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers(&index, &length),
        abi::branch_eq(&copy_done),
        abi::load_u8(&byte, &src, 0),
        abi::store_u8(&byte, &dst, 0),
        abi::add_immediate(&src, &src, 1),
        abi::add_immediate(&dst, &dst, 1),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8("x31", &dst, 0),
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
    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

pub(super) fn lower_fs_path_operation_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    operation: FsPathOperation,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>, Vec<CodeStackSlot>), String> {
    // Vreg-allocated (plan-00-G Phase 2). The path pointer is held across the
    // `arena_alloc` (spilled); the C-string is consumed by the syscall before any
    // later call, so it stays in a register.
    let alloc_ok = format!("{symbol}_alloc_ok");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let invalid_path = format!("{symbol}_invalid_path");
    let call_error = format!("{symbol}_call_error");
    let done = format!("{symbol}_done");

    let mut vregs = Vregs::new();
    let path = vregs.next();
    let alloc = vregs.next();
    let len0 = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&path, abi::return_register()),
        abi::load_u64(&len0, &path, 0),
        abi::compare_immediate(&len0, "0"),
        abi::branch_eq(&invalid_path),
        abi::add_immediate(abi::return_register(), &len0, 1),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ];
    let mut relocations = vec![CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    }];
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
    let len = vregs.next();
    let src = vregs.next();
    let dst = vregs.next();
    let index = vregs.next();
    let byte = vregs.next();
    instructions.extend([
        abi::branch(&done),
        abi::label(&alloc_ok),
        abi::move_register(&alloc, "x1"),
        abi::load_u64(&len, &path, 0),
        abi::add_immediate(&src, &path, 8),
        abi::move_register(&dst, &alloc),
        abi::move_immediate(&index, "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers(&index, &len),
        abi::branch_eq(&copy_done),
        abi::load_u8(&byte, &src, 0),
        abi::compare_immediate(&byte, "0"),
        abi::branch_eq(&invalid_path),
        abi::store_u8(&byte, &dst, 0),
        abi::add_immediate(&src, &src, 1),
        abi::add_immediate(&dst, &dst, 1),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8("x31", &dst, 0),
        abi::move_register(abi::return_register(), &alloc),
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
    instructions.extend([abi::label(&done), abi::return_()]);

    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
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
        kind: RelocIntent::Call,
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
        kind: RelocIntent::Call,
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
        kind: RelocIntent::Call,
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
        kind: RelocIntent::Call,
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

pub(super) fn lower_fs_canonical_path_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>, Vec<CodeStackSlot>), String> {
    // Vreg-allocated (plan-00-G Phase 2). The C-string, the PATH_MAX realpath
    // buffer, the measured length and the result are all arena-allocated; the ones
    // held across a later `arena_alloc`/`realpath` become spilled/callee-saved vregs.
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

    let mut vregs = Vregs::new();
    let path = vregs.next();
    let c_path = vregs.next();
    let buffer = vregs.next();
    let length = vregs.next();
    let result = vregs.next();
    let len0 = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&path, abi::return_register()),
        abi::load_u64(&len0, &path, 0),
        abi::compare_immediate(&len0, "0"),
        abi::branch_eq(&invalid),
        abi::add_immediate(abi::return_register(), &len0, 1),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ];
    let mut relocations = vec![CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    }];
    let len = vregs.next();
    let src = vregs.next();
    let dst = vregs.next();
    let index = vregs.next();
    let byte = vregs.next();
    let cursor = vregs.next();
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&path_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&path_alloc_ok),
        abi::move_register(&c_path, "x1"),
        abi::load_u64(&len, &path, 0),
        abi::add_immediate(&src, &path, 8),
        abi::move_register(&dst, &c_path),
        abi::move_immediate(&index, "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers(&index, &len),
        abi::branch_eq(&copy_done),
        abi::load_u8(&byte, &src, 0),
        abi::compare_immediate(&byte, "0"),
        abi::branch_eq(&invalid),
        abi::store_u8(&byte, &dst, 0),
        abi::add_immediate(&src, &src, 1),
        abi::add_immediate(&dst, &dst, 1),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8("x31", &dst, 0),
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
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&buffer_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&buffer_alloc_ok),
        abi::move_register(&buffer, "x1"),
        abi::move_register(abi::return_register(), &c_path),
        abi::move_register("x1", &buffer),
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
        abi::move_immediate(&length, "Integer", "0"),
        abi::label(&length_loop),
        abi::add_registers(&cursor, &buffer, &length),
        abi::load_u8(&byte, &cursor, 0),
        abi::compare_immediate(&byte, "0"),
        abi::branch_eq(&length_done),
        abi::add_immediate(&length, &length, 1),
        abi::branch(&length_loop),
        abi::label(&length_done),
        abi::add_immediate(abi::return_register(), &length, 9),
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
    let remaining = vregs.next();
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&result_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&result_alloc_ok),
        abi::move_register(&result, "x1"),
        abi::store_u64(&length, &result, 0),
        abi::move_register(&src, &buffer),
        abi::add_immediate(&dst, &result, 8),
        abi::move_register(&remaining, &length),
        abi::label(&result_copy_loop),
        abi::compare_immediate(&remaining, "0"),
        abi::branch_eq(&result_copy_done),
        abi::load_u8(&byte, &src, 0),
        abi::store_u8(&byte, &dst, 0),
        abi::add_immediate(&src, &src, 1),
        abi::add_immediate(&dst, &dst, 1),
        abi::subtract_immediate(&remaining, &remaining, 1),
        abi::branch(&result_copy_loop),
        abi::label(&result_copy_done),
        abi::store_u8("x31", &dst, 0),
        abi::move_register(RESULT_VALUE_REGISTER, &result),
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
    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

pub(super) fn lower_fs_is_within_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>, Vec<CodeStackSlot>), String> {
    // Vreg-allocated (plan-00-G Phase 2). Both input paths, their C-strings, and
    // their two PATH_MAX realpath buffers are arena-allocated; each is held across
    // a later `arena_alloc`/`realpath`, so the allocator spills them across the
    // chain of calls. The final prefix comparison makes no call.
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

    let mut vregs = Vregs::new();
    let base = vregs.next();
    let child = vregs.next();
    let c_base = vregs.next();
    let c_child = vregs.next();
    let base_buffer = vregs.next();
    let child_buffer = vregs.next();
    let len = vregs.next();
    let src = vregs.next();
    let dst = vregs.next();
    let index = vregs.next();
    let byte = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&base, abi::return_register()),
        abi::move_register(&child, "x1"),
        abi::load_u64(&len, &base, 0),
        abi::compare_immediate(&len, "0"),
        abi::branch_eq(&invalid),
        abi::add_immediate(abi::return_register(), &len, 1),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ];
    let mut relocations = vec![CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    }];
    let alloc_reloc = |relocations: &mut Vec<CodeRelocation>| {
        relocations.push(CodeRelocation {
            from: symbol.to_string(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
    };
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&base_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&base_alloc_ok),
        abi::move_register(&c_base, "x1"),
        abi::load_u64(&len, &base, 0),
        abi::add_immediate(&src, &base, 8),
        abi::move_register(&dst, &c_base),
        abi::move_immediate(&index, "Integer", "0"),
        abi::label(&base_copy_loop),
        abi::compare_registers(&index, &len),
        abi::branch_eq(&base_copy_done),
        abi::load_u8(&byte, &src, 0),
        abi::compare_immediate(&byte, "0"),
        abi::branch_eq(&invalid),
        abi::store_u8(&byte, &dst, 0),
        abi::add_immediate(&src, &src, 1),
        abi::add_immediate(&dst, &dst, 1),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&base_copy_loop),
        abi::label(&base_copy_done),
        abi::store_u8("x31", &dst, 0),
        abi::load_u64(&len, &child, 0),
        abi::compare_immediate(&len, "0"),
        abi::branch_eq(&invalid),
        abi::add_immediate(abi::return_register(), &len, 1),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    alloc_reloc(&mut relocations);
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&child_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&child_alloc_ok),
        abi::move_register(&c_child, "x1"),
        abi::load_u64(&len, &child, 0),
        abi::add_immediate(&src, &child, 8),
        abi::move_register(&dst, &c_child),
        abi::move_immediate(&index, "Integer", "0"),
        abi::label(&child_copy_loop),
        abi::compare_registers(&index, &len),
        abi::branch_eq(&child_copy_done),
        abi::load_u8(&byte, &src, 0),
        abi::compare_immediate(&byte, "0"),
        abi::branch_eq(&invalid),
        abi::store_u8(&byte, &dst, 0),
        abi::add_immediate(&src, &src, 1),
        abi::add_immediate(&dst, &dst, 1),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&child_copy_loop),
        abi::label(&child_copy_done),
        abi::store_u8("x31", &dst, 0),
        abi::move_immediate(
            abi::return_register(),
            "Integer",
            &PATH_MAX_PLUS_NUL.to_string(),
        ),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    alloc_reloc(&mut relocations);
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&base_buffer_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&base_buffer_alloc_ok),
        abi::move_register(&base_buffer, "x1"),
        abi::move_immediate(
            abi::return_register(),
            "Integer",
            &PATH_MAX_PLUS_NUL.to_string(),
        ),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    alloc_reloc(&mut relocations);
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&child_buffer_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&child_buffer_alloc_ok),
        abi::move_register(&child_buffer, "x1"),
        abi::move_register(abi::return_register(), &c_base),
        abi::move_register("x1", &base_buffer),
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
        abi::move_register(abi::return_register(), &c_child),
        abi::move_register("x1", &child_buffer),
    ]);
    platform.emit_realpath(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    let bb = vregs.next();
    let cb = vregs.next();
    let bchar = vregs.next();
    let cchar = vregs.next();
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&child_realpath_ok),
        abi::branch(&realpath_error),
        abi::label(&child_realpath_ok),
        abi::move_register(&bb, &base_buffer),
        abi::move_register(&cb, &child_buffer),
        abi::load_u8(&bchar, &bb, 0),
        abi::compare_immediate(&bchar, "47"),
        abi::branch_ne(&compare_loop),
        abi::load_u8(&bchar, &bb, 1),
        abi::compare_immediate(&bchar, "0"),
        abi::branch_eq(&root_true),
        abi::label(&compare_loop),
        abi::load_u8(&bchar, &bb, 0),
        abi::load_u8(&cchar, &cb, 0),
        abi::compare_immediate(&bchar, "0"),
        abi::branch_eq(&base_ended),
        abi::compare_registers(&bchar, &cchar),
        abi::branch_ne(&false_label),
        abi::add_immediate(&bb, &bb, 1),
        abi::add_immediate(&cb, &cb, 1),
        abi::branch(&compare_loop),
        abi::label(&base_ended),
        abi::compare_immediate(&cchar, "0"),
        abi::branch_eq(&true_label),
        abi::compare_immediate(&cchar, "47"),
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
    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
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
    // Vreg-allocated (plan-00-G Phase 2). `parts` (the input List) is held across
    // the `arena_alloc` (spilled); the second pass builds into the allocated string
    // with no further call, so its working registers stay in registers.
    const SEP: &str = "47";
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
    let mut vregs = Vregs::new();
    let parts = vregs.next();
    let result = vregs.next();
    let count = vregs.next();
    let total = vregs.next();
    let index = vregs.next();
    let entry = vregs.next();
    let part_len = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&parts, abi::return_register()),
        // Pass 1: upper-bound length = sum(component lengths) + count separators.
        abi::load_u64(&count, &parts, COLLECTION_OFFSET_COUNT),
        abi::move_immediate(&total, "Integer", "0"),
        abi::move_immediate(&index, "Integer", "0"),
        abi::add_immediate(&entry, &parts, COLLECTION_HEADER_SIZE),
        abi::label(&length_loop),
        abi::compare_registers(&index, &count),
        abi::branch_ge(&length_done),
        abi::load_u64(&part_len, &entry, COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        abi::add_registers(&total, &total, &part_len),
        abi::add_immediate(&entry, &entry, COLLECTION_ENTRY_SIZE),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&length_loop),
        abi::label(&length_done),
        abi::add_registers(abi::return_register(), &total, &count),
        abi::add_immediate(abi::return_register(), abi::return_register(), 9),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ];
    let mut relocations = vec![CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    }];
    let data_base = vregs.next();
    let capacity = vregs.next();
    let lookup = vregs.next();
    let out_base = vregs.next();
    let cursor = vregs.next();
    let scratch = vregs.next();
    let value_off = vregs.next();
    let value_len = vregs.next();
    let byte = vregs.next();
    let prev = vregs.next();
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::move_register(&result, "x1"),
        // Pass 2: build the joined path.
        abi::load_u64(&count, &parts, COLLECTION_OFFSET_COUNT),
        // data base = collection + header + capacity * entry_size (plan-01 §4.2:
        // a grown list has capacity > count, so the data region sits past the
        // full lookup capacity, not just the live entries).
        abi::load_u64(&capacity, &parts, COLLECTION_OFFSET_CAPACITY),
        abi::add_immediate(&data_base, &parts, COLLECTION_HEADER_SIZE),
        abi::move_immediate(&scratch, "Integer", &entry_size),
        abi::multiply_registers(&scratch, &capacity, &scratch),
        abi::add_registers(&data_base, &data_base, &scratch),
        abi::add_immediate(&lookup, &parts, COLLECTION_HEADER_SIZE),
        abi::add_immediate(&out_base, &result, 8),
        abi::move_register(&cursor, &out_base),
        abi::move_immediate(&index, "Integer", "0"),
        abi::label(&build_loop),
        abi::compare_registers(&index, &count),
        abi::branch_ge(&build_done),
        abi::load_u64(&value_len, &lookup, COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        abi::compare_immediate(&value_len, "0"),
        abi::branch_eq(&skip_part),
        abi::load_u64(&value_off, &lookup, COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::add_registers(&value_off, &data_base, &value_off),
        abi::load_u8(&byte, &value_off, 0),
        abi::compare_immediate(&byte, SEP),
        abi::branch_eq(&absolute),
        abi::compare_registers(&cursor, &out_base),
        abi::branch_eq(&no_separator),
        abi::subtract_immediate(&prev, &cursor, 1),
        abi::load_u8(&scratch, &prev, 0),
        abi::compare_immediate(&scratch, SEP),
        abi::branch_eq(&no_separator),
        abi::move_immediate(&scratch, "Byte", SEP),
        abi::store_u8(&scratch, &cursor, 0),
        abi::add_immediate(&cursor, &cursor, 1),
        abi::branch(&copy_part),
        abi::label(&absolute),
        abi::move_register(&cursor, &out_base),
        abi::label(&no_separator),
        abi::label(&copy_part),
        abi::label(&copy_loop),
        abi::compare_immediate(&value_len, "0"),
        abi::branch_eq(&copy_done),
        abi::load_u8(&byte, &value_off, 0),
        abi::store_u8(&byte, &cursor, 0),
        abi::add_immediate(&value_off, &value_off, 1),
        abi::add_immediate(&cursor, &cursor, 1),
        abi::subtract_immediate(&value_len, &value_len, 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::label(&skip_part),
        abi::add_immediate(&lookup, &lookup, COLLECTION_ENTRY_SIZE),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&build_loop),
        abi::label(&build_done),
        abi::subtract_registers(&scratch, &cursor, &out_base),
        abi::store_u64(&scratch, &result, 0),
        abi::move_immediate(&byte, "Integer", "0"),
        abi::store_u8(&byte, &cursor, 0),
        abi::move_register(RESULT_VALUE_REGISTER, &result),
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
    instructions.extend([abi::label(&done), abi::return_()]);
    finalize_vreg_helper("runtime.fsPathJoin", symbol, "String", instructions, relocations)
}
