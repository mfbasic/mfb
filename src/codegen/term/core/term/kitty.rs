//! plan-176-P: the terminal backend's kitty keyboard protocol — enable/disable, the probe
//! window and input events. The constants, data objects, `lower_term_helper` and `StampCtx`
//! live in `term/core/term/mod.rs`.

use super::*;

/// `term::on`'s half of the kitty keyboard protocol (bug-699 Phase 5): ask the
/// terminal whether it speaks it and, if it does, push the flags.
///
/// Skipped when standard input is not in raw mode (not a terminal, or the
/// raw-mode setup failed — the replies would be echoed, or never read) and when
/// the protocol is already on (a second `term::on`). The replies are read from
/// standard input until the Primary Device Attributes reply (`ESC [ ? … c`),
/// which every terminal sends, or [`KITTY_PROBE_MS`] passes. A flags reply
/// (`ESC [ ? … u`) before it means the terminal speaks the protocol. Every other
/// byte that arrives meanwhile — a key the user pressed — is put in the input
/// buffer in order, so the next read sees it.
pub(super) fn emit_kitty_enable(
    ctx: &mut EmitCtx,
    term_state_offset: usize,
    mouse_state_offset: Option<usize>,
    input_state_offset: usize,
    vregs: &mut Vregs,
) -> Result<(), String> {
    use crate::codegen::builtins::io::gen_read_family::{
        emit_stdin_byte_read, MousePump, ReadWait,
    };
    use crate::codegen::io::mouse::clock::emit_monotonic_nanos;
    const ESC: &str = "27";
    let symbol = ctx.symbol;
    let l = |name: &str| format!("{symbol}_kitty_{name}");
    let skip = l("skip");
    let next = l("next");
    let probe_done = l("probe_done");
    let state0 = l("state0");
    let state1 = l("state1");
    let state2 = l("state2");
    let state3 = l("state3");

    // Raw mode on, protocol not yet on.
    let t: Reg = vregs.next().into();
    let addr: Reg = vregs.next().into();
    ctx.instructions.extend([
        abi::load_u64(
            &t,
            ARENA_STATE_REGISTER,
            term_state_offset + TERM_STATE_RAW_ACTIVE_OFFSET,
        ),
        abi::compare_immediate(&t, "0"),
        abi::branch_eq(&skip),
    ]);
    push_symbol_address(
        symbol,
        INPUT_EVENTS_SYMBOL,
        &addr,
        ctx.instructions,
        ctx.relocations,
    );
    ctx.instructions.extend([
        abi::load_u64(&t, &addr, 0),
        abi::compare_immediate(&t, "0"),
        abi::branch_ne(&skip),
    ]);
    emit_write_const(ctx, ESC_KITTY_QUERY_SYMBOL, ESC_KITTY_QUERY.len())?;
    // deadline = now + KITTY_PROBE_MS; state 0; not found.
    let now: Reg = vregs.next().into();
    emit_monotonic_nanos(now.clone(), KITTY_CLOCK_OFFSET, ctx, vregs)?;
    let d: Reg = vregs.next().into();
    let zero: Reg = vregs.next().into();
    ctx.instructions.extend([
        abi::move_immediate(&d, "Integer", &(KITTY_PROBE_MS * 1_000_000).to_string()),
        abi::add_registers(&d, &d, &now),
        abi::store_u64(&d, abi::stack_pointer(), KITTY_DEADLINE_OFFSET),
        abi::move_immediate(&zero, "Integer", "0"),
        abi::store_u64(&zero, abi::stack_pointer(), KITTY_STATE_OFFSET),
        abi::store_u64(&zero, abi::stack_pointer(), KITTY_FOUND_OFFSET),
    ]);

    // One reply byte.
    ctx.instructions.push(abi::label(&next));
    let wait = ReadWait {
        deadline_slot: KITTY_DEADLINE_OFFSET,
        clock_scratch: KITTY_CLOCK_OFFSET,
        pollfd_slot: KITTY_POLLFD_OFFSET,
        timeout_label: probe_done.clone(),
    };
    emit_stdin_byte_read(
        ctx,
        false,
        KITTY_BYTE_OFFSET,
        &l("read_retry"),
        &l("read_resume"),
        // A read that fails or ends the input ends the probe: the protocol
        // stays off, and the reader that next reads meets the condition itself.
        &probe_done,
        &probe_done,
        mouse_state_offset.map(|state_offset| MousePump {
            state_offset,
            clock_scratch: KITTY_CLOCK_OFFSET,
        }),
        Some(&wait),
        vregs,
    )?;
    ctx.instructions.push(abi::branch_eq(&probe_done));
    let b: Reg = vregs.next().into();
    let state: Reg = vregs.next().into();
    ctx.instructions.extend([
        abi::load_u8(&b, abi::stack_pointer(), KITTY_BYTE_OFFSET),
        abi::load_u64(&state, abi::stack_pointer(), KITTY_STATE_OFFSET),
        abi::compare_immediate(&state, "1"),
        abi::branch_eq(&state1),
        abi::compare_immediate(&state, "2"),
        abi::branch_eq(&state2),
        abi::compare_immediate(&state, "3"),
        abi::branch_eq(&state3),
    ]);
    // State 0: outside a reply. ESC may start one; anything else is input.
    let set_state = |ctx: &mut EmitCtx, value: &str, vregs: &mut Vregs| {
        let v: Reg = vregs.next().into();
        ctx.instructions.extend([
            abi::move_immediate(&v, "Integer", value),
            abi::store_u64(&v, abi::stack_pointer(), KITTY_STATE_OFFSET),
        ]);
    };
    ctx.instructions.push(abi::label(&state0));
    let not_esc = l("state0_input");
    ctx.instructions
        .extend([abi::compare_immediate(&b, ESC), abi::branch_ne(&not_esc)]);
    set_state(ctx, "1", vregs);
    ctx.instructions
        .extend([abi::branch(&next), abi::label(&not_esc)]);
    emit_kitty_keep_byte(ctx, input_state_offset, &b, &l("keep_b"), vregs);
    ctx.instructions.push(abi::branch(&next));
    // State 1: after ESC. `[` continues; anything else was a key (Alt+x, or a
    // lone Escape): keep the ESC and look at this byte afresh.
    ctx.instructions.push(abi::label(&state1));
    let not_bracket = l("state1_input");
    ctx.instructions.extend([
        abi::compare_immediate(&b, "91"),
        abi::branch_ne(&not_bracket),
    ]);
    set_state(ctx, "2", vregs);
    ctx.instructions
        .extend([abi::branch(&next), abi::label(&not_bracket)]);
    let esc: Reg = vregs.next().into();
    ctx.instructions
        .push(abi::move_immediate(&esc, "Integer", ESC));
    emit_kitty_keep_byte(ctx, input_state_offset, &esc, &l("keep_esc1"), vregs);
    set_state(ctx, "0", vregs);
    ctx.instructions.push(abi::branch(&state0));
    // State 2: after ESC `[`. `?` starts a reply; anything else is a key's
    // sequence: keep `ESC [` and look at this byte afresh.
    ctx.instructions.push(abi::label(&state2));
    let not_query = l("state2_input");
    ctx.instructions
        .extend([abi::compare_immediate(&b, "63"), abi::branch_ne(&not_query)]);
    set_state(ctx, "3", vregs);
    ctx.instructions
        .extend([abi::branch(&next), abi::label(&not_query)]);
    let esc: Reg = vregs.next().into();
    ctx.instructions
        .push(abi::move_immediate(&esc, "Integer", ESC));
    emit_kitty_keep_byte(ctx, input_state_offset, &esc, &l("keep_esc2"), vregs);
    let bracket: Reg = vregs.next().into();
    ctx.instructions
        .push(abi::move_immediate(&bracket, "Integer", "91"));
    emit_kitty_keep_byte(ctx, input_state_offset, &bracket, &l("keep_bracket"), vregs);
    set_state(ctx, "0", vregs);
    ctx.instructions.push(abi::branch(&state0));
    // State 3: inside a reply's parameters. `u` ends the flags reply, `c` the
    // device-attributes reply (the last one); any other final byte ends a reply
    // this probe did not ask for.
    ctx.instructions.push(abi::label(&state3));
    let not_u = l("state3_not_u");
    let param = l("state3_param");
    ctx.instructions.extend([
        abi::compare_immediate(&b, "99"),
        abi::branch_eq(&probe_done),
        abi::compare_immediate(&b, "117"),
        abi::branch_ne(&not_u),
    ]);
    let one: Reg = vregs.next().into();
    ctx.instructions.extend([
        abi::move_immediate(&one, "Integer", "1"),
        abi::store_u64(&one, abi::stack_pointer(), KITTY_FOUND_OFFSET),
    ]);
    set_state(ctx, "0", vregs);
    ctx.instructions
        .extend([abi::branch(&next), abi::label(&not_u)]);
    // Parameter bytes (0x30..=0x3F) keep the reply going.
    let low = l("state3_end");
    ctx.instructions.extend([
        abi::compare_immediate(&b, "48"),
        abi::branch_lo(&low),
        abi::compare_immediate(&b, "63"),
        abi::branch_ls(&param),
        abi::label(&low),
    ]);
    set_state(ctx, "0", vregs);
    ctx.instructions
        .extend([abi::label(&param), abi::branch(&next)]);

    // The replies are in (or the wait is over): push the flags if the terminal
    // answered the flags query.
    ctx.instructions.push(abi::label(&probe_done));
    let found: Reg = vregs.next().into();
    ctx.instructions.extend([
        abi::load_u64(&found, abi::stack_pointer(), KITTY_FOUND_OFFSET),
        abi::compare_immediate(&found, "0"),
        abi::branch_eq(&skip),
    ]);
    emit_write_const(ctx, ESC_KITTY_PUSH_SYMBOL, ESC_KITTY_PUSH.len())?;
    emit_store_input_events(ctx, 1, vregs);
    ctx.instructions.push(abi::label(&skip));
    Ok(())
}

/// Append the byte in `b` to the input buffer — a key that arrived while
/// `term::on` waited for the terminal's replies. A full buffer drops it (the
/// buffer holds 4096 bytes and the wait is half a second of typing).
fn emit_kitty_keep_byte(
    ctx: &mut EmitCtx,
    input_state_offset: usize,
    b: &Reg,
    label: &str,
    vregs: &mut Vregs,
) {
    use crate::codegen::io::input::{
        INPUT_BUFFER_CAPACITY, INPUT_STATE_BUF_OFFSET, INPUT_STATE_LEN_OFFSET,
    };
    let full = format!("{label}_full");
    let r = vregs.next();
    let len: Reg = vregs.next().into();
    let cap: Reg = vregs.next().into();
    let at: Reg = vregs.next().into();
    crate::codegen::io::stdin::field_addr(
        r,
        ARENA_STATE_REGISTER,
        input_state_offset,
        ctx.instructions,
    );
    let r: Reg = r.into();
    ctx.instructions.extend([
        abi::load_u64(&len, &r, INPUT_STATE_LEN_OFFSET),
        abi::move_immediate(&cap, "Integer", &INPUT_BUFFER_CAPACITY.to_string()),
        abi::compare_registers(&len, &cap),
        abi::branch_ge(&full),
        abi::add_registers(&at, &r, &len),
        abi::store_u8(b, &at, INPUT_STATE_BUF_OFFSET),
        abi::add_immediate(&len, &len, 1),
        abi::store_u64(&len, &r, INPUT_STATE_LEN_OFFSET),
        abi::label(&full),
    ]);
}

/// Store `value` into the process-global "the terminal reports key events" word
/// the console readers consult.
fn emit_store_input_events(ctx: &mut EmitCtx, value: u64, vregs: &mut Vregs) {
    let addr: Reg = vregs.next().into();
    let word: Reg = vregs.next().into();
    push_symbol_address(
        ctx.symbol,
        INPUT_EVENTS_SYMBOL,
        &addr,
        ctx.instructions,
        ctx.relocations,
    );
    ctx.instructions.extend([
        abi::move_immediate(&word, "Integer", &value.to_string()),
        abi::store_u64(&word, &addr, 0),
    ]);
}

/// `term::off`'s half: pop the flags `term::on` pushed, if it pushed them.
pub(super) fn emit_kitty_disable(ctx: &mut EmitCtx, vregs: &mut Vregs) {
    let skip = format!("{}_kitty_off_skip", ctx.symbol);
    let addr: Reg = vregs.next().into();
    let t: Reg = vregs.next().into();
    push_symbol_address(
        ctx.symbol,
        INPUT_EVENTS_SYMBOL,
        &addr,
        ctx.instructions,
        ctx.relocations,
    );
    ctx.instructions.extend([
        abi::load_u64(&t, &addr, 0),
        abi::compare_immediate(&t, "0"),
        abi::branch_eq(&skip),
    ]);
    // The pop is best-effort like every term escape write (its result is not
    // an error); `emit_write_const` cannot fail at emit time on a known symbol,
    // but its signature can, so the error is carried rather than dropped.
    let _ = emit_write_const(ctx, ESC_KITTY_POP_SYMBOL, ESC_KITTY_POP.len());
    emit_store_input_events(ctx, 0, vregs);
    ctx.instructions.push(abi::label(&skip));
}

/// `io::readLine`'s window around a line read under `term::on` (bug-699
/// Phase 5): the line is read in the terminal's cooked mode, where a key must
/// arrive as the terminal's ordinary bytes — under the kitty protocol Return is
/// an escape sequence, so the line discipline would never see the end of the
/// line. `resume = false` pops the flags before the cooked restore; `resume =
/// true` pushes them again after raw mode is re-applied. The "reports events"
/// word is left as it is: `term::on` still owns the protocol, and it is back on
/// before the next key read. `preserve_result` as for
/// [`emit_mouse_tracking_window`].
pub(crate) fn emit_kitty_line_window(
    ctx: &mut EmitCtx,
    resume: bool,
    preserve_result: bool,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let tag = if resume { "resume" } else { "suspend" };
    let skip = format!("{}_kitty_{tag}_skip", ctx.symbol);
    let addr: Reg = vregs.next().into();
    let t: Reg = vregs.next().into();
    push_symbol_address(
        ctx.symbol,
        INPUT_EVENTS_SYMBOL,
        &addr,
        ctx.instructions,
        ctx.relocations,
    );
    ctx.instructions.extend([
        abi::load_u64(&t, &addr, 0),
        abi::compare_immediate(&t, "0"),
        abi::branch_eq(&skip),
    ]);
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
        emit_write_const(ctx, ESC_KITTY_PUSH_SYMBOL, ESC_KITTY_PUSH.len())?;
    } else {
        emit_write_const(ctx, ESC_KITTY_POP_SYMBOL, ESC_KITTY_POP.len())?;
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
