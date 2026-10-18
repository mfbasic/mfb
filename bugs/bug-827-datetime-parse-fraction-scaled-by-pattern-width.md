# bug-827: datetime::parse `f` token scales by run length, not digits read — ".12" under "fff" parses as 12 ms, ".1" under 9×f as 1 ns

Last updated: 2026-10-05
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: tests/rt-behavior/datetime/datetime-parse-fraction-scale-rt (proposed)

## Cause (coordinator, bug-staging-24)

- Rule: a `datetime::parse` `f` run is scaled by the number of digits actually read (owner decision 2026-10-05), as `parseIso` does: `".12"` under `fff` is 120 ms.
- Where: `src/codegen/builtins/datetime/helper_parse_fields.rs:__datetime_parseFields`, the `ch = "f"` arm, scales by the run length; the digit count read (`r.nextPos - vi`) is never used.
- Fix: scale by `10^(9 - digitsRead)` from the count `__datetime_readNum` consumed, the same arithmetic `__datetime_parseIso` uses.

**Owner decision (2026-10-05):** the `f` run is scaled by the number of digits actually read, matching `parseIso`.
`".12"` under `fff` is 120 ms, and `".1"` under nine `f`s is 100 ms. Rejecting with a format
error is not the fix.

`datetime::parse` accepts fewer fractional digits than the `f` run length (the
reader is "up to run length", as the spec says), but then scales the value as if
exactly run-length digits had been read. A fraction with fewer digits than `f`s is
silently mis-scaled by a power of ten: `"10:00:00.12"` against `"HH:mm:ss.fff"`
yields 12 ms rather than 120 ms, and `"10:00:00.1"` against nine `f`s yields
1 ns rather than 100 ms. No error is raised, so the wrong instant goes unnoticed.
`parseIso` handles the same text correctly (it scales by digits read).

The correct behaviour: a fraction of `k` digits (1 ≤ k ≤ run length) means
`digits × 10^(9-k)` nanoseconds — or, if the owner wants "reads run-length digits"
(the man-page wording) to be strict, a short fraction raises `ErrInvalidFormat`.
Either is acceptable; silently scaling by run length is not.

References:

- src/docs/spec/stdlib/02_datetime.md §"Parse grammar": "Numeric tokens read up to a
  token-specific digit cap (… `f` up to its run length). At least one digit is
  required." and "`f` reads its run-length digits then *scales up* to 9-digit
  nanoseconds."
- `mfb man datetime parse`: "fff..fffffffff — fractional second; reads run-length
  digits and scales them to nanoseconds (fff = milliseconds, fffffffff = nanoseconds)"

## Failing Reproduction

```
IMPORT io
IMPORT datetime

FUNC main() AS Nothing
  io::print(datetime::toIso(datetime::parse("10:00:00.12", "HH:mm:ss.fff"), 9))
  io::print(datetime::toIso(datetime::parse("10:00:00.1", "HH:mm:ss.fffffffff"), 9))
  io::print(datetime::toIso(datetime::parse("10:00:00.120", "HH:mm:ss.fff"), 9))
  io::print(datetime::toIso(datetime::parseIso("1970-01-01T10:00:00.12Z"), 9))
END FUNC
```
`/tmp/hardening/mfb build /tmp/hardening/B02/min1/p && /tmp/hardening/B02/min1/p/build/p.out`

- Observed:
```
1970-01-01T10:00:00.012000000Z
1970-01-01T10:00:00.000000001Z
1970-01-01T10:00:00.120000000Z
1970-01-01T10:00:00.120000000Z
```
- Expected: lines 1 and 2 are `…10:00:00.120000000Z` and `…10:00:00.100000000Z`
  (or both raise `77050003`).

Contrast: a full-width fraction (line 3) and `parseIso` (line 4) are correct. A
10-`f` run is also wrong in the other direction: `parse("10:00:00.0123456789",
"HH:mm:ss.ffffffffff")` returns `.123456789` (no scale-down; the 10-digit value
lands in nanos as-is), and `"…1234567891"` overflows nanos and is rejected.

## Root Cause

`src/codegen/builtins/datetime/helper_parse_fields.rs:__datetime_parseFields`, the
`ch = "f"` arm: `__datetime_readNum(value, vi, runLen)` returns only `[value,
nextPos]` (`helper_read_num.rs:__datetime_readNum` reads 1..maxDigits digits), and
the arm then multiplies by 10 from `k = runLen` up to 9. The digit count actually
read (`r.nextPos - vi`) is never used. `func_parse_iso.rs:__datetime_parseIso`
scales by the count it read, which is why it is immune.

## Goal

- `parse(".12", ".fff")` → 120 000 000 ns; `parse(".1", ".fffffffff")` → 100 000 000 ns
  (or both `ErrInvalidFormat`, per owner decision); runs longer than 9 scale down
  (or are rejected) consistently with `format`, which emits at most 9 digits.

### Non-goals (must NOT change)

- `format`'s `f` rendering; `parseIso`; full-width fractions parse unchanged.

## Blast Radius

- `__datetime_parseFields` `f` arm — fixed by this bug.
- `__datetime_parseIso` fraction reader — unaffected (scales by digits read; verified, line 4).
