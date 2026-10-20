//! plan-176-P: the terminal backend's mouse reporting — enable/withdraw, tracking window, mode,
//! injection and polling. The constants, data objects, `lower_term_helper` and `StampCtx` live
//! in `term/core/term/mod.rs`.

use super::*;

/// `term::enableMouse(Boolean)` (plan-94-B): the opt-in mouse-reporting toggle.
///
/// On: write the 1000/1002/1006 mode-set sequence, allocate the ring, set the
/// process-global mouse-mode word to "cells", and run any `MFB_MOUSE_INJECT` bytes
/// through the decoder. Off: write the matching reset, clear the word, free the
/// ring.
///
/// All three modes are set together because they answer three different questions
/// and a terminal needs all three answers: 1000 turns reporting on at all, 1002
/// adds motion-while-dragging (without it a drag is invisible — only the press and
/// the release arrive), and **1006 is mandatory**, not an optimisation: the default
/// encoding biases each coordinate by 32 into one byte and so cannot express a
/// column past 223, let alone a pixel. See `io::mouse::decode`.
///
/// Not gated on `active`: asking for mouse reporting before `term::on` is a
/// sequencing mistake a program can make either way, and a setter that silently
/// no-ops is the established `term::` answer (`emit_set_attr`) rather than an
/// error. The mode word and the ring are independent of TUI mode, so the request
/// simply takes effect.
pub(super) fn emit_enable_mouse(
    ctx: &mut EmitCtx,
    mouse_state_offset: Option<usize>,
    app_mode: bool,
    done: &str,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let Some(mouse_state_offset) = mouse_state_offset else {
        return Err(format!(
            "native code plan emits '{symbol}' without reserving mouse state"
        ));
    };
    let disable = format!("{symbol}_mouse_disable");

    // The argument is the `Boolean` in the MFB argument register.
    let enabled: Reg = vregs.next().into();
    ctx.instructions
        .push(abi::move_register(&enabled, abi::mfb_arg(0)));
    ctx.instructions.push(abi::compare_immediate(&enabled, "0"));
    ctx.instructions.push(abi::branch_eq(&disable));

    // --- enable ---
    //
    // The terminal mode sequences are written only in a console build. In an
    // `--app` build stdout is the window's transcript, not a terminal: the escape
    // would be *displayed* rather than interpreted, printing `[?1000h` into the
    // user's view, and no terminal is listening to turn reporting on anyway. The
    // window's own event handlers are the source there (plan-94-C/D/E), and what
    // switches them on is the mode word below — which is written in both builds.
    if !app_mode {
        emit_write_const(ctx, ESC_MOUSE_ON_SYMBOL, ESC_MOUSE_ON.len())?;
    }
    mouse_ring::emit_ring_alloc(mouse_state_offset, ctx, vregs)?;
    emit_store_mouse_mode(ctx, MOUSE_MODE_CELLS, vregs);
    emit_mouse_inject(ctx, mouse_state_offset, MOUSE_CLOCK_SCRATCH_OFFSET, vregs)?;
    ctx.instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(done),
        abi::label(&disable),
    ]);

    // --- disable ---
    if !app_mode {
        emit_write_const(ctx, ESC_MOUSE_OFF_SYMBOL, ESC_MOUSE_OFF.len())?;
    }
    emit_store_mouse_mode(ctx, MOUSE_MODE_OFF, vregs);
    mouse_ring::emit_ring_free(mouse_state_offset, ctx, vregs)?;
    ctx.instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    Ok(())
}

/// Withdraw mouse tracking: reset the terminal modes, clear the mode word, free
/// the ring. Idempotent, and a no-op when mouse was never enabled.
///
/// The escape write is gated on the mode word rather than emitted unconditionally,
/// so a program that imports the mouse members but never calls
/// `enableMouse(TRUE)` still emits no tracking bytes at all.
pub(super) fn emit_mouse_withdraw(
    ctx: &mut EmitCtx,
    mouse_state_offset: usize,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let skip = format!("{symbol}_mouse_withdraw_skip");

    let addr: Reg = vregs.next().into();
    let mode: Reg = vregs.next().into();
    push_symbol_address(
        symbol,
        MOUSE_MODE_SYMBOL,
        &addr,
        ctx.instructions,
        ctx.relocations,
    );
    ctx.instructions.push(abi::load_u64(&mode, &addr, 0));
    ctx.instructions.push(abi::compare_immediate(&mode, "0"));
    ctx.instructions.push(abi::branch_eq(&skip));
    emit_write_const(ctx, ESC_MOUSE_OFF_SYMBOL, ESC_MOUSE_OFF.len())?;
    emit_store_mouse_mode(ctx, MOUSE_MODE_OFF, vregs);
    mouse_ring::emit_ring_free(mouse_state_offset, ctx, vregs)?;
    ctx.instructions.push(abi::label(&skip));
    Ok(())
}

/// Suspend or resume the terminal's mouse reporting around a cooked-mode read
/// (plan-94-B §3e).
///
/// `io::input` and `io::readLine` bracket their read with a restore of the saved
/// cooked line discipline, which re-enables **echo** for the duration. A mouse
/// report arriving in that window would be echoed onto the user's screen as
/// visible garbage — a literal `\x1b[<0;40;12M` appearing in the middle of what
/// they are typing — and delivered a line at a time rather than a byte at a time.
///
/// So tracking is withdrawn before the restore and re-established after the raw
/// termios goes back. Mouse events are lost for the duration of the line read,
/// which is correct rather than merely tolerable: a program asking the user to
/// type a line is not tracking the mouse, and the alternative on offer was
/// corrupting the thing it *is* doing.
///
/// The mode word is deliberately **not** cleared — the program has not stopped
/// wanting mouse events, only this read has stopped being able to accept them —
/// so the gate reads it to decide whether there is anything to suspend.
///
/// `preserve_result` parks the `Result` registers across the write, for the same
/// reason `emit_console_raw_line_mode` has the flag: the **resume** side runs
/// after the read's result is already staged, and the write clobbers the result
/// bank. Without it `io::readLine` returns a wild pointer and the program faults
/// on the first use of the string — which is exactly what happened the first time
/// this was wired up, and it is invisible until the returned line is touched.
///
/// `vregs` is the caller's minter (bug-721): a fresh `Vregs` here would restart at
/// `%v0` and silently name the same registers as the caller's own vregs.
pub(crate) fn emit_mouse_tracking_window(
    ctx: &mut EmitCtx,
    resume: bool,
    preserve_result: bool,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let tag = if resume { "resume" } else { "suspend" };
    let skip = format!("{symbol}_mouse_{tag}_skip");

    let addr: Reg = vregs.next().into();
    let mode: Reg = vregs.next().into();
    push_symbol_address(
        symbol,
        MOUSE_MODE_SYMBOL,
        &addr,
        ctx.instructions,
        ctx.relocations,
    );
    ctx.instructions.push(abi::load_u64(&mode, &addr, 0));
    ctx.instructions.push(abi::compare_immediate(&mode, "0"));
    ctx.instructions.push(abi::branch_eq(&skip));
    // Distinct vregs from `emit_console_raw_line_mode`'s `%v20…%v22`: that call
    // has finished restoring by the time this one runs, but reusing its parking
    // slots would make the two impossible to reorder safely later.
    let saved_tag: Reg = vregs.next().into();
    let saved_value: Reg = vregs.next().into();
    let saved_message: Reg = vregs.next().into();
    if preserve_result {
        ctx.instructions.extend([
            abi::move_register(&saved_tag, RESULT_TAG_REGISTER),
            abi::move_register(&saved_value, RESULT_VALUE_REGISTER),
            abi::move_register(&saved_message, RESULT_ERROR_MESSAGE_REGISTER),
        ]);
    }
    if resume {
        emit_write_const(ctx, ESC_MOUSE_ON_SYMBOL, ESC_MOUSE_ON.len())?;
    } else {
        emit_write_const(ctx, ESC_MOUSE_OFF_SYMBOL, ESC_MOUSE_OFF.len())?;
    }
    if preserve_result {
        ctx.instructions.extend([
            abi::move_register(RESULT_TAG_REGISTER, &saved_tag),
            abi::move_register(RESULT_VALUE_REGISTER, &saved_value),
            abi::move_register(RESULT_ERROR_MESSAGE_REGISTER, &saved_message),
        ]);
    }
    ctx.instructions.push(abi::label(&skip));
    Ok(())
}

/// Store `value` into the process-global mouse-mode word.
///
/// Process-global rather than arena state because an app backend's mouse handler
/// runs on the UI thread and has no arena state to read (plan-94-A §4.4b). On the
/// console backend nothing reads it yet — but writing it here is what lets
/// plan-94-C/D/E's handlers be driven by the same `enableMouse` call the program
/// already makes.
pub(crate) fn emit_store_mouse_mode(ctx: &mut EmitCtx, value: u64, vregs: &mut Vregs) {
    let symbol = ctx.symbol;
    let addr: Reg = vregs.next().into();
    let word: Reg = vregs.next().into();
    push_symbol_address(
        symbol,
        MOUSE_MODE_SYMBOL,
        &addr,
        ctx.instructions,
        ctx.relocations,
    );
    ctx.instructions
        .push(abi::move_immediate(&word, "Integer", &value.to_string()));
    ctx.instructions.push(abi::store_u64(&word, &addr, 0));
}

/// Feed `MFB_MOUSE_INJECT`'s bytes through the decoder, if it is set.
///
/// The test affordance plan-94-B Corrections B2 settled on, and the same shape as
/// `MFB_WINAPP_INPUT`: it carries the raw SGR bytes a terminal would really send,
/// so it exercises the **real** decoder rather than bypassing it. That is what lets
/// the ring, the TTL and the SGR parse all be proven on a machine with no mouse —
/// and it is the only way plan-94-C/D/E can be exercised at all.
///
/// Pass-through bytes are dropped rather than delivered: injected input is test
/// input, and there is no read in progress to hand a keystroke to.
///
/// **In every build, not only `--debug`,** unlike the other test hooks
/// (`./mfb spec tooling debug-report`). While canvas is in development,
/// `tools/canvas-bench`'s `hit` benchmark needs pointer events in a release build,
/// whose timings a `--debug` build's instrumentation would distort, and this is the
/// only source of them without a real mouse. It moves behind `--debug` once canvas
/// settles.
pub(crate) fn emit_mouse_inject(
    ctx: &mut EmitCtx,
    mouse_state_offset: usize,
    // The caller's 16-byte clock scratch. Passed rather than fixed, because the
    // `term::` and `canvas::` bodies have different frames — and because the
    // injection affordance has to serve both surfaces, not only the terminal one.
    clock_scratch: usize,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let none = format!("{symbol}_inject_none");
    let loop_head = format!("{symbol}_inject_loop");
    let loop_done = format!("{symbol}_inject_done");

    // **POSIX only.** The variable carries raw SGR *bytes*, and Windows'
    // environment API is UTF-16 (`GetEnvironmentVariableW`) — feeding its output
    // to a byte decoder would need a conversion whose only consumer is a test
    // affordance. `getenv` is also not a kernel32 export, so declaring it there
    // fails to link outright.
    //
    // The cost is named rather than hidden: on Windows the mouse path is proven
    // by a human with a mouse on a real Windows host, which is what plan-94-E's
    // "Runtime proof: manual on a Windows host" already specifies. Every other
    // target keeps the affordance.
    if ctx.platform.family() == PlatformFamily::Windows {
        return Ok(());
    }
    let name: Reg = vregs.next().into();
    push_symbol_address(
        symbol,
        MOUSE_INJECT_ENV_SYMBOL,
        &name,
        ctx.instructions,
        ctx.relocations,
    );
    ctx.instructions
        .push(abi::move_register(abi::c_arg(0), &name));
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;
    platform.emit_external_call(
        "getenv",
        symbol,
        platform_imports,
        ctx.instructions,
        ctx.relocations,
    )?;
    let text: Reg = vregs.next().into();
    ctx.instructions
        .push(abi::move_register(&text, abi::c_return(0)));
    ctx.instructions.push(abi::compare_immediate(&text, "0"));
    ctx.instructions.push(abi::branch_eq(&none));

    // Walk the NUL-terminated value one byte at a time, exactly as the read path
    // will, so the same state machine sees the same sequence of bytes.
    let cursor: Reg = vregs.next().into();
    let byte: Reg = vregs.next().into();
    let action: Reg = vregs.next().into();
    let out_byte: Reg = vregs.next().into();
    ctx.instructions.extend([
        abi::move_register(&cursor, &text),
        abi::label(&loop_head),
        abi::load_u8(&byte, &cursor, 0),
        abi::compare_immediate(&byte, "0"),
        abi::branch_eq(&loop_done),
    ]);
    mouse_decode::emit_decode_byte(
        &byte,
        &action,
        &out_byte,
        mouse_state_offset,
        clock_scratch,
        ctx,
        vregs,
    )?;
    ctx.instructions.extend([
        abi::add_immediate(&cursor, &cursor, 1),
        abi::branch(&loop_head),
        abi::label(&loop_done),
        abi::label(&none),
    ]);
    Ok(())
}

/// `term::pollMouse() AS MouseEvent` (plan-94-B): take the next pending event.
///
/// Drains the ring — oldest first, skipping anything past its TTL — and builds the
/// `MouseEvent` record from what it finds. An empty or all-stale ring yields the
/// all-zero record, which reads as `MouseKind.None` because `None` is the
/// first-declared variant of both enums. That sentinel is unchanged from plan-94-A:
/// what changed is where a non-`None` answer comes from.
pub(super) fn emit_poll_mouse(
    ctx: &mut EmitCtx,
    mouse_state_offset: Option<usize>,
    input_state_offset: Option<usize>,
    app_mode: bool,
    done: &str,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let Some(mouse_state_offset) = mouse_state_offset else {
        return Err(format!(
            "native code plan emits '{symbol}' without reserving mouse state"
        ));
    };
    let alloc_ok = format!("{symbol}_alloc_ok");
    let alloc_error = format!("{symbol}_alloc_error");

    let pending = crate::codegen::builtins::io::gen_read_family::PendingInput {
        app_mode,
        input_state_offset,
        slots: crate::codegen::builtins::io::gen_read_family::PendingInputSlots::at(
            POLL_MOUSE_PENDING_OFFSET,
        ),
    };
    let event = mouse_ring::emit_dequeue(mouse_state_offset, &pending, ctx, vregs)?;

    // Park the five values across the allocation call, which clobbers the argument
    // and result banks.
    let slots = [
        (ARG0_OFFSET, &event.kind),
        (ARG1_OFFSET, &event.button),
        (SCRATCH_OFFSET, &event.coord_a),
        (SCRATCH_OFFSET + 8, &event.coord_b),
        (SCRATCH_OFFSET + 16, &event.mods),
    ];
    for (offset, reg) in slots {
        ctx.instructions
            .push(abi::store_u64(reg, abi::stack_pointer(), offset));
    }

    ctx.instructions.extend([
        abi::move_immediate(
            abi::return_register(),
            "Integer",
            &MOUSE_EVENT_RECORD_SIZE.to_string(),
        ),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    ctx.relocations
        .push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    ctx.instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
    ]);

    let rec: Reg = vregs.next().into();
    let value: Reg = vregs.next().into();
    let bit: Reg = vregs.next().into();
    let mods: Reg = vregs.next().into();
    ctx.instructions
        .push(abi::move_register(&rec, RESULT_VALUE_REGISTER));

    // `MouseEvent { kind, button, row, column, shift, ctrl, alt }` — seven 8-byte
    // slots in declaration order. `coord_a` is the row and `coord_b` the column:
    // the decoder already swapped the wire's `x;y` into row-first, so nothing here
    // has to know the wire order.
    for (offset, slot) in [
        (0usize, ARG0_OFFSET),
        (8, ARG1_OFFSET),
        (16, SCRATCH_OFFSET),
        (24, SCRATCH_OFFSET + 8),
    ] {
        ctx.instructions
            .push(abi::load_u64(&value, abi::stack_pointer(), slot));
        ctx.instructions.push(abi::store_u64(&value, &rec, offset));
    }
    // The three modifier flags unpack from the packed bits into `Boolean`s.
    ctx.instructions.push(abi::load_u64(
        &mods,
        abi::stack_pointer(),
        SCRATCH_OFFSET + 16,
    ));
    for (offset, mask) in [
        (32usize, MOUSE_MOD_SHIFT),
        (40, MOUSE_MOD_CTRL),
        (48, MOUSE_MOD_ALT),
    ] {
        let set = format!("{symbol}_mod_set_{mask}");
        let store = format!("{symbol}_mod_store_{mask}");
        ctx.instructions.extend([
            abi::move_immediate(&bit, "Integer", &mask.to_string()),
            abi::and_registers(&value, &mods, &bit),
            abi::compare_immediate(&value, "0"),
            abi::branch_ne(&set),
            abi::move_immediate(&value, "Boolean", "0"),
            abi::branch(&store),
            abi::label(&set),
            abi::move_immediate(&value, "Boolean", "1"),
            abi::label(&store),
            abi::store_u64(&value, &rec, offset),
        ]);
    }

    ctx.instructions.extend([
        abi::move_register(RESULT_VALUE_REGISTER, &rec),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(done),
        abi::label(&alloc_error),
    ]);
    raise_error_into(symbol, "ErrOutOfMemory", ctx.instructions, ctx.relocations);
    Ok(())
}
