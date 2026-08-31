//! `collections::append` — descriptor entry + target-generic lowering (plan-96).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;
const INTO_APPEND: &str =
    "Return a list with one element, or every element of another list, added at the end";
const DESC_APPEND: &str = r#"`collections::append` returns a new list whose contents are those of `value`
followed by the appended content. It takes exactly two arguments; neither is
optional and neither is variadic.

The second argument may be either a single element of the list's element type
`T`, or another `List OF T`. The compiler picks the overload from the static type
of that argument: an argument whose type is exactly the element type appends one
element, and an argument whose type is exactly the same list type concatenates.
Any other type is a compile-time error, because no other combination resolves.

Both forms behave the same way: existing elements keep their relative order, and
the appended content is placed after all of them, in its own order.

`append` does not change `value`. The list it names is unchanged; the modified
list is the returned value, and a program observes the update only through what
it does with that return value. Assigning straight back to the same local
variable — `list = collections::append(list, x)` — is the cheap shape: it grows
the list instead of copying it, so appending in a loop costs about the same per
element however long the list gets. Appending to something reached another way
(a parameter, a module-level `MUT`, or the list a `FOR EACH` is walking) copies
the whole list on every call. The result is the same either way.

`append` is **infallible**: nothing it does raises a trappable error. It has no
index to range-check and no lookup to miss, so an inline `TRAP` written on an
`append` call has a handler that can never run, and the compiler reports it.

Appending an empty list returns a copy of `value` with the same elements in the
same order."#;

const EX: &str = r#"Append a single element:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET numbers AS List OF Integer = collections::append([1, 2], 3)
  io::print(toString(len(numbers)))
  RETURN 0
END FUNC
```

Concatenate a second list:

```
IMPORT collections

FUNC main AS Integer
  LET numbers AS List OF Integer = collections::append([1, 2], [3, 4])
  RETURN 0
END FUNC
```

Build a list in a loop; the argument is never mutated, the result is:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  MUT bytes AS List OF Byte = []
  FOR i = 65 TO 70
    bytes = collections::append(bytes, toByte(i))
  NEXT
  io::print(toString(len(bytes)))
  RETURN 0
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "append",
        intro: INTO_APPEND,
        desc: DESC_APPEND,
        example: EX,
        expected_arguments: Some("List OF T, T or List OF T, List OF T"),
        internal_only: false,
        implementations: vec![
            Implementation {
                params: vec![
                    Parameter {
                        name: "value",
                        desc: "The list to append to. Not modified — you get a new list back.",
                        aliases: &["list"],
                        ty: ParameterType::list_of(ParameterType::var("T")),
                        default: DefaultValue::None,
                    },
                    Parameter {
                        name: "item",
                        desc: "The element to add at the end. Appending a list appends every one of its elements, not the list itself.",
                        aliases: &["items"],
                        ty: ParameterType::var("T"),
                        default: DefaultValue::None,
                    },
                ],
                return_type: ParameterType::Arg(0),
                errors: vec![],
                body: Body::abi_inline(lower_append),
            },
            Implementation {
                params: vec![
                    Parameter {
                        name: "value",
                        desc: "The list to append to. Not modified — you get a new list back.",
                        aliases: &["list"],
                        ty: ParameterType::list_of(ParameterType::var("T")),
                        default: DefaultValue::None,
                    },
                    Parameter {
                        name: "item",
                        desc: "The element to add at the end. Appending a list appends every one of its elements, not the list itself.",
                        aliases: &["items"],
                        ty: ParameterType::list_of(ParameterType::var("T")),
                        default: DefaultValue::None,
                    },
                ],
                return_type: ParameterType::Arg(0),
                errors: vec![],
                body: Body::abi_inline(lower_append),
            },
        ],
    });
}

/// `collections::append` — splice `item` (single element or a whole list) at the
/// end of the list. The shared end-insert body with append's index (`count`).
pub(crate) fn lower_append(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    builder.lower_collection_end_insert(args, "append", false)
}
