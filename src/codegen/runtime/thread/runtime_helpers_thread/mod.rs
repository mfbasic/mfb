// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::VirtualRegister;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::Vregs;
use crate::codegen::error::constants::*;
use crate::codegen::memory::data::*;
use crate::codegen::runtime::thread::*;
use crate::target::shared::abi;
use std::collections::HashMap;

mod handle;
mod queue;
mod sleep;
pub(crate) use handle::*;
pub(crate) use queue::*;
pub(crate) use sleep::*;

/// The `(instructions, relocations, stack_size)` a `thread` OS-seam body emits before
/// the `abi_function` wrapper finalizes it — the successor to the self-finalized
/// `HelperResult`/`HelperBody` (see `fs`'s `FsBodyParts`). `stack_size` is the explicit
/// sp-relative locals region the body reserves; the wrapper seeds the `entry` label and
/// passes `stack_size` to `finalize_vreg_body_with_locals`, byte-identical to the body's
/// former self-finalize.
pub(crate) type ThreadBodyParts = (Vec<CodeInstruction>, Vec<CodeRelocation>, usize);

/// The scratch vregs of one thread-runtime function: minted ONCE, at the top of the
/// function, from that function's own [`Vregs`], and handed by reference to every
/// shared sub-emitter it inlines (`emit_thread_deadline`, `emit_queue_wait`,
/// `emit_close_queue_locked`, …). A sub-emitter never mints its own: a fresh `Vregs`
/// restarts at `%v0` and would alias the caller's vregs (bug-721 — a hand-numbered
/// `"%v9"` in a function that also minted was the Schannel connect's double free).
///
/// The fields keep the numbers of the hand-numbered literals they replaced, and are
/// minted in that order, so each function names exactly the vregs it named before, in
/// the same relative order — the allocator's tie-break (`linear_scan`: interval start,
/// then vreg id) sees the same input, and the emitted machine code is unchanged.
pub(crate) struct ThreadVregs {
    pub(crate) v8: VirtualRegister,
    pub(crate) v9: VirtualRegister,
    pub(crate) v10: VirtualRegister,
    pub(crate) v11: VirtualRegister,
    pub(crate) v12: VirtualRegister,
    pub(crate) v13: VirtualRegister,
    pub(crate) v14: VirtualRegister,
    pub(crate) v15: VirtualRegister,
}

impl ThreadVregs {
    pub(crate) fn mint(vregs: &mut Vregs) -> Self {
        ThreadVregs {
            v8: vregs.next(),
            v9: vregs.next(),
            v10: vregs.next(),
            v11: vregs.next(),
            v12: vregs.next(),
            v13: vregs.next(),
            v14: vregs.next(),
            v15: vregs.next(),
        }
    }
}

pub(crate) enum ThreadSimpleOp {
    IsRunning,
    WaitFor,
    Cancel,
    Drop,
    Poll,
}

pub(crate) fn emit_thread_deadline(
    ctx: &mut EmitCtx,
    r: &ThreadVregs,
    timeout_stack_offset: usize,
    timespec_stack_offset: usize,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let done = format!("{symbol}_deadline_done_{timespec_stack_offset}");
    let nsec_ok = format!("{symbol}_deadline_nsec_ok_{timespec_stack_offset}");
    ctx.instructions.extend([
        abi::load_u64(&r.v9, abi::stack_pointer(), timeout_stack_offset),
        abi::compare_immediate(&r.v9, "0"),
        abi::branch_le(&done),
        abi::move_immediate(abi::c_arg(0), "Integer", "0"),
        abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), timespec_stack_offset),
    ]);
    emit_thread_external_call(ctx, "clock_gettime")?;
    ctx.instructions.extend([
        abi::load_u64(&r.v9, abi::stack_pointer(), timeout_stack_offset),
        abi::move_immediate(&r.v10, "Integer", "1000"),
        abi::signed_divide_registers(&r.v11, &r.v9, &r.v10),
        abi::multiply_subtract_registers(&r.v12, &r.v11, &r.v10, &r.v9),
        abi::move_immediate(&r.v13, "Integer", "1000000"),
        abi::multiply_registers(&r.v12, &r.v12, &r.v13),
        abi::load_u64(&r.v14, abi::stack_pointer(), timespec_stack_offset),
        abi::add_registers(&r.v14, &r.v14, &r.v11),
        abi::load_u64(&r.v15, abi::stack_pointer(), timespec_stack_offset + 8),
        abi::add_registers(&r.v15, &r.v15, &r.v12),
        abi::move_immediate(&r.v13, "Integer", "1000000000"),
        abi::compare_registers(&r.v15, &r.v13),
        abi::branch_lt(&nsec_ok),
        abi::subtract_registers(&r.v15, &r.v15, &r.v13),
        abi::add_immediate(&r.v14, &r.v14, 1),
        abi::label(&nsec_ok),
        abi::store_u64(&r.v14, abi::stack_pointer(), timespec_stack_offset),
        abi::store_u64(&r.v15, abi::stack_pointer(), timespec_stack_offset + 8),
        abi::label(&done),
    ]);
    Ok(())
}

/// The byte size of a worker's arena-state block: the arena state, the entry's seed
/// scratch word and the program's writable globals (bug-369). `thread::start`
/// allocates it from the process heap at this size; the block release frees it.
pub(crate) fn worker_arena_state_size(arena_global_slots: usize) -> usize {
    ENTRY_GLOBALS_OFFSET + arena_global_slots * 8
}

// ---------------------------------------------------------------------------
// Block-lock primitives. Every one reads the control block from the frame word
// `block` (an `arena_free`, a lock or a wait clobbers every caller-saved register,
// so nothing is kept in a register across one).
// ---------------------------------------------------------------------------

/// `pthread_mutex_lock` / `pthread_mutex_unlock` the block lock.
pub(crate) fn emit_block_lock(ctx: &mut EmitCtx, block: usize) -> Result<(), String> {
    emit_block_lock_call(ctx, block, "pthread_mutex_lock")
}

pub(crate) fn emit_block_unlock(ctx: &mut EmitCtx, block: usize) -> Result<(), String> {
    emit_block_lock_call(ctx, block, "pthread_mutex_unlock")
}

/// `pthread_cond_<call>` on the condvar at `queue + cond` of the block in `block`.
fn emit_queue_cond(
    ctx: &mut EmitCtx,
    block: usize,
    queue: usize,
    cond: usize,
    call: &str,
) -> Result<(), String> {
    ctx.instructions.extend([
        abi::load_u64(abi::c_arg(0), abi::stack_pointer(), block),
        abi::add_immediate(abi::c_arg(0), abi::c_arg(0), queue + cond),
    ]);
    emit_thread_external_call(ctx, call)
}

/// Wait on the condvar at `queue + cond` with the block lock (which the caller holds):
/// indefinitely, or until the absolute deadline in the frame's `timespec` slot. The
/// timed form leaves the `pthread_cond_timedwait` return in `c_return(0)`.
fn emit_queue_wait(
    ctx: &mut EmitCtx,
    r: &ThreadVregs,
    block: usize,
    queue: usize,
    cond: usize,
    timespec: Option<usize>,
) -> Result<(), String> {
    ctx.instructions.extend([
        abi::load_u64(&r.v9, abi::stack_pointer(), block),
        abi::add_immediate(abi::c_arg(0), &r.v9, queue + cond),
        abi::add_immediate(abi::c_arg(1), &r.v9, CONTROL_OFFSET_LOCK),
    ]);
    match timespec {
        Some(timespec) => {
            ctx.instructions.push(abi::add_immediate(
                abi::c_arg(2),
                abi::stack_pointer(),
                timespec,
            ));
            emit_thread_external_call(ctx, "pthread_cond_timedwait")
        }
        None => emit_thread_external_call(ctx, "pthread_cond_wait"),
    }
}

/// Mark `queue` closed and wake everyone waiting on it (both directions), so a
/// blocked reader or writer re-checks its predicate. The caller holds the block lock.
fn emit_close_queue_locked(
    ctx: &mut EmitCtx,
    r: &ThreadVregs,
    block: usize,
    queue: usize,
) -> Result<(), String> {
    ctx.instructions.extend([
        abi::load_u64(&r.v8, abi::stack_pointer(), block),
        abi::move_immediate(&r.v9, "Integer", "1"),
        abi::store_u64(&r.v9, &r.v8, queue + THREAD_QUEUE_CLOSED_OFFSET),
    ]);
    emit_queue_cond(
        ctx,
        block,
        queue,
        THREAD_QUEUE_NOT_EMPTY_OFFSET,
        "pthread_cond_broadcast",
    )?;
    emit_queue_cond(
        ctx,
        block,
        queue,
        THREAD_QUEUE_NOT_FULL_OFFSET,
        "pthread_cond_broadcast",
    )
}

/// Wake everyone waiting on either condvar of `queue` without closing it. The caller
/// holds the block lock.
fn emit_wake_queue_locked(ctx: &mut EmitCtx, block: usize, queue: usize) -> Result<(), String> {
    emit_queue_cond(
        ctx,
        block,
        queue,
        THREAD_QUEUE_NOT_EMPTY_OFFSET,
        "pthread_cond_broadcast",
    )?;
    emit_queue_cond(
        ctx,
        block,
        queue,
        THREAD_QUEUE_NOT_FULL_OFFSET,
        "pthread_cond_broadcast",
    )
}

/// Close both resource-plane queues so anyone parked in `thread::transfer` /
/// `thread::accept` re-checks CANCELLED/CLOSED and unblocks (bug-205: cancel and drop
/// once woke only the data plane, so a worker parked in a blocking `accept` never
/// observed the cancel). The caller holds the block lock.
fn emit_close_resource_queues_locked(
    ctx: &mut EmitCtx,
    r: &ThreadVregs,
    block: usize,
) -> Result<(), String> {
    for queue in [
        CONTROL_OFFSET_RESOURCE_INBOUND_QUEUE,
        CONTROL_OFFSET_RESOURCE_OUTBOUND_QUEUE,
    ] {
        emit_close_queue_locked(ctx, r, block, queue)?;
    }
    Ok(())
}

/// Drop one of the block's two references (plan-156-A §4.4) and, when it was the
/// last, free the block. `block` is the frame word holding the block.
///
/// The count only changes under the block lock, so exactly one side sees it reach
/// zero. A handle-side caller that sees zero joins a worker it has not yet joined or
/// detached (the worker has taken its own reference away, so all that is left for it
/// is to unlock and return) before the block goes away.
fn emit_release_reference(
    ctx: &mut EmitCtx,
    r: &ThreadVregs,
    block: usize,
    scratch: usize,
    join_first: bool,
) -> Result<(), String> {
    let site = ctx.instructions.len();
    let kept = format!("{}_release_{site}_kept", ctx.symbol);
    let joined = format!("{}_release_{site}_joined", ctx.symbol);
    emit_block_lock(ctx, block)?;
    ctx.instructions.extend([
        abi::load_u64(&r.v8, abi::stack_pointer(), block),
        abi::load_u64(&r.v9, &r.v8, CONTROL_OFFSET_REFS),
        abi::subtract_immediate(&r.v9, &r.v9, 1),
        abi::store_u64(&r.v9, &r.v8, CONTROL_OFFSET_REFS),
        abi::store_u64(&r.v9, abi::stack_pointer(), scratch),
    ]);
    emit_block_unlock(ctx, block)?;
    ctx.instructions.extend([
        abi::load_u64(&r.v9, abi::stack_pointer(), scratch),
        abi::compare_immediate(&r.v9, "0"),
        abi::branch_ne(&kept),
    ]);
    if join_first {
        ctx.instructions.extend([
            abi::load_u64(&r.v8, abi::stack_pointer(), block),
            abi::load_u64(abi::c_arg(0), &r.v8, CONTROL_OFFSET_OS_HANDLE),
            abi::compare_immediate(abi::c_arg(0), "0"),
            abi::branch_eq(&joined),
            abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        ]);
        emit_thread_external_call(ctx, "pthread_join")?;
        ctx.instructions.extend([
            abi::load_u64(&r.v8, abi::stack_pointer(), block),
            abi::store_u64(abi::ZERO, &r.v8, CONTROL_OFFSET_OS_HANDLE),
            abi::label(&joined),
        ]);
    }
    ctx.instructions.extend([
        abi::load_u64(abi::c_arg(0), abi::stack_pointer(), block),
        abi::branch_link(THREAD_BLOCK_RELEASE_SYMBOL),
    ]);
    ctx.relocations
        .push(internal_branch(ctx.symbol, THREAD_BLOCK_RELEASE_SYMBOL));
    ctx.instructions.push(abi::label(&kept));
    Ok(())
}
