//! plan-142-C: reorder a list **in place** by an index permutation.
//!
//! `sort`/`sortBy` compute the order as an index permutation (the same bottom-up
//! merge the copying lowerings run) and then apply it here instead of gathering a
//! new list. Both list representations keep their per-element records back to
//! back from `block + HEADER` — a fixed-width list's payloads (`width` bytes each),
//! an entry list's 40-byte lookup entries — so one cycle-following pass permutes
//! either: element `k` lives at `block + HEADER + k * stride`. An entry list's
//! payloads do not move, so its data ends out of entry order, which every reader
//! tolerates (payloads are located through their own entry).
//!
//! Cycle following needs one element of temporary space and a visited mark: the
//! mark is bit 63 of the permutation word (indices never reach it), so the
//! permutation buffer is consumed.

use crate::codegen::collection::layout::*;
use crate::codegen::engine::builder::*;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::types::ParameterType;

/// The visited mark on a permutation word.
const VISITED: &str = "9223372036854775808";

impl CodeBuilder<'_> {
    /// Reorder the list in `buffer_slot` so position `k` holds the element that was
    /// at `perm[k]`. `perm_slot` holds the address of `count` permutation words;
    /// `temp_slot` the address of at least one element's stride of scratch. Both are
    /// consumed. Nothing is allocated and nothing can fail.
    pub(crate) fn lower_list_permute_in_place(
        &mut self,
        buffer_slot: usize,
        perm_slot: usize,
        temp_slot: usize,
        element_type: &ParameterType,
    ) {
        let stride =
            kind2_payload_size(element_type).unwrap_or_else(|| list_entry_stride(element_type));
        let base = self.temporary_vreg();
        let perm = self.temporary_vreg();
        let temp = self.temporary_vreg();
        let count = self.temporary_vreg();
        let start = self.temporary_vreg();
        let k = self.temporary_vreg();
        let source = self.temporary_vreg();
        let word = self.temporary_vreg();
        let mark = self.temporary_vreg();
        let scratch = self.temporary_vreg();
        let dst = self.temporary_vreg();
        let src = self.temporary_vreg();
        let len = self.temporary_vreg();
        let copy = self.temporary_vreg();

        let outer = self.label("permute_outer");
        let outer_next = self.label("permute_outer_next");
        let outer_done = self.label("permute_outer_done");
        let cycle = self.label("permute_cycle");
        let close = self.label("permute_close");

        self.emit(abi::load_u64(&base, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&count, &base, COLLECTION_OFFSET_COUNT));
        self.emit(abi::add_immediate(&base, &base, COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64(&perm, abi::stack_pointer(), perm_slot));
        self.emit(abi::load_u64(&temp, abi::stack_pointer(), temp_slot));
        self.emit(abi::move_immediate(&mark, "Integer", VISITED));
        self.emit(abi::move_immediate(&start, "Integer", "0"));

        self.emit(abi::label(&outer));
        self.emit(abi::compare_registers(&start, &count));
        self.emit(abi::branch_ge(&outer_done));
        // word = perm[start]; skip a visited slot and a fixed point.
        self.emit(abi::shift_left_immediate(&scratch, &start, 3));
        self.emit(abi::add_registers(&scratch, &perm, &scratch));
        self.emit(abi::load_u64(&word, &scratch, 0));
        self.emit(abi::and_registers(&source, &word, &mark));
        self.emit(abi::compare_immediate(&source, "0"));
        self.emit(abi::branch_ne(&outer_next));
        self.emit(abi::compare_registers(&word, &start));
        self.emit(abi::branch_eq(&outer_next));
        // temp = element(start)
        self.emit(abi::move_immediate(&len, "Integer", &stride.to_string()));
        self.emit(abi::multiply_registers(&src, &start, &len));
        self.emit(abi::add_registers(&src, &base, &src));
        self.emit(abi::move_register(&dst, &temp));
        self.emit_block_copy_advance(&dst, &src, &len, &copy, "permute_save");
        self.emit(abi::move_register(&k, &start));

        self.emit(abi::label(&cycle));
        // source = perm[k]; perm[k] |= VISITED.
        self.emit(abi::shift_left_immediate(&scratch, &k, 3));
        self.emit(abi::add_registers(&scratch, &perm, &scratch));
        self.emit(abi::load_u64(&source, &scratch, 0));
        self.emit(abi::or_registers(&word, &source, &mark));
        self.emit(abi::store_u64(&word, &scratch, 0));
        self.emit(abi::compare_registers(&source, &start));
        self.emit(abi::branch_eq(&close));
        // element(k) = element(source); k = source.
        self.emit(abi::move_immediate(&len, "Integer", &stride.to_string()));
        self.emit(abi::multiply_registers(&dst, &k, &len));
        self.emit(abi::add_registers(&dst, &base, &dst));
        self.emit(abi::multiply_registers(&src, &source, &len));
        self.emit(abi::add_registers(&src, &base, &src));
        self.emit_block_copy_advance(&dst, &src, &len, &copy, "permute_move");
        self.emit(abi::move_register(&k, &source));
        self.emit(abi::branch(&cycle));

        // element(k) = temp: the cycle closes on its start.
        self.emit(abi::label(&close));
        self.emit(abi::move_immediate(&len, "Integer", &stride.to_string()));
        self.emit(abi::multiply_registers(&dst, &k, &len));
        self.emit(abi::add_registers(&dst, &base, &dst));
        self.emit(abi::move_register(&src, &temp));
        self.emit_block_copy_advance(&dst, &src, &len, &copy, "permute_restore");

        self.emit(abi::label(&outer_next));
        self.emit(abi::add_immediate(&start, &start, 1));
        self.emit(abi::branch(&outer));
        self.emit(abi::label(&outer_done));
    }
}
