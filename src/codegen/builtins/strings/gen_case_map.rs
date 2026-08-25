//! Shared clean-room codegen for `strings::{upper,lower,caseFold}` — Unicode case mapping.

// --- codegen tier imports (migration) ---
use crate::codegen::builtins::strings::UnicodeCaseMap;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::target::shared::abi;
use crate::types::ParameterType;

fn emit_ascii_case_transform(
    builder: &mut CodeBuilder,
    map: UnicodeCaseMap,
    reg: impl Into<Operand>,
) {
    let reg = reg.into();
    let skip = builder.label("strings_case_map_ascii_skip");
    match map {
        UnicodeCaseMap::Upper => {
            builder.emit(abi::compare_immediate(reg.clone(), "97")); // 'a'
            builder.emit(abi::branch_lt(&skip));
            builder.emit(abi::compare_immediate(reg.clone(), "122")); // 'z'
            builder.emit(abi::branch_gt(&skip));
            builder.emit(abi::subtract_immediate(reg.clone(), reg, 32));
        }
        UnicodeCaseMap::Lower | UnicodeCaseMap::CaseFold => {
            builder.emit(abi::compare_immediate(reg.clone(), "65")); // 'A'
            builder.emit(abi::branch_lt(&skip));
            builder.emit(abi::compare_immediate(reg.clone(), "90")); // 'Z'
            builder.emit(abi::branch_gt(&skip));
            builder.emit(abi::add_immediate(reg.clone(), reg, 32));
        }
    }
    builder.emit(abi::label(&skip));
}

pub(crate) fn lower_strings_case_map(
    builder: &mut CodeBuilder,
    value: &ValueResult,
    map: UnicodeCaseMap,
) -> Result<ValueResult, String> {
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
    let scratch28 = builder.temporary_vreg();
    let value = value.clone();
    builder.require_string(map.label(), &value)?;
    let value_slot = builder.spill_to_slot(map.slot_prefix(), &value.location);
    let length_slot = builder.allocate_stack_object("strings_case_map_length", 8);
    let width_slot = builder.allocate_stack_object("strings_case_map_width", 8);
    let result_slot = builder.allocate_stack_object("strings_case_map_result", 8);

    let count_loop = builder.label("strings_case_map_count_loop");
    let count_nonascii = builder.label("strings_case_map_count_nonascii");
    let count_identity = builder.label("strings_case_map_count_identity");
    let count_sequence = builder.label("strings_case_map_count_sequence");
    let count_sequence_loop = builder.label("strings_case_map_count_sequence_loop");
    let count_next = builder.label("strings_case_map_count_next");
    let count_done = builder.label("strings_case_map_count_done");
    let alloc_ok = builder.label("strings_case_map_alloc_ok");
    let write_loop = builder.label("strings_case_map_write_loop");
    let write_nonascii = builder.label("strings_case_map_write_nonascii");
    let write_identity = builder.label("strings_case_map_write_identity");
    let write_sequence = builder.label("strings_case_map_write_sequence");
    let write_sequence_loop = builder.label("strings_case_map_write_sequence_loop");
    let write_next = builder.label("strings_case_map_write_next");
    let write_done = builder.label("strings_case_map_write_done");
    let ascii_scan = builder.label("strings_case_map_ascii_scan");
    let ascii_scan_byte = builder.label("strings_case_map_ascii_scan_byte");
    let ascii_transform = builder.label("strings_case_map_ascii_transform");
    let ascii_size_overflow = builder.label("strings_case_map_ascii_size_overflow");
    let ascii_alloc_ok = builder.label("strings_case_map_ascii_alloc_ok");
    let ascii_transform_loop = builder.label("strings_case_map_ascii_transform_loop");
    let ascii_transform_done = builder.label("strings_case_map_ascii_transform_done");
    let case_slow = builder.label("strings_case_map_slow");
    let case_done = builder.label("strings_case_map_done");

    // F1 (plan-64): whole-string ASCII quick-check, mirroring normalizeNfc's
    // E2 shortcut. When every byte is < 0x80 the string is pure ASCII: case
    // folding maps a-z/A-Z by +/-32 with a 1-byte-in/1-byte-out width, so a
    // single decode-free pass suffices — skipping the two-pass UTF-8 decode
    // (count then write) and the per-codepoint Unicode case-table search.
    // Bit-identical to the slow path for ASCII input (same
    // emit_ascii_case_transform, same byte_len + 9 allocation); any byte
    // >= 0x80 falls through to the slow path below.
    builder.emit(abi::load_u64(&scratch20, abi::stack_pointer(), value_slot));
    builder.emit(abi::load_u64(&scratch21, &scratch20, 0));
    builder.emit(abi::add_immediate(&scratch22, &scratch20, 8));
    // plan-86 F3: SWAR quick-check — test 8 bytes at a time for a high bit
    // (0x8080808080808080 = 9259542123273814144); any set high bit means some
    // byte >= 0x80 → the Unicode slow path. Byte-exact-equivalent to the per-byte
    // `compare 128`, ~8× fewer iterations. A <8-byte tail falls to the byte loop.
    builder.emit(abi::move_immediate(&scratch23, "Integer", "0"));
    builder.emit(abi::move_immediate(
        &scratch24,
        "Integer",
        "9259542123273814144",
    ));
    builder.emit(abi::label(&ascii_scan));
    builder.emit(abi::subtract_registers(&scratch27, &scratch21, &scratch23));
    builder.emit(abi::compare_immediate(&scratch27, "8"));
    builder.emit(abi::branch_lo(&ascii_scan_byte));
    builder.emit(abi::add_registers(&scratch14, &scratch22, &scratch23));
    builder.emit(abi::load_u64(&scratch10, &scratch14, 0));
    builder.emit(abi::and_registers(&scratch10, &scratch10, &scratch24));
    builder.emit(abi::compare_immediate(&scratch10, "0"));
    builder.emit(abi::branch_ne(&case_slow));
    builder.emit(abi::add_immediate(&scratch23, &scratch23, 8));
    builder.emit(abi::branch(&ascii_scan));
    builder.emit(abi::label(&ascii_scan_byte));
    builder.emit(abi::compare_registers(&scratch23, &scratch21));
    builder.emit(abi::branch_ge(&ascii_transform));
    builder.emit(abi::add_registers(&scratch14, &scratch22, &scratch23));
    builder.emit(abi::load_u8(&scratch10, &scratch14, 0));
    builder.emit(abi::compare_immediate(&scratch10, "128"));
    builder.emit(abi::branch_ge(&case_slow));
    builder.emit(abi::add_immediate(&scratch23, &scratch23, 1));
    builder.emit(abi::branch(&ascii_scan_byte));

    // Pure ASCII: allocate byte_len + 9 (8-byte header + trailing NUL),
    // matching the slow path's layout, then transform-copy in one pass.
    builder.emit(abi::label(&ascii_transform));
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
    // ARENA_ALLOC clobbers caller-saved registers; reload source ptr/len.
    builder.emit(abi::load_u64(&scratch20, abi::stack_pointer(), value_slot));
    builder.emit(abi::load_u64(&scratch21, &scratch20, 0));
    builder.emit(abi::add_immediate(&scratch22, &scratch20, 8));
    builder.emit(abi::store_u64(&scratch21, abi::mfb_return(1), 0));
    builder.emit(abi::add_immediate(&scratch28, abi::mfb_return(1), 8));
    builder.emit(abi::move_immediate(&scratch23, "Integer", "0"));
    builder.emit(abi::label(&ascii_transform_loop));
    builder.emit(abi::compare_registers(&scratch23, &scratch21));
    builder.emit(abi::branch_ge(&ascii_transform_done));
    builder.emit(abi::add_registers(&scratch14, &scratch22, &scratch23));
    builder.emit(abi::load_u8(&scratch10, &scratch14, 0));
    emit_ascii_case_transform(builder, map, &scratch10);
    builder.emit(abi::store_u8(&scratch10, &scratch28, 0));
    builder.emit(abi::add_immediate(&scratch28, &scratch28, 1));
    builder.emit(abi::add_immediate(&scratch23, &scratch23, 1));
    builder.emit(abi::branch(&ascii_transform_loop));
    builder.emit(abi::label(&ascii_transform_done));
    builder.emit(abi::move_immediate(&scratch10, "Integer", "0"));
    builder.emit(abi::store_u8(&scratch10, &scratch28, 0));
    builder.emit(abi::branch(&case_done));

    builder.emit(abi::label(&case_slow));
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
    // E1 (plan-39): ASCII fast path. For codepoints < 0x80 the case tables
    // only ever map a-z/A-Z to a single ASCII codepoint (1 byte in, 1 byte
    // out), so skip the ~11-deep Unicode-table binary search entirely and
    // count exactly one output byte.
    builder.emit(abi::compare_immediate(&scratch10, "128"));
    builder.emit(abi::branch_ge(&count_nonascii));
    builder.emit(abi::add_immediate(&scratch24, &scratch24, 1));
    builder.emit(abi::branch(&count_next));
    builder.emit(abi::label(&count_nonascii));
    builder.emit_case_map_lookup(map, &scratch10, &scratch26, &scratch27);
    builder.emit(abi::compare_immediate(&scratch27, "0"));
    builder.emit(abi::branch_eq(&count_identity));
    builder.emit(abi::branch(&count_sequence));
    builder.emit(abi::label(&count_identity));
    // bug-175 B: size the count from the *re-encoded* width of the decoded
    // codepoint (what the write pass emits via emit_utf8_encode_next), not the
    // original decode byte width `scratch11`. For malformed input the two
    // differ (e.g. U+FFFD encodes to 3 bytes), so counting the original width
    // would under-allocate; NFC already sizes from the re-encoded width.
    builder.emit_utf8_encoded_width(&scratch10, &scratch13);
    builder.emit(abi::add_registers(&scratch24, &scratch24, &scratch13));
    builder.emit(abi::branch(&count_next));
    builder.emit(abi::label(&count_sequence));
    builder.emit(abi::label(&count_sequence_loop));
    builder.emit(abi::compare_immediate(&scratch27, "0"));
    builder.emit(abi::branch_eq(&count_next));
    builder.emit(abi::load_u32(&scratch10, &scratch26, 0));
    builder.emit(abi::add_immediate(&scratch26, &scratch26, 4));
    builder.emit_utf8_encoded_width(&scratch10, &scratch13);
    builder.emit(abi::add_registers(&scratch24, &scratch24, &scratch13));
    builder.emit(abi::subtract_immediate(&scratch27, &scratch27, 1));
    builder.emit(abi::branch(&count_sequence_loop));
    builder.emit(abi::label(&count_next));
    builder.emit(abi::load_u64(&scratch11, abi::stack_pointer(), width_slot));
    builder.emit(abi::add_registers(&scratch23, &scratch23, &scratch11));
    builder.emit(abi::branch(&count_loop));
    builder.emit(abi::label(&count_done));
    builder.emit(abi::store_u64(
        &scratch24,
        abi::stack_pointer(),
        length_slot,
    ));

    // bug-175 B: header (+9) add routed through the checked helper so a
    // pathological byte length cannot wrap the allocation size.
    let size_overflow = builder.label("strings_case_map_size_overflow");
    builder.emit_checked_size_add_immediate(abi::return_register(), &scratch24, 9, &size_overflow);
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
    builder.emit_arena_alloc_call();
    builder.emit(abi::branch_eq(&alloc_ok));
    builder.raise_error_bare("ErrOutOfMemory")?;
    builder.emit(abi::label(&size_overflow));
    builder.raise_error_bare("ErrOutOfMemory")?;
    builder.emit(abi::label(&alloc_ok));
    builder.emit(abi::store_u64(
        abi::mfb_return(1),
        abi::stack_pointer(),
        result_slot,
    ));
    builder.emit(abi::load_u64(&scratch24, abi::stack_pointer(), length_slot));
    builder.emit(abi::store_u64(&scratch24, abi::mfb_return(1), 0));

    builder.emit(abi::load_u64(&scratch20, abi::stack_pointer(), value_slot));
    builder.emit(abi::load_u64(&scratch21, &scratch20, 0));
    builder.emit(abi::add_immediate(&scratch22, &scratch20, 8));
    builder.emit(abi::move_immediate(&scratch23, "Integer", "0"));
    builder.emit(abi::load_u64(&scratch28, abi::stack_pointer(), result_slot));
    builder.emit(abi::add_immediate(&scratch28, &scratch28, 8));
    builder.emit(abi::label(&write_loop));
    builder.emit(abi::compare_registers(&scratch23, &scratch21));
    builder.emit(abi::branch_ge(&write_done));
    builder.emit(abi::add_registers(&scratch14, &scratch22, &scratch23));
    builder.emit_utf8_decode_next(&scratch14, &scratch10, &scratch11);
    builder.emit(abi::store_u64(&scratch11, abi::stack_pointer(), width_slot));
    // E1 (plan-39): ASCII fast path mirroring the count pass — range-map the
    // codepoint (a-z/A-Z ±32) and re-encode the single byte directly, so
    // ASCII case folding never touches the Unicode case table.
    builder.emit(abi::compare_immediate(&scratch10, "128"));
    builder.emit(abi::branch_ge(&write_nonascii));
    emit_ascii_case_transform(builder, map, &scratch10);
    builder.emit_utf8_encode_next(&scratch28, &scratch10);
    builder.emit(abi::branch(&write_next));
    builder.emit(abi::label(&write_nonascii));
    builder.emit_case_map_lookup(map, &scratch10, &scratch26, &scratch27);
    builder.emit(abi::compare_immediate(&scratch27, "0"));
    builder.emit(abi::branch_eq(&write_identity));
    builder.emit(abi::branch(&write_sequence));
    builder.emit(abi::label(&write_identity));
    builder.emit_utf8_encode_next(&scratch28, &scratch10);
    builder.emit(abi::branch(&write_next));
    builder.emit(abi::label(&write_sequence));
    builder.emit(abi::label(&write_sequence_loop));
    builder.emit(abi::compare_immediate(&scratch27, "0"));
    builder.emit(abi::branch_eq(&write_next));
    builder.emit(abi::load_u32(&scratch10, &scratch26, 0));
    builder.emit(abi::add_immediate(&scratch26, &scratch26, 4));
    builder.emit_utf8_encode_next(&scratch28, &scratch10);
    builder.emit(abi::subtract_immediate(&scratch27, &scratch27, 1));
    builder.emit(abi::branch(&write_sequence_loop));
    builder.emit(abi::label(&write_next));
    builder.emit(abi::load_u64(&scratch11, abi::stack_pointer(), width_slot));
    builder.emit(abi::add_registers(&scratch23, &scratch23, &scratch11));
    builder.emit(abi::branch(&write_loop));
    builder.emit(abi::label(&write_done));
    // audit-unicode #9: the write pass must end exactly at the byte length
    // the counting pass allocated; a divergence is a silent heap overflow.
    builder.emit(abi::load_u64(&scratch10, abi::stack_pointer(), result_slot));
    builder.emit(abi::load_u64(&scratch11, abi::stack_pointer(), length_slot));
    builder.emit(abi::add_registers(&scratch10, &scratch10, &scratch11));
    builder.emit(abi::add_immediate(&scratch10, &scratch10, 8));
    builder.emit_write_cursor_assert(&scratch28, &scratch10, "strings_case_map");
    builder.emit(abi::move_immediate(&scratch10, "Integer", "0"));
    builder.emit(abi::store_u8(&scratch10, &scratch28, 0));

    builder.emit(abi::label(&case_done));
    let result = builder.allocate_register();
    builder.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::String,
        location: Operand::from(result.render()),
        text: map.name().to_string(),
    })
}
