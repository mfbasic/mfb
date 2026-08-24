# plan-108-D: Verify the pre-filled packages, batch 1 — datetime, fs, encoding, collections, math

Last updated: 2026-08-24
Effort: large (3h–1d)
Depends on: plan-108-C (all authoring done; the workflow + reviewer prompts
have been through 10 packages — verification letters inherit a settled
standard).

Run the accuracy + scope + cross-model-review + apply cycle over the five
largest **pre-filled** packages — **datetime (45 function pages), fs (42),
encoding (28), collections (24), math (21) = 160 pages** plus overviews and
types pages. These pages already carry prose (filled during the builtins
migration); this letter's job is the user's mandate applied to them:
**verify every claim against the actual code, and verify the prose is
developer documentation, not compiler-internals spec** — then update from
the independent review.

Verification of an existing page is cheaper per page than authoring (B/C):
read the page, check each claim (probe program or descriptor table), apply
the MUST-NOT scope list, move on; the cross-model reviewer then re-verifies
independently.

See plan-108-A §3 for the workflow, standard, and harness contract.

References:

- `src/codegen/builtins/{datetime,fs,encoding,collections,math}/` — the
  pages under audit.
- `.ai/collections.md` — internals foil for collections prose (HOF rewrites,
  native lowering, in-place mutation mechanics = spec/internals; the man
  page states the developer contract: "helpers do not mutate their
  arguments" etc., which the collections overview already does well —
  verify, don't rewrite).
- Memory `tofloat-not-correctly-rounded` — a known behavior sharp edge
  (naive float parsing, ~1 ULP off): wherever a datetime/math/encoding page
  makes precision claims, verify them by probe, and document actual
  behavior honestly.
- Memory `inline-headroom-growable-record-collection` — records are
  `WITH`-only (no `a.field = v`): any example using record mutation syntax
  must be checked against real syntax.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-108-C complete | census shows 466/466 desc+example | NOT MET until C lands |

## 1. Goal

- All 160 pages + 5 overviews + types pages verified claim-by-claim and
  scope-checked; every inaccuracy fixed; every internals leak rewritten in
  developer terms or removed.
- All five packages' examples on the harness: datetime, encoding,
  collections, math run-enforced; fs run-enforced against temp paths only
  (examples must never touch real user paths — classification recorded in
  the harness table).
- Cross-model review (opus) per package; ledgers (confirmed → fixed /
  rejected → disproving command) recorded here.
- Harness enforced list now includes all five; census still 100%.

### Non-goals (explicit constraints)

- Per plan-108-A (byte-identical gate; no renderer/schema changes; no
  byte-significant body or `package.mfb` edits; `src/docs/man/**`
  untouched).
- **No wording churn on accurate, in-scope prose** — this is an audit, not
  a rewrite; a page that passes both passes is left byte-for-byte alone.
- Found code bugs: fix or file via write-bug, recorded here.

## 2. Current State

A's census: these five packages carry desc+example on 160 of their 161
pages (datetime 44/45, fs 41/42, encoding 28/28, collections 24/24, math
21/21 — the missing singletons were authored as C's stragglers). The prose
was written during the builtins migration era; it has never been
independently audited, and no example has ever been compiled or run by a
test (A's measurement: 0 example-executing tests pre-108).

### Measured populations

| What | Count | Command |
|---|---|---|
| pages to verify | 160 (+5 overviews, 5 types pages) | `scripts/man-census.sh` at kickoff |
| examples newly under harness enforcement | 160 | harness table diff |
| claims per page | unbounded prose — the reviewer, not a grep, is the coverage instrument | — |

## 3. Design Overview

Per-package: verification pass (steps 1+2 of the workflow, page by page,
fixing as found) → cross-model review → apply. Order: collections first
(the overview makes strong behavioral contracts — "do not mutate", ordering
rules — worth auditing early and its 24 pages calibrate audit pace), then
math, encoding, fs, datetime (largest last, with pace known).

**Risk concentration:** rubber-stamping — an audit pass that reads prose as
plausible instead of checking it. Held by: probe-program discipline for
every behavioral claim that isn't table-derived (clamps/raises, rounding,
timezone/DST claims in datetime, path semantics in fs), and by the
cross-model reviewer whose prompt demands independent verification with
evidence, not proofreading.

### Rejected alternatives

- **Grep-driven claim extraction instead of page-by-page reading.**
  Rejected: prose claims have no uniform spelling (memory
  `census-a-behavior-by-its-effect` — counting by one spelling
  undercounts); the census bounds the page set, the reader/reviewer bound
  the claims.

## Compatibility / Format Impact

None to codegen/wire. Summary re-pins only with 4-question-gate evidence.

## Phases

### Phase 1 — collections, math

- [ ] Verify collections 24 + math 21 pages + overviews + types pages;
      run-enforce both on the harness.
- [ ] Cross-model review per package + apply; ledgers here.
- [ ] Tests: `cargo test --no-fail-fast`; `artifact-gate all`
      byte-identical.

Acceptance: both packages verified, reviewed, harness-enforced; ledgers
recorded.
Commit: —

### Phase 2 — encoding, fs

- [ ] Verify encoding 28 + fs 42 pages + overviews + types pages; fs
      examples rewritten onto temp paths where needed; harness
      classification recorded.
- [ ] Cross-model review + apply; ledgers.
- [ ] Tests: as Phase 1.

Acceptance: both packages verified, reviewed, enforced.
Commit: —

### Phase 3 — datetime

- [ ] Verify 45 pages + overview + types page; timezone/DST/precision
      claims probe-verified.
- [ ] Cross-model review + apply; ledger.
- [ ] Tests: as Phase 1.

Acceptance: datetime verified, reviewed, enforced.
Commit: —

## Validation Plan

- Tests: `cargo test --no-fail-fast` per package; harness enforced for all
  five.
- Coverage check: census 100%; harness table covers all 160 examples.
- Runtime proof: run-enforced examples execute via release `mfb`; probe
  programs for behavioral claims.
- Doc sync: none beyond content.
- Acceptance: full suite; `artifact-gate all`; `test-accept.sh` no NEW
  mismatch; fmt both crates.

## Open Decisions

- None entering the letter.

## Corrections

<Filled in during execution.>

## Summary

The heavy verification batch: the five biggest migrated-prose packages go
under the same evidence discipline the authored packages were born under —
probe-verified claims, scope-checked prose, independently reviewed, with
every example now a tested artifact.
