//! `isNotEmpty` — whether a String, List, Set, or Map has at least one element.

use crate::codegen::registry::RegistryPackage;
use crate::types::ParameterType;

use super::{member, req, IS_NOT_EMPTY};

const INTRO: &str = "Whether a String, List, Set, or Map has at least one element.";

const DESC: &str = r#"`isNotEmpty` answers whether a `String`, `List`, `Set`, or `Map` holds at least
one element. It is written as a bare name, with no `IMPORT` and no package
prefix.

It is exactly the negation of `isEmpty`, and exists so a condition that is about
*having* something reads that way — `IF isNotEmpty(results) THEN` rather than
`IF NOT isEmpty(results) THEN`.

Like `isEmpty` it counts elements, not significance: a `String` of one space is
not empty, so it answers `TRUE` here. Trim first with `strings::trim` if
whitespace should not count.

`isNotEmpty` never fails and changes nothing."#;

const EX: &str = r#"Act only when there is something to act on:

```
IMPORT io

SUB main()
  LET results AS List OF String = ["a", "b"]
  IF isNotEmpty(results) THEN
    io::print("got " & toString(len(results)) & " results")
  ELSE
    io::print("nothing found")
  END IF
END SUB
```

prints:

```
got 2 results
```

A string of spaces counts as non-empty:

```
IMPORT io

SUB main()
  io::print(toString(isNotEmpty("a")))
  io::print(toString(isNotEmpty(" ")))
  io::print(toString(isNotEmpty("")))
END SUB
```

prints:

```
TRUE
TRUE
FALSE
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(member(
        IS_NOT_EMPTY,
        (INTRO, DESC, EX),
        ParameterType::Boolean,
        vec![],
        vec![req(
            "value",
            ParameterType::String,
            "The `String`, `List`, `Set`, or `Map` to test. A string of spaces counts as non-empty.",
        )],
    ));
}
