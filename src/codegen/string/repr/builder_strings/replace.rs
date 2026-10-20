//! plan-176-P: `replace` on strings and lists. String comparison and ordering live in
//! `string/repr/builder_strings/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    pub(crate) fn lower_replace(&mut self, args: &[NirValue]) -> Result<ValueResult, String> {
        let (result, _, _) = self.lower_replace_out(args, StringOut::Block)?;
        Ok(result)
    }

    /// plan-146-E: [`lower_replace`] with the destination chosen by the caller.
    /// `StringOut::Block` is the copying lowering, byte for byte;
    /// `StringOut::Scratch` writes the result into the function's self-update
    /// scratch and answers `(pointer, length slot, "no match" flag slot)` — the
    /// no-match case leaves the binding alone, so the arm skips the copy back.
    pub(crate) fn lower_replace_out(
        &mut self,
        args: &[NirValue],
        out: StringOut,
    ) -> Result<(ValueResult, usize, usize), String> {
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
        let scratch26 = self.temporary_vreg();
        let value = self.lower_value(&args[0])?;
        if let Some(element_type) = typed_list_element_type(&value.type_) {
            let value_slot = self.allocate_stack_object("replace_list_value", 8);
            self.emit(abi::store_u64(
                &value.location,
                abi::stack_pointer(),
                value_slot,
            ));
            let old = self.lower_value(&args[1])?;
            if old.type_ != *element_type {
                return Err(format!(
                    "native list replace old must be {}, got {}",
                    element_type, old.type_
                ));
            }
            let old_slot = self.allocate_stack_object("replace_list_old", 8);
            self.emit(abi::store_u64(
                &old.location,
                abi::stack_pointer(),
                old_slot,
            ));
            let new = self.lower_value(&args[2])?;
            if new.type_ != *element_type {
                return Err(format!(
                    "native list replace new must be {}, got {}",
                    element_type, new.type_
                ));
            }
            let new_slot = self.allocate_stack_object("replace_list_new", 8);
            self.emit(abi::store_u64(
                &new.location,
                abi::stack_pointer(),
                new_slot,
            ));
            let result = self.lower_list_replace(
                value_slot,
                old_slot,
                new_slot,
                &value.type_,
                element_type,
            )?;
            // plan-134-E: the rebuilt list owns its elements' graphs. A `List`
            // never reaches the `String` in-place arm (its own arm serves it), so
            // the extra slots are irrelevant here.
            return Ok((self.own_collection_payload_edges(result)?, 0, 0));
        }
        if value.type_ != ParameterType::String {
            return Err(format!(
                "native string replace value must be String, got {}",
                value.type_
            ));
        }
        let value_slot = self.allocate_stack_object("replace_value", 8);
        self.emit(abi::store_u64(
            &value.location,
            abi::stack_pointer(),
            value_slot,
        ));

        let old = self.lower_value(&args[1])?;
        if old.type_ != ParameterType::String {
            return Err(format!(
                "native string replace old must be String, got {}",
                old.type_
            ));
        }
        let old_slot = self.allocate_stack_object("replace_old", 8);
        self.emit(abi::store_u64(
            &old.location,
            abi::stack_pointer(),
            old_slot,
        ));

        let new = self.lower_value(&args[2])?;
        if new.type_ != ParameterType::String {
            return Err(format!(
                "native string replace new must be String, got {}",
                new.type_
            ));
        }
        let new_slot = self.allocate_stack_object("replace_new", 8);
        self.emit(abi::store_u64(
            &new.location,
            abi::stack_pointer(),
            new_slot,
        ));

        let result_slot = self.allocate_stack_object("replace_result", 8);
        let output_len_slot = self.allocate_stack_object("replace_output_len", 8);
        // plan-146-E: the in-place arm's two extra slots and labels. They are
        // allocated only in that mode, so the copying path's frame is unchanged.
        let (unchanged_slot, scratch_ready, scratch_written) = if matches!(out, StringOut::Scratch)
        {
            (
                self.allocate_stack_object("replace_unchanged", 8),
                self.label("replace_scratch_ready"),
                self.label("replace_scratch_written"),
            )
        } else {
            (0, String::new(), String::new())
        };

        let value_ptr = &scratch8;
        let value_len = &scratch9;
        let old_ptr = &scratch10;
        let old_len = &scratch11;
        let new_ptr = &scratch12;
        let new_len = &scratch13;
        let index = &scratch14;
        let output_len = &scratch15;
        let last_start = &scratch16;
        let match_index = &scratch17;
        let candidate = &scratch20;
        let old_cursor = &scratch21;
        let value_byte = &scratch22;
        let old_byte = &scratch23;
        let dest = &scratch24;
        let new_cursor = &scratch25;
        let new_index = &scratch26;

        let copy_original = self.label("replace_copy_original");
        // bug-533: an empty `old` occurs at every position, and rewriting at
        // every position destroys the input. This is the STRING path only — the
        // `List` overload returned above, and an empty *element* there is an
        // ordinary value, not a degenerate needle.
        let empty_old = self.label("replace_empty_old");
        let first_loop = self.label("replace_first_loop");
        let first_compare = self.label("replace_first_compare");
        let first_match = self.label("replace_first_match");
        let first_next = self.label("replace_first_next");
        let first_done = self.label("replace_first_done");
        let alloc_ok = self.label("replace_alloc_ok");
        let overflow = self.label("replace_overflow");
        let second_loop = self.label("replace_second_loop");
        let second_compare = self.label("replace_second_compare");
        let second_match = self.label("replace_second_match");
        let second_copy_new_loop = self.label("replace_second_copy_new_loop");
        let second_copy_new_done = self.label("replace_second_copy_new_done");
        let second_copy_one = self.label("replace_second_copy_one");
        let second_done = self.label("replace_second_done");
        let done = self.label("replace_done");
        let result = self.allocate_register();

        self.emit(abi::load_u64(value_ptr, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(old_ptr, abi::stack_pointer(), old_slot));
        self.emit(abi::load_u64(new_ptr, abi::stack_pointer(), new_slot));
        self.emit(abi::load_u64(value_len, value_ptr, 0));
        self.emit(abi::load_u64(old_len, old_ptr, 0));
        self.emit(abi::load_u64(new_len, new_ptr, 0));
        self.emit(abi::compare_immediate(old_len, "0"));
        self.emit(abi::branch_eq(&empty_old));
        // An `old` longer than `value` cannot match anywhere; that is an ordinary
        // no-match, and it keeps the copy-through path the empty needle no longer
        // takes.
        self.emit(abi::compare_registers(old_len, value_len));
        self.emit(abi::branch_hi(&copy_original));
        self.emit(abi::add_immediate(value_ptr, value_ptr, 8));
        self.emit(abi::add_immediate(old_ptr, old_ptr, 8));
        self.emit(abi::add_immediate(new_ptr, new_ptr, 8));
        self.emit(abi::move_immediate(index, "Integer", "0"));
        self.emit(abi::move_register(output_len, value_len));
        self.emit(abi::subtract_registers(last_start, value_len, old_len));

        self.emit(abi::label(&first_loop));
        self.emit(abi::compare_registers(index, last_start));
        self.emit(abi::branch_hi(&first_done));
        self.emit(abi::move_immediate(match_index, "Integer", "0"));
        self.emit(abi::add_registers(candidate, value_ptr, index));
        self.emit(abi::move_register(old_cursor, old_ptr));
        self.emit(abi::label(&first_compare));
        self.emit(abi::compare_registers(match_index, old_len));
        self.emit(abi::branch_eq(&first_match));
        self.emit(abi::load_u8(value_byte, candidate, 0));
        self.emit(abi::load_u8(old_byte, old_cursor, 0));
        self.emit(abi::compare_registers(value_byte, old_byte));
        self.emit(abi::branch_ne(&first_next));
        self.emit(abi::add_immediate(candidate, candidate, 1));
        self.emit(abi::add_immediate(old_cursor, old_cursor, 1));
        self.emit(abi::add_immediate(match_index, match_index, 1));
        self.emit(abi::branch(&first_compare));

        self.emit(abi::label(&first_match));
        // output_len += new_len - old_len. old_len <= value_len at a match, so the
        // subtract never underflows; the add is the growth term — trap a 64-bit
        // wrap so the second pass cannot write past the (undersized) allocation.
        self.emit(abi::subtract_registers(output_len, output_len, old_len));
        self.emit_checked_size_add(output_len, output_len, new_len, &overflow);
        self.emit(abi::add_registers(index, index, old_len));
        self.emit(abi::branch(&first_loop));

        self.emit(abi::label(&first_next));
        self.emit(abi::add_immediate(index, index, 1));
        self.emit(abi::branch(&first_loop));

        self.emit(abi::label(&first_done));
        self.emit(abi::store_u64(
            output_len,
            abi::stack_pointer(),
            output_len_slot,
        ));
        // allocate output_len + 9 (block header), trapping the header add's wrap.
        if matches!(out, StringOut::Scratch) {
            // plan-146-E: the rewritten bytes go to the reused scratch instead.
            let scratch = self.emit_string_out_scratch(output_len_slot)?;
            self.emit(abi::load_u64(&result, abi::stack_pointer(), scratch));
            self.emit(abi::store_u64(&result, abi::stack_pointer(), result_slot));
            self.emit(abi::branch(&scratch_ready));
        }
        self.emit_checked_size_add_immediate(abi::return_register(), output_len, 9, &overflow);
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&alloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        // A size wrap reports the same 77010001 an impossible allocation would
        // (x0 does not hold an error code before the call, so the register-based
        // return above cannot be shared). The checked-size helper deposits the
        // partially-computed size into the return register before branching here,
        // so `emit_allocation_error_return` would surface that size as the error
        // code (bug-60 detection, bug-352 code fix).
        self.emit(abi::label(&overflow));
        self.raise_error_bare("ErrOutOfMemory")?;

        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            result_slot,
        ));
        self.emit(abi::load_u64(
            output_len,
            abi::stack_pointer(),
            output_len_slot,
        ));
        self.emit(abi::store_u64(output_len, abi::mfb_return(1), 0));
        self.emit(abi::add_immediate(dest, abi::mfb_return(1), 8));
        if matches!(out, StringOut::Scratch) {
            self.emit(abi::branch(&scratch_written));
            self.emit(abi::label(&scratch_ready));
            // The scratch holds only the bytes: the cursor starts at its first.
            self.emit(abi::load_u64(dest, abi::stack_pointer(), result_slot));
            self.emit(abi::label(&scratch_written));
        }
        self.emit(abi::load_u64(value_ptr, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(old_ptr, abi::stack_pointer(), old_slot));
        self.emit(abi::load_u64(new_ptr, abi::stack_pointer(), new_slot));
        self.emit(abi::load_u64(value_len, value_ptr, 0));
        self.emit(abi::load_u64(old_len, old_ptr, 0));
        self.emit(abi::load_u64(new_len, new_ptr, 0));
        self.emit(abi::add_immediate(value_ptr, value_ptr, 8));
        self.emit(abi::add_immediate(old_ptr, old_ptr, 8));
        self.emit(abi::add_immediate(new_ptr, new_ptr, 8));
        self.emit(abi::subtract_registers(last_start, value_len, old_len));
        self.emit(abi::move_immediate(index, "Integer", "0"));

        self.emit(abi::label(&second_loop));
        self.emit(abi::compare_registers(index, value_len));
        self.emit(abi::branch_ge(&second_done));
        self.emit(abi::compare_registers(index, last_start));
        self.emit(abi::branch_hi(&second_copy_one));
        self.emit(abi::move_immediate(match_index, "Integer", "0"));
        self.emit(abi::add_registers(candidate, value_ptr, index));
        self.emit(abi::move_register(old_cursor, old_ptr));
        self.emit(abi::label(&second_compare));
        self.emit(abi::compare_registers(match_index, old_len));
        self.emit(abi::branch_eq(&second_match));
        self.emit(abi::load_u8(value_byte, candidate, 0));
        self.emit(abi::load_u8(old_byte, old_cursor, 0));
        self.emit(abi::compare_registers(value_byte, old_byte));
        self.emit(abi::branch_ne(&second_copy_one));
        self.emit(abi::add_immediate(candidate, candidate, 1));
        self.emit(abi::add_immediate(old_cursor, old_cursor, 1));
        self.emit(abi::add_immediate(match_index, match_index, 1));
        self.emit(abi::branch(&second_compare));

        self.emit(abi::label(&second_match));
        self.emit(abi::move_immediate(new_index, "Integer", "0"));
        self.emit(abi::move_register(new_cursor, new_ptr));
        self.emit(abi::label(&second_copy_new_loop));
        self.emit(abi::compare_registers(new_index, new_len));
        self.emit(abi::branch_eq(&second_copy_new_done));
        self.emit(abi::load_u8(value_byte, new_cursor, 0));
        self.emit(abi::store_u8(value_byte, dest, 0));
        self.emit(abi::add_immediate(new_cursor, new_cursor, 1));
        self.emit(abi::add_immediate(dest, dest, 1));
        self.emit(abi::add_immediate(new_index, new_index, 1));
        self.emit(abi::branch(&second_copy_new_loop));
        self.emit(abi::label(&second_copy_new_done));
        self.emit(abi::add_registers(index, index, old_len));
        self.emit(abi::branch(&second_loop));

        self.emit(abi::label(&second_copy_one));
        self.emit(abi::add_registers(candidate, value_ptr, index));
        self.emit(abi::load_u8(value_byte, candidate, 0));
        self.emit(abi::store_u8(value_byte, dest, 0));
        self.emit(abi::add_immediate(dest, dest, 1));
        self.emit(abi::add_immediate(index, index, 1));
        self.emit(abi::branch(&second_loop));

        self.emit(abi::label(&second_done));
        if matches!(out, StringOut::Block) {
            self.emit(abi::move_immediate(value_byte, "Integer", "0"));
            self.emit(abi::store_u8(value_byte, dest, 0));
        }
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        if matches!(out, StringOut::Scratch) {
            // A replacement happened, so the arm publishes the scratch.
            self.emit(abi::move_immediate(value_byte, "Integer", "0"));
            self.emit(abi::store_u64(
                value_byte,
                abi::stack_pointer(),
                unchanged_slot,
            ));
        }
        self.emit(abi::branch(&done));

        self.emit(abi::label(&empty_old));
        self.raise_error("strings.replace", "ErrInvalidArgument")?;

        self.emit(abi::label(&copy_original));
        if matches!(out, StringOut::Scratch) {
            // plan-146-E: nothing matched, so the binding already holds the result;
            // the arm skips the copy back entirely (and skips this deep copy).
            let one = self.temporary_vreg();
            self.emit(abi::move_immediate(&one, "Integer", "1"));
            self.emit(abi::store_u64(&one, abi::stack_pointer(), unchanged_slot));
            self.emit(abi::branch(&done));
        }
        // No replacement occurred. The caller owns and frees this result, so it
        // must be a fresh arena block, not an alias of the input `value` (which
        // may be a caller local or a static constant — freeing either would
        // double-free or fault). Deep-copy the input into the arena.
        let original_ptr = self.allocate_register();
        self.emit(abi::load_u64(
            &original_ptr,
            abi::stack_pointer(),
            value_slot,
        ));
        let copied = self.copy_flat_block(&ParameterType::String, &original_ptr)?;
        self.emit(abi::move_register(&result, &copied));
        self.emit(abi::label(&done));

        if matches!(out, StringOut::Scratch) {
            return Ok((
                ValueResult {
                    origin: None,
                    type_: ParameterType::String,
                    location: Operand::from(result),
                    text: "strings.replace".to_string(),
                },
                output_len_slot,
                unchanged_slot,
            ));
        }
        // bug-536 shape B: BOTH arms put a fresh block in `result` — the
        // replacing arm its own `emit_arena_alloc_call`, the no-match arm the
        // `copy_flat_block` right above (which exists precisely so the result is
        // never an alias of the input).
        self.mark_fresh_string(Operand::from(result));
        Ok((
            ValueResult {
                origin: None,
                type_: ParameterType::String,
                location: Operand::from(result),
                text: "replace(String, String, String)".to_string(),
            },
            output_len_slot,
            unchanged_slot,
        ))
    }

    pub(crate) fn lower_list_replace(
        &mut self,
        value_slot: usize,
        old_slot: usize,
        new_slot: usize,
        list_type: &ParameterType,
        element_type: &ParameterType,
    ) -> Result<ValueResult, String> {
        // Entry stride for this element type: zero builds the result entry-free
        // and makes every cursor below stride the data region (plan-57-D).
        let rep_stride = list_entry_stride(element_type);
        let rep_payload = kind2_payload_size(element_type);
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
        let scratch23 = self.temporary_vreg();
        let scratch24 = self.temporary_vreg();
        let scratch25 = self.temporary_vreg();

        let new_payload = PayloadSlot {
            slot: new_slot,
            type_: element_type.clone(),
        };
        let new_len_slot = self.emit_payload_length_to_stack(&new_payload, "replace_new_len")?;
        let data_len_slot = self.allocate_stack_object("replace_list_data_len", 8);
        let result_slot = self.allocate_stack_object("replace_list_result", 8);
        let loop_label = self.label("replace_list_length_loop");
        let add_new = self.label("replace_list_length_add_new");
        let add_old = self.label("replace_list_length_add_old");
        let length_next = self.label("replace_list_length_next");
        let length_done = self.label("replace_list_length_done");
        let alloc_ok = self.label("replace_list_alloc_ok");
        let overflow = self.label("replace_list_overflow");
        let copy_loop = self.label("replace_list_copy_loop");
        let copy_new = self.label("replace_list_copy_new");
        let copy_old = self.label("replace_list_copy_old");
        let copy_done_one = self.label("replace_list_copy_done_one");
        let copy_done = self.label("replace_list_copy_done");

        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), old_slot));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch8,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::move_immediate(&scratch12, "Integer", "0"));
        self.emit(abi::move_immediate(&scratch15, "Integer", "0"));
        if rep_payload.is_some() {
            // kind 2: a byte OFFSET into the data region, not an entry pointer.
            self.emit(abi::move_immediate(&scratch16, "Integer", "0"));
        } else {
            self.emit(abi::add_immediate(
                &scratch16,
                &scratch8,
                COLLECTION_HEADER_SIZE,
            ));
        }
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_registers(&scratch12, &scratch11));
        self.emit(abi::branch_ge(&length_done));
        if let Some(payload) = rep_payload {
            self.emit(abi::move_register(&scratch17, &scratch16));
            self.emit(abi::move_immediate(
                &scratch20,
                "Integer",
                &payload.to_string(),
            ));
        } else {
            self.emit(abi::load_u64(
                &scratch17,
                &scratch16,
                COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
            ));
            self.emit(abi::load_u64(
                &scratch20,
                &scratch16,
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ));
        }
        self.emit_collection_payload_matches_value_branch(
            element_type,
            element_type,
            &scratch8,
            &scratch17,
            &scratch20,
            &scratch9,
            &add_new,
            &add_old,
        )?;
        self.emit(abi::label(&add_new));
        self.emit(abi::load_u64(
            &scratch21,
            abi::stack_pointer(),
            new_len_slot,
        ));
        self.emit_checked_size_add(&scratch15, &scratch15, &scratch21, &overflow);
        self.emit(abi::branch(&length_next));
        self.emit(abi::label(&add_old));
        self.emit_checked_size_add(&scratch15, &scratch15, &scratch20, &overflow);
        self.emit(abi::label(&length_next));
        self.emit(abi::add_immediate(
            &scratch16,
            &scratch16,
            rep_payload.unwrap_or(COLLECTION_ENTRY_SIZE),
        ));
        self.emit(abi::add_immediate(&scratch12, &scratch12, 1));
        self.emit(abi::branch(&loop_label));

        self.emit(abi::label(&length_done));
        self.emit(abi::store_u64(
            &scratch15,
            abi::stack_pointer(),
            data_len_slot,
        ));
        self.emit(abi::move_immediate(
            &scratch14,
            "Integer",
            &rep_stride.to_string(),
        ));
        // size = count*ENTRY_SIZE + HEADER + data_len, trapping any 64-bit wrap so
        // the copy pass cannot overrun the (undersized) allocation (bug-60).
        self.emit_checked_size_multiply(&scratch16, &scratch11, &scratch14, &overflow);
        self.emit_checked_size_add_immediate(
            abi::return_register(),
            &scratch16,
            COLLECTION_HEADER_SIZE,
            &overflow,
        );
        self.emit_checked_size_add(
            abi::return_register(),
            abi::return_register(),
            &scratch15,
            &overflow,
        );
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&alloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        // A size wrap reports the same 77010001 an impossible allocation would
        // (x0 does not hold an error code before the call, so the register-based
        // return above cannot be shared). The checked-size helper deposits the
        // partially-computed size into the return register before branching here,
        // so `emit_allocation_error_return` would surface that size as the error
        // code (bug-60 detection, bug-352 code fix).
        self.emit(abi::label(&overflow));
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
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch8,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::store_u64(
            &scratch11,
            abi::mfb_return(1),
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::store_u64(
            &scratch11,
            abi::mfb_return(1),
            COLLECTION_OFFSET_CAPACITY,
        ));
        self.emit(abi::load_u64(
            &scratch15,
            abi::stack_pointer(),
            data_len_slot,
        ));
        self.emit(abi::store_u64(
            &scratch15,
            abi::mfb_return(1),
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::store_u64(
            &scratch15,
            abi::mfb_return(1),
            COLLECTION_OFFSET_DATA_CAPACITY,
        ));

        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), old_slot));
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), new_slot));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch8,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::load_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            result_slot,
        ));
        if rep_payload.is_some() {
            // kind 2: the copy pass re-seeds the SOURCE cursor as a byte offset,
            // exactly as the length pass did.
            self.emit(abi::move_immediate(&scratch16, "Integer", "0"));
        } else {
            self.emit(abi::add_immediate(
                &scratch16,
                &scratch8,
                COLLECTION_HEADER_SIZE,
            ));
        }
        self.emit(abi::add_immediate(
            &scratch17,
            abi::mfb_return(1),
            COLLECTION_HEADER_SIZE,
        ));
        self.emit_collection_data_pointer_for(&scratch20, &scratch8, element_type);
        self.emit(abi::move_immediate(
            &scratch14,
            "Integer",
            &rep_stride.to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch21, &scratch11, &scratch14));
        self.emit(abi::add_registers(&scratch21, &scratch17, &scratch21));
        self.emit(abi::move_immediate(&scratch12, "Integer", "0"));
        self.emit(abi::move_immediate(&scratch13, "Integer", "0"));

        self.emit(abi::label(&copy_loop));
        self.emit(abi::compare_registers(&scratch12, &scratch11));
        self.emit(abi::branch_ge(&copy_done));
        self.emit(abi::move_immediate(
            &scratch22,
            "Byte",
            &COLLECTION_ENTRY_FLAG_USED.to_string(),
        ));
        if rep_stride != 0 {
            self.emit(abi::store_u8(
                &scratch22,
                &scratch17,
                COLLECTION_ENTRY_OFFSET_FLAGS,
            ));
        }
        self.emit(abi::move_immediate(&scratch22, "Integer", "0"));
        if rep_stride != 0 {
            self.emit(abi::store_u64(
                &scratch22,
                &scratch17,
                COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
            ));
        }
        if rep_stride != 0 {
            self.emit(abi::store_u64(
                &scratch22,
                &scratch17,
                COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
            ));
        }
        if let Some(payload) = rep_payload {
            // kind 2: scratch16 is already the source byte offset.
            self.emit(abi::move_register(&scratch22, &scratch16));
            self.emit(abi::move_immediate(
                &scratch23,
                "Integer",
                &payload.to_string(),
            ));
        } else {
            self.emit(abi::load_u64(
                &scratch22,
                &scratch16,
                COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
            ));
            self.emit(abi::load_u64(
                &scratch23,
                &scratch16,
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ));
        }
        if rep_stride != 0 {
            self.emit(abi::store_u64(
                &scratch13,
                &scratch17,
                COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
            ));
        }
        self.emit_collection_payload_matches_value_branch(
            element_type,
            element_type,
            &scratch8,
            &scratch22,
            &scratch23,
            &scratch9,
            &copy_new,
            &copy_old,
        )?;

        self.emit(abi::label(&copy_new));
        self.emit(abi::load_u64(
            &scratch23,
            abi::stack_pointer(),
            new_len_slot,
        ));
        if rep_stride != 0 {
            self.emit(abi::store_u64(
                &scratch23,
                &scratch17,
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ));
        }
        self.emit(abi::add_registers(&scratch25, &scratch21, &scratch13));
        match element_type {
            ParameterType::Boolean | ParameterType::Byte => {
                self.emit(abi::load_u64(&scratch24, abi::stack_pointer(), new_slot));
                self.emit(abi::store_u8(&scratch24, &scratch25, 0));
            }
            ParameterType::Integer | ParameterType::Float | ParameterType::Fixed => {
                self.emit(abi::load_u64(&scratch24, abi::stack_pointer(), new_slot));
                self.emit(abi::store_u64(&scratch24, &scratch25, 0));
            }
            ParameterType::String => {
                self.emit(abi::load_u64(&scratch24, abi::stack_pointer(), new_slot));
                self.emit(abi::add_immediate(&scratch24, &scratch24, 8));
                self.emit_block_copy_advance(
                    &scratch25,
                    &scratch24,
                    &scratch23,
                    &scratch22,
                    "replace_list_copy_new_string",
                );
            }
            other if self.inline_collection_payload_size(other).is_some() => {
                self.emit(abi::load_u64(&scratch24, abi::stack_pointer(), new_slot));
                self.emit_block_copy_advance(
                    &scratch25,
                    &scratch24,
                    &scratch23,
                    &scratch22,
                    "replace_list_copy_new_inline",
                );
            }
            _ => {
                self.emit(abi::load_u64(&scratch24, abi::stack_pointer(), new_slot));
                self.emit(abi::store_u64(&scratch24, &scratch25, 0));
            }
        }
        self.emit(abi::branch(&copy_done_one));

        self.emit(abi::label(&copy_old));
        if rep_stride != 0 {
            self.emit(abi::store_u64(
                &scratch23,
                &scratch17,
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ));
        }
        self.emit(abi::add_registers(&scratch24, &scratch20, &scratch22));
        self.emit(abi::add_registers(&scratch25, &scratch21, &scratch13));
        self.emit_block_copy_advance(
            &scratch25,
            &scratch24,
            &scratch23,
            &scratch22,
            "replace_list_copy_old",
        );

        self.emit(abi::label(&copy_done_one));
        if let Some(payload) = rep_payload {
            // No destination entry to read the written length back from; for a
            // fixed-width element it is the constant payload size.
            self.emit(abi::move_immediate(
                &scratch23,
                "Integer",
                &payload.to_string(),
            ));
        } else {
            self.emit(abi::load_u64(
                &scratch23,
                &scratch17,
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ));
        }
        self.emit(abi::add_registers(&scratch13, &scratch13, &scratch23));
        self.emit(abi::add_immediate(
            &scratch16,
            &scratch16,
            rep_payload.unwrap_or(COLLECTION_ENTRY_SIZE),
        ));
        self.emit(abi::add_immediate(&scratch17, &scratch17, rep_stride));
        self.emit(abi::add_immediate(&scratch12, &scratch12, 1));
        self.emit(abi::branch(&copy_loop));
        self.emit(abi::label(&copy_done));

        let result = self.allocate_register();
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            origin: None,
            type_: list_type.clone(),
            location: Operand::from(result),
            text: format!("replace({list_type}, {element_type}, {element_type})"),
        })
    }
}
