# plan-108-C: Author the empty packages, batch 2 — net, http, general, astrings, vector

Last updated: 2026-08-24
Effort: large (3h–1d)
Depends on: plan-108-B (batch-1 landed; the workflow has now run on 3
packages beyond the pilot — any standard/harness amendments from B are in).

Author the remaining all-empty packages — **net (23 function pages), http
(19), general (18), astrings (18), vector (17) = 95 pages** plus overviews
and types pages — through the plan-108-A four-step workflow. Also close out
the ~10 empty straggler pages A's census found inside otherwise-filled
packages (exact list from the census re-run; recorded here at kickoff).

See plan-108-A §3 for the workflow, standard, and harness contract.

References:

- `src/codegen/builtins/{net,http,general,astrings,vector}/` — descriptor
  prose fields being filled.
- `planning/old_man/builtins/**` — source material (claims re-verified,
  citations stripped).
- `.ai/net-tls.md` — the INTERNALS doc for net/TLS; useful to the author for
  verifying claims, and the canonical example of content that must NOT leak
  into man prose (readiness/timeout machinery is spec/internals; the man
  page states developer-visible timeout/error behavior only).
- Memory `editing-package-mfb-drifts-many-goldens` — http/net builtins have
  MFBASIC package bodies whose line numbers feed embedded ErrorLoc goldens;
  prose fields in `mod.rs`/func files are fine, but NEVER touch
  `package.mfb` files in this plan (that is a golden-drift event, out of
  scope).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-108-B complete | B's boxes ticked; census 100% for strings/term/testing | NOT MET until B lands |

## 1. Goal

- Every function page in net, http, general, astrings, vector has non-empty
  `intro`/`desc`/`example` + param descs; overviews and types pages
  reviewed/corrected; the straggler pages (list at kickoff) filled.
- All claims behavior-verified; zero internals leakage.
- Harness classification: general/astrings/vector run-enforced (pure);
  net/http compile-only by default, run-enforced only for members that need
  no live endpoint (decided per function, recorded in the harness table —
  no example may depend on an external network).
- Cross-model review (opus) per package; findings ledgers recorded here.
- `scripts/man-census.sh` → 100% fill for all five packages AND for the
  straggler list; at this letter's end, **every one of the census's 466
  function pages has desc+example** (the authoring half of plan-108 done).

### Non-goals (explicit constraints)

- Per plan-108-A (byte-identical gate, no renderer/schema changes, no
  byte-significant body edits, `src/docs/man/**` untouched).
- No `package.mfb` edits (see References — golden-drift trap).
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
| net/http members safe to run-enforce | decided per function in Phase 1 | harness table |
| old_man source coverage | measure at kickoff | `ls planning/old_man/builtins/{net,http,general,astrings,vector}` |

## 3. Design Overview

Same production line as B: one package at a time, author+scope then
review+apply, harness/census/suite per package. Order: general (broadest
developer traffic), astrings, vector, then net, http (the two needing
careful example classification and the most internals-leakage discipline).
Stragglers last (small, scattered).

**Risk concentration:** net/http prose drifting into transport-internals
(readiness, TLS handshake machinery) — exactly the leakage class the user
called out. Held by the standard's MUST-NOT list and reviewers prompted
with `.ai/net-tls.md` as the "this is what internals look like" foil.

### Rejected alternatives

- **Skip examples for net/http since they can't hit the network.**
  Rejected: A's harness contract requires an example everywhere;
  compile-only classification exists precisely for this — a non-running
  example is still type-checked, current, and shown to developers.

## Compatibility / Format Impact

None to codegen/wire. Summary re-pins only with 4-question-gate evidence.

## Phases

### Phase 1 — general, astrings, vector

- [ ] Author 18+18+17 pages + overviews + types pages; run-enforce all
      three on the harness.
- [ ] Cross-model review per package + apply; ledgers here.
- [ ] Tests: `cargo test --no-fail-fast`; census 100% each;
      `artifact-gate all` byte-identical.

Acceptance: three packages fully authored, reviewed, harness-enforced.
Commit: —

### Phase 2 — net, http

- [ ] Author 23+19 pages + overviews + types pages; per-function
      run/compile classification (no external-endpoint dependence).
- [ ] Cross-model review + apply; ledgers.
- [ ] Tests: as Phase 1.

Acceptance: both packages authored, reviewed, enforced with classification
tables.
Commit: —

### Phase 3 — stragglers

- [ ] Fill the ~10 straggler pages (exact list recorded here at kickoff)
      inside their filled packages; review rides along with D/E's sweep of
      those packages EXCEPT accuracy/scope which happen now (a straggler is
      authored to standard immediately).
- [ ] Tests: as Phase 1; census shows **0 pages without desc+example
      tree-wide**.

Acceptance: census-wide authoring complete (466/466 pages carry
desc+example).
Commit: —

## Validation Plan

- Tests: `cargo test --no-fail-fast`; harness enforced for all five
  packages.
- Coverage check: census → zero empty pages anywhere.
- Runtime proof: run-list examples execute via release `mfb`.
- Doc sync: none beyond content (F owns tooling docs).
- Acceptance: full suite; `artifact-gate all`; `test-accept.sh` no NEW
  mismatch; fmt both crates.

## Open Decisions

- None entering — per-function classification decisions are made and
  recorded in-phase.

## Corrections

<Filled in during execution.>

## Summary

The authoring close-out: after this letter no builtin man page is a bare
skeleton — every one of the 466 function pages has verified developer prose
and a harness-checked example, leaving D/E to verify the pre-existing prose
and F to certify.
