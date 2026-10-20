//! plan-176-S: `fs::gen_directory`'s creating directories (`lower_fs_create_directories_helper`). The shared helpers live in
//! `builtins/fs/gen_directory/mod.rs`.

use super::*;

pub(crate) fn lower_fs_create_directories_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<FsBodyParts, String> {
    // Vreg-allocated (plan-00-G Phase 2). The C-string and the scan `cursor` are
    // loop-carried across the per-prefix `mkdir` calls, so the allocator spills
    // them. `errno` stays in the physical register `emit_errno` writes (`x9`) — it
    // is read immediately after, with no call in between. The `stat` buffer that
    // classifies an existing final component is an explicit on-stack local at
    // `sp + 0`, as in `lower_fs_kind_exists_helper`.
    const STAT_OFFSET: usize = 0;
    const STAT_BUF_SIZE: usize = 256;
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
    let err_already_exists = format!("{symbol}_err_already_exists");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let mut vregs = Vregs::new();
    let path = vregs.next();
    let cstring = vregs.next();
    let cstring_size = vregs.next();
    let cursor = vregs.next();
    let len0 = vregs.next();
    let mut instructions = vec![
        // bug-574: `cstring` is the marshalled path this helper never hands back,
        // released at `done`. Nulled FIRST so the two paths that reach `done`
        // without allocating — the empty-path rejection just below and
        // `ErrOutOfMemory` — free nothing.
        abi::move_immediate(&cstring, "Integer", "0"),
        abi::move_immediate(&cstring_size, "Integer", "0"),
        abi::move_register(&path, abi::return_register()),
        abi::load_u64(&len0, &path, 0),
        abi::compare_immediate(&len0, "0"),
        abi::branch_eq(&invalid_path),
        abi::add_immediate(&cstring_size, &len0, 1),
        abi::move_register(abi::return_register(), &cstring_size),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ];
    let mut relocations = vec![internal_branch(symbol, ARENA_ALLOC_SYMBOL)];
    let len = vregs.next();
    let src = vregs.next();
    let dst = vregs.next();
    let index = vregs.next();
    let byte = vregs.next();
    let sep = vregs.next();
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::move_register(&cstring, abi::mfb_return(1)),
        abi::load_u64(&len, &path, 0),
        abi::add_immediate(&src, &path, 8),
        abi::move_register(&dst, &cstring),
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
        &invalid_path,
    );
    // plan-157-C: Windows splits on `\` as well as `/`, never tries to create
    // the drive (`C:`) or UNC share (`\\server\share`) prefix, reads "already
    // exists" as `ERROR_ALREADY_EXISTS` (183, not POSIX `EEXIST` 17 —
    // `emit_errno` is raw `GetLastError` there), and puts back the separator it
    // cut at rather than always writing `/`. Before this, any Windows path that
    // needed a parent created failed with `ErrWriteFailed`. POSIX is unchanged.
    let windows = platform.family() == PlatformFamily::Windows;
    if windows {
        let components = vregs.next();
        emit_windows_create_directories_start(
            symbol,
            &cstring,
            &cursor,
            &byte,
            &components,
            &mut instructions,
        );
    } else {
        instructions.extend([
            abi::move_register(&cursor, &cstring),
            abi::load_u8(&byte, &cstring, 0),
            abi::compare_immediate(&byte, "47"),
            abi::branch_ne(&scan_loop),
            abi::add_immediate(&cursor, &cursor, 1),
        ]);
    }
    instructions.extend([
        abi::label(&scan_loop),
        abi::load_u8(&byte, &cursor, 0),
        abi::compare_immediate(&byte, "0"),
        abi::branch_eq(&final_mkdir),
        abi::compare_immediate(&byte, "47"),
        abi::branch_eq(&mkdir_prefix),
    ]);
    if windows {
        instructions.extend([
            abi::compare_immediate(&byte, "92"), // '\'
            abi::branch_eq(&mkdir_prefix),
        ]);
    }
    instructions.extend([
        abi::add_immediate(&cursor, &cursor, 1),
        abi::branch(&scan_loop),
        abi::label(&mkdir_prefix),
    ]);
    if windows {
        instructions.push(abi::move_register(&sep, &byte));
    }
    instructions.extend([
        abi::store_u8(abi::ZERO, &cursor, 0),
        abi::move_register(abi::return_register(), &cstring),
    ]);
    platform.emit_fs_path_operation(
        symbol,
        FsPathOperation::Mkdir,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    if !windows {
        instructions.push(abi::move_immediate(&sep, "Integer", "47"));
    }
    instructions.extend([
        abi::store_u8(&sep, &cursor, 0),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&prefix_ok),
    ]);
    // "Already exists": POSIX `EEXIST`, Windows `ERROR_ALREADY_EXISTS`.
    let eexist = if windows { "183" } else { "17" };
    let errno_reg = vregs.next();
    platform.emit_errno(
        symbol,
        (&errno_reg).into(),
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(&errno_reg, eexist),
        abi::branch_ne(&call_error),
        abi::label(&prefix_ok),
        abi::add_immediate(&cursor, &cursor, 1),
        abi::branch(&scan_loop),
        abi::label(&final_mkdir),
        abi::move_register(abi::return_register(), &cstring),
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
        (&errno_reg).into(),
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    // bug-694 C5: an existing final component is only a success when it is a
    // directory. Anything else already at `path` — a regular file — is refused
    // with ErrAlreadyExists, as `fs::createDirectory` refuses it, instead of
    // reporting a directory that is not there.
    let mode = vregs.next();
    let mask = vregs.next();
    let expected = vregs.next();
    instructions.extend([
        abi::compare_immediate(&errno_reg, eexist),
        abi::branch_ne(&call_error),
        abi::move_register(abi::return_register(), &cstring),
        abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), STAT_OFFSET),
    ]);
    platform.emit_path_stat(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    platform.emit_stat_is_kind(
        STAT_OFFSET,
        FS_MODE_DIRECTORY,
        mode.into(),
        mask.into(),
        expected.into(),
        &final_ok,
        &err_already_exists,
        &mut instructions,
    );
    instructions.extend([
        // `emit_stat_is_kind` falls through when the entry is some other kind.
        abi::branch(&err_already_exists),
        abi::label(&final_ok),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&call_error),
    ]);
    // bug-701: the same mapping `fs::createDirectory` uses. A component that is a
    // regular file answers `ENOTDIR` on POSIX and `ERROR_PATH_NOT_FOUND` on
    // Windows, which the mapper classifies — both `ErrInvalidPath`.
    emit_fs_host_error_mapping(
        symbol,
        &errno_reg,
        platform.family(),
        FsHostOp::Mkdir,
        false,
        Some(FsPathArg::String((&path).into())),
        FsFallback::Write,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&invalid_path));
    raise_error_into(
        symbol,
        "ErrInvalidArgument",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&err_already_exists)]);
    raise_error_into(
        symbol,
        "ErrAlreadyExists",
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
        &[HelperScratch::new(&cstring, &cstring_size)],
        &mut vregs,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::return_());

    Ok((instructions, relocations, STAT_BUF_SIZE))
}

/// plan-157-C: where a Windows `createDirectories` walk starts — past a drive
/// (`C:`, then one separator), past a UNC share (`\\server\share`, then one
/// separator), or past one leading separator — so no prefix that cannot be
/// created (`C:`, `\\server`) is ever handed to `CreateDirectoryW`. Leaves
/// `cursor` at the first byte to scan. `components` is a scratch vreg.
fn emit_windows_create_directories_start(
    symbol: &str,
    cstring: impl Into<Reg>,
    cursor: impl Into<Reg>,
    byte: impl Into<Reg>,
    components: impl Into<Reg>,
    instructions: &mut Vec<CodeInstruction>,
) {
    let cstring: Reg = cstring.into();
    let (cursor, byte, components): (Reg, Reg, Reg) =
        (cursor.into(), byte.into(), components.into());
    let not_drive = format!("{symbol}_win_not_drive");
    let unc = format!("{symbol}_win_unc");
    let skip_sep = format!("{symbol}_win_skip_sep");
    let started = format!("{symbol}_win_started");
    let share_loop = format!("{symbol}_win_unc_loop");
    let share_next = format!("{symbol}_win_unc_next");
    let is_sep = |target: &str| {
        [
            abi::compare_immediate(byte, "47"),
            abi::branch_eq(target),
            abi::compare_immediate(byte, "92"),
            abi::branch_eq(target),
        ]
    };
    instructions.extend([
        abi::move_register(cursor, cstring),
        // `X:` — a drive letter then a colon.
        abi::load_u8(byte, cstring, 0),
        abi::compare_immediate(byte, "0"),
        abi::branch_eq(&started),
        abi::load_u8(byte, cstring, 1),
        abi::compare_immediate(byte, "58"), // ':'
        abi::branch_ne(&not_drive),
        abi::add_immediate(cursor, cursor, 2),
        abi::branch(&skip_sep),
        abi::label(&not_drive),
        // `\\server\share` — two separators, then two components.
        abi::load_u8(byte, cstring, 0),
    ]);
    let step_sep = format!("{symbol}_win_step_sep");
    let sep0 = format!("{symbol}_win_sep0");
    instructions.extend(is_sep(&sep0));
    instructions.extend([
        abi::branch(&started),
        abi::label(&sep0),
        abi::load_u8(byte, cstring, 1),
    ]);
    instructions.extend(is_sep(&unc));
    instructions.extend([
        // One leading separator only: skip it, as POSIX skips `/`.
        abi::add_immediate(cursor, cursor, 1),
        abi::branch(&started),
        abi::label(&unc),
        abi::add_immediate(cursor, cursor, 2),
        abi::move_immediate(components, "Integer", "2"),
        abi::label(&share_loop),
        abi::load_u8(byte, cursor, 0),
        abi::compare_immediate(byte, "0"),
        abi::branch_eq(&started),
    ]);
    instructions.extend(is_sep(&share_next));
    instructions.extend([
        abi::add_immediate(cursor, cursor, 1),
        abi::branch(&share_loop),
        abi::label(&share_next),
        abi::subtract_immediate(components, components, 1),
        abi::compare_immediate(components, "0"),
        abi::branch_eq(&skip_sep),
        abi::add_immediate(cursor, cursor, 1),
        abi::branch(&share_loop),
        // Past the drive or share: step over one separator if there is one.
        abi::label(&skip_sep),
        abi::load_u8(byte, cursor, 0),
    ]);
    instructions.extend(is_sep(&step_sep));
    instructions.extend([
        abi::branch(&started),
        abi::label(&step_sep),
        abi::add_immediate(cursor, cursor, 1),
        abi::label(&started),
    ]);
}
