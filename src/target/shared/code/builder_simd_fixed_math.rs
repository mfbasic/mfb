use super::*;

// Vectorized / per-lane Fixed (Q32.32) transcendentals — plan-01-simd Phase 4.
//
// `sqrt(Fixed[])` is genuine 2-lane NEON: the scalar digit-by-digit restoring
// square root (`emit_fixed_sqrt`) uses only shift/or/add/sub/compare, all of
// which exist on `.2d`, and its per-lane conditional subtraction becomes a
// branchless `cmge`+`and`+`sub`/`add` select. Every working value stays well
// under 2^63, so the signed `cmge` lane compare matches the scalar's unsigned
// `b.lo` test. The result is therefore bit-identical to the scalar Fixed sqrt.
//
// `log(Fixed[])` / `log10(Fixed[])` cannot be true `.2d` SIMD — the Q32.32 log
// needs a 64x64->128 multiply-high and a 64-bit integer divide, neither of which
// NEON provides on `.2d`. They are instead lowered as a per-lane loop over the
// existing scalar `emit_fixed_log`, which is exact and bit-identical (decided
// with the user; see [[plan-01-simd-progress]]).

impl CodeBuilder<'_> {
    /// `math.sqrt(values AS Fixed[]) AS Fixed[]` — 2-lane NEON restoring sqrt of
    /// the raw Q32.32 lanes. `ErrInvalidArgument` if any lane is negative.
    pub(super) fn lower_simd_sqrt_fixed(
        &mut self,
        input: ValueResult,
        text: String,
    ) -> Result<ValueResult, String> {
        self.reset_temporary_registers();
        let in_ptr = self.allocate_register()?;
        self.emit(abi::move_register(&in_ptr, &input.location));
        let count = self.allocate_register()?;
        self.emit(abi::load_u64(&count, &in_ptr, COLLECTION_OFFSET_COUNT));
        let in_slot = self.allocate_stack_object("simd_fxsqrt_in", 8);
        let count_slot = self.allocate_stack_object("simd_fxsqrt_count", 8);
        self.emit(abi::store_u64(&in_ptr, abi::stack_pointer(), in_slot));
        self.emit(abi::store_u64(&count, abi::stack_pointer(), count_slot));

        self.emit(abi::move_register("x0", &count));
        self.emit(abi::move_immediate(
            "x1",
            "Integer",
            &COLLECTION_TYPE_FIXED.to_string(),
        ));
        self.emit(abi::branch_link(SIMD_ALLOC_LIST_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: SIMD_ALLOC_LIST_SYMBOL.to_string(),
            kind: "branch26".to_string(),
            binding: "internal".to_string(),
            library: None,
        });

        self.reset_temporary_registers();
        let result_base = self.allocate_register()?;
        self.emit(abi::move_register(&result_base, "x0"));
        let alloc_ok = self.label("simd_fxsqrt_alloc_ok");
        self.emit(abi::compare_immediate("x1", "0"));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit(abi::move_register("x0", "x1"));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&alloc_ok));

        let in_ptr = self.allocate_register()?;
        self.emit(abi::load_u64(&in_ptr, abi::stack_pointer(), in_slot));
        let count = self.allocate_register()?;
        self.emit(abi::load_u64(&count, abi::stack_pointer(), count_slot));
        let in_data = self.allocate_register()?;
        self.emit_collection_data_pointer(&in_data, &in_ptr);
        let out_data = self.allocate_register()?;
        self.emit_collection_data_pointer(&out_data, &result_base);
        let pairs = self.allocate_register()?;
        self.emit(abi::shift_right_immediate(&pairs, &count, 1));

        // Persistent vector state: v17 = broadcast(1), v18 = negative-lane mask.
        self.emit(abi::move_immediate("x0", "Integer", "1"));
        self.emit(abi::vector_dup_from_x("v17", "x0"));
        self.emit(abi::vector_eor("v18", "v18", "v18"));

        // --- 2-lane chunk loop ---
        let loop_label = self.label("simd_fxsqrt_loop");
        let loop_done = self.label("simd_fxsqrt_done");
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate(&pairs, "0"));
        self.emit(abi::branch_eq(&loop_done));
        self.emit(abi::vector_load("v0", &in_data, 0));
        self.emit_fixed_sqrt_vector();
        self.emit(abi::vector_store("v3", &out_data, 0));
        self.emit(abi::add_immediate(&in_data, &in_data, 16));
        self.emit(abi::add_immediate(&out_data, &out_data, 16));
        self.emit(abi::subtract_immediate(&pairs, &pairs, 1));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&loop_done));

        // --- Scalar tail (count & 1): broadcast the single element into both
        // lanes, run the same kernel, store lane 0. ---
        self.emit(abi::move_immediate("x1", "Integer", "1"));
        self.emit(abi::and_registers("x1", &count, "x1"));
        let tail_done = self.label("simd_fxsqrt_tail_done");
        self.emit(abi::compare_immediate("x1", "0"));
        self.emit(abi::branch_eq(&tail_done));
        self.emit(abi::load_u64("x0", &in_data, 0));
        self.emit(abi::vector_dup_from_x("v0", "x0"));
        self.emit_fixed_sqrt_vector();
        self.emit(abi::vector_extract_to_x("x0", "v3", 0));
        self.emit(abi::store_u64("x0", &out_data, 0));
        self.emit(abi::label(&tail_done));

        // --- Error reduce: any negative lane → ErrInvalidArgument ---
        self.emit(abi::vector_extract_to_x("x0", "v18", 0));
        self.emit(abi::vector_extract_to_x("x1", "v18", 1));
        self.emit(abi::or_registers("x0", "x0", "x1"));
        let no_err = self.label("simd_fxsqrt_no_err");
        self.emit(abi::compare_immediate("x0", "0"));
        self.emit(abi::branch_eq(&no_err));
        self.emit_invalid_argument_return()?;
        self.emit(abi::label(&no_err));

        Ok(ValueResult {
            type_: "List OF Fixed".to_string(),
            location: result_base,
            text,
        })
    }

    /// Emit the 2-lane restoring-sqrt kernel: input raw Q32.32 lanes in `v0`,
    /// result in `v3`; negative lanes OR-accumulated into `v18`. Assumes
    /// `v17 = broadcast(1)`. Uses `x0` as the (shared) digit counter and vector
    /// scratch `v1..v7,v16,v19` — all caller-saved, like the scalar kernel's
    /// register-tight body. Mirrors `emit_fixed_sqrt` op-for-op so each lane is
    /// bit-identical to the scalar result.
    fn emit_fixed_sqrt_vector(&mut self) {
        // Negative-lane detection (raw < 0): arithmetic shift fills all-ones.
        self.emit(abi::vector_sshr("v16", "v0", 63));
        self.emit(abi::vector_orr("v18", "v18", "v16"));

        // nhi=v1=src, nlo=v2=0, res=v3=0, rem=v4=0.
        self.emit(abi::vector_orr("v1", "v0", "v0")); // nhi = src
        self.emit(abi::vector_eor("v2", "v2", "v2")); // nlo = 0
        self.emit(abi::vector_eor("v3", "v3", "v3")); // res = 0
        self.emit(abi::vector_eor("v4", "v4", "v4")); // rem = 0
        self.emit(abi::move_immediate("x0", "Integer", "48"));

        let loop_label = self.label("simd_fxsqrt_digit");
        let loop_done = self.label("simd_fxsqrt_digit_done");
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate("x0", "0"));
        self.emit(abi::branch_eq(&loop_done));
        // digit = nhi >> 62 (logical, top two bits).
        self.emit(abi::vector_ushr("v5", "v1", 62));
        // 128-bit radicand <<= 2: nhi = (nhi<<2)|(nlo>>62); nlo <<= 2.
        self.emit(abi::vector_shl("v1", "v1", 2));
        self.emit(abi::vector_ushr("v6", "v2", 62));
        self.emit(abi::vector_orr("v1", "v1", "v6"));
        self.emit(abi::vector_shl("v2", "v2", 2));
        // rem = rem*4 + digit; res *= 2.
        self.emit(abi::vector_shl("v4", "v4", 2));
        self.emit(abi::vector_orr("v4", "v4", "v5"));
        self.emit(abi::vector_shl("v3", "v3", 1));
        // trial = 2*res + 1.
        self.emit(abi::vector_shl("v7", "v3", 1));
        self.emit(abi::vector_add("v7", "v7", "v17"));
        // Per-lane: if rem >= trial { rem -= trial; res += 1 }.
        self.emit(abi::vector_cmge("v16", "v4", "v7")); // mask
        self.emit(abi::vector_and("v19", "v7", "v16"));
        self.emit(abi::vector_sub("v4", "v4", "v19"));
        self.emit(abi::vector_and("v19", "v17", "v16"));
        self.emit(abi::vector_add("v3", "v3", "v19"));
        self.emit(abi::subtract_immediate("x0", "x0", 1));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&loop_done));

        // Round to nearest: if rem > res { res += 1 }.
        self.emit(abi::vector_cmgt("v16", "v4", "v3"));
        self.emit(abi::vector_and("v19", "v17", "v16"));
        self.emit(abi::vector_add("v3", "v3", "v19"));
    }

    /// `math.log/log10(values AS Fixed[]) AS Fixed[]` — per-lane loop over the
    /// scalar Q32.32 `emit_fixed_log` (NEON has no `.2d` 64-bit mul-high/divide,
    /// so this is correct-and-bit-identical rather than parallel). Non-positive
    /// lanes raise `ErrInvalidArgument` (the scalar kernel's terminal check).
    pub(super) fn lower_simd_log_fixed(
        &mut self,
        input: ValueResult,
        base10: bool,
        text: String,
    ) -> Result<ValueResult, String> {
        self.reset_temporary_registers();
        let in_ptr = self.allocate_register()?;
        self.emit(abi::move_register(&in_ptr, &input.location));
        let count = self.allocate_register()?;
        self.emit(abi::load_u64(&count, &in_ptr, COLLECTION_OFFSET_COUNT));
        let in_slot = self.allocate_stack_object("simd_fxlog_in", 8);
        let count_slot = self.allocate_stack_object("simd_fxlog_count", 8);
        self.emit(abi::store_u64(&in_ptr, abi::stack_pointer(), in_slot));
        self.emit(abi::store_u64(&count, abi::stack_pointer(), count_slot));

        self.emit(abi::move_register("x0", &count));
        self.emit(abi::move_immediate(
            "x1",
            "Integer",
            &COLLECTION_TYPE_FIXED.to_string(),
        ));
        self.emit(abi::branch_link(SIMD_ALLOC_LIST_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: SIMD_ALLOC_LIST_SYMBOL.to_string(),
            kind: "branch26".to_string(),
            binding: "internal".to_string(),
            library: None,
        });

        self.reset_temporary_registers();
        let result_base = self.allocate_register()?;
        self.emit(abi::move_register(&result_base, "x0"));
        let alloc_ok = self.label("simd_fxlog_alloc_ok");
        self.emit(abi::compare_immediate("x1", "0"));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit(abi::move_register("x0", "x1"));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&alloc_ok));

        // Persistent loop state lives in stack slots: the scalar emit_fixed_log
        // resets the register file each iteration, so nothing survives in
        // registers across it — including `result_base`, which would otherwise be
        // clobbered by the loop's reused `idx` register.
        let base_slot = self.allocate_stack_object("simd_fxlog_base", 8);
        self.emit(abi::store_u64(&result_base, abi::stack_pointer(), base_slot));
        let in_data_slot = self.allocate_stack_object("simd_fxlog_indata", 8);
        let out_data_slot = self.allocate_stack_object("simd_fxlog_outdata", 8);
        let idx_slot = self.allocate_stack_object("simd_fxlog_idx", 8);
        let in_ptr = self.allocate_register()?;
        self.emit(abi::load_u64(&in_ptr, abi::stack_pointer(), in_slot));
        let in_data = self.allocate_register()?;
        self.emit_collection_data_pointer(&in_data, &in_ptr);
        self.emit(abi::store_u64(&in_data, abi::stack_pointer(), in_data_slot));
        let out_data = self.allocate_register()?;
        self.emit_collection_data_pointer(&out_data, &result_base);
        self.emit(abi::store_u64(&out_data, abi::stack_pointer(), out_data_slot));
        self.emit(abi::move_immediate("x0", "Integer", "0"));
        self.emit(abi::store_u64("x0", abi::stack_pointer(), idx_slot));

        let loop_label = self.label("simd_fxlog_loop");
        let loop_done = self.label("simd_fxlog_loop_done");
        self.emit(abi::label(&loop_label));
        // Reload idx and count; exit when idx == count.
        self.reset_temporary_registers();
        let idx = self.allocate_register()?;
        let count = self.allocate_register()?;
        self.emit(abi::load_u64(&idx, abi::stack_pointer(), idx_slot));
        self.emit(abi::load_u64(&count, abi::stack_pointer(), count_slot));
        self.emit(abi::compare_registers(&idx, &count));
        self.emit(abi::branch_ge(&loop_done));
        // addr = in_data + idx*8; load the element.
        let in_data = self.allocate_register()?;
        self.emit(abi::load_u64(&in_data, abi::stack_pointer(), in_data_slot));
        let offset = self.allocate_register()?;
        self.emit(abi::shift_left_immediate(&offset, &idx, 3));
        let addr = self.allocate_register()?;
        self.emit(abi::add_registers(&addr, &in_data, &offset));
        let element = self.allocate_register()?;
        self.emit(abi::load_u64(&element, &addr, 0));
        // result = scalar Fixed log (terminal ErrInvalidArgument on element <= 0).
        let result = self.emit_fixed_log(&element, base10)?;
        // Store result at out_data + idx*8 (reload — emit_fixed_log reset regs).
        let result_slot = self.allocate_stack_object("simd_fxlog_result", 8);
        self.emit(abi::store_u64(&result, abi::stack_pointer(), result_slot));
        self.reset_temporary_registers();
        let idx = self.allocate_register()?;
        let out_data = self.allocate_register()?;
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&idx, abi::stack_pointer(), idx_slot));
        self.emit(abi::load_u64(&out_data, abi::stack_pointer(), out_data_slot));
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        let offset = self.allocate_register()?;
        self.emit(abi::shift_left_immediate(&offset, &idx, 3));
        let addr = self.allocate_register()?;
        self.emit(abi::add_registers(&addr, &out_data, &offset));
        self.emit(abi::store_u64(&result, &addr, 0));
        // idx++ and loop.
        self.emit(abi::add_immediate(&idx, &idx, 1));
        self.emit(abi::store_u64(&idx, abi::stack_pointer(), idx_slot));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&loop_done));

        // Reload the list base for the result (it did not survive the loop's
        // register resets).
        self.reset_temporary_registers();
        let result_base = self.allocate_register()?;
        self.emit(abi::load_u64(&result_base, abi::stack_pointer(), base_slot));
        Ok(ValueResult {
            type_: "List OF Fixed".to_string(),
            location: result_base,
            text,
        })
    }
}
