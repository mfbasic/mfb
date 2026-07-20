# distinct

Remove duplicate elements from a list, keeping the first occurrence of each

## Synopsis

```
collections::distinct OF T(value AS List OF T) AS List OF T
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

`collections::distinct` returns a new list holding the elements of `value` with
duplicates removed. It walks `value` from index `0` upward and appends each
element to the result only when the result does not already contain an element
equal to it, so the **first** occurrence of each distinct value is the one kept
and later duplicates are dropped. [[src/builtins/collections_package.mfb:__collections_distinct]]

First-occurrence order is preserved: the surviving elements appear in the
result in the same relative order they had in `value`. The input is not
modified — `distinct` builds and returns a separate list. An empty input yields
an empty result.

Membership is tested with `collections::contains`, so "equal" here means exactly
the element equality that `contains` uses, and nothing else — there is no
user-supplied comparison and no key-extraction overload. [[src/builtins/collections_package.mfb:__collections_distinct]] [[src/builtins/general.rs:resolve_contains]]
That equality is applied per element type: `Integer`, `Fixed`, `Money`,
`Boolean`, `Byte`, and `Scalar` compare by value; `String` compares by length
and then byte-for-byte over its UTF-8 bytes; a record compares field by field.
[[src/target/shared/code/builder_collection_compare.rs:emit_collection_payload_match_branch]]

Two consequences of that equality deserve care:

- **`Float` is compared bitwise**, not with IEEE-754 numeric equality. `0.0` and
  `-0.0` are therefore treated as *distinct* values and both survive, while two
  `NaN` values with identical bit patterns are treated as *equal* and the second
  is dropped. This matches the packed-payload comparison used for `contains` and
  for map-literal keys. [[src/target/shared/code/builder_collection_compare.rs:emit_collection_payload_match_branch]]
- **String comparison is byte equality**, not Unicode-aware. Two strings that
  are canonically equivalent but differently normalized are distinct here; run
  `strings::normalizeNfc` (or `strings::caseFold` for case-insensitive
  deduplication) over the list first if that is not what you want.

`distinct` is O(n²) in the worst case: `contains` performs a linear scan of the
already-accumulated result for every input element, so a list with n distinct
elements does about n²/2 comparisons. [[src/target/shared/code/builder_collection_queries.rs:lower_collection_contains]]
For large inputs of a comparable key type, building a `Map` keyed by the element
and reading `collections::keys` is asymptotically cheaper, at the cost of losing
first-occurrence order.

`distinct` raises no user-trappable error of its own. It allocates while
building the result, but allocation failure is not a trappable domain error, and
the `append` it uses is classified infallible for exactly that reason.
[[src/builtins/mod.rs:inline_builtin_is_infallible]]

`distinct` is a generic implemented in MFBASIC source; a call is rewritten to
the internal `__collections_distinct` generic and instantiated for the element
type like any other generic function. [[src/builtins/collections.rs:FUNCTIONS]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `List OF T` | The list to deduplicate, scanned in index order from `0`. `T` must be a comparable type. An empty list is accepted. Not modified. [[src/builtins/collections_package.mfb:__collections_distinct]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF T` | A new list containing the first occurrence of each distinct element of `value`, in original relative order. Its length is between `0` and `len(value)`; it equals `len(value)` exactly when no two elements are equal. [[src/builtins/collections_package.mfb:__collections_distinct]] |

## Errors

No errors.

## Type checking

`T` is inferred from the element type of `value` and **must be comparable**,
because `distinct` is implemented in terms of `collections::contains`. A call
whose element type is not comparable is rejected at compile time with
`TYPE_REQUIRES_COMPARABLE`, reported against the internal `collections.contains`
call. [[src/ir/verify/mod.rs:check_builtin_call_args]]

Comparable types are `Integer`, `Float`, `Fixed`, `Money`, `Boolean`, `String`,
`Byte`, `Scalar`, `Nothing`, the built-in `Error` and `ErrorLoc` record shapes,
enum types, and records whose fields are all comparable. `List`, `Map`, `UNION`
types, `Result`, function values, threads, and resource handles are **not**
comparable, so `distinct` cannot be applied to a `List OF List OF T`, a list of
maps, or a list of resource handles. [[src/ir/verify/mod.rs:is_comparable_seen]]

## Examples

Deduplicate a list of integers:

```
IMPORT io
IMPORT collections

FUNC main AS Integer
  LET unique AS List OF Integer = collections::distinct([1, 2, 1, 3, 2])
  io::print(toString(len(unique)))
  RETURN 0
END FUNC
```

First occurrences are kept in their original order:

```
IMPORT io
IMPORT collections

FUNC main AS Integer
  LET names AS List OF String = collections::distinct(["b", "a", "b", "c"])
  io::print(collections::get(names, 0))
  io::print(collections::get(names, 1))
  io::print(collections::get(names, 2))
  RETURN 0
END FUNC
```

Normalize before deduplicating when Unicode equivalence matters:

```
IMPORT io
IMPORT collections
IMPORT strings

FUNC normalize(s AS String) AS String
  RETURN strings::normalizeNfc(s)
END FUNC

FUNC main AS Integer
  LET raw AS List OF String = ["a", "a", "b"]
  LET unique AS List OF String = collections::distinct(collections::transform(raw, normalize))
  io::print(toString(len(unique)))
  RETURN 0
END FUNC
```

The single parameter is named `value`:

```
IMPORT io
IMPORT collections

FUNC main AS Integer
  io::print(toString(len(collections::distinct(value := [1, 1, 2]))))
  RETURN 0
END FUNC
```

## See also

- `mfb man collections contains`
- `mfb man collections sort`
- `mfb man collections groupBy`
- `mfb man collections filter`
- `mfb man collections keys`
