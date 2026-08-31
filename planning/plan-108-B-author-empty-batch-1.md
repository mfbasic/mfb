# plan-108-B: Author the empty packages, batch 1 — strings, term, testing

Last updated: 2026-08-30
Effort: large (3h–1d)
Depends on: plan-108-A (census script, `.ai/man-content.md` standard, and the
pilot-calibrated four-step workflow all exist; A's Prerequisites and no-test
verification policy carry forward).

Author the man prose for the first batch of all-empty packages — **strings
(39 function pages), term (25), testing (12) = 76 pages** plus each package's
overview and types page — through plan-108-A's four-step workflow: accuracy
pass (author from code + old_man source material, every claim
behavior-verified, every example compiled and run while writing it), scope
pass (developer docs, never compiler internals), cross-model review via the
Codex CLI, apply findings.

Batch composition: strings is the highest-developer-traffic empty package;
term and testing round the batch to ~76 pages, and both stress the standard
in useful ways (term: interactive examples that can only be compile-verified
without a tty; testing: `expect` — prose must describe developer semantics
without leaking the desugar story).

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
  Rendered baseline for this letter's packages (2026-08-30,
  `mfb man <pkg> --all | grep -cEi 'borrow|ownership|\bowns\b|pointer|deep
  copy|shallow copy|by reference|heap|refcount|dangling'`): strings 1,
  term 0, testing 0. Low — but these packages are being AUTHORED, so the
  risk is importing the vocabulary from `planning/old_man/**`, which
  predates the ban: strip it on port, do not carry it across. `term`
  pages describe a handle-free surface; if one needs to say a `RES` is
  shared, say **alias** and link `mfb man variable`.
- `src/codegen/builtins/strings/`, `…/term/`, `…/testing/` — the descriptor
  prose fields being filled.
- `planning/old_man/builtins/strings/` etc. — source material (claims
  re-verified, citations stripped; per A's accuracy rule).
- Known behavior sharp edges the strings pages MUST get right (and which
  make good reviewer bait): `strings::mid` raises `ErrIndexOutOfRange`
  rather than clamping (memory `mfb-strings-mid-raises-not-clamps`;
  old_man/builtins/strings/mid.md describes this correctly); string escapes
  are `\u{HEX}` not `\x{…}` (memory `mfb-string-escape-is-u-not-x`) —
  examples must not use non-escapes.
- `term::on` leaves ISIG enabled (`^C` = SIGINT, runtime restores the
  screen) — same memory; a term-page claim to verify, not assume.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-108-A complete | A's boxes ticked; census + standard committed | NOT MET until A lands |

## 1. Goal

- Every function page in strings, term, testing has non-empty `intro` (per
  A's intro policy), `desc`, `example`, and per-parameter `desc`; the
  package overviews and types pages are reviewed/corrected.
- Every claim behavior-verified (probe programs against the release binary
  or descriptor-table-derived); zero internals leakage per
  `.ai/man-content.md`.
- **`scripts/man-census.sh --memory-scope` reports 0** for every package in
  this letter (plan-108-A §3 (2a)): no `borrow`, `pointer`, `ownership`,
  `move`, `free`, `heap`, `lifetime` in rendered output. Where a `RES`
  handle's behavior must be stated, it is stated with **alias** and
  MFBASIC's own verbs (open / close / stays open); anything longer links
  `mfb man variable`.
- Every example compiled and run while authoring; term members that need a
  tty are compile-verified only, noted per function in this letter's
  ledger.
- Cross-model review completed for all three packages; findings ledger
  (confirmed → fixed / rejected → disproving command) recorded here.
- `scripts/man-census.sh` shows all three packages at 100% fill.

### Non-goals (explicit constraints)

- **No new inline explanation of the memory model.** Any page that needs
  more than one sentence about copies or handles links `mfb man variable`
  (authored in A) — it does not re-explain, and never in C/Rust terms.
- Per plan-108-A: no compiler testing (rendering is the verification);
  prose string fields only (never a body, descriptor type, or error table —
  `git diff` per commit shows string-literal prose changes only); no
  renderer or registry-schema changes; `src/docs/man/**` untouched.
- No behavior changes to the builtins themselves. **Exception discipline:**
  if the accuracy pass uncovers an actual code bug (doc says X, code does Y,
  and Y is wrong), that is a found bug — fix it or file it via write-bug per
  AGENTS.md, never paper over it in prose; record it here either way.

## 2. Current State

A's census (re-run at kickoff for exact numbers): strings 39 / term 25 /
testing 12 function pages, 0 with Description or Example
(census 2026-08-24: desc column = 0 for all three). old_man coverage for
these packages exists under `planning/old_man/builtins/<pkg>/` (543 pages
total across all packages).

### Measured populations

| What | Count | Command |
|---|---|---|
| pages to author | 76 (39+25+12) | A's census table; re-run `scripts/man-census.sh` at kickoff |
| old_man source pages available per package | measure at kickoff | `ls planning/old_man/builtins/{strings,term,testing} \| wc -l` |
| term members compile-verified only (no tty) | decided per function during Phase 2 | recorded in this letter's ledger |

## 3. Design Overview

Production-line: one package at a time through A's four steps, one commit
per package per step-pair (author+scope, then review-fixes), census re-run
after each package. strings first (largest, best old_man coverage), then
term, then testing.

**Risk concentration:** silently inheriting a stale old_man claim. Held by
A's accuracy rule (probe programs), the reviewer's verify-not-proofread
prompt, and this letter's ledger requiring evidence per finding.

### Rejected alternatives

- **Author all 76 pages then review once.** Rejected: A's pilot calibration
  works package-by-package; a term-page systemic mistake caught by the
  strings review never gets made.

## Compatibility / Format Impact

None to codegen/wire. `tests/cli_man_summary_plain.rs` pinned text updated
in the same commit only if a pinned summary is itself corrected.

## Phases

### Phase 1 — strings

- [ ] Author 39 pages + overview + types page (accuracy + scope passes);
      every example compiled and run.
- [ ] Cross-model review (Codex) + apply; ledger recorded here.
- [ ] Verify: `mfb man strings --all` + `types` read clean; census 100%
      for strings.

Acceptance: strings fully authored and reviewed.
Commit: —

### Phase 2 — term

- [ ] Author 25 pages + overview + types page; per-function run vs
      compile-only verification noted in the ledger.
- [ ] Cross-model review + apply; ledger.
- [ ] Verify: rendering + census as Phase 1.

Acceptance: term fully authored and reviewed.
Commit: —

### Phase 3 — testing

- [ ] Author 12 pages + overview; describe `expect` semantics in developer
      terms (what a failed expectation reports; never the desugar story).
- [ ] Cross-model review + apply; ledger.
- [ ] Verify: rendering + census as Phase 1.

Acceptance: testing fully authored and reviewed.
Commit: —

## Validation Plan

- Verification: `mfb man <pkg> --all`/`types` rendering per package;
  `scripts/man-census.sh` → 100% fill for strings, term, testing; examples
  and probes compiled/run ad hoc during authoring.
- Doc sync: none beyond the man content itself (F owns tooling/AGENTS.md).
- Hygiene: fmt at session end (prose lives in `.rs` files).

## Open Decisions

- None entering the letter — run-vs-compile verification calls for term are
  made per function during Phase 2 and recorded in the ledger, not
  deferred.

## Corrections

<Filled in during execution.>

## Summary

The first authoring batch: the most-used empty package (strings) plus two
packages that stress-test the standard's example and internals-leakage
rules, all landed through the calibrated four-step workflow with
per-package review ledgers.
