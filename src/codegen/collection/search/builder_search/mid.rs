//! plan-176-Q: `mid` on strings and lists — the string window registers and lowering. `find`
//! lives in `collection/search/builder_search/mod.rs`.

use super::*;

/// plan-146-C: the registers [`CodeBuilder::lower_mid`] allocates for the `String`
/// branch, handed to its window half so both paths keep one allocation order.
pub(crate) struct MidStringRegs<'a> {
    pub(crate) value_ptr: &'a VirtualRegister,
    pub(crate) string_len: &'a VirtualRegister,
    pub(crate) cursor: &'a VirtualRegister,
    pub(crate) remaining: &'a VirtualRegister,
    pub(crate) scalar_index: &'a VirtualRegister,
    pub(crate) start_index: &'a VirtualRegister,
    pub(crate) count_value: &'a VirtualRegister,
    pub(crate) end_index: &'a VirtualRegister,
    pub(crate) byte: &'a VirtualRegister,
    pub(crate) mask: &'a VirtualRegister,
    pub(crate) start_ptr: &'a VirtualRegister,
    pub(crate) end_ptr: &'a VirtualRegister,
    pub(crate) byte_len: &'a VirtualRegister,
}

/// plan-146-C: what `strings::mid`'s window half leaves for its caller.
pub(crate) struct MidWindow {
    /// The copying path's result block slot.
    pub(crate) result_slot: usize,
    pub(crate) start_ptr_slot: usize,
    pub(crate) byte_len_slot: usize,
    /// The copying path's post-allocation label.
    pub(crate) alloc_ok: String,
    /// Where every range check branches; the caller emits the raise at it.
    pub(crate) invalid_range: String,
}

impl CodeBuilder<'_> {
    /// plan-146-C: `strings::mid`'s window half — the range checks and the scalar
    /// walk that locates the selected span, left in `start_ptr_slot`/`byte_len_slot`.
    /// Everything that can raise branches to `invalid_range`, which the caller
    /// raises at after building its result (that is where the copying lowering has
    /// always emitted it, and the arm keeps the same shape). `result_slot` and
    /// `alloc_ok` belong to the copying path; they are created here so both paths
    /// keep one slot and label order.
    pub(crate) fn lower_mid_string_window(
        &mut self,
        value_slot: usize,
        start_slot: usize,
        count_slot: usize,
        r: &MidStringRegs<'_>,
    ) -> Result<MidWindow, String> {
        let result_slot = self.allocate_stack_object("mid_result", 8);
        let value_ptr = r.value_ptr;
        let string_len = r.string_len;
        let cursor = r.cursor;
        let remaining = r.remaining;
        let scalar_index = r.scalar_index;
        let start_index = r.start_index;
        let count_value = r.count_value;
        let end_index = r.end_index;
        let byte = r.byte;
        let mask = r.mask;
        let start_ptr = r.start_ptr;
        let end_ptr = r.end_ptr;
        let byte_len = r.byte_len;
        let start_ptr_slot = self.allocate_stack_object("mid_start_ptr", 8);
        let byte_len_slot = self.allocate_stack_object("mid_byte_len", 8);
        let locate_start = self.label("mid_locate_start");
        let locate_start_continue = self.label("mid_locate_start_continue");
        let locate_start_advanced = self.label("mid_locate_start_advanced");
        let start_ready = self.label("mid_start_ready");
        let locate_end = self.label("mid_locate_end");
        let locate_end_continue = self.label("mid_locate_end_continue");
        let locate_end_advanced = self.label("mid_locate_end_advanced");
        let end_ready = self.label("mid_end_ready");
        let alloc_ok = self.label("mid_alloc_ok");
        let invalid_range = self.label("mid_invalid_range");

        self.emit(abi::load_u64(value_ptr, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(start_index, abi::stack_pointer(), start_slot));
        self.emit(abi::load_u64(count_value, abi::stack_pointer(), count_slot));
        self.emit(abi::compare_immediate(start_index, "0"));
        self.emit(abi::branch_lt(&invalid_range));
        self.emit(abi::compare_immediate(count_value, "0"));
        self.emit(abi::branch_lt(&invalid_range));
        self.emit(abi::add_registers(end_index, start_index, count_value));
        self.emit(abi::compare_registers(end_index, start_index));
        self.emit(abi::branch_lo(&invalid_range));
        self.emit(abi::load_u64(string_len, value_ptr, 0));
        self.emit(abi::add_immediate(cursor, value_ptr, 8));
        self.emit(abi::move_register(start_ptr, cursor));
        self.emit(abi::move_register(end_ptr, cursor));
        self.emit(abi::move_register(remaining, string_len));
        self.emit(abi::move_immediate(scalar_index, "Integer", "0"));
        self.emit(abi::move_immediate(
            mask,
            "Integer",
            crate::codegen::string::unicode_props::UTF8_CONTINUATION_MASK,
        ));

        // plan-77 U4: skip whole 8-byte ASCII runs before the per-scalar walk.
        let ff_diff = self.temporary_vreg();
        let ff_word = self.temporary_vreg();
        let ff_mask = self.temporary_vreg();
        self.emit_ascii_scalar_fastforward(
            cursor,
            remaining,
            scalar_index,
            start_index,
            &ff_diff,
            &ff_word,
            &ff_mask,
        );
        self.emit(abi::label(&locate_start));
        self.emit(abi::compare_registers(scalar_index, start_index));
        self.emit(abi::branch_eq(&start_ready));
        self.emit(abi::compare_immediate(remaining, "0"));
        self.emit(abi::branch_eq(&invalid_range));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::subtract_immediate(remaining, remaining, 1));
        self.emit_scalar_skip_continuations(
            cursor,
            remaining,
            byte,
            mask,
            &locate_start_continue,
            &locate_start_advanced,
        );
        self.emit(abi::label(&locate_start_advanced));
        self.emit(abi::add_immediate(scalar_index, scalar_index, 1));
        self.emit(abi::branch(&locate_start));

        self.emit(abi::label(&start_ready));
        self.emit(abi::move_register(start_ptr, cursor));
        self.emit(abi::move_register(end_ptr, cursor));
        // plan-77 U4: skip whole 8-byte ASCII runs before the per-scalar walk.
        let ff_diff_end = self.temporary_vreg();
        let ff_word_end = self.temporary_vreg();
        let ff_mask_end = self.temporary_vreg();
        self.emit_ascii_scalar_fastforward(
            cursor,
            remaining,
            scalar_index,
            end_index,
            &ff_diff_end,
            &ff_word_end,
            &ff_mask_end,
        );
        self.emit(abi::label(&locate_end));
        self.emit(abi::compare_registers(scalar_index, end_index));
        self.emit(abi::branch_eq(&end_ready));
        self.emit(abi::compare_immediate(remaining, "0"));
        self.emit(abi::branch_eq(&invalid_range));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::subtract_immediate(remaining, remaining, 1));
        self.emit_scalar_skip_continuations(
            cursor,
            remaining,
            byte,
            mask,
            &locate_end_continue,
            &locate_end_advanced,
        );
        self.emit(abi::label(&locate_end_advanced));
        self.emit(abi::add_immediate(scalar_index, scalar_index, 1));
        self.emit(abi::branch(&locate_end));

        self.emit(abi::label(&end_ready));
        self.emit(abi::move_register(end_ptr, cursor));
        self.emit(abi::subtract_registers(byte_len, end_ptr, start_ptr));
        self.emit(abi::store_u64(
            start_ptr,
            abi::stack_pointer(),
            start_ptr_slot,
        ));
        self.emit(abi::store_u64(
            byte_len,
            abi::stack_pointer(),
            byte_len_slot,
        ));
        Ok(MidWindow {
            result_slot,
            start_ptr_slot,
            byte_len_slot,
            alloc_ok,
            invalid_range,
        })
    }

    pub(crate) fn lower_mid(&mut self, args: &[NirValue]) -> Result<ValueResult, String> {
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
        let scratch20 = self.temporary_vreg();
        let scratch21 = self.temporary_vreg();
        let scratch22 = self.temporary_vreg();
        let scratch23 = self.temporary_vreg();
        let scratch24 = self.temporary_vreg();
        let scratch25 = self.temporary_vreg();
        let value = self.lower_value(&args[0])?;
        if let Some(element_type) = typed_list_element_type(&value.type_).cloned() {
            let value_slot = self.allocate_stack_object("mid_list_value", 8);
            self.emit(abi::store_u64(
                &value.location,
                abi::stack_pointer(),
                value_slot,
            ));
            let start = self.lower_value(&args[1])?;
            if start.type_ != ParameterType::Integer {
                return Err(format!(
                    "native list mid start must be Integer, got {}",
                    start.type_
                ));
            }
            let start_slot = self.allocate_stack_object("mid_list_start", 8);
            self.emit(abi::store_u64(
                &start.location,
                abi::stack_pointer(),
                start_slot,
            ));
            let count = self.lower_value(&args[2])?;
            if count.type_ != ParameterType::Integer {
                return Err(format!(
                    "native list mid count must be Integer, got {}",
                    count.type_
                ));
            }
            let count_slot = self.allocate_stack_object("mid_list_count", 8);
            self.emit(abi::store_u64(
                &count.location,
                abi::stack_pointer(),
                count_slot,
            ));
            let result = self.lower_list_mid(
                value_slot,
                start_slot,
                count_slot,
                &value.type_,
                &element_type,
            )?;
            // plan-134-E: the sub-list owns its elements' graphs.
            return self.own_collection_payload_edges(result);
        }
        if value.type_ != ParameterType::String {
            return Err(format!(
                "native string mid value must be String, got {}",
                value.type_
            ));
        }
        let value_slot = self.allocate_stack_object("mid_value", 8);
        self.emit(abi::store_u64(
            &value.location,
            abi::stack_pointer(),
            value_slot,
        ));

        let start = self.lower_value(&args[1])?;
        if start.type_ != ParameterType::Integer {
            return Err(format!(
                "native string mid start must be Integer, got {}",
                start.type_
            ));
        }
        let start_slot = self.allocate_stack_object("mid_start", 8);
        self.emit(abi::store_u64(
            &start.location,
            abi::stack_pointer(),
            start_slot,
        ));

        let count = self.lower_value(&args[2])?;
        if count.type_ != ParameterType::Integer {
            return Err(format!(
                "native string mid count must be Integer, got {}",
                count.type_
            ));
        }
        let count_slot = self.allocate_stack_object("mid_count", 8);
        self.emit(abi::store_u64(
            &count.location,
            abi::stack_pointer(),
            count_slot,
        ));

        let window = self.lower_mid_string_window(
            value_slot,
            start_slot,
            count_slot,
            &MidStringRegs {
                value_ptr: &scratch8,
                string_len: &scratch9,
                cursor: &scratch10,
                remaining: &scratch11,
                scalar_index: &scratch12,
                start_index: &scratch13,
                count_value: &scratch14,
                end_index: &scratch15,
                byte: &scratch16,
                mask: &scratch17,
                start_ptr: &scratch20,
                end_ptr: &scratch21,
                byte_len: &scratch25,
            },
        )?;
        let MidWindow {
            result_slot,
            start_ptr_slot,
            byte_len_slot,
            alloc_ok,
            invalid_range,
        } = window;
        let byte = &scratch16;
        let start_ptr = &scratch20;
        let copy_src = &scratch22;
        let copy_dst = &scratch23;
        let copy_remaining = &scratch24;
        let byte_len = &scratch25;

        // plan-71-C Family-1a: alloc size is arg 0 of the arena-alloc call → `%arg0`.
        self.emit(abi::add_immediate(abi::c_arg(0), byte_len, 9));
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&alloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            result_slot,
        ));
        self.emit(abi::load_u64(byte_len, abi::stack_pointer(), byte_len_slot));
        self.emit(abi::store_u64(byte_len, abi::mfb_return(1), 0));
        self.emit(abi::load_u64(
            start_ptr,
            abi::stack_pointer(),
            start_ptr_slot,
        ));
        self.emit(abi::move_register(copy_src, start_ptr));
        self.emit(abi::add_immediate(copy_dst, abi::mfb_return(1), 8));
        self.emit(abi::move_register(copy_remaining, byte_len));
        // plan-86 F2: 8-byte word-copy (+ byte tail); advances copy_dst/copy_src.
        self.emit_block_copy_advance(copy_dst, copy_src, copy_remaining, byte, "mid_copy");
        self.emit(abi::move_immediate(byte, "Integer", "0"));
        self.emit(abi::store_u8(byte, copy_dst, 0));
        let result = self.allocate_register();
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        let done = self.label("mid_done");
        self.emit(abi::branch(&done));

        self.emit(abi::label(&invalid_range));
        self.raise_error("strings.mid", "ErrIndexOutOfRange")?;
        self.emit(abi::label(&done));

        // bug-536 shape B: `result_slot` holds this lowering's own
        // `emit_arena_alloc_call` block, with the selected span copied into it.
        self.mark_fresh_string(Operand::from(result));
        Ok(ValueResult {
            origin: None,
            type_: ParameterType::String,
            location: Operand::from(result),
            text: "mid(String, Integer, Integer)".to_string(),
        })
    }

    pub(crate) fn lower_list_mid(
        &mut self,
        value_slot: usize,
        start_slot: usize,
        count_slot: usize,
        list_type: &ParameterType,
        element_type: &ParameterType,
    ) -> Result<ValueResult, String> {
        let layout = CollectionTypeLayout::from_type(list_type)
            .ok_or_else(|| format!("native code collection type '{list_type}' is not supported"))?;
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
        let scratch20 = self.temporary_vreg();
        let scratch21 = self.temporary_vreg();
        let scratch22 = self.temporary_vreg();

        // Zero-stride for kind 2 throughout: `mid` neither reads nor writes a
        // lookup table for a fixed-width list (plan-57-D).
        let mid_payload = kind2_payload_size(element_type);
        let data_len_slot = self.allocate_stack_object("mid_list_data_len", 8);
        let disordered_slot = self.allocate_stack_object("mid_list_disordered", 8);
        let result_slot = self.allocate_stack_object("mid_list_result", 8);
        let mid_unordered = self.label("mid_list_unordered");
        let valid_start = self.label("mid_list_valid_start");
        let valid_count = self.label("mid_list_valid_count");
        let range_ok = self.label("mid_list_range_ok");
        let length_loop = self.label("mid_list_length_loop");
        let length_done = self.label("mid_list_length_done");
        let alloc_ok = self.label("mid_list_alloc_ok");
        let copy_done = self.label("mid_list_copy_done");
        let invalid_range = self.label("mid_list_invalid_range");
        let done = self.label("mid_list_done");

        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), start_slot));
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), count_slot));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch8,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::compare_immediate(&scratch9, "0"));
        self.emit(abi::branch_ge(&valid_start));
        self.emit(abi::branch(&invalid_range));
        self.emit(abi::label(&valid_start));
        self.emit(abi::compare_immediate(&scratch10, "0"));
        self.emit(abi::branch_ge(&valid_count));
        self.emit(abi::branch(&invalid_range));
        self.emit(abi::label(&valid_count));
        self.emit(abi::compare_registers(&scratch9, &scratch11));
        self.emit(abi::branch_gt(&invalid_range));
        self.emit(abi::add_registers(&scratch12, &scratch9, &scratch10));
        self.emit(abi::compare_registers(&scratch12, &scratch9));
        self.emit(abi::branch_lt(&invalid_range));
        self.emit(abi::compare_registers(&scratch12, &scratch11));
        self.emit(abi::branch_le(&range_ok));
        self.emit(abi::branch(&invalid_range));

        self.emit(abi::label(&range_ok));
        self.emit(abi::move_immediate(&scratch13, "Integer", "0"));
        self.emit(abi::store_u64(
            &scratch13,
            abi::stack_pointer(),
            data_len_slot,
        ));
        self.emit(abi::move_immediate(
            &scratch14,
            "Integer",
            &mid_payload.map_or(COLLECTION_ENTRY_SIZE, |_| 0).to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch15, &scratch9, &scratch14));
        self.emit(abi::add_immediate(
            &scratch16,
            &scratch8,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(&scratch16, &scratch16, &scratch15)); // src entry[start]

        // kind 2 needs neither the length loop nor the probe: `dataLength` is
        // `count * payloadSize` and the slice is contiguous and in order by
        // construction (plan-57-D). `scratch16` above is meaningless for it, and
        // is not read on this path.
        if let Some(payload) = mid_payload {
            self.emit(abi::move_immediate(&scratch14, "Integer", "0"));
            self.emit(abi::store_u64(
                &scratch14,
                abi::stack_pointer(),
                disordered_slot,
            ));
            self.emit(abi::move_immediate(
                &scratch14,
                "Integer",
                &payload.to_string(),
            ));
            self.emit(abi::multiply_registers(&scratch13, &scratch10, &scratch14));
            self.emit(abi::store_u64(
                &scratch13,
                abi::stack_pointer(),
                data_len_slot,
            ));
            self.emit(abi::branch(&length_done));
        }

        // Order/tightness probe folded into the length loop: `expected` (scratch11)
        // tracks where the next payload should begin if the slice is a contiguous
        // ordered span; any entry whose valueOffset differs sets `disordered_slot`.
        // A sorted `fs::listDirectory` result permutes entry records without moving
        // the data, so its slice is out of order and takes the per-entry fallback.
        let mid_ordered_ok = self.label("mid_list_ordered_ok");
        self.emit(abi::move_immediate(&scratch14, "Integer", "0"));
        self.emit(abi::store_u64(
            &scratch14,
            abi::stack_pointer(),
            disordered_slot,
        ));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch16,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        )); // expected = entry[start].valueOffset

        self.emit(abi::label(&length_loop));
        self.emit(abi::compare_registers(&scratch13, &scratch10));
        self.emit(abi::branch_ge(&length_done));
        self.emit(abi::load_u64(
            &scratch17,
            &scratch16,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::load_u64(
            &scratch9,
            &scratch16,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        )); // vo
        self.emit(abi::compare_registers(&scratch9, &scratch11));
        self.emit(abi::branch_eq(&mid_ordered_ok));
        self.emit(abi::move_immediate(&scratch14, "Integer", "1"));
        self.emit(abi::store_u64(
            &scratch14,
            abi::stack_pointer(),
            disordered_slot,
        ));
        self.emit(abi::label(&mid_ordered_ok));
        self.emit(abi::add_registers(&scratch11, &scratch9, &scratch17)); // expected = vo + vl
        self.emit(abi::load_u64(
            &scratch20,
            abi::stack_pointer(),
            data_len_slot,
        ));
        self.emit(abi::add_registers(&scratch20, &scratch20, &scratch17));
        self.emit(abi::store_u64(
            &scratch20,
            abi::stack_pointer(),
            data_len_slot,
        ));
        self.emit(abi::add_immediate(
            &scratch16,
            &scratch16,
            COLLECTION_ENTRY_SIZE,
        ));
        self.emit(abi::add_immediate(&scratch13, &scratch13, 1));
        self.emit(abi::branch(&length_loop));

        self.emit(abi::label(&length_done));
        self.emit(abi::move_immediate(
            &scratch14,
            "Integer",
            &mid_payload.map_or(COLLECTION_ENTRY_SIZE, |_| 0).to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch15, &scratch10, &scratch14));
        self.emit(abi::load_u64(
            &scratch16,
            abi::stack_pointer(),
            data_len_slot,
        ));
        self.emit(abi::add_immediate(
            abi::return_register(),
            &scratch15,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(
            abi::return_register(),
            abi::return_register(),
            &scratch16,
        ));
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&alloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(
            abi::mfb_return(1),
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
            abi::mfb_return(1),
            COLLECTION_OFFSET_KIND,
        ));
        self.emit(abi::move_immediate(
            &scratch13,
            "Byte",
            &layout.key_type_code.to_string(),
        ));
        self.emit(abi::store_u8(
            &scratch13,
            abi::mfb_return(1),
            COLLECTION_OFFSET_KEY_TYPE,
        ));
        self.emit(abi::move_immediate(
            &scratch13,
            "Byte",
            &layout.value_type_code.to_string(),
        ));
        self.emit(abi::store_u8(
            &scratch13,
            abi::mfb_return(1),
            COLLECTION_OFFSET_VALUE_TYPE,
        ));
        self.emit(abi::move_immediate(&scratch13, "Byte", "1"));
        self.emit(abi::store_u8(
            &scratch13,
            abi::mfb_return(1),
            COLLECTION_OFFSET_FLAGS_VERSION,
        ));
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), count_slot));
        self.emit(abi::store_u64(
            &scratch10,
            abi::mfb_return(1),
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::store_u64(
            &scratch10,
            abi::mfb_return(1),
            COLLECTION_OFFSET_CAPACITY,
        ));
        self.emit(abi::load_u64(
            &scratch16,
            abi::stack_pointer(),
            data_len_slot,
        ));
        self.emit(abi::store_u64(
            &scratch16,
            abi::mfb_return(1),
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::store_u64(
            &scratch16,
            abi::mfb_return(1),
            COLLECTION_OFFSET_DATA_CAPACITY,
        ));

        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), start_slot));
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), count_slot));
        self.emit(abi::load_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            result_slot,
        ));
        self.emit(abi::move_immediate(
            &scratch14,
            "Integer",
            &mid_payload.map_or(COLLECTION_ENTRY_SIZE, |_| 0).to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch15, &scratch9, &scratch14));
        self.emit(abi::add_immediate(
            &scratch16,
            &scratch8,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(&scratch16, &scratch16, &scratch15));
        self.emit(abi::add_immediate(
            &scratch17,
            abi::mfb_return(1),
            COLLECTION_HEADER_SIZE,
        ));
        self.emit_collection_data_pointer_for(&scratch20, &scratch8, &element_type);
        self.emit(abi::multiply_registers(&scratch21, &scratch10, &scratch14));
        self.emit(abi::add_registers(&scratch21, &scratch17, &scratch21));
        // Fast path — an in-order, gap-free slice (the probe found no disorder) is
        // a single contiguous data span: copy `dataLen` bytes from
        // `src.data + srcBaseOffset` (the first slice entry's valueOffset) with one
        // block copy, and copy the entry span verbatim with each valueOffset shifted
        // down by `srcBaseOffset` so the slice repacks from 0 (plan-25-B). A slice
        // that failed the probe (a permuted / gappy source) takes the per-entry
        // re-pack fallback, which reads each entry's own valueOffset.
        self.emit(abi::load_u64(
            &scratch13,
            abi::stack_pointer(),
            disordered_slot,
        ));
        self.emit(abi::compare_immediate(&scratch13, "0"));
        self.emit(abi::branch_ne(&mid_unordered));
        if let Some(payload) = mid_payload {
            // kind 2: srcBaseOffset is `start * payloadSize`. Reload `start` from
            // its slot — `_mfb_arena_alloc` above destroys every caller-saved
            // register, so nothing held across it can be trusted.
            self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), start_slot));
            self.emit(abi::move_immediate(
                &scratch13,
                "Integer",
                &payload.to_string(),
            ));
            self.emit(abi::multiply_registers(&scratch13, &scratch9, &scratch13));
        } else {
            self.emit(abi::load_u64(
                &scratch13,
                &scratch16,
                COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
            )); // srcBaseOffset
        }
        self.emit(abi::add_registers(&scratch20, &scratch20, &scratch13)); // src.data + srcBaseOffset
        self.emit(abi::load_u64(
            &scratch14,
            abi::stack_pointer(),
            data_len_slot,
        ));
        self.emit_block_copy_advance(
            &scratch21,
            &scratch20,
            &scratch14,
            &scratch22,
            "mid_list_data",
        );
        if mid_payload.is_none() {
            self.emit_bulk_copy_entries_shift(
                &scratch16,
                &scratch17,
                &scratch10,
                Some((&scratch13, true)),
                "mid_list_entries",
            );
        }
        self.emit(abi::branch(&copy_done));

        // Fallback: per-entry re-pack for an out-of-order / gappy source (scratch20
        // still holds the un-advanced src data base).
        self.emit(abi::label(&mid_unordered));
        self.emit(abi::move_immediate(&scratch13, "Integer", "0"));
        self.emit_copy_collection_entries(
            &scratch16,
            &scratch20,
            &scratch17,
            &scratch21,
            &scratch13,
            &scratch10,
            "mid_list_copy",
        )?;
        self.emit(abi::branch(&copy_done));

        self.emit(abi::label(&invalid_range));
        self.raise_error("collections.mid", "ErrIndexOutOfRange")?;
        self.emit(abi::label(&copy_done));
        self.emit(abi::branch(&done));
        self.emit(abi::label(&done));

        let result = self.allocate_register();
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            origin: None,
            type_: list_type.clone(),
            location: Operand::from(result),
            text: format!("mid({list_type}, Integer, Integer) over {element_type}"),
        })
    }
}
