# sleb128Encode

Encode a signed `Integer` as a signed LEB128 `List OF Byte`.

## Synopsis

```
encoding::sleb128Encode(value AS Integer) AS List OF Byte
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

`encoding::sleb128Encode` returns the signed [LEB128](https://en.wikipedia.org/wiki/LEB128)
representation of `value`, a base-128 little-endian variable-length encoding
that carries the sign. The value is split into 7-bit groups, least-significant
group first. Each output byte holds one group in its low seven bits; the high
bit (`0x80`) is set on every byte except the last, where it is clear, marking
the end of the sequence. [[src/builtins/encoding_package.mfb:__encoding_sleb128Encode]]

Unlike unsigned LEB128, encoding continues by arithmetic (sign-extending) shift
rather than logical shift: after each group `value` is shifted right by seven
bits with the sign preserved. The sequence terminates only when the remaining
bits are all sign bits *and* the sign bit of the final group (`0x40`) matches —
that is, when the remaining value is `0` and the group's sign bit is clear, or
the remaining value is `-1` and the group's sign bit is set. This guarantees the
top byte sign-extends correctly on decode. [[src/builtins/encoding_package.mfb:__encoding_sleb128Encode]]

At least one byte is always emitted: `0` encodes as the single byte `[0]` and
`-1` encodes as the single byte `[0x7F]`. Both non-negative and negative values
are accepted; use `encoding::uleb128Encode` when the value is known to be
non-negative and the sign byte is unwanted. The inverse operation is
`encoding::sleb128Decode`, which reads one signed LEB128 sequence back into an
`Integer`. [[src/builtins/encoding_package.mfb:__encoding_sleb128Decode]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `Integer` | The signed value to encode. Any `Integer`, positive, negative, or zero. [[src/builtins/encoding.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF Byte` | The signed LEB128 bytes, least-significant group first, with the continuation bit set on all but the final byte. Always contains at least one byte. [[src/builtins/encoding.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Encode a value and round-trip it back through `sleb128Decode`:

```
IMPORT encoding
IMPORT io

SUB main()
  LET bytes AS List OF Byte = encoding::sleb128Encode(-123456)
  io::print(toString(encoding::sleb128Decode(bytes)))
END SUB
```

Small values fit in a single byte:

```
IMPORT encoding
IMPORT io

SUB main()
  io::print(toString(len(encoding::sleb128Encode(0))))
  io::print(toString(len(encoding::sleb128Encode(-1))))
  io::print(toString(len(encoding::sleb128Encode(-64))))
END SUB
```

## See also

- `mfb man encoding sleb128Decode`
- `mfb man encoding uleb128Encode`
- `mfb man encoding varintEncode`
- `mfb man encoding`
