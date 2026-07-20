# contains

Test whether a list holds an item equal to a given value.

## Synopsis

```
collections::contains OF T(value AS List OF T, item AS T) AS Boolean
```

## Package

collections

## Imports

```
IMPORT collections
```

`collections` is a built-in package, so no manifest dependency is required.
[[src/builtins/collections.rs:is_collections_call]]

`contains` is a native `collections::` member and must be called with the
`collections::` qualifier; there is no bare `contains` built-in.
[[src/builtins/collections.rs:is_native_member]]

## Description

`collections::contains` scans `value` from index `0` upward and returns `TRUE`
as soon as an element matches `item`, or `FALSE` after every element has been
examined without a match. The list is neither copied nor mutated, and no element
payload is materialized — the scan compares stored bytes in place.
[[src/target/shared/code/builder_collection_queries.rs:lower_collection_contains]]

This is a list-only member. It does not accept a `Map`, and it is not the
substring test: the `String` form of `contains` lives in the `strings::`
package, not here. [[src/builtins/collections.rs:expected_arguments]]

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

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `List OF T` | The list to scan, examined left to right. Also accepted under the name `collection`. Not copied and not mutated. [[src/builtins/collections.rs:call_param_names]] |
| `item` | `T` | The value to search for. Must be exactly the list's element type. [[src/builtins/collections.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` when some element of `value` matches `item`; `FALSE` when none does, including for an empty list. [[src/builtins/general.rs:resolve_contains]] |

## Errors

No errors.

## Type checking

`collections::contains` takes exactly two arguments.
[[src/builtins/collections.rs:arity]]

The first must be a `List OF T`; a `Map`, a `String`, or any other value is a
compile-time type error. The second must be exactly the element type `T` — a
`List OF Integer` cannot be searched with a `String`, and there is no implicit
conversion between numeric element types. The result is always `Boolean`.
[[src/builtins/general.rs:resolve_contains]]

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

## See also

- `mfb man collections find`
- `mfb man collections hasKey`
- `mfb man collections filter`
- `mfb man collections distinct`
- `mfb man collections`
