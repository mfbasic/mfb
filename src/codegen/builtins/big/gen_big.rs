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
//!   size, rounded up to whole 64-bit words, with a zeroed magnitude region the member
//!   writes into directly. One block per result; no scratch buffer and no second copy
//!   (Open Decision 1).
//! - [`emit_build_int`] **normalizes** that result — drops trailing zero bytes, forces a
//!   zero value non-negative — and publishes it in the result register. It is the one
//!   place canonical form is established, so it is a property of the package rather
//!   than of each lowering.
//!
//! The arithmetic kernels read the byte magnitude as 64-bit words (bug-775; see "Word
//! access" below): add, subtract and multiply step a word at a time, and division and
//! text conversion a 32-bit limb at a time, so a quotient digit always comes from one
//! 64-bit `udiv`. The stored format stays the public little-endian `List OF Byte`.
//!
//! Everything is kept in frame slots between steps. The only call these emitters make
//! is the arena allocation, and every value a later step needs is reloaded from its
//! slot, so no register is assumed to survive it.

use crate::codegen::collection::layout::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::util::Vregs;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::target::shared::abi::Reg;

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
    emit_trim(
        builder,
        vregs,
        &data,
        &count,
        &negative,
        &format!("{symbol}_{tag}_load"),
    );
    builder.instructions.extend([
        abi::store_u64(&count, abi::stack_pointer(), slots.count),
        abi::store_u64(&negative, abi::stack_pointer(), slots.negative),
    ]);
    slots
}

/// Make a result `big::Int` able to hold `capacity_slot` magnitude bytes: the record
/// block, its slot-0 offset, a kind-2 list header with `count` zero, and a zeroed
/// magnitude region. Branches to `alloc_fail` when the block cannot be made.
///
/// The region is rounded up to whole 64-bit words (bug-775), so the word kernels can
/// store whole words into any result: the header's capacity words record the rounded
/// size, which is the size a copy or a release reads, and the extra bytes start zero,
/// so a member that writes fewer than `capacity_slot` bytes still trims to the value
/// it computed.
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
    let rounded = builder.allocate_stack_object(&format!("big_{tag}_rounded"), 8);
    let size = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&size, abi::stack_pointer(), capacity_slot),
        abi::add_immediate(&size, &size, 7),
        abi::shift_right_immediate(&size, &size, 3),
        abi::shift_left_immediate(&size, &size, 3),
        abi::store_u64(&size, abi::stack_pointer(), rounded),
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
        abi::load_u64(&capacity, abi::stack_pointer(), rounded),
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
        // The magnitude region, zeroed a word at a time (its size is a whole number of
        // words): a member writes only the words it computes, and multiplication
        // accumulates into words it has not written yet.
        abi::add_immediate(&cursor, &record, INT_DATA_OFFSET),
        abi::store_u64(&cursor, abi::stack_pointer(), slots.data),
        abi::add_registers(&end, &cursor, &capacity),
        abi::label(&zero_loop),
        abi::compare_registers(&cursor, &end),
        abi::branch_eq(&zero_done),
        abi::store_u64(abi::ZERO, &cursor, 0),
        abi::add_immediate(&cursor, &cursor, 8),
        abi::branch(&zero_loop),
        abi::label(&zero_done),
    ]);
    slots
}

// ---------------------------------------------------------------------------
// Word access (bug-775).
//
// The public magnitude is a little-endian `List OF Byte`, and every target is
// little-endian, so its bytes ARE a little-endian array of 64-bit words — except that
// an operand's byte count need not be a multiple of eight, and a hand-built operand's
// list can end exactly at its last byte. A word kernel therefore reads whole words
// from memory only below the operand's last word, and takes the last word from a frame
// slot that `emit_word_view` assembles byte by byte. No read goes past the operand's
// significant bytes, so no read can leave its block.
//
// A result is made by `emit_alloc_magnitude`, whose region is a whole number of zeroed
// words, so the kernels store whole words into it.
// ---------------------------------------------------------------------------

/// A loaded operand seen as little-endian 64-bit words.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WordSlots {
    /// Address of word 0 (the operand's data slot).
    pub(crate) data: usize,
    /// `ceil(count / 8)`: how many words the significant bytes span.
    pub(crate) words: usize,
    /// Word `words - 1`, assembled from its significant bytes (`0` for a zero operand).
    pub(crate) top: usize,
}

/// The word view of a loaded operand: its word count and its assembled top word, each
/// in a fresh frame slot. Reads only the operand's significant bytes.
pub(crate) fn emit_word_view(
    builder: &mut CodeBuilder,
    vregs: &mut Vregs,
    operand: &IntSlots,
    tag: &str,
) -> WordSlots {
    let symbol = builder.current_symbol.clone();
    let view = WordSlots {
        data: operand.data,
        words: builder.allocate_stack_object(&format!("big_{tag}_words"), 8),
        top: builder.allocate_stack_object(&format!("big_{tag}_top"), 8),
    };
    let assemble = format!("{symbol}_{tag}_top_word");
    let assembled = format!("{symbol}_{tag}_top_word_done");
    let (count, words, start, top, cursor, byte) = (
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
    );
    builder.instructions.extend([
        abi::load_u64(&count, abi::stack_pointer(), operand.count),
        abi::add_immediate(&words, &count, 7),
        abi::shift_right_immediate(&words, &words, 3),
        abi::store_u64(&words, abi::stack_pointer(), view.words),
        abi::move_immediate(&top, "Integer", "0"),
        abi::compare_immediate(&count, "0"),
        abi::branch_eq(&assembled),
        // start = &data[8 * (words - 1)], cursor = &data[count]; gather the bytes
        // between them, most significant first.
        abi::subtract_immediate(&start, &words, 1),
        abi::shift_left_immediate(&start, &start, 3),
        abi::load_u64(&cursor, abi::stack_pointer(), operand.data),
        abi::add_registers(&start, &cursor, &start),
        abi::add_registers(&cursor, &cursor, &count),
        abi::label(&assemble),
        abi::compare_registers(&cursor, &start),
        abi::branch_eq(&assembled),
        abi::subtract_immediate(&cursor, &cursor, 1),
        abi::load_u8(&byte, &cursor, 0),
        abi::shift_left_immediate(&top, &top, 8),
        abi::or_registers(&top, &top, &byte),
        abi::branch(&assemble),
        abi::label(&assembled),
        abi::store_u64(&top, abi::stack_pointer(), view.top),
    ]);
    view
}

/// `dst = word index` of `view`, for `index < words`: memory below the last word, the
/// assembled top word at it. `data` holds the word-0 address and `last` holds
/// `words - 1`. `label_base` must be unique to the call site.
#[allow(clippy::too_many_arguments)]
fn emit_word_at(
    builder: &mut CodeBuilder,
    view: &WordSlots,
    data: impl Into<Reg>,
    last: impl Into<Reg>,
    index: impl Into<Reg>,
    dst: impl Into<Reg>,
    address: impl Into<Reg>,
    label_base: &str,
) {
    let (data, last, index, dst, address): (Reg, Reg, Reg, Reg, Reg) = (
        data.into(),
        last.into(),
        index.into(),
        dst.into(),
        address.into(),
    );
    let use_top = format!("{label_base}_top");
    let done = format!("{label_base}_done");
    builder.instructions.extend([
        abi::compare_registers(index, last),
        abi::branch_eq(&use_top),
        abi::shift_left_immediate(address, index, 3),
        abi::add_registers(address, data, address),
        abi::load_u64(dst, address, 0),
        abi::branch(&done),
        abi::label(&use_top),
        abi::load_u64(dst, abi::stack_pointer(), view.top),
        abi::label(&done),
    ]);
}

/// `dst = word index` of `view`, or zero at and past `words` (held in `words`).
#[allow(clippy::too_many_arguments)]
fn emit_word_or_zero(
    builder: &mut CodeBuilder,
    view: &WordSlots,
    data: impl Into<Reg>,
    words: impl Into<Reg>,
    last: impl Into<Reg>,
    index: impl Into<Reg>,
    dst: impl Into<Reg>,
    address: impl Into<Reg>,
    label_base: &str,
) {
    let (index, dst): (Reg, Reg) = (index.into(), dst.into());
    let present = format!("{label_base}_present");
    let done = format!("{label_base}_absent_done");
    builder.instructions.extend([
        abi::compare_registers(index, words),
        abi::branch_lo(&present),
        abi::move_immediate(dst, "Integer", "0"),
        abi::branch(&done),
        abi::label(&present),
    ]);
    emit_word_at(builder, view, data, last, index, dst, address, label_base);
    builder.instructions.push(abi::label(&done));
}

/// Copy `view`'s words into the buffer whose address is in `dst_slot`: `words - 1`
/// whole words, then the assembled top word. The buffer must hold `8 * words` bytes.
pub(crate) fn emit_copy_words(
    builder: &mut CodeBuilder,
    vregs: &mut Vregs,
    view: &WordSlots,
    dst_slot: usize,
    tag: &str,
) {
    let symbol = builder.current_symbol.clone();
    let copy = format!("{symbol}_{tag}_wcopy");
    let copied = format!("{symbol}_{tag}_wcopy_done");
    let empty = format!("{symbol}_{tag}_wcopy_empty");
    let (src, dst, end, word) = (vregs.next(), vregs.next(), vregs.next(), vregs.next());
    builder.instructions.extend([
        abi::load_u64(&end, abi::stack_pointer(), view.words),
        abi::compare_immediate(&end, "0"),
        abi::branch_eq(&empty),
        abi::load_u64(&src, abi::stack_pointer(), view.data),
        abi::load_u64(&dst, abi::stack_pointer(), dst_slot),
        // end = &src[words - 1]: the top word comes from its slot.
        abi::subtract_immediate(&end, &end, 1),
        abi::shift_left_immediate(&end, &end, 3),
        abi::add_registers(&end, &src, &end),
        abi::label(&copy),
        abi::compare_registers(&src, &end),
        abi::branch_eq(&copied),
        abi::load_u64(&word, &src, 0),
        abi::store_u64(&word, &dst, 0),
        abi::add_immediate(&src, &src, 8),
        abi::add_immediate(&dst, &dst, 8),
        abi::branch(&copy),
        abi::label(&copied),
        abi::load_u64(&word, abi::stack_pointer(), view.top),
        abi::store_u64(&word, &dst, 0),
        abi::label(&empty),
    ]);
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
    emit_trim(
        builder,
        vregs,
        &data,
        &count,
        &negative,
        &format!("{symbol}_{tag}_build"),
    );
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
    out: impl Into<Reg>,
    tag: &str,
) {
    let out: Reg = out.into();
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
    out: impl Into<Reg>,
    tag: &str,
) {
    let out: Reg = out.into();
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
    let view = emit_word_view(builder, vregs, operand, &format!("{tag}_src"));
    let result = emit_alloc_magnitude(builder, vregs, operand.count, tag, alloc_fail);
    emit_copy_words(builder, vregs, &view, result.data, tag);
    emit_build_int(builder, vregs, &result, operand.count, negative_slot, tag);
}

/// The canonical-form rule, emitted once for both directions: walk `count` down past
/// zero high bytes, then clear `negative` when nothing is left. `data` is preserved;
/// `count` and `negative` are updated in place.
fn emit_trim(
    builder: &mut CodeBuilder,
    vregs: &mut Vregs,
    data: impl Into<Reg>,
    count: impl Into<Reg>,
    negative: impl Into<Reg>,
    label_base: &str,
) {
    let data: Reg = data.into();
    let (count, negative): (Reg, Reg) = (count.into(), negative.into());
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
/// Chunked short division (bug-775): a working copy of the magnitude, read as 32-bit
/// limbs, is divided by `chunk = radix^k` — the largest power of the radix below
/// `2^32` (`10^9`, `2^31`, `16^7`, `36^6`) — from the high limb down. Every step's
/// dividend is `rem * 2^32 + limb < chunk * 2^32 <= 2^64`, one 64-bit `udiv`. Each pass
/// therefore yields `k` digits, least significant first, from its remainder: exactly `k`
/// (zero-padded) while the quotient is non-zero, and only the significant ones from the
/// last, top chunk, which is non-zero because the quotient before it was. The copy's
/// zero high limbs are dropped after each pass. The working copy and the digit buffer
/// are this helper's own and are released at the sizes they were made with.
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
    let written_slot = builder.allocate_stack_object(&format!("big_{tag}_written"), 8);
    let string_slot = builder.allocate_stack_object(&format!("big_{tag}_string"), 8);
    let view = emit_word_view(builder, vregs, operand, &format!("{tag}_text"));

    // Sizes: the working copy holds the magnitude's words (plus one, so it is never
    // empty); radix 2 writes at most 8 digits a byte, plus the sign.
    let size = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&size, abi::stack_pointer(), view.words),
        abi::add_immediate(&size, &size, 1),
        abi::shift_left_immediate(&size, &size, 3),
        abi::store_u64(&size, abi::stack_pointer(), work_size),
        abi::load_u64(&size, abi::stack_pointer(), operand.count),
        abi::shift_left_immediate(&size, &size, 3),
        abi::add_immediate(&size, &size, 2),
        abi::store_u64(&size, abi::stack_pointer(), digits_size),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), work_size),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(
        &symbol,
        &mut builder.instructions,
        &mut builder.relocations,
        alloc_fail,
    );
    builder.instructions.extend([
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), work_slot),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), digits_size),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(
        &symbol,
        &mut builder.instructions,
        &mut builder.relocations,
        alloc_fail,
    );
    builder.instructions.extend([
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), digits_slot),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), written_slot),
    ]);

    // Copy the magnitude into the working buffer.
    emit_copy_words(builder, vregs, &view, work_slot, &format!("{tag}_text"));

    // Zero writes a single '0'.
    let digits_done = format!("{symbol}_{tag}_digits_done");
    let nonzero = format!("{symbol}_{tag}_nonzero");
    let (count, zero_buffer, zero_value) = (vregs.next(), vregs.next(), vregs.next());
    builder.instructions.extend([
        abi::load_u64(&count, abi::stack_pointer(), operand.count),
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

    // chunk = radix^k, the largest power below 2^32 (chunk * radix cannot overflow:
    // chunk < 2^32 and radix <= 36).
    let grow = format!("{symbol}_{tag}_chunk_grow");
    let grown = format!("{symbol}_{tag}_chunk_grown");
    let chunked = format!("{symbol}_{tag}_chunked");
    let (radix, chunk, k, next, limit) = (
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
    );
    builder.instructions.extend([
        abi::load_u64(&radix, abi::stack_pointer(), radix_slot),
        abi::move_register(&chunk, &radix),
        abi::move_immediate(&k, "Integer", "1"),
        abi::move_immediate(&limit, "Integer", "4294967296"),
        abi::label(&grow),
        abi::multiply_registers(&next, &chunk, &radix),
        abi::compare_registers(&next, &limit),
        abi::branch_lo(&grown),
        abi::branch(&chunked),
        abi::label(&grown),
        abi::move_register(&chunk, &next),
        abi::add_immediate(&k, &k, 1),
        abi::branch(&grow),
        abi::label(&chunked),
    ]);

    // One pass per chunk: divide the working copy by `chunk`, high limb down. The
    // whole digit loop is call-free, so its state stays in registers.
    let digit_loop = format!("{symbol}_{tag}_digit");
    let divide_loop = format!("{symbol}_{tag}_divide");
    let divided = format!("{symbol}_{tag}_divided");
    let trim_loop = format!("{symbol}_{tag}_trim");
    let trimmed = format!("{symbol}_{tag}_trimmed");
    let emit_digit = format!("{symbol}_{tag}_emit");
    let letter = format!("{symbol}_{tag}_letter");
    let placed = format!("{symbol}_{tag}_placed");
    let top_chunk = format!("{symbol}_{tag}_top_chunk");
    let (work, remaining, cursor, remainder, dividend, quotient, left, out, digit) = (
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
        abi::load_u64(&work, abi::stack_pointer(), work_slot),
        abi::load_u64(&out, abi::stack_pointer(), digits_slot),
        // remaining = ceil(count / 4) limbs.
        abi::load_u64(&remaining, abi::stack_pointer(), operand.count),
        abi::add_immediate(&remaining, &remaining, 3),
        abi::shift_right_immediate(&remaining, &remaining, 2),
        abi::label(&digit_loop),
        abi::shift_left_immediate(&cursor, &remaining, 2),
        abi::add_registers(&cursor, &work, &cursor),
        abi::move_immediate(&remainder, "Integer", "0"),
        abi::label(&divide_loop),
        abi::compare_registers(&cursor, &work),
        abi::branch_eq(&divided),
        abi::subtract_immediate(&cursor, &cursor, 4),
        abi::load_u32(&dividend, &cursor, 0),
        abi::shift_left_immediate(&remainder, &remainder, 32),
        abi::or_registers(&dividend, &dividend, &remainder),
        abi::unsigned_divide_registers(&quotient, &dividend, &chunk),
        abi::store_u32(&quotient, &cursor, 0),
        // remainder = dividend - quotient * chunk
        abi::multiply_subtract_registers(&remainder, &quotient, &chunk, &dividend),
        abi::branch(&divide_loop),
        abi::label(&divided),
        // Drop the working copy's zero high limbs: while work[remaining - 1] == 0.
        abi::label(&trim_loop),
        abi::compare_immediate(&remaining, "0"),
        abi::branch_eq(&trimmed),
        abi::shift_left_immediate(&cursor, &remaining, 2),
        abi::add_registers(&cursor, &work, &cursor),
        abi::subtract_immediate(&cursor, &cursor, 4),
        abi::load_u32(&dividend, &cursor, 0),
        abi::compare_immediate(&dividend, "0"),
        abi::branch_ne(&trimmed),
        abi::subtract_immediate(&remaining, &remaining, 1),
        abi::branch(&trim_loop),
        abi::label(&trimmed),
        // The chunk's digits, least significant first: k of them while a quotient is
        // left, only the significant ones for the top chunk.
        abi::move_register(&left, &k),
        abi::label(&emit_digit),
        abi::unsigned_divide_registers(&quotient, &remainder, &radix),
        abi::multiply_subtract_registers(&digit, &quotient, &radix, &remainder),
        abi::move_register(&remainder, &quotient),
        abi::compare_immediate(&digit, "10"),
        abi::branch_ge(&letter),
        abi::add_immediate(&digit, &digit, 48),
        abi::branch(&placed),
        abi::label(&letter),
        abi::add_immediate(&digit, &digit, 87),
        abi::label(&placed),
        abi::store_u8(&digit, &out, 0),
        abi::add_immediate(&out, &out, 1),
        abi::subtract_immediate(&left, &left, 1),
        abi::compare_immediate(&remaining, "0"),
        abi::branch_eq(&top_chunk),
        abi::compare_immediate(&left, "0"),
        abi::branch_ne(&emit_digit),
        abi::branch(&digit_loop),
        abi::label(&top_chunk),
        abi::compare_immediate(&remainder, "0"),
        abi::branch_ne(&emit_digit),
        // written = out - digits
        abi::load_u64(&cursor, abi::stack_pointer(), digits_slot),
        abi::subtract_registers(&out, &out, &cursor),
        abi::store_u64(&out, abi::stack_pointer(), written_slot),
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
    emit_alloc(
        &symbol,
        &mut builder.instructions,
        &mut builder.relocations,
        alloc_fail,
    );
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
// Division (plan-127-C Phase 1; base 2^32 since bug-775): Knuth, TAOCP Vol. 2 §4.3.1,
// Algorithm D, over 32-bit limbs.
//
// The magnitude's bytes are read as little-endian 32-bit limbs. Base 2^32 keeps every
// intermediate inside 64 bits on every target: D3's estimate divides a two-limb number
// by one limb with a single 64-bit `udiv`, and D4's `qhat * v[i]` is below 2^64. (Base
// 2^64 would need a 128-by-64 division, which AArch64 and riscv64 do not have.) The
// shape follows Hacker's Delight `divmnu64`: a signed 64-bit `t` carries the borrow.
//
// Two different "normalize"s meet in this section and must not be confused:
//
// - Algorithm D's **operand normalization** (step D1) left-shifts the divisor and the
//   dividend by the same `s` bits so the divisor's top limb has its high bit set. That
//   is what bounds each quotient-digit estimate to at most two too large; step D8 shifts
//   the remainder back by `s`. It never touches a record.
// - `emit_build_int`'s **record normalization** is canonical form — trailing zero bytes
//   dropped, zero never negative. The caller applies it to the quotient and the
//   remainder afterwards, exactly as for every other `big` result.
//
// Every value a later step needs lives in a frame slot; the only calls are the block
// allocations and releases, and each loop body is call-free.
// ---------------------------------------------------------------------------

/// Where [`emit_div_mod_magnitude`] leaves its results: each a result record plus the
/// slot holding its written byte count, ready for `emit_build_int`. The quotient slots
/// are written only when the quotient was asked for.
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

/// Store the constant `value` into frame slot `slot`.
fn emit_store_constant(builder: &mut CodeBuilder, vregs: &mut Vregs, value: u64, slot: usize) {
    let register = vregs.next();
    builder.instructions.extend([
        abi::move_immediate(&register, "Integer", &value.to_string()),
        abi::store_u64(&register, abi::stack_pointer(), slot),
    ]);
}

/// `|u| / |v|` and `|u| mod |v|` over two loaded operands — or only the remainder when
/// `want_quotient` is false, which makes no quotient block at all (`modPow`'s and
/// `gcd`'s reductions). **The caller guarantees `v` is not zero.** Three paths, each
/// leaving its results in the same [`DivModSlots`]:
///
/// - `|u|` has fewer bytes than `|v|`: quotient zero, remainder `|u|`;
/// - `|v|` fits one 32-bit limb: short division, two limbs per word of `u`, high word
///   down (no normalization needed);
/// - otherwise Algorithm D, steps D1–D8, with both the estimate correction (D3) and the
///   add-back (D6) — the two places implementations go wrong.
///
/// Branches to `alloc_fail` when a block cannot be made.
pub(crate) fn emit_div_mod_magnitude(
    builder: &mut CodeBuilder,
    vregs: &mut Vregs,
    u: &IntSlots,
    v: &IntSlots,
    want_quotient: bool,
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
    let word_slot = slot(builder, "word");
    let rem_slot = slot(builder, "short_rem");
    let shift_slot = slot(builder, "shift");
    let nv_slot = slot(builder, "nv");
    let nu_slot = slot(builder, "nu");
    let q_size_slot = slot(builder, "q_size");
    let r_size_slot = slot(builder, "r_size");
    let vn_slot = slot(builder, "vn");
    let vn_size_slot = slot(builder, "vn_size");
    let un_slot = slot(builder, "un");
    let un_size_slot = slot(builder, "un_size");
    let view_u = emit_word_view(builder, vregs, u, &format!("{tag}_u"));
    let view_v = emit_word_view(builder, vregs, v, &format!("{tag}_v"));

    let smaller = format!("{symbol}_{tag}_smaller");
    let short = format!("{symbol}_{tag}_short");
    let general = format!("{symbol}_{tag}_general");
    let joined = format!("{symbol}_{tag}_joined");

    // --- Dispatch --------------------------------------------------------------------
    {
        let (count_u, count_v) = (vregs.next(), vregs.next());
        builder
            .instructions
            .push(abi::store_u64(abi::ZERO, abi::stack_pointer(), zero_slot));
        emit_store_constant(builder, vregs, 8, word_slot);
        builder.instructions.extend([
            abi::load_u64(&count_u, abi::stack_pointer(), u.count),
            abi::load_u64(&count_v, abi::stack_pointer(), v.count),
            abi::compare_registers(&count_u, &count_v),
            abi::branch_lo(&smaller),
            abi::compare_immediate(&count_v, "4"),
            abi::branch_ls(&short),
            abi::branch(&general),
        ]);
    }

    // --- |u| shorter than |v|: quotient 0, remainder |u| ------------------------------
    builder.instructions.push(abi::label(&smaller));
    if want_quotient {
        let made =
            emit_alloc_magnitude(builder, vregs, zero_slot, &format!("{tag}_aq"), alloc_fail);
        emit_adopt_result(builder, vregs, &made, &out.quotient);
        emit_move_slot(builder, vregs, zero_slot, out.quotient_count);
    }
    let made = emit_alloc_magnitude(builder, vregs, u.count, &format!("{tag}_ar"), alloc_fail);
    emit_adopt_result(builder, vregs, &made, &out.remainder);
    emit_move_slot(builder, vregs, u.count, out.remainder_count);
    emit_copy_words(
        builder,
        vregs,
        &view_u,
        out.remainder.data,
        &format!("{tag}_a"),
    );
    builder.instructions.push(abi::branch(&joined));

    // --- One-limb divisor: short division ----------------------------------------------
    // Each word of u is two limbs: rem * 2^32 + limb < d * 2^32 <= 2^64, so one 64-bit
    // `udiv` per limb.
    builder.instructions.push(abi::label(&short));
    if want_quotient {
        let made = emit_alloc_magnitude(builder, vregs, u.count, &format!("{tag}_bq"), alloc_fail);
        emit_adopt_result(builder, vregs, &made, &out.quotient);
        emit_move_slot(builder, vregs, u.count, out.quotient_count);
    }
    {
        let divide = format!("{symbol}_{tag}_b_divide");
        let divided = format!("{symbol}_{tag}_b_divided");
        let (data, last, index, divisor, remainder, word, current, half, digit, high, address) = (
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
            abi::load_u64(&data, abi::stack_pointer(), u.data),
            abi::load_u64(&index, abi::stack_pointer(), view_u.words),
            abi::subtract_immediate(&last, &index, 1),
            abi::load_u64(&divisor, abi::stack_pointer(), view_v.top),
            abi::move_immediate(&remainder, "Integer", "0"),
            abi::label(&divide),
            abi::compare_immediate(&index, "0"),
            abi::branch_eq(&divided),
            abi::subtract_immediate(&index, &index, 1),
        ]);
        emit_word_at(
            builder,
            &view_u,
            data,
            last,
            index,
            word,
            address,
            &format!("{symbol}_{tag}_b_word"),
        );
        builder.instructions.extend([
            // High limb: current = rem << 32 | word >> 32.
            abi::shift_left_immediate(&current, &remainder, 32),
            abi::shift_right_immediate(&half, &word, 32),
            abi::or_registers(&current, &current, &half),
            abi::unsigned_divide_registers(&high, &current, &divisor),
            abi::multiply_subtract_registers(&remainder, &high, &divisor, &current),
            // Low limb: current = rem << 32 | word & 0xffffffff.
            abi::shift_left_immediate(&current, &remainder, 32),
            abi::shift_left_immediate(&half, &word, 32),
            abi::shift_right_immediate(&half, &half, 32),
            abi::or_registers(&current, &current, &half),
            abi::unsigned_divide_registers(&digit, &current, &divisor),
            abi::multiply_subtract_registers(&remainder, &digit, &divisor, &current),
        ]);
        if want_quotient {
            builder.instructions.extend([
                abi::shift_left_immediate(&high, &high, 32),
                abi::or_registers(&digit, &digit, &high),
                abi::load_u64(&address, abi::stack_pointer(), out.quotient.data),
                abi::shift_left_immediate(&half, &index, 3),
                abi::add_registers(&address, &address, &half),
                abi::store_u64(&digit, &address, 0),
            ]);
        }
        builder.instructions.extend([
            abi::branch(&divide),
            abi::label(&divided),
            abi::store_u64(&remainder, abi::stack_pointer(), rem_slot),
        ]);
    }
    let made = emit_alloc_magnitude(builder, vregs, word_slot, &format!("{tag}_br"), alloc_fail);
    emit_adopt_result(builder, vregs, &made, &out.remainder);
    emit_move_slot(builder, vregs, word_slot, out.remainder_count);
    {
        let (dst, remainder) = (vregs.next(), vregs.next());
        builder.instructions.extend([
            abi::load_u64(&dst, abi::stack_pointer(), out.remainder.data),
            abi::load_u64(&remainder, abi::stack_pointer(), rem_slot),
            abi::store_u64(&remainder, &dst, 0),
            abi::branch(&joined),
        ]);
    }

    // --- Algorithm D: n = |v| limbs >= 2, m = |u| limbs - n >= 0 ----------------------
    builder.instructions.push(abi::label(&general));
    {
        // nu, nv in limbs; s = clz32(v's top limb). The top limb is the top word's upper
        // half when nv is even, its lower half when nv is odd.
        let (count, limbs, top, zeros, size) = (
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
        );
        let odd = format!("{symbol}_{tag}_d1_odd");
        builder.instructions.extend([
            abi::load_u64(&count, abi::stack_pointer(), u.count),
            abi::add_immediate(&limbs, &count, 3),
            abi::shift_right_immediate(&limbs, &limbs, 2),
            abi::store_u64(&limbs, abi::stack_pointer(), nu_slot),
            abi::load_u64(&count, abi::stack_pointer(), v.count),
            abi::add_immediate(&limbs, &count, 3),
            abi::shift_right_immediate(&limbs, &limbs, 2),
            abi::store_u64(&limbs, abi::stack_pointer(), nv_slot),
            // The remainder spans nv limbs; the quotient m + 1 = nu - nv + 1 limbs.
            abi::shift_left_immediate(&size, &limbs, 2),
            abi::store_u64(&size, abi::stack_pointer(), r_size_slot),
            abi::load_u64(&size, abi::stack_pointer(), nu_slot),
            abi::subtract_registers(&size, &size, &limbs),
            abi::add_immediate(&size, &size, 1),
            abi::shift_left_immediate(&size, &size, 2),
            abi::store_u64(&size, abi::stack_pointer(), q_size_slot),
            abi::load_u64(&top, abi::stack_pointer(), view_v.top),
            abi::move_immediate(&zeros, "Integer", "1"),
            abi::and_registers(&zeros, &limbs, &zeros),
            abi::compare_immediate(&zeros, "0"),
            abi::branch_ne(&odd),
            abi::shift_right_immediate(&top, &top, 32),
            abi::label(&odd),
            // A trimmed top limb is non-zero: its 64-bit leading-zero count is 32..63.
            abi::count_leading_zeros(&zeros, &top),
            abi::subtract_immediate(&zeros, &zeros, 32),
            abi::store_u64(&zeros, abi::stack_pointer(), shift_slot),
            // vn holds v's words; un holds u's words plus one zero word.
            abi::load_u64(&size, abi::stack_pointer(), view_v.words),
            abi::shift_left_immediate(&size, &size, 3),
            abi::store_u64(&size, abi::stack_pointer(), vn_size_slot),
            abi::load_u64(&size, abi::stack_pointer(), view_u.words),
            abi::shift_left_immediate(&size, &size, 3),
            abi::add_immediate(&size, &size, 8),
            abi::store_u64(&size, abi::stack_pointer(), un_size_slot),
            abi::load_u64(abi::return_register(), abi::stack_pointer(), vn_size_slot),
            abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        ]);
    }
    emit_alloc(
        &symbol,
        &mut builder.instructions,
        &mut builder.relocations,
        alloc_fail,
    );
    builder.instructions.extend([
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), vn_slot),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), un_size_slot),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(
        &symbol,
        &mut builder.instructions,
        &mut builder.relocations,
        alloc_fail,
    );
    builder.instructions.push(abi::store_u64(
        abi::mfb_return(1),
        abi::stack_pointer(),
        un_slot,
    ));
    if want_quotient {
        let made = emit_alloc_magnitude(
            builder,
            vregs,
            q_size_slot,
            &format!("{tag}_dq"),
            alloc_fail,
        );
        emit_adopt_result(builder, vregs, &made, &out.quotient);
        emit_move_slot(builder, vregs, q_size_slot, out.quotient_count);
    }
    let made = emit_alloc_magnitude(
        builder,
        vregs,
        r_size_slot,
        &format!("{tag}_dr"),
        alloc_fail,
    );
    emit_adopt_result(builder, vregs, &made, &out.remainder);
    emit_move_slot(builder, vregs, r_size_slot, out.remainder_count);

    // D1: copy both operands into their buffers (un's extra word zero), then shift each
    // left by s in place, top limb down: limb[i] = limb[i] << s | limb[i - 1] >> (32 - s).
    // A 64-bit shift by 32 gives zero, so s = 0 needs no special case, and the 32-bit
    // store keeps only the low limb of `limb[i] << s`.
    emit_copy_words(builder, vregs, &view_v, vn_slot, &format!("{tag}_d1_v"));
    emit_copy_words(builder, vregs, &view_u, un_slot, &format!("{tag}_d1_u"));
    {
        let (un, size) = (vregs.next(), vregs.next());
        builder.instructions.extend([
            abi::load_u64(&un, abi::stack_pointer(), un_slot),
            abi::load_u64(&size, abi::stack_pointer(), view_u.words),
            abi::shift_left_immediate(&size, &size, 3),
            abi::add_registers(&un, &un, &size),
            abi::store_u64(abi::ZERO, &un, 0),
        ]);
    }
    // vn spans nv limbs (no carry out: its top bit lands exactly on bit 31); un spans
    // nu + 1 limbs, the top one receiving u's carry out.
    for (buffer, limbs_slot, extra, name) in [
        (vn_slot, nv_slot, 0usize, "v"),
        (un_slot, nu_slot, 1usize, "u"),
    ] {
        let shift = format!("{symbol}_{tag}_d1_{name}_shift");
        let shifted = format!("{symbol}_{tag}_d1_{name}_shifted");
        let (base, cursor, bits, back, limb, lower) = (
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
            vregs.next(),
        );
        builder.instructions.extend([
            abi::load_u64(&base, abi::stack_pointer(), buffer),
            abi::load_u64(&cursor, abi::stack_pointer(), limbs_slot),
            abi::add_immediate(&cursor, &cursor, extra),
            abi::subtract_immediate(&cursor, &cursor, 1),
            abi::shift_left_immediate(&cursor, &cursor, 2),
            abi::add_registers(&cursor, &base, &cursor),
            abi::load_u64(&bits, abi::stack_pointer(), shift_slot),
            abi::move_immediate(&back, "Integer", "32"),
            abi::subtract_registers(&back, &back, &bits),
            abi::label(&shift),
            abi::compare_registers(&cursor, &base),
            abi::branch_eq(&shifted),
            abi::load_u32(&limb, &cursor, 0),
            abi::shift_left_variable(&limb, &limb, &bits),
            abi::subtract_immediate(&cursor, &cursor, 4),
            abi::load_u32(&lower, &cursor, 0),
            abi::shift_right_variable(&lower, &lower, &back),
            abi::or_registers(&limb, &limb, &lower),
            abi::store_u32(&limb, &cursor, 4),
            abi::branch(&shift),
            abi::label(&shifted),
            abi::load_u32(&limb, &base, 0),
            abi::shift_left_variable(&limb, &limb, &bits),
            abi::store_u32(&limb, &base, 0),
        ]);
    }

    // D2..D7: j runs from m down to 0.
    let digit_loop = format!("{symbol}_{tag}_d2");
    let digit_next = format!("{symbol}_{tag}_d2_next");
    let digits_done = format!("{symbol}_{tag}_d2_done");
    let test = format!("{symbol}_{tag}_d3_test");
    let second = format!("{symbol}_{tag}_d3_second");
    let correct = format!("{symbol}_{tag}_d3_correct");
    let estimated = format!("{symbol}_{tag}_d3_estimated");
    let multiply = format!("{symbol}_{tag}_d4");
    let multiplied = format!("{symbol}_{tag}_d4_done");
    let add_back = format!("{symbol}_{tag}_d6");
    let added = format!("{symbol}_{tag}_d6_done");
    let (vn, nv, vtop, vnext, radix, mask, j1, base, at) = (
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
    let (number, low, qhat, rhat, lhs, rhs) = (
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
    );
    let (pv, pu, end, product, t, k, scratch) = (
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
    );
    builder.instructions.extend([
        abi::load_u64(&vn, abi::stack_pointer(), vn_slot),
        abi::load_u64(&nv, abi::stack_pointer(), nv_slot),
        // vtop = vn[nv - 1], vnext = vn[nv - 2]
        abi::shift_left_immediate(&at, &nv, 2),
        abi::add_registers(&at, &vn, &at),
        abi::subtract_immediate(&at, &at, 4),
        abi::load_u32(&vtop, &at, 0),
        abi::subtract_immediate(&at, &at, 4),
        abi::load_u32(&vnext, &at, 0),
        abi::move_immediate(&radix, "Integer", "4294967296"),
        abi::move_immediate(&mask, "Integer", "4294967295"),
        // j1 = m + 1 = nu - nv + 1
        abi::load_u64(&j1, abi::stack_pointer(), nu_slot),
        abi::subtract_registers(&j1, &j1, &nv),
        abi::add_immediate(&j1, &j1, 1),
        abi::label(&digit_loop),
        abi::compare_immediate(&j1, "0"),
        abi::branch_eq(&digits_done),
        // base = &un[j]; at = &un[j + nv - 2]
        abi::load_u64(&base, abi::stack_pointer(), un_slot),
        abi::subtract_immediate(&scratch, &j1, 1),
        abi::shift_left_immediate(&scratch, &scratch, 2),
        abi::add_registers(&base, &base, &scratch),
        abi::shift_left_immediate(&at, &nv, 2),
        abi::add_registers(&at, &base, &at),
        abi::subtract_immediate(&at, &at, 8),
        // D3: number = un[j + nv] * 2^32 + un[j + nv - 1]
        abi::load_u32(&number, &at, 8),
        abi::shift_left_immediate(&number, &number, 32),
        abi::load_u32(&low, &at, 4),
        abi::or_registers(&number, &number, &low),
        abi::unsigned_divide_registers(&qhat, &number, &vtop),
        abi::multiply_subtract_registers(&rhat, &qhat, &vtop, &number),
        abi::load_u32(&low, &at, 0),
        abi::label(&test),
        // qhat >= 2^32 is too large.
        abi::compare_registers(&qhat, &radix),
        abi::branch_lo(&second),
        abi::branch(&correct),
        abi::label(&second),
        // qhat * vn[nv-2] > rhat * 2^32 + un[j + nv - 2] is too large.
        abi::multiply_registers(&lhs, &qhat, &vnext),
        abi::shift_left_immediate(&rhs, &rhat, 32),
        abi::or_registers(&rhs, &rhs, &low),
        abi::compare_registers(&lhs, &rhs),
        abi::branch_hi(&correct),
        abi::branch(&estimated),
        abi::label(&correct),
        abi::subtract_immediate(&qhat, &qhat, 1),
        abi::add_registers(&rhat, &rhat, &vtop),
        // Once rhat reaches 2^32 the second test can no longer fire.
        abi::compare_registers(&rhat, &radix),
        abi::branch_lo(&test),
        abi::label(&estimated),
        // D4: un[j .. j + nv] -= qhat * vn, with t signed:
        //   p = qhat * vn[i]; t = un[i + j] - k - (p & mask); un[i + j] = t;
        //   k = (p >> 32) - (t >> 32 arithmetic).
        abi::move_register(&pv, &vn),
        abi::move_register(&pu, &base),
        abi::shift_left_immediate(&end, &nv, 2),
        abi::add_registers(&end, &vn, &end),
        abi::move_immediate(&k, "Integer", "0"),
        abi::label(&multiply),
        abi::compare_registers(&pv, &end),
        abi::branch_eq(&multiplied),
        abi::load_u32(&product, &pv, 0),
        abi::multiply_registers(&product, &product, &qhat),
        abi::load_u32(&t, &pu, 0),
        abi::subtract_registers(&t, &t, &k),
        abi::and_registers(&scratch, &product, &mask),
        abi::subtract_registers(&t, &t, &scratch),
        abi::store_u32(&t, &pu, 0),
        abi::shift_right_immediate(&k, &product, 32),
        abi::arithmetic_shift_right_immediate(&scratch, &t, 32),
        abi::subtract_registers(&k, &k, &scratch),
        abi::add_immediate(&pv, &pv, 4),
        abi::add_immediate(&pu, &pu, 4),
        abi::branch(&multiply),
        abi::label(&multiplied),
        // The top position: t = un[j + nv] - k.
        abi::load_u32(&t, &pu, 0),
        abi::subtract_registers(&t, &t, &k),
        abi::store_u32(&t, &pu, 0),
    ]);
    // D5 (below, after D6): q[j] = qhat, corrected when D4 went negative.
    builder.instructions.extend([
        abi::compare_immediate(&t, "0"),
        abi::branch_ge(&digit_next),
        // D6: the subtraction went negative, so qhat was one too large: take one off and
        // add the divisor back into un[j .. j + nv]; the carry out cancels the borrow
        // the subtraction left in un[j + nv].
        abi::subtract_immediate(&qhat, &qhat, 1),
        abi::move_register(&pv, &vn),
        abi::move_register(&pu, &base),
        abi::move_immediate(&k, "Integer", "0"),
        abi::label(&add_back),
        abi::compare_registers(&pv, &end),
        abi::branch_eq(&added),
        abi::load_u32(&t, &pu, 0),
        abi::load_u32(&scratch, &pv, 0),
        abi::add_registers(&t, &t, &scratch),
        abi::add_registers(&t, &t, &k),
        abi::store_u32(&t, &pu, 0),
        abi::shift_right_immediate(&k, &t, 32),
        abi::add_immediate(&pv, &pv, 4),
        abi::add_immediate(&pu, &pu, 4),
        abi::branch(&add_back),
        abi::label(&added),
        abi::load_u32(&t, &pu, 0),
        abi::add_registers(&t, &t, &k),
        abi::store_u32(&t, &pu, 0),
        abi::label(&digit_next),
    ]);
    if want_quotient {
        builder.instructions.extend([
            // q[j] = qhat, at q + 4 * (j1 - 1).
            abi::load_u64(&scratch, abi::stack_pointer(), out.quotient.data),
            abi::subtract_immediate(&at, &j1, 1),
            abi::shift_left_immediate(&at, &at, 2),
            abi::add_registers(&scratch, &scratch, &at),
            abi::store_u32(&qhat, &scratch, 0),
        ]);
    }
    builder.instructions.extend([
        abi::subtract_immediate(&j1, &j1, 1),
        abi::branch(&digit_loop),
        abi::label(&digits_done),
    ]);

    // D8: the remainder is un[0 .. nv] shifted back right by s:
    // r[i] = un[i] >> s | un[i + 1] << (32 - s); un[nv] exists (nu + 1 >= nv + 1 limbs).
    {
        let unshift = format!("{symbol}_{tag}_d8");
        let unshifted = format!("{symbol}_{tag}_d8_done");
        let (un, dst, end, bits, back, limb, upper) = (
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
            abi::load_u64(&end, abi::stack_pointer(), nv_slot),
            abi::shift_left_immediate(&end, &end, 2),
            abi::add_registers(&end, &un, &end),
            abi::load_u64(&bits, abi::stack_pointer(), shift_slot),
            abi::move_immediate(&back, "Integer", "32"),
            abi::subtract_registers(&back, &back, &bits),
            abi::label(&unshift),
            abi::compare_registers(&un, &end),
            abi::branch_eq(&unshifted),
            abi::load_u32(&limb, &un, 0),
            abi::shift_right_variable(&limb, &limb, &bits),
            abi::load_u32(&upper, &un, 4),
            abi::shift_left_variable(&upper, &upper, &back),
            abi::or_registers(&limb, &limb, &upper),
            abi::store_u32(&limb, &dst, 0),
            abi::add_immediate(&un, &un, 4),
            abi::add_immediate(&dst, &dst, 4),
            abi::branch(&unshift),
            abi::label(&unshifted),
        ]);
    }

    // Release the two scratch buffers at the sizes they were made with.
    for (pointer, size) in [(vn_slot, vn_size_slot), (un_slot, un_size_slot)] {
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
    let parts = emit_div_mod_magnitude(builder, vregs, a, b, true, tag, alloc_fail);
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

/// `a mod b` as a signed number (the sign of the dividend, as [`emit_div_mod_int`]'s
/// remainder), normalized, its record address left in `remainder_slot`. No quotient is
/// made. Branches to `zero_divisor` when `b` is zero and to `alloc_fail` when a block
/// cannot be made.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_rem_int(
    builder: &mut CodeBuilder,
    vregs: &mut Vregs,
    a: &IntSlots,
    b: &IntSlots,
    remainder_slot: usize,
    tag: &str,
    zero_divisor: &str,
    alloc_fail: &str,
) {
    let count = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&count, abi::stack_pointer(), b.count),
        abi::compare_immediate(&count, "0"),
        abi::branch_eq(zero_divisor),
    ]);
    let parts = emit_div_mod_magnitude(builder, vregs, a, b, false, tag, alloc_fail);
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
    for (record_slot, size_slot) in [
        (quotient_slot, quotient_size),
        (remainder_slot, remainder_size),
    ] {
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

// ---------------------------------------------------------------------------
// Values from `Integer`s (plan-127-C Phase 3).
// ---------------------------------------------------------------------------

/// A normalized `big::Int` equal to the `Integer` in `value_slot`, in
/// `RESULT_VALUE_REGISTER`: the absolute value as eight little-endian bytes (two's
/// complement negation read unsigned, exact even for `Integer`'s minimum), trimmed by
/// `emit_build_int`. Branches to `alloc_fail` when the block cannot be made. This is
/// `big::fromInteger`'s body, shared with the members that build intermediate values from
/// an `Integer` counter.
pub(crate) fn emit_int_from_integer(
    builder: &mut CodeBuilder,
    vregs: &mut Vregs,
    value_slot: usize,
    tag: &str,
    alloc_fail: &str,
) {
    let symbol = builder.current_symbol.clone();
    let capacity = builder.allocate_stack_object(&format!("big_{tag}_capacity"), 8);
    let negative = builder.allocate_stack_object(&format!("big_{tag}_negative"), 8);
    let positive = format!("{symbol}_{tag}_positive");
    let write_loop = format!("{symbol}_{tag}_write");
    let write_done = format!("{symbol}_{tag}_write_done");
    let eight = vregs.next();
    builder.instructions.extend([
        abi::move_immediate(&eight, "Integer", "8"),
        abi::store_u64(&eight, abi::stack_pointer(), capacity),
    ]);
    let result = emit_alloc_magnitude(builder, vregs, capacity, tag, alloc_fail);
    let (value, flag, zero, cursor, index) = (
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
    );
    builder.instructions.extend([
        abi::load_u64(&value, abi::stack_pointer(), value_slot),
        abi::move_immediate(&flag, "Integer", "0"),
        abi::compare_immediate(&value, "0"),
        abi::branch_ge(&positive),
        abi::move_immediate(&flag, "Integer", "1"),
        abi::move_immediate(&zero, "Integer", "0"),
        abi::subtract_registers(&value, &zero, &value),
        abi::label(&positive),
        abi::store_u64(&flag, abi::stack_pointer(), negative),
        abi::load_u64(&cursor, abi::stack_pointer(), result.data),
        abi::move_immediate(&index, "Integer", "0"),
        abi::label(&write_loop),
        abi::compare_immediate(&index, "8"),
        abi::branch_eq(&write_done),
        abi::store_u8(&value, &cursor, 0),
        abi::shift_right_immediate(&value, &value, 8),
        abi::add_immediate(&cursor, &cursor, 1),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&write_loop),
        abi::label(&write_done),
    ]);
    emit_build_int(builder, vregs, &result, capacity, negative, tag);
}
