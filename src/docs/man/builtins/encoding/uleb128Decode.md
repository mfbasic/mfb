# uleb128Decode

Decode an unsigned LEB128 `List OF Byte` back into an `Integer`.

## Synopsis

```
encoding::uleb128Decode(data AS List OF Byte) AS Integer
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

`encoding::uleb128Decode` reads one unsigned [LEB128](https://en.wikipedia.org/wiki/LEB128)
sequence from `data` and returns the `Integer` it represents. It is the inverse
of `encoding::uleb128Encode`. [[src/builtins/encoding_package.mfb:__encoding_uleb128Decode]]

Bytes are consumed least-significant group first. The low seven bits of each
byte contribute the next 7-bit group; the high bit (`0x80`) is the continuation
flag. Decoding accumulates groups — shifting each successive group left by seven
more bits — and stops at the first byte whose high bit is clear (byte value
below `128`), which terminates the sequence. Any bytes after that terminator are
ignored. [[src/builtins/encoding_package.mfb:__encoding_uleb128Decode]]

`data` must contain at least one byte, and the sequence must be terminated
within it: if the bytes run out before a byte with a clear high bit is seen, the
input is treated as truncated. The accumulated shift may not exceed 63 bits;
a sequence encoding more than 64 significant bits overflows. `data` carries only
magnitude, so the result is always non-negative — use `encoding::sleb128Decode`
for signed values. [[src/builtins/encoding_package.mfb:__encoding_uleb128Decode]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `data` | `List OF Byte` | The unsigned LEB128 bytes to decode, least-significant group first, terminated by a byte whose high bit is clear. Must be non-empty. [[src/builtins/encoding.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Integer` | The decoded non-negative value. [[src/builtins/encoding.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050003` | `ErrInvalidFormat` | `data` is empty, the sequence is truncated (the bytes end before a terminator with a clear high bit), or the value overflows 64 bits (the shift exceeds 63). [[src/builtins/encoding_package.mfb:__encoding_uleb128Decode]] [[src/target/shared/code/error_constants.rs:ERR_INVALID_FORMAT_CODE]] |

## Examples

Round-trip a value through `uleb128Encode` and back:

```
IMPORT encoding
IMPORT io

SUB main()
  LET bytes AS List OF Byte = encoding::uleb128Encode(624485)
  io::print(toString(encoding::uleb128Decode(bytes)))
END SUB
```

Decode a literal two-byte sequence (`300` = `[0xAC, 0x02]`):

```
IMPORT encoding
IMPORT collections
IMPORT io

SUB main()
  MUT bytes AS List OF Byte = []
  bytes = collections::append(bytes, toByte(172))
  bytes = collections::append(bytes, toByte(2))
  io::print(toString(encoding::uleb128Decode(bytes)))
END SUB
```

## See also

- `mfb man encoding uleb128Encode`
- `mfb man encoding sleb128Decode`
- `mfb man encoding varintDecode`
- `mfb man encoding`
