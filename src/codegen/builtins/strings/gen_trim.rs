//! Shared whitespace trim for `strings::{trim,trimStart,trimEnd}`.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::target::shared::abi;
use crate::target::shared::nir::*;

pub(crate) fn lower_strings_trim(
    builder: &mut CodeBuilder,
    value: &NirValue,
    trim_start: bool,
    trim_end: bool,
) -> Result<ValueResult, String> {
    let scratch16 = builder.temporary_vreg();
    let scratch9 = builder.temporary_vreg();
    let scratch10 = builder.temporary_vreg();
    let scratch11 = builder.temporary_vreg();
    let scratch12 = builder.temporary_vreg();
    let scratch13 = builder.temporary_vreg();
    let scratch14 = builder.temporary_vreg();
    let scratch15 = builder.temporary_vreg();
    let value = builder.lower_value(value)?;
    builder.require_string("strings.trim value", &value)?;
    let value_slot = builder.spill_to_slot("strings_trim_value", &value.location);
    let start_slot = builder.allocate_stack_object("strings_trim_start", 8);
    let end_slot = builder.allocate_stack_object("strings_trim_end", 8);
    let done_start = builder.label("strings_trim_start_done");

    builder.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
    builder.emit(abi::load_u64(&scratch9, &scratch16, 0));
    builder.emit(abi::move_immediate(&scratch10, "Integer", "0"));
    builder.emit(abi::store_u64(&scratch10, abi::stack_pointer(), start_slot));
    builder.emit(abi::store_u64(&scratch9, abi::stack_pointer(), end_slot));

    if trim_start {
        let loop_label = builder.label("strings_trim_start_loop");
        let ws_label = builder.label("strings_trim_start_ws");
        builder.emit(abi::add_immediate(&scratch11, &scratch16, 8));
        builder.emit(abi::move_register(&scratch12, &scratch9));
        builder.emit(abi::label(&loop_label));
        builder.emit(abi::compare_immediate(&scratch12, "0"));
        builder.emit(abi::branch_eq(&done_start));
        builder.emit_unicode_whitespace_branch(
            &scratch11,
            &scratch12,
            &scratch13,
            &ws_label,
            &done_start,
        );
        builder.emit(abi::label(&ws_label));
        builder.emit(abi::load_u64(&scratch14, abi::stack_pointer(), start_slot));
        builder.emit(abi::add_registers(&scratch14, &scratch14, &scratch13));
        builder.emit(abi::store_u64(&scratch14, abi::stack_pointer(), start_slot));
        builder.emit(abi::add_registers(&scratch11, &scratch11, &scratch13));
        builder.emit(abi::subtract_registers(&scratch12, &scratch12, &scratch13));
        builder.emit(abi::branch(&loop_label));
    }
    builder.emit(abi::label(&done_start));

    if trim_end {
        let loop_label = builder.label("strings_trim_end_loop");
        let ws_label = builder.label("strings_trim_end_ws");
        let not_ws_label = builder.label("strings_trim_end_not_ws");
        let done_label = builder.label("strings_trim_end_done");
        builder.emit(abi::load_u64(&scratch14, abi::stack_pointer(), start_slot));
        builder.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
        builder.emit(abi::load_u64(&scratch9, &scratch16, 0));
        builder.emit(abi::add_immediate(&scratch11, &scratch16, 8));
        builder.emit(abi::add_registers(&scratch11, &scratch11, &scratch14));
        builder.emit(abi::subtract_registers(&scratch12, &scratch9, &scratch14));
        builder.emit(abi::move_register(&scratch15, &scratch14));
        builder.emit(abi::store_u64(&scratch14, abi::stack_pointer(), end_slot));
        builder.emit(abi::label(&loop_label));
        builder.emit(abi::compare_immediate(&scratch12, "0"));
        builder.emit(abi::branch_eq(&done_label));
        builder.emit_unicode_whitespace_branch(
            &scratch11,
            &scratch12,
            &scratch13,
            &ws_label,
            &not_ws_label,
        );
        builder.emit(abi::label(&ws_label));
        builder.emit(abi::add_registers(&scratch11, &scratch11, &scratch13));
        builder.emit(abi::add_registers(&scratch15, &scratch15, &scratch13));
        builder.emit(abi::subtract_registers(&scratch12, &scratch12, &scratch13));
        builder.emit(abi::branch(&loop_label));
        builder.emit(abi::label(&not_ws_label));
        builder.emit(abi::add_immediate(&scratch11, &scratch11, 1));
        builder.emit(abi::add_immediate(&scratch15, &scratch15, 1));
        builder.emit(abi::subtract_immediate(&scratch12, &scratch12, 1));
        builder.emit(abi::store_u64(&scratch15, abi::stack_pointer(), end_slot));
        builder.emit(abi::branch(&loop_label));
        builder.emit(abi::label(&done_label));
    }

    builder.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
    builder.emit(abi::load_u64(&scratch10, abi::stack_pointer(), start_slot));
    builder.emit(abi::load_u64(&scratch11, abi::stack_pointer(), end_slot));
    builder.emit(abi::subtract_registers(&scratch12, &scratch11, &scratch10));
    builder.emit(abi::add_immediate(&scratch13, &scratch16, 8));
    builder.emit(abi::add_registers(&scratch13, &scratch13, &scratch10));
    let result = builder.emit_materialize_string_from_bytes(&scratch13, &scratch12)?;
    Ok(ValueResult {
        type_: "String".to_string(),
        location: Operand::from(result.render()),
        text: "strings.trim".to_string(),
    })
}
