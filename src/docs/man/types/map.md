# map

Key/value Map values

## Synopsis

```
Map OF K TO V
```

## Description

`Map OF K TO V` is a key/value collection. Keys have type `K` and values have
type `V`, and `K` must be comparable. A map value holds its keys and values:
binding a map with `LET` creates an immutable snapshot, while binding a map with
`MUT` creates a binding you can change locally.

## Literals

Map literals name the key and value types and pair each key with `:=`:

```
LET ages = Map OF String TO Integer { "Ada" := 36, "Grace" := 85 }
LET empty AS Map OF String TO Integer = Map OF String TO Integer { }
```

## Keys

Map keys must be comparable: `Integer`, `Float`, `Fixed`, `Money`, `Boolean`,
`String`, `Byte`, `Scalar`, `Nothing`, enum types, or records whose fields are
all comparable. `List`, `Map`, unions, functions, lambdas, threads, and resource
handles are not comparable and cannot be used as keys. Key equality is a bitwise
comparison, so `Float` keys distinguish `+0.0` from `-0.0` and treat `NaN` as
equal to `NaN` — distinct from the IEEE rule used by the `=` operator on `Float`
values.

## What a map holds

A map holds its keys and values directly, as part of the map value. Looking up a
key takes constant time on average.

## Copying

Maps are copyable only when both the key and value types are copyable.  A copied map is independent of the original: changing one binding never
changes the other.

## Mutation

Collection helper functions such as `set`, `removeKey`, `keys`, `values`, `get`,
`getOr`, `hasKey`, and `contains` return or inspect map values. For a `MUT` map binding, a change can be made in place, so filling a map in a
loop stays fast — while a `LET` map binding remains an immutable snapshot and
helper calls produce a new value.

## Iteration order

Map iteration order is implementation-defined but stable for a given unchanged
map value during one program run: repeated `keys`, `values`, and `FOR EACH`
traversal of the same unchanged map use the same order. Creating a
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
