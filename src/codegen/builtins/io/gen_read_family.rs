//! Shared clean-room stdin-read machinery for the `io` reader members: the
//! per-byte source read (`emit_stdin_byte_read`, with its mouse pump and its
//! deadline wait), which the reader engine
//! (`crate::codegen::io::input::reader`) and `term::on`'s keyboard-protocol probe
//! both go through.

use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::VirtualRegister;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::MOUSE_MODE_SYMBOL;
use crate::codegen::io::mouse::decode as mouse_decode;
use crate::codegen::io::stdin::*;
use crate::codegen::memory::data::push_symbol_address;
use crate::codegen::os::syscall::*;
use crate::target::shared::abi;
/// Emit one stdin byte read for a read helper, choosing the source by mode. In
/// console mode (`!app_mode`) the byte comes from the stdin broadcast log
/// (`_mfb_rt_stdin_next_byte`, plan-15). In app mode stdin is the window input
/// pipe, not fd 0, so the log is not built — keep the direct per-byte
/// `read(0,…,1)` + EINTR guard. Both paths push `retry_label` (the loop/retry head)
/// and leave the `x0 vs 0` flags live for the caller's follow-on `branch_eq`.
///
/// # The mouse pump (plan-94-B §3a)
///
/// This one function is where the decoder belongs, and the reason is structural:
/// it is the **single choke point over both byte sources**. Every `io::` read
/// helper reaches its bytes through here, and the app-vs-console split happens
/// *inside* it — so a decoder placed here serves the console broadcast log and the
/// window input pipe with one implementation. Wrapping `_mfb_rt_stdin_next_byte`
/// instead (the pre-migration draft's plan) would have decoded console reads only
/// and left all three app backends undecoded, which is precisely what plan-94-C/D/E
/// depend on not being true: they inject SGR bytes into that pipe and expect this
/// decoder to eat them.
///
/// `mouse` is `Some` only for a program that uses `enableMouse`/`pollMouse`, and
/// the pump is emitted **only then**. That compile-time gate is what makes the
/// plan's byte-identity claim literally true rather than approximately: a program
/// that never mentions the mouse gets exactly the instruction stream it got before
/// plan-94-B — not that stream plus a mode-word test. The *runtime* mode check
/// still exists, for a mouse program that has not called `enableMouse(TRUE)` yet.
///
/// The pump wraps the read in a loop, because a byte it swallows must not be
/// returned as the caller's byte: on a consumed byte it goes back and reads
/// another, so the caller's contract ("one byte, or EOF") is unchanged.
/// [`crate::codegen::io::mouse::decode`] states the ordering rule the two halves
/// implement.

/// What the mouse pump needs from its caller: where the mouse-state region is, and
/// a 16-byte scratch slot the caller owns for the monotonic clock read.
///
/// Passed as a struct so a caller cannot get the two offsets the wrong way round —
/// both are plain `usize` and one indexes the arena while the other indexes the
/// stack.
#[derive(Clone, Copy)]
pub(crate) struct MousePump {
    /// Arena byte offset of the mouse-state region.
    pub(crate) state_offset: usize,
    /// sp-relative byte offset of the caller's clock scratch.
    pub(crate) clock_scratch: usize,
}

/// A deadline-bounded wait before each source read (bug-699): the reader's
/// frame slots the wait uses, and where it goes when the deadline passes. The
/// wait runs before EVERY source read the call makes — including the ones the
/// mouse pump issues after swallowing a report — so no read blocks past the
/// deadline.
pub(crate) struct ReadWait {
    /// sp-relative slot holding the deadline (monotonic ns; `u64::MAX` = none).
    pub(crate) deadline_slot: usize,
    /// sp-relative 16-byte clock scratch.
    pub(crate) clock_scratch: usize,
    /// sp-relative 8-byte `pollfd`.
    pub(crate) pollfd_slot: usize,
    pub(crate) timeout_label: String,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_stdin_byte_read(
    ctx: &mut EmitCtx,
    app_mode: bool,
    byte_offset: usize,
    retry_label: &str,
    resume_label: &str,
    input_error: &str,
    invalid_context: &str,
    mouse: Option<MousePump>,
    wait: Option<&ReadWait>,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    // The caller's loop head. It sits ABOVE the mouse pump's owed-byte drain when
    // there is a pump, so a caller that loops back for another byte drains a byte
    // the decoder owes (the `x` after a flushed `ESC`) before it reads afresh —
    // a fresh read would re-enter the decoder mid-flush (bug-699: `a ESC x b`
    // read back as `a ESC ESC ESC` under `term::enableMouse`). Placed there only
    // when a pump exists, so every other read keeps its exact instruction stream.
    let head_first = mouse.is_some();
    if head_first {
        ctx.instructions.push(abi::label(retry_label));
    }

    // The pump's outer loop head: a byte the decoder swallowed sends control back
    // here to read another, so the caller still receives exactly one byte.
    let pump_head = format!("{symbol}_mouse_pump_head");
    let pump_done = format!("{symbol}_mouse_pump_done");
    // The pump's registers come from the caller's `Vregs`, so they cannot collide
    // with a caller temporary that is live across the read.
    let pump = mouse.map(|pump| {
        let v = PumpVregs::mint(vregs);
        (pump, v)
    });
    if let Some((pump, v)) = &pump {
        emit_pump_prologue(
            ctx,
            pump,
            v,
            byte_offset,
            app_mode,
            &pump_head,
            &pump_done,
            vregs,
        )?;
    }

    let wait_read = format!("{retry_label}_wait_read");
    if app_mode {
        if !head_first {
            ctx.instructions.push(abi::label(retry_label));
        }
        if let Some(wait) = wait {
            crate::codegen::io::input::reader::emit_read_wait(
                wait, app_mode, &wait_read, ctx, vregs,
            )?;
            ctx.instructions.push(abi::label(&wait_read));
        }
        ctx.instructions.extend([
            // fd 0 (stdin) goes in ARG[0], the read() first-arg register that
            // emit_read_file reads it from. Using return_register() worked only by
            // accident on aarch64 (x0 == ARG[0]); on Win64 return_register() is rax,
            // not ARG[0]=rcx, so emit_read_file read garbage as the fd (plan-66-J-4).
            abi::move_immediate(abi::c_arg(0), "Integer", "0"),
            abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), byte_offset),
            abi::move_immediate(abi::c_arg(2), "Integer", "1"),
        ]);
        platform.emit_read_file(symbol, platform_imports, ctx.instructions, ctx.relocations)?;
        ctx.instructions
            .push(abi::compare_immediate(abi::return_register(), "0"));
        emit_single_op_eintr_guard(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: ctx.instructions,
                relocations: ctx.relocations,
            },
            retry_label,
            resume_label,
            input_error,
        )?;
    } else {
        if !head_first {
            ctx.instructions.push(abi::label(retry_label));
        }
        if let Some(wait) = wait {
            crate::codegen::io::input::reader::emit_read_wait(
                wait, app_mode, &wait_read, ctx, vregs,
            )?;
            ctx.instructions.push(abi::label(&wait_read));
        }
        emit_stdin_next_byte(
            symbol,
            byte_offset,
            retry_label,
            input_error,
            invalid_context,
            vregs,
            ctx.instructions,
            ctx.relocations,
        );
    }
    if let Some((pump, v)) = &pump {
        emit_pump_epilogue(ctx, pump, v, byte_offset, &pump_head, &pump_done, vregs)?;
    }
    Ok(())
}

/// The pump's registers, minted from the CALLER's `Vregs` so they share one
/// namespace with the caller's own temporaries: these values are live across the
/// read and across the decoder, so a name collision with a caller's temporary
/// would be a silent wrong-byte bug rather than a compile error.
struct PumpVregs {
    saved_count: VirtualRegister,
    byte: VirtualRegister,
    action: VirtualRegister,
    out_byte: VirtualRegister,
    mode: VirtualRegister,
    mode_addr: VirtualRegister,
    have: VirtualRegister,
}

impl PumpVregs {
    fn mint(vregs: &mut Vregs) -> Self {
        PumpVregs {
            saved_count: vregs.next(),
            byte: vregs.next(),
            action: vregs.next(),
            out_byte: vregs.next(),
            mode: vregs.next(),
            mode_addr: vregs.next(),
            have: vregs.next(),
        }
    }
}

/// Load the process-global mouse-mode word into `v.mode`.
fn emit_load_mouse_mode(ctx: &mut EmitCtx, v: &PumpVregs) {
    let symbol = ctx.symbol;
    push_symbol_address(
        symbol,
        MOUSE_MODE_SYMBOL,
        &v.mode_addr,
        ctx.instructions,
        ctx.relocations,
    );
    ctx.instructions
        .push(abi::load_u64(&v.mode, &v.mode_addr, 0));
}

/// Before the read: hand back any byte still owed from a flushed escape prefix,
/// rather than reading a new one.
///
/// This ordering is the decoder's contract (see `io::mouse::decode`): owed bytes
/// come first, and only when none are owed is the OS read issued. Getting it the
/// other way round would interleave a replayed `ESC [ Z` with freshly typed input.
#[allow(clippy::too_many_arguments)]
fn emit_pump_prologue(
    ctx: &mut EmitCtx,
    pump: &MousePump,
    v: &PumpVregs,
    byte_offset: usize,
    app_mode: bool,
    pump_head: &str,
    pump_done: &str,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let no_drain = format!("{symbol}_mouse_pump_no_drain");

    ctx.instructions.push(abi::label(pump_head));
    emit_load_mouse_mode(ctx, v);
    ctx.instructions.push(abi::compare_immediate(&v.mode, "0"));
    ctx.instructions.push(abi::branch_eq(&no_drain));
    let escape_wait = format!("{symbol}_mouse_esc_wait");
    mouse_decode::emit_drain_pending(&v.have, &v.out_byte, pump.state_offset, ctx, vregs);
    ctx.instructions.push(abi::compare_immediate(&v.have, "0"));
    ctx.instructions.push(abi::branch_eq(&escape_wait));
    // A replayed byte looks exactly like a freshly read one to the caller: it lands
    // in the same slot, and the synthesized count of 1 reproduces the read's own
    // "got a byte" answer so the caller's EOF test is unchanged.
    ctx.instructions.extend([
        abi::store_u8(&v.out_byte, abi::stack_pointer(), byte_offset),
        abi::move_immediate(&v.saved_count, "Integer", "1"),
        abi::branch(pump_done),
        abi::label(&escape_wait),
    ]);
    emit_pump_escape_wait(ctx, pump, app_mode, pump_head, &no_drain, vregs)?;
    ctx.instructions.push(abi::label(&no_drain));
    Ok(())
}

/// Before a fresh read, settle a held escape prefix (bug-669).
///
/// The decoder is sitting on a prefix — most often a lone ESC — that only the
/// next byte can classify. A blocking read would wait for that byte for as long
/// as the user takes to press another key, so a program could never see the Esc
/// key on its own. Instead wait at most [`mouse_decode::ESCAPE_DELAY_MS`] for
/// input: if some arrives, fall through to `read_label` and let the decoder
/// classify it as usual; if none does, the prefix was keystrokes, so release it
/// and go back to `pump_head`, whose drain hands the bytes over in order.
///
/// Readiness is asked exactly as the readers' timed wait asks it: the stdin broadcast log
/// first in console mode (bytes staged there are invisible to `poll(fd 0)`), then
/// the OS. The pollfd lives in the caller's clock scratch, which is free here —
/// the clock is only read inside the decoder, after the read.
fn emit_pump_escape_wait(
    ctx: &mut EmitCtx,
    pump: &MousePump,
    app_mode: bool,
    pump_head: &str,
    read_label: &str,
    vregs: &mut Vregs,
) -> Result<(), String> {
    const POLLIN_PACKED_FD0: &str = "4294967296";
    let symbol = ctx.symbol;
    let l = |s: &str| format!("{symbol}_mouse_esc_{s}");
    let os_poll = l("os_poll");
    let quiet = l("quiet");

    mouse_decode::emit_branch_unless_prefix_held(read_label, pump.state_offset, ctx, vregs);
    if !app_mode {
        let names: Vec<VirtualRegister> = (0..8).map(|_| vregs.next()).collect();
        let regs = PollReadyRegs {
            addr: &names[0],
            local_pos: &names[1],
            local_filled: &names[2],
            subscriber: &names[3],
            log: &names[4],
            cursor: &names[5],
            fill: &names[6],
            eof: &names[7],
        };
        emit_stdin_poll_ready_check_with(ctx, &regs, read_label, &os_poll)?;
    }
    let pollfd = vregs.next();
    ctx.instructions.extend([
        abi::label(&os_poll),
        abi::move_immediate(&pollfd, "Integer", POLLIN_PACKED_FD0),
        abi::store_u64(&pollfd, abi::stack_pointer(), pump.clock_scratch),
        abi::add_immediate(
            abi::return_register(),
            abi::stack_pointer(),
            pump.clock_scratch,
        ),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
        abi::move_immediate(
            abi::c_arg(2),
            "Integer",
            &mouse_decode::ESCAPE_DELAY_MS.to_string(),
        ),
    ]);
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;
    platform.emit_poll_input(symbol, platform_imports, ctx.instructions, ctx.relocations)?;
    ctx.instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_gt(read_label),
        abi::branch_eq(&quiet),
    ]);
    // A signal interrupted the wait: wait again rather than guess. Any other poll
    // failure is left for the read itself to report.
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
        &os_poll,
        read_label,
    )?;
    ctx.instructions.push(abi::label(&quiet));
    mouse_decode::emit_release_prefix(pump.state_offset, ctx, vregs);
    ctx.instructions.push(abi::branch(pump_head));
    Ok(())
}

/// After the read: decode the byte, and loop for another if the decoder consumed
/// it.
fn emit_pump_epilogue(
    ctx: &mut EmitCtx,
    pump: &MousePump,
    v: &PumpVregs,
    byte_offset: usize,
    pump_head: &str,
    pump_done: &str,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let deliver = format!("{symbol}_mouse_pump_deliver");

    // The read's count must survive the decoder, which calls out to the clock and
    // clobbers the result bank.
    ctx.instructions
        .push(abi::move_register(&v.saved_count, abi::return_register()));
    emit_load_mouse_mode(ctx, v);
    ctx.instructions.push(abi::compare_immediate(&v.mode, "0"));
    ctx.instructions.push(abi::branch_eq(&deliver));
    // EOF and errors are the caller's to interpret, not the decoder's: a 0 or
    // negative count means no byte was read, so there is nothing to decode.
    //
    // Except that EOF also settles a held escape prefix — no report can follow the
    // end of input — so a prefix still buffered then is released first and the
    // EOF comes back on the next read, after its bytes (bug-669).
    let decode = format!("{symbol}_mouse_pump_decode");
    ctx.instructions
        .push(abi::compare_immediate(&v.saved_count, "0"));
    ctx.instructions.push(abi::branch_gt(&decode));
    ctx.instructions.push(abi::branch_lt(&deliver));
    mouse_decode::emit_branch_unless_prefix_held(&deliver, pump.state_offset, ctx, vregs);
    mouse_decode::emit_release_prefix(pump.state_offset, ctx, vregs);
    ctx.instructions.push(abi::branch(pump_head));
    ctx.instructions.push(abi::label(&decode));

    ctx.instructions
        .push(abi::load_u8(&v.byte, abi::stack_pointer(), byte_offset));
    mouse_decode::emit_decode_byte(
        &v.byte,
        &v.action,
        &v.out_byte,
        pump.state_offset,
        pump.clock_scratch,
        ctx,
        vregs,
    )?;
    ctx.instructions.extend([
        abi::compare_immediate(&v.action, &mouse_decode::DECODE_PASS.to_string()),
        // Buffered or consumed as an event: the program gets nothing for this
        // byte, so go around and read another. This is what makes a mouse report
        // invisible to the readers instead of arriving as escape garbage.
        abi::branch_ne(pump_head),
        // Pass: the byte to deliver may not be the byte just read — a flush hands
        // back the head of the buffered prefix instead.
        abi::store_u8(&v.out_byte, abi::stack_pointer(), byte_offset),
        abi::label(&deliver),
        abi::label(pump_done),
        // Re-establish the `count vs 0` flags the caller's follow-on branch reads.
        abi::compare_immediate(&v.saved_count, "0"),
        abi::move_register(abi::return_register(), &v.saved_count),
        abi::compare_immediate(abi::return_register(), "0"),
    ]);
    Ok(())
}

/// The frame slots [`emit_decode_pending_input`] uses (sp-relative), owned by the
/// `pollMouse` body that calls it.
#[derive(Clone, Copy)]
pub(crate) struct PendingInputSlots {
    /// The byte a source read lands in.
    pub(crate) byte: usize,
    /// [`crate::codegen::io::mouse::clock::MOUSE_CLOCK_SCRATCH_BYTES`] of clock
    /// scratch — also the mouse decoder's, and the dequeue's.
    pub(crate) clock: usize,
    /// A `pollfd` for the wait.
    pub(crate) pollfd: usize,
    /// The wait's deadline (always "now": the drain never blocks).
    pub(crate) deadline: usize,
}

impl PendingInputSlots {
    /// Bytes of frame the slots take, from `base`.
    pub(crate) const BYTES: usize =
        8 + crate::codegen::io::mouse::clock::MOUSE_CLOCK_SCRATCH_BYTES + 8 + 8;

    pub(crate) fn at(base: usize) -> PendingInputSlots {
        let clock = base + 8;
        let after_clock = clock + crate::codegen::io::mouse::clock::MOUSE_CLOCK_SCRATCH_BYTES;
        PendingInputSlots {
            byte: base,
            clock,
            pollfd: after_clock,
            deadline: after_clock + 8,
        }
    }
}

/// Where a `pollMouse` finds pending input, and where it keeps the bytes that
/// are not mouse reports.
#[derive(Clone, Copy)]
pub(crate) struct PendingInput {
    pub(crate) app_mode: bool,
    /// The input-buffer region (bug-699), in a program whose `io` readers read
    /// through it.
    pub(crate) input_state_offset: Option<usize>,
    pub(crate) slots: PendingInputSlots,
}

/// bug-1015: decode every byte of input that is already waiting, without
/// blocking, so a `pollMouse` sees the reports the surface has delivered whether
/// or not the program ever reads standard input.
///
/// The mouse decoder runs only on the source read ([`emit_stdin_byte_read`]), so
/// a program that polls the mouse and never reads keys used to decode nothing:
/// the window's (or terminal's) SGR reports sat unread in front of the decoder,
/// and `pollMouse` answered `MouseKind.None` forever, on every backend. This runs
/// that SAME source read — the same mouse pump, the same wait — with a deadline
/// of "now", until nothing more is ready:
///
/// - a report is decoded and enqueued by the pump, which then reads on;
/// - every other byte the pump delivers (a key, a flushed non-report prefix) is
///   appended to the input buffer, in order, so the program's next `io` read
///   takes it exactly as if it had read it itself. The buffer is checked for
///   room BEFORE the read, so a full buffer stops the drain with the byte still
///   unread rather than dropping it. A program with no input buffer has no `io`
///   reader at all, so there is no later read to keep its keystrokes for, and they
///   are dropped;
/// - end of input, a read error, or a thread with no stdin subscription stops the
///   drain silently: the next `io` read meets the condition itself.
///
/// Skipped while mouse reporting is off (the pump would decode nothing), and in a
/// console build without the input buffer: there the program has no stdin reader,
/// so no broadcast log or subscription exists to read through, and its main thread
/// is not subscribed (plan-15). An `--app` window always has its input pipe on
/// fd 0.
pub(crate) fn emit_decode_pending_input(
    ctx: &mut EmitCtx,
    pending: &PendingInput,
    mouse_state_offset: usize,
    vregs: &mut Vregs,
) -> Result<(), String> {
    use crate::codegen::io::input::{
        INPUT_BUFFER_CAPACITY, INPUT_STATE_BUF_OFFSET, INPUT_STATE_LEN_OFFSET,
    };
    if !pending.app_mode && pending.input_state_offset.is_none() {
        return Ok(());
    }
    let symbol = ctx.symbol;
    let l = |name: &str| format!("{symbol}_pending_{name}");
    let head = l("head");
    let done = l("done");
    let s = pending.slots;

    // Mouse reporting off: nothing to decode.
    let mode_addr = vregs.next();
    let mode = vregs.next();
    push_symbol_address(
        symbol,
        MOUSE_MODE_SYMBOL,
        &mode_addr,
        ctx.instructions,
        ctx.relocations,
    );
    let zero = vregs.next();
    ctx.instructions.extend([
        abi::load_u64(&mode, &mode_addr, 0),
        abi::compare_immediate(&mode, "0"),
        abi::branch_eq(&done),
        // Deadline 0: every wait is a non-blocking readiness check.
        abi::move_immediate(&zero, "Integer", "0"),
        abi::store_u64(&zero, abi::stack_pointer(), s.deadline),
        abi::label(&head),
    ]);
    if let Some(input_state_offset) = pending.input_state_offset {
        let r = vregs.next();
        let len = vregs.next();
        let cap = vregs.next();
        crate::codegen::io::stdin::field_addr(
            &r,
            crate::codegen::error::constants::ARENA_STATE_REGISTER,
            input_state_offset,
            ctx.instructions,
        );
        ctx.instructions.extend([
            abi::load_u64(&len, &r, INPUT_STATE_LEN_OFFSET),
            abi::move_immediate(&cap, "Integer", &INPUT_BUFFER_CAPACITY.to_string()),
            abi::compare_registers(&len, &cap),
            abi::branch_ge(&done),
        ]);
    }
    let wait = ReadWait {
        deadline_slot: s.deadline,
        clock_scratch: s.clock,
        pollfd_slot: s.pollfd,
        timeout_label: done.clone(),
    };
    emit_stdin_byte_read(
        ctx,
        pending.app_mode,
        s.byte,
        &l("read_retry"),
        &l("read_resume"),
        &done,
        &done,
        Some(MousePump {
            state_offset: mouse_state_offset,
            clock_scratch: s.clock,
        }),
        Some(&wait),
        vregs,
    )?;
    ctx.instructions.push(abi::branch_eq(&done));
    if let Some(input_state_offset) = pending.input_state_offset {
        let r = vregs.next();
        let len = vregs.next();
        let at = vregs.next();
        let b = vregs.next();
        crate::codegen::io::stdin::field_addr(
            &r,
            crate::codegen::error::constants::ARENA_STATE_REGISTER,
            input_state_offset,
            ctx.instructions,
        );
        ctx.instructions.extend([
            abi::load_u64(&len, &r, INPUT_STATE_LEN_OFFSET),
            abi::add_registers(&at, &r, &len),
            abi::load_u8(&b, abi::stack_pointer(), s.byte),
            abi::store_u8(&b, &at, INPUT_STATE_BUF_OFFSET),
            abi::add_immediate(&len, &len, 1),
            abi::store_u64(&len, &r, INPUT_STATE_LEN_OFFSET),
        ]);
    }
    ctx.instructions
        .extend([abi::branch(&head), abi::label(&done)]);
    Ok(())
}
