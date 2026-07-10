//! The in-tree `toString(Float[, precision])` formatter — the exact fixed-point
//! rendering `snprintf("%.*f", p, v)` produced, computed with integer/limb
//! arithmetic so no libc math or formatting is imported anywhere. The value is
//! guaranteed finite (the `observe_float` boundary traps non-finite results
//! before any Float reaches text), so there is no inf/NaN path.
//!
//! Method (the classical exact fixed-format algorithm):
//! a finite f64 is `± m·2^e2` with `m < 2^53`.
//! - `e2 >= 0`: the value is an integer `V = m·2^e2` (≤ 2^1024). V is held in
//!   base-2^32 limbs and converted to decimal by repeated divmod-10 passes; the
//!   fraction is `p` zeros — no rounding.
//! - `e2 < 0` (k = −e2 ≤ 1074): the integer part `m >> k` fits an i64 (m < 2^53);
//!   the k-bit fraction F = m mod 2^k is normalized to n = ceil(k/32) limbs
//!   pre-shifted so the radix point sits at the limb-array top (value =
//!   limbs/2^(32n); the payload spans ≤ 3 limbs since the pre-shift is < 32).
//!   Each decimal digit is the carry-out of one ×10 pass over the limbs. After
//!   `p` digits the remainder decides rounding: top bit clear → down; set with
//!   any lower bit → up; exact half → ties-to-even on the last emitted digit
//!   (the rounding every correct printf produces). A round-up ripples through
//!   the ASCII digits and can grow the integer part ("9.99" → "10.0").
//!
//! Output shape (identical to `%.*f`): optional '-', at least one integer
//! digit, then `.` + exactly `p` digits when `p > 0`. `-0.0` renders with the
//! sign ("-0.00"). Maximum length 1+309+1+255 = 566 < the 640-byte digit
//! buffer, so no length guard is needed.

use super::*;

/// Internal symbol: `x0` = f64 bit pattern (finite), `x1` = precision 0..=255.
/// Returns the standard allocation Result: `x0` = tag, `x1` = String pointer
/// (an arena string `{len, bytes..., NUL}`), or the out-of-memory error Result.
pub(super) const FLOAT_TO_STRING_SYMBOL: &str = "_mfb_rt_float_to_string";

// Stack locals: 35 limb slots (one 32-bit limb per 8-byte slot, little-endian
// significance) then the 640-byte digit buffer. Integer digits are written
// backward ending at DIGITS_INT_END; fraction digits forward from the same
// boundary, so the final string is a contiguous read with '.' inserted at copy
// time. Integer capacity 384 bytes (≥ 310 digits + round-up growth); fraction
// capacity 256 bytes (≥ 255 digits).
const LIMBS_OFF: usize = 0;
const LIMB_SLOTS: usize = 35;
const DIGITS_OFF: usize = LIMB_SLOTS * 8; // 280
const DIGITS_INT_END: usize = DIGITS_OFF + 384; // 664: int in [280,664), 384 ≥ 311
const LOCAL_SIZE: usize = DIGITS_OFF + 640; // 920 (frac needs 664+255 = 919)
const MASK32: &str = "4294967295";

/// Lower the float formatter helper (vreg-allocated; emitted iff referenced).
pub(super) fn lower_float_to_string_helper() -> CodeFunction {
    let symbol = FLOAT_TO_STRING_SYMBOL;
    let l = |suffix: &str| format!("{symbol}_{suffix}");

    let mut vregs = Vregs::new();
    let mut ins: Vec<CodeInstruction> = vec![abi::label("entry")];
    let mut relocations: Vec<CodeRelocation> = Vec::new();

    // --- Decompose ---------------------------------------------------------
    let bits = vregs.next();
    let prec = vregs.next();
    let sign = vregs.next();
    let m = vregs.next();
    let e2 = vregs.next();
    let tmp = vregs.next();
    let mask = vregs.next();
    ins.extend([
        abi::move_register(&bits, abi::ARG[0]),
        abi::move_register(&prec, abi::ARG[1]),
        abi::shift_right_immediate(&sign, &bits, 63),
        abi::move_immediate(&mask, "Integer", "9223372036854775807"),
        abi::and_registers(&tmp, &bits, &mask), // magnitude bits
        abi::shift_right_immediate(&e2, &tmp, 52), // biased exponent (reuse e2)
        abi::move_immediate(&mask, "Integer", "4503599627370495"), // 2^52-1
        abi::and_registers(&m, &tmp, &mask),
    ]);
    let normal = l("normal");
    let decomposed = l("decomposed");
    ins.extend([
        abi::compare_immediate(&e2, "0"),
        abi::branch_ne(&normal),
        // subnormal (or zero): m = mantissa, e2 = -1074 (no negative immediates
        // in the encoder — build it as 0 - 1074)
        abi::subtract_registers(&e2, abi::ZERO, &e2), // e2 == 0 here
        abi::subtract_immediate(&e2, &e2, 1074),
        abi::branch(&decomposed),
        abi::label(&normal),
        abi::move_immediate(&mask, "Integer", "4503599627370496"), // 2^52
        abi::or_registers(&m, &m, &mask),
        abi::subtract_immediate(&e2, &e2, 1075),
        abi::label(&decomposed),
    ]);

    // Digit-buffer cursors (addresses, so the copy-out is uniform).
    let int_end = vregs.next(); // fixed: one past the last integer digit
    let ip = vregs.next(); // walks backward; final value = first integer digit
    ins.extend([
        abi::add_immediate(&int_end, abi::stack_pointer(), DIGITS_INT_END),
        abi::move_register(&ip, &int_end),
    ]);

    let bigint = l("bigint");
    let assemble = l("assemble");
    ins.extend([abi::compare_immediate(&e2, "0"), abi::branch_ge(&bigint)]);

    // ======================= e2 < 0: fractional path ========================
    let k = vregs.next();
    ins.push(abi::subtract_registers(&k, abi::ZERO, &e2)); // k = 0 - e2 (xzr source)

    // Integer part I = m >> k (k > 63 → 0; m < 2^53 so k in 54..=63 also gives 0).
    let int_part = vregs.next();
    let small_shift = l("int_shift");
    let int_ready = l("int_ready");
    ins.extend([
        abi::compare_immediate(&k, "63"),
        abi::branch_le(&small_shift),
        abi::move_immediate(&int_part, "Integer", "0"),
        abi::branch(&int_ready),
        abi::label(&small_shift),
        abi::shift_right_variable(&int_part, &m, &k),
        abi::label(&int_ready),
    ]);

    // Integer digits, backward from int_end (at least one digit).
    {
        let digit = vregs.next();
        let q = vregs.next();
        let ten = vregs.next();
        let int_loop = l("int_digits");
        ins.push(abi::move_immediate(&ten, "Integer", "10"));
        ins.extend([
            abi::label(&int_loop),
            abi::unsigned_divide_registers(&q, &int_part, &ten),
            abi::multiply_subtract_registers(&digit, &q, &ten, &int_part),
            abi::add_immediate(&digit, &digit, b'0' as usize),
            abi::subtract_immediate(&ip, &ip, 1),
            abi::store_u8(&digit, &ip, 0),
            abi::move_register(&int_part, &q),
            abi::compare_immediate(&int_part, "0"),
            abi::branch_ne(&int_loop),
        ]);
    }

    // F = m mod 2^k (k > 63 → all of m), normalized into n = ceil(k/32) limbs
    // pre-shifted left by s0 = 32n - k, so each ×10 carry-out is one digit.
    let frac = vregs.next();
    let n_limbs = vregs.next();
    let s0 = vregs.next();
    {
        let whole = l("frac_whole");
        let masked = l("frac_masked");
        let one = vregs.next();
        ins.extend([
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
            // n = (k + 31) >> 5 ; s0 = (n << 5) - k
            abi::add_immediate(&n_limbs, &k, 31),
            abi::shift_right_immediate(&n_limbs, &n_limbs, 5),
            abi::shift_left_immediate(&s0, &n_limbs, 5),
            abi::subtract_registers(&s0, &s0, &k),
        ]);
    }

    // Zero limbs [0..n) (they may hold stale bytes from a previous frame use).
    {
        let addr = vregs.next();
        let stop = vregs.next();
        let zero_loop = l("zero_limbs");
        let zero_done = l("zero_done");
        ins.extend([
            abi::add_immediate(&addr, abi::stack_pointer(), LIMBS_OFF),
            abi::shift_left_immediate(&stop, &n_limbs, 3),
            abi::add_registers(&stop, &addr, &stop),
            abi::label(&zero_loop),
            abi::compare_registers(&addr, &stop),
            abi::branch_ge(&zero_done),
            abi::store_u64(abi::ZERO, &addr, 0),
            abi::add_immediate(&addr, &addr, 8),
            abi::branch(&zero_loop),
            abi::label(&zero_done),
        ]);
    }

    // Place F << s0 into limbs 0..2 (the payload spans ≤ 3 limbs: F < 2^53,
    // s0 < 32). s0 == 0 avoids the 32-s0 shift hazard.
    {
        let a = vregs.next();
        let b = vregs.next();
        let r = vregs.next();
        let t = vregs.next();
        let u = vregs.next();
        let base = vregs.next();
        let no_shift = l("place_noshift");
        let placed = l("place_done");
        ins.extend([
            abi::move_immediate(&mask, "Integer", MASK32),
            abi::and_registers(&a, &frac, &mask),
            abi::shift_right_immediate(&b, &frac, 32),
            abi::add_immediate(&base, abi::stack_pointer(), LIMBS_OFF),
            abi::compare_immediate(&s0, "0"),
            abi::branch_eq(&no_shift),
            // limb0 = (a << s0) & M32
            abi::shift_left_variable(&t, &a, &s0),
            abi::and_registers(&u, &t, &mask),
            abi::store_u64(&u, &base, 0),
            // limb1 = ((a >> (32-s0)) | (b << s0)) & M32
            abi::move_immediate(&r, "Integer", "32"),
            abi::subtract_registers(&r, &r, &s0),
            abi::shift_right_variable(&t, &a, &r),
            abi::shift_left_variable(&u, &b, &s0),
            abi::or_registers(&t, &t, &u),
            abi::and_registers(&t, &t, &mask),
            abi::store_u64(&t, &base, 8),
            // limb2 = b >> (32-s0)
            abi::shift_right_variable(&t, &b, &r),
            abi::store_u64(&t, &base, 16),
            abi::branch(&placed),
            abi::label(&no_shift),
            abi::store_u64(&a, &base, 0),
            abi::store_u64(&b, &base, 8),
            abi::label(&placed),
        ]);
    }

    // Fraction digits: p × (limbs ×= 10; digit = carry-out).
    {
        let j = vregs.next();
        let carry = vregs.next();
        let addr = vregs.next();
        let stop = vregs.next();
        let limb = vregs.next();
        let ten = vregs.next();
        let out = vregs.next();
        let digit_loop = l("frac_digit");
        let limb_loop = l("frac_limb");
        let limb_done = l("frac_limb_done");
        let frac_done = l("frac_digits_done");
        ins.extend([
            abi::move_immediate(&j, "Integer", "0"),
            abi::move_immediate(&ten, "Integer", "10"),
            abi::add_immediate(&out, abi::stack_pointer(), DIGITS_INT_END),
            abi::label(&digit_loop),
            abi::compare_registers(&j, &prec),
            abi::branch_ge(&frac_done),
            abi::move_immediate(&carry, "Integer", "0"),
            abi::add_immediate(&addr, abi::stack_pointer(), LIMBS_OFF),
            abi::shift_left_immediate(&stop, &n_limbs, 3),
            abi::add_registers(&stop, &addr, &stop),
            abi::label(&limb_loop),
            abi::compare_registers(&addr, &stop),
            abi::branch_ge(&limb_done),
            abi::load_u64(&limb, &addr, 0),
            abi::multiply_registers(&limb, &limb, &ten),
            abi::add_registers(&limb, &limb, &carry),
            abi::shift_right_immediate(&carry, &limb, 32),
            abi::move_immediate(&mask, "Integer", MASK32),
            abi::and_registers(&limb, &limb, &mask),
            abi::store_u64(&limb, &addr, 0),
            abi::add_immediate(&addr, &addr, 8),
            abi::branch(&limb_loop),
            abi::label(&limb_done),
            abi::add_immediate(&carry, &carry, b'0' as usize),
            abi::store_u8(&carry, &out, 0),
            abi::add_immediate(&out, &out, 1),
            abi::add_immediate(&j, &j, 1),
            abi::branch(&digit_loop),
            abi::label(&frac_done),
        ]);
    }

    // Rounding: remainder r = limbs/2^(32n) vs 1/2. Top bit clear → down. Set
    // with any lower bit → up. Exact half → ties-to-even on the last digit
    // (ASCII parity: '0'..'9' have the digit's parity).
    {
        let top = vregs.next();
        let rest = vregs.next();
        let addr = vregs.next();
        let stop = vregs.next();
        let limb = vregs.next();
        let last = vregs.next();
        let ptr = vregs.next();
        let byte = vregs.next();
        let floor = vregs.next();
        let rest_loop = l("round_rest");
        let rest_done = l("round_rest_done");
        let do_round = l("round_up");
        let tie = l("round_tie");
        let frac_carry = l("round_frac");
        let int_carry = l("round_int");
        let int_grow = l("round_grow");
        let last_from_int = l("round_last_int");
        let have_last = l("round_have_last");
        ins.extend([
            // top limb (index n-1) and its high bit
            abi::add_immediate(&addr, abi::stack_pointer(), LIMBS_OFF),
            abi::subtract_immediate(&stop, &n_limbs, 1),
            abi::shift_left_immediate(&stop, &stop, 3),
            abi::add_registers(&stop, &addr, &stop), // &limb[n-1]
            abi::load_u64(&top, &stop, 0),
            abi::shift_right_immediate(&tmp, &top, 31),
            abi::compare_immediate(&tmp, "0"),
            abi::branch_eq(&assemble), // below half: round down
            // rest = (top & 0x7fffffff) | OR(limbs[0..n-1])
            abi::move_immediate(&mask, "Integer", "2147483647"),
            abi::and_registers(&rest, &top, &mask),
            abi::label(&rest_loop),
            abi::compare_registers(&addr, &stop),
            abi::branch_ge(&rest_done),
            abi::load_u64(&limb, &addr, 0),
            abi::or_registers(&rest, &rest, &limb),
            abi::add_immediate(&addr, &addr, 8),
            abi::branch(&rest_loop),
            abi::label(&rest_done),
            abi::compare_immediate(&rest, "0"),
            abi::branch_ne(&do_round),
            // exact half: round up only when the last emitted digit is odd
            abi::label(&tie),
            abi::compare_immediate(&prec, "0"),
            abi::branch_eq(&last_from_int),
            abi::add_immediate(&ptr, abi::stack_pointer(), DIGITS_INT_END),
            abi::add_registers(&ptr, &ptr, &prec),
            abi::subtract_immediate(&ptr, &ptr, 1), // last fraction digit
            abi::branch(&have_last),
            abi::label(&last_from_int),
            abi::subtract_immediate(&ptr, &int_end, 1), // last integer digit
            abi::label(&have_last),
            abi::load_u8(&last, &ptr, 0),
            abi::move_immediate(&mask, "Integer", "1"),
            abi::and_registers(&last, &last, &mask),
            abi::compare_immediate(&last, "0"),
            abi::branch_eq(&assemble), // even digit: round down
            // fall through to round-up
            abi::label(&do_round),
            // ripple through the fraction digits (if any)
            abi::compare_immediate(&prec, "0"),
            abi::branch_eq(&int_carry),
            abi::add_immediate(&ptr, abi::stack_pointer(), DIGITS_INT_END),
            abi::add_registers(&ptr, &ptr, &prec),
            abi::label(&frac_carry),
            abi::subtract_immediate(&ptr, &ptr, 1),
            abi::add_immediate(&floor, abi::stack_pointer(), DIGITS_INT_END),
            abi::compare_registers(&ptr, &floor),
            abi::branch_lt(&int_carry),
            abi::load_u8(&byte, &ptr, 0),
            abi::compare_immediate(&byte, &(b'9' as u64).to_string()),
            abi::branch_ne(&l("round_frac_bump")),
            abi::move_immediate(&byte, "Integer", &(b'0' as u64).to_string()),
            abi::store_u8(&byte, &ptr, 0),
            abi::branch(&frac_carry),
            abi::label(&l("round_frac_bump")),
            abi::add_immediate(&byte, &byte, 1),
            abi::store_u8(&byte, &ptr, 0),
            abi::branch(&assemble),
            // ripple through the integer digits
            abi::label(&int_carry),
            abi::move_register(&ptr, &int_end),
            abi::label(&l("round_int_loop")),
            abi::subtract_immediate(&ptr, &ptr, 1),
            abi::compare_registers(&ptr, &ip),
            abi::branch_lt(&int_grow),
            abi::load_u8(&byte, &ptr, 0),
            abi::compare_immediate(&byte, &(b'9' as u64).to_string()),
            abi::branch_ne(&l("round_int_bump")),
            abi::move_immediate(&byte, "Integer", &(b'0' as u64).to_string()),
            abi::store_u8(&byte, &ptr, 0),
            abi::branch(&l("round_int_loop")),
            abi::label(&l("round_int_bump")),
            abi::add_immediate(&byte, &byte, 1),
            abi::store_u8(&byte, &ptr, 0),
            abi::branch(&assemble),
            // every integer digit was '9': prepend '1'
            abi::label(&int_grow),
            abi::subtract_immediate(&ip, &ip, 1),
            abi::move_immediate(&byte, "Integer", &(b'1' as u64).to_string()),
            abi::store_u8(&byte, &ip, 0),
            abi::branch(&assemble),
        ]);
    }

    // ======================= e2 >= 0: big-integer path ======================
    {
        let addr = vregs.next();
        let stop = vregs.next();
        let a = vregs.next();
        let b = vregs.next();
        let w = vregs.next();
        let s = vregs.next();
        let carry = vregs.next();
        let limb = vregs.next();
        let ten = vregs.next();
        let q = vregs.next();
        let rem = vregs.next();
        let nonzero = vregs.next();
        let idx = vregs.next();
        let digit = vregs.next();
        let zero_loop = l("big_zero");
        let zero_done = l("big_zero_done");
        let no_shift = l("big_noshift");
        let shift_loop = l("big_shift");
        let shift_done = l("big_shift_done");
        let outer = l("big_outer");
        let inner = l("big_inner");
        let inner_done = l("big_inner_done");
        let big_done = l("big_done");
        let zeros_loop = l("big_frac_zeros");
        let zeros_done = l("big_frac_zeros_done");
        ins.extend([
            abi::label(&bigint),
            // zero all 34 working limbs
            abi::add_immediate(&addr, abi::stack_pointer(), LIMBS_OFF),
            abi::add_immediate(&stop, abi::stack_pointer(), LIMBS_OFF + 34 * 8),
            abi::label(&zero_loop),
            abi::compare_registers(&addr, &stop),
            abi::branch_ge(&zero_done),
            abi::store_u64(abi::ZERO, &addr, 0),
            abi::add_immediate(&addr, &addr, 8),
            abi::branch(&zero_loop),
            abi::label(&zero_done),
            // V = m << e2: word offset w = e2>>5, bit shift s = e2&31
            abi::shift_right_immediate(&w, &e2, 5),
            abi::move_immediate(&mask, "Integer", "31"),
            abi::and_registers(&s, &e2, &mask),
            abi::move_immediate(&mask, "Integer", MASK32),
            abi::and_registers(&a, &m, &mask),
            abi::shift_right_immediate(&b, &m, 32),
            abi::add_immediate(&addr, abi::stack_pointer(), LIMBS_OFF),
            abi::shift_left_immediate(&tmp, &w, 3),
            abi::add_registers(&addr, &addr, &tmp), // &limb[w]
            abi::store_u64(&a, &addr, 0),
            abi::store_u64(&b, &addr, 8),
            abi::compare_immediate(&s, "0"),
            abi::branch_eq(&no_shift),
            // three-limb in-place left shift by s with carry
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
            // decimal digits: divmod the whole limb array by 10 until zero
            abi::move_immediate(&ten, "Integer", "10"),
            abi::label(&outer),
            abi::move_immediate(&rem, "Integer", "0"),
            abi::move_immediate(&nonzero, "Integer", "0"),
            abi::move_immediate(&idx, "Integer", &((34 - 1) * 8).to_string()),
            abi::label(&inner),
            abi::compare_immediate(&idx, "0"),
            abi::branch_lt(&inner_done),
            abi::add_immediate(&addr, abi::stack_pointer(), LIMBS_OFF),
            abi::add_registers(&addr, &addr, &idx),
            abi::load_u64(&limb, &addr, 0),
            abi::shift_left_immediate(&rem, &rem, 32),
            abi::or_registers(&limb, &limb, &rem),
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
            abi::label(&big_done),
            // fraction: p zeros
            abi::move_immediate(&idx, "Integer", "0"),
            abi::add_immediate(&addr, abi::stack_pointer(), DIGITS_INT_END),
            abi::move_immediate(&digit, "Integer", &(b'0' as u64).to_string()),
            abi::label(&zeros_loop),
            abi::compare_registers(&idx, &prec),
            abi::branch_ge(&zeros_done),
            abi::store_u8(&digit, &addr, 0),
            abi::add_immediate(&addr, &addr, 1),
            abi::add_immediate(&idx, &idx, 1),
            abi::branch(&zeros_loop),
            abi::label(&zeros_done),
            // fall through to assemble
        ]);
    }

    // ============================ assemble ==================================
    {
        let total = vregs.next();
        let dst = vregs.next();
        let src = vregs.next();
        let byte = vregs.next();
        let string = vregs.next();
        let alloc_ok = l("alloc_ok");
        let alloc_error = l("alloc_error");
        let no_sign = l("no_sign");
        let int_copy = l("copy_int");
        let int_copy_done = l("copy_int_done");
        let no_frac = l("no_frac");
        let frac_copy = l("copy_frac");
        let frac_copy_done = l("copy_frac_done");
        let done = l("done");
        ins.extend([
            abi::label(&assemble),
            // total = (int_end - ip) + sign + (prec ? prec + 1 : 0)
            abi::subtract_registers(&total, &int_end, &ip),
            abi::add_registers(&total, &total, &sign),
            abi::compare_immediate(&prec, "0"),
            abi::branch_eq(&l("len_ready")),
            abi::add_registers(&total, &total, &prec),
            abi::add_immediate(&total, &total, 1),
            abi::label(&l("len_ready")),
            abi::add_immediate(abi::return_register(), &total, 9),
            abi::move_immediate(abi::ARG[1], "Integer", "8"),
            abi::branch_link(ARENA_ALLOC_SYMBOL),
        ]);
        relocations.push(CodeRelocation {
            from: symbol.to_string(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
        ins.extend([
            abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
            abi::branch_eq(&alloc_ok),
            abi::branch(&alloc_error),
            abi::label(&alloc_ok),
            abi::move_register(&string, abi::RET[1]),
            abi::store_u64(&total, &string, 0),
            abi::add_immediate(&dst, &string, 8),
            // sign
            abi::compare_immediate(&sign, "0"),
            abi::branch_eq(&no_sign),
            abi::move_immediate(&byte, "Integer", &(b'-' as u64).to_string()),
            abi::store_u8(&byte, &dst, 0),
            abi::add_immediate(&dst, &dst, 1),
            abi::label(&no_sign),
            // integer digits [ip, int_end)
            abi::move_register(&src, &ip),
            abi::label(&int_copy),
            abi::compare_registers(&src, &int_end),
            abi::branch_ge(&int_copy_done),
            abi::load_u8(&byte, &src, 0),
            abi::store_u8(&byte, &dst, 0),
            abi::add_immediate(&src, &src, 1),
            abi::add_immediate(&dst, &dst, 1),
            abi::branch(&int_copy),
            abi::label(&int_copy_done),
            // '.' + fraction digits
            abi::compare_immediate(&prec, "0"),
            abi::branch_eq(&no_frac),
            abi::move_immediate(&byte, "Integer", &(b'.' as u64).to_string()),
            abi::store_u8(&byte, &dst, 0),
            abi::add_immediate(&dst, &dst, 1),
            abi::add_immediate(&src, abi::stack_pointer(), DIGITS_INT_END),
            abi::add_registers(&tmp, &src, &prec),
            abi::label(&frac_copy),
            abi::compare_registers(&src, &tmp),
            abi::branch_ge(&frac_copy_done),
            abi::load_u8(&byte, &src, 0),
            abi::store_u8(&byte, &dst, 0),
            abi::add_immediate(&src, &src, 1),
            abi::add_immediate(&dst, &dst, 1),
            abi::branch(&frac_copy),
            abi::label(&frac_copy_done),
            abi::label(&no_frac),
            abi::store_u8(abi::ZERO, &dst, 0), // NUL (matches the snprintf-era tail)
            abi::move_register(RESULT_VALUE_REGISTER, &string),
            abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
            abi::branch(&done),
            abi::label(&alloc_error),
            abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
            abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
        ]);
        push_error_message_address(symbol, ERR_ALLOCATION_SYMBOL, &mut ins, &mut relocations);
        ins.extend([abi::label(&done), abi::return_()]);
    }

    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut ins, &[], LOCAL_SIZE);
    CodeFunction {
        name: "runtime.floatToString".to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "String".to_string(),
        frame,
        stack_slots,
        instructions: ins,
        relocations,
    }
}
