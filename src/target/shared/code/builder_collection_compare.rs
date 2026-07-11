use super::*;

impl CodeBuilder<'_> {
    pub(super) fn emit_compare_bytes_branch(
        &mut self,
        left: &str,
        right: &str,
        len: &str,
        equal_label: &str,
        not_equal_label: &str,
        prefix: &str,
    ) {
        // Scratch as vregs (was out-of-pool x5/x6/x7).
        let remaining_v = self.temporary_vreg();
        let lbyte_v = self.temporary_vreg();
        let rbyte_v = self.temporary_vreg();
        let remaining = remaining_v.as_str();
        let lbyte = lbyte_v.as_str();
        let rbyte = rbyte_v.as_str();
        let loop_label = self.label(&format!("{prefix}_loop"));
        self.emit(abi::move_register(remaining, len));
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate(remaining, "0"));
        self.emit(abi::branch_eq(equal_label));
        self.emit(abi::load_u8(lbyte, left, 0));
        self.emit(abi::load_u8(rbyte, right, 0));
        self.emit(abi::compare_registers(lbyte, rbyte));
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
        // Scratch as vregs (was out-of-pool x2-x7); d0/d1 stay (FP compare).
        let lcur_v = self.temporary_vreg();
        let tmp_v = self.temporary_vreg();
        let rcur_v = self.temporary_vreg();
        let len_v = self.temporary_vreg();
        let lval_v = self.temporary_vreg();
        let rval_v = self.temporary_vreg();
        let lcur = lcur_v.as_str();
        let tmp = tmp_v.as_str();
        let rcur = rcur_v.as_str();
        let len = len_v.as_str();
        let lval = lval_v.as_str();
        let rval = rval_v.as_str();
        match type_ {
            "Nothing" => {
                self.emit(abi::branch(equal_label));
            }
            "Boolean" | "Byte" | "Integer" | "Fixed" => {
                self.emit(abi::load_u64(lval, abi::stack_pointer(), left_slot));
                self.emit(abi::load_u64(rval, abi::stack_pointer(), right_slot));
                self.emit(abi::compare_registers(lval, rval));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            "Float" => {
                self.emit(abi::load_u64(lval, abi::stack_pointer(), left_slot));
                self.emit(abi::load_u64(rval, abi::stack_pointer(), right_slot));
                self.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[0], lval));
                self.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[1], rval));
                self.emit(abi::float_compare_d(abi::FP_SCRATCH[0], abi::FP_SCRATCH[1]));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            "String" => {
                let loop_label = self.label("compare_string_value_loop");
                self.emit(abi::load_u64(lcur, abi::stack_pointer(), left_slot));
                self.emit(abi::load_u64(rcur, abi::stack_pointer(), right_slot));
                self.emit(abi::load_u64(len, lcur, 0));
                self.emit(abi::load_u64(lval, rcur, 0));
                self.emit(abi::compare_registers(len, lval));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit(abi::add_immediate(lcur, lcur, 8));
                self.emit(abi::add_immediate(rcur, rcur, 8));
                self.emit(abi::label(&loop_label));
                self.emit(abi::compare_immediate(len, "0"));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::load_u8(lval, lcur, 0));
                self.emit(abi::load_u8(rval, rcur, 0));
                self.emit(abi::compare_registers(lval, rval));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit(abi::add_immediate(lcur, lcur, 1));
                self.emit(abi::add_immediate(rcur, rcur, 1));
                self.emit(abi::subtract_immediate(len, len, 1));
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
                let inline_string_field = fields
                    .iter()
                    .map(|(_, ft)| self.record_field_is_inlined(other, ft))
                    .collect::<Vec<_>>();
                for (index, (_, field_type)) in fields.iter().enumerate() {
                    let next_field = self.label("compare_record_next_field");
                    let field_left_slot = self.allocate_stack_object("compare_record_left", 8);
                    let field_right_slot = self.allocate_stack_object("compare_record_right", 8);
                    self.emit(abi::load_u64(lcur, abi::stack_pointer(), left_slot));
                    self.emit(abi::load_u64(rcur, abi::stack_pointer(), right_slot));
                    if inline_string_field[index] {
                        // The slot is a block-relative offset; recover the String
                        // borrow pointer (record base + offset) before comparing.
                        self.emit(abi::load_u64(tmp, lcur, index * 8));
                        self.emit(abi::add_registers(lcur, lcur, tmp));
                        self.emit(abi::load_u64(tmp, rcur, index * 8));
                        self.emit(abi::add_registers(rcur, rcur, tmp));
                    } else {
                        self.emit(abi::load_u64(lcur, lcur, index * 8));
                        self.emit(abi::load_u64(rcur, rcur, index * 8));
                    }
                    self.emit(abi::store_u64(lcur, abi::stack_pointer(), field_left_slot));
                    self.emit(abi::store_u64(rcur, abi::stack_pointer(), field_right_slot));
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
                self.emit(abi::load_u64(lval, abi::stack_pointer(), left_slot));
                self.emit(abi::load_u64(rval, abi::stack_pointer(), right_slot));
                self.emit(abi::compare_registers(lval, rval));
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
        // Scratch as vregs (was out-of-pool x2-x7).
        let cur_v = self.temporary_vreg();
        let tmp_v = self.temporary_vreg();
        let vcur_v = self.temporary_vreg();
        let rem_v = self.temporary_vreg();
        let cval_v = self.temporary_vreg();
        let vbyte_v = self.temporary_vreg();
        let cur = cur_v.as_str();
        let tmp = tmp_v.as_str();
        let vcur = vcur_v.as_str();
        let rem = rem_v.as_str();
        let cval = cval_v.as_str();
        let vbyte = vbyte_v.as_str();
        self.emit(abi::move_register(cur, collection));
        self.emit(abi::move_register(tmp, offset));
        self.emit_collection_data_pointer(cur, cur);
        self.emit(abi::add_registers(cur, cur, tmp));
        match type_ {
            "Boolean" | "Byte" => {
                self.emit(abi::load_u8(cval, cur, 0));
                self.emit(abi::compare_registers(cval, value));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            "Integer" | "Float" | "Fixed" => {
                self.emit(abi::load_u64(cval, cur, 0));
                self.emit(abi::compare_registers(cval, value));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            "String" => {
                let loop_label = self.label("collection_string_value_match_loop");
                self.emit(abi::load_u64(tmp, value, 0));
                self.emit(abi::compare_registers(length, tmp));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit(abi::add_immediate(vcur, value, 8));
                self.emit(abi::move_register(rem, length));
                self.emit(abi::label(&loop_label));
                self.emit(abi::compare_immediate(rem, "0"));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::load_u8(cval, cur, 0));
                self.emit(abi::load_u8(vbyte, vcur, 0));
                self.emit(abi::compare_registers(cval, vbyte));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit(abi::add_immediate(cur, cur, 1));
                self.emit(abi::add_immediate(vcur, vcur, 1));
                self.emit(abi::subtract_immediate(rem, rem, 1));
                self.emit(abi::branch(&loop_label));
            }
            other if self.is_pointer_collection_payload_type(other) => {
                self.emit(abi::load_u64(cval, cur, 0));
                self.emit(abi::compare_registers(cval, value));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            other if self.type_model.record_fields.contains_key(other) => {
                self.emit_comparable_values_match_branch(
                    other,
                    cur,
                    value,
                    equal_label,
                    not_equal_label,
                )?;
            }
            other if self.inline_collection_payload_size(other).is_some() => {
                self.emit_compare_bytes_branch(
                    cur,
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
        // Scratch as vregs (was out-of-pool x2-x7).
        let lcur_v = self.temporary_vreg();
        let loff_v = self.temporary_vreg();
        let rcur_v = self.temporary_vreg();
        let roff_v = self.temporary_vreg();
        let lval_v = self.temporary_vreg();
        let rval_v = self.temporary_vreg();
        let lcur = lcur_v.as_str();
        let loff = loff_v.as_str();
        let rcur = rcur_v.as_str();
        let roff = roff_v.as_str();
        let lval = lval_v.as_str();
        let rval = rval_v.as_str();
        self.emit(abi::move_register(lcur, left_collection));
        self.emit(abi::move_register(loff, left_offset));
        self.emit(abi::move_register(rcur, right_collection));
        self.emit(abi::move_register(roff, right_offset));
        self.emit_collection_data_pointer(lcur, lcur);
        self.emit(abi::add_registers(lcur, lcur, loff));
        self.emit_collection_data_pointer(rcur, rcur);
        self.emit(abi::add_registers(rcur, rcur, roff));
        match type_ {
            "Boolean" | "Byte" => {
                self.emit(abi::load_u8(lval, lcur, 0));
                self.emit(abi::load_u8(rval, rcur, 0));
                self.emit(abi::compare_registers(lval, rval));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            "Integer" | "Float" | "Fixed" => {
                self.emit(abi::load_u64(lval, lcur, 0));
                self.emit(abi::load_u64(rval, rcur, 0));
                self.emit(abi::compare_registers(lval, rval));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            "String" => {
                let loop_label = self.label("collection_payload_string_match_loop");
                self.emit(abi::compare_registers(left_length, right_length));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit(abi::move_register(roff, left_length));
                self.emit(abi::label(&loop_label));
                self.emit(abi::compare_immediate(roff, "0"));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::load_u8(lval, lcur, 0));
                self.emit(abi::load_u8(rval, rcur, 0));
                self.emit(abi::compare_registers(lval, rval));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit(abi::add_immediate(lcur, lcur, 1));
                self.emit(abi::add_immediate(rcur, rcur, 1));
                self.emit(abi::subtract_immediate(roff, roff, 1));
                self.emit(abi::branch(&loop_label));
            }
            other if self.is_pointer_collection_payload_type(other) => {
                self.emit(abi::load_u64(lval, lcur, 0));
                self.emit(abi::load_u64(rval, rcur, 0));
                self.emit(abi::compare_registers(lval, rval));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            other if self.type_model.record_fields.contains_key(other) => {
                self.emit(abi::compare_registers(left_length, right_length));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit_comparable_values_match_branch(
                    other,
                    lcur,
                    rcur,
                    equal_label,
                    not_equal_label,
                )?;
            }
            other if self.inline_collection_payload_size(other).is_some() => {
                self.emit(abi::compare_registers(left_length, right_length));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit_compare_bytes_branch(
                    lcur,
                    rcur,
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
