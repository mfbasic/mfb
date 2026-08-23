//! Shared affix strip for `strings::{stripPrefix,stripSuffix}`.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::target::shared::abi;

pub(crate) fn lower_strings_strip(
    builder: &mut CodeBuilder,
    value: &ValueResult,
    part: &ValueResult,
    suffix: bool,
) -> Result<ValueResult, String> {
    let scratch16 = builder.temporary_vreg();
    let scratch17 = builder.temporary_vreg();
    let scratch9 = builder.temporary_vreg();
    let scratch10 = builder.temporary_vreg();
    let scratch11 = builder.temporary_vreg();
    let scratch12 = builder.temporary_vreg();
    let scratch13 = builder.temporary_vreg();
    let value = value.clone();
    builder.require_string("strings.strip value", &value)?;
    let value_slot = builder.spill_to_slot("strings_strip_value", &value.location);
    let part = part.clone();
    builder.require_string("strings.strip part", &part)?;
    let part_slot = builder.spill_to_slot("strings_strip_part", &part.location);
    let ptr_slot = builder.allocate_stack_object("strings_strip_ptr", 8);
    let len_slot = builder.allocate_stack_object("strings_strip_len", 8);

    let matched = builder.label("strings_strip_matched");
    let unchanged = builder.label("strings_strip_unchanged");
    let no_match = builder.label("strings_strip_no_match");
    let build = builder.label("strings_strip_build");

    // x16 = value ptr, x9 = value len, x17 = part ptr, x10 = part len.
    builder.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
    builder.emit(abi::load_u64(&scratch17, abi::stack_pointer(), part_slot));
    builder.emit(abi::load_u64(&scratch9, &scratch16, 0));
    builder.emit(abi::load_u64(&scratch10, &scratch17, 0));
    // part empty or longer than value -> unchanged.
    builder.emit(abi::compare_immediate(&scratch10, "0"));
    builder.emit(abi::branch_eq(&unchanged));
    builder.emit(abi::compare_registers(&scratch10, &scratch9));
    builder.emit(abi::branch_hi(&unchanged));
    builder.emit(abi::add_immediate(&scratch11, &scratch16, 8));
    builder.emit(abi::add_immediate(&scratch12, &scratch17, 8));
    if suffix {
        builder.emit(abi::subtract_registers(&scratch13, &scratch9, &scratch10));
        builder.emit(abi::add_registers(&scratch11, &scratch11, &scratch13));
    }
    builder.emit_string_byte_range_equal_branch(
        &scratch11, &scratch12, &scratch10, &matched, &no_match,
    );
    builder.emit(abi::label(&no_match));
    builder.emit(abi::branch(&unchanged));

    // matched: result = value with one part removed. Compute ptr/len into slots.
    builder.emit(abi::label(&matched));
    builder.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
    builder.emit(abi::load_u64(&scratch17, abi::stack_pointer(), part_slot));
    builder.emit(abi::load_u64(&scratch9, &scratch16, 0));
    builder.emit(abi::load_u64(&scratch10, &scratch17, 0));
    builder.emit(abi::subtract_registers(&scratch12, &scratch9, &scratch10));
    builder.emit(abi::add_immediate(&scratch13, &scratch16, 8));
    if !suffix {
        // strip from front: advance data pointer past the prefix.
        builder.emit(abi::add_registers(&scratch13, &scratch13, &scratch10));
    }
    builder.emit(abi::store_u64(&scratch13, abi::stack_pointer(), ptr_slot));
    builder.emit(abi::store_u64(&scratch12, abi::stack_pointer(), len_slot));
    builder.emit(abi::branch(&build));

    // unchanged: result = whole value (ptr = value+8, len = value len).
    builder.emit(abi::label(&unchanged));
    builder.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
    builder.emit(abi::load_u64(&scratch9, &scratch16, 0));
    builder.emit(abi::add_immediate(&scratch13, &scratch16, 8));
    builder.emit(abi::store_u64(&scratch13, abi::stack_pointer(), ptr_slot));
    builder.emit(abi::store_u64(&scratch9, abi::stack_pointer(), len_slot));

    builder.emit(abi::label(&build));
    builder.emit(abi::load_u64(&scratch13, abi::stack_pointer(), ptr_slot));
    builder.emit(abi::load_u64(&scratch12, abi::stack_pointer(), len_slot));
    let result = builder.emit_materialize_string_from_bytes(&scratch13, &scratch12)?;
    let label = if suffix {
        "strings.stripSuffix"
    } else {
        "strings.stripPrefix"
    };
    Ok(ValueResult {
        origin: None,
        type_: "String".to_string(),
        location: Operand::from(result.render()),
        text: label.to_string(),
    })
}
