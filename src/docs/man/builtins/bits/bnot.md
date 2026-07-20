# bnot

Bitwise NOT (one's complement) of a 64-bit integer.

## Synopsis

```
bits::bnot(a AS Integer) AS Integer
```

## Package

`bits`

## Imports

```
IMPORT bits
```

`bits` is a built-in package, so `IMPORT bits` needs no manifest dependency.

## Description

`bnot` returns the one's complement of `a`: every one of the 64 bit positions is
inverted, so bit *i* of the result is `1` exactly when bit *i* of `a` is `0`, and
`0` otherwise. As a two's-complement arithmetic identity this equals `-(a) - 1`.

The operand and the result are raw two's-complement 64-bit `Integer` bit
patterns; `bnot` does not interpret sign. The operation is total — it is defined
for every input and never raises — has no side effects, and lowers to a single
native AArch64 `mvn` instruction inline rather than calling a runtime helper,
producing identical results on the native and Binary Representation execution
paths. [[src/builtins/bits.rs:call_return_type_name]] [[src/target/shared/code/builder_bits.rs:lower_bits_not]] [[src/target/shared/abi.rs:bitwise_not]]

The name is `bnot` rather than `not` because `NOT` is a reserved logical
(Boolean) keyword and cannot be a package member identifier. [[src/docs/man/builtins/bits/package.md]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `a` | `Integer` | The operand to invert. Any 64-bit value; treated as a raw bit pattern. [[src/builtins/bits.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Integer` | The bitwise complement of `a`, with all 64 bits inverted. `bnot(0)` is `-1` (all bits set) and `bnot(-1)` is `0`. [[src/builtins/bits.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Invert every bit of a value:

```
IMPORT bits
IMPORT io

SUB main()
  LET result AS Integer = bits::bnot(255)
  io::print(toString(result))
END SUB
```

Clear the low byte by ANDing with an inverted mask:

```
IMPORT bits
IMPORT io

SUB main()
  LET value AS Integer = 0x1234
  LET highOnly AS Integer = bits::band(value, bits::bnot(255))
  io::print(toString(highOnly))
END SUB
```

## See also

- `mfb man bits band`
- `mfb man bits bor`
- `mfb man bits bxor`
- `mfb man bits package`
