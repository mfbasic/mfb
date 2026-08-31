//! `isPositive` — whether a number is greater than zero.

use crate::codegen::registry::RegistryPackage;
use crate::types::ParameterType;

use super::{member, req, IS_POSITIVE};

const INTRO: &str = "Whether a number is greater than zero.";

const DESC: &str = r#"`isPositive` answers whether `value` is greater than zero. It is written as a
bare name, with no `IMPORT` and no package prefix.

**Zero is not positive.** `isPositive`, `isNegative` and `isZero` partition the
numbers into three cases with no overlap and no gap, so testing "is it positive"
is not the same as testing "is it not negative" — for zero the two disagree.
That is the whole reason the third predicate exists.

It accepts any of the numeric types, not just `Integer`.

`isPositive` never fails and changes nothing."#;

const EX: &str = r#"The three predicates partition the numbers:

```
IMPORT io

SUB main()
  io::print("isPositive(1) = " & toString(isPositive(1)))
  io::print("isPositive(0) = " & toString(isPositive(0)))
  io::print("isNegative(0) = " & toString(isNegative(0)))
  io::print("isZero(0)     = " & toString(isZero(0)))
END SUB
```

prints:

```
isPositive(1) = TRUE
isPositive(0) = FALSE
isNegative(0) = FALSE
isZero(0)     = TRUE
```

Guard a division:

```
IMPORT io

SUB main()
  LET divisor AS Integer = 0
  IF isPositive(divisor) THEN
    io::print(toString(100 DIV divisor))
  ELSE
    io::print("need a positive divisor")
  END IF
END SUB
```

prints:

```
need a positive divisor
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(member(
        IS_POSITIVE,
        (INTRO, DESC, EX),
        ParameterType::Boolean,
        vec![],
        vec![req(
            "value",
            ParameterType::Integer,
            "The number to test. Zero is not positive.",
        )],
    ));
}
