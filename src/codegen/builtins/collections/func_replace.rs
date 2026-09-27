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

Matching compares each element against `old` with the same equality
`collections::contains` uses, so the element type must be comparable, and `Float`
values match by bit pattern: `-0.0` and `0.0` are distinct; `old` and `new` must both have
exactly the element type `T`. `new` may itself be equal to `old`, in which case
the result is equal to `value`.

Only the **List** overload of `replace` lives in `collections`. The `String`
overload — replacing a substring within a `String` — is a different function that
lives in `strings::`. A `String` first argument is a compile-time error here.

`replace` does not change `value`. The list it names is unchanged; the modified
list is the returned value, and a program observes the update only through what
it does with that return value. Assigning straight back to the same `MUT`
variable — `list = collections::replace(list, old, new)` — is the cheap shape: it
rewrites the matching elements in the list itself instead of building a copy.
Assigning the result anywhere else — `ys = collections::replace(xs, old, new)` —
builds a new list and leaves `xs` as it was. The result is the same either way.

`replace` has no index to range-check, and an `old` that never matches produces an
unchanged copy rather than a failure, so it raises no error of its own. Building
the result needs memory, so it can raise `ErrOutOfMemory`."#;

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
