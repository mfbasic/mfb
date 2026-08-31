//! `toScalar` — convert a value to a `Scalar`.

use crate::codegen::registry::RegistryPackage;
use crate::types::ParameterType;

use super::{member, req, TO_SCALAR};

const INTRO: &str = "Convert a value to a Scalar (Unicode codepoint).";

const DESC: &str = r#"`toScalar` produces a `Scalar` — one Unicode scalar value, the unit `len` counts
a `String` in. It is written as a bare name, with no `IMPORT` and no package
prefix.

From an **`Integer`** it takes the codepoint: `toScalar(65)` is `A`. Valid
codepoints are `0` through `1114111` excluding the surrogate range `55296`
through `57343`, which is not a scalar value; anything else raises
`ErrInvalidArgument`.

From a **`String`** it takes that string's single scalar: `toScalar("A")` is the
same `A`. The string must hold **exactly one** — both `toScalar("ab")` and
`toScalar("")` raise `ErrInvalidArgument`. To take one scalar out of longer
text, use `strings::mid` or `strings::toScalars` first.

A `Scalar` renders through `toString` as the character it is, not as its number.
When you want the number, keep the `Integer` you started from, or reach for
`strings::toScalars` to get a whole string's worth.

`toScalar` is the bridge between codepoint arithmetic and text: build a
character from a computed codepoint, or pull one out of a string to compare
against a known value."#;

const EX: &str = r#"Build a character from its codepoint:

```
IMPORT io

SUB main()
  io::print(toString(toScalar(65)))
  io::print(toString(toScalar("A")))
END SUB
```

prints:

```
A
A
```

An invalid codepoint raises:

```
IMPORT io

SUB main()
  io::print(toString(toScalar(1114112)))
  EXIT SUB
TRAP(err)
  io::print("toScalar raised " & toString(err.code) & " — " & err.message)
  EXIT SUB
END TRAP
END SUB
```

prints:

```
toScalar raised 77050002 — Argument value is not valid for the requested operation.
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(member(
        TO_SCALAR,
        (INTRO, DESC, EX),
        ParameterType::named("Scalar"),
        vec!["ErrInvalidArgument"],
        vec![req(
            "value",
            ParameterType::Integer,
            "A codepoint (0 through 1114111, excluding the surrogate range) or a `String` holding exactly one scalar. Anything else raises `ErrInvalidArgument`.",
        )],
    ));
}
