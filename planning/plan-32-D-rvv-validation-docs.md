# plan-32-D: RVV per-run optimization, validation, CI, and docs

Last updated: 2026-07-08
Effort: medium (1h–2h)
Depends on: plan-32-A, plan-32-B, plan-32-C

Turn C's correct-but-per-op dual-path into a fast, permanently-gated feature:
hoist the runtime dispatch from per-op to per-**run** so the RVV arm keeps values
in vector registers across a kernel region (the actual speedup), then prove the
whole thing — one binary, ≤1 ULP and bit-identical values on **both** V and
non-V execution, under a CI lane that runs the single executable through both
QEMU cpu profiles — and document the portability guarantee.

The single behavioral outcome: `runtime_ulp.py` reports ≤1 ULP for every math
kernel when the same binary runs with `-cpu rv64,v=true` and with `v=false`;
CI runs one build through both profiles; and the docs state that a
`linux-riscv64` binary runs on both V and non-V chips, picking vectors at
run time.

References:

- `tools/math-kernels/runtime_ulp.py`, `tools/math-kernels/ulp.py` — the ULP
  harness; plan-99 §5 requires ≤1 ULP on rv64 (base-D FMA).
- `src/arch/riscv64/v128.rs` `build_slot_map`/`_mfb_rt_v128_slots` — the
  run-boundary reconciliation point the per-run optimization spills to.
- `.ai/remote_systems.md` (`ssh -p 2229` Alpine riscv64; V status unknown, so
  QEMU `-cpu ...,v=true` is the portable oracle); `.ai/specifications.md`,
  `.ai/compiler.md`.
- `planning/old-plans/plan-99-rv64-backend.md` §5 — the parity / ULP / both-libc
  bar this extends to the dual-path binary.

## 1. Goal

- **Per-run dispatch:** one `beqz _mfb_rt_has_rvv` guards a maximal contiguous
  v128 run; the RVV arm keeps intermediate values in `v`-registers across the
  run and spills only live-out values to the slots at the run boundary (where
  the scalar arm and non-v128 code read them). Real vector speedup, not a
  per-op memory round-trip.
- `runtime_ulp.py` ≤1 ULP for sin/cos/tan/exp/log/pow/atan2 on the **same
  binary** under both cpu profiles; nbody/mandelbrot/math values bit-identical
  to AArch64/x86_64 in both.
- A CI lane building the riscv64 binary **once** and running the suite under
  `-cpu rv64,v=true` and `-cpu rv64,v=false`, both green (plus both libc flavors
  where feasible, matching plan-99).
- Docs: the `linux-riscv64` target produces a binary that runs on V and non-V
  chips, selecting RVV at run time; the scalar-only fallback on register
  pressure is noted.

### Non-goals (explicit constraints)

- No new v128 semantics — per-run is a dispatch/residency change; the RVV arm's
  per-op results are C's, unchanged. Any divergence found is fixed in the owning
  A/B/C file.
- Other backends and the non-V execution path stay byte-identical.

## 2. Current State

- After C: every v128 op emits a per-op guard + scalar arm + RVV arm, reconciled
  at the slots. Correct on both chip types, but the RVV arm round-trips through
  memory each op — most of the vector win is left on the table.
- The ULP harness (`runtime_ulp.py`) is the established rv64 math oracle
  (plan-99 validated ≤1 ULP scalar). No dual-path / per-run coverage yet.
- CI has a default riscv64 QEMU lane (plan-99); no V-profile run of the same
  binary.

## 3. Design Overview

1. **Per-run residency.** Reuse `build_vreg_map`'s live ranges (from C) to find
   maximal contiguous v128 runs; emit one guard per run; within the RVV arm keep
   values in their assigned `v`-registers and `vse64` only the run's live-out
   values to slots at the boundary. `vsetivli` once per run. The scalar arm is
   unchanged (still slot-based). Loop bodies are runs re-entered per iteration
   (the guard is loop-invariant and may later be hoisted above the loop, but a
   per-iteration predicted branch is acceptable and simplest).
2. **Two-profile value/ULP gate.** Build once; run the math/vector programs and
   `runtime_ulp.py` under `v=true` and `v=false`; require bit-identical vs.
   AArch64 and ≤1 ULP in both.
3. **CI:** add a V-profile job that reuses the default riscv64 build artifact and
   runs it under a vector-enabled `-cpu`; keep the non-V job. Both must pass on
   the same binary.
4. **Docs/spec:** document the runtime-selection guarantee and the fallback.

**Risk:** low and diagnostic. The per-run change is a residency optimization
over C's proven per-op arm; the gate mostly *confirms* nothing regressed. The
one real hazard is a run boundary that misses a live-out value (silent
corruption) — caught by the value-parity diff, which fails on any bit
difference. QEMU vector-model fidelity vs. silicon is noted; run on `ssh -p 2229`
if it implements V.

## Compatibility / Format Impact

- **Changed:** RVV arm keeps registers across a run (fewer slot stores); CI gains
  a V-profile lane; docs gain the portability guarantee. Additive.
- **Unchanged:** scalar arm bytes, non-V execution, other backends, all values.

## Phases

### Phase 1 — per-run register residency

- [ ] Group contiguous v128 ops into runs (from `build_vreg_map` liveness);
      emit one guard per run; RVV arm keeps values in `v`-registers, `vse64`ing
      only live-out values to slots at the run boundary; one `vsetivli` per run.
- [ ] Tests: a selection unit test that a multi-op run emits a single guard and
      no per-op slot round-trip on the RVV arm; live-out values are spilled at
      the boundary.

Acceptance: same binary, both cpu profiles, still bit-identical to AArch64 for
the math/vector programs; RVV arm shows one guard + register-resident chain per
run (not per-op memory traffic).
Commit: —

### Phase 2 — two-profile value parity + ULP

- [ ] Run the math/vector rt-behavior programs and `runtime_ulp.py` on the one
      binary under `qemu-riscv64 -cpu rv64,v=true,vlen=128` and `v=false`; diff
      f64/i64 output vs. AArch64 goldens (fail on any bit difference); require
      ≤1 ULP in both. Fix any divergence in the owning A/B/C file and re-run.
- [ ] Tests: record the exact QEMU cpu/vlen invocations in the acceptance script
      so the gate is reproducible.

Acceptance: bit-identical vs. AArch64 and ≤1 ULP under **both** profiles on the
same binary.
Commit: —

### Phase 3 — CI lane + docs/spec

- [ ] Add a V-profile CI job that runs the default riscv64 build artifact under a
      vector-enabled `-cpu`; keep the non-V job; both green on the same binary.
- [ ] Document in `src/docs/spec/**` (build-targets) and target/man reference:
      a `linux-riscv64` binary runs on both V and non-V chips, selecting RVV at
      run time via `AT_HWCAP`; the scalar-only fallback on register pressure.
- [ ] Update `.ai/remote_systems.md` if the riscv64 box's V status is confirmed.
- [ ] Tests: CI runs both profiles green; spec-sync gate per `.ai/specifications.md`.

Acceptance: CI green on both cpu profiles for one binary; docs accurately state
the runtime-selection guarantee; spec-sync green.
Commit: —

## Validation Plan

- Tests: per-run emission unit test; two-profile value-parity diffs + ULP;
  both CI jobs; spec-sync.
- Runtime proof: **one** build, run under both QEMU cpu profiles (and `ssh -p
  2229` if it has V), produces values bit-identical to AArch64/x86_64 and ≤1
  ULP — the end-to-end demonstration of the one-binary-for-both goal.
- Doc sync: `src/docs/spec/**` build-targets + `.ai/remote_systems.md`.
- Acceptance: full rt-behavior + acceptance suites green on the single riscv64
  binary under both cpu profiles; ULP ≤1 both; other backends byte-identical
  (`scripts/artifact-gate.sh`).

## Open Decisions

- **Guard hoisting** — per-iteration guard inside kernel loops (simplest,
  predicted) vs. hoisting the guard above the loop to run a fully vector or fully
  scalar loop (faster, more codegen). Recommend per-iteration now; hoist only if
  profiling shows the branch matters. (§3)
- **QEMU vlen** — recommend a `vlen=128` run (the minimum guaranteed V width,
  exercising the 2×f64 assumption) plus optionally a larger-vlen run to confirm
  `vl=2` masking is VLEN-independent. (§3)

## Summary

Turns C's correct per-op dual-path into a fast, permanently-gated one-binary
feature: per-run vector-register residency for real speedup, ≤1 ULP and
bit-identical values proven on the *same executable* under both V and non-V
profiles, a two-profile CI lane, and docs stating the portability guarantee.
Low risk over C; the value-parity diff is the backstop against a missed
run-boundary live-out.
