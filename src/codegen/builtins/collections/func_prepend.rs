//! `collections::prepend` — descriptor entry + target-generic lowering (plan-96).

use crate::codegen::registry::{
    Body, Implementation, Lowering, ParameterType, RegistryFunction, RegistryPackage,
};
use crate::target::shared::code::{CodeBuilder, ValueResult};
use crate::target::shared::nir::NirValue;

const INTO_PREPEND: &str = "Return a list with one element added at the start";
const DESC_PREPEND: &str = r#"`collections::prepend` returns a new list whose first element is `item` and whose
remaining elements are those of `value` in their original order. The result is
always exactly one element longer than `value`. It takes exactly two arguments;
neither is optional and neither is variadic.

Unlike `collections::append`, `prepend` has **only** the single-element form.
There is no list-into-list overload: the second argument must have exactly the
element type `T`, and passing another `List OF T` resolves no overload and is a
compile-time error. The lowering rejects a list-typed item explicitly as well.
To place a whole list in front of another, use `collections::append` with the
operands reversed — `collections::append(front, back)`.

Internally the element is wrapped as a one-element list and spliced into `value`
at index `0`, so the operation is the index-`0` case of the same splice that
backs `append` and `insert`.

`prepend` is value-semantic. The list named by `value` is unchanged; the modified
list is the returned value. When the compiler can prove the target is a uniquely
owned local being reassigned — the `list = collections::prepend(list, x)` shape,
on a non-`by_ref` local that is not the live iterable of an enclosing `FOR EACH` —
it lowers the call to an in-place shift-and-insert with geometric spare capacity
instead of a full copy. This is an optimization only; the observable semantics
are identical either way. Note that prepending must shift every existing lookup
entry right by one, so a repeated prepend stays O(n) per call even on the
in-place path, unlike `append`.

`prepend` is **infallible**: no path in its lowering raises a trappable domain
error. It has no index to range-check and no lookup to miss, so it is classified
as infallible alongside `append` and `replace`, and an inline `TRAP` written on a
`prepend` call has a dead handler (the front end reports
`TYPE_INLINE_TRAP_DEAD_HANDLER`). Allocation exhaustion is not a trappable domain
error in this language.

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

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "prepend",
        intro: INTO_PREPEND,
        desc: DESC_PREPEND,
        example: EX,
        implementations: vec![Implementation {
            params: vec![
                super::param(
                    "value",
                    &["list"],
                    ParameterType::list_of(ParameterType::Var("T")),
                ),
                super::param("item", &[], ParameterType::Var("T")),
            ],
            return_type: ParameterType::Arg(0),
            errors: vec![],
            lowering: Lowering::Helper,
            body: Body::native(None, None, Some(lower_prepend)),
        }],
    });
}

/// `collections::prepend` — splice `item` at index `0`. The shared end-insert
/// body with prepend's index (`0`) and its reject-a-list guard.
pub(crate) fn lower_prepend(
    builder: &mut CodeBuilder,
    args: &[NirValue],
) -> Result<ValueResult, String> {
    builder.lower_collection_end_insert(args, "prepend", true)
}
