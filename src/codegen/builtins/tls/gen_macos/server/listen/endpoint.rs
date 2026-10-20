//! plan-176-T: `lower_tls_listen_macos`'s endpoint phase (`emit_listen_endpoint`). The carriers and frame consts live in
//! `builtins/tls/gen_macos/server/listen/mod.rs`.

use super::*;

/// `lower_tls_listen_macos` phase 2: the configure-TLS block, the secure-TCP parameters, the
/// bind host (null-host, numeric or resolved) and port, the local endpoint, the listener
/// and its dispatch queue.
pub(super) fn emit_listen_endpoint(
    env: &ListenEnv,
    scratch: &ListenScratch,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let ListenEnv {
        symbol,
        platform_imports,
        platform,
        v9,
        v10,
        ..
    } = *env;
    // Build the configure-TLS block that installs the local identity:
    // CFG_INVOKE copies the sec_protocol_options and calls the captured
    // setter with the captured payload — here
    // sec_protocol_options_set_local_identity(options, secIdentity).
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut *ins,
            relocations: &mut *rel,
        },
        NWH,
        "_NSConcreteStackBlock",
        FNPTR,
        &env.load_fail,
    )?;
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::store_u64(&v9, abi::stack_pointer(), CFGBLOCK + BLK_ISA),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), CFGBLOCK + BLK_FLAGS),
    ]);
    emit_data_address(symbol, &v9, CFG_INVOKE, &mut *ins, &mut *rel);
    ins.push(abi::store_u64(
        &v9,
        abi::stack_pointer(),
        CFGBLOCK + BLK_INVOKE,
    ));
    emit_data_address(symbol, &v9, CFG_DESC_SYMBOL, &mut *ins, &mut *rel);
    ins.push(abi::store_u64(
        &v9,
        abi::stack_pointer(),
        CFGBLOCK + BLK_DESC,
    ));
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), SECIDENT),
        abi::store_u64(&v9, abi::stack_pointer(), CFGBLOCK + CFG_CAP_SNAME),
    ]);
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut *ins,
            relocations: &mut *rel,
        },
        NWH,
        "nw_tls_copy_sec_protocol_options",
        FNPTR,
        &env.load_fail,
    )?;
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::store_u64(&v9, abi::stack_pointer(), CFGBLOCK + CFG_CAP_COPYFN),
    ]);
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut *ins,
            relocations: &mut *rel,
        },
        SECH,
        "sec_protocol_options_set_local_identity",
        FNPTR,
        &env.load_fail,
    )?;
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::store_u64(&v9, abi::stack_pointer(), CFGBLOCK + CFG_CAP_SETFN),
    ]);
    // nw_release: the invoke releases the +1 sec_protocol_options the copy fn
    // returns, so each listener stops leaking one (bug-116).
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut *ins,
            relocations: &mut *rel,
        },
        NWH,
        "nw_release",
        FNPTR,
        &env.load_fail,
    )?;
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::store_u64(&v9, abi::stack_pointer(), CFGBLOCK + CFG_CAP_RELEASEFN),
        // bug-477: the server installs no verify block. NULL is what tells the
        // shared invoke to skip that step; leaving stack garbage here would have
        // it call a wild pointer.
        abi::store_u64(abi::ZERO, abi::stack_pointer(), CFGBLOCK + CFG_CAP_VBLOCK),
        abi::store_u64(
            abi::ZERO,
            abi::stack_pointer(),
            CFGBLOCK + CFG_CAP_SETVERIFYFN,
        ),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), CFGBLOCK + CFG_CAP_QUEUE),
    ]);
    // cfg = *_nw_parameters_configure_protocol_default_configuration
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut *ins,
            relocations: &mut *rel,
        },
        NWH,
        "_nw_parameters_configure_protocol_default_configuration",
        FNPTR,
        &env.load_fail,
    )?;
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::load_u64(&v9, &v9, 0),
        abi::store_u64(&v9, abi::stack_pointer(), CFG),
    ]);
    // params = nw_parameters_create_secure_tcp(&cfgBlock, cfg)
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut *ins,
            relocations: &mut *rel,
        },
        NWH,
        "nw_parameters_create_secure_tcp",
        FNPTR,
        &env.load_fail,
    )?;
    ins.extend([
        abi::add_immediate(abi::return_register(), abi::stack_pointer(), CFGBLOCK),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), CFG),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&env.net_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), PARAMS),
    ]);
    // nw_parameters_set_reuse_local_address(params, true)
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut *ins,
            relocations: &mut *rel,
        },
        NWH,
        "nw_parameters_set_reuse_local_address",
        FNPTR,
        &env.load_fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), PARAMS),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
    ]);
    // Local endpoint: empty host binds all interfaces ("0.0.0.0").
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), HOST),
        abi::load_u64(&v10, &v9, 0),
        abi::compare_immediate(&v10, "0"),
        abi::branch_eq(&env.null_host),
    ]);
    emit_cstring(
        symbol,
        "host",
        HOST,
        HOSTCSTR,
        &env.alloc_fail,
        &scratch.host_scratch_until_the_listener_owns_it,
        &mut *ins,
        &mut *rel,
        &mut *vregs,
    );
    // bug-885: `host` is non-empty and not already known-good ("0.0.0.0"
    // took the `null_host` branch below). `nw_endpoint_create_host` cannot be
    // trusted to validate or resolve it: Network.framework treats anything
    // that is not already a usable local-bind endpoint — a hostname like
    // "localhost", a typo, an IPv6 literal, a link-local address with no
    // scope id — as unusable for a LOCAL bind, and silently listens on the
    // wildcard address on an OS-chosen port instead of failing. So resolve
    // `host` with `getaddrinfo` under an AF_INET hint, exactly as
    // `tcp::listen` does: a numeric IPv4 literal round-trips to itself, a
    // resolvable name (`"localhost"` -> 127.0.0.1) becomes its IPv4 address,
    // and anything else — including every IPv6-only literal, since the
    // AF_INET hint makes those unresolvable — raises `ErrAddressInvalid`
    // instead of binding anywhere. `REC_LHOST` (what `tls::localAddress`
    // reports, bug-465) keeps the ORIGINAL `HOSTCSTR` the caller passed, not
    // the resolved form — only `BINDHOSTCSTR`, fed to
    // `nw_endpoint_create_host` below, changes.
    //
    // A numeric IPv4 literal needs none of this -- it round-trips to itself
    // through getaddrinfo+inet_ntop at the cost of an arena alloc+free pair
    // every call, which a `tls::listen("127.0.0.1", ...)` loop can observe
    // (measured: `arena.0.alloc_calls` 20 higher than a build that never
    // resolves a literal). Check it with `inet_pton` first and skip the
    // resolve path entirely when it is already a bind-ready address.
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", "2"), // AF_INET
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), HOSTCSTR),
        abi::add_immediate(abi::c_arg(2), abi::stack_pointer(), PTON_BUF),
    ]);
    platform.emit_external_call("inet_pton", symbol, platform_imports, &mut *ins, &mut *rel)?;
    ins.extend([
        abi::compare_immediate(abi::return_register(), "1"),
        abi::branch_eq(&env.host_is_numeric),
    ]);
    for offset in (0..48).step_by(8) {
        ins.push(abi::store_u64(
            abi::ZERO,
            abi::stack_pointer(),
            RESOLVE_HINTS + offset,
        ));
    }
    ins.extend([
        abi::move_immediate(v9, "Integer", HINTS_FAMILY_WORD),
        abi::store_u64(v9, abi::stack_pointer(), RESOLVE_HINTS),
        abi::move_immediate(v9, "Integer", SOCK_STREAM),
        abi::store_u64(v9, abi::stack_pointer(), RESOLVE_HINTS + 8),
        // service = NULL (host is non-NULL here, so getaddrinfo accepts it).
        abi::store_u64(abi::ZERO, abi::stack_pointer(), RESOLVE_SERVICE),
        // getaddrinfo(host, NULL, &hints, &res)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), HOSTCSTR),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), RESOLVE_SERVICE),
        abi::add_immediate(abi::c_arg(2), abi::stack_pointer(), RESOLVE_HINTS),
        abi::add_immediate(abi::c_arg(3), abi::stack_pointer(), RESOLVE_RES),
    ]);
    platform.emit_external_call(
        "getaddrinfo",
        symbol,
        platform_imports,
        &mut *ins,
        &mut *rel,
    )?;
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&env.addr_invalid_fail),
        // Stash the resolved sockaddr (ai_addr) before the inet_ntop buffer's
        // allocation, which clobbers x0/x1.
        abi::load_u64(v9, abi::stack_pointer(), RESOLVE_RES),
        abi::load_u64(v9, v9, platform.addrinfo_addr_offset()),
        abi::store_u64(v9, abi::stack_pointer(), RESOLVE_SOCKADDR),
        abi::move_immediate(abi::return_register(), "Integer", "64"),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    emit_alloc(symbol, &mut *ins, &mut *rel, &env.alloc_fail);
    ins.extend([
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), RESOLVEDCSTR),
        // inet_ntop(AF_INET, sockaddr + 4, dst, 64)
        abi::move_immediate(abi::return_register(), "Integer", "2"),
        abi::load_u64(v9, abi::stack_pointer(), RESOLVE_SOCKADDR),
        abi::add_immediate(abi::c_arg(1), v9, 4),
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), RESOLVEDCSTR),
        abi::move_immediate(abi::c_arg(3), "Integer", "64"),
    ]);
    platform.emit_external_call("inet_ntop", symbol, platform_imports, &mut *ins, &mut *rel)?;
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&env.inet_ntop_fail),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), RESOLVE_RES),
    ]);
    platform.emit_external_call(
        "freeaddrinfo",
        symbol,
        platform_imports,
        &mut *ins,
        &mut *rel,
    )?;
    ins.extend([
        abi::load_u64(v9, abi::stack_pointer(), RESOLVEDCSTR),
        abi::store_u64(v9, abi::stack_pointer(), BINDHOSTCSTR),
    ]);
    ins.push(abi::branch(&env.host_ready));
    ins.push(abi::label(&env.host_is_numeric));
    ins.extend([
        abi::load_u64(v9, abi::stack_pointer(), HOSTCSTR),
        abi::store_u64(v9, abi::stack_pointer(), BINDHOSTCSTR),
    ]);
    ins.push(abi::branch(&env.host_ready));
    ins.push(abi::label(&env.null_host));
    // bug-984: an arena copy of the static `"0.0.0.0"`, so the block the record
    // parks at `REC_LHOST` is one its close can free whichever path bound it. The
    // string and its NUL are exactly one word.
    const _: () = assert!(ANYHOST.len() + 1 == 8);
    ins.extend([
        abi::move_immediate(
            scratch.host_scratch_until_the_listener_owns_it.size,
            "Integer",
            "8",
        ),
        abi::move_register(
            abi::return_register(),
            scratch.host_scratch_until_the_listener_owns_it.size,
        ),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    emit_alloc(symbol, &mut *ins, &mut *rel, &env.alloc_fail);
    ins.push(abi::move_register(
        scratch.host_scratch_until_the_listener_owns_it.pointer,
        abi::mfb_return(1),
    ));
    emit_data_address(symbol, v9, ANYHOST_SYMBOL, &mut *ins, &mut *rel);
    ins.extend([
        abi::load_u64(v10, v9, 0),
        abi::store_u64(
            v10,
            scratch.host_scratch_until_the_listener_owns_it.pointer,
            0,
        ),
        abi::store_u64(
            scratch.host_scratch_until_the_listener_owns_it.pointer,
            abi::stack_pointer(),
            HOSTCSTR,
        ),
        abi::store_u64(
            scratch.host_scratch_until_the_listener_owns_it.pointer,
            abi::stack_pointer(),
            BINDHOSTCSTR,
        ),
        abi::label(&env.host_ready),
    ]);
    // itoa(port) -> NUL-terminated decimal at PORTBUF, pointer in PORTCSTR.
    emit_port_itoa(symbol, PORT, PORTBUF, PORTCSTR, &mut *ins, &mut *vregs);
    // endpoint = nw_endpoint_create_host(host, port)
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut *ins,
            relocations: &mut *rel,
        },
        NWH,
        "nw_endpoint_create_host",
        FNPTR,
        &env.load_fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), BINDHOSTCSTR),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), PORTCSTR),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        // Stash the result (NULL or a real endpoint) before anything else
        // clobbers x0 -- the free below calls into the arena allocator.
        abi::store_u64(abi::return_register(), abi::stack_pointer(), ENDPOINT),
    ]);
    // bug-885 (review): `nw_endpoint_create_host` has read `BINDHOSTCSTR` by
    // now on every outcome (it returns a new, independent endpoint object, or
    // NULL), so the resolved-host buffer this call allocated can be freed
    // unconditionally here rather than carried any further. Every non-empty
    // `host` (numeric literal or resolved name alike) allocates it; only the
    // empty-host `null_host` branch does not, which is why it was zeroed at
    // entry -- that zero is what lets this be a plain non-NULL check instead
    // of a separate flag.
    ins.extend([
        abi::load_u64(v9, abi::stack_pointer(), RESOLVEDCSTR),
        abi::compare_immediate(v9, "0"),
        abi::branch_eq(&env.resolved_buf_freed),
        abi::move_register(abi::return_register(), v9),
        abi::move_immediate(abi::c_arg(1), "Integer", "64"),
    ]);
    emit_arena_free(symbol, &mut *ins, &mut *rel);
    ins.push(abi::store_u64(
        abi::ZERO,
        abi::stack_pointer(),
        RESOLVEDCSTR,
    ));
    ins.extend([
        abi::label(&env.resolved_buf_freed),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), ENDPOINT),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&env.net_fail),
    ]);
    // nw_parameters_set_local_endpoint(params, endpoint)
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut *ins,
            relocations: &mut *rel,
        },
        NWH,
        "nw_parameters_set_local_endpoint",
        FNPTR,
        &env.load_fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), PARAMS),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), ENDPOINT),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
    ]);
    // listener = nw_listener_create(params)
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut *ins,
            relocations: &mut *rel,
        },
        NWH,
        "nw_listener_create",
        FNPTR,
        &env.load_fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), PARAMS),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&env.net_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), LISTENER),
    ]);
    // The endpoint is retained into the parameters (set_local_endpoint) and the
    // parameters are retained by the listener (nw_listener_create), so release
    // our own references now; otherwise every successful listen leaks one
    // nw_endpoint and one nw_parameters (bug-55).
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut *ins,
            relocations: &mut *rel,
        },
        NWH,
        "nw_release",
        FNPTR,
        &env.load_fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), ENDPOINT),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), PARAMS),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
    ]);
    // queue = dispatch_queue_create("mfb.tls", NULL)
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut *ins,
            relocations: &mut *rel,
        },
        NWH,
        "dispatch_queue_create",
        FNPTR,
        &env.load_fail,
    )?;
    emit_data_address(
        symbol,
        abi::return_register(),
        QLABEL_SYMBOL,
        &mut *ins,
        &mut *rel,
    );
    ins.extend([
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), QUEUE),
    ]);
    Ok(())
}
