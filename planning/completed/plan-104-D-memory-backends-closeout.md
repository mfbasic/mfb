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
| plan-104-C complete | builtins census at 0/annotated; C's phases all `[x]` | MET 2026-08-24 (25 annotated survivors classified in C; C commits 171879971 / 73b2ca8db) |

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

- [x] Convert `src/codegen/cleanup` + `src/codegen/error` (14 reads) +
      `src/codegen/link`'s single scalar-compare file. (The reads became typed
      reads via C's `ValueResult` flip; link's 2 surviving compares are on
      `IrLinkFunction.return_type` — the LINK wire string, annotated.)
- [x] Convert `src/codegen/collection` (68 reads) — typed reads post-C; the
      in-place/compare/search paths' parse-shaped compares inverted to `name()`
      renders; the ForEach chain head structural via typed twins. See the
      Corrections **nominal-domain determination** for the layout web's `&str`
      params (name-domain sinks, not shims).
- [x] Convert `src/codegen/memory` (30 reads) — typed reads post-C;
      `LocalBinding`-class stores typed; the value-semantics/arena walk helpers'
      `types` maps typed (B); layout/width decision inputs unchanged, proven by
      the full gate (0 diffs).
- [x] Tests: unit fixtures updated with the sweeps (`cargo check --all-targets`
      0 errors/warnings).

Acceptance: `cargo test --no-fail-fast` green (0 FAILED); `artifact-gate all`
no NEW diff vs baseline (0 diffs).
Commit: f329cc642

### Phase 2 — backends (`src/target/`) native

- [x] Convert the backends' 16 reads / 11 scalar compares / 3 structural tests.
      (**Corrected census**: all 16 `src/target` `.type_` reads live in the
      SHARED layers — nir/plan/validate — not the per-arch emitters
      (`rg -c '\.type_\b' src/target/` lists only shared files; the per-arch
      backends read type facts via `StorageType`/`ProgramEntrySpec` only).
      The shared layers converted: `storage_for_type`/`is_reference_type`
      variant-matched, validate walkers structural, `LocalBinding` typed.
      Final target censuses: 1 scalar compare (the `Named("Scalar")` guard —
      itself now a variant match on the rendered nominal) and 0 structural
      tests. The `ProgramEntrySpec.language_entry_returns: &str` field stays a
      string seam into the per-arch emitters, annotated.)
- [x] Tests: backend unit fixtures compile untouched.

Acceptance: `cargo test --no-fail-fast` green (0 FAILED); `artifact-gate all`
no NEW diff (0 diffs).
Commit: f329cc642

### Phase 3 — closeout: census, perf, docs

- [x] Re-run the plan-104-A §2 censuses; record the before→after table in this
      file with every survivor annotated.

**Census before → after (commands = plan-104-A §2's, re-run 2026-08-24):**

| Census | A baseline | final | survivors are |
|---|---|---|---|
| codegen scalar `== "Integer"`-style compares | 80 | 45 | builtins 25 (mangled `$T` symbol fragments ~7; `general`'s bespoke string resolver 4; `String` locals derived through the nominal layout web ~14) · engine 12 (`&str`-param helpers, `result_type` chains merging builtins' `Option<&str>` args, `numeric` string-algorithm internals) · collection 3 + memory 3 (nominal-web internals) · link 2 (`IrLinkFunction.return_type` wire strings) |
| codegen structural `strip_prefix("List OF ")`-style tests | 41 | 33 | engine 14 (11 = `type_utils`' string vocabulary retained for the still-string front/middle-end callers + the `&str`-param validate/oracle walks) · builtins 7 (mangled fragments + `general` resolver) · cleanup 4 + collection 3 + memory 5 (nominal-web internals) |
| codegen `format!("List OF …")` type builds | 23 | 20 | all inside the collection-lowering string plumbing feeding `CollectionTypeLayout::from_type` (nominal web) or mangled `Pair$A$B` names |
| `src/target/` scalar compares / structural tests | 11 / 3 | 1 / 0 | the 1 is `storage_for_type`'s `Named("Scalar")` **variant-match guard** (not a string compare on a store) |
| registry boundary parses serving typed callers | 2 | 0 | `resolve_call`'s parse serves only the string wrapper; codegen resolves via `resolve_call_typed` |
| transitional `ParameterType::parse` sites in codegen | ~8 (pre-existing registry/descriptor parses) | 117 | construction-site parses of strings produced inside the nominal web (behavior-safe by `parse∘name = id`; zero same-line `name()`→`parse` round-trips — grep 0); retired as the web's callers go typed (plan-106-E's terminal no-strings census owns the end state) |

**The one load-bearing determination** (also in `.ai/codegen-invariants.md`):
the layout/value-semantics classification web operates over the NOMINAL NAME
domain — name-keyed `TypeModel`, variant-name recursion, `X STATE Y`
composites — so its `&str` params are name-domain sinks (like symbol tables),
entered by a single render from typed values. Converting it to `ParameterType`
would ADD conversions (interning every recursion step) rather than remove
string ops.
- [x] Run `bash scripts/bench-lowering.sh` ×3 (medians per the Open Decision;
      the script takes no binary arg — A's Correction); every probe ≤ baseline,
      most FASTER:

| Probe (debug/release s) | baseline | run medians | delta (release) |
|---|---|---|---|
| trivial | 0.65 / 0.35 | 0.65 / 0.36 | +0.01s (within noise) |
| one-regex | 30.72 / 6.74 | 30.10 / 6.62 | **−1.8%** |
| acceptance | 276.06 / 49.85 | 271.86 / 48.23 | **−3.3%** |

      (Raw runs: trivial 0.65|0.65|0.66 / 0.35|0.36|0.36; one-regex
      30.01|30.11|30.10 / 6.62|6.63|6.57; acceptance 272.08|271.86|269.98 /
      48.23|48.59|47.41.)
- [x] Doc sync: `.ai/codegen-invariants.md` gained the "Types below the AST are
      ParameterType, with a nominal-name string boundary" invariant section;
      `.ai/collections.md`/`.ai/compiler.md` scanned — no string-type-test
      claims to fix (`rg` found none); `13_native-ir.md` reviewed — one
      sentence corrected (`Global`'s empty `type_` is an empty *type*, rendered
      as an empty string in the dump); serialized JSON unchanged.
- [x] Tests: full suite green (0 FAILED) + `test-accept` 2 mismatches = the
      documented pre-existing pair (stdin-EOF acceptance sub-tests at log:2 +
      the `project_name` path-corruption harness bug at log:876) — no NEW
      mismatch.

Acceptance: censuses recorded with annotations; perf numbers recorded and ≤
baseline; docs updated; `cargo test --no-fail-fast` green; `artifact-gate all`
no NEW diff; `test-accept` no NEW mismatch; fmt both crates.
Commit: f438de0f1

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

- **Perf-noise threshold for the bench comparison.** RESOLVED as recommended:
  three runs per probe, median comparison; no probe regressed >3% (the only
  non-improvement is trivial-release +0.01s on a 0.35s probe). (§3)

## Corrections

- **The "backends" census pointed at the shared layers, not the per-arch
  emitters.** All 16 `src/target/` `.type_` reads live under
  `src/target/shared/{nir,plan,validate}` (`rg -c '\.type_\b' src/target/`);
  the per-arch backends (`linux_common`/`macos_aarch64`/`win_x86_64`) have
  zero — they consume type facts only through `StorageType` and
  `ProgramEntrySpec.language_entry_returns` (`&str`, an annotated seam). The
  win_x86_64-ripple warning in Phase 2's text was therefore moot: no per-arch
  file changed.
- **`bash scripts/bench-lowering.sh` takes no binary argument** (inherited
  from A's Correction; the Phase-3 command spelling
  `scripts/bench-lowering.sh target/release/mfb` is ignored argv).
- **Nominal-domain determination** (the load-bearing scope call, §Phase 1 and
  `.ai/codegen-invariants.md`): the layout/value-semantics classification web
  (`type_is_flat` family, `CollectionTypeLayout::from_type`, payload emitters)
  is a NOMINAL-NAME-domain subsystem — name-keyed `TypeModel`, variant-name
  recursion, `X STATE Y` composites. Its `&str` params are name-domain sinks
  entered by one render from typed values; a `ParameterType` conversion would
  intern/parse at every recursion step, ADDING conversions. The 117
  transitional `ParameterType::parse` construction sites inside that plumbing
  are behavior-safe (`parse∘name = id`, zero same-line round-trips) and are
  the measured hand-off to plan-106-E's terminal no-strings census.

## Summary

The highest-blast-radius consumer (memory layout) lands last behind everything
else, and the feature closes with measured proof of its own premise: the
string-op censuses drop to annotated boundaries and lowering is no slower than
the pre-flip baseline. Left untouched by design: the registry's string wrappers
for front-end callers, error-message text, and symbol mangling — each an
annotated deliberate boundary, not a leftover.
