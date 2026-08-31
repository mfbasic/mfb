# plan-108-F: Whole-surface certification + retire the stale man tooling

Last updated: 2026-08-30
Effort: medium (1h–2h) — grows to large if the certification sweeps or the
memory-vocabulary sweep find stragglers
Depends on: plan-108-E (every package authored/verified/reviewed).

Certify plan-108's end state with recorded, re-runnable checks — the same
lesson as plan-106-E: the certificate is a measured sweep, not a claim
assembled from letter tick-boxes (memory
`completeness-claims-need-an-audit`). Then retire the tooling and guidance
that still point at the RETIRED Markdown man tree, so the next author lands
in the registry workflow by default.

Per plan-108-A: the only verification instrument is `mfb man` rendering (+
the census script over it) — no compiler test gates.

References:

- `scripts/update_man.sh`, `scripts/update_man_package.sh` — drive claude
  CLI authoring against `.ai/man_template.md` / `man_type_template.md` /
  `man_package_template.md` for the retired `src/docs/man` tree
  (`update_man.sh:1-25`); dead tooling for builtins pages post-108.
- `AGENTS.md` "Creating or updating `mfb man` content" — guidance to
  extend with `.ai/man-content.md` AND the §3 (2a) memory-vocabulary ban,
  so the next author cannot reintroduce borrow/pointer prose without
  reading the rule. It must also learn the new `variable` topic (the
  section currently lists nine narrative topics).
- plan-108-A §3 (2a) + `src/docs/man/variable/package.md` — the ban and the
  page it points at; this letter certifies both.
- `src/cli/man.rs:1-15` — the renderer's own module doc describing the
  registry design (stays; it is accurate).
- `planning/old_man/**` — stays archived as-is (history), but no live doc
  may point to it as the authoring surface.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-108-E complete | E's boxes ticked; census 100% | NOT MET until E lands |

## 1. Goal

The certification, pasted into this file with each command's output:

- **Fill**: `scripts/man-census.sh` → every function page 100% on
  intro (per A's policy) / desc / example / param-desc; overview + types
  descriptions non-empty for every package.
- **Scope**: a rendered-output sweep for internals leakage —
  `./target/release/mfb man --all` (and per-package `types`) grepped for
  the MUST-NOT vocabulary from `.ai/man-content.md` (at minimum:
  `abi_inline|Body::|monomorph|lowering|NIR|\.ncode|__pkg_|#[a-z]+_[A-Za-z]+\$|\[\[`)
  → 0 hits; every residual hit either fixed or classified here with the
  reason it is legitimate developer vocabulary.
- **Memory vocabulary** (plan-108-A §3 (2a), the hard ban): the same sweep
  run with the banned memory list —
  `borrow|pointer|ownership|owns|owned|owner|move[sd]? (semantics|the value)|consume[sd]?|free[sd]?|frees|heap|stack-allocat|refcount|reference count|garbage collect|lifetime|dangling|deep copy|shallow copy|by reference|by value|RAII`
  over `mfb man --all` **and** the `src/docs/man/variable` topic — **0
  unclassified hits**. The ONLY sanctioned classification is carve-out 1,
  arithmetic borrow in `datetime` (15 rendered lines at baseline); every
  other hit is a defect this letter fixes, not classifies. Baseline to beat,
  measured 2026-08-30: 79 `borrow` / 15 `ownership` / 10 `owns` / 5 `heap` /
  2 `pointer` / 1 `deep copy` / 1 `by reference` across 15 packages — **94
  memory-sense hits** once the 15 `datetime` arithmetic borrows are
  excluded — 54 of them in plan-108-E's packages, 33 in plan-108-C's
  (which includes the network family's 26: tcp 14, udp 11, net 1).
- **The permitted vocabulary is actually used**: spot-check that the pages
  which USED to explain handles now say **alias** (or link
  `mfb man variable`) rather than having simply deleted the contract —
  `mfb man tcp accept`, `mfb man udp receive`, `mfb man tls accept`,
  `mfb man process spawn`, `mfb man audio play` read end-to-end here, with
  the result recorded. The first three are the sharpest test: C settled the
  network family's handle sentences and E's `tls` was required to copy
  them, so any divergence between these three is a failure of that
  hand-off, not a wording nit. Deleting a true statement to pass a grep is the
  failure mode this check exists for.
- **Citations**: rendered output contains no `[[path:symbol]]` markers
  (covered by the sweep above; called out because old_man ports are the
  likely source).
- **Example ledgers complete**: every letter's ledger accounts for every
  example as run or compile-only-with-reason — spot-check the union
  against the census's function list; 0 unaccounted.
- **Tooling retired**: `scripts/update_man.sh`, `update_man_package.sh`,
  and the three `.ai/man_*template*.md` files either deleted or rewritten
  for the registry workflow (Open Decision below); `AGENTS.md`'s man
  section rewritten to point at `.ai/man-content.md` + the registry fields;
  `rg -n 'src/docs/man' AGENTS.md .ai/ scripts/` → only intentionally
  historical references remain, each verified.
- **`mfb man variable` is reachable and consistent**: it renders, the
  topic list shows it, and every package page that gestures at the memory
  model links it rather than restating it (grep the rendered surface for
  competing explanations — the cross-package consistency check below).
- Memory sync: update this project's memory if the audit taught durable
  lessons (e.g. new sharp-edge behaviors discovered by probes); the
  `resources-in-collections-yes-records-no` memory's "the man blurb is
  WRONG" clause updated to reflect the fix landed.

### Non-goals (explicit constraints)

- Per plan-108-A (no compiler testing; no renderer/schema changes;
  `src/docs/man/**` prose guides remain out of scope — the sweep may
  incidentally note leakage there for a future plan, recorded without
  fixing).
- `planning/old_man/**` is not deleted (archive).

## 2. Current State (entering F)

All function pages across the **30** registry packages authored/verified
through A–E with per-package
review ledgers. What has NOT yet happened: any single sweep over the ENTIRE
rendered surface at once (letters worked per-package), and the
tooling/guidance still describes the retired tree.

### Measured populations

| What | Count | Command |
|---|---|---|
| rendered surface to sweep | re-measure (was 30,644 lines at plan-writing) | `./target/release/mfb man --all \| wc -l` |
| stale-tooling references | measure at kickoff | `rg -n 'src/docs/man\|update_man' AGENTS.md .ai/ scripts/` |
| memory-vocabulary hits at plan-writing (the number A–E drive to 0) | 79 borrow / 15 ownership / 10 owns / 5 heap / 2 pointer, 15 packages; 15 of the borrows are the datetime arithmetic carve-out | `./target/release/mfb man --all \| grep -cE '<word>'` (2026-08-30) |

## 3. Design Overview

Two sweeps (fill, scope) + the ledger completeness check, then the tooling
pass. Any straggler a sweep finds is a TASK in this letter, never a
deferral (plan-106-E's rule).

**Risk concentration:** (a) the scope grep's vocabulary list being too
narrow (leakage in words the grep doesn't know) — and for the memory ban
specifically, a page can teach a borrow model with no banned word in it
("the list still holds it, so do not close yours"), which no grep catches;
(b) B–E passing the grep by deleting true handle contracts instead of
restating them in MFBASIC terms. Mitigation: the grep is the
floor, the permitted-vocabulary read-through above is the ceiling; one final cross-model review run (Codex) reads the three largest
packages' rendered output end-to-end as a spot-check of the sweep itself,
and its findings calibrate whether a wider sweep is needed (recorded
either way) — its prompt quotes §3 (2a) and asks explicitly whether any
page still teaches a borrow/ownership mental model without using a banned
word.

### Rejected alternatives

- **Trust the per-letter reviews as certification.** Rejected: per-package
  passes cannot see cross-package inconsistency (terminology drift,
  duplicated-but-diverging explanations of shared concepts like scalar
  indexing) — the whole-surface sweep exists for exactly that class.

## Compatibility / Format Impact

None to codegen/wire.

## Phases

### Phase 1 — the certification sweeps

- [ ] Fill sweep (census) — paste output here; fix stragglers.
- [ ] Scope sweep (rendered grep + spot-check reviewer) — paste results +
      classifications here.
- [ ] Memory-vocabulary sweep (§1) — paste the per-word counts; every hit
      outside the datetime arithmetic carve-out fixed in this letter, not
      classified. Include `mfb man variable` in the swept surface.
- [ ] Permitted-vocabulary read-through of the five resource pages named in
      §1: each still states what happens to the handle, in MFBASIC terms.
- [ ] Ledger completeness check — paste the union-vs-census result here.
- [ ] Cross-package consistency: shared concepts (scalar vs grapheme
      indexing, raise-vs-clamp phrasing, resource lifetime wording) use one
      consistent explanation; fix divergences found by the spot-check.

Acceptance: all sweeps recorded in this file with 0 unclassified hits.
Commit: —

### Phase 2 — tooling + guidance retirement

- [ ] Execute the Open Decision on `update_man*.sh` + templates; rewrite
      `AGENTS.md`'s man-page section around `.ai/man-content.md`, and add a
      one-line statement of the memory ban (permitted: copy / mutate /
      value / alias-for-`RES`; everything else → `mfb man variable` or
      `mfb spec`) so it is visible without opening the standard.
- [ ] Add the `variable` topic to AGENTS.md's narrative-topic list (nine →
      ten).
- [ ] Memory sync per §1.
- [ ] Verify: `rg -n 'src/docs/man\|update_man' AGENTS.md .ai/ scripts/`
      output matches the Goal's residue rule; fmt at session end.

Acceptance: no live doc/script directs authors at the retired tree.
Commit: —

## Validation Plan

- Verification: the sweeps ARE the validation, recorded in this file;
  instrument is `mfb man` rendering + the census script.
- Doc sync: Phase 2 IS the doc sync; a memory entry recording the ban as a
  durable authoring rule (permitted four words, `mfb man variable` as the
  one detailed page) — it is exactly the kind of rule the source does not
  reveal. Archive all six plan-108 letters to
  `planning/completed/` as each finished (this letter last).
- Hygiene: fmt at session end (script/doc edits may touch no Rust; skip
  fmt if `git diff` shows no `.rs` change).

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

The certificate letter: fill and scope invariants proven by recorded
whole-surface sweeps (not assembled from per-letter claims), every example
accounted for in a ledger, and the last pointers to the retired Markdown
workflow removed — leaving `mfb man` documentation accurate,
developer-voiced, example-checked, free of C/Rust memory vocabulary, and
with a single written standard for whoever touches it next.
