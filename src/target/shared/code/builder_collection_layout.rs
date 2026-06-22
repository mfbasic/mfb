use super::*;

impl CodeBuilder<'_> {
    pub(super) fn inline_collection_payload_size(&self, type_: &str) -> Option<usize> {
        if let Some(fields) = self.type_model.record_fields.get(type_) {
            return Some(8 * fields.len());
        }
        if let Some(union_name) = self.type_model.union_variants.get(type_) {
            return self.inline_collection_payload_size(union_name);
        }
        if self.type_model.union_names.contains(type_) {
            let max_fields = self
                .type_model
                .variants_for_union(type_)
                .filter_map(|variant| self.type_model.union_variant_fields.get(variant))
                .map(Vec::len)
                .max()
                .unwrap_or(0);
            return Some(8 * (1 + max_fields));
        }
        None
    }

    fn is_pointer_collection_payload_type(&self, type_: &str) -> bool {
        // A resource handle is a single 8-byte pointer to its record; a collection
        // slot stores a borrow of that pointer exactly like any other pointer
        // payload (§15.6). Resource *unions* carry a tag and are not pointer
        // payloads.
        is_collection_type(type_)
            || (crate::builtins::is_resource_type(type_)
                && !self.type_model.union_names.contains(type_))
    }

    /// Alignment, in bytes, that a packed collection payload of `type_` requires
    /// in the data region. 8-byte scalars (`Integer`/`Float`/`Fixed`), native
    /// collection/object pointers, and inline record/union slot payloads must
    /// begin at 8-byte boundaries; 1-byte scalars (`Boolean`/`Byte`) and UTF-8
    /// `String` bytes have no alignment requirement. `memory_layouts.md`
    /// (Scalar Storage) requires every payload to begin at an offset valid for
    /// its type, with padding bytes unobservable.
    pub(super) fn collection_payload_alignment(&self, type_: &str) -> usize {
        match type_ {
            "Boolean" | "Byte" | "String" => 1,
            "Integer" | "Float" | "Fixed" => 8,
            other if self.is_pointer_collection_payload_type(other) => 8,
            other if self.inline_collection_payload_size(other).is_some() => 8,
            _ => 1,
        }
    }

    /// Rounds the unsigned offset stored at `slot` up to `alignment`. A no-op
    /// for `alignment <= 1`. Uses x12/x13 as scratch so it does not disturb the
    /// x8-x11 registers used by the surrounding collection-writer code.
    pub(super) fn emit_align_offset_slot(&mut self, slot: usize, alignment: usize) {
        if alignment <= 1 {
            return;
        }
        let mask = !((alignment - 1) as u64);
        self.emit(abi::load_u64("x12", abi::stack_pointer(), slot));
        self.emit(abi::add_immediate("x12", "x12", alignment - 1));
        self.emit(abi::move_immediate("x13", "Integer", &mask.to_string()));
        self.emit(abi::and_registers("x12", "x12", "x13"));
        self.emit(abi::store_u64("x12", abi::stack_pointer(), slot));
    }

    /// Rounds the unsigned offset held in `reg` up to `alignment`, using
    /// `scratch` for the alignment mask. A no-op for `alignment <= 1`.
    pub(super) fn emit_align_offset_register(
        &mut self,
        reg: &str,
        alignment: usize,
        scratch: &str,
    ) {
        if alignment <= 1 {
            return;
        }
        let mask = !((alignment - 1) as u64);
        self.emit(abi::add_immediate(reg, reg, alignment - 1));
        self.emit(abi::move_immediate(scratch, "Integer", &mask.to_string()));
        self.emit(abi::and_registers(reg, reg, scratch));
    }

    pub(super) fn emit_copy_bytes(&mut self, dst: &str, src: &str, len: &str, prefix: &str) {
        let remaining = "x13";
        let loop_label = self.label(&format!("{prefix}_loop"));
        let done_label = self.label(&format!("{prefix}_done"));
        self.emit(abi::move_register(remaining, len));
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate(remaining, "0"));
        self.emit(abi::branch_eq(&done_label));
        self.emit(abi::load_u8("x14", src, 0));
        self.emit(abi::store_u8("x14", dst, 0));
        self.emit(abi::add_immediate(src, src, 1));
        self.emit(abi::add_immediate(dst, dst, 1));
        self.emit(abi::subtract_immediate(remaining, remaining, 1));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&done_label));
    }

    pub(super) fn emit_compare_bytes_branch(
        &mut self,
        left: &str,
        right: &str,
        len: &str,
        equal_label: &str,
        not_equal_label: &str,
        prefix: &str,
    ) {
        let remaining = "x5";
        let loop_label = self.label(&format!("{prefix}_loop"));
        self.emit(abi::move_register(remaining, len));
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate(remaining, "0"));
        self.emit(abi::branch_eq(equal_label));
        self.emit(abi::load_u8("x6", left, 0));
        self.emit(abi::load_u8("x7", right, 0));
        self.emit(abi::compare_registers("x6", "x7"));
        self.emit(abi::branch_ne(not_equal_label));
        self.emit(abi::add_immediate(left, left, 1));
        self.emit(abi::add_immediate(right, right, 1));
        self.emit(abi::subtract_immediate(remaining, remaining, 1));
        self.emit(abi::branch(&loop_label));
    }

    pub(super) fn emit_comparable_values_match_branch(
        &mut self,
        type_: &str,
        left: &str,
        right: &str,
        equal_label: &str,
        not_equal_label: &str,
    ) -> Result<(), String> {
        let left_slot = self.allocate_stack_object("compare_left_value", 8);
        let right_slot = self.allocate_stack_object("compare_right_value", 8);
        self.emit(abi::store_u64(left, abi::stack_pointer(), left_slot));
        self.emit(abi::store_u64(right, abi::stack_pointer(), right_slot));
        self.emit_comparable_values_match_branch_from_slots(
            type_,
            left_slot,
            right_slot,
            equal_label,
            not_equal_label,
        )
    }

    fn emit_comparable_values_match_branch_from_slots(
        &mut self,
        type_: &str,
        left_slot: usize,
        right_slot: usize,
        equal_label: &str,
        not_equal_label: &str,
    ) -> Result<(), String> {
        match type_ {
            "Nothing" => {
                self.emit(abi::branch(equal_label));
            }
            "Boolean" | "Byte" | "Integer" | "Fixed" => {
                self.emit(abi::load_u64("x6", abi::stack_pointer(), left_slot));
                self.emit(abi::load_u64("x7", abi::stack_pointer(), right_slot));
                self.emit(abi::compare_registers("x6", "x7"));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            "Float" => {
                self.emit(abi::load_u64("x6", abi::stack_pointer(), left_slot));
                self.emit(abi::load_u64("x7", abi::stack_pointer(), right_slot));
                self.emit(abi::float_move_d_from_x("d0", "x6"));
                self.emit(abi::float_move_d_from_x("d1", "x7"));
                self.emit(abi::float_compare_d("d0", "d1"));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            "String" => {
                let loop_label = self.label("compare_string_value_loop");
                self.emit(abi::load_u64("x2", abi::stack_pointer(), left_slot));
                self.emit(abi::load_u64("x4", abi::stack_pointer(), right_slot));
                self.emit(abi::load_u64("x5", "x2", 0));
                self.emit(abi::load_u64("x6", "x4", 0));
                self.emit(abi::compare_registers("x5", "x6"));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit(abi::add_immediate("x2", "x2", 8));
                self.emit(abi::add_immediate("x4", "x4", 8));
                self.emit(abi::label(&loop_label));
                self.emit(abi::compare_immediate("x5", "0"));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::load_u8("x6", "x2", 0));
                self.emit(abi::load_u8("x7", "x4", 0));
                self.emit(abi::compare_registers("x6", "x7"));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit(abi::add_immediate("x2", "x2", 1));
                self.emit(abi::add_immediate("x4", "x4", 1));
                self.emit(abi::subtract_immediate("x5", "x5", 1));
                self.emit(abi::branch(&loop_label));
            }
            other if self.type_model.record_fields.contains_key(other) => {
                let fields = self
                    .type_model
                    .record_fields
                    .get(other)
                    .cloned()
                    .ok_or_else(|| format!("native record type '{other}' does not resolve"))?;
                if fields.is_empty() {
                    self.emit(abi::branch(equal_label));
                    return Ok(());
                }
                for (index, (_, field_type)) in fields.iter().enumerate() {
                    let next_field = self.label("compare_record_next_field");
                    let field_left_slot = self.allocate_stack_object("compare_record_left", 8);
                    let field_right_slot = self.allocate_stack_object("compare_record_right", 8);
                    self.emit(abi::load_u64("x2", abi::stack_pointer(), left_slot));
                    self.emit(abi::load_u64("x4", abi::stack_pointer(), right_slot));
                    self.emit(abi::load_u64("x2", "x2", index * 8));
                    self.emit(abi::load_u64("x4", "x4", index * 8));
                    self.emit(abi::store_u64("x2", abi::stack_pointer(), field_left_slot));
                    self.emit(abi::store_u64("x4", abi::stack_pointer(), field_right_slot));
                    self.emit_comparable_values_match_branch_from_slots(
                        field_type,
                        field_left_slot,
                        field_right_slot,
                        &next_field,
                        not_equal_label,
                    )?;
                    self.emit(abi::label(&next_field));
                }
                self.emit(abi::branch(equal_label));
            }
            other
                if self
                    .type_model
                    .enum_members
                    .keys()
                    .any(|(enum_type, _)| enum_type == other) =>
            {
                self.emit(abi::load_u64("x6", abi::stack_pointer(), left_slot));
                self.emit(abi::load_u64("x7", abi::stack_pointer(), right_slot));
                self.emit(abi::compare_registers("x6", "x7"));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            other => {
                return Err(format!(
                    "native comparable comparison does not support type '{other}'"
                ));
            }
        }
        Ok(())
    }

    pub(super) fn materialize_inline_value_in_arena(
        &mut self,
        type_: &str,
        source: &str,
    ) -> Result<String, String> {
        let size = self
            .inline_collection_payload_size(type_)
            .ok_or_else(|| format!("native inline type '{type_}' has no fixed storage size"))?;
        let source_slot = self.allocate_stack_object("inline_value_source", 8);
        let result_slot = self.allocate_stack_object("inline_value_result", 8);
        let alloc_ok = self.label("inline_value_alloc_ok");
        self.emit(abi::store_u64(source, abi::stack_pointer(), source_slot));
        self.emit(abi::move_immediate(
            abi::return_register(),
            "Integer",
            &size.to_string(),
        ));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
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
        self.emit(abi::store_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), source_slot));
        self.emit(abi::move_immediate("x13", "Integer", &size.to_string()));
        self.emit_copy_bytes("x1", "x9", "x13", "inline_value_arena_copy");
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(result)
    }

    pub(super) fn lower_len(&mut self, value: &NirValue) -> Result<ValueResult, String> {
        let value = self.lower_value(value)?;
        if value.type_ == "String" {
            let count_slot = self.allocate_stack_object("len_string_count", 8);
            let remaining = self.allocate_register()?;
            let cursor = self.allocate_register()?;
            let byte = self.allocate_register()?;
            let mask = self.allocate_register()?;
            let loop_label = self.label("len_string_loop");
            let continuation_label = self.label("len_string_continuation");
            let next_label = self.label("len_string_next");
            let done_label = self.label("len_string_done");
            self.emit(abi::move_immediate(&byte, "Integer", "0"));
            self.emit(abi::store_u64(&byte, abi::stack_pointer(), count_slot));
            self.emit(abi::load_u64(&remaining, &value.location, 0));
            self.emit(abi::add_immediate(&cursor, &value.location, 8));
            self.emit(abi::move_immediate(&mask, "Integer", "192"));
            self.emit(abi::label(&loop_label));
            self.emit(abi::compare_immediate(&remaining, "0"));
            self.emit(abi::branch_eq(&done_label));
            self.emit(abi::load_u8(&byte, &cursor, 0));
            self.emit(abi::and_registers(&byte, &byte, &mask));
            self.emit(abi::compare_immediate(&byte, "128"));
            self.emit(abi::branch_eq(&continuation_label));
            self.emit(abi::load_u64(&byte, abi::stack_pointer(), count_slot));
            self.emit(abi::add_immediate(&byte, &byte, 1));
            self.emit(abi::store_u64(&byte, abi::stack_pointer(), count_slot));
            self.emit(abi::branch(&next_label));
            self.emit(abi::label(&continuation_label));
            self.emit(abi::label(&next_label));
            self.emit(abi::add_immediate(&cursor, &cursor, 1));
            self.emit(abi::subtract_immediate(&remaining, &remaining, 1));
            self.emit(abi::branch(&loop_label));
            self.emit(abi::label(&done_label));
            let register = self.allocate_register()?;
            self.emit(abi::load_u64(&register, abi::stack_pointer(), count_slot));
            return Ok(ValueResult {
                type_: "Integer".to_string(),
                location: register,
                text: format!("len({})", value.text),
            });
        } else if is_collection_type(&value.type_) {
            let register = self.allocate_register()?;
            self.emit(abi::load_u64(
                &register,
                &value.location,
                COLLECTION_OFFSET_COUNT,
            ));
            return Ok(ValueResult {
                type_: "Integer".to_string(),
                location: register,
                text: format!("len({})", value.text),
            });
        } else {
            return Err(format!(
                "native len does not accept argument type '{}'",
                value.type_
            ));
        }
    }

    pub(super) fn lower_empty_collection(&mut self, type_: &str) -> Result<ValueResult, String> {
        self.lower_collection_values(type_, Vec::new(), "empty collection")
    }

    pub(super) fn lower_list_literal(
        &mut self,
        type_: &str,
        values: &[NirValue],
    ) -> Result<ValueResult, String> {
        let mut slots = Vec::new();
        for value in values {
            let value = self.lower_value(value)?;
            let slot = self.allocate_stack_object("collection_value", 8);
            self.emit(abi::store_u64(&value.location, abi::stack_pointer(), slot));
            slots.push(CollectionValueSlot {
                key: None,
                value: PayloadSlot {
                    slot,
                    type_: value.type_,
                },
            });
        }
        self.lower_collection_values(type_, slots, "list")
    }

    pub(super) fn lower_map_literal(
        &mut self,
        type_: &str,
        entries: &[(NirValue, NirValue)],
    ) -> Result<ValueResult, String> {
        let mut slots = Vec::new();
        for (key, value) in entries {
            let key = self.lower_value(key)?;
            let key_slot = self.allocate_stack_object("collection_key", 8);
            self.emit(abi::store_u64(
                &key.location,
                abi::stack_pointer(),
                key_slot,
            ));
            let value = self.lower_value(value)?;
            let value_slot = self.allocate_stack_object("collection_value", 8);
            self.emit(abi::store_u64(
                &value.location,
                abi::stack_pointer(),
                value_slot,
            ));
            slots.push(CollectionValueSlot {
                key: Some(PayloadSlot {
                    slot: key_slot,
                    type_: key.type_,
                }),
                value: PayloadSlot {
                    slot: value_slot,
                    type_: value.type_,
                },
            });
        }
        self.lower_collection_values(type_, slots, "map")
    }

    pub(super) fn lower_collection_values(
        &mut self,
        type_: &str,
        slots: Vec<CollectionValueSlot>,
        label: &str,
    ) -> Result<ValueResult, String> {
        self.reset_temporary_registers();
        let layout = CollectionTypeLayout::from_type(type_)
            .ok_or_else(|| format!("native code collection type '{type_}' is not supported"))?;
        let count = slots.len();
        let data_len_slot = self.allocate_stack_object("collection_data_len", 8);
        self.emit(abi::move_immediate("x8", "Integer", "0"));
        self.emit(abi::store_u64("x8", abi::stack_pointer(), data_len_slot));
        for slot in &slots {
            if let Some(key) = &slot.key {
                // Map entries pack a key then a value; round each payload's start
                // offset up to its type alignment so the running data length
                // accounts for the same padding the writer inserts below. List
                // payloads are homogeneous and size-aligned, so they never need
                // padding (no key present).
                let key_alignment = self.collection_payload_alignment(&key.type_);
                self.emit_align_offset_slot(data_len_slot, key_alignment);
                self.emit_add_payload_length(data_len_slot, key)?;
                let value_alignment = self.collection_payload_alignment(&slot.value.type_);
                self.emit_align_offset_slot(data_len_slot, value_alignment);
            }
            self.emit_add_payload_length(data_len_slot, &slot.value)?;
        }

        let collection_slot = self.allocate_stack_object("collection_literal", 8);
        let alloc_ok = self.label("collection_alloc_ok");
        self.emit(abi::load_u64("x8", abi::stack_pointer(), data_len_slot));
        self.emit(abi::move_immediate(
            "x9",
            "Integer",
            &(COLLECTION_HEADER_SIZE + count * COLLECTION_ENTRY_SIZE).to_string(),
        ));
        self.emit(abi::add_registers(abi::return_register(), "x8", "x9"));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
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
        self.emit(abi::store_u64("x1", abi::stack_pointer(), collection_slot));

        self.emit_write_collection_header(&layout, count, data_len_slot);

        let data_offset_slot = self.allocate_stack_object("collection_data_offset", 8);
        self.emit(abi::move_immediate("x8", "Integer", "0"));
        self.emit(abi::store_u64("x8", abi::stack_pointer(), data_offset_slot));

        for (index, slot) in slots.iter().enumerate() {
            self.emit_write_collection_entry(collection_slot, index, slot, data_offset_slot)?;
        }
        let register = self.allocate_register()?;
        self.emit(abi::load_u64(
            &register,
            abi::stack_pointer(),
            collection_slot,
        ));
        Ok(ValueResult {
            type_: type_.to_string(),
            location: register,
            text: format!("{label} {type_}"),
        })
    }

    pub(super) fn emit_write_collection_header(
        &mut self,
        layout: &CollectionTypeLayout,
        count: usize,
        data_len_slot: usize,
    ) {
        self.emit(abi::move_immediate("x8", "Byte", &layout.kind.to_string()));
        self.emit(abi::store_u8("x8", "x1", COLLECTION_OFFSET_KIND));
        self.emit(abi::move_immediate(
            "x8",
            "Byte",
            &layout.key_type_code.to_string(),
        ));
        self.emit(abi::store_u8("x8", "x1", COLLECTION_OFFSET_KEY_TYPE));
        self.emit(abi::move_immediate(
            "x8",
            "Byte",
            &layout.value_type_code.to_string(),
        ));
        self.emit(abi::store_u8("x8", "x1", COLLECTION_OFFSET_VALUE_TYPE));
        self.emit(abi::move_immediate("x8", "Byte", "1"));
        self.emit(abi::store_u8("x8", "x1", COLLECTION_OFFSET_FLAGS_VERSION));
        self.emit(abi::move_immediate("x8", "Integer", &count.to_string()));
        self.emit(abi::store_u64("x8", "x1", COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64("x8", "x1", COLLECTION_OFFSET_CAPACITY));
        self.emit(abi::load_u64("x8", abi::stack_pointer(), data_len_slot));
        self.emit(abi::store_u64("x8", "x1", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::store_u64("x8", "x1", COLLECTION_OFFSET_DATA_CAPACITY));
    }

    pub(super) fn emit_write_collection_entry(
        &mut self,
        collection_slot: usize,
        index: usize,
        slot: &CollectionValueSlot,
        data_offset_slot: usize,
    ) -> Result<(), String> {
        let entry_offset = COLLECTION_HEADER_SIZE + index * COLLECTION_ENTRY_SIZE;
        let key_len_slot = if let Some(key) = &slot.key {
            Some(self.emit_payload_length_to_stack(key, "collection_key_len")?)
        } else {
            None
        };
        let value_len_slot =
            self.emit_payload_length_to_stack(&slot.value, "collection_value_len")?;
        let collection_register = "x8";
        self.emit(abi::load_u64(
            collection_register,
            abi::stack_pointer(),
            collection_slot,
        ));

        self.emit(abi::move_immediate(
            "x9",
            "Byte",
            &COLLECTION_ENTRY_FLAG_USED.to_string(),
        ));
        self.emit(abi::store_u8(
            "x9",
            collection_register,
            entry_offset + COLLECTION_ENTRY_OFFSET_FLAGS,
        ));

        if let Some(key_len_slot) = key_len_slot {
            // Align the key payload start to its type alignment before recording
            // its offset (map entries only; lists have no key).
            let key_alignment =
                self.collection_payload_alignment(&slot.key.as_ref().unwrap().type_);
            self.emit_align_offset_slot(data_offset_slot, key_alignment);
            self.emit(abi::load_u64(
                collection_register,
                abi::stack_pointer(),
                collection_slot,
            ));
            self.emit(abi::load_u64("x10", abi::stack_pointer(), data_offset_slot));
            self.emit(abi::store_u64(
                "x10",
                collection_register,
                entry_offset + COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
            ));
            self.emit(abi::load_u64("x11", abi::stack_pointer(), key_len_slot));
            self.emit(abi::store_u64(
                "x11",
                collection_register,
                entry_offset + COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
            ));
            self.emit_copy_payload_to_collection(
                collection_slot,
                key_len_slot,
                slot.key.as_ref().unwrap(),
                data_offset_slot,
            )?;
        } else {
            self.emit(abi::move_immediate("x10", "Integer", "0"));
            self.emit(abi::store_u64(
                "x10",
                collection_register,
                entry_offset + COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
            ));
            self.emit(abi::store_u64(
                "x10",
                collection_register,
                entry_offset + COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
            ));
        }

        // Align the value payload start to its type alignment before recording
        // its offset. Only map entries can leave the cursor unaligned (a
        // variable-length or 1-byte key preceding an 8-byte value); list
        // payloads are homogeneous and stay naturally aligned.
        if slot.key.is_some() {
            let value_alignment = self.collection_payload_alignment(&slot.value.type_);
            self.emit_align_offset_slot(data_offset_slot, value_alignment);
        }
        self.emit(abi::load_u64(
            collection_register,
            abi::stack_pointer(),
            collection_slot,
        ));
        self.emit(abi::load_u64("x10", abi::stack_pointer(), data_offset_slot));
        self.emit(abi::store_u64(
            "x10",
            collection_register,
            entry_offset + COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64("x11", abi::stack_pointer(), value_len_slot));
        self.emit(abi::store_u64(
            "x11",
            collection_register,
            entry_offset + COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit_copy_payload_to_collection(
            collection_slot,
            value_len_slot,
            &slot.value,
            data_offset_slot,
        )?;
        Ok(())
    }

    pub(super) fn emit_add_payload_length(
        &mut self,
        total_slot: usize,
        payload: &PayloadSlot,
    ) -> Result<(), String> {
        let len_slot = self.emit_payload_length_to_stack(payload, "collection_payload_len")?;
        self.emit(abi::load_u64("x8", abi::stack_pointer(), total_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), len_slot));
        self.emit(abi::add_registers("x8", "x8", "x9"));
        self.emit(abi::store_u64("x8", abi::stack_pointer(), total_slot));
        Ok(())
    }

    pub(super) fn emit_payload_length_to_stack(
        &mut self,
        payload: &PayloadSlot,
        label: &str,
    ) -> Result<usize, String> {
        let len_slot = self.allocate_stack_object(label, 8);
        match payload.type_.as_str() {
            "Boolean" | "Byte" => {
                self.emit(abi::move_immediate("x8", "Integer", "1"));
            }
            "Integer" | "Float" | "Fixed" => {
                self.emit(abi::move_immediate("x8", "Integer", "8"));
            }
            "String" => {
                self.emit(abi::load_u64("x8", abi::stack_pointer(), payload.slot));
                self.emit(abi::load_u64("x8", "x8", 0));
            }
            other if self.is_pointer_collection_payload_type(other) => {
                self.emit(abi::move_immediate("x8", "Integer", "8"));
            }
            other if self.inline_collection_payload_size(other).is_some() => {
                let size = self
                    .inline_collection_payload_size(other)
                    .expect("guard ensures inline payload size exists");
                self.emit(abi::move_immediate("x8", "Integer", &size.to_string()));
            }
            other => {
                return Err(format!(
                    "native collection packed payload does not support type '{other}'"
                ));
            }
        }
        self.emit(abi::store_u64("x8", abi::stack_pointer(), len_slot));
        Ok(len_slot)
    }

    pub(super) fn emit_copy_payload_to_collection(
        &mut self,
        collection_slot: usize,
        len_slot: usize,
        payload: &PayloadSlot,
        data_offset_slot: usize,
    ) -> Result<(), String> {
        self.emit(abi::load_u64("x8", abi::stack_pointer(), collection_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), data_offset_slot));
        self.emit(abi::add_immediate("x10", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64("x11", "x8", COLLECTION_OFFSET_CAPACITY));
        self.emit(abi::move_immediate(
            "x12",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x11", "x11", "x12"));
        self.emit(abi::add_registers("x10", "x10", "x11"));
        self.emit(abi::add_registers("x10", "x10", "x9"));

        match payload.type_.as_str() {
            "Boolean" | "Byte" => {
                self.emit(abi::load_u64("x12", abi::stack_pointer(), payload.slot));
                self.emit(abi::store_u8("x12", "x10", 0));
            }
            "Integer" | "Float" | "Fixed" => {
                self.emit(abi::load_u64("x12", abi::stack_pointer(), payload.slot));
                self.emit(abi::store_u64("x12", "x10", 0));
            }
            "String" => {
                let loop_label = self.label("collection_copy_string_loop");
                let done_label = self.label("collection_copy_string_done");
                self.emit(abi::load_u64("x12", abi::stack_pointer(), payload.slot));
                self.emit(abi::add_immediate("x12", "x12", 8));
                self.emit(abi::load_u64("x13", abi::stack_pointer(), len_slot));
                self.emit(abi::label(&loop_label));
                self.emit(abi::compare_immediate("x13", "0"));
                self.emit(abi::branch_eq(&done_label));
                self.emit(abi::load_u8("x14", "x12", 0));
                self.emit(abi::store_u8("x14", "x10", 0));
                self.emit(abi::add_immediate("x12", "x12", 1));
                self.emit(abi::add_immediate("x10", "x10", 1));
                self.emit(abi::subtract_immediate("x13", "x13", 1));
                self.emit(abi::branch(&loop_label));
                self.emit(abi::label(&done_label));
            }
            other if self.is_pointer_collection_payload_type(other) => {
                self.emit(abi::load_u64("x12", abi::stack_pointer(), payload.slot));
                self.emit(abi::store_u64("x12", "x10", 0));
            }
            other if self.inline_collection_payload_size(other).is_some() => {
                self.emit(abi::load_u64("x12", abi::stack_pointer(), payload.slot));
                self.emit(abi::load_u64("x13", abi::stack_pointer(), len_slot));
                self.emit_copy_bytes("x10", "x12", "x13", "collection_copy_inline");
            }
            other => {
                return Err(format!(
                    "native collection packed payload does not support type '{other}'"
                ));
            }
        }

        self.emit(abi::load_u64("x8", abi::stack_pointer(), data_offset_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), len_slot));
        self.emit(abi::add_registers("x8", "x8", "x9"));
        self.emit(abi::store_u64("x8", abi::stack_pointer(), data_offset_slot));
        Ok(())
    }

    pub(super) fn emit_collection_data_pointer(&mut self, dst: &str, collection: &str) {
        let capacity = "x6";
        let entry_size = "x7";
        self.emit(abi::move_register(capacity, collection));
        self.emit(abi::add_immediate(dst, collection, COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64(
            capacity,
            capacity,
            COLLECTION_OFFSET_CAPACITY,
        ));
        self.emit(abi::move_immediate(
            entry_size,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(capacity, capacity, entry_size));
        self.emit(abi::add_registers(dst, dst, capacity));
    }

    pub(super) fn emit_load_collection_payload(
        &mut self,
        type_: &str,
        collection: &str,
        offset: &str,
        length: &str,
    ) -> Result<String, String> {
        let collection_input = "x3";
        let offset_input = "x4";
        let length_input = "x5";
        self.emit(abi::move_register(collection_input, collection));
        self.emit(abi::move_register(offset_input, offset));
        self.emit(abi::move_register(length_input, length));
        let data = self.allocate_register()?;
        self.emit_collection_data_pointer(&data, collection_input);
        self.emit(abi::add_registers(&data, &data, offset_input));
        match type_ {
            "Boolean" | "Byte" => {
                let result = self.allocate_register()?;
                self.emit(abi::load_u8(&result, &data, 0));
                Ok(result)
            }
            "Integer" | "Float" | "Fixed" => {
                let result = self.allocate_register()?;
                self.emit(abi::load_u64(&result, &data, 0));
                Ok(result)
            }
            "String" => self.emit_materialize_string_from_bytes(&data, length_input),
            other if self.is_pointer_collection_payload_type(other) => {
                let result = self.allocate_register()?;
                self.emit(abi::load_u64(&result, &data, 0));
                Ok(result)
            }
            other if self.inline_collection_payload_size(other).is_some() => Ok(data),
            other => Err(format!(
                "native collection packed payload does not support type '{other}'"
            )),
        }
    }

    pub(super) fn emit_materialize_string_from_bytes(
        &mut self,
        source: &str,
        length: &str,
    ) -> Result<String, String> {
        let source_slot = self.allocate_stack_object("collection_string_source", 8);
        let length_slot = self.allocate_stack_object("collection_string_length", 8);
        let result_slot = self.allocate_stack_object("collection_string_result", 8);
        let alloc_ok = self.label("collection_string_alloc_ok");
        let copy_loop = self.label("collection_string_copy_loop");
        let copy_done = self.label("collection_string_copy_done");

        self.emit(abi::store_u64(source, abi::stack_pointer(), source_slot));
        self.emit(abi::store_u64(length, abi::stack_pointer(), length_slot));
        self.emit(abi::add_immediate(abi::return_register(), length, 9));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
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
        self.emit(abi::store_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64("x12", abi::stack_pointer(), length_slot));
        self.emit(abi::store_u64("x12", "x1", 0));
        self.emit(abi::add_immediate("x13", "x1", 8));
        self.emit(abi::load_u64("x14", abi::stack_pointer(), source_slot));
        self.emit(abi::label(&copy_loop));
        self.emit(abi::compare_immediate("x12", "0"));
        self.emit(abi::branch_eq(&copy_done));
        self.emit(abi::load_u8("x15", "x14", 0));
        self.emit(abi::store_u8("x15", "x13", 0));
        self.emit(abi::add_immediate("x14", "x14", 1));
        self.emit(abi::add_immediate("x13", "x13", 1));
        self.emit(abi::subtract_immediate("x12", "x12", 1));
        self.emit(abi::branch(&copy_loop));
        self.emit(abi::label(&copy_done));
        self.emit(abi::move_immediate("x15", "Integer", "0"));
        self.emit(abi::store_u8("x15", "x13", 0));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(result)
    }

    pub(super) fn emit_collection_payload_match_branch(
        &mut self,
        type_: &str,
        collection: &str,
        offset: &str,
        length: &str,
        value: &str,
        equal_label: &str,
        not_equal_label: &str,
    ) -> Result<(), String> {
        let data = self.allocate_register()?;
        self.emit_collection_data_pointer(&data, collection);
        self.emit(abi::add_registers(&data, &data, offset));
        match type_ {
            "Boolean" | "Byte" => {
                let candidate = self.allocate_register()?;
                self.emit(abi::load_u8(&candidate, &data, 0));
                self.emit(abi::compare_registers(&candidate, value));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            "Integer" | "Float" | "Fixed" => {
                let candidate = self.allocate_register()?;
                self.emit(abi::load_u64(&candidate, &data, 0));
                self.emit(abi::compare_registers(&candidate, value));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            "String" => {
                let value_len = self.allocate_register()?;
                let value_cursor = self.allocate_register()?;
                let remaining = self.allocate_register()?;
                let packed_byte = self.allocate_register()?;
                let value_byte = self.allocate_register()?;
                let loop_label = self.label("collection_string_match_loop");
                self.emit(abi::load_u64(&value_len, value, 0));
                self.emit(abi::compare_registers(length, &value_len));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit(abi::add_immediate(&value_cursor, value, 8));
                self.emit(abi::move_register(&remaining, length));
                self.emit(abi::label(&loop_label));
                self.emit(abi::compare_immediate(&remaining, "0"));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::load_u8(&packed_byte, &data, 0));
                self.emit(abi::load_u8(&value_byte, &value_cursor, 0));
                self.emit(abi::compare_registers(&packed_byte, &value_byte));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit(abi::add_immediate(&data, &data, 1));
                self.emit(abi::add_immediate(&value_cursor, &value_cursor, 1));
                self.emit(abi::subtract_immediate(&remaining, &remaining, 1));
                self.emit(abi::branch(&loop_label));
            }
            other if self.is_pointer_collection_payload_type(other) => {
                let candidate = self.allocate_register()?;
                self.emit(abi::load_u64(&candidate, &data, 0));
                self.emit(abi::compare_registers(&candidate, value));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            other if self.type_model.record_fields.contains_key(other) => {
                self.emit_comparable_values_match_branch(
                    other,
                    &data,
                    value,
                    equal_label,
                    not_equal_label,
                )?;
            }
            other if self.inline_collection_payload_size(other).is_some() => {
                self.emit_compare_bytes_branch(
                    &data,
                    value,
                    length,
                    equal_label,
                    not_equal_label,
                    "collection_inline_match",
                );
            }
            other => {
                return Err(format!(
                    "native collection packed payload does not support type '{other}'"
                ));
            }
        }
        Ok(())
    }

    pub(super) fn emit_collection_payload_matches_value_branch(
        &mut self,
        type_: &str,
        collection: &str,
        offset: &str,
        length: &str,
        value: &str,
        equal_label: &str,
        not_equal_label: &str,
    ) -> Result<(), String> {
        self.emit(abi::move_register("x2", collection));
        self.emit(abi::move_register("x3", offset));
        self.emit_collection_data_pointer("x2", "x2");
        self.emit(abi::add_registers("x2", "x2", "x3"));
        match type_ {
            "Boolean" | "Byte" => {
                self.emit(abi::load_u8("x6", "x2", 0));
                self.emit(abi::compare_registers("x6", value));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            "Integer" | "Float" | "Fixed" => {
                self.emit(abi::load_u64("x6", "x2", 0));
                self.emit(abi::compare_registers("x6", value));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            "String" => {
                let loop_label = self.label("collection_string_value_match_loop");
                self.emit(abi::load_u64("x3", value, 0));
                self.emit(abi::compare_registers(length, "x3"));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit(abi::add_immediate("x4", value, 8));
                self.emit(abi::move_register("x5", length));
                self.emit(abi::label(&loop_label));
                self.emit(abi::compare_immediate("x5", "0"));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::load_u8("x6", "x2", 0));
                self.emit(abi::load_u8("x7", "x4", 0));
                self.emit(abi::compare_registers("x6", "x7"));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit(abi::add_immediate("x2", "x2", 1));
                self.emit(abi::add_immediate("x4", "x4", 1));
                self.emit(abi::subtract_immediate("x5", "x5", 1));
                self.emit(abi::branch(&loop_label));
            }
            other if self.is_pointer_collection_payload_type(other) => {
                self.emit(abi::load_u64("x6", "x2", 0));
                self.emit(abi::compare_registers("x6", value));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            other if self.type_model.record_fields.contains_key(other) => {
                self.emit_comparable_values_match_branch(
                    other,
                    "x2",
                    value,
                    equal_label,
                    not_equal_label,
                )?;
            }
            other if self.inline_collection_payload_size(other).is_some() => {
                self.emit_compare_bytes_branch(
                    "x2",
                    value,
                    length,
                    equal_label,
                    not_equal_label,
                    "collection_inline_value_match",
                );
            }
            other => {
                return Err(format!(
                    "native collection packed payload does not support type '{other}'"
                ));
            }
        }
        Ok(())
    }

    pub(super) fn emit_collection_payloads_match_branch(
        &mut self,
        type_: &str,
        left_collection: &str,
        left_offset: &str,
        left_length: &str,
        right_collection: &str,
        right_offset: &str,
        right_length: &str,
        equal_label: &str,
        not_equal_label: &str,
    ) -> Result<(), String> {
        self.emit(abi::move_register("x2", left_collection));
        self.emit(abi::move_register("x3", left_offset));
        self.emit(abi::move_register("x4", right_collection));
        self.emit(abi::move_register("x5", right_offset));
        self.emit_collection_data_pointer("x2", "x2");
        self.emit(abi::add_registers("x2", "x2", "x3"));
        self.emit_collection_data_pointer("x4", "x4");
        self.emit(abi::add_registers("x4", "x4", "x5"));
        match type_ {
            "Boolean" | "Byte" => {
                self.emit(abi::load_u8("x6", "x2", 0));
                self.emit(abi::load_u8("x7", "x4", 0));
                self.emit(abi::compare_registers("x6", "x7"));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            "Integer" | "Float" | "Fixed" => {
                self.emit(abi::load_u64("x6", "x2", 0));
                self.emit(abi::load_u64("x7", "x4", 0));
                self.emit(abi::compare_registers("x6", "x7"));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            "String" => {
                let loop_label = self.label("collection_payload_string_match_loop");
                self.emit(abi::compare_registers(left_length, right_length));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit(abi::move_register("x5", left_length));
                self.emit(abi::label(&loop_label));
                self.emit(abi::compare_immediate("x5", "0"));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::load_u8("x6", "x2", 0));
                self.emit(abi::load_u8("x7", "x4", 0));
                self.emit(abi::compare_registers("x6", "x7"));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit(abi::add_immediate("x2", "x2", 1));
                self.emit(abi::add_immediate("x4", "x4", 1));
                self.emit(abi::subtract_immediate("x5", "x5", 1));
                self.emit(abi::branch(&loop_label));
            }
            other if self.is_pointer_collection_payload_type(other) => {
                self.emit(abi::load_u64("x6", "x2", 0));
                self.emit(abi::load_u64("x7", "x4", 0));
                self.emit(abi::compare_registers("x6", "x7"));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            other if self.type_model.record_fields.contains_key(other) => {
                self.emit(abi::compare_registers(left_length, right_length));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit_comparable_values_match_branch(
                    other,
                    "x2",
                    "x4",
                    equal_label,
                    not_equal_label,
                )?;
            }
            other if self.inline_collection_payload_size(other).is_some() => {
                self.emit(abi::compare_registers(left_length, right_length));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit_compare_bytes_branch(
                    "x2",
                    "x4",
                    left_length,
                    equal_label,
                    not_equal_label,
                    "collection_inline_pair_match",
                );
            }
            other => {
                return Err(format!(
                    "native collection packed payload does not support type '{other}'"
                ));
            }
        }
        Ok(())
    }
}
