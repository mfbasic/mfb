use super::*;

impl CodeBuilder<'_> {
    pub(super) fn lower_collection_append(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let scratch8 = self.temporary_vreg();
        let list = self.lower_value(&args[0])?;
        let Some(element_type) = list_element_type(&list.type_) else {
            return Err(format!(
                "native collection append does not accept {}",
                list.type_
            ));
        };
        let list_slot = self.allocate_stack_object("append_list", 8);
        self.emit(abi::store_u64(
            &list.location,
            abi::stack_pointer(),
            list_slot,
        ));
        let item = self.lower_value(&args[1])?;
        // Observation boundary: a `Float` appended element must be finite
        // (plan-17).
        self.observe_float(&args[1], &item)?;
        // A `d`-native float item is materialized into a GPR before being
        // spilled into the collection payload (plan-01 float-dnative).
        let item = self.materialize_value(item)?;
        let (insert_slot, materialized) =
            self.collection_argument_as_list_slot(&list.type_, &element_type, item)?;
        let index_slot = self.allocate_stack_object("append_index", 8);
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), list_slot));
        self.emit(abi::load_u64(&scratch8, &scratch8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), index_slot));
        let result = self.lower_list_insert_collection(
            list_slot,
            index_slot,
            insert_slot,
            &list.type_,
            &element_type,
        )?;
        if materialized {
            return self.free_intermediate_collection(insert_slot, &list.type_, result);
        }
        Ok(result)
    }

    pub(super) fn lower_collection_prepend(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let scratch8 = self.temporary_vreg();
        let list = self.lower_value(&args[0])?;
        let Some(element_type) = list_element_type(&list.type_) else {
            return Err(format!(
                "native collection prepend does not accept {}",
                list.type_
            ));
        };
        let list_slot = self.allocate_stack_object("prepend_list", 8);
        self.emit(abi::store_u64(
            &list.location,
            abi::stack_pointer(),
            list_slot,
        ));
        let item = self.lower_value(&args[1])?;
        // Observation boundary: a `Float` prepended element must be finite
        // (plan-17).
        self.observe_float(&args[1], &item)?;
        if item.type_ == list.type_ {
            return Err("native collection prepend expects a single item, not a list".to_string());
        }
        // Materialize a `d`-native float before the payload spill (plan-01).
        let item = self.materialize_value(item)?;
        let (insert_slot, materialized) =
            self.collection_argument_as_list_slot(&list.type_, &element_type, item)?;
        let index_slot = self.allocate_stack_object("prepend_index", 8);
        self.emit(abi::move_immediate(&scratch8, "Integer", "0"));
        self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), index_slot));
        let result = self.lower_list_insert_collection(
            list_slot,
            index_slot,
            insert_slot,
            &list.type_,
            &element_type,
        )?;
        if materialized {
            return self.free_intermediate_collection(insert_slot, &list.type_, result);
        }
        Ok(result)
    }

    pub(super) fn lower_collection_insert(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let list = self.lower_value(&args[0])?;
        let Some(element_type) = list_element_type(&list.type_) else {
            return Err(format!(
                "native collection insert does not accept {}",
                list.type_
            ));
        };
        let list_slot = self.allocate_stack_object("insert_list", 8);
        self.emit(abi::store_u64(
            &list.location,
            abi::stack_pointer(),
            list_slot,
        ));
        let index = self.lower_value(&args[1])?;
        if index.type_ != "Integer" {
            return Err(format!(
                "native collection insert index must be Integer, got {}",
                index.type_
            ));
        }
        let index_slot = self.allocate_stack_object("insert_index", 8);
        self.emit(abi::store_u64(
            &index.location,
            abi::stack_pointer(),
            index_slot,
        ));
        let item = self.lower_value(&args[2])?;
        // Observation boundary: a `Float` inserted element must be finite
        // (plan-17).
        self.observe_float(&args[2], &item)?;
        if item.type_ == list.type_ {
            return Err("native collection insert expects a single item, not a list".to_string());
        }
        // Materialize a `d`-native float before the payload spill (plan-01).
        let item = self.materialize_value(item)?;
        let (insert_slot, materialized) =
            self.collection_argument_as_list_slot(&list.type_, &element_type, item)?;
        let result = self.lower_list_insert_collection(
            list_slot,
            index_slot,
            insert_slot,
            &list.type_,
            &element_type,
        )?;
        if materialized {
            return self.free_intermediate_collection(insert_slot, &list.type_, result);
        }
        Ok(result)
    }

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
    fn emit_free_pre_grow_buffer(&mut self, slot: usize, type_: &str) -> Result<(), String> {
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

    pub(super) fn lower_collection_remove_at(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let list = self.lower_value(&args[0])?;
        let Some(element_type) = list_element_type(&list.type_) else {
            return Err(format!(
                "native collection removeAt does not accept {}",
                list.type_
            ));
        };
        let list_slot = self.allocate_stack_object("remove_at_list", 8);
        self.emit(abi::store_u64(
            &list.location,
            abi::stack_pointer(),
            list_slot,
        ));
        let index = self.lower_value(&args[1])?;
        if index.type_ != "Integer" {
            return Err(format!(
                "native collection removeAt index must be Integer, got {}",
                index.type_
            ));
        }
        let index_slot = self.allocate_stack_object("remove_at_index", 8);
        self.emit(abi::store_u64(
            &index.location,
            abi::stack_pointer(),
            index_slot,
        ));
        self.lower_list_remove_at(list_slot, index_slot, &list.type_, &element_type)
    }

    pub(super) fn lower_collection_set(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let collection = self.lower_value(&args[0])?;
        if let Some(element_type) = list_element_type(&collection.type_) {
            let list_slot = self.allocate_stack_object("set_list", 8);
            self.emit(abi::store_u64(
                &collection.location,
                abi::stack_pointer(),
                list_slot,
            ));
            let index = self.lower_value(&args[1])?;
            if index.type_ != "Integer" {
                return Err(format!(
                    "native collection set list index must be Integer, got {}",
                    index.type_
                ));
            }
            let index_slot = self.allocate_stack_object("set_index", 8);
            self.emit(abi::store_u64(
                &index.location,
                abi::stack_pointer(),
                index_slot,
            ));
            let item = self.lower_value(&args[2])?;
            // Observation boundary: a `Float` replacement element must be finite
            // (plan-17).
            self.observe_float(&args[2], &item)?;
            if item.type_ != element_type {
                return Err(format!(
                    "native collection set list item must be {}, got {}",
                    element_type, item.type_
                ));
            }
            // Materialize a `d`-native float before the payload spill (plan-01).
            let item = self.materialize_value(item)?;
            // Do the fallible `removeAt` (which range-checks the index) BEFORE
            // materializing the singleton, so an out-of-range index — the failure
            // an inline `TRAP`'d or auto-propagating `set` hits — routes to the
            // handler with nothing yet allocated, and cannot leak the singleton
            // (bug-147.5). `removeAt` allocates its product only after the bounds
            // pass, so the OOB route allocates nothing at all. Both intermediates
            // are freed on the success path once the insert has copied out of them;
            // the sole remaining leak window is a mid-operation OOM (arena already
            // exhausted), which was equally present before this reorder.
            let removed =
                self.lower_list_remove_at(list_slot, index_slot, &collection.type_, &element_type)?;
            let removed_slot = self.allocate_stack_object("set_removed_list", 8);
            self.emit(abi::store_u64(
                &removed.location,
                abi::stack_pointer(),
                removed_slot,
            ));
            let (singleton_slot, materialized) =
                self.collection_argument_as_list_slot(&collection.type_, &element_type, item)?;
            let mut result = self.lower_list_insert_collection(
                removed_slot,
                index_slot,
                singleton_slot,
                &collection.type_,
                &element_type,
            )?;
            // Both intermediates were fully copied into the result: the
            // materialized singleton and the removeAt product.
            if materialized {
                result =
                    self.free_intermediate_collection(singleton_slot, &collection.type_, result)?;
            }
            return self.free_intermediate_collection(removed_slot, &collection.type_, result);
        }

        if let Some((key_type, value_type)) = map_type_parts(&collection.type_) {
            let map_slot = self.allocate_stack_object("set_map", 8);
            self.emit(abi::store_u64(
                &collection.location,
                abi::stack_pointer(),
                map_slot,
            ));
            let key = self.lower_value(&args[1])?;
            // Observation boundary: a `Float` map key must be finite (plan-17).
            self.observe_float(&args[1], &key)?;
            if key.type_ != key_type {
                return Err(format!(
                    "native collection set map key must be {}, got {}",
                    key_type, key.type_
                ));
            }
            let key = self.materialize_value(key)?;
            let key_slot = self.allocate_stack_object("set_map_key", 8);
            self.emit(abi::store_u64(
                &key.location,
                abi::stack_pointer(),
                key_slot,
            ));
            let value = self.lower_value(&args[2])?;
            // Observation boundary: a `Float` map value must be finite (plan-17).
            self.observe_float(&args[2], &value)?;
            if value.type_ != value_type {
                return Err(format!(
                    "native collection set map value must be {}, got {}",
                    value_type, value.type_
                ));
            }
            let value = self.materialize_value(value)?;
            let value_slot = self.allocate_stack_object("set_map_value", 8);
            self.emit(abi::store_u64(
                &value.location,
                abi::stack_pointer(),
                value_slot,
            ));
            let without =
                self.lower_map_remove_key(map_slot, key_slot, &collection.type_, &key_type)?;
            let without_slot = self.allocate_stack_object("set_map_without", 8);
            self.emit(abi::store_u64(
                &without.location,
                abi::stack_pointer(),
                without_slot,
            ));
            let singleton = self.lower_collection_values(
                &collection.type_,
                vec![CollectionValueSlot {
                    key: Some(PayloadSlot {
                        slot: key_slot,
                        type_: key_type.clone(),
                    }),
                    value: PayloadSlot {
                        slot: value_slot,
                        type_: value_type,
                    },
                }],
                "singleton map",
            )?;
            let singleton_slot = self.allocate_stack_object("set_map_singleton", 8);
            self.emit(abi::store_u64(
                &singleton.location,
                abi::stack_pointer(),
                singleton_slot,
            ));
            // The concat copies both intermediates into the result; free the
            // `without` whole-map copy and the `singleton` map afterward, mirroring
            // the list branch's frees. Without this every non-in-place map `set`
            // leaked one whole-map-sized block plus a singleton per call (bug-145).
            let result = self.lower_map_concat(without_slot, singleton_slot, &collection.type_)?;
            let result =
                self.free_intermediate_collection(without_slot, &collection.type_, result)?;
            return self.free_intermediate_collection(singleton_slot, &collection.type_, result);
        }

        Err(format!(
            "native collection set does not accept {} yet",
            collection.type_
        ))
    }

    pub(super) fn lower_collection_remove_key(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let map = self.lower_value(&args[0])?;
        let Some((key_type, _)) = map_type_parts(&map.type_) else {
            return Err(format!(
                "native collection removeKey does not accept {}",
                map.type_
            ));
        };
        let map_slot = self.allocate_stack_object("remove_key_map", 8);
        self.emit(abi::store_u64(
            &map.location,
            abi::stack_pointer(),
            map_slot,
        ));
        let key = self.lower_value(&args[1])?;
        if key.type_ != key_type {
            return Err(format!(
                "native collection removeKey key must be {}, got {}",
                key_type, key.type_
            ));
        }
        let key_slot = self.allocate_stack_object("remove_key_key", 8);
        // `d`-native float key stores via `str d` (plan-01 float-dnative).
        self.store_value_at(&key, abi::stack_pointer(), key_slot);
        self.lower_map_remove_key(map_slot, key_slot, &map.type_, &key_type)
    }

    /// Insert collection `B` (insert_slot) into list `A` (base_slot) at
    /// `index_slot` using the offset-stable scheme (plan-01 §4.1): copy `A`'s and
    /// `B`'s data regions verbatim (no per-entry repack), then splice the lookup
    /// table with three block moves — head `A` entries verbatim, `B` entries with
    /// each `valueOffset` shifted by `A.dataLength`, tail `A` entries verbatim.
    /// Append is `index == count`, prepend is `index == 0`. Phase 2 keeps the
    /// result tight (`capacity == count`).
    pub(super) fn lower_list_insert_collection(
        &mut self,
        base_slot: usize,
        index_slot: usize,
        insert_slot: usize,
        list_type: &str,
        element_type: &str,
    ) -> Result<ValueResult, String> {
        let scratch20 = self.temporary_vreg();
        let scratch21 = self.temporary_vreg();
        let scratch22 = self.temporary_vreg();
        let scratch23 = self.temporary_vreg();
        let scratch24 = self.temporary_vreg();
        let scratch25 = self.temporary_vreg();
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        let scratch15 = self.temporary_vreg();
        let scratch16 = self.temporary_vreg();
        let scratch17 = self.temporary_vreg();
        let layout = CollectionTypeLayout::from_type(list_type)
            .ok_or_else(|| format!("native code collection type '{list_type}' is not supported"))?;
        for register in [
            scratch20.as_str(),
            scratch21.as_str(),
            scratch22.as_str(),
            scratch23.as_str(),
            scratch24.as_str(),
            scratch25.as_str(),
        ] {
            self.mark_register_used(register);
        }
        // bug-175 E: a variable-length element payload (a record with an inline
        // String, a data union, or a flat nested collection) must start on an
        // aligned offset, exactly as the literal writer pads it
        // (builder_collection_layout.rs). `list_element_padding_alignment` returns
        // 1 for every fixed-size / byte-addressed payload, so every guarded
        // `emit_align_offset_*` below is elided and primitive lists stay
        // byte-identical.
        let value_alignment = self.list_element_padding_alignment(element_type);
        let result_slot = self.allocate_stack_object("list_insert_result", 8);
        let valid_start = self.label("list_insert_valid_start");
        let alloc_ok = self.label("list_insert_alloc_ok");
        let invalid = self.label("list_insert_invalid");
        let done = self.label("list_insert_done");

        // Validate 0 <= index <= count(A), then size the allocation:
        //   HEADER + (count_A + count_B) * ENTRY + (dataLen_A + dataLen_B).
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), base_slot));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), insert_slot));
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), index_slot));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch8,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::compare_immediate(&scratch10, "0"));
        self.emit(abi::branch_ge(&valid_start));
        self.emit(abi::branch(&invalid));
        self.emit(abi::label(&valid_start));
        self.emit(abi::compare_registers(&scratch10, &scratch11));
        self.emit(abi::branch_gt(&invalid));
        self.emit(abi::load_u64(
            &scratch12,
            &scratch9,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::add_registers(&scratch13, &scratch11, &scratch12));
        self.emit(abi::load_u64(
            &scratch14,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        // bug-175 E: pad A's data length up to the element alignment so B's data
        // region — and this reserved allocation size — start on an aligned
        // boundary (consistent with the header write and the data/entry copies).
        if value_alignment > 1 {
            let align_scratch = self.temporary_vreg();
            self.emit_align_offset_register(&scratch14, value_alignment, &align_scratch);
        }
        self.emit(abi::load_u64(
            &scratch15,
            &scratch9,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::add_registers(&scratch15, &scratch14, &scratch15));
        let size_overflow = self.label("list_insert_size_overflow");
        self.emit(abi::move_immediate(
            &scratch16,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        // Checked collection-size arithmetic (bug-147.7): count and dataLength are
        // read from live collection headers, so route count*ENTRY + HEADER + dataLen
        // through the overflow-guarded helpers — a wrapped size would under-allocate
        // and corrupt the heap.
        self.emit_checked_size_multiply(&scratch17, &scratch13, &scratch16, &size_overflow);
        self.emit_checked_size_add_immediate(
            abi::return_register(),
            &scratch17,
            COLLECTION_HEADER_SIZE,
            &size_overflow,
        );
        self.emit_checked_size_add(
            abi::return_register(),
            abi::return_register(),
            &scratch15,
            &size_overflow,
        );
        self.emit(abi::move_immediate(abi::ARG[1], "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&size_overflow));
        self.emit_error_code_return(ERR_OUT_OF_MEMORY_CODE, ERR_ALLOCATION_MESSAGE)?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(
            abi::RET[1],
            abi::stack_pointer(),
            result_slot,
        ));
        let nb = self.temporary_vreg();
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), result_slot));

        // Header: total count / total data length (recomputed from the pointer
        // slots, which survive `arena_alloc`; the pre-alloc registers do not).
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), base_slot));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), insert_slot));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch8,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::load_u64(
            &scratch12,
            &scratch9,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::add_registers(&scratch13, &scratch11, &scratch12));
        self.emit(abi::load_u64(
            &scratch14,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        // bug-175 E: the stored dataLength must include the pad between A's and B's
        // data regions so it matches the reserved size and the aligned copy dst.
        if value_alignment > 1 {
            let align_scratch = self.temporary_vreg();
            self.emit_align_offset_register(&scratch14, value_alignment, &align_scratch);
        }
        self.emit(abi::load_u64(
            &scratch15,
            &scratch9,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::add_registers(&scratch15, &scratch14, &scratch15));
        self.emit_write_list_header_from_registers(&layout, &nb, &scratch13, &scratch15);

        // --- Data region: A verbatim, then B verbatim at offset dataLen_A. ---
        self.emit_collection_data_pointer(&scratch17, &nb); // dst data base
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), base_slot));
        self.emit_collection_data_pointer(&scratch20, &scratch8); // A data base
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
            "list_insert_dataA",
        );
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), insert_slot));
        self.emit_collection_data_pointer(&scratch20, &scratch9); // B data base
        self.emit(abi::load_u64(
            &scratch15,
            &scratch9,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        // bug-175 E: round the dst data cursor (data base + dataLen_A) up to the
        // element alignment before copying B's region. The data base is 8-aligned
        // (HEADER and ENTRY are multiples of 8, block alloc is 8-aligned), so this
        // is exactly `base + align(dataLen_A)` — the same padded dataLen_A the size,
        // header, and entry-offset shift use.
        if value_alignment > 1 {
            let align_scratch = self.temporary_vreg();
            self.emit_align_offset_register(&scratch17, value_alignment, &align_scratch);
        }
        self.emit_block_copy_advance(
            &scratch17,
            &scratch20,
            &scratch15,
            &scratch22,
            "list_insert_dataB",
        );

        // --- Lookup table splice. ---
        self.emit(abi::move_immediate(
            &scratch16,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        // Head: dst.table[0..i) <- A.table[0..i) verbatim.
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), result_slot));
        self.emit(abi::add_immediate(&scratch17, &nb, COLLECTION_HEADER_SIZE)); // dst table cursor
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), base_slot));
        self.emit(abi::add_immediate(
            &scratch20,
            &scratch8,
            COLLECTION_HEADER_SIZE,
        )); // A table cursor
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), index_slot));
        self.emit(abi::multiply_registers(&scratch21, &scratch10, &scratch16)); // i * ENTRY
        self.emit_block_copy_advance(
            &scratch17,
            &scratch20,
            &scratch21,
            &scratch22,
            "list_insert_head",
        );

        // Inserted: dst.table[i..i+count_B) <- B entries, valueOffset += dataLen_A.
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), insert_slot));
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch9,
            COLLECTION_HEADER_SIZE,
        )); // B table cursor
        self.emit(abi::load_u64(
            &scratch11,
            &scratch9,
            COLLECTION_OFFSET_COUNT,
        )); // remaining B entries
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), base_slot));
        self.emit(abi::load_u64(
            &scratch14,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        )); // dataLen_A shift
            // Bulk-copy the `count_B` inserted entries verbatim, then shift each
            // valueOffset up by dataLen_A — their payloads now sit after A's data
            // region (plan-25-B B2). Copying B's entries and B's data region both
            // verbatim (a single uniform offset shift) preserves B's internal layout
            // even when B is not packed in entry order, unlike a per-entry re-pack.
            // The copy advances `scratch17` to dst.table[i+count_B], where the tail
            // copy below resumes.
            // bug-175 E: shift B's valueOffsets by the padded A data length so they
            // match the aligned destination of B's copied data region above.
        if value_alignment > 1 {
            let align_scratch = self.temporary_vreg();
            self.emit_align_offset_register(&scratch14, value_alignment, &align_scratch);
        }
        self.emit_bulk_copy_entries_shift(
            &scratch12,
            &scratch17,
            &scratch11,
            Some((&scratch14, false)),
            "list_insert_b",
        );

        // Tail: dst.table[i+count_B..] <- A.table[i..) verbatim. x20 already points
        // at A entry i (advanced past the head copy).
        self.emit(abi::move_immediate(
            &scratch16,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), base_slot));
        self.emit(abi::load_u64(
            &scratch13,
            &scratch8,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), index_slot));
        self.emit(abi::subtract_registers(&scratch13, &scratch13, &scratch10)); // count_A - i
        self.emit(abi::multiply_registers(&scratch21, &scratch13, &scratch16));
        self.emit_block_copy_advance(
            &scratch17,
            &scratch20,
            &scratch21,
            &scratch22,
            "list_insert_tail",
        );
        self.emit(abi::branch(&done));
        self.emit(abi::label(&invalid));
        self.emit_index_out_of_range_return()?;
        self.emit(abi::label(&done));

        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: list_type.to_string(),
            location: result,
            text: format!("list update {list_type} over {element_type}"),
        })
    }

    /// Append a single element (held in `item_slot`) to the list buffer whose
    /// pointer lives in `buffer_slot`, **mutating `buffer_slot` in place**
    /// (plan-01 §4.2). When the live buffer has a spare lookup slot and spare
    /// data bytes, the element is written into the spare slot and `count` /
    /// `dataLength` are bumped — no allocation, no copy: the amortized-O(1)
    /// lever. Otherwise a geometric-headroom buffer is allocated, the entries
    /// and data region are copied verbatim, and `buffer_slot` is repointed at it;
    /// the element is then written into the now-spare slot. The caller guarantees
    /// the buffer is uniquely owned (see the Assign-site / private-accumulator
    /// call sites). Returns the (possibly new) buffer pointer.
    pub(super) fn lower_list_append_in_place(
        &mut self,
        buffer_slot: usize,
        item_slot: usize,
        list_type: &str,
        element_type: &str,
    ) -> Result<ValueResult, String> {
        let scratch20 = self.temporary_vreg();
        let scratch21 = self.temporary_vreg();
        let scratch22 = self.temporary_vreg();
        let scratch23 = self.temporary_vreg();
        let scratch24 = self.temporary_vreg();
        let scratch25 = self.temporary_vreg();
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        let scratch15 = self.temporary_vreg();
        let scratch16 = self.temporary_vreg();
        let scratch17 = self.temporary_vreg();
        let layout = CollectionTypeLayout::from_type(list_type)
            .ok_or_else(|| format!("native code collection type '{list_type}' is not supported"))?;
        for register in [
            scratch20.as_str(),
            scratch21.as_str(),
            scratch22.as_str(),
            scratch23.as_str(),
            scratch24.as_str(),
            scratch25.as_str(),
        ] {
            self.mark_register_used(register);
        }
        let item = PayloadSlot {
            slot: item_slot,
            type_: element_type.to_string(),
        };
        // bug-175 E: pad the element start offset like the literal writer. Returns
        // 1 (a no-op every align site) for fixed-size / byte-addressed payloads, so
        // primitive lists stay byte-identical.
        let value_alignment = self.list_element_padding_alignment(element_type);
        // Byte size of the new payload (its required data bytes).
        let need_slot = self.emit_payload_length_to_stack(&item, "append_inplace_need")?;
        let data_offset_slot = self.allocate_stack_object("append_inplace_doff", 8);
        let new_cap_slot = self.allocate_stack_object("append_inplace_newcap", 8);
        let new_dcap_slot = self.allocate_stack_object("append_inplace_newdcap", 8);
        let new_buf_slot = self.allocate_stack_object("append_inplace_newbuf", 8);

        let realloc = self.label("append_inplace_realloc");
        let write = self.label("append_inplace_write");
        let alloc_ok = self.label("append_inplace_alloc_ok");
        let dcap_keep = self.label("append_inplace_dcap_keep");

        // Room check: count < capacity AND dataLength + need <= dataCapacity.
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(
            &scratch10,
            &scratch8,
            COLLECTION_OFFSET_CAPACITY,
        ));
        self.emit(abi::compare_registers(&scratch9, &scratch10));
        self.emit(abi::branch_ge(&realloc));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        // bug-175 E: reserve for align(dataLength)+need, matching the aligned write
        // offset below — a "just barely fits" unaligned list must not skip the grow.
        if value_alignment > 1 {
            let align_scratch = self.temporary_vreg();
            self.emit_align_offset_register(&scratch11, value_alignment, &align_scratch);
        }
        self.emit(abi::load_u64(&scratch12, abi::stack_pointer(), need_slot));
        self.emit(abi::add_registers(&scratch11, &scratch11, &scratch12));
        self.emit(abi::load_u64(
            &scratch13,
            &scratch8,
            COLLECTION_OFFSET_DATA_CAPACITY,
        ));
        self.emit(abi::compare_registers(&scratch11, &scratch13));
        self.emit(abi::branch_hi(&realloc));
        self.emit(abi::branch(&write));

        // --- Grow: allocate a headroom buffer; copy entries + data verbatim. ---
        self.emit(abi::label(&realloc));
        // newCapacity = step(capacity).
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
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
            "append_grow_cap",
        );
        self.emit(abi::store_u64(
            &scratch14,
            abi::stack_pointer(),
            new_cap_slot,
        ));
        // newDataCapacity = max(step(dataCapacity), dataLength + need).
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
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
            "append_grow_dcap",
        );
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        // bug-175 E: required = align(dataLength)+need, consistent with the room
        // check and the write so the grown dataCapacity always holds the payload.
        if value_alignment > 1 {
            let align_scratch = self.temporary_vreg();
            self.emit_align_offset_register(&scratch11, value_alignment, &align_scratch);
        }
        self.emit(abi::load_u64(&scratch12, abi::stack_pointer(), need_slot));
        self.emit(abi::add_registers(&scratch11, &scratch11, &scratch12)); // required
        self.emit(abi::compare_registers(&scratch14, &scratch11));
        self.emit(abi::branch_hi(&dcap_keep));
        self.emit(abi::branch_eq(&dcap_keep));
        self.emit(abi::move_register(&scratch14, &scratch11)); // step < required → use required
        self.emit(abi::label(&dcap_keep));
        self.emit(abi::store_u64(
            &scratch14,
            abi::stack_pointer(),
            new_dcap_slot,
        ));

        // alloc size = HEADER + newCapacity * ENTRY + newDataCapacity.
        self.emit(abi::load_u64(
            &scratch14,
            abi::stack_pointer(),
            new_cap_slot,
        ));
        let size_overflow = self.label("list_append_size_overflow");
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
        self.emit(abi::move_immediate(abi::ARG[1], "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&size_overflow));
        self.emit_error_code_return(ERR_OUT_OF_MEMORY_CODE, ERR_ALLOCATION_MESSAGE)?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(
            abi::RET[1],
            abi::stack_pointer(),
            new_buf_slot,
        ));

        // Header: old count / old dataLength, new capacity / data capacity.
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
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
        self.emit_collection_data_pointer(&scratch17, &nb);
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
        self.emit_collection_data_pointer(&scratch20, &scratch8);
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
            "append_grow_data",
        );

        // Copy the live lookup entries verbatim (count * ENTRY bytes).
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), new_buf_slot));
        self.emit(abi::add_immediate(&scratch17, &nb, COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
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
            "append_grow_entries",
        );

        // Free the old buffer, then install the grown one. The in-place append
        // fires only under unique ownership, so the old buffer has no other
        // reference; freeing it here stops the geometric-grow path from leaking
        // it (bug-01: every append that outgrew its capacity accumulated the
        // abandoned buffer, so append-heavy loops grew the arena without bound).
        // Sized capacity-based (HEADER + capacity*ENTRY + dataCapacity), matching
        // what the allocation reserved.
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(
            &scratch9,
            &scratch8,
            COLLECTION_OFFSET_CAPACITY,
        ));
        self.emit(abi::move_immediate(
            &scratch16,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch10, &scratch9, &scratch16));
        self.emit(abi::add_immediate(
            &scratch10,
            &scratch10,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch8,
            COLLECTION_OFFSET_DATA_CAPACITY,
        ));
        self.emit(abi::add_registers(abi::ARG[1], &scratch10, &scratch11));
        self.emit(abi::move_register(abi::return_register(), &scratch8));
        self.emit(abi::branch_link(ARENA_FREE_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_FREE_SYMBOL.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
        // Install the grown buffer; fall through to write the new element.
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), new_buf_slot));
        self.emit(abi::store_u64(&nb, abi::stack_pointer(), buffer_slot));
        self.emit(abi::branch(&write));

        // --- Write the new element into slot[count], payload at dataLength. ---
        self.emit(abi::label(&write));
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        // bug-175 E: the payload starts at the aligned dataLength; this scratch11
        // feeds both the entry valueOffset and the data-copy offset below.
        if value_alignment > 1 {
            let align_scratch = self.temporary_vreg();
            self.emit_align_offset_register(&scratch11, value_alignment, &align_scratch);
        }
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch8,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::move_immediate(
            &scratch16,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch13, &scratch9, &scratch16));
        self.emit(abi::add_registers(&scratch12, &scratch12, &scratch13)); // entry addr
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
        self.emit(abi::move_immediate(&scratch13, "Integer", "0"));
        self.emit(abi::store_u64(
            &scratch13,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::store_u64(
            &scratch13,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::store_u64(
            &scratch11,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        )); // valueOffset = dataLength
        self.emit(abi::load_u64(&scratch13, abi::stack_pointer(), need_slot));
        self.emit(abi::store_u64(
            &scratch13,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        )); // valueLength = need
            // Copy the payload bytes to data base + dataLength.
        self.emit(abi::store_u64(
            &scratch11,
            abi::stack_pointer(),
            data_offset_slot,
        ));
        self.emit_copy_payload_to_collection(buffer_slot, need_slot, &item, data_offset_slot)?;
        // Bump count and dataLength.
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::add_immediate(&scratch9, &scratch9, 1));
        self.emit(abi::store_u64(
            &scratch9,
            &scratch8,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::load_u64(
            &scratch9,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        // bug-175 E: the new dataLength is align(old)+need so it accounts for the
        // pad the payload was written past (keeps the next element aligned too).
        if value_alignment > 1 {
            let align_scratch = self.temporary_vreg();
            self.emit_align_offset_register(&scratch9, value_alignment, &align_scratch);
        }
        self.emit(abi::load_u64(&scratch13, abi::stack_pointer(), need_slot));
        self.emit(abi::add_registers(&scratch9, &scratch9, &scratch13));
        self.emit(abi::store_u64(
            &scratch9,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));

        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), buffer_slot));
        Ok(ValueResult {
            type_: list_type.to_string(),
            location: result,
            text: format!("append in place {list_type} over {element_type}"),
        })
    }

    /// Bulk-append the list at `rhs_slot` onto the list buffer at `buffer_slot`,
    /// **mutating `buffer_slot` in place** (plan-25-B B1). This is the list-RHS
    /// sibling of [`Self::lower_list_append_in_place`]: it grows the uniquely-owned
    /// working buffer once (geometric headroom, sized for the whole batch) when
    /// `count(self) + count(rhs)` entries or `dataLength(self) + dataLength(rhs)`
    /// bytes do not fit, then bulk-copies the RHS data region into the spare data
    /// tail and the RHS lookup entries into the spare slots — each RHS entry's
    /// `valueOffset` shifted by the pre-append `dataLength(self)` since its payload
    /// now sits after `self`'s data. Amortized O(rhs) per call instead of the
    /// value-semantic rebuild that copies the whole accumulated result each call
    /// (the O(n²) `flatten`/`append_batch` path). The caller guarantees `self` is
    /// uniquely owned and that `rhs` is a distinct buffer (the `append(list, list)`
    /// self-alias is excluded at the gate and takes the value path).
    pub(super) fn lower_list_bulk_append_in_place(
        &mut self,
        buffer_slot: usize,
        rhs_slot: usize,
        list_type: &str,
        element_type: &str,
    ) -> Result<ValueResult, String> {
        let layout = CollectionTypeLayout::from_type(list_type)
            .ok_or_else(|| format!("native code collection type '{list_type}' is not supported"))?;
        let s8 = self.temporary_vreg();
        let s9 = self.temporary_vreg();
        let s10 = self.temporary_vreg();
        let s11 = self.temporary_vreg();
        let s12 = self.temporary_vreg();
        let s13 = self.temporary_vreg();
        let s14 = self.temporary_vreg();
        let s15 = self.temporary_vreg();
        let s16 = self.temporary_vreg();
        let s17 = self.temporary_vreg();
        let s20 = self.temporary_vreg();
        let s22 = self.temporary_vreg();

        // bug-175 E: pad self's data length up to the element alignment before rhs's
        // data region is concatenated after it, so rhs's internally-aligned payloads
        // stay aligned. 1 (a no-op at every align site) for fixed-size /
        // byte-addressed payloads keeps primitive lists byte-identical.
        let value_alignment = self.list_element_padding_alignment(element_type);
        let need_count_slot = self.allocate_stack_object("bulk_append_need_count", 8);
        let need_data_slot = self.allocate_stack_object("bulk_append_need_data", 8);
        let new_cap_slot = self.allocate_stack_object("bulk_append_newcap", 8);
        let new_dcap_slot = self.allocate_stack_object("bulk_append_newdcap", 8);
        let new_buf_slot = self.allocate_stack_object("bulk_append_newbuf", 8);

        let realloc = self.label("bulk_append_realloc");
        let write = self.label("bulk_append_write");
        let alloc_ok = self.label("bulk_append_alloc_ok");
        let cap_keep = self.label("bulk_append_cap_keep");
        let dcap_keep = self.label("bulk_append_dcap_keep");

        // need_count = count(self) + count(rhs); need_data = dataLen(self) + dataLen(rhs).
        self.emit(abi::load_u64(&s8, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&s9, &s8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(&s10, abi::stack_pointer(), rhs_slot));
        self.emit(abi::load_u64(&s11, &s10, COLLECTION_OFFSET_COUNT));
        self.emit(abi::add_registers(&s12, &s9, &s11));
        self.emit(abi::store_u64(&s12, abi::stack_pointer(), need_count_slot));
        self.emit(abi::load_u64(&s13, &s8, COLLECTION_OFFSET_DATA_LENGTH));
        // bug-175 E: need_data = align(dataLen_self)+dataLen_rhs. This drives the
        // room check, the grown dataCapacity, AND the stored dataLength, so the pad
        // between the two data regions is reserved everywhere it is consumed.
        if value_alignment > 1 {
            let align_scratch = self.temporary_vreg();
            self.emit_align_offset_register(&s13, value_alignment, &align_scratch);
        }
        self.emit(abi::load_u64(&s14, &s10, COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::add_registers(&s15, &s13, &s14));
        self.emit(abi::store_u64(&s15, abi::stack_pointer(), need_data_slot));

        // Room check: need_count <= capacity AND need_data <= dataCapacity.
        self.emit(abi::load_u64(&s16, &s8, COLLECTION_OFFSET_CAPACITY));
        self.emit(abi::compare_registers(&s12, &s16));
        self.emit(abi::branch_hi(&realloc));
        self.emit(abi::load_u64(&s17, &s8, COLLECTION_OFFSET_DATA_CAPACITY));
        self.emit(abi::compare_registers(&s15, &s17));
        self.emit(abi::branch_hi(&realloc));
        self.emit(abi::branch(&write));

        // --- Grow: allocate a headroom buffer sized for the whole batch; copy
        // self's entries + data verbatim; free the old buffer; install. ---
        self.emit(abi::label(&realloc));
        // newCapacity = max(step(capacity), need_count).
        self.emit(abi::load_u64(&s8, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&s10, &s8, COLLECTION_OFFSET_CAPACITY));
        self.emit_geometric_step(
            &s10,
            &s14,
            &s15,
            COLLECTION_GROW_LOOKUP_INIT,
            COLLECTION_GROW_LOOKUP_TAPER,
            "bulk_append_grow_cap",
        );
        self.emit(abi::load_u64(&s11, abi::stack_pointer(), need_count_slot));
        self.emit(abi::compare_registers(&s14, &s11));
        self.emit(abi::branch_hi(&cap_keep));
        self.emit(abi::branch_eq(&cap_keep));
        self.emit(abi::move_register(&s14, &s11));
        self.emit(abi::label(&cap_keep));
        self.emit(abi::store_u64(&s14, abi::stack_pointer(), new_cap_slot));
        // newDataCapacity = max(step(dataCapacity), need_data).
        self.emit(abi::load_u64(&s8, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&s10, &s8, COLLECTION_OFFSET_DATA_CAPACITY));
        self.emit_geometric_step(
            &s10,
            &s14,
            &s15,
            COLLECTION_GROW_DATA_INIT,
            COLLECTION_GROW_DATA_TAPER,
            "bulk_append_grow_dcap",
        );
        self.emit(abi::load_u64(&s11, abi::stack_pointer(), need_data_slot));
        self.emit(abi::compare_registers(&s14, &s11));
        self.emit(abi::branch_hi(&dcap_keep));
        self.emit(abi::branch_eq(&dcap_keep));
        self.emit(abi::move_register(&s14, &s11));
        self.emit(abi::label(&dcap_keep));
        self.emit(abi::store_u64(&s14, abi::stack_pointer(), new_dcap_slot));

        // alloc size = HEADER + newCapacity * ENTRY + newDataCapacity.
        self.emit(abi::load_u64(&s14, abi::stack_pointer(), new_cap_slot));
        let size_overflow = self.label("list_bulk_append_size_overflow");
        self.emit(abi::move_immediate(
            &s16,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        // Checked collection-size arithmetic (bug-147.7): capacity/dataCapacity are
        // runtime-derived, so guard count*ENTRY + HEADER + dataCap against overflow.
        self.emit_checked_size_multiply(&s17, &s14, &s16, &size_overflow);
        self.emit_checked_size_add_immediate(
            abi::return_register(),
            &s17,
            COLLECTION_HEADER_SIZE,
            &size_overflow,
        );
        self.emit(abi::load_u64(&s15, abi::stack_pointer(), new_dcap_slot));
        self.emit_checked_size_add(
            abi::return_register(),
            abi::return_register(),
            &s15,
            &size_overflow,
        );
        self.emit(abi::move_immediate(abi::ARG[1], "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&size_overflow));
        self.emit_error_code_return(ERR_OUT_OF_MEMORY_CODE, ERR_ALLOCATION_MESSAGE)?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(
            abi::RET[1],
            abi::stack_pointer(),
            new_buf_slot,
        ));

        // Header: old count / old dataLength, new capacity / new data capacity.
        self.emit(abi::load_u64(&s8, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&s9, &s8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(&s11, &s8, COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::load_u64(&s14, abi::stack_pointer(), new_cap_slot));
        self.emit(abi::load_u64(&s15, abi::stack_pointer(), new_dcap_slot));
        let nb = self.temporary_vreg();
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), new_buf_slot));
        self.emit_write_collection_header_full(&layout, &nb, &s9, &s14, &s11, &s15);

        // Copy self's data region verbatim (dataLength bytes), capacity-based base.
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), new_buf_slot));
        self.emit_collection_data_pointer(&s17, &nb);
        self.emit(abi::load_u64(&s8, abi::stack_pointer(), buffer_slot));
        self.emit_collection_data_pointer(&s20, &s8);
        self.emit(abi::load_u64(&s14, &s8, COLLECTION_OFFSET_DATA_LENGTH));
        self.emit_block_copy_advance(&s17, &s20, &s14, &s22, "bulk_append_grow_data");

        // Copy self's live lookup entries verbatim (count * ENTRY bytes).
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), new_buf_slot));
        self.emit(abi::add_immediate(&s17, &nb, COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64(&s8, abi::stack_pointer(), buffer_slot));
        self.emit(abi::add_immediate(&s20, &s8, COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64(&s9, &s8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::move_immediate(
            &s16,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&s14, &s9, &s16));
        self.emit_block_copy_advance(&s17, &s20, &s14, &s22, "bulk_append_grow_entries");

        // Free the old buffer (capacity-based size), then install the grown one.
        // Unique ownership means it has no other reference — leaving it unfreed
        // would leak one buffer per outgrown bulk append.
        self.emit(abi::load_u64(&s8, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&s9, &s8, COLLECTION_OFFSET_CAPACITY));
        self.emit(abi::move_immediate(
            &s16,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&s10, &s9, &s16));
        self.emit(abi::add_immediate(&s10, &s10, COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64(&s11, &s8, COLLECTION_OFFSET_DATA_CAPACITY));
        self.emit(abi::add_registers(abi::ARG[1], &s10, &s11));
        self.emit(abi::move_register(abi::return_register(), &s8));
        self.emit(abi::branch_link(ARENA_FREE_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_FREE_SYMBOL.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), new_buf_slot));
        self.emit(abi::store_u64(&nb, abi::stack_pointer(), buffer_slot));
        self.emit(abi::branch(&write));

        // --- Write: bulk-copy the RHS into self's spare region. ---
        self.emit(abi::label(&write));
        // dst.data + dataLength(self) <- rhs.data (dataLength(rhs) bytes).
        self.emit(abi::load_u64(&s8, abi::stack_pointer(), buffer_slot));
        self.emit_collection_data_pointer(&s17, &s8);
        self.emit(abi::load_u64(&s9, &s8, COLLECTION_OFFSET_DATA_LENGTH));
        // bug-175 E: place rhs's data at the padded self data length. The data base
        // is 8-aligned, so aligning the byte offset here yields `base +
        // align(dataLen_self)`, matching the padded need_data reservation.
        if value_alignment > 1 {
            let align_scratch = self.temporary_vreg();
            self.emit_align_offset_register(&s9, value_alignment, &align_scratch);
        }
        self.emit(abi::add_registers(&s17, &s17, &s9));
        self.emit(abi::load_u64(&s10, abi::stack_pointer(), rhs_slot));
        self.emit_collection_data_pointer(&s20, &s10);
        self.emit(abi::load_u64(&s14, &s10, COLLECTION_OFFSET_DATA_LENGTH));
        self.emit_block_copy_advance(&s17, &s20, &s14, &s22, "bulk_append_data");

        // dst.table[count(self)..] <- rhs entries, each valueOffset += dataLength(self).
        self.emit(abi::load_u64(&s8, abi::stack_pointer(), buffer_slot));
        self.emit(abi::add_immediate(&s17, &s8, COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64(&s9, &s8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::move_immediate(
            &s16,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&s11, &s9, &s16));
        self.emit(abi::add_registers(&s17, &s17, &s11)); // dst entry[count(self)]
        self.emit(abi::load_u64(&s10, abi::stack_pointer(), rhs_slot));
        self.emit(abi::add_immediate(&s20, &s10, COLLECTION_HEADER_SIZE)); // rhs entry base
        self.emit(abi::load_u64(&s11, &s10, COLLECTION_OFFSET_COUNT)); // count(rhs)
        self.emit(abi::load_u64(&s12, &s8, COLLECTION_OFFSET_DATA_LENGTH)); // shift = dataLength(self)
                                                                            // bug-175 E: shift rhs valueOffsets by the padded self data length to match
                                                                            // the aligned destination of rhs's copied data region above.
        if value_alignment > 1 {
            let align_scratch = self.temporary_vreg();
            self.emit_align_offset_register(&s12, value_alignment, &align_scratch);
        }
        self.emit_bulk_copy_entries_shift(
            &s20,
            &s17,
            &s11,
            Some((&s12, false)),
            "bulk_append_entries",
        );

        // Bump count += count(rhs); dataLength += dataLength(rhs).
        self.emit(abi::load_u64(&s8, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&s9, abi::stack_pointer(), need_count_slot));
        self.emit(abi::store_u64(&s9, &s8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(&s9, abi::stack_pointer(), need_data_slot));
        self.emit(abi::store_u64(&s9, &s8, COLLECTION_OFFSET_DATA_LENGTH));

        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), buffer_slot));
        Ok(ValueResult {
            type_: list_type.to_string(),
            location: result,
            text: format!("bulk append in place {list_type} over {element_type}"),
        })
    }

    /// Prepend a single element (held in `item_slot`) to the front of the list
    /// whose pointer lives in `buffer_slot`, **mutating `buffer_slot` in place**
    /// (plan-02 §3). Ensures room exactly like `lower_list_append_in_place`
    /// (geometric grow only when full), then shifts the live lookup entries right
    /// by one (so the new entry takes index 0) and appends the element's payload to
    /// the spare data tail — entry offsets are independent of position, so no data
    /// move is needed. Still O(n) per call (the entry shift), but it drops the
    /// per-call allocation + double copy of the value-semantic insert. The caller
    /// guarantees unique ownership and not an active `FOR EACH` iterable.
    pub(super) fn lower_list_prepend_in_place(
        &mut self,
        buffer_slot: usize,
        item_slot: usize,
        list_type: &str,
        element_type: &str,
    ) -> Result<ValueResult, String> {
        let scratch20 = self.temporary_vreg();
        let scratch21 = self.temporary_vreg();
        let scratch22 = self.temporary_vreg();
        let scratch23 = self.temporary_vreg();
        let scratch24 = self.temporary_vreg();
        let scratch25 = self.temporary_vreg();
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        let scratch15 = self.temporary_vreg();
        let scratch16 = self.temporary_vreg();
        let scratch17 = self.temporary_vreg();
        let layout = CollectionTypeLayout::from_type(list_type)
            .ok_or_else(|| format!("native code collection type '{list_type}' is not supported"))?;
        for register in [
            scratch20.as_str(),
            scratch21.as_str(),
            scratch22.as_str(),
            scratch23.as_str(),
            scratch24.as_str(),
            scratch25.as_str(),
        ] {
            self.mark_register_used(register);
        }
        let item = PayloadSlot {
            slot: item_slot,
            type_: element_type.to_string(),
        };
        // bug-175 E: pad the element start offset like the literal writer; 1 (a
        // no-op at every align site) for fixed-size / byte-addressed payloads keeps
        // primitive lists byte-identical.
        let value_alignment = self.list_element_padding_alignment(element_type);
        let need_slot = self.emit_payload_length_to_stack(&item, "prepend_inplace_need")?;
        let data_offset_slot = self.allocate_stack_object("prepend_inplace_doff", 8);
        let new_cap_slot = self.allocate_stack_object("prepend_inplace_newcap", 8);
        let new_dcap_slot = self.allocate_stack_object("prepend_inplace_newdcap", 8);
        let new_buf_slot = self.allocate_stack_object("prepend_inplace_newbuf", 8);

        let realloc = self.label("prepend_inplace_realloc");
        let write = self.label("prepend_inplace_write");
        let alloc_ok = self.label("prepend_inplace_alloc_ok");
        let dcap_keep = self.label("prepend_inplace_dcap_keep");
        let shift_loop = self.label("prepend_inplace_shift_loop");
        let shift_done = self.label("prepend_inplace_shift_done");

        // Room check: count < capacity AND dataLength + need <= dataCapacity.
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(
            &scratch10,
            &scratch8,
            COLLECTION_OFFSET_CAPACITY,
        ));
        self.emit(abi::compare_registers(&scratch9, &scratch10));
        self.emit(abi::branch_ge(&realloc));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        // bug-175 E: reserve for align(dataLength)+need, matching the aligned write
        // offset below — a "just barely fits" unaligned list must not skip the grow.
        if value_alignment > 1 {
            let align_scratch = self.temporary_vreg();
            self.emit_align_offset_register(&scratch11, value_alignment, &align_scratch);
        }
        self.emit(abi::load_u64(&scratch12, abi::stack_pointer(), need_slot));
        self.emit(abi::add_registers(&scratch11, &scratch11, &scratch12));
        self.emit(abi::load_u64(
            &scratch13,
            &scratch8,
            COLLECTION_OFFSET_DATA_CAPACITY,
        ));
        self.emit(abi::compare_registers(&scratch11, &scratch13));
        self.emit(abi::branch_hi(&realloc));
        self.emit(abi::branch(&write));

        // --- Grow: geometric headroom; copy entries + data verbatim. ---
        self.emit(abi::label(&realloc));
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
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
            "prepend_grow_cap",
        );
        self.emit(abi::store_u64(
            &scratch14,
            abi::stack_pointer(),
            new_cap_slot,
        ));
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
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
            "prepend_grow_dcap",
        );
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        // bug-175 E: required = align(dataLength)+need, consistent with the room
        // check and the write so the grown dataCapacity always holds the payload.
        if value_alignment > 1 {
            let align_scratch = self.temporary_vreg();
            self.emit_align_offset_register(&scratch11, value_alignment, &align_scratch);
        }
        self.emit(abi::load_u64(&scratch12, abi::stack_pointer(), need_slot));
        self.emit(abi::add_registers(&scratch11, &scratch11, &scratch12));
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
        let size_overflow = self.label("list_prepend_size_overflow");
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
        self.emit(abi::move_immediate(abi::ARG[1], "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&size_overflow));
        self.emit_error_code_return(ERR_OUT_OF_MEMORY_CODE, ERR_ALLOCATION_MESSAGE)?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(
            abi::RET[1],
            abi::stack_pointer(),
            new_buf_slot,
        ));
        // Header: old count / old dataLength, new capacity / data capacity.
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
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
        self.emit_collection_data_pointer(&scratch17, &nb);
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
        self.emit_collection_data_pointer(&scratch20, &scratch8);
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
            "prepend_grow_data",
        );
        // Copy the live lookup entries verbatim (count * ENTRY bytes).
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), new_buf_slot));
        self.emit(abi::add_immediate(&scratch17, &nb, COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
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
            "prepend_grow_entries",
        );
        // Free the abandoned pre-grow buffer (still in `buffer_slot`) before
        // installing the grown one — otherwise a prepend-in-a-loop leaks the old
        // buffer on every geometric grow (bug-47, mirrors `append` at :957-991).
        self.emit_free_pre_grow_buffer(buffer_slot, list_type)?;
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), new_buf_slot));
        self.emit(abi::store_u64(&nb, abi::stack_pointer(), buffer_slot));
        self.emit(abi::branch(&write));

        // --- Write: shift entries right by one, new entry at slot[0]. ---
        self.emit(abi::label(&write));
        // Shift lookup entries [0..count) → [1..count+1), backward to avoid overlap.
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::subtract_immediate(&scratch10, &scratch9, 1)); // i = count - 1
        self.emit(abi::label(&shift_loop));
        self.emit(abi::compare_immediate(&scratch10, "0"));
        self.emit(abi::branch_lt(&shift_done));
        // src = buffer + HEADER + i*ENTRY ; dst = src + ENTRY (x8 = buffer, live).
        self.emit(abi::move_immediate(
            &scratch16,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch11, &scratch10, &scratch16));
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch8,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(&scratch11, &scratch12, &scratch11)); // src = entry[i]
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch11,
            COLLECTION_ENTRY_SIZE,
        )); // dst = entry[i+1]
        for offset in [0usize, 8, 16, 24, 32] {
            self.emit(abi::load_u64(&scratch13, &scratch11, offset));
            self.emit(abi::store_u64(&scratch13, &scratch12, offset));
        }
        self.emit(abi::subtract_immediate(&scratch10, &scratch10, 1));
        self.emit(abi::branch(&shift_loop));
        self.emit(abi::label(&shift_done));
        // New entry at slot[0]: payload at dataLength, valueOffset = dataLength.
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        // bug-175 E: the new payload starts at the aligned dataLength; this
        // scratch11 feeds both the entry valueOffset and the data-copy offset.
        if value_alignment > 1 {
            let align_scratch = self.temporary_vreg();
            self.emit_align_offset_register(&scratch11, value_alignment, &align_scratch);
        }
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch8,
            COLLECTION_HEADER_SIZE,
        )); // entry[0]
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
        self.emit(abi::move_immediate(&scratch13, "Integer", "0"));
        self.emit(abi::store_u64(
            &scratch13,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::store_u64(
            &scratch13,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::store_u64(
            &scratch11,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(&scratch13, abi::stack_pointer(), need_slot));
        self.emit(abi::store_u64(
            &scratch13,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        // Copy the payload bytes to data base + dataLength.
        self.emit(abi::store_u64(
            &scratch11,
            abi::stack_pointer(),
            data_offset_slot,
        ));
        self.emit_copy_payload_to_collection(buffer_slot, need_slot, &item, data_offset_slot)?;
        // Bump count and dataLength.
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::add_immediate(&scratch9, &scratch9, 1));
        self.emit(abi::store_u64(
            &scratch9,
            &scratch8,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::load_u64(
            &scratch9,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        // bug-175 E: the new dataLength is align(old)+need so it accounts for the
        // pad the payload was written past (keeps the next element aligned too).
        if value_alignment > 1 {
            let align_scratch = self.temporary_vreg();
            self.emit_align_offset_register(&scratch9, value_alignment, &align_scratch);
        }
        self.emit(abi::load_u64(&scratch13, abi::stack_pointer(), need_slot));
        self.emit(abi::add_registers(&scratch9, &scratch9, &scratch13));
        self.emit(abi::store_u64(
            &scratch9,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));

        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), buffer_slot));
        Ok(ValueResult {
            type_: list_type.to_string(),
            location: result,
            text: format!("prepend in place {list_type} over {element_type}"),
        })
    }

    /// Replace the element at `index_slot` of the list whose buffer pointer lives
    /// in `buffer_slot`, **mutating `buffer_slot` in place** when the replacement
    /// payload fits the target slot (plan-02 §4.1). The common cases — fixed-width
    /// elements, and records/strings whose new payload is the same size or shorter
    /// (`need <= oldLen`) — overwrite the value bytes at the entry's `valueOffset`
    /// and patch `valueLength`: no allocation, no copy. A longer payload
    /// (`need > oldLen`) falls back to the rebuild path (remove + insert), which
    /// repoints `buffer_slot` at a fresh tight buffer. An out-of-range index raises
    /// the same `index out of range` error as the rebuild path. The caller
    /// guarantees the buffer is uniquely owned and not an active `FOR EACH`
    /// iterable. Returns the (possibly new) buffer pointer.
    pub(super) fn lower_list_set_in_place(
        &mut self,
        buffer_slot: usize,
        index_slot: usize,
        item_slot: usize,
        list_type: &str,
        element_type: &str,
    ) -> Result<ValueResult, String> {
        let scratch20 = self.temporary_vreg();
        let scratch21 = self.temporary_vreg();
        let scratch22 = self.temporary_vreg();
        let scratch23 = self.temporary_vreg();
        let scratch24 = self.temporary_vreg();
        let scratch25 = self.temporary_vreg();
        let scratch8 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch16 = self.temporary_vreg();
        let scratch17 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        if CollectionTypeLayout::from_type(list_type).is_none() {
            return Err(format!(
                "native code collection type '{list_type}' is not supported"
            ));
        }
        for register in [
            scratch20.as_str(),
            scratch21.as_str(),
            scratch22.as_str(),
            scratch23.as_str(),
            scratch24.as_str(),
            scratch25.as_str(),
        ] {
            self.mark_register_used(register);
        }
        let item = PayloadSlot {
            slot: item_slot,
            type_: element_type.to_string(),
        };
        // Byte size of the replacement payload.
        let need_slot = self.emit_payload_length_to_stack(&item, "set_inplace_need")?;
        let voffset_slot = self.allocate_stack_object("set_inplace_voff", 8);

        let valid = self.label("set_inplace_valid");
        let invalid = self.label("set_inplace_invalid");
        let rebuild = self.label("set_inplace_rebuild");
        let done = self.label("set_inplace_done");

        // Bounds check: 0 <= index < count.
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), index_slot));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch8,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::compare_immediate(&scratch10, "0"));
        self.emit(abi::branch_ge(&valid));
        self.emit(abi::branch(&invalid));
        self.emit(abi::label(&valid));
        self.emit(abi::compare_registers(&scratch10, &scratch11));
        self.emit(abi::branch_ge(&invalid));

        // entry = buffer + HEADER + index * ENTRY; read valueOffset / valueLength.
        self.emit(abi::move_immediate(
            &scratch16,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch17, &scratch10, &scratch16));
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch8,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(&scratch12, &scratch12, &scratch17));
        self.emit(abi::load_u64(
            &scratch13,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::store_u64(
            &scratch13,
            abi::stack_pointer(),
            voffset_slot,
        ));
        self.emit(abi::load_u64(
            &scratch9,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        )); // oldLen
        self.emit(abi::load_u64(&scratch14, abi::stack_pointer(), need_slot)); // need
                                                                               // Only a same-size replacement overwrites in place (offsets unchanged, no
                                                                               // gap). Any size change — grow OR shrink — rebuilds via removeAt + insert,
                                                                               // which produces a tight buffer; a shrink that overwrote in place would
                                                                               // leave dead space between payloads (plan-25-B).
        self.emit(abi::compare_registers(&scratch14, &scratch9));
        self.emit(abi::branch_ne(&rebuild));

        // --- Overwrite: same-size payload at valueOffset (valueLength unchanged). ---
        self.emit_copy_payload_to_collection(buffer_slot, need_slot, &item, voffset_slot)?;
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), index_slot));
        self.emit(abi::move_immediate(
            &scratch16,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch17, &scratch10, &scratch16));
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch8,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(&scratch12, &scratch12, &scratch17));
        self.emit(abi::load_u64(&scratch14, abi::stack_pointer(), need_slot));
        self.emit(abi::store_u64(
            &scratch14,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::branch(&done));

        // --- Rebuild (payload grew): remove + insert a fresh singleton list. ---
        self.emit(abi::label(&rebuild));
        // Snapshot the original buffer pointer before the rebuild allocates over
        // `buffer_slot`. This path abandons three blocks — the original buffer,
        // the singleton, and the removeAt intermediate — and, because the in-place
        // short-circuit bypassed the general-reassignment free, it owns freeing all
        // of them (bug-47). The singleton + removeAt intermediates are freed exactly
        // as the non-in-place `lower_collection_set` frees them (:314-318); the
        // original buffer additionally, since no scope-drop reclaims it here.
        let orig_slot = self.allocate_stack_object("set_inplace_orig", 8);
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
        self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), orig_slot));
        let singleton = self.lower_collection_values(
            list_type,
            vec![CollectionValueSlot {
                key: None,
                value: PayloadSlot {
                    slot: item_slot,
                    type_: element_type.to_string(),
                },
            }],
            "set rebuild singleton",
        )?;
        let singleton_slot = self.allocate_stack_object("set_inplace_singleton", 8);
        self.emit(abi::store_u64(
            &singleton.location,
            abi::stack_pointer(),
            singleton_slot,
        ));
        let removed =
            self.lower_list_remove_at(buffer_slot, index_slot, list_type, element_type)?;
        let removed_slot = self.allocate_stack_object("set_inplace_removed", 8);
        self.emit(abi::store_u64(
            &removed.location,
            abi::stack_pointer(),
            removed_slot,
        ));
        let rebuilt = self.lower_list_insert_collection(
            removed_slot,
            index_slot,
            singleton_slot,
            list_type,
            element_type,
        )?;
        self.emit(abi::store_u64(
            &rebuilt.location,
            abi::stack_pointer(),
            buffer_slot,
        ));
        // `rebuilt` (now in buffer_slot) is a fresh independent block holding copies
        // of all live data, so the three abandoned blocks are unreachable and
        // uniquely owned — free them. Each is a distinct block (insert_collection
        // allocates fresh; the non-in-place set frees the same singleton/removed
        // pair), so there is no double-free. The threaded result is discarded: the
        // function reloads the (unchanged) buffer_slot after `done`.
        let keep = self.allocate_register()?;
        self.emit(abi::load_u64(&keep, abi::stack_pointer(), buffer_slot));
        let threaded = ValueResult {
            type_: list_type.to_string(),
            location: keep,
            text: String::new(),
        };
        let threaded = self.free_intermediate_collection(singleton_slot, list_type, threaded)?;
        let threaded = self.free_intermediate_collection(removed_slot, list_type, threaded)?;
        let _ = self.free_intermediate_collection(orig_slot, list_type, threaded)?;
        self.emit(abi::branch(&done));

        self.emit(abi::label(&invalid));
        self.emit_index_out_of_range_return()?;
        self.emit(abi::label(&done));

        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), buffer_slot));
        Ok(ValueResult {
            type_: list_type.to_string(),
            location: result,
            text: format!("set in place {list_type} over {element_type}"),
        })
    }

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
    pub(super) fn lower_map_set_in_place(
        &mut self,
        map_slot: usize,
        key_slot: usize,
        value_slot: usize,
        map_type: &str,
        key_type: &str,
        value_type: &str,
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
            scratch20.as_str(),
            scratch21.as_str(),
            scratch22.as_str(),
            scratch23.as_str(),
            scratch24.as_str(),
            scratch25.as_str(),
        ] {
            self.mark_register_used(register);
        }
        let key_align = self.collection_payload_alignment(key_type);
        let value_align = self.collection_payload_alignment(value_type);
        let key_payload = PayloadSlot {
            slot: key_slot,
            type_: key_type.to_string(),
        };
        let value_payload = PayloadSlot {
            slot: value_slot,
            type_: value_type.to_string(),
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
            let collection = self.allocate_register()?;
            let key = self.allocate_register()?;
            let count = self.allocate_register()?;
            let index = self.allocate_register()?;
            let entry = self.allocate_register()?;
            let key_offset = self.allocate_register()?;
            let key_length = self.allocate_register()?;
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
                &key_offset,
                &entry,
                COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
            ));
            self.emit(abi::load_u64(
                &key_length,
                &entry,
                COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
            ));
            self.emit_collection_payload_match_branch(
                key_type,
                &collection,
                &key_offset,
                &key_length,
                &key,
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
        self.emit_copy_payload_to_collection(map_slot, val_len_slot, &value_payload, voff_slot)?;
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
        self.emit(abi::move_immediate(abi::ARG[1], "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&valloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&size_overflow));
        self.emit_error_code_return(ERR_OUT_OF_MEMORY_CODE, ERR_ALLOCATION_MESSAGE)?;
        self.emit(abi::label(&valloc_ok));
        self.emit(abi::store_u64(
            abi::RET[1],
            abi::stack_pointer(),
            new_buf_slot,
        ));
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
        self.emit_collection_data_pointer(&scratch17, &nb);
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
        self.emit_collection_data_pointer(&scratch20, &scratch8);
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
        self.emit_free_pre_grow_buffer(map_slot, map_type)?;
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
        self.emit(abi::move_immediate(abi::ARG[1], "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&size_overflow));
        self.emit_error_code_return(ERR_OUT_OF_MEMORY_CODE, ERR_ALLOCATION_MESSAGE)?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(
            abi::RET[1],
            abi::stack_pointer(),
            new_buf_slot,
        ));
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
        self.emit_collection_data_pointer(&scratch17, &nb);
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
        self.emit_collection_data_pointer(&scratch20, &scratch8);
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
        self.emit_free_pre_grow_buffer(map_slot, map_type)?;
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
        self.emit(abi::load_u64(abi::ARG[0], abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(
            abi::ARG[1],
            abi::ARG[0],
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::subtract_immediate(abi::ARG[1], abi::ARG[1], 1)); // new entry index
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
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), map_slot));
        Ok(ValueResult {
            type_: map_type.to_string(),
            location: result,
            text: format!("map set in place {map_type}"),
        })
    }

    pub(super) fn lower_list_remove_at(
        &mut self,
        base_slot: usize,
        index_slot: usize,
        list_type: &str,
        element_type: &str,
    ) -> Result<ValueResult, String> {
        let scratch20 = self.temporary_vreg();
        let scratch21 = self.temporary_vreg();
        let scratch22 = self.temporary_vreg();
        let scratch23 = self.temporary_vreg();
        let scratch24 = self.temporary_vreg();
        let scratch25 = self.temporary_vreg();
        let scratch8 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch16 = self.temporary_vreg();
        let scratch17 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        let scratch15 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let layout = CollectionTypeLayout::from_type(list_type)
            .ok_or_else(|| format!("native code collection type '{list_type}' is not supported"))?;
        for register in [
            scratch20.as_str(),
            scratch21.as_str(),
            scratch22.as_str(),
            scratch23.as_str(),
            scratch24.as_str(),
            scratch25.as_str(),
        ] {
            self.mark_register_used(register);
        }
        let result_slot = self.allocate_stack_object("list_remove_result", 8);
        let data_len_slot = self.allocate_stack_object("list_remove_data_len", 8);
        let valid_start = self.label("list_remove_valid_start");
        let alloc_ok = self.label("list_remove_alloc_ok");
        let invalid = self.label("list_remove_invalid");
        let done = self.label("list_remove_done");

        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), base_slot));
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), index_slot));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch8,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::compare_immediate(&scratch10, "0"));
        self.emit(abi::branch_ge(&valid_start));
        self.emit(abi::branch(&invalid));
        self.emit(abi::label(&valid_start));
        self.emit(abi::compare_registers(&scratch10, &scratch11));
        self.emit(abi::branch_ge(&invalid));
        self.emit(abi::move_immediate(
            &scratch16,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch17, &scratch10, &scratch16));
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch8,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(&scratch12, &scratch12, &scratch17));
        self.emit(abi::load_u64(
            &scratch14,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::load_u64(
            &scratch15,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::subtract_registers(&scratch15, &scratch14, &scratch15));
        // `arena_alloc` clobbers x15 in its block-grow path; persist the data
        // length so the header write below does not store a stale pointer.
        self.emit(abi::store_u64(
            &scratch15,
            abi::stack_pointer(),
            data_len_slot,
        ));
        self.emit(abi::subtract_immediate(&scratch13, &scratch11, 1));
        let size_overflow = self.label("list_remove_at_size_overflow");
        // Checked collection-size arithmetic (bug-147.7): (count-1) and dataLength
        // are runtime-derived, so guard count*ENTRY + HEADER + dataLen against
        // overflow.
        self.emit_checked_size_multiply(&scratch17, &scratch13, &scratch16, &size_overflow);
        self.emit_checked_size_add_immediate(
            abi::return_register(),
            &scratch17,
            COLLECTION_HEADER_SIZE,
            &size_overflow,
        );
        self.emit_checked_size_add(
            abi::return_register(),
            abi::return_register(),
            &scratch15,
            &size_overflow,
        );
        self.emit(abi::move_immediate(abi::ARG[1], "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&size_overflow));
        self.emit_error_code_return(ERR_OUT_OF_MEMORY_CODE, ERR_ALLOCATION_MESSAGE)?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(
            abi::RET[1],
            abi::stack_pointer(),
            result_slot,
        ));
        let nb = self.temporary_vreg();
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64(
            &scratch15,
            abi::stack_pointer(),
            data_len_slot,
        ));
        self.emit_write_list_header_from_registers(&layout, &nb, &scratch13, &scratch15);

        // Copy phase (no allocations, so registers hold across it). removeAt
        // punches a single contiguous hole in the data region — the removed
        // payload `[holeOffset, holeOffset + holeLen)`. Everything before it stays;
        // everything after shifts left by `holeLen`. So the payloads move as two
        // verbatim block copies (before-hole, after-hole) whatever order the data
        // region is in, the entry table copies as two verbatim spans (prefix
        // `[0..index)`, suffix `[index+1..count)`), and a single cheap pass fixes
        // each surviving `valueOffset` that sat past the hole — no per-payload copy
        // (plan-25-B B2). Testing each entry's own offset keeps it correct for a
        // list whose data is out of entry order after an insert/prepend/set.
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), base_slot)); // source
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), index_slot)); // index
        self.emit(abi::move_immediate(
            &scratch16,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        // removed entry = source + HEADER + index*ENTRY; grab the hole span.
        self.emit(abi::multiply_registers(&scratch11, &scratch10, &scratch16));
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch8,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(&scratch12, &scratch12, &scratch11));
        // scratch23 = holeOffset (removedValueOffset); scratch14 = holeLen.
        self.emit(abi::load_u64(
            &scratch23,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            &scratch14,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));

        // --- Entry table: two verbatim spans (no per-copy offset shift). ---
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch8,
            COLLECTION_HEADER_SIZE,
        )); // src.entry[0]
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), result_slot));
        self.emit(abi::add_immediate(&scratch17, &nb, COLLECTION_HEADER_SIZE)); // dst.entry[0]
                                                                                // Prefix [0..index): index*ENTRY bytes. Advances scratch17 -> dst.entry[index]
                                                                                // and scratch12 -> src.entry[index] (the removed entry).
        self.emit(abi::multiply_registers(&scratch15, &scratch10, &scratch16));
        self.emit_block_copy_advance(
            &scratch17,
            &scratch12,
            &scratch15,
            &scratch22,
            "list_remove_prefix_e",
        );
        // Suffix [index+1..count): skip the removed src entry, copy suffixCount*ENTRY.
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch12,
            COLLECTION_ENTRY_SIZE,
        )); // src.entry[index+1]
        self.emit(abi::load_u64(
            &scratch11,
            &scratch8,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::subtract_registers(&scratch11, &scratch11, &scratch10));
        self.emit(abi::subtract_immediate(&scratch11, &scratch11, 1)); // suffixCount
        self.emit(abi::multiply_registers(&scratch15, &scratch11, &scratch16));
        self.emit_block_copy_advance(
            &scratch17,
            &scratch12,
            &scratch15,
            &scratch22,
            "list_remove_suffix_e",
        );

        // --- Data region: two verbatim blocks around the hole. ---
        self.emit_collection_data_pointer(&scratch20, &scratch8); // src data base (capacity-based)
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), result_slot));
        self.emit_collection_data_pointer(&scratch21, &nb); // dst data base (tight)
                                                            // Before-hole [0, holeOffset): advances scratch21 -> dst.data[holeOffset]
                                                            // and scratch20 -> src.data[holeOffset].
        self.emit(abi::move_register(&scratch15, &scratch23)); // holeOffset (copy consumes it)
        self.emit_block_copy_advance(
            &scratch21,
            &scratch20,
            &scratch15,
            &scratch22,
            "list_remove_prefix_d",
        );
        // After-hole [holeOffset+holeLen, dataLength) -> dst.data[holeOffset]. Skip
        // the removed span in the source, then copy the tail.
        self.emit(abi::add_registers(&scratch20, &scratch20, &scratch14));
        self.emit(abi::load_u64(
            &scratch15,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::subtract_registers(&scratch15, &scratch15, &scratch23));
        self.emit(abi::subtract_registers(&scratch15, &scratch15, &scratch14)); // tailLen
        self.emit_block_copy_advance(
            &scratch21,
            &scratch20,
            &scratch15,
            &scratch22,
            "list_remove_suffix_d",
        );

        // --- Fix up each surviving entry whose payload sat past the hole. ---
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), result_slot));
        self.emit(abi::add_immediate(&scratch17, &nb, COLLECTION_HEADER_SIZE)); // dst.entry[0]
        self.emit(abi::load_u64(
            &scratch11,
            &scratch8,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::subtract_immediate(&scratch11, &scratch11, 1)); // survivor count
        self.emit_offset_compaction_fixup(
            &scratch17,
            &scratch11,
            &scratch23,
            &scratch14,
            "list_remove_fix",
        );
        self.emit(abi::branch(&done));
        self.emit(abi::label(&invalid));
        self.emit_index_out_of_range_return()?;
        self.emit(abi::label(&done));

        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: list_type.to_string(),
            location: result,
            text: format!("removeAt({list_type}, Integer) over {element_type}"),
        })
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

    /// Allocate an empty output list of `output_type` pre-sized to the source
    /// collection at `source_slot`: `capacity = count(source)` lookup slots and
    /// `dataCapacity = dataLength(source)` data bytes, with `count = 0` and
    /// `dataLength = 0` (plan-25-B B2). transform/filter fill it with
    /// `lower_list_append_in_place`, which then writes each element into the
    /// reserved headroom without a single entry-table regrow (transform emits
    /// exactly `count(source)` entries, filter a subset) — and, for filter (whose
    /// output is a subset of its input) and any transform whose outputs are no
    /// larger than its inputs, without a data regrow either. A larger transform
    /// output still regrows its data region correctly (the reservation is a lower
    /// bound, never a cap). The reserved headroom is unobservable and tightened
    /// away when the value is copied out (shrink-to-fit), so the produced list is
    /// value-identical to the geometric-growth build it replaces.
    pub(super) fn lower_reserved_list(
        &mut self,
        output_type: &str,
        source_slot: usize,
    ) -> Result<ValueResult, String> {
        let layout = CollectionTypeLayout::from_type(output_type).ok_or_else(|| {
            format!("native code collection type '{output_type}' is not supported")
        })?;
        let s8 = self.temporary_vreg();
        let s9 = self.temporary_vreg();
        let s10 = self.temporary_vreg();
        let s11 = self.temporary_vreg();
        let s12 = self.temporary_vreg();
        let zero = self.temporary_vreg();
        let result_slot = self.allocate_stack_object("reserved_list_result", 8);
        let cap_slot = self.allocate_stack_object("reserved_list_cap", 8);
        let dcap_slot = self.allocate_stack_object("reserved_list_dcap", 8);
        let alloc_ok = self.label("reserved_list_alloc_ok");
        // capacity = count(source); dataCapacity = dataLength(source).
        self.emit(abi::load_u64(&s8, abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64(&s9, &s8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(&s10, &s8, COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::store_u64(&s9, abi::stack_pointer(), cap_slot));
        self.emit(abi::store_u64(&s10, abi::stack_pointer(), dcap_slot));
        // alloc size = HEADER + capacity * ENTRY + dataCapacity.
        let size_overflow = self.label("reserved_list_size_overflow");
        self.emit(abi::move_immediate(
            &s11,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        // Checked collection-size arithmetic (bug-147.7): capacity/dataCapacity are
        // runtime-derived, so guard count*ENTRY + HEADER + dataCap against overflow.
        self.emit_checked_size_multiply(&s12, &s9, &s11, &size_overflow);
        self.emit_checked_size_add_immediate(
            abi::return_register(),
            &s12,
            COLLECTION_HEADER_SIZE,
            &size_overflow,
        );
        self.emit_checked_size_add(
            abi::return_register(),
            abi::return_register(),
            &s10,
            &size_overflow,
        );
        self.emit(abi::move_immediate(abi::ARG[1], "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&size_overflow));
        self.emit_error_code_return(ERR_OUT_OF_MEMORY_CODE, ERR_ALLOCATION_MESSAGE)?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(
            abi::RET[1],
            abi::stack_pointer(),
            result_slot,
        ));
        let nb = self.temporary_vreg();
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), result_slot));
        // Header: count = 0, capacity, dataLength = 0, dataCapacity.
        self.emit(abi::move_immediate(&zero, "Integer", "0"));
        self.emit(abi::load_u64(&s9, abi::stack_pointer(), cap_slot));
        self.emit(abi::load_u64(&s10, abi::stack_pointer(), dcap_slot));
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), result_slot));
        self.emit_write_collection_header_full(&layout, &nb, &zero, &s9, &zero, &s10);
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: output_type.to_string(),
            location: result,
            text: format!("reserved list {output_type}"),
        })
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

    pub(super) fn lower_map_concat(
        &mut self,
        left_slot: usize,
        right_slot: usize,
        map_type: &str,
    ) -> Result<ValueResult, String> {
        let scratch20 = self.temporary_vreg();
        let scratch21 = self.temporary_vreg();
        let scratch22 = self.temporary_vreg();
        let scratch23 = self.temporary_vreg();
        let scratch24 = self.temporary_vreg();
        let scratch25 = self.temporary_vreg();
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let scratch15 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        let scratch16 = self.temporary_vreg();
        let scratch17 = self.temporary_vreg();
        let layout = CollectionTypeLayout::from_type(map_type)
            .ok_or_else(|| format!("native code collection type '{map_type}' is not supported"))?;
        let key_payload_align = collection_payload_alignment_for_code(layout.key_type_code);
        let value_payload_align = collection_payload_alignment_for_code(layout.value_type_code);
        let map_max_align = key_payload_align.max(value_payload_align);
        for register in [
            scratch20.as_str(),
            scratch21.as_str(),
            scratch22.as_str(),
            scratch23.as_str(),
            scratch24.as_str(),
            scratch25.as_str(),
        ] {
            self.mark_register_used(register);
        }
        let result_slot = self.allocate_stack_object("map_concat_result", 8);
        let alloc_ok = self.label("map_concat_alloc_ok");

        // Offset-stable merge (plan-01 §4.1): copy A's and B's data regions
        // verbatim — B placed at `align(dataLen_A, map_max_align)` so its packed
        // payloads keep their alignment relative to the new base — then concat the
        // lookup tables, shifting every B key/value offset by that same boundary.
        // The B boundary doubles as the per-entry offset shift.
        //
        // Size: HEADER + (count_A+count_B)*ENTRY + (align(dataLen_A)+dataLen_B).
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), left_slot));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), right_slot));
        self.emit(abi::load_u64(
            &scratch10,
            &scratch8,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch9,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::add_registers(&scratch12, &scratch10, &scratch11));
        self.emit(abi::load_u64(
            &scratch13,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit_align_offset_register(&scratch13, map_max_align, &scratch15);
        self.emit(abi::load_u64(
            &scratch14,
            &scratch9,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::add_registers(&scratch14, &scratch13, &scratch14));
        let size_overflow = self.label("map_concat_size_overflow");
        self.emit(abi::move_immediate(
            &scratch15,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        // Checked collection-size arithmetic (bug-147.7): the total count and both
        // data lengths come from live map headers, so guard count*ENTRY + HEADER +
        // dataLen against overflow before allocating.
        self.emit_checked_size_multiply(&scratch16, &scratch12, &scratch15, &size_overflow);
        self.emit_checked_size_add_immediate(
            abi::return_register(),
            &scratch16,
            COLLECTION_HEADER_SIZE,
            &size_overflow,
        );
        self.emit_checked_size_add(
            abi::return_register(),
            abi::return_register(),
            &scratch14,
            &size_overflow,
        );
        // Reserve the map hash bucket region (x12 = total count = capacity).
        self.emit_reserve_map_buckets(true, &scratch12, abi::return_register(), &scratch15);
        self.emit(abi::move_immediate(abi::ARG[1], "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&size_overflow));
        self.emit_error_code_return(ERR_OUT_OF_MEMORY_CODE, ERR_ALLOCATION_MESSAGE)?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(
            abi::RET[1],
            abi::stack_pointer(),
            result_slot,
        ));
        let nb = self.temporary_vreg();
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), result_slot));

        // Header: recompute total count / total data length from the pointer slots
        // (the pre-alloc registers do not survive `arena_alloc`).
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), left_slot));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), right_slot));
        self.emit(abi::load_u64(
            &scratch10,
            &scratch8,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch9,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::add_registers(&scratch12, &scratch10, &scratch11));
        self.emit(abi::load_u64(
            &scratch13,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit_align_offset_register(&scratch13, map_max_align, &scratch15);
        self.emit(abi::load_u64(
            &scratch14,
            &scratch9,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::add_registers(&scratch14, &scratch13, &scratch14));
        self.emit_write_list_header_from_registers(&layout, &nb, &scratch12, &scratch14);

        // --- Data region: A verbatim at base, B verbatim at align(dataLen_A). ---
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), result_slot));
        self.emit_collection_data_pointer(&scratch17, &nb); // x17 = dst data base (stable)
        self.emit(abi::move_register(&scratch23, &scratch17)); // moving copy dst
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), left_slot));
        self.emit_collection_data_pointer(&scratch20, &scratch8); // A data base
        self.emit(abi::load_u64(
            &scratch14,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit_block_copy_advance(
            &scratch23,
            &scratch20,
            &scratch14,
            &scratch22,
            "map_concat_dataA",
        );
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), left_slot));
        self.emit(abi::load_u64(
            &scratch13,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit_align_offset_register(&scratch13, map_max_align, &scratch22); // alignedA
        self.emit(abi::add_registers(&scratch23, &scratch17, &scratch13)); // B dest = base + alignedA
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), right_slot));
        self.emit_collection_data_pointer(&scratch20, &scratch9); // B data base
        self.emit(abi::load_u64(
            &scratch15,
            &scratch9,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit_block_copy_advance(
            &scratch23,
            &scratch20,
            &scratch15,
            &scratch22,
            "map_concat_dataB",
        );

        // --- Lookup table: A entries verbatim, then B entries shifted. ---
        self.emit(abi::move_immediate(
            &scratch16,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), result_slot));
        self.emit(abi::add_immediate(&scratch17, &nb, COLLECTION_HEADER_SIZE)); // dst table cursor
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), left_slot));
        self.emit(abi::add_immediate(
            &scratch20,
            &scratch8,
            COLLECTION_HEADER_SIZE,
        )); // A table cursor
        self.emit(abi::load_u64(
            &scratch10,
            &scratch8,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::multiply_registers(&scratch21, &scratch10, &scratch16)); // count_A * ENTRY
        self.emit_block_copy_advance(
            &scratch17,
            &scratch20,
            &scratch21,
            &scratch22,
            "map_concat_tableA",
        );

        // B entries: keyOffset and valueOffset each += align(dataLen_A).
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), right_slot));
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch9,
            COLLECTION_HEADER_SIZE,
        )); // B table cursor
        self.emit(abi::load_u64(
            &scratch11,
            &scratch9,
            COLLECTION_OFFSET_COUNT,
        )); // remaining
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), left_slot));
        self.emit(abi::load_u64(
            &scratch14,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit_align_offset_register(&scratch14, map_max_align, &scratch22); // shift = alignedA
        let copy_loop = self.label("map_concat_b_loop");
        let copy_done = self.label("map_concat_b_done");
        self.emit(abi::label(&copy_loop));
        self.emit(abi::compare_immediate(&scratch11, "0"));
        self.emit(abi::branch_eq(&copy_done));
        self.emit(abi::move_immediate(
            &scratch22,
            "Byte",
            &COLLECTION_ENTRY_FLAG_USED.to_string(),
        ));
        self.emit(abi::store_u8(
            &scratch22,
            &scratch17,
            COLLECTION_ENTRY_OFFSET_FLAGS,
        ));
        self.emit(abi::load_u64(
            &scratch22,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::add_registers(&scratch22, &scratch22, &scratch14));
        self.emit(abi::store_u64(
            &scratch22,
            &scratch17,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::load_u64(
            &scratch22,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::store_u64(
            &scratch22,
            &scratch17,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::load_u64(
            &scratch22,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::add_registers(&scratch22, &scratch22, &scratch14));
        self.emit(abi::store_u64(
            &scratch22,
            &scratch17,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            &scratch22,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::store_u64(
            &scratch22,
            &scratch17,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_immediate(
            &scratch17,
            &scratch17,
            COLLECTION_ENTRY_SIZE,
        ));
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch12,
            COLLECTION_ENTRY_SIZE,
        ));
        self.emit(abi::subtract_immediate(&scratch11, &scratch11, 1));
        self.emit(abi::branch(&copy_loop));
        self.emit(abi::label(&copy_done));

        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: map_type.to_string(),
            location: result,
            text: format!("map concat {map_type}"),
        })
    }

    pub(super) fn lower_map_remove_key(
        &mut self,
        map_slot: usize,
        key_slot: usize,
        map_type: &str,
        key_type: &str,
    ) -> Result<ValueResult, String> {
        let scratch20 = self.temporary_vreg();
        let scratch21 = self.temporary_vreg();
        let scratch22 = self.temporary_vreg();
        let scratch23 = self.temporary_vreg();
        let scratch24 = self.temporary_vreg();
        let scratch25 = self.temporary_vreg();
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        let scratch15 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let scratch16 = self.temporary_vreg();
        let scratch17 = self.temporary_vreg();
        let layout = CollectionTypeLayout::from_type(map_type)
            .ok_or_else(|| format!("native code collection type '{map_type}' is not supported"))?;
        let key_payload_align = collection_payload_alignment_for_code(layout.key_type_code);
        let value_payload_align = collection_payload_alignment_for_code(layout.value_type_code);
        for register in [
            scratch20.as_str(),
            scratch21.as_str(),
            scratch22.as_str(),
            scratch23.as_str(),
            scratch24.as_str(),
            scratch25.as_str(),
        ] {
            self.mark_register_used(register);
        }
        let result_slot = self.allocate_stack_object("map_remove_result", 8);
        let count_slot = self.allocate_stack_object("map_remove_count", 8);
        let data_len_slot = self.allocate_stack_object("map_remove_data_len", 8);
        let scan_loop = self.label("map_remove_scan_loop");
        let scan_keep = self.label("map_remove_scan_keep");
        let scan_next = self.label("map_remove_scan_next");
        let scan_done = self.label("map_remove_scan_done");
        let alloc_ok = self.label("map_remove_alloc_ok");
        let copy_loop = self.label("map_remove_copy_loop");
        let copy_keep = self.label("map_remove_copy_keep");
        let copy_next = self.label("map_remove_copy_next");
        let copy_done = self.label("map_remove_copy_done");

        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), key_slot));
        self.emit(abi::load_u64(
            &scratch10,
            &scratch8,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::move_immediate(&scratch11, "Integer", "0"));
        self.emit(abi::move_immediate(&scratch14, "Integer", "0"));
        self.emit(abi::move_immediate(&scratch15, "Integer", "0"));
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch8,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::label(&scan_loop));
        self.emit(abi::compare_registers(&scratch11, &scratch10));
        self.emit(abi::branch_ge(&scan_done));
        self.emit(abi::load_u64(
            &scratch13,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::load_u64(
            &scratch16,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit_collection_payload_matches_value_branch(
            key_type, &scratch8, &scratch13, &scratch16, &scratch9, &scan_next, &scan_keep,
        )?;
        self.emit(abi::label(&scan_keep));
        self.emit(abi::add_immediate(&scratch14, &scratch14, 1));
        // Accumulate the retained data length with the same per-payload
        // alignment the copy phase applies, so the precomputed allocation
        // matches the bytes actually written.
        self.emit_align_offset_register(&scratch15, key_payload_align, &scratch16);
        self.emit(abi::load_u64(
            &scratch16,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::add_registers(&scratch15, &scratch15, &scratch16));
        self.emit_align_offset_register(&scratch15, value_payload_align, &scratch16);
        self.emit(abi::load_u64(
            &scratch17,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers(&scratch15, &scratch15, &scratch17));
        self.emit(abi::label(&scan_next));
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch12,
            COLLECTION_ENTRY_SIZE,
        ));
        self.emit(abi::add_immediate(&scratch11, &scratch11, 1));
        self.emit(abi::branch(&scan_loop));
        self.emit(abi::label(&scan_done));
        self.emit(abi::move_immediate(
            &scratch16,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch17, &scratch14, &scratch16));
        self.emit(abi::add_immediate(
            abi::return_register(),
            &scratch17,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(
            abi::return_register(),
            abi::return_register(),
            &scratch15,
        ));
        // `arena_alloc` clobbers both x14 and x15 in its block-grow path; persist
        // the retained count and data length so the header write below does not
        // store stale pointers.
        self.emit(abi::store_u64(&scratch14, abi::stack_pointer(), count_slot));
        self.emit(abi::store_u64(
            &scratch15,
            abi::stack_pointer(),
            data_len_slot,
        ));
        // Reserve the map hash bucket region (x14 = remaining count = capacity).
        self.emit_reserve_map_buckets(true, &scratch14, abi::return_register(), &scratch16);
        self.emit(abi::move_immediate(abi::ARG[1], "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(
            abi::RET[1],
            abi::stack_pointer(),
            result_slot,
        ));
        let nb = self.temporary_vreg();
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64(&scratch14, abi::stack_pointer(), count_slot));
        self.emit(abi::load_u64(
            &scratch15,
            abi::stack_pointer(),
            data_len_slot,
        ));
        self.emit_write_list_header_from_registers(&layout, &nb, &scratch14, &scratch15);

        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), key_slot));
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64(
            &scratch10,
            &scratch8,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::move_immediate(&scratch11, "Integer", "0"));
        self.emit(abi::move_immediate(&scratch13, "Integer", "0"));
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch8,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_immediate(&scratch17, &nb, COLLECTION_HEADER_SIZE));
        self.emit_collection_data_pointer(&scratch20, &scratch8);
        self.emit(abi::load_u64(&scratch14, &nb, COLLECTION_OFFSET_COUNT));
        self.emit(abi::move_immediate(
            &scratch16,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch21, &scratch14, &scratch16));
        self.emit(abi::add_registers(&scratch21, &scratch17, &scratch21));
        self.emit(abi::label(&copy_loop));
        self.emit(abi::compare_registers(&scratch11, &scratch10));
        self.emit(abi::branch_ge(&copy_done));
        self.emit(abi::load_u64(
            &scratch14,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::load_u64(
            &scratch15,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit_collection_payload_matches_value_branch(
            key_type, &scratch8, &scratch14, &scratch15, &scratch9, &copy_next, &copy_keep,
        )?;
        self.emit(abi::label(&copy_keep));
        self.emit_copy_one_map_entry(
            &scratch12,
            &scratch20,
            &scratch17,
            &scratch21,
            &scratch13,
            key_payload_align,
            value_payload_align,
        );
        self.emit(abi::label(&copy_next));
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch12,
            COLLECTION_ENTRY_SIZE,
        ));
        self.emit(abi::add_immediate(&scratch11, &scratch11, 1));
        self.emit(abi::branch(&copy_loop));
        self.emit(abi::label(&copy_done));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: map_type.to_string(),
            location: result,
            text: format!("removeKey({map_type}, {key_type})"),
        })
    }

    pub(super) fn emit_copy_one_map_entry(
        &mut self,
        source_entry: &str,
        source_data: &str,
        dest_entry: &str,
        dest_data: &str,
        dest_data_offset: &str,
        key_align: usize,
        value_align: usize,
    ) {
        let scratch22 = self.temporary_vreg();
        let scratch23 = self.temporary_vreg();
        let scratch24 = self.temporary_vreg();
        let scratch25 = self.temporary_vreg();
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
        // Align the destination cursor to the key payload alignment before
        // recording its offset, matching the packing used when the map was
        // first built. Idempotent when the cursor is already aligned.
        self.emit_align_offset_register(dest_data_offset, key_align, &scratch22);
        self.emit(abi::load_u64(
            &scratch22,
            source_entry,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::load_u64(
            &scratch23,
            source_entry,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::store_u64(
            dest_data_offset,
            dest_entry,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::store_u64(
            &scratch23,
            dest_entry,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::add_registers(&scratch24, source_data, &scratch22));
        self.emit(abi::add_registers(&scratch25, dest_data, dest_data_offset));
        self.emit_block_copy_advance(
            &scratch25,
            &scratch24,
            &scratch23,
            &scratch22,
            "map_entry_key_copy",
        );
        self.emit(abi::load_u64(
            &scratch23,
            dest_entry,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::add_registers(
            dest_data_offset,
            dest_data_offset,
            &scratch23,
        ));

        // Align the destination cursor to the value payload alignment before
        // recording its offset.
        self.emit_align_offset_register(dest_data_offset, value_align, &scratch22);
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
            "map_entry_value_copy",
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
            dest_entry,
            dest_entry,
            COLLECTION_ENTRY_SIZE,
        ));
    }
}
