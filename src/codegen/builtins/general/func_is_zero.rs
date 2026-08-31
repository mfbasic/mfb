//! `isZero` — whether a number equals zero.

use crate::codegen::registry::RegistryPackage;
use crate::types::ParameterType;

use super::{member, req, IS_ZERO};

const INTRO: &str = "Whether a number equals zero.";

const DESC: &str = r#"`isZero` answers whether `value` is exactly zero. It is written as a bare name,
with no `IMPORT` and no package prefix.

It is the third of the sign predicates: `isPositive`, `isNegative` and `isZero`
partition the numbers with no overlap and no gap. Zero is the case the other two
both answer `FALSE` for, which is why it needs its own test rather than being
inferred from them.

The comparison is exact, which is worth thinking about for a `Float`: a
computation that mathematically cancels to zero may land a hair away from it and
answer `FALSE` here. That is a property of floating point, not of this function
— for a tolerance test, compare the magnitude against a threshold you choose.
For `Integer`, `Fixed` and `Money` the question has no such ambiguity.

`isZero` never fails and changes nothing."#;

const EX: &str = r#"Guard a division by testing the divisor:

```
IMPORT io

SUB main()
  LET divisor AS Integer = 0
  IF isZero(divisor) THEN
    io::print("cannot divide by zero")
  ELSE
    io::print(toString(100 DIV divisor))
  END IF
END SUB
```

prints:

```
cannot divide by zero
```

Zero is the case the other two predicates both decline:

```
IMPORT io

SUB main()
  io::print(toString(isZero(0)))
  io::print(toString(isPositive(0)))
  io::print(toString(isNegative(0)))
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
        IS_ZERO,
        (INTRO, DESC, EX),
        ParameterType::Boolean,
        vec![],
        vec![req(
            "value",
            ParameterType::Integer,
            "The number to test. The comparison is exact, which for a `Float` means exactly zero and nothing near it.",
        )],
    ));
}
