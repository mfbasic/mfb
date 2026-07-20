# reduce

Fold a list left to right into a single accumulated value

## Synopsis

```
collections::reduce OF T, U(value AS List OF T, initial AS U, f AS FUNC(U, T) AS U) AS U
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

`collections::reduce` folds `value` into one value. The accumulator starts as
`initial`. The list is walked from the first element to the last, and for each
element the reducer is called as `f(accumulator, element)` — **accumulator
first, element second** — with its return value becoming the accumulator for the
next step. The accumulator left after the final element is the result. It is a
**native** member: the compiler emits the fold loop directly rather than
instantiating an MFBASIC generic. [[src/builtins/collections.rs:is_native_member]]
[[src/target/shared/code/builder_collection_queries.rs:lower_collection_reduce_call]]

The fold direction is left, from index 0 upward: the loop starts at the head of
the entry table and advances one entry per step. For a right-to-left fold, use
`collections::reduceRight`.
[[src/target/shared/code/builder_collection_queries.rs:lower_collection_reduce_call]]

The accumulator type `U` is fixed by `initial`. `f`'s first parameter type, its
success type, and the type of `initial` must all be that same `U`, while `f`'s
second parameter must be the list element type `T`. `U` may differ from `T`, so
a `List OF String` can be folded into an `Integer`.
[[src/builtins/general.rs:resolve_reduce]]

When `value` is empty, the loop body never runs, `f` is never called, and
`initial` is returned unchanged.
[[src/target/shared/code/builder_collection_queries.rs:lower_collection_reduce_call]]

`value` is not modified. Unlike the other three callback members, `reduce`
deliberately does not free the per-element item it materializes for the
callback, because the reducer is allowed to return that item itself as the new
accumulator — freeing it would turn a leak into a use-after-free. Intermediate
accumulators are likewise left unfreed.
[[src/target/shared/code/builder_collection_queries.rs:lower_collection_reduce_call]]

`reduce` raises no domain error of its own. It is classified fallible solely
because a failing `f` propagates: when the reducer returns a non-`Ok` result,
the fold stops immediately at that element, later elements are never visited,
and the reducer's own error is passed through unchanged. No cleanup runs on that
path, since the accumulator may still alias the borrowed `initial`.
[[src/builtins/mod.rs:inline_builtin_is_infallible]]
[[src/target/shared/code/builder_collection_queries.rs:emit_callback_failure_exit]]

An inline `TRAP` on a `reduce` call captures that propagated reducer error at
the call site rather than letting it auto-propagate.
[[src/builtins/mod.rs:inline_builtin_raw_supported]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `List OF T` | The list to fold. Any length is accepted, including the empty list. Also accepted under the name `collection`. [[src/builtins/collections.rs:call_param_names]] |
| `initial` | `U` | The starting accumulator, and the value returned unchanged when `value` is empty. Its type fixes `U`. Also accepted under the name `seed`. [[src/builtins/general.rs:resolve_reduce]] [[src/builtins/collections.rs:call_param_names]] |
| `f` | `FUNC(U, T) AS U` | The reducer, called once per element as `f(accumulator, element)`. Its first parameter must be the accumulator type `U`, its second the element type `T`, and its success type `U`. Also accepted under the name `combine`. [[src/builtins/general.rs:resolve_reduce]] [[src/builtins/collections.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `U` | The accumulator after the last element, that is, the result of the final `f` call. For an empty `value`, `initial` unchanged. [[src/builtins/general.rs:resolve_reduce]] [[src/target/shared/code/builder_collection_queries.rs:lower_collection_reduce_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| — | any error raised by `f` | The reducer fails for some element. The error is propagated unchanged and the fold stops at that element; `reduce` defines no error code of its own. [[src/target/shared/code/builder_collection_queries.rs:emit_callback_failure_exit]] |

## Type checking

`T` is inferred from `value`, which must be a `List`. `U` is inferred from
`initial`. `f` must be a callable of exactly two parameters: the first of type
`U`, the second of the element type `T`, with success type `U`. All three
constraints are checked together, so an `initial` whose type does not match both
`f`'s first parameter and `f`'s return type is a compile-time type error, as is
a reducer with the two parameters in the opposite order. No overload resolves in
those cases. [[src/builtins/general.rs:resolve_reduce]]
[[src/builtins/collections.rs:expected_arguments]]

## Examples

Sum a list with an explicit reducer:

```
IMPORT collections
IMPORT io

FUNC add(total AS Integer, value AS Integer) AS Integer
  RETURN total + value
END FUNC

FUNC main AS Integer
  LET total AS Integer = collections::reduce([1, 2, 3], 10, add)
  io::print(toString(total))
  RETURN 0
END FUNC
```

Fold a `List OF String` into a single `String`, showing that `U` need not equal
`T`'s usual result:

```
IMPORT collections
IMPORT io

FUNC join(text AS String, word AS String) AS String
  RETURN text & word
END FUNC

FUNC main AS Integer
  LET joined AS String = collections::reduce(["hello", "world"], "", join)
  io::print(joined)
  RETURN 0
END FUNC
```

An empty list returns `initial` without calling the reducer:

```
IMPORT collections
IMPORT io

FUNC add(total AS Integer, value AS Integer) AS Integer
  RETURN total + value
END FUNC

FUNC main AS Integer
  LET empty AS List OF Integer = []
  io::print(toString(collections::reduce(empty, 7, add)))
  RETURN 0
END FUNC
```

## See also

- `mfb man collections reduceRight`
- `mfb man collections sum`
- `mfb man collections transform`
- `mfb man collections filter`
- `mfb man collections`
