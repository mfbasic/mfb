//! plan-183-C: the small region — 64 KiB pages, each serving one 16-byte size class
//! up to `ARENA_SMALL_MAX`, with every page's slot bitmap in its descriptor (never in
//! the page). A slot bit is 1 while the slot is handed out.
//!
//! Each class keeps a doubly linked list of its pages that have a free slot (heads
//! in the arena state, links in the descriptors, both as page index + 1). A page
//! whose last slot is freed leaves its class for the empty-page list, from which any
//! class takes its next page before a never-used one is committed.

use super::layout::*;
use super::*;
use crate::codegen::runtime::arena_cap::ARENA_COMMIT_GRANULE;

/// `_mfb_arena_small_page(k)`: put a page on class `k`'s list (`k` = size/16 - 1) and
/// return its index + 1, or 0 when the small region is full or a commit is refused.
pub(crate) const ARENA_SMALL_PAGE_SYMBOL: &str = "_mfb_arena_small_page";

/// `dst = slice base + page × ARENA_SMALL_DESC_SIZE` for a 0-based `page`.
pub(crate) fn emit_desc_address(
    dst: &VirtualRegister,
    page: &VirtualRegister,
    scratch: &VirtualRegister,
    instructions: &mut Vec<CodeInstruction>,
) {
    instructions.extend([
        abi::move_immediate(scratch, "Integer", &ARENA_SMALL_DESC_SIZE.to_string()),
        abi::multiply_registers(dst, page, scratch),
        abi::load_u64(scratch, ARENA_STATE_REGISTER, ARENA_SLICE_BASE_OFFSET),
        abi::add_registers(dst, dst, scratch),
    ]);
}

/// `dst = &state.smallHead[k]`.
pub(crate) fn emit_small_head(
    dst: &VirtualRegister,
    k: &VirtualRegister,
    instructions: &mut Vec<CodeInstruction>,
) {
    instructions.extend([
        abi::shift_left_immediate(dst, k, 3),
        abi::add_registers(dst, ARENA_STATE_REGISTER, dst),
        abi::add_immediate(dst, dst, ARENA_SMALL_HEAD_BASE_OFFSET),
    ]);
}

pub(crate) fn lower_arena_small_page(debug_arena: bool) -> CodeFunction {
    let symbol = ARENA_SMALL_PAGE_SYMBOL;
    let mut vregs = Vregs::new();
    let k = vregs.next();
    let sc = vregs.next();
    let p1 = vregs.next();
    let idx = vregs.next();
    let desc = vregs.next();
    let t = vregs.next();
    let u = vregs.next();
    let need = vregs.next();
    let ceiling = vregs.next();
    let full = "arena_small_page_full";
    let init = "arena_small_page_init";
    let mut instructions = vec![abi::label("entry"), abi::move_register(&k, abi::c_arg(0))];
    let mut relocations = Vec::new();
    let dbg_slot = debug_arena.then(|| {
        let slot = vregs.next();
        emit_debug_arena_slot(
            symbol,
            &slot,
            &mut instructions,
            &mut relocations,
            &mut vregs,
        );
        slot
    });
    emit_scalars(&sc, &mut instructions);
    instructions.extend([
        abi::load_u64(&p1, &sc, ARENA_SC_SMALL_EMPTY),
        abi::compare_immediate(&p1, "0"),
        abi::branch_eq("arena_small_page_fresh"),
        abi::subtract_immediate(&idx, &p1, 1),
    ]);
    emit_desc_address(&desc, &idx, &t, &mut instructions);
    // plan-183-E: an empty page past the ones kept committed was decommitted; commit it
    // again before it leaves the list (a refusal leaves the list as it was).
    instructions.extend([
        abi::load_u64(&t, &desc, ARENA_SMALL_DESC_COLD),
        abi::compare_immediate(&t, "0"),
        abi::branch_ne("arena_small_page_cold"),
        abi::load_u64(&t, &sc, ARENA_SC_EMPTY_HOT),
        abi::subtract_immediate(&t, &t, 1),
        abi::store_u64(&t, &sc, ARENA_SC_EMPTY_HOT),
        abi::branch("arena_small_page_popped"),
        abi::label("arena_small_page_cold"),
        abi::shift_left_immediate(&t, &idx, ARENA_SMALL_PAGE.trailing_zeros() as u8),
        abi::load_u64(&u, &sc, ARENA_SC_SMALL_PAGES),
        abi::add_registers(abi::c_arg(0), &u, &t),
        abi::move_immediate(&t, "Integer", &ARENA_SMALL_PAGE.to_string()),
        abi::add_registers(abi::c_arg(1), abi::c_arg(0), &t),
        abi::branch_link(ARENA_COMMIT_SYMBOL),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(full),
        abi::subtract_immediate(&idx, &p1, 1),
    ]);
    relocations.push(internal_branch(symbol, ARENA_COMMIT_SYMBOL));
    emit_desc_address(&desc, &idx, &t, &mut instructions);
    instructions.extend([
        abi::store_u64(abi::ZERO, &desc, ARENA_SMALL_DESC_COLD),
        abi::label("arena_small_page_popped"),
        abi::load_u64(&t, &desc, ARENA_SMALL_DESC_NEXT),
        abi::store_u64(&t, &sc, ARENA_SC_SMALL_EMPTY),
        abi::branch(init),
        abi::label("arena_small_page_fresh"),
        abi::load_u64(&idx, &sc, ARENA_SC_SMALL_NEXT),
        abi::load_u64(&t, &sc, ARENA_SC_SMALL_LIMIT),
        abi::compare_registers(&idx, &t),
        abi::branch_lo("arena_small_page_room"),
        abi::branch(full),
        abi::label("arena_small_page_room"),
        // The descriptor's bytes, then the page's.
        abi::add_immediate(&t, &idx, 1),
    ]);
    emit_desc_address(&need, &t, &u, &mut instructions);
    instructions.push(abi::load_u64(&ceiling, &sc, ARENA_SC_SMALL_PAGES));
    emit_ensure_up(
        symbol,
        "table",
        &sc,
        ARENA_SC_TABLE_COMMIT,
        &need,
        &ceiling,
        ARENA_COMMIT_GRANULE,
        full,
        &mut instructions,
        &mut relocations,
        &mut vregs,
    );
    instructions.extend([
        abi::add_immediate(&t, &idx, 1),
        abi::shift_left_immediate(&t, &t, ARENA_SMALL_PAGE.trailing_zeros() as u8),
        abi::load_u64(&need, &sc, ARENA_SC_SMALL_PAGES),
        abi::add_registers(&need, &need, &t),
        abi::load_u64(&ceiling, &sc, ARENA_SC_SMALL_END),
    ]);
    emit_ensure_up(
        symbol,
        "pages",
        &sc,
        ARENA_SC_SMALL_COMMIT,
        &need,
        &ceiling,
        ARENA_COMMIT_STEP,
        full,
        &mut instructions,
        &mut relocations,
        &mut vregs,
    );
    instructions.extend([
        abi::add_immediate(&p1, &idx, 1),
        abi::store_u64(&p1, &sc, ARENA_SC_SMALL_NEXT),
    ]);
    // plan-133-C, plan-183-C: a never-used page extends the small region — sample the
    // memory series.
    if let Some(slot) = &dbg_slot {
        instructions.extend([
            abi::move_register(abi::c_arg(0), slot),
            abi::branch_link(DEBUG_ARENA_SAMPLE_SYMBOL),
        ]);
        relocations.push(internal_branch(symbol, DEBUG_ARENA_SAMPLE_SYMBOL));
    }
    emit_desc_address(&desc, &idx, &t, &mut instructions);
    // Initialise the descriptor: class, slot count, all slots free, an empty bitmap
    // but for the bits past the last slot, which stay set so no scan takes them.
    let slots = vregs.next();
    let cursor = vregs.next();
    let end = vregs.next();
    instructions.extend([
        abi::label(init),
        abi::add_immediate(&t, &k, 1),
        abi::store_u64(&t, &desc, ARENA_SMALL_DESC_CLASS),
        abi::shift_left_immediate(&t, &t, 4),
        abi::move_immediate(&slots, "Integer", &ARENA_SMALL_PAGE.to_string()),
        abi::unsigned_divide_registers(&slots, &slots, &t),
        abi::store_u64(&slots, &desc, ARENA_SMALL_DESC_SLOTS),
        abi::store_u64(&slots, &desc, ARENA_SMALL_DESC_FREE),
        abi::store_u64(abi::ZERO, &desc, ARENA_SMALL_DESC_HINT),
        abi::add_immediate(&cursor, &desc, ARENA_SMALL_DESC_BITS),
        abi::add_immediate(&end, &desc, ARENA_SMALL_DESC_SIZE as usize),
        abi::label("arena_small_page_clear"),
        abi::store_u64(abi::ZERO, &cursor, 0),
        abi::add_immediate(&cursor, &cursor, 8),
        abi::compare_registers(&cursor, &end),
        abi::branch_lo("arena_small_page_clear"),
        // Tail: word slots >> 6 gets ~((1 << (slots & 63)) - 1) when slots & 63 != 0.
        abi::move_immediate(&t, "Integer", "63"),
        abi::and_registers(&t, &slots, &t),
        abi::compare_immediate(&t, "0"),
        abi::branch_eq("arena_small_page_tail_done"),
        abi::move_immediate(&u, "Integer", "1"),
        abi::shift_left_variable(&u, &u, &t),
        abi::subtract_immediate(&u, &u, 1),
        abi::bitwise_not(&u, &u),
        abi::shift_right_immediate(&t, &slots, 6),
        abi::shift_left_immediate(&t, &t, 3),
        abi::add_registers(&t, &desc, &t),
        abi::store_u64(&u, &t, ARENA_SMALL_DESC_BITS),
        abi::label("arena_small_page_tail_done"),
    ]);
    // Push it on class k's list.
    let head = vregs.next();
    let old = vregs.next();
    emit_small_head(&head, &k, &mut instructions);
    instructions.extend([
        abi::load_u64(&old, &head, 0),
        abi::store_u64(&old, &desc, ARENA_SMALL_DESC_NEXT),
        abi::store_u64(abi::ZERO, &desc, ARENA_SMALL_DESC_PREV),
        abi::store_u64(&p1, &head, 0),
        abi::compare_immediate(&old, "0"),
        abi::branch_eq("arena_small_page_pushed"),
        abi::subtract_immediate(&old, &old, 1),
    ]);
    emit_desc_address(&t, &old, &u, &mut instructions);
    instructions.extend([
        abi::store_u64(&p1, &t, ARENA_SMALL_DESC_PREV),
        abi::label("arena_small_page_pushed"),
    ]);
    if let Some(slot) = &dbg_slot {
        emit_debug_arena_bump(
            symbol,
            slot,
            COUNTER_SMALL_PAGES,
            "small_pages",
            &mut instructions,
            &mut vregs,
        );
    }
    instructions.extend([
        abi::move_register(abi::return_register(), &p1),
        abi::return_(),
        abi::label(full),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::return_(),
    ]);
    finalize_vreg_helper(
        "runtime.arena_small_page",
        symbol,
        "Integer",
        instructions,
        relocations,
    )
}

/// The small allocation of normalized `size` (16..=2048, alignment ≤ 16): take the
/// lowest free slot of the first page on the class's list (asking
/// `_mfb_arena_small_page` for one when the list is empty), leaving the pointer in
/// `ptr`. Branches to `oom` when no page can be had. `sc` is the scalars base.
pub(crate) fn emit_small_alloc(
    from: &str,
    size: &VirtualRegister,
    sc: &VirtualRegister,
    ptr: &VirtualRegister,
    oom: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
) {
    let k = vregs.next();
    let head = vregs.next();
    let p1 = vregs.next();
    let desc = vregs.next();
    let hint = vregs.next();
    let waddr = vregs.next();
    let w = vregs.next();
    let ones = vregs.next();
    let low = vregs.next();
    let bit = vregs.next();
    let t = vregs.next();
    instructions.extend([
        abi::shift_right_immediate(&k, size, 4),
        abi::subtract_immediate(&k, &k, 1),
    ]);
    emit_small_head(&head, &k, instructions);
    instructions.extend([
        abi::load_u64(&p1, &head, 0),
        abi::compare_immediate(&p1, "0"),
        abi::branch_ne(&format!("{from}_small_have_page")),
        abi::move_register(abi::c_arg(0), &k),
        abi::branch_link(ARENA_SMALL_PAGE_SYMBOL),
        abi::move_register(&p1, abi::return_register()),
        abi::compare_immediate(&p1, "0"),
        abi::branch_eq(oom),
        abi::label(&format!("{from}_small_have_page")),
        abi::subtract_immediate(&t, &p1, 1),
    ]);
    relocations.push(internal_branch(from, ARENA_SMALL_PAGE_SYMBOL));
    emit_desc_address(&desc, &t, &w, instructions);
    // The first word below the hint with a clear bit (the page has a free slot).
    instructions.extend([
        abi::load_u64(&hint, &desc, ARENA_SMALL_DESC_HINT),
        abi::move_immediate(&ones, "Integer", &u64::MAX.to_string()),
        abi::label(&format!("{from}_small_scan")),
        abi::shift_left_immediate(&waddr, &hint, 3),
        abi::add_registers(&waddr, &desc, &waddr),
        abi::load_u64(&w, &waddr, ARENA_SMALL_DESC_BITS),
        abi::compare_registers(&w, &ones),
        abi::branch_ne(&format!("{from}_small_found")),
        abi::add_immediate(&hint, &hint, 1),
        abi::branch(&format!("{from}_small_scan")),
        abi::label(&format!("{from}_small_found")),
        // low = lowest clear bit of w, as a mask; bit = its index.
        abi::bitwise_not(&low, &w),
        abi::subtract_registers(&t, abi::ZERO, &low),
        abi::and_registers(&low, &low, &t),
        abi::count_leading_zeros(&bit, &low),
        abi::move_immediate(&t, "Integer", "63"),
        abi::subtract_registers(&bit, &t, &bit),
        abi::or_registers(&w, &w, &low),
        abi::store_u64(&w, &waddr, ARENA_SMALL_DESC_BITS),
        abi::store_u64(&hint, &desc, ARENA_SMALL_DESC_HINT),
        abi::load_u64(&t, &desc, ARENA_SMALL_DESC_FREE),
        abi::subtract_immediate(&t, &t, 1),
        abi::store_u64(&t, &desc, ARENA_SMALL_DESC_FREE),
        abi::compare_immediate(&t, "0"),
        abi::branch_ne(&format!("{from}_small_listed")),
        // Full: it leaves the class list (it is the head).
        abi::load_u64(&t, &desc, ARENA_SMALL_DESC_NEXT),
        abi::store_u64(&t, &head, 0),
        abi::store_u64(abi::ZERO, &desc, ARENA_SMALL_DESC_NEXT),
        abi::compare_immediate(&t, "0"),
        abi::branch_eq(&format!("{from}_small_listed")),
        abi::subtract_immediate(&t, &t, 1),
    ]);
    emit_desc_address(&low, &t, &w, instructions);
    instructions.extend([
        abi::store_u64(abi::ZERO, &low, ARENA_SMALL_DESC_PREV),
        abi::label(&format!("{from}_small_listed")),
        // ptr = pages + (p1 - 1) × 64 KiB + (hint × 64 + bit) × size.
        abi::shift_left_immediate(&hint, &hint, 6),
        abi::add_registers(&hint, &hint, &bit),
        abi::multiply_registers(&hint, &hint, size),
        abi::subtract_immediate(&t, &p1, 1),
        abi::shift_left_immediate(&t, &t, ARENA_SMALL_PAGE.trailing_zeros() as u8),
        abi::load_u64(ptr, sc, ARENA_SC_SMALL_PAGES),
        abi::add_registers(ptr, ptr, &t),
        abi::add_registers(ptr, ptr, &hint),
    ]);
}

/// The small free of `ptr` (inside the small pages): clear its slot bit, zero the
/// slot, and move the page between lists as its free count crosses 1 or reaches the
/// page's slot count. A pointer that is not a handed-out slot (a slot already free,
/// an empty page, a misaligned address) branches to `skip` having changed nothing.
/// Leaves the slot size in `freed`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_small_free(
    from: &str,
    ptr: &VirtualRegister,
    sc: &VirtualRegister,
    freed: &VirtualRegister,
    skip: &str,
    debug_slot: &Option<VirtualRegister>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
) {
    let page = vregs.next();
    let desc = vregs.next();
    let cls = vregs.next();
    let off = vregs.next();
    let slot = vregs.next();
    let mask = vregs.next();
    let waddr = vregs.next();
    let w = vregs.next();
    let t = vregs.next();
    let u = vregs.next();
    instructions.extend([
        abi::load_u64(&off, sc, ARENA_SC_SMALL_PAGES),
        abi::subtract_registers(&off, ptr, &off),
        abi::shift_right_immediate(&page, &off, ARENA_SMALL_PAGE.trailing_zeros() as u8),
    ]);
    emit_desc_address(&desc, &page, &t, instructions);
    instructions.extend([
        abi::load_u64(&cls, &desc, ARENA_SMALL_DESC_CLASS),
        abi::compare_immediate(&cls, "0"),
        abi::branch_eq(skip),
        abi::shift_left_immediate(freed, &cls, 4),
        abi::move_immediate(&t, "Integer", &(ARENA_SMALL_PAGE - 1).to_string()),
        abi::and_registers(&off, &off, &t),
        abi::unsigned_divide_registers(&slot, &off, freed),
        abi::multiply_registers(&t, &slot, freed),
        abi::compare_registers(&t, &off),
        abi::branch_ne(skip),
        abi::shift_right_immediate(&waddr, &slot, 6),
        abi::shift_left_immediate(&waddr, &waddr, 3),
        abi::add_registers(&waddr, &desc, &waddr),
        abi::move_immediate(&t, "Integer", "63"),
        abi::and_registers(&t, &slot, &t),
        abi::move_immediate(&mask, "Integer", "1"),
        abi::shift_left_variable(&mask, &mask, &t),
        abi::load_u64(&w, &waddr, ARENA_SMALL_DESC_BITS),
        abi::and_registers(&t, &w, &mask),
        abi::compare_immediate(&t, "0"),
        abi::branch_eq(skip),
        abi::bitwise_not(&mask, &mask),
        abi::and_registers(&w, &w, &mask),
        abi::store_u64(&w, &waddr, ARENA_SMALL_DESC_BITS),
        abi::shift_right_immediate(&t, &slot, 6),
        abi::load_u64(&u, &desc, ARENA_SMALL_DESC_HINT),
        abi::compare_registers(&t, &u),
        abi::branch_hi(&format!("{from}_small_hinted")),
        abi::store_u64(&t, &desc, ARENA_SMALL_DESC_HINT),
        abi::label(&format!("{from}_small_hinted")),
    ]);
    if let Some(slot) = debug_slot {
        emit_debug_arena_bump(
            from,
            slot,
            COUNTER_FILL_FREE_CALLS,
            "fill_free_small",
            instructions,
            vregs,
        );
        emit_debug_arena_add(
            from,
            slot,
            COUNTER_FILL_FREE_BYTES,
            freed,
            "fill_free_small_bytes",
            instructions,
            vregs,
        );
    }
    instructions.extend([
        abi::move_register(abi::c_arg(0), ptr),
        abi::move_register(abi::c_arg(1), freed),
        abi::branch_link(ARENA_FILL_ZERO_SYMBOL),
    ]);
    relocations.push(internal_branch(from, ARENA_FILL_ZERO_SYMBOL));
    // The page's lists: back on its class list at its first free slot; to the empty
    // list when its last slot comes back.
    let head = vregs.next();
    let p1 = vregs.next();
    instructions.extend([
        abi::add_immediate(&p1, &page, 1),
        abi::subtract_immediate(&t, &cls, 1),
    ]);
    emit_small_head(&head, &t, instructions);
    instructions.extend([
        abi::load_u64(&t, &desc, ARENA_SMALL_DESC_FREE),
        abi::add_immediate(&t, &t, 1),
        abi::store_u64(&t, &desc, ARENA_SMALL_DESC_FREE),
        abi::compare_immediate(&t, "1"),
        abi::branch_ne(&format!("{from}_small_not_first")),
        abi::load_u64(&u, &head, 0),
        abi::store_u64(&u, &desc, ARENA_SMALL_DESC_NEXT),
        abi::store_u64(abi::ZERO, &desc, ARENA_SMALL_DESC_PREV),
        abi::store_u64(&p1, &head, 0),
        abi::compare_immediate(&u, "0"),
        abi::branch_eq(&format!("{from}_small_lists_done")),
        abi::subtract_immediate(&u, &u, 1),
    ]);
    emit_desc_address(&w, &u, &t, instructions);
    instructions.extend([
        abi::store_u64(&p1, &w, ARENA_SMALL_DESC_PREV),
        abi::branch(&format!("{from}_small_lists_done")),
        abi::label(&format!("{from}_small_not_first")),
        abi::load_u64(&u, &desc, ARENA_SMALL_DESC_SLOTS),
        abi::compare_registers(&t, &u),
        abi::branch_ne(&format!("{from}_small_lists_done")),
        // Empty: unlink from the class list, then onto the empty list — unless it is
        // the class's only page, which stays, so a class that frees and allocates one
        // slot at a time does not re-initialise a page each time.
        abi::load_u64(&t, &desc, ARENA_SMALL_DESC_PREV),
        abi::load_u64(&u, &desc, ARENA_SMALL_DESC_NEXT),
        abi::or_registers(&w, &t, &u),
        abi::compare_immediate(&w, "0"),
        abi::branch_eq(&format!("{from}_small_lists_done")),
        abi::compare_immediate(&t, "0"),
        abi::branch_ne(&format!("{from}_small_mid")),
        abi::store_u64(&u, &head, 0),
        abi::branch(&format!("{from}_small_next_link")),
        abi::label(&format!("{from}_small_mid")),
        abi::subtract_immediate(&t, &t, 1),
    ]);
    emit_desc_address(&w, &t, &mask, instructions);
    instructions.extend([
        abi::store_u64(&u, &w, ARENA_SMALL_DESC_NEXT),
        abi::add_immediate(&t, &t, 1),
        abi::label(&format!("{from}_small_next_link")),
        abi::compare_immediate(&u, "0"),
        abi::branch_eq(&format!("{from}_small_to_empty")),
        abi::subtract_immediate(&u, &u, 1),
    ]);
    emit_desc_address(&w, &u, &mask, instructions);
    instructions.extend([
        abi::store_u64(&t, &w, ARENA_SMALL_DESC_PREV),
        abi::label(&format!("{from}_small_to_empty")),
        abi::load_u64(&t, sc, ARENA_SC_SMALL_EMPTY),
        abi::store_u64(&t, &desc, ARENA_SMALL_DESC_NEXT),
        abi::store_u64(abi::ZERO, &desc, ARENA_SMALL_DESC_PREV),
        abi::store_u64(abi::ZERO, &desc, ARENA_SMALL_DESC_CLASS),
        abi::store_u64(&p1, sc, ARENA_SC_SMALL_EMPTY),
        // plan-183-E: the arena keeps ARENA_SMALL_EMPTY_HOT empty pages committed; a page
        // past those goes back to the OS (it is all zero, and reads zero when committed
        // again).
        abi::load_u64(&t, sc, ARENA_SC_EMPTY_HOT),
        abi::compare_immediate(&t, &ARENA_SMALL_EMPTY_HOT.to_string()),
        abi::branch_lo(&format!("{from}_small_keep_hot")),
        abi::move_immediate(&t, "Integer", "1"),
        abi::store_u64(&t, &desc, ARENA_SMALL_DESC_COLD),
        abi::subtract_immediate(&t, &p1, 1),
        abi::shift_left_immediate(&t, &t, ARENA_SMALL_PAGE.trailing_zeros() as u8),
        abi::load_u64(&u, sc, ARENA_SC_SMALL_PAGES),
        abi::add_registers(abi::c_arg(0), &u, &t),
        abi::move_immediate(&t, "Integer", &ARENA_SMALL_PAGE.to_string()),
        abi::add_registers(abi::c_arg(1), abi::c_arg(0), &t),
        abi::branch_link(super::decommit::ARENA_DECOMMIT_SYMBOL),
        abi::branch(&format!("{from}_small_lists_done")),
        abi::label(&format!("{from}_small_keep_hot")),
        abi::add_immediate(&t, &t, 1),
        abi::store_u64(&t, sc, ARENA_SC_EMPTY_HOT),
        abi::label(&format!("{from}_small_lists_done")),
    ]);
    relocations.push(internal_branch(
        from,
        super::decommit::ARENA_DECOMMIT_SYMBOL,
    ));
}
