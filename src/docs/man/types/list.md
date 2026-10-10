# list

Ordered List values

## Synopsis

```
List OF T
```

## Description

`List OF T` is an ordered sequence. Every item has the same element type `T`,
and indexes are zero-based. A list value holds its items: binding a list with
`LET` creates an immutable snapshot, while binding a list with `MUT` creates a
binding you can change locally.

## Literals

List literals use bare square brackets. An empty list needs an expected type from
an annotation or surrounding context:

```
LET nums = [1, 2, 3]
LET empty AS List OF String = []
```

A long literal can run over several lines. Inside an unclosed `[ ]`, `{ }` or
`( )` a line break does not end the statement, so no `_` is needed. A comma may
follow the last item, a line may start with the comma instead, and a `'` comment
may end any line inside the brackets. The list is the same as if it were
written on one line:

```
LET codes AS List OF String = [
  "AED", "AFN", "ALL",   ' one row per region
  "AMD", "ANG",
]
LET more AS List OF String = [ "USD"
                             , "EUR"
                             ]
```

The same holds for map and set literals, record constructors, `WITH` updates,
the arguments of a call, and an expression in parentheses. A bracket that is
never closed is reported as `MFB_LEX_UNCLOSED_BRACKET` at the line that opened
it. `mfb fmt` keeps these layouts as written and moves a leading comma to the
end of the line before it (`mfb man tooling fmt`).

Brackets after a type name are a record constructor (`TypeName[...]`), never
indexing; there is no indexing-bracket syntax. All list access is through free
functions such as `collections::get`.

## What a list holds

A list holds its items directly, as part of the list value: numbers, strings,
records, unions and nested collections alike.

A list of a union's member type can be used wherever a list of the union is
expected: with `squares AS List OF Square` and `UNION Shape` containing `Square`,
`LET shapes AS List OF Shape = squares` and passing `squares` to a `List OF
Shape` parameter both work. The position receives a copy whose items are
`Shape` values; `squares` itself is unchanged. It does not work the other way: a
`List OF Shape` is not a `List OF Square`, because any of its items may be
another member. Use `MATCH` on each item to pick the members you want. A list of `RES` handles holds
aliases of those open handles (see `mfb man variable`).

## Copying

Lists are copyable only when their element type is copyable.  A copied list is independent of the original: changing one binding never
changes the other.

## Mutation

Collection helper functions such as `append`, `prepend`, `insert`, `set`,
`removeAt`, `filter`, and `transform` return the resulting list value. For a `MUT` list binding updated with the `name = collections::set(name, …)`
idiom, the change can be made in place, so appending in a loop stays fast
(amortized O(1) per append) — while a `LET` list binding remains an immutable
snapshot and helper calls produce a new value. Passing or returning a `MUT` list
across a function boundary hands over a copy, so a caller and a callee never
change the same list.

## Iteration

`FOR EACH` iterates list items from index 0 to `len(value) - 1`:

```
FOR EACH item IN nums
  io::print(toString(item))
NEXT
```

## Errors

No errors.

## See also

- `mfb man types map`
- `mfb man collections`
