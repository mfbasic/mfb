# plan-125-G: man iteration 3 — every package and guide topic re-read as a whole after the page pass

Last updated: 2026-09-04
Effort: large (3h–1d)
Depends on: plan-125-F (iteration 2 complete across all 621 man pages, and
its Phase 4 reconciliation clean — this letter reviews the *result* of that
pass and cannot start against a partial one).

Iteration 3 of three. The unit is again **a whole package or a whole guide
topic** — but the material is not what letter B read. Iteration 2 edited 621
pages *independently*, by design, and that reliably produces:

- **divergence** — two pages now explain one concept differently, because two
  reviewers improved them separately and correctly;
- **redundancy** — the same clarification added to five sibling pages, which
  reads as noise in the package view and is invisible in the page view;
- **broken references** — a page that pointed at a sentence its neighbour no
  longer contains;
- **a shifted centre of gravity** — the overview now promises less, or more,
  than the function pages deliver.

None of those are visible at page granularity. Iteration 3 is the
**re-integration pass**, and it is the reviewer's first sight of each package
in its final form.

**Its success test is inverted from the other two.** If iteration 3 returns
mostly *seam* findings, iteration 2 did its job. If it returns many *factual*
findings, iteration 2 under-performed — and that is a recorded result of this
letter, not something to paper over.

References:

- plan-125-A §3.2 (why this lens is not letter B's), §3.3, §4.3, §5 (the
  iteration-3 prompt, run verbatim).
- plan-125-B — this letter's baseline: B's terminology table, B's per-unit
  ledgers, and B's cross-package consistency findings. Every one is re-checked
  for survival through iteration 2.
- plan-125-C/D/E/F ledgers — the record of what changed on each page.
- `.ai/man-content.md`; plan-108-A §3 (2a).

## Prerequisites

See plan-125-A. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-125-F complete, including its Phase 4 reconciliation | `grep -c '^- \[ \]' planning/plan-125-F-man-iter2-batch4.md` → `0`; F's Phase 4 records 621 units, 0 unaccounted | — |
| all four iteration-2 letters' ledgers are complete | `grep -c '^- \[ \]' planning/plan-125-{C,D,E,F}-*.md` → `0` each | — |

## 1. Goal

- **All 39 units** — 30 packages and 9 guide topics (`color` and `variable`
  completed their iteration 3 in plan-125-A's pilot) — re-read as wholes,
  reviewed by one `codex exec` each from the iteration-3 prompt, and repaired.
- **Every divergence introduced by iteration 2 is repaired**, and the ledger
  records it as *seam* or *fact* so the letter can report the ratio.
- **plan-125-B's terminology table still holds after 621 edits** — verified
  entry by entry, not assumed. Any entry that no longer holds is either
  re-applied or consciously revised, with the revision recorded.
- **The handle-contract table from plan-125-F Phase 1 still holds** across the
  seven resource packages.
- Every finding has a verdict; every rejection a disproving command.
- `--reconcile` exits 0 over the 39-unit list.
- Whole-surface sweeps at target: `--fill` 100%, `--memory-scope` 0
  unclassified, `--scope` 0.
- **The man surface is final entering letter H** — H is a certification sweep,
  not another repair pass, so anything found here is fixed here.

### Non-goals (explicit constraints)

- Per plan-125-A: no compiler test gates; prose and markdown only; `git diff`
  per commit is string literals/markdown only; the reviewer never commits.
- **Not a third accuracy pass.** A factual finding here is fixed *and*
  recorded as an iteration-2 miss; it does not license re-verifying every
  claim, which would cost a fourth pass the plan does not have.
- No wording churn on prose that reads correctly in the package view.

## 2. Current State

Entering G, every one of the 621 man pages has been verified sentence by
sentence and its example compiled and run (letters A pilot, C, D, E, F). No
one has looked at a *package* since letter B, before any of those edits.

### Measured populations

| What | Count | Command |
|---|---|---|
| units in this letter | **39** | 31 packages + 10 topics − `color` − `variable` (A's pilot) |
| pages edited by iteration 2 before this letter | to be measured at kickoff | count the distinct pages appearing in the C/D/E/F ledgers with verdict `confirmed → fixed` |
| B's terminology-table entries to re-verify | to be measured at kickoff | rows in plan-125-B Phase 4's table |
| F's handle-contract table rows | 7 | plan-125-F Phase 1 |
| whole-surface baseline entering G | `--fill` 100%, `--memory-scope` 0 unclassified, `--scope` 0 | plan-125-F Phase 4 |

### Verified properties

- **UNVERIFIED and central: how much divergence iteration 2 actually
  introduced.** The plan asserts it will exist; the honest position is that
  nobody knows the number until this letter measures it. Phase 1 measures it
  before Phase 2 repairs it, so the ratio is a result rather than an
  impression.

## 3. Design Overview

**Phase 1 measures, Phases 2–4 repair.** That order matters: the divergence
count is this plan's only evidence about whether a three-iteration structure
was worth its cost, and it cannot be recovered after the repairs land.

Per unit: my pass (read the package rendered end to end; look for seams, not
facts) → one `codex exec` from the iteration-3 prompt → apply.

**Order:** the packages iteration 2 changed *most* first (they carry the most
seam risk), then the rest, then the guide topics last — a guide topic can only
be checked against a package surface that is already final.

**Risk concentration:**
- **Re-litigating iteration 2.** The strongest pull in this letter is to
  re-verify facts that were verified two weeks ago by a process designed to
  verify them. Held by the prompt's lens and by the ledger's seam/fact
  classification, which makes drift into a third accuracy pass visible
  immediately.
- **Repairing divergence by averaging.** When two pages disagree after
  iteration 2, one of them is right. The repair is to determine which — by
  probe if necessary — not to split the difference into wording that is true
  of neither.
- **Terminology regression.** 621 independent edits are 621 chances to
  reintroduce a spelling B retired. Held by re-verifying B's table entry by
  entry rather than by spot-check.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work; `- [~]` for partial; `- [x] ~~text~~ — moot: <evidence>`
> rather than deleting; fill `Commit:` on landing. **Unticked means NOT DONE.**

### Phase 1 — measure the divergence before repairing it

- [ ] Count the pages iteration 2 actually changed, per package, from the
      C/D/E/F ledgers; record the table here.
- [ ] Re-verify **every** entry in plan-125-B's terminology table against the
      current rendered surface; record each as *holds* / *regressed on N
      pages*.
- [ ] Re-verify the seven rows of plan-125-F's handle-contract table; record
      each as *holds* / *diverged*.
- [ ] Record the entry-state sweeps (`--fill`, `--memory-scope`, `--scope`).

Acceptance: three measurement tables in this file, each produced by a command
recorded beside it; no repair has landed yet.
Commit: —

### Phase 2 — the packages iteration 2 changed most (the top third)

- [ ] The 10 packages with the most iteration-2 fixes, by Phase 1's table —
      one review unit each.
- [ ] Classify every finding as *seam* or *fact* in the ledger; a *fact*
      finding also names the iteration-2 unit that missed it.
- [ ] Repair divergence by determining which page was right, never by
      averaging; record the deciding probe where one was needed.

Acceptance: 10 units `exit 0`; ledgers recorded with seam/fact classification;
sweeps clean for all 10.
Commit: —

### Phase 3 — the remaining 20 packages

- [ ] The other 20 packages, one review unit each.
- [ ] Same seam/fact classification and same repair rule.

Acceptance: 20 units `exit 0`; ledgers recorded; sweeps clean.
Commit: —

### Phase 4 — the nine guide topics, and the iteration-3 result

- [ ] `tour`, `types`, `flow`, `errors`, `lambda`, `link`, `optimizations`,
      `tooling`, `unicode` — each as a whole topic, checked against the now
      final package surface.
- [ ] Re-run the four cross-package consistency dimensions from plan-125-B
      §3.2 over the condensed artifact, to confirm the consistency work
      survived iteration 2.
- [ ] **Record the iteration-3 result**: total findings, seam vs fact, and the
      per-iteration-2-letter attribution of the fact findings. State plainly
      whether iteration 2 performed as designed.
- [ ] `--reconcile` over the full 39-unit list plus the four consistency runs.

Acceptance: 9 units + 4 consistency runs `exit 0`; the seam/fact ratio is
recorded with its attribution; whole-surface sweeps at target (`--fill` 100%,
`--memory-scope` 0 unclassified, `--scope` 0); `--reconcile` exits 0; the man
surface is declared final for letter H, with the sweeps as evidence rather
than the declaration.
Commit: —

## Validation Plan

- Tests: none (man prose); pinned-text tests updated in the same commit if
  touched, then run alone.
- Coverage check: `--reconcile` over the 39-unit list; Phase 1's three
  measurement tables reconciled against the C/D/E/F ledgers.
- Runtime proof: `mfb man <pkg> --all`, `mfb man <pkg> types`,
  `mfb man <topic>` for every unit; any example changed by a repair is
  recompiled and rerun.
- Doc sync: `planning/plan-125-belongs-in-spec.md` appended for any late cut;
  plan-125-B's terminology table updated in place if an entry is consciously
  revised.
- Acceptance: the three whole-surface sweeps at target and `--reconcile` 0.

## Open Decisions

- **What if Phase 1 measures very little divergence?** — Then iteration 3 is
  cheap and the plan says so; that is a legitimate outcome and it is recorded
  as evidence about the three-iteration design, not treated as a reason to
  skip the pass. The pass still has to run to know.
- **A *fact* finding in iteration 3 that contradicts an iteration-2
  rejection** — recommend re-running the disproving command from the
  iteration-2 ledger first. If the command still disproves it, the iteration-3
  finding is rejected with that same command; if it no longer does, the
  original rejection was wrong and both ledgers are corrected.

## Corrections

<!-- Filled in DURING execution. -->

## Summary

This letter is where the three-iteration design pays for itself or does not,
and Phase 1 is deliberately measurement-before-repair so the answer survives
the work. The one real risk is drifting into a third accuracy pass, which
would cost days and duplicate letters C–F; the seam/fact classification exists
to make that drift visible on the first unit rather than the twentieth.
