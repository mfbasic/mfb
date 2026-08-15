//! `collections::mid` — descriptor entry + authored docs.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, ParameterType, RegistryFunction, RegistryPackage,
};

const INTO_MID: &str = "Return a new list holding a contiguous run of elements taken from a list";

const DESC_MID: &str = r#"`collections::mid` returns a new list holding the `count` elements of `value`
that begin at the zero-based index `start`, in their original order. It is a
**native** member: the compiler emits the slice loop directly rather than
instantiating an MFBASIC generic.

This page documents the `List` form only. `collections::mid` accepts nothing but
a `List` as its first argument; the `String` slice of the same name lives in
`strings::`.

All three arguments are required — there is no two-argument "to the end" form —
and `start` and `count` must both be exactly `Integer`.

The range is **validated, not clamped**. Before any element is copied the
lowering checks, in order, that `start` is not negative, that `count` is not
negative, that `start` is not greater than the length of `value`, that
`start + count` does not wrap around, and that `start + count` is not greater
than the length of `value`. Any of those failing raises `ErrIndexOutOfRange`.
A short trailing run is therefore an error rather than a truncated result: on a
three-element list, `mid(value, 2, 2)` fails instead of returning one element.

Empty results are legal at the boundaries, since `start` may equal the length of
`value` and `count` may be `0`: on a four-element list, `mid(value, 4, 0)`
returns an empty list.

The result is a freshly allocated, independently owned list of the same type as
`value`; `value` itself is neither modified nor consumed, and element payloads
are copied into the new list's own data region rather than shared.

`mid` copies the selected run using a fast contiguous path when the source
entries covering the slice are stored in order and packed tightly, and falls
back to a per-entry copy otherwise. A list whose entry records have been
permuted without moving the underlying data — the result of a sorted directory
listing, for instance — takes the fallback. Either way the returned elements are
the same."#;

const EX_MID: &str = r#"Take two elements from the middle:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET numbers AS List OF Integer = [1, 2, 3, 4]
  LET middle AS List OF Integer = collections::mid(numbers, 1, 2)
  io::print(toString(collections::get(middle, 0)))
  io::print(toString(len(middle)))
  RETURN 0
END FUNC
```

An empty slice at the end of the list is legal:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET numbers AS List OF Integer = [1, 2, 3, 4]
  LET empty AS List OF Integer = collections::mid(numbers, 4, 0)
  io::print(toString(len(empty)))
  RETURN 0
END FUNC
```

An over-long range raises rather than truncating, so handle it:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET numbers AS List OF Integer = [1, 2, 3]
  LET tail AS List OF Integer = collections::mid(numbers, 2, 2) TRAP(e)
    io::print("bad range: " & e.message)
    RECOVER []
  END TRAP
  io::print(toString(len(tail)))
  RETURN 0
END FUNC
```"#;

/// `collections::mid` — List slice. Bare-name dispatch (`lower_mid`); [`Body::Intrinsic`].
pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "mid",
        intro: INTO_MID,
        desc: DESC_MID,
        example: EX_MID,
        expected_arguments: Some("List OF T, Integer, Integer"),
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
                    name: "start",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "count",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Arg(0),
            errors: vec!["ErrIndexOutOfRange"],
            body: Body::Intrinsic,
        }],
    });
}
