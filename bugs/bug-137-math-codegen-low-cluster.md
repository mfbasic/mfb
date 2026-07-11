# bug-137 — math codegen LOW cluster: host-libm consts, XOR bump-alloc, Fixed pow neg-exp error code, rand modulo bias, pow(-0.0), FMA fusion latent

**Status:** OPEN. Filed 2026-07-11 (goal-02 review, G9). Six independent LOW /
latent findings in the math/numeric codegen, batched per goal-02.

## 1. Fixed transcendental constants computed with host libm → cross-host codegen nondeterminism

`src/target/shared/code/builder_fixed_math.rs:894-941` (`fixed_raw`,
`cordic_atan_raw` uses `f64::atan()`, `cordic_gain_inverse` uses `sqrt()`). The
Q32.32 constants embedded in generated code are computed via the compiler
*host's* libm. A ≤1-ulp host-libm difference can flip `(x·2^32).round()` for
values whose scaled fraction sits near .5, producing byte-different generated
programs per build host — undermining the "deterministic across targets"
contract and the byte-identical gates (latent: gates compare one-host builds).
Fix: precompute the constants with an exact/rational method at build time, not
host libm.

## 2. `a XOR f()` reads left operand's register after right's lowering without a spill (bump-alloc miscompile)

`src/target/shared/code/builder_numeric.rs:68-86` (`lower_boolean_xor`). Unlike
`lower_arithmetic_binary`/`lower_comparison_binary` (which spill left before
lowering right), XOR keeps `left.location` live in a register across
`lower_value(right)`. Under `-regalloc bump`, right's lowering re-hands out the
same register (and calls clobber x0–x17), so XOR reads right's value for both
operands. LinearScan (default) is safe; bump is the reference oracle. Trigger:
`-regalloc bump` build of `flag XOR check()` → always false. Fix: spill left to
a stack slot before lowering right, as the arithmetic/comparison siblings do.

## 3. Fixed pow with negative whole exponent and underflowed base traps ErrInvalidArgument instead of ErrOverflow

`src/target/shared/code/builder_fixed_math.rs:795-827`
(`emit_fixed_pow_general` integer-exponent path). For `|base| < 1` and a large
negative whole exponent, the forward product truncates to 0 (bug-61 early exit),
then the reciprocal computes `1.0/0` and raises ErrInvalidArgument;
mathematically the result overflows Fixed range, so ErrOverflow is the truthful
code. Trigger: `math::pow(0.5F, -80.0F)` → ErrInvalidArgument (expected
ErrOverflow). Fix: detect the underflow-to-zero forward product and raise
ErrOverflow before the reciprocal.

## 4. `math::rand(min, max)` has modulo bias

`src/target/shared/code/builder_math.rs:951-965` (`lower_math_rand` remainder
reduction). The inclusive span is reduced with `raw mod span` from a single
64-bit PCG64 draw — biased by up to span/2^64, no rejection sampling; the doc
says "uniform inclusive". Negligible for small spans, measurable near 2^63. Fix:
rejection-sample (Lemire) for uniformity.

## 5. `pow(-0.0, non-integer y)` traps ErrFloatNan (dropped fdlibm ±0 prologue)

`src/target/shared/code/builder_pow.rs:443-484` (`emit_pow_yisint` sign test on
raw bits :455-457). The x<0 test is a signed compare of the raw bit pattern, so
−0.0 classifies as negative and a non-integer exponent routes to ret_nan.
IEEE/fdlibm: pow(−0.0, 0.5) = +0. Trigger: `math::pow(-0.0, 0.5)` → ErrFloatNan
(−0.0 constructible via `0.0 * (0.0-1.0)`). Fix: add the ±0 prologue.

## 6. FMA fusion peephole is label/control-flow-blind (latent)

`src/target/shared/code/fma_fusion.rs:89-120` (`fuse_scalar_fma` consumer search
+ redefinition guard). The single-consumer search and redefinition guard scan
only the textual span (i, j); a branch into a label inside the span from code
after j that redefines a multiply operand would make the fused op read the
redefined value. Safe today (each product vreg is fresh, single-def, consumed
within one structured expression); latent if any emitter routes a back-edge
between a product's fmul and its fadd. Fix: bail fusion when a label appears in
the span, or verify no control-flow edge crosses it.
