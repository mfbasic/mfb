# sr

Logical (zero-filling) right shift of a 64-bit integer.

## Synopsis

```
bits::sr(value AS Integer, count AS Integer) AS Integer
```

## Package

`bits`

## Imports

```
IMPORT bits
```

`bits` is a built-in package, so `IMPORT bits` needs no manifest dependency.

## Description

`sr` shifts `value` right by `count` bit positions as an unsigned quantity.
Vacated high bits are filled with zero, and bits shifted past bit 0 are
discarded. A `count` of `0` returns `value` unchanged.

Both `value` and the result are raw two's-complement 64-bit `Integer` bit
patterns; `sr` does not interpret sign. Because the vacated high bits are always
zeroed, the sign bit is *not* replicated — this is the distinction from the
arithmetic right shift `bits::sra`, which preserves the sign bit. For the
left shift see `bits::sl`.

Unlike the total bitwise operations, `sr` validates `count`: it first checks
that `count` is in the range `0` to `63` inclusive and raises
`ErrInvalidArgument` for any value outside it, before performing the shift.
Larger shift amounts are not
implicitly clamped or reduced modulo the width. The operation has no side
effects and lowers to a native variable-shift instruction inline rather than
calling a runtime helper, producing identical results on the native and Binary
Representation execution paths. [[src/target/shared/code/builder_bits.rs:lower_bits_shift]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `Integer` | The value to shift. Any 64-bit value; treated as a raw bit pattern. [[src/builtins/bits.rs:call_param_names]] |
| `count` | `Integer` | The shift amount in bits. Must be in the range `0` to `63` inclusive; any other value raises `ErrInvalidArgument`. [[src/target/shared/code/builder_bits.rs:lower_bits_shift]] |

## Return value

| Type | Description |
| --- | --- |
| `Integer` | `value` shifted right by `count` bits, with vacated high bits zero and bits below bit 0 discarded. Equal to `value` when `count` is `0`. [[src/builtins/bits.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `count` is less than `0` or greater than `63`. [[src/target/shared/code/builder_bits.rs:lower_bits_shift]] [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |

## Examples

Shift a value right by four bits (unsigned divide by 16):

```
IMPORT bits

LET result AS Integer = bits::sr(256, 4)
PRINT result
```

Extract a byte-packed field by shifting it down into place and masking with
`bits::band`:

```
IMPORT bits

LET packed AS Integer = 0xABCD
LET high AS Integer = bits::band(bits::sr(packed, 8), 255)
PRINT high
```

## See also

- `mfb man bits sl`
- `mfb man bits sra`
- `mfb man bits band`
- `mfb man bits package`
