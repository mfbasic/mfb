# plan-108-E: Verify the pre-filled packages, batch 2 — crypto, os, io, process, audio, tls, json, csv, money, regex, app

Last updated: 2026-08-30
Effort: huge (> 3d) — 105 pages across 11 packages plus 54 of the surface's
94 memory-vocabulary rewrites (the tls/process/audio handle prose); split
across sessions by phase
Depends on: plan-108-D (verification batch 1 landed; audit pace and reviewer
calibration proven on 160 pages).

Verify the remaining pre-filled packages — **crypto (17 function pages), os
(15), io (15), process (15), audio (12), tls (10), json (5), csv (5), money
(4), regex (4), app (3) = 105 pages** plus overviews and types pages —
through the same verification cycle as D. This batch carries the plan's one
KNOWN accuracy defect, most of the environment-dependent examples, and
**the largest share of the memory-vocabulary rewrite**.

`tcp` and `udp` were unassigned in plan-108's first draft (A's Corrections,
2026-08-30); they are assigned to **plan-108-C**, alongside `net`. C
therefore lands the network family's handle wording BEFORE this letter runs
— see Prerequisites: `tls` must match what C established, not invent its
own.

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
  **This letter carries the bulk of the violation.** Rendered baseline
  (2026-08-30): tls 23, process 18, audio 10, crypto 1, io 1, money 1 =
  **54**; os/json/csv/regex/app 0. The whole rendered surface holds 94
  memory-sense hits outside the datetime arithmetic carve-out — **54 of
  them (57%) are in this letter**, and C holds the next 26 (net 1, tcp 14,
  udp 11). Budget for it: this is a rewrite pass over the resource
  packages' handle prose, not a spot fix. Named offenders to fix by hand,
  each attributed to its package (line numbers into
  `mfb man --all > /tmp/man-all.txt`, 2026-08-30):
  **tls** `:36160` "The returned Socket is a borrowed pointer — an alias
  into the list" and `:35662` "closing the socket never frees";
  **process** `:12846` "Letting a Process drop at scope exit" and `:12964`
  "not treated as an ownership". Plus **15 of the 25 `Borrowed, not
  consumed` parameter descriptions** — process 7, audio 4, tls 4
  (`mfb man <pkg> --all | grep -c 'Borrowed, not consumed'`); the other 10
  (tcp 6, udp 4) are C's.
- `src/codegen/builtins/{crypto,os,io,process,audio,tls,json,csv,money,regex,app}/`.
- **plan-108-C's net/tcp/udp handle wording** — C fixes 26 memory-sense hits
  across net/tcp/udp before this letter starts. `tls` wraps the same
  socket concepts and must reuse C's sentences verbatim; diff
  `mfb man tcp accept` against `mfb man tls accept` rather than writing new
  prose. This is the cross-package consistency case F sweeps for.
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
| plan-108-D complete | D's boxes ticked; census 100% | **MET** 2026-08-31 — `./scripts/man-census.sh collections math encoding fs datetime` reports 183/183 on intro, desc and example and **286/286 parameter descriptions**; `--memory-scope` 0 (plus the 15 classified datetime arithmetic borrows) and `--scope` 0. Commits `56f99703d`, `3d5759dfe`, `b6256f0c6`, `4d275306b`, `dcf42e404`, `c5646161a`. |
| `mfb man variable` exists (this letter links it instead of re-explaining) | `mfb man variable` renders | **MET** — delivered by plan-108-A Phase 2b, commit `f816298ea` |
| C's net/tcp/udp handle wording is landed and readable | `mfb man tcp accept`, `mfb man udp receive` | **MET** — landed in `fd8e0473d`/`4a429d828` and recorded as a verbatim table in plan-108-C, "The settled network-family handle wording". `tls` copies rows 1, 2, 3 and 6. |

## 1. Goal

- All 105 pages + 11 overviews + types pages verified claim-by-claim and
  scope-checked; the `process` resources-in-collections defect fixed with
  the corrected wording recorded.
- **`scripts/man-census.sh --memory-scope` reports 0 for every package in
  this letter** (baseline 54; see References). The handle contract each
  rewritten sentence carried is preserved in MFBASIC terms — "stays open",
  "the caller still closes it", "an alias of the one in the list" — not
  deleted. `tls` ends with the SAME wording C gave `tcp`/`udp` for the same
  situation.
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

- **No new inline explanation of the memory model.** Any page that needs
  more than one sentence about copies or handles links `mfb man variable`
  (authored in A) — it does not re-explain, and never in C/Rust terms.
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
- [ ] Memory-scope rewrite for `process` (18 hits) and `io` (1): the
      `Borrowed, not consumed` parameter descriptions, "Letting a Process
      drop at scope exit", "not treated as an ownership". `os` is already 0
      — confirm, do not churn. Reuse C's net/tcp/udp handle sentences where
      the situation is the same (a handle that stays open, a handle closed
      by the call); record any process-specific sentence in the ledger.
- [ ] Cross-model review per package + apply; ledgers.
- [ ] Verify: rendering reads clean; census still 100%;
      `--memory-scope` = 0 for all three.

Acceptance: three packages verified; known defect fixed and recorded;
memory-scope 0.
Commit: —

### Phase 2 — crypto, tls, audio, app

The plan's heaviest memory-vocabulary phase: tls 23 + audio 10 + crypto 1 =
34 rendered hits.

- [ ] Verify 17+10+12+3 pages + overviews + types pages; compile-only
      classifications recorded (tls/audio/app largely compile-only).
- [ ] Memory-scope rewrite across tls/audio/crypto to 0. **`tls` reuses
      C's `tcp`/`udp` sentences verbatim** — same sockets, same listener,
      same accept; writing new wording here is the drift F sweeps for.
      Hand-fix the named offenders from References (`:36160` borrowed
      pointer, `:35662` never frees, `:36828` close moves). Where a page
      wants to explain the model, link `mfb man variable`.
- [ ] Cross-model review + apply; ledgers.
- [ ] Verify: rendering + census as Phase 1; `--memory-scope` = 0;
      `mfb man tls accept` diffed against `mfb man tcp accept` (C's) for
      identical handle wording.

Acceptance: four packages verified and reviewed; memory-scope 0; tls handle
wording matches C's tcp/udp.
Commit: —

### Phase 3 — json, csv, money, regex (+ errorcode/perf per A's ruling)

- [ ] Verify 5+5+4+4 pages + overviews + types pages; regex `\x{…}`
      pattern-vs-escape boundary stated precisely; execute A's
      errorcode/perf assignment.
- [ ] Memory-scope: `money` 1 hit; the rest already 0 — confirm.
- [ ] Cross-model review + apply; ledgers.
- [ ] Verify: rendering + census as Phase 1; `--memory-scope` = 0;
      all **30** registry packages now covered by a letter (`ls
      src/codegen/builtins/ | grep -v mod.rs | wc -l` = 30, cross-checked
      against A–E's package lists).

Acceptance: all remaining packages verified and reviewed; the 30-package
coverage cross-check recorded here.
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
