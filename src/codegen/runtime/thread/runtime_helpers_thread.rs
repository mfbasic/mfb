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

/// The five parent-view ops that need no queue of their own: `isRunning`, `waitFor`,
/// `cancel`, `poll`, and the hidden `thread.drop` a parent view's scope end runs.
///
/// plan-156-C: each takes the parent VIEW and reaches the control block through it
/// ([`emit_control_block_of`]); a closed view answers `ErrResourceClosed` before any of
/// them touches the block. A view is closed only by its own drop, so while it is open
/// the block's state is `RUNNING` or `COMPLETED` — never `CLOSED`, which only that drop
/// writes.
pub(crate) fn simple_thread_handle_helper(
    symbol: &str,
    op: ThreadSimpleOp,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<ThreadBodyParts, String> {
    const FRAME_SIZE: usize = 64;
    // The control block, once the view has been checked.
    const HANDLE_OFFSET: usize = 8;
    const VALUE_OFFSET: usize = 16;
    const TAG_OFFSET: usize = 24;
    const ERROR_OFFSET: usize = 32;
    // WaitFor only: origin ErrorLoc of a propagated worker error (0 otherwise).
    const SOURCE_OFFSET: usize = 40;
    // Drop only: the release count left after this side's release.
    const REFS_OFFSET: usize = 48;
    // Drop only: the view itself, which the drop closes.
    const VIEW_OFFSET: usize = 56;

    let view_closed = format!("{symbol}_view_closed");
    let mut vregs = Vregs::new();
    let r = &ThreadVregs::mint(&mut vregs);
    let mut instructions = Vec::new();
    let mut relocations = Vec::new();
    let mut ctx = EmitCtx {
        symbol,
        platform_imports,
        platform,
        instructions: &mut instructions,
        relocations: &mut relocations,
    };
    if matches!(op, ThreadSimpleOp::Drop) {
        ctx.instructions.push(abi::store_u64(
            abi::c_arg(0),
            abi::stack_pointer(),
            VIEW_OFFSET,
        ));
    }
    emit_control_block_of(ctx.instructions, &view_closed, &r.v9);
    ctx.instructions.push(abi::store_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        HANDLE_OFFSET,
    ));
    match op {
        ThreadSimpleOp::IsRunning => {
            let running = format!("{symbol}_running");
            let done = format!("{symbol}_done");
            emit_block_lock(&mut ctx, HANDLE_OFFSET)?;
            ctx.instructions.extend([
                abi::load_u64(&r.v8, abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64(&r.v9, &r.v8, CONTROL_OFFSET_STATE),
                abi::store_u64(&r.v9, abi::stack_pointer(), VALUE_OFFSET),
            ]);
            emit_block_unlock(&mut ctx, HANDLE_OFFSET)?;
            ctx.instructions.extend([
                abi::load_u64(&r.v9, abi::stack_pointer(), VALUE_OFFSET),
                abi::compare_immediate(&r.v9, THREAD_STATE_RUNNING),
                abi::branch_eq(&running),
                abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
                abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
                abi::branch(&done),
                abi::label(&running),
                abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
                abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
                abi::label(&done),
            ]);
        }
        // plan-156-C: repeatable. Every call waits for `COMPLETED` and answers the stored
        // result — the call site copies it out of the block arena each time — and none
        // closes anything. The first to see the worker done joins it; the zeroed OS
        // handle tells every later call (and the drop) it is joined.
        ThreadSimpleOp::WaitFor => {
            let loop_label = format!("{symbol}_wait_loop");
            let result_ready = format!("{symbol}_result_ready");
            let joined = format!("{symbol}_joined");
            emit_block_lock(&mut ctx, HANDLE_OFFSET)?;
            ctx.instructions.extend([
                abi::label(&loop_label),
                abi::load_u64(&r.v8, abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64(&r.v9, &r.v8, CONTROL_OFFSET_STATE),
                abi::compare_immediate(&r.v9, THREAD_STATE_COMPLETED),
                abi::branch_eq(&result_ready),
            ]);
            // `_mfb_rt_thread_finish` broadcasts every condvar of the block as it
            // publishes `COMPLETED`; the outbound not-empty one is the one waited on.
            emit_queue_wait(
                &mut ctx,
                r,
                HANDLE_OFFSET,
                CONTROL_OFFSET_OUTBOUND_QUEUE,
                THREAD_QUEUE_NOT_EMPTY_OFFSET,
                None,
            )?;
            ctx.instructions.extend([
                abi::branch(&loop_label),
                abi::label(&result_ready),
                abi::load_u64(&r.v8, abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64(&r.v9, &r.v8, CONTROL_OFFSET_RESULT_ERROR),
                abi::store_u64(&r.v9, abi::stack_pointer(), ERROR_OFFSET),
                abi::load_u64(&r.v9, &r.v8, CONTROL_OFFSET_RESULT_VALUE),
                abi::store_u64(&r.v9, abi::stack_pointer(), VALUE_OFFSET),
                abi::load_u64(&r.v9, &r.v8, CONTROL_OFFSET_RESULT_TAG),
                abi::store_u64(&r.v9, abi::stack_pointer(), TAG_OFFSET),
                abi::load_u64(&r.v9, &r.v8, CONTROL_OFFSET_RESULT_SOURCE),
                abi::store_u64(&r.v9, abi::stack_pointer(), SOURCE_OFFSET),
            ]);
            emit_block_unlock(&mut ctx, HANDLE_OFFSET)?;
            // bug-622: join, not detach. `COMPLETED` is published under the block lock
            // but the worker still unlocks it and returns after that, so the join is the
            // one point this side knows the worker is gone. It is prompt: nothing is left
            // for the worker to do but return. Only this parent thread reads or writes
            // the OS handle while the view is open, so the unlocked test is sound.
            ctx.instructions.extend([
                abi::load_u64(&r.v8, abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64(abi::c_arg(0), &r.v8, CONTROL_OFFSET_OS_HANDLE),
                abi::compare_immediate(abi::c_arg(0), "0"),
                abi::branch_eq(&joined),
                abi::move_immediate(abi::c_arg(1), "Integer", "0"),
            ]);
            emit_thread_external_call(&mut ctx, "pthread_join")?;
            ctx.instructions.extend([
                abi::load_u64(&r.v8, abi::stack_pointer(), HANDLE_OFFSET),
                abi::store_u64(abi::ZERO, &r.v8, CONTROL_OFFSET_OS_HANDLE),
                abi::label(&joined),
                abi::load_u64(
                    RESULT_ERROR_MESSAGE_REGISTER,
                    abi::stack_pointer(),
                    ERROR_OFFSET,
                ),
                abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
                abi::load_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TAG_OFFSET),
                abi::load_u64(
                    RESULT_ERROR_SOURCE_REGISTER,
                    abi::stack_pointer(),
                    SOURCE_OFFSET,
                ),
            ]);
        }
        // Cancel: set CANCELLED and close every queue the worker could be parked on —
        // both data queues and both resource queues (bug-205) — waking every waiter so
        // it re-checks CANCELLED/CLOSED.
        ThreadSimpleOp::Cancel => {
            emit_block_lock(&mut ctx, HANDLE_OFFSET)?;
            ctx.instructions.extend([
                abi::load_u64(&r.v8, abi::stack_pointer(), HANDLE_OFFSET),
                abi::move_immediate(&r.v9, "Integer", "1"),
                abi::store_u64(&r.v9, &r.v8, CONTROL_OFFSET_CANCELLED),
            ]);
            emit_close_queue_locked(&mut ctx, r, HANDLE_OFFSET, CONTROL_OFFSET_INBOUND_QUEUE)?;
            emit_close_queue_locked(&mut ctx, r, HANDLE_OFFSET, CONTROL_OFFSET_OUTBOUND_QUEUE)?;
            emit_close_resource_queues_locked(&mut ctx, r, HANDLE_OFFSET)?;
            emit_block_unlock(&mut ctx, HANDLE_OFFSET)?;
            ctx.instructions.push(abi::move_immediate(
                RESULT_TAG_REGISTER,
                "Integer",
                RESULT_OK_TAG,
            ));
        }
        // plan-156-C: the parent view's drop — its scope end, or its owner's. It closes
        // the view first, so every later op through it (a `TRAP` handler's, a second
        // drop's) answers `ErrResourceClosed`. Then the thread is done from this side: a
        // worker still running is cancelled, and every queue is closed with its
        // undelivered entries dropped, so any wait on either side wakes and re-checks.
        // A worker this drop saw `COMPLETED` has nothing left to do but unlock and
        // return, so it is joined (unless a `waitFor` already did); a RUNNING worker is
        // detached and finishes on its own, its own release freeing the block if this
        // side's already has. Last, this side gives up its reference.
        ThreadSimpleOp::Drop => {
            let detach = format!("{symbol}_detach");
            let settled = format!("{symbol}_settled");
            ctx.instructions.extend([
                abi::load_u64(&r.v10, abi::stack_pointer(), VIEW_OFFSET),
                abi::move_immediate(&r.v9, "Integer", "1"),
                abi::store_u64(&r.v9, &r.v10, RESOURCE_OFFSET_CLOSED),
            ]);
            emit_block_lock(&mut ctx, HANDLE_OFFSET)?;
            ctx.instructions.extend([
                abi::load_u64(&r.v8, abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64(&r.v9, &r.v8, CONTROL_OFFSET_STATE),
                abi::store_u64(&r.v9, abi::stack_pointer(), VALUE_OFFSET),
                abi::move_immediate(&r.v9, "Integer", THREAD_STATE_CLOSED),
                abi::store_u64(&r.v9, &r.v8, CONTROL_OFFSET_STATE),
                abi::move_immediate(&r.v9, "Integer", "1"),
                abi::store_u64(&r.v9, &r.v8, CONTROL_OFFSET_CANCELLED),
                abi::add_immediate(&r.v10, &r.v8, CONTROL_OFFSET_OUTBOUND_QUEUE),
                abi::store_u64(abi::ZERO, &r.v10, THREAD_QUEUE_COUNT_OFFSET),
                abi::store_u64(abi::ZERO, &r.v10, THREAD_QUEUE_HEAD_OFFSET),
                abi::store_u64(abi::ZERO, &r.v10, THREAD_QUEUE_TAIL_OFFSET),
            ]);
            emit_close_queue_locked(&mut ctx, r, HANDLE_OFFSET, CONTROL_OFFSET_OUTBOUND_QUEUE)?;
            emit_close_queue_locked(&mut ctx, r, HANDLE_OFFSET, CONTROL_OFFSET_INBOUND_QUEUE)?;
            emit_close_resource_queues_locked(&mut ctx, r, HANDLE_OFFSET)?;
            emit_block_unlock(&mut ctx, HANDLE_OFFSET)?;
            ctx.instructions.extend([
                abi::load_u64(&r.v8, abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64(abi::c_arg(0), &r.v8, CONTROL_OFFSET_OS_HANDLE),
                abi::compare_immediate(abi::c_arg(0), "0"),
                abi::branch_eq(&settled),
                abi::load_u64(&r.v9, abi::stack_pointer(), VALUE_OFFSET),
                abi::compare_immediate(&r.v9, THREAD_STATE_COMPLETED),
                abi::branch_ne(&detach),
                abi::move_immediate(abi::c_arg(1), "Integer", "0"),
            ]);
            emit_thread_external_call(&mut ctx, "pthread_join")?;
            ctx.instructions.extend([
                abi::branch(&settled),
                abi::label(&detach),
                abi::load_u64(&r.v8, abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64(abi::c_arg(0), &r.v8, CONTROL_OFFSET_OS_HANDLE),
            ]);
            emit_thread_external_call(&mut ctx, "pthread_detach")?;
            ctx.instructions.extend([
                abi::label(&settled),
                abi::load_u64(&r.v8, abi::stack_pointer(), HANDLE_OFFSET),
                abi::store_u64(abi::ZERO, &r.v8, CONTROL_OFFSET_OS_HANDLE),
            ]);
            emit_release_reference(&mut ctx, r, HANDLE_OFFSET, REFS_OFFSET, true)?;
            ctx.instructions.push(abi::move_immediate(
                RESULT_TAG_REGISTER,
                "Integer",
                RESULT_OK_TAG,
            ));
        }
        ThreadSimpleOp::Poll => {
            let ready = format!("{symbol}_ready");
            let invalid = format!("{symbol}_invalid_timeout");
            let wait_loop = format!("{symbol}_wait_loop");
            let wait_timed = format!("{symbol}_wait_timed");
            let not_ready = format!("{symbol}_not_ready");
            let locked_done = format!("{symbol}_locked_done");
            let done = format!("{symbol}_done");
            ctx.instructions.extend([
                abi::compare_immediate(abi::c_arg(1), "0"),
                abi::branch_lt(&invalid),
                abi::store_u64(abi::c_arg(1), abi::stack_pointer(), VALUE_OFFSET),
            ]);
            emit_thread_deadline(&mut ctx, r, VALUE_OFFSET, ERROR_OFFSET)?;
            emit_block_lock(&mut ctx, HANDLE_OFFSET)?;
            ctx.instructions.extend([
                abi::label(&wait_loop),
                abi::load_u64(&r.v8, abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64(
                    &r.v10,
                    &r.v8,
                    CONTROL_OFFSET_OUTBOUND_QUEUE + THREAD_QUEUE_COUNT_OFFSET,
                ),
                abi::compare_immediate(&r.v10, "0"),
                abi::branch_gt(&ready),
                abi::load_u64(&r.v10, &r.v8, CONTROL_OFFSET_STATE),
                abi::compare_immediate(&r.v10, THREAD_STATE_COMPLETED),
                abi::branch_eq(&not_ready),
                abi::load_u64(&r.v10, abi::stack_pointer(), VALUE_OFFSET),
                abi::compare_immediate(&r.v10, "0"),
                abi::branch_gt(&wait_timed),
                abi::branch(&not_ready),
                abi::label(&wait_timed),
            ]);
            emit_queue_wait(
                &mut ctx,
                r,
                HANDLE_OFFSET,
                CONTROL_OFFSET_OUTBOUND_QUEUE,
                THREAD_QUEUE_NOT_EMPTY_OFFSET,
                Some(ERROR_OFFSET),
            )?;
            ctx.instructions.extend([
                abi::compare_immediate(abi::c_return(0), "0"),
                abi::branch_ne(&not_ready),
                abi::branch(&wait_loop),
                abi::label(&ready),
                abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
                abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
                abi::branch(&locked_done),
                abi::label(&not_ready),
                abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
                abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
                abi::branch(&locked_done),
                abi::label(&invalid),
            ]);
            raise_error_into(
                symbol,
                "ErrInvalidArgument",
                ctx.instructions,
                ctx.relocations,
            );
            ctx.instructions.extend([
                abi::branch(&done),
                abi::label(&locked_done),
                abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
                abi::store_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TAG_OFFSET),
                abi::store_u64(
                    RESULT_ERROR_MESSAGE_REGISTER,
                    abi::stack_pointer(),
                    ERROR_OFFSET,
                ),
            ]);
            emit_block_unlock(&mut ctx, HANDLE_OFFSET)?;
            ctx.instructions.extend([
                abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
                abi::load_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TAG_OFFSET),
                abi::load_u64(
                    RESULT_ERROR_MESSAGE_REGISTER,
                    abi::stack_pointer(),
                    ERROR_OFFSET,
                ),
                abi::label(&done),
            ]);
        }
    }
    ctx.instructions.push(abi::return_());
    emit_view_closed_exit(symbol, &view_closed, ctx.instructions, ctx.relocations);
    Ok((instructions, relocations, FRAME_SIZE))
}

/// The caller-frame slots [`emit_cancellable_sleep_wait`] addresses: the thread
/// control block it waits on, the requested `ms`, a free scratch word, and the
/// 16-byte absolute `timespec` deadline.
pub(crate) struct CancellableSleepSlots {
    pub(crate) handle: usize,
    pub(crate) timeout: usize,
    pub(crate) queue: usize,
    pub(crate) timespec: usize,
}

/// plan-99: the cancellation-aware wait shared by the worker `thread::sleep` and the
/// worker branch of `os::sleep`. Waits on the worker's inbound not-empty condvar — one
/// `thread::cancel` broadcasts — until the ABSOLUTE deadline `now + ms`, so a parent
/// `send` arriving mid-sleep re-loops without shortening it, while a pending
/// cancellation branches to `interrupted_label` promptly.
///
/// The caller has already validated `ms` (`< 0` rejected, `== 0` short-circuited),
/// parked it at `slots.timeout`, and parked the thread control block at
/// `slots.handle`. This block FALLS THROUGH once the deadline is reached (with the
/// block lock released), so the caller must place its `ok` epilogue immediately after
/// it; the `interrupted_label` block is emitted separately by
/// [`emit_cancellable_sleep_interrupted`], because in the stream it belongs *after*
/// that epilogue.
pub(crate) fn emit_cancellable_sleep_wait(
    ctx: &mut EmitCtx,
    r: &ThreadVregs,
    slots: CancellableSleepSlots,
    interrupted_label: &str,
) -> Result<(), String> {
    let wait_loop = format!("{}_wait_loop", ctx.symbol);
    let deadline_reached = format!("{}_deadline_reached", ctx.symbol);
    // Absolute deadline = now + ms (pthread_cond_timedwait consumes it).
    emit_thread_deadline(ctx, r, slots.timeout, slots.timespec)?;
    // `emit_cancellable_sleep_interrupted` unlocks through this slot.
    ctx.instructions.extend([
        abi::load_u64(&r.v8, abi::stack_pointer(), slots.handle),
        abi::store_u64(&r.v8, abi::stack_pointer(), slots.queue),
    ]);
    emit_block_lock(ctx, slots.handle)?;
    ctx.instructions.extend([
        abi::label(&wait_loop),
        // Cancellation requested? wake and fail with ErrInterrupted (poll parity with
        // the worker receive path).
        abi::load_u64(&r.v8, abi::stack_pointer(), slots.handle),
        abi::load_u64(&r.v10, &r.v8, CONTROL_OFFSET_CANCELLED),
        abi::compare_immediate(&r.v10, "0"),
        abi::branch_ne(interrupted_label),
    ]);
    emit_queue_wait(
        ctx,
        r,
        slots.handle,
        CONTROL_OFFSET_INBOUND_QUEUE,
        THREAD_QUEUE_NOT_EMPTY_OFFSET,
        Some(slots.timespec),
    )?;
    ctx.instructions.extend([
        // Non-zero (ETIMEDOUT) = the absolute deadline elapsed → the sleep is done.
        // Zero = a spurious/broadcast wake (a parent `send`, or `cancel`); re-loop to
        // re-check the cancel flag. The absolute deadline is unchanged, so a send never
        // shortens the sleep.
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_ne(&deadline_reached),
        abi::branch(&wait_loop),
        abi::label(&deadline_reached),
    ]);
    emit_block_unlock(ctx, slots.handle)
}

/// plan-99: the `interrupted_label` tail of [`emit_cancellable_sleep_wait`] — release
/// the block lock and set the `ErrInterrupted` result. `block_offset` is the frame
/// word [`emit_cancellable_sleep_wait`] parked the control block in (`slots.queue`).
/// The caller appends the `return_` (it owns the epilogue ordering).
pub(crate) fn emit_cancellable_sleep_interrupted(
    ctx: &mut EmitCtx,
    block_offset: usize,
    interrupted_label: &str,
) -> Result<(), String> {
    ctx.instructions.push(abi::label(interrupted_label));
    emit_block_unlock(ctx, block_offset)?;
    ctx.instructions.extend([
        abi::move_immediate(
            RESULT_VALUE_REGISTER,
            "Integer",
            crate::codegen::registry::runtime_error("ErrInterrupted")
                .expect("errorCode name")
                .0,
        ),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        ctx.symbol,
        crate::codegen::registry::runtime_error_emission("ErrInterrupted")
            .expect("errorCode name")
            .1,
        ctx.instructions,
        ctx.relocations,
    );
    Ok(())
}

/// plan-99: `os::sleep(ms)` — the handle-free, context-aware sleep. Blocks the
/// CALLING thread for at least `ms` milliseconds, whichever thread that is:
///
///   - on the main thread it is a plain, uninterruptible relative delay
///     ([`emit_relative_sleep`], libc `nanosleep` / Win32 `Sleep`) with no wakeup
///     path at all;
///   - inside a worker it is the cancellation-aware condvar wait
///     ([`emit_cancellable_sleep_wait`]), so `thread::cancel` wakes it early with
///     `ErrInterrupted` and a parent `send` does not shorten it.
///
/// The two are told apart by the control-block back-pointer the worker trampoline
/// publishes at [`ARENA_WORKER_THREAD_OFFSET`]: `0` (the main thread's zero-init
/// default) takes the plain delay, non-zero takes the wait — and that non-zero value IS
/// the control block the wait needs, so no handle argument is required.
///
/// `ms < 0` → `ErrInvalidArgument`; `ms == 0` returns immediately without reading the
/// arena at all. There is no `ErrResourceClosed`: `os::sleep` owns no handle.
pub(crate) fn lower_os_sleep_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<ThreadBodyParts, String> {
    const FRAME_SIZE: usize = 80;
    // Worker-branch slots (see `CancellableSleepSlots`).
    const HANDLE_OFFSET: usize = 8;
    const TIMEOUT_OFFSET: usize = 16;
    const QUEUE_OFFSET: usize = 24;
    const TIMESPEC_OFFSET: usize = 32;
    // Main-thread-branch slots: the relative `req`/`rem` timespec pair.
    const REQ_OFFSET: usize = 48;
    const REM_OFFSET: usize = 64;

    let ok = format!("{symbol}_ok");
    let err_arg = format!("{symbol}_invalid");
    let worker = format!("{symbol}_worker");
    let interrupted = format!("{symbol}_interrupted");

    let mut vregs = Vregs::new();
    let r = &ThreadVregs::mint(&mut vregs);
    let mut instructions = Vec::new();
    let mut relocations = Vec::new();
    instructions.extend([
        // ms validation first: `< 0` rejects and `== 0` is an immediate no-op Ok, so a
        // zero sleep never touches the arena or a lock.
        abi::compare_immediate(abi::c_arg(0), "0"),
        abi::branch_lt(&err_arg),
        abi::branch_eq(&ok),
        // Park `ms` for the worker branch; the main-thread branch reads it straight out
        // of c_arg(0), which nothing below clobbers before `emit_relative_sleep` copies
        // it.
        abi::store_u64(abi::c_arg(0), abi::stack_pointer(), TIMEOUT_OFFSET),
        // Am I a worker? The trampoline published this thread's control block at
        // arena+8; the main thread's is the zero-init 0.
        abi::load_u64(&r.v8, ARENA_STATE_REGISTER, ARENA_WORKER_THREAD_OFFSET),
        abi::store_u64(&r.v8, abi::stack_pointer(), HANDLE_OFFSET),
        abi::compare_immediate(&r.v8, "0"),
        abi::branch_ne(&worker),
    ]);
    let mut ctx = EmitCtx {
        symbol,
        platform_imports,
        platform,
        instructions: &mut instructions,
        relocations: &mut relocations,
    };
    // Main thread: a plain relative delay; leaves this block only via `ok`.
    emit_relative_sleep(&mut ctx, r, abi::c_arg(0), REQ_OFFSET, REM_OFFSET, &ok)?;
    // Worker: the cancellation-aware wait, on the control block parked above.
    ctx.instructions.push(abi::label(&worker));
    emit_cancellable_sleep_wait(
        &mut ctx,
        r,
        CancellableSleepSlots {
            handle: HANDLE_OFFSET,
            timeout: TIMEOUT_OFFSET,
            queue: QUEUE_OFFSET,
            timespec: TIMESPEC_OFFSET,
        },
        &interrupted,
    )?;
    ctx.instructions.extend([
        // Nothing return: only the OK tag (no result value).
        abi::label(&ok),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::return_(),
    ]);
    emit_cancellable_sleep_interrupted(&mut ctx, QUEUE_OFFSET, &interrupted)?;
    ctx.instructions.extend([
        abi::return_(),
        abi::label(&err_arg),
        abi::move_immediate(
            RESULT_VALUE_REGISTER,
            "Integer",
            crate::codegen::registry::runtime_error("ErrInvalidArgument")
                .expect("errorCode name")
                .0,
        ),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        crate::codegen::registry::runtime_error_emission("ErrInvalidArgument")
            .expect("errorCode name")
            .1,
        ctx.instructions,
        ctx.relocations,
    );
    ctx.instructions.push(abi::return_());
    Ok((instructions, relocations, FRAME_SIZE))
}

pub(crate) fn thread_queue_write_helper(
    symbol: &str,
    queue_offset: usize,
    parent_send: bool,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<ThreadBodyParts, String> {
    const FRAME_SIZE: usize = 80;
    const HANDLE_OFFSET: usize = 8;
    const DATA_OFFSET: usize = 16;
    const TIMEOUT_OFFSET: usize = 24;
    // The 16-byte absolute deadline, [32, 48).
    const TIMESPEC_OFFSET: usize = 32;
    // plan-156-A: arg 3, the message type's copy function (0 = the word is the value),
    // and arg 4, its drop function (recorded on the queue for the final release).
    const COPY_OFFSET: usize = 48;
    const DROP_OFFSET: usize = 72;
    // The block arena, and the caller's arena parked across the copy into it.
    const ARENA_OFFSET: usize = 56;
    const SAVED_ARENA_OFFSET: usize = 64;

    let invalid = format!("{symbol}_invalid");
    let timeout_ok = format!("{symbol}_timeout_ok");
    let closed = format!("{symbol}_closed");
    let interrupted = format!("{symbol}_interrupted");
    let timeout = format!("{symbol}_timeout");
    let copy_failed = format!("{symbol}_copy_failed");
    let wait_loop = format!("{symbol}_wait_loop");
    let wait_indefinite = format!("{symbol}_wait_indefinite");
    let enqueue = format!("{symbol}_enqueue");
    let store = format!("{symbol}_store");
    let tail_wrap = format!("{symbol}_tail_wrap");
    let unlock = format!("{symbol}_unlock");
    let done = format!("{symbol}_done");
    let mut vregs = Vregs::new();
    let r = &ThreadVregs::mint(&mut vregs);
    let mut instructions = Vec::new();
    let mut relocations = Vec::new();
    let mut ctx = EmitCtx {
        symbol,
        platform_imports,
        platform,
        instructions: &mut instructions,
        relocations: &mut relocations,
    };
    // plan-156-C: the handle argument is a view; work on the block it names.
    let view_closed = format!("{symbol}_view_closed");
    emit_control_block_of(ctx.instructions, &view_closed, &r.v9);
    ctx.instructions.extend([
        abi::store_u64(abi::c_arg(0), abi::stack_pointer(), HANDLE_OFFSET),
        abi::store_u64(abi::c_arg(1), abi::stack_pointer(), DATA_OFFSET),
        abi::store_u64(abi::c_arg(2), abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::store_u64(abi::c_arg(3), abi::stack_pointer(), COPY_OFFSET),
        abi::store_u64(abi::c_arg(4), abi::stack_pointer(), DROP_OFFSET),
        // plan-73-A: a non-negative `timeoutMs` is a real timeout (0 = one immediate
        // attempt, N = wait N ms). The unbounded sentinel (i64::MIN) is the omit=block
        // form and is accepted; any OTHER negative value is `ErrInvalidArgument`.
        abi::compare_immediate(abi::c_arg(2), "0"),
        abi::branch_ge(&timeout_ok),
        abi::move_immediate(&r.v9, "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
        abi::compare_registers(abi::c_arg(2), &r.v9),
        abi::branch_ne(&invalid),
        abi::label(&timeout_ok),
    ]);
    if !parent_send {
        // Re-establish the current-thread register `x20` from the worker's own control
        // block (`x0`, read through its view) rather than asserting equality; see
        // `thread_queue_read_helper`.
        ctx.instructions
            .push(abi::move_register(abi::CURRENT_THREAD, abi::c_arg(0)));
    }
    emit_thread_deadline(&mut ctx, r, TIMEOUT_OFFSET, TIMESPEC_OFFSET)?;
    emit_block_lock(&mut ctx, HANDLE_OFFSET)?;
    ctx.instructions.extend([
        abi::label(&wait_loop),
        abi::load_u64(&r.v8, abi::stack_pointer(), HANDLE_OFFSET),
    ]);
    if parent_send {
        ctx.instructions.extend([
            abi::load_u64(&r.v9, &r.v8, CONTROL_OFFSET_STATE),
            abi::compare_immediate(&r.v9, THREAD_STATE_CLOSED),
            abi::branch_eq(&closed),
            abi::compare_immediate(&r.v9, THREAD_STATE_COMPLETED),
            abi::branch_eq(&interrupted),
        ]);
    }
    ctx.instructions.extend([
        abi::load_u64(&r.v9, &r.v8, CONTROL_OFFSET_CANCELLED),
        abi::compare_immediate(&r.v9, "0"),
        abi::branch_ne(&interrupted),
        abi::add_immediate(&r.v9, &r.v8, queue_offset),
        abi::load_u64(&r.v10, &r.v9, THREAD_QUEUE_CLOSED_OFFSET),
        abi::compare_immediate(&r.v10, "0"),
        abi::branch_ne(&interrupted),
        abi::load_u64(&r.v10, &r.v9, THREAD_QUEUE_COUNT_OFFSET),
        abi::load_u64(&r.v11, &r.v9, THREAD_QUEUE_CAPACITY_OFFSET),
        abi::compare_registers(&r.v10, &r.v11),
        abi::branch_lt(&enqueue),
        abi::load_u64(&r.v12, abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::compare_immediate(&r.v12, "0"),
        // plan-73-A: 0 = one immediate attempt (queue full ⇒ `ErrTimeout`); the
        // unbounded sentinel (< 0) blocks until space; a positive value bounds the wait
        // against the absolute deadline.
        abi::branch_eq(&timeout),
        abi::branch_lt(&wait_indefinite),
    ]);
    emit_queue_wait(
        &mut ctx,
        r,
        HANDLE_OFFSET,
        queue_offset,
        THREAD_QUEUE_NOT_FULL_OFFSET,
        Some(TIMESPEC_OFFSET),
    )?;
    ctx.instructions.extend([
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_ne(&timeout),
        abi::branch(&wait_loop),
        // Unbounded (omit) form: block on the not-full condition until a slot frees or
        // the queue/thread closes (re-checked at the top of `wait_loop`).
        abi::label(&wait_indefinite),
    ]);
    emit_queue_wait(
        &mut ctx,
        r,
        HANDLE_OFFSET,
        queue_offset,
        THREAD_QUEUE_NOT_FULL_OFFSET,
        None,
    )?;
    // plan-156-A: the enqueue is now certain — the queue has room and is open — so
    // copy the message into the block arena, under the block lock that serializes
    // every allocation in it. Copying only here means a failed send (full, closed,
    // cancelled, timed out) never made a copy, and a resource's source is flagged
    // `moved` (by its copy) only when it really moves (bug-425).
    ctx.instructions.extend([
        abi::branch(&wait_loop),
        abi::label(&enqueue),
        abi::load_u64(&r.v9, abi::stack_pointer(), COPY_OFFSET),
        abi::compare_immediate(&r.v9, "0"),
        abi::branch_eq(&store),
        abi::load_u64(&r.v8, abi::stack_pointer(), HANDLE_OFFSET),
        abi::load_u64(&r.v9, &r.v8, CONTROL_OFFSET_ARENA),
        abi::store_u64(&r.v9, abi::stack_pointer(), ARENA_OFFSET),
    ]);
    emit_call_in_block_arena(
        ctx.instructions,
        &r.v15,
        ARENA_OFFSET,
        SAVED_ARENA_OFFSET,
        COPY_OFFSET,
        DATA_OFFSET,
    );
    ctx.instructions.extend([
        abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG),
        abi::branch_ne(&copy_failed),
        abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), DATA_OFFSET),
        abi::label(&store),
        abi::load_u64(&r.v8, abi::stack_pointer(), HANDLE_OFFSET),
        abi::add_immediate(&r.v9, &r.v8, queue_offset),
        abi::load_u64(&r.v10, abi::stack_pointer(), DROP_OFFSET),
        abi::store_u64(&r.v10, &r.v9, THREAD_QUEUE_DROP_OFFSET),
        abi::load_u64(&r.v10, &r.v9, THREAD_QUEUE_TAIL_OFFSET),
        abi::load_u64(&r.v11, &r.v9, THREAD_QUEUE_VALUES_OFFSET),
        abi::shift_left_immediate(&r.v12, &r.v10, THREAD_QUEUE_ENTRY_SHIFT),
        abi::add_registers(&r.v11, &r.v11, &r.v12),
        abi::load_u64(&r.v12, abi::stack_pointer(), DATA_OFFSET),
        abi::store_u64(&r.v12, &r.v11, 0),
        abi::add_immediate(&r.v10, &r.v10, 1),
        abi::load_u64(&r.v11, &r.v9, THREAD_QUEUE_CAPACITY_OFFSET),
        abi::compare_registers(&r.v10, &r.v11),
        abi::branch_lt(&tail_wrap),
        abi::move_immediate(&r.v10, "Integer", "0"),
        abi::label(&tail_wrap),
        abi::store_u64(&r.v10, &r.v9, THREAD_QUEUE_TAIL_OFFSET),
        abi::load_u64(&r.v10, &r.v9, THREAD_QUEUE_COUNT_OFFSET),
        abi::add_immediate(&r.v10, &r.v10, 1),
        abi::store_u64(&r.v10, &r.v9, THREAD_QUEUE_COUNT_OFFSET),
    ]);
    emit_queue_cond(
        &mut ctx,
        HANDLE_OFFSET,
        queue_offset,
        THREAD_QUEUE_NOT_EMPTY_OFFSET,
        "pthread_cond_signal",
    )?;
    ctx.instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&unlock),
        abi::label(&interrupted),
    ]);
    raise_error_into(symbol, "ErrInterrupted", ctx.instructions, ctx.relocations);
    ctx.instructions
        .extend([abi::branch(&unlock), abi::label(&closed)]);
    raise_error_into(
        symbol,
        "ErrResourceClosed",
        ctx.instructions,
        ctx.relocations,
    );
    ctx.instructions
        .extend([abi::branch(&unlock), abi::label(&timeout)]);
    raise_error_into(symbol, "ErrTimeout", ctx.instructions, ctx.relocations);
    // The copy ran out of memory. Whatever it allocated before failing stays in the
    // block arena until the block is freed; its own error is discarded (it was built
    // with the arena register on the block arena) and a plain one raised here.
    ctx.instructions
        .extend([abi::branch(&unlock), abi::label(&copy_failed)]);
    raise_error_into(symbol, "ErrOutOfMemory", ctx.instructions, ctx.relocations);
    ctx.instructions
        .extend([abi::branch(&unlock), abi::label(&invalid)]);
    raise_error_into(
        symbol,
        "ErrInvalidArgument",
        ctx.instructions,
        ctx.relocations,
    );
    ctx.instructions.extend([
        abi::branch(&done),
        abi::label(&unlock),
        abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), DATA_OFFSET),
        abi::store_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::store_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            TIMESPEC_OFFSET,
        ),
    ]);
    emit_block_unlock(&mut ctx, HANDLE_OFFSET)?;
    ctx.instructions.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), DATA_OFFSET),
        abi::load_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::load_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            TIMESPEC_OFFSET,
        ),
        abi::label(&done),
        abi::return_(),
    ]);
    emit_view_closed_exit(symbol, &view_closed, ctx.instructions, ctx.relocations);
    Ok((instructions, relocations, FRAME_SIZE))
}

/// How a queue-read helper treats its caller. The read machinery is shared by the data
/// plane (`receive`/`read`) and the resource plane (`acceptResource`/`readResource`);
/// the only difference is whether the caller's `x0` is its own control block, so the
/// helper may re-establish the current-thread register `x20` and consult the worker's
/// cancellation flag (`WorkerSelf`); a parent caller must do neither because `x0` is the
/// *worker's* block and clobbering `x20` would corrupt the parent thread. A parent caller
/// instead checks the worker's run state for termination.
///
/// bug-181: both modes are waitable. The no-arg `receive`/`accept` overload passes the
/// block sentinel (`TIMEOUT_UNBOUNDED_SENTINEL`, i64::MIN) and waits indefinitely; any
/// other negative `timeoutMs` is rejected with `ErrInvalidArgument`. A parent's
/// indefinite wait is terminated when the worker completes or the queue closes (the
/// worker's exit broadcasts every condvar), so it never deadlocks.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ThreadReadMode {
    /// Worker reading its own queue (`receive`, `acceptResource`): re-establish `x20`
    /// and check the worker cancel flag.
    WorkerSelf,
    /// Parent reading a worker queue (`read`, `readResource`): no `x20` touch, check
    /// the worker's run state for termination.
    Parent,
}

pub(crate) fn thread_queue_read_helper(
    symbol: &str,
    queue_offset: usize,
    mode: ThreadReadMode,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<ThreadBodyParts, String> {
    let worker_self = mode == ThreadReadMode::WorkerSelf;
    const FRAME_SIZE: usize = 96;
    const HANDLE_OFFSET: usize = 8;
    const TIMEOUT_OFFSET: usize = 16;
    // plan-156-A: arg 2, the message type's drop function (0 = the word is the value).
    const DROP_OFFSET: usize = 24;
    const VALUE_OFFSET: usize = 32;
    const TAG_OFFSET: usize = 40;
    const ERROR_OFFSET: usize = 48;
    // The 16-byte absolute deadline, [56, 72).
    const TIMESPEC_OFFSET: usize = 56;
    // The block arena, the caller's arena parked across the drop, and the message
    // being dropped.
    const ARENA_OFFSET: usize = 72;
    const SAVED_ARENA_OFFSET: usize = 80;
    const RETIRED_OFFSET: usize = 88;

    let invalid = format!("{symbol}_invalid");
    let found = format!("{symbol}_found");
    let wait_loop = format!("{symbol}_wait_loop");
    let wait_indefinite = format!("{symbol}_wait_indefinite");
    let timeout_ok = format!("{symbol}_timeout_ok");
    let not_found = format!("{symbol}_not_found");
    let interrupted = format!("{symbol}_interrupted");
    let closed = format!("{symbol}_closed");
    let timeout = format!("{symbol}_timeout");
    let head_wrap = format!("{symbol}_head_wrap");
    let head_advance = format!("{symbol}_head_advance");
    let unlock = format!("{symbol}_unlock");
    let done = format!("{symbol}_done");
    let mut vregs = Vregs::new();
    let r = &ThreadVregs::mint(&mut vregs);
    let mut instructions = Vec::new();
    let mut relocations = Vec::new();
    let mut ctx = EmitCtx {
        symbol,
        platform_imports,
        platform,
        instructions: &mut instructions,
        relocations: &mut relocations,
    };
    // plan-156-C: the handle argument is a view; work on the block it names.
    let view_closed = format!("{symbol}_view_closed");
    emit_control_block_of(ctx.instructions, &view_closed, &r.v9);
    ctx.instructions.extend([
        abi::store_u64(abi::c_arg(0), abi::stack_pointer(), HANDLE_OFFSET),
        abi::store_u64(abi::c_arg(1), abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::store_u64(abi::c_arg(2), abi::stack_pointer(), DROP_OFFSET),
    ]);
    if worker_self {
        // The caller's view is this worker's own (the handle is unforgeable in
        // type-correct code), so `x0` is now its control block. Re-establish the current-thread register
        // `x20` from it rather than asserting equality: generated code between worker
        // ops may clobber `x20`.
        ctx.instructions
            .push(abi::move_register(abi::CURRENT_THREAD, abi::c_arg(0)));
    }
    // bug-181: the no-arg overload passes the block sentinel (i64::MIN) to wait
    // indefinitely; a non-negative `timeoutMs` is a real timeout (0 = poll, N = wait N
    // ms). Any other negative value is rejected with `ErrInvalidArgument`.
    ctx.instructions.extend([
        abi::compare_immediate(abi::c_arg(1), "0"),
        abi::branch_ge(&timeout_ok),
        abi::move_immediate(&r.v9, "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
        abi::compare_registers(abi::c_arg(1), &r.v9),
        abi::branch_ne(&invalid),
        abi::label(&timeout_ok),
    ]);
    emit_thread_deadline(&mut ctx, r, TIMEOUT_OFFSET, TIMESPEC_OFFSET)?;
    emit_block_lock(&mut ctx, HANDLE_OFFSET)?;
    // plan-156-A: drop the block-arena message the PREVIOUS read handed out (bug-646's
    // "one read behind"). The call site copies a message into the reader's own arena
    // after this helper returns, so the message a read returns stays queue-owned until
    // the next read on this queue — strictly later, on this same thread, since each
    // queue has exactly one reader. The drop runs on the block arena, under the lock.
    let retire_done = format!("{symbol}_retire_done");
    ctx.instructions.extend([
        abi::load_u64(&r.v8, abi::stack_pointer(), HANDLE_OFFSET),
        abi::add_immediate(&r.v9, &r.v8, queue_offset),
        abi::load_u64(&r.v10, &r.v9, THREAD_QUEUE_LAST_READ_OFFSET),
        abi::compare_immediate(&r.v10, "0"),
        abi::branch_eq(&retire_done),
        abi::store_u64(abi::ZERO, &r.v9, THREAD_QUEUE_LAST_READ_OFFSET),
        abi::store_u64(&r.v10, abi::stack_pointer(), RETIRED_OFFSET),
        abi::load_u64(&r.v9, &r.v8, CONTROL_OFFSET_ARENA),
        abi::store_u64(&r.v9, abi::stack_pointer(), ARENA_OFFSET),
    ]);
    emit_call_in_block_arena(
        ctx.instructions,
        &r.v15,
        ARENA_OFFSET,
        SAVED_ARENA_OFFSET,
        DROP_OFFSET,
        RETIRED_OFFSET,
    );
    ctx.instructions.extend([
        abi::label(&retire_done),
        abi::label(&wait_loop),
        abi::load_u64(&r.v8, abi::stack_pointer(), HANDLE_OFFSET),
        abi::add_immediate(&r.v9, &r.v8, queue_offset),
        abi::load_u64(&r.v10, &r.v9, THREAD_QUEUE_COUNT_OFFSET),
        abi::compare_immediate(&r.v10, "0"),
        abi::branch_gt(&found),
    ]);
    if worker_self {
        ctx.instructions.extend([
            abi::load_u64(&r.v10, &r.v8, CONTROL_OFFSET_CANCELLED),
            abi::compare_immediate(&r.v10, "0"),
            abi::branch_ne(&interrupted),
        ]);
    }
    ctx.instructions.extend([
        abi::load_u64(&r.v10, &r.v9, THREAD_QUEUE_CLOSED_OFFSET),
        abi::compare_immediate(&r.v10, "0"),
        abi::branch_ne(&not_found),
    ]);
    if !worker_self {
        ctx.instructions.extend([
            abi::load_u64(&r.v10, &r.v8, CONTROL_OFFSET_STATE),
            abi::compare_immediate(&r.v10, THREAD_STATE_CLOSED),
            abi::branch_eq(&closed),
            abi::compare_immediate(&r.v10, THREAD_STATE_COMPLETED),
            abi::branch_eq(&not_found),
        ]);
    }
    ctx.instructions.extend([
        abi::load_u64(&r.v10, abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::compare_immediate(&r.v10, "0"),
        // plan-73-A: an explicit `timeoutMs` of 0 on a still-open but empty queue is one
        // immediate attempt that raises `ErrTimeout`, NOT `ErrNotFound`, which is
        // reserved for a genuinely terminal empty queue (closed / completed worker).
        abi::branch_eq(&timeout),
        abi::branch_lt(&wait_indefinite),
    ]);
    emit_queue_wait(
        &mut ctx,
        r,
        HANDLE_OFFSET,
        queue_offset,
        THREAD_QUEUE_NOT_EMPTY_OFFSET,
        Some(TIMESPEC_OFFSET),
    )?;
    ctx.instructions.extend([
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_ne(&timeout),
        abi::branch(&wait_loop),
        abi::label(&wait_indefinite),
    ]);
    emit_queue_wait(
        &mut ctx,
        r,
        HANDLE_OFFSET,
        queue_offset,
        THREAD_QUEUE_NOT_EMPTY_OFFSET,
        None,
    )?;
    ctx.instructions.extend([
        abi::branch(&wait_loop),
        abi::label(&found),
        abi::load_u64(&r.v8, abi::stack_pointer(), HANDLE_OFFSET),
        abi::add_immediate(&r.v9, &r.v8, queue_offset),
        abi::load_u64(&r.v10, &r.v9, THREAD_QUEUE_HEAD_OFFSET),
        abi::load_u64(&r.v11, &r.v9, THREAD_QUEUE_VALUES_OFFSET),
        abi::shift_left_immediate(&r.v12, &r.v10, THREAD_QUEUE_ENTRY_SHIFT),
        abi::add_registers(&r.v11, &r.v11, &r.v12),
        abi::load_u64(&r.v13, &r.v11, 0),
        abi::store_u64(&r.v13, abi::stack_pointer(), VALUE_OFFSET),
        // Remember a block-arena message as the one the next read drops. Only when this
        // type has a drop function: a word-sized message is not a pointer at all.
        abi::load_u64(&r.v12, abi::stack_pointer(), DROP_OFFSET),
        abi::compare_immediate(&r.v12, "0"),
        abi::branch_eq(&head_advance),
        abi::store_u64(&r.v13, &r.v9, THREAD_QUEUE_LAST_READ_OFFSET),
        abi::label(&head_advance),
        abi::add_immediate(&r.v10, &r.v10, 1),
        abi::load_u64(&r.v11, &r.v9, THREAD_QUEUE_CAPACITY_OFFSET),
        abi::compare_registers(&r.v10, &r.v11),
        abi::branch_lt(&head_wrap),
        abi::move_immediate(&r.v10, "Integer", "0"),
        abi::label(&head_wrap),
        abi::store_u64(&r.v10, &r.v9, THREAD_QUEUE_HEAD_OFFSET),
        abi::load_u64(&r.v10, &r.v9, THREAD_QUEUE_COUNT_OFFSET),
        abi::subtract_immediate(&r.v10, &r.v10, 1),
        abi::store_u64(&r.v10, &r.v9, THREAD_QUEUE_COUNT_OFFSET),
    ]);
    emit_queue_cond(
        &mut ctx,
        HANDLE_OFFSET,
        queue_offset,
        THREAD_QUEUE_NOT_FULL_OFFSET,
        "pthread_cond_signal",
    )?;
    ctx.instructions.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&unlock),
        abi::label(&not_found),
    ]);
    raise_error_into(symbol, "ErrNotFound", ctx.instructions, ctx.relocations);
    ctx.instructions
        .extend([abi::branch(&unlock), abi::label(&interrupted)]);
    raise_error_into(symbol, "ErrInterrupted", ctx.instructions, ctx.relocations);
    ctx.instructions
        .extend([abi::branch(&unlock), abi::label(&closed)]);
    raise_error_into(
        symbol,
        "ErrResourceClosed",
        ctx.instructions,
        ctx.relocations,
    );
    ctx.instructions
        .extend([abi::branch(&unlock), abi::label(&timeout)]);
    raise_error_into(symbol, "ErrTimeout", ctx.instructions, ctx.relocations);
    ctx.instructions
        .extend([abi::branch(&unlock), abi::label(&invalid)]);
    raise_error_into(
        symbol,
        "ErrInvalidArgument",
        ctx.instructions,
        ctx.relocations,
    );
    ctx.instructions.extend([
        abi::branch(&done),
        abi::label(&unlock),
        abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
        abi::store_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TAG_OFFSET),
        abi::store_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            ERROR_OFFSET,
        ),
    ]);
    emit_block_unlock(&mut ctx, HANDLE_OFFSET)?;
    ctx.instructions.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
        abi::load_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TAG_OFFSET),
        abi::load_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            ERROR_OFFSET,
        ),
        abi::label(&done),
        abi::return_(),
    ]);
    emit_view_closed_exit(symbol, &view_closed, ctx.instructions, ctx.relocations);
    Ok((instructions, relocations, FRAME_SIZE))
}

pub(crate) fn thread_is_cancelled_helper() -> ThreadBodyParts {
    // Reads the worker's pinned current-thread register `x20` (the thread control
    // block); an unlocked word read — the flag only ever goes 0 → 1.
    let cancelled = "_mfb_rt_thread_is_cancelled_true";
    let done = "_mfb_rt_thread_is_cancelled_done";
    let mut vregs = Vregs::new();
    let r = &ThreadVregs::mint(&mut vregs);
    let instructions = vec![
        abi::load_u64(&r.v9, abi::CURRENT_THREAD, CONTROL_OFFSET_CANCELLED),
        abi::compare_immediate(&r.v9, "0"),
        abi::branch_ne(cancelled),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(done),
        abi::label(cancelled),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::label(done),
        abi::return_(),
    ];
    (instructions, Vec::new(), 0)
}

/// `_mfb_rt_thread_finish(block, tag, value, message, source)`: the worker's exit,
/// called by the trampoline with the entry's result, on the worker, with the arena
/// register still on the worker's arena. It drains the worker's buffered standard
/// output (when the program can buffer it). Under the block lock it copies the result
/// into the block arena and publishes it with `COMPLETED` (unless the handle side
/// already closed the thread, which then reads nothing), closes the inbound and both
/// resource queues and wakes every waiter of every queue, and drops the worker's
/// reference. Then it unmaps the worker's arena — nothing another thread can read
/// points into it any more (plan-156-B) — and, if its reference was the last, frees
/// the control block itself (the handle side let go of a thread that was still
/// running).
pub(crate) fn lower_thread_finish(
    drain_stdout: bool,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<CodeFunction, String> {
    let symbol = THREAD_FINISH_SYMBOL;
    const LOCALS: usize = 80;
    const BLOCK: usize = 8;
    const TAG: usize = 16;
    const VALUE: usize = 24;
    const MESSAGE: usize = 32;
    const SOURCE: usize = 40;
    const REFS: usize = 48;
    // The block arena, the caller's (the worker's) arena parked across a copy, and
    // the result type's copy function.
    const ARENA: usize = 56;
    const SAVED_ARENA: usize = 64;
    const COPY: usize = 72;
    let result_closed = format!("{symbol}_result_closed");
    let result_store = format!("{symbol}_result_store");
    let error_copy = format!("{symbol}_error_copy");
    let message_done = format!("{symbol}_message_done");
    let source_done = format!("{symbol}_source_done");
    let copy_failed = format!("{symbol}_copy_failed");
    // The worker's error travels as a `String` message and an `ErrorLoc` origin; their
    // copy functions are fixed, requested here for this module.
    let string_copy = super::value_functions::request(
        super::value_functions::ThreadValueFn::Copy,
        &crate::types::ParameterType::String,
    );
    let loc_copy = super::value_functions::request(
        super::value_functions::ThreadValueFn::Copy,
        &crate::types::ParameterType::named("ErrorLoc"),
    );
    let mut vregs = Vregs::new();
    let r = &ThreadVregs::mint(&mut vregs);
    let mut instructions = vec![
        abi::label("entry"),
        abi::store_u64(abi::c_arg(0), abi::stack_pointer(), BLOCK),
        abi::store_u64(abi::c_arg(1), abi::stack_pointer(), TAG),
        abi::store_u64(abi::c_arg(2), abi::stack_pointer(), VALUE),
        abi::store_u64(abi::c_arg(3), abi::stack_pointer(), MESSAGE),
        abi::store_u64(abi::c_arg(4), abi::stack_pointer(), SOURCE),
    ];
    let mut relocations = Vec::new();
    let mut ctx = EmitCtx {
        symbol,
        platform_imports,
        platform,
        instructions: &mut instructions,
        relocations: &mut relocations,
    };
    // The worker's buffered standard output (`io::setBuffered`) lives in its arena and
    // is drained here, the worker's end — the same guarantee the main thread's buffer
    // gets at program exit. Best-effort, like every drain: a write failure has no one
    // left to report to.
    if drain_stdout {
        ctx.instructions.push(abi::branch_link(STDOUT_DRAIN_SYMBOL));
        ctx.relocations
            .push(internal_branch(symbol, STDOUT_DRAIN_SYMBOL));
    }
    emit_block_lock(&mut ctx, BLOCK)?;
    // A handle side that already closed the thread reads no result: skip the copies.
    // Otherwise move the result into the block arena — everything the parent reads
    // must outlive this worker's own arena. The copies run under the block lock with
    // the arena register on the block arena.
    ctx.instructions.extend([
        abi::load_u64(&r.v8, abi::stack_pointer(), BLOCK),
        abi::load_u64(&r.v9, &r.v8, CONTROL_OFFSET_STATE),
        abi::compare_immediate(&r.v9, THREAD_STATE_CLOSED),
        abi::branch_eq(&result_closed),
        abi::load_u64(&r.v9, &r.v8, CONTROL_OFFSET_ARENA),
        abi::store_u64(&r.v9, abi::stack_pointer(), ARENA),
        abi::load_u64(&r.v9, abi::stack_pointer(), TAG),
        abi::compare_immediate(&r.v9, RESULT_OK_TAG),
        abi::branch_ne(&error_copy),
        // Success: the value, through the result type's copy function (0 = a word).
        abi::load_u64(&r.v9, &r.v8, CONTROL_OFFSET_RESULT_COPY),
        abi::store_u64(&r.v9, abi::stack_pointer(), COPY),
        abi::compare_immediate(&r.v9, "0"),
        abi::branch_eq(&result_store),
    ]);
    emit_call_in_block_arena(ctx.instructions, &r.v15, ARENA, SAVED_ARENA, COPY, VALUE);
    ctx.instructions.extend([
        abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG),
        abi::branch_ne(&copy_failed),
        abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE),
        abi::branch(&result_store),
        // Failure: the message and the origin (either may be absent).
        abi::label(&error_copy),
        abi::load_u64(&r.v9, abi::stack_pointer(), MESSAGE),
        abi::compare_immediate(&r.v9, "0"),
        abi::branch_eq(&message_done),
    ]);
    crate::codegen::runtime::thread::emit_with_arena(ctx.instructions, ARENA, SAVED_ARENA, |i| {
        i.extend([
            abi::load_u64(abi::c_arg(0), abi::stack_pointer(), MESSAGE),
            abi::branch_link(&string_copy),
        ]);
    });
    ctx.relocations.push(internal_branch(symbol, &string_copy));
    ctx.instructions.extend([
        abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG),
        abi::branch_ne(&copy_failed),
        abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), MESSAGE),
        abi::label(&message_done),
        abi::load_u64(&r.v9, abi::stack_pointer(), SOURCE),
        abi::compare_immediate(&r.v9, "0"),
        abi::branch_eq(&source_done),
    ]);
    crate::codegen::runtime::thread::emit_with_arena(ctx.instructions, ARENA, SAVED_ARENA, |i| {
        i.extend([
            abi::load_u64(abi::c_arg(0), abi::stack_pointer(), SOURCE),
            abi::branch_link(&loc_copy),
        ]);
    });
    ctx.relocations.push(internal_branch(symbol, &loc_copy));
    ctx.instructions.extend([
        abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG),
        abi::branch_ne(&copy_failed),
        abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), SOURCE),
        abi::label(&source_done),
        abi::branch(&result_store),
        // A copy ran out of memory: the parent gets a plain `ErrOutOfMemory` (static
        // message, no origin) instead of a result that would point into this worker's
        // arena.
        abi::label(&copy_failed),
    ]);
    raise_error_into(symbol, "ErrOutOfMemory", ctx.instructions, ctx.relocations);
    ctx.instructions.extend([
        abi::store_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TAG),
        abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE),
        abi::store_u64(RESULT_ERROR_MESSAGE_REGISTER, abi::stack_pointer(), MESSAGE),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), SOURCE),
        abi::label(&result_store),
        abi::load_u64(&r.v8, abi::stack_pointer(), BLOCK),
        abi::load_u64(&r.v9, abi::stack_pointer(), TAG),
        abi::store_u64(&r.v9, &r.v8, CONTROL_OFFSET_RESULT_TAG),
        abi::load_u64(&r.v9, abi::stack_pointer(), VALUE),
        abi::store_u64(&r.v9, &r.v8, CONTROL_OFFSET_RESULT_VALUE),
        abi::load_u64(&r.v9, abi::stack_pointer(), MESSAGE),
        abi::store_u64(&r.v9, &r.v8, CONTROL_OFFSET_RESULT_ERROR),
        abi::load_u64(&r.v9, abi::stack_pointer(), SOURCE),
        abi::store_u64(&r.v9, &r.v8, CONTROL_OFFSET_RESULT_SOURCE),
        abi::move_immediate(&r.v9, "Integer", THREAD_STATE_COMPLETED),
        abi::store_u64(&r.v9, &r.v8, CONTROL_OFFSET_STATE),
        abi::label(&result_closed),
    ]);
    // Wake any parent blocked writing the inbound queue or waiting on the resource
    // planes; the outbound queue stays open so the parent can still read what was
    // sent, and a parent waiting on it re-checks the state and finds `COMPLETED`.
    emit_close_queue_locked(&mut ctx, r, BLOCK, CONTROL_OFFSET_INBOUND_QUEUE)?;
    emit_close_resource_queues_locked(&mut ctx, r, BLOCK)?;
    emit_wake_queue_locked(&mut ctx, BLOCK, CONTROL_OFFSET_OUTBOUND_QUEUE)?;
    ctx.instructions.extend([
        abi::load_u64(&r.v8, abi::stack_pointer(), BLOCK),
        abi::load_u64(&r.v9, &r.v8, CONTROL_OFFSET_REFS),
        abi::subtract_immediate(&r.v9, &r.v9, 1),
        abi::store_u64(&r.v9, &r.v8, CONTROL_OFFSET_REFS),
        abi::store_u64(&r.v9, abi::stack_pointer(), REFS),
    ]);
    emit_block_unlock(&mut ctx, BLOCK)?;
    // plan-156-B: return the worker's arena to the OS. The result, the messages and
    // the transferred resources are all block-arena copies, and the worker's own
    // scope drops already ran, so nothing reads the arena after this: unmap every
    // chunk it mapped (`_mfb_arena_destroy` walks the arena the pinned register names,
    // which here is the worker's). Its arena-state block is not freed here; it goes
    // with the control block, after a handle side that frees the block has joined
    // this thread.
    ctx.instructions
        .push(abi::branch_link(ARENA_DESTROY_SYMBOL));
    ctx.relocations
        .push(internal_branch(symbol, ARENA_DESTROY_SYMBOL));
    let kept = format!("{symbol}_kept");
    ctx.instructions.extend([
        abi::load_u64(&r.v9, abi::stack_pointer(), REFS),
        abi::compare_immediate(&r.v9, "0"),
        abi::branch_ne(&kept),
        abi::load_u64(abi::c_arg(0), abi::stack_pointer(), BLOCK),
        abi::branch_link(THREAD_BLOCK_RELEASE_SYMBOL),
        abi::label(&kept),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::return_(),
    ]);
    ctx.relocations
        .push(internal_branch(symbol, THREAD_BLOCK_RELEASE_SYMBOL));
    let (frame, stack_slots) = crate::codegen::engine::util::finalize_vreg_body_with_locals(
        &mut instructions,
        &[],
        LOCALS,
    );
    Ok(CodeFunction {
        name: "runtime.thread_finish".to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame,
        instructions,
        relocations,
        stack_slots,
    })
}
