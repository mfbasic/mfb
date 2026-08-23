//! `fs::createTempFile` code generation (UUIDv4 name synthesis + O_EXCL create).

use super::gen_shared::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::memory::data::*;
use crate::target::shared::abi;
use std::collections::HashMap;

pub(crate) fn lower_fs_create_temp_file_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<FsBodyParts, String> {
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
        abi::move_register(&dir, abi::return_register()),
        abi::load_u64(&len0, &dir, 0),
        abi::compare_immediate(&len0, "0"),
        abi::branch_eq(&invalid),
        abi::add_immediate(abi::return_register(), &len0, UUID_FILE_EXTRA),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ];
    let mut relocations = vec![internal_branch(symbol, ARENA_ALLOC_SYMBOL)];
    let dir_len = vregs.next();
    let src = vregs.next();
    let index = vregs.next();
    let byte = vregs.next();
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::move_register(&path, abi::mfb_return(1)),
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
        abi::move_immediate(abi::c_arg(1), "Integer", "16"),
    ]);
    platform.emit_random_bytes(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        // C `int` getentropy return (0/-1) — sign-extend before the signed compare
        // so a -1 error isn't read as large-positive success (bug-04/bug-170).
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
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
        abi::move_immediate(
            abi::c_arg(1),
            "Integer",
            temp_file_open_flags(platform.family()),
        ),
        abi::move_immediate(abi::c_arg(2), "Integer", "384"),
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
        abi::branch_ge(&fd_ok),
        abi::branch(&open_error),
        abi::label(&fd_ok),
        abi::move_register(&fd, abi::return_register()),
        abi::move_immediate(abi::return_register(), "Integer", RESOURCE_RECORD_SIZE),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
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
        // Canonical plan-80 header: tag@0 (x0 is dead after the alloc-ok compare).
        abi::move_immediate(abi::return_register(), "Integer", RESOURCE_TAG_FILE),
        abi::store_u64(
            abi::return_register(),
            abi::mfb_return(1),
            RESOURCE_OFFSET_TAG,
        ),
        abi::store_u64(&fd, abi::mfb_return(1), FILE_OFFSET_FD),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), FILE_OFFSET_CLOSED),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), FILE_OFFSET_STATE),
        // Opt-in per-File output buffer (plan-14-B): a fresh handle is unbuffered.
        // Arena memory is poisoned, so zero the buffer fields explicitly.
        abi::store_u64(abi::ZERO, abi::mfb_return(1), FILE_OFFSET_BUF_PTR),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), FILE_OFFSET_BUF_FILLED),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), FILE_OFFSET_BUF_ENABLED),
        // Transparent read buffer (plan-14-C): empty cache at the fd's position.
        abi::store_u64(abi::ZERO, abi::mfb_return(1), FILE_OFFSET_READ_PTR),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), FILE_OFFSET_READ_POS),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), FILE_OFFSET_READ_FILL),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), FILE_OFFSET_READ_AT_EOF),
        abi::move_register(RESULT_VALUE_REGISTER, abi::mfb_return(1)),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
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
    emit_errno_error_mapping(
        symbol,
        &errno_reg,
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
    instructions.extend([abi::label(&done), abi::return_()]);
    Ok((instructions, relocations, RANDOM_BUF_SIZE))
}

fn temp_file_open_flags(family: PlatformFamily) -> &'static str {
    // Linux (any arch) vs macOS — the O_* bit values differ (Linux O_CREAT=0x40,
    // O_EXCL=0x80, O_CLOEXEC=0x80000; macOS O_CREAT=0x200, O_EXCL=0x800,
    // O_CLOEXEC=0x1000000). Matching only "linux-aarch64" gave linux-x86_64 the
    // macOS bits → a wrong open.
    match family {
        PlatformFamily::Linux => "524482",
        // Windows packs `(disposition << 32) | access` (see `open_flag_set`'s
        // Windows arm). A temp file is created exclusively: CREATE_NEW (1) is the
        // O_CREAT|O_EXCL equivalent (CreateFileW fails with ERROR_FILE_EXISTS if the
        // randomized name already exists), with GENERIC_READ|GENERIC_WRITE
        // (0xC0000000) access. (1 << 32) | 0xC0000000 = 7516192768. plan-66-E.
        PlatformFamily::Windows => "7516192768",
        PlatformFamily::MacOS => {
            // O_RDWR|O_CREAT|O_EXCL|O_CLOEXEC = 0x2|0x200|0x800|0x1000000 = 16779778.
            // The temp fd was previously opened without O_CLOEXEC (bug-102).
            //
            // This decimal was 16779266 — the same OR expression, but evaluated
            // without O_CREAT (0x200 = 512), which the comment above already spelled
            // out correctly (bug-309). Opening a freshly generated, non-existent UUID
            // name with O_EXCL and no O_CREAT is an unconditional ENOENT, so
            // `fs::createTempFile()` failed on every macOS build with
            // ERR_PATH_NOT_FOUND. Linux's 524482 was always right.
            "16779778"
        }
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
