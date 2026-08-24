//! `collections::find` — descriptor entry + authored docs.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTO_FIND: &str =
    "Return the index of the first matching element or contiguous sublist in a list";

const DESC_FIND: &str = r#"`collections::find` scans `value` forward from `start` and returns the
zero-based index of the first match. It is a **native** member: the compiler
emits the search loop directly rather than instantiating an MFBASIC generic.

This page documents the `List` form only. `collections::find` accepts nothing
but a `List` as its first argument; the `String` search of the same name lives in
`strings::`.

Two searches share the name, chosen by the type of the second argument. When it
has the element type `T`, `find` performs an **element search**. When it has the
same `List OF T` type as `value`, `find` performs a **contiguous sublist
search**. The element form is tested first, so for a list of lists — where the
element type is itself a `List` — a second argument of that element type is read
as an element search. Any other second-argument type fails to resolve at compile
time.

`start` is optional. When it is omitted the search begins at index 0; the
lowering supplies that default itself, so an omitted `start` and an explicit `0`
behave identically.

`start` is validated before anything is compared. A negative `start`, or a
`start` greater than the length of `value`, fails with `ErrIndexOutOfRange`. A
`start` exactly equal to the length is **valid**: it selects an empty search
range, which yields `ErrNotFound` for an element search and, for a sublist
search with an empty needle, the index `start` itself.

When no match exists at or after `start`, `find` fails with `ErrNotFound`. It
never returns a sentinel such as `-1`; a search that may legitimately come up
empty needs a `TRAP`, or `collections::contains` if only the yes/no answer is
wanted.

Element equality is decided on the stored payload. `String` elements compare by
length and then byte for byte; `Integer`, `Float`, `Fixed`, and `Money` elements
compare as their stored 64-bit pattern, so `Float` matching is bit-exact and a
`NaN` never matches itself; `Boolean`, `Byte`, and `Scalar` compare as their
narrower stored value; record elements compare field by field. A nested
collection that is stored as a handle rather than inlined compares by identity,
not by contents.

`value` is neither modified nor consumed, and no new collection is allocated."#;

const EX_FIND: &str = r#"Find an element, with and without a starting index:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET numbers AS List OF Integer = [10, 20, 30, 20]
  io::print(toString(collections::find(numbers, 20)))
  io::print(toString(collections::find(numbers, 20, 2)))
  RETURN 0
END FUNC
```

Find a contiguous sublist:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET numbers AS List OF Integer = [10, 20, 30, 20]
  LET needle AS List OF Integer = [20, 30]
  io::print(toString(collections::find(numbers, needle)))
  RETURN 0
END FUNC
```

Handle a missing element instead of letting `ErrNotFound` propagate:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET numbers AS List OF Integer = [1, 2, 3]
  LET index AS Integer = collections::find(numbers, 99) TRAP(e)
    io::print("absent: " & e.message)
    RECOVER -1
  END TRAP
  io::print(toString(index))
  RETURN 0
END FUNC
```"#;

/// `collections::find` — List element/sublist search. Reached through the
/// `native_builtin_target` bare-name dispatch (`lower_find`), so its `Body` is
/// [`Body::Intrinsic`] (no `abi_inline` lowering, no rewrite); the descriptor exists only
/// for return-type resolution, arity, errors, and parameter names.
pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "find",
        intro: INTO_FIND,
        desc: DESC_FIND,
        example: EX_FIND,
        expected_arguments: Some("List OF T, T[, Integer]"),
        internal_only: false,
        implementations: vec![
            Implementation {
                params: vec![
                    Parameter {
                        name: "value",
                        desc: "",
                        aliases: &["list"],
                        ty: ParameterType::list_of(ParameterType::Var("T")),
                        default: DefaultValue::None,
                    },
                    Parameter {
                        name: "item",
                        desc: "",
                        aliases: &["needle"],
                        ty: ParameterType::Var("T"),
                        default: DefaultValue::None,
                    },
                    Parameter {
                        name: "start",
                        desc: "",
                        aliases: &[],
                        ty: ParameterType::Integer,
                        default: DefaultValue::Optional,
                    },
                ],
                return_type: ParameterType::Integer,
                errors: vec!["ErrIndexOutOfRange", "ErrNotFound"],
                body: Body::Intrinsic,
            },
            Implementation {
                params: vec![
                    Parameter {
                        name: "value",
                        desc: "",
                        aliases: &["list"],
                        ty: ParameterType::list_of(ParameterType::Var("T")),
                        default: DefaultValue::None,
                    },
                    Parameter {
                        name: "item",
                        desc: "",
                        aliases: &["needle"],
                        ty: ParameterType::list_of(ParameterType::Var("T")),
                        default: DefaultValue::None,
                    },
                    Parameter {
                        name: "start",
                        desc: "",
                        aliases: &[],
                        ty: ParameterType::Integer,
                        default: DefaultValue::Optional,
                    },
                ],
                return_type: ParameterType::Integer,
                errors: vec!["ErrIndexOutOfRange", "ErrNotFound"],
                body: Body::Intrinsic,
            },
        ],
    });
}
