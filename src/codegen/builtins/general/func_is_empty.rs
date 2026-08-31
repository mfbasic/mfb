//! `isEmpty` — whether a String, List, Set, or Map has no elements.

use crate::codegen::registry::RegistryPackage;
use crate::types::ParameterType;

use super::{member, req, IS_EMPTY};

const INTRO: &str = "Whether a String, List, Set, or Map has no elements.";

const DESC: &str = r#"`isEmpty` answers whether a `String`, `List`, `Set`, or `Map` holds nothing. It
is written as a bare name, with no `IMPORT` and no package prefix.

It says the same thing as `len(value) = 0` and reads better, especially in a
condition. Use whichever is clearer where you are; there is no difference in
meaning.

Empty means **no elements**, not "nothing meaningful": a `String` of one space
is not empty, and neither is a `List` holding one zero. If you want to treat
whitespace-only text as blank, trim it first with `strings::trim`.

`isEmpty` never fails and changes nothing. Its opposite is `isNotEmpty`, which
exists so a positive condition can stay positive rather than becoming
`NOT isEmpty(...)`."#;

const EX: &str = r#"Test each kind of container:

```
IMPORT io

SUB main()
  io::print(toString(isEmpty("")))
  io::print(toString(isEmpty([])))
  io::print(toString(isEmpty(" ")))
END SUB
```

prints:

```
TRUE
TRUE
FALSE
```

Treat whitespace-only input as blank by trimming first:

```
IMPORT io
IMPORT strings

SUB main()
  LET field AS String = "   "
  IF isEmpty(strings::trim(field)) THEN
    io::print("please enter a value")
  ELSE
    io::print("got " & field)
  END IF
END SUB
```

prints:

```
please enter a value
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(member(
        IS_EMPTY,
        (INTRO, DESC, EX),
        ParameterType::Boolean,
        vec![],
        vec![req(
            "value",
            ParameterType::String,
            "The `String`, `List`, `Set`, or `Map` to test. A string of spaces is not empty.",
        )],
    ));
}
