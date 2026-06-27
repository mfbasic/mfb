use super::*;

/// Signed 64-bit minimum, written as its unsigned bit pattern (`abs`/`neg`
/// overflow sentinel for Integer and Fixed lanes).
const INT64_MIN_UNSIGNED: &str = "9223372036854775808";

/// A unary `math::` array kernel: how to transform one input list of 8-byte
/// numeric lanes into a result list, expressed once as a NEON `.2d` sequence for
/// the two-lane chunk loop and once as a scalar sequence for the odd tail. The
/// two forms compute identical per-lane results so the tail matches a vector
/// lane (plan-01-simd §4.3, Open Decision #6).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SimdUnaryKernel {
    /// `Integer[] → Integer[]` absolute value; `ErrOverflow` on an `INT64_MIN`
    /// lane (whose magnitude is not representable).
    AbsInteger,
}

impl SimdUnaryKernel {
    /// Whether this kernel can raise an error, and which one. `None` means the
    /// kernel never sets the per-lane error mask.
    fn error(self) -> Option<SimdError> {
        match self {
            SimdUnaryKernel::AbsInteger => Some(SimdError::Overflow),
        }
    }
}

/// Which error a kernel raises when its reduced per-lane mask is nonzero.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SimdError {
    Overflow,
    #[allow(dead_code)]
    InvalidArgument,
}

impl CodeBuilder<'_> {
    /// Lower a unary `math::` array overload. Reads the input list's `count`,
    /// allocates a tight result list via `_mfb_simd_alloc_list`, streams the data
    /// region two lanes at a time with the kernel's NEON sequence, processes the
    /// odd tail with the scalar sequence, reduces any per-lane error mask to a
    /// single error, and returns the new list.
    ///
    /// `input` must already be lowered (its `location` holds the list pointer).
    /// All loop state is allocated through `allocate_register` (which skips the
    /// reserved `x18`/`x19` and records callee-saved use) and the loop runs
    /// entirely after the alloc call, so no live value crosses
    /// `bl _mfb_simd_alloc_list` ([[arena-alloc-clobbers-x14-x15]]).
    pub(super) fn lower_simd_unary(
        &mut self,
        kernel: SimdUnaryKernel,
        input: ValueResult,
        result_type: &str,
        result_type_code: usize,
        text: String,
    ) -> Result<ValueResult, String> {
        self.reset_temporary_registers();
        // Spill the input pointer and count across the alloc call.
        let in_ptr = self.allocate_register()?;
        self.emit(abi::move_register(&in_ptr, &input.location));
        let count = self.allocate_register()?;
        self.emit(abi::load_u64(&count, &in_ptr, COLLECTION_OFFSET_COUNT));
        let in_slot = self.allocate_stack_object("simd_in_ptr", 8);
        let count_slot = self.allocate_stack_object("simd_count", 8);
        self.emit(abi::store_u64(&in_ptr, abi::stack_pointer(), in_slot));
        self.emit(abi::store_u64(&count, abi::stack_pointer(), count_slot));

        // base = _mfb_simd_alloc_list(count, typeCode) → x0 = base, x1 = status.
        self.emit(abi::move_register("x0", &count));
        self.emit(abi::move_immediate(
            "x1",
            "Integer",
            &result_type_code.to_string(),
        ));
        self.emit(abi::branch_link(SIMD_ALLOC_LIST_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: SIMD_ALLOC_LIST_SYMBOL.to_string(),
            kind: "branch26".to_string(),
            binding: "internal".to_string(),
            library: None,
        });

        // Everything below runs after the only call, so registers are free.
        self.reset_temporary_registers();
        let result_base = self.allocate_register()?;
        self.emit(abi::move_register(&result_base, "x0"));
        let alloc_ok = self.label("simd_alloc_ok");
        self.emit(abi::compare_immediate("x1", "0"));
        self.emit(abi::branch_eq(&alloc_ok));
        // Surface the arena tag (returned in x1) as the allocation error.
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
        let err = self.allocate_register()?;
        self.emit(abi::move_immediate(&err, "Integer", "0"));

        // Error mask accumulator v7 = 0 (always, so the reduce is valid even when
        // the loop body never runs).
        self.emit(abi::vector_eor("v7", "v7", "v7"));
        self.emit_simd_unary_setup(kernel)?;

        // --- 2-lane chunk loop ---
        let loop_label = self.label("simd_chunk_loop");
        let loop_done = self.label("simd_chunk_done");
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate(&pairs, "0"));
        self.emit(abi::branch_eq(&loop_done));
        self.emit(abi::vector_load("v0", &in_data, 0));
        self.emit_simd_unary_vector(kernel)?;
        self.emit(abi::vector_store("v0", &out_data, 0));
        self.emit(abi::add_immediate(&in_data, &in_data, 16));
        self.emit(abi::add_immediate(&out_data, &out_data, 16));
        self.emit(abi::subtract_immediate(&pairs, &pairs, 1));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&loop_done));

        // --- Scalar tail (count & 1) ---
        let one = self.allocate_register()?;
        self.emit(abi::move_immediate(&one, "Integer", "1"));
        let tail = self.allocate_register()?;
        self.emit(abi::and_registers(&tail, &count, &one));
        let tail_done = self.label("simd_tail_done");
        self.emit(abi::compare_immediate(&tail, "0"));
        self.emit(abi::branch_eq(&tail_done));
        self.emit_simd_unary_scalar(kernel, &in_data, &out_data, &err)?;
        self.emit(abi::label(&tail_done));

        // --- Error reduce ---
        if kernel.error().is_some() {
            let lo = self.allocate_register()?;
            let hi = self.allocate_register()?;
            self.emit(abi::vector_extract_to_x(&lo, "v7", 0));
            self.emit(abi::vector_extract_to_x(&hi, "v7", 1));
            self.emit(abi::or_registers(&lo, &lo, &hi));
            self.emit(abi::or_registers(&err, &err, &lo));
            let no_err = self.label("simd_no_err");
            self.emit(abi::compare_immediate(&err, "0"));
            self.emit(abi::branch_eq(&no_err));
            match kernel.error().unwrap() {
                SimdError::Overflow => self.emit_overflow_return()?,
                SimdError::InvalidArgument => self.emit_invalid_argument_return()?,
            }
            self.emit(abi::label(&no_err));
        }

        Ok(ValueResult {
            type_: result_type.to_string(),
            location: result_base,
            text,
        })
    }

    /// Emit any one-time setup the vector loop needs (e.g. broadcasting a
    /// constant into a fixed vector register). Uses `v6` for kernel constants.
    fn emit_simd_unary_setup(&mut self, kernel: SimdUnaryKernel) -> Result<(), String> {
        match kernel {
            SimdUnaryKernel::AbsInteger => {
                // v6 = broadcast(INT64_MIN) for the per-lane overflow compare.
                let min = self.allocate_register()?;
                self.emit(abi::move_immediate(&min, "Integer", INT64_MIN_UNSIGNED));
                self.emit(abi::vector_dup_from_x("v6", &min));
            }
        }
        Ok(())
    }

    /// Emit the per-chunk NEON kernel: input lanes arrive in `v0`, the result is
    /// left in `v0`, and any failing lanes are OR-accumulated into the `v7` mask.
    fn emit_simd_unary_vector(&mut self, kernel: SimdUnaryKernel) -> Result<(), String> {
        match kernel {
            SimdUnaryKernel::AbsInteger => {
                // Detect INT64_MIN lanes from the *input* (abs of INT64_MIN wraps
                // back to INT64_MIN, so the check must precede the abs).
                self.emit(abi::vector_cmeq("v1", "v0", "v6"));
                self.emit(abi::vector_orr("v7", "v7", "v1"));
                self.emit(abi::vector_abs("v0", "v0"));
            }
        }
        Ok(())
    }

    /// Emit the scalar tail kernel: read one lane from `[in_data]`, transform it,
    /// store to `[out_data]`, and set `err` to 1 on a failing lane.
    fn emit_simd_unary_scalar(
        &mut self,
        kernel: SimdUnaryKernel,
        in_data: &str,
        out_data: &str,
        err: &str,
    ) -> Result<(), String> {
        match kernel {
            SimdUnaryKernel::AbsInteger => {
                let value = self.allocate_register()?;
                self.emit(abi::load_u64(&value, in_data, 0));
                let min = self.allocate_register()?;
                self.emit(abi::move_immediate(&min, "Integer", INT64_MIN_UNSIGNED));
                let no_of = self.label("simd_tail_no_overflow");
                self.emit(abi::compare_registers(&value, &min));
                self.emit(abi::branch_ne(&no_of));
                self.emit(abi::move_immediate(err, "Integer", "1"));
                self.emit(abi::label(&no_of));
                // abs: negate when negative.
                let negate = self.label("simd_tail_negate");
                let stored = self.label("simd_tail_stored");
                self.emit(abi::compare_immediate(&value, "0"));
                self.emit(abi::branch_lt(&negate));
                self.emit(abi::store_u64(&value, out_data, 0));
                self.emit(abi::branch(&stored));
                self.emit(abi::label(&negate));
                self.emit(abi::subtract_registers(&value, "xzr", &value));
                self.emit(abi::store_u64(&value, out_data, 0));
                self.emit(abi::label(&stored));
            }
        }
        Ok(())
    }
}
