//! Path-addressed `fs` write/read code generation (writeAll/writeText/writeBytes[+atomic], readText/readBytes).

use super::gen_open::{o_cloexec, open_flag_set};
use super::gen_shared::*;
use crate::codegen::collection::layout::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::memory::arena::{emit_helper_scratch_release, HelperScratch};
use crate::codegen::memory::data::*;
use crate::codegen::os::syscall::*;
use crate::codegen::string::validate::*;
use crate::target::shared::abi;
use std::collections::HashMap;

mod atomic;
mod read_path;
pub(crate) use atomic::lower_fs_atomic_write_helper;
pub(crate) use read_path::{lower_fs_read_bytes_path_helper, lower_fs_read_text_path_helper};

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
/// across `fs_helpers_io.rs`, `link_thunk/`, `net/`, `tls/`, `audio/`). A new
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
    let cap = bytes.then(|| vregs.next());
    let c_path_size = vregs.next();
    let mut instructions = vec![
        // bug-574: `c_path` is the marshalled path this helper never hands back,
        // released at `done`. Nulled FIRST so the paths that reach `done` without
        // allocating — the empty-path rejection just below and `alloc_error` —
        // free nothing.
        abi::move_immediate(&c_path, "Integer", "0"),
        abi::move_immediate(&c_path_size, "Integer", "0"),
        abi::move_register(&path, abi::return_register()),
        abi::move_register(&value, abi::mfb_return(1)),
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
    if let Some(cap) = cap {
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
    emit_fs_host_error_mapping(
        symbol,
        &errno_reg,
        platform.family(),
        FsHostOp::Write,
        false,
        Some(FsPathArg::String((&path).into())),
        FsFallback::Write,
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
    instructions.push(abi::label(&done));
    emit_helper_scratch_release(
        symbol,
        &[HelperScratch::new(&c_path, &c_path_size)],
        &mut vregs,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::return_());
    Ok((instructions, relocations, 0))
}
