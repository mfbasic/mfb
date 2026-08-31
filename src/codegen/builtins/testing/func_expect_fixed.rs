//! `expectFixed` — typed equality on `Fixed`.

use crate::codegen::registry::RegistryPackage;
use crate::types::ParameterType;

use super::{assertion, operands};

const INTRO: &str = r#"Assert two `Fixed` values are equal."#;

const DESC: &str = r#"`expectFixed` passes when both operands are `Fixed` **and** they are equal.

`Fixed` arithmetic is deterministic across platforms, so an exact comparison is
reasonable here in a way it is not always for `Float`: the same computation gives
the same bits everywhere, and a test that passes on one machine passes on all of
them.

Remember that `Fixed` is binary fixed-point, not decimal: most decimal fractions
are rounded to the nearest representable value on the way in. Writing
`expectFixed(x, 0.1)` compares against whatever `0.1` rounds to, not against
exact one-tenth. For exact decimal arithmetic use `Money` instead.

There is no `Fixed` literal suffix. Give the expected value a `Fixed` annotation
(`LET want AS Fixed = 1.5`) or convert with `toFixed`, then pass that — a bare
decimal literal in the call is a `Float` and will not type-check here.

Written as a bare name, valid only inside a `TCASE` body. For the inequality
form, see `expectNFixed`."#;

const EX: &str = r#"Assert a `Fixed` result, annotating the expected value:

```
IMPORT io

FUNC scale(x AS Fixed) AS Fixed
  RETURN x * 3
END FUNC

SUB main()
  LET start AS Fixed = 0.5
  io::print(toString(scale(start)))
END SUB

TESTING
  TGROUP "fixed-point"
    TCASE "scales a fixed value"
      LET start AS Fixed = 0.5
      LET want AS Fixed = 1.5
      expectFixed(scale(start), want)
    END TCASE
  END TGROUP
END TESTING
```

Running `mfb test` reports:

```
* fixed-point
  * [P] scales a fixed value

Tests: 1  Pass: 1  Fail: 0
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(assertion(
        "expectFixed",
        (INTRO, DESC, EX),
        operands(ParameterType::Fixed),
    ));
}
