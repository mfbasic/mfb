//! `collections::replace` — descriptor entry + authored docs.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTO_REPLACE: &str = "Return a list with every element equal to a given value replaced";

const DESC_REPLACE: &str = r#"`collections::replace` returns a new list of the same length as `value` in which
every element equal to `old` has been replaced by `new`, and every other element
is carried over unchanged. It takes exactly three arguments; none is optional and
none is variadic.

All matches are replaced, not just the first, and positions are preserved: the
result has the same length and the same ordering as `value`, differing only at
the indices where `old` occurred. When `old` does not occur, the result is a copy
of `value`. When `value` is empty, the result is empty.

Matching compares each element's stored payload against `old` using the same
element-equality test the rest of the collections layer uses, so the element type
must be one for which that comparison is defined; `old` and `new` must both have
exactly the element type `T`. `new` may itself be equal to `old`, in which case
the result is equal to `value`.

Only the **List** overload of `replace` lives in `collections`. The `String`
overload — replacing a substring within a `String` — is a different function that
lives in `strings::`. A `String` first argument does not resolve here.

`replace` does not change `value`. The list it names is unchanged; the modified
list is the returned value, and a program observes the update only through what
it does with that return value. Unlike `append`, `prepend`, and `set`, there is
no cheap in-place shape for `replace`: every call copies the list.

`replace` is **infallible**: nothing it does raises a trappable error. It has no
index to range-check, and a `new` that never matches is a success producing an
unchanged copy, not a failure — so an inline `TRAP` written on a
`replace` call has a handler that can never run, and the compiler reports it."#;

const EX_REPLACE: &str = r#"Replace every matching element:

```
IMPORT collections

FUNC main AS Integer
  LET values AS List OF Integer = collections::replace([1, 2, 1], 1, 9)
  RETURN 0
END FUNC
```

A needle that does not occur yields an unchanged copy:

```
IMPORT collections
IMPORT strings
IMPORT io

FUNC main AS Integer
  LET words AS List OF String = collections::replace(["a", "b"], "z", "Q")
  io::print(strings::join(words, ","))
  RETURN 0
END FUNC
```

Substituting a placeholder throughout a list:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET cleaned AS List OF String = collections::replace(["x", "b", "x"], "x", "QQ")
  io::print(toString(len(cleaned)))
  RETURN 0
END FUNC
```"#;

/// `collections::replace` — List element replacement. Bare-name dispatch
/// (`lower_replace`); [`Body::Intrinsic`].
pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "replace",
        intro: INTO_REPLACE,
        desc: DESC_REPLACE,
        example: EX_REPLACE,
        expected_arguments: Some("List OF T, T, T"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "The list to work on. Not modified — you get a new list back.",
                    aliases: &["list"],
                    ty: ParameterType::list_of(ParameterType::var("T")),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "old",
                    desc: "The element to look for. Every element equal to it is replaced, not just the first.",
                    aliases: &["needle"],
                    ty: ParameterType::var("T"),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "new",
                    desc: "What to put in its place.",
                    aliases: &["replacement"],
                    ty: ParameterType::var("T"),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Arg(0),
            errors: vec![],
            body: Body::Intrinsic,
        }],
    });
}
