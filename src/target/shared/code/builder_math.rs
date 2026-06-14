use super::*;

impl CodeBuilder<'_> {
    pub(super) fn lower_math_call(
        &mut self,
        function: &str,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        match function {
            "abs" if args.len() == 1 => self.lower_math_abs(&args[0]),
            "min" | "max" if args.len() == 2 => self.lower_math_min_max(function, args),
            "clamp" if args.len() == 3 => self.lower_math_clamp(args),
            "floor" | "ceil" | "round" if args.len() == 1 => {
                self.lower_math_rounding(function, &args[0])
            }
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
                match function {
                    "floor" => self.emit(abi::float_floor_to_signed_x(&dst, "d0")),
                    "ceil" => self.emit(abi::float_ceil_to_signed_x(&dst, "d0")),
                    "round" => self.emit(abi::float_round_to_signed_x(&dst, "d0")),
                    _ => unreachable!(),
                }
            }
            "Fixed" => {
                self.load_numeric_as_double(
                    "d0",
                    &ValueResult {
                        type_: "Fixed".to_string(),
                        location: value.location.clone(),
                        text: value.text.clone(),
                    },
                )?;
                match function {
                    "floor" => self.emit(abi::float_floor_to_signed_x(&dst, "d0")),
                    "ceil" => self.emit(abi::float_ceil_to_signed_x(&dst, "d0")),
                    "round" => self.emit(abi::float_round_to_signed_x(&dst, "d0")),
                    _ => unreachable!(),
                }
            }
            other => return Err(format!("math.{function} does not accept {other}")),
        }
        Ok(ValueResult {
            type_: "Integer".to_string(),
            location: dst,
            text: format!("math.{function}({})", value.text),
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
                self.emit_invalid_argument_return()?;
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
                let dst = self.allocate_register()?;
                self.emit(abi::compare_immediate(&value.location, "0"));
                let valid = self.label("math_fixed_sqrt_valid");
                self.emit(abi::branch_ge(&valid));
                self.emit_invalid_argument_return()?;
                self.emit(abi::label(&valid));
                self.load_numeric_as_double("d0", &value)?;
                self.emit(abi::float_sqrt_d("d0", "d0"));
                self.emit_f64_const("d1", "x17", 4_294_967_296.0);
                self.emit(abi::float_multiply_d("d0", "d0", "d1"));
                self.emit(abi::float_round_to_signed_x(&dst, "d0"));
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
        let symbol = external_math_symbol(function, self.platform_imports)
            .ok_or_else(|| format!("native math lowering does not support math.{function}"))?;
        let Some(library) = self.platform_imports.get(&symbol).cloned() else {
            return Err(format!(
                "native math lowering for math.{function} requires platform import {symbol}"
            ));
        };

        let values = args
            .iter()
            .map(|arg| self.lower_value(arg))
            .collect::<Result<Vec<_>, _>>()?;
        let Some(first) = values.first() else {
            return Err(format!("math.{function} expects at least one argument"));
        };
        if !matches!(first.type_.as_str(), "Float" | "Fixed") {
            return Err(format!("math.{function} does not accept {}", first.type_));
        }
        if values.iter().any(|value| value.type_ != first.type_) {
            return Err(format!("math.{function} requires matching argument types"));
        }
        let slots = values
            .iter()
            .map(|value| {
                let slot = self.allocate_stack_object("math_libsystem_arg", 8);
                self.emit(abi::store_u64(&value.location, abi::stack_pointer(), slot));
                slot
            })
            .collect::<Vec<_>>();

        self.reset_temporary_registers();
        for (index, value) in values.iter().enumerate() {
            let register = self.allocate_register()?;
            self.emit(abi::load_u64(&register, abi::stack_pointer(), slots[index]));
            match value.type_.as_str() {
                "Float" => self.emit(abi::float_move_d_from_x(&format!("d{index}"), &register)),
                "Fixed" => {
                    let fixed_value = ValueResult {
                        type_: "Fixed".to_string(),
                        location: register,
                        text: value.text.clone(),
                    };
                    self.load_numeric_as_double(&format!("d{index}"), &fixed_value)?;
                }
                other => return Err(format!("math.{function} does not accept {other}")),
            }
        }
        if matches!(function, "log" | "log10") {
            self.emit(abi::float_compare_zero_d("d0"));
            let valid = self.label("math_log_positive");
            self.emit(abi::branch_gt(&valid));
            self.emit_invalid_argument_return()?;
            self.emit(abi::label(&valid));
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
        self.emit_math_float_result_check(&result_bits)?;

        match first.type_.as_str() {
            "Float" => Ok(ValueResult {
                type_: "Float".to_string(),
                location: result_bits,
                text: format!("math.{function}({})", join_texts(&values)),
            }),
            "Fixed" => {
                let result = self.allocate_register()?;
                self.emit(abi::float_move_d_from_x("d0", &result_bits));
                self.emit_math_double_to_fixed_value(&result)?;
                Ok(ValueResult {
                    type_: "Fixed".to_string(),
                    location: result,
                    text: format!("math.{function}({})", join_texts(&values)),
                })
            }
            other => Err(format!("math.{function} does not accept {other}")),
        }
    }

    fn emit_math_float_result_check(&mut self, bits: &str) -> Result<(), String> {
        let exponent = self.allocate_register()?;
        let mantissa = self.allocate_register()?;
        let ok = self.label("math_float_result_finite");
        let overflow = self.label("math_float_result_overflow");
        self.emit(abi::move_immediate("x17", "Integer", "9218868437227405312"));
        self.emit(abi::and_registers(&exponent, bits, "x17"));
        self.emit(abi::compare_registers(&exponent, "x17"));
        self.emit(abi::branch_ne(&ok));
        self.emit(abi::move_immediate("x17", "Integer", "4503599627370495"));
        self.emit(abi::and_registers(&mantissa, bits, "x17"));
        self.emit(abi::compare_immediate(&mantissa, "0"));
        self.emit(abi::branch_eq(&overflow));
        self.emit_invalid_argument_return()?;
        self.emit(abi::label(&overflow));
        self.emit_overflow_return()?;
        self.emit(abi::label(&ok));
        Ok(())
    }

    fn emit_math_double_to_fixed_value(&mut self, result: &str) -> Result<(), String> {
        let bits = self.allocate_register()?;
        let exponent = self.allocate_register()?;
        let sign = self.allocate_register()?;
        let mantissa = self.allocate_register()?;
        let range_ok = self.label("math_fixed_result_range_ok");
        let edge = self.label("math_fixed_result_edge");
        let edge_negative = self.label("math_fixed_result_edge_negative");
        let overflow = self.label("math_fixed_result_overflow");
        let ok = self.label("math_fixed_result_ok");
        self.emit(abi::float_move_x_from_d(&bits, "d0"));
        self.emit(abi::shift_right_immediate(&exponent, &bits, 52));
        self.emit(abi::move_immediate("x17", "Integer", "2047"));
        self.emit(abi::and_registers(&exponent, &exponent, "x17"));
        self.emit(abi::compare_immediate(&exponent, "1054"));
        self.emit(abi::branch_lt(&range_ok));
        self.emit(abi::branch_eq(&edge));
        self.emit(abi::branch(&overflow));
        self.emit(abi::label(&edge));
        self.emit(abi::shift_right_immediate(&sign, &bits, 63));
        self.emit(abi::compare_immediate(&sign, "1"));
        self.emit(abi::branch_eq(&edge_negative));
        self.emit(abi::branch(&overflow));
        self.emit(abi::label(&edge_negative));
        self.emit(abi::move_immediate("x17", "Integer", "4503599627370495"));
        self.emit(abi::and_registers(&mantissa, &bits, "x17"));
        self.emit(abi::compare_immediate(&mantissa, "0"));
        self.emit(abi::branch_ne(&overflow));
        self.emit(abi::label(&range_ok));
        self.emit_f64_const("d1", "x17", 4_294_967_296.0);
        self.emit(abi::float_multiply_d("d0", "d0", "d1"));
        self.emit(abi::float_round_to_signed_x(result, "d0"));
        self.emit(abi::branch(&ok));
        self.emit(abi::label(&overflow));
        self.emit_overflow_return()?;
        self.emit(abi::label(&ok));
        Ok(())
    }
}

fn external_math_symbol(
    function: &str,
    platform_imports: &HashMap<String, String>,
) -> Option<String> {
    if !matches!(
        function,
        "pow"
            | "exp"
            | "log"
            | "log10"
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
