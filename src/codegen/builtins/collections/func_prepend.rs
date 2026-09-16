//! `collections::prepend` — descriptor entry + target-generic lowering (plan-96).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;
const INTO_PREPEND: &str = "Return a list with one element added at the start";
const DESC_PREPEND: &str = r#"`collections::prepend` returns a new list whose first element is `item` and whose
remaining elements are those of `value` in their original order. The result is
always exactly one element longer than `value`. It takes exactly two arguments;
neither is optional and neither is variadic.

Unlike `collections::append`, `prepend` has **only** the single-element form.
There is no list-into-list overload: the second argument must have exactly the
element type `T`, and passing another `List OF T` resolves no overload and is a
compile-time error. When the element type is itself a list, a list is a valid
item and is added as one element.
To place a whole list in front of another, use `collections::append` with the
operands reversed — `collections::append(front, back)`.

`prepend` is `insert` at index `0`.

`prepend` does not change `value`. The list it names is unchanged; the modified
list is the returned value. Each prepend shifts every existing element one
position later, so repeated prepends cost O(n) per call; use `append` when
building a list in forward order.

`prepend` has no index to range-check and no lookup to miss, so it raises no
error of its own. Building the result needs memory, so it can raise
`ErrOutOfMemory`.

Prepending to an empty list yields a one-element list."#;

const EX: &str = r#"Add an element to the front:

```
IMPORT collections

FUNC main AS Integer
  LET numbers AS List OF Integer = collections::prepend([2, 3], 1)
  RETURN 0
END FUNC
```

Build a reversed list by prepending in a loop:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  MUT reversed AS List OF Integer = []
  FOR i = 1 TO 5
    reversed = collections::prepend(reversed, i)
  NEXT
  io::print(toString(collections::get(reversed, 0)))
  RETURN 0
END FUNC
```

Put a whole list in front — use `append` with the operands reversed, because
`prepend` has no list overload:

```
IMPORT collections

FUNC main AS Integer
  LET joined AS List OF Integer = collections::append([1, 2], [3, 4])
  RETURN 0
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "prepend",
        intro: INTO_PREPEND,
        desc: DESC_PREPEND,
        example: EX,
        expected_arguments: Some("List OF T, T"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "The list to prepend to. It may be empty, in which case the result has one element. Not modified — you get a new list back.",
                    aliases: &["list"],
                    ty: ParameterType::list_of(ParameterType::var("T")),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "item",
                    desc: "The element to add at the front. Everything already there shifts one place later.",
                    aliases: &[],
                    ty: ParameterType::var("T"),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Arg(0),
            errors: vec![],
            body: Body::abi_inline(lower_prepend),
        }],
    });
}

/// `collections::prepend` — splice `item` at index `0`. The shared end-insert
/// body with prepend's index (`0`) and its reject-a-list guard.
pub(crate) fn lower_prepend(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    builder.lower_collection_end_insert(args, "prepend", true)
}
