//! plan-176-P: freeing — inserting a free chunk, coalescing on flush, and `lower_arena_free`.
//! The shared chunk/bin helpers, the fill and destroy helpers live in
//! `memory/arena/arena/mod.rs`.

use super::*;

/// `arena_insert_free(x0 = ptr, x1 = size)` — insert a chunk into the
/// address-ordered free-list and coalesce with the address-adjacent neighbor on
/// either side. `size` must already be normalized (≥16, multiple of 16) and
/// `ptr` 16-aligned; both hold for every chunk the allocator hands out and for a
/// fresh block's usable region. A `ptr` that is already a free node is a no-op
/// (allocator-03 idempotency guard), so a double-free relinks nothing. Leaf
/// function; vreg-allocated — treat all caller-saved integer registers as
/// clobbered.
pub(crate) fn lower_arena_insert_free(debug_arena: bool) -> CodeFunction {
    let mut vregs = Vregs::new();
    // ptr (x0) / size (x1) are read-only args; this is a leaf, so they stay
    // physical. Everything else is a vreg the allocator places.
    let cur = vregs.next();
    let prev = vregs.next();
    let t1 = vregs.next();
    let t2 = vregs.next();
    let merged = vregs.next();
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    // plan-130-C: count the call. The lookup reads only the registry and fresh
    // vregs, so the argument registers the body reads survive it.
    if debug_arena {
        let slot = vregs.next();
        emit_debug_arena_slot(
            ARENA_INSERT_FREE_SYMBOL,
            &slot,
            &mut instructions,
            &mut relocations,
            &mut vregs,
        );
        emit_debug_arena_bump(
            ARENA_INSERT_FREE_SYMBOL,
            &slot,
            COUNTER_INSERT_FREE_CALLS,
            "insert_free_calls",
            &mut instructions,
            &mut vregs,
        );
    }
    instructions.extend([
        // Walk to the insertion slot: prev = largest node < ptr (or 0),
        // cur = smallest node > ptr (or 0).
        abi::load_u64(&cur, ARENA_STATE_REGISTER, ARENA_FREE_LIST_HEAD_OFFSET),
        abi::move_immediate(&prev, "Integer", "0"),
        abi::label("insert_find"),
        abi::compare_immediate(&cur, "0"),
        abi::branch_eq("insert_slot"),
        abi::compare_registers(&cur, abi::c_arg(0)),
        abi::branch_hi("insert_slot"), // cur > ptr
        // Idempotency guard (allocator-03): the chunk is already a free node —
        // a double-free becomes a no-op instead of double-linking `ptr` and
        // coalescing against its own (about-to-be-rewritten) metadata.
        abi::compare_registers(&cur, abi::c_arg(0)),
        abi::branch_eq("insert_already_free"),
        abi::move_register(&prev, &cur),
        abi::load_u64(&cur, &cur, 0),
        abi::branch("insert_find"),
        abi::label("insert_slot"),
        // merged = merged-into-prev flag.
        abi::move_immediate(&merged, "Integer", "0"),
        abi::compare_immediate(&prev, "0"),
        abi::branch_eq("insert_check_next"),
        abi::load_u64(&t1, &prev, 8),        // prev.size
        abi::add_registers(&t2, &prev, &t1), // prev_end
        abi::compare_registers(&t2, abi::c_arg(0)),
        abi::branch_ne("insert_check_next"),
        // prev is address-adjacent: absorb the chunk into prev.
        abi::add_registers(&t1, &t1, abi::c_arg(1)),
        abi::store_u64(&t1, &prev, 8),
        abi::move_immediate(&merged, "Integer", "1"),
        abi::label("insert_check_next"),
        abi::compare_immediate(&cur, "0"),
        abi::branch_eq("insert_finish_no_next"),
        abi::compare_immediate(&merged, "0"),
        abi::branch_eq("insert_next_unmerged"),
        // Merged into prev already: does the (now larger) prev meet cur?
        abi::load_u64(&t1, &prev, 8),
        abi::add_registers(&t2, &prev, &t1),
        abi::compare_registers(&t2, &cur),
        abi::branch_ne("insert_done"),
        // Absorb cur into prev too (three-way merge).
        abi::load_u64(&t1, &cur, 8),  // cur.size
        abi::load_u64(&t2, &prev, 8), // prev.size
        abi::add_registers(&t2, &t2, &t1),
        abi::store_u64(&t2, &prev, 8),
        abi::load_u64(&t1, &cur, 0), // cur.next
        abi::store_u64(&t1, &prev, 0),
        abi::branch("insert_done"),
        abi::label("insert_next_unmerged"),
        abi::add_registers(&t2, abi::c_arg(0), abi::c_arg(1)), // chunk_end
        abi::compare_registers(&t2, &cur),
        abi::branch_ne("insert_standalone"),
        // chunk is address-adjacent to cur: new node at ptr absorbs cur.
        abi::load_u64(&t1, &cur, 8), // cur.size
        abi::add_registers(&t1, &t1, abi::c_arg(1)),
        abi::store_u64(&t1, abi::c_arg(0), 8),
        abi::load_u64(&t1, &cur, 0), // cur.next
        abi::store_u64(&t1, abi::c_arg(0), 0),
        abi::branch("insert_link_prev"),
        abi::label("insert_standalone"),
        abi::store_u64(&cur, abi::c_arg(0), 0), // ptr.next = cur
        abi::store_u64(abi::c_arg(1), abi::c_arg(0), 8), // ptr.size = size
        abi::branch("insert_link_prev"),
        abi::label("insert_finish_no_next"),
        abi::compare_immediate(&merged, "0"),
        abi::branch_ne("insert_done"), // merged into prev, nothing to link
        abi::store_u64(abi::ZERO, abi::c_arg(0), 0), // ptr.next = 0
        abi::store_u64(abi::c_arg(1), abi::c_arg(0), 8), // ptr.size = size
        abi::branch("insert_link_prev"),
        abi::label("insert_link_prev"),
        abi::compare_immediate(&prev, "0"),
        abi::branch_eq("insert_set_head"),
        abi::store_u64(abi::c_arg(0), &prev, 0), // prev.next = ptr
        abi::branch("insert_done"),
        abi::label("insert_set_head"),
        abi::store_u64(
            abi::c_arg(0),
            ARENA_STATE_REGISTER,
            ARENA_FREE_LIST_HEAD_OFFSET,
        ),
        abi::label("insert_done"),
        abi::return_(),
        abi::label("insert_already_free"),
        abi::return_(),
    ]);
    finalize_vreg_helper(
        "runtime.arena_insert_free",
        ARENA_INSERT_FREE_SYMBOL,
        "Nothing",
        instructions,
        relocations,
    )
}

/// `arena_flush_coalesce()` — plan-64 A1 single-pass flush-before-grow coalesce.
///
/// Gathers the address-ordered free-list head, every quick-bin chain and every
/// large-bin chain into one singly-linked chain (`{next@0, size@8}` overlay, chained
/// via `@0`), sorts that chain by chunk address with an in-place bottom-up
/// linked-list merge sort, then walks it once coalescing physically-adjacent chunks
/// (`cur + cur.size == next`) and re-parking each survivor: `≤ ARENA_QUICK_BIN_MAX`
/// onto its exact-size quick bin, larger onto its large-bin size class (bug-690 —
/// it used to be appended to the address-ordered list, which the flush now leaves
/// empty). It then restarts the large-bin flush credit at minus
/// `ARENA_LARGE_FLUSH_BYTES_PER_CHUNK` per chunk it sorted, which is what gates the
/// big-request flush in `arena_alloc`.
///
/// This replaces the old drain that fed each parked chunk through
/// `arena_insert_free` — an O(free-list) walk per chunk, so O(M²) over M parked
/// chunks, the arena mixed-transient-churn quadratic. The merge sort makes the
/// whole flush O(M log M). Coalescing is address-exact (`cur + size == next`), so a
/// mis-ordered sort can only *miss* an adjacency (under-coalesce, a memory
/// shortfall) — it can never fabricate an overlap, so no sort defect can hand the
/// same bytes to two allocations. Leaf, vreg-allocated: all caller-saved integer
/// registers are clobbered (the caller spills its live `size`/`eff_align`).
pub(crate) fn lower_arena_flush_coalesce(debug_arena: bool) -> CodeFunction {
    let mut vregs = Vregs::new();
    // Gather cursors.
    let gathered = vregs.next();
    let gi = vregs.next();
    let gslot = vregs.next();
    let gnode = vregs.next();
    let gnext = vregs.next();
    // Merge-sort cursors.
    let width = vregs.next();
    let p = vregs.next();
    let q = vregs.next();
    let e = vregs.next();
    let tail = vregs.next();
    let nmerges = vregs.next();
    let psize = vregs.next();
    let qsize = vregs.next();
    let step = vregs.next();
    let newhead = vregs.next();
    // Coalesce cursors.
    let cur = vregs.next();
    let cnext = vregs.next();
    let csize = vregs.next();
    let nsize = vregs.next();
    let cbin = vregs.next();
    let cbinhead = vregs.next();
    let cend = vregs.next();
    let total = vregs.next();
    let lk = vregs.next();
    let lshift = vregs.next();
    let lsub = vregs.next();
    let quick_max = ARENA_QUICK_BIN_MAX.to_string();
    let bin_count = ARENA_QUICK_BIN_COUNT.to_string();
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    // plan-130-C: count the call. The lookup reads only the registry and fresh
    // vregs, so the argument registers the body reads survive it.
    if debug_arena {
        let slot = vregs.next();
        emit_debug_arena_slot(
            ARENA_FLUSH_COALESCE_SYMBOL,
            &slot,
            &mut instructions,
            &mut relocations,
            &mut vregs,
        );
        emit_debug_arena_bump(
            ARENA_FLUSH_COALESCE_SYMBOL,
            &slot,
            COUNTER_FLUSHES,
            "flushes",
            &mut instructions,
            &mut vregs,
        );
    }
    instructions.extend([
        // --- Phase 1: gather list + every quick bin into `gathered` ------------
        abi::load_u64(&gathered, ARENA_STATE_REGISTER, ARENA_FREE_LIST_HEAD_OFFSET),
        abi::store_u64(abi::ZERO, ARENA_STATE_REGISTER, ARENA_FREE_LIST_HEAD_OFFSET),
        abi::move_immediate(&gi, "Integer", "0"),
        abi::label("fc_gather_bin"),
        abi::compare_immediate(&gi, &bin_count),
        abi::branch_eq("fc_gather_done"),
        abi::shift_left_immediate(&gslot, &gi, 3),
        abi::add_registers(&gslot, ARENA_STATE_REGISTER, &gslot),
        abi::load_u64(&gnode, &gslot, ARENA_QUICK_BIN_BASE_OFFSET),
        abi::store_u64(abi::ZERO, &gslot, ARENA_QUICK_BIN_BASE_OFFSET),
        abi::label("fc_gather_chain"),
        abi::compare_immediate(&gnode, "0"),
        abi::branch_eq("fc_gather_nextbin"),
        abi::load_u64(&gnext, &gnode, 0),
        abi::store_u64(&gathered, &gnode, 0), // gnode.next = gathered
        abi::move_register(&gathered, &gnode), // gathered = gnode
        abi::move_register(&gnode, &gnext),
        abi::branch("fc_gather_chain"),
        abi::label("fc_gather_nextbin"),
        abi::add_immediate(&gi, &gi, 1),
        abi::branch("fc_gather_bin"),
        // bug-690: the large bins too, so large frees coalesce with each other and
        // with the small crumbs between them.
        abi::label("fc_gather_done"),
        abi::move_immediate(&gi, "Integer", "0"),
        abi::label("fc_gather_large_bin"),
        abi::compare_immediate(&gi, &ARENA_LARGE_BIN_COUNT.to_string()),
        abi::branch_eq("fc_gather_large_done"),
        abi::shift_left_immediate(&gslot, &gi, 3),
        abi::add_registers(&gslot, ARENA_STATE_REGISTER, &gslot),
        abi::load_u64(&gnode, &gslot, ARENA_LARGE_BIN_BASE_OFFSET),
        abi::store_u64(abi::ZERO, &gslot, ARENA_LARGE_BIN_BASE_OFFSET),
        abi::label("fc_gather_large_chain"),
        abi::compare_immediate(&gnode, "0"),
        abi::branch_eq("fc_gather_large_nextbin"),
        abi::load_u64(&gnext, &gnode, 0),
        abi::store_u64(&gathered, &gnode, 0), // gnode.next = gathered
        abi::move_register(&gathered, &gnode), // gathered = gnode
        abi::move_register(&gnode, &gnext),
        abi::branch("fc_gather_large_chain"),
        abi::label("fc_gather_large_nextbin"),
        abi::add_immediate(&gi, &gi, 1),
        abi::branch("fc_gather_large_bin"),
        abi::label("fc_gather_large_done"),
        abi::move_immediate(&total, "Integer", "0"),
        abi::compare_immediate(&gathered, "0"),
        abi::branch_eq("fc_ret"), // nothing parked
        // --- Phase 2: bottom-up merge sort `gathered` by chunk address --------
        abi::move_immediate(&width, "Integer", "1"),
        abi::label("fc_sort_outer"),
        abi::move_register(&p, &gathered),
        abi::move_immediate(&newhead, "Integer", "0"),
        abi::move_immediate(&tail, "Integer", "0"),
        abi::move_immediate(&nmerges, "Integer", "0"),
        abi::label("fc_sort_pass"),
        abi::compare_immediate(&p, "0"),
        abi::branch_eq("fc_sort_passdone"),
        abi::add_immediate(&nmerges, &nmerges, 1),
        // q = p advanced by `width`; psize = actual length of p's run (≤ width).
        abi::move_register(&q, &p),
        abi::move_immediate(&psize, "Integer", "0"),
        abi::move_immediate(&step, "Integer", "0"),
        abi::label("fc_advq"),
        abi::compare_registers(&step, &width),
        abi::branch_eq("fc_advq_done"),
        abi::compare_immediate(&q, "0"),
        abi::branch_eq("fc_advq_done"),
        abi::add_immediate(&psize, &psize, 1),
        abi::load_u64(&q, &q, 0),
        abi::add_immediate(&step, &step, 1),
        abi::branch("fc_advq"),
        abi::label("fc_advq_done"),
        abi::move_register(&qsize, &width), // q's run is ≤ width; guarded by q==0
        // Merge p's run (psize) with q's run (qsize).
        abi::label("fc_merge"),
        abi::compare_immediate(&psize, "0"),
        abi::branch_ne("fc_merge_p_live"),
        // p exhausted: take from q unless q's run is also done.
        abi::compare_immediate(&qsize, "0"),
        abi::branch_eq("fc_merge_done"),
        abi::compare_immediate(&q, "0"),
        abi::branch_eq("fc_merge_done"),
        abi::move_register(&e, &q),
        abi::load_u64(&q, &q, 0),
        abi::subtract_immediate(&qsize, &qsize, 1),
        abi::branch("fc_merge_append"),
        abi::label("fc_merge_p_live"),
        // p available; take q only if q's run has nodes left.
        abi::compare_immediate(&qsize, "0"),
        abi::branch_eq("fc_merge_take_p"),
        abi::compare_immediate(&q, "0"),
        abi::branch_eq("fc_merge_take_p"),
        // Both live: take the lower address (stable: ties take p).
        abi::compare_registers(&p, &q),
        abi::branch_hi("fc_merge_take_q"),
        abi::label("fc_merge_take_p"),
        abi::move_register(&e, &p),
        abi::load_u64(&p, &p, 0),
        abi::subtract_immediate(&psize, &psize, 1),
        abi::branch("fc_merge_append"),
        abi::label("fc_merge_take_q"),
        abi::move_register(&e, &q),
        abi::load_u64(&q, &q, 0),
        abi::subtract_immediate(&qsize, &qsize, 1),
        abi::label("fc_merge_append"),
        abi::compare_immediate(&tail, "0"),
        abi::branch_eq("fc_merge_sethead"),
        abi::store_u64(&e, &tail, 0), // tail.next = e
        abi::branch("fc_merge_settail"),
        abi::label("fc_merge_sethead"),
        abi::move_register(&newhead, &e),
        abi::label("fc_merge_settail"),
        abi::move_register(&tail, &e),
        abi::branch("fc_merge"),
        abi::label("fc_merge_done"),
        abi::move_register(&p, &q), // advance to the next pair
        abi::branch("fc_sort_pass"),
        abi::label("fc_sort_passdone"),
        abi::store_u64(abi::ZERO, &tail, 0), // terminate the rebuilt chain
        abi::move_register(&gathered, &newhead),
        abi::compare_immediate(&nmerges, "1"),
        abi::branch_hi("fc_sort_more"),
        abi::branch("fc_coalesce"),
        abi::label("fc_sort_more"),
        abi::shift_left_immediate(&width, &width, 1),
        abi::branch("fc_sort_outer"),
        // --- Phase 3: coalesce adjacent + re-park -----------------------------
        abi::label("fc_coalesce"),
        abi::move_register(&cur, &gathered),
        abi::label("fc_coalesce_loop"),
        abi::compare_immediate(&cur, "0"),
        abi::branch_eq("fc_ret"),
        abi::load_u64(&cnext, &cur, 0),
        abi::load_u64(&csize, &cur, 8),
        abi::label("fc_absorb"),
        abi::compare_immediate(&cnext, "0"),
        abi::branch_eq("fc_place"),
        abi::add_registers(&cend, &cur, &csize),
        abi::compare_registers(&cend, &cnext),
        abi::branch_ne("fc_place"), // not physically adjacent
        abi::load_u64(&nsize, &cnext, 8),
        abi::add_registers(&csize, &csize, &nsize),
        abi::load_u64(&cnext, &cnext, 0), // drop the absorbed node
        abi::add_immediate(&total, &total, 1),
        abi::branch("fc_absorb"),
        abi::label("fc_place"),
        abi::store_u64(&csize, &cur, 8),
        abi::add_immediate(&total, &total, 1),
        abi::compare_immediate(&csize, &quick_max),
        abi::branch_hi("fc_place_large"),
        // Re-park onto the exact-size quick bin (class csize/16, head at
        // state + (csize>>4<<3) + QUICK_BIN_BASE_OFFSET-8).
        abi::shift_right_immediate(&cbin, &csize, 4),
        abi::shift_left_immediate(&cbin, &cbin, 3),
        abi::add_registers(&cbin, ARENA_STATE_REGISTER, &cbin),
        abi::load_u64(&cbinhead, &cbin, ARENA_QUICK_BIN_BASE_OFFSET - 8),
        abi::store_u64(&cbinhead, &cur, 0),
        abi::store_u64(&cur, &cbin, ARENA_QUICK_BIN_BASE_OFFSET - 8),
        abi::branch("fc_place_next"),
        // bug-690: a larger run parks on its large-bin size class, where a request
        // of any size it fits finds it in O(1). It used to be appended to the
        // address-ordered list, whose first-fit walk is O(list) per request.
        abi::label("fc_place_large"),
    ]);
    emit_large_bin_slot(
        &cbin,
        &csize,
        &lk,
        &lshift,
        &lsub,
        "fc_place_large_class",
        &mut instructions,
    );
    instructions.extend([
        abi::load_u64(&cbinhead, &cbin, ARENA_LARGE_BIN_BASE_OFFSET),
        abi::store_u64(&cbinhead, &cur, 0),
        abi::store_u64(&cur, &cbin, ARENA_LARGE_BIN_BASE_OFFSET),
        abi::label("fc_place_next"),
        abi::move_register(&cur, &cnext),
        abi::branch("fc_coalesce_loop"),
        // bug-690: the large-bin credit restarts at minus this flush's cost — the
        // `total` chunks it sorted, at ARENA_LARGE_FLUSH_BYTES_PER_CHUNK each — so
        // the next big-request flush waits until that many large bytes were freed.
        abi::label("fc_ret"),
        abi::move_immediate(
            &cbin,
            "Integer",
            &ARENA_LARGE_FLUSH_CREDIT_OFFSET.to_string(),
        ),
        abi::add_registers(&cbin, ARENA_STATE_REGISTER, &cbin),
        abi::shift_left_immediate(
            &total,
            &total,
            ARENA_LARGE_FLUSH_BYTES_PER_CHUNK.trailing_zeros() as u8,
        ),
        abi::move_immediate(&lk, "Integer", "0"),
        abi::subtract_registers(&total, &lk, &total),
        abi::store_u64(&total, &cbin, 0),
        abi::return_(),
    ]);
    finalize_vreg_helper(
        "runtime.arena_flush_coalesce",
        ARENA_FLUSH_COALESCE_SYMBOL,
        "Nothing",
        instructions,
        relocations,
    )
}

/// `arena_free(x0 = ptr, x1 = size)` — return a single compiler-sized allocation
/// to the per-arena allocator. Normalizes `size` exactly as `arena_alloc` did
/// (so the freed extent matches the live chunk), then parks the chunk in O(1):
/// on its exact-size quick bin (`size ≤ ARENA_QUICK_BIN_MAX` — allocator-01) or on
/// its large-bin size class (plan-25-A, bug-690), crediting the large-bin flush
/// with the bytes. Afterwards it zero-fills the payload bytes past the 16-byte
/// FreeNode overlay just written (`_mfb_arena_fill_zero`; allocator-03: the scrub must
/// never overwrite free-list metadata, and an immediate double-free — skipped by the
/// bin-head guard — must never scrub a live node's `{next, size}` words). The node's own
/// two words overwrite the first 16 payload bytes, so every freed byte is overwritten.
/// Never unmaps. Vreg-allocated — treat all caller-saved integer registers as clobbered.
/// `perf` (plan-130-B): bracket the body with the `mfb_free` perf span — true exactly
/// when the `--debug` perf section is active for the module.
pub(crate) fn lower_arena_free(perf: bool, debug_arena: bool) -> CodeFunction {
    let mut vregs = Vregs::new();
    let not_15 = (!(ARENA_MIN_CHUNK - 1)).to_string();
    // ptr/size are live across both helper calls; each tramples every integer
    // register (ALL_INT), so the allocator spills them and reloads before each
    // call — exactly what the old hand frame did with its PTR/SIZE slots.
    let ptr = vregs.next();
    let size = vregs.next();
    let mask = vregs.next();
    let bin_class = vregs.next();
    let bin_slot = vregs.next();
    let bin_head = vregs.next();
    let mut relocations = vec![internal_branch(ARENA_FREE_SYMBOL, ARENA_FILL_ZERO_SYMBOL)];
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&ptr, abi::c_arg(0)),
        // normalize size = round_up(max(size, 1), 16) — x1 is the size arg.
        abi::compare_immediate(abi::c_arg(1), "0"),
        abi::branch_ne("arena_free_size_nonzero"),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
        abi::label("arena_free_size_nonzero"),
        abi::add_immediate(abi::c_arg(1), abi::c_arg(1), (ARENA_MIN_CHUNK - 1) as usize),
        abi::move_immediate(&mask, "Integer", &not_15),
        abi::and_registers(&size, abi::c_arg(1), &mask),
    ];
    // plan-130-C: find this arena's registry slot once, then count the call and its
    // normalized size. In a `--debug` build the double-free exits go through a
    // counting stub emitted after the return.
    let mut dbg_slot: Option<VirtualRegister> = None;
    if debug_arena {
        let slot = vregs.next();
        emit_debug_arena_slot(
            ARENA_FREE_SYMBOL,
            &slot,
            &mut instructions,
            &mut relocations,
            &mut vregs,
        );
        emit_debug_arena_bump(
            ARENA_FREE_SYMBOL,
            &slot,
            COUNTER_FREE_CALLS,
            "free_calls",
            &mut instructions,
            &mut vregs,
        );
        emit_debug_arena_add(
            ARENA_FREE_SYMBOL,
            &slot,
            COUNTER_FREE_BYTES,
            &size,
            "free_bytes",
            &mut instructions,
            &mut vregs,
        );
        dbg_slot = Some(slot);
    }
    let double_free = if debug_arena {
        "arena_free_double"
    } else {
        "arena_free_done"
    };
    // plan-67-F: open the `mfb_free` span now that ptr/size are captured in vregs
    // (spilled across the perf `bl`); the body below re-derives ARG[0]/ARG[1].
    if perf {
        emit_perf_arena_call(
            "perf.start",
            ARENA_FREE_SYMBOL,
            PERF_NAME_MFB_FREE_SYMBOL,
            &mut instructions,
            &mut relocations,
        );
    }
    instructions.extend([
        // Quick-bin park (allocator-01): a chunk ≤ ARENA_QUICK_BIN_MAX pushes
        // onto its exact-size bin head in O(1) — no list walk. The bin slot for
        // class `size/16 - 1` sits at `state + BASE + (size/16 - 1)*8`, i.e.
        // `state + (size >> 4 << 3) + (BASE - 8)`. Bin nodes reuse the FreeNode
        // {next, size} overlay, so a flush can hand them straight to
        // `arena_insert_free`.
        abi::compare_immediate(&size, &ARENA_QUICK_BIN_MAX.to_string()),
        abi::branch_hi("arena_free_large_bin"),
        abi::shift_right_immediate(&bin_class, &size, 4),
        abi::shift_left_immediate(&bin_class, &bin_class, 3),
        abi::add_registers(&bin_slot, ARENA_STATE_REGISTER, &bin_class),
        abi::load_u64(&bin_head, &bin_slot, ARENA_QUICK_BIN_BASE_OFFSET - 8),
        // Idempotency guard (allocator-03 parity, bug-266): if `ptr` is already the
        // bin head, this is an immediate double-free — pushing again would set
        // `ptr.next = ptr` (a self-cycle) and hand `ptr` back to the next two
        // allocations. Skip the push and return, matching `insert_already_free`.
        abi::compare_registers(&ptr, &bin_head),
        abi::branch_eq(double_free),
    ]);
    if let Some(slot) = &dbg_slot {
        emit_debug_arena_live_sub(
            ARENA_FREE_SYMBOL,
            slot,
            &size,
            "free_quick",
            &mut instructions,
            &mut vregs,
        );
    }
    instructions.extend([
        abi::store_u64(&bin_head, &ptr, 0),
        abi::store_u64(&size, &ptr, 8),
        abi::store_u64(&ptr, &bin_slot, ARENA_QUICK_BIN_BASE_OFFSET - 8),
        abi::branch("arena_free_scrub"),
        // A larger chunk (> ARENA_QUICK_BIN_MAX) parks on its large-block size
        // class (plan-25-A, bug-690): an O(1) head push, no address-ordered walk.
        // This is the master benchmark fix — routing large frees through
        // `arena_insert_free` grew the coalescing list without bound (every large
        // 1000-element list op frees ~40 KB), so both the insert here and every
        // later alloc walk went quadratic. A later large request of ANY size the
        // chunk fits can take it (see arena_alloc). Bin nodes reuse the FreeNode
        // {next, size} overlay. The chunk is still scrubbed below.
        abi::label("arena_free_large_bin"),
    ]);
    emit_large_bin_slot(
        &bin_slot,
        &size,
        &bin_class,
        &mask,
        &bin_head,
        "arena_free_large_class",
        &mut instructions,
    );
    instructions.extend([
        abi::load_u64(&bin_head, &bin_slot, ARENA_LARGE_BIN_BASE_OFFSET),
        // Idempotency guard (allocator-03 parity, bug-266): an immediate double-free
        // of a large chunk already at the bin head would self-cycle it; skip.
        abi::compare_registers(&ptr, &bin_head),
        abi::branch_eq(double_free),
    ]);
    if let Some(slot) = &dbg_slot {
        emit_debug_arena_live_sub(
            ARENA_FREE_SYMBOL,
            slot,
            &size,
            "free_large",
            &mut instructions,
            &mut vregs,
        );
    }
    instructions.extend([
        abi::store_u64(&bin_head, &ptr, 0),
        abi::store_u64(&size, &ptr, 8),
        abi::store_u64(&ptr, &bin_slot, ARENA_LARGE_BIN_BASE_OFFSET),
        // bug-690: credit the large-bin flush with the bytes just parked.
        abi::move_immediate(
            &bin_slot,
            "Integer",
            &ARENA_LARGE_FLUSH_CREDIT_OFFSET.to_string(),
        ),
        abi::add_registers(&bin_slot, ARENA_STATE_REGISTER, &bin_slot),
        abi::load_u64(&bin_head, &bin_slot, 0),
        abi::add_registers(&bin_head, &bin_head, &size),
        abi::store_u64(&bin_head, &bin_slot, 0),
        // … then scrub only [ptr+16, ptr+size), preserving the freshly written
        // node words. A 16-byte chunk is all node — nothing to scrub.
        abi::label("arena_free_scrub"),
        abi::compare_immediate(&size, &ARENA_MIN_CHUNK.to_string()),
        abi::branch_eq("arena_free_done"),
    ]);
    // plan-133-B: count the free-path scrub — one call over `size − 16` bytes. Both free
    // paths (quick bin and large bin) reach this label. The length goes through a fresh
    // vreg, so the counters never touch a physical argument register.
    if let Some(slot) = &dbg_slot {
        let scrub_len = vregs.next();
        instructions.push(abi::subtract_immediate(
            &scrub_len,
            &size,
            ARENA_MIN_CHUNK as usize,
        ));
        emit_debug_arena_bump(
            ARENA_FREE_SYMBOL,
            slot,
            COUNTER_FILL_FREE_CALLS,
            "fill_free_calls",
            &mut instructions,
            &mut vregs,
        );
        emit_debug_arena_add(
            ARENA_FREE_SYMBOL,
            slot,
            COUNTER_FILL_FREE_BYTES,
            &scrub_len,
            "fill_free_bytes",
            &mut instructions,
            &mut vregs,
        );
    }
    instructions.extend([
        abi::add_immediate(abi::c_arg(0), &ptr, ARENA_MIN_CHUNK as usize),
        abi::subtract_immediate(abi::c_arg(1), &size, ARENA_MIN_CHUNK as usize),
        abi::branch_link(ARENA_FILL_ZERO_SYMBOL),
        abi::label("arena_free_done"),
    ]);
    // plan-67-F: close the `mfb_free` span at the single exit. The helper returns
    // Nothing, so there is no result register to preserve across the perf `bl`.
    if perf {
        emit_perf_arena_call(
            "perf.end",
            ARENA_FREE_SYMBOL,
            PERF_NAME_MFB_FREE_SYMBOL,
            &mut instructions,
            &mut relocations,
        );
    }
    instructions.push(abi::return_());
    // plan-130-C: an immediate double-free skipped its push; count it and leave
    // through the shared exit.
    if let Some(slot) = &dbg_slot {
        instructions.push(abi::label("arena_free_double"));
        emit_debug_arena_bump(
            ARENA_FREE_SYMBOL,
            slot,
            COUNTER_DOUBLE_FREE_SKIPS,
            "double_free",
            &mut instructions,
            &mut vregs,
        );
        instructions.push(abi::branch("arena_free_done"));
    }
    finalize_vreg_helper(
        "runtime.arena_free",
        ARENA_FREE_SYMBOL,
        "Nothing",
        instructions,
        relocations,
    )
}
