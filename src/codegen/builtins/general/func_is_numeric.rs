//! `isNumeric` — whether a `String` parses as a number.

use crate::codegen::registry::RegistryPackage;
use crate::types::ParameterType;

use super::{member, req, IS_NUMERIC};

const INTRO: &str = "Whether a String parses as a number.";

const DESC: &str = r#"`isNumeric` answers whether `value` would parse as a number, without parsing it.
It is written as a bare name, with no `IMPORT` and no package prefix.

It is the companion to the fallible conversions. `toInt`, `toFloat`, `toFixed`
and `toMoney` all **raise** on text that is not a number rather than returning a
sentinel, so when bad input is an ordinary, expected thing — a field a user
typed, a column from a file — `isNumeric` lets you branch instead of trap.

The **empty string is not numeric**, which is the case worth remembering: an
empty input field answers `FALSE` here rather than converting to `0`.

Both integers and decimals count, so `isNumeric("12")` and `isNumeric("1.5")`
are both `TRUE`.

`isNumeric` never fails and changes nothing. It answers about base-10 text — it
does not know about the `base` argument you can pass `toInt`, so a hexadecimal
string like `"ff"` is not numeric by this test even though `toInt("ff", 16)`
succeeds."#;

const EX: &str = r#"Branch on whether input is a number, rather than trapping:

```
IMPORT io

SUB main()
  io::print(toString(isNumeric("12")))
  io::print(toString(isNumeric("1.5")))
  io::print(toString(isNumeric("abc")))
  io::print(toString(isNumeric("")))
END SUB
```

prints:

```
TRUE
TRUE
FALSE
FALSE
```

Use it to guard a conversion:

```
IMPORT io

SUB main()
  LET field AS String = ""
  IF isNumeric(field) THEN
    io::print("got " & toString(toInt(field)))
  ELSE
    io::print("please enter a number")
  END IF
END SUB
```

prints:

```
please enter a number
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(member(
        IS_NUMERIC,
        (INTRO, DESC, EX),
        ParameterType::Boolean,
        vec![],
        vec![req(
            "value",
            ParameterType::String,
            "The text to test. The empty string is not numeric.",
        )],
    ));
}
