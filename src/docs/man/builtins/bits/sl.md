# sl

Logical left shift of a 64-bit integer.

## Synopsis

```
bits::sl(value AS Integer, count AS Integer) AS Integer
```

## Package

`bits`

## Imports

```
IMPORT bits
```

`bits` is a built-in package, so `IMPORT bits` needs no manifest dependency.

## Description

`sl` shifts `value` left by `count` bit positions. Vacated low bits are filled
with zero, and bits shifted past bit 63 are discarded, so the result keeps only
the low 64 bits of the shifted value. A `count` of `0` returns `value`
unchanged.

Both `value` and the result are raw two's-complement 64-bit `Integer` bit
patterns; `sl` does not interpret sign, and it makes no distinction between a
logical and an arithmetic left shift. For the sign-preserving right shift see
`bits::sra`; for the zero-filling right shift see `bits::sr`.

Unlike the total bitwise operations, `sl` validates `count`: it first checks
that `count` is in the range `0` to `63` inclusive and raises
`ErrInvalidArgument` for any value outside it, before performing the shift. The
operation has no side effects and lowers to a native variable-shift instruction
inline rather than calling a runtime helper, producing identical results on the
native and Binary Representation execution paths. [[src/target/shared/code/builder_bits.rs:lower_bits_shift]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `Integer` | The value to shift. Any 64-bit value; treated as a raw bit pattern. [[src/builtins/bits.rs:call_param_names]] |
| `count` | `Integer` | The shift amount in bits. Must be in the range `0` to `63` inclusive; any other value raises `ErrInvalidArgument`. [[src/target/shared/code/builder_bits.rs:lower_bits_shift]] |

## Return value

| Type | Description |
| --- | --- |
| `Integer` | `value` shifted left by `count` bits, with vacated low bits zero and bits above bit 63 discarded. Equal to `value` when `count` is `0`. [[src/builtins/bits.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `count` is less than `0` or greater than `63`. [[src/target/shared/code/builder_bits.rs:lower_bits_shift]] [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |

## Examples

Shift a value left by four bits (multiply by 16):

```
IMPORT bits

LET result AS Integer = bits::sl(1, 4)
PRINT result
```

Build a byte-packed field by shifting a value into place and combining it with
`bits::bor`:

```
IMPORT bits

LET high AS Integer = bits::sl(0xAB, 8)
LET packed AS Integer = bits::bor(high, 0xCD)
PRINT packed
```

## See also

- `mfb man bits sr`
- `mfb man bits sra`
- `mfb man bits bor`
- `mfb man bits package`
