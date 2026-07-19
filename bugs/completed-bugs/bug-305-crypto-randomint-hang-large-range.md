# bug-305: `crypto::randomInt` hangs (infinite loop) when the requested range exceeds 2^62

Last updated: 2026-07-17
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness / Edge-case (DoS)

Status: Fixed
Regression Test: tests/rt-behavior/crypto/crypto-randomint-wide-range-rt

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

## Resolution

Reproduced first: `crypto::randomInt(0, 2^62 + 1)` printed its preamble and then
hung, killed after 5 s.

The report offered "return a uniformly random value **or** raise a clear error". The
former was taken — the range is legitimate, and refusing it would be a gap in a
general-purpose API rather than a fix.

A wide range now draws 63 bits (`__crypto_rand63`, the 62-bit sibling with the top
byte masked to 7 rather than 6). The rejection loop for that band is simpler than the
narrow one, and for a satisfying reason: for any `range` in `(2^62, 2^63-1]`,
`2 * range > 2^63 >= range`, so `floor(2^63 / range)` is exactly **1**. The rejection
limit therefore *is* `range`, and an accepted draw needs no modulo at all. That also
sidesteps having to name 2^63 — a value an `Integer` cannot hold, which is the same
representability wall that produced the original bug. Acceptance probability is above
1/2, so the loop terminates.

The narrow path (`range <= 2^62`) is untouched, which is why the crypto goldens'
runtime output is unchanged.

### Verification

The fixture brackets the 2^62 boundary on **both** sides — below, at, above, and the
maximum representable range — plus a negative range, an ordinary small range, and a
singleton. Bracketing matters here because the failure mode is a hang, not a wrong
answer: a test that only exercised a wide range would prove termination but not that
the narrow path still works, and one that only exercised narrow ranges would have
passed against the unfixed code.

Six crypto `.ir` goldens moved (they capture the lowered stdlib source). Runtime
behaviour is unchanged and was verified rather than assumed — including
`crypto-kat-valid`, the known-answer tests, whose `build.log` is byte-identical.

Full `cargo test` green; artifact gate 0 diffs; acceptance 1008/1008.
