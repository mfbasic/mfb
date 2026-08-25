//! Shared prefix/suffix-set predicate for `strings::{startsWithAny,endsWithAny}`.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::typed_list_element_type;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;

pub(crate) fn lower_strings_with_any(
    builder: &mut CodeBuilder,
    value: &ValueResult,
    parts: &ValueResult,
    suffix: bool,
) -> Result<ValueResult, String> {
    let scratch16 = builder.temporary_vreg();
    let scratch9 = builder.temporary_vreg();
    let scratch11 = builder.temporary_vreg();
    let scratch17 = builder.temporary_vreg();
    let scratch23 = builder.temporary_vreg();
    let scratch22 = builder.temporary_vreg();
    let scratch21 = builder.temporary_vreg();
    let scratch20 = builder.temporary_vreg();
    let scratch10 = builder.temporary_vreg();
    let scratch12 = builder.temporary_vreg();
    let scratch15 = builder.temporary_vreg();
    let scratch13 = builder.temporary_vreg();
    let value = value.clone();
    builder.require_string("strings.withAny value", &value)?;
    let value_slot = builder.spill_to_slot("strings_with_any_value", &value.location);
    let parts = parts.clone();
    if typed_list_element_type(&parts.type_)
        .map(|type_| type_.name().into_owned())
        .as_deref()
        != Some("String")
    {
        return Err(format!(
            "strings.startsWithAny/endsWithAny parts must be List OF String, got {}",
            parts.type_
        ));
    }
    let parts_slot = builder.spill_to_slot("strings_with_any_parts", &parts.location);
    let result_slot = builder.allocate_stack_object("strings_with_any_result", 8);

    let true_label = builder.label("strings_with_any_true");
    let false_label = builder.label("strings_with_any_false");
    let done_label = builder.label("strings_with_any_done");
    let outer_loop = builder.label("strings_with_any_loop");
    let outer_next = builder.label("strings_with_any_next");
    let no_match = builder.label("strings_with_any_no_match");

    // x16 = value ptr, x9 = value len, x11 = value data ptr.
    builder.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
    builder.emit(abi::load_u64(&scratch9, &scratch16, 0));
    builder.emit(abi::add_immediate(&scratch11, &scratch16, 8));
    // x17 = list ptr, x19 = count, x22 = entry ptr, x21 = data ptr.
    builder.emit(abi::load_u64(&scratch17, abi::stack_pointer(), parts_slot));
    builder.emit(abi::load_u64(
        &scratch23,
        &scratch17,
        COLLECTION_OFFSET_COUNT,
    ));
    builder.emit(abi::add_immediate(
        &scratch22,
        &scratch17,
        COLLECTION_HEADER_SIZE,
    ));
    builder.emit_collection_data_pointer_for(&scratch21, &scratch17, "String");
    builder.emit(abi::move_immediate(&scratch20, "Integer", "0"));

    builder.emit(abi::label(&outer_loop));
    builder.emit(abi::compare_registers(&scratch20, &scratch23));
    builder.emit(abi::branch_ge(&false_label));
    // x10 = element length, x12 = element bytes pointer.
    builder.emit(abi::load_u64(
        &scratch10,
        &scratch22,
        COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
    ));
    builder.emit(abi::load_u64(
        &scratch12,
        &scratch22,
        COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
    ));
    builder.emit(abi::add_registers(&scratch12, &scratch21, &scratch12));
    // element longer than value -> no match.
    builder.emit(abi::compare_registers(&scratch10, &scratch9));
    builder.emit(abi::branch_hi(&outer_next));
    // x15 = compare start in value (offset by len-elementLen for suffix).
    builder.emit(abi::move_register(&scratch15, &scratch11));
    if suffix {
        builder.emit(abi::subtract_registers(&scratch13, &scratch9, &scratch10));
        builder.emit(abi::add_registers(&scratch15, &scratch15, &scratch13));
    }
    builder.emit_string_byte_range_equal_branch(
        &scratch15,
        &scratch12,
        &scratch10,
        &true_label,
        &no_match,
    );
    builder.emit(abi::label(&no_match));
    builder.emit(abi::label(&outer_next));
    builder.emit(abi::add_immediate(
        &scratch22,
        &scratch22,
        COLLECTION_ENTRY_SIZE,
    ));
    builder.emit(abi::add_immediate(&scratch20, &scratch20, 1));
    builder.emit(abi::branch(&outer_loop));

    builder.emit_string_predicate_result(result_slot, &true_label, &false_label, &done_label);
    let label = if suffix {
        "strings.endsWithAny"
    } else {
        "strings.startsWithAny"
    };
    builder.finish_string_predicate_result(label, result_slot)
}
