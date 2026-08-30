//! Shared Money rounding helper (plan-29-D §4.3).
//!
//! `emit_apply_rounding` centralizes the half-away / half-even choice that every
//! Money *arithmetic* rounding site consults (`money::round`, `M / k`,
//! `M * Fixed`, `M / Fixed`, and the `toMoney`/`toFixed` conversions). Given a
//! truncated-toward-zero magnitude quotient/remainder and the divisor magnitude,
//! it reads the per-arena rounding mode and emits the correct half-adjustment,
//! then applies the result sign. The half-away / half-even *policy* is stated
//! here once; `emit_round_double_to_money_raw` (the float→Money conversion path)
//! reaches the same tie-break in the FP domain and so carries its own copy of the
//! parity test — the two cannot share a single emitter because one works on
//! integer magnitudes and the other on an `fcmp` against 0.5 (bug-332 C3).
//!
//! `toString(Money)` deliberately does **not** call this helper: its presentation
//! rounding is a fixed half-away-from-zero rule, independent of the mode
//! (plan-29-G §4.1).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::types::ParameterType;
impl CodeBuilder<'_> {
    /// Round a truncated signed division toward the mode's half rule and write the
    /// signed result into `dst`.
    ///
    /// - `quotient` — the signed quotient truncated toward zero.
    /// - `remainder` — the signed remainder (`dividend - quotient*divisor`).
    /// - `abs_divisor` — `|divisor|` (strictly positive; the caller guards zero).
    /// - `sign_neg` — nonzero when the exact quotient is negative, `0` otherwise
    ///   (needed because a truncated `quotient` of `0` carries no sign).
    ///
    /// Commercial (mode 0) rounds away from zero on `2*|rem| >= |div|`; Banker
    /// (mode 1) does the same except the exact tie (`2*|rem| == |div|`) rounds to
    /// even (increment only when the truncated quotient is odd). Doubling is
    /// avoided (`|rem|` vs `|div| - |rem|`) so nothing overflows near i64::MAX.
    pub(crate) fn emit_apply_rounding(
        &mut self,
        dst: impl Into<Operand>,
        quotient: impl Into<Operand>,
        remainder: impl Into<Operand>,
        abs_divisor: impl Into<Operand>,
        sign_neg: impl Into<Operand>,
    ) -> Result<(), String> {
        let dst = dst.into();
        let quotient = quotient.into();
        let round_up = self.label("money_round_up");
        let round_down = self.label("money_round_down");
        let keep = self.label("money_round_keep");

        // abs_rem = |remainder|
        let abs_rem = self.allocate_register();
        self.emit(abi::move_register(&abs_rem, remainder));
        self.emit_abs_i64(&abs_rem)?;
        // half = |div| - |rem|  (in [1, |div|]); tie when |rem| == half.
        let half = self.allocate_register();
        self.emit(abi::subtract_registers(&half, abs_divisor, &abs_rem));

        // Default: keep the truncated quotient.
        self.emit(abi::move_register(dst.clone(), quotient.clone()));
        self.emit(abi::compare_registers(&abs_rem, &half));
        self.emit(abi::branch_lt(&keep)); // |rem| < half  -> below the half, keep
        self.emit(abi::branch_gt(&round_up)); // |rem| > half  -> past the half, round away

        // Exact tie (|rem| == half): Commercial rounds away, Banker rounds to even.
        let mode = self.allocate_register();
        self.emit(abi::load_u64(
            &mode,
            ARENA_STATE_REGISTER,
            ARENA_ROUNDING_MODE_OFFSET,
        ));
        self.emit(abi::compare_immediate(&mode, "0"));
        self.emit(abi::branch_eq(&round_up)); // Commercial -> away
                                              // Banker: round only when the truncated quotient is odd (to reach even).
        let one = self.allocate_register();
        self.emit(abi::move_immediate(&one, "Integer", "1"));
        let parity = self.allocate_register();
        self.emit(abi::and_registers(&parity, quotient.clone(), &one));
        self.emit(abi::compare_immediate(&parity, "0"));
        self.emit(abi::branch_eq(&keep)); // even quotient -> keep, already even

        // Round the magnitude away from zero: +1 when positive, -1 when negative.
        self.emit(abi::label(&round_up));
        self.emit(abi::compare_immediate(sign_neg, "0"));
        self.emit(abi::branch_ne(&round_down));
        self.emit(abi::add_immediate(dst.clone(), quotient.clone(), 1));
        self.emit(abi::branch(&keep));
        self.emit(abi::label(&round_down));
        self.emit(abi::subtract_immediate(dst, quotient, 1));
        self.emit(abi::label(&keep));
        Ok(())
    }

    /// The central Money `*`/`/`/`MOD`/`DIV` dispatcher (plan-29-E/F). `+`/`-` and
    /// comparison reach `emit_integer_binary` / the compare path directly; this
    /// covers scaling by a scalar, the `M/M` ratio, `M MOD M`, and every `DIV`.
    /// Returns the result location (a GPR for a Money result, an FP register for a
    /// Float result). The front end (plan-29-A) has already rejected every
    /// dimensionally-invalid pairing, so only valid operand shapes arrive here.
    pub(crate) fn emit_money_binary(
        &mut self,
        op: &str,
        left: &ValueResult,
        right: &ValueResult,
        dst: impl Into<Operand>,
    ) -> Result<String, String> {
        let l_money = left.type_ == ParameterType::Money;
        let r_money = right.type_ == ParameterType::Money;
        let dst = dst.into();
        match op {
            // `M ± M` and `M MOD M` are exact integer ops on the raw i64.
            "+" | "-" | "MOD" => {
                self.emit_integer_binary(op, left, right, dst.clone(), false)?;
                Ok(dst.render())
            }
            "*" => {
                // Commutative: identify the Money operand and the scalar factor.
                let (money, scalar) = if l_money {
                    (left, right)
                } else {
                    (right, left)
                };
                self.emit_money_multiply(money, scalar, dst)
            }
            "/" if l_money && r_money => self.emit_money_ratio(left, right),
            "/" => self.emit_money_divide_scalar(left, right, dst),
            // `DIV` is the explicit Float escape: promote both operands to f64.
            "DIV" => self.emit_money_div_to_float(left, right),
            other => Err(format!(
                "native code plan cannot lower Money operator '{other}'"
            )),
        }
    }

    /// `Money * scalar → Money` (plan-29-E §4.1 / plan-29-F §4.1/4.3).
    fn emit_money_multiply(
        &mut self,
        money: &ValueResult,
        scalar: &ValueResult,
        dst: impl Into<Operand>,
    ) -> Result<String, String> {
        let dst = dst.into();
        match &scalar.type_ {
            // Exact integer scaling: `raw * k`, overflow-checked.
            ParameterType::Integer | ParameterType::Byte => {
                self.emit_checked_integer_multiply(dst.clone(), &money.location, &scalar.location)?;
                Ok(dst.render())
            }
            // Exact binary fixed-point scaling: `raw * fixed_raw / 2^32` is exactly
            // what `emit_fixed_multiply` computes when fed the Money raw as the
            // left operand and the Q32.32 raw as the right (plan-29-F §4.1).
            ParameterType::Fixed => {
                self.emit_fixed_multiply(dst.clone(), &money.location, &scalar.location)?;
                Ok(dst.render())
            }
            // Inexact floating scaling (plan-29-F §4.3).
            ParameterType::Float => {
                self.emit_money_scale_float(&money.location, scalar, dst, false)
            }
            other => Err(format!(
                "native code plan cannot scale Money by operand type '{other}'"
            )),
        }
    }

    /// `Money / scalar → Money` (plan-29-E §4.2 / plan-29-F §4.2/4.3). Only the
    /// `Money /` direction reaches here; `scalar / Money` was rejected up front.
    fn emit_money_divide_scalar(
        &mut self,
        money: &ValueResult,
        scalar: &ValueResult,
        dst: impl Into<Operand>,
    ) -> Result<String, String> {
        let dst = dst.into();
        match &scalar.type_ {
            // `raw / k`, mode-rounded (plan-29-E §4.2). `k == 0` → ErrInvalidArgument.
            ParameterType::Integer | ParameterType::Byte => {
                self.emit_nonzero_or_invalid(&scalar.location)?;
                self.emit_integer_division_overflow_check(&money.location, &scalar.location)?;
                let quotient = self.allocate_register();
                self.emit(abi::signed_divide_registers(
                    &quotient,
                    &money.location,
                    &scalar.location,
                ));
                let remainder = self.allocate_register();
                // remainder = raw - quotient * k
                self.emit(abi::multiply_subtract_registers(
                    &remainder,
                    &quotient,
                    &scalar.location,
                    &money.location,
                ));
                let abs_div = self.allocate_register();
                self.emit(abi::move_register(&abs_div, &scalar.location));
                self.emit_abs_i64(&abs_div)?;
                // sign_neg = -1 (nonzero) when the signs of raw and k differ.
                let sign_neg = self.allocate_register();
                self.emit(abi::exclusive_or_registers(
                    &sign_neg,
                    &money.location,
                    &scalar.location,
                ));
                self.emit(abi::arithmetic_shift_right_immediate(
                    &sign_neg, &sign_neg, 63,
                ));
                // Guard k == i64::MIN: `emit_abs_i64` leaves it negative (its
                // magnitude is unrepresentable), which would make the signed
                // half-compare in `emit_apply_rounding` take the wrong branch
                // (bug-230). Because |raw| < 2^63 = |i64::MIN|, the remainder
                // magnitude is always below the half, so the result is exactly the
                // truncated quotient — skip rounding entirely for this divisor.
                let min_divisor = self.allocate_register();
                // i64::MIN as its unsigned bit pattern (2^63); `move_immediate`
                // takes the u64 pattern, not the signed "-9223372036854775808".
                self.emit(abi::move_immediate(&min_divisor, "Integer", F64_SIGN_BIT));
                let not_min = self.label("money_div_scalar_not_min");
                let div_done = self.label("money_div_scalar_done");
                self.emit(abi::compare_registers(&scalar.location, &min_divisor));
                self.emit(abi::branch_ne(&not_min));
                self.emit(abi::move_register(dst.clone(), &quotient));
                self.emit(abi::branch(&div_done));
                self.emit(abi::label(&not_min));
                self.emit_apply_rounding(dst.clone(), &quotient, &remainder, &abs_div, &sign_neg)?;
                self.emit(abi::label(&div_done));
                Ok(dst.render())
            }
            // `raw * 2^32 / fixed_raw` is exactly `emit_fixed_divide(raw, fixed_raw)`
            // (plan-29-F §4.2); it guards `fixed_raw == 0` → ErrInvalidArgument.
            ParameterType::Fixed => {
                self.emit_fixed_divide(dst.clone(), &money.location, &scalar.location)?;
                Ok(dst.render())
            }
            ParameterType::Float => self.emit_money_scale_float(&money.location, scalar, dst, true),
            other => Err(format!(
                "native code plan cannot divide Money by operand type '{other}'"
            )),
        }
    }

    /// `Money / Money → Float` (plan-29-E §4.3): the value ratio `raw_a / raw_b`
    /// (the SCALE cancels). Divide-by-zero follows Float rules (±Inf/NaN caught at
    /// the observation boundary), so no pre-check.
    fn emit_money_ratio(
        &mut self,
        left: &ValueResult,
        right: &ValueResult,
    ) -> Result<String, String> {
        let da = self.allocate_fp_register();
        let db = self.allocate_fp_register();
        self.emit(abi::signed_convert_to_float_d(&da, &left.location));
        self.emit(abi::signed_convert_to_float_d(&db, &right.location));
        let result = self.allocate_fp_register();
        self.emit(abi::float_divide_d(&result, &da, &db));
        Ok(result.render())
    }

    /// `Money DIV scalar|Money → Float` (plan-29-E §4.5 / plan-29-F §4.4): forced
    /// Float division, both operands promoted to their true f64 value.
    fn emit_money_div_to_float(
        &mut self,
        left: &ValueResult,
        right: &ValueResult,
    ) -> Result<String, String> {
        let da = self.allocate_fp_register();
        let db = self.allocate_fp_register();
        self.load_numeric_as_double(&da, left)?;
        self.load_numeric_as_double(&db, right)?;
        let result = self.allocate_fp_register();
        self.emit(abi::float_divide_d(&result, &da, &db));
        Ok(result.render())
    }

    /// `Money * Float` / `Money / Float → Money` (plan-29-F §4.3). Because the
    /// result raw equals `raw * fval` (resp. `raw / fval`) — the SCALE rides
    /// through — the whole computation is done in f64 and rounded back to the raw
    /// i64 under the current mode. A non-finite Float operand → ErrInvalidFormat;
    /// a zero divisor → ErrInvalidArgument; an out-of-range result → ErrOverflow.
    fn emit_money_scale_float(
        &mut self,
        money_raw: impl Into<Operand>,
        scalar: &ValueResult,
        dst: impl Into<Operand>,
        divide: bool,
    ) -> Result<String, String> {
        let dst = dst.into();
        let fval = self.allocate_fp_register();
        self.load_numeric_as_double(&fval, scalar)?;
        self.emit_float_finite_or_invalid(&fval)?;
        let money_d = self.allocate_fp_register();
        self.emit(abi::signed_convert_to_float_d(&money_d, money_raw));
        let result = self.allocate_fp_register();
        if divide {
            // A Money result, so a zero divisor is ErrInvalidArgument (not a Float
            // boundary) — plan-29-F Open Decisions.
            let ok = self.label("money_float_div_ok");
            self.emit(abi::float_compare_zero_d(&fval));
            self.emit(abi::branch_ne(&ok));
            self.raise_error_bare("ErrInvalidArgument")?;
            self.emit(abi::label(&ok));
            self.emit(abi::float_divide_d(&result, &money_d, &fval));
        } else {
            self.emit(abi::float_multiply_d(&result, &money_d, &fval));
        }
        self.emit_round_double_to_money_raw(&result, dst.clone())?;
        Ok(dst.render())
    }

    /// Fail with ErrInvalidFormat when the f64 in `value` is NaN or ±Inf (its
    /// biased exponent is all ones), mirroring `toFixed(Float)`'s guard.
    pub(crate) fn emit_float_finite_or_invalid(
        &mut self,
        value: impl Into<Operand>,
    ) -> Result<(), String> {
        let ok = self.label("money_finite_ok");
        let invalid = self.label("money_finite_invalid");
        let bits = self.allocate_register();
        let exponent = self.allocate_register();
        let mask = self.allocate_register();
        self.emit(abi::float_move_x_from_d(&bits, value));
        self.emit_float_exponent_classify(&exponent, &mask, &bits);
        self.emit(abi::branch_ne(&ok));
        self.emit(abi::label(&invalid));
        self.raise_error_bare("ErrInvalidFormat")?;
        self.emit(abi::label(&ok));
        Ok(())
    }

    /// Round the f64 result raw in `value` to the Money raw i64 in `dst` under the
    /// current mode: Commercial rounds ties away from zero, Banker rounds ties to
    /// even. A non-finite or out-of-range magnitude (`|value| >= 2^63`) fails with
    /// ErrOverflow.
    pub(crate) fn emit_round_double_to_money_raw(
        &mut self,
        value: impl Into<Operand>,
        dst: impl Into<Operand>,
    ) -> Result<(), String> {
        let value = value.into();
        let dst = dst.into();
        let overflow = self.label("money_round_overflow");
        let range_ok = self.label("money_round_range_ok");
        let round_away = self.label("money_round_f_away");
        let round_pos = self.label("money_round_f_pos");
        let done = self.label("money_round_f_done");
        let scratch = self.temporary_vreg();

        // Range guard: |value| >= 2^63 (or non-finite) overflows the raw i64.
        let magnitude = self.allocate_fp_register();
        self.emit(abi::float_abs_d(&magnitude, value.clone()));
        let limit = self.allocate_fp_register();
        self.emit_f64_const(&limit, &scratch, 9_223_372_036_854_775_808.0);
        self.emit(abi::float_compare_d(&magnitude, &limit));
        self.emit(abi::branch_mi(&range_ok)); // |value| < 2^63 (ordered, less-than)
        self.emit(abi::label(&overflow));
        self.raise_error_bare("ErrOverflow")?;
        self.emit(abi::label(&range_ok));

        // q = trunc(value) toward zero.
        let quotient = self.allocate_register();
        self.emit(abi::float_convert_to_signed_x(&quotient, value.clone()));
        // frac = value - (q as f64), in (-1, 1); abs_frac = |frac|.
        let q_f = self.allocate_fp_register();
        self.emit(abi::signed_convert_to_float_d(&q_f, &quotient));
        let frac = self.allocate_fp_register();
        self.emit(abi::float_subtract_d(&frac, value.clone(), &q_f));
        let abs_frac = self.allocate_fp_register();
        self.emit(abi::float_abs_d(&abs_frac, &frac));
        let half = self.allocate_fp_register();
        self.emit_f64_const(&half, &scratch, 0.5);

        self.emit(abi::move_register(dst.clone(), &quotient)); // default: keep the truncation
        self.emit(abi::float_compare_d(&abs_frac, &half));
        self.emit(abi::branch_mi(&done)); // abs_frac < 0.5 → keep
        self.emit(abi::branch_gt(&round_away)); // abs_frac > 0.5 → round away
                                                // Exact half tie: Commercial rounds away; Banker rounds to even.
        let mode = self.allocate_register();
        self.emit(abi::load_u64(
            &mode,
            ARENA_STATE_REGISTER,
            ARENA_ROUNDING_MODE_OFFSET,
        ));
        self.emit(abi::compare_immediate(&mode, "0"));
        self.emit(abi::branch_eq(&round_away)); // Commercial → away
        let one = self.allocate_register();
        self.emit(abi::move_immediate(&one, "Integer", "1"));
        let parity = self.allocate_register();
        self.emit(abi::and_registers(&parity, &quotient, &one));
        self.emit(abi::compare_immediate(&parity, "0"));
        self.emit(abi::branch_eq(&done)); // even quotient → keep

        self.emit(abi::label(&round_away));
        // Round the magnitude away from zero: +1 when value >= 0, −1 when value < 0.
        self.emit(abi::float_compare_zero_d(value.clone()));
        self.emit(abi::branch_ge(&round_pos));
        self.emit(abi::subtract_immediate(dst.clone(), &quotient, 1));
        self.emit(abi::branch(&done));
        self.emit(abi::label(&round_pos));
        self.emit(abi::add_immediate(dst.clone(), &quotient, 1));
        self.emit(abi::label(&done));
        Ok(())
    }

    /// bug-449: parse a decimal `Money` string EXACTLY to its scaled raw i64 in
    /// `dst`, using integer arithmetic only — no f64 — so the exact base-10
    /// contract Money is built on holds on string input too. Mirrors
    /// [`crate::numeric::money_conversion_from_decimal`]: an optional sign, whole
    /// digits, an optional `.` and fractional digits; the first 5 fractional
    /// digits are kept and the 6th settles the value under the current rounding
    /// mode (Commercial away, Banker to even — same as arithmetic rounding), any
    /// nonzero digit past the 6th making a non-tie.
    ///
    /// Control leaves through one of: `dst` set and fall through (success),
    /// `invalid_label` (malformed text), `overflow_label` (out of the i64 raw
    /// range), or `scientific_label` (an `e`/`E` — the rare exponent form is left
    /// to the f64 caller path, since a scientific money string is inherently
    /// approximate). The whole part is bounded to 92233720368547 as it
    /// accumulates so `whole * 100000` cannot overflow before the final range
    /// check; the magnitude is compared unsigned so the exact min
    /// (-92233720368547.75808, raw 2^63) is accepted.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn emit_parse_decimal_string_to_money_raw(
        &mut self,
        source_register: impl Into<Operand>,
        dst: impl Into<Operand>,
        invalid_label: &str,
        overflow_label: &str,
        scientific_label: &str,
    ) -> Result<(), String> {
        let dst = dst.into();
        let string = self.temporary_vreg();
        let length = self.temporary_vreg();
        let cursor = self.temporary_vreg();
        let index = self.temporary_vreg();
        let byte = self.temporary_vreg();
        let digit = self.temporary_vreg();
        let whole = self.temporary_vreg();
        let frac5 = self.temporary_vreg();
        let frac_count = self.temporary_vreg();
        let d6 = self.temporary_vreg();
        let rest_nonzero = self.temporary_vreg();
        let negative = self.temporary_vreg();
        let seen_digit = self.temporary_vreg();
        let dot_seen = self.temporary_vreg();
        let ten = self.temporary_vreg();
        let scratch = self.temporary_vreg();

        let loop_start = self.label("money_parse_loop");
        let after_sign = self.label("money_parse_after_sign");
        let not_minus = self.label("money_parse_not_minus");
        let sign_done = self.label("money_parse_sign_done");
        let dot = self.label("money_parse_dot");
        let is_frac = self.label("money_parse_frac");
        let frac_gt5 = self.label("money_parse_frac_gt5");
        let frac_rest = self.label("money_parse_frac_rest");
        let advance = self.label("money_parse_advance");
        let finish = self.label("money_parse_finish");
        let do_inc = self.label("money_parse_do_inc");
        let no_inc = self.label("money_parse_no_inc");
        let neg_path = self.label("money_parse_neg");
        let set_done = self.label("money_parse_set_done");

        self.emit(abi::move_register(&string, source_register));
        self.emit(abi::load_u64(&length, &string, 0));
        self.emit(abi::compare_immediate(&length, "0"));
        self.emit(abi::branch_eq(invalid_label));
        self.emit(abi::add_immediate(&cursor, &string, 8));
        self.emit(abi::move_immediate(&index, "Integer", "0"));
        self.emit(abi::move_immediate(&negative, "Integer", "0"));
        self.emit(abi::move_immediate(&seen_digit, "Integer", "0"));
        self.emit(abi::move_immediate(&dot_seen, "Integer", "0"));
        self.emit(abi::move_immediate(&whole, "Integer", "0"));
        self.emit(abi::move_immediate(&frac5, "Integer", "0"));
        self.emit(abi::move_immediate(&frac_count, "Integer", "0"));
        self.emit(abi::move_immediate(&d6, "Integer", "0"));
        self.emit(abi::move_immediate(&rest_nonzero, "Integer", "0"));
        self.emit(abi::move_immediate(&ten, "Integer", "10"));

        // Optional leading sign.
        self.emit(abi::load_u8(&byte, &cursor, 0));
        self.emit(abi::compare_immediate(&byte, "45")); // '-'
        self.emit(abi::branch_ne(&not_minus));
        self.emit(abi::move_immediate(&negative, "Integer", "1"));
        self.emit(abi::branch(&after_sign));
        self.emit(abi::label(&not_minus));
        self.emit(abi::compare_immediate(&byte, "43")); // '+'
        self.emit(abi::branch_ne(&sign_done));
        self.emit(abi::label(&after_sign));
        self.emit(abi::add_immediate(&index, &index, 1));
        self.emit(abi::add_immediate(&cursor, &cursor, 1));
        self.emit(abi::label(&sign_done));

        self.emit(abi::label(&loop_start));
        self.emit(abi::compare_registers(&index, &length));
        self.emit(abi::branch_ge(&finish));
        self.emit(abi::load_u8(&byte, &cursor, 0));
        self.emit(abi::compare_immediate(&byte, "46")); // '.'
        self.emit(abi::branch_eq(&dot));
        self.emit(abi::compare_immediate(&byte, "101")); // 'e'
        self.emit(abi::branch_eq(scientific_label));
        self.emit(abi::compare_immediate(&byte, "69")); // 'E'
        self.emit(abi::branch_eq(scientific_label));
        self.emit(abi::compare_immediate(&byte, "48")); // '0'
        self.emit(abi::branch_lo(invalid_label));
        self.emit(abi::compare_immediate(&byte, "57")); // '9'
        self.emit(abi::branch_hi(invalid_label));
        self.emit(abi::move_immediate(&seen_digit, "Integer", "1"));
        self.emit(abi::subtract_immediate(&digit, &byte, 48));
        self.emit(abi::compare_immediate(&dot_seen, "0"));
        self.emit(abi::branch_ne(&is_frac));
        // Integer part: whole = whole * 10 + digit, bounded so `* 100000` is safe.
        self.emit(abi::multiply_registers(&whole, &whole, &ten));
        self.emit(abi::add_registers(&whole, &whole, &digit));
        self.emit(abi::move_immediate(&scratch, "Integer", "92233720368547"));
        self.emit(abi::compare_registers(&whole, &scratch));
        self.emit(abi::branch_hi(overflow_label));
        self.emit(abi::branch(&advance));
        // Fractional part.
        self.emit(abi::label(&is_frac));
        self.emit(abi::add_immediate(&frac_count, &frac_count, 1));
        self.emit(abi::compare_immediate(&frac_count, "5"));
        self.emit(abi::branch_hi(&frac_gt5));
        // The first 5 fractional digits form the kept scaled value.
        self.emit(abi::multiply_registers(&frac5, &frac5, &ten));
        self.emit(abi::add_registers(&frac5, &frac5, &digit));
        self.emit(abi::branch(&advance));
        self.emit(abi::label(&frac_gt5));
        self.emit(abi::compare_immediate(&frac_count, "6"));
        self.emit(abi::branch_ne(&frac_rest));
        self.emit(abi::move_register(&d6, &digit)); // the 6th digit drives rounding
        self.emit(abi::branch(&advance));
        self.emit(abi::label(&frac_rest));
        // Any nonzero digit past the 6th means the 6th place is not an exact tie.
        self.emit(abi::compare_immediate(&digit, "0"));
        self.emit(abi::branch_eq(&advance));
        self.emit(abi::move_immediate(&rest_nonzero, "Integer", "1"));
        self.emit(abi::label(&advance));
        self.emit(abi::add_immediate(&index, &index, 1));
        self.emit(abi::add_immediate(&cursor, &cursor, 1));
        self.emit(abi::branch(&loop_start));

        self.emit(abi::label(&dot));
        self.emit(abi::compare_immediate(&dot_seen, "0"));
        self.emit(abi::branch_ne(invalid_label)); // a second '.' is malformed
        self.emit(abi::move_immediate(&dot_seen, "Integer", "1"));
        self.emit(abi::branch(&advance));

        self.emit(abi::label(&finish));
        self.emit(abi::compare_immediate(&seen_digit, "0"));
        self.emit(abi::branch_eq(invalid_label)); // no digits at all
                                                  // Zero-pad the kept fraction on the RIGHT to exactly 5 places: `.5` is
                                                  // 0.50000 (frac5 50000), not 0.00005. Fewer than 5 fractional digits
                                                  // scales frac5 up by 10 per missing place; 5-or-more already fills it.
        let pad_loop = self.label("money_parse_pad_loop");
        let pad_done = self.label("money_parse_pad_done");
        self.emit(abi::label(&pad_loop));
        self.emit(abi::compare_immediate(&frac_count, "5"));
        self.emit(abi::branch_ge(&pad_done));
        self.emit(abi::multiply_registers(&frac5, &frac5, &ten));
        self.emit(abi::add_immediate(&frac_count, &frac_count, 1));
        self.emit(abi::branch(&pad_loop));
        self.emit(abi::label(&pad_done));
        // Settle the 6th fractional digit into frac5 under the current mode.
        self.emit(abi::compare_immediate(&d6, "5"));
        self.emit(abi::branch_lo(&no_inc)); // < 5 → truncate
        self.emit(abi::branch_hi(&do_inc)); // > 5 → round up
                                            // d6 == 5: past the half (rest nonzero) rounds up; an exact half uses the mode.
        self.emit(abi::compare_immediate(&rest_nonzero, "0"));
        self.emit(abi::branch_ne(&do_inc));
        self.emit(abi::load_u64(
            &scratch,
            ARENA_STATE_REGISTER,
            ARENA_ROUNDING_MODE_OFFSET,
        ));
        self.emit(abi::compare_immediate(&scratch, "0"));
        self.emit(abi::branch_eq(&do_inc)); // Commercial → away
                                            // Banker → to even: round up only when the kept value's last digit is odd.
        self.emit(abi::move_immediate(&scratch, "Integer", "1"));
        self.emit(abi::and_registers(&digit, &frac5, &scratch));
        self.emit(abi::compare_immediate(&digit, "0"));
        self.emit(abi::branch_eq(&no_inc));
        self.emit(abi::label(&do_inc));
        self.emit(abi::add_immediate(&frac5, &frac5, 1));
        self.emit(abi::label(&no_inc));

        // raw_mag = whole * 100000 + frac5 (a frac5 of 100000 carries naturally).
        self.emit(abi::move_immediate(&scratch, "Integer", "100000"));
        self.emit(abi::multiply_registers(&whole, &whole, &scratch));
        self.emit(abi::add_registers(&whole, &whole, &frac5));
        // Apply the sign and the i64 range check (unsigned magnitude compares).
        self.emit(abi::compare_immediate(&negative, "0"));
        self.emit(abi::branch_ne(&neg_path));
        // Positive: magnitude must fit i64::MAX = 9223372036854775807.
        self.emit(abi::move_immediate(
            &scratch,
            "Integer",
            "9223372036854775807",
        ));
        self.emit(abi::compare_registers(&whole, &scratch));
        self.emit(abi::branch_hi(overflow_label));
        self.emit(abi::move_register(&dst, &whole));
        self.emit(abi::branch(&set_done));
        self.emit(abi::label(&neg_path));
        // Negative: magnitude must fit 2^63 = 9223372036854775808 (i64::MIN);
        // 0 - 2^63 wraps to i64::MIN in two's complement, exactly the min Money.
        self.emit(abi::move_immediate(
            &scratch,
            "Integer",
            "9223372036854775807",
        ));
        self.emit(abi::add_immediate(&scratch, &scratch, 1)); // 2^63, unrepresentable as a positive i64 literal
        self.emit(abi::compare_registers(&whole, &scratch));
        self.emit(abi::branch_hi(overflow_label));
        self.emit(abi::move_immediate(&scratch, "Integer", "0"));
        self.emit(abi::subtract_registers(&dst, &scratch, &whole));
        self.emit(abi::label(&set_done));
        Ok(())
    }

    /// floor/ceil/round of a Money raw to its whole-unit Integer count
    /// (plan-29-G §4.7). `q = raw / 100000` truncated toward zero, then adjusted:
    /// floor toward -∞, ceil toward +∞, round half-away-from-zero.
    pub(crate) fn emit_money_rounding_to_integer(
        &mut self,
        function: &str,
        raw: impl Into<Operand>,
        dst: impl Into<Operand>,
    ) -> Result<(), String> {
        let dst = dst.into();
        let raw = raw.into();
        let scale = self.allocate_register();
        let quotient = self.allocate_register();
        let remainder = self.allocate_register();
        self.emit(abi::move_immediate(&scale, "Integer", "100000"));
        self.emit(abi::signed_divide_registers(&quotient, &raw, &scale));
        self.emit(abi::multiply_subtract_registers(
            &remainder, &quotient, &scale, &raw,
        ));
        self.emit(abi::move_register(dst.clone(), &quotient));
        let done = self.label("math_money_round_done");
        match function {
            "floor" => {
                // remainder < 0 (raw negative, non-zero frac) → toward -∞.
                self.emit(abi::compare_immediate(&remainder, "0"));
                self.emit(abi::branch_ge(&done));
                self.emit(abi::subtract_immediate(dst.clone(), &quotient, 1));
            }
            "ceil" => {
                // remainder > 0 (raw positive, non-zero frac) → toward +∞.
                self.emit(abi::compare_immediate(&remainder, "0"));
                self.emit(abi::branch_le(&done));
                self.emit(abi::add_immediate(dst.clone(), &quotient, 1));
            }
            "round" => {
                // half-away: bump the magnitude when 2*|remainder| >= 100000.
                let abs_rem = self.allocate_register();
                let bump_pos = self.label("math_money_round_bump_pos");
                let bump_neg = self.label("math_money_round_bump_neg");
                let half = self.allocate_register();
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
                self.emit(abi::add_immediate(dst.clone(), &quotient, 1));
                self.emit(abi::branch(&done));
                self.emit(abi::label(&bump_neg));
                self.emit(abi::subtract_immediate(dst.clone(), &quotient, 1));
            }
            _ => unreachable!(),
        }
        self.emit(abi::label(&done));
        Ok(())
    }
}
