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

`ctz` returns the number of zero bits *below* the least significant set (`1`) bit
of `value` — equivalently, the bit index of that lowest set bit — counting up
from bit 0 (the lowest bit) toward bit 63. Bits above the lowest set bit do not
participate: `bits::ctz(40)` is `3` whether the value is `40` (`0b101000`) or
`0b1111_1000`, because both have their lowest set bit at index 3.

`value` is treated as a raw two's-complement 64-bit `Integer` bit pattern; `ctz`
does not interpret sign. Negative values are not special-cased the way they are
for `bits::clz`: `bits::ctz(-1)` is `0` (every bit is set), while
`bits::ctz(-2)` is `1`. When bit 0 is set — that is, whenever `value` is odd —
the result is `0`. When `value` is `0` there is no set bit at all, so all 64 bits
count as trailing zeros and the result is `64`; this zero case is the boundary
that most bit-scan primitives leave architecturally undefined, and `mfb` defines
it on every target. The operation is total: it is defined for every `Integer`,
never raises, and has no side effects. [[src/builtins/bits.rs:call_return_type_name]]
[[src/target/shared/code/builder_bits.rs:lower_bits_count_zeros]]

Because the result is the index of the lowest set bit, `ctz` is the primitive
behind alignment and power-of-two work. For a positive power of two,
`bits::ctz(value)` is exactly its base-2 exponent, so it inverts
`bits::sl(1, n)`. A pointer or size is `2^k`-aligned exactly when
`bits::ctz(value) >= k`, which is a cheaper test than a modulo. And `ctz`
composes with the lowest-set-bit idiom `value AND -value`, which clears every
bit but the lowest one: iterating "extract lowest bit, `ctz` it, clear it" walks
a bitmask's set indices in ascending order, one iteration per set bit rather than
one per word bit.

Unlike `bits::clz`, `ctz` scans from the bottom of the word, so it is insensitive
to the width of the whole `Integer` and reports the same answer for a value
whether or not it has been narrowed — `bits::ctz(1)` is `0` for an 8-bit field
and for a 64-bit one alike. That makes it the safer of the two when working with
packed sub-fields. The one place width does intrude is the all-zero input, where
the `64` result reflects the `Integer` width rather than your field's. The mirror
operation, counting zeros from the top, is `bits::clz`; for the count of set bits
anywhere in the word see `bits::popCount`. Note the identity
`bits::ctz(value) = bits::popCount(bits::band(value, -value) - 1)`, which holds
for every `value` including `0`.

`ctz` lowers inline rather than calling a runtime helper: the backend reverses
the bit order of the operand and then counts leading zeros of the reversal, so
`ctz` costs one `rbit` plus a full `clz` on every architecture.
[[src/target/shared/code/builder_bits.rs:lower_bits_count_zeros]]
[[src/target/shared/abi.rs:reverse_bits]]

The instruction budget of that pair differs sharply by target. AArch64 is the
only architecture where both halves are native, encoding the operation as
`RBIT Xd, Xn` followed by `CLZ Xd, Xn` — two instructions total.
[[src/arch/aarch64/encode/emitter.rs:emit_rbit]] x86-64 has no bit-reverse
instruction, so the emitter expands `rbit` into a five-level SWAR swap network
(alternating bits, then pairs, nibbles, bytes, and 16-bit halves) before the
`lzcnt`. That expansion uses `rax` as its mask register and `rdx` as its scratch
accumulator, and preserves both with an explicit push/pop because the allocator's
clobber model does not cover a multi-instruction expansion; consequently the
emitter rejects a `dst` coloured onto `rax` or `rdx`, since the trailing pop
would restore the register and discard the result (bug-284 C6).
[[src/arch/x86_64/encode/emitter.rs:rbit]] RISC-V has neither `rbit` nor `clz` in
base RV64I and assumes no Zbb dependency, so it pays both expansions: the same
five-level swap network plus a byte reverse, followed by `clz`'s six-step
shift-or smear and SWAR population count — roughly four dozen instructions, and
the sequence clobbers the `t0`–`t2` scratch registers.
[[src/arch/riscv64/encode/emitter.rs:emit_reversal]]
[[src/arch/riscv64/encode/sizing.rs:RBIT_LEVELS]]

Despite the three very different encodings the result is identical on every
architecture and on both the native and Binary Representation execution paths. If
you are counting instructions on a hot RISC-V path, prefer `bits::band(value, -value)`
plus a comparison over `ctz` when a boolean alignment test is all you need.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `Integer` | The 64-bit value to inspect. Any `Integer` is accepted; treated as a raw two's-complement bit pattern, not as a signed magnitude. [[src/builtins/bits.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Integer` | The trailing-zero count — the index of the lowest set bit — in the range `0` to `64` inclusive. `0` when bit 0 is set (the value is odd, which includes `-1`); `64` when `value` is `0`. [[src/builtins/bits.rs:call_return_type_name]] |

## Errors

No errors. `ctz` is total over every `Integer` input. Only the variable-shift ops
`bits::sl`, `bits::sr`, and `bits::sra` can raise from the `bits` package, and
they do so for an out-of-range `count`. [[src/builtins/bits.rs:is_bits_shift]]

## Examples

Count the trailing zeros of `40` (`0b101000`) — its lowest set bit is at index 3:

```
IMPORT bits
IMPORT io

SUB main()
  LET result AS Integer = bits::ctz(40)
  io::print(toString(result))
END SUB
```

The all-zero pattern has 64 trailing zeros, while any odd value has none:

```
IMPORT bits
IMPORT io

SUB main()
  io::print(toString(bits::ctz(0)))
  io::print(toString(bits::ctz(255)))
  io::print(toString(bits::ctz(-1)))
END SUB
```

Recover the exponent of a power of two, inverting `bits::sl`:

```
IMPORT bits
IMPORT io

SUB main()
  LET n AS Integer = bits::sl(1, 20)
  io::print(toString(bits::ctz(n)))
END SUB
```

Test whether a value is aligned to a `2^k` boundary without a modulo:

```
IMPORT bits
IMPORT io

FUNC isAligned(value AS Integer, k AS Integer) AS Boolean
  IF value = 0 THEN
    RETURN TRUE
  END IF
  RETURN bits::ctz(value) >= k
END FUNC

SUB main()
  io::print(toString(isAligned(4096, 12)))
  io::print(toString(isAligned(4100, 12)))
END SUB
```

Walk the set bits of a mask in ascending index order, one iteration per set bit:

```
IMPORT bits
IMPORT io

SUB main()
  MUT mask AS Integer = 0b1001_0100
  WHILE mask <> 0
    LET lowest AS Integer = bits::band(mask, -mask)
    io::print(toString(bits::ctz(lowest)))
    mask = bits::bxor(mask, lowest)
  WEND
END SUB
```

## See also

- `mfb man bits clz`
- `mfb man bits popCount`
- `mfb man bits band`
- `mfb man bits sl`
- `mfb man bits package`
