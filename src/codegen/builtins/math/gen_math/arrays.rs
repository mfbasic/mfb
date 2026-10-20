//! plan-176-S: `math` over numeric lists — the element-wise array kernels. `lower_math_call` and
//! the float result checks live in `builtins/math/gen_math/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// Map a numeric element type to its collection value-type code.
    fn numeric_element_type_code(element: &ParameterType) -> Option<usize> {
        match element {
            ParameterType::Integer => Some(COLLECTION_TYPE_INTEGER),
            ParameterType::Float => Some(COLLECTION_TYPE_FLOAT),
            ParameterType::Fixed => Some(COLLECTION_TYPE_FIXED),
            _ => None,
        }
    }

    /// The element type of an array-overload argument, or the shared
    /// `requires a list, got {}` diagnostic (bug-332 D2). Standardizes the wording
    /// that had drifted across the eight `math.*` array overloads.
    ///
    /// plan-106-E: reads the `ListOf` variant instead of stripping its spelling.
    /// The diagnostic still names the type, which is what a diagnostic is for.
    fn list_element_type(
        input_type: &ParameterType,
        function: &str,
    ) -> Result<ParameterType, String> {
        match input_type {
            ParameterType::ListOf(element) => Ok((**element).clone()),
            _ => Err(format!(
                "math.{function} array overload requires a list, got {input_type}"
            )),
        }
    }

    /// `math.abs(values AS T[])` — vectorized absolute value (plan-01-simd §4.4).
    pub(super) fn lower_math_abs_array(
        &mut self,
        arg: &ValueResult,
    ) -> Result<ValueResult, String> {
        use crate::codegen::builtins::vector::builder_simd_math::SimdUnaryKernel;
        let input = arg.clone();
        let text = format!("math.abs({})", input.text);
        let element = Self::list_element_type(&input.type_, "abs")?;
        match element {
            ParameterType::Integer => self.lower_simd_unary(
                SimdUnaryKernel::AbsInteger,
                input,
                &ParameterType::ListOf(Box::new(ParameterType::Integer)),
                COLLECTION_TYPE_INTEGER,
                text,
            ),
            // Fixed is a raw Q32.32 i64; |x| and the INT64_MIN overflow check are
            // the same integer operation as Integer.
            ParameterType::Fixed => self.lower_simd_unary(
                SimdUnaryKernel::AbsInteger,
                input,
                &ParameterType::ListOf(Box::new(ParameterType::Fixed)),
                COLLECTION_TYPE_FIXED,
                text,
            ),
            ParameterType::Float => self.lower_simd_unary(
                SimdUnaryKernel::AbsFloat,
                input,
                &ParameterType::ListOf(Box::new(ParameterType::Float)),
                COLLECTION_TYPE_FLOAT,
                text,
            ),
            other => Err(format!(
                "math.abs array overload does not accept List OF {other}"
            )),
        }
    }

    /// `math.sqrt(values AS Float[])` — vectorized square root (plan-01-simd
    /// §4.4). `ErrFloatDomain` if any lane is negative (bug-617: this said
    /// `ErrInvalidArgument`, which is the FIXED form's error, not this one's).
    pub(super) fn lower_math_sqrt_array(
        &mut self,
        arg: &ValueResult,
    ) -> Result<ValueResult, String> {
        use crate::codegen::builtins::vector::builder_simd_math::SimdUnaryKernel;
        let input = arg.clone();
        let text = format!("math.sqrt({})", input.text);
        let element = Self::list_element_type(&input.type_, "sqrt")?;
        match element {
            ParameterType::Float => self.lower_simd_unary(
                SimdUnaryKernel::SqrtFloat,
                input,
                &ParameterType::ListOf(Box::new(ParameterType::Float)),
                COLLECTION_TYPE_FLOAT,
                text,
            ),
            // Fixed sqrt is a genuine 2-lane NEON restoring sqrt (plan-01-simd §4.5).
            ParameterType::Fixed => self.lower_simd_sqrt_fixed(input, text),
            other => Err(format!(
                "math.sqrt array overload does not accept List OF {other}"
            )),
        }
    }

    /// `math.atan2(y AS Float[], x AS Float[])` / `math.pow(base AS Float[],
    /// exponent AS Float[])` — vectorized binary Float kernels (§4.6).
    /// `ErrInvalidArgument` if the two lists differ in length.
    pub(super) fn lower_math_atan2_pow_array(
        &mut self,
        function: &str,
        args: &[ValueResult],
    ) -> Result<ValueResult, String> {
        use crate::codegen::builtins::vector::builder_simd_float_math::FloatBinaryKernel;
        // Lower and spill each arg before lowering the next (a later arg's call
        // can clobber the earlier list pointer).
        let left = args[0].clone();
        let left_slot = self.allocate_stack_object("simd_flb_left", 8);
        self.emit(abi::store_u64(
            &left.location,
            abi::stack_pointer(),
            left_slot,
        ));
        let right = args[1].clone();
        let right_slot = self.allocate_stack_object("simd_flb_right", 8);
        self.emit(abi::store_u64(
            &right.location,
            abi::stack_pointer(),
            right_slot,
        ));
        if left.type_.name() != "List OF Float" || right.type_.name() != "List OF Float" {
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
    pub(super) fn lower_math_exp_array(
        &mut self,
        arg: &ValueResult,
    ) -> Result<ValueResult, String> {
        use crate::codegen::builtins::vector::builder_simd_float_math::FloatKernel;
        let input = arg.clone();
        let text = format!("math.exp({})", input.text);
        let element = Self::list_element_type(&input.type_, "exp")?;
        match element {
            ParameterType::Float => self.lower_simd_float_unary(FloatKernel::Exp, input, text),
            other => Err(format!(
                "math.exp array overload does not accept List OF {other}"
            )),
        }
    }

    /// `math.sin/cos/tan(values AS Float[]) AS Float[]` — NEON kernels (§4.6).
    pub(super) fn lower_math_trig_array(
        &mut self,
        function: &str,
        arg: &ValueResult,
    ) -> Result<ValueResult, String> {
        use crate::codegen::builtins::vector::builder_simd_float_math::FloatKernel;
        let input = arg.clone();
        let text = format!("math.{function}({})", input.text);
        let element = Self::list_element_type(&input.type_, function)?;
        let kernel = match (function, &element) {
            ("sin", ParameterType::Float) => FloatKernel::Sin,
            ("cos", ParameterType::Float) => FloatKernel::Cos,
            ("tan", ParameterType::Float) => FloatKernel::Tan,
            ("atan", ParameterType::Float) => FloatKernel::Atan,
            ("asin", ParameterType::Float) => FloatKernel::Asin,
            ("acos", ParameterType::Float) => FloatKernel::Acos,
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
    pub(super) fn lower_math_log_array(
        &mut self,
        function: &str,
        arg: &ValueResult,
    ) -> Result<ValueResult, String> {
        let input = arg.clone();
        let text = format!("math.{function}({})", input.text);
        let element = Self::list_element_type(&input.type_, function)?;
        match element {
            ParameterType::Fixed => self.lower_simd_log_fixed(input, function == "log10", text),
            ParameterType::Float => {
                use crate::codegen::builtins::vector::builder_simd_float_math::FloatKernel;
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
    pub(super) fn lower_math_rounding_array(
        &mut self,
        function: &str,
        arg: &ValueResult,
    ) -> Result<ValueResult, String> {
        use crate::codegen::builtins::vector::builder_simd_math::SimdUnaryKernel;
        let input = arg.clone();
        let text = format!("math.{function}({})", input.text);
        let element = Self::list_element_type(&input.type_, function)?;
        let kernel = match (function, &element) {
            ("floor", ParameterType::Float) => SimdUnaryKernel::FloorFloat,
            ("ceil", ParameterType::Float) => SimdUnaryKernel::CeilFloat,
            ("round", ParameterType::Float) => SimdUnaryKernel::RoundFloat,
            ("floor", ParameterType::Fixed) => SimdUnaryKernel::FloorFixed,
            ("ceil", ParameterType::Fixed) => SimdUnaryKernel::CeilFixed,
            ("round", ParameterType::Fixed) => SimdUnaryKernel::RoundFixed,
            (_, other) => {
                return Err(format!(
                    "math.{function} array overload does not accept List OF {other}"
                ))
            }
        };
        self.lower_simd_unary(
            kernel,
            input,
            &ParameterType::ListOf(Box::new(ParameterType::Integer)),
            COLLECTION_TYPE_INTEGER,
            text,
        )
    }

    /// `math.min/max(a AS T[], b AS T[]) AS T[]` — vectorized element-wise
    /// min/max (plan-01-simd §4.4). `ErrInvalidArgument` if lengths differ.
    pub(super) fn lower_math_min_max_array(
        &mut self,
        function: &str,
        args: &[ValueResult],
    ) -> Result<ValueResult, String> {
        use crate::codegen::builtins::vector::builder_simd_math::SimdBinaryKernel;
        // Lower and spill each argument before lowering the next: a later arg's
        // lowering may emit a call that clobbers the caller-saved register holding
        // an earlier list pointer ([[arena-alloc-clobbers-x14-x15]] generalized).
        let left = args[0].clone();
        let left_slot = self.allocate_stack_object("simd_minmax_left", 8);
        self.emit(abi::store_u64(
            &left.location,
            abi::stack_pointer(),
            left_slot,
        ));
        let right = args[1].clone();
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
        let element = Self::list_element_type(&result_type, function)?;
        let code = Self::numeric_element_type_code(&element).ok_or_else(|| {
            format!("math.{function} array overload does not accept List OF {element}")
        })?;
        let kernel = match (function, &element) {
            ("min", ParameterType::Float) => SimdBinaryKernel::MinFloat,
            ("max", ParameterType::Float) => SimdBinaryKernel::MaxFloat,
            // Integer and raw Q32.32 Fixed are both signed-i64 lane compares.
            ("min", ParameterType::Integer | ParameterType::Fixed) => SimdBinaryKernel::MinSigned,
            ("max", ParameterType::Integer | ParameterType::Fixed) => SimdBinaryKernel::MaxSigned,
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
    pub(super) fn lower_math_clamp_array(
        &mut self,
        args: &[ValueResult],
    ) -> Result<ValueResult, String> {
        use crate::codegen::builtins::vector::builder_simd_math::SimdClampKernel;
        // Lower and spill each argument before lowering the next (see
        // lower_math_min_max_array): `low`/`high` may be call results that clobber
        // the register holding the input list pointer.
        let input = args[0].clone();
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
        let low = args[1].clone();
        let low_bits = self.float_value_as_gpr(&low)?;
        let low_slot = self.allocate_stack_object("simd_clamp_low", 8);
        self.emit(abi::store_u64(&low_bits, abi::stack_pointer(), low_slot));
        let high = args[2].clone();
        let high_bits = self.float_value_as_gpr(&high)?;
        let high_slot = self.allocate_stack_object("simd_clamp_high", 8);
        self.emit(abi::store_u64(&high_bits, abi::stack_pointer(), high_slot));
        let element = Self::list_element_type(&result_type, "clamp")?;
        let code = Self::numeric_element_type_code(&element).ok_or_else(|| {
            format!("math.clamp array overload does not accept List OF {element}")
        })?;
        let kernel = match element {
            ParameterType::Float => SimdClampKernel::Float,
            ParameterType::Integer | ParameterType::Fixed => SimdClampKernel::Signed,
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
}
