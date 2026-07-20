# insert

Return a list with one element inserted before a given index

## Synopsis

```
collections::insert OF T(value AS List OF T, index AS Integer, item AS T) AS List OF T
```

## Package

collections

## Imports

```
IMPORT collections
```

`collections` is a built-in package, so no manifest dependency is required.
[[src/builtins/collections.rs:is_collections_call]]

## Description

`collections::insert` returns a new list in which `item` occupies position
`index`, every element of `value` below `index` keeps its position, and every
element from `index` onward is shifted up by one. The result is always exactly
one element longer than `value`. It takes exactly three arguments; none is
optional and none is variadic. [[src/builtins/collections.rs:arity]]

`index` is zero-based and is validated as `0 <= index <= len(value)`. The upper
bound is **inclusive**: `index` equal to the current length is the append
position and is accepted, producing the same result as
`collections::append(value, item)`. A negative `index`, or an `index` strictly
greater than the length, raises `ErrIndexOutOfRange`.
[[src/target/shared/code/builder_collection_mutate.rs:lower_list_insert_collection]]

Only the single-element form exists. `item` must have exactly the element type
`T`; passing another `List OF T` resolves no overload, and the lowering rejects a
list-typed item explicitly with "insert expects a single item, not a list".
Internally the element is wrapped as a one-element list and spliced into `value`
at `index`, which is the same splice that backs `append` (index `= len`) and
`prepend` (index `0`).
[[src/builtins/general.rs:resolve_insert]]
[[src/target/shared/code/builder_collection_mutate.rs:lower_collection_insert]]

`insert` is value-semantic. The list named by `value` is unchanged; the modified
list is the returned value, and a program observes the update only through what
it does with that return value. There is no in-place fast path for `insert` at an
arbitrary index — the compiler's in-place assignment recognizers cover
`append`, bulk `append`, `prepend`, `set`, and string concatenation, not
`insert`. [[src/target/shared/code/builder_inplace_assign.rs:try_inplace_set_assign]]

`insert` is **fallible**: the range check is a real trappable domain error, so an
inline `TRAP` on an `insert` call compiles and catches the out-of-range failure
rather than being reported as a dead handler. The bounds test runs before any
allocation for the result, so a rejected index allocates nothing.
[[src/builtins/mod.rs:inline_builtin_raw_supported]]
[[src/builtins/mod.rs:inline_builtin_is_infallible]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `List OF T` | The list to insert into; left unchanged. Also accepted under the name `list`. Must be a list type; a `Map` or scalar resolves no overload and is a compile-time error. [[src/builtins/collections.rs:call_param_names]] [[src/builtins/general.rs:resolve_insert]] |
| `index` | `Integer` | Zero-based position the inserted element will occupy. Valid range is `0` through `len(value)` inclusive; `len(value)` appends. Must be declared `Integer` exactly — no other numeric type resolves. This parameter has no alternate spelling. [[src/builtins/collections.rs:call_param_names]] [[src/target/shared/code/builder_collection_mutate.rs:lower_list_insert_collection]] |
| `item` | `T` | The single element to insert. Its type must be exactly the list's element type `T`; a `List OF T` is not accepted. This parameter has no alternate spelling. [[src/builtins/collections.rs:call_param_names]] [[src/target/shared/code/builder_collection_mutate.rs:lower_collection_insert]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF T` | A new list of the same type as `value`, one element longer, with `item` at position `index` and every prior element from `index` onward shifted up by one. Inserting at `0` places `item` first; inserting at `len(value)` places it last. [[src/builtins/general.rs:resolve_insert]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050001` | `ErrIndexOutOfRange` | `index` is negative, or `index` is strictly greater than `len(value)`. The append position `index == len(value)` is valid and does not raise. [[src/target/shared/code/error_constants.rs:ERR_INDEX_OUT_OF_RANGE_CODE]] [[src/target/shared/code/builder_collection_mutate.rs:lower_list_insert_collection]] |

## Type checking

The first argument must be a `List OF T`, the second must be `Integer`, and the
third must have exactly the element type `T`. There is no implicit widening or
conversion in any position. A call on a non-list first argument, a non-`Integer`
index, or a mismatched element type resolves to no overload and is rejected at
compile time; the index range itself is a runtime check, not a compile-time one.
[[src/builtins/general.rs:resolve_insert]]

## Examples

Insert in the middle:

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
```

## See also

- `mfb man collections append`
- `mfb man collections prepend`
- `mfb man collections removeAt`
- `mfb man collections set`
- `mfb man collections`
