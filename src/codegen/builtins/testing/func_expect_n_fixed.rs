//! `expectNFixed` — typed inequality on `Fixed`.

use crate::codegen::registry::RegistryPackage;
use crate::types::ParameterType;

use super::{assertion, operands};

const INTRO: &str = r#"Assert two `Fixed` values are not equal."#;

const DESC: &str = r#"`expectNFixed` passes when both operands are `Fixed` **and** they differ. It is
`expectFixed`'s opposite, with the same type requirement and the same caveat
about literals: there is no `Fixed` suffix, so annotate the value you are
comparing against (`LET want AS Fixed = 1.5`) rather than writing a bare decimal,
which is a `Float`.

Because `Fixed` is binary fixed-point, two decimals that differ in the source can
round to the *same* `Fixed` value — in which case this assertion fails even
though the literals look different. That is the same rounding `expectFixed`
relies on, seen from the other side.

Prefer `expectFixed` where you can.

Written as a bare name, valid only inside a `TCASE` body."#;

const EX: &str = r#"Assert two fixed-point values are distinct:

```
IMPORT io

SUB main()
  LET a AS Fixed = 1.0
  io::print(toString(a))
END SUB

TESTING
  TGROUP "fixed-point"
    TCASE "one and two differ"
      LET a AS Fixed = 1.0
      LET b AS Fixed = 2.0
      expectNFixed(a, b)
    END TCASE
  END TGROUP
END TESTING
```

Running `mfb test` reports:

```
* fixed-point
  * [P] one and two differ

Tests: 1  Pass: 1  Fail: 0
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(assertion(
        "expectNFixed",
        (INTRO, DESC, EX),
        operands(ParameterType::Fixed),
    ));
}
