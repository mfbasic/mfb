use super::*;

impl CodeBuilder<'_> {
    /// `collections::get`/`getOr` extract an element as a borrow into the
    /// container's data region for inline composite / nested-collection payloads
    /// (`emit_load_collection_payload`). By value semantics `get` returns an
    /// **owned** value the caller may bind, store, and free, so copy such a
    /// borrow into a standalone arena block (scalars are by-value and `String`
    /// is already materialized fresh, so they pass through). plan-02 Phase 8.
    pub(super) fn materialize_owned_element(
        &mut self,
        result: ValueResult,
    ) -> Result<ValueResult, String> {
        if self.is_freeable_flat_value(&result.type_) && result.type_ != "String" {
            let copied = self.copy_flat_block(&result.type_, &result.location)?;
            return Ok(ValueResult {
                type_: result.type_,
                location: copied,
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
        self.emit(abi::move_immediate(&index, "Integer", "0"));
        self.emit(abi::add_immediate(
            &entry,
            &collection_register,
            COLLECTION_HEADER_SIZE,
        ));

        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_registers(&index, &count));
        self.emit(abi::branch_ge(&not_found));
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
        self.emit_collection_payload_match_branch(
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
        self.emit(abi::add_immediate(&entry, &entry, COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate(&index, &index, 1));
        self.emit(abi::branch(&loop_label));

        self.emit(abi::label(&not_found));
        self.emit(abi::move_immediate(&result, "Boolean", "false"));
        self.emit(abi::label(&done));

        Ok(ValueResult {
            type_: "Boolean".to_string(),
            location: result,
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
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
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

        let Some((key_type, _)) = map_type_parts(&collection.type_) else {
            return Err(format!(
                "native collection hasKey does not accept {}",
                collection.type_
            ));
        };
        if key.type_ != key_type {
            return Err(format!(
                "native collection hasKey key must be {}, got {}",
                key_type, key.type_
            ));
        }

        if Self::map_key_probe_eligible(&key_type) {
            self.reset_temporary_registers();
            let not_found = self.label("has_key_not_found");
            let done = self.label("has_key_done");
            let _ = self.emit_map_probe(collection_slot, key_slot, &key_type, &not_found)?;
            let result = self.allocate_register()?;
            self.emit(abi::move_immediate(&result, "Boolean", "true"));
            self.emit(abi::branch(&done));
            self.emit(abi::label(&not_found));
            self.emit(abi::move_immediate(&result, "Boolean", "false"));
            self.emit(abi::label(&done));
            return Ok(ValueResult {
                type_: "Boolean".to_string(),
                location: result,
                text: format!("hasKey({}) [hash]", collection.type_),
            });
        }

        self.reset_temporary_registers();
        let result = self.allocate_register()?;
        let loop_label = self.label("has_key_loop");
        let found = self.label("has_key_found");
        let next = self.label("has_key_next");
        let not_found = self.label("has_key_not_found");
        let done = self.label("has_key_done");

        self.emit(abi::load_u64(
            &scratch8,
            abi::stack_pointer(),
            collection_slot,
        ));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), key_slot));
        self.emit(abi::load_u64(
            &scratch10,
            &scratch8,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::move_immediate(&scratch11, "Integer", "0"));
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch8,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_registers(&scratch11, &scratch10));
        self.emit(abi::branch_ge(&not_found));
        self.emit(abi::load_u64(
            &scratch13,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::load_u64(
            &scratch14,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit_collection_payload_matches_value_branch(
            &key_type, &scratch8, &scratch13, &scratch14, &scratch9, &found, &next,
        )?;
        self.emit(abi::label(&found));
        self.emit(abi::move_immediate(&result, "Boolean", "true"));
        self.emit(abi::branch(&done));
        self.emit(abi::label(&next));
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch12,
            COLLECTION_ENTRY_SIZE,
        ));
        self.emit(abi::add_immediate(&scratch11, &scratch11, 1));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&not_found));
        self.emit(abi::move_immediate(&result, "Boolean", "false"));
        self.emit(abi::label(&done));

        Ok(ValueResult {
            type_: "Boolean".to_string(),
            location: result,
            text: format!("hasKey({}, {})", collection.type_, key_type),
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
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch15, &scratch9, &scratch14));
        self.emit(abi::add_immediate(
            abi::return_register(),
            &scratch15,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(
            abi::return_register(),
            abi::return_register(),
            &scratch11,
        ));
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
        self.emit(abi::store_u64(abi::RET[1], abi::stack_pointer(), result_slot));
        self.emit(abi::move_immediate(
            &scratch13,
            "Byte",
            &layout.kind.to_string(),
        ));
        self.emit(abi::store_u8(&scratch13, abi::RET[1], COLLECTION_OFFSET_KIND));
        self.emit(abi::move_immediate(
            &scratch13,
            "Byte",
            &layout.key_type_code.to_string(),
        ));
        self.emit(abi::store_u8(&scratch13, abi::RET[1], COLLECTION_OFFSET_KEY_TYPE));
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
        self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64(&scratch9, abi::RET[1], COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64(&scratch9, abi::RET[1], COLLECTION_OFFSET_CAPACITY));
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
        self.emit(abi::load_u64(abi::RET[1], abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch8,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_immediate(&scratch17, abi::RET[1], COLLECTION_HEADER_SIZE));
        self.emit_collection_data_pointer(&scratch20, &scratch8);
        self.emit(abi::move_immediate(
            &scratch14,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
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
        self.emit(abi::store_u8(
            &scratch22,
            &scratch17,
            COLLECTION_ENTRY_OFFSET_FLAGS,
        ));
        self.emit(abi::move_immediate(&scratch22, "Integer", "0"));
        self.emit(abi::store_u64(
            &scratch22,
            &scratch17,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::store_u64(
            &scratch22,
            &scratch17,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::load_u64(&scratch22, &scratch12, offset_field));
        self.emit(abi::load_u64(&scratch23, &scratch12, length_field));
        self.emit(abi::store_u64(
            &scratch11,
            &scratch17,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::store_u64(
            &scratch23,
            &scratch17,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
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
        self.emit(abi::load_u64(
            &scratch23,
            &scratch17,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers(&scratch11, &scratch11, &scratch23));
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch12,
            COLLECTION_ENTRY_SIZE,
        ));
        self.emit(abi::add_immediate(
            &scratch17,
            &scratch17,
            COLLECTION_ENTRY_SIZE,
        ));
        self.emit(abi::add_immediate(&scratch10, &scratch10, 1));
        self.emit(abi::branch(&copy_loop));
        self.emit(abi::label(&copy_done));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: format!("List OF {element_type}"),
            location: result,
            text: if project_key {
                format!("keys({})", collection.type_)
            } else {
                format!("values({})", collection.type_)
            },
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
        self.emit(abi::store_u64(&list.location, abi::stack_pointer(), collection_slot));
        let start = self.lower_value(&args[1])?;
        self.emit(abi::store_u64(&start.location, abi::stack_pointer(), start_slot));
        let stop = self.lower_value(&args[2])?;
        self.emit(abi::store_u64(&stop.location, abi::stack_pointer(), stop_slot));

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
        self.emit(abi::move_immediate(&s14, "Integer", &COLLECTION_ENTRY_SIZE.to_string()));
        self.emit(abi::multiply_registers(&s13, &s10, &s14));
        self.emit(abi::add_immediate(&s15, &s8, COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers(&s15, &s15, &s13));
        self.emit(abi::move_immediate(&s13, "Integer", "0"));
        self.emit(abi::move_immediate(&s17, "Integer", "0"));
        let len_loop = self.label("slice_len_loop");
        let len_done = self.label("slice_len_done");
        self.emit(abi::label(&len_loop));
        self.emit(abi::compare_registers(&s17, &s12));
        self.emit(abi::branch_ge(&len_done));
        self.emit(abi::load_u64(&s20, &s15, COLLECTION_ENTRY_OFFSET_VALUE_LENGTH));
        self.emit(abi::add_registers(&s13, &s13, &s20));
        self.emit(abi::add_immediate(&s15, &s15, COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate(&s17, &s17, 1));
        self.emit(abi::branch(&len_loop));
        self.emit(abi::label(&len_done));
        self.emit(abi::store_u64(&s13, abi::stack_pointer(), data_len_slot));

        // Allocate HEADER + count'*ENTRY + data_len.
        self.emit(abi::load_u64(&s12, abi::stack_pointer(), count_slot));
        self.emit(abi::load_u64(&s13, abi::stack_pointer(), data_len_slot));
        self.emit(abi::move_immediate(&s14, "Integer", &COLLECTION_ENTRY_SIZE.to_string()));
        self.emit(abi::multiply_registers(&s15, &s12, &s14));
        self.emit(abi::add_immediate(abi::return_register(), &s15, COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers(
            abi::return_register(),
            abi::return_register(),
            &s13,
        ));
        self.emit(abi::move_immediate(abi::ARG[1], "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
        let alloc_ok = self.label("slice_alloc_ok");
        self.emit(abi::compare_immediate(abi::return_register(), RESULT_OK_TAG));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(abi::RET[1], abi::stack_pointer(), result_slot));

        // Header.
        self.emit(abi::move_immediate(&s13, "Byte", &layout.kind.to_string()));
        self.emit(abi::store_u8(&s13, abi::RET[1], COLLECTION_OFFSET_KIND));
        self.emit(abi::move_immediate(&s13, "Byte", &layout.key_type_code.to_string()));
        self.emit(abi::store_u8(&s13, abi::RET[1], COLLECTION_OFFSET_KEY_TYPE));
        self.emit(abi::move_immediate(&s13, "Byte", &layout.value_type_code.to_string()));
        self.emit(abi::store_u8(&s13, abi::RET[1], COLLECTION_OFFSET_VALUE_TYPE));
        self.emit(abi::move_immediate(&s13, "Byte", "1"));
        self.emit(abi::store_u8(&s13, abi::RET[1], COLLECTION_OFFSET_FLAGS_VERSION));
        self.emit(abi::load_u64(&s12, abi::stack_pointer(), count_slot));
        self.emit(abi::store_u64(&s12, abi::RET[1], COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64(&s12, abi::RET[1], COLLECTION_OFFSET_CAPACITY));
        self.emit(abi::load_u64(&s13, abi::stack_pointer(), data_len_slot));
        self.emit(abi::store_u64(&s13, abi::RET[1], COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::store_u64(&s13, abi::RET[1], COLLECTION_OFFSET_DATA_CAPACITY));

        // Copy pass: for each entry in [start, start+count') copy its value payload
        // into the new blob and rewrite the entry's value_offset to the running one.
        self.emit(abi::load_u64(&s8, abi::stack_pointer(), collection_slot));
        self.emit(abi::load_u64(abi::RET[1], abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64(&s10, abi::stack_pointer(), start_slot));
        self.emit(abi::load_u64(&s9, abi::stack_pointer(), count_slot));
        self.emit(abi::move_immediate(&s14, "Integer", &COLLECTION_ENTRY_SIZE.to_string()));
        self.emit(abi::multiply_registers(&s13, &s10, &s14));
        self.emit(abi::add_immediate(&s12, &s8, COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers(&s12, &s12, &s13));
        self.emit(abi::add_immediate(&s17, abi::RET[1], COLLECTION_HEADER_SIZE));
        self.emit_collection_data_pointer(&s20, &s8);
        self.emit(abi::multiply_registers(&s21, &s9, &s14));
        self.emit(abi::add_registers(&s21, &s17, &s21));
        self.emit(abi::move_immediate(&s11, "Integer", "0"));
        self.emit(abi::move_immediate(&s10, "Integer", "0"));
        let copy_loop = self.label("slice_copy_loop");
        let copy_done = self.label("slice_copy_done");
        let copy_bytes = self.label("slice_copy_bytes");
        let copy_bytes_done = self.label("slice_copy_bytes_done");
        self.emit(abi::label(&copy_loop));
        self.emit(abi::compare_registers(&s10, &s9));
        self.emit(abi::branch_ge(&copy_done));
        self.emit(abi::move_immediate(&s22, "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()));
        self.emit(abi::store_u8(&s22, &s17, COLLECTION_ENTRY_OFFSET_FLAGS));
        self.emit(abi::move_immediate(&s22, "Integer", "0"));
        self.emit(abi::store_u64(&s22, &s17, COLLECTION_ENTRY_OFFSET_KEY_OFFSET));
        self.emit(abi::store_u64(&s22, &s17, COLLECTION_ENTRY_OFFSET_KEY_LENGTH));
        self.emit(abi::load_u64(&s22, &s12, COLLECTION_ENTRY_OFFSET_VALUE_OFFSET));
        self.emit(abi::load_u64(&s23, &s12, COLLECTION_ENTRY_OFFSET_VALUE_LENGTH));
        self.emit(abi::store_u64(&s11, &s17, COLLECTION_ENTRY_OFFSET_VALUE_OFFSET));
        self.emit(abi::store_u64(&s23, &s17, COLLECTION_ENTRY_OFFSET_VALUE_LENGTH));
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
        self.emit(abi::load_u64(&s23, &s17, COLLECTION_ENTRY_OFFSET_VALUE_LENGTH));
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
            location: result,
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
        self.emit(abi::load_u64(
            &scratch12,
            &scratch11,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit_collection_data_pointer(&scratch15, &scratch8);
        self.emit(abi::add_registers(&scratch15, &scratch15, &scratch12));
        match element_type.as_str() {
            "Integer" => {
                self.emit(abi::load_u64(&scratch16, &scratch15, 0));
                self.emit_checked_integer_add(&scratch14, &scratch14, &scratch16)?;
            }
            "Float" => {
                self.emit(abi::load_u64(&scratch16, &scratch15, 0));
                self.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[0], &scratch14));
                self.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[1], &scratch16));
                self.emit(abi::float_add_d(abi::FP_SCRATCH[0], abi::FP_SCRATCH[0], abi::FP_SCRATCH[1]));
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
            COLLECTION_ENTRY_SIZE,
        ));
        self.emit(abi::add_immediate(&scratch10, &scratch10, 1));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&done));
        let result = self.allocate_register()?;
        self.emit(abi::move_register(&result, &scratch14));
        Ok(ValueResult {
            type_: element_type,
            location: result,
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
        self.emit(abi::add_immediate(
            &scratch10,
            &scratch8,
            COLLECTION_HEADER_SIZE,
        ));
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
        let item =
            self.emit_load_collection_payload(&element_type, &scratch8, &scratch11, &scratch12)?;
        self.emit(abi::move_register(&abi::argument_register(0)?, &item));
        self.emit(abi::load_u64(&scratch17, abi::stack_pointer(), action_slot));
        self.emit_direct_callable_branch(&scratch17);
        self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
        self.emit(abi::branch_eq(&ok_label));
        // A failing callback: forEach owns no accumulator, so no cleanup — under
        // an inline TRAP the raw error routes to the capture point (plan-26-B).
        self.emit_callback_failure_exit(None)?;
        self.emit(abi::label(&ok_label));
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), cursor_slot));
        self.emit(abi::add_immediate(
            &scratch10,
            &scratch10,
            COLLECTION_ENTRY_SIZE,
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
        self.initialize_collection_loop_slots(collection_slot, cursor_slot, remaining_slot);

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
        // The output accumulator is a private, uniquely-owned buffer, so append
        // each transformed item in place with geometric headroom (plan-01 §4.2)
        // — amortized O(1) instead of the O(n) splice the singleton+insert did.
        self.lower_list_append_in_place(output_slot, item_slot, &output_list_type, &output_type)?;
        self.advance_collection_loop(cursor_slot, remaining_slot, &loop_label);
        self.emit(abi::label(&done));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), output_slot));
        Ok(ValueResult {
            type_: output_list_type,
            location: result,
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
        self.initialize_collection_loop_slots(collection_slot, cursor_slot, remaining_slot);

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
        self.advance_collection_loop(cursor_slot, remaining_slot, &loop_label);
        self.emit(abi::label(&done));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), output_slot));
        Ok(ValueResult {
            type_: collection.type_.clone(),
            location: result,
            text: format!("filter({}, {})", collection.type_, action.text),
        })
    }

    pub(super) fn lower_collection_reduce_call(
        &mut self,
        args: &[NirValue],
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
        self.require_direct_callable("reduce", &action)?;
        let action_slot = self.allocate_stack_object("reduce_action", 8);
        self.emit(abi::store_u64(
            &action.location,
            abi::stack_pointer(),
            action_slot,
        ));
        let cursor_slot = self.allocate_stack_object("reduce_cursor", 8);
        let remaining_slot = self.allocate_stack_object("reduce_remaining", 8);
        self.initialize_collection_loop_slots(collection_slot, cursor_slot, remaining_slot);

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
        // A failing reducer: no cleanup — the accumulator may still alias the
        // borrowed seed (no owning copy is inserted for it), so freeing it here
        // would be a use-after-free after the handler recovers; the success path
        // likewise leaves intermediate accumulators unfreed (plan-26-B).
        self.emit_callback_failure_exit(None)?;
        self.emit(abi::label(&ok_label));
        self.emit(abi::store_u64(
            RESULT_VALUE_REGISTER,
            abi::stack_pointer(),
            accumulator_slot,
        ));
        self.advance_collection_loop(cursor_slot, remaining_slot, &loop_label);
        self.emit(abi::label(&done));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(
            &result,
            abi::stack_pointer(),
            accumulator_slot,
        ));
        Ok(ValueResult {
            type_: initial.type_,
            location: result,
            text: format!(
                "reduce({}, {}, {})",
                collection.type_, initial.text, action.text
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
    /// passes `None`: its accumulator may still alias the **borrowed** seed on an
    /// iteration-1 failure (the seed reaches codegen as a bare local with no owning
    /// copy), so freeing it would be a use-after-free after the handler recovers —
    /// and the success path already leaves intermediate accumulators unfreed, so
    /// not freeing here matches it exactly.
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

    pub(super) fn emit_direct_callable_branch(&mut self, location: &str) {
        let saved_env_slot = self.allocate_stack_object("closure_saved_env", 8);
        // Infallible vreg minters: an exhaustion under `-regalloc bump` is recorded
        // and surfaced by `run_register_allocation` instead of panicking (bug-70).
        let code_register = self.temporary_vreg();
        let env_register = self.temporary_vreg();
        self.emit(abi::store_u64(
            CLOSURE_ENV_REGISTER,
            abi::stack_pointer(),
            saved_env_slot,
        ));
        self.emit(abi::load_u64(&code_register, location, CLOSURE_OFFSET_CODE));
        self.emit(abi::load_u64(&env_register, location, CLOSURE_OFFSET_ENV));
        self.emit(abi::move_register(CLOSURE_ENV_REGISTER, &env_register));
        self.emit_callable_branch(&code_register);
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

    pub(super) fn initialize_collection_loop_slots(
        &mut self,
        collection_slot: usize,
        cursor_slot: usize,
        remaining_slot: usize,
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
        self.emit(abi::add_immediate(
            &scratch10,
            &scratch8,
            COLLECTION_HEADER_SIZE,
        ));
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
    ) -> Result<String, String> {
        let scratch8 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), cursor_slot));
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

    pub(super) fn advance_collection_loop(
        &mut self,
        cursor_slot: usize,
        remaining_slot: usize,
        loop_label: &str,
    ) {
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), cursor_slot));
        self.emit(abi::add_immediate(
            &scratch10,
            &scratch10,
            COLLECTION_ENTRY_SIZE,
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
        self.emit(abi::branch(loop_label));
    }
}
