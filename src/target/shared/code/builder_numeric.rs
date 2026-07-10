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
        if self.dnative_floats()
            && left.type_ == "Float"
            && matches!(op, "+" | "-" | "*" | "/" | "DIV")
        {
            let result_is_float = self
                .static_type_name(right)
                .as_deref()
                .map(|right_type| numeric_binary_result_type(op, "Float", right_type) == "Float")
                .unwrap_or(false);
            if result_is_float {
                return self.lower_float_arithmetic_dnative(op, left, right);
            }
        }
        // Carry FP-residency (plan-03 Stage C) across the operand slot
        // round-trip: the reloaded operand register holds the same value, so it
        // is resident in the same `d`-register.
        let left_resident = self.float_residents.get(&left.location).cloned();
        let left_slot = self.allocate_stack_object("arith_left", 8);
        // A `d`-native float operand (possible here only as the right operand of
        // an `Integer op Float`) materializes its bits into a GPR before the
        // integer-style slot spill (plan-01 float-dnative §4.1). For every
        // GP-native value this is the identity, so the bump oracle is unchanged.
        let left_spill = self.float_value_as_gpr(&left)?;
        self.emit(abi::store_u64(&left_spill, abi::stack_pointer(), left_slot));
        let right = self.lower_value(right)?;
        let right_resident = self.float_residents.get(&right.location).cloned();
        let right_slot = self.allocate_stack_object("arith_right", 8);
        let right_spill = self.float_value_as_gpr(&right)?;
        self.emit(abi::store_u64(
            &right_spill,
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
        if let Some(fp) = left_resident {
            self.float_residents.insert(left_register.clone(), fp);
        }
        if let Some(fp) = right_resident {
            self.float_residents.insert(right_register.clone(), fp);
        }
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
        // The result location is `register` for every type except a `d`-native
        // float op, which keeps its result in an FP register (returned by
        // `emit_float_binary`). This path is reached for `Float` only in the
        // `Integer op Float` case (a float left operand takes the fast path
        // above); the integer/fixed cases always land in `register`.
        let mut result_location = register.clone();
        match result_type.as_str() {
            "Byte" | "Integer" => {
                self.emit_integer_binary(op, &left, &right, &register, result_type == "Byte")?;
            }
            "Fixed" => {
                if left.type_ == "Fixed" && right.type_ == "Fixed" {
                    self.emit_fixed_binary(op, &left, &right, &register)?;
                } else {
                    // Float-to-Fixed conversion uses x8-x12 as internal scratch registers.
                    // Keep promoted Fixed operands above that range so conversion cannot clobber them.
                    while self.next_register <= 12 {
                        let _ = self.allocate_register()?;
                    }
                    let left_fixed = self.allocate_register()?;
                    let right_fixed = self.allocate_register()?;
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
                        type_: "Fixed".to_string(),
                        location: left_fixed,
                        text: left.text.clone(),
                    };
                    let right = ValueResult {
                        type_: "Fixed".to_string(),
                        location: right_fixed,
                        text: right.text.clone(),
                    };
                    self.emit_fixed_binary(op, &left, &right, &register)?;
                }
            }
            "Float" => {
                result_location = self.emit_float_binary(op, &left, &right, &register)?;
            }
            other => {
                return Err(format!(
                    "native code plan cannot lower arithmetic result type '{other}'"
                ));
            }
        }
        Ok(ValueResult {
            type_: result_type,
            location: result_location,
            text: format!("({} {op} {})", left.text, right.text),
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
        op: &str,
        left: ValueResult,
        right: &NirValue,
    ) -> Result<ValueResult, String> {
        let left_text = left.text.clone();
        let right = self.lower_value(right)?;
        let result_type = numeric_binary_result_type(op, &left.type_, &right.type_).to_string();
        let right_text = right.text.clone();
        // `dst` is used only by the GP-result operators (`MOD`/`^`); the pure-FP
        // operators ignore it and return their FP register.
        let dst = self.allocate_register()?;
        let location = self.emit_float_binary(op, &left, &right, &dst)?;
        Ok(ValueResult {
            type_: result_type,
            location,
            text: format!(
                "({op_left} {op} {op_right})",
                op_left = left_text,
                op_right = right_text
            ),
        })
    }

    pub(super) fn lower_numeric_unary_negation(
        &mut self,
        operand: ValueResult,
    ) -> Result<ValueResult, String> {
        let register = self.allocate_register()?;
        let mut location = register.clone();
        match operand.type_.as_str() {
            "Byte" => {
                let ok = self.label("byte_unary_ok");
                self.emit(abi::compare_immediate(&operand.location, "0"));
                self.emit(abi::branch_eq(&ok));
                self.emit_underflow_return()?;
                self.emit(abi::label(&ok));
                self.emit(abi::move_register(&register, &operand.location));
            }
            "Integer" => {
                self.emit_min_i64_negation_check(&operand.location, "integer_unary")?;
                let zero = self.allocate_register()?;
                self.emit(abi::move_immediate(&zero, "Integer", "0"));
                self.emit(abi::subtract_registers(&register, &zero, &operand.location));
            }
            "Fixed" => {
                self.emit_min_i64_negation_check(&operand.location, "fixed_unary")?;
                let zero = self.allocate_register()?;
                self.emit(abi::move_immediate(&zero, "Integer", "0"));
                self.emit(abi::subtract_registers(&register, &zero, &operand.location));
            }
            "Float" if self.dnative_floats() => {
                // Negation just flips the sign bit, so a finite operand stays
                // finite — and every live MFBASIC Float is finite (inf/NaN are
                // always errors, never values). No overflow/NaN check is needed.
                // `d`-native: negate in the FP domain and keep the result in its
                // FP register, so the value never round-trips through a GPR
                // (plan-01 float-dnative).
                let d_operand = self.operand_as_double(&operand)?;
                let d_res = self.allocate_fp_register()?;
                self.emit(abi::float_negate_d(&d_res, &d_operand));
                location = d_res;
            }
            "Float" => {
                // GP-native (bump oracle): flip the sign in `d0` and shuttle the
                // bits back to a GPR. (The old emit_float_result_check here also
                // hardcoded `x17` as scratch, which corrupted the result when the
                // allocator handed out x16/x17 — e.g. two inline `-literal` call
                // arguments.)
                self.emit(abi::float_move_d_from_x("d0", &operand.location));
                self.emit(abi::float_negate_d("d0", "d0"));
                self.emit(abi::float_move_x_from_d(&register, "d0"));
            }
            other => {
                return Err(format!(
                    "native code plan does not lower unary operator '-' for {other}"
                ));
            }
        }
        Ok(ValueResult {
            type_: operand.type_,
            location,
            text: format!("(-{})", operand.text),
        })
    }

    pub(super) fn lower_comparison_binary(
        &mut self,
        op: &str,
        left: &NirValue,
        right: &NirValue,
    ) -> Result<ValueResult, String> {
        let left = self.lower_value(left)?;
        // The comparison machinery spills each operand through an integer slot
        // (`str x`/`ldr x`) before loading it as a double for `fcmp`, so a
        // `d`-native float is materialized into a GPR first (plan-01
        // float-dnative). Identity for every GP-native value.
        let left = self.materialize_float(left)?;
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
            let right = self.materialize_float(right)?;
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
        if self.type_model.record_fields.contains_key(&left.type_) {
            if !matches!(op, "=" | "<>") {
                return Err(format!(
                    "native code does not lower record comparison operator '{op}'"
                ));
            }
            let right = self.lower_value(right)?;
            if right.type_ != left.type_ {
                return Err(format!(
                    "native code comparison requires matching record operands, got {} and {}",
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
            let result = self.allocate_register()?;
            let equal_label = self.label("cmp_record_equal");
            let not_equal_label = self.label("cmp_record_not_equal");
            let done_label = self.label("cmp_record_done");
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
            self.emit_comparable_values_match_branch(
                &left.type_,
                &left_register,
                &right_register,
                &equal_label,
                &not_equal_label,
            )?;
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
            return Ok(ValueResult {
                type_: "Boolean".to_string(),
                location: result,
                text: format!("({} {op} {})", left.text, right.text),
            });
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
        self.emit(abi::store_u64(
            &left.location,
            abi::stack_pointer(),
            left_slot,
        ));
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

        // Float comparisons follow IEEE 754 for non-finite operands (plan-17):
        // any relation involving `NaN` is false (and `<>` is true). `fcmp` leaves
        // an unordered result with N clear, C set, Z clear, V set, so `=`/`<>`
        // (EQ/NE) and `>`/`>=` (GT/GE) already fall to the correct side; only `<`
        // and `<=` need the FP conditions `MI`/`LS` rather than the signed
        // `LT`/`LE`, which would wrongly take an unordered NaN as the true side.
        let is_float = promoted == "Float";
        match op {
            "=" => self.emit(abi::branch_eq(&true_label)),
            "<>" => self.emit(abi::branch_ne(&true_label)),
            "<" if is_float => self.emit(abi::branch_mi(&true_label)),
            "<" => self.emit(abi::branch_lt(&true_label)),
            ">" => self.emit(abi::branch_gt(&true_label)),
            "<=" if is_float => self.emit(abi::branch_ls(&true_label)),
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
            "<" | ">" | "<=" | ">=" => {
                return self.lower_string_ordering_binary(op, left, right);
            }
            other => {
                return Err(format!(
                    "native code does not lower string comparison operator '{other}'"
                ));
            }
        }

        let left_len = self.temporary_vreg();
        let right_len = self.temporary_vreg();
        let left_ptr = self.temporary_vreg();
        let right_ptr = self.temporary_vreg();
        let left_byte = self.temporary_vreg();
        let right_byte = self.temporary_vreg();
        let result = self.allocate_register()?;
        let loop_label = self.label("cmp_string_loop");
        let equal_label = self.label("cmp_string_equal");
        let not_equal_label = self.label("cmp_string_not_equal");
        let done_label = self.label("cmp_string_done");

        self.emit(abi::load_u64(&left_len, &left.location, 0));
        self.emit(abi::load_u64(&right_len, &right.location, 0));
        self.emit(abi::compare_registers(&left_len, &right_len));
        self.emit(abi::branch_ne(&not_equal_label));
        self.emit(abi::add_immediate(&left_ptr, &left.location, 8));
        self.emit(abi::add_immediate(&right_ptr, &right.location, 8));
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate(&left_len, "0"));
        self.emit(abi::branch_eq(&equal_label));
        self.emit(abi::load_u8(&left_byte, &left_ptr, 0));
        self.emit(abi::load_u8(&right_byte, &right_ptr, 0));
        self.emit(abi::compare_registers(&left_byte, &right_byte));
        self.emit(abi::branch_ne(&not_equal_label));
        self.emit(abi::add_immediate(&left_ptr, &left_ptr, 1));
        self.emit(abi::add_immediate(&right_ptr, &right_ptr, 1));
        self.emit(abi::subtract_immediate(&left_len, &left_len, 1));
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

    /// Lowers `<`, `>`, `<=`, `>=` for two `String` operands. The order is
    /// lexicographic by Unicode scalar value (§2.2): UTF-8 byte-wise comparison
    /// is identical to code-point order, so we compare the bytes of the common
    /// prefix and, if equal, the shorter string compares less. Bytes and lengths
    /// are compared unsigned. The result is target independent.
    fn lower_string_ordering_binary(
        &mut self,
        op: &str,
        left: &ValueResult,
        right: &ValueResult,
    ) -> Result<ValueResult, String> {
        // Boolean outcome for each of the three orderings, per operator.
        let less_value = if matches!(op, "<" | "<=") {
            "true"
        } else {
            "false"
        };
        let greater_value = if matches!(op, ">" | ">=") {
            "true"
        } else {
            "false"
        };
        let equal_value = if matches!(op, "<=" | ">=") {
            "true"
        } else {
            "false"
        };

        let left_len = self.temporary_vreg();
        let right_len = self.temporary_vreg();
        let min_len = self.temporary_vreg();
        let left_ptr = self.temporary_vreg();
        let right_ptr = self.temporary_vreg();
        let left_byte = self.temporary_vreg();
        let result = self.allocate_register()?;
        let min_done_label = self.label("cmp_string_ord_min");
        let loop_label = self.label("cmp_string_ord_loop");
        let prefix_label = self.label("cmp_string_ord_prefix");
        let less_label = self.label("cmp_string_ord_less");
        let greater_label = self.label("cmp_string_ord_greater");
        let done_label = self.label("cmp_string_ord_done");

        // left_len = len(left), right_len = len(right); min_len = min of the two.
        self.emit(abi::load_u64(&left_len, &left.location, 0));
        self.emit(abi::load_u64(&right_len, &right.location, 0));
        self.emit(abi::move_register(&min_len, &left_len));
        self.emit(abi::compare_registers(&left_len, &right_len));
        self.emit(abi::branch_lo(&min_done_label));
        self.emit(abi::move_register(&min_len, &right_len));
        self.emit(abi::label(&min_done_label));

        // left_ptr/right_ptr point at the first data byte of each string.
        self.emit(abi::add_immediate(&left_ptr, &left.location, 8));
        self.emit(abi::add_immediate(&right_ptr, &right.location, 8));

        // Compare the common prefix byte by byte (right_len is free to reuse as a temp).
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate(&min_len, "0"));
        self.emit(abi::branch_eq(&prefix_label));
        self.emit(abi::load_u8(&left_byte, &left_ptr, 0));
        self.emit(abi::load_u8(&right_len, &right_ptr, 0));
        self.emit(abi::compare_registers(&left_byte, &right_len));
        self.emit(abi::branch_lo(&less_label));
        self.emit(abi::branch_hi(&greater_label));
        self.emit(abi::add_immediate(&left_ptr, &left_ptr, 1));
        self.emit(abi::add_immediate(&right_ptr, &right_ptr, 1));
        self.emit(abi::subtract_immediate(&min_len, &min_len, 1));
        self.emit(abi::branch(&loop_label));

        // Common prefix equal: the shorter string compares less.
        self.emit(abi::label(&prefix_label));
        self.emit(abi::load_u64(&left_len, &left.location, 0));
        self.emit(abi::load_u64(&right_len, &right.location, 0));
        self.emit(abi::compare_registers(&left_len, &right_len));
        self.emit(abi::branch_lo(&less_label));
        self.emit(abi::branch_hi(&greater_label));
        // Equal strings.
        self.emit(abi::move_immediate(&result, "Boolean", equal_value));
        self.emit(abi::branch(&done_label));

        self.emit(abi::label(&less_label));
        self.emit(abi::move_immediate(&result, "Boolean", less_value));
        self.emit(abi::branch(&done_label));

        self.emit(abi::label(&greater_label));
        self.emit(abi::move_immediate(&result, "Boolean", greater_value));
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
            "^" => self.emit_fixed_pow(dst, &left.location, &right.location)?,
            other => {
                return Err(format!(
                    "native code plan does not lower Fixed operator '{other}'"
                ));
            }
        }
        Ok(())
    }

    /// Lower a `Float` arithmetic operator and return the **location of the
    /// result** (plan-01 float-dnative). Under the `d`-native model the result
    /// stays in its FP virtual register (`%fN`) and that register is returned, so
    /// no `fmov`-to-GPR shuttle is emitted; under the bump oracle the result is
    /// moved back to `dst` (a GPR) exactly as before and `dst` is returned. No
    /// finiteness check is emitted here: an anonymous intermediate may be
    /// non-finite and a transient that recovers to finite must not trap — the
    /// check fires only where the value crosses an observation boundary
    /// (plan-17), where `observe_float` reads the returned location.
    pub(super) fn emit_float_binary(
        &mut self,
        op: &str,
        left: &ValueResult,
        right: &ValueResult,
        dst: &str,
    ) -> Result<String, String> {
        match op {
            // The pure-FP arithmetic ops run on FP virtual registers so a chained
            // operand stays resident in a `d`-register instead of round-tripping
            // through a GPR (plan-03 Stage C).
            "+" | "-" | "*" | "/" | "DIV" => {
                let d_left = self.operand_as_double(left)?;
                let d_right = self.operand_as_double(right)?;
                let d_res = self.allocate_fp_register()?;
                match op {
                    "+" => self.emit(abi::float_add_d(&d_res, &d_left, &d_right)),
                    "-" => self.emit(abi::float_subtract_d(&d_res, &d_left, &d_right)),
                    "*" => self.emit(abi::float_multiply_d(&d_res, &d_left, &d_right)),
                    // Division by zero is no longer pre-checked: `x/0` → `±Inf`
                    // and `0/0` → `NaN` flow out as ordinary non-finite results
                    // and trap at the boundary (`ErrFloatOverflow`/`ErrFloatNaN`),
                    // never `ErrFloatDomain` (plan-17 §4.3).
                    _ => self.emit(abi::float_divide_d(&d_res, &d_left, &d_right)),
                }
                // `d`-native model: the FP register *is* the value's home; return
                // it so consumers stay in the FP domain and the GP shuttle is
                // never created. Bump oracle: shuttle to `dst` and return `dst`.
                if self.dnative_floats() {
                    return Ok(d_res);
                }
                self.emit(abi::float_move_x_from_d(dst, &d_res));
                return Ok(dst.to_string());
            }
            "MOD" => {
                self.load_numeric_as_double("d0", left)?;
                self.load_numeric_as_double("d1", right)?;
                // Domain pre-check: b == 0 raises ErrFloatDomain. This stays a
                // genuine pre-check (not a boundary finiteness check) because the
                // in-tree exact `fmod` kernel below requires a non-zero, finite
                // divisor — it does not itself produce the NaN that `a MOD 0`
                // would otherwise yield (plan-17 keeps ErrFloatDomain here).
                self.emit(abi::float_compare_zero_d("d1"));
                let nonzero = self.label("float_mod_divisor_nonzero");
                self.emit(abi::branch_ne(&nonzero));
                self.emit_float_domain_return()?;
                self.emit(abi::label(&nonzero));
                // Move the f64 bit patterns into GPRs and run the in-tree exact
                // fmod kernel (no libm). d0/d1 still hold a/b from above. The
                // result of a finite, non-zero-divisor fmod is always finite, so
                // no per-op finiteness check is needed (plan-17).
                let a_bits = self.allocate_register()?;
                let b_bits = self.allocate_register()?;
                self.emit(abi::float_move_x_from_d(&a_bits, "d0"));
                self.emit(abi::float_move_x_from_d(&b_bits, "d1"));
                let result = self.emit_float_fmod(&a_bits, &b_bits)?;
                self.emit(abi::float_move_d_from_x("d0", &result));
                self.emit(abi::float_move_x_from_d(dst, "d0"));
            }
            "^" => {
                self.load_numeric_as_double("d0", left)?;
                self.load_numeric_as_double("d1", right)?;
                // `emit_float_pow` keeps its domain guards (whole, non-negative
                // exponent) but no longer checks the result: an overflow to `Inf`
                // is an anonymous intermediate that traps only at the boundary
                // (plan-17).
                self.emit_float_pow("d0", "d1")?;
                self.emit(abi::float_move_x_from_d(dst, "d0"));
            }
            other => {
                return Err(format!(
                    "native code plan does not lower Float operator '{other}'"
                ));
            }
        }
        // `MOD`/`^` leave their result in `dst` (a GPR); they are not on a hot
        // path and stay GP-native.
        Ok(dst.to_string())
    }

    /// Whether the `d`-register-native float value model is in effect (plan-01
    /// float-dnative). It is sound only under the liveness-driven allocator (an
    /// FP virtual register held in a `ValueResult` must survive to its consumers;
    /// the bump oracle reuses `d0`–`d7` every statement), so the carrier flip is
    /// gated to `LinearScan` — the bump reference path stays byte-identical.
    pub(super) fn dnative_floats(&self) -> bool {
        self.regalloc_kind == regalloc::RegallocKind::LinearScan
    }

    /// Whether `value` is a `Float` whose canonical home is a `d`-register: its
    /// `location` is an FP virtual register (`%fN`) rather than a GPR/slot holding
    /// the bit pattern (plan-01 float-dnative). Such a value is consumed directly
    /// in the FP domain by float-aware sites (`operand_as_double`,
    /// `load_numeric_as_double`, `fcmp`, the FP-domain finiteness check, a `str d`
    /// store) and materialized into a GPR on demand by [`Self::float_value_as_gpr`]
    /// for every site that needs the raw bits.
    pub(super) fn float_is_dnative(value: &ValueResult) -> bool {
        value.type_ == "Float" && regalloc::parse_fp_vreg(&value.location).is_some()
    }

    /// The single choke point every consumer that needs a `Float`'s **bit
    /// pattern in a GPR** calls (plan-01 float-dnative §4.1). For a GP-native
    /// value it is the identity (returns the existing location); for a
    /// `d`-register-native value it materializes the bits with one `fmov x, d`
    /// into a fresh GPR. Routing all bit consumers through here is what makes the
    /// carrier flip safe — a value that lives only in a `d`-register reaches a GPR
    /// consumer correctly instead of leaking its FP virtual register into a GP
    /// instruction (which would fail to encode rather than silently miscompile).
    pub(super) fn float_value_as_gpr(&mut self, value: &ValueResult) -> Result<String, String> {
        if Self::float_is_dnative(value) {
            let gpr = self.allocate_register()?;
            self.emit(abi::float_move_x_from_d(&gpr, &value.location));
            return Ok(gpr);
        }
        Ok(value.location.clone())
    }

    /// Return `value` with its bits in a GPR (plan-01 float-dnative): a `d`-native
    /// `Float` is `fmov`'d into a fresh GP register and rewrapped as a GP-native
    /// `ValueResult`; every other value (already GP-native) is returned unchanged.
    /// Consumers that hand a value's `location` to a GP instruction — comparisons,
    /// conversions, the math kernels, call/return marshalling — call this so a
    /// `d`-native float never leaks its FP register into a GP-context encoding.
    pub(super) fn materialize_float(&mut self, value: ValueResult) -> Result<ValueResult, String> {
        if Self::float_is_dnative(&value) {
            let gpr = self.allocate_register()?;
            self.emit(abi::float_move_x_from_d(&gpr, &value.location));
            return Ok(ValueResult {
                type_: value.type_,
                location: gpr,
                text: value.text,
            });
        }
        Ok(value)
    }

    /// Store `value` into `[base + offset]`. A `d`-native `Float` is stored
    /// straight from its FP register (`str d`); every other value stores its GPR
    /// (`str x`). The 8 bytes written are identical either way, so a slot written
    /// by `str d` and later read as `ldr x` (copy/transfer/marshalling) is
    /// unchanged — only the in-flight register class differs (plan-01
    /// float-dnative §1 non-goals).
    pub(super) fn store_value_at(&mut self, value: &ValueResult, base: &str, offset: usize) {
        if Self::float_is_dnative(value) {
            self.emit(abi::store_double(&value.location, base, offset));
        } else {
            self.emit(abi::store_u64(&value.location, base, offset));
        }
    }

    /// Materialize `value` into a `d`-register and return it. A `Float` operand
    /// already resident in an FP virtual register (a prior float op's result, or a
    /// `d`-native load) is returned directly with no reload; otherwise a fresh FP
    /// virtual register is loaded (plan-03 Stage C / plan-01 float-dnative).
    pub(super) fn operand_as_double(&mut self, value: &ValueResult) -> Result<String, String> {
        if Self::float_is_dnative(value) {
            return Ok(value.location.clone());
        }
        if value.type_ == "Float" {
            if let Some(resident) = self.float_residents.get(&value.location).cloned() {
                return Ok(resident);
            }
        }
        let dst = self.allocate_fp_register()?;
        self.load_numeric_as_double(&dst, value)?;
        Ok(dst)
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

    fn emit_min_i64_negation_check(
        &mut self,
        value: &str,
        label_prefix: &str,
    ) -> Result<(), String> {
        let min = self.allocate_register()?;
        let ok = self.label(&format!("{label_prefix}_not_min"));
        self.emit(abi::move_immediate(&min, "Integer", "9223372036854775808"));
        self.emit(abi::compare_registers(value, &min));
        self.emit(abi::branch_ne(&ok));
        self.emit_overflow_return()?;
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
        // Bounded-base fast path (bug-61): a base in {-1, 0, 1} has bounded powers,
        // so the checked-multiply loop below never overflows and would iterate the
        // full exponent (up to i64::MAX) — an effective hang. Resolve those bases
        // in closed form. Any |base| >= 2 falls through to the loop, which still
        // terminates within ~63 iterations via the multiply overflow trap.
        let bounded_zero = self.label("pow_bounded_zero");
        self.emit(abi::compare_immediate(base, "1"));
        self.emit(abi::branch_gt(&loop_label)); // base > 1: slow path.
        self.emit(abi::compare_immediate(base, &(-1_i64 as u64).to_string()));
        self.emit(abi::branch_lt(&loop_label)); // base < -1: slow path.
        // base is now one of {-1, 0, 1}; `dst` currently holds 1.
        self.emit(abi::compare_immediate(base, "0"));
        self.emit(abi::branch_eq(&bounded_zero));
        self.emit(abi::branch_gt(&done_label)); // base == 1: 1^n == 1.
        // base == -1: 1 for an even exponent, -1 for an odd exponent.
        let parity = self.allocate_register()?;
        let one_bit = self.allocate_register()?;
        self.emit(abi::move_immediate(&one_bit, "Integer", "1"));
        self.emit(abi::and_registers(&parity, exponent, &one_bit));
        self.emit(abi::compare_immediate(&parity, "0"));
        self.emit(abi::branch_eq(&done_label)); // even exponent: (-1)^n == 1.
        self.emit_neg_i64(dst)?; // odd exponent: (-1)^n == -1.
        self.emit(abi::branch(&done_label));
        self.emit(abi::label(&bounded_zero));
        // base == 0: 0^0 == 1 (dst already 1); 0^n == 0 for n > 0.
        self.emit(abi::compare_immediate(exponent, "0"));
        self.emit(abi::branch_eq(&done_label));
        self.emit(abi::move_immediate(dst, "Integer", "0"));
        self.emit(abi::branch(&done_label));
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
        // The working registers are dead once the product lands in `dst`.
        let saved_registers = self.next_register;
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
        self.next_register = saved_registers;
        Ok(())
    }

    pub(super) fn emit_fixed_divide(
        &mut self,
        dst: &str,
        left: &str,
        right: &str,
    ) -> Result<(), String> {
        // The working registers are dead once the quotient lands in `dst`.
        let saved_registers = self.next_register;
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
        // Admit an integer magnitude up to 2^31 (not 2^31 - 1). The exact result
        // -2147483648.0 (raw i64::MIN) has magnitude integer part 2^31 with a zero
        // fraction; rejecting it here wrongly traps its own representable minimum
        // (bug-61). Anything strictly above 2^31 cannot fit even the negative range
        // and would corrupt the `integer << 32` below, so it still traps. The final
        // signed range check (see below) rejects the sub-cases of 2^31 that are not
        // representable (a positive result, or a negative one with a nonzero
        // fraction).
        self.emit(abi::move_immediate(&max_integer, "Integer", "2147483648"));
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
        // `dst` now holds the unsigned magnitude of the quotient in Q32.32. Apply
        // the sign and range-check the signed result the way `emit_fixed_multiply`
        // does, so the representable minimum -2147483648.0 (raw i64::MIN, magnitude
        // 2^63) is admitted while genuine overflows still trap (bug-61).
        let negative = self.label("fixed_div_negative");
        let negate = self.label("fixed_div_negate");
        let magnitude_overflow = self.label("fixed_div_mag_overflow");
        let quotient_done = self.label("fixed_div_signed");
        self.emit(abi::compare_immediate(&sign, "0"));
        self.emit(abi::branch_lt(&negative));
        // Positive result: the magnitude must fit a signed i64 (top bit clear). A
        // magnitude of 2^63 (exactly +2147483648.0) has the top bit set, so it
        // correctly traps here — only the negative form of that magnitude is
        // representable.
        self.emit(abi::compare_immediate(dst, "0"));
        self.emit(abi::branch_ge(&quotient_done));
        self.emit(abi::branch(&magnitude_overflow));
        self.emit(abi::label(&negative));
        // Negative result = -(magnitude). Representable iff magnitude <= 2^63.
        //  - magnitude < 2^63  (dst >= 0 as signed): negate normally.
        //  - magnitude == 2^63 (dst == i64::MIN): the result is exactly
        //    -2147483648.0, whose raw value is i64::MIN; negating i64::MIN is a
        //    no-op, so keep `dst` unchanged.
        //  - magnitude > 2^63: overflow.
        self.emit(abi::compare_immediate(dst, "0"));
        self.emit(abi::branch_ge(&negate));
        let min_raw = self.allocate_register()?;
        self.emit(abi::move_immediate(
            &min_raw,
            "Integer",
            &(i64::MIN as u64).to_string(),
        ));
        self.emit(abi::compare_registers(dst, &min_raw));
        self.emit(abi::branch_eq(&quotient_done));
        self.emit(abi::label(&magnitude_overflow));
        self.emit_overflow_return()?;
        self.emit(abi::label(&negate));
        self.emit_neg_i64(dst)?;
        self.emit(abi::label(&quotient_done));
        self.next_register = saved_registers;
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
        // Bounded-base fast path (bug-61): |base| == 1.0 has bounded powers, so the
        // loop's only exit (the multiply overflow trap) never fires and it would
        // iterate the full exponent. Resolve ±1.0 in closed form.
        let neg_one_raw = -(one_raw as i64) as u64;
        self.emit(abi::compare_immediate(base, &one_raw.to_string()));
        self.emit(abi::branch_eq(&done_label)); // 1.0^n == 1.0 (dst already 1.0).
        self.emit(abi::compare_immediate(base, &neg_one_raw.to_string()));
        self.emit(abi::branch_ne(&loop_label)); // |base| != 1.0: enter the loop.
        // base == -1.0: 1.0 for an even exponent, -1.0 for an odd exponent.
        let parity = self.allocate_register()?;
        let one_bit = self.allocate_register()?;
        self.emit(abi::move_immediate(&one_bit, "Integer", "1"));
        self.emit(abi::and_registers(&parity, &whole, &one_bit));
        self.emit(abi::compare_immediate(&parity, "0"));
        self.emit(abi::branch_eq(&done_label)); // even exponent: (-1.0)^n == 1.0.
        self.emit_neg_i64(dst)?; // odd exponent: (-1.0)^n == -1.0.
        self.emit(abi::branch(&done_label));
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate(&remaining, "0"));
        self.emit(abi::branch_eq(&done_label));
        // A product that truncates to 0 (any |base| < 1.0, or base == 0.0) stays 0
        // for every remaining multiply, so stop now rather than iterate the whole
        // (possibly enormous) exponent (bug-61). This never changes a result.
        self.emit(abi::compare_immediate(dst, "0"));
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
        self.emit(abi::subtract_registers(register, abi::ZERO, register));
        Ok(())
    }

    pub(super) fn load_numeric_as_double(
        &mut self,
        dst: &str,
        value: &ValueResult,
    ) -> Result<(), String> {
        let fixed_scratch = self.temporary_vreg();
        match value.type_.as_str() {
            // A `d`-native float is already in an FP register: move it `d`-to-`d`
            // (no GPR round-trip). A GP-native float carries its bits in `value
            // .location` and is moved across the `fmov d, x` boundary.
            "Float" if Self::float_is_dnative(value) => {
                self.emit(abi::float_move_d_from_d(dst, &value.location))
            }
            "Float" => self.emit(abi::float_move_d_from_x(dst, &value.location)),
            "Byte" | "Integer" => self.emit(abi::signed_convert_to_float_d(dst, &value.location)),
            "Fixed" => {
                self.emit(abi::signed_convert_to_float_d(dst, &value.location));
                self.emit_f64_const("d7", &fixed_scratch, 4_294_967_296.0);
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
            "Float" => {
                // The Float→Fixed conversion reads the f64 bit pattern, so a
                // `d`-native float is materialized into a GPR first (plan-01).
                let bits = self.float_value_as_gpr(value)?;
                self.emit_float_bits_to_fixed_value(&bits, dst)?
            }
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
        self.emit_float_domain_return()?;
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
        self.emit_float_domain_return()?;
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

    /// `fmod(a, b) = a - n*b`, `n = trunc(a/b)`, computed **exactly** over GPRs
    /// (the IEEE bitwise remainder; musl's 64-bit `fmod`). Returns a register
    /// holding the result's f64 bit pattern. The divisor is guaranteed finite and
    /// non-zero — `Float MOD Float` raises `ErrFloatDomain` for `b == 0` before
    /// calling — and MFBASIC has no inf/NaN Float values, so libm's exception
    /// prologue is omitted. The result is exactly representable (bit-identical to
    /// libm `fmod`), so no accuracy tolerance applies.
    pub(super) fn emit_float_fmod(&mut self, a_loc: &str, b_loc: &str) -> Result<String, String> {
        // Spill the operands and reset the register file: the kernel needs ~12
        // live registers and would otherwise exhaust the file in a busy
        // expression. The caller consumes `result` immediately after the return.
        let a_slot = self.allocate_stack_object("fmod_a", 8);
        let b_slot = self.allocate_stack_object("fmod_b", 8);
        self.emit(abi::store_u64(a_loc, abi::stack_pointer(), a_slot));
        self.emit(abi::store_u64(b_loc, abi::stack_pointer(), b_slot));
        self.reset_temporary_registers();
        let ux = self.allocate_register()?;
        let uy = self.allocate_register()?;
        self.emit(abi::load_u64(&ux, abi::stack_pointer(), a_slot));
        self.emit(abi::load_u64(&uy, abi::stack_pointer(), b_slot));
        let result = self.allocate_register()?;
        // Persistent constants.
        let signmask = self.allocate_register()?;
        let expmask = self.allocate_register()?;
        let mantmask = self.allocate_register()?;
        let implicit = self.allocate_register()?;
        self.emit(abi::move_immediate(
            &signmask,
            "Integer",
            "9223372036854775808",
        )); // 1<<63
        self.emit(abi::move_immediate(&expmask, "Integer", "2047")); // 0x7ff
        self.emit(abi::move_immediate(
            &mantmask,
            "Integer",
            "4503599627370495",
        )); // (1<<52)-1
        self.emit(abi::move_immediate(
            &implicit,
            "Integer",
            "4503599627370496",
        )); // 1<<52
            // sign = ux & SIGN; ex = (ux>>52)&0x7ff; ey = (uy>>52)&0x7ff; uxi = ux.
        let sign = self.allocate_register()?;
        let ex = self.allocate_register()?;
        let ey = self.allocate_register()?;
        let uxi = self.allocate_register()?;
        let i = self.allocate_register()?;
        let shift = self.allocate_register()?;
        self.emit(abi::and_registers(&sign, &ux, &signmask));
        self.emit(abi::shift_right_immediate(&ex, &ux, 52));
        self.emit(abi::and_registers(&ex, &ex, &expmask));
        self.emit(abi::shift_right_immediate(&ey, &uy, 52));
        self.emit(abi::and_registers(&ey, &ey, &expmask));
        self.emit(abi::move_register(&uxi, &ux));

        let end = self.label("fmod_end");
        let return_x = self.label("fmod_return_x");
        let ret_zero = self.label("fmod_ret_zero");

        // |x| <= |y|: compare magnitudes via the sign-stripped (<<1) bit patterns.
        let not_le = self.label("fmod_not_le");
        self.emit(abi::shift_left_immediate(&i, &ux, 1)); // ax2 (reuse i)
        self.emit(abi::shift_left_immediate(&shift, &uy, 1)); // bx2 (reuse shift)
        self.emit(abi::compare_registers(&i, &shift));
        self.emit(abi::branch_hi(&not_le)); // |x| > |y| (unsigned) → reduce
        self.emit(abi::compare_registers(&i, &shift));
        self.emit(abi::branch_ne(&return_x)); // |x| < |y| → result is x
        self.emit(abi::move_register(&result, &sign)); // |x| == |y| → ±0
        self.emit(abi::branch(&end));
        self.emit(abi::label(&return_x));
        self.emit(abi::move_register(&result, &ux));
        self.emit(abi::branch(&end));
        self.emit(abi::label(&not_le));

        // Normalize x: implicit-bit mantissa for normals; shift subnormals up.
        let x_normal = self.label("fmod_x_normal");
        let x_done = self.label("fmod_x_done");
        let x_subloop = self.label("fmod_x_subloop");
        let x_subdone = self.label("fmod_x_subdone");
        self.emit(abi::compare_immediate(&ex, "0"));
        self.emit(abi::branch_ne(&x_normal));
        self.emit(abi::shift_left_immediate(&i, &uxi, 12));
        self.emit(abi::label(&x_subloop));
        self.emit(abi::compare_immediate(&i, "0"));
        self.emit(abi::branch_lt(&x_subdone)); // top bit set → normalized
        self.emit(abi::subtract_immediate(&ex, &ex, 1));
        self.emit(abi::add_registers(&i, &i, &i));
        self.emit(abi::branch(&x_subloop));
        self.emit(abi::label(&x_subdone));
        self.emit(abi::move_immediate(&shift, "Integer", "1"));
        self.emit(abi::subtract_registers(&shift, &shift, &ex)); // 1 - ex
        self.emit(abi::shift_left_variable(&uxi, &uxi, &shift));
        self.emit(abi::branch(&x_done));
        self.emit(abi::label(&x_normal));
        self.emit(abi::and_registers(&uxi, &uxi, &mantmask));
        self.emit(abi::or_registers(&uxi, &uxi, &implicit));
        self.emit(abi::label(&x_done));

        // Normalize y in place (uy becomes its 53-bit mantissa).
        let y_normal = self.label("fmod_y_normal");
        let y_done = self.label("fmod_y_done");
        let y_subloop = self.label("fmod_y_subloop");
        let y_subdone = self.label("fmod_y_subdone");
        self.emit(abi::compare_immediate(&ey, "0"));
        self.emit(abi::branch_ne(&y_normal));
        self.emit(abi::shift_left_immediate(&i, &uy, 12));
        self.emit(abi::label(&y_subloop));
        self.emit(abi::compare_immediate(&i, "0"));
        self.emit(abi::branch_lt(&y_subdone));
        self.emit(abi::subtract_immediate(&ey, &ey, 1));
        self.emit(abi::add_registers(&i, &i, &i));
        self.emit(abi::branch(&y_subloop));
        self.emit(abi::label(&y_subdone));
        self.emit(abi::move_immediate(&shift, "Integer", "1"));
        self.emit(abi::subtract_registers(&shift, &shift, &ey)); // 1 - ey
        self.emit(abi::shift_left_variable(&uy, &uy, &shift));
        self.emit(abi::branch(&y_done));
        self.emit(abi::label(&y_normal));
        self.emit(abi::and_registers(&uy, &uy, &mantmask));
        self.emit(abi::or_registers(&uy, &uy, &implicit));
        self.emit(abi::label(&y_done));

        // Fixed-point remainder: for (; ex>ey; ex--) { i=uxi-uy; if i>=0 { if i==0
        // → ±0; uxi=i } uxi<<=1 }.
        let modloop = self.label("fmod_modloop");
        let modloop_end = self.label("fmod_modloop_end");
        let mod_shift = self.label("fmod_mod_shift");
        self.emit(abi::label(&modloop));
        self.emit(abi::compare_registers(&ex, &ey));
        self.emit(abi::branch_le(&modloop_end));
        self.emit(abi::subtract_registers(&i, &uxi, &uy));
        self.emit(abi::compare_immediate(&i, "0"));
        self.emit(abi::branch_lt(&mod_shift)); // i<0 → keep uxi
        self.emit(abi::branch_eq(&ret_zero)); // i==0 → exact, ±0
        self.emit(abi::move_register(&uxi, &i));
        self.emit(abi::label(&mod_shift));
        self.emit(abi::add_registers(&uxi, &uxi, &uxi));
        self.emit(abi::subtract_immediate(&ex, &ex, 1));
        self.emit(abi::branch(&modloop));
        self.emit(abi::label(&modloop_end));

        // Final step: i=uxi-uy; if i>=0 { if i==0 → ±0; uxi=i }.
        let after_final = self.label("fmod_after_final");
        self.emit(abi::subtract_registers(&i, &uxi, &uy));
        self.emit(abi::compare_immediate(&i, "0"));
        self.emit(abi::branch_lt(&after_final));
        self.emit(abi::branch_eq(&ret_zero));
        self.emit(abi::move_register(&uxi, &i));
        self.emit(abi::label(&after_final));

        // Re-normalize the result mantissa: for (; uxi>>52==0; uxi<<=1, ex--).
        let normloop = self.label("fmod_normloop");
        let normloop_end = self.label("fmod_normloop_end");
        self.emit(abi::label(&normloop));
        self.emit(abi::shift_right_immediate(&i, &uxi, 52));
        self.emit(abi::compare_immediate(&i, "0"));
        self.emit(abi::branch_ne(&normloop_end));
        self.emit(abi::add_registers(&uxi, &uxi, &uxi));
        self.emit(abi::subtract_immediate(&ex, &ex, 1));
        self.emit(abi::branch(&normloop));
        self.emit(abi::label(&normloop_end));

        // Scale back: ex>0 → reattach exponent; ex<=0 → shift into a subnormal.
        let scale_sub = self.label("fmod_scale_sub");
        let scale_done = self.label("fmod_scale_done");
        self.emit(abi::compare_immediate(&ex, "0"));
        self.emit(abi::branch_le(&scale_sub));
        self.emit(abi::subtract_registers(&uxi, &uxi, &implicit)); // drop implicit bit
        self.emit(abi::shift_left_immediate(&shift, &ex, 52)); // ex<<52
        self.emit(abi::or_registers(&uxi, &uxi, &shift));
        self.emit(abi::branch(&scale_done));
        self.emit(abi::label(&scale_sub));
        self.emit(abi::move_immediate(&shift, "Integer", "1"));
        self.emit(abi::subtract_registers(&shift, &shift, &ex)); // 1 - ex
        self.emit(abi::shift_right_variable(&uxi, &uxi, &shift));
        self.emit(abi::label(&scale_done));
        self.emit(abi::or_registers(&uxi, &uxi, &sign)); // restore sign
        self.emit(abi::move_register(&result, &uxi));
        self.emit(abi::branch(&end));

        self.emit(abi::label(&ret_zero));
        self.emit(abi::move_register(&result, &sign)); // ±0
        self.emit(abi::label(&end));
        // Spill the result and reset, so the surrounding expression resumes with a
        // fresh (low-pressure) register file.
        let out_slot = self.allocate_stack_object("fmod_out", 8);
        self.emit(abi::store_u64(&result, abi::stack_pointer(), out_slot));
        self.reset_temporary_registers();
        let out = self.allocate_register()?;
        self.emit(abi::load_u64(&out, abi::stack_pointer(), out_slot));
        Ok(out)
    }
}
