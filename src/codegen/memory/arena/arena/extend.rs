//! plan-183-D: `arena_extend` — growing a chunk where it is.

use super::layout::*;
use super::small::*;
use super::span::*;
use super::*;
use crate::codegen::debug::arena::{COUNTER_EXTEND_CALLS, COUNTER_EXTEND_HITS};
use crate::codegen::runtime::arena_cap::ARENA_COMMIT_GRANULE;

/// `_mfb_arena_extend(ptr, old_size, new_size)`: grow the chunk at `ptr` in place
/// ([`lower_arena_extend`]); 1 when it did, 0 when the caller must move it.
pub(crate) const ARENA_EXTEND_SYMBOL: &str = "_mfb_arena_extend";

/// `arena_extend(x0 = ptr, x1 = old_size, x2 = new_size)` → 1 in the return register
/// when the chunk at `ptr` now holds `new_size` bytes without moving, 0 when it cannot
/// grow where it is and the caller must allocate, copy and free. Both sizes are
/// normalized as `arena_alloc` normalizes them; `old_size` is what the chunk was
/// allocated (or last extended) with.
///
/// * **Small:** only within its slot — a size its class already holds.
/// * **Medium** (under 32 MiB): into the free run that starts at the chunk's end. The
///   medium region grows down, so nothing but a free run lies above a medium chunk. A
///   medium chunk asked for 32 MiB or more is refused: the caller's allocation moves it
///   into the huge region once.
/// * **Huge:** into the free run above it, and on into the huge frontier when the
///   chunk (or that run) ends there — the region grows up, so the newest huge chunk
///   always can, committing as it goes.
///
/// Every byte the chunk gains reads zero, as `arena_alloc`'s do: a free run is zeroed
/// when freed and a fresh commit is the OS's zero pages. The chunk's start record takes
/// its new size, so the free that follows returns all of it. Vreg-allocated; clobbers
/// the caller-saved registers.
pub(crate) fn lower_arena_extend(debug_arena: bool) -> CodeFunction {
    let symbol = ARENA_EXTEND_SYMBOL;
    let mut s = new_span(symbol);
    let sc = s.sc;
    let ptr = s.reg();
    let old = s.reg();
    let new = s.reg();
    let want = s.reg();
    let z = s.reg();
    let rec = s.reg();
    let tag = s.reg();
    let end = s.reg();
    let extra = s.reg();
    let t = s.reg();
    let u = s.reg();
    let is_huge = s.reg();
    let fail = format!("{symbol}_fail");
    let hit = format!("{symbol}_hit");
    let grown = format!("{symbol}_grown");
    let same = format!("{symbol}_same");
    let not_15 = (!(ARENA_MIN_CHUNK - 1)).to_string();
    s.ins.extend([
        abi::move_register(&ptr, abi::c_arg(0)),
        abi::move_register(&old, abi::c_arg(1)),
        abi::move_register(&new, abi::c_arg(2)),
    ]);
    // Normalize both sizes exactly as `arena_alloc` and `arena_free` do.
    for (reg, name) in [(&old, "old"), (&new, "new")] {
        let nonzero = format!("{symbol}_{name}_nonzero");
        s.ins.extend([
            abi::move_immediate(&t, "Integer", &(u64::MAX - ARENA_MIN_CHUNK).to_string()),
            abi::compare_registers(reg, &t),
            abi::branch_hi(&fail),
            abi::compare_immediate(reg, "0"),
            abi::branch_ne(&nonzero),
            abi::move_immediate(reg, "Integer", "1"),
            abi::label(&nonzero),
            abi::add_immediate(reg, reg, (ARENA_MIN_CHUNK - 1) as usize),
            abi::move_immediate(&t, "Integer", &not_15),
            abi::and_registers(reg, reg, &t),
        ]);
    }
    let mut dbg_slot: Option<VirtualRegister> = None;
    if debug_arena {
        let slot = s.reg();
        emit_debug_arena_slot(symbol, &slot, &mut s.ins, &mut s.rel, &mut s.v);
        emit_debug_arena_bump(
            symbol,
            &slot,
            COUNTER_EXTEND_CALLS,
            "extend_calls",
            &mut s.ins,
            &mut s.v,
        );
        dbg_slot = Some(slot);
    }
    // Not a growth (callers only ask to grow): the chunk already holds it.
    s.ins.extend([
        abi::compare_registers(&new, &old),
        abi::branch_ls(&same),
        abi::load_u64(&t, &sc, ARENA_SC_SMALL_PAGES),
        abi::compare_immediate(&t, "0"),
        abi::branch_eq(&fail),
        abi::compare_registers(&ptr, &t),
        abi::branch_lo(&fail),
        abi::load_u64(&t, &sc, ARENA_SC_SMALL_END),
        abi::compare_registers(&t, &ptr),
        abi::branch_ls(&format!("{symbol}_span")),
    ]);
    // Small: the slot's class size bounds the chunk.
    {
        let page = s.reg();
        let desc = s.reg();
        s.ins.extend([
            abi::load_u64(&t, &sc, ARENA_SC_SMALL_PAGES),
            abi::subtract_registers(&t, &ptr, &t),
            abi::shift_right_immediate(&page, &t, ARENA_SMALL_PAGE.trailing_zeros() as u8),
        ]);
        emit_desc_address(&desc, &page, &u, &mut s.ins);
        s.ins.extend([
            abi::load_u64(&t, &desc, ARENA_SMALL_DESC_CLASS),
            abi::shift_left_immediate(&t, &t, 4),
            abi::compare_registers(&new, &t),
            abi::branch_ls(&hit),
            abi::branch(&fail),
        ]);
    }
    // Medium and huge: the chunk's true size from its start record, which must name
    // `ptr` (a pointer into a chunk, or a freed one, is refused).
    s.ins.extend([
        abi::label(&format!("{symbol}_span")),
        abi::load_u64(&t, &sc, ARENA_SC_SLICE_TOP),
        abi::compare_registers(&t, &ptr),
        abi::branch_ls(&fail),
    ]);
    s.record(&rec, &ptr);
    s.ins.extend([
        abi::load_u64(&tag, &rec, 0),
        abi::move_immediate(&t, "Integer", "1"),
        abi::and_registers(&t, &tag, &t),
        abi::compare_immediate(&t, "0"),
        abi::branch_eq(&fail),
        abi::shift_right_immediate(&t, &tag, 48),
        abi::move_immediate(&u, "Integer", "127"),
        abi::and_registers(&t, &t, &u),
        abi::shift_right_immediate(&end, &ptr, 4),
        abi::and_registers(&end, &end, &u),
        abi::compare_registers(&t, &end),
        abi::branch_ne(&fail),
        abi::move_immediate(&u, "Integer", &(((1u64 << 48) - 1) & !15).to_string()),
        abi::and_registers(&z, &tag, &u),
        abi::move_register(&want, &new),
        abi::move_immediate(&is_huge, "Integer", "0"),
        abi::load_u64(&t, &sc, ARENA_SC_HUGE_FRONT),
        abi::compare_registers(&t, &ptr),
        abi::branch_ls(&format!("{symbol}_medium")),
        abi::move_immediate(&is_huge, "Integer", "1"),
    ]);
    // Huge chunks are whole pages.
    emit_round_up(&want, &new, ARENA_HUGE_PAGE, &t, &mut s.ins);
    s.ins.extend([
        abi::compare_registers(&want, &new),
        abi::branch_lo(&fail),
        abi::branch(&format!("{symbol}_sized")),
        abi::label(&format!("{symbol}_medium")),
        // The tier crossing: a medium chunk does not grow into huge sizes.
        abi::move_immediate(&t, "Integer", &ARENA_HUGE_MIN.to_string()),
        abi::compare_registers(&t, &want),
        abi::branch_ls(&fail),
        abi::label(&format!("{symbol}_sized")),
        abi::compare_registers(&want, &z),
        abi::branch_ls(&hit),
        abi::add_registers(&end, &ptr, &z),
        abi::subtract_registers(&extra, &want, &z),
    ]);
    // A free run starting at `end`, inside the chunk's region: huge below its
    // frontier, medium below the slice top.
    let rn = s.reg();
    let ridx = s.reg();
    let rlen = s.reg();
    let rrec = s.reg();
    let head = s.reg();
    let no_run = s.label("no_run");
    let medium_hi = s.label("medium_hi");
    let have_hi = s.label("have_hi");
    s.ins.extend([
        abi::compare_immediate(&is_huge, "0"),
        abi::branch_eq(&medium_hi),
        abi::load_u64(&t, &sc, ARENA_SC_HUGE_FRONT),
        abi::branch(&have_hi),
        abi::label(&medium_hi),
        abi::load_u64(&t, &sc, ARENA_SC_SLICE_TOP),
        abi::label(&have_hi),
        abi::compare_registers(&t, &end),
        abi::branch_ls(&no_run),
    ]);
    s.record(&rrec, &end);
    s.ins.extend([
        abi::load_u64(&t, &rrec, 0),
        abi::move_immediate(&u, "Integer", "3"),
        abi::and_registers(&u, &t, &u),
        abi::compare_immediate(&u, "2"),
        abi::branch_ne(&no_run),
        abi::shift_right_immediate(&ridx, &t, 2),
    ]);
    s.node(&rn, &ridx);
    let short = s.label("run_short");
    let whole = s.label("run_whole");
    let rflags = s.reg();
    let run_end = s.reg();
    let min = s.reg();
    s.ins.extend([
        abi::load_u64(&t, &rn, ARENA_NODE_START),
        abi::compare_registers(&t, &end),
        abi::branch_ne(&no_run),
    ]);
    s.run_len(&rlen, &rn);
    s.run_flags(&rflags, &rn);
    let min_medium = s.label("min_medium");
    let min_set = s.label("min_set");
    s.ins.extend([
        abi::add_registers(&run_end, &end, &rlen),
        abi::compare_immediate(&is_huge, "0"),
        abi::branch_eq(&min_medium),
        abi::move_immediate(&min, "Integer", &HUGE_MIN_RUN.to_string()),
        abi::branch(&min_set),
        abi::label(&min_medium),
        abi::move_immediate(&min, "Integer", &MEDIUM_MIN_RUN.to_string()),
        abi::label(&min_set),
        abi::compare_registers(&rlen, &extra),
        abi::branch_lo(&short),
    ]);
    // The run holds the growth: take `extra` from its low end (all of it when the rest
    // would be too small to be a run). plan-183-E: a cold run commits the pages the
    // growth covers first, before anything changes.
    {
        let cend = s.reg();
        let all = s.label("take_all_end");
        let have = s.label("take_have_end");
        s.ins.extend([
            abi::subtract_registers(&t, &rlen, &extra),
            abi::compare_registers(&t, &min),
            abi::branch_lo(&all),
            abi::add_registers(&cend, &end, &extra),
            abi::branch(&have),
            abi::label(&all),
            abi::move_register(&cend, &run_end),
            abi::label(&have),
        ]);
        s.commit_cold_part(&rflags, &end, &cend, &run_end, &fail);
    }
    s.head(&head, &rlen, &is_huge);
    s.unlink(&head, &rn);
    s.ins.extend([
        abi::store_u64(abi::ZERO, &rrec, 0),
        abi::subtract_registers(&t, &rlen, &extra),
    ]);
    s.ins.extend([
        abi::compare_registers(&t, &min),
        abi::branch_lo(&whole),
        // The rest stays a run: [end + extra, end + rlen), the same node and end record,
        // with the run's flags (a cold run's rest stays cold).
        abi::add_registers(&u, &end, &extra),
        abi::store_u64(&u, &rn, ARENA_NODE_START),
        abi::or_registers(&u, &t, &rflags),
        abi::store_u64(&u, &rn, ARENA_NODE_LEN),
        abi::add_registers(&u, &end, &extra),
    ]);
    s.record(&rrec, &u);
    s.ins.extend([
        abi::shift_left_immediate(&u, &ridx, 2),
        abi::add_immediate(&u, &u, 2),
        abi::store_u64(&u, &rrec, 0),
    ]);
    s.head(&head, &t, &is_huge);
    s.push(&head, &ridx, &rn);
    s.ins.extend([abi::branch(&grown), abi::label(&whole)]);
    // A rest too small to be a run stays in the chunk: the chunk takes the whole run.
    s.ins.extend([
        abi::add_registers(&want, &z, &rlen),
        abi::add_registers(&t, &end, &rlen),
        abi::subtract_immediate(&t, &t, 16),
    ]);
    s.record(&u, &t);
    s.ins.push(abi::store_u64(abi::ZERO, &u, 8));
    s.drop_node(&ridx, &rn);
    s.ins.extend([abi::branch(&grown), abi::label(&short)]);
    // Too short. A huge run that reaches the frontier still serves, with the frontier
    // advanced past it (committing first, so a refusal changes nothing else).
    s.ins.extend([
        abi::compare_immediate(&is_huge, "0"),
        abi::branch_eq(&fail),
        abi::add_registers(&t, &end, &rlen),
        abi::load_u64(&u, &sc, ARENA_SC_HUGE_FRONT),
        abi::compare_registers(&t, &u),
        abi::branch_ne(&fail),
    ]);
    emit_huge_front_advance(&mut s, &ptr, &want, &fail);
    s.commit_cold_part(&rflags, &end, &run_end, &run_end, &fail);
    s.head(&head, &rlen, &is_huge);
    s.unlink(&head, &rn);
    s.ins.extend([
        abi::store_u64(abi::ZERO, &rrec, 0),
        abi::add_registers(&t, &end, &rlen),
        abi::subtract_immediate(&t, &t, 16),
    ]);
    s.record(&u, &t);
    s.ins.push(abi::store_u64(abi::ZERO, &u, 8));
    s.drop_node(&ridx, &rn);
    s.ins.extend([abi::branch(&grown), abi::label(&no_run)]);
    // No run: the huge chunk at the frontier advances it.
    s.ins.extend([
        abi::compare_immediate(&is_huge, "0"),
        abi::branch_eq(&fail),
        abi::load_u64(&t, &sc, ARENA_SC_HUGE_FRONT),
        abi::compare_registers(&end, &t),
        abi::branch_ne(&fail),
    ]);
    emit_huge_front_advance(&mut s, &ptr, &want, &fail);
    s.ins.push(abi::label(&grown));
    s.tag_chunk(&ptr, &want);
    s.ins.push(abi::label(&hit));
    if let Some(slot) = &dbg_slot {
        emit_debug_arena_bump(
            symbol,
            slot,
            COUNTER_EXTEND_HITS,
            "extend_hits",
            &mut s.ins,
            &mut s.v,
        );
        s.ins.push(abi::subtract_registers(&t, &new, &old));
        emit_debug_arena_live_add(symbol, slot, &t, "extend_live", &mut s.ins, &mut s.v);
    }
    s.ins.extend([
        abi::label(&same),
        abi::move_immediate(abi::return_register(), "Integer", "1"),
        abi::return_(),
        abi::label(&fail),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::return_(),
    ]);
    finalize_vreg_helper("runtime.arena_extend", symbol, "Integer", s.ins, s.rel)
}

/// Advance the huge frontier to `ptr + want` (the chunk at `ptr` ends at the frontier,
/// directly or through a free run that does), committing the data and its tag records
/// as `_mfb_arena_huge_alloc`'s frontier path does. Branches to `fail`, changing
/// nothing but the commit edges, when the huge and medium regions would meet.
fn emit_huge_front_advance(
    s: &mut Span<'_>,
    ptr: &VirtualRegister,
    want: &VirtualRegister,
    fail: &str,
) {
    let sc = s.sc;
    let end = s.reg();
    let limit = s.reg();
    let t = s.reg();
    let need = s.reg();
    s.ins.extend([
        abi::add_registers(&end, ptr, want),
        abi::compare_registers(&end, ptr),
        abi::branch_lo(fail),
        abi::load_u64(&limit, &sc, ARENA_SC_MEDIUM_FRONT),
        abi::compare_registers(&end, &limit),
        abi::branch_hi(fail),
        abi::load_u64(&limit, &sc, ARENA_SC_MEDIUM_COMMIT),
    ]);
    let from = s.from;
    let data = s.label("front_data");
    emit_ensure_up(
        from,
        &data,
        &sc,
        ARENA_SC_HUGE_COMMIT,
        &end,
        &limit,
        ARENA_COMMIT_STEP,
        fail,
        &mut s.ins,
        &mut s.rel,
        &mut s.v,
    );
    s.ins.push(abi::subtract_immediate(&t, &end, 16));
    s.record(&need, &t);
    s.ins.extend([
        abi::add_immediate(&need, &need, ARENA_TAG_RECORD_SIZE as usize),
        abi::load_u64(&limit, &sc, ARENA_SC_TAGS_HIGH),
    ]);
    let tags = s.label("front_tags");
    emit_ensure_up(
        from,
        &tags,
        &sc,
        ARENA_SC_TAGS_LOW,
        &need,
        &limit,
        ARENA_COMMIT_GRANULE,
        fail,
        &mut s.ins,
        &mut s.rel,
        &mut s.v,
    );
    s.ins.push(abi::store_u64(&end, &sc, ARENA_SC_HUGE_FRONT));
}
