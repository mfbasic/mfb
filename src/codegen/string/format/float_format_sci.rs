//! `_mfb_rt_float_to_string_sci` — the significant-digit stream behind
//! ECMAScript number rendering (plan-120-G), emitted as NIR.
//!
//! A transliteration of `sci_18` in `float_format_sci_ref.rs`, which is the
//! same thing in Rust, checked against Node v24.12.0 over 50,018 values. Read
//! that module first; this one carries only the register allocation.
//!
//! **It does not round.** It returns the first 18 significant digits
//! *truncated*, the decimal exponent, and a sticky flag saying whether anything
//! non-zero follows. Rounding to `p`, the all-nines ripple, the shortest-form
//! search and ECMAScript's placement all happen in MFBASIC
//! (`helper_stringify_number.rs`), because rounding is the part most likely to
//! be wrong and the hardest to test, and there is no reason for it to live in
//! hand-written assembly. `the_two_factorings_agree` proves the split is exact:
//! rounding an 18-digit truncation at `p`, with sticky recomputed from the
//! dropped digits, is indistinguishable from rounding the exact stream.
//!
//! One call also serves the whole `p = 1..=17` search rather than one call per
//! candidate.
//!
//! ## Why a sibling symbol rather than a mode flag on the fixed formatter
//!
//! Recorded here because plan-120-G Phase 1 asks for the decision and its
//! reasoning:
//!
//! - **The rounding bound is not the same shape.** The fixed formatter rounds
//!   after `prec` FRACTION digits using the limb remainder. Significant-digit
//!   work must round after `p` digits counted from the first non-zero, which for
//!   a large value falls *inside the integer digits* — a case the fixed path has
//!   no code for at all, since `prec >= 0` only ever rounds in the fraction.
//!   (This helper sidesteps it by not rounding, but a shared mode could not.)
//! - **The buffers want opposite things.** Reaching a subnormal's first
//!   significant digit means stepping over ~320 fraction zeros. The fixed path
//!   stores every fraction digit and caps at 255; this one stores 18 and never
//!   materializes a zero. Sharing the layout would mean growing the fixed
//!   fraction buffer to ~350 bytes for digits it never emits.
//! - **`x1` would change meaning** from "fraction places" to "significant
//!   digits" on a symbol with existing callers.
//!
//! The one real cost of a sibling is two copies of the decompose-and-place
//! preamble. That is accepted deliberately: the alternative is threading a mode
//! through a 600-instruction function whose output is pinned byte-for-byte by
//! goldens.

use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::memory::data::*;
use crate::target::shared::abi;

/// `x0` = f64 bits, **magnitude only** (the caller clears the sign), finite and
/// non-zero. Returns the standard allocation Result: `x0` = tag, `x1` = an
/// arena String of the form
///
/// ```text
/// <sticky><18 digits>e<exponent>
/// ```
///
/// where `<sticky>` is `0` or `1`, the digits are truncated rather than
/// rounded, and the exponent is written in decimal with a leading `-` when
/// negative and no `+` when positive. `1e-7` comes back as
/// `0100000000000000000e-7`.
pub(crate) const FLOAT_TO_STRING_SCI_SYMBOL: &str = "_mfb_rt_float_to_string_sci";

const FRAC_DIGIT_SYMBOL: &str = "_mfb_rt_sci_frac_digit";
const FRAC_NONZERO_SYMBOL: &str = "_mfb_rt_sci_frac_nonzero";

/// Significant digits produced. 18 is one more than the 17 the search can ask
/// for, which is exactly what rounding at `p = 17` needs.
const SIG_DIGITS: usize = 18;

// Stack layout. The limb area and the integer-digit buffer mirror the fixed
// formatter's sizes for the same reasons (34 working limbs for `m << e2`; 384
// bytes is >= 310 integer digits).
const LIMBS_OFF: usize = 0;
const LIMB_SLOTS: usize = 35;
const INTDIG_OFF: usize = LIMB_SLOTS * 8; // 280
const INTDIG_END: usize = INTDIG_OFF + 384; // 664
const SIG_OFF: usize = INTDIG_END; // 664: the 18 digits, as values 0..9
const OUT_OFF: usize = SIG_OFF + 24; // 688: the assembled text
const EXP_OFF: usize = OUT_OFF + 32; // 720: exponent digits, written backward
const EXP_END: usize = EXP_OFF + 8; // 728
const LOCAL_SIZE: usize = EXP_END; // 728
const MASK32: &str = "4294967295";

pub(crate) fn lower_float_to_string_sci_helpers() -> Vec<CodeFunction> {
    vec![lower_frac_digit(), lower_frac_nonzero(), lower_sci()]
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

fn internal_call(
    from: &str,
    to: &str,
    ins: &mut Vec<CodeInstruction>,
    relocs: &mut Vec<CodeRelocation>,
) {
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
// `_mfb_rt_sci_frac_digit` — one decimal digit out of the fraction limbs.
//
// `x0` = limb array, `x1` = limb count. Multiplies the array by ten in place;
// the carry out of the top limb is the next digit. Exactly the pass the fixed
// formatter runs inline, factored out because this helper needs it from four
// places and a call per digit is nothing against the rest of the work.
// ---------------------------------------------------------------------------
fn lower_frac_digit() -> CodeFunction {
    let symbol = FRAC_DIGIT_SYMBOL;
    let l = |suffix: &str| format!("{symbol}_{suffix}");
    let mut vregs = Vregs::new();
    let mut ins = vec![abi::label("entry")];

    let base = vregs.next();
    let count = vregs.next();
    let addr = vregs.next();
    let stop = vregs.next();
    let limb = vregs.next();
    let carry = vregs.next();
    let mask = vregs.next();
    let ten = vregs.next();

    let loop_top = l("loop");
    let loop_done = l("done");
    let empty = l("empty");

    ins.extend([
        abi::move_register(&base, abi::c_arg(0)),
        abi::move_register(&count, abi::c_arg(1)),
        abi::move_immediate(&carry, "Integer", "0"),
        abi::compare_immediate(&count, "0"),
        abi::branch_eq(&empty),
        abi::move_register(&addr, &base),
        abi::shift_left_immediate(&stop, &count, 3),
        abi::add_registers(&stop, &base, &stop),
        abi::move_immediate(&mask, "Integer", MASK32),
        abi::move_immediate(&ten, "Integer", "10"),
        abi::label(&loop_top),
        abi::compare_registers(&addr, &stop),
        abi::branch_ge(&loop_done),
        abi::load_u64(&limb, &addr, 0),
        abi::multiply_registers(&limb, &limb, &ten),
        abi::add_registers(&limb, &limb, &carry),
        abi::shift_right_immediate(&carry, &limb, 32),
        abi::and_registers(&limb, &limb, &mask),
        abi::store_u64(&limb, &addr, 0),
        abi::add_immediate(&addr, &addr, 8),
        abi::branch(&loop_top),
        abi::label(&loop_done),
        abi::label(&empty),
        abi::move_register(RESULT_TAG_REGISTER, &carry),
        abi::return_(),
    ]);

    function("runtime.sciFracDigit", symbol, ins, Vec::new(), 0)
}

// ---------------------------------------------------------------------------
// `_mfb_rt_sci_frac_nonzero` — is anything left in the fraction?
//
// `x0` = limb array, `x1` = limb count. Returns `x0` = 1 when any limb is
// non-zero. This is the sticky bit: whether the digits already taken are the
// whole value or only a prefix of it.
// ---------------------------------------------------------------------------
fn lower_frac_nonzero() -> CodeFunction {
    let symbol = FRAC_NONZERO_SYMBOL;
    let l = |suffix: &str| format!("{symbol}_{suffix}");
    let mut vregs = Vregs::new();
    let mut ins = vec![abi::label("entry")];

    let base = vregs.next();
    let count = vregs.next();
    let addr = vregs.next();
    let stop = vregs.next();
    let limb = vregs.next();

    let loop_top = l("loop");
    let zero = l("zero");
    let nonzero = l("nonzero");

    ins.extend([
        abi::move_register(&base, abi::c_arg(0)),
        abi::move_register(&count, abi::c_arg(1)),
        abi::compare_immediate(&count, "0"),
        abi::branch_eq(&zero),
        abi::move_register(&addr, &base),
        abi::shift_left_immediate(&stop, &count, 3),
        abi::add_registers(&stop, &base, &stop),
        abi::label(&loop_top),
        abi::compare_registers(&addr, &stop),
        abi::branch_ge(&zero),
        abi::load_u64(&limb, &addr, 0),
        abi::compare_immediate(&limb, "0"),
        abi::branch_ne(&nonzero),
        abi::add_immediate(&addr, &addr, 8),
        abi::branch(&loop_top),
        abi::label(&zero),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", "0"),
        abi::return_(),
        abi::label(&nonzero),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", "1"),
        abi::return_(),
    ]);

    function("runtime.sciFracNonzero", symbol, ins, Vec::new(), 0)
}

// ---------------------------------------------------------------------------
// `_mfb_rt_float_to_string_sci` — the entry point.
// ---------------------------------------------------------------------------
fn lower_sci() -> CodeFunction {
    let symbol = FLOAT_TO_STRING_SCI_SYMBOL;
    let l = |suffix: &str| format!("{symbol}_{suffix}");
    let mut vregs = Vregs::new();
    let mut relocs: Vec<CodeRelocation> = Vec::new();
    let mut ins = vec![abi::label("entry")];

    let bits = vregs.next();
    let m = vregs.next();
    let e2 = vregs.next();
    let tmp = vregs.next();
    let mask = vregs.next();
    let ten = vregs.next();
    let ip = vregs.next();
    let int_end = vregs.next();
    let limbs = vregs.next();
    let n_limbs = vregs.next();
    let sticky = vregs.next();
    let exponent = vregs.next();
    let digits = vregs.next();
    let index = vregs.next();
    let count = vregs.next();
    let addr = vregs.next();
    let byte = vregs.next();
    let k = vregs.next();
    let int_part = vregs.next();

    let normal = l("normal");
    let decomposed = l("decomposed");
    let bigint = l("bigint");
    let have_digits = l("have_digits");
    let tiny = l("tiny");

    // --- decompose (magnitude only) ---------------------------------------
    ins.extend([
        abi::move_register(&bits, abi::c_arg(0)),
        abi::move_immediate(&mask, "Integer", "9223372036854775807"),
        abi::and_registers(&tmp, &bits, &mask),
        abi::shift_right_immediate(&e2, &tmp, 52),
        abi::move_immediate(&mask, "Integer", F64_MANTISSA_MASK),
        abi::and_registers(&m, &tmp, &mask),
        abi::compare_immediate(&e2, "0"),
        abi::branch_ne(&normal),
        // subnormal: e2 = -1074, mantissa as-is
        abi::move_immediate(&e2, "Integer", "1074"),
        abi::subtract_registers(&e2, abi::ZERO, &e2),
        abi::branch(&decomposed),
        abi::label(&normal),
        abi::move_immediate(&mask, "Integer", "4503599627370496"), // 2^52
        abi::or_registers(&m, &m, &mask),
        abi::subtract_immediate(&e2, &e2, 1075),
        abi::label(&decomposed),
        abi::move_immediate(&ten, "Integer", "10"),
        abi::add_immediate(&int_end, abi::stack_pointer(), INTDIG_END),
        abi::move_register(&ip, &int_end),
        abi::add_immediate(&limbs, abi::stack_pointer(), LIMBS_OFF),
        abi::move_immediate(&n_limbs, "Integer", "0"),
        abi::move_immediate(&sticky, "Integer", "0"),
        abi::add_immediate(&digits, abi::stack_pointer(), SIG_OFF),
        abi::compare_immediate(&e2, "0"),
        abi::branch_ge(&bigint),
    ]);

    // --- e2 < 0: an integer part (maybe zero) and a fraction ---------------
    {
        let s0 = vregs.next();
        let frac = vregs.next();
        let one = vregs.next();
        let q = vregs.next();
        let digit = vregs.next();
        let a = vregs.next();
        let b = vregs.next();
        let r = vregs.next();
        let t = vregs.next();
        let u = vregs.next();
        let stop = vregs.next();
        let int_shift = l("int_shift");
        let int_ready = l("int_ready");
        let no_int = l("no_int");
        let int_loop = l("int_digits");
        let whole = l("frac_whole");
        let masked = l("frac_masked");
        let zero_loop = l("zero_limbs");
        let zero_done = l("zero_limbs_done");
        let no_shift = l("place_noshift");
        let placed = l("place_done");
        ins.extend([
            abi::subtract_registers(&k, abi::ZERO, &e2),
            // integer part = k <= 63 ? m >> k : 0
            abi::compare_immediate(&k, "63"),
            abi::branch_le(&int_shift),
            abi::move_immediate(&int_part, "Integer", "0"),
            abi::branch(&int_ready),
            abi::label(&int_shift),
            abi::shift_right_variable(&int_part, &m, &k),
            abi::label(&int_ready),
            abi::compare_immediate(&int_part, "0"),
            abi::branch_eq(&no_int),
            // digits backward from int_end
            abi::label(&int_loop),
            abi::unsigned_divide_registers(&q, &int_part, &ten),
            abi::multiply_subtract_registers(&digit, &q, &ten, &int_part),
            abi::add_immediate(&digit, &digit, b'0' as usize),
            abi::subtract_immediate(&ip, &ip, 1),
            abi::store_u8(&digit, &ip, 0),
            abi::move_register(&int_part, &q),
            abi::compare_immediate(&int_part, "0"),
            abi::branch_ne(&int_loop),
            abi::label(&no_int),
            // fraction = m & ((1 << k) - 1), or all of m when k > 63
            abi::compare_immediate(&k, "63"),
            abi::branch_hi(&whole),
            abi::move_immediate(&one, "Integer", "1"),
            abi::shift_left_variable(&mask, &one, &k),
            abi::subtract_immediate(&mask, &mask, 1),
            abi::and_registers(&frac, &m, &mask),
            abi::branch(&masked),
            abi::label(&whole),
            abi::move_register(&frac, &m),
            abi::label(&masked),
            // n = ceil(k/32); s0 = 32n - k
            abi::add_immediate(&n_limbs, &k, 31),
            abi::shift_right_immediate(&n_limbs, &n_limbs, 5),
            abi::shift_left_immediate(&s0, &n_limbs, 5),
            abi::subtract_registers(&s0, &s0, &k),
            // clear the limbs
            abi::move_register(&addr, &limbs),
            abi::shift_left_immediate(&stop, &n_limbs, 3),
            abi::add_registers(&stop, &limbs, &stop),
            abi::label(&zero_loop),
            abi::compare_registers(&addr, &stop),
            abi::branch_ge(&zero_done),
            abi::store_u64(abi::ZERO, &addr, 0),
            abi::add_immediate(&addr, &addr, 8),
            abi::branch(&zero_loop),
            abi::label(&zero_done),
            // place frac << s0 across the low three limbs (the payload cannot
            // span more: frac < 2^53 and s0 < 32)
            abi::move_immediate(&mask, "Integer", MASK32),
            abi::and_registers(&a, &frac, &mask),
            abi::shift_right_immediate(&b, &frac, 32),
            abi::compare_immediate(&s0, "0"),
            abi::branch_eq(&no_shift),
            abi::shift_left_variable(&t, &a, &s0),
            abi::and_registers(&u, &t, &mask),
            abi::store_u64(&u, &limbs, 0),
            abi::move_immediate(&r, "Integer", "32"),
            abi::subtract_registers(&r, &r, &s0),
            abi::shift_right_variable(&t, &a, &r),
            abi::shift_left_variable(&u, &b, &s0),
            abi::or_registers(&t, &t, &u),
            abi::and_registers(&t, &t, &mask),
            abi::store_u64(&t, &limbs, 8),
            abi::shift_right_variable(&t, &b, &r),
            abi::store_u64(&t, &limbs, 16),
            abi::branch(&placed),
            abi::label(&no_shift),
            abi::store_u64(&a, &limbs, 0),
            abi::store_u64(&b, &limbs, 8),
            abi::label(&placed),
            abi::branch(&have_digits),
        ]);
    }

    // --- e2 >= 0: a whole number, digits from the limb array ---------------
    {
        let w = vregs.next();
        let s = vregs.next();
        let a = vregs.next();
        let b = vregs.next();
        let carry = vregs.next();
        let limb = vregs.next();
        let q = vregs.next();
        let rem = vregs.next();
        let nonzero = vregs.next();
        let idx = vregs.next();
        let digit = vregs.next();
        let stop = vregs.next();
        let zero_loop = l("big_zero");
        let zero_done = l("big_zero_done");
        let no_shift = l("big_noshift");
        let shift_loop = l("big_shift");
        let shift_done = l("big_shift_done");
        let outer = l("big_outer");
        let inner = l("big_inner");
        let inner_done = l("big_inner_done");
        ins.extend([
            abi::label(&bigint),
            abi::move_register(&addr, &limbs),
            abi::add_immediate(&stop, &limbs, 34 * 8),
            abi::label(&zero_loop),
            abi::compare_registers(&addr, &stop),
            abi::branch_ge(&zero_done),
            abi::store_u64(abi::ZERO, &addr, 0),
            abi::add_immediate(&addr, &addr, 8),
            abi::branch(&zero_loop),
            abi::label(&zero_done),
            // V = m << e2
            abi::shift_right_immediate(&w, &e2, 5),
            abi::move_immediate(&mask, "Integer", "31"),
            abi::and_registers(&s, &e2, &mask),
            abi::move_immediate(&mask, "Integer", MASK32),
            abi::and_registers(&a, &m, &mask),
            abi::shift_right_immediate(&b, &m, 32),
            abi::shift_left_immediate(&tmp, &w, 3),
            abi::add_registers(&addr, &limbs, &tmp),
            abi::store_u64(&a, &addr, 0),
            abi::store_u64(&b, &addr, 8),
            abi::compare_immediate(&s, "0"),
            abi::branch_eq(&no_shift),
            abi::move_immediate(&carry, "Integer", "0"),
            abi::add_immediate(&stop, &addr, 24),
            abi::label(&shift_loop),
            abi::compare_registers(&addr, &stop),
            abi::branch_ge(&shift_done),
            abi::load_u64(&limb, &addr, 0),
            abi::shift_left_variable(&limb, &limb, &s),
            abi::or_registers(&limb, &limb, &carry),
            abi::shift_right_immediate(&carry, &limb, 32),
            abi::move_immediate(&mask, "Integer", MASK32),
            abi::and_registers(&limb, &limb, &mask),
            abi::store_u64(&limb, &addr, 0),
            abi::add_immediate(&addr, &addr, 8),
            abi::branch(&shift_loop),
            abi::label(&shift_done),
            abi::label(&no_shift),
            // digits: divmod the array by ten until it is zero
            abi::label(&outer),
            abi::move_immediate(&rem, "Integer", "0"),
            abi::move_immediate(&nonzero, "Integer", "0"),
            abi::move_immediate(&idx, "Integer", &((34 - 1) * 8).to_string()),
            abi::label(&inner),
            abi::compare_immediate(&idx, "0"),
            abi::branch_lt(&inner_done),
            abi::add_registers(&addr, &limbs, &idx),
            abi::load_u64(&limb, &addr, 0),
            abi::shift_left_immediate(&tmp, &rem, 32),
            abi::or_registers(&limb, &limb, &tmp),
            abi::unsigned_divide_registers(&q, &limb, &ten),
            abi::multiply_subtract_registers(&rem, &q, &ten, &limb),
            abi::store_u64(&q, &addr, 0),
            abi::or_registers(&nonzero, &nonzero, &q),
            abi::subtract_immediate(&idx, &idx, 8),
            abi::branch(&inner),
            abi::label(&inner_done),
            abi::add_immediate(&digit, &rem, b'0' as usize),
            abi::subtract_immediate(&ip, &ip, 1),
            abi::store_u8(&digit, &ip, 0),
            abi::compare_immediate(&nonzero, "0"),
            abi::branch_ne(&outer),
            abi::move_immediate(&n_limbs, "Integer", "0"),
        ]);
    }

    // --- take 18 significant digits ---------------------------------------
    {
        let available = vregs.next();
        let take = vregs.next();
        let digit = vregs.next();
        let src = vregs.next();
        let stop = vregs.next();
        let zeros = vregs.next();
        let have_int = l("have_int");
        let copy_loop = l("copy_int");
        let copy_done = l("copy_int_done");
        let tail_loop = l("int_tail");
        let tail_done = l("int_tail_done");
        let tail_sticky = l("int_tail_sticky");
        let fill_loop = l("fill_frac");
        let fill_done = l("fill_frac_done");
        let take_all = l("take_all");
        let skip_loop = l("skip_zeros");
        let skip_done = l("skip_zeros_done");
        let tiny_fill = l("tiny_fill");
        let tiny_fill_done = l("tiny_fill_done");
        let digits_done = l("digits_done");
        ins.extend([
            abi::label(&have_digits),
            abi::subtract_registers(&available, &int_end, &ip),
            abi::compare_immediate(&available, "0"),
            abi::branch_eq(&tiny),
            abi::label(&have_int),
            // exponent = available - 1
            abi::subtract_immediate(&exponent, &available, 1),
            // take = min(available, 18)
            abi::compare_immediate(&available, &SIG_DIGITS.to_string()),
            abi::branch_le(&take_all),
            abi::move_immediate(&take, "Integer", &SIG_DIGITS.to_string()),
            abi::branch(&copy_loop),
            abi::label(&take_all),
            abi::move_register(&take, &available),
            abi::label(&copy_loop),
            abi::move_immediate(&index, "Integer", "0"),
            abi::label(&l("copy_step")),
            abi::compare_registers(&index, &take),
            abi::branch_ge(&copy_done),
            abi::add_registers(&src, &ip, &index),
            abi::load_u8(&digit, &src, 0),
            abi::add_registers(&addr, &digits, &index),
            abi::store_u8(&digit, &addr, 0),
            abi::add_immediate(&index, &index, 1),
            abi::branch(&l("copy_step")),
            abi::label(&copy_done),
            // Either the integer part ran past 18 digits (its tail feeds
            // sticky) or it fell short (the fraction fills the rest).
            abi::compare_registers(&available, &take),
            abi::branch_eq(&fill_loop),
            abi::add_registers(&src, &ip, &take),
            abi::move_register(&stop, &int_end),
            abi::label(&tail_loop),
            abi::compare_registers(&src, &stop),
            abi::branch_ge(&tail_done),
            abi::load_u8(&digit, &src, 0),
            abi::compare_immediate(&digit, &(b'0' as u64).to_string()),
            abi::branch_ne(&tail_sticky),
            abi::add_immediate(&src, &src, 1),
            abi::branch(&tail_loop),
            abi::label(&tail_sticky),
            abi::move_immediate(&sticky, "Integer", "1"),
            abi::label(&tail_done),
            abi::branch(&digits_done),
            abi::label(&fill_loop),
            abi::move_register(&index, &take),
            abi::label(&l("fill_step")),
            abi::compare_immediate(&index, &SIG_DIGITS.to_string()),
            abi::branch_ge(&fill_done),
        ]);
        ins.extend([
            abi::move_register(abi::c_arg(0), &limbs),
            abi::move_register(abi::c_arg(1), &n_limbs),
        ]);
        internal_call(symbol, FRAC_DIGIT_SYMBOL, &mut ins, &mut relocs);
        ins.extend([
            abi::add_immediate(&digit, RESULT_TAG_REGISTER, b'0' as usize),
            abi::add_registers(&addr, &digits, &index),
            abi::store_u8(&digit, &addr, 0),
            abi::add_immediate(&index, &index, 1),
            abi::branch(&l("fill_step")),
            abi::label(&fill_done),
            abi::branch(&digits_done),
            // --- no integer part: step over the fraction's leading zeros ---
            abi::label(&tiny),
            abi::move_immediate(&zeros, "Integer", "0"),
            abi::label(&skip_loop),
        ]);
        ins.extend([
            abi::move_register(abi::c_arg(0), &limbs),
            abi::move_register(abi::c_arg(1), &n_limbs),
        ]);
        internal_call(symbol, FRAC_DIGIT_SYMBOL, &mut ins, &mut relocs);
        ins.extend([
            abi::compare_immediate(RESULT_TAG_REGISTER, "0"),
            abi::branch_ne(&skip_done),
            abi::add_immediate(&zeros, &zeros, 1),
            abi::branch(&skip_loop),
            abi::label(&skip_done),
            // exponent = -(zeros + 1); the first non-zero digit is in hand
            abi::add_immediate(&exponent, &zeros, 1),
            abi::subtract_registers(&exponent, abi::ZERO, &exponent),
            abi::add_immediate(&digit, RESULT_TAG_REGISTER, b'0' as usize),
            abi::store_u8(&digit, &digits, 0),
            abi::move_immediate(&index, "Integer", "1"),
            abi::label(&tiny_fill),
            abi::compare_immediate(&index, &SIG_DIGITS.to_string()),
            abi::branch_ge(&tiny_fill_done),
        ]);
        ins.extend([
            abi::move_register(abi::c_arg(0), &limbs),
            abi::move_register(abi::c_arg(1), &n_limbs),
        ]);
        internal_call(symbol, FRAC_DIGIT_SYMBOL, &mut ins, &mut relocs);
        ins.extend([
            abi::add_immediate(&digit, RESULT_TAG_REGISTER, b'0' as usize),
            abi::add_registers(&addr, &digits, &index),
            abi::store_u8(&digit, &addr, 0),
            abi::add_immediate(&index, &index, 1),
            abi::branch(&tiny_fill),
            abi::label(&tiny_fill_done),
            abi::label(&digits_done),
        ]);
        let _ = count;
    }

    // --- sticky from whatever is left of the fraction ---------------------
    {
        let no_more = l("no_more");
        ins.extend([
            abi::move_register(abi::c_arg(0), &limbs),
            abi::move_register(abi::c_arg(1), &n_limbs),
        ]);
        internal_call(symbol, FRAC_NONZERO_SYMBOL, &mut ins, &mut relocs);
        ins.extend([
            abi::compare_immediate(RESULT_TAG_REGISTER, "0"),
            abi::branch_eq(&no_more),
            abi::move_immediate(&sticky, "Integer", "1"),
            abi::label(&no_more),
        ]);
    }

    // --- assemble `<sticky><18 digits>e<exponent>` -------------------------
    {
        let out = vregs.next();
        let start = vregs.next();
        let exp_ptr = vregs.next();
        let exp_end = vregs.next();
        let q = vregs.next();
        let digit = vregs.next();
        let total = vregs.next();
        let string = vregs.next();
        let dst = vregs.next();
        let src = vregs.next();
        let positive = l("exp_positive");
        let exp_loop = l("exp_digits");
        let copy_loop = l("out_copy");
        let copy_done = l("out_copy_done");
        let exp_copy = l("exp_copy");
        let exp_copy_done = l("exp_copy_done");
        let alloc_ok = l("alloc_ok");
        let alloc_error = l("alloc_error");
        let done = l("done");
        ins.extend([
            abi::add_immediate(&out, abi::stack_pointer(), OUT_OFF),
            abi::move_register(&start, &out),
            abi::add_immediate(&byte, &sticky, b'0' as usize),
            abi::store_u8(&byte, &out, 0),
            abi::add_immediate(&out, &out, 1),
            abi::move_immediate(&index, "Integer", "0"),
            abi::label(&copy_loop),
            abi::compare_immediate(&index, &SIG_DIGITS.to_string()),
            abi::branch_ge(&copy_done),
            abi::add_registers(&addr, &digits, &index),
            abi::load_u8(&byte, &addr, 0),
            abi::store_u8(&byte, &out, 0),
            abi::add_immediate(&out, &out, 1),
            abi::add_immediate(&index, &index, 1),
            abi::branch(&copy_loop),
            abi::label(&copy_done),
            abi::move_immediate(&byte, "Integer", &(b'e' as u64).to_string()),
            abi::store_u8(&byte, &out, 0),
            abi::add_immediate(&out, &out, 1),
            abi::compare_immediate(&exponent, "0"),
            abi::branch_ge(&positive),
            abi::move_immediate(&byte, "Integer", &(b'-' as u64).to_string()),
            abi::store_u8(&byte, &out, 0),
            abi::add_immediate(&out, &out, 1),
            abi::subtract_registers(&exponent, abi::ZERO, &exponent),
            abi::label(&positive),
            // exponent digits, backward into the scratch then forward into out
            abi::add_immediate(&exp_end, abi::stack_pointer(), EXP_END),
            abi::move_register(&exp_ptr, &exp_end),
            abi::label(&exp_loop),
            abi::unsigned_divide_registers(&q, &exponent, &ten),
            abi::multiply_subtract_registers(&digit, &q, &ten, &exponent),
            abi::add_immediate(&digit, &digit, b'0' as usize),
            abi::subtract_immediate(&exp_ptr, &exp_ptr, 1),
            abi::store_u8(&digit, &exp_ptr, 0),
            abi::move_register(&exponent, &q),
            abi::compare_immediate(&exponent, "0"),
            abi::branch_ne(&exp_loop),
            abi::move_register(&src, &exp_ptr),
            abi::label(&exp_copy),
            abi::compare_registers(&src, &exp_end),
            abi::branch_ge(&exp_copy_done),
            abi::load_u8(&byte, &src, 0),
            abi::store_u8(&byte, &out, 0),
            abi::add_immediate(&src, &src, 1),
            abi::add_immediate(&out, &out, 1),
            abi::branch(&exp_copy),
            abi::label(&exp_copy_done),
            // allocate and copy out
            abi::subtract_registers(&total, &out, &start),
            abi::add_immediate(abi::return_register(), &total, 9),
            abi::move_immediate(abi::c_arg(1), "Integer", "8"),
            abi::branch_link(ARENA_ALLOC_SYMBOL),
        ]);
        relocs.push(CodeRelocation {
            from: symbol.to_string(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
        ins.extend([
            abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
            abi::branch_ne(&alloc_error),
            abi::label(&alloc_ok),
            abi::move_register(&string, abi::mfb_return(1)),
            abi::store_u64(&total, &string, 0),
            abi::add_immediate(&dst, &string, 8),
            abi::add_immediate(&src, abi::stack_pointer(), OUT_OFF),
            abi::label(&l("final_copy")),
            abi::compare_registers(&src, &out),
            abi::branch_ge(&l("final_copy_done")),
            abi::load_u8(&byte, &src, 0),
            abi::store_u8(&byte, &dst, 0),
            abi::add_immediate(&src, &src, 1),
            abi::add_immediate(&dst, &dst, 1),
            abi::branch(&l("final_copy")),
            abi::label(&l("final_copy_done")),
            abi::store_u8(abi::ZERO, &dst, 0),
            abi::move_register(RESULT_VALUE_REGISTER, &string),
            abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
            abi::branch(&done),
            abi::label(&alloc_error),
        ]);
        raise_error_into(symbol, "ErrOutOfMemory", &mut ins, &mut relocs);
        ins.extend([abi::label(&done), abi::return_()]);
    }

    function("runtime.floatToStringSci", symbol, ins, relocs, LOCAL_SIZE)
}
