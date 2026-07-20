# varintEncode

Encode a signed `Integer` as a ZigZag varint `List OF Byte`.

## Synopsis

```
encoding::varintEncode(value AS Integer) AS List OF Byte
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

`encoding::varintEncode` returns the ZigZag [varint](https://protobuf.dev/programming-guides/encoding/#varints)
representation of `value`. It first maps the signed value onto an unsigned one
with ZigZag encoding — `(value << 1) XOR (value >> 63)`, an arithmetic
right shift — so that small-magnitude negatives become small unsigned numbers,
then writes that unsigned result as base-128 [LEB128](https://en.wikipedia.org/wiki/LEB128).
[[src/builtins/encoding_package.mfb:__encoding_varintEncode]]

The ZigZag mapping interleaves signs: `0` maps to `0`, `-1` to `1`, `1` to `2`,
`-2` to `3`, and so on. The mapped value is then split into 7-bit groups,
least-significant group first. Each output byte carries one group in its low
seven bits; the high bit (`0x80`) is set on every byte except the last, where it
is clear, marking the end of the sequence. Because the intermediate value is
shifted right logically, encoding always terminates and at least one byte is
always emitted: `0` encodes as the single byte `[0]`.
[[src/builtins/encoding_package.mfb:__encoding_varintEncode]]

Unlike `encoding::uleb128Encode`, `value` may be negative — ZigZag gives every
signed value a compact unsigned form, so no value is rejected. The inverse
operation is `encoding::varintDecode`, which reads one ZigZag varint sequence
back into a signed `Integer`. [[src/builtins/encoding_package.mfb:__encoding_varintDecode]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `Integer` | The signed 64-bit value to encode. Any `Integer`, negative or non-negative, is accepted. [[src/builtins/encoding.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF Byte` | The ZigZag varint bytes, least-significant group first, with the continuation bit set on all but the final byte. Always contains at least one byte. [[src/builtins/encoding.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Encode a signed value and round-trip it back through `varintDecode`:

```
IMPORT encoding
IMPORT io

SUB main()
  LET bytes AS List OF Byte = encoding::varintEncode(-75)
  io::print(toString(encoding::varintDecode(bytes)))
END SUB
```

Small-magnitude values, positive or negative, fit in a single byte:

```
IMPORT encoding
IMPORT io

SUB main()
  io::print(toString(len(encoding::varintEncode(0))))
  io::print(toString(len(encoding::varintEncode(-1))))
  io::print(toString(len(encoding::varintEncode(63))))
END SUB
```

## See also

- `mfb man encoding varintDecode`
- `mfb man encoding uleb128Encode`
- `mfb man encoding sleb128Encode`
- `mfb man encoding`
