# transform

Map every element of a list through a function and collect the results

## Synopsis

```
collections::transform OF T, U(value AS List OF T, f AS FUNC(T) AS U) AS List OF U
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

`collections::transform` walks `value` from the first element to the last,
calls `f` once per element with that element as its only argument, and appends
each returned value to a new list. The result therefore has exactly as many
elements as `value`, in the same order. It is a **native** member: the compiler
emits the mapping loop directly rather than instantiating an MFBASIC generic.
[[src/builtins/collections.rs:is_native_member]]
[[src/target/shared/code/builder_collection_queries.rs:lower_collection_transform_call]]

The element type of the result is `f`'s success type `U`, so mapping a
`List OF Integer` through a `FUNC(Integer) AS String` yields a `List OF String`.
`U` may differ from `T` or equal it. [[src/builtins/general.rs:resolve_transform]]

`f` must be a callable *value* — a reference to a declared `FUNC`, or a
`LAMBDA`. An overloaded built-in such as `toString` is not a callable value and
cannot be passed here; wrap it in a one-line `FUNC` of your own instead. The
single-argument `general` predicates (`isEven`, `isOdd`, and friends) *are*
ordinary callables and can be passed directly where their type fits.
[[src/builtins/general.rs:arity]]

`f` must produce a value: a callback whose success type is `Nothing` — such as a
`SUB` — does not resolve, because there would be nothing to collect. Use
`collections::forEach` to run a callback purely for its side effects.
[[src/builtins/general.rs:resolve_transform]]

`value` is neither modified nor consumed; the result is a freshly allocated
list. The output is pre-sized to the source list's working set, since
`transform` emits exactly one entry per source element, and each mapped value is
then appended in place.
[[src/target/shared/code/builder_collection_queries.rs:lower_collection_transform_call]]

An empty `value` calls `f` zero times and yields an empty `List OF U`.

`transform` raises no domain error of its own. It is classified fallible solely
because a failing `f` propagates: when the callback returns a non-`Ok` result,
the loop stops immediately at that element, later elements are never visited, no
result list is produced, and the callback's own error is passed through
unchanged. The partially built output is freed on that path before the error
leaves. [[src/builtins/mod.rs:inline_builtin_is_infallible]]
[[src/target/shared/code/builder_collection_queries.rs:emit_callback_failure_exit]]

An inline `TRAP` on a `transform` call captures that propagated callback error
at the call site rather than letting it auto-propagate.
[[src/builtins/mod.rs:inline_builtin_raw_supported]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `List OF T` | The list to map. Any length is accepted, including the empty list. Also accepted under the name `collection`. [[src/builtins/collections.rs:call_param_names]] |
| `f` | `FUNC(T) AS U` | Called once per element with that element; its return value becomes the corresponding result element. Must take exactly one parameter of the element type `T`, and its success type `U` must not be `Nothing`. Also accepted under the name `transform`. [[src/builtins/general.rs:resolve_transform]] [[src/builtins/collections.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF U` | A new list holding `f`'s result for each element of `value`, in source order, with the same length as `value`. For an empty `value`, an empty `List OF U`. [[src/builtins/general.rs:resolve_transform]] [[src/target/shared/code/builder_collection_queries.rs:lower_collection_transform_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| — | any error raised by `f` | The callback fails for some element. The error is propagated unchanged, iteration stops at that element, and the partial result list is freed; `transform` defines no error code of its own. [[src/target/shared/code/builder_collection_queries.rs:emit_callback_failure_exit]] |

## Type checking

`T` is inferred from `value`, which must be a `List`. `f` must be a callable of
exactly one parameter whose type equals the element type `T`, and its success
type `U` must be anything other than `Nothing`. `U` then fixes the result type
as `List OF U`. Passing a non-list first argument, an `f` of the wrong arity, an
`f` whose parameter type differs from `T`, or an `f` that returns `Nothing` is a
compile-time type error — no overload resolves.
[[src/builtins/general.rs:resolve_transform]]
[[src/builtins/collections.rs:expected_arguments]]

## Examples

Map integers to strings, changing the element type:

```
IMPORT collections
IMPORT io

FUNC label(value AS Integer) AS String
  RETURN "n=" & toString(value)
END FUNC

FUNC main AS Integer
  LET labels AS List OF String = collections::transform([1, 2, 3], label)
  io::print(collections::get(labels, 0))
  RETURN 0
END FUNC
```

Map with a `LAMBDA`, keeping the element type:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET tripled AS List OF Integer = collections::transform([1, 2, 3], LAMBDA(value AS Integer) -> value * 3)
  io::print(toString(collections::get(tripled, 2)))
  RETURN 0
END FUNC
```

An empty input yields an empty result:

```
IMPORT collections
IMPORT io

FUNC double(value AS Integer) AS Integer
  RETURN value * 2
END FUNC

FUNC main AS Integer
  LET empty AS List OF Integer = []
  LET mapped AS List OF Integer = collections::transform(empty, double)
  io::print(toString(len(mapped)))
  RETURN 0
END FUNC
```

## See also

- `mfb man collections forEach`
- `mfb man collections filter`
- `mfb man collections reduce`
- `mfb man collections`
