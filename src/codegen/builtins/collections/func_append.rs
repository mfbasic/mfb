//! `collections::append` — descriptor entry + target-generic lowering (plan-96).

use super::{custom, req};
use crate::target::shared::code::{CodeBuilder, ValueResult};
use crate::target::shared::nir::NirValue;
use crate::target::shared::registry::BuiltinFunction;

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

Internally both forms are the same operation: the appended content is wrapped as
a list when it is a single element, and the result is built by splicing that list
into `value` at index `count(value)` — the one-past-the-end position, which the
splice accepts as the append position. Existing elements keep their relative
order, and the appended content is placed after all of them in its own order.

`append` is value-semantic. The list named by `value` is unchanged; the modified
list is the returned value, and a program observes the update only through what
it does with that return value. When the compiler can prove the target is a
uniquely owned local being reassigned — the `list = collections::append(list, x)`
shape, on a non-`by_ref` local that is not the live iterable of an enclosing
`FOR EACH` — it lowers the call to an in-place grow with geometric spare
capacity, making a repeated append amortized O(1) rather than a full copy. This
is an optimization only: the observable semantics are identical either way.

`append` is **infallible**: no path in its lowering raises a trappable domain
error. It has no index to range-check and no lookup to miss, so it is classified
as infallible alongside `prepend` and `replace`, and an inline `TRAP` written on
an `append` call has a dead handler (the front end reports
`TYPE_INLINE_TRAP_DEAD_HANDLER`). Allocation exhaustion is not a trappable domain
error in this language.

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

pub(crate) const APPEND: BuiltinFunction = BuiltinFunction::native(
    "collections.append",
    "append",
    INTO_APPEND,
    DESC_APPEND,
    &[],
    &[custom(&[
        req("value", &["list"], "List OF T"),
        req("item", &["items"], "T"),
    ])],
    lower_append,
)
.with_example(EX);

/// `collections::append` — splice `item` (single element or a whole list) at the
/// end of the list. The shared end-insert body with append's index (`count`).
pub(crate) fn lower_append(
    builder: &mut CodeBuilder,
    args: &[NirValue],
) -> Result<ValueResult, String> {
    builder.lower_collection_end_insert(args, "append", false)
}
