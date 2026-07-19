# bug-304: `json::stringify` silently drops precision for Floats needing more than 9 fractional digits (not round-trip-safe)

Last updated: 2026-07-17
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Fixed
Regression Test: tests/rt-behavior/json/json-number-roundtrip-rt

`__json_stringifyNumber` renders the integer form (`toString(value, 0)`) and
round-trip-checks only that path; otherwise it falls back to a fixed
`toString(value, toByte(9))` (9 decimal places) and trims, with no round-trip check
on the fractional path. Nine decimals cannot round-trip a general binary64, so
significant digits are silently dropped — round-tripping numeric JSON is lossy.

The single correct behavior a fix produces: `json::stringify` emits the shortest
decimal that parses back to the exact same Float (up to 17 significant digits), so
`json::parse` ∘ `json::stringify` is the identity on numbers.

References:

- Found during goal-06 review of `src/builtins/json_package.mfb`.

## Failing Reproduction

```
' json::stringify(json::parse("3.141592653589793"))  -> "3.141592654"
' json::stringify(json::parse("0.12345678901234"))   -> "0.123456789"
```

- Observed: fractional digits past the 9th are dropped.
- Expected: the value round-trips (e.g. `3.141592653589793`).

## Root Cause

`src/builtins/json_package.mfb:140` (`__json_stringifyNumber`): the fractional
fallback uses a fixed 9-place `toString` with no round-trip verification, unlike the
integer path.

## Goal

- Emit the shortest decimal that round-trips: grow precision until
  `toFloat(rendered) = value`, capped at 17 significant digits.

### Non-goals (must NOT change)

- Integer rendering (already round-trips).
- The JSON output format for values that already render exactly.

## Blast Radius

- `__json_stringifyNumber` — fixed here.
- Any consumer relying on the current 9-digit truncation — none intended; the
  contract is round-trip fidelity.

## Fix Design

Iteratively increase the fractional precision (or significant digits) until
`toFloat(rendered) == value`, capping at 17 sig-figs, then trim trailing zeros.
Rejected alternative: always emitting 17 digits — produces ugly non-minimal output.

## Phases

### Phase 1 — failing test
- [ ] Round-trip tests for the repro values (fail today).
### Phase 2 — the fix
- [ ] Shortest-round-tripping decimal.
### Phase 3 — validation
- [ ] Full suite green; existing exact renders unchanged.

## Validation Plan

- Regression: round-trip tests across several precision-sensitive Floats.
- Doc sync: none.

## Summary

The fractional fallback caps at 9 digits with no round-trip check, losing precision.
Growing precision to the shortest round-tripping decimal fixes it; low risk.

## Resolution

`__json_stringifyNumber` now searches for the **shortest** precision whose rendering
parses back to the same Float, keeping the integer form as the first candidate so a
whole number still renders `100` rather than `100.0`. If nothing round-trips it
fails, because emitting a silently-lossy number is precisely the defect being fixed.

### The obvious fix does not work, and finding out why produced a second bug

The natural first attempt was to use the plain one-argument `toString(value)`,
assuming it is the shortest-round-trip formatter (the in-tree
`_mfb_rt_float_to_string`). Written that way, the new round-trip assertion **failed
immediately** — which is the assertion doing its job.

Investigating showed `toString(Float)`'s precision argument **defaults to 2 by
documented design** (`toString(value AS Float, precision AS Byte = 2)`), so at
runtime `toString(pi)` is `3.14`, `toString(0.1)` is `0.10`, and `toString(1.0/3.0)`
is `0.33`. It is not a shortest-round-trip formatter at all.

That also surfaced a genuine divergence worth its own report: a *constant-folded*
`toString(3.141592653589793)` yields the full `3.141592653589793`, ignoring the
documented default that the runtime honors — so the same value prints differently
depending on whether the compiler could see it. Filed as **bug-358**. bug-304 does
not depend on how that is resolved, because the search here is explicit rather than
relying on any default.

### Verification

Seven cases round-trip exactly, chosen to cover both directions of the risk:
`3.141592653589793`, `0.12345678901234`, `-2.718281828459045`, `0.1` and
`0.000000000000123` are the precision-loss cases; `1.5` and `100` guard the opposite
failure, that the search must return the *shortest* rendering and not pad short
values with trailing digits.

Three JSON `.ir` goldens moved, as they capture the lowered stdlib source. Runtime
behaviour is unchanged and was verified rather than assumed: both `json-behavior`'s
and `func_json_stringify_invalid_runtime`'s `build.log` — each embedding the
program's complete output — are byte-identical.

Full `cargo test` green; artifact gate 0 diffs.
