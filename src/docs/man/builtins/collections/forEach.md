# forEach

Call an action once for each element of a list, in order

## Synopsis

```
collections::forEach OF T(value AS List OF T, action AS FUNC(T) AS Nothing) AS Nothing
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

`collections::forEach` walks `value` from the first element to the last and
calls `action` once per element, passing the element as the single argument. It
is a **native** member: the compiler emits the traversal loop directly rather
than instantiating an MFBASIC generic. [[src/builtins/collections.rs:is_native_member]]
[[src/target/shared/code/builder_collection_queries.rs:lower_collection_for_each_call]]

The loop is a straight forward scan over the list's entry table with no
reordering and no skipping, so `action` observes exactly the elements of `value`
in their stored order. `value` is neither copied nor modified; `forEach` builds
no result collection at all and evaluates to `Nothing`.
[[src/target/shared/code/builder_collection_queries.rs:lower_collection_for_each_call]]

`action` must accept exactly one argument of the element type `T` and its
success type must be `Nothing`. A `SUB` is therefore accepted directly, since a
`SUB` has success type `Nothing`; a `FUNC` that produces a value is rejected at
compile time. To collect results instead of discarding them, use
`collections::transform`. [[src/builtins/general.rs:resolve_for_each]]

`action` must be a callable *value* — a reference to a declared `SUB` or `FUNC`.
A package member such as `io::print` is not a callable value and cannot be
passed here; wrap it in a `SUB` of your own, as the first example below does.

`action` is invoked through the shared direct-callable path, which restores a
closure's captured environment around each call, so a callable value that
carries an environment works as well as a plain named reference.
[[src/target/shared/code/builder_collection_queries.rs:emit_direct_callable_branch]]

`forEach` raises no domain error of its own. It is classified fallible solely
because a failing `action` propagates: when the callback returns a non-`Ok`
result, the loop stops immediately at that element, later elements are never
visited, and the callback's own error is passed straight through — unchanged, so
whatever code and message the callback raised is what the caller sees. Because
`forEach` owns no accumulator, no cleanup runs on that path.
[[src/builtins/mod.rs:inline_builtin_is_infallible]]
[[src/target/shared/code/builder_collection_queries.rs:emit_callback_failure_exit]]

An inline `TRAP` on a `forEach` call captures that propagated callback error at
the call site rather than letting it auto-propagate.
[[src/builtins/mod.rs:inline_builtin_raw_supported]]

An empty `value` calls `action` zero times.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `List OF T` | The list to walk. Any length is accepted, including the empty list. Also accepted under the name `collection`. [[src/builtins/collections.rs:call_param_names]] |
| `action` | `FUNC(T) AS Nothing` | Called once per element with that element. Must take exactly one parameter of the element type `T` and have success type `Nothing`, so a `SUB(T)` qualifies. There is no alternate name for this parameter. [[src/builtins/general.rs:resolve_for_each]] [[src/builtins/collections.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | `forEach` produces no value. [[src/builtins/general.rs:resolve_for_each]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| — | any error raised by `action` | The callback fails for some element. The error is propagated unchanged and iteration stops at that element; `forEach` defines no error code of its own. [[src/target/shared/code/builder_collection_queries.rs:emit_callback_failure_exit]] |

## Type checking

`T` is inferred from `value`, which must be a `List`. `action` must be a
callable of exactly one parameter whose type equals the element type `T` and
whose success type is exactly `Nothing`. Passing a non-list first argument, an
`action` of the wrong arity, an `action` whose parameter type differs from `T`,
or an `action` that returns a value is a compile-time type error — no overload
resolves. [[src/builtins/general.rs:resolve_for_each]]
[[src/builtins/collections.rs:expected_arguments]]

## Examples

Print every element with a `SUB`:

```
IMPORT collections
IMPORT io

SUB show(item AS String)
  io::print(item)
END SUB

FUNC main AS Integer
  LET names AS List OF String = ["Ada", "Grace"]
  collections::forEach(names, show)
  RETURN 0
END FUNC
```

The list is left untouched by the walk:

```
IMPORT collections
IMPORT io

SUB report(value AS Integer)
  io::print(toString(value))
END SUB

FUNC main AS Integer
  LET numbers AS List OF Integer = [1, 2, 3]
  collections::forEach(numbers, report)
  io::print(toString(len(numbers)))
  RETURN 0
END FUNC
```

## See also

- `mfb man collections transform`
- `mfb man collections filter`
- `mfb man collections reduce`
- `mfb man collections`
