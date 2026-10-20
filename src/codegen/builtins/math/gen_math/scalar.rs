//! plan-176-S: scalar `math` kernels — transcendentals, binary ops, abs/min/max/clamp, rounding,
//! sqrt and the `Fixed` externals. `lower_math_call` lives in `builtins/math/gen_math/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// Scalar `exp`/`log`/`log10`/`sin`/`cos`: `Float` runs the shared NEON
    /// kernel (bit-identical to the array overload); `Fixed` keeps the
    /// deterministic Q32.32 path. `log`/`log10` keep their `ErrFloatDomain`
    /// pre-check so the scalar error code is unchanged.
    pub(super) fn lower_math_scalar_transcendental(
        &mut self,
        function: &str,
        arg: &ValueResult,
    ) -> Result<ValueResult, String> {
        use crate::codegen::builtins::vector::builder_simd_float_math::FloatKernel;
        // The scalar kernel reads the operand's bits from a GPR, so materialize a
        // `d`-native float into one first (plan-01 float-dnative).
        let value = arg.clone();
        let value = self.materialize_float(value)?;
        match &value.type_ {
            ParameterType::Float => {
                let text = format!("math.{function}({})", value.text);
                // The kernels raise the matching float error themselves
                // (ErrFloatDomain for log/log10 ≤0, ErrFloatInf/ErrFloatNan for
                // exp), so no pre-check is needed.
                let kernel = match function {
                    "exp" => FloatKernel::Exp,
                    "log" => FloatKernel::Log,
                    "log10" => FloatKernel::Log10,
                    "sin" => FloatKernel::Sin,
                    "cos" => FloatKernel::Cos,
                    "tan" => FloatKernel::Tan,
                    "atan" => FloatKernel::Atan,
                    "asin" => FloatKernel::Asin,
                    "acos" => FloatKernel::Acos,
                    _ => return Err(format!("math.{function} has no scalar Float kernel")),
                };
                self.lower_simd_float_scalar(kernel, &value.location, text)
            }
            ParameterType::Fixed => self.lower_fixed_external_math(function, &[value]),
            other => Err(format!("math.{function} does not accept {other}")),
        }
    }

    /// Scalar `atan2`/`pow`: `Float` runs the shared NEON binary kernel
    /// (bit-identical to the array overload, no libm import); `Fixed` keeps the
    /// deterministic Q32.32 path. Mirrors `lower_external_math`'s arg spilling so
    /// a later argument's lowering cannot clobber the first.
    pub(super) fn lower_math_scalar_binary(
        &mut self,
        function: &str,
        args: &[ValueResult],
    ) -> Result<ValueResult, String> {
        use crate::codegen::builtins::vector::builder_simd_float_math::FloatBinaryKernel;
        // The kernels read each operand's bits from a GPR slot, so materialize a
        // `d`-native float before spilling (plan-01 float-dnative).
        let left = args[0].clone();
        let left = self.materialize_float(left)?;
        let left_slot = self.allocate_stack_object("scalar_binary_left", 8);
        self.emit(abi::store_u64(
            &left.location,
            abi::stack_pointer(),
            left_slot,
        ));
        let right = args[1].clone();
        let right = self.materialize_float(right)?;
        let right_slot = self.allocate_stack_object("scalar_binary_right", 8);
        self.emit(abi::store_u64(
            &right.location,
            abi::stack_pointer(),
            right_slot,
        ));
        if left.type_ != right.type_ {
            return Err(format!(
                "math.{function} requires matching argument types, got {} and {}",
                left.type_, right.type_
            ));
        }
        let text = format!("math.{function}({}, {})", left.text, right.text);
        match &left.type_ {
            ParameterType::Float if function == "pow" => {
                // pow is a scalar GPR+FP fdlibm kernel (not SIMD); it produces inf
                // on overflow and NaN for a negative base with a non-integer
                // exponent, which the result check turns into ErrFloatInf /
                // ErrFloatNan (matching the scalar pow man page).
                self.reset_temporary_registers();
                let left_reg = self.allocate_register();
                self.emit(abi::load_u64(&left_reg, abi::stack_pointer(), left_slot));
                let right_reg = self.allocate_register();
                self.emit(abi::load_u64(&right_reg, abi::stack_pointer(), right_slot));
                let result = self.emit_pow_scalar(&left_reg, &right_reg)?;
                self.emit_float_result_check(&result, FloatInfinityError::Infinity)?;
                Ok(ValueResult {
                    origin: None,
                    type_: ParameterType::Float,
                    location: Operand::from(result),
                    text,
                })
            }
            ParameterType::Float => {
                let kernel = match function {
                    "atan2" => FloatBinaryKernel::Atan2,
                    // `pow` never reaches here — the guarded `ParameterType::Float`
                    // arm above diverts it to the scalar fdlibm kernel.
                    _ => return Err(format!("math.{function} has no scalar Float binary kernel")),
                };
                self.reset_temporary_registers();
                let left_reg = self.allocate_register();
                self.emit(abi::load_u64(&left_reg, abi::stack_pointer(), left_slot));
                let right_reg = self.allocate_register();
                self.emit(abi::load_u64(&right_reg, abi::stack_pointer(), right_slot));
                self.lower_simd_float_binary_scalar(kernel, &left_reg, &right_reg, text)
            }
            ParameterType::Fixed => {
                self.reset_temporary_registers();
                let left_reg = self.allocate_register();
                self.emit(abi::load_u64(&left_reg, abi::stack_pointer(), left_slot));
                let right_reg = self.allocate_register();
                self.emit(abi::load_u64(&right_reg, abi::stack_pointer(), right_slot));
                let values = vec![
                    ValueResult {
                        origin: None,
                        type_: ParameterType::Fixed,
                        location: Operand::from(left_reg),
                        text: left.text,
                    },
                    ValueResult {
                        origin: None,
                        type_: ParameterType::Fixed,
                        location: Operand::from(right_reg),
                        text: right.text,
                    },
                ];
                self.lower_fixed_external_math(function, &values)
            }
            other => Err(format!("math.{function} does not accept {other}")),
        }
    }

    pub(super) fn lower_math_abs(&mut self, arg: &ValueResult) -> Result<ValueResult, String> {
        let value = arg.clone();
        let value = self.materialize_float(value)?;
        let dst = self.allocate_register();
        let bound = self.temporary_vreg();
        match &value.type_ {
            // Money is a raw i64, so |x| and the INT64_MIN overflow check are the
            // same integer op as Integer/Fixed (plan-29-G §4.7).
            ParameterType::Integer | ParameterType::Fixed | ParameterType::Money => {
                let ok = self.label("math_abs_ok");
                self.emit(abi::compare_immediate(&value.location, "0"));
                self.emit(abi::branch_ge(&ok));
                self.emit(abi::move_immediate(&bound, "Integer", F64_SIGN_BIT));
                self.emit(abi::compare_registers(&value.location, &bound));
                self.emit(abi::branch_ne(&ok));
                self.raise_error("math.abs", "ErrOverflow")?;
                self.emit(abi::label(&ok));
                self.emit(abi::compare_immediate(&value.location, "0"));
                let done = self.label("math_abs_done");
                let negative = self.label("math_abs_negative");
                self.emit(abi::branch_lt(&negative));
                self.emit(abi::move_register(&dst, &value.location));
                self.emit(abi::branch(&done));
                self.emit(abi::label(&negative));
                self.emit(abi::subtract_registers(&dst, abi::ZERO, &value.location));
                self.emit(abi::label(&done));
            }
            ParameterType::Float => {
                // Hardware `fabs` clears the sign bit in the FP domain (plan-02
                // §4). Bit-identical to the old GPR sign-mask AND for every finite
                // MFBASIC `Float`, but a single hardware instruction.
                self.emit(abi::float_move_d_from_x(
                    abi::FP_SCRATCH[0],
                    &value.location,
                ));
                self.emit(abi::float_abs_d(abi::FP_SCRATCH[0], abi::FP_SCRATCH[0]));
                self.emit(abi::float_move_x_from_d(&dst, abi::FP_SCRATCH[0]));
            }
            other => return Err(format!("math.abs does not accept {other}")),
        }
        Ok(ValueResult {
            origin: None,
            type_: value.type_,
            location: Operand::from(dst),
            text: format!("math.abs({})", value.text),
        })
    }

    pub(super) fn lower_math_min_max(
        &mut self,
        function: &str,
        args: &[ValueResult],
    ) -> Result<ValueResult, String> {
        let left = args[0].clone();
        let left = self.materialize_float(left)?;
        let left_slot = self.allocate_stack_object("math_minmax_left", 8);
        self.emit(abi::store_u64(
            &left.location,
            abi::stack_pointer(),
            left_slot,
        ));
        let right = args[1].clone();
        let right = self.materialize_float(right)?;
        let right_slot = self.allocate_stack_object("math_minmax_right", 8);
        self.emit(abi::store_u64(
            &right.location,
            abi::stack_pointer(),
            right_slot,
        ));
        let dst = self.allocate_register();
        let lhs = self.allocate_register();
        let rhs = self.allocate_register();
        self.emit(abi::load_u64(&lhs, abi::stack_pointer(), left_slot));
        self.emit(abi::load_u64(&rhs, abi::stack_pointer(), right_slot));
        if left.type_ != right.type_ {
            return Err(format!(
                "math.{function} requires matching types, got {} and {}",
                left.type_, right.type_
            ));
        }
        match &left.type_ {
            // Money min/max is the signed-i64 compare+select (plan-29-G §4.7).
            ParameterType::Integer | ParameterType::Fixed | ParameterType::Money => {
                let take_left = self.label("math_minmax_take_left");
                let done = self.label("math_minmax_done");
                self.emit(abi::compare_registers(&lhs, &rhs));
                if function == "min" {
                    self.emit(abi::branch_le(&take_left));
                } else {
                    self.emit(abi::branch_ge(&take_left));
                }
                self.emit(abi::move_register(&dst, &rhs));
                self.emit(abi::branch(&done));
                self.emit(abi::label(&take_left));
                self.emit(abi::move_register(&dst, &lhs));
                self.emit(abi::label(&done));
            }
            ParameterType::Float => {
                // Hardware `fminnm`/`fmaxnm` (plan-02 §4): a single instruction in
                // place of the subtract/compare/branch. For the finite operands
                // MFBASIC produces this is bit-identical to the old ordered
                // compare that returned `lhs` on a tie.
                self.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[0], &lhs));
                self.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[1], &rhs));
                if function == "min" {
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
                self.emit(abi::float_move_x_from_d(&dst, abi::FP_SCRATCH[0]));
            }
            other => return Err(format!("math.{function} does not accept {other}")),
        }
        Ok(ValueResult {
            origin: None,
            type_: left.type_,
            location: Operand::from(dst),
            text: format!("math.{function}({}, {})", left.text, right.text),
        })
    }

    pub(super) fn lower_math_clamp(&mut self, args: &[ValueResult]) -> Result<ValueResult, String> {
        let value = args[0].clone();
        let value = self.materialize_float(value)?;
        let value_slot = self.allocate_stack_object("math_clamp_value", 8);
        self.emit(abi::store_u64(
            &value.location,
            abi::stack_pointer(),
            value_slot,
        ));
        let low = args[1].clone();
        let low = self.materialize_float(low)?;
        let low_slot = self.allocate_stack_object("math_clamp_low", 8);
        self.emit(abi::store_u64(
            &low.location,
            abi::stack_pointer(),
            low_slot,
        ));
        let high = args[2].clone();
        let high = self.materialize_float(high)?;
        let high_slot = self.allocate_stack_object("math_clamp_high", 8);
        self.emit(abi::store_u64(
            &high.location,
            abi::stack_pointer(),
            high_slot,
        ));
        if value.type_ != low.type_ || value.type_ != high.type_ {
            return Err("math.clamp requires three matching numeric arguments".to_string());
        }
        let dst = self.allocate_register();
        let value_reg = self.allocate_register();
        let low_reg = self.allocate_register();
        let high_reg = self.allocate_register();
        self.emit(abi::load_u64(&value_reg, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(&low_reg, abi::stack_pointer(), low_slot));
        self.emit(abi::load_u64(&high_reg, abi::stack_pointer(), high_slot));

        match &value.type_ {
            // Money clamp is the signed-i64 bounds compare+select (plan-29-G §4.7).
            ParameterType::Integer | ParameterType::Fixed | ParameterType::Money => {
                let bounds_valid = self.label("math_clamp_bounds_valid");
                let take_low = self.label("math_clamp_take_low");
                let take_high = self.label("math_clamp_take_high");
                let done = self.label("math_clamp_done");
                self.emit(abi::compare_registers(&low_reg, &high_reg));
                self.emit(abi::branch_le(&bounds_valid));
                self.raise_error_bare("ErrInvalidArgument")?;
                self.emit(abi::label(&bounds_valid));
                self.emit(abi::compare_registers(&value_reg, &low_reg));
                self.emit(abi::branch_lt(&take_low));
                self.emit(abi::compare_registers(&value_reg, &high_reg));
                self.emit(abi::branch_gt(&take_high));
                self.emit(abi::move_register(&dst, &value_reg));
                self.emit(abi::branch(&done));
                self.emit(abi::label(&take_low));
                self.emit(abi::move_register(&dst, &low_reg));
                self.emit(abi::branch(&done));
                self.emit(abi::label(&take_high));
                self.emit(abi::move_register(&dst, &high_reg));
                self.emit(abi::label(&done));
            }
            ParameterType::Float => {
                let bounds_valid = self.label("math_clamp_float_bounds_valid");
                let take_low = self.label("math_clamp_float_take_low");
                let take_high = self.label("math_clamp_float_take_high");
                let done = self.label("math_clamp_float_done");
                self.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[0], &low_reg));
                self.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[1], &high_reg));
                self.emit(abi::float_subtract_d(
                    abi::FP_SCRATCH[2],
                    abi::FP_SCRATCH[0],
                    abi::FP_SCRATCH[1],
                ));
                self.emit(abi::float_compare_zero_d(abi::FP_SCRATCH[2]));
                self.emit(abi::branch_le(&bounds_valid));
                self.raise_error_bare("ErrInvalidArgument")?;
                self.emit(abi::label(&bounds_valid));
                self.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[0], &value_reg));
                self.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[1], &low_reg));
                self.emit(abi::float_subtract_d(
                    abi::FP_SCRATCH[2],
                    abi::FP_SCRATCH[0],
                    abi::FP_SCRATCH[1],
                ));
                self.emit(abi::float_compare_zero_d(abi::FP_SCRATCH[2]));
                self.emit(abi::branch_lt(&take_low));
                self.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[1], &high_reg));
                self.emit(abi::float_subtract_d(
                    abi::FP_SCRATCH[2],
                    abi::FP_SCRATCH[0],
                    abi::FP_SCRATCH[1],
                ));
                self.emit(abi::float_compare_zero_d(abi::FP_SCRATCH[2]));
                self.emit(abi::branch_gt(&take_high));
                self.emit(abi::move_register(&dst, &value_reg));
                self.emit(abi::branch(&done));
                self.emit(abi::label(&take_low));
                self.emit(abi::move_register(&dst, &low_reg));
                self.emit(abi::branch(&done));
                self.emit(abi::label(&take_high));
                self.emit(abi::move_register(&dst, &high_reg));
                self.emit(abi::label(&done));
            }
            other => return Err(format!("math.clamp does not accept {other}")),
        }
        Ok(ValueResult {
            origin: None,
            type_: value.type_,
            location: Operand::from(dst),
            text: format!("math.clamp({}, {}, {})", value.text, low.text, high.text),
        })
    }

    pub(super) fn lower_math_rounding(
        &mut self,
        function: &str,
        arg: &ValueResult,
    ) -> Result<ValueResult, String> {
        let value = arg.clone();
        let value = self.materialize_float(value)?;
        let dst = self.allocate_register();
        match &value.type_ {
            ParameterType::Float => {
                self.emit(abi::float_move_d_from_x(
                    abi::FP_SCRATCH[0],
                    &value.location,
                ));
                self.emit_float_rounding_integer_range_check(&value.location)?;
                match function {
                    "floor" => self.emit(abi::float_floor_to_signed_x(&dst, abi::FP_SCRATCH[0])),
                    "ceil" => self.emit(abi::float_ceil_to_signed_x(&dst, abi::FP_SCRATCH[0])),
                    "round" => self.emit(abi::float_round_to_signed_x(&dst, abi::FP_SCRATCH[0])),
                    _ => unreachable!(),
                }
            }
            ParameterType::Fixed => {
                // Deterministic raw Q32.32 rounding: the integer result of
                // rounding a Fixed always fits in `Integer` range (|real| < 2^31),
                // so no host floating-point conversion is required.
                self.emit_fixed_rounding_to_integer(function, &value.location, &dst)?;
            }
            // floor/ceil/round(Money) → the whole-unit count `raw / 100000`, a
            // fixed rule (round uses half-away-from-zero, not the global mode —
            // an explicit call is presentation-like) (plan-29-G §4.7).
            ParameterType::Money => {
                self.emit_money_rounding_to_integer(function, &value.location, &dst)?;
            }
            other => return Err(format!("math.{function} does not accept {other}")),
        }
        Ok(ValueResult {
            origin: None,
            type_: ParameterType::Integer,
            location: Operand::from(dst),
            text: format!("math.{function}({})", value.text),
        })
    }

    /// Emit the shared IEEE-754 biased-exponent decode (bug-332 D1): shift the
    /// 11-bit exponent field of the f64 bit pattern in `bits` into `exponent`
    /// (using `mask` as scratch), then compare it against 2047 — the all-ones
    /// Inf/NaN field — leaving the flags set for the caller's branch. Registers are
    /// caller-allocated so the divergent range-check tails stay per-site.
    pub(crate) fn emit_float_exponent_classify(
        &mut self,
        exponent: impl Into<Reg>,
        mask: impl Into<Reg>,
        bits: impl Into<Reg>,
    ) {
        let (exponent, mask, bits): (Reg, Reg, Reg) = (exponent.into(), mask.into(), bits.into());
        self.emit(abi::shift_right_immediate(exponent, bits, 52));
        self.emit(abi::move_immediate(mask, "Integer", "2047"));
        self.emit(abi::and_registers(exponent, exponent, mask));
        self.emit(abi::compare_immediate(exponent, "2047"));
    }

    fn emit_float_rounding_integer_range_check(
        &mut self,
        source_bits: impl Into<Reg>,
    ) -> Result<(), String> {
        let source_bits: Reg = source_bits.into();
        let bits = self.allocate_register();
        let exponent = self.allocate_register();
        let sign = self.allocate_register();
        let mantissa = self.allocate_register();
        let mask = self.allocate_register();
        let ok = self.label("math_rounding_float_range_ok");
        let edge = self.label("math_rounding_float_range_edge");
        let edge_negative = self.label("math_rounding_float_range_edge_negative");
        let overflow = self.label("math_rounding_float_range_overflow");

        self.emit(abi::move_register(&bits, source_bits));
        self.emit_float_exponent_classify(&exponent, &mask, &bits);
        self.emit(abi::branch_eq(&overflow));
        self.emit(abi::compare_immediate(&exponent, "1086"));
        self.emit(abi::branch_lt(&ok));
        self.emit(abi::branch_eq(&edge));
        self.emit(abi::branch(&overflow));

        self.emit(abi::label(&edge));
        self.emit(abi::shift_right_immediate(&sign, &bits, 63));
        self.emit(abi::compare_immediate(&sign, "1"));
        self.emit(abi::branch_eq(&edge_negative));
        self.emit(abi::branch(&overflow));
        self.emit(abi::label(&edge_negative));
        self.emit(abi::move_immediate(&mask, "Integer", F64_MANTISSA_MASK));
        self.emit(abi::and_registers(&mantissa, &bits, &mask));
        self.emit(abi::compare_immediate(&mantissa, "0"));
        self.emit(abi::branch_eq(&ok));

        self.emit(abi::label(&overflow));
        self.raise_error_bare("ErrOverflow")?;
        self.emit(abi::label(&ok));
        Ok(())
    }

    pub(super) fn lower_math_sqrt(&mut self, arg: &ValueResult) -> Result<ValueResult, String> {
        let value = arg.clone();
        // Fixed keeps its GPR path (raw Q32.32 sqrt); only Float goes d-native.
        if value.type_ == ParameterType::Float {
            // plan-39 B2: keep sqrt d-register-native — read the operand straight
            // into a `d` register (no materialize-to-GPR + move-back shuttle),
            // `fcmp`/`fsqrt` there, and return the `%fN` result so the consumer
            // (observe_float / a store / another float op) takes it in the FP
            // domain with no re-load. `fsqrt` is IEEE-exact, so the value and the
            // domain trap (x<0 or NaN -> ErrFloatDomain) are bit-identical.
            let text = format!("math.sqrt({})", value.text);
            let src = self.operand_as_double(&value)?;
            self.emit(abi::float_compare_zero_d(&src));
            let valid = self.label("math_sqrt_valid");
            self.emit(abi::branch_ge(&valid));
            self.raise_error("math.sqrt", "ErrFloatDomain")?;
            self.emit(abi::label(&valid));
            let result = self.allocate_fp_register();
            self.emit(abi::float_sqrt_d(&result, &src));
            return Ok(ValueResult {
                origin: None,
                type_: ParameterType::Float,
                location: Operand::from(result),
                text,
            });
        }
        let value = self.materialize_float(value)?;
        match &value.type_ {
            ParameterType::Float => unreachable!("Float handled above"),
            ParameterType::Fixed => {
                self.emit(abi::compare_immediate(&value.location, "0"));
                let valid = self.label("math_fixed_sqrt_valid");
                self.emit(abi::branch_ge(&valid));
                self.raise_error_bare("ErrInvalidArgument")?;
                self.emit(abi::label(&valid));
                // Deterministic raw Q32.32 square root (no host floating-point).
                let dst = self.emit_fixed_sqrt(&value.location)?;
                Ok(ValueResult {
                    origin: None,
                    type_: ParameterType::Fixed,
                    location: Operand::from(dst),
                    text: format!("math.sqrt({})", value.text),
                })
            }
            other => Err(format!("math.sqrt does not accept {other}")),
        }
    }

    /// Lower a `Fixed` transcendental overload to a deterministic Q32.32
    /// implementation. `values` holds the already-lowered `Fixed` arguments.
    fn lower_fixed_external_math(
        &mut self,
        function: &str,
        values: &[ValueResult],
    ) -> Result<ValueResult, String> {
        let text = format!("math.{function}({})", join_texts(values));
        let location = match function {
            "atan2" => self.emit_fixed_atan2(&values[0].location, &values[1].location)?,
            "atan" => {
                let one = self.allocate_register();
                self.emit(abi::move_immediate(
                    &one,
                    abi::IMMEDIATE_CLASS_FIXED,
                    &(1u64 << 32).to_string(),
                ));
                self.emit_fixed_atan2(&values[0].location, &one)?
            }
            "asin" => self.emit_fixed_asin(&values[0].location, false)?,
            "acos" => self.emit_fixed_asin(&values[0].location, true)?,
            "sin" => self.emit_fixed_sin_cos(&values[0].location, false)?,
            "cos" => self.emit_fixed_sin_cos(&values[0].location, true)?,
            "tan" => self.emit_fixed_tan(&values[0].location)?,
            "exp" => self.emit_fixed_exp(&values[0].location)?,
            "log" => self.emit_fixed_log(&values[0].location, false)?,
            "log10" => self.emit_fixed_log(&values[0].location, true)?,
            "pow" => self.emit_fixed_pow_general(&values[0].location, &values[1].location)?,
            other => {
                return Err(format!(
                    "deterministic Fixed math does not support math.{other}"
                ))
            }
        };
        // The deterministic routines reset the register file internally and may
        // return a high-numbered register, leaving little room for the
        // surrounding expression. Normalize by spilling and reloading into a
        // freshly reset register file.
        let slot = self.allocate_stack_object("fixed_math_result", 8);
        self.emit(abi::store_u64(&location, abi::stack_pointer(), slot));
        self.reset_temporary_registers();
        let result = self.allocate_register();
        self.emit(abi::load_u64(&result, abi::stack_pointer(), slot));
        Ok(ValueResult {
            origin: None,
            type_: ParameterType::Fixed,
            location: Operand::from(result),
            text,
        })
    }
}
