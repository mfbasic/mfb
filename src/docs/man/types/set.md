# set

Owned unordered Set values

## Synopsis

```
Set OF T
```

## Description

`Set OF T` is an owned, unordered, deduplicated collection of elements of a
single type `T`. Each distinct element appears at most once: adding an element
that is already present is a no-op, so a set never holds two equal elements. The
element type `T` must be comparable, exactly as a `Map` key must be. A set value
owns its elements: binding a set with `LET` creates an immutable snapshot, while
binding a set with `MUT` creates a locally mutable binding whose value is still
owned by that binding. A `Set` is itself **not** comparable, so it cannot be a
`Map` key, a `Set` element, or an operand of `=`.

## Literals

Set literals name the element type and list the elements between braces.
Duplicates collapse to a single element, and an empty set needs its element type
from the literal or an annotation:

```
LET primes = Set OF Integer { 2, 3, 5, 7 }
LET empty AS Set OF Integer = Set OF Integer { }
LET collapsed = Set OF Integer { 1, 1, 2 }   ' holds 1 and 2 — len is 2
```

## Elements

A set element must be comparable: `Integer`, `Float`, `Fixed`, `Boolean`,
`String`, `Byte`, `Nothing`, enum types, or records whose fields are all
comparable. `List`, `Map`, `Set`, unions, functions, lambdas, threads, and
resource handles are not comparable and cannot be set elements. Element equality
is a bitwise comparison, so `Float` elements distinguish `+0.0` from `-0.0` and
treat `NaN` as equal to `NaN` — distinct from the IEEE rule used by the `=`
operator on `Float` values.

## Owned items and storage

A set is stored as a `Map`-shaped block — a header, an insertion-ordered lookup
table, a packed data region, and a derived hash index — but it is a hash-indexed
set of elements with no values: each element is stored as an entry key, and the
per-entry value is a single implementation-detail tag byte.
[[src/target/shared/code/error_constants.rs:COLLECTION_KIND_SET]]
[[src/target/shared/code/builder_collection_layout.rs:lower_set_literal]]
Membership is an O(1)-average FNV-1a hash probe for the probe-eligible element
types — `Integer`, `Float`, `Fixed`, `Byte`, `Boolean`, and `String` — and a
linear scan over the live entries for any other element type. The hash index is
rebuilt lazily on first use, and the bucket region is shared with the `Map`
layout. [[src/target/shared/code/type_utils.rs:collection_has_buckets]]
[[src/codegen/builtins/collections/common/set.rs:emit_key_membership]]

## Copying

A set is value-semantic and copyable when its element type is copyable. Copying a
set is shrink-to-fit — the copy is re-tightened to its live size, so over that
prefix it is a single contiguous memory copy. A copied set is independent of its
source: mutating one binding never mutates another copied snapshot.

## Mutation

The `collections` package supplies `collections::add` (idempotent insert),
`collections::remove` (a no-op when the element is absent),
`collections::contains` (membership test), and `collections::toList` (the
elements as a `List OF T` in insertion order). All are value-semantic: `add` and
`remove` return a new set, the argument is never modified, and a program observes
the update only through what it does with the returned value. For a
uniquely-owned `MUT` set binding written with the
`name = collections::add(name, …)` idiom, the compiler may update the live buffer
in place; a `LET` set binding remains an immutable snapshot and helper calls
produce a new value. The `collections` package also supplies the pure set-algebra
generics `union`, `intersection`, `difference`, `symmetricDifference`,
`isSubset`, `isSuperset`, `isDisjoint`, and `toSet`.

## Iteration

`FOR EACH` over a set yields each element `T` once, in insertion order:

```
FOR EACH n IN primes
  io::print(toString(n))
NEXT
```

## Errors

No errors.

## See also

- `mfb man types list`
- `mfb man types map`
- `mfb man collections add`
- `mfb man collections union`
- `mfb man collections`
