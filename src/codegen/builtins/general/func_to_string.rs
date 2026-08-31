//! `toString` — render a value as a `String`.

use crate::codegen::registry::RegistryPackage;
use crate::types::ParameterType;

use super::{member, opt, req, TO_STRING};

const INTRO: &str = "Render a value as a String.";

const DESC: &str = r#"`toString` renders a value as text. It is the conversion you reach for most, and
it is written as a bare name with no `IMPORT` and no package prefix.

You need it more often than in most languages, because `&` does not convert:
`"count: " & 3` is a type error, and `"count: " & toString(3)` is what you meant.
There is no implicit conversion anywhere in the language.

An `Integer` renders exactly. A `Float` renders with **two decimal places by
default** — `toString(1.5)` is `"1.50"`, not `"1.5"` — which is convenient for
money-shaped output and surprising the first time you meet it. Pass `precision`
to choose a different number of decimals: `toString(3.14159, toByte(2))` is
`"3.14"`. `Boolean` renders as `TRUE` or `FALSE` in capitals.

`precision` is a `Byte`, so an `Integer` literal needs `toByte` around it.

`toString` never fails."#;

const EX: &str = r#"Render several types, and note what a `Float` does by default:

```
IMPORT io

SUB main()
  io::print(toString(1))
  io::print(toString(1.5))
  io::print(toString(3.14159, toByte(2)))
  io::print(toString(TRUE))
END SUB
```

prints:

```
1
1.50
3.14
TRUE
```

Build a message — `&` will not convert for you:

```
IMPORT io

SUB main()
  LET count AS Integer = 3
  io::print("found " & toString(count) & " items")
END SUB
```

prints:

```
found 3 items
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(member(
        TO_STRING,
        (INTRO, DESC, EX),
        ParameterType::String,
        vec![],
        vec![
            req(
                "value",
                ParameterType::named("Scalar"),
                "The value to render. A `Float` gets two decimal places unless `precision` says otherwise.",
            ),
            opt(
                "precision",
                &["decimals"],
                ParameterType::Byte,
                "How many decimal places to render, for a number. A `Byte`, so wrap an integer literal in `toByte`.",
            ),
        ],
    ));
}
