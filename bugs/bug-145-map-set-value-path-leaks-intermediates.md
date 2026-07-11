# bug-145 — map `collections::set` value path leaks both intermediates

**Status:** OPEN. Filed 2026-07-11 (goal-02 review, G8).
**Severity:** MED — one whole-map copy + one singleton leaked per non-in-place
map `set`.
**Class:** memory-safety (leak).

## Finding

`src/target/shared/code/builder_collection_mutate.rs:346-413`
(`lower_collection_set`, map branch). The list branch frees its `singleton` and
`removed` intermediates (:339-343); the map branch builds `without` (a full map
copy minus the key) and a `singleton` map, concats them, and frees neither.
Every non-in-place map `set` (e.g. `LET m2 = collections::set(m1, k, v)`, or
when the in-place gate declines) leaks one whole-map-sized block plus a
singleton per call.

## Trigger

```
FOR i = 0 TO N
  LET mi AS Map OF ... = collections::set(m, k(i), v(i))
NEXT
```
→ arena grows by ~2 map copies per iteration until function exit.

## Fix

Free the `without` and `singleton` intermediates after the concat in the map
branch, mirroring the list branch's explicit frees.

## Prior art

bug-01/bug-47 fixed list/grow sites only; audit-1-codegen-memory has no entry.
