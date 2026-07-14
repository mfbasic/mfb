use super::*;

pub(super) enum FloatInfinityError {
    Infinity,
    Overflow,
}

impl CodeBuilder<'_> {
    pub(super) fn lower_math_call(
        &mut self,
        function: &str,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        match function {
            "abs" if args.len() == 1 && self.is_list_argument(&args[0]) => {
                self.lower_math_abs_array(&args[0])
            }
            "abs" if args.len() == 1 => self.lower_math_abs(&args[0]),
            "sqrt" if args.len() == 1 && self.is_list_argument(&args[0]) => {
                self.lower_math_sqrt_array(&args[0])
            }
            "floor" | "ceil" | "round" if args.len() == 1 && self.is_list_argument(&args[0]) => {
                self.lower_math_rounding_array(function, &args[0])
            }
            "min" | "max" if args.len() == 2 && self.is_list_argument(&args[0]) => {
                self.lower_math_min_max_array(function, args)
            }
            "clamp" if args.len() == 3 && self.is_list_argument(&args[0]) => {
                self.lower_math_clamp_array(args)
            }
            "log" | "log10" if args.len() == 1 && self.is_list_argument(&args[0]) => {
                self.lower_math_log_array(function, &args[0])
            }
            "exp" if args.len() == 1 && self.is_list_argument(&args[0]) => {
                self.lower_math_exp_array(&args[0])
            }
            "atan2" | "pow" if args.len() == 2 && self.is_list_argument(&args[0]) => {
                self.lower_math_atan2_pow_array(function, args)
            }
            "sin" | "cos" | "tan" | "atan" | "asin" | "acos"
                if args.len() == 1 && self.is_list_argument(&args[0]) =>
            {
                self.lower_math_trig_array(function, &args[0])
            }
            "min" | "max" if args.len() == 2 => self.lower_math_min_max(function, args),
            "clamp" if args.len() == 3 => self.lower_math_clamp(args),
            "floor" | "ceil" | "round" if args.len() == 1 => {
                self.lower_math_rounding(function, &args[0])
            }
            "rand" if args.len() == 2 => self.lower_math_rand(args),
            "seed" if args.len() == 1 => self.lower_math_seed(&args[0]),
            "sqrt" if args.len() == 1 => self.lower_math_sqrt(&args[0]),
            // Scalar Float pow shares the strict array Pow kernel (negative-base
            // integer exponents included); Fixed keeps the Q32.32 path.
            "pow" if args.len() == 2 => self.lower_math_scalar_binary(function, args),
            // Scalar Float atan2 shares the strict array Atan2 kernel (so
            // `math::atan2(y, x)` and `math::atan2([y], [x])[0]` are bit-identical
            // and no libm `atan2` is imported); Fixed keeps the Q32.32 path.
            "atan2" if args.len() == 2 => self.lower_math_scalar_binary(function, args),
            // Scalar Float exp/log/log10/sin/cos share the array NEON kernels
            // (plan-01-simd §4.7: one deterministic surface, <=1 ULP of libm, and
            // the kernels' float error codes — ErrFloatDomain/ErrFloatInf/
            // ErrFloatNan — already match the scalar man pages). Fixed stays on
            // the deterministic Q32.32 path; tan/asin/acos/atan stay on libm until
            // their kernels reach strict <=1 ULP.
            "exp" | "log" | "log10" | "sin" | "cos" | "tan" | "atan" | "asin" | "acos"
                if args.len() == 1 =>
            {
                self.lower_math_scalar_transcendental(function, &args[0])
            }
            other => Err(format!(
                "native math lowering does not support math.{other}"
            )),
        }
    }

    /// Whether `arg`'s static type is a `List OF …` (selects the SIMD array
    /// overloads over the scalar `math::` lowerings).
    pub(super) fn is_list_argument(&self, arg: &NirValue) -> bool {
        self.static_type_name(arg)
            .is_some_and(|type_| type_.starts_with("List OF "))
    }

    /// `math.abs(values AS T[])` — vectorized absolute value (plan-01-simd §4.4).
    fn lower_math_abs_array(&mut self, arg: &NirValue) -> Result<ValueResult, String> {
        use super::builder_simd_math::SimdUnaryKernel;
        let input = self.lower_value(arg)?;
        let text = format!("math.abs({})", input.text);
        let element = input
            .type_
            .strip_prefix("List OF ")
            .ok_or_else(|| {
                format!(
                    "math.abs array overload requires a list, got {}",
                    input.type_
                )
            })?
            .to_string();
        match element.as_str() {
            "Integer" => self.lower_simd_unary(
                SimdUnaryKernel::AbsInteger,
                input,
                "List OF Integer",
                COLLECTION_TYPE_INTEGER,
                text,
            ),
            // Fixed is a raw Q32.32 i64; |x| and the INT64_MIN overflow check are
            // the same integer operation as Integer.
            "Fixed" => self.lower_simd_unary(
                SimdUnaryKernel::AbsInteger,
                input,
                "List OF Fixed",
                COLLECTION_TYPE_FIXED,
                text,
            ),
            "Float" => self.lower_simd_unary(
                SimdUnaryKernel::AbsFloat,
                input,
                "List OF Float",
                COLLECTION_TYPE_FLOAT,
                text,
            ),
            other => Err(format!(
                "math.abs array overload does not accept List OF {other}"
            )),
        }
    }

    /// `math.sqrt(values AS Float[])` — vectorized square root (plan-01-simd
    /// §4.4). `ErrInvalidArgument` if any lane is negative.
    fn lower_math_sqrt_array(&mut self, arg: &NirValue) -> Result<ValueResult, String> {
        use super::builder_simd_math::SimdUnaryKernel;
        let input = self.lower_value(arg)?;
        let text = format!("math.sqrt({})", input.text);
        let element = input
            .type_
            .strip_prefix("List OF ")
            .ok_or_else(|| {
                format!(
                    "math.sqrt array overload requires a list, got {}",
                    input.type_
                )
            })?
            .to_string();
        match element.as_str() {
            "Float" => self.lower_simd_unary(
                SimdUnaryKernel::SqrtFloat,
                input,
                "List OF Float",
                COLLECTION_TYPE_FLOAT,
                text,
            ),
            // Fixed sqrt is a genuine 2-lane NEON restoring sqrt (plan-01-simd §4.5).
            "Fixed" => self.lower_simd_sqrt_fixed(input, text),
            other => Err(format!(
                "math.sqrt array overload does not accept List OF {other}"
            )),
        }
    }

    /// Scalar `exp`/`log`/`log10`/`sin`/`cos`: `Float` runs the shared NEON
    /// kernel (bit-identical to the array overload); `Fixed` keeps the
    /// deterministic Q32.32 path. `log`/`log10` keep their `ErrFloatDomain`
    /// pre-check so the scalar error code is unchanged.
    fn lower_math_scalar_transcendental(
        &mut self,
        function: &str,
        arg: &NirValue,
    ) -> Result<ValueResult, String> {
        use super::builder_simd_float_math::FloatKernel;
        // The scalar kernel reads the operand's bits from a GPR, so materialize a
        // `d`-native float into one first (plan-01 float-dnative).
        let value = self.lower_value(arg)?;
        let value = self.materialize_float(value)?;
        match value.type_.as_str() {
            "Float" => {
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
            "Fixed" => self.lower_fixed_external_math(function, &[value]),
            other => Err(format!("math.{function} does not accept {other}")),
        }
    }

    /// Scalar `atan2`/`pow`: `Float` runs the shared NEON binary kernel
    /// (bit-identical to the array overload, no libm import); `Fixed` keeps the
    /// deterministic Q32.32 path. Mirrors `lower_external_math`'s arg spilling so
    /// a later argument's lowering cannot clobber the first.
    fn lower_math_scalar_binary(
        &mut self,
        function: &str,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        use super::builder_simd_float_math::FloatBinaryKernel;
        // The kernels read each operand's bits from a GPR slot, so materialize a
        // `d`-native float before spilling (plan-01 float-dnative).
        let left = self.lower_value(&args[0])?;
        let left = self.materialize_float(left)?;
        let left_slot = self.allocate_stack_object("scalar_binary_left", 8);
        self.emit(abi::store_u64(
            &left.location,
            abi::stack_pointer(),
            left_slot,
        ));
        let right = self.lower_value(&args[1])?;
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
        match left.type_.as_str() {
            "Float" if function == "pow" => {
                // pow is a scalar GPR+FP fdlibm kernel (not SIMD); it produces inf
                // on overflow and NaN for a negative base with a non-integer
                // exponent, which the result check turns into ErrFloatInf /
                // ErrFloatNan (matching the scalar pow man page).
                self.reset_temporary_registers();
                let left_reg = self.allocate_register()?;
                self.emit(abi::load_u64(&left_reg, abi::stack_pointer(), left_slot));
                let right_reg = self.allocate_register()?;
                self.emit(abi::load_u64(&right_reg, abi::stack_pointer(), right_slot));
                let result = self.emit_pow_scalar(&left_reg, &right_reg)?;
                self.emit_float_result_check(&result, FloatInfinityError::Infinity)?;
                Ok(ValueResult {
                    type_: "Float".to_string(),
                    location: result,
                    text,
                })
            }
            "Float" => {
                let kernel = match function {
                    "atan2" => FloatBinaryKernel::Atan2,
                    // `pow` never reaches here — the `"Float" if function == "pow"`
                    // arm above diverts it to the scalar fdlibm kernel.
                    _ => return Err(format!("math.{function} has no scalar Float binary kernel")),
                };
                self.reset_temporary_registers();
                let left_reg = self.allocate_register()?;
                self.emit(abi::load_u64(&left_reg, abi::stack_pointer(), left_slot));
                let right_reg = self.allocate_register()?;
                self.emit(abi::load_u64(&right_reg, abi::stack_pointer(), right_slot));
                self.lower_simd_float_binary_scalar(kernel, &left_reg, &right_reg, text)
            }
            "Fixed" => {
                self.reset_temporary_registers();
                let left_reg = self.allocate_register()?;
                self.emit(abi::load_u64(&left_reg, abi::stack_pointer(), left_slot));
                let right_reg = self.allocate_register()?;
                self.emit(abi::load_u64(&right_reg, abi::stack_pointer(), right_slot));
                let values = vec![
                    ValueResult {
                        type_: "Fixed".to_string(),
                        location: left_reg,
                        text: left.text,
                    },
                    ValueResult {
                        type_: "Fixed".to_string(),
                        location: right_reg,
                        text: right.text,
                    },
                ];
                self.lower_fixed_external_math(function, &values)
            }
            other => Err(format!("math.{function} does not accept {other}")),
        }
    }

    /// `math.atan2(y AS Float[], x AS Float[])` / `math.pow(base AS Float[],
    /// exponent AS Float[])` — vectorized binary Float kernels (§4.6).
    /// `ErrInvalidArgument` if the two lists differ in length.
    fn lower_math_atan2_pow_array(
        &mut self,
        function: &str,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        use super::builder_simd_float_math::FloatBinaryKernel;
        // Lower and spill each arg before lowering the next (a later arg's call
        // can clobber the earlier list pointer).
        let left = self.lower_value(&args[0])?;
        let left_slot = self.allocate_stack_object("simd_flb_left", 8);
        self.emit(abi::store_u64(
            &left.location,
            abi::stack_pointer(),
            left_slot,
        ));
        let right = self.lower_value(&args[1])?;
        let right_slot = self.allocate_stack_object("simd_flb_right", 8);
        self.emit(abi::store_u64(
            &right.location,
            abi::stack_pointer(),
            right_slot,
        ));
        if left.type_ != "List OF Float" || right.type_ != "List OF Float" {
            return Err(format!(
                "math.{function} array overload requires two List OF Float, got {} and {}",
                left.type_, right.type_
            ));
        }
        let text = format!("math.{function}({}, {})", left.text, right.text);
        // pow is a per-element scalar fdlibm kernel (strict <=1 ULP, negative-base
        // integer exponents); atan2 stays on the SIMD binary kernel.
        if function == "pow" {
            return self.lower_pow_array(left_slot, right_slot, text);
        }
        let kernel = match function {
            "atan2" => FloatBinaryKernel::Atan2,
            _ => return Err(format!("math.{function} has no binary array overload")),
        };
        self.lower_simd_float_binary(kernel, left_slot, right_slot, text)
    }

    /// `math.exp(values AS Float[]) AS Float[]` — NEON polynomial kernel
    /// (plan-01-simd §4.6).
    fn lower_math_exp_array(&mut self, arg: &NirValue) -> Result<ValueResult, String> {
        use super::builder_simd_float_math::FloatKernel;
        let input = self.lower_value(arg)?;
        let text = format!("math.exp({})", input.text);
        let element = input
            .type_
            .strip_prefix("List OF ")
            .ok_or_else(|| {
                format!(
                    "math.exp array overload requires a list, got {}",
                    input.type_
                )
            })?
            .to_string();
        match element.as_str() {
            "Float" => self.lower_simd_float_unary(FloatKernel::Exp, input, text),
            other => Err(format!(
                "math.exp array overload does not accept List OF {other}"
            )),
        }
    }

    /// `math.sin/cos/tan(values AS Float[]) AS Float[]` — NEON kernels (§4.6).
    fn lower_math_trig_array(
        &mut self,
        function: &str,
        arg: &NirValue,
    ) -> Result<ValueResult, String> {
        use super::builder_simd_float_math::FloatKernel;
        let input = self.lower_value(arg)?;
        let text = format!("math.{function}({})", input.text);
        let element = input
            .type_
            .strip_prefix("List OF ")
            .ok_or_else(|| {
                format!(
                    "math.{function} array overload requires a list, got {}",
                    input.type_
                )
            })?
            .to_string();
        let kernel = match (function, element.as_str()) {
            ("sin", "Float") => FloatKernel::Sin,
            ("cos", "Float") => FloatKernel::Cos,
            ("tan", "Float") => FloatKernel::Tan,
            ("atan", "Float") => FloatKernel::Atan,
            ("asin", "Float") => FloatKernel::Asin,
            ("acos", "Float") => FloatKernel::Acos,
            (_, other) => {
                return Err(format!(
                    "math.{function} array overload does not accept List OF {other}"
                ))
            }
        };
        self.lower_simd_float_unary(kernel, input, text)
    }

    /// `math.log/log10(values AS Fixed[]) AS Fixed[]` — per-lane scalar Q32.32
    /// (plan-01-simd §4.5). Float lists route to the Phase 5 kernels.
    fn lower_math_log_array(
        &mut self,
        function: &str,
        arg: &NirValue,
    ) -> Result<ValueResult, String> {
        let input = self.lower_value(arg)?;
        let text = format!("math.{function}({})", input.text);
        let element = input
            .type_
            .strip_prefix("List OF ")
            .ok_or_else(|| {
                format!(
                    "math.{function} array overload requires a list, got {}",
                    input.type_
                )
            })?
            .to_string();
        match element.as_str() {
            "Fixed" => self.lower_simd_log_fixed(input, function == "log10", text),
            "Float" => {
                use super::builder_simd_float_math::FloatKernel;
                let kernel = if function == "log10" {
                    FloatKernel::Log10
                } else {
                    FloatKernel::Log
                };
                self.lower_simd_float_unary(kernel, input, text)
            }
            other => Err(format!(
                "math.{function} array overload does not yet accept List OF {other}"
            )),
        }
    }

    /// `math.floor/ceil/round(values AS Float[]|Fixed[]) AS Integer[]` —
    /// vectorized rounding to a new `List OF Integer` (plan-01-simd §4.4).
    fn lower_math_rounding_array(
        &mut self,
        function: &str,
        arg: &NirValue,
    ) -> Result<ValueResult, String> {
        use super::builder_simd_math::SimdUnaryKernel;
        let input = self.lower_value(arg)?;
        let text = format!("math.{function}({})", input.text);
        let element = input
            .type_
            .strip_prefix("List OF ")
            .ok_or_else(|| {
                format!(
                    "math.{function} array overload requires a list, got {}",
                    input.type_
                )
            })?
            .to_string();
        let kernel = match (function, element.as_str()) {
            ("floor", "Float") => SimdUnaryKernel::FloorFloat,
            ("ceil", "Float") => SimdUnaryKernel::CeilFloat,
            ("round", "Float") => SimdUnaryKernel::RoundFloat,
            ("floor", "Fixed") => SimdUnaryKernel::FloorFixed,
            ("ceil", "Fixed") => SimdUnaryKernel::CeilFixed,
            ("round", "Fixed") => SimdUnaryKernel::RoundFixed,
            (_, other) => {
                return Err(format!(
                    "math.{function} array overload does not accept List OF {other}"
                ))
            }
        };
        self.lower_simd_unary(
            kernel,
            input,
            "List OF Integer",
            COLLECTION_TYPE_INTEGER,
            text,
        )
    }

    fn lower_math_abs(&mut self, arg: &NirValue) -> Result<ValueResult, String> {
        let value = self.lower_value(arg)?;
        let value = self.materialize_float(value)?;
        let dst = self.allocate_register()?;
        let bound = self.temporary_vreg();
        match value.type_.as_str() {
            // Money is a raw i64, so |x| and the INT64_MIN overflow check are the
            // same integer op as Integer/Fixed (plan-29-G §4.7).
            "Integer" | "Fixed" | "Money" => {
                let ok = self.label("math_abs_ok");
                self.emit(abi::compare_immediate(&value.location, "0"));
                self.emit(abi::branch_ge(&ok));
                self.emit(abi::move_immediate(
                    &bound,
                    "Integer",
                    "9223372036854775808",
                ));
                self.emit(abi::compare_registers(&value.location, &bound));
                self.emit(abi::branch_ne(&ok));
                self.emit_overflow_return()?;
                self.emit(abi::label(&ok));
                self.emit(abi::compare_immediate(&value.location, "0"));
                let done = self.label("math_abs_done");
                let negative = self.label("math_abs_negative");
                self.emit(abi::branch_lt(&negative));
                self.emit(abi::move_register(&dst, &value.location));
                self.emit(abi::branch(&done));
                self.emit(abi::label(&negative));
                self.emit(abi::subtract_registers(
                    dst.as_str(),
                    abi::ZERO,
                    &value.location,
                ));
                self.emit(abi::label(&done));
            }
            "Float" => {
                // Hardware `fabs` clears the sign bit in the FP domain (plan-02
                // §4). Bit-identical to the old GPR sign-mask AND for every finite
                // MFBASIC `Float`, but a single hardware instruction.
                self.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[0], &value.location));
                self.emit(abi::float_abs_d(abi::FP_SCRATCH[0], abi::FP_SCRATCH[0]));
                self.emit(abi::float_move_x_from_d(&dst, abi::FP_SCRATCH[0]));
            }
            other => return Err(format!("math.abs does not accept {other}")),
        }
        Ok(ValueResult {
            type_: value.type_,
            location: dst,
            text: format!("math.abs({})", value.text),
        })
    }

    /// Map a numeric element type name to its collection value-type code.
    fn numeric_element_type_code(element: &str) -> Option<usize> {
        match element {
            "Integer" => Some(COLLECTION_TYPE_INTEGER),
            "Float" => Some(COLLECTION_TYPE_FLOAT),
            "Fixed" => Some(COLLECTION_TYPE_FIXED),
            _ => None,
        }
    }

    /// `math.min/max(a AS T[], b AS T[]) AS T[]` — vectorized element-wise
    /// min/max (plan-01-simd §4.4). `ErrInvalidArgument` if lengths differ.
    fn lower_math_min_max_array(
        &mut self,
        function: &str,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        use super::builder_simd_math::SimdBinaryKernel;
        // Lower and spill each argument before lowering the next: a later arg's
        // lowering may emit a call that clobbers the caller-saved register holding
        // an earlier list pointer ([[arena-alloc-clobbers-x14-x15]] generalized).
        let left = self.lower_value(&args[0])?;
        let left_slot = self.allocate_stack_object("simd_minmax_left", 8);
        self.emit(abi::store_u64(
            &left.location,
            abi::stack_pointer(),
            left_slot,
        ));
        let right = self.lower_value(&args[1])?;
        let right_slot = self.allocate_stack_object("simd_minmax_right", 8);
        self.emit(abi::store_u64(
            &right.location,
            abi::stack_pointer(),
            right_slot,
        ));
        if left.type_ != right.type_ {
            return Err(format!(
                "math.{function} array overload requires matching list types, got {} and {}",
                left.type_, right.type_
            ));
        }
        let result_type = left.type_.clone();
        let element = result_type
            .strip_prefix("List OF ")
            .ok_or_else(|| format!("math.{function} array overload requires a list"))?
            .to_string();
        let code = Self::numeric_element_type_code(&element).ok_or_else(|| {
            format!("math.{function} array overload does not accept List OF {element}")
        })?;
        let kernel = match (function, element.as_str()) {
            ("min", "Float") => SimdBinaryKernel::MinFloat,
            ("max", "Float") => SimdBinaryKernel::MaxFloat,
            // Integer and raw Q32.32 Fixed are both signed-i64 lane compares.
            ("min", "Integer" | "Fixed") => SimdBinaryKernel::MinSigned,
            ("max", "Integer" | "Fixed") => SimdBinaryKernel::MaxSigned,
            _ => {
                return Err(format!(
                    "math.{function} array overload does not accept List OF {element}"
                ))
            }
        };
        let text = format!("math.{function}({}, {})", left.text, right.text);
        self.lower_simd_binary(kernel, left_slot, right_slot, &result_type, code, text)
    }

    /// `math.clamp(values AS T[], low AS T, high AS T) AS T[]` — vectorized clamp
    /// (plan-01-simd §4.4). Never errors.
    fn lower_math_clamp_array(&mut self, args: &[NirValue]) -> Result<ValueResult, String> {
        use super::builder_simd_math::SimdClampKernel;
        // Lower and spill each argument before lowering the next (see
        // lower_math_min_max_array): `low`/`high` may be call results that clobber
        // the register holding the input list pointer.
        let input = self.lower_value(&args[0])?;
        let result_type = input.type_.clone();
        let in_slot = self.allocate_stack_object("simd_clamp_in", 8);
        self.emit(abi::store_u64(
            &input.location,
            abi::stack_pointer(),
            in_slot,
        ));
        // Spill the bound's *bits* in a GPR. A `Float` bound may be `d`-native
        // (e.g. the negative literal `-1.0`, produced by `fneg` into an FP
        // register); storing that FP register with the integer `store_u64` leaks
        // it into a GP-context op — an encode failure on riscv (str_u64 cannot
        // take an FP register). `float_value_as_gpr` is the identity for a
        // GP-native value (Integer/Fixed bounds), so only Float bounds change.
        let low = self.lower_value(&args[1])?;
        let low_bits = self.float_value_as_gpr(&low)?;
        let low_slot = self.allocate_stack_object("simd_clamp_low", 8);
        self.emit(abi::store_u64(&low_bits, abi::stack_pointer(), low_slot));
        let high = self.lower_value(&args[2])?;
        let high_bits = self.float_value_as_gpr(&high)?;
        let high_slot = self.allocate_stack_object("simd_clamp_high", 8);
        self.emit(abi::store_u64(&high_bits, abi::stack_pointer(), high_slot));
        let element = result_type
            .strip_prefix("List OF ")
            .ok_or_else(|| "math.clamp array overload requires a list".to_string())?
            .to_string();
        let code = Self::numeric_element_type_code(&element).ok_or_else(|| {
            format!("math.clamp array overload does not accept List OF {element}")
        })?;
        let kernel = match element.as_str() {
            "Float" => SimdClampKernel::Float,
            "Integer" | "Fixed" => SimdClampKernel::Signed,
            other => {
                return Err(format!(
                    "math.clamp array overload does not accept List OF {other}"
                ))
            }
        };
        let text = format!("math.clamp({}, {}, {})", input.text, low.text, high.text);
        self.lower_simd_clamp(
            kernel,
            in_slot,
            low_slot,
            high_slot,
            &result_type,
            code,
            text,
        )
    }

    fn lower_math_min_max(
        &mut self,
        function: &str,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let left = self.lower_value(&args[0])?;
        let left = self.materialize_float(left)?;
        let left_slot = self.allocate_stack_object("math_minmax_left", 8);
        self.emit(abi::store_u64(
            &left.location,
            abi::stack_pointer(),
            left_slot,
        ));
        let right = self.lower_value(&args[1])?;
        let right = self.materialize_float(right)?;
        let right_slot = self.allocate_stack_object("math_minmax_right", 8);
        self.emit(abi::store_u64(
            &right.location,
            abi::stack_pointer(),
            right_slot,
        ));
        let dst = self.allocate_register()?;
        let lhs = self.allocate_register()?;
        let rhs = self.allocate_register()?;
        self.emit(abi::load_u64(&lhs, abi::stack_pointer(), left_slot));
        self.emit(abi::load_u64(&rhs, abi::stack_pointer(), right_slot));
        if left.type_ != right.type_ {
            return Err(format!(
                "math.{function} requires matching types, got {} and {}",
                left.type_, right.type_
            ));
        }
        match left.type_.as_str() {
            // Money min/max is the signed-i64 compare+select (plan-29-G §4.7).
            "Integer" | "Fixed" | "Money" => {
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
            "Float" => {
                // Hardware `fminnm`/`fmaxnm` (plan-02 §4): a single instruction in
                // place of the subtract/compare/branch. For the finite operands
                // MFBASIC produces this is bit-identical to the old ordered
                // compare that returned `lhs` on a tie.
                self.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[0], &lhs));
                self.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[1], &rhs));
                if function == "min" {
                    self.emit(abi::float_min_d(abi::FP_SCRATCH[0], abi::FP_SCRATCH[0], abi::FP_SCRATCH[1]));
                } else {
                    self.emit(abi::float_max_d(abi::FP_SCRATCH[0], abi::FP_SCRATCH[0], abi::FP_SCRATCH[1]));
                }
                self.emit(abi::float_move_x_from_d(&dst, abi::FP_SCRATCH[0]));
            }
            other => return Err(format!("math.{function} does not accept {other}")),
        }
        Ok(ValueResult {
            type_: left.type_,
            location: dst,
            text: format!("math.{function}({}, {})", left.text, right.text),
        })
    }

    fn lower_math_clamp(&mut self, args: &[NirValue]) -> Result<ValueResult, String> {
        let value = self.lower_value(&args[0])?;
        let value = self.materialize_float(value)?;
        let value_slot = self.allocate_stack_object("math_clamp_value", 8);
        self.emit(abi::store_u64(
            &value.location,
            abi::stack_pointer(),
            value_slot,
        ));
        let low = self.lower_value(&args[1])?;
        let low = self.materialize_float(low)?;
        let low_slot = self.allocate_stack_object("math_clamp_low", 8);
        self.emit(abi::store_u64(
            &low.location,
            abi::stack_pointer(),
            low_slot,
        ));
        let high = self.lower_value(&args[2])?;
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
        let dst = self.allocate_register()?;
        let value_reg = self.allocate_register()?;
        let low_reg = self.allocate_register()?;
        let high_reg = self.allocate_register()?;
        self.emit(abi::load_u64(&value_reg, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(&low_reg, abi::stack_pointer(), low_slot));
        self.emit(abi::load_u64(&high_reg, abi::stack_pointer(), high_slot));

        match value.type_.as_str() {
            // Money clamp is the signed-i64 bounds compare+select (plan-29-G §4.7).
            "Integer" | "Fixed" | "Money" => {
                let bounds_valid = self.label("math_clamp_bounds_valid");
                let take_low = self.label("math_clamp_take_low");
                let take_high = self.label("math_clamp_take_high");
                let done = self.label("math_clamp_done");
                self.emit(abi::compare_registers(&low_reg, &high_reg));
                self.emit(abi::branch_le(&bounds_valid));
                self.emit_invalid_argument_return()?;
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
            "Float" => {
                let bounds_valid = self.label("math_clamp_float_bounds_valid");
                let take_low = self.label("math_clamp_float_take_low");
                let take_high = self.label("math_clamp_float_take_high");
                let done = self.label("math_clamp_float_done");
                self.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[0], &low_reg));
                self.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[1], &high_reg));
                self.emit(abi::float_subtract_d(abi::FP_SCRATCH[2], abi::FP_SCRATCH[0], abi::FP_SCRATCH[1]));
                self.emit(abi::float_compare_zero_d(abi::FP_SCRATCH[2]));
                self.emit(abi::branch_le(&bounds_valid));
                self.emit_invalid_argument_return()?;
                self.emit(abi::label(&bounds_valid));
                self.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[0], &value_reg));
                self.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[1], &low_reg));
                self.emit(abi::float_subtract_d(abi::FP_SCRATCH[2], abi::FP_SCRATCH[0], abi::FP_SCRATCH[1]));
                self.emit(abi::float_compare_zero_d(abi::FP_SCRATCH[2]));
                self.emit(abi::branch_lt(&take_low));
                self.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[1], &high_reg));
                self.emit(abi::float_subtract_d(abi::FP_SCRATCH[2], abi::FP_SCRATCH[0], abi::FP_SCRATCH[1]));
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
            type_: value.type_,
            location: dst,
            text: format!("math.clamp({}, {}, {})", value.text, low.text, high.text),
        })
    }

    fn lower_math_rounding(
        &mut self,
        function: &str,
        arg: &NirValue,
    ) -> Result<ValueResult, String> {
        let value = self.lower_value(arg)?;
        let value = self.materialize_float(value)?;
        let dst = self.allocate_register()?;
        match value.type_.as_str() {
            "Float" => {
                self.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[0], &value.location));
                self.emit_float_rounding_integer_range_check(&value.location)?;
                match function {
                    "floor" => self.emit(abi::float_floor_to_signed_x(&dst, abi::FP_SCRATCH[0])),
                    "ceil" => self.emit(abi::float_ceil_to_signed_x(&dst, abi::FP_SCRATCH[0])),
                    "round" => self.emit(abi::float_round_to_signed_x(&dst, abi::FP_SCRATCH[0])),
                    _ => unreachable!(),
                }
            }
            "Fixed" => {
                // Deterministic raw Q32.32 rounding: the integer result of
                // rounding a Fixed always fits in `Integer` range (|real| < 2^31),
                // so no host floating-point conversion is required.
                self.emit_fixed_rounding_to_integer(function, &value.location, &dst)?;
            }
            // floor/ceil/round(Money) → the whole-unit count `raw / 100000`, a
            // fixed rule (round uses half-away-from-zero, not the global mode —
            // an explicit call is presentation-like) (plan-29-G §4.7).
            "Money" => {
                self.emit_money_rounding_to_integer(function, &value.location, &dst)?;
            }
            other => return Err(format!("math.{function} does not accept {other}")),
        }
        Ok(ValueResult {
            type_: "Integer".to_string(),
            location: dst,
            text: format!("math.{function}({})", value.text),
        })
    }

    /// floor/ceil/round of a Money raw to its whole-unit Integer count
    /// (plan-29-G §4.7). `q = raw / 100000` truncated toward zero, then adjusted:
    /// floor toward -∞, ceil toward +∞, round half-away-from-zero.
    fn emit_money_rounding_to_integer(
        &mut self,
        function: &str,
        raw: &str,
        dst: &str,
    ) -> Result<(), String> {
        let scale = self.allocate_register()?;
        let quotient = self.allocate_register()?;
        let remainder = self.allocate_register()?;
        self.emit(abi::move_immediate(&scale, "Integer", "100000"));
        self.emit(abi::signed_divide_registers(&quotient, raw, &scale));
        self.emit(abi::multiply_subtract_registers(&remainder, &quotient, &scale, raw));
        self.emit(abi::move_register(dst, &quotient));
        let done = self.label("math_money_round_done");
        match function {
            "floor" => {
                // remainder < 0 (raw negative, non-zero frac) → toward -∞.
                self.emit(abi::compare_immediate(&remainder, "0"));
                self.emit(abi::branch_ge(&done));
                self.emit(abi::subtract_immediate(dst, &quotient, 1));
            }
            "ceil" => {
                // remainder > 0 (raw positive, non-zero frac) → toward +∞.
                self.emit(abi::compare_immediate(&remainder, "0"));
                self.emit(abi::branch_le(&done));
                self.emit(abi::add_immediate(dst, &quotient, 1));
            }
            "round" => {
                // half-away: bump the magnitude when 2*|remainder| >= 100000.
                let abs_rem = self.allocate_register()?;
                let bump_pos = self.label("math_money_round_bump_pos");
                let bump_neg = self.label("math_money_round_bump_neg");
                let half = self.allocate_register()?;
                self.emit(abi::move_register(&abs_rem, &remainder));
                self.emit_abs_i64(&abs_rem)?;
                // 2*|rem| vs 100000: compare |rem| against 100000 - |rem|.
                self.emit(abi::move_immediate(&half, "Integer", "100000"));
                self.emit(abi::subtract_registers(&half, &half, &abs_rem));
                self.emit(abi::compare_registers(&abs_rem, &half));
                self.emit(abi::branch_lt(&done)); // below the half → keep quotient
                self.emit(abi::compare_immediate(&remainder, "0"));
                self.emit(abi::branch_lt(&bump_neg));
                self.emit(abi::label(&bump_pos));
                self.emit(abi::add_immediate(dst, &quotient, 1));
                self.emit(abi::branch(&done));
                self.emit(abi::label(&bump_neg));
                self.emit(abi::subtract_immediate(dst, &quotient, 1));
            }
            _ => unreachable!(),
        }
        self.emit(abi::label(&done));
        Ok(())
    }

    fn emit_float_rounding_integer_range_check(&mut self, source_bits: &str) -> Result<(), String> {
        let bits = self.allocate_register()?;
        let exponent = self.allocate_register()?;
        let sign = self.allocate_register()?;
        let mantissa = self.allocate_register()?;
        let mask = self.allocate_register()?;
        let ok = self.label("math_rounding_float_range_ok");
        let edge = self.label("math_rounding_float_range_edge");
        let edge_negative = self.label("math_rounding_float_range_edge_negative");
        let overflow = self.label("math_rounding_float_range_overflow");

        self.emit(abi::move_register(&bits, source_bits));
        self.emit(abi::shift_right_immediate(&exponent, &bits, 52));
        self.emit(abi::move_immediate(&mask, "Integer", "2047"));
        self.emit(abi::and_registers(&exponent, &exponent, &mask));
        self.emit(abi::compare_immediate(&exponent, "2047"));
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
        self.emit(abi::move_immediate(&mask, "Integer", "4503599627370495"));
        self.emit(abi::and_registers(&mantissa, &bits, &mask));
        self.emit(abi::compare_immediate(&mantissa, "0"));
        self.emit(abi::branch_eq(&ok));

        self.emit(abi::label(&overflow));
        self.emit_overflow_return()?;
        self.emit(abi::label(&ok));
        Ok(())
    }

    /// Emit a call to `_mfb_rng_next` (advance this thread's PCG64 generator and
    /// return a fresh 64-bit draw in the return register), with the matching
    /// internal call relocation. The call clobbers every caller-saved register.
    fn emit_rng_next_call(&mut self) {
        self.emit(abi::branch_link(RNG_NEXT_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: RNG_NEXT_SYMBOL.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
    }

    /// `math.rand(min, max)` — uniform inclusive integer in `[min, max]`, drawn
    /// from this thread's PCG64 generator. Reports `ErrInvalidArgument` when
    /// `min > max`.
    fn lower_math_rand(&mut self, args: &[NirValue]) -> Result<ValueResult, String> {
        let min = self.lower_value(&args[0])?;
        // `rand(Money, Money) → Money` draws uniformly over the raw i64 range, the
        // same Lemire sampling as Integer (the raws are i64) (plan-29-G §4.7).
        if !matches!(min.type_.as_str(), "Integer" | "Money") {
            return Err(format!("math.rand does not accept {}", min.type_));
        }
        let result_type = min.type_.clone();
        let min_slot = self.allocate_stack_object("math_rand_min", 8);
        self.emit(abi::store_u64(
            &min.location,
            abi::stack_pointer(),
            min_slot,
        ));
        let max = self.lower_value(&args[1])?;
        if max.type_ != result_type {
            return Err(format!("math.rand does not accept {}", max.type_));
        }
        let max_slot = self.allocate_stack_object("math_rand_max", 8);
        self.emit(abi::store_u64(
            &max.location,
            abi::stack_pointer(),
            max_slot,
        ));
        let range_slot = self.allocate_stack_object("math_rand_range", 8);
        // Lemire rejection threshold and the final result are spilled so they
        // survive the redraw loop's `_mfb_rng_next` calls (which clobber every
        // caller-saved register). Routing every cross-call live value through a
        // stack slot keeps the lowering correct under both register allocators.
        let threshold_slot = self.allocate_stack_object("math_rand_threshold", 8);
        let result_slot = self.allocate_stack_object("math_rand_result", 8);

        // Validate min <= max and compute the inclusive span before the draw
        // loop; `_mfb_rng_next` clobbers the caller-saved registers so the span,
        // threshold, and min are reloaded from their slots after every call.
        self.reset_temporary_registers();
        let min_reg = self.allocate_register()?;
        let max_reg = self.allocate_register()?;
        let range_reg = self.allocate_register()?;
        self.emit(abi::load_u64(&min_reg, abi::stack_pointer(), min_slot));
        self.emit(abi::load_u64(&max_reg, abi::stack_pointer(), max_slot));
        let bounds_valid = self.label("math_rand_bounds_valid");
        self.emit(abi::compare_registers(&min_reg, &max_reg));
        self.emit(abi::branch_le(&bounds_valid));
        self.emit_invalid_argument_return()?;
        self.emit(abi::label(&bounds_valid));
        // span = (max - min) + 1; wraps to 0 only for the full Integer range,
        // which the `full_range` branch handles by returning a single raw draw.
        self.emit(abi::subtract_registers(&range_reg, &max_reg, &min_reg));
        self.emit(abi::add_immediate(&range_reg, &range_reg, 1));
        self.emit(abi::store_u64(&range_reg, abi::stack_pointer(), range_slot));

        let full_range = self.label("math_rand_full_range");
        let draw = self.label("math_rand_draw");
        let maybe_reject = self.label("math_rand_maybe_reject");
        let accept = self.label("math_rand_accept");
        let done = self.label("math_rand_done");

        // The full Integer range (span == 0) needs no reduction: a single 64-bit
        // draw is already uniform. It is also the one case where `span` is zero,
        // so the threshold division below (which would `#DE`-trap on x86) is only
        // reached on the bounded path.
        self.emit(abi::compare_immediate(&range_reg, "0"));
        self.emit(abi::branch_eq(&full_range));

        // Lemire's rejection sampling for an unbiased inclusive [min, max]
        // (span = max - min + 1). For a draw `raw`, form the 128-bit product
        // m = raw * span: its high word `hi` is a candidate offset in [0, span)
        // and its low word `lo` decides fairness. `lo` values below the biased
        // tail width t = (2^64 mod span) must be redrawn. Compute
        //   t = (2^64 - span) mod span == (0 - span) mod span
        // once here while `span` is in a register (span != 0 on this path).
        let neg_span = self.allocate_register()?;
        let quotient = self.allocate_register()?;
        let threshold = self.allocate_register()?;
        self.emit(abi::subtract_registers(&neg_span, abi::ZERO, &range_reg));
        self.emit(abi::unsigned_divide_registers(
            &quotient, &neg_span, &range_reg,
        ));
        self.emit(abi::multiply_subtract_registers(
            &threshold,
            &quotient,
            &range_reg,
            &neg_span,
        ));
        self.emit(abi::store_u64(&threshold, abi::stack_pointer(), threshold_slot));

        // draw: raw = _mfb_rng_next(); hi = umulh(raw, span); lo = raw * span.
        // Accept when lo >= span (no draw could then fall in the tail); when
        // lo < span, redraw while lo < t, else accept `hi`.
        self.emit(abi::label(&draw));
        self.emit_rng_next_call();
        self.reset_temporary_registers();
        let span_reg = self.allocate_register()?;
        let product_hi = self.allocate_register()?;
        let product_lo = self.allocate_register()?;
        self.emit(abi::load_u64(&span_reg, abi::stack_pointer(), range_slot));
        self.emit(abi::unsigned_multiply_high_registers(
            &product_hi,
            abi::return_register(),
            &span_reg,
        ));
        self.emit(abi::multiply_registers(
            &product_lo,
            abi::return_register(),
            &span_reg,
        ));
        self.emit(abi::compare_registers(&product_lo, &span_reg));
        self.emit(abi::branch_lo(&maybe_reject)); // lo < span => check the tail
        self.emit(abi::branch(&accept));
        self.emit(abi::label(&maybe_reject));
        let threshold_reg = self.allocate_register()?;
        self.emit(abi::load_u64(
            &threshold_reg,
            abi::stack_pointer(),
            threshold_slot,
        ));
        self.emit(abi::compare_registers(&product_lo, &threshold_reg));
        self.emit(abi::branch_lo(&draw)); // lo < t => biased tail, redraw
        self.emit(abi::label(&accept));
        let min_reload = self.allocate_register()?;
        let offset = self.allocate_register()?;
        self.emit(abi::load_u64(&min_reload, abi::stack_pointer(), min_slot));
        self.emit(abi::add_registers(&offset, &min_reload, &product_hi));
        self.emit(abi::store_u64(&offset, abi::stack_pointer(), result_slot));
        self.emit(abi::branch(&done));

        // Full Integer range: a single raw draw is uniform over all of Integer.
        self.emit(abi::label(&full_range));
        self.emit_rng_next_call();
        self.emit(abi::store_u64(
            abi::return_register(),
            abi::stack_pointer(),
            result_slot,
        ));

        self.emit(abi::label(&done));
        self.reset_temporary_registers();
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: result_type,
            location: result,
            text: format!("math.rand({}, {})", min.text, max.text),
        })
    }

    /// `math.seed(value)` — reseed this thread's PCG64 generator. Returns Nothing.
    fn lower_math_seed(&mut self, arg: &NirValue) -> Result<ValueResult, String> {
        let value = self.lower_value(arg)?;
        if value.type_ != "Integer" {
            return Err(format!("math.seed does not accept {}", value.type_));
        }
        let text = format!("math.seed({})", value.text);
        self.emit(abi::move_register(abi::ARG[1], &value.location));
        self.emit(abi::move_register(
            abi::return_register(),
            ARENA_STATE_REGISTER,
        ));
        self.emit(abi::branch_link(RNG_SEED_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: RNG_SEED_SYMBOL.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
        Ok(ValueResult {
            type_: "Nothing".to_string(),
            location: abi::return_register().to_string(),
            text,
        })
    }

    fn lower_math_sqrt(&mut self, arg: &NirValue) -> Result<ValueResult, String> {
        let value = self.lower_value(arg)?;
        // Fixed keeps its GPR path (raw Q32.32 sqrt); only Float goes d-native.
        if value.type_ == "Float" {
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
            self.emit_float_domain_return()?;
            self.emit(abi::label(&valid));
            let result = self.allocate_fp_register()?;
            self.emit(abi::float_sqrt_d(&result, &src));
            return Ok(ValueResult {
                type_: "Float".to_string(),
                location: result,
                text,
            });
        }
        let value = self.materialize_float(value)?;
        match value.type_.as_str() {
            "Float" => unreachable!("Float handled above"),
            "Fixed" => {
                self.emit(abi::compare_immediate(&value.location, "0"));
                let valid = self.label("math_fixed_sqrt_valid");
                self.emit(abi::branch_ge(&valid));
                self.emit_invalid_argument_return()?;
                self.emit(abi::label(&valid));
                // Deterministic raw Q32.32 square root (no host floating-point).
                let dst = self.emit_fixed_sqrt(&value.location)?;
                Ok(ValueResult {
                    type_: "Fixed".to_string(),
                    location: dst,
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
                let one = self.allocate_register()?;
                self.emit(abi::move_immediate(
                    &one,
                    "Fixed",
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
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), slot));
        Ok(ValueResult {
            type_: "Fixed".to_string(),
            location: result,
            text,
        })
    }

    pub(super) fn emit_float_result_check(
        &mut self,
        bits: &str,
        infinity_error: FloatInfinityError,
    ) -> Result<(), String> {
        // A finite f64 has a biased exponent below 0x7FF. Drop the sign bit by
        // shifting left one — the remaining magnitude pattern orders the three
        // classes under a single unsigned compare: every finite value is
        // strictly below `+inf << 1` (0xFFE0000000000000), `+inf`/`-inf` land
        // exactly on it, and every NaN is above it. This replaces the old
        // exponent+mantissa mask pair (two 64-bit immediates, two `and`s, two
        // scratch registers) with one shift, one immediate, and one compare, and
        // it never hardcodes a result scratch register (see the negation note in
        // `lower_numeric_unary_negation`).
        let magnitude = self.allocate_register()?;
        let inf_bits = self.temporary_vreg();
        let ok = self.label("float_result_finite");
        let nan = self.label("float_result_nan");
        self.emit(abi::shift_left_immediate(&magnitude, bits, 1));
        // 0xFFE0000000000000 == (+inf bits) << 1.
        self.emit(abi::move_immediate(
            &inf_bits,
            "Integer",
            "18437736874454810624",
        ));
        self.emit(abi::compare_registers(&magnitude, &inf_bits));
        self.emit(abi::branch_lo(&ok)); // unsigned < +inf<<1 => finite
        self.emit(abi::branch_hi(&nan)); // unsigned > +inf<<1 => NaN
                                         // Equal => the value is exactly ±inf.
        match infinity_error {
            FloatInfinityError::Infinity => self.emit_float_inf_return()?,
            FloatInfinityError::Overflow => self.emit_float_overflow_return()?,
        }
        self.emit(abi::label(&nan));
        self.emit_float_nan_return()?;
        self.emit(abi::label(&ok));
        Ok(())
    }

    /// FP-domain twin of [`Self::emit_float_result_check`] (plan-16 Piece B). It
    /// computes the **same** three-way finite/inf/NaN predicate, but on the value
    /// while it is still resident in a `d`-register, so the result never has to be
    /// shuttled into a GPR just to be bit-tested. `value` is a `d`-register; it is
    /// read-only here (the `fabs` lands in a scratch FP register), so a chained
    /// float op keeps using it directly.
    ///
    /// `fabs` folds ±Inf onto +Inf, then a single `fcmp` against +Inf orders the
    /// three classes exactly as the integer magnitude compare did: a finite `|x|`
    /// is strictly less than +Inf, ±Inf compares equal, and a NaN compares
    /// unordered (the V flag, caught by `b.vs`). The error each class raises — and
    /// the line/char it stamps — is byte-identical to the GPR path.
    pub(super) fn emit_float_result_check_fp(
        &mut self,
        value: &str,
        infinity_error: FloatInfinityError,
    ) -> Result<(), String> {
        let nan = self.label("float_result_nan");
        let ok = self.label("float_result_finite");
        let magnitude = self.allocate_fp_register()?;
        let positive_inf = self.allocate_fp_register()?;
        let inf_bits = self.temporary_vreg();
        // +inf bits == 0x7FF0000000000000. Materialized through a scratch vreg,
        // never a pooled physical GPR.
        self.emit(abi::move_immediate(
            &inf_bits,
            "Integer",
            "9218868437227405312",
        ));
        self.emit(abi::float_move_d_from_x(&positive_inf, &inf_bits));
        self.emit(abi::float_abs_d(&magnitude, value));
        self.emit(abi::float_compare_d(&magnitude, &positive_inf));
        self.emit(abi::branch_vs(&nan)); // unordered => NaN
        self.emit(abi::branch_ne(&ok)); // |x| < +inf (ordered, not equal) => finite
                                        // Fall-through: |x| == +inf, i.e. the value is exactly ±inf.
        match infinity_error {
            FloatInfinityError::Infinity => self.emit_float_inf_return()?,
            FloatInfinityError::Overflow => self.emit_float_overflow_return()?,
        }
        self.emit(abi::label(&nan));
        self.emit_float_nan_return()?;
        self.emit(abi::label(&ok));
        Ok(())
    }

    /// Observation-boundary finiteness check (plan-17). A `Float` becomes
    /// user-accessible — bound to a named local/global, stored into a
    /// collection element or record field, returned, printed/converted, or
    /// passed as an argument — at a handful of NIR sites; this is the single
    /// choke point each calls so a non-finite traps *there*, at the boundary's
    /// `line:char`, rather than after every arithmetic op.
    ///
    /// Only a fresh float-arithmetic result can be non-finite: the `+ - * /
    /// MOD ^` operators (and unary negation, which propagates a non-finite
    /// operand) are the sole producers — a constant, a read of an
    /// already-finite binding/element/field, or a call result (the callee
    /// checked its own boundary) is finite by the boundary invariant. So the
    /// check is emitted only when `value` is such an arithmetic node, which
    /// also leaves comparisons untouched (they are IEEE, never a boundary) and
    /// lets a transient that recovers to finite — `1.0 / (1e200 * 1e200)` →
    /// `+0.0` — flow through with no trap.
    ///
    /// An escaping `±Inf` raises `ErrFloatOverflow` (the spec's "arithmetic
    /// overflow to infinity", byte-identical to the old per-op arithmetic
    /// check); a `NaN` raises `ErrFloatNaN`.
    pub(super) fn observe_float(
        &mut self,
        value: &NirValue,
        result: &ValueResult,
    ) -> Result<(), String> {
        if result.type_ == "Float" && float_arith_node(value) {
            let saved = self.current_loc;
            if let Some(loc) = super::builder_values::value_loc(value) {
                self.current_loc = loc;
            }
            // A `d`-native result is checked in the FP domain (plan-01
            // float-dnative): the value is still resident in its FP register, so
            // the boundary check reads it there and the store-back stays a plain
            // `str d` with no GPR shuttle — exactly the promoted-accumulator path
            // (plan-16/17). A GP-native result is checked on its bit pattern.
            let outcome = if Self::float_is_dnative(result) {
                self.emit_float_result_check_fp(&result.location, FloatInfinityError::Overflow)
            } else {
                self.emit_float_result_check(&result.location, FloatInfinityError::Overflow)
            };
            self.current_loc = saved;
            outcome?;
        }
        Ok(())
    }

    /// Observation-boundary check for a loop-promoted float accumulator whose
    /// value lives in `d` after an assignment update (plan-17). Uses the
    /// FP-domain variant so the store-back stays peephole-foldable, and fires
    /// only when the assigned `value` is a fresh arithmetic node.
    pub(super) fn observe_promoted_float(
        &mut self,
        value: &NirValue,
        d: &str,
    ) -> Result<(), String> {
        if float_arith_node(value) {
            let saved = self.current_loc;
            if let Some(loc) = super::builder_values::value_loc(value) {
                self.current_loc = loc;
            }
            let outcome = self.emit_float_result_check_fp(d, FloatInfinityError::Overflow);
            self.current_loc = saved;
            outcome?;
        }
        Ok(())
    }
}

/// Whether lowering `value` can yield a non-finite `Float`. Only the float
/// arithmetic operators produce `NaN`/`Inf`, so a `Binary` (any operator — the
/// `observe_float` type guard skips the Boolean-result comparisons) or a
/// `Unary` (negation propagates a non-finite operand) is the only node a
/// boundary must re-check; every other node is finite by construction
/// (plan-17).
pub(super) fn float_arith_node(value: &NirValue) -> bool {
    matches!(value, NirValue::Binary { .. } | NirValue::Unary { .. })
}
