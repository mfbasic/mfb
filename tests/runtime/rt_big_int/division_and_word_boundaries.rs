//! plan-176-B: `rt_big_int`'s division tests and the word-boundary oracle (bug-775).

use super::*;

/// plan-127-C Phase 1: Algorithm D's quiet branches on pinned operands — a pair whose D3
/// estimate correction runs and a pair whose D6 add-back runs, both found by an
/// instrumented simulation and checked against Python's `divmod` — plus the normalization
/// boundary (divisor top byte 127 and 128), a divisor longer than the dividend, a divisor
/// equal to the dividend and to its negation, and a one-byte divisor.
#[test]
fn division_targeted_cases() {
    let lines = run(
        "big_c_division_cases",
        r#"IMPORT io
IMPORT big

FUNC show(parts AS big::DivResult) AS String
  RETURN big::toString(parts.quotient) & " " & big::toString(parts.remainder)
END FUNC

FUNC main() AS Nothing
  LET correctU AS List OF Byte = [15, 137, 103, 105]
  LET correctV AS List OF Byte = [50, 128]
  io::print(show(big::divMod(big::fromBytes(correctU, FALSE), big::fromBytes(correctV, FALSE))))
  LET addBackU AS List OF Byte = [38, 17, 133, 176]
  LET addBackV AS List OF Byte = [167, 28, 129]
  io::print(show(big::divMod(big::fromBytes(addBackU, FALSE), big::fromBytes(addBackV, FALSE))))
  LET dividend AS List OF Byte = [255, 255, 255, 255, 255, 255]
  LET below AS List OF Byte = [255, 127]
  LET at AS List OF Byte = [0, 128]
  io::print(show(big::divMod(big::fromBytes(dividend, FALSE), big::fromBytes(below, FALSE))))
  io::print(show(big::divMod(big::fromBytes(dividend, FALSE), big::fromBytes(at, FALSE))))
  io::print(show(big::divMod(big::fromInteger(-12345), big::fromBytes(dividend, FALSE))))
  LET wide AS big::Int = big::fromBytes(dividend, TRUE)
  io::print(show(big::divMod(wide, wide)))
  io::print(show(big::divMod(wide, big::negate(wide))))
  io::print(show(big::divMod(big::fromBytes(dividend, FALSE), big::fromInteger(7))))
END FUNC
"#,
    );
    // Expected values computed by Python `divmod` (non-negative operands) and by the
    // measured truncating rule for the signed ones — never by this code.
    assert_eq!(
        lines,
        vec![
            "53884 27863",
            "349 8454523",
            "8590196744 7",
            "8589934591 32767",
            "0 -12345",
            "1 0",
            "-1 0",
            "40210710958665 0",
        ]
    );
}

/// plan-127-C Phase 1: the defining property over 10,000 seeded random pairs spanning 1 to
/// 512 bytes of magnitude and all four sign quadrants — `a = b * q + r`, `|r| < |b|`, the
/// remainder takes the dividend's sign and the quotient truncates toward zero. Quotient and
/// remainder are unique under those rules, so the property is a complete check; the seed
/// is fixed so a failure reproduces.
#[test]
fn division_property_over_ten_thousand_seeded_pairs() {
    let project = common::temp_project(
        "big_c_division_property",
        r#"IMPORT io
IMPORT big
IMPORT collections
IMPORT math

FUNC randomMagnitude(length AS Integer) AS List OF Byte
  MUT bytes AS List OF Byte = []
  MUT i AS Integer = 0
  WHILE i < length
    bytes = collections::append(bytes, toByte(math::rand(0, 255)))
    i = i + 1
  END WHILE
  RETURN bytes
END FUNC

FUNC main() AS Nothing
  math::seed(127)
  MUT identityFailures AS Integer = 0
  MUT boundFailures AS Integer = 0
  MUT signFailures AS Integer = 0
  MUT quadrants AS List OF Integer = [0, 0, 0, 0]
  MUT shorterDivisors AS Integer = 0
  MUT pair AS Integer = 0
  WHILE pair < 10000
    MUT a AS big::Int = big::fromBytes(randomMagnitude(math::rand(1, 512)), FALSE)
    MUT b AS big::Int = big::fromBytes(randomMagnitude(math::rand(1, 512)), FALSE)
    IF big::isZero(b) THEN
      b = big::fromInteger(1)
    END IF
    LET quadrant AS Integer = math::rand(0, 3)
    IF quadrant = 1 OR quadrant = 3 THEN
      a = big::negate(a)
    END IF
    IF quadrant = 2 OR quadrant = 3 THEN
      b = big::negate(b)
    END IF
    quadrants = collections::set(quadrants, quadrant, collections::get(quadrants, quadrant) + 1)
    IF len(b.magnitude) < len(a.magnitude) THEN
      shorterDivisors = shorterDivisors + 1
    END IF
    LET parts AS big::DivResult = big::divMod(a, b)
    IF NOT big::equals(big::add(big::multiply(b, parts.quotient), parts.remainder), a) THEN
      identityFailures = identityFailures + 1
    END IF
    IF big::compare(big::abs(parts.remainder), big::abs(b)) >= 0 THEN
      boundFailures = boundFailures + 1
    END IF
    IF NOT big::isZero(parts.remainder) AND big::sign(parts.remainder) <> big::sign(a) THEN
      signFailures = signFailures + 1
    END IF
    IF NOT big::isZero(parts.quotient) AND big::sign(parts.quotient) <> big::sign(a) * big::sign(b) THEN
      signFailures = signFailures + 1
    END IF
    pair = pair + 1
  END WHILE
  io::print("pairs " & toString(pair))
  io::print("shorter " & toString(shorterDivisors))
  io::print("quadrants " & toString(collections::get(quadrants, 0)) & " " & toString(collections::get(quadrants, 1)) & " " & toString(collections::get(quadrants, 2)) & " " & toString(collections::get(quadrants, 3)))
  io::print("identity " & toString(identityFailures))
  io::print("bound " & toString(boundFailures))
  io::print("sign " & toString(signFailures))
END FUNC
"#,
    );
    let binary = common::build_project(&project);
    let (status, stdout) = common::run_bounded(
        &binary,
        Duration::from_secs(600),
        "the 10,000-pair division property did not finish",
    );
    assert!(
        status.success(),
        "program {}:\n{stdout}",
        common::exit_description(&status)
    );
    let _ = std::fs::remove_dir_all(&project);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "pairs 10000", "{stdout}");
    // Both shapes of divisor were exercised, and every quadrant was drawn.
    let shorter: usize = lines[1]
        .trim_start_matches("shorter ")
        .parse()
        .expect("shorter count");
    assert!(shorter > 1000 && shorter < 9000, "{stdout}");
    let quadrants: Vec<usize> = lines[2]
        .trim_start_matches("quadrants ")
        .split(' ')
        .map(|q| q.parse().expect("quadrant count"))
        .collect();
    assert!(quadrants.iter().all(|&q| q > 1000), "{stdout}");
    assert_eq!(
        &lines[3..],
        &["identity 0", "bound 0", "sign 0"],
        "{stdout}"
    );
}

/// plan-127-C Phase 2: `divide`/`remainder` agree with the matching `divMod` field across a
/// seeded spread in every sign quadrant; the §4.2 truncating convention on `(±7, ±2)`, which
/// matches the measured `MOD` (`-7 MOD 2 = -1`, `7 MOD -2 = 1`); each division member raises
/// `ErrInvalidArgument` on a zero divisor.
#[test]
fn division_members_agree_and_raise() {
    let lines = run(
        "big_c_division_members",
        r#"IMPORT io
IMPORT big
IMPORT collections
IMPORT math

FUNC randomMagnitude(length AS Integer) AS List OF Byte
  MUT bytes AS List OF Byte = []
  MUT i AS Integer = 0
  WHILE i < length
    bytes = collections::append(bytes, toByte(math::rand(0, 255)))
    i = i + 1
  END WHILE
  RETURN bytes
END FUNC

FUNC tryDivide(a AS big::Int, b AS big::Int) AS String
  RETURN big::toString(big::divide(a, b))
  TRAP(e)
    RETURN "raised " & toString(e.code)
  END TRAP
END FUNC

FUNC tryRemainder(a AS big::Int, b AS big::Int) AS String
  RETURN big::toString(big::remainder(a, b))
  TRAP(e)
    RETURN "raised " & toString(e.code)
  END TRAP
END FUNC

FUNC tryDivMod(a AS big::Int, b AS big::Int) AS String
  LET parts AS big::DivResult = big::divMod(a, b)
  RETURN big::toString(parts.quotient) & " " & big::toString(parts.remainder)
  TRAP(e)
    RETURN "raised " & toString(e.code)
  END TRAP
END FUNC

FUNC main() AS Nothing
  math::seed(128)
  MUT disagreements AS Integer = 0
  MUT pair AS Integer = 0
  WHILE pair < 500
    MUT a AS big::Int = big::fromBytes(randomMagnitude(math::rand(1, 64)), math::rand(0, 1) = 1)
    MUT b AS big::Int = big::fromBytes(randomMagnitude(math::rand(1, 64)), math::rand(0, 1) = 1)
    IF big::isZero(b) THEN
      b = big::fromInteger(3)
    END IF
    LET parts AS big::DivResult = big::divMod(a, b)
    IF NOT big::equals(big::divide(a, b), parts.quotient) THEN
      disagreements = disagreements + 1
    END IF
    IF NOT big::equals(big::remainder(a, b), parts.remainder) THEN
      disagreements = disagreements + 1
    END IF
    pair = pair + 1
  END WHILE
  io::print("agreement: " & toString(disagreements) & " of " & toString(pair * 2))
  LET seven AS big::Int = big::fromInteger(7)
  LET two AS big::Int = big::fromInteger(2)
  io::print(tryDivMod(seven, two) & " | " & tryDivMod(big::negate(seven), two) & " | " & tryDivMod(seven, big::negate(two)) & " | " & tryDivMod(big::negate(seven), big::negate(two)))
  LET zero AS big::Int = big::fromInteger(0)
  io::print(tryDivide(seven, zero) & " | " & tryRemainder(seven, zero) & " | " & tryDivMod(seven, zero))
END FUNC
"#,
    );
    assert_eq!(
        lines,
        vec![
            "agreement: 0 of 1000",
            "3 1 | -3 -1 | -3 1 | 3 -1",
            "raised 77050002 | raised 77050002 | raised 77050002"
        ]
    );
}

/// bug-775: the word kernels at every word-tail boundary. Operand magnitudes of 1 to 17
/// bytes (whole words, a word plus 1..7 bytes, two words plus one) through every
/// arithmetic member, each operand hand-built with trailing zero bytes so its list's
/// capacity is not a multiple of 8 (a word read past the significant bytes would leave
/// the block); all-`0xff` runs whose `+ 1` carries through every word; `toRadixString`
/// for every radix 2..36 at the chunk boundaries `radix^k - 1`, `radix^k`, `radix^2k - 1`
/// and `radix^2k` (`radix^k` the largest power below 2^32), decimal powers of ten across
/// the 9-digit chunk; `modPow` with exponents 0, 1 and 2^k +- 1; and six divisions whose
/// Algorithm D step needs the add-back (D6), found by simulating base-2^32 Algorithm D.
/// Every expected line was computed by Python's `int`, never by this code.
#[test]
fn word_boundaries_match_an_independent_oracle() {
    let lines = run(
        "big_word_boundaries",
        r#"IMPORT io
IMPORT big

FUNC hex(x AS big::Int) AS String
  RETURN big::toRadixString(x, 16)
END FUNC

FUNC row(n AS Integer, a AS big::Int, b AS big::Int, e AS Integer) AS Nothing
  LET parts AS big::DivResult = big::divMod(a, b)
  io::print(toString(n) & " " & hex(big::add(a, b)) & " " & hex(big::subtract(a, b)) & " " & hex(big::multiply(a, b)))
  io::print(toString(n) & " " & hex(parts.quotient) & " " & hex(parts.remainder) & " " & hex(big::divide(b, a)) & " " & hex(big::remainder(b, a)))
  io::print(toString(n) & " " & hex(big::gcd(a, b)) & " " & hex(big::modPow(a, big::fromInteger(e), b)) & " " & big::toString(big::multiply(a, a)) & " " & toString(big::compare(a, b)))
  io::print(toString(n) & " " & toString(big::equals(big::parse(big::toString(a)), a)) & " " & toString(big::equals(big::parse(hex(b), 16), b)) & " " & hex(big::abs(a)) & " " & hex(big::negate(b)))
END FUNC

FUNC ones(n AS Integer, x AS big::Int) AS Nothing
  LET one AS big::Int = big::fromInteger(1)
  LET up AS big::Int = big::add(x, one)
  LET square AS big::Int = big::multiply(x, x)
  LET parts AS big::DivResult = big::divMod(big::add(square, x), up)
  io::print("ff" & toString(n) & " " & hex(up) & " " & toString(big::equals(big::subtract(up, one), x)) & " " & hex(square) & " " & hex(parts.quotient) & " " & hex(parts.remainder) & " " & big::toString(up))
END FUNC

FUNC radixRow(r AS Integer, k AS Integer) AS Nothing
  LET one AS big::Int = big::fromInteger(1)
  LET p AS big::Int = big::pow(big::fromInteger(r), k)
  LET p2 AS big::Int = big::multiply(p, p)
  LET values AS List OF big::Int = [big::subtract(p, one), p, big::subtract(p2, one), p2, big::negate(big::add(p2, one))]
  MUT line AS String = "r" & toString(r)
  MUT trips AS Integer = 0
  FOR EACH v IN values
    LET text AS String = big::toRadixString(v, r)
    line = line & " " & text
    IF big::equals(big::parse(text, r), v) THEN
      trips = trips + 1
    END IF
  NEXT
  io::print(line & " " & toString(trips))
END FUNC

FUNC main() AS Nothing
  row(1, big::Int[[7, 0], TRUE], big::Int[[4, 201, 142, 83, 24, 221, 0, 0], FALSE], 0)
  row(2, big::Int[[14, 145, 0, 0], FALSE], big::Int[[5, 202, 143, 84, 25, 222, 163, 104, 45, 242, 183, 0, 0, 0], TRUE], 1)
  row(3, big::Int[[21, 152, 27], FALSE], big::Int[[6, 203, 144, 85, 26, 223, 164, 105, 46, 243, 184, 125, 66, 7, 204, 145], FALSE], 2)
  row(4, big::Int[[28, 159, 34, 165, 0], FALSE], big::Int[[7, 204, 145, 86, 0], FALSE], 3)
  row(5, big::Int[[35, 166, 41, 172, 47, 0, 0], TRUE], big::Int[[8, 205, 146, 87, 28, 225, 166, 107, 48, 0, 0], TRUE], 7)
  row(6, big::Int[[42, 173, 48, 179, 54, 185], FALSE], big::Int[[9, 206, 147, 88, 29, 226, 167, 108, 49, 246, 187, 128, 69, 10, 0, 0, 0], FALSE], 8)
  row(7, big::Int[[49, 180, 55, 186, 61, 192, 67, 0], FALSE], big::Int[[10, 207], FALSE], 9)
  row(8, big::Int[[56, 187, 62, 193, 68, 199, 74, 205, 0, 0], FALSE], big::Int[[11, 208, 149, 90, 31, 228, 169, 0], TRUE], 255)
  row(9, big::Int[[63, 194, 69, 200, 75, 206, 81, 212, 87], TRUE], big::Int[[12, 209, 150, 91, 32, 229, 170, 111, 52, 249, 190, 131, 0, 0], FALSE], 256)
  row(10, big::Int[[70, 201, 76, 207, 82, 213, 88, 219, 94, 225, 0], FALSE], big::Int[[13, 210, 151, 92, 33, 230, 171, 112, 53, 250, 191, 132, 73, 14, 211, 152, 93, 0, 0, 0], FALSE], 257)
  row(11, big::Int[[77, 208, 83, 214, 89, 220, 95, 226, 101, 232, 107, 0, 0], FALSE], big::Int[[14, 211, 152, 93, 34], TRUE], 65535)
  row(12, big::Int[[84, 215, 90, 221, 96, 227, 102, 233, 108, 239, 114, 245], FALSE], big::Int[[15, 212, 153, 94, 35, 232, 173, 114, 55, 252, 0], FALSE], 65536)
  row(13, big::Int[[91, 222, 97, 228, 103, 234, 109, 240, 115, 246, 121, 252, 127, 0], TRUE], big::Int[[16, 213, 154, 95, 36, 233, 174, 115, 56, 253, 194, 135, 76, 17, 214, 0, 0], FALSE], 65537)
  row(14, big::Int[[98, 229, 104, 235, 110, 241, 116, 247, 122, 253, 128, 3, 134, 9, 0, 0], FALSE], big::Int[[17, 214, 155, 0, 0, 0], TRUE], 3)
  row(15, big::Int[[105, 236, 111, 242, 117, 248, 123, 254, 129, 4, 135, 10, 141, 16, 147], FALSE], big::Int[[18, 215, 156, 97, 38, 235, 176, 117], FALSE], 1)
  row(16, big::Int[[112, 243, 118, 249, 124, 255, 130, 5, 136, 11, 142, 17, 148, 23, 154, 29, 0], FALSE], big::Int[[19, 216, 157, 98, 39, 236, 177, 118, 59, 0, 197, 138, 79, 0], FALSE], 0)
  row(17, big::Int[[119, 250, 125, 0, 131, 6, 137, 12, 143, 18, 149, 24, 155, 30, 161, 36, 167, 0, 0], TRUE], big::Int[[20, 0, 0], TRUE], 17)
  ones(1, big::Int[[255], FALSE])
  ones(2, big::Int[[255, 255], FALSE])
  ones(3, big::Int[[255, 255, 255], FALSE])
  ones(4, big::Int[[255, 255, 255, 255], FALSE])
  ones(5, big::Int[[255, 255, 255, 255, 255], FALSE])
  ones(6, big::Int[[255, 255, 255, 255, 255, 255], FALSE])
  ones(7, big::Int[[255, 255, 255, 255, 255, 255, 255], FALSE])
  ones(8, big::Int[[255, 255, 255, 255, 255, 255, 255, 255], FALSE])
  ones(9, big::Int[[255, 255, 255, 255, 255, 255, 255, 255, 255], FALSE])
  ones(10, big::Int[[255, 255, 255, 255, 255, 255, 255, 255, 255, 255], FALSE])
  ones(11, big::Int[[255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255], FALSE])
  ones(12, big::Int[[255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255], FALSE])
  ones(13, big::Int[[255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255], FALSE])
  ones(14, big::Int[[255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255], FALSE])
  ones(15, big::Int[[255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255], FALSE])
  ones(16, big::Int[[255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255], FALSE])
  ones(17, big::Int[[255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255], FALSE])
  radixRow(2, 31)
  radixRow(3, 20)
  radixRow(4, 15)
  radixRow(5, 13)
  radixRow(6, 12)
  radixRow(7, 11)
  radixRow(8, 10)
  radixRow(9, 10)
  radixRow(10, 9)
  radixRow(11, 9)
  radixRow(12, 8)
  radixRow(13, 8)
  radixRow(14, 8)
  radixRow(15, 8)
  radixRow(16, 7)
  radixRow(17, 7)
  radixRow(18, 7)
  radixRow(19, 7)
  radixRow(20, 7)
  radixRow(21, 7)
  radixRow(22, 7)
  radixRow(23, 7)
  radixRow(24, 6)
  radixRow(25, 6)
  radixRow(26, 6)
  radixRow(27, 6)
  radixRow(28, 6)
  radixRow(29, 6)
  radixRow(30, 6)
  radixRow(31, 6)
  radixRow(32, 6)
  radixRow(33, 6)
  radixRow(34, 6)
  radixRow(35, 6)
  radixRow(36, 6)
  LET ten AS big::Int = big::fromInteger(10)
  MUT decimals AS String = "dec"
  decimals = decimals & " " & big::toString(big::subtract(big::pow(ten, 9), big::fromInteger(1))) & " " & big::toString(big::pow(ten, 9))
  decimals = decimals & " " & big::toString(big::subtract(big::pow(ten, 18), big::fromInteger(1))) & " " & big::toString(big::pow(ten, 18))
  decimals = decimals & " " & big::toString(big::subtract(big::pow(ten, 19), big::fromInteger(1))) & " " & big::toString(big::pow(ten, 19))
  decimals = decimals & " " & big::toString(big::subtract(big::pow(ten, 27), big::fromInteger(1))) & " " & big::toString(big::pow(ten, 27))
  decimals = decimals & " " & big::toString(big::subtract(big::pow(ten, 36), big::fromInteger(1))) & " " & big::toString(big::pow(ten, 36))
  io::print(decimals)
  LET mpBase AS big::Int = big::Int[[11, 100, 189, 22, 111, 200, 33, 122, 211, 44, 133, 222, 55, 144, 233, 66, 77], FALSE]
  LET mpMod AS big::Int = big::Int[[5, 58, 111, 164, 217, 14, 67, 120, 201, 0, 0], TRUE]
  MUT powers AS String = "mp"
  powers = powers & " " & hex(big::modPow(mpBase, big::fromInteger(0), mpMod))
  powers = powers & " " & hex(big::modPow(mpBase, big::fromInteger(1), mpMod))
  powers = powers & " " & hex(big::modPow(mpBase, big::fromInteger(2), mpMod))
  powers = powers & " " & hex(big::modPow(mpBase, big::fromInteger(3), mpMod))
  powers = powers & " " & hex(big::modPow(mpBase, big::fromInteger(15), mpMod))
  powers = powers & " " & hex(big::modPow(mpBase, big::fromInteger(16), mpMod))
  powers = powers & " " & hex(big::modPow(mpBase, big::fromInteger(17), mpMod))
  powers = powers & " " & hex(big::modPow(mpBase, big::fromInteger(255), mpMod))
  powers = powers & " " & hex(big::modPow(mpBase, big::fromInteger(256), mpMod))
  powers = powers & " " & hex(big::modPow(mpBase, big::fromInteger(257), mpMod))
  powers = powers & " " & hex(big::modPow(mpBase, big::fromInteger(65535), mpMod))
  powers = powers & " " & hex(big::modPow(mpBase, big::fromInteger(65536), mpMod))
  powers = powers & " " & hex(big::modPow(mpBase, big::fromInteger(65537), mpMod))
  io::print(powers)
  LET parts0 AS big::DivResult = big::divMod(big::parse("fffffffefffffffe0000000280000001", 16), big::parse("fffffffefffffffeffffffff", 16))
  io::print("ab0 " & hex(parts0.quotient) & " " & hex(parts0.remainder))
  LET parts1 AS big::DivResult = big::divMod(big::parse("3fffffffe000000007fffffff00000000", 16), big::parse("3fffffffe0000000180000000", 16))
  io::print("ab1 " & hex(parts1.quotient) & " " & hex(parts1.remainder))
  LET parts2 AS big::DivResult = big::divMod(big::parse("17fffffff7fffffff7fffffff7fffffff", 16), big::parse("2fffffffefffffffeffffffff", 16))
  io::print("ab2 " & hex(parts2.quotient) & " " & hex(parts2.remainder))
  LET parts3 AS big::DivResult = big::divMod(big::parse("8000000180000000000000004000000080000001", 16), big::parse("80000001800000000000000180000000", 16))
  io::print("ab3 " & hex(parts3.quotient) & " " & hex(parts3.remainder))
  LET parts4 AS big::DivResult = big::divMod(big::parse("10000000200000002400000000000000380000001", 16), big::parse("1000000020000000380000001", 16))
  io::print("ab4 " & hex(parts4.quotient) & " " & hex(parts4.remainder))
  LET parts5 AS big::DivResult = big::divMod(big::parse("40000000800000000000000280000000", 16), big::parse("40000000800000007fffffff", 16))
  io::print("ab5 " & hex(parts5.quotient) & " " & hex(parts5.remainder))
END FUNC
"#,
    );
    assert_eq!(
        lines,
        vec![
            "1 dd18538ec8fd -dd18538ec90b -60baa48e77f1c",
            "1 0 -7 -1f95c2cb4149 5",
            "1 1 1 49 -1",
            "1 TRUE TRUE 7 -dd18538ec904",
            "2 -b7f22d68a3de19548f38f7 b7f22d68a3de1954905b13 -683a3af6c089c27e481149e146",
            "2 0 910e -144a32cfe8d25bf2544 -3c4d",
            "2 1 910e 1378933956 1",
            "2 TRUE TRUE 910e b7f22d68a3de19548fca05",
            "3 91cc07427db8f32e69a4df1a55ac631b -91cc07427db8f32e69a4df1a557532f1 fb721de0f559bc674a5ec16c4f63c670e377e",
            "3 0 1b9815 5489c473c71af5306f3b9bc0463 18a6e7",
            "3 1 2f96ec6f1b9 3270328644025 -1",
            "3 TRUE TRUE 1b9815 -91cc07427db8f32e69a4df1a5590cb06",
            "4 fbb46b23 4e90d315 37d7adaba198a9c4",
            "4 1 4e90d315 0 5691cc07",
            "4 1 3c131584 7675723421528083216 1",
            "4 TRUE TRUE a5229f1c -5691cc07",
            "5 -306ba6e14c03bc732b 306ba6e0ecab6926e5 90453d81d9a1a1d7465604d3818",
            "5 0 -2fac29a623 104044a96 -748995686",
            "5 1 -2ecf10f1d65684dc8b 41923329730834768292041 1",
            "5 TRUE TRUE 2fac29a623 306ba6e11c5792cd08",
            "6 a4580bbf6316ca89b540bc47b33 -a4580bbf6316ca728e6a56320df 76e6be1898dddbb1a447451b0dad959572be27a",
            "6 0 b936b330ad2a e3277371d425311 6932435fb03f",
            "6 1 383015763f6ad1c5bebda22a075 41471117278516356667439958756 -1",
            "6 TRUE TRUE b936b330ad2a -a4580bbf6316ca7e21d5893ce09",
            "7 43c03dba38833b 43c03dba36e527 36cb176bfc50e0a8ea",
            "7 53c5d5f375 969f 0 cf0a",
            "7 1 22a3 363672329304315796533343798096225 1",
            "7 TRUE TRUE 43c03dba37b431 -cf0a",
            "8 cca0e32566a8eb2d cdf4ab641bd48b43 -883d4d4a8f5d4298618013ed678b68",
            "8 -135 3a6d6c6a6a9df1 0 -a9e41f5a95d00b",
            "8 1 895e9d857ab4c0 218828559765982405281322849784386214976 1",
            "8 TRUE TRUE cd4ac744c13ebb38 a9e41f5a95d00b",
            "9 83bef8dc9b5916d493510ecd -83bef98c43fcb36c23dc934b -2d332af0c8e5b10629ee00c1b04c2a51da8cc489f4",
            "9 0 -57d451ce4bc845c23f -1800170 3dc865ef8d18cf967c",
            "9 1 1a15b5e37a4cc560ceb7bacd 2624937772441768225793155141677463442656129 -1",
            "9 TRUE TRUE 57d451ce4bc845c23f -83bef9346faae5205b96d10c",
            "10 5d98d30e4984c0db944c04bb742be49b53 -5d98d30e4984bf18d6955310ce8d4b08c7 5265ffcf44f0dfa060117b9ee87d18b87e8d92605577e7644ba48e",
            "10 0 e15edb58d552cf4cc946 6a51458a0d6f46 a9f3f69158566dc56ee9",
            "10 1 15eed629e356edf721c9b99faf53b6bf26 1132696725132153892275987576031819084344780062500 -1",
            "10 TRUE TRUE e15edb58d552cf4cc946 -5d98d30e4984bffa3570abe6215c97d20d",
            "11 6be865e25fdc3778bafd3f 6be865e25fdc7c33eca35b -e7c516006e92f0d5331773a73fcdb36",
            "11 -323d69e7d6bfa 15dd08daa1 0 -225d98d30e",
            "11 1 601837205 17017862884991803689996089871277723172281486527903529 1",
            "11 TRUE TRUE 6be865e25fdc59d653d04d 225d98d30e",
            "12 f573eba45c14cb843bf4ab63 f571f33576b8fb3d7ec10345 f1d24d549203cbdcdfd392715ad1d9d2c0d3bbd82dec",
            "12 f921 7dbbfa318fce0c43ea65 0 fc3772ade8235e99d40f",
            "12 3 c9554351a4f85984d0ab 5770343277512540405937198306825720311557496355100424024976 1",
            "12 TRUE TRUE f572ef6ce966e360dd5ad754 -fc3772ade8235e99d40f",
            "13 d610cc8b4906c48340febc7b38f6b5 -d611cc843cf3ac641cd38c43fcb36b -6b05b3faf21e176620cd0249b82151e78dc6942f3f24e3b33bdd9cb0",
            "13 0 -7ffc79f673f06dea67e461de5b -1ac2e 3141cede6b7c0918a5b644bcb6",
            "13 1 -cab476a601f140e48d2a9042a53a6b 102821918131679790257824652558768506043392517342963308214875225 -1",
            "13 TRUE TRUE 7ffc79f673f06dea67e461de5b -d6114c87c2fd3873aee9245f9ad510",
            "14 9860380fd7af774f16eeacd0f51 9860380fd7af774f16eec04bb73 -5cc1ac7ed88e973d45ebf49aa340d2782",
            "14 -fa50c60eb95a169d3d177 31827b 0 -9bd611",
            "14 1 83d1a1 37310532407977354761344809517822951310708635188841164349888821636 1",
            "14 TRUE TRUE 9860380fd7af774f16eeb68e562 9bd611",
            "15 93108d0a870482742ce39c540cc37b 93108d0a87048188cb0d4f90d31557 439c32ed04f2db1808ce6e6f4a859b5af02644e466ce62",
            "15 13fe499403dd7e3 66db2f8cec731973 0 75b0eb26619cd712",
            "15 1 66db2f8cec731973 583090047400293980654494489377930971460567112179535079625413903272362769 1",
            "15 TRUE TRUE 93108d0a870481fe7bf875f26fec69 -75b0eb26619cd712",
            "16 1d9a17e39c530bc37c34eba45c14cb83 1d9a174486c90b4c8ed1135596d91b5d 932991f02417aac571ae34d39e04344995a48b32b116945f70cea9150",
            "16 5f4580 4c7fecdac81548b5a3f040caf0 0 4f8ac5003b76b1ec27629dd813",
            "16 3 1 1548241795953567523893560509944832686394263076341410982883093456843272212736 1",
            "16 TRUE TRUE 1d9a1794118e0b880582ff7cf976f370 -4f8ac5003b76b1ec27629dd813",
            "17 -a724a11e9b1895128f0c890683007dfa8b -a724a11e9b1895128f0c890683007dfa63 d0edc96641deba5732cfab4823c09d7914c",
            "17 85b6e74baf46dda872706d9ecf3397fb9 -3 0 -14",
            "17 1 -3 3234861638015133588703168155165539756693862472346873600225915352059548541196084049 -1",
            "17 TRUE TRUE a724a11e9b1895128f0c890683007dfa77 14",
            "ff1 100 TRUE fe01 ff 0 256",
            "ff2 10000 TRUE fffe0001 ffff 0 65536",
            "ff3 1000000 TRUE fffffe000001 ffffff 0 16777216",
            "ff4 100000000 TRUE fffffffe00000001 ffffffff 0 4294967296",
            "ff5 10000000000 TRUE fffffffffe0000000001 ffffffffff 0 1099511627776",
            "ff6 1000000000000 TRUE fffffffffffe000000000001 ffffffffffff 0 281474976710656",
            "ff7 100000000000000 TRUE fffffffffffffe00000000000001 ffffffffffffff 0 72057594037927936",
            "ff8 10000000000000000 TRUE fffffffffffffffe0000000000000001 ffffffffffffffff 0 18446744073709551616",
            "ff9 1000000000000000000 TRUE fffffffffffffffffe000000000000000001 ffffffffffffffffff 0 4722366482869645213696",
            "ff10 100000000000000000000 TRUE fffffffffffffffffffe00000000000000000001 ffffffffffffffffffff 0 1208925819614629174706176",
            "ff11 10000000000000000000000 TRUE fffffffffffffffffffffe0000000000000000000001 ffffffffffffffffffffff 0 309485009821345068724781056",
            "ff12 1000000000000000000000000 TRUE fffffffffffffffffffffffe000000000000000000000001 ffffffffffffffffffffffff 0 79228162514264337593543950336",
            "ff13 100000000000000000000000000 TRUE fffffffffffffffffffffffffe00000000000000000000000001 ffffffffffffffffffffffffff 0 20282409603651670423947251286016",
            "ff14 10000000000000000000000000000 TRUE fffffffffffffffffffffffffffe0000000000000000000000000001 ffffffffffffffffffffffffffff 0 5192296858534827628530496329220096",
            "ff15 1000000000000000000000000000000 TRUE fffffffffffffffffffffffffffffe000000000000000000000000000001 ffffffffffffffffffffffffffffff 0 1329227995784915872903807060280344576",
            "ff16 100000000000000000000000000000000 TRUE fffffffffffffffffffffffffffffffe00000000000000000000000000000001 ffffffffffffffffffffffffffffffff 0 340282366920938463463374607431768211456",
            "ff17 10000000000000000000000000000000000 TRUE fffffffffffffffffffffffffffffffffe0000000000000000000000000000000001 ffffffffffffffffffffffffffffffffff 0 87112285931760246646623899502532662132736",
            "r2 1111111111111111111111111111111 10000000000000000000000000000000 11111111111111111111111111111111111111111111111111111111111111 100000000000000000000000000000000000000000000000000000000000000 -100000000000000000000000000000000000000000000000000000000000001 5",
            "r3 22222222222222222222 100000000000000000000 2222222222222222222222222222222222222222 10000000000000000000000000000000000000000 -10000000000000000000000000000000000000001 5",
            "r4 333333333333333 1000000000000000 333333333333333333333333333333 1000000000000000000000000000000 -1000000000000000000000000000001 5",
            "r5 4444444444444 10000000000000 44444444444444444444444444 100000000000000000000000000 -100000000000000000000000001 5",
            "r6 555555555555 1000000000000 555555555555555555555555 1000000000000000000000000 -1000000000000000000000001 5",
            "r7 66666666666 100000000000 6666666666666666666666 10000000000000000000000 -10000000000000000000001 5",
            "r8 7777777777 10000000000 77777777777777777777 100000000000000000000 -100000000000000000001 5",
            "r9 8888888888 10000000000 88888888888888888888 100000000000000000000 -100000000000000000001 5",
            "r10 999999999 1000000000 999999999999999999 1000000000000000000 -1000000000000000001 5",
            "r11 aaaaaaaaa 1000000000 aaaaaaaaaaaaaaaaaa 1000000000000000000 -1000000000000000001 5",
            "r12 bbbbbbbb 100000000 bbbbbbbbbbbbbbbb 10000000000000000 -10000000000000001 5",
            "r13 cccccccc 100000000 cccccccccccccccc 10000000000000000 -10000000000000001 5",
            "r14 dddddddd 100000000 dddddddddddddddd 10000000000000000 -10000000000000001 5",
            "r15 eeeeeeee 100000000 eeeeeeeeeeeeeeee 10000000000000000 -10000000000000001 5",
            "r16 fffffff 10000000 ffffffffffffff 100000000000000 -100000000000001 5",
            "r17 ggggggg 10000000 gggggggggggggg 100000000000000 -100000000000001 5",
            "r18 hhhhhhh 10000000 hhhhhhhhhhhhhh 100000000000000 -100000000000001 5",
            "r19 iiiiiii 10000000 iiiiiiiiiiiiii 100000000000000 -100000000000001 5",
            "r20 jjjjjjj 10000000 jjjjjjjjjjjjjj 100000000000000 -100000000000001 5",
            "r21 kkkkkkk 10000000 kkkkkkkkkkkkkk 100000000000000 -100000000000001 5",
            "r22 lllllll 10000000 llllllllllllll 100000000000000 -100000000000001 5",
            "r23 mmmmmmm 10000000 mmmmmmmmmmmmmm 100000000000000 -100000000000001 5",
            "r24 nnnnnn 1000000 nnnnnnnnnnnn 1000000000000 -1000000000001 5",
            "r25 oooooo 1000000 oooooooooooo 1000000000000 -1000000000001 5",
            "r26 pppppp 1000000 pppppppppppp 1000000000000 -1000000000001 5",
            "r27 qqqqqq 1000000 qqqqqqqqqqqq 1000000000000 -1000000000001 5",
            "r28 rrrrrr 1000000 rrrrrrrrrrrr 1000000000000 -1000000000001 5",
            "r29 ssssss 1000000 ssssssssssss 1000000000000 -1000000000001 5",
            "r30 tttttt 1000000 tttttttttttt 1000000000000 -1000000000001 5",
            "r31 uuuuuu 1000000 uuuuuuuuuuuu 1000000000000 -1000000000001 5",
            "r32 vvvvvv 1000000 vvvvvvvvvvvv 1000000000000 -1000000000001 5",
            "r33 wwwwww 1000000 wwwwwwwwwwww 1000000000000 -1000000000001 5",
            "r34 xxxxxx 1000000 xxxxxxxxxxxx 1000000000000 -1000000000001 5",
            "r35 yyyyyy 1000000 yyyyyyyyyyyy 1000000000000 -1000000000001 5",
            "r36 zzzzzz 1000000 zzzzzzzzzzzz 1000000000000 -1000000000001 5",
            "dec 999999999 1000000000 999999999999999999 1000000000000000000 9999999999999999999 10000000000000000000 999999999999999999999999999 1000000000000000000000000000 999999999999999999999999999999999999 1000000000000000000000000000000000000",
            "mp 1 9e8da70c67b1472aa 6b69cdfa7fee76461d 3308aeec8e62e46f82 68609f8093ad6b9bd0 4e2e897bb0681debf9 b14f084b43e677c10b 3d3ccc8703cebab08a 720f506318f24aff1c c72eaaa1f86fdf426e 75c36521611bfe684c 845512a832695f45ec 2d392bcfd604cce7ef",
            "ab0 ffffffff fffffffe0000000280000000",
            "ab1 ffffffff 3fffffffd0000000080000000",
            "ab2 7fffffff 2fffffffefffffffefffffffe",
            "ab3 ffffffff 800000017ffffffec000000200000001",
            "ab4 fffffffffffffffe c00000030000000a80000003",
            "ab5 ffffffff 4000000000000003ffffffff",
        ]
    );
}
