# isEmpty

Test whether a string, list, or map has no contents.

## Synopsis

```
isEmpty(value AS String) AS Boolean
isEmpty(value AS List OF T) AS Boolean
isEmpty(value AS Map OF K TO V) AS Boolean
```

## Package

general

## Imports

None. `general` functions are always available without an `IMPORT` statement. [[src/builtins/general.rs:is_general_call]]

## Description

`isEmpty` returns `TRUE` when `value` has zero length and `FALSE` otherwise. It
computes `len(value)` and reports whether that count is `0`, so it is the logical
negation of `isNotEmpty` and uses exactly the same length rules as `len`.
[[src/target/shared/code/builder_conversions.rs:lower_empty_filter_predicate]]

The notion of length depends on the argument type. For a `String`, emptiness is
measured by Unicode scalar count, so `isEmpty` is `TRUE` only for the zero-length
string `""`; a string of whitespace or any other scalar is not empty. For a
`List OF T`, emptiness is measured by element count, so `isEmpty` is `TRUE` only
for a list with no elements, for any element type `T`. For a `Map OF K TO V`,
emptiness is measured by entry count, so `isEmpty` is `TRUE` only for a map with
no key/value entries. [[src/target/shared/code/builder_collection_layout.rs:lower_len]]

`isEmpty` reads `value` only; it has no side effects and never mutates its
argument. The accepted argument type is resolved at compile time: an argument
that is not a `String`, `List OF T`, or `Map OF K TO V` is rejected during type
checking rather than at run time. `isEmpty` is lowered inline at a direct call
site, and out of line where it is named as a function value, so it may be passed
as a predicate anywhere an ordinary `FUNC` may be. The value form resolves
against the type expected at that position (bug-368). The same predicate is also exposed through
the `filters` package. [[src/builtins/general.rs:resolve_call]]

## Overloads

**`isEmpty(value AS String) AS Boolean`**

Returns `TRUE` when the string has no scalar values.

**`isEmpty(value AS List OF T) AS Boolean`**

Returns `TRUE` when the list has no elements.

**`isEmpty(value AS Map OF K TO V) AS Boolean`**

Returns `TRUE` when the map has no entries.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The string to test. `TRUE` is returned only for the empty string `""`. |
| `value` | `List OF T` | The list to test, holding elements of any type `T`. `TRUE` is returned only when the list has no elements. |
| `value` | `Map OF K TO V` | The map to test, with keys of type `K` and values of type `V`. `TRUE` is returned only when the map has no entries. |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` when `value` has zero length (the empty string, an empty list, or an empty map); `FALSE` when `value` has at least one scalar, element, or entry. |

## Errors

No errors.

## Type checking

`isEmpty` is generic over its single argument and accepts a `String`, a
`List OF T`, or a `Map OF K TO V`. It takes exactly one argument; any other arity
or an argument of any other type is rejected at compile time. The element type
`T` and the map key and value types `K` and `V` are unconstrained. Like other
`general` predicates it may be overridden by a user- or package-defined `FUNC` of
the same name for its own value types. [[src/builtins/general.rs:resolve_call]]

## Examples

Test text:

```
SUB main()
  LET ok AS Boolean = isEmpty("")
END SUB
```

Test a list:

```
SUB main()
  LET values AS List OF Integer = []
  LET ok AS Boolean = isEmpty(values)
END SUB
```

Test a map:

```
SUB main()
  LET scores AS Map OF String TO Integer = Map OF String TO Integer {}
  LET ok AS Boolean = isEmpty(scores)
END SUB
```

Use it as a predicate by wrapping it in a `LAMBDA`:

```
IMPORT collections

SUB main()
  LET blanks AS List OF String = collections::filter(["a", "", "b"], LAMBDA(s AS String) -> isEmpty(s))
END SUB
```

## See also

- `mfb man general isNotEmpty`
- `mfb man general len`
- `mfb man filters isEmpty`
