# bug-144 — base-10 `toInt` signed-cutoff escape returns wrapped values instead of ErrOverflow

**Status:** OPEN. Filed 2026-07-11 (goal-02 review, G8). **Reproduced.**
**Severity:** MED — `toInt` returns an arbitrary wrapped value instead of
failing on overflow.
**Class:** correctness.

## Finding

`src/target/shared/code/builder_conversions.rs:231-237`
(`emit_string_to_int_value`, `compare_registers(acc, cutoff)` + signed
`branch_gt`/`branch_eq`). Same defect class as bug-49 but in the
one-arg/base-10 parser, which bug-49 explicitly (and wrongly) declared
unaffected: parsing `-9223372036854775808` legitimately drives `acc` to exactly
2^63 (negative as i64); any further digit then passes the signed guard (`acc <
cutoff` signed) and `acc*10+d` wraps.

## Trigger (reproduced)

`toInt("-92233720368547758080")` returns `0` (should FAIL 77050010
ErrOverflow). Any ≥20-digit negative whose first 19 digits are
9223372036854775808 yields an arbitrary wrapped value.

## Fix

Apply bug-49's fix to the base-10 path: use an unsigned/explicit overflow check
on `acc*10+d` (or compare against the magnitude cutoff with the sign accounted
for) rather than a signed `branch_gt`/`branch_eq` on `acc`.

## Prior art

Sibling of fixed bug-49 (radix form only); base-10 path not covered by it.
