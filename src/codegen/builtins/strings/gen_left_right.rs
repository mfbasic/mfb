//! Shared grapheme-bounded slice for `strings::{left,right}`.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::target::shared::abi;
use crate::target::shared::nir::*;

pub(crate) fn lower_strings_left_right(
    builder: &mut CodeBuilder,
    value: &NirValue,
    count: &NirValue,
    right: bool,
) -> Result<ValueResult, String> {
    let scratch16 = builder.temporary_vreg();
    let scratch10 = builder.temporary_vreg();
    let scratch9 = builder.temporary_vreg();
    let scratch11 = builder.temporary_vreg();
    let scratch17 = builder.temporary_vreg();
    let scratch12 = builder.temporary_vreg();
    let scratch14 = builder.temporary_vreg();
    let scratch15 = builder.temporary_vreg();
    let scratch13 = builder.temporary_vreg();
    let value = builder.lower_value(value)?;
    builder.require_string("strings.left/right value", &value)?;
    let value_slot = builder.spill_to_slot("strings_lr_value", &value.location);
    let count = builder.lower_value(count)?;
    if count.type_ != "Integer" {
        return Err(format!(
            "strings.left/right count must be Integer, got {}",
            count.type_
        ));
    }
    let count_slot = builder.spill_to_slot("strings_lr_count", &count.location);
    let ptr_slot = builder.allocate_stack_object("strings_lr_ptr", 8);
    let len_slot = builder.allocate_stack_object("strings_lr_len", 8);

    let invalid = builder.label("strings_lr_invalid");
    let build = builder.label("strings_lr_build");

    builder.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
    builder.emit(abi::load_u64(&scratch10, abi::stack_pointer(), count_slot));
    builder.emit(abi::compare_immediate(&scratch10, "0"));
    builder.emit(abi::branch_lt(&invalid));
    builder.emit(abi::load_u64(&scratch9, &scratch16, 0));
    builder.emit(abi::add_immediate(&scratch11, &scratch16, 8));
    // mask = 192, cont byte test == 128.
    builder.emit(abi::move_immediate(&scratch17, "Integer", "192"));

    if !right {
        // Walk forward `count` scalars from the start, tracking byte cursor.
        let walk = builder.label("strings_left_walk");
        let cont = builder.label("strings_left_cont");
        let cont_done = builder.label("strings_left_cont_done");
        let walk_done = builder.label("strings_left_walk_done");
        // x12 = scalars taken, x14 = byte cursor.
        builder.emit(abi::move_immediate(&scratch12, "Integer", "0"));
        builder.emit(abi::move_immediate(&scratch14, "Integer", "0"));
        builder.emit(abi::label(&walk));
        builder.emit(abi::compare_registers(&scratch12, &scratch10));
        builder.emit(abi::branch_ge(&walk_done));
        builder.emit(abi::compare_registers(&scratch14, &scratch9));
        builder.emit(abi::branch_ge(&walk_done));
        // advance one byte (lead), then skip continuation bytes.
        builder.emit(abi::add_immediate(&scratch14, &scratch14, 1));
        builder.emit(abi::label(&cont));
        builder.emit(abi::compare_registers(&scratch14, &scratch9));
        builder.emit(abi::branch_ge(&cont_done));
        builder.emit(abi::add_registers(&scratch15, &scratch11, &scratch14));
        builder.emit(abi::load_u8(&scratch13, &scratch15, 0));
        builder.emit(abi::and_registers(&scratch13, &scratch13, &scratch17));
        builder.emit(abi::compare_immediate(&scratch13, "128"));
        builder.emit(abi::branch_ne(&cont_done));
        builder.emit(abi::add_immediate(&scratch14, &scratch14, 1));
        builder.emit(abi::branch(&cont));
        builder.emit(abi::label(&cont_done));
        builder.emit(abi::add_immediate(&scratch12, &scratch12, 1));
        builder.emit(abi::branch(&walk));
        builder.emit(abi::label(&walk_done));
        // ptr = value+8, len = byte cursor.
        builder.emit(abi::store_u64(&scratch11, abi::stack_pointer(), ptr_slot));
        builder.emit(abi::store_u64(&scratch14, abi::stack_pointer(), len_slot));
    } else {
        // Walk backward `count` scalars from the end (count non-continuation
        // bytes scanning from the end).
        let walk = builder.label("strings_right_walk");
        let walk_done = builder.label("strings_right_walk_done");
        let skip = builder.label("strings_right_skip");
        let counted = builder.label("strings_right_counted");
        // x12 = scalars taken, x14 = byte cursor (one-past current), start at len.
        builder.emit(abi::move_immediate(&scratch12, "Integer", "0"));
        builder.emit(abi::move_register(&scratch14, &scratch9));
        builder.emit(abi::label(&walk));
        builder.emit(abi::compare_registers(&scratch12, &scratch10));
        builder.emit(abi::branch_ge(&walk_done));
        builder.emit(abi::compare_immediate(&scratch14, "0"));
        builder.emit(abi::branch_eq(&walk_done));
        // step back over the scalar: at least one byte, plus any continuation bytes.
        builder.emit(abi::label(&skip));
        builder.emit(abi::subtract_immediate(&scratch14, &scratch14, 1));
        // at index 0 we are necessarily at a scalar boundary.
        builder.emit(abi::compare_immediate(&scratch14, "0"));
        builder.emit(abi::branch_eq(&counted));
        builder.emit(abi::add_registers(&scratch15, &scratch11, &scratch14));
        builder.emit(abi::load_u8(&scratch13, &scratch15, 0));
        builder.emit(abi::and_registers(&scratch13, &scratch13, &scratch17));
        builder.emit(abi::compare_immediate(&scratch13, "128"));
        builder.emit(abi::branch_eq(&skip));
        builder.emit(abi::label(&counted));
        builder.emit(abi::add_immediate(&scratch12, &scratch12, 1));
        builder.emit(abi::branch(&walk));
        builder.emit(abi::label(&walk_done));
        // ptr = value+8+cursor, len = valueLen - cursor.
        builder.emit(abi::add_registers(&scratch13, &scratch11, &scratch14));
        builder.emit(abi::subtract_registers(&scratch12, &scratch9, &scratch14));
        builder.emit(abi::store_u64(&scratch13, abi::stack_pointer(), ptr_slot));
        builder.emit(abi::store_u64(&scratch12, abi::stack_pointer(), len_slot));
    }

    builder.emit(abi::branch(&build));
    builder.emit(abi::label(&invalid));
    builder.raise_error_bare("ErrInvalidArgument")?;
    builder.emit(abi::label(&build));
    builder.emit(abi::load_u64(&scratch13, abi::stack_pointer(), ptr_slot));
    builder.emit(abi::load_u64(&scratch12, abi::stack_pointer(), len_slot));
    let result = builder.emit_materialize_string_from_bytes(&scratch13, &scratch12)?;
    let label = if right {
        "strings.right"
    } else {
        "strings.left"
    };
    Ok(ValueResult {
        type_: "String".to_string(),
        location: Operand::from(result.render()),
        text: label.to_string(),
    })
}
