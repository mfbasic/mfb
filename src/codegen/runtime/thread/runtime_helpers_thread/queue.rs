//! plan-176-V: the thread message-queue write and read helpers. The shared thread primitives
//! live in `runtime/thread/runtime_helpers_thread/mod.rs`.

use super::*;

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
