# plan-125-M: spec iteration 3 — every spec package re-read as a whole after the per-file pass

Last updated: 2026-09-04
Effort: large (3h–1d)
Depends on: plan-125-L (spec iteration 2 complete across all 146 files, its
Phase 4 reconciliation clean, and the citation surface at 0 `MISS-*`).

Iteration 3 of three on the spec surface. The unit is again **a whole
package** — but, exactly as on the man surface, the material is not what
letter I read. Iteration 2 edited 146 files independently, and on a
specification that produces a characteristic set of defects:

- **the same contract now stated twice**, in two topics, by two reviewers who
  each correctly decided it needed stating — a direct violation of
  `.ai/specifications.md`'s single-source-of-truth rule;
- **two topics now stating it differently**, which is worse, because a
  contributor reading either one has no signal that the other exists;
- **reading order broken** — a topic that grew a prerequisite the overview's
  reading prose does not mention;
- **`## See Also` and cross-links pointing at sections that were renamed or
  cut**;
- **citation churn** — new citations added file by file with no view of
  whether the package now cites the same symbol from five places for five
  slightly different claims.

None of those are visible at file granularity. This is the re-integration
pass, and — as on the man surface — **its success test is inverted**: mostly
*seam* findings means iteration 2 did its job; many *fact* findings means it
did not, and that is a recorded result of this letter.

References:

- plan-125-A §3.2, §3.3, §4.2, §4.3, §5 (the spec iteration-3 prompt).
- plan-125-G — the man surface's equivalent letter; its measure-then-repair
  ordering and its seam/fact classification are reused verbatim.
- plan-125-I — this letter's baseline: I's per-package ledgers, its
  contract-coverage enumeration, and its four cross-package consistency
  findings. Every one is re-checked for survival.
- plan-125-J/K/L ledgers and bucket tables — the record of what changed in
  each file.
- `.ai/spec-content.md`; `.ai/specifications.md`.

## Prerequisites

See plan-125-A. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-125-L complete, including its Phase 4 reconciliation | `grep -c '^- \[ \]' planning/plan-125-L-spec-iter2-batch3.md` → `0`; L's Phase 4 records 146 files, 0 unaccounted | — |
| all three iteration-2 letters' ledgers and bucket tables complete | `grep -c '^- \[ \]' planning/plan-125-{J,K,L}-*.md` → `0` each | — |
| citation surface clean | `./scripts/spec-census.sh --citations` → 0 `MISS-*` | — |

## 1. Goal

- **All 11 units** — 12 packages minus `unicode` (its iteration 3 completed in
  plan-125-A's pilot) — re-read as wholes, reviewed by one `codex exec` each
  from the iteration-3 prompt, and repaired.
- **Every single-source-of-truth violation introduced by iteration 2 is
  repaired** — a fact stated in two topics is reduced to one canonical topic
  plus a short summary and a link, per `.ai/specifications.md`.
- **Every contradiction between two topics is resolved by determining which is
  right**, by probe or by reading the code where necessary — never by
  averaging into wording true of neither.
- **plan-125-I's contract-coverage enumeration still holds** after 146 file
  edits — verified entry by entry, not assumed.
- **Reading order holds**: `PACKAGE_ORDER`, every overview's reading prose,
  and every `## See Also` reflect the topics that exist and the order they
  should be read in.
- **Citation hygiene after iteration 2**: the spec-wide citation count is
  recorded before and after; any symbol cited from many places for divergent
  claims is consolidated.
- Every finding classified *seam* or *fact*; every fact finding attributed to
  the iteration-2 letter that missed it.
- `--reconcile` exits 0 over the 11-unit list; `--citations` 0 `MISS-*`;
  `--links` clean; `cargo build` and `cargo test --bin mfb spec` green.
- **The spec surface is final entering letter N** — N certifies, it does not
  repair, so anything found here is fixed here.

### Non-goals (explicit constraints)

- **Not a third accuracy pass.** A factual finding is fixed *and* recorded as
  an iteration-2 miss; it does not license re-verifying every claim.
- No renumbering or reordering of packages; `PACKAGE_ORDER` is corrected only
  if it is wrong about what exists.
- The memory-vocabulary ban does not apply on this surface.
- Compiler behavior is not changed; a code defect goes through `write-bug`.
- No wording churn on prose that reads correctly in the package view.

## 2. Current State

Entering M, every one of the 146 spec files has been read alone and every
claim triaged into plan-125-J's four buckets. Nobody has looked at a *package*
since letter I, before any of those edits.

### Measured populations

| What | Count | Command |
|---|---|---|
| units in this letter | **11** | 12 packages − `unicode` (plan-125-A pilot) |
| files edited by iteration 2 before this letter | to be measured at kickoff | count the distinct files with `confirmed → fixed` findings in the J/K/L ledgers |
| spec-wide citations entering M | to be measured at kickoff | `grep -rhoE '\[\[[^]]+\]\]' src/docs/spec --include='*.md' \| wc -l` — 1,970 at plan-writing, expected to rise (letter L adds citations to `threading` deliberately) |
| I's contract-coverage entries to re-verify | to be measured at kickoff | the enumeration recorded in plan-125-I Phase 2/3 |
| I's cross-package consistency findings to re-verify | 4 dimensions | plan-125-I Phase 4 |
| whole-surface baseline entering M | `--citations` 0 `MISS-*`, `--links` clean | plan-125-L Phase 4 |

### Verified properties

- **UNVERIFIED and central: how much duplication and contradiction iteration 2
  introduced.** The plan asserts it will exist — 146 files edited
  independently against a single-source-of-truth rule is the textbook setup
  for it — but the number is unknown until Phase 1 measures it. It is measured
  before it is repaired, so the answer survives the work.
- **Citation growth is expected, not a defect** — plan-125-L Phase 3
  deliberately adds citations to `threading`, the least-cited package (2 per
  file). Growth is the plan working; *divergent* citation of one symbol for
  incompatible claims is the defect this letter looks for.

## 3. Design Overview

**Phase 1 measures, Phases 2–4 repair** — the same ordering as plan-125-G, and
for the same reason: the duplication and contradiction counts are this plan's
only evidence about whether three iterations on the spec were worth their
cost, and they cannot be recovered after the repairs land.

Per unit: my pass (read the package rendered end to end via
`mfb spec <pkg> --all`; look for seams, not facts) → one `codex exec` from the
iteration-3 prompt → apply.

**Order:** the packages iteration 2 changed most first (most seam risk), then
the rest. `architecture` and `stdlib` are the expected leaders — the largest
and the most-repaired respectively.

**Risk concentration:**
- **Re-litigating iteration 2.** The same pull as letter G, and stronger here
  because spec claims are interesting. Held by the prompt's lens and by the
  seam/fact classification, which makes drift visible on the first unit.
- **Resolving a contradiction by choosing the better-written side.** When two
  topics disagree, one is right about the compiler. The repair is to determine
  which — by probe or by reading the code — and record the deciding evidence.
- **Consolidating a duplicated fact into the wrong canonical topic.**
  `.ai/specifications.md` says each fact has one canonical topic; choosing it
  wrongly moves the fact away from where a contributor will look. The rule:
  the canonical topic is the one that owns the *subject*, and the other keeps
  a short summary plus a link — never a second full body, never nothing.
- **Breaking a citation while consolidating.** Every consolidation re-runs
  `--citations` for the touched packages before the commit lands.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work; `- [~]` for partial; `- [x] ~~text~~ — moot: <evidence>`
> rather than deleting; fill `Commit:` on landing. **Unticked means NOT DONE.**

### Phase 1 — measure the divergence before repairing it

- [ ] Count the files iteration 2 actually changed, per package, from the
      J/K/L ledgers; record the table here.
- [ ] Re-verify **every** entry in plan-125-I's contract-coverage enumeration
      against the current surface; record each as *holds* / *now duplicated in
      N topics* / *now contradicted*.
- [ ] Re-verify plan-125-I's four cross-package consistency findings.
- [ ] Record the spec-wide citation count and the `--citations`/`--links`
      entry state.

Acceptance: four measurement tables in this file, each with the command that
produced it beside it; no repair has landed yet.
Commit: —

### Phase 2 — the packages iteration 2 changed most (top 4 by Phase 1)

- [ ] The 4 packages with the most iteration-2 fixes — one review unit each;
      expected to include `architecture` and `stdlib`.
- [ ] Classify every finding *seam* or *fact*; a fact finding names the
      iteration-2 letter and file that missed it.
- [ ] Repair duplication by choosing the canonical topic and leaving a summary
      plus a link in the other; repair contradiction by determining which side
      is right and recording the deciding evidence.
- [ ] Re-run `--citations` and `--links` for each package before its commit.

Acceptance: 4 units `exit 0`; ledgers recorded with seam/fact classification
and, for every contradiction, the evidence that decided it;
`--citations`/`--links` clean for all four; cargo gates green.
Commit: —

### Phase 3 — the remaining 7 packages

- [ ] The other 7 packages, one review unit each; same classification and same
      repair rules.
- [ ] Verify each package's `## See Also` and its overview's reading prose
      against the topics that now exist.

Acceptance: 7 units `exit 0`; ledgers recorded; every overview's reading prose
and `## See Also` verified against the actual topic list;
`--citations`/`--links` clean; cargo gates green.
Commit: —

### Phase 4 — the cross-package consistency re-run and the iteration-3 result

- [ ] Rebuild the condensed artifact (12 overviews + every `## See Also`) and
      re-run plan-125-I §3.2's four consistency dimensions, to confirm the
      consistency work survived iteration 2.
- [ ] Verify `PACKAGE_ORDER` against the packages that exist and against the
      reading path the overviews describe.
- [ ] **Record the iteration-3 result**: total findings, seam vs fact, the
      per-iteration-2-letter attribution of the fact findings, and the
      duplication/contradiction counts from Phase 1 against what remained
      after repair. State plainly whether iteration 2 performed as designed.
- [ ] `--reconcile` over the full 11-unit list plus the four consistency runs.

Acceptance: 11 units + 4 consistency runs `exit 0`; the seam/fact ratio and
the duplication counts are recorded with attribution; whole-surface sweeps at
target (`--citations` 0 `MISS-*`, `--links` clean, `--render` no leaked
`[[`); `cargo build` and `cargo test --bin mfb spec` green; `--reconcile`
exits 0; the spec surface is declared final for letter N, with the sweeps as
evidence rather than the declaration.
Commit: —

## Validation Plan

- Tests: `cargo build` and `cargo test --bin mfb spec` at the end of every
  phase; `cargo test errorcode` if `diagnostics/02_error-codes.md` is touched.
- Coverage check: `--reconcile` over the 11-unit list; Phase 1's measurement
  tables reconciled against the J/K/L ledgers.
- Runtime proof: `mfb spec <pkg> --all` renders for all 12 with no leaked
  `[[`; any code fence changed by a repair is re-checked.
- Doc sync: plan-125-I's contract-coverage enumeration updated in place where
  an entry is consciously revised.
- Acceptance: the whole-surface sweeps at target, cargo gates green, and
  `--reconcile` 0.

## Open Decisions

- **What if Phase 1 measures very little duplication?** — Then iteration 3 on
  the spec is cheap and the plan records that as evidence about the
  three-iteration design. The pass still runs; the measurement is the point.
- **A *fact* finding here that contradicts an iteration-2 rejection** — re-run
  the disproving command from the J/K/L ledger first. If it still disproves
  the finding, reject it with that command; if it no longer does, the original
  rejection was wrong and both ledgers are corrected.
- **A duplicated fact whose two homes are in different packages** — recommend
  the canonical topic be the one whose *package* owns the subject
  (`PACKAGE_ORDER` and the package overviews define that), with the other
  reduced to a summary and a `mfb spec <pkg> <topic>` link, exactly as
  `.ai/specifications.md` prescribes.

## Corrections

<!-- Filled in DURING execution. -->

## Summary

Single source of truth is the one rule that a per-file pass structurally
cannot keep, so this letter exists to repair what letters J–L were designed to
break. Measuring the damage in Phase 1 before repairing it in Phases 2–4 is
what turns "we ran three iterations" into evidence about whether three
iterations were the right design.
