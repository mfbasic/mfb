//! `expectNString` — typed inequality on `String`.

use crate::codegen::registry::RegistryPackage;
use crate::types::ParameterType;

use super::{assertion, operands};

const INTRO: &str = r#"Assert two `String` values are not equal."#;

const DESC: &str = r#"`expectNString` passes when both operands are `String` **and** they differ. It is
`expectString`'s opposite, with the same type requirement.

The comparison is exact, scalar for scalar, with no case folding and no
normalization — which makes this assertion easy to pass by accident. `"Abc"` and
`"abc"` differ; so do the one-scalar `é` and the `e`-plus-combining-accent `é`,
which look identical on screen. If the difference you mean is a real one rather
than a spelling, normalize with `strings::normalizeNfc` or fold case with
`strings::caseFold` on both sides first.

Prefer `expectString` where you can — asserting a string is not one particular
value passes for every other string there is.

Written as a bare name, valid only inside a `TCASE` body."#;

const EX: &str = r#"Assert a transformation actually changed the text:

```
IMPORT io
IMPORT strings

SUB main()
  io::print(strings::upper("ab"))
END SUB

TESTING
  TGROUP "strings"
    TCASE "upper changes a lowercase string"
      expectNString(strings::upper("ab"), "ab")
    END TCASE
  END TGROUP
END TESTING
```

Running `mfb test` reports:

```
* strings
  * [P] upper changes a lowercase string

Tests: 1  Pass: 1  Fail: 0
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(assertion(
        "expectNString",
        (INTRO, DESC, EX),
        operands(ParameterType::String),
    ));
}
