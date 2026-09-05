//! Native code generation for `udp`'s datagram IO: bind, receive, send.
//!
//! plan-110-E Phase 3 moved these out of `builtins/net/gen_io.rs` for the same
//! reason as `tcp`'s half -- `net` no longer owns a socket, so it should not
//! own the emitters either. The shared pieces (address building, pollfd, the
//! timeout setter) stay in `codegen::os::socket`.

// --- codegen tier imports (migration) ---
use crate::codegen::collection::layout::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::error::emission::*;
use crate::codegen::memory::marshal::push_write_payload_view;
use crate::codegen::os::socket::shared::*;
use crate::codegen::os::syscall::*;
use crate::target::shared::abi;
use std::collections::HashMap;

/// Winsock `WSAETIMEDOUT`: a blocking socket op that hits SO_RCVTIMEO/SO_SNDTIMEO
/// reports this on Windows, where POSIX reports EAGAIN/EWOULDBLOCK (plan-47-I,
/// bug-109). Used only on the `PlatformFamily::Windows` timeout arms.
const WSAETIMEDOUT: &str = "10060";

/// `bindUdp(host, port)`: resolve the local host with `getaddrinfo`, create a
/// `SOCK_DGRAM` socket, and `bind` it. An empty host binds all interfaces
/// (NULL host + `AI_PASSIVE`). Returns a `udp::Socket` handle sharing the `File`
/// record layout.
pub(crate) fn lower_net_bind_udp_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<NetBodyParts, String> {
    const FRAME_SIZE: usize = 128;
    const HOST_OFFSET: usize = 8;
    const PORT_OFFSET: usize = 16;
    const RES_OFFSET: usize = 24;
    const FD_OFFSET: usize = 32;
    const CSTR_OFFSET: usize = 40;
    const HINTS_OFFSET: usize = 48; // 48..96
                                    // getaddrinfo `service`: NULL for a resolved host, the "0" C string below for
                                    // a NULL/bind-all host (getaddrinfo rejects node==service==NULL; the real
                                    // port is patched into sin_port afterward). bug-113.
    const SERVICE_OFFSET: usize = 96;
    const SERVICE_STR_OFFSET: usize = 104; // holds the bytes "0\0…"

    let null_host = format!("{symbol}_null_host");
    let resolved = format!("{symbol}_resolved");
    let resolve_fail = format!("{symbol}_resolve_fail");
    let socket_fail = format!("{symbol}_socket_fail");
    let op_fail = format!("{symbol}_op_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");

    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), HOST_OFFSET),
        abi::store_u64(abi::c_arg(1), abi::stack_pointer(), PORT_OFFSET),
    ]);
    emit_hints(
        HINTS_OFFSET,
        true,
        SOCK_DGRAM,
        &mut instructions,
        &mut vregs,
    );
    // Default getaddrinfo service = NULL (valid whenever the host is non-NULL).
    instructions.push(abi::store_u64(
        abi::ZERO,
        abi::stack_pointer(),
        SERVICE_OFFSET,
    ));
    // Empty host binds all interfaces (NULL host + AI_PASSIVE).
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), HOST_OFFSET),
        abi::load_u64(&v9, &v9, 0),
        abi::compare_immediate(&v9, "0"),
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
        &mut vregs,
    );
    instructions.push(abi::branch(&resolved));
    instructions.extend([
        abi::label(&null_host),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), CSTR_OFFSET),
        // Bind-all: node is NULL, so service must be non-NULL. Stage the C string
        // "0" and point service at it (bug-113); the real port overwrites
        // sin_port afterward.
        abi::move_immediate(&v9, "Integer", "48"),
        abi::store_u64(&v9, abi::stack_pointer(), SERVICE_STR_OFFSET),
        abi::add_immediate(&v9, abi::stack_pointer(), SERVICE_STR_OFFSET),
        abi::store_u64(&v9, abi::stack_pointer(), SERVICE_OFFSET),
        abi::label(&resolved),
        // getaddrinfo(host, service, &hints, &res)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CSTR_OFFSET),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), SERVICE_OFFSET),
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
        // socket(ai_family, ai_socktype | SOCK_CLOEXEC, ai_protocol) (bug-499)
        abi::load_u64(&v9, abi::stack_pointer(), RES_OFFSET),
        abi::load_u32(abi::return_register(), &v9, 4),
        abi::load_u32(abi::c_arg(1), &v9, 8),
        abi::load_u32(abi::c_arg(2), &v9, 12),
    ]);
    emit_socket_type_cloexec(platform, &mut instructions);
    platform.emit_external_call(
        net_symbol(platform, NetSymbol::Socket),
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        // C `int` return (socket fd) — sign-extend before the signed compare
        // (bug-04/bug-170).
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&socket_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
    ]);
    emit_fd_cloexec_fallback(
        platform,
        symbol,
        FD_OFFSET,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        // Overwrite sin_port at ai_addr + 2/3 with the requested port.
        abi::load_u64(&v9, abi::stack_pointer(), RES_OFFSET),
        abi::load_u64(&v9, &v9, platform.addrinfo_addr_offset()),
        abi::load_u64(&v10, abi::stack_pointer(), PORT_OFFSET),
        abi::shift_right_immediate(&v11, &v10, 8),
        abi::store_u8(&v11, &v9, 2),
        abi::store_u8(&v10, &v9, 3),
        // bind(fd, ai_addr, ai_addrlen)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), RES_OFFSET),
        abi::load_u64(abi::c_arg(1), &v9, platform.addrinfo_addr_offset()),
        abi::load_u32(abi::c_arg(2), &v9, 16),
    ]);
    platform.emit_external_call(
        net_symbol(platform, NetSymbol::Bind),
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        // C `int` return (bind) — sign-extend before the signed compare
        // (bug-04/bug-170).
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&op_fail),
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
    emit_make_handle(
        symbol,
        FD_OFFSET,
        RESOURCE_TAG_UDP_SOCKET,
        &mut instructions,
        &mut relocations,
        &alloc_fail,
        &mut vregs,
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
    platform.emit_external_call(
        net_symbol(platform, NetSymbol::Close),
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
    platform.emit_external_call(
        net_symbol(platform, NetSymbol::FreeAddrInfo),
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_fail(
        symbol,
        "ErrNetworkFailed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&resolve_fail));
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

// ---------------------------------------------------------------------------
// net.receiveFrom / net.receiveTextFrom
// ---------------------------------------------------------------------------

/// `receiveFrom(sock, maxBytes)` / `receiveTextFrom(sock, maxBytes)`: receive a
/// single datagram with `recvfrom`, building a `Datagram` (`from`, `bytes`) or
/// `DatagramText` (`from`, `value`) record. The receive buffer is sized
/// `maxBytes + 1` so a datagram larger than `maxBytes` is detected (the returned
/// length exceeds `maxBytes`) and rejected with `ErrMessageTooLarge` rather than
/// silently truncated (§10.3).
pub(crate) fn lower_net_receive_from_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    text: bool,
) -> Result<NetBodyParts, String> {
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
    let recv_retry = format!("{symbol}_recv_retry");
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
        abi::store_u64(abi::c_arg(1), abi::stack_pointer(), MAX_OFFSET),
        abi::load_u64(&v9, abi::return_register(), FILE_OFFSET_CLOSED),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&closed),
        abi::load_u64(&v9, abi::return_register(), FILE_OFFSET_FD),
        abi::store_u64(&v9, abi::stack_pointer(), FD_OFFSET),
        abi::load_u64(&v10, abi::stack_pointer(), MAX_OFFSET),
        abi::compare_immediate(&v10, "0"),
        abi::branch_le(&invalid),
        // Allocate a maxBytes + 1 buffer to detect oversized datagrams.
        abi::add_immediate(abi::return_register(), &v10, 1),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), BUF_OFFSET),
        // recv_retry: an EINTR before any byte moved re-issues recvfrom without
        // re-allocating the buffer; fd/buf/max are reloaded from the stack and
        // addrlen is re-initialized so the identical call is repeated (bug-115).
        abi::label(&recv_retry),
        // recvfrom(fd, buf, maxBytes + 1, 0, &addr_storage, &addrlen)
        abi::move_immediate(&v9, "Integer", &SOCKADDR_STORAGE_SIZE.to_string()),
        abi::store_u64(&v9, abi::stack_pointer(), ADDRLEN_OFFSET),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), BUF_OFFSET),
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), MAX_OFFSET),
        abi::add_immediate(abi::c_arg(2), abi::c_arg(2), 1),
        abi::move_immediate(abi::c_arg(3), "Integer", "0"),
        abi::add_immediate(abi::c_arg(4), abi::stack_pointer(), ADDR_STORAGE_OFFSET),
        abi::add_immediate(abi::c_arg(5), abi::stack_pointer(), ADDRLEN_OFFSET),
    ]);
    // recvfrom takes SIX int args; on Win64 args 5/6 (&from, &fromlen) are stack
    // arguments above the 32-byte shadow, not rdi/rsi (bug-384). The shared
    // helper spills them through the outgoing-args sentinel that finalize_frame
    // reserves; POSIX passes all six in registers, byte-unchanged.
    let win64_six_args = platform.family() == PlatformFamily::Windows;
    crate::codegen::os::ffi::emit_external_int_call(
        platform,
        net_symbol(platform, NetSymbol::RecvFrom),
        symbol,
        6,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    if win64_six_args {
        // ws2_32 recvfrom returns a C `int` with unspecified upper 32 bits; POSIX
        // recvfrom returns a full 64-bit ssize_t. Without this, garbage high bits
        // make `n` read as a huge value and the `n > maxBytes` truncation check
        // below falsely fires ErrMessageTooLarge for a normal datagram.
        instructions.push(abi::sign_extend_word(
            abi::return_register(),
            abi::return_register(),
        ));
    }
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&recv_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), N_OFFSET),
        // Reject truncation: n > maxBytes means the datagram did not fit.
        abi::load_u64(&v9, abi::stack_pointer(), N_OFFSET),
        abi::load_u64(&v10, abi::stack_pointer(), MAX_OFFSET),
        abi::compare_registers(&v10, &v9),
        abi::branch_lt(&too_large),
        // Build the sender Address from the captured sockaddr.
        abi::add_immediate(&v9, abi::stack_pointer(), ADDR_STORAGE_OFFSET),
        abi::store_u64(&v9, abi::stack_pointer(), SADDR_PTR_OFFSET),
    ]);
    emit_address_from_sockaddr(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        net_symbol(platform, NetSymbol::Recv),
        SADDR_PTR_OFFSET,
        HOSTLEN_OFFSET,
        DST_OFFSET,
        AHOST_OFFSET,
        &alloc_fail,
        &addr_fail,
        &mut vregs,
    )?;
    instructions.push(abi::store_u64(
        abi::mfb_return(1),
        abi::stack_pointer(),
        ADDRPTR_OFFSET,
    ));
    if text {
        // Build a String: [u64 len][bytes][nul], validate UTF-8.
        emit_string_result_build(
            symbol,
            BUF_OFFSET,
            N_OFFSET,
            STR_OFFSET,
            &str_copy,
            &str_done,
            &alloc_fail,
            &encoding_error,
            &mut instructions,
            &mut relocations,
        );
    } else {
        // Build a List OF Byte with N elements.
        instructions.extend([
            abi::load_u64(&v10, abi::stack_pointer(), N_OFFSET),
            abi::move_immediate(&v11, "Integer", &byte_list_entry_stride().to_string()),
            abi::multiply_registers(&v12, &v10, &v11),
            abi::add_immediate(&v12, &v12, COLLECTION_HEADER_SIZE),
            abi::add_registers(abi::return_register(), &v12, &v10),
            abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        ]);
        emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
        instructions.extend([
            abi::move_register(&v15, abi::mfb_return(1)), // alloc result -> vreg base (plan-34-B Phase 3)
            abi::store_u64(&v15, abi::stack_pointer(), STR_OFFSET),
            abi::move_immediate(&v9, "Byte", &byte_list_block_kind().to_string()),
            abi::store_u8(&v9, &v15, COLLECTION_OFFSET_KIND),
            abi::move_immediate(&v9, "Byte", &COLLECTION_TYPE_NONE.to_string()),
            abi::store_u8(&v9, &v15, COLLECTION_OFFSET_KEY_TYPE),
            abi::move_immediate(&v9, "Byte", &COLLECTION_TYPE_BYTE.to_string()),
            abi::store_u8(&v9, &v15, COLLECTION_OFFSET_VALUE_TYPE),
            abi::move_immediate(&v9, "Byte", "1"),
            abi::store_u8(&v9, &v15, COLLECTION_OFFSET_FLAGS_VERSION),
            abi::load_u64(&v10, abi::stack_pointer(), N_OFFSET),
            abi::store_u64(&v10, &v15, COLLECTION_OFFSET_COUNT),
            abi::store_u64(&v10, &v15, COLLECTION_OFFSET_CAPACITY),
            abi::store_u64(&v10, &v15, COLLECTION_OFFSET_DATA_LENGTH),
            abi::store_u64(&v10, &v15, COLLECTION_OFFSET_DATA_CAPACITY),
            abi::add_immediate(&v11, &v15, COLLECTION_HEADER_SIZE),
            abi::move_immediate(&v12, "Integer", &byte_list_entry_stride().to_string()),
            abi::multiply_registers(&v13, &v10, &v12),
            abi::add_registers(&v14, &v11, &v13),
            abi::load_u64(&v15, abi::stack_pointer(), BUF_OFFSET),
            abi::move_immediate(&v9, "Integer", "0"),
            abi::label(&entry_loop),
            abi::compare_registers(&v9, &v10),
            abi::branch_eq(&entry_done),
        ]);
        // See the guard in `emit_read_body`: kind 2 has no entry array, and a
        // zero-stride cursor would rewrite one "entry" over the data region.
        if byte_list_entry_stride() != 0 {
            instructions.extend([
                abi::move_immediate(&v12, "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
                abi::store_u8(&v12, &v11, COLLECTION_ENTRY_OFFSET_FLAGS),
                abi::store_u64(abi::ZERO, &v11, COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
                abi::store_u64(abi::ZERO, &v11, COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
                abi::store_u64(&v9, &v11, COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
                abi::move_immediate(&v12, "Integer", "1"),
                abi::store_u64(&v12, &v11, COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
            ]);
        }
        instructions.extend([
            abi::add_registers(&v12, &v14, &v9),
            abi::load_u8(&v13, &v15, 0),
            abi::store_u8(&v13, &v12, 0),
            abi::add_immediate(&v15, &v15, 1),
        ]);
        if byte_list_entry_stride() != 0 {
            instructions.push(abi::add_immediate(&v11, &v11, COLLECTION_ENTRY_SIZE));
        }
        instructions.extend([
            abi::add_immediate(&v9, &v9, 1),
            abi::branch(&entry_loop),
            abi::label(&entry_done),
        ]);
    }
    // Allocate the Datagram/DatagramText record: [from Address][bytes/value].
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "16"),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::move_register(&v15, abi::mfb_return(1)), // alloc result -> vreg base; x1 kept for RESULT_VALUE_REGISTER
        abi::load_u64(&v9, abi::stack_pointer(), ADDRPTR_OFFSET),
        abi::store_u64(&v9, &v15, 0),
        abi::load_u64(&v9, abi::stack_pointer(), STR_OFFSET),
        abi::store_u64(&v9, &v15, 8),
        abi::move_register(RESULT_VALUE_REGISTER, abi::mfb_return(1)),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    // recv_fail: EAGAIN/EWOULDBLOCK is a read timeout; anything else is a
    // network failure.
    instructions.push(abi::label(&recv_fail));
    platform.emit_errno(
        symbol,
        (&v9).into(),
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        // bug-115: a signal that interrupts the blocking recvfrom before any byte
        // moved returns -1/EINTR; re-issue rather than reporting a spurious
        // network failure.
        abi::compare_immediate(&v9, EINTR_ERRNO),
        abi::branch_eq(&recv_retry),
        abi::compare_immediate(&v9, platform.socket_would_block_code()),
        abi::branch_eq(&timeout),
    ]);
    if platform.family() == PlatformFamily::Windows {
        // Winsock recvfrom on an oversized datagram truncates and returns
        // WSAEMSGSIZE via the error channel (POSIX instead returns the filled
        // count, caught by the `n > maxBytes` check above). Map it to the same
        // ErrMessageTooLarge (bug-384).
        instructions.extend([
            abi::compare_immediate(&v9, platform.socket_message_size_code()),
            abi::branch_eq(&too_large),
            // SO_RCVTIMEO timeout is WSAETIMEDOUT on Winsock (bug-109).
            abi::compare_immediate(&v9, WSAETIMEDOUT),
            abi::branch_eq(&timeout),
        ]);
    }
    emit_fail(
        symbol,
        "ErrNetworkFailed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&timeout));
    emit_fail(
        symbol,
        "ErrTimeout",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&too_large));
    emit_fail(
        symbol,
        "ErrMessageTooLarge",
        &mut instructions,
        &mut relocations,
        &done,
    );
    if text {
        instructions.push(abi::label(&encoding_error));
        emit_fail(
            symbol,
            "ErrEncoding",
            &mut instructions,
            &mut relocations,
            &done,
        );
    }
    instructions.push(abi::label(&invalid));
    emit_fail(
        symbol,
        "ErrInvalidArgument",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&addr_fail));
    emit_fail(
        symbol,
        "ErrAddressInvalid",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&closed));
    emit_fail(
        symbol,
        "ErrResourceClosed",
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

// ---------------------------------------------------------------------------
// net.sendTo / net.sendTextTo
// ---------------------------------------------------------------------------

/// `sendTo(sock, address, bytes)` / `sendTextTo(sock, address, value)`: resolve
/// the destination `Address` with `getaddrinfo` and send a single datagram with
/// `sendto`. An oversized datagram (`EMSGSIZE`) maps to `ErrMessageTooLarge`.
pub(crate) fn lower_net_send_to_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    text: bool,
) -> Result<NetBodyParts, String> {
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
    let send_retry = format!("{symbol}_send_retry");
    let send_eintr_skip = format!("{symbol}_send_eintr_skip");
    let send_fail = format!("{symbol}_send_fail");
    let timeout = format!("{symbol}_timeout");
    let too_large = format!("{symbol}_too_large");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");

    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    let v12 = vregs.next();
    let v13 = vregs.next();
    let v14 = vregs.next();
    instructions.extend([
        // x0 = UdpSocket record; reject if closed.
        abi::load_u64(&v9, abi::return_register(), FILE_OFFSET_CLOSED),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&closed),
        abi::load_u64(&v9, abi::return_register(), FILE_OFFSET_FD),
        abi::store_u64(&v9, abi::stack_pointer(), FD_OFFSET),
        // x1 = Address record { host String ptr @0, port @8 }.
        abi::load_u64(&v9, abi::c_arg(1), 0),
        abi::store_u64(&v9, abi::stack_pointer(), HOST_OFFSET),
        abi::load_u64(&v9, abi::c_arg(1), 8),
        abi::store_u64(&v9, abi::stack_pointer(), PORT_OFFSET),
    ]);
    // bug-497 / bug-508: one payload view for every backend — the text form
    // as before, the byte form after a header check (`push_write_payload_view`).
    let bad_payload = format!("{symbol}_bad_payload");
    push_write_payload_view(
        &mut instructions,
        text,
        abi::c_arg(2),
        &v10,
        &v11,
        &v14,
        &v12,
        &v13,
        DLEN_OFFSET,
        DATA_OFFSET,
        &bad_payload,
    );
    emit_hints(
        HINTS_OFFSET,
        false,
        SOCK_DGRAM,
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
        // getaddrinfo(host, NULL, &hints, &res)
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
        // send_retry: an EINTR before any byte was sent re-issues sendto while
        // res is still live (freeaddrinfo has not run yet); fd/data/dlen/ai_addr
        // are reloaded from the stack so the identical call is repeated (bug-115).
        abi::label(&send_retry),
        // Force the requested port into sin_port at ai_addr + 2/3.
        abi::load_u64(&v9, abi::stack_pointer(), RES_OFFSET),
        abi::load_u64(&v9, &v9, platform.addrinfo_addr_offset()),
        abi::load_u64(&v10, abi::stack_pointer(), PORT_OFFSET),
        abi::shift_right_immediate(&v11, &v10, 8),
        abi::store_u8(&v11, &v9, 2),
        abi::store_u8(&v10, &v9, 3),
        // sendto(fd, data, dlen, 0, ai_addr, ai_addrlen)
        abi::load_u64(&v9, abi::stack_pointer(), RES_OFFSET),
        abi::load_u64(abi::c_arg(4), &v9, platform.addrinfo_addr_offset()),
        abi::load_u32(abi::c_arg(5), &v9, 16),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), DATA_OFFSET),
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), DLEN_OFFSET),
        abi::move_immediate(abi::c_arg(3), "Integer", "0"),
    ]);
    // sendto takes SIX int args; on Win64 args 5/6 (ai_addr, ai_addrlen) are
    // stack arguments above the shadow, not rdi/rsi (bug-384). The shared helper
    // spills them through the outgoing-args sentinel; POSIX unchanged.
    let win64_sendto = platform.family() == PlatformFamily::Windows;
    crate::codegen::os::ffi::emit_external_int_call(
        platform,
        net_symbol(platform, NetSymbol::SendTo),
        symbol,
        6,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    if win64_sendto {
        instructions.push(abi::sign_extend_word(
            abi::return_register(),
            abi::return_register(),
        ));
    }
    instructions.push(abi::store_u64(
        abi::return_register(),
        abi::stack_pointer(),
        RET_OFFSET,
    ));
    // Capture errno before freeaddrinfo can disturb it.
    platform.emit_errno(
        symbol,
        (&v9).into(),
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.push(abi::store_u64(&v9, abi::stack_pointer(), ERRNO_OFFSET));
    // bug-115: a signal that interrupts the blocking sendto before any byte is
    // sent returns -1/EINTR; re-issue the sendto (res is still live) rather than
    // reporting a spurious network failure. A datagram send is all-or-nothing, so
    // a non-negative return is a completed send and must not be retried.
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), RET_OFFSET),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ge(&send_eintr_skip),
        abi::load_u64(&v9, abi::stack_pointer(), ERRNO_OFFSET),
        abi::compare_immediate(&v9, EINTR_ERRNO),
        abi::branch_eq(&send_retry),
        abi::label(&send_eintr_skip),
    ]);
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
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), RET_OFFSET),
        abi::compare_immediate(&v9, "0"),
        abi::branch_lt(&send_fail),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    // send_fail: classify by captured errno.
    instructions.extend([
        abi::label(&send_fail),
        abi::load_u64(&v9, abi::stack_pointer(), ERRNO_OFFSET),
        abi::compare_immediate(&v9, platform.socket_would_block_code()),
        abi::branch_eq(&timeout),
        abi::load_u64(&v9, abi::stack_pointer(), ERRNO_OFFSET),
        abi::compare_immediate(&v9, platform.socket_message_size_code()),
        abi::branch_eq(&too_large),
    ]);
    emit_fail(
        symbol,
        "ErrNetworkFailed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&timeout));
    emit_fail(
        symbol,
        "ErrTimeout",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&too_large));
    emit_fail(
        symbol,
        "ErrMessageTooLarge",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&resolve_fail));
    emit_fail(
        symbol,
        "ErrAddressNotFound",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&closed));
    emit_fail(
        symbol,
        "ErrResourceClosed",
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
    if !text {
        // bug-497: the byte form was handed a block whose header is not a
        // `List OF Byte`'s — refuse rather than read a length out of its bytes.
        instructions.push(abi::label(&bad_payload));
        emit_fail(
            symbol,
            "ErrInvalidArgument",
            &mut instructions,
            &mut relocations,
            &done,
        );
    }
    instructions.extend([abi::label(&done), abi::return_()]);
    {
        Ok((instructions, relocations, FRAME_SIZE))
    }
}
