# flatten

Concatenate a list of lists into a single list, one level deep

## Synopsis

```
collections::flatten OF T(value AS List OF List OF T) AS List OF T
```

## Package

collections

## Imports

```
IMPORT collections
```

`collections` is a built-in package, so no manifest dependency is required.
[[src/builtins/collections.rs:FUNCTIONS]]

## Description

`collections::flatten` walks `value` from index 0 upward and concatenates each
inner list onto an accumulating result. It does this by calling
`collections::append(result, inner)` where `inner` is itself a `List OF T` — that
is the list-concatenation overload of `append`, which accepts a second argument
that is either the element type or the same list type as the first argument.
Each inner list is therefore spliced in whole rather than nested as a single
element. [[src/builtins/collections_package.mfb:__collections_flatten]] [[src/builtins/general.rs:resolve_append]]

`flatten` removes exactly **one** level of nesting. Its parameter type is
`List OF List OF T`, so applying it to a `List OF List OF List OF Integer`
produces a `List OF List OF Integer` — the innermost lists survive as elements.
Flattening further requires calling `flatten` again on the result. It is not
recursive and there is no depth parameter.
[[src/builtins/collections_package.mfb:__collections_flatten]]

Order is fully preserved: the inner lists are consumed in their own order, and
the items within each inner list keep their relative order, so the result reads
as the inner lists laid end to end. Empty inner lists contribute nothing and are
simply skipped over; they do not produce a placeholder element. When `value`
itself is empty, the result is an empty list.

`value` is not modified, and neither are the inner lists it holds; the result is
a newly built list. `flatten` invokes no user callback and raises no error.

Note that the template argument `T` is inferred from the argument, so a bare
untyped `[]` literal cannot be passed directly — bind it to a
`List OF List OF T` first, or pass an expression whose type is known.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `List OF List OF T` | The list of inner lists to concatenate. May be empty, and any inner list may be empty. Not modified. [[src/builtins/collections_package.mfb:__collections_flatten]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF T` | A new list holding every item of every inner list of `value`, inner lists in order and items within each inner list in order. Empty when `value` is empty or when every inner list is empty. [[src/builtins/collections_package.mfb:__collections_flatten]] |

## Errors

No errors.

## Type checking

`flatten` is generic over a single template parameter `T`, the element type of
the inner lists. It is inferred from the argument, which must be a
`List OF List OF T`; a plain `List OF T` does not match, and a doubly nested
`List OF List OF List OF T` binds `T` to `List OF ...` and so flattens only its
outermost level. [[src/builtins/collections_package.mfb:__collections_flatten]]

## Examples

Concatenate three inner lists:

```
IMPORT io
IMPORT collections

FUNC main AS Integer
  LET nested AS List OF List OF Integer = [[1, 2], [3], [4, 5]]
  LET flat AS List OF Integer = collections::flatten(nested)
  io::print(toString(len(flat)))
  RETURN 0
END FUNC
```

Empty inner lists contribute nothing:

```
IMPORT io
IMPORT collections

FUNC main AS Integer
  LET nested AS List OF List OF String = [["a"], [], ["b", "c"]]
  LET flat AS List OF String = collections::flatten(value := nested)
  io::print(collections::get(flat, 1))
  RETURN 0
END FUNC
```

Only one level is removed, so flattening twice is two calls:

```
IMPORT io
IMPORT collections

FUNC main AS Integer
  LET deep AS List OF List OF List OF Integer = [[[1, 2], [3]], [[4]]]
  LET once AS List OF List OF Integer = collections::flatten(deep)
  LET twice AS List OF Integer = collections::flatten(once)
  io::print(toString(len(once)) & " " & toString(len(twice)))
  RETURN 0
END FUNC
```

## See also

- `mfb man collections append`
- `mfb man collections chunks`
- `mfb man collections window`
- `mfb man collections zip`
- `mfb man collections transform`
