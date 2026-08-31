//! `toInt` — convert a value to an `Integer`.

use crate::codegen::registry::RegistryPackage;
use crate::types::ParameterType;

use super::{member, opt, req, TO_INT};

const INTRO: &str = "Convert a value to an Integer.";

const DESC: &str = r#"`toInt` converts a value to an `Integer`. It is written as a bare name, with no
`IMPORT` and no package prefix.

From a **`String`** it parses. Base 10 unless you pass `base`, which lets you
read hexadecimal (`toInt("ff", 16)` is `255`), octal, or binary. `base` must be
from 2 through 36; outside that range raises `ErrInvalidFormat`, as does text
that is not a whole number in the base given — including the empty string.

**`isNumeric` is not a safe guard for `toInt`.** It accepts decimal text, so
`isNumeric("1.5")` is `TRUE` while `toInt("1.5")` raises `ErrInvalidFormat`:
`toInt` parses whole numbers only, and does not round or truncate text. Either
`TRAP` the conversion, or go through `toFloat` and then `toInt` if a decimal
should be accepted and truncated.

From a **`Float`** or a `Fixed` it **truncates toward zero**: `toInt(1.9)` is
`1` and `toInt(-1.9)` is `-1`. It does not round. When you want the nearest
integer, round first with `math::round`.

A value too large for a 64-bit `Integer` raises `ErrOverflow`.

`toInt` is one of the fallible conversions, so its result auto-propagates like
any other call: on bad input it routes to your `TRAP`, or fails to your caller.
It never returns a sentinel such as `-1` or `0` to mean "could not convert"."#;

const EX: &str = r#"Parse in base 10 and base 16, and see truncation:

```
IMPORT io

SUB main()
  io::print(toString(toInt("42")))
  io::print(toString(toInt("ff", 16)))
  io::print(toString(toInt(1.9)))
  io::print(toString(toInt(-1.9)))
END SUB
```

prints:

```
42
255
1
-1
```

Bad text raises rather than returning a sentinel:

```
IMPORT io

SUB main()
  io::print(toString(toInt("not a number")))
  EXIT SUB
TRAP(err)
  io::print("toInt raised " & toString(err.code))
  EXIT SUB
END TRAP
END SUB
```

prints:

```
toInt raised 77050003
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(member(
        TO_INT,
        (INTRO, DESC, EX),
        ParameterType::Integer,
        vec![],
        vec![
            req(
                "value",
                ParameterType::String,
                "The text or number to convert. Text is parsed; a `Float` or `Fixed` truncates toward zero.",
            ),
            opt(
                "base",
                &[],
                ParameterType::Integer,
                "The base to parse text in, from 2 through 36. Defaults to 10; pass 16 for hexadecimal, 8 for octal, 2 for binary. Outside 2 through 36 raises `ErrInvalidFormat`.",
            ),
        ],
    ));
}
