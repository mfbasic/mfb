//! plan-176-R: `big::Int` shifts. The layout, slot views, build/compare/trim and release helpers
//! live in `big/gen_big/mod.rs`.

use super::*;

// ---------------------------------------------------------------------------
// Bit operations (plan-127-B Phase 3). They act on the magnitude and keep the sign.
// ---------------------------------------------------------------------------

/// Branch to `invalid` when the `Integer` in `slot` is negative.
pub(crate) fn emit_reject_negative(
    builder: &mut CodeBuilder,
    vregs: &mut Vregs,
    slot: usize,
    invalid: &str,
) {
    let value = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&value, abi::stack_pointer(), slot),
        abi::compare_immediate(&value, "0"),
        abi::branch_lt(invalid),
    ]);
}

/// `|a| << shift` into a result sized `countA + shift / 8 + 1` (zero stays an empty
/// result, whatever the shift). `shift_slot` holds a non-negative `Integer`. Returns the
/// result's slots and the slot holding the written byte count. Branches to `alloc_fail`
/// when the result cannot be made.
pub(crate) fn emit_shift_left_magnitude(
    builder: &mut CodeBuilder,
    vregs: &mut Vregs,
    a: &IntSlots,
    shift_slot: usize,
    tag: &str,
    alloc_fail: &str,
) -> (ResultSlots, usize) {
    let symbol = builder.current_symbol.clone();
    let capacity = builder.allocate_stack_object(&format!("big_{tag}_capacity"), 8);
    let sized = format!("{symbol}_{tag}_sized");
    let count = vregs.next();
    let whole = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&count, abi::stack_pointer(), a.count),
        abi::compare_immediate(&count, "0"),
        abi::branch_eq(&sized),
        abi::load_u64(&whole, abi::stack_pointer(), shift_slot),
        abi::shift_right_immediate(&whole, &whole, 3),
        abi::add_registers(&count, &count, &whole),
        abi::add_immediate(&count, &count, 1),
        abi::label(&sized),
        abi::store_u64(&count, abi::stack_pointer(), capacity),
    ]);
    let result = emit_alloc_magnitude(builder, vregs, capacity, tag, alloc_fail);

    let shift_loop = format!("{symbol}_{tag}_shl");
    let shift_done = format!("{symbol}_{tag}_shl_done");
    let empty = format!("{symbol}_{tag}_shl_empty");
    let data = vregs.next();
    let count = vregs.next();
    let bits = vregs.next();
    let mask = vregs.next();
    let base = vregs.next();
    let index = vregs.next();
    let acc = vregs.next();
    let value = vregs.next();
    let cursor = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&count, abi::stack_pointer(), a.count),
        abi::compare_immediate(&count, "0"),
        abi::branch_eq(&empty),
        abi::load_u64(&data, abi::stack_pointer(), a.data),
        abi::load_u64(&bits, abi::stack_pointer(), shift_slot),
        // Whole bytes move the write position; the remaining bits shift each byte.
        abi::shift_right_immediate(&base, &bits, 3),
        abi::move_immediate(&mask, "Integer", "7"),
        abi::and_registers(&bits, &bits, &mask),
        abi::load_u64(&cursor, abi::stack_pointer(), result.data),
        abi::add_registers(&base, &cursor, &base),
        abi::move_immediate(&index, "Integer", "0"),
        abi::move_immediate(&acc, "Integer", "0"),
        abi::label(&shift_loop),
        abi::compare_registers(&index, &count),
        abi::branch_eq(&shift_done),
        abi::add_registers(&cursor, &data, &index),
        abi::load_u8(&value, &cursor, 0),
        abi::shift_left_variable(&value, &value, &bits),
        abi::or_registers(&value, &value, &acc),
        abi::add_registers(&cursor, &base, &index),
        abi::store_u8(&value, &cursor, 0),
        abi::shift_right_immediate(&acc, &value, 8),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&shift_loop),
        abi::label(&shift_done),
        // The bits shifted out of the top byte.
        abi::add_registers(&cursor, &base, &index),
        abi::store_u8(&acc, &cursor, 0),
        abi::label(&empty),
    ]);
    (result, capacity)
}

/// `|a| >> shift` into a result sized `countA - shift / 8`, or empty when every byte is
/// shifted out. `shift_slot` holds a non-negative `Integer`. Returns the result's slots
/// and the slot holding the written byte count. Branches to `alloc_fail` when the result
/// cannot be made.
pub(crate) fn emit_shift_right_magnitude(
    builder: &mut CodeBuilder,
    vregs: &mut Vregs,
    a: &IntSlots,
    shift_slot: usize,
    tag: &str,
    alloc_fail: &str,
) -> (ResultSlots, usize) {
    let symbol = builder.current_symbol.clone();
    let capacity = builder.allocate_stack_object(&format!("big_{tag}_capacity"), 8);
    let everything = format!("{symbol}_{tag}_everything");
    let sized = format!("{symbol}_{tag}_sized");
    let count = vregs.next();
    let whole = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&count, abi::stack_pointer(), a.count),
        abi::load_u64(&whole, abi::stack_pointer(), shift_slot),
        abi::shift_right_immediate(&whole, &whole, 3),
        abi::compare_registers(&whole, &count),
        abi::branch_ge(&everything),
        abi::subtract_registers(&count, &count, &whole),
        abi::branch(&sized),
        abi::label(&everything),
        abi::move_immediate(&count, "Integer", "0"),
        abi::label(&sized),
        abi::store_u64(&count, abi::stack_pointer(), capacity),
    ]);
    let result = emit_alloc_magnitude(builder, vregs, capacity, tag, alloc_fail);

    let shift_loop = format!("{symbol}_{tag}_shr");
    let shift_done = format!("{symbol}_{tag}_shr_done");
    let no_high = format!("{symbol}_{tag}_shr_no_high");
    let data = vregs.next();
    let count = vregs.next();
    let limit = vregs.next();
    let bits = vregs.next();
    let whole = vregs.next();
    let index = vregs.next();
    let value = vregs.next();
    let high = vregs.next();
    let cursor = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&data, abi::stack_pointer(), a.data),
        abi::load_u64(&count, abi::stack_pointer(), a.count),
        abi::load_u64(&limit, abi::stack_pointer(), capacity),
        abi::load_u64(&bits, abi::stack_pointer(), shift_slot),
        abi::shift_right_immediate(&whole, &bits, 3),
        abi::move_immediate(&high, "Integer", "7"),
        abi::and_registers(&bits, &bits, &high),
        abi::move_immediate(&index, "Integer", "0"),
        abi::label(&shift_loop),
        abi::compare_registers(&index, &limit),
        abi::branch_eq(&shift_done),
        // value = a[index + whole] | a[index + whole + 1] << 8, then drop the low bits.
        abi::add_registers(&cursor, &index, &whole),
        abi::add_registers(&cursor, &data, &cursor),
        abi::load_u8(&value, &cursor, 0),
        abi::add_registers(&high, &index, &whole),
        abi::add_immediate(&high, &high, 1),
        abi::compare_registers(&high, &count),
        abi::branch_ge(&no_high),
        abi::load_u8(&high, &cursor, 1),
        abi::shift_left_immediate(&high, &high, 8),
        abi::or_registers(&value, &value, &high),
        abi::label(&no_high),
        abi::shift_right_variable(&value, &value, &bits),
        abi::load_u64(&cursor, abi::stack_pointer(), result.data),
        abi::add_registers(&cursor, &cursor, &index),
        abi::store_u8(&value, &cursor, 0),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&shift_loop),
        abi::label(&shift_done),
    ]);
    (result, capacity)
}
