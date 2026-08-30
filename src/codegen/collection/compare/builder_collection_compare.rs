// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::target::shared::abi;
use crate::types::ParameterType;
impl CodeBuilder<'_> {
    pub(crate) fn emit_compare_bytes_branch(
        &mut self,
        left: impl Into<Operand>,
        right: impl Into<Operand>,
        len: impl Into<Operand>,
        equal_label: &str,
        not_equal_label: &str,
        prefix: &str,
    ) {
        // Scratch as vregs. bug-175 D: the byte loop advances private scratch
        // copies of the left/right pointers, not the caller's registers — a
        // non-first-entry map-key compare must leave the caller's key pointer
        // untouched.
        let remaining_v = self.temporary_vreg();
        let lbyte_v = self.temporary_vreg();
        let rbyte_v = self.temporary_vreg();
        let lptr_v = self.temporary_vreg();
        let rptr_v = self.temporary_vreg();
        let remaining = &remaining_v;
        let lbyte = &lbyte_v;
        let rbyte = &rbyte_v;
        let lptr = &lptr_v;
        let rptr = &rptr_v;
        let loop_label = self.label(&format!("{prefix}_loop"));
        self.emit(abi::move_register(remaining, len));
        self.emit(abi::move_register(lptr, left));
        self.emit(abi::move_register(rptr, right));
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate(remaining, "0"));
        self.emit(abi::branch_eq(equal_label));
        self.emit(abi::load_u8(lbyte, lptr, 0));
        self.emit(abi::load_u8(rbyte, rptr, 0));
        self.emit(abi::compare_registers(lbyte, rbyte));
        self.emit(abi::branch_ne(not_equal_label));
        self.emit(abi::add_immediate(lptr, lptr, 1));
        self.emit(abi::add_immediate(rptr, rptr, 1));
        self.emit(abi::subtract_immediate(remaining, remaining, 1));
        self.emit(abi::branch(&loop_label));
    }

    /// The byte-by-byte equality loop the three collection payload `String` arms
    /// each hand-rolled. `left`/`right` are advanced IN PLACE and `counter` counts
    /// down to zero; the caller emits the preceding length check and cursor setup,
    /// mints every register and the loop label, and this emits only the loop body.
    ///
    /// This is deliberately NOT [`Self::emit_compare_bytes_branch`], which walks
    /// private scratch copies to leave a caller's map-key pointer untouched
    /// (bug-175 D) — routing these sites through it would add two `move_register`
    /// ops and shift generated output. These arms already own and consume their
    /// cursors, so extracting only the shared loop body keeps the output
    /// byte-identical.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn emit_byte_compare_loop(
        &mut self,
        left: impl Into<Operand>,
        right: impl Into<Operand>,
        counter: impl Into<Operand>,
        left_byte: impl Into<Operand>,
        right_byte: impl Into<Operand>,
        loop_label: &str,
        equal_label: &str,
        not_equal_label: &str,
    ) {
        let left = left.into();
        let right = right.into();
        let counter = counter.into();
        let left_byte = left_byte.into();
        let right_byte = right_byte.into();
        self.emit(abi::label(loop_label));
        self.emit(abi::compare_immediate(counter.clone(), "0"));
        self.emit(abi::branch_eq(equal_label));
        self.emit(abi::load_u8(left_byte.clone(), left.clone(), 0));
        self.emit(abi::load_u8(right_byte.clone(), right.clone(), 0));
        self.emit(abi::compare_registers(
            left_byte.clone(),
            right_byte.clone(),
        ));
        self.emit(abi::branch_ne(not_equal_label));
        self.emit(abi::add_immediate(left.clone(), left.clone(), 1));
        self.emit(abi::add_immediate(right.clone(), right.clone(), 1));
        self.emit(abi::subtract_immediate(counter.clone(), counter.clone(), 1));
        self.emit(abi::branch(loop_label));
    }

    pub(crate) fn emit_comparable_values_match_branch(
        &mut self,
        type_: &str,
        left: impl Into<Operand>,
        right: impl Into<Operand>,
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
        // Scratch as vregs. No FP register is held here: bug-147 made the `Float`
        // arm a bitwise integer compare (see the note on that arm below).
        let lcur_v = self.temporary_vreg();
        let tmp_v = self.temporary_vreg();
        let rcur_v = self.temporary_vreg();
        let len_v = self.temporary_vreg();
        let lval_v = self.temporary_vreg();
        let rval_v = self.temporary_vreg();
        let lcur = &lcur_v;
        let tmp = &tmp_v;
        let rcur = &rcur_v;
        let len = &len_v;
        let lval = &lval_v;
        let rval = &rval_v;
        match type_ {
            "Nothing" => {
                self.emit(abi::branch(equal_label));
            }
            // bug-147: `Float` is compared BITWISE here (loaded bits via
            // `compare_registers`), matching the packed-payload arms in
            // `emit_collection_payload*` and the documented map-literal key
            // semantics (0.0 != -0.0, NaN == NaN). Using `float_compare_d`
            // gave inconsistent FP semantics for record-field Floats.
            // Scalar joins here: a value operand is a full zero-extended register
            // spilled to an 8-byte slot, so the 64-bit equality compare is correct
            // (the packed 4-byte form is handled by the memory-read arms below).
            "Boolean" | "Byte" | "Integer" | "Fixed" | "Float" | "Money" | "Scalar" => {
                self.emit(abi::load_u64(lval, abi::stack_pointer(), left_slot));
                self.emit(abi::load_u64(rval, abi::stack_pointer(), right_slot));
                self.emit(abi::compare_registers(lval, rval));
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
            other
                if self
                    .type_model
                    .record_fields
                    .contains_key(&ParameterType::declared(other)) =>
            {
                let fields = self
                    .type_model
                    .record_fields
                    .get(&ParameterType::declared(other))
                    .cloned()
                    .ok_or_else(|| format!("native record type '{other}' does not resolve"))?;
                if fields.is_empty() {
                    self.emit(abi::branch(equal_label));
                    return Ok(());
                }
                let inline_string_field = fields
                    .iter()
                    .map(|(_, ft)| self.record_field_is_inlined(other, &ft.name()))
                    .collect::<Vec<_>>();
                for (index, (_, field_type)) in fields.iter().enumerate() {
                    let next_field = self.label("compare_record_next_field");
                    let field_left_slot = self.allocate_stack_object("compare_record_left", 8);
                    let field_right_slot = self.allocate_stack_object("compare_record_right", 8);
                    self.emit(abi::load_u64(lcur, abi::stack_pointer(), left_slot));
                    self.emit(abi::load_u64(rcur, abi::stack_pointer(), right_slot));
                    if inline_string_field[index] {
                        // The slot is a block-relative offset; recover the String
                        // alias pointer (record base + offset) before comparing.
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
                        &field_type.name(),
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
                    .any(|(enum_type, _)| enum_type.name() == other) =>
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

    #[allow(clippy::too_many_arguments)]
    /// `stride_type` selects the data-base entry stride: the element type for a
    /// LIST block, or `""` for a MAP block, which keeps its lookup table
    /// whatever its key and value types are. Passing `type_` here for a map
    /// would address a `Map OF Scalar TO T` past its own entry array
    /// (plan-57-D).
    pub(crate) fn emit_collection_payload_match_branch(
        &mut self,
        type_: &str,
        stride_type: &str,
        collection: impl Into<Operand>,
        offset: impl Into<Operand>,
        length: impl Into<Operand>,
        value: impl Into<Operand>,
        equal_label: &str,
        not_equal_label: &str,
    ) -> Result<(), String> {
        let length = length.into();
        let value = value.into();
        let data = self.allocate_register();
        self.emit_collection_data_pointer_for(&data, collection, stride_type);
        self.emit(abi::add_registers(&data, &data, offset));
        match type_ {
            "Boolean" | "Byte" => {
                let candidate = self.allocate_register();
                self.emit(abi::load_u8(&candidate, &data, 0));
                self.emit(abi::compare_registers(&candidate, value.clone()));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            "Scalar" => {
                let candidate = self.allocate_register();
                self.emit(abi::load_u32(&candidate, &data, 0));
                self.emit(abi::compare_registers(&candidate, value.clone()));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            "Integer" | "Float" | "Fixed" | "Money" => {
                let candidate = self.allocate_register();
                self.emit(abi::load_u64(&candidate, &data, 0));
                self.emit(abi::compare_registers(&candidate, value.clone()));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            "String" => {
                let value_len = self.allocate_register();
                let value_cursor = self.allocate_register();
                let remaining = self.allocate_register();
                let packed_byte = self.allocate_register();
                let value_byte = self.allocate_register();
                let loop_label = self.label("collection_string_match_loop");
                self.emit(abi::load_u64(&value_len, value.clone(), 0));
                self.emit(abi::compare_registers(length.clone(), &value_len));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit(abi::add_immediate(&value_cursor, value.clone(), 8));
                self.emit(abi::move_register(&remaining, length.clone()));
                self.emit_byte_compare_loop(
                    &data,
                    &value_cursor,
                    &remaining,
                    &packed_byte,
                    &value_byte,
                    &loop_label,
                    equal_label,
                    not_equal_label,
                );
            }
            other if self.is_pointer_collection_payload_type(other) => {
                let candidate = self.allocate_register();
                self.emit(abi::load_u64(&candidate, &data, 0));
                self.emit(abi::compare_registers(&candidate, value.clone()));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            other
                if self
                    .type_model
                    .record_fields
                    .contains_key(&ParameterType::declared(other)) =>
            {
                self.emit_comparable_values_match_branch(
                    other,
                    &data,
                    value.clone(),
                    equal_label,
                    not_equal_label,
                )?;
            }
            other if self.inline_collection_payload_size(other).is_some() => {
                self.emit_compare_bytes_branch(
                    &data,
                    value.clone(),
                    length.clone(),
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
        type_: &str,
        stride_type: &str,
        collection: impl Into<Operand>,
        offset: impl Into<Operand>,
        length: impl Into<Operand>,
        value: impl Into<Operand>,
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
            "Boolean" | "Byte" => {
                self.emit(abi::load_u8(cval, cur, 0));
                self.emit(abi::compare_registers(cval, value.clone()));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            "Scalar" => {
                self.emit(abi::load_u32(cval, cur, 0));
                self.emit(abi::compare_registers(cval, value.clone()));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            "Integer" | "Float" | "Fixed" | "Money" => {
                self.emit(abi::load_u64(cval, cur, 0));
                self.emit(abi::compare_registers(cval, value.clone()));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            "String" => {
                let loop_label = self.label("collection_string_value_match_loop");
                self.emit(abi::load_u64(tmp, value.clone(), 0));
                self.emit(abi::compare_registers(length.clone(), tmp));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit(abi::add_immediate(vcur, value.clone(), 8));
                self.emit(abi::move_register(rem, length.clone()));
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
            other if self.is_pointer_collection_payload_type(other) => {
                self.emit(abi::load_u64(cval, cur, 0));
                self.emit(abi::compare_registers(cval, value.clone()));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            other
                if self
                    .type_model
                    .record_fields
                    .contains_key(&ParameterType::declared(other)) =>
            {
                self.emit_comparable_values_match_branch(
                    other,
                    cur,
                    value.clone(),
                    equal_label,
                    not_equal_label,
                )?;
            }
            other if self.inline_collection_payload_size(other).is_some() => {
                self.emit_compare_bytes_branch(
                    cur,
                    value.clone(),
                    length.clone(),
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
        type_: &str,
        stride_type: &str,
        left_collection: impl Into<Operand>,
        left_offset: impl Into<Operand>,
        left_length: impl Into<Operand>,
        right_collection: impl Into<Operand>,
        right_offset: impl Into<Operand>,
        right_length: impl Into<Operand>,
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
            "Boolean" | "Byte" => {
                self.emit(abi::load_u8(lval, lcur, 0));
                self.emit(abi::load_u8(rval, rcur, 0));
                self.emit(abi::compare_registers(lval, rval));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            "Scalar" => {
                self.emit(abi::load_u32(lval, lcur, 0));
                self.emit(abi::load_u32(rval, rcur, 0));
                self.emit(abi::compare_registers(lval, rval));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            "Integer" | "Float" | "Fixed" | "Money" => {
                self.emit(abi::load_u64(lval, lcur, 0));
                self.emit(abi::load_u64(rval, rcur, 0));
                self.emit(abi::compare_registers(lval, rval));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            "String" => {
                let loop_label = self.label("collection_payload_string_match_loop");
                self.emit(abi::compare_registers(
                    left_length.clone(),
                    right_length.clone(),
                ));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit(abi::move_register(roff, left_length.clone()));
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
            other
                if self
                    .type_model
                    .record_fields
                    .contains_key(&ParameterType::declared(other)) =>
            {
                self.emit(abi::compare_registers(
                    left_length.clone(),
                    right_length.clone(),
                ));
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
                self.emit(abi::compare_registers(
                    left_length.clone(),
                    right_length.clone(),
                ));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit_compare_bytes_branch(
                    lcur,
                    rcur,
                    left_length.clone(),
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
