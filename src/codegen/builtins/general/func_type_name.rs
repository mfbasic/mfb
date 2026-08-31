//! `typeName` — the name of a value's runtime type.

use crate::codegen::registry::RegistryPackage;
use crate::types::ParameterType;

use super::{member, req, TYPE_NAME};

const INTRO: &str = "The name of a value's type.";

const DESC: &str = r#"`typeName` returns the name of `value`'s type as a `String`, spelled the way you
would write it in source: `"Integer"`, `"Float"`, `"String"`, `"Boolean"`, and
for a container its full element type — `typeName([1])` is `"List OF Integer"`.
It is written as a bare name, with no `IMPORT` and no package prefix.

The name is the type the **compiler** gave the expression, not something
discovered while the program runs — MFBASIC settles every type before the
program starts, and `typeName` is answered then too.

That makes it a diagnostic tool. Use it in a log line, a test message, or while
working out why a value is not the type you expected. It is **not** a way to
branch on type: comparing `typeName` against a string to choose a code path
expresses a decision the language has already made, and a `MATCH` on a `UNION`
says the same thing in a form the compiler checks.

`typeName` never fails, changes nothing, and accepts any value."#;

const EX: &str = r#"Report the type of several values:

```
IMPORT io

SUB main()
  io::print(typeName(1))
  io::print(typeName(1.0))
  io::print(typeName("a"))
  io::print(typeName(TRUE))
  io::print(typeName([1]))
END SUB
```

prints:

```
Integer
Float
String
Boolean
List OF Integer
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(member(
        TYPE_NAME,
        (INTRO, DESC, EX),
        ParameterType::String,
        vec![],
        vec![req(
            "value",
            ParameterType::var("T"),
            "Any value. Its type name comes back spelled as you would write it in source.",
        )],
    ));
}
