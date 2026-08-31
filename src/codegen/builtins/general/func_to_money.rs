//! `toMoney` — convert a value to `Money`.

use crate::codegen::registry::RegistryPackage;
use crate::types::ParameterType;

use super::{member, req, TO_MONEY};

const INTRO: &str = "Convert a value to Money.";

const DESC: &str = r#"`toMoney` converts a value to `Money`, MFBASIC's exact decimal type. It is
written as a bare name, with no `IMPORT` and no package prefix.

From a **`String`** it parses a decimal amount. Text that is not one raises
`ErrInvalidFormat`, and an amount outside the range `Money` can hold raises
`ErrOverflow`.

**Convert from text, not from a `Float`.** `Money` is exact base-10 arithmetic —
one-tenth is one-tenth, and adding a hundred of them gives exactly ten. A
`Float` cannot represent most decimal fractions, so a value that reached a
`Float` first has already lost the exactness `Money` exists to preserve. Parse
the original text.

That is the whole reason to prefer `Money` over `Float` or `Fixed` for currency:
no drift over a long run of additions, and the totals a person checking your
arithmetic by hand will get.

The `money` package controls how `Money` rounds when a calculation needs to."#;

const EX: &str = r#"Parse an amount and add it up exactly:

```
IMPORT io

SUB main()
  MUT total AS Money = toMoney("0.00")
  LET tenth AS Money = toMoney("0.10")
  FOR i = 1 TO 10
    total = total + tenth
  NEXT
  io::print(toString(total))
END SUB
```

prints:

```
1.00
```

Bad text raises rather than converting to zero:

```
IMPORT io

SUB main()
  LET m AS Money = toMoney("free")
  io::print(toString(m))
  EXIT SUB
TRAP(err)
  io::print("toMoney raised " & toString(err.code))
  EXIT SUB
END TRAP
END SUB
```

prints:

```
toMoney raised 77050003
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(member(
        TO_MONEY,
        (INTRO, DESC, EX),
        ParameterType::Money,
        vec!["ErrOverflow", "ErrInvalidFormat"],
        vec![req(
            "value",
            ParameterType::String,
            "The amount to convert. Convert from the original text rather than from a `Float`, which has already lost exactness.",
        )],
    ));
}
