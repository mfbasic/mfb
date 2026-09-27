//! The read engine every buffered `io` reader runs (bug-699 Fix Design layer 4).
//!
//! `io::readRaw`, `readScalar`, `readGrapheme`, `readKey`, `readLine` and `input`
//! each emit ONE copy of this engine into their own body and jump back into it
//! for every unit they need. One pass through it yields one of:
//!
//! - a decoded token (token mode) — the decoder's record at
//!   [`super::INPUT_STATE_TOKEN_OFFSET`], its bytes already consumed;
//! - a run of ill-formed UTF-8 the decoder dropped (token mode) — reported so a
//!   line reader can raise `ErrEncoding` (bug-699 Decision 9a), while the
//!   keystroke readers simply go round again (Decision 2);
//! - one raw byte (byte mode), in the token record's scalar field;
//!
//! or it branches to the caller's timeout / end-of-input / error labels.
//!
//! # Where bytes come from, and how long a read waits
//!
//! Bytes are taken from the input buffer first; only when the buffer cannot
//! answer is the source read — through the same [`emit_stdin_byte_read`] every
//! reader uses (the broadcast log or the window pipe, behind the mouse decoder),
//! one byte at a time, appended to the buffer. The engine emits exactly one such
//! read site, because the mouse decoder's labels are per function.
//!
//! Every source read first waits for input, until the call's deadline
//! ([`ReaderSlots::deadline`], from the `timeoutMs` argument by the language's
//! timeout convention): the log, then `poll` on standard input. A wait that runs
//! out leaves the buffer exactly as it was — a half-arrived key sequence stays
//! for the next read.
//!
//! When the decoder answers "need more" (a lone ESC: the Escape key, or the
//! start of a sequence? a base character: is a combining mark coming?), the
//! engine waits for the byte that decides it — on a keyboard at most
//! [`INPUT_DELAY_MS`] from the moment the front of the buffer started waiting,
//! then decodes with `FINAL`; on a pipe or a file for as long as it takes, since
//! the rest of the sequence is already on its way (bug-699 Decision 3). The
//! start of that wait is kept in the buffer region, so a program polling with
//! `timeoutMs = 0` sees a lone Escape once the delay has passed.

use super::buffer::emit_consume;
use super::decode::{
    FLAG_EVENTS, FLAG_FINAL, INPUT_DECODE_SYMBOL, STATUS_SKIP, STATUS_TOKEN, TOKEN_CONSUMED,
    TOKEN_STATUS, TOKEN_TEXT_SCALAR,
};
use super::{
    INPUT_BUFFER_CAPACITY, INPUT_DELAY_MS, INPUT_STATE_BUF_OFFSET, INPUT_STATE_LEN_OFFSET,
    INPUT_STATE_PENDING_OFFSET, INPUT_STATE_SOURCE_OFFSET, INPUT_STATE_TOKEN_OFFSET, SOURCE_DATA,
    SOURCE_KEYBOARD,
};
use crate::codegen::builtins::io::gen_read_family::{emit_stdin_byte_read, MousePump, ReadWait};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::VirtualRegister;
use crate::codegen::engine::util::Vregs;
use crate::codegen::error::constants::{ARENA_STATE_REGISTER, TIMEOUT_UNBOUNDED_SENTINEL};
use crate::codegen::io::mouse::clock::{emit_monotonic_nanos, MOUSE_CLOCK_SCRATCH_BYTES};
use crate::codegen::io::stdin::field_addr;
use crate::codegen::memory::data::push_symbol_address;
use crate::target::shared::abi;

/// Engine mode: decode one token.
pub(crate) const ENGINE_TOKEN: u64 = 0;
/// Engine mode: take one raw byte.
pub(crate) const ENGINE_BYTE: u64 = 1;

/// Outcome: a token is in the region's token record.
pub(crate) const OUT_TOKEN: u64 = 0;
/// Outcome: a run of ill-formed UTF-8 was dropped.
pub(crate) const OUT_INVALID: u64 = 1;
/// Outcome: one byte, in the token record's scalar field.
pub(crate) const OUT_BYTE: u64 = 2;

const NANOS_PER_MS: u64 = 1_000_000;
/// The longest `timeoutMs` honoured exactly: past it (about 31 years) the wait is
/// that long, which keeps `now + timeout` from overflowing.
const TIMEOUT_MS_CAP: u64 = 1_000_000_000_000;
const ESC: u64 = 0x1B;

/// The frame slots the engine owns in its reader's frame (sp-relative).
#[derive(Clone, Copy)]
pub(crate) struct ReaderSlots {
    /// The byte a source read lands in.
    pub(crate) byte: usize,
    /// [`MOUSE_CLOCK_SCRATCH_BYTES`] of clock scratch — also the mouse decoder's.
    pub(crate) clock: usize,
    /// A `pollfd` for the wait.
    pub(crate) pollfd: usize,
    /// The call's deadline, monotonic nanoseconds; `u64::MAX` = none.
    pub(crate) deadline: usize,
    /// The deadline of the wait in progress.
    pub(crate) wait: usize,
    /// [`FLAG_FINAL`] for the next decode, or 0.
    pub(crate) final_flag: usize,
    /// [`ENGINE_TOKEN`] or [`ENGINE_BYTE`].
    pub(crate) mode: usize,
    /// The decoder flags the reader asks for (`FLAG_CLUSTER`).
    pub(crate) flags: usize,
    /// Which deadline the wait in progress is for: 0 the call's, 1 the delay's.
    pub(crate) fill_kind: usize,
    /// The outcome (`OUT_*`) when the engine falls through.
    pub(crate) outcome: usize,
}

impl ReaderSlots {
    /// Bytes of frame the slots take, from `base`.
    pub(crate) const BYTES: usize = 8 + MOUSE_CLOCK_SCRATCH_BYTES + 8 * 8;

    pub(crate) fn at(base: usize) -> ReaderSlots {
        let clock = base + 8;
        let after_clock = clock + MOUSE_CLOCK_SCRATCH_BYTES;
        ReaderSlots {
            byte: base,
            clock,
            pollfd: after_clock,
            deadline: after_clock + 8,
            wait: after_clock + 16,
            final_flag: after_clock + 24,
            mode: after_clock + 32,
            flags: after_clock + 40,
            fill_kind: after_clock + 48,
            outcome: after_clock + 56,
        }
    }
}

/// What the engine is emitted for.
pub(crate) struct Engine<'a> {
    /// The input-buffer region's arena offset.
    pub(crate) state_offset: usize,
    pub(crate) app_mode: bool,
    /// The mouse-state region's arena offset, in a program that uses the mouse.
    pub(crate) mouse_state_offset: Option<usize>,
    pub(crate) slots: ReaderSlots,
    /// The process-global word `term::on` sets while the terminal reports key
    /// events (the kitty protocol), for a console build. An `--app` window
    /// always reports them.
    pub(crate) events_symbol: Option<&'a str>,
}

/// Where the engine leaves when it cannot produce a unit.
pub(crate) struct EngineLabels<'a> {
    pub(crate) timeout: &'a str,
    pub(crate) eof: &'a str,
    pub(crate) input_error: &'a str,
    pub(crate) invalid_context: &'a str,
}

/// The label a reader jumps to for its next unit. The engine then falls through
/// at [`engine_done_label`] with [`ReaderSlots::outcome`] set.
pub(crate) fn engine_next_label(symbol: &str) -> String {
    format!("{symbol}_in_next")
}

/// See [`engine_next_label`].
pub(crate) fn engine_done_label(symbol: &str) -> String {
    format!("{symbol}_in_done")
}

fn region(dst: VirtualRegister, state_offset: usize, ctx: &mut EmitCtx) {
    field_addr(dst, ARENA_STATE_REGISTER, state_offset, ctx.instructions);
}

fn store_slot_imm(ctx: &mut EmitCtx, vregs: &mut Vregs, slot: usize, value: u64) {
    let v = vregs.next();
    ctx.instructions.extend([
        abi::move_immediate(v, "Integer", &value.to_string()),
        abi::store_u64(v, abi::stack_pointer(), slot),
    ]);
}

/// `dst = u64::MAX`, without a negative immediate.
fn move_max(dst: VirtualRegister, ctx: &mut EmitCtx) {
    ctx.instructions.push(abi::bitwise_not(dst, abi::ZERO));
}

/// The reader prologue: the call's deadline from `timeout` (already validated:
/// the unbounded sentinel, or `0..`), the FINAL flag cleared, and — in a console
/// build — what standard input is, asked once per thread.
pub(crate) fn emit_reader_setup(
    e: &Engine,
    timeout: VirtualRegister,
    ctx: &mut EmitCtx,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let s = e.slots;
    store_slot_imm(ctx, vregs, s.final_flag, 0);
    // deadline = sentinel ? u64::MAX : now + min(timeout, cap) * 1e6
    let unbounded = format!("{symbol}_in_unbounded");
    let deadline_set = format!("{symbol}_in_deadline_set");
    let capped = format!("{symbol}_in_timeout_capped");
    let t = vregs.next();
    let sentinel = vregs.next();
    let cap = vregs.next();
    ctx.instructions.extend([
        abi::store_u64(timeout, abi::stack_pointer(), s.deadline),
        abi::load_u64(t, abi::stack_pointer(), s.deadline),
        abi::move_immediate(sentinel, "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
        abi::compare_registers(t, sentinel),
        abi::branch_eq(&unbounded),
        abi::move_immediate(cap, "Integer", &TIMEOUT_MS_CAP.to_string()),
        abi::compare_registers(t, cap),
        abi::branch_ls(&capped),
        abi::move_register(t, cap),
        abi::label(&capped),
        abi::store_u64(t, abi::stack_pointer(), s.deadline),
    ]);
    let now = vregs.next();
    emit_monotonic_nanos(now, s.clock, ctx, vregs)?;
    let t = vregs.next();
    let scale = vregs.next();
    ctx.instructions.extend([
        abi::load_u64(t, abi::stack_pointer(), s.deadline),
        abi::move_immediate(scale, "Integer", &NANOS_PER_MS.to_string()),
        abi::multiply_registers(t, t, scale),
        abi::add_registers(t, t, now),
        abi::store_u64(t, abi::stack_pointer(), s.deadline),
        abi::branch(&deadline_set),
        abi::label(&unbounded),
    ]);
    let max = vregs.next();
    move_max(max, ctx);
    ctx.instructions.extend([
        abi::store_u64(max, abi::stack_pointer(), s.deadline),
        abi::label(&deadline_set),
    ]);
    // What standard input is: an `--app` window is a keyboard; a console asks
    // `isatty(0)` once and remembers it.
    if !e.app_mode {
        let known = format!("{symbol}_in_source_known");
        let is_tty = format!("{symbol}_in_source_tty");
        let store = format!("{symbol}_in_source_store");
        let r = vregs.next();
        let kind = vregs.next();
        region(r, e.state_offset, ctx);
        ctx.instructions.extend([
            abi::load_u64(kind, r, INPUT_STATE_SOURCE_OFFSET),
            abi::compare_immediate(kind, "0"),
            abi::branch_ne(&known),
            abi::move_immediate(abi::return_register(), "Integer", "0"),
        ]);
        ctx.platform.emit_is_terminal(
            symbol,
            ctx.platform_imports,
            ctx.instructions,
            ctx.relocations,
        )?;
        let r = vregs.next();
        let kind = vregs.next();
        ctx.instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_gt(&is_tty),
            abi::move_immediate(kind, "Integer", &SOURCE_DATA.to_string()),
            abi::branch(&store),
            abi::label(&is_tty),
            abi::move_immediate(kind, "Integer", &SOURCE_KEYBOARD.to_string()),
            abi::label(&store),
        ]);
        region(r, e.state_offset, ctx);
        ctx.instructions.extend([
            abi::store_u64(kind, r, INPUT_STATE_SOURCE_OFFSET),
            abi::label(&known),
        ]);
    }
    Ok(())
}

/// Emit the engine (see the module doc). The reader sets [`ReaderSlots::mode`]
/// (and, for tokens, [`ReaderSlots::flags`]) and jumps to
/// [`engine_next_label`]; the engine falls through at [`engine_done_label`].
pub(crate) fn emit_engine(
    e: &Engine,
    labels: &EngineLabels,
    ctx: &mut EmitCtx,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let s = e.slots;
    let l = |name: &str| format!("{symbol}_in_{name}");
    let next = engine_next_label(symbol);
    let done = engine_done_label(symbol);
    let take_byte = l("take_byte");
    let fill_deadline = l("fill_deadline");
    let fill = l("fill");
    let fill_eof = l("fill_eof");
    let fill_timeout = l("fill_timeout");
    let need_more = l("need_more");
    let skip = l("skip");

    ctx.instructions.push(abi::label(&next));
    let r = vregs.next();
    let len = vregs.next();
    let mode = vregs.next();
    region(r, e.state_offset, ctx);
    ctx.instructions.extend([
        abi::load_u64(len, r, INPUT_STATE_LEN_OFFSET),
        abi::compare_immediate(len, "0"),
        abi::branch_eq(&fill_deadline),
        abi::load_u64(mode, abi::stack_pointer(), s.mode),
        abi::compare_immediate(mode, &ENGINE_BYTE.to_string()),
        abi::branch_eq(&take_byte),
    ]);

    // decode(buf, len, flags | events | final, token)
    let flags = vregs.next();
    let t = vregs.next();
    ctx.instructions.extend([
        abi::load_u64(flags, abi::stack_pointer(), s.flags),
        abi::load_u64(t, abi::stack_pointer(), s.final_flag),
        abi::or_registers(flags, flags, t),
    ]);
    if e.app_mode {
        ctx.instructions.extend([
            abi::move_immediate(t, "Integer", &FLAG_EVENTS.to_string()),
            abi::or_registers(flags, flags, t),
        ]);
    } else if let Some(events_symbol) = e.events_symbol {
        let no_events = l("no_events");
        let addr = vregs.next();
        push_symbol_address(
            symbol,
            events_symbol,
            addr,
            ctx.instructions,
            ctx.relocations,
        );
        ctx.instructions.extend([
            abi::load_u64(t, addr, 0),
            abi::compare_immediate(t, "0"),
            abi::branch_eq(&no_events),
            abi::move_immediate(t, "Integer", &FLAG_EVENTS.to_string()),
            abi::or_registers(flags, flags, t),
            abi::label(&no_events),
        ]);
    }
    let r = vregs.next();
    let len = vregs.next();
    region(r, e.state_offset, ctx);
    ctx.instructions.extend([
        abi::load_u64(len, r, INPUT_STATE_LEN_OFFSET),
        abi::add_immediate(abi::c_arg(0), r, INPUT_STATE_BUF_OFFSET),
        abi::move_register(abi::c_arg(1), len),
        abi::move_register(abi::c_arg(2), flags),
        abi::add_immediate(abi::c_arg(3), r, INPUT_STATE_TOKEN_OFFSET),
        abi::branch_link(INPUT_DECODE_SYMBOL),
    ]);
    ctx.relocations
        .push(internal_branch(symbol, INPUT_DECODE_SYMBOL));
    // The status from the record rather than a register: the decoder's result
    // register and the caller's C-call registers are different ones on x86-64.
    let r = vregs.next();
    let status = vregs.next();
    region(r, e.state_offset, ctx);
    ctx.instructions.extend([
        abi::load_u64(status, r, INPUT_STATE_TOKEN_OFFSET + TOKEN_STATUS),
        abi::compare_immediate(status, &STATUS_SKIP.to_string()),
        abi::branch_eq(&skip),
        abi::compare_immediate(status, &STATUS_TOKEN.to_string()),
        abi::branch_ne(&need_more),
    ]);
    // A token: consume it.
    let consumed = vregs.next();
    ctx.instructions.push(abi::load_u64(
        consumed,
        r,
        INPUT_STATE_TOKEN_OFFSET + TOKEN_CONSUMED,
    ));
    emit_consume(consumed, e.state_offset, ctx, vregs);
    store_slot_imm(ctx, vregs, s.final_flag, 0);
    if let Some(events_symbol) = e.events_symbol {
        emit_interrupt_key(events_symbol, e.state_offset, &next, ctx, vregs)?;
    }
    store_slot_imm(ctx, vregs, s.outcome, OUT_TOKEN);
    ctx.instructions.push(abi::branch(&done));

    // Dropped bytes: an unrecognised escape sequence is nothing at all, so go
    // round again; ill-formed UTF-8 is reported.
    ctx.instructions.push(abi::label(&skip));
    let r = vregs.next();
    let first = vregs.next();
    let consumed = vregs.next();
    region(r, e.state_offset, ctx);
    ctx.instructions.extend([
        abi::load_u8(first, r, INPUT_STATE_BUF_OFFSET),
        abi::load_u64(consumed, r, INPUT_STATE_TOKEN_OFFSET + TOKEN_CONSUMED),
        abi::store_u64(first, abi::stack_pointer(), s.byte),
    ]);
    emit_consume(consumed, e.state_offset, ctx, vregs);
    store_slot_imm(ctx, vregs, s.final_flag, 0);
    let first = vregs.next();
    ctx.instructions.extend([
        abi::load_u64(first, abi::stack_pointer(), s.byte),
        abi::compare_immediate(first, &ESC.to_string()),
        abi::branch_eq(&next),
    ]);
    store_slot_imm(ctx, vregs, s.outcome, OUT_INVALID);
    ctx.instructions.push(abi::branch(&done));

    // A raw byte.
    ctx.instructions.push(abi::label(&take_byte));
    let r = vregs.next();
    let b = vregs.next();
    let one = vregs.next();
    region(r, e.state_offset, ctx);
    ctx.instructions.extend([
        abi::load_u8(b, r, INPUT_STATE_BUF_OFFSET),
        abi::store_u64(b, r, INPUT_STATE_TOKEN_OFFSET + TOKEN_TEXT_SCALAR),
        abi::move_immediate(one, "Integer", "1"),
    ]);
    emit_consume(one, e.state_offset, ctx, vregs);
    store_slot_imm(ctx, vregs, s.outcome, OUT_BYTE);
    ctx.instructions.push(abi::branch(&done));

    // "Need more": wait for the byte that decides it — on a keyboard, until the
    // front has waited the delay; on data, for as long as it takes.
    ctx.instructions.push(abi::label(&need_more));
    let keyboard = l("keyboard");
    if !e.app_mode {
        let r = vregs.next();
        let kind = vregs.next();
        region(r, e.state_offset, ctx);
        ctx.instructions.extend([
            abi::load_u64(kind, r, INPUT_STATE_SOURCE_OFFSET),
            abi::compare_immediate(kind, &SOURCE_KEYBOARD.to_string()),
            abi::branch_ne(&fill_deadline),
        ]);
    }
    ctx.instructions.push(abi::label(&keyboard));
    let now = vregs.next();
    emit_monotonic_nanos(now, s.clock, ctx, vregs)?;
    let r = vregs.next();
    let pending = vregs.next();
    let pending_set = l("pending_set");
    region(r, e.state_offset, ctx);
    ctx.instructions.extend([
        abi::load_u64(pending, r, INPUT_STATE_PENDING_OFFSET),
        abi::compare_immediate(pending, "0"),
        abi::branch_ne(&pending_set),
        abi::move_register(pending, now),
        abi::store_u64(pending, r, INPUT_STATE_PENDING_OFFSET),
        abi::label(&pending_set),
    ]);
    // until = pending + delay; past it, decide with what has arrived.
    let until = vregs.next();
    let delay = vregs.next();
    let decide = l("decide");
    ctx.instructions.extend([
        abi::move_immediate(
            delay,
            "Integer",
            &(INPUT_DELAY_MS * NANOS_PER_MS).to_string(),
        ),
        abi::add_registers(until, pending, delay),
        // now >= until (unsigned): the vocabulary has no `hs`.
        abi::compare_registers(until, now),
        abi::branch_ls(&decide),
    ]);
    // wait = min(deadline, until)
    let deadline = vregs.next();
    let use_until = l("use_until");
    let wait_set = l("wait_set");
    ctx.instructions.extend([
        abi::load_u64(deadline, abi::stack_pointer(), s.deadline),
        abi::compare_registers(until, deadline),
        abi::branch_lo(&use_until),
        abi::store_u64(deadline, abi::stack_pointer(), s.wait),
        abi::branch(&wait_set),
        abi::label(&use_until),
        abi::store_u64(until, abi::stack_pointer(), s.wait),
        abi::label(&wait_set),
    ]);
    store_slot_imm(ctx, vregs, s.fill_kind, 1);
    ctx.instructions.push(abi::branch(&fill));

    ctx.instructions.push(abi::label(&decide));
    store_slot_imm(ctx, vregs, s.final_flag, FLAG_FINAL);
    ctx.instructions.push(abi::branch(&next));

    // Nothing buffered (or data still arriving): wait until the call's deadline.
    ctx.instructions.push(abi::label(&fill_deadline));
    let deadline = vregs.next();
    ctx.instructions.extend([
        abi::load_u64(deadline, abi::stack_pointer(), s.deadline),
        abi::store_u64(deadline, abi::stack_pointer(), s.wait),
    ]);
    store_slot_imm(ctx, vregs, s.fill_kind, 0);

    // The one source read.
    ctx.instructions.push(abi::label(&fill));
    let mouse = e.mouse_state_offset.map(|state_offset| MousePump {
        state_offset,
        clock_scratch: s.clock,
    });
    let wait = ReadWait {
        deadline_slot: s.wait,
        clock_scratch: s.clock,
        pollfd_slot: s.pollfd,
        timeout_label: fill_timeout.clone(),
    };
    emit_stdin_byte_read(
        ctx,
        e.app_mode,
        s.byte,
        &l("read_retry"),
        &l("read_resume"),
        labels.input_error,
        labels.invalid_context,
        mouse,
        Some(&wait),
        vregs,
    )?;
    ctx.instructions.push(abi::branch_eq(&fill_eof));
    // Append the byte. A token never needs more than a few hundred bytes of
    // look-ahead and byte mode reads only into an empty buffer, so the buffer
    // cannot be full here; if it were, the input is not what any reader can
    // take, and the read fails rather than overwrite the region.
    let r = vregs.next();
    let len = vregs.next();
    let cap = vregs.next();
    let at = vregs.next();
    let b = vregs.next();
    region(r, e.state_offset, ctx);
    ctx.instructions.extend([
        abi::load_u64(len, r, INPUT_STATE_LEN_OFFSET),
        abi::move_immediate(cap, "Integer", &INPUT_BUFFER_CAPACITY.to_string()),
        abi::compare_registers(len, cap),
        abi::branch_ge(labels.input_error),
        abi::add_registers(at, r, len),
        abi::load_u8(b, abi::stack_pointer(), s.byte),
        abi::store_u8(b, at, INPUT_STATE_BUF_OFFSET),
        abi::add_immediate(len, len, 1),
        abi::store_u64(len, r, INPUT_STATE_LEN_OFFSET),
        abi::branch(&next),
    ]);

    // End of input: what is buffered is complete as it is.
    ctx.instructions.push(abi::label(&fill_eof));
    let r = vregs.next();
    let len = vregs.next();
    let mode = vregs.next();
    region(r, e.state_offset, ctx);
    ctx.instructions.extend([
        abi::load_u64(len, r, INPUT_STATE_LEN_OFFSET),
        abi::compare_immediate(len, "0"),
        abi::branch_eq(labels.eof),
        abi::load_u64(mode, abi::stack_pointer(), s.mode),
        abi::compare_immediate(mode, &ENGINE_BYTE.to_string()),
        abi::branch_eq(&next),
    ]);
    store_slot_imm(ctx, vregs, s.final_flag, FLAG_FINAL);
    ctx.instructions.push(abi::branch(&next));

    // The wait ran out: the delay's (decide now) or the call's (a timeout).
    ctx.instructions.push(abi::label(&fill_timeout));
    let kind = vregs.next();
    ctx.instructions.extend([
        abi::load_u64(kind, abi::stack_pointer(), s.fill_kind),
        abi::compare_immediate(kind, "1"),
        abi::branch_ne(labels.timeout),
    ]);
    let now = vregs.next();
    emit_monotonic_nanos(now, s.clock, ctx, vregs)?;
    let r = vregs.next();
    let until = vregs.next();
    let delay = vregs.next();
    region(r, e.state_offset, ctx);
    ctx.instructions.extend([
        abi::load_u64(until, r, INPUT_STATE_PENDING_OFFSET),
        abi::move_immediate(
            delay,
            "Integer",
            &(INPUT_DELAY_MS * NANOS_PER_MS).to_string(),
        ),
        abi::add_registers(until, until, delay),
        abi::compare_registers(until, now),
        abi::branch_ls(&decide),
        abi::branch(labels.timeout),
        abi::label(&done),
    ]);
    Ok(())
}

/// Ctrl+C under the kitty keyboard protocol (bug-699 Phase 5). A terminal in
/// that mode sends Ctrl+C as a key (`ESC [ 99 ; 5 u`) instead of the `0x03` the
/// line discipline turns into SIGINT, so a program in `term::on` could no longer
/// be interrupted from the keyboard. The console reader does what the line
/// discipline would have done: a Ctrl+C press or repeat with no other modifier,
/// while the protocol is on, raises SIGINT and is not returned. Emitted after
/// the token is consumed, so a program that survives the signal reads on past
/// it (`next`).
///
/// **POSIX only**: `term::on` never finds the protocol on a Windows console in
/// practice, and `raise` is not one of its imports.
fn emit_interrupt_key(
    events_symbol: &str,
    state_offset: usize,
    next: &str,
    ctx: &mut EmitCtx,
    vregs: &mut Vregs,
) -> Result<(), String> {
    use super::decode::{TOKEN_EVENT, TOKEN_GRAPHEME, TOKEN_GRAPHEME_LEN, TOKEN_KEY, TOKEN_MODS};
    use super::keys::{KeyEvent, KeyName, MOD_ALT, MOD_CTRL, MOD_META, MOD_SHIFT};
    use crate::codegen::engine::types::PlatformFamily;
    const SIGINT: &str = "2";
    if ctx.platform.family() == PlatformFamily::Windows {
        return Ok(());
    }
    let symbol = ctx.symbol;
    let not_interrupt = format!("{symbol}_in_not_interrupt");
    let addr = vregs.next();
    let t = vregs.next();
    push_symbol_address(
        symbol,
        events_symbol,
        addr,
        ctx.instructions,
        ctx.relocations,
    );
    ctx.instructions.extend([
        abi::load_u64(t, addr, 0),
        abi::compare_immediate(t, "0"),
        abi::branch_eq(&not_interrupt),
    ]);
    let r = vregs.next();
    let v = vregs.next();
    let mask = vregs.next();
    region(r, state_offset, ctx);
    let token = INPUT_STATE_TOKEN_OFFSET;
    ctx.instructions.extend([
        abi::load_u64(v, r, token + TOKEN_KEY),
        abi::compare_immediate(v, &(KeyName::Unicode as u64).to_string()),
        abi::branch_ne(&not_interrupt),
        abi::load_u64(v, r, token + TOKEN_EVENT),
        abi::compare_immediate(v, &(KeyEvent::Release as u64).to_string()),
        abi::branch_eq(&not_interrupt),
        abi::load_u64(v, r, token + TOKEN_MODS),
        abi::move_immediate(
            mask,
            "Integer",
            &(MOD_SHIFT | MOD_ALT | MOD_CTRL | MOD_META).to_string(),
        ),
        abi::and_registers(v, v, mask),
        abi::compare_immediate(v, &MOD_CTRL.to_string()),
        abi::branch_ne(&not_interrupt),
        abi::load_u64(v, r, token + TOKEN_GRAPHEME_LEN),
        abi::compare_immediate(v, "1"),
        abi::branch_ne(&not_interrupt),
        abi::load_u8(v, r, token + TOKEN_GRAPHEME),
        abi::compare_immediate(v, &u64::from(b'c').to_string()),
        abi::branch_ne(&not_interrupt),
        abi::move_immediate(abi::c_arg(0), "Integer", SIGINT),
    ]);
    ctx.platform.emit_external_call(
        "raise",
        symbol,
        ctx.platform_imports,
        ctx.instructions,
        ctx.relocations,
    )?;
    ctx.instructions
        .extend([abi::branch(next), abi::label(&not_interrupt)]);
    Ok(())
}

/// Emit the wait before a source read: until the deadline in
/// `wait.deadline_slot` for input to be ready — the broadcast log first in a
/// console build (bytes staged there are invisible to `poll`), then `poll` on
/// standard input. Branches to `read_label` when a read will not block, to
/// `wait.timeout_label` when the deadline passes first. A deadline of
/// `u64::MAX` reads at once (blocking).
pub(crate) fn emit_read_wait(
    wait: &ReadWait,
    app_mode: bool,
    read_label: &str,
    ctx: &mut EmitCtx,
    vregs: &mut Vregs,
) -> Result<(), String> {
    use crate::codegen::io::stdin::emit_stdin_poll_ready_check;
    use crate::codegen::os::syscall::emit_eintr_retry_or_error;
    const POLLIN_PACKED_FD0: &str = "4294967296";
    let symbol = ctx.symbol;
    let l = |name: &str| format!("{symbol}_in_wait_{name}");
    let head = l("head");
    let os = l("os");
    let positive = l("positive");
    let clamp_done = l("clamp_done");
    ctx.instructions.push(abi::label(&head));
    let d = vregs.next();
    let max = vregs.next();
    move_max(max, ctx);
    ctx.instructions.extend([
        abi::load_u64(d, abi::stack_pointer(), wait.deadline_slot),
        abi::compare_registers(d, max),
        abi::branch_eq(read_label),
    ]);
    if !app_mode {
        emit_stdin_poll_ready_check(ctx, vregs, read_label, &os)?;
    }
    ctx.instructions.push(abi::label(&os));
    let now = vregs.next();
    emit_monotonic_nanos(now, wait.clock_scratch, ctx, vregs)?;
    // ms = deadline > now ? ceil((deadline - now) / 1e6) clamped to INT_MAX : 0
    let d = vregs.next();
    let ms = vregs.next();
    let t = vregs.next();
    ctx.instructions.extend([
        abi::load_u64(d, abi::stack_pointer(), wait.deadline_slot),
        abi::move_immediate(ms, "Integer", "0"),
        abi::compare_registers(d, now),
        abi::branch_hi(&positive),
        abi::branch(&clamp_done),
        abi::label(&positive),
        abi::subtract_registers(ms, d, now),
        abi::move_immediate(t, "Integer", &(NANOS_PER_MS - 1).to_string()),
        abi::add_registers(ms, ms, t),
        abi::move_immediate(t, "Integer", &NANOS_PER_MS.to_string()),
        abi::unsigned_divide_registers(ms, ms, t),
        abi::move_immediate(t, "Integer", "2147483647"),
        abi::compare_registers(ms, t),
        abi::branch_ls(&clamp_done),
        abi::move_register(ms, t),
        abi::label(&clamp_done),
    ]);
    let pollfd = vregs.next();
    ctx.instructions.extend([
        abi::move_immediate(pollfd, "Integer", POLLIN_PACKED_FD0),
        abi::store_u64(pollfd, abi::stack_pointer(), wait.pollfd_slot),
        abi::move_register(abi::c_arg(2), ms),
        abi::add_immediate(
            abi::return_register(),
            abi::stack_pointer(),
            wait.pollfd_slot,
        ),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;
    platform.emit_poll_input(symbol, platform_imports, ctx.instructions, ctx.relocations)?;
    ctx.instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_gt(read_label),
        abi::branch_eq(&wait.timeout_label),
    ]);
    // A signal interrupted the wait: wait again. Any other failure is left to
    // the read, which reports it.
    emit_eintr_retry_or_error(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        abi::return_register(),
        false,
        &head,
        read_label,
    )?;
    Ok(())
}
