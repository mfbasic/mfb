//! The native access foundation every `big` member lowers through (plan-127-A §4.3).
//!
//! A `big::Int` argument is a pointer to a two-slot record (plan-127-A §4.2, pinned in
//! Phase 1): slot 0 (`+0`) holds the block-relative offset of the inlined
//! `List OF Byte` magnitude, slot 1 (`+8`) holds `negative` inline. The magnitude is a
//! kind-2 fixed-width list, so its `count` is at `+8` and its bytes start at `+40` for
//! every capacity. A `big::Int` therefore occupies `16 + 40 + dataCapacity` bytes — the
//! size the record marshaller computes and scope exit releases.
//!
//! Three emitters, and every member goes through them:
//!
//! - [`emit_load_int`] reads an argument into `(data, count, negative)` frame slots and
//!   **trims** it: trailing zero bytes are not counted and a zero magnitude reads as
//!   non-negative. The decoder is total by design — a hand-built record that breaks the
//!   canonical form still reads as the number it spells.
//! - [`emit_alloc_magnitude`] makes the result record up front at its largest possible
//!   size, with a zeroed magnitude region the member writes into directly. One block
//!   per result; no scratch buffer and no second copy (Open Decision 1).
//! - [`emit_build_int`] **normalizes** that result — drops trailing zero bytes, forces a
//!   zero value non-negative — and publishes it in the result register. It is the one
//!   place canonical form is established, so it is a property of the package rather
//!   than of each lowering.
//!
//! Everything is kept in frame slots between steps. The only call these emitters make
//! is the arena allocation, and every value a later step needs is reloaded from its
//! slot, so no register is assumed to survive it.

use crate::codegen::collection::layout::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::util::Vregs;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;

/// `big::Int` slot 0: the block-relative offset of the inlined magnitude.
pub(crate) const INT_OFFSET_MAGNITUDE: usize = 0;
/// `big::Int` slot 1: the `negative` flag, inline.
pub(crate) const INT_OFFSET_NEGATIVE: usize = 8;
/// Where the record marshaller places the first inlined field: `8 * fieldCount`.
pub(crate) const INT_MAGNITUDE_BLOCK: usize = 16;
/// The first magnitude byte, relative to the record: the block plus the list header.
pub(crate) const INT_DATA_OFFSET: usize = INT_MAGNITUDE_BLOCK + COLLECTION_HEADER_SIZE;

/// The frame slots a loaded `big::Int` lives in.
#[derive(Clone, Copy, Debug)]
pub(crate) struct IntSlots {
    /// Address of the first (least significant) magnitude byte.
    pub(crate) data: usize,
    /// Significant byte count — trailing zero bytes already excluded.
    pub(crate) count: usize,
    /// `1` when negative, `0` otherwise (always `0` when `count` is `0`).
    pub(crate) negative: usize,
}

/// The frame slots a result under construction lives in.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ResultSlots {
    /// The result record.
    pub(crate) record: usize,
    /// Address of its first magnitude byte.
    pub(crate) data: usize,
}

/// Spill every incoming argument register to its own frame slot, before any other
/// instruction. An argument register is live only until the body's first scratch use
/// or call; after this every argument is read from its slot.
pub(crate) fn emit_spill_args(builder: &mut CodeBuilder, args: &[ValueResult]) -> Vec<usize> {
    args.iter()
        .enumerate()
        .map(|(index, arg)| {
            let slot = builder.allocate_stack_object(&format!("big_arg{index}"), 8);
            builder.emit(abi::store_u64(&arg.location, abi::stack_pointer(), slot));
            slot
        })
        .collect()
}

/// Resolve the `big::Int` whose record address is in `arg_slot` to its data address,
/// significant byte count and sign, each in a fresh frame slot. `tag` keeps the trim
/// loop's labels distinct when a member loads several operands.
pub(crate) fn emit_load_int(
    builder: &mut CodeBuilder,
    vregs: &mut Vregs,
    arg_slot: usize,
    tag: &str,
) -> IntSlots {
    let symbol = builder.current_symbol.clone();
    let slots = IntSlots {
        data: builder.allocate_stack_object(&format!("big_{tag}_data"), 8),
        count: builder.allocate_stack_object(&format!("big_{tag}_count"), 8),
        negative: builder.allocate_stack_object(&format!("big_{tag}_negative"), 8),
    };
    let record = vregs.next();
    let block = vregs.next();
    let data = vregs.next();
    let count = vregs.next();
    let negative = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&record, abi::stack_pointer(), arg_slot),
        abi::load_u64(&block, &record, INT_OFFSET_MAGNITUDE),
        abi::add_registers(&block, &record, &block),
        abi::load_u64(&count, &block, COLLECTION_OFFSET_COUNT),
        abi::add_immediate(&data, &block, COLLECTION_HEADER_SIZE),
        abi::load_u64(&negative, &record, INT_OFFSET_NEGATIVE),
        abi::store_u64(&data, abi::stack_pointer(), slots.data),
    ]);
    emit_trim(builder, vregs, &data, &count, &negative, &format!("{symbol}_{tag}_load"));
    builder.instructions.extend([
        abi::store_u64(&count, abi::stack_pointer(), slots.count),
        abi::store_u64(&negative, abi::stack_pointer(), slots.negative),
    ]);
    slots
}

/// Make a result `big::Int` able to hold `capacity_slot` magnitude bytes: the record
/// block, its slot-0 offset, a kind-2 list header with `count` zero, and a zeroed
/// magnitude region. Branches to `alloc_fail` when the block cannot be made.
pub(crate) fn emit_alloc_magnitude(
    builder: &mut CodeBuilder,
    vregs: &mut Vregs,
    capacity_slot: usize,
    tag: &str,
    alloc_fail: &str,
) -> ResultSlots {
    let symbol = builder.current_symbol.clone();
    let slots = ResultSlots {
        record: builder.allocate_stack_object(&format!("big_{tag}_record"), 8),
        data: builder.allocate_stack_object(&format!("big_{tag}_rdata"), 8),
    };
    let size = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&size, abi::stack_pointer(), capacity_slot),
        abi::add_immediate(&size, &size, INT_DATA_OFFSET),
        abi::move_register(abi::return_register(), &size),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(
        &symbol,
        &mut builder.instructions,
        &mut builder.relocations,
        alloc_fail,
    );
    let record = vregs.next();
    let block = vregs.next();
    let scratch = vregs.next();
    let capacity = vregs.next();
    let cursor = vregs.next();
    let end = vregs.next();
    let zero_loop = format!("{symbol}_{tag}_zero");
    let zero_done = format!("{symbol}_{tag}_zero_done");
    builder.instructions.extend([
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), slots.record),
        abi::load_u64(&record, abi::stack_pointer(), slots.record),
        abi::load_u64(&capacity, abi::stack_pointer(), capacity_slot),
        // Slot 0: the magnitude block sits right after the two slots.
        abi::move_immediate(&scratch, "Integer", &INT_MAGNITUDE_BLOCK.to_string()),
        abi::store_u64(&scratch, &record, INT_OFFSET_MAGNITUDE),
        abi::store_u64(abi::ZERO, &record, INT_OFFSET_NEGATIVE),
        abi::add_immediate(&block, &record, INT_MAGNITUDE_BLOCK),
        // The kind-2 `List OF Byte` header, as `emit_build_byte_list` writes it. The
        // capacity words are the block's real size, so a copy or a release sizes the
        // block exactly; `count`/`dataLength` start at zero and are set by
        // `emit_build_int`.
        abi::move_immediate(&scratch, "Byte", &byte_list_block_kind().to_string()),
        abi::store_u8(&scratch, &block, COLLECTION_OFFSET_KIND),
        abi::move_immediate(&scratch, "Byte", &COLLECTION_TYPE_NONE.to_string()),
        abi::store_u8(&scratch, &block, COLLECTION_OFFSET_KEY_TYPE),
        abi::move_immediate(&scratch, "Byte", &COLLECTION_TYPE_BYTE.to_string()),
        abi::store_u8(&scratch, &block, COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate(&scratch, "Byte", "1"),
        abi::store_u8(&scratch, &block, COLLECTION_OFFSET_FLAGS_VERSION),
        abi::store_u8(abi::ZERO, &block, COLLECTION_OFFSET_BUCKETS_READY),
        abi::store_u64(abi::ZERO, &block, COLLECTION_OFFSET_COUNT),
        abi::store_u64(&capacity, &block, COLLECTION_OFFSET_CAPACITY),
        abi::store_u64(abi::ZERO, &block, COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64(&capacity, &block, COLLECTION_OFFSET_DATA_CAPACITY),
        // The magnitude region, zeroed: a member writes only the bytes it computes,
        // and multiplication accumulates into bytes it has not written yet.
        abi::add_immediate(&cursor, &record, INT_DATA_OFFSET),
        abi::store_u64(&cursor, abi::stack_pointer(), slots.data),
        abi::add_registers(&end, &cursor, &capacity),
        abi::label(&zero_loop),
        abi::compare_registers(&cursor, &end),
        abi::branch_eq(&zero_done),
        abi::store_u8(abi::ZERO, &cursor, 0),
        abi::add_immediate(&cursor, &cursor, 1),
        abi::branch(&zero_loop),
        abi::label(&zero_done),
    ]);
    slots
}

/// Normalize the result in `result` to canonical form and leave its record address in
/// `RESULT_VALUE_REGISTER`. `count_slot` holds how many magnitude bytes the member
/// wrote (at most the capacity); `negative_slot` holds `1` for a negative result.
pub(crate) fn emit_build_int(
    builder: &mut CodeBuilder,
    vregs: &mut Vregs,
    result: &ResultSlots,
    count_slot: usize,
    negative_slot: usize,
    tag: &str,
) {
    let symbol = builder.current_symbol.clone();
    let record = vregs.next();
    let block = vregs.next();
    let data = vregs.next();
    let count = vregs.next();
    let negative = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&data, abi::stack_pointer(), result.data),
        abi::load_u64(&count, abi::stack_pointer(), count_slot),
        abi::load_u64(&negative, abi::stack_pointer(), negative_slot),
    ]);
    emit_trim(builder, vregs, &data, &count, &negative, &format!("{symbol}_{tag}_build"));
    builder.instructions.extend([
        abi::load_u64(&record, abi::stack_pointer(), result.record),
        abi::add_immediate(&block, &record, INT_MAGNITUDE_BLOCK),
        abi::store_u64(&count, &block, COLLECTION_OFFSET_COUNT),
        abi::store_u64(&count, &block, COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64(&negative, &record, INT_OFFSET_NEGATIVE),
        abi::move_register(RESULT_VALUE_REGISTER, &record),
    ]);
}

/// Order the magnitudes of two loaded operands, ignoring sign: `-1`, `0` or `1` into
/// `out`. A longer significant count is larger; equal counts compare byte by byte from
/// the most significant end. Both operands are already trimmed by [`emit_load_int`], so
/// the count comparison is exact.
pub(crate) fn emit_compare_magnitude(
    builder: &mut CodeBuilder,
    vregs: &mut Vregs,
    a: &IntSlots,
    b: &IntSlots,
    out: &str,
    tag: &str,
) {
    let symbol = builder.current_symbol.clone();
    let less = format!("{symbol}_{tag}_less");
    let greater = format!("{symbol}_{tag}_greater");
    let equal = format!("{symbol}_{tag}_equal");
    let walk = format!("{symbol}_{tag}_walk");
    let finished = format!("{symbol}_{tag}_finished");
    let count_a = vregs.next();
    let count_b = vregs.next();
    let data_a = vregs.next();
    let data_b = vregs.next();
    let byte_a = vregs.next();
    let byte_b = vregs.next();
    let cursor = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&count_a, abi::stack_pointer(), a.count),
        abi::load_u64(&count_b, abi::stack_pointer(), b.count),
        abi::compare_registers(&count_a, &count_b),
        abi::branch_lo(&less),
        abi::branch_hi(&greater),
        abi::load_u64(&data_a, abi::stack_pointer(), a.data),
        abi::load_u64(&data_b, abi::stack_pointer(), b.data),
        abi::label(&walk),
        abi::compare_immediate(&count_a, "0"),
        abi::branch_eq(&equal),
        abi::subtract_immediate(&count_a, &count_a, 1),
        abi::add_registers(&cursor, &data_a, &count_a),
        abi::load_u8(&byte_a, &cursor, 0),
        abi::add_registers(&cursor, &data_b, &count_a),
        abi::load_u8(&byte_b, &cursor, 0),
        abi::compare_registers(&byte_a, &byte_b),
        abi::branch_lo(&less),
        abi::branch_hi(&greater),
        abi::branch(&walk),
        abi::label(&less),
        // `move_immediate` refuses a negative literal: build -1 as 0 - 1.
        abi::move_immediate(out, "Integer", "0"),
        abi::subtract_immediate(out, out, 1),
        abi::branch(&finished),
        abi::label(&greater),
        abi::move_immediate(out, "Integer", "1"),
        abi::branch(&finished),
        abi::label(&equal),
        abi::move_immediate(out, "Integer", "0"),
        abi::label(&finished),
    ]);
}

/// Order two loaded operands as signed numbers: `-1`, `0` or `1` into `out`. Differing
/// signs decide it outright (a trimmed zero is never negative, so zero sorts between
/// the negatives and the positives); equal signs defer to the magnitudes, reversed when
/// both are negative.
pub(crate) fn emit_compare_int(
    builder: &mut CodeBuilder,
    vregs: &mut Vregs,
    a: &IntSlots,
    b: &IntSlots,
    out: &str,
    tag: &str,
) {
    let symbol = builder.current_symbol.clone();
    let same_sign = format!("{symbol}_{tag}_same_sign");
    let a_negative = format!("{symbol}_{tag}_a_negative");
    let finished = format!("{symbol}_{tag}_signed_done");
    let sign_a = vregs.next();
    let sign_b = vregs.next();
    let zero = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&sign_a, abi::stack_pointer(), a.negative),
        abi::load_u64(&sign_b, abi::stack_pointer(), b.negative),
        abi::compare_registers(&sign_a, &sign_b),
        abi::branch_eq(&same_sign),
        abi::compare_immediate(&sign_a, "0"),
        abi::branch_ne(&a_negative),
        abi::move_immediate(out, "Integer", "1"),
        abi::branch(&finished),
        abi::label(&a_negative),
        abi::move_immediate(out, "Integer", "0"),
        abi::subtract_immediate(out, out, 1),
        abi::branch(&finished),
        abi::label(&same_sign),
    ]);
    emit_compare_magnitude(builder, vregs, a, b, out, tag);
    let sign = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&sign, abi::stack_pointer(), a.negative),
        abi::compare_immediate(&sign, "0"),
        abi::branch_eq(&finished),
        abi::move_immediate(&zero, "Integer", "0"),
        abi::subtract_registers(out, &zero, out),
        abi::label(&finished),
    ]);
}

/// A new `big::Int` with `operand`'s magnitude and the sign in `negative_slot`
/// (`1` = negative), normalized, in `RESULT_VALUE_REGISTER`. Serves `abs` and `negate`.
/// Branches to `alloc_fail` when the result cannot be made.
pub(crate) fn emit_copy_int(
    builder: &mut CodeBuilder,
    vregs: &mut Vregs,
    operand: &IntSlots,
    negative_slot: usize,
    tag: &str,
    alloc_fail: &str,
) {
    let symbol = builder.current_symbol.clone();
    let result = emit_alloc_magnitude(builder, vregs, operand.count, tag, alloc_fail);
    let copy_loop = format!("{symbol}_{tag}_copy");
    let copy_done = format!("{symbol}_{tag}_copy_done");
    let src = vregs.next();
    let dst = vregs.next();
    let count = vregs.next();
    let index = vregs.next();
    let at = vregs.next();
    let byte = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&src, abi::stack_pointer(), operand.data),
        abi::load_u64(&dst, abi::stack_pointer(), result.data),
        abi::load_u64(&count, abi::stack_pointer(), operand.count),
        abi::move_immediate(&index, "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers(&index, &count),
        abi::branch_eq(&copy_done),
        abi::add_registers(&at, &src, &index),
        abi::load_u8(&byte, &at, 0),
        abi::add_registers(&at, &dst, &index),
        abi::store_u8(&byte, &at, 0),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
    ]);
    emit_build_int(builder, vregs, &result, operand.count, negative_slot, tag);
}

/// The canonical-form rule, emitted once for both directions: walk `count` down past
/// zero high bytes, then clear `negative` when nothing is left. `data` is preserved;
/// `count` and `negative` are updated in place.
fn emit_trim(
    builder: &mut CodeBuilder,
    vregs: &mut Vregs,
    data: &str,
    count: &str,
    negative: &str,
    label_base: &str,
) {
    let cursor = vregs.next();
    let byte = vregs.next();
    let trim_loop = format!("{label_base}_trim");
    let trim_done = format!("{label_base}_trim_done");
    let sign_ok = format!("{label_base}_sign_ok");
    builder.instructions.extend([
        abi::label(&trim_loop),
        abi::compare_immediate(count, "0"),
        abi::branch_eq(&trim_done),
        abi::add_registers(&cursor, data, count),
        abi::subtract_immediate(&cursor, &cursor, 1),
        abi::load_u8(&byte, &cursor, 0),
        abi::compare_immediate(&byte, "0"),
        abi::branch_ne(&trim_done),
        abi::subtract_immediate(count, count, 1),
        abi::branch(&trim_loop),
        abi::label(&trim_done),
        abi::compare_immediate(count, "0"),
        abi::branch_ne(&sign_ok),
        abi::move_immediate(negative, "Integer", "0"),
        abi::label(&sign_ok),
    ]);
}

// ---------------------------------------------------------------------------
// Additive arithmetic (plan-127-B Phase 1).
//
// Byte limbs keep every intermediate small: a byte sum plus a carry is at most
// `255 + 255 + 1 = 511`, and a byte difference minus a borrow is at least
// `0 - 255 - 1 = -256`, so no step needs a widening primitive and none can overflow.
// ---------------------------------------------------------------------------

/// `|a| + |b|` into a fresh result sized `max(countA, countB) + 1`. Returns the result's
/// slots and the slot holding how many bytes were written (the full capacity; the top
/// byte is the final carry, which `emit_build_int` trims when it is zero). Branches to
/// `alloc_fail` when the result cannot be made.
pub(crate) fn emit_add_magnitude(
    builder: &mut CodeBuilder,
    vregs: &mut Vregs,
    a: &IntSlots,
    b: &IntSlots,
    tag: &str,
    alloc_fail: &str,
) -> (ResultSlots, usize) {
    let symbol = builder.current_symbol.clone();
    let capacity = builder.allocate_stack_object(&format!("big_{tag}_capacity"), 8);
    let longer = format!("{symbol}_{tag}_longer");
    let count_a = vregs.next();
    let count_b = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&count_a, abi::stack_pointer(), a.count),
        abi::load_u64(&count_b, abi::stack_pointer(), b.count),
        abi::compare_registers(&count_a, &count_b),
        abi::branch_ge(&longer),
        abi::move_register(&count_a, &count_b),
        abi::label(&longer),
        abi::add_immediate(&count_a, &count_a, 1),
        abi::store_u64(&count_a, abi::stack_pointer(), capacity),
    ]);
    let result = emit_alloc_magnitude(builder, vregs, capacity, tag, alloc_fail);

    // Everything reloads after the allocation call.
    let add_loop = format!("{symbol}_{tag}_add");
    let skip_a = format!("{symbol}_{tag}_skip_a");
    let skip_b = format!("{symbol}_{tag}_skip_b");
    let add_done = format!("{symbol}_{tag}_add_done");
    let index = vregs.next();
    let acc = vregs.next();
    let dst = vregs.next();
    let data_a = vregs.next();
    let data_b = vregs.next();
    let count_a = vregs.next();
    let count_b = vregs.next();
    let limit = vregs.next();
    let cursor = vregs.next();
    let byte = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&dst, abi::stack_pointer(), result.data),
        abi::load_u64(&data_a, abi::stack_pointer(), a.data),
        abi::load_u64(&data_b, abi::stack_pointer(), b.data),
        abi::load_u64(&count_a, abi::stack_pointer(), a.count),
        abi::load_u64(&count_b, abi::stack_pointer(), b.count),
        abi::load_u64(&limit, abi::stack_pointer(), capacity),
        abi::subtract_immediate(&limit, &limit, 1),
        abi::move_immediate(&index, "Integer", "0"),
        abi::move_immediate(&acc, "Integer", "0"),
        abi::label(&add_loop),
        abi::compare_registers(&index, &limit),
        abi::branch_eq(&add_done),
        // acc holds the incoming carry; add each operand's byte where it has one.
        abi::compare_registers(&index, &count_a),
        abi::branch_ge(&skip_a),
        abi::add_registers(&cursor, &data_a, &index),
        abi::load_u8(&byte, &cursor, 0),
        abi::add_registers(&acc, &acc, &byte),
        abi::label(&skip_a),
        abi::compare_registers(&index, &count_b),
        abi::branch_ge(&skip_b),
        abi::add_registers(&cursor, &data_b, &index),
        abi::load_u8(&byte, &cursor, 0),
        abi::add_registers(&acc, &acc, &byte),
        abi::label(&skip_b),
        // Low byte out; the rest is the carry into the next position.
        abi::add_registers(&cursor, &dst, &index),
        abi::store_u8(&acc, &cursor, 0),
        abi::shift_right_immediate(&acc, &acc, 8),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&add_loop),
        abi::label(&add_done),
        // The final carry is the top byte.
        abi::add_registers(&cursor, &dst, &index),
        abi::store_u8(&acc, &cursor, 0),
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
    let result = emit_alloc_magnitude(builder, vregs, larger.count, tag, alloc_fail);

    let sub_loop = format!("{symbol}_{tag}_sub");
    let no_subtrahend = format!("{symbol}_{tag}_no_subtrahend");
    let no_borrow = format!("{symbol}_{tag}_no_borrow");
    let sub_done = format!("{symbol}_{tag}_sub_done");
    let index = vregs.next();
    let borrow = vregs.next();
    let diff = vregs.next();
    let dst = vregs.next();
    let data_l = vregs.next();
    let data_s = vregs.next();
    let count_l = vregs.next();
    let count_s = vregs.next();
    let cursor = vregs.next();
    let byte = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&dst, abi::stack_pointer(), result.data),
        abi::load_u64(&data_l, abi::stack_pointer(), larger.data),
        abi::load_u64(&data_s, abi::stack_pointer(), smaller.data),
        abi::load_u64(&count_l, abi::stack_pointer(), larger.count),
        abi::load_u64(&count_s, abi::stack_pointer(), smaller.count),
        abi::move_immediate(&index, "Integer", "0"),
        abi::move_immediate(&borrow, "Integer", "0"),
        abi::label(&sub_loop),
        abi::compare_registers(&index, &count_l),
        abi::branch_eq(&sub_done),
        abi::add_registers(&cursor, &data_l, &index),
        abi::load_u8(&diff, &cursor, 0),
        abi::subtract_registers(&diff, &diff, &borrow),
        abi::compare_registers(&index, &count_s),
        abi::branch_ge(&no_subtrahend),
        abi::add_registers(&cursor, &data_s, &index),
        abi::load_u8(&byte, &cursor, 0),
        abi::subtract_registers(&diff, &diff, &byte),
        abi::label(&no_subtrahend),
        // A negative byte difference borrows one from the next position.
        abi::move_immediate(&borrow, "Integer", "0"),
        abi::compare_immediate(&diff, "0"),
        abi::branch_ge(&no_borrow),
        abi::add_immediate(&diff, &diff, 256),
        abi::move_immediate(&borrow, "Integer", "1"),
        abi::label(&no_borrow),
        abi::add_registers(&cursor, &dst, &index),
        abi::store_u8(&diff, &cursor, 0),
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
    emit_build_int(builder, vregs, &sum, sum_count, sign_a, &format!("{tag}_sum"));
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

// ---------------------------------------------------------------------------
// Multiplication (plan-127-B Phase 2).
// ---------------------------------------------------------------------------

/// `|a| * |b|` into a fresh result sized `countA + countB`, schoolbook over byte limbs.
/// Returns the result's slots and the slot holding the written byte count (the full
/// capacity; `emit_build_int` trims the top). Branches to `alloc_fail` when the result
/// cannot be made.
///
/// Every partial step is `dst[i + j] + a[i] * b[j] + carry <= 255 + 255 * 255 + 255 =
/// 65535`, so no widening multiply is needed and nothing can overflow. Row `i` writes
/// positions `i .. i + countB`; its final carry lands in `i + countB`, which no earlier
/// row has written (row `i - 1` stops at `i - 1 + countB`), so the carry is at most 255
/// and needs no further propagation. The result region starts zeroed
/// (`emit_alloc_magnitude`), which is what the accumulation reads.
pub(crate) fn emit_mul_magnitude(
    builder: &mut CodeBuilder,
    vregs: &mut Vregs,
    a: &IntSlots,
    b: &IntSlots,
    tag: &str,
    alloc_fail: &str,
) -> (ResultSlots, usize) {
    let symbol = builder.current_symbol.clone();
    let capacity = builder.allocate_stack_object(&format!("big_{tag}_capacity"), 8);
    let row_slot = builder.allocate_stack_object(&format!("big_{tag}_row"), 8);
    let total = vregs.next();
    let count_b = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&total, abi::stack_pointer(), a.count),
        abi::load_u64(&count_b, abi::stack_pointer(), b.count),
        abi::add_registers(&total, &total, &count_b),
        abi::store_u64(&total, abi::stack_pointer(), capacity),
    ]);
    let result = emit_alloc_magnitude(builder, vregs, capacity, tag, alloc_fail);

    let row_loop = format!("{symbol}_{tag}_row");
    let row_next = format!("{symbol}_{tag}_row_next");
    let rows_done = format!("{symbol}_{tag}_rows_done");
    let column_loop = format!("{symbol}_{tag}_column");
    let columns_done = format!("{symbol}_{tag}_columns_done");
    let row = vregs.next();
    let count_a = vregs.next();
    builder.instructions.extend([
        abi::move_immediate(&row, "Integer", "0"),
        abi::store_u64(&row, abi::stack_pointer(), row_slot),
        abi::label(&row_loop),
        abi::load_u64(&row, abi::stack_pointer(), row_slot),
        abi::load_u64(&count_a, abi::stack_pointer(), a.count),
        abi::compare_registers(&row, &count_a),
        abi::branch_eq(&rows_done),
    ]);
    // One row: a[row] times every byte of b, accumulated into dst[row ..].
    let multiplier = vregs.next();
    let data = vregs.next();
    let row_base = vregs.next();
    let column = vregs.next();
    let count_b = vregs.next();
    let data_b = vregs.next();
    let acc = vregs.next();
    let cursor = vregs.next();
    let byte = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&data, abi::stack_pointer(), a.data),
        abi::add_registers(&data, &data, &row),
        abi::load_u8(&multiplier, &data, 0),
        // A zero byte contributes nothing; the region is already zero there.
        abi::compare_immediate(&multiplier, "0"),
        abi::branch_eq(&row_next),
        abi::load_u64(&row_base, abi::stack_pointer(), result.data),
        abi::add_registers(&row_base, &row_base, &row),
        abi::load_u64(&data_b, abi::stack_pointer(), b.data),
        abi::load_u64(&count_b, abi::stack_pointer(), b.count),
        abi::move_immediate(&column, "Integer", "0"),
        abi::move_immediate(&acc, "Integer", "0"),
        abi::label(&column_loop),
        abi::compare_registers(&column, &count_b),
        abi::branch_eq(&columns_done),
        // acc = carry + a[row] * b[column] + dst[row + column]
        abi::add_registers(&cursor, &data_b, &column),
        abi::load_u8(&byte, &cursor, 0),
        abi::multiply_registers(&byte, &byte, &multiplier),
        abi::add_registers(&acc, &acc, &byte),
        abi::add_registers(&cursor, &row_base, &column),
        abi::load_u8(&byte, &cursor, 0),
        abi::add_registers(&acc, &acc, &byte),
        abi::store_u8(&acc, &cursor, 0),
        abi::shift_right_immediate(&acc, &acc, 8),
        abi::add_immediate(&column, &column, 1),
        abi::branch(&column_loop),
        abi::label(&columns_done),
        // The row's carry: position row + countB, untouched by any earlier row.
        abi::add_registers(&cursor, &row_base, &column),
        abi::store_u8(&acc, &cursor, 0),
        abi::label(&row_next),
        abi::add_immediate(&row, &row, 1),
        abi::store_u64(&row, abi::stack_pointer(), row_slot),
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
    let start = emit_alloc_magnitude(builder, vregs, capacity, &format!("{tag}_identity"), alloc_fail);
    if multiply {
        let data = vregs.next();
        let one = vregs.next();
        builder.instructions.extend([
            abi::load_u64(&data, abi::stack_pointer(), start.data),
            abi::move_immediate(&one, "Integer", "1"),
            abi::store_u8(&one, &data, 0),
        ]);
    }
    emit_build_int(builder, vregs, &start, capacity, negative, &format!("{tag}_identity"));

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
        emit_mul_int(builder, vregs, &acc, &element, &format!("{tag}_step"), alloc_fail);
    } else {
        emit_add_int(builder, vregs, &acc, &element, false, &format!("{tag}_step"), alloc_fail);
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

// ---------------------------------------------------------------------------
// Text (plan-127-B Phase 4).
// ---------------------------------------------------------------------------

/// The digits of a loaded operand in `radix` (2..=36, in `radix_slot`) as a new `String`
/// in `RESULT_VALUE_REGISTER`: an optional `-`, then the digits most significant first,
/// `0`-`9` then lowercase `a`-`z`, and `0` for zero. Branches to `alloc_fail` when a
/// block cannot be made.
///
/// This is `emit_div_small` (plan-127-B §4.1) in place: a working copy of the magnitude
/// is divided by the radix from the high byte down, carrying a remainder that stays
/// below the radix, so every step's dividend is below `radix * 256 <= 9216`. Each pass
/// yields the next digit, least significant first, and drops the copy's zero high
/// bytes. The working copy and the digit buffer are this helper's own and are released
/// at the sizes they were made with.
pub(crate) fn emit_int_to_string(
    builder: &mut CodeBuilder,
    vregs: &mut Vregs,
    operand: &IntSlots,
    radix_slot: usize,
    tag: &str,
    alloc_fail: &str,
) {
    let symbol = builder.current_symbol.clone();
    let work_slot = builder.allocate_stack_object(&format!("big_{tag}_work"), 8);
    let work_size = builder.allocate_stack_object(&format!("big_{tag}_work_size"), 8);
    let digits_slot = builder.allocate_stack_object(&format!("big_{tag}_digits"), 8);
    let digits_size = builder.allocate_stack_object(&format!("big_{tag}_digits_size"), 8);
    let remaining_slot = builder.allocate_stack_object(&format!("big_{tag}_remaining"), 8);
    let written_slot = builder.allocate_stack_object(&format!("big_{tag}_written"), 8);
    let string_slot = builder.allocate_stack_object(&format!("big_{tag}_string"), 8);

    // Sizes: the working copy holds the magnitude; radix 2 writes at most 8 digits a
    // byte, plus the sign.
    let size = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&size, abi::stack_pointer(), operand.count),
        abi::add_immediate(&size, &size, 1),
        abi::store_u64(&size, abi::stack_pointer(), work_size),
        abi::load_u64(&size, abi::stack_pointer(), operand.count),
        abi::shift_left_immediate(&size, &size, 3),
        abi::add_immediate(&size, &size, 2),
        abi::store_u64(&size, abi::stack_pointer(), digits_size),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), work_size),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(&symbol, &mut builder.instructions, &mut builder.relocations, alloc_fail);
    builder.instructions.extend([
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), work_slot),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), digits_size),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(&symbol, &mut builder.instructions, &mut builder.relocations, alloc_fail);
    builder.instructions.extend([
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), digits_slot),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), written_slot),
    ]);

    // Copy the magnitude into the working buffer.
    let copy_loop = format!("{symbol}_{tag}_copy");
    let copied = format!("{symbol}_{tag}_copied");
    let src = vregs.next();
    let dst = vregs.next();
    let count = vregs.next();
    let index = vregs.next();
    let at = vregs.next();
    let byte = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&src, abi::stack_pointer(), operand.data),
        abi::load_u64(&dst, abi::stack_pointer(), work_slot),
        abi::load_u64(&count, abi::stack_pointer(), operand.count),
        abi::store_u64(&count, abi::stack_pointer(), remaining_slot),
        abi::move_immediate(&index, "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers(&index, &count),
        abi::branch_eq(&copied),
        abi::add_registers(&at, &src, &index),
        abi::load_u8(&byte, &at, 0),
        abi::add_registers(&at, &dst, &index),
        abi::store_u8(&byte, &at, 0),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&copy_loop),
        abi::label(&copied),
    ]);

    // Zero writes a single '0'.
    let digit_loop = format!("{symbol}_{tag}_digit");
    let digits_done = format!("{symbol}_{tag}_digits_done");
    let nonzero = format!("{symbol}_{tag}_nonzero");
    let zero_buffer = vregs.next();
    let zero_value = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&count, abi::stack_pointer(), remaining_slot),
        abi::compare_immediate(&count, "0"),
        abi::branch_ne(&nonzero),
        abi::load_u64(&zero_buffer, abi::stack_pointer(), digits_slot),
        abi::move_immediate(&zero_value, "Integer", "48"),
        abi::store_u8(&zero_value, &zero_buffer, 0),
        abi::move_immediate(&zero_value, "Integer", "1"),
        abi::store_u64(&zero_value, abi::stack_pointer(), written_slot),
        abi::branch(&digits_done),
        abi::label(&nonzero),
    ]);

    // One pass per digit: divide the working copy by the radix, high byte down.
    let divide_loop = format!("{symbol}_{tag}_divide");
    let divided = format!("{symbol}_{tag}_divided");
    let letter = format!("{symbol}_{tag}_letter");
    let placed = format!("{symbol}_{tag}_placed");
    let trim_loop = format!("{symbol}_{tag}_trim");
    let trimmed = format!("{symbol}_{tag}_trimmed");
    let work = vregs.next();
    let remaining = vregs.next();
    let position = vregs.next();
    let remainder = vregs.next();
    let dividend = vregs.next();
    let quotient = vregs.next();
    let radix = vregs.next();
    let cursor = vregs.next();
    builder.instructions.extend([
        abi::label(&digit_loop),
        abi::load_u64(&remaining, abi::stack_pointer(), remaining_slot),
        abi::compare_immediate(&remaining, "0"),
        abi::branch_eq(&digits_done),
        abi::load_u64(&work, abi::stack_pointer(), work_slot),
        abi::load_u64(&radix, abi::stack_pointer(), radix_slot),
        abi::move_register(&position, &remaining),
        abi::move_immediate(&remainder, "Integer", "0"),
        abi::label(&divide_loop),
        abi::compare_immediate(&position, "0"),
        abi::branch_eq(&divided),
        abi::subtract_immediate(&position, &position, 1),
        abi::add_registers(&cursor, &work, &position),
        abi::load_u8(&dividend, &cursor, 0),
        abi::shift_left_immediate(&remainder, &remainder, 8),
        abi::or_registers(&dividend, &dividend, &remainder),
        abi::unsigned_divide_registers(&quotient, &dividend, &radix),
        abi::store_u8(&quotient, &cursor, 0),
        // remainder = dividend - quotient * radix
        abi::multiply_subtract_registers(&remainder, &quotient, &radix, &dividend),
        abi::branch(&divide_loop),
        abi::label(&divided),
        // The remainder is the next digit, least significant first.
        abi::compare_immediate(&remainder, "10"),
        abi::branch_ge(&letter),
        abi::add_immediate(&remainder, &remainder, 48),
        abi::branch(&placed),
        abi::label(&letter),
        abi::add_immediate(&remainder, &remainder, 87),
        abi::label(&placed),
        abi::load_u64(&position, abi::stack_pointer(), written_slot),
        abi::load_u64(&cursor, abi::stack_pointer(), digits_slot),
        abi::add_registers(&cursor, &cursor, &position),
        abi::store_u8(&remainder, &cursor, 0),
        abi::add_immediate(&position, &position, 1),
        abi::store_u64(&position, abi::stack_pointer(), written_slot),
        // Drop the working copy's zero high bytes: while work[remaining - 1] == 0.
        abi::label(&trim_loop),
        abi::compare_immediate(&remaining, "0"),
        abi::branch_eq(&trimmed),
        abi::subtract_immediate(&cursor, &remaining, 1),
        abi::add_registers(&cursor, &work, &cursor),
        abi::load_u8(&dividend, &cursor, 0),
        abi::compare_immediate(&dividend, "0"),
        abi::branch_ne(&trimmed),
        abi::subtract_immediate(&remaining, &remaining, 1),
        abi::branch(&trim_loop),
        abi::label(&trimmed),
        abi::store_u64(&remaining, abi::stack_pointer(), remaining_slot),
        abi::branch(&digit_loop),
        abi::label(&digits_done),
    ]);

    // The sign, written after the digits so it lands first once reversed.
    let unsigned = format!("{symbol}_{tag}_unsigned");
    let flag = vregs.next();
    let written = vregs.next();
    let buffer = vregs.next();
    let minus = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&flag, abi::stack_pointer(), operand.negative),
        abi::compare_immediate(&flag, "0"),
        abi::branch_eq(&unsigned),
        abi::load_u64(&written, abi::stack_pointer(), written_slot),
        abi::load_u64(&buffer, abi::stack_pointer(), digits_slot),
        abi::add_registers(&buffer, &buffer, &written),
        abi::move_immediate(&minus, "Integer", "45"),
        abi::store_u8(&minus, &buffer, 0),
        abi::add_immediate(&written, &written, 1),
        abi::store_u64(&written, abi::stack_pointer(), written_slot),
        abi::label(&unsigned),
        // The String: length word, the bytes reversed, a trailing NUL.
        abi::load_u64(abi::return_register(), abi::stack_pointer(), written_slot),
        abi::add_immediate(abi::return_register(), abi::return_register(), 9),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(&symbol, &mut builder.instructions, &mut builder.relocations, alloc_fail);
    let reverse_loop = format!("{symbol}_{tag}_reverse");
    let reversed = format!("{symbol}_{tag}_reversed");
    let string = vregs.next();
    let length = vregs.next();
    let source = vregs.next();
    let target = vregs.next();
    let character = vregs.next();
    let step = vregs.next();
    builder.instructions.extend([
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), string_slot),
        abi::load_u64(&string, abi::stack_pointer(), string_slot),
        abi::load_u64(&length, abi::stack_pointer(), written_slot),
        abi::store_u64(&length, &string, 0),
        abi::load_u64(&source, abi::stack_pointer(), digits_slot),
        abi::move_immediate(&step, "Integer", "0"),
        abi::label(&reverse_loop),
        abi::compare_registers(&step, &length),
        abi::branch_eq(&reversed),
        // string[8 + step] = digits[length - 1 - step]
        abi::subtract_registers(&target, &length, &step),
        abi::subtract_immediate(&target, &target, 1),
        abi::add_registers(&target, &source, &target),
        abi::load_u8(&character, &target, 0),
        abi::add_registers(&target, &string, &step),
        abi::store_u8(&character, &target, 8),
        abi::add_immediate(&step, &step, 1),
        abi::branch(&reverse_loop),
        abi::label(&reversed),
        abi::add_registers(&target, &string, &length),
        abi::store_u8(abi::ZERO, &target, 8),
    ]);

    // Release the working copy and the digit buffer at the sizes they were made with.
    for (pointer, size) in [(work_slot, work_size), (digits_slot, digits_size)] {
        builder.instructions.extend([
            abi::load_u64(abi::c_arg(0), abi::stack_pointer(), pointer),
            abi::load_u64(abi::c_arg(1), abi::stack_pointer(), size),
        ]);
        emit_arena_free(&symbol, &mut builder.instructions, &mut builder.relocations);
    }
    builder.instructions.push(abi::load_u64(
        RESULT_VALUE_REGISTER,
        abi::stack_pointer(),
        string_slot,
    ));
}

// ---------------------------------------------------------------------------
// Division (plan-127-C Phase 1): Knuth, TAOCP Vol. 2 §4.3.1, Algorithm D, over byte
// limbs (base 256).
//
// Two different "normalize"s meet in this section and must not be confused:
//
// - Algorithm D's **operand normalization** (step D1) left-shifts the divisor and the
//   dividend by the same `s` bits so the divisor's top byte has its high bit set. That
//   is what bounds each quotient-digit estimate to at most two too large; step D8 shifts
//   the remainder back by `s`. It never touches a record.
// - `emit_build_int`'s **record normalization** is canonical form — trailing zero bytes
//   dropped, zero never negative. The caller applies it to the quotient and the
//   remainder afterwards, exactly as for every other `big` result.
//
// Every value a later step needs lives in a frame slot; the only calls are the block
// allocations and releases, and each loop body is call-free.
// ---------------------------------------------------------------------------

/// Where [`emit_div_mod_magnitude`] leaves its two results: each a result record plus the
/// slot holding its written byte count, ready for `emit_build_int`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DivModSlots {
    pub(crate) quotient: ResultSlots,
    pub(crate) quotient_count: usize,
    pub(crate) remainder: ResultSlots,
    pub(crate) remainder_count: usize,
}

/// Copy the word in frame slot `from` into frame slot `to`.
fn emit_move_slot(builder: &mut CodeBuilder, vregs: &mut Vregs, from: usize, to: usize) {
    let value = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&value, abi::stack_pointer(), from),
        abi::store_u64(&value, abi::stack_pointer(), to),
    ]);
}

/// Record a freshly made result's slots as `target`'s.
fn emit_adopt_result(
    builder: &mut CodeBuilder,
    vregs: &mut Vregs,
    made: &ResultSlots,
    target: &ResultSlots,
) {
    emit_move_slot(builder, vregs, made.record, target.record);
    emit_move_slot(builder, vregs, made.data, target.data);
}

/// `|u| / |v|` and `|u| mod |v|` over two loaded operands. **The caller guarantees `v` is
/// not zero.** Three paths, each leaving its results in the same [`DivModSlots`]:
///
/// - `|u|` has fewer bytes than `|v|`: quotient zero, remainder `|u|`;
/// - `|v|` is one byte: short division from the high byte down (no normalization needed);
/// - otherwise Algorithm D, steps D1–D8, with both the estimate correction (D3) and the
///   add-back (D6) — the two places implementations go wrong.
///
/// Branches to `alloc_fail` when a block cannot be made.
pub(crate) fn emit_div_mod_magnitude(
    builder: &mut CodeBuilder,
    vregs: &mut Vregs,
    u: &IntSlots,
    v: &IntSlots,
    tag: &str,
    alloc_fail: &str,
) -> DivModSlots {
    let symbol = builder.current_symbol.clone();
    let slot = |builder: &mut CodeBuilder, name: &str| {
        builder.allocate_stack_object(&format!("big_{tag}_{name}"), 8)
    };
    let out = DivModSlots {
        quotient: ResultSlots {
            record: slot(builder, "q_record"),
            data: slot(builder, "q_data"),
        },
        quotient_count: slot(builder, "q_count"),
        remainder: ResultSlots {
            record: slot(builder, "r_record"),
            data: slot(builder, "r_data"),
        },
        remainder_count: slot(builder, "r_count"),
    };
    let zero_slot = slot(builder, "zero");
    let one_slot = slot(builder, "one");
    let rem_slot = slot(builder, "short_rem");
    let shift_slot = slot(builder, "shift");
    let m1_slot = slot(builder, "m1");
    let vn_slot = slot(builder, "vn");
    let un_slot = slot(builder, "un");
    let un_size_slot = slot(builder, "un_size");
    let j1_slot = slot(builder, "j1");
    let qhat_slot = slot(builder, "qhat");
    let borrow_slot = slot(builder, "borrow");

    let smaller = format!("{symbol}_{tag}_smaller");
    let short = format!("{symbol}_{tag}_short");
    let general = format!("{symbol}_{tag}_general");
    let joined = format!("{symbol}_{tag}_joined");

    // --- Dispatch --------------------------------------------------------------------
    {
        let (count_u, count_v, one) = (vregs.next(), vregs.next(), vregs.next());
        builder.instructions.extend([
            abi::store_u64(abi::ZERO, abi::stack_pointer(), zero_slot),
            abi::move_immediate(&one, "Integer", "1"),
            abi::store_u64(&one, abi::stack_pointer(), one_slot),
            abi::load_u64(&count_u, abi::stack_pointer(), u.count),
            abi::load_u64(&count_v, abi::stack_pointer(), v.count),
            abi::compare_registers(&count_u, &count_v),
            abi::branch_lo(&smaller),
            abi::compare_immediate(&count_v, "1"),
            abi::branch_eq(&short),
            abi::branch(&general),
        ]);
    }

    // --- |u| shorter than |v|: quotient 0, remainder |u| ------------------------------
    builder.instructions.push(abi::label(&smaller));
    let made = emit_alloc_magnitude(builder, vregs, zero_slot, &format!("{tag}_aq"), alloc_fail);
    emit_adopt_result(builder, vregs, &made, &out.quotient);
    emit_move_slot(builder, vregs, zero_slot, out.quotient_count);
    let made = emit_alloc_magnitude(builder, vregs, u.count, &format!("{tag}_ar"), alloc_fail);
    emit_adopt_result(builder, vregs, &made, &out.remainder);
    emit_move_slot(builder, vregs, u.count, out.remainder_count);
    {
        let copy = format!("{symbol}_{tag}_a_copy");
        let copied = format!("{symbol}_{tag}_a_copied");
        let (src, dst, count, index, at, byte) = (
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
        );
        builder.instructions.extend([
            abi::load_u64(&src, abi::stack_pointer(), u.data),
            abi::load_u64(&dst, abi::stack_pointer(), out.remainder.data),
            abi::load_u64(&count, abi::stack_pointer(), u.count),
            abi::move_immediate(&index, "Integer", "0"),
            abi::label(&copy),
            abi::compare_registers(&index, &count),
            abi::branch_eq(&copied),
            abi::add_registers(&at, &src, &index),
            abi::load_u8(&byte, &at, 0),
            abi::add_registers(&at, &dst, &index),
            abi::store_u8(&byte, &at, 0),
            abi::add_immediate(&index, &index, 1),
            abi::branch(&copy),
            abi::label(&copied),
            abi::branch(&joined),
        ]);
    }

    // --- One-byte divisor: short division ----------------------------------------------
    builder.instructions.push(abi::label(&short));
    let made = emit_alloc_magnitude(builder, vregs, u.count, &format!("{tag}_bq"), alloc_fail);
    emit_adopt_result(builder, vregs, &made, &out.quotient);
    emit_move_slot(builder, vregs, u.count, out.quotient_count);
    {
        let divide = format!("{symbol}_{tag}_b_divide");
        let divided = format!("{symbol}_{tag}_b_divided");
        let (src, dst, divisor, position, remainder, dividend, digit, at) = (
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
        );
        builder.instructions.extend([
            abi::load_u64(&src, abi::stack_pointer(), u.data),
            abi::load_u64(&dst, abi::stack_pointer(), out.quotient.data),
            abi::load_u64(&divisor, abi::stack_pointer(), v.data),
            abi::load_u8(&divisor, &divisor, 0),
            abi::load_u64(&position, abi::stack_pointer(), u.count),
            abi::move_immediate(&remainder, "Integer", "0"),
            abi::label(&divide),
            abi::compare_immediate(&position, "0"),
            abi::branch_eq(&divided),
            abi::subtract_immediate(&position, &position, 1),
            abi::add_registers(&at, &src, &position),
            abi::load_u8(&dividend, &at, 0),
            abi::shift_left_immediate(&remainder, &remainder, 8),
            abi::or_registers(&dividend, &dividend, &remainder),
            abi::unsigned_divide_registers(&digit, &dividend, &divisor),
            // remainder = dividend - digit * divisor
            abi::multiply_subtract_registers(&remainder, &digit, &divisor, &dividend),
            abi::add_registers(&at, &dst, &position),
            abi::store_u8(&digit, &at, 0),
            abi::branch(&divide),
            abi::label(&divided),
            abi::store_u64(&remainder, abi::stack_pointer(), rem_slot),
        ]);
    }
    let made = emit_alloc_magnitude(builder, vregs, one_slot, &format!("{tag}_br"), alloc_fail);
    emit_adopt_result(builder, vregs, &made, &out.remainder);
    emit_move_slot(builder, vregs, one_slot, out.remainder_count);
    {
        let (dst, remainder) = (vregs.next(), vregs.next());
        builder.instructions.extend([
            abi::load_u64(&dst, abi::stack_pointer(), out.remainder.data),
            abi::load_u64(&remainder, abi::stack_pointer(), rem_slot),
            abi::store_u8(&remainder, &dst, 0),
            abi::branch(&joined),
        ]);
    }

    // --- Algorithm D: n = |v| bytes >= 2, m = |u| - n >= 0 ------------------------------
    builder.instructions.push(abi::label(&general));
    {
        // s = clz8(v[n-1]); m + 1; the shifted dividend needs |u| + 1 bytes.
        let (count_u, count_v, top, zeros, size) = (
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
        );
        builder.instructions.extend([
            abi::load_u64(&count_u, abi::stack_pointer(), u.count),
            abi::load_u64(&count_v, abi::stack_pointer(), v.count),
            abi::load_u64(&top, abi::stack_pointer(), v.data),
            abi::add_registers(&top, &top, &count_v),
            abi::subtract_immediate(&top, &top, 1),
            abi::load_u8(&top, &top, 0),
            // A trimmed top byte is non-zero: its 64-bit leading-zero count is 56..63.
            abi::count_leading_zeros(&zeros, &top),
            abi::subtract_immediate(&zeros, &zeros, 56),
            abi::store_u64(&zeros, abi::stack_pointer(), shift_slot),
            abi::subtract_registers(&size, &count_u, &count_v),
            abi::add_immediate(&size, &size, 1),
            abi::store_u64(&size, abi::stack_pointer(), m1_slot),
            abi::add_immediate(&size, &count_u, 1),
            abi::store_u64(&size, abi::stack_pointer(), un_size_slot),
            abi::move_register(abi::return_register(), &count_v),
            abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        ]);
    }
    emit_alloc(&symbol, &mut builder.instructions, &mut builder.relocations, alloc_fail);
    builder.instructions.extend([
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), vn_slot),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), un_size_slot),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(&symbol, &mut builder.instructions, &mut builder.relocations, alloc_fail);
    builder
        .instructions
        .push(abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), un_slot));
    let made = emit_alloc_magnitude(builder, vregs, m1_slot, &format!("{tag}_dq"), alloc_fail);
    emit_adopt_result(builder, vregs, &made, &out.quotient);
    emit_move_slot(builder, vregs, m1_slot, out.quotient_count);
    let made = emit_alloc_magnitude(builder, vregs, v.count, &format!("{tag}_dr"), alloc_fail);
    emit_adopt_result(builder, vregs, &made, &out.remainder);
    emit_move_slot(builder, vregs, v.count, out.remainder_count);

    // D1: vn = v << s over n bytes (no carry out: the top byte's high bit lands exactly on
    // bit 7); un = u << s over |u| bytes, with the carry out as byte |u|.
    for (source, target, count, store_carry, name) in [
        (v, vn_slot, v.count, false, "v"),
        (u, un_slot, u.count, true, "u"),
    ] {
        let shift = format!("{symbol}_{tag}_d1_{name}");
        let shifted = format!("{symbol}_{tag}_d1_{name}_done");
        let (src, dst, n, bits, index, carry, value, at) = (
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
        );
        builder.instructions.extend([
            abi::load_u64(&src, abi::stack_pointer(), source.data),
            abi::load_u64(&dst, abi::stack_pointer(), target),
            abi::load_u64(&n, abi::stack_pointer(), count),
            abi::load_u64(&bits, abi::stack_pointer(), shift_slot),
            abi::move_immediate(&index, "Integer", "0"),
            abi::move_immediate(&carry, "Integer", "0"),
            abi::label(&shift),
            abi::compare_registers(&index, &n),
            abi::branch_eq(&shifted),
            abi::add_registers(&at, &src, &index),
            abi::load_u8(&value, &at, 0),
            abi::shift_left_variable(&value, &value, &bits),
            abi::or_registers(&value, &value, &carry),
            abi::add_registers(&at, &dst, &index),
            abi::store_u8(&value, &at, 0),
            abi::shift_right_immediate(&carry, &value, 8),
            abi::add_immediate(&index, &index, 1),
            abi::branch(&shift),
            abi::label(&shifted),
        ]);
        if store_carry {
            builder.instructions.extend([
                abi::add_registers(&at, &dst, &index),
                abi::store_u8(&carry, &at, 0),
            ]);
        }
    }

    // D2: j runs from m down to 0; `j1_slot` holds j + 1 so the loop test never goes
    // below zero.
    let digit_loop = format!("{symbol}_{tag}_d2");
    let digit_next = format!("{symbol}_{tag}_d2_next");
    let digits_done = format!("{symbol}_{tag}_d2_done");
    emit_move_slot(builder, vregs, m1_slot, j1_slot);
    builder.instructions.push(abi::label(&digit_loop));

    // D3: estimate qhat from the top two bytes of the running remainder over the divisor's
    // top byte, then correct it downward while it is too large.
    {
        let test = format!("{symbol}_{tag}_d3_test");
        let correct = format!("{symbol}_{tag}_d3_correct");
        let estimated = format!("{symbol}_{tag}_d3_estimated");
        let (j1, base, n, at, number, low, vtop, vnext, qhat, rhat, lhs, rhs, limit) = (
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
        );
        builder.instructions.extend([
            abi::load_u64(&j1, abi::stack_pointer(), j1_slot),
            abi::compare_immediate(&j1, "0"),
            abi::branch_eq(&digits_done),
            abi::load_u64(&n, abi::stack_pointer(), v.count),
            // base = &un[j]
            abi::load_u64(&base, abi::stack_pointer(), un_slot),
            abi::add_registers(&base, &base, &j1),
            abi::subtract_immediate(&base, &base, 1),
            // number = un[j + n] * 256 + un[j + n - 1]
            abi::add_registers(&at, &base, &n),
            abi::load_u8(&number, &at, 0),
            abi::shift_left_immediate(&number, &number, 8),
            abi::subtract_immediate(&at, &at, 1),
            abi::load_u8(&low, &at, 0),
            abi::or_registers(&number, &number, &low),
            // vtop = vn[n - 1], vnext = vn[n - 2]
            abi::load_u64(&at, abi::stack_pointer(), vn_slot),
            abi::add_registers(&at, &at, &n),
            abi::subtract_immediate(&at, &at, 1),
            abi::load_u8(&vtop, &at, 0),
            abi::subtract_immediate(&at, &at, 1),
            abi::load_u8(&vnext, &at, 0),
            abi::unsigned_divide_registers(&qhat, &number, &vtop),
            abi::multiply_subtract_registers(&rhat, &qhat, &vtop, &number),
            abi::move_immediate(&limit, "Integer", "256"),
            abi::label(&test),
            // qhat >= 256 is too large.
            abi::compare_registers(&qhat, &limit),
            abi::branch_ge(&correct),
            // qhat * vn[n-2] > rhat * 256 + un[j + n - 2] is too large.
            abi::multiply_registers(&lhs, &qhat, &vnext),
            abi::shift_left_immediate(&rhs, &rhat, 8),
            abi::add_registers(&at, &base, &n),
            abi::subtract_immediate(&at, &at, 2),
            abi::load_u8(&low, &at, 0),
            abi::add_registers(&rhs, &rhs, &low),
            abi::compare_registers(&lhs, &rhs),
            abi::branch_hi(&correct),
            abi::branch(&estimated),
            abi::label(&correct),
            abi::subtract_immediate(&qhat, &qhat, 1),
            abi::add_registers(&rhat, &rhat, &vtop),
            // Once rhat reaches 256 the second test can no longer fire.
            abi::compare_registers(&rhat, &limit),
            abi::branch_lo(&test),
            abi::label(&estimated),
            abi::store_u64(&qhat, abi::stack_pointer(), qhat_slot),
        ]);
    }

    // D4: un[j .. j + n] -= qhat * vn, byte by byte, tracking a multiply carry and a
    // subtract borrow separately. D5: q[j] = qhat.
    {
        let multiply = format!("{symbol}_{tag}_d4");
        let multiplied = format!("{symbol}_{tag}_d4_done");
        let no_borrow = format!("{symbol}_{tag}_d4_no_borrow");
        let top_no_borrow = format!("{symbol}_{tag}_d4_top_no_borrow");
        let (j1, base, vn, n, qhat, index, carry, borrow, product, work, mask) = (
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
        );
        builder.instructions.extend([
            abi::load_u64(&j1, abi::stack_pointer(), j1_slot),
            abi::load_u64(&base, abi::stack_pointer(), un_slot),
            abi::add_registers(&base, &base, &j1),
            abi::subtract_immediate(&base, &base, 1),
            abi::load_u64(&vn, abi::stack_pointer(), vn_slot),
            abi::load_u64(&n, abi::stack_pointer(), v.count),
            abi::load_u64(&qhat, abi::stack_pointer(), qhat_slot),
            abi::move_immediate(&mask, "Integer", "255"),
            abi::move_immediate(&index, "Integer", "0"),
            abi::move_immediate(&carry, "Integer", "0"),
            abi::move_immediate(&borrow, "Integer", "0"),
            abi::label(&multiply),
            abi::compare_registers(&index, &n),
            abi::branch_eq(&multiplied),
            // product = qhat * vn[i] + carry; carry = product >> 8
            abi::add_registers(&work, &vn, &index),
            abi::load_u8(&product, &work, 0),
            abi::multiply_registers(&product, &product, &qhat),
            abi::add_registers(&product, &product, &carry),
            abi::shift_right_immediate(&carry, &product, 8),
            abi::and_registers(&product, &product, &mask),
            // work = un[j + i] - (product & 255) - borrow
            abi::add_registers(&work, &base, &index),
            abi::load_u8(&work, &work, 0),
            abi::subtract_registers(&work, &work, &product),
            abi::subtract_registers(&work, &work, &borrow),
            abi::move_immediate(&borrow, "Integer", "0"),
            abi::compare_immediate(&work, "0"),
            abi::branch_ge(&no_borrow),
            abi::add_immediate(&work, &work, 256),
            abi::move_immediate(&borrow, "Integer", "1"),
            abi::label(&no_borrow),
            abi::add_registers(&product, &base, &index),
            abi::store_u8(&work, &product, 0),
            abi::add_immediate(&index, &index, 1),
            abi::branch(&multiply),
            abi::label(&multiplied),
            // The top position: un[j + n] -= carry + borrow.
            abi::add_registers(&product, &base, &n),
            abi::load_u8(&work, &product, 0),
            abi::subtract_registers(&work, &work, &carry),
            abi::subtract_registers(&work, &work, &borrow),
            abi::move_immediate(&borrow, "Integer", "0"),
            abi::compare_immediate(&work, "0"),
            abi::branch_ge(&top_no_borrow),
            abi::add_immediate(&work, &work, 256),
            abi::move_immediate(&borrow, "Integer", "1"),
            abi::label(&top_no_borrow),
            abi::store_u8(&work, &product, 0),
            abi::store_u64(&borrow, abi::stack_pointer(), borrow_slot),
            // D5: q[j] = qhat.
            abi::load_u64(&product, abi::stack_pointer(), out.quotient.data),
            abi::add_registers(&product, &product, &j1),
            abi::subtract_immediate(&product, &product, 1),
            abi::store_u8(&qhat, &product, 0),
            abi::compare_immediate(&borrow, "0"),
            abi::branch_eq(&digit_next),
        ]);
    }

    // D6: the subtraction went negative, so qhat was one too large: q[j] -= 1 and add the
    // divisor back into un[j .. j + n].
    {
        let add_back = format!("{symbol}_{tag}_d6");
        let added = format!("{symbol}_{tag}_d6_done");
        let (j1, base, vn, n, index, carry, sum, at, qhat) = (
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
        );
        builder.instructions.extend([
            abi::load_u64(&j1, abi::stack_pointer(), j1_slot),
            abi::load_u64(&at, abi::stack_pointer(), out.quotient.data),
            abi::add_registers(&at, &at, &j1),
            abi::subtract_immediate(&at, &at, 1),
            abi::load_u64(&qhat, abi::stack_pointer(), qhat_slot),
            abi::subtract_immediate(&qhat, &qhat, 1),
            abi::store_u8(&qhat, &at, 0),
            abi::load_u64(&base, abi::stack_pointer(), un_slot),
            abi::add_registers(&base, &base, &j1),
            abi::subtract_immediate(&base, &base, 1),
            abi::load_u64(&vn, abi::stack_pointer(), vn_slot),
            abi::load_u64(&n, abi::stack_pointer(), v.count),
            abi::move_immediate(&index, "Integer", "0"),
            abi::move_immediate(&carry, "Integer", "0"),
            abi::label(&add_back),
            abi::compare_registers(&index, &n),
            abi::branch_eq(&added),
            abi::add_registers(&at, &vn, &index),
            abi::load_u8(&sum, &at, 0),
            abi::add_registers(&sum, &sum, &carry),
            abi::add_registers(&at, &base, &index),
            abi::load_u8(&carry, &at, 0),
            abi::add_registers(&sum, &sum, &carry),
            abi::store_u8(&sum, &at, 0),
            abi::shift_right_immediate(&carry, &sum, 8),
            abi::add_immediate(&index, &index, 1),
            abi::branch(&add_back),
            abi::label(&added),
            // The carry out cancels the borrow the subtraction left in un[j + n].
            abi::add_registers(&at, &base, &n),
            abi::load_u8(&sum, &at, 0),
            abi::add_registers(&sum, &sum, &carry),
            abi::store_u8(&sum, &at, 0),
        ]);
    }

    // Next digit.
    {
        let j1 = vregs.next();
        builder.instructions.extend([
            abi::label(&digit_next),
            abi::load_u64(&j1, abi::stack_pointer(), j1_slot),
            abi::subtract_immediate(&j1, &j1, 1),
            abi::store_u64(&j1, abi::stack_pointer(), j1_slot),
            abi::branch(&digit_loop),
            abi::label(&digits_done),
        ]);
    }

    // D8: the remainder is un[0 .. n] shifted back right by s.
    {
        let unshift = format!("{symbol}_{tag}_d8");
        let unshifted = format!("{symbol}_{tag}_d8_done");
        let (un, dst, n, bits, back, index, low, high, at) = (
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
        );
        builder.instructions.extend([
            abi::load_u64(&un, abi::stack_pointer(), un_slot),
            abi::load_u64(&dst, abi::stack_pointer(), out.remainder.data),
            abi::load_u64(&n, abi::stack_pointer(), v.count),
            abi::load_u64(&bits, abi::stack_pointer(), shift_slot),
            abi::move_immediate(&back, "Integer", "8"),
            abi::subtract_registers(&back, &back, &bits),
            abi::move_immediate(&index, "Integer", "0"),
            abi::label(&unshift),
            abi::compare_registers(&index, &n),
            abi::branch_eq(&unshifted),
            // r[i] = un[i] >> s | un[i + 1] << (8 - s); un[n] exists (|u| + 1 bytes).
            abi::add_registers(&at, &un, &index),
            abi::load_u8(&low, &at, 0),
            abi::load_u8(&high, &at, 1),
            abi::shift_right_variable(&low, &low, &bits),
            abi::shift_left_variable(&high, &high, &back),
            abi::or_registers(&low, &low, &high),
            abi::add_registers(&at, &dst, &index),
            abi::store_u8(&low, &at, 0),
            abi::add_immediate(&index, &index, 1),
            abi::branch(&unshift),
            abi::label(&unshifted),
        ]);
    }

    // Release the two scratch buffers at the sizes they were made with.
    for (pointer, size) in [(vn_slot, v.count), (un_slot, un_size_slot)] {
        builder.instructions.extend([
            abi::load_u64(abi::c_arg(0), abi::stack_pointer(), pointer),
            abi::load_u64(abi::c_arg(1), abi::stack_pointer(), size),
        ]);
        emit_arena_free(&symbol, &mut builder.instructions, &mut builder.relocations);
    }

    builder.instructions.push(abi::label(&joined));
    out
}

// ---------------------------------------------------------------------------
// Signed division and `DivResult` (plan-127-C Phase 2).
// ---------------------------------------------------------------------------

use crate::codegen::memory::marshal::{
    emit_build_inlined_record_sized, MarshalRegs, RecordBuildScratch,
};
use crate::types::ParameterType;

/// `a / b` and `a mod b` as signed numbers, each normalized, their record addresses left
/// in `quotient_slot` and `remainder_slot`.
///
/// The quotient truncates toward zero and the remainder takes the sign of the dividend —
/// MFBASIC's own `/` and `MOD` (measured: `-7 / 2 = -3`, `-7 MOD 2 = -1`,
/// `7 MOD -2 = 1`) — so `a = b * quotient + remainder` with `|remainder| < |b|`. Branches
/// to `zero_divisor` when `b` is zero and to `alloc_fail` when a block cannot be made.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_div_mod_int(
    builder: &mut CodeBuilder,
    vregs: &mut Vregs,
    a: &IntSlots,
    b: &IntSlots,
    quotient_slot: usize,
    remainder_slot: usize,
    tag: &str,
    zero_divisor: &str,
    alloc_fail: &str,
) {
    let quotient_sign = builder.allocate_stack_object(&format!("big_{tag}_quotient_sign"), 8);
    let (count, flag_a, flag_b) = (vregs.next(), vregs.next(), vregs.next());
    builder.instructions.extend([
        abi::load_u64(&count, abi::stack_pointer(), b.count),
        abi::compare_immediate(&count, "0"),
        abi::branch_eq(zero_divisor),
        abi::load_u64(&flag_a, abi::stack_pointer(), a.negative),
        abi::load_u64(&flag_b, abi::stack_pointer(), b.negative),
        abi::exclusive_or_registers(&flag_a, &flag_a, &flag_b),
        abi::store_u64(&flag_a, abi::stack_pointer(), quotient_sign),
    ]);
    let parts = emit_div_mod_magnitude(builder, vregs, a, b, tag, alloc_fail);
    emit_build_int(
        builder,
        vregs,
        &parts.quotient,
        parts.quotient_count,
        quotient_sign,
        &format!("{tag}_q"),
    );
    builder.instructions.push(abi::store_u64(
        RESULT_VALUE_REGISTER,
        abi::stack_pointer(),
        quotient_slot,
    ));
    emit_build_int(
        builder,
        vregs,
        &parts.remainder,
        parts.remainder_count,
        a.negative,
        &format!("{tag}_r"),
    );
    builder.instructions.push(abi::store_u64(
        RESULT_VALUE_REGISTER,
        abi::stack_pointer(),
        remainder_slot,
    ));
}

/// Release the `big::Int` record whose address is in `record_slot` — a helper's own
/// intermediate that the caller never sees — at the size it was made with
/// (`INT_DATA_OFFSET + dataCapacity`).
pub(crate) fn emit_release_int(builder: &mut CodeBuilder, vregs: &mut Vregs, record_slot: usize) {
    let symbol = builder.current_symbol.clone();
    let (record, size) = (vregs.next(), vregs.next());
    builder.instructions.extend([
        abi::load_u64(&record, abi::stack_pointer(), record_slot),
        abi::add_immediate(&size, &record, INT_MAGNITUDE_BLOCK),
        abi::load_u64(&size, &size, COLLECTION_OFFSET_DATA_CAPACITY),
        abi::add_immediate(&size, &size, INT_DATA_OFFSET),
        abi::move_register(abi::c_arg(0), &record),
        abi::move_register(abi::c_arg(1), &size),
    ]);
    emit_arena_free(&symbol, &mut builder.instructions, &mut builder.relocations);
}

/// A `big::DivResult` holding the two records whose addresses are in `quotient_slot` and
/// `remainder_slot`, left in `RESULT_VALUE_REGISTER`; the two source records are released
/// afterwards. Both fields are flat `big::Int` records, so the record marshaller inlines
/// each into the one block, sized by the caller as `INT_DATA_OFFSET + dataCapacity`.
/// Branches to `alloc_fail` when the block cannot be made.
pub(crate) fn emit_build_div_result(
    builder: &mut CodeBuilder,
    vregs: &mut Vregs,
    quotient_slot: usize,
    remainder_slot: usize,
    tag: &str,
    alloc_fail: &str,
) -> Result<(), String> {
    let symbol = builder.current_symbol.clone();
    let quotient_size = builder.allocate_stack_object(&format!("big_{tag}_quotient_size"), 8);
    let remainder_size = builder.allocate_stack_object(&format!("big_{tag}_remainder_size"), 8);
    for (record_slot, size_slot) in [(quotient_slot, quotient_size), (remainder_slot, remainder_size)] {
        let (record, size) = (vregs.next(), vregs.next());
        builder.instructions.extend([
            abi::load_u64(&record, abi::stack_pointer(), record_slot),
            abi::add_immediate(&size, &record, INT_MAGNITUDE_BLOCK),
            abi::load_u64(&size, &size, COLLECTION_OFFSET_DATA_CAPACITY),
            abi::add_immediate(&size, &size, INT_DATA_OFFSET),
            abi::store_u64(&size, abi::stack_pointer(), size_slot),
        ]);
    }
    let scratch = RecordBuildScratch {
        size: builder.allocate_stack_object(&format!("big_{tag}_record_size"), 8),
        result: builder.allocate_stack_object(&format!("big_{tag}_record"), 8),
        cursor: builder.allocate_stack_object(&format!("big_{tag}_record_cursor"), 8),
        block_size: builder.allocate_stack_object(&format!("big_{tag}_record_block"), 8),
    };
    let regs = MarshalRegs::fresh(vregs);
    emit_build_inlined_record_sized(
        &symbol,
        tag,
        &ParameterType::named(super::DIV_RESULT_TYPE_ID),
        TypeModel::builtin_records(),
        &[quotient_slot, remainder_slot],
        &[Some(quotient_size), Some(remainder_size)],
        &scratch,
        &regs,
        abi::mfb_return(1),
        alloc_fail,
        &mut builder.instructions,
        &mut builder.relocations,
    )?;
    emit_release_int(builder, vregs, quotient_slot);
    emit_release_int(builder, vregs, remainder_slot);
    builder.instructions.push(abi::load_u64(
        RESULT_VALUE_REGISTER,
        abi::stack_pointer(),
        scratch.result,
    ));
    Ok(())
}
