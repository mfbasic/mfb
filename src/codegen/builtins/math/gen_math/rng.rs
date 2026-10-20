//! plan-176-S: the `math` RNG — `rand` and `seed`. `lower_math_call` lives in
//! `builtins/math/gen_math/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// Emit a call to `_mfb_rng_next` (advance this thread's PCG64 generator and
    /// return a fresh 64-bit draw in the return register), with the matching
    /// internal call relocation. The call clobbers every caller-saved register.
    fn emit_rng_next_call(&mut self) {
        self.emit(abi::branch_link(RNG_NEXT_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: RNG_NEXT_SYMBOL.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
    }

    /// `math.rand(min, max)` — uniform inclusive integer in `[min, max]`, drawn
    /// from this thread's PCG64 generator. Reports `ErrInvalidArgument` when
    /// `min > max`.
    pub(super) fn lower_math_rand(&mut self, args: &[ValueResult]) -> Result<ValueResult, String> {
        let min = args[0].clone();
        // `rand(Money, Money) → Money` draws uniformly over the raw i64 range, the
        // same Lemire sampling as Integer (the raws are i64) (plan-29-G §4.7).
        if !matches!(min.type_.name().as_ref(), "Integer" | "Money") {
            return Err(format!("math.rand does not accept {}", min.type_));
        }
        let result_type = min.type_.clone();
        let min_slot = self.allocate_stack_object("math_rand_min", 8);
        self.emit(abi::store_u64(
            &min.location,
            abi::stack_pointer(),
            min_slot,
        ));
        let max = args[1].clone();
        if max.type_ != result_type {
            return Err(format!("math.rand does not accept {}", max.type_));
        }
        let max_slot = self.allocate_stack_object("math_rand_max", 8);
        self.emit(abi::store_u64(
            &max.location,
            abi::stack_pointer(),
            max_slot,
        ));
        let range_slot = self.allocate_stack_object("math_rand_range", 8);
        // Lemire rejection threshold and the final result are spilled so they
        // survive the redraw loop's `_mfb_rng_next` calls (which clobber every
        // caller-saved register). Routing every cross-call live value through a
        // stack slot keeps the lowering correct under both register allocators.
        let threshold_slot = self.allocate_stack_object("math_rand_threshold", 8);
        let result_slot = self.allocate_stack_object("math_rand_result", 8);

        // Validate min <= max and compute the inclusive span before the draw
        // loop; `_mfb_rng_next` clobbers the caller-saved registers so the span,
        // threshold, and min are reloaded from their slots after every call.
        self.reset_temporary_registers();
        let min_reg = self.allocate_register();
        let max_reg = self.allocate_register();
        let range_reg = self.allocate_register();
        self.emit(abi::load_u64(&min_reg, abi::stack_pointer(), min_slot));
        self.emit(abi::load_u64(&max_reg, abi::stack_pointer(), max_slot));
        let bounds_valid = self.label("math_rand_bounds_valid");
        self.emit(abi::compare_registers(&min_reg, &max_reg));
        self.emit(abi::branch_le(&bounds_valid));
        self.raise_error_bare("ErrInvalidArgument")?;
        self.emit(abi::label(&bounds_valid));
        // span = (max - min) + 1; wraps to 0 only for the full Integer range,
        // which the `full_range` branch handles by returning a single raw draw.
        self.emit(abi::subtract_registers(&range_reg, &max_reg, &min_reg));
        self.emit(abi::add_immediate(&range_reg, &range_reg, 1));
        self.emit(abi::store_u64(&range_reg, abi::stack_pointer(), range_slot));

        let full_range = self.label("math_rand_full_range");
        let draw = self.label("math_rand_draw");
        let maybe_reject = self.label("math_rand_maybe_reject");
        let accept = self.label("math_rand_accept");
        let done = self.label("math_rand_done");

        // The full Integer range (span == 0) needs no reduction: a single 64-bit
        // draw is already uniform. It is also the one case where `span` is zero,
        // so the threshold division below (which would `#DE`-trap on x86) is only
        // reached on the bounded path.
        self.emit(abi::compare_immediate(&range_reg, "0"));
        self.emit(abi::branch_eq(&full_range));

        // Lemire's rejection sampling for an unbiased inclusive [min, max]
        // (span = max - min + 1). For a draw `raw`, form the 128-bit product
        // m = raw * span: its high word `hi` is a candidate offset in [0, span)
        // and its low word `lo` decides fairness. `lo` values below the biased
        // tail width t = (2^64 mod span) must be redrawn. Compute
        //   t = (2^64 - span) mod span == (0 - span) mod span
        // once here while `span` is in a register (span != 0 on this path).
        let neg_span = self.allocate_register();
        let quotient = self.allocate_register();
        let threshold = self.allocate_register();
        self.emit(abi::subtract_registers(&neg_span, abi::ZERO, &range_reg));
        self.emit(abi::unsigned_divide_registers(
            &quotient, &neg_span, &range_reg,
        ));
        self.emit(abi::multiply_subtract_registers(
            &threshold, &quotient, &range_reg, &neg_span,
        ));
        self.emit(abi::store_u64(
            &threshold,
            abi::stack_pointer(),
            threshold_slot,
        ));

        // draw: raw = _mfb_rng_next(); hi = umulh(raw, span); lo = raw * span.
        // Accept when lo >= span (no draw could then fall in the tail); when
        // lo < span, redraw while lo < t, else accept `hi`.
        self.emit(abi::label(&draw));
        self.emit_rng_next_call();
        self.reset_temporary_registers();
        let span_reg = self.allocate_register();
        let product_hi = self.allocate_register();
        let product_lo = self.allocate_register();
        self.emit(abi::load_u64(&span_reg, abi::stack_pointer(), range_slot));
        self.emit(abi::unsigned_multiply_high_registers(
            &product_hi,
            abi::return_register(),
            &span_reg,
        ));
        self.emit(abi::multiply_registers(
            &product_lo,
            abi::return_register(),
            &span_reg,
        ));
        self.emit(abi::compare_registers(&product_lo, &span_reg));
        self.emit(abi::branch_lo(&maybe_reject)); // lo < span => check the tail
        self.emit(abi::branch(&accept));
        self.emit(abi::label(&maybe_reject));
        let threshold_reg = self.allocate_register();
        self.emit(abi::load_u64(
            &threshold_reg,
            abi::stack_pointer(),
            threshold_slot,
        ));
        self.emit(abi::compare_registers(&product_lo, &threshold_reg));
        self.emit(abi::branch_lo(&draw)); // lo < t => biased tail, redraw
        self.emit(abi::label(&accept));
        let min_reload = self.allocate_register();
        let offset = self.allocate_register();
        self.emit(abi::load_u64(&min_reload, abi::stack_pointer(), min_slot));
        self.emit(abi::add_registers(&offset, &min_reload, &product_hi));
        self.emit(abi::store_u64(&offset, abi::stack_pointer(), result_slot));
        self.emit(abi::branch(&done));

        // Full Integer range: a single raw draw is uniform over all of Integer.
        self.emit(abi::label(&full_range));
        self.emit_rng_next_call();
        self.emit(abi::store_u64(
            abi::return_register(),
            abi::stack_pointer(),
            result_slot,
        ));

        self.emit(abi::label(&done));
        self.reset_temporary_registers();
        let result = self.allocate_register();
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            origin: None,
            type_: result_type,
            location: Operand::from(result),
            text: format!("math.rand({}, {})", min.text, max.text),
        })
    }

    /// `math.seed(value)` — reseed this thread's PCG64 generator. Returns Nothing.
    pub(super) fn lower_math_seed(&mut self, arg: &ValueResult) -> Result<ValueResult, String> {
        let value = arg.clone();
        if value.type_ != ParameterType::Integer {
            return Err(format!("math.seed does not accept {}", value.type_));
        }
        let text = format!("math.seed({})", value.text);
        self.emit(abi::move_register(abi::c_arg(1), &value.location));
        self.emit(abi::move_register(
            abi::return_register(),
            ARENA_STATE_REGISTER,
        ));
        self.emit(abi::branch_link(RNG_SEED_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: RNG_SEED_SYMBOL.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
        Ok(ValueResult {
            origin: None,
            type_: ParameterType::Nothing,
            location: abi::return_register().into(),
            text,
        })
    }
}
