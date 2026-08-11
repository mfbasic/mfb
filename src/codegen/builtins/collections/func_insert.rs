//! `collections::insert` — descriptor entry + target-generic lowering (plan-96).

use super::{custom, req};
use crate::codegen::registry::BuiltinFunction;
use crate::target::shared::abi;
use crate::target::shared::code::type_utils::list_element_type;
use crate::target::shared::code::{CodeBuilder, ValueResult};
use crate::target::shared::nir::NirValue;

const INTO_INSERT: &str = "Return a list with one element inserted before a given index";
const DESC_INSERT: &str = r#"`collections::insert` returns a new list in which `item` occupies position
`index`, every element of `value` below `index` keeps its position, and every
element from `index` onward is shifted up by one. The result is always exactly
one element longer than `value`. It takes exactly three arguments; none is
optional and none is variadic.

`index` is zero-based and is validated as `0 <= index <= len(value)`. The upper
bound is **inclusive**: `index` equal to the current length is the append
position and is accepted, producing the same result as
`collections::append(value, item)`. A negative `index`, or an `index` strictly
greater than the length, raises `ErrIndexOutOfRange`.

Only the single-element form exists. `item` must have exactly the element type
`T`; passing another `List OF T` resolves no overload, and the lowering rejects a
list-typed item explicitly with "insert expects a single item, not a list".
Internally the element is wrapped as a one-element list and spliced into `value`
at `index`, which is the same splice that backs `append` (index `= len`) and
`prepend` (index `0`).

`insert` is value-semantic. The list named by `value` is unchanged; the modified
list is the returned value, and a program observes the update only through what
it does with that return value. There is no in-place fast path for `insert` at an
arbitrary index — the compiler's in-place assignment recognizers cover
`append`, bulk `append`, `prepend`, `set`, and string concatenation, not
`insert`.

`insert` is **fallible**: the range check is a real trappable domain error, so an
inline `TRAP` on an `insert` call compiles and catches the out-of-range failure
rather than being reported as a dead handler. The bounds test runs before any
allocation for the result, so a rejected index allocates nothing."#;

const EX: &str = r#"Insert in the middle:

```
IMPORT collections

FUNC main AS Integer
  LET numbers AS List OF Integer = collections::insert([1, 3], 1, 2)
  RETURN 0
END FUNC
```

Insert at the length — the append position, which is in range:

```
IMPORT collections

FUNC main AS Integer
  LET numbers AS List OF Integer = collections::insert([1, 2], 2, 3)
  RETURN 0
END FUNC
```

Catch an out-of-range index with an inline `TRAP`:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET numbers AS List OF Integer = collections::insert([1, 2], 5, 9) TRAP(e)
    io::print(e.message)
    RECOVER [1, 2]
  END TRAP
  RETURN 0
END FUNC
```"#;

pub(crate) const INSERT: BuiltinFunction = BuiltinFunction::native(
    "collections.insert",
    "insert",
    INTO_INSERT,
    DESC_INSERT,
    &["ErrIndexOutOfRange"],
    &[custom(&[
        req("value", &["list"], "List OF T"),
        req("index", &[], "Integer"),
        req("item", &[], "T"),
    ])],
    lower_insert,
)
.with_example(EX);

/// `collections::insert(List OF T, Integer, T) AS List OF T`: splice `item` at
/// `index` (`0 <= index <= len`, range-checked -> `ErrIndexOutOfRange`).
pub(crate) fn lower_insert(
    builder: &mut CodeBuilder,
    args: &[NirValue],
) -> Result<ValueResult, String> {
    let list = builder.lower_value(&args[0])?;
    let Some(element_type) = list_element_type(&list.type_) else {
        return Err(format!(
            "native collection insert does not accept {}",
            list.type_
        ));
    };
    let list_slot = builder.allocate_stack_object("insert_list", 8);
    builder.emit(abi::store_u64(
        &list.location,
        abi::stack_pointer(),
        list_slot,
    ));
    let index = builder.lower_value(&args[1])?;
    if index.type_ != "Integer" {
        return Err(format!(
            "native collection insert index must be Integer, got {}",
            index.type_
        ));
    }
    let index_slot = builder.allocate_stack_object("insert_index", 8);
    builder.emit(abi::store_u64(
        &index.location,
        abi::stack_pointer(),
        index_slot,
    ));
    let item = builder.lower_value(&args[2])?;
    // Observation boundary: a `Float` inserted element must be finite (plan-17).
    builder.observe_float(&args[2], &item)?;
    if item.type_ == list.type_ {
        return Err("native collection insert expects a single item, not a list".to_string());
    }
    // Materialize a `d`-native float before the payload spill (plan-01).
    let item = builder.materialize_value(item)?;
    let (insert_slot, materialized) =
        builder.collection_argument_as_list_slot(&list.type_, &element_type, item)?;
    let result = builder.lower_list_insert_collection(
        list_slot,
        index_slot,
        insert_slot,
        &list.type_,
        &element_type,
    )?;
    if materialized {
        return builder.free_intermediate_collection(insert_slot, &list.type_, result);
    }
    Ok(result)
}
