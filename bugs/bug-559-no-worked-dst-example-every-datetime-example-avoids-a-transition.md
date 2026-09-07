# bug-559: the DST behaviour `civil` and `addDays` document is never demonstrated — every published example picks a zone or date where it cannot happen

Last updated: 2026-09-06
Effort: small (<1h) for the examples; medium if a portable oracle is required (see Open Decisions)
Severity: LOW
Class: Documentation (worked-example coverage gap)

Status: Open
Regression Test: `scripts/man-run-examples.sh datetime --run` (once bug-472 makes
man examples executable), plus a new `tests/rt-behavior/datetime/` fixture pinning
the gap/overlap policy under a fixed `TZ`.

`datetime`'s daylight-saving contract is documented in prose across several
members, and the implementation honours it. What is missing is a single worked
example that *shows* it. Every example the package ships is written against a
zone or a date where a DST transition provably cannot occur, so a reader who
wants to know what happens on the two days a year that matter has only prose.

The prose makes strong, specific promises. `datetime::addDays`
(`src/codegen/builtins/datetime/func_add_days.rs`, `const DESC`):

> Because the result is re-resolved through `dt`'s zone, `addDays` is
> daylight-saving aware: the wall-clock time of day is preserved and the UTC
> offset is recomputed for the new date, so crossing a DST transition shifts the
> underlying instant by the appropriate 23-, 24-, or 25-hour day rather than a
> fixed `86_400` seconds.

`datetime::civil` (`func_civil.rs`, `const DESC`) goes further and specifies the
tie-break policy for both anomalies:

> If they differ, a spring-forward gap (the named local time is skipped) shifts
> forward onto the post-transition offset, and a fall-back overlap (the named
> local time occurs twice) takes the earlier, pre-transition offset.

Now the examples. `addDays`' `const EX` has two blocks and both open with:

```
LET dt AS datetime::DateTime = datetime::toUtc(datetime::now())
```

UTC has no transitions, so neither example can exercise the paragraph above it.
This is not merely unlikely — it is structurally impossible, and the body says
so (`grep -n "zone.kind <> 2" src/codegen/builtins/datetime/func_add_days.rs`
→ line 60):

```
IF dt.zone.kind <> 2 THEN
  RETURN DateTime[__datetime_civilFromDays(newDays), dt.time, dt.zone, dt.offset]
END IF
RETURN __datetime_civil(__datetime_civilFromDays(newDays), dt.time, dt.zone)
```

Only a system zone (`kind = 2`, i.e. `datetime::local()`) reaches
`__datetime_civil` and therefore `__datetime_resolveLocal`. A `toUtc` value takes
the fast path on the line above. **Every shipped `addDays` example runs the
branch that skips DST resolution entirely.**

`civil`'s `const EX` is the same shape from the other direction: its first block
does use `datetime::local()`, but with `datetime::date(2026, 6, 26)` — late June,
nowhere near a transition in either hemisphere — and its second block uses
`datetime::utc()`.

The single behavioural outcome a fix produces: a reader of `mfb man datetime
civil` or `mfb man datetime addDays` can see, in code, what a spring-forward gap
and a fall-back overlap return, without deriving it from the policy paragraph.

References:

- `src/codegen/builtins/datetime/func_add_days.rs` — `const DESC` (the 23/24/25-hour
  promise), `const EX` (two UTC examples), `const BODY` line 60 (the `kind <> 2`
  fast path that makes the promise unreachable from those examples).
- `src/codegen/builtins/datetime/func_civil.rs` — `const DESC` (the gap/overlap
  policy), `const EX` (June + UTC).
- `src/codegen/builtins/datetime/helper_resolve_local.rs` — `__datetime_resolveLocal`,
  the ±86400 probe that implements the policy. This is the code a worked example
  would be demonstrating.
- `src/codegen/builtins/datetime/func_local_offset.rs:216-220` — states that the
  host zone comes from "the `TZ` environment variable or the system zone
  setting", which is the hook that makes a deterministic example possible.
- `.ai/man-content.md` — the content standard; an EXAMPLE is unchecked text.
- bug-472 (Open) — man examples are never compiled, so a new example is prose
  until that lands.
- bug-520 (Open) — no named zones; `Local` is not portable. Directly constrains
  the fix, see Open Decisions.

## Failing Reproduction

There is no runtime misbehaviour to reproduce; the defect is that a documented
behaviour has no demonstration. Two commands establish it:

1. Render the pages and read the examples:

   ```
   mfb man datetime addDays
   mfb man datetime civil
   ```

   Neither example names a transition date, and `addDays`' two examples are both
   UTC.

2. Show that the shipped examples cannot reach the DST code path:

   ```
   grep -n "zone.kind <> 2" src/codegen/builtins/datetime/func_add_days.rs
   ```

   → line 60. `toUtc(...)` yields `kind <> 2`, which returns on line 61 without
   calling `__datetime_civil`.

A positive control for the behaviour itself (this is what an example should
show), run with `TZ=America/New_York`:

- Spring forward, 2026-03-08: `civil(date(2026,3,8), time(2,30), local())` names a
  local time that does not exist. Per the documented policy it resolves onto the
  post-transition offset (-04:00), i.e. 03:30 EDT.
- Fall back, 2026-11-01: `civil(date(2026,11,1), time(1,30), local())` names a local
  time that occurs twice. Per the policy it takes the earlier, pre-transition
  offset (-04:00), i.e. the first 01:30 EDT.
- `addDays` across the boundary: `addDays(civil(date(2026,3,7), time(12,0), local()), 1)`
  must return 2026-03-08 12:00 local — the same wall clock — while
  `resolve()` on the two values differs by 82,800 seconds (23 hours), not 86,400.

**These three expected values are derived from the documented policy, not
measured.** Confirming them against a real `TZ=America/New_York` run is Phase 1;
if any disagrees, the bug is larger than a doc gap and the prose is wrong.

## Root Cause

Nobody wrote one. The examples were authored to show the *shape* of each call
(build a `DateTime`, shift it, recover an `Instant`), and `toUtc(now())` /
`local()` in June are the shortest ways to get a value of the right type. There
is no gate that notices an example never reaches the branch its own paragraph
describes: examples are `&'static str` the compiler never reads, and bug-472
records that they are not even compiled, let alone coverage-checked.

## Goal

`mfb man datetime civil` and `mfb man datetime addDays` each carry one worked
example that crosses a real DST transition, states the `TZ` it assumes, and shows
the concrete before/after values for both the gap and the overlap case.

## Blast Radius

- `src/codegen/builtins/datetime/func_civil.rs` (`const EX`)
- `src/codegen/builtins/datetime/func_add_days.rs` (`const EX`)
- Possibly `func_resolve.rs` / `func_in_zone.rs` if the example is better placed
  once and cross-referenced.
- Prose-field edits shift embedded source lines, so importer `.ir` goldens drift
  (see the "registry description drifts ir goldens" and "editing package.mfb
  drifts many goldens" notes). Regenerate, do not revert.
- No behaviour change. No `.ncode` change expected from prose alone — if one
  appears, that is a bug-hunt trigger, not a re-baseline.

## Fix Design

Add one example to each page. Both must:

1. Name the assumed zone explicitly in the surrounding prose ("with `TZ=America/New_York`"),
   because `datetime::local()` is host-dependent and bug-520 means there is no
   way to name a zone in the language yet.
2. Show the gap and the overlap as separate, commented cases with their expected
   results inline.
3. For `addDays`, use `datetime::local()` rather than `toUtc(now())`, so the
   example actually reaches `__datetime_civil` — and say why the wall clock is
   preserved while the instant moves by 23 or 25 hours.

## Phases

1. **Verify the policy against a real host.** Run the three positive-control
   cases above under `TZ=America/New_York` and record the measured values. If they
   match the documented policy, continue; if not, stop — this is a correctness bug,
   not a doc bug, and needs re-filing.
2. **Write the `civil` example** (gap + overlap), using the Phase 1 measurements.
3. **Write the `addDays` example** (23-hour and 25-hour days).
4. **Regenerate drifted goldens** and prove the delta is only the prose shift.

## Validation Plan

- `mfb man datetime civil` and `mfb man datetime addDays` render, and the new
  examples compile and run: `scripts/man-run-examples.sh datetime --run`.
- `scripts/man-census.sh --memory-scope` reports 0 unclassified hits (the new
  prose must not introduce banned memory vocabulary).
- A new `tests/rt-behavior/datetime/` fixture pins the gap/overlap results under a
  fixed `TZ`, so the example's claims are backed by an executable gate rather than
  by prose. Four goldens required (`build.log`, `.ast`, `.ir`, `.run`).
- Full `cargo test --no-fail-fast`; acceptance via `test-accept.sh`; artifact gate.

## Open Decisions

1. **Is a `TZ`-dependent example acceptable?** It is not portable — that is
   bug-520's whole subject. Options: (a) write the example against
   `TZ=America/New_York` and say so in the prose; (b) wait for bug-520 to add named
   zones and write it portably; (c) both — ship (a) now and revisit when 520 lands.
   Recommendation: **(a)**, because 520 is Effort: huge and this page is misleading
   today. Do not block on 520.
2. **Where does the worked example live?** One example duplicated on two pages, or
   one on `civil` with `addDays` cross-referencing it. The renderer derives
   See-also itself, so a cross-reference is cheap.
3. **Not in scope, recorded so it is not lost:** the other half of the original
   review note asked for a critique of the `datetime` format mini-language versus
   `strftime` / Temporal. That is an API-design review, not a defect, and is
   deliberately excluded here. File separately if wanted.

## Summary

`datetime` documents its daylight-saving behaviour carefully and then never shows
it. `addDays`' two examples are both UTC, and its body returns on the line above
the DST-aware path for any non-system zone; `civil`'s local-zone example is dated
late June. The fix is two worked examples plus an `rt-behavior` fixture so the
claims are executable rather than asserted. Constrained by bug-520 (no named
zones, so the example must state its `TZ`) and by bug-472 (examples are not
compiled today).
