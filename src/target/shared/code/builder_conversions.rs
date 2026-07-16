use super::*;

/// Upper bound on the decimal exponent magnitude accumulated while parsing a
/// numeric string. The representable range of an IEEE-754 double spans roughly
/// 10^-324 to 10^308, so any exponent magnitude at or beyond this clamp drives
/// every representable mantissa to overflow (infinity) or underflow (zero). The
/// value is well above that useful range yet far below 2^63, so accumulation can
/// never wrap a 64-bit register. It also fits the AArch64 12-bit `cmp` immediate.
const DECIMAL_EXPONENT_CLAMP: &str = "1000";

impl CodeBuilder<'_> {
    pub(super) fn lower_to_int(&mut self, args: &[NirValue]) -> Result<ValueResult, String> {
        let value = self.lower_value(&args[0])?;
        // A `d`-native float's bits are read by the conversion, so materialize it
        // into a GPR first (plan-01 float-dnative). Identity for other types.
        let value = self.materialize_float(value)?;
        // `toInt(value)` with a `Byte` or `Scalar` is a width-preserving move: a
        // Byte's value and a Scalar's zero-extended codepoint are already their
        // Integer value. The 2-arg radix form is `String`-only, so both are 1-arg.
        if matches!(value.type_.as_str(), "Byte" | "Scalar") {
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
        // The 2-arg `toInt(text AS String, base AS Integer)` form parses `text`
        // in `base` (plan-02-cleanup §5). Lower and spill the base before
        // resetting temporaries so its register can be reclaimed.
        let base_slot = if args.len() == 2 {
            let base = self.lower_value(&args[1])?;
            let slot = self.allocate_stack_object("to_int_base", 8);
            self.emit(abi::store_u64(&base.location, abi::stack_pointer(), slot));
            Some(slot)
        } else {
            None
        };
        self.reset_temporary_registers();
        let source = self.allocate_register()?;
        self.emit(abi::load_u64(&source, abi::stack_pointer(), value_slot));
        match value.type_.as_str() {
            "Fixed" => self.emit_fixed_to_int_value(&source),
            "Float" => self.emit_float_to_int_value(&source),
            "Money" => self.emit_money_to_int_value(&source),
            "String" => match base_slot {
                Some(slot) => self.emit_string_to_int_value_base(&source, slot),
                None => self.emit_string_to_int_value(&source),
            },
            other => Err(format!(
                "native toInt does not accept argument type '{other}'"
            )),
        }
    }

    pub(super) fn emit_fixed_to_int_value(
        &mut self,
        source_register: &str,
    ) -> Result<ValueResult, String> {
        let value_reg = self.temporary_vreg();
        let value = value_reg.as_str();
        let result = self.allocate_register()?;
        let nonnegative = self.label("fixed_to_int_nonnegative");
        let done = self.label("fixed_to_int_done");
        self.emit(abi::move_register(value, source_register));
        self.emit(abi::compare_immediate(value, "0"));
        self.emit(abi::branch_ge(&nonnegative));
        self.emit(abi::subtract_registers(&result, abi::ZERO, value));
        self.emit(abi::shift_right_immediate(&result, &result, 32));
        self.emit(abi::subtract_registers(&result, abi::ZERO, &result));
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

    /// `toInt(Money)` — the whole-unit count, `raw / 100000` truncated toward
    /// zero (plan-29-G §4.3). Always fits Integer.
    pub(super) fn emit_money_to_int_value(
        &mut self,
        source_register: &str,
    ) -> Result<ValueResult, String> {
        let scale = self.allocate_register()?;
        let result = self.allocate_register()?;
        self.emit(abi::move_immediate(&scale, "Integer", "100000"));
        self.emit(abi::signed_divide_registers(
            &result,
            source_register,
            &scale,
        ));
        Ok(ValueResult {
            type_: "Integer".to_string(),
            location: result,
            text: "toInt(Money)".to_string(),
        })
    }

    pub(super) fn emit_float_to_int_value(
        &mut self,
        source_register: &str,
    ) -> Result<ValueResult, String> {
        let bits_reg = self.temporary_vreg();
        let exponent_reg = self.temporary_vreg();
        let mantissa_reg = self.temporary_vreg();
        let sign_reg = self.temporary_vreg();
        let mask_reg = self.temporary_vreg();
        let bits = bits_reg.as_str();
        let exponent = exponent_reg.as_str();
        let mantissa = mantissa_reg.as_str();
        let sign = sign_reg.as_str();
        let mask = mask_reg.as_str();
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
        self.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[0], bits));
        self.emit(abi::float_convert_to_signed_x(&result, abi::FP_SCRATCH[0]));
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
        // Pure integer parse with no call ABI: every working register is scratch,
        // minted as a vreg so the allocator colors it per-ISA (was hand-pinned
        // x8-x17 + an out-of-pool x6). `xzr`
        // below stays — it is the architectural zero register, not scratch.
        let string_v = self.temporary_vreg();
        let length_v = self.temporary_vreg();
        let index_v = self.temporary_vreg();
        let cursor_v = self.temporary_vreg();
        let byte_v = self.temporary_vreg();
        let acc_v = self.temporary_vreg();
        let negative_v = self.temporary_vreg();
        let digit_v = self.temporary_vreg();
        let cutoff_v = self.temporary_vreg();
        let cutlim_v = self.temporary_vreg();
        let ten_v = self.temporary_vreg();
        let string = string_v.as_str();
        let length = length_v.as_str();
        let index = index_v.as_str();
        let cursor = cursor_v.as_str();
        let byte = byte_v.as_str();
        let acc = acc_v.as_str();
        let negative = negative_v.as_str();
        let digit = digit_v.as_str();
        let cutoff = cutoff_v.as_str();
        let cutlim = cutlim_v.as_str();
        let ten = ten_v.as_str();
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
        // UNSIGNED overflow guard (bug-144): parsing i64::MIN's magnitude drives
        // `acc` to exactly 2^63, which as an i64 register is negative. A signed
        // `>` would then see 2^63 as less than the positive `cutoff` and admit a
        // further digit, wrapping silently. `cutoff`/`cutlim` bound the unsigned
        // magnitude, so the compares must be unsigned (`branch_hi`); equality is
        // sign-agnostic, and positive inputs stay below 2^63 where unsigned and
        // signed order agree. Mirrors the base-N path (bug-49).
        self.emit(abi::compare_registers(acc, cutoff));
        self.emit(abi::branch_hi(&overflow));
        self.emit(abi::branch_eq(&cutoff_equal));
        self.emit(abi::branch(&digit_ok));
        self.emit(abi::label(&cutoff_equal));
        self.emit(abi::compare_registers(digit, cutlim));
        self.emit(abi::branch_hi(&overflow));
        self.emit(abi::label(&digit_ok));
        self.emit(abi::multiply_registers(acc, acc, ten));
        self.emit(abi::add_registers(acc, acc, digit));
        self.emit(abi::add_immediate(index, index, 1));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::branch(&loop_start));

        self.emit(abi::label(&loop_done));
        self.emit(abi::compare_immediate(negative, "0"));
        self.emit(abi::branch_eq(&positive));
        self.emit(abi::subtract_registers(&result, abi::ZERO, acc));
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

    /// Radix-aware string parse for the 2-arg `toInt(text AS String, base AS
    /// Integer)` form (plan-02-cleanup §5). `base_slot` holds the runtime base
    /// (a stack offset). Generalizes `emit_string_to_int_value`'s base-10 digit
    /// accumulation to an arbitrary `base` in `2..=36` with a base-aware digit
    /// validator and runtime overflow cutoff. The optional leading `-`/`+` sign
    /// is kept for every base (backward-compatible with the base-10 path).
    ///
    /// Errors: `base` outside `2..=36`, an empty string, or a digit not valid
    /// for `base` FAIL `77050003` (ErrInvalidFormat); a value outside the i64
    /// range FAILs `77050010` (ErrOverflow).
    pub(super) fn emit_string_to_int_value_base(
        &mut self,
        source_register: &str,
        base_slot: usize,
    ) -> Result<ValueResult, String> {
        // All working registers are scratch (no call ABI); mint as vregs so the
        // allocator colors them per-ISA (was x8-x17 + out-of-pool x6/x7). `xzr`
        // below stays. AArch64 unaffected.
        let string_v = self.temporary_vreg();
        let length_v = self.temporary_vreg();
        let index_v = self.temporary_vreg();
        let cursor_v = self.temporary_vreg();
        let byte_v = self.temporary_vreg();
        let acc_v = self.temporary_vreg();
        let negative_v = self.temporary_vreg();
        let digit_v = self.temporary_vreg();
        let cutoff_v = self.temporary_vreg();
        let cutlim_v = self.temporary_vreg();
        let base_v = self.temporary_vreg();
        let scratch_v = self.temporary_vreg();
        let string = string_v.as_str();
        let length = length_v.as_str();
        let index = index_v.as_str();
        let cursor = cursor_v.as_str();
        let byte = byte_v.as_str();
        let acc = acc_v.as_str();
        let negative = negative_v.as_str();
        let digit = digit_v.as_str();
        let cutoff = cutoff_v.as_str();
        let cutlim = cutlim_v.as_str();
        let base = base_v.as_str();
        let scratch = scratch_v.as_str();
        let invalid = self.label("string_to_int_base_invalid");
        let overflow = self.label("string_to_int_base_overflow");
        let first_not_minus = self.label("string_to_int_base_first_not_minus");
        let sign_done = self.label("string_to_int_base_sign_done");
        let limit_ready = self.label("string_to_int_base_limit_ready");
        let loop_start = self.label("string_to_int_base_loop");
        let loop_done = self.label("string_to_int_base_done");
        let alpha = self.label("string_to_int_base_alpha");
        let digit_decoded = self.label("string_to_int_base_digit_decoded");
        let cutoff_equal = self.label("string_to_int_base_cutoff_equal");
        let digit_ok = self.label("string_to_int_base_digit_ok");
        let positive = self.label("string_to_int_base_positive");
        let done = self.label("string_to_int_base_return");
        let result = self.allocate_register()?;

        // Load the base from its stack slot and validate `2 <= base <= 36`.
        self.emit(abi::load_u64(base, abi::stack_pointer(), base_slot));
        self.emit(abi::move_register(string, source_register));
        self.emit(abi::compare_immediate(base, "2"));
        self.emit(abi::branch_lt(&invalid));
        self.emit(abi::compare_immediate(base, "36"));
        self.emit(abi::branch_gt(&invalid));

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

        // Overflow cutoff: limit = negative ? 2^63 : i64::MAX. With base >= 2,
        // cutoff = limit / base and cutlim = limit - cutoff*base are computed
        // against an UNSIGNED limit; the per-digit check below therefore uses
        // UNSIGNED compares (see bug-49).
        self.emit(abi::move_immediate(
            scratch,
            "Integer",
            "9223372036854775807",
        ));
        self.emit(abi::compare_immediate(negative, "0"));
        self.emit(abi::branch_eq(&limit_ready));
        self.emit(abi::add_immediate(scratch, scratch, 1));
        self.emit(abi::label(&limit_ready));
        self.emit(abi::unsigned_divide_registers(cutoff, scratch, base));
        self.emit(abi::multiply_subtract_registers(
            cutlim, cutoff, base, scratch,
        ));

        self.emit(abi::label(&loop_start));
        self.emit(abi::compare_registers(index, length));
        self.emit(abi::branch_ge(&loop_done));
        self.emit(abi::load_u8(byte, cursor, 0));
        // Decode one base-36 digit into `digit`, rejecting non-alphanumerics.
        // Decimal: '0'..'9' (byte-48 in 0..9). Alpha: 'A'..'Z' / 'a'..'z' map to
        // 10..35 via (byte-65)+10 / (byte-97)+10.
        self.emit(abi::subtract_immediate(digit, byte, 48));
        self.emit(abi::compare_immediate(digit, "10"));
        self.emit(abi::branch_lo(&digit_decoded));
        self.emit(abi::subtract_immediate(scratch, byte, 65));
        self.emit(abi::compare_immediate(scratch, "26"));
        self.emit(abi::branch_lo(&alpha));
        self.emit(abi::subtract_immediate(scratch, byte, 97));
        self.emit(abi::compare_immediate(scratch, "26"));
        self.emit(abi::branch_lo(&alpha));
        self.emit(abi::branch(&invalid));
        self.emit(abi::label(&alpha));
        self.emit(abi::add_immediate(digit, scratch, 10));
        self.emit(abi::label(&digit_decoded));
        // Reject a digit that is not valid for `base` (e.g. '9' in base 2).
        self.emit(abi::compare_registers(digit, base));
        self.emit(abi::branch_ge(&invalid));
        // acc = acc*base + digit, with the standard cutoff overflow guard.
        // `cutoff`/`cutlim` are derived from an UNSIGNED `limit` (2^63 for the
        // negative case), so the comparisons must be UNSIGNED too: for a
        // power-of-two base the accumulator can reach exactly 2^63, which as an
        // i64 register is negative and would fool a signed compare into skipping
        // the trap (bug-49). `branch_hi` is unsigned `>`; equality is
        // sign-agnostic. For positive inputs acc < 2^63, where unsigned and
        // signed order agree, so this is regression-free.
        self.emit(abi::compare_registers(acc, cutoff));
        self.emit(abi::branch_hi(&overflow));
        self.emit(abi::branch_eq(&cutoff_equal));
        self.emit(abi::branch(&digit_ok));
        self.emit(abi::label(&cutoff_equal));
        self.emit(abi::compare_registers(digit, cutlim));
        self.emit(abi::branch_hi(&overflow));
        self.emit(abi::label(&digit_ok));
        self.emit(abi::multiply_registers(acc, acc, base));
        self.emit(abi::add_registers(acc, acc, digit));
        self.emit(abi::add_immediate(index, index, 1));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::branch(&loop_start));

        self.emit(abi::label(&loop_done));
        self.emit(abi::compare_immediate(negative, "0"));
        self.emit(abi::branch_eq(&positive));
        self.emit(abi::subtract_registers(&result, abi::ZERO, acc));
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
            text: "toInt(String, base)".to_string(),
        })
    }

    pub(super) fn lower_to_byte(&mut self, arg: &NirValue) -> Result<ValueResult, String> {
        let value = self.lower_value(arg)?;
        if !matches!(value.type_.as_str(), "Integer" | "Money" | "Scalar") {
            return Err(format!(
                "native toByte does not accept argument type '{}'",
                value.type_
            ));
        }
        // `toByte(Money)` narrows the whole-unit part (`raw / 100000`), then
        // range-checks it exactly like an Integer (plan-29-G §4.3).
        let checked = if value.type_ == "Money" {
            let scale = self.allocate_register()?;
            let whole = self.allocate_register()?;
            self.emit(abi::move_immediate(&scale, "Integer", "100000"));
            self.emit(abi::signed_divide_registers(
                &whole,
                &value.location,
                &scale,
            ));
            whole
        } else {
            value.location.clone()
        };
        let result = self.allocate_register()?;
        let overflow = self.label("to_byte_overflow");
        let ok = self.label("to_byte_ok");
        self.emit(abi::compare_immediate(&checked, "0"));
        self.emit(abi::branch_lt(&overflow));
        self.emit(abi::compare_immediate(&checked, "255"));
        self.emit(abi::branch_hi(&overflow));
        self.emit(abi::move_register(&result, &checked));
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

    /// `toScalar(Integer|String|Byte) -> Scalar` (plan-41-D). `Byte` is an
    /// infallible widen (every byte is a valid scalar); `Integer` fails
    /// `ErrInvalidArgument` for a surrogate (U+D800..U+DFFF) or a value outside
    /// 0..U+10FFFF; `String` decodes the single scalar of a one-scalar string,
    /// failing for an empty or multi-scalar string.
    pub(super) fn lower_to_scalar(&mut self, arg: &NirValue) -> Result<ValueResult, String> {
        let value = self.lower_value(arg)?;
        match value.type_.as_str() {
            "Byte" => {
                let register = self.allocate_register()?;
                self.emit(abi::move_register(&register, &value.location));
                Ok(ValueResult {
                    type_: "Scalar".to_string(),
                    location: register,
                    text: format!("toScalar({})", value.text),
                })
            }
            "Integer" => {
                let cp = value.location.clone();
                let ok = self.label("to_scalar_ok");
                let invalid = self.label("to_scalar_invalid");
                let not_surrogate = self.label("to_scalar_not_surrogate");
                // cp < 0 -> invalid.
                self.emit(abi::compare_immediate(&cp, "0"));
                self.emit(abi::branch_lt(&invalid));
                // cp > 0x10FFFF (1114111) -> invalid.
                self.emit(abi::compare_immediate(&cp, "1114111"));
                self.emit(abi::branch_hi(&invalid));
                // Surrogate band 0xD800..0xDFFF (55296..57343) -> invalid.
                self.emit(abi::compare_immediate(&cp, "55296"));
                self.emit(abi::branch_lo(&not_surrogate));
                self.emit(abi::compare_immediate(&cp, "57343"));
                self.emit(abi::branch_le(&invalid));
                self.emit(abi::label(&not_surrogate));
                self.emit(abi::branch(&ok));
                self.emit(abi::label(&invalid));
                self.emit_invalid_argument_return()?;
                self.emit(abi::label(&ok));
                let register = self.allocate_register()?;
                self.emit(abi::move_register(&register, &cp));
                Ok(ValueResult {
                    type_: "Scalar".to_string(),
                    location: register,
                    text: format!("toScalar({})", value.text),
                })
            }
            "String" => {
                let result = self.emit_string_to_scalar_value(&value.location)?;
                Ok(ValueResult {
                    type_: "Scalar".to_string(),
                    location: result,
                    text: format!("toScalar({})", value.text),
                })
            }
            other => Err(format!(
                "native toScalar does not accept argument type '{other}'"
            )),
        }
    }

    /// Decode the single Unicode scalar of a one-scalar `String` into a codepoint
    /// register. A `String` is guaranteed valid UTF-8, so the decoder trusts
    /// well-formedness and only enforces "exactly one scalar": it computes the
    /// lead byte's expected length, reassembles the codepoint, and traps
    /// `ErrInvalidArgument` when the string is empty or its byte length differs
    /// from that scalar's length (i.e. zero or more than one scalar).
    pub(super) fn emit_string_to_scalar_value(
        &mut self,
        source_register: &str,
    ) -> Result<String, String> {
        let string_v = self.temporary_vreg();
        let length_v = self.temporary_vreg();
        let b0_v = self.temporary_vreg();
        let cp_v = self.temporary_vreg();
        let cont_v = self.temporary_vreg();
        let mask_v = self.temporary_vreg();
        let nbytes_v = self.temporary_vreg();
        let string = string_v.as_str();
        let length = length_v.as_str();
        let b0 = b0_v.as_str();
        let cp = cp_v.as_str();
        let cont = cont_v.as_str();
        let mask = mask_v.as_str();
        let nbytes = nbytes_v.as_str();

        let one_byte = self.label("str_scalar_one");
        let two_byte = self.label("str_scalar_two");
        let three_byte = self.label("str_scalar_three");
        let four_byte = self.label("str_scalar_four");
        let assembled = self.label("str_scalar_assembled");
        let invalid = self.label("str_scalar_invalid");
        let ok = self.label("str_scalar_ok");

        self.emit(abi::move_register(string, source_register));
        self.emit(abi::load_u64(length, string, 0));
        // Empty string -> invalid.
        self.emit(abi::compare_immediate(length, "0"));
        self.emit(abi::branch_eq(&invalid));
        self.emit(abi::load_u8(b0, string, 8));
        // Classify the lead byte by its high bits.
        self.emit(abi::compare_immediate(b0, "128")); // < 0x80 -> 1 byte
        self.emit(abi::branch_lo(&one_byte));
        self.emit(abi::compare_immediate(b0, "192")); // 0x80..0xBF lead -> invalid
        self.emit(abi::branch_lo(&invalid));
        self.emit(abi::compare_immediate(b0, "224")); // < 0xE0 -> 2 bytes
        self.emit(abi::branch_lo(&two_byte));
        self.emit(abi::compare_immediate(b0, "240")); // < 0xF0 -> 3 bytes
        self.emit(abi::branch_lo(&three_byte));
        self.emit(abi::compare_immediate(b0, "248")); // < 0xF8 -> 4 bytes
        self.emit(abi::branch_lo(&four_byte));
        self.emit(abi::branch(&invalid));

        // 1 byte: cp = b0.
        self.emit(abi::label(&one_byte));
        self.emit(abi::move_immediate(nbytes, "Integer", "1"));
        self.emit(abi::move_register(cp, b0));
        self.emit(abi::branch(&assembled));

        // 2 bytes: cp = (b0 & 0x1F) << 6 | (b1 & 0x3F).
        self.emit(abi::label(&two_byte));
        self.emit(abi::move_immediate(nbytes, "Integer", "2"));
        self.emit(abi::move_immediate(mask, "Integer", "31"));
        self.emit(abi::and_registers(cp, b0, mask));
        self.emit(abi::shift_left_immediate(cp, cp, 6));
        self.emit(abi::load_u8(cont, string, 9));
        self.emit(abi::move_immediate(mask, "Integer", "63"));
        self.emit(abi::and_registers(cont, cont, mask));
        self.emit(abi::or_registers(cp, cp, cont));
        self.emit(abi::branch(&assembled));

        // 3 bytes: cp = (b0 & 0x0F) << 12 | (b1 & 0x3F) << 6 | (b2 & 0x3F).
        self.emit(abi::label(&three_byte));
        self.emit(abi::move_immediate(nbytes, "Integer", "3"));
        self.emit(abi::move_immediate(mask, "Integer", "15"));
        self.emit(abi::and_registers(cp, b0, mask));
        self.emit(abi::shift_left_immediate(cp, cp, 12));
        self.emit(abi::load_u8(cont, string, 9));
        self.emit(abi::move_immediate(mask, "Integer", "63"));
        self.emit(abi::and_registers(cont, cont, mask));
        self.emit(abi::shift_left_immediate(cont, cont, 6));
        self.emit(abi::or_registers(cp, cp, cont));
        self.emit(abi::load_u8(cont, string, 10));
        self.emit(abi::and_registers(cont, cont, mask));
        self.emit(abi::or_registers(cp, cp, cont));
        self.emit(abi::branch(&assembled));

        // 4 bytes: cp = (b0 & 0x07)<<18 | (b1&0x3F)<<12 | (b2&0x3F)<<6 | (b3&0x3F).
        self.emit(abi::label(&four_byte));
        self.emit(abi::move_immediate(nbytes, "Integer", "4"));
        self.emit(abi::move_immediate(mask, "Integer", "7"));
        self.emit(abi::and_registers(cp, b0, mask));
        self.emit(abi::shift_left_immediate(cp, cp, 18));
        self.emit(abi::load_u8(cont, string, 9));
        self.emit(abi::move_immediate(mask, "Integer", "63"));
        self.emit(abi::and_registers(cont, cont, mask));
        self.emit(abi::shift_left_immediate(cont, cont, 12));
        self.emit(abi::or_registers(cp, cp, cont));
        self.emit(abi::load_u8(cont, string, 10));
        self.emit(abi::and_registers(cont, cont, mask));
        self.emit(abi::shift_left_immediate(cont, cont, 6));
        self.emit(abi::or_registers(cp, cp, cont));
        self.emit(abi::load_u8(cont, string, 11));
        self.emit(abi::and_registers(cont, cont, mask));
        self.emit(abi::or_registers(cp, cp, cont));
        self.emit(abi::branch(&assembled));

        // Exactly-one-scalar check: the byte length must equal the lead byte's
        // expected length; anything else is zero or more than one scalar.
        self.emit(abi::label(&assembled));
        self.emit(abi::compare_registers(length, nbytes));
        self.emit(abi::branch_ne(&invalid));
        self.emit(abi::branch(&ok));
        self.emit(abi::label(&invalid));
        self.emit_invalid_argument_return()?;
        self.emit(abi::label(&ok));
        let result = self.allocate_register()?;
        self.emit(abi::move_register(&result, cp));
        Ok(result)
    }

    /// `toString(Scalar) -> String` (plan-41-D): UTF-8-encode the one codepoint
    /// into a fresh 1–4 byte `String`. Infallible — every valid `Scalar` is a
    /// valid UTF-8 string. Writes the encoded bytes into a stack buffer, then
    /// materializes an owned arena `String` from them.
    pub(super) fn emit_scalar_to_string_value(
        &mut self,
        source_register: &str,
    ) -> Result<ValueResult, String> {
        let cp_v = self.temporary_vreg();
        let buf_v = self.temporary_vreg();
        let len_v = self.temporary_vreg();
        let tmp_v = self.temporary_vreg();
        let mask_v = self.temporary_vreg();
        let cp = cp_v.as_str();
        let buf = buf_v.as_str();
        let len = len_v.as_str();
        let tmp = tmp_v.as_str();
        let mask = mask_v.as_str();
        let buf_slot = self.allocate_stack_object("scalar_utf8_buf", 8);

        let enc1 = self.label("scalar_str_enc1");
        let enc2 = self.label("scalar_str_enc2");
        let enc3 = self.label("scalar_str_enc3");
        let enc4 = self.label("scalar_str_enc4");
        let encoded = self.label("scalar_str_encoded");

        self.emit(abi::move_register(cp, source_register));
        self.emit(abi::add_immediate(buf, abi::stack_pointer(), buf_slot));
        self.emit(abi::compare_immediate(cp, "128")); // < 0x80 -> 1 byte
        self.emit(abi::branch_lo(&enc1));
        self.emit(abi::compare_immediate(cp, "2048")); // < 0x800 -> 2 bytes
        self.emit(abi::branch_lo(&enc2));
        self.emit(abi::compare_immediate(cp, "65536")); // < 0x10000 -> 3 bytes
        self.emit(abi::branch_lo(&enc3));
        self.emit(abi::branch(&enc4));

        // 1 byte: [cp].
        self.emit(abi::label(&enc1));
        self.emit(abi::store_u8(cp, buf, 0));
        self.emit(abi::move_immediate(len, "Integer", "1"));
        self.emit(abi::branch(&encoded));

        // 2 bytes: [0xC0 | cp>>6, 0x80 | cp&0x3F].
        self.emit(abi::label(&enc2));
        self.emit(abi::shift_right_immediate(tmp, cp, 6));
        self.emit(abi::move_immediate(mask, "Integer", "192"));
        self.emit(abi::or_registers(tmp, tmp, mask));
        self.emit(abi::store_u8(tmp, buf, 0));
        self.emit(abi::move_immediate(mask, "Integer", "63"));
        self.emit(abi::and_registers(tmp, cp, mask));
        self.emit(abi::move_immediate(mask, "Integer", "128"));
        self.emit(abi::or_registers(tmp, tmp, mask));
        self.emit(abi::store_u8(tmp, buf, 1));
        self.emit(abi::move_immediate(len, "Integer", "2"));
        self.emit(abi::branch(&encoded));

        // 3 bytes: [0xE0 | cp>>12, 0x80 | (cp>>6)&0x3F, 0x80 | cp&0x3F].
        self.emit(abi::label(&enc3));
        self.emit(abi::shift_right_immediate(tmp, cp, 12));
        self.emit(abi::move_immediate(mask, "Integer", "224"));
        self.emit(abi::or_registers(tmp, tmp, mask));
        self.emit(abi::store_u8(tmp, buf, 0));
        self.emit(abi::shift_right_immediate(tmp, cp, 6));
        self.emit(abi::move_immediate(mask, "Integer", "63"));
        self.emit(abi::and_registers(tmp, tmp, mask));
        self.emit(abi::move_immediate(mask, "Integer", "128"));
        self.emit(abi::or_registers(tmp, tmp, mask));
        self.emit(abi::store_u8(tmp, buf, 1));
        self.emit(abi::move_immediate(mask, "Integer", "63"));
        self.emit(abi::and_registers(tmp, cp, mask));
        self.emit(abi::move_immediate(mask, "Integer", "128"));
        self.emit(abi::or_registers(tmp, tmp, mask));
        self.emit(abi::store_u8(tmp, buf, 2));
        self.emit(abi::move_immediate(len, "Integer", "3"));
        self.emit(abi::branch(&encoded));

        // 4 bytes: [0xF0 | cp>>18, 0x80 | (cp>>12)&0x3F, 0x80 | (cp>>6)&0x3F,
        //           0x80 | cp&0x3F].
        self.emit(abi::label(&enc4));
        self.emit(abi::shift_right_immediate(tmp, cp, 18));
        self.emit(abi::move_immediate(mask, "Integer", "240"));
        self.emit(abi::or_registers(tmp, tmp, mask));
        self.emit(abi::store_u8(tmp, buf, 0));
        self.emit(abi::shift_right_immediate(tmp, cp, 12));
        self.emit(abi::move_immediate(mask, "Integer", "63"));
        self.emit(abi::and_registers(tmp, tmp, mask));
        self.emit(abi::move_immediate(mask, "Integer", "128"));
        self.emit(abi::or_registers(tmp, tmp, mask));
        self.emit(abi::store_u8(tmp, buf, 1));
        self.emit(abi::shift_right_immediate(tmp, cp, 6));
        self.emit(abi::move_immediate(mask, "Integer", "63"));
        self.emit(abi::and_registers(tmp, tmp, mask));
        self.emit(abi::move_immediate(mask, "Integer", "128"));
        self.emit(abi::or_registers(tmp, tmp, mask));
        self.emit(abi::store_u8(tmp, buf, 2));
        self.emit(abi::move_immediate(mask, "Integer", "63"));
        self.emit(abi::and_registers(tmp, cp, mask));
        self.emit(abi::move_immediate(mask, "Integer", "128"));
        self.emit(abi::or_registers(tmp, tmp, mask));
        self.emit(abi::store_u8(tmp, buf, 3));
        self.emit(abi::move_immediate(len, "Integer", "4"));
        self.emit(abi::branch(&encoded));

        self.emit(abi::label(&encoded));
        // Re-derive the buffer address after the branches (the arena call inside
        // materialize spills it) and build the owned String.
        let buf_addr = self.allocate_register()?;
        self.emit(abi::add_immediate(
            &buf_addr,
            abi::stack_pointer(),
            buf_slot,
        ));
        let result = self.emit_materialize_string_from_bytes(&buf_addr, len)?;
        Ok(ValueResult {
            type_: "String".to_string(),
            location: result,
            text: "toString(Scalar)".to_string(),
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
                self.emit(abi::signed_convert_to_float_d(abi::FP_SCRATCH[0], &source));
                self.emit(abi::float_move_x_from_d(&result, abi::FP_SCRATCH[0]));
            }
            "Fixed" => {
                let temp = ValueResult {
                    type_: "Fixed".to_string(),
                    location: source,
                    text: value.text.clone(),
                };
                self.load_numeric_as_double(abi::FP_SCRATCH[0], &temp)?;
                self.emit(abi::float_move_x_from_d(&result, abi::FP_SCRATCH[0]));
            }
            // `toFloat(Money)` = `raw / 100000.0` (plan-29-G §4.3).
            "Money" => {
                let temp = ValueResult {
                    type_: "Money".to_string(),
                    location: source,
                    text: value.text.clone(),
                };
                self.load_numeric_as_double(abi::FP_SCRATCH[0], &temp)?;
                self.emit(abi::float_move_x_from_d(&result, abi::FP_SCRATCH[0]));
            }
            "String" => {
                let invalid = self.label("to_float_invalid");
                let overflow = self.label("to_float_overflow");
                self.emit_parse_decimal_string_to_double(&source, &invalid)?;
                self.emit_double_overflow_check(abi::FP_SCRATCH[0], &overflow);
                self.emit(abi::float_move_x_from_d(&result, abi::FP_SCRATCH[0]));
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
            // `toFixed(Money)` = `raw * 2^32 / 100000` — exactly `emit_fixed_divide`
            // fed the Money raw and the base-10 scale; its range check traps a
            // Money too large for Fixed's 32-bit integer part (plan-29-G §4.3).
            "Money" => {
                let scale = self.allocate_register()?;
                self.emit(abi::move_immediate(&scale, "Integer", "100000"));
                self.emit_fixed_divide(&result, &source, &scale)?;
            }
            "String" => {
                let invalid = self.label("to_fixed_invalid");
                let overflow = self.label("to_fixed_overflow");
                self.emit_parse_decimal_string_to_double(&source, &invalid)?;
                self.emit_double_overflow_check(abi::FP_SCRATCH[0], &overflow);
                let parsed_bits_reg = self.temporary_vreg();
                let parsed_bits = parsed_bits_reg.as_str();
                self.emit(abi::float_move_x_from_d(parsed_bits, abi::FP_SCRATCH[0]));
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

    /// `toMoney(value)` — the explicit crossing *into* Money from every type
    /// (plan-29-G §4.2). Integer/Byte scale by 100000; Fixed rescales exactly via
    /// the 128-bit `emit_fixed_multiply`; Float and String go through f64 and the
    /// mode-aware round, guarding finiteness and range.
    pub(super) fn lower_to_money(&mut self, arg: &NirValue) -> Result<ValueResult, String> {
        let value = self.lower_value(arg)?;
        // Read a `d`-native float's bits from a GPR for the Float conversion.
        let value = self.materialize_float(value)?;
        let value_slot = self.allocate_stack_object("to_money_value", 8);
        self.emit(abi::store_u64(
            &value.location,
            abi::stack_pointer(),
            value_slot,
        ));
        self.reset_temporary_registers();
        let source = self.allocate_register()?;
        self.emit(abi::load_u64(&source, abi::stack_pointer(), value_slot));
        let result = self.allocate_register()?;
        let scratch = self.temporary_vreg();
        match value.type_.as_str() {
            // Exact: `value * 100000`, overflow-checked for Integer (a Byte is
            // always in range: 255 * 100000 fits i64).
            "Integer" => {
                let scale = self.allocate_register()?;
                self.emit(abi::move_immediate(&scale, "Integer", "100000"));
                self.emit_checked_integer_multiply(&result, &source, &scale)?;
            }
            "Byte" => {
                let scale = self.allocate_register()?;
                self.emit(abi::move_immediate(&scale, "Integer", "100000"));
                self.emit(abi::multiply_registers(&result, &source, &scale));
            }
            // `fixed_raw * 100000 / 2^32` is exactly `emit_fixed_multiply(fixed_raw,
            // 100000)`; its overflow check traps a Fixed too large for Money.
            "Fixed" => {
                let scale = self.allocate_register()?;
                self.emit(abi::move_immediate(&scale, "Integer", "100000"));
                self.emit_fixed_multiply(&result, &source, &scale)?;
            }
            // Inexact: finiteness → ErrInvalidFormat, `value * 100000.0` rounded
            // under the mode, range → ErrOverflow.
            "Float" => {
                let fval = self.allocate_fp_register()?;
                self.emit(abi::float_move_d_from_x(&fval, &source));
                self.emit_float_finite_or_invalid(&fval)?;
                let scale = self.allocate_fp_register()?;
                self.emit_f64_const(&scale, scratch.as_str(), 100_000.0);
                let scaled = self.allocate_fp_register()?;
                self.emit(abi::float_multiply_d(&scaled, &fval, &scale));
                self.emit_round_double_to_money_raw(&scaled, &result)?;
            }
            // Mirror `toFixed(String)`: parse to f64, then scale + mode-round.
            "String" => {
                let invalid = self.label("to_money_invalid");
                let done = self.label("to_money_done");
                self.emit_parse_decimal_string_to_double(&source, &invalid)?;
                let parsed = self.allocate_fp_register()?;
                self.emit(abi::float_move_d_from_d(&parsed, abi::FP_SCRATCH[0]));
                let scale = self.allocate_fp_register()?;
                self.emit_f64_const(&scale, scratch.as_str(), 100_000.0);
                let scaled = self.allocate_fp_register()?;
                self.emit(abi::float_multiply_d(&scaled, &parsed, &scale));
                self.emit_round_double_to_money_raw(&scaled, &result)?;
                self.emit(abi::branch(&done));
                self.emit(abi::label(&invalid));
                self.emit_invalid_format_return()?;
                self.emit(abi::label(&done));
            }
            other => {
                return Err(format!(
                    "native toMoney does not accept argument type '{other}'"
                ))
            }
        }
        Ok(ValueResult {
            type_: "Money".to_string(),
            location: result,
            text: format!("toMoney({})", value.text),
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
        self.emit_double_overflow_check(abi::FP_SCRATCH[0], &invalid);
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
        // The predicate reads the operand's bits, so materialize a `d`-native
        // float into a GPR first (plan-01 float-dnative).
        let value = self.materialize_float(value)?;
        let result = self.allocate_register()?;
        let true_label = self.label(name);
        let done_label = self.label(&format!("{name}_done"));

        match value.type_.as_str() {
            "Integer" | "Fixed" => self.emit(abi::compare_immediate(&value.location, "0")),
            "Float" => {
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
        let bits_reg = self.temporary_vreg();
        let exponent_reg = self.temporary_vreg();
        let mask_reg = self.temporary_vreg();
        let sign_reg = self.temporary_vreg();
        let mantissa_reg = self.temporary_vreg();
        let const_reg = self.temporary_vreg();
        let bits = bits_reg.as_str();
        let exponent = exponent_reg.as_str();
        let mask = mask_reg.as_str();
        let sign = sign_reg.as_str();
        let mantissa = mantissa_reg.as_str();
        let const_bits = const_reg.as_str();
        let invalid = self.label("float_to_fixed_invalid");
        let overflow = self.label("float_to_fixed_overflow");
        let ok = self.label("float_to_fixed_ok");
        let edge = self.label("float_to_fixed_edge");
        let edge_negative = self.label("float_to_fixed_edge_negative");
        let range_ok = self.label("float_to_fixed_range_ok");
        self.emit(abi::move_register(bits, source));
        self.emit(abi::shift_right_immediate(exponent, bits, 52));
        self.emit(abi::move_immediate(mask, "Integer", "2047"));
        self.emit(abi::and_registers(exponent, exponent, mask));
        self.emit(abi::compare_immediate(exponent, "2047"));
        self.emit(abi::branch_eq(&invalid));
        self.emit(abi::compare_immediate(exponent, "1054"));
        self.emit(abi::branch_lt(&range_ok));
        self.emit(abi::branch_eq(&edge));
        self.emit(abi::branch(&overflow));
        self.emit(abi::label(&edge));
        self.emit(abi::shift_right_immediate(sign, bits, 63));
        self.emit(abi::compare_immediate(sign, "1"));
        self.emit(abi::branch_eq(&edge_negative));
        self.emit(abi::branch(&overflow));
        self.emit(abi::label(&edge_negative));
        self.emit(abi::move_immediate(mask, "Integer", "4503599627370495"));
        self.emit(abi::and_registers(mantissa, bits, mask));
        self.emit(abi::compare_immediate(mantissa, "0"));
        self.emit(abi::branch_ne(&overflow));
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
        let string_reg = self.temporary_vreg();
        let length_reg = self.temporary_vreg();
        let index_reg = self.temporary_vreg();
        let cursor_reg = self.temporary_vreg();
        let byte_reg = self.temporary_vreg();
        let digit_reg = self.temporary_vreg();
        let negative_reg = self.temporary_vreg();
        let seen_digit_reg = self.temporary_vreg();
        let ten_bits_reg = self.temporary_vreg();
        let dot_seen_reg = self.temporary_vreg();
        let zero_src_reg = self.temporary_vreg();
        let one_bits_reg = self.temporary_vreg();
        let exponent_reg = self.temporary_vreg();
        let exponent_negative_reg = self.temporary_vreg();
        let exponent_ten_reg = self.temporary_vreg();
        let string = string_reg.as_str();
        let length = length_reg.as_str();
        let index = index_reg.as_str();
        let cursor = cursor_reg.as_str();
        let byte = byte_reg.as_str();
        let digit = digit_reg.as_str();
        let negative = negative_reg.as_str();
        let seen_digit = seen_digit_reg.as_str();
        let ten_bits = ten_bits_reg.as_str();
        let dot_seen = dot_seen_reg.as_str();
        let zero_src = zero_src_reg.as_str();
        let one_bits = one_bits_reg.as_str();
        let exponent = exponent_reg.as_str();
        let exponent_negative = exponent_negative_reg.as_str();
        let exponent_ten = exponent_ten_reg.as_str();
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
        self.emit(abi::move_immediate(zero_src, "Integer", "0"));
        self.emit(abi::signed_convert_to_float_d(abi::FP_SCRATCH[0], zero_src));
        self.emit_f64_const(abi::FP_SCRATCH[1], ten_bits, 10.0);
        self.emit_f64_const(abi::FP_SCRATCH[3], one_bits, 1.0);
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
        self.emit(abi::signed_convert_to_float_d(abi::FP_SCRATCH[2], digit));
        self.emit(abi::move_immediate(seen_digit, "Integer", "1"));
        self.emit(abi::compare_immediate(dot_seen, "0"));
        self.emit(abi::branch_ne(&frac_digit));
        self.emit(abi::label(&int_digit));
        self.emit(abi::float_multiply_d(
            abi::FP_SCRATCH[0],
            abi::FP_SCRATCH[0],
            abi::FP_SCRATCH[1],
        ));
        self.emit(abi::float_add_d(
            abi::FP_SCRATCH[0],
            abi::FP_SCRATCH[0],
            abi::FP_SCRATCH[2],
        ));
        self.emit(abi::branch(&next));
        self.emit(abi::label(&frac_digit));
        self.emit(abi::float_multiply_d(
            abi::FP_SCRATCH[3],
            abi::FP_SCRATCH[3],
            abi::FP_SCRATCH[1],
        ));
        self.emit(abi::float_divide_d(
            abi::FP_SCRATCH[2],
            abi::FP_SCRATCH[2],
            abi::FP_SCRATCH[3],
        ));
        self.emit(abi::float_add_d(
            abi::FP_SCRATCH[0],
            abi::FP_SCRATCH[0],
            abi::FP_SCRATCH[2],
        ));
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
        self.emit(abi::float_multiply_d(
            abi::FP_SCRATCH[0],
            abi::FP_SCRATCH[0],
            abi::FP_SCRATCH[1],
        ));
        self.emit(abi::subtract_immediate(exponent, exponent, 1));
        self.emit(abi::branch(&exponent_multiply_loop));
        self.emit(abi::label(&exponent_divide_loop));
        self.emit(abi::compare_immediate(exponent, "0"));
        self.emit(abi::branch_eq(&exponent_apply_done));
        self.emit(abi::float_divide_d(
            abi::FP_SCRATCH[0],
            abi::FP_SCRATCH[0],
            abi::FP_SCRATCH[1],
        ));
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
        self.emit(abi::float_negate_d(abi::FP_SCRATCH[0], abi::FP_SCRATCH[0]));
        self.emit(abi::label(&positive));
        Ok(())
    }

    pub(super) fn emit_double_overflow_check(&mut self, source: &str, overflow_label: &str) {
        let bits = self.temporary_vreg();
        let exponent = self.temporary_vreg();
        let mask = self.temporary_vreg();
        self.emit(abi::float_move_x_from_d(&bits, source));
        self.emit(abi::shift_right_immediate(&exponent, &bits, 52));
        self.emit(abi::move_immediate(&mask, "Integer", "2047"));
        self.emit(abi::and_registers(&exponent, &exponent, &mask));
        self.emit(abi::compare_immediate(&exponent, "2047"));
        self.emit(abi::branch_eq(overflow_label));
    }
}
