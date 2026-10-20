//! Stdin broadcast log (plan-15). The runtime owns fd 0, reads it in chunks into
//! one process-global append-only log, and every *subscribed* thread reads its own
//! independent cursor over that log — so each subscriber sees the whole stdin byte
//! stream from its subscription point and no byte is consumed out from under another
//! thread. A single-threaded program stays byte-identical: the compiler inserts an
//! implicit main-thread subscription (`_mfb_rt_stdin_subscribe`) at entry, so main's
//! cursor is 0 and it sees the entire stream, and the ~15 single-byte read sites in
//! `io_helpers.rs` route through `_mfb_rt_stdin_next_byte` instead of `read(0,…,1)`.
//!
//! Layout constants live in `error_constants.rs` (`STDIN_LOG_*`, `STDIN_BLOCK_*`,
//! `ARENA_STDIN_*`). Log blocks are `malloc`/`free`d (never per-arena) so a block
//! read on one thread and freed on another never races an arena free-list; the log
//! is the only new cross-thread shared state, guarded by its own mutex + condvar
//! (the same primitives the transfer queues use).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::VirtualRegister;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use std::collections::HashMap;

mod next_byte;
mod subscription;
pub(crate) use next_byte::lower_stdin_next_byte;
pub(crate) use subscription::{
    lower_stdin_recompute_base, lower_stdin_subscribe, lower_stdin_unsubscribe,
};
/// EINTR on every supported platform (see `fs_helpers_io::EINTR_ERRNO`).
const STDIN_EINTR_ERRNO: &str = "4";
/// `U64_MAX`, the "no EOF yet" sentinel for `STDIN_LOG_EOF_OFFSET`.
const U64_MAX_DECIMAL: &str = "18446744073709551615";

/// The single process-global broadcast-log data object (zero-initialized, writable).
/// Emitted whenever the module uses a stdin builtin.
pub(crate) fn stdin_log_data_object() -> CodeDataObject {
    CodeDataObject {
        symbol: STDIN_LOG_SYMBOL.to_string(),
        kind: "raw".to_string(),
        layout: "mfb.runtime.stdin_log.v1 { u8 bytes[size] }".to_string(),
        align: 16,
        size: STDIN_LOG_SIZE,
        value: "00".repeat(STDIN_LOG_SIZE),
    }
}

/// Load the address of the process-global `StdinLog` into `dst` (adrp/add-pageoff
/// with the standard data-address relocation pair).
pub(crate) fn push_log_address(
    from: &str,
    dst: impl Into<abi::Reg>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let dst: abi::Reg = dst.into();
    instructions.push(
        CodeInstruction::new("adrp")
            .field("dst", dst)
            .field("symbol", STDIN_LOG_SYMBOL),
    );
    instructions.push(
        CodeInstruction::new("add_pageoff")
            .field("dst", dst)
            .field("src", dst)
            .field("symbol", STDIN_LOG_SYMBOL),
    );
    relocations.extend([
        CodeRelocation {
            from: from.to_string(),
            to: STDIN_LOG_SYMBOL.to_string(),
            kind: RelocIntent::DataAddrHi,
            binding: "data".to_string(),
            library: None,
        },
        CodeRelocation {
            from: from.to_string(),
            to: STDIN_LOG_SYMBOL.to_string(),
            kind: RelocIntent::DataAddrLo,
            binding: "data".to_string(),
            library: None,
        },
    ]);
}

/// Materialize `base_register + offset` into `dst`. The stdin arena fields sit past
/// rv64's 12-bit `addi` immediate, so their address is computed in a register rather
/// than used as a load/store displacement (mirrors `current_error_slot_address`).
pub(crate) fn field_addr(
    dst: impl Into<abi::Reg>,
    base_register: impl Into<abi::Reg>,
    offset: usize,
    instructions: &mut Vec<CodeInstruction>,
) {
    let base_register: abi::Reg = base_register.into();
    let dst: abi::Reg = dst.into();
    instructions.push(abi::move_immediate(dst, "Integer", &offset.to_string()));
    instructions.push(abi::add_registers(dst, base_register, dst));
}

fn emit_libc(ctx: &mut EmitCtx, name: &str) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    // The broadcast log's growable buffer lives outside the arena, so it uses the
    // process heap; route malloc/free through the platform heap seam (Windows:
    // HeapAlloc/HeapFree). Its mutex/condvar primitives are pthread names; on
    // Windows those have no libc symbol, so route them through the same
    // pthread→Win32 (SRWLOCK/CONDITION_VARIABLE) seam the thread machinery uses
    // (plan-47-H). macOS/Linux keep the direct libc call and stay byte-identical.
    // plan-66-C.
    match name {
        "malloc" => {
            platform.emit_heap_alloc(symbol, platform_imports, ctx.instructions, ctx.relocations)
        }
        "free" => {
            platform.emit_heap_free(symbol, platform_imports, ctx.instructions, ctx.relocations)
        }
        _ if platform.family() == PlatformFamily::Windows => {
            crate::codegen::runtime::thread::emit_thread_external_call(ctx, name)
        }
        _ => platform.emit_external_call(
            name,
            symbol,
            platform_imports,
            ctx.instructions,
            ctx.relocations,
        ),
    }
}

/// Emit one stdin byte read through the broadcast log, replacing the per-byte
/// `read(0, sp+byte_offset, 1)` + EINTR guard at every stdin read site (plan-15
/// §4.3). Sets `x1 = sp + byte_offset` and calls `_mfb_rt_stdin_next_byte`, which
/// stores the byte there and returns `x0 = 1` (got a byte), `0` (EOF/shutdown), or
/// `-1` (a genuine OS read error). A negative return branches to `error_label`
/// (the caller's input-error handler); on `0`/`1` the `cmp x0, 0` flags are left
/// live so the caller's follow-on `branch_eq(<eof-or-truncated>)` fuses on every
/// backend (mirroring `emit_single_op_eintr_guard`'s resume re-compare). EINTR is
/// handled inside `_mfb_rt_stdin_next_byte`, so no caller-side retry label remains.
/// The negative-return scratch is minted from the caller's `vregs`, so it cannot
/// alias a caller temporary live across the read (bug-721).
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_stdin_next_byte(
    symbol: &str,
    byte_offset: usize,
    // A per-site-unique label base (the caller's retry/loop label); the same
    // `byte_offset` recurs across UTF-8 length branches, so it cannot key the label.
    site_label: &str,
    error_label: &str,
    invalid_context_label: &str,
    vregs: &mut Vregs,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let cont = format!("{site_label}_cont");
    let biased = vregs.next();
    instructions.extend([
        abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), byte_offset),
        abi::branch_link(STDIN_NEXT_BYTE_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, STDIN_NEXT_BYTE_SYMBOL));
    instructions.extend([
        // x0: 1 (got a byte), 0 (EOF), -1 (input error), -2 (not subscribed).
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ge(&cont),
        // Negative: distinguish -2 (ErrInvalidContext) from -1 (ErrInput) via +2.
        abi::add_immediate(&biased, abi::return_register(), 2),
        abi::compare_immediate(&biased, "0"),
        abi::branch_eq(invalid_context_label),
        abi::branch(error_label),
        abi::label(&cont),
        // Re-establish the `x0 vs 0` flags for the caller's follow-on branch_eq.
        abi::compare_immediate(abi::return_register(), "0"),
    ]);
}

/// Emit the log-aware readiness check a timed read runs before it waits
/// (plan-15 §4.4; `crate::codegen::io::input::reader::emit_read_wait`). Branches to
/// `ready_label` when the calling thread has a byte immediately available — the
/// arena-local buffer is non-empty, or (under the log mutex) its cursor is behind
/// `fill`, or it has reached the EOF offset (a read would return EOF at once) — and
/// otherwise falls through to `fallthrough_label` (the existing `poll(fd 0)` that
/// reports pending OS data without consuming it). Never blocks and never reads the
/// OS. The mutex is released on every exit. Needed because reads are served from
/// the log: leftover log bytes are invisible to `poll(fd 0)`.
///
/// The temporaries are minted from the caller's `vregs` (bug-721): a hand-numbered
/// name would alias whichever vreg the caller's counter minted with that number.
pub(crate) fn emit_stdin_poll_ready_check(
    ctx: &mut EmitCtx,
    vregs: &mut Vregs,
    ready_label: &str,
    fallthrough_label: &str,
) -> Result<(), String> {
    let addr = vregs.next();
    let cursor = vregs.next();
    let eof = vregs.next();
    let fill = vregs.next();
    let local_filled = vregs.next();
    let log = vregs.next();
    let local_pos = vregs.next();
    let subscriber = vregs.next();
    let regs = PollReadyRegs {
        addr: &addr,
        local_pos: &local_pos,
        local_filled: &local_filled,
        subscriber: &subscriber,
        log: &log,
        cursor: &cursor,
        fill: &fill,
        eof: &eof,
    };
    emit_stdin_poll_ready_check_with(ctx, &regs, ready_label, fallthrough_label)
}

/// The temporaries [`emit_stdin_poll_ready_check_with`] uses. Both callers mint
/// them from their own `Vregs`: the readers' timed wait through
/// [`emit_stdin_poll_ready_check`], and the mouse pump's escape wait (bug-669),
/// which runs the check in the MIDDLE of a read helper whose own temporaries are
/// live across it.
pub(crate) struct PollReadyRegs<'a> {
    pub(crate) addr: &'a VirtualRegister,
    pub(crate) local_pos: &'a VirtualRegister,
    pub(crate) local_filled: &'a VirtualRegister,
    pub(crate) subscriber: &'a VirtualRegister,
    pub(crate) log: &'a VirtualRegister,
    pub(crate) cursor: &'a VirtualRegister,
    pub(crate) fill: &'a VirtualRegister,
    pub(crate) eof: &'a VirtualRegister,
}

/// [`emit_stdin_poll_ready_check`] with caller-chosen temporaries.
pub(crate) fn emit_stdin_poll_ready_check_with(
    ctx: &mut EmitCtx,
    r: &PollReadyRegs,
    ready_label: &str,
    fallthrough_label: &str,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    // Labels are keyed on the caller's fall-through label, not the function: a
    // reader with a mouse pump runs this check twice — the timed wait and the
    // pump's escape wait — and two copies keyed on the symbol would collide.
    let l = |s: &str| format!("{fallthrough_label}_stdin_poll_{s}");
    // Local fast path: pos < filled => a byte is staged.
    field_addr(
        r.addr,
        ARENA_STATE_REGISTER,
        ARENA_STDIN_LOCAL_POS_OFFSET,
        ctx.instructions,
    );
    ctx.instructions.push(abi::load_u64(r.local_pos, r.addr, 0));
    field_addr(
        r.addr,
        ARENA_STATE_REGISTER,
        ARENA_STDIN_LOCAL_FILLED_OFFSET,
        ctx.instructions,
    );
    ctx.instructions.extend([
        abi::load_u64(r.local_filled, r.addr, 0),
        abi::compare_registers(r.local_filled, r.local_pos),
        abi::branch_hi(ready_label),
    ]);
    // Not subscribed => defer to the OS poll (byte-identical to pre-plan-15).
    field_addr(
        r.addr,
        ARENA_STATE_REGISTER,
        ARENA_STDIN_SUBSCRIBER_OFFSET,
        ctx.instructions,
    );
    ctx.instructions.extend([
        abi::load_u64(r.subscriber, r.addr, 0),
        abi::compare_immediate(r.subscriber, "0"),
        abi::branch_eq(fallthrough_label),
    ]);
    push_log_address(symbol, r.log, ctx.instructions, ctx.relocations);
    ctx.instructions
        .push(abi::move_register(abi::c_arg(0), r.log));
    emit_libc(ctx, "pthread_mutex_lock")?;
    push_log_address(symbol, r.log, ctx.instructions, ctx.relocations);
    field_addr(
        r.addr,
        ARENA_STATE_REGISTER,
        ARENA_STDIN_SUBSCRIBER_OFFSET,
        ctx.instructions,
    );
    ctx.instructions.extend([
        abi::load_u64(r.subscriber, r.addr, 0),
        abi::load_u64(r.cursor, r.subscriber, STDIN_SUBSCRIBER_CURSOR_OFFSET),
        abi::load_u64(r.fill, r.log, STDIN_LOG_FILL_OFFSET),
        // cursor < fill => bytes waiting.
        abi::compare_registers(r.fill, r.cursor),
        abi::branch_hi(&l("ready_unlock")),
        // cursor >= eofOffset => EOF is immediately observable.
        abi::load_u64(r.eof, r.log, STDIN_LOG_EOF_OFFSET),
        abi::compare_registers(r.eof, r.cursor),
        abi::branch_ls(&l("ready_unlock")),
        // Nothing in the log for us: unlock and defer to the OS poll.
        abi::move_register(abi::c_arg(0), r.log),
    ]);
    emit_libc(ctx, "pthread_mutex_unlock")?;
    ctx.instructions.push(abi::branch(fallthrough_label));
    ctx.instructions.push(abi::label(&l("ready_unlock")));
    push_log_address(symbol, r.log, ctx.instructions, ctx.relocations);
    ctx.instructions
        .push(abi::move_register(abi::c_arg(0), r.log));
    emit_libc(ctx, "pthread_mutex_unlock")?;
    ctx.instructions.push(abi::branch(ready_label));
    Ok(())
}
