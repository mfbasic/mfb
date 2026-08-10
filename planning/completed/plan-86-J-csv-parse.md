# plan-86-J — native csv parse

Sub-plan **J** of [plan-86](plan-86-benchmark-perf.md). Open (borderline — low priority).

**Covers (1 P1, borderline):** parse csv (5.01, min 4.98 ⇒ linear).

## Root cause
`__csv_parse` (`csv_package.mfb:34`) is an interpreted per-scalar state machine over a `List OF Integer`:
per-scalar `collections::get`, `separatorLength` called **twice per row**, list appends, and per-cp
`out & fromCodepoint` in `__csv_decodeRange`. A3-csv (no intermediate list) already landed; the residual is
the interpreted scan (~15× C). Not arena.

## Fixes
- [x] ~~**J1** — a native byte-level csv-parse builtin (or hoist `separatorLength` to once/row and batch the
  decode). Borderline row (5.01 ≈ the 5 ms complete bar) — **pursue only if it regresses.**~~ — **moot: the
  plan's own condition ("pursue only if it regresses") is measured NOT met.** Re-measured this session
  (release, `--run 10`, box-local, after the A/E collections changes that csv::parse leans on): `parse csv`
  **min 4.82 / med ~5.0 ms** — at the "≤ 5 ms = complete" override boundary and NOT regressed from the 5.01
  baseline (min actually improved 4.98 → 4.82). A native byte-level csv-parse builtin is a large new native
  lowering (a whole state-machine), unjustified for a row already at the complete bar. Correctness of the
  existing interpreted path is intact (csv acceptance checksums + `parse csv = 6003000` unchanged, full
  artifact-gate green). Reopen only if a future change regresses the row past the bar.

## Acceptance
`parse csv` checksum 6003000 unchanged + csv acceptance fixtures.
