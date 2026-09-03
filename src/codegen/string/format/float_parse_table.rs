//! The powers-of-ten table behind `_mfb_rt_string_to_float` (plan-120-F).
//!
//! Eisel–Lemire needs, for every decimal exponent `q` it accepts, the 128 most
//! significant bits of `10^q` viewed as an infinite-precision binary number and
//! normalized so bit 127 is set. That is a static table, so it is built here
//! with exact big-integer arithmetic and emitted as one rodata blob — the same
//! shape as the Unicode runtime tables (`raw_data_object`), and for the same
//! reason: the helper indexes it by a *runtime* decimal exponent, so the values
//! cannot be baked into instructions the way `money`'s CORDIC table is.
//!
//! Entries are 16 bytes, little-endian `lo` then `hi`, indexed by `q - Q_MIN`.
//!
//! Rounding direction matters and is not symmetric:
//!
//! - `q >= 0`: `10^q` terminates in binary. Below 2^128 the window holds it
//!   exactly; above, the low bits are dropped, so the stored value is a
//!   TRUNCATION (an under-estimate).
//! - `q < 0`: `10^q` is a reciprocal and never terminates, so the stored value
//!   is rounded UP (an over-estimate).
//!
//! Either way the stored value differs from the true one by less than one unit
//! in the 128th place, which is exactly the error budget Lemire's proof spends;
//! the algorithm's ambiguity check is what detects the cases where that budget
//! is not enough, and those route to the exact fallback instead.

/// Smallest decimal exponent with a table entry. Below this every finite
/// mantissa underflows to zero, so no entry can change the answer.
pub(crate) const Q_MIN: i32 = -342;
/// Largest decimal exponent with a table entry. Above this every non-zero
/// mantissa overflows to infinity.
pub(crate) const Q_MAX: i32 = 308;
/// Number of 16-byte entries.
pub(crate) const ENTRY_COUNT: usize = (Q_MAX - Q_MIN + 1) as usize;

pub(crate) const POWERS_OF_TEN_SYMBOL: &str = "_mfb_rt_powers_of_ten";

/// A minimal little-endian big natural, enough for `10^342` and one long
/// division. Deliberately local: this runs once at emission time over a few
/// hundred values, so clarity beats speed, and it keeps the table's provenance
/// in one readable place rather than as a pasted array of magic constants.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Big(Vec<u32>);

impl Big {
    fn from_u32(value: u32) -> Self {
        Big(if value == 0 { vec![] } else { vec![value] })
    }

    fn is_zero(&self) -> bool {
        self.0.is_empty()
    }

    fn trim(mut self) -> Self {
        while self.0.last() == Some(&0) {
            self.0.pop();
        }
        self
    }

    fn mul_u32(&self, rhs: u32) -> Self {
        let mut out = Vec::with_capacity(self.0.len() + 1);
        let mut carry: u64 = 0;
        for &limb in &self.0 {
            let product = limb as u64 * rhs as u64 + carry;
            out.push(product as u32);
            carry = product >> 32;
        }
        while carry != 0 {
            out.push(carry as u32);
            carry >>= 32;
        }
        Big(out).trim()
    }

    /// Bit length (0 for zero).
    fn bits(&self) -> usize {
        match self.0.last() {
            None => 0,
            Some(&top) => (self.0.len() - 1) * 32 + (32 - top.leading_zeros() as usize),
        }
    }

    fn bit(&self, index: usize) -> bool {
        let limb = index / 32;
        match self.0.get(limb) {
            None => false,
            Some(&value) => (value >> (index % 32)) & 1 == 1,
        }
    }

    /// `self << shift`.
    fn shl(&self, shift: usize) -> Self {
        if self.is_zero() {
            return self.clone();
        }
        let limb_shift = shift / 32;
        let bit_shift = shift % 32;
        let mut out = vec![0u32; limb_shift];
        let mut carry: u32 = 0;
        for &limb in &self.0 {
            if bit_shift == 0 {
                out.push(limb);
            } else {
                out.push((limb << bit_shift) | carry);
                carry = (limb >> (32 - bit_shift)) as u32;
            }
        }
        if bit_shift != 0 && carry != 0 {
            out.push(carry);
        }
        Big(out).trim()
    }

    /// The 128-bit window starting at `index` (bit `index` becomes bit 0).
    fn window_128(&self, index: usize) -> (u64, u64) {
        let mut lo: u64 = 0;
        let mut hi: u64 = 0;
        for offset in 0..64 {
            if self.bit(index + offset) {
                lo |= 1u64 << offset;
            }
            if self.bit(index + 64 + offset) {
                hi |= 1u64 << offset;
            }
        }
        (hi, lo)
    }

    fn cmp_big(&self, other: &Big) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        if self.0.len() != other.0.len() {
            return self.0.len().cmp(&other.0.len());
        }
        for index in (0..self.0.len()).rev() {
            match self.0[index].cmp(&other.0[index]) {
                Ordering::Equal => continue,
                other => return other,
            }
        }
        Ordering::Equal
    }

    fn sub_assign(&mut self, other: &Big) {
        let mut borrow: i64 = 0;
        for index in 0..self.0.len() {
            let rhs = *other.0.get(index).unwrap_or(&0) as i64;
            let mut diff = self.0[index] as i64 - rhs - borrow;
            if diff < 0 {
                diff += 1i64 << 32;
                borrow = 1;
            } else {
                borrow = 0;
            }
            self.0[index] = diff as u32;
        }
        debug_assert_eq!(borrow, 0, "big subtraction went negative");
        *self = std::mem::replace(self, Big(vec![])).trim();
    }
}

fn pow10(exponent: u32) -> Big {
    let mut value = Big::from_u32(1);
    for _ in 0..exponent {
        value = value.mul_u32(10);
    }
    value
}

/// `ceil(2^shift / divisor)` as a 128-bit pair, for a `divisor` and `shift`
/// chosen so the quotient is exactly 128 bits. Plain schoolbook long division,
/// one bit at a time — a few hundred iterations, run once at emission.
fn ceil_pow2_div(shift: usize, divisor: &Big) -> (u64, u64) {
    let numerator = Big::from_u32(1).shl(shift);
    // Long division: walk the numerator's bits from the top, building the
    // quotient bit by bit.
    let mut remainder = Big::from_u32(0);
    let mut quotient_hi: u64 = 0;
    let mut quotient_lo: u64 = 0;
    let total_bits = numerator.bits();
    for index in (0..total_bits).rev() {
        remainder = remainder.shl(1);
        if numerator.bit(index) {
            if remainder.0.is_empty() {
                remainder = Big::from_u32(1);
            } else {
                remainder.0[0] |= 1;
            }
        }
        if remainder.cmp_big(divisor) != std::cmp::Ordering::Less {
            remainder.sub_assign(divisor);
            // Record quotient bit `index`. The quotient is known to fit 128
            // bits, so `index` is < 128 for every set bit.
            debug_assert!(index < 128, "quotient wider than 128 bits");
            if index < 64 {
                quotient_lo |= 1u64 << index;
            } else {
                quotient_hi |= 1u64 << (index - 64);
            }
        }
    }
    // Round UP: a non-zero remainder means the true value sits above the
    // quotient, and the algorithm's error analysis assumes a reciprocal is an
    // over-estimate.
    if !remainder.is_zero() {
        let (next_lo, carry) = quotient_lo.overflowing_add(1);
        quotient_lo = next_lo;
        if carry {
            quotient_hi = quotient_hi.wrapping_add(1);
        }
    }
    (quotient_hi, quotient_lo)
}

/// The `(hi, lo)` entry for decimal exponent `q`.
pub(crate) fn entry(q: i32) -> (u64, u64) {
    assert!(
        (Q_MIN..=Q_MAX).contains(&q),
        "power-of-ten exponent {q} is outside the table"
    );
    if q >= 0 {
        let value = pow10(q as u32);
        let bits = value.bits();
        if bits <= 128 {
            // Exact: normalize left so bit 127 is set.
            let shifted = value.shl(128 - bits);
            shifted.window_128(0)
        } else {
            // Truncate to the top 128 bits (an under-estimate, as documented).
            value.window_128(bits - 128)
        }
    } else {
        let divisor = pow10((-q) as u32);
        // Choose the shift that makes the quotient exactly 128 bits: for
        // `divisor` in [2^(L-1), 2^L), `2^(L+127)/divisor` lands in (2^127, 2^128].
        // A power of ten above 10^0 is never a power of two, so the open bound
        // is never reached and the quotient always has exactly 128 bits.
        let shift = divisor.bits() + 127;
        ceil_pow2_div(shift, &divisor)
    }
}

/// The whole table as the little-endian hex blob `raw_data_object` wants:
/// `ENTRY_COUNT` entries of `lo` then `hi`, indexed by `q - Q_MIN`.
pub(crate) fn powers_of_ten_hex() -> String {
    let mut bytes = Vec::with_capacity(ENTRY_COUNT * 16);
    for q in Q_MIN..=Q_MAX {
        let (hi, lo) = entry(q);
        bytes.extend_from_slice(&lo.to_le_bytes());
        bytes.extend_from_slice(&hi.to_le_bytes());
    }
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in &bytes {
        output.push_str(&format!("{byte:02x}"));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_entry_is_normalized() {
        // Bit 127 set is what makes the 128-bit product's leading bit
        // predictable, which the whole algorithm rests on.
        for q in Q_MIN..=Q_MAX {
            let (hi, _lo) = entry(q);
            assert!(
                hi >> 63 == 1,
                "10^{q} entry is not normalized: hi = {hi:#018x}"
            );
        }
    }

    #[test]
    fn small_non_negative_powers_are_exact() {
        // 10^q for q <= 38 fits in 128 bits, so the entry must be the exact
        // value left-normalized — no truncation, nothing to round.
        for q in 0..=38u32 {
            let exact = 10u128.pow(q);
            let shift = 128 - (128 - exact.leading_zeros());
            let expected = exact << shift;
            let (hi, lo) = entry(q as i32);
            let got = ((hi as u128) << 64) | lo as u128;
            assert_eq!(got, expected, "10^{q} entry is wrong");
        }
    }

    #[test]
    fn matches_an_independent_computation() {
        // Cross-checked against the same construction evaluated in Python's
        // arbitrary-precision integers, which shares no code with the generator
        // above:
        //
        //   q >= 0: v = 10**q; b = v.bit_length()
        //           w = v << (128-b) if b <= 128 else v >> (b-128)
        //   q <  0: d = 10**(-q); s = d.bit_length() + 127
        //           w = -((-(1 << s)) // d)          # ceil division
        //
        // The ends and the two smallest reciprocals are the informative ones: a
        // systematic error in the shift, the window, or the rounding direction
        // cannot survive all four. `entry(Q_MIN)`'s value is also the published
        // first row of the Eisel–Lemire table, which is a third witness.
        assert_eq!(entry(0), (0x8000_0000_0000_0000, 0x0000_0000_0000_0000));
        assert_eq!(entry(1), (0xa000_0000_0000_0000, 0x0000_0000_0000_0000));
        assert_eq!(entry(-1), (0xcccc_cccc_cccc_cccc, 0xcccc_cccc_cccc_cccd));
        assert_eq!(entry(-2), (0xa3d7_0a3d_70a3_d70a, 0x3d70_a3d7_0a3d_70a4));
        assert_eq!(entry(Q_MIN), (0xeef4_53d6_923b_d65a, 0x113f_aa29_06a1_3b40));
        assert_eq!(entry(Q_MAX), (0x8e67_9c2f_5e44_ff8f, 0x570f_09ea_a7ea_7648));
    }

    #[test]
    fn negative_entries_round_up() {
        // 10^-1 = 0.0CCCC…C repeating, so the exact 128-bit window ends in
        // ...cccc and the stored value must be one greater. This is the single
        // most load-bearing property of the table: rounding the reciprocal DOWN
        // instead would turn the algorithm's over-estimate into an
        // under-estimate and silently break its error bound.
        let (_hi, lo) = entry(-1);
        assert_eq!(
            lo & 0xf,
            0xd,
            "10^-1 low limb must be rounded up to ...cccd"
        );
    }

    #[test]
    fn hex_blob_has_one_entry_per_exponent() {
        let hex = powers_of_ten_hex();
        assert_eq!(hex.len(), ENTRY_COUNT * 16 * 2);
        assert_eq!(ENTRY_COUNT, 651);
        // The q = 0 entry must sit at its index, little-endian lo-then-hi.
        let index = (0 - Q_MIN) as usize;
        let start = index * 32;
        assert_eq!(
            &hex[start..start + 32],
            "00000000000000000000000000000080",
            "the 10^0 entry is misplaced or byte-swapped"
        );
    }
}
