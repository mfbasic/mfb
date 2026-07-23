use super::*;

/// Shared process teardown. Reads the main arena-state address from the writable
/// global, clears the global (so a second entry — e.g. a signal arriving during
/// normal cleanup — becomes a no-op), pins it in `x19`, then conditionally
/// restores the terminal and frees the arena. Both underlying helpers are
/// idempotent (`term::off` gates on its `active` flag; `arena_destroy` clears the
/// block-list head), so the guard is belt-and-suspenders. Preserves `x19`/`x30`
/// for its callers (the entry exit path relies on `x19` afterwards).
pub(super) fn lower_shutdown(
    auto_term_off: bool,
    skip_arena_destroy: bool,
    drain_stdout: bool,
) -> CodeFunction {
    // Vreg-allocated (plan-00-G Phase 2). The allocator builds the frame and saves
    // the link register (there are `bl`s). `x19` (arena_base) is reserved from
    // allocation, but this function deliberately *repoints* it at the main arena to
    // run the teardown helpers, so it saves the caller's `x19` into a vreg (spilled
    // across the calls) and restores it before returning — the entry exit path
    // relies on `x19` afterwards.
    let done = "shutdown_done";
    let mut vregs = Vregs::new();
    let saved_arena = vregs.next();
    let global = vregs.next();
    let arena = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&saved_arena, ARENA_STATE_REGISTER),
    ];
    let mut relocations = Vec::new();
    push_symbol_address(
        SHUTDOWN_SYMBOL,
        MAIN_ARENA_GLOBAL_SYMBOL,
        &global,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::load_u64(&arena, &global, 0),
        abi::store_u64(abi::ZERO, &global, 0),
        abi::compare_immediate(&arena, "0"),
        abi::branch_eq(done),
        abi::move_register(ARENA_STATE_REGISTER, &arena),
    ]);
    // Drain the opt-in stdout buffer at exit (plan-14-A §4.3 hook 1) before the
    // arena — whose block backs the buffer — is freed. `x19` now points at the
    // main arena, so the drain reads the right OUT_* words. A no-op when buffering
    // is off; idempotent with a second entry (a signal during normal cleanup).
    if drain_stdout {
        instructions.push(abi::branch_link(STDOUT_DRAIN_SYMBOL));
        relocations.push(internal_branch(SHUTDOWN_SYMBOL, STDOUT_DRAIN_SYMBOL));
    }
    if auto_term_off {
        instructions.push(abi::branch_link("_mfb_rt_term_term_off"));
        relocations.push(internal_branch(SHUTDOWN_SYMBOL, "_mfb_rt_term_term_off"));
    }
    if !skip_arena_destroy {
        instructions.push(abi::branch_link(ARENA_DESTROY_SYMBOL));
        relocations.push(internal_branch(SHUTDOWN_SYMBOL, ARENA_DESTROY_SYMBOL));
    }
    instructions.extend([
        abi::label(done),
        abi::move_register(ARENA_STATE_REGISTER, &saved_arena),
        abi::return_(),
    ]);
    finalize_vreg_helper(
        "runtime.shutdown",
        SHUTDOWN_SYMBOL,
        "Nothing",
        instructions,
        relocations,
    )
}

/// `void handler(int signo)` for SIGINT/SIGTERM: run the shared teardown, then
/// `_exit(128 + signo)`. It never returns, so it need not preserve the
/// interrupted context; it locates the arena through `_mfb_shutdown`'s global
/// read rather than the interrupted `x19`. The 16-byte frame keeps `sp` aligned
/// across the `bl`s (Darwin requires this) and parks `signo` across the call.
pub(super) fn lower_signal_handler(platform: &dyn CodegenPlatform) -> Result<CodeFunction, String> {
    // Vreg-allocated (plan-00-G Phase 2). `signo` (x0) is parked across the
    // `bl _mfb_shutdown` in a vreg the allocator spills; the allocator + frame
    // builder provide the aligned frame and link-register save. The function never
    // returns (it tail-exits), so nothing needs preserving across it.
    let mut vregs = Vregs::new();
    let signo = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&signo, abi::return_register()),
        abi::branch_link(SHUTDOWN_SYMBOL),
    ];
    let mut relocations = vec![internal_branch(SIGNAL_HANDLER_SYMBOL, SHUTDOWN_SYMBOL)];
    instructions.push(abi::add_immediate(abi::return_register(), &signo, 128));
    platform.emit_program_exit(SIGNAL_HANDLER_SYMBOL, &mut instructions, &mut relocations)?;
    Ok(finalize_vreg_helper(
        "runtime.signal_handler",
        SIGNAL_HANDLER_SYMBOL,
        "Nothing",
        instructions,
        relocations,
    ))
}

/// Append the PCG64 LCG step `state = state * MULT + INC` operating on the
/// 128-bit state held in (`lo`, `hi`). The limbs are read at the start and
/// rewritten in place; `x11`-`x16` are used as scratch (caller-saved, so these
/// leaf helpers need not preserve them).
/// Populate every static closure descriptor's `code` word with `&func` at startup
/// (bug-78). A leaf function, run once from the entry before `main` and before any
/// thread is spawned; `env` stays 0 (the descriptor is BSS). Cannot fail — it only
/// materializes internal addresses and stores them, so the entry runs it with no
/// tag check.
pub(super) fn lower_closure_descriptor_initializer(func_symbols: &[String]) -> CodeFunction {
    let symbol = CLOSURE_DESC_INIT_SYMBOL;
    let mut vregs = Vregs::new();
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    for func_symbol in func_symbols {
        let desc_symbol = closure_descriptor_symbol(func_symbol);
        let func_reg = vregs.next();
        let desc_reg = vregs.next();
        // func_reg = &func ; desc_reg = &descriptor ; descriptor.code = &func.
        push_symbol_address(
            symbol,
            func_symbol,
            &func_reg,
            &mut instructions,
            &mut relocations,
        );
        push_symbol_address(
            symbol,
            &desc_symbol,
            &desc_reg,
            &mut instructions,
            &mut relocations,
        );
        instructions.push(abi::store_u64(&func_reg, &desc_reg, CLOSURE_OFFSET_CODE));
    }
    instructions.push(abi::return_());
    finalize_vreg_helper(
        "closure_descriptor_init",
        symbol,
        "Nothing",
        instructions,
        relocations,
    )
}
