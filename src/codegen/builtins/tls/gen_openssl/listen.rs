//! plan-176-T: the OpenSSL `tls::listen` lowering (`lower_tls_listen_openssl`). The shared
//! imports live in `builtins/tls/gen_openssl/mod.rs`.

use super::*;

// ---------------------------------------------------------------------------
// tls.listen
// ---------------------------------------------------------------------------

pub(crate) fn lower_tls_listen_openssl(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<TlsBodyParts, String> {
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    const FRAME_SIZE: usize = 224;
    const HOST_OFFSET: usize = 8;
    const PORT_OFFSET: usize = 16;
    const CERT_OFFSET: usize = 24;
    const KEY_OFFSET: usize = 32;
    const BACKLOG_OFFSET: usize = 40;
    const RES_OFFSET: usize = 48; // addrinfo*
    const FD_OFFSET: usize = 56;
    const HOSTCSTR_OFFSET: usize = 64;
    const ONE_OFFSET: usize = 72; // SO_REUSEADDR option value
    const CERTCSTR_OFFSET: usize = 80;
    const KEYCSTR_OFFSET: usize = 88;
    const HANDLE_OFFSET: usize = 96;
    const FNPTR_OFFSET: usize = 104;
    const CTX_OFFSET: usize = 112;
    const HINTS_OFFSET: usize = 128; // 128..176
    const SERVICE_OFFSET: usize = 176; // getaddrinfo service: NULL, or &SERVICE_STR
    const SERVICE_STR_OFFSET: usize = 184; // the C string "0" for the bind-all path

    let null_host = format!("{symbol}_null_host");
    let resolved = format!("{symbol}_resolved");
    let resolve_fail = format!("{symbol}_resolve_fail");
    let socket_fail = format!("{symbol}_socket_fail");
    let op_fail = format!("{symbol}_op_fail");
    let ctx_fail = format!("{symbol}_ctx_fail");
    let tls_fail_fd = format!("{symbol}_tls_fail_fd");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let alloc_fail_fd = format!("{symbol}_alloc_fail_fd");
    let alloc_fail_ctx_fd = format!("{symbol}_alloc_fail_ctx_fd");
    let done = format!("{symbol}_done");

    let addr_off = platform.addrinfo_addr_offset();
    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();

    // bug-575: the host, the certificate path and the key path are all copied into
    // arena C-strings for `getaddrinfo` / `SSL_CTX_use_*_file`, and none of them is
    // handed back — `tls::listen` leaked all three. The host scratch in particular
    // MUST be declared here rather than at its `emit_cstring`: the empty-host
    // bind-all path branches to `null_host` and jumps straight past the marshalling,
    // so the release at `done` would otherwise free an undefined vreg.
    let host_scratch = HelperScratch::declare(&mut vregs, &mut instructions);
    let cert_scratch = HelperScratch::declare(&mut vregs, &mut instructions);
    let key_scratch = HelperScratch::declare(&mut vregs, &mut instructions);

    // x0 = host; x1 = port; x2 = certPath; x3 = keyPath; x4 = backlog.
    instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), HOST_OFFSET),
        abi::store_u64(abi::c_arg(1), abi::stack_pointer(), PORT_OFFSET),
        abi::store_u64(abi::c_arg(2), abi::stack_pointer(), CERT_OFFSET),
        abi::store_u64(abi::c_arg(3), abi::stack_pointer(), KEY_OFFSET),
        abi::store_u64(abi::c_arg(4), abi::stack_pointer(), BACKLOG_OFFSET),
    ]);
    // bug-876: a port outside 0..=65535 is ErrAddressInvalid (the `resolve_fail`
    // arm), refused before getaddrinfo/socket exactly as tcp::listen does.
    crate::codegen::os::socket::shared::emit_port_range_check(
        PORT_OFFSET,
        &resolve_fail,
        &mut instructions,
        &mut vregs,
    );
    // Zero a 48-byte hints block; ai_flags = AI_PASSIVE, ai_family = AF_INET,
    // ai_socktype = SOCK_STREAM — the bind/listen resolution mirrors
    // net::listenTcp (an empty host binds all interfaces via a NULL node).
    for offset in (0..48).step_by(8) {
        instructions.push(abi::store_u64(
            abi::ZERO,
            abi::stack_pointer(),
            HINTS_OFFSET + offset,
        ));
    }
    instructions.extend([
        abi::move_immediate(&v9, "Integer", HINTS_FAMILY_WORD_PASSIVE),
        abi::store_u64(&v9, abi::stack_pointer(), HINTS_OFFSET),
        abi::move_immediate(&v9, "Integer", SOCK_STREAM),
        abi::store_u64(&v9, abi::stack_pointer(), HINTS_OFFSET + 8),
        // A named host resolves with a NULL service; only bind-all sets one.
        abi::store_u64(abi::ZERO, abi::stack_pointer(), SERVICE_OFFSET),
        // Empty host => NULL node (all interfaces).
        abi::load_u64(&v9, abi::stack_pointer(), HOST_OFFSET),
        abi::load_u64(&v9, &v9, 0),
        abi::compare_immediate(&v9, "0"),
        abi::branch_eq(&null_host),
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
        abi::branch(&resolved),
        abi::label(&null_host),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), HOSTCSTR_OFFSET),
        // bug-597: bind-all passes a NULL node, so the service must be non-NULL —
        // getaddrinfo(NULL, NULL, …) is EAI_NONAME on glibc and musl, which made
        // `tls::listen("", …)` raise ErrInvalidAddress on Linux while the page
        // documents it as binding every interface. Stage the C string "0" (0x30
        // then a zero terminator) and point service at it, exactly as the shared
        // socket helpers do for `tcp::listen` (bug-113). The real port overwrites
        // sin_port afterward.
        abi::move_immediate(&v9, "Integer", "48"),
        abi::store_u64(&v9, abi::stack_pointer(), SERVICE_STR_OFFSET),
        abi::add_immediate(&v9, abi::stack_pointer(), SERVICE_STR_OFFSET),
        abi::store_u64(&v9, abi::stack_pointer(), SERVICE_OFFSET),
        abi::label(&resolved),
        // getaddrinfo(host, service, &hints, &res)
        abi::load_u64(
            abi::return_register(),
            abi::stack_pointer(),
            HOSTCSTR_OFFSET,
        ),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), SERVICE_OFFSET),
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
        abi::branch_lt(&socket_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        // Overwrite sin_port (ai_addr + 2/3) with the requested port.
        abi::load_u64(&v9, abi::stack_pointer(), RES_OFFSET),
        abi::load_u64(&v9, &v9, addr_off),
        abi::load_u64(&v10, abi::stack_pointer(), PORT_OFFSET),
        abi::shift_right_immediate(&v11, &v10, 8),
        abi::store_u8(&v11, &v9, 2),
        abi::store_u8(&v10, &v9, 3),
        // setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &one, 4) - best effort.
        abi::move_immediate(&v9, "Integer", "1"),
        abi::store_u64(&v9, abi::stack_pointer(), ONE_OFFSET),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::move_immediate(abi::c_arg(1), "Integer", platform.sol_socket()),
        abi::move_immediate(abi::c_arg(2), "Integer", platform.so_reuseaddr()),
        abi::add_immediate(abi::c_arg(3), abi::stack_pointer(), ONE_OFFSET),
        abi::move_immediate(abi::c_arg(4), "Integer", "4"),
    ]);
    platform.emit_external_call(
        "setsockopt",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    // bind(fd, ai_addr, ai_addrlen)
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), RES_OFFSET),
        abi::load_u64(abi::c_arg(1), &v9, addr_off),
        abi::load_u32(abi::c_arg(2), &v9, 16),
    ]);
    platform.emit_external_call(
        "bind",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        // bug-102.3: narrow the C int `bind` return before the signed compare.
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&op_fail),
        // listen(fd, backlog)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), BACKLOG_OFFSET),
    ]);
    platform.emit_external_call(
        "listen",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        // bug-102.3: narrow the C int `listen` return before the signed compare.
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&op_fail),
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
    // --- Server identity: cert chain + private key into a server SSL_CTX ---
    emit_cstring(
        symbol,
        "cert",
        CERT_OFFSET,
        CERTCSTR_OFFSET,
        &alloc_fail_fd,
        &cert_scratch,
        &mut instructions,
        &mut relocations,
        &mut vregs,
    );
    emit_cstring(
        symbol,
        "key",
        KEY_OFFSET,
        KEYCSTR_OFFSET,
        &alloc_fail_fd,
        &key_scratch,
        &mut instructions,
        &mut relocations,
        &mut vregs,
    );
    emit_dlopen_libssl(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        &tls_fail_fd,
    )?;
    // method = TLS_server_method(); stash transiently in the CTX slot.
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "TLS_server_method",
        FNPTR_OFFSET,
        &tls_fail_fd,
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
        &tls_fail_fd,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CTX_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_eq(&tls_fail_fd),
        abi::store_u64(abi::c_return(0), abi::stack_pointer(), CTX_OFFSET),
    ]);
    // SSL_CTX_ctrl(ctx, SSL_CTRL_SET_MIN_PROTO_VERSION, TLS1_2_VERSION, NULL)
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_CTX_ctrl",
        FNPTR_OFFSET,
        &ctx_fail,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CTX_OFFSET),
        abi::move_immediate(abi::c_arg(1), "Integer", SSL_CTRL_SET_MIN_PROTO_VERSION),
        abi::move_immediate(abi::c_arg(2), "Integer", TLS1_2_VERSION),
        abi::move_immediate(abi::c_arg(3), "Integer", "0"),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        // Require the TLS 1.2 floor to have been set (returns 1 on success),
        // matching the checked identity-loading calls below; ctx_fail frees the
        // context and closes the fd — an unchecked failure would silently permit
        // a downgrade (bug-55).
        abi::compare_immediate(abi::c_return(0), "1"),
        abi::branch_ne(&ctx_fail),
    ]);
    // SSL_CTX_use_certificate_chain_file(ctx, certCstr) == 1
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_CTX_use_certificate_chain_file",
        FNPTR_OFFSET,
        &ctx_fail,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CTX_OFFSET),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), CERTCSTR_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        abi::compare_immediate(abi::c_return(0), "1"),
        abi::branch_ne(&ctx_fail),
    ]);
    // SSL_CTX_use_PrivateKey_file(ctx, keyCstr, SSL_FILETYPE_PEM = 1) == 1
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_CTX_use_PrivateKey_file",
        FNPTR_OFFSET,
        &ctx_fail,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CTX_OFFSET),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), KEYCSTR_OFFSET),
        abi::move_immediate(abi::c_arg(2), "Integer", "1"),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        abi::compare_immediate(abi::c_return(0), "1"),
        abi::branch_ne(&ctx_fail),
    ]);
    // SSL_CTX_check_private_key(ctx) == 1
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        HANDLE_OFFSET,
        "SSL_CTX_check_private_key",
        FNPTR_OFFSET,
        &ctx_fail,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CTX_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
        abi::compare_immediate(abi::c_return(0), "1"),
        abi::branch_ne(&ctx_fail),
    ]);
    // Build the Listener record: canonical header { tag, fd, closed=0, STATE=0 }
    // then the TLS tail { ctx } (plan-80).
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", TLS_RECORD_SIZE),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    // The server SSL_CTX already exists here, so an OOM must free it before
    // reporting: `alloc_fail_fd` closes the fd but leaked the context (bug-236).
    // It cannot gain an SSL_CTX_free itself — the pre-ctx cstring allocs share it
    // and the CTX slot is not live for them (the bug-201 class) — so route to a
    // dedicated ctx-freeing OOM exit that falls into it. connect/accept already do
    // full SSL/CTX/fd cleanup on this same OOM class (bug-55).
    emit_alloc(
        symbol,
        &mut instructions,
        &mut relocations,
        &alloc_fail_ctx_fd,
    );
    instructions.extend([
        abi::move_immediate(&v9, "Integer", RESOURCE_TAG_TLS_LISTENER),
        abi::store_u64(&v9, abi::mfb_return(1), RESOURCE_OFFSET_TAG),
        // bug-755: a fresh record is unowned until a binding claims it.
        abi::store_u64(abi::ZERO, abi::mfb_return(1), RESOURCE_OFFSET_OWNER),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), TLS_OFFSET_STATE),
        abi::load_u64(&v9, abi::stack_pointer(), FD_OFFSET),
        abi::store_u64(&v9, abi::mfb_return(1), TLS_LISTENER_OFFSET_FD),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), TLS_LISTENER_OFFSET_CLOSED),
        abi::load_u64(&v9, abi::stack_pointer(), CTX_OFFSET),
        abi::store_u64(&v9, abi::mfb_return(1), TLS_LISTENER_OFFSET_CTX),
        abi::move_register(RESULT_VALUE_REGISTER, abi::mfb_return(1)),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);

    // Error paths.
    // ctx_fail: the server context exists but the identity failed to load —
    // free the context, close the fd, and report ErrTlsFailed.
    instructions.push(abi::label(&ctx_fail));
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
        &tls_fail_fd,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CTX_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
    ]);
    instructions.push(abi::label(&tls_fail_fd));
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
    emit_fail(
        symbol,
        "ErrTlsFailed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    // bind/listen failure: close the fd, release the resolver results.
    instructions.push(abi::label(&op_fail));
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
    instructions.push(abi::label(&socket_fail));
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
    instructions.push(abi::label(&resolve_fail));
    emit_fail(
        symbol,
        "ErrAddressInvalid",
        &mut instructions,
        &mut relocations,
        &done,
    );
    // The record-alloc OOM exit taken once the server SSL_CTX exists: free the
    // context, then fall through into the shared fd-close + ErrOutOfMemory report
    // (bug-236). A `dlsym` miss for SSL_CTX_free still reports the OOM.
    instructions.push(abi::label(&alloc_fail_ctx_fd));
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
        &alloc_fail_fd,
    )?;
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CTX_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFFSET),
        abi::branch_link_register(&v9),
    ]);
    instructions.push(abi::label(&alloc_fail_fd));
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
    instructions.push(abi::label(&alloc_fail));
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
        &[host_scratch, cert_scratch, key_scratch],
        &mut vregs,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::return_());
    {
        Ok((instructions, relocations, FRAME_SIZE))
    }
}
