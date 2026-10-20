//! Streaming/whole-`File` read+write `fs` code generation (writeAll/readAll(+Bytes), eof, readLine).

use super::gen_handle::emit_append_to_file_buffer;
use super::gen_shared::*;
use crate::codegen::collection::layout::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::memory::arena::{emit_arena_free, emit_helper_scratch_release, HelperScratch};
use crate::codegen::memory::data::*;
use crate::codegen::os::syscall::*;
use crate::codegen::string::validate::*;
use crate::target::shared::abi;
use crate::target::shared::abi::Reg;
use std::collections::HashMap;

mod line;
mod position;
use line::emit_reconcile_read_buffer;
pub(crate) use line::lower_fs_read_line_helper;
pub(crate) use position::{
    lower_fs_eof_helper, lower_fs_read_bytes_at_helper, lower_fs_seek_helper, lower_fs_size_helper,
};

pub(crate) fn lower_fs_write_all_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<FsBodyParts, String> {
    // Vreg-allocated (plan-00-G Phase 2). fd / remaining / cursor are loop-carried
    // across the `write` syscall, so the allocator spills them.
    let loop_label = format!("{symbol}_write_loop");
    let done_write = format!("{symbol}_write_done");
    let closed = format!("{symbol}_closed");
    let closed_moved = format!("{symbol}_closed_moved");
    let write_error = format!("{symbol}_write_error");
    let done = format!("{symbol}_done");
    let mut vregs = Vregs::new();
    let file = vregs.next();
    let data = vregs.next();
    let fd = vregs.next();
    let remaining = vregs.next();
    let cursor = vregs.next();
    let closed_flag = vregs.next();
    let buf_enabled = vregs.next();
    let mut instructions = vec![
        abi::move_register(&file, abi::return_register()),
        abi::move_register(&data, abi::mfb_return(1)),
        abi::load_u64(&closed_flag, &file, FILE_OFFSET_CLOSED),
    ];
    instructions.extend(crate::codegen::resource::closed_or_moved_guard(
        &closed_flag,
        &closed,
        &closed_moved,
    ));
    let mut relocations = Vec::new();
    // Reconcile the read buffer before writing: on a read+write handle
    // a write after fs::readLine must land at the true fd position, not the block
    // read-ahead. A no-op when nothing was read-buffered.
    emit_reconcile_read_buffer(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        &mut vregs,
        &file,
        "wa",
        &write_error,
    )?;
    instructions.extend([
        abi::load_u64(&fd, &file, FILE_OFFSET_FD),
        abi::load_u64(&remaining, &data, 0),
        abi::add_immediate(&cursor, &data, 8),
        // Opt-in per-File buffering (plan-14-B): when enabled, append the incoming
        // bytes into the handle's buffer instead of writing them straight through.
        // Off (the default) falls into today's unbuffered direct-write loop.
        abi::load_u64(&buf_enabled, &file, FILE_OFFSET_BUF_ENABLED),
        abi::compare_immediate(&buf_enabled, "0"),
        abi::branch_eq(&loop_label),
    ]);
    emit_append_to_file_buffer(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        &mut vregs,
        &file,
        &cursor,
        &remaining,
        "wa",
        &write_error,
    )?;
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&loop_label),
        abi::compare_immediate(&remaining, "0"),
        abi::branch_eq(&done_write),
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
        &loop_label,
        &write_error,
    )?;
    instructions.extend([
        abi::label(&done_write),
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
    instructions.extend([abi::branch(&done), abi::label(&write_error)]);
    raise_error_into(
        symbol,
        "ErrWriteFailed",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    Ok((instructions, relocations, 0))
}

pub(crate) fn lower_fs_read_all_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<FsBodyParts, String> {
    // Vreg-allocated (plan-00-G Phase 2). fd (across the seeks + read loop), the
    // seek positions/length (across the alloc), and the result string (across the
    // read loop + UTF-8 validation) are vregs the allocator spills.
    let closed = format!("{symbol}_closed");
    let closed_moved = format!("{symbol}_closed_moved");
    let seek_error = format!("{symbol}_seek_error");
    let alloc_ok = format!("{symbol}_alloc_ok");
    let alloc_error = format!("{symbol}_alloc_error");
    let read_loop = format!("{symbol}_read_loop");
    let read_done = format!("{symbol}_read_done");
    let read_error = format!("{symbol}_read_error");
    let done = format!("{symbol}_done");

    let mut vregs = Vregs::new();
    let file = vregs.next();
    let fd = vregs.next();
    let start = vregs.next();
    let end = vregs.next();
    let length = vregs.next();
    let string = vregs.next();
    let remaining = vregs.next();
    let cursor = vregs.next();
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
    let mut relocations = Vec::new();
    // Reconcile the read buffer: a whole-file read after fs::readLine
    // must see the true fd position, not the block read-ahead.
    emit_reconcile_read_buffer(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        &mut vregs,
        &file,
        "readall",
        &seek_error,
    )?;
    instructions.extend([
        abi::load_u64(&fd, &file, FILE_OFFSET_FD),
        abi::move_register(abi::return_register(), &fd),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        abi::move_immediate(abi::c_arg(2), "Integer", "1"),
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
        abi::compare_registers(&end, &start),
        abi::branch_lt(&seek_error),
        abi::subtract_registers(&length, &end, &start),
        abi::add_immediate(abi::return_register(), &length, 9),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
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
        abi::load_u64(abi::c_arg(1), &string, 0),
        abi::add_immediate(abi::c_arg(0), &string, 8),
    ]);
    let encoding_error = format!("{symbol}_encoding_error");
    emit_call_validate_utf8(symbol, &encoding_error, &mut instructions, &mut relocations);
    instructions.extend([
        abi::move_register(RESULT_VALUE_REGISTER, &string),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&encoding_error),
    ]);
    raise_error_into(symbol, "ErrEncoding", &mut instructions, &mut relocations);
    instructions.extend([abi::branch(&done), abi::label(&closed)]);
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

pub(crate) fn lower_fs_write_all_bytes_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<FsBodyParts, String> {
    // Vreg-allocated (plan-00-G Phase 2). Writes the byte-List's data region;
    // fd/remaining/cursor are loop-carried across the `write` syscall (spilled).
    let loop_label = format!("{symbol}_write_loop");
    let done_write = format!("{symbol}_write_done");
    let closed = format!("{symbol}_closed");
    let closed_moved = format!("{symbol}_closed_moved");
    let write_error = format!("{symbol}_write_error");
    let done = format!("{symbol}_done");
    let mut vregs = Vregs::new();
    let file = vregs.next();
    let bytes = vregs.next();
    let fd = vregs.next();
    let remaining = vregs.next();
    let cursor = vregs.next();
    let closed_flag = vregs.next();
    let scratch = vregs.next();
    let buf_enabled = vregs.next();
    let entry_size = vregs.next();
    let mut instructions = vec![
        abi::move_register(&file, abi::return_register()),
        abi::move_register(&bytes, abi::mfb_return(1)),
        abi::load_u64(&closed_flag, &file, FILE_OFFSET_CLOSED),
    ];
    instructions.extend(crate::codegen::resource::closed_or_moved_guard(
        &closed_flag,
        &closed,
        &closed_moved,
    ));
    let mut relocations = Vec::new();
    // Reconcile the read buffer before writing (see fs::writeAll).
    emit_reconcile_read_buffer(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        &mut vregs,
        &file,
        "wab",
        &write_error,
    )?;
    instructions.extend([
        abi::load_u64(&fd, &file, FILE_OFFSET_FD),
        abi::load_u64(&remaining, &bytes, COLLECTION_OFFSET_DATA_LENGTH),
        abi::add_immediate(&cursor, &bytes, COLLECTION_HEADER_SIZE),
        abi::load_u64(&scratch, &bytes, COLLECTION_OFFSET_CAPACITY),
        abi::move_immediate(
            &entry_size,
            "Integer",
            &byte_list_entry_stride().to_string(),
        ),
        abi::multiply_registers(&scratch, &scratch, &entry_size),
        abi::add_registers(&cursor, &cursor, &scratch),
        // Opt-in per-File buffering (plan-14-B): append into the handle's buffer
        // when enabled; off falls into today's unbuffered direct-write loop.
        abi::load_u64(&buf_enabled, &file, FILE_OFFSET_BUF_ENABLED),
        abi::compare_immediate(&buf_enabled, "0"),
        abi::branch_eq(&loop_label),
    ]);
    emit_append_to_file_buffer(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        &mut vregs,
        &file,
        &cursor,
        &remaining,
        "wab",
        &write_error,
    )?;
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&loop_label),
        abi::compare_immediate(&remaining, "0"),
        abi::branch_eq(&done_write),
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
        &loop_label,
        &write_error,
    )?;
    instructions.extend([
        abi::label(&done_write),
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
    instructions.extend([abi::branch(&done), abi::label(&write_error)]);
    raise_error_into(
        symbol,
        "ErrWriteFailed",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    Ok((instructions, relocations, 0))
}

pub(crate) fn lower_fs_read_all_bytes_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<FsBodyParts, String> {
    // Vreg-allocated (plan-00-G Phase 2). fd (across seeks + read loop), seek
    // positions/length (across the alloc), the collection and its data-region base
    // (across the read loop) are spilled vregs; the entry-init loop makes no call.
    let closed = format!("{symbol}_closed");
    let closed_moved = format!("{symbol}_closed_moved");
    let seek_error = format!("{symbol}_seek_error");
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
    let fd = vregs.next();
    let start = vregs.next();
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
        abi::load_u64(&closed_flag, &file, FILE_OFFSET_CLOSED),
    ];
    instructions.extend(crate::codegen::resource::closed_or_moved_guard(
        &closed_flag,
        &closed,
        &closed_moved,
    ));
    let mut relocations = Vec::new();
    // Reconcile the read buffer: a whole-file read after fs::readLine
    // must see the true fd position, not the block read-ahead.
    emit_reconcile_read_buffer(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        &mut vregs,
        &file,
        "readall",
        &seek_error,
    )?;
    instructions.extend([
        abi::load_u64(&fd, &file, FILE_OFFSET_FD),
        abi::move_register(abi::return_register(), &fd),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        abi::move_immediate(abi::c_arg(2), "Integer", "1"),
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
        abi::compare_registers(&end, &start),
        abi::branch_lt(&seek_error),
        abi::subtract_registers(&length, &end, &start),
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
        abi::branch(&alloc_error),
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
        // kind 2 has no entry array to fill (plan-57-D). Emitting this with a
        // zero stride would rewrite one entry over the data region `count`
        // times and run past the block, so it is skipped outright.
    ]);
    if byte_list_entry_stride() != 0 {
        instructions.extend([
            abi::move_immediate(&scratch, "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
            abi::store_u8(&scratch, &entry_cursor, COLLECTION_ENTRY_OFFSET_FLAGS),
            abi::store_u64(abi::ZERO, &entry_cursor, COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
            abi::store_u64(abi::ZERO, &entry_cursor, COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
            abi::store_u64(&idx, &entry_cursor, COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
            abi::move_immediate(&scratch, "Integer", "1"),
            abi::store_u64(
                &scratch,
                &entry_cursor,
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ),
            abi::add_immediate(&entry_cursor, &entry_cursor, byte_list_entry_stride()),
        ]);
    }
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
        abi::move_register(RESULT_VALUE_REGISTER, &collection),
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
