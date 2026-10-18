//! `toFixed` — convert a value to a `Fixed`.

use crate::codegen::registry::RegistryPackage;
use crate::types::ParameterType;

use super::{member, req, TO_FIXED};

const INTRO: &str = "Convert a value to a Fixed.";

const DESC: &str = r#"`toFixed` converts a value to a `Fixed`, MFBASIC's binary fixed-point number. It
is written as a bare name, with no `IMPORT` and no package prefix.

From a **`String`** it parses a decimal number. Text that is not one raises
`ErrInvalidFormat`, and a value outside the range a `Fixed` can hold — roughly
±2.1 billion — raises `ErrOverflow`. From an `Integer`, a `Float` or a `Money` it
converts the number, and one outside that range raises `ErrOverflow` too.

The text is converted exactly, the same way a `Fixed` literal is: the nearest `Fixed`
to the decimal value, a tie going away from zero, with no `Float` step in between.

`Fixed` is **binary** fixed-point, so most decimal fractions are rounded to the
nearest value it can hold on the way in: `toFixed("0.1")` gives the closest
`Fixed` to one-tenth, not exact one-tenth. What it does guarantee is
**determinism** — the same arithmetic gives the same answer on every platform,
which `Float` does not promise. That makes it right for simulation, geometry,
and anywhere a replay has to match.

For monetary amounts, use Money: it represents values with up to five decimal
places exactly. Conversion of more precise inputs can still round.

Use `toFixed` for values computed at runtime. For constants, write a literal with uppercase `F` (`1.5F`) or an annotated
binding (`LET x AS Fixed = 1.5`).

Conversion from Money rounds to the nearest Fixed value under the Money rounding
mode set by `money::setRounding`. Other numeric inputs round to the nearest
Fixed value, with ties away from zero for Float input."#;

const EX: &str = r#"Parse a decimal into a `Fixed`:

```
IMPORT io

FUNC main() AS Nothing
  LET x AS Fixed = toFixed("1.5")
  io::print(toString(x))
END FUNC
```

prints:

```
1.50
```

Text that is not a number raises rather than converting to a value:

```
IMPORT io

FUNC main() AS Nothing
  LET x AS Fixed = toFixed("nope")
  io::print(toString(x))
  RETURN
TRAP(err)
  io::print("toFixed raised " & toString(err.code))
  RETURN
END TRAP
END FUNC
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
