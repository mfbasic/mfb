//! plan-176-S: `fs::gen_read_write`'s position queries and positioned reads — eof, size, seek and readBytesAt. The shared helpers live in
//! `builtins/fs/gen_read_write/mod.rs`.

use super::*;

pub(crate) fn lower_fs_eof_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<FsBodyParts, String> {
    // Vreg-allocated (plan-00-G Phase 2). fd is held across the three seeks, the
    // start position across the second/third — both spilled vregs.
    let closed = format!("{symbol}_closed");
    let closed_moved = format!("{symbol}_closed_moved");
    let seek_error = format!("{symbol}_seek_error");
    let at_eof = format!("{symbol}_at_eof");
    let not_eof = format!("{symbol}_not_eof");
    let done = format!("{symbol}_done");
    let mut vregs = Vregs::new();
    let file = vregs.next();
    let fd = vregs.next();
    let start = vregs.next();
    let end = vregs.next();
    let closed_flag = vregs.next();
    let read_pos = vregs.next();
    let read_fill = vregs.next();
    let mut instructions = vec![
        abi::move_register(&file, abi::return_register()),
        abi::load_u64(&closed_flag, &file, FILE_OFFSET_CLOSED),
    ];
    instructions.extend(crate::codegen::resource::closed_or_moved_guard(
        &closed_flag,
        &closed,
        &closed_moved,
    ));
    instructions.extend([
        abi::load_u64(&fd, &file, FILE_OFFSET_FD),
        // Buffer-aware: unconsumed bytes in the read buffer
        // (READ_POS < READ_FILL) mean not-EOF, whatever the raw fd position. When
        // the buffer is fully consumed the fd sits at the logical position, so the
        // fd-vs-size check below is exact.
        abi::load_u64(&read_pos, &file, FILE_OFFSET_READ_POS),
        abi::load_u64(&read_fill, &file, FILE_OFFSET_READ_FILL),
        abi::compare_registers(&read_pos, &read_fill),
        abi::branch_lt(&not_eof),
        abi::move_register(abi::return_register(), &fd),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        abi::move_immediate(abi::c_arg(2), "Integer", "1"),
    ]);
    let mut relocations = Vec::new();
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&seek_error),
        abi::move_register(&start, abi::return_register()),
        abi::move_register(abi::return_register(), &fd),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        abi::move_immediate(abi::c_arg(2), "Integer", "2"),
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
        abi::move_register(&end, abi::return_register()),
        abi::move_register(abi::return_register(), &fd),
        abi::move_register(abi::c_arg(1), &start),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
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
        abi::compare_registers(&start, &end),
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
    ]);
    raise_error_into(
        symbol,
        "ErrResourceClosed",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&closed_moved)]);
    raise_error_into(
        symbol,
        "ErrResourceMoved",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&seek_error)]);
    raise_error_into(symbol, "ErrReadFailed", &mut instructions, &mut relocations);
    instructions.extend([abi::label(&done), abi::return_()]);
    Ok((instructions, relocations, 0))
}

/// `abi_function` body for `fs::size` (plan-139-E §4.1).
///
/// The save/measure/restore seek triple `fs::eof` and `fs::readAllBytes` already
/// emit, returning the measured end instead of comparing it. Position-neutral: the
/// descriptor is left exactly where it was found, so an `fs::File` shared with a
/// reader (letter A's `Archive` aliases one) never sees its position move.
///
/// The read buffer is deliberately NOT reconciled. A file's length does not depend
/// on the read position, and the raw descriptor position is restored exactly, so
/// nothing `fs::readLine`'s read-ahead recorded becomes stale — reconciling would
/// only throw that read-ahead away.
pub(crate) fn lower_fs_size_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<FsBodyParts, String> {
    // Vreg-allocated (plan-00-G Phase 2). fd is held across the three seeks, the
    // saved position across the second and third, the measured length across the
    // restore — all spilled vregs.
    let closed = format!("{symbol}_closed");
    let closed_moved = format!("{symbol}_closed_moved");
    let seek_error = format!("{symbol}_seek_error");
    let done = format!("{symbol}_done");
    let mut vregs = Vregs::new();
    let file = vregs.next();
    let fd = vregs.next();
    let start = vregs.next();
    let end = vregs.next();
    let closed_flag = vregs.next();
    let mut instructions = vec![
        abi::move_register(&file, abi::return_register()),
        abi::load_u64(&closed_flag, &file, FILE_OFFSET_CLOSED),
    ];
    instructions.extend(crate::codegen::resource::closed_or_moved_guard(
        &closed_flag,
        &closed,
        &closed_moved,
    ));
    instructions.extend([
        abi::load_u64(&fd, &file, FILE_OFFSET_FD),
        abi::move_register(abi::return_register(), &fd),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        abi::move_immediate(abi::c_arg(2), "Integer", "1"),
    ]);
    let mut relocations = Vec::new();
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&seek_error),
        abi::move_register(&start, abi::return_register()),
        abi::move_register(abi::return_register(), &fd),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        abi::move_immediate(abi::c_arg(2), "Integer", "2"),
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
        abi::move_register(&end, abi::return_register()),
        abi::move_register(abi::return_register(), &fd),
        abi::move_register(abi::c_arg(1), &start),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
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
        abi::move_register(RESULT_VALUE_REGISTER, &end),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&closed),
    ]);
    raise_error_into(
        symbol,
        "ErrResourceClosed",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&closed_moved)]);
    raise_error_into(
        symbol,
        "ErrResourceMoved",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&seek_error)]);
    raise_error_into(symbol, "ErrReadFailed", &mut instructions, &mut relocations);
    instructions.extend([abi::label(&done), abi::return_()]);
    Ok((instructions, relocations, 0))
}

/// `abi_function` body for `fs::seek(file, position)` (bug-809).
///
/// Moves the descriptor to the absolute byte offset `position` (`lseek(fd,
/// position, SEEK_SET)` / `SetFilePointerEx(FILE_BEGIN)`). Both per-handle buffers
/// are settled first so the move is exact for whatever comes next:
///
/// * pending buffered OUTPUT is drained through `_mfb_rt_fs_file_drain` — it was
///   written at the old position and must land there, not after the move; a
///   failing drain raises `ErrWriteFailed` and leaves the position untouched;
/// * the READ cache (`fs::readLine`'s block read-ahead) is dropped. It held bytes
///   from the old position; an absolute seek needs no rewind (unlike
///   `emit_reconcile_read_buffer`'s relative one), only an empty cache, so the
///   next read refills from `position`.
///
/// A negative `position` is a caller mistake (`ErrInvalidArgument`), checked
/// before anything is touched. A host refusal to move — a pipe or other
/// non-seekable handle — raises `ErrReadFailed`, as `fs::size`/`fs::eof` do on the
/// same handles. Seeking past the end is allowed (POSIX and Win32 both permit it).
pub(crate) fn lower_fs_seek_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<FsBodyParts, String> {
    let closed = format!("{symbol}_closed");
    let closed_moved = format!("{symbol}_closed_moved");
    let invalid = format!("{symbol}_invalid");
    let drain_error = format!("{symbol}_drain_error");
    let seek_error = format!("{symbol}_seek_error");
    let done = format!("{symbol}_done");
    let mut vregs = Vregs::new();
    let file = vregs.next();
    let position = vregs.next();
    let closed_flag = vregs.next();
    let fd = vregs.next();
    let mut instructions = vec![
        abi::move_register(&file, abi::return_register()),
        abi::move_register(&position, abi::mfb_return(1)),
        abi::load_u64(&closed_flag, &file, FILE_OFFSET_CLOSED),
    ];
    instructions.extend(crate::codegen::resource::closed_or_moved_guard(
        &closed_flag,
        &closed,
        &closed_moved,
    ));
    instructions.extend([
        abi::compare_immediate(&position, "0"),
        abi::branch_lt(&invalid),
        // Drain pending buffered output at the OLD position (a no-op when the
        // handle is unbuffered or nothing is pending). 0 = ok, 1 = write failure.
        abi::move_register(abi::return_register(), &file),
        abi::branch_link(FILE_DRAIN_SYMBOL),
    ]);
    let mut relocations = vec![internal_branch(symbol, FILE_DRAIN_SYMBOL)];
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&drain_error),
        abi::load_u64(&fd, &file, FILE_OFFSET_FD),
        abi::move_register(abi::return_register(), &fd),
        abi::move_register(abi::c_arg(1), &position),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"), // SEEK_SET / FILE_BEGIN
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
        // Drop the read cache: its bytes came from the old position. The next
        // read refills from `position`.
        abi::store_u64(abi::ZERO, &file, FILE_OFFSET_READ_POS),
        abi::store_u64(abi::ZERO, &file, FILE_OFFSET_READ_FILL),
        abi::store_u64(abi::ZERO, &file, FILE_OFFSET_READ_AT_EOF),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&closed),
    ]);
    raise_error_into(
        symbol,
        "ErrResourceClosed",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&closed_moved)]);
    raise_error_into(
        symbol,
        "ErrResourceMoved",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&invalid)]);
    raise_error_into(
        symbol,
        "ErrInvalidArgument",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&drain_error)]);
    raise_error_into(
        symbol,
        "ErrWriteFailed",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&seek_error)]);
    raise_error_into(symbol, "ErrReadFailed", &mut instructions, &mut relocations);
    instructions.extend([abi::label(&done), abi::return_()]);
    Ok((instructions, relocations, 0))
}

/// `abi_function` body for `fs::readBytesAt` (plan-139-E §4.2).
///
/// `lower_fs_read_all_bytes_helper`'s allocation and read loop with the length
/// arithmetic replaced: instead of "everything from the current position to the
/// end", it reads `count` bytes starting at an absolute `offset`, clamped to what
/// the file actually holds. The clamp is what makes the contract "fewer than
/// `count` bytes only at end of file" true by construction rather than by a
/// short-read accident.
///
/// Position-neutral, like `fs::size`: the descriptor position is saved up front
/// and restored on EVERY exit, success or failure. A failed positional read that
/// left the position moved would silently corrupt an unrelated reader sharing the
/// handle, which is exactly what letter A's `Archive` does.
///
/// The read buffer is deliberately NOT reconciled — see `lower_fs_size_helper`.
/// Here there is a second reason: reconciling would make a read-only positional
/// query destroy a concurrent `fs::readLine`'s read-ahead.
pub(crate) fn lower_fs_read_bytes_at_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<FsBodyParts, String> {
    // Vreg-allocated (plan-00-G Phase 2). fd/saved are held across the seeks and
    // the read loop; the collection and its data-region base across the loop; the
    // entry-init loop makes no call.
    let closed = format!("{symbol}_closed");
    let closed_moved = format!("{symbol}_closed_moved");
    let invalid = format!("{symbol}_invalid");
    let seek_error = format!("{symbol}_seek_error");
    let restore_read_error = format!("{symbol}_restore_read_error");
    let restore_alloc_error = format!("{symbol}_restore_alloc_error");
    let clamped = format!("{symbol}_clamped");
    let have_n = format!("{symbol}_have_n");
    let alloc_ok = format!("{symbol}_alloc_ok");
    let entry_loop = format!("{symbol}_entry_loop");
    let entry_done = format!("{symbol}_entry_done");
    let read_loop = format!("{symbol}_read_loop");
    let read_done = format!("{symbol}_read_done");
    let read_error = format!("{symbol}_read_error");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let mut vregs = Vregs::new();
    let file = vregs.next();
    let offset = vregs.next();
    let count = vregs.next();
    let fd = vregs.next();
    let saved = vregs.next();
    let end = vregs.next();
    let length = vregs.next();
    let collection = vregs.next();
    let data_base = vregs.next();
    let entry_cursor = vregs.next();
    let idx = vregs.next();
    let remaining = vregs.next();
    let cursor = vregs.next();
    let scratch = vregs.next();
    let closed_flag = vregs.next();

    let mut instructions = vec![
        abi::move_register(&file, abi::return_register()),
        abi::move_register(&offset, abi::mfb_return(1)),
        abi::move_register(&count, abi::mfb_return(2)),
        abi::load_u64(&closed_flag, &file, FILE_OFFSET_CLOSED),
    ];
    instructions.extend(crate::codegen::resource::closed_or_moved_guard(
        &closed_flag,
        &closed,
        &closed_moved,
    ));
    instructions.extend([
        // A negative offset or count is a caller mistake, not a file condition.
        abi::compare_immediate(&offset, "0"),
        abi::branch_lt(&invalid),
        abi::compare_immediate(&count, "0"),
        abi::branch_lt(&invalid),
        abi::load_u64(&fd, &file, FILE_OFFSET_FD),
        // Save the current position so every exit can put it back.
        abi::move_register(abi::return_register(), &fd),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        abi::move_immediate(abi::c_arg(2), "Integer", "1"),
    ]);
    let mut relocations = Vec::new();
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&seek_error),
        abi::move_register(&saved, abi::return_register()),
        // Measure the file so `count` can be clamped to what actually exists.
        abi::move_register(abi::return_register(), &fd),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        abi::move_immediate(abi::c_arg(2), "Integer", "2"),
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
        abi::move_register(&end, abi::return_register()),
        // length = min(count, max(0, end - offset))
        abi::move_immediate(&length, "Integer", "0"),
        abi::compare_registers(&offset, &end),
        abi::branch_ge(&clamped),
        abi::subtract_registers(&scratch, &end, &offset),
        abi::move_register(&length, &count),
        abi::compare_registers(&length, &scratch),
        abi::branch_le(&have_n),
        abi::move_register(&length, &scratch),
        abi::label(&have_n),
        abi::label(&clamped),
        // Position at the requested offset.
        abi::move_register(abi::return_register(), &fd),
        abi::move_register(abi::c_arg(1), &offset),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
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
        // Allocate the byte List (read_all_bytes' block, with the clamped length).
        abi::move_immediate(&scratch, "Integer", &byte_list_entry_stride().to_string()),
        abi::multiply_registers(&scratch, &length, &scratch),
        abi::add_immediate(&scratch, &scratch, COLLECTION_HEADER_SIZE),
        abi::add_registers(abi::return_register(), &scratch, &length),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&restore_alloc_error),
        abi::label(&alloc_ok),
        abi::move_register(&collection, abi::mfb_return(1)),
        abi::move_immediate(&scratch, "Byte", &byte_list_block_kind().to_string()),
        abi::store_u8(&scratch, &collection, COLLECTION_OFFSET_KIND),
        abi::move_immediate(&scratch, "Byte", &COLLECTION_TYPE_NONE.to_string()),
        abi::store_u8(&scratch, &collection, COLLECTION_OFFSET_KEY_TYPE),
        abi::move_immediate(&scratch, "Byte", &COLLECTION_TYPE_BYTE.to_string()),
        abi::store_u8(&scratch, &collection, COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate(&scratch, "Byte", "1"),
        abi::store_u8(&scratch, &collection, COLLECTION_OFFSET_FLAGS_VERSION),
        abi::store_u64(&length, &collection, COLLECTION_OFFSET_COUNT),
        abi::store_u64(&length, &collection, COLLECTION_OFFSET_CAPACITY),
        abi::store_u64(&length, &collection, COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64(&length, &collection, COLLECTION_OFFSET_DATA_CAPACITY),
        abi::add_immediate(&entry_cursor, &collection, COLLECTION_HEADER_SIZE),
        abi::move_immediate(&scratch, "Integer", &byte_list_entry_stride().to_string()),
        abi::multiply_registers(&scratch, &length, &scratch),
        abi::add_registers(&data_base, &entry_cursor, &scratch),
        abi::move_immediate(&idx, "Integer", "0"),
        abi::label(&entry_loop),
        abi::compare_registers(&idx, &length),
        abi::branch_eq(&entry_done),
        // kind 2 has no entry array to fill (plan-57-D) — see read_all_bytes.
    ]);
    instructions.extend([
        abi::add_immediate(&idx, &idx, 1),
        abi::branch(&entry_loop),
        abi::label(&entry_done),
        abi::move_register(&remaining, &length),
        abi::move_register(&cursor, &data_base),
        abi::label(&read_loop),
        abi::compare_immediate(&remaining, "0"),
        abi::branch_eq(&read_done),
        abi::move_register(abi::return_register(), &fd),
        abi::move_register(abi::c_arg(1), &cursor),
        abi::move_register(abi::c_arg(2), &remaining),
    ]);
    platform.emit_read_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    // `false` = a zero-byte read is a failure here, not a clean end: the clamp
    // above already proved these bytes exist, so a short read means the file was
    // truncated underneath us.
    emit_transfer_loop_tail(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        abi::return_register(),
        false,
        &cursor,
        &remaining,
        &read_loop,
        &restore_read_error,
    )?;
    // Success: put the position back before handing the list over.
    instructions.extend([
        abi::label(&read_done),
        abi::move_register(abi::return_register(), &fd),
        abi::move_register(abi::c_arg(1), &saved),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
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
        abi::move_register(RESULT_VALUE_REGISTER, &collection),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    // Failure tails. Each restores the saved position first, so a raise never
    // leaves the caller's handle somewhere it did not put it. The restore's own
    // result is deliberately ignored: the original error is the one to report.
    for (tail, target) in [
        (&restore_read_error, &read_error),
        (&restore_alloc_error, &alloc_error),
    ] {
        instructions.extend([
            abi::label(tail),
            abi::move_register(abi::return_register(), &fd),
            abi::move_register(abi::c_arg(1), &saved),
            abi::move_immediate(abi::c_arg(2), "Integer", "0"),
        ]);
        platform.emit_seek_file(
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.push(abi::branch(target));
    }
    instructions.push(abi::label(&closed));
    raise_error_into(
        symbol,
        "ErrResourceClosed",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&closed_moved)]);
    raise_error_into(
        symbol,
        "ErrResourceMoved",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&invalid)]);
    raise_error_into(
        symbol,
        "ErrInvalidArgument",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&seek_error),
        abi::label(&read_error),
    ]);
    raise_error_into(symbol, "ErrReadFailed", &mut instructions, &mut relocations);
    instructions.extend([abi::branch(&done), abi::label(&alloc_error)]);
    raise_error_into(
        symbol,
        "ErrOutOfMemory",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    Ok((instructions, relocations, 0))
}
