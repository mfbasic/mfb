# bug-305: `crypto::randomInt` hangs (infinite loop) when the requested range exceeds 2^62

Last updated: 2026-07-17
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness / Edge-case (DoS)

Status: Open
Regression Test: tests/ (new) — `crypto::randomInt` over a range > 2^62 returns or errors, never hangs

`__crypto_randomInt` draws entropy from `__crypto_rand62` (62 bits, `maxVal =
2^62`) and rejection-samples with `limit = maxVal - (maxVal MOD range)`. When
`range > maxVal` (i.e. `max - min + 1 > 2^62`), `maxVal MOD range = maxVal`, so
`limit = 0` and `WHILE v >= limit` (v is always ≥ 0) never terminates. The only
guard is `range <= 0` (i64 overflow), which does not catch ranges in
`(2^62, 2^63-1]`.

The single correct behavior a fix produces: `crypto::randomInt(min, max)` for any
valid `min <= max` in i64 range terminates — either returning a uniformly random
value or raising a clear error for an unsupported range.

References:

- Found during goal-06 review of `src/builtins/crypto_package.mfb`.

## Failing Reproduction

```
' crypto::randomInt(0, 9223372036854775806)            -> hangs
' crypto::randomInt(0, 4611686018427387905)  (2^62+1)  -> hangs
' crypto::randomInt(0, 100)                             -> returns
```

- Observed: hangs (verified with a 4 s timeout) for range > 2^62.
- Expected: returns a value (or raises `ErrInvalidArgument`).

## Root Cause

`src/builtins/crypto_package.mfb:1526` (`__crypto_randomInt`) using
`__crypto_rand62` (`:1513`): the rejection bound `limit = maxVal - (maxVal MOD
range)` collapses to 0 when `range > maxVal = 2^62`, so the rejection loop never
finds an acceptable draw.

## Goal

- Either reject `range > 2^62` with `ErrInvalidArgument`, or draw wider entropy
  (a 64-bit-or-more value with a correspondingly larger `maxVal`) so the rejection
  window is non-empty across the full i64 range.

### Non-goals (must NOT change)

- The uniform-distribution property for supported ranges.
- The `range <= 0` overflow guard.

## Blast Radius

- `__crypto_randomInt` / `__crypto_rand62` — fixed here.
- No other caller of `__crypto_rand62` assumes a 2^62 ceiling for a range this large
  (verify during fix).

## Fix Design

Preferred: widen entropy to 63/64 bits (draw an extra byte, mask to a value whose
`maxVal ≥ i64::MAX`) so any valid i64 range has a non-empty rejection window;
simpler interim: reject `range > 2^62` with a clear error. Rejected alternative:
leaving the loop — an unbounded hang on valid arguments is a DoS.

## Phases

### Phase 1 — failing test
- [ ] Test the two hanging ranges (with a timeout) + a normal range contrast.
### Phase 2 — the fix
- [ ] Widen entropy (or reject the range).
### Phase 3 — validation
- [ ] Full suite green; distribution still uniform for supported ranges.

## Validation Plan

- Regression: large-range termination test; uniformity sanity for a normal range.
- Doc sync: if rejecting, document the range limit in the crypto man page.

## Summary

The rejection-sampling bound collapses to zero past 2^62, hanging on valid
arguments; widening entropy (or rejecting the range) fixes it. Small, well-scoped.
