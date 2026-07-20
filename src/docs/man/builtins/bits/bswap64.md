# bswap64

Reverse the byte order of all 64 bits of an integer.

## Synopsis

```
bits::bswap64(value AS Integer) AS Integer
```

## Package

`bits`

## Imports

```
IMPORT bits
```

`bits` is a built-in package, so `IMPORT bits` needs no manifest dependency.

## Description

`bswap64` reverses the order of the eight bytes that make up the full 64 bits of
`value`: byte `0` (bits `0`..`7`) and byte `7` (bits `56`..`63`) exchange places,
byte `1` (bits `8`..`15`) and byte `6` (bits `48`..`55`) exchange places, byte `2`
(bits `16`..`23`) and byte `5` (bits `40`..`47`) exchange places, and byte `3`
(bits `24`..`31`) and byte `4` (bits `32`..`39`) exchange places, so a value laid
out as `0x1122334455667788` becomes `0x8877665544332211`. This converts the value
between little-endian and big-endian byte order. Unlike `bswap16` and `bswap32`,
every one of the 64 bits participates in the swap, so no bits are cleared. [[src/target/shared/code/builder_bits.rs:lower_bits_bswap]]

`value` is treated as a raw two's-complement 64-bit `Integer` bit pattern;
`bswap64` does not interpret sign. The operation is total — it is defined for
every `Integer` and never raises; only the variable-shift ops (`sl`/`sr`/`sra`)
can raise a `bits::` error — has no side effects, and lowers to a native
doubleword byte-reversal instruction (`rev Xd, Xn`) inline rather than calling a
runtime helper, producing identical results on the native and Binary
Representation execution paths. [[src/builtins/bits.rs:call_return_type_name]] [[src/builtins/bits.rs:is_bits_shift]] [[src/target/shared/abi.rs:reverse_bytes]] [[src/target/shared/code/builder_bits.rs:lower_bits_bswap]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `Integer` | The value whose eight bytes are reversed. All 64 bits participate in the swap. [[src/builtins/bits.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Integer` | The fully byte-reversed value, with byte `0` and byte `7` exchanged, byte `1` and byte `6` exchanged, byte `2` and byte `5` exchanged, and byte `3` and byte `4` exchanged. No bits are cleared. [[src/builtins/bits.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Swap the eight bytes of a 64-bit value:

```
IMPORT bits
IMPORT io

SUB main()
  LET result AS Integer = bits::bswap64(255)
  io::print(toString(result))
END SUB
```

Byte order flips between little-endian and big-endian:

```
IMPORT bits
IMPORT io

SUB main()
  io::print(toString(bits::bswap64(0x1122334455667788)))
END SUB
```

## See also

- `mfb man bits bswap16`
- `mfb man bits bswap32`
- `mfb man bits bnot`
- `mfb man bits package`
