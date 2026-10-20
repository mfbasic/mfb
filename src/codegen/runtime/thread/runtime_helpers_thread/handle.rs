//! plan-176-V: the `Thread` handle helpers — the simple handle ops, `isCancelled` and the
//! worker exit (`lower_thread_finish`). The shared thread primitives live in
//! `runtime/thread/runtime_helpers_thread/mod.rs`.

use super::*;

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
/// output (when the program can buffer it). A program-exit result (tag `2`, the
/// worker ran `os::exit`) ends the process right there with the value as its
/// exit code, through the shared teardown `_mfb_shutdown` (when the program has an
/// entry, `program_shutdown`) and the platform's process exit — it is not a result
/// to publish, and its message/origin registers hold nothing (bug-898). Under the
/// block lock it copies the result into the block arena and stores it (unless the
/// handle side already closed the thread, which then reads nothing). It then unmaps
/// the worker's arena — nothing another thread can read points into it any more
/// (plan-156-B) — and only then, under the lock again, publishes `COMPLETED` (unless
/// the thread was closed meanwhile), closes the inbound and both resource queues,
/// wakes every waiter of every queue, and drops the worker's reference (bug-897: the
/// destroy runs on the worker's arena-state block, which the last release frees). If
/// its reference was the last, it frees the control block itself (the handle side
/// let go of a thread that was still running).
pub(crate) fn lower_thread_finish(
    drain_stdout: bool,
    program_shutdown: bool,
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
    // bug-898: `os::exit` in the worker (or anything it called) ends the
    // process with that code, from here, exactly as the program entry's exit does
    // after its own drain: the shared teardown (the main buffered output, the
    // terminal, the graphics thread — a program that starts threads never destroys
    // the main arena there, so the other threads still running are not pulled out
    // from under), then the platform's process exit, which ends every thread. The
    // worker's lexical cleanup already ran on the way out of its frames, and its
    // module-level resources were closed by the trampoline's teardown. Tag 2 carries
    // the code in `value` and NOTHING in `message`/`source`: it used to fall into the
    // error arm below, which deep-copied those stale registers as a `String` and an
    // `ErrorLoc` (SIGSEGV, or a wild length reported as `ErrOutOfMemory`).
    let not_program_exit = format!("{symbol}_not_program_exit");
    ctx.instructions.extend([
        abi::load_u64(r.v9, abi::stack_pointer(), TAG),
        abi::compare_immediate(r.v9, RESULT_PROGRAM_EXIT_TAG),
        abi::branch_ne(&not_program_exit),
    ]);
    if program_shutdown {
        ctx.instructions.push(abi::branch_link(SHUTDOWN_SYMBOL));
        ctx.relocations
            .push(internal_branch(symbol, SHUTDOWN_SYMBOL));
    }
    ctx.instructions.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        VALUE,
    ));
    platform.emit_program_exit(symbol, ctx.instructions, ctx.relocations)?;
    ctx.instructions.push(abi::label(&not_program_exit));
    if program_shutdown {
        // Report only an error escaping this worker's entry, not a handled
        // raise. Run before either block locking or result copying: the source
        // is still alive even if the parent has already dropped its handle.
        let reported = format!("{symbol}_error_reported");
        ctx.instructions.extend([
            abi::load_u64(r.v9, abi::stack_pointer(), TAG),
            abi::compare_immediate(r.v9, RESULT_OK_TAG),
            abi::branch_eq(&reported),
            abi::load_u64(abi::c_arg(0), abi::stack_pointer(), BLOCK),
            abi::load_u64(abi::c_arg(1), abi::stack_pointer(), VALUE),
            abi::load_u64(abi::c_arg(2), abi::stack_pointer(), MESSAGE),
            abi::load_u64(abi::c_arg(3), abi::stack_pointer(), SOURCE),
            abi::branch_link(crate::codegen::error::report::ERROR_REPORT_SYMBOL),
            abi::label(&reported),
        ]);
        ctx.relocations.push(internal_branch(
            symbol,
            crate::codegen::error::report::ERROR_REPORT_SYMBOL,
        ));
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
        abi::label(&result_closed),
    ]);
    emit_block_unlock(&mut ctx, BLOCK)?;
    // plan-156-B: return the worker's arena to the OS. The result (just stored), the
    // messages and the transferred resources are all block-arena copies, and the
    // worker's own scope drops already ran, so nothing reads the arena after this:
    // unmap every chunk it mapped (`_mfb_arena_destroy` walks the arena the pinned
    // register names, which here is the worker's).
    //
    // bug-897: this runs BEFORE the worker gives up its reference, not after. The
    // destroy works on the worker's arena-state block through the arena register,
    // and that block is freed with the control block by whichever side releases
    // last. A parent view dropped while the worker was RUNNING detaches it (no join)
    // and releases; when that release came after the worker's, it freed the arena
    // state while the worker was still inside the destroy (malloc free-list
    // corruption, SIGABRT/SIGSEGV). While the worker holds its reference nobody can
    // free the block, and after it lets go below it touches nothing but its own
    // stack. `COMPLETED` is published with that release, in one critical section, so
    // a handle side that sees it — and joins — never races the destroy either.
    ctx.instructions
        .push(abi::branch_link(ARENA_DESTROY_SYMBOL));
    ctx.relocations
        .push(internal_branch(symbol, ARENA_DESTROY_SYMBOL));
    emit_block_lock(&mut ctx, BLOCK)?;
    // Publish `COMPLETED` unless the handle side closed the thread meanwhile (it
    // then reads nothing; `CLOSED` stays).
    let closed_meanwhile = format!("{symbol}_closed_meanwhile");
    ctx.instructions.extend([
        abi::load_u64(r.v8, abi::stack_pointer(), BLOCK),
        abi::load_u64(r.v9, r.v8, CONTROL_OFFSET_STATE),
        abi::compare_immediate(r.v9, THREAD_STATE_CLOSED),
        abi::branch_eq(&closed_meanwhile),
        abi::move_immediate(&r.v9, "Integer", THREAD_STATE_COMPLETED),
        abi::store_u64(&r.v9, &r.v8, CONTROL_OFFSET_STATE),
        abi::label(&closed_meanwhile),
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
