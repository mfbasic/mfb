//! `collections::replace` — descriptor entry + authored docs.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, ParameterType, RegistryFunction, RegistryPackage,
};

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

`replace` is value-semantic. The list named by `value` is unchanged; the modified
list is the returned value, and a program observes the update only through what
it does with that return value. There is no in-place fast path for `replace` —
the compiler's in-place assignment recognizers cover `append`, bulk `append`,
`prepend`, `set`, and string concatenation, not `replace`.

`replace` is **infallible**: no path in its lowering raises a trappable domain
error. It has no index to range-check, and a `new` that never matches is a
success producing an unchanged copy, not a failure — so it is classified as
infallible alongside `append` and `prepend`, and an inline `TRAP` written on a
`replace` call has a dead handler (the front end reports
`TYPE_INLINE_TRAP_DEAD_HANDLER`). Allocation exhaustion is not a trappable domain
error in this language."#;

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
pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "replace",
        intro: INTO_REPLACE,
        desc: DESC_REPLACE,
        example: EX_REPLACE,
        expected_arguments: Some("List OF T, T, T"),
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "",
                    aliases: &["list"],
                    ty: ParameterType::list_of(ParameterType::Var("T")),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "old",
                    desc: "",
                    aliases: &["needle"],
                    ty: ParameterType::Var("T"),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "new",
                    desc: "",
                    aliases: &["replacement"],
                    ty: ParameterType::Var("T"),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Arg(0),
            errors: vec![],
            body: Body::Intrinsic,
        }],
    });
}
