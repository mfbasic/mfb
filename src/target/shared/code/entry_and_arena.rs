use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn lower_program_entry(
    entry_symbol: &str,
    language_entry_symbol: &str,
    language_entry_returns: &str,
    language_entry_accepts_args: bool,
    global_initializer_symbol: Option<&str>,
    link_init_symbol: Option<&str>,
    entry_stack_size: usize,
    global_slot_count: usize,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    emit_cleanup_failure_audit: bool,
    seed_rng: bool,
    register_signal_handlers: bool,
) -> Result<CodeFunction, String> {
    let mut instructions = vec![
        abi::label("entry"),
        abi::subtract_stack(entry_stack_size),
        abi::add_immediate(ARENA_STATE_REGISTER, abi::stack_pointer(), 0),
        abi::store_u64("x31", ARENA_STATE_REGISTER, 0),
        abi::store_u64("x31", ARENA_STATE_REGISTER, 8),
        abi::store_u64("x31", ARENA_STATE_REGISTER, 16),
        abi::store_u64("x31", ARENA_STATE_REGISTER, 24),
        // The main arena-state lives on the entry stack (not zero-filled), so the
        // free-list head must be explicitly cleared before the first allocation.
        abi::store_u64("x31", ARENA_STATE_REGISTER, ARENA_FREE_LIST_HEAD_OFFSET),
    ];
    if emit_cleanup_failure_audit {
        instructions.extend([
            abi::store_u64(
                "x31",
                ARENA_STATE_REGISTER,
                ARENA_CLEANUP_FAILURE_COUNT_OFFSET,
            ),
            abi::store_u64(
                "x31",
                ARENA_STATE_REGISTER,
                ARENA_CLEANUP_FAILURE_CODE_OFFSET,
            ),
            abi::store_u64(
                "x31",
                ARENA_STATE_REGISTER,
                ARENA_CLEANUP_FAILURE_MESSAGE_OFFSET,
            ),
        ]);
    }
    for index in 0..global_slot_count {
        instructions.push(abi::store_u64(
            "x31",
            ARENA_STATE_REGISTER,
            ENTRY_GLOBALS_OFFSET + index * 8,
        ));
    }
    let mut relocations = Vec::new();
    let error_label = "entry_error";
    let exit_label = "entry_exit";
    // Publish this thread's arena-state address to the writable global so the
    // signal handler and `_mfb_shutdown` can find the arena without `x19`. `x9`
    // is a scratch temporary here; `x0`/`x1` (argc/argv) are left untouched.
    push_symbol_address(
        entry_symbol,
        MAIN_ARENA_GLOBAL_SYMBOL,
        "x9",
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::store_u64(ARENA_STATE_REGISTER, "x9", 0));
    // Install SIGINT/SIGTERM handlers (console programs). `signal()` clobbers
    // `x0`/`x1`, so argc/argv are parked below the frame across the calls; `x19`
    // pins the entry frame, so temporarily lowering `sp` is safe.
    if register_signal_handlers {
        instructions.extend([
            abi::subtract_stack(16),
            abi::store_u64("x0", abi::stack_pointer(), 0),
            abi::store_u64("x1", abi::stack_pointer(), 8),
        ]);
        for signo in ["2", "15"] {
            instructions.push(abi::move_immediate("x0", "Integer", signo));
            push_symbol_address(
                entry_symbol,
                SIGNAL_HANDLER_SYMBOL,
                "x1",
                &mut instructions,
                &mut relocations,
            );
            platform.emit_libc_call(
                "signal",
                entry_symbol,
                platform_imports,
                &mut instructions,
                &mut relocations,
            )?;
        }
        instructions.extend([
            abi::load_u64("x0", abi::stack_pointer(), 0),
            abi::load_u64("x1", abi::stack_pointer(), 8),
            abi::add_stack(16),
        ]);
    }
    // Seed this thread's PCG64 generator from the OS entropy pool before any
    // user code (including global initializers, which may call `math::rand`).
    // The seed scratch lives in the as-yet-unused args slot; pre-fill it with
    // the arena address so a `getentropy` failure still yields a varying seed.
    if seed_rng {
        instructions.extend([
            abi::store_u64(
                ARENA_STATE_REGISTER,
                abi::stack_pointer(),
                ENTRY_ARGC_OFFSET,
            ),
            abi::add_immediate(
                abi::return_register(),
                abi::stack_pointer(),
                ENTRY_ARGC_OFFSET,
            ),
            abi::move_immediate("x1", "Integer", "8"),
        ]);
        platform.emit_random_bytes(
            entry_symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::load_u64("x1", abi::stack_pointer(), ENTRY_ARGC_OFFSET),
            abi::move_register(abi::return_register(), ARENA_STATE_REGISTER),
            abi::branch_link(RNG_SEED_SYMBOL),
        ]);
        relocations.push(internal_branch(entry_symbol, RNG_SEED_SYMBOL));
    }
    // Capture the arena start time (offset 40) and seed the dedicated memory-fill
    // RNG (offsets 16/24). Always on — entropy fill is a requirement (plan-01 §6),
    // so this runs for every program before the first allocation. The seed is OS
    // entropy XORed with the arena address and start time, so a `getentropy`
    // failure or two arenas seeding in the same instant still yield distinct
    // poison streams. This is a separate stream from `math::rand` (offsets 88/96),
    // so it never perturbs the reproducible language RNG.
    //
    // `argc`/`argv` (x0/x1) are still live here for arg-accepting entries (saved
    // to the stack further below), and this block clobbers x0–x16, so park them
    // in callee-saved x27/x28 — preserved by the libc calls and the fill helpers
    // — and restore them afterward. A local 16-byte stack buffer holds first the
    // `timespec` and then the entropy bytes, so no entry-stack slot is touched.
    instructions.extend([
        abi::move_register("x27", "x0"),
        abi::move_register("x28", "x1"),
        abi::subtract_stack(16),
        abi::move_immediate("x0", "Integer", "0"), // CLOCK_REALTIME
        abi::add_immediate("x1", abi::stack_pointer(), 0),
    ]);
    platform.emit_libc_call(
        "clock_gettime",
        entry_symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64("x9", abi::stack_pointer(), 0),  // tv_sec
        abi::load_u64("x10", abi::stack_pointer(), 8), // tv_nsec
        abi::move_immediate("x11", "Integer", "1000000000"),
        abi::multiply_registers("x9", "x9", "x11"),
        abi::add_registers("x9", "x9", "x10"), // ns = sec*1e9 + nsec
        abi::store_u64("x9", ARENA_STATE_REGISTER, ARENA_START_TIME_OFFSET),
        // Pre-fill the seed scratch with the arena address (getentropy fallback).
        abi::store_u64(ARENA_STATE_REGISTER, abi::stack_pointer(), 0),
        abi::add_immediate(abi::return_register(), abi::stack_pointer(), 0),
        abi::move_immediate("x1", "Integer", "8"),
    ]);
    platform.emit_random_bytes(
        entry_symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64("x1", abi::stack_pointer(), 0), // entropy (or arena addr)
        abi::add_stack(16),
        abi::load_u64("x9", ARENA_STATE_REGISTER, ARENA_START_TIME_OFFSET),
        abi::exclusive_or_registers("x1", "x1", "x9"), // mix start time
        abi::exclusive_or_registers("x1", "x1", ARENA_STATE_REGISTER), // mix arena address
        abi::move_register(abi::return_register(), ARENA_STATE_REGISTER),
        abi::branch_link(ARENA_FILL_SEED_SYMBOL),
        // Restore argc/argv for the arg-materialization path below.
        abi::move_register("x0", "x27"),
        abi::move_register("x1", "x28"),
    ]);
    relocations.push(internal_branch(entry_symbol, ARENA_FILL_SEED_SYMBOL));
    // Resolve native `LINK` bindings (dlopen/dlsym) before anything runs; a load
    // failure aborts before `main` through the standard error path
    // (plan-linker.md §12.1).
    if let Some(symbol) = link_init_symbol {
        instructions.push(abi::branch_link(symbol));
        relocations.push(CodeRelocation {
            from: entry_symbol.to_string(),
            to: symbol.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
        instructions.extend([
            abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG),
            abi::branch_ne(error_label),
        ]);
    }
    if let Some(symbol) = global_initializer_symbol {
        instructions.push(abi::branch_link(symbol));
        relocations.push(CodeRelocation {
            from: entry_symbol.to_string(),
            to: symbol.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
        instructions.extend([
            abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_PROGRAM_EXIT_TAG),
            abi::branch_ne("global_initializer_not_program_exit"),
            abi::move_register(abi::return_register(), RESULT_VALUE_REGISTER),
            abi::branch(exit_label),
            abi::label("global_initializer_not_program_exit"),
        ]);
        instructions.extend([
            abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG),
            abi::branch_ne(error_label),
        ]);
    }
    if language_entry_accepts_args {
        instructions.extend([
            abi::store_u64("x0", abi::stack_pointer(), ENTRY_ARGC_OFFSET),
            abi::store_u64("x1", abi::stack_pointer(), ENTRY_ARGV_OFFSET),
        ]);
        emit_entry_args_list_materialization(error_label, &mut instructions, &mut relocations);
        instructions.push(abi::load_u64(
            "x0",
            abi::stack_pointer(),
            ENTRY_ARGS_LIST_OFFSET,
        ));
    }
    instructions.push(abi::branch_link(language_entry_symbol));
    relocations.push(CodeRelocation {
        from: entry_symbol.to_string(),
        to: language_entry_symbol.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_PROGRAM_EXIT_TAG),
        abi::branch_ne("entry_not_program_exit"),
        abi::move_register(abi::return_register(), RESULT_VALUE_REGISTER),
        abi::branch(exit_label),
        abi::label("entry_not_program_exit"),
    ]);
    instructions.extend([
        abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG),
        abi::branch_ne(error_label),
    ]);
    if language_entry_returns == "Nothing" {
        instructions.push(abi::move_immediate(abi::return_register(), "Integer", "0"));
    } else {
        instructions.push(abi::move_register(
            abi::return_register(),
            RESULT_VALUE_REGISTER,
        ));
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "255"),
            abi::branch_hi("entry_exit_range_error"),
        ]);
    }
    instructions.push(abi::branch(exit_label));
    if language_entry_returns == "Integer" {
        instructions.extend([
            abi::label("entry_exit_range_error"),
            abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OVERFLOW_CODE),
            abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
        ]);
        push_error_message_address(
            entry_symbol,
            ERR_OVERFLOW_SYMBOL,
            &mut instructions,
            &mut relocations,
        );
        instructions.push(abi::branch(error_label));
    }
    instructions.extend([
        abi::label(error_label),
        abi::store_u64(RESULT_VALUE_REGISTER, ARENA_STATE_REGISTER, 32),
        abi::move_register("x20", RESULT_ERROR_MESSAGE_REGISTER),
    ]);
    emit_write_string_object(
        ENTRY_ERROR_PREFIX_SYMBOL,
        entry_symbol,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    emit_write_integer_to_stderr(
        entry_symbol,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    emit_write_string_object(
        ENTRY_ERROR_SEPARATOR_SYMBOL,
        entry_symbol,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64(abi::string_length_register(), "x20", 0),
        abi::add_immediate(abi::string_data_register(), "x20", 8),
        abi::move_immediate(abi::return_register(), "Integer", "2"),
    ]);
    platform.emit_write(
        entry_symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_write_string_object(
        ENTRY_ERROR_NEWLINE_SYMBOL,
        entry_symbol,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    if emit_cleanup_failure_audit {
        emit_cleanup_failure_audit_report(
            entry_symbol,
            platform_imports,
            platform,
            &mut instructions,
            &mut relocations,
        )?;
    }
    instructions.push(abi::move_immediate(
        abi::return_register(),
        "Integer",
        "255",
    ));
    instructions.push(abi::label(exit_label));
    // Run the shared teardown (terminal restore + arena free), then exit. The
    // exit code is parked in the arena-state scratch slot across the call: that
    // slot lives in this stack-resident entry frame (not in the freed mmap
    // blocks), and `_mfb_shutdown` preserves `x19`, so it is valid on return.
    // `_mfb_shutdown` is internally gated and idempotent, so the SIGINT/SIGTERM
    // handler racing this path cannot double-free.
    instructions.push(abi::store_u64(
        abi::return_register(),
        ARENA_STATE_REGISTER,
        32,
    ));
    instructions.push(abi::branch_link(SHUTDOWN_SYMBOL));
    relocations.push(internal_branch(entry_symbol, SHUTDOWN_SYMBOL));
    instructions.push(abi::load_u64(
        abi::return_register(),
        ARENA_STATE_REGISTER,
        32,
    ));
    platform.emit_program_exit(entry_symbol, &mut instructions, &mut relocations)?;
    Ok(CodeFunction {
        name: if entry_symbol == "_main" {
            "program.entry".to_string()
        } else {
            "program.entry.macapp".to_string()
        },
        symbol: entry_symbol.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions,
        relocations,
    })
}

fn emit_entry_args_list_materialization(
    error_label: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    instructions.extend([
        abi::load_u64("x20", abi::stack_pointer(), ENTRY_ARGC_OFFSET),
        abi::load_u64("x21", abi::stack_pointer(), ENTRY_ARGV_OFFSET),
        abi::move_immediate("x22", "Integer", "0"),
        abi::move_immediate("x23", "Integer", "0"),
        abi::label("entry_args_count_loop"),
        abi::compare_registers("x23", "x20"),
        abi::branch_eq("entry_args_count_done"),
        abi::load_u64("x24", "x21", 0),
        abi::move_register("x25", "x24"),
        abi::move_immediate("x26", "Integer", "0"),
        abi::label("entry_args_count_len_loop"),
        abi::load_u8("x27", "x25", 0),
        abi::compare_immediate("x27", "0"),
        abi::branch_eq("entry_args_count_len_done"),
        abi::add_immediate("x26", "x26", 1),
        abi::add_immediate("x25", "x25", 1),
        abi::branch("entry_args_count_len_loop"),
        abi::label("entry_args_count_len_done"),
        abi::add_registers("x22", "x22", "x26"),
        abi::add_immediate("x21", "x21", 8),
        abi::add_immediate("x23", "x23", 1),
        abi::branch("entry_args_count_loop"),
        abi::label("entry_args_count_done"),
        abi::move_immediate("x24", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("x25", "x20", "x24"),
        abi::add_registers("x25", "x25", "x22"),
        abi::store_u64("x22", abi::stack_pointer(), ENTRY_ARGS_DATA_LENGTH_OFFSET),
        abi::store_u64("x20", abi::stack_pointer(), ENTRY_ARGS_COUNT_SAVED_OFFSET),
        abi::add_immediate(abi::return_register(), "x25", COLLECTION_HEADER_SIZE),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: "_main".to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq("entry_args_alloc_ok"),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address("_main", ERR_ALLOCATION_SYMBOL, instructions, relocations);
    instructions.extend([
        abi::branch(error_label),
        abi::label("entry_args_alloc_ok"),
        abi::store_u64("x1", abi::stack_pointer(), ENTRY_ARGS_LIST_OFFSET),
        abi::load_u64("x22", abi::stack_pointer(), ENTRY_ARGS_DATA_LENGTH_OFFSET),
        abi::load_u64("x20", abi::stack_pointer(), ENTRY_ARGS_COUNT_SAVED_OFFSET),
        abi::move_immediate("x8", "Byte", &COLLECTION_KIND_LIST.to_string()),
        abi::store_u8("x8", "x1", COLLECTION_OFFSET_KIND),
        abi::move_immediate("x8", "Byte", &COLLECTION_TYPE_NONE.to_string()),
        abi::store_u8("x8", "x1", COLLECTION_OFFSET_KEY_TYPE),
        abi::move_immediate("x8", "Byte", &COLLECTION_TYPE_STRING.to_string()),
        abi::store_u8("x8", "x1", COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate("x8", "Byte", "1"),
        abi::store_u8("x8", "x1", COLLECTION_OFFSET_FLAGS_VERSION),
        abi::store_u64("x20", "x1", COLLECTION_OFFSET_COUNT),
        abi::store_u64("x20", "x1", COLLECTION_OFFSET_CAPACITY),
        abi::store_u64("x22", "x1", COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64("x22", "x1", COLLECTION_OFFSET_DATA_CAPACITY),
        abi::add_immediate("x23", "x1", COLLECTION_HEADER_SIZE),
        abi::move_immediate("x24", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("x25", "x20", "x24"),
        abi::add_registers("x24", "x23", "x25"),
        abi::move_immediate("x25", "Integer", "0"),
        abi::load_u64("x21", abi::stack_pointer(), ENTRY_ARGV_OFFSET),
        abi::move_immediate("x26", "Integer", "0"),
        abi::label("entry_args_fill_loop"),
        abi::compare_registers("x26", "x20"),
        abi::branch_eq("entry_args_fill_done"),
        abi::load_u64("x27", "x21", 0),
        abi::move_register("x28", "x27"),
        abi::move_immediate("x9", "Integer", "0"),
        abi::label("entry_args_fill_len_loop"),
        abi::load_u8("x10", "x28", 0),
        abi::compare_immediate("x10", "0"),
        abi::branch_eq("entry_args_fill_len_done"),
        abi::add_immediate("x9", "x9", 1),
        abi::add_immediate("x28", "x28", 1),
        abi::branch("entry_args_fill_len_loop"),
        abi::label("entry_args_fill_len_done"),
        abi::move_immediate("x11", "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
        abi::store_u8("x11", "x23", COLLECTION_ENTRY_OFFSET_FLAGS),
        abi::store_u64("x31", "x23", COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
        abi::store_u64("x31", "x23", COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
        abi::store_u64("x25", "x23", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::store_u64("x9", "x23", COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        abi::move_immediate("x11", "Integer", "0"),
        abi::label("entry_args_copy_loop"),
        abi::compare_registers("x11", "x9"),
        abi::branch_eq("entry_args_copy_done"),
        abi::load_u8("x12", "x27", 0),
        abi::store_u8("x12", "x24", 0),
        abi::add_immediate("x27", "x27", 1),
        abi::add_immediate("x24", "x24", 1),
        abi::add_immediate("x11", "x11", 1),
        abi::branch("entry_args_copy_loop"),
        abi::label("entry_args_copy_done"),
        abi::add_registers("x25", "x25", "x9"),
        abi::add_immediate("x23", "x23", COLLECTION_ENTRY_SIZE),
        abi::add_immediate("x21", "x21", 8),
        abi::add_immediate("x26", "x26", 1),
        abi::branch("entry_args_fill_loop"),
        abi::label("entry_args_fill_done"),
    ]);
}

fn emit_cleanup_failure_audit_report(
    from: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let done = "entry_cleanup_failure_audit_done";
    instructions.extend([
        abi::load_u64(
            "x9",
            ARENA_STATE_REGISTER,
            ARENA_CLEANUP_FAILURE_COUNT_OFFSET,
        ),
        abi::compare_immediate("x9", "0"),
        abi::branch_eq(done),
    ]);
    emit_write_string_object(
        CLEANUP_FAILURE_PREFIX_SYMBOL,
        from,
        platform_imports,
        platform,
        instructions,
        relocations,
    )?;
    instructions.push(abi::load_u64(
        "x9",
        ARENA_STATE_REGISTER,
        ARENA_CLEANUP_FAILURE_CODE_OFFSET,
    ));
    instructions.push(abi::store_u64("x9", ARENA_STATE_REGISTER, 32));
    emit_write_integer_to_stderr_with_labels(
        from,
        platform_imports,
        platform,
        instructions,
        relocations,
        "entry_cleanup_failure_code",
    )?;
    emit_write_string_object(
        CLEANUP_FAILURE_SEPARATOR_SYMBOL,
        from,
        platform_imports,
        platform,
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::load_u64(
            "x20",
            ARENA_STATE_REGISTER,
            ARENA_CLEANUP_FAILURE_MESSAGE_OFFSET,
        ),
        abi::load_u64(abi::string_length_register(), "x20", 0),
        abi::add_immediate(abi::string_data_register(), "x20", 8),
        abi::move_immediate(abi::return_register(), "Integer", "2"),
    ]);
    platform.emit_write(from, platform_imports, instructions, relocations)?;
    emit_write_string_object(
        ENTRY_ERROR_NEWLINE_SYMBOL,
        from,
        platform_imports,
        platform,
        instructions,
        relocations,
    )?;
    instructions.push(abi::label(done));
    Ok(())
}

pub(super) fn lower_arena_alloc(platform: &dyn CodegenPlatform) -> Result<CodeFunction, String> {
    // Grow-path frame: the fast (first-fit) path makes no call, but the rare
    // block-grow path calls `arena_fill_random` to poison the new block, so the
    // function carries a frame and saves the link register. The fast path never
    // touches x11–x13/x17, and the grow path saves/restores x11–x13 around the
    // fill call, so the historical clobber contract (x9, x10, x14, x15, x20–x28)
    // is preserved for callers.
    const FRAME_SIZE: usize = 64;
    const LR_SLOT: usize = 0;
    const UBASE_SLOT: usize = 8;
    const USIZE_SLOT: usize = 16;
    const X11_SLOT: usize = 24;
    const X12_SLOT: usize = 32;
    const X13_SLOT: usize = 40;
    let not_15 = (!(ARENA_MIN_CHUNK - 1)).to_string();
    let mut relocations = Vec::new();
    let mut instructions = Vec::new();
    // --- Validate alignment and normalize the request --------------------------
    // x20 = normalized size (rounded up to the 16-byte granule), x21 = effective
    // alignment (raised to ≥16 so every chunk start stays 16-aligned).
    instructions.extend([
        abi::label("entry"),
        abi::subtract_stack(FRAME_SIZE),
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_SLOT),
        abi::compare_immediate("x1", "0"),
        abi::branch_eq("arena_alloc_invalid"),
        abi::subtract_immediate("x9", "x1", 1),
        abi::and_registers("x10", "x1", "x9"),
        abi::compare_immediate("x10", "0"),
        abi::branch_ne("arena_alloc_invalid"),
        // eff align = max(align, 16)
        abi::move_register("x21", "x1"),
        abi::compare_immediate("x21", &ARENA_MIN_CHUNK.to_string()),
        abi::branch_lo("arena_alloc_align_min"),
        abi::branch("arena_alloc_align_ready"),
        abi::label("arena_alloc_align_min"),
        abi::move_immediate("x21", "Integer", &ARENA_MIN_CHUNK.to_string()),
        abi::label("arena_alloc_align_ready"),
        // normalized size = round_up(max(size, 1), 16)
        abi::move_register("x20", "x0"),
        abi::compare_immediate("x20", "0"),
        abi::branch_ne("arena_alloc_size_nonzero"),
        abi::move_immediate("x20", "Integer", "1"),
        abi::label("arena_alloc_size_nonzero"),
        abi::add_immediate("x20", "x20", (ARENA_MIN_CHUNK - 1) as usize),
        abi::move_immediate("x9", "Integer", &not_15),
        abi::and_registers("x20", "x20", "x9"),
        // --- First-fit walk over the address-ordered free-list -----------------
        abi::label("arena_alloc_walk"),
        abi::load_u64("x22", ARENA_STATE_REGISTER, ARENA_FREE_LIST_HEAD_OFFSET),
        abi::move_immediate("x23", "Integer", "0"),
        abi::label("arena_alloc_walk_loop"),
        abi::compare_immediate("x22", "0"),
        abi::branch_eq("arena_alloc_grow"),
        abi::load_u64("x24", "x22", 8),          // cur_size
        abi::subtract_immediate("x9", "x21", 1), // align mask
        abi::add_registers("x25", "x22", "x9"),
        abi::compare_registers("x25", "x22"),
        abi::branch_lo("arena_alloc_walk_next"), // align overflow → skip
        abi::bitwise_not("x10", "x9"),
        abi::and_registers("x25", "x25", "x10"), // aligned
        abi::add_registers("x26", "x25", "x20"), // end_needed
        abi::compare_registers("x26", "x25"),
        abi::branch_lo("arena_alloc_walk_next"), // size overflow → skip
        abi::add_registers("x27", "x22", "x24"), // cur_end
        abi::compare_registers("x26", "x27"),
        abi::branch_hi("arena_alloc_walk_next"), // doesn't fit → next
        abi::branch("arena_alloc_found"),
        abi::label("arena_alloc_walk_next"),
        abi::move_register("x23", "x22"),
        abi::load_u64("x22", "x22", 0),
        abi::branch("arena_alloc_walk_loop"),
        // --- Found: split the chosen chunk -------------------------------------
        // cur=x22, prev=x23, cur_size=x24, aligned=x25, end_needed=x26,
        // cur_end=x27, next=x9, front_pad=x14, tail_size=x15, link target=x10.
        abi::label("arena_alloc_found"),
        abi::load_u64("x9", "x22", 0),                // next
        abi::subtract_registers("x14", "x25", "x22"), // front_pad
        abi::subtract_registers("x15", "x27", "x26"), // tail_size
        abi::compare_immediate("x14", "0"),
        abi::branch_ne("arena_alloc_have_front"),
        abi::compare_immediate("x15", "0"),
        abi::branch_ne("arena_alloc_front0_tail1"),
        // case: chunk consumed exactly → link target is `next`
        abi::move_register("x10", "x9"),
        abi::branch("arena_alloc_set_prev_link"),
        abi::label("arena_alloc_front0_tail1"),
        // case: tail remainder only → new free node at end_needed
        abi::store_u64("x9", "x26", 0),
        abi::store_u64("x15", "x26", 8),
        abi::move_register("x10", "x26"),
        abi::branch("arena_alloc_set_prev_link"),
        abi::label("arena_alloc_have_front"),
        abi::compare_immediate("x15", "0"),
        abi::branch_ne("arena_alloc_front1_tail1"),
        // case: front padding only → shrink node in place at cur
        abi::store_u64("x9", "x22", 0),
        abi::store_u64("x14", "x22", 8),
        abi::move_register("x10", "x22"),
        abi::branch("arena_alloc_set_prev_link"),
        abi::label("arena_alloc_front1_tail1"),
        // case: both front and tail remainders → two free nodes
        abi::store_u64("x26", "x22", 0), // cur.next → tail node
        abi::store_u64("x14", "x22", 8), // cur.size = front_pad
        abi::store_u64("x9", "x26", 0),  // tail.next = next
        abi::store_u64("x15", "x26", 8), // tail.size = tail_size
        abi::move_register("x10", "x22"),
        abi::label("arena_alloc_set_prev_link"),
        abi::compare_immediate("x23", "0"),
        abi::branch_eq("arena_alloc_set_head"),
        abi::store_u64("x10", "x23", 0),
        abi::branch("arena_alloc_done"),
        abi::label("arena_alloc_set_head"),
        abi::store_u64("x10", ARENA_STATE_REGISTER, ARENA_FREE_LIST_HEAD_OFFSET),
        abi::label("arena_alloc_done"),
        abi::move_immediate(abi::return_register(), "Integer", RESULT_OK_TAG),
        abi::move_register("x1", "x25"),
        abi::branch("arena_alloc_ret"),
        // --- Grow: map a new block and add its usable region as a free chunk ---
        abi::label("arena_alloc_grow"),
        abi::add_registers("x23", "x20", "x21"),
        abi::compare_registers("x23", "x20"),
        abi::branch_lo("arena_alloc_oom"),
        abi::add_immediate("x23", "x23", ARENA_BLOCK_HEADER_SIZE),
        abi::move_immediate("x14", "Integer", &ARENA_DEFAULT_BLOCK_SIZE.to_string()),
        abi::compare_registers("x23", "x14"),
        abi::branch_hi("arena_alloc_normal_block"),
        abi::move_immediate("x23", "Integer", &ARENA_DEFAULT_BLOCK_SIZE.to_string()),
        abi::branch("arena_alloc_map_size_ready"),
        abi::label("arena_alloc_normal_block"),
        abi::move_register("x15", "x23"),
        abi::add_immediate("x23", "x23", 4095),
        abi::compare_registers("x23", "x15"),
        abi::branch_lo("arena_alloc_oom"),
        abi::move_immediate("x24", "Integer", &(!4095_u64).to_string()),
        abi::and_registers("x23", "x23", "x24"),
        abi::label("arena_alloc_map_size_ready"),
    ]);
    platform.emit_arena_map(&mut instructions)?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ge("arena_alloc_mapped"),
        abi::branch("arena_alloc_oom"),
        abi::label("arena_alloc_mapped"),
        // Write the block header (prevBlock, blockSize, usableCapacity, bumpOffset)
        // and chain it. bumpOffset is vestigial under the free-list but kept zero
        // so the documented block layout is unchanged.
        abi::load_u64("x24", ARENA_STATE_REGISTER, 0),
        abi::store_u64("x24", abi::return_register(), 0),
        abi::store_u64("x23", abi::return_register(), 8),
        abi::subtract_immediate("x24", "x23", ARENA_BLOCK_HEADER_SIZE),
        abi::store_u64("x24", abi::return_register(), 16),
        abi::store_u64("x31", abi::return_register(), 24),
        abi::store_u64(abi::return_register(), ARENA_STATE_REGISTER, 0),
        // Poison the new block's usable region before first use (plan-01 §6.3).
        // fill_random clobbers x0/x1/x9–x16 and advances x0, so stash ubase/usize
        // and the caller-survivor registers x11–x13 across the call.
        abi::add_immediate("x9", abi::return_register(), ARENA_BLOCK_HEADER_SIZE), // ubase
        abi::store_u64("x9", abi::stack_pointer(), UBASE_SLOT),
        abi::store_u64("x24", abi::stack_pointer(), USIZE_SLOT),
        abi::store_u64("x11", abi::stack_pointer(), X11_SLOT),
        abi::store_u64("x12", abi::stack_pointer(), X12_SLOT),
        abi::store_u64("x13", abi::stack_pointer(), X13_SLOT),
        abi::move_register("x0", "x9"),
        abi::move_register("x1", "x24"),
        abi::branch_link(ARENA_FILL_RANDOM_SYMBOL),
        abi::load_u64("x9", abi::stack_pointer(), UBASE_SLOT),
        abi::load_u64("x10", abi::stack_pointer(), USIZE_SLOT),
        abi::load_u64("x11", abi::stack_pointer(), X11_SLOT),
        abi::load_u64("x12", abi::stack_pointer(), X12_SLOT),
        abi::load_u64("x13", abi::stack_pointer(), X13_SLOT),
        // Insert [base+32, base+32+usableCapacity) as one free chunk, in address
        // order. A fresh block is never adjacent to an existing chunk (the 32-byte
        // header always separates blocks), so no coalescing is required here.
        abi::load_u64("x14", ARENA_STATE_REGISTER, ARENA_FREE_LIST_HEAD_OFFSET), // cur
        abi::move_immediate("x15", "Integer", "0"),                              // prev
        abi::label("arena_alloc_ins_loop"),
        abi::compare_immediate("x14", "0"),
        abi::branch_eq("arena_alloc_ins_do"),
        abi::compare_registers("x14", "x9"),
        abi::branch_hi("arena_alloc_ins_do"),
        abi::move_register("x15", "x14"),
        abi::load_u64("x14", "x14", 0),
        abi::branch("arena_alloc_ins_loop"),
        abi::label("arena_alloc_ins_do"),
        abi::store_u64("x14", "x9", 0),
        abi::store_u64("x10", "x9", 8),
        abi::compare_immediate("x15", "0"),
        abi::branch_eq("arena_alloc_ins_head"),
        abi::store_u64("x9", "x15", 0),
        abi::branch("arena_alloc_walk"),
        abi::label("arena_alloc_ins_head"),
        abi::store_u64("x9", ARENA_STATE_REGISTER, ARENA_FREE_LIST_HEAD_OFFSET),
        abi::branch("arena_alloc_walk"),
        abi::label("arena_alloc_invalid"),
        abi::move_immediate(abi::return_register(), "Integer", ERR_INVALID_ARGUMENT_CODE),
        abi::move_immediate("x1", "Integer", "0"),
        abi::branch("arena_alloc_ret"),
        abi::label("arena_alloc_oom"),
        abi::move_immediate(abi::return_register(), "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate("x1", "Integer", "0"),
        abi::label("arena_alloc_ret"),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_SLOT),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);
    relocations.push(internal_branch(
        ARENA_ALLOC_SYMBOL,
        ARENA_FILL_RANDOM_SYMBOL,
    ));
    Ok(CodeFunction {
        name: "runtime.arena_alloc".to_string(),
        symbol: ARENA_ALLOC_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Pointer".to_string(),
        frame: CodeFrame {
            stack_size: FRAME_SIZE,
            callee_saved: vec![abi::link_register().to_string()],
        },
        stack_slots: Vec::new(),
        instructions,
        relocations,
    })
}

/// `_mfb_simd_alloc_list(x0 = count, x1 = valueTypeCode) -> x0 = base` —
/// allocate a tight homogeneous numeric `List` (plan-01-simd §4.3). The data
/// region is `count` contiguous 8-byte lanes at `base + 40 + count*40`. Returns
/// `0` if the arena allocation fails (the caller raises the allocation error).
///
/// Calls `_mfb_arena_alloc`, whose clobber set is wide (`x0,x1,x9,x10,x14,x15,
/// x16,x20-x28`); `count` and `valueTypeCode` are spilled across the call and
/// reloaded. After the call there are no further calls, so the header/entry
/// writes use scratch GPRs freely.
pub(super) fn lower_simd_alloc_list() -> CodeFunction {
    const FRAME_SIZE: usize = 32;
    const LR_SLOT: usize = 0;
    const COUNT_SLOT: usize = 8;
    const TYPE_SLOT: usize = 16;
    let mut relocations = Vec::new();
    let instructions = vec![
        abi::label("entry"),
        abi::subtract_stack(FRAME_SIZE),
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_SLOT),
        abi::store_u64("x0", abi::stack_pointer(), COUNT_SLOT),
        abi::store_u64("x1", abi::stack_pointer(), TYPE_SLOT),
        // alloc size = 40 (header) + count*40 (lookup table) + count*8 (data)
        //            = 40 + count*48.
        abi::move_immediate(
            "x9",
            "Integer",
            &(COLLECTION_ENTRY_SIZE + 8).to_string(),
        ),
        abi::multiply_registers("x0", "x0", "x9"),
        abi::add_immediate("x0", "x0", COLLECTION_HEADER_SIZE),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
        // x0 = result tag, x1 = pointer. Return x0 = base, x1 = status (0 = ok,
        // else the arena error tag) so the caller can raise the allocation error.
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq("simd_alloc_ok"),
        abi::move_register("x1", abi::return_register()),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::branch("simd_alloc_ret"),
        abi::label("simd_alloc_ok"),
        // x11 = base, x12 = count, x13 = typeCode.
        abi::move_register("x11", "x1"),
        abi::load_u64("x12", abi::stack_pointer(), COUNT_SLOT),
        abi::load_u64("x13", abi::stack_pointer(), TYPE_SLOT),
        // Header: kind=0 (list), keyType=0, valueType=typeCode, flagsVersion=1.
        abi::move_immediate("x8", "Integer", "0"),
        abi::store_u8("x8", "x11", COLLECTION_OFFSET_KIND),
        abi::store_u8("x8", "x11", COLLECTION_OFFSET_KEY_TYPE),
        abi::store_u8("x13", "x11", COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate("x8", "Integer", "1"),
        abi::store_u8("x8", "x11", COLLECTION_OFFSET_FLAGS_VERSION),
        // count, capacity = count; dataLength, dataCapacity = count*8.
        abi::store_u64("x12", "x11", COLLECTION_OFFSET_COUNT),
        abi::store_u64("x12", "x11", COLLECTION_OFFSET_CAPACITY),
        abi::shift_left_immediate("x9", "x12", 3),
        abi::store_u64("x9", "x11", COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64("x9", "x11", COLLECTION_OFFSET_DATA_CAPACITY),
        // Fill the lookup entries: flags=USED, valueOffset=i*8, valueLength=8.
        // x10 = entry ptr, x9 = index, x14 = running value offset.
        abi::add_immediate("x10", "x11", COLLECTION_HEADER_SIZE),
        abi::move_immediate("x9", "Integer", "0"),
        abi::move_immediate("x14", "Integer", "0"),
        abi::label("simd_alloc_entry_loop"),
        abi::compare_registers("x9", "x12"),
        abi::branch_ge("simd_alloc_entry_done"),
        abi::move_immediate("x8", "Integer", &COLLECTION_ENTRY_FLAG_USED.to_string()),
        abi::store_u8("x8", "x10", COLLECTION_ENTRY_OFFSET_FLAGS),
        abi::store_u64("x14", "x10", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::move_immediate("x8", "Integer", "8"),
        abi::store_u64("x8", "x10", COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        abi::add_immediate("x14", "x14", 8),
        abi::add_immediate("x10", "x10", COLLECTION_ENTRY_SIZE),
        abi::add_immediate("x9", "x9", 1),
        abi::branch("simd_alloc_entry_loop"),
        abi::label("simd_alloc_entry_done"),
        abi::move_register(abi::return_register(), "x11"),
        abi::move_immediate("x1", "Integer", "0"),
        abi::label("simd_alloc_ret"),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_SLOT),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ];
    relocations.push(internal_branch(SIMD_ALLOC_LIST_SYMBOL, ARENA_ALLOC_SYMBOL));
    CodeFunction {
        name: "runtime.simd_alloc_list".to_string(),
        symbol: SIMD_ALLOC_LIST_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Pointer".to_string(),
        frame: CodeFrame {
            stack_size: FRAME_SIZE,
            callee_saved: vec![abi::link_register().to_string()],
        },
        stack_slots: Vec::new(),
        instructions,
        relocations,
    }
}

/// `_mfb_build_error_loc(x0 = filename String*, x1 = line, x2 = char)` — allocate
/// an `ErrorLoc` record `{filename(offset)@0, line@8, char@16}` with the filename
/// String block inlined at offset `ERROR_LOC_OBJECT_SIZE`, and return its pointer
/// in `x0` (`x0 = 0` on OOM). This is the out-of-line form of the block formerly
/// emitted inline at every trap site (`emit_build_error_loc`, plan-16); one shared
/// copy replaces ~48 instructions per site. `filename` is never null (the caller
/// passes an empty-String constant when the source file is unknown); the length
/// word it points at drives the inlined block size and the byte copy. Mirrors
/// `simd_alloc_list`: a framed function that calls `_mfb_arena_alloc` and returns
/// null rather than propagating an error (it runs *during* error handling).
pub(super) fn lower_build_error_loc() -> CodeFunction {
    const FRAME_SIZE: usize = 64;
    const LR_SLOT: usize = 0;
    const FN_SLOT: usize = 8;
    const LINE_SLOT: usize = 16;
    const CHAR_SLOT: usize = 24;
    const LEN_SLOT: usize = 32;
    let mut relocations = Vec::new();
    let instructions = vec![
        abi::label("entry"),
        abi::subtract_stack(FRAME_SIZE),
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_SLOT),
        // Save the caller-saved inputs across the allocation call.
        abi::store_u64("x0", abi::stack_pointer(), FN_SLOT),
        abi::store_u64("x1", abi::stack_pointer(), LINE_SLOT),
        abi::store_u64("x2", abi::stack_pointer(), CHAR_SLOT),
        // len = *filename; size = 24 (fixed slots) + len + 9 (inlined String block).
        abi::load_u64("x9", "x0", 0),
        abi::store_u64("x9", abi::stack_pointer(), LEN_SLOT),
        abi::add_immediate("x9", "x9", ERROR_LOC_OBJECT_SIZE + 9),
        abi::move_register(abi::return_register(), "x9"),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
        // x0 = result tag, x1 = pointer. On OOM (tag != ok) return a null pointer.
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq("build_error_loc_ok"),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::branch("build_error_loc_ret"),
        abi::label("build_error_loc_ok"),
        // Fixed slots: filename block-relative offset @0 = 24, line @8, char @16.
        abi::move_immediate("x9", "Integer", &ERROR_LOC_OBJECT_SIZE.to_string()),
        abi::store_u64("x9", "x1", 0),
        abi::load_u64("x9", abi::stack_pointer(), LINE_SLOT),
        abi::store_u64("x9", "x1", 8),
        abi::load_u64("x9", abi::stack_pointer(), CHAR_SLOT),
        abi::store_u64("x9", "x1", 16),
        // Inline the filename String block (len + 9 bytes) at offset 24. The
        // ErrorLoc pointer (x1) is preserved across the copy (dst walks in x10).
        // x10 = dst, x11 = src, x12 = remaining, x14 = scratch.
        abi::add_immediate("x10", "x1", ERROR_LOC_OBJECT_SIZE),
        abi::load_u64("x11", abi::stack_pointer(), FN_SLOT),
        abi::load_u64("x12", abi::stack_pointer(), LEN_SLOT),
        abi::add_immediate("x12", "x12", 9),
        abi::label("build_error_loc_wloop"),
        abi::compare_immediate("x12", "8"),
        abi::branch_lo("build_error_loc_btail"),
        abi::load_u64("x14", "x11", 0),
        abi::store_u64("x14", "x10", 0),
        abi::add_immediate("x11", "x11", 8),
        abi::add_immediate("x10", "x10", 8),
        abi::subtract_immediate("x12", "x12", 8),
        abi::branch("build_error_loc_wloop"),
        abi::label("build_error_loc_btail"),
        abi::compare_immediate("x12", "0"),
        abi::branch_eq("build_error_loc_copy_done"),
        abi::load_u8("x14", "x11", 0),
        abi::store_u8("x14", "x10", 0),
        abi::add_immediate("x11", "x11", 1),
        abi::add_immediate("x10", "x10", 1),
        abi::subtract_immediate("x12", "x12", 1),
        abi::branch("build_error_loc_btail"),
        abi::label("build_error_loc_copy_done"),
        abi::move_register(abi::return_register(), "x1"),
        abi::label("build_error_loc_ret"),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_SLOT),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ];
    relocations.push(internal_branch(BUILD_ERROR_LOC_SYMBOL, ARENA_ALLOC_SYMBOL));
    CodeFunction {
        name: "runtime.build_error_loc".to_string(),
        symbol: BUILD_ERROR_LOC_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Pointer".to_string(),
        frame: CodeFrame {
            stack_size: FRAME_SIZE,
            callee_saved: vec![abi::link_register().to_string()],
        },
        stack_slots: Vec::new(),
        instructions,
        relocations,
    }
}

/// `_mfb_make_error_result(x0=filename, x1=line, x2=char, x3=code, x4=message*)` —
/// build an `ErrorLoc` for the source location and land the standard error
/// `Result` in the return registers: `x0 = RESULT_ERR_TAG`, `x1 = code`,
/// `x2 = message*`, `x3 = ErrorLoc*` (null on OOM). The out-of-line form of the
/// per-trap-site register shuffle (`emit_error_register_return`, plan-16); each
/// site now just loads these five inputs and calls here. Mirrors
/// `lower_build_error_loc`: a framed function that preserves the code/message
/// across the `_mfb_build_error_loc` call.
pub(super) fn lower_make_error_result() -> CodeFunction {
    const FRAME_SIZE: usize = 32;
    const LR_SLOT: usize = 0;
    const CODE_SLOT: usize = 8;
    const MSG_SLOT: usize = 16;
    let mut relocations = Vec::new();
    let instructions = vec![
        abi::label("entry"),
        abi::subtract_stack(FRAME_SIZE),
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_SLOT),
        // Preserve code (x3) and message (x4) across the ErrorLoc allocation; x0/x1/x2
        // are already positioned as `_mfb_build_error_loc`'s filename/line/char args.
        abi::store_u64("x3", abi::stack_pointer(), CODE_SLOT),
        abi::store_u64("x4", abi::stack_pointer(), MSG_SLOT),
        abi::branch_link(BUILD_ERROR_LOC_SYMBOL),
        // Land the Result: tag=ERR, value=code, message=message, source=ErrorLoc.
        abi::move_register(RESULT_ERROR_SOURCE_REGISTER, abi::return_register()),
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), CODE_SLOT),
        abi::load_u64(RESULT_ERROR_MESSAGE_REGISTER, abi::stack_pointer(), MSG_SLOT),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_SLOT),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ];
    relocations.push(internal_branch(
        MAKE_ERROR_RESULT_SYMBOL,
        BUILD_ERROR_LOC_SYMBOL,
    ));
    CodeFunction {
        name: "runtime.make_error_result".to_string(),
        symbol: MAKE_ERROR_RESULT_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Pointer".to_string(),
        frame: CodeFrame {
            stack_size: FRAME_SIZE,
            callee_saved: vec![abi::link_register().to_string()],
        },
        stack_slots: Vec::new(),
        instructions,
        relocations,
    }
}

/// `arena_insert_free(x0 = ptr, x1 = size)` — insert a chunk into the
/// address-ordered free-list and coalesce with the address-adjacent neighbor on
/// either side. `size` must already be normalized (≥16, multiple of 16) and
/// `ptr` 16-aligned; both hold for every chunk the allocator hands out and for a
/// fresh block's usable region. Leaf function; clobbers x9–x13.
pub(super) fn lower_arena_insert_free() -> CodeFunction {
    let instructions = vec![
        abi::label("entry"),
        // Walk to the insertion slot: prev (x10) = largest node < ptr (or 0),
        // cur (x9) = smallest node > ptr (or 0).
        abi::load_u64("x9", ARENA_STATE_REGISTER, ARENA_FREE_LIST_HEAD_OFFSET),
        abi::move_immediate("x10", "Integer", "0"),
        abi::label("insert_find"),
        abi::compare_immediate("x9", "0"),
        abi::branch_eq("insert_slot"),
        abi::compare_registers("x9", "x0"),
        abi::branch_hi("insert_slot"), // cur > ptr
        abi::move_register("x10", "x9"),
        abi::load_u64("x9", "x9", 0),
        abi::branch("insert_find"),
        abi::label("insert_slot"),
        // x13 = merged-into-prev flag.
        abi::move_immediate("x13", "Integer", "0"),
        abi::compare_immediate("x10", "0"),
        abi::branch_eq("insert_check_next"),
        abi::load_u64("x11", "x10", 8),          // prev.size
        abi::add_registers("x12", "x10", "x11"), // prev_end
        abi::compare_registers("x12", "x0"),
        abi::branch_ne("insert_check_next"),
        // prev is address-adjacent: absorb the chunk into prev.
        abi::add_registers("x11", "x11", "x1"),
        abi::store_u64("x11", "x10", 8),
        abi::move_immediate("x13", "Integer", "1"),
        abi::label("insert_check_next"),
        abi::compare_immediate("x9", "0"),
        abi::branch_eq("insert_finish_no_next"),
        abi::compare_immediate("x13", "0"),
        abi::branch_eq("insert_next_unmerged"),
        // Merged into prev already: does the (now larger) prev meet cur?
        abi::load_u64("x11", "x10", 8),
        abi::add_registers("x12", "x10", "x11"),
        abi::compare_registers("x12", "x9"),
        abi::branch_ne("insert_done"),
        // Absorb cur into prev too (three-way merge).
        abi::load_u64("x11", "x9", 8),  // cur.size
        abi::load_u64("x12", "x10", 8), // prev.size
        abi::add_registers("x12", "x12", "x11"),
        abi::store_u64("x12", "x10", 8),
        abi::load_u64("x11", "x9", 0), // cur.next
        abi::store_u64("x11", "x10", 0),
        abi::branch("insert_done"),
        abi::label("insert_next_unmerged"),
        abi::add_registers("x12", "x0", "x1"), // chunk_end
        abi::compare_registers("x12", "x9"),
        abi::branch_ne("insert_standalone"),
        // chunk is address-adjacent to cur: new node at ptr absorbs cur.
        abi::load_u64("x11", "x9", 8), // cur.size
        abi::add_registers("x11", "x11", "x1"),
        abi::store_u64("x11", "x0", 8),
        abi::load_u64("x11", "x9", 0), // cur.next
        abi::store_u64("x11", "x0", 0),
        abi::branch("insert_link_prev"),
        abi::label("insert_standalone"),
        abi::store_u64("x9", "x0", 0), // ptr.next = cur
        abi::store_u64("x1", "x0", 8), // ptr.size = size
        abi::branch("insert_link_prev"),
        abi::label("insert_finish_no_next"),
        abi::compare_immediate("x13", "0"),
        abi::branch_ne("insert_done"), // merged into prev, nothing to link
        abi::store_u64("x31", "x0", 0), // ptr.next = 0
        abi::store_u64("x1", "x0", 8), // ptr.size = size
        abi::branch("insert_link_prev"),
        abi::label("insert_link_prev"),
        abi::compare_immediate("x10", "0"),
        abi::branch_eq("insert_set_head"),
        abi::store_u64("x0", "x10", 0), // prev.next = ptr
        abi::branch("insert_done"),
        abi::label("insert_set_head"),
        abi::store_u64("x0", ARENA_STATE_REGISTER, ARENA_FREE_LIST_HEAD_OFFSET),
        abi::label("insert_done"),
        abi::return_(),
    ];
    CodeFunction {
        name: "runtime.arena_insert_free".to_string(),
        symbol: ARENA_INSERT_FREE_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions,
        relocations: Vec::new(),
    }
}

/// `arena_free(x0 = ptr, x1 = size)` — return a single compiler-sized allocation
/// to the per-arena free-list. Normalizes `size` exactly as `arena_alloc` did
/// (so the freed extent matches the live chunk), entropy-scrubs the chunk
/// (plan-01 §6.2), then coalesces it in via `arena_insert_free`. Never unmaps.
/// Clobbers x9–x16.
pub(super) fn lower_arena_free() -> CodeFunction {
    const FRAME_SIZE: usize = 32;
    const LR_SLOT: usize = 0;
    const PTR_SLOT: usize = 8;
    const SIZE_SLOT: usize = 16;
    let not_15 = (!(ARENA_MIN_CHUNK - 1)).to_string();
    let instructions = vec![
        abi::label("entry"),
        abi::subtract_stack(FRAME_SIZE),
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_SLOT),
        // normalize size = round_up(max(size, 1), 16)
        abi::compare_immediate("x1", "0"),
        abi::branch_ne("arena_free_size_nonzero"),
        abi::move_immediate("x1", "Integer", "1"),
        abi::label("arena_free_size_nonzero"),
        abi::add_immediate("x1", "x1", (ARENA_MIN_CHUNK - 1) as usize),
        abi::move_immediate("x9", "Integer", &not_15),
        abi::and_registers("x1", "x1", "x9"),
        // Scrub the chunk: fill_random clobbers x0/x1/x9–x16 and advances x0, so
        // stash ptr/size and reload them for the coalescing insert.
        abi::store_u64("x0", abi::stack_pointer(), PTR_SLOT),
        abi::store_u64("x1", abi::stack_pointer(), SIZE_SLOT),
        abi::branch_link(ARENA_FILL_RANDOM_SYMBOL),
        abi::load_u64("x0", abi::stack_pointer(), PTR_SLOT),
        abi::load_u64("x1", abi::stack_pointer(), SIZE_SLOT),
        abi::branch_link(ARENA_INSERT_FREE_SYMBOL),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_SLOT),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ];
    CodeFunction {
        name: "runtime.arena_free".to_string(),
        symbol: ARENA_FREE_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: FRAME_SIZE,
            callee_saved: vec![abi::link_register().to_string()],
        },
        stack_slots: Vec::new(),
        instructions,
        relocations: vec![
            internal_branch(ARENA_FREE_SYMBOL, ARENA_FILL_RANDOM_SYMBOL),
            internal_branch(ARENA_FREE_SYMBOL, ARENA_INSERT_FREE_SYMBOL),
        ],
    }
}

pub(super) fn lower_arena_destroy(platform: &dyn CodegenPlatform) -> Result<CodeFunction, String> {
    let mut instructions = Vec::new();
    instructions.extend([
        abi::label("entry"),
        abi::load_u64("x20", ARENA_STATE_REGISTER, 0),
        abi::label("arena_destroy_loop"),
        abi::compare_immediate("x20", "0"),
        abi::branch_eq("arena_destroy_done"),
        abi::load_u64("x21", "x20", 0),
        abi::load_u64("x1", "x20", 8),
        abi::move_register(abi::return_register(), "x20"),
    ]);
    platform.emit_arena_unmap(&mut instructions)?;
    instructions.extend([
        abi::move_register("x20", "x21"),
        abi::branch("arena_destroy_loop"),
        abi::label("arena_destroy_done"),
        abi::store_u64("x31", ARENA_STATE_REGISTER, 0),
        abi::return_(),
    ]);
    Ok(CodeFunction {
        name: "runtime.arena_destroy".to_string(),
        symbol: ARENA_DESTROY_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions,
        relocations: Vec::new(),
    })
}

/// Shared process teardown. Reads the main arena-state address from the writable
/// global, clears the global (so a second entry — e.g. a signal arriving during
/// normal cleanup — becomes a no-op), pins it in `x19`, then conditionally
/// restores the terminal and frees the arena. Both underlying helpers are
/// idempotent (`term::off` gates on its `active` flag; `arena_destroy` clears the
/// block-list head), so the guard is belt-and-suspenders. Preserves `x19`/`x30`
/// for its callers (the entry exit path relies on `x19` afterwards).
pub(super) fn lower_shutdown(auto_term_off: bool, skip_arena_destroy: bool) -> CodeFunction {
    let done = "shutdown_done";
    let mut instructions = vec![
        abi::label("entry"),
        abi::subtract_stack(16),
        abi::store_u64(abi::link_register(), abi::stack_pointer(), 0),
        abi::store_u64(ARENA_STATE_REGISTER, abi::stack_pointer(), 8),
    ];
    let mut relocations = Vec::new();
    push_symbol_address(
        SHUTDOWN_SYMBOL,
        MAIN_ARENA_GLOBAL_SYMBOL,
        "x9",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::load_u64("x10", "x9", 0),
        abi::store_u64("x31", "x9", 0),
        abi::compare_immediate("x10", "0"),
        abi::branch_eq(done),
        abi::move_register(ARENA_STATE_REGISTER, "x10"),
    ]);
    if auto_term_off {
        instructions.push(abi::branch_link("_mfb_rt_term_term_off"));
        relocations.push(internal_branch(SHUTDOWN_SYMBOL, "_mfb_rt_term_term_off"));
    }
    if !skip_arena_destroy {
        instructions.push(abi::branch_link(ARENA_DESTROY_SYMBOL));
        relocations.push(internal_branch(SHUTDOWN_SYMBOL, ARENA_DESTROY_SYMBOL));
    }
    instructions.extend([
        abi::label(done),
        abi::load_u64(ARENA_STATE_REGISTER, abi::stack_pointer(), 8),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), 0),
        abi::add_stack(16),
        abi::return_(),
    ]);
    CodeFunction {
        name: "runtime.shutdown".to_string(),
        symbol: SHUTDOWN_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions,
        relocations,
    }
}

/// `void handler(int signo)` for SIGINT/SIGTERM: run the shared teardown, then
/// `_exit(128 + signo)`. It never returns, so it need not preserve the
/// interrupted context; it locates the arena through `_mfb_shutdown`'s global
/// read rather than the interrupted `x19`. The 16-byte frame keeps `sp` aligned
/// across the `bl`s (Darwin requires this) and parks `signo` across the call.
pub(super) fn lower_signal_handler(platform: &dyn CodegenPlatform) -> Result<CodeFunction, String> {
    let mut instructions = vec![
        abi::label("entry"),
        abi::subtract_stack(16),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), 0),
        abi::branch_link(SHUTDOWN_SYMBOL),
    ];
    let mut relocations = vec![internal_branch(SIGNAL_HANDLER_SYMBOL, SHUTDOWN_SYMBOL)];
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), 0),
        abi::add_immediate(abi::return_register(), abi::return_register(), 128),
        abi::add_stack(16),
    ]);
    platform.emit_program_exit(SIGNAL_HANDLER_SYMBOL, &mut instructions, &mut relocations)?;
    Ok(CodeFunction {
        name: "runtime.signal_handler".to_string(),
        symbol: SIGNAL_HANDLER_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions,
        relocations,
    })
}

/// Append the PCG64 LCG step `state = state * MULT + INC` operating on the
/// 128-bit state held in (`lo`, `hi`). The limbs are read at the start and
/// rewritten in place; `x11`-`x16` are used as scratch (caller-saved, so these
/// leaf helpers need not preserve them).
/// A monotonic virtual-register name generator for a hand-written vreg helper
/// (plan-00-G Phase 2): each call yields a fresh `%vN` the shared allocator
/// colors. Lets the PCG64 / arena helpers be written in target-neutral MIR (no
/// fixed `x9`/`x13`…) so register placement is a per-ISA backend job.
struct Vregs(usize);

impl Vregs {
    fn new() -> Self {
        Vregs(0)
    }

    fn next(&mut self) -> String {
        let name = format!("%v{}", self.0);
        self.0 += 1;
        name
    }
}

/// One PCG64 step `state = state * MULT + INC` (128-bit), over virtual registers.
/// `lo`/`hi` are the caller's state vregs (read and rewritten in place); all
/// scratch is freshly generated. The increment is added with explicit-carry
/// `add_carry` (the carry is a vreg value, not the flags register — plan-00-G §4),
/// so the chain survives register allocation.
fn emit_pcg_step(instructions: &mut Vec<CodeInstruction>, vregs: &mut Vregs, lo: &str, hi: &str) {
    let mult_lo = vregs.next();
    let prod_lo = vregs.next();
    let prod_hi = vregs.next();
    let cross_lo = vregs.next();
    let mult_hi = vregs.next();
    let cross_hi = vregs.next();
    let inc_lo = vregs.next();
    let inc_hi = vregs.next();
    let carry = vregs.next();
    instructions.extend([
        // 128-bit (truncated) product of state by the 128-bit multiplier.
        abi::move_immediate(&mult_lo, "Integer", &PCG_MULT_LO.to_string()),
        abi::multiply_registers(&prod_lo, &mult_lo, lo), // result low limb
        abi::unsigned_multiply_high_registers(&prod_hi, &mult_lo, lo), // carry into high
        abi::multiply_registers(&cross_lo, &mult_lo, hi), // MULT_LO * state_hi
        abi::move_immediate(&mult_hi, "Integer", &PCG_MULT_HI.to_string()),
        abi::multiply_registers(&cross_hi, &mult_hi, lo), // MULT_HI * state_lo
        abi::add_registers(&prod_hi, &prod_hi, &cross_lo),
        abi::add_registers(&prod_hi, &prod_hi, &cross_hi), // result high limb
        // Add the 128-bit increment with the carry as an explicit value.
        abi::move_immediate(&inc_lo, "Integer", &PCG_INC_LO.to_string()),
        abi::move_immediate(&inc_hi, "Integer", &PCG_INC_HI.to_string()),
        abi::add_carry(lo, &carry, &prod_lo, &inc_lo, "xzr"),
        abi::add_carry(hi, "xzr", &prod_hi, &inc_hi, &carry),
    ]);
}

/// `_mfb_rng_next` — advance the calling thread's PCG64 generator one step and
/// return the next 64-bit value in `x0`. State lives in the arena (`x19`).
pub(super) fn lower_rng_next() -> CodeFunction {
    emit_rng_draw(
        "runtime.rng_next",
        RNG_NEXT_SYMBOL,
        ARENA_RNG_STATE_LO_OFFSET,
        ARENA_RNG_STATE_HI_OFFSET,
    )
}

/// One PCG64 draw over vregs: load state from `[x19 + lo/hi]`, step it, store it
/// back, and return the XSL-RR output (rotate `hi ^ lo` right by the top 6 bits
/// of `hi`) in `x0`. Shared by the main RNG (`rng_next`) and the per-arena fill
/// RNG (`arena_fill_next`). `x19` is the reserved arena register (`arena_base`).
fn emit_rng_draw(name: &str, symbol: &str, lo_offset: usize, hi_offset: usize) -> CodeFunction {
    let mut vregs = Vregs::new();
    let lo = vregs.next();
    let hi = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::load_u64(&lo, ARENA_STATE_REGISTER, lo_offset),
        abi::load_u64(&hi, ARENA_STATE_REGISTER, hi_offset),
    ];
    emit_pcg_step(&mut instructions, &mut vregs, &lo, &hi);
    let shift = vregs.next();
    let xored = vregs.next();
    instructions.extend([
        abi::store_u64(&lo, ARENA_STATE_REGISTER, lo_offset),
        abi::store_u64(&hi, ARENA_STATE_REGISTER, hi_offset),
        abi::shift_right_immediate(&shift, &hi, 58),
        abi::exclusive_or_registers(&xored, &hi, &lo),
        abi::rotate_right_registers(abi::return_register(), &xored, &shift),
        abi::return_(),
    ]);
    finalize_vreg_helper(name, symbol, "Integer", instructions, Vec::new())
}

/// `_mfb_rng_seed_at(x0 = arena ptr, x1 = seed)` — initialize the PCG64 state at
/// the given arena from a 64-bit seed, following the canonical seeding dance
/// (`state = 0; step; state += seed; step`).
pub(super) fn lower_rng_seed_at() -> CodeFunction {
    emit_seed_dance(
        "runtime.rng_seed_at",
        RNG_SEED_SYMBOL,
        ARENA_RNG_STATE_LO_OFFSET,
        ARENA_RNG_STATE_HI_OFFSET,
    )
}

/// The canonical PCG64 seeding dance over vregs: `state = 0; step; state +=
/// seed(x1); step; store at x0+lo/hi`. Shared by the main RNG (`rng_seed_at`)
/// and the per-arena fill RNG (`arena_fill_seed`) — same dance, different state
/// words. `x0` (arena ptr) and `x1` (seed) stay physical ABI registers (they are
/// not in the allocatable set, so the allocator never colors them).
fn emit_seed_dance(name: &str, symbol: &str, lo_offset: usize, hi_offset: usize) -> CodeFunction {
    let mut vregs = Vregs::new();
    let lo = vregs.next();
    let hi = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_immediate(&lo, "Integer", "0"),
        abi::move_immediate(&hi, "Integer", "0"),
    ];
    emit_pcg_step(&mut instructions, &mut vregs, &lo, &hi);
    let carry = vregs.next();
    instructions.extend([
        // state += seed (x1), carry as an explicit value (plan-00-G §4).
        abi::add_carry(&lo, &carry, &lo, "x1", "xzr"),
        abi::add_carry(&hi, "xzr", &hi, "xzr", &carry),
    ]);
    emit_pcg_step(&mut instructions, &mut vregs, &lo, &hi);
    instructions.extend([
        abi::store_u64(&lo, "x0", lo_offset),
        abi::store_u64(&hi, "x0", hi_offset),
        abi::return_(),
    ]);
    finalize_vreg_helper(name, symbol, "Nothing", instructions, Vec::new())
}

/// `arena_fill_seed(x0 = arena ptr, x1 = seed)` — seed the dedicated fill RNG at
/// offsets 16/24 from a 64-bit seed (same PCG64 dance as `rng_seed_at`, different
/// state words). Leaf; clobbers x9–x16.
pub(super) fn lower_arena_fill_seed() -> CodeFunction {
    emit_seed_dance(
        "runtime.arena_fill_seed",
        ARENA_FILL_SEED_SYMBOL,
        ARENA_FILL_RNG_LO_OFFSET,
        ARENA_FILL_RNG_HI_OFFSET,
    )
}

/// `arena_fill_next()` — advance the calling thread's fill RNG (`x19`, offsets
/// 16/24) and return the next 64-bit XSL-RR output in `x0`. Leaf; clobbers
/// x9–x16. Used only to draw a child fill seed from the parent at spawn.
pub(super) fn lower_arena_fill_next() -> CodeFunction {
    emit_rng_draw(
        "runtime.arena_fill_next",
        ARENA_FILL_NEXT_SYMBOL,
        ARENA_FILL_RNG_LO_OFFSET,
        ARENA_FILL_RNG_HI_OFFSET,
    )
}

/// `arena_fill_random(x0 = ptr, x1 = len)` — overwrite `len` bytes at `ptr` with
/// output from the calling thread's fill RNG. `len` is rounded up to an 8-byte
/// word; every chunk handed to this helper is a multiple of 16 bytes, so the
/// rounding is exact and never writes past the chunk. Streams PRNG words without
/// a syscall (§6.1). Leaf; clobbers x0, x1, x9–x16.
pub(super) fn lower_arena_fill_random() -> CodeFunction {
    let mut vregs = Vregs::new();
    // The PCG64 state is loop-carried across the fill loop, so `lo`/`hi` are the
    // same vregs the allocator keeps in registers across the back-edge. `x0`
    // (ptr) and `x1` (word count) stay physical — ABI args used as loop counters;
    // this is a leaf, so nothing clobbers them.
    let lo = vregs.next();
    let hi = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        // word count = (len + 7) >> 3
        abi::add_immediate("x1", "x1", 7),
        abi::shift_right_immediate("x1", "x1", 3),
        abi::compare_immediate("x1", "0"),
        abi::branch_eq("arena_fill_done"),
        abi::load_u64(&lo, ARENA_STATE_REGISTER, ARENA_FILL_RNG_LO_OFFSET),
        abi::load_u64(&hi, ARENA_STATE_REGISTER, ARENA_FILL_RNG_HI_OFFSET),
        abi::label("arena_fill_loop"),
    ];
    emit_pcg_step(&mut instructions, &mut vregs, &lo, &hi);
    let shift = vregs.next();
    let xored = vregs.next();
    let word = vregs.next();
    instructions.extend([
        abi::shift_right_immediate(&shift, &hi, 58),
        abi::exclusive_or_registers(&xored, &hi, &lo),
        abi::rotate_right_registers(&word, &xored, &shift),
        abi::store_u64(&word, "x0", 0),
        abi::add_immediate("x0", "x0", 8),
        abi::subtract_immediate("x1", "x1", 1),
        abi::compare_immediate("x1", "0"),
        abi::branch_ne("arena_fill_loop"),
        abi::store_u64(&lo, ARENA_STATE_REGISTER, ARENA_FILL_RNG_LO_OFFSET),
        abi::store_u64(&hi, ARENA_STATE_REGISTER, ARENA_FILL_RNG_HI_OFFSET),
        abi::label("arena_fill_done"),
        abi::return_(),
    ]);
    finalize_vreg_helper(
        "runtime.arena_fill_random",
        ARENA_FILL_RANDOM_SYMBOL,
        "Nothing",
        instructions,
        Vec::new(),
    )
}

fn emit_write_string_object(
    symbol: &str,
    from: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    instructions.extend([
        abi::load_page_address("x21", symbol),
        abi::add_page_offset("x21", "x21", symbol),
        abi::load_u64(abi::string_length_register(), "x21", 0),
        abi::add_immediate(abi::string_data_register(), "x21", 8),
        abi::move_immediate(abi::return_register(), "Integer", "2"),
    ]);
    relocations.extend([
        CodeRelocation {
            from: from.to_string(),
            to: symbol.to_string(),
            kind: RelocIntent::DataAddrHi,
            binding: "data".to_string(),
            library: None,
        },
        CodeRelocation {
            from: from.to_string(),
            to: symbol.to_string(),
            kind: RelocIntent::DataAddrLo,
            binding: "data".to_string(),
            library: None,
        },
    ]);
    platform.emit_write(from, platform_imports, instructions, relocations)
}

fn emit_write_integer_to_stderr(
    from: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    emit_write_integer_to_stderr_with_labels(
        from,
        platform_imports,
        platform,
        instructions,
        relocations,
        "entry_error_code",
    )
}

fn emit_write_integer_to_stderr_with_labels(
    from: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    label_prefix: &str,
) -> Result<(), String> {
    let absolute_ready_label = format!("{label_prefix}_absolute_ready");
    let digit_loop_label = format!("{label_prefix}_digit_loop");
    let digits_done_label = format!("{label_prefix}_digits_done");
    let write_label = format!("{label_prefix}_write");
    instructions.extend([
        abi::subtract_stack(64),
        abi::load_u64("x21", ARENA_STATE_REGISTER, 32),
        abi::compare_immediate("x21", "0"),
        abi::branch_ge(&absolute_ready_label),
        abi::move_immediate("x22", "Integer", "0"),
        abi::subtract_registers("x21", "x22", "x21"),
        abi::label(&absolute_ready_label),
        abi::add_immediate("x23", abi::stack_pointer(), 64),
        abi::move_immediate("x24", "Integer", "10"),
        abi::compare_immediate("x21", "0"),
        abi::branch_ne(&digit_loop_label),
        abi::subtract_immediate("x23", "x23", 1),
        abi::move_immediate("x22", "Integer", "48"),
        abi::store_u8("x22", "x23", 0),
        abi::branch(&digits_done_label),
        abi::label(&digit_loop_label),
        abi::unsigned_divide_registers("x25", "x21", "x24"),
        abi::multiply_subtract_registers("x26", "x25", "x24", "x21"),
        abi::add_immediate("x26", "x26", 48),
        abi::subtract_immediate("x23", "x23", 1),
        abi::store_u8("x26", "x23", 0),
        abi::move_register("x21", "x25"),
        abi::compare_immediate("x21", "0"),
        abi::branch_ne(&digit_loop_label),
        abi::label(&digits_done_label),
        abi::compare_immediate("x19", "0"),
        abi::branch_ge(&write_label),
        abi::subtract_immediate("x23", "x23", 1),
        abi::move_immediate("x22", "Integer", "45"),
        abi::store_u8("x22", "x23", 0),
        abi::label(&write_label),
        abi::add_immediate("x27", abi::stack_pointer(), 64),
        abi::subtract_registers(abi::string_length_register(), "x27", "x23"),
        abi::move_register(abi::string_data_register(), "x23"),
        abi::move_immediate(abi::return_register(), "Integer", "2"),
    ]);
    platform.emit_write(from, platform_imports, instructions, relocations)?;
    instructions.push(abi::add_stack(64));
    Ok(())
}

