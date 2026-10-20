//! plan-176-R: `big::Int` signed division and `DivResult`. The layout, slot views, build/compare/trim and release helpers
//! live in `big/gen_big/mod.rs`.

use super::*;

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
pub(super) fn emit_move_slot(builder: &mut CodeBuilder, vregs: &mut Vregs, from: usize, to: usize) {
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
