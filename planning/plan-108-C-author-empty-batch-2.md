# plan-108-C: Author the empty packages, batch 2 — net, http, general, astrings, vector

Last updated: 2026-08-24
Effort: large (3h–1d)
Depends on: plan-108-B (batch-1 landed; the workflow has now run on 3
packages beyond the pilot — any standard amendments from B are in).

Author the remaining all-empty packages — **net (23 function pages), http
(19), general (18), astrings (18), vector (17) = 95 pages** plus overviews
and types pages — through the plan-108-A four-step workflow. Also close out
the ~10 empty straggler pages A's census found inside otherwise-filled
packages (exact list from the census re-run; recorded here at kickoff).

See plan-108-A §3 for the workflow and the standard. Per A: verification is
`mfb man` rendering + ad-hoc example/probe runs — no compiler test gates.

References:

- `src/codegen/builtins/{net,http,general,astrings,vector}/` — descriptor
  prose fields being filled.
- `planning/old_man/builtins/**` — source material (claims re-verified,
  citations stripped).
- `.ai/net-tls.md` — the INTERNALS doc for net/TLS; useful to the author for
  verifying claims, and the canonical example of content that must NOT leak
  into man prose (readiness/timeout machinery is spec/internals; the man
  page states developer-visible timeout/error behavior only).
- Memory `editing-package-mfb-drifts-many-goldens` — http/net have MFBASIC
  `package.mfb` bodies whose line numbers feed embedded ErrorLoc goldens;
  prose fields in `mod.rs`/func files are fine, but NEVER touch
  `package.mfb` files in this plan (out of scope, and a golden-drift event).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-108-B complete | B's boxes ticked; census 100% for strings/term/testing | NOT MET until B lands |

## 1. Goal

- Every function page in net, http, general, astrings, vector has non-empty
  `intro`/`desc`/`example` + param descs; overviews and types pages
  reviewed/corrected; the straggler pages (list at kickoff) filled.
- All claims behavior-verified; zero internals leakage.
- Every example compiled while authoring, and run where it needs no live
  endpoint (no example may depend on an external network); compile-only
  members noted per function in the ledger.
- Cross-model review (opus) per package; findings ledgers recorded here.
- `scripts/man-census.sh` → 100% fill for all five packages AND the
  straggler list; at this letter's end, **every one of the census's 466
  function pages has desc+example** (the authoring half of plan-108 done).

### Non-goals (explicit constraints)

- Per plan-108-A (no compiler testing; prose string fields only with
  per-commit `git diff` check; no renderer/schema changes;
  `src/docs/man/**` untouched).
- No `package.mfb` edits (see References).
- Found code bugs: fix or file via write-bug, recorded here — never doc'd
  around.

## 2. Current State

A's census: net 23 / http 19 / general 18 / astrings 18 / vector 17
function pages with desc column = 0. Stragglers: 194 total empty − 184 in
the nine all-empty packages = 10 spread across filled packages (identify
exactly at kickoff via the census script's per-function output).

### Measured populations

| What | Count | Command |
|---|---|---|
| pages to author | 95 + stragglers (list at kickoff) | `scripts/man-census.sh` at kickoff |
| net/http members run vs compile-only | decided per function in Phase 2 | this letter's ledger |
| old_man source coverage | measure at kickoff | `ls planning/old_man/builtins/{net,http,general,astrings,vector}` |

## 3. Design Overview

Same production line as B: one package at a time, author+scope then
review+apply, census per package. Order: general (broadest developer
traffic), astrings, vector, then net, http (the two needing the most
internals-leakage discipline and per-function run-vs-compile calls).
Stragglers last (small, scattered).

**Risk concentration:** net/http prose drifting into transport-internals
(readiness, TLS handshake machinery) — exactly the leakage class the user
called out. Held by the standard's MUST-NOT list and reviewers prompted
with `.ai/net-tls.md` as the "this is what internals look like" foil.

### Rejected alternatives

- **Skip examples for net/http since they can't hit the network.**
  Rejected: the standard requires an example everywhere; a compile-verified
  example is still type-checked, current, and shown to developers.

## Compatibility / Format Impact

None to codegen/wire. Summary-pin update only if a pinned summary is itself
corrected.

## Phases

### Phase 1 — general, astrings, vector

- [ ] Author 18+18+17 pages + overviews + types pages; every example
      compiled and run.
- [ ] Cross-model review per package + apply; ledgers here.
- [ ] Verify: rendering reads clean; census 100% each.

Acceptance: three packages fully authored and reviewed.
Commit: —

### Phase 2 — net, http

- [ ] Author 23+19 pages + overviews + types pages; per-function
      run-vs-compile verification recorded (no external-endpoint
      dependence).
- [ ] Cross-model review + apply; ledgers.
- [ ] Verify: rendering + census as Phase 1.

Acceptance: both packages authored and reviewed, ledgered.
Commit: —

### Phase 3 — stragglers

- [ ] Fill the ~10 straggler pages (exact list recorded here at kickoff)
      inside their filled packages, to standard, examples compiled/run;
      their packages' full review rides with D/E's sweep.
- [ ] Verify: census shows **0 pages without desc+example tree-wide**.

Acceptance: census-wide authoring complete (466/466 pages carry
desc+example).
Commit: —

## Validation Plan

- Verification: `mfb man <pkg> --all`/`types` per package;
  `scripts/man-census.sh` → zero empty pages anywhere; examples/probes
  compiled and run ad hoc during authoring.
- Doc sync: none beyond content (F owns tooling docs).
- Hygiene: fmt at session end.

## Open Decisions

- None entering — per-function run-vs-compile decisions are made and
  recorded in-phase.

## Corrections

<Filled in during execution.>

## Summary

The authoring close-out: after this letter no builtin man page is a bare
skeleton — every one of the 466 function pages has verified developer prose
and a compiled example, leaving D/E to verify the pre-existing prose and F
to certify.
