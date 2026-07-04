# bxor

Bitwise exclusive-OR of two 64-bit integers.

## Synopsis

```
bits::bxor(a AS Integer, b AS Integer) AS Integer
```

## Package

`bits`

## Imports

```
IMPORT bits
```

`bits` is a built-in package, so `IMPORT bits` needs no manifest dependency.

## Description

`bxor` returns the bitwise exclusive-OR of `a` and `b`, computed independently
across all 64 bit positions: bit *i* of the result is `1` when bit *i* differs
between the two operands, and `0` when the two bits are equal.

Both operands and the result are raw two's-complement 64-bit `Integer` bit
patterns; `bxor` does not interpret sign. The operation is total — it is defined
for every pair of inputs and never raises — has no side effects, and lowers to a
single native AArch64 `eor` instruction inline rather than calling a runtime
helper, producing identical results on the native and Binary Representation
execution paths. [[src/builtins/bits.rs:call_return_type_name]] [[src/target/shared/code/builder_bits.rs:lower_bits_call]]

XORing a value with itself yields `0`, and XORing with `0` returns the value
unchanged, so `bxor` is its own inverse: `bits::bxor(bits::bxor(x, k), k)`
recovers `x`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `a` | `Integer` | The first operand. Any 64-bit value; treated as a raw bit pattern. [[src/builtins/bits.rs:call_param_names]] |
| `b` | `Integer` | The second operand. Any 64-bit value; treated as a raw bit pattern. [[src/builtins/bits.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Integer` | The bitwise XOR of `a` and `b`. `0` when the operands are equal; equal to either operand when the other is `0`. [[src/builtins/bits.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Toggle the low byte of a value by XORing with an all-ones mask:

```
IMPORT bits

LET value AS Integer = 0x1234
LET toggled AS Integer = bits::bxor(value, 255)
PRINT toggled
```

Swap two integers without a temporary using the XOR identity:

```
IMPORT bits

LET x AS Integer = 5
LET y AS Integer = 9
LET x2 AS Integer = bits::bxor(x, y)
LET y2 AS Integer = bits::bxor(x2, y)
LET x3 AS Integer = bits::bxor(x2, y2)
PRINT x3
PRINT y2
```

## See also

- `mfb man bits band`
- `mfb man bits bor`
- `mfb man bits bnot`
- `mfb man bits package`
