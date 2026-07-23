use super::*;

pub(in crate::target::shared::code::tls) fn lower_tls_connect_macos(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    const FRAME_SIZE: usize = 288;
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
    const CFGBLOCK: usize = 200; // 200..264: the SNI-config block literal
    const TIMEOUT: usize = 264; // timeoutMs (arg x2)
    const DEADLINE: usize = 272; // dispatch_time deadline for the wait

    let wait_loop = format!("{symbol}_wait");
    let ready = format!("{symbol}_ready");
    let conn_fail = format!("{symbol}_conn_fail");
    let conn_timeout = format!("{symbol}_conn_timeout");
    let wait_forever = format!("{symbol}_wait_forever");
    let deadline_ready = format!("{symbol}_deadline_ready");
    let net_fail = format!("{symbol}_net_fail");
    let load_fail = format!("{symbol}_load_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let itoa_loop = format!("{symbol}_itoa");
    let sni_default = format!("{symbol}_sni_default");
    let done = format!("{symbol}_done");

    let mut ins = vec![abi::label("entry")];
    let mut rel = Vec::new();
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), HOST),
        abi::store_u64(abi::ARG[1], abi::stack_pointer(), PORT),
        abi::store_u64(abi::ARG[2], abi::stack_pointer(), TIMEOUT),
        abi::store_u64(abi::ARG[3], abi::stack_pointer(), SNAME),
    ]);
    // itoa(port) -> NUL-terminated decimal at PORTBUF, pointer in PORTCSTR.
    ins.extend([
        abi::move_immediate("%v9", "Integer", "0"),
        abi::store_u8("%v9", abi::stack_pointer(), PORTBUF + 23),
        abi::load_u64("%v10", abi::stack_pointer(), PORT),
        abi::move_immediate("%v11", "Integer", "10"),
        abi::add_immediate("%v14", abi::stack_pointer(), PORTBUF + 22),
        abi::label(&itoa_loop),
        abi::unsigned_divide_registers("%v15", "%v10", "%v11"),
        abi::multiply_subtract_registers("%v16", "%v15", "%v11", "%v10"),
        abi::add_immediate("%v16", "%v16", 48),
        abi::store_u8("%v16", "%v14", 0),
        abi::subtract_immediate("%v14", "%v14", 1),
        abi::move_register("%v10", "%v15"),
        abi::compare_immediate("%v10", "0"),
        abi::branch_ne(&itoa_loop),
        abi::add_immediate("%v13", "%v14", 1),
        abi::store_u64("%v13", abi::stack_pointer(), PORTCSTR),
    ]);
    // dlopen Network.framework.
    emit_data_address(
        symbol,
        abi::return_register(),
        MACLIB_SYMBOL,
        &mut ins,
        &mut rel,
    );
    ins.push(abi::move_immediate(abi::ARG[1], "Integer", RTLD_NOW));
    platform.emit_libc_call("dlopen", symbol, platform_imports, &mut ins, &mut rel)?;
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
    );
    // Allocate the block context.
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", CTX_SIZE),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.push(abi::store_u64(abi::RET[1], abi::stack_pointer(), CTX));
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
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), PORTCSTR),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&net_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), ENDPOINT),
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
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::load_u64("%v9", "%v9", 0),
        abi::store_u64("%v9", abi::stack_pointer(), CFG),
        // The configure-TLS block defaults to the system default. A non-empty
        // serverName swaps in a custom block that overrides the SNI /
        // certificate-validation name (empty => the endpoint host is used).
        abi::store_u64("%v9", abi::stack_pointer(), TLSCFG),
        abi::load_u64("%v9", abi::stack_pointer(), SNAME),
        abi::load_u64("%v10", "%v9", 0),
        abi::compare_immediate("%v10", "0"),
        abi::branch_eq(&sni_default),
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
    );
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
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::store_u64("%v9", abi::stack_pointer(), CFGBLOCK + BLK_ISA),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), CFGBLOCK + BLK_FLAGS),
    ]);
    emit_data_address(symbol, "%v9", CFG_INVOKE, &mut ins, &mut rel);
    ins.push(abi::store_u64(
        "%v9",
        abi::stack_pointer(),
        CFGBLOCK + BLK_INVOKE,
    ));
    emit_data_address(symbol, "%v9", CFG_DESC_SYMBOL, &mut ins, &mut rel);
    ins.push(abi::store_u64(
        "%v9",
        abi::stack_pointer(),
        CFGBLOCK + BLK_DESC,
    ));
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), SNICSTR),
        abi::store_u64("%v9", abi::stack_pointer(), CFGBLOCK + CFG_CAP_SNAME),
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
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::store_u64("%v9", abi::stack_pointer(), CFGBLOCK + CFG_CAP_COPYFN),
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
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::store_u64("%v9", abi::stack_pointer(), CFGBLOCK + CFG_CAP_SETFN),
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
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::store_u64("%v9", abi::stack_pointer(), CFGBLOCK + CFG_CAP_RELEASEFN),
        // tlscfg = &block
        abi::add_immediate("%v9", abi::stack_pointer(), CFGBLOCK),
        abi::store_u64("%v9", abi::stack_pointer(), TLSCFG),
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
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), CFG),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
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
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), PARAMS),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&net_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), CONN),
    ]);
    // nw_connection_create retains both the endpoint and the parameters, so
    // release our own references now; otherwise every successful connect leaks
    // one nw_endpoint and one nw_parameters (bug-55). The connection (CONN),
    // queue, and ctx are handed to the TlsSocket record and released on close.
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
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), PARAMS),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
    ]);
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
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), QUEUE),
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
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        abi::load_u64("%v9", abi::stack_pointer(), CTX),
        abi::store_u64(abi::return_register(), "%v9", CTX_SEM),
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
        abi::load_u64("%v10", abi::stack_pointer(), FNPTR),
        abi::load_u64("%v9", abi::stack_pointer(), CTX),
        abi::store_u64("%v10", "%v9", CTX_SIGNAL),
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
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), QUEUE),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
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
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::store_u64("%v9", abi::stack_pointer(), BLOCK + BLK_ISA),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), BLOCK + BLK_FLAGS),
    ]);
    emit_data_address(symbol, "%v9", STATE_INVOKE, &mut ins, &mut rel);
    ins.push(abi::store_u64(
        "%v9",
        abi::stack_pointer(),
        BLOCK + BLK_INVOKE,
    ));
    emit_data_address(symbol, "%v9", DESC_SYMBOL, &mut ins, &mut rel);
    ins.push(abi::store_u64(
        "%v9",
        abi::stack_pointer(),
        BLOCK + BLK_DESC,
    ));
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), CTX),
        abi::store_u64("%v9", abi::stack_pointer(), BLOCK + BLK_CAP),
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
        abi::add_immediate(abi::ARG[1], abi::stack_pointer(), BLOCK),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
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
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
    ]);
    // Compute the wait deadline: timeoutMs > 0 => dispatch_time(NOW, ms*1e6);
    // otherwise DISPATCH_TIME_FOREVER. It is absolute, so re-waits across the
    // preparing loop all share the original deadline.
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), TIMEOUT),
        abi::compare_immediate("%v9", "0"),
        abi::branch_le(&wait_forever),
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
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), TIMEOUT),
        abi::move_immediate("%v10", "Integer", "1000000"),
        abi::multiply_registers(abi::ARG[1], abi::ARG[1], "%v10"), // ms -> ns
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), DEADLINE),
        abi::branch(&deadline_ready),
        abi::label(&wait_forever),
        abi::move_immediate("%v9", "Integer", "0"),
        abi::bitwise_not("%v9", "%v9"), // DISPATCH_TIME_FOREVER
        abi::store_u64("%v9", abi::stack_pointer(), DEADLINE),
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
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::store_u64("%v9", abi::stack_pointer(), WAITFN),
        abi::label(&wait_loop),
        abi::load_u64("%v9", abi::stack_pointer(), CTX),
        abi::load_u64(abi::return_register(), "%v9", CTX_SEM),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), DEADLINE),
        abi::load_u64("%v10", abi::stack_pointer(), WAITFN),
        abi::branch_link_register("%v10"),
        // Non-zero => the deadline elapsed before any state change signalled.
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&conn_timeout),
        abi::load_u64("%v9", abi::stack_pointer(), CTX),
        abi::load_u32("%v10", "%v9", CTX_STATE),
        abi::compare_immediate("%v10", NW_STATE_READY),
        abi::branch_eq(&ready),
        abi::compare_immediate("%v10", "2"), // preparing
        abi::branch_eq(&wait_loop),
        abi::compare_immediate("%v10", "0"), // invalid
        abi::branch_eq(&wait_loop),
        abi::branch(&conn_fail), // waiting/failed/cancelled
        abi::label(&ready),
    ]);
    // Build the TlsSocket record { closed=0, conn, queue, ctx }.
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", REC_SIZE),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.extend([
        abi::store_u64(abi::ZERO, abi::RET[1], REC_CLOSED),
        abi::load_u64("%v9", abi::stack_pointer(), CONN),
        abi::store_u64("%v9", abi::RET[1], REC_CONN),
        abi::load_u64("%v9", abi::stack_pointer(), QUEUE),
        abi::store_u64("%v9", abi::RET[1], REC_QUEUE),
        abi::load_u64("%v9", abi::stack_pointer(), CTX),
        abi::store_u64("%v9", abi::RET[1], REC_CTX),
        abi::move_register(RESULT_VALUE_REGISTER, abi::RET[1]),
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
    )?;
    emit_fail(
        symbol,
        ERR_TLS_FAILED_CODE,
        ERR_TLS_FAILED_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
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
    )?;
    emit_fail(
        symbol,
        ERR_TIMEOUT_CODE,
        ERR_TIMEOUT_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&net_fail));
    emit_fail(
        symbol,
        ERR_NETWORK_FAILED_CODE,
        ERR_NETWORK_FAILED_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&load_fail));
    emit_fail(
        symbol,
        ERR_TLS_FAILED_CODE,
        ERR_TLS_FAILED_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&alloc_fail));
    emit_fail(
        symbol,
        ERR_OUT_OF_MEMORY_CODE,
        ERR_ALLOCATION_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.extend([abi::label(&done), abi::return_()]);
    {
        let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut ins, &[], FRAME_SIZE);
        Ok((frame, ins, rel, stack_slots))
    }
}

pub(in crate::target::shared::code::tls) fn lower_tls_read_macos(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    text: bool,
) -> HelperResult {
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

    let closed = format!("{symbol}_closed");
    let invalid = format!("{symbol}_invalid");
    let peer_closed = format!("{symbol}_peer_closed");
    let load_fail = format!("{symbol}_load_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let encoding_error = format!("{symbol}_encoding_error");
    let str_copy = format!("{symbol}_str_copy");
    let str_done = format!("{symbol}_str_done");
    let entry_loop = format!("{symbol}_entry_loop");
    let entry_done = format!("{symbol}_entry_done");
    let done = format!("{symbol}_done");

    let mut ins = vec![abi::label("entry")];
    let mut rel = Vec::new();
    ins.extend([
        abi::store_u64(abi::ARG[1], abi::stack_pointer(), MAX),
        abi::load_u64("%v9", abi::return_register(), REC_CLOSED),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&closed),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), REC),
        abi::load_u64("%v9", abi::return_register(), REC_CONN),
        abi::store_u64("%v9", abi::stack_pointer(), CONN),
        abi::load_u64("%v9", abi::return_register(), REC_CTX),
        abi::store_u64("%v9", abi::stack_pointer(), CTX),
        abi::load_u64("%v10", abi::stack_pointer(), MAX),
        abi::compare_immediate("%v10", "0"),
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
    )?;
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
        abi::load_u64("%v10", abi::stack_pointer(), FNPTR),
        abi::load_u64("%v9", abi::stack_pointer(), CTX),
        abi::store_u64("%v10", "%v9", CTX_RETAIN),
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
        RECV_INVOKE,
        CTX,
        BLOCK,
        FNPTR,
        &load_fail,
    )?;
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
        abi::move_immediate(abi::ARG[1], "Integer", "1"),
        abi::load_u64(abi::ARG[2], abi::stack_pointer(), MAX),
        abi::add_immediate(abi::ARG[3], abi::stack_pointer(), BLOCK),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
    ]);
    emit_wait(
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
    )?;
    // A null content is end-of-stream.
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), CTX),
        abi::load_u64("%v10", "%v9", CTX_CONTENT),
        abi::compare_immediate("%v10", "0"),
        abi::branch_eq(&peer_closed),
    ]);
    // dispatch_data_create_map(content, &ptr, &size) -> mapped (contiguous)
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
        abi::load_u64("%v9", abi::stack_pointer(), CTX),
        abi::load_u64(abi::return_register(), "%v9", CTX_CONTENT),
        abi::add_immediate(abi::ARG[1], abi::stack_pointer(), MPTR),
        abi::add_immediate(abi::ARG[2], abi::stack_pointer(), MSIZE),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), MAPPED),
        abi::load_u64("%v9", abi::stack_pointer(), MSIZE),
        abi::store_u64("%v9", abi::stack_pointer(), N),
    ]);
    if text {
        ins.extend([
            abi::load_u64("%v10", abi::stack_pointer(), N),
            abi::add_immediate(abi::return_register(), "%v10", 9),
            abi::move_immediate(abi::ARG[1], "Integer", "8"),
        ]);
        emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
        ins.extend([
            abi::load_u64("%v10", abi::stack_pointer(), N),
            abi::store_u64("%v10", abi::RET[1], 0),
            abi::load_u64("%v11", abi::stack_pointer(), MPTR),
            abi::add_immediate("%v12", abi::RET[1], 8),
            abi::move_immediate("%v13", "Integer", "0"),
            abi::store_u64(abi::RET[1], abi::stack_pointer(), STR),
            abi::label(&str_copy),
            abi::compare_registers("%v13", "%v10"),
            abi::branch_eq(&str_done),
            abi::load_u8("%v14", "%v11", 0),
            abi::store_u8("%v14", "%v12", 0),
            abi::add_immediate("%v11", "%v11", 1),
            abi::add_immediate("%v12", "%v12", 1),
            abi::add_immediate("%v13", "%v13", 1),
            abi::branch(&str_copy),
            abi::label(&str_done),
            abi::store_u8(abi::ZERO, "%v12", 0),
            abi::load_u64("%v9", abi::stack_pointer(), STR),
            abi::add_immediate(abi::return_register(), "%v9", 8),
            abi::load_u64(abi::ARG[1], "%v9", 0),
        ]);
        emit_call_validate_utf8(symbol, &encoding_error, &mut ins, &mut rel);
    } else {
        ins.extend([
            abi::load_u64("%v10", abi::stack_pointer(), N),
            abi::move_immediate("%v11", "Integer", &byte_list_entry_stride().to_string()),
            abi::multiply_registers("%v12", "%v10", "%v11"),
            abi::add_immediate("%v12", "%v12", COLLECTION_HEADER_SIZE),
            abi::add_registers(abi::return_register(), "%v12", "%v10"),
            abi::move_immediate(abi::ARG[1], "Integer", "8"),
        ]);
        emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
        ins.extend([
            abi::store_u64(abi::RET[1], abi::stack_pointer(), STR),
            abi::move_immediate("%v9", "Byte", &byte_list_block_kind().to_string()),
            abi::store_u8("%v9", abi::RET[1], COLLECTION_OFFSET_KIND),
            abi::move_immediate("%v9", "Byte", &COLLECTION_TYPE_NONE.to_string()),
            abi::store_u8("%v9", abi::RET[1], COLLECTION_OFFSET_KEY_TYPE),
            abi::move_immediate("%v9", "Byte", &COLLECTION_TYPE_BYTE.to_string()),
            abi::store_u8("%v9", abi::RET[1], COLLECTION_OFFSET_VALUE_TYPE),
            abi::move_immediate("%v9", "Byte", "1"),
            abi::store_u8("%v9", abi::RET[1], COLLECTION_OFFSET_FLAGS_VERSION),
            abi::load_u64("%v10", abi::stack_pointer(), N),
            abi::store_u64("%v10", abi::RET[1], COLLECTION_OFFSET_COUNT),
            abi::store_u64("%v10", abi::RET[1], COLLECTION_OFFSET_CAPACITY),
            abi::store_u64("%v10", abi::RET[1], COLLECTION_OFFSET_DATA_LENGTH),
            abi::store_u64("%v10", abi::RET[1], COLLECTION_OFFSET_DATA_CAPACITY),
            abi::add_immediate("%v11", abi::RET[1], COLLECTION_HEADER_SIZE),
            abi::move_immediate("%v12", "Integer", &byte_list_entry_stride().to_string()),
            abi::multiply_registers("%v13", "%v10", "%v12"),
            abi::add_registers("%v14", "%v11", "%v13"),
            abi::load_u64("%v15", abi::stack_pointer(), MPTR),
            abi::move_immediate("%v9", "Integer", "0"),
            abi::label(&entry_loop),
            abi::compare_registers("%v9", "%v10"),
            abi::branch_eq(&entry_done),
            // kind 2 has no entry array to fill (plan-57-D). Emitting this with a
            // zero stride would rewrite one entry over the data region `count`
            // times and run past the block, so it is skipped outright.
        ]);
        if byte_list_entry_stride() != 0 {
            ins.extend([
                abi::move_immediate("%v12", "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
                abi::store_u8("%v12", "%v11", COLLECTION_ENTRY_OFFSET_FLAGS),
                abi::store_u64(abi::ZERO, "%v11", COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
                abi::store_u64(abi::ZERO, "%v11", COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
                abi::store_u64("%v9", "%v11", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
                abi::move_immediate("%v12", "Integer", "1"),
                abi::store_u64("%v12", "%v11", COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
            ]);
        }
        // The payload copy runs for BOTH representations.
        ins.extend([
            abi::add_registers("%v12", "%v14", "%v9"),
            abi::load_u8("%v13", "%v15", 0),
            abi::store_u8("%v13", "%v12", 0),
            abi::add_immediate("%v15", "%v15", 1),
            abi::add_immediate("%v11", "%v11", byte_list_entry_stride()),
            abi::add_immediate("%v9", "%v9", 1),
            abi::branch(&entry_loop),
            abi::label(&entry_done),
        ]);
    }
    // Release the mapped data and the retained content, then return.
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
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CTX),
        abi::load_u64(abi::return_register(), abi::return_register(), CTX_CONTENT),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), STR),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    if text {
        // The encoding-error exit must release the mapped data and the retained
        // content before failing, exactly as the success path above does.
        // Otherwise a peer that keeps sending invalid UTF-8 to a program looping
        // on tls::readText drives an unbounded dispatch_data/content leak — a
        // remotely-triggerable memory-exhaustion DoS (bug-52). MAPPED, CTX and
        // CTX_CONTENT are reloaded from stack slots so no live value is held in
        // a caller-saved register across either dispatch_release `bl`.
        ins.push(abi::label(&encoding_error));
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
            abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
            abi::branch_link_register("%v9"),
            abi::load_u64(abi::return_register(), abi::stack_pointer(), CTX),
            abi::load_u64(abi::return_register(), abi::return_register(), CTX_CONTENT),
            abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
            abi::branch_link_register("%v9"),
        ]);
        emit_fail(
            symbol,
            ERR_ENCODING_CODE,
            ERR_ENCODING_SYMBOL,
            &mut ins,
            &mut rel,
            &done,
        );
    }
    ins.push(abi::label(&peer_closed));
    emit_fail(
        symbol,
        ERR_CONNECTION_CLOSED_CODE,
        ERR_CONNECTION_CLOSED_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&invalid));
    emit_fail(
        symbol,
        ERR_INVALID_ARGUMENT_CODE,
        ERR_INVALID_ARGUMENT_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&load_fail));
    emit_fail(
        symbol,
        ERR_TLS_FAILED_CODE,
        ERR_TLS_FAILED_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&closed));
    emit_fail(
        symbol,
        ERR_RESOURCE_CLOSED_CODE,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&alloc_fail));
    emit_fail(
        symbol,
        ERR_OUT_OF_MEMORY_CODE,
        ERR_ALLOCATION_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.extend([abi::label(&done), abi::return_()]);
    {
        let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut ins, &[], FRAME_SIZE);
        Ok((frame, ins, rel, stack_slots))
    }
}

pub(in crate::target::shared::code::tls) fn lower_tls_write_macos(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    text: bool,
) -> HelperResult {
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

    let closed = format!("{symbol}_closed");
    let write_fail = format!("{symbol}_write_fail");
    let load_fail = format!("{symbol}_load_fail");
    let empty = format!("{symbol}_empty");
    let done = format!("{symbol}_done");

    let mut ins = vec![abi::label("entry")];
    let mut rel = Vec::new();
    ins.extend([
        abi::load_u64("%v9", abi::return_register(), REC_CLOSED),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&closed),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), REC),
        abi::load_u64("%v9", abi::return_register(), REC_CONN),
        abi::store_u64("%v9", abi::stack_pointer(), CONN),
        abi::load_u64("%v9", abi::return_register(), REC_CTX),
        abi::store_u64("%v9", abi::stack_pointer(), CTX),
    ]);
    if text {
        ins.extend([
            abi::load_u64("%v10", abi::ARG[1], 0),
            abi::store_u64("%v10", abi::stack_pointer(), DLEN),
            abi::add_immediate("%v11", abi::ARG[1], 8),
            abi::store_u64("%v11", abi::stack_pointer(), DATA),
        ]);
    } else {
        ins.extend([
            abi::load_u64("%v10", abi::ARG[1], COLLECTION_OFFSET_COUNT),
            abi::store_u64("%v10", abi::stack_pointer(), DLEN),
            // The byte payload begins past the CAPACITY-sized entry array, not the
            // COUNT-sized one: an append-built list carries spare capacity, so
            // COUNT*ENTRY would mis-address it (byte payload base is
            // HEADER + CAPACITY*ENTRY). Mirrors the OpenSSL path (bug-157).
        ]);
        push_collection_data_base_from_capacity(
            &mut ins,
            "%v11",
            abi::ARG[1],
            "%v14",
            "%v12",
            "%v13",
        );
        ins.extend([abi::store_u64("%v11", abi::stack_pointer(), DATA)]);
    }
    // Empty payload: nothing to send.
    ins.extend([
        abi::load_u64("%v10", abi::stack_pointer(), DLEN),
        abi::compare_immediate("%v10", "0"),
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
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), DLEN),
        abi::move_immediate(abi::ARG[2], "Integer", "0"),
        abi::move_immediate(abi::ARG[3], "Integer", "0"),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
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
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::load_u64("%v9", "%v9", 0),
        abi::store_u64("%v9", abi::stack_pointer(), CTXDEF),
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
    )?;
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
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), CONTENT),
        abi::load_u64(abi::ARG[2], abi::stack_pointer(), CTXDEF),
        abi::move_immediate(abi::ARG[3], "Integer", "1"),
        abi::add_immediate(abi::ARG[4], abi::stack_pointer(), BLOCK),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
    ]);
    emit_wait(
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
    )?;
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
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        // A non-null error means the send failed.
        abi::load_u64("%v9", abi::stack_pointer(), CTX),
        abi::load_u64("%v10", "%v9", CTX_ERROR),
        abi::compare_immediate("%v10", "0"),
        abi::branch_ne(&write_fail),
        abi::label(&empty),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    ins.push(abi::label(&write_fail));
    emit_fail(
        symbol,
        ERR_TLS_FAILED_CODE,
        ERR_TLS_FAILED_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&load_fail));
    emit_fail(
        symbol,
        ERR_TLS_FAILED_CODE,
        ERR_TLS_FAILED_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&closed));
    emit_fail(
        symbol,
        ERR_RESOURCE_CLOSED_CODE,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.extend([abi::label(&done), abi::return_()]);
    {
        let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut ins, &[], FRAME_SIZE);
        Ok((frame, ins, rel, stack_slots))
    }
}

pub(in crate::target::shared::code::tls) fn lower_tls_close_macos(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    const FRAME_SIZE: usize = 48;
    const REC: usize = 8;
    const HANDLE: usize = 16;
    const FNPTR: usize = 24;
    let already = format!("{symbol}_already");
    let load_fail = format!("{symbol}_load_fail");
    let done = format!("{symbol}_done");

    let mut ins = vec![abi::label("entry")];
    let mut rel = Vec::new();
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), REC),
        abi::load_u64("%v9", abi::return_register(), REC_CLOSED),
        abi::compare_immediate("%v9", "0"),
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
        abi::load_u64("%v9", abi::stack_pointer(), REC),
        abi::load_u64(abi::return_register(), "%v9", REC_CONN),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
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
        abi::load_u64("%v9", abi::stack_pointer(), REC),
        abi::load_u64(abi::return_register(), "%v9", REC_CONN),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
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
        abi::load_u64("%v9", abi::stack_pointer(), REC),
        abi::load_u64(abi::return_register(), "%v9", REC_QUEUE),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&skip_queue),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        abi::label(&skip_queue),
        // NB: ctx->sem is intentionally NOT released here. nw_connection_cancel
        // is asynchronous; the connection's state-changed handler still fires a
        // "cancelled" transition afterwards and does
        // dispatch_semaphore_signal(ctx->sem) — releasing the semaphore now
        // would make that a use-after-free. The single per-connection semaphore
        // is reclaimed with the arena-allocated ctx block (bug-55: the leaks
        // that scale — one per readText/write — are fixed in emit_fresh_sem).
        // Mark closed.
        abi::load_u64("%v9", abi::stack_pointer(), REC),
        abi::move_immediate("%v10", "Integer", "1"),
        abi::store_u64("%v10", "%v9", REC_CLOSED),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    ins.push(abi::label(&load_fail));
    emit_fail(
        symbol,
        ERR_TLS_FAILED_CODE,
        ERR_TLS_FAILED_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.extend([
        abi::label(&already),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::label(&done),
        abi::return_(),
    ]);
    {
        let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut ins, &[], FRAME_SIZE);
        Ok((frame, ins, rel, stack_slots))
    }
}
