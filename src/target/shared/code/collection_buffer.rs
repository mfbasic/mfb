use super::*;

impl CodeBuilder<'_> {
    /// Returns `(slot, materialized)`: `materialized` is true when the item was
    /// wrapped in a freshly arena-allocated singleton list the CALLER must free
    /// after the consuming insert copied out of it (via
    /// [`Self::free_intermediate_collection`]) — leaving it live leaked one
    /// block per value-path append/prepend/insert/set (bug-01's fourth leak:
    /// ~40% of all allocations under `r = append(r, expr)` churn).
    pub(super) fn collection_argument_as_list_slot(
        &mut self,
        list_type: &str,
        element_type: &str,
        item: ValueResult,
    ) -> Result<(usize, bool), String> {
        if item.type_ == list_type {
            let slot = self.allocate_stack_object("collection_insert_list", 8);
            self.emit(abi::store_u64(&item.location, abi::stack_pointer(), slot));
            return Ok((slot, false));
        }
        if item.type_ != element_type {
            return Err(format!(
                "native collection list item must be {}, got {}",
                element_type, item.type_
            ));
        }
        let item_slot = self.allocate_stack_object("collection_insert_item", 8);
        self.emit(abi::store_u64(
            &item.location,
            abi::stack_pointer(),
            item_slot,
        ));
        let singleton = self.lower_collection_values(
            list_type,
            vec![CollectionValueSlot {
                key: None,
                value: PayloadSlot {
                    slot: item_slot,
                    type_: element_type.to_string(),
                },
            }],
            "singleton list",
        )?;
        let slot = self.allocate_stack_object("collection_insert_singleton", 8);
        self.emit(abi::store_u64(
            &singleton.location,
            abi::stack_pointer(),
            slot,
        ));
        Ok((slot, true))
    }

    /// Free an intermediate collection block (a materialized singleton or a
    /// consumed `removeAt` result) after the operation that copied out of it,
    /// preserving `result` across the `arena_free` call (which clobbers every
    /// caller-saved register). No-op for non-flat types, mirroring the
    /// reassignment free guard.
    pub(super) fn free_intermediate_collection(
        &mut self,
        block_slot: usize,
        type_: &str,
        result: ValueResult,
    ) -> Result<ValueResult, String> {
        if !self.is_freeable_flat_value(type_) {
            return Ok(result);
        }
        let keep = self.allocate_stack_object("intermediate_free_keep", 8);
        self.emit(abi::store_u64(&result.location, abi::stack_pointer(), keep));
        self.emit_owned_value_drop(&OwnedValueCleanup {
            type_: type_.to_string(),
            stack_offset: block_slot,
        })?;
        let register = self.allocate_register()?;
        self.emit(abi::load_u64(&register, abi::stack_pointer(), keep));
        Ok(ValueResult {
            type_: result.type_,
            location: register,
            text: String::new(),
        })
    }

    /// Free the backing buffer currently held in `slot` before an in-place grow
    /// installs a fresh block over it (bug-47, the bug-01 class at new sites). The
    /// in-place mutators fire only under unique ownership and never on a live
    /// `FOR EACH` iterable (`try_inplace_*` guards), so the abandoned block has no
    /// other reference — freeing it here is what keeps a `prepend`/`set`-in-a-loop
    /// program's arena footprint bounded by live data instead of leaking one block
    /// per geometric grow, exactly as `append`/`bulk_append` already do. Sizing
    /// (including a map's hash-bucket region) and the spill of the block pointer
    /// across the `arena_free` call (which trashes every caller-saved register) are
    /// handled by `free_intermediate_collection`; the returned pointer is the freed
    /// block and is intentionally discarded — the caller installs the new buffer
    /// from its own slot immediately after. Must be called while `slot` still holds
    /// the pre-grow buffer and after the copy into the new buffer has completed.
    pub(super) fn emit_free_pre_grow_buffer(
        &mut self,
        slot: usize,
        type_: &str,
    ) -> Result<(), String> {
        let keep = self.allocate_register()?;
        self.emit(abi::load_u64(&keep, abi::stack_pointer(), slot));
        let threaded = ValueResult {
            type_: type_.to_string(),
            location: keep,
            text: String::new(),
        };
        self.free_intermediate_collection(slot, type_, threaded)?;
        Ok(())
    }

    /// Write a tight collection header: `capacity == count`, `dataCapacity ==
    /// dataLength` (no headroom). Used by the splice/remove paths that produce a
    /// fresh exact-sized buffer (plan-01 §4.3 — snapshots stay tight).
    pub(super) fn emit_write_list_header_from_registers(
        &mut self,
        layout: &CollectionTypeLayout,
        collection: &str,
        count: &str,
        data_len: &str,
    ) {
        self.emit_write_collection_header_full(
            layout, collection, count, count, data_len, data_len,
        );
    }

    /// Write a collection header with `capacity`/`dataCapacity` distinct from the
    /// live `count`/`dataLength` — the headroom form used by the append grow path
    /// (plan-01 §4.2). `capacity >= count` and `dataCapacity >= dataLength` must
    /// hold; the data region base is computed from `capacity`, so the writer and
    /// every reader agree via `emit_collection_data_pointer`. Uses x22 scratch.
    pub(super) fn emit_write_collection_header_full(
        &mut self,
        layout: &CollectionTypeLayout,
        collection: &str,
        count: &str,
        capacity: &str,
        data_len: &str,
        data_cap: &str,
    ) {
        let scratch22 = self.temporary_vreg();
        self.emit(abi::move_immediate(
            &scratch22,
            "Byte",
            &layout.kind.to_string(),
        ));
        self.emit(abi::store_u8(
            &scratch22,
            collection,
            COLLECTION_OFFSET_KIND,
        ));
        self.emit(abi::move_immediate(
            &scratch22,
            "Byte",
            &layout.key_type_code.to_string(),
        ));
        self.emit(abi::store_u8(
            &scratch22,
            collection,
            COLLECTION_OFFSET_KEY_TYPE,
        ));
        self.emit(abi::move_immediate(
            &scratch22,
            "Byte",
            &layout.value_type_code.to_string(),
        ));
        self.emit(abi::store_u8(
            &scratch22,
            collection,
            COLLECTION_OFFSET_VALUE_TYPE,
        ));
        self.emit(abi::move_immediate(&scratch22, "Byte", "1"));
        self.emit(abi::store_u8(
            &scratch22,
            collection,
            COLLECTION_OFFSET_FLAGS_VERSION,
        ));
        // Mark the map hash index not-ready (built lazily on first probe); a no-op
        // field for lists. Fresh, grown, and copied collections all reset it here.
        self.emit(abi::move_immediate(&scratch22, "Byte", "0"));
        self.emit(abi::store_u8(
            &scratch22,
            collection,
            COLLECTION_OFFSET_BUCKETS_READY,
        ));
        self.emit(abi::store_u64(count, collection, COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64(
            capacity,
            collection,
            COLLECTION_OFFSET_CAPACITY,
        ));
        self.emit(abi::store_u64(
            data_len,
            collection,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::store_u64(
            data_cap,
            collection,
            COLLECTION_OFFSET_DATA_CAPACITY,
        ));
    }

    /// Emit `out_reg = geometric_step(value_reg)` per the plan-01 §5 growth shape:
    /// `0 -> init`; `value < threshold -> value * 2`; otherwise `value * 1.5`
    /// (taper the multiplier once large). All-integer, rounds down on the ×1.5
    /// (still strictly larger than `value`). `value_reg`, `out_reg`, and `scratch`
    /// must be three distinct registers; `value_reg` is preserved.
    pub(super) fn emit_geometric_step(
        &mut self,
        value_reg: &str,
        out_reg: &str,
        scratch: &str,
        init: usize,
        threshold: usize,
        prefix: &str,
    ) {
        let small = self.label(&format!("{prefix}_double"));
        let init_label = self.label(&format!("{prefix}_init"));
        let after = self.label(&format!("{prefix}_after"));
        self.emit(abi::compare_immediate(value_reg, "0"));
        self.emit(abi::branch_eq(&init_label));
        self.emit(abi::move_immediate(
            scratch,
            "Integer",
            &threshold.to_string(),
        ));
        self.emit(abi::compare_registers(value_reg, scratch));
        self.emit(abi::branch_lo(&small));
        // ×1.5: out = value + value/2.
        self.emit(abi::shift_right_immediate(out_reg, value_reg, 1));
        self.emit(abi::add_registers(out_reg, value_reg, out_reg));
        self.emit(abi::branch(&after));
        self.emit(abi::label(&small));
        self.emit(abi::shift_left_immediate(out_reg, value_reg, 1)); // ×2
        self.emit(abi::branch(&after));
        self.emit(abi::label(&init_label));
        self.emit(abi::move_immediate(out_reg, "Integer", &init.to_string()));
        self.emit(abi::label(&after));
    }

    /// Bulk-copy `count` list lookup entries (`count * ENTRY` bytes) verbatim from
    /// the entry-table cursor `src_entry` to `dst_entry` with a single word loop
    /// (`emit_block_copy_advance`), then — when `delta` is `Some((reg, subtract))`
    /// — shift each copied entry's `valueOffset` by the register `reg`
    /// (subtracting when `subtract`, else adding) in a tight fix-up loop. Copying
    /// the entries verbatim preserves each entry's `flags`, zeroed
    /// `keyOffset`/`keyLength`, and `valueLength` — sound because every source
    /// here is a well-formed list whose entries are already `flags = used` with
    /// zero key fields; only the payload's position in the destination data region
    /// moves, so only `valueOffset` needs the uniform shift. `src_entry`,
    /// `dst_entry`, and `count` are clobbered (advanced / decremented); a `delta`
    /// of `None` performs the pure verbatim span with no fix-up. This is the bulk
    /// sibling of the per-entry `emit_copy_collection_entries`, used where the
    /// source span is contiguous and its payloads move by one uniform offset
    /// (plan-25-B).
    pub(super) fn emit_bulk_copy_entries_shift(
        &mut self,
        src_entry: &str,
        dst_entry: &str,
        count: &str,
        delta: Option<(&str, bool)>,
        label_prefix: &str,
    ) {
        let saved_dst = self.temporary_vreg();
        let span = self.temporary_vreg();
        let entry_size = self.temporary_vreg();
        let scratch = self.temporary_vreg();
        let value_offset = self.temporary_vreg();
        // span = count * ENTRY. `count` itself is left intact to drive the fix-up
        // loop below (the multiply reads it, the block copy consumes `span`).
        self.emit(abi::move_immediate(
            &entry_size,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&span, count, &entry_size));
        // Remember where the destination span starts before the copy advances
        // `dst_entry` past it; the fix-up walks this saved cursor.
        self.emit(abi::move_register(&saved_dst, dst_entry));
        self.emit_block_copy_advance(
            dst_entry,
            src_entry,
            &span,
            &scratch,
            &format!("{label_prefix}_span"),
        );
        let Some((delta_reg, subtract)) = delta else {
            return;
        };
        let fixup_loop = self.label(&format!("{label_prefix}_fixup_loop"));
        let fixup_done = self.label(&format!("{label_prefix}_fixup_done"));
        self.emit(abi::label(&fixup_loop));
        self.emit(abi::compare_immediate(count, "0"));
        self.emit(abi::branch_eq(&fixup_done));
        self.emit(abi::load_u64(
            &value_offset,
            &saved_dst,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        if subtract {
            self.emit(abi::subtract_registers(
                &value_offset,
                &value_offset,
                delta_reg,
            ));
        } else {
            self.emit(abi::add_registers(&value_offset, &value_offset, delta_reg));
        }
        self.emit(abi::store_u64(
            &value_offset,
            &saved_dst,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::add_immediate(
            &saved_dst,
            &saved_dst,
            COLLECTION_ENTRY_SIZE,
        ));
        self.emit(abi::subtract_immediate(count, count, 1));
        self.emit(abi::branch(&fixup_loop));
        self.emit(abi::label(&fixup_done));
    }

    /// Compact `count` list entries at `entry_base` in place after a single
    /// removed data span `[hole_offset, hole_offset + hole_len)` has been closed
    /// by two verbatim data-block copies (plan-25-B B2 `removeAt`): subtract
    /// `hole_len` from each entry's `valueOffset` iff its payload sat **past** the
    /// hole (`valueOffset > hole_offset`), leaving the offsets of payloads before
    /// the hole unchanged. This tests each entry's own offset, not its list index,
    /// so it is correct whatever order the data region is in — a list built with
    /// `insert`/`prepend`/`set` packs the spliced payload at the data tail, so
    /// `entry[0]` can point past the hole and must shift while a later entry does
    /// not. Only the one shifting field is touched (no payload move); `entry_base`
    /// and `count` are clobbered.
    pub(super) fn emit_offset_compaction_fixup(
        &mut self,
        entry_base: &str,
        count: &str,
        hole_offset: &str,
        hole_len: &str,
        label_prefix: &str,
    ) {
        let value_offset = self.temporary_vreg();
        let loop_label = self.label(&format!("{label_prefix}_loop"));
        let skip_label = self.label(&format!("{label_prefix}_skip"));
        let done_label = self.label(&format!("{label_prefix}_done"));
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate(count, "0"));
        self.emit(abi::branch_eq(&done_label));
        self.emit(abi::load_u64(
            &value_offset,
            entry_base,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::compare_registers(&value_offset, hole_offset));
        // Offsets are small non-negative data positions, so the signed compare is
        // equivalent to unsigned here; `<= hole_offset` means before the hole.
        self.emit(abi::branch_le(&skip_label));
        self.emit(abi::subtract_registers(
            &value_offset,
            &value_offset,
            hole_len,
        ));
        self.emit(abi::store_u64(
            &value_offset,
            entry_base,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::label(&skip_label));
        self.emit(abi::add_immediate(
            entry_base,
            entry_base,
            COLLECTION_ENTRY_SIZE,
        ));
        self.emit(abi::subtract_immediate(count, count, 1));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&done_label));
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn emit_copy_collection_entries(
        &mut self,
        source_entry: &str,
        source_data: &str,
        dest_entry: &str,
        dest_data: &str,
        dest_data_offset: &str,
        count: &str,
        label_prefix: &str,
    ) -> Result<(), String> {
        let scratch22 = self.temporary_vreg();
        let scratch23 = self.temporary_vreg();
        let scratch24 = self.temporary_vreg();
        let scratch25 = self.temporary_vreg();
        let loop_label = self.label(&format!("{label_prefix}_loop"));
        let done = self.label(&format!("{label_prefix}_done"));
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate(count, "0"));
        self.emit(abi::branch_eq(&done));
        self.emit(abi::move_immediate(
            &scratch22,
            "Byte",
            &COLLECTION_ENTRY_FLAG_USED.to_string(),
        ));
        self.emit(abi::store_u8(
            &scratch22,
            dest_entry,
            COLLECTION_ENTRY_OFFSET_FLAGS,
        ));
        self.emit(abi::move_immediate(&scratch22, "Integer", "0"));
        self.emit(abi::store_u64(
            &scratch22,
            dest_entry,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::store_u64(
            &scratch22,
            dest_entry,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::load_u64(
            &scratch22,
            source_entry,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            &scratch23,
            source_entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::store_u64(
            dest_data_offset,
            dest_entry,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::store_u64(
            &scratch23,
            dest_entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers(&scratch24, source_data, &scratch22));
        self.emit(abi::add_registers(&scratch25, dest_data, dest_data_offset));
        self.emit_block_copy_advance(
            &scratch25,
            &scratch24,
            &scratch23,
            &scratch22,
            &format!("{label_prefix}_value"),
        );
        self.emit(abi::load_u64(
            &scratch23,
            dest_entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers(
            dest_data_offset,
            dest_data_offset,
            &scratch23,
        ));
        self.emit(abi::add_immediate(
            source_entry,
            source_entry,
            COLLECTION_ENTRY_SIZE,
        ));
        self.emit(abi::add_immediate(
            dest_entry,
            dest_entry,
            COLLECTION_ENTRY_SIZE,
        ));
        self.emit(abi::subtract_immediate(count, count, 1));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&done));
        Ok(())
    }
}
