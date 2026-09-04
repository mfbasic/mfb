// --- codegen tier imports (migration) ---
use super::*;
use crate::codegen::engine::builder::emit_arena_free;
use crate::target::shared::abi;
use std::collections::HashMap;
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
    // Host form: x0 = host; x1 = port; x2 = timeoutMs; x3 = serverName; x4 = allowSelfSigned.
    // Address form: x0 = net::Address; x1 = timeoutMs; x2 = serverName; x3 = allowSelfSigned.
    ins.extend(
        crate::codegen::builtins::tls::gen_shared::connect_arg_prologue(
            address, &v9, HOST, PORT, TIMEOUT, SNAME, ALLOW,
        ),
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
        &mut ins,
        &mut rel,
        &mut vregs,
    );
    // Allocate the block context.
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", CTX_SIZE),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
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
    ]);
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
    // plan-73-D: a negative (non-sentinel) `timeoutMs` → ErrInvalidArgument. Reached
    // from the up-front check before any dlopen/alloc/connection, so no cleanup.
    ins.push(abi::label(&conn_invalid));
    emit_fail(symbol, "ErrInvalidArgument", &mut ins, &mut rel, &done);
    ins.push(abi::label(&load_fail));
    emit_fail(symbol, "ErrTlsFailed", &mut ins, &mut rel, &done);
    ins.push(abi::label(&alloc_fail));
    emit_fail(symbol, "ErrOutOfMemory", &mut ins, &mut rel, &done);
    ins.extend([abi::label(&done), abi::return_()]);
    {
        Ok((ins, rel, FRAME_SIZE))
    }
}

pub(crate) fn lower_tls_read_macos(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<TlsBodyParts, String> {
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    let v12 = vregs.next();
    let v13 = vregs.next();
    let v14 = vregs.next();
    let v15 = vregs.next();
    const FRAME_SIZE: usize = 192;
    const REC: usize = 8;
    const CONN: usize = 16;
    const CTX: usize = 24;
    const MAX: usize = 32;
    const HANDLE: usize = 40;
    const FNPTR: usize = 48;
    const MAPPED: usize = 64;
    const MPTR: usize = 72;
    const MSIZE: usize = 80;
    const N: usize = 88;
    const STR: usize = 96;
    const BLOCK: usize = 104; // 104..144
    const PBUF: usize = 144; // plan-76-B: scratch for the drain-armed arena copy
    const DEADLINE: usize = 152; // plan-110-D: computed dispatch_time for the read wait

    let closed = format!("{symbol}_closed");
    let invalid = format!("{symbol}_invalid");
    let peer_closed = format!("{symbol}_peer_closed");
    let load_fail = format!("{symbol}_load_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let entry_loop = format!("{symbol}_entry_loop");
    let entry_done = format!("{symbol}_entry_done");
    let done = format!("{symbol}_done");
    // plan-76-B Phase 4: drain a poll-armed receive into CTX_PEND, then serve any
    // buffered plaintext before falling back to a fresh receive.
    let check_pend = format!("{symbol}_check_pend");
    let drain_map = format!("{symbol}_drain_map");
    let drain_copy_loop = format!("{symbol}_drain_copy_loop");
    let drain_copy_done = format!("{symbol}_drain_copy_done");
    let drain_publish = format!("{symbol}_drain_publish");
    let serve_pending = format!("{symbol}_serve_pending");
    let build_pending_n = format!("{symbol}_build_pending_n");
    let build_start = format!("{symbol}_build_start");
    let recv_path = format!("{symbol}_recv_path");
    let served_pending = format!("{symbol}_served_pending");
    let result_ready = format!("{symbol}_result_ready");
    // plan-110-D: the read deadline installed by `tls::setReadTimeout`.
    let read_timeout = format!("{symbol}_read_timeout");
    let drain_wait = format!("{symbol}_drain_wait");

    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel = Vec::new();
    ins.extend([
        abi::store_u64(abi::c_arg(1), abi::stack_pointer(), MAX),
        abi::load_u64(&v9, abi::return_register(), REC_CLOSED),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&closed),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), REC),
        abi::load_u64(&v9, abi::return_register(), REC_CONN),
        abi::store_u64(&v9, abi::stack_pointer(), CONN),
        abi::load_u64(&v9, abi::return_register(), REC_CTX),
        abi::store_u64(&v9, abi::stack_pointer(), CTX),
        abi::load_u64(&v10, abi::stack_pointer(), MAX),
        abi::compare_immediate(&v10, "0"),
        abi::branch_le(&invalid),
    ]);
    emit_dlopen_maclib(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        &load_fail,
    )?;
    // plan-76-B Phase 4: if a poll receive is outstanding (armed), consume it into
    // CTX_PEND first; then serve any buffered plaintext before posting a fresh
    // receive. When nothing is armed and CTX_PEND is empty (the no-poll case), this
    // falls straight through to `recv_path` — the existing behaviour, byte-for-byte.
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::load_u64(&v10, &v9, CTX_ARMED),
        abi::compare_immediate(&v10, "0"),
        abi::branch_eq(&check_pend),
        abi::label(&drain_wait),
    ]);
    // plan-110-D: bounded by the socket's read deadline (CTX_RTO, unbounded
    // sentinel until `tls::setReadTimeout` installs one — so an unconfigured
    // socket still waits forever). On expiry the receive stays ARMED and its
    // completion block keeps its claim on CTX_PSEM/CTX_PCONTENT, so the NEXT
    // read consumes that same receive rather than posting a second one. That is
    // what makes a timed-out read resumable instead of leaking an outstanding
    // receive.
    emit_wait_bounded(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        CTX,
        FNPTR,
        DEADLINE,
        CTX_PSEM,
        CTX_RTO,
        "rd",
        &read_timeout,
        &load_fail,
        &mut vregs,
    )?;
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::store_u64(abi::ZERO, &v9, CTX_ARMED),
        abi::load_u64(&v10, &v9, CTX_PCONTENT),
        abi::compare_immediate(&v10, "0"),
        abi::branch_eq(&peer_closed), // armed receive returned EOF
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
        "dispatch_data_create_map",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::label(&drain_map),
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::load_u64(abi::return_register(), &v9, CTX_PCONTENT),
        abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), MPTR),
        abi::add_immediate(abi::c_arg(2), abi::stack_pointer(), MSIZE),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), MAPPED),
        abi::load_u64(&v9, abi::stack_pointer(), MSIZE),
        abi::store_u64(&v9, abi::stack_pointer(), N),
        abi::move_register(abi::return_register(), &v9),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.extend([
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), PBUF),
        abi::load_u64(&v9, abi::stack_pointer(), MPTR),
        abi::load_u64(&v10, abi::stack_pointer(), PBUF),
        abi::load_u64(&v11, abi::stack_pointer(), N),
        abi::move_immediate(&v12, "Integer", "0"),
        abi::label(&drain_copy_loop),
        abi::compare_registers(&v12, &v11),
        abi::branch_eq(&drain_copy_done),
        abi::load_u8(&v13, &v9, 0),
        abi::store_u8(&v13, &v10, 0),
        abi::add_immediate(&v9, &v9, 1),
        abi::add_immediate(&v10, &v10, 1),
        abi::add_immediate(&v12, &v12, 1),
        abi::branch(&drain_copy_loop),
        abi::label(&drain_copy_done),
        abi::label(&drain_publish),
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::load_u64(&v10, abi::stack_pointer(), PBUF),
        abi::store_u64(&v10, &v9, CTX_PEND_BUF),
        abi::load_u64(&v10, abi::stack_pointer(), N),
        abi::store_u64(&v10, &v9, CTX_PEND_LEN),
        abi::store_u64(abi::ZERO, &v9, CTX_PEND_OFF),
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
        "dispatch_release",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), MAPPED),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::load_u64(abi::return_register(), &v9, CTX_PCONTENT),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::store_u64(abi::ZERO, &v9, CTX_PCONTENT),
        // Serve the buffered plaintext (or fall to the normal receive path).
        abi::label(&check_pend),
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::load_u64(&v10, &v9, CTX_PEND_OFF),
        abi::load_u64(&v11, &v9, CTX_PEND_LEN),
        abi::compare_registers(&v10, &v11),
        abi::branch_ge(&recv_path),
        abi::label(&serve_pending),
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::load_u64(&v10, &v9, CTX_PEND_OFF),
        abi::load_u64(&v11, &v9, CTX_PEND_LEN),
        abi::subtract_registers(&v11, &v11, &v10), // avail = len - off
        abi::load_u64(&v12, abi::stack_pointer(), MAX),
        abi::compare_registers(&v11, &v12),
        abi::branch_le(&build_pending_n),
        abi::move_register(&v11, &v12), // n = min(avail, maxBytes)
        abi::label(&build_pending_n),
        abi::store_u64(&v11, abi::stack_pointer(), N),
        abi::load_u64(&v12, &v9, CTX_PEND_BUF),
        abi::add_registers(&v12, &v12, &v10),
        abi::store_u64(&v12, abi::stack_pointer(), MPTR),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), MAPPED),
        abi::branch(&build_start),
        abi::label(&recv_path),
    ]);
    // No `emit_fresh_sem` here any more. Read waits on CTX_PSEM now, so
    // recycling CTX_SEM would do nothing for it — and would be actively wrong:
    // `tls::write` may have a send outstanding against the current CTX_SEM
    // (CTX_WARMED), and replacing it underneath that send is the exact
    // stale-semaphore hazard the fresh-sem invariant exists to prevent.
    // ctx->retain = &dispatch_retain (used inside the receive block).
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "dispatch_retain",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(&v10, abi::stack_pointer(), FNPTR),
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::store_u64(&v10, &v9, CTX_RETAIN),
    ]);
    emit_build_block(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        // plan-110-D: the fresh receive now uses the POLL block, not RECV_INVOKE.
        // Both blocks post the same `nw_connection_receive`; the difference is
        // where the completion lands — RECV_POLL_INVOKE stashes into
        // CTX_PCONTENT and signals CTX_PSEM, which is the pair that survives
        // across calls. That matters once a read can time out: the posted
        // receive cannot be cancelled, so its completion must have somewhere to
        // land that the NEXT read will look at. Using one outstanding-receive
        // model for both read and poll also collapses two drains into one.
        RECV_POLL_INVOKE,
        CTX,
        BLOCK,
        FNPTR,
        &load_fail,
        &mut vregs,
    )?;
    // bug-386: if the connection has already transitioned to a terminal state
    // (failed=4 / cancelled=5), the async receive we are about to post will have
    // its completion block dropped by Network.framework, and — since no further
    // state transition will fire the state-changed handler — nothing would ever
    // signal the semaphore, hanging the wait below. Route to peer-closed (EOF),
    // matching how this path already surfaces a receive error. A transition that
    // happens DURING the wait still fires the state handler, so the pre-check
    // plus the handler close the hang on both sides of the race. CTX_STATE is
    // reset to 0 (invalid, < 4) at ctx setup and only the state handler raises
    // it, so a live/ready connection is never short-circuited.
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::load_u32(&v10, &v9, CTX_STATE),
        abi::compare_immediate(&v10, "4"),
        abi::branch_ge(&peer_closed),
    ]);
    // nw_connection_receive(conn, min=1, max=maxBytes, &block)
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "nw_connection_receive",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CONN),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), MAX),
        abi::add_immediate(abi::c_arg(3), abi::stack_pointer(), BLOCK),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        // Mark it outstanding and join the single bounded drain, which owns the
        // wait, the deadline, and the CTX_PCONTENT handling for both the
        // just-posted receive and one a previous `tls::poll` armed.
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::move_immediate(&v10, "Integer", "1"),
        abi::store_u64(&v10, &v9, CTX_ARMED),
        abi::branch(&drain_wait),
    ]);
    // plan-110-D: every read now reaches here from the serve-from-CTX_PEND path
    // with MPTR/N set and MAPPED == 0. The fresh-receive path used to arrive here
    // with a live `dispatch_data` map instead; it now arms the receive and joins
    // the drain, which does its own mapping and releases before publishing into
    // CTX_PEND. One shape reaches the build, so there is one release policy.
    ins.push(abi::label(&build_start));
    ins.extend([
        abi::load_u64(&v10, abi::stack_pointer(), N),
        abi::move_immediate(&v11, "Integer", &byte_list_entry_stride().to_string()),
        abi::multiply_registers(&v12, &v10, &v11),
        abi::add_immediate(&v12, &v12, COLLECTION_HEADER_SIZE),
        abi::add_registers(abi::return_register(), &v12, &v10),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.extend([
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), STR),
        abi::move_immediate(&v9, "Byte", &byte_list_block_kind().to_string()),
        abi::store_u8(&v9, abi::mfb_return(1), COLLECTION_OFFSET_KIND),
        abi::move_immediate(&v9, "Byte", &COLLECTION_TYPE_NONE.to_string()),
        abi::store_u8(&v9, abi::mfb_return(1), COLLECTION_OFFSET_KEY_TYPE),
        abi::move_immediate(&v9, "Byte", &COLLECTION_TYPE_BYTE.to_string()),
        abi::store_u8(&v9, abi::mfb_return(1), COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate(&v9, "Byte", "1"),
        abi::store_u8(&v9, abi::mfb_return(1), COLLECTION_OFFSET_FLAGS_VERSION),
        abi::load_u64(&v10, abi::stack_pointer(), N),
        abi::store_u64(&v10, abi::mfb_return(1), COLLECTION_OFFSET_COUNT),
        abi::store_u64(&v10, abi::mfb_return(1), COLLECTION_OFFSET_CAPACITY),
        abi::store_u64(&v10, abi::mfb_return(1), COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64(&v10, abi::mfb_return(1), COLLECTION_OFFSET_DATA_CAPACITY),
        abi::add_immediate(&v11, abi::mfb_return(1), COLLECTION_HEADER_SIZE),
        abi::move_immediate(&v12, "Integer", &byte_list_entry_stride().to_string()),
        abi::multiply_registers(&v13, &v10, &v12),
        abi::add_registers(&v14, &v11, &v13),
        abi::load_u64(&v15, abi::stack_pointer(), MPTR),
        abi::move_immediate(&v9, "Integer", "0"),
        abi::label(&entry_loop),
        abi::compare_registers(&v9, &v10),
        abi::branch_eq(&entry_done),
        // kind 2 has no entry array to fill (plan-57-D). Emitting this with a
        // zero stride would rewrite one entry over the data region `count`
        // times and run past the block, so it is skipped outright.
    ]);
    if byte_list_entry_stride() != 0 {
        ins.extend([
            abi::move_immediate(&v12, "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
            abi::store_u8(&v12, &v11, COLLECTION_ENTRY_OFFSET_FLAGS),
            abi::store_u64(abi::ZERO, &v11, COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
            abi::store_u64(abi::ZERO, &v11, COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
            abi::store_u64(&v9, &v11, COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
            abi::move_immediate(&v12, "Integer", "1"),
            abi::store_u64(&v12, &v11, COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        ]);
    }
    // The payload copy runs for BOTH representations.
    ins.extend([
        abi::add_registers(&v12, &v14, &v9),
        abi::load_u8(&v13, &v15, 0),
        abi::store_u8(&v13, &v12, 0),
        abi::add_immediate(&v15, &v15, 1),
        abi::add_immediate(&v11, &v11, byte_list_entry_stride()),
        abi::add_immediate(&v9, &v9, 1),
        abi::branch(&entry_loop),
        abi::label(&entry_done),
    ]);

    // Served from CTX_PEND: advance the consume cursor; free + clear the buffer
    // when fully drained so a later poll never overwrites (and leaks) it. There
    // is no NW object to release here — the drain already released the map and
    // the retained content before copying into CTX_PEND.
    ins.extend([
        abi::label(&served_pending),
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::load_u64(&v10, &v9, CTX_PEND_OFF),
        abi::load_u64(&v11, abi::stack_pointer(), N),
        abi::add_registers(&v10, &v10, &v11),
        abi::store_u64(&v10, &v9, CTX_PEND_OFF),
        abi::load_u64(&v11, &v9, CTX_PEND_LEN),
        abi::compare_registers(&v10, &v11),
        abi::branch_lt(&result_ready),
        abi::load_u64(abi::return_register(), &v9, CTX_PEND_BUF),
        abi::load_u64(abi::c_arg(1), &v9, CTX_PEND_LEN),
    ]);
    emit_arena_free(symbol, &mut ins, &mut rel);
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::store_u64(abi::ZERO, &v9, CTX_PEND_BUF),
        abi::store_u64(abi::ZERO, &v9, CTX_PEND_LEN),
        abi::store_u64(abi::ZERO, &v9, CTX_PEND_OFF),
        abi::label(&result_ready),
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), STR),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);

    ins.push(abi::label(&peer_closed));
    emit_fail(symbol, "ErrConnectionClosed", &mut ins, &mut rel, &done);
    ins.push(abi::label(&invalid));
    emit_fail(symbol, "ErrInvalidArgument", &mut ins, &mut rel, &done);
    // plan-110-D: the read deadline elapsed. CTX_ARMED is deliberately still 1
    // and CTX_PCONTENT untouched -- the receive is still outstanding and the
    // next read picks it up. Nothing is released here: releasing the content or
    // clearing ARMED would strand the completion block.
    ins.push(abi::label(&read_timeout));
    emit_fail(symbol, "ErrTimeout", &mut ins, &mut rel, &done);
    ins.push(abi::label(&load_fail));
    emit_fail(symbol, "ErrTlsFailed", &mut ins, &mut rel, &done);
    ins.push(abi::label(&closed));
    emit_fail(symbol, "ErrResourceClosed", &mut ins, &mut rel, &done);
    ins.push(abi::label(&alloc_fail));
    emit_fail(symbol, "ErrOutOfMemory", &mut ins, &mut rel, &done);
    ins.extend([abi::label(&done), abi::return_()]);
    {
        Ok((ins, rel, FRAME_SIZE))
    }
}

pub(crate) fn lower_tls_write_macos(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    text: bool,
) -> Result<TlsBodyParts, String> {
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    let v14 = vregs.next();
    let v12 = vregs.next();
    let v13 = vregs.next();
    const FRAME_SIZE: usize = 160;
    const REC: usize = 8;
    const CONN: usize = 16;
    const CTX: usize = 24;
    const HANDLE: usize = 32;
    const FNPTR: usize = 40;
    const CONTENT: usize = 48;
    const DATA: usize = 56;
    const DLEN: usize = 64;
    const CTXDEF: usize = 72;
    const BLOCK: usize = 80; // 80..120
    const DEADLINE: usize = 120; // plan-110-D: computed dispatch_time for the send wait

    let closed = format!("{symbol}_closed");
    let write_fail = format!("{symbol}_write_fail");
    let load_fail = format!("{symbol}_load_fail");
    let empty = format!("{symbol}_empty");
    let done = format!("{symbol}_done");
    // plan-110-D: the write deadline installed by `tls::setWriteTimeout`.
    let write_timeout = format!("{symbol}_write_timeout");
    let no_pending_send = format!("{symbol}_no_pending_send");

    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel = Vec::new();
    ins.extend([
        abi::load_u64(&v9, abi::return_register(), REC_CLOSED),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&closed),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), REC),
        abi::load_u64(&v9, abi::return_register(), REC_CONN),
        abi::store_u64(&v9, abi::stack_pointer(), CONN),
        abi::load_u64(&v9, abi::return_register(), REC_CTX),
        abi::store_u64(&v9, abi::stack_pointer(), CTX),
    ]);
    // bug-497 / bug-508: one payload view for every backend — the text form
    // as before, the byte form after a header check (`push_write_payload_view`).
    let bad_payload = format!("{symbol}_bad_payload");
    push_write_payload_view(
        &mut ins,
        text,
        abi::c_arg(1),
        &v10,
        &v11,
        &v14,
        &v12,
        &v13,
        DLEN,
        DATA,
        &bad_payload,
    );
    // Empty payload: nothing to send.
    ins.extend([
        abi::load_u64(&v10, abi::stack_pointer(), DLEN),
        abi::compare_immediate(&v10, "0"),
        abi::branch_eq(&empty),
    ]);
    emit_dlopen_maclib(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        &load_fail,
    )?;
    // plan-110-D: a send whose deadline elapsed is still outstanding — its
    // completion block holds the CTX_SEM that was current when it was posted, and
    // `nw_connection_send` has no cancel. Drain it before `emit_fresh_sem`
    // replaces that semaphore, or the completion would signal an object nothing
    // waits on and this send would consume a stale count (the bug-52/55 hazard
    // the fresh-sem invariant exists to prevent). Bounded by the same deadline,
    // so a peer that never drains cannot wedge the next write either.
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::load_u64(&v10, &v9, CTX_WARMED),
        abi::compare_immediate(&v10, "0"),
        abi::branch_eq(&no_pending_send),
    ]);
    emit_wait_bounded(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        CTX,
        FNPTR,
        DEADLINE,
        CTX_SEM,
        CTX_WTO,
        "wrdrain",
        &write_timeout,
        &load_fail,
        &mut vregs,
    )?;
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::store_u64(abi::ZERO, &v9, CTX_WARMED),
        abi::label(&no_pending_send),
    ]);
    emit_fresh_sem(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        CTX,
        FNPTR,
        &load_fail,
        &mut vregs,
    )?;
    // content = dispatch_data_create(data, len, NULL, NULL)  (NULL = copy)
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "dispatch_data_create",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), DATA),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), DLEN),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
        abi::move_immediate(abi::c_arg(3), "Integer", "0"),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), CONTENT),
    ]);
    // ctxdef = *_nw_content_context_default_message
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "_nw_content_context_default_message",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::load_u64(&v9, &v9, 0),
        abi::store_u64(&v9, abi::stack_pointer(), CTXDEF),
    ]);
    emit_build_block(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        SEND_INVOKE,
        CTX,
        BLOCK,
        FNPTR,
        &load_fail,
        &mut vregs,
    )?;
    // bug-386: skip the send + FOREVER wait if the connection is already in a
    // terminal state (failed=4 / cancelled=5). The send completion would be
    // dropped and, with no further state transition to fire the state-changed
    // handler, the wait would hang. Route to write-fail (ErrTlsFailed) — a write
    // to a dead connection is an error, not success. See the read path for the
    // full rationale (the state handler still covers a mid-wait transition).
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::load_u32(&v10, &v9, CTX_STATE),
        abi::compare_immediate(&v10, "4"),
        abi::branch_ge(&write_fail),
    ]);
    // nw_connection_send(conn, content, context, is_complete=true, &block)
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "nw_connection_send",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CONN),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), CONTENT),
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), CTXDEF),
        abi::move_immediate(abi::c_arg(3), "Integer", "1"),
        abi::add_immediate(abi::c_arg(4), abi::stack_pointer(), BLOCK),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        // Outstanding until its completion signals; see the drain above.
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::move_immediate(&v10, "Integer", "1"),
        abi::store_u64(&v10, &v9, CTX_WARMED),
    ]);
    emit_wait_bounded(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        CTX,
        FNPTR,
        DEADLINE,
        CTX_SEM,
        CTX_WTO,
        "wr",
        &write_timeout,
        &load_fail,
        &mut vregs,
    )?;
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::store_u64(abi::ZERO, &v9, CTX_WARMED),
    ]);
    // Release the content we created.
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "dispatch_release",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CONTENT),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        // A non-null error means the send failed.
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::load_u64(&v10, &v9, CTX_ERROR),
        abi::compare_immediate(&v10, "0"),
        abi::branch_ne(&write_fail),
        abi::label(&empty),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    // plan-110-D: the send deadline elapsed. CTX_WARMED stays 1 and CTX_SEM is
    // left alone — the send is still in flight and the next write drains it.
    // The content is deliberately NOT released here: `nw_connection_send` still
    // owns a reference until its completion runs.
    ins.push(abi::label(&write_timeout));
    emit_fail(symbol, "ErrTimeout", &mut ins, &mut rel, &done);
    ins.push(abi::label(&write_fail));
    emit_fail(symbol, "ErrTlsFailed", &mut ins, &mut rel, &done);
    ins.push(abi::label(&load_fail));
    emit_fail(symbol, "ErrTlsFailed", &mut ins, &mut rel, &done);
    ins.push(abi::label(&closed));
    emit_fail(symbol, "ErrResourceClosed", &mut ins, &mut rel, &done);
    if !text {
        // bug-497: the byte form was handed a block whose header is not a
        // `List OF Byte`'s — refuse rather than read a length out of its bytes.
        ins.push(abi::label(&bad_payload));
        emit_fail(symbol, "ErrInvalidArgument", &mut ins, &mut rel, &done);
    }
    ins.extend([abi::label(&done), abi::return_()]);
    {
        Ok((ins, rel, FRAME_SIZE))
    }
}

/// plan-76-B Phase 4: `tls::poll(sock[, timeoutMs]) AS Boolean` on macOS
/// Network.framework, via the outstanding-receive model (Corrections
/// B-macos-blocker / B-macos-blocker-2). NW has no non-blocking data-readiness
/// query and a posted receive cannot be cancelled, so readiness is driven by an
/// ISOLATED poll receive: it posts one `nw_connection_receive` whose completion
/// (RECV_POLL_INVOKE) stashes into `CTX_PCONTENT` and signals the dedicated
/// `CTX_PSEM` — never the read/write `CTX_SEM` — and on completion this helper
/// copies the mapped bytes into a persistent arena buffer (`CTX_PEND_*`) that
/// `tls::read` drains first. A bounded/zero-timeout poll that expires leaves the
/// receive outstanding (`CTX_ARMED`) so its bytes are stashed by the next
/// poll/read rather than lost. Readable = buffered plaintext present, OR the
/// outstanding receive has completed with bytes, OR the connection is terminal
/// (EOF/error). `x0` = sock record, `x1` = timeoutMs.
pub(crate) fn lower_tls_poll_macos(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<TlsBodyParts, String> {
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    let v12 = vregs.next();
    let v13 = vregs.next();
    const FRAME_SIZE: usize = 160;
    const REC: usize = 8;
    const TIMEOUT: usize = 16;
    const CONN: usize = 24;
    const CTX: usize = 32;
    const HANDLE: usize = 40;
    const FNPTR: usize = 48;
    const DEADLINE: usize = 56;
    const MPTR: usize = 64;
    const MSIZE: usize = 72;
    const MAPPED: usize = 80;
    const N: usize = 88;
    const PBUF: usize = 96;
    const BLOCK: usize = 104; // 104..144

    let closed = format!("{symbol}_closed");
    let invalid = format!("{symbol}_invalid");
    let load_fail = format!("{symbol}_load_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let ready = format!("{symbol}_ready");
    let not_ready = format!("{symbol}_not_ready");
    let skip_neg = format!("{symbol}_skip_neg");
    let do_wait = format!("{symbol}_do_wait");
    let wait_forever = format!("{symbol}_wait_forever");
    let wait_now = format!("{symbol}_wait_now");
    let deadline_ready = format!("{symbol}_deadline_ready");
    let stash = format!("{symbol}_stash");
    let stash_release = format!("{symbol}_stash_release");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let done = format!("{symbol}_done");

    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel = Vec::new();
    ins.extend([
        abi::store_u64(abi::c_arg(1), abi::stack_pointer(), TIMEOUT),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), REC),
        abi::load_u64(&v9, abi::return_register(), REC_CLOSED),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&closed),
        abi::load_u64(&v9, abi::return_register(), REC_CONN),
        abi::store_u64(&v9, abi::stack_pointer(), CONN),
        abi::load_u64(&v9, abi::return_register(), REC_CTX),
        abi::store_u64(&v9, abi::stack_pointer(), CTX),
        // Reject a genuine negative timeout (the unbounded sentinel is allowed).
        abi::load_u64(&v9, abi::stack_pointer(), TIMEOUT),
        abi::move_immediate(&v10, "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
        abi::compare_registers(&v9, &v10),
        abi::branch_eq(&skip_neg),
        abi::compare_immediate(&v9, "0"),
        abi::branch_lt(&invalid),
        abi::label(&skip_neg),
        // Fast path: undelivered buffered plaintext already present → readable now.
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::load_u64(&v10, &v9, CTX_PEND_OFF),
        abi::load_u64(&v11, &v9, CTX_PEND_LEN),
        abi::compare_registers(&v10, &v11),
        abi::branch_lt(&ready),
    ]);
    emit_dlopen_maclib(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        &load_fail,
    )?;
    ins.extend([
        // A receive already outstanding (a prior bounded poll timed out) → just wait
        // on it; do not post another.
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::load_u64(&v10, &v9, CTX_ARMED),
        abi::compare_immediate(&v10, "0"),
        abi::branch_ne(&do_wait),
        // Terminal connection (failed=4 / cancelled=5) → readable (read returns EOF).
        abi::load_u32(&v10, &v9, CTX_STATE),
        abi::compare_immediate(&v10, "4"),
        abi::branch_ge(&ready),
    ]);
    // ctx->retain = &dispatch_retain (the poll block retains its content).
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "dispatch_retain",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(&v10, abi::stack_pointer(), FNPTR),
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::store_u64(&v10, &v9, CTX_RETAIN),
        // Reset the poll receive's output slots before posting.
        abi::store_u64(abi::ZERO, &v9, CTX_PCONTENT),
        abi::store_u64(abi::ZERO, &v9, CTX_PERROR),
    ]);
    emit_build_block(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        RECV_POLL_INVOKE,
        CTX,
        BLOCK,
        FNPTR,
        &load_fail,
        &mut vregs,
    )?;
    ins.extend([
        // bug-386 style pre-check: a terminal connection would drop the receive's
        // completion, so route to readable (EOF) rather than arm a receive that
        // never signals.
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::load_u32(&v10, &v9, CTX_STATE),
        abi::compare_immediate(&v10, "4"),
        abi::branch_ge(&ready),
    ]);
    // nw_connection_receive(conn, min=1, max=65536, &block); mark armed.
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "nw_connection_receive",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CONN),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
        abi::move_immediate(abi::c_arg(2), "Integer", "65536"),
        abi::add_immediate(abi::c_arg(3), abi::stack_pointer(), BLOCK),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::move_immediate(&v10, "Integer", "1"),
        abi::store_u64(&v10, &v9, CTX_ARMED),
        // Compute the wait deadline (connect-path policy): sentinel→FOREVER, 0→NOW,
        // >0→dispatch_time(NOW, ms*1e6).
        abi::label(&do_wait),
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
        abi::multiply_registers(abi::c_arg(1), abi::c_arg(1), &v10),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), DEADLINE),
        abi::branch(&deadline_ready),
        abi::label(&wait_now),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), DEADLINE),
        abi::branch(&deadline_ready),
        abi::label(&wait_forever),
        abi::move_immediate(&v9, "Integer", "0"),
        abi::bitwise_not(&v9, &v9), // DISPATCH_TIME_FOREVER
        abi::store_u64(&v9, abi::stack_pointer(), DEADLINE),
        abi::label(&deadline_ready),
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
        "dispatch_semaphore_wait",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::load_u64(abi::return_register(), &v9, CTX_PSEM),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), DEADLINE),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        // Non-zero => the deadline elapsed; the receive stays outstanding (armed)
        // for the next poll/read to consume. Not ready.
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&not_ready),
        // Signaled: the poll receive completed. Clear armed, stash its content.
        abi::label(&stash),
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::store_u64(abi::ZERO, &v9, CTX_ARMED),
        abi::load_u64(&v10, &v9, CTX_PCONTENT),
        abi::compare_immediate(&v10, "0"),
        abi::branch_eq(&ready), // null content = EOF/terminal → readable
    ]);
    // dispatch_data_create_map(content, &ptr, &size) -> mapped
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "dispatch_data_create_map",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::load_u64(abi::return_register(), &v9, CTX_PCONTENT),
        abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), MPTR),
        abi::add_immediate(abi::c_arg(2), abi::stack_pointer(), MSIZE),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), MAPPED),
        abi::load_u64(&v9, abi::stack_pointer(), MSIZE),
        abi::store_u64(&v9, abi::stack_pointer(), N),
        abi::compare_immediate(&v9, "0"),
        abi::branch_eq(&stash_release), // 0-byte map: nothing to buffer
        // Copy the mapped bytes into a persistent arena buffer so no NW object is
        // held across the poll→read boundary.
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), N),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.extend([
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), PBUF),
        // memcpy(PBUF, MPTR, N)
        abi::load_u64(&v9, abi::stack_pointer(), MPTR),
        abi::load_u64(&v10, abi::stack_pointer(), PBUF),
        abi::load_u64(&v11, abi::stack_pointer(), N),
        abi::move_immediate(&v12, "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers(&v12, &v11),
        abi::branch_eq(&copy_done),
        abi::load_u8(&v13, &v9, 0),
        abi::store_u8(&v13, &v10, 0),
        abi::add_immediate(&v9, &v9, 1),
        abi::add_immediate(&v10, &v10, 1),
        abi::add_immediate(&v12, &v12, 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        // Publish the pending buffer.
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::load_u64(&v10, abi::stack_pointer(), PBUF),
        abi::store_u64(&v10, &v9, CTX_PEND_BUF),
        abi::load_u64(&v10, abi::stack_pointer(), N),
        abi::store_u64(&v10, &v9, CTX_PEND_LEN),
        abi::store_u64(abi::ZERO, &v9, CTX_PEND_OFF),
        abi::label(&stash_release),
    ]);
    // Release the mapped data and the retained content.
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "dispatch_release",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), MAPPED),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::load_u64(abi::return_register(), &v9, CTX_PCONTENT),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        abi::load_u64(&v9, abi::stack_pointer(), CTX),
        abi::store_u64(abi::ZERO, &v9, CTX_PCONTENT),
        abi::branch(&ready),
        abi::label(&ready),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&not_ready),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    ins.push(abi::label(&invalid));
    emit_fail(symbol, "ErrInvalidArgument", &mut ins, &mut rel, &done);
    ins.push(abi::label(&load_fail));
    emit_fail(symbol, "ErrTlsFailed", &mut ins, &mut rel, &done);
    ins.push(abi::label(&alloc_fail));
    emit_fail(symbol, "ErrOutOfMemory", &mut ins, &mut rel, &done);
    ins.push(abi::label(&closed));
    emit_fail(symbol, "ErrResourceClosed", &mut ins, &mut rel, &done);
    ins.extend([abi::label(&done), abi::return_()]);
    {
        Ok((ins, rel, FRAME_SIZE))
    }
}

pub(crate) fn lower_tls_close_macos(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<TlsBodyParts, String> {
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    const FRAME_SIZE: usize = 48;
    const REC: usize = 8;
    const HANDLE: usize = 16;
    const FNPTR: usize = 24;
    let already = format!("{symbol}_already");
    let load_fail = format!("{symbol}_load_fail");
    let done = format!("{symbol}_done");
    // plan-76-B Phase 4: free/release the poll-receive pending state on close.
    let skip_pfree = format!("{symbol}_skip_pfree");
    let skip_prelease = format!("{symbol}_skip_prelease");

    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel = Vec::new();
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), REC),
        abi::load_u64(&v9, abi::return_register(), REC_CLOSED),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&already),
    ]);
    emit_dlopen_maclib(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        &load_fail,
    )?;
    // nw_connection_cancel(conn)
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "nw_connection_cancel",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), REC),
        abi::load_u64(abi::return_register(), &v9, REC_CONN),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
    ]);
    // Release the connection, its dispatch queue, and the ctx semaphore that
    // this socket owns; cancelling alone leaves them all leaked on every
    // connect+close (bug-55). The arena-allocated ctx block is reclaimed with
    // the arena. Slots are never NULL for an open (non-closed) socket.
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
        abi::load_u64(&v9, abi::stack_pointer(), REC),
        abi::load_u64(abi::return_register(), &v9, REC_CONN),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
    ]);
    let skip_queue = format!("{symbol}_skip_queue_release");
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut ins,
            relocations: &mut rel,
        },
        HANDLE,
        "dispatch_release",
        FNPTR,
        &load_fail,
    )?;
    ins.extend([
        // Release the queue only if this socket owns it. A client socket stores
        // its own per-connection queue here; an accepted socket stores 0 because
        // it shares the listener's serial queue (released by closeListener), and
        // releasing that shared queue per accepted-close would over-release it.
        abi::load_u64(&v9, abi::stack_pointer(), REC),
        abi::load_u64(abi::return_register(), &v9, REC_QUEUE),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&skip_queue),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        abi::label(&skip_queue),
        // NB: ctx->sem is intentionally NOT released here. nw_connection_cancel
        // is asynchronous; the connection's state-changed handler still fires a
        // "cancelled" transition afterwards and does
        // dispatch_semaphore_signal(ctx->sem) — releasing the semaphore now
        // would make that a use-after-free. The single per-connection semaphore
        // is reclaimed with the arena-allocated ctx block (bug-55: the leaks
        // that scale — one per readText/write — are fixed in emit_fresh_sem).
        // plan-76-B Phase 4: free any buffered poll plaintext (arena) and release an
        // unconsumed poll receive's retained content. CTX_PSEM, like ctx->sem, is
        // NOT released (async cancel would race it); it is reclaimed with the arena
        // ctx block. FNPTR still holds dispatch_release from the queue release above.
        abi::load_u64(&v9, abi::stack_pointer(), REC),
        abi::load_u64(&v10, &v9, REC_CTX),
        abi::load_u64(abi::return_register(), &v10, CTX_PEND_BUF),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&skip_pfree),
        abi::load_u64(abi::c_arg(1), &v10, CTX_PEND_LEN),
    ]);
    emit_arena_free(symbol, &mut ins, &mut rel);
    ins.extend([
        abi::label(&skip_pfree),
        abi::load_u64(&v9, abi::stack_pointer(), REC),
        abi::load_u64(&v10, &v9, REC_CTX),
        abi::load_u64(abi::return_register(), &v10, CTX_PCONTENT),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&skip_prelease),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        abi::label(&skip_prelease),
        // Mark closed.
        abi::load_u64(&v9, abi::stack_pointer(), REC),
        abi::move_immediate(&v10, "Integer", "1"),
        abi::store_u64(&v10, &v9, REC_CLOSED),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    ins.push(abi::label(&load_fail));
    emit_fail(symbol, "ErrTlsFailed", &mut ins, &mut rel, &done);
    ins.extend([
        abi::label(&already),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::label(&done),
        abi::return_(),
    ]);
    {
        Ok((ins, rel, FRAME_SIZE))
    }
}
