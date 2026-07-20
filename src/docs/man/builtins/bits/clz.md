# clz

Count the leading zero bits of a 64-bit integer.

## Synopsis

```
bits::clz(value AS Integer) AS Integer
```

## Package

`bits`

## Imports

```
IMPORT bits
```

`bits` is a built-in package, so `IMPORT bits` needs no manifest dependency.

## Description

`clz` returns the number of zero bits that precede the most significant set (`1`)
bit of `value`, counting down from bit 63 (the highest bit) toward bit 0.

`value` is treated as a raw two's-complement 64-bit `Integer` bit pattern; `clz`
does not interpret sign. When `value` is `0` there is no set bit, so all 64 bits
count as leading zeros and the result is `64`. When bit 63 is set the result is
`0`. The operation is total — it is defined for every `Integer` and never raises
— has no side effects, and lowers to a single native AArch64 count-leading-zeros
instruction rather than calling a runtime helper, producing identical results on
the native and Binary Representation execution paths. [[src/builtins/bits.rs:call_return_type_name]] [[src/target/shared/code/builder_bits.rs:lower_bits_count_zeros]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `Integer` | The 64-bit value to inspect. Any `Integer` is accepted; treated as a raw bit pattern. [[src/builtins/bits.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Integer` | The leading-zero count, in the range `0`..`64`. `0` when bit 63 is set; `64` when `value` is `0`. [[src/builtins/bits.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Count the leading zeros of a small value:

```
IMPORT bits
IMPORT io

SUB main()
  LET result AS Integer = bits::clz(255)
  io::print(toString(result))
END SUB
```

The all-zero pattern has 64 leading zeros:

```
IMPORT bits
IMPORT io

SUB main()
  io::print(toString(bits::clz(0)))
END SUB
```

## See also

- `mfb man bits ctz`
- `mfb man bits popCount`
- `mfb man bits bnot`
- `mfb man bits package`
