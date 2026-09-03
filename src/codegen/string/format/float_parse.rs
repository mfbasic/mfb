//! `_mfb_rt_string_to_float` — the correctly-rounded `toFloat(String)` parser
//! (plan-120-F), emitted as NIR.
//!
//! This is a transliteration of `float_parse_ref.rs`, which is the same
//! algorithm written in Rust and pinned against `str::parse::<f64>()`. Read that
//! module first: it carries the reasoning, and this one carries only the
//! register allocation. Where the two drift, the Rust side is the specification
//! and this side is the defect.
//!
//! It replaces `emit_parse_decimal_string_to_double`, which accumulated digits
//! in binary64 and applied the exponent by repeated multiply/divide by 10.0 —
//! the classic double-rounding construction, off by up to 1 ULP even for values
//! that are exactly representable.
//!
//! Six symbols, emitted together when anything relocates against the entry
//! point:
//!
//! | Symbol | Role |
//! |---|---|
//! | `_mfb_rt_string_to_float` | scan, fast path, Lemire, fallback, assembly |
//! | `_mfb_rt_f2s_lemire` | Eisel–Lemire proper, called up to twice |
//! | `_mfb_rt_f2s_cmp_scaled` | compare `D * 10^q` against `M * 2^e`, exactly |
//! | `_mfb_rt_f2s_mul_small` | `big = big * m + a`, fixed length |
//! | `_mfb_rt_f2s_shl` | `big <<= bits`, fixed length |
//! | `_mfb_rt_f2s_cmp` | compare two fixed-length bignums |
//!
//! **Every ABI argument is copied into a vreg at entry.** On x86-64 `mul` and
//! `umulh` expand to instructions that clobber the registers `c_arg`/`c_return`
//! map to (`rax`/`rdx` — and on Win64 `c_arg(1)` *is* `rdx`), so an argument
//! left in its incoming register is destroyed by the first 128-bit multiply. The
//! PCG64 helper hit exactly this and solves it the same way.

// --- codegen tier imports (migration) ---
use super::float_parse_table::{POWERS_OF_TEN_SYMBOL, Q_MAX, Q_MIN};
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::string::unicode_props::emit_load_data_address_free;
use crate::target::shared::abi;

/// `x0` = String pointer. Returns `x0` = 0 on success or 1 when the text is not
/// a number, and `x1` = the f64 bit pattern.
///
/// Overflow is deliberately NOT an error here: the helper saturates to
/// ±infinity and the caller's existing `emit_double_overflow_check` turns that
/// into `ErrOverflow`. That is what the old inline parser did, so `toFloat`'s
/// contract is unchanged.
pub(crate) const STRING_TO_FLOAT_SYMBOL: &str = "_mfb_rt_string_to_float";

const LEMIRE_SYMBOL: &str = "_mfb_rt_f2s_lemire";
const MUL_SMALL_SYMBOL: &str = "_mfb_rt_f2s_mul_small";
const SHL_SYMBOL: &str = "_mfb_rt_f2s_shl";
const CMP_SYMBOL: &str = "_mfb_rt_f2s_cmp";
const CMP_SCALED_SYMBOL: &str = "_mfb_rt_f2s_cmp_scaled";

/// Big naturals are a fixed number of 32-bit limbs, one per 8-byte slot,
/// least-significant first. One limb per slot wastes half the space and buys
/// carry handling that cannot overflow a 64-bit register: `limb * m + carry`
/// with `limb < 2^32` and `m <= 10^9` stays under 2^62. The formatter next door
/// makes the same trade for the same reason.
///
/// 176 limbs is 5632 bits against a worst case of 4689: the exact fallback's
/// largest operand is `D * 10^308 * 2^1075` with `D` capped at
/// `MAX_EXACT_DIGITS` significant digits (2591 + 1023 + 1075 bits).
const LIMBS: usize = 176;
const LIMB_BYTES: usize = LIMBS * 8;

/// Significant digits kept for the exact comparison. A double's midpoint has at
/// most 768 significant decimal digits, so once two values agree through 780 the
/// only thing left to separate them is whether anything non-zero was dropped —
/// which is what the sticky flag records.
const MAX_EXACT_DIGITS: usize = 780;

// Fixed scratch in the entry point's frame.
const DIG_OFF: usize = 0; // the input digits as one integer
const LEFT_OFF: usize = DIG_OFF + LIMB_BYTES;
const RIGHT_OFF: usize = LEFT_OFF + LIMB_BYTES;
const ENTRY_LOCAL_SIZE: usize = RIGHT_OFF + LIMB_BYTES;

const MANT_BITS: u8 = 52;
/// `u64::MAX >> (52 + 3)` — the ambiguity mask Lemire's error bound needs.
const PRODUCT_MASK: &str = "511";
const INFINITE_POWER: &str = "2047";
const U64_MAX: &str = "18446744073709551615";

/// Every function this parser needs. Emitted as a set because they only ever
/// appear together — the entry point is the sole caller of the rest.
pub(crate) fn lower_string_to_float_helpers() -> Vec<CodeFunction> {
    vec![
        lower_mul_small(),
        lower_shl(),
        lower_cmp(),
        lower_cmp_scaled(),
        lower_lemire(),
        lower_entry(),
    ]
}

fn function(
    name: &str,
    symbol: &str,
    mut ins: Vec<CodeInstruction>,
    relocations: Vec<CodeRelocation>,
    locals: usize,
) -> CodeFunction {
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut ins, &[], locals);
    CodeFunction {
        name: name.to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "Integer".to_string(),
        frame,
        stack_slots,
        instructions: ins,
        relocations,
    }
}

/// A `bl` to one of this family's own symbols, with its relocation.
fn call(from: &str, to: &str, ins: &mut Vec<CodeInstruction>, relocs: &mut Vec<CodeRelocation>) {
    ins.push(abi::branch_link(to));
    relocs.push(CodeRelocation {
        from: from.to_string(),
        to: to.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    });
}

// ---------------------------------------------------------------------------
// `_mfb_rt_f2s_mul_small` — `big = big * multiplier + addend`.
//
// `x0` = buffer, `x1` = multiplier (< 2^32), `x2` = addend (< 2^32).
// Fixed length, so there is no length to return and nothing to reallocate: the
// buffer is sized so the product cannot leave it.
// ---------------------------------------------------------------------------
fn lower_mul_small() -> CodeFunction {
    let symbol = MUL_SMALL_SYMBOL;
    let l = |suffix: &str| format!("{symbol}_{suffix}");
    let mut vregs = Vregs::new();
    let mut ins = vec![abi::label("entry")];

    let buf = vregs.next();
    let mul = vregs.next();
    let carry = vregs.next();
    let index = vregs.next();
    let addr = vregs.next();
    let limb = vregs.next();
    let product = vregs.next();
    let mask = vregs.next();

    let loop_top = l("loop");
    let loop_done = l("done");

    ins.extend([
        abi::move_register(&buf, abi::c_arg(0)),
        abi::move_register(&mul, abi::c_arg(1)),
        abi::move_register(&carry, abi::c_arg(2)),
        abi::move_immediate(&index, "Integer", "0"),
        abi::move_immediate(&mask, "Integer", "4294967295"),
        abi::label(&loop_top),
        abi::compare_immediate(&index, &LIMBS.to_string()),
        abi::branch_ge(&loop_done),
        abi::shift_left_immediate(&addr, &index, 3),
        abi::add_registers(&addr, &buf, &addr),
        abi::load_u64(&limb, &addr, 0),
        abi::multiply_registers(&product, &limb, &mul),
        abi::add_registers(&product, &product, &carry),
        abi::and_registers(&limb, &product, &mask),
        abi::store_u64(&limb, &addr, 0),
        abi::shift_right_immediate(&carry, &product, 32),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&loop_top),
        abi::label(&loop_done),
        abi::return_(),
    ]);

    function("runtime.f2sMulSmall", symbol, ins, Vec::new(), 0)
}

// ---------------------------------------------------------------------------
// `_mfb_rt_f2s_shl` — `big <<= bits`.
//
// `x0` = buffer, `x1` = bit count. Two passes: the sub-limb shift with a carry
// chain, then the whole-limb move. Splitting them keeps each pass one straight
// loop; doing it as `bits` separate one-bit shifts would be ~1000x the work on
// the deepest subnormal.
// ---------------------------------------------------------------------------
fn lower_shl() -> CodeFunction {
    let symbol = SHL_SYMBOL;
    let l = |suffix: &str| format!("{symbol}_{suffix}");
    let mut vregs = Vregs::new();
    let mut ins = vec![abi::label("entry")];

    let buf = vregs.next();
    let bits = vregs.next();
    let limb_shift = vregs.next();
    let bit_shift = vregs.next();
    let inverse = vregs.next();
    let carry = vregs.next();
    let index = vregs.next();
    let addr = vregs.next();
    let source = vregs.next();
    let limb = vregs.next();
    let next = vregs.next();
    let mask = vregs.next();

    let bit_loop = l("bit_loop");
    let bit_done = l("bit_done");
    let no_bit_shift = l("no_bit_shift");
    let limb_loop = l("limb_loop");
    let limb_done = l("limb_done");
    let no_limb_shift = l("no_limb_shift");
    let take_zero = l("take_zero");
    let stored = l("stored");

    ins.extend([
        abi::move_register(&buf, abi::c_arg(0)),
        abi::move_register(&bits, abi::c_arg(1)),
        abi::move_immediate(&mask, "Integer", "4294967295"),
        abi::shift_right_immediate(&limb_shift, &bits, 5), // bits / 32
        abi::move_immediate(&bit_shift, "Integer", "31"),
        abi::and_registers(&bit_shift, &bits, &bit_shift), // bits % 32
        // --- pass 1: shift within limbs ---------------------------------
        abi::compare_immediate(&bit_shift, "0"),
        abi::branch_eq(&no_bit_shift),
        abi::move_immediate(&inverse, "Integer", "32"),
        abi::subtract_registers(&inverse, &inverse, &bit_shift),
        abi::move_immediate(&carry, "Integer", "0"),
        abi::move_immediate(&index, "Integer", "0"),
        abi::label(&bit_loop),
        abi::compare_immediate(&index, &LIMBS.to_string()),
        abi::branch_ge(&bit_done),
        abi::shift_left_immediate(&addr, &index, 3),
        abi::add_registers(&addr, &buf, &addr),
        abi::load_u64(&limb, &addr, 0),
        // The outgoing carry is read before `limb` is overwritten.
        abi::shift_right_variable(&next, &limb, &inverse),
        abi::shift_left_variable(&limb, &limb, &bit_shift),
        abi::or_registers(&limb, &limb, &carry),
        abi::and_registers(&limb, &limb, &mask),
        abi::store_u64(&limb, &addr, 0),
        abi::move_register(&carry, &next),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&bit_loop),
        abi::label(&bit_done),
        abi::label(&no_bit_shift),
        // --- pass 2: move whole limbs -----------------------------------
        abi::compare_immediate(&limb_shift, "0"),
        abi::branch_eq(&no_limb_shift),
        // Walk downward so a source limb is read before it is overwritten.
        abi::move_immediate(&index, "Integer", &(LIMBS - 1).to_string()),
        abi::label(&limb_loop),
        abi::compare_immediate(&index, "0"),
        abi::branch_lt(&limb_done),
        abi::subtract_registers(&source, &index, &limb_shift),
        abi::compare_immediate(&source, "0"),
        abi::branch_lt(&take_zero),
        abi::shift_left_immediate(&addr, &source, 3),
        abi::add_registers(&addr, &buf, &addr),
        abi::load_u64(&limb, &addr, 0),
        abi::branch(&stored),
        abi::label(&take_zero),
        abi::move_immediate(&limb, "Integer", "0"),
        abi::label(&stored),
        abi::shift_left_immediate(&addr, &index, 3),
        abi::add_registers(&addr, &buf, &addr),
        abi::store_u64(&limb, &addr, 0),
        abi::subtract_immediate(&index, &index, 1),
        abi::branch(&limb_loop),
        abi::label(&limb_done),
        abi::label(&no_limb_shift),
        abi::return_(),
    ]);

    function("runtime.f2sShl", symbol, ins, Vec::new(), 0)
}

// ---------------------------------------------------------------------------
// `_mfb_rt_f2s_cmp` — compare two fixed-length bignums.
//
// `x0` = a, `x1` = b. Returns `x0` = 0 when equal, 1 when a > b, 2 when a < b.
// Distinct codes rather than a signed result, so the caller branches on
// equality without a sign convention to get wrong.
// ---------------------------------------------------------------------------
fn lower_cmp() -> CodeFunction {
    let symbol = CMP_SYMBOL;
    let l = |suffix: &str| format!("{symbol}_{suffix}");
    let mut vregs = Vregs::new();
    let mut ins = vec![abi::label("entry")];

    let a = vregs.next();
    let b = vregs.next();
    let index = vregs.next();
    let addr = vregs.next();
    let left = vregs.next();
    let right = vregs.next();

    let loop_top = l("loop");
    let equal = l("equal");
    let greater = l("greater");
    let less = l("less");

    ins.extend([
        abi::move_register(&a, abi::c_arg(0)),
        abi::move_register(&b, abi::c_arg(1)),
        // Most significant limb first: the first difference decides.
        abi::move_immediate(&index, "Integer", &(LIMBS - 1).to_string()),
        abi::label(&loop_top),
        abi::compare_immediate(&index, "0"),
        abi::branch_lt(&equal),
        abi::shift_left_immediate(&addr, &index, 3),
        abi::add_registers(&left, &a, &addr),
        abi::load_u64(&left, &left, 0),
        abi::add_registers(&right, &b, &addr),
        abi::load_u64(&right, &right, 0),
        abi::compare_registers(&left, &right),
        abi::branch_hi(&greater),
        abi::branch_lo(&less),
        abi::subtract_immediate(&index, &index, 1),
        abi::branch(&loop_top),
        abi::label(&equal),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", "0"),
        abi::return_(),
        abi::label(&greater),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", "1"),
        abi::return_(),
        abi::label(&less),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", "2"),
        abi::return_(),
    ]);

    function("runtime.f2sCmp", symbol, ins, Vec::new(), 0)
}

// ---------------------------------------------------------------------------
// `_mfb_rt_f2s_cmp_scaled` — compare `D * 10^q` against `m * 2^e`, exactly.
//
// `x0` = D buffer, `x1` = q (signed), `x2` = m, `x3` = e (signed),
// `x4` = left scratch, `x5` = right scratch. Returns `x0` = 0/1/2 as
// `_mfb_rt_f2s_cmp` does.
//
// Cross-multiplying clears both negative exponents at once, so there is no
// division anywhere and no approximation to bound:
//
//     left  = D * 10^max(q,0) * 2^max(-e,0)
//     right = m * 10^max(-q,0) * 2^max(e,0)
// ---------------------------------------------------------------------------
fn lower_cmp_scaled() -> CodeFunction {
    let symbol = CMP_SCALED_SYMBOL;
    let l = |suffix: &str| format!("{symbol}_{suffix}");
    let mut vregs = Vregs::new();
    let mut relocs: Vec<CodeRelocation> = Vec::new();
    let mut ins = vec![abi::label("entry")];

    let digits = vregs.next();
    let q = vregs.next();
    let m = vregs.next();
    let e = vregs.next();
    let left = vregs.next();
    let right = vregs.next();
    let index = vregs.next();
    let addr = vregs.next();
    let limb = vregs.next();
    let scratch = vregs.next();
    let target = vregs.next();
    let remaining = vregs.next();

    let copy_loop = l("copy_loop");
    let copy_done = l("copy_done");
    let zero_loop = l("zero_loop");
    let zero_done = l("zero_done");
    let pow10_target = l("pow10_target");
    let pow10_left = l("pow10_left");
    let pow10_right = l("pow10_right");
    let pow10_done = l("pow10_done");
    let big_loop = l("big_loop");
    let big_done = l("big_done");
    let small_loop = l("small_loop");
    let small_done = l("small_done");
    let shift_left_side = l("shift_left");
    let shift_done = l("shift_done");

    ins.extend([
        abi::move_register(&digits, abi::c_arg(0)),
        abi::move_register(&q, abi::c_arg(1)),
        abi::move_register(&m, abi::c_arg(2)),
        abi::move_register(&e, abi::c_arg(3)),
        abi::move_register(&left, abi::c_arg(4)),
        abi::move_register(&right, abi::c_arg(5)),
        // left = D
        abi::move_immediate(&index, "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_immediate(&index, &LIMBS.to_string()),
        abi::branch_ge(&copy_done),
        abi::shift_left_immediate(&addr, &index, 3),
        abi::add_registers(&scratch, &digits, &addr),
        abi::load_u64(&limb, &scratch, 0),
        abi::add_registers(&scratch, &left, &addr),
        abi::store_u64(&limb, &scratch, 0),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        // right = m, as two 32-bit limbs then zeros.
        abi::move_immediate(&index, "Integer", "0"),
        abi::label(&zero_loop),
        abi::compare_immediate(&index, &LIMBS.to_string()),
        abi::branch_ge(&zero_done),
        abi::shift_left_immediate(&addr, &index, 3),
        abi::add_registers(&scratch, &right, &addr),
        abi::store_u64(abi::ZERO, &scratch, 0),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&zero_loop),
        abi::label(&zero_done),
        abi::move_immediate(&scratch, "Integer", "4294967295"),
        abi::and_registers(&limb, &m, &scratch),
        abi::store_u64(&limb, &right, 0),
        abi::shift_right_immediate(&limb, &m, 32),
        abi::store_u64(&limb, &right, 8),
        // --- the power of ten goes on whichever side clears it ----------
        abi::compare_immediate(&q, "0"),
        abi::branch_eq(&pow10_done),
        abi::branch_gt(&pow10_left),
        abi::branch(&pow10_right),
        abi::label(&pow10_left),
        abi::move_register(&target, &left),
        abi::move_register(&remaining, &q),
        abi::branch(&pow10_target),
        abi::label(&pow10_right),
        abi::move_register(&target, &right),
        abi::subtract_registers(&remaining, abi::ZERO, &q),
        abi::label(&pow10_target),
        // Nine digits at a time: 10^9 is the largest power of ten below 2^32,
        // so each pass is one small multiply rather than nine.
        abi::label(&big_loop),
        abi::compare_immediate(&remaining, "9"),
        abi::branch_lt(&big_done),
    ]);
    ins.extend([
        abi::move_register(abi::c_arg(0), &target),
        abi::move_immediate(abi::c_arg(1), "Integer", "1000000000"),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
    ]);
    call(symbol, MUL_SMALL_SYMBOL, &mut ins, &mut relocs);
    ins.extend([
        abi::subtract_immediate(&remaining, &remaining, 9),
        abi::branch(&big_loop),
        abi::label(&big_done),
        abi::label(&small_loop),
        abi::compare_immediate(&remaining, "0"),
        abi::branch_le(&small_done),
    ]);
    ins.extend([
        abi::move_register(abi::c_arg(0), &target),
        abi::move_immediate(abi::c_arg(1), "Integer", "10"),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
    ]);
    call(symbol, MUL_SMALL_SYMBOL, &mut ins, &mut relocs);
    ins.extend([
        abi::subtract_immediate(&remaining, &remaining, 1),
        abi::branch(&small_loop),
        abi::label(&small_done),
        abi::label(&pow10_done),
        // --- the power of two goes on the other side --------------------
        abi::compare_immediate(&e, "0"),
        abi::branch_eq(&shift_done),
        abi::branch_lt(&shift_left_side),
        abi::move_register(&target, &right),
        abi::move_register(&remaining, &e),
        abi::branch(&l("do_shift")),
        abi::label(&shift_left_side),
        abi::move_register(&target, &left),
        abi::subtract_registers(&remaining, abi::ZERO, &e),
        abi::label(&l("do_shift")),
        abi::move_register(abi::c_arg(0), &target),
        abi::move_register(abi::c_arg(1), &remaining),
    ]);
    call(symbol, SHL_SYMBOL, &mut ins, &mut relocs);
    ins.extend([
        abi::label(&shift_done),
        abi::move_register(abi::c_arg(0), &left),
        abi::move_register(abi::c_arg(1), &right),
    ]);
    call(symbol, CMP_SYMBOL, &mut ins, &mut relocs);
    ins.push(abi::return_());

    function("runtime.f2sCmpScaled", symbol, ins, relocs, 0)
}

// ---------------------------------------------------------------------------
// `_mfb_rt_f2s_lemire` — Eisel–Lemire.
//
// `x0` = decimal exponent q (signed), `x1` = mantissa w.
// Returns `x0` = 1 when the 128-bit approximation could not certify the
// rounding (the caller must fall back), else 0; `x1` = the mantissa field;
// `x2` = the biased binary exponent.
//
// An uncertified result still carries a value within one ULP of the truth,
// which is what lets the exact fallback seed its search from it instead of
// deriving a second approximation by other means.
// ---------------------------------------------------------------------------
fn lower_lemire() -> CodeFunction {
    let symbol = LEMIRE_SYMBOL;
    let l = |suffix: &str| format!("{symbol}_{suffix}");
    let mut vregs = Vregs::new();
    let mut relocations: Vec<CodeRelocation> = Vec::new();
    let mut ins = vec![abi::label("entry")];

    let q = vregs.next();
    let w = vregs.next();
    let lz = vregs.next();
    let base = vregs.next();
    let table = vregs.next();
    let table_hi = vregs.next();
    let table_lo = vregs.next();
    let first_lo = vregs.next();
    let first_hi = vregs.next();
    let second_hi = vregs.next();
    let carry = vregs.next();
    let scratch = vregs.next();
    let mask = vregs.next();
    let upperbit = vregs.next();
    let shift = vregs.next();
    let mantissa = vregs.next();
    let power2 = vregs.next();
    let uncertain = vregs.next();

    let return_zero = l("zero");
    let return_inf = l("inf");
    let no_second = l("no_second");
    let no_carry = l("no_carry");
    let not_all_ones = l("not_all_ones");
    let set_uncertain = l("set_uncertain");
    let subnormal = l("subnormal");
    let subnormal_small = l("subnormal_small");
    let not_tie = l("not_tie");
    let no_overflow = l("no_overflow");
    let finish = l("finish");

    ins.extend([
        abi::move_register(&q, abi::c_arg(0)),
        abi::move_register(&w, abi::c_arg(1)),
        abi::move_immediate(&uncertain, "Integer", "0"),
        abi::compare_immediate(&w, "0"),
        abi::branch_eq(&return_zero),
        abi::move_immediate(&scratch, "Integer", &Q_MIN.unsigned_abs().to_string()),
        abi::subtract_registers(&scratch, abi::ZERO, &scratch),
        abi::compare_registers(&q, &scratch),
        abi::branch_lt(&return_zero),
        abi::compare_immediate(&q, &Q_MAX.to_string()),
        abi::branch_gt(&return_inf),
        abi::count_leading_zeros(&lz, &w),
        abi::shift_left_variable(&w, &w, &lz),
    ]);

    emit_load_data_address_free(
        symbol,
        POWERS_OF_TEN_SYMBOL,
        &base,
        &mut ins,
        &mut relocations,
    );

    ins.extend([
        abi::move_immediate(&scratch, "Integer", &Q_MIN.unsigned_abs().to_string()),
        abi::add_registers(&table, &q, &scratch), // q - Q_MIN
        abi::shift_left_immediate(&table, &table, 4),
        abi::add_registers(&table, &base, &table),
        abi::load_u64(&table_lo, &table, 0),
        abi::load_u64(&table_hi, &table, 8),
        abi::multiply_registers(&first_lo, &w, &table_hi),
        abi::unsigned_multiply_high_registers(&first_hi, &w, &table_hi),
        // All ones in the low 9 bits means the product is too close to call; a
        // second multiplication against the table's low half sharpens it.
        abi::move_immediate(&mask, "Integer", PRODUCT_MASK),
        abi::and_registers(&scratch, &first_hi, &mask),
        abi::compare_registers(&scratch, &mask),
        abi::branch_ne(&no_second),
        abi::unsigned_multiply_high_registers(&second_hi, &w, &table_lo),
        abi::add_carry(&first_lo, &carry, &first_lo, &second_hi, abi::ZERO),
        abi::compare_immediate(&carry, "0"),
        abi::branch_eq(&no_carry),
        abi::add_immediate(&first_hi, &first_hi, 1),
        abi::label(&no_carry),
        abi::label(&no_second),
        // An all-ones low word means adding one could ripple over the halfway
        // point. Inside a narrow exponent window that is provably harmless;
        // outside it, the result is computed anyway but flagged uncertain so
        // the caller can both fall back and seed from it.
        abi::move_immediate(&scratch, "Integer", U64_MAX),
        abi::compare_registers(&first_lo, &scratch),
        abi::branch_ne(&not_all_ones),
        abi::compare_immediate(&q, "55"),
        abi::branch_gt(&set_uncertain),
        abi::move_immediate(&scratch, "Integer", "27"),
        abi::subtract_registers(&scratch, abi::ZERO, &scratch),
        abi::compare_registers(&q, &scratch),
        abi::branch_ge(&not_all_ones),
        abi::label(&set_uncertain),
        abi::move_immediate(&uncertain, "Integer", "1"),
        abi::label(&not_all_ones),
        abi::shift_right_immediate(&upperbit, &first_hi, 63),
        abi::add_immediate(&shift, &upperbit, (64 - MANT_BITS - 3) as usize),
        abi::shift_right_variable(&mantissa, &first_hi, &shift),
        // power(q) = ((q * 217706) >> 16) + 63, exact over the table's range;
        // then fold in the normalization shift and the f64 bias.
        abi::move_immediate(&scratch, "Integer", "217706"),
        abi::multiply_registers(&power2, &q, &scratch),
        abi::arithmetic_shift_right_immediate(&power2, &power2, 16),
        abi::add_immediate(&power2, &power2, 63),
        abi::add_registers(&power2, &power2, &upperbit),
        abi::subtract_registers(&power2, &power2, &lz),
        abi::add_immediate(&power2, &power2, 1023),
        abi::compare_immediate(&power2, "0"),
        abi::branch_le(&subnormal),
        // An exact tie rounds to even: the discarded part is exactly one half
        // when the low word is 0 or 1 and the shift reproduces `first_hi`.
        abi::compare_immediate(&first_lo, "1"),
        abi::branch_hi(&not_tie),
        abi::compare_immediate(&q, "23"),
        abi::branch_gt(&not_tie),
        abi::move_immediate(&scratch, "Integer", "4"),
        abi::subtract_registers(&scratch, abi::ZERO, &scratch),
        abi::compare_registers(&q, &scratch),
        abi::branch_lt(&not_tie),
        abi::move_immediate(&scratch, "Integer", "3"),
        abi::and_registers(&scratch, &mantissa, &scratch),
        abi::compare_immediate(&scratch, "1"),
        abi::branch_ne(&not_tie),
        abi::shift_left_variable(&scratch, &mantissa, &shift),
        abi::compare_registers(&scratch, &first_hi),
        abi::branch_ne(&not_tie),
        abi::move_immediate(&scratch, "Integer", "1"),
        abi::bitwise_not(&scratch, &scratch),
        abi::and_registers(&mantissa, &mantissa, &scratch),
        abi::label(&not_tie),
        // Round to nearest, then renormalize if the carry pushed a bit out.
        abi::move_immediate(&scratch, "Integer", "1"),
        abi::and_registers(&scratch, &mantissa, &scratch),
        abi::add_registers(&mantissa, &mantissa, &scratch),
        abi::shift_right_immediate(&mantissa, &mantissa, 1),
        abi::move_immediate(&scratch, "Integer", "1"),
        abi::shift_left_immediate(&scratch, &scratch, MANT_BITS + 1),
        abi::compare_registers(&mantissa, &scratch),
        abi::branch_lo(&no_overflow),
        abi::shift_right_immediate(&mantissa, &scratch, 1),
        abi::add_immediate(&power2, &power2, 1),
        abi::label(&no_overflow),
        // Drop the implicit bit; the exponent field carries it.
        abi::move_immediate(&scratch, "Integer", "1"),
        abi::shift_left_immediate(&scratch, &scratch, MANT_BITS),
        abi::bitwise_not(&scratch, &scratch),
        abi::and_registers(&mantissa, &mantissa, &scratch),
        abi::compare_immediate(&power2, INFINITE_POWER),
        abi::branch_ge(&return_inf),
        abi::branch(&finish),
        // --- subnormal --------------------------------------------------
        // The implicit bit is deliberately NOT cleared here: when rounding
        // carries out of the subnormal range the mantissa becomes exactly 2^52
        // and the exponent field becomes 1, and OR-ing them at assembly time
        // produces the smallest normal.
        abi::label(&subnormal),
        abi::move_immediate(&scratch, "Integer", "1"),
        abi::subtract_registers(&scratch, &scratch, &power2), // 1 - power2
        abi::compare_immediate(&scratch, "64"),
        abi::branch_ge(&return_zero),
        abi::shift_right_variable(&mantissa, &mantissa, &scratch),
        abi::move_immediate(&scratch, "Integer", "1"),
        abi::and_registers(&scratch, &mantissa, &scratch),
        abi::add_registers(&mantissa, &mantissa, &scratch),
        abi::shift_right_immediate(&mantissa, &mantissa, 1),
        abi::move_immediate(&scratch, "Integer", "1"),
        abi::shift_left_immediate(&scratch, &scratch, MANT_BITS),
        abi::compare_registers(&mantissa, &scratch),
        abi::branch_lo(&subnormal_small),
        abi::move_immediate(&power2, "Integer", "1"),
        abi::branch(&finish),
        abi::label(&subnormal_small),
        abi::move_immediate(&power2, "Integer", "0"),
        abi::branch(&finish),
        // --- trivial exits ----------------------------------------------
        abi::label(&return_zero),
        abi::move_immediate(&mantissa, "Integer", "0"),
        abi::move_immediate(&power2, "Integer", "0"),
        abi::branch(&finish),
        abi::label(&return_inf),
        abi::move_immediate(&mantissa, "Integer", "0"),
        abi::move_immediate(&power2, "Integer", INFINITE_POWER),
        abi::label(&finish),
        abi::move_register(RESULT_TAG_REGISTER, &uncertain),
        abi::move_register(RESULT_VALUE_REGISTER, &mantissa),
        abi::move_register(abi::mfb_return(2), &power2),
        abi::return_(),
    ]);

    function("runtime.f2sLemire", symbol, ins, relocations, 0)
}

// ---------------------------------------------------------------------------
// `_mfb_rt_string_to_float` — the entry point.
//
// Scan, Lemire, exact fallback, assemble. Entirely integer: there is no f64
// arithmetic anywhere, because a hand-written NIR helper may not name a
// physical FP register and Clinger's fast path is the only part of the
// algorithm that wanted one. `lemire_alone_is_sufficient` in the reference
// proves dropping it changes no result.
// ---------------------------------------------------------------------------
fn lower_entry() -> CodeFunction {
    let symbol = STRING_TO_FLOAT_SYMBOL;
    let l = |suffix: &str| format!("{symbol}_{suffix}");
    let mut vregs = Vregs::new();
    let mut relocs: Vec<CodeRelocation> = Vec::new();
    let mut ins = vec![abi::label("entry")];

    let text = vregs.next();
    let len = vregs.next();
    let cursor = vregs.next();
    let index = vregs.next();
    let negative = vregs.next();
    let byte = vregs.next();
    let digit = vregs.next();
    let ten = vregs.next();
    let scratch = vregs.next();
    let mantissa = vregs.next();
    let digit_count = vregs.next();
    let many = vregs.next();
    let seen = vregs.next();
    let dot = vregs.next();
    let fractional = vregs.next();
    let significant = vregs.next();
    let dropped = vregs.next();
    let exponent_value = vregs.next();
    let exponent_negative = vregs.next();
    let exponent_seen = vregs.next();
    let digits_exponent = vregs.next();
    let exponent = vregs.next();
    let uncertain = vregs.next();
    let best_mantissa = vregs.next();
    let best_power = vregs.next();
    let bits = vregs.next();

    let invalid = l("invalid");
    let check_plus = l("check_plus");
    let after_sign = l("after_sign");
    let sign_done = l("sign_done");
    let scan_loop = l("scan_loop");
    let scan_done = l("scan_done");
    let handle_dot = l("handle_dot");
    let scan_next = l("scan_next");
    let no_fraction = l("no_fraction");
    let not_significant = l("not_significant");
    let drop_digit = l("drop_digit");
    let exponent_start = l("exp_start");
    let exponent_check_plus = l("exp_check_plus");
    let exponent_sign_done = l("exp_sign_done");
    let exponent_loop = l("exp_loop");
    let exponent_skip = l("exp_skip");
    let exponent_done = l("exp_done");
    let exponent_positive = l("exp_positive");
    let have_result = l("have_result");
    let mark_uncertain = l("mark_uncertain");
    let fallback = l("fallback");
    let apply_sign = l("apply_sign");
    let no_sign = l("no_sign");
    let done = l("done");

    // --- scan: sign -------------------------------------------------------
    ins.extend([
        abi::move_register(&text, abi::c_arg(0)),
        abi::load_u64(&len, &text, 0),
        abi::compare_immediate(&len, "0"),
        abi::branch_eq(&invalid),
        abi::add_immediate(&cursor, &text, 8),
        abi::move_immediate(&index, "Integer", "0"),
        abi::move_immediate(&negative, "Integer", "0"),
        abi::move_immediate(&ten, "Integer", "10"),
        abi::load_u8(&byte, &cursor, 0),
        abi::compare_immediate(&byte, "45"),
        abi::branch_ne(&check_plus),
        abi::move_immediate(&negative, "Integer", "1"),
        abi::add_immediate(&index, &index, 1),
        abi::add_immediate(&cursor, &cursor, 1),
        abi::branch(&after_sign),
        abi::label(&check_plus),
        abi::compare_immediate(&byte, "43"),
        abi::branch_ne(&sign_done),
        abi::add_immediate(&index, &index, 1),
        abi::add_immediate(&cursor, &cursor, 1),
        abi::label(&after_sign),
        // A sign with nothing after it is not a number. Only checked when a
        // sign was actually consumed, matching the old scanner exactly.
        abi::compare_registers(&index, &len),
        abi::branch_ge(&invalid),
        abi::label(&sign_done),
        abi::move_immediate(&mantissa, "Integer", "0"),
        abi::move_immediate(&digit_count, "Integer", "0"),
        abi::move_immediate(&many, "Integer", "0"),
        abi::move_immediate(&seen, "Integer", "0"),
        abi::move_immediate(&dot, "Integer", "0"),
        abi::move_immediate(&fractional, "Integer", "0"),
        abi::move_immediate(&significant, "Integer", "0"),
        abi::move_immediate(&dropped, "Integer", "0"),
        abi::move_immediate(&exponent_value, "Integer", "0"),
        abi::move_immediate(&exponent_negative, "Integer", "0"),
        // Starts satisfied: only an `e` clears it, so a number without an
        // exponent passes the same check at the end.
        abi::move_immediate(&exponent_seen, "Integer", "1"),
    ]);

    // --- scan: digits and the dot ----------------------------------------
    ins.extend([
        abi::label(&scan_loop),
        abi::compare_registers(&index, &len),
        abi::branch_ge(&scan_done),
        abi::load_u8(&byte, &cursor, 0),
        abi::compare_immediate(&byte, "46"),
        abi::branch_eq(&handle_dot),
        abi::compare_immediate(&byte, "101"),
        abi::branch_eq(&exponent_start),
        abi::compare_immediate(&byte, "69"),
        abi::branch_eq(&exponent_start),
        abi::compare_immediate(&byte, "48"),
        abi::branch_lo(&invalid),
        abi::compare_immediate(&byte, "57"),
        abi::branch_hi(&invalid),
        abi::subtract_immediate(&digit, &byte, 48),
        abi::move_immediate(&seen, "Integer", "1"),
        abi::compare_immediate(&dot, "0"),
        abi::branch_eq(&no_fraction),
        abi::add_immediate(&fractional, &fractional, 1),
        abi::label(&no_fraction),
        // Leading zeros are not significant and must not consume mantissa room.
        abi::compare_immediate(&digit, "0"),
        abi::branch_eq(&not_significant),
        abi::move_immediate(&significant, "Integer", "1"),
        abi::label(&not_significant),
        abi::compare_immediate(&significant, "0"),
        abi::branch_eq(&scan_next),
        abi::compare_immediate(&digit_count, "19"),
        abi::branch_ge(&drop_digit),
        abi::multiply_registers(&mantissa, &mantissa, &ten),
        abi::add_registers(&mantissa, &mantissa, &digit),
        abi::add_immediate(&digit_count, &digit_count, 1),
        abi::branch(&scan_next),
        abi::label(&drop_digit),
        // Past 19 digits the mantissa is truncated; each dropped digit still
        // shifts the exponent, and `many` tells the caller the value is only
        // bracketed rather than exact.
        abi::move_immediate(&many, "Integer", "1"),
        abi::add_immediate(&dropped, &dropped, 1),
        abi::branch(&scan_next),
        abi::label(&handle_dot),
        abi::compare_immediate(&dot, "0"),
        abi::branch_ne(&invalid),
        abi::move_immediate(&dot, "Integer", "1"),
        abi::label(&scan_next),
        abi::add_immediate(&index, &index, 1),
        abi::add_immediate(&cursor, &cursor, 1),
        abi::branch(&scan_loop),
    ]);

    // --- scan: the exponent ----------------------------------------------
    ins.extend([
        abi::label(&exponent_start),
        abi::compare_immediate(&seen, "0"),
        abi::branch_eq(&invalid),
        abi::add_immediate(&index, &index, 1),
        abi::add_immediate(&cursor, &cursor, 1),
        abi::compare_registers(&index, &len),
        abi::branch_ge(&invalid),
        abi::move_immediate(&exponent_seen, "Integer", "0"),
        abi::load_u8(&byte, &cursor, 0),
        abi::compare_immediate(&byte, "45"),
        abi::branch_ne(&exponent_check_plus),
        abi::move_immediate(&exponent_negative, "Integer", "1"),
        abi::add_immediate(&index, &index, 1),
        abi::add_immediate(&cursor, &cursor, 1),
        abi::branch(&exponent_sign_done),
        abi::label(&exponent_check_plus),
        abi::compare_immediate(&byte, "43"),
        abi::branch_ne(&exponent_sign_done),
        abi::add_immediate(&index, &index, 1),
        abi::add_immediate(&cursor, &cursor, 1),
        abi::label(&exponent_sign_done),
        abi::compare_registers(&index, &len),
        abi::branch_ge(&invalid),
        abi::label(&exponent_loop),
        abi::compare_registers(&index, &len),
        abi::branch_ge(&exponent_done),
        abi::load_u8(&byte, &cursor, 0),
        abi::compare_immediate(&byte, "48"),
        abi::branch_lo(&invalid),
        abi::compare_immediate(&byte, "57"),
        abi::branch_hi(&invalid),
        abi::move_immediate(&exponent_seen, "Integer", "1"),
        // Clamp rather than wrap: past this magnitude every finite mantissa
        // already overflows or underflows, so further digits cannot change the
        // answer. The old scanner clamped for the same reason.
        abi::compare_immediate(&exponent_value, "100000"),
        abi::branch_ge(&exponent_skip),
        abi::subtract_immediate(&digit, &byte, 48),
        abi::multiply_registers(&exponent_value, &exponent_value, &ten),
        abi::add_registers(&exponent_value, &exponent_value, &digit),
        abi::label(&exponent_skip),
        abi::add_immediate(&index, &index, 1),
        abi::add_immediate(&cursor, &cursor, 1),
        abi::branch(&exponent_loop),
        abi::label(&exponent_done),
        abi::label(&scan_done),
        abi::compare_immediate(&seen, "0"),
        abi::branch_eq(&invalid),
        abi::compare_immediate(&exponent_seen, "0"),
        abi::branch_eq(&invalid),
        abi::compare_immediate(&exponent_negative, "0"),
        abi::branch_eq(&exponent_positive),
        abi::subtract_registers(&exponent_value, abi::ZERO, &exponent_value),
        abi::label(&exponent_positive),
        // `digits_exponent` scales the whole digit string; `exponent` scales the
        // truncated 19-digit mantissa.
        abi::subtract_registers(&digits_exponent, &exponent_value, &fractional),
        abi::add_registers(&exponent, &digits_exponent, &dropped),
    ]);

    // --- Eisel-Lemire, once or twice --------------------------------------
    ins.extend([
        abi::move_register(abi::c_arg(0), &exponent),
        abi::move_register(abi::c_arg(1), &mantissa),
    ]);
    call(symbol, LEMIRE_SYMBOL, &mut ins, &mut relocs);
    ins.extend([
        abi::move_register(&uncertain, RESULT_TAG_REGISTER),
        abi::move_register(&best_mantissa, RESULT_VALUE_REGISTER),
        abi::move_register(&best_power, abi::mfb_return(2)),
        // A truncated mantissa brackets the value between `mantissa` and
        // `mantissa + 1`. If both round to the same double the truncation was
        // harmless; if not, only the exact comparison can decide.
        abi::compare_immediate(&many, "0"),
        abi::branch_eq(&have_result),
        abi::compare_immediate(&uncertain, "0"),
        abi::branch_ne(&have_result),
        abi::move_register(abi::c_arg(0), &exponent),
        abi::add_immediate(&scratch, &mantissa, 1),
        abi::move_register(abi::c_arg(1), &scratch),
    ]);
    call(symbol, LEMIRE_SYMBOL, &mut ins, &mut relocs);
    ins.extend([
        abi::compare_immediate(RESULT_TAG_REGISTER, "0"),
        abi::branch_ne(&mark_uncertain),
        abi::compare_registers(RESULT_VALUE_REGISTER, &best_mantissa),
        abi::branch_ne(&mark_uncertain),
        abi::compare_registers(abi::mfb_return(2), &best_power),
        abi::branch_eq(&have_result),
        abi::label(&mark_uncertain),
        abi::move_immediate(&uncertain, "Integer", "1"),
        abi::label(&have_result),
        abi::shift_left_immediate(&bits, &best_power, MANT_BITS),
        abi::or_registers(&bits, &bits, &best_mantissa),
        abi::compare_immediate(&uncertain, "0"),
        abi::branch_eq(&apply_sign),
    ]);

    // --- the exact fallback ------------------------------------------------
    let digits_buffer = vregs.next();
    let left_buffer = vregs.next();
    let right_buffer = vregs.next();
    let kept = vregs.next();
    let sticky = vregs.next();
    let adjust = vregs.next();
    let significant2 = vregs.next();
    let exact_exponent = vregs.next();
    let candidate = vregs.next();
    let biased = vregs.next();
    let fraction = vregs.next();
    let cand_mantissa = vregs.next();
    let cand_exponent = vregs.next();
    let midpoint = vregs.next();
    let addr = vregs.next();

    let zero_loop = l("zero_loop");
    let zero_done = l("zero_done");
    let rescan_sign = l("rescan_sign");
    let rescan_loop = l("rescan_loop");
    let rescan_done = l("rescan_done");
    let rescan_next = l("rescan_next");
    let rescan_take = l("rescan_take");
    let rescan_drop = l("rescan_drop");
    let maybe_leading = l("maybe_leading");
    let no_sticky = l("no_sticky");
    let seed_ready = l("seed_ready");
    let seed_not_inf = l("seed_not_inf");
    let cand_loop = l("cand_loop");
    let cand_normal = l("cand_normal");
    let cand_decomposed = l("cand_decomposed");
    let skip_lower = l("skip_lower");
    let cand_down = l("cand_down");
    let cand_up = l("cand_up");
    let cand_up_once = l("cand_up_once");
    let cand_final = l("cand_final");
    let tie_even = l("tie_even");

    ins.extend([
        abi::label(&fallback),
        abi::add_immediate(&digits_buffer, abi::stack_pointer(), DIG_OFF),
        abi::add_immediate(&left_buffer, abi::stack_pointer(), LEFT_OFF),
        abi::add_immediate(&right_buffer, abi::stack_pointer(), RIGHT_OFF),
        abi::move_immediate(&index, "Integer", "0"),
        abi::label(&zero_loop),
        abi::compare_immediate(&index, &LIMBS.to_string()),
        abi::branch_ge(&zero_done),
        abi::shift_left_immediate(&addr, &index, 3),
        abi::add_registers(&addr, &digits_buffer, &addr),
        abi::store_u64(abi::ZERO, &addr, 0),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&zero_loop),
        abi::label(&zero_done),
        // Walk the text again rather than buffering the digits during the first
        // pass: this path is rare, and a second walk costs nothing a program
        // that never reaches it would notice.
        abi::add_immediate(&cursor, &text, 8),
        abi::move_immediate(&index, "Integer", "0"),
        abi::move_immediate(&kept, "Integer", "0"),
        abi::move_immediate(&sticky, "Integer", "0"),
        abi::move_immediate(&adjust, "Integer", "0"),
        abi::move_immediate(&significant2, "Integer", "0"),
        abi::load_u8(&byte, &cursor, 0),
        abi::compare_immediate(&byte, "45"),
        abi::branch_eq(&rescan_sign),
        abi::compare_immediate(&byte, "43"),
        abi::branch_ne(&rescan_loop),
        abi::label(&rescan_sign),
        abi::add_immediate(&index, &index, 1),
        abi::add_immediate(&cursor, &cursor, 1),
        abi::label(&rescan_loop),
        abi::compare_registers(&index, &len),
        abi::branch_ge(&rescan_done),
        abi::load_u8(&byte, &cursor, 0),
        abi::compare_immediate(&byte, "46"),
        abi::branch_eq(&rescan_next),
        abi::compare_immediate(&byte, "101"),
        abi::branch_eq(&rescan_done),
        abi::compare_immediate(&byte, "69"),
        abi::branch_eq(&rescan_done),
        abi::subtract_immediate(&digit, &byte, 48),
        abi::compare_immediate(&digit, "0"),
        abi::branch_eq(&maybe_leading),
        abi::move_immediate(&significant2, "Integer", "1"),
        abi::branch(&rescan_take),
        abi::label(&maybe_leading),
        abi::compare_immediate(&significant2, "0"),
        abi::branch_eq(&rescan_next),
        abi::label(&rescan_take),
        abi::compare_immediate(&kept, &MAX_EXACT_DIGITS.to_string()),
        abi::branch_ge(&rescan_drop),
        abi::move_register(abi::c_arg(0), &digits_buffer),
        abi::move_immediate(abi::c_arg(1), "Integer", "10"),
        abi::move_register(abi::c_arg(2), &digit),
    ]);
    call(symbol, MUL_SMALL_SYMBOL, &mut ins, &mut relocs);
    ins.extend([
        abi::add_immediate(&kept, &kept, 1),
        abi::branch(&rescan_next),
        abi::label(&rescan_drop),
        // Past the cap the digit only shifts the exponent; whether it was
        // non-zero is remembered so an otherwise exact tie can be broken.
        abi::add_immediate(&adjust, &adjust, 1),
        abi::compare_immediate(&digit, "0"),
        abi::branch_eq(&no_sticky),
        abi::move_immediate(&sticky, "Integer", "1"),
        abi::label(&no_sticky),
        abi::label(&rescan_next),
        abi::add_immediate(&index, &index, 1),
        abi::add_immediate(&cursor, &cursor, 1),
        abi::branch(&rescan_loop),
        abi::label(&rescan_done),
        abi::add_registers(&exact_exponent, &digits_exponent, &adjust),
        // Seed from Lemire's uncertified answer, which is still within one ULP.
        abi::move_register(&candidate, &bits),
        abi::move_immediate(&scratch, "Integer", "9218868437227405312"), // +inf
        abi::compare_registers(&candidate, &scratch),
        abi::branch_lo(&seed_not_inf),
        abi::subtract_immediate(&candidate, &scratch, 1), // largest finite
        abi::label(&seed_not_inf),
        abi::compare_immediate(&candidate, "0"),
        abi::branch_ne(&seed_ready),
        abi::move_immediate(&candidate, "Integer", "1"),
        abi::label(&seed_ready),
    ]);

    ins.extend([
        abi::label(&cand_loop),
        abi::shift_right_immediate(&biased, &candidate, MANT_BITS),
        abi::move_immediate(&scratch, "Integer", INFINITE_POWER),
        abi::and_registers(&biased, &biased, &scratch),
        abi::move_immediate(&scratch, "Integer", "1"),
        abi::shift_left_immediate(&scratch, &scratch, MANT_BITS),
        abi::subtract_immediate(&scratch, &scratch, 1),
        abi::and_registers(&fraction, &candidate, &scratch),
        abi::compare_immediate(&biased, "0"),
        abi::branch_ne(&cand_normal),
        abi::move_register(&cand_mantissa, &fraction),
        abi::move_immediate(&cand_exponent, "Integer", "1074"),
        abi::subtract_registers(&cand_exponent, abi::ZERO, &cand_exponent),
        abi::branch(&cand_decomposed),
        abi::label(&cand_normal),
        abi::move_immediate(&scratch, "Integer", "1"),
        abi::shift_left_immediate(&scratch, &scratch, MANT_BITS),
        abi::or_registers(&cand_mantissa, &fraction, &scratch),
        abi::subtract_immediate(&cand_exponent, &biased, 1075),
        abi::label(&cand_decomposed),
        // Below the lower midpoint? Then the answer is smaller than this
        // candidate. Skipped at zero, which has no lower neighbour.
        abi::compare_immediate(&cand_mantissa, "0"),
        abi::branch_eq(&skip_lower),
        abi::shift_left_immediate(&midpoint, &cand_mantissa, 1),
        abi::subtract_immediate(&midpoint, &midpoint, 1),
        abi::move_register(abi::c_arg(0), &digits_buffer),
        abi::move_register(abi::c_arg(1), &exact_exponent),
        abi::move_register(abi::c_arg(2), &midpoint),
        abi::subtract_immediate(&scratch, &cand_exponent, 1),
        abi::move_register(abi::c_arg(3), &scratch),
        abi::move_register(abi::c_arg(4), &left_buffer),
        abi::move_register(abi::c_arg(5), &right_buffer),
    ]);
    call(symbol, CMP_SCALED_SYMBOL, &mut ins, &mut relocs);
    ins.extend([
        abi::compare_immediate(RESULT_TAG_REGISTER, "2"),
        abi::branch_eq(&cand_down),
        abi::label(&skip_lower),
        abi::shift_left_immediate(&midpoint, &cand_mantissa, 1),
        abi::add_immediate(&midpoint, &midpoint, 1),
        abi::move_register(abi::c_arg(0), &digits_buffer),
        abi::move_register(abi::c_arg(1), &exact_exponent),
        abi::move_register(abi::c_arg(2), &midpoint),
        abi::subtract_immediate(&scratch, &cand_exponent, 1),
        abi::move_register(abi::c_arg(3), &scratch),
        abi::move_register(abi::c_arg(4), &left_buffer),
        abi::move_register(abi::c_arg(5), &right_buffer),
    ]);
    call(symbol, CMP_SCALED_SYMBOL, &mut ins, &mut relocs);
    ins.extend([
        abi::move_register(&scratch, RESULT_TAG_REGISTER),
        abi::compare_immediate(&scratch, "1"),
        abi::branch_eq(&cand_up),
        abi::compare_immediate(&scratch, "0"),
        abi::branch_ne(&cand_final),
        // Exactly the midpoint. Anything dropped past the digit cap makes the
        // true value larger, so it rounds up; otherwise ties go to even.
        abi::compare_immediate(&sticky, "0"),
        abi::branch_ne(&cand_up_once),
        abi::label(&tie_even),
        abi::move_immediate(&scratch, "Integer", "1"),
        abi::and_registers(&scratch, &candidate, &scratch),
        abi::compare_immediate(&scratch, "0"),
        abi::branch_eq(&cand_final),
        abi::label(&cand_up_once),
        abi::add_immediate(&candidate, &candidate, 1),
        abi::branch(&cand_final),
        abi::label(&cand_down),
        abi::subtract_immediate(&candidate, &candidate, 1),
        abi::branch(&cand_loop),
        abi::label(&cand_up),
        abi::add_immediate(&candidate, &candidate, 1),
        abi::branch(&cand_loop),
        abi::label(&cand_final),
        abi::move_register(&bits, &candidate),
    ]);

    // --- assemble ----------------------------------------------------------
    ins.extend([
        abi::label(&apply_sign),
        abi::compare_immediate(&negative, "0"),
        abi::branch_eq(&no_sign),
        abi::move_immediate(&scratch, "Integer", "1"),
        abi::shift_left_immediate(&scratch, &scratch, 63),
        abi::or_registers(&bits, &bits, &scratch),
        abi::label(&no_sign),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", "0"),
        abi::move_register(RESULT_VALUE_REGISTER, &bits),
        abi::branch(&done),
        abi::label(&invalid),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", "1"),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
        abi::label(&done),
        abi::return_(),
    ]);

    function(
        "runtime.stringToFloat",
        symbol,
        ins,
        relocs,
        ENTRY_LOCAL_SIZE,
    )
}
