use super::*;

/// Upper bound on the decimal exponent magnitude accumulated while parsing a
/// numeric string. The representable range of an IEEE-754 double spans roughly
/// 10^-324 to 10^308, so any exponent magnitude at or beyond this clamp drives
/// every representable mantissa to overflow (infinity) or underflow (zero). The
/// value is well above that useful range yet far below 2^63, so accumulation can
/// never wrap a 64-bit register. It also fits the AArch64 12-bit `cmp` immediate.
const DECIMAL_EXPONENT_CLAMP: &str = "1000";

impl CodeBuilder<'_> {
    pub(super) fn lower_to_int(&mut self, arg: &NirValue) -> Result<ValueResult, String> {
        let value = self.lower_value(arg)?;
        if value.type_ == "Byte" {
            let register = self.allocate_register()?;
            self.emit(abi::move_register(&register, &value.location));
            return Ok(ValueResult {
                type_: "Integer".to_string(),
                location: register,
                text: format!("toInt({})", value.text),
            });
        }
        let value_slot = self.allocate_stack_object("to_int_value", 8);
        self.emit(abi::store_u64(
            &value.location,
            abi::stack_pointer(),
            value_slot,
        ));
        self.reset_temporary_registers();
        let source = self.allocate_register()?;
        self.emit(abi::load_u64(&source, abi::stack_pointer(), value_slot));
        match value.type_.as_str() {
            "Fixed" => self.emit_fixed_to_int_value(&source),
            "Float" => self.emit_float_to_int_value(&source),
            "String" => self.emit_string_to_int_value(&source),
            other => Err(format!(
                "native toInt does not accept argument type '{other}'"
            )),
        }
    }

    pub(super) fn emit_fixed_to_int_value(
        &mut self,
        source_register: &str,
    ) -> Result<ValueResult, String> {
        let value = "x8";
        let result = self.allocate_register()?;
        let nonnegative = self.label("fixed_to_int_nonnegative");
        let done = self.label("fixed_to_int_done");
        self.emit(abi::move_register(value, source_register));
        self.emit(abi::compare_immediate(value, "0"));
        self.emit(abi::branch_ge(&nonnegative));
        self.emit(abi::subtract_registers(&result, "xzr", value));
        self.emit(abi::shift_right_immediate(&result, &result, 32));
        self.emit(abi::subtract_registers(&result, "xzr", &result));
        self.emit(abi::branch(&done));
        self.emit(abi::label(&nonnegative));
        self.emit(abi::arithmetic_shift_right_immediate(&result, value, 32));
        self.emit(abi::label(&done));
        Ok(ValueResult {
            type_: "Integer".to_string(),
            location: result,
            text: "toInt(Fixed)".to_string(),
        })
    }

    pub(super) fn emit_float_to_int_value(
        &mut self,
        source_register: &str,
    ) -> Result<ValueResult, String> {
        let bits = "x8";
        let exponent = "x9";
        let mantissa = "x10";
        let sign = "x11";
        let mask = "x12";
        let ok = self.label("float_to_int_ok");
        let check_edge = self.label("float_to_int_check_edge");
        let edge_sign_ok = self.label("float_to_int_edge_sign_ok");
        let overflow = self.label("float_to_int_overflow");
        let invalid = self.label("float_to_int_invalid");
        let result = self.allocate_register()?;

        self.emit(abi::move_register(bits, source_register));
        self.emit(abi::shift_right_immediate(exponent, bits, 52));
        self.emit(abi::move_immediate(mask, "Integer", "2047"));
        self.emit(abi::and_registers(exponent, exponent, mask));
        self.emit(abi::compare_immediate(exponent, "2047"));
        self.emit(abi::branch_eq(&invalid));
        self.emit(abi::compare_immediate(exponent, "1086"));
        self.emit(abi::branch_lt(&ok));
        self.emit(abi::branch_eq(&check_edge));
        self.emit(abi::branch(&overflow));

        self.emit(abi::label(&check_edge));
        self.emit(abi::shift_right_immediate(sign, bits, 63));
        self.emit(abi::compare_immediate(sign, "1"));
        self.emit(abi::branch_eq(&edge_sign_ok));
        self.emit(abi::branch(&overflow));
        self.emit(abi::label(&edge_sign_ok));
        self.emit(abi::move_immediate(mask, "Integer", "4503599627370495"));
        self.emit(abi::and_registers(mantissa, bits, mask));
        self.emit(abi::compare_immediate(mantissa, "0"));
        self.emit(abi::branch_ne(&overflow));

        self.emit(abi::label(&ok));
        self.emit(abi::float_move_d_from_x("d0", bits));
        self.emit(abi::float_convert_to_signed_x(&result, "d0"));
        let done = self.label("float_to_int_done");
        self.emit(abi::branch(&done));
        self.emit(abi::label(&invalid));
        self.emit_invalid_format_return()?;
        self.emit(abi::label(&overflow));
        self.emit_overflow_return()?;
        self.emit(abi::label(&done));

        Ok(ValueResult {
            type_: "Integer".to_string(),
            location: result,
            text: "toInt(Float)".to_string(),
        })
    }

    pub(super) fn emit_string_to_int_value(
        &mut self,
        source_register: &str,
    ) -> Result<ValueResult, String> {
        let string = "x8";
        let length = "x9";
        let index = "x10";
        let cursor = "x11";
        let byte = "x12";
        let acc = "x13";
        let negative = "x14";
        let digit = "x15";
        let cutoff = "x16";
        let cutlim = "x17";
        let ten = "x6";
        let invalid = self.label("string_to_int_invalid");
        let overflow = self.label("string_to_int_overflow");
        let first_not_minus = self.label("string_to_int_first_not_minus");
        let sign_done = self.label("string_to_int_sign_done");
        let loop_start = self.label("string_to_int_loop");
        let loop_done = self.label("string_to_int_done");
        let cutoff_equal = self.label("string_to_int_cutoff_equal");
        let digit_ok = self.label("string_to_int_digit_ok");
        let positive = self.label("string_to_int_positive");
        let done = self.label("string_to_int_return");
        let result = self.allocate_register()?;

        self.emit(abi::move_register(string, source_register));
        self.emit(abi::load_u64(length, string, 0));
        self.emit(abi::compare_immediate(length, "0"));
        self.emit(abi::branch_eq(&invalid));
        self.emit(abi::add_immediate(cursor, string, 8));
        self.emit(abi::move_immediate(index, "Integer", "0"));
        self.emit(abi::move_immediate(acc, "Integer", "0"));
        self.emit(abi::move_immediate(negative, "Integer", "0"));
        self.emit(abi::load_u8(byte, cursor, 0));
        self.emit(abi::compare_immediate(byte, "45"));
        self.emit(abi::branch_ne(&first_not_minus));
        self.emit(abi::move_immediate(negative, "Integer", "1"));
        self.emit(abi::add_immediate(index, index, 1));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::branch(&sign_done));
        self.emit(abi::label(&first_not_minus));
        self.emit(abi::compare_immediate(byte, "43"));
        self.emit(abi::branch_ne(&sign_done));
        self.emit(abi::add_immediate(index, index, 1));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::label(&sign_done));
        self.emit(abi::compare_registers(index, length));
        self.emit(abi::branch_ge(&invalid));
        self.emit(abi::move_immediate(cutoff, "Integer", "922337203685477580"));
        self.emit(abi::move_immediate(cutlim, "Integer", "7"));
        self.emit(abi::compare_immediate(negative, "0"));
        let limit_ready = self.label("string_to_int_limit_ready");
        self.emit(abi::branch_eq(&limit_ready));
        self.emit(abi::move_immediate(cutlim, "Integer", "8"));
        self.emit(abi::label(&limit_ready));
        self.emit(abi::move_immediate(ten, "Integer", "10"));

        self.emit(abi::label(&loop_start));
        self.emit(abi::compare_registers(index, length));
        self.emit(abi::branch_ge(&loop_done));
        self.emit(abi::load_u8(byte, cursor, 0));
        self.emit(abi::compare_immediate(byte, "48"));
        self.emit(abi::branch_lo(&invalid));
        self.emit(abi::compare_immediate(byte, "57"));
        self.emit(abi::branch_hi(&invalid));
        self.emit(abi::subtract_immediate(digit, byte, 48));
        self.emit(abi::compare_registers(acc, cutoff));
        self.emit(abi::branch_gt(&overflow));
        self.emit(abi::branch_eq(&cutoff_equal));
        self.emit(abi::branch(&digit_ok));
        self.emit(abi::label(&cutoff_equal));
        self.emit(abi::compare_registers(digit, cutlim));
        self.emit(abi::branch_gt(&overflow));
        self.emit(abi::label(&digit_ok));
        self.emit(abi::multiply_registers(acc, acc, ten));
        self.emit(abi::add_registers(acc, acc, digit));
        self.emit(abi::add_immediate(index, index, 1));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::branch(&loop_start));

        self.emit(abi::label(&loop_done));
        self.emit(abi::compare_immediate(negative, "0"));
        self.emit(abi::branch_eq(&positive));
        self.emit(abi::subtract_registers(&result, "xzr", acc));
        self.emit(abi::branch(&done));
        self.emit(abi::label(&positive));
        self.emit(abi::move_register(&result, acc));
        self.emit(abi::branch(&done));
        self.emit(abi::label(&invalid));
        self.emit_invalid_format_return()?;
        self.emit(abi::label(&overflow));
        self.emit_overflow_return()?;
        self.emit(abi::label(&done));

        Ok(ValueResult {
            type_: "Integer".to_string(),
            location: result,
            text: "toInt(String)".to_string(),
        })
    }

    pub(super) fn lower_to_byte(&mut self, arg: &NirValue) -> Result<ValueResult, String> {
        let value = self.lower_value(arg)?;
        if value.type_ != "Integer" {
            return Err(format!(
                "native toByte does not accept argument type '{}'",
                value.type_
            ));
        }
        let result = self.allocate_register()?;
        let overflow = self.label("to_byte_overflow");
        let ok = self.label("to_byte_ok");
        self.emit(abi::compare_immediate(&value.location, "0"));
        self.emit(abi::branch_lt(&overflow));
        self.emit(abi::compare_immediate(&value.location, "255"));
        self.emit(abi::branch_hi(&overflow));
        self.emit(abi::move_register(&result, &value.location));
        self.emit(abi::branch(&ok));
        self.emit(abi::label(&overflow));
        self.emit_overflow_return()?;
        self.emit(abi::label(&ok));
        Ok(ValueResult {
            type_: "Byte".to_string(),
            location: result,
            text: format!("toByte({})", value.text),
        })
    }

    pub(super) fn lower_to_float(&mut self, arg: &NirValue) -> Result<ValueResult, String> {
        let value = self.lower_value(arg)?;
        let value_slot = self.allocate_stack_object("to_float_value", 8);
        self.emit(abi::store_u64(
            &value.location,
            abi::stack_pointer(),
            value_slot,
        ));
        self.reset_temporary_registers();
        let source = self.allocate_register()?;
        self.emit(abi::load_u64(&source, abi::stack_pointer(), value_slot));
        let result = self.allocate_register()?;
        match value.type_.as_str() {
            "Integer" => {
                self.emit(abi::signed_convert_to_float_d("d0", &source));
                self.emit(abi::float_move_x_from_d(&result, "d0"));
            }
            "Fixed" => {
                let temp = ValueResult {
                    type_: "Fixed".to_string(),
                    location: source,
                    text: value.text.clone(),
                };
                self.load_numeric_as_double("d0", &temp)?;
                self.emit(abi::float_move_x_from_d(&result, "d0"));
            }
            "String" => {
                let invalid = self.label("to_float_invalid");
                let overflow = self.label("to_float_overflow");
                self.emit_parse_decimal_string_to_double(&source, &invalid)?;
                self.emit_double_overflow_check("d0", &overflow);
                self.emit(abi::float_move_x_from_d(&result, "d0"));
                let done = self.label("to_float_done");
                self.emit(abi::branch(&done));
                self.emit(abi::label(&invalid));
                self.emit_invalid_format_return()?;
                self.emit(abi::label(&overflow));
                self.emit_overflow_return()?;
                self.emit(abi::label(&done));
            }
            other => {
                return Err(format!(
                    "native toFloat does not accept argument type '{other}'"
                ))
            }
        }
        Ok(ValueResult {
            type_: "Float".to_string(),
            location: result,
            text: format!("toFloat({})", value.text),
        })
    }

    pub(super) fn lower_to_fixed(&mut self, arg: &NirValue) -> Result<ValueResult, String> {
        let value = self.lower_value(arg)?;
        let value_slot = self.allocate_stack_object("to_fixed_value", 8);
        self.emit(abi::store_u64(
            &value.location,
            abi::stack_pointer(),
            value_slot,
        ));
        self.reset_temporary_registers();
        let source = self.allocate_register()?;
        self.emit(abi::load_u64(&source, abi::stack_pointer(), value_slot));
        let result = self.allocate_register()?;
        match value.type_.as_str() {
            "Integer" => {
                self.emit_integer_to_fixed_value(&source, &result)?;
            }
            "Float" => {
                self.emit_float_bits_to_fixed_value(&source, &result)?;
            }
            "String" => {
                let invalid = self.label("to_fixed_invalid");
                let overflow = self.label("to_fixed_overflow");
                self.emit_parse_decimal_string_to_double(&source, &invalid)?;
                self.emit_double_overflow_check("d0", &overflow);
                let parsed_bits = "x8";
                self.emit(abi::float_move_x_from_d(parsed_bits, "d0"));
                self.emit_float_bits_to_fixed_value(parsed_bits, &result)?;
                let done = self.label("to_fixed_done");
                self.emit(abi::branch(&done));
                self.emit(abi::label(&invalid));
                self.emit_invalid_format_return()?;
                self.emit(abi::label(&overflow));
                self.emit_overflow_return()?;
                self.emit(abi::label(&done));
            }
            other => {
                return Err(format!(
                    "native toFixed does not accept argument type '{other}'"
                ))
            }
        }
        Ok(ValueResult {
            type_: "Fixed".to_string(),
            location: result,
            text: format!("toFixed({})", value.text),
        })
    }

    pub(super) fn lower_is_numeric(&mut self, arg: &NirValue) -> Result<ValueResult, String> {
        let value = self.lower_value(arg)?;
        if value.type_ != "String" {
            return Err(format!(
                "native isNumeric does not accept argument type '{}'",
                value.type_
            ));
        }
        let value_slot = self.allocate_stack_object("is_numeric_value", 8);
        self.emit(abi::store_u64(
            &value.location,
            abi::stack_pointer(),
            value_slot,
        ));
        self.reset_temporary_registers();
        let source = self.allocate_register()?;
        self.emit(abi::load_u64(&source, abi::stack_pointer(), value_slot));
        let invalid = self.label("is_numeric_false");
        let done = self.label("is_numeric_done");
        let result = self.allocate_register()?;
        self.emit_parse_decimal_string_to_double(&source, &invalid)?;
        self.emit_double_overflow_check("d0", &invalid);
        self.emit(abi::move_immediate(&result, "Boolean", "true"));
        self.emit(abi::branch(&done));
        self.emit(abi::label(&invalid));
        self.emit(abi::move_immediate(&result, "Boolean", "false"));
        self.emit(abi::label(&done));
        Ok(ValueResult {
            type_: "Boolean".to_string(),
            location: result,
            text: format!("isNumeric({})", value.text),
        })
    }

    pub(super) fn lower_integer_parity_predicate(
        &mut self,
        name: &str,
        arg: &NirValue,
        odd: bool,
    ) -> Result<ValueResult, String> {
        let value = self.lower_value(arg)?;
        if value.type_ != "Integer" {
            return Err(format!(
                "native {name} does not accept argument type '{}'",
                value.type_
            ));
        }

        let mask = self.allocate_register()?;
        let result = self.allocate_register()?;
        let true_label = self.label(name);
        let done_label = self.label(&format!("{name}_done"));
        self.emit(abi::move_immediate(&mask, "Integer", "1"));
        self.emit(abi::and_registers(&mask, &value.location, &mask));
        self.emit(abi::compare_immediate(&mask, if odd { "1" } else { "0" }));
        self.emit(abi::branch_eq(&true_label));
        self.emit(abi::move_immediate(&result, "Boolean", "false"));
        self.emit(abi::branch(&done_label));
        self.emit(abi::label(&true_label));
        self.emit(abi::move_immediate(&result, "Boolean", "true"));
        self.emit(abi::label(&done_label));

        Ok(ValueResult {
            type_: "Boolean".to_string(),
            location: result,
            text: format!("{name}({})", value.text),
        })
    }

    pub(super) fn lower_numeric_filter_predicate(
        &mut self,
        name: &str,
        arg: &NirValue,
    ) -> Result<ValueResult, String> {
        let value = self.lower_value(arg)?;
        let result = self.allocate_register()?;
        let true_label = self.label(name);
        let done_label = self.label(&format!("{name}_done"));

        match value.type_.as_str() {
            "Integer" | "Fixed" => self.emit(abi::compare_immediate(&value.location, "0")),
            "Float" => {
                self.emit(abi::float_move_d_from_x("d0", &value.location));
                self.emit(abi::float_compare_zero_d("d0"));
            }
            other => {
                return Err(format!(
                    "native {name} does not accept argument type '{other}'"
                ));
            }
        }

        match name {
            "isPositive" => self.emit(abi::branch_gt(&true_label)),
            "isNegative" => self.emit(abi::branch_lt(&true_label)),
            "isZero" => self.emit(abi::branch_eq(&true_label)),
            other => {
                return Err(format!(
                    "native filter predicate '{other}' is not implemented"
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
            text: format!("{name}({})", value.text),
        })
    }

    pub(super) fn lower_empty_filter_predicate(
        &mut self,
        name: &str,
        arg: &NirValue,
    ) -> Result<ValueResult, String> {
        let len = self.lower_len(arg)?;
        let result = self.allocate_register()?;
        let true_label = self.label(name);
        let done_label = self.label(&format!("{name}_done"));

        self.emit(abi::compare_immediate(&len.location, "0"));
        match name {
            "isEmpty" => self.emit(abi::branch_eq(&true_label)),
            "isNotEmpty" => self.emit(abi::branch_ne(&true_label)),
            other => {
                return Err(format!(
                    "native filter predicate '{other}' is not implemented"
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
            text: format!("{name}({})", len.text),
        })
    }

    pub(super) fn emit_integer_to_fixed_value(
        &mut self,
        source: &str,
        result: &str,
    ) -> Result<(), String> {
        let min = self.allocate_register()?;
        let max = self.allocate_register()?;
        let overflow = self.label("int_to_fixed_overflow");
        let ok = self.label("int_to_fixed_ok");
        self.emit(abi::move_immediate(&min, "Integer", "18446744071562067968"));
        self.emit(abi::compare_registers(source, &min));
        self.emit(abi::branch_lt(&overflow));
        self.emit(abi::move_immediate(&max, "Integer", "2147483647"));
        self.emit(abi::compare_registers(source, &max));
        self.emit(abi::branch_gt(&overflow));
        self.emit(abi::shift_left_immediate(result, source, 32));
        self.emit(abi::branch(&ok));
        self.emit(abi::label(&overflow));
        self.emit_overflow_return()?;
        self.emit(abi::label(&ok));
        Ok(())
    }

    pub(super) fn emit_float_bits_to_fixed_value(
        &mut self,
        source: &str,
        result: &str,
    ) -> Result<(), String> {
        let exponent = "x9";
        let mask = "x10";
        let sign = "x11";
        let mantissa = "x12";
        let invalid = self.label("float_to_fixed_invalid");
        let overflow = self.label("float_to_fixed_overflow");
        let ok = self.label("float_to_fixed_ok");
        let edge = self.label("float_to_fixed_edge");
        let edge_negative = self.label("float_to_fixed_edge_negative");
        let range_ok = self.label("float_to_fixed_range_ok");
        self.emit(abi::move_register("x8", source));
        self.emit(abi::shift_right_immediate(exponent, "x8", 52));
        self.emit(abi::move_immediate(mask, "Integer", "2047"));
        self.emit(abi::and_registers(exponent, exponent, mask));
        self.emit(abi::compare_immediate(exponent, "2047"));
        self.emit(abi::branch_eq(&invalid));
        self.emit(abi::compare_immediate(exponent, "1054"));
        self.emit(abi::branch_lt(&range_ok));
        self.emit(abi::branch_eq(&edge));
        self.emit(abi::branch(&overflow));
        self.emit(abi::label(&edge));
        self.emit(abi::shift_right_immediate(sign, "x8", 63));
        self.emit(abi::compare_immediate(sign, "1"));
        self.emit(abi::branch_eq(&edge_negative));
        self.emit(abi::branch(&overflow));
        self.emit(abi::label(&edge_negative));
        self.emit(abi::move_immediate(mask, "Integer", "4503599627370495"));
        self.emit(abi::and_registers(mantissa, "x8", mask));
        self.emit(abi::compare_immediate(mantissa, "0"));
        self.emit(abi::branch_ne(&overflow));
        self.emit(abi::label(&range_ok));
        self.emit(abi::float_move_d_from_x("d0", "x8"));
        self.emit_f64_const("d1", "x17", 4_294_967_296.0);
        self.emit(abi::float_multiply_d("d0", "d0", "d1"));
        // Round to nearest representable Fixed (ties away from zero) rather than
        // truncating toward zero, as `toFixed(Float)`/`toFixed(String)` require.
        self.emit(abi::float_round_to_signed_x(result, "d0"));
        self.emit(abi::branch(&ok));
        self.emit(abi::label(&invalid));
        self.emit_invalid_format_return()?;
        self.emit(abi::label(&overflow));
        self.emit_overflow_return()?;
        self.emit(abi::label(&ok));
        Ok(())
    }

    pub(super) fn emit_parse_decimal_string_to_double(
        &mut self,
        source_register: &str,
        invalid_label: &str,
    ) -> Result<(), String> {
        let string = "x8";
        let length = "x9";
        let index = "x10";
        let cursor = "x11";
        let byte = "x12";
        let digit = "x13";
        let negative = "x14";
        let seen_digit = "x15";
        let ten_bits = "x16";
        let dot_seen = "x17";
        let exponent = "x4";
        let exponent_negative = "x3";
        let exponent_ten = "x2";
        let loop_start = self.label("parse_decimal_loop");
        let after_sign = self.label("parse_decimal_after_sign");
        let not_minus = self.label("parse_decimal_not_minus");
        let sign_done = self.label("parse_decimal_sign_done");
        let dot = self.label("parse_decimal_dot");
        let frac_digit = self.label("parse_decimal_frac_digit");
        let int_digit = self.label("parse_decimal_int_digit");
        let next = self.label("parse_decimal_next");
        let finish = self.label("parse_decimal_finish");
        let positive = self.label("parse_decimal_positive");
        let exponent_start = self.label("parse_decimal_exponent_start");
        let exponent_not_minus = self.label("parse_decimal_exponent_not_minus");
        let exponent_sign_done = self.label("parse_decimal_exponent_sign_done");
        let exponent_loop = self.label("parse_decimal_exponent_loop");
        let exponent_apply = self.label("parse_decimal_exponent_apply");
        let exponent_multiply_loop = self.label("parse_decimal_exponent_multiply_loop");
        let exponent_divide_loop = self.label("parse_decimal_exponent_divide_loop");
        let exponent_apply_done = self.label("parse_decimal_exponent_apply_done");
        let exponent_skip_accum = self.label("parse_decimal_exponent_skip_accum");
        self.emit(abi::move_register(string, source_register));
        self.emit(abi::load_u64(length, string, 0));
        self.emit(abi::compare_immediate(length, "0"));
        self.emit(abi::branch_eq(invalid_label));
        self.emit(abi::add_immediate(cursor, string, 8));
        self.emit(abi::move_immediate(index, "Integer", "0"));
        self.emit(abi::move_immediate(negative, "Integer", "0"));
        self.emit(abi::move_immediate(seen_digit, "Integer", "0"));
        self.emit(abi::move_immediate(dot_seen, "Integer", "0"));
        self.emit(abi::move_immediate(exponent_ten, "Integer", "10"));
        self.emit(abi::move_immediate("x0", "Integer", "0"));
        self.emit(abi::signed_convert_to_float_d("d0", "x0"));
        self.emit_f64_const("d1", ten_bits, 10.0);
        self.emit_f64_const("d3", "x7", 1.0);
        self.emit(abi::load_u8(byte, cursor, 0));
        self.emit(abi::compare_immediate(byte, "45"));
        self.emit(abi::branch_ne(&not_minus));
        self.emit(abi::move_immediate(negative, "Integer", "1"));
        self.emit(abi::branch(&after_sign));
        self.emit(abi::label(&not_minus));
        self.emit(abi::compare_immediate(byte, "43"));
        self.emit(abi::branch_ne(&sign_done));
        self.emit(abi::label(&after_sign));
        self.emit(abi::add_immediate(index, index, 1));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::compare_registers(index, length));
        self.emit(abi::branch_ge(invalid_label));
        self.emit(abi::label(&sign_done));

        self.emit(abi::label(&loop_start));
        self.emit(abi::compare_registers(index, length));
        self.emit(abi::branch_ge(&finish));
        self.emit(abi::load_u8(byte, cursor, 0));
        self.emit(abi::compare_immediate(byte, "46"));
        self.emit(abi::branch_eq(&dot));
        self.emit(abi::compare_immediate(byte, "69"));
        self.emit(abi::branch_eq(&exponent_start));
        self.emit(abi::compare_immediate(byte, "101"));
        self.emit(abi::branch_eq(&exponent_start));
        self.emit(abi::compare_immediate(byte, "48"));
        self.emit(abi::branch_lo(invalid_label));
        self.emit(abi::compare_immediate(byte, "57"));
        self.emit(abi::branch_hi(invalid_label));
        self.emit(abi::subtract_immediate(digit, byte, 48));
        self.emit(abi::signed_convert_to_float_d("d2", digit));
        self.emit(abi::move_immediate(seen_digit, "Integer", "1"));
        self.emit(abi::compare_immediate(dot_seen, "0"));
        self.emit(abi::branch_ne(&frac_digit));
        self.emit(abi::label(&int_digit));
        self.emit(abi::float_multiply_d("d0", "d0", "d1"));
        self.emit(abi::float_add_d("d0", "d0", "d2"));
        self.emit(abi::branch(&next));
        self.emit(abi::label(&frac_digit));
        self.emit(abi::float_multiply_d("d3", "d3", "d1"));
        self.emit(abi::float_divide_d("d2", "d2", "d3"));
        self.emit(abi::float_add_d("d0", "d0", "d2"));
        self.emit(abi::branch(&next));
        self.emit(abi::label(&dot));
        self.emit(abi::compare_immediate(dot_seen, "0"));
        self.emit(abi::branch_ne(invalid_label));
        self.emit(abi::move_immediate(dot_seen, "Integer", "1"));
        self.emit(abi::label(&next));
        self.emit(abi::add_immediate(index, index, 1));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::branch(&loop_start));

        self.emit(abi::label(&exponent_start));
        self.emit(abi::compare_immediate(seen_digit, "0"));
        self.emit(abi::branch_eq(invalid_label));
        self.emit(abi::add_immediate(index, index, 1));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::compare_registers(index, length));
        self.emit(abi::branch_ge(invalid_label));
        self.emit(abi::move_immediate(exponent, "Integer", "0"));
        self.emit(abi::move_immediate(exponent_negative, "Integer", "0"));
        self.emit(abi::move_immediate(seen_digit, "Integer", "0"));
        self.emit(abi::load_u8(byte, cursor, 0));
        self.emit(abi::compare_immediate(byte, "45"));
        self.emit(abi::branch_ne(&exponent_not_minus));
        self.emit(abi::move_immediate(exponent_negative, "Integer", "1"));
        self.emit(abi::add_immediate(index, index, 1));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::branch(&exponent_sign_done));
        self.emit(abi::label(&exponent_not_minus));
        self.emit(abi::compare_immediate(byte, "43"));
        self.emit(abi::branch_ne(&exponent_sign_done));
        self.emit(abi::add_immediate(index, index, 1));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::label(&exponent_sign_done));
        self.emit(abi::compare_registers(index, length));
        self.emit(abi::branch_ge(invalid_label));

        self.emit(abi::label(&exponent_loop));
        self.emit(abi::compare_registers(index, length));
        self.emit(abi::branch_ge(&exponent_apply));
        self.emit(abi::load_u8(byte, cursor, 0));
        self.emit(abi::compare_immediate(byte, "48"));
        self.emit(abi::branch_lo(invalid_label));
        self.emit(abi::compare_immediate(byte, "57"));
        self.emit(abi::branch_hi(invalid_label));
        self.emit(abi::subtract_immediate(digit, byte, 48));
        self.emit(abi::move_immediate(seen_digit, "Integer", "1"));
        // Clamp exponent accumulation to avoid 64-bit wraparound on absurdly
        // large exponents (e.g. `1e18446744073709551616`). Once the magnitude
        // reaches EXPONENT_CLAMP, any representable mantissa is already forced to
        // overflow to infinity (positive exponent) or underflow to zero
        // (negative exponent), so additional digits cannot change the result.
        // Skipping further accumulation keeps the register far below 2^63 and
        // preserves the overflow/underflow outcome instead of wrapping to a
        // small, wrongly-accepted value.
        self.emit(abi::compare_immediate(exponent, DECIMAL_EXPONENT_CLAMP));
        self.emit(abi::branch_ge(&exponent_skip_accum));
        self.emit(abi::multiply_registers(exponent, exponent, exponent_ten));
        self.emit(abi::add_registers(exponent, exponent, digit));
        self.emit(abi::label(&exponent_skip_accum));
        self.emit(abi::add_immediate(index, index, 1));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::branch(&exponent_loop));

        self.emit(abi::label(&exponent_apply));
        self.emit(abi::compare_immediate(seen_digit, "0"));
        self.emit(abi::branch_eq(invalid_label));
        self.emit(abi::compare_immediate(exponent_negative, "0"));
        self.emit(abi::branch_ne(&exponent_divide_loop));
        self.emit(abi::label(&exponent_multiply_loop));
        self.emit(abi::compare_immediate(exponent, "0"));
        self.emit(abi::branch_eq(&exponent_apply_done));
        self.emit(abi::float_multiply_d("d0", "d0", "d1"));
        self.emit(abi::subtract_immediate(exponent, exponent, 1));
        self.emit(abi::branch(&exponent_multiply_loop));
        self.emit(abi::label(&exponent_divide_loop));
        self.emit(abi::compare_immediate(exponent, "0"));
        self.emit(abi::branch_eq(&exponent_apply_done));
        self.emit(abi::float_divide_d("d0", "d0", "d1"));
        self.emit(abi::subtract_immediate(exponent, exponent, 1));
        self.emit(abi::branch(&exponent_divide_loop));
        self.emit(abi::label(&exponent_apply_done));
        self.emit(abi::move_immediate(seen_digit, "Integer", "1"));
        self.emit(abi::branch(&finish));

        self.emit(abi::label(&finish));
        self.emit(abi::compare_immediate(seen_digit, "0"));
        self.emit(abi::branch_eq(invalid_label));
        self.emit(abi::compare_immediate(negative, "0"));
        self.emit(abi::branch_eq(&positive));
        self.emit(abi::float_negate_d("d0", "d0"));
        self.emit(abi::label(&positive));
        Ok(())
    }

    pub(super) fn emit_double_overflow_check(&mut self, source: &str, overflow_label: &str) {
        self.emit(abi::float_move_x_from_d("x6", source));
        self.emit(abi::shift_right_immediate("x7", "x6", 52));
        self.emit(abi::move_immediate("x5", "Integer", "2047"));
        self.emit(abi::and_registers("x7", "x7", "x5"));
        self.emit(abi::compare_immediate("x7", "2047"));
        self.emit(abi::branch_eq(overflow_label));
    }
}
