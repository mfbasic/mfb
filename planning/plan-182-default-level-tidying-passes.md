# plan-182: Move landed tidying passes to their retargeted dial levels

Last updated: 2026-10-24
Effort: medium (1h–2h of edits; the golden gates add their own runtime)

The `planning/optimizations.md` level review (2026-10-24) retargeted six landed
rows. This plan moves their code gates to match. After it, a default `-O1` build
runs unreachable-code elimination, branch simplification/folding, basic block
merging, CFG simplification and the NIR half of DCE, and `-O2` runs check fusion.
No program's behavior changes; only emitted code does.

References:

- `planning/optimizations.md` — the rows, and "Reclassifications flagged by review" (the why for each move)
- `src/optimizer/catalog.rs` — landed-row levels; renders `mfb man optimizations`
- `.ai/testing-gates.md` — artifact-gate, `MFB_OPT`, golden regeneration

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| The corpus is behavior-clean at `-O2`, where every moved pass already runs | `MFB_OPT=2 bash scripts/test-accept.sh target/release/mfb /tmp/p182-prereq-out` → exit 0 | MET 2026-10-25 on main `2f281c5e7`: `acceptance tests passed (1756 test(s) ran, 73 level-variant golden(s) skipped at -O2)`, exit 0. Baseline `scripts/artifact-gate.sh <main mfb> all` → `1721 tests, 1900 build(s), 2384 golden(s) checked, 0 diff(s)` |

## 1. Goal

- `mfb build -v` at the default level shows non-zero fire counts for the four
  control-flow rows and NIR DCE on a fixture that has dead branches; `-O2` shows
  check fusion firing.
- Every `.run` and `build.log` golden is unchanged.

### Non-goals

- No pass logic changes. Only `level_enabled(N)` arguments, catalog rows, one new
  stats counter, and docs.
- The MIR half of DCE stays Level 2 (it reads the SSA overlay).
- CFG simplification's unlanded parts (hoist common code, two-entry-phi→select)
  are not built here; they gate at Level 2 when they land.
- Dead global elimination stays Level 2 (it shares its gate with constification,
  which needed bug-747; and it fires 0 times today).

## 2. Current state (read 2026-10-24)

| Row | Gate today | File | Target |
|---|---|---|---|
| Unreachable code elimination | `level_enabled(2)` | `src/optimizer/opt1/uce.rs`, `src/optimizer/opt2/uce.rs` | 1 |
| Branch simplification / folding | `level_enabled(2)` | `src/optimizer/opt1/branches.rs`, `src/optimizer/opt2/branches.rs` | 1 |
| Basic block merging | `level_enabled(2)` | `src/optimizer/opt2/merge.rs` | 1 |
| CFG simplification | `level_enabled(2)` | `src/optimizer/opt2/simplifycfg.rs` | 1 |
| DCE, NIR half | `level_enabled(2)` | `src/optimizer/opt1/dce.rs` | 1 |
| DCE, MIR half | `level_enabled(2)` | `src/optimizer/opt2/dce.rs` | 2 (unchanged) |
| Check fusion (= compare elimination / flag reuse) | `level_enabled(3)` | `src/optimizer/opt2/flags.rs` | 2 |

Both DCE halves report through one counter, `stats::count_dead_code_eliminations`
(`src/optimizer/stats.rs`, `DEAD_CODE_ELIMINATION`), under one catalog row.

Goldens that can move (they are emitted after the NIR optimizer,
`src/target/shared/lower.rs:optimize_nir`): `.nir` 23, `.nplan` 23, `.nobj` 18,
`.ncode` 7, `.mir` 2, `.ncodesum` 157
(`git ls-files 'tests/**/golden/*' | grep -cE '\.<kind>$'`). `.ir`/`.ast` are pre-NIR
and cannot move. `.run` (917) and `build.log` must not move.

## 3. Design

Change the gate argument, the catalog `level`, and nothing else. Split DCE's
catalog row in two so the man page and `-v` counts tell the halves apart:
`Dead-code elimination (DCE, NIR)` at 1 with a new `DEAD_CODE_ELIMINATION_NIR`
counter, and `Dead-code elimination (DCE, MIR)` at 2 keeping the existing counter.

Correctness risk: none in logic (each pass already runs at `-O2` over the whole
corpus, which is the prerequisite). The work is golden churn. Byte-identity is not
the gate for phases 2–3: codegen goldens are EXPECTED to move; `.run`/`build.log`
are the behavior gate.

## Phases

### Phase 1 — Check fusion 3 → 2 (no default-build change)

- [x] `src/optimizer/opt2/flags.rs:fuse`: `level_enabled(3)` → `level_enabled(2)`; update the module doc ("Level-3" → "Level-2"). `cargo test --release --bin mfb optimizer::opt2::flags` → 5 passed.
- [x] `src/optimizer/catalog.rs`: "Check fusion with existing comparisons" `level: 3` → `2`.
- [x] (added) `opt2/flags.rs` test `level_two_disables_the_row` pinned the old level (off at 2); now `level_one_disables_the_row` (off at 1). See Corrections.
- [x] (added) `planning/optimizations.md`: Check fusion + Compare elimination rows marked "moved to Level 2 by plan-182 (phase 1)".

Acceptance: default-level goldens untouched; check fusion fires at `-O2`.
  Check: `scripts/artifact-gate.sh target/release/mfb collections` → 0 diffs (est. 2 s); `mfb build -v -O2 examples/ai_chat` → "Check fusion" count > 0 (est. 10 s). (Fixture corrected from `tests/byte-identity/collections`, see Corrections.)
  Result: gate → `1 tests, 6 build(s), 7 golden(s) checked, 0 diff(s)`; `mfb build -v -O2 examples/ai_chat` → `Check fusion with existing comparisons: 8` (the main binary at `-O2` prints no Check fusion line — row not enabled; at `-O3` both print 8).
Commit: —

### Phase 2 — NIR DCE to Level 1

- [ ] `src/optimizer/stats.rs`: add `DEAD_CODE_ELIMINATION_NIR` and `count_dead_code_eliminations_nir`.
- [ ] `src/optimizer/opt1/dce.rs:eliminate`: `level_enabled(2)` → `level_enabled(1)`; call the new counter.
- [ ] `src/optimizer/catalog.rs`: replace the DCE row with two rows — `Dead-code elimination (DCE, NIR)` (level 1, stage NIR, new counter) and `Dead-code elimination (DCE, MIR)` (level 2, stage MIR, existing counter). Keep pipeline order.
- [ ] Regenerate the codegen goldens the full artifact gate flags (procedure in `.ai/testing-gates.md`, "Coverage & the `.ncodesum` goldens").

Acceptance: `.run`/`build.log` unchanged; only native-dump kinds moved.
  Check: `git status --porcelain | grep -vE '\.(nir|nplan|nobj|ncode|ncodesum|mir)$'` → only source files of this phase (est. 1 s).
Commit: —

### Phase 3 — Control-flow tidying rows to Level 1

- [ ] `level_enabled(2)` → `level_enabled(1)` in `opt1/uce.rs`, `opt2/uce.rs`, `opt1/branches.rs`, `opt2/branches.rs`, `opt2/merge.rs`, `opt2/simplifycfg.rs`.
- [ ] `src/optimizer/catalog.rs`: those four rows `level: 2` → `1`.
- [ ] Regenerate the codegen goldens the full artifact gate flags.

Acceptance: same as Phase 2.
  Check: same command as Phase 2 (est. 1 s).
Commit: —

## Validation Plan

- Behavior: `bash scripts/test-accept.sh target/release/mfb` (default level) → every `.run` and `build.log` matches; `MFB_OPT=0` and `MFB_OPT=2` runs exit 0.
- Codegen: `scripts/artifact-gate.sh target/release/mfb all` → green after regeneration (~15–20 min, `.ai/testing-gates.md`).
- Landing tier: `bash scripts/test-fast.sh` plus `cargo test --bin mfb builtins_codegen::optimizer::` (the optimizer sweeps, since levels moved).
- Doc sync: `mfb man optimizations` renders the new levels (from `catalog.rs`); `planning/optimizations.md` already carries the target levels — mark these rows' notes "moved to Level 1 by plan-182" as each phase lands.
- Golden proof (AGENTS.md re-baseline rule): each regenerating commit cites this plan, states the gates above were run in full, and that no `.run`/`build.log` changed.

## Decisions

- Moves and target levels — decided by the owner, 2026-10-24, from the optimizations.md review: default-level exceptions for behavior-preserving rows that need no dataflow (see that doc's reclassification notes).
- One plan file, not split — owner, 2026-10-24.
- Check fusion moves as a whole rather than splitting out compare reuse — they are one mechanism (`opt2/flags.rs`).

## Corrections

- **Prerequisite command was missing an argument.** `scripts/test-accept.sh` requires `<mfb-exe> <actual-output-dir>`; the written command exited 2 with the usage text. Corrected to `MFB_OPT=2 bash scripts/test-accept.sh target/release/mfb /tmp/p182-prereq-out`, run on an unmodified main-tip build (`/tmp/p182-base`).
- **Phase 1's check-fusion fixture never fires check fusion.** `mfb build -v -O3 tests/byte-identity/collections` → `Check fusion with existing comparisons: 0` on both the main and the phase-1 binary, so the check could not pass at any level. `examples/ai_chat` (where `planning/optimizations.md` measured the row) fires 8 at `-O3`; the phase-1 check now uses it at `-O2`.
- **Unit tests pinned each moved row's old level.** Each moved pass has a `level_*_disables_the_row` test asserting the row is off one level below its old gate (`opt2/flags.rs`, `opt1/dce/tests`, `opt1/uce.rs`, `opt1/branches/tests`, `opt2/uce.rs`, `opt2/branches.rs`, `opt2/merge.rs`, `opt2/simplifycfg.rs`). AGENTS.md four questions: (1) written with each row, to pin its catalog gate; (2) protects "the row is off below its catalog level"; (3) nothing else depends (a tree grep of each test name hits only its own file); (4) the catalog level is the owner's decision recorded under Decisions — the old level is what this plan replaces. Each test keeps its intent and moves down one level (off at new level − 1). Tasks added per phase.

## Summary

Seven gate arguments, five catalog levels, one counter split. The risk is golden
churn, held in check by `.run`/`build.log` staying fixed.
