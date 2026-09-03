// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::types::ParameterType;

/// plan-121-C: where a **growing** map/set mutation must put its new block when
/// the collection is *inlined* in a record or `STATE` field.
///
/// A growing operation reallocates, and for an inlined field the thing that must
/// grow is the **record** block, not the collection alone — the collection has no
/// allocation of its own to replace. `lower_map_set_in_place` therefore needs to
/// know three things at each of its two grow sites, and nothing else about the
/// container:
///
/// * allocate `fieldOffset + <the map size it already computed>` rather than just
///   the map size;
/// * the new map buffer is `newRecord + fieldOffset`, and the bytes before it are
///   the record's fixed slots, copied across verbatim;
/// * free the **old record block**, not the old map buffer —
///   `emit_free_pre_grow_buffer` on an inlined sub-block would `free()` a pointer
///   into the middle of the record block.
///
/// Passed as `Option`, and `None` is the plain-local case every existing caller
/// wants: the parameter is explicit rather than builder state precisely so that
/// "which container am I growing" is greppable at every call site instead of
/// being ambient (plan-121-C Correction C2).
#[derive(Debug, Clone, Copy)]
pub(crate) struct InlineGrow {
    /// Frame slot holding the owning **record** block pointer. Repointed on a
    /// realloc, which is what makes the grown block visible to `rec.field`.
    pub(crate) block_slot: usize,
    /// Frame slot holding the field's block-relative offset. Invariant across the
    /// realloc — the prefix is copied verbatim — so it is read once by the caller
    /// and handed in.
    pub(crate) field_off_slot: usize,
}

impl CodeBuilder<'_> {
    /// Set `key -> value` in the map whose buffer pointer lives in `map_slot`,
    /// **mutating the buffer in place** (plan-02 §4.3). Linear-scans for the key
    /// (linear scan): on a hit, overwrites the value bytes when the new value fits
    /// the old slot (`newLen <= oldLen`), else appends the new value to the spare
    /// data tail and repoints the entry (old value becomes dead slack, tightened on
    /// copy — amortized O(1) per set even when values grow). On a miss, writes the
    /// key+value into a
    /// spare lookup slot and the spare data tail (the entry packed exactly like
    /// `emit_write_collection_entry` — key then value, each aligned), bumping
    /// `count`/`dataLength`; when the live buffer is full it grows geometrically
    /// (copying entries + data verbatim, capacity-based base) and then writes. The
    /// caller guarantees unique ownership and not an active `FOR EACH` iterable.
    /// plan-121-C, step 1 of the inlined grow: widen the allocation about to
    /// happen so it holds the **record** block, not just the map.
    ///
    /// `lower_map_set_in_place` has already computed the new map size into
    /// `return_register()` (header + entries + data + bucket region). For an
    /// inlined field the block that must be allocated is `fieldOffset + that`, so
    /// this adds the prefix — through the same checked arithmetic the size
    /// computation uses, because `fieldOffset` is runtime-derived and a wrap here
    /// would under-allocate the record.
    pub(crate) fn emit_inline_grow_extend_size(&mut self, inline: &InlineGrow, overflow: &str) {
        let off = self.temporary_vreg();
        self.emit(abi::load_u64(
            &off,
            abi::stack_pointer(),
            inline.field_off_slot,
        ));
        self.emit_checked_size_add(
            abi::return_register(),
            abi::return_register(),
            &off,
            overflow,
        );
    }

    /// plan-121-C, step 2: split the freshly allocated record block into the
    /// record pointer and the map sub-block pointer, and copy the prefix across.
    ///
    /// The allocation returned the new **record** block. Everything before
    /// `fieldOffset` — the record's fixed slots and any earlier inlined sub-blocks
    /// — is copied verbatim, exactly as `lower_inline_list_append_in_place` does,
    /// which is why `fieldOffset` is invariant across the realloc and can be read
    /// once. `new_buf_slot` is then made to hold `newRecord + fieldOffset` so every
    /// line of the map lowering after this point writes through the sub-block
    /// without knowing it moved.
    ///
    /// Returns the slot holding the new record pointer, which the caller publishes
    /// only after the old block has been freed.
    pub(crate) fn emit_inline_grow_split(
        &mut self,
        inline: &InlineGrow,
        new_buf_slot: usize,
    ) -> usize {
        let new_rec_slot = self.allocate_stack_object("inline_grow_newrec", 8);
        let new_rec = self.temporary_vreg();
        let old_rec = self.temporary_vreg();
        let off = self.temporary_vreg();
        let scratch = self.temporary_vreg();

        // The allocator's result is the new record block.
        self.emit(abi::load_u64(&new_rec, abi::stack_pointer(), new_buf_slot));
        self.emit(abi::store_u64(&new_rec, abi::stack_pointer(), new_rec_slot));

        // Copy the fixed prefix [0, fieldOffset) verbatim.
        self.emit(abi::load_u64(
            &off,
            abi::stack_pointer(),
            inline.field_off_slot,
        ));
        self.emit(abi::load_u64(
            &old_rec,
            abi::stack_pointer(),
            inline.block_slot,
        ));
        self.emit_block_copy_advance(&new_rec, &old_rec, &off, &scratch, "inline_grow_prefix");

        // `new_buf_slot` becomes the sub-block address inside the new record.
        // `emit_block_copy_advance` leaves its cursors past the copied range, so
        // re-derive both operands rather than reusing them.
        self.emit(abi::load_u64(&new_rec, abi::stack_pointer(), new_rec_slot));
        self.emit(abi::load_u64(
            &off,
            abi::stack_pointer(),
            inline.field_off_slot,
        ));
        self.emit(abi::add_registers(&new_rec, &new_rec, &off));
        self.emit(abi::store_u64(&new_rec, abi::stack_pointer(), new_buf_slot));
        new_rec_slot
    }

    /// plan-121-C, step 3: free the **old record block**, and publish the new one.
    ///
    /// This is the step `emit_free_pre_grow_buffer` cannot do for an inlined field:
    /// the old map buffer is not its own allocation, it is bytes inside the old
    /// record block, so freeing it would hand `arena_free` a pointer into the
    /// middle of a live allocation. What must be released is the whole old record,
    /// whose size is `fieldOffset + <the old map block's size>` — the same shape
    /// `lower_inline_list_append_in_place` frees, and the reason
    /// `record_collection_last_inlined` insists the field is *last*: nothing
    /// follows the sub-block, so the record ends where it ends.
    pub(crate) fn emit_inline_grow_free_old(
        &mut self,
        inline: &InlineGrow,
        map_slot: usize,
        map_type: &ParameterType,
        new_rec_slot: usize,
    ) -> Result<(), String> {
        let old_rec = self.temporary_vreg();
        let old_sub = self.temporary_vreg();
        let size = self.temporary_vreg();
        let scratch = self.temporary_vreg();
        let off = self.temporary_vreg();

        // Size the old map sub-block where it still lies, then add the prefix.
        self.emit(abi::load_u64(&old_sub, abi::stack_pointer(), map_slot));
        self.emit_flat_block_size(map_type, &old_sub, &size, &scratch)?;
        self.emit(abi::load_u64(
            &off,
            abi::stack_pointer(),
            inline.field_off_slot,
        ));
        self.emit(abi::add_registers(&size, &size, &off));

        self.emit(abi::load_u64(
            &old_rec,
            abi::stack_pointer(),
            inline.block_slot,
        ));
        self.emit(abi::move_register(abi::c_arg(0), &old_rec));
        self.emit(abi::move_register(abi::c_arg(1), &size));
        self.emit_arena_free_call();

        // Publish the grown record. Only now: everything above still had to read
        // the old block.
        let new_rec = self.temporary_vreg();
        self.emit(abi::load_u64(&new_rec, abi::stack_pointer(), new_rec_slot));
        self.emit(abi::store_u64(
            &new_rec,
            abi::stack_pointer(),
            inline.block_slot,
        ));
        Ok(())
    }

    pub(crate) fn lower_map_set_in_place(
        &mut self,
        map_slot: usize,
        key_slot: usize,
        value_slot: usize,
        map_type: &ParameterType,
        key_type: &ParameterType,
        value_type: &ParameterType,
        inline: Option<InlineGrow>,
    ) -> Result<ValueResult, String> {
        let scratch20 = self.temporary_vreg();
        let scratch21 = self.temporary_vreg();
        let scratch22 = self.temporary_vreg();
        let scratch23 = self.temporary_vreg();
        let scratch24 = self.temporary_vreg();
        let scratch25 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch16 = self.temporary_vreg();
        let scratch8 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch15 = self.temporary_vreg();
        let scratch17 = self.temporary_vreg();
        let layout = CollectionTypeLayout::from_type(map_type)
            .ok_or_else(|| format!("native code collection type '{map_type}' is not supported"))?;
        for register in [
            scratch20, scratch21, scratch22, scratch23, scratch24, scratch25,
        ] {
            self.mark_register_used(&register.render());
        }
        let key_align = self.collection_payload_alignment(key_type);
        let value_align = self.collection_payload_alignment(value_type);
        let key_payload = PayloadSlot {
            slot: key_slot,
            type_: key_type.clone(),
        };
        let value_payload = PayloadSlot {
            slot: value_slot,
            type_: value_type.clone(),
        };
        let key_len_slot = self.emit_payload_length_to_stack(&key_payload, "mapset_klen")?;
        let val_len_slot = self.emit_payload_length_to_stack(&value_payload, "mapset_vlen")?;
        let found_entry_slot = self.allocate_stack_object("mapset_found_entry", 8);
        let found_index_slot = self.allocate_stack_object("mapset_found_index", 8);
        let new_data_len_slot = self.allocate_stack_object("mapset_newdlen", 8);
        let new_cap_slot = self.allocate_stack_object("mapset_newcap", 8);
        let new_dcap_slot = self.allocate_stack_object("mapset_newdcap", 8);
        let new_buf_slot = self.allocate_stack_object("mapset_newbuf", 8);
        let data_offset_slot = self.allocate_stack_object("mapset_doff", 8);
        let voff_slot = self.allocate_stack_object("mapset_voff", 8);
        let entry_addr_slot = self.allocate_stack_object("mapset_entry_addr", 8);

        let loop_label = self.label("mapset_loop");
        let next = self.label("mapset_next");
        let found = self.label("mapset_found");
        let not_found = self.label("mapset_not_found");
        let value_grow = self.label("mapset_value_grow");
        let vgrow = self.label("mapset_vgrow");
        let vwrite = self.label("mapset_vwrite");
        let valloc_ok = self.label("mapset_valloc_ok");
        let vdcap_keep = self.label("mapset_vdcap_keep");
        let grow = self.label("mapset_grow");
        let write = self.label("mapset_write");
        let alloc_ok = self.label("mapset_alloc_ok");
        let dcap_keep = self.label("mapset_dcap_keep");
        let done = self.label("mapset_done");

        // --- Locate the key: O(1) hash probe for eligible key types, else the
        // linear scan. Both store the found entry address + index and branch to
        // `found_handle`, or branch to `not_found`. The probe also lazily builds the
        // bucket index, so a build-via-`set` loop stays O(n). ---
        let found_handle = self.label("mapset_found_handle");
        if Self::map_key_probe_eligible(key_type) {
            let entry_slot = self.emit_map_probe(map_slot, key_slot, key_type, &not_found)?;
            // emit_map_probe already stored the entry address; record the index too
            // (x0 held it before the entry-address math, so recompute it from the
            // entry address: index = (entry - map - HEADER) / ENTRY).
            self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), entry_slot));
            self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), map_slot));
            self.emit(abi::subtract_registers(&scratch9, &scratch9, &scratch10));
            self.emit(abi::subtract_immediate(
                &scratch9,
                &scratch9,
                COLLECTION_HEADER_SIZE,
            ));
            self.emit(abi::move_immediate(
                &scratch16,
                "Integer",
                &COLLECTION_ENTRY_SIZE.to_string(),
            ));
            self.emit(abi::unsigned_divide_registers(
                &scratch9, &scratch9, &scratch16,
            ));
            self.emit(abi::store_u64(
                &scratch9,
                abi::stack_pointer(),
                found_index_slot,
            ));
            self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), entry_slot));
            self.emit(abi::store_u64(
                &scratch9,
                abi::stack_pointer(),
                found_entry_slot,
            ));
            self.emit(abi::branch(&found_handle));
        } else {
            self.reset_temporary_registers();
            let collection = self.allocate_register();
            let key = self.allocate_register();
            let count = self.allocate_register();
            let index = self.allocate_register();
            let entry = self.allocate_register();
            let key_offset = self.allocate_register();
            let key_length = self.allocate_register();
            self.emit(abi::load_u64(&collection, abi::stack_pointer(), map_slot));
            self.emit(abi::load_u64(&key, abi::stack_pointer(), key_slot));
            self.emit(abi::load_u64(&count, &collection, COLLECTION_OFFSET_COUNT));
            self.emit(abi::move_immediate(&index, "Integer", "0"));
            self.emit(abi::add_immediate(
                &entry,
                &collection,
                COLLECTION_HEADER_SIZE,
            ));
            self.emit(abi::label(&loop_label));
            self.emit(abi::compare_registers(&index, &count));
            self.emit(abi::branch_ge(&not_found));
            self.emit(abi::load_u64(
                key_offset,
                &entry,
                COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
            ));
            self.emit(abi::load_u64(
                key_length,
                &entry,
                COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
            ));
            self.emit_collection_payload_match_branch(
                key_type,
                &ParameterType::named(""),
                &collection,
                key_offset,
                key_length,
                key,
                &found,
                &next,
            )?;
            self.emit(abi::label(&next));
            self.emit(abi::add_immediate(&entry, &entry, COLLECTION_ENTRY_SIZE));
            self.emit(abi::add_immediate(&index, &index, 1));
            self.emit(abi::branch(&loop_label));
            self.emit(abi::label(&found));
            self.emit(abi::store_u64(
                &entry,
                abi::stack_pointer(),
                found_entry_slot,
            ));
            self.emit(abi::store_u64(
                &index,
                abi::stack_pointer(),
                found_index_slot,
            ));
            self.emit(abi::branch(&found_handle));
        }

        // --- Found handling (shared): overwrite the value when it fits, else
        // append-and-repoint. Slot-based so it serves both the probe and scan. ---
        self.emit(abi::label(&found_handle));
        self.emit(abi::load_u64(
            &scratch8,
            abi::stack_pointer(),
            found_entry_slot,
        ));
        self.emit(abi::load_u64(
            &scratch9,
            &scratch8,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        )); // oldValLen
        self.emit(abi::load_u64(
            &scratch14,
            abi::stack_pointer(),
            val_len_slot,
        )); // newValLen
        self.emit(abi::compare_registers(&scratch14, &scratch9));
        self.emit(abi::branch_hi(&value_grow)); // newLen > oldLen → append + repoint
        self.emit(abi::load_u64(
            &scratch8,
            abi::stack_pointer(),
            found_entry_slot,
        ));
        self.emit(abi::load_u64(
            &scratch13,
            &scratch8,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::store_u64(&scratch13, abi::stack_pointer(), voff_slot));
        self.emit_copy_payload_to_collection(
            map_slot,
            val_len_slot,
            &value_payload,
            voff_slot,
            &ParameterType::named(""),
        )?;
        self.emit(abi::load_u64(
            &scratch8,
            abi::stack_pointer(),
            found_entry_slot,
        ));
        self.emit(abi::load_u64(
            &scratch14,
            abi::stack_pointer(),
            val_len_slot,
        ));
        self.emit(abi::store_u64(
            &scratch14,
            &scratch8,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::branch(&done));

        // --- Value grew: append the new value to the spare data tail and repoint
        // the entry's valueOffset/valueLength; the old value bytes become dead
        // slack (tightened away on copy, which copies dataLength verbatim). The
        // key, the lookup entry, and `count` are untouched — only the data region
        // grows, geometrically, when there is no headroom. This keeps a map whose
        // values grow (e.g. groupBy's per-key bucket list) amortized O(1) per set
        // instead of the O(map size) remove+concat rebuild. ---
        self.emit(abi::label(&value_grow));
        // newValOffset = align(dataLength, value_align); newDataLength += valLen.
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(
            &scratch9,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::store_u64(
            &scratch9,
            abi::stack_pointer(),
            new_data_len_slot,
        ));
        self.emit_align_offset_slot(new_data_len_slot, value_align);
        self.emit(abi::load_u64(
            &scratch8,
            abi::stack_pointer(),
            new_data_len_slot,
        ));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), val_len_slot));
        self.emit(abi::add_registers(&scratch8, &scratch8, &scratch9));
        self.emit(abi::store_u64(
            &scratch8,
            abi::stack_pointer(),
            new_data_len_slot,
        ));
        // Room: newDataLength <= dataCapacity?
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch8,
            COLLECTION_OFFSET_DATA_CAPACITY,
        ));
        self.emit(abi::load_u64(
            &scratch12,
            abi::stack_pointer(),
            new_data_len_slot,
        ));
        self.emit(abi::compare_registers(&scratch12, &scratch11));
        self.emit(abi::branch_hi(&vgrow));
        self.emit(abi::branch(&vwrite));

        // Grow the data region only (capacity unchanged); copy entries + data
        // verbatim against the capacity-based base, then repoint.
        self.emit(abi::label(&vgrow));
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(
            &scratch10,
            &scratch8,
            COLLECTION_OFFSET_DATA_CAPACITY,
        ));
        self.emit_geometric_step(
            &scratch10,
            &scratch14,
            &scratch15,
            COLLECTION_GROW_DATA_INIT,
            COLLECTION_GROW_DATA_TAPER,
            "mapset_vgrow_dcap",
        );
        self.emit(abi::load_u64(
            &scratch11,
            abi::stack_pointer(),
            new_data_len_slot,
        ));
        self.emit(abi::compare_registers(&scratch14, &scratch11));
        self.emit(abi::branch_hi(&vdcap_keep));
        self.emit(abi::branch_eq(&vdcap_keep));
        self.emit(abi::move_register(&scratch14, &scratch11));
        self.emit(abi::label(&vdcap_keep));
        self.emit(abi::store_u64(
            &scratch14,
            abi::stack_pointer(),
            new_dcap_slot,
        ));
        // alloc = HEADER + capacity * ENTRY + newDataCapacity (capacity unchanged).
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(
            &scratch14,
            &scratch8,
            COLLECTION_OFFSET_CAPACITY,
        ));
        let size_overflow = self.label("map_vgrow_size_overflow");
        self.emit(abi::move_immediate(
            &scratch16,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        // Checked collection-size arithmetic (bug-147.7): capacity/dataCapacity are
        // runtime-derived, so guard count*ENTRY + HEADER + dataCap against overflow.
        self.emit_checked_size_multiply(&scratch17, &scratch14, &scratch16, &size_overflow);
        self.emit_checked_size_add_immediate(
            abi::return_register(),
            &scratch17,
            COLLECTION_HEADER_SIZE,
            &size_overflow,
        );
        self.emit(abi::load_u64(
            &scratch15,
            abi::stack_pointer(),
            new_dcap_slot,
        ));
        self.emit_checked_size_add(
            abi::return_register(),
            abi::return_register(),
            &scratch15,
            &size_overflow,
        );
        // Reserve the map hash bucket region (x14 = capacity, unchanged on vgrow).
        self.emit_reserve_map_buckets(true, &scratch14, abi::return_register(), &scratch16);
        // plan-121-C: an inlined field grows the RECORD, so widen the request by
        // the record prefix before asking the allocator for it.
        if let Some(g) = inline {
            self.emit_inline_grow_extend_size(&g, &size_overflow);
        }
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&valloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&size_overflow));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&valloc_ok));
        self.emit(abi::store_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            new_buf_slot,
        ));
        // plan-121-C: for an inlined field the allocation IS the record; split it
        // so every line below writes through the sub-block unchanged.
        let vgrow_new_rec = inline.map(|g| self.emit_inline_grow_split(&g, new_buf_slot));
        // Header: old count / old dataLength, same capacity, new data capacity.
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(
            &scratch14,
            &scratch8,
            COLLECTION_OFFSET_CAPACITY,
        ));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::load_u64(
            &scratch15,
            abi::stack_pointer(),
            new_dcap_slot,
        ));
        let nb = self.temporary_vreg();
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), new_buf_slot));
        self.emit_write_collection_header_full(
            &layout, &nb, &scratch9, &scratch14, &scratch11, &scratch15,
        );
        // Copy the data region verbatim (dataLength bytes), capacity-based base.
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), new_buf_slot));
        self.emit_collection_data_pointer_for(&scratch17, &nb, &ParameterType::named(""));
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
        self.emit_collection_data_pointer_for(&scratch20, &scratch8, &ParameterType::named(""));
        self.emit(abi::load_u64(
            &scratch14,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit_block_copy_advance(
            &scratch17,
            &scratch20,
            &scratch14,
            &scratch22,
            "mapset_vgrow_data",
        );
        // Copy the lookup entries verbatim (count * ENTRY bytes).
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), new_buf_slot));
        self.emit(abi::add_immediate(&scratch17, &nb, COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
        self.emit(abi::add_immediate(
            &scratch20,
            &scratch8,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::move_immediate(
            &scratch16,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch21, &scratch9, &scratch16));
        self.emit_block_copy_advance(
            &scratch17,
            &scratch20,
            &scratch21,
            &scratch22,
            "mapset_vgrow_entries",
        );
        // Free the abandoned pre-grow buffer (still in `map_slot`, sized with its
        // bucket region) before installing the grown one — otherwise a value-growing
        // map-set in a loop leaks the old buffer on every grow (bug-47).
        //
        // plan-121-C: an inlined sub-block is not its own allocation, so what is
        // released is the whole old RECORD block instead.
        match (inline, vgrow_new_rec) {
            (Some(g), Some(new_rec_slot)) => {
                self.emit_inline_grow_free_old(&g, map_slot, map_type, new_rec_slot)?
            }
            _ => self.emit_free_pre_grow_buffer(map_slot, map_type)?,
        }
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), new_buf_slot));
        self.emit(abi::store_u64(&nb, abi::stack_pointer(), map_slot));
        self.emit(abi::branch(&vwrite));

        // Write the new value at the aligned data tail; repoint the found entry.
        self.emit(abi::label(&vwrite));
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(
            &scratch9,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::store_u64(
            &scratch9,
            abi::stack_pointer(),
            data_offset_slot,
        ));
        self.emit_align_offset_slot(data_offset_slot, value_align);
        // entryAddr = map + HEADER + foundIndex * ENTRY (the buffer may have moved).
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(
            &scratch9,
            abi::stack_pointer(),
            found_index_slot,
        ));
        self.emit(abi::move_immediate(
            &scratch16,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch13, &scratch9, &scratch16));
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch8,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(&scratch12, &scratch12, &scratch13));
        self.emit(abi::store_u64(
            &scratch12,
            abi::stack_pointer(),
            entry_addr_slot,
        ));
        // valueOffset = aligned data offset, valueLength = newValLen.
        self.emit(abi::load_u64(
            &scratch13,
            abi::stack_pointer(),
            data_offset_slot,
        ));
        self.emit(abi::store_u64(
            &scratch13,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            &scratch13,
            abi::stack_pointer(),
            val_len_slot,
        ));
        self.emit(abi::store_u64(
            &scratch13,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit_copy_payload_to_collection(
            map_slot,
            val_len_slot,
            &value_payload,
            data_offset_slot,
            &ParameterType::named(""),
        )?;
        // dataLength = final data offset (includes the alignment pad + new value).
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(
            &scratch9,
            abi::stack_pointer(),
            data_offset_slot,
        ));
        self.emit(abi::store_u64(
            &scratch9,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::branch(&done));

        // --- Not found: compute the would-be new dataLength after the insert. ---
        self.emit(abi::label(&not_found));
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(
            &scratch9,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::store_u64(
            &scratch9,
            abi::stack_pointer(),
            new_data_len_slot,
        ));
        self.emit_align_offset_slot(new_data_len_slot, key_align);
        self.emit(abi::load_u64(
            &scratch8,
            abi::stack_pointer(),
            new_data_len_slot,
        ));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), key_len_slot));
        self.emit(abi::add_registers(&scratch8, &scratch8, &scratch9));
        self.emit(abi::store_u64(
            &scratch8,
            abi::stack_pointer(),
            new_data_len_slot,
        ));
        self.emit_align_offset_slot(new_data_len_slot, value_align);
        self.emit(abi::load_u64(
            &scratch8,
            abi::stack_pointer(),
            new_data_len_slot,
        ));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), val_len_slot));
        self.emit(abi::add_registers(&scratch8, &scratch8, &scratch9));
        self.emit(abi::store_u64(
            &scratch8,
            abi::stack_pointer(),
            new_data_len_slot,
        ));
        // Room check: count < capacity AND newDataLength <= dataCapacity.
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(
            &scratch10,
            &scratch8,
            COLLECTION_OFFSET_CAPACITY,
        ));
        self.emit(abi::compare_registers(&scratch9, &scratch10));
        self.emit(abi::branch_ge(&grow));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch8,
            COLLECTION_OFFSET_DATA_CAPACITY,
        ));
        self.emit(abi::load_u64(
            &scratch12,
            abi::stack_pointer(),
            new_data_len_slot,
        ));
        self.emit(abi::compare_registers(&scratch12, &scratch11));
        self.emit(abi::branch_hi(&grow));
        self.emit(abi::branch(&write));

        // --- Grow: geometric capacity + data, copy entries/data verbatim. ---
        self.emit(abi::label(&grow));
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(
            &scratch10,
            &scratch8,
            COLLECTION_OFFSET_CAPACITY,
        ));
        self.emit_geometric_step(
            &scratch10,
            &scratch14,
            &scratch15,
            COLLECTION_GROW_LOOKUP_INIT,
            COLLECTION_GROW_LOOKUP_TAPER,
            "mapset_grow_cap",
        );
        self.emit(abi::store_u64(
            &scratch14,
            abi::stack_pointer(),
            new_cap_slot,
        ));
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(
            &scratch10,
            &scratch8,
            COLLECTION_OFFSET_DATA_CAPACITY,
        ));
        self.emit_geometric_step(
            &scratch10,
            &scratch14,
            &scratch15,
            COLLECTION_GROW_DATA_INIT,
            COLLECTION_GROW_DATA_TAPER,
            "mapset_grow_dcap",
        );
        // newDataCapacity = max(step(dataCapacity), newDataLength).
        self.emit(abi::load_u64(
            &scratch11,
            abi::stack_pointer(),
            new_data_len_slot,
        ));
        self.emit(abi::compare_registers(&scratch14, &scratch11));
        self.emit(abi::branch_hi(&dcap_keep));
        self.emit(abi::branch_eq(&dcap_keep));
        self.emit(abi::move_register(&scratch14, &scratch11));
        self.emit(abi::label(&dcap_keep));
        self.emit(abi::store_u64(
            &scratch14,
            abi::stack_pointer(),
            new_dcap_slot,
        ));
        // alloc = HEADER + newCapacity * ENTRY + newDataCapacity.
        self.emit(abi::load_u64(
            &scratch14,
            abi::stack_pointer(),
            new_cap_slot,
        ));
        let size_overflow = self.label("map_grow_size_overflow");
        self.emit(abi::move_immediate(
            &scratch16,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        // Checked collection-size arithmetic (bug-147.7): capacity/dataCapacity are
        // runtime-derived, so guard count*ENTRY + HEADER + dataCap against overflow.
        self.emit_checked_size_multiply(&scratch17, &scratch14, &scratch16, &size_overflow);
        self.emit_checked_size_add_immediate(
            abi::return_register(),
            &scratch17,
            COLLECTION_HEADER_SIZE,
            &size_overflow,
        );
        self.emit(abi::load_u64(
            &scratch15,
            abi::stack_pointer(),
            new_dcap_slot,
        ));
        self.emit_checked_size_add(
            abi::return_register(),
            abi::return_register(),
            &scratch15,
            &size_overflow,
        );
        // Reserve the map hash bucket region (x14 = new capacity).
        self.emit_reserve_map_buckets(true, &scratch14, abi::return_register(), &scratch16);
        // plan-121-C: an inlined field grows the RECORD, so widen the request by
        // the record prefix before asking the allocator for it.
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
        // plan-121-C: as at the value-grow site — the allocation IS the record.
        let grow_new_rec = inline.map(|g| self.emit_inline_grow_split(&g, new_buf_slot));
        // Header: old count / old dataLength, new capacity / data capacity.
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::load_u64(
            &scratch14,
            abi::stack_pointer(),
            new_cap_slot,
        ));
        self.emit(abi::load_u64(
            &scratch15,
            abi::stack_pointer(),
            new_dcap_slot,
        ));
        let nb = self.temporary_vreg();
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), new_buf_slot));
        self.emit_write_collection_header_full(
            &layout, &nb, &scratch9, &scratch14, &scratch11, &scratch15,
        );
        // Copy the data region verbatim (dataLength bytes), capacity-based base.
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), new_buf_slot));
        self.emit_collection_data_pointer_for(&scratch17, &nb, &ParameterType::named(""));
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
        self.emit_collection_data_pointer_for(&scratch20, &scratch8, &ParameterType::named(""));
        self.emit(abi::load_u64(
            &scratch14,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit_block_copy_advance(
            &scratch17,
            &scratch20,
            &scratch14,
            &scratch22,
            "mapset_grow_data",
        );
        // Copy the live lookup entries verbatim (count * ENTRY bytes).
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), new_buf_slot));
        self.emit(abi::add_immediate(&scratch17, &nb, COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
        self.emit(abi::add_immediate(
            &scratch20,
            &scratch8,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::move_immediate(
            &scratch16,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch21, &scratch9, &scratch16));
        self.emit_block_copy_advance(
            &scratch17,
            &scratch20,
            &scratch21,
            &scratch22,
            "mapset_grow_entries",
        );
        // Free the abandoned pre-grow buffer (still in `map_slot`, sized with its
        // bucket region) before installing the grown one — otherwise a capacity-
        // growing map-set in a loop leaks the old buffer on every grow (bug-47).
        //
        // plan-121-C: as at the value-grow site — release the old RECORD block.
        match (inline, grow_new_rec) {
            (Some(g), Some(new_rec_slot)) => {
                self.emit_inline_grow_free_old(&g, map_slot, map_type, new_rec_slot)?
            }
            _ => self.emit_free_pre_grow_buffer(map_slot, map_type)?,
        }
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), new_buf_slot));
        self.emit(abi::store_u64(&nb, abi::stack_pointer(), map_slot));
        self.emit(abi::branch(&write));

        // --- Write the new entry into slot[count], key+value aligned in data. ---
        self.emit(abi::label(&write));
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(
            &scratch9,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::store_u64(
            &scratch9,
            abi::stack_pointer(),
            data_offset_slot,
        ));
        // entryAddr = map + HEADER + count * ENTRY.
        self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::move_immediate(
            &scratch16,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch13, &scratch9, &scratch16));
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch8,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(&scratch12, &scratch12, &scratch13));
        self.emit(abi::store_u64(
            &scratch12,
            abi::stack_pointer(),
            entry_addr_slot,
        ));
        self.emit(abi::move_immediate(
            &scratch13,
            "Byte",
            &COLLECTION_ENTRY_FLAG_USED.to_string(),
        ));
        self.emit(abi::store_u8(
            &scratch13,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_FLAGS,
        ));
        // Key: align, record keyOffset/keyLength, copy bytes.
        self.emit_align_offset_slot(data_offset_slot, key_align);
        self.emit(abi::load_u64(
            &scratch12,
            abi::stack_pointer(),
            entry_addr_slot,
        ));
        self.emit(abi::load_u64(
            &scratch13,
            abi::stack_pointer(),
            data_offset_slot,
        ));
        self.emit(abi::store_u64(
            &scratch13,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::load_u64(
            &scratch13,
            abi::stack_pointer(),
            key_len_slot,
        ));
        self.emit(abi::store_u64(
            &scratch13,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit_copy_payload_to_collection(
            map_slot,
            key_len_slot,
            &key_payload,
            data_offset_slot,
            &ParameterType::named(""),
        )?;
        // Value: align, record valueOffset/valueLength, copy bytes.
        self.emit_align_offset_slot(data_offset_slot, value_align);
        self.emit(abi::load_u64(
            &scratch12,
            abi::stack_pointer(),
            entry_addr_slot,
        ));
        self.emit(abi::load_u64(
            &scratch13,
            abi::stack_pointer(),
            data_offset_slot,
        ));
        self.emit(abi::store_u64(
            &scratch13,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            &scratch13,
            abi::stack_pointer(),
            val_len_slot,
        ));
        self.emit(abi::store_u64(
            &scratch13,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit_copy_payload_to_collection(
            map_slot,
            val_len_slot,
            &value_payload,
            data_offset_slot,
            &ParameterType::named(""),
        )?;
        // Header: count++, dataLength = final data offset.
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::add_immediate(&scratch9, &scratch9, 1));
        self.emit(abi::store_u64(
            &scratch9,
            &scratch8,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::load_u64(
            &scratch9,
            abi::stack_pointer(),
            data_offset_slot,
        ));
        self.emit(abi::store_u64(
            &scratch9,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        // Keep the hash index current: if the buckets are already built (a prior
        // probe), insert the new entry incrementally so a build-via-`set` loop stays
        // O(n). The grow path reset the ready flag (the bucket region moved), so it
        // falls through here and is rebuilt lazily on the next probe. The 2*capacity
        // load factor guarantees a free slot for a spare-slot insert.
        let skip_put = self.label("mapset_skip_put");
        self.emit(abi::load_u8(
            &scratch9,
            &scratch8,
            COLLECTION_OFFSET_BUCKETS_READY,
        ));
        self.emit(abi::compare_immediate(&scratch9, "0"));
        self.emit(abi::branch_eq(&skip_put));
        self.emit(abi::load_u64(abi::c_arg(0), abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(
            abi::c_arg(1),
            abi::c_arg(0),
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::subtract_immediate(abi::c_arg(1), abi::c_arg(1), 1)); // new entry index
        self.emit(abi::branch_link(MAP_BUCKET_PUT_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: MAP_BUCKET_PUT_SYMBOL.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::label(&skip_put));

        self.emit(abi::label(&done));
        let result = self.allocate_register();
        self.emit(abi::load_u64(&result, abi::stack_pointer(), map_slot));
        Ok(ValueResult {
            origin: None,
            type_: map_type.clone(),
            location: Operand::from(result.render()),
            text: format!("map set in place {map_type}"),
        })
    }

    /// keep their absolute KEY/VALUE_OFFSETs, so the data region is not moved and no
    /// offset fixup is needed — the same dead-slack model `set` uses when it
    /// overwrites a value). Insertion order is preserved (a shift, not a swap). The
    /// bucket index cannot be repaired incrementally (open addressing over absolute
    /// entry indices, no DELETED sentinel), so it is invalidated and rebuilt lazily
    /// on the next probe — unavoidable given the index (plan-86-D1 scout).
    pub(crate) fn lower_map_remove_key_in_place(
        &mut self,
        map_slot: usize,
        key_slot: usize,
        map_type: &ParameterType,
        key_type: &ParameterType,
    ) -> Result<ValueResult, String> {
        let s8 = self.temporary_vreg();
        let s9 = self.temporary_vreg();
        let s10 = self.temporary_vreg();
        let s11 = self.temporary_vreg();
        let s12 = self.temporary_vreg();
        let s13 = self.temporary_vreg();
        let s16 = self.temporary_vreg();
        let found_slot = self.allocate_stack_object("mrk_found_idx", 8);
        let flag_slot = self.allocate_stack_object("mrk_found_flag", 8);
        let scan_loop = self.label("mrk_scan_loop");
        let scan_match = self.label("mrk_scan_match");
        let scan_next = self.label("mrk_scan_next");
        let scan_done = self.label("mrk_scan_done");
        let shift_loop = self.label("mrk_shift_loop");
        let shift_done = self.label("mrk_shift_done");
        let done = self.label("mrk_done");

        // found_flag = 0
        self.emit(abi::move_immediate(&s8, "Integer", "0"));
        self.emit(abi::store_u64(&s8, abi::stack_pointer(), flag_slot));
        // scan entries 0..count for the matching key.
        self.emit(abi::load_u64(&s8, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(&s9, abi::stack_pointer(), key_slot));
        self.emit(abi::load_u64(&s10, &s8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::move_immediate(&s11, "Integer", "0"));
        self.emit(abi::add_immediate(&s12, &s8, COLLECTION_HEADER_SIZE));
        self.emit(abi::label(&scan_loop));
        self.emit(abi::compare_registers(&s11, &s10));
        self.emit(abi::branch_ge(&scan_done));
        self.emit(abi::load_u64(
            &s13,
            &s12,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::load_u64(
            &s16,
            &s12,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        // arg7 = on-MATCH, arg8 = on-NO-MATCH (per `lower_map_remove_key`).
        self.emit_collection_payload_matches_value_branch(
            key_type,
            &ParameterType::named(""),
            &s8,
            &s13,
            &s16,
            &s9,
            &scan_match,
            &scan_next,
        )?;
        self.emit(abi::label(&scan_match));
        self.emit(abi::store_u64(&s11, abi::stack_pointer(), found_slot));
        self.emit(abi::move_immediate(&s13, "Integer", "1"));
        self.emit(abi::store_u64(&s13, abi::stack_pointer(), flag_slot));
        self.emit(abi::branch(&scan_done)); // keys are unique — first match wins
        self.emit(abi::label(&scan_next));
        self.emit(abi::add_immediate(&s11, &s11, 1));
        self.emit(abi::add_immediate(&s12, &s12, COLLECTION_ENTRY_SIZE));
        self.emit(abi::branch(&scan_loop));
        self.emit(abi::label(&scan_done));
        // If not found, the map is unchanged.
        self.emit(abi::load_u64(&s8, abi::stack_pointer(), flag_slot));
        self.emit(abi::compare_immediate(&s8, "0"));
        self.emit(abi::branch_eq(&done));
        // Shift entries [found+1 .. count) down one 40-byte slot.
        // dst = map + HEADER + found*40 ; src = dst + 40 ; words = (count-1-found)*5.
        self.emit(abi::load_u64(&s8, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(&s10, &s8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(&s11, abi::stack_pointer(), found_slot));
        // s12 = dst = map + HEADER + found*40
        self.emit(abi::move_immediate(
            &s13,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&s16, &s11, &s13));
        self.emit(abi::add_immediate(&s12, &s8, COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers(&s12, &s12, &s16));
        // s16 = words = (count - 1 - found) * 5
        self.emit(abi::subtract_registers(&s16, &s10, &s11));
        self.emit(abi::subtract_immediate(&s16, &s16, 1));
        self.emit(abi::move_immediate(&s13, "Integer", "5"));
        self.emit(abi::multiply_registers(&s16, &s16, &s13));
        // forward word-copy dst[k] = src[k] where src = dst + 40 (dst < src → safe).
        self.emit(abi::move_immediate(&s11, "Integer", "0")); // k
        self.emit(abi::label(&shift_loop));
        self.emit(abi::compare_registers(&s11, &s16));
        self.emit(abi::branch_ge(&shift_done));
        self.emit(abi::shift_left_immediate(&s13, &s11, 3)); // k*8
        self.emit(abi::add_registers(&s9, &s12, &s13)); // &dst[k]
        self.emit(abi::load_u64(&s10, &s9, COLLECTION_ENTRY_SIZE)); // src[k] = dst[k]+40
        self.emit(abi::store_u64(&s10, &s9, 0));
        self.emit(abi::add_immediate(&s11, &s11, 1));
        self.emit(abi::branch(&shift_loop));
        self.emit(abi::label(&shift_done));
        // count -= 1; BUCKETS_READY = 0.
        self.emit(abi::load_u64(&s8, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(&s10, &s8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::subtract_immediate(&s10, &s10, 1));
        self.emit(abi::store_u64(&s10, &s8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::move_immediate(&s13, "Byte", "0"));
        self.emit(abi::store_u8(&s13, &s8, COLLECTION_OFFSET_BUCKETS_READY));
        self.emit(abi::label(&done));
        let result = self.allocate_register();
        self.emit(abi::load_u64(&result, abi::stack_pointer(), map_slot));
        Ok(ValueResult {
            origin: None,
            type_: map_type.clone(),
            location: Operand::from(result.render()),
            text: format!("removeKey_in_place({map_type}, {key_type})"),
        })
    }
}
