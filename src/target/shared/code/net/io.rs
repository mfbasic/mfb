//! Native code generation for the `net` package socket IO helpers: accept,
//! local/remote address, read/write, DNS lookup, and the UDP datagram
//! operations (bind/receive/send). Each `lower_net_*_helper` emits a
//! self-contained AArch64 runtime function returning the standard
//! `(tag, value)` result in `x0`/`x1`. See the parent module for the shared
//! emitters and record-layout invariants.

use std::collections::HashMap;

use super::*;

// ---------------------------------------------------------------------------
// net.accept
// ---------------------------------------------------------------------------

pub(in crate::target::shared::code) fn lower_net_accept_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<
    (
        CodeFrame,
        Vec<CodeInstruction>,
        Vec<CodeRelocation>,
        Vec<CodeStackSlot>,
    ),
    String,
> {
    const FRAME_SIZE: usize = 64;
    const FD_OFFSET: usize = 8;
    const TIMEOUT_OFFSET: usize = 16;

    let closed = format!("{symbol}_closed");
    let accept_fail = format!("{symbol}_accept_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::ARG[1], abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::load_u64("%v9", abi::return_register(), FILE_OFFSET_CLOSED),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&closed),
        abi::load_u64("%v9", abi::return_register(), FILE_OFFSET_FD),
        abi::store_u64("%v9", abi::stack_pointer(), FD_OFFSET),
        // accept(fd, NULL, NULL)
        abi::move_register(abi::return_register(), "%v9"),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
        abi::move_immediate(abi::ARG[2], "Integer", "0"),
    ]);
    platform.emit_libc_call(
        "accept",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&accept_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
    ]);
    emit_make_handle(
        symbol,
        FD_OFFSET,
        &mut instructions,
        &mut relocations,
        &alloc_fail,
    );
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&accept_fail),
    ]);
    emit_fail(
        symbol,
        ERR_NETWORK_FAILED_CODE,
        ERR_NETWORK_FAILED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&closed));
    emit_fail(
        symbol,
        ERR_RESOURCE_CLOSED_CODE,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&alloc_fail));
    emit_fail(
        symbol,
        ERR_OUT_OF_MEMORY_CODE,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    {
        let (frame, stack_slots) =
            finalize_vreg_body_with_locals(&mut instructions, &[], FRAME_SIZE);
        Ok((frame, instructions, relocations, stack_slots))
    }
}

// ---------------------------------------------------------------------------
// net.localAddress / net.remoteAddress
// ---------------------------------------------------------------------------

pub(in crate::target::shared::code) fn lower_net_address_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    remote: bool,
) -> Result<
    (
        CodeFrame,
        Vec<CodeInstruction>,
        Vec<CodeRelocation>,
        Vec<CodeStackSlot>,
    ),
    String,
> {
    const FRAME_SIZE: usize = 224;
    const FD_OFFSET: usize = 8;
    const LEN_OFFSET: usize = 16;
    const DST_OFFSET: usize = 24;
    const HOST_OFFSET: usize = 32;
    const SADDR_PTR_OFFSET: usize = 40;
    const HOSTLEN_OFFSET: usize = 48;
    const ADDR_OFFSET: usize = 64; // 64..192 sockaddr_storage

    let closed = format!("{symbol}_closed");
    let name_fail = format!("{symbol}_name_fail");
    let addr_fail = format!("{symbol}_addr_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::load_u64("%v9", abi::return_register(), FILE_OFFSET_CLOSED),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&closed),
        abi::load_u64("%v9", abi::return_register(), FILE_OFFSET_FD),
        abi::store_u64("%v9", abi::stack_pointer(), FD_OFFSET),
        abi::move_immediate("%v10", "Integer", &SOCKADDR_STORAGE_SIZE.to_string()),
        abi::store_u64("%v10", abi::stack_pointer(), LEN_OFFSET),
        abi::move_register(abi::return_register(), "%v9"),
        abi::add_immediate(abi::ARG[1], abi::stack_pointer(), ADDR_OFFSET),
        abi::add_immediate(abi::ARG[2], abi::stack_pointer(), LEN_OFFSET),
    ]);
    let call = if remote { "getpeername" } else { "getsockname" };
    platform.emit_libc_call(
        call,
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&name_fail),
        abi::add_immediate("%v9", abi::stack_pointer(), ADDR_OFFSET),
        abi::store_u64("%v9", abi::stack_pointer(), SADDR_PTR_OFFSET),
    ]);
    emit_address_from_sockaddr(
        symbol,
        "addr",
        SADDR_PTR_OFFSET,
        HOSTLEN_OFFSET,
        DST_OFFSET,
        HOST_OFFSET,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
        &alloc_fail,
        &addr_fail,
    )?;
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&name_fail),
    ]);
    emit_fail(
        symbol,
        ERR_RESOURCE_CLOSED_CODE,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&addr_fail));
    emit_fail(
        symbol,
        ERR_ADDRESS_INVALID_CODE,
        ERR_ADDRESS_INVALID_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&closed));
    emit_fail(
        symbol,
        ERR_RESOURCE_CLOSED_CODE,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&alloc_fail));
    emit_fail(
        symbol,
        ERR_OUT_OF_MEMORY_CODE,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    {
        let (frame, stack_slots) =
            finalize_vreg_body_with_locals(&mut instructions, &[], FRAME_SIZE);
        Ok((frame, instructions, relocations, stack_slots))
    }
}

// ---------------------------------------------------------------------------
// net.read / net.readText
// ---------------------------------------------------------------------------

pub(in crate::target::shared::code) fn lower_net_read_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    text: bool,
) -> Result<
    (
        CodeFrame,
        Vec<CodeInstruction>,
        Vec<CodeRelocation>,
        Vec<CodeStackSlot>,
    ),
    String,
> {
    const FRAME_SIZE: usize = 96;
    const FD_OFFSET: usize = 8;
    const MAX_OFFSET: usize = 16;
    const BUF_OFFSET: usize = 24;
    const N_OFFSET: usize = 32;
    const STR_OFFSET: usize = 40;

    let closed = format!("{symbol}_closed");
    let invalid = format!("{symbol}_invalid");
    let peer_closed = format!("{symbol}_peer_closed");
    let read_fail = format!("{symbol}_read_fail");
    let timeout = format!("{symbol}_timeout");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let encoding_error = format!("{symbol}_encoding_error");
    let build_list = format!("{symbol}_build_list");
    let entry_loop = format!("{symbol}_entry_loop");
    let entry_done = format!("{symbol}_entry_done");
    let str_copy = format!("{symbol}_str_copy");
    let str_done = format!("{symbol}_str_done");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::ARG[1], abi::stack_pointer(), MAX_OFFSET),
        abi::load_u64("%v9", abi::return_register(), FILE_OFFSET_CLOSED),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&closed),
        abi::load_u64("%v9", abi::return_register(), FILE_OFFSET_FD),
        abi::store_u64("%v9", abi::stack_pointer(), FD_OFFSET),
        abi::load_u64("%v10", abi::stack_pointer(), MAX_OFFSET),
        abi::compare_immediate("%v10", "0"),
        abi::branch_le(&invalid),
        // Allocate a temporary read buffer of maxBytes.
        abi::move_register(abi::return_register(), "%v10"),
        abi::move_immediate(abi::ARG[1], "Integer", "1"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::store_u64(abi::RET[1], abi::stack_pointer(), BUF_OFFSET),
        // read(fd, buf, maxBytes)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), BUF_OFFSET),
        abi::load_u64(abi::ARG[2], abi::stack_pointer(), MAX_OFFSET),
    ]);
    platform.emit_read_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&peer_closed),
        abi::branch_lt(&read_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), N_OFFSET),
    ]);
    if text {
        // Build a String: [u64 len][bytes][nul], validate UTF-8.
        instructions.extend([
            abi::load_u64("%v10", abi::stack_pointer(), N_OFFSET),
            abi::add_immediate(abi::return_register(), "%v10", 9),
            abi::move_immediate(abi::ARG[1], "Integer", "8"),
        ]);
        emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
        instructions.extend([
            abi::move_register("%v15", abi::RET[1]), // alloc result -> vreg base (plan-34-B Phase 3)
            abi::load_u64("%v10", abi::stack_pointer(), N_OFFSET),
            abi::store_u64("%v10", "%v15", 0),
            abi::load_u64("%v11", abi::stack_pointer(), BUF_OFFSET),
            abi::add_immediate("%v12", "%v15", 8),
            abi::move_immediate("%v13", "Integer", "0"),
            abi::store_u64("%v15", abi::stack_pointer(), STR_OFFSET),
            abi::label(&str_copy),
            abi::compare_registers("%v13", "%v10"),
            abi::branch_eq(&str_done),
            abi::load_u8("%v14", "%v11", 0),
            abi::store_u8("%v14", "%v12", 0),
            abi::add_immediate("%v11", "%v11", 1),
            abi::add_immediate("%v12", "%v12", 1),
            abi::add_immediate("%v13", "%v13", 1),
            abi::branch(&str_copy),
            abi::label(&str_done),
            abi::store_u8(abi::ZERO, "%v12", 0),
            // validate_utf8(bytes, len)
            abi::load_u64("%v9", abi::stack_pointer(), STR_OFFSET),
            abi::add_immediate(abi::return_register(), "%v9", 8),
            abi::load_u64(abi::ARG[1], "%v9", 0),
        ]);
        emit_call_validate_utf8(symbol, &encoding_error, &mut instructions, &mut relocations);
        instructions.extend([
            abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), STR_OFFSET),
            abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
            abi::branch(&done),
            abi::label(&encoding_error),
        ]);
        emit_fail(
            symbol,
            ERR_ENCODING_CODE,
            ERR_ENCODING_SYMBOL,
            &mut instructions,
            &mut relocations,
            &done,
        );
    } else {
        // Build a List OF Byte with N elements.
        instructions.extend([
            abi::label(&build_list),
            abi::load_u64("%v10", abi::stack_pointer(), N_OFFSET),
            abi::move_immediate("%v11", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
            abi::multiply_registers("%v12", "%v10", "%v11"),
            abi::add_immediate("%v12", "%v12", COLLECTION_HEADER_SIZE),
            abi::add_registers(abi::return_register(), "%v12", "%v10"),
            abi::move_immediate(abi::ARG[1], "Integer", "8"),
        ]);
        emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
        instructions.extend([
            abi::move_register("%v15", abi::RET[1]), // alloc result -> vreg base (plan-34-B Phase 3)
            abi::move_immediate("%v9", "Byte", &COLLECTION_KIND_LIST.to_string()),
            abi::store_u8("%v9", "%v15", COLLECTION_OFFSET_KIND),
            abi::move_immediate("%v9", "Byte", &COLLECTION_TYPE_NONE.to_string()),
            abi::store_u8("%v9", "%v15", COLLECTION_OFFSET_KEY_TYPE),
            abi::move_immediate("%v9", "Byte", &COLLECTION_TYPE_BYTE.to_string()),
            abi::store_u8("%v9", "%v15", COLLECTION_OFFSET_VALUE_TYPE),
            abi::move_immediate("%v9", "Byte", "1"),
            abi::store_u8("%v9", "%v15", COLLECTION_OFFSET_FLAGS_VERSION),
            abi::load_u64("%v10", abi::stack_pointer(), N_OFFSET),
            abi::store_u64("%v10", "%v15", COLLECTION_OFFSET_COUNT),
            abi::store_u64("%v10", "%v15", COLLECTION_OFFSET_CAPACITY),
            abi::store_u64("%v10", "%v15", COLLECTION_OFFSET_DATA_LENGTH),
            abi::store_u64("%v10", "%v15", COLLECTION_OFFSET_DATA_CAPACITY),
            abi::add_immediate("%v11", "%v15", COLLECTION_HEADER_SIZE),
            abi::move_immediate("%v12", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
            abi::multiply_registers("%v13", "%v10", "%v12"),
            abi::add_registers("%v14", "%v11", "%v13"),
            // x11 = entry cursor, x14 = data region, copy bytes into data.
            abi::load_u64("%v15", abi::stack_pointer(), BUF_OFFSET),
            abi::move_immediate("%v9", "Integer", "0"),
            abi::label(&entry_loop),
            abi::compare_registers("%v9", "%v10"),
            abi::branch_eq(&entry_done),
            abi::move_immediate("%v12", "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
            abi::store_u8("%v12", "%v11", COLLECTION_ENTRY_OFFSET_FLAGS),
            abi::store_u64(abi::ZERO, "%v11", COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
            abi::store_u64(abi::ZERO, "%v11", COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
            abi::store_u64("%v9", "%v11", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
            abi::move_immediate("%v12", "Integer", "1"),
            abi::store_u64("%v12", "%v11", COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
            // data[i] = buf[i]
            abi::add_registers("%v12", "%v14", "%v9"),
            abi::load_u8("%v13", "%v15", 0),
            abi::store_u8("%v13", "%v12", 0),
            abi::add_immediate("%v15", "%v15", 1),
            abi::add_immediate("%v11", "%v11", COLLECTION_ENTRY_SIZE),
            abi::add_immediate("%v9", "%v9", 1),
            abi::branch(&entry_loop),
            abi::label(&entry_done),
            abi::move_register(RESULT_VALUE_REGISTER, abi::RET[1]),
            abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
            abi::branch(&done),
        ]);
    }
    instructions.push(abi::label(&peer_closed));
    emit_fail(
        symbol,
        ERR_CONNECTION_CLOSED_CODE,
        ERR_CONNECTION_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    // read_fail: distinguish a read timeout (EAGAIN) from a closed connection.
    instructions.push(abi::label(&read_fail));
    platform.emit_errno(
        symbol,
        "%v9",
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate("%v9", platform.eagain()),
        abi::branch_eq(&timeout),
    ]);
    emit_fail(
        symbol,
        ERR_CONNECTION_CLOSED_CODE,
        ERR_CONNECTION_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&timeout));
    emit_fail(
        symbol,
        ERR_READ_TIMEOUT_CODE,
        ERR_READ_TIMEOUT_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&invalid));
    emit_fail(
        symbol,
        ERR_INVALID_ARGUMENT_CODE,
        ERR_INVALID_ARGUMENT_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&closed));
    emit_fail(
        symbol,
        ERR_RESOURCE_CLOSED_CODE,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&alloc_fail));
    emit_fail(
        symbol,
        ERR_OUT_OF_MEMORY_CODE,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    {
        let (frame, stack_slots) =
            finalize_vreg_body_with_locals(&mut instructions, &[], FRAME_SIZE);
        Ok((frame, instructions, relocations, stack_slots))
    }
}

// ---------------------------------------------------------------------------
// net.write / net.writeText
// ---------------------------------------------------------------------------

pub(in crate::target::shared::code) fn lower_net_write_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    text: bool,
) -> Result<
    (
        CodeFrame,
        Vec<CodeInstruction>,
        Vec<CodeRelocation>,
        Vec<CodeStackSlot>,
    ),
    String,
> {
    const FRAME_SIZE: usize = 96;
    const FD_OFFSET: usize = 8;
    const SRC_OFFSET: usize = 16; // pointer to the next byte to write
    const REMAINING_OFFSET: usize = 24;

    let closed = format!("{symbol}_closed");
    let write_loop = format!("{symbol}_write_loop");
    let write_done = format!("{symbol}_write_done");
    let write_fail = format!("{symbol}_write_fail");
    let timeout = format!("{symbol}_timeout");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::load_u64("%v9", abi::return_register(), FILE_OFFSET_CLOSED),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&closed),
        abi::load_u64("%v9", abi::return_register(), FILE_OFFSET_FD),
        abi::store_u64("%v9", abi::stack_pointer(), FD_OFFSET),
    ]);
    if text {
        // x1 = String*: data at +8, length at +0.
        instructions.extend([
            abi::load_u64("%v10", abi::ARG[1], 0),
            abi::store_u64("%v10", abi::stack_pointer(), REMAINING_OFFSET),
            abi::add_immediate("%v11", abi::ARG[1], 8),
            abi::store_u64("%v11", abi::stack_pointer(), SRC_OFFSET),
        ]);
    } else {
        // x1 = List OF Byte collection: bytes live inline in the data region at
        // collection + HEADER + count * ENTRY_SIZE.
        instructions.extend([
            abi::load_u64("%v10", abi::ARG[1], COLLECTION_OFFSET_COUNT),
            abi::store_u64("%v10", abi::stack_pointer(), REMAINING_OFFSET),
            abi::move_immediate("%v12", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
            abi::multiply_registers("%v13", "%v10", "%v12"),
            abi::add_immediate("%v13", "%v13", COLLECTION_HEADER_SIZE),
            abi::add_registers("%v11", abi::ARG[1], "%v13"),
            abi::store_u64("%v11", abi::stack_pointer(), SRC_OFFSET),
        ]);
    }
    instructions.extend([
        abi::label(&write_loop),
        abi::load_u64("%v10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::compare_immediate("%v10", "0"),
        abi::branch_eq(&write_done),
        // write(fd, src, remaining)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), SRC_OFFSET),
        abi::move_register(abi::ARG[2], "%v10"),
    ]);
    platform.emit_write(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(&write_fail),
        abi::load_u64("%v11", abi::stack_pointer(), SRC_OFFSET),
        abi::load_u64("%v10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::add_registers("%v11", "%v11", abi::return_register()),
        abi::subtract_registers("%v10", "%v10", abi::return_register()),
        abi::store_u64("%v11", abi::stack_pointer(), SRC_OFFSET),
        abi::store_u64("%v10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::branch(&write_loop),
        abi::label(&write_done),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&write_fail),
    ]);
    platform.emit_errno(
        symbol,
        "%v9",
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate("%v9", platform.eagain()),
        abi::branch_eq(&timeout),
    ]);
    emit_fail(
        symbol,
        ERR_CONNECTION_CLOSED_CODE,
        ERR_CONNECTION_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&timeout));
    emit_fail(
        symbol,
        ERR_WRITE_TIMEOUT_CODE,
        ERR_WRITE_TIMEOUT_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&closed));
    emit_fail(
        symbol,
        ERR_RESOURCE_CLOSED_CODE,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    {
        let (frame, stack_slots) =
            finalize_vreg_body_with_locals(&mut instructions, &[], FRAME_SIZE);
        Ok((frame, instructions, relocations, stack_slots))
    }
}

// ---------------------------------------------------------------------------
// net.lookup
// ---------------------------------------------------------------------------

pub(in crate::target::shared::code) fn lower_net_lookup_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<
    (
        CodeFrame,
        Vec<CodeInstruction>,
        Vec<CodeRelocation>,
        Vec<CodeStackSlot>,
    ),
    String,
> {
    const FRAME_SIZE: usize = 256;
    const HOST_OFFSET: usize = 8;
    const PORT_OFFSET: usize = 16;
    const RES_OFFSET: usize = 24;
    const CSTR_OFFSET: usize = 32;
    const COUNT_OFFSET: usize = 40;
    const NODE_OFFSET: usize = 48;
    const LIST_OFFSET: usize = 56;
    const ENTRY_OFFSET: usize = 64;
    const DATA_OFFSET: usize = 72;
    const INDEX_OFFSET: usize = 80;
    const DST_OFFSET: usize = 88;
    const ADDRHOST_OFFSET: usize = 96;
    const SADDR_PTR_OFFSET: usize = 152;
    const HOSTLEN_OFFSET: usize = 160;
    const HINTS_OFFSET: usize = 104; // 104..152

    let resolve_fail = format!("{symbol}_resolve_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let addr_fail = format!("{symbol}_addr_fail");
    let count_loop = format!("{symbol}_count_loop");
    let count_skip = format!("{symbol}_count_skip");
    let count_done = format!("{symbol}_count_done");
    let fill_loop = format!("{symbol}_fill_loop");
    let fill_skip = format!("{symbol}_fill_skip");
    let fill_done = format!("{symbol}_fill_done");
    let done = format!("{symbol}_done");

    let addr_off = platform.addrinfo_addr_offset();
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), HOST_OFFSET),
        abi::store_u64(abi::ARG[1], abi::stack_pointer(), PORT_OFFSET),
    ]);
    emit_hints(HINTS_OFFSET, false, SOCK_STREAM, &mut instructions);
    emit_cstring(
        symbol,
        "host",
        HOST_OFFSET,
        CSTR_OFFSET,
        &alloc_fail,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CSTR_OFFSET),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
        abi::add_immediate(abi::ARG[2], abi::stack_pointer(), HINTS_OFFSET),
        abi::add_immediate(abi::ARG[3], abi::stack_pointer(), RES_OFFSET),
    ]);
    platform.emit_libc_call(
        "getaddrinfo",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&resolve_fail),
        // Count AF_INET results.
        abi::load_u64("%v9", abi::stack_pointer(), RES_OFFSET),
        abi::store_u64("%v9", abi::stack_pointer(), NODE_OFFSET),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), COUNT_OFFSET),
        abi::label(&count_loop),
        abi::load_u64("%v9", abi::stack_pointer(), NODE_OFFSET),
        abi::compare_immediate("%v9", "0"),
        abi::branch_eq(&count_done),
        abi::load_u32("%v10", "%v9", 4),
        abi::compare_immediate("%v10", AF_INET),
        abi::branch_ne(&count_skip),
        abi::load_u64("%v11", abi::stack_pointer(), COUNT_OFFSET),
        abi::add_immediate("%v11", "%v11", 1),
        abi::store_u64("%v11", abi::stack_pointer(), COUNT_OFFSET),
        abi::label(&count_skip),
        abi::load_u64("%v9", abi::stack_pointer(), NODE_OFFSET),
        abi::load_u64("%v9", "%v9", 40),
        abi::store_u64("%v9", abi::stack_pointer(), NODE_OFFSET),
        abi::branch(&count_loop),
        abi::label(&count_done),
        // Allocate List OF Address: count Address records (16 bytes) inline.
        abi::load_u64("%v10", abi::stack_pointer(), COUNT_OFFSET),
        abi::move_immediate("%v11", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("%v12", "%v10", "%v11"),
        abi::add_immediate("%v12", "%v12", COLLECTION_HEADER_SIZE),
        abi::move_immediate("%v13", "Integer", "16"),
        abi::multiply_registers("%v14", "%v10", "%v13"),
        abi::add_registers(abi::return_register(), "%v12", "%v14"),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::move_register("%v15", abi::RET[1]), // alloc result -> vreg base (plan-34-B Phase 3)
        abi::store_u64("%v15", abi::stack_pointer(), LIST_OFFSET),
        abi::move_immediate("%v9", "Byte", &COLLECTION_KIND_LIST.to_string()),
        abi::store_u8("%v9", "%v15", COLLECTION_OFFSET_KIND),
        abi::move_immediate("%v9", "Byte", &COLLECTION_TYPE_NONE.to_string()),
        abi::store_u8("%v9", "%v15", COLLECTION_OFFSET_KEY_TYPE),
        abi::move_immediate("%v9", "Byte", &COLLECTION_TYPE_OBJECT.to_string()),
        abi::store_u8("%v9", "%v15", COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate("%v9", "Byte", "1"),
        abi::store_u8("%v9", "%v15", COLLECTION_OFFSET_FLAGS_VERSION),
        abi::load_u64("%v10", abi::stack_pointer(), COUNT_OFFSET),
        abi::store_u64("%v10", "%v15", COLLECTION_OFFSET_COUNT),
        abi::store_u64("%v10", "%v15", COLLECTION_OFFSET_CAPACITY),
        abi::move_immediate("%v13", "Integer", "16"),
        abi::multiply_registers("%v14", "%v10", "%v13"),
        abi::store_u64("%v14", "%v15", COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64("%v14", "%v15", COLLECTION_OFFSET_DATA_CAPACITY),
        // entry cursor and data region.
        abi::add_immediate("%v11", "%v15", COLLECTION_HEADER_SIZE),
        abi::store_u64("%v11", abi::stack_pointer(), ENTRY_OFFSET),
        abi::move_immediate("%v12", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("%v13", "%v10", "%v12"),
        abi::add_registers("%v14", "%v11", "%v13"),
        abi::store_u64("%v14", abi::stack_pointer(), DATA_OFFSET),
        // Iterate results again, building one Address per AF_INET node.
        abi::load_u64("%v9", abi::stack_pointer(), RES_OFFSET),
        abi::store_u64("%v9", abi::stack_pointer(), NODE_OFFSET),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), INDEX_OFFSET),
        abi::label(&fill_loop),
        abi::load_u64("%v9", abi::stack_pointer(), NODE_OFFSET),
        abi::compare_immediate("%v9", "0"),
        abi::branch_eq(&fill_done),
        abi::load_u32("%v10", "%v9", 4),
        abi::compare_immediate("%v10", AF_INET),
        abi::branch_ne(&fill_skip),
        // node->ai_addr; force the requested port into sin_port.
        abi::load_u64("%v12", "%v9", addr_off),
        abi::store_u64("%v12", abi::stack_pointer(), SADDR_PTR_OFFSET),
        abi::load_u64("%v10", abi::stack_pointer(), PORT_OFFSET),
        abi::shift_right_immediate("%v11", "%v10", 8),
        abi::store_u8("%v11", "%v12", 2),
        abi::store_u8("%v10", "%v12", 3),
    ]);
    emit_address_from_sockaddr(
        symbol,
        "node",
        SADDR_PTR_OFFSET,
        HOSTLEN_OFFSET,
        DST_OFFSET,
        ADDRHOST_OFFSET,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
        &alloc_fail,
        &addr_fail,
    )?;
    // x1 = Address pointer; copy its 16 bytes into the list data region and
    // record the entry descriptor.
    instructions.extend([
        abi::load_u64("%v9", abi::stack_pointer(), INDEX_OFFSET),
        abi::move_immediate("%v10", "Integer", "16"),
        abi::multiply_registers("%v11", "%v9", "%v10"),
        abi::load_u64("%v12", abi::stack_pointer(), DATA_OFFSET),
        abi::add_registers("%v12", "%v12", "%v11"),
        abi::load_u64("%v13", abi::RET[1], 0),
        abi::store_u64("%v13", "%v12", 0),
        abi::load_u64("%v13", abi::RET[1], 8),
        abi::store_u64("%v13", "%v12", 8),
        // entry descriptor at ENTRY cursor.
        abi::load_u64("%v14", abi::stack_pointer(), ENTRY_OFFSET),
        abi::move_immediate("%v13", "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
        abi::store_u8("%v13", "%v14", COLLECTION_ENTRY_OFFSET_FLAGS),
        abi::store_u64(abi::ZERO, "%v14", COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
        abi::store_u64(abi::ZERO, "%v14", COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
        abi::store_u64("%v11", "%v14", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::move_immediate("%v13", "Integer", "16"),
        abi::store_u64("%v13", "%v14", COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        abi::add_immediate("%v14", "%v14", COLLECTION_ENTRY_SIZE),
        abi::store_u64("%v14", abi::stack_pointer(), ENTRY_OFFSET),
        abi::load_u64("%v9", abi::stack_pointer(), INDEX_OFFSET),
        abi::add_immediate("%v9", "%v9", 1),
        abi::store_u64("%v9", abi::stack_pointer(), INDEX_OFFSET),
        abi::label(&fill_skip),
        abi::load_u64("%v9", abi::stack_pointer(), NODE_OFFSET),
        abi::load_u64("%v9", "%v9", 40),
        abi::store_u64("%v9", abi::stack_pointer(), NODE_OFFSET),
        abi::branch(&fill_loop),
        abi::label(&fill_done),
        // freeaddrinfo(res)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), RES_OFFSET),
    ]);
    platform.emit_libc_call(
        "freeaddrinfo",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), LIST_OFFSET),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&resolve_fail),
    ]);
    emit_fail(
        symbol,
        ERR_ADDRESS_NOT_FOUND_CODE,
        ERR_ADDRESS_NOT_FOUND_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&addr_fail));
    // freeaddrinfo(res): addr_fail is reached only from the inet_ntop-failure
    // branch, where the resolver result list is always allocated (getaddrinfo
    // succeeded). The success exit (fill_done) frees it; without this the error
    // exit leaked the whole addrinfo chain per failed lookup (bug-55).
    instructions.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        RES_OFFSET,
    ));
    platform.emit_libc_call(
        "freeaddrinfo",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_fail(
        symbol,
        ERR_ADDRESS_INVALID_CODE,
        ERR_ADDRESS_INVALID_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&alloc_fail));
    emit_fail(
        symbol,
        ERR_OUT_OF_MEMORY_CODE,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    {
        let (frame, stack_slots) =
            finalize_vreg_body_with_locals(&mut instructions, &[], FRAME_SIZE);
        Ok((frame, instructions, relocations, stack_slots))
    }
}

// ---------------------------------------------------------------------------
// net.bindUdp
// ---------------------------------------------------------------------------

/// `bindUdp(host, port)`: resolve the local host with `getaddrinfo`, create a
/// `SOCK_DGRAM` socket, and `bind` it. An empty host binds all interfaces
/// (NULL host + `AI_PASSIVE`). Returns a `UdpSocket` handle sharing the `File`
/// record layout.
pub(in crate::target::shared::code) fn lower_net_bind_udp_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<
    (
        CodeFrame,
        Vec<CodeInstruction>,
        Vec<CodeRelocation>,
        Vec<CodeStackSlot>,
    ),
    String,
> {
    const FRAME_SIZE: usize = 128;
    const HOST_OFFSET: usize = 8;
    const PORT_OFFSET: usize = 16;
    const RES_OFFSET: usize = 24;
    const FD_OFFSET: usize = 32;
    const CSTR_OFFSET: usize = 40;
    const HINTS_OFFSET: usize = 48; // 48..96

    let null_host = format!("{symbol}_null_host");
    let resolved = format!("{symbol}_resolved");
    let resolve_fail = format!("{symbol}_resolve_fail");
    let socket_fail = format!("{symbol}_socket_fail");
    let op_fail = format!("{symbol}_op_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), HOST_OFFSET),
        abi::store_u64(abi::ARG[1], abi::stack_pointer(), PORT_OFFSET),
    ]);
    emit_hints(HINTS_OFFSET, true, SOCK_DGRAM, &mut instructions);
    // Empty host binds all interfaces (NULL host + AI_PASSIVE).
    instructions.extend([
        abi::load_u64("%v9", abi::stack_pointer(), HOST_OFFSET),
        abi::load_u64("%v9", "%v9", 0),
        abi::compare_immediate("%v9", "0"),
        abi::branch_eq(&null_host),
    ]);
    emit_cstring(
        symbol,
        "host",
        HOST_OFFSET,
        CSTR_OFFSET,
        &alloc_fail,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::branch(&resolved));
    instructions.extend([
        abi::label(&null_host),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), CSTR_OFFSET),
        abi::label(&resolved),
        // getaddrinfo(host, NULL, &hints, &res)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CSTR_OFFSET),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
        abi::add_immediate(abi::ARG[2], abi::stack_pointer(), HINTS_OFFSET),
        abi::add_immediate(abi::ARG[3], abi::stack_pointer(), RES_OFFSET),
    ]);
    platform.emit_libc_call(
        "getaddrinfo",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&resolve_fail),
        // socket(ai_family, ai_socktype, ai_protocol)
        abi::load_u64("%v9", abi::stack_pointer(), RES_OFFSET),
        abi::load_u32(abi::return_register(), "%v9", 4),
        abi::load_u32(abi::ARG[1], "%v9", 8),
        abi::load_u32(abi::ARG[2], "%v9", 12),
    ]);
    platform.emit_libc_call(
        "socket",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&socket_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        // Overwrite sin_port at ai_addr + 2/3 with the requested port.
        abi::load_u64("%v9", abi::stack_pointer(), RES_OFFSET),
        abi::load_u64("%v9", "%v9", platform.addrinfo_addr_offset()),
        abi::load_u64("%v10", abi::stack_pointer(), PORT_OFFSET),
        abi::shift_right_immediate("%v11", "%v10", 8),
        abi::store_u8("%v11", "%v9", 2),
        abi::store_u8("%v10", "%v9", 3),
        // bind(fd, ai_addr, ai_addrlen)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64("%v9", abi::stack_pointer(), RES_OFFSET),
        abi::load_u64(abi::ARG[1], "%v9", platform.addrinfo_addr_offset()),
        abi::load_u32(abi::ARG[2], "%v9", 16),
    ]);
    platform.emit_libc_call(
        "bind",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&op_fail),
        // freeaddrinfo(res)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), RES_OFFSET),
    ]);
    platform.emit_libc_call(
        "freeaddrinfo",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_make_handle(
        symbol,
        FD_OFFSET,
        &mut instructions,
        &mut relocations,
        &alloc_fail,
    );
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    // op_fail: close the socket, free the resolver results, report failure.
    instructions.push(abi::label(&op_fail));
    instructions.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        FD_OFFSET,
    ));
    platform.emit_libc_call(
        "close",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.push(abi::label(&socket_fail));
    instructions.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        RES_OFFSET,
    ));
    platform.emit_libc_call(
        "freeaddrinfo",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_fail(
        symbol,
        ERR_NETWORK_FAILED_CODE,
        ERR_NETWORK_FAILED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&resolve_fail));
    emit_fail(
        symbol,
        ERR_ADDRESS_INVALID_CODE,
        ERR_ADDRESS_INVALID_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&alloc_fail));
    emit_fail(
        symbol,
        ERR_OUT_OF_MEMORY_CODE,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    {
        let (frame, stack_slots) =
            finalize_vreg_body_with_locals(&mut instructions, &[], FRAME_SIZE);
        Ok((frame, instructions, relocations, stack_slots))
    }
}

// ---------------------------------------------------------------------------
// net.receiveFrom / net.receiveTextFrom
// ---------------------------------------------------------------------------

/// `receiveFrom(sock, maxBytes)` / `receiveTextFrom(sock, maxBytes)`: receive a
/// single datagram with `recvfrom`, building a `Datagram` (`from`, `bytes`) or
/// `DatagramText` (`from`, `value`) record. The receive buffer is sized
/// `maxBytes + 1` so a datagram larger than `maxBytes` is detected (the returned
/// length exceeds `maxBytes`) and rejected with `ErrMessageTooLarge` rather than
/// silently truncated (§10.3).
pub(in crate::target::shared::code) fn lower_net_receive_from_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    text: bool,
) -> Result<
    (
        CodeFrame,
        Vec<CodeInstruction>,
        Vec<CodeRelocation>,
        Vec<CodeStackSlot>,
    ),
    String,
> {
    const FRAME_SIZE: usize = 224;
    const FD_OFFSET: usize = 8;
    const MAX_OFFSET: usize = 16;
    const BUF_OFFSET: usize = 24;
    const N_OFFSET: usize = 32;
    const ADDRPTR_OFFSET: usize = 40; // built Address record pointer
    const SADDR_PTR_OFFSET: usize = 48; // pointer to ADDR_STORAGE
    const ADDRLEN_OFFSET: usize = 56; // recvfrom socklen in/out
    const DST_OFFSET: usize = 64;
    const HOSTLEN_OFFSET: usize = 72;
    const AHOST_OFFSET: usize = 80;
    const STR_OFFSET: usize = 88; // built bytes/string pointer
    const ADDR_STORAGE_OFFSET: usize = 96; // 96..224 sockaddr_storage

    let closed = format!("{symbol}_closed");
    let invalid = format!("{symbol}_invalid");
    let recv_fail = format!("{symbol}_recv_fail");
    let timeout = format!("{symbol}_timeout");
    let too_large = format!("{symbol}_too_large");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let addr_fail = format!("{symbol}_addr_fail");
    let encoding_error = format!("{symbol}_encoding_error");
    let str_copy = format!("{symbol}_str_copy");
    let str_done = format!("{symbol}_str_done");
    let entry_loop = format!("{symbol}_entry_loop");
    let entry_done = format!("{symbol}_entry_done");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::ARG[1], abi::stack_pointer(), MAX_OFFSET),
        abi::load_u64("%v9", abi::return_register(), FILE_OFFSET_CLOSED),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&closed),
        abi::load_u64("%v9", abi::return_register(), FILE_OFFSET_FD),
        abi::store_u64("%v9", abi::stack_pointer(), FD_OFFSET),
        abi::load_u64("%v10", abi::stack_pointer(), MAX_OFFSET),
        abi::compare_immediate("%v10", "0"),
        abi::branch_le(&invalid),
        // Allocate a maxBytes + 1 buffer to detect oversized datagrams.
        abi::add_immediate(abi::return_register(), "%v10", 1),
        abi::move_immediate(abi::ARG[1], "Integer", "1"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::store_u64(abi::RET[1], abi::stack_pointer(), BUF_OFFSET),
        // recvfrom(fd, buf, maxBytes + 1, 0, &addr_storage, &addrlen)
        abi::move_immediate("%v9", "Integer", &SOCKADDR_STORAGE_SIZE.to_string()),
        abi::store_u64("%v9", abi::stack_pointer(), ADDRLEN_OFFSET),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), BUF_OFFSET),
        abi::load_u64(abi::ARG[2], abi::stack_pointer(), MAX_OFFSET),
        abi::add_immediate(abi::ARG[2], abi::ARG[2], 1),
        abi::move_immediate(abi::ARG[3], "Integer", "0"),
        abi::add_immediate(abi::ARG[4], abi::stack_pointer(), ADDR_STORAGE_OFFSET),
        abi::add_immediate(abi::ARG[5], abi::stack_pointer(), ADDRLEN_OFFSET),
    ]);
    platform.emit_libc_call(
        "recvfrom",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&recv_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), N_OFFSET),
        // Reject truncation: n > maxBytes means the datagram did not fit.
        abi::load_u64("%v9", abi::stack_pointer(), N_OFFSET),
        abi::load_u64("%v10", abi::stack_pointer(), MAX_OFFSET),
        abi::compare_registers("%v10", "%v9"),
        abi::branch_lt(&too_large),
        // Build the sender Address from the captured sockaddr.
        abi::add_immediate("%v9", abi::stack_pointer(), ADDR_STORAGE_OFFSET),
        abi::store_u64("%v9", abi::stack_pointer(), SADDR_PTR_OFFSET),
    ]);
    emit_address_from_sockaddr(
        symbol,
        "recv",
        SADDR_PTR_OFFSET,
        HOSTLEN_OFFSET,
        DST_OFFSET,
        AHOST_OFFSET,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
        &alloc_fail,
        &addr_fail,
    )?;
    instructions.push(abi::store_u64(abi::RET[1], abi::stack_pointer(), ADDRPTR_OFFSET));
    if text {
        // Build a String: [u64 len][bytes][nul], validate UTF-8.
        instructions.extend([
            abi::load_u64("%v10", abi::stack_pointer(), N_OFFSET),
            abi::add_immediate(abi::return_register(), "%v10", 9),
            abi::move_immediate(abi::ARG[1], "Integer", "8"),
        ]);
        emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
        instructions.extend([
            abi::move_register("%v15", abi::RET[1]), // alloc result -> vreg base (plan-34-B Phase 3)
            abi::load_u64("%v10", abi::stack_pointer(), N_OFFSET),
            abi::store_u64("%v10", "%v15", 0),
            abi::load_u64("%v11", abi::stack_pointer(), BUF_OFFSET),
            abi::add_immediate("%v12", "%v15", 8),
            abi::move_immediate("%v13", "Integer", "0"),
            abi::store_u64("%v15", abi::stack_pointer(), STR_OFFSET),
            abi::label(&str_copy),
            abi::compare_registers("%v13", "%v10"),
            abi::branch_eq(&str_done),
            abi::load_u8("%v14", "%v11", 0),
            abi::store_u8("%v14", "%v12", 0),
            abi::add_immediate("%v11", "%v11", 1),
            abi::add_immediate("%v12", "%v12", 1),
            abi::add_immediate("%v13", "%v13", 1),
            abi::branch(&str_copy),
            abi::label(&str_done),
            abi::store_u8(abi::ZERO, "%v12", 0),
            // validate_utf8(bytes, len)
            abi::load_u64("%v9", abi::stack_pointer(), STR_OFFSET),
            abi::add_immediate(abi::return_register(), "%v9", 8),
            abi::load_u64(abi::ARG[1], "%v9", 0),
        ]);
        emit_call_validate_utf8(symbol, &encoding_error, &mut instructions, &mut relocations);
    } else {
        // Build a List OF Byte with N elements.
        instructions.extend([
            abi::load_u64("%v10", abi::stack_pointer(), N_OFFSET),
            abi::move_immediate("%v11", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
            abi::multiply_registers("%v12", "%v10", "%v11"),
            abi::add_immediate("%v12", "%v12", COLLECTION_HEADER_SIZE),
            abi::add_registers(abi::return_register(), "%v12", "%v10"),
            abi::move_immediate(abi::ARG[1], "Integer", "8"),
        ]);
        emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
        instructions.extend([
            abi::move_register("%v15", abi::RET[1]), // alloc result -> vreg base (plan-34-B Phase 3)
            abi::store_u64("%v15", abi::stack_pointer(), STR_OFFSET),
            abi::move_immediate("%v9", "Byte", &COLLECTION_KIND_LIST.to_string()),
            abi::store_u8("%v9", "%v15", COLLECTION_OFFSET_KIND),
            abi::move_immediate("%v9", "Byte", &COLLECTION_TYPE_NONE.to_string()),
            abi::store_u8("%v9", "%v15", COLLECTION_OFFSET_KEY_TYPE),
            abi::move_immediate("%v9", "Byte", &COLLECTION_TYPE_BYTE.to_string()),
            abi::store_u8("%v9", "%v15", COLLECTION_OFFSET_VALUE_TYPE),
            abi::move_immediate("%v9", "Byte", "1"),
            abi::store_u8("%v9", "%v15", COLLECTION_OFFSET_FLAGS_VERSION),
            abi::load_u64("%v10", abi::stack_pointer(), N_OFFSET),
            abi::store_u64("%v10", "%v15", COLLECTION_OFFSET_COUNT),
            abi::store_u64("%v10", "%v15", COLLECTION_OFFSET_CAPACITY),
            abi::store_u64("%v10", "%v15", COLLECTION_OFFSET_DATA_LENGTH),
            abi::store_u64("%v10", "%v15", COLLECTION_OFFSET_DATA_CAPACITY),
            abi::add_immediate("%v11", "%v15", COLLECTION_HEADER_SIZE),
            abi::move_immediate("%v12", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
            abi::multiply_registers("%v13", "%v10", "%v12"),
            abi::add_registers("%v14", "%v11", "%v13"),
            abi::load_u64("%v15", abi::stack_pointer(), BUF_OFFSET),
            abi::move_immediate("%v9", "Integer", "0"),
            abi::label(&entry_loop),
            abi::compare_registers("%v9", "%v10"),
            abi::branch_eq(&entry_done),
            abi::move_immediate("%v12", "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
            abi::store_u8("%v12", "%v11", COLLECTION_ENTRY_OFFSET_FLAGS),
            abi::store_u64(abi::ZERO, "%v11", COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
            abi::store_u64(abi::ZERO, "%v11", COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
            abi::store_u64("%v9", "%v11", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
            abi::move_immediate("%v12", "Integer", "1"),
            abi::store_u64("%v12", "%v11", COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
            abi::add_registers("%v12", "%v14", "%v9"),
            abi::load_u8("%v13", "%v15", 0),
            abi::store_u8("%v13", "%v12", 0),
            abi::add_immediate("%v15", "%v15", 1),
            abi::add_immediate("%v11", "%v11", COLLECTION_ENTRY_SIZE),
            abi::add_immediate("%v9", "%v9", 1),
            abi::branch(&entry_loop),
            abi::label(&entry_done),
        ]);
    }
    // Allocate the Datagram/DatagramText record: [from Address][bytes/value].
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "16"),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::move_register("%v15", abi::RET[1]), // alloc result -> vreg base; x1 kept for RESULT_VALUE_REGISTER
        abi::load_u64("%v9", abi::stack_pointer(), ADDRPTR_OFFSET),
        abi::store_u64("%v9", "%v15", 0),
        abi::load_u64("%v9", abi::stack_pointer(), STR_OFFSET),
        abi::store_u64("%v9", "%v15", 8),
        abi::move_register(RESULT_VALUE_REGISTER, abi::RET[1]),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    // recv_fail: EAGAIN/EWOULDBLOCK is a read timeout; anything else is a
    // network failure.
    instructions.push(abi::label(&recv_fail));
    platform.emit_errno(
        symbol,
        "%v9",
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate("%v9", platform.eagain()),
        abi::branch_eq(&timeout),
    ]);
    emit_fail(
        symbol,
        ERR_NETWORK_FAILED_CODE,
        ERR_NETWORK_FAILED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&timeout));
    emit_fail(
        symbol,
        ERR_READ_TIMEOUT_CODE,
        ERR_READ_TIMEOUT_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&too_large));
    emit_fail(
        symbol,
        ERR_MESSAGE_TOO_LARGE_CODE,
        ERR_MESSAGE_TOO_LARGE_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    if text {
        instructions.push(abi::label(&encoding_error));
        emit_fail(
            symbol,
            ERR_ENCODING_CODE,
            ERR_ENCODING_SYMBOL,
            &mut instructions,
            &mut relocations,
            &done,
        );
    }
    instructions.push(abi::label(&invalid));
    emit_fail(
        symbol,
        ERR_INVALID_ARGUMENT_CODE,
        ERR_INVALID_ARGUMENT_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&addr_fail));
    emit_fail(
        symbol,
        ERR_ADDRESS_INVALID_CODE,
        ERR_ADDRESS_INVALID_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&closed));
    emit_fail(
        symbol,
        ERR_RESOURCE_CLOSED_CODE,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&alloc_fail));
    emit_fail(
        symbol,
        ERR_OUT_OF_MEMORY_CODE,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    {
        let (frame, stack_slots) =
            finalize_vreg_body_with_locals(&mut instructions, &[], FRAME_SIZE);
        Ok((frame, instructions, relocations, stack_slots))
    }
}

// ---------------------------------------------------------------------------
// net.sendTo / net.sendTextTo
// ---------------------------------------------------------------------------

/// `sendTo(sock, address, bytes)` / `sendTextTo(sock, address, value)`: resolve
/// the destination `Address` with `getaddrinfo` and send a single datagram with
/// `sendto`. An oversized datagram (`EMSGSIZE`) maps to `ErrMessageTooLarge`.
pub(in crate::target::shared::code) fn lower_net_send_to_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    text: bool,
) -> Result<
    (
        CodeFrame,
        Vec<CodeInstruction>,
        Vec<CodeRelocation>,
        Vec<CodeStackSlot>,
    ),
    String,
> {
    const FRAME_SIZE: usize = 144;
    const FD_OFFSET: usize = 8;
    const DATA_OFFSET: usize = 24; // pointer to payload bytes
    const DLEN_OFFSET: usize = 32; // payload length
    const HOST_OFFSET: usize = 40; // destination host String pointer
    const PORT_OFFSET: usize = 48;
    const CSTR_OFFSET: usize = 56;
    const RES_OFFSET: usize = 64;
    const HINTS_OFFSET: usize = 72; // 72..120
    const RET_OFFSET: usize = 120; // sendto return value
    const ERRNO_OFFSET: usize = 128; // captured errno

    let closed = format!("{symbol}_closed");
    let resolve_fail = format!("{symbol}_resolve_fail");
    let send_fail = format!("{symbol}_send_fail");
    let timeout = format!("{symbol}_timeout");
    let too_large = format!("{symbol}_too_large");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    instructions.extend([
        // x0 = UdpSocket record; reject if closed.
        abi::load_u64("%v9", abi::return_register(), FILE_OFFSET_CLOSED),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&closed),
        abi::load_u64("%v9", abi::return_register(), FILE_OFFSET_FD),
        abi::store_u64("%v9", abi::stack_pointer(), FD_OFFSET),
        // x1 = Address record { host String ptr @0, port @8 }.
        abi::load_u64("%v9", abi::ARG[1], 0),
        abi::store_u64("%v9", abi::stack_pointer(), HOST_OFFSET),
        abi::load_u64("%v9", abi::ARG[1], 8),
        abi::store_u64("%v9", abi::stack_pointer(), PORT_OFFSET),
    ]);
    if text {
        // x2 = String*: data at +8, length at +0.
        instructions.extend([
            abi::load_u64("%v10", abi::ARG[2], 0),
            abi::store_u64("%v10", abi::stack_pointer(), DLEN_OFFSET),
            abi::add_immediate("%v11", abi::ARG[2], 8),
            abi::store_u64("%v11", abi::stack_pointer(), DATA_OFFSET),
        ]);
    } else {
        // x2 = List OF Byte: bytes live inline at collection + HEADER +
        // count * ENTRY_SIZE.
        instructions.extend([
            abi::load_u64("%v10", abi::ARG[2], COLLECTION_OFFSET_COUNT),
            abi::store_u64("%v10", abi::stack_pointer(), DLEN_OFFSET),
            abi::move_immediate("%v12", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
            abi::multiply_registers("%v13", "%v10", "%v12"),
            abi::add_immediate("%v13", "%v13", COLLECTION_HEADER_SIZE),
            abi::add_registers("%v11", abi::ARG[2], "%v13"),
            abi::store_u64("%v11", abi::stack_pointer(), DATA_OFFSET),
        ]);
    }
    emit_hints(HINTS_OFFSET, false, SOCK_DGRAM, &mut instructions);
    emit_cstring(
        symbol,
        "host",
        HOST_OFFSET,
        CSTR_OFFSET,
        &alloc_fail,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        // getaddrinfo(host, NULL, &hints, &res)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CSTR_OFFSET),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
        abi::add_immediate(abi::ARG[2], abi::stack_pointer(), HINTS_OFFSET),
        abi::add_immediate(abi::ARG[3], abi::stack_pointer(), RES_OFFSET),
    ]);
    platform.emit_libc_call(
        "getaddrinfo",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&resolve_fail),
        // Force the requested port into sin_port at ai_addr + 2/3.
        abi::load_u64("%v9", abi::stack_pointer(), RES_OFFSET),
        abi::load_u64("%v9", "%v9", platform.addrinfo_addr_offset()),
        abi::load_u64("%v10", abi::stack_pointer(), PORT_OFFSET),
        abi::shift_right_immediate("%v11", "%v10", 8),
        abi::store_u8("%v11", "%v9", 2),
        abi::store_u8("%v10", "%v9", 3),
        // sendto(fd, data, dlen, 0, ai_addr, ai_addrlen)
        abi::load_u64("%v9", abi::stack_pointer(), RES_OFFSET),
        abi::load_u64(abi::ARG[4], "%v9", platform.addrinfo_addr_offset()),
        abi::load_u32(abi::ARG[5], "%v9", 16),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), DATA_OFFSET),
        abi::load_u64(abi::ARG[2], abi::stack_pointer(), DLEN_OFFSET),
        abi::move_immediate(abi::ARG[3], "Integer", "0"),
    ]);
    platform.emit_libc_call(
        "sendto",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.push(abi::store_u64(
        abi::return_register(),
        abi::stack_pointer(),
        RET_OFFSET,
    ));
    // Capture errno before freeaddrinfo can disturb it.
    platform.emit_errno(
        symbol,
        "%v9",
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.push(abi::store_u64("%v9", abi::stack_pointer(), ERRNO_OFFSET));
    instructions.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        RES_OFFSET,
    ));
    platform.emit_libc_call(
        "freeaddrinfo",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64("%v9", abi::stack_pointer(), RET_OFFSET),
        abi::compare_immediate("%v9", "0"),
        abi::branch_lt(&send_fail),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    // send_fail: classify by captured errno.
    instructions.extend([
        abi::label(&send_fail),
        abi::load_u64("%v9", abi::stack_pointer(), ERRNO_OFFSET),
        abi::compare_immediate("%v9", platform.eagain()),
        abi::branch_eq(&timeout),
        abi::load_u64("%v9", abi::stack_pointer(), ERRNO_OFFSET),
        abi::compare_immediate("%v9", platform.emsgsize()),
        abi::branch_eq(&too_large),
    ]);
    emit_fail(
        symbol,
        ERR_NETWORK_FAILED_CODE,
        ERR_NETWORK_FAILED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&timeout));
    emit_fail(
        symbol,
        ERR_WRITE_TIMEOUT_CODE,
        ERR_WRITE_TIMEOUT_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&too_large));
    emit_fail(
        symbol,
        ERR_MESSAGE_TOO_LARGE_CODE,
        ERR_MESSAGE_TOO_LARGE_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&resolve_fail));
    emit_fail(
        symbol,
        ERR_ADDRESS_NOT_FOUND_CODE,
        ERR_ADDRESS_NOT_FOUND_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&closed));
    emit_fail(
        symbol,
        ERR_RESOURCE_CLOSED_CODE,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&alloc_fail));
    emit_fail(
        symbol,
        ERR_OUT_OF_MEMORY_CODE,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    {
        let (frame, stack_slots) =
            finalize_vreg_body_with_locals(&mut instructions, &[], FRAME_SIZE);
        Ok((frame, instructions, relocations, stack_slots))
    }
}

#[cfg(test)]
mod lookup_release_tests {
    // Regression guard for bug-55: net::lookup's addr_fail (inet_ntop-failure)
    // exit must freeaddrinfo(res) like the fill_done success exit, else the whole
    // addrinfo chain leaks on a failed lookup. Counts the emitted freeaddrinfo
    // calls (success exit + error exit).
    use super::*;
    use crate::target::shared::code::mir;
    use crate::target::shared::code::test_support::TestPlatform;

    #[test]
    fn lookup_frees_addrinfo_on_addr_fail() {
        mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
        let imports = HashMap::new();
        let (_f, ins, _r, _s) =
            lower_net_lookup_helper("lk", &imports, &TestPlatform).expect("lower lookup");
        let freeaddrinfo_calls = ins
            .iter()
            .filter(|i| i.op == CodeOp::BranchLink && i.get("target") == Some("_freeaddrinfo"))
            .count();
        assert!(
            freeaddrinfo_calls >= 2,
            "lookup must freeaddrinfo on both the success and addr_fail exits, saw {freeaddrinfo_calls}"
        );
    }
}
