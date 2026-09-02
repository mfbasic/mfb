# plan-121-E: The set-algebra rows measure iteration-order luck, not throughput

Last updated: 2026-09-02
Effort: medium (1h–2h)
Depends on: nothing (independent of A–D; scheduled here so the container work lands first)

`isSuperset`/`isSubset`/`isDisjoint` show 265–412× c -O0 and were originally
scoped as "rewrite the interpreted `Body::mfb` bodies natively". **Spike 5
refuted that diagnosis** (§2). The bodies are already built on native primitives,
a hand-written equivalent costs the same as the builtin, and `contains` is a hash
probe, not a scan. The gap is that **C and mfb do different amounts of scanning
for the same answer**, because both early-exit and they iterate in different
orders.

Behavioral outcome: the set-algebra rows measure comparable work in all three
languages, and the ranking reports mfb's real standing on them — whatever that
turns out to be.

References: `benchmark/README.md` §"Work equivalence" (the invariant this row
satisfies in letter but not in spirit); `.ai/collections.md` §"HOF rewrite
economics"; `benchmark/RANKING.md`.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| Baseline logs present | `ls benchmark/baseline/*.log` → 6 | MET |
| Suite green at HEAD | `cargo test --no-fail-fast` | UNMEASURED — run first |

## 1. Goal

- The set-algebra benchmark rows do comparable scanning work in mfb, C and
  Python, with matching checksums, so their grade reflects implementation
  quality rather than which element happens to fail first.
- After re-running `./benchmark/run.sh 10`, the nine rows carry a grade that is
  *justified* — and if that grade is still poor, a follow-up plan is filed with
  the real root cause.

### Non-goals

- **No weakening of the benchmark to flatter mfb.** AGENTS.md's four-question
  gate applies to the row being changed: the current row must be shown wrong
  before it is touched, and the change must make the work *more* comparable, not
  less. The checksum cross-validation must still pass.
- **No language or compiler change in this sub-plan.** If Phase 1 shows mfb is
  genuinely slow on equal work, that becomes a new plan, not scope absorbed here.
- **No change to `contains`, `Set` layout, or the hash index.**

## 2. Current State

`func_is_superset.rs:53` holds the body:

```
FUNC __collections_isSuperset OF T(a AS Set OF T, b AS Set OF T) AS Boolean
  FOR EACH x IN b
    IF NOT collections::contains(a, x) THEN
      RETURN FALSE
```

The C peer (`benchmark/c/setmatrix.c:105`) is
`for (s2 = 0; s2 < SCAP && sup; s2++) if (used[s2] && !iset_has(b, k[s2])) sup = 0;`
— it walks the **hash slot array** and early-exits on `&& sup`. mfb walks `b` in
**entry order** and early-exits on `RETURN FALSE`. Both stop at the first
counterexample; they just meet it at different points.

### Measured populations

| What | Count | Command |
|---|---|---|
| Set-algebra rows C-or-worse | 9 | `./benchmark/rank.py --csv`, rows `isSuperset`/`isSubset`/`isDisjoint` |
| Worst | 412× (`set (Record-Fixed) isDisjoint`) | same |
| …that lose to CPython | 6 | `awk -F, '$4=="RED"'` over that set |
| Checksum for every such row, every target | `0` | `grep -h "isSuperset\|isDisjoint\|isSubset" benchmark/baseline/*.sums \| sort \| uniq -c` → all `= 0` |

A checksum of `0` means **the predicate is FALSE on every call**, so every
target's inner loop is an early-exit search for one counterexample.

### Verified properties

- **VERIFIED — a native rewrite would gain nothing.** Spike 5 H2 runs a
  hand-written `FOR EACH x IN b / IF NOT contains(a,x)` loop against
  `collections::isSuperset` over the same sets, 200 calls: at N=1600 the hand
  loop takes 1936 µs and the builtin 1915 µs; at N=400, 596 µs versus 451 µs.
  The interpreted-body dispatch is not the cost. This is exactly the case
  `.ai/collections.md` warns about — a `.mfb` body already built on efficient
  native primitives.
- **VERIFIED — `contains` is a hash probe, not a linear scan.** Spike 5 H1, 4000
  probes at constant count: 60, 64, 71, 81 ns/probe at N = 100, 400, 1600, 6400.
  Near-flat across a 64× size range.
- **VERIFIED — the predicate is always FALSE**, so both languages early-exit
  (checksums above).
- **UNVERIFIED — how many elements each language actually examines per call.**
  This is the crux and Phase 1 measures it directly by counting probes.

## 3. Design Overview

Phase 1 counts probes per call in each language. Then one of two things is true,
and the evidence decides which:

- **(a) The work differs by orders of magnitude.** The row is measuring iteration
  order. Fix the benchmark so both languages must examine a comparable number of
  elements — the cleanest form is to make the predicate **TRUE**, which forces a
  full scan everywhere and removes early-exit luck entirely. Re-run and re-rank.
- **(b) The work is comparable and mfb is genuinely slower per probe.** Then
  `contains` at ~60–81 ns is the target (a C hash probe is single-digit ns), and
  that is a compiler plan, filed separately — not absorbed here.

**Where correctness risk concentrates:** changing a benchmark row changes its
checksum, and the run-end cross-validation is the suite's correctness record.
Any change must keep all six targets in agreement, and the four-question gate in
AGENTS.md must be answered in the commit message before the row is touched.

**Byte-identity is irrelevant here** — this sub-plan changes benchmark sources
and possibly nothing in `src/` at all.

### Rejected alternatives

- **Rewrite the three bodies natively anyway.** Rejected on measurement: spike 5
  H2 shows the win is ~0. `.ai/collections.md` predicted this; the spike confirmed it.
- **Leave the rows as they are and mark them known-bad.** Rejected: they are
  currently 9 of the 149 C-or-worse rows and 6 of them carry a RED flag, which
  actively misdirects future effort. A wrong number is worse than no number.

## Phases

### Phase 1 — Count the probes; decide (a) or (b)

- [ ] Instrument the mfb and C peers to count `contains`/`iset_has` calls per
      `isSuperset` call (a counter printed to stderr alongside the checksum, not
      inside the timed region). Record both counts here.
- [ ] Record the same for `isSubset` and `isDisjoint`.
- [ ] Write the conclusion — (a) or (b) — into this plan with the numbers.

Acceptance: probe counts for all three predicates in both languages recorded in
this document, and the branch chosen on that evidence. No source change lands in
this phase beyond the temporary instrumentation, which is reverted.
Commit: —

### Phase 2a — Make the rows do comparable work (only if Phase 1 says (a))

- [ ] Change the set-algebra rows so the predicate is TRUE, forcing a full scan
      in all three languages. Update `benchmark/mfb/gen_set.py`,
      `benchmark/c/setmatrix.c`, `benchmark/python/setmatrix.py` together, and
      regenerate `benchmark/mfb/src/setops.mfb`.
- [ ] Answer AGENTS.md's four questions in the commit message: when/why the row
      was written, what it protects, who else depends on it, and the proof it is
      wrong (Phase 1's probe counts).
- [ ] Re-run `./benchmark/run.sh 10`; confirm the checksum cross-validation
      passes with all rows still shared across ≥2 targets and 0 mismatched.
- [ ] Refresh `benchmark/baseline/` and its README provenance block.

Acceptance: `./benchmark/run.sh 10` reports 0 mismatched checksums; the nine rows
carry a re-measured grade; `benchmark/RANKING.md`'s headline counts are updated
to the new baseline.
Commit: —

### Phase 2b — File the real root cause (only if Phase 1 says (b))

- [ ] Write `planning/plan-NN-set-contains-probe-cost.md` targeting the ~60–81 ns
      probe, with spike 5 H1 as its starting evidence.
- [ ] Record in this plan that E closes with a referral, not a fix.

Acceptance: the follow-up plan exists and this one is archived with its finding.
Commit: —

### Phase 3 — Record the caveat in the ranking system

Whatever Phase 1 concluded, the ranking must not repeat this mistake silently.

- [ ] Add a short subsection to `benchmark/RANKING.md`: an early-exit predicate
      row can satisfy "same observable work" (same answer) while doing wildly
      different amounts of scanning, so a large ratio on such a row must be
      probe-counted before it is believed.
- [ ] Note in `benchmark/README.md` §"Work equivalence" that answer-equality is
      not work-equality for early-exit predicates.

Acceptance: both documents state the caveat; `./benchmark/rank.py` output is
unchanged (this is a documentation phase).
Commit: —

## Validation Plan

- **Tests:** none in `src/` unless Phase 2b applies. The benchmark's own
  checksum cross-validation is the correctness gate for a row change.
- **Coverage check:** n/a — no compiler code changes in the (a) branch.
- **Runtime proof:** `./benchmark/run.sh 10` with 0 mismatched checksums, and the
  nine rows re-graded by `./benchmark/rank.py`.
- **Doc sync:** `benchmark/RANKING.md`, `benchmark/README.md`, and
  `benchmark/baseline/README.md` provenance if the baseline is refreshed.
- **Acceptance:** `cargo test --no-fail-fast` (unchanged, as a regression check),
  plus the benchmark run above.

## Open Decisions

- **Make the predicate TRUE, or match the iteration orders?** Recommend TRUE: it
  removes early-exit luck entirely and is a one-line change in each language,
  whereas matching iteration order across three different set implementations is
  not achievable. (§3)

## Corrections

- **The original scoping of this sub-plan was wrong.** It was written as "rewrite
  the interpreted set-algebra bodies natively", on the reasoning that
  `Body::mfb` implies interpreted overhead. Spike 5 H2 measured the hand-written
  equivalent at 1936 µs against the builtin's 1915 µs at N=1600 — no overhead to
  remove. The sub-plan was rewritten before any code was touched. The general
  lesson is already in `.ai/collections.md`: check whether the `.mfb` body does a
  per-element whole-container copy before assuming a native rewrite pays.

## Summary

This sub-plan may end up changing no compiler code at all. Its value is deleting
nine misleading rows' worth of misdirection — 6 of them currently flagged RED —
and hardening the ranking system against a class of false signal it cannot
currently detect. The real risk is touching a benchmark row at all; the
four-question gate and the checksum cross-validation are what contain it.
