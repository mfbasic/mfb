// --- codegen tier imports (migration) ---
use crate::codegen::builtins;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::typed_map_entry_type_parts;
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use crate::operators::{BinaryOp, UnaryOp};
use crate::target::shared::abi;
use crate::target::shared::nir::*;
use crate::types::ParameterType;

mod default_value;
mod field_access;
mod static_eval;
impl CodeBuilder<'_> {
    pub(crate) fn lower_string_concat(
        &mut self,
        left: &NirValue,
        right: &NirValue,
    ) -> Result<ValueResult, String> {
        let left = self.lower_value(left)?;
        if left.type_ != ParameterType::String {
            return Err(format!(
                "native string concat left operand must be String, got {}",
                left.type_
            ));
        }
        let left_slot = self.allocate_stack_object("concat_left", 8);
        self.emit(abi::store_u64(
            &left.location,
            abi::stack_pointer(),
            left_slot,
        ));
        let right = self.lower_value(right)?;
        if right.type_ != ParameterType::String {
            return Err(format!(
                "native string concat right operand must be String, got {}",
                right.type_
            ));
        }
        let right_slot = self.allocate_stack_object("concat_right", 8);
        self.emit(abi::store_u64(
            &right.location,
            abi::stack_pointer(),
            right_slot,
        ));

        // plan-118-C: the allocation, the header store and both copy loops are
        // in `_mfb_rt_string_concat`, emitted once per module. This was 169
        // instructions per site over 17,221 sites — the largest single expansion
        // category in the compiler. Both operands are reloaded from their slots
        // rather than used from their registers: the call clobbers the
        // caller-saved set, and the slots are already there because the right
        // operand's lowering could clobber the left's register anyway.
        let alloc_ok = self.label("string_concat_alloc_ok");
        self.emit(abi::load_u64(
            abi::c_arg(0),
            abi::stack_pointer(),
            left_slot,
        ));
        self.emit(abi::load_u64(
            abi::c_arg(1),
            abi::stack_pointer(),
            right_slot,
        ));
        self.emit(abi::branch_link(STRING_CONCAT_SYMBOL));
        self.push_internal_call_relocation(STRING_CONCAT_SYMBOL);
        // Capture the result before raising: `raise_error_bare` emits a call of
        // its own, and the physical return register does not survive it.
        let result_ptr = self.allocate_register();
        self.emit(abi::move_register(&result_ptr, abi::mfb_return(0)));
        self.emit(abi::compare_immediate(&result_ptr, "0"));
        self.emit(abi::branch_ne(&alloc_ok));
        // Null means the arena allocation failed. The error is raised HERE, not
        // in the helper, so its `ErrorLoc` names the concatenation the program
        // actually wrote.
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&alloc_ok));

        // bug-536 shape B: `_mfb_rt_string_concat` arena-allocates the result and
        // copies both operands' bytes into it (`lower_string_concat_helper`), so
        // the block is fresh and unaliased — neither operand's storage is handed
        // back. An unbound `a & b` may therefore be freed at statement end.
        self.mark_fresh_string(Operand::from(result_ptr));
        Ok(ValueResult {
            origin: None,
            type_: ParameterType::String,
            location: Operand::from(result_ptr),
            text: format!("({} & {})", left.text, right.text),
        })
    }

    /// String concat / rope fusion — a Level-3 catalog row
    /// (`planning/optimizations.md`): lower a whole `a & b & c & …` chain into
    /// **one** pre-sized allocation and one pass of writes, instead of an
    /// intermediate String per operator.
    ///
    /// `&` is left-associative, so `a & b & c` parses as `(a & b) & c` and the
    /// pairwise lowering allocates and fills a whole intermediate for `a & b`
    /// that is then copied again into the final result and abandoned. For a
    /// chain of `n` operands that is `n - 1` allocations and quadratic copying;
    /// this is `1` allocation and one copy of each byte.
    ///
    /// **What is preserved.** Operands are lowered left to right, exactly as
    /// the nested form evaluates them, so a failing operand fails at the same
    /// point with the same earlier operands already evaluated. The only
    /// difference is that the intermediate allocations never happen — and an
    /// arena block nothing can observe is not observable. The pairwise
    /// [`Self::lower_string_concat`] still handles the two-operand case
    /// verbatim, so nothing about ordinary `a & b` changes.
    pub(crate) fn lower_string_concat_chain(
        &mut self,
        parts: &[&NirValue],
    ) -> Result<ValueResult, String> {
        // 1. Every operand, left to right, parked in its own slot. Parking is
        //    what lets the length pass and the copy pass both revisit them
        //    without holding n registers live across the allocation call.
        let mut slots = Vec::with_capacity(parts.len());
        let mut texts = Vec::with_capacity(parts.len());
        for part in parts {
            let value = self.lower_value(part)?;
            if value.type_ != ParameterType::String {
                return Err(format!(
                    "native string concat operand must be String, got {}",
                    value.type_
                ));
            }
            let slot = self.allocate_stack_object("concat_part", 8);
            self.emit(abi::store_u64(&value.location, abi::stack_pointer(), slot));
            slots.push(slot);
            texts.push(value.text);
        }

        let total_slot = self.allocate_stack_object("concat_total", 8);
        let total_len_v = self.temporary_vreg();
        let part_ptr_v = self.temporary_vreg();
        let part_len_v = self.temporary_vreg();
        let write_cur_v = self.temporary_vreg();
        let read_cur_v = self.temporary_vreg();
        let remaining_v = self.temporary_vreg();
        let byte_v = self.temporary_vreg();
        let total_len = &total_len_v;
        let part_ptr = &part_ptr_v;
        let part_len = &part_len_v;
        let write_cur = &write_cur_v;
        let read_cur = &read_cur_v;
        let remaining = &remaining_v;
        let byte = &byte_v;

        // 2. Sum the byte lengths. Every String's length is its header word, so
        //    this is one load per operand — no scanning.
        self.emit(abi::move_immediate(total_len, "Integer", "0"));
        for slot in &slots {
            self.emit(abi::load_u64(part_ptr, abi::stack_pointer(), *slot));
            self.emit(abi::load_u64(part_len, part_ptr, 0));
            self.emit(abi::add_registers(total_len, total_len, part_len));
        }
        self.emit(abi::store_u64(total_len, abi::stack_pointer(), total_slot));

        // 3. One allocation for the whole chain (8-byte header + bytes + NUL).
        let alloc_ok = self.label("string_concat_chain_alloc_ok");
        self.emit(abi::add_immediate(abi::c_arg(0), total_len, 9));
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&alloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&alloc_ok));
        // Carry the result out of the physical return register immediately: the
        // copy loops' back edges break the result-vs-argument dataflow on ISAs
        // whose result and argument registers differ (the same reason the
        // pairwise lowering does this).
        let result_ptr = self.allocate_register();
        self.emit(abi::move_register(&result_ptr, abi::mfb_return(1)));
        self.emit(abi::load_u64(total_len, abi::stack_pointer(), total_slot));
        self.emit(abi::store_u64(total_len, &result_ptr, 0));
        self.emit(abi::add_immediate(write_cur, &result_ptr, 8));

        // 4. Copy each operand's bytes in order into the single buffer.
        for slot in &slots {
            let loop_label = self.label("string_concat_chain_loop");
            let done_label = self.label("string_concat_chain_done");
            self.emit(abi::load_u64(read_cur, abi::stack_pointer(), *slot));
            self.emit(abi::load_u64(remaining, read_cur, 0));
            self.emit(abi::add_immediate(read_cur, read_cur, 8));
            self.emit(abi::label(&loop_label));
            self.emit(abi::compare_immediate(remaining, "0"));
            self.emit(abi::branch_eq(&done_label));
            self.emit(abi::load_u8(byte, read_cur, 0));
            self.emit(abi::store_u8(byte, write_cur, 0));
            self.emit(abi::add_immediate(read_cur, read_cur, 1));
            self.emit(abi::add_immediate(write_cur, write_cur, 1));
            self.emit(abi::subtract_immediate(remaining, remaining, 1));
            self.emit(abi::branch(&loop_label));
            self.emit(abi::label(&done_label));
        }
        self.emit(abi::move_immediate(byte, "Integer", "0"));
        self.emit(abi::store_u8(byte, write_cur, 0));

        // bug-536 shape B: the one `emit_arena_alloc_call` in step 3 is this
        // block's only allocation and every operand was *copied* into it, so the
        // fused chain's result is as fresh as the pairwise form's.
        self.mark_fresh_string(Operand::from(result_ptr));
        Ok(ValueResult {
            origin: None,
            type_: ParameterType::String,
            location: Operand::from(result_ptr),
            text: format!("({})", texts.join(" & ")),
        })
    }

    pub(crate) fn lower_match_compare(
        &mut self,
        matched: &ValueResult,
        pattern: &NirValue,
        label: &str,
    ) -> Result<(), String> {
        match pattern {
            NirValue::MemberAccess { target, member } => {
                let NirValue::Local(type_name) = target.as_ref() else {
                    return Err("native code enum match pattern must name enum type".to_string());
                };
                let ordinal = self
                    .type_model
                    .enum_members
                    .get(&(ParameterType::declared(type_name), member.clone()))
                    .copied()
                    .ok_or_else(|| {
                        format!("native code enum member '{type_name}.{member}' does not resolve")
                    })?;
                self.emit(abi::compare_immediate(
                    &matched.location,
                    &ordinal.to_string(),
                ));
                self.emit(abi::branch_eq(label));
            }
            NirValue::Local(variant)
                if self
                    .type_model
                    .union_variants
                    .contains_key(&ParameterType::declared(variant)) =>
            {
                let tag = self
                    .type_model
                    .union_variant_tags
                    .get(&ParameterType::declared(variant))
                    .copied()
                    .ok_or_else(|| {
                        format!("native code union variant '{variant}' does not resolve")
                    })?;
                let tag_register = self.allocate_register();
                self.emit(abi::load_u64(&tag_register, &matched.location, 0));
                self.emit(abi::compare_immediate(&tag_register, &tag.to_string()));
                self.emit(abi::branch_eq(label));
            }
            _ => {
                // The pattern's bits in a GPR: the compare below stores them.
                let pattern = self.lower_value(pattern)?;
                let pattern = self.materialize_float(pattern)?;
                // An arm matches exactly when the scrutinee `=` the pattern, so
                // every scrutinee whose `=` is not its register's bits goes through
                // the value-equality emitter: a String (and other block-typed)
                // scrutinee is a block pointer, which `compare_registers` would test
                // for identity (bug-140), and a Float's `-0.0` and `0.0` are
                // different bits that `=` calls equal (bug-845). The other scalars
                // keep the register test.
                if matched.type_ == ParameterType::String
                    || matched.type_ == ParameterType::Float
                    || self.type_model.record_fields.contains_key(&matched.type_)
                {
                    let not_equal = self.label("match_compare_not_equal");
                    self.emit_comparable_values_match_branch(
                        &matched.type_,
                        &matched.location,
                        &pattern.location,
                        label,
                        &not_equal,
                    )?;
                    self.emit(abi::label(&not_equal));
                } else {
                    self.emit(abi::compare_registers(&matched.location, &pattern.location));
                    self.emit(abi::branch_eq(label));
                }
            }
        }
        Ok(())
    }

    /// True when a `Result` payload of `payload_type` is a heap block addressed by
    /// pointer (inlined whole), versus an inline scalar (stored in the 8-byte
    /// payload word). Mirrors the record/collection inline rules (plan-02 §4.3).
    pub(crate) fn result_payload_is_block(&self, payload_type: &ParameterType) -> bool {
        *payload_type == ParameterType::String
            || *payload_type == ParameterType::named("Error")
            || typed_is_collection_type(payload_type)
            || matches!(payload_type, ParameterType::ResultOf(_))
            || self.type_model.record_fields.contains_key(payload_type)
            // A **data** union is inlined whole; a **resource** union is a scalar
            // pointer to its `{tag, ptr}` block, like a concrete resource, so it
            // occupies the 8-byte payload word — not an inlinable block (plan-75
            // gap 4). `union_is_data` base-strips any `STATE T` suffix.
            || self.union_is_data(payload_type)
    }
}
