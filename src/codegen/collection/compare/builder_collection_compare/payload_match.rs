//! plan-176-Q: matching collection payloads against a value or each other. The byte compares
//! and comparable matches live in `collection/compare/builder_collection_compare/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    #[allow(clippy::too_many_arguments)]
    /// `stride_type` selects the data-base entry stride: the element type for a
    /// LIST block, or `""` for a MAP block, which keeps its lookup table
    /// whatever its key and value types are. Passing `type_` here for a map
    /// would address a `Map OF Scalar TO T` past its own entry array
    /// (plan-57-D).
    pub(crate) fn emit_collection_payload_match_branch(
        &mut self,
        type_: &ParameterType,
        stride_type: &ParameterType,
        collection: impl Into<abi::Reg>,
        offset: impl Into<abi::Reg>,
        length: impl Into<abi::Reg>,
        value: impl Into<abi::Reg>,
        equal_label: &str,
        not_equal_label: &str,
    ) -> Result<(), String> {
        let length = length.into();
        let value = value.into();
        let data = self.allocate_register();
        self.emit_collection_data_pointer_for(&data, collection, stride_type);
        self.emit(abi::add_registers(&data, &data, offset));
        match type_ {
            // bug-745: the unit type has one value, so two payloads are equal.
            ParameterType::Nothing => {
                self.emit(abi::branch(equal_label));
            }
            ParameterType::Boolean | ParameterType::Byte => {
                let candidate = self.allocate_register();
                self.emit(abi::load_u8(&candidate, &data, 0));
                self.emit(abi::compare_registers(&candidate, value));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            type_ if type_.is_named("Scalar") => {
                let candidate = self.allocate_register();
                self.emit(abi::load_u32(&candidate, &data, 0));
                self.emit(abi::compare_registers(&candidate, value));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            ParameterType::Integer
            | ParameterType::Float
            | ParameterType::Fixed
            | ParameterType::Money => {
                let candidate = self.allocate_register();
                self.emit(abi::load_u64(&candidate, &data, 0));
                self.emit(abi::compare_registers(&candidate, value));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            // bug-549: two enum ordinals compare as two words. The comparator
            // for a RECORD FIELD of enum type already did this; a collection
            // ELEMENT of enum type could not reach any comparator, because the
            // payload classifier refused the type first.
            other if self.is_enum_type(other) => {
                let candidate = self.allocate_register();
                self.emit(abi::load_u64(&candidate, &data, 0));
                self.emit(abi::compare_registers(&candidate, value));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            ParameterType::String => {
                let value_len = self.allocate_register();
                let value_cursor = self.allocate_register();
                let remaining = self.allocate_register();
                let packed_byte = self.allocate_register();
                let value_byte = self.allocate_register();
                let loop_label = self.label("collection_string_match_loop");
                self.emit(abi::load_u64(&value_len, value, 0));
                self.emit(abi::compare_registers(length, &value_len));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit(abi::add_immediate(&value_cursor, value, 8));
                self.emit(abi::move_register(&remaining, length));
                self.emit_byte_compare_loop(
                    &data,
                    value_cursor,
                    &remaining,
                    &packed_byte,
                    value_byte,
                    &loop_label,
                    equal_label,
                    not_equal_label,
                );
            }
            other if self.is_pointer_collection_payload_type(other) => {
                let candidate = self.allocate_register();
                self.emit(abi::load_u64(&candidate, &data, 0));
                self.emit(abi::compare_registers(&candidate, value));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            other if self.type_model.record_fields.contains_key(other) => {
                self.emit_comparable_keys_match_branch(
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

    #[allow(clippy::too_many_arguments)]
    /// `stride_type` selects the data-base entry stride: the element type for a
    /// LIST block, or `""` for a MAP block, which keeps its lookup table
    /// whatever its key and value types are. Passing `type_` here for a map
    /// would address a `Map OF Scalar TO T` past its own entry array
    /// (plan-57-D).
    pub(crate) fn emit_collection_payload_matches_value_branch(
        &mut self,
        type_: &ParameterType,
        stride_type: &ParameterType,
        collection: impl Into<abi::Reg>,
        offset: impl Into<abi::Reg>,
        length: impl Into<abi::Reg>,
        value: impl Into<abi::Reg>,
        equal_label: &str,
        not_equal_label: &str,
    ) -> Result<(), String> {
        // Scratch as vregs.
        let cur_v = self.temporary_vreg();
        let tmp_v = self.temporary_vreg();
        let vcur_v = self.temporary_vreg();
        let rem_v = self.temporary_vreg();
        let cval_v = self.temporary_vreg();
        let vbyte_v = self.temporary_vreg();
        let cur = &cur_v;
        let tmp = &tmp_v;
        let vcur = &vcur_v;
        let rem = &rem_v;
        let cval = &cval_v;
        let vbyte = &vbyte_v;
        let length = length.into();
        let value = value.into();
        self.emit(abi::move_register(cur, collection));
        self.emit(abi::move_register(tmp, offset));
        self.emit_collection_data_pointer_for(cur, cur, stride_type);
        self.emit(abi::add_registers(cur, cur, tmp));
        match type_ {
            // bug-745: the unit type has one value, so two payloads are equal.
            ParameterType::Nothing => {
                self.emit(abi::branch(equal_label));
            }
            ParameterType::Boolean | ParameterType::Byte => {
                self.emit(abi::load_u8(cval, cur, 0));
                self.emit(abi::compare_registers(cval, value));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            type_ if type_.is_named("Scalar") => {
                self.emit(abi::load_u32(cval, cur, 0));
                self.emit(abi::compare_registers(cval, value));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            ParameterType::Integer
            | ParameterType::Float
            | ParameterType::Fixed
            | ParameterType::Money => {
                self.emit(abi::load_u64(cval, cur, 0));
                self.emit(abi::compare_registers(cval, value));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            ParameterType::String => {
                let loop_label = self.label("collection_string_value_match_loop");
                self.emit(abi::load_u64(tmp, value, 0));
                self.emit(abi::compare_registers(length, tmp));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit(abi::add_immediate(vcur, value, 8));
                self.emit(abi::move_register(rem, length));
                self.emit_byte_compare_loop(
                    cur,
                    vcur,
                    rem,
                    cval,
                    vbyte,
                    &loop_label,
                    equal_label,
                    not_equal_label,
                );
            }
            // bug-549: an enum element, as the word it is.
            other if self.is_enum_type(other) => {
                self.emit(abi::load_u64(cval, cur, 0));
                self.emit(abi::compare_registers(cval, value));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            other if self.is_pointer_collection_payload_type(other) => {
                self.emit(abi::load_u64(cval, cur, 0));
                self.emit(abi::compare_registers(cval, value));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            other if self.type_model.record_fields.contains_key(other) => {
                self.emit_comparable_keys_match_branch(
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

    #[allow(clippy::too_many_arguments)]
    /// `stride_type` selects the data-base entry stride: the element type for a
    /// LIST block, or `""` for a MAP block, which keeps its lookup table
    /// whatever its key and value types are. Passing `type_` here for a map
    /// would address a `Map OF Scalar TO T` past its own entry array
    /// (plan-57-D).
    pub(crate) fn emit_collection_payloads_match_branch(
        &mut self,
        type_: &ParameterType,
        stride_type: &ParameterType,
        left_collection: impl Into<abi::Reg>,
        left_offset: impl Into<abi::Reg>,
        left_length: impl Into<abi::Reg>,
        right_collection: impl Into<abi::Reg>,
        right_offset: impl Into<abi::Reg>,
        right_length: impl Into<abi::Reg>,
        equal_label: &str,
        not_equal_label: &str,
    ) -> Result<(), String> {
        // Scratch as vregs.
        let lcur_v = self.temporary_vreg();
        let loff_v = self.temporary_vreg();
        let rcur_v = self.temporary_vreg();
        let roff_v = self.temporary_vreg();
        let lval_v = self.temporary_vreg();
        let rval_v = self.temporary_vreg();
        let lcur = &lcur_v;
        let loff = &loff_v;
        let rcur = &rcur_v;
        let roff = &roff_v;
        let lval = &lval_v;
        let rval = &rval_v;
        let left_length = left_length.into();
        let right_length = right_length.into();
        self.emit(abi::move_register(lcur, left_collection));
        self.emit(abi::move_register(loff, left_offset));
        self.emit(abi::move_register(rcur, right_collection));
        self.emit(abi::move_register(roff, right_offset));
        self.emit_collection_data_pointer_for(lcur, lcur, stride_type);
        self.emit(abi::add_registers(lcur, lcur, loff));
        self.emit_collection_data_pointer_for(rcur, rcur, stride_type);
        self.emit(abi::add_registers(rcur, rcur, roff));
        match type_ {
            // bug-745: the unit type has one value, so two payloads are equal.
            ParameterType::Nothing => {
                self.emit(abi::branch(equal_label));
            }
            ParameterType::Boolean | ParameterType::Byte => {
                self.emit(abi::load_u8(lval, lcur, 0));
                self.emit(abi::load_u8(rval, rcur, 0));
                self.emit(abi::compare_registers(lval, rval));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            type_ if type_.is_named("Scalar") => {
                self.emit(abi::load_u32(lval, lcur, 0));
                self.emit(abi::load_u32(rval, rcur, 0));
                self.emit(abi::compare_registers(lval, rval));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            ParameterType::Integer
            | ParameterType::Float
            | ParameterType::Fixed
            | ParameterType::Money => {
                self.emit(abi::load_u64(lval, lcur, 0));
                self.emit(abi::load_u64(rval, rcur, 0));
                self.emit(abi::compare_registers(lval, rval));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            // bug-549: two enum elements, pairwise.
            other if self.is_enum_type(other) => {
                self.emit(abi::load_u64(lval, lcur, 0));
                self.emit(abi::load_u64(rval, rcur, 0));
                self.emit(abi::compare_registers(lval, rval));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            ParameterType::String => {
                let loop_label = self.label("collection_payload_string_match_loop");
                self.emit(abi::compare_registers(left_length, right_length));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit(abi::move_register(roff, left_length));
                self.emit_byte_compare_loop(
                    lcur,
                    rcur,
                    roff,
                    lval,
                    rval,
                    &loop_label,
                    equal_label,
                    not_equal_label,
                );
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
                self.emit_comparable_keys_match_branch(
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
