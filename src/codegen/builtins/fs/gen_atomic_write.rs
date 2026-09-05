//! Path-addressed `fs` write/read code generation (writeAll/writeText/writeBytes[+atomic], readText/readBytes).

use super::gen_open::{o_cloexec, open_flag_set};
use super::gen_shared::*;
use crate::codegen::collection::layout::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::memory::data::*;
use crate::codegen::os::syscall::*;
use crate::codegen::string::validate::*;
use crate::target::shared::abi;
use std::collections::HashMap;

/// Narrow a C `int` result in the return register to its true signed 64-bit
/// value. Required before any signed relational compare (`branch_lt`): none of
/// the ABIs we target guarantee the upper 32 bits of an `int` return — AAPCS64
/// and the Darwin arm64 ABI leave `x0[63:32]` unspecified, and x86-64 SysV
/// leaves `rax[63:32]` undefined. When a libc leaves those bits clear, a `-1`
/// (EIO/EBADF/ENOSPC) reads as `+4294967295`, `branch_lt` is not taken, and an
/// `fsync`/`close` durability failure is silently swallowed (bug-04, bug-44).
///
/// This is the named seam for the `fs` atomic helpers, NOT a tree-wide choke
/// point: it is one spelling of the invariant, and the same
/// `sign_extend_word(return_register(), return_register())` pair is written
/// inline at ~50 other `int`-returning wrapper sites (elsewhere in this file and
/// across `fs_helpers_io.rs`, `link_thunk.rs`, `net/`, `tls/`, `audio/`). A new
/// `int`-returning wrapper must therefore apply the extension deliberately —
/// calling this helper will not do it for you.
///
/// `sign_extend_word` lowers per-backend (`sxtw` on aarch64, `sext.w`
/// on riscv64, `movsxd` on x86-64); on riscv64's lp64d ABI the extension is
/// already guaranteed, making the op a semantic no-op there — kept for
/// uniformity so the next backend need not remember it.
fn normalize_c_int_result(instructions: &mut Vec<CodeInstruction>) {
    instructions.push(abi::sign_extend_word(
        abi::return_register(),
        abi::return_register(),
    ));
}

#[derive(Clone, Copy)]
pub(crate) enum AtomicWriteValueKind {
    String,
    Bytes,
}

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
    let mut instructions = vec![
        abi::move_register(&path, abi::return_register()),
        abi::move_register(&value, abi::mfb_return(1)),
        abi::load_u64(&len0, &path, 0),
        abi::compare_immediate(&len0, "0"),
        abi::branch_eq(&invalid),
        abi::add_immediate(abi::return_register(), &len0, 9 + TEMPLATE_SUFFIX.len()),
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
        abi::load_u64(abi::return_register(), &temp_path, 0),
        abi::add_immediate(abi::return_register(), abi::return_register(), 1),
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
        abi::load_u64(abi::return_register(), &path, 0),
        abi::add_immediate(abi::return_register(), abi::return_register(), 1),
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
    // `emit_errno_error_mapping` branches to `done` in every case (including its
    // generic `err_output` tail), so this mkstemps/rename errno path already
    // terminates and cannot fall through into the write/sync close tail below.
    emit_errno_error_mapping(
        symbol,
        &errno_reg,
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
    instructions.extend([abi::label(&done), abi::return_()]);
    Ok((instructions, relocations, 0))
}

pub(crate) fn lower_fs_write_path_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    append: bool,
    bytes: bool,
) -> Result<FsBodyParts, String> {
    // Vreg-allocated (plan-00-G Phase 2). path→C-string, open, write loop, fsync,
    // close. fd (across write/sync/close) and the value (across open) are spilled
    // vregs; the C-string is consumed at open. `bytes` selects the source: a
    // String's inline bytes (`writeText`) or a byte-List's data region
    // (`writeBytes`) — the only difference is the source-pointer/length setup
    // (bug-331 §B).
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

    let flags = open_flag_set(platform.family(), false);
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
    // `cap` is a byte-List-only scratch; allocate its vreg only in that mode so the
    // String path keeps its exact vreg numbering (bug-331 §B).
    let cap = if bytes { vregs.next() } else { String::new() };
    let mut instructions = vec![
        abi::move_register(&path, abi::return_register()),
        abi::move_register(&value, abi::mfb_return(1)),
        abi::load_u64(&len0, &path, 0),
        abi::compare_immediate(&len0, "0"),
        abi::branch_eq(&invalid),
        abi::add_immediate(abi::return_register(), &len0, 1),
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
        abi::move_immediate(abi::c_arg(1), "Integer", mode_flags),
        // Owner-only create mode (0o600 = 384), not world-readable 0o666
        //.
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
        abi::branch_ge(&open_ok),
        abi::branch(&open_error),
        abi::label(&open_ok),
        abi::move_register(&fd, abi::return_register()),
    ]);
    if bytes {
        // Source is a byte-List: length is DATA_LENGTH, the data region starts past
        // the header, and the cursor skips the (capacity * stride) reserved bytes.
        instructions.extend([
            abi::load_u64(&remaining, &value, COLLECTION_OFFSET_DATA_LENGTH),
            abi::add_immediate(&cursor, &value, COLLECTION_HEADER_SIZE),
            abi::load_u64(&cap, &value, COLLECTION_OFFSET_CAPACITY),
            abi::move_immediate(&byte, "Integer", &byte_list_entry_stride().to_string()),
            abi::multiply_registers(&cap, &cap, &byte),
            abi::add_registers(&cursor, &cursor, &cap),
        ]);
    } else {
        // Source is a String: length at offset 0, inline bytes at offset 8.
        instructions.extend([
            abi::load_u64(&remaining, &value, 0),
            abi::add_immediate(&cursor, &value, 8),
        ]);
    }
    instructions.extend([
        abi::label(&write_loop),
        abi::compare_immediate(&remaining, "0"),
        abi::branch_eq(&write_done),
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
    // write_loop before any byte moved rather than treated as a hard ErrOutput
    // (bug-62, matching the File-based write loops in fs_helpers_io.rs).
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
    raise_error_into(
        symbol,
        "ErrWriteFailed",
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
    instructions.extend([abi::branch(&done), abi::label(&close_error)]);
    raise_error_into(
        symbol,
        "ErrWriteFailed",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    Ok((instructions, relocations, 0))
}

pub(crate) fn lower_fs_read_text_path_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<FsBodyParts, String> {
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
    let dst = vregs.next();
    let index = vregs.next();
    let byte = vregs.next();
    let mut instructions = vec![
        abi::move_register(&path, abi::return_register()),
        abi::load_u64(&len0, &path, 0),
        abi::compare_immediate(&len0, "0"),
        abi::branch_eq(&invalid),
        abi::add_immediate(abi::return_register(), &len0, 1),
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
    instructions.extend([abi::label(&seek_error)]);
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
    emit_fs_path_errno_error_mapping(
        symbol,
        &errno_reg,
        platform.family(),
        false,
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
        // Close-free OOM exit: reached from the pre-open C-string alloc failure
        // (fd not yet opened) and, after an inline `close`, from the post-open
        // result-String alloc failure. Closing fd here would close an unassigned
        // vreg on the pre-open path (bug-201).
        abi::label(&alloc_error),
    ]);
    raise_error_into(
        symbol,
        "ErrOutOfMemory",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
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
        abi::move_register(&path, abi::return_register()),
        abi::load_u64(&len0, &path, 0),
        abi::compare_immediate(&len0, "0"),
        abi::branch_eq(&invalid),
        abi::add_immediate(abi::return_register(), &len0, 1),
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
        abi::move_immediate(abi::return_register(), "Integer", RESOURCE_RECORD_SIZE),
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
        // Transparent read buffer: empty cache at the fd's position.
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
        (&errno_reg).into(),
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_fs_path_errno_error_mapping(
        symbol,
        &errno_reg,
        platform.family(),
        false,
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
    Ok((instructions, relocations, 0))
}
