//! plan-176-U: the boolean, short-circuit, xor, arithmetic and unary-negation operator lowerings and
//! their bound-elision helpers. The module doc lives in
//! `engine/operators/builder_numeric/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    pub(crate) fn lower_boolean_binary(
        &mut self,
        op: BinaryOp,
        left: &NirValue,
        right: &NirValue,
    ) -> Result<ValueResult, String> {
        match op {
            BinaryOp::And => self.lower_short_circuit_and(left, right),
            BinaryOp::Or => self.lower_short_circuit_or(left, right),
            BinaryOp::Xor => self.lower_boolean_xor(left, right),
            // Reachable only from malformed IR: `ir::verify` requires Boolean
            // operands for exactly these three, so every other operator on a
            // Boolean pair is rejected before lowering.
            BinaryOp::Equal
            | BinaryOp::NotEqual
            | BinaryOp::Less
            | BinaryOp::LessEqual
            | BinaryOp::Greater
            | BinaryOp::GreaterEqual
            | BinaryOp::Concat
            | BinaryOp::Add
            | BinaryOp::Subtract
            | BinaryOp::Multiply
            | BinaryOp::Divide
            | BinaryOp::Mod
            | BinaryOp::IntDiv
            | BinaryOp::Power => Err(format!(
                "native code plan does not lower boolean operator '{}'",
                op.name()
            )),
        }
    }

    fn lower_short_circuit_and(
        &mut self,
        left: &NirValue,
        right: &NirValue,
    ) -> Result<ValueResult, String> {
        let left = self.lower_value(left)?;
        let result = self.allocate_register();
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
            origin: None,
            type_: ParameterType::Boolean,
            location: Operand::from(result),
            text: format!("({} AND {})", left.text, right.text),
        })
    }

    fn lower_short_circuit_or(
        &mut self,
        left: &NirValue,
        right: &NirValue,
    ) -> Result<ValueResult, String> {
        let left = self.lower_value(left)?;
        let result = self.allocate_register();
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
            origin: None,
            type_: ParameterType::Boolean,
            location: Operand::from(result),
            text: format!("({} OR {})", left.text, right.text),
        })
    }

    fn lower_boolean_xor(
        &mut self,
        left: &NirValue,
        right: &NirValue,
    ) -> Result<ValueResult, String> {
        // Spill `left` across `lower_value(right)` (bug-137): `right` may be a call
        // that re-hands `left`'s caller-saved register and clobbers x0–x17, so
        // reading `left.location` afterwards would see `right`'s value. Mirror the
        // arithmetic-binary sibling: spill left, lower right, reload both.
        let left = self.lower_value(left)?;
        let left_slot = self.allocate_stack_object("xor_left", 8);
        self.emit(abi::store_u64(
            &left.location,
            abi::stack_pointer(),
            left_slot,
        ));
        let left_text = left.text.clone();
        let right = self.lower_value(right)?;
        let right_slot = self.allocate_stack_object("xor_right", 8);
        self.emit(abi::store_u64(
            &right.location,
            abi::stack_pointer(),
            right_slot,
        ));
        let right_text = right.text.clone();
        self.reset_temporary_registers();
        let left_register = self.allocate_register();
        let right_register = self.allocate_register();
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
        let result = self.allocate_register();
        self.emit(abi::exclusive_or_registers(
            &result,
            &left_register,
            &right_register,
        ));
        Ok(ValueResult {
            origin: None,
            type_: ParameterType::Boolean,
            location: Operand::from(result),
            text: format!("({left_text} XOR {right_text})"),
        })
    }

    pub(crate) fn lower_arithmetic_binary(
        &mut self,
        op: BinaryOp,
        left: &NirValue,
        right: &NirValue,
    ) -> Result<ValueResult, String> {
        // plan-39 I1: decide overflow-check elision from the *NIR* operands before
        // they are lowered (and `left`/`right` are shadowed by their ValueResults).
        let elide_overflow = (op == BinaryOp::Subtract && self.integer_sub_elidable(left, right))
            || (op == BinaryOp::Add && self.integer_add_elidable(left, right));
        let left = self.lower_value(left)?;
        // `d`-native float fast path (plan-01 float-dnative): when the operation
        // yields a `Float`, both operands are consumed directly in the FP domain
        // with no GPR spill/reload — the `d`-register-native operand never leaves
        // its FP register. The liveness allocator keeps the left operand's
        // register live across the right operand's lowering (spilling it as a `d`
        // across any nested call), so no manual round-trip is needed. Only
        // available under `LinearScan`, and only for the pure-FP operators:
        // `MOD`/`^` run hardcoded-register kernels (the in-tree `fmod`/`pow`)
        // that assume the reset register file and GPR-reloaded operands the
        // general path provides.
        //
        // The result type — not the left operand type — gates this: `Float -
        // Fixed` promotes to `Fixed` (Fixed dominates), so it must take the
        // general path's `Fixed` lowering. The right operand's static type
        // decides the result type without lowering it twice; an unknown type
        // conservatively falls through to the general path.
        if left.type_ == ParameterType::Float
            && matches!(
                op,
                BinaryOp::Add
                    | BinaryOp::Subtract
                    | BinaryOp::Multiply
                    | BinaryOp::Divide
                    | BinaryOp::IntDiv
            )
        {
            let result_is_float = self
                .static_type_name(right)
                .map(|right_type| {
                    promoted_binary_type(op, &ParameterType::Float, &right_type)
                        == ParameterType::Float
                })
                .unwrap_or(false);
            if result_is_float {
                return self.lower_float_arithmetic_dnative(op, left, right);
            }
        }
        // Carry FP-residency (plan-03 Stage C) across the operand slot
        // round-trip: the reloaded operand register holds the same value, so it
        // is resident in the same `d`-register.
        let left_resident = self.float_residents.get(&left.location.render()).cloned();
        let left_slot = self.allocate_stack_object("arith_left", 8);
        // A `d`-native float operand (possible here only as the right operand of
        // an `Integer op Float`) materializes its bits into a GPR before the
        // integer-style slot spill (plan-01 float-dnative §4.1). For every
        // GP-native value this is the identity.
        let left_spill = self.float_value_as_gpr(&left)?;
        self.emit(abi::store_u64(&left_spill, abi::stack_pointer(), left_slot));
        let right = self.lower_value(right)?;
        let right_resident = self.float_residents.get(&right.location.render()).cloned();
        let right_slot = self.allocate_stack_object("arith_right", 8);
        let right_spill = self.float_value_as_gpr(&right)?;
        self.emit(abi::store_u64(
            &right_spill,
            abi::stack_pointer(),
            right_slot,
        ));
        let left_text = left.text.clone();
        let right_text = right.text.clone();
        let result_type = promoted_binary_type(op, &left.type_, &right.type_);
        self.reset_temporary_registers();
        let left_register = self.allocate_register();
        let right_register = self.allocate_register();
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
        if let Some(fp) = left_resident {
            self.float_residents.insert(left_register.to_string(), fp);
        }
        if let Some(fp) = right_resident {
            self.float_residents.insert(right_register.to_string(), fp);
        }
        let left = ValueResult {
            origin: None,
            type_: left.type_,
            location: Operand::from(left_register),
            text: left_text,
        };
        let right = ValueResult {
            origin: None,
            type_: right.type_,
            location: Operand::from(right_register),
            text: right_text,
        };
        let register = self.allocate_register();
        // The result location is `register` for every type except a `d`-native
        // float op, which keeps its result in an FP register (returned by
        // `emit_float_binary`). This path is reached for `Float` only in the
        // `Integer op Float` case (a float left operand takes the fast path
        // above); the integer/fixed cases always land in `register`.
        let mut result_location: abi::Reg = register.into();
        match &result_type {
            ParameterType::Byte | ParameterType::Integer => {
                self.emit_integer_binary_checked(
                    op,
                    &left,
                    &right,
                    &register,
                    result_type == ParameterType::Byte,
                    elide_overflow,
                )?;
            }
            ParameterType::Fixed => {
                if left.type_ == ParameterType::Fixed && right.type_ == ParameterType::Fixed {
                    self.emit_fixed_binary(op, &left, &right, &register)?;
                } else {
                    // Float-to-Fixed conversion uses x8-x12 as internal scratch registers.
                    // Keep promoted Fixed operands above that range so conversion cannot clobber them.
                    while self.next_register <= 12 {
                        let _ = self.allocate_register();
                    }
                    let left_fixed = self.allocate_register();
                    let right_fixed = self.allocate_register();
                    let left_fixed_slot = self.allocate_stack_object("arith_left_fixed", 8);
                    self.load_numeric_as_fixed(&left_fixed, &left)?;
                    self.emit(abi::store_u64(
                        &left_fixed,
                        abi::stack_pointer(),
                        left_fixed_slot,
                    ));
                    self.emit(abi::load_u64(
                        &right.location,
                        abi::stack_pointer(),
                        right_slot,
                    ));
                    self.load_numeric_as_fixed(&right_fixed, &right)?;
                    self.emit(abi::load_u64(
                        &left_fixed,
                        abi::stack_pointer(),
                        left_fixed_slot,
                    ));
                    let left = ValueResult {
                        origin: None,
                        type_: ParameterType::Fixed,
                        location: Operand::from(left_fixed),
                        text: left.text.clone(),
                    };
                    let right = ValueResult {
                        origin: None,
                        type_: ParameterType::Fixed,
                        location: Operand::from(right_fixed),
                        text: right.text.clone(),
                    };
                    self.emit_fixed_binary(op, &left, &right, &register)?;
                }
            }
            ParameterType::Float => {
                // A Money operand with a Float result is the `M / M` ratio or a
                // `Money DIV …` — routed through the Money dispatcher, which
                // produces the f64 result (plan-29-E §4.3/§4.5).
                if left.type_ == ParameterType::Money || right.type_ == ParameterType::Money {
                    result_location = self.emit_money_binary(op, &left, &right, &register)?;
                } else {
                    result_location = self.emit_float_binary(op, &left, &right, &register)?;
                }
            }
            // A Money result (`M ± M`, `M * k`, `M / k`, `M MOD M`) is settled by
            // the Money dispatcher (plan-29-C/E/F).
            ParameterType::Money => {
                result_location = self.emit_money_binary(op, &left, &right, &register)?;
            }
            other => {
                return Err(format!(
                    "native code plan cannot lower arithmetic result type '{other}'"
                ));
            }
        }
        Ok(ValueResult {
            origin: None,
            type_: result_type.clone(),
            location: Operand::from(result_location),
            text: format!("({} {} {})", left.text, op.name(), right.text),
        })
    }

    /// `d`-native float arithmetic (plan-01 float-dnative): both operands are
    /// consumed in the FP domain and the result stays in an FP virtual register,
    /// so no GPR spill/reload round-trip is emitted. `left` is the already-lowered
    /// `Float` left operand (so the result is `Float`); `right` is its NIR value,
    /// lowered here. The left operand's register survives the right operand's
    /// lowering under the liveness allocator (spilled as a `d` across any nested
    /// call), so it does not need to be manually preserved.
    fn lower_float_arithmetic_dnative(
        &mut self,
        op: BinaryOp,
        left: ValueResult,
        right: &NirValue,
    ) -> Result<ValueResult, String> {
        let left_text = left.text.clone();
        let right = self.lower_value(right)?;
        let result_type = promoted_binary_type(op, &left.type_, &right.type_);
        let right_text = right.text.clone();
        // `dst` is used only by the GP-result operators (`MOD`/`^`); the pure-FP
        // operators ignore it and return their FP register.
        let dst = self.allocate_register();
        let location = self.emit_float_binary(op, &left, &right, &dst)?;
        Ok(ValueResult {
            origin: None,
            type_: result_type.clone(),
            location: Operand::from(location),
            text: format!(
                "({op_left} {} {op_right})",
                op.name(),
                op_left = left_text,
                op_right = right_text
            ),
        })
    }

    pub(crate) fn lower_numeric_unary_negation(
        &mut self,
        operand: ValueResult,
    ) -> Result<ValueResult, String> {
        let register = self.allocate_register();
        let mut location = register;
        match &operand.type_ {
            ParameterType::Byte => {
                let ok = self.label("byte_unary_ok");
                self.emit(abi::compare_immediate(&operand.location, "0"));
                self.emit(abi::branch_eq(&ok));
                self.raise_error_bare("ErrUnderflow")?;
                self.emit(abi::label(&ok));
                self.emit(abi::move_register(&register, &operand.location));
            }
            ParameterType::Integer => {
                self.emit_min_i64_negation_check(&operand.location, "integer_unary")?;
                let zero = self.allocate_register();
                self.emit(abi::move_immediate(&zero, "Integer", "0"));
                self.emit(abi::subtract_registers(&register, &zero, &operand.location));
            }
            ParameterType::Fixed => {
                self.emit_min_i64_negation_check(&operand.location, "fixed_unary")?;
                let zero = self.allocate_register();
                self.emit(abi::move_immediate(&zero, "Integer", "0"));
                self.emit(abi::subtract_registers(&register, &zero, &operand.location));
            }
            // Money is a signed i64 raw, so negate is the Integer path with the
            // same INT64_MIN check (raw `-9223372036854775808` = the min Money,
            // `-92233720368547.75808`, cannot be negated) (plan-29-C §4.3).
            ParameterType::Money => {
                self.emit_min_i64_negation_check(&operand.location, "money_unary")?;
                let zero = self.allocate_register();
                self.emit(abi::move_immediate(&zero, "Integer", "0"));
                self.emit(abi::subtract_registers(&register, &zero, &operand.location));
            }
            ParameterType::Float => {
                // Negation just flips the sign bit, so a finite operand stays
                // finite — and every live MFBASIC Float is finite (inf/NaN are
                // always errors, never values). No overflow/NaN check is needed.
                // `d`-native: negate in the FP domain and keep the result in its
                // FP register, so the value never round-trips through a GPR
                // (plan-01 float-dnative).
                let d_operand = self.operand_as_double(&operand)?;
                let d_res = self.allocate_fp_register();
                self.emit(abi::float_negate_d(&d_res, &d_operand));
                location = d_res;
            }
            other => {
                return Err(format!(
                    "native code plan does not lower unary operator '-' for {other}"
                ));
            }
        }
        Ok(ValueResult {
            origin: None,
            type_: operand.type_,
            location: Operand::from(location),
            text: format!("(-{})", operand.text),
        })
    }

    /// plan-39 I1: whether `left - right` on `Integer`s provably cannot overflow,
    /// given the guard-derived lower bounds tracked in `integer_lower_bounds`.
    /// Fires only for `local - positiveConst` where the local has a proven lower
    /// bound `C` and `C - const` does not underflow (`checked_sub` is `Some`). A
    /// signed subtraction of a positive constant never overflows *upward*
    /// (`local - c < local <= i64::MAX`), so the lower bound is the whole proof.
    /// Conservative default: `false` (the check stays), so this can only ever
    /// remove a check that was provably unnecessary.
    fn integer_sub_elidable(&self, left: &NirValue, right: &NirValue) -> bool {
        let NirValue::Local(name) = left else {
            return false;
        };
        let Some(&lower) = self.integer_lower_bounds.get(name) else {
            return false;
        };
        let NirValue::Const { type_, value } = right else {
            return false;
        };
        if !matches!(type_, ParameterType::Integer) {
            return false;
        }
        let Ok(c) = value.parse::<i64>() else {
            return false;
        };
        // Only a strictly-positive constant subtrahend: then `local - c` can only
        // underflow, and `lower - c` not underflowing bounds the whole range.
        c > 0 && lower.checked_sub(c).is_some()
    }

    /// plan-39 I1 (upper side): whether `left + right` provably cannot overflow.
    /// Fires only for `local + 1` where the local is known strictly less than some
    /// i64 (a `WHILE local < S` body): then `local <= S - 1`, so `local + 1 <= S <=
    /// i64::MAX`. Restricted to `+ 1` because a larger constant could carry `S - 1 +
    /// c` past i64::MAX for a symbolic `S`. Default `false` (the check stays).
    fn integer_add_elidable(&self, left: &NirValue, right: &NirValue) -> bool {
        let NirValue::Local(name) = left else {
            return false;
        };
        if !self.integer_strict_upper.contains(name) {
            return false;
        }
        matches!(right, NirValue::Const { type_, value } if matches!(type_, ParameterType::Integer) && value == "1")
    }

    /// plan-39 I1: the lower bound a guard condition establishes on its fall-through
    /// path — i.e. the bound implied by the condition being **false**. `local < K`
    /// false gives `local >= K`; `local <= K` false gives `local >= K+1`. Only a
    /// `Local < Integer-const` / `Local <= Integer-const` shape yields a bound.
    /// The caller must apply it only when the then-branch is terminal and the else
    /// is empty (so the fall-through is exactly the negated condition).
    pub(crate) fn guard_lower_bound(&self, condition: &NirValue) -> Option<(String, i64)> {
        let NirValue::Binary {
            op, left, right, ..
        } = condition
        else {
            return None;
        };
        let (name, konst) = match (left.as_ref(), right.as_ref()) {
            (NirValue::Local(n), NirValue::Const { type_, value })
                if matches!(type_, ParameterType::Integer) =>
            {
                (n.clone(), value.parse::<i64>().ok()?)
            }
            _ => return None,
        };
        match op {
            BinaryOp::Less => Some((name, konst)),
            BinaryOp::LessEqual => Some((name, konst.checked_add(1)?)),
            // Only a strict/non-strict upper-bound guard yields a bound; no
            // other comparison (and no arithmetic operator) constrains the
            // local from above.
            BinaryOp::Or
            | BinaryOp::Xor
            | BinaryOp::And
            | BinaryOp::Equal
            | BinaryOp::NotEqual
            | BinaryOp::Greater
            | BinaryOp::GreaterEqual
            | BinaryOp::Concat
            | BinaryOp::Add
            | BinaryOp::Subtract
            | BinaryOp::Multiply
            | BinaryOp::Divide
            | BinaryOp::Mod
            | BinaryOp::IntDiv
            | BinaryOp::Power => None,
        }
    }
}
