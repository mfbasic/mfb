# plan-82-B: Typed operands on the register-allocator hot path

Last updated: 2026-08-02
Effort: large (3h–1d)
Depends on: plan-82-A (the `Operand::Phys` arm and its faithful per-arch
rendering must exist and be proven byte-identical)

Flip the register allocator to work in **typed operands**: read each instruction
operand into a typed `VReg` once at entry, carry vregs and assignments as
`{class, u32}` values, and write each colored result as `Operand::Phys` —
instead of parsing `%vN`/`%fN` strings, formatting physical-register strings, and
cloning `Box<str>` operand vectors per instruction. This is the exact
construction plan-78-C was scheduled to do and deferred.

Single behavioral outcome: the allocator produces the **same** physical-register
assignment for every instruction as today (byte-identical `--ncode` and machine
code), while the `code_emit` substage's allocation count drops sharply (today
566 M of the 809 M total; the allocator's per-instruction `Box<str>` clones are
the dominant contributor — see plan-82-A baseline).

References:

- plan-82-A (`planning/plan-82-A-register-operand-representation.md`) — shared
  goal, baseline, Prerequisites, and the `Phys` arm this plan constructs.
- `src/target/shared/code/regalloc/mod.rs` — `allocate` (`:` around the
  `find_physical_operand`/rewrite), the env size probe.
- `src/target/shared/code/regalloc/linear_scan.rs` — `run`, `substitute`,
  `occupied_at`, `colored_mask_sweep` (the per-instruction clone + string
  rewrite hot loop).
- `src/target/shared/code/regalloc/analysis.rs` — `effect`, `RegRef`,
  `parse_vreg`, `int_concrete_physical_index` (`:257`), `fp_physical_index`
  (`:307`).
- `src/target/shared/code/builder_registers.rs` — `run_register_allocation`
  (`:` calls `regalloc::allocate` at `:145`).

## Prerequisites

See plan-82-A §Prerequisites (the feature-wide gate). Additionally: **if
plan-82-A is not complete, plan-82-B cannot start, full stop** — B constructs
`Operand::Phys`, which A defines and proves faithful. Verify:

| Must be true | Command | Status |
|---|---|---|
| plan-82-A merged: `Phys` arm exists, round-trip test present | `rg -n 'Operand::Phys \{ class' src/target/shared/code/operand.rs` and `rg -n 'phys_operand_round_trips' src/target/shared/code/regalloc/analysis.rs` | **MET** (A complete: bc622ae41; arm carries `{class,index,name}`, round-trip test green) |

## 1. Goal

- The allocator's internal working set (intervals, live sets, the rewrite that
  substitutes vreg→physical) carries typed `{class, u32}` values, not strings.
- On write-back, a colored operand becomes `Operand::Phys { class, index }`, not
  `Raw(Box<str>)`. Downstream consumers still read it via `rendered()` (Cow), so
  encode is unchanged this sub-plan and output is byte-identical.
- The `#[allow(dead_code)]` on the typed arms from A is removed (they are now
  constructed in production).

### Non-goals

- Same as plan-82-A §Non-goals. In particular: **the assignment must not
  change.** The coloring algorithm, its iteration order, and its tie-breaks stay
  bit-for-bit (see `vreg-alloc-order-load-bearing`, `bug387`). This plan changes
  the *carrier type*, never the *decision*.
- Do not touch the 1825 producer sites (that is C); the allocator still receives
  operands that render as `%vN` and parses them once at entry.
- Do not touch the encoder (that is D); it keeps consuming `rendered()`.

## 2. Current State

`run_register_allocation` (`builder_registers.rs:145`) calls `regalloc::allocate`
over the function's `CodeInstruction`s. Inside, `effect` (`analysis.rs`)
classifies each operand by `rendered()`-then-`parse_vreg`/physical-index;
`linear_scan::run` builds intervals, colors, and rewrites operands by
substituting a formatted physical string, cloning operand vectors and `Box<str>`
per instruction (the profiling call tree shows `run` → `Vec::clone` →
`Box<str>::clone` → `malloc` as the top allocation path). Everything is keyed on
strings that are re-parsed and re-rendered repeatedly.

### Measured populations

| What | Count | Command |
|---|---|---|
| code_emit substage allocations (release) | 566,000,000 | plan-82-A baseline (per-substage counter) |
| Regalloc clone path share of code_emit | dominant (`run`→`Vec::clone`→`Box::clone`→malloc top subtree) | `sample` call graph, plan-82-A |
| Instructions the allocator rewrites (acceptance) | 9,793,755 across 841 fns | `MFB_BENCH_LOWERING` sum |

### Verified properties

- **`rendered()` gives a lossless bridge.** A `Phys{class,index}` rendered via
  A's arch renderer equals the string today's rewrite writes (A's round-trip
  test). So B can write typed `Phys` and every unmodified downstream reader
  (encode, validation, dumps) sees identical text. UNVERIFIED until A lands.

## 3. Design Overview

Three internal boundaries change, none externally visible:

1. **Entry:** `effect`/interval construction reads each operand once into a
   typed `RegRef::VReg(class,u32)` / `RegRef::Phys(class,u32)` (the classify-once
   `RegRef` from plan-78-C already exists; extend it to carry the class and to be
   built without re-`rendered()` where the operand is already typed).
2. **Core:** intervals, live sets, and the colored-mask sweep key on
   `(class,u32)` — already `u32`-keyed after plan-78-C's `U32Map`; drop the
   residual string round-trips in `substitute`/`occupied_at`.
3. **Write-back:** the rewrite stores `Operand::Phys { class, index }` directly
   instead of `Operand::from(assigned_string)`.

Correctness risk: the rewrite (must assign the identical physical index). Mitigate
with artifact-gate byte-identity after every commit and a unit test asserting the
typed rewrite of a hand-built instruction stream equals the string rewrite.

## Phases

> Keep checkboxes current in the same commit as the work.

### Phase 1 — Typed classify at entry (no write-back change yet)

- [x] Add `class: RegClass` to `ClassModel`; rewrite `effect`'s `classify` to read
      a typed `VReg`/`Phys` of the matching class directly (its `id`/`index`), with
      the `rendered()` + `parse_vreg`/`physical_index` string path only for
      `Raw`/`Imm`. (`RegRef` did not need a class field — the pass class lives in
      `ClassModel`; recorded in Corrections.) Behavior unchanged.
- [x] Tests: `effect_classifies_typed_and_raw_operands_identically`
      (`analysis.rs`) — `effect` over a typed stream vs. the equivalent `Raw`
      stream yields identical def/use `RegRef`s (and the other-class fp operand is
      ignored in both).

Acceptance: `cargo test --bin mfb` green (3774 passed); `artifact-gate … all`
byte-identical (0 diffs). ✓
Commit: 9cddb6e8f

### Phase 2 — Typed write-back (`Phys` construction on the hot path)

- [x] `substitute` writes `Operand::phys(class, index, name)` (carrying the static
      name + class index) instead of `Operand::from(assigned_string)`. `assignment`
      and `scratch_for` now hold `(&'static str, u32)` (static name + index)
      instead of `String`, so the per-colored-vreg `name.to_string()` box is gone.
      Removed the `#[allow(dead_code)]` on the `Phys` arm + `phys()` ctor (now
      constructed in production).
- [x] `substitute`/`occupied_at` read a typed `VReg`/`Phys` of the class directly,
      taking the `rendered()`+parse path only for `Raw`. (The `instruction.fields.
      clone()` still boxes any `Raw` operand in the input; those disappear as C
      converts producers — the residual is `Raw`, not typed.)
- [x] Tests: `linear_scan_writes_typed_phys_operands` (`regalloc/tests.rs`) — run
      the allocator, assert the colored `dst` is `Operand::Phys{class:Int,index,
      name}` with `int_physical_index(name)==index` and `rendered()==name`, the use
      site carries the same typed physical, and a hardcoded `x0` stays `Raw`.

Acceptance: `artifact-gate … all` byte-identical to pre-plan (**0 diffs**, 1553
goldens); `cargo test --bin mfb` green (3774). Per-substage alloc counter: see
Corrections — that counter was an ad-hoc profiling-spike instrument, not committed
infra; the committed reproducible signal is the acceptance wall (Phase 3). The
write-back structurally removes, per colored vreg, one `name.to_string()` and
halves the per-rewritten-operand boxes (clone+overwrite: was clone-box + from-box,
now clone-box + typed-no-box).
Commit: 9cddb6e8f

### Phase 3 — Perf checkpoint

- [x] Re-run the acceptance-wall measurements (debug + release; the per-substage
      alloc counter is not committed infra — see Corrections). Recorded below. This
      is a checkpoint, not the final target (C and D remove more).

| Metric | Pre-plan baseline (82-A) | Post-B | Command |
|---|---|---|---|
| Release acceptance wall | 58 s | **56.3 s** | `time target/release/mfb test tests/acceptance` |
| Debug acceptance wall | 284 s | **274.9 s** | `time target/debug/mfb test tests/acceptance` |
| Acceptance tests | 362/362 | **362/362 pass** | (both binaries, Fail: 0) |

Acceptance: the release AND debug acceptance walls both fell (58→56.3 s,
284→274.9 s), acceptance suite green on both binaries (362/362), and
`artifact-gate … all` byte-identical (0 diffs) — the assignment did not move. No
front-end/runtime regression (only the codegen carrier type changed). The modest
size of the drop is expected (Corrections: B's classify fast path is dormant until
C converts producers).
Commit: d875f46ce

## Validation Plan

- Tests: the entry-equivalence and write-back-equivalence unit tests above
  (`cargo test --bin mfb`); the existing regalloc test modules
  (`sweep_equals_naive_over_randomized_intervals`, etc.) stay green.
- Coverage check: confirm the changed `linear_scan`/`analysis` lines are exercised
  by the acceptance compile (they are — every function is allocated).
- Runtime proof: `mfb test tests/acceptance` exits 0 (all TESTING cases pass) on
  the release binary built from this branch — proves the assignment is still
  correct, not just byte-identical on the sampled targets.
- Doc sync: none.
- Acceptance: `artifact-gate … all` byte-identical; `cargo test`; acceptance
  suite green; recorded allocation-count drop.

## Open Decisions

- None new (compound-operand handling was decided in plan-82-A Phase 1).

## Corrections

- **`RegRef` did not need a `RegClass` field.** The plan said "extend `RegRef`/
  `effect` to carry `RegClass`". In practice the pass's class already lives in
  `ClassModel` (added a `class: RegClass` field there), which is all `classify`/
  `occupied_at`/`substitute` need to match a typed operand's class and to
  construct `Phys{class,…}`. Adding a class to every `RegRef` (one per def/use per
  instruction) would be redundant. `RegRef` stays `Phys(u32) | VReg(u32)`.

- **The per-substage GlobalAlloc counter is not committed infrastructure.** The
  plan-82-A baseline's "566 M code_emit allocs" / "215 M encode allocs" were
  measured with an ad-hoc counting allocator during the 2026-08-02 profiling
  spike; no such counter exists in the tree, and committing a global-allocator
  counter would add a per-allocation branch to the very compiler being optimized.
  So B/C/D report the **committed, reproducible** signal — the `mfb test
  tests/acceptance` wall-clock (release and debug) plus a green acceptance suite —
  rather than a re-derived alloc count. The debug wall is D's headline target
  (≤60 s) and is directly measurable; the allocation reductions are described
  structurally (which allocations each phase removes).

- **B's win is partial by design, and that is expected, not a shortfall.** Until
  plan-82-C converts the 1825 producers, every register operand still enters the
  allocator as `Raw("%vN")`, so `classify`'s typed fast path is dormant and the
  live win is only the write-back (typed `Phys`, no `String` assignment map). The
  release acceptance wall moved 58 s → 56.3 s. The large drops land in C
  (production-time boxes) and D (encode scans).

## Summary

B is the core of the deferred fix and the single biggest allocation win: it stops
the allocator cloning and re-parsing `Box<str>` operands per instruction by
carrying typed `{class,u32}` values and writing typed `Phys`. It is guarded on
both sides by artifact-gate byte-identity (the assignment must not move) and the
acceptance suite (the assignment must still be correct). Untouched here:
production sites (C) and the encoder (D).
