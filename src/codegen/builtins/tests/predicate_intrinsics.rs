//! The general predicates (`isEven`, `isOdd`, `isPositive`, `isNegative`, `isZero`,
//! `isEmpty`, `isNotEmpty`) lower inline in the caller
//! (`engine/value/builder_values/lower_inner.rs`, the `RuntimeCall` intrinsics): a
//! predicate that fell back to a helper call would still answer correctly but cost a
//! call per test, and the helper symbol it named would have to exist.

use super::lowered::calls_on;
use crate::testutil::CodeTarget;

#[test]
fn the_general_predicates_lower_inline() {
    let source = "\
IMPORT io

FUNC main() AS Integer
  MUT n AS Integer = 4
  MUT f AS Float = 2.5
  MUT s AS String = \"abc\"
  MUT xs AS List OF Integer = []
  LET a AS Boolean = isEven(n)
  LET b AS Boolean = isOdd(n)
  LET c AS Boolean = isPositive(n)
  LET d AS Boolean = isNegative(f)
  LET e AS Boolean = isZero(n)
  LET g AS Boolean = isEmpty(xs)
  LET h AS Boolean = isNotEmpty(s)
  io::print(toString(a) & toString(b) & toString(c) & toString(d) & toString(e) & toString(g) & toString(h))
  RETURN 0
END FUNC
";
    for target in CodeTarget::ALL {
        let calls = calls_on(source, "main", target);
        for name in [
            "isEven",
            "isOdd",
            "isPositive",
            "isNegative",
            "isZero",
            "isEmpty",
            "isNotEmpty",
        ] {
            assert!(
                !calls.iter().any(|call| call.contains(name)),
                "{}: {name} is inline, not a call: {calls:?}",
                target.name()
            );
        }
    }
}
