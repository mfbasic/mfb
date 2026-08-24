# plan-104-D: Memory/collections/backends native + feature closeout

Last updated: 2026-08-24
Effort: large (3h–1d)
Depends on: plan-104-C (engine + builtins are native; only the smaller consumer
trees and the closeout remain).

Convert the remaining NIR type consumers — `src/codegen/memory` (30 reads),
`src/codegen/collection` (68), `cleanup`/`error` (14), and the backends under
`src/target/` (16 reads, 11 scalar compares, 3 structural tests) — to native
`ParameterType`, then close the feature out: re-run the string-op censuses and
record the deltas, prove lowering performance against the plan-104 baseline, and
sync the `.ai`/spec docs.

See plan-104-A §3 for the layering and the shared byte-identity gate; plan-104-A
§Prerequisites for the shared gate.

References:

- `src/codegen/memory/` — record layout / inline-field sizing (type-driven
  width decisions; see `.ai/codegen-invariants.md` record-layout invariants).
- `src/codegen/collection/`, `src/codegen/cleanup/`, `src/codegen/error/`.
- `src/target/` backends (per-arch code emitters; `.ai/arch-abi.md`).
- `.ai/codegen-invariants.md`, `.ai/collections.md`, `.ai/compiler.md` — the
  docs that describe codegen's type handling and must reflect the typed NIR.
- `src/docs/spec/architecture/13_native-ir.md` — confirm it describes only the
  serialized JSON (unchanged); update any sentence asserting in-memory `String`
  type fields.

## Prerequisites

See plan-104-A §Prerequisites. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-104-C complete | builtins census at 0/annotated; C's phases all `[x]` | NOT MET until C lands |

## 1. Goal

- `memory`/`collection`/`cleanup`/`error` (112 `.type_` reads total) and
  `src/target/` (16 reads) operate natively on `ParameterType`; the codegen-wide
  scalar-compare / structural-test / `format!` censuses from plan-104-A §2 drop
  to 0 or annotated deliberate boundaries, with the final numbers recorded here.
- Lowering wall-clock is **not slower than the plan-104 baseline**
  (`scripts/bench-lowering.sh` three probes vs
  `planning/plan-104-bench-baseline.txt`).
- `.ai` docs and the NIR spec chapter reflect the typed NIR.

### Non-goals (explicit constraints)

- No change to compiled output — byte-identical vs the plan-104 baseline
  (memory-layout code is the highest-blast-radius consumer left; the gate is
  the proof that width/size decisions are unchanged).
- No performance *tuning* beyond removing the string churn this feature
  targets — `correctness over performance` stands; if a probe regresses, find
  the specific remaining shim/allocation, don't restructure algorithms.
- Backends' instruction selection/ABI logic is untouched — only how they read
  type facts changes.

## 2. Current State

Post-C, the remaining shimmed consumers are the smaller trees:

### Measured populations

| What | Count | Command |
|---|---|---|
| memory `.type_` reads | 30 | `rg -c '\.type_\b' src/codegen/memory` → 30 |
| collection `.type_` reads | 68 | `rg -c '\.type_\b' src/codegen/collection` → 68 |
| cleanup + error `.type_` reads | 14 | `rg -c '\.type_\b' src/codegen/cleanup src/codegen/error` → 5 + 9 |
| `src/target/` `.type_` reads | 16 | `rg -c '\.type_\b' src/target/ \| awk …` → 16 |
| `src/target/` scalar compares / structural tests | 11 / 3 | plan-104-A census |
| memory/link/error/collection/cleanup scalar-compare files | 8 | plan-104-A census (4 memory + 1 each link/error/collection/cleanup) |

### Verified properties

- **Memory layout is type-driven** — inline field sizes and record strides come
  from type facts (`.ai/codegen-invariants.md` record-layout section;
  bug-430/inline-headroom history). A conversion mistake here changes emitted
  layout bytes, which the gate catches per-fixture. VERIFIED context (docs +
  history), which is why this tree lands LAST behind everything else.

## 3. Design Overview

Same compile-driven native conversion as B/C, tree by tree, smallest risk
first: `cleanup`/`error` → `collection` → `memory` → `src/target/` backends.
Then the closeout:

1. **Census re-run** — plan-104-A §2's commands over the final tree; record the
   before→after table here (scalar compares 80→N, structural 41→N, `format!`
   23→N, boundary parses). Annotate every survivor with why it stays (error
   text, symbol mangling, registry string wrappers for non-codegen callers).
2. **Perf proof** — `bench-lowering.sh` three probes vs the baseline file;
   requirement: each probe ≤ baseline within run noise (record the numbers, not
   an adjective).
3. **Doc sync** — `.ai/codegen-invariants.md` (NIR type fields are
   `ParameterType`; the string boundaries that remain), `.ai/collections.md` /
   `.ai/compiler.md` where they describe string type tests in codegen;
   `13_native-ir.md` reviewed (serialized JSON unchanged — fix only sentences
   claiming in-memory strings, if any).

### Rejected alternatives

- **Fold the closeout into C.** Rejected: the census/perf/doc closeout must run
  over the FINAL tree; running it before the last consumer converts would
  record numbers the next commit invalidates.

## Compatibility / Format Impact

None externally observable.

## Phases

### Phase 1 — cleanup/error + collection + memory native

- [ ] Convert `src/codegen/cleanup` + `src/codegen/error` (14 reads) +
      `src/codegen/link`'s single scalar-compare file (0 `.type_` reads there —
      `rg -c '\.type_\b' src/codegen/link` → 0 — but 1 compare file per the
      plan-104-A census).
- [ ] Convert `src/codegen/collection` (68 reads) — the HOF-rewrite/in-place
      mutation paths per `.ai/collections.md`; scoped gate on the collections
      byte-identity fixtures after.
- [ ] Convert `src/codegen/memory` (30 reads) — layout/width decisions; run the
      full gate immediately after this tree alone.
- [ ] Tests: unit fixtures in these trees updated.

Acceptance: `cargo test --no-fail-fast` green; `artifact-gate all` no NEW diff
vs baseline.
Commit: —

### Phase 2 — backends (`src/target/`) native

- [ ] Convert the backends' 16 reads / 11 scalar compares / 3 structural tests
      (per-arch emitters; `win_x86_64` changes ripple all io-importer
      windows ncodesums — but a byte-identical conversion ripples nothing; the
      gate proves it).
- [ ] Tests: backend unit fixtures updated.

Acceptance: `cargo test --no-fail-fast` green; `artifact-gate all` no NEW diff.
Commit: —

### Phase 3 — closeout: census, perf, docs

- [ ] Re-run the plan-104-A §2 censuses; record the before→after table in this
      file with every survivor annotated.
- [ ] Run `scripts/bench-lowering.sh target/release/mfb`; record the three probe
      times next to the baseline; each ≤ baseline within noise. If a probe
      regressed, find the responsible residual shim/allocation and fix it (this
      is a task, not a stop).
- [ ] Doc sync: `.ai/codegen-invariants.md`, `.ai/collections.md`,
      `.ai/compiler.md`; review `13_native-ir.md`.
- [ ] Tests: full suite + `test-accept` (no NEW mismatch beyond the 2 documented
      pre-existing).

Acceptance: censuses recorded with annotations; perf numbers recorded and ≤
baseline; docs updated; `cargo test --no-fail-fast` green; `artifact-gate all`
no NEW diff; `test-accept` no NEW mismatch; fmt both crates.
Commit: —

## Validation Plan

- Tests: per-tree unit suites; full integration suite; `test-accept`.
- Coverage check: collections/memory/backends all have byte-identity fixtures in
  the gate corpus (1,718 goldens at plan-writing time).
- Runtime proof: `artifact-gate all` byte-identical; `bench-lowering.sh` numbers
  vs baseline.
- Doc sync: as Phase 3.
- Acceptance: `cargo test --no-fail-fast`; `artifact-gate all`; `test-accept`;
  `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`.

## Open Decisions

- **Perf-noise threshold for the bench comparison.** Recommend three runs per
  probe, compare medians, flag > 3% regressions for investigation. (§3)

## Corrections

<Filled in during execution.>

## Summary

The highest-blast-radius consumer (memory layout) lands last behind everything
else, and the feature closes with measured proof of its own premise: the
string-op censuses drop to annotated boundaries and lowering is no slower than
the pre-flip baseline. Left untouched by design: the registry's string wrappers
for front-end callers, error-message text, and symbol mangling — each an
annotated deliberate boundary, not a leftover.
