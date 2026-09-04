# plan-125-J: spec iteration 2, batch 1 — architecture and language (47 files)

Last updated: 2026-09-04
Effort: x-large (1d–3d)
Depends on: plan-125-I (spec iteration 1 complete across all 12 packages, the
citation surface at 0 `MISS-*`, and the `belongs-in-spec` ledger closed).

Landing unit: **each Phase below is independently landable and gets its own
commit.** The letter totals x-large; it is never landed as one change, and a
session that lands one phase and stops has left the tree consistent.

Iteration 2 of three on the spec surface, first of three batches. **The review
unit is one file** — one topic, read alone. The question is the one that needs
that isolation: *is every normative claim in this topic true of the compiler
at HEAD, and is every non-obvious one cited to a symbol that exists?*

This is the depth pass, and on this surface it is expensive in a way the man
surface's was not: verifying a spec claim can mean reading a compiler pass.
The triage rule that keeps it bounded is **citation-first** (§3).

Batch 1 is `architecture` (the pipeline, passes and CLI) and `language`
(source semantics) — 47 files, 8,529 lines, 643 citations. Together they are
what a new compiler contributor reads first, and they are the two packages
whose claims are most likely to have drifted with the code.

References:

- plan-125-A §3.2 (why the file lens differs), §3.3, §4.2, §4.3, §5 (the spec
  iteration-2 prompt).
- `.ai/spec-content.md`; `.ai/specifications.md`.
- plan-125-I Phase 1's **suspect-claim list** (the stale-by-deletion citation
  class) — every entry in this batch's packages is resolved here.
- `.ai/codegen-invariants.md`, `.ai/arch-abi.md`, `.ai/build-tooling.md` —
  the invariant docs whose subject matter `architecture` specifies; a
  disagreement between an invariant doc and the spec is a finding in one of
  them and must not be left standing in both.
- Memory `plan-line-citations-decay-silently` — cite the symbol plus its grep,
  never a bare `file.rs:NNN`.
- Memory `codegen-determinism-was-untested`, `never-add-lowering-variants`,
  `abi-inline-two-modes`, `one-type-grammar-parse-is-canonical` — invariants
  `architecture`/`language` claim; each is checkable against the code.

## Prerequisites

See plan-125-A. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-125-I complete | `grep -c '^- \[ \]' planning/plan-125-I-spec-iter1-packages.md` → `0` | — |
| citation surface clean entering the batch | `./scripts/spec-census.sh --citations` → 0 `MISS-*` | — |

## 1. Goal

- **All 47 files** through my per-file pass, one `codex exec` each, and apply.
- **Every normative claim verified against the compiler at HEAD** — by
  reading the cited symbol, or by running the compiler, or by a probe.
  A claim that cannot be verified either gets a citation that makes it
  verifiable or is cut.
- **Every non-obvious implementation claim carries a `[[path:Symbol]]`
  citation** at claim-cluster granularity, and every citation's symbol is
  grep-confirmed at the moment it is written (`.ai/specifications.md`).
- **Every suspect claim from plan-125-I's deletion-class list in these two
  packages is resolved** — verified, corrected, or deleted, with the evidence.
- **Every code fence checked**: MFBASIC fences compile (or are recorded as
  deliberately partial with the reason); non-MFBASIC fences (IR, NIR, asm,
  manifest, CLI output) are checked against real output.
- Every finding has a verdict; every rejection a disproving command.
- `--reconcile` exits 0 over the 47-unit list; `--citations`/`--links` clean.
- `cargo build` and `cargo test --bin mfb spec` green.

### Non-goals (explicit constraints)

- **No cross-file reconciliation here.** If two topics now disagree, record
  it — letter M is the re-integration pass and owns it.
- **No renumbering or reordering of topics**; a new topic only where letter I
  recorded a proven gap.
- The memory-vocabulary ban does not apply on this surface.
- Compiler behavior is not changed; a code defect goes through `write-bug`
  (plan-125-A §3.5).
- No wording churn on a claim that survives verification.

## 2. Current State

Entering J, both packages have been read once as wholes (letter I) and their
citations resolve. **No claim in either has ever been verified against the
code by anyone.**

### Measured populations

| What | Count | Command |
|---|---|---|
| `architecture` files / lines / citations | 24 / 5,030 / 510 | `find src/docs/spec/architecture -name '*.md' \| wc -l`; `cat src/docs/spec/architecture/*.md \| wc -l`; `grep -rhoE '\[\[[^]]+\]\]' src/docs/spec/architecture --include='*.md' \| wc -l` |
| `language` files / lines / citations | 23 / 3,499 / 133 | same, `language` |
| **batch total** | **47 files, 8,529 lines, 643 citations** | sum |
| citation density | `architecture` 21 citations per file vs `language` 6 | 510/24, 133/23 |
| broken citations entering J | 0 | fixed in plan-125-I (6 in `architecture`, 1 in `language` at baseline) |
| rendered size | `mfb spec language --all` → 5,604 lines | `./target/release/mfb spec language --all \| wc -l` |

### Verified properties

- **`language` is under-cited relative to `architecture`** — VERIFIED by the
  density measurement: 6 citations per file against 21. That is not
  necessarily wrong (`language` specifies source semantics, much of which is
  self-evident from the grammar), but it means `language`'s claims are
  *less checkable by construction* and this letter has to verify more of them
  by running the compiler rather than by reading a cited symbol. Budget
  accordingly.
- UNVERIFIED: the accuracy of any claim in either package. That is the whole
  content of this letter.

## 3. Design Overview

Per file: my pass → one `codex exec` from the spec iteration-2 prompt → apply,
at `N` concurrency with the main thread as sole writer.

**The citation-first triage rule** — the only thing that keeps a
"verify every claim" pass bounded on a 26,482-line surface:

1. A claim **with a resolving symbol citation** is verified *at that symbol*:
   read the cited code, confirm the claim, done. This is cheap and it is the
   common case in `architecture`.
2. A claim **with no citation** that is non-obvious is either given one (find
   the symbol, grep-confirm it, cite it) or, if no symbol supports it, treated
   as a claim to verify by running the compiler.
3. A claim **that cannot be verified either way** is cut, and the cut is
   recorded. `.ai/specifications.md`: "Do not add non-verifiable claims" — the
   same rule applies to the ones already there.
4. A claim that **disagrees with the code** is triaged per plan-125-A §3.5:
   spec stale → fix the spec; code wrong → `write-bug`.

**Order:** `language` (23 files, fewer citations but simpler subject; it
calibrates the run-the-compiler style of verification) → `architecture` (24
files, denser and heavier, with pace known).

**Risk concentration:**
- **Unbounded verification.** Held by the triage rule above and by recording,
  per file, how many claims fell into each of the four buckets — a file where
  bucket 2 dominates is a file that needs citations more than it needs prose.
- **Verifying a claim against a doc instead of the code.**
  `.ai/codegen-invariants.md` and friends are *also* prose written by someone;
  agreement between the spec and an invariant doc is not verification. The
  code is the oracle (memory `wrong-oracle-is-ratified-not-caught`).
- **A claim true at `-O0` and false at `-O3`**, or true on one target and
  false on another. `architecture` is full of these. Every conditional claim
  states its condition or is a finding.
- **Citing a symbol without grep-confirming it** — the exact mechanism that
  produced the 61 broken citations letter I just fixed. Every new or
  re-pointed citation is grep-confirmed at the moment it is written, and
  `--citations` is re-run at the end of every phase.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work; `- [~]` for partial; `- [x] ~~text~~ — moot: <evidence>`
> rather than deleting; fill `Commit:` on landing. **Unticked means NOT DONE.**

### Phase 1 — language (23 units)

- [ ] All 23 files, one review unit each.
- [ ] Every claim triaged into the four buckets; record the per-file bucket
      counts in this letter's ledger.
- [ ] Every MFBASIC code fence compiled (or recorded as deliberately partial
      with the reason).
- [ ] Add citations where bucket 2 claims are non-obvious and a symbol exists;
      grep-confirm each.
- [ ] Resolve every suspect claim from plan-125-I's deletion list attributed
      to `language`.

Acceptance: 23 units `exit 0`; `--citations language` → 0 `MISS-*`;
`--links language` clean; the bucket table is filled for all 23 files;
`cargo build` and `cargo test --bin mfb spec` green; `mfb spec language --all`
renders with no leaked `[[`.
Commit: —

### Phase 2 — architecture, files 01–12 (12 units)

- [ ] `01_commands` through `12_monomorphization`, one review unit each.
- [ ] Same four-bucket triage and bucket table.
- [ ] Every conditional claim (optimization level, target, feature) states its
      condition.
- [ ] Every non-MFBASIC fence (IR, NIR, CLI output) checked against real
      output from the current binary, not against its own plausibility.

Acceptance: 12 units `exit 0`; `--citations`/`--links` clean for the touched
files; bucket table filled; cargo gates green.
Commit: —

### Phase 3 — architecture, files 13–24 (12 units)

- [ ] The remaining 12 files, one review unit each.
- [ ] Same triage, same fence rule, same conditional-claim rule.
- [ ] Resolve every suspect claim from plan-125-I's deletion list attributed
      to `architecture`.

Acceptance: 12 units `exit 0`; `--citations architecture` → 0 `MISS-*`;
`--links architecture` clean; `--reconcile` exits 0 over the whole 47-unit
batch; the bucket table covers all 47 files; `cargo build` and
`cargo test --bin mfb spec` green; `mfb spec architecture --all` renders with
no leaked `[[`.
Commit: —

## Validation Plan

- Tests: `cargo build` and `cargo test --bin mfb spec` at the end of every
  phase. No full suite (memory `scope-the-test-run-to-the-blast-radius`),
  unless a `write-bug` fix in this letter touches compiler code — then that
  fix's own gates apply.
- Coverage check: `--reconcile` over the 47-unit list; the bucket table
  reconciled against the file list — 47 files, 0 unaccounted.
- Runtime proof: `mfb spec architecture --all` and `mfb spec language --all`
  render, no leaked `[[`; every MFBASIC fence compiled; every CLI/IR fence
  compared against real output.
- Doc sync: any disagreement found between the spec and an `.ai/*.md`
  invariant doc is resolved in **both**, in the same commit, per
  AGENTS.md's read-before-that-kind-of-work list.
- Acceptance: 0 `MISS-*` for both packages, `--links` clean, cargo gates
  green, `--reconcile` 0.

## Open Decisions

- **How much of `architecture` should be cited versus described?** —
  Recommend the existing 21-per-file density as the floor for
  `architecture` and a deliberate increase for `language`: a contributor
  cannot check a claim they cannot locate, and the 61 broken citations
  letter I fixed are evidence that citations decay *and* evidence that they
  were being used.
- **A claim that is true but that the compiler does not guarantee** (an
  incidental behavior documented as a contract) — recommend marking it
  explicitly as non-normative or cutting it. A spec that promises incidental
  behavior converts an implementation detail into a compatibility obligation.

## Corrections

<!-- Filled in DURING execution. -->

## Summary

This is the first time anyone has checked whether the specification is true.
The bounded-ness of that job rests entirely on the citation-first triage rule,
and the per-file bucket counts are how the letter proves it stayed bounded
rather than quietly degrading into proofreading. `language`'s low citation
density is the measured risk: fewer of its claims can be checked cheaply, so
more of them have to be checked by running the compiler.
