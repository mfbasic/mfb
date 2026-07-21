# sra

Arithmetic (sign-filling) right shift of a 64-bit integer.

## Synopsis

```
bits::sra(value AS Integer, count AS Integer) AS Integer
```

## Package

`bits`

## Imports

```
IMPORT bits
```

`bits` is a built-in package, so `IMPORT bits` needs no manifest dependency.

## Description

`sra` shifts `value` right by `count` bit positions as a signed quantity.
Vacated high bits are filled with a copy of the sign bit (bit 63 of `value`),
and bits shifted past bit 0 are discarded. A `count` of `0` returns `value`
unchanged.

Both `value` and the result are raw two's-complement 64-bit `Integer` bit
patterns. Because the sign bit is replicated into the vacated high bits,
shifting a negative value keeps it negative — this is the distinction from the
logical right shift `bits::sr`, which zero-fills the vacated high bits. For a
non-negative `value` the two produce identical results. For the left shift see
`bits::sl`.

`sra` is not the same as signed division by a power of two. Because the
discarded low bits are dropped rather than rounded, a negative result is
rounded toward negative infinity, not toward zero: `bits::sra(-1, 1)` is `-1`
where `-1 / 2` is `0`, and `bits::sra(-3, 1)` is `-2`. A `count` of `63`
therefore collapses `value` to `0` when it is non-negative and to `-1` (all
bits set) when it is negative.

Unlike the total bitwise operations, `sra` validates `count`: it first checks
that `count` is in the range `0` to `63` inclusive and raises
`ErrInvalidArgument` for any value outside it, before performing the shift.
Larger shift amounts are not implicitly clamped or reduced modulo the width —
that is the difference from the rotates `bits::rl64` and `bits::rr64`, which
accept any `count` and let the hardware reduce it. [[src/target/shared/code/builder_bits.rs:lower_bits_shift]] [[src/target/shared/code/builder_bits.rs:lower_bits_rotate]]

The operation has no side effects and lowers inline to the target-neutral `asrv`
machine op rather than calling a runtime helper. Every backend encodes it
natively: `asrv Xd, Xn, Xm` on AArch64, `sra rd, rs1, rs2` on RISC-V, and a
`mov` of the count into `rcx` followed by `sar dst, cl` on x86-64, whose shift
instruction takes its variable count only in `cl`. The result is identical on
every architecture and on both the native and Binary Representation execution
paths. [[src/target/shared/abi.rs:arithmetic_shift_right_variable]] [[src/arch/aarch64/encode/emitter.rs:emit_asrv]] [[src/arch/riscv64/encode/emitter.rs:emit_r]] [[src/arch/x86_64/encode/emitter.rs:var_shift]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `Integer` | The value to shift. Any 64-bit value; treated as a signed two's-complement bit pattern. [[src/builtins/bits.rs:call_param_names]] |
| `count` | `Integer` | The shift amount in bits. Must be in the range `0` to `63` inclusive; any other value raises `ErrInvalidArgument`. [[src/target/shared/code/builder_bits.rs:lower_bits_shift]] |

## Return value

| Type | Description |
| --- | --- |
| `Integer` | `value` shifted right by `count` bits, with vacated high bits set to the original sign bit and bits below bit 0 discarded. Equal to `value` when `count` is `0`. [[src/builtins/bits.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `count` is less than `0` or greater than `63`. [[src/target/shared/code/builder_bits.rs:lower_bits_shift]] [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |

## Examples

Arithmetic shift of a negative value preserves its sign (signed divide by 16):

```
IMPORT bits
IMPORT io

SUB main()
  LET result AS Integer = bits::sra(-256, 4)
  io::print(toString(result))
END SUB
```

Sign-extend the low byte of a packed field by shifting it up to bit 63 and back
down:

```
IMPORT bits
IMPORT io

SUB main()
  LET byte AS Integer = 0x80
  LET signed AS Integer = bits::sra(bits::sl(byte, 56), 56)
  io::print(toString(signed))
END SUB
```

## See also

- `mfb man bits sr`
- `mfb man bits sl`
- `mfb man bits band`
- `mfb man bits rr64`
- `mfb man bits package`
