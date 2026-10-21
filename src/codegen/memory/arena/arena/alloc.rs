//! plan-176-P: the arena allocator (`lower_arena_alloc`) and the SIMD list allocator. The
//! shared chunk/bin helpers, the fill and destroy helpers live in `memory/arena/arena/mod.rs`.

use super::*;

/// `perf` (plan-130-B): bracket the body with the `mfb_alloc` perf span — true exactly
/// when the `--debug` perf section is active for the module.
/// `debug_arena` (plan-130-C): count this helper's events in the `--debug` arena
/// registry — true exactly when the arena section is active for the module.
pub(crate) fn lower_arena_alloc(
    platform: &dyn CodegenPlatform,
    perf: bool,
    debug_arena: bool,
    // plan-179-C: charge each mapped block against the arena memory cap. The lock
    // calls need the platform's imports.
    arena_cap: bool,
    platform_imports: &std::collections::HashMap<String, String>,
) -> Result<CodeFunction, String> {
    // Vreg-allocated (plan-00-G Phase 2): the body names virtual registers and the
    // shared allocator places them per-ISA; `finalize_vreg_helper` runs the
    // allocator + `finalize_frame` (which builds the frame, saves the link
    // register because the grow path calls `arena_fill_zero`, and saves any
    // callee-saved registers the allocator used).
    //
    // Register contract (allocator-06): the standard runtime-helper one the
    // regalloc call-clobber model already assumes — all caller-saved integer
    // registers (`x0`–`x17`) are clobbered; callee-saved (`x19`–`x28`) are
    // preserved by the PCS frame. No caller holds a value in a physical register
    // across the call (audited tree-wide; every caller spills to stack slots or
    // vregs). The historical `x8/x11/x12/x13/x17` survivor reservation was
    // byte-identical-migration scaffolding and is gone.
    let not_15 = (!(ARENA_MIN_CHUNK - 1)).to_string();
    let mut vregs = Vregs::new();
    // Values that live across blocks: the normalized request, the walk cursor, and
    // the split geometry. `size`/`eff_align` are loop-carried (the grow path loops
    // back to the walk), so the allocator spills them across the grow call.
    let eff_align = vregs.next();
    let size = vregs.next();
    let cur = vregs.next();
    let prev = vregs.next();
    let cur_size = vregs.next();
    let aligned = vregs.next();
    let end_needed = vregs.next();
    let cur_end = vregs.next();
    // --- Validate alignment and normalize the request --------------------------
    let align_low = vregs.next();
    let align_pow2 = vregs.next();
    let not15 = vregs.next();
    let max_request = vregs.next();
    // Quick-bin fast path + flush-before-grow state (allocator-01).
    let flushed = vregs.next();
    let bin_class = vregs.next();
    let bin_slot = vregs.next();
    let bin_head = vregs.next();
    let bin_next = vregs.next();
    let bin_scan = vregs.next();
    let bin_scan_end = vregs.next();
    let bin_rem = vregs.next();
    // Segregated large-block size classes (plan-25-A, bug-690): a large request
    // takes the best fit from its class, else a chunk from a higher class, and
    // splits it, before falling to the first-fit walk.
    let lg_slot = vregs.next();
    let lg_link = vregs.next();
    let lg_cur = vregs.next();
    let lg_next = vregs.next();
    let lg_msize = vregs.next();
    let lg_best = vregs.next();
    let lg_best_size = vregs.next();
    let lg_budget = vregs.next();
    let lg_end = vregs.next();
    let lg_k = vregs.next();
    let lg_shift = vregs.next();
    let lg_sub = vregs.next();
    let mut relocations = vec![internal_branch(ARENA_ALLOC_SYMBOL, ARENA_FILL_ZERO_SYMBOL)];
    // plan-64 A1: the flush-before-grow path calls arena_flush_coalesce.
    relocations.push(internal_branch(
        ARENA_ALLOC_SYMBOL,
        ARENA_FLUSH_COALESCE_SYMBOL,
    ));
    // plan-133-C: a `--debug` grow records a memory-series sample.
    if debug_arena {
        relocations.push(internal_branch(
            ARENA_ALLOC_SYMBOL,
            DEBUG_ARENA_SAMPLE_SYMBOL,
        ));
    }
    let mut instructions = vec![
        abi::label("entry"),
        abi::compare_immediate(abi::c_arg(1), "0"),
        abi::branch_eq("arena_alloc_invalid"),
        abi::subtract_immediate(&align_low, abi::c_arg(1), 1),
        abi::and_registers(&align_pow2, abi::c_arg(1), &align_low),
        abi::compare_immediate(&align_pow2, "0"),
        abi::branch_ne("arena_alloc_invalid"),
        // eff align = max(align, 16)
        abi::move_register(&eff_align, abi::c_arg(1)),
        abi::compare_immediate(&eff_align, &ARENA_MIN_CHUNK.to_string()),
        abi::branch_lo("arena_alloc_align_min"),
        abi::branch("arena_alloc_align_ready"),
        abi::label("arena_alloc_align_min"),
        abi::move_immediate(&eff_align, "Integer", &ARENA_MIN_CHUNK.to_string()),
        abi::label("arena_alloc_align_ready"),
        // normalized size = round_up(max(size, 1), 16)
        abi::move_register(&size, abi::c_arg(0)),
    ];
    // plan-67-F: open the `mfb_alloc` span now that size/eff_align are captured in
    // vregs (spilled across the perf `bl`); the body below re-derives ARG[0]/ARG[1].
    if perf {
        emit_perf_arena_call(
            "perf.start",
            ARENA_ALLOC_SYMBOL,
            PERF_NAME_MFB_ALLOC_SYMBOL,
            &mut instructions,
            &mut relocations,
        );
    }
    let mut dbg_slot: Option<VirtualRegister> = None;
    instructions.extend([
        // Reject a raw request within ARENA_MIN_CHUNK of u64::MAX before the
        // +15 granule round-up (allocator-02, audit-1 MEM-07): without this
        // bound the round-up wraps and the allocation succeeds *small*, turning
        // every unchecked caller-side size computation into a heap OOB write.
        // No request this large can ever be satisfied, so rejecting it as
        // invalid loses nothing.
        abi::move_immediate(
            &max_request,
            "Integer",
            &(u64::MAX - ARENA_MIN_CHUNK).to_string(),
        ),
        abi::compare_registers(&size, &max_request),
        abi::branch_hi("arena_alloc_invalid"),
        abi::compare_immediate(&size, "0"),
        abi::branch_ne("arena_alloc_size_nonzero"),
        abi::move_immediate(&size, "Integer", "1"),
        abi::label("arena_alloc_size_nonzero"),
        abi::add_immediate(&size, &size, (ARENA_MIN_CHUNK - 1) as usize),
        abi::move_immediate(&not15, "Integer", &not_15),
        abi::and_registers(&size, &size, &not15),
    ]);
    // plan-130-C: find this arena's registry slot once, then count the call and its
    // normalized size. Invalid requests branched away above and are not counted.
    if debug_arena {
        let slot = vregs.next();
        emit_debug_arena_slot(
            ARENA_ALLOC_SYMBOL,
            &slot,
            &mut instructions,
            &mut relocations,
            &mut vregs,
        );
        emit_debug_arena_bump(
            ARENA_ALLOC_SYMBOL,
            &slot,
            COUNTER_ALLOC_CALLS,
            "alloc_calls",
            &mut instructions,
            &mut vregs,
        );
        emit_debug_arena_add(
            ARENA_ALLOC_SYMBOL,
            &slot,
            COUNTER_ALLOC_BYTES,
            &size,
            "alloc_bytes",
            &mut instructions,
            &mut vregs,
        );
        dbg_slot = Some(slot);
    }
    instructions.extend([
        // --- Quick-bin pop (allocator-01) ---------------------------------------
        // An exact-class bin hit serves the request in O(1): both sides
        // normalize identically (≥16, 16-multiple) and every chunk ever handed
        // out is 16-aligned, so any bin node satisfies any eff_align ≤ 16
        // request of its class. eff_align > 16 requests bypass bins entirely.
        // `flushed` arms the one flush-before-grow retry below.
        abi::move_immediate(&flushed, "Integer", "0"),
        abi::compare_immediate(&eff_align, &ARENA_MIN_CHUNK.to_string()),
        abi::branch_hi("arena_alloc_walk"),
        abi::compare_immediate(&size, &ARENA_QUICK_BIN_MAX.to_string()),
        abi::branch_hi("arena_alloc_large_bin"),
        abi::shift_right_immediate(&bin_class, &size, 4),
        abi::shift_left_immediate(&bin_class, &bin_class, 3),
        abi::add_registers(&bin_slot, ARENA_STATE_REGISTER, &bin_class),
        abi::load_u64(&bin_head, &bin_slot, ARENA_QUICK_BIN_BASE_OFFSET - 8),
        abi::compare_immediate(&bin_head, "0"),
        abi::branch_eq("arena_alloc_bin_scan"),
        abi::load_u64(&bin_next, &bin_head, 0),
        abi::store_u64(&bin_next, &bin_slot, ARENA_QUICK_BIN_BASE_OFFSET - 8),
    ]);
    emit_debug_alloc_hit(
        &dbg_slot,
        COUNTER_HIT_QUICK_BIN,
        &size,
        "hit_quick",
        &mut instructions,
        &mut vregs,
    );
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", RESULT_OK_TAG),
        abi::move_register(abi::mfb_return(1), &bin_head),
        abi::branch("arena_alloc_ret"),
        // Exact bin empty (allocator-01): bump-serve from the designated
        // victim (DV) — one active carve chunk held in the arena state.
        // Splitting parked bin inventory on every miss shaves it into
        // sub-class crumbs that nothing requests (measured on
        // benchmark/bignum-modexp: 21-30% hit rates, tens of thousands of
        // stranded fragments, flush storms); concentrating all small-miss
        // carving in one chunk keeps parked inventory intact so the exact-bin
        // hit rate climbs toward 100% under churn (dlmalloc's dv). The DV is
        // 16-aligned by construction and eff_align ≤ 16 on this path.
        abi::label("arena_alloc_bin_scan"),
        abi::load_u64(&bin_rem, ARENA_STATE_REGISTER, ARENA_CARVE_SIZE_OFFSET),
        abi::compare_registers(&bin_rem, &size),
        abi::branch_lo("arena_alloc_dv_renew"),
        abi::load_u64(&bin_head, ARENA_STATE_REGISTER, ARENA_CARVE_PTR_OFFSET),
        abi::subtract_registers(&bin_rem, &bin_rem, &size),
        abi::add_registers(&bin_next, &bin_head, &size),
        abi::store_u64(&bin_next, ARENA_STATE_REGISTER, ARENA_CARVE_PTR_OFFSET),
        abi::store_u64(&bin_rem, ARENA_STATE_REGISTER, ARENA_CARVE_SIZE_OFFSET),
    ]);
    emit_debug_alloc_hit(
        &dbg_slot,
        COUNTER_HIT_CARVE,
        &size,
        "hit_carve",
        &mut instructions,
        &mut vregs,
    );
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", RESULT_OK_TAG),
        abi::move_register(abi::mfb_return(1), &bin_head),
        abi::branch("arena_alloc_ret"),
        // DV exhausted: retire its remnant (park ≤ QUICK_BIN_MAX in its exact
        // bin; a larger remnant joins the coalescing list) and acquire a new
        // DV — largest parked bin first (top-down scan, so the DV lives long),
        // then the walk (which hands over a WHOLE chunk — no split), then the
        // flush retry, then a fresh block from the grow path.
        abi::label("arena_alloc_dv_renew"),
        abi::compare_immediate(&bin_rem, "0"),
        abi::branch_eq("arena_alloc_dv_scan"),
        abi::load_u64(&bin_head, ARENA_STATE_REGISTER, ARENA_CARVE_PTR_OFFSET),
        abi::compare_immediate(&bin_rem, &ARENA_QUICK_BIN_MAX.to_string()),
        abi::branch_hi("arena_alloc_dv_retire_list"),
        abi::shift_right_immediate(&bin_scan, &bin_rem, 4),
        abi::shift_left_immediate(&bin_scan, &bin_scan, 3),
        abi::add_registers(&bin_scan, ARENA_STATE_REGISTER, &bin_scan),
        abi::load_u64(&bin_next, &bin_scan, ARENA_QUICK_BIN_BASE_OFFSET - 8),
        abi::store_u64(&bin_next, &bin_head, 0),
        abi::store_u64(&bin_rem, &bin_head, 8),
        abi::store_u64(&bin_head, &bin_scan, ARENA_QUICK_BIN_BASE_OFFSET - 8),
        abi::branch("arena_alloc_dv_cleared"),
        abi::label("arena_alloc_dv_retire_list"),
        // Rare: a large remnant coalesces back into the list. `size` and
        // `eff_align` are loop-carried vregs, spilled across the call.
        abi::move_register(abi::c_arg(0), &bin_head),
        abi::move_register(abi::c_arg(1), &bin_rem),
        abi::branch_link(ARENA_INSERT_FREE_SYMBOL),
        abi::label("arena_alloc_dv_cleared"),
        abi::store_u64(abi::ZERO, ARENA_STATE_REGISTER, ARENA_CARVE_SIZE_OFFSET),
        // Acquire: largest parked bin ≥ this request becomes the new DV.
        abi::label("arena_alloc_dv_scan"),
        abi::add_immediate(
            &bin_scan_end,
            ARENA_STATE_REGISTER,
            ARENA_QUICK_BIN_BASE_OFFSET - 8,
        ),
        abi::add_registers(&bin_scan_end, &bin_scan_end, &bin_class),
        abi::add_immediate(
            &bin_scan,
            ARENA_STATE_REGISTER,
            ARENA_QUICK_BIN_BASE_OFFSET + ARENA_QUICK_BIN_COUNT * 8,
        ),
        abi::label("arena_alloc_dv_scan_loop"),
        abi::subtract_immediate(&bin_scan, &bin_scan, 8),
        abi::compare_registers(&bin_scan, &bin_scan_end),
        abi::branch_lo("arena_alloc_walk"),
        abi::load_u64(&bin_head, &bin_scan, 0),
        abi::compare_immediate(&bin_head, "0"),
        abi::branch_eq("arena_alloc_dv_scan_loop"),
        abi::load_u64(&bin_next, &bin_head, 0),
        abi::store_u64(&bin_next, &bin_scan, 0),
        abi::load_u64(&bin_rem, &bin_head, 8),
    ]);
    // plan-130-C: a renewed designated victim serves this request — counted here,
    // before the shared serve label, because the walk's whole-chunk take also jumps
    // to that label and is already counted as a walk hit.
    emit_debug_alloc_hit(
        &dbg_slot,
        COUNTER_HIT_CARVE,
        &size,
        "hit_carve_renew",
        &mut instructions,
        &mut vregs,
    );
    instructions.extend([
        abi::label("arena_alloc_dv_serve"),
        // Serve `size` from the new DV chunk (bin_head/bin_rem) and store the
        // shrunken DV.
        abi::subtract_registers(&bin_rem, &bin_rem, &size),
        abi::add_registers(&bin_next, &bin_head, &size),
        abi::store_u64(&bin_next, ARENA_STATE_REGISTER, ARENA_CARVE_PTR_OFFSET),
        abi::store_u64(&bin_rem, ARENA_STATE_REGISTER, ARENA_CARVE_SIZE_OFFSET),
        abi::move_immediate(abi::return_register(), "Integer", RESULT_OK_TAG),
        abi::move_register(abi::mfb_return(1), &bin_head),
        abi::branch("arena_alloc_ret"),
        // --- Segregated large-block size classes (plan-25-A, bug-690) ----------
        // A large request (size > QUICK_BIN_MAX, eff_align ≤ 16 — larger aligns
        // branched straight to the walk above) is served from the large bins in
        // O(1): the best fit among the first ARENA_LARGE_BIN_SCAN_LIMIT nodes of its
        // own size class (an exact size is taken at once — repetitive large-list
        // churn pops the chunk it just freed, as the old exact-size bins did), else
        // the head of the next non-empty HIGHER class, every chunk of which is
        // larger than any size in this one. The chunk is split and the remainder
        // re-parked (≤ QUICK_BIN_MAX → its quick bin, larger → its size class).
        // Diverting large frees off the address-ordered list (see arena_free)
        // keeps that list short, so a miss's fall-through walk stays cheap under
        // heavy large-list churn — the benchmark's ~30× inflation was this list
        // growing without bound (plan-25-A). The exact-size-only bins this
        // replaced reused a freed block only for a request of exactly its size, so
        // a program whose large sizes never repeat mapped without bound (bug-690).
        abi::label("arena_alloc_large_bin"),
    ]);
    emit_large_bin_slot(
        &lg_slot,
        &size,
        &lg_k,
        &lg_shift,
        &lg_sub,
        "arena_alloc_large_class",
        &mut instructions,
    );
    instructions.extend([
        // lg_link tracks the address of the word that points at lg_cur (the bin
        // head cell first, then each visited node's `next` at +0), so the chosen
        // node unlinks in O(1) whether it is the head or mid-list. lg_best is the
        // link word of the smallest fitting node seen so far (0 = none yet).
        abi::add_immediate(&lg_link, &lg_slot, ARENA_LARGE_BIN_BASE_OFFSET),
        abi::load_u64(&lg_cur, &lg_link, 0),
        abi::move_immediate(&lg_best, "Integer", "0"),
        abi::move_immediate(
            &lg_budget,
            "Integer",
            &ARENA_LARGE_BIN_SCAN_LIMIT.to_string(),
        ),
        abi::label("arena_alloc_large_scan"),
        abi::compare_immediate(&lg_cur, "0"),
        abi::branch_eq("arena_alloc_large_scan_done"),
        abi::load_u64(&lg_msize, &lg_cur, 8),
        abi::compare_registers(&lg_msize, &size),
        abi::branch_lo("arena_alloc_large_scan_next"),
        abi::compare_registers(&lg_msize, &size),
        abi::branch_eq("arena_alloc_large_take"),
        abi::compare_immediate(&lg_best, "0"),
        abi::branch_eq("arena_alloc_large_scan_best"),
        abi::compare_registers(&lg_msize, &lg_best_size),
        abi::branch_hi("arena_alloc_large_scan_next"),
        abi::label("arena_alloc_large_scan_best"),
        abi::move_register(&lg_best, &lg_link),
        abi::move_register(&lg_best_size, &lg_msize),
        abi::label("arena_alloc_large_scan_next"),
        abi::subtract_immediate(&lg_budget, &lg_budget, 1),
        abi::compare_immediate(&lg_budget, "0"),
        abi::branch_eq("arena_alloc_large_scan_done"),
        abi::move_register(&lg_link, &lg_cur),
        abi::load_u64(&lg_cur, &lg_cur, 0),
        abi::branch("arena_alloc_large_scan"),
        abi::label("arena_alloc_large_scan_done"),
        abi::compare_immediate(&lg_best, "0"),
        abi::branch_eq("arena_alloc_large_up"),
        abi::move_register(&lg_link, &lg_best),
        abi::load_u64(&lg_cur, &lg_link, 0),
        abi::branch("arena_alloc_large_take"),
        // Nothing in this class fits: the lowest non-empty higher class.
        abi::label("arena_alloc_large_up"),
        abi::add_immediate(
            &lg_end,
            ARENA_STATE_REGISTER,
            ARENA_LARGE_BIN_BASE_OFFSET + ARENA_LARGE_BIN_COUNT * 8,
        ),
        abi::add_immediate(&lg_link, &lg_slot, ARENA_LARGE_BIN_BASE_OFFSET),
        abi::label("arena_alloc_large_up_loop"),
        abi::add_immediate(&lg_link, &lg_link, 8),
        abi::compare_registers(&lg_link, &lg_end),
        abi::branch_eq("arena_alloc_walk"),
        abi::load_u64(&lg_cur, &lg_link, 0),
        abi::compare_immediate(&lg_cur, "0"),
        abi::branch_eq("arena_alloc_large_up_loop"),
        // Take lg_cur (linked from lg_link), split off `size`, re-park the rest.
        abi::label("arena_alloc_large_take"),
        abi::load_u64(&lg_next, &lg_cur, 0),
        abi::store_u64(&lg_next, &lg_link, 0),
        abi::load_u64(&lg_msize, &lg_cur, 8),
        abi::subtract_registers(&lg_msize, &lg_msize, &size),
        abi::compare_immediate(&lg_msize, "0"),
        abi::branch_eq("arena_alloc_large_hit"),
        abi::add_registers(&lg_next, &lg_cur, &size),
    ]);
    emit_park_chunk(
        &lg_next,
        &lg_msize,
        &lg_slot,
        &bin_head,
        &lg_k,
        &lg_shift,
        &lg_sub,
        "arena_alloc_large_rem",
        &mut instructions,
    );
    instructions.extend([abi::label("arena_alloc_large_hit")]);
    emit_debug_alloc_hit(
        &dbg_slot,
        COUNTER_HIT_LARGE_BIN,
        &size,
        "hit_large",
        &mut instructions,
        &mut vregs,
    );
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", RESULT_OK_TAG),
        abi::move_register(abi::mfb_return(1), &lg_cur),
        abi::branch("arena_alloc_ret"),
        // --- First-fit walk over the address-ordered free-list -----------------
        abi::label("arena_alloc_walk"),
        abi::load_u64(&cur, ARENA_STATE_REGISTER, ARENA_FREE_LIST_HEAD_OFFSET),
        abi::move_immediate(&prev, "Integer", "0"),
        abi::label("arena_alloc_walk_loop"),
        abi::compare_immediate(&cur, "0"),
        abi::branch_eq("arena_alloc_grow"),
        abi::load_u64(&cur_size, &cur, 8), // cur_size
    ]);
    let align_mask = vregs.next();
    let align_notmask = vregs.next();
    instructions.extend([
        abi::subtract_immediate(&align_mask, &eff_align, 1), // align mask
        abi::add_registers(&aligned, &cur, &align_mask),
        abi::compare_registers(&aligned, &cur),
        abi::branch_lo("arena_alloc_walk_next"), // align overflow → skip
        abi::bitwise_not(&align_notmask, &align_mask),
        abi::and_registers(&aligned, &aligned, &align_notmask), // aligned
        abi::add_registers(&end_needed, &aligned, &size),       // end_needed
        abi::compare_registers(&end_needed, &aligned),
        abi::branch_lo("arena_alloc_walk_next"), // size overflow → skip
        abi::add_registers(&cur_end, &cur, &cur_size), // cur_end
        abi::compare_registers(&end_needed, &cur_end),
        abi::branch_hi("arena_alloc_walk_next"), // doesn't fit → next
    ]);
    // plan-130-C: the walk found a fit — counted at the walk, not at `found`, which
    // the grow path also enters (and counts as a grow).
    emit_debug_alloc_hit(
        &dbg_slot,
        COUNTER_HIT_WALK,
        &size,
        "hit_walk",
        &mut instructions,
        &mut vregs,
    );
    instructions.extend([
        abi::branch("arena_alloc_found"),
        abi::label("arena_alloc_walk_next"),
        abi::move_register(&prev, &cur),
        abi::load_u64(&cur, &cur, 0),
        abi::branch("arena_alloc_walk_loop"),
    ]);
    // --- Found: split the chosen chunk -------------------------------------
    let next_node = vregs.next();
    let front_pad = vregs.next();
    let tail_size = vregs.next();
    let link = vregs.next();
    instructions.extend([
        // Split the chosen chunk. Remainders ≤ ARENA_QUICK_BIN_MAX go to their
        // exact-size bin instead of the list (allocator-01): a walk-split's
        // front/tail crumbs would otherwise accumulate at the head of the
        // address-ordered list — never allocated, never coalesced — and every
        // later walk would pay for them (measured: tens of thousands of 32–80
        // byte crumbs, linear growth, quadratic total). Binned remainders stay
        // poppable and re-coalesce at the next flush. The list link therefore
        // chains only the pieces that stay on the list, in address order
        // (cur < end_needed < next_node).
        abi::label("arena_alloc_found"),
        abi::load_u64(&next_node, &cur, 0), // next
        // A small request takes the WHOLE chunk as the new designated victim
        // (the old DV was retired before reaching the walk): unlink it and
        // bump-serve. Splitting here would shave parked inventory into
        // crumbs; big requests (or align > 16) keep the four-case split.
        abi::compare_immediate(&eff_align, &ARENA_MIN_CHUNK.to_string()),
        abi::branch_hi("arena_alloc_found_split"),
        abi::compare_immediate(&size, &ARENA_QUICK_BIN_MAX.to_string()),
        abi::branch_hi("arena_alloc_found_split"),
        abi::compare_immediate(&prev, "0"),
        abi::branch_eq("arena_alloc_found_dv_head"),
        abi::store_u64(&next_node, &prev, 0),
        abi::branch("arena_alloc_found_dv_take"),
        abi::label("arena_alloc_found_dv_head"),
        abi::store_u64(
            &next_node,
            ARENA_STATE_REGISTER,
            ARENA_FREE_LIST_HEAD_OFFSET,
        ),
        abi::label("arena_alloc_found_dv_take"),
        abi::move_register(&bin_head, &cur),
        abi::subtract_registers(&bin_rem, &cur_end, &cur),
        abi::branch("arena_alloc_dv_serve"),
        abi::label("arena_alloc_found_split"),
        abi::subtract_registers(&front_pad, &aligned, &cur), // front_pad
        abi::subtract_registers(&tail_size, &cur_end, &end_needed), // tail_size
        // Tail remainder first (higher address): bin it, list it, or nothing.
        abi::move_register(&link, &next_node),
        abi::compare_immediate(&tail_size, "0"),
        abi::branch_eq("arena_alloc_front"),
        abi::compare_immediate(&tail_size, &ARENA_QUICK_BIN_MAX.to_string()),
        abi::branch_hi("arena_alloc_tail_list"),
        abi::shift_right_immediate(&bin_class, &tail_size, 4),
        abi::shift_left_immediate(&bin_class, &bin_class, 3),
        abi::add_registers(&bin_slot, ARENA_STATE_REGISTER, &bin_class),
        abi::load_u64(&bin_head, &bin_slot, ARENA_QUICK_BIN_BASE_OFFSET - 8),
        abi::store_u64(&bin_head, &end_needed, 0),
        abi::store_u64(&tail_size, &end_needed, 8),
        abi::store_u64(&end_needed, &bin_slot, ARENA_QUICK_BIN_BASE_OFFSET - 8),
        abi::branch("arena_alloc_front"),
        abi::label("arena_alloc_tail_list"),
        abi::store_u64(&next_node, &end_needed, 0),
        abi::store_u64(&tail_size, &end_needed, 8),
        abi::move_register(&link, &end_needed),
        // Front remainder (lower address): bin it, list it, or nothing.
        abi::label("arena_alloc_front"),
        abi::compare_immediate(&front_pad, "0"),
        abi::branch_eq("arena_alloc_set_prev_link"),
        abi::compare_immediate(&front_pad, &ARENA_QUICK_BIN_MAX.to_string()),
        abi::branch_hi("arena_alloc_front_list"),
        abi::shift_right_immediate(&bin_class, &front_pad, 4),
        abi::shift_left_immediate(&bin_class, &bin_class, 3),
        abi::add_registers(&bin_slot, ARENA_STATE_REGISTER, &bin_class),
        abi::load_u64(&bin_head, &bin_slot, ARENA_QUICK_BIN_BASE_OFFSET - 8),
        abi::store_u64(&bin_head, &cur, 0),
        abi::store_u64(&front_pad, &cur, 8),
        abi::store_u64(&cur, &bin_slot, ARENA_QUICK_BIN_BASE_OFFSET - 8),
        abi::branch("arena_alloc_set_prev_link"),
        abi::label("arena_alloc_front_list"),
        abi::store_u64(&link, &cur, 0), // cur.next → tail node or next
        abi::store_u64(&front_pad, &cur, 8),
        abi::move_register(&link, &cur),
        abi::label("arena_alloc_set_prev_link"),
        abi::compare_immediate(&prev, "0"),
        abi::branch_eq("arena_alloc_set_head"),
        abi::store_u64(&link, &prev, 0),
        abi::branch("arena_alloc_done"),
        abi::label("arena_alloc_set_head"),
        abi::store_u64(&link, ARENA_STATE_REGISTER, ARENA_FREE_LIST_HEAD_OFFSET),
        abi::label("arena_alloc_done"),
        abi::move_immediate(abi::return_register(), "Integer", RESULT_OK_TAG),
        abi::move_register(abi::mfb_return(1), &aligned),
        abi::branch("arena_alloc_ret"),
    ]);
    // --- Grow: map a new block and carve the request from it ----------------
    let map_size = vregs.next();
    let default_block = vregs.next();
    let saved_size = vregs.next();
    let page_mask = vregs.next();
    instructions.extend([
        abi::label("arena_alloc_grow"),
        abi::compare_immediate(&eff_align, &ARENA_MIN_CHUNK.to_string()),
        abi::branch_hi("arena_alloc_grow_big"),
        abi::compare_immediate(&size, &ARENA_QUICK_BIN_MAX.to_string()),
        abi::branch_hi("arena_alloc_grow_big"),
        // bug-690: before a SMALL request maps, it takes the smallest parked large
        // chunk as its new designated victim — whole, exactly as the walk hands one
        // over (the old DV was retired on the way here). Without this, memory freed
        // in large blocks was never available to a small request at all.
        abi::add_immediate(
            &lg_end,
            ARENA_STATE_REGISTER,
            ARENA_LARGE_BIN_BASE_OFFSET + ARENA_LARGE_BIN_COUNT * 8,
        ),
        abi::add_immediate(&lg_link, ARENA_STATE_REGISTER, ARENA_LARGE_BIN_BASE_OFFSET),
        abi::label("arena_alloc_small_large_loop"),
        abi::load_u64(&lg_cur, &lg_link, 0),
        abi::compare_immediate(&lg_cur, "0"),
        abi::branch_ne("arena_alloc_small_large_take"),
        abi::add_immediate(&lg_link, &lg_link, 8),
        abi::compare_registers(&lg_link, &lg_end),
        abi::branch_ne("arena_alloc_small_large_loop"),
        // Flush-before-grow (allocator-01) for a SMALL request: it only reaches
        // here after the exact bin, the larger-bin scan, the walk and the large
        // bins all failed, and coalescing adjacent parked chunks genuinely can
        // produce a fit. The `flushed` flag arms exactly one retry — a second
        // miss falls through to the map below.
        abi::compare_immediate(&flushed, "0"),
        abi::branch_ne("arena_alloc_grow_map"),
        // plan-64 A1: skip the flush when the address-ordered list is empty. Then
        // the only free chunks are the parked SMALL bins, whose coalescing merges
        // exact-size chunks a pure-small workload is actively recycling (datetime's
        // scattered records) — destroying reuse and re-triggering the flush every
        // grow, for no fit a mapped block would not serve as cheaply. A non-empty
        // list means there ARE larger chunks worth merging small ones into.
        abi::load_u64(&map_size, ARENA_STATE_REGISTER, ARENA_FREE_LIST_HEAD_OFFSET),
        abi::compare_immediate(&map_size, "0"),
        abi::branch_eq("arena_alloc_grow_map"),
        abi::move_immediate(&flushed, "Integer", "1"),
        // plan-64 A1: single-pass sort+coalesce (gather bins+list → merge sort by
        // address → coalesce adjacent → re-park). Replaces the old per-chunk
        // `arena_insert_free` drain + re-park sweep, whose O(list)-per-chunk insert
        // made a flush of M parked chunks O(M²) (the arena mixed-transient-churn
        // quadratic). `size`/`eff_align` are loop-carried across the call; the
        // clobber model spills them. The retry re-enters at the victim renewal,
        // which sees every re-parked chunk (and comes back here for a large one).
        abi::branch_link(ARENA_FLUSH_COALESCE_SYMBOL),
        abi::branch("arena_alloc_dv_scan"),
        // The victim is at most ARENA_LARGE_DV_MAX: a bigger chunk is split and
        // its tail re-parked, so one small request does not lock a whole large
        // block away from the large requests that freed it.
        abi::label("arena_alloc_small_large_take"),
        abi::load_u64(&lg_next, &lg_cur, 0),
        abi::store_u64(&lg_next, &lg_link, 0),
        abi::load_u64(&bin_rem, &lg_cur, 8),
        abi::move_immediate(&lg_msize, "Integer", &ARENA_LARGE_DV_MAX.to_string()),
        abi::compare_registers(&bin_rem, &lg_msize),
        abi::branch_ls("arena_alloc_small_large_dv"),
        abi::subtract_registers(&lg_msize, &bin_rem, &lg_msize),
        abi::move_immediate(&bin_rem, "Integer", &ARENA_LARGE_DV_MAX.to_string()),
        abi::add_registers(&lg_next, &lg_cur, &bin_rem),
    ]);
    emit_park_chunk(
        &lg_next,
        &lg_msize,
        &lg_slot,
        &bin_scan,
        &lg_k,
        &lg_shift,
        &lg_sub,
        "arena_alloc_small_large_rem",
        &mut instructions,
    );
    instructions.extend([
        abi::label("arena_alloc_small_large_dv"),
        abi::move_register(&bin_head, &lg_cur),
    ]);
    emit_debug_alloc_hit(
        &dbg_slot,
        COUNTER_HIT_LARGE_BIN,
        &size,
        "hit_large_dv",
        &mut instructions,
        &mut vregs,
    );
    instructions.extend([
        abi::branch("arena_alloc_dv_serve"),
        // A BIG request (> QUICK_BIN_MAX, eff_align ≤ 16) that found no fit in the
        // large bins or on the list flushes once before it maps (bug-690): its own
        // frees parked in size classes never merge otherwise, so a program whose
        // large sizes vary splits its free memory into pieces too small for the
        // next request and maps without bound. The flush is gated on the large-bin
        // credit (`ARENA_LARGE_FLUSH_CREDIT_OFFSET`): it runs only once more large
        // bytes were freed since the last flush than that flush left parked, which
        // keeps it amortized O(1) per freed byte — plan-64 A1 measured an ungated
        // big-request flush draining huge parked inventories for nothing. The
        // retry re-enters the large-bin search, where the coalesced runs now sit.
        // An align > 16 request never uses the bins, so it grows directly.
        abi::label("arena_alloc_grow_big"),
        abi::compare_immediate(&flushed, "0"),
        abi::branch_ne("arena_alloc_grow_map"),
        abi::compare_immediate(&eff_align, &ARENA_MIN_CHUNK.to_string()),
        abi::branch_hi("arena_alloc_grow_map"),
        abi::move_immediate(
            &map_size,
            "Integer",
            &ARENA_LARGE_FLUSH_CREDIT_OFFSET.to_string(),
        ),
        abi::add_registers(&map_size, ARENA_STATE_REGISTER, &map_size),
        abi::load_u64(&map_size, &map_size, 0),
        abi::compare_immediate(&map_size, "0"),
        abi::branch_le("arena_alloc_grow_map"),
        abi::move_immediate(&flushed, "Integer", "1"),
        abi::branch_link(ARENA_FLUSH_COALESCE_SYMBOL),
        abi::branch("arena_alloc_large_bin"),
        abi::label("arena_alloc_grow_map"),
        abi::add_registers(&map_size, &size, &eff_align),
        abi::compare_registers(&map_size, &size),
        abi::branch_lo("arena_alloc_oom"),
        // bug-690: a big request that missed while a chunk of its own size class
        // or the one below is parked is climbing past what it freed — the next one
        // is usually a little larger again (a scene or a copy whose length creeps
        // up frame by frame). Mapped blocks never merge with each other, so an
        // exact-size block would make every step of that climb map afresh: the
        // arena would grow by the sum of the steps, not by the largest. Map a
        // quarter more: the slack parks in the large bins, and the request's own
        // free re-merges with it into a run that the next few, slightly larger,
        // requests fit, so a climb maps a geometric series. A request with nothing
        // near its size parked (a program's first big allocation, or a list
        // doubling past its old buffer) maps exactly. `lg_slot` is the request's
        // class slot, set by the large-bin search every such request ran.
        abi::compare_immediate(&eff_align, &ARENA_MIN_CHUNK.to_string()),
        abi::branch_hi("arena_alloc_grow_map_sized"),
        abi::compare_immediate(&size, &ARENA_QUICK_BIN_MAX.to_string()),
        abi::branch_ls("arena_alloc_grow_map_sized"),
        abi::load_u64(&saved_size, &lg_slot, ARENA_LARGE_BIN_BASE_OFFSET),
        abi::compare_immediate(&saved_size, "0"),
        abi::branch_ne("arena_alloc_grow_map_slack"),
        abi::compare_registers(&lg_slot, ARENA_STATE_REGISTER),
        abi::branch_eq("arena_alloc_grow_map_sized"), // class 0 has no class below
        abi::load_u64(&saved_size, &lg_slot, ARENA_LARGE_BIN_BASE_OFFSET - 8),
        abi::compare_immediate(&saved_size, "0"),
        abi::branch_eq("arena_alloc_grow_map_sized"),
        abi::label("arena_alloc_grow_map_slack"),
        abi::shift_right_immediate(&saved_size, &size, 2),
        abi::add_registers(&map_size, &map_size, &saved_size),
        abi::compare_registers(&map_size, &saved_size),
        abi::branch_lo("arena_alloc_oom"),
        abi::label("arena_alloc_grow_map_sized"),
        abi::add_immediate(&map_size, &map_size, ARENA_BLOCK_HEADER_SIZE),
        // Carry-check the header add (allocator-02): a wrapped map_size would
        // round up to the default block size, the block could never satisfy
        // the huge request, and the walk-then-grow loop would mmap 4 KiB
        // blocks forever. A wrapped value is < ARENA_BLOCK_HEADER_SIZE while
        // any legitimate map_size is >= 1 + 16 + 32.
        abi::compare_immediate(&map_size, &ARENA_BLOCK_HEADER_SIZE.to_string()),
        abi::branch_lo("arena_alloc_oom"),
        abi::move_immediate(
            &default_block,
            "Integer",
            &ARENA_DEFAULT_BLOCK_SIZE.to_string(),
        ),
        abi::compare_registers(&map_size, &default_block),
        abi::branch_hi("arena_alloc_normal_block"),
        abi::move_immediate(&map_size, "Integer", &ARENA_DEFAULT_BLOCK_SIZE.to_string()),
        abi::branch("arena_alloc_map_size_ready"),
        abi::label("arena_alloc_normal_block"),
        abi::move_register(&saved_size, &map_size),
        abi::add_immediate(&map_size, &map_size, 4095),
        abi::compare_registers(&map_size, &saved_size),
        abi::branch_lo("arena_alloc_oom"),
        abi::move_immediate(&page_mask, "Integer", &(!4095_u64).to_string()),
        abi::and_registers(&map_size, &map_size, &page_mask),
        abi::label("arena_alloc_map_size_ready"),
    ]);
    // plan-179-C: charge the block to the arena memory cap before mapping it; a
    // charge past the limit is `ErrOutOfMemory` without a map.
    let cap_charged = vregs.next();
    if arena_cap {
        crate::codegen::runtime::arena_cap::emit_map_charge(
            &mut crate::codegen::runtime::arena_cap::CapSite {
                symbol: ARENA_ALLOC_SYMBOL,
                platform,
                platform_imports,
                instructions: &mut instructions,
                relocations: &mut relocations,
            },
            &map_size,
            &cap_charged,
            "arena_alloc_oom",
            &mut vregs,
        )?;
    }
    // mmap `map_size` bytes; the result is left in the return register. `map_size`
    // is live across the syscall (read again below for the block header), so the
    // allocator keeps it in a callee-saved register or spills it.
    platform.emit_arena_map(map_size.into(), &mut instructions)?;
    let prev_block = vregs.next();
    let usable = vregs.next();
    let ubase = vregs.next();
    let ins_cur = vregs.next();
    let ins_prev = vregs.next();
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ge("arena_alloc_mapped"),
    ]);
    // A failed map gives its charge back.
    if arena_cap {
        crate::codegen::runtime::arena_cap::emit_map_refund(
            &mut crate::codegen::runtime::arena_cap::CapSite {
                symbol: ARENA_ALLOC_SYMBOL,
                platform,
                platform_imports,
                instructions: &mut instructions,
                relocations: &mut relocations,
            },
            &map_size,
            &cap_charged,
            &mut vregs,
        )?;
    }
    instructions.extend([
        abi::branch("arena_alloc_oom"),
        abi::label("arena_alloc_mapped"),
    ]);
    // plan-130-C: a fresh block serves this request — one grow, one map of
    // `map_size` bytes. The mapped address stays in the return register; the
    // counters touch only fresh vregs.
    if let Some(slot) = &dbg_slot {
        emit_debug_arena_bump(
            ARENA_ALLOC_SYMBOL,
            slot,
            COUNTER_MAPS,
            "maps",
            &mut instructions,
            &mut vregs,
        );
        emit_debug_arena_add(
            ARENA_ALLOC_SYMBOL,
            slot,
            COUNTER_MAPPED_BYTES,
            &map_size,
            "mapped_bytes",
            &mut instructions,
            &mut vregs,
        );
    }
    emit_debug_alloc_hit(
        &dbg_slot,
        COUNTER_GROW,
        &size,
        "grow",
        &mut instructions,
        &mut vregs,
    );
    instructions.extend([
        // Write the block header (prevBlock, blockSize, usableCapacity, bumpOffset)
        // and chain it. bumpOffset is vestigial under the free-list but kept zero
        // so the documented block layout is unchanged.
        abi::load_u64(&prev_block, ARENA_STATE_REGISTER, 0),
        abi::store_u64(&prev_block, abi::return_register(), 0),
        abi::store_u64(&map_size, abi::return_register(), 8),
        abi::subtract_immediate(&usable, &map_size, ARENA_BLOCK_HEADER_SIZE),
        abi::store_u64(&usable, abi::return_register(), 16),
        abi::store_u64(abi::ZERO, abi::return_register(), 24),
        abi::store_u64(abi::return_register(), ARENA_STATE_REGISTER, 0),
        // Zero-fill the new block's usable region before first use.
        // `ubase`/`usable` are live across the fill call, so the allocator spills
        // them (the call's clobber mask is every integer register).
        abi::add_immediate(&ubase, abi::return_register(), ARENA_BLOCK_HEADER_SIZE), // ubase
    ]);
    // plan-133-B: count the grow-path fill — one call over the block's `usable` bytes.
    if let Some(slot) = &dbg_slot {
        emit_debug_arena_bump(
            ARENA_ALLOC_SYMBOL,
            slot,
            COUNTER_FILL_GROW_CALLS,
            "fill_grow_calls",
            &mut instructions,
            &mut vregs,
        );
        emit_debug_arena_add(
            ARENA_ALLOC_SYMBOL,
            slot,
            COUNTER_FILL_GROW_BYTES,
            &usable,
            "fill_grow_bytes",
            &mut instructions,
            &mut vregs,
        );
        // plan-133-C: sample the arena's memory series. The helper reads the clock and
        // the process's peak RSS itself and returns at once for an unregistered arena
        // (slot 0); `ubase`, `usable`, `size` and `eff_align` are vregs the allocator
        // spills across the call.
        instructions.extend([
            abi::move_register(abi::c_arg(0), slot),
            abi::branch_link(DEBUG_ARENA_SAMPLE_SYMBOL),
        ]);
    }
    instructions.extend([
        abi::move_register(abi::c_arg(0), &ubase),
        abi::move_register(abi::c_arg(1), &usable),
        abi::branch_link(ARENA_FILL_ZERO_SYMBOL),
        // Serve the request directly from the fresh chunk (allocator-05): the
        // block was sized so `usable >= size + eff_align`, so instead of
        // linking the whole chunk and re-walking the entire list to rediscover
        // it, walk once to the chunk's address-ordered slot, park the successor
        // in the fresh node's `next` word (`arena_alloc_found` reads it from
        // `[cur, 0]`), and enter the existing four-case split with
        // `cur = ubase`, `prev = ins_prev`. The split links only the
        // remainder(s). A fresh block is never adjacent to an existing chunk
        // (the 32-byte header always separates blocks), so no coalescing is
        // required here.
        abi::load_u64(&ins_cur, ARENA_STATE_REGISTER, ARENA_FREE_LIST_HEAD_OFFSET), // cur
        abi::move_immediate(&ins_prev, "Integer", "0"),                             // prev
        abi::label("arena_alloc_ins_loop"),
        abi::compare_immediate(&ins_cur, "0"),
        abi::branch_eq("arena_alloc_ins_do"),
        abi::compare_registers(&ins_cur, &ubase),
        abi::branch_hi("arena_alloc_ins_do"),
        abi::move_register(&ins_prev, &ins_cur),
        abi::load_u64(&ins_cur, &ins_cur, 0),
        abi::branch("arena_alloc_ins_loop"),
        abi::label("arena_alloc_ins_do"),
        abi::store_u64(&ins_cur, &ubase, 0), // fresh.next = successor
        abi::move_register(&cur, &ubase),
        abi::move_register(&prev, &ins_prev),
        // aligned = round_up(ubase, eff_align); end_needed = aligned + size;
        // cur_end = ubase + usable — the same geometry the walk computes. The
        // walk's overflow-skip guards are unnecessary for a fresh mapping
        // (mmap'd extents cannot wrap).
        abi::subtract_immediate(&align_mask, &eff_align, 1),
        abi::add_registers(&aligned, &cur, &align_mask),
        abi::bitwise_not(&align_notmask, &align_mask),
        abi::and_registers(&aligned, &aligned, &align_notmask),
        abi::add_registers(&end_needed, &aligned, &size),
        abi::add_registers(&cur_end, &cur, &usable),
        abi::branch("arena_alloc_found"),
        abi::label("arena_alloc_invalid"),
        abi::move_immediate(
            abi::return_register(),
            "Integer",
            crate::codegen::registry::runtime_error("ErrInvalidArgument")
                .expect("errorCode name")
                .0,
        ),
        abi::move_immediate(abi::mfb_return(1), "Integer", "0"),
        abi::branch("arena_alloc_ret"),
        abi::label("arena_alloc_oom"),
        abi::move_immediate(
            abi::return_register(),
            "Integer",
            crate::codegen::registry::runtime_error("ErrOutOfMemory")
                .expect("errorCode name")
                .0,
        ),
        abi::move_immediate(abi::mfb_return(1), "Integer", "0"),
        abi::label("arena_alloc_ret"),
    ]);
    // plan-67-F: close the `mfb_alloc` span at the single exit. The result (tag in
    // the return register, pointer in RET[1]) is live and the perf `bl` clobbers
    // both, so save them into vregs across the call and restore before returning.
    if perf {
        let saved_tag = vregs.next();
        let saved_ptr = vregs.next();
        instructions.push(abi::move_register(&saved_tag, abi::return_register()));
        instructions.push(abi::move_register(&saved_ptr, abi::mfb_return(1)));
        emit_perf_arena_call(
            "perf.end",
            ARENA_ALLOC_SYMBOL,
            PERF_NAME_MFB_ALLOC_SYMBOL,
            &mut instructions,
            &mut relocations,
        );
        instructions.push(abi::move_register(abi::return_register(), &saved_tag));
        instructions.push(abi::move_register(abi::mfb_return(1), &saved_ptr));
    }
    instructions.push(abi::return_());
    Ok(finalize_vreg_helper(
        "runtime.arena_alloc",
        ARENA_ALLOC_SYMBOL,
        "Pointer",
        instructions,
        relocations,
    ))
}

/// `_mfb_simd_alloc_list(x0 = count, x1 = valueTypeCode) -> x0 = base` —
/// allocate a tight homogeneous numeric `List` (plan-01-simd §4.3). The data
/// region is `count` contiguous 8-byte lanes at `base + 40 + count*40`. Returns
/// `0` if the arena allocation fails (the caller raises the allocation error).
///
/// Calls `_mfb_arena_alloc`, whose clobber set is wide (`x0,x1,x9,x10,x14,x15,
/// x16,x20-x28`); `count` and `valueTypeCode` are spilled across the call and
/// reloaded. After the call there are no further calls, so the header/entry
/// writes use scratch GPRs freely.
pub(crate) fn lower_simd_alloc_list() -> CodeFunction {
    let mut vregs = Vregs::new();
    // count/typeCode are live across the arena_alloc call (ALL_INT clobber), so
    // they spill — the old hand frame's COUNT/TYPE slots, now allocator-managed.
    let count = vregs.next();
    let type_code = vregs.next();
    let stride = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&count, abi::c_arg(0)),
        abi::move_register(&type_code, abi::c_arg(1)),
        // alloc size = COLLECTION_HEADER_SIZE + count*(stride + 8) (lookup + data).
        // Every list this helper builds has an 8-byte fixed-width element
        // (Integer/Float/Fixed/Money), so its stride is uniform — kind 2 drops
        // the lookup array and the block becomes HEADER + count*8 (plan-57-D).
        abi::move_immediate(
            &stride,
            "Integer",
            &(list_entry_stride(&ParameterType::Integer) + 8).to_string(),
        ),
        abi::multiply_registers(abi::c_arg(0), &count, &stride),
        abi::add_immediate(abi::c_arg(0), abi::c_arg(0), COLLECTION_HEADER_SIZE),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
        // x0 = result tag, x1 = pointer. Return x0 = base, x1 = status (0 = ok,
        // else the arena error tag) so the caller can raise the allocation error.
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq("simd_alloc_ok"),
        abi::move_register(abi::mfb_return(1), abi::return_register()),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::branch("simd_alloc_ret"),
        abi::label("simd_alloc_ok"),
    ];
    let base = vregs.next();
    let scratch = vregs.next();
    // For kind 0 the KIND and KEY_TYPE stores share one zeroed register, exactly
    // as before; for kind 2 the kind byte is non-zero so KEY_TYPE needs its own.
    let kind_zero = vregs.next();
    let zero_kind_scratch = if list_block_kind(&ParameterType::Integer) == 0 {
        scratch
    } else {
        kind_zero
    };
    let data_len = vregs.next();
    let entry = vregs.next();
    let index = vregs.next();
    let value_off = vregs.next();
    if list_block_kind(&ParameterType::Integer) != 0 {
        // Only kind 2 needs a separate zero register (see `zero_kind_scratch`);
        // emitting this unconditionally would change the kind-0 instruction
        // stream, which artifact-gate would — correctly — reject.
        instructions.push(abi::move_immediate(&kind_zero, "Integer", "0"));
    }
    instructions.extend([
        abi::move_register(&base, abi::mfb_return(1)),
        // Header: kind, keyType=0, valueType=typeCode, flagsVersion=1. The kind-0
        // build keeps the shared zero register for both stores, so its emitted
        // sequence is unchanged.
        abi::move_immediate(
            &scratch,
            "Integer",
            &list_block_kind(&ParameterType::Integer).to_string(),
        ),
        abi::store_u8(&scratch, &base, COLLECTION_OFFSET_KIND),
        abi::store_u8(&zero_kind_scratch, &base, COLLECTION_OFFSET_KEY_TYPE),
        abi::store_u8(&type_code, &base, COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate(&scratch, "Integer", "1"),
        abi::store_u8(&scratch, &base, COLLECTION_OFFSET_FLAGS_VERSION),
        // count, capacity = count; dataLength, dataCapacity = count*8.
        abi::store_u64(&count, &base, COLLECTION_OFFSET_COUNT),
        abi::store_u64(&count, &base, COLLECTION_OFFSET_CAPACITY),
        abi::shift_left_immediate(&data_len, &count, 3),
        abi::store_u64(&data_len, &base, COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64(&data_len, &base, COLLECTION_OFFSET_DATA_CAPACITY),
    ]);
    // kind 2 has no lookup array to fill.
    if list_entry_stride(&ParameterType::Integer) != 0 {
        instructions.extend([
            // Fill the lookup entries: flags=USED, valueOffset=i*8, valueLength=8.
            abi::add_immediate(&entry, &base, COLLECTION_HEADER_SIZE),
            abi::move_immediate(&index, "Integer", "0"),
            abi::move_immediate(&value_off, "Integer", "0"),
            abi::label("simd_alloc_entry_loop"),
            abi::compare_registers(&index, &count),
            abi::branch_ge("simd_alloc_entry_done"),
            abi::move_immediate(&scratch, "Integer", &COLLECTION_ENTRY_FLAG_USED.to_string()),
            abi::store_u8(&scratch, &entry, COLLECTION_ENTRY_OFFSET_FLAGS),
            abi::store_u64(&value_off, &entry, COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
            abi::move_immediate(&scratch, "Integer", "8"),
            abi::store_u64(&scratch, &entry, COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
            abi::add_immediate(&value_off, &value_off, 8),
            abi::add_immediate(&entry, &entry, COLLECTION_ENTRY_SIZE),
            abi::add_immediate(&index, &index, 1),
            abi::branch("simd_alloc_entry_loop"),
            abi::label("simd_alloc_entry_done"),
        ]);
    }
    instructions.extend([
        abi::move_register(abi::return_register(), &base),
        abi::move_immediate(abi::mfb_return(1), "Integer", "0"),
        abi::label("simd_alloc_ret"),
        abi::return_(),
    ]);
    finalize_vreg_helper(
        "runtime.simd_alloc_list",
        SIMD_ALLOC_LIST_SYMBOL,
        "Pointer",
        instructions,
        vec![internal_branch(SIMD_ALLOC_LIST_SYMBOL, ARENA_ALLOC_SYMBOL)],
    )
}
