# sleb128Decode

Decode a signed LEB128 `List OF Byte` back into an `Integer`.

## Synopsis

```
encoding::sleb128Decode(data AS List OF Byte) AS Integer
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

`encoding::sleb128Decode` reads one signed [LEB128](https://en.wikipedia.org/wiki/LEB128)
sequence from `data` and returns the `Integer` it represents. It is the inverse
of `encoding::sleb128Encode`. [[src/builtins/encoding_package.mfb:__encoding_sleb128Decode]]

Bytes are consumed least-significant group first. The low seven bits of each
byte contribute the next 7-bit group; the high bit (`0x80`) is the continuation
flag. Decoding accumulates groups — shifting each successive group left by seven
more bits — and stops at the first byte whose high bit is clear (byte value
below `128`), which terminates the sequence. Any bytes after that terminator are
ignored. [[src/builtins/encoding_package.mfb:__encoding_sleb128Decode]]

Unlike `encoding::uleb128Decode`, the terminating group carries the sign. When
the final byte's sign bit (`0x40`) is set and the accumulated shift is still
below `64`, the result is sign-extended by filling every higher bit with ones, so
the value decodes as negative. A clear `0x40` leaves the value non-negative. This
mirrors the arithmetic (sign-extending) shift used by `encoding::sleb128Encode`.
[[src/builtins/encoding_package.mfb:__encoding_sleb128Decode]]

`data` must contain at least one byte, and the sequence must be terminated
within it: if the bytes run out before a byte with a clear high bit is seen, the
input is treated as truncated. The accumulated shift may not exceed `63` bits;
a sequence encoding more than 64 significant bits overflows.
[[src/builtins/encoding_package.mfb:__encoding_sleb128Decode]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `data` | `List OF Byte` | The signed LEB128 bytes to decode, least-significant group first, terminated by a byte whose high bit is clear. The terminating group's `0x40` bit carries the sign. Must be non-empty. [[src/builtins/encoding.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Integer` | The decoded signed value, sign-extended from the terminating group. [[src/builtins/encoding.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050003` | `ErrInvalidFormat` | `data` is empty, the sequence is truncated (the bytes end before a terminator with a clear high bit), or the value overflows 64 bits (the shift exceeds `63`). [[src/builtins/encoding_package.mfb:__encoding_sleb128Decode]] [[src/target/shared/code/error_constants.rs:ERR_INVALID_FORMAT_CODE]] |

## Examples

Round-trip a signed value through `sleb128Encode` and back:

```
IMPORT encoding

LET bytes AS List OF Byte = encoding::sleb128Encode(-123456)
io::print(encoding::sleb128Decode(bytes))
```

Decode a single terminating byte whose `0x40` sign bit is set (`-2` = `[0x7E]`):

```
IMPORT encoding
IMPORT collections

MUT bytes AS List OF Byte = []
bytes = collections::append(bytes, toByte(126))
io::print(encoding::sleb128Decode(bytes))
```

## See also

- `mfb man encoding sleb128Encode`
- `mfb man encoding uleb128Decode`
- `mfb man encoding varintDecode`
- `mfb man encoding`
