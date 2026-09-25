//! Shared grapheme-cluster segmentation for `strings::{graphemes,graphemesCount,graphemeAt}`.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::types::ParameterType;

pub(crate) fn lower_strings_graphemes(
    builder: &mut CodeBuilder,
    value: &ValueResult,
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
    let value = value.clone();
    builder.require_string("strings.graphemes value", &value)?;
    let value_slot = builder.spill_to_slot("strings_graphemes_value", &value.location);
    let count_slot = builder.allocate_stack_object("strings_graphemes_count", 8);
    let state_bc_slot = builder.allocate_stack_object("strings_graphemes_state_bc", 8);
    let state_icb_slot = builder.allocate_stack_object("strings_graphemes_state_icb", 8);
    let result_slot = builder.allocate_stack_object("strings_graphemes_result", 8);
    let layout = CollectionTypeLayout::from_type(&ParameterType::list_of(ParameterType::String))
        .ok_or_else(|| {
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
    builder.emit_collection_data_pointer_for(
        &scratch21,
        abi::mfb_return(1),
        &ParameterType::String,
    );
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

    let result = builder.allocate_register();
    builder.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::list_of(ParameterType::String),
        location: Operand::from(result.render()),
        text: "strings.graphemes".to_string(),
    })
}

/// plan-146-C: the window half of `strings::graphemeAt` — the `(ptr, len)` of the
/// `index`-th grapheme cluster inside `value`'s own bytes, raising
/// `ErrIndexOutOfRange` for a negative index or one past the last cluster.
///
/// The copying lowering gets the span out of the `List OF String` that
/// [`lower_strings_graphemes`] builds, which allocates; an in-place arm must
/// allocate nothing, so this walks the clusters directly with the same break
/// emitters the segmentation uses (`emit_grapheme_break_branch`,
/// `emit_grapheme_state_update`) and stops at the wanted one. The walk's state
/// lives in frame slots: the property emitters clobber registers.
pub(crate) fn grapheme_at_window(
    builder: &mut CodeBuilder,
    value: &ValueResult,
    index: &ValueResult,
) -> Result<(VirtualRegister, VirtualRegister), String> {
    builder.require_string("strings.graphemeAt value", value)?;
    if index.type_ != ParameterType::Integer {
        return Err(format!(
            "strings.graphemeAt index must be Integer, got {}",
            index.type_
        ));
    }
    let value_slot = builder.spill_to_slot("strings_grapheme_at_w_value", &value.location);
    let index_slot = builder.spill_to_slot("strings_grapheme_at_w_index", &index.location);
    let state_bc_slot = builder.allocate_stack_object("strings_grapheme_at_w_bc", 8);
    let state_icb_slot = builder.allocate_stack_object("strings_grapheme_at_w_icb", 8);
    let start_slot = builder.allocate_stack_object("strings_grapheme_at_w_start", 8);
    let cluster_slot = builder.allocate_stack_object("strings_grapheme_at_w_cluster", 8);
    let cursor_slot = builder.allocate_stack_object("strings_grapheme_at_w_cursor", 8);
    let ptr_slot = builder.allocate_stack_object("strings_grapheme_at_w_ptr", 8);
    let len_slot = builder.allocate_stack_object("strings_grapheme_at_w_len", 8);

    let v = builder.temporary_vreg();
    let len = builder.temporary_vreg();
    let bytes = builder.temporary_vreg();
    let cursor = builder.temporary_vreg();
    let width = builder.temporary_vreg();
    let scalar = builder.temporary_vreg();
    let props = builder.temporary_vreg();
    let bc = builder.temporary_vreg();
    let icb = builder.temporary_vreg();
    let prev_bc = builder.temporary_vreg();
    let prev_icb = builder.temporary_vreg();
    let start = builder.temporary_vreg();
    let cluster = builder.temporary_vreg();
    let want = builder.temporary_vreg();
    let scan = builder.temporary_vreg();

    let invalid = builder.label("strings_grapheme_at_w_invalid");
    let walk = builder.label("strings_grapheme_at_w_walk");
    let brk = builder.label("strings_grapheme_at_w_break");
    let no_brk = builder.label("strings_grapheme_at_w_no_break");
    let after = builder.label("strings_grapheme_at_w_after");
    let last = builder.label("strings_grapheme_at_w_last");
    let found = builder.label("strings_grapheme_at_w_found");

    // A negative index, or no cluster at all, is out of range.
    builder.emit(abi::load_u64(&v, abi::stack_pointer(), value_slot));
    builder.emit(abi::load_u64(&len, &v, 0));
    builder.emit(abi::load_u64(&want, abi::stack_pointer(), index_slot));
    builder.emit(abi::compare_immediate(&want, "0"));
    builder.emit(abi::branch_lt(&invalid));
    builder.emit(abi::compare_immediate(&len, "0"));
    builder.emit(abi::branch_eq(&invalid));

    // The first scalar opens the first cluster; its properties are the state.
    builder.emit(abi::add_immediate(&bytes, &v, 8));
    builder.emit_utf8_decode_next(&bytes, &scalar, &width);
    builder.emit_unicode_property_lookup(&scalar, &props);
    builder.emit_unicode_property_boundclass(&props, &prev_bc);
    builder.emit_unicode_property_indic_conjunct_break(&props, &prev_icb);
    builder.emit(abi::store_u64(
        &prev_bc,
        abi::stack_pointer(),
        state_bc_slot,
    ));
    builder.emit(abi::store_u64(
        &prev_icb,
        abi::stack_pointer(),
        state_icb_slot,
    ));
    builder.emit(abi::move_immediate(&start, "Integer", "0"));
    builder.emit(abi::store_u64(&start, abi::stack_pointer(), start_slot));
    builder.emit(abi::move_immediate(&cluster, "Integer", "0"));
    builder.emit(abi::store_u64(&cluster, abi::stack_pointer(), cluster_slot));
    builder.emit(abi::store_u64(&width, abi::stack_pointer(), cursor_slot));

    builder.emit(abi::label(&walk));
    builder.emit(abi::load_u64(&v, abi::stack_pointer(), value_slot));
    builder.emit(abi::load_u64(&len, &v, 0));
    builder.emit(abi::add_immediate(&bytes, &v, 8));
    builder.emit(abi::load_u64(&cursor, abi::stack_pointer(), cursor_slot));
    builder.emit(abi::compare_registers(&cursor, &len));
    builder.emit(abi::branch_ge(&last));
    builder.emit(abi::add_registers(&scan, &bytes, &cursor));
    builder.emit_utf8_decode_next(&scan, &scalar, &width);
    builder.emit_unicode_property_lookup(&scalar, &props);
    builder.emit_unicode_property_boundclass(&props, &bc);
    builder.emit_unicode_property_indic_conjunct_break(&props, &icb);
    builder.emit(abi::load_u64(&prev_bc, abi::stack_pointer(), state_bc_slot));
    builder.emit(abi::load_u64(
        &prev_icb,
        abi::stack_pointer(),
        state_icb_slot,
    ));
    builder.emit_grapheme_break_branch(&prev_bc, &prev_icb, &bc, &icb, &brk, &no_brk);

    // A break before this scalar ends the cluster that started at `start`.
    builder.emit(abi::label(&brk));
    builder.emit_grapheme_state_update(&prev_bc, &prev_icb, &bc, &icb);
    builder.emit(abi::store_u64(
        &prev_bc,
        abi::stack_pointer(),
        state_bc_slot,
    ));
    builder.emit(abi::store_u64(
        &prev_icb,
        abi::stack_pointer(),
        state_icb_slot,
    ));
    builder.emit(abi::load_u64(&cluster, abi::stack_pointer(), cluster_slot));
    builder.emit(abi::load_u64(&want, abi::stack_pointer(), index_slot));
    builder.emit(abi::compare_registers(&cluster, &want));
    builder.emit(abi::branch_eq(&found));
    builder.emit(abi::add_immediate(&cluster, &cluster, 1));
    builder.emit(abi::store_u64(&cluster, abi::stack_pointer(), cluster_slot));
    builder.emit(abi::load_u64(&cursor, abi::stack_pointer(), cursor_slot));
    builder.emit(abi::store_u64(&cursor, abi::stack_pointer(), start_slot));
    builder.emit(abi::branch(&after));

    builder.emit(abi::label(&no_brk));
    builder.emit_grapheme_state_update(&prev_bc, &prev_icb, &bc, &icb);
    builder.emit(abi::store_u64(
        &prev_bc,
        abi::stack_pointer(),
        state_bc_slot,
    ));
    builder.emit(abi::store_u64(
        &prev_icb,
        abi::stack_pointer(),
        state_icb_slot,
    ));
    builder.emit(abi::branch(&after));

    builder.emit(abi::label(&after));
    builder.emit(abi::load_u64(&cursor, abi::stack_pointer(), cursor_slot));
    builder.emit(abi::add_registers(&cursor, &cursor, &width));
    builder.emit(abi::store_u64(&cursor, abi::stack_pointer(), cursor_slot));
    builder.emit(abi::branch(&walk));

    // The last cluster runs to the end of the string.
    builder.emit(abi::label(&last));
    builder.emit(abi::load_u64(&cluster, abi::stack_pointer(), cluster_slot));
    builder.emit(abi::load_u64(&want, abi::stack_pointer(), index_slot));
    builder.emit(abi::compare_registers(&cluster, &want));
    builder.emit(abi::branch_ne(&invalid));
    // `cursor` is the string's length here: the walk steps scalar by scalar.
    builder.emit(abi::branch(&found));

    builder.emit(abi::label(&invalid));
    builder.raise_error("strings.graphemeAt", "ErrIndexOutOfRange")?;

    // `[start, cursor)` of the wanted cluster (at `last`, `cursor` is the length).
    builder.emit(abi::label(&found));
    builder.emit(abi::load_u64(&v, abi::stack_pointer(), value_slot));
    builder.emit(abi::add_immediate(&bytes, &v, 8));
    builder.emit(abi::load_u64(&start, abi::stack_pointer(), start_slot));
    builder.emit(abi::load_u64(&cursor, abi::stack_pointer(), cursor_slot));
    builder.emit(abi::add_registers(&bytes, &bytes, &start));
    builder.emit(abi::subtract_registers(&cursor, &cursor, &start));
    builder.emit(abi::store_u64(&bytes, abi::stack_pointer(), ptr_slot));
    builder.emit(abi::store_u64(&cursor, abi::stack_pointer(), len_slot));
    builder.emit(abi::load_u64(&bytes, abi::stack_pointer(), ptr_slot));
    builder.emit(abi::load_u64(&cursor, abi::stack_pointer(), len_slot));
    Ok((bytes, cursor))
}
