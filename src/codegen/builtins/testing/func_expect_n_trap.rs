//! `expectNTrap` — assert an expression does not fail.

use crate::codegen::registry::{DefaultValue, Parameter, RegistryPackage};
use crate::types::ParameterType;

use super::assertion;

const INTRO: &str = r#"Assert evaluating an expression does not trap."#;

const DESC: &str = r#"`expectNTrap` passes when evaluating `expression` succeeds, and fails the case
when it raises. It asserts only that the call went through — it says nothing
about the value it produced.

Use it where "this input is accepted" is the thing under test, especially for
inputs near a boundary that used to be rejected. Where you also care about the
answer, assert on the value instead: `expectEqual` and the typed forms already
fail the case if the call raises, so wrapping them in `expectNTrap` adds nothing.

`expression` has to be a call — a bare value or a constant is a compile error,
because nothing in it could ever trap. If the call is one that cannot fail,
`expectNTrap` simply always passes.

Written as a bare name, valid only inside a `TCASE` body. For the opposite, see
`expectTrap`."#;

const EX: &str = r#"Assert that a boundary input is accepted:

```
IMPORT io
IMPORT strings

SUB main()
  io::print(strings::mid("abc", 0, 3))
END SUB

TESTING
  TGROUP "strings"
    TCASE "taking the whole string is in range"
      expectNTrap(strings::mid("abc", 0, 3))
    END TCASE
  END TGROUP
END TESTING
```

Running `mfb test` reports:

```
* strings
  * [P] taking the whole string is in range

Tests: 1  Pass: 1  Fail: 0
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(assertion(
        "expectNTrap",
        (INTRO, DESC, EX),
        vec![Parameter {
            name: "expression",
            desc: "The call asserted to succeed. It must be a call — a bare value has nothing that could fail.",
            aliases: &[],
            ty: ParameterType::var("T"),
            default: DefaultValue::None,
        }],
    ));
}
