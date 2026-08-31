//! `len` — the number of elements in a String, List, Set, or Map.

use crate::codegen::registry::RegistryPackage;
use crate::types::ParameterType;

use super::{member, req, LEN};

const INTRO: &str = "The number of elements in a String, List, Set, or Map.";

const DESC: &str = r#"`len` counts what is in a value: the Unicode scalar values in a `String`, or the
elements in a `List`, `Set`, or `Map`. It is written as a bare name — there is no
`IMPORT` and no package prefix.

For a `String` it counts **Unicode scalar values**, not bytes and not
user-perceived characters. That distinction matters the moment text stops being
ASCII: `len` of a string holding one accented letter is `1`, while
`strings::byteLen` of the same string is `2`. When you want the count a reader
would give — where a flag or a family emoji counts as one — use
`strings::graphemesCount`.

The empty string and every empty collection give `0`. `len` never fails, changes
nothing, and works the same on every platform.

`len` is the general form; each container also has its own vocabulary in
`collections`, and `strings::byteLen` and `strings::graphemesCount` are the other
two ways to measure text."#;

const EX: &str = r#"Count a string and a list:

```
IMPORT io

SUB main()
  io::print(toString(len("abc")))
  io::print(toString(len("")))
  io::print(toString(len([1, 2, 3])))
END SUB
```

prints:

```
3
0
3
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(member(
        LEN,
        (INTRO, DESC, EX),
        ParameterType::Integer,
        vec![],
        vec![req(
            "value",
            ParameterType::String,
            "The `String`, `List`, `Set`, or `Map` to measure. A `String` is counted in Unicode scalar values.",
        )],
    ));
}
