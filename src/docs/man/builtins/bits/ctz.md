# ctz

Count the trailing zero bits of a 64-bit integer.

## Synopsis

```
bits::ctz(value AS Integer) AS Integer
```

## Package

`bits`

## Imports

```
IMPORT bits
```

`bits` is a built-in package, so `IMPORT bits` needs no manifest dependency.

## Description

`ctz` returns the number of zero bits that follow the least significant set (`1`)
bit of `value`, counting up from bit 0 (the lowest bit) toward bit 63 (the
highest bit).

`value` is treated as a raw two's-complement 64-bit `Integer` bit pattern; `ctz`
does not interpret sign. When `value` is `0` there is no set bit, so all 64 bits
count as trailing zeros and the result is `64`. When bit 0 is set (the value is
odd) the result is `0`. The operation is total — it is defined for every
`Integer` and never raises — has no side effects, and lowers inline by reversing
the bits (`RBIT`) and then counting leading zeros rather than calling a runtime
helper, producing identical results on the native and Binary Representation
execution paths. [[src/builtins/bits.rs:call_return_type_name]] [[src/target/shared/code/builder_bits.rs:lower_bits_count_zeros]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `Integer` | The 64-bit value to inspect. Any `Integer` is accepted; treated as a raw bit pattern. [[src/builtins/bits.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Integer` | The trailing-zero count, in the range `0`..`64`. `0` when bit 0 is set (the value is odd); `64` when `value` is `0`. [[src/builtins/bits.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Count the trailing zeros of a small value:

```
IMPORT bits

LET result AS Integer = bits::ctz(255)
PRINT result
```

The all-zero pattern has 64 trailing zeros:

```
IMPORT bits

PRINT bits::ctz(0)
```

## See also

- `mfb man bits clz`
- `mfb man bits popCount`
- `mfb man bits bnot`
- `mfb man bits package`
