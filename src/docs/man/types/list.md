# list

Owned ordered List values

## Synopsis

```
List OF T
```

## Description

`List OF T` is an owned ordered sequence. Every item has the same element type
`T`, and indexes are zero-based. A list value owns its contents: binding a list
with `LET` creates an immutable snapshot, while binding a list with `MUT` creates
a locally mutable binding whose value is still owned by that binding.

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

## Owned items and storage

A list stores its items in one contiguous allocation — a header, a lookup table
that holds list order, and a packed data region — so a list value is a single
owned block. Primitive items are stored as payload bytes; `String` items are
stored as their UTF-8 bytes; and records, data-only unions, and *flat* nested
collections (a `List`/`Map` whose own payloads are flat) are inlined into the
data region as their full block. The only payloads stored as an 8-byte pointer
handle rather than inline are a resource and a non-flat nested collection (one
whose own payloads include a resource). [[src/target/shared/code/builder_collection_layout.rs:is_pointer_collection_payload_type]]

## Copying

Lists are copyable only when their element type is copyable. Copying a list is
shrink-to-fit: the copy is re-tightened so its capacity equals its length, so
over that tight prefix it is a single contiguous memory copy and no mutable
working headroom leaks into the snapshot. A copied list is independent of the
original: mutating one binding never mutates another copied snapshot.

## Mutation

Collection helper functions such as `append`, `prepend`, `insert`, `set`,
`removeAt`, `filter`, and `transform` return the resulting list value. For a
uniquely-owned `MUT` list binding written with the `name = collections::set(name, …)`
idiom, the compiler may update the live buffer in place — an append into spare
headroom is amortized O(1) — while a `LET` list binding remains an immutable
snapshot and helper calls produce a new value. Passing or returning a `MUT` list
across a function boundary freezes it into an immutable owned value, so no caller
and callee ever share a mutable buffer.

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
