//! plan-176-V: `os::sleep` and the cancellable sleep it waits on. The shared thread primitives
//! live in `runtime/thread/runtime_helpers_thread/mod.rs`.

use super::*;

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
