//! `isNegative` — whether a number is less than zero.

use crate::codegen::registry::RegistryPackage;
use crate::types::ParameterType;

use super::{member, req, IS_NEGATIVE};

const INTRO: &str = "Whether a number is less than zero.";

const DESC: &str = r#"`isNegative` answers whether `value` is less than zero. It is written as a bare
name, with no `IMPORT` and no package prefix.

**Zero is not negative.** Together with `isPositive` and `isZero` it partitions
the numbers into three non-overlapping cases, so "not positive" and "negative"
are different tests and disagree exactly at zero.

It accepts any of the numeric types, not just `Integer`.

`isNegative` never fails and changes nothing. To turn a negative into its
magnitude, use `math::abs`."#;

const EX: &str = r#"Reject a negative quantity:

```
IMPORT io

SUB main()
  LET quantity AS Integer = -3
  IF isNegative(quantity) THEN
    io::print("quantity cannot be negative")
  ELSE
    io::print("ordering " & toString(quantity))
  END IF
END SUB
```

prints:

```
quantity cannot be negative
```

Zero belongs to neither side:

```
IMPORT io

SUB main()
  io::print(toString(isNegative(-1)))
  io::print(toString(isNegative(0)))
  io::print(toString(isNegative(1)))
END SUB
```

prints:

```
TRUE
FALSE
FALSE
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(member(
        IS_NEGATIVE,
        (INTRO, DESC, EX),
        ParameterType::Boolean,
        vec![],
        vec![req(
            "value",
            ParameterType::Integer,
            "The number to test. Zero is not negative.",
        )],
    ));
}
