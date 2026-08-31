//! `expectInteger` — typed equality on `Integer`.

use crate::codegen::registry::RegistryPackage;
use crate::types::ParameterType;

use super::{assertion, operands};

const INTRO: &str = r#"Assert two `Integer` values are equal."#;

const DESC: &str = r#"`expectInteger` passes when both operands are `Integer` **and** they are equal.

The difference from `expectEqual` is the type check. `expectEqual` compares
numerically across numeric types, so `expectEqual(1, 1.0)` passes;
`expectInteger` requires both sides to be exactly `Integer`, and a `Float`
operand is a compile error rather than a silently passing test. Reach for this
one whenever a wrong type would itself be a bug in the code under test.

On failure the case stops there and the report shows the expected and actual
values with the source line.

Written as a bare name, valid only inside a `TCASE` body. For the inequality
form, see `expectNInteger`; for the other typed forms, `expectFloat`,
`expectFixed`, and `expectString`."#;

const EX: &str = r#"Pin both the type and the value of a result:

```
IMPORT io

FUNC add(a AS Integer, b AS Integer) AS Integer
  RETURN a + b
END FUNC

SUB main()
  io::print(toString(add(2, 3)))
END SUB

TESTING
  TGROUP "arithmetic"
    TCASE "adds two integers"
      expectInteger(add(2, 3), 5)
    END TCASE
  END TGROUP
END TESTING
```

Running `mfb test` reports:

```
* arithmetic
  * [P] adds two integers

Tests: 1  Pass: 1  Fail: 0
```

Had the expected value been wrong, the report names the case `[F]` and shows
both values with the line the assertion is on:

```
IMPORT io

FUNC add(a AS Integer, b AS Integer) AS Integer
  RETURN a + b
END FUNC

SUB main()
  io::print(toString(add(2, 3)))
END SUB

TESTING
  TGROUP "arithmetic"
    TCASE "adds two integers"
      expectInteger(add(2, 3), 99)
    END TCASE
  END TGROUP
END TESTING
```

Running `mfb test` reports:

```
* arithmetic
  * [F] adds two integers
    X expected 99, got 5  (src/main.mfb:14)

Tests: 1  Pass: 0  Fail: 1
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(assertion(
        "expectInteger",
        (INTRO, DESC, EX),
        operands(ParameterType::Integer),
    ));
}
