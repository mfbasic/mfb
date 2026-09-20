# bug-659: `math::exp` on a large-magnitude `Fixed` wraps its argument reduction — overflow and underflow swap, silently

Last updated: 2026-09-19
Effort: small–medium
Severity: **MEDIUM** — a silently wrong value (`0.00`) where the correct behavior is
`ErrOverflow`, with no diagnostic. The mirrored case raises where it should return zero.
Class: Correctness (Fixed transcendental kernel)

Status: Open
Regression Test: none yet — see Phase 1

`math::exp` on a `Fixed` argument computes `2^n * exp(r)` with `n = round(x / ln2)`
(`src/codegen/builtins/money/gen_fixed_math.rs:1155` — `emit_fixed_exp`). The `x / ln2`
step is a Q32.32 multiply by `fixed_inv_ln2()` with **no overflow check**, so once
`|x| > 2^31 · ln2 ≈ 1.4885e9` the product leaves `Fixed` range and wraps. `n` is then
extracted from the wrapped value (`n = (scaled + FIXED_HALF) >> 32`,
`gen_fixed_math.rs:1169-1172`) and comes out with the **wrong sign**.

`emit_fixed_scale_by_power_of_two` (`gen_fixed_math.rs:1221`) branches on that sign
(`compare_immediate(n, "0")` / `branch_lt(negative)`, `:1236-1237`): a non-negative `n`
doubles with an overflow check that raises `ErrOverflow`, a negative `n` halves with no
check at all. So a wrapped sign does not merely misreport — it selects the *opposite*
arm, and the overflow check is skipped entirely.

**The single correct behavior a fix produces:** `math::exp` on a `Fixed` raises
`ErrOverflow` for every argument whose true result exceeds `Fixed` range, and returns
`0.00` for every argument whose true result underflows to zero — with no magnitude of
input at which those two swap.

## Failing Reproduction

```basic
IMPORT io
IMPORT math

FUNC probe(v AS Float) AS String
  LET x AS Fixed = toFixed(v)
  LET r = toString(math::exp(x)) TRAP(e)
    RECOVER "RAISED " & toString(e.code)
  END TRAP
  RETURN r
END FUNC

SUB main()
  io::print("exp(-2e9) = " & probe(-2000000000.0))
  io::print("exp(-1e9) = " & probe(-1000000000.0))
  io::print("exp(1e9)  = " & probe(1000000000.0))
  io::print("exp(1.5e9)= " & probe(1500000000.0))
  io::print("exp(2e9)  = " & probe(2000000000.0))
END SUB
```

Observed (macos-aarch64, release compiler built from `main` at `be60f76eb`):

```
exp(-2e9) = RAISED 77050010      <- WRONG: e^-2e9 underflows to 0, must not raise
exp(-1e9) = 0.00                 <- correct
exp(1e9)  = RAISED 77050010      <- correct
exp(1.5e9)= -0.00                <- WRONG: must raise ErrOverflow (note the negative zero)
exp(2e9)  = 0.00                 <- WRONG: must raise ErrOverflow
```

The two behaviors are exactly inverted above the threshold. Measured boundary: `exp(1488522191.0)`
still raises correctly; `exp(1500000000.0)` returns `-0.00`. That bracket contains
`2^31 · ln2 = 1488522190.6`, which is the predicted wrap point of the `x / ln2` product.

Control — the ordinary range is unaffected, so this is a large-magnitude defect only:

```
exp(21)  = 1318815732.25   exp(22) = RAISED 77050010   exp(30)   = RAISED 77050010
exp(-40) = 0.00            exp(-100) = 0.00            exp(-1e6) = 0.00
```

## Root cause

`emit_fixed_exp` (`gen_fixed_math.rs:1158`):

```rust
// n = round(x / ln2).
let inv_ln2 = self.allocate_register();
self.emit_const_i64(&inv_ln2, fixed_inv_ln2());
let scaled = self.emit_fixed_mul(&x, &inv_ln2)?;   // <- unchecked; wraps for |x| > 2^31*ln2
let n = self.allocate_register();
self.emit(abi::move_immediate(&n, "Integer", &FIXED_HALF.to_string()));
self.emit(abi::add_registers(&n, &scaled, &n));
self.emit(abi::arithmetic_shift_right_immediate(&n, &n, 32));
```

`emit_fixed_mul` produces a Q32.32 product with no range check, so `scaled` wraps and the
`asr #32` yields a sign-flipped `n`. `emit_fixed_scale_by_power_of_two` then dispatches on
that sign, and its negative arm has no overflow check (`gen_fixed_math.rs:1259` onward) —
which is why the failure is silent rather than a wrong-but-raised result.

The declared-error list is not implicated: `ErrOverflow` is correctly declared on the
`Fixed` overload (and bug-617 confirms it belongs only there). This is a kernel defect.

## Non-goals

- Changing `math::exp`'s behavior anywhere in the ordinary range. `exp(21)` through
  `exp(1e9)` and `exp(-40)` through `exp(-1e9)` are all correct today and must stay
  byte-identical.
- Changing the `Float` overloads, which raise `ErrFloatInf` by a different path and were
  measured correct.

## Blast-radius audit

- `emit_fixed_scale_by_power_of_two` is shared with `pow` ("Used to recombine the exponent
  in `exp`/`pow`", `gen_fixed_math.rs:1218-1219`). **`math::pow` on a `Fixed` with a large
  exponent must be probed for the same swap** — it very likely has it.
- Any other caller of `emit_fixed_mul` that feeds a shift-count or branch-sign derived from
  the product. Audit for the same unchecked-product-then-sign-branch shape.
- `emit_fixed_log` / the other Fixed transcendentals that perform an argument reduction.

## Fix sketch

Range-check the argument reduction before extracting `n`: if `|x|` exceeds the largest
argument whose `x / ln2` fits Q32.32, the result is already outside `Fixed` range in one
direction or zero in the other, and the branch can be decided from the SIGN OF `x` rather
than from the wrapped product. Equivalently, clamp/guard `scaled` so `n` cannot change sign
relative to `x`. Either way the overflow check must not be skippable by a negative `n`.

## Provenance

Found 2026-09-19 by a sub-audit during bug-617's per-overload error verification (an agent
probing `math::exp`'s `Fixed` overload for `ErrOverflow` reachability noticed
`exp(toFixed(2000000000.0))` returning `0.00`). Independently reproduced and the boundary
bisected on the main thread before filing. Not caused by bug-617's fix, which is
declaration-only and does not touch any lowering — reproduced on the unmodified `main`
compiler.
