//! plan-176-S: `fs::gen_atomic_write`'s reading a whole file by path, as text or bytes. The shared helpers live in
//! `builtins/fs/gen_atomic_write/mod.rs`.

use super::*;

pub(crate) fn lower_fs_read_text_path_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<FsBodyParts, String> {
    // Vreg-allocated (plan-00-G Phase 2). path→C-string, open(read), seek end/start
    // for the size, alloc the string, read loop, close, UTF-8 validate. fd (across
    // seeks/read/close), the length, and the result string are spilled vregs.
    let scan_loop = format!("{symbol}_path_scan_loop");
    let scan_done = format!("{symbol}_path_scan_done");
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

    let flags = open_flag_set(platform.family(), false);
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
    let index = vregs.next();
    let byte = vregs.next();
    let mut instructions = vec![
        // plan-146-F: `c_path` is the caller's `path` block borrowed at `+8`,
        // where its bytes already end in a NUL — no copy, so nothing to release
        // at `done` (this is what bug-574's scratch was). An empty path, and an
        // interior NUL that would truncate what the host opens, are still
        // rejected.
        abi::move_register(&path, abi::return_register()),
        abi::load_u64(&len0, &path, 0),
        abi::compare_immediate(&len0, "0"),
        abi::branch_eq(&invalid),
        abi::add_immediate(&c_path, &path, 8),
    ];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::load_u64(&len, &path, 0),
        abi::move_register(&src, &c_path),
        abi::move_immediate(&index, "Integer", "0"),
    ]);
    emit_cstring_nul_scan(
        &mut instructions,
        &len,
        &src,
        &index,
        &byte,
        &scan_loop,
        &scan_done,
        &invalid,
    );
    instructions.extend([
        abi::move_register(abi::return_register(), &c_path),
        abi::move_immediate(abi::c_arg(1), "Integer", flags.read),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
    ]);
    platform.emit_open_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        // C `int` open fd — sign-extend before the signed compare (bug-04/bug-170).
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
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
        abi::branch_lt(&close_and_read_error),
        abi::move_register(&length, abi::return_register()),
        abi::move_register(abi::return_register(), &fd),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
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
        abi::branch_lt(&close_and_read_error),
        abi::add_immediate(abi::return_register(), &length, 9),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&string_alloc_ok),
        // The result-String alloc failed after `open`+`seek`: close the live fd
        // before reporting OOM (bug-101). Only this post-open failure closes fd;
        // the pre-open C-string alloc failure jumps to the close-free `alloc_error`
        // (bug-201 — it would otherwise close an unassigned fd vreg).
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
        abi::label(&string_alloc_ok),
        abi::move_register(&string, abi::mfb_return(1)),
        abi::store_u64(&length, &string, 0),
        abi::move_register(&remaining, &length),
        abi::add_immediate(&cursor, &string, 8),
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
    // A 0 return is an unexpected EOF (the file shrank) and stays a hard error; a
    // negative return is EINTR-retried at read_loop before any byte moved rather
    // than treated as a hard ErrRead (bug-62; reads always go through libc, so
    // raw_return is false).
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
        &read_error,
    )?;
    instructions.extend([
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
        abi::add_immediate(abi::c_arg(0), &string, 8),
        abi::move_register(abi::c_arg(1), &length),
    ]);
    emit_call_validate_utf8(symbol, &encoding_error, &mut instructions, &mut relocations);
    instructions.extend([
        abi::move_register(RESULT_VALUE_REGISTER, &string),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&encoding_error),
    ]);
    raise_error_into(symbol, "ErrEncoding", &mut instructions, &mut relocations);
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
    // bug-701: on POSIX a read-only open of a directory succeeds, and it is the
    // measurement or the read that fails — `EISDIR`, or on Linux ext4 a size no
    // allocation can satisfy (the `alloc_error` exit below). Report the
    // directory, as Windows does by refusing the open.
    let directory = format!("{symbol}_directory");
    let posix = platform.family() != PlatformFamily::Windows;
    instructions.push(abi::label(&seek_error));
    if posix {
        emit_branch_if_directory(
            symbol,
            FsPathArg::String((&path).into()),
            &directory,
            &mut instructions,
            &mut relocations,
        );
    }
    raise_error_into(symbol, "ErrReadFailed", &mut instructions, &mut relocations);
    instructions.extend([abi::branch(&done), abi::label(&open_error)]);
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
        FsHostOp::Read,
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
    instructions.extend([
        abi::branch(&done),
        // Close-free OOM exit: reached, after an inline `close`, from the
        // post-open result-String alloc failure. Closing fd here would close an
        // unassigned vreg (bug-201); plan-146-F removed the other reader of this
        // label, the pre-open C-string alloc failure, with the copy itself.
        abi::label(&alloc_error),
    ]);
    if posix {
        emit_branch_if_directory(
            symbol,
            FsPathArg::String((&path).into()),
            &directory,
            &mut instructions,
            &mut relocations,
        );
    }
    raise_error_into(
        symbol,
        "ErrOutOfMemory",
        &mut instructions,
        &mut relocations,
    );
    if posix {
        instructions.extend([abi::branch(&done), abi::label(&directory)]);
        raise_error_into(
            symbol,
            "ErrInvalidPath",
            &mut instructions,
            &mut relocations,
        );
    }
    instructions.push(abi::label(&done));
    instructions.push(abi::return_());
    Ok((instructions, relocations, 0))
}

pub(crate) fn lower_fs_read_bytes_path_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<FsBodyParts, String> {
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

    let flags = open_flag_set(platform.family(), false);
    let mut vregs = Vregs::new();
    let path = vregs.next();
    let c_path = vregs.next();
    let record = vregs.next();
    let record_size = vregs.next();
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
    let c_path_size = vregs.next();
    let mut instructions = vec![
        // bug-574: `c_path` is the marshalled path this helper never hands back,
        // released at `done`. Nulled FIRST so the paths that reach `done` without
        // allocating — the empty-path rejection just below and `alloc_error` —
        // free nothing.
        abi::move_immediate(&c_path, "Integer", "0"),
        abi::move_immediate(&c_path_size, "Integer", "0"),
        abi::move_immediate(&record, "Integer", "0"),
        abi::move_immediate(&record_size, "Integer", "0"),
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
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::move_register(&c_path, abi::mfb_return(1)),
        abi::load_u64(&len, &path, 0),
        abi::add_immediate(&src, &path, 8),
        abi::move_register(&dst, &c_path),
        abi::move_immediate(&index, "Integer", "0"),
    ]);
    emit_cstring_copy(
        &mut instructions,
        true,
        &len,
        &src,
        &dst,
        &index,
        &byte,
        &copy_loop,
        &copy_done,
        &invalid,
    );
    instructions.extend([
        abi::move_register(abi::return_register(), &c_path),
        abi::move_immediate(abi::c_arg(1), "Integer", flags.read),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
    ]);
    platform.emit_open_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        // C `int` open fd — sign-extend before the signed compare (bug-04/bug-170).
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ge(&open_ok),
        abi::branch(&open_error),
        abi::label(&open_ok),
        abi::move_register(&fd, abi::return_register()),
        abi::move_immediate(&record_size, "Integer", RESOURCE_RECORD_SIZE),
        abi::move_register(abi::return_register(), &record_size),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&file_alloc_ok),
        // The File-record alloc failed after `open` succeeded: close the fd before
        // reporting OOM so the error path does not leak the OS fd. `fd` is
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
        // bug-574: this `File` record is INTERNAL — it exists only so the shared
        // `readAllBytes` body has a handle to read through, and it is not what
        // this call hands back (that is the `List OF Byte` `readAllBytes` builds).
        // `fs::openFile`'s record IS the return value and is NOT freed here.
        abi::move_register(&record, abi::mfb_return(1)),
        // Canonical plan-80 header: tag@0 (x0 is dead after the alloc-ok compare).
        abi::move_immediate(abi::return_register(), "Integer", RESOURCE_TAG_FILE),
        abi::store_u64(
            abi::return_register(),
            abi::mfb_return(1),
            RESOURCE_OFFSET_TAG,
        ),
        abi::store_u64(&fd, abi::mfb_return(1), FILE_OFFSET_FD),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), FILE_OFFSET_CLOSED),
        // bug-755: a fresh record is unowned until a binding claims it.
        abi::store_u64(abi::ZERO, abi::mfb_return(1), RESOURCE_OFFSET_OWNER),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), FILE_OFFSET_STATE),
        // Opt-in per-File output buffer (plan-14-B): a fresh handle is unbuffered.
        // Arena memory is not guaranteed zero (a reused chunk keeps its free-list node
        // words), so zero the buffer fields explicitly.
        abi::store_u64(abi::ZERO, abi::mfb_return(1), FILE_OFFSET_BUF_PTR),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), FILE_OFFSET_BUF_FILLED),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), FILE_OFFSET_BUF_ENABLED),
        // Transparent read buffer: empty cache at the fd's position. `readAllBytes`
        // never fills it (it is the whole-file path), so the record owns no second
        // block when it is released at `done`.
        abi::store_u64(abi::ZERO, abi::mfb_return(1), FILE_OFFSET_READ_PTR),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), FILE_OFFSET_READ_POS),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), FILE_OFFSET_READ_FILL),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), FILE_OFFSET_READ_AT_EOF),
        abi::move_register(abi::return_register(), abi::mfb_return(1)),
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
    let restore = format!("{symbol}_restore");
    let directory = format!("{symbol}_directory");
    if platform.family() != PlatformFamily::Windows {
        // bug-701: a POSIX read-only open of a directory succeeds, so the read
        // through it is what fails (`EISDIR`, or on Linux ext4 a measured size no
        // allocation can satisfy). Report that as the directory it is, as Windows
        // does by refusing the open.
        instructions.extend([
            abi::compare_immediate(&save_tag, RESULT_OK_TAG),
            abi::branch_eq(&restore),
        ]);
        emit_branch_if_directory(
            symbol,
            FsPathArg::String((&path).into()),
            &directory,
            &mut instructions,
            &mut relocations,
        );
    }
    instructions.extend([
        abi::label(&restore),
        abi::move_register(RESULT_TAG_REGISTER, &save_tag),
        abi::move_register(RESULT_VALUE_REGISTER, &save_value),
        abi::move_register(RESULT_ERROR_MESSAGE_REGISTER, &save_message),
        abi::branch(&done),
        abi::label(&directory),
    ]);
    raise_error_into(
        symbol,
        "ErrInvalidPath",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&open_error)]);
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
        FsHostOp::Read,
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
            HelperScratch::new(&record, &record_size),
        ],
        &mut vregs,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::return_());
    Ok((instructions, relocations, 0))
}
