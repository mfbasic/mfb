//! plan-176-S: `fs::gen_directory`'s listing a directory (`lower_fs_list_directory_helper`). The shared helpers live in
//! `builtins/fs/gen_directory/mod.rs`.

use super::*;

pub(crate) fn lower_fs_list_directory_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<FsBodyParts, String> {
    // Vreg-allocated (plan-00-G Phase 2). Two passes over the directory: count
    // entries + name bytes, allocate the List, then fill + sort. Every value held
    // across an opendir/readdir/closedir/sort call (c_path, dir handle, count,
    // data_len, collection, and the three fill cursors) is a vreg the allocator
    // spills; dirent fields are per-iteration scratch that never cross a call.
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
    let second_open_error = format!("{symbol}_second_open_error");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let mut vregs = Vregs::new();
    let path = vregs.next();
    let c_path = vregs.next();
    let dir = vregs.next();
    let count = vregs.next();
    let data_len = vregs.next();
    let collection = vregs.next();
    let entry_cursor = vregs.next();
    let data_cursor = vregs.next();
    let data_offset = vregs.next();
    // bug-48: bound the fill pass by the pass-1 allocation. `block_end` is the
    // one-past-the-end address of the data region (data-region-start + data_len);
    // `actual_count` is how many entries pass 2 actually wrote. A concurrent
    // writer that grows the directory between the two scans is truncated to the
    // sized capacity instead of overflowing the arena block, and the header is
    // trimmed to what was written so a shrink leaves no poisoned trailing entry.
    let block_end = vregs.next();
    let actual_count = vregs.next();
    let len0 = vregs.next();
    let len = vregs.next();
    let src = vregs.next();
    let dst = vregs.next();
    let index = vregs.next();
    let namelen = vregs.next();
    let nameptr = vregs.next();
    let byte = vregs.next();
    let scratch = vregs.next();

    let c_path_size = vregs.next();
    // bug-696: the List, when the second scan's open fails after it was
    // allocated; released at `done` like `c_path`. Nulled with it below.
    let abandoned = vregs.next();
    let abandoned_size = vregs.next();

    let mut instructions = vec![
        // bug-574: `c_path` is the marshalled path this helper never hands back
        // (the List it returns is a SECOND, separate allocation), released at
        // `done`. Nulled FIRST so the paths that reach `done` without allocating —
        // the empty-path rejection just below and `ErrOutOfMemory` — free nothing.
        abi::move_immediate(&c_path, "Integer", "0"),
        abi::move_immediate(&c_path_size, "Integer", "0"),
        abi::move_immediate(&abandoned, "Integer", "0"),
        abi::move_immediate(&abandoned_size, "Integer", "0"),
        abi::move_register(&path, abi::return_register()),
        abi::load_u64(&len0, &path, 0),
        abi::compare_immediate(&len0, "0"),
        abi::branch_eq(&invalid),
        abi::add_immediate(&c_path_size, &len0, 1),
        abi::move_register(abi::return_register(), &c_path_size),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ];
    let mut relocations = vec![internal_branch(symbol, ARENA_ALLOC_SYMBOL)];
    let alloc_reloc = |relocations: &mut Vec<CodeRelocation>| {
        relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    };
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&path_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&path_alloc_ok),
        abi::move_register(&c_path, abi::mfb_return(1)),
        abi::load_u64(&len, &path, 0),
        abi::add_immediate(&src, &path, 8),
        abi::move_register(&dst, &c_path),
        abi::move_immediate(&index, "Integer", "0"),
        abi::label(&path_copy_loop),
        abi::compare_registers(&index, &len),
        abi::branch_eq(&path_copy_done),
        abi::load_u8(&byte, &src, 0),
        abi::compare_immediate(&byte, "0"),
        abi::branch_eq(&invalid),
        abi::store_u8(&byte, &dst, 0),
        abi::add_immediate(&src, &src, 1),
        abi::add_immediate(&dst, &dst, 1),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&path_copy_loop),
        abi::label(&path_copy_done),
        abi::store_u8(abi::ZERO, &dst, 0),
        abi::move_register(abi::return_register(), &c_path),
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
        abi::move_register(&dir, abi::return_register()),
        abi::move_immediate(&count, "Integer", "0"),
        abi::move_immediate(&data_len, "Integer", "0"),
        abi::label(&count_loop),
        abi::move_register(abi::return_register(), &dir),
    ]);
    platform.emit_readdir(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    platform.emit_read_dir_entry(
        &format!("{symbol}_count"),
        nameptr.into(),
        namelen.into(),
        byte.into(),
        scratch.into(),
        &mut instructions,
    );
    let count_keep = count_skip.replace("skip", "keep");
    instructions.extend([
        abi::compare_immediate(&namelen, "1"),
        abi::branch_ne(&count_skip),
        abi::load_u8(&byte, &nameptr, 0),
        abi::compare_immediate(&byte, "46"),
        abi::branch_eq(&count_loop),
        abi::label(&count_skip),
        abi::compare_immediate(&namelen, "2"),
        abi::branch_ne(&count_keep),
        abi::load_u8(&byte, &nameptr, 0),
        abi::compare_immediate(&byte, "46"),
        abi::branch_ne(&count_keep),
        abi::load_u8(&byte, &nameptr, 1),
        abi::compare_immediate(&byte, "46"),
        abi::branch_eq(&count_loop),
        abi::label(&count_keep),
        abi::add_immediate(&count, &count, 1),
        abi::add_registers(&data_len, &data_len, &namelen),
        abi::branch(&count_loop),
        abi::label(&count_done),
        abi::move_register(abi::return_register(), &dir),
    ]);
    platform.emit_closedir(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::move_immediate(&scratch, "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers(&scratch, &count, &scratch),
        abi::add_registers(&scratch, &scratch, &data_len),
        abi::add_immediate(abi::return_register(), &scratch, COLLECTION_HEADER_SIZE),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    alloc_reloc(&mut relocations);
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::move_register(&collection, abi::mfb_return(1)),
        abi::move_immediate(&scratch, "Byte", &COLLECTION_KIND_LIST.to_string()),
        abi::store_u8(&scratch, &collection, COLLECTION_OFFSET_KIND),
        abi::move_immediate(&scratch, "Byte", &COLLECTION_TYPE_NONE.to_string()),
        abi::store_u8(&scratch, &collection, COLLECTION_OFFSET_KEY_TYPE),
        abi::move_immediate(&scratch, "Byte", &COLLECTION_TYPE_STRING.to_string()),
        abi::store_u8(&scratch, &collection, COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate(&scratch, "Byte", "1"),
        abi::store_u8(&scratch, &collection, COLLECTION_OFFSET_FLAGS_VERSION),
        // CAPACITY / DATA_CAPACITY are the pass-1 allocation sizes and must not be
        // trimmed: readers locate the value data region at
        // `HEADER + CAPACITY*ENTRY_SIZE + DATA_CAPACITY`, which is where pass 2
        // physically writes. COUNT / DATA_LENGTH are the *used* amounts and are
        // written after the fill loop from what pass 2 actually produced (bug-48).
        abi::store_u64(&count, &collection, COLLECTION_OFFSET_CAPACITY),
        abi::store_u64(&data_len, &collection, COLLECTION_OFFSET_DATA_CAPACITY),
        abi::add_immediate(&entry_cursor, &collection, COLLECTION_HEADER_SIZE),
        abi::move_immediate(&scratch, "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers(&scratch, &count, &scratch),
        abi::add_registers(&data_cursor, &entry_cursor, &scratch),
        abi::move_immediate(&data_offset, "Integer", "0"),
        // data region spans [data_cursor, data_cursor + data_len); block_end is
        // the fill-pass byte ceiling. actual_count counts entries pass 2 writes.
        abi::add_registers(&block_end, &data_cursor, &data_len),
        abi::move_immediate(&actual_count, "Integer", "0"),
        abi::move_register(abi::return_register(), &c_path),
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
        abi::branch(&second_open_error),
        abi::label(&second_open_ok),
        abi::move_register(&dir, abi::return_register()),
        abi::label(&fill_loop),
        abi::move_register(abi::return_register(), &dir),
    ]);
    platform.emit_readdir(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    platform.emit_read_dir_entry(
        &format!("{symbol}_fill"),
        nameptr.into(),
        namelen.into(),
        byte.into(),
        scratch.into(),
        &mut instructions,
    );
    let fill_keep = fill_skip.replace("skip", "keep");
    instructions.extend([
        abi::compare_immediate(&namelen, "1"),
        abi::branch_ne(&fill_skip),
        abi::load_u8(&byte, &nameptr, 0),
        abi::compare_immediate(&byte, "46"),
        abi::branch_eq(&fill_loop),
        abi::label(&fill_skip),
        abi::compare_immediate(&namelen, "2"),
        abi::branch_ne(&fill_keep),
        abi::load_u8(&byte, &nameptr, 0),
        abi::compare_immediate(&byte, "46"),
        abi::branch_ne(&fill_keep),
        abi::load_u8(&byte, &nameptr, 1),
        abi::compare_immediate(&byte, "46"),
        abi::branch_eq(&fill_loop),
        abi::label(&fill_keep),
        // bug-48 bound 1: never write more entries than pass 1 sized capacity for.
        abi::compare_registers(&actual_count, &count),
        abi::branch_ge(&fill_done),
        // bug-48 bound 2: never copy a name past the end of the data region.
        abi::add_registers(&scratch, &data_cursor, &namelen),
        abi::compare_registers(&scratch, &block_end),
        abi::branch_hi(&fill_done),
        abi::move_immediate(&scratch, "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
        abi::store_u8(&scratch, &entry_cursor, COLLECTION_ENTRY_OFFSET_FLAGS),
        abi::store_u64(abi::ZERO, &entry_cursor, COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
        abi::store_u64(abi::ZERO, &entry_cursor, COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
        abi::store_u64(
            &data_offset,
            &entry_cursor,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ),
        abi::store_u64(
            &namelen,
            &entry_cursor,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ),
        abi::move_immediate(&index, "Integer", "0"),
        abi::label(&copy_name_loop),
        abi::compare_registers(&index, &namelen),
        abi::branch_eq(&copy_name_done),
        abi::load_u8(&byte, &nameptr, 0),
        abi::store_u8(&byte, &data_cursor, 0),
        abi::add_immediate(&nameptr, &nameptr, 1),
        abi::add_immediate(&data_cursor, &data_cursor, 1),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&copy_name_loop),
        abi::label(&copy_name_done),
        abi::add_registers(&data_offset, &data_offset, &namelen),
        abi::add_immediate(&entry_cursor, &entry_cursor, COLLECTION_ENTRY_SIZE),
        abi::add_immediate(&actual_count, &actual_count, 1),
        abi::branch(&fill_loop),
        abi::label(&fill_done),
        abi::move_register(abi::return_register(), &dir),
    ]);
    platform.emit_closedir(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    // bug-48: trim the header to what pass 2 actually wrote. On a shrink race
    // pass 2 produces fewer entries/bytes than pass 1 sized for; storing the
    // pass-1 totals would leave `count - actual_count` trailing entries holding
    // uninitialized arena bytes that sort_string_list would dereference as
    // (offset, length) string descriptors. actual_count / data_offset are the
    // exact used amounts. CAPACITY / DATA_CAPACITY keep the pass-1 sizes.
    instructions.extend([
        abi::store_u64(&actual_count, &collection, COLLECTION_OFFSET_COUNT),
        abi::store_u64(&data_offset, &collection, COLLECTION_OFFSET_DATA_LENGTH),
    ]);
    instructions.push(abi::move_register(abi::return_register(), &collection));
    instructions.push(abi::branch_link(SORT_STRING_LIST_SYMBOL));
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: SORT_STRING_LIST_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::move_register(RESULT_VALUE_REGISTER, &collection),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        // bug-696: the directory went away between the two scans, after the List
        // was allocated. Hand it to the release at `done` — at its pass-1 size —
        // and report the open error. No call here, so `errno` is still the
        // failed open's.
        abi::label(&second_open_error),
        abi::move_register(&abandoned, &collection),
        abi::move_immediate(
            &abandoned_size,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ),
        abi::multiply_registers(&abandoned_size, &count, &abandoned_size),
        abi::add_registers(&abandoned_size, &abandoned_size, &data_len),
        abi::add_immediate(&abandoned_size, &abandoned_size, COLLECTION_HEADER_SIZE),
        abi::label(&open_error),
    ]);
    let errno_reg = vregs.next();
    platform.emit_errno(
        symbol,
        (&errno_reg).into(),
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_fs_host_error_mapping(
        symbol,
        &errno_reg,
        platform.family(),
        FsHostOp::Resolve,
        false,
        Some(FsPathArg::String((&path).into())),
        FsFallback::Read,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&invalid)]);
    raise_error_into(
        symbol,
        "ErrInvalidArgument",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&alloc_error)]);
    raise_error_into(
        symbol,
        "ErrOutOfMemory",
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::label(&done));
    emit_helper_scratch_release(
        symbol,
        &[
            HelperScratch::new(&c_path, &c_path_size),
            HelperScratch::new(&abandoned, &abandoned_size),
        ],
        &mut vregs,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::return_());
    Ok((instructions, relocations, 0))
}
