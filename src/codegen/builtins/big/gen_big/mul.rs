//! plan-176-R: `big::Int` multiplication and list folds. The layout, slot views, build/compare/trim and release helpers
//! live in `big/gen_big/mod.rs`.

use super::*;

// ---------------------------------------------------------------------------
// Multiplication (plan-127-B Phase 2; word-wide since bug-775).
// ---------------------------------------------------------------------------

/// Copy the slots of two word views into the `outer` and `inner` role slots.
fn emit_assign_word_roles(
    builder: &mut CodeBuilder,
    vregs: &mut Vregs,
    to_outer: &WordSlots,
    to_inner: &WordSlots,
    outer: &WordSlots,
    inner: &WordSlots,
) {
    for (from, to) in [
        (to_outer.data, outer.data),
        (to_outer.words, outer.words),
        (to_outer.top, outer.top),
        (to_inner.data, inner.data),
        (to_inner.words, inner.words),
        (to_inner.top, inner.top),
    ] {
        emit_move_slot(builder, vregs, from, to);
    }
}

/// `|a| * |b|` into a fresh result sized `8 * (wordsA + wordsB)` bytes, schoolbook over
/// 64-bit words. Returns the result's slots and the slot holding the written byte count
/// (the full capacity; `emit_build_int` trims the top). Branches to `alloc_fail` when
/// the result cannot be made.
///
/// Each step is `dst[i + j] + x * y + carry` with `x * y` split into its low (`mul`) and
/// high (`umulh`) words. The sum is at most `(2^64 - 1)^2 + 2 * (2^64 - 1) = 2^128 - 1`,
/// so the two carries out of the low word always fit in the high word, which becomes
/// the next carry. Row `i` writes words `i .. i + wordsInner`, and its final carry lands
/// in `i + wordsInner`, which no earlier row has written, so it is stored, not added.
/// The result region starts zeroed (`emit_alloc_magnitude`), which is what the
/// accumulation reads.
///
/// The operand with more words runs in the inner loop, so a product with a one-word
/// factor (`acc * k`) is one pass over the long operand rather than one row per word.
/// The inner loop reads whole words up to the operand's last word and finishes with its
/// assembled top word, so no read passes its significant bytes.
pub(crate) fn emit_mul_magnitude(
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
    let slot = |builder: &mut CodeBuilder, name: &str| {
        builder.allocate_stack_object(&format!("big_{tag}_{name}"), 8)
    };
    let capacity = slot(builder, "capacity");
    let outer = WordSlots {
        data: slot(builder, "outer_data"),
        words: slot(builder, "outer_words"),
        top: slot(builder, "outer_top"),
    };
    let inner = WordSlots {
        data: slot(builder, "inner_data"),
        words: slot(builder, "inner_words"),
        top: slot(builder, "inner_top"),
    };
    let a_inner = format!("{symbol}_{tag}_a_inner");
    let ordered = format!("{symbol}_{tag}_ordered");
    let (words_a, words_b, total) = (vregs.next(), vregs.next(), vregs.next());
    builder.instructions.extend([
        abi::load_u64(&words_a, abi::stack_pointer(), view_a.words),
        abi::load_u64(&words_b, abi::stack_pointer(), view_b.words),
        abi::add_registers(&total, &words_a, &words_b),
        abi::shift_left_immediate(&total, &total, 3),
        abi::store_u64(&total, abi::stack_pointer(), capacity),
        abi::compare_registers(&words_a, &words_b),
        abi::branch_ge(&a_inner),
    ]);
    // `b` has more words: it runs inside.
    emit_assign_word_roles(builder, vregs, &view_a, &view_b, &outer, &inner);
    builder
        .instructions
        .extend([abi::branch(&ordered), abi::label(&a_inner)]);
    emit_assign_word_roles(builder, vregs, &view_b, &view_a, &outer, &inner);
    builder.instructions.push(abi::label(&ordered));
    let result = emit_alloc_magnitude(builder, vregs, capacity, tag, alloc_fail);

    let row_loop = format!("{symbol}_{tag}_row");
    let row_next = format!("{symbol}_{tag}_row_next");
    let rows_done = format!("{symbol}_{tag}_rows_done");
    let column_loop = format!("{symbol}_{tag}_column");
    let column_last = format!("{symbol}_{tag}_column_last");
    let (row, outer_data, outer_words, outer_last, multiplier, address) = (
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
    );
    let (inner_data, inner_end, inner_cursor, out_cursor, carry) = (
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
    );
    let (y, low, high, word, carry_out) = (
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
    );
    builder.instructions.extend([
        abi::load_u64(&outer_data, abi::stack_pointer(), outer.data),
        abi::load_u64(&outer_words, abi::stack_pointer(), outer.words),
        abi::subtract_immediate(&outer_last, &outer_words, 1),
        abi::load_u64(&inner_data, abi::stack_pointer(), inner.data),
        // inner_end = &inner[wordsInner - 1], the word the top slot stands for.
        abi::load_u64(&inner_end, abi::stack_pointer(), inner.words),
        abi::subtract_immediate(&inner_end, &inner_end, 1),
        abi::shift_left_immediate(&inner_end, &inner_end, 3),
        abi::add_registers(&inner_end, &inner_data, &inner_end),
        abi::move_immediate(&row, "Integer", "0"),
        abi::label(&row_loop),
        abi::compare_registers(&row, &outer_words),
        abi::branch_eq(&rows_done),
    ]);
    emit_word_at(
        builder,
        &outer,
        outer_data,
        outer_last,
        row,
        multiplier,
        address,
        &format!("{symbol}_{tag}_multiplier"),
    );
    builder.instructions.extend([
        // A zero word contributes nothing; the region is already zero there.
        abi::compare_immediate(&multiplier, "0"),
        abi::branch_eq(&row_next),
        abi::load_u64(&out_cursor, abi::stack_pointer(), result.data),
        abi::shift_left_immediate(&address, &row, 3),
        abi::add_registers(&out_cursor, &out_cursor, &address),
        abi::move_register(&inner_cursor, &inner_data),
        abi::move_immediate(&carry, "Integer", "0"),
        abi::label(&column_loop),
        abi::compare_registers(&inner_cursor, &inner_end),
        abi::branch_eq(&column_last),
        abi::load_u64(&y, &inner_cursor, 0),
    ]);
    // One step: dst = low(x*y + carry + dst), carry = high(...).
    let step = |builder: &mut CodeBuilder| {
        builder.instructions.extend([
            abi::multiply_registers(&low, &multiplier, &y),
            abi::unsigned_multiply_high_registers(&high, &multiplier, &y),
            abi::add_carry(&low, &carry_out, &low, &carry, abi::ZERO),
            abi::add_registers(&high, &high, &carry_out),
            abi::load_u64(&word, &out_cursor, 0),
            abi::add_carry(&low, &carry_out, &low, &word, abi::ZERO),
            abi::add_registers(&carry, &high, &carry_out),
            abi::store_u64(&low, &out_cursor, 0),
        ]);
    };
    step(builder);
    builder.instructions.extend([
        abi::add_immediate(&inner_cursor, &inner_cursor, 8),
        abi::add_immediate(&out_cursor, &out_cursor, 8),
        abi::branch(&column_loop),
        abi::label(&column_last),
        abi::load_u64(&y, abi::stack_pointer(), inner.top),
    ]);
    step(builder);
    builder.instructions.extend([
        // The row's carry: word row + wordsInner, untouched by any earlier row.
        abi::store_u64(&carry, &out_cursor, 8),
        abi::label(&row_next),
        abi::add_immediate(&row, &row, 1),
        abi::branch(&row_loop),
        abi::label(&rows_done),
    ]);
    (result, capacity)
}

/// `a * b` as signed numbers, normalized, in `RESULT_VALUE_REGISTER`: the magnitudes
/// multiply and the sign is negative exactly when the operand signs differ.
/// `emit_build_int` clears it for a zero product. Branches to `alloc_fail` when the
/// result cannot be made.
pub(crate) fn emit_mul_int(
    builder: &mut CodeBuilder,
    vregs: &mut Vregs,
    a: &IntSlots,
    b: &IntSlots,
    tag: &str,
    alloc_fail: &str,
) {
    let sign = builder.allocate_stack_object(&format!("big_{tag}_sign"), 8);
    let flag_a = vregs.next();
    let flag_b = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&flag_a, abi::stack_pointer(), a.negative),
        abi::load_u64(&flag_b, abi::stack_pointer(), b.negative),
        abi::exclusive_or_registers(&flag_a, &flag_a, &flag_b),
        abi::store_u64(&flag_a, abi::stack_pointer(), sign),
    ]);
    let (product, product_count) = emit_mul_magnitude(builder, vregs, a, b, tag, alloc_fail);
    emit_build_int(builder, vregs, &product, product_count, sign, tag);
}

// ---------------------------------------------------------------------------
// Aggregates (plan-127-B Phase 2).
// ---------------------------------------------------------------------------

/// Fold the `List OF big::Int` whose address is in `list_slot` into one value — a sum,
/// or a product when `multiply` — normalized, in `RESULT_VALUE_REGISTER`. An empty list
/// gives the identity: zero for a sum, one for a product.
///
/// One native call covers the whole list, which is the point of `big::sum` and
/// `big::product`. Each step makes a fresh accumulator through `emit_add_int` /
/// `emit_mul_int`, and the previous accumulator — this helper's own intermediate, never
/// seen by the caller — is released at the size it was made with
/// (`INT_DATA_OFFSET + dataCapacity`), so a long fold does not pile up blocks.
/// Branches to `alloc_fail` when a step cannot be made.
pub(crate) fn emit_fold_list(
    builder: &mut CodeBuilder,
    vregs: &mut Vregs,
    list_slot: usize,
    multiply: bool,
    tag: &str,
    alloc_fail: &str,
) {
    let symbol = builder.current_symbol.clone();
    let identity = if multiply { "1" } else { "0" };
    let capacity = builder.allocate_stack_object(&format!("big_{tag}_identity_capacity"), 8);
    let negative = builder.allocate_stack_object(&format!("big_{tag}_identity_negative"), 8);
    let acc_slot = builder.allocate_stack_object(&format!("big_{tag}_acc"), 8);
    let element_slot = builder.allocate_stack_object(&format!("big_{tag}_element"), 8);
    let next_slot = builder.allocate_stack_object(&format!("big_{tag}_next"), 8);
    let index_slot = builder.allocate_stack_object(&format!("big_{tag}_index"), 8);

    // The identity: an empty magnitude (zero) or the single byte 1 (one).
    let scratch = vregs.next();
    builder.instructions.extend([
        abi::move_immediate(&scratch, "Integer", identity),
        abi::store_u64(&scratch, abi::stack_pointer(), capacity),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), negative),
    ]);
    let start = emit_alloc_magnitude(
        builder,
        vregs,
        capacity,
        &format!("{tag}_identity"),
        alloc_fail,
    );
    if multiply {
        let data = vregs.next();
        let one = vregs.next();
        builder.instructions.extend([
            abi::load_u64(&data, abi::stack_pointer(), start.data),
            abi::move_immediate(&one, "Integer", "1"),
            abi::store_u8(&one, &data, 0),
        ]);
    }
    emit_build_int(
        builder,
        vregs,
        &start,
        capacity,
        negative,
        &format!("{tag}_identity"),
    );

    let fold_loop = format!("{symbol}_{tag}_fold");
    let fold_done = format!("{symbol}_{tag}_fold_done");
    let index = vregs.next();
    builder.instructions.extend([
        abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), acc_slot),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), index_slot),
        abi::label(&fold_loop),
    ]);
    // Element `index`: the lookup entry's value offset, from the data region base
    // `list + HEADER + capacity * ENTRY` (capacity, never count).
    let list = vregs.next();
    let count = vregs.next();
    let list_capacity = vregs.next();
    let entry = vregs.next();
    let offset = vregs.next();
    let base = vregs.next();
    let stride = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&index, abi::stack_pointer(), index_slot),
        abi::load_u64(&list, abi::stack_pointer(), list_slot),
        abi::load_u64(&count, &list, COLLECTION_OFFSET_COUNT),
        abi::compare_registers(&index, &count),
        abi::branch_eq(&fold_done),
        abi::move_immediate(&stride, "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers(&entry, &index, &stride),
        abi::add_immediate(&entry, &entry, COLLECTION_HEADER_SIZE),
        abi::add_registers(&entry, &list, &entry),
        abi::load_u64(&offset, &entry, COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::load_u64(&list_capacity, &list, COLLECTION_OFFSET_CAPACITY),
        abi::multiply_registers(&base, &list_capacity, &stride),
        abi::add_immediate(&base, &base, COLLECTION_HEADER_SIZE),
        abi::add_registers(&base, &list, &base),
        abi::add_registers(&base, &base, &offset),
        abi::store_u64(&base, abi::stack_pointer(), element_slot),
    ]);
    let acc = emit_load_int(builder, vregs, acc_slot, &format!("{tag}_acc"));
    let element = emit_load_int(builder, vregs, element_slot, &format!("{tag}_element"));
    if multiply {
        emit_mul_int(
            builder,
            vregs,
            &acc,
            &element,
            &format!("{tag}_step"),
            alloc_fail,
        );
    } else {
        emit_add_int(
            builder,
            vregs,
            &acc,
            &element,
            false,
            &format!("{tag}_step"),
            alloc_fail,
        );
    }
    // Release the accumulator this step replaced, at the size it was made with.
    let old = vregs.next();
    let size = vregs.next();
    builder.instructions.extend([
        abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), next_slot),
        abi::load_u64(&old, abi::stack_pointer(), acc_slot),
        abi::add_immediate(&size, &old, INT_MAGNITUDE_BLOCK),
        abi::load_u64(&size, &size, COLLECTION_OFFSET_DATA_CAPACITY),
        abi::add_immediate(&size, &size, INT_DATA_OFFSET),
        abi::move_register(abi::c_arg(0), &old),
        abi::move_register(abi::c_arg(1), &size),
    ]);
    emit_arena_free(&symbol, &mut builder.instructions, &mut builder.relocations);
    let bump = vregs.next();
    let moved = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&moved, abi::stack_pointer(), next_slot),
        abi::store_u64(&moved, abi::stack_pointer(), acc_slot),
        abi::load_u64(&bump, abi::stack_pointer(), index_slot),
        abi::add_immediate(&bump, &bump, 1),
        abi::store_u64(&bump, abi::stack_pointer(), index_slot),
        abi::branch(&fold_loop),
        abi::label(&fold_done),
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), acc_slot),
    ]);
}
