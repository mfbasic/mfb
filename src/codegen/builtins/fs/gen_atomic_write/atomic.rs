//! plan-176-S: `fs::gen_atomic_write`'s the atomic write (`lower_fs_atomic_write_helper`). The shared helpers live in
//! `builtins/fs/gen_atomic_write/mod.rs`.

use super::*;

pub(crate) fn lower_fs_atomic_write_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    value_kind: AtomicWriteValueKind,
) -> Result<FsBodyParts, String> {
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
    // errno) so the rename failure is still mapped to the right Result.
    let saved_errno = vregs.next();
    let temp_path_size = vregs.next();
    let c_temp_size = vregs.next();
    let c_final_size = vregs.next();
    let mut instructions = vec![
        // bug-574: all three allocations are this helper's own scratch — it
        // returns `Nothing` — so all three are released at `done`. Nulled FIRST:
        // the empty-path rejection reaches `done` with none allocated, and each
        // OOM tail (including the `unlink_alloc_error` one) with only the earlier
        // ones.
        abi::move_immediate(&temp_path, "Integer", "0"),
        abi::move_immediate(&temp_path_size, "Integer", "0"),
        abi::move_immediate(&c_temp, "Integer", "0"),
        abi::move_immediate(&c_temp_size, "Integer", "0"),
        abi::move_immediate(&c_final, "Integer", "0"),
        abi::move_immediate(&c_final_size, "Integer", "0"),
        abi::move_register(&path, abi::return_register()),
        abi::move_register(&value, abi::mfb_return(1)),
        abi::load_u64(&len0, &path, 0),
        abi::compare_immediate(&len0, "0"),
        abi::branch_eq(&invalid),
        abi::add_immediate(&temp_path_size, &len0, 9 + TEMPLATE_SUFFIX.len()),
        abi::move_register(abi::return_register(), &temp_path_size),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ];
    let mut relocations = vec![internal_branch(symbol, ARENA_ALLOC_SYMBOL)];
    let alloc_reloc = |relocations: &mut Vec<CodeRelocation>| {
        relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    };
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&temp_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&temp_alloc_ok),
        abi::move_register(&temp_path, abi::mfb_return(1)),
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
        abi::move_immediate(abi::c_arg(1), "Integer", &MKTEMPS_SUFFIX_LEN.to_string()),
    ]);
    platform.emit_mkstemps(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        // C `int` mkstemps fd — sign-extend before the signed compare (bug-04/bug-170).
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
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
                abi::move_immediate(&byte, "Integer", &byte_list_entry_stride().to_string()),
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
        abi::move_register(abi::c_arg(1), &cursor),
        abi::move_register(abi::c_arg(2), &remaining),
    ]);
    platform.emit_write(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    // A 0 return moved nothing (hard error); a negative return is EINTR-retried at
    // write_loop (re-issuing with the unchanged cursor/remaining) before any byte
    // moved rather than treated as a hard ErrOutput (bug-62, matching the
    // File-based write loops in fs_helpers_io.rs).
    emit_transfer_loop_tail(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        abi::return_register(),
        write_uses_raw_syscall(platform),
        &cursor,
        &remaining,
        &write_loop,
        &write_error,
    )?;
    instructions.extend([
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
        abi::load_u64(&c_temp_size, &temp_path, 0),
        abi::add_immediate(&c_temp_size, &c_temp_size, 1),
        abi::move_register(abi::return_register(), &c_temp_size),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    alloc_reloc(&mut relocations);
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&c_temp_alloc_ok),
        abi::branch(&unlink_alloc_error),
        abi::label(&c_temp_alloc_ok),
        abi::move_register(&c_temp, abi::mfb_return(1)),
        abi::load_u64(&c_final_size, &path, 0),
        abi::add_immediate(&c_final_size, &c_final_size, 1),
        abi::move_register(abi::return_register(), &c_final_size),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    alloc_reloc(&mut relocations);
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&c_final_alloc_ok),
        abi::branch(&unlink_alloc_error),
        abi::label(&c_final_alloc_ok),
        abi::move_register(&c_final, abi::mfb_return(1)),
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
        abi::move_register(abi::c_arg(1), &c_final),
    ]);
    platform.emit_rename_path(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    // bug-166: durability of the rename itself requires fsyncing the containing
    // directory — otherwise the directory-entry update can be lost across a
    // crash/power loss even though the temp file's data was fsynced. Derive the
    // parent directory from the final-path C-string, open it O_RDONLY, fsync, and
    // close it before reporting Ok.
    let dir_scan = vregs.next();
    let dir_slash = vregs.next();
    let dir_fd = vregs.next();
    let dir_scan_loop = format!("{symbol}_dir_scan_loop");
    let dir_scan_next = format!("{symbol}_dir_scan_next");
    let dir_scan_done = format!("{symbol}_dir_scan_done");
    let dir_root = format!("{symbol}_dir_root");
    let dir_cwd = format!("{symbol}_dir_cwd");
    let dir_open = format!("{symbol}_dir_open");
    let dir_done = format!("{symbol}_dir_done");
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&rename_ok),
        // rename failed: the temp file still exists on disk — unlink it before
        // mapping the errno. The mkstemps-failure path (no temp) enters at
        // `rename_error` instead and skips the unlink.
        abi::branch(&rename_failed),
        abi::label(&rename_ok),
        // Scan `c_final` for the last '/' (47); `dir_slash` = 0 means none found.
        // `c_final` already served the rename, so we are free to truncate it here.
        abi::move_immediate(&dir_slash, "Integer", "0"),
        abi::move_register(&dir_scan, &c_final),
        abi::label(&dir_scan_loop),
        abi::load_u8(&byte, &dir_scan, 0),
        abi::compare_immediate(&byte, "0"),
        abi::branch_eq(&dir_scan_done),
        abi::compare_immediate(&byte, "47"),
        abi::branch_ne(&dir_scan_next),
        abi::move_register(&dir_slash, &dir_scan),
        abi::label(&dir_scan_next),
        abi::add_immediate(&dir_scan, &dir_scan, 1),
        abi::branch(&dir_scan_loop),
        abi::label(&dir_scan_done),
        abi::compare_immediate(&dir_slash, "0"),
        abi::branch_eq(&dir_cwd),
        abi::compare_registers(&dir_slash, &c_final),
        abi::branch_eq(&dir_root),
        // "dir/file" -> NUL-terminate at the last slash so the string names the dir.
        abi::store_u8(abi::ZERO, &dir_slash, 0),
        abi::branch(&dir_open),
        // "/file" -> the parent directory is the filesystem root "/".
        abi::label(&dir_root),
        abi::store_u8(abi::ZERO, &c_final, 1),
        abi::branch(&dir_open),
        // "file" (no slash) -> the parent directory is the current directory ".".
        abi::label(&dir_cwd),
        abi::move_immediate(&byte, "Byte", "46"),
        abi::store_u8(&byte, &c_final, 0),
        abi::store_u8(abi::ZERO, &c_final, 1),
        abi::label(&dir_open),
        // open(dir, O_RDONLY | O_CLOEXEC, 0) — close-on-exec so a concurrent
        // `process::spawn` child cannot inherit the directory fd (bug-499).
        abi::move_register(abi::return_register(), &c_final),
        abi::move_immediate(abi::c_arg(1), "Integer", o_cloexec(platform.family())),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
    ]);
    platform.emit_open_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    normalize_c_int_result(&mut instructions);
    instructions.extend([
        // Directory fsync is best-effort: the atomic rename already succeeded, so a
        // directory that cannot be opened or fsynced must not fail the write.
        abi::move_register(&dir_fd, abi::return_register()),
        abi::compare_immediate(&dir_fd, "0"),
        abi::branch_lt(&dir_done),
        abi::move_register(abi::return_register(), &dir_fd),
    ]);
    platform.emit_sync_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.push(abi::move_register(abi::return_register(), &dir_fd));
    platform.emit_close_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::label(&dir_done),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&rename_error),
    ]);
    let errno_reg = vregs.next();
    platform.emit_errno(
        symbol,
        (&errno_reg).into(),
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
        (&errno_reg).into(),
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
    // `emit_fs_host_error_mapping` branches to `done` in every case (including
    // its fallback), so this mkstemps/rename errno path already terminates and
    // cannot fall through into the write/sync close tail below. Both calls act on
    // the target's name — the temp file sits beside it — so the target is the
    // path a host code is classified against.
    emit_fs_host_error_mapping(
        symbol,
        &errno_reg,
        platform.family(),
        FsHostOp::Replace,
        false,
        Some(FsPathArg::String((&path).into())),
        FsFallback::Write,
        &mut instructions,
        &mut relocations,
        &done,
    );
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
    raise_error_into(
        symbol,
        "ErrWriteFailed",
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
    instructions.extend([abi::label(&alloc_error)]);
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
            HelperScratch::new(&temp_path, &temp_path_size),
            HelperScratch::new(&c_temp, &c_temp_size),
            HelperScratch::new(&c_final, &c_final_size),
        ],
        &mut vregs,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::return_());
    Ok((instructions, relocations, 0))
}
