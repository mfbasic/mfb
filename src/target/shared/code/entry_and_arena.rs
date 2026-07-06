use super::*;

#[allow(clippy::too_many_arguments)]
pub(crate) fn lower_program_entry(
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
    let mut instructions = vec![abi::label("entry")];
    // A raw Linux ELF entry is jumped to with `argc` at `[sp]` / `argv` at
    // `[sp+8]` and undefined argument registers; load them into the `x0`/`x1`
    // the rest of the entry expects BEFORE the frame is carved (the entry does
    // not pass through `finalize_frame`, so `[sp,0]` here is the true initial
    // stack). macOS delivers them in `x0`/`x1` (libSystem calls `main`).
    if language_entry_accepts_args && !platform.entry_args_in_registers() {
        instructions.extend([
            abi::load_u64("x0", abi::stack_pointer(), 0),
            abi::add_immediate("x1", abi::stack_pointer(), 8),
        ]);
    }
    instructions.extend([
        abi::subtract_stack(entry_stack_size),
        abi::add_immediate(ARENA_STATE_REGISTER, abi::stack_pointer(), 0),
        // Zero the whole arena state with a loop (allocator-04): the entry
        // frame is live stack, NOT zero-filled, and this initializer must stay
        // in lockstep with the thread-spawn child-state zeroing
        // (`runtime_helpers.rs` `lower_thread_start_helper`) — both zero
        // exactly `ARENA_STATE_SIZE`, so growing the state (e.g. quick bins)
        // can never leave a field as garbage in one path but not the other.
        // `x9`/`x10` are free scratch here; `x0`/`x1` (argc/argv) are live.
        abi::move_register("x9", ARENA_STATE_REGISTER),
        abi::add_immediate("x10", ARENA_STATE_REGISTER, ARENA_STATE_SIZE),
        abi::label("entry_arena_state_zero"),
        abi::store_u64("x31", "x9", 0),
        abi::add_immediate("x9", "x9", 8),
        abi::compare_registers("x9", "x10"),
        abi::branch_lo("entry_arena_state_zero"),
    ]);
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
    //
    // Park argc/argv into callee-saved x27/x28 NOW, while they are still live in
    // x0/x1. Everything from here on — the seed_rng `getentropy`/`RNG_SEED`
    // calls, the always-on fill block's `clock_gettime`/`getentropy`/
    // `ARENA_FILL_SEED`, and the later `LINK`/global-initializer calls — clobbers
    // x0/x1, so an arg-accepting entry must preserve them across all of it and
    // read the args region back from x27/x28. Doing this BEFORE the seed_rng
    // block fixes the crash where a program that both takes `args` and uses
    // `math::rand` parked garbage (the `RNG_SEED` return) instead of argv.
    // Non-arg entries keep the original sequence (parked inside the fill block)
    // so their entry code stays byte-identical.
    if language_entry_accepts_args {
        instructions.extend([
            abi::move_register("x27", "x0"),
            abi::move_register("x28", "x1"),
        ]);
    }
    if seed_rng {
        instructions.extend([
            abi::store_u64(
                ARENA_STATE_REGISTER,
                abi::stack_pointer(),
                ENTRY_SEED_SCRATCH_OFFSET,
            ),
            abi::add_immediate(
                abi::return_register(),
                abi::stack_pointer(),
                ENTRY_SEED_SCRATCH_OFFSET,
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
            abi::load_u64("x1", abi::stack_pointer(), ENTRY_SEED_SCRATCH_OFFSET),
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
    // `argc`/`argv` (x0/x1) may still be live here (for a non-arg entry they are
    // irrelevant), and this block clobbers x0–x16, so park them in callee-saved
    // x27/x28 — preserved by the libc calls and the fill helpers — and restore
    // them afterward. A local 16-byte stack buffer holds first the `timespec`
    // and then the entropy bytes, so no entry-stack slot is touched. An
    // arg-accepting entry has already parked argc/argv into x27/x28 above (before
    // the seed_rng block clobbered x0/x1), so it must NOT re-park garbage here.
    if !language_entry_accepts_args {
        instructions.extend([
            abi::move_register("x27", "x0"),
            abi::move_register("x28", "x1"),
        ]);
    }
    instructions.extend([
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
        // The args region sits at the top of the entry frame (above the
        // globals); `entry_stack_size` includes ENTRY_ARGS_REGION_SIZE for an
        // arg-accepting entry (see the mod.rs sizing).
        let args_base = entry_stack_size - ENTRY_ARGS_REGION_SIZE;
        // Source argc/argv from the preserved callee-saved registers rather than
        // x0/x1: a `LINK` initializer or global initializer runs between here and
        // the top-of-entry parking, and those `bl`s clobber x0/x1 (but preserve
        // x27/x28).
        instructions.extend([
            abi::store_u64("x27", abi::stack_pointer(), args_base),
            abi::store_u64("x28", abi::stack_pointer(), args_base + 8),
        ]);
        emit_entry_args_list_materialization(
            error_label,
            args_base,
            &mut instructions,
            &mut relocations,
        );
        instructions.push(abi::load_u64("x0", abi::stack_pointer(), args_base + 16));
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
    args_base: usize,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    instructions.extend([
        abi::load_u64("x20", abi::stack_pointer(), args_base),
        abi::load_u64("x21", abi::stack_pointer(), args_base + 8),
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
        abi::store_u64("x22", abi::stack_pointer(), args_base + 24),
        abi::store_u64("x20", abi::stack_pointer(), args_base + 32),
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
    // The fill phase below uses ONLY `x9`–`x17`: the x86 residual-scratch pool
    // has 11 distinct registers, so `map_scratch_register` wraps at `xN+11` —
    // `x9`/`x20` share rbx, `x10`/`x21` share rsi, and so on. Mixing the low
    // scratch with `x20`–`x28` in one live range is fine on AArch64 (all
    // distinct) but self-clobbering on x86 (the original fill loop's `x10` byte
    // load destroyed the `x21` argv cursor). `x9`–`x17` map to nine distinct
    // GPRs on both ISAs. Everything is reloaded from the entry-frame slots
    // after the allocation call, so nothing needs to survive it in a register.
    instructions.extend([
        abi::branch(error_label),
        abi::label("entry_args_alloc_ok"),
        abi::store_u64("x1", abi::stack_pointer(), args_base + 16),
        abi::load_u64("x16", abi::stack_pointer(), args_base + 24),
        abi::load_u64("x9", abi::stack_pointer(), args_base + 32),
        abi::move_immediate("x17", "Byte", &COLLECTION_KIND_LIST.to_string()),
        abi::store_u8("x17", "x1", COLLECTION_OFFSET_KIND),
        abi::move_immediate("x17", "Byte", &COLLECTION_TYPE_NONE.to_string()),
        abi::store_u8("x17", "x1", COLLECTION_OFFSET_KEY_TYPE),
        abi::move_immediate("x17", "Byte", &COLLECTION_TYPE_STRING.to_string()),
        abi::store_u8("x17", "x1", COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate("x17", "Byte", "1"),
        abi::store_u8("x17", "x1", COLLECTION_OFFSET_FLAGS_VERSION),
        abi::store_u64("x9", "x1", COLLECTION_OFFSET_COUNT),
        abi::store_u64("x9", "x1", COLLECTION_OFFSET_CAPACITY),
        abi::store_u64("x16", "x1", COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64("x16", "x1", COLLECTION_OFFSET_DATA_CAPACITY),
        // x11 = entry cursor, x12 = data write cursor (= entries end).
        abi::add_immediate("x11", "x1", COLLECTION_HEADER_SIZE),
        abi::move_immediate("x17", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("x12", "x9", "x17"),
        abi::add_registers("x12", "x11", "x12"),
        // x13 = value-offset accumulator, x14 = index, x10 = argv cursor.
        abi::move_immediate("x13", "Integer", "0"),
        abi::load_u64("x10", abi::stack_pointer(), args_base + 8),
        abi::move_immediate("x14", "Integer", "0"),
        abi::label("entry_args_fill_loop"),
        abi::compare_registers("x14", "x9"),
        abi::branch_eq("entry_args_fill_done"),
        abi::load_u64("x15", "x10", 0), // x15 = argv[i] (NUL-terminated source)
        abi::move_immediate("x17", "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
        abi::store_u8("x17", "x11", COLLECTION_ENTRY_OFFSET_FLAGS),
        abi::store_u64("x31", "x11", COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
        abi::store_u64("x31", "x11", COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
        abi::store_u64("x13", "x11", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        // Copy bytes until the NUL, counting the length in x16 as we go (one
        // pass replaces the original separate strlen + copy loops).
        abi::move_immediate("x16", "Integer", "0"),
        abi::label("entry_args_copy_loop"),
        abi::load_u8("x17", "x15", 0),
        abi::compare_immediate("x17", "0"),
        abi::branch_eq("entry_args_copy_done"),
        abi::store_u8("x17", "x12", 0),
        abi::add_immediate("x15", "x15", 1),
        abi::add_immediate("x12", "x12", 1),
        abi::add_immediate("x16", "x16", 1),
        abi::branch("entry_args_copy_loop"),
        abi::label("entry_args_copy_done"),
        abi::store_u64("x16", "x11", COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        abi::add_registers("x13", "x13", "x16"),
        abi::add_immediate("x11", "x11", COLLECTION_ENTRY_SIZE),
        abi::add_immediate("x10", "x10", 8),
        abi::add_immediate("x14", "x14", 1),
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
    // Vreg-allocated (plan-00-G Phase 2): the body names virtual registers and the
    // shared allocator places them per-ISA; `finalize_vreg_helper` runs the
    // allocator + `finalize_frame` (which builds the frame, saves the link
    // register because the grow path calls `arena_fill_random`, and saves any
    // callee-saved registers the allocator used).
    //
    // Register contract (allocator-06): the standard runtime-helper one the
    // regalloc call-clobber model already assumes — all caller-saved integer
    // registers (`x0`–`x17`) are clobbered; callee-saved (`x19`–`x28`) are
    // preserved by the PCS frame. No caller holds a value in a physical register
    // across the call (audited tree-wide; every caller spills to stack slots or
    // vregs). The historical `x8/x11/x12/x13/x17` survivor reservation was
    // byte-identical-migration scaffolding and is gone.
    let not_15 = (!(ARENA_MIN_CHUNK - 1)).to_string();
    let mut vregs = Vregs::new();
    // Values that live across blocks: the normalized request, the walk cursor, and
    // the split geometry. `size`/`eff_align` are loop-carried (the grow path loops
    // back to the walk), so the allocator spills them across the grow call.
    let eff_align = vregs.next();
    let size = vregs.next();
    let cur = vregs.next();
    let prev = vregs.next();
    let cur_size = vregs.next();
    let aligned = vregs.next();
    let end_needed = vregs.next();
    let cur_end = vregs.next();
    // --- Validate alignment and normalize the request --------------------------
    let align_low = vregs.next();
    let align_pow2 = vregs.next();
    let not15 = vregs.next();
    let max_request = vregs.next();
    // Quick-bin fast path + flush-before-grow state (allocator-01).
    let flushed = vregs.next();
    let bin_class = vregs.next();
    let bin_slot = vregs.next();
    let bin_head = vregs.next();
    let bin_next = vregs.next();
    let bin_scan = vregs.next();
    let bin_scan_end = vregs.next();
    let bin_rem = vregs.next();
    // Segregated large-block bins (plan-25-A): a large request pops an exact-size
    // node from its hashed bin before falling to the first-fit walk.
    let lg_mask = vregs.next();
    let lg_slot = vregs.next();
    let lg_link = vregs.next();
    let lg_cur = vregs.next();
    let lg_next = vregs.next();
    let lg_msize = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::compare_immediate("x1", "0"),
        abi::branch_eq("arena_alloc_invalid"),
        abi::subtract_immediate(&align_low, "x1", 1),
        abi::and_registers(&align_pow2, "x1", &align_low),
        abi::compare_immediate(&align_pow2, "0"),
        abi::branch_ne("arena_alloc_invalid"),
        // eff align = max(align, 16)
        abi::move_register(&eff_align, "x1"),
        abi::compare_immediate(&eff_align, &ARENA_MIN_CHUNK.to_string()),
        abi::branch_lo("arena_alloc_align_min"),
        abi::branch("arena_alloc_align_ready"),
        abi::label("arena_alloc_align_min"),
        abi::move_immediate(&eff_align, "Integer", &ARENA_MIN_CHUNK.to_string()),
        abi::label("arena_alloc_align_ready"),
        // normalized size = round_up(max(size, 1), 16)
        abi::move_register(&size, "x0"),
        // Reject a raw request within ARENA_MIN_CHUNK of u64::MAX before the
        // +15 granule round-up (allocator-02, audit-1 MEM-07): without this
        // bound the round-up wraps and the allocation succeeds *small*, turning
        // every unchecked caller-side size computation into a heap OOB write.
        // No request this large can ever be satisfied, so rejecting it as
        // invalid loses nothing.
        abi::move_immediate(
            &max_request,
            "Integer",
            &(u64::MAX - ARENA_MIN_CHUNK).to_string(),
        ),
        abi::compare_registers(&size, &max_request),
        abi::branch_hi("arena_alloc_invalid"),
        abi::compare_immediate(&size, "0"),
        abi::branch_ne("arena_alloc_size_nonzero"),
        abi::move_immediate(&size, "Integer", "1"),
        abi::label("arena_alloc_size_nonzero"),
        abi::add_immediate(&size, &size, (ARENA_MIN_CHUNK - 1) as usize),
        abi::move_immediate(&not15, "Integer", &not_15),
        abi::and_registers(&size, &size, &not15),
        // --- Quick-bin pop (allocator-01) ---------------------------------------
        // An exact-class bin hit serves the request in O(1): both sides
        // normalize identically (≥16, 16-multiple) and every chunk ever handed
        // out is 16-aligned, so any bin node satisfies any eff_align ≤ 16
        // request of its class. eff_align > 16 requests bypass bins entirely.
        // `flushed` arms the one flush-before-grow retry below.
        abi::move_immediate(&flushed, "Integer", "0"),
        abi::compare_immediate(&eff_align, &ARENA_MIN_CHUNK.to_string()),
        abi::branch_hi("arena_alloc_walk"),
        abi::compare_immediate(&size, &ARENA_QUICK_BIN_MAX.to_string()),
        abi::branch_hi("arena_alloc_large_bin"),
        abi::shift_right_immediate(&bin_class, &size, 4),
        abi::shift_left_immediate(&bin_class, &bin_class, 3),
        abi::add_registers(&bin_slot, ARENA_STATE_REGISTER, &bin_class),
        abi::load_u64(&bin_head, &bin_slot, ARENA_QUICK_BIN_BASE_OFFSET - 8),
        abi::compare_immediate(&bin_head, "0"),
        abi::branch_eq("arena_alloc_bin_scan"),
        abi::load_u64(&bin_next, &bin_head, 0),
        abi::store_u64(&bin_next, &bin_slot, ARENA_QUICK_BIN_BASE_OFFSET - 8),
        abi::move_immediate(abi::return_register(), "Integer", RESULT_OK_TAG),
        abi::move_register("x1", &bin_head),
        abi::branch("arena_alloc_ret"),
        // Exact bin empty (allocator-01): bump-serve from the designated
        // victim (DV) — one active carve chunk held in the arena state.
        // Splitting parked bin inventory on every miss shaves it into
        // sub-class crumbs that nothing requests (measured on
        // benchmark/bignum-modexp: 21-30% hit rates, tens of thousands of
        // stranded fragments, flush storms); concentrating all small-miss
        // carving in one chunk keeps parked inventory intact so the exact-bin
        // hit rate climbs toward 100% under churn (dlmalloc's dv). The DV is
        // 16-aligned by construction and eff_align ≤ 16 on this path.
        abi::label("arena_alloc_bin_scan"),
        abi::load_u64(&bin_rem, ARENA_STATE_REGISTER, ARENA_CARVE_SIZE_OFFSET),
        abi::compare_registers(&bin_rem, &size),
        abi::branch_lo("arena_alloc_dv_renew"),
        abi::load_u64(&bin_head, ARENA_STATE_REGISTER, ARENA_CARVE_PTR_OFFSET),
        abi::subtract_registers(&bin_rem, &bin_rem, &size),
        abi::add_registers(&bin_next, &bin_head, &size),
        abi::store_u64(&bin_next, ARENA_STATE_REGISTER, ARENA_CARVE_PTR_OFFSET),
        abi::store_u64(&bin_rem, ARENA_STATE_REGISTER, ARENA_CARVE_SIZE_OFFSET),
        abi::move_immediate(abi::return_register(), "Integer", RESULT_OK_TAG),
        abi::move_register("x1", &bin_head),
        abi::branch("arena_alloc_ret"),
        // DV exhausted: retire its remnant (park ≤ QUICK_BIN_MAX in its exact
        // bin; a larger remnant joins the coalescing list) and acquire a new
        // DV — largest parked bin first (top-down scan, so the DV lives long),
        // then the walk (which hands over a WHOLE chunk — no split), then the
        // flush retry, then a fresh block from the grow path.
        abi::label("arena_alloc_dv_renew"),
        abi::compare_immediate(&bin_rem, "0"),
        abi::branch_eq("arena_alloc_dv_scan"),
        abi::load_u64(&bin_head, ARENA_STATE_REGISTER, ARENA_CARVE_PTR_OFFSET),
        abi::compare_immediate(&bin_rem, &ARENA_QUICK_BIN_MAX.to_string()),
        abi::branch_hi("arena_alloc_dv_retire_list"),
        abi::shift_right_immediate(&bin_scan, &bin_rem, 4),
        abi::shift_left_immediate(&bin_scan, &bin_scan, 3),
        abi::add_registers(&bin_scan, ARENA_STATE_REGISTER, &bin_scan),
        abi::load_u64(&bin_next, &bin_scan, ARENA_QUICK_BIN_BASE_OFFSET - 8),
        abi::store_u64(&bin_next, &bin_head, 0),
        abi::store_u64(&bin_rem, &bin_head, 8),
        abi::store_u64(&bin_head, &bin_scan, ARENA_QUICK_BIN_BASE_OFFSET - 8),
        abi::branch("arena_alloc_dv_cleared"),
        abi::label("arena_alloc_dv_retire_list"),
        // Rare: a large remnant coalesces back into the list. `size` and
        // `eff_align` are loop-carried vregs, spilled across the call.
        abi::move_register("x0", &bin_head),
        abi::move_register("x1", &bin_rem),
        abi::branch_link(ARENA_INSERT_FREE_SYMBOL),
        abi::label("arena_alloc_dv_cleared"),
        abi::store_u64("x31", ARENA_STATE_REGISTER, ARENA_CARVE_SIZE_OFFSET),
        // Acquire: largest parked bin ≥ this request becomes the new DV.
        abi::label("arena_alloc_dv_scan"),
        abi::add_immediate(
            &bin_scan_end,
            ARENA_STATE_REGISTER,
            ARENA_QUICK_BIN_BASE_OFFSET - 8,
        ),
        abi::add_registers(&bin_scan_end, &bin_scan_end, &bin_class),
        abi::add_immediate(
            &bin_scan,
            ARENA_STATE_REGISTER,
            ARENA_QUICK_BIN_BASE_OFFSET + ARENA_QUICK_BIN_COUNT * 8,
        ),
        abi::label("arena_alloc_dv_scan_loop"),
        abi::subtract_immediate(&bin_scan, &bin_scan, 8),
        abi::compare_registers(&bin_scan, &bin_scan_end),
        abi::branch_lo("arena_alloc_walk"),
        abi::load_u64(&bin_head, &bin_scan, 0),
        abi::compare_immediate(&bin_head, "0"),
        abi::branch_eq("arena_alloc_dv_scan_loop"),
        abi::load_u64(&bin_next, &bin_head, 0),
        abi::store_u64(&bin_next, &bin_scan, 0),
        abi::load_u64(&bin_rem, &bin_head, 8),
        abi::label("arena_alloc_dv_serve"),
        // Serve `size` from the new DV chunk (bin_head/bin_rem) and store the
        // shrunken DV.
        abi::subtract_registers(&bin_rem, &bin_rem, &size),
        abi::add_registers(&bin_next, &bin_head, &size),
        abi::store_u64(&bin_next, ARENA_STATE_REGISTER, ARENA_CARVE_PTR_OFFSET),
        abi::store_u64(&bin_rem, ARENA_STATE_REGISTER, ARENA_CARVE_SIZE_OFFSET),
        abi::move_immediate(abi::return_register(), "Integer", RESULT_OK_TAG),
        abi::move_register("x1", &bin_head),
        abi::branch("arena_alloc_ret"),
        // --- Segregated large-block bin pop (plan-25-A) ------------------------
        // A large request (size > QUICK_BIN_MAX, eff_align ≤ 16 — larger aligns
        // branched straight to the walk above) first scans its hashed bin for an
        // EXACT-size free node and returns it whole (no split) in O(1) amortized.
        // Diverting large frees off the address-ordered list (see arena_free)
        // keeps that list short, so both a bin hit here and a bin miss's
        // fall-through walk stay cheap under heavy large-list churn — the
        // benchmark's ~30× inflation was this list growing without bound. The
        // scan is an exact match because free and alloc normalize `size`
        // identically, so a reused chunk round-trips to the same bin; a chunk of
        // a different colliding size is simply skipped (it stays parked and is
        // recovered by the large flush-before-grow drain).
        abi::label("arena_alloc_large_bin"),
        abi::shift_right_immediate(&lg_slot, &size, 4),
        abi::move_immediate(
            &lg_mask,
            "Integer",
            &(ARENA_LARGE_BIN_COUNT - 1).to_string(),
        ),
        abi::and_registers(&lg_slot, &lg_slot, &lg_mask),
        abi::shift_left_immediate(&lg_slot, &lg_slot, 3),
        abi::add_registers(&lg_slot, ARENA_STATE_REGISTER, &lg_slot),
        // lg_link tracks the address of the word that points at lg_cur (the bin
        // head cell first, then each visited node's `next` at +0), so an
        // exact-size hit unlinks in O(1) whether it is the head or mid-list.
        abi::add_immediate(&lg_link, &lg_slot, ARENA_LARGE_BIN_BASE_OFFSET),
        abi::load_u64(&lg_cur, &lg_link, 0),
        abi::label("arena_alloc_large_scan"),
        abi::compare_immediate(&lg_cur, "0"),
        abi::branch_eq("arena_alloc_walk"),
        abi::load_u64(&lg_msize, &lg_cur, 8),
        abi::compare_registers(&lg_msize, &size),
        abi::branch_eq("arena_alloc_large_hit"),
        abi::move_register(&lg_link, &lg_cur),
        abi::load_u64(&lg_cur, &lg_cur, 0),
        abi::branch("arena_alloc_large_scan"),
        abi::label("arena_alloc_large_hit"),
        abi::load_u64(&lg_next, &lg_cur, 0),
        abi::store_u64(&lg_next, &lg_link, 0),
        abi::move_immediate(abi::return_register(), "Integer", RESULT_OK_TAG),
        abi::move_register("x1", &lg_cur),
        abi::branch("arena_alloc_ret"),
        // --- First-fit walk over the address-ordered free-list -----------------
        abi::label("arena_alloc_walk"),
        abi::load_u64(&cur, ARENA_STATE_REGISTER, ARENA_FREE_LIST_HEAD_OFFSET),
        abi::move_immediate(&prev, "Integer", "0"),
        abi::label("arena_alloc_walk_loop"),
        abi::compare_immediate(&cur, "0"),
        abi::branch_eq("arena_alloc_grow"),
        abi::load_u64(&cur_size, &cur, 8), // cur_size
    ];
    let align_mask = vregs.next();
    let align_notmask = vregs.next();
    instructions.extend([
        abi::subtract_immediate(&align_mask, &eff_align, 1), // align mask
        abi::add_registers(&aligned, &cur, &align_mask),
        abi::compare_registers(&aligned, &cur),
        abi::branch_lo("arena_alloc_walk_next"), // align overflow → skip
        abi::bitwise_not(&align_notmask, &align_mask),
        abi::and_registers(&aligned, &aligned, &align_notmask), // aligned
        abi::add_registers(&end_needed, &aligned, &size),       // end_needed
        abi::compare_registers(&end_needed, &aligned),
        abi::branch_lo("arena_alloc_walk_next"), // size overflow → skip
        abi::add_registers(&cur_end, &cur, &cur_size), // cur_end
        abi::compare_registers(&end_needed, &cur_end),
        abi::branch_hi("arena_alloc_walk_next"), // doesn't fit → next
        abi::branch("arena_alloc_found"),
        abi::label("arena_alloc_walk_next"),
        abi::move_register(&prev, &cur),
        abi::load_u64(&cur, &cur, 0),
        abi::branch("arena_alloc_walk_loop"),
    ]);
    // --- Found: split the chosen chunk -------------------------------------
    let next_node = vregs.next();
    let front_pad = vregs.next();
    let tail_size = vregs.next();
    let link = vregs.next();
    instructions.extend([
        // Split the chosen chunk. Remainders ≤ ARENA_QUICK_BIN_MAX go to their
        // exact-size bin instead of the list (allocator-01): a walk-split's
        // front/tail crumbs would otherwise accumulate at the head of the
        // address-ordered list — never allocated, never coalesced — and every
        // later walk would pay for them (measured: tens of thousands of 32–80
        // byte crumbs, linear growth, quadratic total). Binned remainders stay
        // poppable and re-coalesce at the next flush. The list link therefore
        // chains only the pieces that stay on the list, in address order
        // (cur < end_needed < next_node).
        abi::label("arena_alloc_found"),
        abi::load_u64(&next_node, &cur, 0), // next
        // A small request takes the WHOLE chunk as the new designated victim
        // (the old DV was retired before reaching the walk): unlink it and
        // bump-serve. Splitting here would shave parked inventory into
        // crumbs; big requests (or align > 16) keep the four-case split.
        abi::compare_immediate(&eff_align, &ARENA_MIN_CHUNK.to_string()),
        abi::branch_hi("arena_alloc_found_split"),
        abi::compare_immediate(&size, &ARENA_QUICK_BIN_MAX.to_string()),
        abi::branch_hi("arena_alloc_found_split"),
        abi::compare_immediate(&prev, "0"),
        abi::branch_eq("arena_alloc_found_dv_head"),
        abi::store_u64(&next_node, &prev, 0),
        abi::branch("arena_alloc_found_dv_take"),
        abi::label("arena_alloc_found_dv_head"),
        abi::store_u64(
            &next_node,
            ARENA_STATE_REGISTER,
            ARENA_FREE_LIST_HEAD_OFFSET,
        ),
        abi::label("arena_alloc_found_dv_take"),
        abi::move_register(&bin_head, &cur),
        abi::subtract_registers(&bin_rem, &cur_end, &cur),
        abi::branch("arena_alloc_dv_serve"),
        abi::label("arena_alloc_found_split"),
        abi::subtract_registers(&front_pad, &aligned, &cur), // front_pad
        abi::subtract_registers(&tail_size, &cur_end, &end_needed), // tail_size
        // Tail remainder first (higher address): bin it, list it, or nothing.
        abi::move_register(&link, &next_node),
        abi::compare_immediate(&tail_size, "0"),
        abi::branch_eq("arena_alloc_front"),
        abi::compare_immediate(&tail_size, &ARENA_QUICK_BIN_MAX.to_string()),
        abi::branch_hi("arena_alloc_tail_list"),
        abi::shift_right_immediate(&bin_class, &tail_size, 4),
        abi::shift_left_immediate(&bin_class, &bin_class, 3),
        abi::add_registers(&bin_slot, ARENA_STATE_REGISTER, &bin_class),
        abi::load_u64(&bin_head, &bin_slot, ARENA_QUICK_BIN_BASE_OFFSET - 8),
        abi::store_u64(&bin_head, &end_needed, 0),
        abi::store_u64(&tail_size, &end_needed, 8),
        abi::store_u64(&end_needed, &bin_slot, ARENA_QUICK_BIN_BASE_OFFSET - 8),
        abi::branch("arena_alloc_front"),
        abi::label("arena_alloc_tail_list"),
        abi::store_u64(&next_node, &end_needed, 0),
        abi::store_u64(&tail_size, &end_needed, 8),
        abi::move_register(&link, &end_needed),
        // Front remainder (lower address): bin it, list it, or nothing.
        abi::label("arena_alloc_front"),
        abi::compare_immediate(&front_pad, "0"),
        abi::branch_eq("arena_alloc_set_prev_link"),
        abi::compare_immediate(&front_pad, &ARENA_QUICK_BIN_MAX.to_string()),
        abi::branch_hi("arena_alloc_front_list"),
        abi::shift_right_immediate(&bin_class, &front_pad, 4),
        abi::shift_left_immediate(&bin_class, &bin_class, 3),
        abi::add_registers(&bin_slot, ARENA_STATE_REGISTER, &bin_class),
        abi::load_u64(&bin_head, &bin_slot, ARENA_QUICK_BIN_BASE_OFFSET - 8),
        abi::store_u64(&bin_head, &cur, 0),
        abi::store_u64(&front_pad, &cur, 8),
        abi::store_u64(&cur, &bin_slot, ARENA_QUICK_BIN_BASE_OFFSET - 8),
        abi::branch("arena_alloc_set_prev_link"),
        abi::label("arena_alloc_front_list"),
        abi::store_u64(&link, &cur, 0), // cur.next → tail node or next
        abi::store_u64(&front_pad, &cur, 8),
        abi::move_register(&link, &cur),
        abi::label("arena_alloc_set_prev_link"),
        abi::compare_immediate(&prev, "0"),
        abi::branch_eq("arena_alloc_set_head"),
        abi::store_u64(&link, &prev, 0),
        abi::branch("arena_alloc_done"),
        abi::label("arena_alloc_set_head"),
        abi::store_u64(&link, ARENA_STATE_REGISTER, ARENA_FREE_LIST_HEAD_OFFSET),
        abi::label("arena_alloc_done"),
        abi::move_immediate(abi::return_register(), "Integer", RESULT_OK_TAG),
        abi::move_register("x1", &aligned),
        abi::branch("arena_alloc_ret"),
    ]);
    // --- Grow: map a new block and carve the request from it ----------------
    let map_size = vregs.next();
    let default_block = vregs.next();
    let saved_size = vregs.next();
    let page_mask = vregs.next();
    // Flush-before-grow scratch (allocator-01): the drain loop's cursors are
    // loop-carried across the `arena_insert_free` calls, so they live in vregs
    // the allocator spills.
    let flush_index = vregs.next();
    let flush_offset = vregs.next();
    let flush_slot = vregs.next();
    let flush_node = vregs.next();
    let flush_next = vregs.next();
    instructions.extend([
        abi::label("arena_alloc_grow"),
        // Flush-before-grow (allocator-01), gated to SMALL requests: a small
        // request only reaches here after the exact bin, the larger-bin scan,
        // and the walk all failed — so the bins hold nothing ≥ this class,
        // the drain is cheap, and coalescing adjacent parked chunks genuinely
        // can produce a fit. A BIG request (> QUICK_BIN_MAX, or align > 16)
        // grows directly: its flush would drain a large parked-small inventory
        // through the O(list) insert (measured: hundreds of millions of insert
        // steps) for chunks that almost never coalesce past interleaved live
        // objects into a big-enough run. The `flushed` flag arms exactly one
        // retry — a second walk miss falls through to the map below.
        abi::compare_immediate(&flushed, "0"),
        abi::branch_ne("arena_alloc_grow_map"),
        abi::compare_immediate(&eff_align, &ARENA_MIN_CHUNK.to_string()),
        abi::branch_hi("arena_alloc_grow_map"),
        abi::compare_immediate(&size, &ARENA_QUICK_BIN_MAX.to_string()),
        abi::branch_hi("arena_alloc_grow_map"),
        abi::move_immediate(&flushed, "Integer", "1"),
        abi::move_immediate(&flush_index, "Integer", "0"),
        abi::label("arena_alloc_flush_bin"),
        abi::compare_immediate(&flush_index, &ARENA_QUICK_BIN_COUNT.to_string()),
        abi::branch_eq("arena_alloc_flush_done"),
        abi::shift_left_immediate(&flush_offset, &flush_index, 3),
        abi::add_registers(&flush_slot, ARENA_STATE_REGISTER, &flush_offset),
        abi::load_u64(&flush_node, &flush_slot, ARENA_QUICK_BIN_BASE_OFFSET),
        abi::store_u64("x31", &flush_slot, ARENA_QUICK_BIN_BASE_OFFSET),
        abi::label("arena_alloc_flush_chain"),
        abi::compare_immediate(&flush_node, "0"),
        abi::branch_eq("arena_alloc_flush_next_bin"),
        abi::load_u64(&flush_next, &flush_node, 0),
        abi::load_u64("x1", &flush_node, 8),
        abi::move_register("x0", &flush_node),
        abi::branch_link(ARENA_INSERT_FREE_SYMBOL),
        abi::move_register(&flush_node, &flush_next),
        abi::branch("arena_alloc_flush_chain"),
        abi::label("arena_alloc_flush_next_bin"),
        abi::add_immediate(&flush_index, &flush_index, 1),
        abi::branch("arena_alloc_flush_bin"),
        // Post-flush re-park sweep: after coalescing, move every list chunk
        // ≤ QUICK_BIN_MAX back onto its exact-size bin. Without this, drained
        // small chunks that did not merge into large runs rot on the list
        // forever — nothing small ever walks (bins and the victim serve
        // first), so ONLY large requests pay for them, once per walk
        // (measured: 17k dead 16-byte nodes doubling a JSON parse). After the
        // sweep the list holds only > QUICK_BIN_MAX chunks and the retry
        // re-enters through the victim-renewal bin scan, which sees every
        // swept chunk.
        abi::label("arena_alloc_flush_done"),
        abi::move_immediate(&flush_slot, "Integer", "0"), // prev
        abi::load_u64(
            &flush_node,
            ARENA_STATE_REGISTER,
            ARENA_FREE_LIST_HEAD_OFFSET,
        ),
        abi::label("arena_alloc_sweep_loop"),
        abi::compare_immediate(&flush_node, "0"),
        abi::branch_eq("arena_alloc_sweep_done"),
        abi::load_u64(&flush_next, &flush_node, 0),
        abi::load_u64(&flush_offset, &flush_node, 8),
        abi::compare_immediate(&flush_offset, &ARENA_QUICK_BIN_MAX.to_string()),
        abi::branch_hi("arena_alloc_sweep_keep"),
        // Unlink cur from the list …
        abi::compare_immediate(&flush_slot, "0"),
        abi::branch_eq("arena_alloc_sweep_unlink_head"),
        abi::store_u64(&flush_next, &flush_slot, 0),
        abi::branch("arena_alloc_sweep_binpush"),
        abi::label("arena_alloc_sweep_unlink_head"),
        abi::store_u64(
            &flush_next,
            ARENA_STATE_REGISTER,
            ARENA_FREE_LIST_HEAD_OFFSET,
        ),
        abi::label("arena_alloc_sweep_binpush"),
        // … and push it onto its exact-size bin (node.size at +8 is intact).
        abi::shift_right_immediate(&bin_scan, &flush_offset, 4),
        abi::shift_left_immediate(&bin_scan, &bin_scan, 3),
        abi::add_registers(&bin_scan, ARENA_STATE_REGISTER, &bin_scan),
        abi::load_u64(&bin_head, &bin_scan, ARENA_QUICK_BIN_BASE_OFFSET - 8),
        abi::store_u64(&bin_head, &flush_node, 0),
        abi::store_u64(&flush_node, &bin_scan, ARENA_QUICK_BIN_BASE_OFFSET - 8),
        abi::move_register(&flush_node, &flush_next),
        abi::branch("arena_alloc_sweep_loop"),
        abi::label("arena_alloc_sweep_keep"),
        abi::move_register(&flush_slot, &flush_node),
        abi::move_register(&flush_node, &flush_next),
        abi::branch("arena_alloc_sweep_loop"),
        abi::label("arena_alloc_sweep_done"),
        abi::branch("arena_alloc_dv_scan"),
        abi::label("arena_alloc_grow_map"),
        abi::add_registers(&map_size, &size, &eff_align),
        abi::compare_registers(&map_size, &size),
        abi::branch_lo("arena_alloc_oom"),
        abi::add_immediate(&map_size, &map_size, ARENA_BLOCK_HEADER_SIZE),
        // Carry-check the header add (allocator-02): a wrapped map_size would
        // round up to the default block size, the block could never satisfy
        // the huge request, and the walk-then-grow loop would mmap 4 KiB
        // blocks forever. A wrapped value is < ARENA_BLOCK_HEADER_SIZE while
        // any legitimate map_size is >= 1 + 16 + 32.
        abi::compare_immediate(&map_size, &ARENA_BLOCK_HEADER_SIZE.to_string()),
        abi::branch_lo("arena_alloc_oom"),
        abi::move_immediate(
            &default_block,
            "Integer",
            &ARENA_DEFAULT_BLOCK_SIZE.to_string(),
        ),
        abi::compare_registers(&map_size, &default_block),
        abi::branch_hi("arena_alloc_normal_block"),
        abi::move_immediate(&map_size, "Integer", &ARENA_DEFAULT_BLOCK_SIZE.to_string()),
        abi::branch("arena_alloc_map_size_ready"),
        abi::label("arena_alloc_normal_block"),
        abi::move_register(&saved_size, &map_size),
        abi::add_immediate(&map_size, &map_size, 4095),
        abi::compare_registers(&map_size, &saved_size),
        abi::branch_lo("arena_alloc_oom"),
        abi::move_immediate(&page_mask, "Integer", &(!4095_u64).to_string()),
        abi::and_registers(&map_size, &map_size, &page_mask),
        abi::label("arena_alloc_map_size_ready"),
    ]);
    // mmap `map_size` bytes; the result is left in the return register. `map_size`
    // is live across the syscall (read again below for the block header), so the
    // allocator keeps it in a callee-saved register or spills it.
    platform.emit_arena_map(&map_size, &mut instructions)?;
    let prev_block = vregs.next();
    let usable = vregs.next();
    let ubase = vregs.next();
    let ins_cur = vregs.next();
    let ins_prev = vregs.next();
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ge("arena_alloc_mapped"),
        abi::branch("arena_alloc_oom"),
        abi::label("arena_alloc_mapped"),
        // Write the block header (prevBlock, blockSize, usableCapacity, bumpOffset)
        // and chain it. bumpOffset is vestigial under the free-list but kept zero
        // so the documented block layout is unchanged.
        abi::load_u64(&prev_block, ARENA_STATE_REGISTER, 0),
        abi::store_u64(&prev_block, abi::return_register(), 0),
        abi::store_u64(&map_size, abi::return_register(), 8),
        abi::subtract_immediate(&usable, &map_size, ARENA_BLOCK_HEADER_SIZE),
        abi::store_u64(&usable, abi::return_register(), 16),
        abi::store_u64("x31", abi::return_register(), 24),
        abi::store_u64(abi::return_register(), ARENA_STATE_REGISTER, 0),
        // Poison the new block's usable region before first use (plan-01 §6.3).
        // `ubase`/`usable` are live across the fill call, so the allocator spills
        // them (the call's clobber mask is every integer register).
        abi::add_immediate(&ubase, abi::return_register(), ARENA_BLOCK_HEADER_SIZE), // ubase
        abi::move_register("x0", &ubase),
        abi::move_register("x1", &usable),
        abi::branch_link(ARENA_FILL_RANDOM_SYMBOL),
        // Serve the request directly from the fresh chunk (allocator-05): the
        // block was sized so `usable >= size + eff_align`, so instead of
        // linking the whole chunk and re-walking the entire list to rediscover
        // it, walk once to the chunk's address-ordered slot, park the successor
        // in the fresh node's `next` word (`arena_alloc_found` reads it from
        // `[cur, 0]`), and enter the existing four-case split with
        // `cur = ubase`, `prev = ins_prev`. The split links only the
        // remainder(s). A fresh block is never adjacent to an existing chunk
        // (the 32-byte header always separates blocks), so no coalescing is
        // required here.
        abi::load_u64(&ins_cur, ARENA_STATE_REGISTER, ARENA_FREE_LIST_HEAD_OFFSET), // cur
        abi::move_immediate(&ins_prev, "Integer", "0"),                             // prev
        abi::label("arena_alloc_ins_loop"),
        abi::compare_immediate(&ins_cur, "0"),
        abi::branch_eq("arena_alloc_ins_do"),
        abi::compare_registers(&ins_cur, &ubase),
        abi::branch_hi("arena_alloc_ins_do"),
        abi::move_register(&ins_prev, &ins_cur),
        abi::load_u64(&ins_cur, &ins_cur, 0),
        abi::branch("arena_alloc_ins_loop"),
        abi::label("arena_alloc_ins_do"),
        abi::store_u64(&ins_cur, &ubase, 0), // fresh.next = successor
        abi::move_register(&cur, &ubase),
        abi::move_register(&prev, &ins_prev),
        // aligned = round_up(ubase, eff_align); end_needed = aligned + size;
        // cur_end = ubase + usable — the same geometry the walk computes. The
        // walk's overflow-skip guards are unnecessary for a fresh mapping
        // (mmap'd extents cannot wrap).
        abi::subtract_immediate(&align_mask, &eff_align, 1),
        abi::add_registers(&aligned, &cur, &align_mask),
        abi::bitwise_not(&align_notmask, &align_mask),
        abi::and_registers(&aligned, &aligned, &align_notmask),
        abi::add_registers(&end_needed, &aligned, &size),
        abi::add_registers(&cur_end, &cur, &usable),
        abi::branch("arena_alloc_found"),
        abi::label("arena_alloc_invalid"),
        abi::move_immediate(abi::return_register(), "Integer", ERR_INVALID_ARGUMENT_CODE),
        abi::move_immediate("x1", "Integer", "0"),
        abi::branch("arena_alloc_ret"),
        abi::label("arena_alloc_oom"),
        abi::move_immediate(abi::return_register(), "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate("x1", "Integer", "0"),
        abi::label("arena_alloc_ret"),
        abi::return_(),
    ]);
    let relocations = vec![internal_branch(
        ARENA_ALLOC_SYMBOL,
        ARENA_FILL_RANDOM_SYMBOL,
    )];
    Ok(finalize_vreg_helper(
        "runtime.arena_alloc",
        ARENA_ALLOC_SYMBOL,
        "Pointer",
        instructions,
        relocations,
    ))
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
    let mut vregs = Vregs::new();
    // count/typeCode are live across the arena_alloc call (ALL_INT clobber), so
    // they spill — the old hand frame's COUNT/TYPE slots, now allocator-managed.
    let count = vregs.next();
    let type_code = vregs.next();
    let stride = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&count, "x0"),
        abi::move_register(&type_code, "x1"),
        // alloc size = COLLECTION_HEADER_SIZE + count*(ENTRY_SIZE + 8) (lookup + data).
        abi::move_immediate(&stride, "Integer", &(COLLECTION_ENTRY_SIZE + 8).to_string()),
        abi::multiply_registers("x0", &count, &stride),
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
    ];
    let base = vregs.next();
    let scratch = vregs.next();
    let data_len = vregs.next();
    let entry = vregs.next();
    let index = vregs.next();
    let value_off = vregs.next();
    instructions.extend([
        abi::move_register(&base, "x1"),
        // Header: kind=0 (list), keyType=0, valueType=typeCode, flagsVersion=1.
        abi::move_immediate(&scratch, "Integer", "0"),
        abi::store_u8(&scratch, &base, COLLECTION_OFFSET_KIND),
        abi::store_u8(&scratch, &base, COLLECTION_OFFSET_KEY_TYPE),
        abi::store_u8(&type_code, &base, COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate(&scratch, "Integer", "1"),
        abi::store_u8(&scratch, &base, COLLECTION_OFFSET_FLAGS_VERSION),
        // count, capacity = count; dataLength, dataCapacity = count*8.
        abi::store_u64(&count, &base, COLLECTION_OFFSET_COUNT),
        abi::store_u64(&count, &base, COLLECTION_OFFSET_CAPACITY),
        abi::shift_left_immediate(&data_len, &count, 3),
        abi::store_u64(&data_len, &base, COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64(&data_len, &base, COLLECTION_OFFSET_DATA_CAPACITY),
        // Fill the lookup entries: flags=USED, valueOffset=i*8, valueLength=8.
        abi::add_immediate(&entry, &base, COLLECTION_HEADER_SIZE),
        abi::move_immediate(&index, "Integer", "0"),
        abi::move_immediate(&value_off, "Integer", "0"),
        abi::label("simd_alloc_entry_loop"),
        abi::compare_registers(&index, &count),
        abi::branch_ge("simd_alloc_entry_done"),
        abi::move_immediate(&scratch, "Integer", &COLLECTION_ENTRY_FLAG_USED.to_string()),
        abi::store_u8(&scratch, &entry, COLLECTION_ENTRY_OFFSET_FLAGS),
        abi::store_u64(&value_off, &entry, COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::move_immediate(&scratch, "Integer", "8"),
        abi::store_u64(&scratch, &entry, COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        abi::add_immediate(&value_off, &value_off, 8),
        abi::add_immediate(&entry, &entry, COLLECTION_ENTRY_SIZE),
        abi::add_immediate(&index, &index, 1),
        abi::branch("simd_alloc_entry_loop"),
        abi::label("simd_alloc_entry_done"),
        abi::move_register(abi::return_register(), &base),
        abi::move_immediate("x1", "Integer", "0"),
        abi::label("simd_alloc_ret"),
        abi::return_(),
    ]);
    finalize_vreg_helper(
        "runtime.simd_alloc_list",
        SIMD_ALLOC_LIST_SYMBOL,
        "Pointer",
        instructions,
        vec![internal_branch(SIMD_ALLOC_LIST_SYMBOL, ARENA_ALLOC_SYMBOL)],
    )
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
    let mut vregs = Vregs::new();
    // filename/line/char and the computed len are live across the arena_alloc
    // call; held in vregs, the allocator spills them (arena_alloc tramples every
    // integer register — `call_clobber_mask` returns ALL_INT — so nothing
    // survives in a register, exactly as the old hand frame spilled them).
    let filename = vregs.next();
    let line = vregs.next();
    let char_pos = vregs.next();
    let len = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&filename, "x0"),
        abi::move_register(&line, "x1"),
        abi::move_register(&char_pos, "x2"),
        // len = *filename; size = ERROR_LOC_OBJECT_SIZE + len + 9 (inlined String).
        abi::load_u64(&len, &filename, 0),
        abi::add_immediate(abi::return_register(), &len, ERROR_LOC_OBJECT_SIZE + 9),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
        // x0 = result tag, x1 = pointer. On OOM (tag != ok) return a null pointer.
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq("build_error_loc_ok"),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::branch("build_error_loc_ret"),
        abi::label("build_error_loc_ok"),
    ];
    // x1 = ErrorLoc pointer (physical, preserved across the leaf copy below).
    let obj_off = vregs.next();
    let dst = vregs.next();
    let src = vregs.next();
    let remaining = vregs.next();
    let scratch = vregs.next();
    instructions.extend([
        // Fixed slots: filename block-relative offset @0 = OBJECT_SIZE, line @8, char @16.
        abi::move_immediate(&obj_off, "Integer", &ERROR_LOC_OBJECT_SIZE.to_string()),
        abi::store_u64(&obj_off, "x1", 0),
        abi::store_u64(&line, "x1", 8),
        abi::store_u64(&char_pos, "x1", 16),
        // Inline the filename String block (len + 9 bytes) at offset OBJECT_SIZE.
        abi::add_immediate(&dst, "x1", ERROR_LOC_OBJECT_SIZE),
        abi::move_register(&src, &filename),
        abi::add_immediate(&remaining, &len, 9),
        abi::label("build_error_loc_wloop"),
        abi::compare_immediate(&remaining, "8"),
        abi::branch_lo("build_error_loc_btail"),
        abi::load_u64(&scratch, &src, 0),
        abi::store_u64(&scratch, &dst, 0),
        abi::add_immediate(&src, &src, 8),
        abi::add_immediate(&dst, &dst, 8),
        abi::subtract_immediate(&remaining, &remaining, 8),
        abi::branch("build_error_loc_wloop"),
        abi::label("build_error_loc_btail"),
        abi::compare_immediate(&remaining, "0"),
        abi::branch_eq("build_error_loc_copy_done"),
        abi::load_u8(&scratch, &src, 0),
        abi::store_u8(&scratch, &dst, 0),
        abi::add_immediate(&src, &src, 1),
        abi::add_immediate(&dst, &dst, 1),
        abi::subtract_immediate(&remaining, &remaining, 1),
        abi::branch("build_error_loc_btail"),
        abi::label("build_error_loc_copy_done"),
        abi::move_register(abi::return_register(), "x1"),
        abi::label("build_error_loc_ret"),
        abi::return_(),
    ]);
    let relocations = vec![internal_branch(BUILD_ERROR_LOC_SYMBOL, ARENA_ALLOC_SYMBOL)];
    finalize_vreg_helper(
        "runtime.build_error_loc",
        BUILD_ERROR_LOC_SYMBOL,
        "Pointer",
        instructions,
        relocations,
    )
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
    let mut vregs = Vregs::new();
    // Preserve code (x3) and message (x4) across the ErrorLoc allocation: holding
    // them in vregs makes the allocator keep them in callee-saved registers (it
    // spills/saves them automatically — no manual frame slots). x0/x1/x2 are
    // already positioned as `_mfb_build_error_loc`'s filename/line/char args.
    let code = vregs.next();
    let message = vregs.next();
    let instructions = vec![
        abi::label("entry"),
        abi::move_register(&code, "x3"),
        abi::move_register(&message, "x4"),
        abi::branch_link(BUILD_ERROR_LOC_SYMBOL),
        // Land the Result: tag=ERR, value=code, message=message, source=ErrorLoc.
        // Set source (x3) from the call result (x0) before x0 is reused for the tag.
        abi::move_register(RESULT_ERROR_SOURCE_REGISTER, abi::return_register()),
        abi::move_register(RESULT_VALUE_REGISTER, &code),
        abi::move_register(RESULT_ERROR_MESSAGE_REGISTER, &message),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
        abi::return_(),
    ];
    let relocations = vec![internal_branch(
        MAKE_ERROR_RESULT_SYMBOL,
        BUILD_ERROR_LOC_SYMBOL,
    )];
    finalize_vreg_helper(
        "runtime.make_error_result",
        MAKE_ERROR_RESULT_SYMBOL,
        "Pointer",
        instructions,
        relocations,
    )
}

/// `arena_insert_free(x0 = ptr, x1 = size)` — insert a chunk into the
/// address-ordered free-list and coalesce with the address-adjacent neighbor on
/// either side. `size` must already be normalized (≥16, multiple of 16) and
/// `ptr` 16-aligned; both hold for every chunk the allocator hands out and for a
/// fresh block's usable region. A `ptr` that is already a free node is a no-op
/// (allocator-03 idempotency guard), so a double-free relinks nothing. Leaf
/// function; vreg-allocated — treat all caller-saved integer registers as
/// clobbered.
pub(super) fn lower_arena_insert_free() -> CodeFunction {
    let mut vregs = Vregs::new();
    // ptr (x0) / size (x1) are read-only args; this is a leaf, so they stay
    // physical. Everything else is a vreg the allocator places.
    let cur = vregs.next();
    let prev = vregs.next();
    let t1 = vregs.next();
    let t2 = vregs.next();
    let merged = vregs.next();
    let instructions = vec![
        abi::label("entry"),
        // Walk to the insertion slot: prev = largest node < ptr (or 0),
        // cur = smallest node > ptr (or 0).
        abi::load_u64(&cur, ARENA_STATE_REGISTER, ARENA_FREE_LIST_HEAD_OFFSET),
        abi::move_immediate(&prev, "Integer", "0"),
        abi::label("insert_find"),
        abi::compare_immediate(&cur, "0"),
        abi::branch_eq("insert_slot"),
        abi::compare_registers(&cur, "x0"),
        abi::branch_hi("insert_slot"), // cur > ptr
        // Idempotency guard (allocator-03): the chunk is already a free node —
        // a double-free becomes a no-op instead of double-linking `ptr` and
        // coalescing against its own (about-to-be-rewritten) metadata.
        abi::compare_registers(&cur, "x0"),
        abi::branch_eq("insert_already_free"),
        abi::move_register(&prev, &cur),
        abi::load_u64(&cur, &cur, 0),
        abi::branch("insert_find"),
        abi::label("insert_slot"),
        // merged = merged-into-prev flag.
        abi::move_immediate(&merged, "Integer", "0"),
        abi::compare_immediate(&prev, "0"),
        abi::branch_eq("insert_check_next"),
        abi::load_u64(&t1, &prev, 8),        // prev.size
        abi::add_registers(&t2, &prev, &t1), // prev_end
        abi::compare_registers(&t2, "x0"),
        abi::branch_ne("insert_check_next"),
        // prev is address-adjacent: absorb the chunk into prev.
        abi::add_registers(&t1, &t1, "x1"),
        abi::store_u64(&t1, &prev, 8),
        abi::move_immediate(&merged, "Integer", "1"),
        abi::label("insert_check_next"),
        abi::compare_immediate(&cur, "0"),
        abi::branch_eq("insert_finish_no_next"),
        abi::compare_immediate(&merged, "0"),
        abi::branch_eq("insert_next_unmerged"),
        // Merged into prev already: does the (now larger) prev meet cur?
        abi::load_u64(&t1, &prev, 8),
        abi::add_registers(&t2, &prev, &t1),
        abi::compare_registers(&t2, &cur),
        abi::branch_ne("insert_done"),
        // Absorb cur into prev too (three-way merge).
        abi::load_u64(&t1, &cur, 8),  // cur.size
        abi::load_u64(&t2, &prev, 8), // prev.size
        abi::add_registers(&t2, &t2, &t1),
        abi::store_u64(&t2, &prev, 8),
        abi::load_u64(&t1, &cur, 0), // cur.next
        abi::store_u64(&t1, &prev, 0),
        abi::branch("insert_done"),
        abi::label("insert_next_unmerged"),
        abi::add_registers(&t2, "x0", "x1"), // chunk_end
        abi::compare_registers(&t2, &cur),
        abi::branch_ne("insert_standalone"),
        // chunk is address-adjacent to cur: new node at ptr absorbs cur.
        abi::load_u64(&t1, &cur, 8), // cur.size
        abi::add_registers(&t1, &t1, "x1"),
        abi::store_u64(&t1, "x0", 8),
        abi::load_u64(&t1, &cur, 0), // cur.next
        abi::store_u64(&t1, "x0", 0),
        abi::branch("insert_link_prev"),
        abi::label("insert_standalone"),
        abi::store_u64(&cur, "x0", 0), // ptr.next = cur
        abi::store_u64("x1", "x0", 8), // ptr.size = size
        abi::branch("insert_link_prev"),
        abi::label("insert_finish_no_next"),
        abi::compare_immediate(&merged, "0"),
        abi::branch_ne("insert_done"), // merged into prev, nothing to link
        abi::store_u64("x31", "x0", 0), // ptr.next = 0
        abi::store_u64("x1", "x0", 8), // ptr.size = size
        abi::branch("insert_link_prev"),
        abi::label("insert_link_prev"),
        abi::compare_immediate(&prev, "0"),
        abi::branch_eq("insert_set_head"),
        abi::store_u64("x0", &prev, 0), // prev.next = ptr
        abi::branch("insert_done"),
        abi::label("insert_set_head"),
        abi::store_u64("x0", ARENA_STATE_REGISTER, ARENA_FREE_LIST_HEAD_OFFSET),
        abi::label("insert_done"),
        abi::return_(),
        abi::label("insert_already_free"),
        abi::return_(),
    ];
    finalize_vreg_helper(
        "runtime.arena_insert_free",
        ARENA_INSERT_FREE_SYMBOL,
        "Nothing",
        instructions,
        Vec::new(),
    )
}

/// `arena_free(x0 = ptr, x1 = size)` — return a single compiler-sized allocation
/// to the per-arena allocator. Normalizes `size` exactly as `arena_alloc` did
/// (so the freed extent matches the live chunk), then either parks the chunk on
/// its exact-size quick bin (`size ≤ ARENA_QUICK_BIN_MAX`, O(1) push —
/// allocator-01) or coalesces it into the address-ordered list via
/// `arena_insert_free`; afterwards it entropy-scrubs the payload bytes past the
/// 16-byte FreeNode overlay just written (plan-01 §6.2, allocator-03: the
/// insert must never read PRNG-poisoned free-list metadata, and a double-free —
/// an idempotent no-op inside the insert — must never scrub a live node's
/// `{next, size}` words). Never unmaps. Vreg-allocated — treat all caller-saved
/// integer registers as clobbered.
pub(super) fn lower_arena_free() -> CodeFunction {
    let mut vregs = Vregs::new();
    let not_15 = (!(ARENA_MIN_CHUNK - 1)).to_string();
    // ptr/size are live across both helper calls; each tramples every integer
    // register (ALL_INT), so the allocator spills them and reloads before each
    // call — exactly what the old hand frame did with its PTR/SIZE slots.
    let ptr = vregs.next();
    let size = vregs.next();
    let mask = vregs.next();
    let bin_class = vregs.next();
    let bin_slot = vregs.next();
    let bin_head = vregs.next();
    let instructions = vec![
        abi::label("entry"),
        abi::move_register(&ptr, "x0"),
        // normalize size = round_up(max(size, 1), 16) — x1 is the size arg.
        abi::compare_immediate("x1", "0"),
        abi::branch_ne("arena_free_size_nonzero"),
        abi::move_immediate("x1", "Integer", "1"),
        abi::label("arena_free_size_nonzero"),
        abi::add_immediate("x1", "x1", (ARENA_MIN_CHUNK - 1) as usize),
        abi::move_immediate(&mask, "Integer", &not_15),
        abi::and_registers(&size, "x1", &mask),
        // Quick-bin park (allocator-01): a chunk ≤ ARENA_QUICK_BIN_MAX pushes
        // onto its exact-size bin head in O(1) — no list walk. The bin slot for
        // class `size/16 - 1` sits at `state + BASE + (size/16 - 1)*8`, i.e.
        // `state + (size >> 4 << 3) + (BASE - 8)`. Bin nodes reuse the FreeNode
        // {next, size} overlay, so a flush can hand them straight to
        // `arena_insert_free`.
        abi::compare_immediate(&size, &ARENA_QUICK_BIN_MAX.to_string()),
        abi::branch_hi("arena_free_large_bin"),
        abi::shift_right_immediate(&bin_class, &size, 4),
        abi::shift_left_immediate(&bin_class, &bin_class, 3),
        abi::add_registers(&bin_slot, ARENA_STATE_REGISTER, &bin_class),
        abi::load_u64(&bin_head, &bin_slot, ARENA_QUICK_BIN_BASE_OFFSET - 8),
        abi::store_u64(&bin_head, &ptr, 0),
        abi::store_u64(&size, &ptr, 8),
        abi::store_u64(&ptr, &bin_slot, ARENA_QUICK_BIN_BASE_OFFSET - 8),
        abi::branch("arena_free_scrub"),
        // A larger chunk (> ARENA_QUICK_BIN_MAX) parks on its hashed large-block
        // bin (plan-25-A): an O(1) head push keyed by `(size >> 4) & (COUNT-1)`,
        // no address-ordered walk. This is the master benchmark fix — routing
        // large frees through `arena_insert_free` grew the coalescing list
        // without bound (every large 1000-element list op frees ~40 KB), so both
        // the insert here and every later alloc walk went quadratic. Bin nodes
        // reuse the FreeNode {next, size} overlay so the flush-before-grow drain
        // can hand them straight to `arena_insert_free` when coalescing is
        // needed. The chunk is still scrubbed below.
        abi::label("arena_free_large_bin"),
        abi::shift_right_immediate(&bin_class, &size, 4),
        abi::move_immediate(&mask, "Integer", &(ARENA_LARGE_BIN_COUNT - 1).to_string()),
        abi::and_registers(&bin_class, &bin_class, &mask),
        abi::shift_left_immediate(&bin_class, &bin_class, 3),
        abi::add_registers(&bin_slot, ARENA_STATE_REGISTER, &bin_class),
        abi::load_u64(&bin_head, &bin_slot, ARENA_LARGE_BIN_BASE_OFFSET),
        abi::store_u64(&bin_head, &ptr, 0),
        abi::store_u64(&size, &ptr, 8),
        abi::store_u64(&ptr, &bin_slot, ARENA_LARGE_BIN_BASE_OFFSET),
        // … then scrub only [ptr+16, ptr+size), preserving the freshly written
        // node words. A 16-byte chunk is all node — nothing to scrub.
        abi::label("arena_free_scrub"),
        abi::compare_immediate(&size, &ARENA_MIN_CHUNK.to_string()),
        abi::branch_eq("arena_free_done"),
        abi::add_immediate("x0", &ptr, ARENA_MIN_CHUNK as usize),
        abi::subtract_immediate("x1", &size, ARENA_MIN_CHUNK as usize),
        abi::branch_link(ARENA_FILL_RANDOM_SYMBOL),
        abi::label("arena_free_done"),
        abi::return_(),
    ];
    finalize_vreg_helper(
        "runtime.arena_free",
        ARENA_FREE_SYMBOL,
        "Nothing",
        instructions,
        vec![internal_branch(ARENA_FREE_SYMBOL, ARENA_FILL_RANDOM_SYMBOL)],
    )
}

pub(super) fn lower_arena_destroy(platform: &dyn CodegenPlatform) -> Result<CodeFunction, String> {
    // Vreg-allocated (plan-00-G Phase 2): walk the block list and `munmap` each
    // block. `head` (the loop cursor) and `next` are loop-carried across the
    // `munmap` syscall, so the allocator keeps them in callee-saved registers (or
    // spills them); the syscall's own ABI registers (x0/x1 + the syscall-number
    // register) stay physical. The block address/size are passed to the syscall in
    // x0/x1, exactly where `emit_arena_unmap` expects them.
    let mut vregs = Vregs::new();
    let head = vregs.next();
    let next = vregs.next();
    let clear_cursor = vregs.next();
    let clear_limit = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::load_u64(&head, ARENA_STATE_REGISTER, 0),
        abi::label("arena_destroy_loop"),
        abi::compare_immediate(&head, "0"),
        abi::branch_eq("arena_destroy_done"),
        abi::load_u64(&next, &head, 0),
        abi::load_u64("x1", &head, 8),
        abi::move_register(abi::return_register(), &head),
    ];
    platform.emit_arena_unmap(&mut instructions)?;
    instructions.extend([
        abi::move_register(&head, &next),
        abi::branch("arena_destroy_loop"),
        abi::label("arena_destroy_done"),
        // Leave the arena fully inert (allocator-04): clear the free-list head
        // alongside the block-list head — it points into the just-unmapped
        // blocks, and a stale head would turn any post-destroy allocation into
        // a use-after-free walk. The quick bins (allocator-01) point into the
        // same unmapped blocks, so clear them too.
        abi::store_u64("x31", ARENA_STATE_REGISTER, 0),
        abi::store_u64("x31", ARENA_STATE_REGISTER, ARENA_FREE_LIST_HEAD_OFFSET),
        abi::add_immediate(
            &clear_cursor,
            ARENA_STATE_REGISTER,
            ARENA_QUICK_BIN_BASE_OFFSET,
        ),
        abi::add_immediate(&clear_limit, ARENA_STATE_REGISTER, ARENA_STATE_SIZE),
        abi::label("arena_destroy_bins"),
        abi::store_u64("x31", &clear_cursor, 0),
        abi::add_immediate(&clear_cursor, &clear_cursor, 8),
        abi::compare_registers(&clear_cursor, &clear_limit),
        abi::branch_lo("arena_destroy_bins"),
        abi::return_(),
    ]);
    Ok(finalize_vreg_helper(
        "runtime.arena_destroy",
        ARENA_DESTROY_SYMBOL,
        "Nothing",
        instructions,
        Vec::new(),
    ))
}

/// Shared process teardown. Reads the main arena-state address from the writable
/// global, clears the global (so a second entry — e.g. a signal arriving during
/// normal cleanup — becomes a no-op), pins it in `x19`, then conditionally
/// restores the terminal and frees the arena. Both underlying helpers are
/// idempotent (`term::off` gates on its `active` flag; `arena_destroy` clears the
/// block-list head), so the guard is belt-and-suspenders. Preserves `x19`/`x30`
/// for its callers (the entry exit path relies on `x19` afterwards).
pub(super) fn lower_shutdown(
    auto_term_off: bool,
    skip_arena_destroy: bool,
    drain_stdout: bool,
) -> CodeFunction {
    // Vreg-allocated (plan-00-G Phase 2). The allocator builds the frame and saves
    // the link register (there are `bl`s). `x19` (arena_base) is reserved from
    // allocation, but this function deliberately *repoints* it at the main arena to
    // run the teardown helpers, so it saves the caller's `x19` into a vreg (spilled
    // across the calls) and restores it before returning — the entry exit path
    // relies on `x19` afterwards.
    let done = "shutdown_done";
    let mut vregs = Vregs::new();
    let saved_arena = vregs.next();
    let global = vregs.next();
    let arena = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&saved_arena, ARENA_STATE_REGISTER),
    ];
    let mut relocations = Vec::new();
    push_symbol_address(
        SHUTDOWN_SYMBOL,
        MAIN_ARENA_GLOBAL_SYMBOL,
        &global,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::load_u64(&arena, &global, 0),
        abi::store_u64("x31", &global, 0),
        abi::compare_immediate(&arena, "0"),
        abi::branch_eq(done),
        abi::move_register(ARENA_STATE_REGISTER, &arena),
    ]);
    // Drain the opt-in stdout buffer at exit (plan-14-A §4.3 hook 1) before the
    // arena — whose block backs the buffer — is freed. `x19` now points at the
    // main arena, so the drain reads the right OUT_* words. A no-op when buffering
    // is off; idempotent with a second entry (a signal during normal cleanup).
    if drain_stdout {
        instructions.push(abi::branch_link(STDOUT_DRAIN_SYMBOL));
        relocations.push(internal_branch(SHUTDOWN_SYMBOL, STDOUT_DRAIN_SYMBOL));
    }
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
        abi::move_register(ARENA_STATE_REGISTER, &saved_arena),
        abi::return_(),
    ]);
    finalize_vreg_helper(
        "runtime.shutdown",
        SHUTDOWN_SYMBOL,
        "Nothing",
        instructions,
        relocations,
    )
}

/// `void handler(int signo)` for SIGINT/SIGTERM: run the shared teardown, then
/// `_exit(128 + signo)`. It never returns, so it need not preserve the
/// interrupted context; it locates the arena through `_mfb_shutdown`'s global
/// read rather than the interrupted `x19`. The 16-byte frame keeps `sp` aligned
/// across the `bl`s (Darwin requires this) and parks `signo` across the call.
pub(super) fn lower_signal_handler(platform: &dyn CodegenPlatform) -> Result<CodeFunction, String> {
    // Vreg-allocated (plan-00-G Phase 2). `signo` (x0) is parked across the
    // `bl _mfb_shutdown` in a vreg the allocator spills; the allocator + frame
    // builder provide the aligned frame and link-register save. The function never
    // returns (it tail-exits), so nothing needs preserving across it.
    let mut vregs = Vregs::new();
    let signo = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&signo, abi::return_register()),
        abi::branch_link(SHUTDOWN_SYMBOL),
    ];
    let mut relocations = vec![internal_branch(SIGNAL_HANDLER_SYMBOL, SHUTDOWN_SYMBOL)];
    instructions.push(abi::add_immediate(abi::return_register(), &signo, 128));
    platform.emit_program_exit(SIGNAL_HANDLER_SYMBOL, &mut instructions, &mut relocations)?;
    Ok(finalize_vreg_helper(
        "runtime.signal_handler",
        SIGNAL_HANDLER_SYMBOL,
        "Nothing",
        instructions,
        relocations,
    ))
}

/// Append the PCG64 LCG step `state = state * MULT + INC` operating on the
/// 128-bit state held in (`lo`, `hi`). The limbs are read at the start and
/// rewritten in place; `x11`-`x16` are used as scratch (caller-saved, so these
/// leaf helpers need not preserve them).
/// A monotonic virtual-register name generator for a hand-written vreg helper
/// (plan-00-G Phase 2): each call yields a fresh `%vN` the shared allocator
/// colors. Lets the PCG64 / arena helpers be written in target-neutral MIR (no
/// fixed `x9`/`x13`…) so register placement is a per-ISA backend job.
pub(super) struct Vregs(usize);

impl Vregs {
    pub(super) fn new() -> Self {
        Vregs(0)
    }

    pub(super) fn next(&mut self) -> String {
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
    // Copy the `x0` (arena ptr) and `x1` (seed) ABI args into vregs that survive
    // the `emit_pcg_step` `mul`/`umulh` — on x86 those clobber the registers
    // `x0`/`x1` map to (`rax`/`rdx`), which would otherwise destroy the seed and
    // the store base mid-dance.
    let ptr = vregs.next();
    let seed = vregs.next();
    let lo = vregs.next();
    let hi = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&ptr, "x0"),
        abi::move_register(&seed, "x1"),
        abi::move_immediate(&lo, "Integer", "0"),
        abi::move_immediate(&hi, "Integer", "0"),
    ];
    emit_pcg_step(&mut instructions, &mut vregs, &lo, &hi);
    let carry = vregs.next();
    instructions.extend([
        // state += seed, carry as an explicit value (plan-00-G §4).
        abi::add_carry(&lo, &carry, &lo, &seed, "xzr"),
        abi::add_carry(&hi, "xzr", &hi, "xzr", &carry),
    ]);
    emit_pcg_step(&mut instructions, &mut vregs, &lo, &hi);
    instructions.extend([
        abi::store_u64(&lo, &ptr, lo_offset),
        abi::store_u64(&hi, &ptr, hi_offset),
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
    // same vregs the allocator keeps in registers across the back-edge. The `x0`
    // (ptr) and `x1` (word count) ABI args become loop-carried vregs too: copy
    // them in at entry so the allocator places them in callee-saved registers.
    // On AArch64 they could stay physical (a leaf clobbers nothing), but x86's
    // `mul`/`umulh` in the PCG step clobber the registers `x0`/`x1` map to
    // (`rax`/`rdx`), so a physical counter would be destroyed mid-loop.
    let ptr = vregs.next();
    let count = vregs.next();
    let lo = vregs.next();
    let hi = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&ptr, "x0"),
        abi::move_register(&count, "x1"),
        // word count = (len + 7) >> 3
        abi::add_immediate(&count, &count, 7),
        abi::shift_right_immediate(&count, &count, 3),
        abi::compare_immediate(&count, "0"),
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
        abi::store_u64(&word, &ptr, 0),
        abi::add_immediate(&ptr, &ptr, 8),
        abi::subtract_immediate(&count, &count, 1),
        abi::compare_immediate(&count, "0"),
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
