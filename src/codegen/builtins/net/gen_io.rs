//! Native code generation for `net`'s resolver: `lookup`.
//!
//! plan-110-E Phase 3: everything else that lived here moved out. The stream
//! half (accept/read/write) went to `tcp`, the datagram half
//! (bind/receive/send) to `udp`, and the shared address builder to
//! `codegen::os::socket::shared`, which is where `tcp`, `udp`, `tls` and
//! `ping` can all reach it. What is left is the one emitter that is genuinely
//! net's: turning a host name into a `List OF Address`.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::error::emission::*;
use crate::codegen::os::socket::shared::*;
use crate::target::shared::abi;
use std::collections::HashMap;

pub(crate) fn lower_net_lookup_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<NetBodyParts, String> {
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
    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    let v12 = vregs.next();
    let v13 = vregs.next();
    let v14 = vregs.next();
    let v15 = vregs.next();
    instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), HOST_OFFSET),
        abi::store_u64(abi::c_arg(1), abi::stack_pointer(), PORT_OFFSET),
    ]);
    emit_hints(
        HINTS_OFFSET,
        false,
        SOCK_STREAM,
        &mut instructions,
        &mut vregs,
    );
    emit_cstring(
        symbol,
        "host",
        HOST_OFFSET,
        CSTR_OFFSET,
        &alloc_fail,
        &mut instructions,
        &mut relocations,
        &mut vregs,
    );
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CSTR_OFFSET),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        abi::add_immediate(abi::c_arg(2), abi::stack_pointer(), HINTS_OFFSET),
        abi::add_immediate(abi::c_arg(3), abi::stack_pointer(), RES_OFFSET),
    ]);
    platform.emit_external_call(
        net_symbol(platform, NetSymbol::GetAddrInfo),
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&resolve_fail),
        // Count AF_INET results.
        abi::load_u64(&v9, abi::stack_pointer(), RES_OFFSET),
        abi::store_u64(&v9, abi::stack_pointer(), NODE_OFFSET),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), COUNT_OFFSET),
        abi::label(&count_loop),
        abi::load_u64(&v9, abi::stack_pointer(), NODE_OFFSET),
        abi::compare_immediate(&v9, "0"),
        abi::branch_eq(&count_done),
        abi::load_u32(&v10, &v9, 4),
        abi::compare_immediate(&v10, AF_INET),
        abi::branch_ne(&count_skip),
        abi::load_u64(&v11, abi::stack_pointer(), COUNT_OFFSET),
        abi::add_immediate(&v11, &v11, 1),
        abi::store_u64(&v11, abi::stack_pointer(), COUNT_OFFSET),
        abi::label(&count_skip),
        abi::load_u64(&v9, abi::stack_pointer(), NODE_OFFSET),
        abi::load_u64(&v9, &v9, 40),
        abi::store_u64(&v9, abi::stack_pointer(), NODE_OFFSET),
        abi::branch(&count_loop),
        abi::label(&count_done),
        // Allocate List OF Address: count Address records (16 bytes) inline.
        abi::load_u64(&v10, abi::stack_pointer(), COUNT_OFFSET),
        abi::move_immediate(&v11, "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers(&v12, &v10, &v11),
        abi::add_immediate(&v12, &v12, COLLECTION_HEADER_SIZE),
        abi::move_immediate(&v13, "Integer", "16"),
        abi::multiply_registers(&v14, &v10, &v13),
        abi::add_registers(abi::return_register(), &v12, &v14),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::move_register(&v15, abi::mfb_return(1)), // alloc result -> vreg base (plan-34-B Phase 3)
        abi::store_u64(&v15, abi::stack_pointer(), LIST_OFFSET),
        abi::move_immediate(&v9, "Byte", &COLLECTION_KIND_LIST.to_string()),
        abi::store_u8(&v9, &v15, COLLECTION_OFFSET_KIND),
        abi::move_immediate(&v9, "Byte", &COLLECTION_TYPE_NONE.to_string()),
        abi::store_u8(&v9, &v15, COLLECTION_OFFSET_KEY_TYPE),
        abi::move_immediate(&v9, "Byte", &COLLECTION_TYPE_OBJECT.to_string()),
        abi::store_u8(&v9, &v15, COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate(&v9, "Byte", "1"),
        abi::store_u8(&v9, &v15, COLLECTION_OFFSET_FLAGS_VERSION),
        abi::load_u64(&v10, abi::stack_pointer(), COUNT_OFFSET),
        abi::store_u64(&v10, &v15, COLLECTION_OFFSET_COUNT),
        abi::store_u64(&v10, &v15, COLLECTION_OFFSET_CAPACITY),
        abi::move_immediate(&v13, "Integer", "16"),
        abi::multiply_registers(&v14, &v10, &v13),
        abi::store_u64(&v14, &v15, COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64(&v14, &v15, COLLECTION_OFFSET_DATA_CAPACITY),
        // entry cursor and data region.
        abi::add_immediate(&v11, &v15, COLLECTION_HEADER_SIZE),
        abi::store_u64(&v11, abi::stack_pointer(), ENTRY_OFFSET),
        abi::move_immediate(&v12, "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers(&v13, &v10, &v12),
        abi::add_registers(&v14, &v11, &v13),
        abi::store_u64(&v14, abi::stack_pointer(), DATA_OFFSET),
        // Iterate results again, building one Address per AF_INET node.
        abi::load_u64(&v9, abi::stack_pointer(), RES_OFFSET),
        abi::store_u64(&v9, abi::stack_pointer(), NODE_OFFSET),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), INDEX_OFFSET),
        abi::label(&fill_loop),
        abi::load_u64(&v9, abi::stack_pointer(), NODE_OFFSET),
        abi::compare_immediate(&v9, "0"),
        abi::branch_eq(&fill_done),
        abi::load_u32(&v10, &v9, 4),
        abi::compare_immediate(&v10, AF_INET),
        abi::branch_ne(&fill_skip),
        // node->ai_addr; force the requested port into sin_port.
        abi::load_u64(&v12, &v9, addr_off),
        abi::store_u64(&v12, abi::stack_pointer(), SADDR_PTR_OFFSET),
        abi::load_u64(&v10, abi::stack_pointer(), PORT_OFFSET),
        abi::shift_right_immediate(&v11, &v10, 8),
        abi::store_u8(&v11, &v12, 2),
        abi::store_u8(&v10, &v12, 3),
    ]);
    emit_address_from_sockaddr(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "node",
        SADDR_PTR_OFFSET,
        HOSTLEN_OFFSET,
        DST_OFFSET,
        ADDRHOST_OFFSET,
        &alloc_fail,
        &addr_fail,
        &mut vregs,
    )?;
    // x1 = Address pointer; copy its 16 bytes into the list data region and
    // record the entry descriptor.
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), INDEX_OFFSET),
        abi::move_immediate(&v10, "Integer", "16"),
        abi::multiply_registers(&v11, &v9, &v10),
        abi::load_u64(&v12, abi::stack_pointer(), DATA_OFFSET),
        abi::add_registers(&v12, &v12, &v11),
        abi::load_u64(&v13, abi::mfb_return(1), 0),
        abi::store_u64(&v13, &v12, 0),
        abi::load_u64(&v13, abi::mfb_return(1), 8),
        abi::store_u64(&v13, &v12, 8),
        // entry descriptor at ENTRY cursor.
        abi::load_u64(&v14, abi::stack_pointer(), ENTRY_OFFSET),
        abi::move_immediate(&v13, "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
        abi::store_u8(&v13, &v14, COLLECTION_ENTRY_OFFSET_FLAGS),
        abi::store_u64(abi::ZERO, &v14, COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
        abi::store_u64(abi::ZERO, &v14, COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
        abi::store_u64(&v11, &v14, COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::move_immediate(&v13, "Integer", "16"),
        abi::store_u64(&v13, &v14, COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        abi::add_immediate(&v14, &v14, COLLECTION_ENTRY_SIZE),
        abi::store_u64(&v14, abi::stack_pointer(), ENTRY_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), INDEX_OFFSET),
        abi::add_immediate(&v9, &v9, 1),
        abi::store_u64(&v9, abi::stack_pointer(), INDEX_OFFSET),
        abi::label(&fill_skip),
        abi::load_u64(&v9, abi::stack_pointer(), NODE_OFFSET),
        abi::load_u64(&v9, &v9, 40),
        abi::store_u64(&v9, abi::stack_pointer(), NODE_OFFSET),
        abi::branch(&fill_loop),
        abi::label(&fill_done),
        // freeaddrinfo(res)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), RES_OFFSET),
    ]);
    platform.emit_external_call(
        net_symbol(platform, NetSymbol::FreeAddrInfo),
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
        "ErrAddressNotFound",
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
    platform.emit_external_call(
        net_symbol(platform, NetSymbol::FreeAddrInfo),
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_fail(
        symbol,
        "ErrAddressInvalid",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&alloc_fail));
    emit_fail(
        symbol,
        "ErrOutOfMemory",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    {
        Ok((instructions, relocations, FRAME_SIZE))
    }
}

#[cfg(test)]
mod lookup_release_tests {
    // Regression guard for bug-55: net::lookup's addr_fail (inet_ntop-failure)
    // exit must freeaddrinfo(res) like the fill_done success exit, else the whole
    // addrinfo chain leaks on a failed lookup. Counts the emitted freeaddrinfo
    // calls (success exit + error exit).
    use super::*;
    use crate::arch::ops::CodeOp;
    use crate::codegen::engine::mir;
    use crate::codegen::engine::tests::TestPlatform;

    #[test]
    fn lookup_frees_addrinfo_on_addr_fail() {
        mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
        let imports = HashMap::new();
        let (ins, _r, _s) =
            lower_net_lookup_helper("lk", &imports, &TestPlatform).expect("lower lookup");
        let freeaddrinfo_calls = ins
            .iter()
            .filter(|i| {
                i.op == CodeOp::BranchLink && i.get("target").as_deref() == Some("_freeaddrinfo")
            })
            .count();
        assert!(
            freeaddrinfo_calls >= 2,
            "lookup must freeaddrinfo on both the success and addr_fail exits, saw {freeaddrinfo_calls}"
        );
    }
}
