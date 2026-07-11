use super::*;

/// Narrow a C `int` result in the return register to its true signed 64-bit
/// value. Required before any signed relational compare (`branch_lt`): none of
/// the ABIs we target guarantee the upper 32 bits of an `int` return — AAPCS64
/// and the Darwin arm64 ABI leave `x0[63:32]` unspecified, and x86-64 SysV
/// leaves `rax[63:32]` undefined. When a libc leaves those bits clear, a `-1`
/// (EIO/EBADF/ENOSPC) reads as `+4294967295`, `branch_lt` is not taken, and an
/// `fsync`/`close` durability failure is silently swallowed (bug-04, bug-44).
///
/// This is the single owner of the invariant: it lives at the comparison seam
/// so a newly added `int`-returning platform wrapper cannot reintroduce the
/// class. `sign_extend_word` lowers per-backend (`sxtw` on aarch64, `sext.w`
/// on riscv64, `movsxd` on x86-64); on riscv64's lp64d ABI the extension is
/// already guaranteed, making the op a semantic no-op there — kept for
/// uniformity so the next backend need not remember it.
fn normalize_c_int_result(instructions: &mut Vec<CodeInstruction>) {
    instructions.push(abi::sign_extend_word(
        abi::return_register(),
        abi::return_register(),
    ));
}

pub(super) fn lower_fs_create_temp_file_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<
    (
        CodeFrame,
        Vec<CodeInstruction>,
        Vec<CodeRelocation>,
        Vec<CodeStackSlot>,
    ),
    String,
> {
    // Vreg-allocated (plan-00-G Phase 2). The 16-byte random buffer is an explicit
    // on-stack local at sp+0 (`finalize_vreg_body_with_locals`); dir/path/cursor/fd
    // (held across the random-bytes / open / record-alloc calls) are spilled vregs.
    const RANDOM_OFFSET: usize = 0;
    const RANDOM_BUF_SIZE: usize = 16;
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

    let mut vregs = Vregs::new();
    let dir = vregs.next();
    let path = vregs.next();
    let cursor = vregs.next();
    let fd = vregs.next();
    let len0 = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&dir, abi::return_register()),
        abi::load_u64(&len0, &dir, 0),
        abi::compare_immediate(&len0, "0"),
        abi::branch_eq(&invalid),
        abi::add_immediate(abi::return_register(), &len0, UUID_FILE_EXTRA),
        abi::move_immediate(abi::ARG[1], "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ];
    let mut relocations = vec![CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    }];
    let dir_len = vregs.next();
    let src = vregs.next();
    let index = vregs.next();
    let byte = vregs.next();
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::move_register(&path, abi::RET[1]),
        abi::move_register(&cursor, &path),
        abi::load_u64(&dir_len, &dir, 0),
        abi::add_immediate(&src, &dir, 8),
        abi::move_immediate(&index, "Integer", "0"),
        abi::label(&copy_dir),
        abi::compare_registers(&index, &dir_len),
        abi::branch_eq(&copy_done),
        abi::load_u8(&byte, &src, 0),
        abi::compare_immediate(&byte, "0"),
        abi::branch_eq(&invalid),
        abi::store_u8(&byte, &cursor, 0),
        abi::add_immediate(&src, &src, 1),
        abi::add_immediate(&cursor, &cursor, 1),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&copy_dir),
        abi::label(&copy_done),
    ]);
    for b in b"/mfb-" {
        instructions.extend([
            abi::move_immediate(&byte, "Byte", &b.to_string()),
            abi::store_u8(&byte, &cursor, 0),
            abi::add_immediate(&cursor, &cursor, 1),
        ]);
    }
    instructions.extend([
        abi::add_immediate(abi::return_register(), abi::stack_pointer(), RANDOM_OFFSET),
        abi::move_immediate(abi::ARG[1], "Integer", "16"),
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
    ]);
    emit_uuid_v4_to_path(
        symbol,
        &mut instructions,
        &mut vregs,
        RANDOM_OFFSET,
        &cursor,
    );
    for b in b".tmp" {
        instructions.extend([
            abi::move_immediate(&byte, "Byte", &b.to_string()),
            abi::store_u8(&byte, &cursor, 0),
            abi::add_immediate(&cursor, &cursor, 1),
        ]);
    }
    instructions.extend([
        abi::store_u8(abi::ZERO, &cursor, 0),
        abi::move_register(abi::return_register(), &path),
        abi::move_immediate(abi::ARG[1], "Integer", temp_file_open_flags(platform.target())),
        abi::move_immediate(abi::ARG[2], "Integer", "384"),
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
        abi::move_register(&fd, abi::return_register()),
        abi::move_immediate(abi::return_register(), "Integer", RESOURCE_RECORD_SIZE),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
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
        // The File-record alloc failed after `open` created the temp file: close the
        // fd before reporting OOM so the error path does not leak the OS fd
        // (bug-63). `fd` is a spilled vreg, surviving the failed alloc and this
        // close. (The temp file itself is the caller's to clean up, matching the
        // success contract of createTempFile.)
        abi::move_register(abi::return_register(), &fd),
    ]);
    platform.emit_close_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::branch(&alloc_error),
        abi::label(&file_alloc_ok),
        abi::store_u64(&fd, abi::RET[1], FILE_OFFSET_FD),
        abi::store_u64(abi::ZERO, abi::RET[1], FILE_OFFSET_CLOSED),
        abi::store_u64(abi::ZERO, abi::RET[1], FILE_OFFSET_STATE),
        // Opt-in per-File output buffer (plan-14-B): a fresh handle is unbuffered.
        // Arena memory is poisoned, so zero the buffer fields explicitly.
        abi::store_u64(abi::ZERO, abi::RET[1], FILE_OFFSET_BUF_PTR),
        abi::store_u64(abi::ZERO, abi::RET[1], FILE_OFFSET_BUF_FILLED),
        abi::store_u64(abi::ZERO, abi::RET[1], FILE_OFFSET_BUF_ENABLED),
        // Transparent read buffer (plan-14-C): empty cache at the fd's position.
        abi::store_u64(abi::ZERO, abi::RET[1], FILE_OFFSET_READ_PTR),
        abi::store_u64(abi::ZERO, abi::RET[1], FILE_OFFSET_READ_POS),
        abi::store_u64(abi::ZERO, abi::RET[1], FILE_OFFSET_READ_FILL),
        abi::store_u64(abi::ZERO, abi::RET[1], FILE_OFFSET_READ_AT_EOF),
        abi::move_register(RESULT_VALUE_REGISTER, abi::RET[1]),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&open_error),
    ]);
    let errno_reg = vregs.next();
    platform.emit_errno(
        symbol,
        &errno_reg,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_errno_error_mapping(symbol, &errno_reg, &mut instructions, &mut relocations, &done);
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
    let (frame, stack_slots) =
        finalize_vreg_body_with_locals(&mut instructions, &[], RANDOM_BUF_SIZE);
    Ok((frame, instructions, relocations, stack_slots))
}

fn temp_file_open_flags(target: &str) -> &'static str {
    // Linux (any arch) vs macOS — the O_* bit values differ (Linux O_CREAT=0x40,
    // O_EXCL=0x80, O_CLOEXEC=0x80000; macOS O_CREAT=0x200, O_EXCL=0x800,
    // O_CLOEXEC=0x1000000). Matching only "linux-aarch64" gave linux-x86_64 the
    // macOS bits → a wrong open.
    if target.starts_with("linux") {
        "524482"
    } else {
        // O_RDWR|O_CREAT|O_EXCL|O_CLOEXEC = 0x2|0x200|0x800|0x1000000 = 16779266.
        // The temp fd was previously opened without O_CLOEXEC (bug-102).
        "16779266"
    }
}

fn emit_uuid_v4_to_path(
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    vregs: &mut Vregs,
    random_offset: usize,
    cursor: &str,
) {
    let dash = vregs.next();
    let byte = vregs.next();
    let mask = vregs.next();
    let high = vregs.next();
    let low = vregs.next();
    for index in 0..16 {
        if matches!(index, 4 | 6 | 8 | 10) {
            instructions.extend([
                abi::move_immediate(&dash, "Byte", "45"),
                abi::store_u8(&dash, cursor, 0),
                abi::add_immediate(cursor, cursor, 1),
            ]);
        }
        instructions.push(abi::load_u8(
            &byte,
            abi::stack_pointer(),
            random_offset + index,
        ));
        if index == 6 {
            instructions.extend([
                abi::move_immediate(&mask, "Integer", "15"),
                abi::and_registers(&byte, &byte, &mask),
                abi::move_immediate(&mask, "Integer", "64"),
                abi::or_registers(&byte, &byte, &mask),
            ]);
        } else if index == 8 {
            instructions.extend([
                abi::move_immediate(&mask, "Integer", "63"),
                abi::and_registers(&byte, &byte, &mask),
                abi::move_immediate(&mask, "Integer", "128"),
                abi::or_registers(&byte, &byte, &mask),
            ]);
        }
        instructions.extend([
            abi::shift_right_immediate(&high, &byte, 4),
            abi::move_immediate(&low, "Integer", "15"),
            abi::and_registers(&low, &byte, &low),
        ]);
        emit_hex_nibble_to_path(symbol, instructions, vregs, index, "high", &high, cursor);
        emit_hex_nibble_to_path(symbol, instructions, vregs, index, "low", &low, cursor);
    }
}

fn emit_hex_nibble_to_path(
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    vregs: &mut Vregs,
    byte_index: usize,
    half: &str,
    nibble: &str,
    cursor: &str,
) {
    let digit = format!("{symbol}_uuid_{byte_index}_{half}_digit");
    let store = format!("{symbol}_uuid_{byte_index}_{half}_store");
    let ascii = vregs.next();
    instructions.extend([
        abi::compare_immediate(nibble, "10"),
        abi::branch_lt(&digit),
        abi::add_immediate(&ascii, nibble, 87),
        abi::branch(&store),
        abi::label(&digit),
        abi::add_immediate(&ascii, nibble, 48),
        abi::label(&store),
        abi::store_u8(&ascii, cursor, 0),
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
) -> Result<
    (
        CodeFrame,
        Vec<CodeInstruction>,
        Vec<CodeRelocation>,
        Vec<CodeStackSlot>,
    ),
    String,
> {
    // Vreg-allocated (plan-00-G Phase 2). Atomic write: build a temp template,
    // mkstemps, write the value, fsync, close, then rename onto the final path.
    // Every value held across one of those calls (path, value, temp_path, fd, the
    // write cursors, the two C-strings) is a spilled vreg; all buffers are
    // arena-allocated, so there is no on-stack buffer.
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
    // bug-63: post-`mkstemps` failure tails unlink the temp file before erroring so
    // a failed atomic write never litters the target directory with a stray temp.
    let unlink_alloc_error = format!("{symbol}_unlink_alloc_error");
    let rename_error = format!("{symbol}_rename_error");
    let rename_failed = format!("{symbol}_rename_failed");
    let rename_error_map = format!("{symbol}_rename_error_map");
    let done = format!("{symbol}_done");

    let mut vregs = Vregs::new();
    let path = vregs.next();
    let value = vregs.next();
    let temp_path = vregs.next();
    let fd = vregs.next();
    let remaining = vregs.next();
    let cursor = vregs.next();
    let c_temp = vregs.next();
    let c_final = vregs.next();
    let len0 = vregs.next();
    let plen = vregs.next();
    let datalen = vregs.next();
    let src = vregs.next();
    let dst = vregs.next();
    let index = vregs.next();
    let byte = vregs.next();
    // Holds the rename errno across the temp-file unlink call (which itself sets
    // errno) so the rename failure is still mapped to the right Result (bug-63).
    let saved_errno = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&path, abi::return_register()),
        abi::move_register(&value, abi::RET[1]),
        abi::load_u64(&len0, &path, 0),
        abi::compare_immediate(&len0, "0"),
        abi::branch_eq(&invalid),
        abi::add_immediate(abi::return_register(), &len0, 9 + TEMPLATE_SUFFIX.len()),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
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
        abi::branch_eq(&temp_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&temp_alloc_ok),
        abi::move_register(&temp_path, abi::RET[1]),
        abi::load_u64(&plen, &path, 0),
        abi::add_immediate(&datalen, &plen, TEMPLATE_SUFFIX.len()),
        abi::store_u64(&datalen, &temp_path, 0),
        abi::add_immediate(&src, &path, 8),
        abi::add_immediate(&dst, &temp_path, 8),
        abi::move_immediate(&index, "Integer", "0"),
        abi::label(&copy_path_loop),
        abi::compare_registers(&index, &plen),
        abi::branch_eq(&copy_path_done),
        abi::load_u8(&byte, &src, 0),
        abi::compare_immediate(&byte, "0"),
        abi::branch_eq(&invalid),
        abi::store_u8(&byte, &dst, 0),
        abi::add_immediate(&src, &src, 1),
        abi::add_immediate(&dst, &dst, 1),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&copy_path_loop),
        abi::label(&copy_path_done),
    ]);
    for b in MFB_PREFIX.iter().chain(X_MARKERS).chain(TMP_SUFFIX) {
        instructions.extend([
            abi::move_immediate(&byte, "Byte", &b.to_string()),
            abi::store_u8(&byte, &dst, 0),
            abi::add_immediate(&dst, &dst, 1),
        ]);
    }
    instructions.extend([
        abi::store_u8(abi::ZERO, &dst, 0),
        abi::add_immediate(abi::return_register(), &temp_path, 8),
        abi::move_immediate(abi::ARG[1], "Integer", &MKTEMPS_SUFFIX_LEN.to_string()),
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
        abi::move_register(&fd, abi::return_register()),
    ]);
    match value_kind {
        AtomicWriteValueKind::String => {
            instructions.extend([
                abi::load_u64(&remaining, &value, 0),
                abi::add_immediate(&cursor, &value, 8),
            ]);
        }
        AtomicWriteValueKind::Bytes => {
            let cap = vregs.next();
            instructions.extend([
                abi::load_u64(&remaining, &value, COLLECTION_OFFSET_DATA_LENGTH),
                abi::add_immediate(&cursor, &value, COLLECTION_HEADER_SIZE),
                abi::load_u64(&cap, &value, COLLECTION_OFFSET_CAPACITY),
                abi::move_immediate(&byte, "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
                abi::multiply_registers(&cap, &cap, &byte),
                abi::add_registers(&cursor, &cursor, &cap),
            ]);
        }
    }
    instructions.extend([
        abi::label(&write_loop),
        abi::compare_immediate(&remaining, "0"),
        abi::branch_eq(&write_ok),
        abi::move_register(abi::return_register(), &fd),
        abi::move_register(abi::ARG[1], &cursor),
        abi::move_register(abi::ARG[2], &remaining),
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
        abi::add_registers(&cursor, &cursor, abi::return_register()),
        abi::subtract_registers(&remaining, &remaining, abi::return_register()),
        abi::branch(&write_loop),
        abi::label(&write_ok),
        abi::move_register(abi::return_register(), &fd),
    ]);
    platform.emit_sync_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    normalize_c_int_result(&mut instructions);
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&sync_error),
        abi::move_register(abi::return_register(), &fd),
    ]);
    platform.emit_close_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    normalize_c_int_result(&mut instructions);
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&close_error),
        abi::load_u64(abi::return_register(), &temp_path, 0),
        abi::add_immediate(abi::return_register(), abi::return_register(), 1),
        abi::move_immediate(abi::ARG[1], "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    alloc_reloc(&mut relocations);
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&c_temp_alloc_ok),
        abi::branch(&unlink_alloc_error),
        abi::label(&c_temp_alloc_ok),
        abi::move_register(&c_temp, abi::RET[1]),
        abi::load_u64(abi::return_register(), &path, 0),
        abi::add_immediate(abi::return_register(), abi::return_register(), 1),
        abi::move_immediate(abi::ARG[1], "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    alloc_reloc(&mut relocations);
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&c_final_alloc_ok),
        abi::branch(&unlink_alloc_error),
        abi::label(&c_final_alloc_ok),
        abi::move_register(&c_final, abi::RET[1]),
        abi::load_u64(&plen, &temp_path, 0),
        abi::add_immediate(&src, &temp_path, 8),
        abi::move_register(&dst, &c_temp),
        abi::move_immediate(&index, "Integer", "0"),
        abi::label(&c_temp_loop),
        abi::compare_registers(&index, &plen),
        abi::branch_eq(&c_temp_done),
        abi::load_u8(&byte, &src, 0),
        abi::store_u8(&byte, &dst, 0),
        abi::add_immediate(&src, &src, 1),
        abi::add_immediate(&dst, &dst, 1),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&c_temp_loop),
        abi::label(&c_temp_done),
        abi::store_u8(abi::ZERO, &dst, 0),
        abi::load_u64(&plen, &path, 0),
        abi::add_immediate(&src, &path, 8),
        abi::move_register(&dst, &c_final),
        abi::move_immediate(&index, "Integer", "0"),
        abi::label(&c_final_loop),
        abi::compare_registers(&index, &plen),
        abi::branch_eq(&c_final_done),
        abi::load_u8(&byte, &src, 0),
        abi::store_u8(&byte, &dst, 0),
        abi::add_immediate(&src, &src, 1),
        abi::add_immediate(&dst, &dst, 1),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&c_final_loop),
        abi::label(&c_final_done),
        abi::store_u8(abi::ZERO, &dst, 0),
        abi::move_register(abi::return_register(), &c_temp),
        abi::move_register(abi::ARG[1], &c_final),
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
        // rename failed: the temp file still exists on disk — unlink it before
        // mapping the errno (bug-63). The mkstemps-failure path (no temp) enters at
        // `rename_error` instead and skips the unlink.
        abi::branch(&rename_failed),
        abi::label(&rename_ok),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&rename_error),
    ]);
    let errno_reg = vregs.next();
    platform.emit_errno(
        symbol,
        &errno_reg,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.push(abi::branch(&rename_error_map));
    // rename failure: capture the rename errno, unlink the leftover temp file
    // (which sets errno itself), restore the rename errno, then map it.
    instructions.push(abi::label(&rename_failed));
    platform.emit_errno(
        symbol,
        &errno_reg,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::move_register(&saved_errno, &errno_reg),
        abi::add_immediate(abi::return_register(), &temp_path, 8),
    ]);
    platform.emit_fs_path_operation(
        symbol,
        FsPathOperation::Unlink,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::move_register(&errno_reg, &saved_errno),
        abi::label(&rename_error_map),
    ]);
    emit_errno_error_mapping(symbol, &errno_reg, &mut instructions, &mut relocations, &done);
    // `emit_errno_error_mapping`'s generic `err_output` case does not branch to
    // `done` — terminate the mkstemps/rename errno path explicitly so it cannot
    // fall through into the write/sync close tail below and re-close the fd (a
    // garbage fd vreg on mkstemps failure; an already-closed fd on rename failure).
    instructions.push(abi::branch(&done));
    instructions.extend([
        abi::label(&write_error),
        abi::label(&sync_error),
        abi::move_register(abi::return_register(), &fd),
    ]);
    platform.emit_close_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    // bug-63: the write/fsync/close failure tails converge here — the temp file
    // exists on disk, so unlink it before reporting ErrOutput. ErrOutput carries a
    // fixed code, so clobbering errno in the unlink call is harmless.
    instructions.extend([
        abi::label(&close_error),
        abi::add_immediate(abi::return_register(), &temp_path, 8),
    ]);
    platform.emit_fs_path_operation(
        symbol,
        FsPathOperation::Unlink,
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
    // bug-63: an alloc failure AFTER mkstemps (the c_temp/c_final C-string buffers)
    // must unlink the leftover temp file before reporting OOM. The pre-mkstemps
    // temp_path alloc branches straight to `alloc_error`, where no temp exists yet.
    // This block unlinks, then falls through into the shared `alloc_error` result.
    instructions.extend([
        abi::branch(&done),
        abi::label(&unlink_alloc_error),
        abi::add_immediate(abi::return_register(), &temp_path, 8),
    ]);
    platform.emit_fs_path_operation(
        symbol,
        FsPathOperation::Unlink,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
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

pub(super) fn lower_fs_write_text_path_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    append: bool,
) -> Result<
    (
        CodeFrame,
        Vec<CodeInstruction>,
        Vec<CodeRelocation>,
        Vec<CodeStackSlot>,
    ),
    String,
> {
    // Vreg-allocated (plan-00-G Phase 2). path→C-string, open, write loop, fsync,
    // close. fd (across write/sync/close) and the value (across open) are spilled
    // vregs; the C-string is consumed at open.
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
    let mut vregs = Vregs::new();
    let path = vregs.next();
    let value = vregs.next();
    let c_path = vregs.next();
    let fd = vregs.next();
    let remaining = vregs.next();
    let cursor = vregs.next();
    let len0 = vregs.next();
    let len = vregs.next();
    let src = vregs.next();
    let dst = vregs.next();
    let index = vregs.next();
    let byte = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&path, abi::return_register()),
        abi::move_register(&value, abi::RET[1]),
        abi::load_u64(&len0, &path, 0),
        abi::compare_immediate(&len0, "0"),
        abi::branch_eq(&invalid),
        abi::add_immediate(abi::return_register(), &len0, 1),
        abi::move_immediate(abi::ARG[1], "Integer", "1"),
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
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::move_register(&c_path, abi::RET[1]),
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
        abi::store_u8(abi::ZERO, &dst, 0),
        abi::move_register(abi::return_register(), &c_path),
        abi::move_immediate(abi::ARG[1], "Integer", mode_flags),
        abi::move_immediate(abi::ARG[2], "Integer", "438"),
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
        abi::move_register(&fd, abi::return_register()),
        abi::load_u64(&remaining, &value, 0),
        abi::add_immediate(&cursor, &value, 8),
        abi::label(&write_loop),
        abi::compare_immediate(&remaining, "0"),
        abi::branch_eq(&write_done),
        abi::move_register(abi::return_register(), &fd),
        abi::move_register(abi::ARG[1], &cursor),
        abi::move_register(abi::ARG[2], &remaining),
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
        abi::add_registers(&cursor, &cursor, abi::return_register()),
        abi::subtract_registers(&remaining, &remaining, abi::return_register()),
        abi::branch(&write_loop),
        abi::label(&write_done),
        abi::move_register(abi::return_register(), &fd),
    ]);
    platform.emit_sync_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    normalize_c_int_result(&mut instructions);
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&write_error),
        abi::move_register(abi::return_register(), &fd),
    ]);
    platform.emit_close_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    normalize_c_int_result(&mut instructions);
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&close_error),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&write_error),
        abi::move_register(abi::return_register(), &fd),
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
    let errno_reg = vregs.next();
    platform.emit_errno(
        symbol,
        &errno_reg,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_errno_error_mapping(symbol, &errno_reg, &mut instructions, &mut relocations, &done);
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
    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

pub(super) fn lower_fs_read_text_path_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<
    (
        CodeFrame,
        Vec<CodeInstruction>,
        Vec<CodeRelocation>,
        Vec<CodeStackSlot>,
    ),
    String,
> {
    // Vreg-allocated (plan-00-G Phase 2). path→C-string, open(read), seek end/start
    // for the size, alloc the string, read loop, close, UTF-8 validate. fd (across
    // seeks/read/close), the length, and the result string are spilled vregs.
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
    let mut vregs = Vregs::new();
    let path = vregs.next();
    let c_path = vregs.next();
    let fd = vregs.next();
    let length = vregs.next();
    let string = vregs.next();
    let remaining = vregs.next();
    let cursor = vregs.next();
    let len0 = vregs.next();
    let len = vregs.next();
    let src = vregs.next();
    let dst = vregs.next();
    let index = vregs.next();
    let byte = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&path, abi::return_register()),
        abi::load_u64(&len0, &path, 0),
        abi::compare_immediate(&len0, "0"),
        abi::branch_eq(&invalid),
        abi::add_immediate(abi::return_register(), &len0, 1),
        abi::move_immediate(abi::ARG[1], "Integer", "1"),
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
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::move_register(&c_path, abi::RET[1]),
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
        abi::store_u8(abi::ZERO, &dst, 0),
        abi::move_register(abi::return_register(), &c_path),
        abi::move_immediate(abi::ARG[1], "Integer", flags.read),
        abi::move_immediate(abi::ARG[2], "Integer", "0"),
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
        abi::move_register(&fd, abi::return_register()),
        // Restage the fd as the first argument explicitly. On AArch64 `x0`
        // still holds open's return so this looks redundant, but on x86-64 the
        // result register (rax) and the first argument register (rdi) differ —
        // without this, lseek reads whatever the libc open wrapper left in rdi
        // (glibc: AT_FDCWD → EBADF; musl happened to leave the fd there, which
        // masked the bug). Every sibling seek/read/close site already does this.
        abi::move_register(abi::return_register(), &fd),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
        abi::move_immediate(abi::ARG[2], "Integer", "2"),
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
        abi::move_register(&length, abi::return_register()),
        abi::move_register(abi::return_register(), &fd),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
        abi::move_immediate(abi::ARG[2], "Integer", "0"),
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
        abi::add_immediate(abi::return_register(), &length, 9),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
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
        abi::move_register(&string, abi::RET[1]),
        abi::store_u64(&length, &string, 0),
        abi::move_register(&remaining, &length),
        abi::add_immediate(&cursor, &string, 8),
        abi::label(&read_loop),
        abi::compare_immediate(&remaining, "0"),
        abi::branch_eq(&read_done),
        abi::move_register(abi::return_register(), &fd),
        abi::move_register(abi::ARG[1], &cursor),
        abi::move_register(abi::ARG[2], &remaining),
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
        abi::add_registers(&cursor, &cursor, abi::return_register()),
        abi::subtract_registers(&remaining, &remaining, abi::return_register()),
        abi::branch(&read_loop),
        abi::label(&read_done),
        abi::store_u8(abi::ZERO, &cursor, 0),
        abi::move_register(abi::return_register(), &fd),
    ]);
    platform.emit_close_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    let encoding_error = format!("{symbol}_encoding_error");
    instructions.extend([
        abi::add_immediate(abi::ARG[0], &string, 8),
        abi::move_register(abi::ARG[1], &length),
    ]);
    emit_call_validate_utf8(symbol, &encoding_error, &mut instructions, &mut relocations);
    instructions.extend([
        abi::move_register(RESULT_VALUE_REGISTER, &string),
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
        abi::move_register(abi::return_register(), &fd),
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
    let errno_reg = vregs.next();
    platform.emit_errno(
        symbol,
        &errno_reg,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_fs_path_errno_error_mapping(
        symbol,
        &errno_reg,
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
        // The result-String allocation failed after the file fd was opened and
        // seeked; close the fd before returning ErrOutOfMemory so a caught OOM in
        // a loop doesn't leak an fd per call and exhaust the fd table (bug-101,
        // mirroring the fd-close on this helper's seek/read error paths).
        abi::move_register(abi::return_register(), &fd),
    ]);
    platform.emit_close_file(symbol, platform_imports, &mut instructions, &mut relocations)?;
    instructions.extend([
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

pub(super) fn lower_fs_write_bytes_path_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    append: bool,
) -> Result<
    (
        CodeFrame,
        Vec<CodeInstruction>,
        Vec<CodeRelocation>,
        Vec<CodeStackSlot>,
    ),
    String,
> {
    // Vreg-allocated (plan-00-G Phase 2). Like write_text_path, but the source is a
    // byte-List's data region. fd / value spill across the calls.
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
    let mut vregs = Vregs::new();
    let path = vregs.next();
    let value = vregs.next();
    let c_path = vregs.next();
    let fd = vregs.next();
    let remaining = vregs.next();
    let cursor = vregs.next();
    let len0 = vregs.next();
    let len = vregs.next();
    let src = vregs.next();
    let dst = vregs.next();
    let index = vregs.next();
    let byte = vregs.next();
    let cap = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&path, abi::return_register()),
        abi::move_register(&value, abi::RET[1]),
        abi::load_u64(&len0, &path, 0),
        abi::compare_immediate(&len0, "0"),
        abi::branch_eq(&invalid),
        abi::add_immediate(abi::return_register(), &len0, 1),
        abi::move_immediate(abi::ARG[1], "Integer", "1"),
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
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::move_register(&c_path, abi::RET[1]),
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
        abi::store_u8(abi::ZERO, &dst, 0),
        abi::move_register(abi::return_register(), &c_path),
        abi::move_immediate(abi::ARG[1], "Integer", mode_flags),
        abi::move_immediate(abi::ARG[2], "Integer", "438"),
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
        abi::move_register(&fd, abi::return_register()),
        abi::load_u64(&remaining, &value, COLLECTION_OFFSET_DATA_LENGTH),
        abi::add_immediate(&cursor, &value, COLLECTION_HEADER_SIZE),
        abi::load_u64(&cap, &value, COLLECTION_OFFSET_CAPACITY),
        abi::move_immediate(&byte, "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers(&cap, &cap, &byte),
        abi::add_registers(&cursor, &cursor, &cap),
        abi::label(&write_loop),
        abi::compare_immediate(&remaining, "0"),
        abi::branch_eq(&write_done),
        abi::move_register(abi::return_register(), &fd),
        abi::move_register(abi::ARG[1], &cursor),
        abi::move_register(abi::ARG[2], &remaining),
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
        abi::add_registers(&cursor, &cursor, abi::return_register()),
        abi::subtract_registers(&remaining, &remaining, abi::return_register()),
        abi::branch(&write_loop),
        abi::label(&write_done),
        abi::move_register(abi::return_register(), &fd),
    ]);
    platform.emit_sync_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    normalize_c_int_result(&mut instructions);
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&write_error),
        abi::move_register(abi::return_register(), &fd),
    ]);
    platform.emit_close_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    normalize_c_int_result(&mut instructions);
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&close_error),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&write_error),
        abi::move_register(abi::return_register(), &fd),
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
    let errno_reg = vregs.next();
    platform.emit_errno(
        symbol,
        &errno_reg,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_errno_error_mapping(symbol, &errno_reg, &mut instructions, &mut relocations, &done);
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
    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

pub(super) fn lower_fs_read_bytes_path_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<
    (
        CodeFrame,
        Vec<CodeInstruction>,
        Vec<CodeRelocation>,
        Vec<CodeStackSlot>,
    ),
    String,
> {
    // Vreg-allocated (plan-00-G Phase 2). path→C-string, open(read), wrap in a File
    // record, delegate to `readAllBytes`, then close (stashing the Result across the
    // close in vregs). fd and the saved Result fields are spilled vregs.
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
    let mut vregs = Vregs::new();
    let path = vregs.next();
    let c_path = vregs.next();
    let fd = vregs.next();
    let save_tag = vregs.next();
    let save_value = vregs.next();
    let save_message = vregs.next();
    let len0 = vregs.next();
    let len = vregs.next();
    let src = vregs.next();
    let dst = vregs.next();
    let index = vregs.next();
    let byte = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&path, abi::return_register()),
        abi::load_u64(&len0, &path, 0),
        abi::compare_immediate(&len0, "0"),
        abi::branch_eq(&invalid),
        abi::add_immediate(abi::return_register(), &len0, 1),
        abi::move_immediate(abi::ARG[1], "Integer", "1"),
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
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::move_register(&c_path, abi::RET[1]),
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
        abi::store_u8(abi::ZERO, &dst, 0),
        abi::move_register(abi::return_register(), &c_path),
        abi::move_immediate(abi::ARG[1], "Integer", flags.read),
        abi::move_immediate(abi::ARG[2], "Integer", "0"),
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
        abi::move_register(&fd, abi::return_register()),
        abi::move_immediate(abi::return_register(), "Integer", RESOURCE_RECORD_SIZE),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
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
        // The File-record alloc failed after `open` succeeded: close the fd before
        // reporting OOM so the error path does not leak the OS fd (bug-63). `fd` is
        // a spilled vreg, surviving the failed alloc and this close.
        abi::move_register(abi::return_register(), &fd),
    ]);
    platform.emit_close_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::branch(&alloc_error),
        abi::label(&file_alloc_ok),
        abi::store_u64(&fd, abi::RET[1], FILE_OFFSET_FD),
        abi::store_u64(abi::ZERO, abi::RET[1], FILE_OFFSET_CLOSED),
        abi::store_u64(abi::ZERO, abi::RET[1], FILE_OFFSET_STATE),
        // Opt-in per-File output buffer (plan-14-B): a fresh handle is unbuffered.
        // Arena memory is poisoned, so zero the buffer fields explicitly.
        abi::store_u64(abi::ZERO, abi::RET[1], FILE_OFFSET_BUF_PTR),
        abi::store_u64(abi::ZERO, abi::RET[1], FILE_OFFSET_BUF_FILLED),
        abi::store_u64(abi::ZERO, abi::RET[1], FILE_OFFSET_BUF_ENABLED),
        // Transparent read buffer (plan-14-C): empty cache at the fd's position.
        abi::store_u64(abi::ZERO, abi::RET[1], FILE_OFFSET_READ_PTR),
        abi::store_u64(abi::ZERO, abi::RET[1], FILE_OFFSET_READ_POS),
        abi::store_u64(abi::ZERO, abi::RET[1], FILE_OFFSET_READ_FILL),
        abi::store_u64(abi::ZERO, abi::RET[1], FILE_OFFSET_READ_AT_EOF),
        abi::move_register(abi::return_register(), abi::RET[1]),
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
        abi::move_register(&save_tag, RESULT_TAG_REGISTER),
        abi::move_register(&save_value, RESULT_VALUE_REGISTER),
        abi::move_register(&save_message, RESULT_ERROR_MESSAGE_REGISTER),
        abi::move_register(abi::return_register(), &fd),
    ]);
    platform.emit_close_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::move_register(RESULT_TAG_REGISTER, &save_tag),
        abi::move_register(RESULT_VALUE_REGISTER, &save_value),
        abi::move_register(RESULT_ERROR_MESSAGE_REGISTER, &save_message),
        abi::branch(&done),
        abi::label(&open_error),
    ]);
    let errno_reg = vregs.next();
    platform.emit_errno(
        symbol,
        &errno_reg,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_fs_path_errno_error_mapping(
        symbol,
        &errno_reg,
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
    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}
