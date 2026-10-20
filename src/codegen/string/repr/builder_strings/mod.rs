// --- codegen tier imports (migration) ---
use crate::codegen::collection::assign::string_self_update::StringOut;
use crate::codegen::collection::layout::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::typed_list_element_type;
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use crate::codegen::string::format::float_format_sci::FLOAT_TO_STRING_SCI_SYMBOL;
use crate::codegen::string::format::*;
use crate::operators::BinaryOp;
use crate::target::shared::abi;
use crate::target::shared::nir::*;
use crate::types::ParameterType;

mod numeric_text;
mod replace;
mod to_string;
impl CodeBuilder<'_> {
    pub(crate) fn lower_string_comparison_binary(
        &mut self,
        op: BinaryOp,
        left: &ValueResult,
        right: &ValueResult,
    ) -> Result<ValueResult, String> {
        match op {
            BinaryOp::Equal | BinaryOp::NotEqual => {}
            BinaryOp::Less | BinaryOp::Greater | BinaryOp::LessEqual | BinaryOp::GreaterEqual => {
                return self.lower_string_ordering_binary(op, left, right);
            }
            // `&` is lowered by the concat path and every other operator is
            // rejected on String operands by `ir::verify` before lowering.
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
                    "native code does not lower string comparison operator '{}'",
                    op.name()
                ));
            }
        }

        let left_len = self.temporary_vreg();
        let right_len = self.temporary_vreg();
        let left_ptr = self.temporary_vreg();
        let right_ptr = self.temporary_vreg();
        let left_byte = self.temporary_vreg();
        let right_byte = self.temporary_vreg();
        let result = self.allocate_register();
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

        Ok(ValueResult {
            origin: None,
            type_: ParameterType::Boolean,
            location: Operand::from(result),
            text: format!("({} {} {})", left.text, op.name(), right.text),
        })
    }

    /// Lowers `<`, `>`, `<=`, `>=` for two `String` operands. The order is
    /// lexicographic by Unicode scalar value (§2.2): UTF-8 byte-wise comparison
    /// is identical to code-point order, so we compare the bytes of the common
    /// prefix and, if equal, the shorter string compares less. Bytes and lengths
    /// are compared unsigned. The result is target independent.
    pub(crate) fn lower_string_ordering_binary(
        &mut self,
        op: BinaryOp,
        left: &ValueResult,
        right: &ValueResult,
    ) -> Result<ValueResult, String> {
        // Boolean outcome for each of the three orderings, per operator.
        let less_value = if matches!(op, BinaryOp::Less | BinaryOp::LessEqual) {
            "true"
        } else {
            "false"
        };
        let greater_value = if matches!(op, BinaryOp::Greater | BinaryOp::GreaterEqual) {
            "true"
        } else {
            "false"
        };
        let equal_value = if matches!(op, BinaryOp::LessEqual | BinaryOp::GreaterEqual) {
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
        let result = self.allocate_register();
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
            origin: None,
            type_: ParameterType::Boolean,
            location: Operand::from(result),
            text: format!("({} {} {})", left.text, op.name(), right.text),
        })
    }
}
