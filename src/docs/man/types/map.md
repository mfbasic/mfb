# map

Owned key/value Map values

## Synopsis

```
Map OF K TO V
```

## Description

`Map OF K TO V` is an owned key/value collection. Keys have type `K` and values
have type `V`, and `K` must be comparable. A map value owns its keys and values:
binding a map with `LET` creates an immutable snapshot, while binding a map with
`MUT` creates a locally mutable binding whose value is still owned by that
binding.

## Literals

Map literals name the key and value types and pair each key with `:=`:

```
LET ages = Map OF String TO Integer { "Ada" := 36, "Grace" := 85 }
LET empty AS Map OF String TO Integer = Map OF String TO Integer { }
```

## Keys

Map keys must be comparable: `Integer`, `Float`, `Fixed`, `Boolean`, `String`,
`Byte`, `Nothing`, enum types, or records whose fields are all comparable.
`List`, `Map`, unions, functions, lambdas, threads, and resource handles are not
comparable and cannot be used as keys. Key equality is a bitwise comparison, so
`Float` keys distinguish `+0.0` from `-0.0` and treat `NaN` as equal to `NaN` —
distinct from the IEEE rule used by the `=` operator on `Float` values.

## Owned items and storage

A map stores its keys and values in one contiguous allocation — a header, an
insertion-ordered lookup table, a packed data region, and a derived hash index.
Primitive keys and values are stored as payload bytes; `String` payloads are
stored as their UTF-8 bytes; and records, data-only unions, and *flat* nested
collections are inlined into the data region as their full block. The only
payloads stored as an 8-byte pointer handle are a resource and a non-flat nested
collection. Key lookup uses an O(1)-average FNV-1a hash index that is rebuilt
lazily on first use. [[src/target/shared/code/builder_collection_layout.rs:is_pointer_collection_payload_type]]

## Copying

Maps are copyable only when both the key and value types are copyable. Copying a
map is shrink-to-fit — the copy is re-tightened to its live size, so over that
prefix it is a single contiguous memory copy. A copied map is independent of the
original: mutating one binding never mutates another copied snapshot.

## Mutation

Collection helper functions such as `set`, `removeKey`, `keys`, `values`, `get`,
`getOr`, `hasKey`, and `contains` return or inspect map values. For a
uniquely-owned `MUT` map binding the compiler may update the live buffer in place
— inserting a new key into spare headroom, or overwriting a same-size value —
while a `LET` map binding remains an immutable snapshot and helper calls produce
a new value.

## Iteration order

Map iteration order is implementation-defined but stable for a given unchanged
map value during one program run: repeated `keys`, `values`, and `FOR EACH`
traversal of the same unchanged map use the same insertion order. Creating a
changed map value may choose a different order. `FOR EACH` over a map yields
`MapEntry OF K TO V` values:

```
FOR EACH entry IN ages
  io::print(entry.key & ": " & toString(entry.value))
NEXT
```

## Errors

No errors.

## See also

- `mfb man types list`
- `mfb man collections`
