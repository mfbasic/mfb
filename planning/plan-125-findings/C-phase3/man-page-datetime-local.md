### 1. DST construction policy omitted
UNIT:      man-page:datetime/local
CLAIM:     "Projecting a `datetime::Instant` through this zone with `datetime::inZone` consults that table for the instant being projected, so the result is DST-correct: the same local zone yields one offset for a summer instant and another for a winter instant when the host observes daylight saving time."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/datetime/func_civil.rs:__datetime_civil` resolves civil fields through `__datetime_resolveLocal`; `src/codegen/builtins/datetime/helper_resolve_local.rs:BODY` chooses the earlier offset in an overlap and shifts a gap forward. Scratch probe `/tmp/plan-125-scratch/C-phase3/man-page-datetime-local/dst/src/main.mfb`, run with `TZ=America/New_York`, printed:
`2026-03-08 03:30:00 -04:00`
`2026-11-01 01:30:00 -04:00`
SUGGESTED: Add: "When you construct a `datetime::DateTime` with `datetime::civil` and this zone, a nonexistent spring-forward time shifts forward by the gap, and an ambiguous fall-back time uses the earlier occurrence."