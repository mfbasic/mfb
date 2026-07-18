# bug-304: `json::stringify` silently drops precision for Floats needing more than 9 fractional digits (not round-trip-safe)

Last updated: 2026-07-17
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: tests/ (new) — `json::stringify(json::parse(x))` round-trips a full-precision Float

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
