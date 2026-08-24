# plan-108-F: Whole-surface certification + retire the stale man tooling

Last updated: 2026-08-24
Effort: medium (1h–2h) — grows to large only if the certification sweeps
find stragglers
Depends on: plan-108-E (every package authored/verified/reviewed; harness
registry-wide).

Certify plan-108's end state with recorded, re-runnable checks — the same
lesson as plan-106-E: the certificate is a measured sweep, not a claim
assembled from letter tick-boxes (memory
`completeness-claims-need-an-audit`). Then retire the tooling and guidance
that still point at the RETIRED Markdown man tree, so the next author lands
in the registry workflow by default.

References:

- `scripts/update_man.sh`, `scripts/update_man_package.sh` — drive claude
  CLI authoring against `.ai/man_template.md` / `man_type_template.md` /
  `man_package_template.md` for the retired `src/docs/man` tree
  (`update_man.sh:1-25`); dead tooling for builtins pages post-108.
- `AGENTS.md` "Creating or updating a man page (`src/docs/man/**` …)" —
  stale guidance to replace with the registry workflow +
  `.ai/man-content.md`.
- `src/cli/man.rs:1-15` — the renderer's own module doc describing the
  registry design (stays; it is accurate).
- `planning/old_man/**` — stays archived as-is (history), but no live doc
  may point to it as the authoring surface.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-108-E complete | E's boxes ticked; harness enforced registry-wide | NOT MET until E lands |

## 1. Goal

The certification, pasted into this file with each command's output:

- **Fill**: `scripts/man-census.sh` → every function page 100% on
  intro (per A's policy) / desc / example / param-desc; overview + types
  descriptions non-empty for every package.
- **Examples**: `cargo test --no-fail-fast` includes `man_examples.rs`
  enforcing every registry package; the run/compile classification table
  has a recorded row for every compile-only example (no silent gaps).
- **Scope**: a rendered-output sweep for internals leakage —
  `./target/release/mfb man --all` (and per-package `types`) grepped for
  the MUST-NOT vocabulary from `.ai/man-content.md` (at minimum:
  `abi_inline|Body::|monomorph|lowering|NIR|\.ncode|__pkg_|#[a-z]+_[A-Za-z]+\$|\[\[`)
  → 0 hits; every residual hit either fixed or classified here with the
  reason it is legitimate developer vocabulary.
- **Citations**: rendered output contains no `[[path:symbol]]` markers
  (covered by the sweep above; called out because old_man ports are the
  likely source).
- **Tooling retired**: `scripts/update_man.sh`, `update_man_package.sh`,
  and the three `.ai/man_*template*.md` files either deleted or rewritten
  for the registry workflow (Open Decision below); `AGENTS.md`'s man
  section rewritten to point at `.ai/man-content.md` + the registry fields;
  `rg -n 'src/docs/man' AGENTS.md .ai/ scripts/` → only intentionally
  historical references remain, each verified.
- Memory sync: update this project's memory if the audit taught durable
  lessons (e.g. new sharp-edge behaviors discovered by probes); the
  `process`-blurb memory (`resources-in-collections-yes-records-no`) gets
  its "the man blurb is WRONG" clause updated to reflect the fix landed.

### Non-goals (explicit constraints)

- Per plan-108-A (byte-identical gate; no renderer/schema changes;
  `src/docs/man/**` prose guides remain out of scope — but the sweep may
  incidentally note leakage there for a future plan, recorded without
  fixing).
- `planning/old_man/**` is not deleted (archive).

## 2. Current State (entering F)

All 466+ function pages authored/verified through A–E with per-package
review ledgers; harness registry-wide. What has NOT yet happened: any
single sweep over the ENTIRE rendered surface at once (letters worked
per-package), and the tooling/guidance still describes the retired tree.

### Measured populations

| What | Count | Command |
|---|---|---|
| rendered surface to sweep | re-measure (was 30,644 lines at plan-writing) | `./target/release/mfb man --all \| wc -l` |
| stale-tooling references | measure at kickoff | `rg -n 'src/docs/man\|update_man' AGENTS.md .ai/ scripts/` |

## 3. Design Overview

Three sweeps (fill, examples, scope) then the tooling pass. Any straggler a
sweep finds is a TASK in this letter, never a deferral (plan-106-E's rule).

**Risk concentration:** the scope grep's vocabulary list being too narrow
(leakage in words the grep doesn't know). Mitigation: the grep is the
floor; one final cross-model reviewer (opus) reads the three largest
packages' rendered output end-to-end as a spot-check of the sweep itself,
and its findings calibrate whether a wider sweep is needed (recorded
either way).

### Rejected alternatives

- **Trust the per-letter reviews as certification.** Rejected: per-package
  passes cannot see cross-package inconsistency (terminology drift,
  duplicated-but-diverging explanations of shared concepts like scalar
  indexing) — the whole-surface sweep exists for exactly that class.

## Compatibility / Format Impact

None to codegen/wire.

## Phases

### Phase 1 — the three certification sweeps

- [ ] Fill sweep (census) — paste output here; fix stragglers.
- [ ] Example sweep (suite) — paste enforcement table summary here.
- [ ] Scope sweep (rendered grep + spot-check reviewer) — paste results +
      classifications here.
- [ ] Cross-package consistency: shared concepts (scalar vs grapheme
      indexing, raise-vs-clamp phrasing, resource lifetime wording) use one
      consistent explanation; fix divergences found by the spot-check.

Acceptance: all three sweeps recorded in this file with 0 unclassified
hits; suite green.
Commit: —

### Phase 2 — tooling + guidance retirement

- [ ] Execute the Open Decision on `update_man*.sh` + templates; rewrite
      `AGENTS.md`'s man-page section around `.ai/man-content.md`.
- [ ] Memory sync per §1.
- [ ] Tests: full `cargo test --no-fail-fast`; `artifact-gate all`;
      `test-accept.sh` no NEW mismatch; fmt both crates.

Acceptance: no live doc/script directs authors at the retired tree; gates
green.
Commit: —

## Validation Plan

- Tests: full suite (`--no-fail-fast`).
- Coverage check: the sweeps ARE the coverage instrument, recorded here.
- Runtime proof: harness + `artifact-gate all` + `test-accept.sh`.
- Doc sync: Phase 2 IS the doc sync; archive all six plan-108 letters to
  `planning/completed/` as each finished (this letter last).
- Acceptance: full suite; gate; test-accept; fmt both crates.

## Open Decisions

- **Delete vs rewrite `update_man*.sh` + `.ai/man_*template*.md`**:
  recommend DELETE the scripts (their claude-CLI batch workflow is
  superseded by this plan's per-package workflow) and delete the templates,
  folding anything still valuable (section order, conditional-section
  rules) into `.ai/man-content.md` — one authoring doc, one truth. Rewrite
  only if the user wants a scripted batch re-review capability kept.

## Corrections

<Filled in during execution.>

## Summary

The certificate letter: fill, example, and scope invariants proven by
recorded whole-surface sweeps (not assembled from per-letter claims), plus
the last pointers to the retired Markdown workflow removed — leaving `mfb
man` documentation accurate, developer-voiced, example-tested, and with a
single written standard for whoever touches it next.
