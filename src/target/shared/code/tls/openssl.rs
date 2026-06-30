//! OpenSSL `dlopen`/`dlsym` TLS backend: the socket-timeout connect/read/
//! write/close helpers and their OpenSSL machinery (see `super` for the
//! shared emit helpers and `macos` for the Network.framework backend).

use std::collections::HashMap;

use super::*;
use crate::arch::aarch64::abi;

pub(crate) fn lower_tls_connect_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>, Vec<CodeStackSlot>), String> {
    if platform.target().contains("macos") {
        return macos::lower_tls_connect_macos(symbol, platform_imports, platform);
    }
    const FRAME_SIZE: usize = 256;
    const LR_OFFSET: usize = 0;
    const FD_OFFSET: usize = 8;
    const HANDLE_OFFSET: usize = 16;
    const CTX_OFFSET: usize = 24;
    const SSL_OFFSET: usize = 32;
    const FNPTR_OFFSET: usize = 40;
    const HOST_OFFSET: usize = 48; // connect: host String ptr
    const PORT_OFFSET: usize = 56; // connect: port
    const SNAME_OFFSET: usize = 64; // serverName String ptr
    const HOSTCSTR_OFFSET: usize = 72;
    const SNICSTR_OFFSET: usize = 80;
    const RES_OFFSET: usize = 88; // addrinfo*
    const HINTS_OFFSET: usize = 96; // 96..144
    const TIMEOUT_OFFSET: usize = 144; // timeoutMs
    const FLAGS_OFFSET: usize = 152; // saved socket flags for non-blocking connect
    const POLLFD_OFFSET: usize = 160; // pollfd { fd; events; revents }
    const SOERR_OFFSET: usize = 168; // getsockopt SO_ERROR output
    const SOLEN_OFFSET: usize = 176; // getsockopt option length
    const TIMEVAL_OFFSET: usize = 184; // 184..200: tv_sec (8) + tv_usec (8)

    let resolve_fail = format!("{symbol}_resolve_fail");
    let net_fail = format!("{symbol}_net_fail");
    let net_fail_fd = format!("{symbol}_net_fail_fd");
    let connect_timeout = format!("{symbol}_connect_timeout");
    let blocking_connect = format!("{symbol}_blocking_connect");
    let nb_connected = format!("{symbol}_nb_connected");
    let connected = format!("{symbol}_connected");
    let hs_timeout_set = format!("{symbol}_hs_timeout_set");
    let hs_timeout_clear = format!("{symbol}_hs_timeout_clear");
    let tls_fail = format!("{symbol}_tls_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let load_fail = format!("{symbol}_load_fail");
    let use_sname = format!("{symbol}_use_sname");
    let sni_ready = format!("{symbol}_sni_ready");
    let done = format!("{symbol}_done");

    let addr_off = platform.addrinfo_addr_offset();
    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        LR_OFFSET,
    ));

    // x0 = host; x1 = port; x2 = timeoutMs; x3 = serverName.
    instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), HOST_OFFSET),
        abi::store_u64("x1", abi::stack_pointer(), PORT_OFFSET),
        abi::store_u64("x2", abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::store_u64("x3", abi::stack_pointer(), SNAME_OFFSET),
    ]);
    // Resolve + connect a TCP socket. Zero a 48-byte hints block and set
    // ai_family = AF_INET, ai_socktype = SOCK_STREAM.
    for offset in (0..48).step_by(8) {
        instructions.push(abi::store_u64(
            "x31",
            abi::stack_pointer(),
            HINTS_OFFSET + offset,
        ));
    }
    instructions.extend([
        abi::move_immediate("x9", "Integer", HINTS_FAMILY_WORD),
        abi::store_u64("x9", abi::stack_pointer(), HINTS_OFFSET),
        abi::move_immediate("x9", "Integer", SOCK_STREAM),
        abi::store_u64("x9", abi::stack_pointer(), HINTS_OFFSET + 8),
    ]);
    emit_cstring(
        symbol,
        "host",
        HOST_OFFSET,
        HOSTCSTR_OFFSET,
        &alloc_fail,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::load_u64(
            abi::return_register(),
            abi::stack_pointer(),
            HOSTCSTR_OFFSET,
        ),
        abi::move_immediate("x1", "Integer", "0"),
        abi::add_immediate("x2", abi::stack_pointer(), HINTS_OFFSET),
        abi::add_immediate("x3", abi::stack_pointer(), RES_OFFSET),
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
        abi::load_u64("x9", abi::stack_pointer(), RES_OFFSET),
        abi::load_u32(abi::return_register(), "x9", 4),
        abi::load_u32("x1", "x9", 8),
        abi::load_u32("x2", "x9", 12),
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
        abi::branch_lt(&net_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        // Overwrite sin_port (ai_addr + 2/3) with the requested port.
        abi::load_u64("x9", abi::stack_pointer(), RES_OFFSET),
        abi::load_u64("x9", "x9", addr_off),
        abi::load_u64("x10", abi::stack_pointer(), PORT_OFFSET),
        abi::shift_right_immediate("x11", "x10", 8),
        abi::store_u8("x11", "x9", 2),
        abi::store_u8("x10", "x9", 3),
    ]);
    // Connect the socket, bounded by timeoutMs when > 0 (non-blocking connect +
    // poll, then restore blocking mode), else a plain blocking connect. Mirrors
    // net::connectTcp. DNS (getaddrinfo above) is not bounded by timeoutMs.
    instructions.extend([
        abi::load_u64("x9", abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::compare_immediate("x9", "0"),
        abi::branch_le(&blocking_connect),
        // flags = fcntl(fd, F_GETFL, 0)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::move_immediate("x1", "Integer", "3"),
        abi::move_immediate("x2", "Integer", "0"),
    ]);
    platform.emit_variadic_call(
        "fcntl",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), FLAGS_OFFSET),
        // fcntl(fd, F_SETFL, flags | O_NONBLOCK)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::move_immediate("x1", "Integer", "4"),
        abi::load_u64("x2", abi::stack_pointer(), FLAGS_OFFSET),
        abi::move_immediate("x9", "Integer", platform.o_nonblock()),
        abi::or_registers("x2", "x2", "x9"),
    ]);
    platform.emit_variadic_call(
        "fcntl",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    // connect(fd, ai_addr, ai_addrlen)
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64("x9", abi::stack_pointer(), RES_OFFSET),
        abi::load_u64("x1", "x9", addr_off),
        abi::load_u32("x2", "x9", 16),
    ]);
    platform.emit_libc_call(
        "connect",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&nb_connected),
    ]);
    // In progress? Anything other than EINPROGRESS is a hard failure.
    platform.emit_errno(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate("x9", platform.einprogress()),
        abi::branch_ne(&net_fail_fd),
        // poll(&pollfd { fd, POLLOUT }, 1, timeoutMs)
        abi::load_u64("x9", abi::stack_pointer(), FD_OFFSET),
        abi::store_u64("x9", abi::stack_pointer(), POLLFD_OFFSET),
        abi::move_immediate("x10", "Integer", "4"), // POLLOUT
        abi::store_u8("x10", abi::stack_pointer(), POLLFD_OFFSET + 4),
        abi::store_u8("x31", abi::stack_pointer(), POLLFD_OFFSET + 5),
        abi::store_u8("x31", abi::stack_pointer(), POLLFD_OFFSET + 6),
        abi::store_u8("x31", abi::stack_pointer(), POLLFD_OFFSET + 7),
        abi::add_immediate(abi::return_register(), abi::stack_pointer(), POLLFD_OFFSET),
        abi::move_immediate("x1", "Integer", "1"),
        abi::load_u64("x2", abi::stack_pointer(), TIMEOUT_OFFSET),
    ]);
    platform.emit_libc_call(
        "poll",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&net_fail_fd),
        abi::branch_eq(&connect_timeout),
        // getsockopt(fd, SOL_SOCKET, SO_ERROR, &err, &len)
        abi::move_immediate("x9", "Integer", "4"),
        abi::store_u64("x9", abi::stack_pointer(), SOLEN_OFFSET),
        abi::store_u64("x31", abi::stack_pointer(), SOERR_OFFSET),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::move_immediate("x1", "Integer", platform.sol_socket()),
        abi::move_immediate("x2", "Integer", platform.so_error()),
        abi::add_immediate("x3", abi::stack_pointer(), SOERR_OFFSET),
        abi::add_immediate("x4", abi::stack_pointer(), SOLEN_OFFSET),
    ]);
    platform.emit_libc_call(
        "getsockopt",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&net_fail_fd),
        abi::load_u32("x9", abi::stack_pointer(), SOERR_OFFSET),
        abi::compare_immediate("x9", "0"),
        abi::branch_ne(&net_fail_fd),
        // Connected: restore blocking mode with fcntl(fd, F_SETFL, flags).
        abi::label(&nb_connected),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::move_immediate("x1", "Integer", "4"),
        abi::load_u64("x2", abi::stack_pointer(), FLAGS_OFFSET),
    ]);
    platform.emit_variadic_call(
        "fcntl",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::branch(&connected),
        // Blocking connect path (timeoutMs <= 0).
        abi::label(&blocking_connect),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64("x9", abi::stack_pointer(), RES_OFFSET),
        abi::load_u64("x1", "x9", addr_off),
        abi::load_u32("x2", "x9", 16),
    ]);
    platform.emit_libc_call(
        "connect",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&net_fail_fd),
        abi::label(&connected),
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
    // Bound the blocking TLS handshake by timeoutMs (SO_RCVTIMEO/SO_SNDTIMEO),
    // cleared again after the handshake so read/write stay unbounded.
    instructions.extend([
        abi::load_u64("x1", abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::compare_immediate("x1", "0"),
        abi::branch_le(&hs_timeout_set),
        // tv_sec = ms / 1000, tv_usec = (ms % 1000) * 1000
        abi::move_immediate("x10", "Integer", "1000"),
        abi::unsigned_divide_registers("x11", "x1", "x10"),
        abi::multiply_subtract_registers("x12", "x11", "x10", "x1"),
        abi::move_immediate("x13", "Integer", "1000"),
        abi::multiply_registers("x12", "x12", "x13"),
        abi::store_u64("x11", abi::stack_pointer(), TIMEVAL_OFFSET),
        abi::store_u64("x12", abi::stack_pointer(), TIMEVAL_OFFSET + 8),
    ]);
    emit_set_sock_timeouts(
        symbol,
        FD_OFFSET,
        TIMEVAL_OFFSET,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.push(abi::label(&hs_timeout_set));
    // SNI/validation name = serverName if non-empty, else host.
    instructions.extend([
        abi::load_u64("x9", abi::stack_pointer(), SNAME_OFFSET),
        abi::load_u64("x10", "x9", 0),
        abi::compare_immediate("x10", "0"),
        abi::branch_ne(&use_sname),
    ]);
    emit_cstring(
        symbol,
        "snihost",
        HOST_OFFSET,
        SNICSTR_OFFSET,
        &alloc_fail,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::branch(&sni_ready));
    instructions.push(abi::label(&use_sname));
    emit_cstring(
        symbol,
        "sni",
        SNAME_OFFSET,
        SNICSTR_OFFSET,
        &alloc_fail,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::label(&sni_ready));

    // --- OpenSSL handshake (shared) ---
    emit_dlopen_libssl(
        symbol,
        HANDLE_OFFSET,
        &load_fail,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    // method = TLS_client_method(); stash transiently in the CTX slot.
    emit_dlsym(
        symbol,
        HANDLE_OFFSET,
        "TLS_client_method",
        FNPTR_OFFSET,
        &load_fail,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64("x9", abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register("x9"),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), CTX_OFFSET),
    ]);
    // ctx = SSL_CTX_new(method)
    emit_dlsym(
        symbol,
        HANDLE_OFFSET,
        "SSL_CTX_new",
        FNPTR_OFFSET,
        &load_fail,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CTX_OFFSET),
        abi::load_u64("x9", abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register("x9"),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&tls_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), CTX_OFFSET),
    ]);
    // SSL_CTX_set_default_verify_paths(ctx) -- best effort, ignore result.
    emit_dlsym(
        symbol,
        HANDLE_OFFSET,
        "SSL_CTX_set_default_verify_paths",
        FNPTR_OFFSET,
        &load_fail,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CTX_OFFSET),
        abi::load_u64("x9", abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register("x9"),
    ]);
    // ssl = SSL_new(ctx)
    emit_dlsym(
        symbol,
        HANDLE_OFFSET,
        "SSL_new",
        FNPTR_OFFSET,
        &load_fail,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CTX_OFFSET),
        abi::load_u64("x9", abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register("x9"),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&tls_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
    ]);
    // SSL_set_fd(ssl, fd)
    emit_dlsym(
        symbol,
        HANDLE_OFFSET,
        "SSL_set_fd",
        FNPTR_OFFSET,
        &load_fail,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64("x1", abi::stack_pointer(), FD_OFFSET),
        abi::load_u64("x9", abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register("x9"),
        abi::compare_immediate(abi::return_register(), "1"),
        abi::branch_ne(&tls_fail),
    ]);
    // SSL_set_verify(ssl, SSL_VERIFY_PEER, NULL)
    emit_dlsym(
        symbol,
        HANDLE_OFFSET,
        "SSL_set_verify",
        FNPTR_OFFSET,
        &load_fail,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::move_immediate("x1", "Integer", SSL_VERIFY_PEER),
        abi::move_immediate("x2", "Integer", "0"),
        abi::load_u64("x9", abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register("x9"),
    ]);
    // SSL_set1_host(ssl, sniCstr)
    emit_dlsym(
        symbol,
        HANDLE_OFFSET,
        "SSL_set1_host",
        FNPTR_OFFSET,
        &load_fail,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64("x1", abi::stack_pointer(), SNICSTR_OFFSET),
        abi::load_u64("x9", abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register("x9"),
        abi::compare_immediate(abi::return_register(), "1"),
        abi::branch_ne(&tls_fail),
    ]);
    // SSL_ctrl(ssl, SSL_CTRL_SET_TLSEXT_HOSTNAME, TLSEXT_NAMETYPE_host_name, sniCstr) -- SNI
    emit_dlsym(
        symbol,
        HANDLE_OFFSET,
        "SSL_ctrl",
        FNPTR_OFFSET,
        &load_fail,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::move_immediate("x1", "Integer", SSL_CTRL_SET_TLSEXT_HOSTNAME),
        abi::move_immediate("x2", "Integer", TLSEXT_NAMETYPE_HOST_NAME),
        abi::load_u64("x3", abi::stack_pointer(), SNICSTR_OFFSET),
        abi::load_u64("x9", abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register("x9"),
        // SSL_ctrl(ssl, SSL_CTRL_SET_MIN_PROTO_VERSION, TLS1_2_VERSION, NULL)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::move_immediate("x1", "Integer", SSL_CTRL_SET_MIN_PROTO_VERSION),
        abi::move_immediate("x2", "Integer", TLS1_2_VERSION),
        abi::move_immediate("x3", "Integer", "0"),
        abi::load_u64("x9", abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register("x9"),
    ]);
    // r = SSL_connect(ssl); require 1.
    emit_dlsym(
        symbol,
        HANDLE_OFFSET,
        "SSL_connect",
        FNPTR_OFFSET,
        &load_fail,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64("x9", abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register("x9"),
        abi::compare_immediate(abi::return_register(), "1"),
        abi::branch_ne(&tls_fail),
    ]);
    // v = SSL_get_verify_result(ssl); require X509_V_OK (0).
    emit_dlsym(
        symbol,
        HANDLE_OFFSET,
        "SSL_get_verify_result",
        FNPTR_OFFSET,
        &load_fail,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64("x9", abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register("x9"),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&tls_fail),
    ]);
    // Handshake done: clear SO_*TIMEO (zero timeval) so read/write are unbounded.
    instructions.extend([
        abi::load_u64("x9", abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::compare_immediate("x9", "0"),
        abi::branch_le(&hs_timeout_clear),
        abi::store_u64("x31", abi::stack_pointer(), TIMEVAL_OFFSET),
        abi::store_u64("x31", abi::stack_pointer(), TIMEVAL_OFFSET + 8),
    ]);
    emit_set_sock_timeouts(
        symbol,
        FD_OFFSET,
        TIMEVAL_OFFSET,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.push(abi::label(&hs_timeout_clear));
    // Build the TlsSocket record { fd, closed = 0, ssl, ctx }.
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", TLS_RECORD_SIZE),
        abi::move_immediate("x1", "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::load_u64("x9", abi::stack_pointer(), FD_OFFSET),
        abi::store_u64("x9", "x1", TLS_OFFSET_FD),
        abi::store_u64("x31", "x1", TLS_OFFSET_CLOSED),
        abi::load_u64("x9", abi::stack_pointer(), SSL_OFFSET),
        abi::store_u64("x9", "x1", TLS_OFFSET_SSL),
        abi::load_u64("x9", abi::stack_pointer(), CTX_OFFSET),
        abi::store_u64("x9", "x1", TLS_OFFSET_CTX),
        abi::move_register(RESULT_VALUE_REGISTER, "x1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);

    // Error paths.
    instructions.push(abi::label(&tls_fail));
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
    emit_fail(
        symbol,
        ERR_TLS_FAILED_CODE,
        ERR_TLS_FAILED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );

    instructions.push(abi::label(&load_fail));
    emit_fail(
        symbol,
        ERR_TLS_FAILED_CODE,
        ERR_TLS_FAILED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );

    instructions.push(abi::label(&net_fail_fd));
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
    instructions.push(abi::label(&net_fail));
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
    // The TCP connect did not complete before timeoutMs: close the pending
    // socket, release the resolver results, and report a timeout.
    instructions.push(abi::label(&connect_timeout));
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
        ERR_TIMEOUT_CODE,
        ERR_TIMEOUT_SYMBOL,
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
    instructions.push(abi::label(&alloc_fail));
    emit_fail(
        symbol,
        ERR_OUT_OF_MEMORY_CODE,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );

    instructions.extend([
        abi::label(&done),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);
    Ok((frame(FRAME_SIZE), instructions, relocations, Vec::new()))
}

// ---------------------------------------------------------------------------
// tls.read / tls.readText
// ---------------------------------------------------------------------------

pub(crate) fn lower_tls_read_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    text: bool,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>, Vec<CodeStackSlot>), String> {
    if platform.target().contains("macos") {
        return macos::lower_tls_read_macos(symbol, platform_imports, platform, text);
    }
    const FRAME_SIZE: usize = 96;
    const LR_OFFSET: usize = 0;
    const SSL_OFFSET: usize = 8;
    const MAX_OFFSET: usize = 16;
    const BUF_OFFSET: usize = 24;
    const N_OFFSET: usize = 32;
    const HANDLE_OFFSET: usize = 40;
    const FNPTR_OFFSET: usize = 48;
    const STR_OFFSET: usize = 56;

    let closed = format!("{symbol}_closed");
    let invalid = format!("{symbol}_invalid");
    let peer_closed = format!("{symbol}_peer_closed");
    let read_fail = format!("{symbol}_read_fail");
    let load_fail = format!("{symbol}_load_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let encoding_error = format!("{symbol}_encoding_error");
    let str_copy = format!("{symbol}_str_copy");
    let str_done = format!("{symbol}_str_done");
    let entry_loop = format!("{symbol}_entry_loop");
    let entry_done = format!("{symbol}_entry_done");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::store_u64("x1", abi::stack_pointer(), MAX_OFFSET),
        abi::load_u64("x9", abi::return_register(), TLS_OFFSET_CLOSED),
        abi::compare_immediate("x9", "0"),
        abi::branch_ne(&closed),
        abi::load_u64("x9", abi::return_register(), TLS_OFFSET_SSL),
        abi::store_u64("x9", abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), MAX_OFFSET),
        abi::compare_immediate("x10", "0"),
        abi::branch_le(&invalid),
        // Allocate a maxBytes read buffer.
        abi::move_register(abi::return_register(), "x10"),
        abi::move_immediate("x1", "Integer", "1"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.push(abi::store_u64("x1", abi::stack_pointer(), BUF_OFFSET));
    emit_dlopen_libssl(
        symbol,
        HANDLE_OFFSET,
        &load_fail,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    emit_dlsym(
        symbol,
        HANDLE_OFFSET,
        "SSL_read",
        FNPTR_OFFSET,
        &load_fail,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    // n = SSL_read(ssl, buf, maxBytes)
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64("x1", abi::stack_pointer(), BUF_OFFSET),
        abi::load_u64("x2", abi::stack_pointer(), MAX_OFFSET),
        abi::load_u64("x9", abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register("x9"),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&peer_closed),
        abi::branch_lt(&read_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), N_OFFSET),
    ]);
    if text {
        instructions.extend([
            abi::load_u64("x10", abi::stack_pointer(), N_OFFSET),
            abi::add_immediate(abi::return_register(), "x10", 9),
            abi::move_immediate("x1", "Integer", "8"),
        ]);
        emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
        instructions.extend([
            abi::load_u64("x10", abi::stack_pointer(), N_OFFSET),
            abi::store_u64("x10", "x1", 0),
            abi::load_u64("x11", abi::stack_pointer(), BUF_OFFSET),
            abi::add_immediate("x12", "x1", 8),
            abi::move_immediate("x13", "Integer", "0"),
            abi::store_u64("x1", abi::stack_pointer(), STR_OFFSET),
            abi::label(&str_copy),
            abi::compare_registers("x13", "x10"),
            abi::branch_eq(&str_done),
            abi::load_u8("x14", "x11", 0),
            abi::store_u8("x14", "x12", 0),
            abi::add_immediate("x11", "x11", 1),
            abi::add_immediate("x12", "x12", 1),
            abi::add_immediate("x13", "x13", 1),
            abi::branch(&str_copy),
            abi::label(&str_done),
            abi::store_u8("x31", "x12", 0),
            abi::load_u64("x9", abi::stack_pointer(), STR_OFFSET),
            abi::add_immediate(abi::return_register(), "x9", 8),
            abi::load_u64("x1", "x9", 0),
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
        instructions.extend([
            abi::load_u64("x10", abi::stack_pointer(), N_OFFSET),
            abi::move_immediate("x11", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
            abi::multiply_registers("x12", "x10", "x11"),
            abi::add_immediate("x12", "x12", COLLECTION_HEADER_SIZE),
            abi::add_registers(abi::return_register(), "x12", "x10"),
            abi::move_immediate("x1", "Integer", "8"),
        ]);
        emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
        instructions.extend([
            abi::move_immediate("x9", "Byte", &COLLECTION_KIND_LIST.to_string()),
            abi::store_u8("x9", "x1", COLLECTION_OFFSET_KIND),
            abi::move_immediate("x9", "Byte", &COLLECTION_TYPE_NONE.to_string()),
            abi::store_u8("x9", "x1", COLLECTION_OFFSET_KEY_TYPE),
            abi::move_immediate("x9", "Byte", &COLLECTION_TYPE_BYTE.to_string()),
            abi::store_u8("x9", "x1", COLLECTION_OFFSET_VALUE_TYPE),
            abi::move_immediate("x9", "Byte", "1"),
            abi::store_u8("x9", "x1", COLLECTION_OFFSET_FLAGS_VERSION),
            abi::load_u64("x10", abi::stack_pointer(), N_OFFSET),
            abi::store_u64("x10", "x1", COLLECTION_OFFSET_COUNT),
            abi::store_u64("x10", "x1", COLLECTION_OFFSET_CAPACITY),
            abi::store_u64("x10", "x1", COLLECTION_OFFSET_DATA_LENGTH),
            abi::store_u64("x10", "x1", COLLECTION_OFFSET_DATA_CAPACITY),
            abi::add_immediate("x11", "x1", COLLECTION_HEADER_SIZE),
            abi::move_immediate("x12", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
            abi::multiply_registers("x13", "x10", "x12"),
            abi::add_registers("x14", "x11", "x13"),
            abi::load_u64("x15", abi::stack_pointer(), BUF_OFFSET),
            abi::move_immediate("x9", "Integer", "0"),
            abi::label(&entry_loop),
            abi::compare_registers("x9", "x10"),
            abi::branch_eq(&entry_done),
            abi::move_immediate("x12", "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
            abi::store_u8("x12", "x11", COLLECTION_ENTRY_OFFSET_FLAGS),
            abi::store_u64("x31", "x11", COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
            abi::store_u64("x31", "x11", COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
            abi::store_u64("x9", "x11", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
            abi::move_immediate("x12", "Integer", "1"),
            abi::store_u64("x12", "x11", COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
            abi::add_registers("x12", "x14", "x9"),
            abi::load_u8("x13", "x15", 0),
            abi::store_u8("x13", "x12", 0),
            abi::add_immediate("x15", "x15", 1),
            abi::add_immediate("x11", "x11", COLLECTION_ENTRY_SIZE),
            abi::add_immediate("x9", "x9", 1),
            abi::branch(&entry_loop),
            abi::label(&entry_done),
            abi::move_register(RESULT_VALUE_REGISTER, "x1"),
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
    instructions.push(abi::label(&read_fail));
    emit_fail(
        symbol,
        ERR_TLS_FAILED_CODE,
        ERR_TLS_FAILED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&load_fail));
    emit_fail(
        symbol,
        ERR_TLS_FAILED_CODE,
        ERR_TLS_FAILED_SYMBOL,
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
    instructions.extend([
        abi::label(&done),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);
    Ok((frame(FRAME_SIZE), instructions, relocations, Vec::new()))
}

// ---------------------------------------------------------------------------
// tls.write / tls.writeText
// ---------------------------------------------------------------------------

pub(crate) fn lower_tls_write_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    text: bool,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>, Vec<CodeStackSlot>), String> {
    if platform.target().contains("macos") {
        return macos::lower_tls_write_macos(symbol, platform_imports, platform, text);
    }
    const FRAME_SIZE: usize = 80;
    const LR_OFFSET: usize = 0;
    const SSL_OFFSET: usize = 8;
    const SRC_OFFSET: usize = 16;
    const REMAINING_OFFSET: usize = 24;
    const HANDLE_OFFSET: usize = 32;
    const FNPTR_OFFSET: usize = 40;

    let closed = format!("{symbol}_closed");
    let load_fail = format!("{symbol}_load_fail");
    let write_loop = format!("{symbol}_write_loop");
    let write_done = format!("{symbol}_write_done");
    let write_fail = format!("{symbol}_write_fail");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::load_u64("x9", abi::return_register(), TLS_OFFSET_CLOSED),
        abi::compare_immediate("x9", "0"),
        abi::branch_ne(&closed),
        abi::load_u64("x9", abi::return_register(), TLS_OFFSET_SSL),
        abi::store_u64("x9", abi::stack_pointer(), SSL_OFFSET),
    ]);
    if text {
        instructions.extend([
            abi::load_u64("x10", "x1", 0),
            abi::store_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
            abi::add_immediate("x11", "x1", 8),
            abi::store_u64("x11", abi::stack_pointer(), SRC_OFFSET),
        ]);
    } else {
        instructions.extend([
            abi::load_u64("x10", "x1", COLLECTION_OFFSET_COUNT),
            abi::store_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
            abi::move_immediate("x12", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
            abi::multiply_registers("x13", "x10", "x12"),
            abi::add_immediate("x13", "x13", COLLECTION_HEADER_SIZE),
            abi::add_registers("x11", "x1", "x13"),
            abi::store_u64("x11", abi::stack_pointer(), SRC_OFFSET),
        ]);
    }
    emit_dlopen_libssl(
        symbol,
        HANDLE_OFFSET,
        &load_fail,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    emit_dlsym(
        symbol,
        HANDLE_OFFSET,
        "SSL_write",
        FNPTR_OFFSET,
        &load_fail,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::label(&write_loop),
        abi::load_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::compare_immediate("x10", "0"),
        abi::branch_eq(&write_done),
        // n = SSL_write(ssl, src, remaining)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64("x1", abi::stack_pointer(), SRC_OFFSET),
        abi::move_register("x2", "x10"),
        abi::load_u64("x9", abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register("x9"),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(&write_fail),
        abi::load_u64("x11", abi::stack_pointer(), SRC_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::add_registers("x11", "x11", abi::return_register()),
        abi::subtract_registers("x10", "x10", abi::return_register()),
        abi::store_u64("x11", abi::stack_pointer(), SRC_OFFSET),
        abi::store_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::branch(&write_loop),
        abi::label(&write_done),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    instructions.push(abi::label(&write_fail));
    emit_fail(
        symbol,
        ERR_TLS_FAILED_CODE,
        ERR_TLS_FAILED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&load_fail));
    emit_fail(
        symbol,
        ERR_TLS_FAILED_CODE,
        ERR_TLS_FAILED_SYMBOL,
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
    instructions.extend([
        abi::label(&done),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);
    Ok((frame(FRAME_SIZE), instructions, relocations, Vec::new()))
}

// ---------------------------------------------------------------------------
// tls.close
// ---------------------------------------------------------------------------

pub(crate) fn lower_tls_close_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>, Vec<CodeStackSlot>), String> {
    if platform.target().contains("macos") {
        return macos::lower_tls_close_macos(symbol, platform_imports, platform);
    }
    const FRAME_SIZE: usize = 80;
    const LR_OFFSET: usize = 0;
    const REC_OFFSET: usize = 8;
    const SSL_OFFSET: usize = 16;
    const CTX_OFFSET: usize = 24;
    const FD_OFFSET: usize = 32;
    const HANDLE_OFFSET: usize = 40;
    const FNPTR_OFFSET: usize = 48;

    let already = format!("{symbol}_already");
    let load_fail = format!("{symbol}_load_fail");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), REC_OFFSET),
        // Idempotent: a closed handle returns OK.
        abi::load_u64("x9", abi::return_register(), TLS_OFFSET_CLOSED),
        abi::compare_immediate("x9", "0"),
        abi::branch_ne(&already),
        abi::load_u64("x9", abi::return_register(), TLS_OFFSET_SSL),
        abi::store_u64("x9", abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64("x9", abi::return_register(), TLS_OFFSET_CTX),
        abi::store_u64("x9", abi::stack_pointer(), CTX_OFFSET),
        abi::load_u64("x9", abi::return_register(), TLS_OFFSET_FD),
        abi::store_u64("x9", abi::stack_pointer(), FD_OFFSET),
    ]);
    emit_dlopen_libssl(
        symbol,
        HANDLE_OFFSET,
        &load_fail,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    // SSL_shutdown(ssl)
    emit_dlsym(
        symbol,
        HANDLE_OFFSET,
        "SSL_shutdown",
        FNPTR_OFFSET,
        &load_fail,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64("x9", abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register("x9"),
    ]);
    // SSL_free(ssl)
    emit_dlsym(
        symbol,
        HANDLE_OFFSET,
        "SSL_free",
        FNPTR_OFFSET,
        &load_fail,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64("x9", abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register("x9"),
    ]);
    // SSL_CTX_free(ctx)
    emit_dlsym(
        symbol,
        HANDLE_OFFSET,
        "SSL_CTX_free",
        FNPTR_OFFSET,
        &load_fail,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CTX_OFFSET),
        abi::load_u64("x9", abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register("x9"),
    ]);
    // close(fd)
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
    // Mark the record closed.
    instructions.extend([
        abi::load_u64("x9", abi::stack_pointer(), REC_OFFSET),
        abi::move_immediate("x10", "Integer", "1"),
        abi::store_u64("x10", "x9", TLS_OFFSET_CLOSED),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    // A failure to resolve OpenSSL during close still closes the fd and reports
    // success-ish OK (the session is gone); but to surface load problems we map
    // it to ErrTlsFailed.
    instructions.push(abi::label(&load_fail));
    emit_fail(
        symbol,
        ERR_TLS_FAILED_CODE,
        ERR_TLS_FAILED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([
        abi::label(&already),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::label(&done),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);
    Ok((frame(FRAME_SIZE), instructions, relocations, Vec::new()))
}
