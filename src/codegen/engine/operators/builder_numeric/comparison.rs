//! plan-176-U: the comparison operator lowerings (`lower_comparison_binary`,
//! `lower_numeric_comparison_binary`). The module doc lives in
//! `engine/operators/builder_numeric/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    pub(crate) fn lower_comparison_binary(
        &mut self,
        op: BinaryOp,
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
        if left.type_ == ParameterType::String {
            let right = self.lower_value(right)?;
            if right.type_ != ParameterType::String {
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
            let left = ValueResult {
                origin: None,
                type_: left.type_,
                location: Operand::from(left_register),
                text: left.text,
            };
            let right = ValueResult {
                origin: None,
                type_: right.type_,
                location: Operand::from(right_register),
                text: right.text,
            };
            return self.lower_string_comparison_binary(op, &left, &right);
        }
        if matches!(
            left.type_.name().as_ref(),
            "Byte" | "Integer" | "Fixed" | "Float" | "Money"
        ) {
            let right = self.lower_value(right)?;
            let right = self.materialize_float(right)?;
            let right_slot = self.allocate_stack_object("cmp_right", 8);
            self.emit(abi::store_u64(
                &right.location,
                abi::stack_pointer(),
                right_slot,
            ));
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
            let left = ValueResult {
                origin: None,
                type_: left.type_,
                location: Operand::from(left_register),
                text: left.text,
            };
            let right = ValueResult {
                origin: None,
                type_: right.type_,
                location: Operand::from(right_register),
                text: right.text,
            };
            return self.lower_numeric_comparison_binary(op, &left, &right);
        }
        if self.type_model.record_fields.contains_key(&left.type_) {
            if !matches!(op, BinaryOp::Equal | BinaryOp::NotEqual) {
                return Err(format!(
                    "native code does not lower record comparison operator '{}'",
                    op.name()
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
            let left_register = self.allocate_register();
            let right_register = self.allocate_register();
            let result = self.allocate_register();
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
                if op == BinaryOp::Equal {
                    "true"
                } else {
                    "false"
                },
            ));
            self.emit(abi::branch(&done_label));
            self.emit(abi::label(&not_equal_label));
            self.emit(abi::move_immediate(
                &result,
                "Boolean",
                if op == BinaryOp::Equal {
                    "false"
                } else {
                    "true"
                },
            ));
            self.emit(abi::label(&done_label));
            return Ok(ValueResult {
                origin: None,
                type_: ParameterType::Boolean,
                location: Operand::from(result),
                text: format!("({} {} {})", left.text, op.name(), right.text),
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
        let left_register = self.allocate_register();
        let right_register = self.allocate_register();
        let result = self.allocate_register();
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
            BinaryOp::Equal => self.emit(abi::branch_eq(&true_label)),
            BinaryOp::NotEqual => self.emit(abi::branch_ne(&true_label)),
            BinaryOp::Less => self.emit(abi::branch_lt(&true_label)),
            BinaryOp::Greater => self.emit(abi::branch_gt(&true_label)),
            BinaryOp::LessEqual => self.emit(abi::branch_le(&true_label)),
            BinaryOp::GreaterEqual => self.emit(abi::branch_ge(&true_label)),
            // Only the six comparisons produce a Boolean; `ir::verify` rejects
            // every other operator on this path before lowering.
            BinaryOp::Or
            | BinaryOp::Xor
            | BinaryOp::And
            | BinaryOp::Concat
            | BinaryOp::Add
            | BinaryOp::Subtract
            | BinaryOp::Multiply
            | BinaryOp::Divide
            | BinaryOp::Mod
            | BinaryOp::IntDiv
            | BinaryOp::Power => {
                return Err(format!(
                    "native code plan does not lower comparison operator '{}'",
                    op.name()
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
            text: format!("({} {} {})", left.text, op.name(), right.text),
        })
    }

    fn lower_numeric_comparison_binary(
        &mut self,
        op: BinaryOp,
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
        let left = ValueResult {
            origin: None,
            type_: left.type_.clone(),
            location: Operand::from(left_register),
            text: left.text.clone(),
        };
        let right = ValueResult {
            origin: None,
            type_: right.type_.clone(),
            location: Operand::from(right_register),
            text: right.text.clone(),
        };

        // Comparisons promote exactly as arithmetic does (Fixed > Float > Integer
        // > Byte, §4.1): a Float/Fixed pair compares as Fixed, so the Float
        // operand takes the checked, rounding Float→Fixed conversion and can
        // raise ErrOverflow/ErrInvalidFormat like `fixed + float` (bug-816).
        let promoted = promoted_binary_type(BinaryOp::Add, &left.type_, &right.type_);
        let result = self.allocate_register();
        let true_label = self.label("cmp_true");
        let done_label = self.label("cmp_done");

        match &promoted {
            // Money compares as a signed i64 raw (same scale ⇒ raw order = value
            // order); no promotion, no float path (plan-29-C §4.3).
            ParameterType::Byte | ParameterType::Integer | ParameterType::Money => {
                self.emit(abi::compare_registers(&left.location, &right.location));
            }
            ParameterType::Fixed => {
                let left_fixed = self.allocate_register();
                let right_fixed = self.allocate_register();
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
            ParameterType::Float => {
                self.load_numeric_as_double(abi::FP_SCRATCH[0], &left)?;
                self.load_numeric_as_double(abi::FP_SCRATCH[1], &right)?;
                self.emit(abi::float_compare_d(abi::FP_SCRATCH[0], abi::FP_SCRATCH[1]));
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
        let is_float = promoted == ParameterType::Float;
        match op {
            BinaryOp::Equal => self.emit(abi::branch_eq(&true_label)),
            BinaryOp::NotEqual => self.emit(abi::branch_ne(&true_label)),
            BinaryOp::Less if is_float => self.emit(abi::branch_mi(&true_label)),
            BinaryOp::Less => self.emit(abi::branch_lt(&true_label)),
            BinaryOp::Greater => self.emit(abi::branch_gt(&true_label)),
            BinaryOp::LessEqual if is_float => self.emit(abi::branch_ls(&true_label)),
            BinaryOp::LessEqual => self.emit(abi::branch_le(&true_label)),
            BinaryOp::GreaterEqual => self.emit(abi::branch_ge(&true_label)),
            // Only the six comparisons produce a Boolean; `ir::verify` rejects
            // every other operator on this path before lowering.
            BinaryOp::Or
            | BinaryOp::Xor
            | BinaryOp::And
            | BinaryOp::Concat
            | BinaryOp::Add
            | BinaryOp::Subtract
            | BinaryOp::Multiply
            | BinaryOp::Divide
            | BinaryOp::Mod
            | BinaryOp::IntDiv
            | BinaryOp::Power => {
                return Err(format!(
                    "native code plan does not lower comparison operator '{}'",
                    op.name()
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
            text: format!("({} {} {})", left.text, op.name(), right.text),
        })
    }
}
