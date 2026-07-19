use super::*;

/// Signed 64-bit minimum, written as its unsigned bit pattern (`abs`/`neg`
/// overflow sentinel for Integer and Fixed lanes).
const INT64_MIN_UNSIGNED: &str = "9223372036854775808";
/// `0x7FFF_FFFF_FFFF_FFFF` — clears the IEEE-754 sign bit (scalar Float `abs`).
const FLOAT_ABS_MASK: &str = "9223372036854775807";
/// IEEE-754 maximum biased-exponent field (Inf/NaN) for the float→int range check.
const FLOAT_EXP_INF_NAN: &str = "2047";
/// `2^63` as an `f64` bit pattern (`0x43E0_0000_0000_0000`) — the smallest double
/// that does not fit in a signed 64-bit integer.
const FLOAT_TWO_POW_63_BITS: &str = "4890909195324358656";
/// `-2^63` as an `f64` bit pattern (`0xC3E0_0000_0000_0000`) — exactly `INT64_MIN`,
/// the most negative value that *does* fit.
const FLOAT_NEG_TWO_POW_63_BITS: &str = "14114281232179134464";
/// Q32.32 scale `2^32`, the fixed-point fraction mask `2^32-1`, and half `2^31`.
/// bug-175 H: `FIXED_FRACTION_MASK_STR` (RoundFixed's fraction mask) and
/// `FIXED_ONE_MINUS_1_STR` (CeilFixed's "one ULP below 1.0" bias) intentionally
/// share the value `2^32-1`; the two names are kept for call-site intent.
const FIXED_SHIFT: u8 = 32;
const FIXED_FRACTION_MASK_STR: &str = "4294967295";
const FIXED_ONE_MINUS_1_STR: &str = "4294967295";
const FIXED_HALF_STR: &str = "2147483648";

/// A unary `math::` array kernel: how to transform one input list of 8-byte
/// numeric lanes into a result list, expressed once as a NEON `.2d` sequence for
/// the two-lane chunk loop and once as a scalar sequence for the odd tail. The
/// two forms compute identical per-lane results so the tail matches a vector
/// lane (plan-01-simd §4.3, Open Decision #6).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SimdUnaryKernel {
    /// `Integer[]→Integer[]` / `Fixed[]→Fixed[]` absolute value (both are a raw
    /// i64 `abs`); `ErrOverflow` on an `INT64_MIN` lane (magnitude unrepresentable).
    AbsInteger,
    /// `Float[]→Float[]` absolute value (clear the sign bit); never errors.
    AbsFloat,
    /// `Float[]→Float[]` square root; `ErrInvalidArgument` on a negative lane.
    SqrtFloat,
    /// `Float[]→Integer[]` floor/ceil/round (round = ties away). `ErrOverflow`
    /// when a rounded lane falls outside the signed 64-bit range.
    FloorFloat,
    CeilFloat,
    RoundFloat,
    /// `Fixed[]→Integer[]` floor/ceil/round on the raw Q32.32 lanes. The integer
    /// part always fits in `Integer`, so these never error.
    FloorFixed,
    CeilFixed,
    RoundFixed,
}

impl SimdUnaryKernel {
    /// Whether this kernel can raise an error, and which one. `None` means the
    /// kernel never sets the per-lane error mask.
    fn error(self) -> Option<SimdError> {
        match self {
            SimdUnaryKernel::AbsInteger => Some(SimdError::Overflow),
            SimdUnaryKernel::SqrtFloat => Some(SimdError::FloatDomain),
            SimdUnaryKernel::FloorFloat
            | SimdUnaryKernel::CeilFloat
            | SimdUnaryKernel::RoundFloat => Some(SimdError::Overflow),
            SimdUnaryKernel::AbsFloat
            | SimdUnaryKernel::FloorFixed
            | SimdUnaryKernel::CeilFixed
            | SimdUnaryKernel::RoundFixed => None,
        }
    }

    /// The NEON round-to-integral instruction for the float rounders.
    fn float_round_mnemonic(self) -> Option<&'static str> {
        match self {
            SimdUnaryKernel::FloorFloat => Some("frintm_v"),
            SimdUnaryKernel::CeilFloat => Some("frintp_v"),
            SimdUnaryKernel::RoundFloat => Some("frinta_v"),
            _ => None,
        }
    }
}

/// Which error a kernel raises when its reduced per-lane mask is nonzero.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SimdError {
    Overflow,
    /// Float domain failure (e.g. `sqrt(Float[])` negative lane) — matches the
    /// scalar `math::sqrt(Float)` man page's `ErrFloatDomain`.
    FloatDomain,
}

/// A two-array `math::` kernel (`min`/`max`). NEON has no `smin`/`smax` on `.2d`,
/// so the integer/Fixed forms select with `cmgt`+`bsl`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SimdBinaryKernel {
    MinFloat,
    MaxFloat,
    /// Signed i64 lane min/max — used for both `Integer` and raw Q32.32 `Fixed`.
    MinSigned,
    MaxSigned,
}

/// A clamp kernel: one list lane clamped between two broadcast scalars
/// `max(min(x, high), low)`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SimdClampKernel {
    Float,
    /// Signed i64 — `Integer` and raw Q32.32 `Fixed`.
    Signed,
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
        self.emit(abi::move_register(abi::ARG[0], &count));
        self.emit(abi::move_immediate(
            abi::ARG[1],
            "Integer",
            &result_type_code.to_string(),
        ));
        self.emit(abi::branch_link(SIMD_ALLOC_LIST_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: SIMD_ALLOC_LIST_SYMBOL.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });

        // Everything below runs after the only call, so registers are free.
        self.reset_temporary_registers();
        let result_base = self.allocate_register()?;
        self.emit(abi::move_register(&result_base, abi::return_register()));
        let alloc_ok = self.label("simd_alloc_ok");
        self.emit(abi::compare_immediate(abi::RET[1], "0"));
        self.emit(abi::branch_eq(&alloc_ok));
        // Surface the arena tag (returned in x1) as the allocation error.
        self.emit(abi::move_register(abi::return_register(), abi::RET[1]));
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
        self.emit(abi::vector_eor(
            abi::VEC_SCRATCH[7],
            abi::VEC_SCRATCH[7],
            abi::VEC_SCRATCH[7],
        ));
        self.emit_simd_unary_setup(kernel)?;

        // --- 2-lane chunk loop ---
        let loop_label = self.label("simd_chunk_loop");
        let loop_done = self.label("simd_chunk_done");
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate(&pairs, "0"));
        self.emit(abi::branch_eq(&loop_done));
        self.emit(abi::vector_load(abi::VEC_SCRATCH[0], &in_data, 0));
        self.emit_simd_unary_vector(kernel)?;
        self.emit(abi::vector_store(abi::VEC_SCRATCH[0], &out_data, 0));
        self.emit(abi::add_immediate(&in_data, &in_data, 16));
        self.emit(abi::add_immediate(&out_data, &out_data, 16));
        self.emit(abi::subtract_immediate(&pairs, &pairs, 1));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&loop_done));

        // --- Scalar tail (count & 1) ---  (allocator-placed scratch)
        let tail_bit = self.allocate_register()?;
        self.emit(abi::move_immediate(&tail_bit, "Integer", "1"));
        self.emit(abi::and_registers(&tail_bit, &count, &tail_bit));
        let tail_done = self.label("simd_tail_done");
        self.emit(abi::compare_immediate(&tail_bit, "0"));
        self.emit(abi::branch_eq(&tail_done));
        self.emit_simd_unary_scalar(kernel, &in_data, &out_data, &err)?;
        self.emit(abi::label(&tail_done));

        // --- Error reduce ---  (the two mask lanes, allocator-placed)
        if kernel.error().is_some() {
            let lane0 = self.allocate_register()?;
            let lane1 = self.allocate_register()?;
            self.emit(abi::vector_extract_to_x(&lane0, abi::VEC_SCRATCH[7], 0));
            self.emit(abi::vector_extract_to_x(&lane1, abi::VEC_SCRATCH[7], 1));
            self.emit(abi::or_registers(&lane0, &lane0, &lane1));
            self.emit(abi::or_registers(&err, &err, &lane0));
            let no_err = self.label("simd_no_err");
            self.emit(abi::compare_immediate(&err, "0"));
            self.emit(abi::branch_eq(&no_err));
            match kernel.error().unwrap() {
                SimdError::Overflow => self.emit_overflow_return()?,
                SimdError::FloatDomain => self.emit_float_domain_return()?,
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
                self.emit(abi::vector_dup_from_x(abi::VEC_SCRATCH[6], &min));
            }
            SimdUnaryKernel::AbsFloat | SimdUnaryKernel::SqrtFloat => {}
            SimdUnaryKernel::FloorFloat
            | SimdUnaryKernel::CeilFloat
            | SimdUnaryKernel::RoundFloat => {
                // v4 = exp(Inf/NaN), v5 = +2^63, v6 = -2^63 (range bounds).
                self.broadcast_const(abi::VEC_SCRATCH[4], FLOAT_EXP_INF_NAN)?;
                self.broadcast_const(abi::VEC_SCRATCH[5], FLOAT_TWO_POW_63_BITS)?;
                self.broadcast_const(abi::VEC_SCRATCH[6], FLOAT_NEG_TWO_POW_63_BITS)?;
            }
            SimdUnaryKernel::FloorFixed => {}
            SimdUnaryKernel::CeilFixed => {
                // bug-308: the fraction mask and a 1, not the `ONE-1` bias — see the
                // kernel body for why the bias form was wrong.
                self.broadcast_const(abi::VEC_SCRATCH[4], FIXED_FRACTION_MASK_STR)?;
                self.broadcast_const(abi::VEC_SCRATCH[5], "0")?;
                self.broadcast_const(abi::VEC_SCRATCH[6], "1")?;
            }
            SimdUnaryKernel::RoundFixed => {
                self.broadcast_const(abi::VEC_SCRATCH[4], FIXED_FRACTION_MASK_STR)?;
                self.broadcast_const(abi::VEC_SCRATCH[5], FIXED_HALF_STR)?;
                self.broadcast_const(abi::VEC_SCRATCH[6], "1")?;
            }
        }
        Ok(())
    }

    /// Materialize `value` into a GPR and broadcast it into both `.2d` lanes of
    /// the given vector register. Uses the caller-saved scratch `x0` (free: the
    /// loop runs after the only call), so it does not consume a loop register.
    fn broadcast_const(&mut self, vreg: &str, value: &str) -> Result<(), String> {
        let tmp = self.allocate_register()?;
        self.emit(abi::move_immediate(&tmp, "Integer", value));
        self.emit(abi::vector_dup_from_x(vreg, &tmp));
        Ok(())
    }

    /// Emit the per-chunk NEON kernel: input lanes arrive in `v0`, the result is
    /// left in `v0`, and any failing lanes are OR-accumulated into the `v7` mask.
    fn emit_simd_unary_vector(&mut self, kernel: SimdUnaryKernel) -> Result<(), String> {
        match kernel {
            SimdUnaryKernel::AbsInteger => {
                // Detect INT64_MIN lanes from the *input* (abs of INT64_MIN wraps
                // back to INT64_MIN, so the check must precede the abs).
                self.emit(abi::vector_cmeq(
                    abi::VEC_SCRATCH[1],
                    abi::VEC_SCRATCH[0],
                    abi::VEC_SCRATCH[6],
                ));
                self.emit(abi::vector_orr(
                    abi::VEC_SCRATCH[7],
                    abi::VEC_SCRATCH[7],
                    abi::VEC_SCRATCH[1],
                ));
                self.emit(abi::vector_abs(abi::VEC_SCRATCH[0], abi::VEC_SCRATCH[0]));
            }
            SimdUnaryKernel::AbsFloat => {
                self.emit(abi::vector_fabs(abi::VEC_SCRATCH[0], abi::VEC_SCRATCH[0]));
            }
            SimdUnaryKernel::SqrtFloat => {
                // Negative lanes (from the input) have no real square root.
                self.emit(abi::vector_fcmlt_zero(
                    abi::VEC_SCRATCH[1],
                    abi::VEC_SCRATCH[0],
                ));
                self.emit(abi::vector_orr(
                    abi::VEC_SCRATCH[7],
                    abi::VEC_SCRATCH[7],
                    abi::VEC_SCRATCH[1],
                ));
                self.emit(abi::vector_fsqrt(abi::VEC_SCRATCH[0], abi::VEC_SCRATCH[0]));
            }
            SimdUnaryKernel::FloorFloat
            | SimdUnaryKernel::CeilFloat
            | SimdUnaryKernel::RoundFloat => {
                let frint = kernel.float_round_mnemonic().unwrap();
                // Inf/NaN: exp field == 2047 (caught here; range compares below
                // miss NaN, which compares false against everything).
                self.emit(abi::vector_ushr(
                    abi::VEC_SCRATCH[2],
                    abi::VEC_SCRATCH[0],
                    52,
                ));
                self.emit(abi::vector_and(
                    abi::VEC_SCRATCH[2],
                    abi::VEC_SCRATCH[2],
                    abi::VEC_SCRATCH[4],
                ));
                self.emit(abi::vector_cmeq(
                    abi::VEC_SCRATCH[1],
                    abi::VEC_SCRATCH[2],
                    abi::VEC_SCRATCH[4],
                ));
                self.emit(abi::vector_orr(
                    abi::VEC_SCRATCH[7],
                    abi::VEC_SCRATCH[7],
                    abi::VEC_SCRATCH[1],
                ));
                // Round to integral, then bounds-check the rounded double.
                self.emit(
                    CodeInstruction::new(frint)
                        .field("dst", abi::VEC_SCRATCH[3])
                        .field("src", abi::VEC_SCRATCH[0]),
                );
                self.emit(abi::vector_fcmge(
                    abi::VEC_SCRATCH[1],
                    abi::VEC_SCRATCH[3],
                    abi::VEC_SCRATCH[5],
                )); // rounded >= 2^63
                self.emit(abi::vector_orr(
                    abi::VEC_SCRATCH[7],
                    abi::VEC_SCRATCH[7],
                    abi::VEC_SCRATCH[1],
                ));
                self.emit(abi::vector_fcmgt(
                    abi::VEC_SCRATCH[1],
                    abi::VEC_SCRATCH[6],
                    abi::VEC_SCRATCH[3],
                )); // -2^63 > rounded
                self.emit(abi::vector_orr(
                    abi::VEC_SCRATCH[7],
                    abi::VEC_SCRATCH[7],
                    abi::VEC_SCRATCH[1],
                ));
                self.emit(abi::vector_fcvtzs(abi::VEC_SCRATCH[0], abi::VEC_SCRATCH[3]));
            }
            SimdUnaryKernel::FloorFixed => {
                // Arithmetic shift right by 32 rounds toward -infinity.
                self.emit(abi::vector_sshr(
                    abi::VEC_SCRATCH[0],
                    abi::VEC_SCRATCH[0],
                    FIXED_SHIFT,
                ));
            }
            SimdUnaryKernel::CeilFixed => {
                // result = floor(x) + (frac != 0), matching the scalar Fixed ceil.
                //
                // bug-308: this used to compute `ceil(x) = floor(x + (ONE-1))`,
                // biasing by `2^32-1` before the shift. That add is modular i64 and
                // overflows for any raw `> i64::MAX - (2^32-1)` — i.e. every Fixed
                // whose value lies in (2147483647, 2147483648). It wrapped to a
                // large negative and the arithmetic shift produced a large negative
                // integer, so `math::ceil([x, x])` returned -2147483648 where both
                // the scalar overload and a length-1 list (odd tail → scalar path)
                // returned the correct 2147483648. The result is representable, so
                // nothing errored — it was simply wrong, and wrong only for even
                // lengths.
                //
                // Deriving the whole part by shifting FIRST cannot overflow: the
                // shift is a narrowing of a value already in range, and the +1 is
                // applied to the integer part, which always fits.
                self.emit(abi::vector_sshr(
                    abi::VEC_SCRATCH[1],
                    abi::VEC_SCRATCH[0],
                    FIXED_SHIFT,
                )); // whole = floor(x)
                self.emit(abi::vector_and(
                    abi::VEC_SCRATCH[2],
                    abi::VEC_SCRATCH[0],
                    abi::VEC_SCRATCH[4],
                )); // frac
                self.emit(abi::vector_cmgt(
                    abi::VEC_SCRATCH[3],
                    abi::VEC_SCRATCH[2],
                    abi::VEC_SCRATCH[5],
                )); // frac > 0  (frac is masked, so never negative)
                self.emit(abi::vector_and(
                    abi::VEC_SCRATCH[3],
                    abi::VEC_SCRATCH[3],
                    abi::VEC_SCRATCH[6],
                )); // mask & 1
                self.emit(abi::vector_add(
                    abi::VEC_SCRATCH[0],
                    abi::VEC_SCRATCH[1],
                    abi::VEC_SCRATCH[3],
                ));
            }
            SimdUnaryKernel::RoundFixed => {
                // result = floor(x) + (frac >= threshold), threshold = half + sign
                // (ties away from zero, matching the scalar Fixed rounder).
                self.emit(abi::vector_sshr(
                    abi::VEC_SCRATCH[1],
                    abi::VEC_SCRATCH[0],
                    FIXED_SHIFT,
                )); // whole
                self.emit(abi::vector_and(
                    abi::VEC_SCRATCH[2],
                    abi::VEC_SCRATCH[0],
                    abi::VEC_SCRATCH[4],
                )); // frac
                self.emit(abi::vector_ushr(
                    abi::VEC_SCRATCH[3],
                    abi::VEC_SCRATCH[0],
                    63,
                )); // sign bit (0/1)
                self.emit(abi::vector_add(
                    abi::VEC_SCRATCH[3],
                    abi::VEC_SCRATCH[5],
                    abi::VEC_SCRATCH[3],
                )); // threshold
                self.emit(abi::vector_cmge(
                    abi::VEC_SCRATCH[3],
                    abi::VEC_SCRATCH[2],
                    abi::VEC_SCRATCH[3],
                )); // frac >= threshold
                self.emit(abi::vector_and(
                    abi::VEC_SCRATCH[3],
                    abi::VEC_SCRATCH[3],
                    abi::VEC_SCRATCH[6],
                )); // mask & 1
                self.emit(abi::vector_add(
                    abi::VEC_SCRATCH[0],
                    abi::VEC_SCRATCH[1],
                    abi::VEC_SCRATCH[3],
                ));
            }
        }
        Ok(())
    }

    /// Emit the scalar tail kernel: read one lane from `[in_data]`, transform it,
    /// store to `[out_data]`, and set `err` to 1 on a failing lane. Transient
    /// scratch uses the caller-saved `x0`–`x5` (free after the alloc call) so the
    /// tail does not consume loop registers; `in_data`/`out_data`/`err` are the
    /// persistent loop registers (`x8`+).
    fn emit_simd_unary_scalar(
        &mut self,
        kernel: SimdUnaryKernel,
        in_data: &str,
        out_data: &str,
        err: &str,
    ) -> Result<(), String> {
        // Per-element scratch, allocator-placed (plan-34-B Phase 3): `elem` is the
        // loaded lane, `tmp` the transform temporary / convert destination.
        let elem = self.allocate_register()?;
        let tmp = self.allocate_register()?;
        self.emit(abi::load_u64(&elem, in_data, 0));
        match kernel {
            SimdUnaryKernel::AbsInteger => {
                self.emit(abi::move_immediate(&tmp, "Integer", INT64_MIN_UNSIGNED));
                let no_of = self.label("simd_tail_no_overflow");
                self.emit(abi::compare_registers(&elem, &tmp));
                self.emit(abi::branch_ne(&no_of));
                self.emit(abi::move_immediate(err, "Integer", "1"));
                self.emit(abi::label(&no_of));
                // abs: negate when negative.
                let negate = self.label("simd_tail_negate");
                let stored = self.label("simd_tail_stored");
                self.emit(abi::compare_immediate(&elem, "0"));
                self.emit(abi::branch_lt(&negate));
                self.emit(abi::store_u64(&elem, out_data, 0));
                self.emit(abi::branch(&stored));
                self.emit(abi::label(&negate));
                self.emit(abi::subtract_registers(&elem, abi::ZERO, &elem));
                self.emit(abi::store_u64(&elem, out_data, 0));
                self.emit(abi::label(&stored));
            }
            SimdUnaryKernel::AbsFloat => {
                self.emit(abi::move_immediate(&tmp, "Integer", FLOAT_ABS_MASK));
                self.emit(abi::and_registers(&elem, &elem, &tmp));
                self.emit(abi::store_u64(&elem, out_data, 0));
            }
            SimdUnaryKernel::SqrtFloat => {
                self.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[0], &elem));
                self.emit(abi::float_compare_zero_d(abi::FP_SCRATCH[0]));
                let no_err = self.label("simd_tail_sqrt_ok");
                // ge 0 is fine; lt 0 (or unordered/NaN) fails the domain.
                self.emit(abi::branch_ge(&no_err));
                self.emit(abi::move_immediate(err, "Integer", "1"));
                self.emit(abi::label(&no_err));
                self.emit(abi::float_sqrt_d(abi::FP_SCRATCH[0], abi::FP_SCRATCH[0]));
                self.emit(abi::float_move_x_from_d(&elem, abi::FP_SCRATCH[0]));
                self.emit(abi::store_u64(&elem, out_data, 0));
            }
            SimdUnaryKernel::FloorFloat
            | SimdUnaryKernel::CeilFloat
            | SimdUnaryKernel::RoundFloat => {
                self.emit_float_to_int_overflow_to_err(&elem, err)?;
                self.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[0], &elem));
                match kernel {
                    SimdUnaryKernel::FloorFloat => {
                        self.emit(abi::float_floor_to_signed_x(&tmp, abi::FP_SCRATCH[0]))
                    }
                    SimdUnaryKernel::CeilFloat => {
                        self.emit(abi::float_ceil_to_signed_x(&tmp, abi::FP_SCRATCH[0]))
                    }
                    SimdUnaryKernel::RoundFloat => {
                        self.emit(abi::float_round_to_signed_x(&tmp, abi::FP_SCRATCH[0]))
                    }
                    _ => unreachable!(),
                }
                self.emit(abi::store_u64(&tmp, out_data, 0));
            }
            SimdUnaryKernel::FloorFixed
            | SimdUnaryKernel::CeilFixed
            | SimdUnaryKernel::RoundFixed => {
                let function = match kernel {
                    SimdUnaryKernel::FloorFixed => "floor",
                    SimdUnaryKernel::CeilFixed => "ceil",
                    SimdUnaryKernel::RoundFixed => "round",
                    _ => unreachable!(),
                };
                self.emit_fixed_rounding_to_integer(function, &elem, &tmp)?;
                self.emit(abi::store_u64(&tmp, out_data, 0));
            }
        }
        Ok(())
    }

    /// Scalar float→int range check that sets `err` to 1 (rather than returning)
    /// when `bits` cannot round into the signed 64-bit range. Mirrors the
    /// terminal `emit_float_rounding_integer_range_check`: a value overflows when
    /// its biased exponent exceeds 1086, equals 2047 (Inf/NaN), or equals 1086
    /// and is not exactly `-2^63`.
    fn emit_float_to_int_overflow_to_err(&mut self, bits: &str, err: &str) -> Result<(), String> {
        // Allocator-placed scratch (plan-34-B Phase 3); `bits` is the caller's
        // element vreg, `err` its error accumulator.
        let exponent = self.allocate_register()?;
        let mask = self.allocate_register()?;
        let sign = self.allocate_register()?;
        let mantissa = self.allocate_register()?;
        let ok = self.label("simd_tail_round_ok");
        let edge = self.label("simd_tail_round_edge");
        let overflow = self.label("simd_tail_round_overflow");

        self.emit(abi::shift_right_immediate(&exponent, bits, 52));
        self.emit(abi::move_immediate(&mask, "Integer", "2047"));
        self.emit(abi::and_registers(&exponent, &exponent, &mask));
        self.emit(abi::compare_immediate(&exponent, "2047"));
        self.emit(abi::branch_eq(&overflow));
        self.emit(abi::compare_immediate(&exponent, "1086"));
        self.emit(abi::branch_lt(&ok));
        self.emit(abi::branch_eq(&edge));
        self.emit(abi::branch(&overflow));

        self.emit(abi::label(&edge));
        // exp == 1086 is only representable when it is exactly -2^63 (sign set,
        // zero mantissa); anything else overflows.
        self.emit(abi::shift_right_immediate(&sign, bits, 63));
        self.emit(abi::compare_immediate(&sign, "1"));
        self.emit(abi::branch_ne(&overflow));
        self.emit(abi::move_immediate(&mask, "Integer", "4503599627370495"));
        self.emit(abi::and_registers(&mantissa, bits, &mask));
        self.emit(abi::compare_immediate(&mantissa, "0"));
        self.emit(abi::branch_eq(&ok));

        self.emit(abi::label(&overflow));
        self.emit(abi::move_immediate(err, "Integer", "1"));
        self.emit(abi::label(&ok));
        Ok(())
    }

    /// Lower a two-array `math::` overload (`min`/`max`). Both lists must have the
    /// same length (`ErrInvalidArgument` otherwise). Streams both data regions in
    /// lockstep, two lanes at a time, with the odd tail handled scalar-wise.
    pub(super) fn lower_simd_binary(
        &mut self,
        kernel: SimdBinaryKernel,
        left_slot: usize,
        right_slot: usize,
        result_type: &str,
        result_type_code: usize,
        text: String,
    ) -> Result<ValueResult, String> {
        // `left_slot`/`right_slot` already hold the two list pointers (the caller
        // spilled them as it lowered, so neither crossed the other's lowering).
        self.reset_temporary_registers();
        let left_ptr = self.allocate_register()?;
        self.emit(abi::load_u64(&left_ptr, abi::stack_pointer(), left_slot));
        let right_ptr = self.allocate_register()?;
        self.emit(abi::load_u64(&right_ptr, abi::stack_pointer(), right_slot));
        let count = self.allocate_register()?;
        let rcount = self.allocate_register()?;
        self.emit(abi::load_u64(&count, &left_ptr, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(&rcount, &right_ptr, COLLECTION_OFFSET_COUNT));
        // Lengths must match.
        let lengths_ok = self.label("simd_bin_lengths_ok");
        self.emit(abi::compare_registers(&count, &rcount));
        self.emit(abi::branch_eq(&lengths_ok));
        self.emit_invalid_argument_return()?;
        self.emit(abi::label(&lengths_ok));

        let count_slot = self.allocate_stack_object("simd_bin_count", 8);
        self.emit(abi::store_u64(&count, abi::stack_pointer(), count_slot));

        self.emit(abi::move_register(abi::ARG[0], &count));
        self.emit(abi::move_immediate(
            abi::ARG[1],
            "Integer",
            &result_type_code.to_string(),
        ));
        self.emit(abi::branch_link(SIMD_ALLOC_LIST_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: SIMD_ALLOC_LIST_SYMBOL.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });

        self.reset_temporary_registers();
        let result_base = self.allocate_register()?;
        self.emit(abi::move_register(&result_base, abi::return_register()));
        let alloc_ok = self.label("simd_bin_alloc_ok");
        self.emit(abi::compare_immediate(abi::RET[1], "0"));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit(abi::move_register(abi::return_register(), abi::RET[1]));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&alloc_ok));

        let left_ptr = self.allocate_register()?;
        self.emit(abi::load_u64(&left_ptr, abi::stack_pointer(), left_slot));
        let right_ptr = self.allocate_register()?;
        self.emit(abi::load_u64(&right_ptr, abi::stack_pointer(), right_slot));
        let count = self.allocate_register()?;
        self.emit(abi::load_u64(&count, abi::stack_pointer(), count_slot));
        let left_data = self.allocate_register()?;
        self.emit_collection_data_pointer(&left_data, &left_ptr);
        let right_data = self.allocate_register()?;
        self.emit_collection_data_pointer(&right_data, &right_ptr);
        let out_data = self.allocate_register()?;
        self.emit_collection_data_pointer(&out_data, &result_base);
        let pairs = self.allocate_register()?;
        self.emit(abi::shift_right_immediate(&pairs, &count, 1));

        let loop_label = self.label("simd_bin_loop");
        let loop_done = self.label("simd_bin_done");
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate(&pairs, "0"));
        self.emit(abi::branch_eq(&loop_done));
        self.emit(abi::vector_load(abi::VEC_SCRATCH[0], &left_data, 0));
        self.emit(abi::vector_load(abi::VEC_SCRATCH[1], &right_data, 0));
        self.emit_simd_binary_vector(kernel);
        self.emit(abi::vector_store(abi::VEC_SCRATCH[0], &out_data, 0));
        self.emit(abi::add_immediate(&left_data, &left_data, 16));
        self.emit(abi::add_immediate(&right_data, &right_data, 16));
        self.emit(abi::add_immediate(&out_data, &out_data, 16));
        self.emit(abi::subtract_immediate(&pairs, &pairs, 1));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&loop_done));

        // Scalar tail (count & 1): left/right lanes are allocator-placed vregs.
        let tail_bit = self.allocate_register()?;
        self.emit(abi::move_immediate(&tail_bit, "Integer", "1"));
        self.emit(abi::and_registers(&tail_bit, &count, &tail_bit));
        let tail_done = self.label("simd_bin_tail_done");
        self.emit(abi::compare_immediate(&tail_bit, "0"));
        self.emit(abi::branch_eq(&tail_done));
        let left_lane = self.allocate_register()?;
        let right_lane = self.allocate_register()?;
        self.emit(abi::load_u64(&left_lane, &left_data, 0));
        self.emit(abi::load_u64(&right_lane, &right_data, 0));
        self.emit_simd_binary_scalar(kernel, &left_lane, &right_lane);
        self.emit(abi::store_u64(&left_lane, &out_data, 0));
        self.emit(abi::label(&tail_done));

        Ok(ValueResult {
            type_: result_type.to_string(),
            location: result_base,
            text,
        })
    }

    /// Per-chunk NEON min/max: lanes in `v0` (left) and `v1` (right); result → `v0`.
    fn emit_simd_binary_vector(&mut self, kernel: SimdBinaryKernel) {
        match kernel {
            SimdBinaryKernel::MinFloat => self.emit(abi::vector_fmin(
                abi::VEC_SCRATCH[0],
                abi::VEC_SCRATCH[0],
                abi::VEC_SCRATCH[1],
            )),
            SimdBinaryKernel::MaxFloat => self.emit(abi::vector_fmax(
                abi::VEC_SCRATCH[0],
                abi::VEC_SCRATCH[0],
                abi::VEC_SCRATCH[1],
            )),
            SimdBinaryKernel::MinSigned => {
                // min(a,b): select a where b>a (a is smaller), else b.
                self.emit(abi::vector_cmgt(
                    abi::VEC_SCRATCH[2],
                    abi::VEC_SCRATCH[1],
                    abi::VEC_SCRATCH[0],
                )); // b > a
                self.emit(abi::vector_bsl(
                    abi::VEC_SCRATCH[2],
                    abi::VEC_SCRATCH[0],
                    abi::VEC_SCRATCH[1],
                ));
                self.emit(abi::vector_orr(
                    abi::VEC_SCRATCH[0],
                    abi::VEC_SCRATCH[2],
                    abi::VEC_SCRATCH[2],
                ));
            }
            SimdBinaryKernel::MaxSigned => {
                // max(a,b): select a where a>b, else b.
                self.emit(abi::vector_cmgt(
                    abi::VEC_SCRATCH[2],
                    abi::VEC_SCRATCH[0],
                    abi::VEC_SCRATCH[1],
                )); // a > b
                self.emit(abi::vector_bsl(
                    abi::VEC_SCRATCH[2],
                    abi::VEC_SCRATCH[0],
                    abi::VEC_SCRATCH[1],
                ));
                self.emit(abi::vector_orr(
                    abi::VEC_SCRATCH[0],
                    abi::VEC_SCRATCH[2],
                    abi::VEC_SCRATCH[2],
                ));
            }
        }
    }

    /// Scalar tail min/max: `x0` = left, `x1` = right; result → `x0`.
    fn emit_simd_binary_scalar(&mut self, kernel: SimdBinaryKernel, left: &str, right: &str) {
        let done = self.label("simd_bin_tail_sel_done");
        match kernel {
            SimdBinaryKernel::MinSigned => {
                self.emit(abi::compare_registers(left, right));
                self.emit(abi::branch_le(&done)); // left <= right → keep left
                self.emit(abi::move_register(left, right));
            }
            SimdBinaryKernel::MaxSigned => {
                self.emit(abi::compare_registers(left, right));
                self.emit(abi::branch_ge(&done)); // left >= right → keep left
                self.emit(abi::move_register(left, right));
            }
            SimdBinaryKernel::MinFloat | SimdBinaryKernel::MaxFloat => {
                // `fminnm`/`fmaxnm` — the same sign-of-zero-aware instruction the
                // vector body (`fmin`/`fmax`) and the scalar `math::min(Float,
                // Float)` overload use, so the odd tail lane is bit-identical to a
                // body lane. The old `fsub`+`fcmp #0` treated `+0.0`/`-0.0` as
                // equal on a tie and kept the wrong-signed zero (bug-68). For the
                // finite values a `List OF Float` can hold (NaN/Inf are rejected at
                // the finiteness boundary) `fminnm`/`fmaxnm` equals the body's
                // `fmin`/`fmax` exactly.
                self.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[0], left));
                self.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[1], right));
                if matches!(kernel, SimdBinaryKernel::MinFloat) {
                    self.emit(abi::float_min_d(
                        abi::FP_SCRATCH[0],
                        abi::FP_SCRATCH[0],
                        abi::FP_SCRATCH[1],
                    ));
                } else {
                    self.emit(abi::float_max_d(
                        abi::FP_SCRATCH[0],
                        abi::FP_SCRATCH[0],
                        abi::FP_SCRATCH[1],
                    ));
                }
                self.emit(abi::float_move_x_from_d(left, abi::FP_SCRATCH[0]));
            }
        }
        self.emit(abi::label(&done));
    }

    #[allow(clippy::too_many_arguments)]
    /// Lower `math::clamp(values AS T[], low AS T, high AS T)` — clamp each lane to
    /// `[low, high]` via `max(min(x, high), low)`. `low`/`high` are broadcast into
    /// both `.2d` lanes. Never errors.
    pub(super) fn lower_simd_clamp(
        &mut self,
        kernel: SimdClampKernel,
        in_slot: usize,
        low_slot: usize,
        high_slot: usize,
        result_type: &str,
        result_type_code: usize,
        text: String,
    ) -> Result<ValueResult, String> {
        // `in_slot`/`low_slot`/`high_slot` already hold the list pointer and the
        // two scalar bounds (the caller spilled them as it lowered each arg).
        let count_slot = self.allocate_stack_object("simd_clamp_count", 8);

        self.reset_temporary_registers();
        // low > high is invalid (matches the scalar math::clamp man page).
        let low = self.allocate_register()?;
        let high = self.allocate_register()?;
        self.emit(abi::load_u64(&low, abi::stack_pointer(), low_slot));
        self.emit(abi::load_u64(&high, abi::stack_pointer(), high_slot));
        let bounds_ok = self.label("simd_clamp_bounds_ok");
        match kernel {
            SimdClampKernel::Float => {
                self.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[0], &low));
                self.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[1], &high));
                self.emit(abi::float_compare_d(abi::FP_SCRATCH[0], abi::FP_SCRATCH[1]));
                self.emit(abi::branch_le(&bounds_ok)); // low <= high
            }
            SimdClampKernel::Signed => {
                self.emit(abi::compare_registers(&low, &high));
                self.emit(abi::branch_le(&bounds_ok));
            }
        }
        self.emit_invalid_argument_return()?;
        self.emit(abi::label(&bounds_ok));

        self.reset_temporary_registers();
        let in_ptr = self.allocate_register()?;
        self.emit(abi::load_u64(&in_ptr, abi::stack_pointer(), in_slot));
        let count = self.allocate_register()?;
        self.emit(abi::load_u64(&count, &in_ptr, COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64(&count, abi::stack_pointer(), count_slot));

        self.emit(abi::move_register(abi::ARG[0], &count));
        self.emit(abi::move_immediate(
            abi::ARG[1],
            "Integer",
            &result_type_code.to_string(),
        ));
        self.emit(abi::branch_link(SIMD_ALLOC_LIST_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: SIMD_ALLOC_LIST_SYMBOL.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });

        self.reset_temporary_registers();
        let result_base = self.allocate_register()?;
        self.emit(abi::move_register(&result_base, abi::return_register()));
        let alloc_ok = self.label("simd_clamp_alloc_ok");
        self.emit(abi::compare_immediate(abi::RET[1], "0"));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit(abi::move_register(abi::return_register(), abi::RET[1]));
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
        // v5 = broadcast(low), v6 = broadcast(high).
        let bound = self.temporary_vreg();
        self.emit(abi::load_u64(&bound, abi::stack_pointer(), low_slot));
        self.emit(abi::vector_dup_from_x(abi::VEC_SCRATCH[5], &bound));
        self.emit(abi::load_u64(&bound, abi::stack_pointer(), high_slot));
        self.emit(abi::vector_dup_from_x(abi::VEC_SCRATCH[6], &bound));

        let loop_label = self.label("simd_clamp_loop");
        let loop_done = self.label("simd_clamp_loop_done");
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate(&pairs, "0"));
        self.emit(abi::branch_eq(&loop_done));
        self.emit(abi::vector_load(abi::VEC_SCRATCH[0], &in_data, 0));
        self.emit_simd_clamp_vector(kernel);
        self.emit(abi::vector_store(abi::VEC_SCRATCH[0], &out_data, 0));
        self.emit(abi::add_immediate(&in_data, &in_data, 16));
        self.emit(abi::add_immediate(&out_data, &out_data, 16));
        self.emit(abi::subtract_immediate(&pairs, &pairs, 1));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&loop_done));

        // Scalar tail: lane / bounds are allocator-placed vregs (below).
        let tail_bit = self.temporary_vreg();
        self.emit(abi::move_immediate(&tail_bit, "Integer", "1"));
        self.emit(abi::and_registers(&tail_bit, &count, &tail_bit));
        let tail_done = self.label("simd_clamp_tail_done");
        self.emit(abi::compare_immediate(&tail_bit, "0"));
        self.emit(abi::branch_eq(&tail_done));
        // Scratch-pool registers (not x0-x2): the x86 remap colors ABI
        // registers by boundary role, and a block mixing a staged x1 (RETS[1]
        // = rdx) with a role-colored x2 (CALL_ARGS[2] = rdx) collides — the
        // low bound aliased the high bound and the tail lane clamped against
        // the wrong limit. x9-x11 map to three distinct GPRs on both ISAs.
        let lane = self.temporary_vreg();
        let low_bound = self.temporary_vreg();
        let high_bound = self.temporary_vreg();
        self.emit(abi::load_u64(&lane, &in_data, 0));
        self.emit(abi::load_u64(&low_bound, abi::stack_pointer(), low_slot));
        self.emit(abi::load_u64(&high_bound, abi::stack_pointer(), high_slot));
        self.emit_simd_clamp_scalar(kernel, &lane, &low_bound, &high_bound);
        self.emit(abi::store_u64(&lane, &out_data, 0));
        self.emit(abi::label(&tail_done));

        Ok(ValueResult {
            type_: result_type.to_string(),
            location: result_base,
            text,
        })
    }

    /// Per-chunk clamp: lane in `v0`, bounds in `v5` (low) / `v6` (high).
    fn emit_simd_clamp_vector(&mut self, kernel: SimdClampKernel) {
        match kernel {
            SimdClampKernel::Float => {
                self.emit(abi::vector_fmin(
                    abi::VEC_SCRATCH[0],
                    abi::VEC_SCRATCH[0],
                    abi::VEC_SCRATCH[6],
                )); // min(x, high)
                self.emit(abi::vector_fmax(
                    abi::VEC_SCRATCH[0],
                    abi::VEC_SCRATCH[0],
                    abi::VEC_SCRATCH[5],
                )); // max(.., low)
            }
            SimdClampKernel::Signed => {
                // min(x, high): select x where high>x else high.
                self.emit(abi::vector_cmgt(
                    abi::VEC_SCRATCH[1],
                    abi::VEC_SCRATCH[6],
                    abi::VEC_SCRATCH[0],
                ));
                self.emit(abi::vector_bsl(
                    abi::VEC_SCRATCH[1],
                    abi::VEC_SCRATCH[0],
                    abi::VEC_SCRATCH[6],
                ));
                self.emit(abi::vector_orr(
                    abi::VEC_SCRATCH[0],
                    abi::VEC_SCRATCH[1],
                    abi::VEC_SCRATCH[1],
                ));
                // max(.., low): select v0 where v0>low else low.
                self.emit(abi::vector_cmgt(
                    abi::VEC_SCRATCH[1],
                    abi::VEC_SCRATCH[0],
                    abi::VEC_SCRATCH[5],
                ));
                self.emit(abi::vector_bsl(
                    abi::VEC_SCRATCH[1],
                    abi::VEC_SCRATCH[0],
                    abi::VEC_SCRATCH[5],
                ));
                self.emit(abi::vector_orr(
                    abi::VEC_SCRATCH[0],
                    abi::VEC_SCRATCH[1],
                    abi::VEC_SCRATCH[1],
                ));
            }
        }
    }

    /// Scalar tail clamp: `lane` = lane, `low` = low, `high` = high; result → `lane`.
    fn emit_simd_clamp_scalar(
        &mut self,
        kernel: SimdClampKernel,
        lane: &str,
        low: &str,
        high: &str,
    ) {
        match kernel {
            SimdClampKernel::Signed => {
                // lane = min(lane, high)
                let skip_hi = self.label("simd_clamp_tail_skip_hi");
                self.emit(abi::compare_registers(lane, high));
                self.emit(abi::branch_le(&skip_hi));
                self.emit(abi::move_register(lane, high));
                self.emit(abi::label(&skip_hi));
                // lane = max(lane, low)
                let skip_lo = self.label("simd_clamp_tail_skip_lo");
                self.emit(abi::compare_registers(lane, low));
                self.emit(abi::branch_ge(&skip_lo));
                self.emit(abi::move_register(lane, low));
                self.emit(abi::label(&skip_lo));
            }
            SimdClampKernel::Float => {
                // `fminnm`/`fmaxnm` — matching the vector body
                // (`vector_fmin`/`vector_fmax`) so the odd tail lane is
                // bit-identical to a body lane on signed zeros. The old
                // `fsub`+`fcmp #0` lost the sign of a `±0.0` tie (bug-68).
                // lane = max(min(lane, high), low)
                self.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[0], lane));
                self.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[1], high));
                self.emit(abi::float_min_d(
                    abi::FP_SCRATCH[0],
                    abi::FP_SCRATCH[0],
                    abi::FP_SCRATCH[1],
                )); // min(lane, high)
                self.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[1], low));
                self.emit(abi::float_max_d(
                    abi::FP_SCRATCH[0],
                    abi::FP_SCRATCH[0],
                    abi::FP_SCRATCH[1],
                )); // max(.., low)
                self.emit(abi::float_move_x_from_d(lane, abi::FP_SCRATCH[0]));
            }
        }
    }
}
