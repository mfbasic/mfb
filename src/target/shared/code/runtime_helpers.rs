use super::*;

pub(super) const THREAD_BLOCK_SIZE: usize = 120;
pub(super) const THREAD_OFFSET_STATE: usize = 0;
pub(super) const THREAD_OFFSET_CANCELLED: usize = 8;
pub(super) const THREAD_OFFSET_RESULT_TAG: usize = 16;
pub(super) const THREAD_OFFSET_RESULT_VALUE: usize = 24;
pub(super) const THREAD_OFFSET_RESULT_ERROR: usize = 32;
pub(super) const THREAD_OFFSET_INBOUND_QUEUE: usize = 40;
pub(super) const THREAD_OFFSET_OUTBOUND_QUEUE: usize = 48;
pub(super) const THREAD_OFFSET_OS_HANDLE: usize = 56;
pub(super) const THREAD_OFFSET_ENTRY: usize = 64;
pub(super) const THREAD_OFFSET_DATA: usize = 72;
pub(super) const THREAD_OFFSET_ARENA_STATE: usize = 80;
pub(super) const THREAD_OFFSET_PARENT_ARENA_STATE: usize = 88;
// Origin `ErrorLoc` pointer of a worker's terminal error, captured by the
// trampoline so `thread::waitFor` can recover the worker's source location.
pub(super) const THREAD_OFFSET_RESULT_SOURCE: usize = 96;
// Resource plane (§7): two dedicated queues for `thread::transfer`/
// `thread::accept`, independent of the data-channel inbound/outbound queues so a
// thread can carry both planes at once. The resource plane mirrors the data
// plane's split: the inbound queue carries parent→worker transfers (parent
// `transfer`, worker `accept`); the outbound queue carries worker→parent
// transfers (worker `transfer`, parent `accept`). Two queues keep the directions
// isolated so a thread's own transfer is never re-read by its own accept.
pub(super) const THREAD_OFFSET_RESOURCE_INBOUND_QUEUE: usize = 104;
pub(super) const THREAD_OFFSET_RESOURCE_OUTBOUND_QUEUE: usize = 112;
pub(super) const THREAD_STATE_RUNNING: &str = "0";
pub(super) const THREAD_STATE_COMPLETED: &str = "1";
pub(super) const THREAD_STATE_CLOSED: &str = "2";

// bug-181: the no-arg `thread::receive(t)` / `thread::accept(t)` overload blocks
// indefinitely. The 1-arg lowering pads the missing `timeoutMs` with this
// unreachable sentinel — `i64::MIN` as its `u64` bit pattern (0x8000000000000000).
// The queue-read helper waits forever on exactly this value and rejects every other
// negative `timeoutMs` with `ErrInvalidArgument`; because a valid explicit timeout
// is always `>= 0`, no user-supplied value can collide with the block sentinel. The
// immediate encoder parses `u64`, so the sentinel is spelled as the unsigned decimal
// of i64::MIN's bit pattern rather than a signed `-9223372036854775808`.
pub(super) const THREAD_RECEIVE_BLOCK_SENTINEL: &str = "9223372036854775808";

pub(super) const THREAD_QUEUE_NOT_EMPTY_OFFSET: usize = 64;
pub(super) const THREAD_QUEUE_NOT_FULL_OFFSET: usize = 128;
pub(super) const THREAD_QUEUE_CAPACITY_OFFSET: usize = 192;
pub(super) const THREAD_QUEUE_COUNT_OFFSET: usize = 200;
pub(super) const THREAD_QUEUE_HEAD_OFFSET: usize = 208;
pub(super) const THREAD_QUEUE_TAIL_OFFSET: usize = 216;
pub(super) const THREAD_QUEUE_CLOSED_OFFSET: usize = 224;
pub(super) const THREAD_QUEUE_VALUES_OFFSET: usize = 232;
/// Head of a singly-linked list of orphaned message copies to reclaim (bug-147.5b).
/// A `thread.send` deep-copies the message into the DESTINATION arena before the
/// enqueue commits; a failed send (queue full / closed / cancelled) leaves that
/// copy orphaned there, and a sender-side free would be a cross-thread arena race.
/// Instead the send-failure path pushes the copy onto this list (under the queue
/// mutex, using the dead block's own first two words as `{next, size}`), and the
/// DESTINATION thread drains + frees it in its OWN arena on its next queue read
/// (also under the mutex) — every free on the owning thread, every list op
/// serialized by the mutex both paths already hold.
pub(super) const THREAD_QUEUE_PENDING_FREE_OFFSET: usize = 240;
pub(super) const THREAD_QUEUE_BLOCK_SIZE: usize = 248;

pub(super) fn thread_symbol(platform: &dyn CodegenPlatform, name: &str) -> String {
    if platform.target() == "macos-aarch64" {
        format!("_{name}")
    } else {
        name.to_string()
    }
}

pub(super) fn emit_thread_external_call(ctx: &mut EmitCtx, name: &str) -> Result<(), String> {
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    // `ctx.symbol` is the emitting symbol, which is exactly what the old `from`
    // parameter carried — the two were always passed the same value.
    let symbol = thread_symbol(platform, name);
    ctx.instructions.push(abi::branch_link(&symbol));
    ctx.relocations
        .push(external_branch(ctx.symbol, &symbol, platform_imports)?);
    Ok(())
}

pub(super) fn emit_thread_queue_alloc(
    ctx: &mut EmitCtx,
    limit_stack_offset: usize,
    cb_stack_offset: usize,
    queue_stack_offset: usize,
    cb_queue_offset: usize,
    done_label: &str,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    let alloc_queue_ok = format!("{symbol}_queue_{cb_queue_offset}_alloc_ok");
    let alloc_values_ok = format!("{symbol}_queue_{cb_queue_offset}_values_ok");
    let size_overflow = format!("{symbol}_queue_{cb_queue_offset}_size_overflow");
    let init_error = format!("{symbol}_queue_{cb_queue_offset}_init_error");
    let init_done = format!("{symbol}_queue_{cb_queue_offset}_init_done");

    ctx.instructions.extend([
        abi::move_immediate(abi::ARG[0], "Integer", &THREAD_QUEUE_BLOCK_SIZE.to_string()),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    ctx.relocations
        .push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    ctx.instructions.extend([
        abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG),
        abi::branch_eq(&alloc_queue_ok),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        ctx.instructions,
        ctx.relocations,
    );
    ctx.instructions.extend([
        abi::branch(done_label),
        abi::label(&alloc_queue_ok),
        abi::store_u64(abi::RET[1], abi::stack_pointer(), queue_stack_offset),
        abi::load_u64("%v9", abi::stack_pointer(), cb_stack_offset),
        abi::store_u64(abi::RET[1], "%v9", cb_queue_offset),
        abi::load_u64("%v10", abi::stack_pointer(), limit_stack_offset),
        abi::store_u64("%v10", abi::RET[1], THREAD_QUEUE_CAPACITY_OFFSET),
        abi::store_u64(abi::ZERO, abi::RET[1], THREAD_QUEUE_COUNT_OFFSET),
        abi::store_u64(abi::ZERO, abi::RET[1], THREAD_QUEUE_HEAD_OFFSET),
        abi::store_u64(abi::ZERO, abi::RET[1], THREAD_QUEUE_TAIL_OFFSET),
        abi::store_u64(abi::ZERO, abi::RET[1], THREAD_QUEUE_CLOSED_OFFSET),
        abi::move_immediate("%v11", "Integer", "8"),
        // size = capacity * 8. The limit is upper-bounded in lower_thread_start_helper
        // so this cannot wrap in practice, but trap the high half anyway (defense in
        // depth; bug-60): a wrap would size the block tiny while the stored capacity
        // stays huge, so a later enqueue would index out of the allocation.
        abi::unsigned_multiply_high_registers("%v12", "%v10", "%v11"),
        abi::compare_immediate("%v12", "0"),
        abi::branch_ne(&size_overflow),
        abi::multiply_registers(abi::ARG[0], "%v10", "%v11"),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    ctx.relocations
        .push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    ctx.instructions.extend([
        abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG),
        abi::branch_eq(&alloc_values_ok),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        ctx.instructions,
        ctx.relocations,
    );
    ctx.instructions.push(abi::branch(done_label));
    // capacity * 8 wrapped 64 bits: raise the same catchable allocation error as an
    // oversized request rather than under-allocate the value array (bug-60).
    ctx.instructions.extend([
        abi::label(&size_overflow),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        ctx.instructions,
        ctx.relocations,
    );
    ctx.instructions.extend([
        abi::branch(done_label),
        abi::label(&alloc_values_ok),
        abi::load_u64("%v9", abi::stack_pointer(), queue_stack_offset),
        abi::store_u64(abi::RET[1], "%v9", THREAD_QUEUE_VALUES_OFFSET),
        // Empty pending-free list (bug-147.5b).
        abi::store_u64(abi::ZERO, "%v9", THREAD_QUEUE_PENDING_FREE_OFFSET),
        abi::move_register(abi::ARG[0], "%v9"),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
    ]);
    emit_thread_external_call(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        "pthread_mutex_init",
    )?;
    ctx.instructions.extend([
        abi::compare_immediate(abi::RET[0], "0"),
        abi::branch_ne(&init_error),
        abi::load_u64("%v9", abi::stack_pointer(), queue_stack_offset),
        abi::add_immediate(abi::ARG[0], "%v9", THREAD_QUEUE_NOT_EMPTY_OFFSET),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
    ]);
    emit_thread_external_call(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        "pthread_cond_init",
    )?;
    ctx.instructions.extend([
        abi::compare_immediate(abi::RET[0], "0"),
        abi::branch_ne(&init_error),
        abi::load_u64("%v9", abi::stack_pointer(), queue_stack_offset),
        abi::add_immediate(abi::ARG[0], "%v9", THREAD_QUEUE_NOT_FULL_OFFSET),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
    ]);
    emit_thread_external_call(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        "pthread_cond_init",
    )?;
    ctx.instructions.extend([
        abi::compare_immediate(abi::RET[0], "0"),
        abi::branch_ne(&init_error),
        abi::branch(&init_done),
        abi::label(&init_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INTERRUPTED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INTERRUPTED_SYMBOL,
        ctx.instructions,
        ctx.relocations,
    );
    ctx.instructions.push(abi::branch(done_label));
    ctx.instructions.push(abi::label(&init_done));
    Ok(())
}

pub(super) fn lower_thread_helper(
    symbol: &str,
    call: &str,
    uses_rng: bool,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    match call {
        "thread.start" => lower_thread_start_helper(symbol, uses_rng, platform_imports, platform),
        "thread.isRunning" => simple_thread_handle_helper(
            symbol,
            ThreadSimpleOp::IsRunning,
            platform_imports,
            platform,
        ),
        "thread.waitFor" => {
            simple_thread_handle_helper(symbol, ThreadSimpleOp::WaitFor, platform_imports, platform)
        }
        "thread.cancel" => {
            simple_thread_handle_helper(symbol, ThreadSimpleOp::Cancel, platform_imports, platform)
        }
        "thread.drop" => {
            simple_thread_handle_helper(symbol, ThreadSimpleOp::Drop, platform_imports, platform)
        }
        "thread.send" => thread_queue_write_helper(
            symbol,
            THREAD_OFFSET_INBOUND_QUEUE,
            true,
            platform_imports,
            platform,
        ),
        "thread.poll" => {
            simple_thread_handle_helper(symbol, ThreadSimpleOp::Poll, platform_imports, platform)
        }
        "thread.read" => thread_queue_read_helper(
            symbol,
            THREAD_OFFSET_OUTBOUND_QUEUE,
            ThreadReadMode::Parent,
            platform_imports,
            platform,
        ),
        "thread.receive" => thread_queue_read_helper(
            symbol,
            THREAD_OFFSET_INBOUND_QUEUE,
            ThreadReadMode::WorkerSelf,
            platform_imports,
            platform,
        ),
        "thread.emit" => thread_queue_write_helper(
            symbol,
            THREAD_OFFSET_OUTBOUND_QUEUE,
            false,
            platform_imports,
            platform,
        ),
        // Resource plane: transfer/accept mirror send/receive, split by direction
        // across two queues (like the data plane's send/emit and receive/read).
        // Parent→worker uses the inbound resource queue; worker→parent uses the
        // outbound resource queue, so a thread's own transfer is never re-read by
        // its own accept.
        //
        // transferResource: parent writes the inbound resource queue (mirrors send).
        "thread.transferResource" => thread_queue_write_helper(
            symbol,
            THREAD_OFFSET_RESOURCE_INBOUND_QUEUE,
            true,
            platform_imports,
            platform,
        ),
        // emitResource: worker writes the outbound resource queue (mirrors emit).
        "thread.emitResource" => thread_queue_write_helper(
            symbol,
            THREAD_OFFSET_RESOURCE_OUTBOUND_QUEUE,
            false,
            platform_imports,
            platform,
        ),
        // acceptResource: worker reads the inbound resource queue (mirrors receive).
        "thread.acceptResource" => thread_queue_read_helper(
            symbol,
            THREAD_OFFSET_RESOURCE_INBOUND_QUEUE,
            ThreadReadMode::WorkerSelf,
            platform_imports,
            platform,
        ),
        // readResource: parent reads the outbound resource queue (mirrors read).
        "thread.readResource" => thread_queue_read_helper(
            symbol,
            THREAD_OFFSET_RESOURCE_OUTBOUND_QUEUE,
            ThreadReadMode::Parent,
            platform_imports,
            platform,
        ),
        "thread.isCancelled" => Ok(thread_is_cancelled_helper()),
        "thread.openStdIn" => lower_thread_stdin_subscription_helper(symbol, true),
        "thread.closeStdIn" => lower_thread_stdin_subscription_helper(symbol, false),
        _ => Err(format!("native thread helper does not implement {call}")),
    }
}

/// `thread::openStdIn`/`closeStdIn` (plan-15 §4.5). A thin wrapper over the stdin
/// broadcast subscribe/unsubscribe helpers: `x0 = 0` (the padded no-arg self form)
/// subscribes the calling thread (its arena `x19`); a non-null parent `Thread`
/// handle subscribes the worker behind it (its arena at `THREAD_OFFSET_ARENA_STATE`).
/// Returns `Nothing`.
fn lower_thread_stdin_subscription_helper(symbol: &str, subscribe: bool) -> HelperResult {
    let target = if subscribe {
        STDIN_SUBSCRIBE_SYMBOL
    } else {
        STDIN_UNSUBSCRIBE_SYMBOL
    };
    let worker = format!("{symbol}_worker");
    let do_call = format!("{symbol}_call");
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register("%v9", abi::ARG[0]),
        abi::compare_immediate(abi::ARG[0], "0"),
        abi::branch_ne(&worker),
        // Self form: subscribe the calling thread's own arena.
        abi::move_register(abi::ARG[0], ARENA_STATE_REGISTER),
        abi::branch(&do_call),
        abi::label(&worker),
        // Worker form: the parent `Thread` handle carries the worker's arena state.
        abi::load_u64(abi::ARG[0], "%v9", THREAD_OFFSET_ARENA_STATE),
        abi::label(&do_call),
        abi::branch_link(target),
    ];
    let relocations = vec![internal_branch(symbol, target)];
    instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    instructions.push(abi::return_());
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], 0);
    Ok((frame, instructions, relocations, stack_slots))
}

fn lower_thread_start_helper(
    symbol: &str,
    uses_rng: bool,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    // Vreg-allocated (plan-00-G Phase 2): the control-block/queue scratch slots are
    // an explicit sp-relative local region; x9/x10 scratch becomes vregs. Runs in
    // the parent (x20 is not the worker thread block here), so no reservation.
    const FRAME_SIZE: usize = 160;
    const ENTRY_OFFSET: usize = 8;
    const DATA_OFFSET: usize = 16;
    const IN_LIMIT_OFFSET: usize = 24;
    const OUT_LIMIT_OFFSET: usize = 32;
    const CB_OFFSET: usize = 40;
    const QUEUE_OFFSET: usize = 48;
    // pthread_attr_t scratch: 64 bytes covers musl/glibc (56) and macOS (64).
    const ATTR_OFFSET: usize = 56;
    // Largest queue limit whose `capacity * 8` byte size still fits in 64 bits.
    const MAX_QUEUE_LIMIT: u64 = u64::MAX / 8;

    let invalid_limit = format!("{symbol}_invalid_limit");
    let alloc_block_ok = format!("{symbol}_alloc_block_ok");
    let alloc_worker_arena_ok = format!("{symbol}_alloc_worker_arena_ok");
    let spawn_error = format!("{symbol}_spawn_error");
    let parent_done = format!("{symbol}_parent_done");
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();

    instructions.extend([
        abi::store_u64(abi::ARG[0], abi::stack_pointer(), ENTRY_OFFSET),
        abi::store_u64(abi::ARG[1], abi::stack_pointer(), DATA_OFFSET),
        abi::store_u64(abi::ARG[2], abi::stack_pointer(), IN_LIMIT_OFFSET),
        abi::store_u64(abi::ARG[3], abi::stack_pointer(), OUT_LIMIT_OFFSET),
        abi::compare_immediate(abi::ARG[2], "1"),
        abi::branch_lt(&invalid_limit),
        abi::compare_immediate(abi::ARG[3], "1"),
        abi::branch_lt(&invalid_limit),
        // Upper-bound the queue limit so the later `capacity * 8` value-array size
        // (emit_thread_queue_alloc) cannot wrap 64 bits and under-allocate. The cap
        // is the largest capacity whose `*8` still fits (u64::MAX / 8); an
        // out-of-range limit is rejected as an invalid argument (bug-60).
        abi::move_immediate("%v12", "Integer", &MAX_QUEUE_LIMIT.to_string()),
        abi::compare_registers(abi::ARG[2], "%v12"),
        abi::branch_hi(&invalid_limit),
        abi::compare_registers(abi::ARG[3], "%v12"),
        abi::branch_hi(&invalid_limit),
        abi::move_immediate(abi::ARG[0], "Integer", &THREAD_BLOCK_SIZE.to_string()),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG),
        abi::branch_eq(&alloc_block_ok),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&parent_done),
        abi::label(&alloc_block_ok),
        abi::store_u64(abi::RET[1], abi::stack_pointer(), CB_OFFSET),
        abi::move_register("%v9", abi::RET[1]),
        abi::store_u64(abi::ZERO, "%v9", THREAD_OFFSET_STATE),
        abi::store_u64(abi::ZERO, "%v9", THREAD_OFFSET_CANCELLED),
        abi::store_u64(abi::ZERO, "%v9", THREAD_OFFSET_RESULT_TAG),
        abi::store_u64(abi::ZERO, "%v9", THREAD_OFFSET_RESULT_VALUE),
        abi::store_u64(abi::ZERO, "%v9", THREAD_OFFSET_RESULT_ERROR),
        abi::store_u64(abi::ZERO, "%v9", THREAD_OFFSET_RESULT_SOURCE),
        abi::store_u64(abi::ZERO, "%v9", THREAD_OFFSET_INBOUND_QUEUE),
        abi::store_u64(abi::ZERO, "%v9", THREAD_OFFSET_OUTBOUND_QUEUE),
        abi::store_u64(abi::ZERO, "%v9", THREAD_OFFSET_RESOURCE_INBOUND_QUEUE),
        abi::store_u64(abi::ZERO, "%v9", THREAD_OFFSET_RESOURCE_OUTBOUND_QUEUE),
        abi::store_u64(abi::ZERO, "%v9", THREAD_OFFSET_OS_HANDLE),
        // PARENT_ARENA_STATE is written with the real value a few lines below;
        // the zero-init store here was dead (bug-102).
        abi::load_u64("%v10", abi::stack_pointer(), ENTRY_OFFSET),
        abi::store_u64("%v10", "%v9", THREAD_OFFSET_ENTRY),
        abi::load_u64("%v10", abi::stack_pointer(), DATA_OFFSET),
        abi::store_u64("%v10", "%v9", THREAD_OFFSET_DATA),
        abi::store_u64(
            ARENA_STATE_REGISTER,
            "%v9",
            THREAD_OFFSET_PARENT_ARENA_STATE,
        ),
        abi::move_immediate(abi::ARG[0], "Integer", &ARENA_STATE_SIZE.to_string()),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG),
        abi::branch_eq(&alloc_worker_arena_ok),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    // Zero the child arena state with a loop over ARENA_STATE_SIZE
    // (allocator-04): the block is arena-allocated (poisoned, not zero), and
    // this initializer must stay in lockstep with the program-entry zeroing
    // (`entry_and_arena.rs` `lower_program_entry`) — both zero exactly
    // `ARENA_STATE_SIZE`, so growing the state (e.g. quick bins) can never
    // leave a field as garbage in one path but not the other.
    let child_zero_loop = format!("{symbol}_child_arena_zero");
    instructions.extend([
        abi::branch(&parent_done),
        abi::label(&alloc_worker_arena_ok),
        abi::move_register("%v11", abi::RET[1]),
        abi::add_immediate("%v12", abi::RET[1], ARENA_STATE_SIZE),
        abi::label(&child_zero_loop),
        abi::store_u64(abi::ZERO, "%v11", 0),
        abi::add_immediate("%v11", "%v11", 8),
        abi::compare_registers("%v11", "%v12"),
        abi::branch_lo(&child_zero_loop),
        abi::load_u64("%v9", abi::stack_pointer(), CB_OFFSET),
        abi::store_u64(abi::RET[1], "%v9", THREAD_OFFSET_ARENA_STATE),
        // Inherit the parent's Money rounding mode (plan-29-D): the child arena was
        // just zeroed (= Commercial), so copy the spawning thread's mode field
        // (`x19` is the parent arena here) into the child, which then diverges
        // independently — consistent with per-thread RNG/state isolation.
        abi::load_u64("%v11", abi::ARENA, ARENA_ROUNDING_MODE_OFFSET),
        abi::store_u64("%v11", abi::RET[1], ARENA_ROUNDING_MODE_OFFSET),
    ]);

    if uses_rng {
        // Give the new thread its own PCG64 stream by drawing a 64-bit seed from
        // the spawning thread's generator (runs in the parent, so `x19` is the
        // parent arena and the draw is race-free). Reload the child arena from
        // the control block afterwards because the draw clobbers x0-x18.
        instructions.push(abi::branch_link(RNG_NEXT_SYMBOL));
        relocations.push(internal_branch(symbol, RNG_NEXT_SYMBOL));
        instructions.extend([
            abi::move_register(abi::ARG[1], abi::return_register()),
            abi::load_u64("%v9", abi::stack_pointer(), CB_OFFSET),
            abi::load_u64(abi::return_register(), "%v9", THREAD_OFFSET_ARENA_STATE),
        ]);
        instructions.push(abi::branch_link(RNG_SEED_SYMBOL));
        relocations.push(internal_branch(symbol, RNG_SEED_SYMBOL));
    }

    // Seed the worker's dedicated memory-fill RNG (always on, plan-01 §6). Draw an
    // entropy-derived 64-bit value from the parent's fill stream — race-free since
    // this runs in the parent (`x19` is the parent arena) — and XOR the worker
    // arena address so each worker poisons with a distinct stream. The worker's
    // fill RNG (offsets 16/24) is separate from its `math::rand` stream, exactly
    // as on the main thread. (`arenaStartTime` at offset 40 stays 0 for workers;
    // it is a main-thread diagnostic and not needed for the seed's distinctness.)
    instructions.push(abi::branch_link(ARENA_FILL_NEXT_SYMBOL));
    relocations.push(internal_branch(symbol, ARENA_FILL_NEXT_SYMBOL));
    instructions.extend([
        abi::move_register(abi::ARG[1], abi::return_register()),
        abi::load_u64("%v9", abi::stack_pointer(), CB_OFFSET),
        abi::load_u64(abi::return_register(), "%v9", THREAD_OFFSET_ARENA_STATE),
        abi::exclusive_or_registers(abi::ARG[1], abi::ARG[1], abi::return_register()),
    ]);
    instructions.push(abi::branch_link(ARENA_FILL_SEED_SYMBOL));
    relocations.push(internal_branch(symbol, ARENA_FILL_SEED_SYMBOL));

    emit_thread_queue_alloc(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        IN_LIMIT_OFFSET,
        CB_OFFSET,
        QUEUE_OFFSET,
        THREAD_OFFSET_INBOUND_QUEUE,
        &parent_done,
    )?;
    emit_thread_queue_alloc(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        OUT_LIMIT_OFFSET,
        CB_OFFSET,
        QUEUE_OFFSET,
        THREAD_OFFSET_OUTBOUND_QUEUE,
        &parent_done,
    )?;
    // Resource plane queues (§7): inbound (parent→worker) bounded like the
    // inbound data queue, outbound (worker→parent) bounded like the outbound
    // data queue.
    emit_thread_queue_alloc(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        IN_LIMIT_OFFSET,
        CB_OFFSET,
        QUEUE_OFFSET,
        THREAD_OFFSET_RESOURCE_INBOUND_QUEUE,
        &parent_done,
    )?;
    emit_thread_queue_alloc(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        OUT_LIMIT_OFFSET,
        CB_OFFSET,
        QUEUE_OFFSET,
        THREAD_OFFSET_RESOURCE_OUTBOUND_QUEUE,
        &parent_done,
    )?;

    let pthread_create_symbol = if platform.target() == "macos-aarch64" {
        "_pthread_create"
    } else {
        "pthread_create"
    };
    let (attr_init_symbol, attr_setstacksize_symbol) = if platform.target() == "macos-aarch64" {
        ("_pthread_attr_init", "_pthread_attr_setstacksize")
    } else {
        ("pthread_attr_init", "pthread_attr_setstacksize")
    };
    // Give the worker an explicit 8 MiB stack. musl's default pthread stack is
    // 128 KiB — far below what the main thread gets (typically 8 MiB via
    // RLIMIT_STACK) — so worker code with large frames (the regex engine has a
    // ~230 KiB frame) overflowed the stack on Linux/musl while passing on
    // macOS (512 KiB default). The memory is reserved lazily (virtual), so the
    // cost per thread is address space, not RSS. The attr is stack scratch;
    // pthread_attr_destroy is a no-op for a stacksize-only attr on musl,
    // glibc, and macOS, so it is not called.
    instructions.push(abi::add_immediate(
        abi::ARG[0],
        abi::stack_pointer(),
        ATTR_OFFSET,
    ));
    instructions.push(abi::branch_link(attr_init_symbol));
    relocations.push(external_branch(symbol, attr_init_symbol, platform_imports)?);
    instructions.extend([
        abi::add_immediate(abi::ARG[0], abi::stack_pointer(), ATTR_OFFSET),
        abi::move_immediate(abi::ARG[1], "Integer", &(8 * 1024 * 1024).to_string()),
    ]);
    instructions.push(abi::branch_link(attr_setstacksize_symbol));
    relocations.push(external_branch(
        symbol,
        attr_setstacksize_symbol,
        platform_imports,
    )?);
    instructions.extend([
        abi::load_u64("%v9", abi::stack_pointer(), CB_OFFSET),
        abi::add_immediate(abi::ARG[0], "%v9", THREAD_OFFSET_OS_HANDLE),
        abi::add_immediate(abi::ARG[1], abi::stack_pointer(), ATTR_OFFSET),
    ]);
    instructions.push(abi::load_page_address(
        abi::ARG[2],
        THREAD_TRAMPOLINE_SYMBOL,
    ));
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: THREAD_TRAMPOLINE_SYMBOL.to_string(),
        kind: RelocIntent::DataAddrHi,
        binding: "data".to_string(),
        library: None,
    });
    instructions.push(abi::add_page_offset(
        abi::ARG[2],
        abi::ARG[2],
        THREAD_TRAMPOLINE_SYMBOL,
    ));
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: THREAD_TRAMPOLINE_SYMBOL.to_string(),
        kind: RelocIntent::DataAddrLo,
        binding: "data".to_string(),
        library: None,
    });
    instructions.extend([
        abi::move_register(abi::ARG[3], "%v9"),
        abi::branch_link(pthread_create_symbol),
    ]);
    relocations.push(external_branch(
        symbol,
        pthread_create_symbol,
        platform_imports,
    )?);
    instructions.extend([
        abi::compare_immediate(abi::RET[0], "0"),
        abi::branch_ne(&spawn_error),
        abi::load_u64("%v9", abi::stack_pointer(), CB_OFFSET),
        abi::move_register(RESULT_VALUE_REGISTER, "%v9"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&parent_done),
    ]);

    instructions.extend([
        abi::label(&invalid_limit),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INVALID_ARGUMENT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INVALID_ARGUMENT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&parent_done),
        abi::label(&spawn_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INTERRUPTED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INTERRUPTED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::branch(&parent_done));
    instructions.extend([abi::label(&parent_done), abi::return_()]);

    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], FRAME_SIZE);
    Ok((frame, instructions, relocations, stack_slots))
}

pub(crate) fn lower_thread_trampoline(
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    uses_stdin: bool,
) -> Result<CodeFunction, String> {
    // Machine-floor code: hand-managed frame + pinned registers across the worker
    // call and the `pthread_*` calls, so the allocator cannot run here. Scratch is
    // the neutral `abi::SCRATCH` pool (realized in `abi.rs`), confined to the low
    // indices `SCRATCH[4]`/`SCRATCH[5]` (AArch64 `x13`/`x14`) — NOT `SCRATCH[0]`
    // (`x9`): the x86 residual-scratch pool wraps at xN+11, so `x9` would alias the
    // parked control-block register (`abi::CURRENT_THREAD` = `x20`, both `rbx`) and
    // `load SCRATCH[0],[%thread,…]` would destroy the block pointer it dereferences.
    // `x13`/`x14` (`r9`/`r10`) are distinct from the current-thread register on both
    // ISAs.
    const FRAME_SIZE: usize = 80;
    const LR_OFFSET: usize = 0;
    const ARENA_OFFSET: usize = 8;
    const X20_OFFSET: usize = 16;
    const CLOSURE_OFFSET: usize = 24;
    const CB_OFFSET: usize = 32;
    const TAG_OFFSET: usize = 40;
    const VALUE_OFFSET: usize = 48;
    const ERROR_OFFSET: usize = 56;
    const SOURCE_OFFSET: usize = 64;
    let result_closed = format!("{THREAD_TRAMPOLINE_SYMBOL}_result_closed");

    let mut instructions = vec![
        abi::label("entry"),
        abi::subtract_stack(FRAME_SIZE),
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::store_u64(ARENA_STATE_REGISTER, abi::stack_pointer(), ARENA_OFFSET),
        abi::store_u64(abi::CURRENT_THREAD, abi::stack_pointer(), X20_OFFSET),
        abi::store_u64(CLOSURE_ENV_REGISTER, abi::stack_pointer(), CLOSURE_OFFSET),
        abi::move_register(abi::CURRENT_THREAD, abi::ARG[0]),
        abi::store_u64(abi::CURRENT_THREAD, abi::stack_pointer(), CB_OFFSET),
        abi::load_u64(
            ARENA_STATE_REGISTER,
            abi::CURRENT_THREAD,
            THREAD_OFFSET_ARENA_STATE,
        ),
        abi::load_u64(abi::SCRATCH[4], abi::CURRENT_THREAD, THREAD_OFFSET_ENTRY),
        abi::load_u64(CLOSURE_ENV_REGISTER, abi::SCRATCH[4], CLOSURE_OFFSET_ENV),
        abi::load_u64(abi::SCRATCH[4], abi::SCRATCH[4], CLOSURE_OFFSET_CODE),
        abi::load_u64(abi::ARG[1], abi::CURRENT_THREAD, THREAD_OFFSET_DATA),
        abi::move_register(abi::ARG[0], abi::CURRENT_THREAD),
        abi::branch_link_register(abi::SCRATCH[4]),
        abi::store_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TAG_OFFSET),
        abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
        abi::store_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            ERROR_OFFSET,
        ),
        abi::store_u64(
            RESULT_ERROR_SOURCE_REGISTER,
            abi::stack_pointer(),
            SOURCE_OFFSET,
        ),
    ];
    let mut relocations = Vec::new();
    // plan-15 §4.5: auto-unsubscribe the worker from the stdin broadcast log at
    // teardown so an early-exiting worker never permanently pins the log's
    // reclamation point (which would eventually block other readers at the cap).
    // `x19` still holds the worker arena here; the result registers are already
    // parked on the stack, and `unsubscribe` preserves the callee-saved arena base.
    if uses_stdin {
        instructions.push(abi::move_register(abi::ARG[0], ARENA_STATE_REGISTER));
        instructions.push(abi::branch_link(STDIN_UNSUBSCRIBE_SYMBOL));
        relocations.push(internal_branch(
            THREAD_TRAMPOLINE_SYMBOL,
            STDIN_UNSUBSCRIBE_SYMBOL,
        ));
    }
    instructions.extend([
        abi::load_u64(abi::CURRENT_THREAD, abi::stack_pointer(), CB_OFFSET),
        abi::load_u64(
            abi::SCRATCH[4],
            abi::CURRENT_THREAD,
            THREAD_OFFSET_INBOUND_QUEUE,
        ),
        abi::move_register(abi::ARG[0], abi::SCRATCH[4]),
    ]);
    emit_thread_external_call(
        &mut EmitCtx {
            symbol: THREAD_TRAMPOLINE_SYMBOL,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_mutex_lock",
    )?;
    instructions.extend([
        abi::load_u64(abi::CURRENT_THREAD, abi::stack_pointer(), CB_OFFSET),
        abi::load_u64(
            abi::SCRATCH[4],
            abi::CURRENT_THREAD,
            THREAD_OFFSET_INBOUND_QUEUE,
        ),
        abi::move_immediate(abi::SCRATCH[5], "Integer", "1"),
        abi::store_u64(abi::SCRATCH[5], abi::SCRATCH[4], THREAD_QUEUE_CLOSED_OFFSET),
        abi::add_immediate(abi::ARG[0], abi::SCRATCH[4], THREAD_QUEUE_NOT_EMPTY_OFFSET),
    ]);
    emit_thread_external_call(
        &mut EmitCtx {
            symbol: THREAD_TRAMPOLINE_SYMBOL,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_cond_broadcast",
    )?;
    instructions.extend([
        abi::load_u64(abi::CURRENT_THREAD, abi::stack_pointer(), CB_OFFSET),
        abi::load_u64(
            abi::SCRATCH[4],
            abi::CURRENT_THREAD,
            THREAD_OFFSET_INBOUND_QUEUE,
        ),
        abi::add_immediate(abi::ARG[0], abi::SCRATCH[4], THREAD_QUEUE_NOT_FULL_OFFSET),
    ]);
    emit_thread_external_call(
        &mut EmitCtx {
            symbol: THREAD_TRAMPOLINE_SYMBOL,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_cond_broadcast",
    )?;
    instructions.extend([
        abi::load_u64(abi::CURRENT_THREAD, abi::stack_pointer(), CB_OFFSET),
        abi::load_u64(
            abi::ARG[0],
            abi::CURRENT_THREAD,
            THREAD_OFFSET_INBOUND_QUEUE,
        ),
    ]);
    emit_thread_external_call(
        &mut EmitCtx {
            symbol: THREAD_TRAMPOLINE_SYMBOL,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_mutex_unlock",
    )?;
    // Close both resource-plane queues on worker exit, mirroring the data
    // queues: wake any parent blocked in `thread::transfer` (writing the inbound
    // resource queue) or `thread::accept` (reading the outbound resource queue).
    for resource_queue_offset in [
        THREAD_OFFSET_RESOURCE_INBOUND_QUEUE,
        THREAD_OFFSET_RESOURCE_OUTBOUND_QUEUE,
    ] {
        instructions.extend([
            abi::load_u64(abi::CURRENT_THREAD, abi::stack_pointer(), CB_OFFSET),
            abi::load_u64(abi::SCRATCH[4], abi::CURRENT_THREAD, resource_queue_offset),
            abi::move_register(abi::ARG[0], abi::SCRATCH[4]),
        ]);
        emit_thread_external_call(
            &mut EmitCtx {
                symbol: THREAD_TRAMPOLINE_SYMBOL,
                platform_imports,
                platform,
                instructions: &mut instructions,
                relocations: &mut relocations,
            },
            "pthread_mutex_lock",
        )?;
        instructions.extend([
            abi::load_u64(abi::CURRENT_THREAD, abi::stack_pointer(), CB_OFFSET),
            abi::load_u64(abi::SCRATCH[4], abi::CURRENT_THREAD, resource_queue_offset),
            abi::move_immediate(abi::SCRATCH[5], "Integer", "1"),
            abi::store_u64(abi::SCRATCH[5], abi::SCRATCH[4], THREAD_QUEUE_CLOSED_OFFSET),
            abi::add_immediate(abi::ARG[0], abi::SCRATCH[4], THREAD_QUEUE_NOT_EMPTY_OFFSET),
        ]);
        emit_thread_external_call(
            &mut EmitCtx {
                symbol: THREAD_TRAMPOLINE_SYMBOL,
                platform_imports,
                platform,
                instructions: &mut instructions,
                relocations: &mut relocations,
            },
            "pthread_cond_broadcast",
        )?;
        instructions.extend([
            abi::load_u64(abi::CURRENT_THREAD, abi::stack_pointer(), CB_OFFSET),
            abi::load_u64(abi::SCRATCH[4], abi::CURRENT_THREAD, resource_queue_offset),
            abi::add_immediate(abi::ARG[0], abi::SCRATCH[4], THREAD_QUEUE_NOT_FULL_OFFSET),
        ]);
        emit_thread_external_call(
            &mut EmitCtx {
                symbol: THREAD_TRAMPOLINE_SYMBOL,
                platform_imports,
                platform,
                instructions: &mut instructions,
                relocations: &mut relocations,
            },
            "pthread_cond_broadcast",
        )?;
        instructions.extend([
            abi::load_u64(abi::CURRENT_THREAD, abi::stack_pointer(), CB_OFFSET),
            abi::load_u64(abi::ARG[0], abi::CURRENT_THREAD, resource_queue_offset),
        ]);
        emit_thread_external_call(
            &mut EmitCtx {
                symbol: THREAD_TRAMPOLINE_SYMBOL,
                platform_imports,
                platform,
                instructions: &mut instructions,
                relocations: &mut relocations,
            },
            "pthread_mutex_unlock",
        )?;
    }
    instructions.extend([
        abi::load_u64(abi::CURRENT_THREAD, abi::stack_pointer(), CB_OFFSET),
        abi::load_u64(
            abi::SCRATCH[4],
            abi::CURRENT_THREAD,
            THREAD_OFFSET_OUTBOUND_QUEUE,
        ),
        abi::move_register(abi::ARG[0], abi::SCRATCH[4]),
    ]);
    emit_thread_external_call(
        &mut EmitCtx {
            symbol: THREAD_TRAMPOLINE_SYMBOL,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_mutex_lock",
    )?;
    instructions.extend([
        abi::load_u64(abi::CURRENT_THREAD, abi::stack_pointer(), CB_OFFSET),
        abi::load_u64(abi::SCRATCH[4], abi::CURRENT_THREAD, THREAD_OFFSET_STATE),
        abi::compare_immediate(abi::SCRATCH[4], THREAD_STATE_CLOSED),
        abi::branch_eq(&result_closed),
        abi::load_u64(abi::CURRENT_THREAD, abi::stack_pointer(), CB_OFFSET),
        abi::load_u64(abi::SCRATCH[4], abi::stack_pointer(), TAG_OFFSET),
        abi::store_u64(
            abi::SCRATCH[4],
            abi::CURRENT_THREAD,
            THREAD_OFFSET_RESULT_TAG,
        ),
        abi::load_u64(abi::SCRATCH[4], abi::stack_pointer(), VALUE_OFFSET),
        abi::store_u64(
            abi::SCRATCH[4],
            abi::CURRENT_THREAD,
            THREAD_OFFSET_RESULT_VALUE,
        ),
        abi::load_u64(abi::SCRATCH[4], abi::stack_pointer(), ERROR_OFFSET),
        abi::store_u64(
            abi::SCRATCH[4],
            abi::CURRENT_THREAD,
            THREAD_OFFSET_RESULT_ERROR,
        ),
        abi::load_u64(abi::SCRATCH[4], abi::stack_pointer(), SOURCE_OFFSET),
        abi::store_u64(
            abi::SCRATCH[4],
            abi::CURRENT_THREAD,
            THREAD_OFFSET_RESULT_SOURCE,
        ),
        abi::move_immediate(abi::SCRATCH[5], "Integer", THREAD_STATE_COMPLETED),
        abi::store_u64(abi::SCRATCH[5], abi::CURRENT_THREAD, THREAD_OFFSET_STATE),
        abi::load_u64(
            abi::SCRATCH[4],
            abi::CURRENT_THREAD,
            THREAD_OFFSET_OUTBOUND_QUEUE,
        ),
        abi::add_immediate(abi::ARG[0], abi::SCRATCH[4], THREAD_QUEUE_NOT_EMPTY_OFFSET),
    ]);
    emit_thread_external_call(
        &mut EmitCtx {
            symbol: THREAD_TRAMPOLINE_SYMBOL,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_cond_broadcast",
    )?;
    instructions.extend([
        abi::load_u64(abi::CURRENT_THREAD, abi::stack_pointer(), CB_OFFSET),
        abi::load_u64(
            abi::SCRATCH[4],
            abi::CURRENT_THREAD,
            THREAD_OFFSET_OUTBOUND_QUEUE,
        ),
        abi::add_immediate(abi::ARG[0], abi::SCRATCH[4], THREAD_QUEUE_NOT_FULL_OFFSET),
    ]);
    emit_thread_external_call(
        &mut EmitCtx {
            symbol: THREAD_TRAMPOLINE_SYMBOL,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_cond_broadcast",
    )?;
    instructions.extend([
        abi::label(&result_closed),
        abi::load_u64(abi::CURRENT_THREAD, abi::stack_pointer(), CB_OFFSET),
        abi::load_u64(
            abi::ARG[0],
            abi::CURRENT_THREAD,
            THREAD_OFFSET_OUTBOUND_QUEUE,
        ),
    ]);
    emit_thread_external_call(
        &mut EmitCtx {
            symbol: THREAD_TRAMPOLINE_SYMBOL,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_mutex_unlock",
    )?;
    instructions.extend([
        abi::move_immediate(abi::RET[0], "Integer", "0"),
        abi::load_u64(ARENA_STATE_REGISTER, abi::stack_pointer(), ARENA_OFFSET),
        abi::load_u64(CLOSURE_ENV_REGISTER, abi::stack_pointer(), CLOSURE_OFFSET),
        abi::load_u64(abi::CURRENT_THREAD, abi::stack_pointer(), X20_OFFSET),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);
    // plan-34-D: the trampoline is machine-floor shared lowering that bypasses
    // the allocator — its stream must still name no physical register (pinned
    // registers and scratch are `abi` tokens, realized during selection).
    if let Some(offense) = regalloc::find_physical_operand(&instructions) {
        return Err(format!(
            "thread-trampoline lowering violated the zero-physical-register \
             invariant (plan-34-D): {offense}"
        ));
    }
    Ok(CodeFunction {
        name: "runtime.thread.trampoline".to_string(),
        symbol: THREAD_TRAMPOLINE_SYMBOL.to_string(),
        params: vec![CodeParam {
            name: "controlBlock".to_string(),
            type_: "ThreadControlBlock".to_string(),
            location: abi::ARG[0].to_string(),
        }],
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: FRAME_SIZE,
            callee_saved: vec![
                abi::link_register().to_string(),
                abi::CURRENT_THREAD.to_string(),
            ],
        },
        stack_slots: Vec::new(),
        instructions,
        relocations,
    })
}
