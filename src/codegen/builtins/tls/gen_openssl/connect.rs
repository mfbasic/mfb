//! plan-176-T: the OpenSSL `tls::connect` lowering (`lower_tls_connect_openssl`). The shared
//! imports live in `builtins/tls/gen_openssl/mod.rs`.

use super::*;

pub(crate) fn lower_tls_connect_openssl(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    address: bool,
) -> Result<TlsBodyParts, String> {
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    let v14 = vregs.next();
    let v15 = vregs.next();
    let v12 = vregs.next();
    let v13 = vregs.next();
    const FRAME_SIZE: usize = 256;
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
    const HSTOFLAG: usize = 200; // plan-73-D: 1 if the handshake recv timed out (SO_*TIMEO)
    const ALLOW: usize = 208; // bug-477: allowSelfSigned (0/1)
    const VCB: usize = 216; // bug-477: SSL_set_verify's callback argument (0 or &cb)

    let resolve_fail = format!("{symbol}_resolve_fail");
    let net_fail = format!("{symbol}_net_fail");
    let net_fail_fd = format!("{symbol}_net_fail_fd");
    let connect_timeout = format!("{symbol}_connect_timeout");
    let connect_invalid = format!("{symbol}_connect_invalid");
    let port_invalid = format!("{symbol}_port_invalid");
    let blocking_connect = format!("{symbol}_blocking_connect");
    let nb_connected = format!("{symbol}_nb_connected");
    let connected = format!("{symbol}_connected");
    let hs_timeout_set = format!("{symbol}_hs_timeout_set");
    let hs_timeout_clear = format!("{symbol}_hs_timeout_clear");
    let tls_fail = format!("{symbol}_tls_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let alloc_fail_raw = format!("{symbol}_alloc_fail_raw");
    let load_fail = format!("{symbol}_load_fail");
    let use_sname = format!("{symbol}_use_sname");
    let sni_ready = format!("{symbol}_sni_ready");
    let done = format!("{symbol}_done");

    let addr_off = platform.addrinfo_addr_offset();
    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();

    // bug-575: the two C-string copies this body marshals — the host for
    // `getaddrinfo`, and the SNI/validation name for `SSL_set1_host` — are the
    // helper's own scratch: handed to OpenSSL and never returned to MFBASIC, so
    // nothing on the caller side could see them and every `tls::connect` leaked its
    // host name twice. Declared HERE, ahead of every branch that can reach `done`,
    // so the release at `done` reads a null on the paths that never marshalled (the
    // `connect_invalid` timeout rejection, the `alloc_fail` tail itself).
    let host_scratch = HelperScratch::declare(&mut vregs, &mut instructions);
    // `snihost` and `sni` are the two exclusive arms of one choice writing one
    // slot, so they share one scratch.
    let sni_scratch = HelperScratch::declare(&mut vregs, &mut instructions);

    // Host form: x0 = host; x1 = port; x2 = timeoutMs; x3 = serverName; x4 = allowSelfSigned.
    // Address form: x0 = net::Address; x1 = timeoutMs; x2 = serverName; x3 = allowSelfSigned.
    instructions.extend(super::gen_shared::connect_arg_prologue(
        address,
        &v9,
        HOST_OFFSET,
        PORT_OFFSET,
        TIMEOUT_OFFSET,
        SNAME_OFFSET,
        ALLOW,
    ));
    instructions.extend([
        // Sentinel-initialise the fd (-1) and the SSL/SSL_CTX slots (0) so the
        // alloc_fail exit can close/free exactly what has been acquired without
        // touching a garbage fd or object (bug-55).
        abi::move_immediate(&v9, "Integer", "0"),
        abi::bitwise_not(&v9, &v9),
        abi::store_u64(&v9, abi::stack_pointer(), FD_OFFSET),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), SSL_OFFSET),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), CTX_OFFSET),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), HSTOFLAG),
    ]);
    // bug-876: a port outside 0..=65535 is ErrAddressInvalid, refused before
    // getaddrinfo/socket exactly as tcp::connect does.
    crate::codegen::os::socket::shared::emit_port_range_check(
        PORT_OFFSET,
        &port_invalid,
        &mut instructions,
        &mut vregs,
    );
    {
        // plan-73-D: reject a negative (non-sentinel) `timeoutMs` up front — before
        // getaddrinfo/socket, so nothing leaks. The omitted overload pads the
        // unbounded sentinel (i64::MIN, allowed → the blocking connect + blocking
        // handshake below); `0`/`> 0` pass through.
        let ts_ok = format!("{symbol}_ts_ok");
        let ts_store = format!("{symbol}_ts_clamped");
        instructions.extend([
            abi::load_u64(&v9, abi::stack_pointer(), TIMEOUT_OFFSET),
            abi::move_immediate(&v10, "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
            abi::compare_registers(&v9, &v10),
            abi::branch_eq(&ts_ok),
            abi::compare_immediate(&v9, "0"),
            abi::branch_lt(&connect_invalid),
            // Clamp `> 0` to INT_MAX and store it back: poll() and the handshake
            // SO_*TIMEO take a bounded C `int`, so a value with bit 31 set would be
            // read as a negative (block-forever) timeout (bug-239). net clamps the
            // same way (net/poll.rs); tls was missing it. The sentinel skips this
            // (branch_eq above) and stays in the slot as the block form.
            abi::move_immediate(&v10, "Integer", "2147483647"),
            abi::compare_registers(&v9, &v10),
            abi::branch_le(&ts_store),
            abi::move_register(&v9, &v10),
            abi::label(&ts_store),
            abi::store_u64(&v9, abi::stack_pointer(), TIMEOUT_OFFSET),
            abi::label(&ts_ok),
        ]);
    }
    // Resolve + connect a TCP socket. Zero a 48-byte hints block and set
    // ai_family = AF_INET, ai_socktype = SOCK_STREAM.
    for offset in (0..48).step_by(8) {
        instructions.push(abi::store_u64(
            abi::ZERO,
            abi::stack_pointer(),
            HINTS_OFFSET + offset,
        ));
    }
    instructions.extend([
        abi::move_immediate(&v9, "Integer", HINTS_FAMILY_WORD),
        abi::store_u64(&v9, abi::stack_pointer(), HINTS_OFFSET),
        abi::move_immediate(&v9, "Integer", SOCK_STREAM),
        abi::store_u64(&v9, abi::stack_pointer(), HINTS_OFFSET + 8),
    ]);
    emit_cstring(
        symbol,
        "host",
        HOST_OFFSET,
        HOSTCSTR_OFFSET,
        &alloc_fail,
        &host_scratch,
        &mut instructions,
        &mut relocations,
        &mut vregs,
    );
    instructions.extend([
        abi::load_u64(
            abi::return_register(),
            abi::stack_pointer(),
            HOSTCSTR_OFFSET,
        ),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        abi::add_immediate(abi::c_arg(2), abi::stack_pointer(), HINTS_OFFSET),
        abi::add_immediate(abi::c_arg(3), abi::stack_pointer(), RES_OFFSET),
    ]);
    platform.emit_external_call(
        "getaddrinfo",
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
        "socket",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        // bug-102.3: narrow the C int `socket` return before the signed compare.
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&net_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        // Overwrite sin_port (ai_addr + 2/3) with the requested port.
        abi::load_u64(&v9, abi::stack_pointer(), RES_OFFSET),
        abi::load_u64(&v9, &v9, addr_off),
        abi::load_u64(&v10, abi::stack_pointer(), PORT_OFFSET),
        abi::shift_right_immediate(&v11, &v10, 8),
        abi::store_u8(&v11, &v9, 2),
        abi::store_u8(&v10, &v9, 3),
    ]);
    // plan-73-D. Connect the socket: the unbounded sentinel => a plain blocking
    // connect (omit = block); `0` => a non-blocking connect + poll(0) (one immediate
    // attempt → `ErrTimeout` unless it completes at once); `> 0` => non-blocking
    // connect + poll(timeoutMs). Negatives were rejected up front. Mirrors
    // net::connectTcp. DNS (getaddrinfo above) is not bounded by timeoutMs.
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::move_immediate(&v10, "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
        abi::compare_registers(&v9, &v10),
        abi::branch_eq(&blocking_connect),
        // flags = fcntl(fd, F_GETFL, 0)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::move_immediate(abi::c_arg(1), "Integer", "3"),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
    ]);
    platform.emit_variadic_external_call(
        "fcntl",
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
        abi::load_u64(abi::c_arg(1), &v9, addr_off),
        abi::load_u32(abi::c_arg(2), &v9, 16),
    ]);
    platform.emit_external_call(
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
        (&v9).into(),
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(&v9, platform.socket_in_progress_code()),
        abi::branch_ne(&net_fail_fd),
        // poll(&pollfd { fd, POLLOUT }, 1, timeoutMs)
        abi::load_u64(&v9, abi::stack_pointer(), FD_OFFSET),
        abi::store_u64(&v9, abi::stack_pointer(), POLLFD_OFFSET),
        abi::move_immediate(&v10, "Integer", "4"), // POLLOUT
        abi::store_u8(&v10, abi::stack_pointer(), POLLFD_OFFSET + 4),
        abi::store_u8(abi::ZERO, abi::stack_pointer(), POLLFD_OFFSET + 5),
        abi::store_u8(abi::ZERO, abi::stack_pointer(), POLLFD_OFFSET + 6),
        abi::store_u8(abi::ZERO, abi::stack_pointer(), POLLFD_OFFSET + 7),
        abi::add_immediate(abi::return_register(), abi::stack_pointer(), POLLFD_OFFSET),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), TIMEOUT_OFFSET),
    ]);
    platform.emit_external_call(
        "poll",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        // bug-102.3: narrow the C int `poll` return before the signed compare.
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&net_fail_fd),
        abi::branch_eq(&connect_timeout),
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
    platform.emit_external_call(
        "getsockopt",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        // bug-102.3: narrow the C int `getsockopt` return before the signed compare.
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&net_fail_fd),
        abi::load_u32(&v9, abi::stack_pointer(), SOERR_OFFSET),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&net_fail_fd),
        // Connected: restore blocking mode with fcntl(fd, F_SETFL, flags).
        abi::label(&nb_connected),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::move_immediate(abi::c_arg(1), "Integer", "4"),
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), FLAGS_OFFSET),
    ]);
    platform.emit_variadic_external_call(
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
        abi::load_u64(&v9, abi::stack_pointer(), RES_OFFSET),
        abi::load_u64(abi::c_arg(1), &v9, addr_off),
        abi::load_u32(abi::c_arg(2), &v9, 16),
    ]);
    platform.emit_external_call(
        "connect",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        // bug-102.3: narrow the C int `connect` return before the signed compare.
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&net_fail_fd),
        abi::label(&connected),
        // freeaddrinfo(res)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), RES_OFFSET),
    ]);
    platform.emit_external_call(
        "freeaddrinfo",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    // plan-73-D. Bound the blocking TLS handshake by timeoutMs (SO_RCVTIMEO/
    // SO_SNDTIMEO), cleared again after the handshake so read/write stay unbounded.
    // The unbounded sentinel => leave the handshake unbounded (omit = block); `0` =>
    // the smallest nonzero wait (tv_usec = 1µs, near-immediate `ErrTimeout` — a
    // SO_*TIMEO of {0,0} is *infinite*, so 0 cannot be literal); `> 0` => the timeval.
    let hs_ts_ok = format!("{symbol}_hs_ts_ok");
    instructions.extend([
        abi::load_u64(&v14, abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::move_immediate(&v15, "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
        abi::compare_registers(&v14, &v15),
        abi::branch_eq(&hs_timeout_set),
        // tv_sec = ms / 1000, tv_usec = (ms % 1000) * 1000
        abi::move_immediate(&v10, "Integer", "1000"),
        abi::unsigned_divide_registers(&v11, &v14, &v10),
        abi::multiply_subtract_registers(&v12, &v11, &v10, &v14),
        abi::move_immediate(&v13, "Integer", "1000"),
        abi::multiply_registers(&v12, &v12, &v13),
        // 0 => bump tv_usec to 1µs so the handshake is non-blocking, not infinite.
        abi::compare_immediate(&v14, "0"),
        abi::branch_ne(&hs_ts_ok),
        abi::move_immediate(&v12, "Integer", "1"),
        abi::label(&hs_ts_ok),
        abi::store_u64(&v11, abi::stack_pointer(), TIMEVAL_OFFSET),
        abi::store_u64(&v12, abi::stack_pointer(), TIMEVAL_OFFSET + 8),
    ]);
    emit_set_sock_timeouts(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        FD_OFFSET,
        TIMEVAL_OFFSET,
    )?;
    instructions.push(abi::label(&hs_timeout_set));
    // SNI/validation name = serverName if non-empty, else host.
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), SNAME_OFFSET),
        abi::load_u64(&v10, &v9, 0),
        abi::compare_immediate(&v10, "0"),
        abi::branch_ne(&use_sname),
    ]);
    emit_cstring(
        symbol,
        "snihost",
        HOST_OFFSET,
        SNICSTR_OFFSET,
        &alloc_fail,
        &sni_scratch,
        &mut instructions,
        &mut relocations,
        &mut vregs,
    );
    instructions.push(abi::branch(&sni_ready));
    instructions.push(abi::label(&use_sname));
    emit_cstring(
        symbol,
        "sni",
        SNAME_OFFSET,
        SNICSTR_OFFSET,
        &alloc_fail,
        &sni_scratch,
        &mut instructions,
        &mut relocations,
        &mut vregs,
    );
    instructions.push(abi::label(&sni_ready));

    // --- OpenSSL handshake (shared) ---
    emit_dlopen_libssl(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        &load_fail,
    )?;
    // method = TLS_client_method(); stash transiently in the CTX slot.
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "TLS_client_method",
        FNPTR_OFFSET,
        &load_fail,
    )?;
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        abi::store_u64(abi::c_return(0), abi::stack_pointer(), CTX_OFFSET),
    ]);
    // ctx = SSL_CTX_new(method)
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_CTX_new",
        FNPTR_OFFSET,
        &load_fail,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CTX_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_eq(&tls_fail),
        abi::store_u64(abi::c_return(0), abi::stack_pointer(), CTX_OFFSET),
    ]);
    // SSL_CTX_set_default_verify_paths(ctx) -- best effort, ignore result.
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_CTX_set_default_verify_paths",
        FNPTR_OFFSET,
        &load_fail,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CTX_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
    ]);
    // ssl = SSL_new(ctx)
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_new",
        FNPTR_OFFSET,
        &load_fail,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CTX_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_eq(&tls_fail),
        abi::store_u64(abi::c_return(0), abi::stack_pointer(), SSL_OFFSET),
    ]);
    // SSL_set_fd(ssl, fd)
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_set_fd",
        FNPTR_OFFSET,
        &load_fail,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        abi::compare_immediate(abi::c_return(0), "1"),
        abi::branch_ne(&tls_fail),
    ]);
    // bug-477: the mode stays SSL_VERIFY_PEER in BOTH forms — only the callback
    // argument changes. NULL is today's behaviour (abort on the first chain
    // error); with `allowSelfSigned` set it is `_mfb_tls_verify_cb`, which
    // forgives the three trust-anchor codes and returns 1 so verification
    // CONTINUES into the hostname and validity-date checks. Dropping to
    // SSL_VERIFY_NONE instead — the shape the bug document originally proposed —
    // was measured to make a name-mismatched self-signed certificate
    // indistinguishable from a name-correct one (both report 18), which is why
    // this is a callback and not a mode change.
    // The callback argument is staged into a FRAME SLOT, not into `c_arg(2)`,
    // and only loaded into the register after the last `dlsym` below. Every
    // `emit_dlsym` is a C call and a C call clobbers the argument registers, so
    // writing `c_arg(2)` here and calling `dlsym` afterwards destroys it —
    // measured on box 2223, where the handshake never started and the peer
    // logged "0 server accepts". See `.ai/arch-abi.md`, "Stage ABI args via
    // temporaries".
    instructions.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), VCB));
    {
        let strict = format!("{symbol}_verify_strict");
        instructions.extend([
            abi::load_u64(&v10, abi::stack_pointer(), ALLOW),
            abi::compare_immediate(&v10, "0"),
            abi::branch_eq(&strict),
        ]);
        // Publish the two X509_STORE_CTX accessors the callback needs. It receives
        // no user pointer, so they travel through a process-global slot pair
        // rather than a capture; concurrent connects all store the same values.
        for (name, slot) in [
            ("X509_STORE_CTX_get_error", VFN_GET_ERROR),
            ("X509_STORE_CTX_set_error", VFN_SET_ERROR),
        ] {
            emit_dlsym(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                HANDLE_OFFSET,
                name,
                FNPTR_OFFSET,
                &load_fail,
            )?;
            emit_data_address(
                symbol,
                &v10,
                TLS_VERIFY_FNS,
                &mut instructions,
                &mut relocations,
            );
            instructions.extend([
                abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
                abi::store_u64(&v9, &v10, slot),
            ]);
        }
        emit_data_address(
            symbol,
            &v10,
            TLS_VERIFY_CB,
            &mut instructions,
            &mut relocations,
        );
        instructions.push(abi::store_u64(&v10, abi::stack_pointer(), VCB));
        instructions.push(abi::label(&strict));
    }
    // SSL_set_verify's own fnptr is re-resolved here because the loop above
    // reused FNPTR_OFFSET for the two accessors.
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_set_verify",
        FNPTR_OFFSET,
        &load_fail,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::move_immediate(abi::c_arg(1), "Integer", SSL_VERIFY_PEER),
        // Loaded HERE, after the last dlsym, for the reason given at VCB.
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), VCB),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
    ]);
    // SSL_set1_host(ssl, sniCstr) — verifies the peer certificate against DNS-name
    // SANs/CN only. TLS to an IP literal is therefore UNSUPPORTED: matching an
    // `iPAddress` SAN needs X509_VERIFY_PARAM_set1_ip (a separate libssl symbol)
    // driven by a runtime numeric-host check, so an IP-literal connection fails
    // verification and closes rather than validating by IP. This fails *closed*
    // (over-strict), never open — there is no verification bypass (bug-177 C).
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_set1_host",
        FNPTR_OFFSET,
        &load_fail,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), SNICSTR_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        abi::compare_immediate(abi::c_return(0), "1"),
        abi::branch_ne(&tls_fail),
    ]);
    // SSL_ctrl(ssl, SSL_CTRL_SET_TLSEXT_HOSTNAME, TLSEXT_NAMETYPE_host_name, sniCstr) -- SNI
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_ctrl",
        FNPTR_OFFSET,
        &load_fail,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::move_immediate(abi::c_arg(1), "Integer", SSL_CTRL_SET_TLSEXT_HOSTNAME),
        abi::move_immediate(abi::c_arg(2), "Integer", TLSEXT_NAMETYPE_HOST_NAME),
        abi::load_u64(abi::c_arg(3), abi::stack_pointer(), SNICSTR_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        // SSL_ctrl(ssl, SSL_CTRL_SET_MIN_PROTO_VERSION, TLS1_2_VERSION, NULL)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::move_immediate(abi::c_arg(1), "Integer", SSL_CTRL_SET_MIN_PROTO_VERSION),
        abi::move_immediate(abi::c_arg(2), "Integer", TLS1_2_VERSION),
        abi::move_immediate(abi::c_arg(3), "Integer", "0"),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        // Require the TLS 1.2 floor to have been set (returns 1 on success),
        // matching the checked SSL_set1_host / SSL_connect / verify-result calls
        // — an unchecked failure would silently permit a downgrade (bug-55).
        abi::compare_immediate(abi::c_return(0), "1"),
        abi::branch_ne(&tls_fail),
    ]);
    // r = SSL_connect(ssl); require 1.
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_connect",
        FNPTR_OFFSET,
        &load_fail,
    )?;
    let hs_connected = format!("{symbol}_hs_connected");
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        abi::compare_immediate(abi::c_return(0), "1"),
        abi::branch_eq(&hs_connected),
    ]);
    // plan-73-D: SSL_connect failed. If the handshake recv hit the SO_RCVTIMEO we
    // installed (errno == EWOULDBLOCK/EAGAIN), classify it as a TIMEOUT (ErrTimeout),
    // matching the macOS backend and the connect-poll timeout, rather than the
    // generic ErrTlsFailed. Any other failure stays ErrTlsFailed.
    platform.emit_errno(
        symbol,
        (&v9).into(),
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(&v9, platform.socket_would_block_code()),
        abi::branch_ne(&tls_fail),
        abi::move_immediate(&v9, "Integer", "1"),
        abi::store_u64(&v9, abi::stack_pointer(), HSTOFLAG),
        abi::branch(&tls_fail),
        abi::label(&hs_connected),
    ]);
    // v = SSL_get_verify_result(ssl); require X509_V_OK (0).
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_get_verify_result",
        FNPTR_OFFSET,
        &load_fail,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_ne(&tls_fail),
    ]);
    // Handshake done: clear SO_*TIMEO (zero timeval) so read/write are unbounded.
    // plan-73-D: only the sentinel (omit) left the handshake unbounded; `0`/`> 0`
    // installed a SO_*TIMEO that must be cleared here.
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::move_immediate(&v10, "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
        abi::compare_registers(&v9, &v10),
        abi::branch_eq(&hs_timeout_clear),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), TIMEVAL_OFFSET),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), TIMEVAL_OFFSET + 8),
    ]);
    emit_set_sock_timeouts(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        FD_OFFSET,
        TIMEVAL_OFFSET,
    )?;
    instructions.push(abi::label(&hs_timeout_clear));
    // Build the Socket record: canonical header { tag, fd, closed=0, STATE=0 }
    // then the TLS tail { ssl, ctx } (plan-80).
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", TLS_RECORD_SIZE),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::move_immediate(&v9, "Integer", RESOURCE_TAG_TLS_OPENSSL),
        abi::store_u64(&v9, abi::mfb_return(1), RESOURCE_OFFSET_TAG),
        // bug-755: a fresh record is unowned until a binding claims it.
        abi::store_u64(abi::ZERO, abi::mfb_return(1), RESOURCE_OFFSET_OWNER),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), TLS_OFFSET_STATE),
        abi::load_u64(&v9, abi::stack_pointer(), FD_OFFSET),
        abi::store_u64(&v9, abi::mfb_return(1), TLS_OFFSET_FD),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), TLS_OFFSET_CLOSED),
        // bug-564: no read has reported the peer's close yet.
        abi::store_u64(abi::ZERO, abi::mfb_return(1), TLS_OFFSET_PEER_CLOSED),
        abi::load_u64(&v9, abi::stack_pointer(), SSL_OFFSET),
        abi::store_u64(&v9, abi::mfb_return(1), TLS_OFFSET_SSL),
        abi::load_u64(&v9, abi::stack_pointer(), CTX_OFFSET),
        abi::store_u64(&v9, abi::mfb_return(1), TLS_OFFSET_CTX),
        abi::move_register(RESULT_VALUE_REGISTER, abi::mfb_return(1)),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);

    // Error paths.
    instructions.push(abi::label(&tls_fail));
    // Free the SSL session and per-connection SSL_CTX before closing the fd.
    // tls_fail is branched to from SSL_new onward — SSL_set_fd, SSL_set1_host,
    // the min-proto ctrl, SSL_connect and SSL_get_verify_result — at every one
    // of which this frame owns both objects. It used to close only the fd, so a
    // client reconnect loop against an expired- or untrusted-cert host leaked
    // one SSL + one SSL_CTX (several KB of OpenSSL heap) per failure, while the
    // sibling alloc_fail and the accept-side ssl_fail freed them (bug-317 T2).
    // Slots are sentinel-initialised to 0, so both frees are null-guarded and a
    // missing symbol falls through to tls_fail_raw, still reporting the TLS
    // failure rather than masking it as a load error.
    let tls_fail_raw = format!("{symbol}_tls_fail_raw");
    let tf_skip_ssl = format!("{symbol}_tf_skip_ssl");
    let tf_skip_ctx = format!("{symbol}_tf_skip_ctx");
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), SSL_OFFSET),
        abi::compare_immediate(&v9, "0"),
        abi::branch_eq(&tf_skip_ssl),
    ]);
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_free",
        FNPTR_OFFSET,
        &tls_fail_raw,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        abi::label(&tf_skip_ssl),
        abi::load_u64(&v9, abi::stack_pointer(), CTX_OFFSET),
        abi::compare_immediate(&v9, "0"),
        abi::branch_eq(&tf_skip_ctx),
    ]);
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_CTX_free",
        FNPTR_OFFSET,
        &tls_fail_raw,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CTX_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
    ]);
    instructions.push(abi::label(&tf_skip_ctx));
    instructions.push(abi::label(&tls_fail_raw));
    instructions.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        FD_OFFSET,
    ));
    platform.emit_external_call(
        "close",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    // plan-73-D: a handshake recv that hit the installed SO_*TIMEO is a timeout
    // (ErrTimeout), everything else on this path is a TLS failure (ErrTlsFailed).
    let tls_fail_timeout = format!("{symbol}_tls_fail_timeout");
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), HSTOFLAG),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&tls_fail_timeout),
    ]);
    emit_fail(
        symbol,
        "ErrTlsFailed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&tls_fail_timeout));
    emit_fail(
        symbol,
        "ErrTimeout",
        &mut instructions,
        &mut relocations,
        &done,
    );

    instructions.push(abi::label(&load_fail));
    // Every dlopen/dlsym on the libssl handshake path runs after the TCP socket is
    // already connected, so close the fd (guarded fd >= 0, mirroring alloc_fail's
    // close) before failing — otherwise a near-fatal OpenSSL-missing environment
    // leaks the connected socket (bug-177 B).
    let lf_skip_fd = format!("{symbol}_lf_skip_fd");
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), FD_OFFSET),
        abi::compare_immediate(&v9, "0"),
        abi::branch_lt(&lf_skip_fd),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
    ]);
    platform.emit_external_call(
        "close",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.push(abi::label(&lf_skip_fd));
    emit_fail(
        symbol,
        "ErrTlsFailed",
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
    platform.emit_external_call(
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
    platform.emit_external_call(
        "freeaddrinfo",
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
    // The TCP connect did not complete before timeoutMs: close the pending
    // socket, release the resolver results, and report a timeout.
    instructions.push(abi::label(&connect_timeout));
    instructions.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        FD_OFFSET,
    ));
    platform.emit_external_call(
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
    platform.emit_external_call(
        "freeaddrinfo",
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
    // plan-73-D: a negative (non-sentinel) `timeoutMs` → ErrInvalidArgument. Reached
    // from the up-front check before getaddrinfo/socket, so nothing to clean up.
    instructions.push(abi::label(&connect_invalid));
    emit_fail(
        symbol,
        "ErrInvalidArgument",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&port_invalid));
    emit_fail(
        symbol,
        "ErrAddressInvalid",
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
    instructions.push(abi::label(&alloc_fail));
    // Free the SSL session and context and close the socket the aborted record
    // would have owned. The tls_fail/net_fail_fd exits close the fd, but a
    // post-handshake record-alloc OOM otherwise leaked fd + SSL + SSL_CTX
    // (bug-55). Slots are sentinel-initialised (fd = -1, SSL/CTX = 0), so each
    // step is null/-1-guarded and the frees' dlsym only runs once the object
    // exists (libssl loaded). dlsym failures route to alloc_fail_raw.
    let af_skip_ssl = format!("{symbol}_af_skip_ssl");
    let af_skip_ctx = format!("{symbol}_af_skip_ctx");
    let af_skip_fd = format!("{symbol}_af_skip_fd");
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), SSL_OFFSET),
        abi::compare_immediate(&v9, "0"),
        abi::branch_eq(&af_skip_ssl),
    ]);
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_free",
        FNPTR_OFFSET,
        &alloc_fail_raw,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), SSL_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        abi::label(&af_skip_ssl),
        abi::load_u64(&v9, abi::stack_pointer(), CTX_OFFSET),
        abi::compare_immediate(&v9, "0"),
        abi::branch_eq(&af_skip_ctx),
    ]);
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_CTX_free",
        FNPTR_OFFSET,
        &alloc_fail_raw,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CTX_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        abi::label(&af_skip_ctx),
        abi::load_u64(&v9, abi::stack_pointer(), FD_OFFSET),
        abi::compare_immediate(&v9, "0"),
        abi::branch_lt(&af_skip_fd),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
    ]);
    platform.emit_external_call(
        "close",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.push(abi::label(&af_skip_fd));
    instructions.push(abi::label(&alloc_fail_raw));
    emit_fail(
        symbol,
        "ErrOutOfMemory",
        &mut instructions,
        &mut relocations,
        &done,
    );

    instructions.push(abi::label(&done));
    emit_helper_scratch_release(
        symbol,
        &[host_scratch, sni_scratch],
        &mut vregs,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::return_());
    {
        Ok((instructions, relocations, FRAME_SIZE))
    }
}
