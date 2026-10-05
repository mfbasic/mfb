# plan-171-E: If-conversion — forming selects from branch diamonds

Last updated: 2026-10-04
Effort: medium (1h–2h)
Depends on: plan-171-D (the `cset`/`csel` ops and their optimizer knowledge)

plan-171-D gives the compiler conditional set/select and converts the builders
it knows about. User code still writes selects as statements —
`IF a < b THEN m = a ELSE m = b`, `LET s = 0 : IF x THEN s = 1` — and those reach
the selected stream as diamonds and triangles. This sub-plan lands the
`planning/optimizations.md` rows "If-conversion (predication)" and
"Select/cmov formation" (both Level 3, stage MIR, both currently blocked on the
missing primitive D supplies): a MIR row that rewrites a diamond or triangle
whose arms are single pure moves into `cmp; csel`/`cset`.

Outcome: at `-O3` and above, on AArch64 and x86-64, such a diamond compiles to a
compare and one select with no branch; at `-O2` and below, and on riscv64 at any
level, emitted code is unchanged; outputs are unchanged everywhere.

References: plan-171-D; `src/optimizer/opt2/mod.rs:optimize_mir` (pipeline
order), `simplifycfg.rs`, `merge.rs`, `uce.rs` (label census),
`branches.rs`; `src/optimizer/catalog.rs`; `planning/optimizations.md` rows
above; `.ai/codegen-invariants.md` "A pass belongs on the `-O` dial only if it
can't change a value or a trap".

## Prerequisites

See plan-171-A §Prerequisites. In addition: plan-171-D complete
(`ls planning/plan-171-D-* 2>/dev/null` → no match).

## 1. Goal

- Row "If-conversion" (level 3, stage MIR), in `src/optimizer/opt2/ifconv.rs`,
  enabled where `Backend::has_conditional_select()` (new; AArch64 and x86-64
  `true`, riscv64 `false`).
- Shapes (post-selection AArch64/x86 stream, vregs):
  - diamond: `cmp…; b.cc Lt; X: mov|mov_imm d, f; b Lj; Lt: mov|mov_imm d, t; Lj:`
  - triangle: `mov|mov_imm d, f; cmp…; b.cc Lj; mov|mov_imm d, t; Lj:`
  - 0/1 arms (`mov_imm 0`/`mov_imm 1`) produce `cset` instead of `csel`.
- Conditions: `Lt`/`Lj` referenced only by this shape (UCE's census), arms are
  exactly one `mov` of a register or `mov_imm` (no load, store, call, trap,
  flag write), both arms define the same vreg `d`, flags dead at `Lj`
  (B's `flagslive`).
- Rewrite: hoist each `mov_imm` arm into a fresh vreg before the `cmp` (flag
  preserving), then `cmp; csel d, t, f, cc` (or `cset`).

### Non-goals

- No conversion of arms with more than one instruction, or with any memory,
  call, or trapping op (checked arithmetic is `adds`/`subs` and is excluded).
- Not on riscv64 (the select there is seven instructions — plan-171-D §4.3 — and
  replacing a two-way branch with it is not a win for predictable branches).
- Not at `-O0`–`-O2` (Level 3 row per `planning/optimizations.md`).

## 2. Current State

- After selection, a fused `br_cc` is `cmp; b.cc` again on AArch64/x86
  (`select_aarch64`); the opt2 rows run on that stream before allocation
  (`optimize_mir`), with vregs, labels, and the SSA overlay
  (`opt2/plans/ssa.rs`).
- How often the shapes occur in the corpus: UNMEASURED (Phase 1, census pattern
  "two-move diamond / triangle").
- `simplifycfg` and `merge` already delete branch-to-next and fuse blocks; run
  after this row they clean up the stranded labels.

### Verified properties

- *Every row self-guards on its own catalog level* — read `optimize_mir`'s doc
  and body ("self-guarding stays inside each pass").
- *Moving a `mov_imm` before a `cmp` preserves flags* — `flag_preserving` lists
  immediate materialization (`opt2/plans/mark.rs`).

## 3. Design Overview

One pattern-matching row, placed after `branches::fold_branches` and before
`sink`/`threading`/`simplifycfg`/`merge` so those rows clean up after it.
Correctness risk: misidentifying a label as private (another branch also jumps
to `Lj` — then `d` might be defined differently on that path; the row requires
`Lj`'s only predecessors to be the two arms) and a live-flags join. Gate class:
behavior-changing at `-O3+` only; expected golden diffs: only goldens built at
`-O3+` (most goldens build at the default `-O1` — Phase 1 lists which fixtures
set a level; if none, no golden moves and the proof is `rt_codegen_forms` +
`rt_regalloc_levels_agree`).

Rejected: running on NIR (Opt1) — the structured `IF` there does not yet know
which arms lower to single moves; the MIR row sees the final shape.

## Phases

> **NOTE — keep the checkboxes current** (same rules as plan-171-A).

### Phase 1 — measure

- [ ] Census (aarch64, x86) at `-O3` of the corpus: diamonds/triangles with
      single-move arms; fixtures that build at `-O3+`
      (`rg -l '"opt"|-O3|optLevel' tests --glob 'project.json'`); record in §2.

Acceptance: §2 has no UNMEASURED. Check: `grep -c UNMEASURED planning/plan-171-E-*.md` → 0 (est. 15 min).
Commit: —

### Phase 2 — the row

- [ ] `Backend::has_conditional_select` (mir.rs trait + three impls).
- [ ] `src/optimizer/opt2/ifconv.rs` (§1 shapes and conditions); wire into
      `optimize_mir` after `branches`; catalog row + stats counter.
- [ ] Unit tests: diamond, triangle, 0/1 → `cset`, inverted condition order,
      shared join label (not converted), arm with a load (not converted), arm
      with `adds` (not converted), flags live at join (not converted), riscv64
      backend (not converted), `-O2` (not converted).
- [ ] Fixture `tests/rt-behavior/control-flow/if-select-shapes` built at `-O3`
      (its `project.json` sets the level): min/max/sign/flag patterns written as
      statements, both edges taken, printing results.
- [ ] `rt_codegen_forms`: the fixture's aarch64/x86 dump at `-O3` has `csel`/
      `cset` and no `b.` in the converted function; at `-O1` it has the branch.
- [ ] `rt_regalloc_levels_agree`: add the fixture (all levels agree).
- [ ] Golden re-baseline only if Phase 1 found `-O3` goldens.
- [ ] Runtime: macOS acceptance; 2223 and 2227 `linux-runtime-proof.sh` for the
      fixture (`FILTER=if-select-shapes`); 2230 Windows run.
- [ ] Benchmark (macOS host): `run.sh 10` compares `mfb-O3` rows against
      `benchmark/baseline/mfb-O3.log`.

Acceptance: converted at `-O3` on AArch64/x86 only; every level and target prints the same.
  Check: `cargo test --bin mfb opt2::ifconv` → pass (est. 2 min); `cargo test --release --test rt_regalloc_levels_agree` → pass (est. 6 min).
Commit: —

### Phase 3 — review sweep, docs

- [ ] Census diff at `-O3`; for remaining two-move diamonds, record why
      (multi-instruction arm, shared join, live flags).
- [ ] `planning/optimizations.md` rows "If-conversion", "Select/cmov formation" →
      `Y` with landing notes; `mfb man optimizations` shows the row.
- [ ] `mfb spec architecture mir-instruction-set` (or the optimizer's owning topic
      named by `mfb spec architecture --all | grep -n 'optimize_mir'`): the row.

Acceptance: docs render. Check: `target/release/mfb man optimizations | grep -c 'If-conversion'` → 1 (est. 1 min).
Commit: —

## Validation Plan

- Tests: `ifconv` units, fixture, `rt_codegen_forms`, levels-agree.
- Runtime proof: Phase 2's targets (riscv64 unaffected — row disabled; its
  goldens must not move).
- Final gate (once): full `cargo test --no-fail-fast` to a log; `EXIT=0` (est. 60–90 min).

## Open Decisions

1. Level — **recommend 3**, as `planning/optimizations.md` classifies it, vs. 2
   (it is behavior-preserving; but the table's level is the project's existing
   decision and this plan does not re-litigate it).

## Corrections

<Filled in during execution.>

## Summary

Small, contained row; the risk is the private-join proof, covered by negative
unit tests. riscv64 is deliberately excluded.
