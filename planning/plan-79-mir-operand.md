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
| plan-78-A complete (`Operand` + render + corpus test) | `ls planning/completed/plan-78-A-* 2>/dev/null` OR A's phases ticked | NOT MET (A not yet started) |
| plan-78-B complete (`CodeInstruction.fields` is `Operand`) | `ls planning/completed/plan-78-B-* 2>/dev/null` OR B's phases ticked | NOT MET (B not yet started) |
| Repo builds clean; goldens green | `cargo build --bin mfb && bash scripts/artifact-gate.sh target/debug/mfb all` | UNVERIFIED — run first |

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

- [ ] With plan-78 landed, run `scripts/bench-lowering.sh`; add a probe (or a
      one-off counter) isolating `lower_to_mir` + `select` time on the one-regex
      and full-acceptance builds. Record in `planning/plan-79-baseline.txt`.
- [ ] If the seam is negligible (<~2% of lowering), note that plan-79's value is
      **representation consistency**, not speed — and say so in the Summary.

Acceptance: `planning/plan-79-baseline.txt` records the MIR-boundary cost before
the flip.
Commit: —

### Phase 2 — Flip `MirInstruction` to `Operand` + migrate producers/dump

- [ ] Flip `mir.rs:28`; migrate the 4 construction sites, `lower_to_mir`,
      `fuse`/fusion, and `rename_field_values` to `Operand`.
- [ ] Render `Operand` in `ToCodeJson for MirInstruction` (`mir.rs:748`).
- [ ] Tests: the mir round-trip/fusion tests (`mir.rs:830` `after.fields ==
      before.fields` etc.) adapt to `Operand`; add one asserting a MIR operand is
      a typed `Operand`, not a string.
- [ ] `artifact-gate … all` — zero diffs (incl. `.mir` goldens).

Acceptance: compiler builds; `cargo test --bin mfb` green; `.mir` and `.ncode`
byte-identical.
Commit: —

### Phase 3 — Migrate the per-arch `select` readers (largest blast radius)

- [ ] Rewrite `aarch64/select.rs`, `x86_64/select.rs`, `riscv64/select.rs`, and
      `riscv64/v128.rs` to read `Operand`s and build `CodeInstruction` `Operand`s
      directly — no `String` parse at the boundary.
- [ ] Tests: each arch's `select`/encode test module stays green; add a spill-free
      and a spill-heavy fixture per arch confirming byte-identical `.ncode`.
- [ ] `artifact-gate … all` — zero diffs across all four targets.

Acceptance: `artifact-gate … all` byte-identical; the MIR boundary performs no
`Operand`↔`String` conversion (verified by inspection + the Phase-1 probe showing
the seam cost gone); `bench-lowering.sh` shows no regression (and any measured
seam win realized).
Commit: —

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

<Filled in during execution.>

## Summary

plan-79 removes the last stringly operand surface (`MirInstruction`) so codegen
carries one `Operand` model from shared lowering through selection to encoding,
and closes the typed→String→typed round-trip plan-78 leaves at the MIR boundary.
Its risk is the three per-arch `select` decoders and `rename_field_values`,
guarded by byte-identical `.mir`/`.ncode` goldens. Whether it is a *speed* win or
purely a *consistency* win is settled by Phase 1's measurement, not assumed here.
