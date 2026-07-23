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

use super::*;

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
    pub(super) fn emit_apply_rounding(
        &mut self,
        dst: &str,
        quotient: &str,
        remainder: &str,
        abs_divisor: &str,
        sign_neg: &str,
    ) -> Result<(), String> {
        let round_up = self.label("money_round_up");
        let round_down = self.label("money_round_down");
        let keep = self.label("money_round_keep");

        // abs_rem = |remainder|
        let abs_rem = self.allocate_register()?;
        self.emit(abi::move_register(&abs_rem, remainder));
        self.emit_abs_i64(&abs_rem)?;
        // half = |div| - |rem|  (in [1, |div|]); tie when |rem| == half.
        let half = self.allocate_register()?;
        self.emit(abi::subtract_registers(&half, abs_divisor, &abs_rem));

        // Default: keep the truncated quotient.
        self.emit(abi::move_register(dst, quotient));
        self.emit(abi::compare_registers(&abs_rem, &half));
        self.emit(abi::branch_lt(&keep)); // |rem| < half  -> below the half, keep
        self.emit(abi::branch_gt(&round_up)); // |rem| > half  -> past the half, round away

        // Exact tie (|rem| == half): Commercial rounds away, Banker rounds to even.
        let mode = self.allocate_register()?;
        self.emit(abi::load_u64(
            &mode,
            ARENA_STATE_REGISTER,
            ARENA_ROUNDING_MODE_OFFSET,
        ));
        self.emit(abi::compare_immediate(&mode, "0"));
        self.emit(abi::branch_eq(&round_up)); // Commercial -> away
                                              // Banker: round only when the truncated quotient is odd (to reach even).
        let one = self.allocate_register()?;
        self.emit(abi::move_immediate(&one, "Integer", "1"));
        let parity = self.allocate_register()?;
        self.emit(abi::and_registers(&parity, quotient, &one));
        self.emit(abi::compare_immediate(&parity, "0"));
        self.emit(abi::branch_eq(&keep)); // even quotient -> keep, already even

        // Round the magnitude away from zero: +1 when positive, -1 when negative.
        self.emit(abi::label(&round_up));
        self.emit(abi::compare_immediate(sign_neg, "0"));
        self.emit(abi::branch_ne(&round_down));
        self.emit(abi::add_immediate(dst, quotient, 1));
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
    pub(super) fn emit_money_binary(
        &mut self,
        op: &str,
        left: &ValueResult,
        right: &ValueResult,
        dst: &str,
    ) -> Result<String, String> {
        let l_money = left.type_ == "Money";
        let r_money = right.type_ == "Money";
        match op {
            // `M ± M` and `M MOD M` are exact integer ops on the raw i64.
            "+" | "-" | "MOD" => {
                self.emit_integer_binary(op, left, right, dst, false)?;
                Ok(dst.to_string())
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
        dst: &str,
    ) -> Result<String, String> {
        match scalar.type_.as_str() {
            // Exact integer scaling: `raw * k`, overflow-checked.
            "Integer" | "Byte" => {
                self.emit_checked_integer_multiply(dst, &money.location, &scalar.location)?;
                Ok(dst.to_string())
            }
            // Exact binary fixed-point scaling: `raw * fixed_raw / 2^32` is exactly
            // what `emit_fixed_multiply` computes when fed the Money raw as the
            // left operand and the Q32.32 raw as the right (plan-29-F §4.1).
            "Fixed" => {
                self.emit_fixed_multiply(dst, &money.location, &scalar.location)?;
                Ok(dst.to_string())
            }
            // Inexact floating scaling (plan-29-F §4.3).
            "Float" => self.emit_money_scale_float(&money.location, scalar, dst, false),
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
        dst: &str,
    ) -> Result<String, String> {
        match scalar.type_.as_str() {
            // `raw / k`, mode-rounded (plan-29-E §4.2). `k == 0` → ErrInvalidArgument.
            "Integer" | "Byte" => {
                self.emit_nonzero_or_invalid(&scalar.location)?;
                self.emit_integer_division_overflow_check(&money.location, &scalar.location)?;
                let quotient = self.allocate_register()?;
                self.emit(abi::signed_divide_registers(
                    &quotient,
                    &money.location,
                    &scalar.location,
                ));
                let remainder = self.allocate_register()?;
                // remainder = raw - quotient * k
                self.emit(abi::multiply_subtract_registers(
                    &remainder,
                    &quotient,
                    &scalar.location,
                    &money.location,
                ));
                let abs_div = self.allocate_register()?;
                self.emit(abi::move_register(&abs_div, &scalar.location));
                self.emit_abs_i64(&abs_div)?;
                // sign_neg = -1 (nonzero) when the signs of raw and k differ.
                let sign_neg = self.allocate_register()?;
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
                let min_divisor = self.allocate_register()?;
                // i64::MIN as its unsigned bit pattern (2^63); `move_immediate`
                // takes the u64 pattern, not the signed "-9223372036854775808".
                self.emit(abi::move_immediate(&min_divisor, "Integer", F64_SIGN_BIT));
                let not_min = self.label("money_div_scalar_not_min");
                let div_done = self.label("money_div_scalar_done");
                self.emit(abi::compare_registers(&scalar.location, &min_divisor));
                self.emit(abi::branch_ne(&not_min));
                self.emit(abi::move_register(dst, &quotient));
                self.emit(abi::branch(&div_done));
                self.emit(abi::label(&not_min));
                self.emit_apply_rounding(dst, &quotient, &remainder, &abs_div, &sign_neg)?;
                self.emit(abi::label(&div_done));
                Ok(dst.to_string())
            }
            // `raw * 2^32 / fixed_raw` is exactly `emit_fixed_divide(raw, fixed_raw)`
            // (plan-29-F §4.2); it guards `fixed_raw == 0` → ErrInvalidArgument.
            "Fixed" => {
                self.emit_fixed_divide(dst, &money.location, &scalar.location)?;
                Ok(dst.to_string())
            }
            "Float" => self.emit_money_scale_float(&money.location, scalar, dst, true),
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
        let da = self.allocate_fp_register()?;
        let db = self.allocate_fp_register()?;
        self.emit(abi::signed_convert_to_float_d(&da, &left.location));
        self.emit(abi::signed_convert_to_float_d(&db, &right.location));
        let result = self.allocate_fp_register()?;
        self.emit(abi::float_divide_d(&result, &da, &db));
        Ok(result)
    }

    /// `Money DIV scalar|Money → Float` (plan-29-E §4.5 / plan-29-F §4.4): forced
    /// Float division, both operands promoted to their true f64 value.
    fn emit_money_div_to_float(
        &mut self,
        left: &ValueResult,
        right: &ValueResult,
    ) -> Result<String, String> {
        let da = self.allocate_fp_register()?;
        let db = self.allocate_fp_register()?;
        self.load_numeric_as_double(&da, left)?;
        self.load_numeric_as_double(&db, right)?;
        let result = self.allocate_fp_register()?;
        self.emit(abi::float_divide_d(&result, &da, &db));
        Ok(result)
    }

    /// `Money * Float` / `Money / Float → Money` (plan-29-F §4.3). Because the
    /// result raw equals `raw * fval` (resp. `raw / fval`) — the SCALE rides
    /// through — the whole computation is done in f64 and rounded back to the raw
    /// i64 under the current mode. A non-finite Float operand → ErrInvalidFormat;
    /// a zero divisor → ErrInvalidArgument; an out-of-range result → ErrOverflow.
    fn emit_money_scale_float(
        &mut self,
        money_raw: &str,
        scalar: &ValueResult,
        dst: &str,
        divide: bool,
    ) -> Result<String, String> {
        let fval = self.allocate_fp_register()?;
        self.load_numeric_as_double(&fval, scalar)?;
        self.emit_float_finite_or_invalid(&fval)?;
        let money_d = self.allocate_fp_register()?;
        self.emit(abi::signed_convert_to_float_d(&money_d, money_raw));
        let result = self.allocate_fp_register()?;
        if divide {
            // A Money result, so a zero divisor is ErrInvalidArgument (not a Float
            // boundary) — plan-29-F Open Decisions.
            let ok = self.label("money_float_div_ok");
            self.emit(abi::float_compare_zero_d(&fval));
            self.emit(abi::branch_ne(&ok));
            self.emit_invalid_argument_return()?;
            self.emit(abi::label(&ok));
            self.emit(abi::float_divide_d(&result, &money_d, &fval));
        } else {
            self.emit(abi::float_multiply_d(&result, &money_d, &fval));
        }
        self.emit_round_double_to_money_raw(&result, dst)?;
        Ok(dst.to_string())
    }

    /// Fail with ErrInvalidFormat when the f64 in `value` is NaN or ±Inf (its
    /// biased exponent is all ones), mirroring `toFixed(Float)`'s guard.
    pub(super) fn emit_float_finite_or_invalid(&mut self, value: &str) -> Result<(), String> {
        let ok = self.label("money_finite_ok");
        let invalid = self.label("money_finite_invalid");
        let bits = self.allocate_register()?;
        let exponent = self.allocate_register()?;
        let mask = self.allocate_register()?;
        self.emit(abi::float_move_x_from_d(&bits, value));
        self.emit_float_exponent_classify(&exponent, &mask, &bits);
        self.emit(abi::branch_ne(&ok));
        self.emit(abi::label(&invalid));
        self.emit_invalid_format_return()?;
        self.emit(abi::label(&ok));
        Ok(())
    }

    /// Round the f64 result raw in `value` to the Money raw i64 in `dst` under the
    /// current mode: Commercial rounds ties away from zero, Banker rounds ties to
    /// even. A non-finite or out-of-range magnitude (`|value| >= 2^63`) fails with
    /// ErrOverflow.
    pub(super) fn emit_round_double_to_money_raw(
        &mut self,
        value: &str,
        dst: &str,
    ) -> Result<(), String> {
        let overflow = self.label("money_round_overflow");
        let range_ok = self.label("money_round_range_ok");
        let round_away = self.label("money_round_f_away");
        let round_pos = self.label("money_round_f_pos");
        let done = self.label("money_round_f_done");
        let scratch = self.temporary_vreg();

        // Range guard: |value| >= 2^63 (or non-finite) overflows the raw i64.
        let magnitude = self.allocate_fp_register()?;
        self.emit(abi::float_abs_d(&magnitude, value));
        let limit = self.allocate_fp_register()?;
        self.emit_f64_const(&limit, scratch.as_str(), 9_223_372_036_854_775_808.0);
        self.emit(abi::float_compare_d(&magnitude, &limit));
        self.emit(abi::branch_mi(&range_ok)); // |value| < 2^63 (ordered, less-than)
        self.emit(abi::label(&overflow));
        self.emit_overflow_return()?;
        self.emit(abi::label(&range_ok));

        // q = trunc(value) toward zero.
        let quotient = self.allocate_register()?;
        self.emit(abi::float_convert_to_signed_x(&quotient, value));
        // frac = value - (q as f64), in (-1, 1); abs_frac = |frac|.
        let q_f = self.allocate_fp_register()?;
        self.emit(abi::signed_convert_to_float_d(&q_f, &quotient));
        let frac = self.allocate_fp_register()?;
        self.emit(abi::float_subtract_d(&frac, value, &q_f));
        let abs_frac = self.allocate_fp_register()?;
        self.emit(abi::float_abs_d(&abs_frac, &frac));
        let half = self.allocate_fp_register()?;
        self.emit_f64_const(&half, scratch.as_str(), 0.5);

        self.emit(abi::move_register(dst, &quotient)); // default: keep the truncation
        self.emit(abi::float_compare_d(&abs_frac, &half));
        self.emit(abi::branch_mi(&done)); // abs_frac < 0.5 → keep
        self.emit(abi::branch_gt(&round_away)); // abs_frac > 0.5 → round away
                                                // Exact half tie: Commercial rounds away; Banker rounds to even.
        let mode = self.allocate_register()?;
        self.emit(abi::load_u64(
            &mode,
            ARENA_STATE_REGISTER,
            ARENA_ROUNDING_MODE_OFFSET,
        ));
        self.emit(abi::compare_immediate(&mode, "0"));
        self.emit(abi::branch_eq(&round_away)); // Commercial → away
        let one = self.allocate_register()?;
        self.emit(abi::move_immediate(&one, "Integer", "1"));
        let parity = self.allocate_register()?;
        self.emit(abi::and_registers(&parity, &quotient, &one));
        self.emit(abi::compare_immediate(&parity, "0"));
        self.emit(abi::branch_eq(&done)); // even quotient → keep

        self.emit(abi::label(&round_away));
        // Round the magnitude away from zero: +1 when value >= 0, −1 when value < 0.
        self.emit(abi::float_compare_zero_d(value));
        self.emit(abi::branch_ge(&round_pos));
        self.emit(abi::subtract_immediate(dst, &quotient, 1));
        self.emit(abi::branch(&done));
        self.emit(abi::label(&round_pos));
        self.emit(abi::add_immediate(dst, &quotient, 1));
        self.emit(abi::label(&done));
        Ok(())
    }

    /// floor/ceil/round of a Money raw to its whole-unit Integer count
    /// (plan-29-G §4.7). `q = raw / 100000` truncated toward zero, then adjusted:
    /// floor toward -∞, ceil toward +∞, round half-away-from-zero.
    pub(super) fn emit_money_rounding_to_integer(
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
        self.emit(abi::multiply_subtract_registers(
            &remainder, &quotient, &scale, raw,
        ));
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
}
