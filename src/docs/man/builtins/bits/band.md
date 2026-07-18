# band

Bitwise AND of two 64-bit integers.

## Synopsis

```
bits::band(a AS Integer, b AS Integer) AS Integer
```

## Package

`bits`

## Imports

```
IMPORT bits
```

`bits` is a built-in package, so `IMPORT bits` needs no manifest dependency.

## Description

`band` returns the bitwise AND of `a` and `b`, computed independently across all
64 bit positions: bit *i* of the result is `1` only when bit *i* is `1` in both
operands, and `0` otherwise.

Both operands and the result are raw two's-complement 64-bit `Integer` bit
patterns; `band` does not interpret sign. The operation is total — it is defined
for every pair of inputs and never raises — has no side effects, and lowers to a
single native AArch64 `and` instruction inline rather than calling a runtime
helper, producing identical results on the native and Binary Representation
execution paths. [[src/builtins/bits.rs:call_return_type_name]] [[src/target/shared/code/builder_bits.rs:lower_bits_binary]] [[src/target/shared/abi.rs:and_registers]]

The name is `band` rather than `and` because `AND` is a reserved logical
(Boolean) keyword and cannot be a package member identifier. [[src/docs/man/builtins/bits/package.md]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `a` | `Integer` | The first operand. Any 64-bit value; treated as a raw bit pattern. [[src/builtins/bits.rs:call_param_names]] |
| `b` | `Integer` | The second operand. Any 64-bit value; treated as a raw bit pattern. [[src/builtins/bits.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Integer` | The bitwise AND of `a` and `b`. `0` when the operands share no set bits; equal to either operand when it is a subset of the other. [[src/builtins/bits.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Mask off all but the low byte of a value:

```
IMPORT bits

LET value AS Integer = 0x1234
LET low AS Integer = bits::band(value, 255)
PRINT low
```

Test whether a specific bit is set by ANDing with a single-bit mask:

```
IMPORT bits

LET flags AS Integer = 6
LET bit1Set AS Integer = bits::band(flags, 2)
PRINT bit1Set
```

## See also

- `mfb man bits bor`
- `mfb man bits bxor`
- `mfb man bits bnot`
- `mfb man bits package`
