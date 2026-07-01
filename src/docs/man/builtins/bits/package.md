# bits

Integer bitwise, shift, and rotate operations

## Synopsis

```
IMPORT bits
bits::band(a, b)
bits::sl(value, count)
bits::rl32(value, count)
bits::popCount(value)
bits::bswap32(value)
```

## Description

The `bits` package provides the bitwise integer operations the language operator
set intentionally omits. The reserved words `AND`, `OR`, `XOR`, and `NOT` are
logical (Boolean) operators, so byte-level codecs and other bit-twiddling are
written with these functions instead. The Boolean operations are named
`band`/`bor`/`bxor`/`bnot` precisely because `and`/`or`/`xor`/`not` are reserved
logical keywords and cannot be package member identifiers. [[src/builtins/bits.rs:is_bits_call]]

Every operand and result is a raw two's-complement 64-bit `Integer` bit pattern.
The functions do not interpret sign except where a signature says so — `sra`, the
arithmetic right shift. Every function takes and returns `Integer`, never Float,
String, or a collection. [[src/builtins/bits.rs:call_return_type_name]]

Each function lowers to one (or a few) native instructions inline, like
`math::abs`, rather than calling a runtime helper, and produces identical results
on the native and Binary Representation execution paths. [[src/target/shared/code/builder_bits.rs:lower_bits_call]]

Shifts (`sl`, `sr`, `sra`) validate their `count` argument. Rotates come in four
named width variants — `rl32`/`rr32` rotate the low 32 bits (for word-oriented
algorithms such as ChaCha20) and `rl64`/`rr64` rotate all 64 bits. Rotate counts
are reduced modulo the rotate width, so any count is defined and the rotates do
not raise. `clz`/`ctz`/`popCount` count leading zeros, trailing zeros, and set
bits; `bswap16`/`bswap32`/`bswap64` reverse the bytes of the low 16/32 or all 64
bits. All functions are total except the three shifts. [[src/target/shared/code/builder_bits.rs:lower_bits_rotate]]

`bits` is a built-in package: `IMPORT bits` needs no manifest dependency.

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | raised by `sl`, `sr`, and `sra` when `count` is outside `0..63` [[src/target/shared/code/builder_bits.rs:99]] |
