# contains

Test whether a list holds an item equal to a given value.

## Synopsis

```
collections::contains OF T(value AS List OF T, item AS T) AS Boolean
collections::contains OF T(value AS Set OF T, item AS T) AS Boolean
```

## Package

collections

## Imports

```
IMPORT collections
```

`collections` is a built-in package, so no manifest dependency is required.
[[src/codegen/builtins/collections/mod.rs:is_collections_call]]

`contains` is a native `collections::` member and must be called with the
`collections::` qualifier; there is no bare `contains` built-in.
[[src/codegen/builtins/collections/mod.rs:is_native_member]]

## Description

`collections::contains` scans `value` from index `0` upward and returns `TRUE`
as soon as an element matches `item`, or `FALSE` after every element has been
examined without a match. The list is neither copied nor mutated, and no element
payload is materialized — the scan compares stored bytes in place.
[[src/codegen/builtins/collections/func_contains.rs:lower_contains]]

`contains` also has a **`Set OF T`** overload. Both forms take
`(collection, element) AS Boolean` and answer the same membership question; the
compiler picks the overload from the static type of the first argument. On a
`List` the scan is linear (below); on a `Set` membership is an O(1)-average hash
probe for a probe-eligible element type and a linear scan otherwise. It does not
accept a `Map`, and it is not the substring test: the `String` form of
`contains` lives in the `strings::` package, not here.
[[src/codegen/builtins/collections/mod.rs:resolve_contains]]
[[src/codegen/builtins/collections/mod.rs:COLLECTIONS]]

Equality is payload comparison, resolved by the element type:

- `Boolean` and `Byte` compare one stored byte; `Scalar` compares four; and
  `Integer`, `Float`, `Fixed`, and `Money` compare their stored 64-bit value.
- `String` compares length first, then bytes, so the match is exact and
  byte-oriented — no case folding, trimming, or Unicode normalization is applied.
- A record element is compared field by field.
- A resource handle, or a nested collection that is not stored flat, is compared
  by its stored handle rather than by its contents.

[[src/target/shared/code/builder_collection_compare.rs:emit_collection_payload_match_branch]]

Because numeric comparison is bitwise, a `Float` search for `NaN` is always
`FALSE` even if the list contains `NaN`, and searching for `-0.0` does not match
a stored `0.0`.

An empty list always yields `FALSE`, since the loop exits on the first bounds
check. `collections::contains` raises no trappable domain error, so an inline
`TRAP` on a `contains` call has a dead handler.
[[src/builtins/mod.rs:inline_builtin_is_infallible]]

`contains` answers only whether a match exists. Use `collections::find` when the
position of the match is needed.

## Overloads

**`collections::contains OF T(value AS List OF T, item AS T) AS Boolean`**

The list form scans `value` from index `0` upward, comparing each stored element
payload against `item`, and returns `TRUE` on the first match or `FALSE` after
examining every element. It is O(n) in the list length.
[[src/codegen/builtins/collections/mod.rs:resolve_contains]]
[[src/codegen/builtins/collections/func_contains.rs:lower_contains]]

**`collections::contains OF T(value AS Set OF T, item AS T) AS Boolean`**

The set form tests membership through the set's hash index: an O(1)-average
FNV-1a probe for the probe-eligible element types (`Integer`, `Float`, `Fixed`,
`Byte`, `Boolean`, `String`), falling back to a linear scan over the live entries
for any other element type. The answer is identical to the list form's; only the
lookup strategy differs. [[src/codegen/builtins/collections/mod.rs:resolve_contains]]
[[src/target/shared/code/builder_collection_queries.rs:emit_key_membership]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `List OF T` or `Set OF T` | The collection to test. A list is examined left to right; a set is probed through its hash index. Also accepted under the name `collection`. Not copied and not mutated. [[src/codegen/builtins/collections/mod.rs:call_param_names]] |
| `item` | `T` | The value to search for. Must be exactly the collection's element type. [[src/codegen/builtins/collections/mod.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` when some element of `value` matches `item`; `FALSE` when none does, including for an empty list or set. [[src/codegen/builtins/collections/mod.rs:resolve_contains]] |

## Errors

No errors.

## Type checking

`collections::contains` takes exactly two arguments.
[[src/codegen/builtins/collections/mod.rs:COLLECTIONS]]

The first must be a `List OF T` or a `Set OF T`; a `Map`, a `String`, or any
other value is a compile-time type error. The second must be exactly the element
type `T` — a `List OF Integer` cannot be searched with a `String`, and there is
no implicit conversion between numeric element types. The result is always
`Boolean`. [[src/codegen/builtins/collections/mod.rs:resolve_contains]]

## Examples

Test list membership:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET numbers AS List OF Integer = [1, 2, 3]
  io::print(toString(collections::contains(numbers, 2)))
  io::print(toString(collections::contains(numbers, 9)))
  RETURN 0
END FUNC
```

Branch on membership:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET names AS List OF String = ["Ada", "Grace"]
  IF collections::contains(names, "Ada") THEN
    io::print("found")
  END IF
  RETURN 0
END FUNC
```

An empty list contains nothing:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET empty AS List OF Integer = []
  io::print(toString(collections::contains(empty, 0)))
  RETURN 0
END FUNC
```

Test set membership; the same call works on a `Set`:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET s AS Set OF Integer = Set OF Integer { 1, 2, 3 }
  io::print(toString(collections::contains(s, 2)))
  io::print(toString(collections::contains(s, 9)))
  RETURN 0
END FUNC
```

## See also

- `mfb man collections find`
- `mfb man collections hasKey`
- `mfb man collections filter`
- `mfb man collections distinct`
- `mfb man types set`
- `mfb man collections`
