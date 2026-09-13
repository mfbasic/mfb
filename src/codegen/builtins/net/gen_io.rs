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
use crate::codegen::memory::arena::{emit_helper_scratch_release, HelperScratch};
use crate::codegen::memory::marshal::{
    emit_build_record_list, MarshalRegs, RecordBuildScratch, RecordListScratch,
    RECORD_LIST_PAIR_SIZE,
};
use crate::codegen::os::socket::shared::*;
use crate::target::shared::abi;
use std::collections::HashMap;

/// `net::lookup(host[, port])`: resolve `host` with `getaddrinfo` and return one
/// `net::Address` per `AF_INET` result, each carrying the requested port.
///
/// plan-132: every `Address` is the spec-canonical flat record (its host inlined),
/// so the list's elements are variable-length. Each is built as its own block and
/// recorded as a `(pointer, size)` pair; `emit_build_record_list` then lays the
/// list out exactly as a source-built `List OF net::Address` and frees the
/// per-element blocks and the pair array.
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
    const PAIRS_OFFSET: usize = 56; // one (Address pointer, size) pair per result
    const INDEX_OFFSET: usize = 64;
    const DST_OFFSET: usize = 72; // the address builder's inet_ntop scratch
    const ADDRHOST_OFFSET: usize = 80; // the address builder's host String scratch
    const HOSTLEN_OFFSET: usize = 88;
    const APORT_OFFSET: usize = 96;
    const HINTS_OFFSET: usize = 104; // 104..152
    const SADDR_PTR_OFFSET: usize = 152;
    const RSIZE_OFFSET: usize = 160; // the built Address's byte size
    const RRESULT_OFFSET: usize = 168; // the built Address
    const RCURSOR_OFFSET: usize = 176;
    const RBLOCK_OFFSET: usize = 184;
    const LCURSOR_OFFSET: usize = 192;
    const LINDEX_OFFSET: usize = 200;
    const LLIST_OFFSET: usize = 208;

    let resolve_fail = format!("{symbol}_resolve_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let addr_fail = format!("{symbol}_addr_fail");
    let count_loop = format!("{symbol}_count_loop");
    let count_skip = format!("{symbol}_count_skip");
    let count_done = format!("{symbol}_count_done");
    let pairs_ready = format!("{symbol}_pairs_ready");
    let fill_loop = format!("{symbol}_fill_loop");
    let fill_skip = format!("{symbol}_fill_skip");
    let fill_done = format!("{symbol}_fill_done");
    let done = format!("{symbol}_done");

    let addr_off = platform.addrinfo_addr_offset();
    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    let mut vregs = Vregs::new();
    let host_scratch = HelperScratch::declare(&mut vregs, &mut instructions);
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    let v12 = vregs.next();
    let v13 = vregs.next();
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
        &host_scratch,
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
        // One (pointer, size) pair per AF_INET result. An empty result allocates
        // no array, and `emit_build_record_list` frees none for it.
        abi::store_u64(abi::ZERO, abi::stack_pointer(), PAIRS_OFFSET),
        abi::load_u64(&v10, abi::stack_pointer(), COUNT_OFFSET),
        abi::compare_immediate(&v10, "0"),
        abi::branch_eq(&pairs_ready),
        abi::move_immediate(&v11, "Integer", &RECORD_LIST_PAIR_SIZE.to_string()),
        abi::multiply_registers(abi::return_register(), &v10, &v11),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), PAIRS_OFFSET),
        abi::label(&pairs_ready),
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
        DST_OFFSET,
        &AddressSlots {
            len: HOSTLEN_OFFSET,
            host: ADDRHOST_OFFSET,
            port: APORT_OFFSET,
            record: RecordBuildScratch {
                size: RSIZE_OFFSET,
                result: RRESULT_OFFSET,
                cursor: RCURSOR_OFFSET,
                block_size: RBLOCK_OFFSET,
            },
        },
        &alloc_fail,
        &addr_fail,
        &mut vregs,
    )?;
    // pairs[index] = (the built Address, its byte size).
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), INDEX_OFFSET),
        abi::move_immediate(&v10, "Integer", &RECORD_LIST_PAIR_SIZE.to_string()),
        abi::multiply_registers(&v11, &v9, &v10),
        abi::load_u64(&v12, abi::stack_pointer(), PAIRS_OFFSET),
        abi::add_registers(&v12, &v12, &v11),
        abi::load_u64(&v13, abi::stack_pointer(), RRESULT_OFFSET),
        abi::store_u64(&v13, &v12, 0),
        abi::load_u64(&v13, abi::stack_pointer(), RSIZE_OFFSET),
        abi::store_u64(&v13, &v12, 8),
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
    emit_build_record_list(
        symbol,
        "lookup",
        PAIRS_OFFSET,
        COUNT_OFFSET,
        &RecordListScratch {
            cursor: LCURSOR_OFFSET,
            index: LINDEX_OFFSET,
            list: LLIST_OFFSET,
        },
        &MarshalRegs::fresh(&mut vregs),
        RESULT_VALUE_REGISTER,
        &alloc_fail,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
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
    instructions.push(abi::label(&done));
    // bug-574: release the marshalled host C-string; the host call consumed
    // it and nothing on the MFBASIC side of this call can see it.
    emit_helper_scratch_release(
        symbol,
        &[host_scratch],
        &mut vregs,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::return_());
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
