//! Shared padding for `strings::{padLeft,padRight}`.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::target::shared::abi;
use crate::target::shared::nir::*;

pub(crate) fn lower_strings_pad(
    builder: &mut CodeBuilder,
    args: &[NirValue],
    right: bool,
) -> Result<ValueResult, String> {
    let scratch9 = builder.temporary_vreg();
    let scratch13 = builder.temporary_vreg();
    let scratch12 = builder.temporary_vreg();
    let scratch10 = builder.temporary_vreg();
    let scratch17 = builder.temporary_vreg();
    let scratch11 = builder.temporary_vreg();
    let scratch14 = builder.temporary_vreg();
    let scratch16 = builder.temporary_vreg();
    let scratch15 = builder.temporary_vreg();
    let value = builder.lower_value(&args[0])?;
    builder.require_string("strings.pad value", &value)?;
    let value_slot = builder.spill_to_slot("strings_pad_value", &value.location);
    let width = builder.lower_value(&args[1])?;
    if width.type_ != "Integer" {
        return Err(format!(
            "strings.pad width must be Integer, got {}",
            width.type_
        ));
    }
    let width_slot = builder.spill_to_slot("strings_pad_width", &width.location);
    let pad_slot = if args.len() == 3 {
        let pad = builder.lower_value(&args[2])?;
        builder.require_string("strings.pad padChar", &pad)?;
        builder.spill_to_slot("strings_pad_char", &pad.location)
    } else {
        // Default padChar is a single space " ". Materialize a one-byte String
        // (0x20) so the downstream code path is uniform.
        let space_slot = builder.allocate_stack_object("strings_pad_space_byte", 8);
        builder.emit(abi::move_immediate(&scratch9, "Byte", "32"));
        builder.emit(abi::store_u8(&scratch9, abi::stack_pointer(), space_slot));
        builder.emit(abi::add_immediate(
            &scratch13,
            abi::stack_pointer(),
            space_slot,
        ));
        builder.emit(abi::move_immediate(&scratch12, "Integer", "1"));
        let space = builder.emit_materialize_string_from_bytes(&scratch13, &scratch12)?;
        builder.spill_to_slot("strings_pad_char", &space.render())
    };
    // Number of pad chars to prepend/append.
    let pad_count_slot = builder.allocate_stack_object("strings_pad_count", 8);
    // Byte length of one padChar.
    let pad_len_slot = builder.allocate_stack_object("strings_pad_char_len", 8);
    let total_slot = builder.allocate_stack_object("strings_pad_total", 8);
    let result_slot = builder.allocate_stack_object("strings_pad_result", 8);

    let invalid = builder.label("strings_pad_invalid");
    let alloc_ok = builder.label("strings_pad_alloc_ok");

    // Validate width >= 0.
    builder.emit(abi::load_u64(&scratch10, abi::stack_pointer(), width_slot));
    builder.emit(abi::compare_immediate(&scratch10, "0"));
    builder.emit(abi::branch_lt(&invalid));

    // Validate padChar is exactly one scalar (len>0 and scalar count == 1).
    builder.emit(abi::load_u64(&scratch17, abi::stack_pointer(), pad_slot));
    builder.emit(abi::load_u64(&scratch9, &scratch17, 0));
    builder.emit(abi::compare_immediate(&scratch9, "0"));
    builder.emit(abi::branch_eq(&invalid));
    builder.emit(abi::store_u64(
        &scratch9,
        abi::stack_pointer(),
        pad_len_slot,
    ));
    {
        let loop_label = builder.label("strings_pad_scalars_loop");
        let not_cont = builder.label("strings_pad_scalars_not_cont");
        let after = builder.label("strings_pad_scalars_after");
        let done = builder.label("strings_pad_scalars_done");
        builder.emit(abi::add_immediate(&scratch11, &scratch17, 8));
        builder.emit_scalar_count_loop(
            &scratch11,
            &scratch12,
            &scratch14,
            &scratch15,
            &scratch13,
            &scratch16,
            &scratch9,
            &loop_label,
            &not_cont,
            &after,
            &done,
        );
        builder.emit(abi::compare_immediate(&scratch14, "1"));
        builder.emit(abi::branch_ne(&invalid));
        // The count above is byte-structural (non-continuation bytes == 1);
        // additionally require the scalar to be well-formed UTF-8
        // (audit-unicode #7). The validating decoder substitutes U+FFFD with
        // width 1 for any malformed sequence, so a valid single scalar — the
        // only padChar constructible from source — is exactly one that
        // decodes across the whole padChar and re-encodes at the same width.
        builder.emit_utf8_decode_next(&scratch11, &scratch12, &scratch14);
        builder.emit(abi::compare_registers(&scratch14, &scratch9));
        builder.emit(abi::branch_ne(&invalid));
        builder.emit_utf8_encoded_width(&scratch12, &scratch13);
        builder.emit(abi::compare_registers(&scratch13, &scratch9));
        builder.emit(abi::branch_ne(&invalid));
    }

    // Count scalars in value into x14.
    builder.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
    builder.emit(abi::load_u64(&scratch9, &scratch16, 0));
    {
        let loop_label = builder.label("strings_pad_value_loop");
        let not_cont = builder.label("strings_pad_value_not_cont");
        let after = builder.label("strings_pad_value_after");
        let done = builder.label("strings_pad_value_done");
        builder.emit(abi::add_immediate(&scratch11, &scratch16, 8));
        builder.emit_scalar_count_loop(
            &scratch11,
            &scratch12,
            &scratch14,
            &scratch15,
            &scratch13,
            &scratch17,
            &scratch9,
            &loop_label,
            &not_cont,
            &after,
            &done,
        );
    }
    // pad_count = max(0, width - scalarLen).
    builder.emit(abi::load_u64(&scratch10, abi::stack_pointer(), width_slot));
    {
        let no_pad = builder.label("strings_pad_no_pad");
        let have_pad = builder.label("strings_pad_have_pad");
        builder.emit(abi::compare_registers(&scratch10, &scratch14));
        builder.emit(abi::branch_le(&no_pad));
        builder.emit(abi::subtract_registers(&scratch10, &scratch10, &scratch14));
        builder.emit(abi::branch(&have_pad));
        builder.emit(abi::label(&no_pad));
        builder.emit(abi::move_immediate(&scratch10, "Integer", "0"));
        builder.emit(abi::label(&have_pad));
    }
    builder.emit(abi::store_u64(
        &scratch10,
        abi::stack_pointer(),
        pad_count_slot,
    ));

    // total = valueLen + pad_count * padLen, rejecting sizes that do not fit
    // in 64 bits: an unchecked wrap here allocated small while the pad loop
    // wrote the full pad_count*padLen bytes (audit-unicode #2, heap
    // overflow). Unrepresentable widths raise the same catchable 77050002 as
    // the other argument rejections.
    builder.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
    builder.emit(abi::load_u64(&scratch9, &scratch16, 0));
    builder.emit(abi::load_u64(
        &scratch11,
        abi::stack_pointer(),
        pad_len_slot,
    ));
    builder.emit_checked_size_multiply(&scratch12, &scratch10, &scratch11, &invalid);
    builder.emit_checked_size_add(&scratch11, &scratch9, &scratch12, &invalid);
    builder.emit(abi::store_u64(&scratch11, abi::stack_pointer(), total_slot));

    // allocate total + 9.
    builder.emit_checked_size_add_immediate(abi::return_register(), &scratch11, 9, &invalid);
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
    builder.emit_arena_alloc_call();
    builder.emit(abi::branch_eq(&alloc_ok));
    builder.raise_error_bare("ErrOutOfMemory")?;
    builder.emit(abi::label(&alloc_ok));
    builder.emit(abi::store_u64(
        abi::mfb_return(1),
        abi::stack_pointer(),
        result_slot,
    ));
    builder.emit(abi::load_u64(&scratch11, abi::stack_pointer(), total_slot));
    builder.emit(abi::store_u64(&scratch11, abi::mfb_return(1), 0));

    // Write the output. Carry the result pointer in a vreg rather than
    // holding the arena_alloc result register across the copy (the
    // concat/split pattern). Copy-loop scratch is minted as vregs too.
    let out_ptr_v = builder.temporary_vreg();
    let out_ptr = &out_ptr_v;
    let pad_src_v = builder.temporary_vreg();
    let pad_cnt_v = builder.temporary_vreg();
    let byte_v = builder.temporary_vreg();
    let pad_src = &pad_src_v;
    let pad_cnt = &pad_cnt_v;
    let byte = &byte_v;
    builder.emit(abi::load_u64(out_ptr, abi::stack_pointer(), result_slot));
    builder.emit(abi::add_immediate(&scratch13, out_ptr, 8));

    let copy_value = |b: &mut CodeBuilder| {
        // copy value bytes (x14 base, x9 len) to x13.
        b.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
        b.emit(abi::load_u64(&scratch9, &scratch16, 0));
        b.emit(abi::add_immediate(&scratch14, &scratch16, 8));
        let loop_label = b.label("strings_pad_copy_value_loop");
        let done = b.label("strings_pad_copy_value_done");
        b.emit(abi::label(&loop_label));
        b.emit(abi::compare_immediate(&scratch9, "0"));
        b.emit(abi::branch_eq(&done));
        b.emit(abi::load_u8(byte, &scratch14, 0));
        b.emit(abi::store_u8(byte, &scratch13, 0));
        b.emit(abi::add_immediate(&scratch14, &scratch14, 1));
        b.emit(abi::add_immediate(&scratch13, &scratch13, 1));
        b.emit(abi::subtract_immediate(&scratch9, &scratch9, 1));
        b.emit(abi::branch(&loop_label));
        b.emit(abi::label(&done));
    };
    let copy_pads = |b: &mut CodeBuilder, tag: &str| {
        // write pad_count copies of padChar (x14 base, x11 len) to x13.
        b.emit(abi::load_u64(
            &scratch10,
            abi::stack_pointer(),
            pad_count_slot,
        ));
        b.emit(abi::load_u64(&scratch17, abi::stack_pointer(), pad_slot));
        b.emit(abi::add_immediate(&scratch14, &scratch17, 8));
        b.emit(abi::load_u64(
            &scratch11,
            abi::stack_pointer(),
            pad_len_slot,
        ));
        let outer = b.label(&format!("strings_pad_{tag}_outer"));
        let outer_done = b.label(&format!("strings_pad_{tag}_outer_done"));
        let inner = b.label(&format!("strings_pad_{tag}_inner"));
        let inner_done = b.label(&format!("strings_pad_{tag}_inner_done"));
        b.emit(abi::label(&outer));
        b.emit(abi::compare_immediate(&scratch10, "0"));
        b.emit(abi::branch_eq(&outer_done));
        b.emit(abi::move_register(pad_src, &scratch14));
        b.emit(abi::move_register(pad_cnt, &scratch11));
        b.emit(abi::label(&inner));
        b.emit(abi::compare_immediate(pad_cnt, "0"));
        b.emit(abi::branch_eq(&inner_done));
        b.emit(abi::load_u8(byte, pad_src, 0));
        b.emit(abi::store_u8(byte, &scratch13, 0));
        b.emit(abi::add_immediate(pad_src, pad_src, 1));
        b.emit(abi::add_immediate(&scratch13, &scratch13, 1));
        b.emit(abi::subtract_immediate(pad_cnt, pad_cnt, 1));
        b.emit(abi::branch(&inner));
        b.emit(abi::label(&inner_done));
        b.emit(abi::subtract_immediate(&scratch10, &scratch10, 1));
        b.emit(abi::branch(&outer));
        b.emit(abi::label(&outer_done));
    };

    if right {
        copy_value(builder);
        copy_pads(builder, "right");
    } else {
        copy_pads(builder, "left");
        copy_value(builder);
    }
    builder.emit(abi::move_immediate(byte, "Integer", "0"));
    builder.emit(abi::store_u8(byte, &scratch13, 0));
    let result = builder.allocate_register()?;
    builder.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
    let after = builder.label("strings_pad_after");
    builder.emit(abi::branch(&after));
    builder.emit(abi::label(&invalid));
    builder.raise_error_bare("ErrInvalidArgument")?;
    builder.emit(abi::label(&after));
    let label = if right {
        "strings.padRight"
    } else {
        "strings.padLeft"
    };
    Ok(ValueResult {
        type_: "String".to_string(),
        location: Operand::from(result.render()),
        text: label.to_string(),
    })
}
