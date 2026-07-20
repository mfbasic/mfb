# isNotEmpty

Test whether a string, list, or map has at least one element.

## Synopsis

```
isNotEmpty(value AS String) AS Boolean
isNotEmpty(value AS List OF T) AS Boolean
isNotEmpty(value AS Map OF K TO V) AS Boolean
```

## Package

general

## Imports

None. `general` functions are always available without an `IMPORT` statement. [[src/builtins/general.rs:is_general_call]]

## Description

`isNotEmpty` returns `TRUE` when `value` has a nonzero length and `FALSE`
otherwise. It computes `len(value)` and reports whether that count is not `0`, so
it is the logical negation of `isEmpty` and uses exactly the same length rules as
`len`. [[src/target/shared/code/builder_conversions.rs:lower_empty_filter_predicate]]

The notion of length depends on the argument type. For a `String`, non-emptiness
is measured by Unicode scalar count, so `isNotEmpty` is `TRUE` for any string that
holds at least one scalar (including a string of only whitespace) and `FALSE`
only for the zero-length string `""`. For a `List OF T`, non-emptiness is measured
by element count, so `isNotEmpty` is `TRUE` for any list with at least one
element, for any element type `T`. For a `Map OF K TO V`, non-emptiness is
measured by entry count, so `isNotEmpty` is `TRUE` for any map with at least one
key/value entry. [[src/target/shared/code/builder_collection_layout.rs:lower_len]]

`isNotEmpty` reads `value` only; it has no side effects and never mutates its
argument. The accepted argument type is resolved at compile time: an argument that
is not a `String`, `List OF T`, or `Map OF K TO V` is rejected during type
checking rather than at run time. `isNotEmpty` is lowered inline at a direct
call site, and out of line where it is named as a function value, so it may be
passed as a predicate anywhere an ordinary `FUNC` may be. The value form resolves
against the type expected at that position (bug-368). The same predicate is also exposed
through the `filters` package. [[src/builtins/general.rs:resolve_call]]

## Overloads

**`isNotEmpty(value AS String) AS Boolean`**

Returns `TRUE` when the string has at least one scalar value.

**`isNotEmpty(value AS List OF T) AS Boolean`**

Returns `TRUE` when the list has at least one element.

**`isNotEmpty(value AS Map OF K TO V) AS Boolean`**

Returns `TRUE` when the map has at least one entry.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The string to test. `FALSE` is returned only for the empty string `""`. |
| `value` | `List OF T` | The list to test, holding elements of any type `T`. `TRUE` is returned when the list has at least one element. |
| `value` | `Map OF K TO V` | The map to test, with keys of type `K` and values of type `V`. `TRUE` is returned when the map has at least one entry. |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` when `value` has at least one scalar, element, or entry; `FALSE` when `value` has zero length (the empty string, an empty list, or an empty map). |

## Errors

No errors.

## Type checking

`isNotEmpty` is generic over its single argument and accepts a `String`, a
`List OF T`, or a `Map OF K TO V`. It takes exactly one argument; any other arity
or an argument of any other type is rejected at compile time. The element type
`T` and the map key and value types `K` and `V` are unconstrained. Like other
`general` predicates it may be overridden by a user- or package-defined `FUNC` of
the same name for its own value types. [[src/builtins/general.rs:resolve_call]]

## Examples

Test text:

```
SUB main()
  LET ok AS Boolean = isNotEmpty("hello")
END SUB
```

Test a list:

```
SUB main()
  LET values AS List OF Integer = [1]
  LET ok AS Boolean = isNotEmpty(values)
END SUB
```

Test a map:

```
SUB main()
  LET scores AS Map OF String TO Integer = Map OF String TO Integer { "a" := 1 }
  LET ok AS Boolean = isNotEmpty(scores)
END SUB
```

Use it as a predicate by wrapping it in a `LAMBDA`:

```
IMPORT collections

SUB main()
  LET filled AS List OF String = collections::filter(["a", "", "b"], LAMBDA(s AS String) -> isNotEmpty(s))
END SUB
```

## See also

- `mfb man general isEmpty`
- `mfb man general len`
- `mfb man filters isNotEmpty`
