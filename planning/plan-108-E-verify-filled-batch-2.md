# plan-108-E: Verify the pre-filled packages, batch 2 — crypto, os, io, process, audio, tls, json, csv, money, regex, app

Last updated: 2026-08-24
Effort: large (3h–1d)
Depends on: plan-108-D (verification batch 1 landed; audit pace and reviewer
calibration proven on 160 pages).

Verify the remaining pre-filled packages — **crypto (17 function pages), os
(15), io (15), process (15), audio (12), tls (10), json (5), csv (5), money
(4), regex (4), app (3) = 105 pages** plus overviews and types pages —
through the same verification cycle as D. This batch carries the plan's one
KNOWN accuracy defect and most of the environment-dependent examples.

See plan-108-A §3 for the workflow and the standard. Per A: verification is
`mfb man` rendering + ad-hoc example/probe runs — no compiler test gates.

References:

- `src/codegen/builtins/{crypto,os,io,process,audio,tls,json,csv,money,regex,app}/`.
- **Known defect to fix here**: the `process` package prose claiming a
  resource "cannot be stored as a collection element" — WRONG per spec
  §15.6 (`List/Map OF RES …` is valid; ownership floats up); memory
  `resources-in-collections-yes-records-no`. Fix the prose, and record the
  corrected wording in this letter's ledger.
- Memory `mfb-string-escape-is-u-not-x` — `\x{…}` is regex-PATTERN-only
  syntax, not a string escape: the regex pages must state this boundary
  precisely (it is exactly the confusion a developer hits).
- `.ai/resources-packages.md` — resource-internals foil: man pages state
  developer-visible resource lifetime rules only.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-108-D complete | D's boxes ticked; census 100% | NOT MET until D lands |

## 1. Goal

- All 105 pages + 11 overviews + types pages verified claim-by-claim and
  scope-checked; the `process` resources-in-collections defect fixed with
  the corrected wording recorded.
- Every example compiled during the pass, and run where the environment
  permits (crypto/json/csv/money/regex are pure — run them; os/process
  where side-effect-safe: env reads, temp-dir spawns; io/audio/tls/app
  compile-only where they need a tty/device/endpoint) — each compile-only
  call noted per function in the ledger.
- Cross-model review (Codex) per package; ledgers recorded here.
- The `errorcode`/`perf` resolution from A executed if A assigned them here
  (whatever pages they own verified the same way, or the out-of-scope
  reason restated).
- Census still 100%; every registry package now authored or verified.

### Non-goals (explicit constraints)

- Per plan-108-A (no compiler testing; prose string fields only with
  per-commit `git diff` check; no renderer/schema changes; no
  `package.mfb` edits; `src/docs/man/**` untouched).
- No wording churn on accurate, in-scope prose.
- Found code bugs: fix or file via write-bug, recorded here.

## 2. Current State

A's census: these eleven packages carry desc+example on 105 of ~111 pages
(stragglers were authored in C Phase 3). Same migration-era provenance and
zero prior audit as D's batch. The `process` defect is already known-wrong
by memory + spec cite — it needs the fix, not a re-derivation.

### Measured populations

| What | Count | Command |
|---|---|---|
| pages to verify | 105 (+11 overviews, types pages) | `scripts/man-census.sh` at kickoff |
| known defects entering | 1 (`process` resources blurb) | memory + spec §15.6 |
| compile-only examples | one ledger row per function so classified | this letter's ledger |

## 3. Design Overview

Same per-package cycle as D. Order: process first (carries the known
defect — land the certain fix early), then io, os, crypto, tls, audio, app,
then the small pure four (json, csv, money, regex) as a closing sweep.

**Risk concentration:** example optimism — an example that "runs fine here"
but is environment-fragile (stdin EOF, audio device, tls endpoint). Held
by: compile-only classification for tty/device/endpoint members, recorded
per function — never an unrecorded skip (no silent gaps).

### Rejected alternatives

- **Defer the `process` fix to a bug doc.** Rejected: it is a one-line
  prose correction with the disproof already in hand (spec §15.6); fixing
  in-letter with the ledger entry is the write-bug small-triage path.

## Compatibility / Format Impact

None to codegen/wire. Summary-pin update only if a pinned summary is itself
corrected.

## Phases

### Phase 1 — process, io, os

- [ ] Verify 15+15+15 pages + overviews + types pages; fix the `process`
      resources-in-collections defect (ledger: old wording → new wording →
      spec cite); io stdin examples classified in the ledger.
- [ ] Cross-model review per package + apply; ledgers.
- [ ] Verify: rendering reads clean; census still 100%.

Acceptance: three packages verified; known defect fixed and recorded.
Commit: —

### Phase 2 — crypto, tls, audio, app

- [ ] Verify 17+10+12+3 pages + overviews + types pages; compile-only
      classifications recorded (tls/audio/app largely compile-only).
- [ ] Cross-model review + apply; ledgers.
- [ ] Verify: rendering + census as Phase 1.

Acceptance: four packages verified and reviewed.
Commit: —

### Phase 3 — json, csv, money, regex (+ errorcode/perf per A's ruling)

- [ ] Verify 5+5+4+4 pages + overviews + types pages; regex `\x{…}`
      pattern-vs-escape boundary stated precisely; execute A's
      errorcode/perf assignment.
- [ ] Cross-model review + apply; ledgers.
- [ ] Verify: rendering + census as Phase 1; every registry package now
      covered by a letter.

Acceptance: all remaining packages verified and reviewed.
Commit: —

## Validation Plan

- Verification: `mfb man <pkg> --all`/`types` per package; census still
  100%; examples/probes compiled and (where possible) run ad hoc; the
  ledger has a row for every compile-only example (no silent gaps).
- Doc sync: none beyond content.
- Hygiene: fmt at session end.

## Open Decisions

- None entering — classification calls are made and recorded in-phase.

## Corrections

<Filled in during execution.>

## Summary

The verification close-out: every remaining migrated-prose package audited,
the one defect we already knew about fixed with its disproof cited, and
every example in the registry finally compiled — leaving F to certify the
whole surface and retire the dead tooling.
