//! plan-142-B: shrink a list **in place** to a subset of its elements.
//!
//! One primitive serves every shrinking self-update — `filter`, `distinct`,
//! `take`, `drop`, `mid` — because they differ only in *which* elements survive,
//! never in how survivors are moved. [`KeepSource`] names the survivors; the
//! compaction moves them down, frees what the dropped ones owned, and never
//! allocates on the common path.
//!
//! **Order matters, and is not guaranteed.** A fixed-width list (`kind2`) stores
//! element `i` at `i * width`, so compaction is one forward pass. A variable-width
//! list keeps a lookup-entry table over a packed data region, and the data need not
//! be in entry order: `insert`/`prepend` put a new payload at the data tail, a
//! length-changing `set` leaves its old span behind as dead bytes, and a sorted
//! list permutes entries over unmoved data. So the compaction probes first:
//!
//! * **In order** — every payload starts at the aligned end of the previous one.
//!   One pass moves each survivor's payload and entry down (the destination never
//!   passes the source, so a forward copy is safe) and the data region ends tight.
//! * **Out of order** — the survivors' entries move down, and the dropped payloads
//!   stay behind as dead bytes, which a list already tolerates (the contract of
//!   `emit_repack_list_data`, which a length-changing `set` relies on). When the
//!   dead bytes then outnumber the live ones, that repack runs: a loop of shrinks
//!   cannot grow the block without bound, and the one allocation is paid for by at
//!   least as many bytes dropped.
//!
//! The caller guarantees what every in-place arm guarantees: the block is uniquely
//! owned and no `FOR EACH` walks it (plan-142-A's container gates). Any error the
//! operation can raise must be raised **before** this runs — compaction is the
//! write, and it cannot fail.

use crate::codegen::collection::layout::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::types::ParameterType;

/// Which elements a compaction keeps.
#[derive(Debug, Clone, Copy)]
pub(crate) enum KeepSource {
    /// The frame slot holds a pointer to one byte per element; non-zero = keep.
    /// (`filter`, `distinct` — marks computed before any write.)
    Marks { marks_slot: usize },
    /// Keep the index range `[start, start + len)`. Both slots hold values the
    /// caller has already clamped into `[0, count]`. (`take`, `drop`, `mid`.)
    Range { start_slot: usize, len_slot: usize },
}

impl CodeBuilder<'_> {
    /// Branch to `keep` when element `index` survives, else to `drop`.
    fn emit_keep_branch(
        &mut self,
        source: &KeepSource,
        index: &VirtualRegister,
        keep: &str,
        drop: &str,
    ) {
        match *source {
            KeepSource::Marks { marks_slot } => {
                let marks = self.temporary_vreg();
                let mark = self.temporary_vreg();
                self.emit(abi::load_u64(&marks, abi::stack_pointer(), marks_slot));
                self.emit(abi::add_registers(&marks, &marks, index));
                self.emit(abi::load_u8(&mark, &marks, 0));
                self.emit(abi::compare_immediate(&mark, "0"));
                self.emit(abi::branch_ne(keep));
                self.emit(abi::branch(drop));
            }
            KeepSource::Range {
                start_slot,
                len_slot,
            } => {
                let start = self.temporary_vreg();
                let end = self.temporary_vreg();
                self.emit(abi::load_u64(&start, abi::stack_pointer(), start_slot));
                self.emit(abi::compare_registers(index, &start));
                self.emit(abi::branch_lt(drop));
                self.emit(abi::load_u64(&end, abi::stack_pointer(), len_slot));
                self.emit(abi::add_registers(&end, &end, &start));
                self.emit(abi::compare_registers(index, &end));
                self.emit(abi::branch_lt(keep));
                self.emit(abi::branch(drop));
            }
        }
    }

    /// Shrink the list whose block pointer lives in `buffer_slot` to the elements
    /// `keep` names, in their original order, **mutating the block in place**.
    /// `count` and `dataLength` are updated; `capacity` and `dataCapacity` are not
    /// (the freed room is headroom a later `append` reuses). On the out-of-order
    /// path the block may be repacked, so `buffer_slot` can be repointed.
    pub(crate) fn lower_list_compact_in_place(
        &mut self,
        buffer_slot: usize,
        keep: &KeepSource,
        list_type: &ParameterType,
        element_type: &ParameterType,
    ) -> Result<(), String> {
        CollectionTypeLayout::from_type(list_type)
            .ok_or_else(|| format!("native code collection type '{list_type}' is not supported"))?;
        let index_slot = self.allocate_stack_object("compact_index", 8);
        // A dropped element that owns a graph (a recursive value) must have it freed
        // before its entry is overwritten — the same obligation as
        // `lower_list_remove_at_in_place` (plan-134-H). The drop calls clobber every
        // register, so this runs as its own pass over a spilled index.
        if kind2_payload_size(element_type).is_none() && self.owns_graph(element_type) {
            let top = self.label("compact_drop_loop");
            let drop = self.label("compact_drop_one");
            let next = self.label("compact_drop_next");
            let done = self.label("compact_drop_done");
            let index = self.temporary_vreg();
            let base = self.temporary_vreg();
            let count = self.temporary_vreg();
            self.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), index_slot));
            self.emit(abi::label(&top));
            self.emit(abi::load_u64(&index, abi::stack_pointer(), index_slot));
            self.emit(abi::load_u64(&base, abi::stack_pointer(), buffer_slot));
            self.emit(abi::load_u64(&count, &base, COLLECTION_OFFSET_COUNT));
            self.emit(abi::compare_registers(&index, &count));
            self.emit(abi::branch_ge(&done));
            self.emit_keep_branch(keep, &index, &next, &drop);
            self.emit(abi::label(&drop));
            self.emit_drop_list_element(buffer_slot, index_slot, element_type)?;
            self.emit(abi::label(&next));
            let index = self.temporary_vreg();
            self.emit(abi::load_u64(&index, abi::stack_pointer(), index_slot));
            self.emit(abi::add_immediate(&index, &index, 1));
            self.emit(abi::store_u64(&index, abi::stack_pointer(), index_slot));
            self.emit(abi::branch(&top));
            self.emit(abi::label(&done));
        }
        match kind2_payload_size(element_type) {
            Some(width) => self.emit_compact_fixed_width(buffer_slot, keep, element_type, width),
            None => self.emit_compact_entries(buffer_slot, keep, list_type, element_type)?,
        }
        Ok(())
    }

    /// Fixed-width (`kind2`) compaction: element `i` lives at `data + i * width`.
    fn emit_compact_fixed_width(
        &mut self,
        buffer_slot: usize,
        keep: &KeepSource,
        element_type: &ParameterType,
        width: usize,
    ) {
        let base = self.temporary_vreg();
        let data = self.temporary_vreg();
        let count = self.temporary_vreg();
        let index = self.temporary_vreg();
        let kept = self.temporary_vreg();
        let src = self.temporary_vreg();
        let dst = self.temporary_vreg();
        let len = self.temporary_vreg();
        let scratch = self.temporary_vreg();
        let copy = self.temporary_vreg();

        if let KeepSource::Range {
            start_slot,
            len_slot,
        } = *keep
        {
            // One contiguous run: slide `[start, start + len)` to the front.
            let done = self.label("compact_k2_range_done");
            self.emit(abi::load_u64(&base, abi::stack_pointer(), buffer_slot));
            self.emit_collection_data_pointer_for(&dst, &base, element_type);
            self.emit(abi::load_u64(&index, abi::stack_pointer(), start_slot));
            self.emit(abi::move_immediate(&scratch, "Integer", &width.to_string()));
            self.emit(abi::multiply_registers(&src, &index, &scratch));
            self.emit(abi::add_registers(&src, &src, &dst));
            self.emit(abi::load_u64(&kept, abi::stack_pointer(), len_slot));
            self.emit(abi::multiply_registers(&len, &kept, &scratch));
            self.emit(abi::compare_immediate(&index, "0"));
            self.emit(abi::branch_eq(&done));
            self.emit_block_copy_advance(&dst, &src, &len, &copy, "compact_k2_range");
            self.emit(abi::label(&done));
            self.emit(abi::load_u64(&base, abi::stack_pointer(), buffer_slot));
            self.emit(abi::load_u64(&kept, abi::stack_pointer(), len_slot));
            self.emit(abi::store_u64(&kept, &base, COLLECTION_OFFSET_COUNT));
            self.emit(abi::move_immediate(&scratch, "Integer", &width.to_string()));
            self.emit(abi::multiply_registers(&len, &kept, &scratch));
            self.emit(abi::store_u64(&len, &base, COLLECTION_OFFSET_DATA_LENGTH));
            return;
        }

        let top = self.label("compact_k2_loop");
        let keep_label = self.label("compact_k2_keep");
        let next = self.label("compact_k2_next");
        let done = self.label("compact_k2_done");
        let moved = self.label("compact_k2_moved");
        self.emit(abi::load_u64(&base, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&count, &base, COLLECTION_OFFSET_COUNT));
        self.emit_collection_data_pointer_for(&data, &base, element_type);
        self.emit(abi::move_immediate(&index, "Integer", "0"));
        self.emit(abi::move_immediate(&kept, "Integer", "0"));
        self.emit(abi::label(&top));
        self.emit(abi::compare_registers(&index, &count));
        self.emit(abi::branch_ge(&done));
        self.emit_keep_branch(keep, &index, &keep_label, &next);
        self.emit(abi::label(&keep_label));
        self.emit(abi::compare_registers(&index, &kept));
        self.emit(abi::branch_eq(&moved));
        self.emit(abi::move_immediate(&scratch, "Integer", &width.to_string()));
        self.emit(abi::multiply_registers(&src, &index, &scratch));
        self.emit(abi::add_registers(&src, &src, &data));
        self.emit(abi::multiply_registers(&dst, &kept, &scratch));
        self.emit(abi::add_registers(&dst, &dst, &data));
        self.emit(abi::move_immediate(&len, "Integer", &width.to_string()));
        self.emit_block_copy_advance(&dst, &src, &len, &copy, "compact_k2_elem");
        self.emit(abi::label(&moved));
        self.emit(abi::add_immediate(&kept, &kept, 1));
        self.emit(abi::label(&next));
        self.emit(abi::add_immediate(&index, &index, 1));
        self.emit(abi::branch(&top));
        self.emit(abi::label(&done));
        self.emit(abi::store_u64(&kept, &base, COLLECTION_OFFSET_COUNT));
        self.emit(abi::move_immediate(&scratch, "Integer", &width.to_string()));
        self.emit(abi::multiply_registers(&len, &kept, &scratch));
        self.emit(abi::store_u64(&len, &base, COLLECTION_OFFSET_DATA_LENGTH));
    }

    /// Entry-table compaction for a variable-width list; see the module doc for
    /// the in-order / out-of-order split.
    fn emit_compact_entries(
        &mut self,
        buffer_slot: usize,
        keep: &KeepSource,
        list_type: &ParameterType,
        element_type: &ParameterType,
    ) -> Result<(), String> {
        let stride = list_entry_stride(element_type);
        let alignment = self.list_element_padding_alignment(element_type);
        let base = self.temporary_vreg();
        let data = self.temporary_vreg();
        let count = self.temporary_vreg();
        let index = self.temporary_vreg();
        let entry = self.temporary_vreg();
        let off = self.temporary_vreg();
        let len = self.temporary_vreg();
        let expected = self.temporary_vreg();
        let align_scratch = self.temporary_vreg();

        let probe_loop = self.label("compact_probe_loop");
        let probe_next = self.label("compact_probe_next");
        let disordered = self.label("compact_disordered");
        let ordered = self.label("compact_ordered");
        let done = self.label("compact_done");

        // --- Probe: is every payload at the aligned end of the previous one? ---
        self.emit(abi::load_u64(&base, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&count, &base, COLLECTION_OFFSET_COUNT));
        self.emit(abi::add_immediate(&entry, &base, COLLECTION_HEADER_SIZE));
        self.emit(abi::move_immediate(&index, "Integer", "0"));
        self.emit(abi::move_immediate(&expected, "Integer", "0"));
        self.emit(abi::label(&probe_loop));
        self.emit(abi::compare_registers(&index, &count));
        self.emit(abi::branch_ge(&ordered));
        self.emit_align_offset_register(&expected, alignment, &align_scratch);
        self.emit(abi::load_u64(
            &off,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::compare_registers(&off, &expected));
        self.emit(abi::branch_eq(&probe_next));
        self.emit(abi::branch(&disordered));
        self.emit(abi::label(&probe_next));
        self.emit(abi::load_u64(
            &len,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers(&expected, &expected, &len));
        self.emit(abi::add_immediate(&entry, &entry, stride));
        self.emit(abi::add_immediate(&index, &index, 1));
        self.emit(abi::branch(&probe_loop));

        // --- In order: slide each survivor's payload and entry down. ---
        self.emit(abi::label(&ordered));
        {
            let kept = self.temporary_vreg();
            let write = self.temporary_vreg();
            let src = self.temporary_vreg();
            let dst = self.temporary_vreg();
            let remaining = self.temporary_vreg();
            let copy = self.temporary_vreg();
            let top = self.label("compact_ord_loop");
            let keep_label = self.label("compact_ord_keep");
            let payload_in_place = self.label("compact_ord_payload_ok");
            let entry_in_place = self.label("compact_ord_entry_ok");
            let next = self.label("compact_ord_next");
            let end = self.label("compact_ord_end");
            self.emit(abi::load_u64(&base, abi::stack_pointer(), buffer_slot));
            self.emit(abi::load_u64(&count, &base, COLLECTION_OFFSET_COUNT));
            self.emit_collection_data_pointer_for(&data, &base, element_type);
            self.emit(abi::move_immediate(&index, "Integer", "0"));
            self.emit(abi::move_immediate(&kept, "Integer", "0"));
            self.emit(abi::move_immediate(&write, "Integer", "0"));
            self.emit(abi::label(&top));
            self.emit(abi::compare_registers(&index, &count));
            self.emit(abi::branch_ge(&end));
            self.emit_keep_branch(keep, &index, &keep_label, &next);
            self.emit(abi::label(&keep_label));
            // entry = &entries[index]
            self.emit(abi::move_immediate(
                &align_scratch,
                "Integer",
                &stride.to_string(),
            ));
            self.emit(abi::multiply_registers(&entry, &index, &align_scratch));
            self.emit(abi::add_registers(&entry, &entry, &base));
            self.emit(abi::add_immediate(&entry, &entry, COLLECTION_HEADER_SIZE));
            self.emit(abi::load_u64(
                &off,
                &entry,
                COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
            ));
            self.emit(abi::load_u64(
                &len,
                &entry,
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ));
            self.emit_align_offset_register(&write, alignment, &align_scratch);
            // Payload: data+off → data+write (write <= off, so forward is safe).
            self.emit(abi::compare_registers(&off, &write));
            self.emit(abi::branch_eq(&payload_in_place));
            self.emit(abi::add_registers(&src, &data, &off));
            self.emit(abi::add_registers(&dst, &data, &write));
            self.emit(abi::move_register(&remaining, &len));
            self.emit_block_copy_advance(&dst, &src, &remaining, &copy, "compact_ord_payload");
            self.emit(abi::label(&payload_in_place));
            self.emit(abi::store_u64(
                &write,
                &entry,
                COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
            ));
            // Entry: entries[index] → entries[kept] (kept <= index).
            self.emit(abi::compare_registers(&index, &kept));
            self.emit(abi::branch_eq(&entry_in_place));
            self.emit(abi::move_register(&src, &entry));
            self.emit(abi::move_immediate(
                &align_scratch,
                "Integer",
                &stride.to_string(),
            ));
            self.emit(abi::multiply_registers(&dst, &kept, &align_scratch));
            self.emit(abi::add_registers(&dst, &dst, &base));
            self.emit(abi::add_immediate(&dst, &dst, COLLECTION_HEADER_SIZE));
            self.emit(abi::move_immediate(
                &remaining,
                "Integer",
                &stride.to_string(),
            ));
            self.emit_block_copy_advance(&dst, &src, &remaining, &copy, "compact_ord_entry");
            self.emit(abi::label(&entry_in_place));
            self.emit(abi::add_registers(&write, &write, &len));
            self.emit(abi::add_immediate(&kept, &kept, 1));
            self.emit(abi::label(&next));
            self.emit(abi::add_immediate(&index, &index, 1));
            self.emit(abi::branch(&top));
            self.emit(abi::label(&end));
            self.emit_align_offset_register(&write, alignment, &align_scratch);
            self.emit(abi::store_u64(&kept, &base, COLLECTION_OFFSET_COUNT));
            self.emit(abi::store_u64(&write, &base, COLLECTION_OFFSET_DATA_LENGTH));
            self.emit(abi::branch(&done));
        }

        // --- Out of order: move the survivors' entries; payloads stay put. ---
        self.emit(abi::label(&disordered));
        {
            let kept = self.temporary_vreg();
            let live = self.temporary_vreg();
            let src = self.temporary_vreg();
            let dst = self.temporary_vreg();
            let remaining = self.temporary_vreg();
            let copy = self.temporary_vreg();
            let top = self.label("compact_dis_loop");
            let keep_label = self.label("compact_dis_keep");
            let entry_in_place = self.label("compact_dis_entry_ok");
            let next = self.label("compact_dis_next");
            let end = self.label("compact_dis_end");
            let emptied = self.label("compact_dis_emptied");
            let repack = self.label("compact_dis_repack");
            let live_slot = self.allocate_stack_object("compact_live", 8);
            self.emit(abi::load_u64(&base, abi::stack_pointer(), buffer_slot));
            self.emit(abi::load_u64(&count, &base, COLLECTION_OFFSET_COUNT));
            self.emit(abi::move_immediate(&index, "Integer", "0"));
            self.emit(abi::move_immediate(&kept, "Integer", "0"));
            self.emit(abi::move_immediate(&live, "Integer", "0"));
            self.emit(abi::label(&top));
            self.emit(abi::compare_registers(&index, &count));
            self.emit(abi::branch_ge(&end));
            self.emit_keep_branch(keep, &index, &keep_label, &next);
            self.emit(abi::label(&keep_label));
            self.emit(abi::move_immediate(
                &align_scratch,
                "Integer",
                &stride.to_string(),
            ));
            self.emit(abi::multiply_registers(&entry, &index, &align_scratch));
            self.emit(abi::add_registers(&entry, &entry, &base));
            self.emit(abi::add_immediate(&entry, &entry, COLLECTION_HEADER_SIZE));
            self.emit(abi::load_u64(
                &len,
                &entry,
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ));
            self.emit_align_offset_register(&live, alignment, &align_scratch);
            self.emit(abi::add_registers(&live, &live, &len));
            self.emit(abi::compare_registers(&index, &kept));
            self.emit(abi::branch_eq(&entry_in_place));
            self.emit(abi::move_register(&src, &entry));
            self.emit(abi::move_immediate(
                &align_scratch,
                "Integer",
                &stride.to_string(),
            ));
            self.emit(abi::multiply_registers(&dst, &kept, &align_scratch));
            self.emit(abi::add_registers(&dst, &dst, &base));
            self.emit(abi::add_immediate(&dst, &dst, COLLECTION_HEADER_SIZE));
            self.emit(abi::move_immediate(
                &remaining,
                "Integer",
                &stride.to_string(),
            ));
            self.emit_block_copy_advance(&dst, &src, &remaining, &copy, "compact_dis_entry");
            self.emit(abi::label(&entry_in_place));
            self.emit(abi::add_immediate(&kept, &kept, 1));
            self.emit(abi::label(&next));
            self.emit(abi::add_immediate(&index, &index, 1));
            self.emit(abi::branch(&top));
            self.emit(abi::label(&end));
            self.emit(abi::store_u64(&kept, &base, COLLECTION_OFFSET_COUNT));
            self.emit(abi::store_u64(&live, abi::stack_pointer(), live_slot));
            // Nothing survived: the whole data region is dead, so it simply resets.
            self.emit(abi::compare_immediate(&kept, "0"));
            self.emit(abi::branch_eq(&emptied));
            // Dead bytes (dataLength - live) outnumber live ones: repack.
            self.emit(abi::load_u64(&len, &base, COLLECTION_OFFSET_DATA_LENGTH));
            self.emit(abi::subtract_registers(&len, &len, &live));
            self.emit(abi::compare_registers(&len, &live));
            self.emit(abi::branch_hi(&repack));
            self.emit(abi::branch(&done));
            self.emit(abi::label(&emptied));
            self.emit(abi::store_u64(
                abi::ZERO,
                &base,
                COLLECTION_OFFSET_DATA_LENGTH,
            ));
            self.emit(abi::branch(&done));
            self.emit(abi::label(&repack));
            let extra_slot = self.allocate_stack_object("compact_repack_extra", 8);
            self.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), extra_slot));
            self.emit_repack_list_data(buffer_slot, extra_slot, list_type, element_type, stride)?;
        }
        self.emit(abi::label(&done));
        Ok(())
    }
}
