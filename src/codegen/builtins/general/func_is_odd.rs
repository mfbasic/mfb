//! `isOdd` — whether an `Integer` is odd.

use crate::codegen::registry::RegistryPackage;
use crate::types::ParameterType;

use super::{member, req, IS_ODD};

const INTRO: &str = "Whether an Integer is odd.";

const DESC: &str = r#"`isOdd` answers whether `value` leaves a remainder when divided by two. It is
written as a bare name, with no `IMPORT` and no package prefix.

It is exactly the negation of `isEven`, and exists so the intent reads
positively where that is what you mean. Like `isEven` it treats negatives by the
sign-independent meaning, so `isOdd(-3)` is `TRUE`.

Zero is not odd.

`isOdd` never fails and changes nothing."#;

const EX: &str = r#"Filter to the odd numbers:

```
IMPORT io

SUB main()
  FOR n = 1 TO 6
    IF isOdd(n) THEN
      io::print(toString(n))
    END IF
  NEXT
END SUB
```

prints:

```
1
3
5
```

Including across zero and the negatives:

```
IMPORT io

SUB main()
  io::print(toString(isOdd(4)))
  io::print(toString(isOdd(-3)))
  io::print(toString(isOdd(0)))
END SUB
```

prints:

```
FALSE
TRUE
FALSE
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(member(
        IS_ODD,
        (INTRO, DESC, EX),
        ParameterType::Boolean,
        vec![],
        vec![req(
            "value",
            ParameterType::Integer,
            "The number to test. Zero is not odd, and negatives follow the sign-independent meaning.",
        )],
    ));
}
