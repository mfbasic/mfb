use super::*;

impl CodeBuilder<'_> {
    pub(super) fn lower_boolean_binary(
        &mut self,
        op: &str,
        left: &NirValue,
        right: &NirValue,
    ) -> Result<ValueResult, String> {
        match op {
            "AND" => self.lower_short_circuit_and(left, right),
            "OR" => self.lower_short_circuit_or(left, right),
            "XOR" => self.lower_boolean_xor(left, right),
            other => Err(format!(
                "native code plan does not lower boolean operator '{other}'"
            )),
        }
    }

    fn lower_short_circuit_and(
        &mut self,
        left: &NirValue,
        right: &NirValue,
    ) -> Result<ValueResult, String> {
        let left = self.lower_value(left)?;
        let result = self.allocate_register()?;
        let right_label = self.label("bool_and_right");
        let done_label = self.label("bool_and_done");
        self.emit(abi::compare_immediate(&left.location, "0"));
        self.emit(abi::branch_ne(&right_label));
        self.emit(abi::move_immediate(&result, "Boolean", "false"));
        self.emit(abi::branch(&done_label));
        self.emit(abi::label(&right_label));
        let right = self.lower_value(right)?;
        self.emit(abi::move_register(&result, &right.location));
        self.emit(abi::label(&done_label));
        Ok(ValueResult {
            type_: "Boolean".to_string(),
            location: result,
            text: format!("({} AND {})", left.text, right.text),
        })
    }

    fn lower_short_circuit_or(
        &mut self,
        left: &NirValue,
        right: &NirValue,
    ) -> Result<ValueResult, String> {
        let left = self.lower_value(left)?;
        let result = self.allocate_register()?;
        let right_label = self.label("bool_or_right");
        let done_label = self.label("bool_or_done");
        self.emit(abi::compare_immediate(&left.location, "0"));
        self.emit(abi::branch_eq(&right_label));
        self.emit(abi::move_immediate(&result, "Boolean", "true"));
        self.emit(abi::branch(&done_label));
        self.emit(abi::label(&right_label));
        let right = self.lower_value(right)?;
        self.emit(abi::move_register(&result, &right.location));
        self.emit(abi::label(&done_label));
        Ok(ValueResult {
            type_: "Boolean".to_string(),
            location: result,
            text: format!("({} OR {})", left.text, right.text),
        })
    }

    fn lower_boolean_xor(
        &mut self,
        left: &NirValue,
        right: &NirValue,
    ) -> Result<ValueResult, String> {
        let left = self.lower_value(left)?;
        let right = self.lower_value(right)?;
        let result = self.allocate_register()?;
        self.emit(abi::exclusive_or_registers(
            &result,
            &left.location,
            &right.location,
        ));
        Ok(ValueResult {
            type_: "Boolean".to_string(),
            location: result,
            text: format!("({} XOR {})", left.text, right.text),
        })
    }

    pub(super) fn lower_arithmetic_binary(
        &mut self,
        op: &str,
        left: &NirValue,
        right: &NirValue,
    ) -> Result<ValueResult, String> {
        let left = self.lower_value(left)?;
        let left_slot = self.allocate_stack_object("arith_left", 8);
        self.emit(abi::store_u64(
            &left.location,
            abi::stack_pointer(),
            left_slot,
        ));
        let right = self.lower_value(right)?;
        let right_slot = self.allocate_stack_object("arith_right", 8);
        self.emit(abi::store_u64(
            &right.location,
            abi::stack_pointer(),
            right_slot,
        ));
        let left_text = left.text.clone();
        let right_text = right.text.clone();
        let result_type = numeric_binary_result_type(op, &left.type_, &right.type_).to_string();
        self.reset_temporary_registers();
        let left_register = self.allocate_register()?;
        let right_register = self.allocate_register()?;
        self.emit(abi::load_u64(
            &left_register,
            abi::stack_pointer(),
            left_slot,
        ));
        self.emit(abi::load_u64(
            &right_register,
            abi::stack_pointer(),
            right_slot,
        ));
        let left = ValueResult {
            type_: left.type_,
            location: left_register,
            text: left_text,
        };
        let right = ValueResult {
            type_: right.type_,
            location: right_register,
            text: right_text,
        };
        let register = self.allocate_register()?;
        match result_type.as_str() {
            "Byte" | "Integer" => {
                self.emit_integer_binary(op, &left, &right, &register, result_type == "Byte")?;
            }
            "Fixed" => self.emit_fixed_binary(op, &left, &right, &register)?,
            "Float" => self.emit_float_binary(op, &left, &right, &register)?,
            other => {
                return Err(format!(
                    "native code plan cannot lower arithmetic result type '{other}'"
                ));
            }
        }
        Ok(ValueResult {
            type_: result_type,
            location: register,
            text: format!("({} {op} {})", left.text, right.text),
        })
    }

    pub(super) fn lower_comparison_binary(
        &mut self,
        op: &str,
        left: &NirValue,
        right: &NirValue,
    ) -> Result<ValueResult, String> {
        let left = self.lower_value(left)?;
        let left_slot = self.allocate_stack_object("cmp_left", 8);
        self.emit(abi::store_u64(
            &left.location,
            abi::stack_pointer(),
            left_slot,
        ));
        if left.type_ == "String" {
            let right = self.lower_value(right)?;
            if right.type_ != "String" {
                return Err(format!(
                    "native code comparison requires matching String operands, got {} and {}",
                    left.type_, right.type_
                ));
            }
            let right_slot = self.allocate_stack_object("cmp_right", 8);
            self.emit(abi::store_u64(
                &right.location,
                abi::stack_pointer(),
                right_slot,
            ));
            self.reset_temporary_registers();
            let left_register = self.allocate_register()?;
            let right_register = self.allocate_register()?;
            self.emit(abi::load_u64(
                &left_register,
                abi::stack_pointer(),
                left_slot,
            ));
            self.emit(abi::load_u64(
                &right_register,
                abi::stack_pointer(),
                right_slot,
            ));
            let left = ValueResult {
                type_: left.type_,
                location: left_register,
                text: left.text,
            };
            let right = ValueResult {
                type_: right.type_,
                location: right_register,
                text: right.text,
            };
            return self.lower_string_comparison_binary(op, &left, &right);
        }
        if matches!(left.type_.as_str(), "Byte" | "Integer" | "Fixed" | "Float") {
            let right = self.lower_value(right)?;
            let right_slot = self.allocate_stack_object("cmp_right", 8);
            self.emit(abi::store_u64(
                &right.location,
                abi::stack_pointer(),
                right_slot,
            ));
            self.reset_temporary_registers();
            let left_register = self.allocate_register()?;
            let right_register = self.allocate_register()?;
            self.emit(abi::load_u64(
                &left_register,
                abi::stack_pointer(),
                left_slot,
            ));
            self.emit(abi::load_u64(
                &right_register,
                abi::stack_pointer(),
                right_slot,
            ));
            let left = ValueResult {
                type_: left.type_,
                location: left_register,
                text: left.text,
            };
            let right = ValueResult {
                type_: right.type_,
                location: right_register,
                text: right.text,
            };
            return self.lower_numeric_comparison_binary(op, &left, &right);
        }
        let right = self.lower_value(right)?;
        let right_slot = self.allocate_stack_object("cmp_right", 8);
        self.emit(abi::store_u64(
            &right.location,
            abi::stack_pointer(),
            right_slot,
        ));
        self.reset_temporary_registers();
        let left_register = self.allocate_register()?;
        let right_register = self.allocate_register()?;
        let result = self.allocate_register()?;
        let true_label = self.label("cmp_true");
        let done_label = self.label("cmp_done");
        self.emit(abi::load_u64(
            &left_register,
            abi::stack_pointer(),
            left_slot,
        ));
        self.emit(abi::load_u64(
            &right_register,
            abi::stack_pointer(),
            right_slot,
        ));
        self.emit(abi::compare_registers(&left_register, &right_register));
        match op {
            "=" => self.emit(abi::branch_eq(&true_label)),
            "<>" => self.emit(abi::branch_ne(&true_label)),
            "<" => self.emit(abi::branch_lt(&true_label)),
            ">" => self.emit(abi::branch_gt(&true_label)),
            "<=" => self.emit(abi::branch_le(&true_label)),
            ">=" => self.emit(abi::branch_ge(&true_label)),
            other => {
                return Err(format!(
                    "native code plan does not lower comparison operator '{other}'"
                ));
            }
        }
        self.emit(abi::move_immediate(&result, "Boolean", "false"));
        self.emit(abi::branch(&done_label));
        self.emit(abi::label(&true_label));
        self.emit(abi::move_immediate(&result, "Boolean", "true"));
        self.emit(abi::label(&done_label));
        Ok(ValueResult {
            type_: "Boolean".to_string(),
            location: result,
            text: format!("({} {op} {})", left.text, right.text),
        })
    }

    fn lower_numeric_comparison_binary(
        &mut self,
        op: &str,
        left: &ValueResult,
        right: &ValueResult,
    ) -> Result<ValueResult, String> {
        let left_slot = self.allocate_stack_object("cmp_left", 8);
        let right_slot = self.allocate_stack_object("cmp_right", 8);
        self.emit(abi::store_u64(&left.location, abi::stack_pointer(), left_slot));
        self.emit(abi::store_u64(&right.location, abi::stack_pointer(), right_slot));

        self.reset_temporary_registers();
        let left_register = self.allocate_register()?;
        let right_register = self.allocate_register()?;
        self.emit(abi::load_u64(
            &left_register,
            abi::stack_pointer(),
            left_slot,
        ));
        self.emit(abi::load_u64(
            &right_register,
            abi::stack_pointer(),
            right_slot,
        ));
        let left = ValueResult {
            type_: left.type_.clone(),
            location: left_register,
            text: left.text.clone(),
        };
        let right = ValueResult {
            type_: right.type_.clone(),
            location: right_register,
            text: right.text.clone(),
        };

        let promoted = if left.type_ == "Float" || right.type_ == "Float" {
            "Float".to_string()
        } else if left.type_ == "Fixed" || right.type_ == "Fixed" {
            "Fixed".to_string()
        } else {
            numeric_binary_result_type("+", &left.type_, &right.type_).to_string()
        };
        let result = self.allocate_register()?;
        let true_label = self.label("cmp_true");
        let done_label = self.label("cmp_done");

        match promoted.as_str() {
            "Byte" | "Integer" => {
                self.emit(abi::compare_registers(&left.location, &right.location));
            }
            "Fixed" => {
                let left_fixed = self.allocate_register()?;
                let right_fixed = self.allocate_register()?;
                let left_fixed_slot = self.allocate_stack_object("cmp_left_fixed", 8);
                self.load_numeric_as_fixed(&left_fixed, &left)?;
                self.emit(abi::store_u64(
                    &left_fixed,
                    abi::stack_pointer(),
                    left_fixed_slot,
                ));
                self.load_numeric_as_fixed(&right_fixed, &right)?;
                self.emit(abi::load_u64(
                    &left_fixed,
                    abi::stack_pointer(),
                    left_fixed_slot,
                ));
                self.emit(abi::compare_registers(&left_fixed, &right_fixed));
            }
            "Float" => {
                self.load_numeric_as_double("d0", &left)?;
                self.load_numeric_as_double("d1", &right)?;
                self.emit(abi::float_compare_d("d0", "d1"));
            }
            other => {
                return Err(format!(
                    "native code plan cannot lower numeric comparison result type '{other}'"
                ));
            }
        }

        match op {
            "=" => self.emit(abi::branch_eq(&true_label)),
            "<>" => self.emit(abi::branch_ne(&true_label)),
            "<" => self.emit(abi::branch_lt(&true_label)),
            ">" => self.emit(abi::branch_gt(&true_label)),
            "<=" => self.emit(abi::branch_le(&true_label)),
            ">=" => self.emit(abi::branch_ge(&true_label)),
            other => {
                return Err(format!(
                    "native code plan does not lower comparison operator '{other}'"
                ));
            }
        }
        self.emit(abi::move_immediate(&result, "Boolean", "false"));
        self.emit(abi::branch(&done_label));
        self.emit(abi::label(&true_label));
        self.emit(abi::move_immediate(&result, "Boolean", "true"));
        self.emit(abi::label(&done_label));
        Ok(ValueResult {
            type_: "Boolean".to_string(),
            location: result,
            text: format!("({} {op} {})", left.text, right.text),
        })
    }

    fn lower_string_comparison_binary(
        &mut self,
        op: &str,
        left: &ValueResult,
        right: &ValueResult,
    ) -> Result<ValueResult, String> {
        match op {
            "=" | "<>" => {}
            other => {
                return Err(format!(
                    "native code does not lower string comparison operator '{other}'"
                ));
            }
        }

        let result = self.allocate_register()?;
        let loop_label = self.label("cmp_string_loop");
        let equal_label = self.label("cmp_string_equal");
        let not_equal_label = self.label("cmp_string_not_equal");
        let done_label = self.label("cmp_string_done");

        self.emit(abi::load_u64("x11", &left.location, 0));
        self.emit(abi::load_u64("x12", &right.location, 0));
        self.emit(abi::compare_registers("x11", "x12"));
        self.emit(abi::branch_ne(&not_equal_label));
        self.emit(abi::add_immediate("x13", &left.location, 8));
        self.emit(abi::add_immediate("x14", &right.location, 8));
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate("x11", "0"));
        self.emit(abi::branch_eq(&equal_label));
        self.emit(abi::load_u8("x15", "x13", 0));
        self.emit(abi::load_u8("x16", "x14", 0));
        self.emit(abi::compare_registers("x15", "x16"));
        self.emit(abi::branch_ne(&not_equal_label));
        self.emit(abi::add_immediate("x13", "x13", 1));
        self.emit(abi::add_immediate("x14", "x14", 1));
        self.emit(abi::subtract_immediate("x11", "x11", 1));
        self.emit(abi::branch(&loop_label));

        self.emit(abi::label(&equal_label));
        self.emit(abi::move_immediate(
            &result,
            "Boolean",
            if op == "=" { "true" } else { "false" },
        ));
        self.emit(abi::branch(&done_label));

        self.emit(abi::label(&not_equal_label));
        self.emit(abi::move_immediate(
            &result,
            "Boolean",
            if op == "=" { "false" } else { "true" },
        ));
        self.emit(abi::label(&done_label));

        Ok(ValueResult {
            type_: "Boolean".to_string(),
            location: result,
            text: format!("({} {op} {})", left.text, right.text),
        })
    }

    pub(super) fn emit_integer_binary(
        &mut self,
        op: &str,
        left: &ValueResult,
        right: &ValueResult,
        dst: &str,
        byte_result: bool,
    ) -> Result<(), String> {
        match op {
            "+" => {
                self.emit(abi::add_registers_set_flags(
                    dst,
                    &left.location,
                    &right.location,
                ));
                self.emit_overflow_if_flags_set()?;
                if byte_result {
                    self.emit_byte_upper_bound_check(dst)?;
                }
            }
            "-" => {
                if byte_result {
                    let underflow_label = self.label("byte_underflow");
                    let ok_label = self.label("byte_ok");
                    self.emit(abi::compare_registers(&left.location, &right.location));
                    self.emit(abi::branch_lo(&underflow_label));
                    self.emit(abi::subtract_registers(
                        dst,
                        &left.location,
                        &right.location,
                    ));
                    self.emit(abi::branch(&ok_label));
                    self.emit(abi::label(&underflow_label));
                    self.emit_underflow_return()?;
                    self.emit(abi::label(&ok_label));
                } else {
                    self.emit(abi::subtract_registers_set_flags(
                        dst,
                        &left.location,
                        &right.location,
                    ));
                    self.emit_overflow_if_flags_set()?;
                }
            }
            "*" => {
                self.emit_checked_integer_multiply(dst, &left.location, &right.location)?;
                if byte_result {
                    self.emit_byte_upper_bound_check(dst)?;
                }
            }
            "/" | "DIV" => {
                self.emit_nonzero_or_invalid(&right.location)?;
                self.emit_integer_division_overflow_check(&left.location, &right.location)?;
                self.emit(abi::signed_divide_registers(
                    dst,
                    &left.location,
                    &right.location,
                ));
            }
            "MOD" => {
                self.emit_nonzero_or_invalid(&right.location)?;
                self.emit_integer_division_overflow_check(&left.location, &right.location)?;
                let quotient = self.allocate_register()?;
                self.emit(abi::signed_divide_registers(
                    &quotient,
                    &left.location,
                    &right.location,
                ));
                self.emit(abi::multiply_subtract_registers(
                    dst,
                    &quotient,
                    &right.location,
                    &left.location,
                ));
            }
            "^" => self.emit_integer_pow(dst, &left.location, &right.location, byte_result)?,
            other => {
                return Err(format!(
                    "native code plan does not lower integer operator '{other}'"
                ));
            }
        }
        Ok(())
    }

    pub(super) fn emit_fixed_binary(
        &mut self,
        op: &str,
        left: &ValueResult,
        right: &ValueResult,
        dst: &str,
    ) -> Result<(), String> {
        match op {
            "+" => {
                self.emit(abi::add_registers_set_flags(
                    dst,
                    &left.location,
                    &right.location,
                ));
                self.emit_overflow_if_flags_set()?;
            }
            "-" => {
                self.emit(abi::subtract_registers_set_flags(
                    dst,
                    &left.location,
                    &right.location,
                ));
                self.emit_overflow_if_flags_set()?;
            }
            "*" => self.emit_fixed_multiply(dst, &left.location, &right.location)?,
            "/" => self.emit_fixed_divide(dst, &left.location, &right.location)?,
            "MOD" => {
                self.emit_fixed_divide(dst, &left.location, &right.location)?;
                let product = self.allocate_register()?;
                self.emit_fixed_multiply(&product, dst, &right.location)?;
                self.emit(abi::subtract_registers_set_flags(
                    dst,
                    &left.location,
                    &product,
                ));
                self.emit_overflow_if_flags_set()?;
            }
            "^" => self.emit_fixed_pow(dst, &left.location, &right.location)?,
            other => {
                return Err(format!(
                    "native code plan does not lower Fixed operator '{other}'"
                ));
            }
        }
        Ok(())
    }

    pub(super) fn emit_float_binary(
        &mut self,
        op: &str,
        left: &ValueResult,
        right: &ValueResult,
        dst: &str,
    ) -> Result<(), String> {
        self.load_numeric_as_double("d0", left)?;
        self.load_numeric_as_double("d1", right)?;
        match op {
            "+" => self.emit(abi::float_add_d("d0", "d0", "d1")),
            "-" => self.emit(abi::float_subtract_d("d0", "d0", "d1")),
            "*" => self.emit(abi::float_multiply_d("d0", "d0", "d1")),
            "/" | "DIV" => {
                self.emit(abi::float_compare_zero_d("d1"));
                let nonzero = self.label("float_divisor_nonzero");
                self.emit(abi::branch_ne(&nonzero));
                self.emit_invalid_argument_return()?;
                self.emit(abi::label(&nonzero));
                self.emit(abi::float_divide_d("d0", "d0", "d1"));
            }
            "^" => self.emit_float_pow("d0", "d1")?,
            other => {
                return Err(format!(
                    "native code plan does not lower Float operator '{other}'"
                ));
            }
        }
        self.emit(abi::float_move_x_from_d(dst, "d0"));
        Ok(())
    }

    pub(super) fn emit_overflow_if_flags_set(&mut self) -> Result<(), String> {
        let ok_label = self.label("overflow_ok");
        self.emit(abi::branch_vc(&ok_label));
        self.emit_overflow_return()?;
        self.emit(abi::label(&ok_label));
        Ok(())
    }

    pub(super) fn emit_checked_integer_add(
        &mut self,
        dst: &str,
        lhs: &str,
        rhs: &str,
    ) -> Result<(), String> {
        self.emit(abi::add_registers_set_flags(dst, lhs, rhs));
        self.emit_overflow_if_flags_set()
    }

    pub(super) fn emit_byte_upper_bound_check(&mut self, value: &str) -> Result<(), String> {
        let overflow_label = self.label("byte_overflow");
        let ok_label = self.label("byte_ok");
        self.emit(abi::compare_immediate(value, "255"));
        self.emit(abi::branch_hi(&overflow_label));
        self.emit(abi::branch(&ok_label));
        self.emit(abi::label(&overflow_label));
        self.emit_overflow_return()?;
        self.emit(abi::label(&ok_label));
        Ok(())
    }

    pub(super) fn emit_checked_integer_multiply(
        &mut self,
        dst: &str,
        left: &str,
        right: &str,
    ) -> Result<(), String> {
        let high = self.allocate_register()?;
        let sign = self.allocate_register()?;
        let ok_label = self.label("mul_ok");
        self.emit(abi::multiply_registers(dst, left, right));
        self.emit(abi::signed_multiply_high_registers(&high, left, right));
        self.emit(abi::arithmetic_shift_right_immediate(&sign, dst, 63));
        self.emit(abi::compare_registers(&high, &sign));
        self.emit(abi::branch_eq(&ok_label));
        self.emit_overflow_return()?;
        self.emit(abi::label(&ok_label));
        Ok(())
    }

    pub(super) fn emit_nonzero_or_invalid(&mut self, value: &str) -> Result<(), String> {
        let ok_label = self.label("nonzero");
        self.emit(abi::compare_immediate(value, "0"));
        self.emit(abi::branch_ne(&ok_label));
        self.emit_invalid_argument_return()?;
        self.emit(abi::label(&ok_label));
        Ok(())
    }

    pub(super) fn emit_integer_division_overflow_check(
        &mut self,
        left: &str,
        right: &str,
    ) -> Result<(), String> {
        let min = self.allocate_register()?;
        let minus_one = self.allocate_register()?;
        let not_min = self.label("div_not_min");
        let ok = self.label("div_overflow_ok");
        self.emit(abi::move_immediate(&min, "Integer", "9223372036854775808"));
        self.emit(abi::compare_registers(left, &min));
        self.emit(abi::branch_ne(&not_min));
        self.emit(abi::move_immediate(
            &minus_one,
            "Integer",
            &u64::MAX.to_string(),
        ));
        self.emit(abi::compare_registers(right, &minus_one));
        self.emit(abi::branch_ne(&ok));
        self.emit_overflow_return()?;
        self.emit(abi::label(&not_min));
        self.emit(abi::label(&ok));
        Ok(())
    }

    pub(super) fn emit_integer_pow(
        &mut self,
        dst: &str,
        base: &str,
        exponent: &str,
        byte_result: bool,
    ) -> Result<(), String> {
        let loop_label = self.label("pow_loop");
        let done_label = self.label("pow_done");
        let nonnegative = self.label("pow_nonnegative");
        let remaining = self.allocate_register()?;
        self.emit(abi::compare_immediate(exponent, "0"));
        self.emit(abi::branch_ge(&nonnegative));
        self.emit_invalid_argument_return()?;
        self.emit(abi::label(&nonnegative));
        self.emit(abi::move_register(&remaining, exponent));
        self.emit(abi::move_immediate(dst, "Integer", "1"));
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate(&remaining, "0"));
        self.emit(abi::branch_eq(&done_label));
        self.emit_checked_integer_multiply(dst, dst, base)?;
        self.emit(abi::subtract_immediate(&remaining, &remaining, 1));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&done_label));
        if byte_result {
            self.emit_byte_upper_bound_check(dst)?;
        }
        Ok(())
    }

    pub(super) fn emit_fixed_multiply(
        &mut self,
        dst: &str,
        left: &str,
        right: &str,
    ) -> Result<(), String> {
        let high = self.allocate_register()?;
        let shifted_high = self.allocate_register()?;
        let max_high = self.allocate_register()?;
        let min_high = self.allocate_register()?;
        let overflow = self.label("fixed_mul_overflow");
        let ok = self.label("fixed_mul_ok");
        self.emit(abi::multiply_registers(dst, left, right));
        self.emit(abi::signed_multiply_high_registers(&high, left, right));
        self.emit(abi::move_immediate(&max_high, "Integer", "2147483647"));
        self.emit(abi::compare_registers(&high, &max_high));
        self.emit(abi::branch_gt(&overflow));
        self.emit(abi::move_immediate(
            &min_high,
            "Integer",
            &(-2147483648_i64 as u64).to_string(),
        ));
        self.emit(abi::compare_registers(&high, &min_high));
        self.emit(abi::branch_lt(&overflow));
        self.emit(abi::shift_right_immediate(dst, dst, 32));
        self.emit(abi::shift_left_immediate(&shifted_high, &high, 32));
        self.emit(abi::or_registers(dst, &shifted_high, dst));
        self.emit(abi::branch(&ok));
        self.emit(abi::label(&overflow));
        self.emit_overflow_return()?;
        self.emit(abi::label(&ok));
        Ok(())
    }

    pub(super) fn emit_fixed_divide(
        &mut self,
        dst: &str,
        left: &str,
        right: &str,
    ) -> Result<(), String> {
        self.emit_nonzero_or_invalid(right)?;
        let lhs_abs = self.allocate_register()?;
        let rhs_abs = self.allocate_register()?;
        let sign = self.allocate_register()?;
        let integer = self.allocate_register()?;
        let remainder = self.allocate_register()?;
        let fraction = self.allocate_register()?;
        let counter = self.allocate_register()?;
        let bit = self.allocate_register()?;
        self.emit(abi::move_register(&lhs_abs, left));
        self.emit(abi::move_register(&rhs_abs, right));
        self.emit(abi::exclusive_or_registers(&sign, &lhs_abs, &rhs_abs));
        self.emit_abs_i64(&lhs_abs)?;
        self.emit_abs_i64(&rhs_abs)?;
        self.emit(abi::unsigned_divide_registers(&integer, &lhs_abs, &rhs_abs));
        self.emit(abi::multiply_subtract_registers(
            &remainder, &integer, &rhs_abs, &lhs_abs,
        ));
        let max_integer = self.allocate_register()?;
        let overflow = self.label("fixed_div_overflow");
        let integer_ok = self.label("fixed_div_integer_ok");
        self.emit(abi::move_immediate(&max_integer, "Integer", "2147483647"));
        self.emit(abi::compare_registers(&integer, &max_integer));
        self.emit(abi::branch_hi(&overflow));
        self.emit(abi::shift_left_immediate(dst, &integer, 32));
        self.emit(abi::move_immediate(&fraction, "Integer", "0"));
        self.emit(abi::move_immediate(&counter, "Integer", "32"));
        self.emit(abi::branch(&integer_ok));
        self.emit(abi::label(&overflow));
        self.emit_overflow_return()?;
        self.emit(abi::label(&integer_ok));

        let loop_start = self.label("fixed_div_loop");
        let skip_subtract = self.label("fixed_div_skip_subtract");
        let done = self.label("fixed_div_done");
        self.emit(abi::label(&loop_start));
        self.emit(abi::compare_immediate(&counter, "0"));
        self.emit(abi::branch_eq(&done));
        self.emit(abi::shift_left_immediate(&remainder, &remainder, 1));
        self.emit(abi::shift_left_immediate(&fraction, &fraction, 1));
        self.emit(abi::compare_registers(&remainder, &rhs_abs));
        self.emit(abi::branch_lo(&skip_subtract));
        self.emit(abi::subtract_registers(&remainder, &remainder, &rhs_abs));
        self.emit(abi::move_immediate(&bit, "Integer", "1"));
        self.emit(abi::or_registers(&fraction, &fraction, &bit));
        self.emit(abi::label(&skip_subtract));
        self.emit(abi::subtract_immediate(&counter, &counter, 1));
        self.emit(abi::branch(&loop_start));

        self.emit(abi::label(&done));
        self.emit(abi::or_registers(dst, dst, &fraction));
        let negative = self.label("fixed_div_negative");
        let quotient_done = self.label("fixed_div_signed");
        self.emit(abi::compare_immediate(&sign, "0"));
        self.emit(abi::branch_lt(&negative));
        self.emit(abi::compare_immediate(dst, "0"));
        self.emit(abi::branch_ge(&quotient_done));
        self.emit_overflow_return()?;
        self.emit(abi::label(&negative));
        self.emit_neg_i64(dst)?;
        self.emit(abi::label(&quotient_done));
        Ok(())
    }

    pub(super) fn emit_fixed_pow(
        &mut self,
        dst: &str,
        base: &str,
        exponent: &str,
    ) -> Result<(), String> {
        let one_raw = 1_u64 << 32;
        let remaining = self.allocate_register()?;
        let whole = self.allocate_register()?;
        let nonnegative = self.label("fixed_pow_nonnegative");
        let loop_label = self.label("fixed_pow_loop");
        let done_label = self.label("fixed_pow_done");
        self.emit(abi::compare_immediate(exponent, "0"));
        self.emit(abi::branch_ge(&nonnegative));
        self.emit_invalid_argument_return()?;
        self.emit(abi::label(&nonnegative));
        self.emit(abi::arithmetic_shift_right_immediate(&whole, exponent, 32));
        self.emit(abi::shift_left_immediate(&remaining, &whole, 32));
        self.emit(abi::compare_registers(&remaining, exponent));
        let exponent_is_whole = self.label("fixed_pow_whole");
        self.emit(abi::branch_eq(&exponent_is_whole));
        self.emit_invalid_argument_return()?;
        self.emit(abi::label(&exponent_is_whole));
        self.emit(abi::move_register(&remaining, &whole));
        self.emit(abi::move_immediate(dst, "Fixed", &one_raw.to_string()));
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate(&remaining, "0"));
        self.emit(abi::branch_eq(&done_label));
        self.emit_fixed_multiply(dst, dst, base)?;
        self.emit(abi::subtract_immediate(&remaining, &remaining, 1));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&done_label));
        Ok(())
    }

    pub(super) fn emit_abs_i64(&mut self, register: &str) -> Result<(), String> {
        let positive = self.label("abs_positive");
        self.emit(abi::compare_immediate(register, "0"));
        self.emit(abi::branch_ge(&positive));
        self.emit_neg_i64(register)?;
        self.emit(abi::label(&positive));
        Ok(())
    }

    pub(super) fn emit_neg_i64(&mut self, register: &str) -> Result<(), String> {
        self.emit(abi::subtract_registers(register, "xzr", register));
        Ok(())
    }

    pub(super) fn load_numeric_as_double(
        &mut self,
        dst: &str,
        value: &ValueResult,
    ) -> Result<(), String> {
        match value.type_.as_str() {
            "Float" => self.emit(abi::float_move_d_from_x(dst, &value.location)),
            "Byte" | "Integer" => self.emit(abi::signed_convert_to_float_d(dst, &value.location)),
            "Fixed" => {
                self.emit(abi::signed_convert_to_float_d(dst, &value.location));
                self.emit_f64_const("d7", "x17", 4_294_967_296.0);
                self.emit(abi::float_divide_d(dst, dst, "d7"));
            }
            other => {
                return Err(format!(
                    "native Float arithmetic cannot load operand type '{other}'"
                ));
            }
        }
        Ok(())
    }

    pub(super) fn load_numeric_as_fixed(
        &mut self,
        dst: &str,
        value: &ValueResult,
    ) -> Result<(), String> {
        match value.type_.as_str() {
            "Fixed" => self.emit(abi::move_register(dst, &value.location)),
            "Byte" | "Integer" => self.emit_integer_to_fixed_value(&value.location, dst)?,
            "Float" => self.emit_float_bits_to_fixed_value(&value.location, dst)?,
            other => {
                return Err(format!(
                    "native Fixed comparison cannot load operand type '{other}'"
                ));
            }
        }
        Ok(())
    }

    pub(super) fn emit_f64_const(&mut self, dst: &str, scratch: &str, value: f64) {
        self.emit(abi::move_immediate(
            scratch,
            "Integer",
            &value.to_bits().to_string(),
        ));
        self.emit(abi::float_move_d_from_x(dst, scratch));
    }

    pub(super) fn emit_float_pow(&mut self, dst: &str, exponent: &str) -> Result<(), String> {
        let nonnegative = self.label("float_pow_nonnegative");
        let exponent_whole = self.label("float_pow_whole");
        let loop_label = self.label("float_pow_loop");
        let done_label = self.label("float_pow_done");
        self.emit(abi::float_compare_zero_d(exponent));
        self.emit(abi::branch_ge(&nonnegative));
        self.emit_invalid_argument_return()?;
        self.emit(abi::label(&nonnegative));
        let exponent_int = self.allocate_register()?;
        let exponent_roundtrip = self.allocate_register()?;
        let exponent_bits = self.allocate_register()?;
        let scratch = self.allocate_register()?;
        self.emit(abi::float_convert_to_signed_x(&exponent_int, exponent));
        self.emit(abi::signed_convert_to_float_d("d2", &exponent_int));
        self.emit(abi::float_move_x_from_d(&exponent_roundtrip, "d2"));
        self.emit(abi::float_move_x_from_d(&exponent_bits, exponent));
        self.emit(abi::compare_registers(&exponent_roundtrip, &exponent_bits));
        self.emit(abi::branch_eq(&exponent_whole));
        self.emit_invalid_argument_return()?;
        self.emit(abi::label(&exponent_whole));
        self.emit_f64_const("d2", &scratch, 1.0);
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate(&exponent_int, "0"));
        self.emit(abi::branch_eq(&done_label));
        self.emit(abi::float_multiply_d("d2", "d2", dst));
        self.emit(abi::subtract_immediate(&exponent_int, &exponent_int, 1));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&done_label));
        self.emit_f64_const("d7", &scratch, 0.0);
        self.emit(abi::float_add_d(dst, "d2", "d7"));
        Ok(())
    }
}
