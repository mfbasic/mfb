# popCount

Count the set (`1`) bits of a 64-bit integer (population count).

## Synopsis

```
bits::popCount(value AS Integer) AS Integer
```

## Package

`bits`

## Imports

```
IMPORT bits
```

`bits` is a built-in package, so `IMPORT bits` needs no manifest dependency.

## Description

`popCount` returns the number of set (`1`) bits in `value`, also known as its
Hamming weight or population count.

`value` is treated as a raw two's-complement 64-bit `Integer` bit pattern;
`popCount` does not interpret sign, so every one of the 64 bit positions is
inspected regardless of whether `value` is negative. When `value` is `0` no bits
are set and the result is `0`; when every bit is set (the bit pattern `-1`) the
result is `64`. The operation is total — it is defined for every `Integer` and
never raises — has no side effects, and lowers to a single native AArch64
population-count instruction rather than calling a runtime helper, producing
identical results on the native and Binary Representation execution paths. [[src/builtins/bits.rs:call_return_type_name]] [[src/target/shared/code/builder_bits.rs:lower_bits_popcount]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `Integer` | The 64-bit value to inspect. Any `Integer` is accepted; treated as a raw bit pattern. [[src/builtins/bits.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Integer` | The count of set bits, in the range `0`..`64`. `0` when `value` is `0`; `64` when every bit is set (`-1`). [[src/builtins/bits.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Count the set bits of a small value:

```
IMPORT bits

LET result AS Integer = bits::popCount(255)
PRINT result
```

The all-ones pattern has 64 set bits:

```
IMPORT bits

PRINT bits::popCount(-1)
```

## See also

- `mfb man bits clz`
- `mfb man bits ctz`
- `mfb man bits bnot`
- `mfb man bits package`
