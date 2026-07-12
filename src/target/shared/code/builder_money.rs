//! Inline lowering for the `money::` package callables (plan-29-D).
//!
//! `setRounding` / `getRounding` read and write the per-arena rounding-mode field
//! (`ARENA_ROUNDING_MODE_OFFSET`); `round(value, decimals)` settles a Money to a
//! given number of places under the current mode via the shared
//! `emit_apply_rounding` helper. The `Rounding` enum itself is declared in
//! `money_package.mfb`; its members carry their discriminants (`Commercial = 0`,
//! `Banker = 1`), which are exactly the stored values.

use super::*;

impl CodeBuilder<'_> {
    pub(super) fn lower_money_call(
        &mut self,
        function: &str,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        match function {
            "setRounding" if args.len() == 1 => self.lower_money_set_rounding(&args[0]),
            "getRounding" if args.is_empty() => self.lower_money_get_rounding(),
            "round" if args.len() == 2 => self.lower_money_round(&args[0], &args[1]),
            other => Err(format!(
                "native code plan cannot lower money.{other}/{}",
                args.len()
            )),
        }
    }

    /// `money::setRounding(mode)` — store `mode & 1` into the arena rounding-mode
    /// field. The `Rounding` value arrives as its i64 discriminant. Returns Nothing.
    fn lower_money_set_rounding(&mut self, arg: &NirValue) -> Result<ValueResult, String> {
        let mode = self.lower_value(arg)?;
        let text = format!("money.setRounding({})", mode.text);
        let masked = self.allocate_register()?;
        let one = self.allocate_register()?;
        self.emit(abi::move_immediate(&one, "Integer", "1"));
        self.emit(abi::and_registers(&masked, &mode.location, &one));
        self.emit(abi::store_u64(
            &masked,
            ARENA_STATE_REGISTER,
            ARENA_ROUNDING_MODE_OFFSET,
        ));
        Ok(ValueResult {
            type_: "Nothing".to_string(),
            location: abi::return_register().to_string(),
            text,
        })
    }

    /// `money::getRounding()` — load the arena rounding-mode field (`0`/`1`) as a
    /// `Rounding` value (the enum is i64-carried by its discriminant).
    fn lower_money_get_rounding(&mut self) -> Result<ValueResult, String> {
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(
            &result,
            ARENA_STATE_REGISTER,
            ARENA_ROUNDING_MODE_OFFSET,
        ));
        Ok(ValueResult {
            type_: "Rounding".to_string(),
            location: result,
            text: "money.getRounding()".to_string(),
        })
    }

    /// `money::round(value, decimals)` — settle `value` to `decimals` places under
    /// the current mode (plan-29-D §4.4). `decimals` outside `0..5` fails with
    /// ErrInvalidArgument; `5` is the identity. Exact integer arithmetic: divide by
    /// `10^(5-decimals)`, round the remainder through `emit_apply_rounding`, then
    /// re-multiply (which cannot overflow — the product stays within one divisor of
    /// the original raw).
    fn lower_money_round(
        &mut self,
        value_arg: &NirValue,
        decimals_arg: &NirValue,
    ) -> Result<ValueResult, String> {
        let value = self.lower_value(value_arg)?;
        let decimals = self.lower_value(decimals_arg)?;
        let text = format!("money.round({}, {})", value.text, decimals.text);
        let raw = value.location;
        let dec = decimals.location;

        // decimals must be in 0..=5.
        let lo_ok = self.label("money_round_lo_ok");
        self.emit(abi::compare_immediate(&dec, "0"));
        self.emit(abi::branch_ge(&lo_ok));
        self.emit_invalid_argument_return()?;
        self.emit(abi::label(&lo_ok));
        let hi_ok = self.label("money_round_hi_ok");
        self.emit(abi::compare_immediate(&dec, "5"));
        self.emit(abi::branch_le(&hi_ok));
        self.emit_invalid_argument_return()?;
        self.emit(abi::label(&hi_ok));

        // divisor = 10^(5 - decimals), built by a bounded (<=5) multiply loop.
        let exponent = self.allocate_register()?;
        self.emit(abi::move_immediate(&exponent, "Integer", "5"));
        self.emit(abi::subtract_registers(&exponent, &exponent, &dec));
        let divisor = self.allocate_register()?;
        self.emit(abi::move_immediate(&divisor, "Integer", "1"));
        let ten = self.allocate_register()?;
        self.emit(abi::move_immediate(&ten, "Integer", "10"));
        let loop_label = self.label("money_round_pow_loop");
        let loop_done = self.label("money_round_pow_done");
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate(&exponent, "0"));
        self.emit(abi::branch_eq(&loop_done));
        self.emit(abi::multiply_registers(&divisor, &divisor, &ten));
        self.emit(abi::subtract_immediate(&exponent, &exponent, 1));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&loop_done));

        // q = raw / divisor, r = raw - q*divisor, sign_neg = raw < 0.
        let quotient = self.allocate_register()?;
        self.emit(abi::signed_divide_registers(&quotient, &raw, &divisor));
        let remainder = self.allocate_register()?;
        self.emit(abi::multiply_subtract_registers(
            &remainder, &quotient, &divisor, &raw,
        ));
        let sign_neg = self.allocate_register()?;
        self.emit(abi::arithmetic_shift_right_immediate(&sign_neg, &raw, 63));
        let rounded = self.allocate_register()?;
        self.emit_apply_rounding(&rounded, &quotient, &remainder, &divisor, &sign_neg)?;
        // result = rounded * divisor (back to Money scale; cannot overflow).
        let result = self.allocate_register()?;
        self.emit(abi::multiply_registers(&result, &rounded, &divisor));
        Ok(ValueResult {
            type_: "Money".to_string(),
            location: result,
            text,
        })
    }
}
