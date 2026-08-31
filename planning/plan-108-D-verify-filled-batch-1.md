# plan-108-D: Verify the pre-filled packages, batch 1 — datetime, fs, encoding, collections, math

Last updated: 2026-08-30
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
read the page, check each claim (probe program or descriptor table), compile
and run its example (never done before — A measured zero prior example
verification), apply the MUST-NOT scope list, move on; the cross-model
reviewer then re-verifies independently.

See plan-108-A §3 for the workflow and the standard. Per A: verification is
`mfb man` rendering + ad-hoc example/probe runs — no compiler test gates.

References:

- **plan-108-A §3 (2a) — the memory-vocabulary hard ban.** Permitted:
  **copy**, **mutate**, **value**, **alias** (`RES` handles only).
  Banned from rendered output: `borrow`, `pointer`, `ownership`/`owns`,
  `move`, `free`, `heap`, `refcount`, `lifetime`, `deep/shallow copy`,
  `by reference`, `drop` (memory sense) — use A's rewrite table, and link
  `mfb man variable` instead of re-explaining the model on a package page.
  Run `scripts/man-census.sh --memory-scope <pkg>` before closing each
  package; record before/after counts in the ledger.
  Rendered baseline (2026-08-30): datetime 15, collections 4, fs 0,
  encoding 0, math 0. **All 15 datetime hits are carve-out 1 — arithmetic
  borrow** ("a negative nanos value borrows a second"), NOT memory: keep
  them and classify the whole set once in this letter's ledger rather than
  per page. `fs` looks clean only because its 37 source hits are Rust
  module-doc comments that never render (A's population table) — verify by
  rendering, never by grepping the `.rs` file. `collections`'s 4 are the
  real work here: the overview's copy/mutation contract is exactly what
  `mfb man variable` now owns, so cut and link rather than restate.
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
| plan-108-C complete | census shows every function page across all 30 packages carrying desc+example (denominator per A's Phase 1 census — the first draft's 466 excluded tcp/udp) | NOT MET until C lands |

## 1. Goal

- All 160 pages + 5 overviews + types pages verified claim-by-claim and
  scope-checked; every inaccuracy fixed; every internals leak rewritten in
  developer terms or removed.
- **`scripts/man-census.sh --memory-scope` reports 0** for every package in
  this letter (plan-108-A §3 (2a)): no `borrow`, `pointer`, `ownership`,
  `move`, `free`, `heap`, `lifetime` in rendered output. Where a `RES`
  handle's behavior must be stated, it is stated with **alias** and
  MFBASIC's own verbs (open / close / stays open); anything longer links
  `mfb man variable`.
- Every example compiled and run during the pass (fs examples against temp
  paths only — rewrite any example that touches a real user path);
  compile-only members, if any, noted in the ledger.
- Cross-model review (Codex) per package; ledgers (confirmed → fixed /
  rejected → disproving command) recorded here.
- Census still 100% for all five.

### Non-goals (explicit constraints)

- **No new inline explanation of the memory model.** Any page that needs
  more than one sentence about copies or handles links `mfb man variable`
  (authored in A) — it does not re-explain, and never in C/Rust terms.
- Per plan-108-A (no compiler testing; prose string fields only with
  per-commit `git diff` check; no renderer/schema changes; no
  `package.mfb` edits; `src/docs/man/**` untouched).
- **No wording churn on accurate, in-scope prose** — this is an audit, not
  a rewrite; a page that passes both passes is left byte-for-byte alone.
- Found code bugs: fix or file via write-bug, recorded here.

## 2. Current State

A's census: these five packages carry desc+example on 160 of their 161
pages (datetime 44/45, fs 41/42, encoding 28/28, collections 24/24, math
21/21 — the missing singletons were authored as C's stragglers). The prose
was written during the builtins migration era; it has never been
independently audited, and no example has ever been compiled or run
(A's measurement).

### Measured populations

| What | Count | Command |
|---|---|---|
| pages to verify | 160 (+5 overviews, 5 types pages) | `scripts/man-census.sh` at kickoff |
| examples never before compiled | 160 | A's measurement (zero prior example verification) |
| claims per page | unbounded prose — the reader and the reviewer, not a grep, are the coverage instrument | — |

## 3. Design Overview

Per-package: verification pass (steps 1+2 of the workflow, page by page,
fixing as found) → cross-model review → apply. Order: collections first
(the overview makes strong behavioral contracts — "do not mutate", ordering
rules — worth auditing early, and its 24 pages calibrate audit pace), then
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

None to codegen/wire. Summary-pin update only if a pinned summary is itself
corrected.

## Phases

### Phase 1 — collections, math

- [ ] Verify collections 24 + math 21 pages + overviews + types pages;
      every example compiled and run.
- [ ] Cross-model review per package + apply; ledgers here.
- [ ] Verify: rendering reads clean; census still 100%.

Acceptance: both packages verified and reviewed; ledgers recorded.
Commit: —

### Phase 2 — encoding, fs

- [ ] Verify encoding 28 + fs 42 pages + overviews + types pages; fs
      examples rewritten onto temp paths where needed, all compiled and
      run.
- [ ] Cross-model review + apply; ledgers.
- [ ] Verify: rendering + census as Phase 1.

Acceptance: both packages verified and reviewed.
Commit: —

### Phase 3 — datetime

- [ ] Verify 45 pages + overview + types page; timezone/DST/precision
      claims probe-verified; examples compiled and run.
- [ ] Cross-model review + apply; ledger.
- [ ] Verify: rendering + census as Phase 1.

Acceptance: datetime verified and reviewed.
Commit: —

## Validation Plan

- Verification: `mfb man <pkg> --all`/`types` per package; census still
  100%; examples and probes compiled/run ad hoc with the release binary.
- Doc sync: none beyond content.
- Hygiene: fmt at session end.

## Open Decisions

- None entering the letter.

## Corrections

<Filled in during execution.>

## Summary

The heavy verification batch: the five biggest migrated-prose packages go
under the same evidence discipline the authored packages were born under —
probe-verified claims, scope-checked prose, every example finally compiled
and run, independently reviewed.
