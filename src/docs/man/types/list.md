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

Brackets after a type name are a record constructor (`TypeName[...]`), never
indexing; there is no indexing-bracket syntax. All list access is through free
functions such as `collections::get`.

## What a list holds

A list holds its items directly, as part of the list value: numbers, strings,
records, unions and nested collections alike. A list of `RES` handles holds
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
