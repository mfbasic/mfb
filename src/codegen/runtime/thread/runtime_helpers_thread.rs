// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
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

pub(crate) enum ThreadSimpleOp {
    IsRunning,
    WaitFor,
    Cancel,
    Drop,
    Poll,
}

pub(crate) fn emit_thread_deadline(
    ctx: &mut EmitCtx,
    timeout_stack_offset: usize,
    timespec_stack_offset: usize,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    let done = format!("{symbol}_deadline_done_{timespec_stack_offset}");
    let nsec_ok = format!("{symbol}_deadline_nsec_ok_{timespec_stack_offset}");
    ctx.instructions.extend([
        abi::load_u64("%v9", abi::stack_pointer(), timeout_stack_offset),
        abi::compare_immediate("%v9", "0"),
        abi::branch_le(&done),
        abi::move_immediate(abi::c_arg(0), "Integer", "0"),
        abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), timespec_stack_offset),
    ]);
    emit_thread_external_call(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        "clock_gettime",
    )?;
    ctx.instructions.extend([
        abi::load_u64("%v9", abi::stack_pointer(), timeout_stack_offset),
        abi::move_immediate("%v10", "Integer", "1000"),
        abi::signed_divide_registers("%v11", "%v9", "%v10"),
        abi::multiply_subtract_registers("%v12", "%v11", "%v10", "%v9"),
        abi::move_immediate("%v13", "Integer", "1000000"),
        abi::multiply_registers("%v12", "%v12", "%v13"),
        abi::load_u64("%v14", abi::stack_pointer(), timespec_stack_offset),
        abi::add_registers("%v14", "%v14", "%v11"),
        abi::load_u64("%v15", abi::stack_pointer(), timespec_stack_offset + 8),
        abi::add_registers("%v15", "%v15", "%v12"),
        abi::move_immediate("%v13", "Integer", "1000000000"),
        abi::compare_registers("%v15", "%v13"),
        abi::branch_lt(&nsec_ok),
        abi::subtract_registers("%v15", "%v15", "%v13"),
        abi::add_immediate("%v14", "%v14", 1),
        abi::label(&nsec_ok),
        abi::store_u64("%v14", abi::stack_pointer(), timespec_stack_offset),
        abi::store_u64("%v15", abi::stack_pointer(), timespec_stack_offset + 8),
        abi::label(&done),
    ]);
    Ok(())
}

/// The byte size of a worker's arena-state block: the arena state, the entry's seed
/// scratch word and the program's writable globals (bug-369). `thread::start` allocates it
/// from the spawning thread's arena at this size and `thread.drop` frees it at the same
/// size (bug-622).
pub(crate) fn worker_arena_state_size(arena_global_slots: usize) -> usize {
    ENTRY_GLOBALS_OFFSET + arena_global_slots * 8
}

/// bug-622: what `thread.drop` needs to free a finished thread's plumbing — the worker
/// arena state's size, and whether that arena is in the `--debug` arena registry.
#[derive(Clone, Copy)]
pub(crate) struct ThreadRelease {
    pub(crate) worker_arena_size: usize,
    pub(crate) debug_arena_registry: bool,
}

/// bug-622: free a finished thread's plumbing — everything `thread::start` carved out of
/// the spawning thread's arena: the four queues (each queue record, its value ring and the
/// orphaned message copies parked on its pending-free list), the worker arena-state block,
/// and last the control block itself.
///
/// Sound only once the worker has been JOINED: the trampoline still unlocks the outbound
/// mutex after publishing `COMPLETED`, and the worker's pinned arena register points into
/// the state block until it returns. Emitted on the calling helper's frame — the handle is
/// parked at `handle_offset` and `queue_offset` is a free scratch word — and every
/// `arena_free` clobbers the caller-saved registers, so both are reloaded after each call.
///
/// A pending-free node was carved by whichever side's send failed; freeing it here adopts
/// it into this thread's arena, exactly as the queue read's drain already does
/// (bug-147.5b), and with the worker joined nothing can race it. The worker arena's own
/// chunks are not reachable from here and stay mapped (`planning/todo.md` Bucket List 1).
fn emit_release_thread_plumbing(
    ctx: &mut EmitCtx,
    handle_offset: usize,
    queue_offset: usize,
    ring_index_offset: usize,
    ring_left_offset: usize,
    ring_entry_offset: usize,
    ring_state_offset: usize,
    release: ThreadRelease,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    // `thread.drop` releases from two sites (a handle `waitFor` joined, and one it joins
    // itself), so every label carries the emission point to stay unique in the helper.
    let site = ctx.instructions.len();
    for cb_queue_offset in [
        THREAD_OFFSET_INBOUND_QUEUE,
        THREAD_OFFSET_OUTBOUND_QUEUE,
        THREAD_OFFSET_RESOURCE_INBOUND_QUEUE,
        THREAD_OFFSET_RESOURCE_OUTBOUND_QUEUE,
    ] {
        let absent = format!("{symbol}_release_{site}_queue_{cb_queue_offset}_absent");
        let drain = format!("{symbol}_release_{site}_queue_{cb_queue_offset}_drain");
        let drained = format!("{symbol}_release_{site}_queue_{cb_queue_offset}_drained");
        let retire = format!("{symbol}_release_{site}_queue_{cb_queue_offset}_retire");
        let ring_loop = format!("{symbol}_release_{site}_queue_{cb_queue_offset}_ring");
        let ring_wrap = format!("{symbol}_release_{site}_queue_{cb_queue_offset}_ring_wrap");
        let ring_done = format!("{symbol}_release_{site}_queue_{cb_queue_offset}_ring_done");
        let ring_state_done =
            format!("{symbol}_release_{site}_queue_{cb_queue_offset}_ring_state_done");
        let drain_state_done =
            format!("{symbol}_release_{site}_queue_{cb_queue_offset}_drain_state_done");
        // bug-646: THIS thread is the queue's sender only on the two INBOUND queues (it
        // spawned the worker, so it is the one that `send`s / `transfer`s into them), and
        // every block on a pending-free list was carved by that queue's sender (bug-498).
        // Reclaiming an OUTBOUND queue's list here would push the WORKER's blocks onto
        // this thread's bins — sound as adoption, but it charges a free to an arena that
        // never allocated the block (`free_calls` outruns `alloc_calls` in the `--debug`
        // report, which is the signature of a double free) and it is not this thread's
        // memory to recycle. An outbound list is drained by the worker's own next
        // `thread::send`; whatever is left when it exits stays in the worker's arena,
        // where it was carved.
        let reclaims_pending_free = matches!(
            cb_queue_offset,
            THREAD_OFFSET_INBOUND_QUEUE | THREAD_OFFSET_RESOURCE_INBOUND_QUEUE
        );
        // bug-650 case 2: only a resource-plane block carries a STATE block, and only
        // there is a node's / entry's third word guaranteed to be in range.
        let resource_plane = matches!(
            cb_queue_offset,
            THREAD_OFFSET_RESOURCE_INBOUND_QUEUE | THREAD_OFFSET_RESOURCE_OUTBOUND_QUEUE
        );
        ctx.instructions.extend([
            abi::load_u64("%v8", abi::stack_pointer(), handle_offset),
            abi::load_u64("%v9", "%v8", cb_queue_offset),
            abi::compare_immediate("%v9", "0"),
            abi::branch_eq(&absent),
            abi::store_u64("%v9", abi::stack_pointer(), queue_offset),
        ]);
        if reclaims_pending_free {
            ctx.instructions.extend([
                // bug-646: the block the reader's LAST read handed out was never retired
                // (no further read followed it), so park it on the pending-free list here
                // and let the drain below reclaim it with everything else. The worker is
                // joined by the time the plumbing is released, so this is the only thread
                // touching the queue and the mutex is not needed. Size 0 = not
                // reclaimable; drop the reference.
                abi::load_u64("%v10", "%v9", THREAD_QUEUE_LAST_READ_PTR_OFFSET),
                abi::compare_immediate("%v10", "0"),
                abi::branch_eq(&retire),
                abi::load_u64("%v11", "%v9", THREAD_QUEUE_LAST_READ_SIZE_OFFSET),
                abi::compare_immediate("%v11", "0"),
                abi::branch_eq(&retire),
                abi::load_u64("%v12", "%v9", THREAD_QUEUE_PENDING_FREE_OFFSET),
                abi::store_u64("%v12", "%v10", 0),
                abi::store_u64("%v11", "%v10", 8),
            ]);
            if resource_plane {
                // bug-650 case 2: and its STATE size, in the node's third word — on the
                // resource planes ONLY. A data-plane block can be smaller than 24 bytes
                // (a short String is `len + 9`), so writing +16 there would run past the
                // allocation.
                ctx.instructions.extend([
                    abi::load_u64("%v11", "%v9", THREAD_QUEUE_LAST_READ_STATE_SIZE_OFFSET),
                    abi::store_u64("%v11", "%v10", PENDING_FREE_STATE_SIZE),
                ]);
            }
            ctx.instructions.extend([
                abi::store_u64("%v10", "%v9", THREAD_QUEUE_PENDING_FREE_OFFSET),
                abi::label(&retire),
                abi::store_u64(abi::ZERO, "%v9", THREAD_QUEUE_LAST_READ_PTR_OFFSET),
                abi::store_u64(abi::ZERO, "%v9", THREAD_QUEUE_LAST_READ_SIZE_OFFSET),
                abi::store_u64(abi::ZERO, "%v9", THREAD_QUEUE_LAST_READ_STATE_SIZE_OFFSET),
                // bug-650 case 1: every message still IN the ring — sent but never
                // received — with its `{value, size}` entry. bug-646's protocol reaches
                // only a block a read handed out, because that is the only point it
                // learns of one; an undelivered block is reachable solely by walking the
                // ring, which nothing did, so a program that sends more than its worker
                // reads leaked all of it. Walk `count` entries from `head`, the same
                // window `thread_queue_read_helper` dequeues from, and free each.
                //
                // Same ownership rule as the pending-free drain this sits inside: only an
                // INBOUND queue, whose blocks THIS thread carved (bug-498), so the free
                // returns them to the arena that allocated them. Size 0 keeps the
                // established fail-safe — skip rather than guess a size.
                //
                // The loop counter and index live in the frame, not registers:
                // `arena_free` clobbers every caller-saved register, and both are
                // advanced BEFORE the call so the next iteration reloads a consistent
                // pair whatever the free did.
                abi::load_u64("%v10", "%v9", THREAD_QUEUE_HEAD_OFFSET),
                abi::store_u64("%v10", abi::stack_pointer(), ring_index_offset),
                abi::load_u64("%v10", "%v9", THREAD_QUEUE_COUNT_OFFSET),
                abi::store_u64("%v10", abi::stack_pointer(), ring_left_offset),
                abi::label(&ring_loop),
                abi::load_u64("%v10", abi::stack_pointer(), ring_left_offset),
                abi::compare_immediate("%v10", "0"),
                abi::branch_eq(&ring_done),
                abi::subtract_immediate("%v10", "%v10", 1),
                abi::store_u64("%v10", abi::stack_pointer(), ring_left_offset),
                abi::load_u64("%v9", abi::stack_pointer(), queue_offset),
                abi::load_u64("%v10", abi::stack_pointer(), ring_index_offset),
                abi::load_u64("%v11", "%v9", THREAD_QUEUE_VALUES_OFFSET),
                abi::shift_left_immediate("%v12", "%v10", THREAD_QUEUE_ENTRY_SHIFT),
                abi::add_registers("%v11", "%v11", "%v12"),
                abi::store_u64("%v11", abi::stack_pointer(), ring_entry_offset),
                // Advance the index (wrapping at capacity) before the free below.
                abi::add_immediate("%v10", "%v10", 1),
                abi::load_u64("%v12", "%v9", THREAD_QUEUE_CAPACITY_OFFSET),
                abi::compare_registers("%v10", "%v12"),
                abi::branch_lt(&ring_wrap),
                abi::move_immediate("%v10", "Integer", "0"),
                abi::label(&ring_wrap),
                abi::store_u64("%v10", abi::stack_pointer(), ring_index_offset),
                abi::load_u64(abi::c_arg(0), "%v11", 0),
                abi::compare_immediate(abi::c_arg(0), "0"),
                abi::branch_eq(&ring_loop),
            ]);
            if resource_plane {
                // bug-650 case 2: an undelivered resource record's STATE block, freed
                // before the record that points at it. The record pointer is re-derived
                // from the frame after the call.
                ctx.instructions.extend([
                    abi::store_u64(abi::c_arg(0), abi::stack_pointer(), ring_state_offset),
                    abi::load_u64(abi::c_arg(1), "%v11", THREAD_QUEUE_ENTRY_STATE_SIZE),
                    abi::compare_immediate(abi::c_arg(1), "0"),
                    abi::branch_eq(&ring_state_done),
                    abi::load_u64(abi::c_arg(0), abi::c_arg(0), RESOURCE_OFFSET_STATE),
                    abi::compare_immediate(abi::c_arg(0), "0"),
                    abi::branch_eq(&ring_state_done),
                    abi::branch_link(ARENA_FREE_SYMBOL),
                ]);
                ctx.relocations
                    .push(internal_branch(symbol, ARENA_FREE_SYMBOL));
                ctx.instructions.extend([
                    abi::label(&ring_state_done),
                    abi::load_u64(abi::c_arg(0), abi::stack_pointer(), ring_state_offset),
                    abi::load_u64("%v9", abi::stack_pointer(), queue_offset),
                    abi::load_u64("%v11", abi::stack_pointer(), ring_entry_offset),
                ]);
            }
            ctx.instructions.extend([
                abi::load_u64(abi::c_arg(1), "%v11", 8),
                abi::compare_immediate(abi::c_arg(1), "0"),
                abi::branch_eq(&ring_loop),
                abi::branch_link(ARENA_FREE_SYMBOL),
            ]);
            ctx.relocations
                .push(internal_branch(symbol, ARENA_FREE_SYMBOL));
            ctx.instructions.extend([
                abi::branch(&ring_loop),
                abi::label(&ring_done),
                // The ring is about to be freed; leave the counters consistent so nothing
                // that reads the queue between here and the free sees a phantom message.
                abi::load_u64("%v9", abi::stack_pointer(), queue_offset),
                abi::store_u64(abi::ZERO, "%v9", THREAD_QUEUE_COUNT_OFFSET),
                abi::store_u64(abi::ZERO, "%v9", THREAD_QUEUE_HEAD_OFFSET),
                abi::store_u64(abi::ZERO, "%v9", THREAD_QUEUE_TAIL_OFFSET),
                abi::label(&drain),
                abi::load_u64("%v9", abi::stack_pointer(), queue_offset),
                abi::load_u64("%v10", "%v9", THREAD_QUEUE_PENDING_FREE_OFFSET),
                abi::compare_immediate("%v10", "0"),
                abi::branch_eq(&drained),
                abi::load_u64("%v11", "%v10", 0),
                abi::store_u64("%v11", "%v9", THREAD_QUEUE_PENDING_FREE_OFFSET),
            ]);
            if resource_plane {
                // bug-650 case 2: the record's STATE block first, while the record is
                // still readable — its pointer lives at RESOURCE_OFFSET_STATE, which the
                // node's `{next, size, state_size}` words do not reach. The node pointer
                // is parked in the frame across the call, since `arena_free` clobbers
                // every caller-saved register.
                ctx.instructions.extend([
                    abi::store_u64("%v10", abi::stack_pointer(), ring_index_offset),
                    abi::load_u64(abi::c_arg(1), "%v10", PENDING_FREE_STATE_SIZE),
                    abi::compare_immediate(abi::c_arg(1), "0"),
                    abi::branch_eq(&drain_state_done),
                    abi::load_u64(abi::c_arg(0), "%v10", RESOURCE_OFFSET_STATE),
                    abi::compare_immediate(abi::c_arg(0), "0"),
                    abi::branch_eq(&drain_state_done),
                    abi::branch_link(ARENA_FREE_SYMBOL),
                ]);
                ctx.relocations
                    .push(internal_branch(symbol, ARENA_FREE_SYMBOL));
                ctx.instructions.extend([
                    abi::label(&drain_state_done),
                    abi::load_u64("%v10", abi::stack_pointer(), ring_index_offset),
                ]);
            }
            ctx.instructions.extend([
                abi::load_u64(abi::c_arg(1), "%v10", 8),
                abi::move_register(abi::c_arg(0), "%v10"),
                abi::branch_link(ARENA_FREE_SYMBOL),
            ]);
            ctx.relocations
                .push(internal_branch(symbol, ARENA_FREE_SYMBOL));
            ctx.instructions
                .extend([abi::branch(&drain), abi::label(&drained)]);
        }
        // The value ring: `capacity * THREAD_QUEUE_ENTRY_SIZE` bytes, the size
        // `emit_thread_queue_alloc` gave it.
        ctx.instructions.extend([
            abi::load_u64("%v9", abi::stack_pointer(), queue_offset),
            abi::load_u64("%v10", "%v9", THREAD_QUEUE_CAPACITY_OFFSET),
            abi::move_immediate("%v11", "Integer", &THREAD_QUEUE_ENTRY_SIZE.to_string()),
            abi::multiply_registers(abi::c_arg(1), "%v10", "%v11"),
            abi::load_u64(abi::c_arg(0), "%v9", THREAD_QUEUE_VALUES_OFFSET),
            abi::branch_link(ARENA_FREE_SYMBOL),
        ]);
        ctx.relocations
            .push(internal_branch(symbol, ARENA_FREE_SYMBOL));
        ctx.instructions.extend([
            abi::load_u64(abi::c_arg(0), abi::stack_pointer(), queue_offset),
            abi::move_immediate(
                abi::c_arg(1),
                "Integer",
                &THREAD_QUEUE_BLOCK_SIZE.to_string(),
            ),
            abi::branch_link(ARENA_FREE_SYMBOL),
        ]);
        ctx.relocations
            .push(internal_branch(symbol, ARENA_FREE_SYMBOL));
        ctx.instructions.push(abi::label(&absent));
    }
    let no_worker_arena = format!("{symbol}_release_{site}_no_worker_arena");
    ctx.instructions.extend([
        abi::load_u64("%v8", abi::stack_pointer(), handle_offset),
        abi::load_u64("%v9", "%v8", THREAD_OFFSET_ARENA_STATE),
        abi::compare_immediate("%v9", "0"),
        abi::branch_eq(&no_worker_arena),
    ]);
    if release.debug_arena_registry {
        crate::codegen::debug::arena::emit_debug_arena_unregister(
            symbol,
            "%v9",
            ["%v10", "%v11", "%v12", "%v13"],
            ctx.instructions,
            ctx.relocations,
        );
    }
    ctx.instructions.extend([
        abi::move_register(abi::c_arg(0), "%v9"),
        abi::move_immediate(
            abi::c_arg(1),
            "Integer",
            &release.worker_arena_size.to_string(),
        ),
        abi::branch_link(ARENA_FREE_SYMBOL),
    ]);
    ctx.relocations
        .push(internal_branch(symbol, ARENA_FREE_SYMBOL));
    ctx.instructions.extend([
        abi::label(&no_worker_arena),
        abi::load_u64(abi::c_arg(0), abi::stack_pointer(), handle_offset),
        abi::move_immediate(abi::c_arg(1), "Integer", &THREAD_BLOCK_SIZE.to_string()),
        abi::branch_link(ARENA_FREE_SYMBOL),
    ]);
    ctx.relocations
        .push(internal_branch(symbol, ARENA_FREE_SYMBOL));
    Ok(())
}

pub(crate) fn simple_thread_handle_helper(
    symbol: &str,
    op: ThreadSimpleOp,
    release: ThreadRelease,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<ThreadBodyParts, String> {
    // bug-650: 64 -> 96 for the four words `emit_release_thread_plumbing`'s ring walk and
    // STATE-block frees keep in the frame (an `arena_free` clobbers every caller-saved
    // register, so none of them can live in one).
    const FRAME_SIZE: usize = 96;
    const HANDLE_OFFSET: usize = 8;
    const VALUE_OFFSET: usize = 16;
    const TAG_OFFSET: usize = 24;
    const ERROR_OFFSET: usize = 32;
    // WaitFor only: origin ErrorLoc of a propagated worker error (0 otherwise).
    const SOURCE_OFFSET: usize = 40;
    // Drop only: the queue whose blocks `emit_release_thread_plumbing` is freeing.
    const RELEASE_QUEUE_OFFSET: usize = 48;
    // Drop only: the drop mode (`THREAD_DROP_CLOSE` | `THREAD_DROP_RELEASE`), rewritten
    // to `DROP_MODE_FREE` when this drop's release takes the owner count to 0.
    const MODE_OFFSET: usize = 56;
    // Drop only: the ring-walk cursor and remaining count (bug-650 case 1).
    const RING_INDEX_OFFSET: usize = 64;
    const RING_LEFT_OFFSET: usize = 72;
    // Drop only: the ring entry being drained, and the record whose STATE block is being
    // freed ahead of it (bug-650 case 2). Both survive an `arena_free`'s register clobber.
    const RING_ENTRY_OFFSET: usize = 80;
    const RING_STATE_OFFSET: usize = 88;
    const DROP_MODE_FREE: &str = "7";

    let mut instructions = Vec::new();
    let mut relocations = Vec::new();
    instructions.extend([abi::store_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        HANDLE_OFFSET,
    )]);
    // bug-622: a drop settles its owner count before anything else. A close-only drop (a
    // trap route) is not counted. Otherwise the release takes one owner away; reaching 0
    // turns the drop into close-and-free, and a release-only drop that leaves other owners
    // (a moved-from caller) is done — it neither closes nor frees. Every handle is
    // counted, including the zeroed CLOSED handle an inline `TRAP` on `thread::start`
    // binds (it starts at 1, and its last release frees just its block).
    if matches!(op, ThreadSimpleOp::Drop) {
        let counted = format!("{symbol}_owners_counted");
        let keep = format!("{symbol}_precheck_keep");
        instructions.extend([
            abi::store_u64(abi::c_arg(1), abi::stack_pointer(), MODE_OFFSET),
            abi::load_u64("%v9", abi::stack_pointer(), MODE_OFFSET),
            abi::compare_immediate("%v9", &THREAD_DROP_CLOSE.to_string()),
            abi::branch_eq(&counted),
            abi::load_u64("%v10", abi::c_arg(0), THREAD_OFFSET_OWNERS),
            abi::subtract_immediate("%v10", "%v10", 1),
            abi::store_u64("%v10", abi::c_arg(0), THREAD_OFFSET_OWNERS),
            abi::compare_immediate("%v10", "0"),
            abi::branch_ne(&counted),
            abi::move_immediate("%v9", "Integer", DROP_MODE_FREE),
            abi::store_u64("%v9", abi::stack_pointer(), MODE_OFFSET),
            abi::label(&counted),
            abi::load_u64("%v9", abi::stack_pointer(), MODE_OFFSET),
            abi::compare_immediate("%v9", &THREAD_DROP_RELEASE.to_string()),
            abi::branch_eq(&keep),
        ]);
    }
    // bug-479: answer for a CLOSED handle BEFORE touching its queue.
    //
    // Every arm below opens by loading `THREAD_OFFSET_OUTBOUND_QUEUE` (or the
    // inbound one) and `pthread_mutex_lock`ing it, and only reads
    // `THREAD_OFFSET_STATE` once the lock is held. That is fine for a handle
    // `thread::start` built, and fatal for the one the error path of an inline
    // `TRAP` on `thread::start` binds: its queues are null, so the lock faults
    // before the op can notice the handle is closed. See
    // `emit_closed_thread_handle`.
    //
    // **The unlocked read is sound because `CLOSED` is terminal.** The only
    // writers of `THREAD_OFFSET_STATE` are the zero-init at creation
    // (`runtime_helpers.rs`, `RUNNING`), the worker storing `COMPLETED` as it
    // exits, and the two sites here that store `CLOSED` — nothing ever moves a
    // handle OUT of `CLOSED`. So a racing read that sees `CLOSED` is right
    // forever, and one that sees anything else falls through to the existing
    // locked path, which re-reads the state under the lock and decides there. The
    // pre-check can only ever short-circuit; it can never decide differently from
    // the code it precedes.
    //
    // The answers are the ones each op already gives a closed handle, so this
    // invents no contract: `ErrResourceClosed` for every query/wait/cancel, and
    // success for `Drop`, which already treats an already-closed handle as a
    // no-op.
    let precheck_closed = format!("{symbol}_precheck_closed");
    let precheck_done = format!("{symbol}_precheck_done");
    instructions.extend([
        abi::load_u64("%v9", abi::c_arg(0), THREAD_OFFSET_STATE),
        abi::compare_immediate("%v9", THREAD_STATE_CLOSED),
        abi::branch_eq(&precheck_closed),
    ]);
    match op {
        ThreadSimpleOp::IsRunning => {
            let running = format!("{symbol}_running");
            let closed = format!("{symbol}_closed");
            let done = format!("{symbol}_done");
            instructions.extend([
                abi::load_u64("%v9", abi::c_arg(0), THREAD_OFFSET_OUTBOUND_QUEUE),
                abi::move_register(abi::c_arg(0), "%v9"),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_mutex_lock",
            )?;
            instructions.extend([
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("%v9", "%v8", THREAD_OFFSET_STATE),
                abi::store_u64("%v9", abi::stack_pointer(), VALUE_OFFSET),
                abi::load_u64(abi::c_arg(0), "%v8", THREAD_OFFSET_OUTBOUND_QUEUE),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_mutex_unlock",
            )?;
            instructions.extend([
                abi::load_u64("%v9", abi::stack_pointer(), VALUE_OFFSET),
                abi::compare_immediate("%v9", THREAD_STATE_CLOSED),
                abi::branch_eq(&closed),
                abi::compare_immediate("%v9", THREAD_STATE_RUNNING),
                abi::branch_eq(&running),
                abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
                abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
                abi::branch(&done),
                abi::label(&running),
                abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
                abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
                abi::branch(&done),
                abi::label(&closed),
            ]);
            raise_error_into(
                symbol,
                "ErrResourceClosed",
                &mut instructions,
                &mut relocations,
            );
            instructions.extend([abi::label(&done)]);
        }
        ThreadSimpleOp::WaitFor => {
            let loop_label = format!("{symbol}_wait_loop");
            let closed = format!("{symbol}_closed");
            let result_ready = format!("{symbol}_result_ready");
            let done = format!("{symbol}_done");
            instructions.extend([
                abi::load_u64("%v9", abi::c_arg(0), THREAD_OFFSET_OUTBOUND_QUEUE),
                abi::move_register(abi::c_arg(0), "%v9"),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_mutex_lock",
            )?;
            instructions.extend([
                abi::label(&loop_label),
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("%v9", "%v8", THREAD_OFFSET_STATE),
                abi::compare_immediate("%v9", THREAD_STATE_CLOSED),
                abi::branch_eq(&closed),
                abi::compare_immediate("%v9", THREAD_STATE_COMPLETED),
                abi::branch_eq(&result_ready),
                abi::load_u64("%v9", "%v8", THREAD_OFFSET_OUTBOUND_QUEUE),
                abi::add_immediate(abi::c_arg(0), "%v9", THREAD_QUEUE_NOT_EMPTY_OFFSET),
                abi::move_register(abi::c_arg(1), "%v9"),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_cond_wait",
            )?;
            instructions.extend([
                abi::branch(&loop_label),
                abi::label(&result_ready),
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64(
                    RESULT_ERROR_MESSAGE_REGISTER,
                    "%v8",
                    THREAD_OFFSET_RESULT_ERROR,
                ),
                abi::load_u64(RESULT_VALUE_REGISTER, "%v8", THREAD_OFFSET_RESULT_VALUE),
                abi::load_u64(RESULT_TAG_REGISTER, "%v8", THREAD_OFFSET_RESULT_TAG),
                abi::load_u64(
                    RESULT_ERROR_SOURCE_REGISTER,
                    "%v8",
                    THREAD_OFFSET_RESULT_SOURCE,
                ),
                abi::store_u64(
                    RESULT_ERROR_MESSAGE_REGISTER,
                    abi::stack_pointer(),
                    ERROR_OFFSET,
                ),
                abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
                abi::store_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TAG_OFFSET),
                abi::store_u64(
                    RESULT_ERROR_SOURCE_REGISTER,
                    abi::stack_pointer(),
                    SOURCE_OFFSET,
                ),
                abi::move_immediate("%v9", "Integer", THREAD_STATE_CLOSED),
                abi::store_u64("%v9", "%v8", THREAD_OFFSET_STATE),
                abi::load_u64("%v10", "%v8", THREAD_OFFSET_OUTBOUND_QUEUE),
                abi::store_u64("%v9", "%v10", THREAD_QUEUE_CLOSED_OFFSET),
                abi::store_u64(abi::ZERO, "%v10", THREAD_QUEUE_COUNT_OFFSET),
                abi::load_u64(abi::c_arg(0), "%v8", THREAD_OFFSET_OUTBOUND_QUEUE),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_mutex_unlock",
            )?;
            // bug-622: join, not detach. `COMPLETED` is stored under the outbound mutex,
            // but the trampoline still unlocks that mutex and returns after it, so the
            // handle's plumbing is not free-able until the worker has exited — and the
            // join is the one point the parent knows it has. It is prompt: nothing is
            // left for the worker to do but return. The zeroed OS handle then marks the
            // handle JOINED, which is what licenses `thread.drop` to free its blocks.
            instructions.extend([
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64(abi::c_arg(0), "%v8", THREAD_OFFSET_OS_HANDLE),
                abi::move_immediate(abi::c_arg(1), "Integer", "0"),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_join",
            )?;
            instructions.extend([
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::store_u64(abi::ZERO, "%v8", THREAD_OFFSET_OS_HANDLE),
                abi::load_u64(
                    RESULT_ERROR_MESSAGE_REGISTER,
                    abi::stack_pointer(),
                    ERROR_OFFSET,
                ),
                abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
                abi::load_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TAG_OFFSET),
                abi::branch(&done),
                abi::label(&closed),
            ]);
            raise_error_into(
                symbol,
                "ErrResourceClosed",
                &mut instructions,
                &mut relocations,
            );
            instructions.extend([
                abi::store_u64(
                    RESULT_ERROR_MESSAGE_REGISTER,
                    abi::stack_pointer(),
                    ERROR_OFFSET,
                ),
                abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
                abi::store_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TAG_OFFSET),
                // waitFor's own error (resource closed): no worker origin.
                abi::store_u64(abi::ZERO, abi::stack_pointer(), SOURCE_OFFSET),
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64(abi::c_arg(0), "%v8", THREAD_OFFSET_OUTBOUND_QUEUE),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_mutex_unlock",
            )?;
            instructions.extend([
                abi::load_u64(
                    RESULT_ERROR_MESSAGE_REGISTER,
                    abi::stack_pointer(),
                    ERROR_OFFSET,
                ),
                abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
                abi::load_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TAG_OFFSET),
                abi::label(&done),
                abi::load_u64(
                    RESULT_ERROR_SOURCE_REGISTER,
                    abi::stack_pointer(),
                    SOURCE_OFFSET,
                ),
            ]);
        }
        // Close + broadcast both resource-plane queues so a worker parked in a
        // blocking `acceptResource` (or a parent in `transferResource`) re-checks
        // CANCELLED/CLOSED and unblocks. cancel/drop previously touched only the
        // two data-plane queues, so such a worker never woke — a permanent hang and,
        // on drop, a detached leaked thread (bug-205). Mirrors the trampoline-exit
        // close loop; the handle lives at HANDLE_OFFSET on this helper's frame.
        ThreadSimpleOp::Cancel => {
            let closed = format!("{symbol}_closed");
            let closed_unlocked = format!("{symbol}_closed_unlocked");
            let inbound_unlocked = format!("{symbol}_inbound_unlocked");
            instructions.extend([
                abi::load_u64("%v9", abi::c_arg(0), THREAD_OFFSET_INBOUND_QUEUE),
                abi::move_register(abi::c_arg(0), "%v9"),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_mutex_lock",
            )?;
            instructions.extend([
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("%v9", "%v8", THREAD_OFFSET_STATE),
                abi::compare_immediate("%v9", THREAD_STATE_CLOSED),
                abi::branch_eq(&closed),
                abi::move_immediate("%v9", "Integer", "1"),
                abi::store_u64("%v9", "%v8", THREAD_OFFSET_CANCELLED),
                abi::load_u64("%v10", "%v8", THREAD_OFFSET_INBOUND_QUEUE),
                abi::store_u64("%v9", "%v10", THREAD_QUEUE_CLOSED_OFFSET),
                abi::add_immediate(abi::c_arg(0), "%v10", THREAD_QUEUE_NOT_EMPTY_OFFSET),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_cond_broadcast",
            )?;
            instructions.extend([
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("%v10", "%v8", THREAD_OFFSET_INBOUND_QUEUE),
                abi::add_immediate(abi::c_arg(0), "%v10", THREAD_QUEUE_NOT_FULL_OFFSET),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_cond_broadcast",
            )?;
            instructions.extend([
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64(abi::c_arg(0), "%v8", THREAD_OFFSET_INBOUND_QUEUE),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_mutex_unlock",
            )?;
            instructions.extend([
                abi::label(&inbound_unlocked),
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("%v9", "%v8", THREAD_OFFSET_OUTBOUND_QUEUE),
                abi::move_register(abi::c_arg(0), "%v9"),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_mutex_lock",
            )?;
            instructions.extend([
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::move_immediate("%v9", "Integer", "1"),
                abi::load_u64("%v10", "%v8", THREAD_OFFSET_OUTBOUND_QUEUE),
                abi::store_u64("%v9", "%v10", THREAD_QUEUE_CLOSED_OFFSET),
                abi::add_immediate(abi::c_arg(0), "%v10", THREAD_QUEUE_NOT_EMPTY_OFFSET),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_cond_broadcast",
            )?;
            instructions.extend([
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("%v10", "%v8", THREAD_OFFSET_OUTBOUND_QUEUE),
                abi::add_immediate(abi::c_arg(0), "%v10", THREAD_QUEUE_NOT_FULL_OFFSET),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_cond_broadcast",
            )?;
            instructions.extend([
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64(abi::c_arg(0), "%v8", THREAD_OFFSET_OUTBOUND_QUEUE),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_mutex_unlock",
            )?;
            // Wake anyone parked on the resource plane too (bug-205).
            emit_close_resource_queues(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                HANDLE_OFFSET,
            )?;
            instructions.extend([
                abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
                abi::branch(&closed_unlocked),
                abi::label(&closed),
            ]);
            raise_error_into(
                symbol,
                "ErrResourceClosed",
                &mut instructions,
                &mut relocations,
            );
            instructions.extend([
                abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
                abi::store_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TAG_OFFSET),
                abi::store_u64(
                    RESULT_ERROR_MESSAGE_REGISTER,
                    abi::stack_pointer(),
                    ERROR_OFFSET,
                ),
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64(abi::c_arg(0), "%v8", THREAD_OFFSET_INBOUND_QUEUE),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_mutex_unlock",
            )?;
            instructions.extend([
                abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
                abi::load_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TAG_OFFSET),
                abi::load_u64(
                    RESULT_ERROR_MESSAGE_REGISTER,
                    abi::stack_pointer(),
                    ERROR_OFFSET,
                ),
                abi::label(&closed_unlocked),
            ]);
        }
        ThreadSimpleOp::Drop => {
            let already_closed = format!("{symbol}_already_closed");
            let outbound_unlocked = format!("{symbol}_outbound_unlocked");
            let inbound_unlocked = format!("{symbol}_inbound_unlocked");
            let done = format!("{symbol}_done");
            instructions.extend([
                abi::load_u64("%v9", abi::c_arg(0), THREAD_OFFSET_OUTBOUND_QUEUE),
                abi::move_register(abi::c_arg(0), "%v9"),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_mutex_lock",
            )?;
            instructions.extend([
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("%v9", "%v8", THREAD_OFFSET_STATE),
                abi::store_u64("%v9", abi::stack_pointer(), VALUE_OFFSET),
                abi::compare_immediate("%v9", THREAD_STATE_CLOSED),
                abi::branch_eq(&already_closed),
                abi::move_immediate("%v9", "Integer", THREAD_STATE_CLOSED),
                abi::store_u64("%v9", "%v8", THREAD_OFFSET_STATE),
                abi::load_u64("%v10", "%v8", THREAD_OFFSET_OUTBOUND_QUEUE),
                abi::store_u64("%v9", "%v10", THREAD_QUEUE_CLOSED_OFFSET),
                abi::store_u64(abi::ZERO, "%v10", THREAD_QUEUE_COUNT_OFFSET),
                abi::store_u64(abi::ZERO, "%v10", THREAD_QUEUE_HEAD_OFFSET),
                abi::store_u64(abi::ZERO, "%v10", THREAD_QUEUE_TAIL_OFFSET),
                abi::add_immediate(abi::c_arg(0), "%v10", THREAD_QUEUE_NOT_EMPTY_OFFSET),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_cond_broadcast",
            )?;
            instructions.extend([
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("%v10", "%v8", THREAD_OFFSET_OUTBOUND_QUEUE),
                abi::add_immediate(abi::c_arg(0), "%v10", THREAD_QUEUE_NOT_FULL_OFFSET),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_cond_broadcast",
            )?;
            instructions.push(abi::label(&already_closed));
            instructions.extend([
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64(abi::c_arg(0), "%v8", THREAD_OFFSET_OUTBOUND_QUEUE),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_mutex_unlock",
            )?;
            instructions.extend([
                abi::load_u64("%v9", abi::stack_pointer(), VALUE_OFFSET),
                abi::compare_immediate("%v9", THREAD_STATE_CLOSED),
                abi::branch_eq(&done),
                abi::label(&outbound_unlocked),
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("%v9", "%v8", THREAD_OFFSET_INBOUND_QUEUE),
                abi::move_register(abi::c_arg(0), "%v9"),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_mutex_lock",
            )?;
            instructions.extend([
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::move_immediate("%v9", "Integer", "1"),
                abi::store_u64("%v9", "%v8", THREAD_OFFSET_CANCELLED),
                abi::load_u64("%v10", "%v8", THREAD_OFFSET_INBOUND_QUEUE),
                abi::store_u64("%v9", "%v10", THREAD_QUEUE_CLOSED_OFFSET),
                abi::store_u64(abi::ZERO, "%v10", THREAD_QUEUE_COUNT_OFFSET),
                abi::store_u64(abi::ZERO, "%v10", THREAD_QUEUE_HEAD_OFFSET),
                abi::store_u64(abi::ZERO, "%v10", THREAD_QUEUE_TAIL_OFFSET),
                abi::add_immediate(abi::c_arg(0), "%v10", THREAD_QUEUE_NOT_EMPTY_OFFSET),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_cond_broadcast",
            )?;
            instructions.extend([
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("%v10", "%v8", THREAD_OFFSET_INBOUND_QUEUE),
                abi::add_immediate(abi::c_arg(0), "%v10", THREAD_QUEUE_NOT_FULL_OFFSET),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_cond_broadcast",
            )?;
            instructions.extend([
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64(abi::c_arg(0), "%v8", THREAD_OFFSET_INBOUND_QUEUE),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_mutex_unlock",
            )?;
            instructions.push(abi::label(&inbound_unlocked));
            // Wake anyone parked on the resource plane before detaching, or a worker
            // blocked in acceptResource never observes CANCELLED and the detached
            // thread leaks forever (bug-205).
            emit_close_resource_queues(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                HANDLE_OFFSET,
            )?;
            // bug-622: a worker this drop saw `COMPLETED` (the state parked at
            // `VALUE_OFFSET` was read under the outbound mutex) has nothing left to do
            // but unlock and return, so join it and free its plumbing. A RUNNING worker
            // is cancelled and detached as before: it still holds pointers into the
            // control block and its arena state, so nothing can be freed from here
            // (a running worker's plumbing is reclaimed at process exit).
            let detach = format!("{symbol}_detach");
            instructions.extend([
                abi::load_u64("%v9", abi::stack_pointer(), VALUE_OFFSET),
                abi::compare_immediate("%v9", THREAD_STATE_COMPLETED),
                abi::branch_ne(&detach),
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64(abi::c_arg(0), "%v8", THREAD_OFFSET_OS_HANDLE),
                abi::move_immediate(abi::c_arg(1), "Integer", "0"),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_join",
            )?;
            // Joined: zero the OS handle, exactly as `waitFor` marks its join. Only the
            // drop that released the LAST owner frees; otherwise another binding (an
            // alias, a moved-from caller, a trap handler) still reads the block, and the
            // drop that later takes the count to 0 frees it through the closed pre-check.
            let joined_kept = format!("{symbol}_joined_kept");
            instructions.extend([
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::store_u64(abi::ZERO, "%v8", THREAD_OFFSET_OS_HANDLE),
                abi::load_u64("%v9", abi::stack_pointer(), MODE_OFFSET),
                abi::compare_immediate("%v9", DROP_MODE_FREE),
                abi::branch_ne(&joined_kept),
            ]);
            emit_release_thread_plumbing(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                HANDLE_OFFSET,
                RELEASE_QUEUE_OFFSET,
                RING_INDEX_OFFSET,
                RING_LEFT_OFFSET,
                RING_ENTRY_OFFSET,
                RING_STATE_OFFSET,
                release,
            )?;
            instructions.extend([
                abi::label(&joined_kept),
                abi::branch(&done),
                abi::label(&detach),
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64(abi::c_arg(0), "%v8", THREAD_OFFSET_OS_HANDLE),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_detach",
            )?;
            instructions.extend([
                abi::label(&done),
                abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
            ]);
        }
        ThreadSimpleOp::Poll => {
            let ready = format!("{symbol}_ready");
            let closed = format!("{symbol}_closed");
            let invalid = format!("{symbol}_invalid_timeout");
            let wait_loop = format!("{symbol}_wait_loop");
            let wait_timed = format!("{symbol}_wait_timed");
            let not_ready = format!("{symbol}_not_ready");
            let locked_done = format!("{symbol}_locked_done");
            let done = format!("{symbol}_done");
            instructions.extend([
                abi::compare_immediate(abi::c_arg(1), "0"),
                abi::branch_lt(&invalid),
                abi::store_u64(abi::c_arg(1), abi::stack_pointer(), VALUE_OFFSET),
            ]);
            emit_thread_deadline(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                VALUE_OFFSET,
                ERROR_OFFSET,
            )?;
            instructions.extend([
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("%v9", "%v8", THREAD_OFFSET_OUTBOUND_QUEUE),
                abi::move_register(abi::c_arg(0), "%v9"),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_mutex_lock",
            )?;
            instructions.extend([
                abi::label(&wait_loop),
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("%v9", "%v8", THREAD_OFFSET_OUTBOUND_QUEUE),
                abi::load_u64("%v10", "%v8", THREAD_OFFSET_STATE),
                abi::compare_immediate("%v10", THREAD_STATE_CLOSED),
                abi::branch_eq(&closed),
                abi::load_u64("%v10", "%v9", THREAD_QUEUE_COUNT_OFFSET),
                abi::compare_immediate("%v10", "0"),
                abi::branch_gt(&ready),
                abi::load_u64("%v10", "%v8", THREAD_OFFSET_STATE),
                abi::compare_immediate("%v10", THREAD_STATE_COMPLETED),
                abi::branch_eq(&not_ready),
                abi::load_u64("%v10", abi::stack_pointer(), VALUE_OFFSET),
                abi::compare_immediate("%v10", "0"),
                abi::branch_gt(&wait_timed),
                abi::branch(&not_ready),
                abi::label(&wait_timed),
                abi::add_immediate(abi::c_arg(0), "%v9", THREAD_QUEUE_NOT_EMPTY_OFFSET),
                abi::move_register(abi::c_arg(1), "%v9"),
                abi::add_immediate(abi::c_arg(2), abi::stack_pointer(), ERROR_OFFSET),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_cond_timedwait",
            )?;
            instructions.extend([
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
                abi::label(&closed),
            ]);
            raise_error_into(
                symbol,
                "ErrResourceClosed",
                &mut instructions,
                &mut relocations,
            );
            instructions.extend([abi::branch(&locked_done), abi::label(&invalid)]);
            raise_error_into(
                symbol,
                "ErrInvalidArgument",
                &mut instructions,
                &mut relocations,
            );
            instructions.extend([
                abi::branch(&done),
                abi::label(&locked_done),
                abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
                abi::store_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TAG_OFFSET),
                abi::store_u64(
                    RESULT_ERROR_MESSAGE_REGISTER,
                    abi::stack_pointer(),
                    ERROR_OFFSET,
                ),
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64(abi::c_arg(0), "%v8", THREAD_OFFSET_OUTBOUND_QUEUE),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_mutex_unlock",
            )?;
            instructions.extend([
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
    instructions.push(abi::branch(&precheck_done));
    instructions.push(abi::label(&precheck_closed));
    match op {
        // `thread.drop` on an already-closed handle succeeds -- the same answer its
        // own `already_closed` path gives below.
        //
        // bug-622: a closed handle is also where the LAST release frees. Only a drop
        // whose release took the owner count to 0 (`DROP_MODE_FREE`) may free: another
        // binding (an alias, a moved-from caller, a trap handler) may still read it.
        // Two closed shapes reach here. A started thread `thread::waitFor` or a closing
        // drop joined (zero OS handle) still owns its queues: free all its plumbing. The
        // zeroed `TRAP`-path handle (bug-479) has no queues and never had a worker: free
        // just its block. A started thread still unjoined (a running worker that was
        // detached) keeps a non-zero OS handle and is never freed from here.
        ThreadSimpleOp::Drop => {
            let keep = format!("{symbol}_precheck_keep");
            let block_only = format!("{symbol}_precheck_block_only");
            instructions.extend([
                abi::load_u64("%v9", abi::stack_pointer(), MODE_OFFSET),
                abi::compare_immediate("%v9", DROP_MODE_FREE),
                abi::branch_ne(&keep),
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("%v9", "%v8", THREAD_OFFSET_OS_HANDLE),
                abi::compare_immediate("%v9", "0"),
                abi::branch_ne(&keep),
                abi::load_u64("%v9", "%v8", THREAD_OFFSET_OUTBOUND_QUEUE),
                abi::compare_immediate("%v9", "0"),
                abi::branch_eq(&block_only),
            ]);
            emit_release_thread_plumbing(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                HANDLE_OFFSET,
                RELEASE_QUEUE_OFFSET,
                RING_INDEX_OFFSET,
                RING_LEFT_OFFSET,
                RING_ENTRY_OFFSET,
                RING_STATE_OFFSET,
                release,
            )?;
            instructions.extend([
                abi::branch(&keep),
                abi::label(&block_only),
                abi::load_u64(abi::c_arg(0), abi::stack_pointer(), HANDLE_OFFSET),
                abi::move_immediate(abi::c_arg(1), "Integer", &THREAD_BLOCK_SIZE.to_string()),
                abi::branch_link(ARENA_FREE_SYMBOL),
            ]);
            relocations.push(internal_branch(symbol, ARENA_FREE_SYMBOL));
            instructions.extend([
                abi::label(&keep),
                abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
            ]);
        }
        _ => {
            raise_error_into(
                symbol,
                "ErrResourceClosed",
                &mut instructions,
                &mut relocations,
            );
            // `raise_error_into` sets VALUE/TAG/MESSAGE and nothing else, so the
            // ORIGIN register has to be zeroed here. `WaitFor` is the only op that
            // returns a meaningful one — it propagates the worker's `ErrorLoc` —
            // and its own closed path zeroes the slot with the comment "waitFor's
            // own error (resource closed): no worker origin". Skipping this
            // returned a garbage pointer that the `TRAP` handler dereferenced:
            // caught by the positive pin as a SIGSEGV on a SECOND `waitFor` of a
            // closed handle, which the pre-fix compiler answered `77030004`.
            instructions.push(abi::move_immediate(
                RESULT_ERROR_SOURCE_REGISTER,
                "Integer",
                "0",
            ));
        }
    }
    instructions.push(abi::label(&precheck_done));
    instructions.push(abi::return_());
    Ok((instructions, relocations, FRAME_SIZE))
}

/// The caller-frame slots [`emit_cancellable_sleep_wait`] addresses: the thread
/// control block it waits on, the requested `ms`, the scratch word it parks the
/// inbound-queue pointer in, and the 16-byte absolute `timespec` deadline.
pub(crate) struct CancellableSleepSlots {
    pub(crate) handle: usize,
    pub(crate) timeout: usize,
    pub(crate) queue: usize,
    pub(crate) timespec: usize,
}

/// plan-99: the cancellation-aware wait shared by the worker `thread::sleep` and
/// the worker branch of `os::sleep`. Waits on the worker's inbound not-empty
/// condvar — the one `thread::cancel` broadcasts — until the ABSOLUTE deadline
/// `now + ms`, so a parent `send` arriving mid-sleep re-loops without shortening
/// it, while a pending cancellation branches to `interrupted_label` promptly.
///
/// The caller has already validated `ms` (`< 0` rejected, `== 0` short-circuited),
/// parked it at `slots.timeout`, and parked the thread control block at
/// `slots.handle`. This block FALLS THROUGH once the deadline is reached (with the
/// queue mutex released), so the caller must place its `ok` epilogue immediately
/// after it; the `interrupted_label` block is emitted separately by
/// [`emit_cancellable_sleep_interrupted`], because in the stream it belongs *after*
/// that epilogue.
pub(crate) fn emit_cancellable_sleep_wait(
    ctx: &mut EmitCtx,
    slots: CancellableSleepSlots,
    interrupted_label: &str,
) -> Result<(), String> {
    let wait_loop = format!("{}_wait_loop", ctx.symbol);
    let deadline_reached = format!("{}_deadline_reached", ctx.symbol);
    // Absolute deadline = now + ms (pthread_cond_timedwait consumes it).
    emit_thread_deadline(ctx, slots.timeout, slots.timespec)?;
    // Lock the inbound queue mutex (the queue base pointer IS the mutex — offset 0).
    ctx.instructions.extend([
        abi::load_u64("%v8", abi::stack_pointer(), slots.handle),
        abi::load_u64("%v9", "%v8", THREAD_OFFSET_INBOUND_QUEUE),
        abi::store_u64("%v9", abi::stack_pointer(), slots.queue),
        abi::move_register(abi::c_arg(0), "%v9"),
    ]);
    emit_thread_external_call(ctx, "pthread_mutex_lock")?;
    ctx.instructions.extend([
        abi::label(&wait_loop),
        // Cancellation requested? wake and fail with ErrInterrupted (poll parity
        // with the worker receive path).
        abi::load_u64("%v8", abi::stack_pointer(), slots.handle),
        abi::load_u64("%v10", "%v8", THREAD_OFFSET_CANCELLED),
        abi::compare_immediate("%v10", "0"),
        abi::branch_ne(interrupted_label),
        // Wait on the inbound not-empty condvar until the absolute deadline.
        abi::load_u64("%v9", abi::stack_pointer(), slots.queue),
        abi::add_immediate(abi::c_arg(0), "%v9", THREAD_QUEUE_NOT_EMPTY_OFFSET),
        abi::move_register(abi::c_arg(1), "%v9"), // mutex = queue base
        abi::add_immediate(abi::c_arg(2), abi::stack_pointer(), slots.timespec),
    ]);
    emit_thread_external_call(ctx, "pthread_cond_timedwait")?;
    ctx.instructions.extend([
        // Non-zero (ETIMEDOUT) = the absolute deadline elapsed → the sleep is done.
        // Zero = a spurious/broadcast wake (a parent `send`, or `cancel`); re-loop
        // to re-check the cancel flag. The absolute deadline is unchanged, so a
        // send never shortens the sleep.
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_ne(&deadline_reached),
        abi::branch(&wait_loop),
        abi::label(&deadline_reached),
        abi::load_u64("%v9", abi::stack_pointer(), slots.queue),
        abi::move_register(abi::c_arg(0), "%v9"),
    ]);
    emit_thread_external_call(ctx, "pthread_mutex_unlock")?;
    Ok(())
}

/// plan-99: the `interrupted_label` tail of [`emit_cancellable_sleep_wait`] —
/// release the queue mutex and set the `ErrInterrupted` result. The caller appends
/// the `return_` (it owns the epilogue ordering).
pub(crate) fn emit_cancellable_sleep_interrupted(
    ctx: &mut EmitCtx,
    queue_offset: usize,
    interrupted_label: &str,
) -> Result<(), String> {
    ctx.instructions.extend([
        abi::label(interrupted_label),
        abi::load_u64("%v9", abi::stack_pointer(), queue_offset),
        abi::move_register(abi::c_arg(0), "%v9"),
    ]);
    emit_thread_external_call(ctx, "pthread_mutex_unlock")?;
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
///     path at all — byte-for-byte what the parent `thread::sleep` did;
///   - inside a worker it is the cancellation-aware condvar wait
///     ([`emit_cancellable_sleep_wait`]), so `thread::cancel` wakes it early with
///     `ErrInterrupted` and a parent `send` does not shorten it.
///
/// The two are told apart by the TCB back-pointer the worker trampoline publishes
/// at [`ARENA_WORKER_THREAD_OFFSET`]: `0` (the main thread's zero-init default)
/// takes the plain delay, non-zero takes the wait — and that non-zero value IS the
/// control block the wait needs, so no handle argument is required.
///
/// `ms < 0` → `ErrInvalidArgument`; `ms == 0` returns immediately without reading
/// the arena at all. There is no `ErrResourceClosed`: `os::sleep` owns no handle.
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
    // Main-thread-branch slots: the relative `req`/`rem` timespec pair. Disjoint
    // from the worker slots above — only one branch ever runs, but keeping them
    // separate keeps each block readable on its own.
    const REQ_OFFSET: usize = 48;
    const REM_OFFSET: usize = 64;

    let ok = format!("{symbol}_ok");
    let err_arg = format!("{symbol}_invalid");
    let worker = format!("{symbol}_worker");
    let interrupted = format!("{symbol}_interrupted");

    let mut instructions = Vec::new();
    let mut relocations = Vec::new();
    instructions.extend([
        // ms validation first: `< 0` rejects and `== 0` is an immediate no-op Ok,
        // so a zero sleep never touches the arena or a queue lock.
        abi::compare_immediate(abi::c_arg(0), "0"),
        abi::branch_lt(&err_arg),
        abi::branch_eq(&ok),
        // Park `ms` for the worker branch (`emit_cancellable_sleep_wait` reads it
        // from the frame); the main-thread branch reads it straight out of c_arg(0),
        // which nothing below clobbers before `emit_relative_sleep` copies it.
        abi::store_u64(abi::c_arg(0), abi::stack_pointer(), TIMEOUT_OFFSET),
        // Am I a worker? The trampoline published this thread's control block at
        // arena+8; the main thread's is the zero-init 0. Park it for the worker
        // branch rather than holding it in a vreg across the main-thread branch's
        // `nanosleep` call (a hand-picked `%vN` here is caller-saved).
        abi::load_u64("%v8", ARENA_STATE_REGISTER, ARENA_WORKER_THREAD_OFFSET),
        abi::store_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
        abi::compare_immediate("%v8", "0"),
        abi::branch_ne(&worker),
    ]);
    // Main thread: a plain relative delay; leaves this block only via `ok`.
    emit_relative_sleep(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        abi::c_arg(0),
        REQ_OFFSET,
        REM_OFFSET,
        &ok,
    )?;
    // Worker: the cancellation-aware wait, on the control block parked above.
    instructions.push(abi::label(&worker));
    emit_cancellable_sleep_wait(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        CancellableSleepSlots {
            handle: HANDLE_OFFSET,
            timeout: TIMEOUT_OFFSET,
            queue: QUEUE_OFFSET,
            timespec: TIMESPEC_OFFSET,
        },
        &interrupted,
    )?;
    instructions.extend([
        // Nothing return: only the OK tag (no result value). Reached by falling out
        // of the worker wait, and by branch from the main-thread delay and `ms == 0`.
        abi::label(&ok),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::return_(),
    ]);
    emit_cancellable_sleep_interrupted(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        QUEUE_OFFSET,
        &interrupted,
    )?;
    instructions.extend([
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
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::return_());
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
    const QUEUE_OFFSET: usize = 32;
    const TIMESPEC_OFFSET: usize = 40;
    // Byte size of the message copy (arg 3), so a failed send can record it on the
    // pending-free list for the destination to reclaim (bug-147.5b). Must sit past
    // the 16-byte timespec at [40, 56): `emit_thread_deadline` writes tv_nsec at
    // TIMESPEC_OFFSET+8 (=48) and `clock_gettime` writes all 16 bytes, so a size
    // field at 48 would be clobbered by the deadline before the failed-send path
    // reloads it (bug-163).
    const DATA_SIZE_OFFSET: usize = 56;
    // bug-650 case 2: byte size of the message's STATE block (arg 4), 0 when it has
    // none. Travels with the copy into the ring entry's third word so a reclaimer can
    // free the STATE block as well as the record.
    const DATA_STATE_SIZE_OFFSET: usize = 64;
    // Only the resource planes carry a record with a STATE block; a data-plane block can
    // be smaller than a pending-free node's third word, so the STATE word is written and
    // read on these two queues alone.
    let resource_plane = matches!(
        queue_offset,
        THREAD_OFFSET_RESOURCE_INBOUND_QUEUE | THREAD_OFFSET_RESOURCE_OUTBOUND_QUEUE
    );

    let invalid = format!("{symbol}_invalid");
    let timeout_ok = format!("{symbol}_timeout_ok");
    let closed = format!("{symbol}_closed");
    let interrupted = format!("{symbol}_interrupted");
    let timeout = format!("{symbol}_timeout");
    let wait_loop = format!("{symbol}_wait_loop");
    let wait_timed = format!("{symbol}_wait_timed");
    let wait_indefinite = format!("{symbol}_wait_indefinite");
    let enqueue = format!("{symbol}_enqueue");
    let tail_wrap = format!("{symbol}_tail_wrap");
    let unlock = format!("{symbol}_unlock");
    let done = format!("{symbol}_done");
    let mut instructions = Vec::new();
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::c_arg(0), abi::stack_pointer(), HANDLE_OFFSET),
        abi::store_u64(abi::c_arg(1), abi::stack_pointer(), DATA_OFFSET),
        abi::store_u64(abi::c_arg(2), abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::store_u64(abi::c_arg(3), abi::stack_pointer(), DATA_SIZE_OFFSET),
        abi::store_u64(abi::c_arg(4), abi::stack_pointer(), DATA_STATE_SIZE_OFFSET),
        // plan-73-A: a non-negative `timeoutMs` is a real timeout (0 = one immediate
        // attempt, N = wait N ms). The unbounded sentinel (i64::MIN) is the omit=block
        // form and is accepted; any OTHER negative value is rejected with
        // `ErrInvalidArgument`. Mirrors the read helper's prologue.
        abi::compare_immediate(abi::c_arg(2), "0"),
        abi::branch_ge(&timeout_ok),
        abi::move_immediate("%v9", "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
        abi::compare_registers(abi::c_arg(2), "%v9"),
        abi::branch_ne(&invalid),
        abi::label(&timeout_ok),
    ]);
    if !parent_send {
        // Re-establish the current-thread register `x20` from the worker's own
        // control block (`x0`) rather than asserting equality; see the matching
        // note in `thread_queue_read_helper`.
        instructions.push(abi::move_register(abi::CURRENT_THREAD, abi::c_arg(0)));
    }
    emit_thread_deadline(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        TIMEOUT_OFFSET,
        TIMESPEC_OFFSET,
    )?;
    instructions.extend([
        abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
        abi::load_u64("%v9", "%v8", queue_offset),
        abi::store_u64("%v9", abi::stack_pointer(), QUEUE_OFFSET),
        abi::move_register(abi::c_arg(0), "%v9"),
    ]);
    emit_thread_external_call(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_mutex_lock",
    )?;
    // bug-646: drain the queue's pending-free list — every block on it was carved by
    // THIS side (the sender allocates the boundary copy in its own arena, bug-498), so
    // this is the thread whose bins the memory must go back to. Two paths put blocks
    // there: a failed send's orphaned copy (bug-147.5b) and a successful hand-over the
    // reader has finished with (bug-646). We hold the queue mutex, and each node carries
    // `{next, size}` in its own first two words. The queue pointer is reloaded from its
    // frame slot every iteration because `arena_free` clobbers caller-saved registers.
    let write_drain_loop = format!("{symbol}_pending_free_drain");
    let write_drain_done = format!("{symbol}_pending_free_done");
    instructions.extend([
        abi::label(&write_drain_loop),
        abi::load_u64("%v9", abi::stack_pointer(), QUEUE_OFFSET),
        abi::load_u64("%v10", "%v9", THREAD_QUEUE_PENDING_FREE_OFFSET),
        abi::compare_immediate("%v10", "0"),
        abi::branch_eq(&write_drain_done),
        abi::load_u64("%v11", "%v10", 0),
        abi::store_u64("%v11", "%v9", THREAD_QUEUE_PENDING_FREE_OFFSET),
        abi::load_u64(abi::c_arg(1), "%v10", 8),
        abi::move_register(abi::c_arg(0), "%v10"),
        abi::branch_link(ARENA_FREE_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_FREE_SYMBOL));
    instructions.extend([
        abi::branch(&write_drain_loop),
        abi::label(&write_drain_done),
    ]);
    instructions.push(abi::label(&wait_loop));
    if parent_send {
        instructions.extend([
            abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
            abi::load_u64("%v9", "%v8", THREAD_OFFSET_STATE),
            abi::compare_immediate("%v9", THREAD_STATE_CLOSED),
            abi::branch_eq(&closed),
            abi::compare_immediate("%v9", THREAD_STATE_COMPLETED),
            abi::branch_eq(&interrupted),
            abi::load_u64("%v9", "%v8", THREAD_OFFSET_CANCELLED),
            abi::compare_immediate("%v9", "0"),
            abi::branch_ne(&interrupted),
        ]);
    } else {
        instructions.extend([
            abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
            abi::load_u64("%v9", "%v8", THREAD_OFFSET_CANCELLED),
            abi::compare_immediate("%v9", "0"),
            abi::branch_ne(&interrupted),
        ]);
    }
    instructions.extend([
        abi::load_u64("%v9", abi::stack_pointer(), QUEUE_OFFSET),
        abi::load_u64("%v10", "%v9", THREAD_QUEUE_CLOSED_OFFSET),
        abi::compare_immediate("%v10", "0"),
        abi::branch_ne(&interrupted),
        abi::load_u64("%v10", "%v9", THREAD_QUEUE_COUNT_OFFSET),
        abi::load_u64("%v11", "%v9", THREAD_QUEUE_CAPACITY_OFFSET),
        abi::compare_registers("%v10", "%v11"),
        abi::branch_lt(&enqueue),
        abi::load_u64("%v12", abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::compare_immediate("%v12", "0"),
        // plan-73-A: 0 = one immediate attempt (queue full ⇒ `ErrTimeout`); the
        // unbounded sentinel (< 0) blocks until space; a positive value bounds the
        // wait against the absolute deadline.
        abi::branch_eq(&timeout),
        abi::branch_lt(&wait_indefinite),
        abi::label(&wait_timed),
        abi::add_immediate(abi::c_arg(0), "%v9", THREAD_QUEUE_NOT_FULL_OFFSET),
        abi::move_register(abi::c_arg(1), "%v9"),
        abi::add_immediate(abi::c_arg(2), abi::stack_pointer(), TIMESPEC_OFFSET),
    ]);
    emit_thread_external_call(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_cond_timedwait",
    )?;
    instructions.extend([
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_ne(&timeout),
        abi::branch(&wait_loop),
        // Unbounded (omit) form: block on the not-full condition until a slot frees
        // or the queue/thread closes (re-checked at the top of `wait_loop`).
        abi::label(&wait_indefinite),
        abi::add_immediate(abi::c_arg(0), "%v9", THREAD_QUEUE_NOT_FULL_OFFSET),
        abi::move_register(abi::c_arg(1), "%v9"),
    ]);
    emit_thread_external_call(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_cond_wait",
    )?;
    instructions.extend([
        abi::branch(&wait_loop),
        abi::label(&enqueue),
        abi::load_u64("%v9", abi::stack_pointer(), QUEUE_OFFSET),
        abi::load_u64("%v10", "%v9", THREAD_QUEUE_TAIL_OFFSET),
        abi::load_u64("%v11", "%v9", THREAD_QUEUE_VALUES_OFFSET),
        abi::shift_left_immediate("%v12", "%v10", THREAD_QUEUE_ENTRY_SHIFT),
        abi::add_registers("%v11", "%v11", "%v12"),
        abi::load_u64("%v12", abi::stack_pointer(), DATA_OFFSET),
        abi::store_u64("%v12", "%v11", 0),
        // bug-646: the entry's second word is the copy's byte size (arg 3; 0 = "not
        // reclaimable"), so the reader can hand the block back for the sender to free
        // without knowing the message type.
        abi::load_u64("%v12", abi::stack_pointer(), DATA_SIZE_OFFSET),
        abi::store_u64("%v12", "%v11", 8),
        // bug-650 case 2: the entry's third word is the STATE block's byte size (arg 4;
        // 0 = none), so a reclaimer can free it alongside the record.
        abi::load_u64("%v12", abi::stack_pointer(), DATA_STATE_SIZE_OFFSET),
        abi::store_u64("%v12", "%v11", THREAD_QUEUE_ENTRY_STATE_SIZE),
        abi::add_immediate("%v10", "%v10", 1),
        abi::load_u64("%v11", "%v9", THREAD_QUEUE_CAPACITY_OFFSET),
        abi::compare_registers("%v10", "%v11"),
        abi::branch_lt(&tail_wrap),
        abi::move_immediate("%v10", "Integer", "0"),
        abi::label(&tail_wrap),
        abi::store_u64("%v10", "%v9", THREAD_QUEUE_TAIL_OFFSET),
        abi::load_u64("%v10", "%v9", THREAD_QUEUE_COUNT_OFFSET),
        abi::add_immediate("%v10", "%v10", 1),
        abi::store_u64("%v10", "%v9", THREAD_QUEUE_COUNT_OFFSET),
        abi::add_immediate(abi::c_arg(0), "%v9", THREAD_QUEUE_NOT_EMPTY_OFFSET),
    ]);
    emit_thread_external_call(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_cond_signal",
    )?;
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&unlock),
        abi::label(&interrupted),
    ]);
    raise_error_into(
        symbol,
        "ErrInterrupted",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&unlock), abi::label(&closed)]);
    raise_error_into(
        symbol,
        "ErrResourceClosed",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&unlock), abi::label(&timeout)]);
    raise_error_into(symbol, "ErrTimeout", &mut instructions, &mut relocations);
    instructions.extend([abi::branch(&unlock), abi::label(&invalid)]);
    raise_error_into(
        symbol,
        "ErrInvalidArgument",
        &mut instructions,
        &mut relocations,
    );
    let skip_orphan_push = format!("{symbol}_skip_orphan_push");
    instructions.extend([
        abi::branch(&done),
        abi::label(&unlock),
        // bug-147.5b: a failed send (tag != Ok) leaves the message copy orphaned (in
        // the SENDER's arena since bug-498). Still holding the queue mutex, push it onto the
        // queue's pending-free list — reusing the dead block's own first two words as
        // `{next, size}` — so the destination reclaims it (in its own arena) on its
        // next read. `DATA_OFFSET` still holds the copy pointer here (it is reused as
        // a result-register spill slot only below). The result registers are live, so
        // scratch stays in %v8-%v11.
        abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG),
        abi::branch_eq(&skip_orphan_push),
        // A size of 0 means the caller did not hand us a reclaimable block (a scalar
        // message with no copy, or a type whose exact copy size we do not compute) —
        // skip the push and let it leak (bounded, reclaimed at worker teardown)
        // rather than risk a wrong-size `arena_free`.
        abi::load_u64("%v10", abi::stack_pointer(), DATA_SIZE_OFFSET),
        abi::compare_immediate("%v10", "0"),
        abi::branch_eq(&skip_orphan_push),
        abi::load_u64("%v8", abi::stack_pointer(), QUEUE_OFFSET),
        abi::load_u64("%v9", abi::stack_pointer(), DATA_OFFSET),
        abi::load_u64("%v11", "%v8", THREAD_QUEUE_PENDING_FREE_OFFSET),
        abi::store_u64("%v11", "%v9", 0),
        abi::store_u64("%v10", "%v9", 8),
        abi::store_u64("%v9", "%v8", THREAD_QUEUE_PENDING_FREE_OFFSET),
    ]);
    if resource_plane {
        // bug-650 case 2: the orphan's STATE block goes with it. Third word of the node,
        // in range because a resource-plane block is always one RESOURCE_RECORD_SIZE
        // record; its STATE POINTER is still readable at RESOURCE_OFFSET_STATE, which
        // the node's first two words do not reach.
        instructions.extend([
            abi::load_u64("%v10", abi::stack_pointer(), DATA_STATE_SIZE_OFFSET),
            abi::store_u64("%v10", "%v9", PENDING_FREE_STATE_SIZE),
        ]);
    }
    instructions.extend([
        abi::label(&skip_orphan_push),
        abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), DATA_OFFSET),
        abi::store_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::store_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            TIMESPEC_OFFSET,
        ),
        abi::load_u64(abi::c_arg(0), abi::stack_pointer(), QUEUE_OFFSET),
    ]);
    emit_thread_external_call(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_mutex_unlock",
    )?;
    instructions.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), DATA_OFFSET),
        abi::load_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::load_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            TIMESPEC_OFFSET,
        ),
        abi::label(&done),
    ]);
    instructions.push(abi::return_());
    Ok((instructions, relocations, FRAME_SIZE))
}

/// How a queue-read helper treats its caller. The read machinery is shared by the
/// data plane (`receive`/`read`) and the resource plane
/// (`acceptResource`/`readResource`); the only difference is whether the caller's
/// `x0` is its own control block, so the helper may re-establish the current-thread
/// register `x20` and consult the worker's cancellation flag (`WorkerSelf`); a
/// parent caller must do neither because `x0` is the *worker's* block and clobbering
/// `x20` would corrupt the parent thread. A parent caller instead checks the
/// worker's run state for termination.
///
/// bug-181: both modes are waitable. The no-arg `receive`/`accept` overload passes
/// the block sentinel (`TIMEOUT_UNBOUNDED_SENTINEL`, i64::MIN) and waits
/// indefinitely; any other negative `timeoutMs` is rejected with
/// `ErrInvalidArgument`. A parent's indefinite wait is terminated when the worker
/// completes or closes the queue (the trampoline broadcasts the queue's condvar on
/// exit), so it never deadlocks.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ThreadReadMode {
    /// Worker reading its own queue (`receive`, `acceptResource`): re-establish
    /// `x20` and check the worker cancel flag.
    WorkerSelf,
    /// Parent reading a worker queue (`read`, `readResource`): no `x20` touch,
    /// check the worker's run state for termination.
    Parent,
}

pub(crate) fn thread_queue_read_helper(
    symbol: &str,
    queue_offset: usize,
    mode: ThreadReadMode,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<ThreadBodyParts, String> {
    // `WorkerSelf` callers pass their own control block, so the helper restores
    // `x20` and reads the worker cancel flag; parent callers do neither.
    let worker_self = mode == ThreadReadMode::WorkerSelf;
    // bug-650 case 2: only the resource planes carry a record with a STATE block, and
    // only there is a pending-free node's third word guaranteed in range.
    let resource_plane = matches!(
        queue_offset,
        THREAD_OFFSET_RESOURCE_INBOUND_QUEUE | THREAD_OFFSET_RESOURCE_OUTBOUND_QUEUE
    );
    const FRAME_SIZE: usize = 80;
    const HANDLE_OFFSET: usize = 8;
    const TIMEOUT_OFFSET: usize = 16;
    const QUEUE_OFFSET: usize = 24;
    const VALUE_OFFSET: usize = 32;
    const TAG_OFFSET: usize = 40;
    const ERROR_OFFSET: usize = 48;
    const TIMESPEC_OFFSET: usize = 56;

    let invalid = format!("{symbol}_invalid");
    let found = format!("{symbol}_found");
    let wait_loop = format!("{symbol}_wait_loop");
    let wait_timed = format!("{symbol}_wait_timed");
    let wait_indefinite = format!("{symbol}_wait_indefinite");
    let timeout_ok = format!("{symbol}_timeout_ok");
    let not_found = format!("{symbol}_not_found");
    let interrupted = format!("{symbol}_interrupted");
    let closed = format!("{symbol}_closed");
    let timeout = format!("{symbol}_timeout");
    let head_wrap = format!("{symbol}_head_wrap");
    let unlock = format!("{symbol}_unlock");
    let done = format!("{symbol}_done");
    let mut instructions = Vec::new();
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::c_arg(0), abi::stack_pointer(), HANDLE_OFFSET),
        abi::store_u64(abi::c_arg(1), abi::stack_pointer(), TIMEOUT_OFFSET),
    ]);
    if worker_self {
        // The caller's `x0` is this worker's own control block (the handle is
        // unforgeable in type-correct code). Re-establish the current-thread
        // register `x20` from it rather than asserting equality: arbitrary
        // generated code between worker ops (e.g. arena allocation) may clobber
        // `x20`, so we restore the invariant here instead of failing on it.
        instructions.push(abi::move_register(abi::CURRENT_THREAD, abi::c_arg(0)));
    }
    // bug-181: the no-arg `receive`/`accept` overload passes the block sentinel
    // (i64::MIN) to wait indefinitely; a non-negative `timeoutMs` is a real timeout
    // (0 = poll, N = wait N ms). Any other negative value is an explicit user
    // timeout below zero and is rejected with `ErrInvalidArgument`.
    instructions.extend([
        abi::compare_immediate(abi::c_arg(1), "0"),
        abi::branch_ge(&timeout_ok),
        abi::move_immediate("%v9", "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
        abi::compare_registers(abi::c_arg(1), "%v9"),
        abi::branch_ne(&invalid),
        abi::label(&timeout_ok),
    ]);
    emit_thread_deadline(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        TIMEOUT_OFFSET,
        TIMESPEC_OFFSET,
    )?;
    instructions.extend([
        abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
        abi::load_u64("%v9", "%v8", queue_offset),
        abi::store_u64("%v9", abi::stack_pointer(), QUEUE_OFFSET),
        abi::move_register(abi::c_arg(0), "%v9"),
    ]);
    emit_thread_external_call(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_mutex_lock",
    )?;
    // bug-646: retire the block the PREVIOUS read handed out. `thread::receive` /
    // `thread::accept` deep-copy the queued block into the reader's OWN arena at the
    // call site, after which the queued block — carved by the sender (bug-498) — has no
    // owner at all; that was the leak. It cannot be parked the instant it is handed out
    // (the call-site copy has not run yet, and the sender's drain would free it out from
    // under the copy), so each read parks the one before it: the copy of block N-1 is
    // finished by the time this thread asks for block N, and each queue has exactly one
    // reader (the worker for an inbound queue, the parent for an outbound one), so
    // "the previous read" is a strictly earlier point on THIS thread.
    //
    // The park is a push onto the queue's pending-free list under the queue mutex, using
    // the dead block's own first two words as `{next, size}` — the SENDER drains it (at
    // its next write, or at `emit_release_thread_plumbing`), because the sender's arena
    // is where the memory came from and a worker's bins die with the worker. A size of 0
    // means "not reclaimable" (a scalar message, or a type whose exact copy size the send
    // helper does not compute): drop the reference without freeing.
    //
    // This replaces bug-147.5b's reader-side drain of the same list, which freed the
    // sender's blocks into the READER's bins — memory-safe adoption, but it only moved
    // the leak when the reader was a short-lived worker.
    let retire_clear = format!("{symbol}_pending_free_clear");
    let retire_done = format!("{symbol}_pending_free_done");
    instructions.extend([
        abi::load_u64("%v9", abi::stack_pointer(), QUEUE_OFFSET),
        abi::load_u64("%v10", "%v9", THREAD_QUEUE_LAST_READ_PTR_OFFSET),
        abi::compare_immediate("%v10", "0"),
        abi::branch_eq(&retire_done),
        abi::load_u64("%v11", "%v9", THREAD_QUEUE_LAST_READ_SIZE_OFFSET),
        abi::compare_immediate("%v11", "0"),
        abi::branch_eq(&retire_clear),
        abi::load_u64("%v12", "%v9", THREAD_QUEUE_PENDING_FREE_OFFSET),
        abi::store_u64("%v12", "%v10", 0),
        abi::store_u64("%v11", "%v10", 8),
        abi::store_u64("%v10", "%v9", THREAD_QUEUE_PENDING_FREE_OFFSET),
    ]);
    if resource_plane {
        // bug-650 case 2: carry the parked block's STATE size into the node's third
        // word, so the sender's drain frees the STATE block as well as the record.
        instructions.extend([
            abi::load_u64("%v11", "%v9", THREAD_QUEUE_LAST_READ_STATE_SIZE_OFFSET),
            abi::store_u64("%v11", "%v10", PENDING_FREE_STATE_SIZE),
        ]);
    }
    instructions.extend([
        abi::label(&retire_clear),
        abi::store_u64(abi::ZERO, "%v9", THREAD_QUEUE_LAST_READ_PTR_OFFSET),
        abi::store_u64(abi::ZERO, "%v9", THREAD_QUEUE_LAST_READ_SIZE_OFFSET),
        abi::store_u64(abi::ZERO, "%v9", THREAD_QUEUE_LAST_READ_STATE_SIZE_OFFSET),
        abi::label(&retire_done),
    ]);
    instructions.extend([
        abi::label(&wait_loop),
        abi::load_u64("%v9", abi::stack_pointer(), QUEUE_OFFSET),
        abi::load_u64("%v10", "%v9", THREAD_QUEUE_COUNT_OFFSET),
        abi::compare_immediate("%v10", "0"),
        abi::branch_gt(&found),
    ]);
    if worker_self {
        instructions.extend([
            abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
            abi::load_u64("%v10", "%v8", THREAD_OFFSET_CANCELLED),
            abi::compare_immediate("%v10", "0"),
            abi::branch_ne(&interrupted),
        ]);
    }
    instructions.extend([
        abi::load_u64("%v10", "%v9", THREAD_QUEUE_CLOSED_OFFSET),
        abi::compare_immediate("%v10", "0"),
        abi::branch_ne(&not_found),
    ]);
    if !worker_self {
        instructions.extend([
            abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
            abi::load_u64("%v10", "%v8", THREAD_OFFSET_STATE),
            abi::compare_immediate("%v10", THREAD_STATE_CLOSED),
            abi::branch_eq(&closed),
            abi::compare_immediate("%v10", THREAD_STATE_COMPLETED),
            abi::branch_eq(&not_found),
        ]);
    }
    instructions.extend([
        abi::load_u64("%v10", abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::compare_immediate("%v10", "0"),
        // plan-73-A: an explicit `timeoutMs` of 0 on a still-open but empty queue is
        // one immediate attempt that raises `ErrTimeout` (the convention's `0` =
        // one-attempt rule), NOT `ErrNotFound`. `ErrNotFound` is reserved above for a
        // genuinely terminal empty queue (closed flag / completed worker).
        abi::branch_eq(&timeout),
        abi::branch_lt(&wait_indefinite),
        abi::label(&wait_timed),
        abi::add_immediate(abi::c_arg(0), "%v9", THREAD_QUEUE_NOT_EMPTY_OFFSET),
        abi::move_register(abi::c_arg(1), "%v9"),
        abi::add_immediate(abi::c_arg(2), abi::stack_pointer(), TIMESPEC_OFFSET),
    ]);
    emit_thread_external_call(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_cond_timedwait",
    )?;
    instructions.extend([
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_ne(&timeout),
        abi::branch(&wait_loop),
        abi::label(&wait_indefinite),
        abi::add_immediate(abi::c_arg(0), "%v9", THREAD_QUEUE_NOT_EMPTY_OFFSET),
        abi::move_register(abi::c_arg(1), "%v9"),
    ]);
    emit_thread_external_call(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_cond_wait",
    )?;
    instructions.extend([
        abi::branch(&wait_loop),
        abi::label(&found),
        abi::load_u64("%v9", abi::stack_pointer(), QUEUE_OFFSET),
        abi::load_u64("%v10", "%v9", THREAD_QUEUE_HEAD_OFFSET),
        abi::load_u64("%v11", "%v9", THREAD_QUEUE_VALUES_OFFSET),
        abi::shift_left_immediate("%v12", "%v10", THREAD_QUEUE_ENTRY_SHIFT),
        abi::add_registers("%v11", "%v11", "%v12"),
        abi::load_u64(RESULT_VALUE_REGISTER, "%v11", 0),
        // bug-646: remember the block (and the size the sender recorded with it) as the
        // one to retire on this queue's next read — the reader's only chance to give the
        // sender its memory back, since the call site copies the block and then drops it.
        abi::load_u64("%v12", "%v11", 8),
        abi::store_u64(
            RESULT_VALUE_REGISTER,
            "%v9",
            THREAD_QUEUE_LAST_READ_PTR_OFFSET,
        ),
        abi::store_u64("%v12", "%v9", THREAD_QUEUE_LAST_READ_SIZE_OFFSET),
        // bug-650 case 2: and the STATE size the sender recorded in the entry's third
        // word, so the retire above can free that block too.
        abi::load_u64("%v12", "%v11", THREAD_QUEUE_ENTRY_STATE_SIZE),
        abi::store_u64("%v12", "%v9", THREAD_QUEUE_LAST_READ_STATE_SIZE_OFFSET),
        abi::add_immediate("%v10", "%v10", 1),
        abi::load_u64("%v11", "%v9", THREAD_QUEUE_CAPACITY_OFFSET),
        abi::compare_registers("%v10", "%v11"),
        abi::branch_lt(&head_wrap),
        abi::move_immediate("%v10", "Integer", "0"),
        abi::label(&head_wrap),
        abi::store_u64("%v10", "%v9", THREAD_QUEUE_HEAD_OFFSET),
        abi::load_u64("%v10", "%v9", THREAD_QUEUE_COUNT_OFFSET),
        abi::subtract_immediate("%v10", "%v10", 1),
        abi::store_u64("%v10", "%v9", THREAD_QUEUE_COUNT_OFFSET),
        abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
        abi::add_immediate(abi::c_arg(0), "%v9", THREAD_QUEUE_NOT_FULL_OFFSET),
    ]);
    emit_thread_external_call(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_cond_signal",
    )?;
    instructions.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&unlock),
        abi::label(&not_found),
    ]);
    raise_error_into(symbol, "ErrNotFound", &mut instructions, &mut relocations);
    instructions.extend([abi::branch(&unlock), abi::label(&interrupted)]);
    raise_error_into(
        symbol,
        "ErrInterrupted",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&unlock), abi::label(&closed)]);
    raise_error_into(
        symbol,
        "ErrResourceClosed",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&unlock), abi::label(&timeout)]);
    raise_error_into(symbol, "ErrTimeout", &mut instructions, &mut relocations);
    instructions.extend([abi::branch(&unlock), abi::label(&invalid)]);
    raise_error_into(
        symbol,
        "ErrInvalidArgument",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&unlock),
        abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
        abi::store_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TAG_OFFSET),
        abi::store_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            ERROR_OFFSET,
        ),
        abi::load_u64(abi::c_arg(0), abi::stack_pointer(), QUEUE_OFFSET),
    ]);
    emit_thread_external_call(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_mutex_unlock",
    )?;
    instructions.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
        abi::load_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TAG_OFFSET),
        abi::load_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            ERROR_OFFSET,
        ),
        abi::label(&done),
    ]);
    instructions.push(abi::return_());
    Ok((instructions, relocations, FRAME_SIZE))
}

pub(crate) fn thread_is_cancelled_helper() -> ThreadBodyParts {
    // Reads the worker's pinned current-thread register `x20` (the thread control
    // block); reserve it so the allocator never colors the `%v9` scratch onto it.
    let cancelled = "_mfb_rt_thread_is_cancelled_true";
    let done = "_mfb_rt_thread_is_cancelled_done";
    let instructions = vec![
        abi::load_u64("%v9", abi::CURRENT_THREAD, THREAD_OFFSET_CANCELLED),
        abi::compare_immediate("%v9", "0"),
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

/// Close and broadcast both **resource-plane** queues of the thread whose handle
/// lives at `[sp + handle_offset]`, so anyone parked on them re-checks
/// CANCELLED/CLOSED and unblocks.
///
/// `thread::cancel`/`thread::drop` closed and broadcast only the two data-plane
/// queues, so a worker parked in a blocking `acceptResource` waited on the
/// resource-inbound `not_empty` condvar that was never broadcast — it never woke to
/// observe CANCELLED, hanging permanently (and leaking a detached thread on drop).
/// The data-plane `receive` was woken correctly, and the trampoline exit already
/// closes both resource queues "to wake any parent/worker blocked", which is the
/// contract cancel/drop violated (bug-205).
fn emit_close_resource_queues(ctx: &mut EmitCtx, handle_offset: usize) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    for resource_queue_offset in [
        THREAD_OFFSET_RESOURCE_INBOUND_QUEUE,
        THREAD_OFFSET_RESOURCE_OUTBOUND_QUEUE,
    ] {
        ctx.instructions.extend([
            abi::load_u64("%v8", abi::stack_pointer(), handle_offset),
            abi::load_u64("%v10", "%v8", resource_queue_offset),
            abi::move_register(abi::c_arg(0), "%v10"),
        ]);
        emit_thread_external_call(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: ctx.instructions,
                relocations: ctx.relocations,
            },
            "pthread_mutex_lock",
        )?;
        ctx.instructions.extend([
            abi::load_u64("%v8", abi::stack_pointer(), handle_offset),
            abi::load_u64("%v10", "%v8", resource_queue_offset),
            abi::move_immediate("%v9", "Integer", "1"),
            abi::store_u64("%v9", "%v10", THREAD_QUEUE_CLOSED_OFFSET),
            abi::add_immediate(abi::c_arg(0), "%v10", THREAD_QUEUE_NOT_EMPTY_OFFSET),
        ]);
        emit_thread_external_call(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: ctx.instructions,
                relocations: ctx.relocations,
            },
            "pthread_cond_broadcast",
        )?;
        ctx.instructions.extend([
            abi::load_u64("%v8", abi::stack_pointer(), handle_offset),
            abi::load_u64("%v10", "%v8", resource_queue_offset),
            abi::add_immediate(abi::c_arg(0), "%v10", THREAD_QUEUE_NOT_FULL_OFFSET),
        ]);
        emit_thread_external_call(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: ctx.instructions,
                relocations: ctx.relocations,
            },
            "pthread_cond_broadcast",
        )?;
        ctx.instructions.extend([
            abi::load_u64("%v8", abi::stack_pointer(), handle_offset),
            abi::load_u64(abi::c_arg(0), "%v8", resource_queue_offset),
        ]);
        emit_thread_external_call(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: ctx.instructions,
                relocations: ctx.relocations,
            },
            "pthread_mutex_unlock",
        )?;
    }
    Ok(())
}
