// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::error::constants::*;
use crate::codegen::string::format::float_parse::STRING_TO_FLOAT_SYMBOL;
use crate::target::shared::abi;
use crate::target::shared::nir::*;
use crate::types::ParameterType;

mod to_byte_scalar;
mod to_int;

impl CodeBuilder<'_> {
    pub(crate) fn lower_to_float(&mut self, arg: &NirValue) -> Result<ValueResult, String> {
        let value = self.lower_value(arg)?;
        let value_slot = self.allocate_stack_object("to_float_value", 8);
        self.emit(abi::store_u64(
            &value.location,
            abi::stack_pointer(),
            value_slot,
        ));
        self.reset_temporary_registers();
        let source = self.allocate_register();
        self.emit(abi::load_u64(&source, abi::stack_pointer(), value_slot));
        let result = self.allocate_register();
        match &value.type_ {
            ParameterType::Integer => {
                self.emit(abi::signed_convert_to_float_d(abi::FP_SCRATCH[0], &source));
                self.emit(abi::float_move_x_from_d(&result, abi::FP_SCRATCH[0]));
            }
            ParameterType::Fixed => {
                let temp = ValueResult {
                    origin: None,
                    type_: ParameterType::Fixed,
                    location: Operand::from(source),
                    text: value.text.clone(),
                };
                self.load_numeric_as_double(abi::FP_SCRATCH[0], &temp)?;
                self.emit(abi::float_move_x_from_d(&result, abi::FP_SCRATCH[0]));
            }
            // `toFloat(Money)` = `raw / 100000.0` (plan-29-G §4.3).
            ParameterType::Money => {
                let temp = ValueResult {
                    origin: None,
                    type_: ParameterType::Money,
                    location: Operand::from(source),
                    text: value.text.clone(),
                };
                self.load_numeric_as_double(abi::FP_SCRATCH[0], &temp)?;
                self.emit(abi::float_move_x_from_d(&result, abi::FP_SCRATCH[0]));
            }
            ParameterType::String => {
                let invalid = self.label("to_float_invalid");
                let overflow = self.label("to_float_overflow");
                self.emit_parse_decimal_string_to_double(&source, &invalid)?;
                self.emit_double_overflow_check(abi::FP_SCRATCH[0], &overflow);
                self.emit(abi::float_move_x_from_d(&result, abi::FP_SCRATCH[0]));
                let done = self.label("to_float_done");
                self.emit(abi::branch(&done));
                self.emit(abi::label(&invalid));
                self.raise_error("toFloat", "ErrInvalidFormat")?;
                self.emit(abi::label(&overflow));
                self.raise_error("toFloat", "ErrOverflow")?;
                self.emit(abi::label(&done));
            }
            other => {
                return Err(format!(
                    "native toFloat does not accept argument type '{other}'"
                ))
            }
        }
        Ok(ValueResult {
            origin: None,
            type_: ParameterType::Float,
            location: Operand::from(result),
            text: format!("toFloat({})", value.text),
        })
    }

    pub(crate) fn lower_to_fixed(&mut self, arg: &NirValue) -> Result<ValueResult, String> {
        let value = self.lower_value(arg)?;
        // A `Float` argument is an observation boundary (spec `04_types.md`:
        // "passed as an argument" includes every built-in's argument), so a
        // non-finite arithmetic result raises ErrFloatOverflow/ErrFloatNaN here,
        // as it does for `toString`, rather than reaching the conversion (bug-807).
        self.observe_float(arg, &value)?;
        // A `d`-native float's bits are read by the conversion, so materialize it
        // into a GPR first (plan-01 float-dnative).
        let value = self.materialize_float(value)?;
        let value_slot = self.allocate_stack_object("to_fixed_value", 8);
        self.emit(abi::store_u64(
            &value.location,
            abi::stack_pointer(),
            value_slot,
        ));
        self.reset_temporary_registers();
        let source = self.allocate_register();
        self.emit(abi::load_u64(&source, abi::stack_pointer(), value_slot));
        let result = self.allocate_register();
        match &value.type_ {
            ParameterType::Integer => {
                self.emit_integer_to_fixed_value(&source, &result)?;
            }
            ParameterType::Float => {
                self.emit_float_bits_to_fixed_value(&source, &result)?;
            }
            // `toFixed(Money)` = `raw * 2^32 / 100000`, rounded once under the
            // Money mode (plan-29-G §4.3, bug-812); the range check traps a Money
            // too large for Fixed's 32-bit integer part. The divisor is the
            // nonzero constant scale, so there is no zero guard.
            ParameterType::Money => {
                let scale = self.allocate_register();
                self.emit(abi::move_immediate(&scale, "Integer", "100000"));
                self.emit_money_fixed_divide(&result, &source, &scale, true)?;
            }
            ParameterType::String => {
                let invalid = self.label("to_fixed_invalid");
                let overflow = self.label("to_fixed_overflow");
                // bug-831: the text converts EXACTLY (integer arithmetic, one
                // rounding), as the `Fixed` literal path does; no binary64 step.
                self.emit_parse_decimal_string_to_fixed_raw(&source, &result, &invalid, &overflow)?;
                let done = self.label("to_fixed_done");
                self.emit(abi::branch(&done));
                self.emit(abi::label(&invalid));
                self.raise_error("toFixed", "ErrInvalidFormat")?;
                self.emit(abi::label(&overflow));
                self.raise_error("toFixed", "ErrOverflow")?;
                self.emit(abi::label(&done));
            }
            other => {
                return Err(format!(
                    "native toFixed does not accept argument type '{other}'"
                ))
            }
        }
        Ok(ValueResult {
            origin: None,
            type_: ParameterType::Fixed,
            location: Operand::from(result),
            text: format!("toFixed({})", value.text),
        })
    }

    /// `toMoney(value)` — the explicit crossing *into* Money from every type
    /// (plan-29-G §4.2). Integer/Byte scale by 100000; Fixed rescales through the
    /// 128-bit, mode-rounded `emit_money_fixed_multiply`; Float and String go
    /// through f64 and the mode-aware round, guarding finiteness and range.
    pub(crate) fn lower_to_money(&mut self, arg: &NirValue) -> Result<ValueResult, String> {
        let value = self.lower_value(arg)?;
        // A `Float` argument is an observation boundary (spec `04_types.md`:
        // "passed as an argument" includes every built-in's argument), so a
        // non-finite arithmetic result raises ErrFloatOverflow/ErrFloatNaN here,
        // as it does for `toString`, rather than reaching the conversion (bug-807).
        self.observe_float(arg, &value)?;
        // Read a `d`-native float's bits from a GPR for the Float conversion.
        let value = self.materialize_float(value)?;
        let value_slot = self.allocate_stack_object("to_money_value", 8);
        self.emit(abi::store_u64(
            &value.location,
            abi::stack_pointer(),
            value_slot,
        ));
        self.reset_temporary_registers();
        let source = self.allocate_register();
        self.emit(abi::load_u64(&source, abi::stack_pointer(), value_slot));
        let result = self.allocate_register();
        let scratch = self.temporary_vreg();
        match &value.type_ {
            // Exact: `value * 100000`, overflow-checked for Integer (a Byte is
            // always in range: 255 * 100000 fits i64).
            ParameterType::Integer => {
                let scale = self.allocate_register();
                self.emit(abi::move_immediate(&scale, "Integer", "100000"));
                self.emit_checked_integer_multiply(&result, &source, &scale)?;
            }
            ParameterType::Byte => {
                let scale = self.allocate_register();
                self.emit(abi::move_immediate(&scale, "Integer", "100000"));
                self.emit(abi::multiply_registers(&result, &source, &scale));
            }
            // `fixed_raw * 100000 / 2^32`, rounded once under the Money mode
            // (bug-812). Unchecked: a Fixed is at most 2^31 in magnitude, so the
            // Money raw is below 2^48 and always fits (bug-807).
            ParameterType::Fixed => {
                let scale = self.allocate_register();
                self.emit(abi::move_immediate(&scale, "Integer", "100000"));
                self.emit_money_fixed_multiply(&result, &source, &scale, false)?;
            }
            // Inexact: finiteness → ErrInvalidFormat, `value * 100000.0` rounded
            // under the mode, range → ErrOverflow.
            ParameterType::Float => {
                let fval = self.allocate_fp_register();
                self.emit(abi::float_move_d_from_x(&fval, &source));
                self.emit_float_finite_or_invalid(&fval)?;
                let scale = self.allocate_fp_register();
                self.emit_f64_const(&scale, &scratch, 100_000.0);
                let scaled = self.allocate_fp_register();
                self.emit(abi::float_multiply_d(&scaled, &fval, &scale));
                self.emit_round_double_to_money_raw(&scaled, &result)?;
            }
            // bug-449: parse the decimal text EXACTLY (integer arithmetic, no
            // f64) so Money's exact base-10 contract holds on string input — the
            // f64 path overflowed the valid max and misrounded large ties. The
            // rare scientific-notation form (`e`/`E`) falls back to the f64 parse,
            // which is approximate anyway.
            ParameterType::String => {
                let invalid = self.label("to_money_invalid");
                let overflow = self.label("to_money_overflow");
                let scientific = self.label("to_money_scientific");
                let done = self.label("to_money_done");
                self.emit_parse_decimal_string_to_money_raw(
                    &source,
                    &result,
                    &invalid,
                    &overflow,
                    &scientific,
                )?;
                self.emit(abi::branch(&done));
                // Scientific-notation fallback: f64 parse, scale, mode-round.
                self.emit(abi::label(&scientific));
                self.emit_parse_decimal_string_to_double(&source, &invalid)?;
                let parsed = self.allocate_fp_register();
                self.emit(abi::float_move_d_from_d(&parsed, abi::FP_SCRATCH[0]));
                let scale = self.allocate_fp_register();
                self.emit_f64_const(&scale, &scratch, 100_000.0);
                let scaled = self.allocate_fp_register();
                self.emit(abi::float_multiply_d(&scaled, &parsed, &scale));
                self.emit_round_double_to_money_raw(&scaled, &result)?;
                self.emit(abi::branch(&done));
                self.emit(abi::label(&overflow));
                self.raise_error("toMoney", "ErrOverflow")?;
                self.emit(abi::label(&invalid));
                self.raise_error("toMoney", "ErrInvalidFormat")?;
                self.emit(abi::label(&done));
            }
            other => {
                return Err(format!(
                    "native toMoney does not accept argument type '{other}'"
                ))
            }
        }
        Ok(ValueResult {
            origin: None,
            type_: ParameterType::Money,
            location: Operand::from(result),
            text: format!("toMoney({})", value.text),
        })
    }

    pub(crate) fn lower_is_numeric(&mut self, arg: &NirValue) -> Result<ValueResult, String> {
        let value = self.lower_value(arg)?;
        if value.type_ != ParameterType::String {
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
        let source = self.allocate_register();
        self.emit(abi::load_u64(&source, abi::stack_pointer(), value_slot));
        let invalid = self.label("is_numeric_false");
        let done = self.label("is_numeric_done");
        let result = self.allocate_register();
        self.emit_parse_decimal_string_to_double(&source, &invalid)?;
        self.emit_double_overflow_check(abi::FP_SCRATCH[0], &invalid);
        self.emit(abi::move_immediate(&result, "Boolean", "true"));
        self.emit(abi::branch(&done));
        self.emit(abi::label(&invalid));
        self.emit(abi::move_immediate(&result, "Boolean", "false"));
        self.emit(abi::label(&done));
        Ok(ValueResult {
            origin: None,
            type_: ParameterType::Boolean,
            location: Operand::from(result),
            text: format!("isNumeric({})", value.text),
        })
    }

    pub(crate) fn lower_integer_parity_predicate(
        &mut self,
        name: &str,
        arg: &NirValue,
        odd: bool,
    ) -> Result<ValueResult, String> {
        let value = self.lower_value(arg)?;
        if value.type_ != ParameterType::Integer {
            return Err(format!(
                "native {name} does not accept argument type '{}'",
                value.type_
            ));
        }

        let mask = self.allocate_register();
        let result = self.allocate_register();
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
            origin: None,
            type_: ParameterType::Boolean,
            location: Operand::from(result),
            text: format!("{name}({})", value.text),
        })
    }

    pub(crate) fn lower_numeric_filter_predicate(
        &mut self,
        name: &str,
        arg: &NirValue,
    ) -> Result<ValueResult, String> {
        let value = self.lower_value(arg)?;
        // A `Float` argument is an observation boundary (spec `04_types.md`:
        // "passed as an argument" includes every built-in's argument), so a
        // non-finite arithmetic result raises ErrFloatOverflow/ErrFloatNaN here,
        // as it does for `toString`, rather than reaching the conversion (bug-807).
        self.observe_float(arg, &value)?;
        // The predicate reads the operand's bits, so materialize a `d`-native
        // float into a GPR first (plan-01 float-dnative).
        let value = self.materialize_float(value)?;
        let result = self.allocate_register();
        let true_label = self.label(name);
        let done_label = self.label(&format!("{name}_done"));

        match &value.type_ {
            ParameterType::Integer | ParameterType::Fixed => {
                self.emit(abi::compare_immediate(&value.location, "0"))
            }
            ParameterType::Float => {
                self.emit(abi::float_move_d_from_x(
                    abi::FP_SCRATCH[0],
                    &value.location,
                ));
                self.emit(abi::float_compare_zero_d(abi::FP_SCRATCH[0]));
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
            origin: None,
            type_: ParameterType::Boolean,
            location: Operand::from(result),
            text: format!("{name}({})", value.text),
        })
    }

    pub(crate) fn lower_empty_filter_predicate(
        &mut self,
        name: &str,
        arg: &NirValue,
    ) -> Result<ValueResult, String> {
        let len = self.lower_len(arg)?;
        let result = self.allocate_register();
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
            origin: None,
            type_: ParameterType::Boolean,
            location: Operand::from(result),
            text: format!("{name}({})", len.text),
        })
    }

    pub(crate) fn emit_integer_to_fixed_value(
        &mut self,
        source: impl Into<abi::Reg>,
        result: impl Into<abi::Reg>,
    ) -> Result<(), String> {
        let source: abi::Reg = source.into();
        let min = self.allocate_register();
        let max = self.allocate_register();
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
        self.raise_error_bare("ErrOverflow")?;
        self.emit(abi::label(&ok));
        Ok(())
    }

    pub(crate) fn emit_float_bits_to_fixed_value(
        &mut self,
        source: impl Into<abi::Reg>,
        result: impl Into<abi::Reg>,
    ) -> Result<(), String> {
        let bits_reg = self.temporary_vreg();
        let exponent_reg = self.temporary_vreg();
        let mask_reg = self.temporary_vreg();
        let sign_reg = self.temporary_vreg();
        let mantissa_reg = self.temporary_vreg();
        let const_reg = self.temporary_vreg();
        let bits = &bits_reg;
        let exponent = &exponent_reg;
        let mask = &mask_reg;
        let sign = &sign_reg;
        let mantissa = &mantissa_reg;
        let const_bits = &const_reg;
        let invalid = self.label("float_to_fixed_invalid");
        let overflow = self.label("float_to_fixed_overflow");
        let ok = self.label("float_to_fixed_ok");
        let edge = self.label("float_to_fixed_edge");
        let edge_negative = self.label("float_to_fixed_edge_negative");
        let range_ok = self.label("float_to_fixed_range_ok");
        self.emit(abi::move_register(bits, source));
        self.emit_float_exponent_range_guard(
            bits,
            exponent,
            mask,
            Some(sign.into()),
            Some(mantissa.into()),
            Some("1054"),
            &range_ok,
            &edge,
            &edge_negative,
            &invalid,
            &overflow,
        );
        self.emit(abi::label(&range_ok));
        self.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[0], bits));
        self.emit_f64_const(abi::FP_SCRATCH[1], const_bits, 4_294_967_296.0);
        self.emit(abi::float_multiply_d(
            abi::FP_SCRATCH[0],
            abi::FP_SCRATCH[0],
            abi::FP_SCRATCH[1],
        ));
        // Round to nearest representable Fixed (ties away from zero) rather than
        // truncating toward zero, as `toFixed(Float)`/`toFixed(String)` require.
        self.emit(abi::float_round_to_signed_x(result, abi::FP_SCRATCH[0]));
        self.emit(abi::branch(&ok));
        self.emit(abi::label(&invalid));
        self.raise_error_bare("ErrInvalidFormat")?;
        self.emit(abi::label(&overflow));
        self.raise_error_bare("ErrOverflow")?;
        self.emit(abi::label(&ok));
        Ok(())
    }

    /// Parse a decimal String into `FP_SCRATCH[0]`, branching to `invalid_label`
    /// when the text is not a number.
    ///
    /// plan-120-F: this used to be ~300 inline instructions that accumulated
    /// digits in binary64 and applied the exponent by repeated multiply/divide
    /// by 10.0 — the classic double-rounding construction, wrong by up to 1 ULP
    /// even for values binary64 represents exactly. It now calls
    /// `_mfb_rt_string_to_float`, an Eisel–Lemire parser with an exact
    /// big-integer fallback (`codegen::string::format::float_parse`).
    ///
    /// The seam is unchanged on purpose: the helper still leaves its result in
    /// `FP_SCRATCH[0]` and still saturates to ±infinity rather than reporting
    /// overflow itself, so both call sites keep their existing
    /// `emit_double_overflow_check` and `toFloat`/`toFixed` raise exactly the
    /// codes they raised before.
    pub(crate) fn emit_parse_decimal_string_to_double(
        &mut self,
        source_register: impl Into<abi::Reg>,
        invalid_label: &str,
    ) -> Result<(), String> {
        self.emit(abi::move_register(abi::c_arg(0), source_register));
        self.emit_symbol_call(STRING_TO_FLOAT_SYMBOL);
        self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, "0"));
        self.emit(abi::branch_ne(invalid_label));
        self.emit(abi::float_move_d_from_x(
            abi::FP_SCRATCH[0],
            RESULT_VALUE_REGISTER,
        ));
        Ok(())
    }

    /// Shared IEEE-754 `double` exponent/range guard behind the float→Integer and
    /// float→Fixed conversions and the plain overflow pre-check. `bits` already
    /// holds the raw f64 bit pattern — the caller performs the GPR (`move_register`)
    /// or FP (`float_move_x_from_d`) move into it, since that is the one op that
    /// genuinely differs across the three sites. The caller also mints `exponent`,
    /// `mask`, `sign`, `mantissa` (in its own order) and every label, so this emits
    /// only the shared op sequence and stays byte-identical to the hand-written
    /// copies it replaced.
    ///
    /// `threshold = None` is the `emit_double_overflow_check` shape: emit just the
    /// NaN/Inf exponent test (all-ones exponent → `invalid`); `sign`, `mantissa`
    /// (`None`), and the three range/edge labels are unused. `threshold = Some(t)` adds the
    /// range check — `< t` → `range_ok`, `> t` → `overflow` — and, for the boundary
    /// exponent `== t`, the edge block that accepts only the exact minimum-magnitude
    /// value (sign set, zero mantissa) and traps everything else to `overflow`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn emit_float_exponent_range_guard(
        &mut self,
        bits: impl Into<abi::Reg>,
        exponent: impl Into<abi::Reg>,
        mask: impl Into<abi::Reg>,
        sign: Option<abi::Reg>,
        mantissa: Option<abi::Reg>,
        threshold: Option<&str>,
        range_ok: &str,
        edge: &str,
        edge_sign_ok: &str,
        invalid: &str,
        overflow: &str,
    ) {
        let bits: abi::Reg = bits.into();
        let exponent: abi::Reg = exponent.into();
        let mask: abi::Reg = mask.into();
        self.emit(abi::shift_right_immediate(exponent, bits, 52));
        self.emit(abi::move_immediate(mask, "Integer", "2047"));
        self.emit(abi::and_registers(exponent, exponent, mask));
        self.emit(abi::compare_immediate(exponent, "2047"));
        self.emit(abi::branch_eq(invalid));
        let Some(threshold) = threshold else {
            return;
        };
        self.emit(abi::compare_immediate(exponent, threshold));
        self.emit(abi::branch_lt(range_ok));
        self.emit(abi::branch_eq(edge));
        self.emit(abi::branch(overflow));
        self.emit(abi::label(edge));
        let sign = sign.expect("a range threshold needs the caller's sign register");
        let mantissa = mantissa.expect("a range threshold needs the caller's mantissa register");
        self.emit(abi::shift_right_immediate(sign, bits, 63));
        self.emit(abi::compare_immediate(sign, "1"));
        self.emit(abi::branch_eq(edge_sign_ok));
        self.emit(abi::branch(overflow));
        self.emit(abi::label(edge_sign_ok));
        self.emit(abi::move_immediate(mask, "Integer", F64_MANTISSA_MASK));
        self.emit(abi::and_registers(mantissa, bits, mask));
        self.emit(abi::compare_immediate(mantissa, "0"));
        self.emit(abi::branch_ne(overflow));
    }

    pub(crate) fn emit_double_overflow_check(
        &mut self,
        source: impl Into<abi::Reg>,
        overflow_label: &str,
    ) {
        let bits = self.temporary_vreg();
        let exponent = self.temporary_vreg();
        let mask = self.temporary_vreg();
        self.emit(abi::float_move_x_from_d(&bits, source));
        self.emit_float_exponent_range_guard(
            &bits,
            &exponent,
            &mask,
            None,
            None,
            None,
            "",
            "",
            "",
            overflow_label,
            "",
        );
    }
}
