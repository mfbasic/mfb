//! Shared grapheme-cluster segmentation for `strings::{graphemes,graphemesCount,graphemeAt}`.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::target::shared::nir::*;

pub(crate) fn lower_strings_graphemes(
    builder: &mut CodeBuilder,
    value: &NirValue,
) -> Result<ValueResult, String> {
    let scratch16 = builder.temporary_vreg();
    let scratch9 = builder.temporary_vreg();
    let scratch14 = builder.temporary_vreg();
    let scratch22 = builder.temporary_vreg();
    let scratch10 = builder.temporary_vreg();
    let scratch11 = builder.temporary_vreg();
    let scratch12 = builder.temporary_vreg();
    let scratch24 = builder.temporary_vreg();
    let scratch25 = builder.temporary_vreg();
    let scratch23 = builder.temporary_vreg();
    let scratch15 = builder.temporary_vreg();
    let scratch26 = builder.temporary_vreg();
    let scratch27 = builder.temporary_vreg();
    let scratch13 = builder.temporary_vreg();
    let scratch20 = builder.temporary_vreg();
    let scratch21 = builder.temporary_vreg();
    let scratch28 = builder.temporary_vreg();
    let value = builder.lower_value(value)?;
    builder.require_string("strings.graphemes value", &value)?;
    let value_slot = builder.spill_to_slot("strings_graphemes_value", &value.location);
    let count_slot = builder.allocate_stack_object("strings_graphemes_count", 8);
    let state_bc_slot = builder.allocate_stack_object("strings_graphemes_state_bc", 8);
    let state_icb_slot = builder.allocate_stack_object("strings_graphemes_state_icb", 8);
    let result_slot = builder.allocate_stack_object("strings_graphemes_result", 8);
    let layout = CollectionTypeLayout::from_type("List OF String").ok_or_else(|| {
        "native strings.graphemes cannot resolve List OF String layout".to_string()
    })?;

    let count_empty = builder.label("strings_graphemes_count_empty");
    let count_loop = builder.label("strings_graphemes_count_loop");
    let count_break = builder.label("strings_graphemes_count_break");
    let count_no_break = builder.label("strings_graphemes_count_no_break");
    let count_after_break = builder.label("strings_graphemes_count_after_break");
    let count_done = builder.label("strings_graphemes_count_done");
    let alloc_ok = builder.label("strings_graphemes_alloc_ok");
    let write_empty = builder.label("strings_graphemes_write_empty");
    let write_loop = builder.label("strings_graphemes_write_loop");
    let write_break = builder.label("strings_graphemes_write_break");
    let write_no_break = builder.label("strings_graphemes_write_no_break");
    let write_after_break = builder.label("strings_graphemes_write_after_break");
    let write_final = builder.label("strings_graphemes_write_final");

    builder.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
    builder.emit(abi::load_u64(&scratch9, &scratch16, 0));
    builder.emit(abi::compare_immediate(&scratch9, "0"));
    builder.emit(abi::branch_eq(&count_empty));
    builder.emit(abi::add_immediate(&scratch14, &scratch16, 8));
    builder.emit(abi::move_immediate(&scratch22, "Integer", "1"));
    builder.emit_utf8_decode_next(&scratch14, &scratch10, &scratch11);
    builder.emit_unicode_property_lookup(&scratch10, &scratch12);
    builder.emit_unicode_property_boundclass(&scratch12, &scratch24);
    builder.emit_unicode_property_indic_conjunct_break(&scratch12, &scratch25);
    builder.emit(abi::move_register(&scratch23, &scratch11));
    builder.emit(abi::label(&count_loop));
    builder.emit(abi::compare_registers(&scratch23, &scratch9));
    builder.emit(abi::branch_ge(&count_done));
    builder.emit(abi::add_registers(&scratch15, &scratch14, &scratch23));
    builder.emit_utf8_decode_next(&scratch15, &scratch10, &scratch11);
    builder.emit_unicode_property_lookup(&scratch10, &scratch12);
    builder.emit_unicode_property_boundclass(&scratch12, &scratch26);
    builder.emit_unicode_property_indic_conjunct_break(&scratch12, &scratch27);
    builder.emit_grapheme_break_branch(
        &scratch24,
        &scratch25,
        &scratch26,
        &scratch27,
        &count_break,
        &count_no_break,
    );
    builder.emit(abi::label(&count_break));
    builder.emit(abi::add_immediate(&scratch22, &scratch22, 1));
    builder.emit(abi::branch(&count_after_break));
    builder.emit(abi::label(&count_no_break));
    builder.emit(abi::branch(&count_after_break));
    builder.emit(abi::label(&count_after_break));
    builder.emit_grapheme_state_update(&scratch24, &scratch25, &scratch26, &scratch27);
    builder.emit(abi::add_registers(&scratch23, &scratch23, &scratch11));
    builder.emit(abi::branch(&count_loop));
    builder.emit(abi::label(&count_empty));
    builder.emit(abi::move_immediate(&scratch22, "Integer", "0"));
    builder.emit(abi::label(&count_done));
    builder.emit(abi::store_u64(&scratch22, abi::stack_pointer(), count_slot));

    // Checked size arithmetic (audit-unicode #8): the grapheme count is
    // derived from an arena-bounded string, so a wrap is unreachable on real
    // hardware, but every arena-size computation shares the same
    // self-defending shape.
    let size_overflow = builder.label("strings_graphemes_size_overflow");
    builder.emit(abi::move_immediate(
        &scratch13,
        "Integer",
        &COLLECTION_ENTRY_SIZE.to_string(),
    ));
    builder.emit_checked_size_multiply(&scratch13, &scratch13, &scratch22, &size_overflow);
    builder.emit_checked_size_add_immediate(
        abi::return_register(),
        &scratch13,
        COLLECTION_HEADER_SIZE,
        &size_overflow,
    );
    builder.emit_checked_size_add(
        abi::return_register(),
        abi::return_register(),
        &scratch9,
        &size_overflow,
    );
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
    builder.emit_arena_alloc_call();
    builder.emit(abi::branch_eq(&alloc_ok));
    builder.raise_error_bare("ErrOutOfMemory")?;
    // A size wrap reports the same 77010001 an impossible allocation would
    // (x0 does not hold an error code before the call, so the register-based
    // return above cannot be shared).
    builder.emit(abi::label(&size_overflow));
    builder.raise_error_bare("ErrOutOfMemory")?;
    builder.emit(abi::label(&alloc_ok));
    builder.emit(abi::store_u64(
        abi::mfb_return(1),
        abi::stack_pointer(),
        result_slot,
    ));
    builder.emit(abi::load_u64(&scratch11, abi::stack_pointer(), count_slot));
    builder.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
    builder.emit(abi::load_u64(&scratch9, &scratch16, 0));
    builder.emit_write_list_header_from_registers(
        &layout,
        abi::mfb_return(1),
        &scratch11,
        &scratch9,
    );

    builder.emit(abi::compare_immediate(&scratch9, "0"));
    builder.emit(abi::branch_eq(&write_empty));
    builder.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
    builder.emit(abi::load_u64(&scratch9, &scratch16, 0));
    builder.emit(abi::add_immediate(&scratch14, &scratch16, 8));
    builder.emit(abi::load_u64(
        abi::mfb_return(1),
        abi::stack_pointer(),
        result_slot,
    ));
    builder.emit(abi::add_immediate(
        &scratch20,
        abi::mfb_return(1),
        COLLECTION_HEADER_SIZE,
    ));
    builder.emit_collection_data_pointer_for(&scratch21, abi::mfb_return(1), "String");
    builder.emit(abi::move_immediate(&scratch22, "Integer", "0"));
    builder.emit(abi::move_immediate(&scratch24, "Integer", "0"));
    builder.emit_utf8_decode_next(&scratch14, &scratch10, &scratch11);
    builder.emit_unicode_property_lookup(&scratch10, &scratch12);
    builder.emit_unicode_property_boundclass(&scratch12, &scratch25);
    builder.emit_unicode_property_indic_conjunct_break(&scratch12, &scratch26);
    builder.emit(abi::store_u64(
        &scratch25,
        abi::stack_pointer(),
        state_bc_slot,
    ));
    builder.emit(abi::store_u64(
        &scratch26,
        abi::stack_pointer(),
        state_icb_slot,
    ));
    builder.emit(abi::move_register(&scratch23, &scratch11));
    builder.emit(abi::label(&write_loop));
    builder.emit(abi::compare_registers(&scratch23, &scratch9));
    builder.emit(abi::branch_ge(&write_final));
    builder.emit(abi::add_registers(&scratch15, &scratch14, &scratch23));
    builder.emit_utf8_decode_next(&scratch15, &scratch10, &scratch11);
    builder.emit_unicode_property_lookup(&scratch10, &scratch12);
    builder.emit_unicode_property_boundclass(&scratch12, &scratch27);
    builder.emit_unicode_property_indic_conjunct_break(&scratch12, &scratch28);
    builder.emit(abi::load_u64(
        &scratch25,
        abi::stack_pointer(),
        state_bc_slot,
    ));
    builder.emit(abi::load_u64(
        &scratch26,
        abi::stack_pointer(),
        state_icb_slot,
    ));
    builder.emit_grapheme_break_branch(
        &scratch25,
        &scratch26,
        &scratch27,
        &scratch28,
        &write_break,
        &write_no_break,
    );
    builder.emit(abi::label(&write_break));
    builder.emit_grapheme_state_update(&scratch25, &scratch26, &scratch27, &scratch28);
    builder.emit(abi::store_u64(
        &scratch25,
        abi::stack_pointer(),
        state_bc_slot,
    ));
    builder.emit(abi::store_u64(
        &scratch26,
        abi::stack_pointer(),
        state_icb_slot,
    ));
    builder.emit_string_split_write_entry(
        &scratch20, &scratch21, &scratch22, &scratch24, &scratch23, &scratch14,
    )?;
    builder.emit(abi::move_register(&scratch24, &scratch23));
    builder.emit(abi::branch(&write_after_break));
    builder.emit(abi::label(&write_no_break));
    builder.emit_grapheme_state_update(&scratch25, &scratch26, &scratch27, &scratch28);
    builder.emit(abi::store_u64(
        &scratch25,
        abi::stack_pointer(),
        state_bc_slot,
    ));
    builder.emit(abi::store_u64(
        &scratch26,
        abi::stack_pointer(),
        state_icb_slot,
    ));
    builder.emit(abi::branch(&write_after_break));
    builder.emit(abi::label(&write_after_break));
    builder.emit(abi::add_registers(&scratch23, &scratch23, &scratch11));
    builder.emit(abi::branch(&write_loop));
    builder.emit(abi::label(&write_final));
    builder.emit_string_split_write_entry(
        &scratch20, &scratch21, &scratch22, &scratch24, &scratch9, &scratch14,
    )?;
    // audit-unicode #9: the write pass must have emitted exactly the entry
    // count and payload bytes the counting pass allocated; a divergence is a
    // silent heap overflow.
    builder.emit_write_cursor_assert(&scratch22, &scratch9, "strings_graphemes_data");
    builder.emit(abi::load_u64(&scratch10, abi::stack_pointer(), result_slot));
    builder.emit(abi::load_u64(&scratch11, abi::stack_pointer(), count_slot));
    builder.emit(abi::move_immediate(
        &scratch12,
        "Integer",
        &COLLECTION_ENTRY_SIZE.to_string(),
    ));
    builder.emit(abi::multiply_registers(&scratch11, &scratch11, &scratch12));
    builder.emit(abi::add_registers(&scratch10, &scratch10, &scratch11));
    builder.emit(abi::add_immediate(
        &scratch10,
        &scratch10,
        COLLECTION_HEADER_SIZE,
    ));
    builder.emit_write_cursor_assert(&scratch20, &scratch10, "strings_graphemes_entries");
    builder.emit(abi::label(&write_empty));

    let result = builder.allocate_register()?;
    builder.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
    Ok(ValueResult {
        type_: "List OF String".to_string(),
        location: Operand::from(result.render()),
        text: "strings.graphemes".to_string(),
    })
}
