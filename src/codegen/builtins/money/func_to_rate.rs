//! `money::toRate` — build a `money::Rate` from text or a whole number.
//!
//! The two overloads live in `func_to_rate_string` and `func_to_rate_integer`;
//! this file carries the shared documentation and registers them as one member.

use super::{func_to_rate_integer, func_to_rate_string};
use crate::codegen::registry::{RegistryFunction, RegistryPackage};

const INTRO: &str = r#"Build an exact `money::Rate` from decimal text or a whole number."#;

const DESC: &str = r#"`money::toRate(text)` reads a decimal rate exactly. The text is digits, then
optionally a `.` and more digits: `"0.842"`, `"1"`, `"0.0067"`. There is no binary
step, so the rate is exactly the number written.

`money::toRate(value)` makes the whole-number rate `value`: `money::toRate(1)` is the
rate 1.

A rate holds at most 12 decimal places and must be above zero and at most
`9223372.036854775807`.

- More than 12 decimal places raises `ErrInvalidFormat` (`77050003`). Nothing is rounded, so `"0.0000000000001"` is rejected rather than read as zero. Trailing zeros count as places.
- Text that is not a decimal number raises `ErrInvalidFormat`: an empty string, spaces, a `+`, an exponent such as `1e3`, or a `.` without a digit on each side.
- A rate of zero or below raises `ErrInvalidArgument` (`77050002`), for text such as `"0"` or `"-1"` and for `money::toRate(0)`.
- A rate above `9223372.036854775807` raises `ErrOverflow` (`77050010`).
- `ErrOutOfMemory` means the program ran out of memory.

There is no `Float` or `Fixed` overload: both are binary, and converting one would
store a rate slightly different from the one written."#;

const EX: &str = r#"Read rates from text and print them back:

```
IMPORT io
IMPORT money

FUNC main() AS Nothing
  LET eur AS money::Rate = money::toRate("0.842")
  io::print(toString(eur))
  io::print(toString(eur.scaled))
  io::print(toString(money::toRate(64)))
END FUNC
```

Text with too many places is rejected, not rounded:

```
IMPORT errorCode
IMPORT io
IMPORT money

FUNC main() AS Nothing
  LET r AS money::Rate = money::toRate("0.0000000000001") TRAP(err)
    io::print(toString(err.code = errorCode::ErrInvalidFormat))
    RECOVER money::toRate(1)
  END TRAP
  io::print(toString(r))
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "toRate",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![
            func_to_rate_string::implementation(),
            func_to_rate_integer::implementation(),
        ],
    });
}
