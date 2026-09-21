//! plan-142-D: shrink a `Map`/`Set` **in place** to a subset of its entries, and
//! reserve room for a batch of inserts before the first one.
//!
//! A map block is `[header][capacity × 40-byte entries][data][capacity × 16-byte
//! buckets]`; each entry locates its key and value payloads in the data region by
//! offset. `lower_map_compact_in_place` is the many-key form of
//! `lower_map_remove_key_in_place`: the survivors' entries move down in their
//! order (iteration order is insertion order, so it is preserved), a dropped
//! value's graph is freed first, and the hash index is marked not-ready so the
//! next probe rebuilds it. As for `removeKey`, a dropped entry's payload bytes
//! stay behind in the data region.
//!
//! `emit_map_reserve` grows a map once, geometrically, so that a following batch of
//! `lower_map_set_in_place` calls never reallocates mid-batch: the only
//! allocation — and so the only possible `ErrOutOfMemory` — comes before the first
//! write, which keeps a multi-key self-update (`union`, `merge`, …) failure-atomic.

use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::typed_map_type_parts;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::types::ParameterType;

impl CodeBuilder<'_> {
    /// Keep the entries of the map in `map_slot` whose mark byte (at the address in
    /// `marks_slot`, one per entry) is non-zero; drop the rest. Cannot fail and
    /// allocates nothing.
    pub(crate) fn lower_map_compact_in_place(
        &mut self,
        map_slot: usize,
        marks_slot: usize,
        map_type: &ParameterType,
    ) -> Result<(), String> {
        let value_type = typed_map_type_parts(map_type)
            .map(|(_, value)| value.clone())
            .or_else(|| {
                crate::codegen::engine::types::typed_set_element_type(map_type)
                    .map(|_| ParameterType::Boolean)
            })
            .ok_or_else(|| format!("native in-place map compaction of non-map {map_type}"))?;
        // A dropped value that owns a graph is freed before its entry is
        // overwritten; the drop calls, so this is its own pass over slots.
        if self.owns_graph(&value_type) {
            let index_slot = self.allocate_stack_object("mcompact_drop_index", 8);
            let entry_slot = self.allocate_stack_object("mcompact_drop_entry", 8);
            let top = self.label("mcompact_drop_loop");
            let next = self.label("mcompact_drop_next");
            let done = self.label("mcompact_drop_done");
            self.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), index_slot));
            self.emit(abi::label(&top));
            let base = self.temporary_vreg();
            let count = self.temporary_vreg();
            let index = self.temporary_vreg();
            let mark = self.temporary_vreg();
            self.emit(abi::load_u64(&base, abi::stack_pointer(), map_slot));
            self.emit(abi::load_u64(&count, &base, COLLECTION_OFFSET_COUNT));
            self.emit(abi::load_u64(&index, abi::stack_pointer(), index_slot));
            self.emit(abi::compare_registers(&index, &count));
            self.emit(abi::branch_ge(&done));
            self.emit(abi::load_u64(&mark, abi::stack_pointer(), marks_slot));
            self.emit(abi::add_registers(&mark, &mark, &index));
            self.emit(abi::load_u8(&mark, &mark, 0));
            self.emit(abi::compare_immediate(&mark, "0"));
            self.emit(abi::branch_ne(&next));
            let entry = self.temporary_vreg();
            self.emit(abi::move_immediate(
                &entry,
                "Integer",
                &COLLECTION_ENTRY_SIZE.to_string(),
            ));
            self.emit(abi::multiply_registers(&entry, &index, &entry));
            self.emit(abi::add_registers(&entry, &entry, &base));
            self.emit(abi::add_immediate(&entry, &entry, COLLECTION_HEADER_SIZE));
            self.emit(abi::store_u64(&entry, abi::stack_pointer(), entry_slot));
            self.emit_drop_entry_value(map_slot, entry_slot, &value_type)?;
            self.emit(abi::label(&next));
            let index = self.temporary_vreg();
            self.emit(abi::load_u64(&index, abi::stack_pointer(), index_slot));
            self.emit(abi::add_immediate(&index, &index, 1));
            self.emit(abi::store_u64(&index, abi::stack_pointer(), index_slot));
            self.emit(abi::branch(&top));
            self.emit(abi::label(&done));
        }
        // Move each survivor's entry to the next free position.
        let base = self.temporary_vreg();
        let count = self.temporary_vreg();
        let marks = self.temporary_vreg();
        let index = self.temporary_vreg();
        let kept = self.temporary_vreg();
        let mark = self.temporary_vreg();
        let src = self.temporary_vreg();
        let dst = self.temporary_vreg();
        let len = self.temporary_vreg();
        let copy = self.temporary_vreg();
        let top = self.label("mcompact_loop");
        let keep = self.label("mcompact_keep");
        let in_place = self.label("mcompact_in_place");
        let next = self.label("mcompact_next");
        let done = self.label("mcompact_done");
        self.emit(abi::load_u64(&base, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(&count, &base, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(&marks, abi::stack_pointer(), marks_slot));
        self.emit(abi::move_immediate(&index, "Integer", "0"));
        self.emit(abi::move_immediate(&kept, "Integer", "0"));
        self.emit(abi::label(&top));
        self.emit(abi::compare_registers(&index, &count));
        self.emit(abi::branch_ge(&done));
        self.emit(abi::add_registers(&mark, &marks, &index));
        self.emit(abi::load_u8(&mark, &mark, 0));
        self.emit(abi::compare_immediate(&mark, "0"));
        self.emit(abi::branch_ne(&keep));
        self.emit(abi::branch(&next));
        self.emit(abi::label(&keep));
        self.emit(abi::compare_registers(&index, &kept));
        self.emit(abi::branch_eq(&in_place));
        self.emit(abi::move_immediate(
            &len,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&src, &index, &len));
        self.emit(abi::add_registers(&src, &src, &base));
        self.emit(abi::add_immediate(&src, &src, COLLECTION_HEADER_SIZE));
        self.emit(abi::multiply_registers(&dst, &kept, &len));
        self.emit(abi::add_registers(&dst, &dst, &base));
        self.emit(abi::add_immediate(&dst, &dst, COLLECTION_HEADER_SIZE));
        self.emit_block_copy_advance(&dst, &src, &len, &copy, "mcompact_entry");
        self.emit(abi::label(&in_place));
        self.emit(abi::add_immediate(&kept, &kept, 1));
        self.emit(abi::label(&next));
        self.emit(abi::add_immediate(&index, &index, 1));
        self.emit(abi::branch(&top));
        self.emit(abi::label(&done));
        self.emit(abi::store_u64(&kept, &base, COLLECTION_OFFSET_COUNT));
        let zero = self.temporary_vreg();
        self.emit(abi::move_immediate(&zero, "Byte", "0"));
        self.emit(abi::store_u8(&zero, &base, COLLECTION_OFFSET_BUCKETS_READY));
        Ok(())
    }

    /// Empty the map in `map_slot` in place — the result of `difference(s, s)` and
    /// `symmetricDifference(s, s)`. Frees any value graphs, then zeroes `count` and
    /// `dataLength` and marks the index not-ready.
    pub(crate) fn lower_map_clear_in_place(
        &mut self,
        map_slot: usize,
        map_type: &ParameterType,
    ) -> Result<(), String> {
        // Free every value's graph first (a map with no graph-owning values — every
        // `Set`, and most maps — skips this pass entirely).
        let value_type = typed_map_type_parts(map_type).map(|(_, value)| value.clone());
        if value_type
            .as_ref()
            .is_some_and(|value| self.owns_graph(value))
        {
            let index_slot = self.allocate_stack_object("mclear_index", 8);
            let entry_slot = self.allocate_stack_object("mclear_entry", 8);
            let top = self.label("mclear_drop_loop");
            let done = self.label("mclear_drop_done");
            self.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), index_slot));
            self.emit(abi::label(&top));
            let base = self.temporary_vreg();
            let count = self.temporary_vreg();
            let index = self.temporary_vreg();
            self.emit(abi::load_u64(&base, abi::stack_pointer(), map_slot));
            self.emit(abi::load_u64(&count, &base, COLLECTION_OFFSET_COUNT));
            self.emit(abi::load_u64(&index, abi::stack_pointer(), index_slot));
            self.emit(abi::compare_registers(&index, &count));
            self.emit(abi::branch_ge(&done));
            let entry = self.temporary_vreg();
            self.emit(abi::move_immediate(
                &entry,
                "Integer",
                &COLLECTION_ENTRY_SIZE.to_string(),
            ));
            self.emit(abi::multiply_registers(&entry, &index, &entry));
            self.emit(abi::add_registers(&entry, &entry, &base));
            self.emit(abi::add_immediate(&entry, &entry, COLLECTION_HEADER_SIZE));
            self.emit(abi::store_u64(&entry, abi::stack_pointer(), entry_slot));
            self.emit_drop_entry_value(map_slot, entry_slot, value_type.as_ref().unwrap())?;
            let index = self.temporary_vreg();
            self.emit(abi::load_u64(&index, abi::stack_pointer(), index_slot));
            self.emit(abi::add_immediate(&index, &index, 1));
            self.emit(abi::store_u64(&index, abi::stack_pointer(), index_slot));
            self.emit(abi::branch(&top));
            self.emit(abi::label(&done));
        }
        let base = self.temporary_vreg();
        let zero = self.temporary_vreg();
        self.emit(abi::load_u64(&base, abi::stack_pointer(), map_slot));
        self.emit(abi::store_u64(abi::ZERO, &base, COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64(
            abi::ZERO,
            &base,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::move_immediate(&zero, "Byte", "0"));
        self.emit(abi::store_u8(&zero, &base, COLLECTION_OFFSET_BUCKETS_READY));
        Ok(())
    }

    /// Make the map in `map_slot` able to take `extra_entries_slot` more entries
    /// and `extra_bytes_slot` more data bytes without reallocating, growing it once
    /// (geometrically) if not. The entries and data are copied verbatim — the data
    /// region's base moves with the capacity, and every entry locates its payloads
    /// relative to that base — and the new block's hash index starts not-ready.
    pub(crate) fn emit_map_reserve(
        &mut self,
        map_slot: usize,
        extra_entries_slot: usize,
        extra_bytes_slot: usize,
        map_type: &ParameterType,
    ) -> Result<(), String> {
        let layout = CollectionTypeLayout::from_type(map_type)
            .ok_or_else(|| format!("native code collection type '{map_type}' is not supported"))?;
        let new_cap_slot = self.allocate_stack_object("mreserve_newcap", 8);
        let new_dcap_slot = self.allocate_stack_object("mreserve_newdcap", 8);
        let new_buf_slot = self.allocate_stack_object("mreserve_newbuf", 8);
        let grow = self.label("mreserve_grow");
        let fits = self.label("mreserve_fits");
        let cap_ok = self.label("mreserve_cap_ok");
        let dcap_ok = self.label("mreserve_dcap_ok");
        let alloc_ok = self.label("mreserve_alloc_ok");
        let overflow = self.label("mreserve_size_overflow");

        let base = self.temporary_vreg();
        let count = self.temporary_vreg();
        let cap = self.temporary_vreg();
        let dlen = self.temporary_vreg();
        let dcap = self.temporary_vreg();
        let extra = self.temporary_vreg();
        let need = self.temporary_vreg();
        let step = self.temporary_vreg();
        let scratch = self.temporary_vreg();

        // Room check: count + extraEntries <= capacity AND dataLength + extraBytes
        // <= dataCapacity.
        self.emit(abi::load_u64(&base, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(&count, &base, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(&cap, &base, COLLECTION_OFFSET_CAPACITY));
        self.emit(abi::load_u64(
            &extra,
            abi::stack_pointer(),
            extra_entries_slot,
        ));
        self.emit(abi::add_registers(&need, &count, &extra));
        self.emit(abi::compare_registers(&need, &cap));
        self.emit(abi::branch_hi(&grow));
        self.emit(abi::load_u64(&dlen, &base, COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::load_u64(&dcap, &base, COLLECTION_OFFSET_DATA_CAPACITY));
        self.emit(abi::load_u64(
            &extra,
            abi::stack_pointer(),
            extra_bytes_slot,
        ));
        self.emit(abi::add_registers(&need, &dlen, &extra));
        self.emit(abi::compare_registers(&need, &dcap));
        self.emit(abi::branch_hi(&grow));
        self.emit(abi::branch(&fits));

        // newCapacity = max(step(capacity), count + extraEntries);
        // newDataCapacity = max(step(dataCapacity), dataLength + extraBytes).
        self.emit(abi::label(&grow));
        self.emit(abi::load_u64(&base, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(&cap, &base, COLLECTION_OFFSET_CAPACITY));
        self.emit_geometric_step(
            &cap,
            &step,
            &scratch,
            COLLECTION_GROW_LOOKUP_INIT,
            COLLECTION_GROW_LOOKUP_TAPER,
            "mreserve_cap",
        );
        self.emit(abi::load_u64(&count, &base, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(
            &extra,
            abi::stack_pointer(),
            extra_entries_slot,
        ));
        self.emit(abi::add_registers(&need, &count, &extra));
        self.emit(abi::compare_registers(&step, &need));
        self.emit(abi::branch_hi(&cap_ok));
        self.emit(abi::move_register(&step, &need));
        self.emit(abi::label(&cap_ok));
        self.emit(abi::store_u64(&step, abi::stack_pointer(), new_cap_slot));
        self.emit(abi::load_u64(&dcap, &base, COLLECTION_OFFSET_DATA_CAPACITY));
        self.emit_geometric_step(
            &dcap,
            &step,
            &scratch,
            COLLECTION_GROW_DATA_INIT,
            COLLECTION_GROW_DATA_TAPER,
            "mreserve_dcap",
        );
        self.emit(abi::load_u64(&dlen, &base, COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::load_u64(
            &extra,
            abi::stack_pointer(),
            extra_bytes_slot,
        ));
        self.emit(abi::add_registers(&need, &dlen, &extra));
        self.emit(abi::compare_registers(&step, &need));
        self.emit(abi::branch_hi(&dcap_ok));
        self.emit(abi::move_register(&step, &need));
        self.emit(abi::label(&dcap_ok));
        self.emit(abi::store_u64(&step, abi::stack_pointer(), new_dcap_slot));

        // size = HEADER + newCapacity*ENTRY + newDataCapacity + newCapacity*16
        // (the block-size authority for a Map/Set), checked.
        let size = self.temporary_vreg();
        let buckets = self.temporary_vreg();
        self.emit(abi::load_u64(&cap, abi::stack_pointer(), new_cap_slot));
        self.emit(abi::move_immediate(
            &scratch,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit_checked_size_multiply(&size, &cap, &scratch, &overflow);
        self.emit(abi::move_immediate(&scratch, "Integer", "16"));
        self.emit_checked_size_multiply(&buckets, &cap, &scratch, &overflow);
        self.emit_checked_size_add(&size, &size, &buckets, &overflow);
        self.emit(abi::load_u64(&dcap, abi::stack_pointer(), new_dcap_slot));
        self.emit_checked_size_add(&size, &size, &dcap, &overflow);
        self.emit_checked_size_add_immediate(
            abi::return_register(),
            &size,
            COLLECTION_HEADER_SIZE,
            &overflow,
        );
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&alloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&overflow));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            new_buf_slot,
        ));
        // Header: same count / dataLength, the new capacities; index not-ready.
        let nb = self.temporary_vreg();
        let count = self.temporary_vreg();
        let cap = self.temporary_vreg();
        let dlen = self.temporary_vreg();
        let dcap = self.temporary_vreg();
        let old = self.temporary_vreg();
        self.emit(abi::load_u64(&old, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(&count, &old, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(&dlen, &old, COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::load_u64(&cap, abi::stack_pointer(), new_cap_slot));
        self.emit(abi::load_u64(&dcap, abi::stack_pointer(), new_dcap_slot));
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), new_buf_slot));
        self.emit_write_collection_header_full(&layout, &nb, &count, &cap, &dlen, &dcap);
        // Entries: count * ENTRY bytes, verbatim.
        let src = self.temporary_vreg();
        let dst = self.temporary_vreg();
        let len = self.temporary_vreg();
        let copy = self.temporary_vreg();
        self.emit(abi::load_u64(&old, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), new_buf_slot));
        self.emit(abi::add_immediate(&src, &old, COLLECTION_HEADER_SIZE));
        self.emit(abi::add_immediate(&dst, &nb, COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64(&len, &old, COLLECTION_OFFSET_COUNT));
        self.emit(abi::move_immediate(
            &scratch,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&len, &len, &scratch));
        self.emit_block_copy_advance(&dst, &src, &len, &copy, "mreserve_entries");
        // Data: dataLength bytes, from the old data base to the new one.
        self.emit(abi::load_u64(&old, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), new_buf_slot));
        self.emit_collection_data_pointer_for(&src, &old, &ParameterType::named(""));
        self.emit_collection_data_pointer_for(&dst, &nb, &ParameterType::named(""));
        self.emit(abi::load_u64(&len, &old, COLLECTION_OFFSET_DATA_LENGTH));
        self.emit_block_copy_advance(&dst, &src, &len, &copy, "mreserve_data");
        // Free the old block, then publish the new one.
        self.emit_free_pre_grow_buffer(map_slot, map_type)?;
        let nb = self.temporary_vreg();
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), new_buf_slot));
        self.emit(abi::store_u64(&nb, abi::stack_pointer(), map_slot));
        self.emit(abi::label(&fits));
        Ok(())
    }
}
