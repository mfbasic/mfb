//! `isEven` — whether an `Integer` is even.

use crate::codegen::registry::RegistryPackage;
use crate::types::ParameterType;

use super::{member, req, IS_EVEN};

const INTRO: &str = "Whether an Integer is even.";

const DESC: &str = r#"`isEven` answers whether `value` divides by two exactly. It is written as a bare
name, with no `IMPORT` and no package prefix.

It reads better than the arithmetic it replaces — `IF isEven(row) THEN` rather
than `IF row MOD 2 = 0 THEN` — and it gets negative numbers right, which the
hand-written form does not always: `isEven(-3)` is `FALSE` and `isEven(-4)` is
`TRUE`, following the sign-independent meaning of even rather than whatever
`MOD` returns for a negative operand.

Zero is even.

`isEven` never fails and changes nothing. Its opposite is `isOdd`."#;

const EX: &str = r#"Alternate row styling:

```
IMPORT io

SUB main()
  FOR row = 0 TO 3
    IF isEven(row) THEN
      io::print(toString(row) & ": shaded")
    ELSE
      io::print(toString(row) & ": plain")
    END IF
  NEXT
END SUB
```

prints:

```
0: shaded
1: plain
2: shaded
3: plain
```

Negatives follow the sign-independent meaning:

```
IMPORT io

SUB main()
  io::print(toString(isEven(4)))
  io::print(toString(isEven(-3)))
  io::print(toString(isEven(-4)))
  io::print(toString(isEven(0)))
END SUB
```

prints:

```
TRUE
FALSE
TRUE
TRUE
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(member(
        IS_EVEN,
        (INTRO, DESC, EX),
        ParameterType::Boolean,
        vec![],
        vec![req(
            "value",
            ParameterType::Integer,
            "The number to test. Zero is even, and negatives follow the sign-independent meaning.",
        )],
    ));
}
