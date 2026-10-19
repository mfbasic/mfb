//! plan-176-B: the second half of `rt_big_int`'s `modPow`-against-`remainder(pow)` grid
//! (positive bases). The first half is in `powers.rs`.

use super::*;

/// plan-127-C Phase 3: `modPow` against `remainder(pow)` over the 195 small signed cases
/// whose base is 1, 3 or 250 (exponents 0..12, moduli -17, -1, 2, 97, 256), in base-major
/// order. The bases up to zero are `mod_pow_matches_remainder_of_pow_for_bases_up_to_zero`.
#[test]
fn mod_pow_matches_remainder_of_pow_for_positive_bases() {
    let lines = run(
        "big_c_mod_pow_grid_positive",
        r#"IMPORT io
IMPORT big

FUNC main() AS Nothing
  MUT modPowMismatches AS Integer = 0
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(0), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(1), 0), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(0), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(1), 0), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(0), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(1), 0), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(0), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(1), 0), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(0), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(1), 0), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(1), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(1), 1), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(1), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(1), 1), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(1), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(1), 1), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(1), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(1), 1), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(1), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(1), 1), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(2), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(1), 2), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(2), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(1), 2), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(2), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(1), 2), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(2), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(1), 2), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(2), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(1), 2), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(3), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(1), 3), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(3), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(1), 3), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(3), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(1), 3), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(3), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(1), 3), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(3), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(1), 3), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(4), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(1), 4), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(4), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(1), 4), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(4), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(1), 4), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(4), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(1), 4), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(4), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(1), 4), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(5), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(1), 5), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(5), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(1), 5), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(5), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(1), 5), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(5), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(1), 5), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(5), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(1), 5), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(6), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(1), 6), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(6), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(1), 6), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(6), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(1), 6), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(6), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(1), 6), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(6), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(1), 6), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(7), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(1), 7), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(7), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(1), 7), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(7), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(1), 7), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(7), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(1), 7), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(7), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(1), 7), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(8), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(1), 8), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(8), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(1), 8), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(8), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(1), 8), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(8), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(1), 8), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(8), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(1), 8), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(9), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(1), 9), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(9), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(1), 9), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(9), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(1), 9), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(9), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(1), 9), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(9), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(1), 9), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(10), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(1), 10), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(10), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(1), 10), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(10), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(1), 10), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(10), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(1), 10), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(10), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(1), 10), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(11), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(1), 11), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(11), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(1), 11), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(11), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(1), 11), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(11), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(1), 11), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(11), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(1), 11), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(12), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(1), 12), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(12), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(1), 12), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(12), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(1), 12), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(12), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(1), 12), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(1), big::fromInteger(12), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(1), 12), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(0), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(3), 0), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(0), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(3), 0), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(0), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(3), 0), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(0), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(3), 0), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(0), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(3), 0), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(1), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(3), 1), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(1), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(3), 1), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(1), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(3), 1), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(1), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(3), 1), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(1), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(3), 1), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(2), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(3), 2), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(2), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(3), 2), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(2), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(3), 2), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(2), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(3), 2), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(2), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(3), 2), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(3), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(3), 3), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(3), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(3), 3), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(3), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(3), 3), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(3), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(3), 3), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(3), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(3), 3), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(4), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(3), 4), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(4), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(3), 4), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(4), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(3), 4), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(4), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(3), 4), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(4), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(3), 4), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(5), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(3), 5), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(5), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(3), 5), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(5), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(3), 5), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(5), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(3), 5), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(5), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(3), 5), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(6), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(3), 6), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(6), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(3), 6), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(6), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(3), 6), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(6), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(3), 6), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(6), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(3), 6), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(7), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(3), 7), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(7), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(3), 7), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(7), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(3), 7), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(7), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(3), 7), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(7), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(3), 7), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(8), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(3), 8), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(8), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(3), 8), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(8), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(3), 8), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(8), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(3), 8), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(8), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(3), 8), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(9), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(3), 9), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(9), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(3), 9), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(9), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(3), 9), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(9), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(3), 9), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(9), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(3), 9), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(10), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(3), 10), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(10), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(3), 10), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(10), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(3), 10), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(10), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(3), 10), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(10), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(3), 10), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(11), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(3), 11), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(11), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(3), 11), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(11), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(3), 11), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(11), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(3), 11), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(11), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(3), 11), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(12), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(3), 12), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(12), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(3), 12), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(12), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(3), 12), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(12), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(3), 12), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(3), big::fromInteger(12), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(3), 12), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(0), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(250), 0), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(0), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(250), 0), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(0), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(250), 0), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(0), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(250), 0), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(0), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(250), 0), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(1), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(250), 1), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(1), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(250), 1), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(1), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(250), 1), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(1), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(250), 1), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(1), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(250), 1), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(2), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(250), 2), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(2), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(250), 2), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(2), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(250), 2), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(2), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(250), 2), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(2), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(250), 2), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(3), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(250), 3), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(3), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(250), 3), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(3), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(250), 3), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(3), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(250), 3), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(3), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(250), 3), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(4), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(250), 4), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(4), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(250), 4), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(4), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(250), 4), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(4), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(250), 4), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(4), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(250), 4), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(5), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(250), 5), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(5), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(250), 5), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(5), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(250), 5), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(5), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(250), 5), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(5), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(250), 5), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(6), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(250), 6), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(6), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(250), 6), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(6), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(250), 6), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(6), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(250), 6), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(6), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(250), 6), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(7), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(250), 7), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(7), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(250), 7), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(7), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(250), 7), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(7), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(250), 7), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(7), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(250), 7), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(8), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(250), 8), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(8), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(250), 8), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(8), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(250), 8), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(8), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(250), 8), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(8), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(250), 8), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(9), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(250), 9), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(9), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(250), 9), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(9), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(250), 9), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(9), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(250), 9), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(9), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(250), 9), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(10), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(250), 10), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(10), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(250), 10), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(10), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(250), 10), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(10), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(250), 10), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(10), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(250), 10), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(11), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(250), 11), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(11), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(250), 11), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(11), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(250), 11), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(11), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(250), 11), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(11), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(250), 11), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(12), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(250), 12), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(12), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(250), 12), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(12), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(250), 12), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(12), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(250), 12), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(250), big::fromInteger(12), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(250), 12), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  io::print("modPow vs remainder(pow): " & toString(modPowMismatches) & " of 195")
END FUNC
"#,
    );
    assert_eq!(lines, vec!["modPow vs remainder(pow): 0 of 195"]);
}
