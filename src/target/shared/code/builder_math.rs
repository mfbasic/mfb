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
            "min" | "max" if args.len() == 2 => self.lower_math_min_max(function, args),
            "clamp" if args.len() == 3 => self.lower_math_clamp(args),
            "floor" | "ceil" | "round" if args.len() == 1 => {
                self.lower_math_rounding(function, &args[0])
            }
            "rand" if args.len() == 2 => self.lower_math_rand(args),
            "seed" if args.len() == 1 => self.lower_math_seed(&args[0]),
            "sqrt" if args.len() == 1 => self.lower_math_sqrt(&args[0]),
            "pow" if args.len() == 2 => self.lower_external_math(function, args),
            "atan2" if args.len() == 2 => self.lower_external_math(function, args),
            "exp" | "log" | "log10" | "sin" | "cos" | "tan" | "asin" | "acos" | "atan"
                if args.len() == 1 =>
            {
                self.lower_external_math(function, args)
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
            .ok_or_else(|| format!("math.abs array overload requires a list, got {}", input.type_))?
            .to_string();
        match element.as_str() {
            "Integer" => self.lower_simd_unary(
                SimdUnaryKernel::AbsInteger,
                input,
                "List OF Integer",
                COLLECTION_TYPE_INTEGER,
                text,
            ),
            other => Err(format!("math.abs array overload does not accept List OF {other}")),
        }
    }

    fn lower_math_abs(&mut self, arg: &NirValue) -> Result<ValueResult, String> {
        let value = self.lower_value(arg)?;
        let dst = self.allocate_register()?;
        match value.type_.as_str() {
            "Integer" | "Fixed" => {
                let ok = self.label("math_abs_ok");
                self.emit(abi::compare_immediate(&value.location, "0"));
                self.emit(abi::branch_ge(&ok));
                self.emit(abi::move_immediate("x17", "Integer", "9223372036854775808"));
                self.emit(abi::compare_registers(&value.location, "x17"));
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
                    "xzr",
                    &value.location,
                ));
                self.emit(abi::label(&done));
            }
            "Float" => {
                self.emit(abi::move_immediate("x17", "Integer", "9223372036854775807"));
                self.emit(abi::and_registers(&dst, &value.location, "x17"));
            }
            other => return Err(format!("math.abs does not accept {other}")),
        }
        Ok(ValueResult {
            type_: value.type_,
            location: dst,
            text: format!("math.abs({})", value.text),
        })
    }

    fn lower_math_min_max(
        &mut self,
        function: &str,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let left = self.lower_value(&args[0])?;
        let left_slot = self.allocate_stack_object("math_minmax_left", 8);
        self.emit(abi::store_u64(
            &left.location,
            abi::stack_pointer(),
            left_slot,
        ));
        let right = self.lower_value(&args[1])?;
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
            "Integer" | "Fixed" => {
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
                self.emit(abi::float_move_d_from_x("d0", &lhs));
                self.emit(abi::float_move_d_from_x("d1", &rhs));
                self.emit(abi::float_subtract_d("d2", "d0", "d1"));
                self.emit(abi::float_compare_zero_d("d2"));
                let take_left = self.label("math_minmax_float_take_left");
                let done = self.label("math_minmax_float_done");
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
        let value_slot = self.allocate_stack_object("math_clamp_value", 8);
        self.emit(abi::store_u64(
            &value.location,
            abi::stack_pointer(),
            value_slot,
        ));
        let low = self.lower_value(&args[1])?;
        let low_slot = self.allocate_stack_object("math_clamp_low", 8);
        self.emit(abi::store_u64(
            &low.location,
            abi::stack_pointer(),
            low_slot,
        ));
        let high = self.lower_value(&args[2])?;
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
            "Integer" | "Fixed" => {
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
                self.emit(abi::float_move_d_from_x("d0", &low_reg));
                self.emit(abi::float_move_d_from_x("d1", &high_reg));
                self.emit(abi::float_subtract_d("d2", "d0", "d1"));
                self.emit(abi::float_compare_zero_d("d2"));
                self.emit(abi::branch_le(&bounds_valid));
                self.emit_invalid_argument_return()?;
                self.emit(abi::label(&bounds_valid));
                self.emit(abi::float_move_d_from_x("d0", &value_reg));
                self.emit(abi::float_move_d_from_x("d1", &low_reg));
                self.emit(abi::float_subtract_d("d2", "d0", "d1"));
                self.emit(abi::float_compare_zero_d("d2"));
                self.emit(abi::branch_lt(&take_low));
                self.emit(abi::float_move_d_from_x("d1", &high_reg));
                self.emit(abi::float_subtract_d("d2", "d0", "d1"));
                self.emit(abi::float_compare_zero_d("d2"));
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
        let dst = self.allocate_register()?;
        match value.type_.as_str() {
            "Float" => {
                self.emit(abi::float_move_d_from_x("d0", &value.location));
                self.emit_float_rounding_integer_range_check(&value.location)?;
                match function {
                    "floor" => self.emit(abi::float_floor_to_signed_x(&dst, "d0")),
                    "ceil" => self.emit(abi::float_ceil_to_signed_x(&dst, "d0")),
                    "round" => self.emit(abi::float_round_to_signed_x(&dst, "d0")),
                    _ => unreachable!(),
                }
            }
            "Fixed" => {
                // Deterministic raw Q32.32 rounding: the integer result of
                // rounding a Fixed always fits in `Integer` range (|real| < 2^31),
                // so no host floating-point conversion is required.
                self.emit_fixed_rounding_to_integer(function, &value.location, &dst)?;
            }
            other => return Err(format!("math.{function} does not accept {other}")),
        }
        Ok(ValueResult {
            type_: "Integer".to_string(),
            location: dst,
            text: format!("math.{function}({})", value.text),
        })
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

    /// `math.rand(min, max)` — uniform inclusive integer in `[min, max]`, drawn
    /// from this thread's PCG64 generator. Reports `ErrInvalidArgument` when
    /// `min > max`.
    fn lower_math_rand(&mut self, args: &[NirValue]) -> Result<ValueResult, String> {
        let min = self.lower_value(&args[0])?;
        if min.type_ != "Integer" {
            return Err(format!("math.rand does not accept {}", min.type_));
        }
        let min_slot = self.allocate_stack_object("math_rand_min", 8);
        self.emit(abi::store_u64(
            &min.location,
            abi::stack_pointer(),
            min_slot,
        ));
        let max = self.lower_value(&args[1])?;
        if max.type_ != "Integer" {
            return Err(format!("math.rand does not accept {}", max.type_));
        }
        let max_slot = self.allocate_stack_object("math_rand_max", 8);
        self.emit(abi::store_u64(
            &max.location,
            abi::stack_pointer(),
            max_slot,
        ));
        let range_slot = self.allocate_stack_object("math_rand_range", 8);

        // Validate min <= max and compute the inclusive span before the call;
        // `_mfb_rng_next` clobbers the caller-saved registers so the span is
        // spilled and reloaded afterwards.
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
        // which the `full_range` branch handles by returning the raw draw.
        self.emit(abi::subtract_registers(&range_reg, &max_reg, &min_reg));
        self.emit(abi::add_immediate(&range_reg, &range_reg, 1));
        self.emit(abi::store_u64(&range_reg, abi::stack_pointer(), range_slot));

        self.emit(abi::branch_link(RNG_NEXT_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: RNG_NEXT_SYMBOL.to_string(),
            kind: "branch26".to_string(),
            binding: "internal".to_string(),
            library: None,
        });

        self.reset_temporary_registers();
        let result = self.allocate_register()?;
        let range_reg = self.allocate_register()?;
        let min_reg = self.allocate_register()?;
        let quotient = self.allocate_register()?;
        let remainder = self.allocate_register()?;
        self.emit(abi::load_u64(&range_reg, abi::stack_pointer(), range_slot));
        self.emit(abi::load_u64(&min_reg, abi::stack_pointer(), min_slot));
        let full_range = self.label("math_rand_full_range");
        let done = self.label("math_rand_done");
        self.emit(abi::compare_immediate(&range_reg, "0"));
        self.emit(abi::branch_eq(&full_range));
        // remainder = raw - (raw / span) * span  (unsigned modulo)
        self.emit(abi::unsigned_divide_registers(
            &quotient,
            abi::return_register(),
            &range_reg,
        ));
        self.emit(abi::multiply_subtract_registers(
            &remainder,
            &quotient,
            &range_reg,
            abi::return_register(),
        ));
        self.emit(abi::add_registers(&result, &min_reg, &remainder));
        self.emit(abi::branch(&done));
        self.emit(abi::label(&full_range));
        self.emit(abi::move_register(&result, abi::return_register()));
        self.emit(abi::label(&done));
        Ok(ValueResult {
            type_: "Integer".to_string(),
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
        self.emit(abi::move_register("x1", &value.location));
        self.emit(abi::move_register(
            abi::return_register(),
            ARENA_STATE_REGISTER,
        ));
        self.emit(abi::branch_link(RNG_SEED_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: RNG_SEED_SYMBOL.to_string(),
            kind: "branch26".to_string(),
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
        match value.type_.as_str() {
            "Float" => {
                let dst = self.allocate_register()?;
                self.emit(abi::float_move_d_from_x("d0", &value.location));
                self.emit(abi::float_compare_zero_d("d0"));
                let valid = self.label("math_sqrt_valid");
                self.emit(abi::branch_ge(&valid));
                self.emit_float_domain_return()?;
                self.emit(abi::label(&valid));
                self.emit(abi::float_sqrt_d("d0", "d0"));
                self.emit(abi::float_move_x_from_d(&dst, "d0"));
                Ok(ValueResult {
                    type_: "Float".to_string(),
                    location: dst,
                    text: format!("math.sqrt({})", value.text),
                })
            }
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

    fn lower_external_math(
        &mut self,
        function: &str,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        // Lower each argument and spill it to its own stack slot immediately.
        // Lowering a later argument can reset the temporary register file (e.g.
        // `toFixed`), which would otherwise clobber an earlier argument still
        // held only in a register.
        let mut slots = Vec::with_capacity(args.len());
        let mut types = Vec::with_capacity(args.len());
        let mut texts = Vec::with_capacity(args.len());
        for arg in args {
            let value = self.lower_value(arg)?;
            let slot = self.allocate_stack_object("math_arg", 8);
            self.emit(abi::store_u64(&value.location, abi::stack_pointer(), slot));
            slots.push(slot);
            types.push(value.type_);
            texts.push(value.text);
        }
        let Some(first_type) = types.first().cloned() else {
            return Err(format!("math.{function} expects at least one argument"));
        };
        if !matches!(first_type.as_str(), "Float" | "Fixed") {
            return Err(format!("math.{function} does not accept {first_type}"));
        }
        if types.iter().any(|type_| type_ != &first_type) {
            return Err(format!("math.{function} requires matching argument types"));
        }
        // `Fixed` overloads use deterministic compiler-owned Q32.32 paths rather
        // than the platform libm, which is non-deterministic across targets.
        if first_type == "Fixed" {
            self.reset_temporary_registers();
            let values = slots
                .iter()
                .zip(texts.iter())
                .map(|(slot, text)| {
                    let register = self.allocate_register()?;
                    self.emit(abi::load_u64(&register, abi::stack_pointer(), *slot));
                    Ok(ValueResult {
                        type_: "Fixed".to_string(),
                        location: register,
                        text: text.clone(),
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            return self.lower_fixed_external_math(function, &values);
        }
        let symbol = external_math_symbol(function, self.platform_imports)
            .ok_or_else(|| format!("native math lowering does not support math.{function}"))?;
        let Some(library) = self.platform_imports.get(&symbol).cloned() else {
            return Err(format!(
                "native math lowering for math.{function} requires platform import {symbol}"
            ));
        };

        self.reset_temporary_registers();
        for (index, slot) in slots.iter().enumerate() {
            let register = self.allocate_register()?;
            self.emit(abi::load_u64(&register, abi::stack_pointer(), *slot));
            self.emit(abi::float_move_d_from_x(&format!("d{index}"), &register));
        }
        if matches!(function, "log" | "log10") {
            // Float domain failures (non-positive input) report ErrFloatDomain.
            self.emit(abi::float_compare_zero_d("d0"));
            let valid = self.label("math_log_positive");
            self.emit(abi::branch_gt(&valid));
            self.emit_float_domain_return()?;
            self.emit(abi::label(&valid));
        }
        if matches!(function, "asin" | "acos") {
            // Arc sine/cosine are only defined on [-1.0, 1.0]; inputs outside the
            // domain would otherwise produce NaN. Report the domain failure
            // explicitly as ErrFloatDomain. The error path is terminal, so its
            // scratch registers are dead on the in-domain path; restore the
            // allocation counter afterwards to avoid inflating register pressure.
            let saved_registers = self.next_register;
            let valid = self.label("math_arc_in_domain");
            let domain_error = self.label("math_arc_domain_error");
            // value > 1.0 OR value < -1.0  =>  out of domain
            self.emit_f64_const("d1", "x17", 1.0);
            self.emit(abi::float_subtract_d("d2", "d0", "d1"));
            self.emit(abi::float_compare_zero_d("d2"));
            self.emit(abi::branch_gt(&domain_error));
            self.emit_f64_const("d1", "x17", -1.0);
            self.emit(abi::float_subtract_d("d2", "d0", "d1"));
            self.emit(abi::float_compare_zero_d("d2"));
            self.emit(abi::branch_ge(&valid));
            self.emit(abi::label(&domain_error));
            self.emit_float_domain_return()?;
            self.emit(abi::label(&valid));
            self.next_register = saved_registers;
        }

        self.emit(abi::branch_link(&symbol));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: symbol,
            kind: "branch26".to_string(),
            binding: "external".to_string(),
            library: Some(library),
        });

        let result_bits = self.allocate_register()?;
        self.emit(abi::float_move_x_from_d(&result_bits, "d0"));
        self.emit_float_result_check(&result_bits, FloatInfinityError::Infinity)?;

        Ok(ValueResult {
            type_: "Float".to_string(),
            location: result_bits,
            text: format!("math.{function}({})", texts.join(", ")),
        })
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
        let exponent = self.allocate_register()?;
        let mantissa = self.allocate_register()?;
        let ok = self.label("float_result_finite");
        let infinity = self.label("float_result_infinity");
        self.emit(abi::move_immediate("x17", "Integer", "9218868437227405312"));
        self.emit(abi::and_registers(&exponent, bits, "x17"));
        self.emit(abi::compare_registers(&exponent, "x17"));
        self.emit(abi::branch_ne(&ok));
        self.emit(abi::move_immediate("x17", "Integer", "4503599627370495"));
        self.emit(abi::and_registers(&mantissa, bits, "x17"));
        self.emit(abi::compare_immediate(&mantissa, "0"));
        self.emit(abi::branch_eq(&infinity));
        self.emit_float_nan_return()?;
        self.emit(abi::label(&infinity));
        match infinity_error {
            FloatInfinityError::Infinity => self.emit_float_inf_return()?,
            FloatInfinityError::Overflow => self.emit_float_overflow_return()?,
        }
        self.emit(abi::label(&ok));
        Ok(())
    }
}

pub(super) fn external_math_symbol(
    function: &str,
    platform_imports: &HashMap<String, String>,
) -> Option<String> {
    if !matches!(
        function,
        "pow"
            | "exp"
            | "log"
            | "log10"
            | "fmod"
            | "sin"
            | "cos"
            | "tan"
            | "asin"
            | "acos"
            | "atan"
            | "atan2"
    ) {
        return None;
    }
    let prefixed = format!("_{function}");
    if platform_imports.contains_key(&prefixed) {
        return Some(prefixed);
    }
    if platform_imports.contains_key(function) {
        return Some(function.to_string());
    }
    None
}
