# plan-108-B: Author the empty packages, batch 1 — strings, term, testing

Last updated: 2026-08-24
Effort: large (3h–1d)
Depends on: plan-108-A (census script, `.ai/man-content.md` standard,
`tests/man_examples.rs` harness, and the pilot-calibrated four-step workflow
all exist; A's Prerequisites gate carries forward).

Author the man prose for the first batch of all-empty packages — **strings
(39 function pages), term (25), testing (12) = 76 pages** plus each package's
overview and types page — through plan-108-A's four-step workflow: accuracy
pass (author from code + old_man source material, every claim
behavior-verified), scope pass (developer docs, never compiler internals),
cross-model subagent review (opus), apply findings.

Batch composition: strings is the highest-developer-traffic empty package;
term and testing round the batch to ~76 pages, and both stress the standard
in useful ways (term: interactive/env-dependent examples → compile-only
classification; testing: `expect` desugars — prose must describe developer
semantics without leaking the lowering story).

See plan-108-A §3 for the workflow, the standard, and the harness contract.

References:

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
| plan-108-A complete | A's boxes ticked; harness enforces bits+thread | NOT MET until A lands |

## 1. Goal

- Every function page in strings, term, testing has non-empty `intro` (per
  A's intro policy decision), `desc`, `example`, and per-parameter `desc`;
  the package overviews and types pages are reviewed/corrected.
- Every claim behavior-verified (probe programs against the release binary
  or descriptor-table-derived); zero internals leakage per
  `.ai/man-content.md`.
- Examples on the harness: strings + testing run-enforced; term's
  classification decided per function (interactive members compile-only,
  pure members like sizing/attribute helpers run where they don't need a
  tty) and recorded in the harness table.
- Cross-model review completed for all three packages; findings ledger
  (confirmed → fixed / rejected → disproving command) recorded here.
- `scripts/man-census.sh` shows all three packages at 100% fill.

### Non-goals (explicit constraints)

- Per plan-108-A: codegen byte-identical (`artifact-gate all`); no renderer
  or registry-schema changes; no edits to byte-significant MFBASIC bodies;
  `src/docs/man/**` prose guides untouched.
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
| term functions needing compile-only classification | decided per function during Phase 1 | recorded in `tests/man_examples.rs` table |

## 3. Design Overview

Production-line: one package at a time through A's four steps, one commit
per package per step-pair (author+scope, then review-fixes), harness +
census + suite after each package. strings first (largest, best old_man
coverage), then term, then testing.

**Risk concentration:** silently inheriting a stale old_man claim. Held by
A's accuracy rule (probe programs), the reviewer's verify-not-proofread
prompt, and this letter's ledger requiring evidence per finding.

### Rejected alternatives

- **Author all 76 pages then review once.** Rejected: A's pilot calibration
  works package-by-package; a term-page systemic mistake caught by the
  strings review never gets made.

## Compatibility / Format Impact

None to codegen/wire. `cli_man_summary_plain.rs` re-pin only if a pinned
summary is corrected (4-question gate evidence recorded).

## Phases

### Phase 1 — strings

- [ ] Author 39 pages + overview + types page (accuracy + scope passes);
      add strings to the harness run-enforced list.
- [ ] Cross-model review (opus) + apply; ledger recorded here.
- [ ] Tests: `cargo test --no-fail-fast`; census 100% for strings;
      `artifact-gate all` byte-identical.

Acceptance: strings fully authored, reviewed, harness-enforced.
Commit: —

### Phase 2 — term

- [ ] Author 25 pages + overview + types page; per-function run/compile
      classification recorded in the harness table.
- [ ] Cross-model review + apply; ledger.
- [ ] Tests: as Phase 1.

Acceptance: term fully authored, reviewed, harness-enforced (with
classification table).
Commit: —

### Phase 3 — testing

- [ ] Author 12 pages + overview; describe `expect` semantics in developer
      terms (what a failed expectation reports; never the desugar story).
- [ ] Cross-model review + apply; ledger.
- [ ] Tests: as Phase 1.

Acceptance: testing fully authored, reviewed, harness-enforced.
Commit: —

## Validation Plan

- Tests: `cargo test --no-fail-fast` per package; `tests/man_examples.rs`
  enforced for all three.
- Coverage check: `scripts/man-census.sh` → 100% fill for strings, term,
  testing.
- Runtime proof: examples execute (run-list) via release `mfb`; probe
  programs during authoring.
- Doc sync: none beyond the man content itself (F owns tooling/AGENTS.md).
- Acceptance: full suite; `artifact-gate all`; `test-accept.sh` no NEW
  mismatch; fmt both crates.

## Open Decisions

- None entering the letter — classification calls (term run/compile) are
  made per function during Phase 2 and recorded in the harness table, not
  deferred.

## Corrections

<Filled in during execution.>

## Summary

The first authoring batch: the most-used empty package (strings) plus two
packages that stress-test the standard's example classification and
internals-leakage rules, all landed through the calibrated four-step
workflow with per-package review ledgers.
