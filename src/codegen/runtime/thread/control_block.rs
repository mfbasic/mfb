//! plan-156-A: the process-global thread table and the thread control block.
//!
//! A `thread::start` allocates one **control block** from the process heap — memory
//! that belongs to no thread's arena — and registers it in the process-global
//! **thread table**. Everything the parent side and the worker side share lives in
//! the block: one lock, the run state and cancel flag, the four message/resource
//! queues (rings, not separate objects), the worker's stored result, and the
//! **block arena**, an arena state the block owns and every cross-thread value is
//! copied into.
//!
//! One lock guards all of it ([`CONTROL_OFFSET_LOCK`]). Every blocking wait
//! (`send`, `receive`, `poll`, `accept`, `waitFor`, a worker's cancellable sleep)
//! waits on a per-queue, per-direction condvar with that lock. Every allocation or
//! free in the block arena happens under it, with the pinned arena register pointed
//! at the block arena for exactly the length of the copy
//! ([`emit_call_in_block_arena`]).
//!
//! **Lifetime.** A block carries two references ([`CONTROL_OFFSET_REFS`]): the
//! handle side's and the worker's. The worker drops its reference as the last act
//! of its run (`_mfb_rt_thread_finish`); the handle side drops its reference when
//! its parent view is dropped (`thread.drop`). Whichever takes the count to zero frees
//! the block ([`lower_thread_block_release`]): the table slot, the block arena,
//! the worker's arena-state block and the block itself. The count is internal
//! bookkeeping; the language has no reference counting.
//!
//! The table exists for the runtime's own bookkeeping — a slot per live block,
//! handed out and returned under the table lock, which is taken only at start and
//! at the final release, never by a per-message operation. It grows without a
//! limit: a full slot array doubles.

use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::{finalize_vreg_body_with_locals, Vregs};
use crate::codegen::error::constants::*;
use crate::codegen::memory::data::*;
use crate::codegen::runtime::thread::*;
use crate::target::shared::abi;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// The control block
// ---------------------------------------------------------------------------

/// The block's one mutex (a `pthread_mutex_t`, an SRWLOCK on Windows). 64 bytes
/// covers every target's `pthread_mutex_t` (macOS is the largest at 64).
pub(crate) const CONTROL_OFFSET_LOCK: usize = 0;
/// `RUNNING` / `COMPLETED` / `CLOSED` ([`THREAD_STATE_RUNNING`] …).
pub(crate) const CONTROL_OFFSET_STATE: usize = 64;
/// Non-zero once the parent cancelled (or dropped) the thread.
pub(crate) const CONTROL_OFFSET_CANCELLED: usize = 72;
/// The worker's result, stored by `_mfb_rt_thread_finish`. On success the value is
/// the block-arena copy of the returned value; on failure the message and the
/// origin `ErrorLoc` are block-arena copies of the worker's.
pub(crate) const CONTROL_OFFSET_RESULT_TAG: usize = 80;
pub(crate) const CONTROL_OFFSET_RESULT_VALUE: usize = 88;
pub(crate) const CONTROL_OFFSET_RESULT_ERROR: usize = 96;
pub(crate) const CONTROL_OFFSET_RESULT_SOURCE: usize = 104;
/// The two-sided release count (§4.4 of plan-156-A): 2 at start, one for the handle
/// side and one for the worker. Only changed under the block lock.
pub(crate) const CONTROL_OFFSET_REFS: usize = 112;
/// This block's index in the thread table's slot array.
pub(crate) const CONTROL_OFFSET_SLOT: usize = 120;
/// The block arena's arena state (a heap block of [`ARENA_STATE_SIZE`] bytes).
pub(crate) const CONTROL_OFFSET_ARENA: usize = 128;
/// The per-type function that deep-copies the worker's success value into the
/// current arena (0 = a scalar, stored as is). Called by `_mfb_rt_thread_finish`
/// with the arena register pointed at the block arena.
pub(crate) const CONTROL_OFFSET_RESULT_COPY: usize = 136;
/// The OS thread handle (`pthread_t` / Windows `HANDLE`). Zeroed once the worker is
/// joined or detached, so no path joins it twice.
pub(crate) const CONTROL_OFFSET_OS_HANDLE: usize = 144;
/// The entry closure and the seed the trampoline hands it — the seed is the block-arena
/// copy the start helper made (or the word itself for a word-sized seed).
pub(crate) const CONTROL_OFFSET_ENTRY: usize = 152;
pub(crate) const CONTROL_OFFSET_DATA: usize = 160;
/// The worker's arena-state block (a heap block: arena state, seed scratch word and
/// the writable globals region, [`worker_arena_state_size`]).
pub(crate) const CONTROL_OFFSET_WORKER_ARENA: usize = 168;
/// The four queues, embedded ([`THREAD_QUEUE_SIZE`] bytes each). Each is a ring of
/// `{value, …}` entries in the block arena plus its two condvars; all four share the
/// block lock.
pub(crate) const CONTROL_OFFSET_INBOUND_QUEUE: usize = 192;
pub(crate) const CONTROL_OFFSET_OUTBOUND_QUEUE: usize =
    CONTROL_OFFSET_INBOUND_QUEUE + THREAD_QUEUE_SIZE;
/// Resource plane (§7): the inbound queue carries parent→worker transfers (parent
/// `transfer`, worker `accept`); the outbound queue carries worker→parent ones.
pub(crate) const CONTROL_OFFSET_RESOURCE_INBOUND_QUEUE: usize =
    CONTROL_OFFSET_OUTBOUND_QUEUE + THREAD_QUEUE_SIZE;
pub(crate) const CONTROL_OFFSET_RESOURCE_OUTBOUND_QUEUE: usize =
    CONTROL_OFFSET_RESOURCE_INBOUND_QUEUE + THREAD_QUEUE_SIZE;
/// plan-156-C: the worker's view — the `ThreadWorker` value the trampoline hands the
/// entry — is a resource record embedded here, `HANDLE` pointing back at this block.
/// It lives exactly as long as the worker's reference: nothing outside the worker can
/// name it, and the worker cannot outlive its own reference.
pub(crate) const CONTROL_OFFSET_WORKER_VIEW: usize =
    CONTROL_OFFSET_RESOURCE_OUTBOUND_QUEUE + THREAD_QUEUE_SIZE;
pub(crate) const CONTROL_BLOCK_SIZE: usize =
    CONTROL_OFFSET_WORKER_VIEW + RESOURCE_RECORD_SIZE_BYTES;

// Every field is addressed with an immediate displacement off the block pointer;
// rv64's load/store immediate is a signed 12-bit field.
const _: () = assert!(CONTROL_BLOCK_SIZE <= 2047);

// ---------------------------------------------------------------------------
// The thread table
// ---------------------------------------------------------------------------

/// The process-global thread table, a writable data object.
pub(crate) const THREAD_TABLE_SYMBOL: &str = "_mfb_rt_thread_table";
/// The table mutex: statically initialized (the `os::` env lock's bytes), so it is
/// valid before any code runs.
pub(crate) const THREAD_TABLE_LOCK: usize = 0;
/// The slot array (a heap block of `capacity` words). A used slot holds its block
/// pointer; a free slot holds `(next_free << 1) | 1`, where `next_free` is the next
/// free index plus one (0 ends the list) — the low bit tells the two apart, since a
/// block pointer is 8-aligned.
pub(crate) const THREAD_TABLE_SLOTS: usize = 64;
pub(crate) const THREAD_TABLE_CAPACITY: usize = 72;
/// Live blocks.
pub(crate) const THREAD_TABLE_USED: usize = 80;
/// Head of the free-slot list: a free index plus one, 0 when the list is empty.
pub(crate) const THREAD_TABLE_FREE: usize = 88;
/// Slots ever handed out: every index below this has been used at least once.
pub(crate) const THREAD_TABLE_HIGH: usize = 96;
pub(crate) const THREAD_TABLE_SIZE: usize = 104;
/// The slot array's first capacity; each growth doubles it.
pub(crate) const THREAD_TABLE_INITIAL_CAPACITY: usize = 64;

/// `_mfb_rt_thread_table_acquire(block)`: give `block` a table slot (storing its
/// index at [`CONTROL_OFFSET_SLOT`]). `ErrOutOfMemory` when the slot array had to
/// grow and the heap refused.
pub(crate) const THREAD_TABLE_ACQUIRE_SYMBOL: &str = "_mfb_rt_thread_table_acquire";
/// `_mfb_rt_thread_table_release(index)`: return a slot to the free list.
pub(crate) const THREAD_TABLE_RELEASE_SYMBOL: &str = "_mfb_rt_thread_table_release";
/// `_mfb_rt_thread_block_arena_create()`: a fresh block arena state in the value
/// register (tag `Ok`), or `ErrOutOfMemory`.
pub(crate) const THREAD_BLOCK_ARENA_CREATE_SYMBOL: &str = "_mfb_rt_thread_block_arena_create";
/// `_mfb_rt_thread_block_arena_destroy(state)`: unmap every chunk the block arena
/// mapped, retire it from the `--debug` registry, and free its state.
pub(crate) const THREAD_BLOCK_ARENA_DESTROY_SYMBOL: &str = "_mfb_rt_thread_block_arena_destroy";
/// `_mfb_rt_thread_block_release(block)`: free a block whose release count reached
/// zero — its slot, its block arena, the worker's arena-state block and the block.
pub(crate) const THREAD_BLOCK_RELEASE_SYMBOL: &str = "_mfb_rt_thread_block_release";
/// `_mfb_rt_thread_finish(block, tag, value, message, source)`: the worker's exit,
/// called by the trampoline once the entry returned.
pub(crate) const THREAD_FINISH_SYMBOL: &str = "_mfb_rt_thread_finish";

/// The thread table's data object: the statically initialized mutex, then zeroed
/// bookkeeping words (no slot array until the first `thread::start`).
pub(crate) fn thread_table_data_object(family: PlatformFamily) -> CodeDataObject {
    let mut value = crate::codegen::builtins::os::os_env_lock_init_hex(family);
    debug_assert_eq!(
        value.len(),
        crate::codegen::builtins::os::OS_ENV_LOCK_SIZE * 2
    );
    debug_assert_eq!(
        crate::codegen::builtins::os::OS_ENV_LOCK_SIZE,
        THREAD_TABLE_SLOTS
    );
    for _ in THREAD_TABLE_SLOTS..THREAD_TABLE_SIZE {
        value.push_str("00");
    }
    CodeDataObject {
        symbol: THREAD_TABLE_SYMBOL.to_string(),
        kind: "raw".to_string(),
        layout: "mfb.runtime.thread_table.v1 { u8 mutex[64]; u64 slots; u64 capacity; \
                 u64 used; u64 free; u64 high }"
            .to_string(),
        align: 8,
        size: THREAD_TABLE_SIZE,
        value,
    }
}

fn thread_call(
    symbol: &str,
    call: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    emit_thread_external_call(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions,
            relocations,
        },
        call,
    )
}

fn table_lock(
    symbol: &str,
    call: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    push_symbol_address(
        symbol,
        THREAD_TABLE_SYMBOL,
        abi::c_arg(0),
        instructions,
        relocations,
    );
    instructions.push(abi::add_immediate(
        abi::c_arg(0),
        abi::c_arg(0),
        THREAD_TABLE_LOCK,
    ));
    thread_call(
        symbol,
        call,
        platform_imports,
        platform,
        instructions,
        relocations,
    )
}

fn finish_helper(
    name: &str,
    symbol: &str,
    returns: &str,
    mut instructions: Vec<CodeInstruction>,
    relocations: Vec<CodeRelocation>,
    locals: usize,
) -> CodeFunction {
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], locals);
    CodeFunction {
        name: name.to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: returns.to_string(),
        frame,
        instructions,
        relocations,
        stack_slots,
    }
}

/// `_mfb_rt_thread_table_acquire(block)`.
pub(crate) fn lower_thread_table_acquire(
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<CodeFunction, String> {
    let symbol = THREAD_TABLE_ACQUIRE_SYMBOL;
    const LOCALS: usize = 48;
    const BLOCK: usize = 8;
    const INDEX: usize = 16;
    const NEW_SLOTS: usize = 24;
    const NEW_CAPACITY: usize = 32;
    let from_free = format!("{symbol}_from_free");
    let fresh = format!("{symbol}_fresh");
    let grow = format!("{symbol}_grow");
    let grow_copy = format!("{symbol}_grow_copy");
    let grow_copied = format!("{symbol}_grow_copied");
    let grow_no_old = format!("{symbol}_grow_no_old");
    let have_capacity = format!("{symbol}_have_capacity");
    let store = format!("{symbol}_store");
    let oom = format!("{symbol}_oom");
    let done = format!("{symbol}_done");
    let mut vregs = Vregs::new();
    let r = &ThreadVregs::mint(&mut vregs);
    let mut instructions = vec![
        abi::label("entry"),
        abi::store_u64(abi::c_arg(0), abi::stack_pointer(), BLOCK),
    ];
    let mut relocations = Vec::new();
    table_lock(
        symbol,
        "pthread_mutex_lock",
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    push_symbol_address(
        symbol,
        THREAD_TABLE_SYMBOL,
        &r.v8,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        // A free slot first: reuse keeps the array as small as the peak live count.
        abi::load_u64(&r.v9, &r.v8, THREAD_TABLE_FREE),
        abi::compare_immediate(&r.v9, "0"),
        abi::branch_ne(&from_free),
        // Otherwise the next never-used slot, growing the array when it is full.
        abi::load_u64(&r.v9, &r.v8, THREAD_TABLE_HIGH),
        abi::load_u64(&r.v10, &r.v8, THREAD_TABLE_CAPACITY),
        abi::compare_registers(&r.v9, &r.v10),
        abi::branch_lo(&fresh),
        abi::branch(&grow),
        abi::label(&from_free),
        // %v9 = index + 1; the free slot holds `(next << 1) | 1`.
        abi::subtract_immediate(&r.v9, &r.v9, 1),
        abi::store_u64(&r.v9, abi::stack_pointer(), INDEX),
        abi::load_u64(&r.v10, &r.v8, THREAD_TABLE_SLOTS),
        abi::shift_left_immediate(&r.v11, &r.v9, 3),
        abi::add_registers(&r.v10, &r.v10, &r.v11),
        abi::load_u64(&r.v11, &r.v10, 0),
        abi::shift_right_immediate(&r.v11, &r.v11, 1),
        abi::store_u64(&r.v11, &r.v8, THREAD_TABLE_FREE),
        abi::branch(&store),
        abi::label(&fresh),
        abi::store_u64(&r.v9, abi::stack_pointer(), INDEX),
        abi::add_immediate(&r.v9, &r.v9, 1),
        abi::store_u64(&r.v9, &r.v8, THREAD_TABLE_HIGH),
        abi::branch(&store),
        // Grow: a new array twice the size (the initial capacity when there is none),
        // the used prefix copied across, the old array freed. Blocks never move — the
        // array holds pointers to them — so nothing outside the table notices.
        abi::label(&grow),
        abi::load_u64(&r.v10, &r.v8, THREAD_TABLE_CAPACITY),
        abi::compare_immediate(&r.v10, "0"),
        abi::branch_ne(&have_capacity),
        abi::move_immediate(
            &r.v10,
            "Integer",
            &(THREAD_TABLE_INITIAL_CAPACITY / 2).to_string(),
        ),
        abi::label(&have_capacity),
        abi::add_registers(&r.v10, &r.v10, &r.v10),
        abi::store_u64(&r.v10, abi::stack_pointer(), NEW_CAPACITY),
        abi::shift_left_immediate(abi::c_arg(0), &r.v10, 3),
    ]);
    platform.emit_heap_alloc(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_eq(&oom),
        abi::store_u64(abi::c_return(0), abi::stack_pointer(), NEW_SLOTS),
    ]);
    push_symbol_address(
        symbol,
        THREAD_TABLE_SYMBOL,
        &r.v8,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        // Copy the `high` used words (every slot below it, used or free).
        abi::load_u64(&r.v9, &r.v8, THREAD_TABLE_SLOTS),
        abi::load_u64(&r.v10, abi::stack_pointer(), NEW_SLOTS),
        abi::load_u64(&r.v11, &r.v8, THREAD_TABLE_HIGH),
        abi::shift_left_immediate(&r.v11, &r.v11, 3),
        abi::add_registers(&r.v11, &r.v11, &r.v9),
        abi::compare_registers(&r.v9, &r.v11),
        abi::branch_lo(&grow_copy),
        abi::branch(&grow_copied),
        abi::label(&grow_copy),
        abi::load_u64(&r.v12, &r.v9, 0),
        abi::store_u64(&r.v12, &r.v10, 0),
        abi::add_immediate(&r.v9, &r.v9, 8),
        abi::add_immediate(&r.v10, &r.v10, 8),
        abi::compare_registers(&r.v9, &r.v11),
        abi::branch_lo(&grow_copy),
        abi::label(&grow_copied),
        abi::load_u64(abi::c_arg(0), &r.v8, THREAD_TABLE_SLOTS),
        abi::compare_immediate(abi::c_arg(0), "0"),
        abi::branch_eq(&grow_no_old),
    ]);
    platform.emit_heap_free(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.push(abi::label(&grow_no_old));
    push_symbol_address(
        symbol,
        THREAD_TABLE_SYMBOL,
        &r.v8,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::load_u64(&r.v9, abi::stack_pointer(), NEW_SLOTS),
        abi::store_u64(&r.v9, &r.v8, THREAD_TABLE_SLOTS),
        abi::load_u64(&r.v9, abi::stack_pointer(), NEW_CAPACITY),
        abi::store_u64(&r.v9, &r.v8, THREAD_TABLE_CAPACITY),
        abi::load_u64(&r.v9, &r.v8, THREAD_TABLE_HIGH),
        abi::store_u64(&r.v9, abi::stack_pointer(), INDEX),
        abi::add_immediate(&r.v9, &r.v9, 1),
        abi::store_u64(&r.v9, &r.v8, THREAD_TABLE_HIGH),
        // Store the block at its slot and its index in the block.
        abi::label(&store),
        abi::load_u64(&r.v9, abi::stack_pointer(), INDEX),
        abi::load_u64(&r.v10, &r.v8, THREAD_TABLE_SLOTS),
        abi::shift_left_immediate(&r.v11, &r.v9, 3),
        abi::add_registers(&r.v10, &r.v10, &r.v11),
        abi::load_u64(&r.v12, abi::stack_pointer(), BLOCK),
        abi::store_u64(&r.v12, &r.v10, 0),
        abi::store_u64(&r.v9, &r.v12, CONTROL_OFFSET_SLOT),
        abi::load_u64(&r.v9, &r.v8, THREAD_TABLE_USED),
        abi::add_immediate(&r.v9, &r.v9, 1),
        abi::store_u64(&r.v9, &r.v8, THREAD_TABLE_USED),
    ]);
    table_lock(
        symbol,
        "pthread_mutex_unlock",
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&oom),
    ]);
    table_lock(
        symbol,
        "pthread_mutex_unlock",
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    raise_error_into(
        symbol,
        "ErrOutOfMemory",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    Ok(finish_helper(
        "runtime.thread_table_acquire",
        symbol,
        "Nothing",
        instructions,
        relocations,
        LOCALS,
    ))
}

/// `_mfb_rt_thread_table_release(index)`.
pub(crate) fn lower_thread_table_release(
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<CodeFunction, String> {
    let symbol = THREAD_TABLE_RELEASE_SYMBOL;
    const LOCALS: usize = 16;
    const INDEX: usize = 8;
    let mut vregs = Vregs::new();
    let r = &ThreadVregs::mint(&mut vregs);
    let mut instructions = vec![
        abi::label("entry"),
        abi::store_u64(abi::c_arg(0), abi::stack_pointer(), INDEX),
    ];
    let mut relocations = Vec::new();
    table_lock(
        symbol,
        "pthread_mutex_lock",
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    push_symbol_address(
        symbol,
        THREAD_TABLE_SYMBOL,
        &r.v8,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::load_u64(&r.v9, abi::stack_pointer(), INDEX),
        abi::load_u64(&r.v10, &r.v8, THREAD_TABLE_SLOTS),
        abi::shift_left_immediate(&r.v11, &r.v9, 3),
        abi::add_registers(&r.v10, &r.v10, &r.v11),
        // slot = (free << 1) | 1; free = index + 1.
        abi::load_u64(&r.v11, &r.v8, THREAD_TABLE_FREE),
        abi::shift_left_immediate(&r.v11, &r.v11, 1),
        abi::add_immediate(&r.v11, &r.v11, 1),
        abi::store_u64(&r.v11, &r.v10, 0),
        abi::add_immediate(&r.v9, &r.v9, 1),
        abi::store_u64(&r.v9, &r.v8, THREAD_TABLE_FREE),
        abi::load_u64(&r.v9, &r.v8, THREAD_TABLE_USED),
        abi::subtract_immediate(&r.v9, &r.v9, 1),
        abi::store_u64(&r.v9, &r.v8, THREAD_TABLE_USED),
    ]);
    table_lock(
        symbol,
        "pthread_mutex_unlock",
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::return_(),
    ]);
    Ok(finish_helper(
        "runtime.thread_table_release",
        symbol,
        "Nothing",
        instructions,
        relocations,
        LOCALS,
    ))
}

// ---------------------------------------------------------------------------
// The block arena
// ---------------------------------------------------------------------------

/// `_mfb_rt_thread_block_arena_create()`: a zeroed arena state from the heap, its
/// entropy-fill stream seeded from the calling thread's (XOR the state's address, so
/// each block arena poisons with a distinct stream), and — in a `--debug` build —
/// registered in the arena registry as a `block` arena. No chunk is mapped until the
/// first allocation.
pub(crate) fn lower_thread_block_arena_create(
    debug_arena_registry: bool,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<CodeFunction, String> {
    let symbol = THREAD_BLOCK_ARENA_CREATE_SYMBOL;
    const LOCALS: usize = 16;
    const STATE: usize = 8;
    let zero = format!("{symbol}_zero");
    let oom = format!("{symbol}_oom");
    let done = format!("{symbol}_done");
    let mut vregs = Vregs::new();
    let r = &ThreadVregs::mint(&mut vregs);
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_immediate(abi::c_arg(0), "Integer", &ARENA_STATE_SIZE.to_string()),
    ];
    let mut relocations = Vec::new();
    platform.emit_heap_alloc(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_eq(&oom),
        abi::store_u64(abi::c_return(0), abi::stack_pointer(), STATE),
        abi::move_register(&r.v9, abi::c_return(0)),
        abi::add_immediate(&r.v10, &r.v9, ARENA_STATE_SIZE),
        abi::label(&zero),
        abi::store_u64(abi::ZERO, &r.v9, 0),
        abi::add_immediate(&r.v9, &r.v9, 8),
        abi::compare_registers(&r.v9, &r.v10),
        abi::branch_lo(&zero),
    ]);
    if debug_arena_registry {
        instructions.extend([
            abi::load_u64(abi::c_arg(0), abi::stack_pointer(), STATE),
            abi::move_immediate(
                abi::c_arg(1),
                "Integer",
                crate::codegen::debug::ARENA_KIND_THREAD_BLOCK,
            ),
            abi::branch_link(crate::codegen::debug::DEBUG_ARENA_REGISTER_SYMBOL),
        ]);
        relocations.push(internal_branch(
            symbol,
            crate::codegen::debug::DEBUG_ARENA_REGISTER_SYMBOL,
        ));
    }
    // The fill stream: drawn from the CALLING thread's own stream (`x19` is its
    // arena, so the draw is race-free), mixed with this state's address.
    instructions.push(abi::branch_link(ARENA_FILL_NEXT_SYMBOL));
    relocations.push(internal_branch(symbol, ARENA_FILL_NEXT_SYMBOL));
    instructions.extend([
        abi::move_register(abi::c_arg(1), abi::return_register()),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), STATE),
        abi::exclusive_or_registers(abi::c_arg(1), abi::c_arg(1), abi::return_register()),
    ]);
    instructions.push(abi::branch_link(ARENA_FILL_SEED_SYMBOL));
    relocations.push(internal_branch(symbol, ARENA_FILL_SEED_SYMBOL));
    instructions.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), STATE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&oom),
    ]);
    raise_error_into(
        symbol,
        "ErrOutOfMemory",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    Ok(finish_helper(
        "runtime.thread_block_arena_create",
        symbol,
        "Integer",
        instructions,
        relocations,
        LOCALS,
    ))
}

/// Run `body` with the pinned arena register pointed at the arena state held in
/// the frame word `arena_slot`, restoring the caller's arena afterwards. `saved_slot`
/// is a free frame word that parks the caller's arena across `body`.
///
/// The arena register is callee-saved and pinned, so nothing inside `body` (an
/// ordinary helper call) moves it; every path out of `body` must fall through to the
/// restore — the copy/drop helpers this brackets return their error in the result
/// registers rather than leaving by any other route.
pub(crate) fn emit_with_arena(
    instructions: &mut Vec<CodeInstruction>,
    arena_slot: usize,
    saved_slot: usize,
    body: impl FnOnce(&mut Vec<CodeInstruction>),
) {
    instructions.extend([
        abi::store_u64(ARENA_STATE_REGISTER, abi::stack_pointer(), saved_slot),
        abi::load_u64(ARENA_STATE_REGISTER, abi::stack_pointer(), arena_slot),
    ]);
    body(instructions);
    instructions.push(abi::load_u64(
        ARENA_STATE_REGISTER,
        abi::stack_pointer(),
        saved_slot,
    ));
}

/// Call the per-type value function whose address is in the frame word `fn_slot`
/// with the value in the frame word `value_slot` as its argument, with the arena
/// register pointed at the block arena in `arena_slot` (see [`emit_with_arena`]).
/// The function's tagged result is left in the result registers. `callee` is a vreg
/// the caller minted (bug-721: never a hand-numbered one) that carries the function's
/// address to the indirect call.
pub(crate) fn emit_call_in_block_arena(
    instructions: &mut Vec<CodeInstruction>,
    callee: impl Into<abi::Reg>,
    arena_slot: usize,
    saved_slot: usize,
    fn_slot: usize,
    value_slot: usize,
) {
    let callee: abi::Reg = callee.into();
    emit_with_arena(instructions, arena_slot, saved_slot, |instructions| {
        instructions.extend([
            abi::load_u64(abi::c_arg(0), abi::stack_pointer(), value_slot),
            abi::load_u64(callee, abi::stack_pointer(), fn_slot),
            abi::branch_link_register(callee),
        ]);
    });
}

/// plan-156-C: a `thread::*` op receives a thread VIEW in `c_arg(0)` — the parent's
/// `Thread` or the worker's `ThreadWorker` resource record — not the control block.
/// Branch to `closed` when the view's `RESOURCE_OFFSET_CLOSED` word is set (a dropped
/// view, or the closed default an inline `TRAP` binds); otherwise replace `c_arg(0)`
/// with the control block the view names (`RESOURCE_OFFSET_HANDLE`), so the op's body
/// works on the block. Emitted first, before the op reads any argument. `closed` is
/// emitted by [`emit_view_closed_exit`]. `scratch` is a vreg the caller minted
/// (bug-721); it holds the view's closed word.
pub(crate) fn emit_control_block_of(
    instructions: &mut Vec<CodeInstruction>,
    closed: &str,
    scratch: impl Into<abi::Reg>,
) {
    let scratch: abi::Reg = scratch.into();
    instructions.extend([
        // A null view is a binding whose drop already freed its record (a `TRAP`
        // handler reading a thread the error route dropped): closed, too.
        abi::compare_immediate(abi::c_arg(0), "0"),
        abi::branch_eq(closed),
        abi::load_u64(scratch, abi::c_arg(0), RESOURCE_OFFSET_CLOSED),
        abi::compare_immediate(scratch, "0"),
        abi::branch_ne(closed),
        abi::load_u64(abi::c_arg(0), abi::c_arg(0), RESOURCE_OFFSET_HANDLE),
    ]);
}

/// The `closed` exit of [`emit_control_block_of`]: return `ErrResourceClosed` with no
/// origin (the one op that returns an origin, `waitFor`, returns a worker's).
pub(crate) fn emit_view_closed_exit(
    symbol: &str,
    closed: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    instructions.push(abi::label(closed));
    raise_error_into(symbol, "ErrResourceClosed", instructions, relocations);
    instructions.extend([
        abi::move_immediate(RESULT_ERROR_SOURCE_REGISTER, "Integer", "0"),
        abi::return_(),
    ]);
}

/// `_mfb_rt_thread_block_arena_destroy(state)`: unmap the block arena's chunks
/// (`_mfb_arena_destroy`, run with the arena register pointed at it so it walks this
/// arena's block list and counts into this arena's `--debug` slot), retire the slot,
/// and free the state.
pub(crate) fn lower_thread_block_arena_destroy(
    debug_arena_registry: bool,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<CodeFunction, String> {
    let symbol = THREAD_BLOCK_ARENA_DESTROY_SYMBOL;
    const LOCALS: usize = 32;
    const STATE: usize = 8;
    const SAVED_ARENA: usize = 16;
    let mut vregs = Vregs::new();
    let r = &ThreadVregs::mint(&mut vregs);
    let mut instructions = vec![
        abi::label("entry"),
        abi::store_u64(abi::c_arg(0), abi::stack_pointer(), STATE),
    ];
    let mut relocations = Vec::new();
    emit_with_arena(&mut instructions, STATE, SAVED_ARENA, |instructions| {
        instructions.push(abi::branch_link(ARENA_DESTROY_SYMBOL));
    });
    relocations.push(internal_branch(symbol, ARENA_DESTROY_SYMBOL));
    if debug_arena_registry {
        instructions.push(abi::load_u64(&r.v9, abi::stack_pointer(), STATE));
        crate::codegen::debug::arena::emit_debug_arena_unregister(
            symbol,
            &r.v9,
            [&r.v10, &r.v11, &r.v12, &r.v13],
            &mut instructions,
            &mut relocations,
        );
    }
    instructions.push(abi::load_u64(abi::c_arg(0), abi::stack_pointer(), STATE));
    platform.emit_heap_free(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::return_(),
    ]);
    Ok(finish_helper(
        "runtime.thread_block_arena_destroy",
        symbol,
        "Nothing",
        instructions,
        relocations,
        LOCALS,
    ))
}

/// `_mfb_rt_thread_block_release(block)`: free a control block whose release count
/// reached zero.
///
/// Reached from the handle side's last release (`thread.drop`) or from the worker's
/// own exit (`_mfb_rt_thread_finish`), whichever took the count to zero, so nothing
/// else touches the block by now: a handle-side caller joined the worker first, and a
/// worker caller is the worker. Frees, in order: the table slot; every message still
/// owned by a queue, through the queue's drop function (which closes a resource);
/// the block arena (the seed, the result and everything else still in it go with
/// it); the worker's arena-state block (retired from the `--debug` registry first);
/// the block.
///
/// A worker releasing its OWN block frees its arena-state block while the pinned
/// arena register still points at it. That is sound because nothing after it reads
/// through the register: the frees below are heap frees, and the trampoline restores
/// the register from its own frame before it returns.
pub(crate) fn lower_thread_block_release(
    debug_arena_registry: bool,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<CodeFunction, String> {
    let symbol = THREAD_BLOCK_RELEASE_SYMBOL;
    const LOCALS: usize = 64;
    const BLOCK: usize = 8;
    // The queue drop pass: the block arena, the caller's arena parked across a drop,
    // the ring cursor and remaining count, the value being dropped, the drop function.
    const ARENA: usize = 16;
    const SAVED_ARENA: usize = 24;
    const INDEX: usize = 32;
    const LEFT: usize = 40;
    const VALUE: usize = 48;
    const DROP: usize = 56;
    let no_arena = format!("{symbol}_no_arena");
    let no_worker_arena = format!("{symbol}_no_worker_arena");
    let mut vregs = Vregs::new();
    let r = &ThreadVregs::mint(&mut vregs);
    let mut instructions = vec![
        abi::label("entry"),
        abi::store_u64(abi::c_arg(0), abi::stack_pointer(), BLOCK),
        abi::load_u64(abi::c_arg(0), abi::c_arg(0), CONTROL_OFFSET_SLOT),
        abi::branch_link(THREAD_TABLE_RELEASE_SYMBOL),
    ];
    let mut relocations = vec![internal_branch(symbol, THREAD_TABLE_RELEASE_SYMBOL)];
    // Drop every message still owned by a queue — each one never received, and the
    // last one read — with the queue's recorded drop function, before the block arena
    // they live in is destroyed. For a resource that closes the handle (the drop
    // function closes a record that is still open), so a transfer nobody accepted
    // does not leak its OS handle. A queue nothing was ever written to has no drop
    // function and is skipped; so is everything when the block has no arena (a start
    // that failed before creating it).
    for queue in [
        CONTROL_OFFSET_INBOUND_QUEUE,
        CONTROL_OFFSET_OUTBOUND_QUEUE,
        CONTROL_OFFSET_RESOURCE_INBOUND_QUEUE,
        CONTROL_OFFSET_RESOURCE_OUTBOUND_QUEUE,
    ] {
        let next = format!("{symbol}_queue_{queue}_done");
        let last_done = format!("{symbol}_queue_{queue}_last_done");
        let ring = format!("{symbol}_queue_{queue}_ring");
        let wrapped = format!("{symbol}_queue_{queue}_wrapped");
        instructions.extend([
            abi::load_u64(&r.v8, abi::stack_pointer(), BLOCK),
            abi::load_u64(&r.v9, &r.v8, CONTROL_OFFSET_ARENA),
            abi::compare_immediate(&r.v9, "0"),
            abi::branch_eq(&next),
            abi::store_u64(&r.v9, abi::stack_pointer(), ARENA),
            abi::load_u64(&r.v9, &r.v8, queue + THREAD_QUEUE_DROP_OFFSET),
            abi::compare_immediate(&r.v9, "0"),
            abi::branch_eq(&next),
            abi::store_u64(&r.v9, abi::stack_pointer(), DROP),
            abi::load_u64(&r.v9, &r.v8, queue + THREAD_QUEUE_LAST_READ_OFFSET),
            abi::compare_immediate(&r.v9, "0"),
            abi::branch_eq(&last_done),
            abi::store_u64(&r.v9, abi::stack_pointer(), VALUE),
        ]);
        emit_call_in_block_arena(&mut instructions, &r.v15, ARENA, SAVED_ARENA, DROP, VALUE);
        instructions.extend([
            abi::label(&last_done),
            abi::load_u64(&r.v8, abi::stack_pointer(), BLOCK),
            abi::load_u64(&r.v9, &r.v8, queue + THREAD_QUEUE_HEAD_OFFSET),
            abi::store_u64(&r.v9, abi::stack_pointer(), INDEX),
            abi::load_u64(&r.v9, &r.v8, queue + THREAD_QUEUE_COUNT_OFFSET),
            abi::store_u64(&r.v9, abi::stack_pointer(), LEFT),
            abi::label(&ring),
            abi::load_u64(&r.v9, abi::stack_pointer(), LEFT),
            abi::compare_immediate(&r.v9, "0"),
            abi::branch_eq(&next),
            abi::subtract_immediate(&r.v9, &r.v9, 1),
            abi::store_u64(&r.v9, abi::stack_pointer(), LEFT),
            abi::load_u64(&r.v8, abi::stack_pointer(), BLOCK),
            abi::load_u64(&r.v10, abi::stack_pointer(), INDEX),
            abi::load_u64(&r.v11, &r.v8, queue + THREAD_QUEUE_VALUES_OFFSET),
            abi::shift_left_immediate(&r.v12, &r.v10, THREAD_QUEUE_ENTRY_SHIFT),
            abi::add_registers(&r.v11, &r.v11, &r.v12),
            abi::load_u64(&r.v12, &r.v11, 0),
            abi::store_u64(&r.v12, abi::stack_pointer(), VALUE),
            // Advance the cursor, wrapping at capacity, before the drop call.
            abi::add_immediate(&r.v10, &r.v10, 1),
            abi::load_u64(&r.v11, &r.v8, queue + THREAD_QUEUE_CAPACITY_OFFSET),
            abi::compare_registers(&r.v10, &r.v11),
            abi::branch_lt(&wrapped),
            abi::move_immediate(&r.v10, "Integer", "0"),
            abi::label(&wrapped),
            abi::store_u64(&r.v10, abi::stack_pointer(), INDEX),
        ]);
        emit_call_in_block_arena(&mut instructions, &r.v15, ARENA, SAVED_ARENA, DROP, VALUE);
        instructions.extend([abi::branch(&ring), abi::label(&next)]);
    }
    instructions.extend([
        abi::load_u64(&r.v8, abi::stack_pointer(), BLOCK),
        abi::load_u64(abi::c_arg(0), &r.v8, CONTROL_OFFSET_ARENA),
        abi::compare_immediate(abi::c_arg(0), "0"),
        abi::branch_eq(&no_arena),
        abi::branch_link(THREAD_BLOCK_ARENA_DESTROY_SYMBOL),
        abi::label(&no_arena),
    ]);
    relocations.push(internal_branch(symbol, THREAD_BLOCK_ARENA_DESTROY_SYMBOL));
    instructions.extend([
        abi::load_u64(&r.v8, abi::stack_pointer(), BLOCK),
        abi::load_u64(&r.v9, &r.v8, CONTROL_OFFSET_WORKER_ARENA),
        abi::compare_immediate(&r.v9, "0"),
        abi::branch_eq(&no_worker_arena),
    ]);
    if debug_arena_registry {
        crate::codegen::debug::arena::emit_debug_arena_unregister(
            symbol,
            &r.v9,
            [&r.v10, &r.v11, &r.v12, &r.v13],
            &mut instructions,
            &mut relocations,
        );
    }
    instructions.push(abi::move_register(abi::c_arg(0), &r.v9));
    platform.emit_heap_free(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::label(&no_worker_arena),
        abi::load_u64(abi::c_arg(0), abi::stack_pointer(), BLOCK),
    ]);
    platform.emit_heap_free(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::return_(),
    ]);
    Ok(finish_helper(
        "runtime.thread_block_release",
        symbol,
        "Nothing",
        instructions,
        relocations,
        LOCALS,
    ))
}

/// Lock (`call` = `pthread_mutex_lock`) or unlock the block lock of the control
/// block held in the frame word `block_slot`.
pub(crate) fn emit_block_lock_call(
    ctx: &mut EmitCtx,
    block_slot: usize,
    call: &str,
) -> Result<(), String> {
    ctx.instructions.extend([
        abi::load_u64(abi::c_arg(0), abi::stack_pointer(), block_slot),
        abi::add_immediate(abi::c_arg(0), abi::c_arg(0), CONTROL_OFFSET_LOCK),
    ]);
    emit_thread_external_call(ctx, call)
}
