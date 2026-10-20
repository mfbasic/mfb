//! plan-176-P: the terminal backend's screen mode — entering and leaving the alternate screen
//! (`emit_on`, `emit_off`). The constants, data objects, `lower_term_helper` and `StampCtx`
//! live in `term/core/term/mod.rs`.

use super::*;

pub(super) fn emit_on(
    ctx: &mut EmitCtx,
    term_state_offset: usize,
    mouse_state_offset: Option<usize>,
    input_state_offset: Option<usize>,
    done: &str,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;
    let v9: Reg = vregs.next().into();

    // plan-35-B: allocate the console shadow-grid header block sized to the
    // terminal *before* marking TUI mode active, so a program never sees
    // `active == 1` with a null grid. On allocation failure surface
    // `ERR_OUT_OF_MEMORY` and leave the terminal untouched.
    let alloc_fail = format!("{symbol}_grid_alloc_fail");
    let request = match platform.family() {
        PlatformFamily::MacOS => DARWIN_TIOCGWINSZ,
        PlatformFamily::Linux => LINUX_TIOCGWINSZ,
        // Windows ignores the ioctl request value (emit_terminal_size uses
        // GetConsoleScreenBufferInfo); a placeholder keeps the match total.
        PlatformFamily::Windows => "0",
    };
    // bug-967: a re-entered `term::on` (TUI already on) finds the current grid in
    // slot 48. Park it, allocate the new one (sized to the terminal as it is NOW),
    // and free the parked one only once the new block is in place — so an
    // allocation failure still leaves the running TUI its grid, and the
    // replacement no longer orphans ~445 KB (24x80) per call.
    let previous_grid: Reg = vregs.next().into();
    ctx.instructions.extend([
        abi::load_u64(
            previous_grid,
            ARENA_STATE_REGISTER,
            term_state_offset + term_grid::TERM_STATE_GRID_OFFSET,
        ),
        abi::store_u64(previous_grid, abi::stack_pointer(), PREVIOUS_GRID_OFFSET),
    ]);
    term_grid::emit_grid_alloc(
        symbol,
        term_state_offset,
        request,
        SCRATCH_OFFSET,
        ARG0_OFFSET,
        ARG1_OFFSET,
        &alloc_fail,
        platform,
        platform_imports,
        ctx.instructions,
        ctx.relocations,
        vregs,
    )?;
    term_grid::emit_grid_free_previous(
        symbol,
        PREVIOUS_GRID_OFFSET,
        ctx.instructions,
        ctx.relocations,
        vregs,
    );
    // Reset all state to defaults (plan §4.2). Foreground white, background
    // black, bold/underline off, cursor visible, active on.
    let writes: &[(usize, &str)] = &[
        (TERM_STATE_ACTIVE_OFFSET, "1"),
        (TERM_STATE_FG_OFFSET, DEFAULT_FOREGROUND_PACKED),
        (TERM_STATE_BG_OFFSET, "0"),
        (TERM_STATE_BOLD_OFFSET, "0"),
        (TERM_STATE_UNDERLINE_OFFSET, "0"),
        (TERM_STATE_CURSOR_VISIBLE_OFFSET, "1"),
        (TERM_STATE_DID_RESIZE_OFFSET, "0"),
    ];
    for (offset, value) in writes {
        ctx.instructions
            .push(abi::move_immediate(&v9, "Integer", value));
        ctx.instructions.push(abi::store_u64(
            &v9,
            ARENA_STATE_REGISTER,
            term_state_offset + offset,
        ));
    }
    // Enable ANSI/VT output interpretation before the first escape write. No-op on
    // POSIX terminals; on Windows this sets ENABLE_VIRTUAL_TERMINAL_PROCESSING on
    // the stdout console handle so the ESC_ON sequence (and every later styling
    // write) renders instead of printing raw (plan-66-D).
    platform.emit_enable_vt_output(symbol, platform_imports, ctx.instructions, ctx.relocations)?;
    emit_write_const(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        ESC_ON_SYMBOL,
        ESC_ON.len(),
    )?;
    // bug-149: entering interactive TUI mode also puts the console tty into
    // single-key (cbreak) mode once — `~ICANON`/`~ECHO`/`VMIN=1`/`VTIME=0` — so a
    // key-reading loop (`io::readKey`, `io::readScalar`, …) sees bare keypresses
    // without waiting for Return. The saved cooked discipline is parked in the term-state region (off
    // `ARENA_STATE_REGISTER`), from which `term::off` and `io::input`/
    // `io::readLine` restore it. When stdin is not a tty (piped input,
    // acceptance harness) `emit_configure_stdin_terminal` leaves the raw-active
    // flag at 0 and this is inert. A `tcgetattr`/`tcsetattr` failure branches to
    // `raw_failed`, which clears the flag — a terminal-setup failure must not make
    // the reads think raw mode is live (term setters are best-effort, §4.2.1).
    let raw_failed = format!("{symbol}_raw_failed");
    let raw_done = format!("{symbol}_raw_done");
    let raw_slots = TerminalModeSlots {
        active: term_state_offset + TERM_STATE_RAW_ACTIVE_OFFSET,
        saved_tag: term_state_offset + TERM_STATE_RAW_ACTIVE_OFFSET,
        saved_value: term_state_offset + TERM_STATE_RAW_ACTIVE_OFFSET,
        saved_message: term_state_offset + TERM_STATE_RAW_ACTIVE_OFFSET,
        original: term_state_offset + TERM_STATE_COOKED_TERMIOS_OFFSET,
        modified: term_state_offset + TERM_STATE_RAW_TERMIOS_OFFSET,
    };
    // No `emit_on` value is live across this call, so a counter scoped to it
    // cannot alias one (bug-721).
    let mut vregs = Vregs::new();
    // bug-728: a re-entered `term::on` finds raw mode already live. Saving again
    // would overwrite the cooked discipline with the raw one, and `term::off`
    // would then "restore" raw mode. Keep the first save: skip straight past the
    // configuration when the raw-active flag is already set.
    let raw_live: Reg = vregs.next().into();
    ctx.instructions.extend([
        abi::load_u64(
            &raw_live,
            ARENA_STATE_REGISTER,
            term_state_offset + TERM_STATE_RAW_ACTIVE_OFFSET,
        ),
        abi::compare_immediate(&raw_live, "0"),
        abi::branch_ne(&raw_done),
    ]);
    emit_configure_stdin_terminal(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        &mut vregs,
        &raw_slots,
        ARENA_STATE_REGISTER,
        true,
        true,
        &raw_failed,
    )?;
    ctx.instructions.push(abi::branch(&raw_done));
    ctx.instructions.push(abi::label(&raw_failed));
    ctx.instructions
        .push(abi::move_immediate(&v9, "Integer", "0"));
    ctx.instructions.push(abi::store_u64(
        &v9,
        ARENA_STATE_REGISTER,
        term_state_offset + TERM_STATE_RAW_ACTIVE_OFFSET,
    ));
    ctx.instructions.push(abi::label(&raw_done));
    // bug-699 Phase 5: switch a terminal that speaks the kitty keyboard protocol
    // to it, so the key readers see presses, repeats and releases with every
    // modifier. Only in a program whose readers decode keys, and only once stdin
    // is in raw mode (so the terminal's replies are not echoed).
    if let Some(input_state_offset) = input_state_offset {
        emit_kitty_enable(
            ctx,
            term_state_offset,
            mouse_state_offset,
            input_state_offset,
            &mut vregs,
        )?;
    }
    ctx.instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    ctx.instructions.push(abi::branch(done));
    // Grid allocation failed: active was never set, so the terminal is untouched.
    ctx.instructions.push(abi::label(&alloc_fail));
    raise_error_into(symbol, "ErrOutOfMemory", ctx.instructions, ctx.relocations);
    Ok(())
}

pub(super) fn emit_off(
    ctx: &mut EmitCtx,
    term_state_offset: usize,
    mouse_state_offset: Option<usize>,
    input_state_offset: Option<usize>,
    done: &str,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;
    let v9: Reg = vregs.next().into();

    let inactive = format!("{symbol}_inactive");
    emit_gate_inactive(term_state_offset, &inactive, v9, ctx.instructions);
    // plan-35-C: present the final frame before restoring the user's screen, so
    // the last frame the program drew is shown. Reuse the `term::sync` helper as
    // the present routine (force-emitted whenever `term::` is used).
    ctx.instructions
        .push(abi::branch_link("_mfb_rt_term_term_sync"));
    ctx.relocations
        .push(internal_branch(symbol, "_mfb_rt_term_term_sync"));
    // bug-149: leaving TUI mode restores the saved cooked line discipline that
    // `term::on` captured, so the terminal returns to canonical/echoing input.
    // A no-op when the raw-active flag is 0 (stdin was never put into raw mode).
    let raw_restore_skip = format!("{symbol}_raw_restore_skip");
    ctx.instructions.push(abi::load_u64(
        &v9,
        ARENA_STATE_REGISTER,
        term_state_offset + TERM_STATE_RAW_ACTIVE_OFFSET,
    ));
    ctx.instructions.push(abi::compare_immediate(&v9, "0"));
    ctx.instructions.push(abi::branch_eq(&raw_restore_skip));
    ctx.instructions
        .push(abi::move_immediate(abi::return_register(), "Integer", "0"));
    ctx.instructions
        .push(abi::move_immediate(abi::c_arg(1), "Integer", "0"));
    ctx.instructions.push(abi::add_immediate(
        abi::c_arg(2),
        ARENA_STATE_REGISTER,
        term_state_offset + TERM_STATE_COOKED_TERMIOS_OFFSET,
    ));
    platform.emit_terminal_control_call(
        TerminalControlCall::SetAttrs,
        symbol,
        platform_imports,
        ctx.instructions,
        ctx.relocations,
    )?;
    ctx.instructions
        .push(abi::move_immediate(&v9, "Integer", "0"));
    ctx.instructions.push(abi::store_u64(
        &v9,
        ARENA_STATE_REGISTER,
        term_state_offset + TERM_STATE_RAW_ACTIVE_OFFSET,
    ));
    ctx.instructions.push(abi::label(&raw_restore_skip));
    // bug-699 Phase 5: hand the keyboard back in the mode `term::on` found it —
    // pop the kitty flags it pushed. Also reached at exit (the shutdown path
    // calls `term::off`), so a program that forgets leaves the terminal clean.
    if input_state_offset.is_some() {
        emit_kitty_disable(ctx, vregs);
    }
    emit_write_const(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        ESC_OFF_SYMBOL,
        ESC_OFF.len(),
    )?;
    ctx.instructions
        .push(abi::move_immediate(&v9, "Integer", "0"));
    ctx.instructions.push(abi::store_u64(
        &v9,
        ARENA_STATE_REGISTER,
        term_state_offset + TERM_STATE_ACTIVE_OFFSET,
    ));
    // plan-35-B: free the shadow-grid block and zero its slot (no-op if null).
    term_grid::emit_grid_free(
        symbol,
        term_state_offset,
        ctx.instructions,
        ctx.relocations,
        vregs,
    );
    ctx.instructions.push(abi::label(&inactive));
    // plan-94-B: leaving TUI mode also withdraws mouse tracking, so a program that
    // enabled it and forgot to disable it does not leave the user's terminal
    // reporting every movement to whatever runs next.
    //
    // **After the `inactive` label, so it runs whether or not TUI mode was on** —
    // mouse mode is independent of `term::on`, and a program may enable the mouse
    // without ever entering TUI mode.
    //
    // **Emitted only when the program uses the mouse at all** (`mouse_state_offset`
    // is `Some`). That is not an optimisation: `_mfb_rt_mouse_mode` and the mouse
    // escape objects only exist in a mouse program, so referencing them from every
    // `term::off` would leave an undefined symbol in every other one — and it would
    // put mouse-reset bytes in the output of a program that never asked for mouse,
    // which is exactly what plan-94-A's "no new ANSI bytes" test forbids.
    if let Some(mouse_state_offset) = mouse_state_offset {
        emit_mouse_withdraw(ctx, mouse_state_offset, vregs)?;
    }
    ctx.instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    let _ = done;
    Ok(())
}
