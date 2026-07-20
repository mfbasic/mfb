# bswap32

Reverse the byte order of the low 32 bits of an integer.

## Synopsis

```
bits::bswap32(value AS Integer) AS Integer
```

## Package

`bits`

## Imports

```
IMPORT bits
```

`bits` is a built-in package, so `IMPORT bits` needs no manifest dependency.

## Description

`bswap32` reverses the order of the four bytes that make up the low 32 bits of
`value`: byte `0` (bits `0`..`7`) and byte `3` (bits `24`..`31`) exchange places,
and byte `1` (bits `8`..`15`) and byte `2` (bits `16`..`23`) exchange places, so a
value laid out as `0xAABBCCDD` becomes `0xDDCCBBAA`. Every bit above bit `31`
(bits `32`..`63`) is cleared to zero in the result, so the output is always a
non-negative 32-bit quantity regardless of the high bits of `value`. [[src/target/shared/code/builder_bits.rs:lower_bits_bswap]]

`value` is treated as a raw two's-complement 64-bit `Integer` bit pattern;
`bswap32` does not interpret sign. The operation is total — it is defined for
every `Integer` and never raises; only the variable-shift ops (`sl`/`sr`/`sra`)
can raise a `bits::` error — has no side effects, and lowers to a native word
byte-reversal instruction (`rev Wd, Wn`, which zero-extends into the upper half)
inline rather than calling a runtime helper, producing identical results on the
native and Binary Representation execution paths. [[src/builtins/bits.rs:is_bits_shift]] [[src/target/shared/abi.rs:reverse_bytes_word]] [[src/target/shared/code/builder_bits.rs:lower_bits_bswap]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `Integer` | The value whose low 32 bits are byte-reversed. Bits above bit `31` are ignored and do not appear in the result. [[src/builtins/bits.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Integer` | The byte-reversed low 32 bits of `value`, with bits `32`..`63` cleared to zero. Always in the range `0`..`4294967295`. [[src/builtins/bits.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Swap the four low bytes of a 32-bit value:

```
IMPORT bits
IMPORT io

SUB main()
  LET result AS Integer = bits::bswap32(0x000000FF)
  io::print(toString(result))
END SUB
```

Bits above bit 31 are cleared, so the result stays in `0`..`4294967295`:

```
IMPORT bits
IMPORT io

SUB main()
  io::print(toString(bits::bswap32(0x1122334455667788)))
END SUB
```

## See also

- `mfb man bits bswap16`
- `mfb man bits bswap64`
- `mfb man bits bnot`
- `mfb man bits package`
