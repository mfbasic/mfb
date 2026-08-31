//! `expectString` — typed equality on `String`.

use crate::codegen::registry::RegistryPackage;
use crate::types::ParameterType;

use super::{assertion, operands};

const INTRO: &str = r#"Assert two `String` values are equal."#;

const DESC: &str = r#"`expectString` passes when both operands are `String` **and** they are equal.

The comparison is exact, scalar for scalar, with no case folding and no Unicode
normalization. Two strings that look identical on screen can still differ — `é`
written as one scalar and `é` written as `e` plus a combining accent are not
equal here. When that distinction is not the thing under test, normalize both
sides first with `strings::normalizeNfc`, or compare case-insensitively with
`strings::caseFold`.

Unlike `expectEqual`, this requires both operands to be exactly `String`, so
passing something that merely converts to one is a compile error rather than a
silently passing test.

Written as a bare name, valid only inside a `TCASE` body. For the inequality
form, see `expectNString`."#;

const EX: &str = r#"Assert a string transformation:

```
IMPORT io
IMPORT strings

SUB main()
  io::print(strings::upper("ab"))
END SUB

TESTING
  TGROUP "strings"
    TCASE "upper uppercases"
      expectString(strings::upper("ab"), "AB")
    END TCASE
    TCASE "comparing normalized forms"
      expectString(strings::normalizeNfc("abc"), "abc")
    END TCASE
  END TGROUP
END TESTING
```

Running `mfb test` reports:

```
* strings
  * [P] upper uppercases
  * [P] comparing normalized forms

Tests: 2  Pass: 2  Fail: 0
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(assertion(
        "expectString",
        (INTRO, DESC, EX),
        operands(ParameterType::String),
    ));
}
