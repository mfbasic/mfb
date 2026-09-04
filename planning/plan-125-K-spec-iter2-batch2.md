# plan-125-K: spec iteration 2, batch 2 — stdlib, package, memory (45 files)

Last updated: 2026-09-04
Effort: x-large (1d–3d)
Depends on: plan-125-J (iteration 2 is one ordered pass; J's four-bucket
triage rule and its calibrated pace carry forward).

Landing unit: **each Phase below is independently landable and gets its own
commit.** The letter totals x-large; it is never landed as one change, and a
session that lands one phase and stops has left the tree consistent.

Iteration 2, batch 2. The unit is one file. Three packages that share a
property: **each specifies a contract something else in the tree depends on
byte-for-byte**, so a stale claim here is not a documentation defect, it is a
trap.

- **`stdlib`** (19 files, 3,758 lines) — the regex/datetime/csv/json/http/url/
  PCG64 models. It carried **42 of the 61 broken citations** at baseline, the
  highest concentration in the spec by a wide margin.
- **`package`** (16 files, 1,743 lines) — the `.mfp` byte format. A wrong
  offset or field order here is directly falsifiable and directly harmful.
- **`memory`** (10 files, 2,109 lines) — runtime value layouts, the native
  calling convention, the runtime-helper ABI. The counterpart of everything
  the man surface was forbidden to say, and the destination of the largest
  share of the `belongs-in-spec` ledger.

References:

- plan-125-A §3.2/§3.3/§4.2/§4.3/§5 (the spec iteration-2 prompt).
- plan-125-J §3 — the **citation-first four-bucket triage rule**, applied
  unchanged.
- `.ai/spec-content.md`; `.ai/specifications.md`.
- plan-125-I Phase 1's suspect-claim list — `stdlib`'s share is the largest
  in the plan and is resolved here.
- `.ai/collections.md`, `.ai/resources-packages.md` — invariant docs whose
  subject matter `memory` specifies; a disagreement is a finding in one of
  them, never left standing in both.
- Memory `a-leak-counter-must-cover-everything-it-guards`,
  `inline-headroom-growable-record-collection`,
  `collection-set-in-place-only-for-same-function-local`,
  `arena-state-is-per-thread` — layout and lifetime invariants `memory`
  claims; each is checkable.
- Memory `committed-mfp-goes-stale-on-resource-requalification`,
  `new-error-in-a-package-needs-a-data-object-row` — `package` format
  realities.
- Memory `splitting-package-mfb-render-order-doc-asymmetry` — the mechanism
  behind `stdlib`'s 42 broken citations.

## Prerequisites

See plan-125-A. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-125-J complete | `grep -c '^- \[ \]' planning/plan-125-J-spec-iter2-batch1.md` → `0` | — |
| citation surface still clean | `./scripts/spec-census.sh --citations` → 0 `MISS-*` | — |

## 1. Goal

- **All 45 files** through my per-file pass, one `codex exec` each, and apply.
- **Every claim triaged by plan-125-J's four buckets**, with the per-file
  bucket counts recorded — the evidence that the pass stayed bounded.
- **Every byte-level claim in `package` verified against a real `.mfp`** —
  offsets, field order, magic numbers and section layout checked against a
  package the current compiler produced, not against the text's own
  consistency.
- **Every layout and ABI claim in `memory` verified against the code** that
  implements it, at the cited symbol.
- **`stdlib`'s suspect-claim list fully resolved** — the deletion-class
  citations letter I identified, where the symbol exists nowhere in `src/` and
  the surrounding claim is therefore in question.
- **The `belongs-in-spec` entries attributed to `memory` are covered** —
  letter I resolved them as *covered* or *gap*; the gaps are filled here.
- Every finding has a verdict; every rejection a disproving command.
- `--reconcile` exits 0 over the 45-unit list; `--citations`/`--links` clean.
- `cargo build` and `cargo test --bin mfb spec` green.

### Non-goals (explicit constraints)

- **No cross-file reconciliation** (letter M owns it).
- The memory-vocabulary ban does not apply on this surface — `memory` in
  particular is *supposed* to say ownership, move, pointer and heap.
- Compiler behavior is not changed; a code defect goes through `write-bug`.
- **No change to the `.mfp` format**, only to its description. If the
  description is right and the code is wrong, that is a `write-bug`, not a
  spec edit.
- No wording churn on a claim that survives verification.

## 2. Current State

All three packages have been read as wholes (letter I) and their citations
resolve. `stdlib`'s 42 broken citations were repaired there, and — critically
— letter I split them into *stale by move* (re-pointed) and *stale by
deletion* (**claim suspect**). This letter is where the deletion class is
actually adjudicated.

### Measured populations

| What | Count | Command |
|---|---|---|
| `stdlib` files / lines / citations | 19 / 3,758 / 256 | `find src/docs/spec/stdlib -name '*.md' \| wc -l`; `cat src/docs/spec/stdlib/*.md \| wc -l`; `grep -rhoE '\[\[[^]]+\]\]' src/docs/spec/stdlib --include='*.md' \| wc -l` |
| `package` files / lines / citations | 16 / 1,743 / 89 | same, `package` |
| `memory` files / lines / citations | 10 / 2,109 / 139 | same, `memory` |
| **batch total** | **45 files, 7,610 lines, 484 citations** | sum |
| broken citations at baseline (repaired in I) | stdlib 42, memory 4, package 0 | plan-125-I §2 |
| share of the whole plan's citation rot in `stdlib` | 42 of 61 = 69% | plan-125-I §2 |
| citation density | `stdlib` 13/file, `memory` 14/file, `package` 6/file | 256/19, 139/10, 89/16 |

### Verified properties

- **`stdlib`'s rot has a known mechanism** — VERIFIED in plan-125-A §2 by
  spot-check: `__http_dechunk` is cited at
  `src/codegen/builtins/http/mod.rs` and now lives in
  `helper_dechunk_bytes.rs` (`grep -rl '__http_dechunk'
  src/codegen/builtins/http/`). The MFBASIC helper bodies moved out of
  `mod.rs` during the package split and `stdlib` never followed. That means
  the *claims* around them describe an older file organization and are worth
  reading with suspicion even where letter I re-pointed the link successfully.
- **At least one deletion-class claim exists in this batch's subject area** —
  VERIFIED: `lower_io_write_helper` is cited in
  `src/docs/spec/memory/07_runtime-helper-abi.md` and exists **nowhere else in
  the tree** (`grep -rl 'lower_io_write_helper' src/` → that file alone). The
  helper it names is gone; the ABI claim built on it is unverified and is
  adjudicated in Phase 3.
- **`package` has the lowest citation density of the three** (6 per file) —
  VERIFIED by measurement. For a byte-format spec that is not automatically
  wrong, because the format itself is the contract; but it means Phase 2's
  verification is done against a produced `.mfp`, not against cited code.
- UNVERIFIED: the accuracy of any claim in the batch. That is this letter.

## 3. Design Overview

Per file: my pass → one `codex exec` → apply, `N` concurrency, main thread
sole writer. plan-125-J's four-bucket triage rule applies unchanged.

**Order:** `package` (16, the most mechanically checkable — a produced `.mfp`
is an unambiguous oracle, so it calibrates fastest) → `memory` (10, the
densest per line) → `stdlib` (19, largest and carrying the suspect-claim
backlog, last with pace known).

**Risk concentration:**
- **Adjudicating the deletion class by rewriting from current code.** The
  temptation, on finding that `lower_io_write_helper` no longer exists, is to
  find today's equivalent and re-word the claim around it. That is authoring a
  new contract inside a verification pass. The rule: establish what the
  contract *is* from the code, and if the spec's claim was about something
  that no longer exists, cut it and record the cut — a replacement claim is a
  deliberate act, made explicitly.
- **Verifying a byte-format claim against the text's own consistency.** Held
  by producing a real `.mfp` with the current compiler and reading it. Memory
  `committed-mfp-goes-stale-on-resource-requalification` is the standing
  warning that a checked-in `.mfp` is not a valid oracle — build one.
- **`memory` claims that are per-architecture.** A layout or ABI claim true on
  AArch64 and false on x86-64 must state which; `.ai/arch-abi.md` is the map
  of where those differences live, and the code is the oracle.
- **`stdlib` claims about MFBASIC-implemented packages** whose man pages the
  plan just verified page by page. A disagreement between the certified man
  page and the spec is a high-confidence finding — the man page was probe-
  verified, the spec claim never has been.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work; `- [~]` for partial; `- [x] ~~text~~ — moot: <evidence>`
> rather than deleting; fill `Commit:` on landing. **Unticked means NOT DONE.**

### Phase 1 — package (16 units)

- [ ] Build a real `.mfp` with the current release binary and use it as the
      oracle; record how it was produced.
- [ ] All 16 files, one review unit each; every offset, field order, magic
      number and section claim checked against that `.mfp`.
- [ ] Four-bucket triage; per-file bucket counts recorded.
- [ ] A description that disagrees with a correct format is fixed here; a
      format that disagrees with a correct description is a `write-bug`.

Acceptance: 16 units `exit 0`; every byte-level claim in the ledger names the
`.mfp` bytes that confirmed it; `--citations package` → 0 `MISS-*`; `--links`
clean; cargo gates green.
Commit: —

### Phase 2 — memory (10 units)

- [ ] All 10 files, one review unit each.
- [ ] Every layout, calling-convention and runtime-helper-ABI claim verified
      at its cited symbol; every per-architecture claim states its
      architecture.
- [ ] **Adjudicate `07_runtime-helper-abi.md`'s `lower_io_write_helper`
      claim** — the known deletion-class case — and record the verdict and the
      evidence.
- [ ] Fill the `belongs-in-spec` gaps letter I attributed to `memory`.
- [ ] Reconcile against `.ai/collections.md` and `.ai/resources-packages.md`;
      any disagreement fixed in both, in the same commit.

Acceptance: 10 units `exit 0`; the `lower_io_write_helper` claim has a
recorded verdict with evidence; every per-architecture claim names its
architecture; `--citations memory` → 0 `MISS-*`; cargo gates green.
Commit: —

### Phase 3 — stdlib (19 units)

- [ ] All 19 files, one review unit each.
- [ ] **Resolve every deletion-class suspect claim** from plan-125-I's list
      attributed to `stdlib` — the largest concentration in the plan.
- [ ] Cross-check every `stdlib` model claim against the corresponding
      certified man pages (`http`, `net`, `datetime`, `json`, `csv`, `regex`);
      a disagreement is a finding, and the man page's probe evidence is on the
      record from letters C–F.
- [ ] Four-bucket triage; per-file bucket counts recorded.

Acceptance: 19 units `exit 0`; every suspect claim resolved with evidence;
`--citations stdlib` → 0 `MISS-*`; `--links stdlib` clean; `--reconcile`
exits 0 over the whole 45-unit batch; the bucket table covers all 45 files;
`cargo build` and `cargo test --bin mfb spec` green; `mfb spec stdlib --all`
renders with no leaked `[[`.
Commit: —

## Validation Plan

- Tests: `cargo build` and `cargo test --bin mfb spec` at the end of every
  phase; a `write-bug` fix landing here brings its own gates.
- Coverage check: `--reconcile` over the 45-unit list; the bucket table
  reconciled to 45 files, 0 unaccounted; the suspect-claim list reconciled to
  0 unresolved.
- Runtime proof: a real `.mfp` produced and read for Phase 1;
  `mfb spec <pkg> --all` renders for all three with no leaked `[[`.
- Doc sync: disagreements with `.ai/collections.md` /
  `.ai/resources-packages.md` fixed in both surfaces in the same commit.
- Acceptance: 0 `MISS-*` for all three, `--links` clean, cargo gates green,
  `--reconcile` 0.

## Open Decisions

- **What replaces a deleted contract in `memory`?** — Recommend: cut the
  claim, record the cut, and open the question of whether the *current*
  contract needs specifying as an explicit task rather than writing it inline
  during a verification pass. `.ai/specifications.md` prefers an accurate stub
  over a wrong topic, and a hastily reconstructed ABI claim is the worst of
  both.
- **Should `stdlib` cite the split `helper_*.rs` files or the package
  overview?** — Recommend citing the symbol wherever it actually lives, per
  `.ai/specifications.md` ("symbol-preferred"), and accepting that this makes
  the citations churn with future splits. The alternative — citing the
  directory — is stable and unfalsifiable, which is how the rot went unnoticed
  for so long.

## Corrections

<!-- Filled in DURING execution. -->

## Summary

`stdlib` carried 69% of the spec's citation rot from one code motion, so the
real work of this letter is not re-pointing links — letter I did that — but
deciding which of the claims built on the vanished symbols were ever true.
`package` is the cheapest and most decisive verification in the whole spec
track because a produced `.mfp` is an unambiguous oracle; `memory` is the most
consequential, because it is where the man surface sent everything it was not
allowed to say.
