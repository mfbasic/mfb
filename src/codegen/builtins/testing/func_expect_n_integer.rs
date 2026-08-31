//! `expectNInteger` — typed inequality on `Integer`.

use crate::codegen::registry::RegistryPackage;
use crate::types::ParameterType;

use super::{assertion, operands};

const INTRO: &str = r#"Assert two `Integer` values are not equal."#;

const DESC: &str = r#"`expectNInteger` passes when both operands are `Integer` **and** they differ. It
is `expectInteger`'s opposite, with the same type requirement: a `Float` operand
is a compile error, not a passing test.

Prefer `expectInteger` where you can. Asserting a value is not `0` passes for
every other integer there is, so it catches far less than asserting the value you
actually expect. `expectNInteger` earns its place when the exact result is
unspecified but one particular result would be a bug — a counter that must have
moved, an index that must not be the sentinel.

Written as a bare name, valid only inside a `TCASE` body."#;

const EX: &str = r#"Assert a counter moved, without pinning where to:

```
IMPORT io

FUNC nextId(current AS Integer) AS Integer
  RETURN current + 1
END FUNC

SUB main()
  io::print(toString(nextId(7)))
END SUB

TESTING
  TGROUP "identifiers"
    TCASE "nextId returns something new"
      expectNInteger(nextId(7), 7)
    END TCASE
  END TGROUP
END TESTING
```

Running `mfb test` reports:

```
* identifiers
  * [P] nextId returns something new

Tests: 1  Pass: 1  Fail: 0
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(assertion(
        "expectNInteger",
        (INTRO, DESC, EX),
        operands(ParameterType::Integer),
    ));
}
