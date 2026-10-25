//! plan-183-E: handing committed memory the program stopped using back to the OS.

use super::layout::*;
use super::span::*;
use super::*;

/// `_mfb_arena_decommit(lo, hi)`: decommit the whole pages `[lo, hi)` (both multiples of
/// [`ARENA_DECOMMIT_PAGE`]; nothing when `hi <= lo`), give the bytes back to the arena
/// memory cap and take them off the arena's committed bytes. The pages read zero when
/// committed again, and fault until then. Returns nothing.
pub(crate) const ARENA_DECOMMIT_SYMBOL: &str = "_mfb_arena_decommit";
/// The unit memory goes back to the OS in: 16 KiB, a whole page on every target
/// (macOS AArch64's pages are 16 KiB, the others' 4 KiB).
pub(crate) const ARENA_DECOMMIT_PAGE: u64 = ARENA_HUGE_PAGE;

/// See [`ARENA_DECOMMIT_SYMBOL`].
pub(crate) fn lower_arena_decommit(
    platform: &dyn CodegenPlatform,
    debug_arena: bool,
    arena_cap: bool,
    platform_imports: &std::collections::HashMap<String, String>,
) -> Result<CodeFunction, String> {
    let symbol = ARENA_DECOMMIT_SYMBOL;
    let mut vregs = Vregs::new();
    let lo = vregs.next();
    let hi = vregs.next();
    let len = vregs.next();
    let sc = vregs.next();
    let total = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&lo, abi::c_arg(0)),
        abi::move_register(&hi, abi::c_arg(1)),
        abi::compare_registers(&hi, &lo),
        abi::branch_ls("arena_decommit_none"),
        abi::subtract_registers(&len, &hi, &lo),
    ];
    let mut relocations = Vec::new();
    platform.emit_arena_vm(
        ArenaVm::Decommit,
        (&lo).into(),
        (&len).into(),
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    if arena_cap {
        crate::codegen::runtime::arena_cap::emit_decommit_refund(
            &mut crate::codegen::runtime::arena_cap::CapSite {
                symbol,
                platform,
                platform_imports,
                instructions: &mut instructions,
                relocations: &mut relocations,
            },
            &len,
            &mut vregs,
        )?;
    }
    emit_scalars(&sc, &mut instructions);
    instructions.extend([
        abi::load_u64(&total, &sc, ARENA_SC_COMMITTED),
        abi::subtract_registers(&total, &total, &len),
        abi::store_u64(&total, &sc, ARENA_SC_COMMITTED),
    ]);
    if debug_arena {
        let slot = vregs.next();
        emit_debug_arena_slot(
            symbol,
            &slot,
            &mut instructions,
            &mut relocations,
            &mut vregs,
        );
        emit_debug_arena_bump(
            symbol,
            &slot,
            COUNTER_DECOMMITS,
            "decommits",
            &mut instructions,
            &mut vregs,
        );
        emit_debug_arena_add(
            symbol,
            &slot,
            COUNTER_DECOMMITTED_BYTES,
            &len,
            "decommitted_bytes",
            &mut instructions,
            &mut vregs,
        );
        let skip = "arena_decommit_dbg_skip";
        instructions.extend([
            abi::compare_immediate(&slot, "0"),
            abi::branch_eq(skip),
            abi::store_u64(&total, &slot, COUNTER_COMMITTED_BYTES),
            abi::label(skip),
        ]);
    }
    instructions.extend([abi::label("arena_decommit_none"), abi::return_()]);
    Ok(finalize_vreg_helper(
        "runtime.arena_decommit",
        symbol,
        "Nothing",
        instructions,
        relocations,
    ))
}

/// `_mfb_arena_sweep()`: run every `ARENA_SWEEP_EVERY` medium and huge frees (by
/// `arena_free`). Memory the program stopped using goes back to the OS after it has
/// stayed unused across two sweeps — the hysteresis that keeps a run freed and taken
/// again in a loop from being decommitted and recommitted every time:
///
/// * a hot free run of `ARENA_LARGE_FREE` or more on the huge list or a medium class
///   that can hold one is flagged *seen* at its first sweep and made cold at its second
///   (its whole pages decommitted);
/// * a free run at a frontier — ending at the huge frontier, or starting at the medium
///   frontier — is flagged at its first sweep and at its second leaves the region: the
///   frontier moves back over it, and the commit edge back to the next commit step,
///   decommitting what lies past it.
///
/// A run that changes in between (taken from, merged) loses its flag. At most
/// `ARENA_SWEEP_BUDGET` runs are looked at per sweep. Vreg-allocated; clobbers the
/// caller-saved registers.
pub(crate) const ARENA_SWEEP_SYMBOL: &str = "_mfb_arena_sweep";
/// Medium and huge frees between sweeps.
pub(crate) const ARENA_SWEEP_EVERY: u64 = 64;
const ARENA_SWEEP_BUDGET: u64 = 1024;

/// See [`ARENA_SWEEP_SYMBOL`].
pub(crate) fn lower_arena_sweep() -> CodeFunction {
    let symbol = ARENA_SWEEP_SYMBOL;
    let mut s = new_span(symbol);
    let sc = s.sc;
    let done = format!("{symbol}_done");
    let budget = s.reg();
    let headp = s.reg();
    let headend = s.reg();
    let cur = s.reg();
    let n = s.reg();
    let len = s.reg();
    let flags = s.reg();
    let st = s.reg();
    let t = s.reg();
    let u = s.reg();
    s.ins.extend([
        abi::load_u64(&t, &sc, ARENA_SC_SMALL_PAGES),
        abi::compare_immediate(&t, "0"),
        abi::branch_eq(&done),
        abi::move_immediate(&budget, "Integer", &ARENA_SWEEP_BUDGET.to_string()),
    ]);
    // The lists: the medium classes from the one holding ARENA_LARGE_FREE up, then the
    // huge list.
    {
        let k = s.reg();
        let shift = s.reg();
        let sub = s.reg();
        s.ins.push(abi::move_immediate(
            &t,
            "Integer",
            &ARENA_LARGE_FREE.to_string(),
        ));
        let prefix = s.label("sweep_class");
        emit_medium_class_slot(&headp, &t, &k, &shift, &sub, &prefix, &mut s.ins);
        s.ins.extend([
            abi::add_immediate(&headp, &headp, ARENA_MEDIUM_HEAD_BASE_OFFSET),
            abi::add_immediate(
                &headend,
                ARENA_STATE_REGISTER,
                ARENA_MEDIUM_HEAD_BASE_OFFSET + ARENA_MEDIUM_CLASS_COUNT * 8,
            ),
        ]);
    }
    let list = s.label("sweep_list");
    let walk = s.label("sweep_walk");
    let lists_done = s.label("sweep_lists_done");
    let huge_list = s.label("sweep_huge_list");
    let mark = s.label("sweep_mark");
    s.ins.extend([
        abi::label(&list),
        abi::compare_registers(&headp, &headend),
        abi::branch_eq(&huge_list),
        abi::branch_hi(&lists_done),
        abi::load_u64(&cur, &headp, 0),
        abi::add_immediate(&headp, &headp, 8),
        abi::branch(&walk),
        // The huge list last; `headp` then moves past `headend`, which ends the walk.
        abi::label(&huge_list),
        abi::load_u64(&cur, ARENA_STATE_REGISTER, ARENA_HUGE_HEAD_OFFSET),
        abi::add_immediate(&headp, &headp, 8),
        abi::label(&walk),
        abi::compare_immediate(&cur, "0"),
        abi::branch_eq(&list),
        abi::compare_immediate(&budget, "0"),
        abi::branch_eq(&lists_done),
        abi::subtract_immediate(&budget, &budget, 1),
    ]);
    s.node(&n, &cur);
    s.run_len(&len, &n);
    s.run_flags(&flags, &n);
    s.ins.extend([
        abi::load_u64(&cur, &n, ARENA_NODE_NEXT),
        abi::move_immediate(&t, "Integer", &RUN_COLD.to_string()),
        abi::and_registers(&t, &flags, &t),
        abi::compare_immediate(&t, "0"),
        abi::branch_ne(&walk),
        abi::move_immediate(&t, "Integer", &ARENA_LARGE_FREE.to_string()),
        abi::compare_registers(&len, &t),
        abi::branch_lo(&walk),
        abi::move_immediate(&t, "Integer", &RUN_SEEN.to_string()),
        abi::and_registers(&t, &flags, &t),
        abi::compare_immediate(&t, "0"),
        abi::branch_eq(&mark),
        // Seen at the last sweep too: decommit its whole pages, flag it cold (keeping a
        // frontier flag).
        abi::load_u64(&st, &n, ARENA_NODE_START),
        abi::move_immediate(&t, "Integer", &RUN_EDGE_SEEN.to_string()),
        abi::and_registers(&t, &flags, &t),
        abi::add_registers(&t, &t, &len),
        abi::add_immediate(&t, &t, RUN_COLD as usize),
        abi::store_u64(&t, &n, ARENA_NODE_LEN),
        abi::add_registers(&u, &st, &len),
    ]);
    {
        let lo = s.reg();
        let hi = s.reg();
        let w = s.reg();
        emit_round_up(&lo, &st, ARENA_DECOMMIT_PAGE, &w, &mut s.ins);
        emit_round_down(&hi, &u, ARENA_DECOMMIT_PAGE, &w, &mut s.ins);
        s.ins.extend([
            abi::move_register(abi::c_arg(0), &lo),
            abi::move_register(abi::c_arg(1), &hi),
            abi::branch_link(ARENA_DECOMMIT_SYMBOL),
            abi::branch(&walk),
        ]);
        s.rel.push(internal_branch(symbol, ARENA_DECOMMIT_SYMBOL));
    }
    s.ins.extend([
        abi::label(&mark),
        abi::load_u64(&t, &n, ARENA_NODE_LEN),
        abi::add_immediate(&t, &t, RUN_SEEN as usize),
        abi::store_u64(&t, &n, ARENA_NODE_LEN),
        abi::branch(&walk),
        abi::label(&lists_done),
    ]);
    emit_retreat_huge(&mut s);
    emit_retreat_medium(&mut s);
    s.ins.extend([abi::label(&done), abi::return_()]);
    finalize_vreg_helper("runtime.arena_sweep", symbol, "Nothing", s.ins, s.rel)
}

/// A frontier run seen at the last sweep falls through (it retreats); otherwise it is
/// flagged and the code branches to `out`. Its own flag, `RUN_EDGE_SEEN`, so the large-run
/// rule's mark in the same sweep cannot count as the frontier's first sweep.
fn emit_seen_or_mark(s: &mut Span<'_>, n: &VirtualRegister, flags: &VirtualRegister, out: &str) {
    let t = s.reg();
    let seen = s.label("retreat_seen");
    s.ins.extend([
        abi::move_immediate(&t, "Integer", &RUN_EDGE_SEEN.to_string()),
        abi::and_registers(&t, flags, &t),
        abi::compare_immediate(&t, "0"),
        abi::branch_ne(&seen),
        abi::load_u64(&t, n, ARENA_NODE_LEN),
        abi::add_immediate(&t, &t, RUN_EDGE_SEEN as usize),
        abi::store_u64(&t, n, ARENA_NODE_LEN),
        abi::branch(out),
        abi::label(&seen),
    ]);
}

/// The huge frontier retreats over the free run that ends at it (seen twice): the run
/// leaves its list and the tags, `hugeFront` drops to its start, and `hugeCommit` to
/// the commit step above that, decommitting `[newCommit, oldCommit)`. Everything from
/// the frontier to `hugeCommit` must stay committed — the next frontier chunk commits
/// only past it — so for a cold run the edge drops only to `page_up(start)`, the end of
/// its committed start edge, and only `[page_down(end), oldCommit)` (its end edge and
/// the gap above it) is decommitted: its interior already is.
fn emit_retreat_huge(s: &mut Span<'_>) {
    let sc = s.sc;
    let out = s.label("retreat_huge_out");
    let front = s.reg();
    let rec = s.reg();
    let idx = s.reg();
    let n = s.reg();
    let st = s.reg();
    let len = s.reg();
    let flags = s.reg();
    let t = s.reg();
    let head = s.reg();
    let is_huge = s.reg();
    s.ins.extend([
        abi::load_u64(&front, &sc, ARENA_SC_HUGE_FRONT),
        abi::load_u64(&t, &sc, ARENA_SC_SPAN_BASE),
        abi::compare_registers(&front, &t),
        abi::branch_ls(&out),
        abi::subtract_immediate(&t, &front, 16),
    ]);
    s.record(&rec, &t);
    s.ins.extend([
        abi::load_u64(&idx, &rec, 8),
        abi::compare_immediate(&idx, "0"),
        abi::branch_eq(&out),
    ]);
    s.node(&n, &idx);
    s.ins.push(abi::load_u64(&st, &n, ARENA_NODE_START));
    s.run_len(&len, &n);
    s.run_flags(&flags, &n);
    s.ins.extend([
        abi::add_registers(&t, &st, &len),
        abi::compare_registers(&t, &front),
        abi::branch_ne(&out),
    ]);
    emit_seen_or_mark(s, &n, &flags, &out);
    s.ins.push(abi::move_immediate(&is_huge, "Integer", "1"));
    s.head(&head, &len, &is_huge);
    s.unlink(&head, &n);
    let srec = s.reg();
    s.record(&srec, &st);
    s.ins.extend([
        abi::store_u64(abi::ZERO, &srec, 0),
        abi::store_u64(abi::ZERO, &rec, 8),
    ]);
    s.drop_node(&idx, &n);
    s.ins.push(abi::store_u64(&st, &sc, ARENA_SC_HUGE_FRONT));
    // The new edge `a` and the decommit's start `lo`: roundUp(st, step) for a hot run;
    // for a cold one page_up(st) and max(page_up(st), page_down(end)).
    let a = s.reg();
    let lo = s.reg();
    let b = s.reg();
    let w = s.reg();
    let hot = s.label("retreat_huge_hot");
    let lowered = s.label("retreat_huge_lowered");
    let edged = s.label("retreat_huge_edged");
    emit_round_up(&a, &st, ARENA_COMMIT_STEP, &w, &mut s.ins);
    s.ins.extend([
        abi::move_register(&lo, &a),
        abi::move_immediate(&t, "Integer", &RUN_COLD.to_string()),
        abi::and_registers(&t, &flags, &t),
        abi::compare_immediate(&t, "0"),
        abi::branch_eq(&hot),
    ]);
    emit_round_up(&a, &st, ARENA_DECOMMIT_PAGE, &w, &mut s.ins);
    s.ins.extend([
        abi::move_register(&lo, &a),
        abi::add_registers(&t, &st, &len),
    ]);
    emit_round_down(&t, &t, ARENA_DECOMMIT_PAGE, &w, &mut s.ins);
    s.ins.extend([
        abi::compare_registers(&t, &lo),
        abi::branch_ls(&edged),
        abi::move_register(&lo, &t),
        abi::label(&edged),
        abi::label(&hot),
        abi::load_u64(&b, &sc, ARENA_SC_HUGE_COMMIT),
        abi::move_register(abi::c_arg(0), &lo),
        abi::move_register(abi::c_arg(1), &b),
        abi::branch_link(ARENA_DECOMMIT_SYMBOL),
        abi::load_u64(&b, &sc, ARENA_SC_HUGE_COMMIT),
        abi::compare_registers(&b, &a),
        abi::branch_ls(&lowered),
        abi::store_u64(&a, &sc, ARENA_SC_HUGE_COMMIT),
        abi::label(&lowered),
        abi::label(&out),
    ]);
    s.rel.push(internal_branch(s.from, ARENA_DECOMMIT_SYMBOL));
}

/// The medium frontier retreats over the free run that starts at it (seen twice), its
/// mirror image: `mediumFront` rises to the run's end, `mediumCommit` to the commit step
/// below that, decommitting `[oldCommit, newCommit)`. For a cold run the edge rises only
/// to `page_down(end)`, the start of its committed end edge, and only
/// `[oldCommit, min(page_down(end), page_up(start)))` is decommitted.
fn emit_retreat_medium(s: &mut Span<'_>) {
    let sc = s.sc;
    let out = s.label("retreat_medium_out");
    let front = s.reg();
    let rec = s.reg();
    let idx = s.reg();
    let n = s.reg();
    let len = s.reg();
    let flags = s.reg();
    let e = s.reg();
    let t = s.reg();
    let u = s.reg();
    let head = s.reg();
    let is_huge = s.reg();
    s.ins.extend([
        abi::load_u64(&front, &sc, ARENA_SC_MEDIUM_FRONT),
        abi::load_u64(&t, &sc, ARENA_SC_SLICE_TOP),
        abi::compare_registers(&t, &front),
        abi::branch_ls(&out),
    ]);
    s.record(&rec, &front);
    s.ins.extend([
        abi::load_u64(&t, &rec, 0),
        abi::move_immediate(&u, "Integer", "3"),
        abi::and_registers(&u, &t, &u),
        abi::compare_immediate(&u, "2"),
        abi::branch_ne(&out),
        abi::shift_right_immediate(&idx, &t, 2),
    ]);
    s.node(&n, &idx);
    s.ins.extend([
        abi::load_u64(&t, &n, ARENA_NODE_START),
        abi::compare_registers(&t, &front),
        abi::branch_ne(&out),
    ]);
    s.run_len(&len, &n);
    s.run_flags(&flags, &n);
    emit_seen_or_mark(s, &n, &flags, &out);
    s.ins.extend([
        abi::move_immediate(&is_huge, "Integer", "0"),
        abi::add_registers(&e, &front, &len),
    ]);
    s.head(&head, &len, &is_huge);
    s.unlink(&head, &n);
    let erec = s.reg();
    s.ins.push(abi::subtract_immediate(&t, &e, 16));
    s.record(&erec, &t);
    s.ins.extend([
        abi::store_u64(abi::ZERO, &rec, 0),
        abi::store_u64(abi::ZERO, &erec, 8),
    ]);
    s.drop_node(&idx, &n);
    s.ins.push(abi::store_u64(&e, &sc, ARENA_SC_MEDIUM_FRONT));
    // The new edge `a` and the decommit's end `hi`: roundDown(e, step) for a hot run;
    // for a cold one page_down(e) and min(page_down(e), page_up(front)).
    let a = s.reg();
    let hi = s.reg();
    let b = s.reg();
    let w = s.reg();
    let hot = s.label("retreat_medium_hot");
    let raised = s.label("retreat_medium_raised");
    let edged = s.label("retreat_medium_edged");
    emit_round_down(&a, &e, ARENA_COMMIT_STEP, &w, &mut s.ins);
    s.ins.extend([
        abi::move_register(&hi, &a),
        abi::move_immediate(&t, "Integer", &RUN_COLD.to_string()),
        abi::and_registers(&t, &flags, &t),
        abi::compare_immediate(&t, "0"),
        abi::branch_eq(&hot),
    ]);
    emit_round_down(&a, &e, ARENA_DECOMMIT_PAGE, &w, &mut s.ins);
    s.ins.push(abi::move_register(&hi, &a));
    emit_round_up(&t, &front, ARENA_DECOMMIT_PAGE, &w, &mut s.ins);
    s.ins.extend([
        abi::compare_registers(&t, &hi),
        abi::branch_hi(&edged),
        abi::move_register(&hi, &t),
        abi::label(&edged),
        abi::label(&hot),
        abi::load_u64(&b, &sc, ARENA_SC_MEDIUM_COMMIT),
        abi::move_register(abi::c_arg(0), &b),
        abi::move_register(abi::c_arg(1), &hi),
        abi::branch_link(ARENA_DECOMMIT_SYMBOL),
        abi::load_u64(&b, &sc, ARENA_SC_MEDIUM_COMMIT),
        abi::compare_registers(&a, &b),
        abi::branch_ls(&raised),
        abi::store_u64(&a, &sc, ARENA_SC_MEDIUM_COMMIT),
        abi::label(&raised),
        abi::label(&out),
    ]);
    s.rel.push(internal_branch(s.from, ARENA_DECOMMIT_SYMBOL));
}
