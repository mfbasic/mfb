# plan-78-C: Typed-operand register allocation + `colored_mask_at` sweep

Last updated: 2026-08-02
Effort: medium (1h–2h)
Depends on: plan-78-B (storage must already be `Operand`-typed)

Migrate the register allocator's hot loops to read the typed `Operand` values
directly (integer register ids) instead of re-parsing/hashing/`str::eq`-comparing
operand strings, and replace the O(vregs × interval) `colored_mask_at`
construction with an endpoint sweep. This sub-plan **delivers the perf win**: it
removes the measured `str::eq` (#1/#2 self-time) and SipHash costs from the
analysis and eliminates the spill-path quadratic — all while keeping emitted code
byte-identical.

The single behavioral outcome: the lowering/regalloc pass is dramatically faster
(one `regex::match` const ≤ 3 s debug from 31 s; `mfb test tests/acceptance`
≤ 60 s debug from 4 m 21 s) with `artifact-gate … all` still diff-free.

References:

- plan-78-B (`planning/plan-78-B-flip-storage.md`) — provides `operand(name) ->
  &Operand` typed reads on `CodeInstruction`.
- `src/target/shared/code/regalloc/analysis.rs` — `effect`, `is_tracked`,
  `physical_index`, liveness.
- `src/target/shared/code/regalloc/linear_scan.rs` — the scan, `colored_mask_at`
  (:170-182), the rewrite loop (:201).

## Prerequisites

See plan-78-A's Prerequisites table (feature-wide). Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-78-B complete (fields are `Operand`, `operand()` accessor exists) | B's phases all ticked (commits 4eafd3830, 02f9bd2ea) | MET (2026-08-02) |
| `bench-lowering.sh` baselines recorded (from A) | `cat planning/plan-78-baseline.txt` | MET (2026-08-02: one-regex 29.2s debug, acceptance 266s debug, regex fn 860,981 instrs / 135,293 int vregs) |

> If B is not complete, C cannot start — full stop. C reads `operand()`, which B
> introduces; it does not itself flip storage.

## 1. Goal

- `analysis::effect`, `is_tracked`, and the liveness/scan/rewrite loops read
  register class + id from `Operand` (integers), with **no** per-operand
  `parse_vreg`/`physical_index` string work and **no** `Vec<String>` operand
  clones.
- `colored_mask_at` (`linear_scan.rs:170`) is built by an endpoint sweep in
  O(instructions + Σ interval endpoints), producing a bit-identical mask.
- Measured perf goal met (see Goal above) with `artifact-gate … all` diff-free.

### Non-goals (explicit constraints)

- **No emitted-byte change.** Allocation decisions (which vreg → which physical,
  which spills, spill-slot order, the bug-87 `(start, id)` tie-break) are
  identical; `artifact-gate … all` is the guardrail.
- **No `MirInstruction`/selection change** (out of scope, not hot).
- **No `-regalloc bump` change.**

## 2. Current State

Post-B, `CodeInstruction` carries typed `Operand`s but the allocator still reads
them as strings via the rendered `get()` path:

- `effect` (`analysis.rs:315`) iterates `instruction.fields`, tests
  `DEF_FIELDS`/`USE_FIELDS.contains(name)` (`:23,27`) and `is_tracked(value)`
  (`:303`), and clones matching operand **strings** into `Vec<String>`.
- `is_tracked` → `parse_vreg` (`mod.rs:45`) + `physical_index`
  (`int_concrete_physical_index`, `analysis.rs:213`), whose core is a linear
  `REG_ARRAY.position(|&reg| reg == name)` (`:227`) — the measured #1/#2
  self-time `str::eq`.
- `effect` is computed **3×** per instruction (`analysis.rs:520`, `:601`,
  `linear_scan.rs:202`).
- `colored_mask_at` (`linear_scan.rs:170-182`): for each colored vreg, OR a bit
  across every instruction index in its (over-approximated, wide) `[s,e]`
  interval — O(vregs × interval), on the spill path only (regex spills heavily).

### Verified properties

- **The perf cost is register-string handling + the spill quadratic.** Profile
  self-time: `str::eq` #1+#2 (~800 samples), SipHash/`hashbrown`, `memmove`/
  `memcmp`, slice `position`/`any`; call tree `allocate`→`linear_scan::run` ≈ 80%,
  `analyze` ≈ 20%. (`sample <pid>` during the one-regex build.)
- **B exposes register class+id as integers via `operand()`** — so `effect` can
  classify def/use and read register identity with zero parsing.

## 3. Design Overview

Two independent wins, both in `regalloc/`:

1. **Typed reads** — rewrite `effect`/`is_tracked` to match on `Operand`:
   `VReg{class,id}` and `Phys{class,index}` are already the integers the scan
   needs; `Raw`/`Imm` are "not a tracked register of this class". Drop
   `parse_vreg`/`physical_index` from the hot path and the `Vec<String>` clones.
   Compute `effect` **once** per instruction per class and share it across
   liveness and the rewrite loop (the 3→1 dedup).
2. **Sweep `colored_mask_at`** — emit `(s, +bit)`/`(e+1, -bit)` events per colored
   vreg, sort once, fold across instruction indices maintaining a running mask.
   Representation-independent; would be worth doing even without B.

Correctness risk: both must reproduce the *exact* current allocation and masks.
Guarded by `artifact-gate … all` plus a property test asserting the sweep mask ==
the naive double-loop mask.

## 4. Detailed Design

- `Effect` becomes index/id-based (store `(RegClass, u32)` register ids, or
  small bitsets, not `Vec<String>`); `effect(instruction, class)` reads
  `instruction.operand(name)` and matches the `Operand` arm.
- `is_tracked(op: &Operand, class)` is a match, not two string parses.
- Memoize `Vec<Effect>` once per `(function, class)`; pass it to `analyze` and to
  the rewrite loop instead of recomputing (`analysis.rs:520,601`,
  `linear_scan.rs:202`).
- `colored_mask_at`: build from interval endpoints (§3); assert equality with the
  naive computation under test.
- The final rewrite (`mod.rs:359`) that substitutes colored vregs → physicals now
  writes `Operand::Phys{class,index}` directly (no sentinel string round-trip).

## Compatibility / Format Impact

None externally. `.ncode`/`.mir`/executables byte-identical.

## Phases

> **NOTE — keep boxes/`Commit:` current; run `artifact-gate … all` after each.**

### Phase 1 — Sweep-based `colored_mask_at` (representation-independent)

Land the algorithmic win first — it's isolated and doesn't depend on the typed
reads.

- [ ] Rewrite `colored_mask_at` (`linear_scan.rs:170-182`) as an endpoint sweep.
- [ ] Tests: property test in `regalloc/tests.rs` — sweep mask == naive mask over
      randomized intervals; a spill-heavy fixture stays byte-identical.
- [ ] `artifact-gate … all` — zero diffs.

Acceptance: `artifact-gate … all` byte-identical; sweep==naive property test
passes; `bench-lowering.sh` shows the spill-path cost no longer scales with
vregs × function size.
Commit: —

### Phase 2 — Typed `effect`/liveness + compute-once

The main perf win.

- [ ] Rewrite `effect`/`is_tracked` (`analysis.rs`) to match on `Operand`; drop
      `parse_vreg`/`physical_index` from the hot path; remove `Vec<String>`
      operand clones.
- [ ] Compute `Vec<Effect>` once per `(function, class)`; share across `analyze`
      and the rewrite loop (dedup the 3 sites).
- [ ] Have the vreg→physical rewrite write `Operand::Phys` directly (`mod.rs:359`).
- [ ] Tests: coloring-output-unchanged on spill-heavy Int/Fp fixtures; a
      determinism check (two builds byte-identical); an assertion that `effect`
      runs once per instruction per class.
- [ ] `artifact-gate … all` — zero diffs.

Acceptance: `artifact-gate … all` byte-identical; `str::eq` and SipHash out of
the top self-time in the one-regex profile; **feature goal met** —
`bench-lowering.sh` reports one `regex::match` const ≤ 3 s debug and
`mfb test tests/acceptance` ≤ 60 s debug.
Commit: —

## Validation Plan

- Tests: `cargo test --bin mfb` incl. the sweep property test, the typed-effect
  tests, and the determinism check.
- Byte-identity: `artifact-gate.sh … all` diff-free after every phase (guardrail).
- Runtime proof: `mfb test tests/acceptance` exits 0 with all cases passing —
  proves codegen still *executes* correctly, not just that bytes match.
- Performance: `bench-lowering.sh` before/after each phase; final numbers meet §1
  and are recorded next to the A baselines.
- Coverage: `scripts/coverage-check.sh` — changed regalloc code stays ≥95%.
- Acceptance: `cargo test --workspace` + `artifact-gate … all` green.

## Open Decisions

- **`Effect` storage shape** — `Vec<(RegClass,u32)>` vs. per-class bitsets.
  Recommend bitsets if the id space is dense per function (cheapest liveness
  merge), else id vectors. Decide from the Phase-1 function-size measurement. (§4)

## Corrections

<Filled in during execution.>

## Summary

C is where the speed arrives: typed register reads delete the `str::eq`/hash hot
loops, compute-once removes the 3× `effect` recompute, and the sweep kills the
spill-path quadratic — all provably byte-identical via `artifact-gate … all`.
After C, the acceptance suite compiles well within CI budget on the debug binary
that broke it, without changing a single emitted byte.
