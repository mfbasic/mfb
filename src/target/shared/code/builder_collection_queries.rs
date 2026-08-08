use super::*;

impl CodeBuilder<'_> {
    /// `collections::get`/`getOr` extract an element as an alias into the
    /// container's data region for inline composite / nested-collection payloads
    /// (`emit_load_collection_payload`). By value semantics `get` returns an
    /// **owned** value the caller may bind, store, and free, so copy such a
    /// alias into a standalone arena block (scalars are by-value and `String`
    /// is already materialized fresh, so they pass through). plan-02 Phase 8.
    pub(super) fn materialize_owned_element(
        &mut self,
        result: ValueResult,
    ) -> Result<ValueResult, String> {
        if self.is_freeable_flat_value(&result.type_) && result.type_ != "String" {
            let copied = self.copy_flat_block(&result.type_, &result.location)?;
            return Ok(ValueResult {
                type_: result.type_,
                location: copied.render(),
                text: result.text,
            });
        }
        Ok(result)
    }

    pub(super) fn lower_collection_get(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let collection = self.lower_value(&args[0])?;
        let collection_slot = self.allocate_stack_object("get_collection", 8);
        self.emit(abi::store_u64(
            &collection.location,
            abi::stack_pointer(),
            collection_slot,
        ));

        let key = self.lower_value(&args[1])?;
        let key_slot = self.allocate_stack_object("get_key", 8);
        // A `d`-native float map key stores via `str d`, bit-identical to the
        // `str x` a later bitwise key compare reads (plan-01 float-dnative).
        self.store_value_at(&key, abi::stack_pointer(), key_slot);

        if let Some(element_type) = list_element_type(&collection.type_) {
            if key.type_ != "Integer" {
                return Err(format!(
                    "native collection get list index must be Integer, got {}",
                    key.type_
                ));
            }
            let result =
                self.lower_list_get(collection_slot, key_slot, &collection.type_, &element_type)?;
            return self.materialize_owned_element(result);
        }

        if let Some((key_type, value_type)) = map_type_parts(&collection.type_) {
            if key.type_ != key_type {
                return Err(format!(
                    "native collection get map key must be {}, got {}",
                    key_type, key.type_
                ));
            }
            let result = self.lower_map_get(
                collection_slot,
                key_slot,
                &collection.type_,
                &key_type,
                &value_type,
            )?;
            return self.materialize_owned_element(result);
        }

        Err(format!(
            "native collection get does not accept {}",
            collection.type_
        ))
    }

    pub(super) fn lower_collection_contains(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let collection = self.lower_value(&args[0])?;
        let collection_slot = self.allocate_stack_object("contains_collection", 8);
        self.emit(abi::store_u64(
            &collection.location,
            abi::stack_pointer(),
            collection_slot,
        ));

        let item = self.lower_value(&args[1])?;
        let item_slot = self.allocate_stack_object("contains_item", 8);
        // A `d`-native float item stores via `str d`, bit-identical to the
        // `str x` the element compare reads back (plan-01 float-dnative).
        self.store_value_at(&item, abi::stack_pointer(), item_slot);

        // A `Set OF T` membership test is the Map-shaped hash probe / linear scan
        // over the element (= entry key), shared with `hasKey` (plan-63-B). Decided
        // on the *lowered* type so a nested-call first argument (`contains(union(a,
        // b), x)`, whose static type is unknown pre-lowering) still routes here.
        if let Some(element_type) = set_element_type(&collection.type_) {
            return self.emit_key_membership(
                collection_slot,
                item_slot,
                &element_type,
                "contains",
                &collection.type_,
            );
        }

        let Some(element_type) = list_element_type(&collection.type_) else {
            return Err(format!(
                "native collection contains does not accept {}",
                collection.type_
            ));
        };
        if item.type_ != element_type {
            return Err(format!(
                "native collection contains item must be {}, got {}",
                element_type, item.type_
            ));
        }

        self.reset_temporary_registers();
        let collection_register = self.allocate_register()?;
        let item_register = self.allocate_register()?;
        let count = self.allocate_register()?;
        let index = self.allocate_register()?;
        let entry = self.allocate_register()?;
        let value_offset = self.allocate_register()?;
        let value_length = self.allocate_register()?;
        let result = self.allocate_register()?;
        let loop_label = self.label("contains_loop");
        let found = self.label("contains_found");
        let next = self.label("contains_next");
        let not_found = self.label("contains_not_found");
        let done = self.label("contains_done");

        self.emit(abi::load_u64(
            &collection_register,
            abi::stack_pointer(),
            collection_slot,
        ));
        self.emit(abi::load_u64(
            &item_register,
            abi::stack_pointer(),
            item_slot,
        ));
        self.emit(abi::load_u64(
            &count,
            &collection_register,
            COLLECTION_OFFSET_COUNT,
        ));
        // kind 2 walks the data region: `entry` carries a byte OFFSET from the
        // data base rather than an entry pointer, and the span is derivable from
        // the cursor and the constant payload size (plan-57-D).
        let contains_payload = kind2_payload_size(&element_type);
        self.emit(abi::move_immediate(&index, "Integer", "0"));
        if contains_payload.is_some() {
            self.emit(abi::move_immediate(&entry, "Integer", "0"));
        } else {
            self.emit(abi::add_immediate(
                &entry,
                &collection_register,
                COLLECTION_HEADER_SIZE,
            ));
        }

        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_registers(&index, &count));
        self.emit(abi::branch_ge(&not_found));
        if let Some(payload) = contains_payload {
            self.emit(abi::move_register(&value_offset, &entry));
            self.emit(abi::move_immediate(
                &value_length,
                "Integer",
                &payload.to_string(),
            ));
        } else {
            self.emit(abi::load_u64(
                &value_offset,
                &entry,
                COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
            ));
            self.emit(abi::load_u64(
                &value_length,
                &entry,
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ));
        }
        self.emit_collection_payload_match_branch(
            &element_type,
            &element_type,
            &collection_register,
            &value_offset,
            &value_length,
            &item_register,
            &found,
            &next,
        )?;

        self.emit(abi::label(&found));
        self.emit(abi::move_immediate(&result, "Boolean", "true"));
        self.emit(abi::branch(&done));

        self.emit(abi::label(&next));
        self.emit(abi::add_immediate(
            &entry,
            &entry,
            contains_payload.unwrap_or(COLLECTION_ENTRY_SIZE),
        ));
        self.emit(abi::add_immediate(&index, &index, 1));
        self.emit(abi::branch(&loop_label));

        self.emit(abi::label(&not_found));
        self.emit(abi::move_immediate(&result, "Boolean", "false"));
        self.emit(abi::label(&done));

        Ok(ValueResult {
            type_: "Boolean".to_string(),
            location: result.render(),
            text: format!("contains({}, {})", collection.type_, element_type),
        })
    }

    pub(super) fn lower_collection_get_or(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let collection = self.lower_value(&args[0])?;
        let collection_slot = self.allocate_stack_object("get_or_collection", 8);
        self.emit(abi::store_u64(
            &collection.location,
            abi::stack_pointer(),
            collection_slot,
        ));

        let key = self.lower_value(&args[1])?;
        let key_slot = self.allocate_stack_object("get_or_key", 8);
        // `d`-native float key/default store via `str d` (plan-01 float-dnative).
        self.store_value_at(&key, abi::stack_pointer(), key_slot);

        let default = self.lower_value(&args[2])?;
        let default_slot = self.allocate_stack_object("get_or_default", 8);
        self.store_value_at(&default, abi::stack_pointer(), default_slot);

        if let Some(element_type) = list_element_type(&collection.type_) {
            if key.type_ != "Integer" {
                return Err(format!(
                    "native collection getOr list index must be Integer, got {}",
                    key.type_
                ));
            }
            if default.type_ != element_type {
                return Err(format!(
                    "native collection getOr default must be {}, got {}",
                    element_type, default.type_
                ));
            }
            let result = self.lower_list_get_or(
                collection_slot,
                key_slot,
                default_slot,
                &collection.type_,
                &element_type,
            )?;
            return self.materialize_owned_element(result);
        }

        if let Some((key_type, value_type)) = map_type_parts(&collection.type_) {
            if key.type_ != key_type {
                return Err(format!(
                    "native collection getOr map key must be {}, got {}",
                    key_type, key.type_
                ));
            }
            if default.type_ != value_type {
                return Err(format!(
                    "native collection getOr default must be {}, got {}",
                    value_type, default.type_
                ));
            }
            let result = self.lower_map_get_or(
                collection_slot,
                key_slot,
                default_slot,
                &collection.type_,
                &key_type,
                &value_type,
            )?;
            return self.materialize_owned_element(result);
        }

        Err(format!(
            "native collection getOr does not accept {}",
            collection.type_
        ))
    }

    pub(super) fn lower_collection_has_key(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let collection = self.lower_value(&args[0])?;
        let collection_slot = self.allocate_stack_object("has_key_collection", 8);
        self.emit(abi::store_u64(
            &collection.location,
            abi::stack_pointer(),
            collection_slot,
        ));
        let key = self.lower_value(&args[1])?;
        let key_slot = self.allocate_stack_object("has_key_key", 8);
        // `d`-native float key stores via `str d` (plan-01 float-dnative).
        self.store_value_at(&key, abi::stack_pointer(), key_slot);

        // A Map keys on its key type; a Set (reached via `contains`, plan-63-B)
        // keys on its element type — both are the bytes the probe compares.
        let Some(key_type) = map_type_parts(&collection.type_)
            .map(|(key, _)| key.to_string())
            .or_else(|| set_element_type(&collection.type_))
        else {
            return Err(format!(
                "native collection hasKey/contains does not accept {}",
                collection.type_
            ));
        };
        let key_type = key_type.as_str();
        if key.type_ != key_type {
            return Err(format!(
                "native collection hasKey key must be {}, got {}",
                key_type, key.type_
            ));
        }
        return self.emit_key_membership(
            collection_slot,
            key_slot,
            key_type,
            "has_key",
            &collection.type_,
        );
    }

    /// The shared Map/Set membership test: probe the FNV-1a bucket index for a
    /// probe-eligible key type, else linear-scan the entry keys, yielding a
    /// `Boolean`. Both `collections::hasKey` (Map) and the Set overload of
    /// `collections::contains` (plan-63-B) lower through here — a Set's element is
    /// its entry key, so the byte-compare is identical. `collection_slot` holds the
    /// collection pointer and `key_slot` the needle, both already spilled.
    pub(super) fn emit_key_membership(
        &mut self,
        collection_slot: usize,
        key_slot: usize,
        key_type: &str,
        label_prefix: &str,
        collection_type: &str,
    ) -> Result<ValueResult, String> {
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();

        if Self::map_key_probe_eligible(key_type) {
            self.reset_temporary_registers();
            let not_found = self.label(&format!("{label_prefix}_not_found"));
            let done = self.label(&format!("{label_prefix}_done"));
            let _ = self.emit_map_probe(collection_slot, key_slot, key_type, &not_found)?;
            let result = self.allocate_register()?;
            self.emit(abi::move_immediate(&result, "Boolean", "true"));
            self.emit(abi::branch(&done));
            self.emit(abi::label(&not_found));
            self.emit(abi::move_immediate(&result, "Boolean", "false"));
            self.emit(abi::label(&done));
            return Ok(ValueResult {
                type_: "Boolean".to_string(),
                location: result.render(),
                text: format!("{label_prefix}({collection_type}) [hash]"),
            });
        }

        self.reset_temporary_registers();
        let result = self.allocate_register()?;
        let loop_label = self.label(&format!("{label_prefix}_loop"));
        let found = self.label(&format!("{label_prefix}_found"));
        let next = self.label(&format!("{label_prefix}_next"));
        let not_found = self.label(&format!("{label_prefix}_not_found"));
        let done = self.label(&format!("{label_prefix}_done"));

        self.emit(abi::load_u64(
            &scratch8,
            abi::stack_pointer(),
            collection_slot,
        ));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), key_slot));
        self.emit_entry_scan_setup(
            &scratch8,
            &scratch10,
            &scratch11,
            &scratch12,
            &scratch13,
            &scratch14,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
            &loop_label,
            &not_found,
        );
        self.emit_collection_payload_matches_value_branch(
            key_type, "", &scratch8, &scratch13, &scratch14, &scratch9, &found, &next,
        )?;
        self.emit(abi::label(&found));
        self.emit(abi::move_immediate(&result, "Boolean", "true"));
        self.emit(abi::branch(&done));
        self.emit_entry_scan_advance(&scratch12, &scratch11, &next, &loop_label);
        self.emit(abi::label(&not_found));
        self.emit(abi::move_immediate(&result, "Boolean", "false"));
        self.emit(abi::label(&done));

        Ok(ValueResult {
            type_: "Boolean".to_string(),
            location: result.render(),
            text: format!("{label_prefix}({collection_type}, {key_type})"),
        })
    }

    pub(super) fn lower_collection_keys(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let collection = self.lower_value(&args[0])?;
        let Some((key_type, _)) = map_type_parts(&collection.type_) else {
            return Err(format!(
                "native collection keys does not accept {}",
                collection.type_
            ));
        };
        self.lower_map_projection(&collection, &key_type, true)
    }

    pub(super) fn lower_collection_values_builtin(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let collection = self.lower_value(&args[0])?;
        let Some((_, value_type)) = map_type_parts(&collection.type_) else {
            return Err(format!(
                "native collection values does not accept {}",
                collection.type_
            ));
        };
        self.lower_map_projection(&collection, &value_type, false)
    }

    pub(super) fn lower_map_projection(
        &mut self,
        collection: &ValueResult,
        element_type: &str,
        project_key: bool,
    ) -> Result<ValueResult, String> {
        // The projected list's own entry stride: zero when the element type is
        // fixed-width, so the result is built entry-free (plan-57-D).
        let proj_stride = list_entry_stride(element_type);
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        let scratch15 = self.temporary_vreg();
        let scratch17 = self.temporary_vreg();
        let scratch20 = self.temporary_vreg();
        let scratch21 = self.temporary_vreg();
        let scratch22 = self.temporary_vreg();
        let scratch23 = self.temporary_vreg();
        let scratch24 = self.temporary_vreg();
        let scratch25 = self.temporary_vreg();
        let collection_slot = self.allocate_stack_object("map_projection_collection", 8);
        self.emit(abi::store_u64(
            &collection.location,
            abi::stack_pointer(),
            collection_slot,
        ));
        let layout = CollectionTypeLayout::from_type(&format!("List OF {element_type}"))
            .ok_or_else(|| {
                format!("native code collection type 'List OF {element_type}' is not supported")
            })?;
        let data_len_slot = self.allocate_stack_object("map_projection_data_len", 8);
        let result_slot = self.allocate_stack_object("map_projection_result", 8);
        let length_loop = self.label("map_projection_length_loop");
        let length_done = self.label("map_projection_length_done");
        let alloc_ok = self.label("map_projection_alloc_ok");
        let copy_loop = self.label("map_projection_copy_loop");
        let copy_bytes = self.label("map_projection_copy_bytes");
        let copy_bytes_done = self.label("map_projection_copy_bytes_done");
        let copy_done = self.label("map_projection_copy_done");
        let offset_field = if project_key {
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET
        } else {
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET
        };
        let length_field = if project_key {
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH
        } else {
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH
        };

        self.emit(abi::load_u64(
            &scratch8,
            abi::stack_pointer(),
            collection_slot,
        ));
        self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::move_immediate(&scratch10, "Integer", "0"));
        self.emit(abi::move_immediate(&scratch11, "Integer", "0"));
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch8,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::label(&length_loop));
        self.emit(abi::compare_registers(&scratch10, &scratch9));
        self.emit(abi::branch_ge(&length_done));
        self.emit(abi::load_u64(&scratch13, &scratch12, length_field));
        self.emit(abi::add_registers(&scratch11, &scratch11, &scratch13));
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch12,
            COLLECTION_ENTRY_SIZE,
        ));
        self.emit(abi::add_immediate(&scratch10, &scratch10, 1));
        self.emit(abi::branch(&length_loop));
        self.emit(abi::label(&length_done));
        self.emit(abi::store_u64(
            &scratch11,
            abi::stack_pointer(),
            data_len_slot,
        ));
        self.emit(abi::move_immediate(
            &scratch14,
            "Integer",
            &proj_stride.to_string(),
        ));
        // Checked collection-size arithmetic (bug-147.7 / bug-232): count and
        // dataLength come from live collection headers, so route
        // count*ENTRY + HEADER + dataLen through the overflow-guarded helpers the
        // mutate path uses — a wrapped 64-bit size would under-allocate.
        let size_overflow = self.label("map_projection_size_overflow");
        self.emit_checked_size_multiply(&scratch15, &scratch9, &scratch14, &size_overflow);
        self.emit_checked_size_add_immediate(
            abi::return_register(),
            &scratch15,
            COLLECTION_HEADER_SIZE,
            &size_overflow,
        );
        self.emit_checked_size_add(
            abi::return_register(),
            abi::return_register(),
            &scratch11,
            &size_overflow,
        );
        self.emit(abi::move_immediate(abi::ARG[1], "Integer", "8"));
        self.emit_arena_alloc_call();
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
        self.emit(abi::move_immediate(
            &scratch13,
            "Byte",
            &layout.kind.to_string(),
        ));
        self.emit(abi::store_u8(
            &scratch13,
            abi::RET[1],
            COLLECTION_OFFSET_KIND,
        ));
        self.emit(abi::move_immediate(
            &scratch13,
            "Byte",
            &layout.key_type_code.to_string(),
        ));
        self.emit(abi::store_u8(
            &scratch13,
            abi::RET[1],
            COLLECTION_OFFSET_KEY_TYPE,
        ));
        self.emit(abi::move_immediate(
            &scratch13,
            "Byte",
            &layout.value_type_code.to_string(),
        ));
        self.emit(abi::store_u8(
            &scratch13,
            abi::RET[1],
            COLLECTION_OFFSET_VALUE_TYPE,
        ));
        self.emit(abi::move_immediate(&scratch13, "Byte", "1"));
        self.emit(abi::store_u8(
            &scratch13,
            abi::RET[1],
            COLLECTION_OFFSET_FLAGS_VERSION,
        ));
        // `arena_alloc` does not zero the block, so the bucket-index-ready byte is
        // stale poison. This result is a `List OF ...` (never consults the bucket
        // index), but leaving it unwritten is an OOB read waiting to happen if the
        // shape ever changes — zero it like the header writers do (bug-232).
        self.emit(abi::store_u8(
            abi::ZERO,
            abi::RET[1],
            COLLECTION_OFFSET_BUCKETS_READY,
        ));
        self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64(
            &scratch9,
            abi::RET[1],
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::store_u64(
            &scratch9,
            abi::RET[1],
            COLLECTION_OFFSET_CAPACITY,
        ));
        self.emit(abi::load_u64(
            &scratch11,
            abi::stack_pointer(),
            data_len_slot,
        ));
        self.emit(abi::store_u64(
            &scratch11,
            abi::RET[1],
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::store_u64(
            &scratch11,
            abi::RET[1],
            COLLECTION_OFFSET_DATA_CAPACITY,
        ));

        self.emit(abi::load_u64(
            &scratch8,
            abi::stack_pointer(),
            collection_slot,
        ));
        self.emit(abi::load_u64(
            abi::RET[1],
            abi::stack_pointer(),
            result_slot,
        ));
        self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch8,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_immediate(
            &scratch17,
            abi::RET[1],
            COLLECTION_HEADER_SIZE,
        ));
        self.emit_collection_data_pointer_for(&scratch20, &scratch8, "");
        self.emit(abi::move_immediate(
            &scratch14,
            "Integer",
            &proj_stride.to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch21, &scratch9, &scratch14));
        self.emit(abi::add_registers(&scratch21, &scratch17, &scratch21));
        self.emit(abi::move_immediate(&scratch10, "Integer", "0"));
        self.emit(abi::move_immediate(&scratch11, "Integer", "0"));
        self.emit(abi::label(&copy_loop));
        self.emit(abi::compare_registers(&scratch10, &scratch9));
        self.emit(abi::branch_ge(&copy_done));
        self.emit(abi::move_immediate(
            &scratch22,
            "Byte",
            &COLLECTION_ENTRY_FLAG_USED.to_string(),
        ));
        if proj_stride != 0 {
            self.emit(abi::store_u8(
                &scratch22,
                &scratch17,
                COLLECTION_ENTRY_OFFSET_FLAGS,
            ));
        }
        self.emit(abi::move_immediate(&scratch22, "Integer", "0"));
        if proj_stride != 0 {
            self.emit(abi::store_u64(
                &scratch22,
                &scratch17,
                COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
            ));
        }
        if proj_stride != 0 {
            self.emit(abi::store_u64(
                &scratch22,
                &scratch17,
                COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
            ));
        }
        self.emit(abi::load_u64(&scratch22, &scratch12, offset_field));
        self.emit(abi::load_u64(&scratch23, &scratch12, length_field));
        if proj_stride != 0 {
            self.emit(abi::store_u64(
                &scratch11,
                &scratch17,
                COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
            ));
        }
        if proj_stride != 0 {
            self.emit(abi::store_u64(
                &scratch23,
                &scratch17,
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ));
        }
        self.emit(abi::add_registers(&scratch24, &scratch20, &scratch22));
        self.emit(abi::add_registers(&scratch25, &scratch21, &scratch11));
        self.emit(abi::label(&copy_bytes));
        self.emit(abi::compare_immediate(&scratch23, "0"));
        self.emit(abi::branch_eq(&copy_bytes_done));
        self.emit(abi::load_u8(&scratch22, &scratch24, 0));
        self.emit(abi::store_u8(&scratch22, &scratch25, 0));
        self.emit(abi::add_immediate(&scratch24, &scratch24, 1));
        self.emit(abi::add_immediate(&scratch25, &scratch25, 1));
        self.emit(abi::subtract_immediate(&scratch23, &scratch23, 1));
        self.emit(abi::branch(&copy_bytes));
        self.emit(abi::label(&copy_bytes_done));
        if proj_stride != 0 {
            // Reload the length from the entry just written.
            self.emit(abi::load_u64(
                &scratch23,
                &scratch17,
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ));
        } else {
            // kind 2 wrote no entry to read back, so re-derive the length from
            // the SOURCE entry — the same value the copy loop consumed.
            self.emit(abi::load_u64(&scratch23, &scratch12, length_field));
        }
        self.emit(abi::add_registers(&scratch11, &scratch11, &scratch23));
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch12,
            COLLECTION_ENTRY_SIZE,
        ));
        self.emit(abi::add_immediate(&scratch17, &scratch17, proj_stride));
        self.emit(abi::add_immediate(&scratch10, &scratch10, 1));
        self.emit(abi::branch(&copy_loop));
        self.emit(abi::label(&copy_done));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: format!("List OF {element_type}"),
            location: result.render(),
            text: if project_key {
                format!("keys({})", collection.type_)
            } else {
                format!("values({})", collection.type_)
            },
        })
    }

    /// plan-39 A4: intercept `#collections_zip$A$B` and build the paired list
    /// natively when A and B are both fixed-width scalars (the `Pair$A$B` record is
    /// then a flat 16 bytes `[a@0][b@8]`). Anything else — a String/List/record
    /// element, or a non-list argument — falls back to the FUNC (`Ok(None)`).
    pub(super) fn try_inline_zip_op(
        &mut self,
        target: &str,
        args: &[NirValue],
    ) -> Result<Option<ValueResult>, String> {
        let Some(rest) = target.strip_prefix("#collections_zip$") else {
            return Ok(None);
        };
        if args.len() != 2 {
            return Ok(None);
        }
        // The suffix is `<A>$<B>`; only accept it when both are simple fixed-width
        // scalar type names (no nested `$`, so the split is unambiguous).
        let parts: Vec<&str> = rest.split('$').collect();
        if parts.len() != 2 {
            return Ok(None);
        }
        let is_fixed = |t: &str| {
            matches!(
                t,
                "Integer" | "Float" | "Fixed" | "Byte" | "Boolean" | "Scalar"
            )
        };
        if !is_fixed(parts[0]) || !is_fixed(parts[1]) {
            return Ok(None);
        }
        let list_type = format!("List OF Pair${}${}", parts[0], parts[1]);
        let Some(layout) = CollectionTypeLayout::from_type(&list_type) else {
            return Ok(None);
        };
        let result = self.lower_list_zip_fixed(args, &list_type, layout)?;
        Ok(Some(result))
    }

    /// Build `List OF Pair$A$B` from two fixed-width-scalar lists: `n =
    /// min(len a, len b)` entries, each holding the flat 16-byte record
    /// `[a[i]@0][b[i]@8]`. Mirrors `lower_list_slice_range`'s allocate + header +
    /// copy-loop shape; the copy reads one 8-byte value from each source blob.
    /// Load one zip source element from `addr` into `addr`, at its payload width.
    ///
    /// The kind-0 path always read 8 bytes because a lookup entry gave no width
    /// to work from and the `Pair` field is 8 bytes wide regardless. Under kind 2
    /// the width IS known, so a `Byte`/`Boolean` element reads 1 byte and a
    /// `Scalar` 4, instead of 8 — which for those types would pull in the
    /// following elements' packed payload bytes as high-order garbage. `None` is
    /// the kind-0 shape, kept byte-identical.
    fn emit_zip_payload_load(&mut self, addr: impl Into<Operand>, payload: Option<usize>) {
        let addr = addr.into();
        match payload {
            Some(1) => self.emit(abi::load_u8(addr.clone(), addr.clone(), 0)),
            Some(4) => self.emit(abi::load_u32(addr.clone(), addr.clone(), 0)),
            _ => self.emit(abi::load_u64(addr.clone(), addr.clone(), 0)),
        }
    }

    fn lower_list_zip_fixed(
        &mut self,
        args: &[NirValue],
        list_type: &str,
        layout: CollectionTypeLayout,
    ) -> Result<ValueResult, String> {
        const REC: usize = 16; // Pair of two fixed-width fields: [f0@0][f1@8].
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
        let s21 = self.temporary_vreg();
        let s22 = self.temporary_vreg();

        let a_slot = self.allocate_stack_object("zip_a", 8);
        let b_slot = self.allocate_stack_object("zip_b", 8);
        let n_slot = self.allocate_stack_object("zip_n", 8);
        let result_slot = self.allocate_stack_object("zip_result", 8);

        let a = self.lower_value(&args[0])?;
        self.emit(abi::store_u64(&a.location, abi::stack_pointer(), a_slot));
        let b = self.lower_value(&args[1])?;
        self.emit(abi::store_u64(&b.location, abi::stack_pointer(), b_slot));

        // n = min(count_a, count_b).
        self.emit(abi::load_u64(&s8, abi::stack_pointer(), a_slot));
        self.emit(abi::load_u64(&s9, &s8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(&s10, abi::stack_pointer(), b_slot));
        self.emit(abi::load_u64(&s11, &s10, COLLECTION_OFFSET_COUNT));
        let n_done = self.label("zip_n_done");
        self.emit(abi::compare_registers(&s9, &s11));
        self.emit(abi::branch_le(&n_done));
        self.emit(abi::move_register(&s9, &s11));
        self.emit(abi::label(&n_done));
        self.emit(abi::store_u64(&s9, abi::stack_pointer(), n_slot));

        // Allocate HEADER + n*ENTRY + n*REC, through the overflow-guarded helpers
        // the mutate path uses (bug-147.7 / bug-232): a wrapped 64-bit size would
        // under-allocate.
        let size_overflow = self.label("zip_size_overflow");
        self.emit(abi::move_immediate(
            &s14,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit_checked_size_multiply(&s15, &s9, &s14, &size_overflow);
        self.emit_checked_size_add_immediate(
            abi::return_register(),
            &s15,
            COLLECTION_HEADER_SIZE,
            &size_overflow,
        );
        self.emit(abi::move_immediate(&s16, "Integer", &REC.to_string()));
        self.emit_checked_size_multiply(&s16, &s9, &s16, &size_overflow);
        self.emit_checked_size_add(
            abi::return_register(),
            abi::return_register(),
            &s16,
            &size_overflow,
        );
        self.emit(abi::move_immediate(abi::ARG[1], "Integer", "8"));
        self.emit_symbol_call(ARENA_ALLOC_SYMBOL);
        let alloc_ok = self.label("zip_alloc_ok");
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

        // Header. data_length = data_capacity = n*REC.
        self.emit(abi::load_u64(&s9, abi::stack_pointer(), n_slot));
        self.emit(abi::move_immediate(&s16, "Integer", &REC.to_string()));
        self.emit(abi::multiply_registers(&s16, &s9, &s16));
        self.emit(abi::move_immediate(&s13, "Byte", &layout.kind.to_string()));
        self.emit(abi::store_u8(&s13, abi::RET[1], COLLECTION_OFFSET_KIND));
        self.emit(abi::move_immediate(
            &s13,
            "Byte",
            &layout.key_type_code.to_string(),
        ));
        self.emit(abi::store_u8(&s13, abi::RET[1], COLLECTION_OFFSET_KEY_TYPE));
        self.emit(abi::move_immediate(
            &s13,
            "Byte",
            &layout.value_type_code.to_string(),
        ));
        self.emit(abi::store_u8(
            &s13,
            abi::RET[1],
            COLLECTION_OFFSET_VALUE_TYPE,
        ));
        self.emit(abi::move_immediate(&s13, "Byte", "1"));
        self.emit(abi::store_u8(
            &s13,
            abi::RET[1],
            COLLECTION_OFFSET_FLAGS_VERSION,
        ));
        // `arena_alloc` does not zero the block: zero the bucket-index-ready byte
        // rather than leaving stale poison (bug-232).
        self.emit(abi::store_u8(
            abi::ZERO,
            abi::RET[1],
            COLLECTION_OFFSET_BUCKETS_READY,
        ));
        self.emit(abi::store_u64(&s9, abi::RET[1], COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64(&s9, abi::RET[1], COLLECTION_OFFSET_CAPACITY));
        self.emit(abi::store_u64(
            &s16,
            abi::RET[1],
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::store_u64(
            &s16,
            abi::RET[1],
            COLLECTION_OFFSET_DATA_CAPACITY,
        ));

        // Copy loop: entry i holds [a[i]@0][b[i]@8] at blob offset i*REC.
        self.emit(abi::load_u64(&s8, abi::stack_pointer(), a_slot));
        self.emit(abi::load_u64(&s10, abi::stack_pointer(), b_slot));
        self.emit(abi::load_u64(
            abi::RET[1],
            abi::stack_pointer(),
            result_slot,
        ));
        self.emit(abi::load_u64(&s9, abi::stack_pointer(), n_slot));
        // s20 = a blob base, s21 = b blob base, s22 = result blob base. The two
        // inputs are separate lists with their own element types, so each takes
        // its own stride (plan-57-D).
        let a_element = list_element_type(&a.type_).unwrap_or_default();
        let b_element = list_element_type(&b.type_).unwrap_or_default();
        // Both inputs are fixed-width by `try_inline_zip_op`'s guard, so under
        // the entry-free representation BOTH are kind 2 and neither has an entry
        // to read. s12/s13 then carry a byte OFFSET from the blob base rather
        // than an entry pointer, and stride by the payload width. Reading an
        // entry's `valueOffset` off a kind-2 block yields payload bytes, which
        // are then added to the blob base and dereferenced — a wild load, which
        // is how this was found (a SIGSEGV in the benchmark's `zip`).
        let a_payload = kind2_payload_size(&a_element);
        let b_payload = kind2_payload_size(&b_element);
        // s12 = a entry ptr / payload offset, s13 = b likewise, s17 = result
        // entry ptr. The result is a `List OF Pair`, a record element, so it is
        // variable-width and keeps its entry table either way.
        if a_payload.is_some() {
            self.emit(abi::move_immediate(&s12, "Integer", "0"));
        } else {
            self.emit(abi::add_immediate(&s12, &s8, COLLECTION_HEADER_SIZE));
        }
        if b_payload.is_some() {
            self.emit(abi::move_immediate(&s13, "Integer", "0"));
        } else {
            self.emit(abi::add_immediate(&s13, &s10, COLLECTION_HEADER_SIZE));
        }
        self.emit(abi::add_immediate(
            &s17,
            abi::RET[1],
            COLLECTION_HEADER_SIZE,
        ));
        self.emit_collection_data_pointer_for(&s20, &s8, &a_element);
        self.emit_collection_data_pointer_for(&s21, &s10, &b_element);
        self.emit(abi::move_immediate(
            &s16,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&s22, &s9, &s16));
        self.emit(abi::add_registers(&s22, &s17, &s22));
        // s11 = running result-blob offset, s14 = i.
        self.emit(abi::move_immediate(&s11, "Integer", "0"));
        self.emit(abi::move_immediate(&s14, "Integer", "0"));
        let loop_l = self.label("zip_copy_loop");
        let loop_done = self.label("zip_copy_done");
        self.emit(abi::label(&loop_l));
        self.emit(abi::compare_registers(&s14, &s9));
        self.emit(abi::branch_ge(&loop_done));
        // result entry i: flags USED, key 0, value_offset = running, length = REC.
        self.emit(abi::move_immediate(
            &s15,
            "Byte",
            &COLLECTION_ENTRY_FLAG_USED.to_string(),
        ));
        self.emit(abi::store_u8(&s15, &s17, COLLECTION_ENTRY_OFFSET_FLAGS));
        self.emit(abi::move_immediate(&s15, "Integer", "0"));
        self.emit(abi::store_u64(
            &s15,
            &s17,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::store_u64(
            &s15,
            &s17,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::store_u64(
            &s11,
            &s17,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::move_immediate(&s15, "Integer", &REC.to_string()));
        self.emit(abi::store_u64(
            &s15,
            &s17,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        // a[i] value at a_blob + offset, where the offset is the entry's
        // `valueOffset` (kind 0) or the running payload offset itself (kind 2).
        if a_payload.is_some() {
            self.emit(abi::move_register(&s15, &s12));
        } else {
            self.emit(abi::load_u64(
                &s15,
                &s12,
                COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
            ));
        }
        self.emit(abi::add_registers(&s15, &s20, &s15));
        self.emit_zip_payload_load(&s15, a_payload);
        // dest = result_blob + running.
        self.emit(abi::add_registers(&s16, &s22, &s11));
        self.emit(abi::store_u64(&s15, &s16, 0));
        // b[i] value.
        if b_payload.is_some() {
            self.emit(abi::move_register(&s15, &s13));
        } else {
            self.emit(abi::load_u64(
                &s15,
                &s13,
                COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
            ));
        }
        self.emit(abi::add_registers(&s15, &s21, &s15));
        self.emit_zip_payload_load(&s15, b_payload);
        self.emit(abi::store_u64(&s15, &s16, 8));
        // advance.
        self.emit(abi::add_immediate(&s11, &s11, REC));
        self.emit(abi::add_immediate(
            &s12,
            &s12,
            a_payload.unwrap_or(COLLECTION_ENTRY_SIZE),
        ));
        self.emit(abi::add_immediate(
            &s13,
            &s13,
            b_payload.unwrap_or(COLLECTION_ENTRY_SIZE),
        ));
        self.emit(abi::add_immediate(&s17, &s17, COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate(&s14, &s14, 1));
        self.emit(abi::branch(&loop_l));
        self.emit(abi::label(&loop_done));

        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: list_type.to_string(),
            location: result.render(),
            text: format!("zip({list_type})"),
        })
    }

    /// plan-39 A4: intercept the internal `#collections_slice$T` helper and lower
    /// it as a native contiguous-range copy. The only callers are the window/chunks
    /// source generics, which always pass in-bounds `[start, stop)`; a non-list or
    /// unsupported element type falls back to the FUNC (`Ok(None)`).
    pub(super) fn try_inline_slice_op(
        &mut self,
        target: &str,
        args: &[NirValue],
    ) -> Result<Option<ValueResult>, String> {
        if !target.starts_with("#collections_slice$") || args.len() != 3 {
            return Ok(None);
        }
        // Peek the static list type without committing side effects: the arg is a
        // simple local in the generic body, so its static type is known.
        let Some(list_type) = self.static_type_name(&args[0]) else {
            return Ok(None);
        };
        let Some(element_type) = list_element_type(&list_type) else {
            return Ok(None);
        };
        if CollectionTypeLayout::from_type(&list_type).is_none() {
            return Ok(None);
        }
        let result = self.lower_list_slice_range(args, &element_type)?;
        Ok(Some(result))
    }

    /// Build a new `List` holding the source entries `[start, stop)`. Adapts
    /// `lower_map_projection`'s byte-wise payload copy with a running destination
    /// offset — correct for every element type. `start`/`stop` are clamped to
    /// `[0, count]` so an out-of-range index can never read past the source block
    /// (the live callers always pass valid ranges).
    pub(super) fn lower_list_slice_range(
        &mut self,
        args: &[NirValue],
        element_type: &str,
    ) -> Result<ValueResult, String> {
        let layout = CollectionTypeLayout::from_type(&format!("List OF {element_type}"))
            .ok_or_else(|| {
                format!("native code collection type 'List OF {element_type}' is not supported")
            })?;
        let s8 = self.temporary_vreg();
        let s9 = self.temporary_vreg();
        let s10 = self.temporary_vreg();
        let s11 = self.temporary_vreg();
        let s12 = self.temporary_vreg();
        let s13 = self.temporary_vreg();
        let s14 = self.temporary_vreg();
        let s15 = self.temporary_vreg();
        let s17 = self.temporary_vreg();
        let s20 = self.temporary_vreg();
        let s21 = self.temporary_vreg();
        let s22 = self.temporary_vreg();
        let s23 = self.temporary_vreg();
        let s24 = self.temporary_vreg();
        let s25 = self.temporary_vreg();

        let collection_slot = self.allocate_stack_object("slice_collection", 8);
        let start_slot = self.allocate_stack_object("slice_start", 8);
        let stop_slot = self.allocate_stack_object("slice_stop", 8);
        let count_slot = self.allocate_stack_object("slice_count", 8);
        let data_len_slot = self.allocate_stack_object("slice_data_len", 8);
        let result_slot = self.allocate_stack_object("slice_result", 8);

        // Lower each argument and spill immediately so a later lowering (which may
        // reset the temporary-register pool) cannot alias a live input.
        let list = self.lower_value(&args[0])?;
        self.emit(abi::store_u64(
            &list.location,
            abi::stack_pointer(),
            collection_slot,
        ));
        let start = self.lower_value(&args[1])?;
        self.emit(abi::store_u64(
            &start.location,
            abi::stack_pointer(),
            start_slot,
        ));
        let stop = self.lower_value(&args[2])?;
        self.emit(abi::store_u64(
            &stop.location,
            abi::stack_pointer(),
            stop_slot,
        ));

        // Clamp start into [0, count] and stop into [start, count]; count' = stop-start.
        self.emit(abi::load_u64(&s8, abi::stack_pointer(), collection_slot));
        self.emit(abi::load_u64(&s9, &s8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(&s10, abi::stack_pointer(), start_slot));
        let s_ge0 = self.label("slice_s_ge0");
        self.emit(abi::compare_immediate(&s10, "0"));
        self.emit(abi::branch_ge(&s_ge0));
        self.emit(abi::move_immediate(&s10, "Integer", "0"));
        self.emit(abi::label(&s_ge0));
        let s_le = self.label("slice_s_le");
        self.emit(abi::compare_registers(&s10, &s9));
        self.emit(abi::branch_le(&s_le));
        self.emit(abi::move_register(&s10, &s9));
        self.emit(abi::label(&s_le));
        self.emit(abi::load_u64(&s11, abi::stack_pointer(), stop_slot));
        let e_ges = self.label("slice_e_ges");
        self.emit(abi::compare_registers(&s11, &s10));
        self.emit(abi::branch_ge(&e_ges));
        self.emit(abi::move_register(&s11, &s10));
        self.emit(abi::label(&e_ges));
        let e_le = self.label("slice_e_le");
        self.emit(abi::compare_registers(&s11, &s9));
        self.emit(abi::branch_le(&e_le));
        self.emit(abi::move_register(&s11, &s9));
        self.emit(abi::label(&e_le));
        self.emit(abi::subtract_registers(&s12, &s11, &s10));
        self.emit(abi::store_u64(&s10, abi::stack_pointer(), start_slot));
        self.emit(abi::store_u64(&s12, abi::stack_pointer(), count_slot));

        // Length pass: sum value_lengths of entries [start, start+count').
        // kind 2 has no entries and a constant payload, so the sum is
        // `count * payloadSize` (plan-57-D).
        let slice_payload = kind2_payload_size(&element_type);
        let len_loop = self.label("slice_len_loop");
        let len_done = self.label("slice_len_done");
        if let Some(payload) = slice_payload {
            self.emit(abi::move_immediate(&s14, "Integer", &payload.to_string()));
            self.emit(abi::multiply_registers(&s13, &s12, &s14));
            self.emit(abi::branch(&len_done));
        }
        self.emit(abi::move_immediate(
            &s14,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&s13, &s10, &s14));
        self.emit(abi::add_immediate(&s15, &s8, COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers(&s15, &s15, &s13));
        self.emit(abi::move_immediate(&s13, "Integer", "0"));
        self.emit(abi::move_immediate(&s17, "Integer", "0"));
        self.emit(abi::label(&len_loop));
        self.emit(abi::compare_registers(&s17, &s12));
        self.emit(abi::branch_ge(&len_done));
        self.emit(abi::load_u64(
            &s20,
            &s15,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers(&s13, &s13, &s20));
        self.emit(abi::add_immediate(&s15, &s15, COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate(&s17, &s17, 1));
        self.emit(abi::branch(&len_loop));
        self.emit(abi::label(&len_done));
        self.emit(abi::store_u64(&s13, abi::stack_pointer(), data_len_slot));

        // Allocate HEADER + count'*ENTRY + data_len.
        self.emit(abi::load_u64(&s12, abi::stack_pointer(), count_slot));
        self.emit(abi::load_u64(&s13, abi::stack_pointer(), data_len_slot));
        // Overflow-guarded size arithmetic (bug-147.7 / bug-232): count and
        // data_len come from live headers; a wrapped size would under-allocate.
        let size_overflow = self.label("slice_size_overflow");
        self.emit(abi::move_immediate(
            &s14,
            "Integer",
            &slice_payload
                .map_or(COLLECTION_ENTRY_SIZE, |_| 0)
                .to_string(),
        ));
        self.emit_checked_size_multiply(&s15, &s12, &s14, &size_overflow);
        self.emit_checked_size_add_immediate(
            abi::return_register(),
            &s15,
            COLLECTION_HEADER_SIZE,
            &size_overflow,
        );
        self.emit_checked_size_add(
            abi::return_register(),
            abi::return_register(),
            &s13,
            &size_overflow,
        );
        self.emit(abi::move_immediate(abi::ARG[1], "Integer", "8"));
        self.emit_symbol_call(ARENA_ALLOC_SYMBOL);
        let alloc_ok = self.label("slice_alloc_ok");
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

        // Header.
        self.emit(abi::move_immediate(&s13, "Byte", &layout.kind.to_string()));
        self.emit(abi::store_u8(&s13, abi::RET[1], COLLECTION_OFFSET_KIND));
        self.emit(abi::move_immediate(
            &s13,
            "Byte",
            &layout.key_type_code.to_string(),
        ));
        self.emit(abi::store_u8(&s13, abi::RET[1], COLLECTION_OFFSET_KEY_TYPE));
        self.emit(abi::move_immediate(
            &s13,
            "Byte",
            &layout.value_type_code.to_string(),
        ));
        self.emit(abi::store_u8(
            &s13,
            abi::RET[1],
            COLLECTION_OFFSET_VALUE_TYPE,
        ));
        self.emit(abi::move_immediate(&s13, "Byte", "1"));
        self.emit(abi::store_u8(
            &s13,
            abi::RET[1],
            COLLECTION_OFFSET_FLAGS_VERSION,
        ));
        // `arena_alloc` does not zero the block: zero the bucket-index-ready byte
        // rather than leaving stale poison (bug-232).
        self.emit(abi::store_u8(
            abi::ZERO,
            abi::RET[1],
            COLLECTION_OFFSET_BUCKETS_READY,
        ));
        self.emit(abi::load_u64(&s12, abi::stack_pointer(), count_slot));
        self.emit(abi::store_u64(&s12, abi::RET[1], COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64(
            &s12,
            abi::RET[1],
            COLLECTION_OFFSET_CAPACITY,
        ));
        self.emit(abi::load_u64(&s13, abi::stack_pointer(), data_len_slot));
        self.emit(abi::store_u64(
            &s13,
            abi::RET[1],
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::store_u64(
            &s13,
            abi::RET[1],
            COLLECTION_OFFSET_DATA_CAPACITY,
        ));

        // Copy pass: for each entry in [start, start+count') copy its value payload
        // into the new blob and rewrite the entry's value_offset to the running one.
        self.emit(abi::load_u64(&s8, abi::stack_pointer(), collection_slot));
        self.emit(abi::load_u64(
            abi::RET[1],
            abi::stack_pointer(),
            result_slot,
        ));
        self.emit(abi::load_u64(&s10, abi::stack_pointer(), start_slot));
        self.emit(abi::load_u64(&s9, abi::stack_pointer(), count_slot));
        self.emit(abi::move_immediate(
            &s14,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&s13, &s10, &s14));
        self.emit(abi::add_immediate(&s12, &s8, COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers(&s12, &s12, &s13));
        self.emit(abi::add_immediate(
            &s17,
            abi::RET[1],
            COLLECTION_HEADER_SIZE,
        ));
        self.emit_collection_data_pointer_for(&s20, &s8, element_type);
        self.emit(abi::multiply_registers(&s21, &s9, &s14));
        self.emit(abi::add_registers(&s21, &s17, &s21));
        self.emit(abi::move_immediate(&s11, "Integer", "0"));
        self.emit(abi::move_immediate(&s10, "Integer", "0"));
        let copy_loop = self.label("slice_copy_loop");
        let copy_done = self.label("slice_copy_done");
        let copy_bytes = self.label("slice_copy_bytes");
        let copy_bytes_done = self.label("slice_copy_bytes_done");
        // kind 2: the slice is one contiguous span of the data region and there
        // are no entries to rebuild, so the whole per-element loop below reduces
        // to a single block copy (plan-57-D).
        if let Some(payload) = slice_payload {
            self.emit(abi::load_u64(&s10, abi::stack_pointer(), start_slot));
            self.emit(abi::load_u64(&s9, abi::stack_pointer(), count_slot));
            self.emit(abi::move_immediate(&s14, "Integer", &payload.to_string()));
            self.emit(abi::multiply_registers(&s13, &s10, &s14)); // start * payload
            self.emit(abi::add_registers(&s24, &s20, &s13)); // src.data + start*p
            self.emit(abi::load_u64(
                abi::RET[1],
                abi::stack_pointer(),
                result_slot,
            ));
            self.emit(abi::add_immediate(
                &s25,
                abi::RET[1],
                COLLECTION_HEADER_SIZE,
            ));
            self.emit(abi::multiply_registers(&s23, &s9, &s14)); // count * payload
            self.emit_block_copy_advance(&s25, &s24, &s23, &s22, "slice_kind2");
            self.emit(abi::branch(&copy_done));
        }
        self.emit(abi::label(&copy_loop));
        self.emit(abi::compare_registers(&s10, &s9));
        self.emit(abi::branch_ge(&copy_done));
        self.emit(abi::move_immediate(
            &s22,
            "Byte",
            &COLLECTION_ENTRY_FLAG_USED.to_string(),
        ));
        self.emit(abi::store_u8(&s22, &s17, COLLECTION_ENTRY_OFFSET_FLAGS));
        self.emit(abi::move_immediate(&s22, "Integer", "0"));
        self.emit(abi::store_u64(
            &s22,
            &s17,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::store_u64(
            &s22,
            &s17,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::load_u64(
            &s22,
            &s12,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            &s23,
            &s12,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::store_u64(
            &s11,
            &s17,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::store_u64(
            &s23,
            &s17,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers(&s24, &s20, &s22));
        self.emit(abi::add_registers(&s25, &s21, &s11));
        self.emit(abi::label(&copy_bytes));
        self.emit(abi::compare_immediate(&s23, "0"));
        self.emit(abi::branch_eq(&copy_bytes_done));
        self.emit(abi::load_u8(&s22, &s24, 0));
        self.emit(abi::store_u8(&s22, &s25, 0));
        self.emit(abi::add_immediate(&s24, &s24, 1));
        self.emit(abi::add_immediate(&s25, &s25, 1));
        self.emit(abi::subtract_immediate(&s23, &s23, 1));
        self.emit(abi::branch(&copy_bytes));
        self.emit(abi::label(&copy_bytes_done));
        self.emit(abi::load_u64(
            &s23,
            &s17,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers(&s11, &s11, &s23));
        self.emit(abi::add_immediate(&s12, &s12, COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate(&s17, &s17, COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate(&s10, &s10, 1));
        self.emit(abi::branch(&copy_loop));
        self.emit(abi::label(&copy_done));

        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: format!("List OF {element_type}"),
            location: result.render(),
            text: format!("slice(List OF {element_type})"),
        })
    }

    pub(super) fn lower_collection_sum(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        let scratch15 = self.temporary_vreg();
        let scratch16 = self.temporary_vreg();
        let collection = self.lower_value(&args[0])?;
        let Some(element_type) = list_element_type(&collection.type_) else {
            return Err(format!(
                "native collection sum does not accept {}",
                collection.type_
            ));
        };
        if !matches!(element_type.as_str(), "Integer" | "Float" | "Fixed") {
            return Err(format!(
                "native collection sum does not accept {}",
                collection.type_
            ));
        }
        let collection_slot = self.allocate_stack_object("sum_collection", 8);
        self.emit(abi::store_u64(
            &collection.location,
            abi::stack_pointer(),
            collection_slot,
        ));
        let loop_label = self.label("sum_loop");
        let done = self.label("sum_done");
        self.emit(abi::load_u64(
            &scratch8,
            abi::stack_pointer(),
            collection_slot,
        ));
        self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::move_immediate(&scratch10, "Integer", "0"));
        self.emit(abi::add_immediate(
            &scratch11,
            &scratch8,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::move_immediate(&scratch14, &element_type, "0"));
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_registers(&scratch10, &scratch9));
        self.emit(abi::branch_ge(&done));
        // kind 2: the cursor (scratch11) already walks the data region, so it IS
        // the payload address — there is no entry to indirect through.
        if kind2_payload_size(&element_type).is_some() {
            self.emit(abi::move_register(&scratch15, &scratch11));
        } else {
            self.emit(abi::load_u64(
                &scratch12,
                &scratch11,
                COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
            ));
            self.emit_collection_data_pointer_for(&scratch15, &scratch8, &element_type);
            self.emit(abi::add_registers(&scratch15, &scratch15, &scratch12));
        }
        match element_type.as_str() {
            "Integer" => {
                self.emit(abi::load_u64(&scratch16, &scratch15, 0));
                self.emit_checked_integer_add(&scratch14, &scratch14, &scratch16)?;
            }
            "Float" => {
                self.emit(abi::load_u64(&scratch16, &scratch15, 0));
                self.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[0], &scratch14));
                self.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[1], &scratch16));
                self.emit(abi::float_add_d(
                    abi::FP_SCRATCH[0],
                    abi::FP_SCRATCH[0],
                    abi::FP_SCRATCH[1],
                ));
                self.emit(abi::float_move_x_from_d(&scratch14, abi::FP_SCRATCH[0]));
            }
            "Fixed" => {
                self.emit(abi::load_u64(&scratch16, &scratch15, 0));
                self.emit_checked_integer_add(&scratch14, &scratch14, &scratch16)?;
            }
            _ => unreachable!(),
        }
        self.emit(abi::add_immediate(
            &scratch11,
            &scratch11,
            kind2_payload_size(&element_type).unwrap_or(COLLECTION_ENTRY_SIZE),
        ));
        self.emit(abi::add_immediate(&scratch10, &scratch10, 1));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&done));
        let result = self.allocate_register()?;
        self.emit(abi::move_register(&result, &scratch14));
        Ok(ValueResult {
            type_: element_type,
            location: result.render(),
            text: format!("sum({})", collection.type_),
        })
    }

    pub(super) fn lower_collection_for_each_call(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch17 = self.temporary_vreg();
        let collection = self.lower_value(&args[0])?;
        let Some(element_type) = list_element_type(&collection.type_) else {
            return Err(format!(
                "native collection forEach does not accept {}",
                collection.type_
            ));
        };
        let action = self.lower_value(&args[1])?;
        if !action.type_.starts_with("FUNC(") {
            return Err(format!(
                "native collection forEach action must be a function, got {}",
                action.type_
            ));
        }
        if action.location == "void" {
            return Err(
                "native collection forEach action does not have a callable location".to_string(),
            );
        }
        let action_slot = self.allocate_stack_object("for_each_call_action", 8);
        self.emit(abi::store_u64(
            &action.location,
            abi::stack_pointer(),
            action_slot,
        ));
        let collection_slot = self.allocate_stack_object("for_each_call_collection", 8);
        let cursor_slot = self.allocate_stack_object("for_each_call_cursor", 8);
        let remaining_slot = self.allocate_stack_object("for_each_call_remaining", 8);
        self.emit(abi::store_u64(
            &collection.location,
            abi::stack_pointer(),
            collection_slot,
        ));
        self.emit(abi::load_u64(
            &scratch8,
            abi::stack_pointer(),
            collection_slot,
        ));
        self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
        // A kind-2 list has no entry table: the cursor carries a byte OFFSET from
        // the data base rather than an entry pointer, and strides by payloadSize
        // (plan-57-D), matching `lower_for_each`. Reading an entry's
        // offset/length words off a kind-2 block interprets payload bytes as
        // addresses, which segfaults rather than returning a wrong value.
        let payload_size = kind2_payload_size(&element_type);
        if payload_size.is_some() {
            self.emit(abi::move_immediate(&scratch10, "Integer", "0"));
        } else {
            self.emit(abi::add_immediate(
                &scratch10,
                &scratch8,
                COLLECTION_HEADER_SIZE,
            ));
        }
        self.emit(abi::store_u64(
            &scratch10,
            abi::stack_pointer(),
            cursor_slot,
        ));
        self.emit(abi::store_u64(
            &scratch9,
            abi::stack_pointer(),
            remaining_slot,
        ));
        let loop_label = self.label("for_each_call_loop");
        let ok_label = self.label("for_each_call_ok");
        let done = self.label("for_each_call_done");
        self.emit(abi::label(&loop_label));
        self.emit(abi::load_u64(
            &scratch9,
            abi::stack_pointer(),
            remaining_slot,
        ));
        self.emit(abi::compare_immediate(&scratch9, "0"));
        self.emit(abi::branch_eq(&done));
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), cursor_slot));
        if let Some(payload) = payload_size {
            self.emit(abi::move_register(&scratch11, &scratch10));
            self.emit(abi::move_immediate(
                &scratch12,
                "Integer",
                &payload.to_string(),
            ));
        } else {
            self.emit(abi::load_u64(
                &scratch11,
                &scratch10,
                COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
            ));
            self.emit(abi::load_u64(
                &scratch12,
                &scratch10,
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ));
        }
        self.emit(abi::load_u64(
            &scratch8,
            abi::stack_pointer(),
            collection_slot,
        ));
        let item =
            self.emit_load_collection_payload(&element_type, &scratch8, &scratch11, &scratch12)?;
        // bug-307: stash the block pointer before the callback; the call clobbers
        // every caller-saved register, so the register alone cannot be relied on.
        let free_slot = self.allocate_stack_object("for_each_item_free", 8);
        self.emit(abi::store_u64(&item, abi::stack_pointer(), free_slot));
        self.emit(abi::move_register(&abi::argument_register(0)?, &item));
        self.emit(abi::load_u64(&scratch17, abi::stack_pointer(), action_slot));
        self.emit_direct_callable_branch(&scratch17);
        self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
        self.emit(abi::branch_eq(&ok_label));
        // A failing callback: forEach owns no accumulator, so no cleanup — under
        // an inline TRAP the raw error routes to the capture point (plan-26-B).
        self.emit_callback_failure_exit(None)?;
        self.emit(abi::label(&ok_label));
        // bug-307: the callback took the item by value and retains nothing, so the
        // freshly materialized String block is dead here.
        self.free_collection_loop_item(free_slot, &element_type)?;
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), cursor_slot));
        self.emit(abi::add_immediate(
            &scratch10,
            &scratch10,
            payload_size.unwrap_or(COLLECTION_ENTRY_SIZE),
        ));
        self.emit(abi::store_u64(
            &scratch10,
            abi::stack_pointer(),
            cursor_slot,
        ));
        self.emit(abi::load_u64(
            &scratch9,
            abi::stack_pointer(),
            remaining_slot,
        ));
        self.emit(abi::subtract_immediate(&scratch9, &scratch9, 1));
        self.emit(abi::store_u64(
            &scratch9,
            abi::stack_pointer(),
            remaining_slot,
        ));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&done));
        Ok(ValueResult {
            type_: "Nothing".to_string(),
            location: "void".to_string(),
            text: format!("forEach({}, {})", collection.type_, action.text),
        })
    }

    pub(super) fn lower_collection_transform_call(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let scratch9 = self.temporary_vreg();
        let scratch17 = self.temporary_vreg();
        let collection = self.lower_value(&args[0])?;
        let Some(element_type) = list_element_type(&collection.type_) else {
            return Err(format!(
                "native collection transform does not accept {}",
                collection.type_
            ));
        };
        let collection_slot = self.allocate_stack_object("transform_collection", 8);
        self.emit(abi::store_u64(
            &collection.location,
            abi::stack_pointer(),
            collection_slot,
        ));
        let action = self.lower_value(&args[1])?;
        let output_type = callable_return_type(&action.type_).ok_or_else(|| {
            format!(
                "native collection transform action must be a function, got {}",
                action.type_
            )
        })?;
        self.require_direct_callable("transform", &action)?;
        let action_slot = self.allocate_stack_object("transform_action", 8);
        self.emit(abi::store_u64(
            &action.location,
            abi::stack_pointer(),
            action_slot,
        ));
        let output_list_type = format!("List OF {output_type}");
        // Pre-size the output to the source's working set so the per-element
        // append never regrows the entry table (transform emits exactly
        // count(source) entries) — plan-25-B B2.
        let output = self.lower_reserved_list(&output_list_type, collection_slot)?;
        let output_slot = self.allocate_stack_object("transform_output", 8);
        let cursor_slot = self.allocate_stack_object("transform_cursor", 8);
        let remaining_slot = self.allocate_stack_object("transform_remaining", 8);
        self.emit(abi::store_u64(
            &output.location,
            abi::stack_pointer(),
            output_slot,
        ));
        self.initialize_collection_loop_slots(
            collection_slot,
            cursor_slot,
            remaining_slot,
            &element_type,
        );

        let loop_label = self.label("transform_call_loop");
        let ok_label = self.label("transform_call_ok");
        let done = self.label("transform_call_done");
        self.emit(abi::label(&loop_label));
        self.emit(abi::load_u64(
            &scratch9,
            abi::stack_pointer(),
            remaining_slot,
        ));
        self.emit(abi::compare_immediate(&scratch9, "0"));
        self.emit(abi::branch_eq(&done));
        let item = self.load_collection_loop_item(collection_slot, cursor_slot, &element_type)?;
        // bug-307: stash before the callback (calls clobber caller-saved registers).
        let free_slot = self.allocate_stack_object("transform_item_free", 8);
        self.emit(abi::store_u64(&item, abi::stack_pointer(), free_slot));
        self.emit(abi::move_register(&abi::argument_register(0)?, &item));
        self.emit(abi::load_u64(&scratch17, abi::stack_pointer(), action_slot));
        self.emit_direct_callable_branch(&scratch17);
        self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
        self.emit(abi::branch_eq(&ok_label));
        // A failing callback: free the partial output list (a private, uniquely-
        // owned buffer) before routing the raw error to the inline-TRAP capture
        // point (plan-26-B); non-trapped, this is the same auto-propagating return.
        self.emit_callback_failure_exit(Some((output_slot, output_list_type.clone())))?;
        self.emit(abi::label(&ok_label));

        let item_slot = self.allocate_stack_object("transform_item", 8);
        self.emit(abi::store_u64(
            RESULT_VALUE_REGISTER,
            abi::stack_pointer(),
            item_slot,
        ));
        // bug-307: only AFTER the callback's result is safely in its slot. The free
        // is a call and would otherwise destroy RESULT_VALUE_REGISTER before it was
        // stored. The appended value is that result, a separate allocation, so the
        // source item is not retained and is dead here.
        self.free_collection_loop_item(free_slot, &element_type)?;
        // The output accumulator is a private, uniquely-owned buffer, so append
        // each transformed item in place with geometric headroom (plan-01 §4.2)
        // — amortized O(1) instead of the O(n) splice the singleton+insert did.
        self.lower_list_append_in_place(output_slot, item_slot, &output_list_type, &output_type)?;
        self.advance_collection_loop(cursor_slot, remaining_slot, &loop_label, &element_type);
        self.emit(abi::label(&done));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), output_slot));
        Ok(ValueResult {
            type_: output_list_type,
            location: result.render(),
            text: format!("transform({}, {})", collection.type_, action.text),
        })
    }

    pub(super) fn lower_collection_filter_call(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let scratch9 = self.temporary_vreg();
        let scratch17 = self.temporary_vreg();
        let collection = self.lower_value(&args[0])?;
        let Some(element_type) = list_element_type(&collection.type_) else {
            return Err(format!(
                "native collection filter does not accept {}",
                collection.type_
            ));
        };
        let collection_slot = self.allocate_stack_object("filter_collection", 8);
        self.emit(abi::store_u64(
            &collection.location,
            abi::stack_pointer(),
            collection_slot,
        ));
        let action = self.lower_value(&args[1])?;
        let output_type = callable_return_type(&action.type_).ok_or_else(|| {
            format!(
                "native collection filter predicate must be a function, got {}",
                action.type_
            )
        })?;
        if output_type != "Boolean" {
            return Err(format!(
                "native collection filter predicate must return Boolean, got {output_type}"
            ));
        }
        self.require_direct_callable("filter", &action)?;
        let action_slot = self.allocate_stack_object("filter_action", 8);
        self.emit(abi::store_u64(
            &action.location,
            abi::stack_pointer(),
            action_slot,
        ));
        // Pre-size the output to the source: filter's result is a subset, so the
        // per-element append regrows neither the entry table nor the data region
        // (plan-25-B B2).
        let output = self.lower_reserved_list(&collection.type_, collection_slot)?;
        let output_slot = self.allocate_stack_object("filter_output", 8);
        let cursor_slot = self.allocate_stack_object("filter_cursor", 8);
        let remaining_slot = self.allocate_stack_object("filter_remaining", 8);
        let item_slot = self.allocate_stack_object("filter_item", 8);
        self.emit(abi::store_u64(
            &output.location,
            abi::stack_pointer(),
            output_slot,
        ));
        self.initialize_collection_loop_slots(
            collection_slot,
            cursor_slot,
            remaining_slot,
            &element_type,
        );

        let loop_label = self.label("filter_call_loop");
        let ok_label = self.label("filter_call_ok");
        let keep_label = self.label("filter_call_keep");
        let skip_label = self.label("filter_call_skip");
        let done = self.label("filter_call_done");
        self.emit(abi::label(&loop_label));
        self.emit(abi::load_u64(
            &scratch9,
            abi::stack_pointer(),
            remaining_slot,
        ));
        self.emit(abi::compare_immediate(&scratch9, "0"));
        self.emit(abi::branch_eq(&done));
        let item = self.load_collection_loop_item(collection_slot, cursor_slot, &element_type)?;
        self.emit(abi::store_u64(&item, abi::stack_pointer(), item_slot));
        self.emit(abi::move_register(&abi::argument_register(0)?, &item));
        self.emit(abi::load_u64(&scratch17, abi::stack_pointer(), action_slot));
        self.emit_direct_callable_branch(&scratch17);
        self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
        self.emit(abi::branch_eq(&ok_label));
        // A failing predicate: free the partial output list before routing the raw
        // error to the inline-TRAP capture point (plan-26-B).
        self.emit_callback_failure_exit(Some((output_slot, collection.type_.clone())))?;
        self.emit(abi::label(&ok_label));
        self.emit(abi::compare_immediate(RESULT_VALUE_REGISTER, "0"));
        self.emit(abi::branch_ne(&keep_label));
        self.emit(abi::branch(&skip_label));
        self.emit(abi::label(&keep_label));
        // Private accumulator → append in place with headroom (plan-01 §4.2).
        self.lower_list_append_in_place(output_slot, item_slot, &collection.type_, &element_type)?;
        self.emit(abi::label(&skip_label));
        // bug-307: freed after the append on purpose. `emit_copy_payload_to_collection`
        // COPIES the String's bytes into the output's packed data region rather than
        // storing the pointer, so the source block is dead on both the keep and skip
        // paths — which is why the free sits below `skip_label`, covering both.
        // `item_slot` already holds the pointer (stored before the callback), so it
        // survives both calls.
        self.free_collection_loop_item(item_slot, &element_type)?;
        self.advance_collection_loop(cursor_slot, remaining_slot, &loop_label, &element_type);
        self.emit(abi::label(&done));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), output_slot));
        Ok(ValueResult {
            type_: collection.type_.clone(),
            location: result.render(),
            text: format!("filter({}, {})", collection.type_, action.text),
        })
    }

    /// plan-64 D4 / plan-86 A2: native `collections::partition` for **8-byte
    /// fixed-width elements** (Integer/Float/Fixed/Money) and for **String**.
    /// Splits the source into `matched`/`unmatched` in a single predicate pass —
    /// exactly like the `.mfb` `__collections_partition`, but without the
    /// per-element `collections::get` copy and indirect-append churn — then builds
    /// the `Partition OF T` record by inlining both flat lists once (the same
    /// `emit_build_inlined_record` the interpreted `Partition[matched, unmatched]`
    /// constructor uses, so the record bytes are constructed identically). String
    /// items are read through `load_collection_loop_item` (materializes an owned
    /// block), written through `lower_list_append_in_place` (copies the bytes into
    /// the destination data region), and the materialized item is freed after the
    /// append (plan-86 A2, mirroring `filter`). Scalar/Byte elements fall through to
    /// the `.mfb` version at the dispatch gate.
    pub(super) fn lower_collection_partition_call(
        &mut self,
        args: &[NirValue],
        element_type: &str,
    ) -> Result<ValueResult, String> {
        let scratch9 = self.temporary_vreg();
        let scratch17 = self.temporary_vreg();
        let collection = self.lower_value(&args[0])?;
        if list_element_type(&collection.type_).as_deref() != Some(element_type) {
            return Err(format!(
                "native partition element mismatch: {} vs {element_type}",
                collection.type_
            ));
        }
        let collection_slot = self.allocate_stack_object("partition_collection", 8);
        self.emit(abi::store_u64(
            &collection.location,
            abi::stack_pointer(),
            collection_slot,
        ));
        let action = self.lower_value(&args[1])?;
        let output_type = callable_return_type(&action.type_).ok_or_else(|| {
            format!(
                "native collection partition predicate must be a function, got {}",
                action.type_
            )
        })?;
        if output_type != "Boolean" {
            return Err(format!(
                "native collection partition predicate must return Boolean, got {output_type}"
            ));
        }
        self.require_direct_callable("partition", &action)?;
        let action_slot = self.allocate_stack_object("partition_action", 8);
        self.emit(abi::store_u64(
            &action.location,
            abi::stack_pointer(),
            action_slot,
        ));

        // Two subset outputs, each pre-sized to the source so neither append
        // regrows (a partition is a full split — |matched| + |unmatched| == |src|).
        let matched = self.lower_reserved_list(&collection.type_, collection_slot)?;
        let matched_slot = self.allocate_stack_object("partition_matched", 8);
        self.emit(abi::store_u64(
            &matched.location,
            abi::stack_pointer(),
            matched_slot,
        ));
        let unmatched = self.lower_reserved_list(&collection.type_, collection_slot)?;
        let unmatched_slot = self.allocate_stack_object("partition_unmatched", 8);
        self.emit(abi::store_u64(
            &unmatched.location,
            abi::stack_pointer(),
            unmatched_slot,
        ));

        let cursor_slot = self.allocate_stack_object("partition_cursor", 8);
        let remaining_slot = self.allocate_stack_object("partition_remaining", 8);
        let item_slot = self.allocate_stack_object("partition_item", 8);
        self.initialize_collection_loop_slots(
            collection_slot,
            cursor_slot,
            remaining_slot,
            element_type,
        );

        let loop_label = self.label("partition_loop");
        let ok_label = self.label("partition_ok");
        let to_unmatched = self.label("partition_unmatched");
        let after_append = self.label("partition_after_append");
        let done = self.label("partition_done");
        self.emit(abi::label(&loop_label));
        self.emit(abi::load_u64(
            &scratch9,
            abi::stack_pointer(),
            remaining_slot,
        ));
        self.emit(abi::compare_immediate(&scratch9, "0"));
        self.emit(abi::branch_eq(&done));
        let item = self.load_collection_loop_item(collection_slot, cursor_slot, element_type)?;
        self.emit(abi::store_u64(&item, abi::stack_pointer(), item_slot));
        self.emit(abi::move_register(&abi::argument_register(0)?, &item));
        self.emit(abi::load_u64(&scratch17, abi::stack_pointer(), action_slot));
        self.emit_direct_callable_branch(&scratch17);
        self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
        self.emit(abi::branch_eq(&ok_label));
        // A failing predicate: free BOTH partial output lists before routing the
        // raw error to the inline-TRAP capture point (plan-26-B). `emit_owned_value_drop`
        // clobbers caller-saved regs, so the four raw-result registers are spilled
        // across the two drops and reloaded before the branch.
        match self.raw_result_capture.clone() {
            None => self.emit(abi::return_()),
            Some(capture) => {
                let regs = [
                    RESULT_TAG_REGISTER,
                    RESULT_VALUE_REGISTER,
                    RESULT_ERROR_MESSAGE_REGISTER,
                    RESULT_ERROR_SOURCE_REGISTER,
                ];
                let save: Vec<usize> = regs
                    .iter()
                    .map(|_| self.allocate_stack_object("partition_fail_result", 8))
                    .collect();
                for (reg, slot) in regs.iter().zip(&save) {
                    self.emit(abi::store_u64(reg, abi::stack_pointer(), *slot));
                }
                self.emit_owned_value_drop(&OwnedValueCleanup {
                    type_: collection.type_.clone(),
                    stack_offset: matched_slot,
                    closure_captures: None,
                })?;
                self.emit_owned_value_drop(&OwnedValueCleanup {
                    type_: collection.type_.clone(),
                    stack_offset: unmatched_slot,
                    closure_captures: None,
                })?;
                for (reg, slot) in regs.iter().zip(&save) {
                    self.emit(abi::load_u64(reg, abi::stack_pointer(), *slot));
                }
                self.emit(abi::branch(&capture));
            }
        }
        self.emit(abi::label(&ok_label));
        self.emit(abi::compare_immediate(RESULT_VALUE_REGISTER, "0"));
        self.emit(abi::branch_eq(&to_unmatched));
        self.lower_list_append_in_place(matched_slot, item_slot, &collection.type_, element_type)?;
        self.emit(abi::branch(&after_append));
        self.emit(abi::label(&to_unmatched));
        self.lower_list_append_in_place(
            unmatched_slot,
            item_slot,
            &collection.type_,
            element_type,
        )?;
        self.emit(abi::label(&after_append));
        // bug-307 (plan-86 A2): freed after the append on purpose, mirroring
        // `lower_collection_filter_call`. `lower_list_append_in_place` COPIES the
        // String's bytes into the destination's packed data region rather than
        // storing the pointer, so the materialized source block is dead on both the
        // matched and unmatched paths — which is why the free sits at `after_append`,
        // covering both. A no-op for fixed-width elements (they materialize nothing).
        // `item_slot` already holds the pointer (stored before the predicate call),
        // so it survives both appends.
        self.free_collection_loop_item(item_slot, element_type)?;
        self.advance_collection_loop(cursor_slot, remaining_slot, &loop_label, element_type);
        self.emit(abi::label(&done));

        // Build `Partition OF T` = {matched, unmatched} by inlining both flat lists,
        // then free the two now-consumed intermediate blocks (the record owns byte
        // copies). `free_intermediate_collection` spills the record pointer across
        // each `arena_free`.
        // The monomorphized generic record is NIR-mangled `Partition$<T>` (the same
        // key `record_fields` holds and the interpreted `Partition[...]` constructor
        // looks up), NOT the surface `Partition OF <T>`.
        let record_type = format!("Partition${element_type}");
        let record_reg =
            self.emit_build_inlined_record(&record_type, &[matched_slot, unmatched_slot])?;
        let record = ValueResult {
            type_: record_type.clone(),
            location: record_reg.render(),
            text: format!("partition({}, {})", collection.type_, action.text),
        };
        let record = self.free_intermediate_collection(matched_slot, &collection.type_, record)?;
        let record =
            self.free_intermediate_collection(unmatched_slot, &collection.type_, record)?;
        Ok(record)
    }

    /// plan-64 D2: native `collections::sortBy` for **8-byte fixed-width items**
    /// (Integer/Float/Fixed/Money) and **signed 8-byte keys** (Integer/Fixed/Money).
    /// Gated in the dispatch; String/Scalar/Byte/Float keys fall through to the
    /// `.mfb` `__collections_sortBy`. The `.mfb` version copies both whole lists
    /// (`MUT itemsDst = items`/`keysDst = keys`) every pass — pure waste, every slot
    /// is overwritten by the merge — over ⌈log₂n⌉ passes. This version allocates the
    /// two ping-pong buffer pairs once and swaps their pointers per pass. Stable
    /// bottom-up merge sort, taking the left run on ties, so the sorted order is
    /// byte-identical to the interpreted version.
    pub(super) fn lower_collection_sortby_call(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let collection = self.lower_value(&args[0])?;
        let item_type = list_element_type(&collection.type_)
            .ok_or_else(|| format!("native sortBy does not accept {}", collection.type_))?;
        // plan-86 A1: for a String item list the 8-byte merge cannot move the
        // variable-width payloads, so `gather` mode sorts an Integer index
        // permutation with the identical word-merge and gathers the Strings once
        // at the end (see the `if gather` blocks below). The dispatch gate only
        // routes String here when both args are re-eval-safe (the keys build
        // re-lowers them through `transform`).
        let gather = item_type == "String";
        let list_type = collection.type_.clone();
        let coll_slot = self.allocate_stack_object("sortby_coll", 8);
        self.emit(abi::store_u64(
            &collection.location,
            abi::stack_pointer(),
            coll_slot,
        ));
        let action = self.lower_value(&args[1])?;
        let key_type = callable_return_type(&action.type_).ok_or_else(|| {
            format!(
                "native sortBy keyFn must be a function, got {}",
                action.type_
            )
        })?;
        self.require_direct_callable("sortBy", &action)?;
        let action_slot = self.allocate_stack_object("sortby_action", 8);
        self.emit(abi::store_u64(
            &action.location,
            abi::stack_pointer(),
            action_slot,
        ));
        let keys_type = format!("List OF {key_type}");

        // n = count(collection).
        let n_slot = self.allocate_stack_object("sortby_n", 8);
        let r0 = self.temporary_vreg();
        let r1 = self.temporary_vreg();
        self.emit(abi::load_u64(&r0, abi::stack_pointer(), coll_slot));
        self.emit(abi::load_u64(&r1, &r0, COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64(&r1, abi::stack_pointer(), n_slot));

        let (keys_slot, items_slot, itemsb_slot, keysb_slot) = if gather {
            // plan-86 A1 gather mode (String items). keys is built by the native
            // `transform` lowering — a correctly-sized `List OF Integer` (data
            // region n*8), which the manual fixed-width fill below CANNOT be for a
            // String source (its data region is the string bytes, far smaller than
            // n*8). itemsB/keysB/items are then reserved FROM keys_slot (n*8), never
            // the String source. `transform` re-lowers args[0]/args[1]; the dispatch
            // gate only routes String sortBy here when both are re-eval-safe.
            let keys = self.lower_collection_transform_call(args)?;
            let keys_slot = self.allocate_stack_object("sortby_keys", 8);
            self.emit(abi::store_u64(&keys.location, abi::stack_pointer(), keys_slot));
            // items = the index permutation [0, 1, ..., n-1] the merge sorts.
            let items = self.lower_reserved_list(&keys_type, keys_slot)?;
            let items_slot = self.allocate_stack_object("sortby_items", 8);
            self.emit(abi::store_u64(&items.location, abi::stack_pointer(), items_slot));
            let gi_slot = self.allocate_stack_object("sortby_idxfill_i", 8);
            let gfill = self.label("sortby_idxfill");
            let gfill_done = self.label("sortby_idxfill_done");
            self.emit(abi::move_immediate(&r0, "Integer", "0"));
            self.emit(abi::store_u64(&r0, abi::stack_pointer(), gi_slot));
            self.emit(abi::label(&gfill));
            self.emit(abi::load_u64(&r0, abi::stack_pointer(), gi_slot));
            self.emit(abi::load_u64(&r1, abi::stack_pointer(), n_slot));
            self.emit(abi::compare_registers(&r0, &r1));
            self.emit(abi::branch_ge(&gfill_done));
            let gaddr = self.temporary_vreg();
            let goff = self.temporary_vreg();
            self.emit(abi::load_u64(&gaddr, abi::stack_pointer(), items_slot));
            self.emit(abi::add_immediate(&gaddr, &gaddr, COLLECTION_HEADER_SIZE));
            self.emit(abi::shift_left_immediate(&goff, &r0, 3));
            self.emit(abi::add_registers(&gaddr, &gaddr, &goff));
            self.emit(abi::store_u64(&r0, &gaddr, 0));
            self.emit(abi::add_immediate(&r0, &r0, 1));
            self.emit(abi::store_u64(&r0, abi::stack_pointer(), gi_slot));
            self.emit(abi::branch(&gfill));
            self.emit(abi::label(&gfill_done));
            // itemsB / keysB scratch, both List OF Integer sized n*8 (from keys_slot).
            let itemsb = self.lower_reserved_list(&keys_type, keys_slot)?;
            let itemsb_slot = self.allocate_stack_object("sortby_itemsb", 8);
            self.emit(abi::store_u64(&itemsb.location, abi::stack_pointer(), itemsb_slot));
            let keysb = self.lower_reserved_list(&keys_type, keys_slot)?;
            let keysb_slot = self.allocate_stack_object("sortby_keysb", 8);
            self.emit(abi::store_u64(&keysb.location, abi::stack_pointer(), keysb_slot));
            (keys_slot, items_slot, itemsb_slot, keysb_slot)
        } else {
            // keys = reserved List OF key_type (count 0, capacity n); fill by calling
            // keyFn on each source element, writing keys[i] directly.
            let keys = self.lower_reserved_list(&keys_type, coll_slot)?;
            let keys_slot = self.allocate_stack_object("sortby_keys", 8);
            self.emit(abi::store_u64(
                &keys.location,
                abi::stack_pointer(),
                keys_slot,
            ));

            let i_slot = self.allocate_stack_object("sortby_i", 8);
            let k_loop = self.label("sortby_keys_loop");
            let k_done = self.label("sortby_keys_done");
            let k_ok = self.label("sortby_keys_ok");
            self.emit(abi::move_immediate(&r0, "Integer", "0"));
            self.emit(abi::store_u64(&r0, abi::stack_pointer(), i_slot));
            self.emit(abi::label(&k_loop));
            self.emit(abi::load_u64(&r0, abi::stack_pointer(), i_slot));
            self.emit(abi::load_u64(&r1, abi::stack_pointer(), n_slot));
            self.emit(abi::compare_registers(&r0, &r1));
            self.emit(abi::branch_ge(&k_done));
            // item = source[i] : load_u64(collBase + HEADER + i*8).
            let addr = self.temporary_vreg();
            let off = self.temporary_vreg();
            self.emit(abi::load_u64(&addr, abi::stack_pointer(), coll_slot));
            self.emit(abi::add_immediate(&addr, &addr, COLLECTION_HEADER_SIZE));
            self.emit(abi::shift_left_immediate(&off, &r0, 3));
            self.emit(abi::add_registers(&addr, &addr, &off));
            let item = self.temporary_vreg();
            self.emit(abi::load_u64(&item, &addr, 0));
            self.emit(abi::move_register(&abi::argument_register(0)?, &item));
            let act = self.temporary_vreg();
            self.emit(abi::load_u64(&act, abi::stack_pointer(), action_slot));
            self.emit_direct_callable_branch(&act);
            self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
            self.emit(abi::branch_eq(&k_ok));
            // keyFn failed: free the partial keys buffer, propagate the raw error.
            self.emit_callback_failure_exit(Some((keys_slot, keys_type.clone())))?;
            self.emit(abi::label(&k_ok));
            // keys[i] = RESULT_VALUE_REGISTER.
            self.emit(abi::load_u64(&addr, abi::stack_pointer(), keys_slot));
            self.emit(abi::add_immediate(&addr, &addr, COLLECTION_HEADER_SIZE));
            self.emit(abi::load_u64(&r0, abi::stack_pointer(), i_slot));
            self.emit(abi::shift_left_immediate(&off, &r0, 3));
            self.emit(abi::add_registers(&addr, &addr, &off));
            self.emit(abi::store_u64(RESULT_VALUE_REGISTER, &addr, 0));
            self.emit(abi::add_immediate(&r0, &r0, 1));
            self.emit(abi::store_u64(&r0, abi::stack_pointer(), i_slot));
            self.emit(abi::branch(&k_loop));
            self.emit(abi::label(&k_done));

            // items = a tight, uniquely-owned copy of the source (count == n); two
            // scratch buffers (itemsB/keysB) for the ping-pong merge.
            let srcreg = self.temporary_vreg();
            self.emit(abi::load_u64(&srcreg, abi::stack_pointer(), coll_slot));
            let items_copy = self.copy_collection_tight(&list_type, &srcreg)?;
            let items_slot = self.allocate_stack_object("sortby_items", 8);
            self.emit(abi::store_u64(
                &items_copy,
                abi::stack_pointer(),
                items_slot,
            ));
            let itemsb = self.lower_reserved_list(&list_type, coll_slot)?;
            let itemsb_slot = self.allocate_stack_object("sortby_itemsb", 8);
            self.emit(abi::store_u64(
                &itemsb.location,
                abi::stack_pointer(),
                itemsb_slot,
            ));
            let keysb = self.lower_reserved_list(&keys_type, coll_slot)?;
            let keysb_slot = self.allocate_stack_object("sortby_keysb", 8);
            self.emit(abi::store_u64(
                &keysb.location,
                abi::stack_pointer(),
                keysb_slot,
            ));
            (keys_slot, items_slot, itemsb_slot, keysb_slot)
        };

        // --- Bottom-up stable merge sort, ping-ponging the four buffer slots. ---
        let width_slot = self.allocate_stack_object("sortby_width", 8);
        let lo_slot = self.allocate_stack_object("sortby_lo", 8);
        let outer = self.label("sortby_outer");
        let outer_done = self.label("sortby_outer_done");
        let mid_loop = self.label("sortby_mid_loop");
        let mid_done = self.label("sortby_mid_done");
        let merge_loop = self.label("sortby_merge_loop");
        let merge_end = self.label("sortby_merge_end");
        let take_j = self.label("sortby_take_j");
        let after_take = self.label("sortby_after_take");
        let copy_i = self.label("sortby_copy_i");
        let copy_i_done = self.label("sortby_copy_i_done");
        let copy_j = self.label("sortby_copy_j");
        let copy_j_done = self.label("sortby_copy_j_done");

        // Pass geometry / base pointers (loaded fresh each pass; no calls here so
        // they stay in vregs). width/lo persist through the inner loops via slots.
        let its = self.temporary_vreg(); // itemsSrc base (+HEADER)
        let kys = self.temporary_vreg(); // keysSrc base
        let itd = self.temporary_vreg(); // itemsDst base
        let kyd = self.temporary_vreg(); // keysDst base
        let width = self.temporary_vreg();
        let n = self.temporary_vreg();
        let lo = self.temporary_vreg();
        let mid = self.temporary_vreg();
        let hi = self.temporary_vreg();
        let ii = self.temporary_vreg();
        let jj = self.temporary_vreg();
        let kk = self.temporary_vreg();
        let ki = self.temporary_vreg();
        let kj = self.temporary_vreg();
        let vv = self.temporary_vreg();
        let t0 = self.temporary_vreg();
        let t1 = self.temporary_vreg();

        self.emit(abi::move_immediate(&width, "Integer", "1"));
        self.emit(abi::store_u64(&width, abi::stack_pointer(), width_slot));
        self.emit(abi::label(&outer));
        self.emit(abi::load_u64(&width, abi::stack_pointer(), width_slot));
        self.emit(abi::load_u64(&n, abi::stack_pointer(), n_slot));
        self.emit(abi::compare_registers(&width, &n));
        self.emit(abi::branch_ge(&outer_done));
        // base pointers for this pass
        self.emit(abi::load_u64(&its, abi::stack_pointer(), items_slot));
        self.emit(abi::add_immediate(&its, &its, COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64(&kys, abi::stack_pointer(), keys_slot));
        self.emit(abi::add_immediate(&kys, &kys, COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64(&itd, abi::stack_pointer(), itemsb_slot));
        self.emit(abi::add_immediate(&itd, &itd, COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64(&kyd, abi::stack_pointer(), keysb_slot));
        self.emit(abi::add_immediate(&kyd, &kyd, COLLECTION_HEADER_SIZE));
        self.emit(abi::move_immediate(&lo, "Integer", "0"));
        self.emit(abi::store_u64(&lo, abi::stack_pointer(), lo_slot));
        self.emit(abi::label(&mid_loop));
        self.emit(abi::load_u64(&lo, abi::stack_pointer(), lo_slot));
        self.emit(abi::load_u64(&n, abi::stack_pointer(), n_slot));
        self.emit(abi::compare_registers(&lo, &n));
        self.emit(abi::branch_ge(&mid_done));
        self.emit(abi::load_u64(&width, abi::stack_pointer(), width_slot));
        // mid = min(lo + width, n)
        self.emit(abi::add_registers(&mid, &lo, &width));
        let mid_ok = self.label("sortby_mid_clamp_ok");
        self.emit(abi::compare_registers(&mid, &n));
        self.emit(abi::branch_le(&mid_ok));
        self.emit(abi::move_register(&mid, &n));
        self.emit(abi::label(&mid_ok));
        // hi = min(lo + 2*width, n) == min(mid + width, n)
        self.emit(abi::add_registers(&hi, &mid, &width));
        let hi_ok = self.label("sortby_hi_clamp_ok");
        self.emit(abi::compare_registers(&hi, &n));
        self.emit(abi::branch_le(&hi_ok));
        self.emit(abi::move_register(&hi, &n));
        self.emit(abi::label(&hi_ok));
        // i = lo; j = mid; k = lo
        self.emit(abi::move_register(&ii, &lo));
        self.emit(abi::move_register(&jj, &mid));
        self.emit(abi::move_register(&kk, &lo));
        // while i < mid AND j < hi: take the smaller key (left run on ties = stable).
        self.emit(abi::label(&merge_loop));
        self.emit(abi::compare_registers(&ii, &mid));
        self.emit(abi::branch_ge(&merge_end));
        self.emit(abi::compare_registers(&jj, &hi));
        self.emit(abi::branch_ge(&merge_end));
        self.emit(abi::shift_left_immediate(&t0, &ii, 3));
        self.emit(abi::add_registers(&t1, &kys, &t0));
        self.emit(abi::load_u64(&ki, &t1, 0));
        self.emit(abi::shift_left_immediate(&t0, &jj, 3));
        self.emit(abi::add_registers(&t1, &kys, &t0));
        self.emit(abi::load_u64(&kj, &t1, 0));
        self.emit(abi::compare_registers(&kj, &ki));
        self.emit(abi::branch_lt(&take_j));
        // take i: itemsDst[k] = itemsSrc[i]; keysDst[k] = keysSrc[i]
        self.emit(abi::shift_left_immediate(&t0, &ii, 3));
        self.emit(abi::add_registers(&t1, &its, &t0));
        self.emit(abi::load_u64(&vv, &t1, 0));
        self.emit(abi::shift_left_immediate(&t0, &kk, 3));
        self.emit(abi::add_registers(&t1, &itd, &t0));
        self.emit(abi::store_u64(&vv, &t1, 0));
        self.emit(abi::shift_left_immediate(&t0, &kk, 3));
        self.emit(abi::add_registers(&t1, &kyd, &t0));
        self.emit(abi::store_u64(&ki, &t1, 0));
        self.emit(abi::add_immediate(&ii, &ii, 1));
        self.emit(abi::branch(&after_take));
        self.emit(abi::label(&take_j));
        // take j: itemsDst[k] = itemsSrc[j]; keysDst[k] = keysSrc[j]
        self.emit(abi::shift_left_immediate(&t0, &jj, 3));
        self.emit(abi::add_registers(&t1, &its, &t0));
        self.emit(abi::load_u64(&vv, &t1, 0));
        self.emit(abi::shift_left_immediate(&t0, &kk, 3));
        self.emit(abi::add_registers(&t1, &itd, &t0));
        self.emit(abi::store_u64(&vv, &t1, 0));
        self.emit(abi::shift_left_immediate(&t0, &kk, 3));
        self.emit(abi::add_registers(&t1, &kyd, &t0));
        self.emit(abi::store_u64(&kj, &t1, 0));
        self.emit(abi::add_immediate(&jj, &jj, 1));
        self.emit(abi::label(&after_take));
        self.emit(abi::add_immediate(&kk, &kk, 1));
        self.emit(abi::branch(&merge_loop));
        self.emit(abi::label(&merge_end));
        // copy the remaining left run (while i < mid)
        self.emit(abi::label(&copy_i));
        self.emit(abi::compare_registers(&ii, &mid));
        self.emit(abi::branch_ge(&copy_i_done));
        self.emit(abi::shift_left_immediate(&t0, &ii, 3));
        self.emit(abi::add_registers(&t1, &its, &t0));
        self.emit(abi::load_u64(&vv, &t1, 0));
        self.emit(abi::shift_left_immediate(&t0, &kk, 3));
        self.emit(abi::add_registers(&t1, &itd, &t0));
        self.emit(abi::store_u64(&vv, &t1, 0));
        self.emit(abi::shift_left_immediate(&t0, &ii, 3));
        self.emit(abi::add_registers(&t1, &kys, &t0));
        self.emit(abi::load_u64(&vv, &t1, 0));
        self.emit(abi::shift_left_immediate(&t0, &kk, 3));
        self.emit(abi::add_registers(&t1, &kyd, &t0));
        self.emit(abi::store_u64(&vv, &t1, 0));
        self.emit(abi::add_immediate(&ii, &ii, 1));
        self.emit(abi::add_immediate(&kk, &kk, 1));
        self.emit(abi::branch(&copy_i));
        self.emit(abi::label(&copy_i_done));
        // copy the remaining right run (while j < hi)
        self.emit(abi::label(&copy_j));
        self.emit(abi::compare_registers(&jj, &hi));
        self.emit(abi::branch_ge(&copy_j_done));
        self.emit(abi::shift_left_immediate(&t0, &jj, 3));
        self.emit(abi::add_registers(&t1, &its, &t0));
        self.emit(abi::load_u64(&vv, &t1, 0));
        self.emit(abi::shift_left_immediate(&t0, &kk, 3));
        self.emit(abi::add_registers(&t1, &itd, &t0));
        self.emit(abi::store_u64(&vv, &t1, 0));
        self.emit(abi::shift_left_immediate(&t0, &jj, 3));
        self.emit(abi::add_registers(&t1, &kys, &t0));
        self.emit(abi::load_u64(&vv, &t1, 0));
        self.emit(abi::shift_left_immediate(&t0, &kk, 3));
        self.emit(abi::add_registers(&t1, &kyd, &t0));
        self.emit(abi::store_u64(&vv, &t1, 0));
        self.emit(abi::add_immediate(&jj, &jj, 1));
        self.emit(abi::add_immediate(&kk, &kk, 1));
        self.emit(abi::branch(&copy_j));
        self.emit(abi::label(&copy_j_done));
        // lo += 2*width
        self.emit(abi::load_u64(&lo, abi::stack_pointer(), lo_slot));
        self.emit(abi::load_u64(&width, abi::stack_pointer(), width_slot));
        self.emit(abi::add_registers(&lo, &lo, &width));
        self.emit(abi::add_registers(&lo, &lo, &width));
        self.emit(abi::store_u64(&lo, abi::stack_pointer(), lo_slot));
        self.emit(abi::branch(&mid_loop));
        self.emit(abi::label(&mid_done));
        // swap the buffer-pointer slots: items <-> itemsB, keys <-> keysB.
        self.emit(abi::load_u64(&t0, abi::stack_pointer(), items_slot));
        self.emit(abi::load_u64(&t1, abi::stack_pointer(), itemsb_slot));
        self.emit(abi::store_u64(&t1, abi::stack_pointer(), items_slot));
        self.emit(abi::store_u64(&t0, abi::stack_pointer(), itemsb_slot));
        self.emit(abi::load_u64(&t0, abi::stack_pointer(), keys_slot));
        self.emit(abi::load_u64(&t1, abi::stack_pointer(), keysb_slot));
        self.emit(abi::store_u64(&t1, abi::stack_pointer(), keys_slot));
        self.emit(abi::store_u64(&t0, abi::stack_pointer(), keysb_slot));
        // width *= 2
        self.emit(abi::load_u64(&width, abi::stack_pointer(), width_slot));
        self.emit(abi::add_registers(&width, &width, &width));
        self.emit(abi::store_u64(&width, abi::stack_pointer(), width_slot));
        self.emit(abi::branch(&outer));
        self.emit(abi::label(&outer_done));

        if gather {
            // plan-86 A1 gather finalize: items_slot now holds the sorted index
            // permutation. Build the String result by copying source[idx] into it
            // in sorted order, then free the four Integer index buffers. The result
            // is pre-sized to the source (same n entries, same total bytes — a
            // permutation), so no append regrows.
            let result = self.lower_reserved_list(&list_type, coll_slot)?;
            let result_slot = self.allocate_stack_object("sortby_result", 8);
            self.emit(abi::store_u64(&result.location, abi::stack_pointer(), result_slot));
            let gk_slot = self.allocate_stack_object("sortby_gather_k", 8);
            let gitem_slot = self.allocate_stack_object("sortby_gather_item", 8);
            let gloop = self.label("sortby_gather_loop");
            let gdone = self.label("sortby_gather_done");
            self.emit(abi::move_immediate(&r0, "Integer", "0"));
            self.emit(abi::store_u64(&r0, abi::stack_pointer(), gk_slot));
            self.emit(abi::label(&gloop));
            self.emit(abi::load_u64(&r0, abi::stack_pointer(), gk_slot));
            self.emit(abi::load_u64(&r1, abi::stack_pointer(), n_slot));
            self.emit(abi::compare_registers(&r0, &r1));
            self.emit(abi::branch_ge(&gdone));
            // idx = items[k] (word load from the sorted permutation buffer).
            let gaddr = self.temporary_vreg();
            let goff = self.temporary_vreg();
            let gidx = self.temporary_vreg();
            self.emit(abi::load_u64(&gaddr, abi::stack_pointer(), items_slot));
            self.emit(abi::add_immediate(&gaddr, &gaddr, COLLECTION_HEADER_SIZE));
            self.emit(abi::shift_left_immediate(&goff, &r0, 3));
            self.emit(abi::add_registers(&gaddr, &gaddr, &goff));
            self.emit(abi::load_u64(&gidx, &gaddr, 0));
            // (voff, vlen) = source entry[idx]; materialize an owned String from
            // the source data region, append (copies the bytes), then free it.
            let gvoff = self.temporary_vreg();
            let gvlen = self.temporary_vreg();
            let gscr1 = self.temporary_vreg();
            let gscr2 = self.temporary_vreg();
            let gcoll = self.temporary_vreg();
            self.emit(abi::load_u64(&gcoll, abi::stack_pointer(), coll_slot));
            self.emit_element_value_offset(&gvoff, &gvlen, &gcoll, &gidx, &gscr1, &gscr2, "String");
            let gitem = self.emit_load_collection_payload("String", &gcoll, &gvoff, &gvlen)?;
            self.emit(abi::store_u64(&gitem, abi::stack_pointer(), gitem_slot));
            self.lower_list_append_in_place(result_slot, gitem_slot, &list_type, "String")?;
            self.free_collection_loop_item(gitem_slot, "String")?;
            self.emit(abi::load_u64(&r0, abi::stack_pointer(), gk_slot));
            self.emit(abi::add_immediate(&r0, &r0, 1));
            self.emit(abi::store_u64(&r0, abi::stack_pointer(), gk_slot));
            self.emit(abi::branch(&gloop));
            self.emit(abi::label(&gdone));
            let result_reg = self.allocate_register()?;
            self.emit(abi::load_u64(&result_reg, abi::stack_pointer(), result_slot));
            let threaded = ValueResult {
                type_: list_type.clone(),
                location: result_reg.render(),
                text: String::new(),
            };
            let threaded = self.free_intermediate_collection(items_slot, &keys_type, threaded)?;
            let threaded = self.free_intermediate_collection(itemsb_slot, &keys_type, threaded)?;
            let threaded = self.free_intermediate_collection(keys_slot, &keys_type, threaded)?;
            let threaded = self.free_intermediate_collection(keysb_slot, &keys_type, threaded)?;
            return Ok(ValueResult {
                type_: list_type.clone(),
                location: threaded.location,
                text: format!("sortBy({list_type})"),
            });
        }

        // The sorted data is in items_slot (each pass swaps its output back into it).
        // Scratch buffers start count 0, so stamp count = n, dataLength = n*8 to make
        // the returned block a valid list, then free the three unused buffers.
        self.emit(abi::load_u64(&r0, abi::stack_pointer(), items_slot));
        self.emit(abi::load_u64(&n, abi::stack_pointer(), n_slot));
        self.emit(abi::store_u64(&n, &r0, COLLECTION_OFFSET_COUNT));
        self.emit(abi::shift_left_immediate(&t0, &n, 3));
        self.emit(abi::store_u64(&t0, &r0, COLLECTION_OFFSET_DATA_LENGTH));

        let result_reg = self.allocate_register()?;
        self.emit(abi::load_u64(&result_reg, abi::stack_pointer(), items_slot));
        let threaded = ValueResult {
            type_: list_type.clone(),
            location: result_reg.render(),
            text: String::new(),
        };
        let threaded = self.free_intermediate_collection(itemsb_slot, &list_type, threaded)?;
        let threaded = self.free_intermediate_collection(keys_slot, &keys_type, threaded)?;
        let threaded = self.free_intermediate_collection(keysb_slot, &keys_type, threaded)?;
        Ok(ValueResult {
            type_: list_type.clone(),
            location: threaded.location,
            text: format!("sortBy({list_type})"),
        })
    }

    /// plan-86 A1: reserve a `List OF Integer` with exactly `n` slots (data region
    /// `n*8`), sized from the runtime count in `n_slot` rather than a source
    /// collection's data region. `lower_reserved_list` sizes the data region from a
    /// source's `dataLength`, which for a String source is the string bytes — far
    /// smaller than the `n*8` an index-permutation buffer needs — so the native
    /// index sorts allocate their buffers through here. Header stamped count=n,
    /// capacity=n, dataLength=n*8, dataCapacity=n*8 (a well-formed fixed-width list).
    pub(super) fn reserve_integer_index_list(
        &mut self,
        n_slot: usize,
    ) -> Result<ValueResult, String> {
        let layout = CollectionTypeLayout::from_type("List OF Integer")
            .ok_or_else(|| "native index-list layout unavailable".to_string())?;
        let n = self.temporary_vreg();
        let eight = self.temporary_vreg();
        let bytes = self.temporary_vreg();
        let result_slot = self.allocate_stack_object("index_list_result", 8);
        let overflow = self.label("index_list_overflow");
        let alloc_ok = self.label("index_list_alloc_ok");
        // bytes = n*8 (checked); size = HEADER + bytes (checked).
        self.emit(abi::load_u64(&n, abi::stack_pointer(), n_slot));
        self.emit(abi::move_immediate(&eight, "Integer", "8"));
        self.emit_checked_size_multiply(&bytes, &n, &eight, &overflow);
        self.emit_checked_size_add_immediate(
            abi::return_register(),
            &bytes,
            COLLECTION_HEADER_SIZE,
            &overflow,
        );
        self.emit(abi::move_immediate(abi::ARG[1], "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&overflow));
        self.emit_error_code_return(ERR_OUT_OF_MEMORY_CODE, ERR_ALLOCATION_MESSAGE)?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(abi::RET[1], abi::stack_pointer(), result_slot));
        // Header: count = capacity = n; dataLength = dataCapacity = n*8.
        let base = self.temporary_vreg();
        let nn = self.temporary_vreg();
        let bb = self.temporary_vreg();
        self.emit(abi::load_u64(&base, abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64(&nn, abi::stack_pointer(), n_slot));
        self.emit(abi::shift_left_immediate(&bb, &nn, 3));
        self.emit_write_collection_header_full(&layout, &base, &nn, &nn, &bb, &bb);
        let reg = self.allocate_register()?;
        self.emit(abi::load_u64(&reg, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: "List OF Integer".to_string(),
            location: reg.render(),
            text: "index-list".to_string(),
        })
    }

    /// plan-86 A1: native `collections::sort` for a String item list. `sort` has no
    /// key function, so the merge compares the source Strings **lexicographically**
    /// (unsigned bytes; a prefix is smaller) instead of an 8-byte key. As with
    /// `sortBy`, the 8-byte merge cannot move variable-width payloads, so this sorts
    /// an Integer index permutation `[0..n)` — stable bottom-up merge, taking the
    /// left run on ties — and gathers `source[idx]` once at the end. Only String is
    /// routed here (fixed-width `sort` has no native path today either); the gate
    /// requires a re-eval-safe source.
    pub(super) fn lower_collection_sort_call(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let source = self.lower_value(&args[0])?;
        let list_type = source.type_.clone();
        let elem = list_element_type(&list_type)
            .ok_or_else(|| format!("native sort does not accept {list_type}"))?;
        if elem != "String" {
            return Err(format!("native sort only supports String items, got {elem}"));
        }
        let coll_slot = self.allocate_stack_object("sort_coll", 8);
        self.emit(abi::store_u64(&source.location, abi::stack_pointer(), coll_slot));
        let n_slot = self.allocate_stack_object("sort_n", 8);
        let r0 = self.temporary_vreg();
        let r1 = self.temporary_vreg();
        self.emit(abi::load_u64(&r0, abi::stack_pointer(), coll_slot));
        self.emit(abi::load_u64(&r1, &r0, COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64(&r1, abi::stack_pointer(), n_slot));

        // items = [0..n) permutation; itemsB = ping-pong scratch. Both List OF
        // Integer sized n*8 (count-based).
        let items = self.reserve_integer_index_list(n_slot)?;
        let items_slot = self.allocate_stack_object("sort_items", 8);
        self.emit(abi::store_u64(&items.location, abi::stack_pointer(), items_slot));
        let fi_slot = self.allocate_stack_object("sort_fill_i", 8);
        let fl = self.label("sort_fill");
        let fl_done = self.label("sort_fill_done");
        self.emit(abi::move_immediate(&r0, "Integer", "0"));
        self.emit(abi::store_u64(&r0, abi::stack_pointer(), fi_slot));
        self.emit(abi::label(&fl));
        self.emit(abi::load_u64(&r0, abi::stack_pointer(), fi_slot));
        self.emit(abi::load_u64(&r1, abi::stack_pointer(), n_slot));
        self.emit(abi::compare_registers(&r0, &r1));
        self.emit(abi::branch_ge(&fl_done));
        let fa = self.temporary_vreg();
        let fo = self.temporary_vreg();
        self.emit(abi::load_u64(&fa, abi::stack_pointer(), items_slot));
        self.emit(abi::add_immediate(&fa, &fa, COLLECTION_HEADER_SIZE));
        self.emit(abi::shift_left_immediate(&fo, &r0, 3));
        self.emit(abi::add_registers(&fa, &fa, &fo));
        self.emit(abi::store_u64(&r0, &fa, 0));
        self.emit(abi::add_immediate(&r0, &r0, 1));
        self.emit(abi::store_u64(&r0, abi::stack_pointer(), fi_slot));
        self.emit(abi::branch(&fl));
        self.emit(abi::label(&fl_done));
        let itemsb = self.reserve_integer_index_list(n_slot)?;
        let itemsb_slot = self.allocate_stack_object("sort_itemsb", 8);
        self.emit(abi::store_u64(&itemsb.location, abi::stack_pointer(), itemsb_slot));

        // --- Bottom-up stable merge over the index buffer; the compare is a
        //     lexicographic byte compare of the two source Strings. ---
        let width_slot = self.allocate_stack_object("sort_width", 8);
        let lo_slot = self.allocate_stack_object("sort_lo", 8);
        let outer = self.label("sort_outer");
        let outer_done = self.label("sort_outer_done");
        let mid_loop = self.label("sort_mid_loop");
        let mid_done = self.label("sort_mid_done");
        let merge_loop = self.label("sort_merge_loop");
        let merge_end = self.label("sort_merge_end");
        let take_j = self.label("sort_take_j");
        let take_i = self.label("sort_take_i");
        let after_take = self.label("sort_after_take");
        let copy_i = self.label("sort_copy_i");
        let copy_i_done = self.label("sort_copy_i_done");
        let copy_j = self.label("sort_copy_j");
        let copy_j_done = self.label("sort_copy_j_done");

        let its = self.temporary_vreg(); // itemsSrc base (+HEADER)
        let itd = self.temporary_vreg(); // itemsDst base (+HEADER)
        let width = self.temporary_vreg();
        let n = self.temporary_vreg();
        let lo = self.temporary_vreg();
        let mid = self.temporary_vreg();
        let hi = self.temporary_vreg();
        let ii = self.temporary_vreg();
        let jj = self.temporary_vreg();
        let kk = self.temporary_vreg();
        let vv = self.temporary_vreg();
        let t0 = self.temporary_vreg();
        let t1 = self.temporary_vreg();

        self.emit(abi::move_immediate(&width, "Integer", "1"));
        self.emit(abi::store_u64(&width, abi::stack_pointer(), width_slot));
        self.emit(abi::label(&outer));
        self.emit(abi::load_u64(&width, abi::stack_pointer(), width_slot));
        self.emit(abi::load_u64(&n, abi::stack_pointer(), n_slot));
        self.emit(abi::compare_registers(&width, &n));
        self.emit(abi::branch_ge(&outer_done));
        self.emit(abi::load_u64(&its, abi::stack_pointer(), items_slot));
        self.emit(abi::add_immediate(&its, &its, COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64(&itd, abi::stack_pointer(), itemsb_slot));
        self.emit(abi::add_immediate(&itd, &itd, COLLECTION_HEADER_SIZE));
        self.emit(abi::move_immediate(&lo, "Integer", "0"));
        self.emit(abi::store_u64(&lo, abi::stack_pointer(), lo_slot));
        self.emit(abi::label(&mid_loop));
        self.emit(abi::load_u64(&lo, abi::stack_pointer(), lo_slot));
        self.emit(abi::load_u64(&n, abi::stack_pointer(), n_slot));
        self.emit(abi::compare_registers(&lo, &n));
        self.emit(abi::branch_ge(&mid_done));
        self.emit(abi::load_u64(&width, abi::stack_pointer(), width_slot));
        self.emit(abi::add_registers(&mid, &lo, &width));
        let mid_ok = self.label("sort_mid_clamp_ok");
        self.emit(abi::compare_registers(&mid, &n));
        self.emit(abi::branch_le(&mid_ok));
        self.emit(abi::move_register(&mid, &n));
        self.emit(abi::label(&mid_ok));
        self.emit(abi::add_registers(&hi, &mid, &width));
        let hi_ok = self.label("sort_hi_clamp_ok");
        self.emit(abi::compare_registers(&hi, &n));
        self.emit(abi::branch_le(&hi_ok));
        self.emit(abi::move_register(&hi, &n));
        self.emit(abi::label(&hi_ok));
        self.emit(abi::move_register(&ii, &lo));
        self.emit(abi::move_register(&jj, &mid));
        self.emit(abi::move_register(&kk, &lo));
        self.emit(abi::label(&merge_loop));
        self.emit(abi::compare_registers(&ii, &mid));
        self.emit(abi::branch_ge(&merge_end));
        self.emit(abi::compare_registers(&jj, &hi));
        self.emit(abi::branch_ge(&merge_end));
        // Decide take_j iff source[its[jj]] < source[its[ii]] lexicographically.
        self.emit_index_string_less_branch(coll_slot, &its, &ii, &jj, &take_j, &take_i);
        self.emit(abi::label(&take_i));
        // itemsDst[kk] = itemsSrc[ii]
        self.emit(abi::shift_left_immediate(&t0, &ii, 3));
        self.emit(abi::add_registers(&t1, &its, &t0));
        self.emit(abi::load_u64(&vv, &t1, 0));
        self.emit(abi::shift_left_immediate(&t0, &kk, 3));
        self.emit(abi::add_registers(&t1, &itd, &t0));
        self.emit(abi::store_u64(&vv, &t1, 0));
        self.emit(abi::add_immediate(&ii, &ii, 1));
        self.emit(abi::branch(&after_take));
        self.emit(abi::label(&take_j));
        // itemsDst[kk] = itemsSrc[jj]
        self.emit(abi::shift_left_immediate(&t0, &jj, 3));
        self.emit(abi::add_registers(&t1, &its, &t0));
        self.emit(abi::load_u64(&vv, &t1, 0));
        self.emit(abi::shift_left_immediate(&t0, &kk, 3));
        self.emit(abi::add_registers(&t1, &itd, &t0));
        self.emit(abi::store_u64(&vv, &t1, 0));
        self.emit(abi::add_immediate(&jj, &jj, 1));
        self.emit(abi::label(&after_take));
        self.emit(abi::add_immediate(&kk, &kk, 1));
        self.emit(abi::branch(&merge_loop));
        self.emit(abi::label(&merge_end));
        // copy remaining left run
        self.emit(abi::label(&copy_i));
        self.emit(abi::compare_registers(&ii, &mid));
        self.emit(abi::branch_ge(&copy_i_done));
        self.emit(abi::shift_left_immediate(&t0, &ii, 3));
        self.emit(abi::add_registers(&t1, &its, &t0));
        self.emit(abi::load_u64(&vv, &t1, 0));
        self.emit(abi::shift_left_immediate(&t0, &kk, 3));
        self.emit(abi::add_registers(&t1, &itd, &t0));
        self.emit(abi::store_u64(&vv, &t1, 0));
        self.emit(abi::add_immediate(&ii, &ii, 1));
        self.emit(abi::add_immediate(&kk, &kk, 1));
        self.emit(abi::branch(&copy_i));
        self.emit(abi::label(&copy_i_done));
        // copy remaining right run
        self.emit(abi::label(&copy_j));
        self.emit(abi::compare_registers(&jj, &hi));
        self.emit(abi::branch_ge(&copy_j_done));
        self.emit(abi::shift_left_immediate(&t0, &jj, 3));
        self.emit(abi::add_registers(&t1, &its, &t0));
        self.emit(abi::load_u64(&vv, &t1, 0));
        self.emit(abi::shift_left_immediate(&t0, &kk, 3));
        self.emit(abi::add_registers(&t1, &itd, &t0));
        self.emit(abi::store_u64(&vv, &t1, 0));
        self.emit(abi::add_immediate(&jj, &jj, 1));
        self.emit(abi::add_immediate(&kk, &kk, 1));
        self.emit(abi::branch(&copy_j));
        self.emit(abi::label(&copy_j_done));
        self.emit(abi::load_u64(&lo, abi::stack_pointer(), lo_slot));
        self.emit(abi::load_u64(&width, abi::stack_pointer(), width_slot));
        self.emit(abi::add_registers(&lo, &lo, &width));
        self.emit(abi::add_registers(&lo, &lo, &width));
        self.emit(abi::store_u64(&lo, abi::stack_pointer(), lo_slot));
        self.emit(abi::branch(&mid_loop));
        self.emit(abi::label(&mid_done));
        // swap items <-> itemsB
        self.emit(abi::load_u64(&t0, abi::stack_pointer(), items_slot));
        self.emit(abi::load_u64(&t1, abi::stack_pointer(), itemsb_slot));
        self.emit(abi::store_u64(&t1, abi::stack_pointer(), items_slot));
        self.emit(abi::store_u64(&t0, abi::stack_pointer(), itemsb_slot));
        self.emit(abi::load_u64(&width, abi::stack_pointer(), width_slot));
        self.emit(abi::add_registers(&width, &width, &width));
        self.emit(abi::store_u64(&width, abi::stack_pointer(), width_slot));
        self.emit(abi::branch(&outer));
        self.emit(abi::label(&outer_done));

        // Gather: items_slot holds the sorted index permutation; build the String
        // result by copying source[idx] in order, then free the two index buffers.
        let result = self.lower_reserved_list(&list_type, coll_slot)?;
        let result_slot = self.allocate_stack_object("sort_result", 8);
        self.emit(abi::store_u64(&result.location, abi::stack_pointer(), result_slot));
        let gk_slot = self.allocate_stack_object("sort_gather_k", 8);
        let gitem_slot = self.allocate_stack_object("sort_gather_item", 8);
        let gloop = self.label("sort_gather_loop");
        let gdone = self.label("sort_gather_done");
        self.emit(abi::move_immediate(&r0, "Integer", "0"));
        self.emit(abi::store_u64(&r0, abi::stack_pointer(), gk_slot));
        self.emit(abi::label(&gloop));
        self.emit(abi::load_u64(&r0, abi::stack_pointer(), gk_slot));
        self.emit(abi::load_u64(&r1, abi::stack_pointer(), n_slot));
        self.emit(abi::compare_registers(&r0, &r1));
        self.emit(abi::branch_ge(&gdone));
        let gaddr = self.temporary_vreg();
        let goff = self.temporary_vreg();
        let gidx = self.temporary_vreg();
        self.emit(abi::load_u64(&gaddr, abi::stack_pointer(), items_slot));
        self.emit(abi::add_immediate(&gaddr, &gaddr, COLLECTION_HEADER_SIZE));
        self.emit(abi::shift_left_immediate(&goff, &r0, 3));
        self.emit(abi::add_registers(&gaddr, &gaddr, &goff));
        self.emit(abi::load_u64(&gidx, &gaddr, 0));
        let gvoff = self.temporary_vreg();
        let gvlen = self.temporary_vreg();
        let gscr1 = self.temporary_vreg();
        let gscr2 = self.temporary_vreg();
        let gcoll = self.temporary_vreg();
        self.emit(abi::load_u64(&gcoll, abi::stack_pointer(), coll_slot));
        self.emit_element_value_offset(&gvoff, &gvlen, &gcoll, &gidx, &gscr1, &gscr2, "String");
        let gitem = self.emit_load_collection_payload("String", &gcoll, &gvoff, &gvlen)?;
        self.emit(abi::store_u64(&gitem, abi::stack_pointer(), gitem_slot));
        self.lower_list_append_in_place(result_slot, gitem_slot, &list_type, "String")?;
        self.free_collection_loop_item(gitem_slot, "String")?;
        self.emit(abi::load_u64(&r0, abi::stack_pointer(), gk_slot));
        self.emit(abi::add_immediate(&r0, &r0, 1));
        self.emit(abi::store_u64(&r0, abi::stack_pointer(), gk_slot));
        self.emit(abi::branch(&gloop));
        self.emit(abi::label(&gdone));
        let result_reg = self.allocate_register()?;
        self.emit(abi::load_u64(&result_reg, abi::stack_pointer(), result_slot));
        let threaded = ValueResult {
            type_: list_type.clone(),
            location: result_reg.render(),
            text: String::new(),
        };
        let idx_type = "List OF Integer".to_string();
        let threaded = self.free_intermediate_collection(items_slot, &idx_type, threaded)?;
        let threaded = self.free_intermediate_collection(itemsb_slot, &idx_type, threaded)?;
        Ok(ValueResult {
            type_: list_type.clone(),
            location: threaded.location,
            text: format!("sort({list_type})"),
        })
    }

    /// plan-86 A1: branch to `less_label` iff the source String at index `its[ii]`
    /// is lexicographically greater than the one at `its[jj]` — i.e. the right run
    /// element (jj) is strictly smaller, so the stable merge takes it. Bytes are
    /// unsigned; a proper prefix is the smaller string; equal keys fall to
    /// `ge_label` (take the left run, preserving stability). No calls, so the many
    /// scratch registers here survive the whole compare.
    fn emit_index_string_less_branch(
        &mut self,
        coll_slot: usize,
        its: &VirtualRegister,
        ii: &VirtualRegister,
        jj: &VirtualRegister,
        less_label: &str,
        ge_label: &str,
    ) {
        let sb = self.temporary_vreg();
        let db = self.temporary_vreg();
        let idx_i = self.temporary_vreg();
        let idx_j = self.temporary_vreg();
        let voff_i = self.temporary_vreg();
        let vlen_i = self.temporary_vreg();
        let voff_j = self.temporary_vreg();
        let vlen_j = self.temporary_vreg();
        let sc1 = self.temporary_vreg();
        let sc2 = self.temporary_vreg();
        let ptr_i = self.temporary_vreg();
        let ptr_j = self.temporary_vreg();
        let minlen = self.temporary_vreg();
        let lbyte = self.temporary_vreg();
        let rbyte = self.temporary_vreg();
        let off = self.temporary_vreg();
        // idxI = its[ii], idxJ = its[jj]
        self.emit(abi::shift_left_immediate(&off, ii, 3));
        self.emit(abi::add_registers(&sc1, its, &off));
        self.emit(abi::load_u64(&idx_i, &sc1, 0));
        self.emit(abi::shift_left_immediate(&off, jj, 3));
        self.emit(abi::add_registers(&sc1, its, &off));
        self.emit(abi::load_u64(&idx_j, &sc1, 0));
        // source base + data base
        self.emit(abi::load_u64(&sb, abi::stack_pointer(), coll_slot));
        self.emit_collection_data_pointer_for(&db, &sb, "String");
        // (voffI, vlenI) and (voffJ, vlenJ)
        self.emit_element_value_offset(&voff_i, &vlen_i, &sb, &idx_i, &sc1, &sc2, "String");
        self.emit_element_value_offset(&voff_j, &vlen_j, &sb, &idx_j, &sc1, &sc2, "String");
        self.emit(abi::add_registers(&ptr_i, &db, &voff_i));
        self.emit(abi::add_registers(&ptr_j, &db, &voff_j));
        // minlen = min(vlenI, vlenJ)
        let min_done = self.label("sort_cmp_min_done");
        self.emit(abi::move_register(&minlen, &vlen_i));
        self.emit(abi::compare_registers(&vlen_j, &vlen_i));
        self.emit(abi::branch_ge(&min_done));
        self.emit(abi::move_register(&minlen, &vlen_j));
        self.emit(abi::label(&min_done));
        // byte-compare J vs I over minlen. Advances ptr_j/ptr_i/minlen.
        let cmp_loop = self.label("sort_cmp_loop");
        let cmp_eq = self.label("sort_cmp_prefix_eq");
        let cmp_ne = self.label("sort_cmp_byte_ne");
        self.emit_byte_compare_loop(
            &ptr_j, &ptr_i, &minlen, &lbyte, &rbyte, &cmp_loop, &cmp_eq, &cmp_ne,
        );
        // First differing byte: J < I iff lbyte(J) < rbyte(I).
        self.emit(abi::label(&cmp_ne));
        self.emit(abi::compare_registers(&lbyte, &rbyte));
        self.emit(abi::branch_lt(less_label));
        self.emit(abi::branch(ge_label));
        // Prefix equal: J < I iff vlenJ < vlenI (a proper prefix is smaller).
        self.emit(abi::label(&cmp_eq));
        self.emit(abi::compare_registers(&vlen_j, &vlen_i));
        self.emit(abi::branch_lt(less_label));
        self.emit(abi::branch(ge_label));
    }

    /// plan-64 C2: native `collections::mapValues` for a same-type 8-byte
    /// fixed-width value (V == U in Integer/Float/Fixed/Money), gated by parsing
    /// the monomorphized target `#collections_mapValues$K$V$U`. The `.mfb` version
    /// rebuilds the whole map entry-by-entry (`set(result, e.key, f(e.value))`,
    /// N inserts, leaving `ready=0`); this copies the map's key/bucket structure
    /// once and rewrites each value payload in place (keys unchanged → the copied
    /// index stays valid). Every other instantiation falls through to the `.mfb`.
    pub(super) fn lower_collection_map_values_call(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let map = self.lower_value(&args[0])?;
        let map_type = map.type_.clone();
        let map_slot = self.allocate_stack_object("mapvalues_map", 8);
        self.emit(abi::store_u64(
            &map.location,
            abi::stack_pointer(),
            map_slot,
        ));
        let action = self.lower_value(&args[1])?;
        self.require_direct_callable("mapValues", &action)?;
        let action_slot = self.allocate_stack_object("mapvalues_action", 8);
        self.emit(abi::store_u64(
            &action.location,
            abi::stack_pointer(),
            action_slot,
        ));

        // result = tight copy of the map (keys + bucket structure preserved).
        let srcreg = self.temporary_vreg();
        self.emit(abi::load_u64(&srcreg, abi::stack_pointer(), map_slot));
        let result_copy = self.copy_collection_tight(&map_type, &srcreg)?;
        let result_slot = self.allocate_stack_object("mapvalues_result", 8);
        self.emit(abi::store_u64(
            &result_copy,
            abi::stack_pointer(),
            result_slot,
        ));

        let n_slot = self.allocate_stack_object("mapvalues_n", 8);
        let i_slot = self.allocate_stack_object("mapvalues_i", 8);
        let r = self.temporary_vreg();
        let r2 = self.temporary_vreg();
        self.emit(abi::load_u64(&r, abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64(&r2, &r, COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64(&r2, abi::stack_pointer(), n_slot));

        let loop_l = self.label("mapvalues_loop");
        let done_l = self.label("mapvalues_done");
        let ok_l = self.label("mapvalues_ok");
        let entry = self.temporary_vreg();
        let off = self.temporary_vreg();
        let idxoff = self.temporary_vreg();
        let valoff = self.temporary_vreg();
        let base = self.temporary_vreg();
        let resreg = self.temporary_vreg();
        let valaddr = self.temporary_vreg();
        let val = self.temporary_vreg();
        let act = self.temporary_vreg();

        self.emit(abi::move_immediate(&r, "Integer", "0"));
        self.emit(abi::store_u64(&r, abi::stack_pointer(), i_slot));
        self.emit(abi::label(&loop_l));
        self.emit(abi::load_u64(&r, abi::stack_pointer(), i_slot));
        self.emit(abi::load_u64(&r2, abi::stack_pointer(), n_slot));
        self.emit(abi::compare_registers(&r, &r2));
        self.emit(abi::branch_ge(&done_l));
        // valAddr = dataBase(result) + entry[i].valueOffset
        self.emit(abi::load_u64(&entry, abi::stack_pointer(), result_slot));
        self.emit(abi::add_immediate(&entry, &entry, COLLECTION_HEADER_SIZE));
        self.emit(abi::move_immediate(
            &off,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&idxoff, &r, &off));
        self.emit(abi::add_registers(&entry, &entry, &idxoff));
        self.emit(abi::load_u64(
            &valoff,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(&resreg, abi::stack_pointer(), result_slot));
        self.emit_collection_data_pointer_for(&base, &resreg, "");
        self.emit(abi::add_registers(&valaddr, &base, &valoff));
        self.emit(abi::load_u64(&val, &valaddr, 0));
        // f(value)
        self.emit(abi::move_register(&abi::argument_register(0)?, &val));
        self.emit(abi::load_u64(&act, abi::stack_pointer(), action_slot));
        self.emit_direct_callable_branch(&act);
        self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
        self.emit(abi::branch_eq(&ok_l));
        self.emit_callback_failure_exit(Some((result_slot, map_type.clone())))?;
        self.emit(abi::label(&ok_l));
        // Recompute valAddr (the call clobbered caller-saved regs) and store f's result.
        self.emit(abi::load_u64(&r, abi::stack_pointer(), i_slot));
        self.emit(abi::load_u64(&entry, abi::stack_pointer(), result_slot));
        self.emit(abi::add_immediate(&entry, &entry, COLLECTION_HEADER_SIZE));
        self.emit(abi::move_immediate(
            &off,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&idxoff, &r, &off));
        self.emit(abi::add_registers(&entry, &entry, &idxoff));
        self.emit(abi::load_u64(
            &valoff,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(&resreg, abi::stack_pointer(), result_slot));
        self.emit_collection_data_pointer_for(&base, &resreg, "");
        self.emit(abi::add_registers(&valaddr, &base, &valoff));
        self.emit(abi::store_u64(RESULT_VALUE_REGISTER, &valaddr, 0));
        self.emit(abi::add_immediate(&r, &r, 1));
        self.emit(abi::store_u64(&r, abi::stack_pointer(), i_slot));
        self.emit(abi::branch(&loop_l));
        self.emit(abi::label(&done_l));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: map_type.clone(),
            location: result.render(),
            text: format!("mapValues({map_type})"),
        })
    }

    /// plan-64 D3: native `collections::window` for 8-byte fixed-width elements
    /// with a constant `size >= 1` and constant `stride >= 1` (so the `size < 1`
    /// FAIL guard is provably unnecessary). The `.mfb` allocates a fresh slice per
    /// window then copies it into the result and abandons it (alloc + copy + copy +
    /// free per window); this builds the `List OF List OF T` result directly —
    /// each window is a kind-2 inner block written in place at the outer's data
    /// tail with one copy from the source. `size`/`stride` are the parsed literals.
    pub(super) fn lower_collection_window_call(
        &mut self,
        args: &[NirValue],
        size: i64,
        stride: i64,
    ) -> Result<ValueResult, String> {
        let source = self.lower_value(&args[0])?;
        let elem = list_element_type(&source.type_)
            .ok_or_else(|| format!("native window does not accept {}", source.type_))?;
        let inner_type = source.type_.clone();
        let outer_type = format!("List OF {inner_type}");
        let outer_layout = CollectionTypeLayout::from_type(&outer_type)
            .ok_or_else(|| format!("native window cannot resolve {outer_type}"))?;
        let inner_layout = CollectionTypeLayout::from_type(&inner_type)
            .ok_or_else(|| format!("native window cannot resolve {inner_type}"))?;
        let _ = elem;
        let inner_block_size = COLLECTION_HEADER_SIZE + (size as usize) * 8;
        let source_slot = self.allocate_stack_object("window_source", 8);
        self.emit(abi::store_u64(
            &source.location,
            abi::stack_pointer(),
            source_slot,
        ));

        let wc_slot = self.allocate_stack_object("window_count", 8);
        let result_slot = self.allocate_stack_object("window_result", 8);
        let n = self.temporary_vreg();
        let wc = self.temporary_vreg();
        let t = self.temporary_vreg();
        // n = count(source); windowCount = n >= size ? (n - size)/stride + 1 : 0.
        self.emit(abi::load_u64(&n, abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64(&n, &n, COLLECTION_OFFSET_COUNT));
        let wc_zero = self.label("window_wc_zero");
        let wc_done = self.label("window_wc_done");
        // stride == 1 (gated), so windowCount = n - size + 1 (no division).
        self.emit(abi::compare_immediate(&n, &size.to_string()));
        self.emit(abi::branch_lt(&wc_zero));
        self.emit(abi::subtract_immediate(&wc, &n, size as usize));
        self.emit(abi::add_immediate(&wc, &wc, 1));
        self.emit(abi::branch(&wc_done));
        self.emit(abi::label(&wc_zero));
        self.emit(abi::move_immediate(&wc, "Integer", "0"));
        self.emit(abi::label(&wc_done));
        self.emit(abi::store_u64(&wc, abi::stack_pointer(), wc_slot));

        // alloc = HEADER + windowCount*ENTRY_SIZE + windowCount*innerBlockSize.
        let size_overflow = self.label("window_size_overflow");
        let per = COLLECTION_ENTRY_SIZE + inner_block_size;
        self.emit(abi::move_immediate(&t, "Integer", &per.to_string()));
        self.emit_checked_size_multiply(abi::return_register(), &wc, &t, &size_overflow);
        self.emit_checked_size_add_immediate(
            abi::return_register(),
            abi::return_register(),
            COLLECTION_HEADER_SIZE,
            &size_overflow,
        );
        let alloc_ok = self.label("window_alloc_ok");
        self.emit(abi::move_immediate(abi::ARG[1], "Integer", "8"));
        self.emit_arena_alloc_call();
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
        // Outer header: count = capacity = windowCount; dataLength = dataCapacity =
        // windowCount * innerBlockSize.
        let outer = self.temporary_vreg();
        self.emit(abi::load_u64(&outer, abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64(&wc, abi::stack_pointer(), wc_slot));
        let dlen = self.temporary_vreg();
        self.emit(abi::move_immediate(
            &t,
            "Integer",
            &inner_block_size.to_string(),
        ));
        self.emit(abi::multiply_registers(&dlen, &wc, &t));
        self.emit_write_list_header_from_registers(&outer_layout, &outer, &wc, &dlen);

        // Per-window construction.
        let w = self.temporary_vreg();
        let entry = self.temporary_vreg();
        let inner = self.temporary_vreg();
        let outer_data = self.temporary_vreg();
        let src_data = self.temporary_vreg();
        let srcp = self.temporary_vreg();
        let dstp = self.temporary_vreg();
        let cnt = self.temporary_vreg();
        let tmp = self.temporary_vreg();
        let loop_l = self.label("window_loop");
        let done_l = self.label("window_done");
        let copy_l = self.label("window_copy");
        let copy_done = self.label("window_copy_done");
        // outerData = outer + HEADER + windowCount*ENTRY_SIZE.
        self.emit(abi::load_u64(&outer, abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64(&wc, abi::stack_pointer(), wc_slot));
        self.emit(abi::move_immediate(
            &t,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&t, &wc, &t));
        self.emit(abi::add_immediate(
            &outer_data,
            &outer,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(&outer_data, &outer_data, &t));
        // srcData = source + HEADER (kind-2 source).
        self.emit(abi::load_u64(&t, abi::stack_pointer(), source_slot));
        self.emit(abi::add_immediate(&src_data, &t, COLLECTION_HEADER_SIZE));
        self.emit(abi::move_immediate(&w, "Integer", "0"));
        self.emit(abi::label(&loop_l));
        self.emit(abi::load_u64(&wc, abi::stack_pointer(), wc_slot));
        self.emit(abi::compare_registers(&w, &wc));
        self.emit(abi::branch_ge(&done_l));
        // entry = outer + HEADER + w*ENTRY_SIZE; flags=USED, valueOffset=w*innerBlockSize, valueLength=innerBlockSize.
        self.emit(abi::load_u64(&outer, abi::stack_pointer(), result_slot));
        self.emit(abi::move_immediate(
            &t,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&t, &w, &t));
        self.emit(abi::add_immediate(&entry, &outer, COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers(&entry, &entry, &t));
        self.emit(abi::move_immediate(
            &tmp,
            "Integer",
            &COLLECTION_ENTRY_FLAG_USED.to_string(),
        ));
        self.emit(abi::store_u8(&tmp, &entry, COLLECTION_ENTRY_OFFSET_FLAGS));
        self.emit(abi::move_immediate(
            &tmp,
            "Integer",
            &inner_block_size.to_string(),
        ));
        self.emit(abi::multiply_registers(&tmp, &w, &tmp)); // w*innerBlockSize
        self.emit(abi::store_u64(
            &tmp,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::move_immediate(
            &t,
            "Integer",
            &inner_block_size.to_string(),
        ));
        self.emit(abi::store_u64(
            &t,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        // inner = outerData + w*innerBlockSize; write kind-2 inner header (count=cap=size, dataLen=dataCap=size*8).
        self.emit(abi::add_registers(&inner, &outer_data, &tmp));
        self.emit(abi::move_immediate(&cnt, "Integer", &size.to_string()));
        self.emit(abi::move_immediate(
            &t,
            "Integer",
            &((size as usize) * 8).to_string(),
        ));
        self.emit_write_list_header_from_registers(&inner_layout, &inner, &cnt, &t);
        // Copy size elements from src_data + (w*stride)*8 into inner + HEADER.
        self.emit(abi::move_immediate(&t, "Integer", &stride.to_string()));
        self.emit(abi::multiply_registers(&t, &w, &t)); // i = w*stride
        self.emit(abi::shift_left_immediate(&t, &t, 3)); // i*8
        self.emit(abi::add_registers(&srcp, &src_data, &t));
        self.emit(abi::add_immediate(&dstp, &inner, COLLECTION_HEADER_SIZE));
        self.emit(abi::move_immediate(&cnt, "Integer", "0"));
        self.emit(abi::label(&copy_l));
        self.emit(abi::compare_immediate(&cnt, &size.to_string()));
        self.emit(abi::branch_ge(&copy_done));
        self.emit(abi::load_u64(&tmp, &srcp, 0));
        self.emit(abi::store_u64(&tmp, &dstp, 0));
        self.emit(abi::add_immediate(&srcp, &srcp, 8));
        self.emit(abi::add_immediate(&dstp, &dstp, 8));
        self.emit(abi::add_immediate(&cnt, &cnt, 1));
        self.emit(abi::branch(&copy_l));
        self.emit(abi::label(&copy_done));
        self.emit(abi::add_immediate(&w, &w, 1));
        self.emit(abi::branch(&loop_l));
        self.emit(abi::label(&done_l));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: outer_type,
            location: result.render(),
            text: format!("window({})", source.type_),
        })
    }

    /// plan-64 D3: native `collections::chunks` for 8-byte fixed-width elements
    /// with a constant `size >= 1`. Non-overlapping consecutive chunks; the final
    /// chunk may be shorter. Builds the `List OF List OF T` result directly (outer
    /// kind-0 list, per-chunk kind-2 inner blocks written in place at the data
    /// tail), one word-copy per chunk — no per-chunk slice-alloc/copy/free.
    pub(super) fn lower_collection_chunks_call(
        &mut self,
        args: &[NirValue],
        size: i64,
    ) -> Result<ValueResult, String> {
        let source = self.lower_value(&args[0])?;
        let _elem = list_element_type(&source.type_)
            .ok_or_else(|| format!("native chunks does not accept {}", source.type_))?;
        let inner_type = source.type_.clone();
        let outer_type = format!("List OF {inner_type}");
        let outer_layout = CollectionTypeLayout::from_type(&outer_type)
            .ok_or_else(|| format!("native chunks cannot resolve {outer_type}"))?;
        let inner_layout = CollectionTypeLayout::from_type(&inner_type)
            .ok_or_else(|| format!("native chunks cannot resolve {inner_type}"))?;
        // Uniform per-chunk block stride (the last chunk over-allocates to this and
        // leaves a harmless tail gap — free uses dataCapacity, reads use offsets).
        let block_stride = COLLECTION_HEADER_SIZE + (size as usize) * 8;
        let source_slot = self.allocate_stack_object("chunks_source", 8);
        self.emit(abi::store_u64(
            &source.location,
            abi::stack_pointer(),
            source_slot,
        ));

        let n_slot = self.allocate_stack_object("chunks_n", 8);
        let cc_slot = self.allocate_stack_object("chunks_cc", 8);
        let result_slot = self.allocate_stack_object("chunks_result", 8);
        let n = self.temporary_vreg();
        let cc = self.temporary_vreg();
        let s = self.temporary_vreg();
        let t = self.temporary_vreg();
        self.emit(abi::load_u64(&n, abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64(&n, &n, COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64(&n, abi::stack_pointer(), n_slot));
        // chunkCount = ceil(n/size), via a count loop (no divide primitive).
        let cc_loop = self.label("chunks_cc_loop");
        let cc_done = self.label("chunks_cc_done");
        self.emit(abi::move_immediate(&cc, "Integer", "0"));
        self.emit(abi::move_immediate(&s, "Integer", "0"));
        self.emit(abi::label(&cc_loop));
        self.emit(abi::compare_registers(&s, &n));
        self.emit(abi::branch_ge(&cc_done));
        self.emit(abi::add_immediate(&cc, &cc, 1));
        self.emit(abi::add_immediate(&s, &s, size as usize));
        self.emit(abi::branch(&cc_loop));
        self.emit(abi::label(&cc_done));
        self.emit(abi::store_u64(&cc, abi::stack_pointer(), cc_slot));

        // alloc = HEADER + cc*(ENTRY_SIZE + block_stride).
        let size_overflow = self.label("chunks_size_overflow");
        let per = COLLECTION_ENTRY_SIZE + block_stride;
        self.emit(abi::move_immediate(&t, "Integer", &per.to_string()));
        self.emit_checked_size_multiply(abi::return_register(), &cc, &t, &size_overflow);
        self.emit_checked_size_add_immediate(
            abi::return_register(),
            abi::return_register(),
            COLLECTION_HEADER_SIZE,
            &size_overflow,
        );
        let alloc_ok = self.label("chunks_alloc_ok");
        self.emit(abi::move_immediate(abi::ARG[1], "Integer", "8"));
        self.emit_arena_alloc_call();
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
        let outer = self.temporary_vreg();
        let dlen = self.temporary_vreg();
        self.emit(abi::load_u64(&outer, abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64(&cc, abi::stack_pointer(), cc_slot));
        self.emit(abi::move_immediate(
            &t,
            "Integer",
            &block_stride.to_string(),
        ));
        self.emit(abi::multiply_registers(&dlen, &cc, &t));
        self.emit_write_list_header_from_registers(&outer_layout, &outer, &cc, &dlen);

        let w = self.temporary_vreg();
        let entry = self.temporary_vreg();
        let inner = self.temporary_vreg();
        let outer_data = self.temporary_vreg();
        let src_data = self.temporary_vreg();
        let srcp = self.temporary_vreg();
        let dstp = self.temporary_vreg();
        let csz = self.temporary_vreg();
        let cnt = self.temporary_vreg();
        let tmp = self.temporary_vreg();
        let loop_l = self.label("chunks_loop");
        let done_l = self.label("chunks_done");
        let clamp_l = self.label("chunks_clamp_done");
        let copy_l = self.label("chunks_copy");
        let copy_done = self.label("chunks_copy_done");
        self.emit(abi::load_u64(&outer, abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64(&cc, abi::stack_pointer(), cc_slot));
        self.emit(abi::move_immediate(
            &t,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&t, &cc, &t));
        self.emit(abi::add_immediate(
            &outer_data,
            &outer,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(&outer_data, &outer_data, &t));
        self.emit(abi::load_u64(&t, abi::stack_pointer(), source_slot));
        self.emit(abi::add_immediate(&src_data, &t, COLLECTION_HEADER_SIZE));
        self.emit(abi::move_immediate(&w, "Integer", "0"));
        self.emit(abi::label(&loop_l));
        self.emit(abi::load_u64(&cc, abi::stack_pointer(), cc_slot));
        self.emit(abi::compare_registers(&w, &cc));
        self.emit(abi::branch_ge(&done_l));
        // start = w*size ; chunkSize = min(size, n - start)
        self.emit(abi::move_immediate(&t, "Integer", &size.to_string()));
        self.emit(abi::multiply_registers(&s, &w, &t)); // start
        self.emit(abi::load_u64(&n, abi::stack_pointer(), n_slot));
        self.emit(abi::subtract_registers(&csz, &n, &s)); // remaining
        self.emit(abi::compare_immediate(&csz, &size.to_string()));
        self.emit(abi::branch_lt(&clamp_l));
        self.emit(abi::move_immediate(&csz, "Integer", &size.to_string()));
        self.emit(abi::label(&clamp_l));
        // entry
        self.emit(abi::load_u64(&outer, abi::stack_pointer(), result_slot));
        self.emit(abi::move_immediate(
            &t,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&t, &w, &t));
        self.emit(abi::add_immediate(&entry, &outer, COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers(&entry, &entry, &t));
        self.emit(abi::move_immediate(
            &tmp,
            "Integer",
            &COLLECTION_ENTRY_FLAG_USED.to_string(),
        ));
        self.emit(abi::store_u8(&tmp, &entry, COLLECTION_ENTRY_OFFSET_FLAGS));
        self.emit(abi::move_immediate(
            &tmp,
            "Integer",
            &block_stride.to_string(),
        ));
        self.emit(abi::multiply_registers(&tmp, &w, &tmp)); // w*block_stride
        self.emit(abi::store_u64(
            &tmp,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        // valueLength = HEADER + chunkSize*8
        self.emit(abi::shift_left_immediate(&t, &csz, 3));
        self.emit(abi::add_immediate(&t, &t, COLLECTION_HEADER_SIZE));
        self.emit(abi::store_u64(
            &t,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        // inner block header (count = chunkSize, dataLen = chunkSize*8)
        self.emit(abi::add_registers(&inner, &outer_data, &tmp));
        self.emit(abi::shift_left_immediate(&t, &csz, 3));
        self.emit_write_list_header_from_registers(&inner_layout, &inner, &csz, &t);
        // copy chunkSize elements from src_data + start*8 into inner + HEADER
        self.emit(abi::shift_left_immediate(&t, &s, 3));
        self.emit(abi::add_registers(&srcp, &src_data, &t));
        self.emit(abi::add_immediate(&dstp, &inner, COLLECTION_HEADER_SIZE));
        self.emit(abi::move_immediate(&cnt, "Integer", "0"));
        self.emit(abi::label(&copy_l));
        self.emit(abi::compare_registers(&cnt, &csz));
        self.emit(abi::branch_ge(&copy_done));
        self.emit(abi::load_u64(&tmp, &srcp, 0));
        self.emit(abi::store_u64(&tmp, &dstp, 0));
        self.emit(abi::add_immediate(&srcp, &srcp, 8));
        self.emit(abi::add_immediate(&dstp, &dstp, 8));
        self.emit(abi::add_immediate(&cnt, &cnt, 1));
        self.emit(abi::branch(&copy_l));
        self.emit(abi::label(&copy_done));
        self.emit(abi::add_immediate(&w, &w, 1));
        self.emit(abi::branch(&loop_l));
        self.emit(abi::label(&done_l));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: outer_type,
            location: result.render(),
            text: format!("chunks({})", source.type_),
        })
    }

    /// plan-64 D1: native `collections::groupBy` (8-byte fixed-width T/V, Integer
    /// key, re-eval-safe `value`). Grows each bucket as a top-level list keyed via
    /// an inline open-addressing hash table (no O(bucket²) get-copy), then
    /// materializes the `Map OF K TO List OF V` once. Else `.mfb`.
    pub(super) fn lower_collection_group_by_call(
        &mut self,
        args: &[NirValue],
        key_type: &str,
        value_type: &str,
    ) -> Result<ValueResult, String> {
        let list_v = format!("List OF {value_type}");
        let map_type = format!("Map OF {key_type} TO {list_v}");
        let int_layout = CollectionTypeLayout::from_type("List OF Integer")
            .ok_or_else(|| "groupBy: int layout".to_string())?;
        let _k_layout = CollectionTypeLayout::from_type(&format!("List OF {key_type}"))
            .ok_or_else(|| "groupBy: key layout".to_string())?;
        let v_layout = CollectionTypeLayout::from_type(&list_v)
            .ok_or_else(|| "groupBy: value layout".to_string())?;
        let keys = self.lower_collection_transform_call(&[args[0].clone(), args[1].clone()])?;
        let keys_slot = self.allocate_stack_object("gb_keys", 8);
        self.emit(abi::store_u64(
            &keys.location,
            abi::stack_pointer(),
            keys_slot,
        ));
        let vals = self.lower_collection_transform_call(&[args[0].clone(), args[2].clone()])?;
        let vals_slot = self.allocate_stack_object("gb_vals", 8);
        self.emit(abi::store_u64(
            &vals.location,
            abi::stack_pointer(),
            vals_slot,
        ));

        let n_slot = self.allocate_stack_object("gb_n", 8);
        let ts_slot = self.allocate_stack_object("gb_ts", 8);
        let mask_slot = self.allocate_stack_object("gb_mask", 8);
        let nb_slot = self.allocate_stack_object("gb_nb", 8);
        let hk_slot = self.allocate_stack_object("gb_hk", 8);
        let ho_slot = self.allocate_stack_object("gb_ho", 8);
        let bp_slot = self.allocate_stack_object("gb_bp", 8);
        let ko_slot = self.allocate_stack_object("gb_ko", 8);
        let result_slot = self.allocate_stack_object("gb_result", 8);
        let i_slot = self.allocate_stack_object("gb_i", 8);
        let slot_save = self.allocate_stack_object("gb_slotsave", 8);
        let bidx_slot = self.allocate_stack_object("gb_bidx", 8);
        let bucket_slot = self.allocate_stack_object("gb_bucket", 8);
        let val_slot = self.allocate_stack_object("gb_val", 8);
        let key_slot = self.allocate_stack_object("gb_key", 8);
        let r = self.temporary_vreg();
        let t = self.temporary_vreg();
        let ovf = self.label("gb_ovf");
        // n
        self.emit(abi::load_u64(&r, abi::stack_pointer(), keys_slot));
        self.emit(abi::load_u64(&r, &r, COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64(&r, abi::stack_pointer(), n_slot));
        // tableSize = smallest pow2 >= 2n (>=2); mask = ts-1
        let ts = self.temporary_vreg();
        let two_n = self.temporary_vreg();
        self.emit(abi::add_registers(&two_n, &r, &r));
        self.emit(abi::move_immediate(&ts, "Integer", "2"));
        let ts_loop = self.label("gb_ts_loop");
        let ts_done = self.label("gb_ts_done");
        self.emit(abi::label(&ts_loop));
        self.emit(abi::compare_registers(&ts, &two_n));
        self.emit(abi::branch_ge(&ts_done));
        self.emit(abi::add_registers(&ts, &ts, &ts));
        self.emit(abi::branch(&ts_loop));
        self.emit(abi::label(&ts_done));
        self.emit(abi::store_u64(&ts, abi::stack_pointer(), ts_slot));
        self.emit(abi::subtract_immediate(&t, &ts, 1));
        self.emit(abi::store_u64(&t, abi::stack_pointer(), mask_slot));

        // Allocate the four kind-2 List OF Integer scratch buffers.
        let after_alloc = self.label("gb_after_alloc");
        for (cap_slot, dst_slot) in [
            (ts_slot, hk_slot),
            (ts_slot, ho_slot),
            (n_slot, bp_slot),
            (n_slot, ko_slot),
        ] {
            self.emit(abi::load_u64(&r, abi::stack_pointer(), cap_slot));
            self.emit(abi::shift_left_immediate(abi::return_register(), &r, 3));
            self.emit_checked_size_add_immediate(
                abi::return_register(),
                abi::return_register(),
                COLLECTION_HEADER_SIZE,
                &ovf,
            );
            self.emit(abi::move_immediate(abi::ARG[1], "Integer", "8"));
            self.emit_arena_alloc_call();
            let ok = self.label("gb_scratch_ok");
            self.emit(abi::branch_eq(&ok));
            self.emit_allocation_error_return()?;
            self.emit(abi::label(&ok));
            self.emit(abi::store_u64(abi::RET[1], abi::stack_pointer(), dst_slot));
            self.emit(abi::load_u64(&r, abi::stack_pointer(), cap_slot));
            let dl = self.temporary_vreg();
            self.emit(abi::shift_left_immediate(&dl, &r, 3));
            self.emit(abi::load_u64(&t, abi::stack_pointer(), dst_slot));
            self.emit_write_list_header_from_registers(&int_layout, &t, &r, &dl);
        }
        self.emit(abi::branch(&after_alloc));
        self.emit(abi::label(&ovf));
        self.emit_error_code_return(ERR_OUT_OF_MEMORY_CODE, ERR_ALLOCATION_MESSAGE)?;
        self.emit(abi::label(&after_alloc));
        // Zero hashOcc data.
        let zp = self.temporary_vreg();
        let zj = self.temporary_vreg();
        let zloop = self.label("gb_zloop");
        let zdone = self.label("gb_zdone");
        self.emit(abi::load_u64(&zp, abi::stack_pointer(), ho_slot));
        self.emit(abi::add_immediate(&zp, &zp, COLLECTION_HEADER_SIZE));
        self.emit(abi::move_immediate(&zj, "Integer", "0"));
        self.emit(abi::load_u64(&ts, abi::stack_pointer(), ts_slot));
        self.emit(abi::label(&zloop));
        self.emit(abi::compare_registers(&zj, &ts));
        self.emit(abi::branch_ge(&zdone));
        self.emit(abi::store_u64(abi::ZERO, &zp, 0));
        self.emit(abi::add_immediate(&zp, &zp, 8));
        self.emit(abi::add_immediate(&zj, &zj, 1));
        self.emit(abi::branch(&zloop));
        self.emit(abi::label(&zdone));
        self.emit(abi::move_immediate(&r, "Integer", "0"));
        self.emit(abi::store_u64(&r, abi::stack_pointer(), nb_slot));
        self.emit(abi::store_u64(&r, abi::stack_pointer(), i_slot));

        // --- element loop ---
        let key = self.temporary_vreg();
        let slot = self.temporary_vreg();
        let occ = self.temporary_vreg();
        let mask = self.temporary_vreg();
        let base = self.temporary_vreg();
        let addr = self.temporary_vreg();
        let p = self.temporary_vreg();
        let el_loop = self.label("gb_el_loop");
        let el_done = self.label("gb_el_done");
        let probe = self.label("gb_probe");
        let found = self.label("gb_found");
        let insert = self.label("gb_insert");
        let el_next = self.label("gb_el_next");
        self.emit(abi::label(&el_loop));
        self.emit(abi::load_u64(&r, abi::stack_pointer(), i_slot));
        self.emit(abi::load_u64(&t, abi::stack_pointer(), n_slot));
        self.emit(abi::compare_registers(&r, &t));
        self.emit(abi::branch_ge(&el_done));
        // key = keys[i]; val = vals[i]
        self.emit(abi::shift_left_immediate(&t, &r, 3));
        self.emit(abi::load_u64(&base, abi::stack_pointer(), keys_slot));
        self.emit(abi::add_immediate(&base, &base, COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers(&addr, &base, &t));
        self.emit(abi::load_u64(&key, &addr, 0));
        self.emit(abi::store_u64(&key, abi::stack_pointer(), key_slot));
        self.emit(abi::load_u64(&base, abi::stack_pointer(), vals_slot));
        self.emit(abi::add_immediate(&base, &base, COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers(&addr, &base, &t));
        self.emit(abi::load_u64(&p, &addr, 0));
        self.emit(abi::store_u64(&p, abi::stack_pointer(), val_slot));
        // probe: slot = key & mask
        self.emit(abi::load_u64(&mask, abi::stack_pointer(), mask_slot));
        self.emit(abi::and_registers(&slot, &key, &mask));
        self.emit(abi::label(&probe));
        self.emit(abi::load_u64(&base, abi::stack_pointer(), ho_slot));
        self.emit(abi::add_immediate(&base, &base, COLLECTION_HEADER_SIZE));
        self.emit(abi::shift_left_immediate(&t, &slot, 3));
        self.emit(abi::add_registers(&addr, &base, &t));
        self.emit(abi::load_u64(&occ, &addr, 0));
        self.emit(abi::compare_immediate(&occ, "0"));
        self.emit(abi::branch_eq(&insert));
        self.emit(abi::load_u64(&base, abi::stack_pointer(), hk_slot));
        self.emit(abi::add_immediate(&base, &base, COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers(&addr, &base, &t));
        self.emit(abi::load_u64(&p, &addr, 0));
        self.emit(abi::compare_registers(&p, &key));
        self.emit(abi::branch_eq(&found));
        self.emit(abi::add_immediate(&slot, &slot, 1));
        self.emit(abi::and_registers(&slot, &slot, &mask));
        self.emit(abi::branch(&probe));
        // found: bidx = occ-1; append val to bucketPtrs[bidx] (spill bidx across call)
        self.emit(abi::label(&found));
        self.emit(abi::subtract_immediate(&p, &occ, 1));
        self.emit(abi::store_u64(&p, abi::stack_pointer(), bidx_slot));
        self.emit(abi::load_u64(&base, abi::stack_pointer(), bp_slot));
        self.emit(abi::add_immediate(&base, &base, COLLECTION_HEADER_SIZE));
        self.emit(abi::shift_left_immediate(&t, &p, 3));
        self.emit(abi::add_registers(&addr, &base, &t));
        self.emit(abi::load_u64(&p, &addr, 0));
        self.emit(abi::store_u64(&p, abi::stack_pointer(), bucket_slot));
        self.lower_list_append_in_place(bucket_slot, val_slot, &list_v, value_type)?;
        self.emit(abi::load_u64(&p, abi::stack_pointer(), bucket_slot));
        self.emit(abi::load_u64(&base, abi::stack_pointer(), bp_slot));
        self.emit(abi::add_immediate(&base, &base, COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64(&t, abi::stack_pointer(), bidx_slot));
        self.emit(abi::shift_left_immediate(&t, &t, 3));
        self.emit(abi::add_registers(&addr, &base, &t));
        self.emit(abi::store_u64(&p, &addr, 0));
        self.emit(abi::branch(&el_next));
        // insert: new 1-element bucket; register in arrays + hash (spill slot across alloc)
        self.emit(abi::label(&insert));
        self.emit(abi::store_u64(&slot, abi::stack_pointer(), slot_save));
        self.emit(abi::move_immediate(
            abi::return_register(),
            "Integer",
            &(COLLECTION_HEADER_SIZE + 8).to_string(),
        ));
        self.emit(abi::move_immediate(abi::ARG[1], "Integer", "8"));
        self.emit_arena_alloc_call();
        let ins_ok = self.label("gb_ins_ok");
        self.emit(abi::branch_eq(&ins_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&ins_ok));
        self.emit(abi::store_u64(
            abi::RET[1],
            abi::stack_pointer(),
            bucket_slot,
        ));
        // bucket header: count=cap=1, dataLen=dataCap=8; store val at +HEADER
        self.emit(abi::move_immediate(&r, "Integer", "1"));
        self.emit(abi::move_immediate(&t, "Integer", "8"));
        self.emit(abi::load_u64(&p, abi::stack_pointer(), bucket_slot));
        self.emit_write_list_header_from_registers(&v_layout, &p, &r, &t);
        self.emit(abi::load_u64(&t, abi::stack_pointer(), val_slot));
        self.emit(abi::store_u64(&t, &p, COLLECTION_HEADER_SIZE));
        // nb = load; bucketPtrs[nb]=bucket; keyOrder[nb]=key; hashKeys[slot]=key; hashOcc[slot]=nb+1
        self.emit(abi::load_u64(&r, abi::stack_pointer(), nb_slot)); // nb
        self.emit(abi::shift_left_immediate(&t, &r, 3));
        self.emit(abi::load_u64(&base, abi::stack_pointer(), bp_slot));
        self.emit(abi::add_immediate(&base, &base, COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers(&addr, &base, &t));
        self.emit(abi::load_u64(&p, abi::stack_pointer(), bucket_slot));
        self.emit(abi::store_u64(&p, &addr, 0));
        self.emit(abi::load_u64(&key, abi::stack_pointer(), key_slot));
        self.emit(abi::load_u64(&base, abi::stack_pointer(), ko_slot));
        self.emit(abi::add_immediate(&base, &base, COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers(&addr, &base, &t));
        self.emit(abi::store_u64(&key, &addr, 0));
        self.emit(abi::load_u64(&slot, abi::stack_pointer(), slot_save));
        self.emit(abi::shift_left_immediate(&t, &slot, 3));
        self.emit(abi::load_u64(&base, abi::stack_pointer(), hk_slot));
        self.emit(abi::add_immediate(&base, &base, COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers(&addr, &base, &t));
        self.emit(abi::store_u64(&key, &addr, 0));
        self.emit(abi::add_immediate(&p, &r, 1)); // nb+1
        self.emit(abi::load_u64(&base, abi::stack_pointer(), ho_slot));
        self.emit(abi::add_immediate(&base, &base, COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers(&addr, &base, &t));
        self.emit(abi::store_u64(&p, &addr, 0));
        self.emit(abi::add_immediate(&r, &r, 1));
        self.emit(abi::store_u64(&r, abi::stack_pointer(), nb_slot));
        self.emit(abi::label(&el_next));
        self.emit(abi::load_u64(&r, abi::stack_pointer(), i_slot));
        self.emit(abi::add_immediate(&r, &r, 1));
        self.emit(abi::store_u64(&r, abi::stack_pointer(), i_slot));
        self.emit(abi::branch(&el_loop));
        self.emit(abi::label(&el_done));

        // --- final map: result = {}; for b: set(result, keyOrder[b], bucketPtrs[b]); free bucket ---
        let empty = self.lower_map_literal(&map_type, &[])?;
        self.emit(abi::store_u64(
            &empty.location,
            abi::stack_pointer(),
            result_slot,
        ));
        let b_slot = self.allocate_stack_object("gb_b", 8);
        self.emit(abi::move_immediate(&r, "Integer", "0"));
        self.emit(abi::store_u64(&r, abi::stack_pointer(), b_slot));
        let fm_loop = self.label("gb_fm_loop");
        let fm_done = self.label("gb_fm_done");
        self.emit(abi::label(&fm_loop));
        self.emit(abi::load_u64(&r, abi::stack_pointer(), b_slot));
        self.emit(abi::load_u64(&t, abi::stack_pointer(), nb_slot));
        self.emit(abi::compare_registers(&r, &t));
        self.emit(abi::branch_ge(&fm_done));
        self.emit(abi::shift_left_immediate(&t, &r, 3));
        self.emit(abi::load_u64(&base, abi::stack_pointer(), ko_slot));
        self.emit(abi::add_immediate(&base, &base, COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers(&addr, &base, &t));
        self.emit(abi::load_u64(&p, &addr, 0));
        self.emit(abi::store_u64(&p, abi::stack_pointer(), key_slot));
        self.emit(abi::load_u64(&base, abi::stack_pointer(), bp_slot));
        self.emit(abi::add_immediate(&base, &base, COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers(&addr, &base, &t));
        self.emit(abi::load_u64(&p, &addr, 0));
        self.emit(abi::store_u64(&p, abi::stack_pointer(), bucket_slot));
        let set = self.lower_map_set_in_place(
            result_slot,
            key_slot,
            bucket_slot,
            &map_type,
            key_type,
            &list_v,
        )?;
        self.emit(abi::store_u64(
            &set.location,
            abi::stack_pointer(),
            result_slot,
        ));
        // free the now-copied bucket
        let keep = ValueResult {
            type_: map_type.clone(),
            location: {
                let z = self.allocate_register()?;
                self.emit(abi::load_u64(&z, abi::stack_pointer(), result_slot));
                z.render()
            },
            text: String::new(),
        };
        self.free_intermediate_collection(bucket_slot, &list_v, keep)?;
        self.emit(abi::load_u64(&r, abi::stack_pointer(), b_slot));
        self.emit(abi::add_immediate(&r, &r, 1));
        self.emit(abi::store_u64(&r, abi::stack_pointer(), b_slot));
        self.emit(abi::branch(&fm_loop));
        self.emit(abi::label(&fm_done));
        // free the six scratch buffers (thread result through)
        let mut threaded = ValueResult {
            type_: map_type.clone(),
            location: {
                let z = self.allocate_register()?;
                self.emit(abi::load_u64(&z, abi::stack_pointer(), result_slot));
                z.render()
            },
            text: String::new(),
        };
        for (s, ty) in [
            (keys_slot, format!("List OF {key_type}")),
            (vals_slot, list_v.clone()),
            (hk_slot, "List OF Integer".to_string()),
            (ho_slot, "List OF Integer".to_string()),
            (bp_slot, "List OF Integer".to_string()),
            (ko_slot, "List OF Integer".to_string()),
        ] {
            threaded = self.free_intermediate_collection(s, &ty, threaded)?;
        }
        Ok(ValueResult {
            type_: map_type,
            location: threaded.location,
            text: "groupBy".to_string(),
        })
    }

    pub(super) fn lower_collection_reduce_call(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        self.lower_collection_reduce_impl(args, false)
    }

    /// `reduceRight` (plan-86-B, B2): the same left-fold machinery as `reduce`,
    /// walked from the last element to the first. `reduceRight(xs, init, f)` folds
    /// `f(acc, xs[n-1]), f(acc, xs[n-2]), …, f(acc, xs[0])`, which is exactly
    /// `reduce` over the reverse-iterated elements with the identical
    /// `FUNC(U, T) AS U` reducer — so it shares the accumulator reclamation and the
    /// aliasing-safe frees below, differing only in the loop-slot walk direction.
    pub(super) fn lower_collection_reduce_right_call(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        self.lower_collection_reduce_impl(args, true)
    }

    fn lower_collection_reduce_impl(
        &mut self,
        args: &[NirValue],
        reverse: bool,
    ) -> Result<ValueResult, String> {
        let scratch9 = self.temporary_vreg();
        let scratch17 = self.temporary_vreg();
        let collection = self.lower_value(&args[0])?;
        let Some(element_type) = list_element_type(&collection.type_) else {
            return Err(format!(
                "native collection reduce does not accept {}",
                collection.type_
            ));
        };
        let collection_slot = self.allocate_stack_object("reduce_collection", 8);
        self.emit(abi::store_u64(
            &collection.location,
            abi::stack_pointer(),
            collection_slot,
        ));
        let initial = self.lower_value(&args[1])?;
        let accumulator_slot = self.allocate_stack_object("reduce_accumulator", 8);
        self.emit(abi::store_u64(
            &initial.location,
            abi::stack_pointer(),
            accumulator_slot,
        ));
        let action = self.lower_value(&args[2])?;
        let output_type = callable_return_type(&action.type_).ok_or_else(|| {
            format!(
                "native collection reduce reducer must be a function, got {}",
                action.type_
            )
        })?;
        if output_type != initial.type_ {
            return Err(format!(
                "native collection reduce reducer must return {}, got {output_type}",
                initial.type_
            ));
        }
        self.require_direct_callable(if reverse { "reduceRight" } else { "reduce" }, &action)?;
        let action_slot = self.allocate_stack_object("reduce_action", 8);
        self.emit(abi::store_u64(
            &action.location,
            abi::stack_pointer(),
            action_slot,
        ));
        let cursor_slot = self.allocate_stack_object("reduce_cursor", 8);
        let remaining_slot = self.allocate_stack_object("reduce_remaining", 8);
        if reverse {
            self.initialize_collection_loop_slots_reverse(
                collection_slot,
                cursor_slot,
                remaining_slot,
                &element_type,
            );
        } else {
            self.initialize_collection_loop_slots(
                collection_slot,
                cursor_slot,
                remaining_slot,
                &element_type,
            );
        }

        // plan-86-B (B1): reclaim the superseded loop item and accumulator each
        // iteration instead of leaking them (the pre-fix native `reduce` never
        // freed either, so a `List OF String` fold over N passes grew arena RSS by
        // ~one block per element per pass — the 3.5× penalty that made native
        // `reduce` slower than interpreted `reduceRight` doing the same fold).
        //
        // The bug-307 hazard the leak avoided is real but detectable at runtime:
        // the reducer may adopt the item (`FUNC(acc, x) RETURN x`) or return the
        // accumulator unchanged/in-place, in which case the block is still live.
        // MFBASIC value semantics guarantees a returned String never *partially*
        // aliases an input (a slice/concat produces a fresh owned block), so a
        // pointer-equality check against the item and the old accumulator is a
        // sufficient and exact aliasing test. We free only blocks we own and that
        // the reducer did not carry forward.
        //
        // `acc_owned` tracks whether the current accumulator is a block this loop
        // is responsible for freeing. The seed starts `owned = 0`: its ownership
        // stays with the caller (value semantics), exactly as before this fix, so
        // it is never freed here and the returned result is never double-freed.
        // This machinery is emitted only when a String block is actually at risk
        // of leaking (String accumulator and/or String element); scalar folds keep
        // their prior byte-identical codegen.
        let manages_owned = initial.type_ == "String" || element_type == "String";
        let (item_slot, acc_owned_slot, new_slot, new_owned_slot) = if manages_owned {
            let item_slot = self.allocate_stack_object("reduce_item", 8);
            let acc_owned_slot = self.allocate_stack_object("reduce_acc_owned", 8);
            let new_slot = self.allocate_stack_object("reduce_new", 8);
            let new_owned_slot = self.allocate_stack_object("reduce_new_owned", 8);
            let zero = self.temporary_vreg();
            self.emit(abi::move_immediate(&zero, "Integer", "0"));
            self.emit(abi::store_u64(&zero, abi::stack_pointer(), acc_owned_slot));
            (
                Some(item_slot),
                Some(acc_owned_slot),
                Some(new_slot),
                Some(new_owned_slot),
            )
        } else {
            (None, None, None, None)
        };

        let loop_label = self.label("reduce_call_loop");
        let ok_label = self.label("reduce_call_ok");
        let done = self.label("reduce_call_done");
        self.emit(abi::label(&loop_label));
        self.emit(abi::load_u64(
            &scratch9,
            abi::stack_pointer(),
            remaining_slot,
        ));
        self.emit(abi::compare_immediate(&scratch9, "0"));
        self.emit(abi::branch_eq(&done));
        let item = self.load_collection_loop_item(collection_slot, cursor_slot, &element_type)?;
        if let Some(item_slot) = item_slot {
            // Spill the (freshly materialized, owned) item so it survives the
            // reducer call's register clobber and can be freed/compared after.
            self.emit(abi::store_u64(&item, abi::stack_pointer(), item_slot));
        }
        self.emit(abi::load_u64(
            &abi::argument_register(0)?,
            abi::stack_pointer(),
            accumulator_slot,
        ));
        self.emit(abi::move_register(&abi::argument_register(1)?, &item));
        self.emit(abi::load_u64(&scratch17, abi::stack_pointer(), action_slot));
        self.emit_direct_callable_branch(&scratch17);
        self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
        self.emit(abi::branch_eq(&ok_label));
        // Failure path (bug-307 / plan-26-B): pass `None` (no cleanup). The
        // success path below reclaims the superseded item and accumulator via
        // runtime aliasing checks, but on a *failing* iteration the current item
        // and accumulator may still alias the reducer's live inputs or the
        // non-owned seed, and the ownership bookkeeping has not yet been committed
        // for this iteration — so freeing here could double-free or free a
        // borrowed block after the handler recovers. Leaking the at-most-one
        // in-flight item/accumulator on the rare error path is the safe choice;
        // the hot success path is where the reclamation matters (plan-86-B).
        self.emit_callback_failure_exit(None)?;
        self.emit(abi::label(&ok_label));
        if manages_owned {
            // Slots are all `Some` under `manages_owned`.
            let item_slot = item_slot.unwrap();
            let acc_owned_slot = acc_owned_slot.unwrap();
            let new_slot = new_slot.unwrap();
            let new_owned_slot = new_owned_slot.unwrap();

            // Spill the reducer output; the frees below clobber every caller-saved
            // register (the `arena_free` call), so `new` must live in a slot.
            self.emit(abi::store_u64(
                RESULT_VALUE_REGISTER,
                abi::stack_pointer(),
                new_slot,
            ));

            // new_owned = (new == old_acc) ? old_owned : 1.
            //   - fresh result  → we own it (free it next iteration / not at all if final)
            //   - returned acc unchanged/in-place → inherit the old ownership
            //   - adopted item  → new != old_acc, so owned = 1 (the item block is owned)
            let r_owned = self.temporary_vreg();
            let r_new = self.temporary_vreg();
            let r_acc = self.temporary_vreg();
            let owned_done = self.label("reduce_new_owned_done");
            self.emit(abi::move_immediate(&r_owned, "Integer", "1"));
            self.emit(abi::load_u64(&r_new, abi::stack_pointer(), new_slot));
            self.emit(abi::load_u64(
                &r_acc,
                abi::stack_pointer(),
                accumulator_slot,
            ));
            self.emit(abi::compare_registers(&r_new, &r_acc));
            self.emit(abi::branch_ne(&owned_done));
            self.emit(abi::load_u64(
                &r_owned,
                abi::stack_pointer(),
                acc_owned_slot,
            ));
            self.emit(abi::label(&owned_done));
            self.emit(abi::store_u64(
                &r_owned,
                abi::stack_pointer(),
                new_owned_slot,
            ));

            // Free the loop item unless the reducer adopted it as the new
            // accumulator (item == new). Only String items own a standalone block;
            // fixed-width items materialize nothing.
            if element_type == "String" {
                let r_item = self.temporary_vreg();
                let r_new2 = self.temporary_vreg();
                let item_kept = self.label("reduce_item_kept");
                self.emit(abi::load_u64(&r_item, abi::stack_pointer(), item_slot));
                self.emit(abi::load_u64(&r_new2, abi::stack_pointer(), new_slot));
                self.emit(abi::compare_registers(&r_item, &r_new2));
                self.emit(abi::branch_eq(&item_kept));
                self.free_collection_loop_item(item_slot, "String")?;
                self.emit(abi::label(&item_kept));
            }

            // Free the superseded accumulator when this loop owns it (owned != 0)
            // and the reducer produced a distinct result (old_acc != new). This
            // frees a String block from a stack slot — the same operation
            // `free_collection_loop_item` performs, applied to the accumulator.
            if initial.type_ == "String" {
                let r_o = self.temporary_vreg();
                let r_a = self.temporary_vreg();
                let r_n = self.temporary_vreg();
                let acc_kept = self.label("reduce_acc_kept");
                self.emit(abi::load_u64(&r_o, abi::stack_pointer(), acc_owned_slot));
                self.emit(abi::compare_immediate(&r_o, "0"));
                self.emit(abi::branch_eq(&acc_kept));
                self.emit(abi::load_u64(&r_a, abi::stack_pointer(), accumulator_slot));
                self.emit(abi::load_u64(&r_n, abi::stack_pointer(), new_slot));
                self.emit(abi::compare_registers(&r_a, &r_n));
                self.emit(abi::branch_eq(&acc_kept));
                self.free_collection_loop_item(accumulator_slot, "String")?;
                self.emit(abi::label(&acc_kept));
            }

            // Commit the new accumulator and its ownership for the next iteration.
            let r_commit = self.temporary_vreg();
            self.emit(abi::load_u64(&r_commit, abi::stack_pointer(), new_slot));
            self.emit(abi::store_u64(
                &r_commit,
                abi::stack_pointer(),
                accumulator_slot,
            ));
            let r_commit_owned = self.temporary_vreg();
            self.emit(abi::load_u64(
                &r_commit_owned,
                abi::stack_pointer(),
                new_owned_slot,
            ));
            self.emit(abi::store_u64(
                &r_commit_owned,
                abi::stack_pointer(),
                acc_owned_slot,
            ));
        } else {
            self.emit(abi::store_u64(
                RESULT_VALUE_REGISTER,
                abi::stack_pointer(),
                accumulator_slot,
            ));
        }
        if reverse {
            self.advance_collection_loop_reverse(
                cursor_slot,
                remaining_slot,
                &loop_label,
                &element_type,
            );
        } else {
            self.advance_collection_loop(cursor_slot, remaining_slot, &loop_label, &element_type);
        }
        self.emit(abi::label(&done));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(
            &result,
            abi::stack_pointer(),
            accumulator_slot,
        ));
        Ok(ValueResult {
            type_: initial.type_,
            location: result.render(),
            text: format!(
                "{}({}, {}, {})",
                if reverse { "reduceRight" } else { "reduce" },
                collection.type_,
                initial.text,
                action.text
            ),
        })
    }

    /// The callback-failure exit shared by the collection loop members
    /// (`transform`/`filter`/`reduce`/`forEach`). When the user callback returns a
    /// non-`Ok` `Result`, the raw error is already in the standard tag/value/
    /// message/source registers (x0–x3). Two routes:
    ///
    /// - **Non-trapped** (`raw_result_capture` is `None`): the member auto-
    ///   propagates the error with a bare `return` — byte-identical to before
    ///   plan-26-B.
    /// - **Inline `TRAP`** (`raw_result_capture` is `Some`): free the member's
    ///   loop-scoped intermediate (via `cleanup`), then branch to the capture point
    ///   leaving the raw `Result` in the registers for `materialize_current_result`.
    ///   Because the cleanup's `arena_free` clobbers every caller-saved register
    ///   (including x0–x3), the raw `Result` is spilled around it and reloaded.
    ///
    /// `cleanup` names the member's private, uniquely-owned intermediate to free
    /// (`transform`/`filter`: the partial output list; `forEach`: none). `reduce`
    /// passes `None`: on a failing iteration its accumulator may still alias the
    /// **non-owned** seed (the seed reaches codegen as a bare local with no owning
    /// copy) or the reducer's live inputs, and its per-iteration ownership flag has
    /// not been committed yet, so freeing here could double-free or free a borrowed
    /// block after the handler recovers. Its *success* path reclaims the superseded
    /// item/accumulator itself via runtime aliasing checks (plan-86-B).
    pub(super) fn emit_callback_failure_exit(
        &mut self,
        cleanup: Option<(usize, String)>,
    ) -> Result<(), String> {
        let Some(label) = self.raw_result_capture.clone() else {
            self.emit(abi::return_());
            return Ok(());
        };
        if let Some((block_slot, type_)) = cleanup {
            let regs = [
                RESULT_TAG_REGISTER,
                RESULT_VALUE_REGISTER,
                RESULT_ERROR_MESSAGE_REGISTER,
                RESULT_ERROR_SOURCE_REGISTER,
            ];
            let slots: Vec<usize> = regs
                .iter()
                .map(|_| self.allocate_stack_object("callback_fail_result", 8))
                .collect();
            for (reg, slot) in regs.iter().zip(&slots) {
                self.emit(abi::store_u64(reg, abi::stack_pointer(), *slot));
            }
            self.emit_owned_value_drop(&OwnedValueCleanup {
                type_,
                stack_offset: block_slot,
                closure_captures: None,
            })?;
            for (reg, slot) in regs.iter().zip(&slots) {
                self.emit(abi::load_u64(reg, abi::stack_pointer(), *slot));
            }
        }
        self.emit(abi::branch(&label));
        Ok(())
    }

    pub(super) fn require_direct_callable(
        &self,
        name: &str,
        action: &ValueResult,
    ) -> Result<(), String> {
        if !action.type_.starts_with("FUNC(") {
            return Err(format!(
                "native collection {name} action must be a function, got {}",
                action.type_
            ));
        }
        if action.location == "void" {
            return Err(format!(
                "native collection {name} action does not have a callable location"
            ));
        }
        Ok(())
    }

    pub(super) fn emit_direct_callable_branch(&mut self, location: impl Into<Operand>) {
        let saved_env_slot = self.allocate_stack_object("closure_saved_env", 8);
        // Infallible vreg minters: an exhaustion under `-regalloc bump` is recorded
        // and surfaced by `run_register_allocation` instead of panicking (bug-70).
        let code_register = self.temporary_vreg();
        let env_register = self.temporary_vreg();
        let location = location.into();
        self.emit(abi::store_u64(
            CLOSURE_ENV_REGISTER,
            abi::stack_pointer(),
            saved_env_slot,
        ));
        self.emit(abi::load_u64(
            &code_register,
            location.clone(),
            CLOSURE_OFFSET_CODE,
        ));
        self.emit(abi::load_u64(
            &env_register,
            location.clone(),
            CLOSURE_OFFSET_ENV,
        ));
        self.emit(abi::move_register(CLOSURE_ENV_REGISTER, &env_register));
        self.emit_callable_branch(&code_register.render());
        self.emit(abi::load_u64(
            CLOSURE_ENV_REGISTER,
            abi::stack_pointer(),
            saved_env_slot,
        ));
    }

    pub(super) fn emit_callable_branch(&mut self, location: &str) {
        // A callable held in a register (a physical `x*` or a not-yet-colored
        // virtual register) is an indirect `blr`; a bare function symbol is a
        // direct `bl` + relocation.
        if location.starts_with('x') || regalloc::parse_vreg(location).is_some() {
            self.emit(abi::branch_link_register(location));
            return;
        }
        self.emit(abi::branch_link(location));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: location.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
    }

    /// Seed a List/Map walk: cursor at the first lookup entry, bound at `count`.
    ///
    /// `element_type` is unused today — the walk strides the entry table for
    /// every element type alike. It is threaded through because plan-57-D gives
    /// fixed-width-scalar lists no entry table at all, so the cursor there
    /// strides the *data region* by `payloadSize` instead. Adding the parameter
    /// when that lands would mean touching every cursor loop twice.
    pub(super) fn initialize_collection_loop_slots(
        &mut self,
        collection_slot: usize,
        cursor_slot: usize,
        remaining_slot: usize,
        element_type: &str,
    ) {
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        self.emit(abi::load_u64(
            &scratch8,
            abi::stack_pointer(),
            collection_slot,
        ));
        self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
        // kind 2 has no entry table to walk, so the cursor carries a byte OFFSET
        // from the data base instead of an entry pointer (plan-57-D). That keeps
        // `emit_load_collection_payload`'s `(collection, offset, length)` shape
        // usable unchanged for both representations.
        if kind2_payload_size(element_type).is_some() {
            self.emit(abi::move_immediate(&scratch10, "Integer", "0"));
        } else {
            self.emit(abi::add_immediate(
                &scratch10,
                &scratch8,
                COLLECTION_HEADER_SIZE,
            ));
        }
        self.emit(abi::store_u64(
            &scratch10,
            abi::stack_pointer(),
            cursor_slot,
        ));
        self.emit(abi::store_u64(
            &scratch9,
            abi::stack_pointer(),
            remaining_slot,
        ));
    }

    pub(super) fn load_collection_loop_item(
        &mut self,
        collection_slot: usize,
        cursor_slot: usize,
        element_type: &str,
    ) -> Result<VirtualRegister, String> {
        let scratch8 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), cursor_slot));
        if let Some(payload) = kind2_payload_size(element_type) {
            self.emit(abi::move_immediate(
                &scratch12,
                "Integer",
                &payload.to_string(),
            ));
            self.emit(abi::load_u64(
                &scratch8,
                abi::stack_pointer(),
                collection_slot,
            ));
            return self.emit_load_collection_payload(
                element_type,
                &scratch8,
                &scratch10,
                &scratch12,
            );
        }
        self.emit(abi::load_u64(
            &scratch11,
            &scratch10,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            &scratch12,
            &scratch10,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::load_u64(
            &scratch8,
            abi::stack_pointer(),
            collection_slot,
        ));
        self.emit_load_collection_payload(element_type, &scratch8, &scratch11, &scratch12)
    }

    /// Release a loop item that [`Self::load_collection_loop_item`] materialized
    /// fresh (bug-307).
    ///
    /// Every arm of `emit_load_collection_payload` except `String` hands back a
    /// alias -- a scalar loaded from the packed data region, or a pointer into it.
    /// The `String` arm is the exception: it `arena_alloc`s a fresh owned block
    /// (`emit_materialize_string_from_bytes`) because a packed String has no
    /// standalone header to point at. That block was moved into the callback's
    /// argument register and then never referenced again and never freed, so
    /// `forEach`/`transform`/`filter`/`reduce`/`reduceRight` over a `List OF
    /// String` grew arena RSS by one block per element per pass -- unbounded across
    /// repeated iteration, since nothing reclaimed it between passes.
    ///
    /// The callback receives it by value and does not take ownership, so the block
    /// is dead the moment the callback returns and freeing it here is safe. A
    /// callback that *returns* something derived from it returns a separate
    /// allocation.
    ///
    /// A no-op for every other element type, which allocate nothing to free.
    /// Takes the item by STACK SLOT, not by register, and deliberately so: the
    /// callback between materialization and this free is a call, and a call
    /// destroys every caller-saved register (see [[arena-alloc-clobbers-x14-x15]]).
    /// Reading the pointer back from a slot is what makes the free safe across it.
    pub(super) fn free_collection_loop_item(
        &mut self,
        item_slot: usize,
        element_type: &str,
    ) -> Result<(), String> {
        if element_type != "String" {
            return Ok(());
        }
        let size_slot = self.allocate_stack_object("loop_item_free_size", 8);
        self.emit_inlined_block_size_from_ptr_slot("String", item_slot, size_slot)?;
        self.emit(abi::load_u64(
            abi::return_register(),
            abi::stack_pointer(),
            item_slot,
        ));
        self.emit(abi::load_u64(abi::ARG[1], abi::stack_pointer(), size_slot));
        self.emit_arena_free_call();
        Ok(())
    }

    /// Step a List/Map walk one element on and branch back to `loop_label`.
    ///
    /// `element_type` is unused today for the same reason as
    /// [`Self::initialize_collection_loop_slots`]: the stride is
    /// `COLLECTION_ENTRY_SIZE` for every element type, and becomes `payloadSize`
    /// for a fixed-width list under plan-57-D.
    pub(super) fn advance_collection_loop(
        &mut self,
        cursor_slot: usize,
        remaining_slot: usize,
        loop_label: &str,
        element_type: &str,
    ) {
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let stride = kind2_payload_size(element_type).unwrap_or(COLLECTION_ENTRY_SIZE);
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), cursor_slot));
        self.emit(abi::add_immediate(&scratch10, &scratch10, stride));
        self.emit(abi::store_u64(
            &scratch10,
            abi::stack_pointer(),
            cursor_slot,
        ));
        self.emit(abi::load_u64(
            &scratch9,
            abi::stack_pointer(),
            remaining_slot,
        ));
        self.emit(abi::subtract_immediate(&scratch9, &scratch9, 1));
        self.emit(abi::store_u64(
            &scratch9,
            abi::stack_pointer(),
            remaining_slot,
        ));
        self.emit(abi::branch(loop_label));
    }

    /// The reverse-walk twin of [`Self::initialize_collection_loop_slots`]
    /// (plan-86-B, `reduceRight`): `remaining = count` as before, but the cursor
    /// starts at the LAST element so the walk runs from `count - 1` down to `0`.
    ///
    /// The cursor carries the same representation the forward walk uses — a byte
    /// offset from the data base for a kind-2 fixed-width list, an entry pointer
    /// otherwise — so [`Self::load_collection_loop_item`] reads it unchanged.
    /// When `count == 0` the loop's `remaining == 0` guard fires before the cursor
    /// is ever dereferenced, so the (then-negative) last-element address computed
    /// here is never used.
    pub(super) fn initialize_collection_loop_slots_reverse(
        &mut self,
        collection_slot: usize,
        cursor_slot: usize,
        remaining_slot: usize,
        element_type: &str,
    ) {
        let coll = self.temporary_vreg();
        let count = self.temporary_vreg();
        let stride_reg = self.temporary_vreg();
        let index = self.temporary_vreg();
        let offset = self.temporary_vreg();
        let cursor = self.temporary_vreg();
        let stride = kind2_payload_size(element_type).unwrap_or(COLLECTION_ENTRY_SIZE);
        self.emit(abi::load_u64(&coll, abi::stack_pointer(), collection_slot));
        self.emit(abi::load_u64(&count, &coll, COLLECTION_OFFSET_COUNT));
        // last index = count - 1; last-element byte offset = (count - 1) * stride.
        self.emit(abi::move_immediate(
            &stride_reg,
            "Integer",
            &stride.to_string(),
        ));
        self.emit(abi::subtract_immediate(&index, &count, 1));
        self.emit(abi::multiply_registers(&offset, &index, &stride_reg));
        if kind2_payload_size(element_type).is_some() {
            // kind 2: the cursor is the raw byte offset from the data base.
            self.emit(abi::move_register(&cursor, &offset));
        } else {
            // kind 0: the cursor is an entry pointer = base + HEADER + offset.
            self.emit(abi::add_immediate(&cursor, &coll, COLLECTION_HEADER_SIZE));
            self.emit(abi::add_registers(&cursor, &cursor, &offset));
        }
        self.emit(abi::store_u64(&cursor, abi::stack_pointer(), cursor_slot));
        self.emit(abi::store_u64(&count, abi::stack_pointer(), remaining_slot));
    }

    /// The reverse-walk twin of [`Self::advance_collection_loop`]: step the cursor
    /// one element BACK (toward index 0) and decrement `remaining`.
    pub(super) fn advance_collection_loop_reverse(
        &mut self,
        cursor_slot: usize,
        remaining_slot: usize,
        loop_label: &str,
        element_type: &str,
    ) {
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let stride = kind2_payload_size(element_type).unwrap_or(COLLECTION_ENTRY_SIZE);
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), cursor_slot));
        self.emit(abi::subtract_immediate(&scratch10, &scratch10, stride));
        self.emit(abi::store_u64(
            &scratch10,
            abi::stack_pointer(),
            cursor_slot,
        ));
        self.emit(abi::load_u64(
            &scratch9,
            abi::stack_pointer(),
            remaining_slot,
        ));
        self.emit(abi::subtract_immediate(&scratch9, &scratch9, 1));
        self.emit(abi::store_u64(
            &scratch9,
            abi::stack_pointer(),
            remaining_slot,
        ));
        self.emit(abi::branch(loop_label));
    }
}
