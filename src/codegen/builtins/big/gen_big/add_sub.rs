//! plan-176-R: `big::Int` addition and subtraction. The layout, slot views, build/compare/trim and release helpers
//! live in `big/gen_big/mod.rs`.

use super::*;

// ---------------------------------------------------------------------------
// Additive arithmetic (plan-127-B Phase 1; word-wide since bug-775).
//
// The magnitudes are walked a 64-bit word at a time with an explicit carry register
// (`abi::add_carry`): a word sum plus a carry is `lhs + rhs + carryIn`, and its carry
// out is 0 or 1. A subtraction is the same chain over the complemented subtrahend,
// `x - y - borrow = x + !y + (1 - borrow)`, so the carry it propagates is the
// complement of the borrow.
// ---------------------------------------------------------------------------

/// `|a| + |b|` into a fresh result one word longer than the longer operand spans.
/// Returns the result's slots and the slot holding how many bytes were written (all of
/// them; the top word is the final carry, which `emit_build_int` trims when it is zero).
/// Branches to `alloc_fail` when the result cannot be made.
pub(crate) fn emit_add_magnitude(
    builder: &mut CodeBuilder,
    vregs: &mut Vregs,
    a: &IntSlots,
    b: &IntSlots,
    tag: &str,
    alloc_fail: &str,
) -> (ResultSlots, usize) {
    let symbol = builder.current_symbol.clone();
    let view_a = emit_word_view(builder, vregs, a, &format!("{tag}_a"));
    let view_b = emit_word_view(builder, vregs, b, &format!("{tag}_b"));
    let capacity = builder.allocate_stack_object(&format!("big_{tag}_capacity"), 8);
    let longer = format!("{symbol}_{tag}_longer");
    let (words_a, words_b) = (vregs.next(), vregs.next());
    builder.instructions.extend([
        abi::load_u64(&words_a, abi::stack_pointer(), view_a.words),
        abi::load_u64(&words_b, abi::stack_pointer(), view_b.words),
        abi::compare_registers(&words_a, &words_b),
        abi::branch_ge(&longer),
        abi::move_register(&words_a, &words_b),
        abi::label(&longer),
        abi::add_immediate(&words_a, &words_a, 1),
        abi::shift_left_immediate(&words_a, &words_a, 3),
        abi::store_u64(&words_a, abi::stack_pointer(), capacity),
    ]);
    let result = emit_alloc_magnitude(builder, vregs, capacity, tag, alloc_fail);

    // Everything reloads after the allocation call.
    let add_loop = format!("{symbol}_{tag}_add");
    let add_done = format!("{symbol}_{tag}_add_done");
    let (index, limit, carry, carry_out, dst) = (
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
    );
    let (data_a, words_a, last_a, data_b, words_b, last_b) = (
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
    );
    let (x, y, address) = (vregs.next(), vregs.next(), vregs.next());
    builder.instructions.extend([
        abi::load_u64(&dst, abi::stack_pointer(), result.data),
        abi::load_u64(&data_a, abi::stack_pointer(), a.data),
        abi::load_u64(&words_a, abi::stack_pointer(), view_a.words),
        abi::subtract_immediate(&last_a, &words_a, 1),
        abi::load_u64(&data_b, abi::stack_pointer(), b.data),
        abi::load_u64(&words_b, abi::stack_pointer(), view_b.words),
        abi::subtract_immediate(&last_b, &words_b, 1),
        abi::load_u64(&limit, abi::stack_pointer(), capacity),
        abi::shift_right_immediate(&limit, &limit, 3),
        abi::subtract_immediate(&limit, &limit, 1),
        abi::move_immediate(&index, "Integer", "0"),
        abi::move_immediate(&carry, "Integer", "0"),
        abi::label(&add_loop),
        abi::compare_registers(&index, &limit),
        abi::branch_eq(&add_done),
    ]);
    emit_word_or_zero(
        builder,
        &view_a,
        data_a,
        words_a,
        last_a,
        index,
        x,
        address,
        &format!("{symbol}_{tag}_add_x"),
    );
    emit_word_or_zero(
        builder,
        &view_b,
        data_b,
        words_b,
        last_b,
        index,
        y,
        address,
        &format!("{symbol}_{tag}_add_y"),
    );
    builder.instructions.extend([
        // `dst` aliases `lhs`: x86-64 allocates only four integer registers, and an
        // `add_carry` with a register carry-in and a separate `dst` names five.
        abi::add_carry(&x, &carry_out, &x, &y, &carry),
        abi::move_register(&carry, &carry_out),
        abi::store_u64(&x, &dst, 0),
        abi::add_immediate(&dst, &dst, 8),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&add_loop),
        abi::label(&add_done),
        // The final carry is the top word.
        abi::store_u64(&carry, &dst, 0),
    ]);
    (result, capacity)
}

/// `|larger| - |smaller|` into a fresh result sized `countLarger`. **The caller
/// guarantees `|larger| >= |smaller|`** — the sign dispatch in [`emit_add_int`] does so
/// with [`emit_compare_magnitude`]; this emitter does not check. Returns the result's
/// slots and the slot holding the written byte count. Branches to `alloc_fail` when the
/// result cannot be made.
pub(crate) fn emit_sub_magnitude(
    builder: &mut CodeBuilder,
    vregs: &mut Vregs,
    larger: &IntSlots,
    smaller: &IntSlots,
    tag: &str,
    alloc_fail: &str,
) -> (ResultSlots, usize) {
    let symbol = builder.current_symbol.clone();
    let view_l = emit_word_view(builder, vregs, larger, &format!("{tag}_l"));
    let view_s = emit_word_view(builder, vregs, smaller, &format!("{tag}_s"));
    let result = emit_alloc_magnitude(builder, vregs, larger.count, tag, alloc_fail);

    let sub_loop = format!("{symbol}_{tag}_sub");
    let sub_done = format!("{symbol}_{tag}_sub_done");
    let (index, carry, carry_out, dst) = (vregs.next(), vregs.next(), vregs.next(), vregs.next());
    let (data_l, words_l, last_l, data_s, words_s, last_s) = (
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
    );
    let (x, y, address) = (vregs.next(), vregs.next(), vregs.next());
    builder.instructions.extend([
        abi::load_u64(&dst, abi::stack_pointer(), result.data),
        abi::load_u64(&data_l, abi::stack_pointer(), larger.data),
        abi::load_u64(&words_l, abi::stack_pointer(), view_l.words),
        abi::subtract_immediate(&last_l, &words_l, 1),
        abi::load_u64(&data_s, abi::stack_pointer(), smaller.data),
        abi::load_u64(&words_s, abi::stack_pointer(), view_s.words),
        abi::subtract_immediate(&last_s, &words_s, 1),
        abi::move_immediate(&index, "Integer", "0"),
        // No borrow into the first word: the complemented chain starts with carry 1.
        abi::move_immediate(&carry, "Integer", "1"),
        abi::label(&sub_loop),
        abi::compare_registers(&index, &words_l),
        abi::branch_eq(&sub_done),
    ]);
    emit_word_at(
        builder,
        &view_l,
        data_l,
        last_l,
        index,
        x,
        address,
        &format!("{symbol}_{tag}_sub_x"),
    );
    emit_word_or_zero(
        builder,
        &view_s,
        data_s,
        words_s,
        last_s,
        index,
        y,
        address,
        &format!("{symbol}_{tag}_sub_y"),
    );
    builder.instructions.extend([
        abi::bitwise_not(&y, &y),
        // `dst` aliases `lhs`, as in `emit_add_magnitude`.
        abi::add_carry(&x, &carry_out, &x, &y, &carry),
        abi::move_register(&carry, &carry_out),
        abi::store_u64(&x, &dst, 0),
        abi::add_immediate(&dst, &dst, 8),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&sub_loop),
        abi::label(&sub_done),
    ]);
    (result, larger.count)
}

/// `a + b` (or `a - b` when `subtract`) as signed numbers, normalized, in
/// `RESULT_VALUE_REGISTER` (plan-127-B §4.2). With `sb` the sign of `b` flipped for a
/// subtraction:
///
/// - equal signs: the magnitudes add and the result takes that sign;
/// - opposing signs: the smaller magnitude comes off the larger and the result takes
///   the larger operand's sign. `emit_build_int` clears the sign of a zero result, which
///   is why `1 + (-1)` is canonical zero and not a negative zero.
pub(crate) fn emit_add_int(
    builder: &mut CodeBuilder,
    vregs: &mut Vregs,
    a: &IntSlots,
    b: &IntSlots,
    subtract: bool,
    tag: &str,
    alloc_fail: &str,
) {
    let symbol = builder.current_symbol.clone();
    let sign_a = builder.allocate_stack_object(&format!("big_{tag}_sign_a"), 8);
    let sign_b = builder.allocate_stack_object(&format!("big_{tag}_sign_b"), 8);
    let opposing = format!("{symbol}_{tag}_opposing");
    let b_larger = format!("{symbol}_{tag}_b_larger");
    let joined = format!("{symbol}_{tag}_joined");
    let flag_a = vregs.next();
    let flag_b = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&flag_a, abi::stack_pointer(), a.negative),
        abi::load_u64(&flag_b, abi::stack_pointer(), b.negative),
    ]);
    if subtract {
        let one = vregs.next();
        builder.instructions.extend([
            abi::move_immediate(&one, "Integer", "1"),
            abi::exclusive_or_registers(&flag_b, &flag_b, &one),
        ]);
    }
    builder.instructions.extend([
        abi::store_u64(&flag_a, abi::stack_pointer(), sign_a),
        abi::store_u64(&flag_b, abi::stack_pointer(), sign_b),
        abi::compare_registers(&flag_a, &flag_b),
        abi::branch_ne(&opposing),
    ]);

    // Equal effective signs: add the magnitudes.
    let (sum, sum_count) =
        emit_add_magnitude(builder, vregs, a, b, &format!("{tag}_sum"), alloc_fail);
    emit_build_int(
        builder,
        vregs,
        &sum,
        sum_count,
        sign_a,
        &format!("{tag}_sum"),
    );
    builder
        .instructions
        .extend([abi::branch(&joined), abi::label(&opposing)]);

    // Opposing signs: subtract the smaller magnitude from the larger.
    let order = vregs.next();
    emit_compare_magnitude(builder, vregs, a, b, &order, &format!("{tag}_order"));
    builder.instructions.extend([
        abi::compare_immediate(&order, "0"),
        abi::branch_lt(&b_larger),
    ]);
    let (a_minus_b, a_minus_b_count) =
        emit_sub_magnitude(builder, vregs, a, b, &format!("{tag}_ab"), alloc_fail);
    emit_build_int(
        builder,
        vregs,
        &a_minus_b,
        a_minus_b_count,
        sign_a,
        &format!("{tag}_ab"),
    );
    builder
        .instructions
        .extend([abi::branch(&joined), abi::label(&b_larger)]);
    let (b_minus_a, b_minus_a_count) =
        emit_sub_magnitude(builder, vregs, b, a, &format!("{tag}_ba"), alloc_fail);
    emit_build_int(
        builder,
        vregs,
        &b_minus_a,
        b_minus_a_count,
        sign_b,
        &format!("{tag}_ba"),
    );
    builder.instructions.push(abi::label(&joined));
}
