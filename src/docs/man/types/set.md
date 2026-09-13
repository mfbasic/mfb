# set

Unordered Set values

## Synopsis

```
Set OF T
```

## Description

`Set OF T` is an unordered, deduplicated collection of elements of a
single type `T`. Each distinct element appears at most once: adding an element
that is already present is a no-op, so a set never holds two equal elements. The
element type `T` must be comparable, exactly as a `Map` key must be. A set value holds its elements: binding a set
with `LET` creates an immutable snapshot, while binding a set with `MUT` creates a
binding you can change locally. A `Set` is itself **not** comparable, so it cannot be a
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

A set element must be comparable: `Integer`, `Float`, `Fixed`, `Money`,
`Boolean`, `String`, `Byte`, `Scalar`, `Nothing`, enum types, or records whose
fields are all comparable. `List`, `Map`, `Set`, unions, functions, lambdas,
threads, and resource handles are not comparable and cannot be set elements.
Element equality is a bitwise comparison, so `Float` elements distinguish `+0.0`
from `-0.0` and treat `NaN` as equal to `NaN` — distinct from the IEEE rule used
by the `=` operator on `Float` values.

## Membership

Checking whether a set contains an element takes constant time on average for
`Integer`, `Float`, `Fixed`, `Byte`, `Boolean` and `String` elements, and time
proportional to the set's size for any other element type.

## Copying

A set is value-semantic and copyable when its element type is copyable.  A copied set is independent of its source: changing one binding never changes
the other.

## Mutation

The `collections` package supplies `collections::add` (idempotent insert),
`collections::remove` (a no-op when the element is absent),
`collections::contains` (membership test), and `collections::toList` (the
elements as a `List OF T` in insertion order). All are value-semantic: `add` and
`remove` return a new set, the argument is never modified, and a program observes
the update only through what it does with the returned value. For a `MUT` set binding updated with the
`name = collections::add(name, …)` idiom, the change can be made in place; a `LET` set binding remains an immutable snapshot and helper calls
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
