// --- codegen tier imports (migration) ---
use crate::codegen::collection::layout::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::types::ParameterType;

mod append;
mod bulk_append;
mod inline_append;
mod inline_bulk_append;
mod insert;
mod remove;
mod set;
mod splice;

/// Where [`CodeBuilder::lower_list_splice_in_place`] puts the new element.
///
/// `Front` is not "a slot holding zero". It is a separate variant precisely so
/// `prepend` keeps emitting the instructions it emitted before `insert` shared
/// its implementation: materializing a constant 0 into a register would add
/// instructions and churn a committed golden for no gain.
#[derive(Debug, Clone, Copy)]
pub(crate) enum SpliceAt {
    /// Index 0 — `collections::prepend`.
    Front,
    /// A runtime index in this frame slot — `collections::insert`.
    At(usize),
}

/// bug-732: the frame slots [`CodeBuilder::emit_list_packed_length`] fills — the
/// data length a copy of the list carries, and whether it packs (1) or copies
/// verbatim (0).
pub(crate) struct PackedLength {
    length_slot: usize,
    packs_slot: usize,
}

impl CodeBuilder<'_> {
    /// bug-732: the data length a copy of the entry list in `list_slot` needs —
    /// its `dataLength`, or, when its dead bytes (left by in-place removals)
    /// outnumber its live ones, the aligned end of its live payloads packed in
    /// entry order. Walks the entries once, which the copy's own entry copy
    /// already costs.
    fn emit_list_packed_length(
        &mut self,
        list_slot: usize,
        element_type: &ParameterType,
    ) -> PackedLength {
        let alignment = self.list_element_padding_alignment(element_type);
        let stride = list_entry_stride(element_type);
        let length_slot = self.allocate_stack_object("list_packed_length", 8);
        let packs_slot = self.allocate_stack_object("list_packed_packs", 8);
        let base = self.temporary_vreg();
        let count = self.temporary_vreg();
        let entry = self.temporary_vreg();
        let index = self.temporary_vreg();
        let live = self.temporary_vreg();
        let len = self.temporary_vreg();
        let scratch = self.temporary_vreg();
        let walk = self.label("list_packed_walk");
        let walked = self.label("list_packed_walked");
        let verbatim = self.label("list_packed_verbatim");
        let decided = self.label("list_packed_decided");
        self.emit(abi::load_u64(&base, abi::stack_pointer(), list_slot));
        self.emit(abi::load_u64(&count, &base, COLLECTION_OFFSET_COUNT));
        self.emit(abi::add_immediate(&entry, &base, COLLECTION_HEADER_SIZE));
        self.emit(abi::move_immediate(&index, "Integer", "0"));
        self.emit(abi::move_immediate(&live, "Integer", "0"));
        self.emit(abi::label(&walk));
        self.emit(abi::compare_registers(&index, &count));
        self.emit(abi::branch_ge(&walked));
        self.emit_align_offset_register(&live, alignment, &scratch);
        self.emit(abi::load_u64(
            &len,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers(&live, &live, &len));
        self.emit(abi::add_immediate(&entry, &entry, stride));
        self.emit(abi::add_immediate(&index, &index, 1));
        self.emit(abi::branch(&walk));
        self.emit(abi::label(&walked));
        self.emit_align_offset_register(&live, alignment, &scratch);
        // Pack only when dataLength > 2 * live; `dataLength - live` can underflow
        // (see `emit_list_reclaim_before_grow`).
        self.emit(abi::load_u64(&len, &base, COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::add_registers(&scratch, &live, &live));
        self.emit(abi::compare_registers(&len, &scratch));
        self.emit(abi::branch_ls(&verbatim));
        self.emit(abi::store_u64(&live, abi::stack_pointer(), length_slot));
        self.emit(abi::move_immediate(&scratch, "Integer", "1"));
        self.emit(abi::store_u64(&scratch, abi::stack_pointer(), packs_slot));
        self.emit(abi::branch(&decided));
        self.emit(abi::label(&verbatim));
        self.emit(abi::store_u64(&len, abi::stack_pointer(), length_slot));
        self.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), packs_slot));
        self.emit(abi::label(&decided));
        PackedLength {
            length_slot,
            packs_slot,
        }
    }

    /// bug-732: when [`Self::emit_list_packed_length`] chose to pack A, copy A's
    /// payloads into the result `lower_list_insert_collection` built, one per A
    /// entry in A's order (the result's entries `[0, index)` then
    /// `[index + count_B, count)`), packed from data offset 0, rewriting each
    /// entry's `valueOffset`. The entries still hold A's offsets, which locate
    /// the payloads in A.
    fn emit_list_insert_pack_a(
        &mut self,
        base_slot: usize,
        index_slot: usize,
        insert_slot: usize,
        result_slot: usize,
        packed: &PackedLength,
        element_type: &ParameterType,
    ) {
        let alignment = self.list_element_padding_alignment(element_type);
        let stride = list_entry_stride(element_type);
        let top = self.label("list_insert_pack_loop");
        let copy = self.label("list_insert_pack_copy");
        let advance = self.label("list_insert_pack_next");
        let done = self.label("list_insert_pack_done");
        let a = self.temporary_vreg();
        let nb = self.temporary_vreg();
        let src_data = self.temporary_vreg();
        let dst_data = self.temporary_vreg();
        let entry = self.temporary_vreg();
        let index = self.temporary_vreg();
        let total = self.temporary_vreg();
        let gap_start = self.temporary_vreg();
        let gap_end = self.temporary_vreg();
        let cursor = self.temporary_vreg();
        let off = self.temporary_vreg();
        let len = self.temporary_vreg();
        let src = self.temporary_vreg();
        let dst = self.temporary_vreg();
        let scratch = self.temporary_vreg();
        self.emit(abi::load_u64(
            &scratch,
            abi::stack_pointer(),
            packed.packs_slot,
        ));
        self.emit(abi::compare_immediate(&scratch, "0"));
        self.emit(abi::branch_eq(&done));
        self.emit(abi::load_u64(&a, abi::stack_pointer(), base_slot));
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), result_slot));
        self.emit_collection_data_pointer_for(&src_data, &a, element_type);
        self.emit_collection_data_pointer_for(&dst_data, &nb, element_type);
        self.emit(abi::load_u64(&total, &nb, COLLECTION_OFFSET_COUNT));
        // B's entries sit at [index, index + count_B): not A's, skipped.
        self.emit(abi::load_u64(&gap_start, abi::stack_pointer(), index_slot));
        self.emit(abi::load_u64(&scratch, abi::stack_pointer(), insert_slot));
        self.emit(abi::load_u64(&gap_end, &scratch, COLLECTION_OFFSET_COUNT));
        self.emit(abi::add_registers(&gap_end, &gap_end, &gap_start));
        self.emit(abi::add_immediate(&entry, &nb, COLLECTION_HEADER_SIZE));
        self.emit(abi::move_immediate(&index, "Integer", "0"));
        self.emit(abi::move_immediate(&cursor, "Integer", "0"));
        self.emit(abi::label(&top));
        self.emit(abi::compare_registers(&index, &total));
        self.emit(abi::branch_ge(&done));
        self.emit(abi::compare_registers(&index, &gap_start));
        self.emit(abi::branch_lt(&copy));
        self.emit(abi::compare_registers(&index, &gap_end));
        self.emit(abi::branch_lt(&advance));
        self.emit(abi::label(&copy));
        self.emit_align_offset_register(&cursor, alignment, &scratch);
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
        self.emit(abi::store_u64(
            &cursor,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::add_registers(&src, &src_data, &off));
        self.emit(abi::add_registers(&dst, &dst_data, &cursor));
        self.emit(abi::add_registers(&cursor, &cursor, &len));
        self.emit_block_copy_advance(&dst, &src, &len, &scratch, "list_insert_pack_payload");
        self.emit(abi::label(&advance));
        self.emit(abi::add_immediate(&entry, &entry, stride));
        self.emit(abi::add_immediate(&index, &index, 1));
        self.emit(abi::branch(&top));
        self.emit(abi::label(&done));
    }

    /// bug-732: the bytes a bulk append writes into the list — the RHS's data
    /// region plus the worst-case pad before it — into a fresh slot, for
    /// [`Self::emit_list_reclaim_before_grow`].
    fn emit_bulk_append_extra(&mut self, rhs_slot: usize) -> usize {
        let slot = self.allocate_stack_object("bulk_append_extra", 8);
        let rhs = self.temporary_vreg();
        self.emit(abi::load_u64(&rhs, abi::stack_pointer(), rhs_slot));
        self.emit(abi::load_u64(&rhs, &rhs, COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::add_immediate(&rhs, &rhs, 7));
        self.emit(abi::store_u64(&rhs, abi::stack_pointer(), slot));
        slot
    }

    /// bug-732: the first step of every in-place grow of a variable-width list's
    /// data region — reclaim the dead bytes instead of copying them.
    ///
    /// An in-place removal (`removeAt`, a truncating `take`/`drop`/`mid`) that
    /// takes out a payload which is not the data tail leaves its bytes behind as
    /// dead space rather than moving every later payload: that is what keeps it
    /// costing the elements it removes and moves. Those bytes must not travel
    /// with the list forever, because a grow copies `dataLength` verbatim and
    /// steps the capacity from it — a loop that keeps removing and adding would
    /// grow the block without bound. So a grow first walks the entries once
    /// (O(count), which the grow's own entry copy already costs) to total the live
    /// bytes, and when the dead bytes outnumber them, repacks the payloads into a
    /// block sized from the live bytes (`emit_repack_list_data`, with room for the
    /// `need_slot` bytes about to be written) and jumps back to `retry`, the
    /// caller's room check. The repacked block has no dead bytes, so the second
    /// pass never repacks again; if the entry table is still full it takes the
    /// ordinary grow. With dead ≤ live at every verbatim grow, the data capacity
    /// stays within a constant factor of the live bytes.
    ///
    /// Emits nothing for a fixed-width (kind 2) list, which has no holes.
    /// `buffer_slot` holds the list (for a record field, its sub-block address,
    /// with `inline` naming the record).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn emit_list_reclaim_before_grow(
        &mut self,
        buffer_slot: usize,
        need_slot: usize,
        list_type: &ParameterType,
        element_type: &ParameterType,
        inline: Option<crate::codegen::collection::map::map_mutate::InlineGrow>,
        retry: &str,
    ) -> Result<(), String> {
        let stride = list_entry_stride(element_type);
        if stride == 0 {
            return Ok(());
        }
        let alignment = self.list_element_padding_alignment(element_type);
        let base = self.temporary_vreg();
        let count = self.temporary_vreg();
        let entry = self.temporary_vreg();
        let index = self.temporary_vreg();
        let live = self.temporary_vreg();
        let len = self.temporary_vreg();
        let scratch = self.temporary_vreg();
        let walk = self.label("list_reclaim_walk");
        let walked = self.label("list_reclaim_walked");
        let grow = self.label("list_reclaim_grow");
        self.emit(abi::load_u64(&base, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&count, &base, COLLECTION_OFFSET_COUNT));
        self.emit(abi::add_immediate(&entry, &base, COLLECTION_HEADER_SIZE));
        self.emit(abi::move_immediate(&index, "Integer", "0"));
        self.emit(abi::move_immediate(&live, "Integer", "0"));
        self.emit(abi::label(&walk));
        self.emit(abi::compare_registers(&index, &count));
        self.emit(abi::branch_ge(&walked));
        self.emit_align_offset_register(&live, alignment, &scratch);
        self.emit(abi::load_u64(
            &len,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers(&live, &live, &len));
        self.emit(abi::add_immediate(&entry, &entry, stride));
        self.emit(abi::add_immediate(&index, &index, 1));
        self.emit(abi::branch(&walk));
        self.emit(abi::label(&walked));
        self.emit_align_offset_register(&live, alignment, &scratch);
        // Repack when the dead bytes outnumber the live ones (dataLength > 2 *
        // live) — or, while the entry table still has room, when there are any
        // dead bytes at all (dataLength > live). The second rule is what keeps the
        // ENTRY capacity bounded: the verbatim grow steps `capacity` as well as
        // `dataCapacity`, and a repack sizes the data region from the live bytes
        // (smaller than before), so a list that keeps overflowing its data while
        // holding a few elements alternated repack / verbatim grow and doubled
        // its entry capacity on every verbatim grow — exponential memory for a
        // list of two strings (bug-732 Correction C7). A data-only overflow over dead bytes is
        // now always a repack, which keeps `capacity`.
        //
        // Both tests compare, never subtract: `live` counts the entry-order
        // alignment pads, and an out-of-order data region can hold fewer, so
        // `dataLength - live` can underflow (bug-733's map reclaim looped
        // forever on exactly that). Right after a repack `dataLength == live`, so
        // neither test holds and the retry cannot repack again.
        let any_dead = self.label("list_reclaim_any_dead");
        let cap = self.temporary_vreg();
        self.emit(abi::load_u64(&len, &base, COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::load_u64(&cap, &base, COLLECTION_OFFSET_CAPACITY));
        self.emit(abi::move_register(&scratch, &live));
        self.emit(abi::compare_registers(&count, &cap));
        self.emit(abi::branch_lo(&any_dead));
        self.emit(abi::add_registers(&scratch, &live, &live));
        self.emit(abi::label(&any_dead));
        self.emit(abi::compare_registers(&len, &scratch));
        self.emit(abi::branch_ls(&grow));
        self.emit_repack_list_data(
            buffer_slot,
            need_slot,
            list_type,
            element_type,
            stride,
            inline,
        )?;
        self.emit(abi::branch(retry));
        self.emit(abi::label(&grow));
        Ok(())
    }

    /// bug-627: grow a variable-width list block's **data capacity** and **repack**
    /// its live payloads, leaving `capacity` and the entry order as they are.
    ///
    /// A length-changing `set` writes a longer payload at the data tail and leaves
    /// its old span behind as dead bytes, so `dataLength` counts live payloads plus
    /// those holes. When a tail write no longer fits, this copies each entry's
    /// payload — located by its own `valueOffset`/`valueLength`, so any payload
    /// order is fine — into a fresh block, packed in entry order with the element
    /// alignment, and rewrites the offsets. The dead bytes are dropped here.
    ///
    /// `extra_slot` holds the byte count the caller writes next. The new data
    /// capacity is `step(live + extra + count)` (geometric, `emit_geometric_step`):
    /// sized from the LIVE bytes, so a loop that keeps rewriting a few elements
    /// cannot inflate the block, and with at least `count` + half the live bytes of
    /// headroom, so the O(count + live) repack is paid for by at least that many
    /// bytes of writes before the next one. Geometric, not tight: a tight block
    /// overflows on the very next widening (plan-121-F Correction F1).
    ///
    /// On return `buffer_slot` holds the new block, whose `dataLength` is the
    /// aligned end of the packed payloads; the old block is freed. Payload bytes
    /// are copied verbatim, so a pointer edge inside one keeps its target, exactly
    /// as the verbatim grow this replaces did.
    ///
    /// plan-145-E: `inline` is `Some` for a list inlined in a record or `STATE`
    /// field. The new block is then the RECORD (`fieldOffset` + the list):
    /// `InlineGrow`'s three steps copy the prefix, the packed list is built at the
    /// sub-block inside it, and the old record is freed; `buffer_slot` (the
    /// sub-block address) is repointed into the new record.
    pub(crate) fn emit_repack_list_data(
        &mut self,
        buffer_slot: usize,
        extra_slot: usize,
        list_type: &ParameterType,
        element_type: &ParameterType,
        entry_stride: usize,
        inline: Option<crate::codegen::collection::map::map_mutate::InlineGrow>,
    ) -> Result<(), String> {
        let layout = CollectionTypeLayout::from_type(list_type)
            .ok_or_else(|| format!("native code collection type '{list_type}' is not supported"))?;
        let value_alignment = self.list_element_padding_alignment(element_type);
        let base = self.temporary_vreg();
        let nb = self.temporary_vreg();
        let cap = self.temporary_vreg();
        let cnt = self.temporary_vreg();
        let entry = self.temporary_vreg();
        let index = self.temporary_vreg();
        let cursor = self.temporary_vreg();
        let len = self.temporary_vreg();
        let off = self.temporary_vreg();
        let step = self.temporary_vreg();
        let tmp = self.temporary_vreg();
        let src = self.temporary_vreg();
        let dst = self.temporary_vreg();
        let sdata = self.temporary_vreg();
        let ddata = self.temporary_vreg();
        let cpy = self.temporary_vreg();
        let align_scratch = self.temporary_vreg();

        let new_buf_slot = self.allocate_stack_object("set_repack_newbuf", 8);
        let new_dcap_slot = self.allocate_stack_object("set_repack_newdcap", 8);

        let len_loop = self.label("set_repack_len_loop");
        let len_done = self.label("set_repack_len_done");
        let copy_loop = self.label("set_repack_copy_loop");
        let copy_done = self.label("set_repack_copy_done");
        let keep = self.label("set_repack_keep");
        let alloc_ok = self.label("set_repack_alloc_ok");
        let size_overflow = self.label("set_repack_size_overflow");

        // Length pass: live = the aligned end of every payload packed in entry order.
        self.emit(abi::load_u64(&base, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&cnt, &base, COLLECTION_OFFSET_COUNT));
        self.emit(abi::add_immediate(&entry, &base, COLLECTION_HEADER_SIZE));
        self.emit(abi::move_immediate(&index, "Integer", "0"));
        self.emit(abi::move_immediate(&cursor, "Integer", "0"));
        self.emit(abi::label(&len_loop));
        self.emit(abi::compare_registers(&index, &cnt));
        self.emit(abi::branch_ge(&len_done));
        self.emit_align_offset_register(&cursor, value_alignment, &align_scratch);
        self.emit(abi::load_u64(
            &len,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers(&cursor, &cursor, &len));
        self.emit(abi::add_immediate(&entry, &entry, entry_stride));
        self.emit(abi::add_immediate(&index, &index, 1));
        self.emit(abi::branch(&len_loop));
        self.emit(abi::label(&len_done));
        self.emit_align_offset_register(&cursor, value_alignment, &align_scratch);

        // newDataCapacity = step(live + extra + count), never below that sum.
        self.emit(abi::load_u64(&tmp, abi::stack_pointer(), extra_slot));
        self.emit(abi::add_registers(&len, &cursor, &tmp));
        self.emit(abi::add_registers(&len, &len, &cnt));
        self.emit_geometric_step(
            &len,
            &step,
            &tmp,
            COLLECTION_GROW_DATA_INIT,
            COLLECTION_GROW_DATA_TAPER,
            "set_grow_dcap",
        );
        self.emit(abi::compare_registers(&step, &len));
        self.emit(abi::branch_hi(&keep));
        self.emit(abi::branch_eq(&keep));
        self.emit(abi::move_register(&step, &len));
        self.emit(abi::label(&keep));
        self.emit(abi::store_u64(&step, abi::stack_pointer(), new_dcap_slot));

        // allocSize = HEADER + capacity*ENTRY + newDataCapacity, checked
        // (bug-147.7: capacity and dataCapacity are runtime-derived).
        self.emit(abi::load_u64(&base, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&cap, &base, COLLECTION_OFFSET_CAPACITY));
        self.emit(abi::move_immediate(
            &tmp,
            "Integer",
            &entry_stride.to_string(),
        ));
        self.emit_checked_size_multiply(&len, &cap, &tmp, &size_overflow);
        self.emit_checked_size_add_immediate(
            abi::return_register(),
            &len,
            COLLECTION_HEADER_SIZE,
            &size_overflow,
        );
        self.emit(abi::load_u64(&step, abi::stack_pointer(), new_dcap_slot));
        self.emit_checked_size_add(
            abi::return_register(),
            abi::return_register(),
            &step,
            &size_overflow,
        );
        if let Some(g) = inline {
            self.emit_inline_grow_extend_size(&g, &size_overflow);
        }
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&alloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&size_overflow));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            new_buf_slot,
        ));
        let new_rec = inline.map(|g| self.emit_inline_grow_split(&g, new_buf_slot));

        // The header and the entry table copy verbatim: `capacity` is unchanged, so
        // the data region keeps its block-relative base.
        self.emit(abi::load_u64(&base, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), new_buf_slot));
        self.emit(abi::load_u64(&cap, &base, COLLECTION_OFFSET_CAPACITY));
        self.emit(abi::move_immediate(
            &tmp,
            "Integer",
            &entry_stride.to_string(),
        ));
        self.emit(abi::multiply_registers(&len, &cap, &tmp));
        self.emit(abi::add_immediate(&len, &len, COLLECTION_HEADER_SIZE));
        self.emit(abi::move_register(&src, &base));
        self.emit(abi::move_register(&dst, &nb));
        self.emit_block_copy_advance(&dst, &src, &len, &cpy, "set_repack_head");

        // Copy pass: each payload to the running aligned cursor, rewriting its offset.
        self.emit(abi::load_u64(&base, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), new_buf_slot));
        self.emit_collection_data_pointer_for(&sdata, &base, element_type);
        self.emit_collection_data_pointer_for(&ddata, &nb, element_type);
        self.emit(abi::load_u64(&cnt, &base, COLLECTION_OFFSET_COUNT));
        self.emit(abi::add_immediate(&entry, &nb, COLLECTION_HEADER_SIZE));
        self.emit(abi::move_immediate(&index, "Integer", "0"));
        self.emit(abi::move_immediate(&cursor, "Integer", "0"));
        self.emit(abi::label(&copy_loop));
        self.emit(abi::compare_registers(&index, &cnt));
        self.emit(abi::branch_ge(&copy_done));
        self.emit_align_offset_register(&cursor, value_alignment, &align_scratch);
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
        self.emit(abi::store_u64(
            &cursor,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::add_registers(&src, &sdata, &off));
        self.emit(abi::add_registers(&dst, &ddata, &cursor));
        self.emit(abi::add_registers(&cursor, &cursor, &len));
        self.emit_block_copy_advance(&dst, &src, &len, &cpy, "set_repack_payload");
        self.emit(abi::add_immediate(&entry, &entry, entry_stride));
        self.emit(abi::add_immediate(&index, &index, 1));
        self.emit(abi::branch(&copy_loop));
        self.emit(abi::label(&copy_done));
        self.emit_align_offset_register(&cursor, value_alignment, &align_scratch);

        // Header: same count / capacity, the packed dataLength, the new dataCapacity.
        self.emit(abi::load_u64(&step, abi::stack_pointer(), new_dcap_slot));
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), new_buf_slot));
        self.emit(abi::load_u64(&cap, &nb, COLLECTION_OFFSET_CAPACITY));
        self.emit_write_collection_header_full(&layout, &nb, &cnt, &cap, &cursor, &step);

        // Free the old block (for an inlined list, the old RECORD), then publish
        // the new pointer.
        match (inline, new_rec) {
            (Some(g), Some(new_rec_slot)) => {
                self.emit_inline_grow_free_old(&g, buffer_slot, list_type, new_rec_slot)?
            }
            _ => self.emit_free_pre_grow_buffer(buffer_slot, list_type)?,
        }
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), new_buf_slot));
        self.emit(abi::store_u64(&nb, abi::stack_pointer(), buffer_slot));
        Ok(())
    }
}
