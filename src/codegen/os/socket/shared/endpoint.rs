//! plan-176-O: `net.connectTcp` / `net.listenTcp` — the TCP endpoint helpers. The socket
//! constants, the address helpers and `lower_net_address_helper` live in
//! `codegen/os/socket/shared/mod.rs`.

use super::*;

// ---------------------------------------------------------------------------
// net.connectTcp / net.listenTcp
// ---------------------------------------------------------------------------

/// Shared lowering for `connectTcp` and `listenTcp`: both resolve the host with
/// `getaddrinfo`, create a socket, and then either `connect` or `bind`+`listen`.
fn lower_net_endpoint_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    listen: bool,
    address: bool,
) -> Result<NetBodyParts, String> {
    const FRAME_SIZE: usize = 192;
    const HOST_OFFSET: usize = 8;
    const PORT_OFFSET: usize = 16;
    const EXTRA_OFFSET: usize = 24; // timeoutMs (connect) or backlog (listen)
    const RES_OFFSET: usize = 32;
    const FD_OFFSET: usize = 40;
    const CSTR_OFFSET: usize = 48;
    const ONE_OFFSET: usize = 56;
    const HINTS_OFFSET: usize = 64; // 64..112
    const FLAGS_OFFSET: usize = 112; // saved socket flags for non-blocking connect
    const POLLFD_OFFSET: usize = 120; // pollfd { fd; events; revents }
    const SOERR_OFFSET: usize = 128; // getsockopt SO_ERROR output
    const SOLEN_OFFSET: usize = 136; // getsockopt option length
                                     // getaddrinfo `service` pointer (NULL for a resolved host; the `"0"` C string
                                     // below for a NULL/bind-all host, since getaddrinfo rejects node==service==NULL
                                     // — the real port is patched into sin_port afterward). bug-113.
    const SERVICE_OFFSET: usize = 144;
    const SERVICE_STR_OFFSET: usize = 152; // holds the bytes "0\0…"
                                           // plan-73-C: the former bounded `DEFAULT_CONNECT_TIMEOUT_MS` (120 s, bug-261) is
                                           // removed. Under the timeout convention an omitted connect timeout BLOCKS until
                                           // the connection resolves (like every other omit); a caller that must bound the
                                           // wedge passes a positive `timeoutMs` (http does, via `__HTTP_CONNECT_TIMEOUT_MS`).

    let null_host = format!("{symbol}_null_host");
    let resolved = format!("{symbol}_resolved");
    let resolve_fail = format!("{symbol}_resolve_fail");
    let socket_fail = format!("{symbol}_socket_fail");
    let op_fail = format!("{symbol}_op_fail");
    let connect_use_timeout = format!("{symbol}_connect_use_timeout");
    let connect_invalid = format!("{symbol}_connect_invalid");
    let port_invalid = format!("{symbol}_port_invalid");
    let connect_ts_ok = format!("{symbol}_connect_ts_ok");
    let nb_connected = format!("{symbol}_nb_connected");
    let connect_poll_retry = format!("{symbol}_connect_poll_retry");
    let connect_poll_ready = format!("{symbol}_connect_poll_ready");
    let connect_timeout = format!("{symbol}_connect_timeout");
    let connect_timeout_ok = format!("{symbol}_connect_timeout_ok");
    let listen_backlog_ok = format!("{symbol}_listen_backlog_ok");
    let connected_done = format!("{symbol}_connected_done");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");

    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    let mut vregs = Vregs::new();
    let host_scratch = HelperScratch::declare(&mut vregs, &mut instructions);
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    if address {
        // x0 = the flat `net::Address` (plan-132): slot 0 holds the host `String`'s
        // block-relative offset, so the host is `x0 + [x0]`; the port is slot 1.
        // x1 = timeoutMs.
        instructions.extend([
            abi::load_u64(&v9, abi::return_register(), 0),
            abi::add_registers(&v9, abi::return_register(), &v9),
            abi::store_u64(&v9, abi::stack_pointer(), HOST_OFFSET),
            abi::load_u64(&v9, abi::return_register(), 8),
            abi::store_u64(&v9, abi::stack_pointer(), PORT_OFFSET),
            abi::store_u64(abi::c_arg(1), abi::stack_pointer(), EXTRA_OFFSET),
        ]);
    } else {
        instructions.extend([
            abi::store_u64(abi::return_register(), abi::stack_pointer(), HOST_OFFSET),
            abi::store_u64(abi::c_arg(1), abi::stack_pointer(), PORT_OFFSET),
            abi::store_u64(abi::c_arg(2), abi::stack_pointer(), EXTRA_OFFSET),
        ]);
    }
    // bug-876: refuse a port outside 0..=65535 before any resolver or socket call.
    emit_port_range_check(PORT_OFFSET, &port_invalid, &mut instructions, &mut vregs);
    if !listen {
        // plan-73-C: validate the connect `timeoutMs` up front, before the resolver
        // or socket is allocated, so a rejection leaks nothing. The omitted overload
        // pads the unbounded sentinel (allowed → block); any OTHER negative is
        // `ErrInvalidArgument`. `0`/`> 0` pass through (the sentinel is converted to a
        // -1 infinite poll at the connect wait; `EXTRA_OFFSET` is the backlog for the
        // listen path, which does not take this check).
        instructions.extend([
            abi::load_u64(&v9, abi::stack_pointer(), EXTRA_OFFSET),
            abi::move_immediate(&v10, "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
            abi::compare_registers(&v9, &v10),
            abi::branch_eq(&connect_ts_ok),
            abi::compare_immediate(&v9, "0"),
            abi::branch_lt(&connect_invalid),
            abi::label(&connect_ts_ok),
        ]);
    }
    emit_hints(
        HINTS_OFFSET,
        listen,
        SOCK_STREAM,
        &mut instructions,
        &mut vregs,
    );
    // Default getaddrinfo service = NULL (valid whenever the host is non-NULL).
    instructions.push(abi::store_u64(
        abi::ZERO,
        abi::stack_pointer(),
        SERVICE_OFFSET,
    ));
    // Choose host C string. An empty host on a listener binds all interfaces
    // (NULL host + AI_PASSIVE).
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), HOST_OFFSET),
        abi::load_u64(&v9, &v9, 0),
        abi::compare_immediate(&v9, "0"),
    ]);
    if listen {
        instructions.push(abi::branch_eq(&null_host));
    }
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
    instructions.push(abi::branch(&resolved));
    if listen {
        instructions.extend([
            abi::label(&null_host),
            abi::store_u64(abi::ZERO, abi::stack_pointer(), CSTR_OFFSET),
            // Bind-all: node is NULL, so service must be non-NULL. Stage the C
            // string "0" (0x30 then a zero terminator) and point service at it,
            // so getaddrinfo(NULL, "0", &hints|AI_PASSIVE, …) returns the wildcard
            // address instead of EAI_NONAME (bug-113). The real port overwrites
            // sin_port afterward.
            abi::move_immediate(&v9, "Integer", "48"),
            abi::store_u64(&v9, abi::stack_pointer(), SERVICE_STR_OFFSET),
            abi::add_immediate(&v9, abi::stack_pointer(), SERVICE_STR_OFFSET),
            abi::store_u64(&v9, abi::stack_pointer(), SERVICE_OFFSET),
        ]);
    }
    instructions.extend([
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
        // Overwrite sin_port at ai_addr + 2/3 with the requested port (network
        // byte order).
        abi::load_u64(&v9, abi::stack_pointer(), RES_OFFSET),
        abi::load_u64(&v9, &v9, platform.addrinfo_addr_offset()),
        abi::load_u64(&v10, abi::stack_pointer(), PORT_OFFSET),
        abi::shift_right_immediate(&v11, &v10, 8),
        abi::store_u8(&v11, &v9, 2),
        abi::store_u8(&v10, &v9, 3),
    ]);
    if listen {
        // setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &one, 4) - best effort.
        instructions.extend([
            abi::move_immediate(&v9, "Integer", "1"),
            abi::store_u64(&v9, abi::stack_pointer(), ONE_OFFSET),
            abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
            abi::move_immediate(abi::c_arg(1), "Integer", platform.sol_socket()),
            abi::move_immediate(abi::c_arg(2), "Integer", platform.so_reuseaddr()),
            abi::add_immediate(abi::c_arg(3), abi::stack_pointer(), ONE_OFFSET),
            abi::move_immediate(abi::c_arg(4), "Integer", "4"),
        ]);
        // setsockopt takes FIVE int args; on Win64 the 5th (optlen) is a stack
        // argument above the shadow, not rdi (bug-384). POSIX passes it in a
        // register, byte-unchanged.
        crate::codegen::os::ffi::emit_external_int_call(
            platform,
            net_symbol(platform, NetSymbol::SetSockOpt),
            symbol,
            5,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        // bind(fd, ai_addr, ai_addrlen)
        instructions.extend([
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
            // listen(fd, backlog)
            abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
            abi::load_u64(abi::c_arg(1), abi::stack_pointer(), EXTRA_OFFSET),
            // Clamp backlog to INT_MAX: listen() takes a C `int`, so a 64-bit value
            // with bit 31 set would be passed as a negative backlog (bug-239).
            abi::move_immediate(&v9, "Integer", "2147483647"),
            abi::compare_registers(abi::c_arg(1), &v9),
            abi::branch_le(&listen_backlog_ok),
            abi::move_register(abi::c_arg(1), &v9),
            abi::label(&listen_backlog_ok),
        ]);
        platform.emit_external_call(
            net_symbol(platform, NetSymbol::Listen),
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            // C `int` return (listen) — sign-extend before the signed compare
            // (bug-04/bug-170).
            abi::sign_extend_word(abi::return_register(), abi::return_register()),
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_lt(&op_fail),
        ]);
    } else {
        // plan-73-C timeout convention. Every connect takes the non-blocking-connect
        // + `poll` path: the OMITTED overload padded the unbounded sentinel → block
        // until the connection resolves, i.e. poll() with a -1 timeout; `0` is one
        // immediate, non-blocking attempt (poll with a 0 timeout → `ErrTimeout`
        // unless it completed at once); a positive value is honored (clamped to
        // INT_MAX below). Negatives were rejected up front. The former bounded
        // `DEFAULT_CONNECT_TIMEOUT_MS` safety default is gone — callers own the
        // never-wedge property by passing a positive timeout (http does, via
        // `__HTTP_CONNECT_TIMEOUT_MS`). Blocking mode is restored on success.
        instructions.extend([
            abi::load_u64(&v9, abi::stack_pointer(), EXTRA_OFFSET),
            abi::move_immediate(&v10, "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
            abi::compare_registers(&v9, &v10),
            abi::branch_ne(&connect_use_timeout),
            // Omitted: convert the sentinel to -1 so poll() below blocks indefinitely.
            abi::bitwise_not(&v9, abi::ZERO),
            abi::store_u64(&v9, abi::stack_pointer(), EXTRA_OFFSET),
            abi::label(&connect_use_timeout),
        ]);
        if platform.family() != PlatformFamily::Windows {
            // flags = fcntl(fd, F_GETFL, 0). Winsock's ioctlsocket(FIONBIO) is
            // stateless, so Windows skips the read and emit_set_nonblocking ignores
            // FLAGS_OFFSET.
            instructions.extend([
                abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
                abi::move_immediate(abi::c_arg(1), "Integer", "3"),
                abi::move_immediate(abi::c_arg(2), "Integer", "0"),
            ]);
            platform.emit_variadic_external_call(
                net_symbol(platform, NetSymbol::Fcntl),
                symbol,
                platform_imports,
                &mut instructions,
                &mut relocations,
            )?;
            instructions.push(abi::store_u64(
                abi::return_register(),
                abi::stack_pointer(),
                FLAGS_OFFSET,
            ));
        }
        // fcntl(fd, F_SETFL, flags | O_NONBLOCK) — Windows: ioctlsocket(fd, FIONBIO, &1)
        platform.emit_set_nonblocking(
            FD_OFFSET,
            FLAGS_OFFSET,
            vregs.next().into(),
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        // connect(fd, ai_addr, ai_addrlen)
        instructions.extend([
            abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
            abi::load_u64(&v9, abi::stack_pointer(), RES_OFFSET),
            abi::load_u64(abi::c_arg(1), &v9, platform.addrinfo_addr_offset()),
            abi::load_u32(abi::c_arg(2), &v9, 16),
        ]);
        platform.emit_external_call(
            net_symbol(platform, NetSymbol::Connect),
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            // C `int` return (connect) — sign-extend before comparing so a success
            // 0 with dirty upper x0 bits is still recognized (bug-04/bug-170).
            abi::sign_extend_word(abi::return_register(), abi::return_register()),
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_eq(&nb_connected),
        ]);
        // In progress? Anything other than EINPROGRESS is a hard failure.
        platform.emit_errno(
            symbol,
            (&v9).into(),
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::compare_immediate(&v9, platform.socket_in_progress_code()),
            abi::branch_ne(&op_fail),
            // poll(&pollfd { fd, POLLOUT }, 1, timeoutMs); connect_poll_retry
            // re-runs the pollfd rebuild + poll on an EINTR (bug-115).
            abi::label(&connect_poll_retry),
            abi::load_u64(&v9, abi::stack_pointer(), FD_OFFSET),
            abi::store_u64(&v9, abi::stack_pointer(), POLLFD_OFFSET),
        ]);
        // The connect wait is a WRITABILITY poll. This used to be written inline
        // in the POSIX layout on every platform — `events` at +4 with POLLOUT (4)
        // — which on Windows lands inside the 8-byte `SOCKET` of a `WSAPOLLFD`
        // and leaves the real `events` (at +8) zero. WSAPoll rejects a zero
        // `events`, so every bounded connect failed. `revents` occupies +10..+11
        // on Windows, four bytes below `SOERR_OFFSET`, which is only written
        // AFTER this poll returns.
        emit_pollfd_events_for(platform, POLLFD_OFFSET, true, &mut instructions, &mut vregs);
        instructions.extend([
            abi::add_immediate(abi::return_register(), abi::stack_pointer(), POLLFD_OFFSET),
            abi::move_immediate(abi::c_arg(1), "Integer", "1"),
            abi::load_u64(abi::c_arg(2), abi::stack_pointer(), EXTRA_OFFSET),
            // Clamp the connect timeout to INT_MAX: poll() takes a C `int`, so a
            // 64-bit value with bit 31 set would block forever (bug-239).
            abi::move_immediate(&v11, "Integer", "2147483647"),
            abi::compare_registers(abi::c_arg(2), &v11),
            abi::branch_le(&connect_timeout_ok),
            abi::move_register(abi::c_arg(2), &v11),
            abi::label(&connect_timeout_ok),
        ]);
        platform.emit_external_call(
            net_symbol(platform, NetSymbol::Poll),
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            // C `int` return (poll) — sign-extend before the signed compares; a -1
            // poll error read as large-positive would wrongly take branch_gt
            // (connect_poll_ready) and treat the socket as writable (bug-04/bug-170).
            abi::sign_extend_word(abi::return_register(), abi::return_register()),
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_eq(&connect_timeout),
            abi::branch_gt(&connect_poll_ready),
        ]);
        // bug-115: a negative poll return is either EINTR (re-issue the poll) or a
        // genuine failure. poll goes through libc here, so read errno.
        platform.emit_errno(
            symbol,
            (&v9).into(),
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::compare_immediate(&v9, EINTR_ERRNO),
            abi::branch_eq(&connect_poll_retry),
            abi::branch(&op_fail),
            abi::label(&connect_poll_ready),
            // getsockopt(fd, SOL_SOCKET, SO_ERROR, &err, &len)
            abi::move_immediate(&v9, "Integer", "4"),
            abi::store_u64(&v9, abi::stack_pointer(), SOLEN_OFFSET),
            abi::store_u64(abi::ZERO, abi::stack_pointer(), SOERR_OFFSET),
            abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
            abi::move_immediate(abi::c_arg(1), "Integer", platform.sol_socket()),
            abi::move_immediate(abi::c_arg(2), "Integer", platform.so_error()),
            abi::add_immediate(abi::c_arg(3), abi::stack_pointer(), SOERR_OFFSET),
            abi::add_immediate(abi::c_arg(4), abi::stack_pointer(), SOLEN_OFFSET),
        ]);
        // getsockopt takes FIVE int args; on Win64 the 5th (&optlen) is a stack
        // argument above the shadow, not rdi (bug-384) — a garbage optlen makes
        // getsockopt fail and the non-blocking connect never resolves. POSIX
        // passes it in a register, byte-unchanged.
        crate::codegen::os::ffi::emit_external_int_call(
            platform,
            net_symbol(platform, NetSymbol::GetSockOpt),
            symbol,
            5,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            // C `int` return (getsockopt) — sign-extend before the signed compare
            // (bug-04/bug-170).
            abi::sign_extend_word(abi::return_register(), abi::return_register()),
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_lt(&op_fail),
            abi::load_u32(&v9, abi::stack_pointer(), SOERR_OFFSET),
            abi::compare_immediate(&v9, "0"),
            abi::branch_ne(&op_fail),
            // Connected: restore blocking mode with fcntl(fd, F_SETFL, flags).
            abi::label(&nb_connected),
        ]);
        if platform.family() == PlatformFamily::Windows {
            // Winsock: ioctlsocket(fd, FIONBIO, &0) — no flags word to restore.
            platform.emit_restore_blocking(
                FD_OFFSET,
                FLAGS_OFFSET,
                symbol,
                platform_imports,
                &mut instructions,
                &mut relocations,
            )?;
        } else {
            instructions.extend([
                abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
                abi::move_immediate(abi::c_arg(1), "Integer", "4"),
                abi::load_u64(abi::c_arg(2), abi::stack_pointer(), FLAGS_OFFSET),
            ]);
            platform.emit_variadic_external_call(
                net_symbol(platform, NetSymbol::Fcntl),
                symbol,
                platform_imports,
                &mut instructions,
                &mut relocations,
            )?;
        }
        // Both the caller-timeout and default-timeout connects converge here after
        // restoring blocking mode; the old unbounded blocking-connect path (taken
        // when timeoutMs <= 0) is gone — that case now uses the bounded default
        // deadline above (bug-261).
        instructions.push(abi::label(&connected_done));
    }
    // freeaddrinfo(res)
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
    emit_make_handle(
        symbol,
        FD_OFFSET,
        if listen {
            RESOURCE_TAG_LISTENER
        } else {
            RESOURCE_TAG_SOCKET
        },
        &mut instructions,
        &mut relocations,
        &alloc_fail,
        &mut vregs,
    );
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    // op_fail / socket_fail: free resources then report network failure. op_fail
    // closes the socket fd (loaded from FD_OFFSET below) before falling through to
    // socket_fail, which frees the addrinfo — so no fd or addrinfo leaks on the
    // error paths (bug-268 / OS-06: the earlier "fd leaks" note was stale).
    // plan-73-C: a negative (non-sentinel) connect `timeoutMs` → ErrInvalidArgument.
    // Reached from the up-front check before any socket/resolver allocation, so there
    // is nothing to clean up. Emitted only on the connect path (listen never branches
    // here) to keep the listen codegen byte-identical.
    if !listen {
        instructions.push(abi::label(&connect_invalid));
        emit_fail(
            symbol,
            "ErrInvalidArgument",
            &mut instructions,
            &mut relocations,
            &done,
        );
    }
    // bug-876: a port outside 0..=65535, reached before any resolver/socket
    // allocation, so there is nothing to release.
    instructions.push(abi::label(&port_invalid));
    emit_fail(
        symbol,
        "ErrAddressInvalid",
        &mut instructions,
        &mut relocations,
        &done,
    );
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
    // A connect that did not complete before its deadline: close the pending
    // socket, release the resolver results, and report a timeout.
    instructions.push(abi::label(&connect_timeout));
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
        "ErrTimeout",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&resolve_fail));
    if listen {
        emit_fail(
            symbol,
            "ErrAddressInvalid",
            &mut instructions,
            &mut relocations,
            &done,
        );
    } else {
        emit_fail(
            symbol,
            "ErrAddressNotFound",
            &mut instructions,
            &mut relocations,
            &done,
        );
    }
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

pub(crate) fn lower_net_connect_tcp_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<NetBodyParts, String> {
    lower_net_endpoint_helper(symbol, platform_imports, platform, false, false)
}

pub(crate) fn lower_net_connect_tcp_addr_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<NetBodyParts, String> {
    lower_net_endpoint_helper(symbol, platform_imports, platform, false, true)
}

pub(crate) fn lower_net_listen_tcp_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<NetBodyParts, String> {
    lower_net_endpoint_helper(symbol, platform_imports, platform, true, false)
}
