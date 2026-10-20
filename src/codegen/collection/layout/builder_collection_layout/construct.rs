//! plan-176-Q: constructing collections — `len`, empty collections, literals, headers, entries
//! and payloads. The list-layout helpers live in
//! `collection/layout/builder_collection_layout/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    pub(crate) fn lower_len(&mut self, value: &NirValue) -> Result<ValueResult, String> {
        // bug-760 B1: a `Map`/`Set`'s length is `count - dead`, read straight off a
        // binding that may hold tombstones — no settle, so `len` stays O(1).
        self.map_settle_exempt_armed = true;
        let value = self.lower_value(value)?;
        if value.type_ == ParameterType::String {
            let count_slot = self.allocate_stack_object("len_string_count", 8);
            let remaining = self.allocate_register();
            let cursor = self.allocate_register();
            let byte = self.allocate_register();
            let mask = self.allocate_register();
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
            let register = self.allocate_register();
            self.emit(abi::load_u64(&register, abi::stack_pointer(), count_slot));
            Ok(ValueResult {
                origin: None,
                type_: ParameterType::Integer,
                location: Operand::from(register),
                text: format!("len({})", value.text),
            })
        } else if typed_is_collection_type(&value.type_) {
            let register = self.allocate_register();
            self.emit(abi::load_u64(
                &register,
                &value.location,
                COLLECTION_OFFSET_COUNT,
            ));
            if crate::codegen::collection::map::map_mutate::is_map_or_set_type(&value.type_) {
                let dead = self.allocate_register();
                self.emit(abi::load_u32(
                    &dead,
                    &value.location,
                    COLLECTION_OFFSET_MAP_STATE,
                ));
                self.emit(abi::shift_right_immediate(
                    &dead,
                    &dead,
                    MAP_DEAD_ENTRIES_SHIFT,
                ));
                self.emit(abi::subtract_registers(&register, &register, &dead));
            }
            Ok(ValueResult {
                origin: None,
                type_: ParameterType::Integer,
                location: Operand::from(register),
                text: format!("len({})", value.text),
            })
        } else {
            Err(format!(
                "native len does not accept argument type '{}'",
                value.type_
            ))
        }
    }

    pub(crate) fn lower_empty_collection(
        &mut self,
        type_: &ParameterType,
    ) -> Result<ValueResult, String> {
        self.lower_collection_values(type_, Vec::new(), "empty collection")
    }

    pub(crate) fn lower_list_literal(
        &mut self,
        type_: &ParameterType,
        values: &[NirValue],
    ) -> Result<ValueResult, String> {
        let mut slots = Vec::new();
        for value_node in values {
            let value = self.lower_value_stored(value_node)?;
            // Observation boundary: a `Float` list element must be finite
            // (plan-17).
            self.observe_float(value_node, &value)?;
            // The element is stored into the collection payload through an
            // integer slot, so a `d`-native float is materialized first (plan-01
            // float-dnative).
            let value = self.materialize_value(value)?;
            let slot = self.allocate_stack_object("collection_value", 8);
            self.emit(abi::store_u64(&value.location, abi::stack_pointer(), slot));
            slots.push(CollectionValueSlot {
                key: None,
                value: PayloadSlot {
                    slot,
                    type_: value.type_.clone(),
                },
            });
        }
        self.lower_collection_values(type_, slots, "list")
    }

    /// Lower a `Set OF T { … }` literal (plan-63). A `Set` block is Map-shaped
    /// with a 1-byte `Boolean` value (always TRUE); the literal is built by
    /// inserting each element into a growing, uniquely-owned buffer so duplicates
    /// collapse. Mirrors [`lower_set_add`](Self::lower_set_add)'s idioms, but the
    /// buffer starts empty and is mutated IN PLACE (no per-element copy — the
    /// literal owns its buffer exclusively).
    pub(crate) fn lower_set_literal(
        &mut self,
        type_: &ParameterType,
        values: &[NirValue],
    ) -> Result<ValueResult, String> {
        let element_type = crate::codegen::engine::types::typed_set_element_type(type_)
            .cloned()
            .ok_or_else(|| format!("lower_set_literal: not a set type '{type_}'"))?;
        // Start from an empty set and spill its pointer to a stack slot so it can
        // be reloaded and rewritten after each (possibly reallocating) insert.
        let result = self.lower_empty_collection(type_)?;
        let set_slot = self.allocate_stack_object("set_lit", 8);
        self.emit(abi::store_u64(
            &result.location,
            abi::stack_pointer(),
            set_slot,
        ));
        // A reusable 1-byte `Boolean` TRUE — every element maps to it.
        let true_slot = self.allocate_stack_object("set_lit_true", 8);
        let true_reg = self.allocate_register();
        self.emit(abi::move_immediate(&true_reg, "Boolean", "true"));
        self.emit(abi::store_u64(&true_reg, abi::stack_pointer(), true_slot));
        for value_node in values {
            let item = self.lower_value_stored(value_node)?;
            // Observation boundary: a `Float` element must be finite (plan-17).
            self.observe_float(value_node, &item)?;
            // A `d`-native float element is materialized into a GPR before the
            // integer-slot store (plan-01 float-dnative).
            let item = self.materialize_value(item)?;
            let item_slot = self.allocate_stack_object("set_lit_item", 8);
            self.store_value_at(&item, abi::stack_pointer(), item_slot);
            // In-place insert on the uniquely-owned literal buffer, then store the
            // (possibly reallocated) pointer back.
            let inserted = self.lower_map_set_in_place(
                set_slot,
                item_slot,
                true_slot,
                type_,
                &element_type,
                &ParameterType::Boolean,
                None,
            )?;
            self.emit(abi::store_u64(
                &inserted.location,
                abi::stack_pointer(),
                set_slot,
            ));
        }
        let register = self.allocate_register();
        self.emit(abi::load_u64(&register, abi::stack_pointer(), set_slot));
        Ok(ValueResult {
            origin: None,
            type_: type_.clone(),
            location: Operand::from(register),
            text: format!("set literal {type_}"),
        })
    }

    pub(crate) fn lower_map_literal(
        &mut self,
        type_: &ParameterType,
        entries: &[(NirValue, NirValue)],
    ) -> Result<ValueResult, String> {
        let mut slots = Vec::new();
        for (key_node, value_node) in entries {
            let key = self.lower_value(key_node)?;
            // Observation boundary: a `Float` map key/value must be finite
            // (a non-finite key is rejected at insert; plan-17). Map keys still
            // *compare* bitwise — only finiteness is enforced here.
            self.observe_float(key_node, &key)?;
            // A `d`-native float key/value is materialized into a GPR before the
            // integer-slot store (plan-01 float-dnative).
            let key = self.materialize_value(key)?;
            let key_slot = self.allocate_stack_object("collection_key", 8);
            self.emit(abi::store_u64(
                &key.location,
                abi::stack_pointer(),
                key_slot,
            ));
            let value = self.lower_value_stored(value_node)?;
            self.observe_float(value_node, &value)?;
            let value = self.materialize_value(value)?;
            let value_slot = self.allocate_stack_object("collection_value", 8);
            self.emit(abi::store_u64(
                &value.location,
                abi::stack_pointer(),
                value_slot,
            ));
            slots.push(CollectionValueSlot {
                key: Some(PayloadSlot {
                    slot: key_slot,
                    type_: key.type_.clone(),
                }),
                value: PayloadSlot {
                    slot: value_slot,
                    type_: value.type_.clone(),
                },
            });
        }
        self.lower_collection_values(type_, slots, "map")
    }

    pub(crate) fn lower_collection_values(
        &mut self,
        type_: &ParameterType,
        slots: Vec<CollectionValueSlot>,
        label: &str,
    ) -> Result<ValueResult, String> {
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        self.reset_temporary_registers();
        // A list literal whose *declared* element type is `Unknown` must not be
        // laid out from that placeholder. `expression_type` falls back to
        // `List OF Unknown` whenever it cannot type the first element — a call to
        // a builtin like `toByte` is enough (`[toByte(1), toByte(2)]`) — but the
        // allocation below sizes the entry array from the first slot's ACTUAL
        // value type. Layout, header, and the scope-drop free size all key off
        // `type_`, so leaving the placeholder in place makes them disagree with
        // what was reserved: the block is allocated entry-free and later freed as
        // `capacity * 40` bytes past its end, corrupting the arena free list —
        // bug-02's exact failure mode. Refine the type from the elements so every
        // consumer sees the one type the block was actually built to.
        let first_list_element = slots
            .first()
            .filter(|slot| slot.key.is_none())
            .map(|slot| slot.value.type_.clone());
        let first_list_element = first_list_element.as_ref();
        let refined_type = refined_list_literal_type(type_, first_list_element);
        let type_ = refined_type.as_ref().unwrap_or(type_);
        let layout = CollectionTypeLayout::from_type(type_)
            .ok_or_else(|| format!("native code collection type '{type_}' is not supported"))?;
        let count = slots.len();
        let data_len_slot = self.allocate_stack_object("collection_data_len", 8);
        self.emit(abi::move_immediate(&scratch8, "Integer", "0"));
        self.emit(abi::store_u64(
            &scratch8,
            abi::stack_pointer(),
            data_len_slot,
        ));
        for slot in &slots {
            if let Some(key) = &slot.key {
                // Map entries pack a key then a value; round each payload's start
                // offset up to its type alignment so the running data length
                // accounts for the same padding the writer inserts below.
                let key_alignment = self.collection_payload_alignment(&key.type_);
                self.emit_align_offset_slot(data_len_slot, key_alignment);
                self.emit_add_payload_length(data_len_slot, key)?;
                let value_alignment = self.collection_payload_alignment(&slot.value.type_);
                self.emit_align_offset_slot(data_len_slot, value_alignment);
            } else {
                // List payloads are homogeneous. Fixed-size elements pack with no
                // gap (their size is a whole multiple of their alignment), but a
                // *variable-length* element (a record with an inlined String
                // field, a data union, or a flat nested collection) can end on a
                // non-8 boundary and leave the next element's U64 slots unaligned,
                // so round the running length up before appending the next one
                // (bug-147.4). The writer below applies the identical rounding;
                // `list_element_padding_alignment` returns 1 for every fixed-size
                // or byte-addressed payload, keeping primitive lists byte-identical.
                let value_alignment = self.list_element_padding_alignment(&slot.value.type_);
                self.emit_align_offset_slot(data_len_slot, value_alignment);
            }
            self.emit_add_payload_length(data_len_slot, &slot.value)?;
        }

        let collection_slot = self.allocate_stack_object("collection_literal", 8);
        let alloc_ok = self.label("collection_alloc_ok");
        self.emit(abi::load_u64(
            &scratch8,
            abi::stack_pointer(),
            data_len_slot,
        ));
        // A map *or* set reserves a `2*capacity` u64 bucket array past the data
        // region; capacity == count for a literal, so fold it into the constant.
        // `collection_has_buckets` keeps a Set's reservation in lockstep with the
        // sizing/copy/free paths (plan-63-B) — omitting it would size a Set literal
        // short and corrupt the arena on the lazy bucket build.
        let bucket_bytes = if matches!(&type_, ParameterType::MapOf(..) | ParameterType::SetOf(_)) {
            count * MAP_BUCKET_SIZE * 2
        } else {
            0
        };
        // The lookup-entry stride for this literal's element type: zero for a
        // fixed-width list, which drops the entry array from the allocation
        // entirely (plan-57-D). Taken from the first slot's value type — a
        // literal's elements are all the declared element type — and zero only
        // for a keyless (list) slot, since a map keeps its entries.
        let literal_entry_stride = match slots.first() {
            Some(slot) if slot.key.is_none() => list_entry_stride(&slot.value.type_),
            _ => COLLECTION_ENTRY_SIZE,
        };
        self.emit(abi::move_immediate(
            &scratch9,
            "Integer",
            &(COLLECTION_HEADER_SIZE + count * literal_entry_stride + bucket_bytes).to_string(),
        ));
        self.emit(abi::add_registers(
            abi::return_register(),
            &scratch8,
            &scratch9,
        ));
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&alloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            collection_slot,
        ));

        self.emit_write_collection_header(&layout, count, data_len_slot);

        let data_offset_slot = self.allocate_stack_object("collection_data_offset", 8);
        self.emit(abi::move_immediate(&scratch8, "Integer", "0"));
        self.emit(abi::store_u64(
            &scratch8,
            abi::stack_pointer(),
            data_offset_slot,
        ));

        for (index, slot) in slots.iter().enumerate() {
            self.emit_write_collection_entry(collection_slot, index, slot, data_offset_slot)?;
        }
        let register = self.allocate_register();
        self.emit(abi::load_u64(
            &register,
            abi::stack_pointer(),
            collection_slot,
        ));
        Ok(ValueResult {
            origin: None,
            type_: type_.clone(),
            location: Operand::from(register),
            text: format!("{label} {type_}"),
        })
    }

    pub(crate) fn emit_write_collection_header(
        &mut self,
        layout: &CollectionTypeLayout,
        count: usize,
        data_len_slot: usize,
    ) {
        let scratch8 = self.temporary_vreg();
        self.emit(abi::move_immediate(
            &scratch8,
            "Byte",
            &layout.kind.to_string(),
        ));
        self.emit(abi::store_u8(
            &scratch8,
            abi::mfb_return(1),
            COLLECTION_OFFSET_KIND,
        ));
        self.emit(abi::move_immediate(
            &scratch8,
            "Byte",
            &layout.key_type_code.to_string(),
        ));
        self.emit(abi::store_u8(
            &scratch8,
            abi::mfb_return(1),
            COLLECTION_OFFSET_KEY_TYPE,
        ));
        self.emit(abi::move_immediate(
            &scratch8,
            "Byte",
            &layout.value_type_code.to_string(),
        ));
        self.emit(abi::store_u8(
            &scratch8,
            abi::mfb_return(1),
            COLLECTION_OFFSET_VALUE_TYPE,
        ));
        self.emit(abi::move_immediate(&scratch8, "Byte", "1"));
        self.emit(abi::store_u8(
            &scratch8,
            abi::mfb_return(1),
            COLLECTION_OFFSET_FLAGS_VERSION,
        ));
        // Map hash index built lazily on first probe, and no tombstones (bug-760
        // B1) — one word (no-op field for lists).
        self.emit(abi::move_immediate(&scratch8, "Integer", "0"));
        self.emit(abi::store_u32(
            &scratch8,
            abi::mfb_return(1),
            COLLECTION_OFFSET_MAP_STATE,
        ));
        self.emit(abi::move_immediate(
            &scratch8,
            "Integer",
            &count.to_string(),
        ));
        self.emit(abi::store_u64(
            &scratch8,
            abi::mfb_return(1),
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::store_u64(
            &scratch8,
            abi::mfb_return(1),
            COLLECTION_OFFSET_CAPACITY,
        ));
        self.emit(abi::load_u64(
            &scratch8,
            abi::stack_pointer(),
            data_len_slot,
        ));
        self.emit(abi::store_u64(
            &scratch8,
            abi::mfb_return(1),
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::store_u64(
            &scratch8,
            abi::mfb_return(1),
            COLLECTION_OFFSET_DATA_CAPACITY,
        ));
    }

    pub(crate) fn emit_write_collection_entry(
        &mut self,
        collection_slot: usize,
        index: usize,
        slot: &CollectionValueSlot,
        data_offset_slot: usize,
    ) -> Result<(), String> {
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let entry_offset = COLLECTION_HEADER_SIZE + index * COLLECTION_ENTRY_SIZE;
        // A kind-2 list has no lookup entry to write: element `i` is at
        // `dataBase + i * payloadSize` by construction (plan-57-D). The payload
        // copies below still run — they are what advances `data_offset_slot` —
        // only the entry-field stores are skipped. A map slot (`key.is_some()`)
        // always writes its entry.
        let writes_entry = slot.key.is_some() || list_entry_stride(&slot.value.type_) != 0;
        let key_len_slot = if let Some(key) = &slot.key {
            Some(self.emit_payload_length_to_stack(key, "collection_key_len")?)
        } else {
            None
        };
        let value_len_slot =
            self.emit_payload_length_to_stack(&slot.value, "collection_value_len")?;
        let collection_register = &scratch8;
        self.emit(abi::load_u64(
            collection_register,
            abi::stack_pointer(),
            collection_slot,
        ));

        self.emit(abi::move_immediate(
            &scratch9,
            "Byte",
            &COLLECTION_ENTRY_FLAG_USED.to_string(),
        ));
        if writes_entry {
            self.emit(abi::store_u8(
                &scratch9,
                collection_register,
                entry_offset + COLLECTION_ENTRY_OFFSET_FLAGS,
            ));
        }

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
            self.emit(abi::load_u64(
                &scratch10,
                abi::stack_pointer(),
                data_offset_slot,
            ));
            if writes_entry {
                self.emit(abi::store_u64(
                    &scratch10,
                    collection_register,
                    entry_offset + COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
                ));
            }
            self.emit(abi::load_u64(
                &scratch11,
                abi::stack_pointer(),
                key_len_slot,
            ));
            if writes_entry {
                self.emit(abi::store_u64(
                    &scratch11,
                    collection_register,
                    entry_offset + COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
                ));
            }
            let no_stride = ParameterType::named("");
            self.emit_copy_payload_to_collection(
                collection_slot,
                key_len_slot,
                slot.key.as_ref().unwrap(),
                data_offset_slot,
                if slot.key.is_some() {
                    // The EMPTY nominal is this tree's "no declared type" marker
                    // (`type_utils::is_unset_type`); it was spelled `""` here.
                    // `list_entry_stride` answers the same for both.
                    &no_stride
                } else {
                    &slot.value.type_
                },
            )?;
        } else {
            self.emit(abi::move_immediate(&scratch10, "Integer", "0"));
            if writes_entry {
                self.emit(abi::store_u64(
                    &scratch10,
                    collection_register,
                    entry_offset + COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
                ));
            }
            if writes_entry {
                self.emit(abi::store_u64(
                    &scratch10,
                    collection_register,
                    entry_offset + COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
                ));
            }
        }

        // Align the value payload start before recording its offset. Map entries
        // round to the value's type alignment (a variable-length or 1-byte key
        // preceding an 8-byte value can leave the cursor unaligned). List entries
        // only need rounding for a *variable-length* element whose size may not be
        // a multiple of 8 (bug-147.4); `list_element_padding_alignment` returns 1
        // for fixed-size list payloads, so those stay byte-identical. This
        // mirrors the allocation-size pass exactly, so the recorded offset never
        // runs past the allocated block.
        let value_alignment = if slot.key.is_some() {
            self.collection_payload_alignment(&slot.value.type_)
        } else {
            self.list_element_padding_alignment(&slot.value.type_)
        };
        self.emit_align_offset_slot(data_offset_slot, value_alignment);
        self.emit(abi::load_u64(
            collection_register,
            abi::stack_pointer(),
            collection_slot,
        ));
        self.emit(abi::load_u64(
            &scratch10,
            abi::stack_pointer(),
            data_offset_slot,
        ));
        if writes_entry {
            self.emit(abi::store_u64(
                &scratch10,
                collection_register,
                entry_offset + COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
            ));
        }
        self.emit(abi::load_u64(
            &scratch11,
            abi::stack_pointer(),
            value_len_slot,
        ));
        if writes_entry {
            self.emit(abi::store_u64(
                &scratch11,
                collection_register,
                entry_offset + COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ));
        }
        let no_stride = ParameterType::named("");
        self.emit_copy_payload_to_collection(
            collection_slot,
            value_len_slot,
            &slot.value,
            data_offset_slot,
            if slot.key.is_some() {
                // See the sibling call above: the EMPTY nominal is the tree's
                // "no declared type" marker, spelled `""` before plan-111-E.
                &no_stride
            } else {
                &slot.value.type_
            },
        )?;
        Ok(())
    }

    pub(crate) fn emit_add_payload_length(
        &mut self,
        total_slot: usize,
        payload: &PayloadSlot,
    ) -> Result<(), String> {
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let len_slot = self.emit_payload_length_to_stack(payload, "collection_payload_len")?;
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), total_slot));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), len_slot));
        self.emit(abi::add_registers(&scratch8, &scratch8, &scratch9));
        self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), total_slot));
        Ok(())
    }

    pub(crate) fn emit_payload_length_to_stack(
        &mut self,
        payload: &PayloadSlot,
        label: &str,
    ) -> Result<usize, String> {
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let len_slot = self.allocate_stack_object(label, 8);
        match &payload.type_ {
            ParameterType::Nothing | ParameterType::Boolean | ParameterType::Byte => {
                self.emit(abi::move_immediate(&scratch8, "Integer", "1"));
            }
            type_ if type_.is_named("Scalar") => {
                self.emit(abi::move_immediate(&scratch8, "Integer", "4"));
            }
            ParameterType::Integer
            | ParameterType::Float
            | ParameterType::Fixed
            | ParameterType::Money => {
                self.emit(abi::move_immediate(&scratch8, "Integer", "8"));
            }
            // bug-549: an enum member is its ordinal, one word wide. Without
            // this arm `List OF <enum>` type-checks — the spec lists enum types
            // as comparable, right beside the primitives — and then dies at
            // lowering on "packed payload does not support type".
            other if self.is_enum_type(other) => {
                self.emit(abi::move_immediate(&scratch8, "Integer", "8"));
            }
            // A function value is a single 8-byte closure pointer, stored by
            // reference exactly like a pointer payload (bug-73).
            other if matches!(other, ParameterType::Func(..)) => {
                self.emit(abi::move_immediate(&scratch8, "Integer", "8"));
            }
            ParameterType::String => {
                self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), payload.slot));
                self.emit(abi::load_u64(&scratch8, &scratch8, 0));
            }
            other if self.is_pointer_collection_payload_type(other) => {
                self.emit(abi::move_immediate(&scratch8, "Integer", "8"));
            }
            other if self.record_has_inline_data(other) => {
                // A record with inlined String fields is variable-length; size
                // its full flat block at runtime (plan-02 §4.2).
                self.emit_record_block_size_to_slot(other, payload.slot, len_slot)?;
                return Ok(len_slot);
            }
            other if self.union_is_data(other) => {
                // A data union is variable-length; read its `size` word at +8
                // (plan-02 §4.3).
                self.emit_data_union_size_to_slot(payload.slot, len_slot);
                return Ok(len_slot);
            }
            other if typed_is_collection_type(other) => {
                // A flat nested collection is inlined as its own block; size it at
                // runtime (plan-02 §4.4).
                self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), payload.slot));
                self.emit_flat_block_size(other, &scratch8, &scratch9, &scratch10)?;
                self.emit(abi::store_u64(&scratch9, abi::stack_pointer(), len_slot));
                return Ok(len_slot);
            }
            other if self.inline_collection_payload_size(other).is_some() => {
                let size = self
                    .inline_collection_payload_size(other)
                    .expect("guard ensures inline payload size exists");
                self.emit(abi::move_immediate(&scratch8, "Integer", &size.to_string()));
            }
            other => {
                return Err(format!(
                    "native collection packed payload does not support type '{other}'"
                ));
            }
        }
        self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), len_slot));
        Ok(len_slot)
    }

    /// Copy a payload into a collection's data region at `data_offset_slot`.
    ///
    /// `stride_type` selects the data base's entry stride, exactly as for the
    /// readers: the element type for a LIST, `""` for a MAP. Deriving it from
    /// `payload.type_` is wrong for a map — a `Map OF Scalar TO T` has a
    /// fixed-width KEY, and the entry-free base would write that key inside the
    /// map's own lookup table (plan-57-D).
    pub(crate) fn emit_copy_payload_to_collection(
        &mut self,
        collection_slot: usize,
        len_slot: usize,
        payload: &PayloadSlot,
        data_offset_slot: usize,
        stride_type: &ParameterType,
    ) -> Result<(), String> {
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        self.emit(abi::load_u64(
            &scratch8,
            abi::stack_pointer(),
            collection_slot,
        ));
        self.emit(abi::load_u64(
            &scratch9,
            abi::stack_pointer(),
            data_offset_slot,
        ));
        self.emit(abi::add_immediate(
            &scratch10,
            &scratch8,
            COLLECTION_HEADER_SIZE,
        ));
        // The data base uses this element type's entry stride, which is zero for
        // a kind-2 list — the payload write and every reader must agree on where
        // the data region starts (plan-57-D).
        if list_entry_stride(stride_type) != 0 {
            self.emit(abi::load_u64(
                &scratch11,
                &scratch8,
                COLLECTION_OFFSET_CAPACITY,
            ));
            self.emit(abi::move_immediate(
                &scratch12,
                "Integer",
                &COLLECTION_ENTRY_SIZE.to_string(),
            ));
            self.emit(abi::multiply_registers(&scratch11, &scratch11, &scratch12));
            self.emit(abi::add_registers(&scratch10, &scratch10, &scratch11));
        }
        self.emit(abi::add_registers(&scratch10, &scratch10, &scratch9));

        match &payload.type_ {
            // bug-745: the unit value's payload is the constant byte 0, whatever
            // register the value arrived in.
            ParameterType::Nothing => {
                self.emit(abi::move_immediate(&scratch12, "Integer", "0"));
                self.emit(abi::store_u8(&scratch12, &scratch10, 0));
            }
            ParameterType::Boolean | ParameterType::Byte => {
                self.emit(abi::load_u64(
                    &scratch12,
                    abi::stack_pointer(),
                    payload.slot,
                ));
                self.emit(abi::store_u8(&scratch12, &scratch10, 0));
            }
            type_ if type_.is_named("Scalar") => {
                self.emit(abi::load_u64(
                    &scratch12,
                    abi::stack_pointer(),
                    payload.slot,
                ));
                self.emit(abi::store_u32(&scratch12, &scratch10, 0));
            }
            ParameterType::Integer
            | ParameterType::Float
            | ParameterType::Fixed
            | ParameterType::Money => {
                self.emit(abi::load_u64(
                    &scratch12,
                    abi::stack_pointer(),
                    payload.slot,
                ));
                self.emit(abi::store_u64(&scratch12, &scratch10, 0));
            }
            // bug-549: an enum ordinal is stored as the word it is.
            other if self.is_enum_type(other) => {
                self.emit(abi::load_u64(
                    &scratch12,
                    abi::stack_pointer(),
                    payload.slot,
                ));
                self.emit(abi::store_u64(&scratch12, &scratch10, 0));
            }
            // A function value stores its 8-byte closure pointer verbatim; the
            // closure object it points at is arena-lifetime and shared, never
            // copied on insert (reference semantics, bug-73).
            other if matches!(other, ParameterType::Func(..)) => {
                self.emit(abi::load_u64(
                    &scratch12,
                    abi::stack_pointer(),
                    payload.slot,
                ));
                self.emit(abi::store_u64(&scratch12, &scratch10, 0));
            }
            ParameterType::String => {
                let loop_label = self.label("collection_copy_string_loop");
                let done_label = self.label("collection_copy_string_done");
                self.emit(abi::load_u64(
                    &scratch12,
                    abi::stack_pointer(),
                    payload.slot,
                ));
                self.emit(abi::add_immediate(&scratch12, &scratch12, 8));
                self.emit(abi::load_u64(&scratch13, abi::stack_pointer(), len_slot));
                self.emit(abi::label(&loop_label));
                self.emit(abi::compare_immediate(&scratch13, "0"));
                self.emit(abi::branch_eq(&done_label));
                self.emit(abi::load_u8(&scratch14, &scratch12, 0));
                self.emit(abi::store_u8(&scratch14, &scratch10, 0));
                self.emit(abi::add_immediate(&scratch12, &scratch12, 1));
                self.emit(abi::add_immediate(&scratch10, &scratch10, 1));
                self.emit(abi::subtract_immediate(&scratch13, &scratch13, 1));
                self.emit(abi::branch(&loop_label));
                self.emit(abi::label(&done_label));
            }
            other if self.is_pointer_collection_payload_type(other) => {
                self.emit(abi::load_u64(
                    &scratch12,
                    abi::stack_pointer(),
                    payload.slot,
                ));
                self.emit(abi::store_u64(&scratch12, &scratch10, 0));
            }
            other
                if self.inline_collection_payload_size(other).is_some()
                    || typed_is_collection_type(other) =>
            {
                // Inline record/union slot bytes, or a flat nested collection
                // block — copy `len_slot` bytes verbatim (plan-02 §4.2–§4.4).
                self.emit(abi::load_u64(
                    &scratch12,
                    abi::stack_pointer(),
                    payload.slot,
                ));
                self.emit(abi::load_u64(&scratch13, abi::stack_pointer(), len_slot));
                self.emit_copy_bytes(&scratch10, &scratch12, &scratch13, "collection_copy_inline");
            }
            other => {
                return Err(format!(
                    "native collection packed payload does not support type '{other}'"
                ));
            }
        }

        self.emit(abi::load_u64(
            &scratch8,
            abi::stack_pointer(),
            data_offset_slot,
        ));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), len_slot));
        self.emit(abi::add_registers(&scratch8, &scratch8, &scratch9));
        self.emit(abi::store_u64(
            &scratch8,
            abi::stack_pointer(),
            data_offset_slot,
        ));
        Ok(())
    }

    /// Add the map hash-index bucket-region byte size (`2*capacity` u64 buckets =
    /// `capacity * MAP_BUCKET_SIZE * 2` bytes) held in `capacity_reg` onto the
    /// running allocation size in `size_reg`, using `scratch_reg` (plan-02
    /// Phase 6). A no-op for lists (`is_map == false`) so list allocations are
    /// unchanged. The bucket region sits past the data region, so the
    /// capacity-based data base (`emit_collection_data_pointer`) is unaffected.
    pub(crate) fn emit_reserve_map_buckets(
        &mut self,
        is_map: bool,
        capacity_reg: impl Into<abi::Reg>,
        size_reg: impl Into<abi::Reg>,
        scratch_reg: impl Into<abi::Reg>,
    ) {
        if !is_map {
            return;
        }
        let size_reg = size_reg.into();
        let scratch_reg = scratch_reg.into();
        // 2 * capacity buckets * 8 bytes = capacity << 4.
        self.emit(abi::shift_left_immediate(scratch_reg, capacity_reg, 4));
        self.emit(abi::add_registers(size_reg, size_reg, scratch_reg));
    }
}
