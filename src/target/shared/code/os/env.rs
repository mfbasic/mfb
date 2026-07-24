use super::*;
use crate::target::shared::abi;
use std::collections::HashMap;

/// Whether `module` uses any `os::` helper that must serialize on the env/pwd
/// lock, so the writable mutex global is emitted (see `OS_ENV_LOCK_SYMBOL`).
pub(crate) fn module_uses_env_lock(module: &NirModule) -> bool {
    OS_ENV_LOCK_CALLS
        .iter()
        .any(|call| module_uses_call(module, call))
}

/// The statically-initialized bytes of the env/pwd mutex for `target`, as a hex
/// string (two chars per byte), so no runtime initializer call is needed. Linux
/// `PTHREAD_MUTEX_INITIALIZER` is an all-zero `pthread_mutex_t`; macOS is
/// `{ _PTHREAD_MUTEX_SIG_init, {0} }`, i.e. the `0x32AAABA7` signature in the first
/// 8-byte `__sig` word with the rest zero, which libc lazily first-use-initializes
/// on the first `pthread_mutex_lock` (exactly as a static `PTHREAD_MUTEX_INITIALIZER`
/// does).
pub(crate) fn os_env_lock_init_hex(family: PlatformFamily) -> String {
    let mut bytes = [0u8; OS_ENV_LOCK_SIZE];
    match family {
        // `_PTHREAD_MUTEX_SIG_init` = 0x32AAABA7, little-endian in the `long __sig`.
        PlatformFamily::MacOS => bytes[0..4].copy_from_slice(&0x32AA_ABA7u32.to_le_bytes()),
        // Linux `PTHREAD_MUTEX_INITIALIZER` is an all-zero `pthread_mutex_t`.
        PlatformFamily::Linux => {}
        // The Windows env/pwd lock is an SRWLOCK, whose valid initial value
        // (`SRWLOCK_INIT`) is all zero — exactly like PTHREAD_MUTEX_INITIALIZER; the
        // Acquire/Release SRW calls the lock/unlock arms emit need no separate init.
        PlatformFamily::Windows => {}
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Acquire the env/pwd lock: `pthread_mutex_lock(&_mfb_rt_os_env_lock)`. Emitted at
/// helper entry, after incoming `String*` arguments have been saved into vregs (the
/// call clobbers all caller-saved registers).
pub(super) fn emit_env_lock(ctx: &mut EmitCtx) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    push_symbol_address(
        symbol,
        OS_ENV_LOCK_SYMBOL,
        abi::ARG[0],
        ctx.instructions,
        ctx.relocations,
    );
    platform.emit_libc_call(
        "pthread_mutex_lock",
        symbol,
        platform_imports,
        ctx.instructions,
        ctx.relocations,
    )
}

/// Release the env/pwd lock and return. The four result registers (tag/value/
/// message/source) are preserved across the `pthread_mutex_unlock` call — which
/// clobbers all caller-saved registers — through vregs the allocator keeps live.
/// Every helper routes all exit paths through a single `done` label so exactly one
/// balanced unlock runs per (matched) lock.
pub(super) fn emit_env_unlock_return(ctx: &mut EmitCtx, vregs: &mut Vregs) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    let saved_tag = vregs.next();
    let saved_value = vregs.next();
    let saved_message = vregs.next();
    let saved_source = vregs.next();
    ctx.instructions.extend([
        abi::move_register(&saved_tag, RESULT_TAG_REGISTER),
        abi::move_register(&saved_value, RESULT_VALUE_REGISTER),
        abi::move_register(&saved_message, RESULT_ERROR_MESSAGE_REGISTER),
        abi::move_register(&saved_source, RESULT_ERROR_SOURCE_REGISTER),
    ]);
    push_symbol_address(
        symbol,
        OS_ENV_LOCK_SYMBOL,
        abi::ARG[0],
        ctx.instructions,
        ctx.relocations,
    );
    platform.emit_libc_call(
        "pthread_mutex_unlock",
        symbol,
        platform_imports,
        ctx.instructions,
        ctx.relocations,
    )?;
    ctx.instructions.extend([
        abi::move_register(RESULT_TAG_REGISTER, &saved_tag),
        abi::move_register(RESULT_VALUE_REGISTER, &saved_value),
        abi::move_register(RESULT_ERROR_MESSAGE_REGISTER, &saved_message),
        abi::move_register(RESULT_ERROR_SOURCE_REGISTER, &saved_source),
        abi::return_(),
    ]);
    Ok(())
}

pub(super) fn lower_get_env(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    with_fallback: bool,
) -> HelperResult {
    let not_found = format!("{symbol}_not_found");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let mut vregs = Vregs::new();
    let name = vregs.next();
    let fallback = vregs.next();
    let cname = vregs.next();
    let value = vregs.next();
    let mut instructions = vec![abi::label("entry"), abi::move_register(&name, abi::ARG[0])];
    if with_fallback {
        instructions.push(abi::move_register(&fallback, abi::ARG[1]));
    }
    let mut relocations = Vec::new();
    // Serialize the whole `getenv` + marshal-into-arena against a concurrent
    // `os::setEnv` relocating/freeing `environ` (bug-64).
    emit_env_lock(&mut EmitCtx {
        symbol,
        platform_imports,
        platform,
        instructions: &mut instructions,
        relocations: &mut relocations,
    })?;
    marshal_cstring(
        symbol,
        &name,
        &cname,
        &alloc_error,
        &format!("{symbol}_name"),
        &mut vregs,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::move_register(abi::ARG[0], &cname));
    platform.emit_libc_call(
        "getenv",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::move_register(&value, abi::return_register()),
        abi::compare_immediate(&value, "0"),
        abi::branch_eq(&not_found),
    ]);
    build_string_from_cstr(
        symbol,
        &value,
        &alloc_error,
        &format!("{symbol}_found"),
        &mut vregs,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&not_found)]);
    if with_fallback {
        // Return a fresh owned copy of `fallback` (by its stored length, so an
        // embedded NUL is preserved).
        let flen = vregs.next();
        let alloc_ok = format!("{symbol}_fb_ok");
        let copy_loop = format!("{symbol}_fb_copy_loop");
        let copy_done = format!("{symbol}_fb_copy_done");
        let block = vregs.next();
        let src = vregs.next();
        let dst = vregs.next();
        let index = vregs.next();
        let byte = vregs.next();
        instructions.extend([
            abi::load_u64(&flen, &fallback, 0),
            abi::add_immediate(abi::return_register(), &flen, 9),
            abi::move_immediate(abi::ARG[1], "Integer", "8"),
            abi::branch_link(ARENA_ALLOC_SYMBOL),
        ]);
        alloc_reloc(symbol, &mut relocations);
        instructions.extend([
            abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
            abi::branch_ne(&alloc_error),
            abi::label(&alloc_ok),
            abi::move_register(&block, abi::RET[1]),
            abi::load_u64(&flen, &fallback, 0),
            abi::store_u64(&flen, &block, 0),
            abi::add_immediate(&src, &fallback, 8),
            abi::add_immediate(&dst, &block, 8),
            abi::move_immediate(&index, "Integer", "0"),
            abi::label(&copy_loop),
            abi::compare_registers(&index, &flen),
            abi::branch_eq(&copy_done),
            abi::load_u8(&byte, &src, 0),
            abi::store_u8(&byte, &dst, 0),
            abi::add_immediate(&src, &src, 1),
            abi::add_immediate(&dst, &dst, 1),
            abi::add_immediate(&index, &index, 1),
            abi::branch(&copy_loop),
            abi::label(&copy_done),
            abi::store_u8(abi::ZERO, &dst, 0),
            abi::move_register(RESULT_VALUE_REGISTER, &block),
            abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
            abi::branch(&done),
        ]);
    } else {
        instructions.extend([
            abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_NOT_FOUND_CODE),
            abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
        ]);
        push_error_message_address(
            symbol,
            ERR_NOT_FOUND_SYMBOL,
            &mut instructions,
            &mut relocations,
        );
        instructions.push(abi::branch(&done));
    }
    instructions.push(abi::label(&alloc_error));
    push_alloc_error(symbol, &mut instructions, &mut relocations);
    instructions.push(abi::label(&done));
    emit_env_unlock_return(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        &mut vregs,
    )?;

    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

pub(super) fn lower_has_env(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let present = format!("{symbol}_present");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let mut vregs = Vregs::new();
    let name = vregs.next();
    let cname = vregs.next();
    let mut instructions = vec![abi::label("entry"), abi::move_register(&name, abi::ARG[0])];
    let mut relocations = Vec::new();
    // Serialize the `getenv` probe against a concurrent `os::setEnv` relocating
    // `environ` (bug-64).
    emit_env_lock(&mut EmitCtx {
        symbol,
        platform_imports,
        platform,
        instructions: &mut instructions,
        relocations: &mut relocations,
    })?;
    marshal_cstring(
        symbol,
        &name,
        &cname,
        &alloc_error,
        &format!("{symbol}_name"),
        &mut vregs,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::move_register(abi::ARG[0], &cname));
    platform.emit_libc_call(
        "getenv",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&present),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&present),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&alloc_error),
    ]);
    push_alloc_error(symbol, &mut instructions, &mut relocations);
    instructions.push(abi::label(&done));
    emit_env_unlock_return(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        &mut vregs,
    )?;

    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

pub(super) fn lower_set_env(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let ok = format!("{symbol}_ok");
    let fail = format!("{symbol}_fail");
    let oom = format!("{symbol}_oom");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let mut vregs = Vregs::new();
    let name = vregs.next();
    let value = vregs.next();
    let cname = vregs.next();
    let cvalue = vregs.next();
    let errno = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&name, abi::ARG[0]),
        abi::move_register(&value, abi::ARG[1]),
    ];
    let mut relocations = Vec::new();
    // Hold the lock across `setenv` so a concurrent env reader on another thread
    // never observes a half-relocated `environ` (bug-64).
    emit_env_lock(&mut EmitCtx {
        symbol,
        platform_imports,
        platform,
        instructions: &mut instructions,
        relocations: &mut relocations,
    })?;
    marshal_cstring(
        symbol,
        &name,
        &cname,
        &alloc_error,
        &format!("{symbol}_name"),
        &mut vregs,
        &mut instructions,
        &mut relocations,
    );
    marshal_cstring(
        symbol,
        &value,
        &cvalue,
        &alloc_error,
        &format!("{symbol}_value"),
        &mut vregs,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::move_register(abi::ARG[0], &cname),
        abi::move_register(abi::ARG[1], &cvalue),
        abi::move_immediate(abi::ARG[2], "Integer", "1"),
    ]);
    platform.emit_libc_call(
        "setenv",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&fail),
        abi::label(&ok),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&fail),
    ]);
    // Distinguish ENOMEM (→ ErrOutOfMemory) from every other errno (→
    // ErrInvalidArgument: empty name, or a name containing '=').
    platform.emit_errno(
        symbol,
        &errno,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(&errno, ERRNO_ENOMEM),
        abi::branch_eq(&oom),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INVALID_ARGUMENT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INVALID_ARGUMENT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&oom)]);
    push_alloc_error(symbol, &mut instructions, &mut relocations);
    instructions.extend([abi::branch(&done), abi::label(&alloc_error)]);
    push_alloc_error(symbol, &mut instructions, &mut relocations);
    instructions.push(abi::label(&done));
    emit_env_unlock_return(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        &mut vregs,
    )?;

    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

pub(super) fn lower_unset_env(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let mut vregs = Vregs::new();
    let name = vregs.next();
    let cname = vregs.next();
    let mut instructions = vec![abi::label("entry"), abi::move_register(&name, abi::ARG[0])];
    let mut relocations = Vec::new();
    // Hold the lock across `unsetenv` so a concurrent env reader on another thread
    // never observes a half-relocated `environ` (bug-64).
    emit_env_lock(&mut EmitCtx {
        symbol,
        platform_imports,
        platform,
        instructions: &mut instructions,
        relocations: &mut relocations,
    })?;
    marshal_cstring(
        symbol,
        &name,
        &cname,
        &alloc_error,
        &format!("{symbol}_name"),
        &mut vregs,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::move_register(abi::ARG[0], &cname));
    platform.emit_libc_call(
        "unsetenv",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    // `unsetenv` is a no-op for an absent variable; treat any return as success.
    instructions.extend([
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&alloc_error),
    ]);
    push_alloc_error(symbol, &mut instructions, &mut relocations);
    instructions.push(abi::label(&done));
    emit_env_unlock_return(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        &mut vregs,
    )?;

    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

/// `os::environ()` — walk `char **environ` twice: pass 1 counts entries and the
/// total key+value data bytes (the `=` separator is dropped); pass 2 allocates
/// the `Map OF String` (header + entry table + data + lazy bucket region) and
/// fills it. Each `KEY=VALUE` splits at the first `=`.
pub(super) fn lower_environ(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let count_loop = format!("{symbol}_count_loop");
    let count_done = format!("{symbol}_count_done");
    let count_scan = format!("{symbol}_count_scan");
    let count_scan_done = format!("{symbol}_count_scan_done");
    let count_data = format!("{symbol}_count_data");
    let count_next = format!("{symbol}_count_next");
    let alloc_ok = format!("{symbol}_alloc_ok");
    let fill_loop = format!("{symbol}_fill_loop");
    let fill_done = format!("{symbol}_fill_done");
    let key_scan = format!("{symbol}_key_scan");
    let key_scan_done = format!("{symbol}_key_scan_done");
    let key_copy_loop = format!("{symbol}_key_copy_loop");
    let key_copy_done = format!("{symbol}_key_copy_done");
    let val_len_loop = format!("{symbol}_val_len_loop");
    let val_store = format!("{symbol}_val_store");
    let val_copy_loop = format!("{symbol}_val_copy_loop");
    let val_copy_done = format!("{symbol}_val_copy_done");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let mut vregs = Vregs::new();
    let envp = vregs.next();
    let cursor = vregs.next();
    let entry_ptr = vregs.next();
    let count = vregs.next();
    let data_bytes = vregs.next();
    let scan = vregs.next();
    let byte = vregs.next();
    let collection = vregs.next();
    let entry_cursor = vregs.next();
    let data_cursor = vregs.next();
    let data_offset = vregs.next();
    let scratch = vregs.next();
    let key_len = vregs.next();
    let val_ptr = vregs.next();
    let val_len = vregs.next();
    let src = vregs.next();
    let eq_flag = vregs.next();

    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    // Hold the lock across the whole two-pass `environ` walk and the marshal into
    // the arena `Map`, so a concurrent `os::setEnv` cannot relocate/free the array
    // or its strings mid-walk (bug-64).
    emit_env_lock(&mut EmitCtx {
        symbol,
        platform_imports,
        platform,
        instructions: &mut instructions,
        relocations: &mut relocations,
    })?;
    platform.emit_environ_pointer(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::move_register(&envp, abi::return_register()),
        // Pass 1: count entries and data bytes.
        abi::move_register(&cursor, &envp),
        abi::move_immediate(&count, "Integer", "0"),
        abi::move_immediate(&data_bytes, "Integer", "0"),
        abi::label(&count_loop),
        abi::load_u64(&entry_ptr, &cursor, 0),
        abi::compare_immediate(&entry_ptr, "0"),
        abi::branch_eq(&count_done),
        // Scan "KEY=VALUE": every byte before the NUL contributes to data, minus
        // exactly the FIRST '=' separator. A '=' inside the value (e.g.
        // `LS_COLORS`) is kept — pass 2 splits only at the first '=', so pass 1
        // must undercount by exactly one to keep the data region correctly sized.
        abi::move_register(&scan, &entry_ptr),
        abi::move_immediate(&eq_flag, "Integer", "0"),
        abi::label(&count_scan),
        abi::load_u8(&byte, &scan, 0),
        abi::compare_immediate(&byte, "0"),
        abi::branch_eq(&count_scan_done),
        abi::compare_immediate(&byte, "61"), // '='
        abi::branch_ne(&count_data),
        abi::compare_immediate(&eq_flag, "0"),
        abi::branch_ne(&count_data), // a later '=' is value data
        abi::move_immediate(&eq_flag, "Integer", "1"), // first '=' is the separator
        abi::branch(&count_next),
        abi::label(&count_data),
        abi::add_immediate(&data_bytes, &data_bytes, 1),
        abi::label(&count_next),
        abi::add_immediate(&scan, &scan, 1),
        abi::branch(&count_scan),
        abi::label(&count_scan_done),
        abi::add_immediate(&count, &count, 1),
        abi::add_immediate(&cursor, &cursor, 8),
        abi::branch(&count_loop),
        abi::label(&count_done),
        // size = HEADER + count*ENTRY_SIZE + data_bytes + count*(2*MAP_BUCKET_SIZE)
        abi::move_immediate(
            &scratch,
            "Integer",
            &(COLLECTION_ENTRY_SIZE + 2 * MAP_BUCKET_SIZE).to_string(),
        ),
        abi::multiply_registers(&scratch, &count, &scratch),
        abi::add_registers(&scratch, &scratch, &data_bytes),
        abi::add_immediate(abi::return_register(), &scratch, COLLECTION_HEADER_SIZE),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    alloc_reloc(symbol, &mut relocations);
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_ne(&alloc_error),
        abi::label(&alloc_ok),
        abi::move_register(&collection, abi::RET[1]),
        // Header.
        abi::move_immediate(&scratch, "Byte", &COLLECTION_KIND_MAP.to_string()),
        abi::store_u8(&scratch, &collection, COLLECTION_OFFSET_KIND),
        abi::move_immediate(&scratch, "Byte", &COLLECTION_TYPE_STRING.to_string()),
        abi::store_u8(&scratch, &collection, COLLECTION_OFFSET_KEY_TYPE),
        abi::move_immediate(&scratch, "Byte", &COLLECTION_TYPE_STRING.to_string()),
        abi::store_u8(&scratch, &collection, COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate(&scratch, "Byte", "1"),
        abi::store_u8(&scratch, &collection, COLLECTION_OFFSET_FLAGS_VERSION),
        abi::move_immediate(&scratch, "Byte", "0"),
        abi::store_u8(&scratch, &collection, COLLECTION_OFFSET_BUCKETS_READY),
        abi::store_u64(&count, &collection, COLLECTION_OFFSET_COUNT),
        abi::store_u64(&count, &collection, COLLECTION_OFFSET_CAPACITY),
        abi::store_u64(&data_bytes, &collection, COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64(&data_bytes, &collection, COLLECTION_OFFSET_DATA_CAPACITY),
        // entry_cursor = base + HEADER; data_cursor = entry table end.
        abi::add_immediate(&entry_cursor, &collection, COLLECTION_HEADER_SIZE),
        abi::move_immediate(&scratch, "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers(&scratch, &count, &scratch),
        abi::add_registers(&data_cursor, &entry_cursor, &scratch),
        abi::move_immediate(&data_offset, "Integer", "0"),
        // Pass 2: fill.
        abi::move_register(&cursor, &envp),
        abi::label(&fill_loop),
        abi::load_u64(&entry_ptr, &cursor, 0),
        abi::compare_immediate(&entry_ptr, "0"),
        abi::branch_eq(&fill_done),
        // key_len = index of first '=' (or full length if none).
        abi::move_register(&scan, &entry_ptr),
        abi::move_immediate(&key_len, "Integer", "0"),
        abi::label(&key_scan),
        abi::load_u8(&byte, &scan, 0),
        abi::compare_immediate(&byte, "0"),
        abi::branch_eq(&key_scan_done),
        abi::compare_immediate(&byte, "61"), // '='
        abi::branch_eq(&key_scan_done),
        abi::add_immediate(&key_len, &key_len, 1),
        abi::add_immediate(&scan, &scan, 1),
        abi::branch(&key_scan),
        abi::label(&key_scan_done),
        // Entry: FLAGS=used, KEY_OFFSET=data_offset, KEY_LENGTH=key_len.
        abi::move_immediate(&scratch, "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
        abi::store_u8(&scratch, &entry_cursor, COLLECTION_ENTRY_OFFSET_FLAGS),
        abi::store_u64(
            &data_offset,
            &entry_cursor,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ),
        abi::store_u64(&key_len, &entry_cursor, COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
        // Copy key bytes [entry_ptr .. entry_ptr+key_len) into the data region.
        abi::move_register(&src, &entry_ptr),
        abi::move_immediate(&scratch, "Integer", "0"),
        abi::label(&key_copy_loop),
        abi::compare_registers(&scratch, &key_len),
        abi::branch_eq(&key_copy_done),
        abi::load_u8(&byte, &src, 0),
        abi::store_u8(&byte, &data_cursor, 0),
        abi::add_immediate(&src, &src, 1),
        abi::add_immediate(&data_cursor, &data_cursor, 1),
        abi::add_immediate(&scratch, &scratch, 1),
        abi::branch(&key_copy_loop),
        abi::label(&key_copy_done),
        abi::add_registers(&data_offset, &data_offset, &key_len),
        // val_ptr points at the '=' (or the NUL, for a key with no '=').
        abi::add_registers(&val_ptr, &entry_ptr, &key_len),
        abi::move_immediate(&val_len, "Integer", "0"),
        abi::load_u8(&byte, &val_ptr, 0),
        abi::compare_immediate(&byte, "0"),
        abi::branch_eq(&val_store), // no '=': empty value (val_ptr at NUL, len 0)
        abi::add_immediate(&val_ptr, &val_ptr, 1), // skip '='
        // val_len = strlen(val_ptr).
        abi::move_register(&scan, &val_ptr),
        abi::label(&val_len_loop),
        abi::load_u8(&byte, &scan, 0),
        abi::compare_immediate(&byte, "0"),
        abi::branch_eq(&val_store),
        abi::add_immediate(&val_len, &val_len, 1),
        abi::add_immediate(&scan, &scan, 1),
        abi::branch(&val_len_loop),
        abi::label(&val_store),
        // VALUE_OFFSET / VALUE_LENGTH.
        abi::store_u64(
            &data_offset,
            &entry_cursor,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ),
        abi::store_u64(
            &val_len,
            &entry_cursor,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ),
        abi::move_register(&src, &val_ptr),
        abi::move_immediate(&scratch, "Integer", "0"),
        abi::label(&val_copy_loop),
        abi::compare_registers(&scratch, &val_len),
        abi::branch_eq(&val_copy_done),
        abi::load_u8(&byte, &src, 0),
        abi::store_u8(&byte, &data_cursor, 0),
        abi::add_immediate(&src, &src, 1),
        abi::add_immediate(&data_cursor, &data_cursor, 1),
        abi::add_immediate(&scratch, &scratch, 1),
        abi::branch(&val_copy_loop),
        abi::label(&val_copy_done),
        abi::add_registers(&data_offset, &data_offset, &val_len),
        abi::add_immediate(&entry_cursor, &entry_cursor, COLLECTION_ENTRY_SIZE),
        abi::add_immediate(&cursor, &cursor, 8),
        abi::branch(&fill_loop),
        abi::label(&fill_done),
        abi::move_register(RESULT_VALUE_REGISTER, &collection),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&alloc_error),
    ]);
    push_alloc_error(symbol, &mut instructions, &mut relocations);
    instructions.push(abi::label(&done));
    emit_env_unlock_return(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        &mut vregs,
    )?;

    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}
