# any

Test whether at least one element of a list satisfies a predicate

## Synopsis

```
collections::any OF T(value AS List OF T, predicate AS FUNC(T) AS Boolean) AS Boolean
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

`collections::any` walks `value` from index `0` upward and calls `predicate`
with each element in turn. It returns `TRUE` as soon as a call returns `TRUE`,
without examining any later element, and returns `FALSE` only after every
element has been tested and none matched. [[src/builtins/collections_package.mfb:__collections_any]]

The scan short-circuits: `predicate` is called at most once per element, and no
call is made for elements after the first match. Callers must not rely on
`predicate` being invoked for the whole list.

For an empty list `any` returns `FALSE`, since there is no element that could
match. This is the dual of `collections::all`, which returns `TRUE` for an
empty list.

`predicate` is an ordinary function value of type `FUNC(T) AS Boolean` — a named
`FUNC` or a `LAMBDA`. Because it is called as an ordinary call, an error raised
inside `predicate` is **not** absorbed by `any`: it propagates out of the
`collections::any` call to the caller, where a function-level or inline `TRAP`
may catch it. `any` itself defines no error of its own. Note that a lambda
passed here may not capture an outer `MUT` binding; the callback position proven
non-escaping is `collections::forEach`, not `any`. [[src/builtins/mod.rs:is_nonescaping_callback_arg]]

`any` is a generic implemented in MFBASIC source; a call is rewritten to the
internal `__collections_any` generic and instantiated for the element type like
any other generic function. [[src/builtins/collections.rs:FUNCTIONS]] It does not
mutate `value` and has no other side effects beyond whatever `predicate` does.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `List OF T` | The list to scan, in index order starting at `0`. An empty list is accepted and yields `FALSE`. Not modified. [[src/builtins/collections_package.mfb:__collections_any]] |
| `predicate` | `FUNC(T) AS Boolean` | Test applied to each element. Called with one element at a time; the scan stops at the first call that returns `TRUE`. An error it raises propagates to the caller. [[src/builtins/collections_package.mfb:__collections_any]] |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` when `predicate` returns `TRUE` for at least one element of `value`; `FALSE` when it returns `FALSE` for every element, including when `value` is empty. [[src/builtins/collections_package.mfb:__collections_any]] |

## Errors

No errors.

## Type checking

`T` is inferred from the element type of `value` and may be any type; `any`
imposes no comparability or orderability constraint on `T`, because elements are
never compared to one another — they are only passed to `predicate`. The second
argument must be a function value taking exactly one `T` and returning
`Boolean`. [[src/builtins/collections_package.mfb:__collections_any]]

## Examples

Test a list of integers for a positive element:

```
IMPORT io
IMPORT collections

FUNC isPos(n AS Integer) AS Boolean
  RETURN n > 0
END FUNC

FUNC main AS Integer
  io::print(toString(collections::any([-1, 0, 3], isPos)))
  RETURN 0
END FUNC
```

An empty list never matches:

```
IMPORT io
IMPORT collections

FUNC isPos(n AS Integer) AS Boolean
  RETURN n > 0
END FUNC

FUNC main AS Integer
  LET empty AS List OF Integer = []
  io::print(toString(collections::any(empty, isPos)))
  RETURN 0
END FUNC
```

Named arguments bind by the declared parameter names `value` and `predicate`:

```
IMPORT io
IMPORT collections

FUNC isPos(n AS Integer) AS Boolean
  RETURN n > 0
END FUNC

FUNC main AS Integer
  io::print(toString(collections::any(value := [-1, 2], predicate := isPos)))
  RETURN 0
END FUNC
```

## See also

- `mfb man collections all`
- `mfb man collections findIndex`
- `mfb man collections contains`
- `mfb man collections filter`
- `mfb man collections partition`
