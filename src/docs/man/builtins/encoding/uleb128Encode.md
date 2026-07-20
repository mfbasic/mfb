# uleb128Encode

Encode a non-negative `Integer` as an unsigned LEB128 `List OF Byte`.

## Synopsis

```
encoding::uleb128Encode(value AS Integer) AS List OF Byte
```

## Package

encoding

## Imports

```
IMPORT encoding
```

`encoding` is a built-in package written in MFBASIC source, so no manifest
dependency is required. [[src/builtins/encoding.rs:augmented_project]]

## Description

`encoding::uleb128Encode` returns the unsigned [LEB128](https://en.wikipedia.org/wiki/LEB128)
representation of `value`, a base-128 little-endian variable-length encoding.
The value is split into 7-bit groups, least-significant group first. Each output
byte carries one group in its low seven bits; the high bit (`0x80`) is set on
every byte except the last, where it is clear, marking the end of the sequence.
[[src/builtins/encoding_package.mfb:__encoding_uleb128Encode]]

At least one byte is always emitted: `0` encodes as the single byte `[0]`.
Because groups are produced until the remaining value reaches zero, the output
length grows by one byte for every additional seven significant bits — for
example values in `0`–`127` produce one byte, `128`–`16383` produce two bytes,
and so on. [[src/builtins/encoding_package.mfb:__encoding_uleb128Encode]]

`value` must be non-negative; unsigned LEB128 has no representation for negative
numbers. Use `encoding::sleb128Encode` for signed values. The inverse operation
is `encoding::uleb128Decode`, which reads one unsigned LEB128 sequence back into
an `Integer`. [[src/builtins/encoding_package.mfb:__encoding_uleb128Decode]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `Integer` | The non-negative value to encode. Must be `>= 0`. [[src/builtins/encoding.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF Byte` | The unsigned LEB128 bytes, least-significant group first, with the continuation bit set on all but the final byte. Always contains at least one byte. [[src/builtins/encoding.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050003` | `ErrInvalidFormat` | `value` is negative (`value < 0`); unsigned LEB128 has no representation for negative numbers. [[src/builtins/encoding_package.mfb:__encoding_uleb128Encode]] [[src/target/shared/code/error_constants.rs:ERR_INVALID_FORMAT_CODE]] |

## Examples

Encode a value and round-trip it back through `uleb128Decode`:

```
IMPORT encoding
IMPORT io

SUB main()
  LET bytes AS List OF Byte = encoding::uleb128Encode(624485)
  io::print(toString(encoding::uleb128Decode(bytes)))
END SUB
```

Small values fit in a single byte:

```
IMPORT encoding
IMPORT io

SUB main()
  io::print(toString(len(encoding::uleb128Encode(0))))
  io::print(toString(len(encoding::uleb128Encode(127))))
  io::print(toString(len(encoding::uleb128Encode(128))))
END SUB
```

## See also

- `mfb man encoding uleb128Decode`
- `mfb man encoding sleb128Encode`
- `mfb man encoding varintEncode`
- `mfb man encoding`
