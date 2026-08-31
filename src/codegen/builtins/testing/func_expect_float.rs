//! `expectFloat` — typed equality on `Float`.

use crate::codegen::registry::RegistryPackage;
use crate::types::ParameterType;

use super::{assertion, operands};

const INTRO: &str = r#"Assert two `Float` values are equal."#;

const DESC: &str = r#"`expectFloat` passes when both operands are `Float` **and** they are equal.

The comparison is exact. There is no tolerance and no epsilon, so two results
that differ in their last bit fail — which is usually what you want from a test,
but it does mean that asserting on the result of a chain of floating-point
arithmetic can be fragile. Where a computation is only expected to be close,
assert on a rounded value rather than the raw one.

Unlike `expectEqual`, which compares numerically across numeric types, this
requires both sides to be exactly `Float`: an `Integer` operand is a compile
error rather than a silently passing test.

Written as a bare name, valid only inside a `TCASE` body. For the inequality
form, see `expectNFloat`. For exact decimal money, `Money` values compare with
`expectEqual`; for deterministic fixed-point, use `expectFixed`."#;

const EX: &str = r#"Assert an exact floating-point result:

```
IMPORT io

FUNC half(x AS Float) AS Float
  RETURN x / 2.0
END FUNC

SUB main()
  io::print(toString(half(3.0)))
END SUB

TESTING
  TGROUP "arithmetic"
    TCASE "halves a float exactly"
      expectFloat(half(3.0), 1.5)
    END TCASE
  END TGROUP
END TESTING
```

Running `mfb test` reports:

```
* arithmetic
  * [P] halves a float exactly

Tests: 1  Pass: 1  Fail: 0
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(assertion(
        "expectFloat",
        (INTRO, DESC, EX),
        operands(ParameterType::Float),
    ));
}
