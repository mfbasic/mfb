//! `bits::sl`, `sr` and `sra` take three forms, chosen by what is known about the count.
//!
//! A shift count must be `0..=63` or the call raises `ErrInvalidArgument`. The lowering
//! picks the cheapest form the count's shape PROVES safe (bug-774):
//!
//!   * a literal count in range: an immediate shift, no check at all;
//!   * a count masked by `bits::band(x, m)` with `m` a literal in `0..=63` (either
//!     operand order): the mask clears the sign bit and every bit above `m`'s, so the
//!     count is in `0..=m` whatever `x` is — a variable shift with the range check
//!     DROPPED;
//!   * any other count: a variable shift behind an explicit `0..=63` check.
//!
//! The check is a branch to a `bits_shift_out_of_range` label, so the number of those
//! labels in a function is the number of shifts that kept their check: a mask that was
//! not recognized would keep one and cost a compare and branch per shift, and a check
//! dropped for a count nothing proved would turn an out-of-range shift into a hardware
//! shift by `n MOD 64`.

use crate::codegen::engine::tests::test_support::Stream;
use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_src_cached, code_function, CodeTarget};

const SHIFTS: &str = "\
IMPORT bits
IMPORT io

FUNC literal(x AS Integer) AS Integer
  RETURN bits::sl(x, 3) + bits::sr(x, 3) + bits::sra(x, 3)
END FUNC

FUNC masked(x AS Integer, n AS Integer) AS Integer
  RETURN bits::sl(x, bits::band(n, 7)) + bits::sr(x, bits::band(7, n)) + bits::sra(x, bits::band(n, 63))
END FUNC

FUNC unproven(x AS Integer, n AS Integer) AS Integer
  RETURN bits::sl(x, n) + bits::sr(x, n) + bits::sra(x, n)
END FUNC

FUNC wide(x AS Integer, n AS Integer) AS Integer
  RETURN bits::sl(x, bits::band(n, 255)) + bits::sra(x, bits::band(n, 255))
END FUNC

FUNC main() AS Integer
  io::print(toString(literal(1) + masked(1, 2) + unproven(1, 2) + wide(1, 2)))
  LET keepLiteral = literal
  LET keepUnproven = unproven
  RETURN 0
END FUNC
";

fn range_checks(function: &str, target: CodeTarget) -> usize {
    let plan = code_for_src_cached(SHIFTS, target, Console);
    Stream::of(code_function(plan, function))
        .labels()
        .iter()
        .filter(|(_, name)| name.starts_with("bits_shift_out_of_range"))
        .count()
}

#[test]
fn only_a_count_nothing_proves_keeps_its_range_check() {
    for target in [CodeTarget::LinuxX86_64, CodeTarget::MacosAarch64] {
        assert_eq!(
            range_checks("literal", target),
            0,
            "{}: an in-range literal needs no check",
            target.name()
        );
        assert_eq!(
            range_checks("masked", target),
            0,
            "{}: a count masked into 0..=63 is proven in range, in either operand order",
            target.name()
        );
        assert_eq!(
            range_checks("unproven", target),
            3,
            "{}: sl, sr and sra of an unproven count each keep their check",
            target.name()
        );
        assert_eq!(
            range_checks("wide", target),
            2,
            "{}: a mask past 63 proves nothing about the range",
            target.name()
        );
    }
}

#[test]
fn every_shift_form_lowers_on_every_backend() {
    for target in CodeTarget::ALL {
        let result = crate::testutil::try_code_for_src(SHIFTS, target, Console);
        assert!(result.is_ok(), "{}: {:?}", target.name(), result.err());
    }
}
