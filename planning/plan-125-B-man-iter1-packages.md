# plan-125-B: man iteration 1 — every package and guide topic as a whole, plus the cross-surface consistency review

Last updated: 2026-09-04
Effort: large (3h–1d)
Depends on: plan-125-A (the standards, the harness, the prompts, and the
pilot's measured per-unit cost all exist; without them this letter has no
instrument and no calibration).

Iteration 1 of three. The review unit is **a whole package or a whole guide
topic**, read as one document. This is the only iteration that can see
*coverage* (something a developer needs that no page mentions), *internal
consistency* (siblings explaining one concept two ways), *the overview's
promises against what its functions deliver*, and *ordering and
discoverability*. It is deliberately not a per-claim pass — that is
iteration 2 (letters C–F).

It ends with the **cross-package consistency review**: the one review in the
whole plan that looks at all 41 units at once, before iteration 2 fragments
the surface into 590 independent edits.

Audience, restated because it is the whole point: **the MFBASIC developer,
using and learning the language.** Not a compiler contributor. A sentence
that requires a compiler mental model to parse is a finding here even if it
is true.

References:

- plan-125-A §3.1 (the audience table), §3.2 (why the three lenses differ),
  §3.3 (the per-unit workflow), §4.3 (the fan-out harness), §5 (the
  iteration-1 prompt, run verbatim).
- `.ai/man-content.md` — the standard, as extended by plan-125-A Phase 2 to
  cover the narrative topics.
- plan-108-A §3 (2a) — the memory-vocabulary ban, its rewrite table, and the
  two carve-outs. Unchanged and still binding.
- `planning/plan-125-belongs-in-spec.md` — this letter appends to it.

## Prerequisites

See plan-125-A. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-125-A complete | `grep -c '^- \[ \]' planning/plan-125-A-standards-tooling-pilot.md` → `0` | — |
| the pilot's cost table is filled with measured numbers | read plan-125-A Phase 5's table | — |
| `--reconcile` is self-tested | plan-125-A Phase 3 acceptance | — |

## 1. Goal

- **All 39 remaining iteration-1 units** — 30 packages (31 minus `color`,
  done in A's pilot) and 9 guide topics (10 minus `variable`, done in A's
  pilot) — have been through my pass, one `codex exec` review, and apply.
- **Every unit's ledger is in this file**: finding / verdict / evidence, and
  for every rejection **the command that disproves it**.
- **Coverage findings are acted on, not deferred.** Iteration 1's
  characteristic finding is "this package never tells the developer X". A
  missing page or a missing paragraph is a task in this letter.
- **The cross-package consistency review is complete** (§3.2) and its
  findings applied: one concept, one vocabulary, across the whole surface.
- **`planning/plan-125-belongs-in-spec.md` carries every sentence this letter
  cut for being too internal**, with the spec package it belongs to.
- Sweeps still clean for every touched package:
  `./scripts/man-census.sh --memory-scope <pkg>` → 0 unclassified;
  `--scope <pkg>` → 0; `--fill` still 100%.
- `./scripts/doc-review-fanout.sh --reconcile` exits 0 over this letter's
  39-unit list plus the consistency runs.

### Non-goals (explicit constraints)

- Per plan-125-A: no compiler test gates; prose fields and markdown only;
  `git diff` per commit shows string-literal/markdown changes only; no
  renderer or schema changes; the reviewer never commits.
- **Not a per-claim verification pass.** Resist the pull to verify every
  sentence here — that is iteration 2, and doing it now costs the plan a
  whole pass for nothing. Verify what the *package-level* lens surfaces.
- No wording churn on prose that passes the lens.

## 2. Current State

plan-125-A Phase 1 re-censused the surface. Entering this letter, the man
surface is 100% filled, 0 unclassified memory-vocabulary hits, 0 internals
hits — and **never reviewed as whole packages by anyone but plan-108**, which
did not see `color` or `canvas` and did not cover the guide topics at all.

### Measured populations

| What | Count | Command |
|---|---|---|
| iteration-1 units in this letter | **39** | 31 packages + 10 topics = 41, minus `color` and `variable` (plan-125-A Phase 5 pilot) |
| function pages behind those units | 510 | 538 minus `color` 28 |
| guide pages behind those units | 31 | 32 minus `variable` 1 |
| packages plan-108 never reviewed | 2 (`color`, `canvas`) | `color` did not exist; `canvas` is recorded as missed in plan-108-E's Corrections. `color` is done in A's pilot, so **`canvas` is this letter's highest-yield unit** |
| guide topics never independently reviewed | 10 | plan-108 authored `variable` and explicitly excluded the rest (plan-108-A Non-goals) |
| largest units | collections 49 pages, datetime 44, fs 41, strings 39, encoding 30 | `./scripts/man-census.sh --fill` |
| guide topics with subtopics | 4 (types 10 pages, flow 8, tour 6, tooling 2) | `find src/docs/man -name '*.md'` |

### Verified properties

- **The guide topics have never been through any review process** — VERIFIED
  by reading plan-108-A's Non-goals: "`src/docs/man/**` prose guides (tour,
  errors, link, lambda, …) are OUT of plan-108's scope", with `variable` the
  single carve-out. They are the pages a learner reads first and the least
  audited material in the product.
- **`canvas` carries 106 type descriptions** (`--fill` TYPES column
  `106/106`) — by far the largest `types` page, and one no reviewer has seen.
  Its unit is closer in size to a small package than to a types page.
- UNVERIFIED: whether any package regressed since plan-108. Measured by this
  letter, assumed neither way.

## 3. Design Overview

### 3.1 Order of units

Highest-uncertainty first, so a systemic finding is discovered while there are
still 38 units to apply it to:

1. **`canvas`** — never reviewed, largest types page.
2. **The 9 guide topics** — never reviewed, and they set the vocabulary every
   package page borrows. A terminology decision made here propagates.
3. **The packages that changed most since plan-108** — `datetime`, `http`,
   `process`, `json`, `tls`, `term`, `astrings`, `crypto` (`git log
   --since=2026-08-31 --name-only --format='' -- src/codegen/builtins | grep
   -oE 'builtins/[a-zA-Z]+/' | sort | uniq -c | sort -rn`).
4. The remainder, largest first.

### 3.2 The cross-package consistency review

Cannot be one run over 51,540 lines. It is run over a **condensed artifact** —
every package overview plus every `types` page plus every guide topic
overview (a small fraction of the surface, and the part where vocabulary is
actually *established*) — in four dimension-scoped runs:

1. **Concept vocabulary** — is one thing called one thing? (handle vs
   resource; fails vs errors vs raises; index vs position; byte vs character
   vs grapheme; empty vs blank).
2. **Overview shape** — do the 31 overviews answer the same questions in the
   same order, so a developer learns to read them?
3. **Guide↔package agreement** — do the 10 topics and the package pages agree,
   especially `types`, `errors`, `flow` and `variable` against the packages
   that lean on them?
4. **Handle/resource contract** — the packages that own `RES` handles (`fs`,
   `io`, `tcp`, `udp`, `tls`, `net`, `process`, `audio`, `canvas`, `term`)
   must state the open/close contract identically. plan-108-F recorded this
   as the sharpest divergence test and it has 27 packages of churn since.

Each dimension's findings are applied across **all** affected units, not just
the one that surfaced them — plan-108-B's recorded lesson: *a reviewer finding
is usually a class, not an instance.*

### 3.3 Risk

- **Scope creep into iteration 2.** The biggest cost risk in this letter is
  verifying claims that iteration 2 will verify anyway. Held by the prompt
  (plan-125-A §5 iteration-1 prompt states the lens and forbids per-claim
  verification) and by the ledger recording finding *class* per unit.
- **A coverage finding that is really a missing feature.** "The package never
  says how to X" sometimes means the package cannot X. That is a product
  observation, recorded in the ledger and *not* documented as if it worked.
- Network-package units: the reviewer cannot bind sockets (plan-108-C's
  lesson); any probe for `tcp`/`udp`/`tls`/`net`/`http` is run by the main
  thread.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work. `- [~]` for partial, with one line on what remains.
> `- [x] ~~text~~ — moot: <evidence>` rather than deleting. Fill `Commit:`
> the moment a phase lands. **An unticked box means NOT DONE.**

### Phase 1 — canvas and the nine guide topics (11 units)

The never-reviewed material, and the material that sets vocabulary for
everything after it.

- [ ] `canvas` (19 function pages + overview + a 106-description types page).
- [ ] `tour`, `types`, `flow`, `errors`, `lambda`, `link`, `optimizations`,
      `tooling`, `unicode` — each as a whole topic including its subtopic
      pages.
- [ ] Record every terminology decision made here in a running table in this
      file; later phases conform to it rather than re-deciding.
- [ ] Append every "belongs in spec" cut to
      `planning/plan-125-belongs-in-spec.md`.

Acceptance: 11 units in the manifest with `exit 0`, no `FAILED`, no `DIRTY`;
each has a ledger in this file with a verdict per finding and a disproving
command per rejection; `--memory-scope`/`--scope` clean for `canvas`;
`mfb man <topic>` renders for all nine.
Commit: —

### Phase 2 — the eight most-changed packages (8 units)

- [ ] `datetime`, `http`, `process`, `json`, `tls`, `term`, `astrings`,
      `crypto` — the packages with the most commits since plan-108 closed.
- [ ] For each: reconcile the page against what actually changed
      (`git log --since=2026-08-31 -- src/codegen/builtins/<pkg>`) — a prose
      field that was not touched by a behavior change is the likely defect.

Acceptance: 8 units reconciled in the manifest; ledgers recorded; sweeps clean
for all eight.
Commit: —

### Phase 3 — the remaining 20 packages

- [ ] `collections`, `fs`, `strings`, `encoding`, `math`, `vector`, `os`,
      `general`, `bits`, `io`, `audio`, `tcp`, `testing`, `thread`, `udp`,
      `net`, `csv`, `money`, `regex`, `app`, `errorCode` (21 names; `color` is
      A's pilot — 20 units remain here after Phase 2's eight).
- [ ] Apply every class-level finding from Phases 1–2 across these units as
      part of the pass, not as a separate sweep.

Acceptance: every remaining unit in the manifest with `exit 0`; ledgers
recorded; `./scripts/man-census.sh --fill` still 100%, `--memory-scope` 0
unclassified and `--scope` 0 across the whole surface.
Commit: —

### Phase 4 — the cross-package consistency review

- [ ] Build the condensed artifact (§3.2): all 31 overviews + all 20 types
      pages + all 10 topic overviews, concatenated deterministically.
- [ ] Run the four dimension-scoped reviews.
- [ ] Apply each finding **as a class** across every affected unit; record in
      the ledger which units each class touched.
- [ ] Re-run `--reconcile` over the full 39-unit list plus the four
      consistency runs.

Acceptance: four consistency runs in the manifest; every finding has a verdict
and, if confirmed, a list of the units it was applied to; `--reconcile` exits
0; the terminology table in this file is complete and is what letters C–G
conform to.
Commit: —

## Validation Plan

- Tests: none (man prose). If a fix changes text pinned by
  `tests/cli_man_summary_plain.rs` or `tests/cli_canvas_man_examples_compile.rs`,
  update the pin in the same commit and run that test alone.
- Coverage check: `./scripts/doc-review-fanout.sh --reconcile` over this
  letter's unit list — a clean letter means every unit *ran*.
- Runtime proof: `mfb man <pkg> --all`, `mfb man <pkg> types`,
  `mfb man <topic>` for every touched unit.
- Doc sync: `planning/plan-125-belongs-in-spec.md` appended; the terminology
  table in this file kept current.
- Acceptance: the four sweeps (`--fill`, `--memory-scope`, `--scope`,
  `--reconcile`) at their targets.

## Open Decisions

- **Does a coverage gap get a new function page, or a paragraph on an existing
  one?** — Recommend a paragraph wherever the information belongs to an
  existing member, and a new page only when a registry member genuinely has no
  page. A new page for a member that does not exist is a feature request, not
  a doc fix, and is recorded rather than written.

## Corrections

<!-- Filled in DURING execution. -->

## Summary

The yield concentrates in Phase 1: `canvas` and the nine guide topics are the
only material in the man surface that no independent reviewer has ever seen.
Phase 4 is the plan's only whole-surface look before iteration 2 fragments it,
so a vocabulary decision deferred out of Phase 4 costs 590 pages of drift.
