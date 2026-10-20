//! plan-176-P: the stdin broadcast's subscriptions — recomputing the shared base, subscribe and unsubscribe. The log layout, `field_addr`, `emit_libc` and the
//! poll-ready checks live in `io/stdin/stdin_broadcast/mod.rs`.

use super::*;

/// `_mfb_rt_stdin_recompute_base` — recompute `base = min(cursor over active
/// subscribers)` (or `fill` if none) and free every block entirely before `base`.
/// Assumes the log mutex is held. No args, returns nothing.
pub(crate) fn lower_stdin_recompute_base(
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<CodeFunction, String> {
    let symbol = STDIN_RECOMPUTE_BASE_SYMBOL;
    let l = |s: &str| format!("{symbol}_{s}");
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    let mut vregs = Vregs::new();
    let v51 = vregs.next();
    let v56 = vregs.next();
    let v61 = vregs.next();
    let v67 = vregs.next();
    let v70 = vregs.next();
    let v72 = vregs.next();
    let v73 = vregs.next();
    let v74 = vregs.next();
    let v78 = vregs.next();
    let v80 = vregs.next();
    let v82 = vregs.next();
    push_log_address(symbol, &v78, &mut instructions, &mut relocations);
    instructions.extend([
        abi::load_u64(&v70, &v78, STDIN_LOG_FILL_OFFSET),
        abi::move_register(&v80, &v70),
        abi::add_immediate(&v67, &v78, STDIN_LOG_REGISTRY_OFFSET),
        abi::move_immediate(&v74, "Integer", "0"),
        abi::label(&l("reg_loop")),
        abi::compare_immediate(&v74, &STDIN_LOG_MAX_SUBSCRIBERS.to_string()),
        abi::branch_ge(&l("reg_done")),
        abi::load_u64(&v51, &v67, STDIN_SUBSCRIBER_ACTIVE_OFFSET),
        abi::compare_immediate(&v51, "0"),
        abi::branch_eq(&l("reg_next")),
        abi::load_u64(&v61, &v67, STDIN_SUBSCRIBER_CURSOR_OFFSET),
        // c >= min (unsigned) => not a new minimum (`min <= c`).
        abi::compare_registers(&v80, &v61),
        abi::branch_ls(&l("reg_next")),
        abi::move_register(&v80, &v61),
        abi::label(&l("reg_next")),
        abi::add_immediate(&v67, &v67, STDIN_SUBSCRIBER_ENTRY_SIZE),
        abi::add_immediate(&v74, &v74, 1),
        abi::branch(&l("reg_loop")),
        abi::label(&l("reg_done")),
        abi::store_u64(&v80, &v78, STDIN_LOG_BASE_OFFSET),
    ]);
    // Reclaim: while head != 0 and headEnd <= base, free head.
    instructions.push(abi::label(&l("reclaim")));
    push_log_address(symbol, &v78, &mut instructions, &mut relocations);
    instructions.extend([
        abi::load_u64(&v73, &v78, STDIN_LOG_HEAD_OFFSET),
        abi::compare_immediate(&v73, "0"),
        abi::branch_eq(&l("reclaim_done")),
        abi::load_u64(&v82, &v73, STDIN_BLOCK_NEXT_OFFSET),
        abi::compare_immediate(&v82, "0"),
        abi::branch_eq(&l("he_fill")),
        abi::load_u64(&v72, &v82, STDIN_BLOCK_BASE_OFFSET),
        abi::branch(&l("he_ok")),
        abi::label(&l("he_fill")),
        abi::load_u64(&v72, &v78, STDIN_LOG_FILL_OFFSET),
        abi::label(&l("he_ok")),
        abi::load_u64(&v56, &v78, STDIN_LOG_BASE_OFFSET),
        abi::compare_registers(&v72, &v56),
        abi::branch_hi(&l("reclaim_done")),
        // headEnd <= base: unlink and free head.
        abi::store_u64(&v82, &v78, STDIN_LOG_HEAD_OFFSET),
        abi::compare_immediate(&v82, "0"),
        abi::branch_ne(&l("has_next")),
        abi::store_u64(abi::ZERO, &v78, STDIN_LOG_TAIL_OFFSET),
        abi::label(&l("has_next")),
        abi::move_register(abi::c_arg(0), &v73),
    ]);
    emit_libc(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "free",
    )?;
    instructions.push(abi::branch(&l("reclaim")));
    instructions.push(abi::label(&l("reclaim_done")));
    instructions.push(abi::return_());
    Ok(finalize_vreg_helper(
        "runtime.stdin.recompute_base",
        symbol,
        "Nothing",
        instructions,
        relocations,
    ))
}

/// `_mfb_rt_stdin_subscribe` — lazily initialize the global log and subscribe the
/// target thread at the current frontier. `x0` = the arena-state pointer of the
/// thread to subscribe (the caller's own `x19` for the self form). Idempotent per
/// thread. Returns nothing.
pub(crate) fn lower_stdin_subscribe(
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<CodeFunction, String> {
    let symbol = STDIN_SUBSCRIBE_SYMBOL;
    let l = |s: &str| format!("{symbol}_{s}");
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    let mut vregs = Vregs::new();
    let v1 = vregs.next();
    let v51 = vregs.next();
    let v52 = vregs.next();
    let v53 = vregs.next();
    let v67 = vregs.next();
    let v70 = vregs.next();
    let v74 = vregs.next();
    let v75 = vregs.next();
    let v78 = vregs.next();
    let v79 = vregs.next();
    let v89 = vregs.next();
    instructions.push(abi::move_register(&v53, abi::c_arg(0)));
    // Lazy setup: init mutex/cond, eofOffset = U64_MAX, initialized = 1. Runs
    // single-threaded at main entry (the compat shim), so no init race in practice.
    push_log_address(symbol, &v78, &mut instructions, &mut relocations);
    instructions.extend([
        abi::load_u64(&v75, &v78, STDIN_LOG_INITIALIZED_OFFSET),
        abi::compare_immediate(&v75, "0"),
        abi::branch_ne(&l("already_init")),
        abi::move_register(abi::c_arg(0), &v78),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
    ]);
    emit_libc(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_mutex_init",
    )?;
    push_log_address(symbol, &v78, &mut instructions, &mut relocations);
    instructions.extend([
        abi::add_immediate(abi::c_arg(0), &v78, STDIN_LOG_CV_OFFSET),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
    ]);
    emit_libc(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_cond_init",
    )?;
    push_log_address(symbol, &v78, &mut instructions, &mut relocations);
    instructions.extend([
        abi::move_immediate(&v79, "Integer", U64_MAX_DECIMAL),
        abi::store_u64(&v79, &v78, STDIN_LOG_EOF_OFFSET),
        abi::move_immediate(&v1, "Integer", "1"),
        abi::store_u64(&v1, &v78, STDIN_LOG_INITIALIZED_OFFSET),
        abi::label(&l("already_init")),
    ]);
    // Lock, register the thread if not already subscribed.
    push_log_address(symbol, &v78, &mut instructions, &mut relocations);
    instructions.push(abi::move_register(abi::c_arg(0), &v78));
    emit_libc(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_mutex_lock",
    )?;
    push_log_address(symbol, &v78, &mut instructions, &mut relocations);
    field_addr(&v52, &v53, ARENA_STDIN_SUBSCRIBER_OFFSET, &mut instructions);
    instructions.extend([
        abi::load_u64(&v89, &v52, 0),
        abi::compare_immediate(&v89, "0"),
        abi::branch_ne(&l("unlock")),
        // Scan for a free registry slot.
        abi::add_immediate(&v67, &v78, STDIN_LOG_REGISTRY_OFFSET),
        abi::move_immediate(&v74, "Integer", "0"),
        abi::label(&l("find")),
        // Registry-full (bug-243): the subscriber table holds at most
        // STDIN_LOG_MAX_SUBSCRIBERS (128) concurrently-live subscribers. When full,
        // the calling thread is left unregistered (STDIN_SUBSCRIBER stays null), so
        // its later reads return ErrInvalidContext ("not
        // subscribed") — which is accurate (it genuinely is not subscribed). A
        // distinct capacity error would require making `thread::openStdIn` fallible
        // (it currently returns `Nothing`); the 128-subscriber cap is the documented
        // limit. Reaching it needs >128 concurrently-live threads each calling
        // openStdIn — far beyond normal use.
        abi::compare_immediate(&v74, &STDIN_LOG_MAX_SUBSCRIBERS.to_string()),
        abi::branch_ge(&l("unlock")),
        abi::load_u64(&v51, &v67, STDIN_SUBSCRIBER_ACTIVE_OFFSET),
        abi::compare_immediate(&v51, "0"),
        abi::branch_eq(&l("got_slot")),
        abi::add_immediate(&v67, &v67, STDIN_SUBSCRIBER_ENTRY_SIZE),
        abi::add_immediate(&v74, &v74, 1),
        abi::branch(&l("find")),
        abi::label(&l("got_slot")),
        abi::move_immediate(&v1, "Integer", "1"),
        abi::store_u64(&v1, &v67, STDIN_SUBSCRIBER_ACTIVE_OFFSET),
        abi::load_u64(&v70, &v78, STDIN_LOG_FILL_OFFSET),
        abi::store_u64(&v70, &v67, STDIN_SUBSCRIBER_CURSOR_OFFSET),
    ]);
    field_addr(&v52, &v53, ARENA_STDIN_SUBSCRIBER_OFFSET, &mut instructions);
    instructions.push(abi::store_u64(&v67, &v52, 0));
    instructions.push(abi::label(&l("unlock")));
    push_log_address(symbol, &v78, &mut instructions, &mut relocations);
    instructions.push(abi::move_register(abi::c_arg(0), &v78));
    emit_libc(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_mutex_unlock",
    )?;
    instructions.push(abi::return_());
    Ok(finalize_vreg_helper(
        "runtime.stdin.subscribe",
        symbol,
        "Nothing",
        instructions,
        relocations,
    ))
}

/// `_mfb_rt_stdin_unsubscribe` — release the target thread's registry entry, clear
/// its `STDIN_SUBSCRIBER`, recompute `base`, and broadcast so a capped producer may
/// proceed. `x0` = the arena-state pointer of the thread to unsubscribe. A no-op if
/// the log was never initialized or the thread was not subscribed. Returns nothing.
pub(crate) fn lower_stdin_unsubscribe(
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<CodeFunction, String> {
    let symbol = STDIN_UNSUBSCRIBE_SYMBOL;
    let l = |s: &str| format!("{symbol}_{s}");
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    let mut vregs = Vregs::new();
    let v52 = vregs.next();
    let v53 = vregs.next();
    let v75 = vregs.next();
    let v78 = vregs.next();
    let v89 = vregs.next();
    instructions.push(abi::move_register(&v53, abi::c_arg(0)));
    push_log_address(symbol, &v78, &mut instructions, &mut relocations);
    instructions.extend([
        abi::load_u64(&v75, &v78, STDIN_LOG_INITIALIZED_OFFSET),
        abi::compare_immediate(&v75, "0"),
        abi::branch_eq(&l("done")),
        abi::move_register(abi::c_arg(0), &v78),
    ]);
    emit_libc(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_mutex_lock",
    )?;
    field_addr(&v52, &v53, ARENA_STDIN_SUBSCRIBER_OFFSET, &mut instructions);
    instructions.extend([
        abi::load_u64(&v89, &v52, 0),
        abi::compare_immediate(&v89, "0"),
        abi::branch_eq(&l("unlock")),
        abi::store_u64(abi::ZERO, &v89, STDIN_SUBSCRIBER_ACTIVE_OFFSET),
        abi::store_u64(abi::ZERO, &v52, 0),
    ]);
    instructions.push(abi::branch_link(STDIN_RECOMPUTE_BASE_SYMBOL));
    relocations.push(internal_branch(symbol, STDIN_RECOMPUTE_BASE_SYMBOL));
    push_log_address(symbol, &v78, &mut instructions, &mut relocations);
    instructions.push(abi::add_immediate(abi::c_arg(0), &v78, STDIN_LOG_CV_OFFSET));
    emit_libc(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_cond_broadcast",
    )?;
    instructions.push(abi::label(&l("unlock")));
    push_log_address(symbol, &v78, &mut instructions, &mut relocations);
    instructions.push(abi::move_register(abi::c_arg(0), &v78));
    emit_libc(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_mutex_unlock",
    )?;
    instructions.push(abi::label(&l("done")));
    instructions.push(abi::return_());
    Ok(finalize_vreg_helper(
        "runtime.stdin.unsubscribe",
        symbol,
        "Nothing",
        instructions,
        relocations,
    ))
}
