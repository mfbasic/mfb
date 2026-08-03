# MirInstruction Typed-Operand Plan

Last updated: 2026-08-02
Effort: large (3h–1d)
Depends on: plan-78 (A and B complete — the `Operand` type must exist and
`CodeInstruction.fields` must already be `Operand`-typed)

Flip the neutral-MIR instruction's operand representation from `String` to the
typed `Operand` introduced by plan-78, so codegen has **one** operand model end
to end. `MirInstruction` (`src/target/shared/code/mir.rs:28`) is the stringly
twin of `CodeInstruction`: it sits between shared lowering and per-backend
selection (`Backend::select(&[MirInstruction]) -> Vec<CodeInstruction>`,
`mir.rs:539`). After plan-78 makes `CodeInstruction.fields` typed, the MIR
boundary does a **typed → String → typed** round-trip (`lower_to_mir` renders
`Operand` to `String`; `select` parses it back) — this plan removes that
round-trip and the last stringly operand surface.

The single behavioral outcome: `MirInstruction.fields: Vec<(&'static str,
Operand)>`, the compiler builds and all tests pass, and `artifact-gate … all`
(and the `.mir` goldens) are byte-identical.

References:

- plan-78-A (`planning/plan-78-A-operand-type.md`) — the `Operand` type + `render`
  this plan reuses (it does **not** redefine `Operand`).
- plan-78-B (`planning/plan-78-B-flip-storage.md`) — the `CodeInstruction`
  representation flip this plan mirrors.
- `src/target/shared/code/mir.rs` — `MirInstruction`, `lower_to_mir` (:413),
  `rename_field_values` (:504), the `.mir` dump (`ToCodeJson`, :748), the
  fusion/fuse producers (:424,447,479,488,496).

## Prerequisites

This is a precondition on the whole plan, not a dependency to negotiate: **if
plan-78 (A and B) is not complete, plan-79 cannot start, full stop.** plan-79 does
not define `Operand`, and it does not flip `CodeInstruction` — it consumes both as
already-done. plan-78-C (the regalloc migration) is *not* required for plan-79 and
may land before or after it.

| Must be true | Command | Status |
|---|---|---|
| plan-78-A complete (`Operand` + render + corpus test) | `rg -n 'enum Operand' src/target/shared/code/operand.rs` | **MET** (plan-78 merged; `Operand` exists) |
| plan-78-B complete (`CodeInstruction.fields` is `Operand`) | `rg -n 'fields: Vec<\(&.*str, Operand\)>' src/target/shared/code/types.rs` | **MET** |
| plan-82-A/B/C landed on this branch (typed `VirtualRegister`, typed regalloc + producers) | `rg -n 'struct VirtualRegister' src/target/shared/code/operand.rs` and `rg -n 'Operand::phys\(' src/target/shared/code/regalloc/linear_scan.rs` | **MET** (ffea88cb6) |
| Repo builds clean; goldens green | `cargo build --bin mfb && bash scripts/artifact-gate.sh target/release/mfb all` | **MET** (0 diffs at C) |

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run and update before continuing and before stopping. If you stop, report
> the status of *all* prerequisites, not just the one that blocked you.

## 1. Goal

- `MirInstruction.fields: Vec<(&'static str, Operand)>` (`mir.rs:28`).
- `lower_to_mir` (`mir.rs:413`) moves/clones `Operand`s from `CodeInstruction`
  with **no render-to-string**; each backend's `select` reads `Operand`s and
  builds `CodeInstruction` `Operand`s with **no parse-from-string**.
- `rename_field_values` (`mir.rs:504`) rewrites operands by matching the typed
  `Operand` (the ARENA_BASE realization) rather than string rename.
- `.mir` goldens and `artifact-gate … all` are byte-identical.

### Non-goals (explicit constraints)

- **No emitted-byte / golden change.** A `.mir` or `.ncode` diff means a render
  gap — fix `Operand`/the migration, never re-baseline (AGENTS.md).
- **No new `Operand` arms** and **no `CodeInstruction` change** — both are
  plan-78's; plan-79 only touches the MIR twin and its readers.
- **No selection-*algorithm* change.** Each `select` produces the identical
  `CodeInstruction` stream; only how it reads operands changes.

## 2. Current State

`MirInstruction { op: CodeOp, fields: Vec<(&'static str, String)> }` (`mir.rs:28`)
— structurally identical to `CodeInstruction` pre-plan-78. It is:

- **Produced** by `lower_to_mir` (`mir.rs:413`), which clones operand strings out
  of `CodeInstruction` (`:424 branch.fields`, `:447 adrp.fields.clone()`,
  `:496 setter.fields.clone()`, fusion `:479,488 fuse(...)`), and by the riscv
  SIMD path (`riscv64/v128.rs:1614`).
- **Mutated** by `rename_field_values` (`mir.rs:504`), which realizes the neutral
  arena-base name to the ISA register string.
- **Consumed** by each backend's `select` — `aarch64/select.rs`,
  `x86_64/select.rs`, `riscv64/select.rs` (+ `riscv64/v128.rs`) — reading
  `.fields` to emit `CodeInstruction`s (e.g. `mir.rs:1152,1181,1267,1304` in the
  shared fusion/analysis; `:1587,1669` iterate `.fields`).
- **Dumped** to `.mir` via `impl ToCodeJson for MirInstruction` (`mir.rs:748-753`),
  printing each value verbatim.

### Measured populations

| What | Count | Command |
|---|---|---|
| Files mentioning `MirInstruction` | 10 | `grep -rl "MirInstruction" src --include="*.rs" \| wc -l` → mir.rs + arch {aarch64,x86_64,riscv64} {select,backend}.rs + riscv64/{mod,v128}.rs + x86_64/mod.rs |
| `MirInstruction {…}` construction sites | 4 | `grep -rn "MirInstruction *{" src --include="*.rs" \| grep -v "struct \|-> "` → mir.rs:428,445,494; riscv64/v128.rs:1614 |
| `MirInstruction::new` (builder) | 0 | `grep -rn "MirInstruction::new" src --include="*.rs" \| wc -l` (built by struct literal, not a `new`/`field` builder) |
| Per-arch `select` readers | 3 (+v128) | `aarch64/select.rs`, `x86_64/select.rs`, `riscv64/select.rs`, `riscv64/v128.rs` |

### Verified properties

- **plan-78 leaves a typed→String→typed seam here.** After 78-B,
  `CodeInstruction.fields` is `Operand`; `lower_to_mir` clones those into
  `MirInstruction`, which is still `String` — so the clone must render, and each
  `select` must parse back to build `CodeInstruction` `Operand`s. This plan closes
  that seam. (The seam's *cost* is UNMEASURED — see Open Decisions.)
- **`.mir` byte-identity is decided by the rendered value strings**, exactly as
  for `.ncode` (`mir.rs:748` prints values verbatim). plan-78-A's proven `render`
  guarantees identity once the dump renders `Operand`.

## 3. Design Overview

`MirInstruction` gets no new builder — it is built by struct literal from
`CodeInstruction` operands. Because plan-78 already makes those operands
`Operand`, the flip is largely *removing conversions*:

1. **Type flip** — `mir.rs:28` field type → `Operand`; the 4 construction sites
   move/clone `Operand` instead of `String`.
2. **Producers** — `lower_to_mir` and `fuse`/fusion clone `Operand`s directly (no
   render); `rename_field_values` matches/rewrites the `Operand` (the arena-base
   realization becomes a typed `Phys`/`Raw` rewrite).
3. **Consumers** — the 3 `select` readers (+ v128) read operands via the same
   typed accessor pattern plan-78-C uses on `CodeInstruction`; they build
   `CodeInstruction` `Operand`s directly.
4. **Dump** — `ToCodeJson for MirInstruction` renders `Operand` (byte-identical
   `.mir`).

Correctness risk concentrates in the three `select` implementations (each an
ISA's operand decode) and `rename_field_values` — all guarded by
`artifact-gate … all` (which regenerates and diffs `.ncode`; add `.mir` fixtures
to the gate's coverage if not already swept).

Rejected: leaving `MirInstruction` stringly-typed permanently. It keeps two
operand models, forces plan-78's render/parse round-trip at every MIR boundary,
and leaves a stringly foothold that erodes plan-78's win over time.

## 4. Detailed Design

- `mir.rs:28` → `fields: Vec<(&'static str, Operand)>`.
- `lower_to_mir` (`mir.rs:413`): the `.fields.clone()`/`.iter().cloned()` sites now
  clone `Operand`; `fuse` (used at `:479,488`) threads `Operand`.
- `rename_field_values` (`mir.rs:504`): replace the string rename of the neutral
  arena-base with a match on `Operand` (rewrite the arena-base placeholder to the
  realized `Phys`); update the `v == realization` comparisons (`mir.rs:1633`) and
  the `"share"` key checks (`mir.rs:1267`) — key checks are name-based and
  unaffected; value checks compare `Operand`s.
- Each `select` (`aarch64/x86_64/riscv64 select.rs`, `riscv64/v128.rs`): read
  operands via `operand(name)`/match on `Operand`; construct `CodeInstruction`
  `Operand`s (mirroring plan-78-B's constructors).
- `ToCodeJson for MirInstruction` (`mir.rs:748`): render `Operand` values.

## Compatibility / Format Impact

Externally none — `.mir`/`.ncode`/executables byte-identical. Internal only: the
`MirInstruction.fields` element type (crate-private).

## Phases

> **NOTE — keep boxes/`Commit:` current; run `artifact-gate … all` after each.**

### Phase 1 — Measure the MIR-boundary round-trip

Decide (and record) how much the typed→String→typed seam actually costs, so the
plan's value is stated in numbers, not assumed. Uses plan-78-A's harness.

- [x] **Measured — the seam is NOT negligible; it is the dominant barrier to
      plan-82's win.** plan-82-A/B/C typed every operand at the `CodeInstruction`
      layer but the counting-allocator probe showed only a 2.3% total-allocation
      drop (808,803,959 → 789,917,084 on `mfb test tests/acceptance`), because
      `lower_to_mir` renders every operand to `String` and `select` rebuilds each as
      `Operand::Raw` — discarding the typed operands before regalloc/encode. The
      profile's top-of-stack includes `Operand::render` (the round-trip) and the
      per-instruction `Vec` churn the round-trip feeds. Full evidence: plan-82-A
      §CORE-PREMISE FALSIFICATION.
- [x] The seam is >2% (it caps plan-82's entire win), so plan-79 is a **speed**
      plan, not merely consistency — it is the prerequisite that makes plan-82-B/C/D
      effective.

Acceptance: the MIR-boundary cost is recorded (the plan-82 falsification
measurement); it is the reason plan-82 stalled at 2.3%.
Commit: (recorded with Phase 2)

### Phase 2 — Flip `MirInstruction` to `Operand` + migrate producers/dump

- [x] `MirInstruction.fields: Vec<(&'static str, Operand)>`; `mir_fields_from_code`
      / `code_fields_from_mir` are clones (no render/`from`); `lower_to_mir` `fuse`
      literal pushes use `.into()`; `rename_field_values`/`rename_operand_field_values`
      match `Raw` directly (no per-field `render()` — see Corrections); the token
      realization loop matches `Raw` directly.
- [x] `ToCodeJson for MirInstruction` renders `value.render()` (identical `.mir`).
- [x] Tests: the mir round-trip/fusion tests + the `get`/`base` helpers adapt to
      `Operand` (`.render()`/`.as_deref()`); `cargo test --bin mfb` green (3774).
- [x] `artifact-gate … all` — **0 diffs** (all four targets).

Acceptance: builds; `cargo test --bin mfb` 3774 green; `.mir`/`.ncode`
byte-identical (0 diffs). ✓
Commit: (recorded with Phase 3)

### Phase 3 — Migrate the per-arch `select` readers + measure the win

- [x] `aarch64/select.rs`, `x86_64/select.rs`, `riscv64/select.rs`, `riscv64/v128.rs`
      read MIR operands (`render()`/`rendered()` for decisions) and pass the typed
      `Operand` through by clone (no `Operand::from(&str)` re-`Raw`). Whole crate +
      tests green (3774).
- [x] `artifact-gate … all` — **0 diffs** across all four targets (twice: after the
      select migration, and again after the per-field render-alloc fixes).
- [x] **Allocation measurement (the plan-82 win, finally realized):**

| Metric | plan base 03201b38d | post-A/B/C | **post-plan-79** | command |
|---|---|---|---|---|
| Total allocations | 808,803,959 | 789,917,084 | **640,307,625** (−20.8% vs base) | counting-allocator probe on `mfb test tests/acceptance` |
| Release acceptance wall | 58 s | 56 s | **52.2 s** | `time target/release/mfb test tests/acceptance` |
| Debug acceptance wall | 284 s | 275 s | *(measured in Phase-3 followup)* | `time target/debug/mfb test tests/acceptance` |
| Acceptance | 362/362 | 362/362 | **362/362** | (release + debug) |

Acceptance: `artifact-gate … all` byte-identical (0 diffs); the MIR boundary does no
`Operand`↔`String` round-trip; **allocations fell 168.5M (−20.8%)** — the win
plan-82-A/B/C could not realize because of the String MIR barrier this plan removed.
Commit: (recorded next commit)

## Validation Plan

- Tests: `cargo test --bin mfb` (mir + the three arch select/encode modules).
- Byte-identity: `artifact-gate.sh … all` diff-free after every phase, including
  `.mir` goldens — the guardrail; a diff is a render/migration gap, not a
  re-baseline.
- Cross-target: the `all` sweep covers aarch64/x86_64/riscv64 + linux/windows
  data images.
- Runtime proof: `mfb test tests/acceptance` exits 0 (codegen still executes
  correctly across the MIR boundary, not just byte-matches).
- Coverage: `scripts/coverage-check.sh` — migrated mir/select code stays ≥95%.
- Doc sync: if any `mir.md`/`.ai` note describes `MirInstruction` operands as
  strings, update it.
- Acceptance: `cargo test --workspace` + `artifact-gate … all` green.

## Open Decisions

- **Is the perf win real or is this consistency-only?** UNMEASURED — the original
  acceptance-hang profile showed selection *not* on the hot path, so the
  typed→String→typed seam may be negligible. Phase 1 measures it. Recommendation:
  proceed regardless (one operand model is worth it for maintainability and to
  protect plan-78's win), but state honestly in the Summary which it turned out to
  be. (§2, Phase 1)

## Corrections

- **Reading a typed MIR field for a *decision* must not `render()` per field.** The
  first cut naively used `value.render()` in `rename_field_values`,
  `rename_operand_field_values`, and the aarch64 token-realization loop — three
  passes that touch **every field of every instruction** in the compile. That
  *added* ~160M allocations (measured: 789.9M → 801.8M, a regression) because
  `render()` on a `VReg`/`Phys`/`Imm` allocates a `String`. Fix: those three passes
  only ever match a **`Raw` physical/token** (the arena base, the abi role tokens),
  never a typed operand, so they now `if let Operand::Raw(text) = value` and compare
  the `&str` directly — zero allocation. That turned the regression into the −20.8%
  win (801.8M → 640.3M). Lesson: typing MIR pays off only if the per-field decision
  reads stay allocation-free (match `Raw`, or use `rendered()`'s borrow, never
  `render()`), because those passes dwarf the pass-through operand count.

- **Prereqs were stale, not unmet.** The doc's Status column said plan-78 A/B "NOT
  MET (not yet started)"; in fact plan-78 is merged and plan-82-A/B/C landed the
  typed representation. Corrected in place; plan-79 proceeded.

- **riscv64/x86_64 select kept a stringly *internal* decode.** Those backends read
  many field values for selection decisions (`field_value`, `cond_and_target`, the
  v128 slot/vreg maps); they now `render()` those to `String` internally (byte-
  identical) rather than being fully re-typed. That is fine: the acceptance perf
  target is aarch64, the pass-through operands (the allocation-relevant ones) stay
  typed via `code_fields_from_mir`'s clone on every backend, and byte-identity holds
  across all four targets (gate: 0 diffs).

## Summary

plan-79 removes the last stringly operand surface (`MirInstruction`) so codegen
carries one `Operand` model from shared lowering through selection to encoding,
and closes the typed→String→typed round-trip plan-78 leaves at the MIR boundary.
Its risk is the three per-arch `select` decoders and `rename_field_values`,
guarded by byte-identical `.mir`/`.ncode` goldens. Whether it is a *speed* win or
purely a *consistency* win is settled by Phase 1's measurement, not assumed here.
