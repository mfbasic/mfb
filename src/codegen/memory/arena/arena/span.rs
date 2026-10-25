//! plan-183-C, plan-183-D: the huge and medium regions — one span between the small
//! region and the slice top, huge growing up from its base and medium down from the
//! top.
//!
//! Every chunk and every free run in the span is larger than the 2 KiB block a tag
//! record covers, so at most one of them starts, and at most one free run ends, in
//! any block. That is what makes the tags exact: a free finds its chunk's size in
//! the start record of its block, its left neighbour as the free run whose end record
//! lies in the block before its first byte, and its right neighbour as the free run
//! whose start record lies in the block of its end — each checked against the run's
//! node, all O(1), and none of it in free memory.
//!
//! Free runs live in the node pool and on doubly linked lists: 64 medium size classes
//! (best fit within a class, else any run of a higher class) and one huge list. A free
//! merges with its free neighbours at once. Allocation takes a run's low end, so the
//! rest stays above the new chunk, where `arena_extend` (`extend.rs`) grows into.

use super::decommit::{ARENA_DECOMMIT_PAGE, ARENA_DECOMMIT_SYMBOL};
use super::layout::*;
use super::*;
use crate::codegen::runtime::arena_cap::ARENA_COMMIT_GRANULE;

/// `_mfb_arena_medium_alloc(size)` / `_mfb_arena_huge_alloc(size)`: a chunk of exactly
/// `size` bytes or a little more (a remainder too small to be a run stays in the
/// chunk), in the return register (0 on failure), with 1 in the second result
/// register when it came from the region's frontier and 0 when from a free run.
pub(crate) const ARENA_MEDIUM_ALLOC_SYMBOL: &str = "_mfb_arena_medium_alloc";
pub(crate) const ARENA_HUGE_ALLOC_SYMBOL: &str = "_mfb_arena_huge_alloc";
/// `_mfb_arena_span_free(ptr)`: free the medium or huge chunk at `ptr` and return its
/// size, or 0 when `ptr` is not the start of a chunk handed out (nothing changes).
pub(crate) const ARENA_SPAN_FREE_SYMBOL: &str = "_mfb_arena_span_free";

/// The smallest free run each region keeps: a remainder below it stays in the chunk.
pub(super) const MEDIUM_MIN_RUN: u64 = ARENA_SMALL_MAX + 16;
pub(super) const HUGE_MIN_RUN: u64 = ARENA_HUGE_PAGE;
/// plan-183-E: a run's flags, in the low bits of its node's length word (lengths are
/// multiples of 16). **Cold**: the run's whole pages `[page_up(start),
/// page_down(end))` are decommitted, its partial edge pages committed — so taking any
/// part of it commits the pages that part covers first. **Seen**: the sweep found the
/// run free last time; a hot run seen twice is made cold (`decommit.rs`). A run's rest
/// after a take keeps its flags — those bytes stayed free — and a merged run takes its
/// runs' seen flags: memory freed next to an idle run ages with it, so a chunk taken and
/// freed again and again at a run's edge cannot keep the rest of the run young.
pub(super) const RUN_COLD: u64 = 1;
pub(super) const RUN_SEEN: u64 = 2;
/// The sweep found the run at a frontier last time; seen twice there, it retreats.
pub(super) const RUN_EDGE_SEEN: u64 = 4;
const RUN_FLAGS: u64 = 15;
/// A free of a chunk this large decommits its whole pages instead of zero-filling them.
pub(super) const ARENA_LARGE_FREE: u64 = 4 << 20;

/// One helper's emission state.
pub(super) struct Span<'a> {
    pub(super) from: &'a str,
    pub(super) ins: Vec<CodeInstruction>,
    pub(super) rel: Vec<CodeRelocation>,
    pub(super) v: Vregs,
    pub(super) sc: VirtualRegister,
    labels: usize,
}

impl Span<'_> {
    pub(super) fn label(&mut self, name: &str) -> String {
        self.labels += 1;
        format!("{}_{name}_{}", self.from, self.labels)
    }

    pub(super) fn reg(&mut self) -> VirtualRegister {
        self.v.next()
    }

    /// `dst = nodes + idx × 32`.
    pub(super) fn node(&mut self, dst: &VirtualRegister, idx: &VirtualRegister) {
        let base = self.reg();
        self.ins.extend([
            abi::shift_left_immediate(dst, idx, ARENA_NODE_SIZE.trailing_zeros() as u8),
            abi::load_u64(&base, &self.sc, ARENA_SC_NODES),
            abi::add_registers(dst, dst, &base),
        ]);
    }

    /// `dst = tags + ((addr - mediumBase) >> 11) × 16`.
    pub(super) fn record(&mut self, dst: &VirtualRegister, addr: &VirtualRegister) {
        let base = self.reg();
        self.ins.extend([
            abi::load_u64(&base, &self.sc, ARENA_SC_SPAN_BASE),
            abi::subtract_registers(dst, addr, &base),
            abi::shift_right_immediate(dst, dst, ARENA_TAG_BLOCK_SHIFT as u8),
            abi::shift_left_immediate(dst, dst, 4),
            abi::load_u64(&base, &self.sc, ARENA_SC_TAGS),
            abi::add_registers(dst, dst, &base),
        ]);
    }

    /// `dst = &list head` for a run of `size` bytes: the huge list when `huge` is
    /// non-zero, else the medium class of `size`.
    pub(super) fn head(
        &mut self,
        dst: &VirtualRegister,
        size: &VirtualRegister,
        huge: &VirtualRegister,
    ) {
        let medium = self.label("head_medium");
        let done = self.label("head_done");
        let (k, shift, sub) = (self.reg(), self.reg(), self.reg());
        self.ins.extend([
            abi::compare_immediate(huge, "0"),
            abi::branch_eq(&medium),
            abi::add_immediate(dst, ARENA_STATE_REGISTER, ARENA_HUGE_HEAD_OFFSET),
            abi::branch(&done),
            abi::label(&medium),
        ]);
        let prefix = self.label("class");
        emit_medium_class_slot(dst, size, &k, &shift, &sub, &prefix, &mut self.ins);
        self.ins.extend([
            abi::add_immediate(dst, dst, ARENA_MEDIUM_HEAD_BASE_OFFSET),
            abi::label(&done),
        ]);
    }

    /// Push node `idx` (address `n`) on the list at `head`.
    pub(super) fn push(
        &mut self,
        head: &VirtualRegister,
        idx: &VirtualRegister,
        n: &VirtualRegister,
    ) {
        let old = self.reg();
        let t = self.reg();
        let done = self.label("pushed");
        self.ins.extend([
            abi::load_u64(&old, head, 0),
            abi::store_u64(&old, n, ARENA_NODE_NEXT),
            abi::store_u64(abi::ZERO, n, ARENA_NODE_PREV),
            abi::store_u64(idx, head, 0),
            abi::compare_immediate(&old, "0"),
            abi::branch_eq(&done),
        ]);
        self.node(&t, &old);
        self.ins
            .extend([abi::store_u64(idx, &t, ARENA_NODE_PREV), abi::label(&done)]);
    }

    /// Unlink node `n` from the list at `head`.
    pub(super) fn unlink(&mut self, head: &VirtualRegister, n: &VirtualRegister) {
        let next = self.reg();
        let prev = self.reg();
        let t = self.reg();
        let mid = self.label("unlink_mid");
        let link_next = self.label("unlink_next");
        let done = self.label("unlinked");
        self.ins.extend([
            abi::load_u64(&next, n, ARENA_NODE_NEXT),
            abi::load_u64(&prev, n, ARENA_NODE_PREV),
            abi::compare_immediate(&prev, "0"),
            abi::branch_ne(&mid),
            abi::store_u64(&next, head, 0),
            abi::branch(&link_next),
            abi::label(&mid),
        ]);
        self.node(&t, &prev);
        self.ins.extend([
            abi::store_u64(&next, &t, ARENA_NODE_NEXT),
            abi::label(&link_next),
            abi::compare_immediate(&next, "0"),
            abi::branch_eq(&done),
        ]);
        self.node(&t, &next);
        self.ins.extend([
            abi::store_u64(&prev, &t, ARENA_NODE_PREV),
            abi::label(&done),
        ]);
    }

    /// `idx` = a node from the pool (its free list, else a fresh one, committing the
    /// pool as it grows). Branches to `fail` when the pool is exhausted or refused.
    fn take_node(&mut self, idx: &VirtualRegister, fail: &str) {
        let n = self.reg();
        let t = self.reg();
        let fresh = self.label("node_fresh");
        let done = self.label("node_done");
        self.ins.extend([
            abi::load_u64(idx, ARENA_STATE_REGISTER, ARENA_NODE_FREE_OFFSET),
            abi::compare_immediate(idx, "0"),
            abi::branch_eq(&fresh),
        ]);
        self.node(&n, idx);
        self.ins.extend([
            abi::load_u64(&t, &n, ARENA_NODE_NEXT),
            abi::store_u64(&t, ARENA_STATE_REGISTER, ARENA_NODE_FREE_OFFSET),
            abi::branch(&done),
            abi::label(&fresh),
            abi::load_u64(idx, &self.sc, ARENA_SC_NODE_NEXT),
            abi::load_u64(&t, &self.sc, ARENA_SC_NODE_LIMIT),
            abi::compare_registers(idx, &t),
            abi::branch_hi(fail),
            abi::branch_eq(fail),
            abi::add_immediate(&t, idx, 1),
            abi::store_u64(&t, &self.sc, ARENA_SC_NODE_NEXT),
        ]);
        let need = self.reg();
        let ceiling = self.reg();
        self.node(&need, &t);
        self.ins
            .push(abi::load_u64(&ceiling, &self.sc, ARENA_SC_SPAN_BASE));
        let tag = self.label("nodes");
        let sc = self.sc;
        emit_ensure_up(
            self.from,
            &tag,
            &sc,
            ARENA_SC_NODE_COMMIT,
            &need,
            &ceiling,
            ARENA_COMMIT_GRANULE,
            fail,
            &mut self.ins,
            &mut self.rel,
            &mut self.v,
        );
        self.ins.push(abi::label(&done));
    }

    /// Return node `idx` (address `n`) to the pool.
    pub(super) fn drop_node(&mut self, idx: &VirtualRegister, n: &VirtualRegister) {
        let t = self.reg();
        self.ins.extend([
            abi::load_u64(&t, ARENA_STATE_REGISTER, ARENA_NODE_FREE_OFFSET),
            abi::store_u64(&t, n, ARENA_NODE_NEXT),
            abi::store_u64(idx, ARENA_STATE_REGISTER, ARENA_NODE_FREE_OFFSET),
        ]);
    }

    /// `dst` = node `n`'s run length, its flags masked away.
    pub(super) fn run_len(&mut self, dst: &VirtualRegister, n: &VirtualRegister) {
        let t = self.reg();
        self.ins.extend([
            abi::load_u64(dst, n, ARENA_NODE_LEN),
            abi::move_immediate(&t, "Integer", &(!RUN_FLAGS).to_string()),
            abi::and_registers(dst, dst, &t),
        ]);
    }

    /// `dst` = node `n`'s flags.
    pub(super) fn run_flags(&mut self, dst: &VirtualRegister, n: &VirtualRegister) {
        let t = self.reg();
        self.ins.extend([
            abi::load_u64(dst, n, ARENA_NODE_LEN),
            abi::move_immediate(&t, "Integer", &RUN_FLAGS.to_string()),
            abi::and_registers(dst, dst, &t),
        ]);
    }

    /// plan-183-E: before `[start, cend)` is handed out of a run `[start, run_end)` whose
    /// flags are `flags`: when the run is cold, commit the pages the part covers —
    /// `[page_up(start), min(page_up(cend), page_down(run_end)))` — branching to `fail`
    /// (nothing changed) when the commit is refused. The rest of the run keeps the cold
    /// invariant: its whole pages from `page_up(cend)` on stay decommitted.
    pub(super) fn commit_cold_part(
        &mut self,
        flags: &VirtualRegister,
        start: &VirtualRegister,
        cend: &VirtualRegister,
        run_end: &VirtualRegister,
        fail: &str,
    ) {
        let hot = self.label("cold_part_hot");
        let capped = self.label("cold_part_capped");
        let lo = self.reg();
        let hi = self.reg();
        let t = self.reg();
        let u = self.reg();
        self.ins.extend([
            abi::move_immediate(&t, "Integer", &RUN_COLD.to_string()),
            abi::and_registers(&t, flags, &t),
            abi::compare_immediate(&t, "0"),
            abi::branch_eq(&hot),
        ]);
        emit_round_up(&lo, start, ARENA_DECOMMIT_PAGE, &t, &mut self.ins);
        emit_round_up(&hi, cend, ARENA_DECOMMIT_PAGE, &t, &mut self.ins);
        emit_round_down(&u, run_end, ARENA_DECOMMIT_PAGE, &t, &mut self.ins);
        self.ins.extend([
            abi::compare_registers(&hi, &u),
            abi::branch_ls(&capped),
            abi::move_register(&hi, &u),
            abi::label(&capped),
            abi::compare_registers(&hi, &lo),
            abi::branch_ls(&hot),
            abi::move_register(abi::c_arg(0), &lo),
            abi::move_register(abi::c_arg(1), &hi),
            abi::branch_link(ARENA_COMMIT_SYMBOL),
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_ne(fail),
            abi::label(&hot),
        ]);
        self.rel
            .push(internal_branch(self.from, ARENA_COMMIT_SYMBOL));
    }

    /// Write the allocated-chunk start tag for `[start, start + size)`:
    /// `size | (start >> 4 & 127) << 48 | 1`. Bits 55–63 are zero, and every reader
    /// masks them away.
    pub(super) fn tag_chunk(&mut self, start: &VirtualRegister, size: &VirtualRegister) {
        let rec = self.reg();
        let t = self.reg();
        let u = self.reg();
        self.record(&rec, start);
        self.ins.extend([
            abi::shift_right_immediate(&t, start, 4),
            abi::move_immediate(&u, "Integer", "127"),
            abi::and_registers(&t, &t, &u),
            abi::shift_left_immediate(&t, &t, 48),
            abi::or_registers(&t, &t, size),
            abi::add_immediate(&t, &t, 1),
            abi::store_u64(&t, &rec, 0),
        ]);
    }
}

pub(super) fn new_span(from: &str) -> Span<'_> {
    let mut v = Vregs::new();
    let sc = v.next();
    let mut ins = vec![abi::label("entry")];
    emit_scalars(&sc, &mut ins);
    Span {
        from,
        ins,
        rel: Vec::new(),
        v,
        sc,
        labels: 0,
    }
}

/// `_mfb_arena_medium_alloc` (`huge` false) / `_mfb_arena_huge_alloc` (`huge` true).
pub(crate) fn lower_arena_span_alloc(huge: bool) -> CodeFunction {
    let symbol = if huge {
        ARENA_HUGE_ALLOC_SYMBOL
    } else {
        ARENA_MEDIUM_ALLOC_SYMBOL
    };
    let mut s = new_span(symbol);
    let size = s.reg();
    let is_huge = s.reg();
    let head = s.reg();
    let cur = s.reg();
    let best = s.reg();
    let best_size = s.reg();
    let budget = s.reg();
    let n = s.reg();
    let z = s.reg();
    let t = s.reg();
    let start = s.reg();
    let fail = format!("{symbol}_fail");
    let take = format!("{symbol}_take");
    let frontier = format!("{symbol}_frontier");
    s.ins.extend([
        abi::move_register(&size, abi::c_arg(0)),
        abi::move_immediate(&is_huge, "Integer", if huge { "1" } else { "0" }),
    ]);
    s.head(&head, &size, &is_huge);
    // Best fit among the first runs of the list: an exact size at once, else the
    // smallest that fits.
    let scan = s.label("scan");
    let next = s.label("scan_next");
    let scanned = s.label("scanned");
    let better = s.label("better");
    let limit = if huge {
        ARENA_HUGE_SCAN_LIMIT
    } else {
        ARENA_MEDIUM_SCAN_LIMIT
    };
    s.ins.extend([
        abi::load_u64(&cur, &head, 0),
        abi::move_immediate(&best, "Integer", "0"),
        abi::move_immediate(&best_size, "Integer", "0"),
        abi::move_immediate(&budget, "Integer", &limit.to_string()),
        abi::label(&scan),
        abi::compare_immediate(&cur, "0"),
        abi::branch_eq(&scanned),
    ]);
    s.node(&n, &cur);
    s.run_len(&z, &n);
    s.ins.extend([
        abi::compare_registers(&z, &size),
        abi::branch_lo(&next),
        abi::branch_ne(&format!("{symbol}_inexact")),
        abi::move_register(&best, &cur),
        abi::branch(&take),
        abi::label(&format!("{symbol}_inexact")),
        abi::compare_immediate(&best, "0"),
        abi::branch_eq(&better),
        abi::compare_registers(&z, &best_size),
        abi::branch_lo(&better),
        abi::branch(&next),
        abi::label(&better),
        abi::move_register(&best, &cur),
        abi::move_register(&best_size, &z),
        abi::label(&next),
        abi::subtract_immediate(&budget, &budget, 1),
        abi::compare_immediate(&budget, "0"),
        abi::branch_eq(&scanned),
        abi::load_u64(&cur, &n, ARENA_NODE_NEXT),
        abi::branch(&scan),
        abi::label(&scanned),
        abi::compare_immediate(&best, "0"),
        abi::branch_ne(&take),
    ]);
    if !huge {
        // Any run of a higher class fits.
        let up = s.label("up");
        let end = s.reg();
        s.ins.extend([
            abi::add_immediate(
                &end,
                ARENA_STATE_REGISTER,
                ARENA_MEDIUM_HEAD_BASE_OFFSET + ARENA_MEDIUM_CLASS_COUNT * 8,
            ),
            abi::move_register(&t, &head),
            abi::label(&up),
            abi::add_immediate(&t, &t, 8),
            abi::compare_registers(&t, &end),
            abi::branch_eq(&frontier),
            abi::load_u64(&best, &t, 0),
            abi::compare_immediate(&best, "0"),
            abi::branch_eq(&up),
            abi::branch(&take),
        ]);
    } else {
        s.ins.push(abi::branch(&frontier));
    }
    // Take run `best`: unlink it, carve `size` from its low end, re-list the rest.
    let whole = s.label("whole");
    let carved = s.label("carved");
    let node_head = s.reg();
    let rec = s.reg();
    s.ins.push(abi::label(&take));
    s.node(&n, &best);
    s.ins.push(abi::load_u64(&start, &n, ARENA_NODE_START));
    s.run_len(&z, &n);
    // plan-183-E: a cold run commits the pages of the part handed out first, before
    // anything changes, so a refused commit fails the allocation cleanly.
    {
        let flags = s.reg();
        let cend = s.reg();
        let run_end = s.reg();
        let rest_whole = s.label("take_whole_end");
        let have_end = s.label("take_have_end");
        s.run_flags(&flags, &n);
        s.ins.extend([
            abi::add_registers(&run_end, &start, &z),
            abi::subtract_registers(&t, &z, &size),
            abi::compare_immediate(
                &t,
                &(if huge { HUGE_MIN_RUN } else { MEDIUM_MIN_RUN }).to_string(),
            ),
            abi::branch_lo(&rest_whole),
            abi::add_registers(&cend, &start, &size),
            abi::branch(&have_end),
            abi::label(&rest_whole),
            abi::move_register(&cend, &run_end),
            abi::label(&have_end),
        ]);
        s.commit_cold_part(&flags, &start, &cend, &run_end, &fail);
    }
    s.head(&node_head, &z, &is_huge);
    s.unlink(&node_head, &n);
    s.ins.extend([
        abi::subtract_registers(&t, &z, &size),
        abi::compare_immediate(
            &t,
            &(if huge { HUGE_MIN_RUN } else { MEDIUM_MIN_RUN }).to_string(),
        ),
        abi::branch_lo(&whole),
        // The rest: [start + size, start + z), the same node and end record, with the
        // run's flags (a cold run's rest stays cold: its whole pages are still
        // decommitted).
        abi::add_registers(&cur, &start, &size),
        abi::store_u64(&cur, &n, ARENA_NODE_START),
    ]);
    {
        let flags = s.reg();
        s.run_flags(&flags, &n);
        s.ins.extend([
            abi::or_registers(&flags, &t, &flags),
            abi::store_u64(&flags, &n, ARENA_NODE_LEN),
        ]);
    }
    s.record(&rec, &cur);
    s.ins.extend([
        abi::shift_left_immediate(&cur, &best, 2),
        abi::add_immediate(&cur, &cur, 2),
        abi::store_u64(&cur, &rec, 0),
    ]);
    s.head(&node_head, &t, &is_huge);
    s.push(&node_head, &best, &n);
    s.ins.extend([
        abi::branch(&carved),
        abi::label(&whole),
        abi::move_register(&size, &z),
    ]);
    // The whole run: its end record goes, and its node.
    s.ins.extend([
        abi::add_registers(&t, &start, &z),
        abi::subtract_immediate(&t, &t, 16),
    ]);
    s.record(&rec, &t);
    s.ins.push(abi::store_u64(abi::ZERO, &rec, 8));
    s.drop_node(&best, &n);
    s.ins.push(abi::label(&carved));
    s.tag_chunk(&start, &size);
    s.ins.extend([
        abi::move_register(abi::return_register(), &start),
        abi::move_immediate(abi::mfb_return(1), "Integer", "0"),
        abi::return_(),
        abi::label(&frontier),
    ]);
    // The region's frontier: huge carves up from its top, medium down from its
    // bottom (plan-183-D).
    let end = s.reg();
    let limit_reg = s.reg();
    let need = s.reg();
    let sc = s.sc;
    if huge {
        s.ins.extend([
            abi::load_u64(&start, &sc, ARENA_SC_HUGE_FRONT),
            abi::add_registers(&end, &start, &size),
            abi::compare_registers(&end, &start),
            abi::branch_lo(&fail),
            abi::load_u64(&limit_reg, &sc, ARENA_SC_MEDIUM_FRONT),
            abi::compare_registers(&end, &limit_reg),
            abi::branch_hi(&fail),
            abi::load_u64(&limit_reg, &sc, ARENA_SC_MEDIUM_COMMIT),
        ]);
        emit_ensure_up(
            symbol,
            "data",
            &sc,
            ARENA_SC_HUGE_COMMIT,
            &end,
            &limit_reg,
            ARENA_COMMIT_STEP,
            &fail,
            &mut s.ins,
            &mut s.rel,
            &mut s.v,
        );
        // Tag records for every block of the chunk.
        s.ins.push(abi::subtract_immediate(&t, &end, 16));
        s.record(&need, &t);
        s.ins.extend([
            abi::add_immediate(&need, &need, ARENA_TAG_RECORD_SIZE as usize),
            abi::load_u64(&limit_reg, &sc, ARENA_SC_TAGS_HIGH),
        ]);
        emit_ensure_up(
            symbol,
            "tags",
            &sc,
            ARENA_SC_TAGS_LOW,
            &need,
            &limit_reg,
            ARENA_COMMIT_GRANULE,
            &fail,
            &mut s.ins,
            &mut s.rel,
            &mut s.v,
        );
        s.ins.push(abi::store_u64(&end, &sc, ARENA_SC_HUGE_FRONT));
    } else {
        s.ins.extend([
            abi::load_u64(&end, &sc, ARENA_SC_MEDIUM_FRONT),
            abi::subtract_registers(&start, &end, &size),
            abi::compare_registers(&start, &end),
            abi::branch_hi(&fail),
            abi::load_u64(&limit_reg, &sc, ARENA_SC_HUGE_FRONT),
            abi::compare_registers(&start, &limit_reg),
            abi::branch_lo(&fail),
            abi::load_u64(&limit_reg, &sc, ARENA_SC_HUGE_COMMIT),
        ]);
        emit_ensure_down(
            symbol,
            "data",
            &sc,
            ARENA_SC_MEDIUM_COMMIT,
            &start,
            &limit_reg,
            ARENA_COMMIT_STEP,
            &fail,
            &mut s.ins,
            &mut s.rel,
            &mut s.v,
        );
        s.record(&need, &start);
        s.ins
            .push(abi::load_u64(&limit_reg, &sc, ARENA_SC_TAGS_LOW));
        emit_ensure_down(
            symbol,
            "tags",
            &sc,
            ARENA_SC_TAGS_HIGH,
            &need,
            &limit_reg,
            ARENA_COMMIT_GRANULE,
            &fail,
            &mut s.ins,
            &mut s.rel,
            &mut s.v,
        );
        s.ins
            .push(abi::store_u64(&start, &sc, ARENA_SC_MEDIUM_FRONT));
    }
    s.tag_chunk(&start, &size);
    s.ins.extend([
        abi::move_register(abi::return_register(), &start),
        abi::move_immediate(abi::mfb_return(1), "Integer", "1"),
        abi::return_(),
        abi::label(&fail),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::move_immediate(abi::mfb_return(1), "Integer", "0"),
        abi::return_(),
    ]);
    finalize_vreg_helper(
        if huge {
            "runtime.arena_huge_alloc"
        } else {
            "runtime.arena_medium_alloc"
        },
        symbol,
        "Pointer",
        s.ins,
        s.rel,
    )
}

/// `_mfb_arena_span_free(ptr)`: the chunk's size comes from its start record (whose
/// granule offset must match `ptr`, so a pointer into a chunk, a freed chunk or a
/// free run is refused). The chunk is zeroed whole, then merged with a free
/// neighbour on either side within its own region and listed.
pub(crate) fn lower_arena_span_free(debug_arena: bool) -> CodeFunction {
    let symbol = ARENA_SPAN_FREE_SYMBOL;
    let mut s = new_span(symbol);
    let ptr = s.reg();
    let size = s.reg();
    let rec = s.reg();
    let tag = s.reg();
    let t = s.reg();
    let u = s.reg();
    let is_huge = s.reg();
    let lo = s.reg();
    let hi = s.reg();
    let left = s.reg();
    let right = s.reg();
    let ln = s.reg();
    let rn = s.reg();
    let end = s.reg();
    let head = s.reg();
    let refuse = format!("{symbol}_refuse");
    let sc = s.sc;
    s.ins.extend([
        abi::move_register(&ptr, abi::c_arg(0)),
        abi::load_u64(&t, &sc, ARENA_SC_SPAN_BASE),
        abi::compare_registers(&ptr, &t),
        abi::branch_lo(&refuse),
        abi::load_u64(&t, &sc, ARENA_SC_SLICE_TOP),
        abi::compare_registers(&ptr, &t),
        abi::branch_lo(&format!("{symbol}_in_span")),
        abi::branch(&refuse),
        abi::label(&format!("{symbol}_in_span")),
    ]);
    s.record(&rec, &ptr);
    s.ins.extend([
        abi::load_u64(&tag, &rec, 0),
        abi::move_immediate(&t, "Integer", "1"),
        abi::and_registers(&t, &tag, &t),
        abi::compare_immediate(&t, "0"),
        abi::branch_eq(&refuse),
        abi::shift_right_immediate(&t, &tag, 48),
        abi::move_immediate(&u, "Integer", "127"),
        abi::and_registers(&t, &t, &u),
        abi::shift_right_immediate(&end, &ptr, 4),
        abi::and_registers(&end, &end, &u),
        abi::compare_registers(&t, &end),
        abi::branch_ne(&refuse),
        abi::move_immediate(&u, "Integer", &(((1u64 << 48) - 1) & !15).to_string()),
        abi::and_registers(&size, &tag, &u),
        abi::store_u64(abi::ZERO, &rec, 0),
    ]);
    // plan-183-E: a chunk of `ARENA_LARGE_FREE` or more gives its whole pages back
    // instead of zeroing them (they read zero when committed again) and fills only its
    // partial edge pages; it joins its neighbours as a cold run. A smaller chunk is
    // zeroed whole.
    let ccold = s.reg();
    let filled = s.reg();
    let small_fill = s.label("fill_whole");
    let filled_done = s.label("fill_done");
    {
        let lo = s.reg();
        let hi = s.reg();
        let cend = s.reg();
        s.ins.extend([
            abi::move_immediate(&ccold, "Integer", "0"),
            abi::move_register(&filled, &size),
            abi::move_immediate(&t, "Integer", &ARENA_LARGE_FREE.to_string()),
            abi::compare_registers(&size, &t),
            abi::branch_lo(&small_fill),
            abi::add_registers(&cend, &ptr, &size),
        ]);
        emit_round_up(&lo, &ptr, ARENA_DECOMMIT_PAGE, &t, &mut s.ins);
        emit_round_down(&hi, &cend, ARENA_DECOMMIT_PAGE, &t, &mut s.ins);
        s.ins.extend([
            abi::move_immediate(&ccold, "Integer", &RUN_COLD.to_string()),
            abi::subtract_registers(&filled, &lo, &ptr),
            abi::subtract_registers(&t, &cend, &hi),
            abi::add_registers(&filled, &filled, &t),
            abi::move_register(abi::c_arg(0), &ptr),
            abi::subtract_registers(abi::c_arg(1), &lo, &ptr),
            abi::branch_link(ARENA_FILL_ZERO_SYMBOL),
        ]);
        // `lo`/`hi`/`cend` are recomputed after the call (it clobbers them).
        s.ins.push(abi::add_registers(&cend, &ptr, &size));
        emit_round_down(&hi, &cend, ARENA_DECOMMIT_PAGE, &t, &mut s.ins);
        s.ins.extend([
            abi::move_register(abi::c_arg(0), &hi),
            abi::subtract_registers(abi::c_arg(1), &cend, &hi),
            abi::branch_link(ARENA_FILL_ZERO_SYMBOL),
        ]);
        s.ins.push(abi::add_registers(&cend, &ptr, &size));
        emit_round_up(&lo, &ptr, ARENA_DECOMMIT_PAGE, &t, &mut s.ins);
        emit_round_down(&hi, &cend, ARENA_DECOMMIT_PAGE, &t, &mut s.ins);
        s.ins.extend([
            abi::move_register(abi::c_arg(0), &lo),
            abi::move_register(abi::c_arg(1), &hi),
            abi::branch_link(ARENA_DECOMMIT_SYMBOL),
            abi::branch(&filled_done),
            abi::label(&small_fill),
            abi::move_register(abi::c_arg(0), &ptr),
            abi::move_register(abi::c_arg(1), &size),
            abi::branch_link(ARENA_FILL_ZERO_SYMBOL),
            abi::label(&filled_done),
        ]);
        s.rel.push(internal_branch(symbol, ARENA_FILL_ZERO_SYMBOL));
        s.rel.push(internal_branch(symbol, ARENA_FILL_ZERO_SYMBOL));
        s.rel.push(internal_branch(symbol, ARENA_FILL_ZERO_SYMBOL));
        s.rel.push(internal_branch(symbol, ARENA_DECOMMIT_SYMBOL));
    }
    if debug_arena {
        let slot = s.reg();
        emit_debug_arena_slot(symbol, &slot, &mut s.ins, &mut s.rel, &mut s.v);
        emit_debug_arena_bump(
            symbol,
            &slot,
            COUNTER_FILL_FREE_CALLS,
            "fill_free",
            &mut s.ins,
            &mut s.v,
        );
        emit_debug_arena_add(
            symbol,
            &slot,
            COUNTER_FILL_FREE_BYTES,
            &filled,
            "fill_free_bytes",
            &mut s.ins,
            &mut s.v,
        );
    }
    // The region: huge from the span base up to its front, medium from its front up
    // to the slice top. Merges never cross a region's bounds.
    let medium = s.label("medium");
    let bounded = s.label("bounded");
    s.ins.extend([
        abi::move_immediate(&is_huge, "Integer", "1"),
        abi::load_u64(&lo, &sc, ARENA_SC_SPAN_BASE),
        abi::load_u64(&hi, &sc, ARENA_SC_HUGE_FRONT),
        abi::compare_registers(&ptr, &hi),
        abi::branch_lo(&bounded),
        abi::label(&medium),
        abi::move_immediate(&is_huge, "Integer", "0"),
        abi::load_u64(&lo, &sc, ARENA_SC_MEDIUM_FRONT),
        abi::load_u64(&hi, &sc, ARENA_SC_SLICE_TOP),
        abi::label(&bounded),
        abi::add_registers(&end, &ptr, &size),
        abi::move_immediate(&left, "Integer", "0"),
        abi::move_immediate(&right, "Integer", "0"),
    ]);
    // Left: the free run whose end record is in the block before `ptr`.
    let no_left = s.label("no_left");
    s.ins.extend([
        abi::compare_registers(&ptr, &lo),
        abi::branch_ls(&no_left),
        abi::subtract_immediate(&t, &ptr, 16),
    ]);
    s.record(&u, &t);
    s.ins.extend([
        abi::load_u64(&t, &u, 8),
        abi::compare_immediate(&t, "0"),
        abi::branch_eq(&no_left),
    ]);
    s.node(&ln, &t);
    s.ins.push(abi::load_u64(&u, &ln, ARENA_NODE_START));
    s.run_len(&head, &ln);
    s.ins.extend([
        abi::add_registers(&u, &u, &head),
        abi::compare_registers(&u, &ptr),
        abi::branch_ne(&no_left),
        abi::move_register(&left, &t),
        abi::label(&no_left),
    ]);
    // Right: the free run whose start record is in the block of `end`.
    let no_right = s.label("no_right");
    s.ins.extend([
        abi::compare_registers(&end, &hi),
        abi::branch_lo(&format!("{symbol}_right_in")),
        abi::branch(&no_right),
        abi::label(&format!("{symbol}_right_in")),
    ]);
    let rrec = s.reg();
    s.record(&rrec, &end);
    s.ins.extend([
        abi::load_u64(&t, &rrec, 0),
        abi::move_immediate(&u, "Integer", "3"),
        abi::and_registers(&u, &t, &u),
        abi::compare_immediate(&u, "2"),
        abi::branch_ne(&no_right),
        abi::shift_right_immediate(&t, &t, 2),
    ]);
    s.node(&rn, &t);
    s.ins.extend([
        abi::load_u64(&u, &rn, ARENA_NODE_START),
        abi::compare_registers(&u, &end),
        abi::branch_ne(&no_right),
        abi::move_register(&right, &t),
        abi::label(&no_right),
    ]);
    // plan-183-E: what the merge is made of, before it changes anything — the left
    // run `[ls, ptr)`, the chunk `[ptr, end)`, the right run `[end, re)` — and whether
    // each is cold. The merged run is cold when any part is (`cold_fixup` below).
    let ls = s.reg();
    let lflags = s.reg();
    let re = s.reg();
    let rflags = s.reg();
    let res = s.reg();
    {
        let no_l = s.label("parts_no_left");
        let no_r = s.label("parts_no_right");
        s.ins.extend([
            abi::move_register(&ls, &ptr),
            abi::move_immediate(&lflags, "Integer", "0"),
            abi::move_register(&re, &end),
            abi::move_immediate(&rflags, "Integer", "0"),
            abi::move_immediate(&res, "Integer", "0"),
            abi::compare_immediate(&left, "0"),
            abi::branch_eq(&no_l),
            abi::load_u64(&ls, &ln, ARENA_NODE_START),
        ]);
        s.run_flags(&lflags, &ln);
        s.ins.extend([
            abi::label(&no_l),
            abi::compare_immediate(&right, "0"),
            abi::branch_eq(&no_r),
        ]);
        s.run_len(&re, &rn);
        s.ins.push(abi::add_registers(&re, &re, &end));
        s.run_flags(&rflags, &rn);
        s.ins.push(abi::label(&no_r));
    }
    let only_left = s.label("only_left");
    let only_right = s.label("only_right");
    let neither = s.label("neither");
    let done = s.label("merged");
    s.ins.extend([
        abi::compare_immediate(&left, "0"),
        abi::branch_eq(&format!("{symbol}_no_l")),
        abi::compare_immediate(&right, "0"),
        abi::branch_eq(&only_left),
    ]);
    // Both: left absorbs the chunk and right.
    let lsize = s.reg();
    let rsize = s.reg();
    s.run_len(&lsize, &ln);
    s.head(&head, &lsize, &is_huge);
    s.unlink(&head, &ln);
    s.run_len(&rsize, &rn);
    s.head(&head, &rsize, &is_huge);
    s.unlink(&head, &rn);
    s.ins.extend([
        abi::store_u64(abi::ZERO, &rrec, 0),
        abi::subtract_immediate(&t, &ptr, 16),
    ]);
    s.record(&u, &t);
    s.ins.extend([
        abi::store_u64(abi::ZERO, &u, 8),
        abi::add_registers(&lsize, &lsize, &size),
        abi::add_registers(&lsize, &lsize, &rsize),
        abi::store_u64(&lsize, &ln, ARENA_NODE_LEN),
        abi::add_registers(&t, &end, &rsize),
        abi::subtract_immediate(&t, &t, 16),
    ]);
    s.record(&u, &t);
    s.ins.push(abi::store_u64(&left, &u, 8));
    s.drop_node(&right, &rn);
    s.head(&head, &lsize, &is_huge);
    s.push(&head, &left, &ln);
    s.ins.extend([
        abi::move_register(&res, &ln),
        abi::branch(&done),
        abi::label(&only_left),
    ]);
    // Left only: it grows over the chunk.
    s.run_len(&lsize, &ln);
    s.head(&head, &lsize, &is_huge);
    s.unlink(&head, &ln);
    s.ins.push(abi::subtract_immediate(&t, &ptr, 16));
    s.record(&u, &t);
    s.ins.extend([
        abi::store_u64(abi::ZERO, &u, 8),
        abi::add_registers(&lsize, &lsize, &size),
        abi::store_u64(&lsize, &ln, ARENA_NODE_LEN),
        abi::subtract_immediate(&t, &end, 16),
    ]);
    s.record(&u, &t);
    s.ins.push(abi::store_u64(&left, &u, 8));
    s.head(&head, &lsize, &is_huge);
    s.push(&head, &left, &ln);
    s.ins.extend([
        abi::move_register(&res, &ln),
        abi::branch(&done),
        abi::label(&format!("{symbol}_no_l")),
        abi::compare_immediate(&right, "0"),
        abi::branch_eq(&neither),
        abi::label(&only_right),
    ]);
    // Right only: it grows down over the chunk.
    s.run_len(&rsize, &rn);
    s.head(&head, &rsize, &is_huge);
    s.unlink(&head, &rn);
    s.ins.extend([
        abi::store_u64(abi::ZERO, &rrec, 0),
        abi::add_registers(&rsize, &rsize, &size),
        abi::store_u64(&rsize, &rn, ARENA_NODE_LEN),
        abi::store_u64(&ptr, &rn, ARENA_NODE_START),
        abi::shift_left_immediate(&t, &right, 2),
        abi::add_immediate(&t, &t, 2),
        abi::store_u64(&t, &rec, 0),
    ]);
    s.head(&head, &rsize, &is_huge);
    s.push(&head, &right, &rn);
    s.ins.extend([
        abi::move_register(&res, &rn),
        abi::branch(&done),
        abi::label(&neither),
    ]);
    // Neither: a new run. With the pool exhausted the bytes stay unlisted (zeroed,
    // unreachable) rather than corrupt anything; the pool holds as many nodes as the
    // span can hold runs, so only a refused commit gets here.
    let node = s.reg();
    s.take_node(&node, &done);
    s.node(&ln, &node);
    s.ins.extend([
        abi::store_u64(&ptr, &ln, ARENA_NODE_START),
        abi::store_u64(&size, &ln, ARENA_NODE_LEN),
        abi::shift_left_immediate(&t, &node, 2),
        abi::add_immediate(&t, &t, 2),
        abi::store_u64(&t, &rec, 0),
        abi::subtract_immediate(&t, &end, 16),
    ]);
    s.record(&u, &t);
    s.ins.push(abi::store_u64(&node, &u, 8));
    s.head(&head, &size, &is_huge);
    s.push(&head, &node, &ln);
    s.ins
        .extend([abi::move_register(&res, &ln), abi::label(&done)]);
    emit_cold_fixup(&mut s, &res, &ls, &lflags, &ptr, &end, &ccold, &re, &rflags);
    s.ins.extend([
        abi::move_register(abi::return_register(), &size),
        abi::return_(),
        abi::label(&refuse),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::return_(),
    ]);
    finalize_vreg_helper("runtime.arena_span_free", symbol, "Integer", s.ins, s.rel)
}

/// plan-183-E: after a free merged `[ls, ptr)`, the chunk `[ptr, end)` and `[end, re)`
/// into the run whose node is at `res` (0: the bytes were left unlisted), make that run
/// cold when any part was (`lflags`/`rflags` are the runs' flags, `ccold` the chunk's):
/// flag it, and decommit the whole pages of the merged run that are not decommitted yet
/// — the gaps around the cold parts' interiors — so the committed count stays exact.
#[allow(clippy::too_many_arguments)]
fn emit_cold_fixup(
    s: &mut Span<'_>,
    res: &VirtualRegister,
    ls: &VirtualRegister,
    lflags: &VirtualRegister,
    ptr: &VirtualRegister,
    end: &VirtualRegister,
    ccold: &VirtualRegister,
    re: &VirtualRegister,
    rflags: &VirtualRegister,
) {
    let skip = s.label("cold_skip");
    let cur = s.reg();
    let t = s.reg();
    let u = s.reg();
    s.ins.extend([
        abi::compare_immediate(res, "0"),
        abi::branch_eq(&skip),
        // The runs' seen flags carry over to the merged run.
        abi::or_registers(&t, lflags, rflags),
        abi::move_immediate(&u, "Integer", &(RUN_SEEN | RUN_EDGE_SEEN).to_string()),
        abi::and_registers(&t, &t, &u),
        abi::load_u64(&u, res, ARENA_NODE_LEN),
        abi::or_registers(&u, &u, &t),
        abi::store_u64(&u, res, ARENA_NODE_LEN),
        abi::or_registers(&t, lflags, rflags),
        abi::or_registers(&t, &t, ccold),
        abi::move_immediate(&u, "Integer", &RUN_COLD.to_string()),
        abi::and_registers(&t, &t, &u),
        abi::compare_immediate(&t, "0"),
        abi::branch_eq(&skip),
        abi::load_u64(&u, res, ARENA_NODE_LEN),
        abi::or_registers(&u, &u, &t),
        abi::store_u64(&u, res, ARENA_NODE_LEN),
    ]);
    emit_round_up(&cur, ls, ARENA_DECOMMIT_PAGE, &t, &mut s.ins);
    // Nothing past the merged run's last whole page is decommitted: a cold part shorter
    // than a page has an empty interior, and `page_up` of its start can lie past the
    // run's end, in the page of the live chunk after it.
    let cap = s.reg();
    emit_round_down(&cap, re, ARENA_DECOMMIT_PAGE, &t, &mut s.ins);
    // Each cold part: decommit up to its interior, then step past it.
    for (flags, start, stop, tag) in [
        (lflags, ls, ptr, "left"),
        (ccold, ptr, end, "chunk"),
        (rflags, end, re, "right"),
    ] {
        let hot = s.label(&format!("cold_{tag}_hot"));
        let ahead = s.label(&format!("cold_{tag}_ahead"));
        let capped = s.label(&format!("cold_{tag}_capped"));
        s.ins.extend([
            abi::move_immediate(&u, "Integer", &RUN_COLD.to_string()),
            abi::and_registers(&u, flags, &u),
            abi::compare_immediate(&u, "0"),
            abi::branch_eq(&hot),
        ]);
        emit_round_up(&u, start, ARENA_DECOMMIT_PAGE, &t, &mut s.ins);
        s.ins.extend([
            abi::compare_registers(&u, &cap),
            abi::branch_ls(&capped),
            abi::move_register(&u, &cap),
            abi::label(&capped),
            abi::move_register(abi::c_arg(0), &cur),
            abi::move_register(abi::c_arg(1), &u),
            abi::branch_link(ARENA_DECOMMIT_SYMBOL),
        ]);
        s.rel.push(internal_branch(s.from, ARENA_DECOMMIT_SYMBOL));
        emit_round_down(&u, stop, ARENA_DECOMMIT_PAGE, &t, &mut s.ins);
        s.ins.extend([
            abi::compare_registers(&u, &cur),
            abi::branch_ls(&ahead),
            abi::move_register(&cur, &u),
            abi::label(&ahead),
            abi::label(&hot),
        ]);
    }
    s.ins.extend([
        abi::move_register(abi::c_arg(0), &cur),
        abi::move_register(abi::c_arg(1), &cap),
        abi::branch_link(ARENA_DECOMMIT_SYMBOL),
        abi::label(&skip),
    ]);
    s.rel.push(internal_branch(s.from, ARENA_DECOMMIT_SYMBOL));
}
