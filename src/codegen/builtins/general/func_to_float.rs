//! `toFloat` — convert a value to a `Float`.

use crate::codegen::registry::RegistryPackage;
use crate::types::ParameterType;

use super::{member, req, TO_FLOAT};

const INTRO: &str = "Convert a value to a Float.";

const DESC: &str = r#"`toFloat` converts a value to a `Float`, MFBASIC's double-precision floating
point number. It is written as a bare name, with no `IMPORT` and no package
prefix.

From a **`String`** it parses a decimal number, with an optional sign and an
optional exponent. Text that is not a number — including the empty string —
raises `ErrInvalidFormat`, and a value outside the range a `Float` can hold
raises `ErrOverflow`. `isNumeric` answers first if you would rather test than
trap.

From an **`Integer`** the conversion is exact for values up to about 2^53; above
that, a `Float` cannot represent every integer and the result is the nearest one
it can.

Reach for `toFloat` for measurement and scientific quantities. For money, use
`toMoney` — `Float` cannot represent most decimal fractions exactly, so
accumulating currency in it drifts. For deterministic fixed-point, use
`toFixed`.

Note that rendering a `Float` with `toString` gives two decimal places by
default, so a round trip through text is not identity unless you pass a
`precision`."#;

const EX: &str = r#"Parse a decimal, and see what `toString` does to it:

```
IMPORT io

SUB main()
  io::print(toString(toFloat("1.5")))
  io::print(toString(toFloat("-2.5e3")))
END SUB
```

prints:

```
1.50
-2500.00
```

Test before converting, rather than trapping:

```
IMPORT io

SUB main()
  LET text AS String = "abc"
  IF isNumeric(text) THEN
    io::print(toString(toFloat(text)))
  ELSE
    io::print("not a number: " & text)
  END IF
END SUB
```

prints:

```
not a number: abc
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(member(
        TO_FLOAT,
        (INTRO, DESC, EX),
        ParameterType::Float,
        vec!["ErrOverflow", "ErrInvalidFormat"],
        vec![req(
            "value",
            ParameterType::String,
            "The text or number to convert. Text is parsed as a decimal, with an optional sign and exponent.",
        )],
    ));
}
