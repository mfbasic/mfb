//! `fs` directory + cwd code generation (currentDirectory/tempDirectory/setCurrentDirectory/createDirectory/deleteDirectory/deleteFile/createDirectories/listDirectory).

use super::gen_shared::*;
use crate::codegen::collection::sort::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::memory::arena::{emit_helper_scratch_release, HelperScratch};
use crate::codegen::memory::data::*;
use crate::target::shared::abi;
use crate::target::shared::abi::Reg;
use std::collections::HashMap;

mod create;
mod list;
pub(crate) use create::lower_fs_create_directories_helper;
pub(crate) use list::lower_fs_list_directory_helper;

pub(crate) fn lower_fs_current_directory_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<FsBodyParts, String> {
    // Vreg-allocated (plan-00-G Phase 2). The `getcwd` buffer is arena-allocated
    // (not on-stack); the buffer pointer and the measured length are held across
    // the second `arena_alloc`, so as vregs the allocator keeps them in callee-saved
    // registers / spills them, replacing the old BUFFER_OFFSET/LENGTH_OFFSET slots.
    const GETCWD_CAPACITY: &str = "4096";

    let temp_alloc_ok = format!("{symbol}_temp_alloc_ok");
    let string_alloc_ok = format!("{symbol}_string_alloc_ok");
    let count_loop = format!("{symbol}_count_loop");
    let count_done = format!("{symbol}_count_done");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let read_error = format!("{symbol}_read_error");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let mut vregs = Vregs::new();
    let buffer = vregs.next();
    let buffer_size = vregs.next();
    let length = vregs.next();
    let mut instructions = vec![
        // bug-574: `buffer` is the host call's scratch, this helper's own and
        // never handed back; it is released at `done`. Nulled FIRST so a path
        // that reaches `done` without allocating frees nothing — a runtime
        // pointer guard, not a whole-program proof.
        abi::move_immediate(&buffer, "Integer", "0"),
        abi::move_immediate(&buffer_size, "Integer", "0"),
        abi::move_immediate(&buffer_size, "Integer", GETCWD_CAPACITY),
        abi::move_register(abi::return_register(), &buffer_size),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ];
    let mut relocations = vec![internal_branch(symbol, ARENA_ALLOC_SYMBOL)];
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&temp_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&temp_alloc_ok),
        abi::move_register(&buffer, abi::mfb_return(1)),
        abi::move_register(abi::return_register(), abi::mfb_return(1)),
        abi::move_immediate(abi::c_arg(1), "Integer", GETCWD_CAPACITY),
    ]);
    platform.emit_current_directory(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    let cursor = vregs.next();
    let byte = vregs.next();
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(&read_error),
        abi::move_register(&cursor, &buffer),
        abi::move_immediate(&length, "Integer", "0"),
        abi::label(&count_loop),
        abi::load_u8(&byte, &cursor, 0),
        abi::compare_immediate(&byte, "0"),
        abi::branch_eq(&count_done),
        abi::add_immediate(&cursor, &cursor, 1),
        abi::add_immediate(&length, &length, 1),
        abi::branch(&count_loop),
        abi::label(&count_done),
        abi::add_immediate(abi::return_register(), &length, 9),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    let src = vregs.next();
    let dst = vregs.next();
    let index = vregs.next();
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&string_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&string_alloc_ok),
        abi::store_u64(&length, abi::mfb_return(1), 0),
        abi::move_register(&src, &buffer),
        abi::add_immediate(&dst, abi::mfb_return(1), 8),
        abi::move_immediate(&index, "Integer", "0"),
    ]);
    emit_cstring_copy(
        &mut instructions,
        false,
        &length,
        &src,
        &dst,
        &index,
        &byte,
        &copy_loop,
        &copy_done,
        &copy_done,
    );
    instructions.extend([
        abi::move_register(RESULT_VALUE_REGISTER, abi::mfb_return(1)),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
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
    instructions.push(abi::label(&done));
    emit_helper_scratch_release(
        symbol,
        &[HelperScratch::new(&buffer, &buffer_size)],
        &mut vregs,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::return_());

    Ok((instructions, relocations, 0))
}

pub(crate) fn lower_fs_temp_directory_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<FsBodyParts, String> {
    // Vreg-allocated (plan-00-G Phase 2). The temp-dir path is read into an
    // arena buffer (not on-stack); the buffer pointer and length are held across
    // the second `arena_alloc` as vregs (allocator spills / callee-saves them).
    const TEMP_CAPACITY: &str = "4096";

    let temp_alloc_ok = format!("{symbol}_temp_alloc_ok");
    let string_alloc_ok = format!("{symbol}_string_alloc_ok");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let read_error = format!("{symbol}_read_error");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let mut vregs = Vregs::new();
    let buffer = vregs.next();
    let buffer_size = vregs.next();
    let length = vregs.next();
    let mut instructions = vec![
        // bug-574: `buffer` is the host call's scratch, this helper's own and
        // never handed back; it is released at `done`. Nulled FIRST so a path
        // that reaches `done` without allocating frees nothing — a runtime
        // pointer guard, not a whole-program proof.
        abi::move_immediate(&buffer, "Integer", "0"),
        abi::move_immediate(&buffer_size, "Integer", "0"),
        abi::move_immediate(&buffer_size, "Integer", TEMP_CAPACITY),
        abi::move_register(abi::return_register(), &buffer_size),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ];
    let mut relocations = vec![internal_branch(symbol, ARENA_ALLOC_SYMBOL)];
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&temp_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&temp_alloc_ok),
        abi::move_register(&buffer, abi::mfb_return(1)),
        abi::move_register(abi::return_register(), abi::mfb_return(1)),
        abi::move_immediate(abi::c_arg(1), "Integer", TEMP_CAPACITY),
    ]);
    platform.emit_temp_directory(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    // sec-03: clamp the platform hook's returned length to the buffer capacity
    // before it drives the String allocation and copy loop. macOS
    // `emit_temp_directory` forwards `confstr`'s return, which per its contract is
    // the size *required* to hold the full path and can exceed the 4096 buffer on
    // truncation; an unclamped value would read past the fixed buffer. Clamping in
    // the shared caller bounds every present and future platform hook with one
    // guard (the Linux hook already clamps internally, so this is a no-op there).
    let temp_len_clamped = format!("{symbol}_temp_len_clamped");
    instructions.extend([
        abi::compare_immediate(abi::return_register(), TEMP_CAPACITY),
        abi::branch_le(&temp_len_clamped),
        abi::move_immediate(abi::return_register(), "Integer", TEMP_CAPACITY),
        abi::label(&temp_len_clamped),
    ]);
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(&read_error),
        abi::move_register(&length, abi::return_register()),
        abi::add_immediate(abi::return_register(), abi::return_register(), 9),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    let src = vregs.next();
    let dst = vregs.next();
    let index = vregs.next();
    let byte = vregs.next();
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&string_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&string_alloc_ok),
        abi::store_u64(&length, abi::mfb_return(1), 0),
        abi::move_register(&src, &buffer),
        abi::add_immediate(&dst, abi::mfb_return(1), 8),
        abi::move_immediate(&index, "Integer", "0"),
    ]);
    emit_cstring_copy(
        &mut instructions,
        false,
        &length,
        &src,
        &dst,
        &index,
        &byte,
        &copy_loop,
        &copy_done,
        &copy_done,
    );
    instructions.extend([
        abi::move_register(RESULT_VALUE_REGISTER, abi::mfb_return(1)),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
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
    instructions.push(abi::label(&done));
    emit_helper_scratch_release(
        symbol,
        &[HelperScratch::new(&buffer, &buffer_size)],
        &mut vregs,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::return_());
    // `platform.emit_temp_directory` above may park values in `sp + 0 ..
    // TEMP_DIRECTORY_SCRATCH_BYTES` across its environment lookup, so that window
    // has to be reserved here rather than left to overlap the spill area — or, as
    // in bug-360, the caller's frame.
    Ok((instructions, relocations, TEMP_DIRECTORY_SCRATCH_BYTES))
}

pub(crate) fn lower_fs_path_operation_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    operation: FsPathOperation,
) -> Result<FsBodyParts, String> {
    // Vreg-allocated (plan-00-G Phase 2). The path pointer is held across the
    // `arena_alloc` (spilled); the C-string is consumed by the syscall before any
    // later call, so it stays in a register.
    let alloc_ok = format!("{symbol}_alloc_ok");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let invalid_path = format!("{symbol}_invalid_path");
    let call_error = format!("{symbol}_call_error");
    let done = format!("{symbol}_done");

    let mut vregs = Vregs::new();
    let path = vregs.next();
    let alloc = vregs.next();
    let alloc_size = vregs.next();
    let len0 = vregs.next();
    let mut instructions = vec![
        // bug-574: `alloc` is the marshalled path this helper never hands back,
        // released at `done`. Nulled FIRST so the two paths that reach `done`
        // without allocating — the empty-path rejection just below and
        // `ErrOutOfMemory` — free nothing.
        abi::move_immediate(&alloc, "Integer", "0"),
        abi::move_immediate(&alloc_size, "Integer", "0"),
        abi::move_register(&path, abi::return_register()),
        abi::load_u64(&len0, &path, 0),
        abi::compare_immediate(&len0, "0"),
        abi::branch_eq(&invalid_path),
        abi::add_immediate(&alloc_size, &len0, 1),
        abi::move_register(abi::return_register(), &alloc_size),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ];
    let mut relocations = vec![internal_branch(symbol, ARENA_ALLOC_SYMBOL)];
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
    ]);
    raise_error_into(
        symbol,
        "ErrOutOfMemory",
        &mut instructions,
        &mut relocations,
    );
    let len = vregs.next();
    let src = vregs.next();
    let dst = vregs.next();
    let index = vregs.next();
    let byte = vregs.next();
    instructions.extend([
        abi::branch(&done),
        abi::label(&alloc_ok),
        abi::move_register(&alloc, abi::mfb_return(1)),
        abi::load_u64(&len, &path, 0),
        abi::add_immediate(&src, &path, 8),
        abi::move_register(&dst, &alloc),
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
    instructions.extend([abi::move_register(abi::return_register(), &alloc)]);
    platform.emit_fs_path_operation(
        symbol,
        operation,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&call_error),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&call_error),
    ]);
    let errno_reg = vregs.next();
    platform.emit_errno(
        symbol,
        (&errno_reg).into(),
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    let (op, fallback) = match operation {
        FsPathOperation::Chdir => (FsHostOp::Chdir, FsFallback::Read),
        FsPathOperation::Unlink => (FsHostOp::Unlink, FsFallback::Write),
        FsPathOperation::Mkdir => (FsHostOp::Mkdir, FsFallback::Write),
        FsPathOperation::Rmdir => (FsHostOp::Rmdir, FsFallback::Write),
    };
    emit_fs_host_error_mapping(
        symbol,
        &errno_reg,
        platform.family(),
        op,
        false,
        Some(FsPathArg::String((&path).into())),
        fallback,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&invalid_path)]);
    raise_error_into(
        symbol,
        "ErrInvalidArgument",
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::label(&done));
    emit_helper_scratch_release(
        symbol,
        &[HelperScratch::new(&alloc, &alloc_size)],
        &mut vregs,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::return_());

    Ok((instructions, relocations, 0))
}
