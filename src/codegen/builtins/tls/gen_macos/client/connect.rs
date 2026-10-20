//! plan-176-T: the Network.framework `tls::connect` lowering (`lower_tls_connect_macos`). The
//! shared imports live in `builtins/tls/gen_macos/client/mod.rs`.

use super::*;

pub(crate) fn lower_tls_connect_macos(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    address: bool,
) -> Result<TlsBodyParts, String> {
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    const FRAME_SIZE: usize = 384;
    const HOST: usize = 8;
    const PORT: usize = 16;
    const HANDLE: usize = 24;
    const FNPTR: usize = 32;
    const CTX: usize = 40;
    const ENDPOINT: usize = 48;
    const PARAMS: usize = 56;
    const CONN: usize = 64;
    const QUEUE: usize = 72;
    const HOSTCSTR: usize = 80;
    const PORTCSTR: usize = 88;
    const CFG: usize = 96;
    const WAITFN: usize = 104;
    const BLOCK: usize = 112; // 112..152
    const PORTBUF: usize = 152; // 152..176
    const SNAME: usize = 176; // serverName String ptr (arg x3)
    const SNICSTR: usize = 184; // serverName as a C string
    const TLSCFG: usize = 192; // chosen configure-TLS block pointer
                               // bug-477 grew this block from 64 to 88 bytes (three more captures), so
                               // everything after it moved up 24. Getting this wrong is silent and total:
                               // at the old offsets `CFG_CAP_QUEUE` landed exactly on `ALLOW`, so storing
                               // the queue zeroed the flag and the verify block was never installed —
                               // `allowSelfSigned := TRUE` behaved identically to omitting it.
    const CFGBLOCK: usize = 200; // 200..288: the configure block literal
    const TIMEOUT: usize = 288; // timeoutMs (arg x2)
    const DEADLINE: usize = 296; // dispatch_time deadline for the wait
    const ALLOW: usize = 304; // bug-477: allowSelfSigned (0/1)
    const SECH: usize = 312; // bug-477: Security.framework handle
    const CFH: usize = 320; // bug-477: CoreFoundation handle
    const VBLOCK: usize = 328; // bug-477: 328..368, the verify block literal
    const VNAME: usize = 368; // bug-477: the name the verify block validates against

    let wait_loop = format!("{symbol}_wait");
    let ready = format!("{symbol}_ready");
    let conn_fail = format!("{symbol}_conn_fail");
    let conn_timeout = format!("{symbol}_conn_timeout");
    let conn_invalid = format!("{symbol}_conn_invalid");
    let port_invalid = format!("{symbol}_port_invalid");
    let wait_forever = format!("{symbol}_wait_forever");
    let wait_now = format!("{symbol}_wait_now");
    let deadline_ready = format!("{symbol}_deadline_ready");
    let net_fail = format!("{symbol}_net_fail");
    let load_fail = format!("{symbol}_load_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let sni_default = format!("{symbol}_sni_default");
    let have_sname = format!("{symbol}_have_sname");
    let sname_done = format!("{symbol}_sname_done");
    let verify_done = format!("{symbol}_verify_done");
    let done = format!("{symbol}_done");

    // bug-477: the configure block literal must not run into the next frame
    // slot. This is the exact overlap that made the flag read 0.
    const _: () = assert!(CFGBLOCK + CFG_BLOCK_SIZE <= TIMEOUT);
    const _: () = assert!(VBLOCK + 40 <= VNAME);
    const _: () = assert!(VNAME + 8 <= FRAME_SIZE);
    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel = Vec::new();
    // bug-575: the host C-string `nw_endpoint_create_host` reads, and the SNI copy
    // `sec_protocol_options_set_tls_server_name` reads, are this helper's own
    // scratch — handed to Network.framework and never returned to MFBASIC, so no
    // caller-side ownership analysis could reach them and every `tls::connect`
    // leaked its host name. Declared here, ahead of every branch that can reach
    // `done`: the `conn_invalid` timeout rejection returns before marshalling
    // anything, and the SNI copy only happens on the `have_sname` arm.
    let host_scratch = HelperScratch::declare(&mut vregs, &mut ins);
    let sni_scratch = HelperScratch::declare(&mut vregs, &mut ins);
    // Host form: x0 = host; x1 = port; x2 = timeoutMs; x3 = serverName; x4 = allowSelfSigned.
    // Address form: x0 = net::Address; x1 = timeoutMs; x2 = serverName; x3 = allowSelfSigned.
    ins.extend(
        crate::codegen::builtins::tls::gen_shared::connect_arg_prologue(
            address, &v9, HOST, PORT, TIMEOUT, SNAME, ALLOW,
        ),
    );
    // bug-876: a port outside 0..=65535 is ErrAddressInvalid, refused before any
    // dlopen/alloc/connection work exactly as tcp::connect does.
    crate::codegen::os::socket::shared::emit_port_range_check(
        PORT,
        &port_invalid,
        &mut ins,
        &mut vregs,
    );
    {
        // plan-73-D: reject a negative (non-sentinel) `timeoutMs` up front — before
        // any dlopen/alloc/connection, so the reject leaks nothing. The omitted
        // overload pads the unbounded sentinel (i64::MIN), which is allowed (→ FOREVER
        // at the deadline block); `0`/`> 0` pass through.
        let ts_ok = format!("{symbol}_ts_ok");
        let ts_store = format!("{symbol}_ts_clamped");
        ins.extend([
            abi::load_u64(&v9, abi::stack_pointer(), TIMEOUT),
            abi::move_immediate(&v10, "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
            abi::compare_registers(&v9, &v10),
            abi::branch_eq(&ts_ok),
            abi::compare_immediate(&v9, "0"),
            abi::branch_lt(&conn_invalid),
            // Clamp `> 0` to INT_MAX and store it back so `ms * 1e6` cannot overflow
            // and every backend treats a huge timeout identically (bounded at INT_MAX
            // ms), matching net and the poll-based backends. Sentinel skips this.
            abi::move_immediate(&v10, "Integer", "2147483647"),
            abi::compare_registers(&v9, &v10),
            abi::branch_le(&ts_store),
            abi::move_register(&v9, &v10),
            abi::label(&ts_store),
            abi::store_u64(&v9, abi::stack_pointer(), TIMEOUT),
            abi::label(&ts_ok),
        ]);
    }
    // itoa(port) -> NUL-terminated decimal at PORTBUF, pointer in PORTCSTR.
    emit_port_itoa(symbol, PORT, PORTBUF, PORTCSTR, &mut ins, &mut vregs);
    // dlopen Network.framework.
    emit_data_address(
        symbol,
        abi::return_register(),
        MACLIB_SYMBOL,
        &mut ins,
        &mut rel,
    );
    ins.push(abi::move_immediate(abi::c_arg(1), "Integer", RTLD_NOW));
    platform.emit_external_call("dlopen", symbol, platform_imports, &mut ins, &mut rel)?;
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), HANDLE),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&load_fail),
    ]);
    emit_cstring(
        symbol,
        "host",
        HOST,
        HOSTCSTR,
        &alloc_fail,
        &host_scratch,
        &mut ins,
        &mut rel,
        &mut vregs,
    );
    // Allocate the block context on the C heap (bug-623 D, see `CTX_SIZE`).
    emit_ctx_calloc(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        CTX_SIZE,
        &alloc_fail,
    )?;
    ins.push(abi::store_u64(
        abi::mfb_return(1),
        abi::stack_pointer(),
        CTX,
    ));
    // endpoint = nw_endpoint_create_host(host, port)
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "nw_endpoint_create_host",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), HOSTCSTR),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), PORTCSTR),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&net_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), ENDPOINT),
    ]);
    // bug-477: the queue is created HERE, before the parameters, because the
    // configure block runs synchronously inside `nw_parameters_create_secure_tcp`
    // and `sec_protocol_options_set_verify_block` needs a queue to hand the
    // verify block. It used to be created after the parameters; nothing else
    // depends on the ordering (it needs only the Network.framework handle).
    // queue = dispatch_queue_create("mfb.tls", NULL)
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "dispatch_queue_create",
        FNPTR,
        &load_fail,
    )?;
    emit_data_address(
        symbol,
        abi::return_register(),
        QLABEL_SYMBOL,
        &mut ins,
        &mut rel,
    );
    ins.extend([
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), QUEUE),
    ]);
    // cfg = *_nw_parameters_configure_protocol_default_configuration
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "_nw_parameters_configure_protocol_default_configuration",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::load_u64(&v9, &v9, 0),
        abi::store_u64(&v9, abi::stack_pointer(), CFG),
        // The configure-TLS block defaults to the system default. The custom
        // block replaces it when EITHER a non-empty serverName overrides the SNI
        // / certificate-validation name, OR (bug-477) allowSelfSigned needs a
        // verify block installed. The two are independent: the flag may be set
        // with no serverName, in which case the name stays the endpoint host.
        abi::store_u64(&v9, abi::stack_pointer(), TLSCFG),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), SNICSTR),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), CFGBLOCK + CFG_CAP_VBLOCK),
        // The verify block validates against serverName when given, else host.
        abi::load_u64(&v9, abi::stack_pointer(), HOSTCSTR),
        abi::store_u64(&v9, abi::stack_pointer(), VNAME),
        abi::load_u64(&v9, abi::stack_pointer(), SNAME),
        abi::load_u64(&v10, &v9, 0),
        abi::compare_immediate(&v10, "0"),
        abi::branch_ne(&have_sname),
        abi::load_u64(&v10, abi::stack_pointer(), ALLOW),
        abi::compare_immediate(&v10, "0"),
        abi::branch_eq(&sni_default),
        abi::branch(&sname_done),
        abi::label(&have_sname),
    ]);
    // serverName given: copy it to a C string and build a configure block
    // whose invoke calls sec_protocol_options_set_tls_server_name. The block
    // is invoked synchronously during nw_parameters_create_secure_tcp, so the
    // stack literal stays live for its whole lifetime.
    emit_cstring(
        symbol,
        "sni",
        SNAME,
        SNICSTR,
        &alloc_fail,
        &sni_scratch,
        &mut ins,
        &mut rel,
        &mut vregs,
    );
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), SNICSTR),
        abi::store_u64(&v9, abi::stack_pointer(), VNAME),
        // Both paths rejoin BEFORE the block literal is built: a flag-only call
        // has no serverName but still needs the custom configure block.
        abi::label(&sname_done),
    ]);
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "_NSConcreteStackBlock",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::store_u64(&v9, abi::stack_pointer(), CFGBLOCK + BLK_ISA),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), CFGBLOCK + BLK_FLAGS),
    ]);
    emit_data_address(symbol, &v9, CFG_INVOKE, &mut ins, &mut rel);
    ins.push(abi::store_u64(
        &v9,
        abi::stack_pointer(),
        CFGBLOCK + BLK_INVOKE,
    ));
    emit_data_address(symbol, &v9, CFG_DESC_SYMBOL, &mut ins, &mut rel);
    ins.push(abi::store_u64(
        &v9,
        abi::stack_pointer(),
        CFGBLOCK + BLK_DESC,
    ));
    ins.extend([
        // NULL here means "do not call set_tls_server_name", which is exactly the
        // empty-serverName contract; the invoke null-checks it (bug-477).
        abi::load_u64(&v9, abi::stack_pointer(), SNICSTR),
        abi::store_u64(&v9, abi::stack_pointer(), CFGBLOCK + CFG_CAP_SNAME),
    ]);
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "nw_tls_copy_sec_protocol_options",
        FNPTR,
        &load_fail,
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
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "sec_protocol_options_set_tls_server_name",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::store_u64(&v9, abi::stack_pointer(), CFGBLOCK + CFG_CAP_SETFN),
    ]);
    // nw_release: the invoke releases the +1 sec_protocol_options the copy fn
    // returns, so each configured connection stops leaking one (bug-116).
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "nw_release",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::store_u64(&v9, abi::stack_pointer(), CFGBLOCK + CFG_CAP_RELEASEFN),
        abi::store_u64(
            abi::ZERO,
            abi::stack_pointer(),
            CFGBLOCK + CFG_CAP_SETVERIFYFN,
        ),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), CFGBLOCK + CFG_CAP_QUEUE),
    ]);
    // bug-477 `allowSelfSigned`: build the verify block and hand the configure
    // block everything it needs to install it. Skipped entirely when the flag is
    // off, so a strict connection performs no extra dlopen and carries a NULL
    // CFG_CAP_VBLOCK — which the invoke reads as "leave verification alone".
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), ALLOW),
        abi::compare_immediate(&v9, "0"),
        abi::branch_eq(&verify_done),
    ]);
    // Security.framework and CoreFoundation. These used to be opened only by the
    // server path (identity import); the client verify block calls `Sec*`/`CF*`
    // too, so it opens them itself rather than depending on that path having run.
    for (lib, slot) in [(MACSEC_SYMBOL, SECH), (MACCF_SYMBOL, CFH)] {
        emit_dlopen_at(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: &mut ins,
                relocations: &mut rel,
            },
            lib,
            slot,
            &load_fail,
        )?;
    }
    // Publish every entry point the verify block calls. It runs on the dispatch
    // queue — a different thread — so it cannot re-`dlsym`; it reads this global
    // table instead. Every connect writes the same process-wide constants.
    for (name, framework) in VERIFY_FN_NAMES {
        let handle = match framework {
            Framework::Network => HANDLE,
            Framework::Security => SECH,
            Framework::CoreFoundation => CFH,
        };
        dlsym(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: &mut ins,
                relocations: &mut rel,
            },
            handle,
            name,
            FNPTR,
            &load_fail,
        )?;
        emit_data_address(symbol, &v10, VERIFY_FNS_SYMBOL, &mut ins, &mut rel);
        ins.extend([
            abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
            abi::store_u64(&v9, &v10, verify_fn_slot(name)),
        ]);
    }
    // The verify block literal: one capture, the name to validate against.
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "_NSConcreteStackBlock",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::store_u64(&v9, abi::stack_pointer(), VBLOCK + BLK_ISA),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), VBLOCK + BLK_FLAGS),
    ]);
    emit_data_address(symbol, &v9, VERIFY_INVOKE, &mut ins, &mut rel);
    ins.push(abi::store_u64(
        &v9,
        abi::stack_pointer(),
        VBLOCK + BLK_INVOKE,
    ));
    // DESC_SYMBOL is the size-40 descriptor: 32-byte header + one capture.
    emit_data_address(symbol, &v9, DESC_SYMBOL, &mut ins, &mut rel);
    ins.push(abi::store_u64(&v9, abi::stack_pointer(), VBLOCK + BLK_DESC));
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), VNAME),
        abi::store_u64(&v9, abi::stack_pointer(), VBLOCK + VERIFY_CAP_SNAME),
    ]);
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "sec_protocol_options_set_verify_block",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::store_u64(&v9, abi::stack_pointer(), CFGBLOCK + CFG_CAP_SETVERIFYFN),
        abi::load_u64(&v9, abi::stack_pointer(), QUEUE),
        abi::store_u64(&v9, abi::stack_pointer(), CFGBLOCK + CFG_CAP_QUEUE),
        abi::add_immediate(&v9, abi::stack_pointer(), VBLOCK),
        abi::store_u64(&v9, abi::stack_pointer(), CFGBLOCK + CFG_CAP_VBLOCK),
    ]);
    ins.push(abi::label(&verify_done));
    ins.extend([
        // tlscfg = &block
        abi::add_immediate(&v9, abi::stack_pointer(), CFGBLOCK),
        abi::store_u64(&v9, abi::stack_pointer(), TLSCFG),
    ]);
    ins.push(abi::label(&sni_default));
    // params = nw_parameters_create_secure_tcp(tlscfg, cfg)
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "nw_parameters_create_secure_tcp",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), TLSCFG),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), CFG),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&net_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), PARAMS),
    ]);
    // conn = nw_connection_create(endpoint, params)
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "nw_connection_create",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), ENDPOINT),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), PARAMS),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&net_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), CONN),
    ]);
    // nw_connection_create retains both the endpoint and the parameters, so
    // release our own references now; otherwise every successful connect leaks
    // one nw_endpoint and one nw_parameters (bug-55). The connection (CONN),
    // queue, and ctx are handed to the Socket record and released on close.
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "nw_release",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), ENDPOINT),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), PARAMS),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
    ]);
    // ctx->sem = dispatch_semaphore_create(0)
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "dispatch_semaphore_create",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::store_u64(abi::return_register(), &v9, CTX_SEM),
    ]);
    // plan-76-B Phase 4: create the dedicated poll-receive semaphore (value 0) and
    // zero the outstanding-receive/pending slots. Reuses the just-resolved
    // dispatch_semaphore_create in FNPTR.
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::store_u64(abi::return_register(), &v9, CTX_PSEM),
        abi::store_u64(abi::ZERO, &v9, CTX_PCONTENT),
        abi::store_u64(abi::ZERO, &v9, CTX_PERROR),
        abi::store_u64(abi::ZERO, &v9, CTX_PEND_BUF),
        abi::store_u64(abi::ZERO, &v9, CTX_PEND_LEN),
        abi::store_u64(abi::ZERO, &v9, CTX_PEND_OFF),
        abi::store_u64(abi::ZERO, &v9, CTX_ARMED),
        // plan-110-D: no read/write deadline until one is installed. The
        // sentinel is what makes the waits below stay FOREVER on a socket
        // whose owner never called a timeout setter.
        abi::move_immediate(&v10, "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
        abi::store_u64(&v10, &v9, CTX_RTO),
        abi::store_u64(&v10, &v9, CTX_WTO),
        abi::store_u64(abi::ZERO, &v9, CTX_WARMED),
    ]);
    // ctx->signal = &dispatch_semaphore_signal
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "dispatch_semaphore_signal",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(&v10, abi::stack_pointer(), FNPTR),
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::store_u64(&v10, &v9, CTX_SIGNAL),
    ]);
    // bug-483: ctx->edomfn = &nw_error_get_error_domain, and edom = 0 ("no error
    // classified yet"). Only the send trampoline may touch an `nw_error` — it is
    // released when the completion block returns — so the pointer is parked here
    // for it.
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "nw_error_get_error_domain",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(&v10, abi::stack_pointer(), FNPTR),
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::store_u64(&v10, &v9, CTX_EDOMFN),
        abi::store_u64(abi::ZERO, &v9, CTX_EDOM),
    ]);
    // nw_connection_set_queue(conn, queue)
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "nw_connection_set_queue",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CONN),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), QUEUE),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
    ]);
    // Build the state-changed block literal on the stack.
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "_NSConcreteStackBlock",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::store_u64(&v9, abi::stack_pointer(), BLOCK + BLK_ISA),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), BLOCK + BLK_FLAGS),
    ]);
    emit_data_address(symbol, &v9, STATE_INVOKE, &mut ins, &mut rel);
    ins.push(abi::store_u64(
        &v9,
        abi::stack_pointer(),
        BLOCK + BLK_INVOKE,
    ));
    emit_data_address(symbol, &v9, DESC_SYMBOL, &mut ins, &mut rel);
    ins.push(abi::store_u64(&v9, abi::stack_pointer(), BLOCK + BLK_DESC));
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::store_u64(&v9, abi::stack_pointer(), BLOCK + BLK_CAP),
    ]);
    // nw_connection_set_state_changed_handler(conn, &block)
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "nw_connection_set_state_changed_handler",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CONN),
        abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), BLOCK),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
    ]);
    // nw_connection_start(conn)
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "nw_connection_start",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CONN),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
    ]);
    // plan-73-D. Compute the wait deadline: the unbounded sentinel =>
    // DISPATCH_TIME_FOREVER (omit = block); `0` => DISPATCH_TIME_NOW (one immediate
    // attempt → `ErrTimeout` if the handshake is not instantly complete); `> 0` =>
    // dispatch_time(NOW, ms*1e6). Negatives were rejected up front. The deadline is
    // absolute, so re-waits across the preparing loop all share it.
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), TIMEOUT),
        abi::move_immediate(&v10, "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
        abi::compare_registers(&v9, &v10),
        abi::branch_eq(&wait_forever),
        abi::compare_immediate(&v9, "0"),
        abi::branch_eq(&wait_now),
    ]);
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "dispatch_time",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"), // DISPATCH_TIME_NOW
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), TIMEOUT),
        abi::move_immediate(&v10, "Integer", "1000000"),
        abi::multiply_registers(abi::c_arg(1), abi::c_arg(1), &v10), // ms -> ns
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), DEADLINE),
        abi::branch(&deadline_ready),
        // 0 => DISPATCH_TIME_NOW (0): the semaphore wait returns at once.
        abi::label(&wait_now),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), DEADLINE),
        abi::branch(&deadline_ready),
        abi::label(&wait_forever),
        abi::move_immediate(&v9, "Integer", "0"),
        abi::bitwise_not(&v9, &v9), // DISPATCH_TIME_FOREVER
        abi::store_u64(&v9, abi::stack_pointer(), DEADLINE),
        abi::label(&deadline_ready),
    ]);
    // Wait for a terminal state, bounded by the deadline.
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "dispatch_semaphore_wait",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::store_u64(&v9, abi::stack_pointer(), WAITFN),
        abi::label(&wait_loop),
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::load_u64(abi::return_register(), &v9, CTX_SEM),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), DEADLINE),
        abi::load_u64(&v10, abi::stack_pointer(), WAITFN),
        abi::branch_link_register(&v10),
        // Non-zero => the deadline elapsed before any state change signalled.
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&conn_timeout),
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::load_u32(&v10, &v9, CTX_STATE),
        abi::compare_immediate(&v10, NW_STATE_READY),
        abi::branch_eq(&ready),
        abi::compare_immediate(&v10, "2"), // preparing
        abi::branch_eq(&wait_loop),
        abi::compare_immediate(&v10, "0"), // invalid
        abi::branch_eq(&wait_loop),
        abi::branch(&conn_fail), // waiting/failed/cancelled
        abi::label(&ready),
    ]);
    // Build the Socket record: header { tag, conn, closed=0, STATE=0 } then
    // the macOS tail { ctx, queue } (plan-80).
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", REC_SIZE),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.extend([
        abi::move_immediate(&v9, "Integer", RESOURCE_TAG_TLS_MACOS),
        abi::store_u64(&v9, abi::mfb_return(1), REC_TAG),
        // bug-755: a fresh record is unowned until a binding claims it.
        abi::store_u64(abi::ZERO, abi::mfb_return(1), RESOURCE_OFFSET_OWNER),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), REC_STATE),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), REC_CLOSED),
        abi::load_u64(&v9, abi::stack_pointer(), CONN),
        abi::store_u64(&v9, abi::mfb_return(1), REC_CONN),
        abi::load_u64(&v9, abi::stack_pointer(), QUEUE),
        abi::store_u64(&v9, abi::mfb_return(1), REC_QUEUE),
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::store_u64(&v9, abi::mfb_return(1), REC_CTX),
        abi::move_register(RESULT_VALUE_REGISTER, abi::mfb_return(1)),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    // conn_fail / conn_timeout: cancel the connection, then release the two
    // objects this failed connect still owns — the nw_connection (+1 from
    // nw_connection_create) and its per-connection dispatch queue. Both labels
    // are reached only after CONN and QUEUE are stored, and the success path
    // hands them to the record for close to release; before bug-317 these exits
    // only cancelled, so a client reconnect loop against an unreachable or
    // untrusted host leaked one connection and one queue per attempt.
    ins.push(abi::label(&conn_fail));
    emit_cancel_and_release_conn(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        CONN,
        FNPTR,
        &load_fail,
        &mut vregs,
    )?;
    emit_release_queue(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        QUEUE,
        FNPTR,
        &load_fail,
        &mut vregs,
    )?;
    // Drain to the terminal `cancelled` state before returning (bug-380).
    // `nw_connection_cancel` is asynchronous, and the state-changed handler
    // (STATE_INVOKE) dereferences the arena-allocated `ctx` on *every* invocation.
    // Letting the failed-connect helper return before the connection's final
    // `cancelled` transition lets that handler run against a freed `ctx` after the
    // program exits → EXC_BAD_ACCESS on the mfb.tls queue (intermittent,
    // load-dependent). `cancelled` (state 5) is terminal — nothing transitions
    // after it — so waiting until `ctx->state` reaches it guarantees no handler
    // runs afterward, no matter how many transitions `cancel` produced or whether
    // a leftover signal is consumed first. This mirrors the connect wait loop
    // above and reuses its resolved `dispatch_semaphore_wait` in WAITFN.
    let cancel_drain = format!("{symbol}_cancel_drain");
    ins.extend([
        abi::label(&cancel_drain),
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::load_u64(abi::return_register(), &v9, CTX_SEM),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        abi::bitwise_not(abi::c_arg(1), abi::c_arg(1)), // DISPATCH_TIME_FOREVER
        abi::load_u64(&v10, abi::stack_pointer(), WAITFN),
        abi::branch_link_register(&v10),
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::load_u32(&v10, &v9, CTX_STATE),
        abi::compare_immediate(&v10, "5"), // nw_connection_state_cancelled
        abi::branch_ne(&cancel_drain),
        // bug-623: drained to `cancelled`, no handler can reach the ctx any more,
        // and no receive/send was ever posted on this never-ready connection — so
        // the ctx this helper allocated goes back to the C heap instead of
        // leaking one block per failed connect.
        abi::load_u64(abi::c_arg(0), abi::stack_pointer(), CTX),
    ]);
    emit_ctx_free(&mut EmitCtx {
        symbol,
        platform_imports,
        platform,
        instructions: &mut ins,
        relocations: &mut rel,
    })?;
    emit_fail(symbol, "ErrTlsFailed", &mut ins, &mut rel, &done);
    // conn_timeout: the deadline elapsed; cancel the connection, report a
    // timeout.
    ins.push(abi::label(&conn_timeout));
    emit_cancel_and_release_conn(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        CONN,
        FNPTR,
        &load_fail,
        &mut vregs,
    )?;
    emit_release_queue(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        QUEUE,
        FNPTR,
        &load_fail,
        &mut vregs,
    )?;
    emit_fail(symbol, "ErrTimeout", &mut ins, &mut rel, &done);
    ins.push(abi::label(&net_fail));
    emit_fail(symbol, "ErrNetworkFailed", &mut ins, &mut rel, &done);
    // bug-876: out-of-range port; reached before any acquisition, so no cleanup.
    ins.push(abi::label(&port_invalid));
    emit_fail(symbol, "ErrAddressInvalid", &mut ins, &mut rel, &done);
    // plan-73-D: a negative (non-sentinel) `timeoutMs` → ErrInvalidArgument. Reached
    // from the up-front check before any dlopen/alloc/connection, so no cleanup.
    ins.push(abi::label(&conn_invalid));
    emit_fail(symbol, "ErrInvalidArgument", &mut ins, &mut rel, &done);
    ins.push(abi::label(&load_fail));
    emit_fail(symbol, "ErrTlsFailed", &mut ins, &mut rel, &done);
    ins.push(abi::label(&alloc_fail));
    emit_fail(symbol, "ErrOutOfMemory", &mut ins, &mut rel, &done);
    ins.push(abi::label(&done));
    emit_helper_scratch_release(
        symbol,
        &[host_scratch, sni_scratch],
        &mut vregs,
        &mut ins,
        &mut rel,
    );
    ins.push(abi::return_());
    {
        Ok((ins, rel, FRAME_SIZE))
    }
}
