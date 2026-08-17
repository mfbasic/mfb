# Bit Operations (bits)

The `bits` package is the language's low-level integer bit-twiddling layer: the
logical, shift, rotate, bit-counting, and byte-swap primitives that the operator
set intentionally omits. `IMPORT bits` needs no manifest dependency. This topic
specifies the *semantic model* behind the package — the value model every op
shares, the exact behavior of each op family, and the "one native instruction"
property. The per-function API — signatures, parameters, return types — is owned
by `./mfb man bits`. [[src/codegen/registry/mod.rs:is_member]]

Higher-level codecs are built directly on these primitives: the `encoding`
package leans on `bits` for its bit-buffer engines and varint arithmetic
(`./mfb spec stdlib encoding`), and the `crypto` software cores compute over
them.

## Value and operand model

Every `bits::` op takes and returns `Integer` and nothing else — never Float,
`Byte`, `String`, or a collection. A call whose argument is not `Integer` does
not resolve, so a mistyped operand is a compile-time error rather than a coercion.
[[src/builtins/mod.rs:resolve_call_return_type]] [[src/codegen/registry/mod.rs:call_return_type]]

Each operand and result is a **raw two's-complement 64-bit bit pattern**. The
ops do not interpret sign — with one deliberate exception, the arithmetic right
shift `bits::sra`, which replicates the sign bit. Everywhere else a negative
`Integer` is simply its 64-bit two's-complement encoding, and a value that would
overflow a signed 64-bit `Integer` is nonetheless a well-defined bit pattern the
ops act on directly (no trapping-overflow check applies to bit operations). This
is what makes `bits` the substrate for byte-level codecs: the sign bit is just
bit 63.

## Logical ops (band / bor / bxor / bnot)

The four Boolean-per-bit operations are named `band`/`bor`/`bxor`/`bnot`
precisely because `AND`/`OR`/`XOR`/`NOT` are reserved *logical* (Boolean-valued)
keywords in the language and cannot be package member identifiers. `band`, `bor`,
and `bxor` are binary and compute the bitwise conjunction, disjunction, and
exclusive-or of their two 64-bit operands; `bnot` is unary and complements every
bit. All four are **total** — they have no failing input.
[[src/codegen/builtins/bits/func_band.rs:lower_bits_band]] [[src/codegen/builtins/bits/func_bnot.rs:lower_bits_bnot]]

## Shifts (sl / sr / sra)

The three shifts move bits by a runtime `count` and are the **only** `bits::`
ops that can raise. Before shifting, `count` is validated to lie in `0..63`
inclusive; a count below `0` or above `63` raises `ErrInvalidArgument`
(`77050002`) and shifts nothing. The amount is **not** implicitly clamped or
reduced modulo the width — an out-of-range shift is an error, not a defined
no-op, which keeps behavior identical to the native variable-shift instruction
only over the range where that instruction is unambiguous.
[[src/codegen/builtins/bits/func_sl.rs:lower_bits_sl]] [[src/codegen/builtins/errorcode/mod.rs:ErrInvalidArgument]]

Within the valid range:

- **`sl`** shifts left; vacated low bits are zero and bits shifted past bit 63
  are discarded.
- **`sr`** is the **logical** right shift: vacated high bits are filled with
  **zero**, so the sign bit is not replicated.
- **`sra`** is the **arithmetic** right shift: vacated high bits are filled with
  a copy of bit 63 (the sign bit), so a negative value stays negative. `sr` and
  `sra` coincide for any non-negative value.

A `count` of `0` returns `value` unchanged for all three. Because the three
shifts can fail, they participate in the inline-`TRAP` fallibility census while
every other `bits::` op is treated as infallible. [[src/builtins/mod.rs:inline_builtin_raw_supported]]

## Rotates (rl32 / rr32 / rl64 / rr64)

Rotates come in four named width variants and are **total** — no count is
rejected. The width names the modulus the rotate wraps at:

- **`rl64` / `rr64`** rotate all 64 bits: bits leaving one end re-enter the
  other.
- **`rl32` / `rr32`** rotate only the **low 32 bits** (for word-oriented
  algorithms such as ChaCha20), operating on the low word and clearing the high
  32 bits of the result.

The rotate amount is reduced **modulo the rotate width** by the hardware, so any
count — including a negative or large one — is defined and no rotate raises. The
target architecture provides only a rotate-*right* primitive, so a left rotate by
`count` is realized as a right rotate by `-count`, which the width-modular
reduction makes exact. [[src/codegen/builtins/bits/func_rl64.rs:lower_bits_rl64]]

## Bit counting (clz / ctz / popCount)

- **`clz`** counts leading (most-significant) zero bits; **`ctz`** counts
  trailing (least-significant) zero bits. Both return **`64` for a zero input**
  (every bit is a zero to count). `ctz` is realized by reversing the bit order
  and counting leading zeros. [[src/codegen/builtins/bits/func_clz.rs:lower_bits_clz]]
- **`popCount`** is the 64-bit Hamming weight — the number of set bits, `0..64`.
  It is computed with the standard SWAR fold (shift/mask/multiply over the
  integer ALU), not a SIMD population-count, so it produces the identical count
  through the same integer-only path on every backend. [[src/codegen/builtins/bits/func_pop_count.rs:lower_bits_pop_count]]

All three are total.

## Byte swaps (bswap16 / bswap32 / bswap64)

The byte-reversal ops reverse the byte order of a fixed low-width field and clear
everything above it:

- **`bswap16`** reverses the low **2** bytes and zeroes bits 16..63.
- **`bswap32`** reverses the low **4** bytes and zeroes bits 32..63.
- **`bswap64`** reverses all **8** bytes.

These are pure byte-order reversals — the tool for converting between big-endian
and little-endian serializations of a fixed-width field. They are total.
[[src/codegen/builtins/bits/func_bswap16.rs:lower_bits_bswap16]]

## One native instruction per op

Every `bits::` op lowers to one (or a few) native machine instructions **inline**
— the same way `math::abs` does — rather than calling a runtime helper or any
libm/source routine. `band` is a single `AND`, `clz` a single count-leading-zeros,
`bswap64` a single byte-reverse, and so on; only `popCount` (the SWAR fold) and
the derived left-rotate/`ctz`/`bswap16` forms expand to a short fixed instruction
sequence, and none of these branch on data. [[src/codegen/registry/mod.rs:native_lower]]

The consequences are determinism and portability. Because the operations are
plain integer-ALU work over a two's-complement bit pattern — with no floating
point, no SIMD population-count, and no platform library — each op produces a
**byte-identical** result on all three backends and on both execution paths (the
native code and the Binary Representation interpreter agree). The lowering carries
no hidden state and no allocation, so bit-level codecs written on `bits` compute
the same output regardless of target.

## Error model

Only `sl`, `sr`, and `sra` can raise, and only for one reason: a shift `count`
outside `0..63`, reported as `ErrInvalidArgument` (`77050002`). Every logical,
rotate, bit-counting, and byte-swap op is **total** — it is defined for every
64-bit input and has no failing case (rotate counts are width-modular; `clz`/`ctz`
define zero as `64`). No `bits::` op range-checks its *value* operand; the only
check anywhere in the package is the shift-count bounds test.
[[src/codegen/builtins/bits/func_sl.rs:lower_bits_sl]] [[src/builtins/mod.rs:inline_builtin_raw_supported]]

## See Also

- `./mfb man bits` — the per-function API reference (band, bor, bxor, bnot, sl,
  sr, sra, rl32/rr32/rl64/rr64, clz, ctz, popCount, bswap16/32/64).
- `./mfb spec stdlib encoding` — the codec package that builds its bit-buffer and
  varint engines on these primitives.
- `./mfb spec language operators` — the logical `AND`/`OR`/`XOR`/`NOT` operators
  that `band`/`bor`/`bxor`/`bnot` deliberately complement.
- `./mfb spec language types` — the `Integer` (signed two's-complement 64-bit)
  domain every `bits::` op acts on.
- `./mfb spec architecture aarch64-instruction-set` — the native instructions the
  ops lower to.
- `./mfb spec diagnostics error-codes` — `ErrInvalidArgument` and the shared
  runtime error codes.
