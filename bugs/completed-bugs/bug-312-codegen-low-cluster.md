# bug-312: codegen LOW cluster (Fixed toString truncation vs. rounding, toScalar UTF-8 trust, Float^huge-exponent error code)

Last updated: 2026-07-17
Effort: small (<1h across items)
Severity: LOW
Class: Correctness / Memory-safety (latent) / Footgun

Status: Fixed
Regression Test: per-item

LOW-severity codegen residuals found during goal-06. Distinct root causes, one
document per the repo's low-cluster convention.

References:

- Found during goal-06 review of `src/target/shared/code/builder_strings.rs`,
  `builder_conversions.rs`, `builder_numeric.rs`.

## Items

### K1 — `toString(Fixed, precision)` truncates its fraction while Float and Money round
- `src/target/shared/code/builder_strings.rs:1442-1455` (`emit_fixed_to_string_value`,
  `fraction_loop`).
- The Fixed fraction loop emits `(frac*10)>>32` per digit with no final-digit
  rounding — pure truncation — while `emit_float_to_string_value` rounds ties-to-even
  and `emit_money_to_string_value` pre-rounds half-away. So the three fixed-precision
  decimal overloads the man page groups together disagree.
- Repro: `toString(toFixed("0.666"), 2b)` → `"0.66"` (rounding → `"0.67"`);
  `toString(toFixed("0.99"), 1b)` → `"0.9"` (→ `"1.0"`); `toString(0.666, 2b)` (Float)
  → `"0.67"`.
- Fix: apply the same rounding (half-away or ties-to-even to match Float) to the Fixed
  fraction before rendering `precision` digits; or, if truncation is intended, state
  it in the `toString(Fixed)` man/spec.
- Prior-work: new (bug-295 is x86 `math::round` double-rounding, unrelated).

### K2 — `toScalar(String)` decoder trusts UTF-8 well-formedness (no continuation-byte validation)
- `src/target/shared/code/builder_conversions.rs:576-688`
  (`emit_string_to_scalar_value`).
- Unlike `emit_utf8_decode_next` (hardened per audit-unicode #3) and the `padChar`
  scalar check (audit-unicode #7), this decoder classifies the lead byte and
  unconditionally reads fixed offsets `string+9/+10/+11` for 2/3/4-byte leads before
  the "exactly one scalar" length check. Safe only because the `String` UTF-8
  invariant guarantees ≥k data bytes for a k-byte lead; if any path ever produced a
  non-UTF-8 `String` (truncated multibyte lead), the fixed-offset reads run past the
  allocation. Latent (Strings are guaranteed valid UTF-8 today).
- Fix: validate each continuation byte (`(b & 0xC0) == 0x80`) before consuming, for
  defense-in-depth parity with the sibling decoders.
- Prior-work: new (the sibling decoders were hardened in audit-unicode; this one was
  left trusting).

### K3 — `Float ^ wholeExponent` raises `ErrFloatDomain` for a whole exponent ≥ 2^63
- `src/target/shared/code/builder_numeric.rs:1793-1809` (`emit_float_pow`).
- The whole-exponent test converts the f64 exponent with a saturating `fcvtzs`,
  round-trips back to f64, and compares for equality. An exponent whose magnitude
  exceeds i64::MAX (e.g. `1.0e19`) saturates to i64::MAX, whose f64 round-trip is
  unequal, so the code takes `emit_float_domain_return()` — reporting `ErrFloatDomain`
  for an exponent that is in fact whole. For bases where the true result is finite
  (`1.0 ^ 1.0e19 = 1.0`, `0.5 ^ 1.0e19 = 0.0`) the value is lost; for base>1 the
  correct `ErrFloatOverflow` is mis-coded as `ErrFloatDomain`. POSSIBLE (marginal
  input range; reasoned, not run).
- Fix: treat a saturated conversion (|exponent| ≥ 2^63) whose round-trip is unequal as
  "whole" (any f64 ≥ 2^52 is already an integer), or special-case `|exponent| >= 2^52`
  to skip the fractional-exponent rejection.
- Prior-work: new (bug-135 introduced the kernel; the saturation edge unaddressed).

## Goal

- Fixed toString rounds consistently with Float/Money (or documents truncation);
  toScalar validates continuation bytes; Float^huge-whole-exponent returns the correct
  value/error code.

### Non-goals (must NOT change)

- Correct outputs for in-range inputs.
- The float formatter / Money rounding (already correct).

## Blast Radius

Each item is a single cited codegen site; land per item.

## Fix Design / Phases

- [ ] Phase 1: rt-behavior tests for K1 (rounding) and K3 (huge exponent); a
      malformed-String unit path is not constructible from source for K2 (defense-in-
      depth, assertion-level).
- [ ] Phase 2: apply per-item fixes.
- [ ] Phase 3: artifact gate + rt-behavior green; no golden drift beyond intended.

## Validation Plan

- Regression: K1 rounding tests; K3 exponent tests.
- Doc sync: `toString(Fixed)` man/spec if truncation is kept (K1).

## Summary

Three codegen residuals: a Fixed/Float rounding inconsistency, a defense-in-depth
UTF-8 validation gap, and a marginal Float-pow error-code edge. Each is a small
localized fix.

## Resolution

All three landed. K1 and K3 were reproduced first and both matched the report
exactly.

**K1 — Fixed now rounds, matching the sibling overloads.** The man page describes all
three fixed-precision overloads identically ("with `precision` digits after the
decimal point") and says nothing about truncation, so behaving differently was the
defect; rounding was chosen over documenting truncation for that reason.

The rounding is applied to the raw **magnitude**, before it is split into integer and
fraction parts — copied from how the Money renderer solves the same problem. That
placement is the whole trick: the digits are emitted left to right, so a carry out of
the fraction (`0.99` at one place → `1.0`) cannot be applied after the integer digits
are already in the buffer. Rounding the magnitude first lets the existing split see
the carried value.

One subtlety cost a second iteration. The half-ULP is `2^31 / 10^p`, and a truncating
divide makes the bias strictly *less* than half — so a value sitting exactly on the
boundary rounded **down**: the exactly-representable `0.125` at two places gave
`0.12`. Using a ceiling divide puts the boundary on the away-from-zero side. Caught
because the test battery included an exact binary half, not only the reported cases.

Precision ≥ 10 skips rounding entirely: the Q32.32 fraction carries ~9.6 decimal
digits, so beyond that the remaining places are exact zeros — which also keeps `10^p`
inside 64 bits.

**K2 — the length check now precedes the reads.** The report asked for
continuation-byte validation, which was added, but the hazard it *describes* is the
fixed-offset reads at `string+9/10/11` running past the allocation — and validating a
byte cannot prevent the read that fetched it. So each multi-byte arm first checks
`length >= nbytes` and only then reads. Both guards are present: the length check
prevents the over-read, the continuation check rejects malformed input.

**K3 — a saturated conversion is no longer read as "fractional".** Every finite f64 at
or above 2^52 is already an integer, so such an exponent short-circuits to the whole
path without a round-trip. The bound is `biased ∈ [1075, 2046]`: the upper end
deliberately excludes 2047, since Inf/NaN is not a whole number and must keep falling
through to the domain rejection. Results: `1.0 ^ 1e19` returns `1.0` and
`0.5 ^ 1e19` returns `0.0` (both previously lost), and `2.0 ^ 1e19` now reports
`77050015` (ErrFloatOverflow) rather than the mis-coded `77050012`.

The fixture covers all three together, and deliberately includes the cases a fix
could break rather than only the reported failures: for K1 the exact halves, the
negatives, the carry, and a value needing no rounding; for K3 an ordinary whole
exponent plus a fractional and a negative one, both of which must still be domain
errors; for K2 all four UTF-8 lengths plus non-single-scalar input.

### Two goldens recorded the truncation bug and were regenerated — with proof

K1 changed real output, and acceptance flagged `math_package_valid` and
`bug128_fixed_atan2_overflow_min`. Those goldens were regenerated, but only after
establishing the new values are the correct ones rather than merely the new ones:

- `piFixed` at 6 places moved `3.141592` → `3.141593`, `eFixed` `2.718281` →
  `2.718282`, `twoOverPiFixed` `0.636619` → `0.636620`. Each is the correctly
  *rounded* value; the old ones were the truncations.
- The decisive evidence is in the test itself: `math_package_valid` prints each
  constant in **both** Float and Fixed form on adjacent lines, and the Float lines
  already read `3.141593` / `2.718282` / `0.636620`. The goldens recorded the two
  forms disagreeing — which is exactly the defect — and now they agree.
- `atan2` moved `0.78` → `0.79` and `-2.35` → `-2.36`. π/4 is 0.7853…, so 0.79 is
  correct to two places; −3π/4 is −2.3562…, so −2.36 is.

Same situation as bug-309: a golden defending a live defect. The difference from an
unjustified re-baseline is that each new value was checked against the mathematics
and against the test's own Float sibling, not accepted because it was what the code
now produced.

Full `cargo test` green; artifact gate 0 diffs; acceptance 1013/1013.
