# all

Test whether every element of a list satisfies a predicate

## Synopsis

```
collections::all OF T(value AS List OF T, predicate AS FUNC(T) AS Boolean) AS Boolean
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

`collections::all` walks `value` from index `0` upward and calls `predicate`
with each element in turn. It returns `FALSE` as soon as a call returns `FALSE`,
without examining any later element, and returns `TRUE` only after every element
has been tested and all matched. [[src/builtins/collections_package.mfb:__collections_all]]

The scan short-circuits: `predicate` is called at most once per element, and no
call is made for elements after the first non-matching one. Callers must not
rely on `predicate` being invoked for the whole list.

For an empty list `all` returns `TRUE`, the vacuous result: there is no element
that fails the test. This is the dual of `collections::any`, which returns
`FALSE` for an empty list.

`predicate` is an ordinary function value of type `FUNC(T) AS Boolean` — a named
`FUNC` or a `LAMBDA`. Because it is called as an ordinary call, an error raised
inside `predicate` is **not** absorbed by `all`: it propagates out of the
`collections::all` call to the caller, where a function-level or inline `TRAP`
may catch it. `all` itself defines no error of its own. Note that a lambda
passed here may not capture an outer `MUT` binding; the callback position proven
non-escaping is `collections::forEach`, not `all`. [[src/builtins/mod.rs:is_nonescaping_callback_arg]]

`all` is a generic implemented in MFBASIC source; a call is rewritten to the
internal `__collections_all` generic and instantiated for the element type like
any other generic function. [[src/builtins/collections.rs:FUNCTIONS]] It does not
mutate `value` and has no other side effects beyond whatever `predicate` does.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `List OF T` | The list to scan, in index order starting at `0`. An empty list is accepted and yields `TRUE`. Not modified. [[src/builtins/collections_package.mfb:__collections_all]] |
| `predicate` | `FUNC(T) AS Boolean` | Test applied to each element. Called with one element at a time; the scan stops at the first call that returns `FALSE`. An error it raises propagates to the caller. [[src/builtins/collections_package.mfb:__collections_all]] |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` when `predicate` returns `TRUE` for every element of `value`, including when `value` is empty; `FALSE` when at least one element fails the test. [[src/builtins/collections_package.mfb:__collections_all]] |

## Errors

No errors.

## Type checking

`T` is inferred from the element type of `value` and may be any type; `all`
imposes no comparability or orderability constraint on `T`, because elements are
never compared to one another — they are only passed to `predicate`. The second
argument must be a function value taking exactly one `T` and returning
`Boolean`. [[src/builtins/collections_package.mfb:__collections_all]]

## Examples

Test that every integer in a list is positive:

```
IMPORT io
IMPORT collections

FUNC isPos(n AS Integer) AS Boolean
  RETURN n > 0
END FUNC

FUNC main AS Integer
  io::print(toString(collections::all([1, 2, 3], isPos)))
  RETURN 0
END FUNC
```

An empty list satisfies every predicate:

```
IMPORT io
IMPORT collections

FUNC isPos(n AS Integer) AS Boolean
  RETURN n > 0
END FUNC

FUNC main AS Integer
  LET empty AS List OF Integer = []
  io::print(toString(collections::all(empty, isPos)))
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
  io::print(toString(collections::all(value := [1, 0], predicate := isPos)))
  RETURN 0
END FUNC
```

## See also

- `mfb man collections any`
- `mfb man collections filter`
- `mfb man collections findIndex`
- `mfb man collections partition`
- `mfb man collections contains`
