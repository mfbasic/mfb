//! plan-176-T: `lower_tls_listen_macos`'s listener-start phase (`emit_listen_start`). The carriers and frame consts live in
//! `builtins/tls/gen_macos/server/listen/mod.rs`.

use super::*;

/// `lower_tls_listen_macos` phase 3: the listener context, its handlers, `nw_listener_start`,
/// the wait for ready, and the `Listener` record.
pub(super) fn emit_listen_start(
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
    // Allocate + initialize the listener context (shared ctx prefix + ring).
    // On the C heap (bug-623 D, see `CTX_SIZE`).
    emit_ctx_calloc(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut *ins,
            relocations: &mut *rel,
        },
        LCTX_SIZE,
        &env.alloc_fail,
    )?;
    ins.push(abi::store_u64(
        abi::mfb_return(1),
        abi::stack_pointer(),
        LCTX,
    ));
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut *ins,
            relocations: &mut *rel,
        },
        NWH,
        "dispatch_semaphore_create",
        FNPTR,
        &env.load_fail,
    )?;
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
        abi::load_u64(&v9, abi::stack_pointer(), LCTX),
        abi::store_u64(abi::return_register(), &v9, CTX_SEM),
        abi::store_u64(abi::ZERO, &v9, CTX_STATE),
        abi::store_u64(abi::ZERO, &v9, CTX_CONTENT),
        abi::store_u64(abi::ZERO, &v9, CTX_ERROR),
        abi::store_u64(abi::ZERO, &v9, LCTX_HEAD),
        abi::store_u64(abi::ZERO, &v9, LCTX_TAIL),
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
        "dispatch_semaphore_signal",
        FNPTR,
        &env.load_fail,
    )?;
    ins.extend([
        abi::load_u64(&v10, abi::stack_pointer(), FNPTR),
        abi::load_u64(&v9, abi::stack_pointer(), LCTX),
        abi::store_u64(&v10, &v9, CTX_SIGNAL),
    ]);
    // ctx->retain = &nw_retain (the conn handler retains queued connections).
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut *ins,
            relocations: &mut *rel,
        },
        NWH,
        "nw_retain",
        FNPTR,
        &env.load_fail,
    )?;
    ins.extend([
        abi::load_u64(&v10, abi::stack_pointer(), FNPTR),
        abi::load_u64(&v9, abi::stack_pointer(), LCTX),
        abi::store_u64(&v10, &v9, CTX_RETAIN),
    ]);
    // bug-483: the listener shares STATE_INVOKE with connection contexts, so it
    // must carry the same error-domain pair — the trampoline writes them
    // unconditionally and an uninitialised CTX_EDOMFN would be called.
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut *ins,
            relocations: &mut *rel,
        },
        NWH,
        "nw_error_get_error_domain",
        FNPTR,
        &env.load_fail,
    )?;
    ins.extend([
        abi::load_u64(&v10, abi::stack_pointer(), FNPTR),
        abi::load_u64(&v9, abi::stack_pointer(), LCTX),
        abi::store_u64(&v10, &v9, CTX_EDOMFN),
        abi::store_u64(abi::ZERO, &v9, CTX_EDOM),
    ]);
    // nw_listener_set_queue(listener, queue)
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut *ins,
            relocations: &mut *rel,
        },
        NWH,
        "nw_listener_set_queue",
        FNPTR,
        &env.load_fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), LISTENER),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), QUEUE),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
    ]);
    // State-changed handler (the shared STATE_INVOKE trampoline over lctx).
    emit_build_block(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut *ins,
            relocations: &mut *rel,
        },
        NWH,
        STATE_INVOKE,
        LCTX,
        SBLOCK,
        FNPTR,
        &env.load_fail,
        &mut *vregs,
    )?;
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut *ins,
            relocations: &mut *rel,
        },
        NWH,
        "nw_listener_set_state_changed_handler",
        FNPTR,
        &env.load_fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), LISTENER),
        abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), SBLOCK),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
    ]);
    // New-connection handler (retain + enqueue + signal).
    emit_build_block(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut *ins,
            relocations: &mut *rel,
        },
        NWH,
        LCONN_INVOKE,
        LCTX,
        CBLOCK,
        FNPTR,
        &env.load_fail,
        &mut *vregs,
    )?;
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut *ins,
            relocations: &mut *rel,
        },
        NWH,
        "nw_listener_set_new_connection_handler",
        FNPTR,
        &env.load_fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), LISTENER),
        abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), CBLOCK),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
    ]);
    // nw_listener_start(listener)
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut *ins,
            relocations: &mut *rel,
        },
        NWH,
        "nw_listener_start",
        FNPTR,
        &env.load_fail,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), LISTENER),
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::branch_link_register(&v9),
    ]);
    // Wait until the listener is ready (bind complete) or failed.
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut *ins,
            relocations: &mut *rel,
        },
        NWH,
        "dispatch_semaphore_wait",
        FNPTR,
        &env.load_fail,
    )?;
    ins.extend([
        abi::load_u64(&v9, abi::stack_pointer(), FNPTR),
        abi::store_u64(&v9, abi::stack_pointer(), WAITFN),
        abi::label(&env.wait_loop),
        abi::load_u64(&v9, abi::stack_pointer(), LCTX),
        abi::load_u64(abi::return_register(), &v9, CTX_SEM),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        abi::bitwise_not(abi::c_arg(1), abi::c_arg(1)), // DISPATCH_TIME_FOREVER
        abi::load_u64(&v10, abi::stack_pointer(), WAITFN),
        abi::branch_link_register(&v10),
        abi::load_u64(&v9, abi::stack_pointer(), LCTX),
        abi::load_u32(&v10, &v9, CTX_STATE),
        abi::compare_immediate(&v10, NW_LISTENER_READY),
        abi::branch_eq(&env.ready),
        abi::compare_immediate(&v10, NW_LISTENER_FAILED),
        abi::branch_ge(&env.listen_fail), // failed / cancelled
        abi::branch(&env.wait_loop),      // invalid / waiting
        abi::label(&env.ready),
    ]);
    // Build the Listener record: header { tag, listener, closed=0, STATE=0 }
    // then the macOS tail { lctx, queue, bound host } (plan-80, bug-465).
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", REC_SIZE),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut *ins, &mut *rel, &env.alloc_fail);
    ins.extend([
        abi::move_immediate(&v9, "Integer", RESOURCE_TAG_TLS_LISTENER),
        abi::store_u64(&v9, abi::mfb_return(1), REC_TAG),
        // bug-755: a fresh record is unowned until a binding claims it.
        abi::store_u64(abi::ZERO, abi::mfb_return(1), RESOURCE_OFFSET_OWNER),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), REC_STATE),
        abi::store_u64(abi::ZERO, abi::mfb_return(1), REC_CLOSED),
        abi::load_u64(&v9, abi::stack_pointer(), LISTENER),
        abi::store_u64(&v9, abi::mfb_return(1), REC_CONN),
        abi::load_u64(&v9, abi::stack_pointer(), QUEUE),
        abi::store_u64(&v9, abi::mfb_return(1), REC_QUEUE),
        abi::load_u64(&v9, abi::stack_pointer(), LCTX),
        abi::store_u64(&v9, abi::mfb_return(1), REC_CTX),
        // bug-465: keep the host C string so `tls::localAddress(listener)` has
        // something to report. `nw_listener_get_port` answers the port and
        // nothing in Network.framework answers the address, so this is the only
        // record of what was bound. bug-984: the record OWNS this arena block from
        // here on and the listener's close frees it, so the scratch pointer is
        // nulled and the release at `done` leaves it alone.
        abi::load_u64(&v9, abi::stack_pointer(), HOSTCSTR),
        abi::store_u64(&v9, abi::mfb_return(1), REC_LHOST),
        abi::move_immediate(
            scratch.host_scratch_until_the_listener_owns_it.pointer,
            "Integer",
            "0",
        ),
        abi::move_register(RESULT_VALUE_REGISTER, abi::mfb_return(1)),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&env.done),
    ]);
    Ok(())
}
