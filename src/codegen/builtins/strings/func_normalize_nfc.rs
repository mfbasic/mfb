//! `strings.normalizeNfc` — descriptor + clean-room native lowering.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

pub(crate) fn lower(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    if let Some(value) = builder.static_strings_package_string("strings.normalizeNfc", args)? {
        let register = builder.load_string_constant(&value)?;
        return Ok(ValueResult {
            origin: None,
            type_: "String".to_string(),
            location: Operand::from(register.render()),
            text: "strings.normalizeNfc".to_string(),
        });
    }
    if args.len() != 1 {
        return Err("strings.normalizeNfc: no native lowering for these arguments".to_string());
    }
    let value = &args[0];

    let scratch20 = builder.temporary_vreg();
    let scratch21 = builder.temporary_vreg();
    let scratch22 = builder.temporary_vreg();
    let scratch23 = builder.temporary_vreg();
    let scratch24 = builder.temporary_vreg();
    let scratch14 = builder.temporary_vreg();
    let scratch10 = builder.temporary_vreg();
    let scratch11 = builder.temporary_vreg();
    let scratch26 = builder.temporary_vreg();
    let scratch27 = builder.temporary_vreg();
    let scratch13 = builder.temporary_vreg();
    let scratch25 = builder.temporary_vreg();
    let scratch12 = builder.temporary_vreg();
    let scratch15 = builder.temporary_vreg();
    let scratch28 = builder.temporary_vreg();
    let scratch16 = builder.temporary_vreg();
    let scratch17 = builder.temporary_vreg();
    let scratch9 = builder.temporary_vreg();
    let scratch8 = builder.temporary_vreg();
    let value = value.clone();
    builder.require_string("strings.normalizeNfc value", &value)?;
    let value_slot = builder.spill_to_slot("strings_normalize_nfc_value", &value.location);
    let scalar_count_slot = builder.allocate_stack_object("strings_normalize_nfc_scalar_count", 8);
    let temp_slot = builder.allocate_stack_object("strings_normalize_nfc_temp", 8);
    let composed_count_slot =
        builder.allocate_stack_object("strings_normalize_nfc_composed_count", 8);
    let output_len_slot = builder.allocate_stack_object("strings_normalize_nfc_output_len", 8);
    let width_slot = builder.allocate_stack_object("strings_normalize_nfc_width", 8);
    let result_slot = builder.allocate_stack_object("strings_normalize_nfc_result", 8);

    let count_loop = builder.label("strings_nfc_count_loop");
    let count_identity = builder.label("strings_nfc_count_identity");
    let count_next = builder.label("strings_nfc_count_next");
    let count_done = builder.label("strings_nfc_count_done");
    let temp_alloc_ok = builder.label("strings_nfc_temp_alloc_ok");
    let fill_loop = builder.label("strings_nfc_fill_loop");
    let fill_identity = builder.label("strings_nfc_fill_identity");
    let fill_sequence_loop = builder.label("strings_nfc_fill_sequence_loop");
    let fill_store = builder.label("strings_nfc_fill_store");
    let fill_next = builder.label("strings_nfc_fill_next");
    let fill_done = builder.label("strings_nfc_fill_done");
    let order_loop = builder.label("strings_nfc_order_loop");
    let order_done = builder.label("strings_nfc_order_done");
    let order_no_swap = builder.label("strings_nfc_order_no_swap");
    let order_swap = builder.label("strings_nfc_order_swap");
    let order_decrement = builder.label("strings_nfc_order_decrement");
    let compose_loop = builder.label("strings_nfc_compose_loop");
    let compose_write = builder.label("strings_nfc_compose_write");
    let compose_try = builder.label("strings_nfc_compose_try");
    let compose_try_tables = builder.label("strings_nfc_compose_try_tables");
    let compose_scan_loop = builder.label("strings_nfc_compose_scan_loop");
    let compose_found = builder.label("strings_nfc_compose_found");
    let compose_found_direct = builder.label("strings_nfc_compose_found_direct");
    let compose_next = builder.label("strings_nfc_compose_next");
    let compose_no_starter = builder.label("strings_nfc_compose_no_starter");
    let compose_nonstarter = builder.label("strings_nfc_compose_nonstarter");
    let compose_nonstarter_update = builder.label("strings_nfc_compose_nonstarter_update");
    let compose_nonstarter_done = builder.label("strings_nfc_compose_nonstarter_done");
    let byte_len_loop = builder.label("strings_nfc_byte_len_loop");
    let byte_len_done = builder.label("strings_nfc_byte_len_done");
    let result_alloc_ok = builder.label("strings_nfc_result_alloc_ok");
    let encode_loop = builder.label("strings_nfc_encode_loop");
    let encode_done = builder.label("strings_nfc_encode_done");
    let ascii_scan = builder.label("strings_nfc_ascii_scan");
    let ascii_copy = builder.label("strings_nfc_ascii_copy");
    let ascii_size_overflow = builder.label("strings_nfc_ascii_size_overflow");
    let ascii_alloc_ok = builder.label("strings_nfc_ascii_alloc_ok");
    let ascii_copy_loop = builder.label("strings_nfc_ascii_copy_loop");
    let ascii_copy_done = builder.label("strings_nfc_ascii_copy_done");
    let nfc_slow = builder.label("strings_nfc_slow");
    let nfc_done = builder.label("strings_nfc_done");

    // E2 (plan-39): NFC quick-check. A pure-ASCII string is already in NFC and
    // its canonical form is byte-identical to the input, so scan for any byte
    // >= 0x80 and, when there are none, return a plain copy — skipping the
    // decompose/reorder/compose passes and their per-codepoint table searches.
    builder.emit(abi::load_u64(&scratch20, abi::stack_pointer(), value_slot));
    builder.emit(abi::load_u64(&scratch21, &scratch20, 0));
    builder.emit(abi::add_immediate(&scratch22, &scratch20, 8));
    builder.emit(abi::move_immediate(&scratch23, "Integer", "0"));
    builder.emit(abi::label(&ascii_scan));
    builder.emit(abi::compare_registers(&scratch23, &scratch21));
    builder.emit(abi::branch_ge(&ascii_copy));
    builder.emit(abi::add_registers(&scratch14, &scratch22, &scratch23));
    builder.emit(abi::load_u8(&scratch10, &scratch14, 0));
    builder.emit(abi::compare_immediate(&scratch10, "128"));
    builder.emit(abi::branch_ge(&nfc_slow));
    builder.emit(abi::add_immediate(&scratch23, &scratch23, 1));
    builder.emit(abi::branch(&ascii_scan));

    builder.emit(abi::label(&ascii_copy));
    // Allocate byte_len + 9 (8-byte header + trailing NUL), matching the slow
    // path's result layout; the checked add self-defends against a wrap.
    builder.emit_checked_size_add_immediate(
        abi::return_register(),
        &scratch21,
        9,
        &ascii_size_overflow,
    );
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
    builder.emit_arena_alloc_call();
    builder.emit(abi::branch_eq(&ascii_alloc_ok));
    builder.raise_error_bare("ErrOutOfMemory")?;
    builder.emit(abi::label(&ascii_size_overflow));
    builder.raise_error_bare("ErrOutOfMemory")?;
    builder.emit(abi::label(&ascii_alloc_ok));
    builder.emit(abi::store_u64(
        abi::mfb_return(1),
        abi::stack_pointer(),
        result_slot,
    ));
    // ARENA_ALLOC clobbers the caller-saved registers, so reload the source
    // pointer/length from their stack homes before copying.
    builder.emit(abi::load_u64(&scratch20, abi::stack_pointer(), value_slot));
    builder.emit(abi::load_u64(&scratch21, &scratch20, 0));
    builder.emit(abi::add_immediate(&scratch22, &scratch20, 8));
    builder.emit(abi::store_u64(&scratch21, abi::mfb_return(1), 0));
    builder.emit(abi::add_immediate(&scratch28, abi::mfb_return(1), 8));
    builder.emit(abi::move_immediate(&scratch23, "Integer", "0"));
    builder.emit(abi::label(&ascii_copy_loop));
    builder.emit(abi::compare_registers(&scratch23, &scratch21));
    builder.emit(abi::branch_ge(&ascii_copy_done));
    builder.emit(abi::add_registers(&scratch14, &scratch22, &scratch23));
    builder.emit(abi::load_u8(&scratch10, &scratch14, 0));
    builder.emit(abi::store_u8(&scratch10, &scratch28, 0));
    builder.emit(abi::add_immediate(&scratch28, &scratch28, 1));
    builder.emit(abi::add_immediate(&scratch23, &scratch23, 1));
    builder.emit(abi::branch(&ascii_copy_loop));
    builder.emit(abi::label(&ascii_copy_done));
    builder.emit(abi::move_immediate(&scratch10, "Integer", "0"));
    builder.emit(abi::store_u8(&scratch10, &scratch28, 0));
    builder.emit(abi::branch(&nfc_done));

    builder.emit(abi::label(&nfc_slow));
    builder.emit(abi::load_u64(&scratch20, abi::stack_pointer(), value_slot));
    builder.emit(abi::load_u64(&scratch21, &scratch20, 0));
    builder.emit(abi::add_immediate(&scratch22, &scratch20, 8));
    builder.emit(abi::move_immediate(&scratch23, "Integer", "0"));
    builder.emit(abi::move_immediate(&scratch24, "Integer", "0"));
    builder.emit(abi::label(&count_loop));
    builder.emit(abi::compare_registers(&scratch23, &scratch21));
    builder.emit(abi::branch_ge(&count_done));
    builder.emit(abi::add_registers(&scratch14, &scratch22, &scratch23));
    builder.emit_utf8_decode_next(&scratch14, &scratch10, &scratch11);
    builder.emit(abi::store_u64(&scratch11, abi::stack_pointer(), width_slot));
    builder.emit_unicode_u32_mapping_lookup(
        &scratch10,
        UNICODE_NFD_ENTRIES_SYMBOL,
        crate::unicode::runtime_tables::tables().nfd_entries.len(),
        UNICODE_NFD_SEQUENCES_SYMBOL,
        &scratch26,
        &scratch27,
    );
    builder.emit(abi::compare_immediate(&scratch27, "0"));
    builder.emit(abi::branch_eq(&count_identity));
    builder.emit(abi::add_registers(&scratch24, &scratch24, &scratch27));
    builder.emit(abi::branch(&count_next));
    builder.emit(abi::label(&count_identity));
    builder.emit(abi::add_immediate(&scratch24, &scratch24, 1));
    builder.emit(abi::label(&count_next));
    builder.emit(abi::load_u64(&scratch11, abi::stack_pointer(), width_slot));
    builder.emit(abi::add_registers(&scratch23, &scratch23, &scratch11));
    builder.emit(abi::branch(&count_loop));
    builder.emit(abi::label(&count_done));
    builder.emit(abi::store_u64(
        &scratch24,
        abi::stack_pointer(),
        scalar_count_slot,
    ));

    // Checked temp-buffer sizing (audit-unicode #8): the decomposed scalar
    // count is derived from an arena-bounded string, so a wrap is
    // unreachable on real hardware, but every arena-size computation shares
    // the same self-defending shape.
    let size_overflow = builder.label("strings_nfc_size_overflow");
    builder.emit(abi::move_immediate(&scratch13, "Integer", "8"));
    builder.emit_checked_size_multiply(
        abi::return_register(),
        &scratch24,
        &scratch13,
        &size_overflow,
    );
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
    builder.emit_arena_alloc_call();
    builder.emit(abi::branch_eq(&temp_alloc_ok));
    builder.raise_error_bare("ErrOutOfMemory")?;
    // A size wrap reports the same 77010001 an impossible allocation would
    // (x0 does not hold an error code before the call, so the register-based
    // return above cannot be shared).
    builder.emit(abi::label(&size_overflow));
    builder.raise_error_bare("ErrOutOfMemory")?;
    builder.emit(abi::label(&temp_alloc_ok));
    builder.emit(abi::store_u64(
        abi::mfb_return(1),
        abi::stack_pointer(),
        temp_slot,
    ));

    builder.emit(abi::load_u64(&scratch20, abi::stack_pointer(), value_slot));
    builder.emit(abi::load_u64(&scratch21, &scratch20, 0));
    builder.emit(abi::add_immediate(&scratch22, &scratch20, 8));
    builder.emit(abi::load_u64(&scratch25, abi::stack_pointer(), temp_slot));
    builder.emit(abi::move_immediate(&scratch23, "Integer", "0"));
    builder.emit(abi::move_immediate(&scratch24, "Integer", "0"));
    builder.emit(abi::label(&fill_loop));
    builder.emit(abi::compare_registers(&scratch23, &scratch21));
    builder.emit(abi::branch_ge(&fill_done));
    builder.emit(abi::add_registers(&scratch14, &scratch22, &scratch23));
    builder.emit_utf8_decode_next(&scratch14, &scratch10, &scratch11);
    builder.emit(abi::store_u64(&scratch11, abi::stack_pointer(), width_slot));
    builder.emit_unicode_u32_mapping_lookup(
        &scratch10,
        UNICODE_NFD_ENTRIES_SYMBOL,
        crate::unicode::runtime_tables::tables().nfd_entries.len(),
        UNICODE_NFD_SEQUENCES_SYMBOL,
        &scratch26,
        &scratch27,
    );
    builder.emit(abi::compare_immediate(&scratch27, "0"));
    builder.emit(abi::branch_eq(&fill_identity));
    builder.emit(abi::label(&fill_sequence_loop));
    builder.emit(abi::compare_immediate(&scratch27, "0"));
    builder.emit(abi::branch_eq(&fill_next));
    builder.emit(abi::load_u32(&scratch10, &scratch26, 0));
    builder.emit(abi::add_immediate(&scratch26, &scratch26, 4));
    builder.emit(abi::branch(&fill_store));
    builder.emit(abi::label(&fill_identity));
    builder.emit(abi::label(&fill_store));
    builder.emit(abi::shift_left_immediate(&scratch12, &scratch24, 3));
    builder.emit(abi::add_registers(&scratch12, &scratch25, &scratch12));
    builder.emit(abi::store_u64(&scratch10, &scratch12, 0));
    builder.emit(abi::add_immediate(&scratch24, &scratch24, 1));
    builder.emit(abi::compare_immediate(&scratch27, "0"));
    builder.emit(abi::branch_eq(&fill_next));
    builder.emit(abi::subtract_immediate(&scratch27, &scratch27, 1));
    builder.emit(abi::branch(&fill_sequence_loop));
    builder.emit(abi::label(&fill_next));
    builder.emit(abi::load_u64(&scratch11, abi::stack_pointer(), width_slot));
    builder.emit(abi::add_registers(&scratch23, &scratch23, &scratch11));
    builder.emit(abi::branch(&fill_loop));
    builder.emit(abi::label(&fill_done));

    builder.emit(abi::load_u64(&scratch25, abi::stack_pointer(), temp_slot));
    builder.emit(abi::load_u64(
        &scratch21,
        abi::stack_pointer(),
        scalar_count_slot,
    ));
    builder.emit(abi::move_immediate(&scratch23, "Integer", "0"));
    builder.emit(abi::label(&order_loop));
    // x12 (dead at the loop head; redefined below) — not x6: ABI registers
    // stay physical, and the x86 remap role-colors them (x6/x7 both
    // collapsed onto rax, corrupting the scan pointers).
    builder.emit(abi::add_immediate(&scratch12, &scratch23, 1));
    builder.emit(abi::compare_registers(&scratch12, &scratch21));
    builder.emit(abi::branch_ge(&order_done));
    builder.emit(abi::shift_left_immediate(&scratch12, &scratch23, 3));
    builder.emit(abi::add_registers(&scratch12, &scratch25, &scratch12));
    builder.emit(abi::load_u64(&scratch10, &scratch12, 0));
    builder.emit(abi::load_u64(&scratch11, &scratch12, 8));
    builder.emit_unicode_property_lookup(&scratch10, &scratch13);
    builder.emit_unicode_property_combining_class(&scratch13, &scratch14);
    builder.emit_unicode_property_lookup(&scratch11, &scratch13);
    builder.emit_unicode_property_combining_class(&scratch13, &scratch15);
    builder.emit(abi::compare_immediate(&scratch15, "0"));
    builder.emit(abi::branch_eq(&order_no_swap));
    builder.emit(abi::compare_registers(&scratch14, &scratch15));
    builder.emit(abi::branch_hi(&order_swap));
    builder.emit(abi::branch(&order_no_swap));
    builder.emit(abi::label(&order_swap));
    builder.emit(abi::store_u64(&scratch11, &scratch12, 0));
    builder.emit(abi::store_u64(&scratch10, &scratch12, 8));
    builder.emit(abi::compare_immediate(&scratch23, "0"));
    builder.emit(abi::branch_gt(&order_decrement));
    builder.emit(abi::add_immediate(&scratch23, &scratch23, 1));
    builder.emit(abi::branch(&order_loop));
    builder.emit(abi::label(&order_decrement));
    builder.emit(abi::subtract_immediate(&scratch23, &scratch23, 1));
    builder.emit(abi::branch(&order_loop));
    builder.emit(abi::label(&order_no_swap));
    builder.emit(abi::add_immediate(&scratch23, &scratch23, 1));
    builder.emit(abi::branch(&order_loop));
    builder.emit(abi::label(&order_done));

    builder.emit(abi::load_u64(&scratch25, abi::stack_pointer(), temp_slot));
    builder.emit(abi::load_u64(
        &scratch21,
        abi::stack_pointer(),
        scalar_count_slot,
    ));
    builder.emit(abi::move_immediate(&scratch23, "Integer", "0"));
    builder.emit(abi::move_immediate(&scratch24, "Integer", "0"));
    builder.emit(abi::move_immediate(&scratch26, "Integer", "0"));
    builder.emit(abi::move_immediate(&scratch27, "Integer", "0"));
    builder.emit(abi::move_immediate(&scratch28, "Integer", "0"));
    builder.emit(abi::label(&compose_loop));
    builder.emit(abi::compare_registers(&scratch23, &scratch21));
    builder.emit(abi::branch_ge(&compose_next));
    builder.emit(abi::shift_left_immediate(&scratch12, &scratch23, 3));
    builder.emit(abi::add_registers(&scratch12, &scratch25, &scratch12));
    builder.emit(abi::load_u64(&scratch10, &scratch12, 0));
    builder.emit_unicode_property_lookup(&scratch10, &scratch13);
    builder.emit_unicode_property_combining_class(&scratch13, &scratch15);
    builder.emit(abi::compare_immediate(&scratch26, "0"));
    builder.emit(abi::branch_eq(&compose_no_starter));
    builder.emit(abi::compare_immediate(&scratch15, "0"));
    builder.emit(abi::branch_eq(&compose_try));
    builder.emit(abi::compare_registers(&scratch15, &scratch28));
    builder.emit(abi::branch_hi(&compose_try));
    builder.emit(abi::branch(&compose_write));
    builder.emit(abi::label(&compose_try));
    builder.emit(abi::shift_left_immediate(&scratch12, &scratch27, 3));
    builder.emit(abi::add_registers(&scratch12, &scratch25, &scratch12));
    builder.emit(abi::load_u64(&scratch11, &scratch12, 0));
    builder.emit_hangul_composition_attempt(
        &scratch11,
        &scratch10,
        &scratch14,
        &compose_found_direct,
        &compose_try_tables,
    );
    builder.emit(abi::label(&compose_try_tables));
    builder.emit_unicode_property_lookup(&scratch11, &scratch13);
    builder.emit_unicode_property_comb_index(&scratch13, &scratch16);
    builder.emit_unicode_property_comb_length(&scratch13, &scratch17);
    builder.emit_unicode_property_lookup(&scratch10, &scratch13);
    builder.emit_unicode_property_flags(&scratch13, &scratch9);
    // x13/x9 are dead here (both consumed by the property extraction just
    // above); use them — not x6/x7: ABI registers stay physical and the
    // x86 remap role-colors them (x6 and x7 both collapsed
    // onto rax, so the scan pointer lost its table base).
    builder.emit(abi::move_immediate(&scratch13, "Integer", "1023"));
    builder.emit(abi::compare_registers(&scratch16, &scratch13));
    builder.emit(abi::branch_ge(&compose_write));
    builder.emit(abi::move_immediate(&scratch13, "Integer", "1"));
    builder.emit(abi::and_registers(&scratch9, &scratch9, &scratch13));
    builder.emit(abi::compare_immediate(&scratch9, "0"));
    builder.emit(abi::branch_eq(&compose_write));
    builder.emit_load_data_address(&scratch13, UNICODE_COMBINATIONS_SECOND_SYMBOL);
    builder.emit(abi::shift_left_immediate(&scratch9, &scratch16, 2));
    builder.emit(abi::add_registers(&scratch13, &scratch13, &scratch9));
    builder.emit_load_data_address(&scratch8, UNICODE_COMBINATIONS_COMBINED_SYMBOL);
    builder.emit(abi::add_registers(&scratch8, &scratch8, &scratch9));
    builder.emit(abi::label(&compose_scan_loop));
    builder.emit(abi::compare_immediate(&scratch17, "0"));
    builder.emit(abi::branch_eq(&compose_write));
    builder.emit(abi::load_u32(&scratch14, &scratch13, 0));
    builder.emit(abi::compare_registers(&scratch14, &scratch10));
    builder.emit(abi::branch_eq(&compose_found));
    builder.emit(abi::branch_hi(&compose_write));
    builder.emit(abi::add_immediate(&scratch13, &scratch13, 4));
    builder.emit(abi::add_immediate(&scratch8, &scratch8, 4));
    builder.emit(abi::subtract_immediate(&scratch17, &scratch17, 1));
    builder.emit(abi::branch(&compose_scan_loop));
    builder.emit(abi::label(&compose_found));
    builder.emit(abi::load_u32(&scratch14, &scratch8, 0));
    builder.emit(abi::label(&compose_found_direct));
    builder.emit(abi::shift_left_immediate(&scratch12, &scratch27, 3));
    builder.emit(abi::add_registers(&scratch12, &scratch25, &scratch12));
    builder.emit(abi::store_u64(&scratch14, &scratch12, 0));
    builder.emit(abi::add_immediate(&scratch23, &scratch23, 1));
    builder.emit(abi::branch(&compose_loop));
    builder.emit(abi::label(&compose_no_starter));
    builder.emit(abi::label(&compose_write));
    builder.emit(abi::shift_left_immediate(&scratch12, &scratch24, 3));
    builder.emit(abi::add_registers(&scratch12, &scratch25, &scratch12));
    builder.emit(abi::store_u64(&scratch10, &scratch12, 0));
    builder.emit(abi::compare_immediate(&scratch15, "0"));
    builder.emit(abi::branch_ne(&compose_nonstarter));
    builder.emit(abi::move_immediate(&scratch26, "Integer", "1"));
    builder.emit(abi::move_register(&scratch27, &scratch24));
    builder.emit(abi::move_immediate(&scratch28, "Integer", "0"));
    builder.emit(abi::branch(&compose_nonstarter_done));
    builder.emit(abi::label(&compose_nonstarter));
    builder.emit(abi::compare_registers(&scratch15, &scratch28));
    builder.emit(abi::branch_hi(&compose_nonstarter_update));
    builder.emit(abi::branch(&compose_nonstarter_done));
    builder.emit(abi::label(&compose_nonstarter_update));
    builder.emit(abi::move_register(&scratch28, &scratch15));
    builder.emit(abi::label(&compose_nonstarter_done));
    builder.emit(abi::add_immediate(&scratch24, &scratch24, 1));
    builder.emit(abi::add_immediate(&scratch23, &scratch23, 1));
    builder.emit(abi::branch(&compose_loop));
    builder.emit(abi::label(&compose_next));
    builder.emit(abi::store_u64(
        &scratch24,
        abi::stack_pointer(),
        composed_count_slot,
    ));

    builder.emit(abi::move_immediate(&scratch23, "Integer", "0"));
    builder.emit(abi::move_immediate(&scratch24, "Integer", "0"));
    builder.emit(abi::label(&byte_len_loop));
    builder.emit(abi::load_u64(
        &scratch21,
        abi::stack_pointer(),
        composed_count_slot,
    ));
    builder.emit(abi::compare_registers(&scratch23, &scratch21));
    builder.emit(abi::branch_ge(&byte_len_done));
    builder.emit(abi::shift_left_immediate(&scratch12, &scratch23, 3));
    builder.emit(abi::load_u64(&scratch25, abi::stack_pointer(), temp_slot));
    builder.emit(abi::add_registers(&scratch12, &scratch25, &scratch12));
    builder.emit(abi::load_u64(&scratch10, &scratch12, 0));
    builder.emit_utf8_encoded_width(&scratch10, &scratch11);
    builder.emit(abi::add_registers(&scratch24, &scratch24, &scratch11));
    builder.emit(abi::add_immediate(&scratch23, &scratch23, 1));
    builder.emit(abi::branch(&byte_len_loop));
    builder.emit(abi::label(&byte_len_done));
    builder.emit(abi::store_u64(
        &scratch24,
        abi::stack_pointer(),
        output_len_slot,
    ));

    // bug-378: header (+9) add routed through the checked helper so a
    // pathological composed byte length cannot wrap the allocation size,
    // matching every sibling string builder (case-map, graphemes, ...).
    let size_overflow = builder.label("strings_nfc_size_overflow");
    builder.emit_checked_size_add_immediate(abi::return_register(), &scratch24, 9, &size_overflow);
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
    builder.emit_arena_alloc_call();
    builder.emit(abi::branch_eq(&result_alloc_ok));
    builder.raise_error_bare("ErrOutOfMemory")?;
    builder.emit(abi::label(&size_overflow));
    builder.raise_error_bare("ErrOutOfMemory")?;
    builder.emit(abi::label(&result_alloc_ok));
    builder.emit(abi::store_u64(
        abi::mfb_return(1),
        abi::stack_pointer(),
        result_slot,
    ));
    builder.emit(abi::load_u64(
        &scratch24,
        abi::stack_pointer(),
        output_len_slot,
    ));
    builder.emit(abi::store_u64(&scratch24, abi::mfb_return(1), 0));
    builder.emit(abi::add_immediate(&scratch28, abi::mfb_return(1), 8));
    builder.emit(abi::move_immediate(&scratch23, "Integer", "0"));
    builder.emit(abi::label(&encode_loop));
    builder.emit(abi::load_u64(
        &scratch21,
        abi::stack_pointer(),
        composed_count_slot,
    ));
    builder.emit(abi::compare_registers(&scratch23, &scratch21));
    builder.emit(abi::branch_ge(&encode_done));
    builder.emit(abi::shift_left_immediate(&scratch12, &scratch23, 3));
    builder.emit(abi::load_u64(&scratch25, abi::stack_pointer(), temp_slot));
    builder.emit(abi::add_registers(&scratch12, &scratch25, &scratch12));
    builder.emit(abi::load_u64(&scratch10, &scratch12, 0));
    builder.emit_utf8_encode_next(&scratch28, &scratch10);
    builder.emit(abi::add_immediate(&scratch23, &scratch23, 1));
    builder.emit(abi::branch(&encode_loop));
    builder.emit(abi::label(&encode_done));
    // audit-unicode #9: the encode pass must end exactly at the byte length
    // the counting pass allocated; a divergence is a silent heap overflow.
    builder.emit(abi::load_u64(&scratch10, abi::stack_pointer(), result_slot));
    builder.emit(abi::load_u64(
        &scratch11,
        abi::stack_pointer(),
        output_len_slot,
    ));
    builder.emit(abi::add_registers(&scratch10, &scratch10, &scratch11));
    builder.emit(abi::add_immediate(&scratch10, &scratch10, 8));
    builder.emit_write_cursor_assert(&scratch28, &scratch10, "strings_nfc");
    builder.emit(abi::move_immediate(&scratch10, "Integer", "0"));
    builder.emit(abi::store_u8(&scratch10, &scratch28, 0));

    builder.emit(abi::label(&nfc_done));
    let result = builder.allocate_register()?;
    builder.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
    Ok(ValueResult {
        origin: None,
        type_: "String".to_string(),
        location: Operand::from(result.render()),
        text: "strings.normalizeNfc".to_string(),
    })
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "normalizeNfc",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "value",
                desc: "",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::abi_inline(lower),
        }],
    });
}
