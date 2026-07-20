# filter

Keep the elements of a list for which a predicate returns TRUE

## Synopsis

```
collections::filter OF T(value AS List OF T, predicate AS FUNC(T) AS Boolean) AS List OF T
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

`collections::filter` walks `value` from the first element to the last, calls
`predicate` once per element, and appends the element to a new list when the
predicate returns `TRUE`. Elements for which the predicate returns `FALSE` are
skipped. It is a **native** member: the compiler emits the selection loop
directly rather than instantiating an MFBASIC generic.
[[src/builtins/collections.rs:is_native_member]]
[[src/target/shared/code/builder_collection_queries.rs:lower_collection_filter_call]]

Relative order is preserved: kept elements appear in the result in the same
order they had in `value`. The result has the same type as `value`, so filtering
a `List OF String` yields a `List OF String`, and its length is between zero and
the length of `value`. [[src/builtins/general.rs:resolve_filter]]

`value` is neither modified nor consumed; the result is a freshly allocated
list, pre-sized to the source so the per-element append never has to regrow.
[[src/target/shared/code/builder_collection_queries.rs:lower_collection_filter_call]]

`predicate` must accept exactly one argument of the element type `T` and return
`Boolean`. This is enforced both when the call is resolved and again in the
lowering. [[src/builtins/general.rs:resolve_filter]]
[[src/target/shared/code/builder_collection_queries.rs:lower_collection_filter_call]]

The single-argument `general` predicates — `isEven`, `isOdd`, `isPositive`,
`isNegative`, `isZero`, `isEmpty`, and `isNotEmpty` — are ordinary
`FUNC(T) AS Boolean` callables and can be passed directly whenever their
argument type matches the element type. [[src/builtins/general.rs:arity]]

An empty `value` calls `predicate` zero times and yields an empty list.

`filter` raises no domain error of its own. It is classified fallible solely
because a failing `predicate` propagates: when the callback returns a non-`Ok`
result, the loop stops immediately at that element, later elements are never
visited, no result list is produced, and the callback's own error is passed
through unchanged. The partially built output is freed on that path before the
error leaves. [[src/builtins/mod.rs:inline_builtin_is_infallible]]
[[src/target/shared/code/builder_collection_queries.rs:emit_callback_failure_exit]]

An inline `TRAP` on a `filter` call captures that propagated callback error at
the call site rather than letting it auto-propagate.
[[src/builtins/mod.rs:inline_builtin_raw_supported]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `List OF T` | The list to filter. Any length is accepted, including the empty list. Also accepted under the name `collection`. [[src/builtins/collections.rs:call_param_names]] |
| `predicate` | `FUNC(T) AS Boolean` | Called once per element with that element; the element is kept when it returns `TRUE`. Must take exactly one parameter of the element type `T` and return `Boolean`. There is no alternate name for this parameter. [[src/builtins/general.rs:resolve_filter]] [[src/builtins/collections.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF T` | A new list of the same type as `value`, holding the elements for which `predicate` returned `TRUE`, in their original relative order. Empty when no element is kept or when `value` is empty. [[src/builtins/general.rs:resolve_filter]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| — | any error raised by `predicate` | The callback fails for some element. The error is propagated unchanged, iteration stops at that element, and the partial result list is freed; `filter` defines no error code of its own. [[src/target/shared/code/builder_collection_queries.rs:emit_callback_failure_exit]] |

## Type checking

`T` is inferred from `value`, which must be a `List`. `predicate` must be a
callable of exactly one parameter whose type equals the element type `T` and
whose success type is exactly `Boolean`. The result type is the same
`List OF T`. Passing a non-list first argument, a `predicate` of the wrong
arity, a `predicate` whose parameter type differs from `T`, or a `predicate`
that returns anything other than `Boolean` is a compile-time type error — no
overload resolves. [[src/builtins/general.rs:resolve_filter]]
[[src/builtins/collections.rs:expected_arguments]]

## Examples

Keep the even numbers with a built-in predicate:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET evens AS List OF Integer = collections::filter([1, 2, 3, 4], isEven)
  io::print(toString(len(evens)))
  RETURN 0
END FUNC
```

Keep the non-empty strings:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET names AS List OF String = collections::filter(["Ada", "", "Grace"], isNotEmpty)
  io::print(collections::get(names, 0))
  RETURN 0
END FUNC
```

Filter with a `LAMBDA`:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET small AS List OF Integer = collections::filter([1, 2, 5, 9], LAMBDA(value AS Integer) -> value < 3)
  io::print(toString(len(small)))
  RETURN 0
END FUNC
```

## See also

- `mfb man collections transform`
- `mfb man collections forEach`
- `mfb man collections reduce`
- `mfb man collections partition`
- `mfb man collections`
