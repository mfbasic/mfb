use super::simd_kernel_coeffs::EXP_COEFFS;
use super::*;

// NEON f64 polynomial kernels for the Float transcendentals — plan-01-simd
// Phase 5. Hand-written, identical on every target, no external math library.
// Coefficients come from `simd_kernel_coeffs.rs` (Remez minimax, validated <=1
// ULP against the committed macOS-libm reference vectors in
// `tests/_data/math_kernel_ref/`). The odd tail reuses the vector kernel by
// broadcasting the single element into both lanes (Open Decision #6) — no
// separate scalar path, so the tail lane is bit-identical to a body lane.

/// `ln2`, full f64 — the exp range-reduction divisor.
const LN2: f64 = 0.693_147_180_559_945_309_417_232_121_458_18;
/// fdlibm two-part `ln2` so `n*ln2` reconstructs past double precision.
const LN2_HI: f64 = 6.931_471_803_691_238_164_90e-01;
const LN2_LO: f64 = 1.908_214_929_270_587_700_02e-10;

impl CodeBuilder<'_> {
    /// `math.exp(values AS Float[]) AS Float[]` — vectorized `e^x`. Range-reduce
    /// `x = n*ln2 + r`, evaluate `e^r` by the minimax polynomial (Horner via
    /// `fmla`), and scale by `2^n`. `ErrOverflow` when a lane's result exceeds the
    /// finite double range (`n > 1023`).
    pub(super) fn lower_simd_exp_float(
        &mut self,
        input: ValueResult,
        text: String,
    ) -> Result<ValueResult, String> {
        self.reset_temporary_registers();
        let in_ptr = self.allocate_register()?;
        self.emit(abi::move_register(&in_ptr, &input.location));
        let count = self.allocate_register()?;
        self.emit(abi::load_u64(&count, &in_ptr, COLLECTION_OFFSET_COUNT));
        let in_slot = self.allocate_stack_object("simd_exp_in", 8);
        let count_slot = self.allocate_stack_object("simd_exp_count", 8);
        self.emit(abi::store_u64(&in_ptr, abi::stack_pointer(), in_slot));
        self.emit(abi::store_u64(&count, abi::stack_pointer(), count_slot));

        self.emit(abi::move_register("x0", &count));
        self.emit(abi::move_immediate(
            "x1",
            "Integer",
            &COLLECTION_TYPE_FLOAT.to_string(),
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
        let alloc_ok = self.label("simd_exp_alloc_ok");
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

        // Persistent kernel constants: v16=ln2, v17=0.5, v18=ln2hi, v19=ln2lo,
        // v20=1023, v21=0, v23=-1022; v22 = accumulated overflow mask.
        self.broadcast_f64("v16", LN2);
        self.broadcast_f64("v17", 0.5);
        self.broadcast_f64("v18", LN2_HI);
        self.broadcast_f64("v19", LN2_LO);
        self.broadcast_i64("v20", 1023);
        self.emit(abi::vector_eor("v21", "v21", "v21"));
        self.broadcast_i64("v23", -1022);
        self.emit(abi::vector_eor("v22", "v22", "v22"));

        // --- 2-lane chunk loop ---
        let loop_label = self.label("simd_exp_loop");
        let loop_done = self.label("simd_exp_done");
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate(&pairs, "0"));
        self.emit(abi::branch_eq(&loop_done));
        self.emit(abi::vector_load("v0", &in_data, 0));
        self.emit_exp_vector();
        self.emit(abi::vector_store("v0", &out_data, 0));
        self.emit(abi::add_immediate(&in_data, &in_data, 16));
        self.emit(abi::add_immediate(&out_data, &out_data, 16));
        self.emit(abi::subtract_immediate(&pairs, &pairs, 1));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&loop_done));

        // --- Scalar tail (broadcast the single element, run the kernel) ---
        self.emit(abi::move_immediate("x1", "Integer", "1"));
        self.emit(abi::and_registers("x1", &count, "x1"));
        let tail_done = self.label("simd_exp_tail_done");
        self.emit(abi::compare_immediate("x1", "0"));
        self.emit(abi::branch_eq(&tail_done));
        self.emit(abi::load_u64("x0", &in_data, 0));
        self.emit(abi::vector_dup_from_x("v0", "x0"));
        self.emit_exp_vector();
        self.emit(abi::vector_extract_to_x("x0", "v0", 0));
        self.emit(abi::store_u64("x0", &out_data, 0));
        self.emit(abi::label(&tail_done));

        // --- Error reduce: any overflow lane → ErrOverflow ---
        self.emit(abi::vector_extract_to_x("x0", "v22", 0));
        self.emit(abi::vector_extract_to_x("x1", "v22", 1));
        self.emit(abi::or_registers("x0", "x0", "x1"));
        let no_err = self.label("simd_exp_no_err");
        self.emit(abi::compare_immediate("x0", "0"));
        self.emit(abi::branch_eq(&no_err));
        self.emit_overflow_return()?;
        self.emit(abi::label(&no_err));

        Ok(ValueResult {
            type_: "List OF Float".to_string(),
            location: result_base,
            text,
        })
    }

    /// Emit the 2-lane `exp` kernel: input lanes in `v0`, result in `v0`, overflow
    /// lanes OR-accumulated into `v22`. Assumes the persistent constants in
    /// v16-v21,v23. Working scratch: v1-v6.
    fn emit_exp_vector(&mut self) {
        // n = floor(x/ln2 + 0.5).
        self.emit(abi::vector_fdiv("v1", "v0", "v16"));
        self.emit(abi::vector_fadd("v1", "v1", "v17"));
        self.emit(abi::vector_frintm("v1", "v1"));
        // r = x - n*ln2hi - n*ln2lo (fused, Cody-Waite).
        self.emit(abi::vector_orr("v2", "v0", "v0"));
        self.emit(abi::vector_fmls("v2", "v1", "v18"));
        self.emit(abi::vector_fmls("v2", "v1", "v19"));
        // Horner: acc = c[11]; acc = c[i] + acc*r for i = 10..=0.
        self.broadcast_f64("v3", EXP_COEFFS[EXP_COEFFS.len() - 1]);
        for i in (0..EXP_COEFFS.len() - 1).rev() {
            self.broadcast_f64("v4", EXP_COEFFS[i]);
            self.emit(abi::vector_fmla("v4", "v3", "v2"));
            self.emit(abi::vector_orr("v3", "v4", "v4"));
        }
        // n_i = (i64)n; overflow if n_i > 1023, underflow if n_i < -1022.
        self.emit(abi::vector_fcvtzs("v5", "v1"));
        self.emit(abi::vector_cmgt("v6", "v5", "v20"));
        self.emit(abi::vector_orr("v22", "v22", "v6"));
        self.emit(abi::vector_cmgt("v6", "v23", "v5")); // underflow mask (kept)
        // scale = 2^n via the IEEE-754 exponent field: ((n_i+1023) << 52).
        self.emit(abi::vector_add("v5", "v5", "v20"));
        self.emit(abi::vector_shl("v5", "v5", 52));
        self.emit(abi::vector_fmul("v0", "v3", "v5"));
        // Flush underflowing lanes to 0.
        self.emit(abi::vector_bsl("v6", "v21", "v0"));
        self.emit(abi::vector_orr("v0", "v6", "v6"));
    }

    /// Broadcast an `f64` constant's bit pattern into both `.2d` lanes of `vreg`.
    fn broadcast_f64(&mut self, vreg: &str, value: f64) {
        self.emit(abi::move_immediate("x0", "Integer", &value.to_bits().to_string()));
        self.emit(abi::vector_dup_from_x(vreg, "x0"));
    }

    /// Broadcast a signed `i64` constant into both `.2d` lanes of `vreg`.
    fn broadcast_i64(&mut self, vreg: &str, value: i64) {
        self.emit(abi::move_immediate(
            "x0",
            "Integer",
            &(value as u64).to_string(),
        ));
        self.emit(abi::vector_dup_from_x(vreg, "x0"));
    }
}
