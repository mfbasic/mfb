# bug-91 — Fixed literals with ≥39 fractional digits rejected instead of rounded

**Status:** OPEN. Filed 2026-07-10 (goal-02 review, G1).
**Severity:** LOW — edge-case compile error where rounding is documented.
**Class:** footgun.

## Finding

`src/numeric.rs:157-174` — `fixed_raw_from_decimal` accumulates the fractional
part as a full i128 integer (`fractional_value`) with a matching power-of-ten
`denominator`, both via `checked_mul(10)`. Once a literal has ≥39 fractional
digits, `denominator` exceeds i128::MAX (~1.7e38) and the function errors with
"Fixed constant … has too many digits".

The documented contract is round-half-up on the fractional part; a 32.32 Fixed
resolves only ~1e-10, so digits past ~10 should round, not reject. Notably
`1e-39F` — which `expand_scientific_notation` expands to 39 fractional digits —
fails to compile even though its value rounds to exactly 0.

## Trigger

```
LET x = 1e-39F
```
or `LET x = 0.<39 digits>F` → compile error instead of the nearest
representable Fixed (0 here).

## Fix sketch

Stop accumulating once past the significant fractional precision (~11 digits
for round-half-up at 2^-32): clamp the accumulation and use the next digit for
the rounding decision, ignoring the rest (they only matter for the half-way
tie, which one extra sticky bit covers).

## Prior art

bug-07 (Fixed min literal) and bug-11 cover different Fixed/exponent cases;
neither covers fractional-digit-count overflow.
