//! `toFixed` — convert a value to a `Fixed`.

use crate::codegen::registry::RegistryPackage;
use crate::types::ParameterType;

use super::{member, req, TO_FIXED};

const INTRO: &str = "Convert a value to a Fixed.";

const DESC: &str = r#"`toFixed` converts a value to a `Fixed`, MFBASIC's binary fixed-point number. It
is written as a bare name, with no `IMPORT` and no package prefix.

From a **`String`** it parses a decimal number. Text that is not one raises
`ErrInvalidFormat`, and a value outside the range a `Fixed` can hold — roughly
±2.1 billion — raises `ErrOverflow`.

`Fixed` is **binary** fixed-point, so most decimal fractions are rounded to the
nearest value it can hold on the way in: `toFixed("0.1")` gives the closest
`Fixed` to one-tenth, not exact one-tenth. What it does guarantee is
**determinism** — the same arithmetic gives the same answer on every platform,
which `Float` does not promise. That makes it right for simulation, geometry,
and anywhere a replay has to match.

For money, use `toMoney` instead: it is exact decimal arithmetic and does not
round your inputs.

`toFixed` is also the way to get a `Fixed` at all where a literal will not do —
there is no `Fixed` suffix, so the alternative is an annotated binding
(`LET x AS Fixed = 1.5`)."#;

const EX: &str = r#"Parse a decimal into a `Fixed`:

```
IMPORT io

SUB main()
  LET x AS Fixed = toFixed("1.5")
  io::print(toString(x))
END SUB
```

prints:

```
1.50
```

Text that is not a number raises rather than converting to a value:

```
IMPORT io

SUB main()
  LET x AS Fixed = toFixed("nope")
  io::print(toString(x))
  EXIT SUB
TRAP(err)
  io::print("toFixed raised " & toString(err.code))
  EXIT SUB
END TRAP
END SUB
```

prints:

```
toFixed raised 77050003
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(member(
        TO_FIXED,
        (INTRO, DESC, EX),
        ParameterType::Fixed,
        vec!["ErrOverflow", "ErrInvalidFormat"],
        vec![req(
            "value",
            ParameterType::String,
            "The text or number to convert. A decimal fraction is rounded to the nearest representable `Fixed`.",
        )],
    ));
}
