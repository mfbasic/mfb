//! `expectNEqual` — the general inequality assertion.

use crate::codegen::registry::RegistryPackage;
use crate::types::ParameterType;

use super::{assertion, operands};

const INTRO: &str = r#"Assert `actual` does not equal `expected` (generic)."#;

const DESC: &str = r#"`expectNEqual` passes when `actual` differs from `expected` and fails the case
when they are equal. It is `expectEqual`'s opposite and carries the same rules.

The comparison is the language's own `<>`, so the two operands must be
comparable with it, and `Integer` and `Float` compare numerically —
`expectNEqual(1, 1.0)` **fails**, because those are equal. Use `expectNInteger`
or `expectNFloat` when the operand's type matters too.

Both operands must be printable — a number, `String`, `Byte`, or `List OF Byte`
— because a failure has to show them.

Prefer asserting what a value *is* over what it is not, where you can:
`expectEqual` pins one answer, while `expectNEqual` passes for every value but
one. It earns its place when the exact result is unspecified but one particular
result would be a bug — a generated identifier that must not repeat, say."#;

const EX: &str = r#"Assert two identifiers are not the same:

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
    TCASE "does not return the wrong sum"
      expectNEqual(add(2, 3), 6)
    END TCASE
  END TGROUP
END TESTING
```

Running `mfb test` reports:

```
* arithmetic
  * [P] does not return the wrong sum

Tests: 1  Pass: 1  Fail: 0
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(assertion(
        "expectNEqual",
        (INTRO, DESC, EX),
        operands(ParameterType::var("T")),
    ));
}
