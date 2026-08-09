# plan-86-J — native csv parse

Sub-plan **J** of [plan-86](plan-86-benchmark-perf.md). Open (borderline — low priority).

**Covers (1 P1, borderline):** parse csv (5.01, min 4.98 ⇒ linear).

## Root cause
`__csv_parse` (`csv_package.mfb:34`) is an interpreted per-scalar state machine over a `List OF Integer`:
per-scalar `collections::get`, `separatorLength` called **twice per row**, list appends, and per-cp
`out & fromCodepoint` in `__csv_decodeRange`. A3-csv (no intermediate list) already landed; the residual is
the interpreted scan (~15× C). Not arena.

## Fixes
- [ ] **J1** — a native byte-level csv-parse builtin (or hoist `separatorLength` to once/row and batch the
  decode). Borderline row (5.01 ≈ the 5 ms complete bar) — **pursue only if it regresses.**

## Acceptance
`parse csv` checksum 6003000 unchanged + csv acceptance fixtures.
