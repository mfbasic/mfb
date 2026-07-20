# varintDecode

Decode a ZigZag varint `List OF Byte` back into a signed `Integer`.

## Synopsis

```
encoding::varintDecode(data AS List OF Byte) AS Integer
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

`encoding::varintDecode` reads one ZigZag [varint](https://protobuf.dev/programming-guides/encoding/#varints)
sequence from `data` and returns the signed `Integer` it represents. It is the
inverse of `encoding::varintEncode`. [[src/builtins/encoding_package.mfb:__encoding_varintDecode]]

Decoding proceeds in two steps. First the bytes are read as an unsigned
[LEB128](https://en.wikipedia.org/wiki/LEB128) sequence — least-significant 7-bit
group first, with the high bit (`0x80`) of each byte marking continuation and the
first byte with a clear high bit terminating the sequence. Then the ZigZag
mapping is reversed — `(u >> 1) XOR -(u AND 1)` — turning the unsigned value back
into the original signed value, so that small-magnitude negatives round-trip
correctly. Because the ZigZag reversal is pure arithmetic on the decoded value,
it never fails on its own; every error surfaces from the underlying LEB128 read.
[[src/builtins/encoding_package.mfb:__encoding_varintDecode]]

`data` must contain at least one byte, and the sequence must be terminated within
it: if the bytes run out before a byte with a clear high bit is seen, the input
is treated as truncated. The accumulated shift may not exceed 63 bits; a sequence
encoding more than 64 significant bits overflows. Any bytes after the terminator
are ignored. [[src/builtins/encoding_package.mfb:__encoding_uleb128Decode]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `data` | `List OF Byte` | The ZigZag varint bytes to decode, least-significant group first, terminated by a byte whose high bit is clear. Must be non-empty. [[src/builtins/encoding.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Integer` | The decoded signed value, negative or non-negative, reproduced from the ZigZag mapping. [[src/builtins/encoding.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050003` | `ErrInvalidFormat` | `data` is empty, the sequence is truncated (the bytes end before a terminator with a clear high bit), or the value overflows 64 bits (the shift exceeds 63). [[src/builtins/encoding_package.mfb:__encoding_uleb128Decode]] [[src/target/shared/code/error_constants.rs:ERR_INVALID_FORMAT_CODE]] |

## Examples

Round-trip a signed value through `varintEncode` and back:

```
IMPORT encoding
IMPORT io

SUB main()
  LET bytes AS List OF Byte = encoding::varintEncode(-75)
  io::print(toString(encoding::varintDecode(bytes)))
END SUB
```

Decode a literal two-byte sequence (`-75` = `[0x95, 0x01]`):

```
IMPORT encoding
IMPORT collections
IMPORT io

SUB main()
  MUT bytes AS List OF Byte = []
  bytes = collections::append(bytes, toByte(149))
  bytes = collections::append(bytes, toByte(1))
  io::print(toString(encoding::varintDecode(bytes)))
END SUB
```

## See also

- `mfb man encoding varintEncode`
- `mfb man encoding uleb128Decode`
- `mfb man encoding sleb128Decode`
- `mfb man encoding`
