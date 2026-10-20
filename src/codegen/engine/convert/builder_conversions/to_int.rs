//! plan-176-U: the `toInt` conversion and its fixed/money/float/string helpers. The other conversions live in
//! `engine/convert/builder_conversions/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    pub(crate) fn lower_to_int(&mut self, args: &[NirValue]) -> Result<ValueResult, String> {
        let value = self.lower_value(&args[0])?;
        // A `Float` argument is an observation boundary (spec `04_types.md`:
        // "passed as an argument" includes every built-in's argument), so a
        // non-finite arithmetic result raises ErrFloatOverflow/ErrFloatNaN here,
        // as it does for `toString`, rather than reaching the conversion (bug-807).
        self.observe_float(&args[0], &value)?;
        // A `d`-native float's bits are read by the conversion, so materialize it
        // into a GPR first (plan-01 float-dnative). Identity for other types.
        let value = self.materialize_float(value)?;
        // `toInt(value)` with a `Byte` or `Scalar` is a width-preserving move: a
        // Byte's value and a Scalar's zero-extended codepoint are already their
        // Integer value. So is an enum (plan-140-B): its value IS the member's
        // 0-based declaration index. The 2-arg radix form is `String`-only, so all
        // three are 1-arg.
        if matches!(value.type_.name().as_ref(), "Byte" | "Scalar")
            || self.type_model.is_enum_type(&value.type_)
        {
            let register = self.allocate_register();
            self.emit(abi::move_register(&register, &value.location));
            return Ok(ValueResult {
                origin: None,
                type_: ParameterType::Integer,
                location: Operand::from(register),
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
        let source = self.allocate_register();
        self.emit(abi::load_u64(&source, abi::stack_pointer(), value_slot));
        match &value.type_ {
            ParameterType::Fixed => self.emit_fixed_to_int_value(&source),
            ParameterType::Float => self.emit_float_to_int_value(&source),
            ParameterType::Money => self.emit_money_to_int_value(&source),
            ParameterType::String => match base_slot {
                Some(slot) => self.emit_string_to_int_value_base(&source, slot),
                None => self.emit_string_to_int_value(&source),
            },
            other => Err(format!(
                "native toInt does not accept argument type '{other}'"
            )),
        }
    }

    pub(crate) fn emit_fixed_to_int_value(
        &mut self,
        source_register: impl Into<abi::Reg>,
    ) -> Result<ValueResult, String> {
        let value_reg = self.temporary_vreg();
        let value = &value_reg;
        let result = self.allocate_register();
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
            origin: None,
            type_: ParameterType::Integer,
            location: Operand::from(result),
            text: "toInt(Fixed)".to_string(),
        })
    }

    /// `toInt(Money)` — the whole-unit count, `raw / 100000` truncated toward
    /// zero (plan-29-G §4.3). Always fits Integer.
    pub(crate) fn emit_money_to_int_value(
        &mut self,
        source_register: impl Into<abi::Reg>,
    ) -> Result<ValueResult, String> {
        let scale = self.allocate_register();
        let result = self.allocate_register();
        self.emit(abi::move_immediate(&scale, "Integer", "100000"));
        self.emit(abi::signed_divide_registers(
            &result,
            source_register,
            &scale,
        ));
        Ok(ValueResult {
            origin: None,
            type_: ParameterType::Integer,
            location: Operand::from(result),
            text: "toInt(Money)".to_string(),
        })
    }

    pub(crate) fn emit_float_to_int_value(
        &mut self,
        source_register: impl Into<abi::Reg>,
    ) -> Result<ValueResult, String> {
        let bits_reg = self.temporary_vreg();
        let exponent_reg = self.temporary_vreg();
        let mantissa_reg = self.temporary_vreg();
        let sign_reg = self.temporary_vreg();
        let mask_reg = self.temporary_vreg();
        let bits = &bits_reg;
        let exponent = &exponent_reg;
        let mantissa = &mantissa_reg;
        let sign = &sign_reg;
        let mask = &mask_reg;
        let ok = self.label("float_to_int_ok");
        let check_edge = self.label("float_to_int_check_edge");
        let edge_sign_ok = self.label("float_to_int_edge_sign_ok");
        let overflow = self.label("float_to_int_overflow");
        let invalid = self.label("float_to_int_invalid");
        let result = self.allocate_register();

        self.emit(abi::move_register(bits, source_register));
        self.emit_float_exponent_range_guard(
            bits,
            exponent,
            mask,
            Some(sign.into()),
            Some(mantissa.into()),
            Some("1086"),
            &ok,
            &check_edge,
            &edge_sign_ok,
            &invalid,
            &overflow,
        );

        self.emit(abi::label(&ok));
        self.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[0], bits));
        self.emit(abi::float_convert_to_signed_x(&result, abi::FP_SCRATCH[0]));
        let done = self.label("float_to_int_done");
        self.emit(abi::branch(&done));
        self.emit(abi::label(&invalid));
        self.raise_error_bare("ErrInvalidFormat")?;
        self.emit(abi::label(&overflow));
        self.raise_error_bare("ErrOverflow")?;
        self.emit(abi::label(&done));

        Ok(ValueResult {
            origin: None,
            type_: ParameterType::Integer,
            location: Operand::from(result),
            text: "toInt(Float)".to_string(),
        })
    }

    /// Sign/length prologue shared by the base-10 and radix string→Integer parses:
    /// load the length, reject empty, point `cursor` at the first byte, zero
    /// `index`/`acc`/`negative`, consume an optional leading `+`/`-`, and reject a
    /// sign with no digits. The caller mints every register (and moves `source`
    /// into `string`, which sits at a different point in the two parsers) and every
    /// label, so this emits only the shared op run and stays byte-identical.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn emit_string_to_int_sign_prologue(
        &mut self,
        string: impl Into<abi::Reg>,
        length: impl Into<abi::Reg>,
        index: impl Into<abi::Reg>,
        cursor: impl Into<abi::Reg>,
        byte: impl Into<abi::Reg>,
        acc: impl Into<abi::Reg>,
        negative: impl Into<abi::Reg>,
        invalid: &str,
        first_not_minus: &str,
        sign_done: &str,
    ) {
        let string: abi::Reg = string.into();
        let length: abi::Reg = length.into();
        let index: abi::Reg = index.into();
        let cursor: abi::Reg = cursor.into();
        let byte: abi::Reg = byte.into();
        let negative: abi::Reg = negative.into();
        self.emit(abi::load_u64(length, string, 0));
        self.emit(abi::compare_immediate(length, "0"));
        self.emit(abi::branch_eq(invalid));
        self.emit(abi::add_immediate(cursor, string, 8));
        self.emit(abi::move_immediate(index, "Integer", "0"));
        self.emit(abi::move_immediate(acc, "Integer", "0"));
        self.emit(abi::move_immediate(negative, "Integer", "0"));
        self.emit(abi::load_u8(byte, cursor, 0));
        self.emit(abi::compare_immediate(byte, "45"));
        self.emit(abi::branch_ne(first_not_minus));
        self.emit(abi::move_immediate(negative, "Integer", "1"));
        self.emit(abi::add_immediate(index, index, 1));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::branch(sign_done));
        self.emit(abi::label(first_not_minus));
        self.emit(abi::compare_immediate(byte, "43"));
        self.emit(abi::branch_ne(sign_done));
        self.emit(abi::add_immediate(index, index, 1));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::label(sign_done));
        self.emit(abi::compare_registers(index, length));
        self.emit(abi::branch_ge(invalid));
    }

    /// The UNSIGNED cutoff/cutlim overflow guard shared by both integer parses,
    /// emitted once so its hazard note lives in a single place. `cutoff`/`cutlim`
    /// bound the unsigned magnitude, so the compares are unsigned (`branch_hi`):
    /// parsing i64::MIN's magnitude drives `acc` to exactly 2^63 — negative as a
    /// signed i64 — which a signed compare would wrongly admit, wrapping silently
    /// (bug-49 / bug-144). Equality is sign-agnostic; positive inputs stay below
    /// 2^63 where unsigned and signed order agree.
    pub(crate) fn emit_int_parse_cutoff_guard(
        &mut self,
        acc: impl Into<abi::Reg>,
        cutoff: impl Into<abi::Reg>,
        digit: impl Into<abi::Reg>,
        cutlim: impl Into<abi::Reg>,
        overflow: &str,
        cutoff_equal: &str,
        digit_ok: &str,
    ) {
        self.emit(abi::compare_registers(acc, cutoff));
        self.emit(abi::branch_hi(overflow));
        self.emit(abi::branch_eq(cutoff_equal));
        self.emit(abi::branch(digit_ok));
        self.emit(abi::label(cutoff_equal));
        self.emit(abi::compare_registers(digit, cutlim));
        self.emit(abi::branch_hi(overflow));
        self.emit(abi::label(digit_ok));
    }

    /// Sign-application epilogue shared by both integer parses: negate `acc` into
    /// `result` when the sign was `-`, else copy it, then the trap tails. The
    /// caller mints `result` and every label, so this is byte-identical to the two
    /// hand-written copies.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn emit_int_parse_sign_epilogue(
        &mut self,
        result: impl Into<abi::Reg>,
        acc: impl Into<abi::Reg>,
        negative: impl Into<abi::Reg>,
        loop_done: &str,
        positive: &str,
        done: &str,
        invalid: &str,
        overflow: &str,
    ) -> Result<(), String> {
        let result: abi::Reg = result.into();
        let acc: abi::Reg = acc.into();
        self.emit(abi::label(loop_done));
        self.emit(abi::compare_immediate(negative, "0"));
        self.emit(abi::branch_eq(positive));
        self.emit(abi::subtract_registers(result, abi::ZERO, acc));
        self.emit(abi::branch(done));
        self.emit(abi::label(positive));
        self.emit(abi::move_register(result, acc));
        self.emit(abi::branch(done));
        self.emit(abi::label(invalid));
        self.raise_error_bare("ErrInvalidFormat")?;
        self.emit(abi::label(overflow));
        self.raise_error_bare("ErrOverflow")?;
        self.emit(abi::label(done));
        Ok(())
    }

    pub(crate) fn emit_string_to_int_value(
        &mut self,
        source_register: impl Into<abi::Reg>,
    ) -> Result<ValueResult, String> {
        // Pure integer parse with no call ABI: every working register is scratch,
        // minted as a vreg so the allocator colors it per-ISA. `xzr` below stays
        // — it is the architectural zero register, not scratch.
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
        let string = &string_v;
        let length = &length_v;
        let index = &index_v;
        let cursor = &cursor_v;
        let byte = &byte_v;
        let acc = &acc_v;
        let negative = &negative_v;
        let digit = &digit_v;
        let cutoff = &cutoff_v;
        let cutlim = &cutlim_v;
        let ten = &ten_v;
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
        let result = self.allocate_register();

        self.emit(abi::move_register(string, source_register));
        self.emit_string_to_int_sign_prologue(
            string,
            length,
            index,
            cursor,
            byte,
            acc,
            negative,
            &invalid,
            &first_not_minus,
            &sign_done,
        );
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
        self.emit_int_parse_cutoff_guard(
            acc,
            cutoff,
            digit,
            cutlim,
            &overflow,
            &cutoff_equal,
            &digit_ok,
        );
        self.emit(abi::multiply_registers(acc, acc, ten));
        self.emit(abi::add_registers(acc, acc, digit));
        self.emit(abi::add_immediate(index, index, 1));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::branch(&loop_start));

        self.emit_int_parse_sign_epilogue(
            &result, acc, negative, &loop_done, &positive, &done, &invalid, &overflow,
        )?;

        Ok(ValueResult {
            origin: None,
            type_: ParameterType::Integer,
            location: Operand::from(result),
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
    pub(crate) fn emit_string_to_int_value_base(
        &mut self,
        source_register: impl Into<abi::Reg>,
        base_slot: usize,
    ) -> Result<ValueResult, String> {
        // All working registers are scratch (no call ABI); mint as vregs so the
        // allocator colors them per-ISA. `xzr` below stays — it is the
        // architectural zero register, not scratch.
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
        let string = &string_v;
        let length = &length_v;
        let index = &index_v;
        let cursor = &cursor_v;
        let byte = &byte_v;
        let acc = &acc_v;
        let negative = &negative_v;
        let digit = &digit_v;
        let cutoff = &cutoff_v;
        let cutlim = &cutlim_v;
        let base = &base_v;
        let scratch = &scratch_v;
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
        let result = self.allocate_register();

        // Load the base from its stack slot and validate `2 <= base <= 36`.
        self.emit(abi::load_u64(base, abi::stack_pointer(), base_slot));
        self.emit(abi::move_register(string, source_register));
        self.emit(abi::compare_immediate(base, "2"));
        self.emit(abi::branch_lt(&invalid));
        self.emit(abi::compare_immediate(base, "36"));
        self.emit(abi::branch_gt(&invalid));

        self.emit_string_to_int_sign_prologue(
            string,
            length,
            index,
            cursor,
            byte,
            acc,
            negative,
            &invalid,
            &first_not_minus,
            &sign_done,
        );

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
        // acc = acc*base + digit, with the shared cutoff overflow guard.
        self.emit_int_parse_cutoff_guard(
            acc,
            cutoff,
            digit,
            cutlim,
            &overflow,
            &cutoff_equal,
            &digit_ok,
        );
        self.emit(abi::multiply_registers(acc, acc, base));
        self.emit(abi::add_registers(acc, acc, digit));
        self.emit(abi::add_immediate(index, index, 1));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::branch(&loop_start));

        self.emit_int_parse_sign_epilogue(
            &result, acc, negative, &loop_done, &positive, &done, &invalid, &overflow,
        )?;

        Ok(ValueResult {
            origin: None,
            type_: ParameterType::Integer,
            location: Operand::from(result),
            text: "toInt(String, base)".to_string(),
        })
    }
}
