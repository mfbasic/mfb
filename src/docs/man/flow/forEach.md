# forEach

Collection iteration

## Synopsis

```
FOR EACH item IN values
  ...
NEXT
```

## Description

`FOR EACH` iterates a `List OF T` or `Map OF K TO V` source; any other source
type is a compile error (`TYPE_FOR_EACH_REQUIRES_COLLECTION`). A list loop binds
the loop variable as `T`, visited in index order. A map loop binds the loop
variable as `MapEntry OF K TO V`, whose `entry.key` has type `K` and
`entry.value` has type `V`; any other field access is `TYPE_UNKNOWN_FIELD`. Map
loop order is implementation-defined but stable for a given unchanged map value,
matching the order used by `keys` and `values`.

`EXIT FOR` leaves the loop and `CONTINUE FOR` skips to the next item.

## Errors

No errors.

## Examples

Iterate a list:

```
FOR EACH line IN lines : io::print(line) : NEXT
```

Iterate a map by entry:

```
FOR EACH entry IN scores : io::print(entry.key & "=" & toString(entry.value)) : NEXT
```

## See also

- `mfb man flow for`
- `mfb man types map`
