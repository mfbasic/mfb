use super::*;

pub(super) fn lower_arena_alloc(platform: &dyn CodegenPlatform) -> Result<CodeFunction, String> {
    // Vreg-allocated (plan-00-G Phase 2): the body names virtual registers and the
    // shared allocator places them per-ISA; `finalize_vreg_helper` runs the
    // allocator + `finalize_frame` (which builds the frame, saves the link
    // register because the grow path calls `arena_fill_random`, and saves any
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
    // Segregated large-block bins (plan-25-A): a large request pops an exact-size
    // node from its hashed bin before falling to the first-fit walk.
    let lg_mask = vregs.next();
    let lg_slot = vregs.next();
    let lg_link = vregs.next();
    let lg_cur = vregs.next();
    let lg_next = vregs.next();
    let lg_msize = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::compare_immediate(abi::ARG[1], "0"),
        abi::branch_eq("arena_alloc_invalid"),
        abi::subtract_immediate(&align_low, abi::ARG[1], 1),
        abi::and_registers(&align_pow2, abi::ARG[1], &align_low),
        abi::compare_immediate(&align_pow2, "0"),
        abi::branch_ne("arena_alloc_invalid"),
        // eff align = max(align, 16)
        abi::move_register(&eff_align, abi::ARG[1]),
        abi::compare_immediate(&eff_align, &ARENA_MIN_CHUNK.to_string()),
        abi::branch_lo("arena_alloc_align_min"),
        abi::branch("arena_alloc_align_ready"),
        abi::label("arena_alloc_align_min"),
        abi::move_immediate(&eff_align, "Integer", &ARENA_MIN_CHUNK.to_string()),
        abi::label("arena_alloc_align_ready"),
        // normalized size = round_up(max(size, 1), 16)
        abi::move_register(&size, abi::ARG[0]),
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
        abi::move_immediate(abi::return_register(), "Integer", RESULT_OK_TAG),
        abi::move_register(abi::RET[1], &bin_head),
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
        abi::move_immediate(abi::return_register(), "Integer", RESULT_OK_TAG),
        abi::move_register(abi::RET[1], &bin_head),
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
        abi::move_register(abi::ARG[0], &bin_head),
        abi::move_register(abi::ARG[1], &bin_rem),
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
        abi::label("arena_alloc_dv_serve"),
        // Serve `size` from the new DV chunk (bin_head/bin_rem) and store the
        // shrunken DV.
        abi::subtract_registers(&bin_rem, &bin_rem, &size),
        abi::add_registers(&bin_next, &bin_head, &size),
        abi::store_u64(&bin_next, ARENA_STATE_REGISTER, ARENA_CARVE_PTR_OFFSET),
        abi::store_u64(&bin_rem, ARENA_STATE_REGISTER, ARENA_CARVE_SIZE_OFFSET),
        abi::move_immediate(abi::return_register(), "Integer", RESULT_OK_TAG),
        abi::move_register(abi::RET[1], &bin_head),
        abi::branch("arena_alloc_ret"),
        // --- Segregated large-block bin pop (plan-25-A) ------------------------
        // A large request (size > QUICK_BIN_MAX, eff_align ≤ 16 — larger aligns
        // branched straight to the walk above) first scans its hashed bin for an
        // EXACT-size free node and returns it whole (no split) in O(1) amortized.
        // Diverting large frees off the address-ordered list (see arena_free)
        // keeps that list short, so both a bin hit here and a bin miss's
        // fall-through walk stay cheap under heavy large-list churn — the
        // benchmark's ~30× inflation was this list growing without bound. The
        // scan is an exact match because free and alloc normalize `size`
        // identically, so a reused chunk round-trips to the same bin; a chunk of
        // a different colliding size is simply skipped (it stays parked in its bin
        // and is only reclaimed at `arena_destroy` — there is no flush-before-grow
        // drain for large bins; bug-175 H corrected the stale claim).
        abi::label("arena_alloc_large_bin"),
        abi::shift_right_immediate(&lg_slot, &size, 4),
        abi::move_immediate(
            &lg_mask,
            "Integer",
            &(ARENA_LARGE_BIN_COUNT - 1).to_string(),
        ),
        abi::and_registers(&lg_slot, &lg_slot, &lg_mask),
        abi::shift_left_immediate(&lg_slot, &lg_slot, 3),
        abi::add_registers(&lg_slot, ARENA_STATE_REGISTER, &lg_slot),
        // lg_link tracks the address of the word that points at lg_cur (the bin
        // head cell first, then each visited node's `next` at +0), so an
        // exact-size hit unlinks in O(1) whether it is the head or mid-list.
        abi::add_immediate(&lg_link, &lg_slot, ARENA_LARGE_BIN_BASE_OFFSET),
        abi::load_u64(&lg_cur, &lg_link, 0),
        abi::label("arena_alloc_large_scan"),
        abi::compare_immediate(&lg_cur, "0"),
        abi::branch_eq("arena_alloc_walk"),
        abi::load_u64(&lg_msize, &lg_cur, 8),
        abi::compare_registers(&lg_msize, &size),
        abi::branch_eq("arena_alloc_large_hit"),
        abi::move_register(&lg_link, &lg_cur),
        abi::load_u64(&lg_cur, &lg_cur, 0),
        abi::branch("arena_alloc_large_scan"),
        abi::label("arena_alloc_large_hit"),
        abi::load_u64(&lg_next, &lg_cur, 0),
        abi::store_u64(&lg_next, &lg_link, 0),
        abi::move_immediate(abi::return_register(), "Integer", RESULT_OK_TAG),
        abi::move_register(abi::RET[1], &lg_cur),
        abi::branch("arena_alloc_ret"),
        // --- First-fit walk over the address-ordered free-list -----------------
        abi::label("arena_alloc_walk"),
        abi::load_u64(&cur, ARENA_STATE_REGISTER, ARENA_FREE_LIST_HEAD_OFFSET),
        abi::move_immediate(&prev, "Integer", "0"),
        abi::label("arena_alloc_walk_loop"),
        abi::compare_immediate(&cur, "0"),
        abi::branch_eq("arena_alloc_grow"),
        abi::load_u64(&cur_size, &cur, 8), // cur_size
    ];
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
        abi::move_register(abi::RET[1], &aligned),
        abi::branch("arena_alloc_ret"),
    ]);
    // --- Grow: map a new block and carve the request from it ----------------
    let map_size = vregs.next();
    let default_block = vregs.next();
    let saved_size = vregs.next();
    let page_mask = vregs.next();
    // Flush-before-grow scratch (allocator-01): the drain loop's cursors are
    // loop-carried across the `arena_insert_free` calls, so they live in vregs
    // the allocator spills.
    let flush_index = vregs.next();
    let flush_offset = vregs.next();
    let flush_slot = vregs.next();
    let flush_node = vregs.next();
    let flush_next = vregs.next();
    instructions.extend([
        abi::label("arena_alloc_grow"),
        // Flush-before-grow (allocator-01), gated to SMALL requests: a small
        // request only reaches here after the exact bin, the larger-bin scan,
        // and the walk all failed — so the bins hold nothing ≥ this class,
        // the drain is cheap, and coalescing adjacent parked chunks genuinely
        // can produce a fit. A BIG request (> QUICK_BIN_MAX, or align > 16)
        // grows directly: its flush would drain a large parked-small inventory
        // through the O(list) insert (measured: hundreds of millions of insert
        // steps) for chunks that almost never coalesce past interleaved live
        // objects into a big-enough run. The `flushed` flag arms exactly one
        // retry — a second walk miss falls through to the map below.
        abi::compare_immediate(&flushed, "0"),
        abi::branch_ne("arena_alloc_grow_map"),
        abi::compare_immediate(&eff_align, &ARENA_MIN_CHUNK.to_string()),
        abi::branch_hi("arena_alloc_grow_map"),
        abi::compare_immediate(&size, &ARENA_QUICK_BIN_MAX.to_string()),
        abi::branch_hi("arena_alloc_grow_map"),
        abi::move_immediate(&flushed, "Integer", "1"),
        abi::move_immediate(&flush_index, "Integer", "0"),
        abi::label("arena_alloc_flush_bin"),
        abi::compare_immediate(&flush_index, &ARENA_QUICK_BIN_COUNT.to_string()),
        abi::branch_eq("arena_alloc_flush_done"),
        abi::shift_left_immediate(&flush_offset, &flush_index, 3),
        abi::add_registers(&flush_slot, ARENA_STATE_REGISTER, &flush_offset),
        abi::load_u64(&flush_node, &flush_slot, ARENA_QUICK_BIN_BASE_OFFSET),
        abi::store_u64(abi::ZERO, &flush_slot, ARENA_QUICK_BIN_BASE_OFFSET),
        abi::label("arena_alloc_flush_chain"),
        abi::compare_immediate(&flush_node, "0"),
        abi::branch_eq("arena_alloc_flush_next_bin"),
        abi::load_u64(&flush_next, &flush_node, 0),
        abi::load_u64(abi::ARG[1], &flush_node, 8),
        abi::move_register(abi::ARG[0], &flush_node),
        abi::branch_link(ARENA_INSERT_FREE_SYMBOL),
        abi::move_register(&flush_node, &flush_next),
        abi::branch("arena_alloc_flush_chain"),
        abi::label("arena_alloc_flush_next_bin"),
        abi::add_immediate(&flush_index, &flush_index, 1),
        abi::branch("arena_alloc_flush_bin"),
        // Post-flush re-park sweep: after coalescing, move every list chunk
        // ≤ QUICK_BIN_MAX back onto its exact-size bin. Without this, drained
        // small chunks that did not merge into large runs rot on the list
        // forever — nothing small ever walks (bins and the victim serve
        // first), so ONLY large requests pay for them, once per walk
        // (measured: 17k dead 16-byte nodes doubling a JSON parse). After the
        // sweep the list holds only > QUICK_BIN_MAX chunks and the retry
        // re-enters through the victim-renewal bin scan, which sees every
        // swept chunk.
        abi::label("arena_alloc_flush_done"),
        abi::move_immediate(&flush_slot, "Integer", "0"), // prev
        abi::load_u64(
            &flush_node,
            ARENA_STATE_REGISTER,
            ARENA_FREE_LIST_HEAD_OFFSET,
        ),
        abi::label("arena_alloc_sweep_loop"),
        abi::compare_immediate(&flush_node, "0"),
        abi::branch_eq("arena_alloc_sweep_done"),
        abi::load_u64(&flush_next, &flush_node, 0),
        abi::load_u64(&flush_offset, &flush_node, 8),
        abi::compare_immediate(&flush_offset, &ARENA_QUICK_BIN_MAX.to_string()),
        abi::branch_hi("arena_alloc_sweep_keep"),
        // Unlink cur from the list …
        abi::compare_immediate(&flush_slot, "0"),
        abi::branch_eq("arena_alloc_sweep_unlink_head"),
        abi::store_u64(&flush_next, &flush_slot, 0),
        abi::branch("arena_alloc_sweep_binpush"),
        abi::label("arena_alloc_sweep_unlink_head"),
        abi::store_u64(
            &flush_next,
            ARENA_STATE_REGISTER,
            ARENA_FREE_LIST_HEAD_OFFSET,
        ),
        abi::label("arena_alloc_sweep_binpush"),
        // … and push it onto its exact-size bin (node.size at +8 is intact).
        abi::shift_right_immediate(&bin_scan, &flush_offset, 4),
        abi::shift_left_immediate(&bin_scan, &bin_scan, 3),
        abi::add_registers(&bin_scan, ARENA_STATE_REGISTER, &bin_scan),
        abi::load_u64(&bin_head, &bin_scan, ARENA_QUICK_BIN_BASE_OFFSET - 8),
        abi::store_u64(&bin_head, &flush_node, 0),
        abi::store_u64(&flush_node, &bin_scan, ARENA_QUICK_BIN_BASE_OFFSET - 8),
        abi::move_register(&flush_node, &flush_next),
        abi::branch("arena_alloc_sweep_loop"),
        abi::label("arena_alloc_sweep_keep"),
        abi::move_register(&flush_slot, &flush_node),
        abi::move_register(&flush_node, &flush_next),
        abi::branch("arena_alloc_sweep_loop"),
        abi::label("arena_alloc_sweep_done"),
        abi::branch("arena_alloc_dv_scan"),
        abi::label("arena_alloc_grow_map"),
        abi::add_registers(&map_size, &size, &eff_align),
        abi::compare_registers(&map_size, &size),
        abi::branch_lo("arena_alloc_oom"),
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
    // mmap `map_size` bytes; the result is left in the return register. `map_size`
    // is live across the syscall (read again below for the block header), so the
    // allocator keeps it in a callee-saved register or spills it.
    platform.emit_arena_map(&map_size, &mut instructions)?;
    let prev_block = vregs.next();
    let usable = vregs.next();
    let ubase = vregs.next();
    let ins_cur = vregs.next();
    let ins_prev = vregs.next();
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ge("arena_alloc_mapped"),
        abi::branch("arena_alloc_oom"),
        abi::label("arena_alloc_mapped"),
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
        // Poison the new block's usable region before first use (plan-01 §6.3).
        // `ubase`/`usable` are live across the fill call, so the allocator spills
        // them (the call's clobber mask is every integer register).
        abi::add_immediate(&ubase, abi::return_register(), ARENA_BLOCK_HEADER_SIZE), // ubase
        abi::move_register(abi::ARG[0], &ubase),
        abi::move_register(abi::ARG[1], &usable),
        abi::branch_link(ARENA_FILL_RANDOM_SYMBOL),
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
        abi::move_immediate(abi::return_register(), "Integer", ERR_INVALID_ARGUMENT_CODE),
        abi::move_immediate(abi::RET[1], "Integer", "0"),
        abi::branch("arena_alloc_ret"),
        abi::label("arena_alloc_oom"),
        abi::move_immediate(abi::return_register(), "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(abi::RET[1], "Integer", "0"),
        abi::label("arena_alloc_ret"),
        abi::return_(),
    ]);
    let relocations = vec![internal_branch(
        ARENA_ALLOC_SYMBOL,
        ARENA_FILL_RANDOM_SYMBOL,
    )];
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
pub(super) fn lower_simd_alloc_list() -> CodeFunction {
    let mut vregs = Vregs::new();
    // count/typeCode are live across the arena_alloc call (ALL_INT clobber), so
    // they spill — the old hand frame's COUNT/TYPE slots, now allocator-managed.
    let count = vregs.next();
    let type_code = vregs.next();
    let stride = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&count, abi::ARG[0]),
        abi::move_register(&type_code, abi::ARG[1]),
        // alloc size = COLLECTION_HEADER_SIZE + count*(stride + 8) (lookup + data).
        // Every list this helper builds has an 8-byte fixed-width element
        // (Integer/Float/Fixed/Money), so its stride is uniform — kind 2 drops
        // the lookup array and the block becomes HEADER + count*8 (plan-57-D).
        abi::move_immediate(
            &stride,
            "Integer",
            &(list_entry_stride("Integer") + 8).to_string(),
        ),
        abi::multiply_registers(abi::ARG[0], &count, &stride),
        abi::add_immediate(abi::ARG[0], abi::ARG[0], COLLECTION_HEADER_SIZE),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
        // x0 = result tag, x1 = pointer. Return x0 = base, x1 = status (0 = ok,
        // else the arena error tag) so the caller can raise the allocation error.
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq("simd_alloc_ok"),
        abi::move_register(abi::RET[1], abi::return_register()),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::branch("simd_alloc_ret"),
        abi::label("simd_alloc_ok"),
    ];
    let base = vregs.next();
    let scratch = vregs.next();
    // For kind 0 the KIND and KEY_TYPE stores share one zeroed register, exactly
    // as before; for kind 2 the kind byte is non-zero so KEY_TYPE needs its own.
    let kind_zero = vregs.next();
    let zero_kind_scratch = if list_block_kind("Integer") == 0 {
        scratch.clone()
    } else {
        kind_zero.clone()
    };
    let data_len = vregs.next();
    let entry = vregs.next();
    let index = vregs.next();
    let value_off = vregs.next();
    if list_block_kind("Integer") != 0 {
        // Only kind 2 needs a separate zero register (see `zero_kind_scratch`);
        // emitting this unconditionally would change the kind-0 instruction
        // stream, which artifact-gate would — correctly — reject.
        instructions.push(abi::move_immediate(&kind_zero, "Integer", "0"));
    }
    instructions.extend([
        abi::move_register(&base, abi::RET[1]),
        // Header: kind, keyType=0, valueType=typeCode, flagsVersion=1. The kind-0
        // build keeps the shared zero register for both stores, so its emitted
        // sequence is unchanged.
        abi::move_immediate(&scratch, "Integer", &list_block_kind("Integer").to_string()),
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
    if list_entry_stride("Integer") != 0 {
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
        abi::move_immediate(abi::RET[1], "Integer", "0"),
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

/// `arena_insert_free(x0 = ptr, x1 = size)` — insert a chunk into the
/// address-ordered free-list and coalesce with the address-adjacent neighbor on
/// either side. `size` must already be normalized (≥16, multiple of 16) and
/// `ptr` 16-aligned; both hold for every chunk the allocator hands out and for a
/// fresh block's usable region. A `ptr` that is already a free node is a no-op
/// (allocator-03 idempotency guard), so a double-free relinks nothing. Leaf
/// function; vreg-allocated — treat all caller-saved integer registers as
/// clobbered.
pub(super) fn lower_arena_insert_free() -> CodeFunction {
    let mut vregs = Vregs::new();
    // ptr (x0) / size (x1) are read-only args; this is a leaf, so they stay
    // physical. Everything else is a vreg the allocator places.
    let cur = vregs.next();
    let prev = vregs.next();
    let t1 = vregs.next();
    let t2 = vregs.next();
    let merged = vregs.next();
    let instructions = vec![
        abi::label("entry"),
        // Walk to the insertion slot: prev = largest node < ptr (or 0),
        // cur = smallest node > ptr (or 0).
        abi::load_u64(&cur, ARENA_STATE_REGISTER, ARENA_FREE_LIST_HEAD_OFFSET),
        abi::move_immediate(&prev, "Integer", "0"),
        abi::label("insert_find"),
        abi::compare_immediate(&cur, "0"),
        abi::branch_eq("insert_slot"),
        abi::compare_registers(&cur, abi::ARG[0]),
        abi::branch_hi("insert_slot"), // cur > ptr
        // Idempotency guard (allocator-03): the chunk is already a free node —
        // a double-free becomes a no-op instead of double-linking `ptr` and
        // coalescing against its own (about-to-be-rewritten) metadata.
        abi::compare_registers(&cur, abi::ARG[0]),
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
        abi::compare_registers(&t2, abi::ARG[0]),
        abi::branch_ne("insert_check_next"),
        // prev is address-adjacent: absorb the chunk into prev.
        abi::add_registers(&t1, &t1, abi::ARG[1]),
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
        abi::add_registers(&t2, abi::ARG[0], abi::ARG[1]), // chunk_end
        abi::compare_registers(&t2, &cur),
        abi::branch_ne("insert_standalone"),
        // chunk is address-adjacent to cur: new node at ptr absorbs cur.
        abi::load_u64(&t1, &cur, 8), // cur.size
        abi::add_registers(&t1, &t1, abi::ARG[1]),
        abi::store_u64(&t1, abi::ARG[0], 8),
        abi::load_u64(&t1, &cur, 0), // cur.next
        abi::store_u64(&t1, abi::ARG[0], 0),
        abi::branch("insert_link_prev"),
        abi::label("insert_standalone"),
        abi::store_u64(&cur, abi::ARG[0], 0), // ptr.next = cur
        abi::store_u64(abi::ARG[1], abi::ARG[0], 8), // ptr.size = size
        abi::branch("insert_link_prev"),
        abi::label("insert_finish_no_next"),
        abi::compare_immediate(&merged, "0"),
        abi::branch_ne("insert_done"), // merged into prev, nothing to link
        abi::store_u64(abi::ZERO, abi::ARG[0], 0), // ptr.next = 0
        abi::store_u64(abi::ARG[1], abi::ARG[0], 8), // ptr.size = size
        abi::branch("insert_link_prev"),
        abi::label("insert_link_prev"),
        abi::compare_immediate(&prev, "0"),
        abi::branch_eq("insert_set_head"),
        abi::store_u64(abi::ARG[0], &prev, 0), // prev.next = ptr
        abi::branch("insert_done"),
        abi::label("insert_set_head"),
        abi::store_u64(
            abi::ARG[0],
            ARENA_STATE_REGISTER,
            ARENA_FREE_LIST_HEAD_OFFSET,
        ),
        abi::label("insert_done"),
        abi::return_(),
        abi::label("insert_already_free"),
        abi::return_(),
    ];
    finalize_vreg_helper(
        "runtime.arena_insert_free",
        ARENA_INSERT_FREE_SYMBOL,
        "Nothing",
        instructions,
        Vec::new(),
    )
}

/// `arena_free(x0 = ptr, x1 = size)` — return a single compiler-sized allocation
/// to the per-arena allocator. Normalizes `size` exactly as `arena_alloc` did
/// (so the freed extent matches the live chunk), then either parks the chunk on
/// its exact-size quick bin (`size ≤ ARENA_QUICK_BIN_MAX`, O(1) push —
/// allocator-01) or coalesces it into the address-ordered list via
/// `arena_insert_free`; afterwards it entropy-scrubs the payload bytes past the
/// 16-byte FreeNode overlay just written (plan-01 §6.2, allocator-03: the
/// insert must never read PRNG-poisoned free-list metadata, and a double-free —
/// an idempotent no-op inside the insert — must never scrub a live node's
/// `{next, size}` words). Never unmaps. Vreg-allocated — treat all caller-saved
/// integer registers as clobbered.
pub(super) fn lower_arena_free() -> CodeFunction {
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
    let instructions = vec![
        abi::label("entry"),
        abi::move_register(&ptr, abi::ARG[0]),
        // normalize size = round_up(max(size, 1), 16) — x1 is the size arg.
        abi::compare_immediate(abi::ARG[1], "0"),
        abi::branch_ne("arena_free_size_nonzero"),
        abi::move_immediate(abi::ARG[1], "Integer", "1"),
        abi::label("arena_free_size_nonzero"),
        abi::add_immediate(abi::ARG[1], abi::ARG[1], (ARENA_MIN_CHUNK - 1) as usize),
        abi::move_immediate(&mask, "Integer", &not_15),
        abi::and_registers(&size, abi::ARG[1], &mask),
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
        abi::branch_eq("arena_free_done"),
        abi::store_u64(&bin_head, &ptr, 0),
        abi::store_u64(&size, &ptr, 8),
        abi::store_u64(&ptr, &bin_slot, ARENA_QUICK_BIN_BASE_OFFSET - 8),
        abi::branch("arena_free_scrub"),
        // A larger chunk (> ARENA_QUICK_BIN_MAX) parks on its hashed large-block
        // bin (plan-25-A): an O(1) head push keyed by `(size >> 4) & (COUNT-1)`,
        // no address-ordered walk. This is the master benchmark fix — routing
        // large frees through `arena_insert_free` grew the coalescing list
        // without bound (every large 1000-element list op frees ~40 KB), so both
        // the insert here and every later alloc walk went quadratic. Bin nodes
        // reuse the FreeNode {next, size} overlay so the flush-before-grow drain
        // can hand them straight to `arena_insert_free` when coalescing is
        // needed. The chunk is still scrubbed below.
        abi::label("arena_free_large_bin"),
        abi::shift_right_immediate(&bin_class, &size, 4),
        abi::move_immediate(&mask, "Integer", &(ARENA_LARGE_BIN_COUNT - 1).to_string()),
        abi::and_registers(&bin_class, &bin_class, &mask),
        abi::shift_left_immediate(&bin_class, &bin_class, 3),
        abi::add_registers(&bin_slot, ARENA_STATE_REGISTER, &bin_class),
        abi::load_u64(&bin_head, &bin_slot, ARENA_LARGE_BIN_BASE_OFFSET),
        // Idempotency guard (allocator-03 parity, bug-266): an immediate double-free
        // of a large chunk already at the bin head would self-cycle it; skip.
        abi::compare_registers(&ptr, &bin_head),
        abi::branch_eq("arena_free_done"),
        abi::store_u64(&bin_head, &ptr, 0),
        abi::store_u64(&size, &ptr, 8),
        abi::store_u64(&ptr, &bin_slot, ARENA_LARGE_BIN_BASE_OFFSET),
        // … then scrub only [ptr+16, ptr+size), preserving the freshly written
        // node words. A 16-byte chunk is all node — nothing to scrub.
        abi::label("arena_free_scrub"),
        abi::compare_immediate(&size, &ARENA_MIN_CHUNK.to_string()),
        abi::branch_eq("arena_free_done"),
        abi::add_immediate(abi::ARG[0], &ptr, ARENA_MIN_CHUNK as usize),
        abi::subtract_immediate(abi::ARG[1], &size, ARENA_MIN_CHUNK as usize),
        abi::branch_link(ARENA_FILL_RANDOM_SYMBOL),
        abi::label("arena_free_done"),
        abi::return_(),
    ];
    finalize_vreg_helper(
        "runtime.arena_free",
        ARENA_FREE_SYMBOL,
        "Nothing",
        instructions,
        vec![internal_branch(ARENA_FREE_SYMBOL, ARENA_FILL_RANDOM_SYMBOL)],
    )
}

pub(super) fn lower_arena_destroy(platform: &dyn CodegenPlatform) -> Result<CodeFunction, String> {
    // Vreg-allocated (plan-00-G Phase 2): walk the block list and `munmap` each
    // block. `head` (the loop cursor) and `next` are loop-carried across the
    // `munmap` syscall, so the allocator keeps them in callee-saved registers (or
    // spills them); the syscall's own ABI registers (x0/x1 + the syscall-number
    // register) stay physical. The block address/size are passed to the syscall in
    // x0/x1, exactly where `emit_arena_unmap` expects them.
    let mut vregs = Vregs::new();
    let head = vregs.next();
    let next = vregs.next();
    let clear_cursor = vregs.next();
    let clear_limit = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::load_u64(&head, ARENA_STATE_REGISTER, 0),
        abi::label("arena_destroy_loop"),
        abi::compare_immediate(&head, "0"),
        abi::branch_eq("arena_destroy_done"),
        abi::load_u64(&next, &head, 0),
        abi::load_u64(abi::SYSARG[1], &head, 8),
        abi::move_register(abi::return_register(), &head),
    ];
    platform.emit_arena_unmap(&mut instructions)?;
    instructions.extend([
        abi::move_register(&head, &next),
        abi::branch("arena_destroy_loop"),
        abi::label("arena_destroy_done"),
        // Leave the arena fully inert (allocator-04): clear the free-list head
        // alongside the block-list head — it points into the just-unmapped
        // blocks, and a stale head would turn any post-destroy allocation into
        // a use-after-free walk. The quick bins (allocator-01) point into the
        // same unmapped blocks, so clear them too.
        abi::store_u64(abi::ZERO, ARENA_STATE_REGISTER, 0),
        abi::store_u64(abi::ZERO, ARENA_STATE_REGISTER, ARENA_FREE_LIST_HEAD_OFFSET),
        abi::add_immediate(
            &clear_cursor,
            ARENA_STATE_REGISTER,
            ARENA_QUICK_BIN_BASE_OFFSET,
        ),
        abi::add_immediate(&clear_limit, ARENA_STATE_REGISTER, ARENA_STATE_SIZE),
        abi::label("arena_destroy_bins"),
        abi::store_u64(abi::ZERO, &clear_cursor, 0),
        abi::add_immediate(&clear_cursor, &clear_cursor, 8),
        abi::compare_registers(&clear_cursor, &clear_limit),
        abi::branch_lo("arena_destroy_bins"),
        abi::return_(),
    ]);
    Ok(finalize_vreg_helper(
        "runtime.arena_destroy",
        ARENA_DESTROY_SYMBOL,
        "Nothing",
        instructions,
        Vec::new(),
    ))
}
