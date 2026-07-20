# len

Return the number of elements in a string, list, or map.

## Synopsis

```
len(value AS String) AS Integer
len(value AS List OF T) AS Integer
len(value AS Map OF K TO V) AS Integer
```

## Package

general

## Imports

None. `general` functions are always available without an `IMPORT` statement. [[src/builtins/general.rs:is_general_call]]

## Description

`len` returns the size of `value` as an `Integer`. The meaning of "size" depends
on the kind of value supplied, and the argument must be a `String`, a `List OF T`,
or a `Map OF K TO V`; any other argument type is a compile-time error. [[src/builtins/general.rs:resolve_call]]

For a `String`, `len` returns the number of Unicode scalar values it contains,
counted by scanning the UTF-8 bytes and counting every leading (non-continuation)
byte. The count is measured in Unicode scalar values, not UTF-8 bytes and not
user-perceived grapheme clusters. A multi-byte character such as `"é"` or `"😀"`
counts as one scalar value even though it occupies several bytes, while a grapheme
made of a base character plus combining marks counts as more than one scalar
value. Use `strings::byteLen` for the byte length and `strings::graphemes` when
grapheme clusters are needed. [[src/target/shared/code/builder_collection_layout.rs:lower_len]]

For a `List OF T`, `len` returns the number of elements, regardless of the element
type `T`. For a `Map OF K TO V`, `len` returns the number of key/value entries.
Both are read directly from the collection's stored element count. [[src/target/shared/code/builder_collection_layout.rs:lower_len]]

An empty `String`, empty `List`, or empty `Map` returns `0`. `len` reads `value`
only and has no side effects; it never mutates its argument and never fails at run
time. [[src/target/shared/code/builder_values.rs:lower_infallible_member]]

## Overloads

**`len(value AS String) AS Integer`**

Returns the number of Unicode scalar values in `value`.

**`len(value AS List OF T) AS Integer`**

Returns the number of elements in `value` for any element type `T`.

**`len(value AS Map OF K TO V) AS Integer`**

Returns the number of key/value entries in `value` for any key type `K` and value
type `V`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The string whose Unicode scalar values are counted. |
| `value` | `List OF T` | The list whose elements are counted, for any element type `T`. |
| `value` | `Map OF K TO V` | The map whose entries are counted, for any key type `K` and value type `V`. |

## Return value

| Type | Description |
| --- | --- |
| `Integer` | The number of scalar values, list elements, or map entries in `value`. Returns `0` for an empty string, list, or map. |

## Errors

No errors.

## Type checking

`len` is generic over its single argument and accepts a `String`, a `List OF T`,
or a `Map OF K TO V`. It takes exactly one argument; any other arity or an
argument of any other type is rejected at compile time. The element type `T` and
the map key and value types `K` and `V` are unconstrained. [[src/builtins/general.rs:resolve_call]]

## Examples

Count string scalar values:

```
SUB main()
  LET count AS Integer = len("hello")
END SUB
```

Multi-byte characters count as single scalar values:

```
SUB main()
  LET emoji AS Integer = len("😀")
END SUB
```

Count the elements of a list:

```
SUB main()
  LET items AS List OF Integer = [10, 20, 30]
  LET total AS Integer = len(items)
END SUB
```

Count the entries of a map:

```
SUB main()
  LET ages AS Map OF String TO Integer = Map OF String TO Integer { "ada" := 36, "bo" := 9 }
  LET pairs AS Integer = len(ages)
END SUB
```

## See also

- `mfb man filters isEmpty`
- `mfb man filters isNotEmpty`
- `mfb man strings byteLen`
- `mfb man strings graphemes`
