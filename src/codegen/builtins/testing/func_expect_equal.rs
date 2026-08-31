//! `expectEqual` — the general equality assertion.

use crate::codegen::registry::RegistryPackage;
use crate::types::ParameterType;

use super::{assertion, operands};

const INTRO: &str = r#"Assert `actual` equals `expected` (generic)."#;

const DESC: &str = r#"`expectEqual` passes when `actual` equals `expected` and fails the case
otherwise. It is the assertion to reach for when you simply want two values to
match and do not care about pinning their type.

The comparison is the language's own `=`, so the two operands must be comparable
with it, and `Integer` and `Float` compare numerically — `expectEqual(1, 1.0)`
passes. When that is not what you want, use `expectInteger` or `expectFloat`,
which also require the operand to be exactly that type.

Both operands must be printable — a number, `String`, `Byte`, or `List OF Byte`
— because a failure has to show them. Comparing something unprintable is a
compile error, not a failing test.

On failure the case stops there: later lines in the same `TCASE` do not run,
while sibling cases and groups carry on. The report names the case `[F]` and
prints the mismatch with its source line.

Like every assertion here, `expectEqual` is written as a bare name and is valid
only inside a `TCASE` body."#;

const EX: &str = r#"Assert a function returns what you expect:

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
    TCASE "adds two numbers"
      expectEqual(add(2, 3), 5)
    END TCASE
  END TGROUP
END TESTING
```

Running `mfb test` reports:

```
* arithmetic
  * [P] adds two numbers

Tests: 1  Pass: 1  Fail: 0
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(assertion(
        "expectEqual",
        (INTRO, DESC, EX),
        operands(ParameterType::var("T")),
    ));
}
