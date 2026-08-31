//! `expectNFloat` — typed inequality on `Float`.

use crate::codegen::registry::RegistryPackage;
use crate::types::ParameterType;

use super::{assertion, operands};

const INTRO: &str = r#"Assert two `Float` values are not equal."#;

const DESC: &str = r#"`expectNFloat` passes when both operands are `Float` **and** they differ. It is
`expectFloat`'s opposite, with the same type requirement: an `Integer` operand is
a compile error, not a passing test.

The comparison is exact, which cuts the other way here than it does for
`expectFloat`: two values that differ only in their last bit count as different,
so this assertion passes on a difference far too small to matter. If what you
mean is "meaningfully different", compare rounded values instead.

Prefer `expectFloat` where you can — asserting a value is not one particular
float passes for almost every float there is.

Written as a bare name, valid only inside a `TCASE` body."#;

const EX: &str = r#"Assert a computation moved the value at all:

```
IMPORT io

FUNC perturb(x AS Float) AS Float
  RETURN x + 1.0
END FUNC

SUB main()
  io::print(toString(perturb(1.0)))
END SUB

TESTING
  TGROUP "arithmetic"
    TCASE "perturb changes its input"
      expectNFloat(perturb(1.0), 1.0)
    END TCASE
  END TGROUP
END TESTING
```

Running `mfb test` reports:

```
* arithmetic
  * [P] perturb changes its input

Tests: 1  Pass: 1  Fail: 0
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(assertion(
        "expectNFloat",
        (INTRO, DESC, EX),
        operands(ParameterType::Float),
    ));
}
