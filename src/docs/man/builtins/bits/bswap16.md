# bswap16

Reverse the byte order of the low 16 bits of an integer.

## Synopsis

```
bits::bswap16(value AS Integer) AS Integer
```

## Package

`bits`

## Imports

```
IMPORT bits
```

`bits` is a built-in package, so `IMPORT bits` needs no manifest dependency.

## Description

`bswap16` swaps the two bytes that make up the low 16 bits of `value`: byte `0`
(bits `0`..`7`) and byte `1` (bits `8`..`15`) exchange places, so a value laid
out as `0xHHLL` becomes `0xLLHH`. Every bit above bit `15` (bits `16`..`63`) is
cleared to zero in the result, so the output is always a non-negative 16-bit
quantity regardless of the high bits of `value`. [[src/target/shared/code/builder_bits.rs:lower_bits_bswap]]

`value` is treated as a raw two's-complement 64-bit `Integer` bit pattern;
`bswap16` does not interpret sign. The operation is total — it is defined for
every `Integer` and never raises — has no side effects, and lowers to native
byte-reversal instructions inline rather than calling a runtime helper. [[src/builtins/bits.rs:is_bits_shift]] [[src/target/shared/code/builder_bits.rs:lower_bits_bswap]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `Integer` | The value whose low 16 bits are byte-reversed. Bits above bit `15` are ignored and do not appear in the result. [[src/builtins/bits.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Integer` | The byte-reversed low 16 bits of `value`, with bits `16`..`63` cleared to zero. Always in the range `0`..`65535`. [[src/builtins/bits.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Swap the two low bytes of a 16-bit value:

```
IMPORT bits
IMPORT io

SUB main()
  LET result AS Integer = bits::bswap16(0x00FF)
  io::print(toString(result))
END SUB
```

Bits above bit 15 are cleared, so the result stays in `0`..`65535`:

```
IMPORT bits
IMPORT io

SUB main()
  io::print(toString(bits::bswap16(0x11223344)))
END SUB
```

## See also

- `mfb man bits bswap32`
- `mfb man bits bswap64`
- `mfb man bits bnot`
- `mfb man bits package`
