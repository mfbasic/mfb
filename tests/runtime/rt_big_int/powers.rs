//! plan-176-B: `rt_big_int`'s powers tests — the `factorial`/`pow`/`gcd`/`modPow` oracle
//! and the first half of the `modPow`-against-`remainder(pow)` grid (bases up to zero).

use super::*;

/// plan-127-C Phase 3: `factorial` (0, 1, 20, 100), `pow`, `gcd` (signs, zero, a large
/// shared factor, consecutive Fibonacci numbers) and `modPow` against values computed by
/// Python; the 390-case `modPow`-against-`remainder(pow)` grid is the two
/// `mod_pow_matches_remainder_of_pow_*` tests; `pow` against a folded `multiply`; and the
/// four `ErrInvalidArgument` cases.
#[test]
fn powers_gcd_factorial_and_mod_pow_match_an_independent_oracle() {
    let lines = run(
        "big_c_powers",
        r#"IMPORT io
IMPORT big

FUNC tryPow(n AS Integer) AS String
  RETURN big::toString(big::pow(big::fromInteger(2), n))
  TRAP(e)
    RETURN "raised " & toString(e.code)
  END TRAP
END FUNC

FUNC tryFactorial(n AS Integer) AS String
  RETURN big::toString(big::factorial(n))
  TRAP(e)
    RETURN "raised " & toString(e.code)
  END TRAP
END FUNC

FUNC tryModPow(e AS Integer, m AS Integer) AS String
  RETURN big::toString(big::modPow(big::fromInteger(3), big::fromInteger(e), big::fromInteger(m)))
  TRAP(err)
    RETURN "raised " & toString(err.code)
  END TRAP
END FUNC

FUNC main() AS Nothing
  io::print(big::toString(big::factorial(0)))
  io::print(big::toString(big::factorial(1)))
  io::print(big::toString(big::factorial(20)))
  io::print(big::toString(big::factorial(100)))
  io::print(big::toString(big::pow(big::fromInteger(7), 0)))
  io::print(big::toString(big::pow(big::fromInteger(0), 0)))
  io::print(big::toString(big::pow(big::fromInteger(-13), 1)))
  io::print(big::toString(big::pow(big::fromInteger(2), 100)))
  io::print(big::toString(big::pow(big::fromInteger(-3), 41)))
  io::print(big::toString(big::pow(big::parse("123456789123456789"), 7)))
  io::print(big::toString(big::gcd(big::fromInteger(-12), big::fromInteger(18))))
  io::print(big::toString(big::gcd(big::fromInteger(0), big::fromInteger(-7))))
  io::print(big::toString(big::gcd(big::fromInteger(0), big::fromInteger(0))))
  io::print(big::toString(big::gcd(big::parse("47660691512487140583616282547"), big::parse("-131194280351434800284477681165901434922899313011950236555"))))
  io::print(big::toString(big::gcd(big::parse("581811569836004006491505558634099066259034153405766997246569401"), big::parse("359579325206583560961765665172189099052367214309267232255589801"))))
  io::print(big::toString(big::modPow(big::parse("3"), big::parse("1000000000000000000000000000000"), big::parse("1000000007"))))
  io::print(big::toString(big::modPow(big::parse("-2"), big::parse("3"), big::parse("5"))))
  io::print(big::toString(big::modPow(big::parse("-2"), big::parse("4"), big::parse("5"))))
  io::print(big::toString(big::modPow(big::parse("5"), big::parse("0"), big::parse("13"))))
  io::print(big::toString(big::modPow(big::parse("5"), big::parse("0"), big::parse("1"))))
  io::print(big::toString(big::modPow(big::parse("5"), big::parse("0"), big::parse("-1"))))
  io::print(big::toString(big::modPow(big::parse("12345678901234567890"), big::parse("987654321"), big::parse("-1000000000000000003"))))
  io::print(big::toString(big::modPow(big::parse("-7"), big::parse("18446744073709551617"), big::parse("170141183460469231731687303715884105727"))))
  MUT powMismatches AS Integer = 0
  MUT folded AS big::Int = big::fromInteger(1)
  MUT e AS Integer = 0
  WHILE e <= 40
    IF NOT big::equals(big::pow(big::fromInteger(-37), e), folded) THEN
      powMismatches = powMismatches + 1
    END IF
    folded = big::multiply(folded, big::fromInteger(-37))
    e = e + 1
  END WHILE
  io::print("pow vs folded multiply: " & toString(powMismatches) & " of 41")
  io::print(tryPow(-1) & " | " & tryFactorial(-1) & " | " & tryModPow(-1, 7) & " | " & tryModPow(2, 0))
END FUNC
"#,
    );
    assert_eq!(
        lines,
        vec![
            "1",
            "1",
            "2432902008176640000",
            "93326215443944152681699238856266700490715968264381621468592963895217599993229915608941463976156518286253697920827223758251185210916864000000000000000000000000",
            "1",
            "1",
            "-13",
            "1267650600228229401496703205376",
            "-36472996377170786403",
            "437124192680754946535812089517240575642356431671694191678580822912775854282944444159297631378312438244249145659881926429",
            "6",
            "7",
            "0",
            "618970019642690137449562111",
            "1",
            "965115194",
            "-3",
            "1",
            "1",
            "0",
            "0",
            "745465334772758213",
            "-100316993018521585394941311808528038793",
            "pow vs folded multiply: 0 of 41",
            "raised 77050002 | raised 77050002 | raised 77050002 | raised 77050002"
        ]
    );
}

/// plan-127-C Phase 3: `modPow` against `remainder(pow)` over the 195 small signed cases
/// whose base is -9, -2 or 0 (exponents 0..12, moduli -17, -1, 2, 97, 256), in base-major
/// order. The bases above zero are `mod_pow_matches_remainder_of_pow_for_positive_bases`.
#[test]
fn mod_pow_matches_remainder_of_pow_for_bases_up_to_zero() {
    let lines = run(
        "big_c_mod_pow_grid_up_to_zero",
        r#"IMPORT io
IMPORT big

FUNC main() AS Nothing
  MUT modPowMismatches AS Integer = 0
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(0), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-9), 0), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(0), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-9), 0), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(0), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-9), 0), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(0), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-9), 0), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(0), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-9), 0), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(1), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-9), 1), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(1), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-9), 1), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(1), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-9), 1), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(1), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-9), 1), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(1), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-9), 1), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(2), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-9), 2), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(2), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-9), 2), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(2), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-9), 2), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(2), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-9), 2), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(2), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-9), 2), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(3), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-9), 3), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(3), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-9), 3), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(3), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-9), 3), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(3), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-9), 3), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(3), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-9), 3), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(4), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-9), 4), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(4), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-9), 4), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(4), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-9), 4), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(4), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-9), 4), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(4), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-9), 4), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(5), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-9), 5), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(5), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-9), 5), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(5), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-9), 5), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(5), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-9), 5), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(5), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-9), 5), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(6), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-9), 6), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(6), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-9), 6), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(6), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-9), 6), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(6), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-9), 6), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(6), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-9), 6), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(7), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-9), 7), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(7), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-9), 7), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(7), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-9), 7), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(7), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-9), 7), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(7), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-9), 7), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(8), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-9), 8), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(8), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-9), 8), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(8), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-9), 8), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(8), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-9), 8), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(8), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-9), 8), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(9), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-9), 9), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(9), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-9), 9), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(9), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-9), 9), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(9), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-9), 9), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(9), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-9), 9), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(10), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-9), 10), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(10), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-9), 10), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(10), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-9), 10), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(10), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-9), 10), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(10), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-9), 10), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(11), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-9), 11), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(11), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-9), 11), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(11), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-9), 11), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(11), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-9), 11), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(11), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-9), 11), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(12), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-9), 12), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(12), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-9), 12), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(12), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-9), 12), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(12), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-9), 12), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-9), big::fromInteger(12), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-9), 12), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(0), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-2), 0), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(0), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-2), 0), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(0), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-2), 0), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(0), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-2), 0), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(0), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-2), 0), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(1), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-2), 1), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(1), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-2), 1), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(1), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-2), 1), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(1), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-2), 1), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(1), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-2), 1), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(2), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-2), 2), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(2), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-2), 2), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(2), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-2), 2), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(2), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-2), 2), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(2), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-2), 2), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(3), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-2), 3), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(3), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-2), 3), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(3), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-2), 3), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(3), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-2), 3), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(3), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-2), 3), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(4), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-2), 4), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(4), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-2), 4), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(4), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-2), 4), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(4), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-2), 4), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(4), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-2), 4), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(5), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-2), 5), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(5), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-2), 5), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(5), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-2), 5), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(5), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-2), 5), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(5), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-2), 5), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(6), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-2), 6), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(6), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-2), 6), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(6), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-2), 6), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(6), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-2), 6), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(6), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-2), 6), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(7), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-2), 7), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(7), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-2), 7), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(7), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-2), 7), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(7), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-2), 7), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(7), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-2), 7), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(8), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-2), 8), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(8), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-2), 8), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(8), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-2), 8), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(8), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-2), 8), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(8), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-2), 8), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(9), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-2), 9), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(9), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-2), 9), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(9), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-2), 9), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(9), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-2), 9), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(9), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-2), 9), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(10), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-2), 10), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(10), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-2), 10), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(10), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-2), 10), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(10), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-2), 10), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(10), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-2), 10), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(11), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-2), 11), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(11), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-2), 11), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(11), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-2), 11), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(11), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-2), 11), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(11), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-2), 11), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(12), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(-2), 12), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(12), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(-2), 12), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(12), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(-2), 12), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(12), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(-2), 12), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(-2), big::fromInteger(12), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(-2), 12), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(0), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(0), 0), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(0), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(0), 0), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(0), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(0), 0), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(0), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(0), 0), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(0), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(0), 0), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(1), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(0), 1), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(1), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(0), 1), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(1), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(0), 1), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(1), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(0), 1), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(1), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(0), 1), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(2), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(0), 2), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(2), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(0), 2), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(2), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(0), 2), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(2), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(0), 2), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(2), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(0), 2), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(3), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(0), 3), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(3), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(0), 3), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(3), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(0), 3), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(3), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(0), 3), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(3), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(0), 3), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(4), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(0), 4), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(4), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(0), 4), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(4), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(0), 4), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(4), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(0), 4), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(4), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(0), 4), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(5), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(0), 5), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(5), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(0), 5), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(5), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(0), 5), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(5), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(0), 5), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(5), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(0), 5), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(6), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(0), 6), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(6), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(0), 6), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(6), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(0), 6), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(6), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(0), 6), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(6), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(0), 6), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(7), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(0), 7), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(7), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(0), 7), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(7), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(0), 7), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(7), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(0), 7), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(7), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(0), 7), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(8), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(0), 8), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(8), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(0), 8), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(8), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(0), 8), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(8), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(0), 8), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(8), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(0), 8), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(9), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(0), 9), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(9), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(0), 9), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(9), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(0), 9), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(9), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(0), 9), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(9), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(0), 9), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(10), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(0), 10), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(10), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(0), 10), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(10), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(0), 10), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(10), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(0), 10), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(10), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(0), 10), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(11), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(0), 11), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(11), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(0), 11), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(11), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(0), 11), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(11), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(0), 11), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(11), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(0), 11), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(12), big::fromInteger(-17)), big::remainder(big::pow(big::fromInteger(0), 12), big::fromInteger(-17))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(12), big::fromInteger(-1)), big::remainder(big::pow(big::fromInteger(0), 12), big::fromInteger(-1))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(12), big::fromInteger(2)), big::remainder(big::pow(big::fromInteger(0), 12), big::fromInteger(2))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(12), big::fromInteger(97)), big::remainder(big::pow(big::fromInteger(0), 12), big::fromInteger(97))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  IF NOT big::equals(big::modPow(big::fromInteger(0), big::fromInteger(12), big::fromInteger(256)), big::remainder(big::pow(big::fromInteger(0), 12), big::fromInteger(256))) THEN
    modPowMismatches = modPowMismatches + 1
  END IF
  io::print("modPow vs remainder(pow): " & toString(modPowMismatches) & " of 195")
END FUNC
"#,
    );
    assert_eq!(lines, vec!["modPow vs remainder(pow): 0 of 195"]);
}
