# bug-129 — fdlibm `pow` port returns garbage / 0 for subnormal-range results

**Status:** FIXED (commit e0fa88b8, 2026-07-11).
**Severity:** HIGH — `math::pow` silently returns wrong finite values across the
entire subnormal result band (both scalar and array overloads).
**Class:** correctness (silent wrong value; passes the finiteness check).

## Finding

`src/target/shared/code/builder_pow.rs:735-760` (`emit_pow_exp2` final scaling),
:779-790 (`emit_pow_scalbn`).

fdlibm's `if ((j>>20) <= 0) z = scalbn(z, n)` uses a signed 32-bit `j`. Here
`j = hi32(z) + (n<<20)` lives in a 64-bit register and the test is `LSR #20` +
`<= 0`: for n ≤ −1024, `j` is negative-as-64-bit, LSR yields a huge positive
value, and the NORMAL path runs `set_hi(z, j & 0xffffffff)` — constructing a
sign-bit-set, huge-exponent double. When the scalbn path *is* taken (n ≈ −1023),
`emit_pow_scalbn` builds the factor as `(1023+n)<<52` = bit pattern 0 = +0.0,
flushing the result to 0 (the comment's "1023+n stays a valid exponent" is false
at the boundary). The underflow gate only cuts at |y·log2 x| ≥ ~1075.19, so the
whole subnormal result range is reachable.

## Trigger

- `math::pow(2.0, -1030.0)` → ≈ −3.7e307 (bits 0xFF900000_xxxxxxxx) instead of
  2^-1030 ≈ 8.7e-311.
- `math::pow(2.0, -1023.0)` → 0.0 instead of 2^-1023.

Silent — the result is finite so `emit_float_result_check` passes.

## Fix sketch

Implement the signed-32-bit `j` semantics correctly (sign-extend / `ASR`, not
`LSR`), and build the scalbn factor for the subnormal boundary the way fdlibm
does (two-step scaling with the 2^54 compensation), so subnormal results are
produced instead of a malformed exponent or +0.0.

## Prior art

bug-61 covers Fixed/Integer pow only; the ULP harness reference vectors
evidently exclude the subnormal band.

## Resolution

FIXED in commit e0fa88b8. arithmetic shift for the signed-j exponent test + faithful two-step scalbn (bias +54, x 2**-54).

Regression test: `tests/rt-behavior/math/bug129_pow_subnormal` (fails on the unfixed compiler). Full
acceptance (871) and `cargo test` pass.
