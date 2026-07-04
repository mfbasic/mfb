# bor

Bitwise OR of two 64-bit integers.

## Synopsis

```
bits::bor(a AS Integer, b AS Integer) AS Integer
```

## Package

`bits`

## Imports

```
IMPORT bits
```

`bits` is a built-in package, so `IMPORT bits` needs no manifest dependency.

## Description

`bor` returns the bitwise OR of `a` and `b`, computed independently across all
64 bit positions: bit *i* of the result is `1` when bit *i* is `1` in either
operand (or both), and `0` only when bit *i* is `0` in both operands.

Both operands and the result are raw two's-complement 64-bit `Integer` bit
patterns; `bor` does not interpret sign. The operation is total — it is defined
for every pair of inputs and never raises — has no side effects, and lowers to a
single native AArch64 `orr` instruction inline rather than calling a runtime
helper, producing identical results on the native and Binary Representation
execution paths. [[src/builtins/bits.rs:call_return_type_name]] [[src/target/shared/code/builder_bits.rs:lower_bits_call]]

The name is `bor` rather than `or` because `OR` is a reserved logical (Boolean)
keyword and cannot be a package member identifier. [[src/docs/man/builtins/bits/package.md]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `a` | `Integer` | The first operand. Any 64-bit value; treated as a raw bit pattern. [[src/builtins/bits.rs:call_param_names]] |
| `b` | `Integer` | The second operand. Any 64-bit value; treated as a raw bit pattern. [[src/builtins/bits.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Integer` | The bitwise OR of `a` and `b`. Equal to either operand when the other is `0`; equal to `-1` (all bits set) when the operands together cover every bit position. [[src/builtins/bits.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Combine two single-bit flag masks into one value:

```
IMPORT bits

LET flags AS Integer = bits::bor(1, 4)
PRINT flags
```

Force the low two bits of a value on, leaving the rest unchanged:

```
IMPORT bits

LET value AS Integer = 0x1234
LET result AS Integer = bits::bor(value, 3)
PRINT result
```

## See also

- `mfb man bits band`
- `mfb man bits bxor`
- `mfb man bits bnot`
- `mfb man bits package`
