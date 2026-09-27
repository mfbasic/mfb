//! Shared whitespace trim for `strings::{trim,trimStart,trimEnd}`.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::target::shared::abi;
use crate::types::ParameterType;

/// bug-732: trim trailing whitespace by stepping BACK from the end, one whitespace
/// scalar at a time, so the cost is the whitespace removed, not the string's
/// length. `end_slot` holds the current end (the string length on entry) and
/// never passes below `start_slot`.
///
/// The whitespace set is `emit_unicode_whitespace_branch`'s: ASCII 9–13 and 32
/// (1 byte), U+0085 and U+00A0 (`C2 85`, `C2 A0`), U+1680 (`E1 9A 80`),
/// U+2000–U+200A, U+2028, U+2029, U+202F (`E2 80 80`–`8A`, `A8`, `A9`, `AF`),
/// U+205F (`E2 81 9F`) and U+3000 (`E3 80 80`). Every multi-byte sequence starts
/// with a lead byte and continues with bytes `80`–`BF`, which no sequence starts
/// with, so the sequence that ends at `end` is found by its last 1–3 bytes alone —
/// the same decomposition the forward scan this replaces reached from the front.
fn emit_trim_end_backward(
    builder: &mut CodeBuilder,
    value_slot: usize,
    start_slot: usize,
    end_slot: usize,
) {
    let base = builder.temporary_vreg();
    let start = builder.temporary_vreg();
    let end = builder.temporary_vreg();
    let room = builder.temporary_vreg();
    let last = builder.temporary_vreg();
    let lead = builder.temporary_vreg();
    let mid = builder.temporary_vreg();
    let at = builder.temporary_vreg();
    let loop_label = builder.label("strings_trim_end_loop");
    let three = builder.label("strings_trim_end_three");
    let e1 = builder.label("strings_trim_end_e1");
    let e2 = builder.label("strings_trim_end_e2");
    let e2_80 = builder.label("strings_trim_end_e2_80");
    let e3 = builder.label("strings_trim_end_e3");
    let back1 = builder.label("strings_trim_end_back1");
    let back2 = builder.label("strings_trim_end_back2");
    let back3 = builder.label("strings_trim_end_back3");
    let done = builder.label("strings_trim_end_done");

    builder.emit(abi::load_u64(&base, abi::stack_pointer(), value_slot));
    builder.emit(abi::add_immediate(&base, &base, 8));
    builder.emit(abi::load_u64(&start, abi::stack_pointer(), start_slot));
    builder.emit(abi::load_u64(&end, abi::stack_pointer(), end_slot));
    builder.emit(abi::label(&loop_label));
    // room = end - start: how many bytes a sequence ending at `end` may span.
    builder.emit(abi::subtract_registers(&room, &end, &start));
    builder.emit(abi::compare_immediate(&room, "0"));
    builder.emit(abi::branch_le(&done));
    // `at` = the byte three before `end`, so the sequence's bytes are at `at + 0..3`
    // (only the ones `room` admits are read).
    builder.emit(abi::add_registers(&at, &base, &end));
    builder.emit(abi::subtract_immediate(&at, &at, 3));
    builder.emit(abi::load_u8(&last, &at, 2));
    // One byte: 9..=13 or 32.
    builder.emit(abi::compare_immediate(&last, "32"));
    builder.emit(abi::branch_eq(&back1));
    builder.emit(abi::compare_immediate(&last, "9"));
    builder.emit(abi::branch_lo(&done));
    builder.emit(abi::compare_immediate(&last, "13"));
    builder.emit(abi::branch_le(&back1));
    // Anything below 0x80 that is not whitespace ends the scan; so does a byte
    // that cannot end a multi-byte sequence.
    builder.emit(abi::compare_immediate(&last, "128"));
    builder.emit(abi::branch_lo(&done));
    // Two bytes: C2 85, C2 A0.
    builder.emit(abi::compare_immediate(&room, "2"));
    builder.emit(abi::branch_lo(&done));
    builder.emit(abi::load_u8(&lead, &at, 1));
    builder.emit(abi::compare_immediate(&lead, "194"));
    builder.emit(abi::branch_ne(&three));
    builder.emit(abi::compare_immediate(&last, "133"));
    builder.emit(abi::branch_eq(&back2));
    builder.emit(abi::compare_immediate(&last, "160"));
    builder.emit(abi::branch_eq(&back2));
    builder.emit(abi::branch(&done));
    // Three bytes: `lead mid last`.
    builder.emit(abi::label(&three));
    builder.emit(abi::compare_immediate(&room, "3"));
    builder.emit(abi::branch_lo(&done));
    builder.emit(abi::load_u8(&lead, &at, 0));
    builder.emit(abi::load_u8(&mid, &at, 1));
    builder.emit(abi::compare_immediate(&lead, "225"));
    builder.emit(abi::branch_eq(&e1));
    builder.emit(abi::compare_immediate(&lead, "226"));
    builder.emit(abi::branch_eq(&e2));
    builder.emit(abi::compare_immediate(&lead, "227"));
    builder.emit(abi::branch_eq(&e3));
    builder.emit(abi::branch(&done));
    // E1 9A 80 (U+1680).
    builder.emit(abi::label(&e1));
    builder.emit(abi::compare_immediate(&mid, "154"));
    builder.emit(abi::branch_ne(&done));
    builder.emit(abi::compare_immediate(&last, "128"));
    builder.emit(abi::branch_eq(&back3));
    builder.emit(abi::branch(&done));
    // E2 80 xx and E2 81 9F.
    builder.emit(abi::label(&e2));
    builder.emit(abi::compare_immediate(&mid, "128"));
    builder.emit(abi::branch_eq(&e2_80));
    builder.emit(abi::compare_immediate(&mid, "129"));
    builder.emit(abi::branch_ne(&done));
    builder.emit(abi::compare_immediate(&last, "159"));
    builder.emit(abi::branch_eq(&back3));
    builder.emit(abi::branch(&done));
    // E2 80 80..=8A, A8, A9, AF (U+2000–U+200A, U+2028, U+2029, U+202F).
    builder.emit(abi::label(&e2_80));
    builder.emit(abi::compare_immediate(&last, "138"));
    builder.emit(abi::branch_ls(&back3));
    builder.emit(abi::compare_immediate(&last, "168"));
    builder.emit(abi::branch_eq(&back3));
    builder.emit(abi::compare_immediate(&last, "169"));
    builder.emit(abi::branch_eq(&back3));
    builder.emit(abi::compare_immediate(&last, "175"));
    builder.emit(abi::branch_eq(&back3));
    builder.emit(abi::branch(&done));
    // E3 80 80 (U+3000).
    builder.emit(abi::label(&e3));
    builder.emit(abi::compare_immediate(&mid, "128"));
    builder.emit(abi::branch_ne(&done));
    builder.emit(abi::compare_immediate(&last, "128"));
    builder.emit(abi::branch_eq(&back3));
    builder.emit(abi::branch(&done));
    builder.emit(abi::label(&back1));
    builder.emit(abi::subtract_immediate(&end, &end, 1));
    builder.emit(abi::branch(&loop_label));
    builder.emit(abi::label(&back2));
    builder.emit(abi::subtract_immediate(&end, &end, 2));
    builder.emit(abi::branch(&loop_label));
    builder.emit(abi::label(&back3));
    builder.emit(abi::subtract_immediate(&end, &end, 3));
    builder.emit(abi::branch(&loop_label));
    builder.emit(abi::label(&done));
    builder.emit(abi::store_u64(&end, abi::stack_pointer(), end_slot));
}

/// plan-146-C: the window half — the `[start, end)` of the trimmed result inside
/// `value`'s bytes. `value` is the pre-lowered text argument (the `abi_inline`
/// dispatch lowers it).
pub(crate) fn lower_strings_trim_window(
    builder: &mut CodeBuilder,
    value: &ValueResult,
    trim_start: bool,
    trim_end: bool,
) -> Result<(VirtualRegister, VirtualRegister), String> {
    let scratch16 = builder.temporary_vreg();
    let scratch9 = builder.temporary_vreg();
    let scratch10 = builder.temporary_vreg();
    let scratch11 = builder.temporary_vreg();
    let scratch12 = builder.temporary_vreg();
    let scratch13 = builder.temporary_vreg();
    let scratch14 = builder.temporary_vreg();
    builder.require_string("strings.trim value", value)?;
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
        emit_trim_end_backward(builder, value_slot, start_slot, end_slot);
    }

    builder.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
    builder.emit(abi::load_u64(&scratch10, abi::stack_pointer(), start_slot));
    builder.emit(abi::load_u64(&scratch11, abi::stack_pointer(), end_slot));
    builder.emit(abi::subtract_registers(&scratch12, &scratch11, &scratch10));
    builder.emit(abi::add_immediate(&scratch13, &scratch16, 8));
    builder.emit(abi::add_registers(&scratch13, &scratch13, &scratch10));
    Ok((scratch13, scratch12))
}

pub(crate) fn lower_strings_trim(
    builder: &mut CodeBuilder,
    value: &ValueResult,
    trim_start: bool,
    trim_end: bool,
) -> Result<ValueResult, String> {
    let (ptr, len) = lower_strings_trim_window(builder, value, trim_start, trim_end)?;
    let result = builder.emit_materialize_string_from_bytes(&ptr, &len)?;
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::String,
        location: Operand::from(result),
        text: "strings.trim".to_string(),
    })
}
