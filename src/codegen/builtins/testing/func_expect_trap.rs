//! `expectTrap` — assert an expression fails.

use crate::codegen::registry::{DefaultValue, Parameter, RegistryPackage};
use crate::types::ParameterType;

use super::assertion;

const INTRO: &str =
    r#"Assert evaluating an expression traps (optionally with a given error code)."#;

const DESC: &str = r#"`expectTrap` passes when evaluating `expression` fails, and fails the case when
it succeeds. It is how you test the unhappy path: that bad input really is
rejected, rather than quietly producing a wrong answer.

Given a `code`, it is stricter — the expression must fail **and** the failure's
`error.code` must be exactly that code. That turns a vague "something went
wrong" into a real assertion about *which* thing went wrong, and is worth doing
whenever a function can fail in more than one way. The codes have names:
`errorCode::ErrIndexOutOfRange` reads better than `77050001` and is the same
value.

`expression` has to be a call — that is what there is to fail. A bare value or a
constant is a compile error, because nothing in it could ever trap.

If the call turns out to be one that cannot fail, nothing special happens:
`expectTrap` simply fails at run time, since no trap occurred.

Note that `expectTrap` swallows the failure it is testing for, so the case
continues afterwards. Written as a bare name, valid only inside a `TCASE` body.
For the opposite, see `expectNTrap`."#;

const EX: &str = r#"Assert that an out-of-range slice is rejected:

```
IMPORT io
IMPORT strings

SUB main()
  io::print(strings::mid("abc", 0, 2))
END SUB

TESTING
  TGROUP "strings"
    TCASE "mid raises rather than clamping"
      expectTrap(strings::mid("abc", 0, 99))
    END TCASE
  END TGROUP
END TESTING
```

Pin the exact failure, so a different error would not pass the test:

```
IMPORT io
IMPORT strings

SUB main()
  io::print(strings::mid("abc", 0, 2))
END SUB

TESTING
  TGROUP "strings"
    TCASE "mid raises the out-of-range error specifically"
      expectTrap(strings::mid("abc", 0, 99), 77050001)
    END TCASE
  END TGROUP
END TESTING
```

Running `mfb test` reports:

```
* strings
  * [P] mid raises the out-of-range error specifically

Tests: 1  Pass: 1  Fail: 0
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    // `expectTrap(expr)` / `expectTrap(expr, code)`: a guardable expression plus an
    // optional expected `error.code`. The `code` slot is `Optional` — it widens arity
    // to (1, 2) but is not default-padded (the desugar selects the trap-with-code body
    // by argument count).
    pkg.add_function(assertion(
        "expectTrap",
        (INTRO, DESC, EX),
        vec![
            Parameter {
                name: "expression",
                desc: "The call asserted to fail. It must be a call — a bare value has nothing that could fail.",
                aliases: &[],
                ty: ParameterType::var("T"),
                default: DefaultValue::None,
            },
            Parameter {
                name: "code",
                desc: "The exact `error.code` the failure must carry. Omit it to accept any failure.",
                aliases: &[],
                ty: ParameterType::Integer,
                default: DefaultValue::Optional,
            },
        ],
    ));
}
